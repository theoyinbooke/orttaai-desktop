//! Personal Memory — dictionary replacements and snippet expansions (Mac
//! `RuleBasedTextProcessor` dictionary/snippet passes). Pure, OS-agnostic.
//!
//! - **Dictionary**: whole-phrase replacements, longest source first, matched
//!   at word boundaries (`(?<![\p{L}\p{N}_])source(?![\p{L}\p{N}_])`), so
//!   surrounding punctuation is kept. Case-insensitive unless the entry is
//!   case-sensitive. Sources may be several words ("open ai" → "OpenAI").
//! - **Snippets**: the Mac expands a snippet when the whole dictation is its
//!   trigger, optionally after "insert", "use", "expand" or "snippet"
//!   ([`MemoryService::resolve_snippet`]). [`MemoryService::apply`] keeps the
//!   older desktop behaviour of also expanding triggers inline.
//!
//! The full transcript pipeline lives in [`crate::text`].

use crate::text::chars::normalized_key;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// One dictionary rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryRule {
    /// Store row id, when the rule came from the database.
    pub id: Option<i64>,
    /// What speech recognition hears.
    pub source: String,
    /// How it should be written.
    pub target: String,
    pub case_sensitive: bool,
    /// Times it fired (orders the vocabulary-bias prompt).
    pub usage_count: i64,
}

/// One snippet rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetRule {
    pub id: Option<i64>,
    /// The phrase the user says.
    pub trigger: String,
    /// The text it types.
    pub expansion: String,
    pub usage_count: i64,
}

/// A dictionary rule that fired, with how many times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryHit {
    pub id: Option<i64>,
    pub source: String,
    pub target: String,
    pub count: usize,
}

/// Result of the dictionary pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryPass {
    pub text: String,
    /// Rules that fired, in the order they were applied.
    pub hits: Vec<DictionaryHit>,
}

/// Active dictionary and snippet rules, ready to apply during dictation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryService {
    dictionary: Vec<DictionaryRule>,
    snippets: Vec<SnippetRule>,
}

/// Commands that may precede a spoken snippet trigger.
const SNIPPET_COMMAND_PREFIXES: &[&str] = &["insert ", "use ", "expand ", "snippet "];

impl MemoryService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a case-insensitive dictionary term (e.g. `"npm"` → `"NPM"`).
    pub fn add_term(&mut self, from: impl AsRef<str>, to: impl Into<String>) {
        self.add_term_with_case(from, to, false);
    }

    /// Add a dictionary term, matched case-sensitively when `case_sensitive`.
    /// A term with the same (normalized) source replaces the earlier one.
    pub fn add_term_with_case(
        &mut self,
        from: impl AsRef<str>,
        to: impl Into<String>,
        case_sensitive: bool,
    ) {
        self.add_dictionary_rule(DictionaryRule {
            id: None,
            source: from.as_ref().trim().to_string(),
            target: to.into().trim().to_string(),
            case_sensitive,
            usage_count: 0,
        });
    }

    /// Add a full dictionary rule (from the store).
    pub fn add_dictionary_rule(&mut self, rule: DictionaryRule) {
        if rule.source.trim().is_empty() {
            return;
        }
        let key = normalized_key(&rule.source);
        self.dictionary.retain(|r| normalized_key(&r.source) != key);
        self.dictionary.push(rule);
    }

    /// Add a snippet expansion (e.g. `"my address"` → `"12 Harbour Road"`).
    pub fn add_snippet(&mut self, trigger: impl AsRef<str>, expansion: impl Into<String>) {
        self.add_snippet_rule(SnippetRule {
            id: None,
            trigger: trigger.as_ref().trim().to_string(),
            expansion: expansion.into(),
            usage_count: 0,
        });
    }

    /// Add a full snippet rule (from the store).
    pub fn add_snippet_rule(&mut self, rule: SnippetRule) {
        let key = normalized_key(&rule.trigger);
        if key.is_empty() {
            return;
        }
        self.snippets.retain(|r| normalized_key(&r.trigger) != key);
        self.snippets.push(rule);
    }

    pub fn is_empty(&self) -> bool {
        self.dictionary.is_empty() && self.snippets.is_empty()
    }

    pub fn dictionary(&self) -> &[DictionaryRule] {
        &self.dictionary
    }

    pub fn snippets(&self) -> &[SnippetRule] {
        &self.snippets
    }

    /// Compile the dictionary for repeated use (the text pipeline keeps one).
    pub fn compile_dictionary(&self) -> CompiledDictionary {
        CompiledDictionary::new(&self.dictionary)
    }

    /// Run the dictionary pass once.
    pub fn apply_dictionary(&self, text: &str) -> DictionaryPass {
        self.compile_dictionary().apply(text)
    }

    /// The snippet whose trigger is the whole dictation (ignoring case, extra
    /// spaces and closing punctuation), optionally after "insert", "use",
    /// "expand" or "snippet".
    pub fn resolve_snippet(&self, text: &str) -> Option<&SnippetRule> {
        let normalized = normalized_key(text);
        if normalized.is_empty() || self.snippets.is_empty() {
            return None;
        }
        let bare = normalized
            .trim_end_matches(['.', '!', '?', ',', '…', ';', ':'])
            .trim_end()
            .to_string();
        let find = |key: &str| {
            self.snippets
                .iter()
                .rev()
                .find(|s| normalized_key(&s.trigger) == key)
        };
        for candidate in [&normalized, &bare] {
            if let Some(hit) = find(candidate) {
                return Some(hit);
            }
            for prefix in SNIPPET_COMMAND_PREFIXES {
                if let Some(rest) = candidate.strip_prefix(prefix) {
                    if let Some(hit) = find(&normalized_key(rest)) {
                        return Some(hit);
                    }
                }
            }
        }
        None
    }

    /// Expand snippet triggers wherever they appear as whole phrases (longest
    /// first). Returns the text and the triggers that fired.
    pub fn expand_inline_snippets(&self, text: &str) -> (String, Vec<String>) {
        let mut ordered: Vec<&SnippetRule> = self.snippets.iter().collect();
        ordered.sort_by(|a, b| {
            b.trigger
                .chars()
                .count()
                .cmp(&a.trigger.chars().count())
                .then_with(|| a.trigger.cmp(&b.trigger))
        });
        let mut value = text.to_string();
        let mut fired = Vec::new();
        for rule in ordered {
            let Some(re) = phrase_regex(&rule.trigger, false) else {
                continue;
            };
            let (next, count) = replace_phrase(&re, &value, &rule.expansion);
            if count > 0 {
                value = next;
                fired.push(rule.trigger.clone());
            }
        }
        (value, fired)
    }

    /// Dictionary replacements, then inline snippet expansion, preserving
    /// surrounding punctuation. Whitespace is normalized to single spaces.
    pub fn apply(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let pass = self.apply_dictionary(&normalized);
        self.expand_inline_snippets(&pass.text).0
    }
}

/// A dictionary compiled to matchers, longest source first.
#[derive(Debug, Clone, Default)]
pub struct CompiledDictionary {
    rules: Vec<(DictionaryRule, Regex)>,
}

impl CompiledDictionary {
    pub fn new(rules: &[DictionaryRule]) -> Self {
        let mut ordered: Vec<&DictionaryRule> = rules.iter().collect();
        ordered.sort_by(|a, b| {
            let (la, lb) = (
                a.source.trim().chars().count(),
                b.source.trim().chars().count(),
            );
            lb.cmp(&la).then_with(|| a.source.cmp(&b.source))
        });
        Self {
            rules: ordered
                .into_iter()
                .filter_map(|r| phrase_regex(&r.source, r.case_sensitive).map(|re| (r.clone(), re)))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Replace every whole-phrase occurrence of each source with its target,
    /// one rule after another (a later rule sees earlier replacements).
    pub fn apply(&self, text: &str) -> DictionaryPass {
        let mut value = text.to_string();
        let mut hits = Vec::new();
        for (rule, re) in &self.rules {
            let (next, count) = replace_phrase(re, &value, &rule.target);
            if count == 0 {
                continue;
            }
            value = next;
            hits.push(DictionaryHit {
                id: rule.id,
                source: rule.source.clone(),
                target: rule.target.clone(),
                count,
            });
        }
        DictionaryPass { text: value, hits }
    }
}

fn phrase_regex(phrase: &str, case_sensitive: bool) -> Option<Regex> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return None;
    }
    RegexBuilder::new(&regex::escape(phrase))
        .case_insensitive(!case_sensitive)
        .build()
        .ok()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replace whole-phrase matches of `re` (not touching a letter, digit or `_`
/// on either side) with `replacement`, literally.
fn replace_phrase(re: &Regex, text: &str, replacement: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut pos = 0;
    let mut count = 0;
    while pos <= text.len() {
        let Some(m) = re.find_at(text, pos) else {
            break;
        };
        let before_ok = text[..m.start()]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let after_ok = text[m.end()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok && m.end() > m.start() {
            out.push_str(&text[cursor..m.start()]);
            out.push_str(replacement);
            cursor = m.end();
            pos = m.end();
            count += 1;
        } else {
            pos = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
        }
    }
    if count == 0 {
        return (text.to_string(), 0);
    }
    out.push_str(&text[cursor..]);
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_matches_whole_phrases_longest_first() {
        let mut m = MemoryService::new();
        m.add_term("open ai", "OpenAI");
        m.add_term("open", "Open");
        m.add_term("whispr", "Wispr");
        let pass = m.apply_dictionary("I use open ai, whispr flow and open source. whisprs?");
        assert_eq!(
            pass.text,
            "I use OpenAI, Wispr flow and Open source. whisprs?"
        );
        let fired: Vec<(&str, usize)> = pass
            .hits
            .iter()
            .map(|h| (h.source.as_str(), h.count))
            .collect();
        assert_eq!(fired, vec![("open ai", 1), ("whispr", 1), ("open", 1)]);
    }

    #[test]
    fn case_sensitive_entries_match_exact_capitalization() {
        let mut m = MemoryService::new();
        m.add_term_with_case("Go", "Golang", true);
        assert_eq!(m.apply_dictionary("Go and go").text, "Golang and go");
        let mut m = MemoryService::new();
        m.add_term_with_case("go", "Golang", false);
        assert_eq!(m.apply_dictionary("Go and go").text, "Golang and Golang");
    }

    #[test]
    fn punctuation_and_unicode_neighbours_are_kept() {
        let mut m = MemoryService::new();
        m.add_term("temi tope", "Temitope");
        assert_eq!(
            m.apply_dictionary("(temi tope), temi tope's & temi topes")
                .text,
            "(Temitope), Temitope's & temi topes"
        );
        m.add_term("café", "Café Noir");
        assert_eq!(m.apply_dictionary("the café.").text, "the Café Noir.");
    }

    #[test]
    fn overlapping_rejected_match_does_not_hide_a_later_one() {
        let mut m = MemoryService::new();
        m.add_term("aa", "X");
        assert_eq!(m.apply_dictionary("aaa aa").text, "aaa X");
    }

    #[test]
    fn later_terms_with_the_same_source_replace_earlier_ones() {
        let mut m = MemoryService::new();
        m.add_term("npm", "Npm");
        m.add_term("NPM ", "NPM");
        assert_eq!(m.dictionary().len(), 1);
        assert_eq!(m.apply("i love npm"), "i love NPM");
    }

    #[test]
    fn snippets_resolve_on_the_whole_dictation() {
        let mut m = MemoryService::new();
        m.add_snippet("my email", "me@example.com");
        for said in [
            "my email",
            "Insert my email.",
            "  use   MY email ",
            "snippet my email!",
        ] {
            assert_eq!(
                m.resolve_snippet(said).map(|s| s.expansion.as_str()),
                Some("me@example.com"),
                "{said}"
            );
        }
        assert!(m.resolve_snippet("send my email to Bob").is_none());
        assert!(m.resolve_snippet("").is_none());
    }

    #[test]
    fn legacy_apply_expands_multi_word_snippets_inline() {
        let mut m = MemoryService::new();
        m.add_term("npm", "NPM");
        m.add_snippet("my addr", "123 Main St");
        assert_eq!(m.apply("i  love Npm."), "i love NPM.");
        assert_eq!(m.apply("send my addr, please"), "send 123 Main St, please");
        let (_, fired) = m.expand_inline_snippets("my addr and my addr");
        assert_eq!(fired, vec!["my addr".to_string()]);
    }
}
