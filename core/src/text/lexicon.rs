//! Common English words (Mac `EnglishLexicon`), used to keep the fuzzy
//! dictionary pass and edit learning away from ordinary vocabulary ("verse"
//! must never become "Vercel").
//!
//! The list is the Mac app's bundled `english-words.txt` (public-domain
//! Webster's Second, trimmed, plus modern words: 108,200 lowercase ASCII
//! words, sorted). It ships inside the binary front-coded and zlib-deflated
//! (~205 KB instead of ~900 KB): each line is one byte `'0' + n` giving how
//! many leading letters it shares with the previous word, then the rest of
//! the word. It is inflated once, lazily, on first use. Inflected forms are
//! not listed: lookups strip common suffixes instead.

use std::collections::HashSet;
use std::sync::OnceLock;

static PACKED: &[u8] = include_bytes!("../../assets/english-words.fc.zlib");

fn words() -> &'static HashSet<&'static str> {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        let Ok(raw) = miniz_oxide::inflate::decompress_to_vec_zlib(PACKED) else {
            tracing::error!(
                "english word list failed to inflate; fuzzy dictionary matching disabled"
            );
            return HashSet::new();
        };
        let Ok(text) = String::from_utf8(raw) else {
            return HashSet::new();
        };
        let mut set = HashSet::with_capacity(110_000);
        let mut joined = String::with_capacity(text.len() * 2);
        let mut bounds = Vec::with_capacity(110_000);
        let mut previous = String::new();
        for line in text.lines() {
            let Some(first) = line.as_bytes().first() else {
                continue;
            };
            let shared = (first.saturating_sub(b'0')) as usize;
            let mut word = String::with_capacity(shared + line.len());
            word.push_str(&previous[..shared.min(previous.len())]);
            word.push_str(&line[1..]);
            let start = joined.len();
            joined.push_str(&word);
            bounds.push((start, joined.len()));
            previous = word;
        }
        // One allocation for every word, kept for the life of the process.
        let joined: &'static str = Box::leak(joined.into_boxed_str());
        for (start, end) in bounds {
            set.insert(&joined[start..end]);
        }
        set
    })
}

/// Words under three letters are not in the file.
const SHORT_WORDS: &[&str] = &[
    "a", "i", "am", "an", "as", "at", "be", "by", "do", "go", "he", "hi", "if", "in", "is", "it",
    "me", "my", "no", "of", "oh", "ok", "on", "or", "so", "to", "up", "us", "we",
];

/// Everyday words the bundled list lacks. Without them a phrase like
/// "Mac mini" counted as unusual.
const SUPPLEMENTAL_WORDS: &[&str] = &[
    "mini",
    "spacebar",
    "checkbox",
    "dropdowns",
    "unmute",
    "touchpad",
    "trackpad",
];

/// Contractions whose base is not simply the part before the apostrophe.
const IRREGULAR_CONTRACTIONS: &[&str] = &[
    "can't", "won't", "shan't", "ain't", "y'all", "o'clock", "ma'am",
];

/// Endings after an apostrophe that leave a real word in front of them.
const CONTRACTION_SUFFIXES: &[&str] = &["n't", "'s", "'m", "'re", "'ve", "'ll", "'d"];

const SUFFIX_RULES: &[(&str, &[&str])] = &[
    ("ies", &["y"]),
    ("es", &[""]),
    ("s", &[""]),
    ("ed", &["", "e"]),
    ("d", &[""]),
    ("ing", &["", "e"]),
    ("ly", &[""]),
    ("er", &["", "e"]),
    ("ers", &["", "e"]),
    ("est", &["", "e"]),
    ("ness", &[""]),
];

/// False when the word list could not be loaded, in which case callers must
/// not assume an unlisted word is unusual.
pub fn is_available() -> bool {
    !words().is_empty()
}

/// Number of words in the bundled list (diagnostics/tests).
pub fn word_count() -> usize {
    words().len()
}

/// `word` must already be lowercase. Accepts listed words and their regular
/// inflections (verses, codes, stopped, running).
pub fn is_real_word(word: &str) -> bool {
    let word = word.replace('\u{2019}', "'");
    if word.contains('\'') {
        return is_contraction(&word);
    }
    if word.chars().count() < 3 {
        return SHORT_WORDS.contains(&word.as_str());
    }
    let list = words();
    if list.contains(word.as_str()) || SUPPLEMENTAL_WORDS.contains(&word.as_str()) {
        return true;
    }
    stems(&word).iter().any(|s| list.contains(s.as_str()))
}

fn is_contraction(word: &str) -> bool {
    if IRREGULAR_CONTRACTIONS.contains(&word) {
        return true;
    }
    for suffix in CONTRACTION_SUFFIXES {
        if let Some(base) = word.strip_suffix(suffix) {
            if !base.is_empty() && !base.contains('\'') && is_real_word(base) {
                return true;
            }
        }
    }
    false
}

/// The base forms `word` could be a regular inflection of, with the suffix
/// stripped and any dropped letter restored (stopped → stop, coded → code,
/// parties → party). Not checked against the word list.
pub fn stems(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (suffix, restorations) in SUFFIX_RULES {
        let Some(stem) = word.strip_suffix(suffix) else {
            continue;
        };
        let chars: Vec<char> = stem.chars().collect();
        if chars.len() < 3 {
            continue;
        }
        for r in *restorations {
            out.push(format!("{stem}{r}"));
        }
        if chars[chars.len() - 1] == chars[chars.len() - 2] {
            out.push(chars[..chars.len() - 1].iter().collect());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_loads_every_word() {
        assert!(is_available());
        assert_eq!(word_count(), 108_200);
        assert!(PACKED.len() < 400_000, "keep the bundled list small");
    }

    #[test]
    fn accepts_words_and_regular_inflections_but_not_names() {
        for w in [
            "verse",
            "verses",
            "arena",
            "script",
            "transcript",
            "cache",
            "cash",
            "codes",
            "stopped",
            "meetup",
            "meetups",
            "app",
            "aardvark",
            "zymurgy",
        ] {
            assert!(is_real_word(w), "{w}");
        }
        for w in [
            "temitope",
            "temitayo",
            "meetumo",
            "oyinbooke",
            "tematope",
            "vercel",
            "supabase",
        ] {
            assert!(!is_real_word(w), "{w}");
        }
    }

    #[test]
    fn knows_common_modern_developer_and_product_words() {
        for w in [
            "codec",
            "codecs",
            "github",
            "kubernetes",
            "postgres",
            "prisma",
            "figma",
            "tailwind",
            "websocket",
            "websockets",
            "codebase",
            "codebases",
            "docker",
            "terraform",
            "graphql",
            "typescript",
            "javascript",
            "firebase",
            "cloudflare",
            "stripe",
            "webhook",
            "webhooks",
            "changelog",
            "deployed",
            "deploying",
            "refactored",
            "dashboards",
            "middleware",
            "microservices",
            "ffmpeg",
            "bitrate",
            "openai",
            "chatgpt",
            "iphone",
            "macbook",
            "signup",
            "freemium",
        ] {
            assert!(is_real_word(w), "{w}");
        }
    }

    #[test]
    fn contractions_and_short_words() {
        for w in [
            "isn't",
            "they're",
            "can't",
            "it's",
            "i'm",
            "we\u{2019}ve",
            "ok",
            "a",
        ] {
            assert!(is_real_word(w), "{w}");
        }
        assert!(!is_real_word("zq"));
        assert!(!is_real_word("folasade's"));
    }

    #[test]
    fn stems_restore_dropped_letters() {
        assert!(stems("stopped").contains(&"stop".to_string()));
        assert!(stems("coded").contains(&"code".to_string()));
        assert!(stems("parties").contains(&"party".to_string()));
        assert!(stems("is").is_empty());
    }
}
