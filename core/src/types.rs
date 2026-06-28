//! Shared domain types. These are OS-agnostic and serializable so the UI can
//! consume them directly over Tauri's IPC.

use serde::{Deserialize, Serialize};

/// Target sample rate for the transcription engine (Whisper expects 16 kHz mono).
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Hard recording cap and the countdown warning, mirroring the macOS app.
pub const MAX_RECORDING_SECS: u32 = 45;
pub const COUNTDOWN_WARNING_SECS: u32 = 35;

/// Where the dictation state machine currently is. Drives the tray icon and panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingState {
    Idle,
    Recording,
    Processing,
    Error,
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

/// Decoding options handed to the transcriber. Param ranges are re-validated
/// against whisper.cpp when the real backend lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeOptions {
    /// `None` means auto-detect the language.
    pub language: Option<String>,
    pub preset: DecodePreset,
    pub temperature: f32,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            language: Some("en".to_string()),
            preset: DecodePreset::Balanced,
            temperature: 0.0,
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
}

/// Identifies an audio input device. Opaque string so each backend can use its
/// own naming (PipeWire node, WASAPI endpoint, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceId(pub String);
