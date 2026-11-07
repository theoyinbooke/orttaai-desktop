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
            // With Word Boost's model (WORD_BOOST_MB), which comes with it.
            approx_size_mb: 763,
            multilingual: false,
            archive_bytes: 482_440_461,
        },
        ExpressModelSpec {
            id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
            name: "Express · Multilingual (Parakeet v3)",
            description: "Fast on CPU. 25 European languages, auto-detected.",
            approx_size_mb: 772,
            multilingual: true,
            archive_bytes: 487_170_055,
        },
    ]
}

#[cfg(test)]
mod catalog_tests {
    #[test]
    fn multilingual_flags_match_the_settings_list() {
        for spec in super::catalog() {
            assert_eq!(
                spec.multilingual,
                crate::settings::MULTILINGUAL_EXPRESS_MODELS.contains(&spec.id),
                "{}",
                spec.id
            );
        }
    }
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

/// Word Boost's model (Mac: Parakeet CTC 110M): the CTC branch of
/// `nvidia/parakeet-tdt_ctc-110m`, downloaded with every Express model.
pub const WORD_BOOST_ID: &str = "sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8";
const WORD_BOOST_ARCHIVE_BYTES: u64 = 104_337_827;
/// The model's SentencePiece tokenizer, to spell dictionary words in its tokens
/// (the same file the Mac's Word Boost loads).
const WORD_BOOST_TOKENIZER_URL: &str =
    "https://huggingface.co/FluidInference/parakeet-ctc-110m-coreml/resolve/main/tokenizer.json";
/// On disk, for size estimates.
pub const WORD_BOOST_MB: u32 = 132;

pub fn word_boost_dir() -> Result<PathBuf> {
    Ok(express_dir()?.join(WORD_BOOST_ID))
}

fn word_boost_files(dir: &Path) -> [PathBuf; 3] {
    [
        dir.join("model.int8.onnx"),
        dir.join("tokens.txt"),
        dir.join("tokenizer.json"),
    ]
}

pub fn word_boost_installed() -> bool {
    word_boost_dir()
        .map(|d| word_boost_files(&d).iter().all(|f| f.is_file()))
        .unwrap_or(false)
}

/// Download Word Boost's model and tokenizer, reporting progress in `0.0..=1.0`.
pub fn download_word_boost(
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let root = express_dir()?;
    let final_dir = root.join(WORD_BOOST_ID);
    if word_boost_installed() {
        return Ok(final_dir);
    }
    let staging = root.join(format!("{WORD_BOOST_ID}.part"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        fetch_archive(
            &format!("{RELEASE_BASE}/{WORD_BOOST_ID}.tar.bz2"),
            WORD_BOOST_ARCHIVE_BYTES,
            &["model.int8.onnx", "tokens.txt"],
            &staging,
            cancel,
            &mut |f| on_progress(f * 0.97),
        )?;
        let resp = ureq::get(WORD_BOOST_TOKENIZER_URL)
            .call()
            .map_err(|e| CoreError::Transcription(format!("download failed: {e}")))?;
        let mut file = std::fs::File::create(staging.join("tokenizer.json"))?;
        std::io::copy(&mut resp.into_body().into_reader(), &mut file)?;
        if !word_boost_files(&staging).iter().all(|f| f.is_file()) {
            return Err(CoreError::Transcription(
                "downloaded Word Boost files are incomplete".into(),
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

/// Stream a `.tar.bz2` and keep the top-level `wanted` files in `dir`.
fn fetch_archive(
    url: &str,
    size_hint: u64,
    wanted: &[&str],
    dir: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f32),
) -> Result<()> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| CoreError::Transcription(format!("download failed: {e}")))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(size_hint);
    let reader = ProgressReader {
        inner: resp.into_body().into_reader(),
        read: 0,
        total,
        cancel,
        on_progress,
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
        if wanted.contains(&name.as_str()) && path.components().count() <= 2 {
            entry.unpack(dir.join(&name))?;
        } else {
            // Drain skipped entries (test wavs etc.) to keep the stream moving.
            std::io::copy(&mut entry, &mut std::io::sink())?;
        }
    }
    Ok(())
}

/// Download + unpack an Express model, then Word Boost's model when it isn't
/// there yet (the Mac's Express package holds both), reporting progress in
/// `0.0..=1.0`. Set `cancel` to abort; a cancelled or failed download leaves
/// nothing behind.
pub fn download(
    id: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let spec = spec(id)
        .ok_or_else(|| CoreError::Transcription(format!("unknown Express model '{id}'")))?;
    let boost_share = if word_boost_installed() { 0.0 } else { 0.15 };
    let root = express_dir()?;
    let final_dir = root.join(spec.id);
    if !is_installed(spec.id) {
        let staging = root.join(format!("{}.part", spec.id));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let result = fetch_archive(
            &format!("{RELEASE_BASE}/{}.tar.bz2", spec.id),
            spec.archive_bytes,
            &[
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ],
            &staging,
            cancel,
            &mut |f| on_progress(f * (1.0 - boost_share)),
        )
        .and_then(|()| {
            if model_files(&staging).iter().all(|f| f.is_file()) {
                Ok(())
            } else {
                Err(CoreError::Transcription(
                    "downloaded archive is missing model files".into(),
                ))
            }
        });
        match result {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&final_dir);
                std::fs::rename(&staging, &final_dir)?;
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        }
    }
    if boost_share > 0.0 {
        download_word_boost(cancel, |f| on_progress(1.0 - boost_share + f * boost_share))?;
    }
    on_progress(1.0);
    Ok(final_dir)
}

struct ProgressReader<'a, R: Read> {
    inner: R,
    read: u64,
    total: u64,
    cancel: &'a AtomicBool,
    on_progress: &'a mut dyn FnMut(f32),
}

impl<R: Read> Read for ProgressReader<'_, R> {
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
    /// Word Boost, loaded when its model is on disk and there are words to boost.
    word_boost: Mutex<Option<std::sync::Arc<crate::wordboost::WordBoost>>>,
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
            word_boost: Mutex::new(None),
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

    fn live_mode(&self) -> crate::live::LiveMode {
        crate::live::LiveMode::Segments
    }

    fn transcribe(&self, samples: &[f32], _opts: &DecodeOptions) -> Result<String> {
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
        let boost = self
            .word_boost
            .lock()
            .unwrap()
            .clone()
            .filter(|w| w.active());
        // Word Boost scores the same audio while Parakeet decodes it, so both
        // share one clock and the boost costs little extra time.
        let (result, log_probs) = std::thread::scope(|scope| {
            let scorer = boost.as_ref().map(|w| {
                let audio = &padded;
                scope.spawn(move || w.log_probs(audio))
            });
            let result = {
                let guard = self.recognizer.lock().unwrap();
                guard
                    .as_ref()
                    .ok_or(CoreError::ModelNotLoaded)
                    .map(|recognizer| {
                        let stream = recognizer.create_stream();
                        stream.accept_waveform(TARGET_SAMPLE_RATE as i32, &padded);
                        recognizer.decode(&stream);
                        stream.get_result()
                    })
            };
            let log_probs = scorer.and_then(|h| match h.join() {
                Ok(Ok(lp)) => Some(lp),
                Ok(Err(e)) => {
                    tracing::warn!("Word Boost skipped: {e}");
                    None
                }
                Err(_) => None,
            });
            (result, log_probs)
        });
        let Some(result) = result? else {
            return Ok(String::new());
        };
        let mut text = result.text.clone();
        if let (Some(w), Some(log_probs), Some(starts)) =
            (boost.as_ref(), log_probs, result.timestamps.as_ref())
        {
            let timings = crate::wordboost::rescore::TokenTiming::from_starts_and_durations(
                &result.tokens,
                starts,
                result.durations.as_deref().unwrap_or(&[]),
            );
            if let Some(out) = w.boost(&text, &timings, &log_probs) {
                let applied = out.proposals.iter().filter(|p| p.applied).count();
                for p in &out.proposals {
                    if std::env::var_os("ORTTAAI_WORD_BOOST_DEBUG").is_some() {
                        eprintln!("word boost proposal: {p:?}");
                    }
                    tracing::info!(
                        "Word Boost {}: -> {} [margin {:.2}, spelling {:.2}]",
                        if p.applied { "applied" } else { "rejected" },
                        p.term,
                        p.margin,
                        p.similarity
                    );
                }
                tracing::info!(
                    "Word Boost: {applied} of {} proposals applied",
                    out.proposals.len()
                );
                text = out.text;
            }
        }
        // A finished recording's scores are used once.
        if let Some(w) = boost.as_ref() {
            w.clear();
        }
        Ok(normalize_spacing(&text))
    }

    fn set_word_boost_terms(&self, terms: &[crate::text::WordBoostTerm]) {
        let mut slot = self.word_boost.lock().unwrap();
        if slot.is_none() && !terms.is_empty() {
            if let Some(dir) = std::env::var_os("ORTTAAI_WORD_BOOST_DIR") {
                // Tests point at a model folder outside the data dir.
                if let Ok(w) = crate::wordboost::WordBoost::load(Path::new(&dir), self.threads) {
                    *slot = Some(std::sync::Arc::new(w));
                }
            }
        }
        if terms.is_empty() {
            if let Some(w) = slot.as_ref() {
                w.set_terms(&[]);
            }
            return;
        }
        if slot.is_none() && word_boost_installed() {
            match word_boost_dir().and_then(|d| crate::wordboost::WordBoost::load(&d, self.threads))
            {
                Ok(w) => *slot = Some(std::sync::Arc::new(w)),
                Err(e) => tracing::warn!("Word Boost model failed to load: {e}"),
            }
        }
        if let Some(w) = slot.as_ref() {
            let n = w.set_terms(terms);
            tracing::info!("Word Boost listening for {n} dictionary words");
        }
    }

    fn word_boost_active(&self) -> bool {
        self.word_boost
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|w| w.active())
    }

    fn prepare_word_boost(&self, prefix: &[f32]) {
        let boost = self.word_boost.lock().unwrap().clone();
        if let Some(w) = boost.filter(|w| w.active()) {
            w.prepare(prefix);
        }
    }

    fn discard_prepared_word_boost(&self) {
        if let Some(w) = self.word_boost.lock().unwrap().as_ref() {
            w.clear();
        }
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

    /// Word Boost end to end: `ORTTAAI_WORD_BOOST_DIR` holds model.int8.onnx and
    /// tokenizer.json, and `ORTTAAI_WORD_BOOST_WAV` is the clip made on a Mac with
    /// `say -v Samantha -o word-boost.wav --data-format=LEI16@16000 "Temitope
    /// moved the orders from Supabase to Postgres this morning."`. Parakeet alone
    /// hears "Tami Tope ... Superbase to Postgers". The other terms must stay out.
    #[test]
    #[ignore]
    fn word_boost_spells_a_dictionary_name() {
        let dir = std::env::var("ORTTAAI_EXPRESS_MODEL_DIR").expect("ORTTAAI_EXPRESS_MODEL_DIR");
        std::env::var("ORTTAAI_WORD_BOOST_DIR").expect("ORTTAAI_WORD_BOOST_DIR");
        // Its own clip (see above), so suites that set ORTTAAI_TEST_WAV for
        // the other real-audio tests can run this one too.
        let wav = std::env::var("ORTTAAI_WORD_BOOST_WAV").expect("ORTTAAI_WORD_BOOST_WAV");
        let mut reader = hound::WavReader::open(wav).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect();
        let t = ExpressTranscriber::from_dir(Path::new(&dir), 4).unwrap();
        let plain = t.transcribe(&samples, &DecodeOptions::default()).unwrap();
        let term = |text: &str| crate::text::WordBoostTerm {
            text: text.into(),
            aliases: Vec::new(),
        };
        let names = ["Temitope", "Supabase", "Postgres"];
        let distractors = ["Kubernetes", "Orttaai", "Xcode"];
        let terms: Vec<_> = names.iter().chain(&distractors).map(|n| term(n)).collect();
        t.set_word_boost_terms(&terms);
        assert!(t.word_boost_active());
        let started = std::time::Instant::now();
        let boosted = t.transcribe(&samples, &DecodeOptions::default()).unwrap();
        eprintln!(
            "plain:   {plain}\nboosted: {boosted}\n({:?})",
            started.elapsed()
        );
        for name in names {
            assert!(!plain.contains(name), "{plain}");
            assert!(boosted.contains(name), "{boosted}");
        }
        for name in distractors {
            assert!(!boosted.contains(name), "{boosted}");
        }
        assert!(boosted.contains("moved the orders from"), "{boosted}");
    }
}
