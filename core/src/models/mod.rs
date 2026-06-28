//! Whisper model catalog + GGUF downloader (behind the `models` feature).
//!
//! Models are stored as `<data-dir>/orttaai/models/<id>.bin`, where the id is the
//! ggml file stem (e.g. `ggml-base.en`). The catalog mirrors the tiers the macOS
//! app offers; downloads stream from Hugging Face with a progress callback.

use crate::error::{CoreError, Result};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// A model offered for download.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub approx_size_mb: u32,
    pub multilingual: bool,
    pub url: String,
}

/// A catalog entry plus whether it is present on disk.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    #[serde(flatten)]
    pub spec: ModelSpec,
    pub downloaded: bool,
    pub path: String,
}

fn spec(id: &str, name: &str, size: u32, multilingual: bool) -> ModelSpec {
    ModelSpec {
        id: id.to_string(),
        name: name.to_string(),
        approx_size_mb: size,
        multilingual,
        url: format!("{HF_BASE}/{id}.bin"),
    }
}

/// The static catalog of downloadable models, fastest → most accurate.
///
/// Quantized (`q5`) tiers are the default recommendation: ~2.4–2.9× smaller than
/// f16 and faster to load and decode on CPU, with only a minor accuracy cost.
/// `large-v3-turbo` is the speed/accuracy sweet spot for a capable GPU.
pub fn catalog() -> Vec<ModelSpec> {
    vec![
        spec("ggml-tiny.en-q5_1", "Tiny · English (Q5)", 32, false),
        spec("ggml-base.en-q5_1", "Base · English (Q5)", 60, false),
        spec("ggml-base.en", "Base · English", 148, false),
        spec("ggml-base-q5_1", "Base (Q5)", 60, true),
        spec("ggml-small.en-q5_1", "Small · English (Q5)", 190, false),
        spec("ggml-small-q5_1", "Small (Q5)", 190, true),
        spec("ggml-medium-q5_0", "Medium (Q5)", 539, true),
        spec("ggml-large-v3-turbo-q5_0", "Large v3 Turbo (Q5)", 574, true),
        spec("ggml-large-v3-turbo", "Large v3 Turbo", 1620, true),
        spec("ggml-large-v3-q5_0", "Large v3 (Q5)", 1081, true),
    ]
}

/// `<data-dir>/orttaai/models`, created on demand.
pub fn models_dir() -> Result<PathBuf> {
    let dir = directories::ProjectDirs::from("org", "orttaai", "Orttaai")
        .map(|dirs| dirs.data_dir().join("models"))
        .ok_or_else(|| std::io::Error::other("no data directory available"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Local file path for a model id (whether or not it exists yet).
pub fn local_path(id: &str) -> Result<PathBuf> {
    Ok(models_dir()?.join(format!("{id}.bin")))
}

pub fn is_downloaded(id: &str) -> bool {
    local_path(id).map(|p| p.exists()).unwrap_or(false)
}

/// The catalog annotated with on-disk status.
pub fn list() -> Result<Vec<ModelInfo>> {
    let dir = models_dir()?;
    Ok(catalog()
        .into_iter()
        .map(|spec| {
            let path = dir.join(format!("{}.bin", spec.id));
            ModelInfo {
                downloaded: path.exists(),
                path: path.to_string_lossy().to_string(),
                spec,
            }
        })
        .collect())
}

/// Download a model by id, reporting progress in `0.0..=1.0`. Streams to a
/// temporary file and renames on success so partial downloads aren't usable.
pub fn download(id: &str, mut on_progress: impl FnMut(f64)) -> Result<PathBuf> {
    let url = catalog()
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| s.url)
        .ok_or_else(|| CoreError::Transcription(format!("unknown model id: {id}")))?;

    let final_path = local_path(id)?;
    let tmp_path = final_path.with_extension("bin.part");

    let response = ureq::get(&url)
        .call()
        .map_err(|e| CoreError::Transcription(format!("download request failed: {e}")))?;
    let total: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut downloaded: u64 = 0;
    on_progress(0.0);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        if let Some(total) = total {
            if total > 0 {
                on_progress((downloaded as f64 / total as f64).min(1.0));
            }
        }
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&tmp_path, &final_path)?;
    on_progress(1.0);
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::catalog;

    #[test]
    fn catalog_is_well_formed() {
        let entries = catalog();
        assert!(entries.len() >= 4);
        for spec in entries {
            assert!(spec.id.starts_with("ggml-"), "bad id: {}", spec.id);
            assert!(spec.url.ends_with(".bin"), "bad url: {}", spec.url);
            assert!(spec.approx_size_mb > 0);
        }
    }
}
