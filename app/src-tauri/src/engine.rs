//! The dictation engine: owns the coordinator worker thread, the global hotkeys,
//! the floating overlay, audio devices + level meter, hardware detection, and
//! model management. (Dictation/engine area.)
//!
//! One worker thread owns the [`DictationCoordinator`]: hotkeys, the tray and
//! IPC only enqueue [`Cmd`]s, so the multi-second decode never blocks them. The
//! worker ticks the coordinator ~20×/s (caps, countdown, hands-free silence
//! auto-stop, error dismissal), diffs its status into events, and runs the slow
//! finalize (decode → pipeline → inject) itself.
//!
//! Events: `engine-state` (phase string), `engine-status` ([`EngineSnapshot`]),
//! `audio-level`, `audio-monitor-level`, `sound-cue`, `transcript`,
//! `dictation-complete`, `history-changed`, `engine-warning`, `engine-error`,
//! `model-progress`, `model-done`, `model-error`, `model-cancelled`.

use crate::{persist, pipeline};
use orttaai_core::audio::{AudioCapture, CpalAudioCapture, LevelMonitor};
use orttaai_core::clipboard::SystemClipboard;
use orttaai_core::coordinator::{
    CoordinatorConfig, CoordinatorStatus, DictationCoordinator, DictationOutcome,
};
use orttaai_core::hardware::{self, HardwareInfo};
use orttaai_core::hotkey::{default_manager, HotkeyBinding, HotkeyCallback, HotkeyManager};
use orttaai_core::injection::edit_watch::{EditObserver, EditWatcher, SystemFieldReader};
use orttaai_core::injection::{InjectionService, SystemTextInjector, TextInjector};
use orttaai_core::models;
use orttaai_core::settings::{Settings, SpeechEngine};
use orttaai_core::store::{now_unix, Store};
use orttaai_core::transcription::{decode_options_from_settings, WhisperTranscriber};
use orttaai_core::types::{AudioDevice, HotkeyCombo, RecordingMode, RecordingState, SessionKind};
use orttaai_core::vad::{SileroVad, VoiceDetector};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

pub const TRAY_ID: &str = "main";
/// Worker tick: timers, level meter, status diffing.
const TICK: Duration = Duration::from_millis(50);
/// Overlay size (logical px) — matches `tauri.conf.json`.
const PANEL_W: f64 = 320.0;
const PANEL_H: f64 = 60.0;

/// The tray's status line (`status` menu item), updated with engine state.
pub struct TrayStatus(pub tauri::menu::MenuItem<tauri::Wry>);

// ---- Engine state (Tauri-managed) ------------------------------------------

/// Work queued to the dictation worker.
pub enum Cmd {
    HotkeyDown,
    HotkeyUp,
    EditDown,
    EditUp,
    /// Tray / click-to-record: start hands-free, or stop.
    Toggle,
    /// The overlay's stop button.
    Stop,
    Reconfigure(Box<Settings>),
    ReloadMemory,
    ResetAudio(Sender<Result<String, String>>),
    VoiceReady(Box<SileroVad>),
    Shutdown,
}

impl Cmd {
    /// Gestures are dropped while a dictation is being processed (the Mac
    /// ignores the shortcut while busy); control commands never are.
    fn is_gesture(&self) -> bool {
        matches!(
            self,
            Cmd::HotkeyDown | Cmd::HotkeyUp | Cmd::EditDown | Cmd::EditUp | Cmd::Toggle | Cmd::Stop
        )
    }
}

/// Everything the UI shows about the engine (`engine_status`, `engine-status`).
#[derive(Debug, Clone, Serialize, Default)]
pub struct EngineSnapshot {
    pub running: bool,
    /// `off | loading | idle | recording | processing | error`.
    pub phase: String,
    pub status: Option<CoordinatorStatus>,
    pub model_id: Option<String>,
    pub hotkey: String,
    pub edit_hotkey: Option<String>,
    pub hotkey_backend: Option<String>,
    pub hotkey_error: Option<String>,
    pub hotkeys_suspended: bool,
    pub injector: Option<String>,
    pub clipboard: Option<String>,
    /// Silero voice detector loaded (else the loudness fallback is used).
    pub voice_detector: bool,
    pub last_transcript: Option<String>,
    pub wayland: bool,
    pub portal_hotkeys: bool,
}

struct Monitor {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[derive(Default)]
pub struct EngineState {
    hotkey: Mutex<Option<Box<dyn HotkeyManager>>>,
    commands: Mutex<Option<Sender<Cmd>>>,
    running: AtomicBool,
    starting: AtomicBool,
    hotkeys_suspended: AtomicBool,
    snapshot: Mutex<EngineSnapshot>,
    monitor: Mutex<Option<Monitor>>,
    /// Pauses the settings level meter while dictation owns the microphone.
    monitor_paused: Arc<AtomicBool>,
    /// Learn-from-edits observer, wired by the text area via
    /// [`set_edit_observer`].
    edit_observer: Mutex<Option<EditObserver>>,
}

impl EngineState {
    fn send(&self, cmd: Cmd) -> bool {
        match self.commands.lock().unwrap().as_ref() {
            Some(tx) => tx.send(cmd).is_ok(),
            None => false,
        }
    }
}

/// Wire the learn-from-edits observer: called once per watched dictation with
/// the field text before and after the user's fixes (see
/// `orttaai_core::injection::edit_watch`). Only active when
/// `Settings::learn_from_edits` is on.
pub fn set_edit_observer(app: &AppHandle, observer: EditObserver) {
    *app.state::<EngineState>().edit_observer.lock().unwrap() = Some(observer);
}

fn is_wayland() -> bool {
    orttaai_core::injection::is_wayland_session()
}

fn update_snapshot(app: &AppHandle, f: impl FnOnce(&mut EngineSnapshot)) -> EngineSnapshot {
    let state = app.state::<EngineState>();
    let mut snap = state.snapshot.lock().unwrap();
    f(&mut snap);
    snap.running = state.running.load(Ordering::SeqCst);
    snap.hotkeys_suspended = state.hotkeys_suspended.load(Ordering::SeqCst);
    snap.clone()
}

fn publish_snapshot(app: &AppHandle, f: impl FnOnce(&mut EngineSnapshot)) {
    let snap = update_snapshot(app, f);
    let _ = app.emit("engine-status", &snap);
}

#[tauri::command]
pub fn engine_status(app: AppHandle) -> EngineSnapshot {
    let s = Settings::load_or_default();
    update_snapshot(&app, |snap| {
        if snap.phase.is_empty() {
            snap.phase = "off".into();
        }
        snap.hotkey = s.push_to_talk.display();
        snap.edit_hotkey = s.edit_commands_enabled.then(|| s.edit_shortcut.display());
        snap.wayland = is_wayland();
        snap.portal_hotkeys = orttaai_core::hotkey::uses_portal();
    })
}

// ---- Start / stop ------------------------------------------------------------

/// Start the dictation engine. Loading (and warming) the whisper model takes
/// seconds for the larger models, so setup runs on a blocking background thread
/// — the WebView's main thread is never blocked. Emits `engine-state` "loading"
/// while the model loads, then "idle" (ready) or "off" (failed/rolled back).
#[tauri::command]
pub async fn start_dictation(app: AppHandle, model_path: Option<String>) -> Result<(), String> {
    {
        let state = app.state::<EngineState>();
        if state.running.load(Ordering::SeqCst) || state.starting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
    }
    emit_state(&app, "loading");

    let worker_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        start_engine(&worker_app, model_path.filter(|p| !p.trim().is_empty()))
    })
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

/// The model file for `settings` (must be downloaded).
fn resolve_model(settings: &Settings, explicit: Option<String>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(PathBuf::from(p));
    }
    let path = models::local_path(&settings.model_id).map_err(|e| e.to_string())?;
    if !models::is_downloaded(&settings.model_id) {
        return Err(format!(
            "The speech model “{}” isn't downloaded yet — download it in Models.",
            settings.model_id
        ));
    }
    Ok(path)
}

/// Active dictionary targets + snippet triggers, most used first (Mac
/// `vocabularyBiasTerms`: an overflowing dictionary keeps the names the user
/// actually dictates).
fn vocabulary_terms(settings: &Settings) -> Vec<String> {
    if !settings.vocabulary_bias {
        return Vec::new();
    }
    let Ok(store) = Store::open_default() else {
        return Vec::new();
    };
    let mut terms: Vec<(i64, String)> = Vec::new();
    if settings.dictionary_enabled {
        for e in store.list_dictionary().unwrap_or_default() {
            if e.is_active {
                terms.push((e.usage_count, e.target));
            }
        }
    }
    if settings.snippets_enabled {
        for s in store.list_snippets().unwrap_or_default() {
            if s.is_active {
                terms.push((s.usage_count, s.trigger));
            }
        }
    }
    terms.sort_by_key(|t| std::cmp::Reverse(t.0));
    terms.into_iter().map(|(_, t)| t).collect()
}

/// Word Boost's words: the Memory dictionary's targets with the spellings
/// mapped to them, while Express and Word Boost are on (Mac
/// `sessionWordBoostTerms`).
fn word_boost_terms(settings: &Settings) -> Vec<orttaai_core::text::WordBoostTerm> {
    if !settings.express_active() || !settings.express_word_boost {
        return Vec::new();
    }
    Store::open_default()
        .and_then(|s| s.text_processor(settings))
        .map(|p| p.word_boost_terms())
        .unwrap_or_default()
}

fn load_voice_detector() -> Option<SileroVad> {
    if !models::vad_installed() {
        return None;
    }
    let path = models::vad_model_path().ok()?;
    match SileroVad::load(&path) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("orttaai: voice detector unavailable, using loudness: {e}");
            None
        }
    }
}

/// Build and start the engine on the calling (background) thread: load the
/// model, warm it, spin up the dictation worker, and register the hotkeys.
fn start_engine(app: &AppHandle, model_path: Option<String>) -> Result<(), String> {
    let state = app.state::<EngineState>();
    let settings = Settings::load_or_default();
    let ((transcriber, model_id), fallback) = load_transcriber(&settings, model_path)?;
    // After a fallback the rest of the engine (pipeline, language) is Whisper's.
    let settings = if let Some(why) = fallback {
        let _ = app.emit("engine-warning", why);
        Settings {
            engine: SpeechEngine::Whisper,
            ..settings
        }
    } else {
        settings
    };

    let injector = SystemTextInjector::new();
    let injector_name = injector.backend_name().to_string();
    let clipboard = SystemClipboard::new();
    let clipboard_name = clipboard.backend_name().to_string();
    let injection = InjectionService::new(Box::new(injector), Box::new(clipboard));

    let mut coord = DictationCoordinator::new(
        transcriber,
        Box::new(CpalAudioCapture::new()),
        injection,
        pipeline::build_pipeline(&settings),
        decode_options_from_settings(&settings),
    );
    coord.set_config(CoordinatorConfig::from_settings(&settings));
    coord.set_rewriter(pipeline::build_rewriter(&settings));
    coord.set_vocabulary(vocabulary_terms(&settings));
    coord.set_word_boost_terms(&word_boost_terms(&settings));
    let voice = load_voice_detector();
    let has_voice = voice.is_some();
    coord.set_voice_detector(voice.map(|v| Box::new(v) as _));

    if settings.warm_models {
        // Mac: a silent 2 s decode so the first dictation is fast.
        if let Err(e) = coord.warm_up() {
            eprintln!("orttaai: warm-up decode failed (harmless): {e}");
        }
    }

    let (tx, rx) = channel::<Cmd>();
    {
        let worker = Worker::new(app.clone(), coord, settings.clone());
        std::thread::Builder::new()
            .name("orttaai-dictation".into())
            .spawn(move || worker.run(rx))
            .map_err(|e| format!("spawn dictation worker: {e}"))?;
    }
    *state.commands.lock().unwrap() = Some(tx.clone());
    state.running.store(true, Ordering::SeqCst);

    publish_snapshot(app, |s| {
        s.model_id = Some(model_id);
        s.injector = Some(injector_name);
        s.clipboard = Some(clipboard_name);
        s.voice_detector = has_voice;
        s.wayland = is_wayland();
        s.portal_hotkeys = orttaai_core::hotkey::uses_portal();
    });
    register_hotkeys(app, &settings);

    // First hands-free use needs Silero; fetch it (≈0.9 MB, checksum-verified)
    // in the background. Until then auto-stop uses the loudness fallback.
    if !has_voice {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if models::download_vad(|_| {}).is_ok() {
                if let Some(v) = load_voice_detector() {
                    let _ = tx.send(Cmd::VoiceReady(Box::new(v)));
                }
            }
        });
    }
    emit_state(app, "idle");
    Ok(())
}

type LoadedTranscriber = (Box<dyn orttaai_core::transcription::Transcriber>, String);

/// Load the speech engine the settings ask for: whisper.cpp (GGUF) or Express
/// (Parakeet via sherpa-onnx). Returns the transcriber, the model id shown in
/// the UI and recorded with each dictation, and, when Express was asked for but
/// couldn't run, why dictation fell back to Whisper.
fn load_transcriber(
    settings: &Settings,
    explicit: Option<String>,
) -> Result<(LoadedTranscriber, Option<String>), String> {
    // Express chosen but dictation isn't English: Whisper, as on the Mac.
    if settings.express_active() && explicit.is_none() {
        match load_express(settings) {
            Ok(loaded) => return Ok((loaded, None)),
            // A dictation never fails because of Express: use Whisper and say so.
            Err(why) if models::is_downloaded(&settings.model_id) => {
                eprintln!("orttaai: Express could not start ({why}); dictating with Whisper");
                let whisper = load_whisper(settings, None)?;
                return Ok((
                    whisper,
                    Some(format!(
                        "Express couldn't start, so dictation uses Whisper. {why}"
                    )),
                ));
            }
            Err(why) => return Err(why),
        }
    }
    Ok((load_whisper(settings, explicit)?, None))
}

fn load_express(settings: &Settings) -> Result<LoadedTranscriber, String> {
    use orttaai_core::transcription::express;
    let id = settings.express_model.trim();
    if !express::is_installed(id) {
        return Err(format!(
            "The Express model “{id}” isn't downloaded yet. Download it in Model, or switch back to Whisper."
        ));
    }
    let dir = express::model_dir(id).map_err(|e| e.to_string())?;
    let t =
        orttaai_core::transcription::ExpressTranscriber::from_dir(&dir, settings.decode.n_threads)
            .map_err(|e| e.to_string())?;
    Ok((Box::new(t), id.to_string()))
}

fn load_whisper(
    settings: &Settings,
    explicit: Option<String>,
) -> Result<LoadedTranscriber, String> {
    let path = resolve_model(settings, explicit)?;
    let t = WhisperTranscriber::from_path(&path).map_err(|e| e.to_string())?;
    Ok((Box::new(t), transcriber_id(&path)))
}

fn transcriber_id(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string()
}

/// (Re-)register the global shortcuts: push-to-talk, plus edit-selection when
/// voice edit is on. Failure never stops the engine — the user can still
/// dictate with click-to-record or the tray.
fn register_hotkeys(app: &AppHandle, settings: &Settings) {
    let state = app.state::<EngineState>();
    if let Some(mut old) = state.hotkey.lock().unwrap().take() {
        let _ = old.unregister();
    }
    if state.hotkeys_suspended.load(Ordering::SeqCst) {
        return;
    }
    let Some(tx) = state.commands.lock().unwrap().clone() else {
        return;
    };
    let cb = |cmd: fn() -> Cmd| -> HotkeyCallback {
        let tx = tx.clone();
        Box::new(move || {
            let _ = tx.send(cmd());
        })
    };
    let mut bindings = vec![HotkeyBinding::push_to_talk(
        settings.push_to_talk.clone(),
        cb(|| Cmd::HotkeyDown),
        cb(|| Cmd::HotkeyUp),
    )];
    if settings.edit_commands_enabled {
        bindings.push(HotkeyBinding::edit_selection(
            settings.edit_shortcut.clone(),
            cb(|| Cmd::EditDown),
            cb(|| Cmd::EditUp),
        ));
    }
    let mut manager = default_manager();
    let backend = manager.backend_name().to_string();
    match manager.register_all(bindings) {
        Ok(()) => {
            *state.hotkey.lock().unwrap() = Some(manager);
            publish_snapshot(app, |s| {
                s.hotkey = settings.push_to_talk.display();
                s.edit_hotkey = settings
                    .edit_commands_enabled
                    .then(|| settings.edit_shortcut.display());
                s.hotkey_backend = Some(backend);
                s.hotkey_error = None;
            });
            if is_wayland() {
                let _ = app.emit(
                    "engine-warning",
                    "On Wayland, hold your shortcut to record and release to insert. If pressing it does nothing, open your desktop's Settings → Keyboard and assign a key to Orttaai's “push-to-talk” shortcut.",
                );
            }
        }
        Err(e) => {
            let msg = format!(
                "Global shortcut unavailable ({e}). Use “Click to record” (or the tray) to dictate."
            );
            publish_snapshot(app, |s| {
                s.hotkey_backend = Some(backend);
                s.hotkey_error = Some(e.to_string());
            });
            let _ = app.emit("engine-warning", msg);
        }
    }
}

#[tauri::command]
pub fn stop_dictation(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
    if let Some(mut hotkey) = state.hotkey.lock().unwrap().take() {
        hotkey.unregister().map_err(|e| e.to_string())?;
    }
    // The worker exits after any in-flight decode.
    if let Some(tx) = state.commands.lock().unwrap().take() {
        let _ = tx.send(Cmd::Shutdown);
    }
    state.running.store(false, Ordering::SeqCst);
    emit_state(&app, "off");
    publish_snapshot(&app, |s| {
        s.status = None;
        s.model_id = None;
    });
    Ok(())
}

/// Toggle recording via the worker. Shared by the IPC command and the tray menu
/// so both work when the global hotkey doesn't (Wayland).
pub fn toggle_recording_impl(app: &AppHandle, state: &EngineState) {
    if !state.send(Cmd::Toggle) {
        let _ = app.emit("engine-error", "Engine is not running — press Start first.");
    }
}

/// Click-to-record: the first call starts a hands-free recording, the second
/// stops, transcribes, and inserts.
#[tauri::command]
pub fn toggle_recording(app: AppHandle, state: State<EngineState>) {
    toggle_recording_impl(&app, state.inner());
}

/// The overlay's stop button.
#[tauri::command]
pub fn stop_recording(state: State<EngineState>) {
    state.send(Cmd::Stop);
}

/// Pause global shortcuts while the shortcut recorder listens (so pressing the
/// current shortcut records it instead of dictating), and resume after.
#[tauri::command]
pub fn suspend_hotkeys(app: AppHandle, suspended: bool) {
    let state = app.state::<EngineState>();
    state.hotkeys_suspended.store(suspended, Ordering::SeqCst);
    if suspended {
        if let Some(mut h) = state.hotkey.lock().unwrap().take() {
            let _ = h.unregister();
        }
        publish_snapshot(&app, |_| {});
    } else if state.running.load(Ordering::SeqCst) {
        let settings = Settings::load_or_default();
        let app2 = app.clone();
        // The portal path can block on a dialog; keep it off the IPC thread.
        std::thread::spawn(move || register_hotkeys(&app2, &settings));
    } else {
        publish_snapshot(&app, |_| {});
    }
}

/// Validate a recorded shortcut before saving it.
#[tauri::command]
pub fn validate_shortcut(combo: HotkeyCombo, other: Option<HotkeyCombo>) -> Result<(), String> {
    orttaai_core::hotkey::validate_combo(&combo).map_err(|e| e.to_string())?;
    if other.is_some_and(|o| o.same_chord(&combo)) {
        return Err("That shortcut is already used by the other Orttaai shortcut.".into());
    }
    Ok(())
}

// ---- Worker ------------------------------------------------------------------

struct Worker {
    app: AppHandle,
    coord: DictationCoordinator,
    settings: Settings,
    last: Option<CoordinatorStatus>,
    watcher: EditWatcher,
}

impl Worker {
    fn new(app: AppHandle, coord: DictationCoordinator, settings: Settings) -> Self {
        let forward = {
            let app = app.clone();
            Arc::new(move |obs| {
                let observer = app
                    .state::<EngineState>()
                    .edit_observer
                    .lock()
                    .unwrap()
                    .clone();
                if let Some(o) = observer {
                    o(obs);
                }
            })
        };
        Self {
            app,
            coord,
            settings,
            last: None,
            watcher: EditWatcher::new(Arc::new(SystemFieldReader), forward),
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(cmd) => {
                    if !self.handle(cmd) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.coord.tick();
            self.publish();
            if self.coord.has_pending() {
                let result = self.coord.finish_pending();
                self.on_outcome(result);
                self.publish();
                // Shortcut presses queued while busy are ignored (Mac).
                let mut alive = true;
                while let Ok(cmd) = rx.try_recv() {
                    if !cmd.is_gesture() && !self.handle(cmd) {
                        alive = false;
                        break;
                    }
                }
                if !alive {
                    break;
                }
            }
        }
        self.coord.cancel();
        self.watcher.finish_current();
    }

    /// `false` = shut down.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::HotkeyDown => self.coord.handle_hotkey_down(),
            Cmd::HotkeyUp => self.coord.handle_hotkey_up(),
            Cmd::EditDown => self.coord.handle_edit_hotkey_down(),
            Cmd::EditUp => self.coord.handle_edit_hotkey_up(),
            Cmd::Toggle => self.coord.toggle(),
            Cmd::Stop => self
                .coord
                .stop_recording(orttaai_core::coordinator::StopReason::PillStop),
            Cmd::Reconfigure(s) => self.reconfigure(*s),
            Cmd::ReloadMemory => {
                self.coord
                    .set_pipeline(pipeline::build_pipeline(&self.settings));
                self.coord.set_vocabulary(vocabulary_terms(&self.settings));
                self.coord
                    .set_word_boost_terms(&word_boost_terms(&self.settings));
            }
            Cmd::ResetAudio(reply) => {
                let r = self
                    .coord
                    .reset_audio()
                    .map(|_| {
                        "Audio pipeline reset. Try the microphone level meter again.".to_string()
                    })
                    .map_err(|e| match e {
                        orttaai_core::CoreError::Audio(m) => m,
                        other => other.to_string(),
                    });
                let _ = reply.send(r);
            }
            Cmd::VoiceReady(v) => {
                self.coord
                    .set_voice_detector(Some(v as Box<dyn VoiceDetector>));
                publish_snapshot(&self.app, |s| s.voice_detector = true);
            }
            Cmd::Shutdown => return false,
        }
        true
    }

    fn reconfigure(&mut self, next: Settings) {
        let mut next = next;
        let model_changed = next.model_id != self.settings.model_id
            || next.models_dir != self.settings.models_dir
            || next.engine != self.settings.engine
            || next.express_model != self.settings.express_model
            || next.express_active() != self.settings.express_active();
        if model_changed && self.coord.state() == RecordingState::Idle {
            emit_state(&self.app, "loading");
            match load_transcriber(&next, None) {
                Ok(((t, id), fallback)) => {
                    if let Some(why) = fallback {
                        let _ = self.app.emit("engine-warning", why);
                        next.engine = SpeechEngine::Whisper;
                    }
                    self.coord.replace_transcriber(t);
                    if next.warm_models {
                        let _ = self.coord.warm_up();
                    }
                    publish_snapshot(&self.app, |s| s.model_id = Some(id));
                }
                Err(e) => {
                    let _ = self.app.emit(
                        "engine-error",
                        format!(
                            "Couldn't switch speech models: {e} Still using the previous model."
                        ),
                    );
                }
            }
            emit_state(&self.app, "idle");
        }
        self.coord
            .set_config(CoordinatorConfig::from_settings(&next));
        self.coord
            .set_decode_options(decode_options_from_settings(&next));
        self.coord.set_pipeline(pipeline::build_pipeline(&next));
        self.coord.set_rewriter(pipeline::build_rewriter(&next));
        self.coord.set_vocabulary(vocabulary_terms(&next));
        self.coord.set_word_boost_terms(&word_boost_terms(&next));
        self.settings = next;
    }

    /// Diff the coordinator status into events, cues, overlay and tray.
    fn publish(&mut self) {
        let st = self.coord.status();
        let recording = st.state == RecordingState::Recording;
        let was_recording = self
            .last
            .as_ref()
            .is_some_and(|l| l.state == RecordingState::Recording);
        if recording {
            let _ = self.app.emit("audio-level", self.coord.level());
        } else if was_recording {
            let _ = self.app.emit("audio-level", 0.0f32);
        }
        if self.last.as_ref() == Some(&st) {
            return;
        }
        let prev = self.last.take();
        if self.settings.sound_cues {
            if let Some(cue) = sound_cue(prev.as_ref(), &st) {
                let _ = self.app.emit("sound-cue", cue);
            }
        }
        if recording && !was_recording {
            // The next insertion would mix into the field being watched.
            self.watcher.finish_current();
        }
        let phase_changed =
            prev.as_ref().map(|p| (p.state, p.mode, p.kind)) != Some((st.state, st.mode, st.kind));
        if phase_changed {
            emit_phase(&self.app, &st);
        }
        let status = st.clone();
        publish_snapshot(&self.app, |s| s.status = Some(status));
        self.last = Some(st);
    }

    fn on_outcome(&mut self, result: orttaai_core::Result<DictationOutcome>) {
        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                let msg = self.coord.status().error.unwrap_or_else(|| e.to_string());
                let _ = self.app.emit("engine-error", msg);
                return;
            }
        };
        let Some(text) = outcome.transcript.clone() else {
            return;
        };
        let id = outcome
            .to_record(now_unix())
            .and_then(|r| persist::save_record(&r));
        if !outcome.dictionary_hits.is_empty() || !outcome.snippet_hits.is_empty() {
            if let Ok(store) = Store::open_default() {
                let _ = store.record_memory_usage(&outcome.dictionary_hits, &outcome.snippet_hits);
            }
        }
        let _ = self.app.emit("transcript", text.clone());
        let _ = self.app.emit("history-changed", ());
        let _ = self.app.emit(
            "dictation-complete",
            serde_json::json!({ "id": id, "outcome": &outcome }),
        );
        publish_snapshot(&self.app, |s| s.last_transcript = Some(text.clone()));

        let delivered = outcome.injection_method.is_some_and(|m| m.delivered());
        if delivered
            && outcome.kind == SessionKind::Dictation
            && self.settings.learn_from_edits
            && self
                .app
                .state::<EngineState>()
                .edit_observer
                .lock()
                .unwrap()
                .is_some()
        {
            self.watcher.watch(&text, outcome.app.clone());
        }
        if let Some(err) = &outcome.inject_error {
            let _ = self.app.emit(
                "engine-warning",
                format!(
                    "Couldn't type into the focused app ({err}). Saved to History and left on the clipboard — press Ctrl+V to paste."
                ),
            );
        }
    }
}

/// Which cue a status change plays (Mac: start / stop / error / hands-free).
fn sound_cue(prev: Option<&CoordinatorStatus>, now: &CoordinatorStatus) -> Option<&'static str> {
    let prev_state = prev.map(|p| p.state);
    match now.state {
        RecordingState::Recording if prev_state != Some(RecordingState::Recording) => Some("start"),
        RecordingState::Recording
            if prev.is_some_and(|p| p.mode == RecordingMode::PushToTalk)
                && now.mode == RecordingMode::HandsFree =>
        {
            Some("hands_free")
        }
        RecordingState::Processing if prev_state == Some(RecordingState::Recording) => Some("stop"),
        RecordingState::Error if prev_state != Some(RecordingState::Error) => Some("error"),
        _ => None,
    }
}

fn phase_name(state: RecordingState) -> &'static str {
    match state {
        RecordingState::Idle => "idle",
        RecordingState::Recording => "recording",
        RecordingState::Processing => "processing",
        RecordingState::Error => "error",
    }
}

fn emit_phase(app: &AppHandle, st: &CoordinatorStatus) {
    let line = match (st.state, st.kind, st.mode) {
        (RecordingState::Recording, SessionKind::Edit, _) => "Recording edit instruction…",
        (RecordingState::Recording, _, RecordingMode::HandsFree) => "Recording (hands-free)…",
        (RecordingState::Recording, _, _) => "Recording…",
        (RecordingState::Processing, _, _) => "Processing…",
        (RecordingState::Error, _, _) => "Error",
        (RecordingState::Idle, _, _) => "Ready",
    };
    apply_phase(app, phase_name(st.state), line);
}

/// Broadcast a phase, reflect it in the tray, and show/hide the overlay.
pub fn emit_state(app: &AppHandle, phase: &str) {
    let line = match phase {
        "loading" => "Loading model…",
        "idle" => "Ready",
        "recording" => "Recording…",
        "processing" => "Processing…",
        "error" => "Error",
        _ => "Off",
    };
    apply_phase(app, phase, line);
}

fn apply_phase(app: &AppHandle, phase: &str, line: &str) {
    let _ = app.emit("engine-state", phase);
    publish_snapshot(app, |s| s.phase = phase.to_string());
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(format!("Orttaai — {line}")));
    }
    if let Some(status) = app.try_state::<TrayStatus>() {
        let _ = status.0.set_text(format!("Orttaai — {line}"));
    }
    let state = app.state::<EngineState>();
    state.monitor_paused.store(
        phase == "recording" || phase == "processing",
        Ordering::SeqCst,
    );

    // Floating overlay. Always shown while recording. On Linux it is hidden the
    // instant recording ends — BEFORE "processing", which is when injection
    // runs: even with `focusable:false`, GNOME/Mutter does not reliably keep an
    // always-on-top window out of the focus chain, so a panel left visible
    // during injection can capture the synthesized keystrokes. Windows keeps a
    // `WS_EX_NOACTIVATE` window out of focus, so the pill stays up to show the
    // processing estimate. Errors (after any injection) show briefly
    // everywhere; every other phase hides it, so it can't get stuck on-screen.
    if let Some(panel) = app.get_webview_window("panel") {
        let show = match phase {
            "recording" | "error" => true,
            "processing" => !cfg!(target_os = "linux"),
            _ => false,
        };
        let _ = if show { panel.show() } else { panel.hide() };
    }
}

/// Park the floating panel near the bottom-center of the primary monitor.
/// (No click-through: `set_ignore_cursor_events` on the not-yet-realized hidden
/// window panics inside tao's GTK event loop, and `focusable:false` alone
/// already stops it stealing keyboard focus.)
pub fn position_panel(app: &tauri::App) {
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(Some(monitor)) = panel.primary_monitor() {
            let scale = monitor.scale_factor();
            let size = monitor.size();
            let origin = monitor.position();
            let w = (PANEL_W * scale) as i32;
            let h = (PANEL_H * scale) as i32;
            let x = origin.x + (size.width as i32 - w) / 2;
            let y = origin.y + size.height as i32 - h - (96.0 * scale) as i32;
            let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

/// React to a settings save: re-register shortcuts, apply dictation settings
/// live, reload the model, follow the audio device in the level meter.
pub fn on_settings_changed(app: &AppHandle, old: &Settings, new: &Settings) {
    let state = app.state::<EngineState>();
    let running = state.running.load(Ordering::SeqCst);
    if running {
        state.send(Cmd::Reconfigure(Box::new(new.clone())));
        let shortcuts_changed = old.push_to_talk != new.push_to_talk
            || old.edit_shortcut != new.edit_shortcut
            || old.edit_commands_enabled != new.edit_commands_enabled;
        if shortcuts_changed && !state.hotkeys_suspended.load(Ordering::SeqCst) {
            let app2 = app.clone();
            let settings = new.clone();
            std::thread::spawn(move || register_hotkeys(&app2, &settings));
        }
    } else {
        publish_snapshot(app, |s| {
            s.hotkey = new.push_to_talk.display();
            s.edit_hotkey = new
                .edit_commands_enabled
                .then(|| new.edit_shortcut.display());
        });
    }
    if old.audio_device != new.audio_device && state.monitor.lock().unwrap().is_some() {
        let _ = start_monitor(app, new.audio_device.clone());
    }
}

/// App finished launching: reload memory on `memory-changed`, and auto-start the
/// engine when `start_engine_on_launch` and the model is downloaded.
pub fn on_app_ready(app: &AppHandle) {
    {
        let app2 = app.clone();
        app.listen("memory-changed", move |_| {
            app2.state::<EngineState>().send(Cmd::ReloadMemory);
        });
    }
    let _ = update_snapshot(app, |s| s.phase = "off".into());
    let settings = Settings::load_or_default();
    // Ready to run when the Whisper model is on disk, or Express is the engine
    // and its model is (an Express-only install starts too).
    let ready = models::is_downloaded(&settings.model_id)
        || (settings.express_active()
            && orttaai_core::transcription::express::is_installed(settings.express_model.trim()));
    if settings.start_engine_on_launch && ready {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = start_dictation(app2.clone(), None).await {
                let _ = app2.emit("engine-error", format!("Couldn't start dictation: {e}"));
            }
        });
    }
}

// ---- Audio ---------------------------------------------------------------------

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDevice>, String> {
    orttaai_core::audio::list_input_devices().map_err(|e| e.to_string())
}

fn start_monitor(app: &AppHandle, device: Option<String>) -> Result<Option<String>, String> {
    let state = app.state::<EngineState>();
    drop(state.monitor.lock().unwrap().take());
    let stop = Arc::new(AtomicBool::new(false));
    let paused = state.monitor_paused.clone();
    let (ready_tx, ready_rx) = channel::<Result<Option<String>, String>>();
    let thread_stop = stop.clone();
    let app2 = app.clone();
    let thread = std::thread::Builder::new()
        .name("orttaai-meter".into())
        .spawn(move || {
            let mut monitor = match LevelMonitor::start(device.clone()) {
                Ok(m) => {
                    let _ = ready_tx.send(Ok(m.device().map(str::to_string)));
                    Some(m)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            while !thread_stop.load(Ordering::SeqCst) {
                // While dictation owns the mic the meter follows the recording
                // level instead (no second stream on the device).
                if paused.load(Ordering::SeqCst) {
                    monitor = None;
                } else if monitor.is_none() {
                    monitor = LevelMonitor::start(device.clone()).ok();
                }
                if let Some(m) = &monitor {
                    let _ = app2.emit("audio-monitor-level", m.level());
                }
                std::thread::sleep(Duration::from_millis(66));
            }
            let _ = app2.emit("audio-monitor-level", 0.0f32);
        })
        .map_err(|e| e.to_string())?;
    let name = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "The microphone didn't respond.".to_string())??;
    *state.monitor.lock().unwrap() = Some(Monitor {
        stop,
        thread: Some(thread),
    });
    Ok(name)
}

/// Live input level for Settings › Audio and the setup mic check: emits
/// `audio-monitor-level` until [`stop_level_monitor`]. Returns the device
/// actually metered.
#[tauri::command]
pub async fn start_level_monitor(
    app: AppHandle,
    device: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || start_monitor(&app, device))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn stop_level_monitor(state: State<EngineState>) {
    drop(state.monitor.lock().unwrap().take());
}

/// "Reset Audio" (Mac): restart capture so the next recording opens the device
/// fresh. Only while idle.
#[tauri::command]
pub async fn reset_audio(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<EngineState>();
        let had_monitor = state.monitor.lock().unwrap().take().is_some();
        let result = if state.running.load(Ordering::SeqCst) {
            let (tx, rx) = channel();
            if !state.send(Cmd::ResetAudio(tx)) {
                return Err("Audio service is unavailable.".to_string());
            }
            rx.recv_timeout(Duration::from_secs(10))
                .map_err(|_| "Audio reset timed out.".to_string())?
        } else {
            // Not dictating: a fresh capture probe is the reset.
            let mut probe = CpalAudioCapture::new();
            probe.reset().map_err(|e| e.to_string())?;
            Ok("Audio pipeline reset. Try the microphone level meter again.".to_string())
        };
        if had_monitor {
            let device = Settings::load_or_default().audio_device;
            let _ = start_monitor(&app, device);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Hardware + platform -------------------------------------------------------

fn hardware_base() -> &'static Mutex<Option<HardwareInfo>> {
    static HW: OnceLock<Mutex<Option<HardwareInfo>>> = OnceLock::new();
    HW.get_or_init(|| Mutex::new(None))
}

/// Hardware info (probed once; free disk and recommendation refreshed per call).
fn hardware_now(settings: &Settings) -> HardwareInfo {
    let dir = models::models_dir().unwrap_or_else(|_| std::env::temp_dir());
    let mut cache = hardware_base().lock().unwrap();
    let info = cache.get_or_insert_with(|| hardware::detect(&dir, &settings.dictation_language));
    info.free_disk_gb = hardware::free_disk_bytes(&dir)
        .map(|b| b as f64 / 1_073_741_824.0)
        .unwrap_or(info.free_disk_gb);
    info.recommended_model = hardware::recommended_model(info.tier, &settings.dictation_language);
    info.clone()
}

#[tauri::command]
pub async fn hardware_info() -> Result<HardwareInfo, String> {
    tauri::async_runtime::spawn_blocking(|| hardware_now(&Settings::load_or_default()))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct PlatformCheck {
    /// `linux | windows | macos`.
    pub platform: String,
    /// `x11 | wayland | windows | macos`.
    pub session: String,
    pub desktop: Option<String>,
    pub injector: String,
    pub clipboard: String,
    pub hotkeys: String,
    pub portal_hotkeys: bool,
    pub portal_typing: bool,
    pub wtype: bool,
    pub wl_clipboard: bool,
    /// Password-field detection is reliable here.
    pub secure_detection: bool,
    pub microphones: usize,
    pub notes: Vec<String>,
}

/// Setup › Permissions: what this desktop can do and what's missing.
#[tauri::command]
pub async fn platform_check() -> Result<PlatformCheck, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let wayland = is_wayland();
        let platform = std::env::consts::OS.to_string();
        let session = if cfg!(target_os = "linux") {
            if wayland { "wayland" } else { "x11" }.to_string()
        } else {
            platform.clone()
        };
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let wtype = orttaai_core::injection::wtype_available();
        let wl_clipboard = orttaai_core::clipboard::on_path("wl-copy")
            && orttaai_core::clipboard::on_path("wl-paste");
        let portal_compiled = cfg!(target_os = "linux");
        let microphones = orttaai_core::audio::list_input_devices()
            .map(|d| d.len())
            .unwrap_or(0);
        let mut notes = Vec::new();
        if microphones == 0 {
            notes.push("No microphone found. Connect one, then check again.".to_string());
        }
        if wayland {
            notes.push("Wayland: Orttaai asks your desktop for permission to type (Remote Desktop) and to listen for its shortcut (Global Shortcuts). Approve both prompts.".to_string());
            if !wtype {
                notes.push("Optional: install `wtype` (sudo apt install wtype) for typing on wlroots desktops (Sway, Hyprland).".to_string());
            }
            if !wl_clipboard {
                notes.push("Install `wl-clipboard` (sudo apt install wl-clipboard) so paste mode and voice edit can use the clipboard.".to_string());
            }
        }
        if cfg!(target_os = "linux") {
            notes.push("Linux can't tell Orttaai when a password field is focused. Turn on “Never type into password fields” in Settings › Dictation if you want to be safe.".to_string());
        }
        if cfg!(windows) {
            notes.push("Windows: apps running as Administrator ignore typing from normal apps. Run Orttaai as Administrator too if you dictate into them.".to_string());
            notes.push("Windows may ask for microphone access the first time (Settings › Privacy › Microphone › Let desktop apps access your microphone).".to_string());
        }
        let hotkeys = default_manager().backend_name().to_string();
        let injector = injector_hint();
        PlatformCheck {
            platform,
            session,
            desktop,
            injector,
            clipboard: SystemClipboard::new().backend_name().to_string(),
            hotkeys,
            portal_hotkeys: orttaai_core::hotkey::uses_portal(),
            portal_typing: portal_compiled && wayland,
            wtype,
            wl_clipboard,
            secure_detection: cfg!(windows),
            microphones,
            notes,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// The injector name without opening a portal session (which may prompt).
fn injector_hint() -> String {
    if cfg!(windows) {
        "SendInput".into()
    } else if is_wayland() {
        "RemoteDesktop portal, wtype fallback".into()
    } else if cfg!(target_os = "linux") {
        "XTest (X11)".into()
    } else {
        "CGEvent".into()
    }
}

// ---- Models ----------------------------------------------------------------------

#[tauri::command]
pub async fn list_models() -> Result<Vec<models::ModelInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = Settings::load_or_default();
        let hw = hardware_now(&settings);
        models::list(Some(&hw), &settings.dictation_language).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct ModelStorage {
    pub dir: String,
    pub default_dir: String,
    pub custom: bool,
    pub used_bytes: u64,
    pub free_gb: f64,
}

#[tauri::command]
pub fn models_storage() -> Result<ModelStorage, String> {
    let settings = Settings::load_or_default();
    let dir = models::models_dir().map_err(|e| e.to_string())?;
    let default_dir = models::default_models_dir().map_err(|e| e.to_string())?;
    Ok(ModelStorage {
        custom: settings
            .models_dir
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty()),
        used_bytes: models::disk_usage(&dir),
        free_gb: hardware::free_disk_bytes(&dir)
            .map(|b| b as f64 / 1_073_741_824.0)
            .unwrap_or(0.0),
        dir: dir.to_string_lossy().to_string(),
        default_dir: default_dir.to_string_lossy().to_string(),
    })
}

/// Choose a custom models folder (`None` = back to the default). Existing
/// downloads stay where they are; models in the new folder are picked up.
#[tauri::command]
pub fn set_models_dir(app: AppHandle, path: Option<String>) -> Result<ModelStorage, String> {
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        models::check_models_dir(std::path::Path::new(p))
            .map_err(|e| format!("Choose a writable folder ({e})."))?;
    }
    crate::settings_cmds::update_settings(app, serde_json::json!({ "models_dir": path }))?;
    models_storage()
}

/// Download a model (or `silero-vad`) in the background, emitting
/// `model-progress` / `model-done` / `model-error` / `model-cancelled`.
#[tauri::command]
pub fn download_model(app: AppHandle, id: String) {
    std::thread::spawn(move || {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let on_progress = move |p: models::Progress| {
            let _ = progress_app.emit(
                "model-progress",
                serde_json::json!({
                    "id": progress_id,
                    "fraction": p.fraction,
                    "downloaded": p.downloaded,
                    "total": p.total,
                }),
            );
        };
        let result = if id == models::VAD_DOWNLOAD_ID {
            models::download_vad(on_progress)
        } else {
            models::download(&id, on_progress)
        };
        match result {
            Ok(path) => {
                let _ = app.emit(
                    "model-done",
                    serde_json::json!({ "id": id, "path": path.to_string_lossy() }),
                );
            }
            Err(e) if models::is_cancelled(&e) => {
                let _ = app.emit("model-cancelled", serde_json::json!({ "id": id }));
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

#[tauri::command]
pub fn cancel_download(id: String) -> bool {
    models::cancel_download(&id)
}

/// Delete a downloaded model file. The engine keeps an already-loaded model
/// until it's restarted.
#[tauri::command]
pub fn delete_model(id: String) -> Result<(), String> {
    models::delete(&id).map_err(|e| e.to_string())
}

/// Make `id` the active model (hot-swapped by a running engine).
#[tauri::command]
pub fn use_model(app: AppHandle, id: String) -> Result<(), String> {
    if !models::is_downloaded(&id) {
        return Err("Download the model first.".into());
    }
    crate::settings_cmds::update_settings(app, serde_json::json!({ "model_id": id })).map(|_| ())
}

/// Full SHA-256 check of a downloaded model.
#[tauri::command]
pub async fn verify_model(id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || models::verify(&id).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct VadStatus {
    pub installed: bool,
    pub downloading: bool,
    pub path: String,
    pub size_bytes: u64,
}

#[tauri::command]
pub fn vad_status() -> VadStatus {
    VadStatus {
        installed: models::vad_installed(),
        downloading: models::active_downloads()
            .iter()
            .any(|d| d == models::VAD_DOWNLOAD_ID),
        path: models::vad_model_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: models::VAD_SIZE_BYTES,
    }
}
