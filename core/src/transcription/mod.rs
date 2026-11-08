//! Speech-to-text. The real backend (whisper.cpp via `whisper-rs`) lives behind
//! the `whisper` feature; everything above this trait is portable.
//!
//! This module also owns the Mac `TranscriptionService` policy that is not
//! backend-specific: decode options from settings (preset, language, Low
//! Latency, Advanced Decoding), the vocabulary-bias prompt, and the warm-up.

use crate::error::Result;
use crate::settings::Settings;
use crate::types::{DecodeOptions, DecodeTuning, TARGET_SAMPLE_RATE};

pub trait Transcriber: Send + Sync {
    /// Load a model by id (e.g. `"ggml-base.en"`).
    fn load_model(&mut self, model_id: &str) -> Result<()>;

    /// The currently loaded model id, if any.
    fn loaded_model(&self) -> Option<&str>;

    /// Transcribe 16 kHz mono `f32` samples to text.
    fn transcribe(&self, samples: &[f32], opts: &DecodeOptions) -> Result<String>;

    /// How this engine decodes while the speaker is still talking.
    fn live_mode(&self) -> crate::live::LiveMode {
        crate::live::LiveMode::Off
    }

    /// Word Boost (Express): the dictionary words to listen for, snapshotted
    /// per dictation. Engines without Word Boost ignore them.
    fn set_word_boost_terms(&self, _terms: &[crate::text::WordBoostTerm]) {}

    /// Whether Word Boost has words to check in the next dictation.
    fn word_boost_active(&self) -> bool {
        false
    }

    /// Score the finished 15 s chunks of a recording that is still going, so
    /// Word Boost has little left to do at release (Mac `prepareWordBoost`).
    fn prepare_word_boost(&self, _prefix: &[f32]) {}

    /// Forget chunks scored for a recording that won't be transcribed.
    fn discard_prepared_word_boost(&self) {}
}

/// Run a silent warm-up decode so the first real dictation doesn't pay the
/// cold-start cost (Mac `warmUp`: two seconds, since shorter clips can skip
/// the encoder). An empty result is expected and fine.
pub fn warm_up(transcriber: &dyn Transcriber, opts: &DecodeOptions) -> Result<()> {
    let silence = vec![0.0f32; 2 * TARGET_SAMPLE_RATE as usize];
    let opts = DecodeOptions {
        initial_prompt: None,
        ..opts.clone()
    };
    transcriber.transcribe(&silence, &opts).map(|_| ())
}

/// Decode options for the current settings: the dictation language (`"auto"`
/// = detect), the preset,
/// Advanced Decoding overrides (validated) and thread count.
pub fn decode_options_from_settings(settings: &Settings) -> DecodeOptions {
    let lang = settings.dictation_language.trim();
    let language = if lang.is_empty() || lang.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(lang.to_ascii_lowercase())
    };
    let adv = &settings.advanced_decoding;
    let tuning = adv.enabled.then(|| DecodeTuning {
        temperature: adv.temperature.clamp(0.0, 1.0),
        best_of: adv.best_of.clamp(1, 10),
        fallback_count: adv.temperature_fallback_count.clamp(0, 10),
        compression_ratio_threshold: adv.compression_ratio_threshold.clamp(1.0, 5.0),
        logprob_threshold: adv.logprob_threshold.clamp(-5.0, 0.0),
        no_speech_threshold: adv.no_speech_threshold.clamp(0.0, 1.0),
    });
    DecodeOptions {
        language,
        preset: settings.decode.preset,
        temperature: settings.decode.temperature,
        n_threads: settings.decode.n_threads,
        initial_prompt: None,
        tuning,
    }
}

/// Cap for the bias vocabulary (Mac `maxBiasTermCount`).
pub const MAX_BIAS_TERMS: usize = 40;
/// Token budget for the bias prompt (Mac `maxBiasPromptTokenCount`): prefill
/// runs the decoder over every prompt token, so it is kept small.
pub const MAX_BIAS_PROMPT_TOKENS: usize = 32;
/// Longest audio a bias prompt may condition: one Whisper window. Longer
/// decodes re-apply the prompt per window, which the Mac eval measured
/// compounding into fallback cascades.
pub const BIAS_PROMPT_MAX_SAMPLES: usize = 28 * TARGET_SAMPLE_RATE as usize;

/// Trims, drops empties, dedupes case-insensitively (first spelling wins),
/// orders longest-first (longer terms are the ones ASR mangles most and must
/// survive the cap), and caps the count.
pub fn normalized_bias_terms(terms: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for term in terms {
        let t = term.trim();
        if t.is_empty() || !seen.insert(t.to_lowercase()) {
            continue;
        }
        unique.push(t.to_string());
    }
    unique.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
    unique.truncate(MAX_BIAS_TERMS);
    unique
}

/// Rough whisper BPE token count (≈ 4 characters per token, at least one per
/// word) — whisper.cpp tokenizes the prompt itself, so this only budgets.
fn approx_tokens(s: &str) -> usize {
    let words = s.split_whitespace().count().max(1);
    s.chars().count().div_ceil(4).max(words)
}

/// The vocabulary-bias prompt: the user's terms as a comma-separated sentence,
/// fitted whole (a term that would overflow the budget is dropped, never cut).
pub fn bias_prompt(terms: &[String]) -> Option<String> {
    let mut used = 0usize;
    let mut kept: Vec<String> = Vec::new();
    for term in normalized_bias_terms(terms) {
        let cost = approx_tokens(&term) + 1; // + separator
        if used + cost > MAX_BIAS_PROMPT_TOKENS {
            continue;
        }
        used += cost;
        kept.push(term);
    }
    if kept.is_empty() {
        return None;
    }
    Some(format!("{}.", kept.join(", ")))
}

/// The bias prompt to use for this audio: skipped for silent audio (prompt
/// bleed-through) and for audio longer than one Whisper window.
pub fn bias_prompt_for(terms: &[String], samples: &[f32]) -> Option<String> {
    if terms.is_empty()
        || samples.len() > BIAS_PROMPT_MAX_SAMPLES
        || !crate::vad::contains_speech_energy(samples)
    {
        return None;
    }
    bias_prompt(terms)
}

/// Deterministic transcriber for tests and the `demo` CLI on any OS.
pub struct MockTranscriber {
    canned: String,
    model: Option<String>,
    seen: std::sync::Arc<std::sync::Mutex<Vec<DecodeOptions>>>,
}

impl MockTranscriber {
    pub fn new(canned: impl Into<String>) -> Self {
        Self {
            canned: canned.into(),
            model: Some("mock".to_string()),
            seen: Default::default(),
        }
    }

    /// The options each `transcribe` call received.
    pub fn calls(&self) -> std::sync::Arc<std::sync::Mutex<Vec<DecodeOptions>>> {
        self.seen.clone()
    }
}

impl Transcriber for MockTranscriber {
    fn load_model(&mut self, model_id: &str) -> Result<()> {
        self.model = Some(model_id.to_string());
        Ok(())
    }

    fn loaded_model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn transcribe(&self, _samples: &[f32], opts: &DecodeOptions) -> Result<String> {
        self.seen.lock().unwrap().push(opts.clone());
        Ok(self.canned.clone())
    }
}

#[cfg(feature = "whisper")]
mod whisper;
#[cfg(feature = "whisper")]
pub use whisper::{whisper_params, WhisperParams, WhisperTranscriber};

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bias_terms_dedupe_and_order_longest_first() {
        let t = normalized_bias_terms(&terms(&["Jon", " Orttaai ", "jon", "", "Kubernetes"]));
        assert_eq!(t, vec!["Kubernetes", "Orttaai", "Jon"]);
    }

    #[test]
    fn bias_prompt_fits_budget_with_whole_terms() {
        let many: Vec<String> = (0..60).map(|i| format!("Term{i:02}")).collect();
        let p = bias_prompt(&many).unwrap();
        assert!(approx_tokens(&p) <= MAX_BIAS_PROMPT_TOKENS + 2, "{p}");
        assert!(p.ends_with('.'));
        assert!(p
            .split(", ")
            .all(|t| t.trim_end_matches('.').starts_with("Term")));
        assert!(bias_prompt(&[]).is_none());
    }

    #[test]
    fn bias_prompt_skips_silence_and_long_audio() {
        let t = terms(&["Orttaai"]);
        assert!(bias_prompt_for(&t, &vec![0.0; 16_000]).is_none());
        let speech: Vec<f32> = (0..16_000)
            .map(|i| ((i as f32) * 0.1).sin() * 0.3)
            .collect();
        assert_eq!(bias_prompt_for(&t, &speech).as_deref(), Some("Orttaai."));
        let long = vec![0.3; BIAS_PROMPT_MAX_SAMPLES + 1];
        assert!(bias_prompt_for(&t, &long).is_none());
    }

    #[test]
    fn settings_map_to_decode_options() {
        let mut s = Settings {
            dictation_language: "auto".into(),
            ..Default::default()
        };
        assert_eq!(decode_options_from_settings(&s).language, None);
        s.dictation_language = "FR".into();
        assert_eq!(
            decode_options_from_settings(&s).language.as_deref(),
            Some("fr")
        );
        assert!(decode_options_from_settings(&s).tuning.is_none());
        s.advanced_decoding.enabled = true;
        s.advanced_decoding.best_of = 99;
        let t = decode_options_from_settings(&s).tuning.unwrap();
        assert_eq!(t.best_of, 10);
    }
}

#[cfg(feature = "express")]
pub mod express;
#[cfg(feature = "express")]
pub use express::ExpressTranscriber;
