//! Which transcript pipeline and voice-edit rewriter the engine uses.
//!
//! The pipeline mirrors the Mac's finalize path: the text processor
//! (dictionary → fuzzy dictionary → snippets → filler cleanup → spoken
//! formatting → automatic paragraphs), then the optional local-LLM polish
//! (S1-mini or a general model), which never fails — it falls back to the
//! cleaned text. Express (Parakeet) output gets the Mac's number formatting
//! first. The engine rebuilds both on `settings-changed` and `memory-changed`,
//! so changes apply without a restart.

use orttaai_core::coordinator::{EditRewriter, PipelineOutput, TranscriptPipeline};
use orttaai_core::llm::{client_from_settings, LlmClient};
use orttaai_core::memory::MemoryService;
use orttaai_core::polish::{PolishOptions, Polisher};
use orttaai_core::settings::Settings;
use orttaai_core::store::Store;
use orttaai_core::text::{format_numbers, TextOptions, TextProcessor};
use std::sync::Arc;
use std::time::Instant;

/// The full desktop pipeline: text processor + optional polish.
pub struct AppPipeline {
    processor: TextProcessor,
    polisher: Option<Polisher>,
    express: bool,
}

impl TranscriptPipeline for AppPipeline {
    fn process(&self, raw: &str, app: Option<&str>) -> PipelineOutput {
        let started = Instant::now();
        let input = if self.express {
            format_numbers(raw)
        } else {
            raw.to_string()
        };
        let out = self.processor.process_for_app(&input, app);
        let text_ms = started.elapsed().as_millis() as i64;

        // A snippet is the user's own exact text — never rewrite it (Mac skips
        // polish when a snippet expanded).
        let (text, polished, polish_ms) = match &self.polisher {
            Some(p) if out.snippet_hits.is_empty() && !out.text.trim().is_empty() => {
                let o = p.polish(&out.text, app);
                (o.text, o.polished, o.ms)
            }
            _ => (out.text, false, 0),
        };
        PipelineOutput {
            text,
            dictionary_hits: out.dictionary_hits,
            snippet_hits: out.snippet_hits,
            polished,
            text_ms,
            polish_ms,
        }
    }
}

/// The text processor for these settings, falling back to an empty memory if
/// the store can't be opened (dictation must never fail on it).
fn processor(settings: &Settings) -> TextProcessor {
    match Store::open_default().and_then(|s| s.text_processor(settings)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("orttaai: personal memory unavailable ({e}); dictating without it");
            TextProcessor::new(TextOptions::from_settings(settings), MemoryService::new())
        }
    }
}

/// The transcript pipeline for these settings.
pub fn build_pipeline(settings: &Settings) -> Box<dyn TranscriptPipeline> {
    let processor = processor(settings);
    // Formal and Casual are written by Polish, so they apply only while Polish
    // is on (Settings › Text says so); otherwise dictation reads as Clean.
    let polisher = settings.polish_enabled.then(|| {
        let opts = PolishOptions::from_settings(settings);
        let protected = processor.vocabulary_terms();
        Polisher::new(
            client_from_settings(settings),
            opts.with_protected_words(protected),
        )
    });
    Box::new(AppPipeline {
        processor,
        polisher,
        express: settings.express_active(),
    })
}

/// Voice edit through the local LLM (Mac `EditCommandProcessor`).
struct LlmRewriter {
    client: Arc<dyn LlmClient>,
    model: String,
    timeout_ms: u64,
    max_chars: usize,
}

impl EditRewriter for LlmRewriter {
    fn rewrite(&self, selection: &str, instruction: &str) -> orttaai_core::Result<String> {
        orttaai_core::edit::rewrite_selection(
            self.client.as_ref(),
            &self.model,
            selection,
            instruction,
            self.timeout_ms,
            self.max_chars,
        )
    }
}

/// The model voice edit uses: the polish model, as Settings says, unless it's
/// S1-mini (a polish-only fine-tune that can't follow edit instructions); then
/// the chat or insights model.
pub fn edit_model(settings: &Settings) -> Option<String> {
    [
        &settings.polish_model,
        &settings.chat_model,
        &settings.insights_model,
    ]
    .into_iter()
    .map(|m| m.trim())
    .find(|m| !m.is_empty() && !m.to_ascii_lowercase().contains("s1-mini"))
    .map(str::to_string)
}

/// The voice-edit rewriter for these settings; `None` when voice edit is off
/// or no local model is configured (the engine then explains what's missing).
pub fn build_rewriter(settings: &Settings) -> Option<Box<dyn EditRewriter>> {
    if !settings.edit_commands_enabled {
        return None;
    }
    let model = edit_model(settings)?;
    Some(Box::new(LlmRewriter {
        client: client_from_settings(settings),
        model,
        timeout_ms: settings.edit_timeout_ms,
        max_chars: settings.edit_max_chars,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_model_skips_s1_mini_and_blanks() {
        let s = Settings {
            chat_model: " ".into(),
            polish_model: "hf.co/superwhisper/s1-mini-GGUF:Q4_K_M".into(),
            insights_model: "qwen3:4b".into(),
            ..Default::default()
        };
        assert_eq!(edit_model(&s).as_deref(), Some("qwen3:4b"));
        let none = Settings {
            chat_model: String::new(),
            polish_model: String::new(),
            insights_model: String::new(),
            ..Default::default()
        };
        assert!(edit_model(&none).is_none());
        let polish_first = Settings {
            chat_model: "llama3.2:3b".into(),
            polish_model: "qwen3:4b".into(),
            ..Default::default()
        };
        assert_eq!(edit_model(&polish_first).as_deref(), Some("qwen3:4b"));
    }

    #[test]
    fn pipeline_cleans_and_skips_polish_when_off() {
        let s = Settings {
            polish_enabled: false,
            ..Default::default()
        };
        let p = AppPipeline {
            processor: TextProcessor::new(TextOptions::from_settings(&s), MemoryService::new()),
            polisher: None,
            express: false,
        };
        let out = p.process("um so the meeting is at noon", None);
        assert!(!out.polished);
        assert!(!out.text.to_lowercase().starts_with("um"), "{}", out.text);
    }

    #[test]
    fn rewriter_needs_voice_edit_on() {
        let s = Settings {
            edit_commands_enabled: false,
            chat_model: "qwen3:4b".into(),
            ..Default::default()
        };
        assert!(build_rewriter(&s).is_none());
    }
}
