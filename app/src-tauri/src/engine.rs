//! The dictation engine: owns the coordinator worker thread, the global hotkey,
//! the floating overlay, and model management. (Dictation/engine area.)

use crate::persist;
use orttaai_core::audio::CpalAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::hotkey::{default_manager, HotkeyCallback, HotkeyManager};
use orttaai_core::injection::SystemTextInjector;
use orttaai_core::settings::Settings;
#[allow(unused_imports)]
use orttaai_core::store::Store as _StoreForAreas;
use orttaai_core::store::Store;
use orttaai_core::transcription::WhisperTranscriber;
use orttaai_core::types::RecordingState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub const TRAY_ID: &str = "main";

// ---- Engine state (Tauri-managed) ------------------------------------------

/// A dictation action queued to the worker thread. Sent from the hotkey event
/// loop and the tray/IPC toggle so the multi-second whisper decode never runs on
/// (and blocks) those threads.
pub enum DictationCmd {
    Press,
    Release,
    Toggle,
}

#[derive(Default)]
pub struct EngineState {
    hotkey: Mutex<Option<Box<dyn HotkeyManager>>>,
    /// Queue to the single dictation worker thread (which owns the coordinator
    /// and runs the slow decode). `None` when the engine is stopped.
    commands: Mutex<Option<std::sync::mpsc::Sender<DictationCmd>>>,
    /// The app focused when recording started — recorded as the dictation's
    /// source. `None` on Wayland, where the focused app can't be detected.
    source_app: Mutex<Option<String>>,
    running: AtomicBool,
    /// Set for the duration of `start_dictation` while the (slow) model load runs
    /// on a background thread — guards against overlapping starts from a double
    /// click or the tray firing while a start is already in flight.
    starting: AtomicBool,
}

#[derive(Serialize)]
pub struct EngineStatus {
    running: bool,
}

#[tauri::command]
pub fn engine_status(state: State<EngineState>) -> EngineStatus {
    EngineStatus {
        running: state.running.load(Ordering::SeqCst),
    }
}

/// Start the dictation engine. Loading the whisper model takes seconds for the
/// larger models, so the whole setup runs on a blocking background thread via
/// `spawn_blocking` — the WebView's main thread is never blocked, so the window
/// never goes "Not Responding". Emits `engine-state` "loading" while the model
/// loads, then "idle" (ready) or "off" (failed/rolled back).
#[tauri::command]
pub async fn start_dictation(app: AppHandle, model_path: String) -> Result<(), String> {
    {
        let state = app.state::<EngineState>();
        // No-op if already running; the `starting` swap also rejects a second
        // start fired while the first is still loading the model.
        if state.running.load(Ordering::SeqCst) || state.starting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
    }
    emit_state(&app, "loading");

    let worker_app = app.clone();
    let outcome =
        tauri::async_runtime::spawn_blocking(move || start_engine(&worker_app, model_path))
            .await
            .unwrap_or_else(|e| Err(format!("engine start task panicked: {e}")));

    app.state::<EngineState>()
        .starting
        .store(false, Ordering::SeqCst);
    if outcome.is_err() {
        // Roll the UI back to "off" so the Start button returns and the user can
        // fix the problem (e.g. download the model) and retry.
        emit_state(&app, "off");
    }
    outcome
}

/// Build and start the engine on the calling (background) thread: load the model,
/// spin up the dictation worker, and register the global hotkey. Blocking — only
/// ever called from `spawn_blocking`.
fn start_engine(app: &AppHandle, model_path: String) -> Result<(), String> {
    let state = app.state::<EngineState>();
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
        // Decode preset + thread count come from settings (the Performance panel).
        settings.decode.clone(),
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
                    "On Wayland, hold your shortcut to record and release to insert. If pressing it does nothing, open GNOME Settings → Keyboard and assign a key to Orttaai's “push-to-talk” shortcut.",
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
    emit_state(app, "idle");
    Ok(())
}

/// Start recording on the shared coordinator and reflect it in the UI. Safe to
/// call from the hotkey thread or an IPC command.
fn do_press(app: &AppHandle, coordinator: &Arc<Mutex<DictationCoordinator>>) {
    let mut coord = coordinator.lock().unwrap();
    match coord.on_press() {
        Ok(()) if coord.state() == RecordingState::Recording => {
            drop(coord);
            // Capture the focused app now (recording start) as this dictation's
            // source. `None` on Wayland, where focus can't be detected.
            *app.state::<EngineState>().source_app.lock().unwrap() =
                orttaai_core::focus::focused_app();
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
                let source = app
                    .state::<EngineState>()
                    .source_app
                    .lock()
                    .unwrap()
                    .clone();
                persist::save_transcript(&text, o.duration_ms, source);
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

#[tauri::command]
pub fn stop_dictation(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
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
pub fn toggle_recording_impl(app: &AppHandle, state: &EngineState) {
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
pub fn toggle_recording(app: AppHandle, state: State<EngineState>) {
    toggle_recording_impl(&app, state.inner());
}

/// Broadcast a state change to the frontend, reflect it in the tray tooltip, and
/// show/hide the floating recording panel.
pub fn emit_state(app: &AppHandle, state: &str) {
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
    // Floating recording overlay — shown ONLY while recording (key held), and
    // hidden the instant the key is released (i.e. before "processing", which is
    // when transcription + INJECTION run). This is deliberate: even with
    // `focusable:false`, GNOME/Mutter does not reliably keep an always-on-top
    // window out of the focus chain, so a panel left visible during injection
    // can capture the synthesized keystrokes. Hiding it on release returns focus
    // to the user's target app before any text is typed. (`engine-state` is the
    // single source of truth; every non-"recording" state hides it, so it can't
    // get stuck on-screen.)
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = if state == "recording" {
            panel.show()
        } else {
            panel.hide()
        };
    }
}

// ---- Models -----------------------------------------------------------------

#[tauri::command]
pub fn list_models() -> Result<Vec<orttaai_core::models::ModelInfo>, String> {
    orttaai_core::models::list().map_err(|e| e.to_string())
}

/// Delete a downloaded model file to reclaim disk. The active model can be
/// deleted too; the engine keeps any already-loaded model until it's restarted.
#[tauri::command]
pub fn delete_model(id: String) -> Result<(), String> {
    orttaai_core::models::delete(&id).map_err(|e| e.to_string())
}

/// Download a model in the background, emitting `model-progress`/`model-done`/
/// `model-error` events so the UI can show a progress bar without blocking.
#[tauri::command]
pub fn download_model(app: AppHandle, id: String) {
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

/// Park the floating panel near the bottom-center of the primary monitor.
/// (No click-through: `set_ignore_cursor_events` on the not-yet-realized hidden
/// window panics inside tao's GTK event loop, and `focusable:false` alone already
/// stops it stealing keyboard focus — the property that matters for injection.)
pub fn position_panel(app: &tauri::App) {
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(Some(monitor)) = panel.primary_monitor() {
            let size = monitor.size();
            let x = (size.width as i32 - 220) / 2;
            let y = size.height as i32 - 160;
            let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

/// React to a settings save (re-register hotkeys, reload pipeline options…).
pub fn on_settings_changed(_app: &AppHandle, _old: &Settings, _new: &Settings) {}

/// App finished launching: optionally auto-start the engine.
pub fn on_app_ready(_app: &AppHandle) {}
