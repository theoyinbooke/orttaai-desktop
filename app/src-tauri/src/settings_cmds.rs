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

/// Settings › Reset App Data (Mac `AppResetService`): stop dictation, empty the
/// database, delete the speech models Orttaai downloaded, the settings file and
/// the diagnostic log, then restart into first-run Setup. Only a failed
/// database reset is reported; every later step is best-effort and logged.
/// Files the user saved elsewhere (share cards, memory exports) are kept.
#[tauri::command]
pub async fn reset_app_data(app: AppHandle) -> Result<(), String> {
    use orttaai_core::{models, store::Store, transcription::express};
    use tauri::Manager;

    // Read where models live before the settings file goes.
    let settings = Settings::load_or_default();
    let custom_models = settings
        .models_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    if let Err(e) = crate::engine::stop_dictation(app.clone(), app.state()) {
        tracing::warn!("reset: stopping dictation failed: {e}");
    }
    for id in models::active_downloads() {
        models::cancel_download(&id);
    }
    for spec in express::catalog() {
        crate::express_cmds::express_cancel(spec.id.to_string());
    }
    crate::express_cmds::express_cancel(express::WORD_BOOST_ID.to_string());

    let store = Store::open_default().map_err(err)?;
    store.reset_all().map_err(err)?;
    if let Err(e) = store.vacuum() {
        tracing::warn!("reset: compacting the database failed: {e}");
    }
    drop(store);
    let _ = app.emit("history-changed", ());
    let _ = app.emit("memory-changed", ());

    // The active models folder, plus the default one (Express installed there
    // before a custom folder was chosen). Only Orttaai's own files go.
    let mut model_dirs: Vec<std::path::PathBuf> = custom_models.into_iter().collect();
    match models::default_models_dir() {
        Ok(d) if !model_dirs.contains(&d) => model_dirs.push(d),
        Ok(_) => {}
        Err(e) => tracing::warn!("reset: no default models folder: {e}"),
    }
    for dir in &model_dirs {
        for (path, e) in models::delete_owned_in(dir) {
            tracing::warn!("reset: couldn't delete {}: {e}", path.display());
        }
    }
    if let Ok(default) = models::default_models_dir() {
        // Orttaai's own folder: removed only if nothing else is left in it.
        let _ = std::fs::remove_dir(default);
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Some(path) = Settings::config_path() {
        files.push(path.with_extension("json.tmp"));
        files.push(path);
    }
    if let Ok(dir) = Store::data_dir() {
        files.push(dir.join("orttaai.log"));
    }
    for path in files {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("reset: couldn't delete {}: {e}", path.display()),
        }
    }

    // Without a settings file the next launch opens first-run Setup.
    app.request_restart();
    Ok(())
}
