//! The Word Boost CTC model's tokenizer: a port of FluidAudio's `CtcTokenizer`
//! and `BpeTokenizer` (Apache-2.0, FluidInference), which read the Hugging Face
//! `tokenizer.json` that ships with Parakeet CTC 110M
//! (`FluidInference/parakeet-ctc-110m-coreml`).
//!
//! Like FluidAudio it implements encoding only, and the same way: lowercase,
//! NFKC, spaces become `▁`, a `▁` word boundary in front, the text is split
//! into characters and BPE merges are applied lowest rank first (every
//! occurrence of the winning pair at once). Unknown pieces become `<unk>`.
//! It ignores the file's normalizer, pre-tokenizer and byte fallback, as
//! FluidAudio does, so ids match what the Mac's Word Boost scores.
//!
//! Two approximations, both irrelevant to the real (ASCII-only) vocabulary:
//! NFKC is a subset (compatibility spaces, full-width ASCII, Latin ligatures,
//! ellipsis, super/subscript digits), and Swift's grapheme clusters are
//! approximated by keeping combining marks, variation selectors and
//! zero-width joins with the character before them.

use std::collections::HashMap;

use serde_json::Value;

use crate::{CoreError, Result};

/// SentencePiece's word-boundary marker.
pub const WORD_BOUNDARY: char = '\u{2581}';

/// BPE tokenizer for the Word Boost CTC model.
#[derive(Debug, Clone)]
pub struct CtcTokenizer {
    vocab: HashMap<String, u32>,
    /// Merge rank by pair; the lowest rank merges first.
    merges: HashMap<(String, String), usize>,
    added_tokens: HashMap<String, u32>,
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::Transcription(format!("Word Boost tokenizer: {}", message.into()))
}

fn token_id(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|id| u32::try_from(id).ok())
}

impl CtcTokenizer {
    /// Parses a Hugging Face `tokenizer.json` with a BPE model.
    pub fn from_json(json: &str) -> Result<Self> {
        let root: Value = serde_json::from_str(json)?;
        let root = root
            .as_object()
            .ok_or_else(|| invalid("root is not an object"))?;
        let model = root
            .get("model")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("missing model"))?;
        let model_type = model
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("missing model.type"))?;
        if model_type != "BPE" {
            return Err(invalid(format!(
                "unsupported model type {model_type}; only BPE is supported"
            )));
        }

        let vocab_object = model
            .get("vocab")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("missing model.vocab"))?;
        let mut vocab = HashMap::with_capacity(vocab_object.len());
        for (token, id) in vocab_object {
            let id = token_id(id).ok_or_else(|| invalid(format!("bad id for {token:?}")))?;
            vocab.insert(token.clone(), id);
        }

        let merge_list = model
            .get("merges")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("missing model.merges"))?;
        let mut merges = HashMap::with_capacity(merge_list.len());
        for (rank, merge) in merge_list.iter().enumerate() {
            // "a b" (FluidAudio's format); newer files write ["a", "b"].
            let pair = match merge {
                Value::String(text) => text
                    .split_once(' ')
                    .filter(|(a, b)| !a.is_empty() && !b.is_empty())
                    .map(|(a, b)| (a.to_string(), b.to_string())),
                Value::Array(parts) => match parts.as_slice() {
                    [Value::String(a), Value::String(b)] => Some((a.clone(), b.clone())),
                    _ => None,
                },
                _ => None,
            };
            if let Some(pair) = pair {
                // The first occurrence of a pair is the one FluidAudio finds.
                merges.entry(pair).or_insert(rank);
            }
        }

        let mut added_tokens = HashMap::new();
        if let Some(list) = root.get("added_tokens").and_then(Value::as_array) {
            for token in list {
                let content = token.get("content").and_then(Value::as_str);
                let id = token.get("id").and_then(token_id);
                if let (Some(content), Some(id)) = (content, id) {
                    added_tokens.insert(content.to_string(), id);
                }
            }
        }

        Ok(Self {
            vocab,
            merges,
            added_tokens,
        })
    }

    /// Number of entries in the model vocabulary (not counting added tokens).
    /// The CTC blank is the class after them.
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Token ids for `text`, starting at a word boundary.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.encode_with(text, true)
    }

    /// Token ids for `text` without the leading `▁`, for a term that may start
    /// mid-word in the audio (a compound like "Liv marli" -> Livmarli).
    pub fn encode_without_boundary(&self, text: &str) -> Vec<u32> {
        self.encode_with(text, false)
    }

    fn encode_with(&self, text: &str, prepend_word_boundary: bool) -> Vec<u32> {
        let normalized = nfkc_subset(&text.to_lowercase());
        let mut preprocessed = String::with_capacity(normalized.len() + 3);
        if prepend_word_boundary {
            preprocessed.push(WORD_BOUNDARY);
        }
        for c in normalized.chars() {
            preprocessed.push(if c == ' ' { WORD_BOUNDARY } else { c });
        }

        let mut word = clusters(&preprocessed);
        while word.len() >= 2 {
            let mut best: Option<(usize, usize)> = None; // (rank, index)
            for i in 0..word.len() - 1 {
                let key = (word[i].clone(), word[i + 1].clone());
                if let Some(&rank) = self.merges.get(&key) {
                    if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                        best = Some((rank, i));
                    }
                }
            }
            let Some((_, index)) = best else { break };
            let first = word[index].clone();
            let second = word[index + 1].clone();
            let mut merged = Vec::with_capacity(word.len());
            let mut i = 0;
            while i < word.len() {
                if i + 1 < word.len() && word[i] == first && word[i + 1] == second {
                    merged.push(format!("{first}{second}"));
                    i += 2;
                } else {
                    merged.push(std::mem::take(&mut word[i]));
                    i += 1;
                }
            }
            word = merged;
        }

        let unknown = self
            .added_tokens
            .get("<unk>")
            .or_else(|| self.vocab.get("<unk>"))
            .copied()
            .unwrap_or(0);
        word.iter()
            .map(|piece| {
                self.added_tokens
                    .get(piece)
                    .or_else(|| self.vocab.get(piece))
                    .copied()
                    .unwrap_or(unknown)
            })
            .collect()
    }
}

/// Common combining marks (Unicode category M), including variation selectors.
pub(crate) fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0900..=0x0903
            | 0x093A..=0x094F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xE0100..=0xE01EF)
}

/// Marks, joiners and emoji modifiers that Swift keeps in the same
/// `Character` as the scalar before them.
pub(crate) fn is_extending(c: char) -> bool {
    is_mark(c) || matches!(c as u32, 0x200C..=0x200D | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

/// The text split into approximate grapheme clusters.
fn clusters(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(text.len());
    let mut joined = false;
    for c in text.chars() {
        match out.last_mut() {
            Some(last) if joined || is_extending(c) => last.push(c),
            _ => out.push(c.to_string()),
        }
        joined = c == '\u{200D}';
    }
    out
}

/// The parts of NFKC that can turn a character into one the vocabulary has.
fn nfkc_subset(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => {
                out.push(' ')
            }
            '\u{FF01}'..='\u{FF5E}' => {
                out.push(char::from_u32(c as u32 - 0xFEE0).unwrap_or(c));
            }
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            '\u{2024}' => out.push('.'),
            '\u{2025}' => out.push_str(".."),
            '\u{2026}' => out.push_str("..."),
            '\u{00B9}' => out.push('1'),
            '\u{00B2}' => out.push('2'),
            '\u{00B3}' => out.push('3'),
            '\u{2070}' => out.push('0'),
            '\u{2074}'..='\u{2079}' => {
                out.push(char::from_u32(c as u32 - 0x2070 + 0x30).unwrap_or(c))
            }
            '\u{2080}'..='\u{2089}' => {
                out.push(char::from_u32(c as u32 - 0x2080 + 0x30).unwrap_or(c))
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `<unk>` 0, `▁` 1, `a`..`z` 2..=27, plus `.` 28 and a few merges.
    pub(crate) fn letters_json(merges: &[&str]) -> String {
        let mut vocab = vec![
            "\"<unk>\": 0".to_string(),
            format!("\"{WORD_BOUNDARY}\": 1"),
        ];
        for (i, c) in ('a'..='z').enumerate() {
            vocab.push(format!("\"{c}\": {}", i + 2));
        }
        for (i, merge) in merges.iter().enumerate() {
            let (a, b) = merge.split_once(' ').unwrap();
            vocab.push(format!("\"{a}{b}\": {}", 28 + i));
        }
        let merges: Vec<String> = merges.iter().map(|m| format!("\"{m}\"")).collect();
        format!(
            r#"{{"added_tokens": [{{"id": 0, "content": "<unk>"}}],
                "model": {{"type": "BPE", "vocab": {{{}}}, "merges": [{}]}}}}"#,
            vocab.join(", "),
            merges.join(", ")
        )
    }

    /// Letters only, no merges: every word is `▁` then its letters.
    pub(crate) fn letters() -> CtcTokenizer {
        CtcTokenizer::from_json(&letters_json(&[])).unwrap()
    }

    /// The id of a letter in [`letters`].
    pub(crate) fn id(c: char) -> u32 {
        if c == WORD_BOUNDARY {
            1
        } else {
            c as u32 - 'a' as u32 + 2
        }
    }

    #[test]
    fn spells_words_after_a_boundary_and_lowercases() {
        let tok = letters();
        assert_eq!(tok.encode("Ab"), vec![1, id('a'), id('b')]);
        assert_eq!(tok.encode_without_boundary("Ab"), vec![id('a'), id('b')]);
        assert_eq!(tok.encode("a b"), vec![1, id('a'), 1, id('b')]);
        assert_eq!(tok.vocab_size(), 28);
    }

    #[test]
    fn unknown_characters_become_unk() {
        let tok = letters();
        assert_eq!(tok.encode("a."), vec![1, id('a'), 0]);
        // A decomposed accent stays one character, as in Swift.
        assert_eq!(tok.encode("e\u{301}"), vec![1, 0]);
        assert_eq!(tok.encode(""), vec![1]);
        assert!(tok.encode_without_boundary("").is_empty());
    }

    #[test]
    fn merges_apply_lowest_rank_first_to_every_occurrence() {
        // "b c" outranks "a b": "abcbc" -> a, bc, bc, never ab.
        let json = letters_json(&["b c", "a b", "\u{2581} a"]);
        let tok = CtcTokenizer::from_json(&json).unwrap();
        let bc = 28;
        let boundary_a = 30;
        assert_eq!(tok.encode_without_boundary("abcbc"), vec![id('a'), bc, bc]);
        assert_eq!(tok.encode("abcbc"), vec![boundary_a, bc, bc]);
    }

    #[test]
    fn compatibility_forms_fold_to_ascii() {
        let tok = letters();
        assert_eq!(tok.encode("\u{FF21}b"), tok.encode("ab"));
        assert_eq!(tok.encode("a\u{00A0}b"), tok.encode("a b"));
        assert_eq!(tok.encode("\u{FB01}"), tok.encode("fi"));
    }

    #[test]
    fn accepts_pair_merges_and_rejects_other_models() {
        let json = letters_json(&[]).replace(r#""merges": []"#, r#""merges": [["a", "b"]]"#);
        let json = json.replace(r#""a": 2"#, r#""a": 2, "ab": 40"#);
        let tok = CtcTokenizer::from_json(&json).unwrap();
        assert_eq!(tok.encode_without_boundary("ab"), vec![40]);

        assert!(CtcTokenizer::from_json("{}").is_err());
        assert!(CtcTokenizer::from_json("not json").is_err());
        assert!(CtcTokenizer::from_json(r#"{"model": {"type": "Unigram"}}"#).is_err());
        assert!(CtcTokenizer::from_json(
            r#"{"model": {"type": "BPE", "vocab": {"a": -1}, "merges": []}}"#
        )
        .is_err());
    }

    /// The real tokenizer: `ORTTAAI_WORDBOOST_TOKENIZER=/path/to/tokenizer.json
    /// cargo test -p orttaai-core --lib wordboost -- --ignored`. Expected ids
    /// were computed from the file with FluidAudio's algorithm.
    #[test]
    #[ignore = "needs ORTTAAI_WORDBOOST_TOKENIZER pointing at the real tokenizer.json"]
    fn real_tokenizer_matches_known_ids() {
        let path =
            std::env::var("ORTTAAI_WORDBOOST_TOKENIZER").expect("set ORTTAAI_WORDBOOST_TOKENIZER");
        let tok = CtcTokenizer::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(tok.vocab_size(), 1024);
        assert_eq!(tok.encode("the"), vec![6]);
        assert_eq!(tok.encode("hello world"), vec![80, 30, 969, 632]);
        assert_eq!(
            tok.encode_without_boundary("hello world"),
            vec![426, 30, 969, 632]
        );
        assert_eq!(tok.encode("code"), vec![16, 220, 967]);
        assert_eq!(tok.encode("Xcode"), vec![966, 996, 979, 220, 967]);
        assert_eq!(tok.encode("WhisperKit"), vec![60, 18, 318, 990, 14]);
        assert_eq!(tok.encode("Qwen"), vec![966, 1009, 983, 19]);
        assert_eq!(tok.encode("café"), vec![804, 984, 0]);
        assert_eq!(tok.encode("\u{FF38}code"), tok.encode("xcode"));
        // FluidAudio's BpeTokenizerVariantsTests.
        let with = tok.encode("nvidia");
        let without = tok.encode_without_boundary("nvidia");
        assert_eq!(with, vec![43, 989, 69, 457]);
        assert_ne!(with.first(), without.first());
        assert!(without.len() <= with.len() + 1);
        assert_ne!(tok.encode("hello"), tok.encode_without_boundary("hello"));
    }
}
