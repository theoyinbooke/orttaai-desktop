//! User settings. Serialized to JSON under the platform config dir
//! (`~/.config/orttaai` on Linux, `%APPDATA%\orttaai` on Windows) via `directories`.
//!
//! Mirrors the macOS app's `AppSettings` (UserDefaults keys) so every toggle the
//! Mac exposes has a home here. Every field is `#[serde(default)]`, so older
//! settings files load cleanly and new fields pick up their defaults.

use crate::error::Result;
use crate::types::{DecodeOptions, HotkeyCombo, Modifier};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Which speech engine transcribes dictations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeechEngine {
    /// whisper.cpp (GGUF models) — every language, every platform.
    #[default]
    Whisper,
    /// Parakeet TDT via sherpa-onnx — English-only, fastest on CPU ("Express").
    Express,
}

/// How transcripts are cleaned before they are typed (Mac `TextStyleMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StyleMode {
    /// Exactly what the recognizer heard (after dictionary/snippets).
    Raw,
    /// Fillers and stutters removed, light punctuation/casing fixes.
    #[default]
    Clean,
    Formal,
    Casual,
}

/// Which local LLM server backs the AI features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    #[default]
    Ollama,
    LmStudio,
}

/// Which engine polishes transcripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishEngine {
    /// The S1-mini polish model (Qwen3 0.6B fine-tune), served by the local LLM host.
    #[default]
    S1Mini,
    /// Any general model on the local LLM host.
    LocalLlm,
}

/// How text reaches the focused app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    /// Paste for long text (fast, keeps formatting), type for short text.
    #[default]
    Auto,
    /// Always synthesize keystrokes.
    Type,
    /// Always paste via the clipboard (restored afterwards when `preserve_clipboard`).
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDepth {
    #[default]
    Low,
    Medium,
    High,
}

/// Whisper's advanced decoding knobs (Mac "Advanced Decoding" sliders).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedDecoding {
    pub enabled: bool,
    pub temperature: f32,
    pub best_of: i32,
    pub temperature_fallback_count: i32,
    pub compression_ratio_threshold: f32,
    pub logprob_threshold: f32,
    pub no_speech_threshold: f32,
}

impl Default for AdvancedDecoding {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 0.0,
            best_of: 5,
            temperature_fallback_count: 3,
            compression_ratio_threshold: 2.4,
            logprob_threshold: -1.0,
            no_speech_threshold: 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- Speech model ------------------------------------------------------
    pub model_id: String,
    pub engine: SpeechEngine,
    pub decode: DecodeOptions,
    pub advanced_decoding: AdvancedDecoding,
    /// Bias decoding towards dictionary words (whisper initial prompt).
    pub vocabulary_bias: bool,
    pub express_word_boost: bool,
    pub low_latency: bool,
    /// Keep the model resident and run a silent warm-up decode at start.
    pub warm_models: bool,

    // ---- Dictation ---------------------------------------------------------
    pub push_to_talk: HotkeyCombo,
    /// A short tap of the shortcut starts hands-free recording.
    pub hands_free_enabled: bool,
    pub hands_free_silence_stop: bool,
    /// Seconds of silence before a hands-free recording stops (1–10).
    pub hands_free_silence_secs: f32,
    /// Hands-free hard cap in seconds (120–1800).
    pub hands_free_max_secs: u32,
    /// Push-to-talk hard cap in seconds (10–300).
    pub max_recording_secs: u32,
    pub dictation_language: String,
    pub audio_device: Option<String>,
    pub sound_cues: bool,

    // ---- Voice edit --------------------------------------------------------
    pub edit_commands_enabled: bool,
    pub edit_shortcut: HotkeyCombo,
    pub edit_timeout_ms: u64,
    pub edit_max_chars: usize,

    // ---- Text --------------------------------------------------------------
    pub style_mode: StyleMode,
    pub spoken_formatting: bool,
    pub auto_paragraphs: bool,
    pub dictionary_enabled: bool,
    pub fuzzy_dictionary: bool,
    pub snippets_enabled: bool,
    pub disfluency_cleanup: bool,
    pub ai_suggestions: bool,
    pub learn_from_edits: bool,

    // ---- Injection ---------------------------------------------------------
    pub injection_mode: InjectionMode,
    pub preserve_clipboard: bool,
    /// XDG RemoteDesktop portal restore token (Wayland) — lets us re-open the
    /// input-injection session on later launches without re-prompting.
    pub wayland_restore_token: Option<String>,
    /// Refuse to type when the focused field can't be confirmed non-secure.
    pub strict_secure: bool,

    // ---- Local AI ----------------------------------------------------------
    pub llm_provider: LlmProvider,
    pub ollama_endpoint: String,
    pub lmstudio_endpoint: String,
    pub polish_enabled: bool,
    pub polish_engine: PolishEngine,
    pub polish_model: String,
    pub polish_timeout_ms: u64,
    pub polish_max_chars: usize,
    pub chat_model: String,
    pub chat_thinking: ThinkingDepth,
    pub insights_enabled: bool,
    pub insights_model: String,
    pub insights_context_tokens: u32,
    pub insights_thinking: bool,
    pub semantic_enabled: bool,
    pub semantic_auto_index: bool,
    pub semantic_keyword_fallback: bool,
    pub semantic_embedding_model: String,
    pub semantic_summary_enabled: bool,
    pub semantic_summary_model: String,

    // ---- Coach -------------------------------------------------------------
    pub coach_enabled: bool,
    pub coach_first_run_done: bool,
    /// The coach for every way of talking without its own pick in `coach_personas`.
    pub coach_persona: String,
    /// Per-app "way of talking" override, keyed like the Mac's `CoachAppCatalog.key`
    /// (the lowercased app id, else `name:` + the lowercased app name) →
    /// `ai|people|writing|elsewhere|off`.
    pub coach_app_overrides: BTreeMap<String, String>,
    /// Plays the scene partners; empty = the coach model.
    pub coach_partner_model: String,
    /// Answers the judged checks; empty = automatic (the best model that passed
    /// the judge check, else the one recommended for this machine's memory).
    pub coach_judge_model: String,
    /// Writes the note after each rep; empty = automatic, as for the judge.
    pub coach_model: String,
    /// The coach per way of talking (`ai|people|writing|elsewhere` → persona id).
    pub coach_personas: BTreeMap<String, String>,
    /// Read the coach's note aloud after each rep.
    pub coach_read_aloud: bool,
    /// Spoken scenes: your turn starts when the partner stops and ends on a pause.
    pub coach_hands_free: bool,
    /// Model setup was finished or put off, so it doesn't open by itself again.
    pub coach_model_setup_seen: bool,
    /// Preferred system voice for Coach (WebView speech synthesis); empty = automatic.
    pub coach_voice: String,

    // ---- General -----------------------------------------------------------
    pub appearance: Appearance,
    pub launch_at_login: bool,
    pub start_engine_on_launch: bool,
    pub show_processing_estimate: bool,
    pub celebrate_milestones: bool,
    pub has_completed_setup: bool,
    pub auto_download_updates: bool,
    pub analytics_time_range: String,
    /// Version whose What's New card was last shown.
    pub last_whats_new_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_id: "ggml-base.en-q5_1".to_string(),
            engine: SpeechEngine::Whisper,
            decode: DecodeOptions::default(),
            advanced_decoding: AdvancedDecoding::default(),
            vocabulary_bias: true,
            express_word_boost: true,
            low_latency: false,
            warm_models: true,

            push_to_talk: HotkeyCombo::default(),
            hands_free_enabled: true,
            hands_free_silence_stop: true,
            hands_free_silence_secs: 4.0,
            hands_free_max_secs: 600,
            max_recording_secs: 90,
            dictation_language: "en".to_string(),
            audio_device: None,
            sound_cues: true,

            edit_commands_enabled: false,
            edit_shortcut: HotkeyCombo {
                key: "E".to_string(),
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
            },
            edit_timeout_ms: 6000,
            edit_max_chars: 1500,

            style_mode: StyleMode::Clean,
            spoken_formatting: true,
            auto_paragraphs: true,
            dictionary_enabled: true,
            fuzzy_dictionary: true,
            snippets_enabled: true,
            disfluency_cleanup: true,
            ai_suggestions: true,
            learn_from_edits: false,

            injection_mode: InjectionMode::Auto,
            preserve_clipboard: true,
            wayland_restore_token: None,
            strict_secure: false,

            llm_provider: LlmProvider::Ollama,
            ollama_endpoint: "http://localhost:11434".to_string(),
            lmstudio_endpoint: "http://localhost:1234".to_string(),
            polish_enabled: false,
            polish_engine: PolishEngine::S1Mini,
            polish_model: "gemma4:e2b".to_string(),
            polish_timeout_ms: 3000,
            polish_max_chars: 400,
            chat_model: String::new(),
            chat_thinking: ThinkingDepth::Low,
            insights_enabled: false,
            insights_model: String::new(),
            insights_context_tokens: 16_384,
            insights_thinking: false,
            semantic_enabled: false,
            semantic_auto_index: true,
            semantic_keyword_fallback: true,
            semantic_embedding_model: "all-minilm".to_string(),
            semantic_summary_enabled: false,
            semantic_summary_model: String::new(),

            coach_enabled: true,
            coach_first_run_done: false,
            coach_persona: "mentor".to_string(),
            coach_app_overrides: BTreeMap::new(),
            coach_partner_model: "nemotron-3.5-bolt:e4b".to_string(),
            coach_judge_model: String::new(),
            coach_model: String::new(),
            coach_personas: BTreeMap::new(),
            coach_read_aloud: false,
            coach_hands_free: true,
            coach_model_setup_seen: false,
            coach_voice: String::new(),

            appearance: Appearance::System,
            launch_at_login: false,
            start_engine_on_launch: true,
            show_processing_estimate: true,
            celebrate_milestones: true,
            has_completed_setup: false,
            auto_download_updates: true,
            analytics_time_range: "30d".to_string(),
            last_whats_new_version: None,
        }
    }
}

impl Settings {
    /// Cross-platform config-file path, or `None` if no home dir can be resolved.
    pub fn config_path() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("ORTTAAI_CONFIG_DIR") {
            return Some(PathBuf::from(dir).join("settings.json"));
        }
        ProjectDirs::from("org", "orttaai", "Orttaai")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    /// Load settings from disk, falling back to defaults on any error.
    pub fn load_or_default() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(json) => Self::from_json_lenient(&json),
            Err(_) => Self::default(),
        }
    }

    /// Parse settings, migrating keys from older desktop builds. Never fails:
    /// unknown/invalid input yields defaults.
    pub fn from_json_lenient(json: &str) -> Self {
        let mut s: Settings = serde_json::from_str(json).unwrap_or_default();
        // v0.2.x stored the old default model; keep whatever the user picked.
        s.clamp();
        s
    }

    /// Persist settings to the config path, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()
            .ok_or_else(|| std::io::Error::other("no config directory available"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        // Write-then-rename so a crash mid-write never corrupts settings.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Merge a JSON object of changed fields into these settings (the UI sends
    /// partial patches). Invalid patches leave settings untouched.
    pub fn merged(&self, patch: &serde_json::Value) -> Result<Self> {
        let mut base = serde_json::to_value(self)?;
        merge_json(&mut base, patch);
        let mut next: Settings = serde_json::from_value(base)?;
        next.clamp();
        Ok(next)
    }

    /// Keep numeric settings inside the ranges the UI allows.
    pub fn clamp(&mut self) {
        self.hands_free_silence_secs = self.hands_free_silence_secs.clamp(1.0, 10.0);
        self.hands_free_max_secs = self.hands_free_max_secs.clamp(120, 1800);
        self.max_recording_secs = self.max_recording_secs.clamp(10, 300);
        self.decode.n_threads = self.decode.n_threads.clamp(0, 256);
        self.polish_timeout_ms = self.polish_timeout_ms.clamp(500, 30_000);
        self.polish_max_chars = self.polish_max_chars.clamp(50, 5000);
        self.edit_timeout_ms = self.edit_timeout_ms.clamp(1000, 60_000);
        self.edit_max_chars = self.edit_max_chars.clamp(100, 20_000);
        self.insights_context_tokens = self.insights_context_tokens.clamp(2048, 131_072);
    }

    /// The endpoint for the configured LLM provider.
    pub fn llm_endpoint(&self) -> &str {
        match self.llm_provider {
            LlmProvider::Ollama => &self.ollama_endpoint,
            LlmProvider::LmStudio => &self.lmstudio_endpoint,
        }
    }
}

/// Recursive JSON object merge: objects merge key-by-key, everything else replaces.
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge_json(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_files_still_load() {
        let old = r#"{"model_id":"ggml-small.en","push_to_talk":{"key":"Space","modifiers":["ctrl","shift"]},"preserve_clipboard":false,"low_latency":false,"ollama_endpoint":"http://x:1"}"#;
        let s = Settings::from_json_lenient(old);
        assert_eq!(s.model_id, "ggml-small.en");
        assert!(!s.preserve_clipboard);
        assert_eq!(s.ollama_endpoint, "http://x:1");
        assert!(s.hands_free_enabled);
        assert_eq!(s.max_recording_secs, 90);
    }

    #[test]
    fn garbage_yields_defaults() {
        let s = Settings::from_json_lenient("not json");
        assert_eq!(s.model_id, Settings::default().model_id);
    }

    #[test]
    fn merge_patch_updates_nested_and_clamps() {
        let s = Settings::default();
        let next = s
            .merged(&serde_json::json!({
                "decode": {"preset": "fast"},
                "max_recording_secs": 9999,
                "style_mode": "formal"
            }))
            .unwrap();
        assert_eq!(next.decode.preset, crate::types::DecodePreset::Fast);
        assert_eq!(next.decode.language.as_deref(), Some("en"));
        assert_eq!(next.max_recording_secs, 300);
        assert_eq!(next.style_mode, StyleMode::Formal);
    }

    #[test]
    fn bad_patch_errors() {
        let s = Settings::default();
        assert!(s
            .merged(&serde_json::json!({"max_recording_secs": "x"}))
            .is_err());
    }
}
