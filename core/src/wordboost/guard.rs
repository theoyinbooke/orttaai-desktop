//! Decides which Word Boost replacements to keep: a port of the Mac's
//! `WordBoostGuard`. The CTC rescorer proposes replacing a heard phrase with a
//! dictionary term when the term sounds close enough; on the Mac's gauntlet
//! corpora one in five of those proposals turned an ordinary word into a
//! product name (code -> Xcode, release -> camelCase). These rules were fitted
//! on 134 applied proposals: they keep 103 of 104 correct ones and none of the
//! 27 wrong ones.

use crate::text::lexicon;

/// One proposal from the rescorer.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate<'a> {
    /// The phrase the speech model heard, as it appears in the transcript.
    pub heard: &'a str,
    pub term: &'a str,
    /// CTC log-probability of the term minus that of the heard phrase.
    pub acoustic_margin: f32,
}

/// Below this the term sounds clearly worse than what was heard.
pub const MARGIN_FLOOR: f32 = -4.0;
/// Replacing ordinary English words needs the term to sound nearly as likely
/// as the words themselves.
pub const ORDINARY_WORDS_MARGIN: f32 = -0.75;
/// The heard phrase must be spelled at least this much like the term. Every
/// correct proposal in the gauntlet set scores 0.56 or more; reported bad
/// swaps score far lower (Mac mini -> Meetumo 0.14).
pub const MINIMUM_SPELLING_SIMILARITY: f32 = 0.5;
/// A near-spelled join may differ by this many letters...
pub const NEAR_SPELLED_MAX_EDITS: usize = 2;
/// ...when the term is at least this long, so code -> Xcode never counts.
pub const NEAR_SPELLED_MIN_LENGTH: usize = 12;

pub fn accepts(candidate: &Candidate<'_>) -> bool {
    let heard_words = words(candidate.heard);
    let term_words = words(candidate.term);
    let heard_letters = heard_words.concat();
    let term_letters = term_words.concat();
    if heard_words.is_empty() || term_letters.is_empty() {
        return false;
    }

    // "use effect callback" -> useEffectCallback: the user's spelling of
    // exactly what was heard.
    if heard_letters == term_letters
        || (term_letters.chars().count() >= NEAR_SPELLED_MIN_LENGTH
            && edit_distance(&heard_letters, &term_letters) <= NEAR_SPELLED_MAX_EDITS)
    {
        return true;
    }
    if spelling_similarity(&heard_letters, &term_letters) < MINIMUM_SPELLING_SIMILARITY {
        return false;
    }
    // "keep LM Studio" -> "LM Studio" would delete a spoken word.
    if heard_words.len() > term_words.len() && term_words.iter().all(|w| heard_words.contains(w)) {
        return false;
    }
    if candidate.acoustic_margin < MARGIN_FLOOR {
        return false;
    }
    if lexicon::is_available() && heard_words.iter().all(|w| lexicon::is_real_word(w)) {
        return candidate.acoustic_margin >= ORDINARY_WORDS_MARGIN;
    }
    true
}

/// Lowercased letters, digits and apostrophes, punctuation dropped.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphabetic() || c.is_numeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// 1 for identical letters, 0 for nothing in common.
pub fn spelling_similarity(a: &str, b: &str) -> f32 {
    let length = a.chars().count().max(b.chars().count());
    if length == 0 {
        return 1.0;
    }
    1.0 - edit_distance(a, b) as f32 / length as f32
}

/// Levenshtein distance over characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases are real Word Boost proposals from the Mac's gauntlet corpora
    // with their measured acoustic margins (WordBoostTests.swift).
    fn accepts(heard: &str, term: &str, margin: f32) -> bool {
        super::accepts(&Candidate {
            heard,
            term,
            acoustic_margin: margin,
        })
    }

    #[test]
    fn spelled_out_joins_are_kept() {
        assert!(accepts("use effect callback", "useEffectCallback", -2.13));
        assert!(accepts("decoding options,", "DecodingOptions", -3.36));
        assert!(accepts("decoding option", "DecodingOptions", -1.88));
        assert!(accepts(
            "Finalized live transcription",
            "finalizeLiveTranscription",
            -1.73
        ));
        assert!(accepts(
            "committed sample cant",
            "committedSampleCount",
            -1.4
        ));
    }

    #[test]
    fn ordinary_words_need_strong_acoustic_evidence() {
        assert!(!accepts("code", "Xcode", -1.67));
        assert!(!accepts("decoder.", "Xcode", -0.93));
        assert!(!accepts("transcript", "TypeScript", -4.87));
        assert!(!accepts("grab", "GRDB", -1.95));
        assert!(!accepts("Whisper", "WhisperKit", -1.68));
        assert!(accepts("common case", "camelCase", -0.58));
        // The one correct proposal the guard gives up: the audio was unclear.
        assert!(!accepts("upcast", "appcast", -2.04));
    }

    #[test]
    fn misheard_names_are_kept() {
        assert!(accepts("Quen", "Qwen", -2.24));
        assert!(accepts("Aluwa soon", "Oluwaseun", -2.08));
        assert!(accepts("Tamitoke", "Temitope", -1.97));
    }

    #[test]
    fn floor_rejects_terms_that_sound_clearly_worse() {
        assert!(!accepts("runbook,", "Oyinbooke", -5.61));
        assert!(!accepts("release.", "camelCase", -4.23));
    }

    #[test]
    fn terms_spelled_nothing_like_the_heard_words_are_rejected() {
        assert!(!accepts("Mac mini", "Meetumo", -0.4));
        assert!(!accepts("space bar", "supabase", -0.3));
        assert!(!accepts("make a", "Arsenal", -0.2));
        assert!(!accepts("I'm", "Meetumo", 0.5));
        // Near-misses of names still pass.
        assert!(accepts("Folosiday", "Folasade", -1.43));
        assert!(accepts("Olama's", "Ollama", 0.84));
    }

    #[test]
    fn contractions_and_common_words_count_as_ordinary_words() {
        for word in [
            "i'm",
            "it's",
            "don't",
            "didn't",
            "you're",
            "we've",
            "they'll",
            "she'd",
            "can't",
            "won't",
            "it\u{2019}s",
        ] {
            assert!(lexicon::is_real_word(word), "{word}");
        }
        assert!(lexicon::is_real_word("mini"));
        assert!(lexicon::is_real_word("spacebar"));
        assert!(!lexicon::is_real_word("olama's"));
        assert_eq!(words("It\u{2019}s fine"), vec!["it's", "fine"]);
    }

    #[test]
    fn replacements_that_drop_spoken_words_are_rejected() {
        assert!(!accepts("keep LM Studio", "LM Studio", 0.29));
        assert!(!accepts("or LM Studio", "LM Studio", -0.26));
    }

    #[test]
    fn empty_or_punctuation_only_input_is_rejected() {
        assert!(!accepts("", "Qwen", 1.0));
        assert!(!accepts("...", "Qwen", 1.0));
        assert!(!accepts("Quen", "", 1.0));
    }

    #[test]
    fn edit_distance_and_similarity() {
        assert_eq!(edit_distance("code", "xcode"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(spelling_similarity("", ""), 1.0);
        assert!((spelling_similarity("quen", "qwen") - 0.75).abs() < 1e-6);
    }
}
