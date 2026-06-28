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

        let cleaned = strip_nonspeech_markers(raw.trim());
        let text = self.memory.apply(&cleaned);
        if text.trim().is_empty() {
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

/// whisper.cpp annotates non-speech audio with bracketed tags like
/// `[BLANK_AUDIO]`, `[Music]`, or `(dramatic music)` — and emits them *inline*
/// within an otherwise-real transcript, not only on their own. Strip any
/// bracketed/parenthesized span that names a non-speech sound, leaving genuine
/// dictated parentheticals (e.g. "call me (maybe)") intact, then collapse the
/// whitespace the removed spans leave behind.
fn strip_nonspeech_markers(text: &str) -> String {
    const NONSPEECH: &[&str] = &[
        "blank_audio",
        "music",
        "silence",
        "applause",
        "laughter",
        "inaudible",
        "noise",
        "no audio",
        "no speech",
        "sighs",
        "coughs",
        "beep",
    ];
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let closer = match c {
            '[' => Some(']'),
            '(' => Some(')'),
            _ => None,
        };
        if let Some(close_ch) = closer {
            if let Some(rel) = text[i + 1..].find(close_ch) {
                let inner = text[i + 1..i + 1 + rel].to_ascii_lowercase();
                if NONSPEECH.iter().any(|k| inner.contains(k)) {
                    i += 1 + rel + 1; // skip the whole span, including the closer
                    continue;
                }
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::strip_nonspeech_markers;

    #[test]
    fn strips_inline_and_standalone_nonspeech_tags() {
        assert_eq!(
            strip_nonspeech_markers("I hope it is working. [BLANK_AUDIO]"),
            "I hope it is working."
        );
        assert_eq!(
            strip_nonspeech_markers("[Music] This is working. [Music]"),
            "This is working."
        );
        assert_eq!(strip_nonspeech_markers("(dramatic music)"), "");
        assert_eq!(strip_nonspeech_markers("[BLANK_AUDIO]"), "");
    }

    #[test]
    fn keeps_genuine_parentheticals() {
        assert_eq!(strip_nonspeech_markers("call me (maybe)"), "call me (maybe)");
        assert_eq!(
            strip_nonspeech_markers("the deadline (urgent) is today"),
            "the deadline (urgent) is today"
        );
    }
}
