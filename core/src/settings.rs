//! User settings. Serialized to JSON under the platform config dir
//! (`~/.config/orttaai` on Linux, `%APPDATA%\orttaai` on Windows) via `directories`.

use crate::error::Result;
use crate::types::{DecodeOptions, HotkeyCombo};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model_id: String,
    pub decode: DecodeOptions,
    pub push_to_talk: HotkeyCombo,
    pub preserve_clipboard: bool,
    pub low_latency: bool,
    pub ollama_endpoint: String,
    /// XDG RemoteDesktop portal restore token (Wayland) — lets us re-open the
    /// input-injection session on later launches without re-prompting.
    #[serde(default)]
    pub wayland_restore_token: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_id: "ggml-base.en".to_string(),
            decode: DecodeOptions::default(),
            push_to_talk: HotkeyCombo::default(),
            preserve_clipboard: true,
            low_latency: false,
            ollama_endpoint: "http://localhost:11434".to_string(),
            wayland_restore_token: None,
        }
    }
}

impl Settings {
    /// Cross-platform config-file path, or `None` if no home dir can be resolved.
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("org", "orttaai", "Orttaai")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    /// Load settings from disk, falling back to defaults on any error.
    pub fn load_or_default() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist settings to the config path, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()
            .ok_or_else(|| std::io::Error::other("no config directory available"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}
