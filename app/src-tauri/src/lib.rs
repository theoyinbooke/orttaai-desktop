//! Orttaai desktop shell — a thin Tauri layer over `orttaai-core`.
//!
//! The UI is disposable; all real logic lives in the core. Read-only commands
//! expose settings + history; the engine commands drive the real dictation loop
//! (whisper + cpal + injection behind a global hotkey) and emit live state events.

use orttaai_core::audio::CpalAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::hotkey::{HotkeyCallback, HotkeyManager, SystemHotkeyManager};
use orttaai_core::injection::SystemTextInjector;
use orttaai_core::memory::MemoryService;
use orttaai_core::settings::Settings;
use orttaai_core::store::Store;
use orttaai_core::transcription::WhisperTranscriber;
use orttaai_core::types::{DecodeOptions, HotkeyCombo, Modifier};
use serde::Serialize;
use std::path::Path;
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

    let transcriber =
        WhisperTranscriber::from_path(Path::new(&model_path)).map_err(|e| e.to_string())?;
    let coordinator = DictationCoordinator::new(
        Box::new(transcriber),
        Box::new(CpalAudioCapture::new()),
        Box::new(SystemTextInjector::new()),
        MemoryService::new(),
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
            recent_history,
            engine_status,
            start_dictation,
            stop_dictation
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
