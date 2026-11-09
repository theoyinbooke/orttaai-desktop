//! "Express" engine. English runs Fluid Speech Mini (FluidVoice), the model
//! the Mac's Express engine runs as Core ML, here as its ONNX build on the ONNX
//! Runtime that `sherpa-onnx` links statically ([`super::fluid`]). Other
//! languages run NVIDIA Parakeet TDT 0.6B v3 (int8) through sherpa-onnx; PCs
//! that installed the earlier English model, Parakeet TDT 0.6B v2, keep it
//! until they switch.
//!
//! Fluid Speech Mini's files come from Hugging Face, pinned to one revision and
//! checked by SHA-256; the Parakeet models are `.tar.bz2` archives from the
//! sherpa-onnx releases. Each unpacks to `<models>/express/<id>/`.

use crate::error::{CoreError, Result};
use crate::models::download_resumable;
use crate::transcription::Transcriber;
use crate::types::{DecodeOptions, TARGET_SAMPLE_RATE};
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::fluid;

const RELEASE_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// Fluid Speech Mini, the English Express model (the Mac's).
pub const FLUID_MINI_ID: &str = "fluid-speech-mini-onnx";
/// The English model before Fluid Speech Mini.
pub const PARAKEET_V2_ID: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";
const FLUID_MINI_BASE: &str = "https://huggingface.co/FluidVoice-Labs/fluid-speech-mini-onnx/resolve/fc592620325027419883cf272e70588d8a788c0a";

/// One file of a model downloaded file by file.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

const FLUID_MINI_FILES: &[ModelFile] = &[
    ModelFile {
        name: "encoder.onnx.data",
        bytes: 253_429_760,
        sha256: "f865c582bec1bd57aa7f2242f3739f38987b7bf471fee4dca7d5c2714eec0747",
    },
    ModelFile {
        name: "decoder.onnx",
        bytes: 24_461_296,
        sha256: "9ac7e6c22952c242d56df0992ba19489a05b87a8ed37e23798469b9316099af8",
    },
    ModelFile {
        name: "joint.onnx",
        bytes: 11_840_320,
        sha256: "2e704908834b9f8cc7db88757b8966bac266e19bc00a527c46e9950281205253",
    },
    ModelFile {
        name: "frontend.onnx",
        bytes: 8_697_130,
        sha256: "cdbea06631920f25f6183e1389129e9c37106a766c6e130b7f0389b9531b43b7",
    },
    ModelFile {
        name: "tokenizer.json",
        bytes: 1_159_959,
        sha256: "7c200d88ac60da4de246cbd38e981a732f2e82580fdd285cb757d8d1f4af9627",
    },
    ModelFile {
        name: "encoder.onnx",
        bytes: 426_102,
        sha256: "f8b3a45e71a33b44179a21ef6d6474779747846c4b01c9fd690abcebb88bea86",
    },
    ModelFile {
        name: "mel_filters.npy",
        bytes: 131_712,
        sha256: "ce607f7ae1ce68fd01c02e1177fb1d11ee209c6176d35e47011917ca7d9cd4f2",
    },
    ModelFile {
        name: "config.json",
        bytes: 508,
        sha256: "0c4d0c84dadc0d439dfe3c843a4b482bf3ebb50421f781fcbafc7236903234f4",
    },
];

/// Where a model comes from.
#[derive(Debug, Clone, Copy)]
pub enum ModelSource {
    /// A sherpa-onnx release archive holding a Parakeet transducer.
    SherpaArchive,
    /// Files fetched one by one (Fluid Speech Mini).
    Files {
        base: &'static str,
        files: &'static [ModelFile],
    },
}

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
    /// Replaced by a newer model: listed only while installed.
    pub previous: bool,
    #[serde(skip)]
    pub source: ModelSource,
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
            id: FLUID_MINI_ID,
            name: "Express · English (Fluid Speech Mini)",
            description: "The Mac's Express model. English only, best with names and with other voices in the room.",
            // With Word Boost's model (WORD_BOOST_MB), which comes with it.
            approx_size_mb: 418,
            multilingual: false,
            archive_bytes: 300_146_787,
            previous: false,
            source: ModelSource::Files {
                base: FLUID_MINI_BASE,
                files: FLUID_MINI_FILES,
            },
        },
        ExpressModelSpec {
            id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
            name: "Express · Multilingual (Parakeet v3)",
            description: "Fast on CPU. 25 European languages, auto-detected.",
            approx_size_mb: 772,
            multilingual: true,
            archive_bytes: 487_170_055,
            previous: false,
            source: ModelSource::SherpaArchive,
        },
        ExpressModelSpec {
            id: PARAKEET_V2_ID,
            name: "Express · English (Parakeet v2)",
            description: "The earlier English model. Fluid Speech Mini replaces it.",
            approx_size_mb: 763,
            multilingual: false,
            archive_bytes: 482_440_461,
            previous: true,
            source: ModelSource::SherpaArchive,
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

/// The model id a dictation records: Fluid Speech Mini under the Mac's id, so
/// History and Insights name it the same on every device.
pub fn recorded_model_id(id: &str) -> &str {
    if id == FLUID_MINI_ID {
        "fluid-speech-mini"
    } else {
        id
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
    express_path(id, has_model)
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

/// Whether `dir` holds a whole model of either kind.
fn has_model(dir: &Path) -> bool {
    fluid::is_model_dir(dir) || model_files(dir).iter().all(|f| f.is_file())
}

pub fn is_installed(id: &str) -> bool {
    model_dir(id).map(|d| has_model(&d)).unwrap_or(false)
}

pub fn list() -> Result<Vec<ExpressModelInfo>> {
    let mut models = Vec::new();
    for spec in catalog() {
        let downloaded = is_installed(spec.id);
        if spec.previous && !downloaded {
            continue;
        }
        let dir = model_dir(spec.id)?;
        models.push(ExpressModelInfo {
            downloaded,
            path: dir.to_string_lossy().into_owned(),
            spec,
        });
    }
    Ok(models)
}

pub fn delete(id: &str) -> Result<()> {
    let dir = model_dir(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// The names Orttaai creates inside an `express` folder: each catalog model's
/// folder and Word Boost's, plus their download staging folders and partial
/// archives. Used by Reset App Data to remove only Orttaai's own files.
pub fn owned_entry_names() -> Vec<String> {
    catalog()
        .into_iter()
        .map(|s| s.id)
        .chain(std::iter::once(WORD_BOOST_ID))
        .flat_map(|id| {
            [
                id.to_string(),
                format!("{id}.part"),
                format!("{id}.tar.bz2.part"),
            ]
        })
        .collect()
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
        if let ModelSource::Files { base, files } = spec.source {
            // Finished files stay in the staging folder, so a failed download
            // picks up where it stopped; cancelling starts over.
            let fetched = fetch_files(base, files, &staging, cancel, &mut |f| {
                on_progress(f * (1.0 - boost_share))
            });
            match fetched {
                Ok(()) => {
                    let _ = std::fs::remove_dir_all(&final_dir);
                    std::fs::rename(&staging, &final_dir)?;
                }
                Err(e) => {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = std::fs::remove_dir_all(&staging);
                    }
                    return Err(e);
                }
            }
        }
    }
    if matches!(spec.source, ModelSource::SherpaArchive) && !is_installed(spec.id) {
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

/// Download `files` from `base` into `dir`, each checked by size and SHA-256,
/// reporting progress by bytes in `0.0..=1.0`. Files already there are kept.
fn fetch_files(
    base: &str,
    files: &[ModelFile],
    dir: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f32),
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let total: u64 = files.iter().map(|f| f.bytes).sum::<u64>().max(1);
    let mut done = 0u64;
    for file in files {
        let path = dir.join(file.name);
        let complete = std::fs::metadata(&path).is_ok_and(|m| m.len() == file.bytes);
        if !complete {
            let part = dir.join(format!("{}.part", file.name));
            let before = done;
            download_resumable(
                &format!("{base}/{}", file.name),
                &part,
                file.bytes,
                cancel,
                &mut |f| on_progress((before as f32 + f * file.bytes as f32) / total as f32),
            )?;
            if let Err(e) = crate::models::verify_file(&part, file.bytes, file.sha256) {
                // A finished but wrong file must not be resumed next time.
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
            std::fs::rename(&part, &path)?;
        }
        done += file.bytes;
        on_progress(done as f32 / total as f32);
    }
    Ok(())
}

/// The model behind Express: Fluid Speech Mini, or a Parakeet transducer
/// through sherpa-onnx.
enum Recognizer {
    Fluid(Box<fluid::FluidModel>),
    Sherpa(OfflineRecognizer),
}

/// Express's text and, for Word Boost, its token timings.
struct Recognized {
    text: String,
    tokens: Vec<String>,
    starts: Option<Vec<f32>>,
    durations: Vec<f32>,
}

impl Recognizer {
    fn recognize(&self, samples: &[f32]) -> Result<Option<Recognized>> {
        match self {
            Self::Fluid(model) => {
                let out = model.transcribe(samples)?;
                Ok(Some(Recognized {
                    text: out.text,
                    tokens: out.tokens,
                    starts: Some(out.starts),
                    durations: out.durations,
                }))
            }
            Self::Sherpa(recognizer) => {
                let stream = recognizer.create_stream();
                stream.accept_waveform(TARGET_SAMPLE_RATE as i32, samples);
                recognizer.decode(&stream);
                Ok(stream.get_result().map(|r| Recognized {
                    text: r.text.clone(),
                    tokens: r.tokens.clone(),
                    starts: r.timestamps.clone(),
                    durations: r.durations.clone().unwrap_or_default(),
                }))
            }
        }
    }
}

/// Express, implementing [`Transcriber`].
pub struct ExpressTranscriber {
    recognizer: Mutex<Option<Recognizer>>,
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

    /// Load from a model directory: Fluid Speech Mini's files, or a Parakeet
    /// transducer's (encoder/decoder/joiner/tokens).
    pub fn from_dir(dir: &Path, threads: i32) -> Result<Self> {
        let mut t = Self::new();
        if threads > 0 {
            t.threads = threads;
        }
        t.load_dir(dir)?;
        t.model = dir
            .file_name()
            .map(|n| recorded_model_id(&n.to_string_lossy()).to_string());
        Ok(t)
    }

    fn load_dir(&mut self, dir: &Path) -> Result<()> {
        if dir.join("joint.onnx").is_file() {
            let model = fluid::FluidModel::load(dir, self.threads)?;
            *self.recognizer.lock().unwrap() = Some(Recognizer::Fluid(Box::new(model)));
            return Ok(());
        }
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
        *self.recognizer.lock().unwrap() = Some(Recognizer::Sherpa(recognizer));
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
        self.model = Some(recorded_model_id(model_id).to_string());
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
        // Word Boost scores the same audio while Express decodes it, so both
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
                    .and_then(|recognizer| recognizer.recognize(&padded))
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
            (boost.as_ref(), log_probs, result.starts.as_ref())
        {
            let timings = crate::wordboost::rescore::TokenTiming::from_starts_and_durations(
                &result.tokens,
                starts,
                &result.durations,
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
        assert_eq!(c.len(), 3);
        let ids: std::collections::HashSet<_> = c.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(spec(FLUID_MINI_ID).is_some());
        assert!(spec(PARAKEET_V2_ID).is_some_and(|s| s.previous));
        assert!(spec("nope").is_none());
        // The English default is the first English model listed.
        assert_eq!(c[0].id, crate::settings::Settings::default().express_model);
    }

    #[test]
    fn fluid_files_add_up_to_the_listed_download() {
        let ModelSource::Files { files, .. } = spec(FLUID_MINI_ID).unwrap().source else {
            panic!("Fluid Speech Mini downloads file by file");
        };
        let total: u64 = files.iter().map(|f| f.bytes).sum();
        assert_eq!(total, spec(FLUID_MINI_ID).unwrap().archive_bytes);
        let mut names: Vec<_> = files.iter().map(|f| f.name).collect();
        names.sort_unstable();
        let mut wanted = fluid::FILES.to_vec();
        wanted.sort_unstable();
        assert_eq!(names, wanted);
        assert!(files.iter().all(|f| f.sha256.len() == 64));
    }

    #[test]
    fn fluid_speech_mini_is_recorded_under_the_macs_id() {
        assert_eq!(recorded_model_id(FLUID_MINI_ID), "fluid-speech-mini");
        assert_eq!(recorded_model_id(PARAKEET_V2_ID), PARAKEET_V2_ID);
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

    /// Accuracy run over an eval manifest (`gauntlet/asr_eval`), written in
    /// score.py's raw format: the whole Express path (quiet tail, padding,
    /// Word Boost with the manifest's `bias_vocabulary` when
    /// `ORTTAAI_EVAL_BIAS=1`), then number formatting as dictation does.
    /// `ORTTAAI_EXPRESS_MODEL_DIR=… ORTTAAI_WORD_BOOST_DIR=… ORTTAAI_EVAL_MANIFEST=… ORTTAAI_EVAL_OUT=…
    /// cargo test --release -p orttaai-core --features express --lib -- --ignored express_eval`
    #[test]
    #[ignore]
    fn express_eval_manifest() {
        let dir = std::env::var("ORTTAAI_EXPRESS_MODEL_DIR").expect("ORTTAAI_EXPRESS_MODEL_DIR");
        let manifest = std::env::var("ORTTAAI_EVAL_MANIFEST").expect("ORTTAAI_EVAL_MANIFEST");
        let out = std::env::var("ORTTAAI_EVAL_OUT").expect("ORTTAAI_EVAL_OUT");
        let bias = std::env::var("ORTTAAI_EVAL_BIAS").is_ok_and(|v| v == "1");
        let m: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let base = Path::new(&manifest).parent().unwrap();
        let t = ExpressTranscriber::from_dir(Path::new(&dir), 4).unwrap();
        let terms: Vec<crate::text::WordBoostTerm> = if bias {
            m["bias_vocabulary"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| crate::text::WordBoostTerm {
                    text: v.as_str().unwrap().into(),
                    aliases: Vec::new(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut items = Vec::new();
        for item in m["items"].as_array().unwrap() {
            let mut reader =
                hound::WavReader::open(base.join(item["wav"].as_str().unwrap())).unwrap();
            assert_eq!(reader.spec().sample_rate, TARGET_SAMPLE_RATE);
            let samples: Vec<f32> = match reader.spec().sample_format {
                hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
                hound::SampleFormat::Int => reader
                    .samples::<i16>()
                    .map(|s| s.unwrap() as f32 / 32768.0)
                    .collect(),
            };
            // Terms are set per dictation, as the app does.
            t.set_word_boost_terms(&terms);
            let started = std::time::Instant::now();
            let raw = t.transcribe(&samples, &DecodeOptions::default()).unwrap();
            let ms = started.elapsed().as_millis() as u64;
            let text = crate::text::numbers::format_numbers(&raw);
            items.push(serde_json::json!({"id": item["id"], "whole": {"text": text, "ms": ms}}));
        }
        let run = serde_json::json!({
            "model": t.loaded_model().unwrap_or("express"),
            "preset": "desktop",
            "speedup": 1,
            "biasEnabled": bias,
            "biasTermCount": terms.len(),
            "items": items,
        });
        std::fs::write(&out, serde_json::to_string_pretty(&run).unwrap()).unwrap();
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
