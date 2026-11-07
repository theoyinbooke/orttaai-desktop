//! The dictation state machine — the heart of the app, fully OS-agnostic
//! (Mac `DictationCoordinator`).
//!
//! States: idle → recording → processing → idle, or → error (auto-dismissed
//! after 2 s). Sessions are a *dictation* (inject the transcript) or an *edit*
//! (the transcript is an instruction applied to the selected text).
//!
//! Recording modes, Mac-exact:
//! - **hold** the shortcut = push-to-talk: release stops and inserts;
//! - a short **tap** (< 0.5 s, [`crate::gesture`]) = hands-free: recording
//!   continues until the next tap, the overlay's stop button, sustained silence
//!   (`hands_free_silence_secs`, Silero VAD or the adaptive-loudness fallback),
//!   or the hands-free cap;
//! - push-to-talk is capped at `max_recording_secs`; both caps count down the
//!   final 20 s and keep every sample already heard.
//!
//! The coordinator owns only trait objects and an injectable [`Clock`], so it is
//! identical on every platform and unit-testable with mock backends. Timers are
//! driven by [`DictationCoordinator::tick`], which the host calls ~20×/s; the
//! slow part (decode → text pipeline → inject) runs in
//! [`DictationCoordinator::finish_pending`] on the host's worker thread.

use crate::audio::AudioCapture;
use crate::error::{CoreError, Result};
use crate::gesture::{GestureInterpreter, ReleaseAction};
use crate::injection::{InjectionConfig, InjectionService, SelectionCapture};
use crate::memory::MemoryService;
use crate::settings::Settings;
use crate::store::TranscriptionRecord;
use crate::transcription::{bias_prompt_for, Transcriber};
use crate::types::{
    DecodeOptions, DeviceId, InjectionMethod, InjectionResult, RecordingMode, RecordingState,
    SessionKind, COUNTDOWN_WARNING_SECS, TARGET_SAMPLE_RATE,
};
use crate::vad::{AutoStopPolicy, VoiceDetector};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---- Pipeline seams -----------------------------------------------------------

/// What the text pipeline made of a raw transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PipelineOutput {
    pub text: String,
    /// Dictionary sources applied (for usage counts).
    pub dictionary_hits: Vec<String>,
    /// Snippet triggers expanded (for usage counts).
    pub snippet_hits: Vec<String>,
    pub polished: bool,
    pub text_ms: i64,
    pub polish_ms: i64,
}

/// Raw transcript → final text (dictionary, snippets, cleanup, polish…). The
/// text/polish areas provide the full implementation; [`MemoryPipeline`] is the
/// default.
pub trait TranscriptPipeline: Send + Sync {
    fn process(&self, raw: &str, app: Option<&str>) -> PipelineOutput;
}

/// Default pipeline: Personal Memory dictionary + snippets only.
pub struct MemoryPipeline(pub MemoryService);

impl TranscriptPipeline for MemoryPipeline {
    fn process(&self, raw: &str, _app: Option<&str>) -> PipelineOutput {
        let started = Instant::now();
        let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let pass = self.0.apply_dictionary(&normalized);
        let (text, snippet_hits) = self.0.expand_inline_snippets(&pass.text);
        let mut dictionary_hits: Vec<String> = Vec::new();
        for hit in pass.hits {
            let key = hit.source.to_lowercase();
            if !dictionary_hits.contains(&key) {
                dictionary_hits.push(key);
            }
        }
        PipelineOutput {
            text,
            dictionary_hits,
            snippet_hits: snippet_hits.into_iter().map(|t| t.to_lowercase()).collect(),
            polished: false,
            text_ms: started.elapsed().as_millis() as i64,
            polish_ms: 0,
        }
    }
}

/// Voice edit: apply a spoken instruction to the selected text. Must return an
/// error — never invented text — when it can't do the edit; the selection is
/// then left untouched.
pub trait EditRewriter: Send + Sync {
    fn rewrite(&self, selection: &str, instruction: &str) -> crate::Result<String>;
}

// ---- Clock --------------------------------------------------------------------

/// Monotonic time source (injectable so gestures/caps are testable).
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

/// Wall-clock monotonic time since construction.
pub struct SystemClock(Instant);

impl SystemClock {
    pub fn new() -> Self {
        Self(Instant::now())
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Test clock advanced by hand.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<Mutex<Duration>>);

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

// ---- Configuration --------------------------------------------------------------

/// Dictation settings the coordinator applies live.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub hands_free_enabled: bool,
    /// Hands-free silence auto-stop window; `None` = off.
    pub silence_stop: Option<Duration>,
    pub hands_free_max: Duration,
    pub max_recording: Duration,
    pub edit_enabled: bool,
    pub edit_max_chars: usize,
    pub show_processing_estimate: bool,
    pub vocabulary_bias: bool,
    /// Preferred input device by persistent name; `None` = system default.
    pub audio_device: Option<String>,
    /// The selected model id (reported when the transcriber has none).
    pub model_id: String,
    pub injection: InjectionConfig,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

impl CoordinatorConfig {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            hands_free_enabled: s.hands_free_enabled,
            silence_stop: s
                .hands_free_silence_stop
                .then(|| Duration::from_secs_f32(s.hands_free_silence_secs.clamp(1.0, 10.0))),
            hands_free_max: Duration::from_secs(s.hands_free_max_secs.clamp(120, 1800) as u64),
            max_recording: Duration::from_secs(s.max_recording_secs.clamp(10, 300) as u64),
            edit_enabled: s.edit_commands_enabled,
            edit_max_chars: s.edit_max_chars,
            show_processing_estimate: s.show_processing_estimate,
            vocabulary_bias: s.vocabulary_bias,
            audio_device: s.audio_device.clone().filter(|d| !d.trim().is_empty()),
            model_id: s.model_id.clone(),
            injection: InjectionConfig {
                mode: s.injection_mode,
                preserve_clipboard: s.preserve_clipboard,
                low_latency: s.low_latency,
                strict_secure: s.strict_secure,
            },
        }
    }
}

/// Why a recording ended (Mac `RecordingEndReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    HoldRelease,
    HotkeyTapStop,
    /// The overlay's stop button, the tray, or click-to-record.
    PillStop,
    SilenceAutoStop,
    CapTimer,
}

// ---- Constants (Mac) ------------------------------------------------------------

/// Recordings shorter than this are discarded as accidental.
pub const MIN_RECORDING: Duration = Duration::from_millis(500);
/// Error pill auto-dismiss.
pub const ERROR_DISMISS: Duration = Duration::from_secs(2);
/// Hands-free silence monitor poll.
const SILENCE_POLL: Duration = Duration::from_millis(250);
/// Capture coverage that counts as complete.
const MIN_COVERAGE: f64 = 0.90;
/// A gap is worth keeping with ≥ half the audio and ≥ 3 s of it.
const MIN_PARTIAL_COVERAGE: f64 = 0.5;
const MIN_PARTIAL_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * 3;
/// Stream start-up latency allowance when judging coverage (desktop audio
/// stacks deliver the first buffer later than Core Audio does).
const CAPTURE_START_GRACE: Duration = Duration::from_millis(300);
/// Up to ~1 s of preceding audio handed to the voice detector as context.
const VOICE_CONTEXT_SAMPLES: usize = 15_872;

/// Whether a finished recording has enough of its audio to transcribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCompleteness {
    Complete,
    /// A gap, but most of a real recording arrived: keep it and say so.
    Partial,
    /// Too little arrived to be worth transcribing.
    Dropped,
}

pub fn capture_coverage(samples: usize, recording: Duration) -> f64 {
    let expected_ms = recording.saturating_sub(CAPTURE_START_GRACE).as_millis() as f64;
    if expected_ms <= 0.0 {
        return if samples > 0 { 1.0 } else { 0.0 };
    }
    let expected = expected_ms * TARGET_SAMPLE_RATE as f64 / 1000.0;
    (samples as f64 / expected).min(1.0)
}

pub fn capture_completeness(samples: usize, coverage: f64) -> CaptureCompleteness {
    if coverage >= MIN_COVERAGE {
        CaptureCompleteness::Complete
    } else if coverage >= MIN_PARTIAL_COVERAGE && samples >= MIN_PARTIAL_SAMPLES {
        CaptureCompleteness::Partial
    } else {
        CaptureCompleteness::Dropped
    }
}

/// Pill message for a capture start failure (Mac `startFailureMessage`).
pub fn start_failure_message(err: &CoreError) -> &'static str {
    let m = err.to_string().to_lowercase();
    if m.contains("permission") || m.contains("denied") || m.contains("access") {
        "Microphone access needed"
    } else if m.contains("no default input")
        || m.contains("no audio input")
        || m.contains("no microphone")
        || m.contains("not found")
    {
        "No microphone signal"
    } else {
        "Can't start recording"
    }
}

/// Initial processing-time factor (× recording length) by model size; refined
/// from measured dictations.
fn initial_processing_factor(model_id: &str) -> f32 {
    let l = model_id.to_ascii_lowercase();
    let cpu = if l.contains("tiny") {
        0.1
    } else if l.contains("base") {
        0.15
    } else if l.contains("small") {
        0.3
    } else if l.contains("medium") {
        0.6
    } else if l.contains("large") {
        0.8
    } else {
        0.3
    };
    if cfg!(feature = "cuda") {
        cpu * 0.3
    } else {
        cpu
    }
}

// ---- Status + outcome -------------------------------------------------------------

/// Snapshot for the UI (`engine-status` event) — diffed by the host to drive
/// sound cues, the overlay and the tray.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoordinatorStatus {
    pub state: RecordingState,
    pub mode: RecordingMode,
    pub kind: SessionKind,
    /// Whole seconds recorded (0 when not recording).
    pub elapsed_secs: u64,
    /// Seconds left in the final 20 s of the recording window.
    pub countdown: Option<u32>,
    /// "~Ns to process" while processing (when enabled).
    pub estimate_secs: Option<u32>,
    /// The error pill's message.
    pub error: Option<String>,
    /// Audio capture is reconnecting mid-recording.
    pub recovering: bool,
    /// The app being dictated into, when known.
    pub target_app: Option<String>,
}

/// Result of a completed session — everything the Mac records with a history row.
#[derive(Debug, Clone, Serialize)]
pub struct DictationOutcome {
    pub result: InjectionResult,
    /// The final text typed (or left on the clipboard), when one was produced.
    pub transcript: Option<String>,
    /// What the recognizer heard, before the text pipeline.
    pub raw_text: Option<String>,
    /// How long the user spoke, in ms.
    pub duration_ms: i64,
    /// Set when a transcript was produced but delivery failed.
    pub inject_error: Option<String>,
    pub kind: SessionKind,
    pub edit_instruction: Option<String>,
    pub app: Option<String>,
    pub audio_device: Option<String>,
    pub model_id: String,
    pub injection_method: Option<InjectionMethod>,
    pub polished: bool,
    pub transcription_ms: i64,
    pub text_ms: i64,
    pub polish_ms: i64,
    pub injection_ms: i64,
    /// Release → delivered, in ms.
    pub processing_ms: i64,
    pub dictionary_hits: Vec<String>,
    pub snippet_hits: Vec<String>,
    /// Part of the recording's audio never arrived; the rest was used.
    pub lost_audio: bool,
    /// The error pill message for this session, if any.
    pub error: Option<String>,
}

impl DictationOutcome {
    fn empty(kind: SessionKind, duration_ms: i64, model_id: String) -> Self {
        Self {
            result: InjectionResult::NoTranscript,
            transcript: None,
            raw_text: None,
            duration_ms,
            inject_error: None,
            kind,
            edit_instruction: None,
            app: None,
            audio_device: None,
            model_id,
            injection_method: None,
            polished: false,
            transcription_ms: 0,
            text_ms: 0,
            polish_ms: 0,
            injection_ms: 0,
            processing_ms: 0,
            dictionary_hits: Vec::new(),
            snippet_hits: Vec::new(),
            lost_audio: false,
            error: None,
        }
    }

    /// The history row for this session; `None` when nothing was produced.
    pub fn to_record(&self, created_at: i64) -> Option<TranscriptionRecord> {
        let text = self.transcript.as_ref()?;
        let mut r =
            TranscriptionRecord::new(text.clone(), self.app.clone(), self.duration_ms, created_at);
        r.raw_text = self.raw_text.clone();
        r.processing_ms = self.processing_ms;
        r.transcription_ms = Some(self.transcription_ms);
        r.text_processing_ms = Some(self.text_ms);
        r.polish_ms = self.polished.then_some(self.polish_ms);
        r.injection_ms = Some(self.injection_ms);
        r.model_id = self.model_id.clone();
        r.audio_device = self.audio_device.clone();
        r.injection_method = self.injection_method.map(|m| m.as_str().to_string());
        r.entry_kind = self.kind.as_str().to_string();
        r.edit_instruction = self.edit_instruction.clone();
        r.polished = self.polished;
        Some(r)
    }
}

// ---- Coordinator --------------------------------------------------------------------

struct Session {
    started: Duration,
    app: Option<String>,
    device: Option<String>,
    /// Edit sessions: the captured selection (read at the edit key's release).
    selection: Option<String>,
}

struct Pending {
    samples: Vec<f32>,
    duration_ms: i64,
    kind: SessionKind,
    app: Option<String>,
    device: Option<String>,
    selection: Option<String>,
    lost_audio: bool,
}

pub type FocusedAppProvider = Box<dyn Fn() -> Option<String> + Send + Sync>;

pub struct DictationCoordinator {
    transcriber: Box<dyn Transcriber>,
    audio: Box<dyn AudioCapture>,
    injection: InjectionService,
    pipeline: Box<dyn TranscriptPipeline>,
    rewriter: Option<Box<dyn EditRewriter>>,
    voice: Option<Box<dyn VoiceDetector>>,
    clock: Arc<dyn Clock>,
    focused_app: FocusedAppProvider,
    config: CoordinatorConfig,
    options: DecodeOptions,
    vocabulary: Vec<String>,

    state: RecordingState,
    mode: RecordingMode,
    kind: SessionKind,
    gesture: GestureInterpreter,
    edit_gesture: GestureInterpreter,
    session: Option<Session>,
    pending: Option<Pending>,
    error: Option<(String, Duration)>,
    countdown: Option<u32>,
    estimate_secs: Option<u32>,
    policy: AutoStopPolicy,
    last_silence_check: Duration,
    processing_factor: f32,
}

impl DictationCoordinator {
    pub fn new(
        transcriber: Box<dyn Transcriber>,
        audio: Box<dyn AudioCapture>,
        injection: InjectionService,
        pipeline: Box<dyn TranscriptPipeline>,
        options: DecodeOptions,
    ) -> Self {
        let config = CoordinatorConfig::default();
        let processing_factor =
            initial_processing_factor(transcriber.loaded_model().unwrap_or(&config.model_id));
        let mut injection = injection;
        injection.set_config(config.injection.clone());
        Self {
            transcriber,
            audio,
            injection,
            pipeline,
            rewriter: None,
            voice: None,
            clock: Arc::new(SystemClock::new()),
            focused_app: Box::new(crate::focus::focused_app),
            config,
            options,
            vocabulary: Vec::new(),
            state: RecordingState::Idle,
            mode: RecordingMode::PushToTalk,
            kind: SessionKind::Dictation,
            gesture: GestureInterpreter::new(),
            edit_gesture: GestureInterpreter::new(),
            session: None,
            pending: None,
            error: None,
            countdown: None,
            estimate_secs: None,
            policy: AutoStopPolicy::new(),
            last_silence_check: Duration::ZERO,
            processing_factor,
        }
    }

    /// Convenience: Personal Memory pipeline.
    pub fn with_memory(
        transcriber: Box<dyn Transcriber>,
        audio: Box<dyn AudioCapture>,
        injection: InjectionService,
        memory: MemoryService,
        options: DecodeOptions,
    ) -> Self {
        Self::new(
            transcriber,
            audio,
            injection,
            Box::new(MemoryPipeline(memory)),
            options,
        )
    }

    // ---- configuration ----

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn set_focused_app_provider(&mut self, provider: FocusedAppProvider) {
        self.focused_app = provider;
    }

    pub fn config(&self) -> &CoordinatorConfig {
        &self.config
    }

    /// Apply dictation settings live. Takes effect for the next session (the
    /// injection policy immediately).
    pub fn set_config(&mut self, config: CoordinatorConfig) {
        self.injection.set_config(config.injection.clone());
        self.config = config;
    }

    pub fn set_decode_options(&mut self, options: DecodeOptions) {
        self.options = options;
    }

    pub fn decode_options(&self) -> &DecodeOptions {
        &self.options
    }

    /// Opt in to blocking injection when the secure-field status is unknown.
    pub fn set_strict_secure(&mut self, strict: bool) {
        let mut c = self.config.clone();
        c.injection.strict_secure = strict;
        self.set_config(c);
    }

    /// Vocabulary-bias terms (active dictionary targets + snippet triggers).
    pub fn set_vocabulary(&mut self, terms: Vec<String>) {
        self.vocabulary = terms;
    }

    pub fn set_pipeline(&mut self, pipeline: Box<dyn TranscriptPipeline>) {
        self.pipeline = pipeline;
    }

    pub fn set_rewriter(&mut self, rewriter: Option<Box<dyn EditRewriter>>) {
        self.rewriter = rewriter;
    }

    /// Silero (or any) voice detector for hands-free auto-stop; `None` uses the
    /// adaptive-loudness fallback.
    pub fn set_voice_detector(&mut self, voice: Option<Box<dyn VoiceDetector>>) {
        self.voice = voice;
    }

    pub fn has_voice_detector(&self) -> bool {
        self.voice.is_some()
    }

    pub fn replace_transcriber(&mut self, transcriber: Box<dyn Transcriber>) {
        if let Some(id) = transcriber.loaded_model() {
            self.processing_factor = initial_processing_factor(id);
        }
        self.transcriber = transcriber;
    }

    pub fn loaded_model(&self) -> Option<&str> {
        self.transcriber.loaded_model()
    }

    /// Silent decode so the first dictation doesn't pay the cold start.
    pub fn warm_up(&self) -> Result<()> {
        crate::transcription::warm_up(&*self.transcriber, &self.options)
    }

    pub fn injection(&self) -> &InjectionService {
        &self.injection
    }

    /// "Reset Audio" (Mac): only while idle.
    pub fn reset_audio(&mut self) -> Result<()> {
        if self.state != RecordingState::Idle {
            return Err(CoreError::Audio(
                "Stop dictation before resetting audio.".to_string(),
            ));
        }
        self.audio.reset()
    }

    // ---- observation ----

    pub fn state(&self) -> RecordingState {
        self.state
    }

    pub fn mode(&self) -> RecordingMode {
        self.mode
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Current input level (0.0..=1.0) for the live mic meter.
    pub fn level(&self) -> f32 {
        if self.state == RecordingState::Recording {
            self.audio.level()
        } else {
            0.0
        }
    }

    pub fn status(&self) -> CoordinatorStatus {
        let now = self.clock.now();
        let recording = self.state == RecordingState::Recording;
        CoordinatorStatus {
            state: self.state,
            mode: self.mode,
            kind: self.kind,
            elapsed_secs: match (&self.session, recording) {
                (Some(s), true) => now.saturating_sub(s.started).as_secs(),
                _ => 0,
            },
            countdown: if recording { self.countdown } else { None },
            estimate_secs: if self.state == RecordingState::Processing {
                self.estimate_secs
            } else {
                None
            },
            error: self.error.as_ref().map(|(m, _)| m.clone()),
            recovering: recording && self.audio.is_recovering(),
            target_app: self.session.as_ref().and_then(|s| s.app.clone()),
        }
    }

    // ---- hotkey gestures (tap vs hold) ----

    /// Key-down on the push-to-talk shortcut. Recording starts immediately in
    /// idle — tap/hold disambiguation never delays capture. While a hands-free
    /// dictation is active, a key-down stops it (second tap). Ignored while
    /// processing or showing an error.
    pub fn handle_hotkey_down(&mut self) {
        match self.state {
            RecordingState::Idle => {
                self.gesture.record_press(self.clock.now());
                self.begin_session(SessionKind::Dictation);
                if self.state != RecordingState::Recording {
                    self.gesture.reset();
                }
            }
            RecordingState::Recording => {
                if self.mode == RecordingMode::HandsFree && self.kind == SessionKind::Dictation {
                    // The key-up that follows finds no press and is ignored.
                    self.gesture.reset();
                    self.stop_recording(StopReason::HotkeyTapStop);
                }
                // A repeat while holding, or the dictation key during an edit
                // session: inert.
            }
            RecordingState::Processing | RecordingState::Error => self.gesture.reset(),
        }
    }

    /// Key-up on the push-to-talk shortcut: a tap promotes the running
    /// recording to hands-free; a hold release stops it.
    pub fn handle_hotkey_up(&mut self) {
        let action = self
            .gesture
            .evaluate_release(self.clock.now(), self.config.hands_free_enabled);
        if self.state != RecordingState::Recording
            || self.mode != RecordingMode::PushToTalk
            || self.kind != SessionKind::Dictation
        {
            return;
        }
        self.apply_gesture(action);
    }

    /// Key-down on the voice-edit shortcut. Starts recording the spoken
    /// instruction in idle; stops a hands-free edit (second tap). The selection
    /// is read when the key is released, once the user's fingers are off the
    /// shortcut's modifiers (a synthetic Ctrl+C with Shift still held would be
    /// Ctrl+Shift+C).
    pub fn handle_edit_hotkey_down(&mut self) {
        if !self.config.edit_enabled {
            return;
        }
        match self.state {
            RecordingState::Idle => {
                if self.rewriter.is_none() {
                    self.set_error(
                        "Voice edit needs a local AI model. Set one up in Settings › AI Features.",
                    );
                    return;
                }
                self.edit_gesture.record_press(self.clock.now());
                self.begin_session(SessionKind::Edit);
                if self.state != RecordingState::Recording {
                    self.edit_gesture.reset();
                }
            }
            RecordingState::Recording => {
                if self.kind == SessionKind::Edit && self.mode == RecordingMode::HandsFree {
                    self.edit_gesture.reset();
                    self.stop_recording(StopReason::HotkeyTapStop);
                }
            }
            RecordingState::Processing | RecordingState::Error => self.edit_gesture.reset(),
        }
    }

    /// Key-up on the voice-edit shortcut: capture the selection, then tap →
    /// hands-free, hold → stop.
    pub fn handle_edit_hotkey_up(&mut self) {
        let action = self
            .edit_gesture
            .evaluate_release(self.clock.now(), self.config.hands_free_enabled);
        if action == ReleaseAction::Ignore
            || self.state != RecordingState::Recording
            || self.kind != SessionKind::Edit
            || self.mode != RecordingMode::PushToTalk
        {
            return;
        }
        if !self.ensure_selection() {
            return;
        }
        self.apply_gesture(action);
    }

    fn apply_gesture(&mut self, action: ReleaseAction) {
        match action {
            ReleaseAction::PromoteToHandsFree => self.promote_to_hands_free(),
            ReleaseAction::StopRecording => self.stop_recording(StopReason::HoldRelease),
            ReleaseAction::Ignore => {}
        }
    }

    /// Tray / click-to-record / overlay mic: idle → a hands-free recording
    /// (plain recording when hands-free is off); recording → stop.
    pub fn toggle(&mut self) {
        match self.state {
            RecordingState::Recording => self.stop_recording(StopReason::PillStop),
            RecordingState::Idle | RecordingState::Error => {
                if self.state == RecordingState::Error {
                    self.clear_error();
                }
                self.start_hands_free();
            }
            RecordingState::Processing => {}
        }
    }

    /// Start a recording that is hands-free from the outset (no key is held).
    pub fn start_hands_free(&mut self) {
        self.begin_session(SessionKind::Dictation);
        if self.config.hands_free_enabled {
            self.promote_to_hands_free();
        }
    }

    // ---- legacy one-shot API (CLI, simple hosts) ----

    /// Begin a push-to-talk dictation. Recovers from a previous error so one
    /// failure doesn't brick dictation for the session.
    pub fn on_press(&mut self) -> Result<()> {
        if self.state == RecordingState::Error {
            self.clear_error();
        }
        if self.state != RecordingState::Idle {
            return Ok(());
        }
        self.begin_session(SessionKind::Dictation);
        match self.state {
            RecordingState::Recording => Ok(()),
            _ => Err(CoreError::Audio(
                self.error
                    .as_ref()
                    .map(|(m, _)| m.clone())
                    .unwrap_or_else(|| "Can't start recording".into()),
            )),
        }
    }

    /// Stop, transcribe, process and inject.
    pub fn on_release(&mut self) -> Result<DictationOutcome> {
        if self.state != RecordingState::Recording {
            return Ok(DictationOutcome::empty(
                self.kind,
                0,
                self.config.model_id.clone(),
            ));
        }
        self.stop_recording(StopReason::HoldRelease);
        self.finish_pending()
    }

    // ---- session lifecycle ----

    fn begin_session(&mut self, kind: SessionKind) {
        if self.state != RecordingState::Idle {
            return;
        }
        self.mode = RecordingMode::PushToTalk;
        self.kind = kind;
        // Capture the target app NOW, before any overlay appears.
        let app = (self.focused_app)();
        let device = self.config.audio_device.clone().map(DeviceId);
        match self.audio.start(device) {
            Ok(()) => {
                let now = self.clock.now();
                self.session = Some(Session {
                    started: now,
                    app,
                    device: self.audio.active_device(),
                    selection: None,
                });
                self.policy = AutoStopPolicy::new();
                self.last_silence_check = now;
                self.countdown = None;
                if let Some(v) = self.voice.as_mut() {
                    v.reset();
                }
                self.state = RecordingState::Recording;
                tracing::debug!("recording started ({kind:?})");
            }
            Err(e) => {
                tracing::error!("failed to start recording: {e}");
                self.kind = SessionKind::Dictation;
                self.set_error(start_failure_message(&e));
            }
        }
    }

    /// Switch an in-progress push-to-talk recording to hands-free. Capture
    /// continues untouched; the cap moves to the hands-free budget and the
    /// silence auto-stop starts evaluating.
    fn promote_to_hands_free(&mut self) {
        if self.state != RecordingState::Recording || self.mode != RecordingMode::PushToTalk {
            return;
        }
        self.mode = RecordingMode::HandsFree;
        self.countdown = None;
        self.advance_policy();
        tracing::info!("hands-free mode engaged");
    }

    fn max_duration(&self) -> Duration {
        match self.mode {
            RecordingMode::PushToTalk => self.config.max_recording,
            RecordingMode::HandsFree => self.config.hands_free_max,
        }
    }

    /// Drive timers: countdown + cap, hands-free silence auto-stop, error
    /// auto-dismiss. Call ~20×/s.
    pub fn tick(&mut self) {
        let now = self.clock.now();
        match self.state {
            RecordingState::Recording => {
                let Some(started) = self.session.as_ref().map(|s| s.started) else {
                    return;
                };
                let elapsed = now.saturating_sub(started);
                let max = self.max_duration();
                if elapsed >= max {
                    tracing::info!("cap reached at {}s; stopping", max.as_secs());
                    self.stop_recording(StopReason::CapTimer);
                    return;
                }
                let warn_from =
                    max.saturating_sub(Duration::from_secs(COUNTDOWN_WARNING_SECS as u64));
                self.countdown = (elapsed >= warn_from).then(|| {
                    let left = max - elapsed;
                    (left.as_millis().div_ceil(1000) as u32).max(1)
                });
                if self.mode == RecordingMode::HandsFree
                    && now.saturating_sub(self.last_silence_check) >= SILENCE_POLL
                {
                    self.last_silence_check = now;
                    self.evaluate_silence(elapsed);
                }
            }
            RecordingState::Error => {
                if let Some((_, since)) = &self.error {
                    if now.saturating_sub(*since) >= ERROR_DISMISS {
                        self.clear_error();
                    }
                }
            }
            _ => {}
        }
    }

    fn advance_policy(&mut self) {
        let total = self.audio.captured_samples();
        let audio = &self.audio;
        self.policy.advance(total, |from| audio.samples_from(from));
    }

    fn listen_for_voice(&mut self) {
        if self.policy.voice_detector_failed() {
            return;
        }
        let Some(voice) = self.voice.as_mut() else {
            return;
        };
        let start = self.policy.voice_scanned_samples();
        let chunk = voice.chunk_samples();
        let available = self.audio.captured_samples().saturating_sub(start);
        if chunk == 0 || available < chunk {
            return;
        }
        let ctx_start = start.saturating_sub(VOICE_CONTEXT_SAMPLES);
        let all = self.audio.samples_from(ctx_start);
        let split = (start - ctx_start).min(all.len());
        let (context, rest) = all.split_at(split);
        let whole = (rest.len() / chunk) * chunk;
        match voice.speech(context, &rest[..whole]) {
            Ok(verdicts) => self.policy.heard_voice(&verdicts, chunk),
            Err(e) => {
                tracing::error!("voice detector failed mid-recording; using loudness: {e}");
                self.policy.abandon_voice_detector();
            }
        }
    }

    /// Stop a hands-free recording once sustained speech has been followed by
    /// the configured silence window.
    fn evaluate_silence(&mut self, elapsed: Duration) {
        let Some(window) = self.config.silence_stop else {
            return;
        };
        self.listen_for_voice();
        self.advance_policy();
        if self.policy.should_stop(window, elapsed, MIN_RECORDING) {
            tracing::info!(
                "hands-free auto-stop ({}): trailing silence {} ms",
                if self.policy.uses_voice_detector() {
                    "voice"
                } else {
                    "loudness"
                },
                self.policy.trailing_silence_ms()
            );
            self.stop_recording(StopReason::SilenceAutoStop);
        }
    }

    /// Edit sessions: read the selection once. `false` (session aborted with an
    /// error pill) when there is none, it's a password field, or it's too long.
    fn ensure_selection(&mut self) -> bool {
        if self.kind != SessionKind::Edit {
            return true;
        }
        if self.session.as_ref().is_some_and(|s| s.selection.is_some()) {
            return true;
        }
        let message = match self.injection.capture_selection() {
            SelectionCapture::Captured(text) => {
                if text.chars().count() > self.config.edit_max_chars {
                    "Selection too long to edit"
                } else {
                    if let Some(s) = self.session.as_mut() {
                        s.selection = Some(text);
                    }
                    return true;
                }
            }
            SelectionCapture::NoSelection => "Select text first",
            SelectionCapture::BlockedSecureField => "Can't edit password fields",
        };
        self.abort_session(message);
        false
    }

    /// Discard the recording and show `message`.
    fn abort_session(&mut self, message: &str) {
        let _ = self.audio.stop();
        self.gesture.reset();
        self.edit_gesture.reset();
        self.end_session();
        self.set_error(message);
    }

    /// Cancel any recording without transcribing (engine stopping).
    pub fn cancel(&mut self) {
        if self.state == RecordingState::Recording {
            let _ = self.audio.stop();
        }
        self.pending = None;
        self.end_session();
        self.error = None;
        self.state = RecordingState::Idle;
    }

    fn end_session(&mut self) {
        self.session = None;
        self.countdown = None;
        self.mode = RecordingMode::PushToTalk;
        self.kind = SessionKind::Dictation;
        self.policy = AutoStopPolicy::new();
    }

    fn set_error(&mut self, message: &str) {
        self.state = RecordingState::Error;
        self.error = Some((message.to_string(), self.clock.now()));
    }

    fn clear_error(&mut self) {
        self.error = None;
        if self.state == RecordingState::Error {
            self.state = RecordingState::Idle;
        }
    }

    /// End the recording and queue it for [`finish_pending`](Self::finish_pending).
    pub fn stop_recording(&mut self, reason: StopReason) {
        if self.state != RecordingState::Recording {
            return;
        }
        // An edit stopped before its key was released (stop button, cap,
        // silence) still needs its selection.
        if self.kind == SessionKind::Edit && !self.ensure_selection() {
            return;
        }
        let Some(session) = self.session.take() else {
            self.state = RecordingState::Idle;
            return;
        };
        let samples = match self.audio.stop() {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("stop capture failed: {e}");
                self.end_session();
                self.set_error("Microphone unavailable. Try again.");
                return;
            }
        };
        let wall = self.clock.now().saturating_sub(session.started);
        let audio_len = Duration::from_secs_f64(samples.len() as f64 / TARGET_SAMPLE_RATE as f64);
        let duration = wall.max(audio_len);

        if duration < MIN_RECORDING {
            tracing::info!("recording too short ({:?}); skipping", duration);
            self.end_session();
            self.state = RecordingState::Idle;
            return;
        }

        let coverage = capture_coverage(samples.len(), wall.max(Duration::from_millis(1)));
        let lost_audio = match capture_completeness(samples.len(), coverage) {
            CaptureCompleteness::Complete => false,
            CaptureCompleteness::Partial => {
                tracing::warn!(
                    "keeping incomplete recording ({:.0}% coverage)",
                    coverage * 100.0
                );
                true
            }
            CaptureCompleteness::Dropped => {
                tracing::error!(
                    "rejecting incomplete recording ({:.0}% coverage)",
                    coverage * 100.0
                );
                self.end_session();
                self.set_error("Audio was dropped. Please dictate again.");
                return;
            }
        };

        tracing::info!(
            "recording stopped ({reason:?}) after {:.1}s",
            duration.as_secs_f32()
        );
        let duration_ms = duration.as_millis() as i64;
        self.estimate_secs = self.config.show_processing_estimate.then(|| {
            (duration.as_secs_f32() * self.processing_factor)
                .max(1.0)
                .round() as u32
        });
        self.pending = Some(Pending {
            samples,
            duration_ms,
            kind: self.kind,
            app: session.app,
            device: session.device,
            selection: session.selection,
            lost_audio,
        });
        self.countdown = None;
        self.state = RecordingState::Processing;
    }

    /// Transcribe and deliver the queued recording. Slow (the decode) — run on
    /// a worker thread. `Err` only for a failed transcription (the state is then
    /// `Error` with the pill message); every other failure is reported in the
    /// outcome.
    pub fn finish_pending(&mut self) -> Result<DictationOutcome> {
        let model_id = self
            .transcriber
            .loaded_model()
            .map(str::to_string)
            .unwrap_or_else(|| self.config.model_id.clone());
        let Some(p) = self.pending.take() else {
            return Ok(DictationOutcome::empty(self.kind, 0, model_id));
        };
        let started = Instant::now();
        let mut out = DictationOutcome::empty(p.kind, p.duration_ms, model_id);
        out.app = p.app.clone();
        out.audio_device = p.device.clone();
        out.lost_audio = p.lost_audio;

        let mut opts = self.options.clone();
        opts.initial_prompt = if self.config.vocabulary_bias {
            bias_prompt_for(&self.vocabulary, &p.samples)
        } else {
            None
        };
        let t0 = Instant::now();
        let raw = match self.transcriber.transcribe(&p.samples, &opts) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("transcription failed: {e}");
                let message = match &e {
                    CoreError::ModelNotLoaded => "No speech model loaded. Pick one in Models.",
                    _ => "Couldn't transcribe. Try again.",
                };
                self.end_session();
                self.set_error(message);
                return Err(e);
            }
        };
        out.transcription_ms = t0.elapsed().as_millis() as i64;
        let cleaned = strip_nonspeech_markers(raw.trim());
        out.raw_text = (!cleaned.is_empty()).then(|| cleaned.clone());

        let outcome = match p.kind {
            SessionKind::Edit => self.finish_edit(out, &cleaned, p.selection),
            SessionKind::Dictation => self.finish_dictation(out, &cleaned, p.lost_audio),
        };
        let mut outcome = outcome;
        outcome.processing_ms = started.elapsed().as_millis() as i64;
        if outcome.transcript.is_some() && p.duration_ms > 0 {
            let measured = outcome.processing_ms as f32 / p.duration_ms as f32;
            self.processing_factor =
                (0.7 * self.processing_factor + 0.3 * measured).clamp(0.05, 3.0);
        }
        self.end_session();
        Ok(outcome)
    }

    fn finish_dictation(
        &mut self,
        mut out: DictationOutcome,
        cleaned: &str,
        lost_audio: bool,
    ) -> DictationOutcome {
        let processed = self.pipeline.process(cleaned, out.app.as_deref());
        out.text_ms = processed.text_ms;
        out.polish_ms = processed.polish_ms;
        out.polished = processed.polished;
        out.dictionary_hits = processed.dictionary_hits;
        out.snippet_hits = processed.snippet_hits;
        let text = processed.text.trim().to_string();
        if text.is_empty() {
            self.state = RecordingState::Idle;
            return out;
        }
        if self.injection.secure_blocked() {
            tracing::info!("injection blocked: focused field is (or may be) a password field");
            out.result = InjectionResult::BlockedSecureField;
            out.injection_method = Some(InjectionMethod::Blocked);
            self.fail(&mut out, "Can't dictate into password fields");
            return out;
        }
        let delivery = self.injection.deliver(&text, out.app.as_deref());
        out.injection_ms = delivery.ms;
        out.injection_method = Some(delivery.method);
        out.result = delivery.result;
        out.transcript = Some(text);
        match delivery.result {
            InjectionResult::Success => {
                if lost_audio {
                    self.fail(&mut out, "Some audio was lost. The rest was inserted.");
                } else {
                    self.state = RecordingState::Idle;
                }
            }
            _ => {
                out.inject_error = delivery.error;
                let msg = if delivery.method == InjectionMethod::Clipboard {
                    "Couldn't insert text. Press Ctrl+V to paste it."
                } else {
                    "Couldn't insert text."
                };
                self.fail(&mut out, msg);
            }
        }
        out
    }

    /// Apply the spoken instruction to the captured selection and replace it.
    /// Every failure leaves the selection untouched and shows an honest error —
    /// a refusal or garbage response is never injected.
    fn finish_edit(
        &mut self,
        mut out: DictationOutcome,
        instruction: &str,
        selection: Option<String>,
    ) -> DictationOutcome {
        let instruction = instruction.trim();
        if instruction.is_empty() {
            self.fail(&mut out, "Didn't catch the instruction. Try again.");
            return out;
        }
        out.edit_instruction = Some(instruction.to_string());
        let Some(selection) = selection else {
            self.fail(&mut out, "Select text first");
            return out;
        };
        let Some(rewriter) = self.rewriter.as_ref() else {
            self.fail(
                &mut out,
                "Voice edit needs a local AI model. Set one up in Settings › AI Features.",
            );
            return out;
        };
        let t0 = Instant::now();
        let edited = rewriter.rewrite(&selection, instruction);
        out.text_ms = t0.elapsed().as_millis() as i64;
        let edited = match edited {
            Ok(e) => e,
            Err(e) => {
                let reason = match e {
                    CoreError::Llm(m) | CoreError::Transcription(m) => m,
                    other => other.to_string(),
                };
                tracing::warn!("edit failed: {reason}");
                self.fail(&mut out, &format!("Couldn't edit: {reason}"));
                return out;
            }
        };
        if edited.trim().is_empty() {
            self.fail(
                &mut out,
                "Couldn't edit the text. The selection was left as it was.",
            );
            return out;
        }
        if edited.trim() == selection.trim() {
            self.fail(&mut out, "No changes needed");
            return out;
        }
        let delivery = self.injection.deliver(&edited, out.app.as_deref());
        out.injection_ms = delivery.ms;
        out.injection_method = Some(delivery.method);
        out.result = delivery.result;
        match delivery.result {
            InjectionResult::Success => {
                out.transcript = Some(edited);
                self.state = RecordingState::Idle;
            }
            InjectionResult::BlockedSecureField => {
                self.fail(&mut out, "Can't edit password fields");
            }
            _ => {
                out.transcript = Some(edited);
                out.inject_error = delivery.error;
                self.fail(
                    &mut out,
                    "Couldn't insert the edit. Press Ctrl+V to paste it.",
                );
            }
        }
        out
    }

    fn fail(&mut self, out: &mut DictationOutcome, message: &str) {
        out.error = Some(message.to_string());
        self.set_error(message);
    }
}

/// whisper.cpp annotates non-speech audio with bracketed tags like
/// `[BLANK_AUDIO]`, `[Music]`, or `(dramatic music)` — and emits them *inline*
/// within an otherwise-real transcript, not only on their own. Strip any
/// bracketed/parenthesized span that names a non-speech sound, leaving genuine
/// dictated parentheticals (e.g. "call me (maybe)") intact, then collapse the
/// whitespace the removed spans leave behind.
pub fn strip_nonspeech_markers(text: &str) -> String {
    const NONSPEECH: &[&str] = &[
        "blank_audio",
        "music",
        "silence",
        "applause",
        "laughter",
        "inaudible",
        "noise",
        "no audio",
        "no speech",
        "sighs",
        "coughs",
        "beep",
    ];
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let closer = match c {
            '[' => Some(']'),
            '(' => Some(')'),
            _ => None,
        };
        if let Some(close_ch) = closer {
            if let Some(rel) = text[i + 1..].find(close_ch) {
                let inner = text[i + 1..i + 1 + rel].to_ascii_lowercase();
                if NONSPEECH.iter().any(|k| inner.contains(k)) {
                    i += 1 + rel + 1; // skip the whole span, including the closer
                    continue;
                }
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_inline_and_standalone_nonspeech_tags() {
        assert_eq!(
            strip_nonspeech_markers("I hope it is working. [BLANK_AUDIO]"),
            "I hope it is working."
        );
        assert_eq!(
            strip_nonspeech_markers("[Music] This is working. [Music]"),
            "This is working."
        );
        assert_eq!(strip_nonspeech_markers("(dramatic music)"), "");
        assert_eq!(strip_nonspeech_markers("[BLANK_AUDIO]"), "");
    }

    #[test]
    fn keeps_genuine_parentheticals() {
        assert_eq!(
            strip_nonspeech_markers("call me (maybe)"),
            "call me (maybe)"
        );
        assert_eq!(
            strip_nonspeech_markers("the deadline (urgent) is today"),
            "the deadline (urgent) is today"
        );
    }

    #[test]
    fn coverage_and_completeness_follow_mac_rules() {
        let secs = |s: f64| Duration::from_secs_f64(s);
        assert_eq!(capture_coverage(16_000 * 5, secs(5.3)), 1.0);
        let c = capture_coverage(16_000 * 3, secs(5.3));
        assert!((c - 0.6).abs() < 0.01);
        assert_eq!(
            capture_completeness(16_000 * 3, c),
            CaptureCompleteness::Partial
        );
        assert_eq!(
            capture_completeness(16_000, 0.6),
            CaptureCompleteness::Dropped
        );
        assert_eq!(
            capture_completeness(16_000 * 10, 0.3),
            CaptureCompleteness::Dropped
        );
        assert_eq!(
            capture_completeness(100, 0.95),
            CaptureCompleteness::Complete
        );
    }

    #[test]
    fn start_failures_get_accurate_messages() {
        assert_eq!(
            start_failure_message(&CoreError::Audio("no default input device".into())),
            "No microphone signal"
        );
        assert_eq!(
            start_failure_message(&CoreError::Audio("Permission denied".into())),
            "Microphone access needed"
        );
        assert_eq!(
            start_failure_message(&CoreError::Audio("backend exploded".into())),
            "Can't start recording"
        );
    }

    #[test]
    fn memory_pipeline_reports_hits() {
        let mut m = MemoryService::new();
        m.add_term("npm", "NPM");
        m.add_snippet("sig", "— sent via Orttaai");
        let out = MemoryPipeline(m).process("i love npm, sig", None);
        assert_eq!(out.text, "i love NPM, — sent via Orttaai");
        assert_eq!(out.dictionary_hits, vec!["npm"]);
        assert_eq!(out.snippet_hits, vec!["sig"]);
        assert!(!out.polished);
    }

    #[test]
    fn config_from_settings_clamps_and_maps() {
        let mut s = Settings {
            hands_free_silence_stop: false,
            audio_device: Some("  ".into()),
            ..Default::default()
        };
        let c = CoordinatorConfig::from_settings(&s);
        assert!(c.silence_stop.is_none());
        assert!(c.audio_device.is_none());
        assert_eq!(c.max_recording, Duration::from_secs(90));
        s.hands_free_silence_stop = true;
        s.hands_free_silence_secs = 6.0;
        assert_eq!(
            CoordinatorConfig::from_settings(&s).silence_stop,
            Some(Duration::from_secs(6))
        );
    }
}
