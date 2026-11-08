//! The counts, from what was actually said (the transcript before Polish):
//! um and uh, the phrases that fill a pause, pace, sentence length and words
//! said twice. Counted, never guessed. A faithful port of the Mac's
//! `CoachCounting` (itself Orttakey's `CoachStats`), so every app agrees.
//!
//! The Mac uses ICU regular expressions with lookaround and a backreference;
//! the `regex` crate has neither, so those three rules are matched by hand
//! with exactly the same semantics (see the tests, which pin every case).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Below this a dictation is too short to tell a pace from.
pub const MINIMUM_WORDS_FOR_PACE: usize = 10;
pub const MINIMUM_MS_FOR_PACE: i64 = 5_000;

fn um_uh_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // um, uh and their stretched spellings (umm, uhh, erm, hmm).
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:u+m+|u+h+m*|e+r+m+|h+m+)\b").expect("um/uh pattern"))
}

fn word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\w+").expect("word pattern"))
}

fn simple_phrase(word: &str) -> &'static Regex {
    static YOU_KNOW: OnceLock<Regex> = OnceLock::new();
    static I_MEAN: OnceLock<Regex> = OnceLock::new();
    static BASICALLY: OnceLock<Regex> = OnceLock::new();
    static ACTUALLY: OnceLock<Regex> = OnceLock::new();
    static LIKE: OnceLock<Regex> = OnceLock::new();
    static QUESTION_AFTER: OnceLock<Regex> = OnceLock::new();
    let (cell, pattern) = match word {
        "you know" => (&YOU_KNOW, r"(?i)\byou know\b"),
        "I mean" => (&I_MEAN, r"(?i)\bI mean\b"),
        "basically" => (&BASICALLY, r"(?i)\bbasically\b"),
        "actually" => (&ACTUALLY, r"(?i)\bactually\b"),
        "like" => (&LIKE, r"(?i)like"),
        _ => (
            &QUESTION_AFTER,
            r"(?i)^\s+(?:what|how|that|why|when|where|who|if)\b",
        ),
    };
    cell.get_or_init(|| Regex::new(pattern).expect("phrase pattern"))
}

/// The phrase fillers, in the Mac's order.
pub const PHRASES: [&str; 5] = ["you know", "I mean", "like", "basically", "actually"];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || unicode_mark_or_connector(c)
}

/// `\w` in Unicode also takes combining marks and connector punctuation.
fn unicode_mark_or_connector(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{203F}' | '\u{2040}' | '\u{2054}' | '\u{FE33}' | '\u{FE34}' | '\u{FE4D}'..='\u{FE4F}' | '\u{FF3F}')
}

/// Byte ranges of each match of a phrase filler, as ICU would find them.
fn phrase_matches(word: &str, text: &str) -> Vec<(usize, usize)> {
    match word {
        // "you know" unless a question word follows it.
        "you know" => {
            let after = simple_phrase("?");
            simple_phrase("you know")
                .find_iter(text)
                .filter(|m| !after.is_match(&text[m.end()..]))
                .map(|m| (m.start(), m.end()))
                .collect()
        }
        // "like" only beside a comma: `(?<=,\s)like\b|\blike(?=,)`.
        "like" => simple_phrase("like")
            .find_iter(text)
            .filter(|m| {
                let before = &text[..m.start()];
                let after = &text[m.end()..];
                let next = after.chars().next();
                let mut back = before.chars().rev();
                let prev = back.clone().next();
                let after_comma_space =
                    matches!(back.next(), Some(c) if c.is_whitespace()) && back.next() == Some(',');
                let boundary_after = next.is_none_or(|c| !is_word_char(c));
                let boundary_before = prev.is_none_or(|c| !is_word_char(c));
                (after_comma_space && boundary_after) || (boundary_before && next == Some(','))
            })
            .map(|m| (m.start(), m.end()))
            .collect(),
        other => simple_phrase(other)
            .find_iter(text)
            .map(|m| (m.start(), m.end()))
            .collect(),
    }
}

/// Each `\b(\w+)[\s,]+\1\b` match (case-insensitive): the whole match and the
/// first word's range.
fn repeat_matches(text: &str) -> Vec<((usize, usize), (usize, usize))> {
    let words: Vec<(usize, usize)> = word_re()
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < words.len() {
        let (a0, a1) = words[i];
        let (b0, b1) = words[i + 1];
        let gap = &text[a1..b0];
        let joined = !gap.is_empty() && gap.chars().all(|c| c.is_whitespace() || c == ',');
        if joined && text[a0..a1].to_lowercase() == text[b0..b1].to_lowercase() {
            found.push(((a0, b1), (a0, a1)));
            i += 2;
        } else {
            i += 1;
        }
    }
    found
}

/// Each filler found and how often: "um", "uh", then the phrases.
pub fn filler_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in um_uh_re().find_iter(text) {
        let word = m.as_str().to_lowercase();
        let key = if word.starts_with('u') && word.contains('h') {
            "uh"
        } else {
            "um"
        };
        *counts.entry(key.to_string()).or_default() += 1;
    }
    for word in PHRASES {
        let found = phrase_matches(word, text).len();
        if found > 0 {
            *counts.entry(word.to_string()).or_default() += found;
        }
    }
    counts
}

pub fn um_uh_count(text: &str) -> usize {
    filler_counts(text)
        .iter()
        .filter(|(k, _)| k.as_str() == "um" || k.as_str() == "uh")
        .map(|(_, v)| v)
        .sum()
}

pub fn phrase_filler_count(text: &str) -> usize {
    filler_counts(text)
        .iter()
        .filter(|(k, _)| k.as_str() != "um" && k.as_str() != "uh")
        .map(|(_, v)| v)
        .sum()
}

/// The same word twice in a row ("I I think").
pub fn repeat_count(text: &str) -> usize {
    repeat_matches(text).len()
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Words a minute, or `None` when the dictation is too short to tell.
pub fn pace(text: &str, recording_ms: i64) -> Option<i64> {
    let words = word_count(text);
    if words < MINIMUM_WORDS_FOR_PACE || recording_ms < MINIMUM_MS_FOR_PACE {
        return None;
    }
    Some((words as f64 / (recording_ms as f64 / 60_000.0)).round() as i64)
}

/// Swift's `CharacterSet.whitespaces`: space separators (Zs) and the tab, not newlines.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

fn trim_spaces(s: &str) -> String {
    s.trim_matches(is_space).to_string()
}

/// Sentences as written, each keeping its closing punctuation. Orttakey's
/// Paragrapher: "3.5" or "e.g. this" don't end a sentence; a capital, digit,
/// quote or bracket after a space does.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        current.push(c);
        if !".?!".contains(c) {
            continue;
        }
        let next = chars.get(index + 1).copied();
        if next.is_none() || next == Some(' ') {
            let after = chars[index + 1..].iter().copied().find(|&x| x != ' ');
            let ends = match after {
                None => true,
                Some(a) => a.is_uppercase() || a.is_numeric() || "\"“'(".contains(a),
            };
            if ends {
                result.push(trim_spaces(&current));
                current.clear();
            }
        }
    }
    let rest = trim_spaces(&current);
    if !rest.is_empty() {
        result.push(rest);
    }
    result
}

/// Word counts of each sentence, empty ones left out.
pub fn sentence_lengths(text: &str) -> Vec<i64> {
    sentences(text)
        .iter()
        .map(|s| word_count(s) as i64)
        .filter(|&n| n > 0)
        .collect()
}

/// How much of what was said Polish or the dictionary rewrote, 0 to 1: a
/// word-level edit distance between what was heard and what was written,
/// after taking out the fillers and repeats the cleaner removes anyway.
pub fn polish_change_ratio(heard: &str, written: &str) -> f64 {
    let said = comparable_words(&without_disfluencies(heard));
    let kept = comparable_words(written);
    let longest = said.len().max(kept.len());
    if longest == 0 {
        return 0.0;
    }
    edit_distance(&said, &kept) as f64 / longest as f64
}

fn replace_ranges(text: &str, ranges: &[(usize, usize)], with: impl Fn(usize) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, &(a, b)) in ranges.iter().enumerate() {
        out.push_str(&text[last..a]);
        out.push_str(&with(i));
        last = b;
    }
    out.push_str(&text[last..]);
    out
}

pub(crate) fn without_disfluencies(text: &str) -> String {
    let ranges: Vec<(usize, usize)> = um_uh_re()
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();
    let mut result = replace_ranges(text, &ranges, |_| " ".to_string());
    for word in PHRASES {
        let ranges = phrase_matches(word, &result);
        result = replace_ranges(&result, &ranges, |_| " ".to_string());
    }
    let repeats = repeat_matches(&result);
    let whole: Vec<(usize, usize)> = repeats.iter().map(|r| r.0).collect();
    let firsts: Vec<String> = repeats
        .iter()
        .map(|r| result[r.1 .0..r.1 .1].to_string())
        .collect();
    replace_ranges(&result, &whole, |i| firsts[i].clone())
}

/// Lowercased words without punctuation, so "Hello," and "hello" match.
pub(crate) fn comparable_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphabetic() || c.is_numeric() || c == '\'' || c == '’'))
        .filter(|s| !s.is_empty())
        .take(2_000)
        .map(str::to_string)
        .collect()
}

pub(crate) fn edit_distance(a: &[String], b: &[String]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let substitution = previous[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1).min(current[j - 1] + 1).min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// One dictation, as Coach needs it.
#[derive(Debug, Clone)]
pub struct CoachDictation {
    pub created_at: i64,
    /// What the speech model heard, before Polish: fillers and all.
    pub heard: String,
    /// What landed in the app.
    pub written: String,
    pub recording_ms: i64,
    /// False on rows where only the polished text was kept, whose fillers,
    /// repeats and sentences can't be told.
    pub raw_known: bool,
}

/// The counts for one dictation. Numbers only: no text is kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictationMeasures {
    pub words: i64,
    pub um_uh: i64,
    pub phrase_fillers: i64,
    pub wpm: Option<i64>,
    pub sentence_lengths: Vec<i64>,
    pub repeats: i64,
    /// `None` when what was heard isn't known.
    pub polish_change_ratio: Option<f64>,
    pub raw_known: bool,
}

impl DictationMeasures {
    /// Bumped whenever a rule changes, so stored counts are redone.
    pub const VERSION: i64 = 1;

    pub fn measuring(d: &CoachDictation) -> Self {
        let heard = d.heard.as_str();
        Self {
            words: word_count(heard) as i64,
            um_uh: um_uh_count(heard) as i64,
            phrase_fillers: phrase_filler_count(heard) as i64,
            wpm: pace(heard, d.recording_ms),
            sentence_lengths: sentence_lengths(heard),
            repeats: repeat_count(heard) as i64,
            polish_change_ratio: d.raw_known.then(|| polish_change_ratio(heard, &d.written)),
            raw_known: d.raw_known,
        }
    }

    /// The upper middle length, as Orttakey takes it.
    pub fn median_sentence(&self) -> Option<i64> {
        let mut sorted = self.sentence_lengths.clone();
        sorted.sort_unstable();
        sorted.get(sorted.len() / 2).copied()
    }

    pub fn longest_sentence(&self) -> Option<i64> {
        self.sentence_lengths.iter().copied().max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn um_and_uh_including_stretched_spellings() {
        let counts = filler_counts("Um, so umm I think uh the uhh thing, erm, hmm, uhm works");
        // "erm" and "hmm" file as um; "uhm" files as uh.
        assert_eq!(counts.get("um"), Some(&4));
        assert_eq!(counts.get("uh"), Some(&3));
        assert_eq!(um_uh_count("Umbrella and hum don't count, nor huh"), 0);
    }

    #[test]
    fn you_know_unless_a_question_follows() {
        assert_eq!(
            filler_counts("It was, you know, fine").get("you know"),
            Some(&1)
        );
        assert_eq!(
            filler_counts("You know what I did? You know how it goes").get("you know"),
            None
        );
        assert_eq!(
            filler_counts("you know that, you know").get("you know"),
            Some(&1)
        );
    }

    #[test]
    fn like_only_beside_a_comma() {
        assert_eq!(
            filler_counts("It was, like, really big").get("like"),
            Some(&1)
        );
        assert_eq!(
            filler_counts("It was, like big, like, you see").get("like"),
            Some(&2)
        );
        assert_eq!(
            filler_counts("I like it and it looks like rain").get("like"),
            None
        );
        assert_eq!(filler_counts("so, likely not").get("like"), None);
        assert_eq!(filler_counts("unlike, that").get("like"), None);
    }

    #[test]
    fn other_phrases() {
        let text = "I mean it basically works. Actually it does, I MEAN it.";
        let counts = filler_counts(text);
        assert_eq!(counts.get("I mean"), Some(&2));
        assert_eq!(counts.get("basically"), Some(&1));
        assert_eq!(counts.get("actually"), Some(&1));
        assert_eq!(phrase_filler_count(text), 4);
    }

    #[test]
    fn repeats_count_words_said_twice() {
        assert_eq!(repeat_count("I I think the the plan works"), 2);
        assert_eq!(repeat_count("We had had enough"), 1);
        assert_eq!(repeat_count("so, so good"), 1);
        assert_eq!(repeat_count("this is fine"), 0);
        assert_eq!(repeat_count("The the"), 1);
        assert_eq!(repeat_count("the the the"), 1);
    }

    #[test]
    fn words_are_whitespace_tokens() {
        assert_eq!(word_count("  one two\nthree\tfour  "), 4);
        assert_eq!(word_count(""), 0);
    }

    #[test]
    fn pace_needs_ten_words_and_five_seconds() {
        let ten = ["word"; 10].join(" ");
        assert_eq!(pace(&ten, 4_999), None);
        assert_eq!(pace("too short", 60_000), None);
        assert_eq!(pace(&ten, 5_000), Some(120));
        let thirty = ["word"; 30].join(" ");
        assert_eq!(pace(&thirty, 12_000), Some(150));
    }

    #[test]
    fn sentences_split_like_paragrapher() {
        assert_eq!(
            sentences("It costs 3.5 dollars. Then we go! Why? e.g. this stays."),
            vec![
                "It costs 3.5 dollars.",
                "Then we go!",
                "Why? e.g. this stays."
            ]
        );
        // A quote may open the next sentence, but a closing quote or bracket
        // right after the stop keeps the sentence going, as in Orttakey.
        assert_eq!(
            sentences("He said. \"Fine.\" (Later.) 4 more"),
            vec!["He said.", "\"Fine.\" (Later.) 4 more"]
        );
        assert_eq!(sentence_lengths("One two three. Four five."), vec![3, 2]);
        assert!(sentence_lengths("").is_empty());
    }

    #[test]
    fn polish_fixes_ignore_removed_fillers() {
        // Only the fillers went: nothing was rewritten.
        assert_eq!(
            polish_change_ratio("Um so I I think, like, it works", "So I think it works."),
            0.0
        );
        // Two of six words changed.
        let ratio = polish_change_ratio(
            "send the file to sam today",
            "Send the report to Sam tomorrow.",
        );
        assert!((ratio - 2.0 / 6.0).abs() < 0.0001);
        assert_eq!(polish_change_ratio("", ""), 0.0);
    }

    #[test]
    fn measures_a_known_raw_transcript() {
        let m = DictationMeasures::measuring(&CoachDictation {
            created_at: 0,
            heard: "Um so the the plan is basically done. We ship it on Friday, you know, after review."
                .into(),
            written: "The plan is done. We ship it on Friday after review.".into(),
            recording_ms: 6_000,
            raw_known: true,
        });
        assert_eq!(m.words, 17);
        assert_eq!(m.um_uh, 1);
        assert_eq!(m.phrase_fillers, 2);
        assert_eq!(m.repeats, 1);
        assert_eq!(m.wpm, Some(170));
        assert_eq!(m.sentence_lengths, vec![8, 9]);
        assert_eq!(m.median_sentence(), Some(9));
        assert_eq!(m.longest_sentence(), Some(9));
        assert!(m.polish_change_ratio.is_some());
    }

    #[test]
    fn round_trips_as_json() {
        let m = DictationMeasures {
            words: 20,
            um_uh: 1,
            phrase_fillers: 0,
            wpm: None,
            sentence_lengths: vec![12, 8],
            repeats: 0,
            polish_change_ratio: Some(0.1),
            raw_known: true,
        };
        let back: DictationMeasures =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
