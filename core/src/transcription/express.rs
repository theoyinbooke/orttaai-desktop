//! "Express" engine — NVIDIA Parakeet TDT 0.6B (int8) through sherpa-onnx.
//!
//! The macOS app's Express engine runs Parakeet on the Neural Engine via CoreML.
//! On Linux/Windows the same model family runs on CPU through ONNX Runtime
//! (statically linked by `sherpa-onnx`), typically 10–20× faster than real time
//! on a modern CPU — the fastest option on machines without an NVIDIA GPU.
//!
//! Models are downloaded as `.tar.bz2` archives from the sherpa-onnx releases and
//! unpacked to `<data-dir>/models/express/<id>/`.

use crate::error::{CoreError, Result};
use crate::transcription::Transcriber;
use crate::types::{DecodeOptions, TARGET_SAMPLE_RATE};
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const RELEASE_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// A downloadable Express model.
#[derive(Debug, Clone, Serialize)]
pub struct ExpressModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub approx_size_mb: u32,
    pub multilingual: bool,
    /// Download size in bytes (used to verify the archive).
    pub archive_bytes: u64,
}

/// A catalog entry plus whether it is installed.
#[derive(Debug, Clone, Serialize)]
pub struct ExpressModelInfo {
    #[serde(flatten)]
    pub spec: ExpressModelSpec,
    pub downloaded: bool,
    pub path: String,
}

pub fn catalog() -> Vec<ExpressModelSpec> {
    vec![
        ExpressModelSpec {
            id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
            name: "Express · English (Parakeet v2)",
            description: "Fastest on CPU. English only, with punctuation and casing.",
            approx_size_mb: 631,
            multilingual: false,
            archive_bytes: 482_440_461,
        },
        ExpressModelSpec {
            id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
            name: "Express · Multilingual (Parakeet v3)",
            description: "Fast on CPU. 25 European languages, auto-detected.",
            approx_size_mb: 640,
            multilingual: true,
            archive_bytes: 487_170_055,
        },
    ]
}

pub fn spec(id: &str) -> Option<ExpressModelSpec> {
    catalog().into_iter().find(|s| s.id == id)
}

/// `<data-dir>/models/express`, created on demand.
pub fn express_dir() -> Result<PathBuf> {
    let dir = crate::store::Store::data_dir()?
        .join("models")
        .join("express");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn model_dir(id: &str) -> Result<PathBuf> {
    Ok(express_dir()?.join(id))
}

/// The four files a Parakeet transducer needs.
fn model_files(dir: &Path) -> [PathBuf; 4] {
    [
        dir.join("encoder.int8.onnx"),
        dir.join("decoder.int8.onnx"),
        dir.join("joiner.int8.onnx"),
        dir.join("tokens.txt"),
    ]
}

pub fn is_installed(id: &str) -> bool {
    model_dir(id)
        .map(|d| model_files(&d).iter().all(|f| f.is_file()))
        .unwrap_or(false)
}

pub fn list() -> Result<Vec<ExpressModelInfo>> {
    catalog()
        .into_iter()
        .map(|spec| {
            let dir = model_dir(spec.id)?;
            Ok(ExpressModelInfo {
                downloaded: is_installed(spec.id),
                path: dir.to_string_lossy().into_owned(),
                spec,
            })
        })
        .collect()
}

pub fn delete(id: &str) -> Result<()> {
    let dir = model_dir(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Download + unpack an Express model, reporting progress in `0.0..=1.0`.
/// Set `cancel` to abort; a cancelled or failed download leaves nothing behind.
pub fn download(
    id: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let spec = spec(id)
        .ok_or_else(|| CoreError::Transcription(format!("unknown Express model '{id}'")))?;
    let root = express_dir()?;
    let final_dir = root.join(spec.id);
    if is_installed(spec.id) {
        return Ok(final_dir);
    }
    let staging = root.join(format!("{}.part", spec.id));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> Result<()> {
        let url = format!("{RELEASE_BASE}/{}.tar.bz2", spec.id);
        let resp = ureq::get(&url)
            .call()
            .map_err(|e| CoreError::Transcription(format!("download failed: {e}")))?;
        let total = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(spec.archive_bytes);
        let reader = ProgressReader {
            inner: resp.into_body().into_reader(),
            read: 0,
            total,
            cancel,
            on_progress: &mut on_progress,
        };
        let decoder = bzip2::read::BzDecoder::new(reader);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            // Archive layout: `<id>/<file>`; keep only the model files we need.
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            let wanted = matches!(
                name.as_str(),
                "encoder.int8.onnx" | "decoder.int8.onnx" | "joiner.int8.onnx" | "tokens.txt"
            );
            if wanted && path.components().count() <= 2 {
                entry.unpack(staging.join(&name))?;
            } else {
                // Drain skipped entries (test wavs etc.) to keep the stream moving.
                std::io::copy(&mut entry, &mut std::io::sink())?;
            }
        }
        if !model_files(&staging).iter().all(|f| f.is_file()) {
            return Err(CoreError::Transcription(
                "downloaded archive is missing model files".into(),
            ));
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&final_dir);
            std::fs::rename(&staging, &final_dir)?;
            on_progress(1.0);
            Ok(final_dir)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

struct ProgressReader<'a, R: Read, F: FnMut(f32)> {
    inner: R,
    read: u64,
    total: u64,
    cancel: &'a AtomicBool,
    on_progress: &'a mut F,
}

impl<R: Read, F: FnMut(f32)> Read for ProgressReader<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "download cancelled",
            ));
        }
        let n = self.inner.read(buf)?;
        let before = self.read * 200 / self.total.max(1);
        self.read += n as u64;
        let after = self.read * 200 / self.total.max(1);
        if after != before {
            (self.on_progress)((self.read as f32 / self.total.max(1) as f32).min(0.99));
        }
        Ok(n)
    }
}

/// Parakeet via sherpa-onnx, implementing [`Transcriber`].
pub struct ExpressTranscriber {
    recognizer: Mutex<Option<OfflineRecognizer>>,
    model: Option<String>,
    threads: i32,
}

// SAFETY: the sherpa-onnx recognizer is only touched behind the Mutex, one
// decode at a time; the underlying C object has no thread affinity.
unsafe impl Send for ExpressTranscriber {}
unsafe impl Sync for ExpressTranscriber {}

impl ExpressTranscriber {
    pub fn new() -> Self {
        Self {
            recognizer: Mutex::new(None),
            model: None,
            threads: default_threads(),
        }
    }

    /// Load from an unpacked model directory (encoder/decoder/joiner/tokens).
    pub fn from_dir(dir: &Path, threads: i32) -> Result<Self> {
        let mut t = Self::new();
        if threads > 0 {
            t.threads = threads;
        }
        t.load_dir(dir)?;
        t.model = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        Ok(t)
    }

    fn load_dir(&mut self, dir: &Path) -> Result<()> {
        let [encoder, decoder, joiner, tokens] = model_files(dir);
        for f in [&encoder, &decoder, &joiner, &tokens] {
            if !f.is_file() {
                return Err(CoreError::Transcription(format!(
                    "Express model file missing: {}",
                    f.display()
                )));
            }
        }
        let s = |p: &PathBuf| Some(p.to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: s(&encoder),
            decoder: s(&decoder),
            joiner: s(&joiner),
        };
        config.model_config.tokens = s(&tokens);
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = self.threads;
        config.decoding_method = Some("greedy_search".into());
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| CoreError::Transcription("could not load the Express model".into()))?;
        *self.recognizer.lock().unwrap() = Some(recognizer);
        Ok(())
    }
}

impl Default for ExpressTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcriber for ExpressTranscriber {
    fn load_model(&mut self, model_id: &str) -> Result<()> {
        let dir = model_dir(model_id)?;
        self.load_dir(&dir)?;
        self.model = Some(model_id.to_string());
        Ok(())
    }

    fn loaded_model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn transcribe(&self, samples: &[f32], _opts: &DecodeOptions) -> Result<String> {
        let guard = self.recognizer.lock().unwrap();
        let recognizer = guard.as_ref().ok_or(CoreError::ModelNotLoaded)?;
        if samples.is_empty() {
            return Ok(String::new());
        }
        // Parakeet clips the last word when audio ends abruptly (the Mac's
        // ExpressAudioConditioner pads too): add 0.4 s of trailing silence.
        let mut padded = Vec::with_capacity(samples.len() + 6400);
        padded.extend_from_slice(samples);
        padded.extend(std::iter::repeat_n(
            0.0f32,
            (TARGET_SAMPLE_RATE as usize * 2) / 5,
        ));
        let stream = recognizer.create_stream();
        stream.accept_waveform(TARGET_SAMPLE_RATE as i32, &padded);
        recognizer.decode(&stream);
        let text = stream.get_result().map(|r| r.text).unwrap_or_default();
        Ok(normalize_spacing(&text))
    }
}

/// Parakeet sometimes glues a currency/number onto the previous word
/// ("Sarah$2,450"); re-insert the missing space and collapse whitespace.
pub fn normalize_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev: Option<char> = None;
    for c in text.chars() {
        if let Some(p) = prev {
            let glued_currency = p.is_alphabetic() && matches!(c, '$' | '€' | '£');
            if glued_currency {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn default_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_lookup_works() {
        let c = catalog();
        assert_eq!(c.len(), 2);
        assert!(spec("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8").is_some());
        assert!(spec("nope").is_none());
    }

    #[test]
    fn fixes_glued_currency() {
        assert_eq!(
            normalize_spacing("I owe Sarah$2,450.  Thanks"),
            "I owe Sarah $2,450. Thanks"
        );
        assert_eq!(normalize_spacing("costs $5"), "costs $5");
    }

    #[test]
    fn unloaded_transcriber_errors() {
        let t = ExpressTranscriber::new();
        assert!(matches!(
            t.transcribe(&[0.0; 10], &DecodeOptions::default()),
            Err(CoreError::ModelNotLoaded)
        ));
    }

    /// Real decode, run with:
    /// `ORTTAAI_EXPRESS_MODEL_DIR=… ORTTAAI_TEST_WAV=… cargo test -p orttaai-core --features express -- --ignored express_real`
    #[test]
    #[ignore]
    fn express_real_decode() {
        let dir = std::env::var("ORTTAAI_EXPRESS_MODEL_DIR").expect("ORTTAAI_EXPRESS_MODEL_DIR");
        let wav = std::env::var("ORTTAAI_TEST_WAV").expect("ORTTAAI_TEST_WAV");
        let mut reader = hound::WavReader::open(wav).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect();
        let t = ExpressTranscriber::from_dir(Path::new(&dir), 4).unwrap();
        let text = t.transcribe(&samples, &DecodeOptions::default()).unwrap();
        assert!(text.to_lowercase().contains("quarterly report"), "{text}");
    }
}
