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
use crate::models::download_resumable;
use crate::transcription::Transcriber;
use crate::types::{DecodeOptions, TARGET_SAMPLE_RATE};
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
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

/// `express` in the models folder (Settings › Model Storage, like the Whisper
/// models), created on demand.
pub fn express_dir() -> Result<PathBuf> {
    let dir = crate::models::models_dir()?.join("express");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Where Express kept its files before it followed the models folder.
fn legacy_express_dir() -> Option<PathBuf> {
    Some(crate::models::default_models_dir().ok()?.join("express"))
}

/// `name` in the Express folder, or in the old default folder when it was
/// installed there before a custom models folder was chosen.
fn express_path(name: &str, installed: impl Fn(&Path) -> bool) -> Result<PathBuf> {
    let dir = express_dir()?.join(name);
    if !installed(&dir) {
        if let Some(old) = legacy_express_dir().map(|d| d.join(name)) {
            if installed(&old) {
                return Ok(old);
            }
        }
    }
    Ok(dir)
}

pub fn model_dir(id: &str) -> Result<PathBuf> {
    express_path(id, |d| model_files(d).iter().all(|f| f.is_file()))
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
    express_path(WORD_BOOST_ID, |d| {
        word_boost_files(d).iter().all(|f| f.is_file())
    })
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
            &root.join(format!("{WORD_BOOST_ID}.tar.bz2.part")),
            cancel,
            &mut |f| on_progress(f * 0.97),
        )?;
        let tokenizer = staging.join("tokenizer.json.part");
        download_resumable(
            WORD_BOOST_TOKENIZER_URL,
            &tokenizer,
            360_106,
            cancel,
            &mut |_| {},
        )?;
        std::fs::rename(&tokenizer, staging.join("tokenizer.json"))?;
        if !word_boost_files(&staging).iter().all(|f| f.is_file()) {
            return Err(CoreError::Download(
                "The Word Boost download is incomplete. Try again.".into(),
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

/// Download a `.tar.bz2` to `part` (resuming whatever is already there), then
/// keep its top-level `wanted` files in `dir`. Big model archives are fetched
/// to a file rather than unpacked from the network, so a dropped connection
/// (common on Windows Wi-Fi: "forcibly closed by the remote host") resumes
/// where it stopped, within this call and on the next one.
fn fetch_archive(
    url: &str,
    size_hint: u64,
    wanted: &[&str],
    dir: &Path,
    part: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f32),
) -> Result<()> {
    if let Err(e) = download_resumable(url, part, size_hint, cancel, &mut |f| on_progress(f * 0.97))
    {
        // Cancelling means starting over next time; any other failure resumes.
        if cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(part);
        }
        return Err(e);
    }
    let unpacked = unpack(part, wanted, dir);
    if unpacked.is_err() {
        // A finished but unreadable archive must not be resumed next time.
        let _ = std::fs::remove_file(part);
    }
    unpacked?;
    let _ = std::fs::remove_file(part);
    on_progress(1.0);
    Ok(())
}

/// Raise a last word spoken much more quietly than the rest of the recording,
/// which Parakeet otherwise tends to drop ("with a good book" -> "with a good").
/// Port of the Mac's `ExpressAudioConditioner.liftingQuietTail`: on its corpus
/// 3 of 6 soft endings kept their last word before, 6 of 6 after, with no
/// change on 50 real recordings. Only the last 1.5 s can change.
pub fn lift_quiet_tail(samples: &[f32]) -> std::borrow::Cow<'_, [f32]> {
    use std::borrow::Cow;
    const FRAME: usize = 320; // 20 ms
    const TAIL: usize = TARGET_SAMPLE_RATE as usize * 3 / 2;
    const RAMP: usize = TARGET_SAMPLE_RATE as usize / 10;
    const MAX_GAIN: f32 = 4.0;
    /// At least 100 ms of speech in the tail: a soft word, not room hiss.
    const MIN_TAIL_SPEECH_FRAMES: usize = 5;
    fn percentile(values: &[f32], fraction: f64) -> f32 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f32::total_cmp);
        sorted[((sorted.len() as f64 * fraction) as usize).min(sorted.len() - 1)]
    }

    let frame_count = samples.len().saturating_sub(1) / FRAME;
    let tail_frames = TAIL / FRAME;
    if frame_count <= tail_frames {
        return Cow::Borrowed(samples);
    }
    let frames: Vec<f32> = (0..frame_count)
        .map(|i| {
            let f = &samples[i * FRAME..(i + 1) * FRAME];
            (f.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt()
        })
        .collect();
    let floor = percentile(&frames, 0.1);
    let bar = (3.0 * floor).max(0.008);
    let speech: Vec<f32> = frames.iter().copied().filter(|&r| r > bar).collect();
    if speech.is_empty() {
        return Cow::Borrowed(samples);
    }
    let typical = percentile(&speech, 0.5);
    let tail_speech: Vec<f32> = frames[frame_count - tail_frames..]
        .iter()
        .copied()
        .filter(|&r| r > bar)
        .collect();
    if tail_speech.len() < MIN_TAIL_SPEECH_FRAMES {
        return Cow::Borrowed(samples);
    }
    let tail_level = percentile(&tail_speech, 0.5);
    if tail_level >= 0.5 * typical {
        return Cow::Borrowed(samples);
    }
    let gain = MAX_GAIN.min(typical / tail_level);
    let mut lifted = samples.to_vec();
    let start = lifted.len() - tail_frames * FRAME;
    for (i, s) in lifted[start..].iter_mut().enumerate() {
        let ramp = (i as f32 / RAMP as f32).min(1.0);
        *s = (*s * (1.0 + (gain - 1.0) * ramp)).clamp(-1.0, 1.0);
    }
    Cow::Owned(lifted)
}

/// Unpack the `wanted` files that sit directly in the archive's top folder.
fn unpack(archive_path: &Path, wanted: &[&str], dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = bzip2::read::BzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // `<id>/<file>`, sometimes written `./<id>/<file>`: count real names only.
        let depth = path
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if wanted.contains(&name.as_str()) && depth <= 2 {
            entry.unpack(dir.join(&name))?;
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
    let spec =
        spec(id).ok_or_else(|| CoreError::Download(format!("Unknown Express model '{id}'.")))?;
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
            &root.join(format!("{}.tar.bz2.part", spec.id)),
            cancel,
            &mut |f| on_progress(f * (1.0 - boost_share)),
        )
        .and_then(|()| {
            if model_files(&staging).iter().all(|f| f.is_file()) {
                Ok(())
            } else {
                Err(CoreError::Download(
                    "The Express download is incomplete. Try again.".into(),
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
    // Word Boost never stands between you and Express (the Mac loads Express
    // without it too): if its download fails, Express is ready anyway and the
    // Model page offers Word Boost's download.
    if boost_share > 0.0 {
        if let Err(e) =
            download_word_boost(cancel, |f| on_progress(1.0 - boost_share + f * boost_share))
        {
            if cancel.load(Ordering::Relaxed) {
                return Err(e);
            }
            tracing::warn!("Express is ready, but Word Boost's download failed: {e}");
        }
    }
    on_progress(1.0);
    Ok(final_dir)
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
        // A soft last word is lifted, then Parakeet gets 0.4 s of trailing
        // silence, since it clips the last word when audio ends abruptly
        // (both from the Mac's ExpressAudioConditioner).
        let lifted = lift_quiet_tail(samples);
        let mut padded = Vec::with_capacity(lifted.len() + 6400);
        padded.extend_from_slice(&lifted);
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
                // Scores only: the log never holds what was said or the
                // dictionary words themselves.
                for p in &out.proposals {
                    tracing::info!(
                        "Word Boost {} a proposal [margin {:.2}, spelling {:.2}]",
                        if p.applied { "applied" } else { "rejected" },
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

    fn speech(secs: f32, amp: f32) -> Vec<f32> {
        let n = (secs * TARGET_SAMPLE_RATE as f32) as usize;
        (0..n).map(|i| amp * (i as f32 * 0.07).sin()).collect()
    }

    #[test]
    fn a_soft_last_word_is_lifted_and_normal_audio_is_left_alone() {
        // A pause, the body at 0.2, a final word at 0.03: lifted (gain
        // capped at 4). Real recordings have quiet moments that set the floor.
        let mut soft = vec![0.0; 16_000];
        soft.extend(speech(4.0, 0.2));
        soft.extend(speech(1.5, 0.03));
        let lifted = lift_quiet_tail(&soft);
        assert!(matches!(lifted, std::borrow::Cow::Owned(_)));
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let tail = &lifted[lifted.len() - 8000..];
        assert!(peak(tail) > 0.11 && peak(tail) <= 0.121, "{}", peak(tail));
        // The body never changes.
        assert_eq!(&lifted[..soft.len() - 24_000], &soft[..soft.len() - 24_000]);

        // An even recording, a silent tail, and a short clip come back untouched.
        assert!(matches!(
            lift_quiet_tail(&speech(5.0, 0.2)),
            std::borrow::Cow::Borrowed(_)
        ));
        let mut silent_tail = vec![0.0; 16_000];
        silent_tail.extend(speech(4.0, 0.2));
        silent_tail.extend(vec![0.0; 24_000]);
        assert!(matches!(
            lift_quiet_tail(&silent_tail),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(
            lift_quiet_tail(&speech(1.0, 0.02)),
            std::borrow::Cow::Borrowed(_)
        ));
    }

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

    /// Real network: a download stopped part-way resumes with a Range request,
    /// and the Word Boost archive (whose entries start with `./`) unpacks.
    /// `ORTTAAI_DOWNLOAD_TEST=1 cargo test -p orttaai-core --features express -- --ignored download_resumes`
    #[test]
    #[ignore]
    fn download_resumes_and_unpacks() {
        std::env::var("ORTTAAI_DOWNLOAD_TEST").expect("ORTTAAI_DOWNLOAD_TEST");
        let dir = std::env::temp_dir().join(format!("orttaai-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join("ctc.tar.bz2.part");
        let url = format!("{RELEASE_BASE}/{WORD_BOOST_ID}.tar.bz2");
        // Stop at about 30 %, as a dropped connection would.
        let stop = AtomicBool::new(false);
        let first = download_resumable(&url, &part, WORD_BOOST_ARCHIVE_BYTES, &stop, &mut |f| {
            if f > 0.3 {
                stop.store(true, Ordering::SeqCst);
            }
        });
        assert!(first.is_err());
        let partial = std::fs::metadata(&part).unwrap().len();
        assert!(
            partial > 0 && partial < WORD_BOOST_ARCHIVE_BYTES,
            "{partial}"
        );
        let mut lowest = 1.0f32;
        let go = AtomicBool::new(false);
        download_resumable(&url, &part, WORD_BOOST_ARCHIVE_BYTES, &go, &mut |f| {
            lowest = lowest.min(f)
        })
        .unwrap();
        assert!(
            lowest >= 0.29,
            "resumed, not restarted (first progress {lowest})"
        );
        assert_eq!(
            std::fs::metadata(&part).unwrap().len(),
            WORD_BOOST_ARCHIVE_BYTES
        );
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        unpack(&part, &["model.int8.onnx", "tokens.txt"], &out).unwrap();
        assert!(out.join("model.int8.onnx").is_file() && out.join("tokens.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
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
