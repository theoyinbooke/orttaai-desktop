//! Real transcription backend: whisper.cpp via `whisper-rs`. Enabled by the
//! `whisper` feature. The same code runs on macOS (dev host), Linux, and Windows;
//! GPU acceleration is a build-flag concern (whisper-rs features `cuda`/`vulkan`),
//! not a code change.

use super::Transcriber;
use crate::error::{CoreError, Result};
use crate::types::{DecodeOptions, DecodePreset};
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Concrete whisper.cpp decode parameters for one transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct WhisperParams {
    /// Candidates per sampled fallback decode (unused at temperature 0).
    pub best_of: i32,
    pub temperature: f32,
    /// whisper.cpp retries at `temperature + k·inc` up to 1.0 when a decode
    /// fails its quality gates; `0` disables the fallback.
    pub temperature_inc: f32,
    pub entropy_thold: f32,
    pub logprob_thold: f32,
    pub no_speech_thold: f32,
}

/// Map the preset / Advanced Decoding / Low Latency onto whisper.cpp.
///
/// Preset baselines follow the Mac (`resolvedDecodingOptions`) with one
/// desktop rule from `docs/performance.md`: Fast and Balanced never take the
/// temperature fallback (each one re-decodes the whole clip on CPU, silently
/// multiplying latency); Accuracy keeps the Mac's five fallbacks.
pub fn whisper_params(opts: &DecodeOptions) -> WhisperParams {
    let mut p = match opts.preset {
        DecodePreset::Fast => WhisperParams {
            best_of: 3,
            temperature: 0.0,
            temperature_inc: 0.0,
            entropy_thold: 2.4,
            logprob_thold: -1.0,
            no_speech_thold: 0.65,
        },
        DecodePreset::Balanced => WhisperParams {
            best_of: 5,
            temperature: opts.temperature.clamp(0.0, 1.0),
            temperature_inc: 0.0,
            entropy_thold: 2.4,
            logprob_thold: -1.0,
            no_speech_thold: 0.6,
        },
        DecodePreset::Accuracy => WhisperParams {
            best_of: 5,
            temperature: 0.0,
            temperature_inc: 0.2, // five fallbacks: 0.2, 0.4 … 1.0
            entropy_thold: 2.4,
            logprob_thold: -1.0,
            no_speech_thold: 0.6,
        },
    };
    if let Some(t) = &opts.tuning {
        p.temperature = t.temperature;
        p.best_of = t.best_of.max(1);
        // whisper.cpp tries t0+inc, t0+2inc … ≤ 1.0, so (1 - t0)/N gives N.
        p.temperature_inc = if t.fallback_count > 0 && t.temperature < 1.0 {
            (1.0 - t.temperature) / t.fallback_count as f32
        } else {
            0.0
        };
        p.entropy_thold = t.compression_ratio_threshold;
        p.logprob_thold = t.logprob_threshold;
        p.no_speech_thold = t.no_speech_threshold;
    }
    if opts.low_latency {
        p.temperature_inc = 0.0;
        p.best_of = 1;
    }
    p
}

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
        whisper_rs::install_logging_hooks();
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
        if !path.exists() {
            return Err(CoreError::Transcription(format!(
                "model '{id}' is not downloaded"
            )));
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| CoreError::Transcription("non-UTF-8 model path".to_string()))?;
        // Explicit params so GPU builds actually use the device. `use_gpu` only
        // has an effect when a GPU backend was compiled in; flash_attn is a
        // CUDA-kernel win (it disables DTW token timestamps, unused here).
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

        let wp = whisper_params(opts);
        let mut params = FullParams::new(SamplingStrategy::Greedy {
            best_of: wp.best_of,
        });
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

        // Push-to-talk: each clip is one independent utterance, so drop the
        // prior-transcript context, force a single segment for short clips, and
        // skip timestamp work — pure latency wins for plain-text dictation.
        // Long hands-free recordings (> one 30 s window) need multiple segments.
        params.set_no_context(true);
        params.set_single_segment(samples.len() <= 30 * 16_000);
        params.set_no_timestamps(true);
        params.set_suppress_nst(true);
        params.set_temperature(wp.temperature);
        params.set_temperature_inc(wp.temperature_inc);
        params.set_entropy_thold(wp.entropy_thold);
        params.set_logprob_thold(wp.logprob_thold);
        params.set_no_speech_thold(wp.no_speech_thold);
        if let Some(prompt) = opts.initial_prompt.as_deref() {
            let clean: String = prompt.chars().filter(|c| *c != '\0').collect();
            if !clean.trim().is_empty() {
                params.set_initial_prompt(&clean);
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DecodeTuning;

    #[test]
    fn presets_map_to_mac_baselines() {
        let mut o = DecodeOptions {
            preset: DecodePreset::Fast,
            ..Default::default()
        };
        let f = whisper_params(&o);
        assert_eq!(f.temperature_inc, 0.0);
        assert_eq!(f.no_speech_thold, 0.65);
        o.preset = DecodePreset::Accuracy;
        let a = whisper_params(&o);
        assert!((a.temperature_inc - 0.2).abs() < 1e-6);
        o.low_latency = true;
        assert_eq!(whisper_params(&o).temperature_inc, 0.0);
        assert_eq!(whisper_params(&o).best_of, 1);
    }

    #[test]
    fn advanced_decoding_overrides_preset() {
        let o = DecodeOptions {
            tuning: Some(DecodeTuning {
                temperature: 0.2,
                best_of: 4,
                fallback_count: 4,
                compression_ratio_threshold: 2.0,
                logprob_threshold: -0.5,
                no_speech_threshold: 0.4,
            }),
            ..Default::default()
        };
        let p = whisper_params(&o);
        assert_eq!(p.best_of, 4);
        assert!((p.temperature_inc - 0.2).abs() < 1e-6);
        assert_eq!(p.entropy_thold, 2.0);
        assert_eq!(p.no_speech_thold, 0.4);
    }
}
