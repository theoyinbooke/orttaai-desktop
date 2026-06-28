//! Orttaai desktop shell — a thin Tauri layer over `orttaai-core`.
//!
//! The UI is disposable; all real logic lives in the core. Read-only commands
//! expose settings + history; the engine commands drive the real dictation loop
//! (whisper + cpal + injection behind a global hotkey) and emit live state events.

use orttaai_core::audio::CpalAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::hotkey::{default_manager, HotkeyCallback, HotkeyManager};
use orttaai_core::injection::SystemTextInjector;
use orttaai_core::settings::Settings;
use orttaai_core::store::{Store, TranscriptionRecord};
use orttaai_core::transcription::WhisperTranscriber;
use orttaai_core::types::{DecodeOptions, HotkeyCombo, Modifier, RecordingState};
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

/// A dictation action queued to the worker thread. Sent from the hotkey event
/// loop and the tray/IPC toggle so the multi-second whisper decode never runs on
/// (and blocks) those threads.
enum DictationCmd {
    Press,
    Release,
    Toggle,
}

#[derive(Default)]
struct EngineState {
    hotkey: Mutex<Option<Box<dyn HotkeyManager>>>,
    /// Queue to the single dictation worker thread (which owns the coordinator
    /// and runs the slow decode). `None` when the engine is stopped.
    commands: Mutex<Option<std::sync::mpsc::Sender<DictationCmd>>>,
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
    let mut coordinator = DictationCoordinator::new(
        Box::new(transcriber),
        Box::new(CpalAudioCapture::new()),
        Box::new(SystemTextInjector::new()),
        memory,
        DecodeOptions::default(),
    );
    // Refuse to type into fields we can't confirm are non-secure (password
    // boxes) when the user opts in. On Linux/Wayland field status is always
    // Unknown, so this is the only secure-field guard available there.
    coordinator.set_strict_secure(settings.strict_secure);
    let coordinator = Arc::new(Mutex::new(coordinator));

    // One worker thread owns the decode so it never blocks the hotkey event loop
    // (which must stay free to deliver the next press / handle stop) or an IPC
    // handler. The hotkey and tray/IPC callbacks just enqueue commands, which the
    // worker runs in order.
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<DictationCmd>();
    {
        let app = app.clone();
        let coordinator = coordinator.clone();
        std::thread::Builder::new()
            .name("orttaai-dictation".into())
            .spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        DictationCmd::Press => do_press(&app, &coordinator),
                        DictationCmd::Release => do_release(&app, &coordinator),
                        DictationCmd::Toggle => {
                            let recording = matches!(
                                coordinator.lock().unwrap().state(),
                                RecordingState::Recording
                            );
                            if recording {
                                do_release(&app, &coordinator);
                            } else {
                                do_press(&app, &coordinator);
                            }
                        }
                    }
                }
            })
            .map_err(|e| format!("spawn dictation worker: {e}"))?;
    }
    *state.commands.lock().unwrap() = Some(cmd_tx.clone());

    let on_down: HotkeyCallback = {
        let tx = cmd_tx.clone();
        Box::new(move || {
            let _ = tx.send(DictationCmd::Press);
        })
    };
    let on_up: HotkeyCallback = {
        let tx = cmd_tx;
        Box::new(move || {
            let _ = tx.send(DictationCmd::Release);
        })
    };

    // Try to grab the global hotkey. On Wayland (and some WMs) this can't
    // register — or registers but never delivers events — so DON'T fail the
    // whole engine: keep running and let the user dictate with the on-screen /
    // tray "Record" toggle instead.
    let mut hotkey = default_manager();
    match hotkey.register(settings.push_to_talk, on_down, on_up) {
        Ok(()) => {
            *state.hotkey.lock().unwrap() = Some(hotkey);
            if is_wayland() {
                let _ = app.emit(
                    "engine-warning",
                    "On Wayland your shortcut is a toggle: with your target app focused, press it once to start, then press it again to stop and insert. (Configure it in Settings → Keyboard.)",
                );
            }
        }
        Err(e) => {
            let _ = app.emit(
                "engine-warning",
                format!(
                    "Global push-to-talk is unavailable ({e}). Use the “Click to record” button (or the tray) to dictate."
                ),
            );
        }
    }
    state.running.store(true, Ordering::SeqCst);
    emit_state(&app, "idle");
    Ok(())
}

/// Start recording on the shared coordinator and reflect it in the UI. Safe to
/// call from the hotkey thread or an IPC command.
fn do_press(app: &AppHandle, coordinator: &Arc<Mutex<DictationCoordinator>>) {
    let mut coord = coordinator.lock().unwrap();
    match coord.on_press() {
        Ok(()) if coord.state() == RecordingState::Recording => {
            drop(coord);
            emit_state(app, "recording");
            spawn_level_meter(app.clone(), coordinator.clone());
        }
        Ok(()) => {} // already recording — ignore the repeat press
        Err(e) => {
            drop(coord);
            let _ = app.emit("engine-error", e.to_string());
        }
    }
}

/// While recording, emit `audio-level` (0.0..=1.0) ~12×/s so the UI can show a
/// live mic meter — the user's proof that the right input source is captured.
fn spawn_level_meter(app: AppHandle, coordinator: Arc<Mutex<DictationCoordinator>>) {
    std::thread::spawn(move || {
        loop {
            let (recording, level) = {
                let c = coordinator.lock().unwrap();
                (c.state() == RecordingState::Recording, c.level())
            };
            if !recording {
                break;
            }
            let _ = app.emit("audio-level", level);
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
        let _ = app.emit("audio-level", 0.0_f32);
    });
}

/// Stop recording, transcribe, inject, and persist the transcript. Blocks for
/// the duration of the decode, so it always runs on the dictation worker thread
/// (never the hotkey event loop or an IPC handler).
fn do_release(app: &AppHandle, coordinator: &Arc<Mutex<DictationCoordinator>>) {
    emit_state(app, "processing");
    let outcome = coordinator.lock().unwrap().on_release();
    match outcome {
        Ok(o) => {
            eprintln!(
                "orttaai: dictation done — has_transcript={}, inject_error={:?}",
                o.transcript.is_some(),
                o.inject_error
            );
            if let Some(text) = o.transcript {
                persist_transcript(&text, o.duration_ms);
                let _ = app.emit("transcript", text.clone());
                let _ = app.emit("history-changed", ());
                if let Some(err) = o.inject_error {
                    // Typing failed (common on GNOME/Wayland). Don't lose the
                    // transcript: it is saved to History; also copy it so the
                    // user can paste it manually.
                    copy_to_clipboard(&text);
                    let _ = app.emit(
                        "engine-warning",
                        format!("Couldn't type into the focused app ({err}). Saved to History and copied to the clipboard — press Ctrl+V to paste."),
                    );
                }
            }
        }
        Err(e) => {
            let _ = app.emit("engine-error", e.to_string());
        }
    }
    emit_state(app, "idle");
}

/// Best-effort clipboard copy so a transcript is recoverable when injection
/// fails (common on GNOME/Wayland). Tries `wl-copy` (Wayland) then `xclip`/`xsel`.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let candidates: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (bin, args) in candidates {
        if let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            // wl-copy forks to serve the selection; don't block waiting on it.
            return;
        }
    }
}

/// Whether we're in a Wayland session (where the global hotkey is unreliable).
fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

/// Best-effort write of a completed dictation to the history store so Home and
/// History actually populate.
fn persist_transcript(text: &str, duration_ms: i64) {
    match Store::open_default() {
        Ok(store) => {
            let record = TranscriptionRecord::new(text, active_app(), duration_ms, now_unix());
            if let Err(e) = store.insert_transcription(&record) {
                eprintln!("orttaai: failed to persist transcription: {e}");
            }
        }
        Err(e) => eprintln!("orttaai: failed to open history store: {e}"),
    }
}

/// Current Unix time in seconds.
fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Name of the foreground application, when known. Real per-platform detection
/// (X11 `_NET_ACTIVE_WINDOW`, Windows `GetForegroundWindow`) is a follow-up;
/// Wayland has no portable API, so this stays `None` there.
fn active_app() -> Option<String> {
    None
}

#[tauri::command]
fn stop_dictation(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
    if let Some(mut hotkey) = state.hotkey.lock().unwrap().take() {
        hotkey.unregister().map_err(|e| e.to_string())?;
    }
    // Dropping the sender closes the channel so the worker thread exits after it
    // finishes any in-flight decode.
    *state.commands.lock().unwrap() = None;
    state.running.store(false, Ordering::SeqCst);
    emit_state(&app, "off");
    Ok(())
}

/// Toggle recording via the dictation worker. Shared by the IPC command and the
/// tray menu so both work when the global hotkey doesn't (Wayland). Enqueuing
/// keeps the caller responsive — the worker runs the (blocking) decode.
fn toggle_recording_impl(app: &AppHandle, state: &EngineState) {
    match state.commands.lock().unwrap().as_ref() {
        Some(tx) => {
            let _ = tx.send(DictationCmd::Toggle);
        }
        None => {
            let _ = app.emit("engine-error", "Engine is not running — press Start first.");
        }
    }
}

/// Manual push-to-talk toggle for when the global hotkey is unavailable (Wayland)
/// or the user prefers clicking: the first call starts recording, the second
/// stops, transcribes, and injects.
#[tauri::command]
fn toggle_recording(app: AppHandle, state: State<EngineState>) {
    toggle_recording_impl(&app, state.inner());
}

/// Broadcast a state change to the frontend, reflect it in the tray tooltip, and
/// show/hide the floating recording panel.
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
    // NOTE: the floating panel window steals keyboard focus when shown on
    // Wayland, which routed the injected text to the wrong window. Keep it hidden
    // until it can be made non-focusable; recording state still shows via the
    // tray tooltip, the main-window badge, and the mic meter.
    let _ = app.get_webview_window("panel").map(|p| p.hide());
}

// ---- Settings + models ------------------------------------------------------

#[derive(serde::Deserialize)]
struct SettingsInput {
    model_id: String,
    push_to_talk: String,
    preserve_clipboard: bool,
    low_latency: bool,
    ollama_endpoint: String,
    strict_secure: bool,
}

#[tauri::command]
fn set_settings(input: SettingsInput) -> Result<(), String> {
    let mut settings = Settings::load_or_default();
    settings.model_id = input.model_id;
    settings.push_to_talk = HotkeyCombo::parse(&input.push_to_talk);
    settings.preserve_clipboard = input.preserve_clipboard;
    settings.low_latency = input.low_latency;
    settings.ollama_endpoint = input.ollama_endpoint;
    settings.strict_secure = input.strict_secure;
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
    strict_secure: bool,
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
        strict_secure: settings.strict_secure,
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
    let rec = MenuItem::with_id(app, "rec", "Start / stop recording", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &rec, &quit])?;

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
            "rec" => toggle_recording_impl(app, app.state::<EngineState>().inner()),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Park the floating panel near the bottom-center of the primary monitor.
fn position_panel(app: &tauri::App) {
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(Some(monitor)) = panel.primary_monitor() {
            let size = monitor.size();
            let x = (size.width as i32 - 220) / 2;
            let y = size.height as i32 - 160;
            let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .manage(EngineState::default())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            build_tray(app)?;
            position_panel(app);
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
            toggle_recording,
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
