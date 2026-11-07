//! Orttaai desktop shell — a thin Tauri layer over `orttaai-core`.
//!
//! The UI is disposable; all real logic lives in the core. Commands are grouped
//! by feature area, one module each:
//! - `engine`   — dictation engine, hotkeys, overlay, models
//! - `memory`   — dictionary, snippets, learning suggestions
//! - `ai`       — Chat AI, polish, voice edit, writing insights, tone
//! - `semantic` — semantic memory / Memory Graph
//! - `insights` — Home, analytics, history, milestones, What's New
//! - `coach`    — Coach
//! - `settings_cmds` — settings read/patch, reset, clear history

mod ai;
mod coach;
mod engine;
mod insights;
mod memory;
mod persist;
mod pipeline;
mod semantic;
mod settings_cmds;

use engine::{EngineState, TRAY_ID};
use orttaai_core::settings::Settings;
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

/// Map any displayable error into the `String` errors Tauri commands return.
pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Called after every settings save with the previous and new values, so
/// areas can react (re-register hotkeys, reload the pipeline, toggle autostart…).
pub(crate) fn on_settings_changed(app: &AppHandle, old: &Settings, new: &Settings) {
    // == engine: live-apply dictation settings ==
    engine::on_settings_changed(app, old, new);
    // == semantic: index on enable / embedding-model change ==
    semantic::on_settings_changed(app, old, new);
    // == general: launch at login ==
}

#[derive(Serialize)]
struct AppInfo {
    name: String,
    version: String,
    platform: String,
    /// `x11 | wayland | windows | macos`.
    session: String,
    /// `cpu | cuda | vulkan` — which whisper backend this build links.
    gpu_backend: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    let session = if cfg!(target_os = "linux") {
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|s| s.eq_ignore_ascii_case("wayland"))
                .unwrap_or(false)
        {
            "wayland"
        } else {
            "x11"
        }
    } else {
        std::env::consts::OS
    };
    let gpu_backend = if cfg!(feature = "cuda") {
        "cuda"
    } else if cfg!(feature = "vulkan") {
        "vulkan"
    } else {
        "cpu"
    };
    AppInfo {
        name: "Orttaai".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        session: session.to_string(),
        gpu_backend: gpu_backend.to_string(),
    }
}

/// Bring the main window forward, optionally on a specific page.
pub(crate) fn show_main(app: &AppHandle, route: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(route) = route {
        use tauri::Emitter;
        let _ = app.emit("navigate", route);
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Orttaai — Off", false, None::<&str>)?;
    // The engine updates this line with its state ("Recording…", "Ready"…).
    app.manage(engine::TrayStatus(status.clone()));
    let rec = MenuItem::with_id(app, "rec", "Start / stop recording", true, None::<&str>)?;
    let home = MenuItem::with_id(app, "home", "Home", true, None::<&str>)?;
    let analytics = MenuItem::with_id(app, "analytics", "Analytics", true, None::<&str>)?;
    let setup = MenuItem::with_id(app, "setup", "Run Setup…", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "updates", "Check for Updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Orttaai", true, None::<&str>)?;
    let sep = || tauri::menu::PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &sep()?,
            &rec,
            &sep()?,
            &home,
            &analytics,
            &setup,
            &settings,
            &updates,
            &sep()?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Orttaai")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "home" => show_main(app, Some("home")),
            "analytics" => show_main(app, Some("analytics")),
            "setup" => show_main(app, Some("setup")),
            "settings" => show_main(app, Some("settings")),
            "updates" => show_main(app, Some("check-updates")),
            "rec" => engine::toggle_recording_impl(app, app.state::<EngineState>().inner()),
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
        // Must be first: a second launch focuses the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app, None);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(EngineState::default())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            build_tray(app)?;
            engine::position_panel(app);
            engine::on_app_ready(app.handle());
            // Launched at login → stay in the tray.
            if std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window keeps Orttaai running in the tray (like the
            // Mac menu-bar app); Quit lives in the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // == shell ==
            app_info,
            settings_cmds::get_settings,
            settings_cmds::update_settings,
            settings_cmds::reset_settings,
            settings_cmds::clear_history,
            settings_cmds::reset_app_data,
            // == engine ==
            engine::engine_status,
            engine::start_dictation,
            engine::stop_dictation,
            engine::toggle_recording,
            engine::stop_recording,
            engine::suspend_hotkeys,
            engine::validate_shortcut,
            engine::list_audio_devices,
            engine::start_level_monitor,
            engine::stop_level_monitor,
            engine::reset_audio,
            engine::hardware_info,
            engine::platform_check,
            engine::list_models,
            engine::models_storage,
            engine::set_models_dir,
            engine::download_model,
            engine::cancel_download,
            engine::delete_model,
            engine::use_model,
            engine::verify_model,
            engine::vad_status,
            // == memory ==
            memory::list_dictionary,
            memory::save_dictionary,
            memory::delete_dictionary,
            memory::list_snippets,
            memory::save_snippet,
            memory::delete_snippet,
            memory::set_dictionary_active,
            memory::set_snippet_active,
            memory::list_suggestions,
            memory::accept_suggestion,
            memory::dismiss_suggestion,
            memory::list_edit_checks,
            memory::analyze_history,
            memory::export_memory,
            memory::import_memory,
            memory::preview_text_pipeline,
            // == ai ==
            ai::ai_status,
            ai::ai_test_connection,
            ai::ai_models,
            ai::ai_catalog,
            ai::ai_pull_model,
            ai::ai_warm_models,
            ai::s1_mini_status,
            ai::s1_mini_install,
            ai::ai_polish_test,
            ai::ai_rewrite_selection,
            ai::chat_conversations,
            ai::chat_messages,
            ai::chat_rename,
            ai::chat_delete,
            ai::chat_models,
            ai::chat_info,
            ai::chat_send,
            ai::chat_stop,
            ai::writing_insights_info,
            ai::writing_insights_generate,
            ai::writing_insights_quick,
            ai::writing_insights_latest,
            ai::writing_insights_history,
            ai::writing_insights_get,
            ai::writing_insights_freshness,
            ai::writing_insights_compare,
            ai::writing_insights_apps,
            ai::writing_apply_recommendation,
            ai::insight_snapshot_pin,
            ai::insight_snapshot_delete,
            ai::tone_analyze,
            ai::tone_latest,
            ai::tone_history,
            ai::tone_get,
            ai::tone_sample_count,
            // == semantic ==
            semantic::semantic_status,
            semantic::semantic_llm_status,
            semantic::semantic_search,
            semantic::semantic_index_now,
            semantic::semantic_clear_index,
            semantic::semantic_rebuild_graph,
            semantic::semantic_concept_graph,
            semantic::semantic_concept_detail,
            semantic::semantic_concept_brief,
            semantic::semantic_activity,
            semantic::semantic_activity_start,
            semantic::semantic_activity_stop,
            semantic::semantic_insight_report,
            semantic::semantic_generate_insights,
            semantic::semantic_findings,
            semantic::semantic_open_loops,
            semantic::semantic_set_finding_status,
            semantic::semantic_signals,
            semantic::semantic_home_insights,
            // == insights ==
            insights::dashboard_stats,
            insights::delete_transcription,
            insights::recent_history,
            insights::home_summary,
            insights::analytics_dashboard,
            insights::github_prompt_check,
            insights::history_page,
            insights::history_apps,
            insights::get_transcription,
            insights::copy_transcription,
            insights::milestones_overview,
            insights::milestones_pending,
            insights::milestones_evaluate,
            insights::milestones_mark_celebrated,
            insights::whats_new_pending,
            insights::whats_new_mark_seen,
            insights::save_share_card,
            insights::copy_share_card,
            insights::reveal_share_card,
            // == coach ==
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
