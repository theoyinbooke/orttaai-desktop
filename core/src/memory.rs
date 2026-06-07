//! Personal Memory — dictionary replacements and snippet expansions applied to
//! transcripts before injection. Pure, OS-agnostic logic (ports from the macOS app).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryService {
    /// Whole-word replacements (keys stored lowercased for case-insensitive match).
    dictionary: HashMap<String, String>,
    /// Snippet triggers expanded to longer text (keys lowercased).
    snippets: HashMap<String, String>,
}

impl MemoryService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a dictionary term replacement (e.g. `"npm"` → `"NPM"`).
    pub fn add_term(&mut self, from: impl AsRef<str>, to: impl Into<String>) {
        self.dictionary
            .insert(from.as_ref().to_lowercase(), to.into());
    }

    /// Add a snippet expansion (e.g. `"addr"` → `"123 Main St"`).
    pub fn add_snippet(&mut self, trigger: impl AsRef<str>, expansion: impl Into<String>) {
        self.snippets
            .insert(trigger.as_ref().to_lowercase(), expansion.into());
    }

    pub fn is_empty(&self) -> bool {
        self.dictionary.is_empty() && self.snippets.is_empty()
    }

    /// Apply dictionary + snippet replacements token-by-token, preserving any
    /// surrounding punctuation. Whitespace is normalized to single spaces.
    pub fn apply(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        text.split_whitespace()
            .map(|token| self.replace_token(token))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn replace_token(&self, token: &str) -> String {
        let core = token.trim_matches(|c: char| !c.is_alphanumeric());
        if core.is_empty() {
            return token.to_string();
        }
        let key = core.to_lowercase();
        let replacement = self
            .dictionary
            .get(&key)
            .or_else(|| self.snippets.get(&key));
        match replacement {
            Some(rep) => {
                // `core` is a trimmed slice of `token`, so `find` gives its offset.
                let start = token.find(core).unwrap_or(0);
                let prefix = &token[..start];
                let suffix = &token[start + core.len()..];
                format!("{prefix}{rep}{suffix}")
            }
            None => token.to_string(),
        }
    }
}
