//! Whisper model catalog + verified, cancellable downloader (`models` feature).
//!
//! Models are stored as `<models-dir>/<id>.bin`, where the id is the ggml file
//! stem (e.g. `ggml-base.en-q5_1`). The models folder defaults to
//! `<data-dir>/models` and can be moved to a custom folder
//! (`Settings::models_dir`). Downloads stream from Hugging Face to a `.part`
//! file, are checked against the published size and SHA-256, and only then
//! renamed into place — a partial or corrupt file is never loadable.
//!
//! The catalog carries the Mac's Speed/Accuracy labels and descriptions; the
//! "Recommended" flag comes from [`crate::hardware`].

use crate::error::{CoreError, Result};
use crate::hardware::HardwareInfo;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
const HF_VAD_BASE: &str = "https://huggingface.co/ggml-org/whisper-vad/resolve/main";

/// Mac `SpeedLabel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SpeedLabel {
    Fastest,
    Fast,
    Moderate,
    Slow,
}

/// Mac `AccuracyLabel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AccuracyLabel {
    Basic,
    Good,
    Great,
    Best,
}

/// A model offered for download.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub approx_size_mb: u32,
    /// Exact published size, checked after download.
    pub size_bytes: u64,
    /// Published SHA-256 (Hugging Face LFS oid), checked after download.
    pub sha256: String,
    pub multilingual: bool,
    pub english_only: bool,
    pub quantized: bool,
    pub url: String,
    pub speed: SpeedLabel,
    pub accuracy: AccuracyLabel,
    /// Mac model description ("Short dictation", …).
    pub description: String,
    /// RAM this model wants for comfortable decoding.
    pub min_ram_gb: u32,
}

/// A catalog entry plus on-disk status and fit for this computer.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    #[serde(flatten)]
    pub spec: ModelSpec,
    pub downloaded: bool,
    pub path: String,
    /// Recommended for this computer's hardware tier + dictation language.
    pub recommended: bool,
    /// Fits this computer's RAM.
    pub supported: bool,
    /// A download is in progress.
    pub downloading: bool,
}

/// Mac family heuristics (`ModelSettingsView` speed/accuracy/description).
fn family_labels(id: &str) -> (SpeedLabel, AccuracyLabel, &'static str, u32) {
    let l = id.to_ascii_lowercase();
    if l.contains("tiny") {
        (
            SpeedLabel::Fastest,
            AccuracyLabel::Basic,
            "Quick notes, commands",
            2,
        )
    } else if l.contains("base") {
        (SpeedLabel::Fast, AccuracyLabel::Good, "Short dictation", 2)
    } else if l.contains("small") {
        (
            SpeedLabel::Fast,
            AccuracyLabel::Great,
            "General dictation",
            4,
        )
    } else if l.contains("medium") {
        (
            SpeedLabel::Moderate,
            AccuracyLabel::Great,
            "Longer dictation",
            8,
        )
    } else if l.contains("turbo") {
        (
            SpeedLabel::Moderate,
            AccuracyLabel::Best,
            "Maximum accuracy, optimized speed",
            8,
        )
    } else {
        (
            SpeedLabel::Slow,
            AccuracyLabel::Best,
            "Highest accuracy, slowest",
            16,
        )
    }
}

fn spec(id: &str, name: &str, size_bytes: u64, sha256: &str) -> ModelSpec {
    let (speed, accuracy, description, min_ram_gb) = family_labels(id);
    let english_only = id.contains(".en");
    ModelSpec {
        id: id.to_string(),
        name: name.to_string(),
        approx_size_mb: (size_bytes / 1_000_000) as u32,
        size_bytes,
        sha256: sha256.to_string(),
        multilingual: !english_only,
        english_only,
        quantized: id.contains("-q5") || id.contains("-q8"),
        url: format!("{HF_BASE}/{id}.bin"),
        speed,
        accuracy,
        description: description.to_string(),
        min_ram_gb,
    }
}

/// The static catalog of downloadable models, fastest → most accurate. Sizes
/// and hashes are the published Hugging Face LFS values.
///
/// Quantized (`q5`) tiers are the default recommendation: ~2.4–2.9× smaller
/// than f16 and faster to load and decode on CPU, with only a minor accuracy
/// cost. `large-v3-turbo` is the speed/accuracy sweet spot for a capable GPU.
pub fn catalog() -> Vec<ModelSpec> {
    vec![
        spec(
            "ggml-tiny.en-q5_1",
            "Tiny · English (Q5)",
            32_166_155,
            "c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b",
        ),
        spec(
            "ggml-tiny-q5_1",
            "Tiny (Q5)",
            32_152_673,
            "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        ),
        spec(
            "ggml-base.en-q5_1",
            "Base · English (Q5)",
            59_721_011,
            "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        ),
        spec(
            "ggml-base-q5_1",
            "Base (Q5)",
            59_707_625,
            "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        ),
        spec(
            "ggml-base.en",
            "Base · English",
            147_964_211,
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        ),
        spec(
            "ggml-small.en-q5_1",
            "Small · English (Q5)",
            190_098_681,
            "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        ),
        spec(
            "ggml-small-q5_1",
            "Small (Q5)",
            190_085_487,
            "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        ),
        spec(
            "ggml-small.en",
            "Small · English",
            487_614_201,
            "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        ),
        spec(
            "ggml-medium.en-q5_0",
            "Medium · English (Q5)",
            539_225_533,
            "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0",
        ),
        spec(
            "ggml-medium-q5_0",
            "Medium (Q5)",
            539_212_467,
            "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        ),
        spec(
            "ggml-large-v3-turbo-q5_0",
            "Large v3 Turbo (Q5)",
            574_041_195,
            "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        ),
        spec(
            "ggml-large-v3-q5_0",
            "Large v3 (Q5)",
            1_081_140_203,
            "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        ),
        spec(
            "ggml-large-v3-turbo",
            "Large v3 Turbo",
            1_624_555_275,
            "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        ),
    ]
}

/// With English dictation, multilingual Small uses its English-only
/// checkpoint, which is more accurate for English (Mac
/// `TranscriptionModelSelectionPolicy`). Every other model, and Small in any
/// other language or auto-detect, is used as chosen.
pub fn english_variant(id: &str, language: &str) -> Option<&'static str> {
    let lang = language.trim().to_ascii_lowercase();
    if lang != "en" && !lang.starts_with("en-") {
        return None;
    }
    match id.trim() {
        "ggml-small-q5_1" => Some("ggml-small.en-q5_1"),
        "ggml-small" => Some("ggml-small.en"),
        _ => None,
    }
}

pub fn find_spec(id: &str) -> Option<ModelSpec> {
    catalog().into_iter().find(|s| s.id == id)
}

/// The default models folder: `<data-dir>/models` (honours `ORTTAAI_DATA_DIR`).
pub fn default_models_dir() -> Result<PathBuf> {
    let base = if let Some(dir) = std::env::var_os("ORTTAAI_DATA_DIR") {
        PathBuf::from(dir)
    } else {
        directories::ProjectDirs::from("org", "orttaai", "Orttaai")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .ok_or_else(|| std::io::Error::other("no data directory available"))?
    };
    Ok(base.join("models"))
}

/// The models folder for `custom` (a user-chosen folder) or the default.
pub fn models_dir_for(custom: Option<&str>) -> Result<PathBuf> {
    let dir = match custom.map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => default_models_dir()?,
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The active models folder (custom folder from settings, else the default).
pub fn models_dir() -> Result<PathBuf> {
    let settings = crate::settings::Settings::load_or_default();
    models_dir_for(settings.models_dir.as_deref())
}

/// Check a folder can hold models: creatable and writable.
pub fn check_models_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    let probe = path.join(".orttaai-write-test");
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)?;
    Ok(())
}

/// Local file path for a model id (whether or not it exists yet).
pub fn local_path(id: &str) -> Result<PathBuf> {
    Ok(models_dir()?.join(format!("{id}.bin")))
}

/// Present on disk with the published size (a quick integrity check).
pub fn is_downloaded_in(dir: &Path, id: &str) -> bool {
    let path = dir.join(format!("{id}.bin"));
    match (std::fs::metadata(&path), find_spec(id)) {
        (Ok(meta), Some(spec)) => meta.len() == spec.size_bytes,
        (Ok(meta), None) => meta.len() > 0,
        _ => false,
    }
}

pub fn is_downloaded(id: &str) -> bool {
    models_dir()
        .map(|d| is_downloaded_in(&d, id))
        .unwrap_or(false)
}

/// Delete a downloaded model's file to reclaim disk. No-op if it isn't present.
pub fn delete(id: &str) -> Result<()> {
    if find_spec(id).is_none() {
        return Err(CoreError::Transcription(format!("unknown model id: {id}")));
    }
    let path = local_path(id)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let part = path.with_extension("bin.part");
    if part.exists() {
        let _ = std::fs::remove_file(part);
    }
    Ok(())
}

/// The catalog annotated with on-disk status and hardware fit.
pub fn list(hw: Option<&HardwareInfo>, language: &str) -> Result<Vec<ModelInfo>> {
    let dir = models_dir()?;
    let recommended = hw.map(|h| crate::hardware::recommended_model(h.tier, language));
    let active = active_downloads();
    Ok(catalog()
        .into_iter()
        .map(|spec| {
            let path = dir.join(format!("{}.bin", spec.id));
            ModelInfo {
                downloaded: is_downloaded_in(&dir, &spec.id),
                path: path.to_string_lossy().to_string(),
                recommended: recommended.as_deref() == Some(spec.id.as_str()),
                supported: hw
                    .map(|h| h.ram_gb == 0 || h.ram_gb >= spec.min_ram_gb)
                    .unwrap_or(true),
                downloading: active.contains(&spec.id),
                spec,
            }
        })
        .collect())
}

/// Bytes used by downloaded catalog models in `dir`.
pub fn disk_usage(dir: &Path) -> u64 {
    catalog()
        .iter()
        .filter_map(|s| std::fs::metadata(dir.join(format!("{}.bin", s.id))).ok())
        .map(|m| m.len())
        .sum()
}

// ---- Downloads --------------------------------------------------------------

/// Progress of one download.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub fraction: f64,
    pub downloaded: u64,
    pub total: u64,
}

fn cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    CANCELS.get_or_init(Default::default)
}

/// Ids with a download in flight.
pub fn active_downloads() -> Vec<String> {
    cancels().lock().unwrap().keys().cloned().collect()
}

/// Ask an in-flight download to stop. Returns whether one was running.
pub fn cancel_download(id: &str) -> bool {
    match cancels().lock().unwrap().get(id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// Download error kinds the UI distinguishes.
pub fn is_cancelled(err: &CoreError) -> bool {
    matches!(err, CoreError::Transcription(m) if m == CANCELLED)
}

const CANCELLED: &str = "download cancelled";

/// Download a catalog model by id into the active models folder.
pub fn download(id: &str, on_progress: impl FnMut(Progress)) -> Result<PathBuf> {
    let spec =
        find_spec(id).ok_or_else(|| CoreError::Transcription(format!("unknown model id: {id}")))?;
    let dest = local_path(id)?;
    download_verified(
        id,
        &spec.url,
        spec.size_bytes,
        &spec.sha256,
        &dest,
        on_progress,
    )
}

struct CancelGuard(String);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        cancels().lock().unwrap().remove(&self.0);
    }
}

/// Stream `url` to `dest` via `dest.part`, then check size + SHA-256 before the
/// rename. Cancellable with [`cancel_download`] under `key`.
pub fn download_verified(
    key: &str,
    url: &str,
    expected_size: u64,
    expected_sha256: &str,
    dest: &Path,
    mut on_progress: impl FnMut(Progress),
) -> Result<PathBuf> {
    let cancel = {
        let mut map = cancels().lock().unwrap();
        if map.contains_key(key) {
            return Err(CoreError::Transcription(format!(
                "{key} is already downloading"
            )));
        }
        let flag = Arc::new(AtomicBool::new(false));
        map.insert(key.to_string(), flag.clone());
        flag
    };
    let _guard = CancelGuard(key.to_string());

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The partial file survives a dropped connection, so the next try (or the
    // next click) picks up where it stopped.
    let tmp = dest.with_extension("bin.part");
    on_progress(Progress {
        fraction: 0.0,
        downloaded: 0,
        total: expected_size,
    });
    let fetched = download_resumable(url, &tmp, expected_size, &cancel, &mut |f| {
        on_progress(Progress {
            fraction: f as f64,
            downloaded: (f as f64 * expected_size as f64) as u64,
            total: expected_size,
        })
    });
    if let Err(e) = fetched {
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&tmp);
            return Err(CoreError::Transcription(CANCELLED.into()));
        }
        return Err(e);
    }
    if let Err(e) = verify_file(&tmp, expected_size, expected_sha256) {
        // A finished but wrong file must not be resumed next time.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, dest)?;
    on_progress(Progress {
        fraction: 1.0,
        downloaded: expected_size,
        total: expected_size,
    });
    Ok(dest.to_path_buf())
}

/// Check a downloaded file's size and SHA-256 (either check is skipped when
/// its expected value is unknown).
fn verify_file(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<()> {
    let size = std::fs::metadata(path)?.len();
    if expected_size > 0 && size != expected_size {
        return Err(CoreError::Download(format!(
            "The download is incomplete (got {size} of {expected_size} bytes). Try again."
        )));
    }
    if !expected_sha256.is_empty() && !sha256_file(path)?.eq_ignore_ascii_case(expected_sha256) {
        return Err(CoreError::Download(
            "The downloaded file failed its checksum. Try again.".into(),
        ));
    }
    Ok(())
}

/// Tries in a row that make no progress before a download gives up.
const DOWNLOAD_ATTEMPTS: u32 = 6;
/// Longest one request may run. A connection that stalls without being closed
/// would otherwise block forever; when this runs out the download carries on
/// from where it is with a fresh request.
const REQUEST_SECONDS: u64 = 120;

/// GET `url` into `part`, continuing an earlier partial file with an HTTP
/// Range request and retrying dropped or stalled connections with a short
/// back-off. Only tries that fetch nothing count towards giving up.
pub(crate) fn download_resumable(
    url: &str,
    part: &Path,
    size_hint: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f32),
) -> Result<()> {
    let cap = std::time::Duration::from_secs(REQUEST_SECONDS);
    download_resumable_capped(url, part, size_hint, cancel, on_progress, cap)
}

fn download_resumable_capped(
    url: &str,
    part: &Path,
    size_hint: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f32),
    request_cap: std::time::Duration,
) -> Result<()> {
    let cancelled = || CoreError::Download("Download cancelled.".into());
    use std::time::Duration;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(request_cap))
        .build()
        .into();
    let mut last_error = String::new();
    let mut failures = 0u32;
    let mut progressed = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        if progressed {
            // The last request got somewhere: carry on straight away.
            failures = 0;
        } else if failures > 0 {
            if failures >= DOWNLOAD_ATTEMPTS {
                break;
            }
            let wait = [1u64, 2, 4, 8, 8][(failures as usize - 1).min(4)];
            tracing::warn!("download interrupted ({last_error}); retrying in {wait}s");
            std::thread::sleep(Duration::from_secs(wait));
        }
        let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let mut request = agent.get(url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = match request.call() {
            Ok(r) => r,
            // Nothing left to fetch: the earlier attempt got it all.
            Err(ureq::Error::StatusCode(416)) if have > 0 => return Ok(()),
            Err(e) => {
                last_error = e.to_string();
                failures += 1;
                progressed = false;
                continue;
            }
        };
        let resumed = response.status().as_u16() == 206;
        let length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let start = if resumed { have } else { 0 };
        let total = length.map_or(size_hint.max(start), |l| start + l);
        let mut file = if resumed {
            std::fs::OpenOptions::new().append(true).open(part)?
        } else {
            std::fs::File::create(part)?
        };
        let mut reader = response.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = start;
        let mut reported = u64::MAX;
        let outcome: std::io::Result<()> = loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            match reader.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    if let Err(e) = std::io::Write::write_all(&mut file, &buf[..n]) {
                        break Err(e);
                    }
                    got += n as u64;
                    let step = got * 200 / total.max(1);
                    if step != reported {
                        reported = step;
                        on_progress((got as f32 / total.max(1) as f32).min(1.0));
                    }
                }
                Err(e) => break Err(e),
            }
        };
        match outcome {
            Ok(()) if length.is_none_or(|l| got >= start + l) => return Ok(()),
            Ok(()) => last_error = "the connection ended early".into(),
            Err(e) => last_error = e.to_string(),
        }
        // A server that ignores Range starts over every time, so only count
        // progress that a resumed (or first) request made.
        progressed = got > start && (resumed || have == 0);
        if !progressed {
            failures += 1;
        }
    }
    Err(CoreError::Download(format!(
        "Couldn't finish the download after {DOWNLOAD_ATTEMPTS} tries. Check your connection and try again; it picks up where it stopped. ({last_error})"
    )))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file on disk (full verification of an existing model).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Fully verify a downloaded model against the published hash.
pub fn verify(id: &str) -> Result<bool> {
    let spec =
        find_spec(id).ok_or_else(|| CoreError::Transcription(format!("unknown model id: {id}")))?;
    let path = local_path(id)?;
    if !path.exists() {
        return Ok(false);
    }
    if std::fs::metadata(&path)?.len() != spec.size_bytes {
        return Ok(false);
    }
    Ok(sha256_file(&path)?.eq_ignore_ascii_case(&spec.sha256))
}

// ---- Silero VAD -------------------------------------------------------------

/// whisper.cpp Silero v5.1.2 (`ggml-org/whisper-vad`), ~0.9 MB.
pub const VAD_SIZE_BYTES: u64 = 885_098;
pub const VAD_SHA256: &str = "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf";
/// Download key for [`cancel_download`].
pub const VAD_DOWNLOAD_ID: &str = "silero-vad";

/// Where the voice-detector model lives (next to the speech models).
pub fn vad_model_path() -> Result<PathBuf> {
    Ok(models_dir()?.join(crate::vad::SILERO_MODEL_FILE))
}

/// Present with the published size.
pub fn vad_installed() -> bool {
    vad_model_path()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() == VAD_SIZE_BYTES)
        .unwrap_or(false)
}

/// Download the Silero model (checksum-verified). Until it's present,
/// hands-free auto-stop uses the adaptive-loudness fallback.
pub fn download_vad(on_progress: impl FnMut(Progress)) -> Result<PathBuf> {
    let dest = vad_model_path()?;
    download_verified(
        VAD_DOWNLOAD_ID,
        &format!("{HF_VAD_BASE}/{}", crate::vad::SILERO_MODEL_FILE),
        VAD_SIZE_BYTES,
        VAD_SHA256,
        &dest,
        on_progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed() {
        let entries = catalog();
        assert!(entries.len() >= 10);
        for spec in &entries {
            assert!(spec.id.starts_with("ggml-"), "bad id: {}", spec.id);
            assert!(spec.url.ends_with(".bin"), "bad url: {}", spec.url);
            assert_eq!(spec.sha256.len(), 64);
            assert!(spec.size_bytes > 1_000_000);
            assert_eq!(spec.english_only, spec.id.contains(".en"));
        }
        let mut ids: Vec<_> = entries.iter().map(|s| &s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), entries.len());
    }

    #[test]
    fn mac_labels_by_family() {
        let tiny = find_spec("ggml-tiny.en-q5_1").unwrap();
        assert_eq!(
            (tiny.speed, tiny.accuracy),
            (SpeedLabel::Fastest, AccuracyLabel::Basic)
        );
        let base = find_spec("ggml-base.en-q5_1").unwrap();
        assert_eq!(
            (base.speed, base.accuracy),
            (SpeedLabel::Fast, AccuracyLabel::Good)
        );
        let turbo = find_spec("ggml-large-v3-turbo-q5_0").unwrap();
        assert_eq!(turbo.description, "Maximum accuracy, optimized speed");
        let large = find_spec("ggml-large-v3-q5_0").unwrap();
        assert_eq!(large.speed, SpeedLabel::Slow);
    }

    use std::io::Write;

    fn tmp_file(name: &str) -> (PathBuf, std::fs::File) {
        let dir = std::env::temp_dir().join(format!("orttaai-models-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let f = std::fs::File::create(&p).unwrap();
        (p, f)
    }

    #[test]
    fn verification_accepts_a_good_file_and_rejects_bad_ones() {
        let data = b"hello whisper".to_vec();
        let sha = hex(&Sha256::digest(&data));
        let write = |name: &str, bytes: &[u8]| {
            let (p, mut f) = tmp_file(name);
            f.write_all(bytes).unwrap();
            p
        };
        let good = write("good.bin", &data);
        verify_file(&good, 13, &sha).unwrap();
        assert_eq!(sha256_file(&good).unwrap(), sha);

        let short = write("short.bin", &data[..5]);
        let err = verify_file(&short, 13, &sha).unwrap_err();
        assert!(err.to_string().contains("incomplete"));

        let corrupt = write("corrupt.bin", b"hello WHISPER");
        let err = verify_file(&corrupt, 13, &sha).unwrap_err();
        assert!(err.to_string().contains("checksum"));

        // Unknown size and hash: nothing to check against.
        verify_file(&corrupt, 0, "").unwrap();
    }

    #[test]
    fn cancel_during_retries_reports_cancelled_and_leaves_no_part() {
        let (dest, _) = tmp_file("cancelled.bin");
        let _ = std::fs::remove_file(&dest);
        let key = "test-cancel-mid-retry";
        let handle = {
            let dest = dest.clone();
            // Nothing listens on port 9, so every attempt fails and retries.
            std::thread::spawn(move || {
                download_verified(key, "http://127.0.0.1:9/never", 10, "", &dest, |_| {})
            })
        };
        while !cancel_download(key) {
            std::thread::yield_now();
        }
        let err = handle.join().unwrap().unwrap_err();
        assert!(is_cancelled(&err), "{err}");
        assert!(!dest.with_extension("bin.part").exists());
    }

    /// A server that sends part of the file and then goes quiet without
    /// closing: the request is cut off at its cap and the rest is fetched with
    /// a Range request.
    #[test]
    fn stalled_connection_is_cut_off_and_resumed() {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        let body: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Arc::new(body.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        // Each connection gets its own thread, so a retry isn't queued behind
        // the stalled one.
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().take(2).enumerate() {
                let (served, tx) = (Arc::clone(&served), tx.clone());
                std::thread::spawn(move || {
                    let mut stream = stream.unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line.trim().is_empty() {
                            break;
                        }
                        let lower = line.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("range: bytes=") {
                            range = v.trim().trim_end_matches('-').parse::<usize>().ok();
                        }
                    }
                    tx.send(range).unwrap();
                    if n == 0 {
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n").unwrap();
                        stream.write_all(&served[..400]).unwrap();
                        stream.flush().unwrap();
                        // Stall, holding the connection open.
                        std::thread::sleep(std::time::Duration::from_secs(8));
                    } else {
                        let from = range.unwrap_or(0);
                        write!(
                            stream,
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-999/1000\r\n\r\n",
                            1000 - from
                        )
                        .unwrap();
                        stream.write_all(&served[from..]).unwrap();
                    }
                });
            }
        });
        let (part, _) = tmp_file("stalled.bin.part");
        let _ = std::fs::remove_file(&part);
        let started = std::time::Instant::now();
        download_resumable_capped(
            &format!("http://127.0.0.1:{port}/model.bin"),
            &part,
            1000,
            &AtomicBool::new(false),
            &mut |_| {},
            std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert!(started.elapsed().as_secs() < 7, "cut off at the cap");
        assert_eq!(std::fs::read(&part).unwrap(), body);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![None, Some(400)]);
    }

    #[test]
    fn english_small_uses_the_english_checkpoint() {
        assert_eq!(
            english_variant("ggml-small-q5_1", "en"),
            Some("ggml-small.en-q5_1")
        );
        assert_eq!(
            english_variant("ggml-small", "en-GB"),
            Some("ggml-small.en")
        );
        assert_eq!(english_variant("ggml-small-q5_1", "auto"), None);
        assert_eq!(english_variant("ggml-small-q5_1", "fr"), None);
        assert_eq!(english_variant("ggml-base-q5_1", "en"), None);
        // Both ends of the swap are real catalog models.
        assert!(
            find_spec("ggml-small-q5_1").is_some() && find_spec("ggml-small.en-q5_1").is_some()
        );
    }

    #[test]
    fn cancel_registry_tracks_active_downloads() {
        assert!(!cancel_download("nothing-running"));
        let flag = Arc::new(AtomicBool::new(false));
        cancels()
            .lock()
            .unwrap()
            .insert("x-test".into(), flag.clone());
        assert!(active_downloads().contains(&"x-test".to_string()));
        assert!(cancel_download("x-test"));
        assert!(flag.load(Ordering::SeqCst));
        drop(CancelGuard("x-test".into()));
        assert!(!active_downloads().contains(&"x-test".to_string()));
    }

    #[test]
    fn custom_models_dir_is_used_and_checked() {
        let dir =
            std::env::temp_dir().join(format!("orttaai-custom-models-{}", std::process::id()));
        let got = models_dir_for(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(got, dir);
        check_models_dir(&dir).unwrap();
        assert!(!is_downloaded_in(&dir, "ggml-base.en-q5_1"));
        std::fs::write(dir.join("ggml-base.en-q5_1.bin"), b"short").unwrap();
        assert!(
            !is_downloaded_in(&dir, "ggml-base.en-q5_1"),
            "wrong size is not downloaded"
        );
    }
}
