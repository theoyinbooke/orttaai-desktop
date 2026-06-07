//! Speech-to-text. The real backend (whisper.cpp via `whisper-rs`) lands in
//! Phase 1 behind the `whisper` feature; everything above this trait is portable.

use crate::error::Result;
use crate::types::DecodeOptions;

pub trait Transcriber: Send + Sync {
    /// Load a model by id (e.g. `"ggml-base.en"`).
    fn load_model(&mut self, model_id: &str) -> Result<()>;

    /// The currently loaded model id, if any.
    fn loaded_model(&self) -> Option<&str>;

    /// Transcribe 16 kHz mono `f32` samples to text.
    fn transcribe(&self, samples: &[f32], opts: &DecodeOptions) -> Result<String>;
}

/// Deterministic transcriber for tests and the `demo` CLI on any OS.
pub struct MockTranscriber {
    canned: String,
    model: Option<String>,
}

impl MockTranscriber {
    pub fn new(canned: impl Into<String>) -> Self {
        Self {
            canned: canned.into(),
            model: Some("mock".to_string()),
        }
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

    fn transcribe(&self, _samples: &[f32], _opts: &DecodeOptions) -> Result<String> {
        Ok(self.canned.clone())
    }
}

#[cfg(feature = "whisper")]
mod whisper;
#[cfg(feature = "whisper")]
pub use whisper::WhisperTranscriber;
