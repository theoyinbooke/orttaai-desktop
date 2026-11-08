//! Word Boost rescoring: turns a transcript, its token timings and the CTC
//! model's per-frame log-probabilities into a transcript with dictionary
//! terms put back where the audio supports them.
//!
//! This is the Mac's `ExpressTranscriber.boosted(...)`: FluidAudio's
//! `VocabularyRescorer.ctcTokenEvaluateCandidates(...)` proposes replacements,
//! the ones its own legacy pass would apply (`legacyOutcome == .applied`) go to
//! [`guard::accepts`](super::guard::accepts), and the survivors are written
//! into the untouched transcript by `applying(...)`.
//!
//! Ported from FluidAudio (Apache-2.0, Copyright FluidInference):
//! `Rescorer/VocabularyRescorer*.swift`, `WordSpotting/CtcDPAlgorithm.swift`,
//! `WordSpotting/CtcKeywordSpotter.swift` (`spotKeywordsFromLogProbs`),
//! `CustomVocabularyContext.swift`, `ContextBiasingConstants.swift` and
//! `VocabularyBoostingSession.swift` (`itnDefaultConfig`). Only the path the
//! Mac reaches is ported, with the parameters it passes: the term-centric
//! search (FluidAudio's BK-tree word-centric search is off), cbw and minimum
//! similarity from `rescorerConfig(forVocabSize:)`, a 0.5 s margin, and the
//! spotter-anchored rescue pass with `itnDefaultConfig`'s similarity floors.
//! The `FLUID_*` environment overrides, debug logging, reason strings,
//! capitalization of the legacy rewritten text, wildcard tokens and per-term
//! weights or similarity overrides are left out: nothing on that path reads
//! them.
//!
//! The timings and the log-probs come from different models here (Parakeet
//! TDT 0.6B through sherpa-onnx, and Parakeet CTC 110M). That is fine: the
//! rescorer only uses each timing's piece text (to rebuild words on `▁`) and
//! its times; phrases are re-spelled with the CTC tokenizer before scoring,
//! exactly as FluidAudio does. Both must be on the same clock: time zero is
//! the first sample the CTC model scored. Pass the recognizer's raw text, as
//! the Mac does: the words rebuilt from the tokens must appear in it in order,
//! or nothing is replaced.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::guard;
use super::tokenizer::{is_extending, is_mark, CtcTokenizer, WORD_BOUNDARY};
pub use crate::text::WordBoostTerm;

// MARK: - Public API

/// One recognizer token: a SentencePiece piece (`▁` starts a word) and its
/// times in seconds from the start of the audio.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenTiming {
    pub token: String,
    pub start: f64,
    pub end: f64,
}

impl TokenTiming {
    /// Timings from a recognizer that reports start times and durations
    /// (sherpa-onnx). A missing duration ends the token where it starts.
    pub fn from_starts_and_durations(
        tokens: &[String],
        starts: &[f32],
        durations: &[f32],
    ) -> Vec<TokenTiming> {
        tokens
            .iter()
            .zip(starts)
            .enumerate()
            .map(|(i, (token, &start))| {
                let start = f64::from(start);
                let duration = durations.get(i).map_or(0.0, |&d| f64::from(d));
                TokenTiming {
                    token: token.clone(),
                    start,
                    end: start + duration,
                }
            })
            .collect()
    }
}

/// One replacement the rescorer would apply, and whether the guard kept it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The phrase as heard (private: what the user said).
    pub heard: String,
    pub term: String,
    /// CTC score of the term minus that of the heard phrase.
    pub margin: f32,
    /// [`guard::spelling_similarity`] of their letters, as the Mac logs it.
    pub similarity: f32,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoostOutcome {
    pub text: String,
    /// Every proposal, kept or not, so a wrong swap can be traced.
    pub proposals: Vec<Proposal>,
}

/// The dictionary, spelled in the CTC model's tokens, with the thresholds
/// FluidAudio picks for its size. Build it once per vocabulary.
#[derive(Debug, Clone)]
pub struct PreparedVocabulary {
    terms: Vec<PreparedTerm>,
    /// Every normalized canonical and alias form, of every term.
    normalized_set: HashSet<String>,
    tokenizer: CtcTokenizer,
    cbw: f32,
    min_similarity: f32,
    blank_id: usize,
}

#[derive(Debug, Clone)]
struct PreparedTerm {
    text: String,
    lowercased: String,
    tokens: Vec<u32>,
    forms: Vec<NormalizedForm>,
}

impl PreparedVocabulary {
    /// The Mac's `setWordBoost`: terms the tokenizer cannot spell are dropped.
    /// The CTC blank is the class after the tokenizer's vocabulary (1024 for
    /// Parakeet CTC 110M), as FluidAudio sets it from the model's vocabulary.
    pub fn new(terms: &[WordBoostTerm], tokenizer: &CtcTokenizer) -> Self {
        let kept: Vec<(&WordBoostTerm, Vec<u32>)> = terms
            .iter()
            .filter_map(|term| {
                let tokens = tokenizer.encode(&term.text);
                (!tokens.is_empty()).then_some((term, tokens))
            })
            .collect();

        // FluidAudio's prepareNormalizedForms: a term's forms include the
        // aliases of every term with the same spelling, then its own.
        let mut aliases_by_canonical: HashMap<String, Vec<&str>> = HashMap::new();
        for (term, _) in &kept {
            aliases_by_canonical
                .entry(term.text.to_lowercase())
                .or_default()
                .extend(term.aliases.iter().map(String::as_str));
        }
        let prepared: Vec<PreparedTerm> = kept
            .iter()
            .map(|(term, tokens)| {
                let lowercased = term.text.to_lowercase();
                let mut aliases = aliases_by_canonical
                    .get(&lowercased)
                    .cloned()
                    .unwrap_or_default();
                aliases.extend(term.aliases.iter().map(String::as_str));
                PreparedTerm {
                    forms: normalized_forms(&term.text, &aliases),
                    text: term.text.clone(),
                    lowercased,
                    tokens: tokens.clone(),
                }
            })
            .collect();
        let normalized_set = prepared
            .iter()
            .flat_map(|t| t.forms.iter().map(|f| f.normalized.clone()))
            .collect();

        let (size_min_similarity, cbw) = rescorer_config_for_vocab_size(prepared.len());
        Self {
            terms: prepared,
            normalized_set,
            tokenizer: tokenizer.clone(),
            cbw,
            min_similarity: size_min_similarity.max(CONTEXT_MIN_SIMILARITY),
            blank_id: tokenizer.vocab_size(),
        }
    }

    /// Overrides the CTC blank class.
    pub fn with_blank_id(mut self, blank_id: usize) -> Self {
        self.blank_id = blank_id;
        self
    }

    pub fn blank_id(&self) -> usize {
        self.blank_id
    }

    /// How many terms Word Boost will use (what the Mac's `setWordBoost` returns).
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// Applies the replacements FluidAudio's rescorer would apply, minus the ones
/// the guard rejects. Never fails: odd input returns the text unchanged.
pub fn boost(
    transcript: &str,
    timings: &[TokenTiming],
    log_probs: &[Vec<f32>],
    frame_duration: f64,
    vocab: &PreparedVocabulary,
) -> BoostOutcome {
    let mut outcome = BoostOutcome {
        text: transcript.to_string(),
        proposals: Vec::new(),
    };
    if vocab.is_empty()
        || timings.is_empty()
        || log_probs.is_empty()
        || !frame_duration.is_finite()
        || frame_duration <= 0.0
    {
        return outcome;
    }

    let candidates = evaluate_candidates(transcript, timings, log_probs, frame_duration, vocab);
    let mut replacements = Vec::new();
    for candidate in candidates {
        let (Outcome::Applied, Some(range), Some(vocabulary_score), Some(original_score)) = (
            candidate.outcome,
            candidate.text_range,
            candidate.raw_vocabulary_score,
            candidate.raw_original_score,
        ) else {
            continue;
        };
        let margin = vocabulary_score - original_score;
        let applied = guard::accepts(&guard::Candidate {
            heard: &candidate.base_phrase,
            term: &candidate.canonical_term,
            acoustic_margin: margin,
        });
        let similarity = guard::spelling_similarity(
            &guard::words(&candidate.base_phrase).concat(),
            &guard::words(&candidate.canonical_term).concat(),
        );
        if applied {
            replacements.push((range, candidate.canonical_term.clone()));
        }
        outcome.proposals.push(Proposal {
            heard: candidate.base_phrase,
            term: candidate.canonical_term,
            margin,
            similarity,
            applied,
        });
    }
    outcome.text = apply_replacements(&replacements, transcript);
    outcome
}

/// The Mac's `ExpressTranscriber.applying`: replaces UTF-8 byte ranges of
/// `text` with terms, keeping punctuation that was attached to the heard
/// phrase ("release." keeps its period). Ranges that are out of bounds or
/// not on character boundaries are skipped.
pub fn apply_replacements(replacements: &[(Range<usize>, String)], text: &str) -> String {
    let mut result = text.to_string();
    let mut sorted: Vec<&(Range<usize>, String)> = replacements.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.0.start));
    for (range, term) in sorted {
        if range.start > range.end
            || range.end > text.len()
            || range.end > result.len()
            || !result.is_char_boundary(range.start)
            || !result.is_char_boundary(range.end)
        {
            continue;
        }
        let heard = &result[range.clone()];
        let is_word_char = |c: char| c.is_alphabetic() || c.is_numeric();
        let leading: String = heard.chars().take_while(|&c| !is_word_char(c)).collect();
        let mut trailing: Vec<char> = heard
            .chars()
            .rev()
            .take_while(|&c| !is_word_char(c))
            .collect();
        trailing.reverse();
        let trailing: String = trailing.into_iter().collect();
        result.replace_range(range.clone(), &format!("{leading}{term}{trailing}"));
    }
    result
}

// MARK: - Constants (ContextBiasingConstants, CustomVocabularyContext)

/// `CustomVocabularyContext.minSimilarity` default.
const CONTEXT_MIN_SIMILARITY: f32 = 0.52;
/// `CustomVocabularyContext.minTermLength` default (characters).
const MIN_TERM_LENGTH: usize = 3;
/// The Mac's `marginSeconds`.
const MARGIN_SECONDS: f64 = 0.5;
const LARGE_VOCAB_THRESHOLD: usize = 10;
const EXTRA_LARGE_VOCAB_THRESHOLD: usize = 100;
const LENGTH_RATIO_THRESHOLD: f32 = 0.75;
const SHORT_WORD_SIMILARITY: f32 = 0.80;
const SHORT_WORD_MAX_LENGTH: usize = 4;
const STOPWORD_SPAN_SIMILARITY: f32 = 0.85;
const SPOTTER_RESCUE_MIN_SCORE: f32 = -10.0;
const SPOTTER_RESCUE_FALLBACK_RADIUS_SECONDS: f64 = 0.5;
const BASELINE_TOKEN_COUNT_FOR_THRESHOLD: usize = 3;
const THRESHOLD_RELAXATION_PER_TOKEN: f32 = 1.0;

/// `ContextBiasingConstants.rescorerConfig(forVocabSize:)`: (minSimilarity, cbw).
fn rescorer_config_for_vocab_size(size: usize) -> (f32, f32) {
    let min_similarity = if size > EXTRA_LARGE_VOCAB_THRESHOLD {
        0.60
    } else if size > LARGE_VOCAB_THRESHOLD {
        0.55
    } else {
        0.50
    };
    (min_similarity, 4.5)
}

/// `VocabularyRescorer.Config`, with the values of `itnDefaultConfig`.
#[derive(Debug, Clone, Copy)]
struct Config {
    use_adaptive_thresholds: bool,
    reference_token_count: usize,
    short_term_cbw_taper_pivot: usize,
    short_term_cbw_taper_exponent: f32,
    spotter_rescue_min_similarity: f32,
    spotter_rescue_multi_word_min_similarity: f32,
    spotter_rescue_enabled: bool,
}

const ITN_DEFAULT_CONFIG: Config = Config {
    use_adaptive_thresholds: true,
    reference_token_count: 3,
    short_term_cbw_taper_pivot: 1,
    short_term_cbw_taper_exponent: 2.0,
    spotter_rescue_min_similarity: 0.30,
    spotter_rescue_multi_word_min_similarity: 0.50,
    spotter_rescue_enabled: true,
};

impl Config {
    /// Shorter terms keep the base boost (the taper is off); longer ones get
    /// `cbw * (1 + log2(tokens / reference) * 0.3)`.
    fn adaptive_cbw(&self, base_cbw: f32, token_count: usize) -> f32 {
        if !self.use_adaptive_thresholds {
            return base_cbw;
        }
        let pivot = self.short_term_cbw_taper_pivot;
        if pivot > 1 && token_count < pivot {
            let ratio = token_count.max(1) as f32 / pivot as f32;
            return base_cbw * ratio.powf(self.short_term_cbw_taper_exponent);
        }
        if token_count <= self.reference_token_count {
            return base_cbw;
        }
        let ratio = token_count as f32 / self.reference_token_count as f32;
        base_cbw * (1.0 + ratio.log2() * 0.3)
    }
}

/// Single-word stopwords: a lone one is never replaced.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "some", "any", "no", "every", "each", "all", "and", "or", "but", "so", "if",
    "then", "than", "as", "in", "on", "at", "to", "for", "of", "with", "by", "from", "up", "down",
    "out", "about", "into", "over", "after", "before", "between", "under", "is", "are", "was",
    "were", "be", "been", "being", "am", "have", "has", "had", "do", "does", "did", "will",
    "would", "can", "could", "go", "goes", "went", "come", "comes", "came", "get", "got", "take",
    "took", "make", "made", "say", "said", "see", "saw", "know", "knew", "think", "thought", "i",
    "you", "he", "she", "it", "we", "they", "me", "him", "her", "us", "them", "my", "your", "his",
    "its", "our", "their", "this", "that", "these", "those", "who", "what", "which", "where",
    "when", "how", "why", "just", "also", "only", "even", "still", "now", "here", "there", "very",
    "well", "back", "way", "own", "new", "old", "good", "great", "first", "last",
];

/// True function words: a multi-word span containing one needs 0.85.
const MULTI_WORD_STOPWORDS: &[&str] = &[
    "a", "an", "the", "some", "any", "no", "every", "each", "all", "and", "or", "but", "so", "if",
    "then", "than", "as", "in", "on", "at", "to", "for", "of", "with", "by", "from", "up", "down",
    "out", "about", "into", "over", "after", "before", "between", "under", "is", "are", "was",
    "were", "be", "been", "being", "am", "have", "has", "had", "do", "does", "did", "will",
    "would", "can", "could", "i", "you", "he", "she", "it", "we", "they", "me", "him", "her", "us",
    "them", "my", "your", "his", "its", "our", "their", "this", "that", "these", "those", "who",
    "what", "which", "where", "when", "how", "why",
];

fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

fn is_multi_word_stopword(word: &str) -> bool {
    MULTI_WORD_STOPWORDS.contains(&word)
}

// MARK: - Candidate evaluation (VocabularyRescorer+TokenRescoring)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Applied,
    RejectedByComparison,
    UnavailableEvidence,
    SupersededByOverlap,
}

/// FluidAudio's `CandidateEvidence`, the fields the Mac reads.
#[derive(Debug, Clone)]
struct Evidence {
    base_phrase: String,
    canonical_term: String,
    raw_vocabulary_score: Option<f32>,
    raw_original_score: Option<f32>,
    /// Byte range of the phrase in the transcript, when exactly aligned.
    text_range: Option<Range<usize>>,
    comparison_passed: bool,
    outcome: Outcome,
}

/// FluidAudio's `CTCMatchCandidate`. Spans are always contiguous on this path.
struct MatchCandidate<'a> {
    original_phrase: String,
    term: &'a PreparedTerm,
    similarity: f32,
    span: Range<usize>,
    span_start_time: f64,
    span_end_time: f64,
}

struct MatchResult {
    should_replace: bool,
    comparison_was_performed: bool,
    raw_vocabulary_score: Option<f32>,
    raw_original_score: Option<f32>,
}

/// A candidate that passed the CTC comparison, waiting for arbitration.
#[derive(Debug, Clone)]
struct Pending {
    evidence_id: usize,
    span: Range<usize>,
    similarity: f32,
}

struct Evaluation<'a> {
    words: &'a [WordTiming],
    aligned: &'a [Option<Range<usize>>],
    transcript: &'a str,
    log_probs: &'a [Vec<f32>],
    frame_duration: f64,
    vocab: &'a PreparedVocabulary,
    config: Config,
    evidence: Vec<Evidence>,
    pending: Vec<Pending>,
}

impl Evaluation<'_> {
    /// `evaluateCTCMatch` + `recordCandidateEvidence`, then queue it if it passed.
    fn evaluate(&mut self, candidate: MatchCandidate<'_>) {
        let result = evaluate_ctc_match(
            &candidate,
            self.log_probs,
            self.frame_duration,
            self.vocab,
            &self.config,
        );
        let comparison_passed = result.comparison_was_performed && result.should_replace;
        let outcome = if !result.comparison_was_performed {
            Outcome::UnavailableEvidence
        } else if comparison_passed {
            // Arbitration turns this into SupersededByOverlap when it loses.
            Outcome::Applied
        } else {
            Outcome::RejectedByComparison
        };
        let text_range = candidate_text_range(
            candidate.span.clone(),
            self.aligned,
            self.transcript,
            &candidate.original_phrase,
        );
        let evidence_id = self.evidence.len();
        self.evidence.push(Evidence {
            base_phrase: candidate.original_phrase,
            canonical_term: candidate.term.text.clone(),
            raw_vocabulary_score: result.raw_vocabulary_score,
            raw_original_score: result.raw_original_score,
            text_range,
            comparison_passed,
            outcome,
        });
        if result.should_replace {
            self.pending.push(Pending {
                evidence_id,
                span: candidate.span,
                similarity: candidate.similarity,
            });
        }
    }

    fn candidate<'t>(
        &self,
        term: &'t PreparedTerm,
        original_phrase: String,
        similarity: f32,
        span: Range<usize>,
    ) -> MatchCandidate<'t> {
        MatchCandidate {
            original_phrase,
            term,
            similarity,
            span_start_time: self.words[span.start].start,
            span_end_time: self.words[span.end - 1].end,
            span,
        }
    }
}

/// `ctcTokenEvaluateCandidates`: every candidate that reached the CTC
/// comparison, with its final legacy outcome.
fn evaluate_candidates(
    transcript: &str,
    timings: &[TokenTiming],
    log_probs: &[Vec<f32>],
    frame_duration: f64,
    vocab: &PreparedVocabulary,
) -> Vec<Evidence> {
    let words = build_word_timings(timings);
    let base_words: Vec<&str> = words.iter().map(|w| w.word.as_str()).collect();
    let aligned = align_base_words_to_utf8_ranges(transcript, &base_words);
    if words.is_empty() || log_probs.is_empty() {
        return Vec::new();
    }

    let mut eval = Evaluation {
        words: &words,
        aligned: &aligned,
        transcript,
        log_probs,
        frame_duration,
        vocab,
        config: ITN_DEFAULT_CONFIG,
        evidence: Vec::new(),
        pending: Vec::new(),
    };
    term_centric_candidates(&mut eval);
    if eval.config.spotter_rescue_enabled && vocab.terms.len() <= LARGE_VOCAB_THRESHOLD {
        spotter_anchored_candidates(&mut eval);
    }

    let applied = arbitrate(&eval.pending);
    let applied_ids: HashSet<usize> = applied
        .iter()
        .map(|&i| eval.pending[i].evidence_id)
        .collect();
    let mut evidence = eval.evidence;
    for (id, candidate) in evidence.iter_mut().enumerate() {
        if candidate.comparison_passed {
            candidate.outcome = if applied_ids.contains(&id) {
                Outcome::Applied
            } else {
                Outcome::SupersededByOverlap
            };
        }
    }
    evidence
}

/// `rescoreWithConstrainedCTCTermCentric`, up to the rescue pass.
fn term_centric_candidates(eval: &mut Evaluation<'_>) {
    let vocab = eval.vocab;
    let words = eval.words;
    let normalized_words: Vec<String> = words
        .iter()
        .map(|w| normalize_for_similarity(&w.word))
        .collect();
    let min_similarity = vocab.min_similarity;

    for term in &vocab.terms {
        let vocab_term = term.text.as_str();
        let term_length = vocab_term.chars().count();
        if term_length < MIN_TERM_LENGTH || term.tokens.is_empty() || term.forms.is_empty() {
            continue;
        }
        let normalized_canonical = normalize_for_similarity(vocab_term);
        let current_set: HashSet<&str> = term.forms.iter().map(|f| f.normalized.as_str()).collect();
        let multi_word_forms: Vec<&NormalizedForm> =
            term.forms.iter().filter(|f| f.word_count > 1).collect();
        let single_word_forms: Vec<&NormalizedForm> =
            term.forms.iter().filter(|f| f.word_count == 1).collect();
        let matches_other_term = |normalized: &str| {
            vocab.normalized_set.contains(normalized) && !current_set.contains(normalized)
        };

        if !multi_word_forms.is_empty() {
            let max_word_count = multi_word_forms
                .iter()
                .map(|f| f.word_count)
                .max()
                .unwrap_or(0);
            let min_word_count = multi_word_forms
                .iter()
                .map(|f| f.word_count)
                .min()
                .unwrap_or(0);
            let max_span = 4.min(max_word_count + 1);
            let min_span = 2.max(min_word_count);
            // FluidAudio skips the whole term here, single-word forms included.
            if min_span > max_span {
                continue;
            }
            for span_length in min_span..=max_span {
                if span_length > words.len() {
                    break;
                }
                for start in 0..=(words.len() - span_length) {
                    let span = start..start + span_length;
                    let phrase = join_words(&words[span.clone()]);
                    let normalized_phrase = normalize_for_similarity(&phrase);
                    if normalized_phrase.is_empty() {
                        continue;
                    }
                    let similarity = best_similarity(&normalized_phrase, &multi_word_forms);
                    if normalized_phrase == normalized_canonical
                        || matches_other_term(&normalized_phrase)
                    {
                        continue;
                    }
                    if similarity < required_similarity(min_similarity, span_length) {
                        continue;
                    }
                    let candidate = eval.candidate(term, phrase, similarity, span);
                    eval.evaluate(candidate);
                }
            }
        }

        if !single_word_forms.is_empty() {
            for (index, timing) in words.iter().enumerate() {
                let normalized_word = normalized_words[index].as_str();
                if normalized_word.is_empty()
                    || normalized_word == normalized_canonical
                    || matches_other_term(normalized_word)
                {
                    continue;
                }
                let mut similarity = best_similarity(normalized_word, &single_word_forms);
                let mut span_length = 1;

                // Compound words: "Liv Mali" for Livmarli.
                let next = normalized_words.get(index + 1).map(String::as_str);
                let after = normalized_words.get(index + 2).map(String::as_str);
                let close_to_term = |word: &str| {
                    single_word_forms
                        .iter()
                        .any(|f| string_similarity(word, &f.normalized) >= 0.9)
                };
                if let Some(next) = next.filter(|w| !w.is_empty()) {
                    if term_length >= 4 && !close_to_term(next) {
                        let joined = format!("{normalized_word}{next}");
                        let joined_similarity = best_similarity(&joined, &single_word_forms);
                        if joined_similarity > similarity {
                            similarity = joined_similarity;
                            span_length = 2;
                        }
                    }
                    if let Some(after) = after.filter(|w| !w.is_empty()) {
                        if term_length >= 8 && !close_to_term(next) && !close_to_term(after) {
                            let joined = format!("{normalized_word}{next}{after}");
                            let joined_similarity = best_similarity(&joined, &single_word_forms);
                            if joined_similarity > similarity {
                                similarity = joined_similarity;
                                span_length = 3;
                            }
                        }
                    }
                }

                let mut threshold = required_similarity(min_similarity, span_length);
                if span_length == 1 {
                    threshold = length_ratio_threshold(normalized_word, term_length, threshold);
                }
                // Stopword rules.
                if span_length == 1 && is_stopword(normalized_word) {
                    continue;
                }
                if span_length >= 2
                    && normalized_words[index..index + span_length]
                        .iter()
                        .any(|w| is_multi_word_stopword(w))
                {
                    threshold = threshold.max(STOPWORD_SPAN_SIMILARITY);
                }
                if similarity < threshold {
                    continue;
                }

                let span = index..index + span_length;
                let phrase = if span_length == 1 {
                    timing.word.clone()
                } else {
                    join_words(&words[span.clone()])
                };
                let candidate = eval.candidate(term, phrase, similarity, span);
                eval.evaluate(candidate);
            }
        }
    }
}

/// `collectSpotterAnchoredCandidates`: terms the CTC keyword spotter hears
/// where the transcript has something spelled too differently for the
/// similarity gate above.
fn spotter_anchored_candidates(eval: &mut Evaluation<'_>) {
    let vocab = eval.vocab;
    let words = eval.words;
    let mut detections = spot_keywords(eval.log_probs, eval.frame_duration, vocab);
    if detections.is_empty() {
        return;
    }
    detections.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let mut seen_spans: HashSet<(String, usize, usize)> = HashSet::new();

    for detection in detections {
        let term = &vocab.terms[detection.term];
        if term.text.chars().count() < MIN_TERM_LENGTH {
            continue;
        }
        let span = word_indices(words, detection.start_time, detection.end_time);
        if span.is_empty() || span.len() > 4 {
            continue;
        }
        if !seen_spans.insert((term.lowercased.clone(), span.start, span.len())) {
            continue;
        }

        let phrase = join_words(&words[span.clone()]);
        let normalized_phrase = normalize_for_similarity(&phrase);
        let current_set: HashSet<&str> = term.forms.iter().map(|f| f.normalized.as_str()).collect();
        if vocab.normalized_set.contains(&normalized_phrase)
            && !current_set.contains(normalized_phrase.as_str())
        {
            continue;
        }
        if normalized_phrase == normalize_for_similarity(&term.text)
            || current_set.contains(normalized_phrase.as_str())
            || term.tokens.is_empty()
        {
            continue;
        }

        // Never rescue over a span of nothing but common short words.
        let span_words: Vec<String> = words[span.clone()]
            .iter()
            .map(|w| normalize_for_similarity(&w.word))
            .collect();
        if span_words.iter().all(|w| is_stopword(w)) {
            continue;
        }

        let forms: Vec<&NormalizedForm> = term.forms.iter().collect();
        let similarity = best_similarity(&normalized_phrase, &forms);
        let floor = if span.len() >= 2 {
            eval.config.spotter_rescue_multi_word_min_similarity
        } else {
            eval.config.spotter_rescue_min_similarity
        };
        if similarity < floor {
            continue;
        }
        let candidate = eval.candidate(term, phrase, similarity, span);
        eval.evaluate(candidate);
    }
}

/// `arbitratePendingReplacements`: by similarity in 0.05 buckets, then the
/// shorter span, then raw similarity; greedily without overlaps. Returns the
/// indices into `pending` that win.
fn arbitrate(pending: &[Pending]) -> Vec<usize> {
    let bucket = |similarity: f32| (similarity / 0.05).round() as i64;
    let mut order: Vec<usize> = (0..pending.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&pending[a], &pending[b]);
        bucket(b.similarity)
            .cmp(&bucket(a.similarity))
            .then(a.span.len().cmp(&b.span.len()))
            .then(
                b.similarity
                    .partial_cmp(&a.similarity)
                    .unwrap_or(Ordering::Equal),
            )
    });
    let mut occupied: HashSet<usize> = HashSet::new();
    let mut applied = Vec::new();
    for index in order {
        let span = pending[index].span.clone();
        if span.clone().any(|i| occupied.contains(&i)) {
            continue;
        }
        occupied.extend(span);
        applied.push(index);
    }
    applied
}

/// `evaluateCTCMatch`: the term's constrained CTC score (with and without its
/// leading boundary, whichever is better) plus the adaptive boost, against
/// the heard phrase's score in the same window.
fn evaluate_ctc_match(
    candidate: &MatchCandidate<'_>,
    log_probs: &[Vec<f32>],
    frame_duration: f64,
    vocab: &PreparedVocabulary,
    config: &Config,
) -> MatchResult {
    let margin_frames = (MARGIN_SECONDS / frame_duration) as i64;
    let span_start_frame = (candidate.span_start_time / frame_duration) as i64;
    let span_end_frame = (candidate.span_end_time / frame_duration) as i64;
    let search_start = span_start_frame.saturating_sub(margin_frames).max(0);
    let search_end = span_end_frame
        .saturating_add(margin_frames)
        .min(log_probs.len() as i64);
    let blank = vocab.blank_id;

    let term_tokens = &candidate.term.tokens;
    let mut vocabulary_score =
        ctc_word_spot_constrained(log_probs, term_tokens, search_start, search_end, blank);
    let mut tokens_used = term_tokens.len();
    let mid_word = vocab
        .tokenizer
        .encode_without_boundary(&candidate.term.text);
    if !mid_word.is_empty() && &mid_word != term_tokens {
        let score =
            ctc_word_spot_constrained(log_probs, &mid_word, search_start, search_end, blank);
        if score > vocabulary_score {
            vocabulary_score = score;
            tokens_used = mid_word.len();
        }
    }
    let finite = |v: f32| v.is_finite().then_some(v);

    let original_tokens = vocab.tokenizer.encode(&candidate.original_phrase);
    if original_tokens.is_empty() {
        return MatchResult {
            should_replace: false,
            comparison_was_performed: false,
            raw_vocabulary_score: finite(vocabulary_score),
            raw_original_score: None,
        };
    }
    let original_score =
        ctc_word_spot_constrained(log_probs, &original_tokens, search_start, search_end, blank);
    let boosted = vocabulary_score + config.adaptive_cbw(vocab.cbw, tokens_used);
    MatchResult {
        should_replace: boosted > original_score,
        comparison_was_performed: true,
        raw_vocabulary_score: finite(vocabulary_score),
        raw_original_score: finite(original_score),
    }
}

// MARK: - Keyword spotting (CtcKeywordSpotter.spotKeywordsFromLogProbs)

struct Detection {
    /// Index into the vocabulary's terms.
    term: usize,
    score: f32,
    start_time: f64,
    end_time: f64,
}

fn spot_keywords(
    log_probs: &[Vec<f32>],
    frame_duration: f64,
    vocab: &PreparedVocabulary,
) -> Vec<Detection> {
    let mut detections = Vec::new();
    for (index, term) in vocab.terms.iter().enumerate() {
        if term.text.chars().count() < MIN_TERM_LENGTH || term.tokens.is_empty() {
            continue;
        }
        let extra_tokens = term
            .tokens
            .len()
            .saturating_sub(BASELINE_TOKEN_COUNT_FOR_THRESHOLD);
        let threshold =
            SPOTTER_RESCUE_MIN_SCORE - extra_tokens as f32 * THRESHOLD_RELAXATION_PER_TOKEN;
        for (score, start, end) in
            ctc_word_spot_multiple(log_probs, &term.tokens, threshold, vocab.blank_id)
        {
            detections.push(Detection {
                term: index,
                score,
                start_time: start as f64 * frame_duration,
                end_time: end as f64 * frame_duration,
            });
        }
    }
    detections
}

/// `wordIndices(in:overlapping:end:)`: the contiguous run of words whose
/// times overlap the detection; failing that, the nearest word if its centre
/// is within half a second.
fn word_indices(words: &[WordTiming], start: f64, end: f64) -> Range<usize> {
    if words.is_empty() || start.is_nan() || end.is_nan() || start >= end {
        return 0..0;
    }
    let mut overlapping = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if word.end < start {
            continue;
        }
        if word.start > end {
            break;
        }
        overlapping.push(index);
    }
    let Some(&first) = overlapping.first() else {
        let center = (start + end) / 2.0;
        let mut nearest: Option<(usize, f64)> = None;
        for (index, word) in words.iter().enumerate() {
            let delta = ((word.start + word.end) / 2.0 - center).abs();
            if nearest.is_none_or(|(_, best)| delta < best) {
                nearest = Some((index, delta));
            }
        }
        return match nearest {
            Some((index, delta)) if delta <= SPOTTER_RESCUE_FALLBACK_RADIUS_SECONDS => {
                index..index + 1
            }
            _ => 0..0,
        };
    };
    let mut last = first;
    for &index in &overlapping[1..] {
        if index != last + 1 {
            break;
        }
        last = index;
    }
    first..last + 1
}

// MARK: - CTC dynamic programming (CtcDPAlgorithm)

/// `terminalStates`: blank-aware CTC word spotting with free starts. For each
/// frame count `t` (0..=T), the best raw score of a path that has consumed the
/// whole keyword, its start frame and the frame of its last token.
fn terminal_states(
    log_probs: &[Vec<f32>],
    tokens: &[u32],
    blank: usize,
) -> (Vec<f32>, Vec<usize>, Vec<usize>) {
    let t_len = log_probs.len();
    let n = tokens.len();
    let neg = -f32::MAX;
    let mut scores = vec![if n == 0 { 0.0 } else { neg }; t_len + 1];
    let mut starts = vec![0usize; t_len + 1];
    let mut ends = vec![0usize; t_len + 1];
    if n == 0 {
        return (scores, starts, ends);
    }

    // [B, t1, B, t2, ..., tN, B]; None is the blank.
    let mut symbols: Vec<Option<u32>> = Vec::with_capacity(2 * n + 1);
    for &token in tokens {
        symbols.push(None);
        symbols.push(Some(token));
    }
    symbols.push(None);
    let s_len = symbols.len();
    let can_skip_blank = |index: usize| -> bool {
        index >= 2
            && match symbols[index] {
                None => false,
                Some(current) => symbols[index - 2] != Some(current),
            }
    };

    let mut previous_scores = vec![neg; s_len];
    let mut current_scores = vec![neg; s_len];
    let mut previous_starts = vec![0usize; s_len];
    let mut current_starts = vec![0usize; s_len];
    let mut previous_last = vec![0usize; s_len];
    let mut current_last = vec![0usize; s_len];
    previous_scores[0] = 0.0;

    for t in 1..=t_len {
        current_scores[0] = 0.0;
        current_starts[0] = t;
        current_last[0] = 0;
        let frame = &log_probs[t - 1];
        for s in 1..s_len {
            let symbol = symbols[s];
            let emission = match symbol {
                None => frame.get(blank).copied().unwrap_or(0.0),
                Some(id) => frame.get(id as usize).copied().unwrap_or(neg),
            };
            let stay = previous_scores[s];
            let advance = previous_scores[s - 1];
            let skip = if can_skip_blank(s) {
                previous_scores[s - 2]
            } else {
                neg
            };

            let mut best = stay;
            let mut kind = 0; // 0 stay, 1 advance, 2 skip blank
            if advance > best {
                best = advance;
                kind = 1;
            }
            if skip > best {
                best = skip;
                kind = 2;
            }
            if best <= neg / 2.0 {
                current_scores[s] = neg;
                current_starts[s] = 0;
                current_last[s] = 0;
                continue;
            }
            current_scores[s] = best + emission;
            let is_token = symbol.is_some();
            match kind {
                0 => {
                    current_starts[s] = previous_starts[s];
                    current_last[s] = if is_token { t } else { previous_last[s] };
                }
                1 => {
                    current_starts[s] = if s == 1 {
                        t - 1
                    } else {
                        previous_starts[s - 1]
                    };
                    current_last[s] = if is_token { t } else { previous_last[s - 1] };
                }
                _ => {
                    current_starts[s] = previous_starts[s - 2];
                    current_last[s] = if is_token { t } else { previous_last[s - 2] };
                }
            }
        }
        let (token_state, blank_state) = (2 * n - 1, 2 * n);
        let pick = if current_scores[token_state] >= current_scores[blank_state] {
            token_state
        } else {
            blank_state
        };
        scores[t] = current_scores[pick];
        starts[t] = current_starts[pick];
        ends[t] = current_last[pick];
        std::mem::swap(&mut previous_scores, &mut current_scores);
        std::mem::swap(&mut previous_starts, &mut current_starts);
        std::mem::swap(&mut previous_last, &mut current_last);
    }
    (scores, starts, ends)
}

/// `ctcWordSpotConstrained`: the best per-token score of the keyword within
/// frames `start..end` (clamped), or -inf when it cannot fit.
fn ctc_word_spot_constrained(
    log_probs: &[Vec<f32>],
    tokens: &[u32],
    start: i64,
    end: i64,
    blank: usize,
) -> f32 {
    let n = tokens.len();
    let clamped_start = start.max(0);
    let clamped_end = end.min(log_probs.len() as i64);
    if n == 0 || clamped_end <= clamped_start {
        return f32::NEG_INFINITY;
    }
    let window = &log_probs[clamped_start as usize..clamped_end as usize];
    if window.len() < n {
        return f32::NEG_INFINITY;
    }
    let (scores, _, _) = terminal_states(window, tokens, blank);
    let mut best = -f32::MAX;
    for &score in &scores[n..=window.len()] {
        if score > best {
            best = score;
        }
    }
    best / n as f32
}

/// `ctcWordSpotMultiple` with overlap merging: every local maximum of the
/// per-token score at or above `min_score`, as (score, start, end) frames.
fn ctc_word_spot_multiple(
    log_probs: &[Vec<f32>],
    tokens: &[u32],
    min_score: f32,
    blank: usize,
) -> Vec<(f32, usize, usize)> {
    let t_len = log_probs.len();
    let n = tokens.len();
    if n == 0 || t_len == 0 || t_len < n {
        return Vec::new();
    }
    let (scores, starts, ends) = terminal_states(log_probs, tokens, blank);
    let norm = n as f32;
    let mut candidates = Vec::new();
    for t in n..=t_len {
        let score = scores[t] / norm;
        let previous = if t > n {
            scores[t - 1] / norm
        } else {
            -f32::MAX
        };
        let next = if t < t_len {
            scores[t + 1] / norm
        } else {
            -f32::MAX
        };
        if score >= previous && score > next && score >= min_score {
            candidates.push((score, starts[t], ends[t]));
        }
    }
    if candidates.is_empty() {
        let mut best_end = 0;
        let mut best = -f32::MAX;
        for (t, &raw) in scores.iter().enumerate().skip(n) {
            let score = raw / norm;
            if score > best {
                best = score;
                best_end = t;
            }
        }
        if best >= min_score {
            candidates.push((best, starts[best_end], ends[best_end]));
        }
    }

    candidates.sort_by_key(|c| c.1);
    let mut merged: Vec<(f32, usize, usize)> = Vec::new();
    for candidate in candidates {
        match merged.last_mut() {
            Some(last) if candidate.1 <= last.2 => {
                let end = last.2.max(candidate.2);
                let mut best = if candidate.0 > last.0 {
                    candidate
                } else {
                    *last
                };
                best.2 = end;
                *last = best;
            }
            _ => merged.push(candidate),
        }
    }
    merged
}

// MARK: - Words and text (VocabularyRescorer + Utilities)

#[derive(Debug, Clone, PartialEq)]
struct WordTiming {
    word: String,
    start: f64,
    end: f64,
}

fn is_word_boundary(token: &str) -> bool {
    token.starts_with(WORD_BOUNDARY) || token.starts_with(' ')
}

fn strip_word_boundary(token: &str) -> &str {
    token
        .strip_prefix(WORD_BOUNDARY)
        .or_else(|| token.strip_prefix(' '))
        .unwrap_or(token)
}

/// Foundation's `CharacterSet.whitespaces`: spaces and tab, not newlines.
fn is_inline_whitespace(c: char) -> bool {
    c == '\t'
        || (c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ))
}

/// `buildWordTimings`: tokens grouped into words on `▁`; `<blank>`, `<pad>`
/// and empty tokens are skipped.
fn build_word_timings(timings: &[TokenTiming]) -> Vec<WordTiming> {
    let mut words = Vec::new();
    let mut current = String::new();
    let (mut start, mut end) = (0.0, 0.0);
    let mut push = |current: &str, start: f64, end: f64| {
        let trimmed = current.trim_matches(is_inline_whitespace);
        if !trimmed.is_empty() {
            words.push(WordTiming {
                word: trimmed.to_string(),
                start,
                end,
            });
        }
    };
    for timing in timings {
        let token = timing.token.as_str();
        if token.is_empty() || token == "<blank>" || token == "<pad>" {
            continue;
        }
        let starts_new_word = is_word_boundary(token) || current.is_empty();
        if starts_new_word && !current.is_empty() {
            push(&current, start, end);
            current.clear();
        }
        if starts_new_word {
            current = strip_word_boundary(token).to_string();
            start = timing.start;
        } else {
            current.push_str(token);
        }
        end = timing.end;
    }
    push(&current, start, end);
    words
}

fn join_words(words: &[WordTiming]) -> String {
    words
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `normalizeForSimilarity`: lowercase, letters, digits, marks, apostrophes
/// and hyphens kept, runs of spaces, tabs and newlines collapsed to one space,
/// everything else dropped.
fn normalize_for_similarity(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = true;
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() || is_mark(c) || c == '\'' || c == '-' {
            result.push(c);
            last_was_space = false;
        } else if matches!(c, ' ' | '\t' | '\n') && !last_was_space && !result.is_empty() {
            result.push(' ');
            last_was_space = true;
        }
    }
    result.trim().to_string()
}

/// `stringSimilarity`: 1 - Levenshtein / longer length, case-insensitive.
fn string_similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    guard::spelling_similarity(&a, &b)
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedForm {
    normalized: String,
    word_count: usize,
}

/// `normalizedForms`: the canonical spelling then each alias, normalized,
/// without empty or repeated forms.
fn normalized_forms(canonical: &str, aliases: &[&str]) -> Vec<NormalizedForm> {
    let mut seen = HashSet::new();
    let mut forms = Vec::new();
    for raw in std::iter::once(canonical).chain(aliases.iter().copied()) {
        let normalized = normalize_for_similarity(raw);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        forms.push(NormalizedForm {
            word_count: normalized.split(' ').filter(|w| !w.is_empty()).count(),
            normalized,
        });
    }
    forms
}

/// The best similarity of `text` to any form, 0 when none is above 0.
fn best_similarity(text: &str, forms: &[&NormalizedForm]) -> f32 {
    let mut best = 0.0;
    for form in forms {
        let similarity = string_similarity(text, &form.normalized);
        if similarity > best {
            best = similarity;
        }
    }
    best
}

/// `requiredSimilarity`: multi-word spans need at least 0.55.
fn required_similarity(min_similarity: f32, span_length: usize) -> f32 {
    if span_length >= 2 {
        min_similarity.max(0.55)
    } else {
        min_similarity
    }
}

/// `checkLengthRatioRules`: a short word much shorter than the term needs 0.80
/// ("you" -> Yu, "or" -> VR).
fn length_ratio_threshold(normalized_word: &str, term_length: usize, min_similarity: f32) -> f32 {
    let word_length = normalized_word.chars().count();
    let ratio = word_length as f32 / term_length as f32;
    if ratio < LENGTH_RATIO_THRESHOLD && word_length <= SHORT_WORD_MAX_LENGTH {
        min_similarity.max(SHORT_WORD_SIMILARITY)
    } else {
        min_similarity
    }
}

// MARK: - Aligning words to the transcript

fn is_inter_word_delimiter(c: char) -> bool {
    !(c.is_alphabetic() || c.is_numeric())
}

fn is_leading_quote(c: char) -> bool {
    matches!(
        c,
        '"' | '\u{201C}' | '\u{AB}' | '\u{2039}' | '\u{300C}' | '\u{300E}'
    )
}

fn is_trailing_quote(c: char) -> bool {
    matches!(
        c,
        '"' | '\u{201D}' | '\u{BB}' | '\u{203A}' | '\u{300D}' | '\u{300F}'
    )
}

fn is_trailing_sentence_punctuation(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | '\u{2026}' | '\u{3002}' | '\u{3001}' | '\u{FF1B}'
    )
}

fn is_technical_punctuation(c: char) -> bool {
    matches!(
        c,
        '#' | '.' | '@' | '%' | '&' | '*' | '/' | '\\' | '_' | '-' | '`' | '\'' | '\u{2019}' | '^'
    )
}

/// Unicode math and currency symbols (Sm, Sc), the common ones.
fn is_math_or_currency_symbol(c: char) -> bool {
    matches!(c as u32,
        0x24 | 0x2B | 0x3C..=0x3E | 0x7C | 0x7E
            | 0xA2..=0xA5 | 0xAC | 0xB1 | 0xD7 | 0xF7
            | 0x3F6 | 0x58F | 0x606..=0x608 | 0x60B | 0x9F2 | 0x9F3 | 0xE3F
            | 0x2044 | 0x2052 | 0x207A..=0x207C | 0x208A..=0x208C | 0x20A0..=0x20C0
            | 0x2118 | 0x2140..=0x2144 | 0x214B | 0x2190..=0x2194 | 0x21D2 | 0x21D4
            | 0x21F4..=0x22FF | 0x2320..=0x2321 | 0x237C | 0x239B..=0x23B3
            | 0x25B7 | 0x25C1 | 0x25F8..=0x25FF | 0x266F | 0x27C0..=0x27C4
            | 0x27C7..=0x27E5 | 0x27F0..=0x27FF | 0x2900..=0x2982 | 0x2999..=0x29D7
            | 0x29DC..=0x29FB | 0x29FE..=0x2AFF | 0x2B30..=0x2B44 | 0x2B47..=0x2B4C
            | 0xFB29 | 0xFDFC | 0xFE62 | 0xFE64..=0xFE66 | 0xFE69 | 0xFF04 | 0xFF0B
            | 0xFF1C..=0xFF1E | 0xFF5C | 0xFF5E | 0xFFE0..=0xFFE2 | 0xFFE5 | 0xFFE6
            | 0xFFE9..=0xFFEC | 0x1EEF0 | 0x1EEF1)
}

/// `isSafeLexicalBoundary`, judged on a character cluster's base scalar.
fn is_safe_lexical_boundary(c: char) -> bool {
    c.is_alphabetic()
        || c.is_numeric()
        || is_technical_punctuation(c)
        || is_math_or_currency_symbol(c)
}

/// `lexicalUTF8Range`: an exact match with its quotation wrappers and
/// sentence punctuation trimmed, or None when its edges are not safe to
/// replace ("String?", "func()", "Azure™").
fn lexical_range(range: Range<usize>, text: &str) -> Option<Range<usize>> {
    let (mut lower, mut upper) = (range.start, range.end);
    while lower < upper {
        let c = text[lower..].chars().next()?;
        if !is_leading_quote(c) {
            break;
        }
        lower += c.len_utf8();
    }
    while lower < upper {
        let c = text[..upper].chars().next_back()?;
        if !(is_trailing_quote(c) || is_trailing_sentence_punctuation(c)) {
            break;
        }
        upper -= c.len_utf8();
    }
    if lower >= upper {
        return None;
    }
    let first = text[lower..upper].chars().next()?;
    let last = text[lower..upper]
        .chars()
        .rev()
        .find(|&c| !is_extending(c))
        .unwrap_or(first);
    (is_safe_lexical_boundary(first) && is_safe_lexical_boundary(last)).then_some(lower..upper)
}

/// `alignBaseWordsToUTF8Ranges`: each word's byte range in the untouched
/// transcript, when the whole sequence matches exactly once with only
/// non-alphanumeric text between words. Otherwise every range is None.
fn align_base_words_to_utf8_ranges(text: &str, words: &[&str]) -> Vec<Option<Range<usize>>> {
    if words.is_empty() {
        return Vec::new();
    }
    if words.iter().any(|w| w.is_empty()) {
        return vec![None; words.len()];
    }

    struct Frame {
        state: (usize, usize),
        limit: usize,
        next: Option<usize>,
        alignments_at_entry: usize,
    }
    let frame = |word_index: usize, cursor: usize, alignments: usize| -> Frame {
        let limit = text[cursor..]
            .char_indices()
            .find(|&(_, c)| !is_inter_word_delimiter(c))
            .map_or(text.len(), |(i, _)| cursor + i);
        Frame {
            state: (word_index, cursor),
            limit,
            next: Some(cursor),
            alignments_at_entry: alignments,
        }
    };

    let mut alignment_count = 0;
    let mut alignment: Vec<Range<usize>> = Vec::new();
    let mut path: Vec<Range<usize>> = Vec::with_capacity(words.len());
    let mut dead_states: HashSet<(usize, usize)> = HashSet::new();
    let mut stack = vec![frame(0, 0, 0)];

    while alignment_count < 2 {
        let Some(top) = stack.last_mut() else { break };
        let word_index = top.state.0;
        if word_index == words.len() {
            if top.limit == text.len() {
                alignment_count += 1;
                if alignment_count == 1 {
                    alignment = path.clone();
                }
            }
            stack.pop();
            path.pop();
            continue;
        }
        let Some(start) = top.next.filter(|&s| s < text.len()) else {
            if alignment_count == top.alignments_at_entry {
                dead_states.insert(top.state);
            }
            stack.pop();
            path.pop();
            continue;
        };
        top.next = if start < top.limit {
            text[start..].chars().next().map(|c| start + c.len_utf8())
        } else {
            None
        };
        let word = words[word_index];
        if !text[start..].starts_with(word) {
            continue;
        }
        let matched_end = start + word.len();
        if dead_states.contains(&(word_index + 1, matched_end)) {
            continue;
        }
        path.push(start..matched_end);
        stack.push(frame(word_index + 1, matched_end, alignment_count));
    }

    if alignment_count != 1 {
        return vec![None; words.len()];
    }
    alignment
        .into_iter()
        .map(|r| lexical_range(r, text))
        .collect()
}

/// `candidateBaseTextUTF8Range`: the byte range of a word span, when every
/// word is aligned and the text there normalizes to the evaluated phrase.
fn candidate_text_range(
    word_range: Range<usize>,
    aligned: &[Option<Range<usize>>],
    text: &str,
    phrase: &str,
) -> Option<Range<usize>> {
    if word_range.is_empty() || word_range.end > aligned.len() {
        return None;
    }
    let mut ranges = Vec::with_capacity(word_range.len());
    for index in word_range {
        ranges.push(aligned[index].clone()?);
    }
    let range = ranges.first()?.start..ranges.last()?.end;
    let candidate_text = text.get(range.clone())?;
    (normalize_for_similarity(candidate_text) == normalize_for_similarity(phrase)).then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wordboost::tokenizer::tests::{id, letters};

    fn timing(token: &str, start: f64, end: f64) -> TokenTiming {
        TokenTiming {
            token: token.to_string(),
            start,
            end,
        }
    }

    fn term(text: &str, aliases: &[&str]) -> WordBoostTerm {
        WordBoostTerm {
            text: text.to_string(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn texts(ranges: &[Option<Range<usize>>], text: &str) -> Vec<Option<String>> {
        ranges
            .iter()
            .map(|r| r.clone().map(|r| text[r].to_string()))
            .collect()
    }

    // MARK: Utilities (FluidAudio VocabularyRescorerUtilsTests)

    #[test]
    fn string_similarity_is_case_insensitive_levenshtein() {
        assert!((string_similarity("nvidia", "nvidia") - 1.0).abs() < 0.01);
        assert!(string_similarity("abc", "xyz").abs() < 0.01);
        assert!((string_similarity("NVIDIA", "nvidia") - 1.0).abs() < 0.01);
        assert!((string_similarity("bose", "boz") - 0.5).abs() < 0.01);
        assert!((string_similarity("", "") - 1.0).abs() < 0.01);
        assert!(string_similarity("abc", "").abs() < 0.01);
        assert!((string_similarity("nvida", "nvidia") - (1.0 - 1.0 / 6.0)).abs() < 0.01);
    }

    #[test]
    fn normalize_for_similarity_keeps_letters_apostrophes_and_hyphens() {
        assert_eq!(normalize_for_similarity("Hello World!"), "hello world");
        assert_eq!(normalize_for_similarity("It's"), "it's");
        assert_eq!(normalize_for_similarity("Ramirez-Santos"), "ramirez-santos");
        assert_eq!(normalize_for_similarity("  hello   world  "), "hello world");
        assert_eq!(normalize_for_similarity(""), "");
        assert_eq!(normalize_for_similarity("Test123"), "test123");
        assert_eq!(
            normalize_for_similarity("hello\tworld\nfoo"),
            "hello world foo"
        );
        assert_eq!(normalize_for_similarity("Cafe\u{301}"), "cafe\u{301}");
    }

    #[test]
    fn adaptive_cbw_grows_for_long_terms_only() {
        let config = ITN_DEFAULT_CONFIG;
        assert!((config.adaptive_cbw(3.0, 3) - 3.0).abs() < 0.01);
        assert!((config.adaptive_cbw(3.0, 6) - 3.9).abs() < 0.01);
        assert!((config.adaptive_cbw(3.0, 2) - 3.0).abs() < 0.01);
        for tokens in 1..=3 {
            assert!((config.adaptive_cbw(4.5, tokens) - 4.5).abs() < 0.01);
        }
        let disabled = Config {
            use_adaptive_thresholds: false,
            ..ITN_DEFAULT_CONFIG
        };
        assert!((disabled.adaptive_cbw(3.0, 10) - 3.0).abs() < 0.01);
        // FluidAudio's opt-in short-term taper, which the Mac leaves off.
        let tapered = Config {
            reference_token_count: 5,
            short_term_cbw_taper_pivot: 5,
            ..ITN_DEFAULT_CONFIG
        };
        assert!((tapered.adaptive_cbw(4.5, 1) - 0.18).abs() < 0.01);
        assert!((tapered.adaptive_cbw(4.5, 3) - 1.62).abs() < 0.01);
        assert!((tapered.adaptive_cbw(4.5, 4) - 2.88).abs() < 0.01);
        assert!((tapered.adaptive_cbw(4.5, 5) - 4.5).abs() < 0.01);
    }

    #[test]
    fn stopword_sets_split_function_words_from_content_words() {
        for word in [
            "new", "old", "good", "great", "first", "last", "well", "back", "way", "own", "just",
            "also", "only", "even", "still", "now", "here", "there", "very",
        ] {
            assert!(!is_multi_word_stopword(word), "{word}");
        }
        for word in [
            "a", "the", "and", "or", "is", "to", "for", "in", "of", "with", "by", "i", "you", "he",
            "she", "it", "we", "they", "this", "that",
        ] {
            assert!(is_multi_word_stopword(word), "{word}");
        }
        for word in [
            "just", "new", "old", "good", "great", "back", "way", "own", "now", "here", "there",
            "still",
        ] {
            assert!(is_stopword(word), "{word}");
        }
    }

    #[test]
    fn size_config_matches_fluidaudio() {
        assert_eq!(rescorer_config_for_vocab_size(1), (0.50, 4.5));
        assert_eq!(rescorer_config_for_vocab_size(10), (0.50, 4.5));
        assert_eq!(rescorer_config_for_vocab_size(11), (0.55, 4.5));
        assert_eq!(rescorer_config_for_vocab_size(101), (0.60, 4.5));
        let tok = letters();
        let small = PreparedVocabulary::new(&[term("Qwen", &[])], &tok);
        assert!((small.min_similarity - 0.52).abs() < 1e-6);
        let many: Vec<WordBoostTerm> = (0..11).map(|i| term(&format!("term{i}"), &[])).collect();
        let large = PreparedVocabulary::new(&many, &tok);
        assert!((large.min_similarity - 0.55).abs() < 1e-6);
        assert_eq!(large.len(), 11);
        assert_eq!(small.blank_id(), 28);
        assert_eq!(small.with_blank_id(1024).blank_id(), 1024);
    }

    // MARK: Forms (VocabularyCandidateEvidenceTests, DiscoveryTests)

    #[test]
    fn normalized_forms_keep_canonical_first_and_drop_repeats() {
        let forms = normalized_forms("ESLint", &["E S lint", "es-lint", "eslint", "!!!", ""]);
        let normalized: Vec<&str> = forms.iter().map(|f| f.normalized.as_str()).collect();
        assert_eq!(normalized, vec!["eslint", "e s lint", "es-lint"]);
        assert_eq!(
            forms.iter().map(|f| f.word_count).collect::<Vec<_>>(),
            vec![1, 3, 1]
        );
    }

    #[test]
    fn prepared_forms_share_aliases_between_spellings_of_one_term() {
        let tok = letters();
        let terms = [
            term("ESLint", &["E S lint", "es lint"]),
            term("eslint", &["E S LINT", "easylint"]),
            term("Claude Code", &["cloud code", "", "!!!"]),
            term("AI", &[]),
        ];
        let vocab = PreparedVocabulary::new(&terms, &tok);
        let forms = |i: usize| -> Vec<&str> {
            vocab.terms[i]
                .forms
                .iter()
                .map(|f| f.normalized.as_str())
                .collect()
        };
        assert_eq!(forms(0), vec!["eslint", "e s lint", "es lint", "easylint"]);
        assert_eq!(forms(1), vec!["eslint", "e s lint", "es lint", "easylint"]);
        assert_eq!(forms(2), vec!["claude code", "cloud code"]);
        assert_eq!(forms(3), vec!["ai"]);
        let expected: HashSet<String> = [
            "eslint",
            "e s lint",
            "es lint",
            "easylint",
            "claude code",
            "cloud code",
            "ai",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(vocab.normalized_set, expected);
        assert!(PreparedVocabulary::new(&[], &tok).is_empty());
    }

    // MARK: Word timings

    #[test]
    fn word_timings_join_pieces_and_skip_blank_and_pad() {
        let words = build_word_timings(&[
            timing("\u{2581}test", 0.0, 0.2),
            timing("ing", 0.2, 0.4),
            timing("\u{2581}again", 0.4, 0.8),
        ]);
        assert_eq!(
            words,
            vec![
                WordTiming {
                    word: "testing".into(),
                    start: 0.0,
                    end: 0.4
                },
                WordTiming {
                    word: "again".into(),
                    start: 0.4,
                    end: 0.8
                },
            ]
        );
        let words = build_word_timings(&[
            timing("\u{2581}hello", 0.0, 0.2),
            timing("<blank>", 0.2, 0.3),
            timing("\u{2581}world", 0.3, 0.5),
            timing("<pad>", 0.5, 0.6),
            timing("", 0.6, 0.7),
            timing(".", 0.7, 0.8),
        ]);
        let words: Vec<&str> = words.iter().map(|w| w.word.as_str()).collect();
        assert_eq!(words, vec!["hello", "world."]);
        // A lone boundary token starts nothing.
        let words = build_word_timings(&[timing("\u{2581}", 0.0, 0.1), timing("ok", 0.1, 0.2)]);
        assert_eq!(
            words,
            vec![WordTiming {
                word: "ok".into(),
                start: 0.1,
                end: 0.2
            }]
        );
    }

    #[test]
    fn token_timings_from_starts_and_durations() {
        let tokens = vec!["\u{2581}a".to_string(), "b".to_string(), "c".to_string()];
        let timings =
            TokenTiming::from_starts_and_durations(&tokens, &[0.0, 0.5, 1.0], &[0.25, 0.5]);
        assert_eq!(timings.len(), 3);
        assert_eq!(timings[0].end, 0.25);
        assert_eq!(timings[1].end, 1.0);
        assert_eq!(timings[2].end, 1.0);
    }

    // MARK: Alignment (VocabularyCandidateEvidenceTests)

    #[test]
    fn alignment_handles_punctuation_contractions_hyphens_and_unicode() {
        let text = "\u{201C}Hello,\u{201D} don't re-enter Caf\u{E9} \u{2615}\u{FE0F}.";
        let words = ["\u{201C}Hello,\u{201D}", "don't", "re-enter", "Caf\u{E9}"];
        let ranges = align_base_words_to_utf8_ranges(text, &words);
        assert_eq!(
            texts(&ranges, text),
            vec![
                Some("Hello".into()),
                Some("don't".into()),
                Some("re-enter".into()),
                Some("Caf\u{E9}".into())
            ]
        );
        let phrase = candidate_text_range(1..3, &ranges, text, "don't re-enter").unwrap();
        assert_eq!(&text[phrase], "don't re-enter");
    }

    #[test]
    fn alignment_preserves_technical_boundary_symbols() {
        let text = "Use C++, C#, .NET, @MainActor, #if, and $PATH.";
        let words = [
            "Use",
            "C++,",
            "C#,",
            ".NET,",
            "@MainActor,",
            "#if,",
            "and",
            "$PATH.",
        ];
        let ranges = texts(&align_base_words_to_utf8_ranges(text, &words), text);
        assert_eq!(ranges[1].as_deref(), Some("C++"));
        assert_eq!(ranges[2].as_deref(), Some("C#"));
        assert_eq!(ranges[3].as_deref(), Some(".NET"));
        assert_eq!(ranges[4].as_deref(), Some("@MainActor"));
        assert_eq!(ranges[5].as_deref(), Some("#if"));
        assert_eq!(ranges[7].as_deref(), Some("$PATH"));
    }

    #[test]
    fn alignment_fails_closed_for_ambiguous_or_symbol_edges() {
        for text in [
            "String?",
            "func()",
            "[Azure]",
            "Azure\u{2122}",
            "Azure\u{2615}\u{FE0F}",
        ] {
            assert_eq!(
                align_base_words_to_utf8_ranges(text, &[text]),
                vec![None],
                "{text}"
            );
        }
    }

    #[test]
    fn repeated_words_resolve_only_through_the_complete_sequence() {
        let ranges = align_base_words_to_utf8_ranges("go go", &["go", "go"]);
        assert_eq!(ranges, vec![Some(0..2), Some(3..5)]);
        assert_eq!(
            align_base_words_to_utf8_ranges("go go", &["go"]),
            vec![None]
        );
    }

    #[test]
    fn missing_reordered_and_normalization_only_matches_fail_closed() {
        let cases: [(&str, &[&str]); 6] = [
            ("one two", &["two", "one"]),
            ("one", &["one", "two"]),
            ("Azure", &["azure"]),
            ("Caf\u{E9}", &["Cafe"]),
            ("Caf\u{E9}", &["Cafe\u{301}"]),
            ("a b", &["a", ""]),
        ];
        for (text, words) in cases {
            let ranges = align_base_words_to_utf8_ranges(text, words);
            assert_eq!(ranges.len(), words.len());
            assert!(ranges.iter().all(Option::is_none), "{text} / {words:?}");
        }
        assert!(align_base_words_to_utf8_ranges("anything", &[]).is_empty());
    }

    #[test]
    fn long_transcript_aligns_without_recursion() {
        let vocabulary = [
            "so",
            "we",
            "should",
            "um",
            "go",
            "go",
            "C++,",
            "re-enter",
            "don't",
            "\u{201C}remote\u{201D}",
        ];
        let words: Vec<&str> = (0..20_000)
            .map(|i| vocabulary[i * 7 % vocabulary.len()])
            .collect();
        let text = words.join(" ");
        let ranges = align_base_words_to_utf8_ranges(&text, &words);
        assert_eq!(ranges.len(), words.len());
        assert!(ranges.iter().all(Option::is_some));
        let truncated = align_base_words_to_utf8_ranges(&text, &words[..words.len() - 1]);
        assert!(truncated.iter().all(Option::is_none));
    }

    #[test]
    fn candidate_range_rejects_phrase_mismatch() {
        let text = "Caf\u{E9} noir";
        let ranges = align_base_words_to_utf8_ranges(text, &["Caf\u{E9}", "noir"]);
        assert!(candidate_text_range(0..1, &ranges, text, "coffee").is_none());
        assert!(candidate_text_range(0..1, &ranges, text, "caf\u{E9}").is_some());
        assert!(candidate_text_range(1..3, &ranges, text, "noir").is_none());
        assert!(candidate_text_range(0..0, &ranges, text, "").is_none());
    }

    // MARK: Arbitration (VocabularyCandidateArbitrationEvidenceTests)

    #[test]
    fn longer_span_with_clearly_better_similarity_wins_the_overlap() {
        // rom vimza -> Romvimza (0.89) beats vimza -> Cimzia (0.67).
        let pending = vec![
            Pending {
                evidence_id: 1,
                span: 1..2,
                similarity: 0.67,
            },
            Pending {
                evidence_id: 0,
                span: 0..2,
                similarity: 0.89,
            },
        ];
        assert_eq!(arbitrate(&pending), vec![1]);
        // Equal buckets: the shorter span wins.
        let pending = vec![
            Pending {
                evidence_id: 0,
                span: 0..2,
                similarity: 0.80,
            },
            Pending {
                evidence_id: 1,
                span: 1..2,
                similarity: 0.81,
            },
        ];
        assert_eq!(arbitrate(&pending), vec![1]);
    }

    #[test]
    fn non_overlapping_passes_all_apply() {
        let pending = vec![
            Pending {
                evidence_id: 1,
                span: 2..4,
                similarity: 0.7,
            },
            Pending {
                evidence_id: 0,
                span: 0..1,
                similarity: 0.9,
            },
        ];
        let mut applied = arbitrate(&pending);
        applied.sort();
        assert_eq!(applied, vec![0, 1]);
        assert!(arbitrate(&[]).is_empty());
    }

    // MARK: CTC DP (FluidAudio CtcDPAlgorithmTests)

    fn hot_frames(
        frames: usize,
        vocab: usize,
        hot: &[(usize, usize)],
        high: f32,
        cold: f32,
    ) -> Vec<Vec<f32>> {
        let mut matrix = vec![vec![cold; vocab]; frames];
        for &(frame, token) in hot {
            if frame < frames && token < vocab {
                matrix[frame][token] = high;
            }
        }
        matrix
    }

    fn frame(
        vocab: usize,
        hot: Option<usize>,
        high: f32,
        blank: usize,
        blank_score: f32,
        cold: f32,
    ) -> Vec<f32> {
        let mut row = vec![cold; vocab];
        if blank < vocab {
            row[blank] = blank_score;
        }
        if let Some(h) = hot.filter(|&h| h < vocab) {
            row[h] = high;
        }
        row
    }

    const DEFAULT_BLANK: usize = 1024;

    #[test]
    fn constrained_window_finds_or_misses_the_keyword() {
        let lp = hot_frames(20, 5, &[(5, 0), (6, 1)], -0.1, -10.0);
        assert!(ctc_word_spot_constrained(&lp, &[0, 1], 3, 12, DEFAULT_BLANK) > -1.0);
        let lp = hot_frames(20, 5, &[(15, 0), (16, 1)], -0.1, -10.0);
        assert!(ctc_word_spot_constrained(&lp, &[0, 1], 0, 10, DEFAULT_BLANK) < -5.0);
        let lp = hot_frames(5, 3, &[(2, 0)], -0.1, -10.0);
        assert!(ctc_word_spot_constrained(&lp, &[0], -5, 100, DEFAULT_BLANK) > f32::NEG_INFINITY);
        let lp = hot_frames(20, 5, &[], -0.1, -10.0);
        assert_eq!(
            ctc_word_spot_constrained(&lp, &[0, 1, 2], 5, 7, DEFAULT_BLANK),
            f32::NEG_INFINITY
        );
        let lp = hot_frames(10, 5, &[], -0.1, -10.0);
        assert_eq!(
            ctc_word_spot_constrained(&lp, &[0], 5, 5, DEFAULT_BLANK),
            f32::NEG_INFINITY
        );
        assert_eq!(
            ctc_word_spot_constrained(&lp, &[], 0, 10, DEFAULT_BLANK),
            f32::NEG_INFINITY
        );
        assert_eq!(
            ctc_word_spot_constrained(&lp, &[0], 50, 60, DEFAULT_BLANK),
            f32::NEG_INFINITY
        );
    }

    #[test]
    fn perfect_match_scores_its_average_log_prob() {
        let lp = hot_frames(3, 3, &[(0, 0), (1, 1), (2, 2)], -0.05, -10.0);
        let score = ctc_word_spot_constrained(&lp, &[0, 1, 2], 0, 3, DEFAULT_BLANK);
        assert!((score + 0.05).abs() < 0.01);
    }

    #[test]
    fn blank_emission_cost_is_accumulated() {
        let (blank, v) = (3, 4);
        let lp = vec![
            frame(v, Some(0), -0.1, blank, -0.5, -10.0),
            frame(v, None, -0.1, blank, -0.5, -10.0),
            frame(v, None, -0.1, blank, -0.5, -10.0),
            frame(v, None, -0.1, blank, -0.5, -10.0),
            frame(v, Some(1), -0.1, blank, -0.5, -10.0),
        ];
        let score = ctc_word_spot_constrained(&lp, &[0, 1], 0, 5, blank);
        assert!((score + 0.85).abs() < 0.01, "{score}");
    }

    #[test]
    fn repeated_tokens_require_an_intervening_blank() {
        let (blank, v) = (2, 3);
        let no_blank = vec![
            frame(v, Some(0), -0.1, blank, -0.5, -10.0),
            frame(v, Some(0), -0.1, blank, -0.5, -10.0),
        ];
        let with_blank = vec![
            frame(v, Some(0), -0.1, blank, -0.5, -10.0),
            frame(v, None, -0.1, blank, -0.5, -10.0),
            frame(v, Some(0), -0.1, blank, -0.5, -10.0),
        ];
        let without = ctc_word_spot_constrained(&no_blank, &[0, 0], 0, 2, blank);
        let with = ctc_word_spot_constrained(&with_blank, &[0, 0], 0, 3, blank);
        assert!(with > without + 1.0);
    }

    #[test]
    fn multiple_detections_keep_independent_spans() {
        let lp: Vec<Vec<f32>> = (0..24)
            .map(|f| {
                let token = if [3, 17].contains(&f) {
                    Some(0)
                } else if [4, 18].contains(&f) {
                    Some(1)
                } else {
                    None
                };
                frame(3, token, -0.05, 2, -0.2, -12.0)
            })
            .collect();
        let results = ctc_word_spot_multiple(&lp, &[0, 1], -0.1, 2);
        assert_eq!(results.iter().map(|r| r.1).collect::<Vec<_>>(), vec![3, 17]);
        assert_eq!(results.iter().map(|r| r.2).collect::<Vec<_>>(), vec![5, 19]);
        for result in results {
            assert!((result.0 + 0.05).abs() < 1e-6);
        }
    }

    #[test]
    fn multiple_detections_handle_empty_and_weak_input() {
        let lp = hot_frames(5, 3, &[], -0.1, -10.0);
        assert!(ctc_word_spot_multiple(&lp, &[], -15.0, DEFAULT_BLANK).is_empty());
        assert!(ctc_word_spot_multiple(&[], &[0], -15.0, DEFAULT_BLANK).is_empty());
        assert!(ctc_word_spot_multiple(&lp, &[0], -5.0, DEFAULT_BLANK).is_empty());
        let lp = hot_frames(10, 5, &[(2, 0)], -0.1, -10.0);
        let results = ctc_word_spot_multiple(&lp, &[0], -1.0, DEFAULT_BLANK);
        assert!(!results.is_empty());
        assert!(results[0].0 > -1.0);
    }

    #[test]
    fn detections_map_to_overlapping_or_nearby_words() {
        let words = vec![
            WordTiming {
                word: "a".into(),
                start: 0.0,
                end: 0.5,
            },
            WordTiming {
                word: "b".into(),
                start: 0.6,
                end: 1.0,
            },
            WordTiming {
                word: "c".into(),
                start: 3.0,
                end: 3.5,
            },
        ];
        assert_eq!(word_indices(&words, 0.4, 0.7), 0..2);
        assert_eq!(word_indices(&words, 1.2, 1.4), 1..2); // nearest, 0.5 s away
        assert_eq!(word_indices(&words, 2.0, 2.1), 0..0); // nothing near
        assert_eq!(word_indices(&words, 0.7, 0.7), 0..0);
        assert_eq!(word_indices(&[], 0.0, 1.0), 0..0);
    }

    // MARK: Applying (the Mac's WordBoostTests)

    #[test]
    fn replacements_keep_attached_punctuation() {
        let text = "Ask Olen Ruaju, then ship Whisper kit.";
        let olen = text.len() - "Olen Ruaju, then ship Whisper kit.".len();
        let whisper = text.len() - "Whisper kit.".len();
        let result = apply_replacements(
            &[
                (olen..olen + "Olen Ruaju,".len(), "Olanrewaju".to_string()),
                (whisper..text.len(), "WhisperKit".to_string()),
            ],
            text,
        );
        assert_eq!(result, "Ask Olanrewaju, then ship WhisperKit.");
    }

    #[test]
    fn replacements_index_utf8_bytes_and_skip_bad_ranges() {
        let text = "Caf\u{E9} notes for Quen";
        let start = text.len() - "Quen".len();
        assert_eq!(
            apply_replacements(&[(start..text.len(), "Qwen".to_string())], text),
            "Caf\u{E9} notes for Qwen"
        );
        // Mid-character, reversed and out-of-bounds ranges change nothing.
        for range in [4..5, Range { start: 3, end: 1 }, 0..999] {
            assert_eq!(apply_replacements(&[(range, "X".to_string())], text), text);
        }
    }

    // MARK: Boosting synthetic audio

    const FRAME: f64 = 0.08;

    /// CTC frames for the letters tokenizer (blank 28): each token hot for one
    /// frame and then a blank frame, with silence around.
    fn speak(spoken: &str) -> Vec<Vec<f32>> {
        let classes = 29;
        let blank = 28;
        let silence = || frame(classes, None, 0.0, blank, -0.05, -10.0);
        let mut frames: Vec<Vec<f32>> = (0..3).map(|_| silence()).collect();
        for c in spoken.chars() {
            let token = if c == ' ' { id(WORD_BOUNDARY) } else { id(c) } as usize;
            frames.push(frame(classes, Some(token), -0.05, blank, -3.0, -10.0));
            frames.push(silence());
        }
        frames.extend((0..3).map(|_| silence()));
        frames
    }

    /// Recognizer tokens for `words` of the transcript, timed where `speak`
    /// put them (each word's letters at two frames per character).
    fn timings_for(spoken: &str, pieces: &[&[&str]]) -> Vec<TokenTiming> {
        let mut out = Vec::new();
        let mut frame_index = 3;
        for (word, pieces) in spoken.split(' ').filter(|w| !w.is_empty()).zip(pieces) {
            let start = frame_index as f64 * FRAME;
            frame_index += 2 * (word.chars().count() + 1);
            let end = frame_index as f64 * FRAME;
            for (i, piece) in pieces.iter().enumerate() {
                let piece = if i == 0 {
                    format!("\u{2581}{piece}")
                } else {
                    piece.to_string()
                };
                out.push(timing(&piece, start, end));
            }
        }
        out
    }

    fn vocabulary(terms: &[WordBoostTerm]) -> PreparedVocabulary {
        PreparedVocabulary::new(terms, &letters())
    }

    #[test]
    fn a_misheard_name_is_replaced_and_keeps_its_comma() {
        let spoken = " ask qwen then ship";
        let timings = timings_for(
            spoken,
            &[&["Ask"], &["Qu", "en", ","], &["then"], &["ship", "."]],
        );
        let vocab = vocabulary(&[term("Qwen", &[])]);
        let outcome = boost(
            "Ask Quen, then ship.",
            &timings,
            &speak(spoken),
            FRAME,
            &vocab,
        );
        assert_eq!(outcome.text, "Ask Qwen, then ship.");
        assert_eq!(outcome.proposals.len(), 1, "{:?}", outcome.proposals);
        let proposal = &outcome.proposals[0];
        assert_eq!(
            (proposal.heard.as_str(), proposal.term.as_str()),
            ("Quen,", "Qwen")
        );
        assert!(proposal.applied);
        assert!(proposal.margin > 0.0, "{proposal:?}");
        assert!((proposal.similarity - 0.75).abs() < 1e-6);
    }

    #[test]
    fn an_ordinary_word_is_not_turned_into_a_product_name() {
        // The audio says "code"; Xcode sounds close enough for FluidAudio's
        // rescorer to apply it, and the guard keeps the ordinary word.
        let spoken = " ask code";
        let timings = timings_for(spoken, &[&["Ask"], &["code"]]);
        let vocab = vocabulary(&[term("Xcode", &[])]);
        let outcome = boost("Ask code", &timings, &speak(spoken), FRAME, &vocab);
        assert_eq!(outcome.text, "Ask code");
        assert!(!outcome.proposals.is_empty());
        for proposal in &outcome.proposals {
            assert_eq!(
                (proposal.heard.as_str(), proposal.term.as_str()),
                ("code", "Xcode")
            );
            assert!(!proposal.applied);
            assert!(
                proposal.margin < guard::ORDINARY_WORDS_MARGIN,
                "{proposal:?}"
            );
            assert!(proposal.margin > guard::MARGIN_FLOOR, "{proposal:?}");
        }
    }

    #[test]
    fn a_spelled_out_join_becomes_the_term() {
        let spoken = " ship whisper kit";
        let timings = timings_for(spoken, &[&["Ship"], &["Whis", "per"], &["kit", "."]]);
        let vocab = vocabulary(&[term("WhisperKit", &[])]);
        let outcome = boost("Ship Whisper kit.", &timings, &speak(spoken), FRAME, &vocab);
        assert_eq!(outcome.text, "Ship WhisperKit.");
        assert_eq!(outcome.proposals.iter().filter(|p| p.applied).count(), 1);
    }

    #[test]
    fn a_multi_word_alias_finds_the_term() {
        let spoken = " ask olanrewaju";
        // The recognizer split the name in two; its halves share its frames
        // (11..33, as `speak` lays them out).
        let (start, middle, end) = (11.0 * FRAME, 22.0 * FRAME, 33.0 * FRAME);
        let timings = vec![
            timing("\u{2581}Ask", 3.0 * FRAME, start),
            timing("\u{2581}Alan", start, middle),
            timing("\u{2581}Ru", middle, end),
            timing("aju", middle, end),
            timing(".", middle, end),
        ];
        let vocab = vocabulary(&[term("Olanrewaju", &["alan ruaju"])]);
        let outcome = boost("Ask Alan Ruaju.", &timings, &speak(spoken), FRAME, &vocab);
        assert_eq!(outcome.text, "Ask Olanrewaju.");
    }

    #[test]
    fn nothing_to_propose_leaves_the_text_alone() {
        let spoken = " ship it";
        let timings = timings_for(spoken, &[&["Ship"], &["it", "."]]);
        let vocab = vocabulary(&[term("Qwen", &[])]);
        let outcome = boost("Ship it.", &timings, &speak(spoken), FRAME, &vocab);
        assert_eq!(
            outcome,
            BoostOutcome {
                text: "Ship it.".into(),
                proposals: vec![]
            }
        );
        let empty = vocabulary(&[]);
        assert!(boost("Ship it.", &timings, &speak(spoken), FRAME, &empty)
            .proposals
            .is_empty());
    }

    #[test]
    fn odd_input_never_panics_and_never_rewrites_blindly() {
        let vocab = vocabulary(&[
            term("Qwen", &["kwen"]),
            term("WhisperKit", &[]),
            term("ab", &[]),
        ]);
        let spoken = " ask qwen";
        let good = speak(spoken);
        let good_timings = timings_for(spoken, &[&["Ask"], &["Quen"]]);
        let text = "Ask Quen";
        // Degenerate frames and durations.
        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                boost(text, &good_timings, &good, duration, &vocab).text,
                text
            );
        }
        assert_eq!(boost(text, &good_timings, &[], FRAME, &vocab).text, text);
        assert_eq!(boost(text, &[], &good, FRAME, &vocab).text, text);
        // Text that does not contain the recognizer's words: no replacement.
        assert_eq!(
            boost("Something else", &good_timings, &good, FRAME, &vocab).text,
            "Something else"
        );
        // Ragged, short, NaN and infinite frames; strange times.
        let ragged: Vec<Vec<f32>> = (0..40)
            .map(|i| match i % 5 {
                0 => vec![],
                1 => vec![f32::NAN; 3],
                2 => vec![f32::NEG_INFINITY; 29],
                3 => vec![f32::INFINITY; 40],
                _ => vec![-1.0; 7],
            })
            .collect();
        let strange = vec![
            timing("\u{2581}Ask", f64::NAN, -3.0),
            timing("\u{2581}Quen", 1e12, -1e12),
            timing("<blank>", 0.0, 0.0),
            timing("", 0.0, 0.0),
            timing("\u{2581}", 0.0, 0.0),
        ];
        for frames in [&ragged, &good] {
            for timings in [&strange, &good_timings] {
                for duration in [FRAME, 1e-9, 1e9] {
                    let _ = boost(text, timings, frames, duration, &vocab);
                    let _ = boost("", timings, frames, duration, &vocab);
                    let _ = boost(
                        "\u{2581}\u{2615}\u{FE0F} \"",
                        timings,
                        frames,
                        duration,
                        &vocab,
                    );
                }
            }
        }
        // A pseudo-random sweep.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let pieces = [
            "\u{2581}qu",
            "en",
            "\u{2581}whis",
            "per",
            "\u{2581}kit",
            ".",
            ",",
            "\u{2581}a",
            "b",
            "\u{2581}",
        ];
        for _ in 0..50 {
            let count = (next() % 12) as usize;
            let mut timings = Vec::new();
            let mut text = String::new();
            for i in 0..count {
                let piece = pieces[(next() % pieces.len() as u64) as usize];
                text.push_str(&piece.replace('\u{2581}', " "));
                timings.push(timing(piece, i as f64 * 0.2, i as f64 * 0.2 + 0.3));
            }
            let frames: Vec<Vec<f32>> = (0..(next() % 60) as usize)
                .map(|_| (0..29).map(|_| -((next() % 1000) as f32) / 100.0).collect())
                .collect();
            let _ = boost(text.trim(), &timings, &frames, FRAME, &vocab);
        }
    }
}
