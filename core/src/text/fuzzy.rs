//! Second dictionary pass (Mac `FuzzyDictionaryMatcher`): corrects recognizer
//! near-misses of dictionary *targets* (Tematope → Temitope, chat GPT →
//! ChatGPT). Runs after the literal source/target rows and is deliberately
//! conservative: a wrong rewrite of a real word or of a different person's
//! name costs more than a missed correction. English only (the word list is):
//! callers skip it for any other dictation language.
//!
//! A window is 1–3 consecutive words separated only by spaces. It is rewritten
//! to the target's exact spelling when either
//!  - compound-exact: its letters-only skeleton equals the target's
//!    (no speech threshold → noSpeechThreshold, app cast → appcast); or
//!  - near-miss: same first letter and a small, vowel-only spelling drift
//!    (Postgar SQL → PostgreSQL, see [`FuzzyMatcher::near_miss`]).
//!
//! A window that is only a regular inflection of the target, or the other way
//! round (websockets / WebSocket), is never touched. A single word that is a
//! valid English word is never touched, a multi-word window of real words
//! matches only as an exact compound, and windows with stopwords are skipped
//! unless the target itself contains that word.

use super::chars::{is_blank, skeleton};
use super::lexicon;
use std::collections::{HashMap, HashSet};

/// Targets shorter than this (letters/digits) are ignored: short names collide
/// with too many real words.
pub const MINIMUM_TARGET_LENGTH: usize = 6;
const MAXIMUM_WINDOW_LENGTH: usize = 3;
const MINIMUM_WINDOW_SKELETON: usize = 5;
/// Targets shorter than this tolerate one vowel edit, longer ones two.
const LONG_TARGET_LENGTH: usize = 8;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "nor", "so", "yet", "if", "then", "than", "that", "this",
    "these", "those", "of", "to", "in", "on", "at", "by", "for", "with", "from", "into", "onto",
    "about", "as", "is", "are", "was", "were", "be", "been", "being", "am", "do", "does", "did",
    "done", "has", "have", "had", "having", "will", "would", "can", "could", "shall", "should",
    "may", "might", "must", "i", "me", "my", "mine", "you", "your", "yours", "he", "him", "his",
    "she", "her", "hers", "it", "its", "we", "us", "our", "ours", "they", "them", "their",
    "theirs", "not", "no", "yes", "there", "here", "who", "whom", "what", "which", "when", "where",
    "why", "how", "all", "any", "some", "each", "every", "up", "out", "off", "over", "just",
    "very", "also", "too", "again", "more", "most", "much", "many", "only", "own", "same", "such",
    "both", "few", "other", "another", "because", "while", "after", "before", "during", "through",
    "between", "against", "under", "above", "below", "via", "per",
];

/// Characters that make a word part of a URL, path, email, tag or identifier
/// rather than prose.
const CODE_ADJACENT: &[char] = &['@', '/', '\\', '`', '#', '$', '_'];

/// One rewrite made by the fuzzy pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub original: String,
    pub target: String,
}

#[derive(Debug, Clone)]
struct Target {
    text: String,
    skeleton: Vec<char>,
    spelling: String,
    stems: HashSet<String>,
    consonants: Vec<char>,
    merged_vowels: Vec<char>,
    words: HashSet<String>,
    /// "WebSocket", "GitHub": the user spelled it with a brand casing, so even
    /// a lowercase spelling that is a listed word is recased.
    has_internal_capital: bool,
}

struct Token {
    start: usize,
    end: usize,
    word: String,
    skeleton: Vec<char>,
    is_blocked: bool,
    is_real_word: bool,
}

struct Candidate<'a> {
    length: usize,
    start: usize,
    end: usize,
    target: &'a Target,
    is_exact: bool,
    distance: usize,
}

/// The precomputed fuzzy pass for one dictionary.
#[derive(Debug, Clone, Default)]
pub struct FuzzyMatcher {
    targets_by_first_letter: HashMap<char, Vec<Target>>,
    exact_texts: HashSet<String>,
}

impl FuzzyMatcher {
    /// Precomputes every per-target cache, so the per-dictation pass only
    /// compares.
    pub fn new<S: AsRef<str>>(targets: &[S]) -> Self {
        let mut seen = HashSet::new();
        let mut by_first: HashMap<char, Vec<Target>> = HashMap::new();
        let mut exact = HashSet::new();
        for text in targets {
            let trimmed = text.as_ref().trim();
            let skel = skeleton(trimmed);
            if skel.len() < MINIMUM_TARGET_LENGTH || !seen.insert(trimmed.to_lowercase()) {
                continue;
            }
            let spelling: String = skel.iter().collect();
            let target = Target {
                text: trimmed.to_string(),
                stems: lexicon::stems(&spelling).into_iter().collect(),
                consonants: skel.iter().copied().filter(|c| !is_vowel(*c)).collect(),
                merged_vowels: merging_repeated_vowels(&skel),
                words: words_in(trimmed).into_iter().collect(),
                has_internal_capital: trimmed.chars().skip(1).any(char::is_uppercase),
                spelling,
                skeleton: skel.clone(),
            };
            by_first.entry(skel[0]).or_default().push(target);
            exact.insert(trimmed.to_string());
        }
        Self {
            targets_by_first_letter: by_first,
            exact_texts: exact,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.targets_by_first_letter.is_empty()
    }

    /// Rewrites every near-miss in `text`, returning the new text and each
    /// replacement made, in order.
    pub fn apply(&self, text: &str) -> (String, Vec<Replacement>) {
        if self.is_empty() || !lexicon::is_available() {
            return (text.to_string(), Vec::new());
        }
        let chars: Vec<char> = text.chars().collect();
        let tokens = tokenize(&chars);
        let mut edits: Vec<(usize, usize, Replacement)> = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            if let Some(m) = self.best_candidate(index, &tokens, &chars) {
                edits.push((
                    m.start,
                    m.end,
                    Replacement {
                        original: chars[m.start..m.end].iter().collect(),
                        target: m.target.text.clone(),
                    },
                ));
                index += m.length;
            } else {
                index += 1;
            }
        }
        if edits.is_empty() {
            return (text.to_string(), Vec::new());
        }
        let mut result = String::with_capacity(text.len());
        let mut cursor = 0;
        for (start, end, rep) in &edits {
            result.extend(&chars[cursor..*start]);
            result.push_str(&rep.target);
            cursor = *end;
        }
        result.extend(&chars[cursor..]);
        (result, edits.into_iter().map(|(_, _, r)| r).collect())
    }

    fn best_candidate<'a>(
        &'a self,
        start: usize,
        tokens: &[Token],
        text: &[char],
    ) -> Option<Candidate<'a>> {
        let mut best: Option<Candidate<'a>> = None;
        for length in 1..=MAXIMUM_WINDOW_LENGTH {
            if start + length > tokens.len() {
                break;
            }
            let window = &tokens[start..start + length];
            if window.iter().any(|t| t.is_blocked) {
                break;
            }
            if length > 1 && !is_separated_by_spaces(window, text) {
                break;
            }
            let skel: Vec<char> = window.iter().flat_map(|t| t.skeleton.clone()).collect();
            if skel.len() < MINIMUM_WINDOW_SKELETON {
                continue;
            }
            let Some(targets) = self.targets_by_first_letter.get(&skel[0]) else {
                continue;
            };
            let range_start = window[0].start;
            let range_end = window[length - 1].end;
            let window_text: String = text[range_start..range_end].iter().collect();
            // Already spelled exactly as a target: nothing to correct.
            if self.exact_texts.contains(&window_text) {
                continue;
            }
            let stopwords_in_window: HashSet<&str> = window
                .iter()
                .map(|t| t.word.as_str())
                .filter(|w| STOPWORDS.contains(w))
                .collect();
            let all_real = window.iter().all(|t| t.is_real_word);
            let spelling: String = skel.iter().collect();
            let stems: HashSet<String> = lexicon::stems(&spelling).into_iter().collect();

            for target in targets {
                if stems.contains(&target.spelling) || target.stems.contains(&spelling) {
                    continue;
                }
                let candidate = if skel == target.skeleton {
                    let may_recase = !all_real || target.has_internal_capital;
                    let allowed =
                        length > 1 || (may_recase && !has_own_casing(&window_text, target));
                    let stopwords_ok = stopwords_in_window
                        .iter()
                        .all(|w| target.words.contains(*w));
                    (allowed && stopwords_ok).then_some(Candidate {
                        length,
                        start: range_start,
                        end: range_end,
                        target,
                        is_exact: true,
                        distance: 0,
                    })
                } else if !all_real && stopwords_in_window.is_empty() {
                    near_miss(length, range_start, range_end, &skel, target)
                } else {
                    None
                };
                if let Some(c) = candidate {
                    if is_better(&c, best.as_ref()) {
                        best = Some(c);
                    }
                }
            }
        }
        best
    }
}

/// True when the window differs from the target only by capitalization and is
/// mixed case, like "ChatGPT" against a dictionary's "ChatGpt": that is the
/// recognizer's own deliberate spelling, so it is left alone.
fn has_own_casing(window: &str, target: &Target) -> bool {
    if window.to_lowercase() != target.text.to_lowercase() {
        return false;
    }
    let rest: String = window.chars().skip(1).collect();
    rest != rest.to_lowercase()
}

/// A near-miss must look like the same word heard by ear, and never like a
/// different real name. All of these hold: the same consonants in the same
/// order; the same last letter; the same length once a repeated vowel counts
/// as one; and a vowel edit budget of one for targets under eight letters and
/// two for longer ones (a swap within {a, e, i} or within {o, u} costs one,
/// anything else two).
fn near_miss<'a>(
    length: usize,
    start: usize,
    end: usize,
    skel: &[char],
    target: &'a Target,
) -> Option<Candidate<'a>> {
    if skel.last() != target.skeleton.last() {
        return None;
    }
    let consonants: Vec<char> = skel.iter().copied().filter(|c| !is_vowel(*c)).collect();
    if consonants != target.consonants {
        return None;
    }
    let merged = merging_repeated_vowels(skel);
    if merged.len() != target.merged_vowels.len() {
        return None;
    }
    let budget = if target.skeleton.len() < LONG_TARGET_LENGTH {
        1
    } else {
        2
    };
    let distance = vowel_edit_distance(&merged, &target.merged_vowels);
    (distance <= budget).then_some(Candidate {
        length,
        start,
        end,
        target,
        is_exact: false,
        distance,
    })
}

/// Exact compounds beat near-misses, longer windows beat shorter ones (so
/// "Mitomo app" is not split), then the smaller edit distance wins.
fn is_better(candidate: &Candidate<'_>, best: Option<&Candidate<'_>>) -> bool {
    let Some(best) = best else {
        return true;
    };
    if candidate.is_exact != best.is_exact {
        return candidate.is_exact;
    }
    if candidate.length != best.length {
        return candidate.length > best.length;
    }
    candidate.distance < best.distance
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn tokenize(text: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if !is_token_char(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && is_token_char(text[i]) {
            i += 1;
        }
        let raw: String = text[start..i].iter().collect();
        let skel = skeleton(&raw);
        let word: String = skel.iter().collect();
        tokens.push(Token {
            start,
            end: i,
            is_blocked: raw.contains('_') || is_code_adjacent(start, i, text),
            is_real_word: !raw.chars().any(char::is_numeric) && lexicon::is_real_word(&word),
            word,
            skeleton: skel,
        });
    }
    tokens
}

fn is_code_adjacent(start: usize, end: usize, text: &[char]) -> bool {
    if start > 0 {
        let before = text[start - 1];
        if CODE_ADJACENT.contains(&before) {
            return true;
        }
        // "file.mitomo": a dot glued to a word on the left.
        if before == '.' && start > 1 && text[start - 2].is_alphanumeric() {
            return true;
        }
    }
    if end < text.len() {
        let after = text[end];
        if CODE_ADJACENT.contains(&after) {
            return true;
        }
        // "mitomo.com": a dot glued to a word on the right.
        if after == '.' && text.get(end + 1).is_some_and(|c| c.is_alphanumeric()) {
            return true;
        }
    }
    false
}

fn is_separated_by_spaces(window: &[Token], text: &[char]) -> bool {
    window.windows(2).all(|pair| {
        let gap = &text[pair[0].end..pair[1].start];
        !gap.is_empty() && gap.iter().all(|c| is_blank(*c))
    })
}

fn is_vowel(c: char) -> bool {
    "aeiou".contains(c)
}

/// Vowels are told apart by ear within {a, e, i} and within {o, u}, far less
/// across the two.
fn vowel_group(c: char) -> u8 {
    u8::from(!"aei".contains(c))
}

/// "meetumo" → "metumo": a doubled vowel is one long vowel sound.
fn merging_repeated_vowels(skel: &[char]) -> Vec<char> {
    let mut merged: Vec<char> = Vec::with_capacity(skel.len());
    for &c in skel {
        if is_vowel(c) && merged.last() == Some(&c) {
            continue;
        }
        merged.push(c);
    }
    merged
}

/// Splits "noSpeechThreshold", "PostgreSQL", "Meetumo-app" into lowercase
/// words.
fn words_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (offset, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(previous) = current.chars().last() {
            let starts_upper_run = c.is_uppercase() && previous.is_lowercase();
            let ends_upper_run = c.is_uppercase()
                && previous.is_uppercase()
                && chars.get(offset + 1).is_some_and(|n| n.is_lowercase());
            let switches_kind = c.is_numeric() != previous.is_numeric();
            if starts_upper_run || ends_upper_run || switches_kind {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .map(|w| skeleton(&w).into_iter().collect())
        .collect()
}

/// Levenshtein distance where swapping two vowels of the same group costs one
/// and any other substitution costs two (a delete plus an insert), so a
/// consonant can never be traded for a vowel cheaply.
fn vowel_edit_distance(lhs: &[char], rhs: &[char]) -> usize {
    if lhs.is_empty() {
        return rhs.len();
    }
    if rhs.is_empty() {
        return lhs.len();
    }
    let mut previous: Vec<usize> = (0..=rhs.len()).collect();
    let mut current = vec![0usize; rhs.len() + 1];
    for (row, &left) in lhs.iter().enumerate() {
        current[0] = row + 1;
        for (column, &right) in rhs.iter().enumerate() {
            let substitution = if left == right {
                0
            } else if is_vowel(left) && is_vowel(right) && vowel_group(left) == vowel_group(right) {
                1
            } else {
                2
            };
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[rhs.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corrected(text: &str, targets: &[&str]) -> String {
        FuzzyMatcher::new(targets).apply(text).0
    }

    #[test]
    fn near_miss_names_are_corrected_to_the_target_spelling() {
        assert_eq!(
            corrected("Tematope will present the metrics.", &["Temitope"]),
            "Temitope will present the metrics."
        );
        for variant in ["Mitumo", "Metumo"] {
            assert_eq!(
                corrected(&format!("Ask {variant} about it"), &["Meetumo"]),
                "Ask Meetumo about it",
                "{variant}"
            );
        }
        assert_eq!(
            corrected("Ask Yemesi to update it", &["Yemisi"]),
            "Ask Yemisi to update it"
        );
    }

    #[test]
    fn names_heard_with_a_different_syllable_shape_are_left_alone() {
        assert_eq!(
            corrected("Owenbook asked", &["Oyinbooke"]),
            "Owenbook asked"
        );
        assert_eq!(
            corrected("Taimytope will draft it.", &["Temitope"]),
            "Taimytope will draft it."
        );
        assert_eq!(
            corrected("Send it to Folosaday today", &["Folasade"]),
            "Send it to Folosaday today"
        );
        assert_eq!(
            corrected("Install Olima first", &["Ollama"]),
            "Install Olima first"
        );
    }

    #[test]
    fn near_miss_brands_across_word_boundaries_are_corrected() {
        assert_eq!(
            corrected("The Postgar SQL replica lagged.", &["PostgreSQL"]),
            "The PostgreSQL replica lagged."
        );
        assert_eq!(
            corrected("Mitomo app is live", &["Meetumo-app"]),
            "Meetumo-app is live"
        );
    }

    #[test]
    fn compound_exact_windows_are_joined_to_the_target() {
        assert_eq!(
            corrected("I love chat GPT a lot", &["ChatGPT"]),
            "I love ChatGPT a lot"
        );
        assert_eq!(
            corrected("Set the no speech threshold now", &["noSpeechThreshold"]),
            "Set the noSpeechThreshold now"
        );
        assert_eq!(
            corrected("The app cast feed is stale", &["appcast"]),
            "The appcast feed is stale"
        );
        assert_eq!(
            corrected("Use the use effect callback hook", &["useEffectCallback"]),
            "Use the useEffectCallback hook"
        );
        assert_eq!(
            corrected("The Whisper Kit upgrade", &["WhisperKit"]),
            "The WhisperKit upgrade"
        );
        assert_eq!(
            corrected("half in camel case", &["camelCase"]),
            "half in camelCase"
        );
        assert_eq!(
            corrected("The energy VAD framing", &["EnergyVAD"]),
            "The EnergyVAD framing"
        );
    }

    #[test]
    fn target_casing_is_applied_and_mixed_case_is_kept() {
        assert_eq!(
            corrected("open the websocket now", &["WebSocket"]),
            "open the WebSocket now"
        );
        assert_eq!(
            corrected("asked chatgpt about it", &["ChatGpt"]),
            "asked ChatGpt about it"
        );
        assert_eq!(
            corrected("look at ChatGPT, you know", &["ChatGpt"]),
            "look at ChatGPT, you know"
        );
        assert_eq!(
            corrected("open Websocket now", &["WebSocket"]),
            "open WebSocket now"
        );
    }

    #[test]
    fn every_occurrence_is_corrected_and_reported() {
        let (text, reps) = FuzzyMatcher::new(&["Temitope", "Meetumo"])
            .apply("Tematope met Mitumo, then Tematope left.");
        assert_eq!(text, "Temitope met Meetumo, then Temitope left.");
        let pairs: Vec<(&str, &str)> = reps
            .iter()
            .map(|r| (r.original.as_str(), r.target.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("Tematope", "Temitope"),
                ("Mitumo", "Meetumo"),
                ("Tematope", "Temitope")
            ]
        );
    }

    #[test]
    fn already_correct_text_is_untouched() {
        let targets = [
            "Temitope",
            "Meetumo-app",
            "noSpeechThreshold",
            "PostgreSQL",
            "Vercel",
        ];
        for text in [
            "Temitope",
            "Meetumo-app ships",
            "the noSpeechThreshold value",
            "PostgreSQL and Vercel",
        ] {
            assert_eq!(corrected(text, &targets), text);
        }
    }

    #[test]
    fn real_english_words_near_targets_are_never_rewritten() {
        let targets = [
            "Vercel",
            "Meetumo",
            "Arsenal",
            "TypeScript",
            "Temitope",
            "Yemisi",
            "Folasade",
            "Supabase",
            "Codexis",
        ];
        for sentence in [
            "Read the verse and then the verses again.",
            "Join the meetup and other meetups.",
            "The arena was full and the arsenal was empty.",
            "Write the script and read the transcript.",
            "Clear the cache and pay cash.",
            "Enter the codes and use the codex.",
            "Temitayo and Yemi are different people from Temitope.",
            "A super base for a super basement.",
            "The vessel was late.",
        ] {
            assert_eq!(corrected(sentence, &targets), sentence);
        }
    }

    #[test]
    fn different_real_people_with_similar_names_are_not_rewritten() {
        let targets = ["Temitope", "Yemisi", "Folasade"];
        for name in ["Temitayo", "Yemi", "Folake", "Tomiwa"] {
            let sentence = format!("Ask {name} to join.");
            assert_eq!(corrected(&sentence, &targets), sentence, "{name}");
        }
        let cases: &[(&str, &[&str])] = &[
            ("Michael", &["Michelle", "Michaela", "Michele", "Michal"]),
            ("Claude", &["Claudia", "Claudio", "Cloud"]),
            ("Olamide", &["Olumide"]),
            ("Sophia", &["Sophie"]),
            (
                "Temitope",
                &[
                    "Temitayo", "Temidayo", "Yemi", "Folake", "Tomiwa", "Temitopa",
                ],
            ),
            ("Adebayo", &["Adebayor", "Adebayi"]),
            ("Oluwaseun", &["Oluwaseyi"]),
            ("Folasade", &["Folake", "Folasida"]),
        ];
        for (target, names) in cases {
            for name in *names {
                let sentence = format!("Ask {name} to join.");
                assert_eq!(
                    corrected(&sentence, &[target]),
                    sentence,
                    "{name} vs {target}"
                );
            }
        }
    }

    #[test]
    fn windows_with_stopwords_are_skipped_unless_the_target_contains_them() {
        assert_eq!(
            corrected("with the Tematope team", &["Temitope"]),
            "with the Temitope team"
        );
        assert_eq!(
            corrected("set no speech threshold", &["noSpeechThreshold"]),
            "set noSpeechThreshold"
        );
        assert_eq!(corrected("go to gether", &["Together"]), "go to gether");
    }

    #[test]
    fn multi_word_windows_of_real_words_match_only_as_exact_compounds() {
        let sentence = "The finalized live transcription call";
        assert_eq!(
            corrected(sentence, &["finalizeLiveTranscription"]),
            sentence
        );
        assert_eq!(corrected("super base", &["Supabase"]), "super base");
    }

    #[test]
    fn adjacent_words_are_not_swallowed() {
        assert_eq!(
            corrected("Tematope presented slides", &["Temitope"]),
            "Temitope presented slides"
        );
        assert_eq!(corrected("Mitumo apps", &["Meetumo"]), "Meetumo apps");
    }

    #[test]
    fn short_targets_and_windows_are_ignored() {
        assert_eq!(
            corrected("We ship with Tori", &["Tauri"]),
            "We ship with Tori"
        );
        assert_eq!(corrected("Qen models", &["Qwen"]), "Qen models");
        assert!(FuzzyMatcher::new(&["Tauri", "groq", "  "]).is_empty());
    }

    #[test]
    fn code_and_path_adjacent_words_are_left_alone() {
        for text in [
            "mail me@Tematope.org",
            "open /usr/Tematope/bin",
            "call snake_Tematope_case",
            "see Tematope.com now",
        ] {
            assert_eq!(corrected(text, &["Temitope"]), text);
        }
    }

    #[test]
    fn different_first_letter_is_never_corrected() {
        assert_eq!(
            corrected("Alain Rouajou wants it", &["Olanrewaju"]),
            "Alain Rouajou wants it"
        );
    }

    #[test]
    fn inflected_forms_are_never_rewritten_either_way() {
        for (target, text) in [
            ("WebSocket", "We use websockets a lot."),
            ("WebSocket", "Two Websockets stayed open."),
            ("Codebase", "the codebases are big"),
            ("Codebase", "the code bases are big"),
            ("Vercel", "We are vercelling it tonight."),
            ("Vercel", "The Vercels differ."),
            ("Temitope", "Both Temitopes came."),
            ("Meetumo", "Two Meetumos merged."),
            ("Codebases", "the codebase is big"),
            ("WebSockets", "open one websocket"),
            ("Temitopes", "ask Temitope"),
            ("Vercels", "deploy on Vercel"),
        ] {
            assert_eq!(corrected(text, &[target]), text, "{text}");
        }
    }

    #[test]
    fn name_near_miss_rules_are_vowel_only_and_keep_ending_and_length() {
        for (target, heard) in [
            ("Temitope", "Tematope"),
            ("Yemisi", "Yemesi"),
            ("Meetumo", "Mitumo"),
            ("Meetumo", "Metumo"),
            ("Folasade", "Folesede"),
        ] {
            assert_eq!(
                corrected(&format!("Ask {heard} now"), &[target]),
                format!("Ask {target} now"),
                "{heard}"
            );
        }
        for (target, heard) in [
            ("Yemisi", "Yemosi"),
            ("Yemisi", "Yamasi"),
            ("Yemisi", "Yemisa"),
            ("Temitope", "Temitopa"),
            ("Folasade", "Folasida"),
            ("Temitope", "Temtope"),
            ("Folasade", "Falisade"),
            ("Temitope", "Temitayo"),
        ] {
            let s = format!("Ask {heard} now");
            assert_eq!(corrected(&s, &[target]), s, "{heard}");
        }
    }

    #[test]
    fn brand_cased_targets_are_recased_even_when_lowercase_is_a_word() {
        assert!(lexicon::is_real_word("github"));
        assert_eq!(
            corrected("push it to github now", &["GitHub"]),
            "push it to GitHub now"
        );
        assert_eq!(
            corrected("the cursor blinks", &["Cursor"]),
            "the cursor blinks"
        );
    }

    #[test]
    fn exact_compound_rule_rewrites_ordinary_phrases_that_spell_a_target() {
        assert_eq!(
            corrected("what is the tail wind speed", &["Tailwind"]),
            "what is the Tailwind speed"
        );
        assert_eq!(
            corrected("I asked open AI about it", &["OpenAI"]),
            "I asked OpenAI about it"
        );
        assert_eq!(
            corrected("go to the back end now", &["Backend"]),
            "go to the Backend now"
        );
    }

    #[test]
    fn diacritics_fold_for_matching() {
        assert_eq!(
            corrected("Ask Olúwaseun now", &["Oluwaseun"]),
            "Ask Oluwaseun now"
        );
    }
}
