//! Learning dictionary words and snippets (Mac `EditCorrectionFinder` and
//! `MemoryLearningService`, with Apple Foundation Models replaced by the local
//! LLM host).
//!
//! - [`corrections`] / [`find_corrections`]: compare a text field right after a
//!   dictation was inserted with the same field a little later and keep only
//!   small, name-like fixes inside the dictated text.
//! - [`heuristic_drafts`]: deterministic suggestions from history — repeated
//!   phrases become snippets (Mac `HeuristicLearningAnalyzer`), and a name the
//!   history spells two ways becomes a dictionary pair.
//! - [`llm_drafts`]: the "AI suggestions" analyzer — the Mac's prompt sent to a
//!   local model, with the same sanitizing so nothing invented gets through.

use super::chars::{edit_distance, normalized_key};
use super::lexicon;
use crate::error::Result;
use crate::llm::{ChatOptions, LlmClient};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

// ---- Suggestion drafts -----------------------------------------------------------

/// What a suggestion would create when accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Dictionary,
    Snippet,
}

impl SuggestionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dictionary => "dictionary",
            Self::Snippet => "snippet",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "snippet" {
            Self::Snippet
        } else {
            Self::Dictionary
        }
    }
}

/// A suggestion before it is stored (Mac `LearningSuggestionDraft`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestionDraft {
    pub kind: SuggestionKind,
    /// Dictionary: what was heard. Snippet: the trigger.
    pub source: String,
    /// Dictionary: how to write it. Snippet: the expansion.
    pub target: String,
    pub confidence: f64,
    pub evidence: Option<String>,
}

/// Evidence prefix for suggestions learned from the user's own fixes; the UI
/// labels those "Your edit".
pub const EDIT_EVIDENCE_PREFIX: &str = "Corrected after dictating";

/// Evidence for a correction learned in `app`.
pub fn edit_evidence(app: Option<&str>) -> String {
    match app {
        Some(app) if !app.trim().is_empty() => {
            format!("{EDIT_EVIDENCE_PREFIX} into {}", app.trim())
        }
        _ => EDIT_EVIDENCE_PREFIX.to_string(),
    }
}

/// Confidence given to a correction the user made by hand.
pub const EDIT_CONFIDENCE: f64 = 0.9;

// ---- Edit checks -------------------------------------------------------------------

/// How one check of a dictation's field ended (Mac `EditCheckOutcome`). Shown
/// in Memory › Suggestions so the person can see the feature at work; never
/// carries any text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditCheckOutcome {
    /// A fixed name or term became a suggestion.
    Learned,
    /// A name or term was fixed, but it is already in the dictionary or was
    /// turned down before.
    AlreadyKnown,
    /// The dictation was edited, but not in a way that teaches a word.
    EditedNoName,
    /// The dictation was left as it was.
    NoChange,
    /// The field could be read, but the dictation wasn't in it.
    TextNotFound,
    /// The app didn't let Orttaai read the field.
    Unreadable,
    /// A password field: never read.
    SecureField,
    /// Learning works on English dictation only.
    NotEnglish,
}

impl EditCheckOutcome {
    pub const ALL: [EditCheckOutcome; 8] = [
        Self::Learned,
        Self::AlreadyKnown,
        Self::EditedNoName,
        Self::NoChange,
        Self::TextNotFound,
        Self::Unreadable,
        Self::SecureField,
        Self::NotEnglish,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learned => "learned",
            Self::AlreadyKnown => "already_known",
            Self::EditedNoName => "edited_no_name",
            Self::NoChange => "no_change",
            Self::TextNotFound => "text_not_found",
            Self::Unreadable => "unreadable",
            Self::SecureField => "secure_field",
            Self::NotEnglish => "not_english",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// The row text in Memory › Suggestions (Mac `MemoryView.description`).
    pub fn description(self, learned: i64) -> String {
        match self {
            Self::Learned if learned == 1 => "Learned a word from your fix".into(),
            Self::Learned => format!("Learned {learned} words from your fixes"),
            Self::AlreadyKnown => "Fixed a word Orttaai already knows".into(),
            Self::EditedNoName => "Edited, but no misheard name to learn".into(),
            Self::NoChange => "Left as dictated".into(),
            Self::TextNotFound => "Couldn't find the dictation in the box".into(),
            Self::Unreadable => "This app didn't let Orttaai read the box".into(),
            Self::SecureField => "Password box: not read".into(),
            Self::NotEnglish => "Learning works on English dictation only".into(),
        }
    }
}

// ---- Edit corrections ----------------------------------------------------------------

/// A word the user fixed by hand after a dictation was inserted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    /// What the dictation wrote.
    pub heard: String,
    /// What the user changed it to.
    pub corrected: String,
}

/// At most this many dictated words replaced by one correction ("for lash a
/// day" → "Folasade").
pub const MAX_HEARD_WORDS: usize = 4;
pub const MAX_CORRECTED_WORDS: usize = 2;
pub const MAX_CORRECTIONS_PER_EDIT: usize = 3;
/// Word lists longer than this are not aligned (a huge rewrite).
const MAX_ALIGNED_WORDS: usize = 400;

#[derive(Debug, Clone)]
struct Word {
    text: String,
    start: usize,
    end: usize,
}

/// Corrections when the field held exactly the dictation (`inserted`) and now
/// holds `edited`.
pub fn find_corrections(inserted: &str, edited: &str) -> Vec<Correction> {
    corrections(
        inserted,
        inserted,
        Some(inserted.encode_utf16().count()),
        edited,
    )
}

/// `before` is the field right after the dictation `inserted` went in, with
/// the caret at `insertion_end_utf16` (the end of the inserted text, in UTF-16
/// units as accessibility APIs report it); `after` is the same field later.
/// Only small replacements inside the dictated text count, the new spelling
/// must look like a name or term and sound like what was heard, and a rewrite
/// of the dictation is ignored.
pub fn corrections(
    inserted: &str,
    raw_before: &str,
    insertion_end_utf16: Option<usize>,
    raw_after: &str,
) -> Vec<Correction> {
    let inserted = comparable(inserted);
    let before: Vec<char> = comparable(raw_before).chars().collect();
    let after: Vec<char> = comparable(raw_after).chars().collect();
    let caret = insertion_end_utf16.map(|caret| {
        let prefix = utf16_prefix(raw_before, caret);
        comparable(prefix).chars().count()
    });
    if before == after {
        return Vec::new();
    }
    let Some((ins_start, ins_end)) = locate(&inserted, &before, caret) else {
        return Vec::new();
    };

    // Only the stretch that differs needs aligning: trim what the two texts
    // share at both ends, but keep the whole dictated text in view.
    let shared_prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let limit = before.len().min(after.len()) - shared_prefix;
    let shared_suffix = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    let window_start = shared_prefix.min(ins_start);
    let kept_suffix = shared_suffix.min(before.len() - ins_end);
    let before_words = words(&before, window_start, before.len() - kept_suffix);
    let after_words = words(&after, window_start, after.len() - kept_suffix);
    if before_words.len() > MAX_ALIGNED_WORDS || after_words.len() > MAX_ALIGNED_WORDS {
        return Vec::new();
    }

    let inserted_word_count = words(&before, ins_start, ins_end).len();
    let mut found = Vec::new();
    let mut changed_inserted_words = 0usize;
    let a: Vec<&str> = before_words.iter().map(|w| w.text.as_str()).collect();
    let b: Vec<&str> = after_words.iter().map(|w| w.text.as_str()).collect();
    for (heard_range, corrected_range) in replacement_blocks(&a, &b) {
        let heard_words = &before_words[heard_range];
        let corrected_words = &after_words[corrected_range];
        let (Some(first), Some(last)) = (heard_words.first(), heard_words.last()) else {
            continue;
        };
        if first.start < ins_start || last.end > ins_end {
            continue;
        }
        changed_inserted_words += heard_words.len();
        if heard_words.len() > MAX_HEARD_WORDS
            || corrected_words.is_empty()
            || corrected_words.len() > MAX_CORRECTED_WORDS
        {
            continue;
        }
        let (cf, cl) = (
            &corrected_words[0],
            &corrected_words[corrected_words.len() - 1],
        );
        let heard: String = before[first.start..last.end].iter().collect();
        let corrected: String = after[cf.start..cl.end].iter().collect();
        let corrected_texts: Vec<&str> = corrected_words.iter().map(|w| w.text.as_str()).collect();
        if is_dictionary_worthy(
            &heard,
            &corrected,
            &corrected_texts,
            heard_words.len() > corrected_words.len(),
            // A fix that starts a new sentence capitalizes for that reason.
            starts_sentence(first.start, &before) || starts_sentence(cf.start, &after),
        ) {
            found.push(Correction { heard, corrected });
        }
    }

    // Rewording more than half of the dictation is a rewrite, not a fix
    // (short dictations still allow one full misheard name).
    if changed_inserted_words > MAX_HEARD_WORDS.max(inserted_word_count / 2) {
        return Vec::new();
    }
    found.truncate(MAX_CORRECTIONS_PER_EDIT);
    found
}

fn utf16_prefix(text: &str, utf16: usize) -> &str {
    let mut units = 0usize;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return &text[..i];
        }
        units += c.len_utf16();
    }
    text
}

fn is_dictionary_worthy(
    heard: &str,
    corrected: &str,
    corrected_words: &[&str],
    joins_words: bool,
    at_sentence_start: bool,
) -> bool {
    let heard_letters = letters(heard);
    let corrected_letters = letters(corrected);
    if corrected_letters.is_empty() || heard == corrected {
        return false;
    }
    // Capitals in mid-sentence mark a name ("tanya" → "Tania", "x code" →
    // "Xcode") even when the word list holds the word.
    let capitalized_name = !at_sentence_start
        && corrected_words
            .iter()
            .all(|w| w.chars().next().is_some_and(char::is_uppercase));
    if heard_letters == corrected_letters {
        // A case-only fix is worth keeping for terms like GitHub or GRDB, and
        // for a name capitalized in mid-sentence, not for capitalizing the
        // start of a sentence or an ordinary word.
        if corrected_words.iter().any(|w| has_inner_uppercase(w)) {
            return true;
        }
        return capitalized_name
            && (joins_words || corrected_words.iter().any(|w| looks_like_term(w)));
    }
    if !(capitalized_name || corrected_words.iter().any(|w| looks_like_term(w))) {
        return false;
    }
    // The fix has to sound like what was heard: a misspelled or misheard
    // name, not a different word chosen on reflection.
    sounds_alike(&heard_letters, &corrected_letters)
}

/// Close in spelling, or the same consonants in the same order ("or tie" →
/// "Orttaai"), which a different word chosen on reflection rarely is ("cat" →
/// "kubectl").
pub fn sounds_alike(heard: &str, corrected: &str) -> bool {
    let (h, c) = (heard.chars().count(), corrected.chars().count());
    if edit_distance(heard, corrected) <= 2.max(h.max(c) / 2) {
        return true;
    }
    let heard_key = consonant_key(heard);
    let corrected_key = consonant_key(corrected);
    let (hk, ck) = (heard_key.chars().count(), corrected_key.chars().count());
    if hk.min(ck) < 2 {
        return false;
    }
    let allowed = usize::from(hk.max(ck) >= 4);
    edit_distance(&heard_key, &corrected_key) <= allowed
}

/// Lowercase letters reduced to their consonant sounds: vowels and
/// silent-prone h/w dropped, letters that sound alike folded, doubles
/// collapsed ("orttaai" → "rt", "kubernetes" → "kbrnts").
pub fn consonant_key(letters: &str) -> String {
    let mut key = String::new();
    let mut previous: Option<char> = None;
    for c in letters.replace("ph", "f").chars() {
        let folded = match c {
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'h' | 'w' => {
                previous = None;
                continue;
            }
            'c' | 'q' => 'k',
            'z' => 's',
            other => other,
        };
        if Some(folded) != previous {
            key.push(folded);
        }
        previous = Some(folded);
    }
    key
}

/// Contractions and possessives are judged by the word they're built on.
const CONTRACTION_ENDINGS: &[&str] = &["t", "re", "ve", "ll", "d", "m", "s"];

/// A name, product or code term rather than an ordinary English word.
fn looks_like_term(word: &str) -> bool {
    let lowered = word.to_lowercase();
    if has_inner_uppercase(word) {
        return true;
    }
    if word.chars().any(char::is_numeric) && word.chars().any(char::is_alphabetic) {
        return true;
    }
    let parts: Vec<&str> = lowered
        .split(['\'', '\u{2019}'])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() == 2 && CONTRACTION_ENDINGS.contains(&parts[1]) {
        // "isn't" splits as "isn" + "t": the base is "is".
        let mut bases = vec![parts[0].to_string()];
        if parts[1] == "t" {
            let mut shorter = parts[0].to_string();
            shorter.pop();
            bases.push(shorter);
        }
        return !bases.iter().any(|b| lexicon::is_real_word(b));
    }
    word.chars().any(char::is_alphabetic) && !lexicon::is_real_word(&lowered)
}

/// Whether the word starting at `index` begins a sentence: the start of the
/// field, or after . ! or ? (skipping spaces and opening quotes).
fn starts_sentence(index: usize, text: &[char]) -> bool {
    for &c in text[..index].iter().rev() {
        if c.is_whitespace() || "\"'([“‘".contains(c) {
            continue;
        }
        return ".!?".contains(c);
    }
    true
}

fn has_inner_uppercase(word: &str) -> bool {
    word.chars().skip(1).any(char::is_uppercase)
}

fn letters(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Every run of whitespace as one plain space (editors keep spaces and line
/// breaks their own way).
pub fn normalized_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// Text as compared: whitespace normalized, and curly quotes as straight
/// ones, since many editors curl them as the text goes in.
pub fn comparable(text: &str) -> String {
    normalized_whitespace(text)
        .chars()
        .map(|c| match c {
            '‘' | '’' => '\'',
            '“' | '”' => '"',
            other => other,
        })
        .collect()
}

fn lower_eq(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Case-insensitive occurrences of `needle` in `hay`, non-overlapping.
fn occurrences(needle: &[char], hay: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if needle
            .iter()
            .zip(&hay[i..i + needle.len()])
            .all(|(a, b)| lower_eq(*a, *b))
        {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Whether a field's text holds the dictation, ignoring how the editor stored
/// its whitespace and quotes, and a capital it added or removed.
pub fn field_contains(text: &str, inserted: &str) -> bool {
    let needle: Vec<char> = comparable(inserted)
        .trim_matches([' ', '\t'])
        .chars()
        .collect();
    if needle.is_empty() {
        return false;
    }
    let hay: Vec<char> = comparable(text).chars().collect();
    !occurrences(&needle, &hay).is_empty()
}

/// The occurrence of `inserted` that ends at, or closest to, the caret.
fn locate(inserted: &str, text: &[char], caret: Option<usize>) -> Option<(usize, usize)> {
    let needle: Vec<char> = inserted.trim().chars().collect();
    let ranges = occurrences(&needle, text);
    match caret {
        None => ranges.last().copied(),
        Some(caret) => ranges
            .into_iter()
            .min_by_key(|(_, end)| end.abs_diff(caret)),
    }
}

fn words(text: &[char], start: usize, end: usize) -> Vec<Word> {
    let mut out = Vec::new();
    let mut i = start;
    let joiner = |c: char| "'’-".contains(c);
    while i < end {
        if !text[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let s = i;
        while i < end && (text[i].is_alphanumeric() || joiner(text[i])) {
            i += 1;
        }
        let mut e = i;
        while e > s && joiner(text[e - 1]) {
            e -= 1;
        }
        out.push(Word {
            text: text[s..e].iter().collect(),
            start: s,
            end: e,
        });
    }
    out
}

type Block = (std::ops::Range<usize>, std::ops::Range<usize>);

/// Runs of words removed from `a` and added in `b` between the words the two
/// lists share (longest common subsequence).
fn replacement_blocks(a: &[&str], b: &[&str]) -> Vec<Block> {
    let mut lengths = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lengths[i][j] = if a[i] == b[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let mut blocks = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut block_start: Option<(usize, usize)> = None;
    let close =
        |blocks: &mut Vec<Block>, start: &mut Option<(usize, usize)>, i: usize, j: usize| {
            if let Some((sa, sb)) = start.take() {
                if i > sa && j > sb {
                    blocks.push((sa..i, sb..j));
                }
            }
        };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            close(&mut blocks, &mut block_start, i, j);
            i += 1;
            j += 1;
        } else {
            if block_start.is_none() {
                block_start = Some((i, j));
            }
            if j < b.len() && (i == a.len() || lengths[i][j + 1] >= lengths[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
    }
    close(&mut blocks, &mut block_start, i, j);
    blocks
}

// ---- History analyzers ----------------------------------------------------------------

/// Name shown for the deterministic analyzer.
pub const HEURISTIC_ANALYZER: &str = "Heuristic Analyzer";

/// Deterministic suggestions from recent history (newest first): repeated
/// dictations become snippets (Mac `HeuristicLearningAnalyzer`), and a name or
/// term the history spells two ways becomes a dictionary pair.
pub fn heuristic_drafts(
    history: &[String],
    existing_dictionary_sources: &[String],
    existing_snippet_triggers: &[String],
) -> Vec<SuggestionDraft> {
    let mut drafts = snippet_drafts(history, existing_snippet_triggers);
    drafts.extend(spelling_variant_drafts(
        history,
        existing_dictionary_sources,
    ));
    drafts
}

fn snippet_drafts(history: &[String], existing_triggers: &[String]) -> Vec<SuggestionDraft> {
    let existing: HashSet<String> = existing_triggers
        .iter()
        .map(|t| normalized_key(t))
        .collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut representative: HashMap<String, String> = HashMap::new();
    for record in history {
        let text = record.trim();
        let len = text.chars().count();
        if !(18..=180).contains(&len) {
            continue;
        }
        let word_count = text.split_whitespace().count();
        if !(4..=20).contains(&word_count) {
            continue;
        }
        let key = normalized_key(text);
        if key.is_empty() {
            continue;
        }
        *counts.entry(key.clone()).or_default() += 1;
        representative
            .entry(key)
            .or_insert_with(|| text.to_string());
    }
    let mut repeated: Vec<(String, usize)> = counts.into_iter().filter(|(_, c)| *c >= 2).collect();
    repeated.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    repeated
        .into_iter()
        .take(8)
        .filter_map(|(key, count)| {
            let expansion = representative.get(&key)?.clone();
            let trigger = suggested_trigger(&key);
            let normalized = normalized_key(&trigger);
            if normalized.is_empty() || existing.contains(&normalized) {
                return None;
            }
            Some(SuggestionDraft {
                kind: SuggestionKind::Snippet,
                source: trigger,
                target: expansion,
                confidence: (0.45 + count as f64 * 0.08).min(0.82),
                evidence: Some(format!("Appeared {count}x in recent dictations.")),
            })
        })
        .collect()
}

fn suggested_trigger(normalized: &str) -> String {
    let words: Vec<&str> = normalized.split(' ').collect();
    if words.is_empty() {
        return normalized.to_string();
    }
    let count = (words.len() / 3).clamp(2, 4).min(words.len());
    words[..count].join(" ")
}

/// A name the user has dictated (and kept) one way at least twice, and that
/// shows up spelled another way that sounds the same: the rarer spelling is
/// most likely the recognizer's mishearing.
fn spelling_variant_drafts(
    history: &[String],
    existing_sources: &[String],
) -> Vec<SuggestionDraft> {
    let existing: HashSet<String> = existing_sources.iter().map(|s| normalized_key(s)).collect();
    // Spelling → count, for words that look like names or terms.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for text in history {
        let chars: Vec<char> = text.chars().collect();
        for w in words(&chars, 0, chars.len()) {
            let n = w.text.chars().count();
            let starts_upper = w.text.chars().next().is_some_and(char::is_uppercase);
            if n < 4 || w.text.chars().any(char::is_numeric) || !starts_upper {
                continue;
            }
            if w.text.contains(['\'', '\u{2019}']) || !looks_like_term(&w.text) {
                continue;
            }
            *counts.entry(w.text).or_default() += 1;
        }
    }
    let spellings: Vec<(String, usize)> = counts.into_iter().collect();
    let mut drafts = Vec::new();
    for (variant, variant_count) in &spellings {
        // The best-attested spelling that sounds like this one.
        let canonical = spellings
            .iter()
            .filter(|(other, other_count)| {
                other != variant
                    && *other_count >= 2
                    && *other_count > *variant_count
                    && other
                        .chars()
                        .next()
                        .map(|c| c.to_lowercase().collect::<String>())
                        == variant
                            .chars()
                            .next()
                            .map(|c| c.to_lowercase().collect::<String>())
                    && letters(other) != letters(variant)
                    && sounds_alike(&letters(variant), &letters(other))
                    && consonant_key(&letters(variant)) == consonant_key(&letters(other))
            })
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        let Some((canonical, canonical_count)) = canonical else {
            continue;
        };
        if existing.contains(&normalized_key(variant)) {
            continue;
        }
        drafts.push(SuggestionDraft {
            kind: SuggestionKind::Dictionary,
            source: variant.clone(),
            target: canonical.clone(),
            confidence: (0.4 + *canonical_count as f64 * 0.1).min(0.8),
            evidence: Some(format!(
                "Written as {canonical} {canonical_count}x and as {variant} {variant_count}x in recent dictations."
            )),
        });
    }
    drafts.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source.cmp(&b.source))
    });
    drafts.truncate(8);
    drafts
}

/// The JSON the AI analyzer must return.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AiLearningResponse {
    #[serde(default)]
    pub dictionary: Vec<AiDictionarySuggestion>,
    #[serde(default)]
    pub snippets: Vec<AiSnippetSuggestion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiDictionarySuggestion {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSnippetSuggestion {
    pub trigger: String,
    pub expansion: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Option<String>,
}

/// The Mac analyzer prompt (history newest first, at most 120 lines of 220
/// characters).
pub fn ai_prompt(
    history: &[String],
    existing_dictionary_sources: &[String],
    existing_snippet_triggers: &[String],
) -> Option<String> {
    let lines: Vec<String> = history
        .iter()
        .take(120)
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| {
            if l.chars().count() > 220 {
                format!("{}...", l.chars().take(220).collect::<String>())
            } else {
                l.to_string()
            }
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    let numbered: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("[{}] {l}", i + 1))
        .collect();
    Some(format!(
        r#"You are helping build a personal on-device dictation assistant memory.
Analyze the history and extract reusable suggestions.

Return ONLY valid JSON with this exact shape:
{{
  "dictionary": [
    {{"source": "otta ai", "target": "Orttaai", "confidence": 0.8, "evidence": "Written as otta ai in 3 dictations"}}
  ],
  "snippets": [
    {{"trigger": "my address", "expansion": "12 Harbour Road, Lagos", "confidence": 0.7, "evidence": "Dictated word for word 4 times"}}
  ]
}}

Rules:
- Write the values only. Never put labels such as "Misheard term:" inside a value.
- dictionary.source must be copied exactly from the history: a name or term the speech recognizer got wrong.
- dictionary.target is how it should be spelled, and must sound like the source. Never pair words that sound different, and never expand an acronym.
- snippets.expansion must be text that appears word for word in the history more than once; the trigger is 2 to 4 words.
- confidence must be between 0 and 1.
- avoid duplicates with existing entries.
- keep output concise; maximum 8 dictionary items and 8 snippets. Return empty lists when nothing qualifies.

Existing dictionary terms:
{}

Existing snippet triggers:
{}

History:
{}"#,
        existing_dictionary_sources.join(", "),
        existing_snippet_triggers.join(", "),
        numbered.join("\n")
    ))
}

/// JSON schema for structured output on hosts that support it.
pub fn ai_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "dictionary": {"type": "array", "items": {"type": "object", "properties": {
                "source": {"type": "string"}, "target": {"type": "string"},
                "confidence": {"type": "number"}, "evidence": {"type": "string"}},
                "required": ["source", "target", "confidence"]}},
            "snippets": {"type": "array", "items": {"type": "object", "properties": {
                "trigger": {"type": "string"}, "expansion": {"type": "string"},
                "confidence": {"type": "number"}, "evidence": {"type": "string"}},
                "required": ["trigger", "expansion", "confidence"]}}
        },
        "required": ["dictionary", "snippets"]
    })
}

/// Asks the local model for suggestions and keeps only those that survive
/// [`sanitize`]. An empty history yields no drafts and no request.
pub fn llm_drafts(
    llm: &dyn LlmClient,
    model: &str,
    history: &[String],
    existing_dictionary_sources: &[String],
    existing_snippet_triggers: &[String],
) -> Result<Vec<SuggestionDraft>> {
    let Some(prompt) = ai_prompt(
        history,
        existing_dictionary_sources,
        existing_snippet_triggers,
    ) else {
        return Ok(Vec::new());
    };
    let opts = ChatOptions {
        model: model.to_string(),
        temperature: Some(0.2),
        think: Some(false),
        json_schema: Some(ai_response_schema()),
        timeout_ms: Some(120_000),
        ..Default::default()
    };
    let reply = llm.complete("", &prompt, &opts)?;
    let parsed: AiLearningResponse = crate::llm::parse_json_reply(&reply)?;
    Ok(sanitize(
        &parsed,
        history,
        existing_dictionary_sources,
        existing_snippet_triggers,
    ))
}

const LABEL_PREFIXES: &[&str] = &[
    "misheard term",
    "preferred term",
    "short trigger",
    "expanded text",
    "source",
    "target",
    "trigger",
    "expansion",
    "heard as",
    "suggested",
];

/// Strips a label the model copied into a value ("Misheard term: X") and
/// surrounding quotes.
pub fn strip_label(value: &str) -> String {
    let mut text = value.trim();
    let lower = text.to_lowercase();
    for label in LABEL_PREFIXES {
        if !lower.starts_with(label) || !text.is_char_boundary(label.len()) {
            continue;
        }
        let rest = text[label.len()..].trim_start();
        if let Some(stripped) = rest.strip_prefix([':', '-', '–']) {
            text = stripped.trim_start();
            break;
        }
    }
    text.trim_matches(|c: char| c.is_whitespace() || "\"'“”‘’".contains(c))
        .to_string()
}

/// A dictionary pair is kept only when the heard form was actually said and
/// the replacement sounds like it: a misspelled name, not a different word
/// or an expanded acronym.
pub fn is_plausible_dictionary_pair(source: &str, target: &str, history: &str) -> bool {
    if !history.contains(&normalized_whitespace(&source.to_lowercase())) {
        return false;
    }
    let heard = letters(source);
    let wanted = letters(target);
    if heard.is_empty() || wanted.is_empty() || (heard == wanted && source == target) {
        return false;
    }
    let distance = edit_distance(&heard, &wanted);
    distance <= 2.max(heard.chars().count().max(wanted.chars().count()) / 2)
}

/// Keeps model suggestions that were really said (dictionary) or really
/// repeated (snippets), and that don't duplicate existing entries.
pub fn sanitize(
    parsed: &AiLearningResponse,
    history: &[String],
    existing_dictionary_sources: &[String],
    existing_snippet_triggers: &[String],
) -> Vec<SuggestionDraft> {
    let history_text = normalized_whitespace(&history.join(" \n ").to_lowercase());
    let existing_sources: HashSet<String> = existing_dictionary_sources
        .iter()
        .map(|s| normalized_key(s))
        .collect();
    let existing_triggers: HashSet<String> = existing_snippet_triggers
        .iter()
        .map(|s| normalized_key(s))
        .collect();
    let mut drafts = Vec::new();
    for item in parsed.dictionary.iter().take(8) {
        let source = strip_label(&item.source);
        let target = strip_label(&item.target);
        if source.is_empty() || target.is_empty() || source == target {
            continue;
        }
        if existing_sources.contains(&normalized_key(&source)) {
            continue;
        }
        if !is_plausible_dictionary_pair(&source, &target, &history_text) {
            continue;
        }
        drafts.push(SuggestionDraft {
            kind: SuggestionKind::Dictionary,
            source,
            target,
            confidence: item.confidence.clamp(0.0, 1.0),
            evidence: item.evidence.clone(),
        });
    }
    for item in parsed.snippets.iter().take(8) {
        let trigger = strip_label(&item.trigger);
        let expansion = strip_label(&item.expansion);
        if trigger.is_empty() || expansion.is_empty() || trigger == expansion {
            continue;
        }
        // A snippet is for text the person really repeats, never one the
        // model wrote itself.
        if trigger.split(' ').count() > 5
            || !history_text.contains(&normalized_whitespace(&expansion.to_lowercase()))
        {
            continue;
        }
        if existing_triggers.contains(&normalized_key(&trigger)) {
            continue;
        }
        drafts.push(SuggestionDraft {
            kind: SuggestionKind::Snippet,
            source: trigger,
            target: expansion,
            confidence: item.confidence.clamp(0.0, 1.0),
            evidence: item.evidence.clone(),
        });
    }
    drafts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(h: &str, c: &str) -> Correction {
        Correction {
            heard: h.into(),
            corrected: c.into(),
        }
    }

    fn fixes(prefix: &str, inserted: &str, suffix: &str, edited: &str) -> Vec<Correction> {
        let before = format!("{prefix}{inserted}{suffix}");
        let caret = format!("{prefix}{inserted}").encode_utf16().count();
        corrections(inserted, &before, Some(caret), edited)
    }

    fn simple(inserted: &str, edited: &str) -> Vec<Correction> {
        fixes("", inserted, "", edited)
    }

    #[test]
    fn misheard_names_and_terms_are_learned() {
        assert_eq!(
            simple(
                "Please remind me to call for Lash a day tomorrow.",
                "Please remind me to call Folasade tomorrow."
            ),
            vec![pair("for Lash a day", "Folasade")]
        );
        assert_eq!(
            simple(
                "Olen Ruij is presenting the roadmap.",
                "Olanrewaju is presenting the roadmap."
            ),
            vec![pair("Olen Ruij", "Olanrewaju")]
        );
        assert_eq!(
            simple(
                "Dear Mr. Adayemi, the invoice is overdue.",
                "Dear Mr. Adeyemi, the invoice is overdue."
            ),
            vec![pair("Adayemi", "Adeyemi")]
        );
        assert_eq!(
            simple(
                "grab migrations run inside a write transaction.",
                "GRDB migrations run inside a write transaction."
            ),
            vec![pair("grab", "GRDB")]
        );
        assert_eq!(
            simple(
                "Build or tie for the release.",
                "Build Orttaai for the release."
            ),
            vec![pair("or tie", "Orttaai")]
        );
    }

    #[test]
    fn case_fixes_count_only_for_terms_with_inner_capitals() {
        assert_eq!(
            simple(
                "push the branch to github today",
                "push the branch to GitHub today"
            ),
            vec![pair("github", "GitHub")]
        );
        assert_eq!(simple("hello there, team", "Hello there, team"), vec![]);
    }

    #[test]
    fn ordinary_edits_are_ignored() {
        assert_eq!(
            simple("It was a big release.", "It was a large release."),
            vec![]
        );
        assert_eq!(
            simple("Use the cat command here.", "Use the kubectl command here."),
            vec![]
        );
        assert_eq!(
            simple(
                "We should meet Adayemi on Friday.",
                "Let's push the Okonkwo meeting to Monday."
            ),
            vec![]
        );
        assert_eq!(simple("Ship it on Friday.", "Ship it on Friday."), vec![]);
        assert_eq!(
            simple(
                "I think their going to review it.",
                "I think they're going to review it."
            ),
            vec![]
        );
        assert_eq!(
            simple("It is not ready yet.", "It isn't ready yet."),
            vec![]
        );
    }

    #[test]
    fn only_edits_inside_the_dictation_count() {
        assert_eq!(
            simple("Thanks Sarah.", "Thanks Sarah. Talk soon, Olanrewaju"),
            vec![]
        );
        assert_eq!(
            simple(
                "Ask Adayemi about the deck.",
                "Ask Adeyemi about the deck. Thanks!"
            ),
            vec![pair("Adayemi", "Adeyemi")]
        );
        assert_eq!(
            fixes(
                "Dear Jon, ",
                "the report is attached.",
                "",
                "Dear Jonathon, the report is attached."
            ),
            vec![]
        );
    }

    #[test]
    fn the_occurrence_at_the_caret_is_the_dictation() {
        let earlier = "Call Adayemi. ";
        let inserted = "Call Adayemi.";
        let before = format!("{earlier}{inserted}");
        let caret = before.encode_utf16().count();
        assert_eq!(
            corrections(
                inserted,
                &before,
                Some(caret),
                &format!("{earlier}Call Adeyemi.")
            ),
            vec![pair("Adayemi", "Adeyemi")]
        );
        assert_eq!(
            corrections(
                inserted,
                &before,
                Some(caret),
                &format!("Call Adeyemi. {inserted}")
            ),
            vec![]
        );
    }

    #[test]
    fn caret_is_measured_in_utf16() {
        assert_eq!(
            fixes(
                "🎉🎉 Great news! ",
                "Folashade joined the team.",
                "",
                "🎉🎉 Great news! Folasade joined the team."
            ),
            vec![pair("Folashade", "Folasade")]
        );
    }

    #[test]
    fn names_capitalized_mid_sentence_are_learned() {
        assert_eq!(
            simple(
                "Send it to ola in lagos tomorrow.",
                "Send it to Ola in Lagos tomorrow."
            ),
            vec![pair("ola", "Ola"), pair("lagos", "Lagos")]
        );
        assert_eq!(
            simple("Open x code and build it.", "Open Xcode and build it."),
            vec![pair("x code", "Xcode")]
        );
        assert_eq!(
            simple("I met with Tanya today.", "I met with Tania today."),
            vec![pair("Tanya", "Tania")]
        );
        assert_eq!(
            simple(
                "Please deploy the cooper netties cluster.",
                "Please deploy the Kubernetes cluster."
            ),
            vec![pair("cooper netties", "Kubernetes")]
        );
    }

    #[test]
    fn capitals_that_are_not_names_are_ignored() {
        assert_eq!(
            simple("Meet me at the park later.", "Meet me at the Park later."),
            vec![]
        );
        assert_eq!(
            simple(
                "We shipped it yesterday and the team loved it.",
                "We shipped it yesterday. The team loved it."
            ),
            vec![]
        );
        assert_eq!(simple("lagos is hot today.", "Lagos is hot today."), vec![]);
    }

    #[test]
    fn sound_keys() {
        assert_eq!(consonant_key("orttaai"), "rt");
        assert_eq!(consonant_key("kubernetes"), "kbrnts");
        assert_eq!(consonant_key("philip"), "flp");
        assert!(sounds_alike("ortie", "orttaai"));
        assert!(!sounds_alike("cat", "kubectl"));
    }

    #[test]
    fn dictation_is_found_though_the_editor_capitalized_and_curled_it() {
        let inserted = "ask Adayemi about the client's deck.";
        let before = "Ask Adayemi about the client’s deck.";
        assert_eq!(
            corrections(
                inserted,
                before,
                Some(before.encode_utf16().count()),
                "Ask Adeyemi about the client’s deck."
            ),
            vec![pair("Adayemi", "Adeyemi")]
        );
        assert_eq!(
            corrections(
                "Call Adayemi.",
                "Something else entirely.",
                Some(24),
                "Call Adeyemi."
            ),
            vec![]
        );
        assert!(field_contains(
            "  Ask   Adayemi’s deck ",
            "ask adayemi's deck"
        ));
    }

    #[test]
    fn find_corrections_compares_the_dictation_with_the_edit() {
        assert_eq!(
            find_corrections("Ask Adayemi today.", "Ask Adeyemi today."),
            vec![pair("Adayemi", "Adeyemi")]
        );
    }

    #[test]
    fn every_outcome_has_words() {
        for o in EditCheckOutcome::ALL {
            assert!(!o.description(1).is_empty());
            assert_eq!(EditCheckOutcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(
            EditCheckOutcome::Learned.description(2),
            "Learned 2 words from your fixes"
        );
    }

    #[test]
    fn heuristic_suggests_repeated_dictations_as_snippets() {
        let line = "Thanks so much for your help, talk soon".to_string();
        let history = vec![line.clone(), "short".into(), line.clone(), line.clone()];
        let drafts = heuristic_drafts(&history, &[], &[]);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, SuggestionKind::Snippet);
        assert_eq!(drafts[0].source, "thanks so");
        assert_eq!(drafts[0].target, line);
        assert!((drafts[0].confidence - 0.69).abs() < 1e-9);
        assert!(heuristic_drafts(&history, &[], &["Thanks  so".into()]).is_empty());
    }

    #[test]
    fn heuristic_suggests_the_better_attested_spelling_of_a_name() {
        let history: Vec<String> = [
            "Ask Folasade to send the deck.",
            "Folasade joined the call.",
            "Remind Folashade about Friday.",
            "Meet Temitope and Folasade later.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let drafts = heuristic_drafts(&history, &[], &[]);
        assert_eq!(drafts.len(), 1, "{drafts:?}");
        assert_eq!(drafts[0].kind, SuggestionKind::Dictionary);
        assert_eq!(drafts[0].source, "Folashade");
        assert_eq!(drafts[0].target, "Folasade");
        assert!(heuristic_drafts(&history, &["folashade".into()], &[]).is_empty());
    }

    #[test]
    fn ai_labels_are_stripped() {
        assert_eq!(strip_label("Misheard term: VASEL"), "VASEL");
        assert_eq!(strip_label("Preferred term: \"Superbase\""), "Superbase");
        assert_eq!(strip_label("Short trigger: my address"), "my address");
        assert_eq!(strip_label("Orttaai"), "Orttaai");
    }

    #[test]
    fn ai_suggestions_must_have_been_said_and_sound_alike() {
        let parsed: AiLearningResponse = serde_json::from_value(serde_json::json!({
            "dictionary": [
                {"source": "Misheard term: otta ai", "target": "Preferred term: Orttaai", "confidence": 0.8},
                {"source": "ASNA", "target": "Superbase", "confidence": 0.7},
                {"source": "VASEL", "target": "Virtual Asset Exchange", "confidence": 0.8},
                {"source": "Kubernets", "target": "Kubernetes", "confidence": 0.6}
            ],
            "snippets": [
                {"trigger": "system delay", "expansion": "troubleshooting guide for intermittent delays", "confidence": 0.9},
                {"trigger": "sign off", "expansion": "Thanks so much, talk soon", "confidence": 0.7}
            ]
        }))
        .unwrap();
        let history: Vec<String> = [
            "I was testing the fastest model ever in otta ai today.",
            "ASNA and VASEL came up in the meeting. Thanks so much, talk soon",
            "Thanks so much, talk soon",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let drafts = sanitize(&parsed, &history, &[], &[]);
        let dict: Vec<(&str, &str)> = drafts
            .iter()
            .filter(|d| d.kind == SuggestionKind::Dictionary)
            .map(|d| (d.source.as_str(), d.target.as_str()))
            .collect();
        assert_eq!(dict, vec![("otta ai", "Orttaai")]);
        let snips: Vec<&str> = drafts
            .iter()
            .filter(|d| d.kind == SuggestionKind::Snippet)
            .map(|d| d.target.as_str())
            .collect();
        assert_eq!(snips, vec!["Thanks so much, talk soon"]);
    }

    #[test]
    fn llm_drafts_parse_the_model_reply() {
        let reply = r#"```json
{"dictionary":[{"source":"otta ai","target":"Orttaai","confidence":0.8}],"snippets":[]}
```"#;
        let llm = crate::llm::MockLlm::fixed(reply);
        let history = vec!["I love otta ai.".to_string()];
        let drafts = llm_drafts(&llm, "m", &history, &[], &[]).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].target, "Orttaai");
        assert_eq!(llm.call_count(), 1);
        assert!(llm_drafts(&llm, "m", &[], &[], &[]).unwrap().is_empty());
        assert_eq!(llm.call_count(), 1);
    }
}
