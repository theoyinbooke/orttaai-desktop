//! Settings commands: the UI reads the whole `Settings` JSON and writes partial
//! patches, which are merged, clamped, saved, and broadcast as `settings-changed`.

use crate::err;
use orttaai_core::settings::Settings;
use tauri::{AppHandle, Emitter};

/// Open the system's microphone privacy page (Windows: Settings › Privacy ›
/// Microphone). Linux has no single place for it, so it says so.
#[tauri::command]
pub fn open_microphone_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        tauri_plugin_opener::open_url("ms-settings:privacy-microphone", None::<&str>).map_err(err)
    }
    #[cfg(not(windows))]
    {
        Err("Open your desktop's sound settings and check the input device.".into())
    }
}

/// Whether dictation uses Express with the saved settings. With an
/// English-only Express model and another language, Whisper does the work.
#[tauri::command]
pub fn express_in_use() -> bool {
    Settings::load_or_default().express_active()
}

#[tauri::command]
pub fn get_settings() -> Settings {
    Settings::load_or_default()
}

/// Merge `patch` (a partial `Settings` object) into the saved settings.
#[tauri::command]
pub fn update_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    let current = Settings::load_or_default();
    let next = current.merged(&patch).map_err(err)?;
    next.save().map_err(err)?;
    let _ = app.emit("settings-changed", &next);
    crate::on_settings_changed(&app, &current, &next);
    Ok(next)
}

#[tauri::command]
pub fn reset_settings(app: AppHandle) -> Result<Settings, String> {
    let current = Settings::load_or_default();
    // Keep onboarding state + the Wayland grant so a reset doesn't re-prompt.
    let next = Settings {
        has_completed_setup: current.has_completed_setup,
        wayland_restore_token: current.wayland_restore_token.clone(),
        ..Default::default()
    };
    next.save().map_err(err)?;
    let _ = app.emit("settings-changed", &next);
    crate::on_settings_changed(&app, &current, &next);
    Ok(next)
}

#[tauri::command]
pub fn clear_history(app: AppHandle) -> Result<(), String> {
    orttaai_core::store::Store::open_default()
        .and_then(|s| s.clear_history())
        .map_err(err)?;
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[tauri::command]
pub fn reset_app_data(app: AppHandle) -> Result<(), String> {
    orttaai_core::store::Store::open_default()
        .and_then(|s| s.reset_all())
        .map_err(err)?;
    let _ = app.emit("history-changed", ());
    let _ = app.emit("memory-changed", ());
    Ok(())
}
