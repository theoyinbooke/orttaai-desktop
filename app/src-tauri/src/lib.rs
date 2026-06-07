//! Orttaai desktop shell — a thin Tauri layer over `orttaai-core`.
//!
//! The UI is disposable; all real logic lives in the core. These commands expose
//! settings + history to the frontend; the dictation engine is wired in next.

use orttaai_core::settings::Settings;
use orttaai_core::store::Store;
use orttaai_core::types::{HotkeyCombo, Modifier};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

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

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Orttaai", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::new()
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
        .setup(|app| {
            build_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            get_settings,
            recent_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
