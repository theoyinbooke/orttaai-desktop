//! Real transcription backend: whisper.cpp via `whisper-rs`. Enabled by the
//! `whisper` feature. The same code runs on macOS (dev host), Linux, and Windows;
//! GPU acceleration is a build-flag concern (whisper-rs features `cuda`/`vulkan`/
//! `metal`/`hipblas`), not a code change.

use super::Transcriber;
use crate::error::{CoreError, Result};
use crate::types::{DecodeOptions, DecodePreset};
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperTranscriber {
    ctx: Option<WhisperContext>,
    model_id: Option<String>,
    models_dir: PathBuf,
    n_threads: i32,
}

impl WhisperTranscriber {
    /// Create an unloaded transcriber that resolves model ids against `models_dir`
    /// (looking for `<models_dir>/<model_id>.bin`).
    pub fn new(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            ctx: None,
            model_id: None,
            models_dir: models_dir.into(),
            n_threads: default_threads(),
        }
    }

    /// Load a model directly from a `.bin`/`.gguf` file path.
    pub fn from_path(model_path: &Path) -> Result<Self> {
        let models_dir = model_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut t = Self::new(models_dir);
        let id = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_string();
        t.load_ctx(model_path, id)?;
        Ok(t)
    }

    fn load_ctx(&mut self, path: &Path, id: String) -> Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| CoreError::Transcription("non-UTF-8 model path".to_string()))?;
        // Explicit params so GPU builds actually use the device. `use_gpu` only has
        // an effect when a GPU backend was compiled in (the `vulkan`/`cuda`
        // features); on a CPU build it's a no-op. flash_attn is a CUDA-kernel win
        // (it disables DTW token timestamps, which dictation doesn't use).
        let mut cparams = WhisperContextParameters::default();
        cparams.use_gpu(cfg!(any(feature = "vulkan", feature = "cuda")));
        cparams.gpu_device(0);
        cparams.flash_attn(cfg!(feature = "cuda"));
        let ctx = WhisperContext::new_with_params(path_str, cparams)
            .map_err(|e| CoreError::Transcription(format!("failed to load model {id}: {e}")))?;
        self.ctx = Some(ctx);
        self.model_id = Some(id);
        Ok(())
    }
}

impl Transcriber for WhisperTranscriber {
    fn load_model(&mut self, model_id: &str) -> Result<()> {
        let path = self.models_dir.join(format!("{model_id}.bin"));
        self.load_ctx(&path, model_id.to_string())
    }

    fn loaded_model(&self) -> Option<&str> {
        self.model_id.as_deref()
    }

    fn transcribe(&self, samples: &[f32], opts: &DecodeOptions) -> Result<String> {
        let ctx = self.ctx.as_ref().ok_or(CoreError::ModelNotLoaded)?;
        let mut state = ctx
            .create_state()
            .map_err(|e| CoreError::Transcription(format!("create state: {e}")))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // `Some("auto")` means auto-detect, which whisper-rs expresses as `None`.
        let language = match opts.language.as_deref() {
            Some("auto") | None => None,
            other => other,
        };
        params.set_language(language);

        let threads = if opts.n_threads > 0 {
            opts.n_threads
        } else {
            self.n_threads
        };
        params.set_n_threads(threads.max(1));

        // Push-to-talk: each clip is one short, independent utterance, so drop the
        // prior-transcript prompt, force a single segment, and skip timestamp/DTW
        // work — all pure latency wins with no quality cost for plain-text dictation.
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        params.set_suppress_nst(true);
        params.set_temperature(opts.temperature);
        // whisper.cpp's default temperature fallback re-decodes a "hard" clip up to
        // ~6× (0.0→0.2→…), silently multiplying latency. Disable it for the fast
        // presets; only Accuracy keeps the fallback for genuinely tough audio.
        let temp_inc = if matches!(opts.preset, DecodePreset::Accuracy) {
            0.2
        } else {
            0.0
        };
        params.set_temperature_inc(temp_inc);

        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, samples)
            .map_err(|e| CoreError::Transcription(format!("decode: {e}")))?;

        let segments = state.full_n_segments();
        let mut text = String::new();
        for i in 0..segments {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let chunk = segment
                .to_str()
                .map_err(|e| CoreError::Transcription(format!("segment text: {e}")))?;
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(chunk.trim());
        }
        Ok(text.trim().to_string())
    }
}

/// Default decode threads. whisper.cpp matmul is compute-bound, so on a hybrid
/// CPU (P + E cores + hyperthreads) piling on every logical thread hurts — the
/// slowest E-core/HT straggler gates each layer. Cap at ~the performance-core
/// count; users can override via `DecodeOptions::n_threads`.
fn default_threads() -> i32 {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    logical.min(8) as i32
}
