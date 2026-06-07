//! Orttaai desktop shell — a thin Tauri layer over `orttaai-core`.
//!
//! The UI is disposable; all real logic lives in the core. Read-only commands
//! expose settings + history; the engine commands drive the real dictation loop
//! (whisper + cpal + injection behind a global hotkey) and emit live state events.

use orttaai_core::audio::CpalAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::hotkey::{HotkeyCallback, HotkeyManager, SystemHotkeyManager};
use orttaai_core::injection::SystemTextInjector;
use orttaai_core::settings::Settings;
use orttaai_core::store::Store;
use orttaai_core::transcription::WhisperTranscriber;
use orttaai_core::types::{DecodeOptions, HotkeyCombo, Modifier};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State,
};

const TRAY_ID: &str = "main";

// ---- Engine state (Tauri-managed) ------------------------------------------

#[derive(Default)]
struct EngineState {
    hotkey: Mutex<Option<SystemHotkeyManager>>,
    running: AtomicBool,
}

#[derive(Serialize)]
struct EngineStatus {
    running: bool,
}

#[tauri::command]
fn engine_status(state: State<EngineState>) -> EngineStatus {
    EngineStatus {
        running: state.running.load(Ordering::SeqCst),
    }
}

#[tauri::command]
fn start_dictation(
    app: AppHandle,
    state: State<EngineState>,
    model_path: String,
) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        return Ok(());
    }
    let settings = Settings::load_or_default();

    // Empty path → use the active model from settings (must be downloaded).
    let resolved = if model_path.trim().is_empty() {
        let path =
            orttaai_core::models::local_path(&settings.model_id).map_err(|e| e.to_string())?;
        if !path.exists() {
            return Err(format!(
                "model '{}' is not downloaded — pick one in the Models tab",
                settings.model_id
            ));
        }
        path
    } else {
        std::path::PathBuf::from(&model_path)
    };

    let transcriber = WhisperTranscriber::from_path(&resolved).map_err(|e| e.to_string())?;
    let memory = Store::open_default()
        .and_then(|s| s.load_memory_service())
        .unwrap_or_default();
    let coordinator = DictationCoordinator::new(
        Box::new(transcriber),
        Box::new(CpalAudioCapture::new()),
        Box::new(SystemTextInjector::new()),
        memory,
        DecodeOptions::default(),
    );
    let coordinator = Arc::new(Mutex::new(coordinator));

    let on_down: HotkeyCallback = {
        let app = app.clone();
        let coordinator = coordinator.clone();
        Box::new(move || {
            if coordinator.lock().unwrap().on_press().is_ok() {
                emit_state(&app, "recording");
            }
        })
    };
    let on_up: HotkeyCallback = {
        let app = app.clone();
        let coordinator = coordinator.clone();
        Box::new(move || {
            emit_state(&app, "processing");
            match coordinator.lock().unwrap().on_release() {
                Ok(outcome) => {
                    if let Some(text) = outcome.transcript {
                        let _ = app.emit("transcript", text);
                    }
                }
                Err(e) => {
                    let _ = app.emit("engine-error", e.to_string());
                }
            }
            emit_state(&app, "idle");
        })
    };

    let mut hotkey = SystemHotkeyManager::new();
    hotkey
        .register(settings.push_to_talk, on_down, on_up)
        .map_err(|e| e.to_string())?;
    *state.hotkey.lock().unwrap() = Some(hotkey);
    state.running.store(true, Ordering::SeqCst);
    emit_state(&app, "idle");
    Ok(())
}

#[tauri::command]
fn stop_dictation(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
    if let Some(mut hotkey) = state.hotkey.lock().unwrap().take() {
        hotkey.unregister().map_err(|e| e.to_string())?;
    }
    state.running.store(false, Ordering::SeqCst);
    emit_state(&app, "off");
    Ok(())
}

/// Broadcast a state change to the frontend and reflect it in the tray tooltip.
fn emit_state(app: &AppHandle, state: &str) {
    let _ = app.emit("engine-state", state);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = match state {
            "recording" => "Orttaai — recording…",
            "processing" => "Orttaai — transcribing…",
            "idle" => "Orttaai — listening",
            _ => "Orttaai",
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

// ---- Settings + models ------------------------------------------------------

#[derive(serde::Deserialize)]
struct SettingsInput {
    model_id: String,
    push_to_talk: String,
    preserve_clipboard: bool,
    low_latency: bool,
    ollama_endpoint: String,
}

#[tauri::command]
fn set_settings(input: SettingsInput) -> Result<(), String> {
    let mut settings = Settings::load_or_default();
    settings.model_id = input.model_id;
    settings.push_to_talk = HotkeyCombo::parse(&input.push_to_talk);
    settings.preserve_clipboard = input.preserve_clipboard;
    settings.low_latency = input.low_latency;
    settings.ollama_endpoint = input.ollama_endpoint;
    settings.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_models() -> Result<Vec<orttaai_core::models::ModelInfo>, String> {
    orttaai_core::models::list().map_err(|e| e.to_string())
}

/// Download a model in the background, emitting `model-progress`/`model-done`/
/// `model-error` events so the UI can show a progress bar without blocking.
#[tauri::command]
fn download_model(app: AppHandle, id: String) {
    std::thread::spawn(move || {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let result = orttaai_core::models::download(&id, move |fraction| {
            let _ = progress_app.emit(
                "model-progress",
                serde_json::json!({ "id": progress_id, "fraction": fraction }),
            );
        });
        match result {
            Ok(path) => {
                let _ = app.emit(
                    "model-done",
                    serde_json::json!({ "id": id, "path": path.to_string_lossy() }),
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "model-error",
                    serde_json::json!({ "id": id, "error": e.to_string() }),
                );
            }
        }
    });
}

// ---- Ollama (Chat AI) -------------------------------------------------------

#[tauri::command]
async fn ollama_models() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = Settings::load_or_default();
        orttaai_core::llm::list_models(&settings.ollama_endpoint).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn ollama_chat(prompt: String, model: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = Settings::load_or_default();
        let model = if model.trim().is_empty() {
            "llama3.2".to_string()
        } else {
            model
        };
        orttaai_core::llm::generate(&settings.ollama_endpoint, &model, &prompt)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Analytics + Personal Memory --------------------------------------------

#[tauri::command]
fn dashboard_stats() -> Result<orttaai_core::store::DashboardStats, String> {
    Store::open_default()
        .and_then(|s| s.stats())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_memory() -> Result<Vec<orttaai_core::store::MemoryEntry>, String> {
    Store::open_default()
        .and_then(|s| s.list_memory())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn add_memory(kind: String, trigger: String, replacement: String) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    store
        .add_memory(&kind, &trigger, &replacement)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_memory(id: i64) -> Result<(), String> {
    Store::open_default()
        .and_then(|s| s.delete_memory(id))
        .map_err(|e| e.to_string())
}

// ---- Read-only commands -----------------------------------------------------

#[derive(Serialize)]
struct AppInfo {
    name: String,
    version: String,
    platform: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Orttaai".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
    }
}

#[derive(Serialize)]
struct SettingsDto {
    model_id: String,
    push_to_talk: String,
    preserve_clipboard: bool,
    low_latency: bool,
    ollama_endpoint: String,
}

#[tauri::command]
fn get_settings() -> SettingsDto {
    let settings = Settings::load_or_default();
    SettingsDto {
        model_id: settings.model_id,
        push_to_talk: format_combo(&settings.push_to_talk),
        preserve_clipboard: settings.preserve_clipboard,
        low_latency: settings.low_latency,
        ollama_endpoint: settings.ollama_endpoint,
    }
}

fn format_combo(combo: &HotkeyCombo) -> String {
    let mut parts: Vec<String> = combo
        .modifiers
        .iter()
        .map(|m| {
            match m {
                Modifier::Ctrl => "Ctrl",
                Modifier::Shift => "Shift",
                Modifier::Alt => "Alt",
                Modifier::Meta => "Meta",
            }
            .to_string()
        })
        .collect();
    parts.push(combo.key.clone());
    parts.join("+")
}

#[derive(Serialize)]
struct HistoryItem {
    id: i64,
    text: String,
    app: Option<String>,
    word_count: i64,
    created_at: i64,
}

#[tauri::command]
fn recent_history(limit: i64) -> Result<Vec<HistoryItem>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let records = store
        .recent(limit.clamp(1, 500))
        .map_err(|e| e.to_string())?;
    Ok(records
        .into_iter()
        .map(|r| HistoryItem {
            id: r.id.unwrap_or(0),
            text: r.text,
            app: r.app,
            word_count: r.word_count,
            created_at: r.created_at,
        })
        .collect())
}

// ---- Tray + app bootstrap ---------------------------------------------------

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Orttaai", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Orttaai")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(EngineState::default())
        .setup(|app| {
            build_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            get_settings,
            set_settings,
            list_models,
            download_model,
            recent_history,
            engine_status,
            start_dictation,
            stop_dictation,
            ollama_models,
            ollama_chat,
            dashboard_stats,
            list_memory,
            add_memory,
            delete_memory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
