//! The dictation state machine — the heart of the app, fully OS-agnostic.
//!
//! `on_press` starts capture; `on_release` stops, transcribes, applies Personal
//! Memory, checks for a secure field, and injects the text. The coordinator owns
//! only trait objects, so it is identical on every platform and unit-testable with
//! mock backends.

use crate::audio::AudioCapture;
use crate::error::Result;
use crate::injection::TextInjector;
use crate::memory::MemoryService;
use crate::transcription::Transcriber;
use crate::types::{DecodeOptions, InjectionResult, RecordingState, SecureFieldStatus};
use std::time::Instant;

/// Result of a completed dictation.
#[derive(Debug, Clone)]
pub struct DictationOutcome {
    pub result: InjectionResult,
    /// The final (memory-applied) transcript, when one was produced.
    pub transcript: Option<String>,
    /// How long the key was held, in milliseconds (0 when not measured).
    pub duration_ms: i64,
    /// Set when a transcript was produced but injection failed — the caller
    /// should still persist/show it and can fall back to the clipboard.
    pub inject_error: Option<String>,
}

pub struct DictationCoordinator {
    transcriber: Box<dyn Transcriber>,
    audio: Box<dyn AudioCapture>,
    injector: Box<dyn TextInjector>,
    memory: MemoryService,
    options: DecodeOptions,
    state: RecordingState,
    /// Set on press, used to measure how long the user held the key.
    recording_started: Option<Instant>,
    /// When true, also block injection if the secure status is `Unknown`
    /// (the safe-by-default opt-in for Wayland where detection is impossible).
    strict_secure: bool,
}

impl DictationCoordinator {
    pub fn new(
        transcriber: Box<dyn Transcriber>,
        audio: Box<dyn AudioCapture>,
        injector: Box<dyn TextInjector>,
        memory: MemoryService,
        options: DecodeOptions,
    ) -> Self {
        Self {
            transcriber,
            audio,
            injector,
            memory,
            options,
            state: RecordingState::Idle,
            recording_started: None,
            strict_secure: false,
        }
    }

    pub fn state(&self) -> RecordingState {
        self.state
    }

    /// Current input level (0.0..=1.0) for the live mic meter.
    pub fn level(&self) -> f32 {
        self.audio.level()
    }

    /// Opt in to blocking injection when the secure-field status is unknown.
    pub fn set_strict_secure(&mut self, strict: bool) {
        self.strict_secure = strict;
    }

    /// Hotkey pressed — begin recording. No-op while already recording.
    pub fn on_press(&mut self) -> Result<()> {
        // Recover from a previous failure so one error doesn't brick dictation
        // for the rest of the session.
        if self.state == RecordingState::Error {
            self.state = RecordingState::Idle;
        }
        if self.state != RecordingState::Idle {
            return Ok(());
        }
        self.audio.start(None).inspect_err(|_| {
            self.state = RecordingState::Error;
        })?;
        self.recording_started = Some(Instant::now());
        self.state = RecordingState::Recording;
        tracing::debug!("recording started");
        Ok(())
    }

    /// Hotkey released — stop, transcribe, apply memory, and inject.
    pub fn on_release(&mut self) -> Result<DictationOutcome> {
        if self.state != RecordingState::Recording {
            return Ok(DictationOutcome {
                result: InjectionResult::NoTranscript,
                transcript: None,
                duration_ms: 0,
                inject_error: None,
            });
        }
        self.state = RecordingState::Processing;
        let duration_ms = self
            .recording_started
            .take()
            .map(|t| t.elapsed().as_millis() as i64)
            .unwrap_or(0);

        let samples = self.audio.stop().inspect_err(|_| {
            self.state = RecordingState::Error;
        })?;

        // A too-quick tap captures no audio; don't error, just do nothing.
        if samples.is_empty() {
            self.state = RecordingState::Idle;
            return Ok(DictationOutcome {
                result: InjectionResult::NoTranscript,
                transcript: None,
                duration_ms,
                inject_error: None,
            });
        }

        let raw = self
            .transcriber
            .transcribe(&samples, &self.options)
            .inspect_err(|_| {
                self.state = RecordingState::Error;
            })?;

        let text = self.memory.apply(raw.trim());
        if text.is_empty() || is_blank_marker(&text) {
            self.state = RecordingState::Idle;
            return Ok(DictationOutcome {
                result: InjectionResult::NoTranscript,
                transcript: None,
                duration_ms,
                inject_error: None,
            });
        }

        if self.should_block_secure() {
            tracing::info!("injection blocked: focused field is (or may be) secure");
            self.state = RecordingState::Idle;
            return Ok(DictationOutcome {
                result: InjectionResult::BlockedSecureField,
                transcript: None,
                duration_ms,
                inject_error: None,
            });
        }

        // Inject — but if it fails (common on GNOME/Wayland, which lacks the
        // virtual-keyboard protocol `wtype` needs), keep the transcript so it is
        // still saved, shown, and recoverable from the clipboard rather than lost.
        self.state = RecordingState::Idle;
        match self.injector.inject(&text) {
            Ok(result) => Ok(DictationOutcome {
                result,
                transcript: Some(text),
                duration_ms,
                inject_error: None,
            }),
            Err(e) => Ok(DictationOutcome {
                result: InjectionResult::Failed,
                transcript: Some(text),
                duration_ms,
                inject_error: Some(e.to_string()),
            }),
        }
    }

    fn should_block_secure(&self) -> bool {
        match self.injector.is_secure_field_focused() {
            SecureFieldStatus::Secure => true,
            SecureFieldStatus::Unknown => self.strict_secure,
            SecureFieldStatus::NotSecure => false,
        }
    }
}

/// whisper.cpp emits placeholder tokens like `[BLANK_AUDIO]` for silence — treat
/// those as "nothing said" rather than a real transcript.
fn is_blank_marker(text: &str) -> bool {
    let t = text.trim();
    t.eq_ignore_ascii_case("[BLANK_AUDIO]")
        || t.eq_ignore_ascii_case("[ Silence ]")
        || t.eq_ignore_ascii_case("(silence)")
}
