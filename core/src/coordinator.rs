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

/// Result of a completed dictation.
#[derive(Debug, Clone)]
pub struct DictationOutcome {
    pub result: InjectionResult,
    /// The final (memory-applied) transcript, when one was produced.
    pub transcript: Option<String>,
}

pub struct DictationCoordinator {
    transcriber: Box<dyn Transcriber>,
    audio: Box<dyn AudioCapture>,
    injector: Box<dyn TextInjector>,
    memory: MemoryService,
    options: DecodeOptions,
    state: RecordingState,
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
            strict_secure: false,
        }
    }

    pub fn state(&self) -> RecordingState {
        self.state
    }

    /// Opt in to blocking injection when the secure-field status is unknown.
    pub fn set_strict_secure(&mut self, strict: bool) {
        self.strict_secure = strict;
    }

    /// Hotkey pressed — begin recording. No-op unless idle.
    pub fn on_press(&mut self) -> Result<()> {
        if self.state != RecordingState::Idle {
            return Ok(());
        }
        self.audio.start(None).inspect_err(|_| {
            self.state = RecordingState::Error;
        })?;
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
            });
        }
        self.state = RecordingState::Processing;

        let samples = self.audio.stop().inspect_err(|_| {
            self.state = RecordingState::Error;
        })?;

        let raw = self
            .transcriber
            .transcribe(&samples, &self.options)
            .inspect_err(|_| {
                self.state = RecordingState::Error;
            })?;

        let text = self.memory.apply(raw.trim());
        if text.is_empty() {
            self.state = RecordingState::Idle;
            return Ok(DictationOutcome {
                result: InjectionResult::NoTranscript,
                transcript: None,
            });
        }

        if self.should_block_secure() {
            tracing::info!("injection blocked: focused field is (or may be) secure");
            self.state = RecordingState::Idle;
            return Ok(DictationOutcome {
                result: InjectionResult::BlockedSecureField,
                transcript: None,
            });
        }

        let result = self.injector.inject(&text).inspect_err(|_| {
            self.state = RecordingState::Error;
        })?;
        self.state = RecordingState::Idle;
        Ok(DictationOutcome {
            result,
            transcript: Some(text),
        })
    }

    fn should_block_secure(&self) -> bool {
        match self.injector.is_secure_field_focused() {
            SecureFieldStatus::Secure => true,
            SecureFieldStatus::Unknown => self.strict_secure,
            SecureFieldStatus::NotSecure => false,
        }
    }
}
