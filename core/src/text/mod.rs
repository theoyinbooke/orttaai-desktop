//! Transcript text pipeline (Mac `RuleBasedTextProcessor`).
//!
//! [`TextProcessor::process`] runs, in the Mac's order: dictionary → fuzzy
//! dictionary → snippets → disfluency cleanup → spoken formatting → automatic
//! paragraphs. Polish (the LLM pass) is separate and runs afterwards; it reads
//! [`style_instruction`] for the Formal and Casual style modes. Express
//! (Parakeet) output should go through [`format_numbers`] before `process`,
//! as on the Mac — Whisper already writes digits.
//!
//! Also here: [`learning`] (learn dictionary words from edits and history).

pub mod chars;
pub mod disfluency;
pub mod formatting;
pub mod fuzzy;
pub mod learning;
pub mod lexicon;
mod numbers;

pub use numbers::format_numbers;

use crate::memory::{CompiledDictionary, MemoryService};
use crate::settings::{Settings, StyleMode};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Which passes run (built from [`Settings`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextOptions {
    pub style_mode: StyleMode,
    pub spoken_formatting: bool,
    pub auto_paragraphs: bool,
    pub dictionary_enabled: bool,
    pub fuzzy_dictionary: bool,
    pub snippets_enabled: bool,
    pub disfluency_cleanup: bool,
    /// Effective dictation language (`"en"`, `"es"`, `"auto"` …). The fuzzy
    /// dictionary and disfluency cleanup run for `"en"` only.
    pub language: String,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

impl TextOptions {
    /// Mirrors the Mac's `effectiveDictationLanguage`: low-latency mode forces
    /// English for "auto", and Express (Parakeet) is English-only.
    pub fn from_settings(s: &Settings) -> Self {
        // Express (Parakeet) is English-only and runs only for English.
        let language = s.effective_language();
        Self {
            style_mode: s.style_mode,
            spoken_formatting: s.spoken_formatting,
            auto_paragraphs: s.auto_paragraphs,
            dictionary_enabled: s.dictionary_enabled,
            fuzzy_dictionary: s.fuzzy_dictionary,
            snippets_enabled: s.snippets_enabled,
            disfluency_cleanup: s.disfluency_cleanup,
            language,
        }
    }

    fn is_english(&self) -> bool {
        self.language == "en"
            || self.language.starts_with("en-")
            || self.language.starts_with("en_")
    }
}

/// The pipeline's output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessedText {
    pub text: String,
    /// Dictionary sources that fired — one entry per literal match, ready for
    /// `Store::record_memory_usage`. Fuzzy corrections never count (they order
    /// the bias prompt, and a near-miss is not evidence of demand).
    pub dictionary_hits: Vec<String>,
    /// The snippet trigger that expanded, if any.
    pub snippet_hits: Vec<String>,
    /// Human-readable notes on what each pass changed (for previews/logs).
    pub changes: Vec<String>,
}

/// A dictionary target for Express's Word Boost, with the spellings the user
/// mapped to it as aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordBoostTerm {
    pub text: String,
    pub aliases: Vec<String>,
}

/// The transcript pipeline for one settings + memory snapshot. Cheap to
/// share across threads; rebuild it when settings or memory change.
#[derive(Debug)]
pub struct TextProcessor {
    opts: TextOptions,
    memory: MemoryService,
    dictionary: CompiledDictionary,
    /// Built on first use (or by [`TextProcessor::vocabulary_prompt`], which
    /// runs before decoding) so the finalize path only compares.
    fuzzy: OnceLock<fuzzy::FuzzyMatcher>,
}

/// Bias-prompt caps (Mac `TranscriptionService`): at most 40 terms, fitted
/// whole into about 32 prompt tokens.
pub const MAX_BIAS_TERMS: usize = 40;
pub const MAX_BIAS_PROMPT_TOKENS: usize = 32;

impl TextProcessor {
    pub fn new(opts: TextOptions, memory: MemoryService) -> Self {
        let dictionary = memory.compile_dictionary();
        Self {
            opts,
            memory,
            dictionary,
            fuzzy: OnceLock::new(),
        }
    }

    pub fn options(&self) -> &TextOptions {
        &self.opts
    }

    pub fn memory(&self) -> &MemoryService {
        &self.memory
    }

    fn fuzzy_matcher(&self) -> &fuzzy::FuzzyMatcher {
        self.fuzzy.get_or_init(|| {
            let targets: Vec<&str> = self
                .memory
                .dictionary()
                .iter()
                .map(|r| r.target.as_str())
                .collect();
            fuzzy::FuzzyMatcher::new(&targets)
        })
    }

    fn fuzzy_enabled(&self) -> bool {
        self.opts.dictionary_enabled && self.opts.fuzzy_dictionary && self.opts.is_english()
    }

    /// Run the pipeline with no known target app.
    pub fn process(&self, raw: &str) -> ProcessedText {
        self.process_for_app(raw, None)
    }

    /// Run the pipeline for text going into `target_app` (automatic
    /// paragraphs never apply in terminals).
    pub fn process_for_app(&self, raw: &str, target_app: Option<&str>) -> ProcessedText {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return ProcessedText {
                text: raw.to_string(),
                ..Default::default()
            };
        }
        let mut out = ProcessedText::default();
        let mut text = trimmed.to_string();

        if self.opts.dictionary_enabled {
            let pass = self.dictionary.apply(&text);
            text = pass.text;
            for hit in pass.hits {
                out.changes.push(format!(
                    "Dictionary: '{}' -> '{}' ({}x)",
                    hit.source, hit.target, hit.count
                ));
                out.dictionary_hits
                    .extend(std::iter::repeat_n(hit.source, hit.count));
            }
            // The word list that guards real words is English only, so any
            // other (or auto-detected) language skips the fuzzy pass.
            if self.fuzzy_enabled() {
                let (fixed, replacements) = self.fuzzy_matcher().apply(&text);
                text = fixed;
                out.changes.extend(
                    replacements
                        .into_iter()
                        .map(|r| format!("Dictionary (fuzzy): '{}' -> '{}'", r.original, r.target)),
                );
            }
        }

        let mut expanded_snippet = false;
        if self.opts.snippets_enabled {
            if let Some(snippet) = self.memory.resolve_snippet(&text) {
                out.changes.push(format!(
                    "Snippet expanded: '{}' -> '{}'",
                    text, snippet.expansion
                ));
                text = snippet.expansion.clone();
                out.snippet_hits.push(snippet.trigger.clone());
                expanded_snippet = true;
            }
        }

        // Raw keeps exactly what was heard (after dictionary and snippets).
        let cleanup = self.opts.style_mode != StyleMode::Raw;

        // Runs before spoken formatting so "um, new line" still reaches the
        // line-break command with the filler already gone. A snippet
        // expansion is the user's own text and is never edited.
        if cleanup && self.opts.disfluency_cleanup && !expanded_snippet && self.opts.is_english() {
            let cleaned = disfluency::clean(&text);
            text = cleaned.text;
            out.changes.extend(cleaned.changes);
        }

        if cleanup && self.opts.spoken_formatting {
            let (formatted, changes) = formatting::format_spoken(&text);
            text = formatted;
            out.changes.extend(changes);
        }

        let speaker_set_breaks = out
            .changes
            .iter()
            .any(|c| c.starts_with("Spoken formatting:") && c.contains("line break"));
        if cleanup
            && self.opts.auto_paragraphs
            && !expanded_snippet
            && !speaker_set_breaks
            && formatting::auto_paragraphs_apply_to(target_app)
        {
            let (formatted, changes) = formatting::format_paragraphs(&text);
            text = formatted;
            out.changes.extend(changes);
        }

        out.text = text;
        out
    }

    /// Active dictionary targets and snippet triggers, most-used first (Mac
    /// `vocabularyBiasTerms`). Dictionary targets are the spellings the
    /// recognizer should produce; snippet triggers are the phrases the user
    /// actually speaks.
    pub fn vocabulary_terms(&self) -> Vec<String> {
        let mut targets: Vec<_> = self.memory.dictionary().iter().collect();
        targets.sort_by_key(|r| std::cmp::Reverse(r.usage_count));
        let mut triggers: Vec<_> = self.memory.snippets().iter().collect();
        triggers.sort_by_key(|s| std::cmp::Reverse(s.usage_count));
        targets
            .into_iter()
            .map(|r| r.target.clone())
            .chain(triggers.into_iter().map(|s| s.trigger.clone()))
            .collect()
    }

    /// Whisper initial prompt that biases decoding toward the user's words: a
    /// comma-separated sentence of bias terms (deduplicated, longest first,
    /// at most [`MAX_BIAS_TERMS`]), each fitted whole into about
    /// [`MAX_BIAS_PROMPT_TOKENS`] tokens. `None` when there is nothing to bias
    /// toward. The engine should use it only when `Settings::vocabulary_bias`
    /// is on, and (as on the Mac) only for audio of at most one 28-second
    /// Whisper window. Calling it also warms the fuzzy matcher and word list.
    pub fn vocabulary_prompt(&self) -> Option<String> {
        if self.fuzzy_enabled() {
            let _ = self.fuzzy_matcher();
            let _ = lexicon::is_available();
        }
        let terms = normalized_bias_terms(&self.vocabulary_terms(), MAX_BIAS_TERMS);
        let mut included: Vec<&str> = Vec::new();
        for term in &terms {
            let mut candidate = included.clone();
            candidate.push(term);
            let sentence = format!("{}.", candidate.join(", "));
            if estimated_tokens(&sentence) > MAX_BIAS_PROMPT_TOKENS {
                continue;
            }
            included = candidate;
        }
        (!included.is_empty()).then(|| format!("{}.", included.join(", ")))
    }

    /// Dictionary targets for Express's Word Boost, each with the sources the
    /// user mapped to it as aliases. Snippet triggers are left out.
    pub fn word_boost_terms(&self) -> Vec<WordBoostTerm> {
        let mut order: Vec<String> = Vec::new();
        let mut terms: std::collections::HashMap<String, WordBoostTerm> = Default::default();
        for rule in self.memory.dictionary() {
            let target = rule.target.trim();
            if target.is_empty() {
                continue;
            }
            let key = target.to_lowercase();
            let entry = terms.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                WordBoostTerm {
                    text: target.to_string(),
                    aliases: Vec::new(),
                }
            });
            let source = rule.source.trim();
            if !source.is_empty()
                && source.to_lowercase() != key
                && !entry
                    .aliases
                    .iter()
                    .any(|a| a.to_lowercase() == source.to_lowercase())
            {
                entry.aliases.push(source.to_string());
            }
        }
        order.into_iter().filter_map(|k| terms.remove(&k)).collect()
    }
}

/// Trims, drops empties, dedupes case-insensitively (first spelling wins),
/// orders longest-first (longer terms are the ones ASR mangles most and must
/// survive the cap), and caps the count.
pub fn normalized_bias_terms(terms: &[String], max_terms: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_lowercase()) {
            continue;
        }
        unique.push(trimmed.to_string());
    }
    unique.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
    unique.truncate(max_terms);
    unique
}

/// A conservative estimate of Whisper BPE tokens: about one token per three
/// letters of each word, plus one per punctuation mark.
pub fn estimated_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    for word in text.split_whitespace() {
        let letters = word.chars().filter(|c| c.is_alphanumeric()).count();
        let marks = word.chars().filter(|c| !c.is_alphanumeric()).count();
        tokens += letters.div_ceil(3).max(usize::from(letters > 0)) + marks;
    }
    tokens
}

/// Extra instruction for the polish model in the Formal and Casual style
/// modes. Raw and Clean are fully deterministic and need none.
pub fn style_instruction(mode: StyleMode) -> Option<&'static str> {
    match mode {
        StyleMode::Raw | StyleMode::Clean => None,
        StyleMode::Formal => Some(
            "Write it in a formal, professional register: complete sentences, no slang, and no \
             contractions (write \"do not\", not \"don't\"). Keep every fact, name and number, and \
             do not add anything that was not said.",
        ),
        StyleMode::Casual => Some(
            "Write it in a relaxed, conversational register: natural contractions and plain, \
             friendly wording, like a message to a colleague. Keep every fact, name and number, \
             and do not add anything that was not said.",
        ),
    }
}

#[cfg(test)]
mod tests;
