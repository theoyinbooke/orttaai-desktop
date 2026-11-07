//! Shared domain types. These are OS-agnostic and serializable so the UI can
//! consume them directly over Tauri's IPC.

use serde::{Deserialize, Serialize};

/// Target sample rate for the transcription engine (Whisper expects 16 kHz mono).
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// The countdown (and its red warning treatment in the pill) covers the final
/// 20 seconds of the recording window, for both push-to-talk and hands-free
/// (Mac `DictationCoordinator.countdownWarningWindowSeconds`).
pub const COUNTDOWN_WARNING_SECS: u32 = 20;

/// Where the dictation state machine currently is. Drives the tray icon and panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingState {
    Idle,
    Recording,
    Processing,
    Error,
}

/// How the active recording is being driven (Mac `RecordingMode`). Push-to-talk
/// is the classic hold-the-hotkey mode; hands-free keeps recording after the key
/// is released (started by a tap) and stops on the next tap, the overlay's stop
/// button, sustained silence, or its own duration cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    #[default]
    PushToTalk,
    HandsFree,
}

/// What the active session is for (Mac `SessionKind`). A dictation injects the
/// transcript; an edit treats it as an instruction applied to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    #[default]
    Dictation,
    Edit,
}

impl SessionKind {
    /// The `entry_kind` stored with a history record.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Dictation => "dictation",
            SessionKind::Edit => "edit",
        }
    }
}

/// Outcome of a text-injection attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionResult {
    Success,
    BlockedSecureField,
    NoTranscript,
    /// A transcript was produced but typing it into the focused app failed
    /// (e.g. GNOME/Wayland has no virtual-keyboard protocol for `wtype`).
    Failed,
}

/// How a transcript actually reached the focused app — stored with each history
/// record (`TranscriptionRecord::injection_method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMethod {
    /// Synthesized keystrokes (enigo / SendInput / XTest / wtype).
    Typed,
    /// Keystrokes through the XDG RemoteDesktop portal (GNOME/Wayland).
    Portal,
    /// Clipboard + paste chord (Ctrl+V, Shift+Insert fallback).
    Paste,
    /// Every delivery path failed; the text was left on the clipboard so the
    /// user can paste it manually.
    Clipboard,
    /// Nothing could be delivered, not even to the clipboard.
    Failed,
    /// Refused: the focused field is (or may be) a password field.
    Blocked,
}

impl InjectionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            InjectionMethod::Typed => "typed",
            InjectionMethod::Portal => "portal",
            InjectionMethod::Paste => "paste",
            InjectionMethod::Clipboard => "clipboard",
            InjectionMethod::Failed => "failed",
            InjectionMethod::Blocked => "blocked",
        }
    }

    /// Whether the text reached the focused app.
    pub fn delivered(self) -> bool {
        matches!(
            self,
            InjectionMethod::Typed | InjectionMethod::Portal | InjectionMethod::Paste
        )
    }
}

/// Whether the focused field is a secure/password field. `Unknown` is the honest
/// answer on Wayland, where there is no reliable detection API (see `docs/gaps.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecureFieldStatus {
    Secure,
    NotSecure,
    Unknown,
}

/// Speed/accuracy preset, mapped to concrete decoding parameters by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DecodePreset {
    Fast,
    #[default]
    Balanced,
    Accuracy,
}

/// Manual whisper decoding knobs (Mac "Advanced Decoding"), already validated.
/// When present they replace the preset baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeTuning {
    pub temperature: f32,
    /// Candidates sampled per fallback step (whisper `best_of`).
    pub best_of: i32,
    /// How many higher-temperature retries a low-confidence decode may take.
    pub fallback_count: i32,
    /// Repetition guard (whisper.cpp `entropy_thold`, the analogue of
    /// OpenAI's compression-ratio threshold).
    pub compression_ratio_threshold: f32,
    pub logprob_threshold: f32,
    pub no_speech_threshold: f32,
}

/// Decoding options handed to the transcriber.
///
/// The persisted fields live in `Settings::decode`; the `#[serde(skip)]` ones
/// are per-session runtime inputs the coordinator fills in (vocabulary-bias
/// prompt, advanced tuning, low-latency mode) and never reach the settings file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DecodeOptions {
    /// `None` (or `"auto"`) means auto-detect the language.
    pub language: Option<String>,
    pub preset: DecodePreset,
    pub temperature: f32,
    /// Decode threads; `0` lets the engine pick (≈ physical performance cores).
    pub n_threads: i32,
    /// Vocabulary-bias prompt (whisper `initial_prompt`): the user's dictionary
    /// words, so the recognizer spells them right.
    #[serde(skip)]
    pub initial_prompt: Option<String>,
    /// Advanced decoding overrides; `None` uses the preset baseline.
    #[serde(skip)]
    pub tuning: Option<DecodeTuning>,
    /// Low Latency Mode: greedy decode, never a temperature fallback.
    #[serde(skip)]
    pub low_latency: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            language: Some("en".to_string()),
            preset: DecodePreset::Balanced,
            temperature: 0.0,
            n_threads: 0,
            initial_prompt: None,
            tuning: None,
            low_latency: false,
        }
    }
}

/// A keyboard modifier for a global hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

/// A global hotkey combination, e.g. Ctrl+Shift+Space for push-to-talk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyCombo {
    pub key: String,
    pub modifiers: Vec<Modifier>,
}

impl Default for HotkeyCombo {
    /// The macOS app's default push-to-talk chord.
    fn default() -> Self {
        Self {
            key: "Space".to_string(),
            modifiers: vec![Modifier::Ctrl, Modifier::Shift],
        }
    }
}

impl HotkeyCombo {
    /// `"Ctrl+Shift+Space"`-style display string (inverse of [`parse`](Self::parse)).
    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = self
            .modifiers
            .iter()
            .map(|m| match m {
                Modifier::Ctrl => "Ctrl",
                Modifier::Shift => "Shift",
                Modifier::Alt => "Alt",
                Modifier::Meta => "Meta",
            })
            .collect();
        parts.push(&self.key);
        parts.join("+")
    }

    /// Best-effort parse of a `"Ctrl+Shift+Space"`-style string.
    pub fn parse(s: &str) -> Self {
        let mut modifiers = Vec::new();
        let mut key = "Space".to_string();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.push(Modifier::Ctrl),
                "shift" => modifiers.push(Modifier::Shift),
                "alt" | "option" => modifiers.push(Modifier::Alt),
                "meta" | "cmd" | "command" | "super" | "win" => modifiers.push(Modifier::Meta),
                _ => key = part.to_string(),
            }
        }
        Self { key, modifiers }
    }

    /// Same physical chord, regardless of modifier order.
    pub fn same_chord(&self, other: &HotkeyCombo) -> bool {
        let norm = |c: &HotkeyCombo| {
            let mut m: Vec<u8> = c
                .modifiers
                .iter()
                .map(|m| match m {
                    Modifier::Ctrl => 0,
                    Modifier::Shift => 1,
                    Modifier::Alt => 2,
                    Modifier::Meta => 3,
                })
                .collect();
            m.sort_unstable();
            m.dedup();
            (c.key.to_ascii_lowercase(), m)
        };
        norm(self) == norm(other)
    }
}

/// Identifies an audio input device. Opaque string so each backend can use its
/// own naming (PipeWire node, WASAPI endpoint, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceId(pub String);

/// An input device as listed in Settings › Audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDevice {
    /// The persistent name stored in `Settings::audio_device`.
    pub name: String,
    pub is_default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_runtime_fields_never_serialize() {
        let opts = DecodeOptions {
            initial_prompt: Some("Orttaai".into()),
            low_latency: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(!json.contains("initial_prompt"));
        assert!(!json.contains("low_latency"));
        let back: DecodeOptions = serde_json::from_str(r#"{"preset":"fast"}"#).unwrap();
        assert_eq!(back.preset, DecodePreset::Fast);
        assert_eq!(back.language.as_deref(), Some("en"));
    }

    #[test]
    fn same_chord_ignores_modifier_order() {
        let a = HotkeyCombo::parse("Ctrl+Shift+E");
        let b = HotkeyCombo::parse("shift+ctrl+e");
        assert!(a.same_chord(&b));
        assert!(!a.same_chord(&HotkeyCombo::default()));
    }

    #[test]
    fn injection_method_names_match_history_column() {
        assert_eq!(InjectionMethod::Paste.as_str(), "paste");
        assert!(InjectionMethod::Portal.delivered());
        assert!(!InjectionMethod::Clipboard.delivered());
    }
}
