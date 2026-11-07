//! Chunking, vector codec (f32 little-endian blobs), cosine similarity, and
//! the lexical fallback embedder (Mac `LexicalSemanticEmbeddingProvider`).

use super::util::{djb2, sha256_hex};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Words per chunk and overlap between consecutive chunks (Mac values).
pub const CHUNK_WORD_LIMIT: usize = 140;
pub const CHUNK_OVERLAP_WORDS: usize = 24;

/// Model id of the lexical fallback embedder.
pub const LEXICAL_MODEL_ID: &str = "lexical-fallback-v1";
pub const LEXICAL_PROVIDER_NAME: &str = "Lexical Fallback";
const LEXICAL_DIMENSION: usize = 256;

/// One chunk of a dictation, before persistence.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkDraft {
    pub chunk_index: i64,
    pub text: String,
    pub text_hash: String,
    pub word_count: i64,
}

/// Split text into overlapping word windows.
pub fn make_chunks(text: &str) -> Vec<ChunkDraft> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut chunks = Vec::new();
    if words.is_empty() {
        return chunks;
    }
    let mut start = 0;
    let mut index = 0;
    while start < words.len() {
        let end = (start + CHUNK_WORD_LIMIT).min(words.len());
        let chunk_text = words[start..end].join(" ").trim().to_string();
        if !chunk_text.is_empty() {
            chunks.push(ChunkDraft {
                chunk_index: index,
                text_hash: text_hash(&chunk_text),
                text: chunk_text,
                word_count: (end - start) as i64,
            });
            index += 1;
        }
        if end == words.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP_WORDS).max(start + 1);
    }
    chunks
}

/// Content hash (full SHA-256 hex).
pub fn text_hash(text: &str) -> String {
    sha256_hex(text, 32)
}

pub fn encode(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn decode(bytes: &[u8], dimension: usize) -> Option<Vec<f32>> {
    if dimension == 0 || bytes.len() != dimension * 4 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
    )
}

pub fn normalized(v: &[f32]) -> Vec<f32> {
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        v.iter().map(|x| x / mag).collect()
    } else {
        v.to_vec()
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut ma, mut mb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        ma += x * x;
        mb += y * y;
    }
    if ma <= 0.0 || mb <= 0.0 {
        return 0.0;
    }
    (dot / (ma.sqrt() * mb.sqrt())) as f64
}

fn stop_words() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        "the and for that this with from have has had are was were you your but not can will just \
         into about what when where they them then than our out get got all any one two use using \
         there their would could should because really very"
            .split_whitespace()
            .collect()
    })
}

/// Lowercased content tokens (>2 chars, stopwords removed).
pub fn lexical_tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|t| t.chars().count() > 2 && !stop_words().contains(t))
        .map(String::from)
        .collect()
}

/// Deterministic hashed bag-of-words vector, normalized.
pub fn lexical_embedding(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; LEXICAL_DIMENSION];
    let tokens = lexical_tokens(text);
    if tokens.is_empty() {
        return v;
    }
    for t in tokens {
        let idx = (djb2(&t) % LEXICAL_DIMENSION as u64) as usize;
        v[idx] += if t.chars().count() > 6 { 1.35 } else { 1.0 };
    }
    normalized(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_overlap_and_hash() {
        let text = (0..300)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = make_chunks(&text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].word_count, 140);
        assert!(chunks[1].text.starts_with("w116 "));
        assert!(chunks[2].text.starts_with("w232 "));
        assert!(chunks[2].text.ends_with("w299"));
        assert_eq!(chunks[2].word_count, 68);
        assert_eq!(chunks[0].text_hash.len(), 64);
        assert_ne!(chunks[0].text_hash, chunks[1].text_hash);
        assert!(make_chunks("   ").is_empty());
        let short = make_chunks("hello there world");
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].text, "hello there world");
    }

    #[test]
    fn codec_round_trips() {
        let v = vec![0.5f32, -1.25, 3.0];
        let bytes = encode(&v);
        assert_eq!(bytes.len(), 12);
        assert_eq!(decode(&bytes, 3).unwrap(), v);
        assert!(decode(&bytes, 4).is_none());
    }

    #[test]
    fn cosine_and_lexical() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        let a = lexical_embedding("deploy the docker container to production");
        let b = lexical_embedding("docker container deploy production server");
        let c = lexical_embedding("grandma birthday cake recipe");
        assert!(cosine(&a, &b) > cosine(&a, &c));
        assert_eq!(a.len(), 256);
    }
}
