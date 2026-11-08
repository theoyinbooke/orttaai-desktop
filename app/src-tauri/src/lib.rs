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
mod express_cmds;
mod insights;
mod link;
mod memory;
mod persist;
mod pipeline;
mod semantic;
mod settings_cmds;

use engine::{EngineState, TRAY_ID};
use orttaai_core::settings::Settings;
use serde::Serialize;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
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
    // == tray: the Polish check mark follows the setting ==
    if old.polish_enabled != new.polish_enabled {
        if let Some(polish) = app.try_state::<TrayPolish>() {
            let _ = polish.0.set_checked(new.polish_enabled);
        }
    }
    // == general: launch at login ==
    if old.launch_at_login != new.launch_at_login {
        sync_launch_at_login(app, new);
    }
}

/// Make the OS autostart entry match `launch_at_login`.
fn sync_launch_at_login(app: &AppHandle, settings: &Settings) {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    let enabled = launcher.is_enabled().unwrap_or(false);
    // On, it's written every time: the entry holds the program's path, which
    // an update can change (the Linux binary is now `orttaai`, was `app`).
    let result = match (settings.launch_at_login, enabled) {
        (true, _) => launcher.enable(),
        (false, true) => launcher.disable(),
        (false, false) => Ok(()),
    };
    if let Err(e) = result {
        use tauri::Emitter;
        let _ = app.emit(
            "engine-warning",
            format!("Couldn't change Launch at Login: {e}"),
        );
    }
}

/// Learn from my edits: when the engine's watch ends, turn the user's fixes
/// into dictionary suggestions (text-pipeline area's learner).
fn wire_learn_from_edits(app: &AppHandle) {
    use orttaai_core::injection::edit_watch::{EditCheckOutcome as Watched, EditObservation};
    use orttaai_core::store::{EditObservation as Learn, Store};
    use orttaai_core::text::learning::EditCheckOutcome as Outcome;
    use tauri::Emitter;
    let handle = app.clone();
    engine::set_edit_observer(
        app,
        std::sync::Arc::new(move |obs: EditObservation| {
            let Ok(store) = Store::open_default() else {
                return;
            };
            let at = orttaai_core::store::now_unix();
            let logged = match (obs.outcome, obs.before, obs.after) {
                (Watched::Edited, Some(before), Some(after)) => {
                    let settings = Settings::load_or_default();
                    let english = settings.dictation_language.eq_ignore_ascii_case("en")
                        || settings.express_active();
                    store
                        .learn_from_edit(
                            &Learn {
                                inserted: obs.inserted,
                                before,
                                caret_utf16: None,
                                after,
                                app_name: obs.app.clone(),
                            },
                            english,
                        )
                        .map(|(_, learned)| learned > 0)
                        .unwrap_or(false)
                }
                (watched, _, _) => {
                    let outcome = match watched {
                        Watched::NoChange => Outcome::NoChange,
                        Watched::TextNotFound => Outcome::TextNotFound,
                        Watched::SecureField => Outcome::SecureField,
                        Watched::Edited | Watched::Unreadable => Outcome::Unreadable,
                    };
                    let _ = store.insert_edit_check(outcome, obs.app.as_deref(), 0, at);
                    false
                }
            };
            if logged {
                let _ = handle.emit("memory-changed", ());
            }
            let _ = handle.emit("edit-checks-changed", ());
        }),
    );
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

/// The first-run / "Run Setup" window: its own fixed 600 x 750 window, as on the
/// Mac, not a page inside the main window.
pub(crate) fn open_setup(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("index.html".into()))
        .title("Orttaai Setup")
        .inner_size(600.0, 750.0)
        .resizable(false)
        .maximizable(false)
        .center()
        .build()?;
    let _ = window.set_focus();
    Ok(())
}

/// Opens Setup from a place that can't build a window inline (menu events, the
/// second-instance callback).
fn open_setup_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = open_setup(&app) {
            eprintln!("orttaai: couldn't open setup: {e}");
        }
    });
}

#[tauri::command]
async fn open_setup_window(app: AppHandle) -> Result<(), String> {
    open_setup(&app).map_err(|e| e.to_string())
}

/// Setup is done: its window goes away and the workspace opens on Overview.
#[tauri::command]
async fn finish_setup(app: AppHandle) -> Result<(), String> {
    show_main(&app, Some("home"));
    if let Some(window) = app.get_webview_window("setup") {
        window.destroy().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Bring the main window forward, optionally on a specific page. Until setup
/// is finished that means the setup window.
pub(crate) fn show_main(app: &AppHandle, route: Option<&str>) {
    if route == Some("setup") {
        open_setup_later(app);
        return;
    }
    if !Settings::load_or_default().has_completed_setup {
        open_setup_later(app);
        return;
    }
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

/// The tray's "Polish" check item, kept in step with `Settings::polish_enabled`.
struct TrayPolish(CheckMenuItem<tauri::Wry>);

/// The tray's Polish item was clicked: flip `polish_enabled` and save it the
/// way the Settings page does, so every window and the engine follow. With no
/// polish model set up it still flips; the AI page explains what's missing.
fn toggle_polish_from_tray(app: &AppHandle) {
    let enabled = !Settings::load_or_default().polish_enabled;
    let saved = settings_cmds::update_settings(
        app.clone(),
        serde_json::json!({ "polish_enabled": enabled }),
    );
    if let Err(e) = saved {
        use tauri::Emitter;
        let _ = app.emit("engine-warning", format!("Couldn't change Polish: {e}"));
    }
    // The menu toggles its own check on click; put it back to the saved value
    // (also when nothing changed, or the save failed).
    if let Some(polish) = app.try_state::<TrayPolish>() {
        let _ = polish
            .0
            .set_checked(Settings::load_or_default().polish_enabled);
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Orttaai — Off", false, None::<&str>)?;
    // The engine updates this line with its state ("Recording…", "Ready"…).
    app.manage(engine::TrayStatus(status.clone()));
    let rec = MenuItem::with_id(
        app,
        "rec",
        engine::tray_dictate_label("off"),
        true,
        None::<&str>,
    )?;
    app.manage(engine::TrayDictate(rec.clone()));
    let polish = CheckMenuItem::with_id(
        app,
        "polish",
        "Polish",
        true,
        Settings::load_or_default().polish_enabled,
        None::<&str>,
    )?;
    app.manage(TrayPolish(polish.clone()));
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
            &polish,
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

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        // The engine swaps in the recording / processing / error variants.
        .icon(engine::tray_icon("off"))
        .tooltip("Orttaai")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "home" => show_main(app, Some("home")),
            "analytics" => show_main(app, Some("analytics")),
            "setup" => show_main(app, Some("setup")),
            "settings" => show_main(app, Some("settings")),
            "updates" => show_main(app, Some("check-updates")),
            "polish" => toggle_polish_from_tray(app),
            "rec" => dictate_from_tray(app),
            _ => {}
        });
    // Windows: the tray icon is the click-to-dictate handle (the menu is on
    // right-click). Linux trays only open the menu, whatever the click.
    #[cfg(target_os = "windows")]
    let tray = tray
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                dictate_from_tray(tray.app_handle());
            }
        });
    tray.build(app)?;
    orttaai_core::focus::track_foreground();
    Ok(())
}

/// Start or stop a dictation from the tray (a click on the icon on Windows, or
/// the menu item). The taskbar took focus for the click, so the window that
/// was in front comes back first and the words go there. A second click within
/// [`TRAY_DOUBLE_CLICK_MS`] (a double-click) is ignored, so it can't start and
/// at once stop.
fn dictate_from_tray(app: &AppHandle) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let last = LAST_CLICK_MS.swap(now, Ordering::SeqCst);
    if now.saturating_sub(last) < TRAY_DOUBLE_CLICK_MS {
        return;
    }
    let app = app.clone();
    // Waiting for the window to come back mustn't hold up the event loop.
    std::thread::spawn(move || {
        let state = app.state::<EngineState>();
        // With the engine off the reason shows in the main window, which may
        // be hidden: bring it up so it isn't missed.
        if !state.is_running() {
            show_main(&app, None);
        } else if !orttaai_core::focus::return_from_tray() {
            tracing::info!("no app window to return to after the tray click");
        }
        engine::toggle_recording_impl(&app, state.inner());
    });
}

/// Two tray clicks closer than this are one double-click.
const TRAY_DOUBLE_CLICK_MS: u64 = 400;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// The diagnostic log's first lines: what it holds, and what it never does.
const LOG_PRIVACY_NOTE: &str = "\
Orttaai diagnostic log. It never records what you dictate, your transcripts, \
dictionary, snippets, clipboard, selected text or anything else you type or say. \
It holds only technical events (engine start, model loading, audio device \
changes, timings and error messages) that help troubleshoot a crash or an error. \
Your home folder is written as ~, so file paths don't show your account name.\n";

/// The log file, with the user's home folder written as `~` so paths in it
/// don't carry the account name.
struct MaskedLog {
    file: std::fs::File,
    /// The home folder as it appears in log lines: plain, and with Windows
    /// backslashes escaped (paths printed with `{:?}`).
    home: Vec<String>,
}

impl std::io::Write for MaskedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(text) = std::str::from_utf8(buf) {
            if self.home.iter().any(|h| text.contains(h.as_str())) {
                let mut masked = text.to_string();
                for h in &self.home {
                    masked = masked.replace(h.as_str(), "~");
                }
                self.file.write_all(masked.as_bytes())?;
                return Ok(buf.len());
            }
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Off unless asked for: `ORTTAAI_LOG=info` (any tracing filter) writes the
/// engine's log to `orttaai.log` in Orttaai's data folder, for a bug report.
/// See [`LOG_PRIVACY_NOTE`] for what it may contain.
fn init_diagnostic_log() {
    let Ok(filter) = std::env::var("ORTTAAI_LOG") else {
        return;
    };
    let Ok(dir) = orttaai_core::store::Store::data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("orttaai.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    use std::io::Write;
    let _ = write!(file, "\n{LOG_PRIVACY_NOTE}");
    let home: Vec<String> = ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|h| h.trim_end_matches(['/', '\\']).to_string())
        .filter(|h| h.len() > 1)
        .flat_map(|h| {
            let escaped = h.replace('\\', "\\\\");
            // Longest first, so the escaped form is replaced before the plain one.
            [escaped, h]
        })
        .collect();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(MaskedLog { file, home }))
        .try_init();
}

pub fn run() {
    init_diagnostic_log();
    tauri::Builder::default()
        // Must be first: a second launch focuses the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app, None);
        }))
        .plugin(tauri_plugin_opener::init())
        // Native folder picker (Model Storage).
        .plugin(tauri_plugin_dialog::init())
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
            wire_learn_from_edits(app.handle());
            sync_launch_at_login(app.handle(), &Settings::load_or_default());
            engine::on_app_ready(app.handle());
            // Orttaai Link: sync with Orttaai on a Mac once linked.
            link::start(app.handle());
            // The main window starts hidden: a new install sees only the setup
            // window, and a launch at login stays in the tray.
            if !Settings::load_or_default().has_completed_setup {
                open_setup(app.handle())?;
            } else if !std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
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
                } else if window.label() == "setup"
                    && !Settings::load_or_default().has_completed_setup
                {
                    // Nothing else is open yet, and a tray isn't a given on Linux,
                    // so closing first-run setup quits; the next launch reopens it.
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // == shell ==
            app_info,
            open_setup_window,
            finish_setup,
            settings_cmds::get_settings,
            settings_cmds::open_microphone_settings,
            settings_cmds::express_in_use,
            settings_cmds::update_settings,
            settings_cmds::reset_settings,
            settings_cmds::clear_history,
            settings_cmds::reset_app_data,
            // == Orttaai Link ==
            link::link_status,
            link::link_discover,
            link::link_with_code,
            link::link_with_invite,
            link::link_unlink,
            link::link_sync_now,
            link::link_devices,
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
            express_cmds::express_models,
            express_cmds::express_download,
            express_cmds::word_boost_status,
            express_cmds::word_boost_download,
            express_cmds::express_cancel,
            express_cmds::express_delete,
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
            coach::coach_catalog,
            coach::coach_snapshot,
            coach::coach_judge_state,
            coach::coach_check_judges,
            coach::coach_start_habit,
            coach::coach_end_habit,
            coach::coach_set_app_rule,
            coach::coach_remove_app_rule,
            coach::coach_set_persona,
            coach::coach_month_report,
            coach::coach_begin_rep,
            coach::coach_next_rep,
            coach::coach_end_session,
            coach::coach_score_rep,
            coach::coach_practice_arm,
            coach::coach_practice_disarm,
            coach::coach_practice_discard,
            coach::coach_scene_begin,
            coach::coach_scene_warm,
            coach::coach_scene_reply,
            coach::coach_scene_mood,
            coach::coach_scene_end,
            coach::coach_models,
            coach::coach_compare_models,
            coach::coach_setup_status,
            coach::coach_run_setup,
            coach::coach_start_ollama,
            coach::coach_save_card,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod log_tests {
    use super::MaskedLog;
    use std::io::Write;

    #[test]
    fn the_home_folder_never_reaches_the_log() {
        let path = std::env::temp_dir().join(format!("orttaai-log-test-{}", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let home = r"C:\Users\Ada Lovelace".to_string();
        let mut log = MaskedLog {
            file,
            home: vec![home.replace('\\', r"\\"), home],
        };
        writeln!(
            log,
            r"loading model from 'C:\Users\Ada Lovelace\AppData\m.bin'"
        )
        .unwrap();
        writeln!(log, r#"open failed: "C:\\Users\\Ada Lovelace\\x""#).unwrap();
        writeln!(log, "engine started").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(!text.contains("Lovelace"), "{text}");
        assert!(
            text.contains(r"'~\AppData\m.bin'") && text.contains(r#""~\\x""#),
            "{text}"
        );
        assert!(text.contains("engine started"));
    }
}
