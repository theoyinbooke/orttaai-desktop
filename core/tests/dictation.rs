//! End-to-end coordinator tests using mock backends — these run on any OS,
//! including the macOS dev host, and gate the build in CI.

use orttaai_core::audio::{MockAudioCapture, MockAudioFeed};
use orttaai_core::clipboard::MockClipboard;
use orttaai_core::coordinator::{
    CoordinatorConfig, DictationCoordinator, EditRewriter, ManualClock, MemoryPipeline,
};
use orttaai_core::injection::{InjectionLog, InjectionService, MockTextInjector};
use orttaai_core::memory::MemoryService;
use orttaai_core::selection::{MockFieldSelector, SelectionPeek};
use orttaai_core::settings::{InjectionMode, Settings};
use orttaai_core::store::{Store, TranscriptionRecord};
use orttaai_core::transcription::MockTranscriber;
use orttaai_core::types::{
    DecodeOptions, InjectionMethod, InjectionResult, RecordingMode, RecordingState, SessionKind,
};
use orttaai_core::vad::{MockVoiceDetector, SILERO_CHUNK_SAMPLES};
use orttaai_core::CoreError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SR: usize = 16_000;

fn tone(secs: f32, amp: f32) -> Vec<f32> {
    (0..(secs * SR as f32) as usize)
        .map(|i| amp * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / SR as f32).sin())
        .collect()
}

fn silence(secs: f32) -> Vec<f32> {
    vec![0.0; (secs * SR as f32) as usize]
}

struct Rig {
    coord: DictationCoordinator,
    clock: ManualClock,
    log: InjectionLog,
    clip: Arc<MockClipboard>,
    feed: MockAudioFeed,
}

impl Rig {
    fn advance(&mut self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
        self.coord.tick();
    }

    /// Advance in 50 ms ticks, feeding `per_tick` samples each tick.
    fn run(&mut self, ms: u64, per_tick: &[f32]) {
        let mut t = 0;
        while t < ms {
            if !per_tick.is_empty() {
                self.feed.push(per_tick);
            }
            self.advance(50);
            t += 50;
        }
    }
}

fn settings() -> Settings {
    Settings::default()
}

fn rig_with(
    transcript: &str,
    injector: MockTextInjector,
    audio: MockAudioCapture,
    memory: MemoryService,
    s: &Settings,
) -> Rig {
    let clip = Arc::new(MockClipboard::with_text("user's clipboard"));
    let injector = injector.with_clipboard(clip.clone());
    let log = injector.log();
    let feed = audio.feed();
    let clock = ManualClock::new();
    let injection =
        InjectionService::new(Box::new(injector), Box::new(clip.clone())).with_sleep(|_| {});
    let mut coord = DictationCoordinator::new(
        Box::new(MockTranscriber::new(transcript)),
        Box::new(audio),
        injection,
        Box::new(MemoryPipeline(memory)),
        DecodeOptions::default(),
    )
    .with_clock(Arc::new(clock.clone()));
    coord.set_config(CoordinatorConfig::from_settings(s));
    coord.set_focused_app_provider(Box::new(|| Some("TestApp".to_string())));
    Rig {
        coord,
        clock,
        log,
        clip,
        feed,
    }
}

fn rig(transcript: &str, injector: MockTextInjector, memory: MemoryService) -> Rig {
    rig_with(
        transcript,
        injector,
        MockAudioCapture::new(1.0),
        memory,
        &settings(),
    )
}

// ---- Legacy one-shot API ------------------------------------------------------

#[test]
fn full_loop_injects_memory_applied_transcript() {
    let mut memory = MemoryService::new();
    memory.add_term("world", "WORLD");
    let mut r = rig("hello world", MockTextInjector::new(), memory);

    assert_eq!(r.coord.state(), RecordingState::Idle);
    r.coord.on_press().unwrap();
    assert_eq!(r.coord.state(), RecordingState::Recording);

    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(outcome.transcript.as_deref(), Some("hello WORLD"));
    assert_eq!(outcome.raw_text.as_deref(), Some("hello world"));
    assert_eq!(outcome.dictionary_hits, vec!["world"]);
    assert_eq!(outcome.app.as_deref(), Some("TestApp"));
    assert_eq!(outcome.injection_method, Some(InjectionMethod::Typed));
    assert_eq!(r.log.last().as_deref(), Some("hello WORLD"));
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn secure_field_blocks_injection() {
    let mut r = rig(
        "secret password",
        MockTextInjector::secure(),
        MemoryService::new(),
    );
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();

    assert_eq!(outcome.result, InjectionResult::BlockedSecureField);
    assert!(
        outcome.transcript.is_none(),
        "never kept for a password field"
    );
    assert!(r.log.all().is_empty(), "nothing typed into a secure field");
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Can't dictate into password fields")
    );
}

#[test]
fn strict_secure_blocks_when_field_status_unknown() {
    let mut r = rig(
        "my secret",
        MockTextInjector::unknown(),
        MemoryService::new(),
    );
    r.coord.set_strict_secure(true);
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::BlockedSecureField);
    assert_eq!(
        outcome.transcript.as_deref(),
        Some("my secret"),
        "the field may be ordinary text, so the dictation must survive"
    );
    assert!(
        r.log.all().is_empty(),
        "must not type when unsure and strict"
    );
}

#[test]
fn unknown_field_status_types_when_not_strict() {
    let mut r = rig(
        "hello there",
        MockTextInjector::unknown(),
        MemoryService::new(),
    );
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(r.log.last().as_deref(), Some("hello there"));
}

#[test]
fn injection_failure_preserves_transcript_on_clipboard() {
    let mut r = rig("keep me", MockTextInjector::failing(), MemoryService::new());
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();

    assert_eq!(outcome.result, InjectionResult::Failed);
    assert_eq!(outcome.transcript.as_deref(), Some("keep me"));
    assert!(outcome.inject_error.is_some());
    assert_eq!(outcome.injection_method, Some(InjectionMethod::Clipboard));
    assert_eq!(r.clip.get_text_now().as_deref(), Some("keep me"));
    assert_eq!(r.coord.state(), RecordingState::Error);
    r.advance(2000);
    assert_eq!(
        r.coord.state(),
        RecordingState::Idle,
        "error auto-dismisses"
    );
}

#[test]
fn blank_audio_marker_is_not_injected() {
    let mut r = rig(
        "[BLANK_AUDIO]",
        MockTextInjector::new(),
        MemoryService::new(),
    );
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::NoTranscript);
    assert!(r.log.all().is_empty());
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn inline_nonspeech_marker_is_stripped_before_injection() {
    let mut r = rig(
        "hello there [BLANK_AUDIO]",
        MockTextInjector::new(),
        MemoryService::new(),
    );
    r.coord.on_press().unwrap();
    r.coord.on_release().unwrap();
    assert_eq!(r.log.last().as_deref(), Some("hello there"));
}

#[test]
fn release_without_press_is_a_noop() {
    let mut r = rig("ignored", MockTextInjector::new(), MemoryService::new());
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::NoTranscript);
}

/// A transcriber that always fails — used to exercise error-state recovery.
struct FailingTranscriber;
impl orttaai_core::transcription::Transcriber for FailingTranscriber {
    fn load_model(&mut self, _model_id: &str) -> orttaai_core::Result<()> {
        Ok(())
    }
    fn loaded_model(&self) -> Option<&str> {
        Some("failing")
    }
    fn transcribe(&self, _samples: &[f32], _opts: &DecodeOptions) -> orttaai_core::Result<String> {
        Err(CoreError::Transcription("boom".into()))
    }
}

#[test]
fn coordinator_recovers_from_error_state() {
    let mut r = rig("unused", MockTextInjector::new(), MemoryService::new());
    r.coord.replace_transcriber(Box::new(FailingTranscriber));

    r.coord.on_press().unwrap();
    let result = r.coord.on_release();
    assert!(matches!(result, Err(CoreError::Transcription(_))));
    assert_eq!(r.coord.state(), RecordingState::Error);
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Couldn't transcribe. Try again.")
    );

    // A subsequent press recovers and starts a fresh recording.
    r.coord.on_press().unwrap();
    assert_eq!(r.coord.state(), RecordingState::Recording);
}

// ---- Gestures: hold vs tap ------------------------------------------------------

#[test]
fn hold_is_push_to_talk() {
    let mut r = rig("held words", MockTextInjector::new(), MemoryService::new());
    r.coord.handle_hotkey_down();
    assert_eq!(r.coord.state(), RecordingState::Recording);
    r.run(2000, &tone(0.05, 0.2));
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.transcript.as_deref(), Some("held words"));
    assert_eq!(out.kind, SessionKind::Dictation);
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn tap_starts_hands_free_and_next_tap_stops() {
    let mut r = rig("hands free", MockTextInjector::new(), MemoryService::new());
    r.coord.handle_hotkey_down();
    r.advance(200);
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Recording);
    assert_eq!(r.coord.mode(), RecordingMode::HandsFree);

    r.run(3000, &tone(0.05, 0.2));
    r.coord.handle_hotkey_down();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    // The key-up of the stopping tap is ignored.
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.transcript.as_deref(), Some("hands free"));
}

#[test]
fn tap_with_hands_free_disabled_is_a_too_short_hold() {
    let mut s = settings();
    s.hands_free_enabled = false;
    let mut r = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::with_samples(silence(0.2)),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_hotkey_down();
    r.advance(200);
    r.coord.handle_hotkey_up();
    assert_eq!(
        r.coord.state(),
        RecordingState::Idle,
        "too short → discarded"
    );
    assert!(!r.coord.has_pending());
}

#[test]
fn hotkey_is_ignored_while_processing_or_in_error() {
    let mut r = rig("x", MockTextInjector::new(), MemoryService::new());
    r.coord.handle_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    r.coord.handle_hotkey_down();
    assert_eq!(r.coord.state(), RecordingState::Processing);

    let mut bad = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::failing(),
        MemoryService::new(),
        &settings(),
    );
    bad.coord.handle_hotkey_down();
    assert_eq!(bad.coord.state(), RecordingState::Error);
    assert_eq!(
        bad.coord.status().error.as_deref(),
        Some("No microphone signal")
    );
    bad.coord.handle_hotkey_down();
    assert_eq!(
        bad.coord.state(),
        RecordingState::Error,
        "Mac ignores keys while erroring"
    );
    bad.advance(2000);
    assert_eq!(bad.coord.state(), RecordingState::Idle);
}

#[test]
fn right_key_tap_starts_hands_free_and_the_next_tap_inserts() {
    // Hands-free is off: this key has no hold, so it's hands-free anyway.
    let mut s = settings();
    s.hands_free_enabled = false;
    let mut r = rig_with(
        "right control",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_right_key_tap();
    assert_eq!(r.coord.state(), RecordingState::Recording);
    assert_eq!(r.coord.mode(), RecordingMode::HandsFree);
    r.run(1500, &tone(0.05, 0.2));
    r.coord.handle_right_key_tap();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    // A tap while it's being processed does nothing.
    r.coord.handle_right_key_tap();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.transcript.as_deref(), Some("right control"));
    assert_eq!(out.kind, SessionKind::Dictation);
}

#[test]
fn right_key_tap_stops_a_shortcut_recording_and_leaves_edits_alone() {
    // Holding the shortcut, then tapping Right Ctrl: it stops, and the
    // shortcut's release that follows does nothing more.
    let mut r = rig(
        "held then tapped",
        MockTextInjector::new(),
        MemoryService::new(),
    );
    r.coord.handle_hotkey_down();
    r.run(1500, &tone(0.05, 0.2));
    r.coord.handle_right_key_tap();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    r.coord.handle_hotkey_up();
    assert_eq!(
        r.coord.finish_pending().unwrap().transcript.as_deref(),
        Some("held then tapped")
    );

    let mut e = edit_rig(Some("an edit"), Some(Box::new(Shout)));
    e.coord.handle_edit_hotkey_down();
    e.advance(150);
    e.coord.handle_edit_hotkey_up();
    e.coord.handle_right_key_tap();
    assert_eq!(
        e.coord.state(),
        RecordingState::Recording,
        "Right Ctrl never disturbs an edit"
    );
    assert_eq!(e.coord.kind(), SessionKind::Edit);
}

#[test]
fn toggle_records_hands_free_then_stops() {
    let mut r = rig("toggled", MockTextInjector::new(), MemoryService::new());
    r.coord.toggle();
    assert_eq!(r.coord.mode(), RecordingMode::HandsFree);
    r.run(1500, &tone(0.05, 0.2));
    r.coord.toggle();
    assert!(r.coord.has_pending());
    assert_eq!(
        r.coord.finish_pending().unwrap().transcript.as_deref(),
        Some("toggled")
    );
}

// ---- Caps and countdown ----------------------------------------------------------

#[test]
fn push_to_talk_cap_counts_down_and_keeps_audio() {
    let mut s = settings();
    s.max_recording_secs = 30;
    let mut r = rig_with(
        "capped",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_hotkey_down();
    r.run(5000, &tone(0.05, 0.2));
    assert_eq!(r.coord.status().countdown, None);
    r.run(10_000, &tone(0.05, 0.2));
    assert_eq!(r.coord.status().countdown, Some(15));
    r.run(14_000, &tone(0.05, 0.2));
    assert_eq!(r.coord.status().countdown, Some(1));
    r.run(1000, &tone(0.05, 0.2));
    assert_eq!(
        r.coord.state(),
        RecordingState::Processing,
        "cap stops the recording"
    );
    let out = r.coord.finish_pending().unwrap();
    assert!(out.duration_ms >= 29_000);
    assert_eq!(out.transcript.as_deref(), Some("capped"));
}

#[test]
fn hands_free_uses_its_own_cap() {
    let mut s = settings();
    s.max_recording_secs = 10;
    s.hands_free_max_secs = 120;
    s.hands_free_silence_stop = false;
    let mut r = rig_with(
        "long",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_hotkey_down();
    r.advance(100);
    r.coord.handle_hotkey_up();
    r.run(30_000, &tone(0.05, 0.2));
    assert_eq!(
        r.coord.state(),
        RecordingState::Recording,
        "past the push-to-talk cap"
    );
    r.run(70_000, &tone(0.05, 0.2));
    assert_eq!(r.coord.status().countdown, Some(20));
    r.run(20_000, &tone(0.05, 0.2));
    assert_eq!(r.coord.state(), RecordingState::Processing);
}

// ---- Hands-free silence auto-stop -------------------------------------------------

#[test]
fn hands_free_stops_after_silence_with_loudness_fallback() {
    let mut s = settings();
    s.hands_free_silence_secs = 2.0;
    let mut r = rig_with(
        "auto stopped",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_hotkey_down();
    r.advance(100);
    r.coord.handle_hotkey_up();
    r.run(1500, &tone(0.05, 0.3)); // speech
    r.run(1500, &silence(0.05));
    assert_eq!(
        r.coord.state(),
        RecordingState::Recording,
        "1.5 s silence < 2 s window"
    );
    r.run(800, &silence(0.05));
    assert_eq!(r.coord.state(), RecordingState::Processing);
    assert_eq!(
        r.coord.finish_pending().unwrap().transcript.as_deref(),
        Some("auto stopped")
    );
}

#[test]
fn silence_without_speech_never_auto_stops() {
    let mut s = settings();
    s.hands_free_silence_secs = 1.0;
    let mut r = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.toggle();
    r.run(8000, &silence(0.05));
    assert_eq!(
        r.coord.state(),
        RecordingState::Recording,
        "still gathering thoughts"
    );
}

#[test]
fn push_to_talk_ignores_silence() {
    let mut s = settings();
    s.hands_free_silence_secs = 1.0;
    let mut r = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.handle_hotkey_down();
    r.run(1000, &tone(0.05, 0.3));
    r.run(4000, &silence(0.05));
    assert_eq!(r.coord.state(), RecordingState::Recording);
}

#[test]
fn voice_detector_verdicts_decide_over_loudness() {
    let mut s = settings();
    s.hands_free_silence_secs = 1.0;
    let mut r = rig_with(
        "voiced",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    // Loud fan noise throughout, but the detector hears a voice only at first.
    let mut v = MockVoiceDetector::default();
    v.verdicts.extend([true, true, true, true]);
    r.coord.set_voice_detector(Some(Box::new(v)));
    r.coord.toggle();
    let chunk = tone(SILERO_CHUNK_SAMPLES as f32 / SR as f32 / 5.0, 0.3);
    r.run(4000, &chunk);
    assert_eq!(
        r.coord.state(),
        RecordingState::Processing,
        "voice says silence despite noise"
    );
}

#[test]
fn failing_voice_detector_falls_back_to_loudness() {
    let mut s = settings();
    s.hands_free_silence_secs = 1.0;
    let mut r = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &s,
    );
    r.coord.set_voice_detector(Some(Box::new(MockVoiceDetector {
        fail: true,
        ..Default::default()
    })));
    r.coord.toggle();
    r.run(1000, &tone(0.05, 0.3));
    r.run(1500, &silence(0.05));
    assert_eq!(r.coord.state(), RecordingState::Processing);
}

// ---- Coverage -------------------------------------------------------------------

#[test]
fn mostly_missing_audio_is_dropped() {
    let mut r = rig_with(
        "x",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &settings(),
    );
    r.coord.handle_hotkey_down();
    r.feed.push(&tone(0.5, 0.2));
    r.run(5000, &[]);
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Error);
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Audio was dropped. Please dictate again.")
    );
}

#[test]
fn partial_audio_is_kept_and_reported() {
    let mut r = rig_with(
        "most of it",
        MockTextInjector::new(),
        MockAudioCapture::new(0.0),
        MemoryService::new(),
        &settings(),
    );
    r.coord.handle_hotkey_down();
    r.feed.push(&tone(3.5, 0.2));
    r.run(5000, &[]);
    r.coord.handle_hotkey_up();
    let out = r.coord.finish_pending().unwrap();
    assert!(out.lost_audio);
    assert_eq!(r.log.last().as_deref(), Some("most of it"));
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Some audio was lost. The rest was inserted.")
    );
}

// ---- Injection modes through the coordinator ----------------------------------------

#[test]
fn paste_mode_restores_clipboard_and_records_method() {
    let mut s = settings();
    s.injection_mode = InjectionMode::Paste;
    let mut r = rig_with(
        "pasted words",
        MockTextInjector::new(),
        MockAudioCapture::new(1.0),
        MemoryService::new(),
        &s,
    );
    r.coord.on_press().unwrap();
    let out = r.coord.on_release().unwrap();
    assert_eq!(out.injection_method, Some(InjectionMethod::Paste));
    assert_eq!(r.log.last().as_deref(), Some("pasted words"));
    assert_eq!(r.clip.get_text_now().as_deref(), Some("user's clipboard"));
}

#[test]
fn vocabulary_bias_prompt_reaches_the_transcriber_for_speech_only() {
    let transcriber = MockTranscriber::new("Orttaai rocks");
    let calls = transcriber.calls();
    let clip = Arc::new(MockClipboard::new());
    let injection = InjectionService::new(
        Box::new(MockTextInjector::new().with_clipboard(clip.clone())),
        Box::new(clip),
    )
    .with_sleep(|_| {});
    let mut coord = DictationCoordinator::new(
        Box::new(transcriber),
        Box::new(MockAudioCapture::with_samples(tone(1.0, 0.3))),
        injection,
        Box::new(MemoryPipeline(MemoryService::new())),
        DecodeOptions::default(),
    );
    coord.set_vocabulary(vec!["Orttaai".into(), "Kubernetes".into()]);
    coord.on_press().unwrap();
    coord.on_release().unwrap();
    let prompt = calls.lock().unwrap()[0].initial_prompt.clone();
    assert_eq!(prompt.as_deref(), Some("Kubernetes, Orttaai."));

    // Turning vocabulary bias off drops the prompt.
    let mut cfg = coord.config().clone();
    cfg.vocabulary_bias = false;
    coord.set_config(cfg);
    coord.on_press().unwrap();
    coord.on_release().unwrap();
    assert!(calls.lock().unwrap()[1].initial_prompt.is_none());
}

#[test]
fn silent_audio_gets_no_bias_prompt() {
    let transcriber = MockTranscriber::new("x");
    let calls = transcriber.calls();
    let clip = Arc::new(MockClipboard::new());
    let injection = InjectionService::new(
        Box::new(MockTextInjector::new().with_clipboard(clip.clone())),
        Box::new(clip),
    )
    .with_sleep(|_| {});
    let mut coord = DictationCoordinator::new(
        Box::new(transcriber),
        Box::new(MockAudioCapture::new(1.0)),
        injection,
        Box::new(MemoryPipeline(MemoryService::new())),
        DecodeOptions::default(),
    );
    coord.set_vocabulary(vec!["Orttaai".into()]);
    coord.on_press().unwrap();
    coord.on_release().unwrap();
    assert!(calls.lock().unwrap()[0].initial_prompt.is_none());
}

// ---- Voice edit ------------------------------------------------------------------

struct Shout;
impl EditRewriter for Shout {
    fn rewrite(&self, selection: &str, _instruction: &str) -> orttaai_core::Result<String> {
        Ok(selection.to_uppercase())
    }
}

struct Refuse;
impl EditRewriter for Refuse {
    fn rewrite(&self, _s: &str, _i: &str) -> orttaai_core::Result<String> {
        Err(CoreError::Llm("The local model is not running".into()))
    }
}

struct Same;
impl EditRewriter for Same {
    fn rewrite(&self, s: &str, _i: &str) -> orttaai_core::Result<String> {
        Ok(format!("  {s} "))
    }
}

fn edit_rig(selection: Option<&str>, rewriter: Option<Box<dyn EditRewriter>>) -> Rig {
    let mut s = settings();
    s.edit_commands_enabled = true;
    let mut inj = MockTextInjector::new();
    if let Some(sel) = selection {
        inj = inj.with_selection(sel);
    }
    let mut r = rig_with(
        "make it shout",
        inj,
        MockAudioCapture::new(1.5),
        MemoryService::new(),
        &s,
    );
    r.coord.set_rewriter(rewriter);
    r
}

#[test]
fn edit_session_replaces_the_selection() {
    let mut r = edit_rig(Some("hello there"), Some(Box::new(Shout)));
    r.coord.handle_edit_hotkey_down();
    assert_eq!(r.coord.kind(), SessionKind::Edit);
    r.run(1500, &[]);
    r.coord.handle_edit_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.kind, SessionKind::Edit);
    assert_eq!(out.edit_instruction.as_deref(), Some("make it shout"));
    assert_eq!(out.transcript.as_deref(), Some("HELLO THERE"));
    assert_eq!(r.log.last().as_deref(), Some("HELLO THERE"));
    assert_eq!(
        r.clip.get_text_now().as_deref(),
        Some("user's clipboard"),
        "copy restored"
    );
    let rec = out.to_record(1_700_000_000).unwrap();
    assert_eq!(rec.entry_kind, "edit");
    assert_eq!(rec.edit_instruction.as_deref(), Some("make it shout"));
}

#[test]
fn edit_without_selection_never_records() {
    let mut r = edit_rig(None, Some(Box::new(Shout)));
    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Error);
    assert_eq!(r.coord.status().error.as_deref(), Some("Select text first"));
    assert!(!r.coord.has_pending());
}

#[test]
fn edit_failure_leaves_selection_untouched() {
    let mut r = edit_rig(Some("keep this"), Some(Box::new(Refuse)));
    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    let out = r.coord.finish_pending().unwrap();
    assert!(out.transcript.is_none());
    assert!(r.log.all().is_empty(), "never invent text");
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Couldn't edit: The local model is not running")
    );
}

#[test]
fn unchanged_edit_says_so() {
    let mut r = edit_rig(Some("already fine"), Some(Box::new(Same)));
    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    r.coord.finish_pending().unwrap();
    assert!(r.log.all().is_empty());
    assert_eq!(r.coord.status().error.as_deref(), Some("No changes needed"));
}

#[test]
fn edit_needs_a_rewriter_and_the_setting() {
    let mut r = edit_rig(Some("x"), None);
    r.coord.handle_edit_hotkey_down();
    assert_eq!(r.coord.state(), RecordingState::Error);
    assert!(r.coord.status().error.unwrap().contains("local AI model"));

    let mut no_edit = settings();
    no_edit.edit_commands_enabled = false;
    let mut off = rig_with(
        "x",
        MockTextInjector::new().with_selection("x"),
        MockAudioCapture::new(1.0),
        MemoryService::new(),
        &no_edit,
    );
    off.coord.set_rewriter(Some(Box::new(Shout)));
    off.coord.handle_edit_hotkey_down();
    assert_eq!(
        off.coord.state(),
        RecordingState::Idle,
        "edit commands off → inert"
    );
}

#[test]
fn edit_tap_goes_hands_free_and_dictation_key_is_inert() {
    let mut r = edit_rig(Some("tap edit"), Some(Box::new(Shout)));
    r.coord.handle_edit_hotkey_down();
    r.advance(150);
    r.coord.handle_edit_hotkey_up();
    assert_eq!(r.coord.mode(), RecordingMode::HandsFree);
    assert_eq!(r.coord.kind(), SessionKind::Edit);
    r.coord.handle_hotkey_down();
    r.coord.handle_hotkey_up();
    assert_eq!(
        r.coord.state(),
        RecordingState::Recording,
        "dictation key never disturbs an edit"
    );
    r.run(1500, &[]);
    r.coord.handle_edit_hotkey_down();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.transcript.as_deref(), Some("TAP EDIT"));
}

// ---- Editing from the dictation shortcut, and the last dictation -----------------------

struct Offline;
impl EditRewriter for Offline {
    fn rewrite(&self, _s: &str, _i: &str) -> orttaai_core::Result<String> {
        Err(CoreError::Llm("connection refused".into()))
    }
}

/// A rig whose transcript is `words`, with a scripted accessibility selector.
fn selection_rig(
    words: &str,
    rewriter: Option<Box<dyn EditRewriter>>,
    s: &Settings,
) -> (Rig, MockFieldSelector) {
    let mut r = rig_with(
        words,
        MockTextInjector::new(),
        MockAudioCapture::new(1.5),
        MemoryService::new(),
        s,
    );
    r.coord.set_rewriter(rewriter);
    let selector = MockFieldSelector::new();
    r.coord.set_field_selector(Arc::new(selector.clone()));
    (r, selector)
}

/// Hold the dictation shortcut for a second and process the recording.
fn dictate(r: &mut Rig) -> orttaai_core::coordinator::DictationOutcome {
    r.coord.handle_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_hotkey_up();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    r.coord.finish_pending().unwrap()
}

fn selected(text: &str) -> SelectionPeek {
    SelectionPeek::Selected {
        text: text.into(),
        single_line: false,
    }
}

#[test]
fn instruction_on_the_dictation_key_edits_the_selection() {
    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Shout)), &settings());
    sel.set_peek(selected("hello there"));
    let out = dictate(&mut r);
    assert_eq!(out.kind, SessionKind::Edit);
    assert_eq!(out.edit_instruction.as_deref(), Some("Make this shorter."));
    assert_eq!(out.transcript.as_deref(), Some("HELLO THERE"));
    assert_eq!(r.log.all(), vec!["HELLO THERE"]);
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn ordinary_words_replace_the_selection_as_before() {
    let (mut r, sel) = selection_rig(
        "Thanks, see you Monday.",
        Some(Box::new(Shout)),
        &settings(),
    );
    sel.set_peek(selected("old sentence"));
    let out = dictate(&mut r);
    assert_eq!(out.kind, SessionKind::Dictation);
    assert_eq!(r.log.last().as_deref(), Some("Thanks, see you Monday."));
}

#[test]
fn one_line_fields_are_never_edited_from_the_dictation_key() {
    // An address bar selects its URL when clicked.
    let (mut r, sel) = selection_rig("Translate to French.", Some(Box::new(Shout)), &settings());
    sel.set_peek(SelectionPeek::Selected {
        text: "https://example.com".into(),
        single_line: true,
    });
    let out = dictate(&mut r);
    assert_eq!(out.kind, SessionKind::Dictation);
    assert_eq!(r.log.last().as_deref(), Some("Translate to French."));
}

#[test]
fn without_a_model_the_dictation_key_types_as_before() {
    let (mut r, sel) = selection_rig("Make this shorter.", None, &settings());
    sel.set_peek(selected("old sentence"));
    dictate(&mut r);
    assert_eq!(sel.peeks(), 0, "no model: nothing is read");
    assert_eq!(r.log.last().as_deref(), Some("Make this shorter."));

    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Offline)), &settings());
    sel.set_peek(selected("old sentence"));
    let out = dictate(&mut r);
    assert_eq!(out.kind, SessionKind::Dictation);
    assert_eq!(r.log.last().as_deref(), Some("Make this shorter."));
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn failed_edit_keeps_the_words_in_history_and_types_nothing() {
    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Refuse)), &settings());
    sel.set_peek(selected("keep this"));
    let out = dictate(&mut r);
    assert!(r.log.all().is_empty(), "the selection is left as it was");
    assert_eq!(r.coord.state(), RecordingState::Error);
    let rec = out
        .to_record(1_700_000_000)
        .expect("the words reach History");
    assert_eq!(rec.text, "Make this shorter.");
    assert_eq!(rec.entry_kind, "dictation");
}

#[test]
fn edit_is_not_pasted_when_the_selection_moved_away() {
    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Shout)), &settings());
    sel.set_peek(selected("hello there"));
    sel.set_current(Some("")); // a click cleared it while the instruction was spoken
    dictate(&mut r);
    assert!(r.log.all().is_empty());
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("The selection changed, so nothing was replaced")
    );
}

#[test]
fn dictation_key_editing_needs_english_the_setting_and_no_terminal() {
    let mut french = settings();
    french.dictation_language = "fr".into();
    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Shout)), &french);
    sel.set_peek(selected("x y"));
    dictate(&mut r);
    assert_eq!(sel.peeks(), 0);

    let mut off = settings();
    off.edit_with_dictation_key = false;
    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Shout)), &off);
    dictate(&mut r);
    assert_eq!(sel.peeks(), 0);

    let (mut r, sel) = selection_rig("Make this shorter.", Some(Box::new(Shout)), &settings());
    r.coord
        .set_focused_app_provider(Box::new(|| Some("WindowsTerminal".to_string())));
    dictate(&mut r);
    assert_eq!(sel.peeks(), 0, "terminals are left alone");
}

#[test]
fn edit_key_with_nothing_selected_edits_the_last_dictation() {
    let (mut r, sel) = selection_rig("see you on monday", Some(Box::new(Shout)), &settings());
    dictate(&mut r);
    assert_eq!(r.log.last().as_deref(), Some("see you on monday"));
    sel.set_selectable(true);

    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    assert_eq!(sel.selected(), vec!["see you on monday"]);
    let out = r.coord.finish_pending().unwrap();
    assert_eq!(out.kind, SessionKind::Edit);
    assert_eq!(r.log.last().as_deref(), Some("SEE YOU ON MONDAY"));
    assert!(sel.deselected().is_empty());
}

#[test]
fn failed_edit_of_the_last_dictation_puts_the_caret_back() {
    let (mut r, sel) = selection_rig("see you on monday", Some(Box::new(Refuse)), &settings());
    dictate(&mut r);
    sel.set_selectable(true);
    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    r.coord.finish_pending().unwrap();
    assert_eq!(sel.deselected(), vec!["see you on monday"]);
}

#[test]
fn the_last_dictation_is_forgotten_after_ten_minutes_or_a_new_dictation() {
    let (mut r, sel) = selection_rig("see you on monday", Some(Box::new(Shout)), &settings());
    dictate(&mut r);
    sel.set_selectable(true);
    r.advance(601_000);
    r.coord.handle_edit_hotkey_down();
    r.run(1000, &[]);
    r.coord.handle_edit_hotkey_up();
    assert_eq!(r.coord.status().error.as_deref(), Some("Select text first"));
    assert!(sel.selected().is_empty());
}

#[test]
fn right_shift_tap_records_an_edit_until_the_next_tap() {
    let (mut r, sel) = selection_rig("see you on monday", Some(Box::new(Shout)), &settings());
    dictate(&mut r);
    sel.set_selectable(true);
    r.coord.handle_right_shift_tap();
    assert_eq!(r.coord.kind(), SessionKind::Edit);
    assert_eq!(r.coord.mode(), RecordingMode::HandsFree);
    r.run(1000, &[]);
    r.coord.handle_right_shift_tap();
    assert_eq!(r.coord.state(), RecordingState::Processing);
    r.coord.finish_pending().unwrap();
    assert_eq!(r.log.last().as_deref(), Some("SEE YOU ON MONDAY"));
}

#[test]
fn right_shift_tap_never_disturbs_a_dictation_and_needs_editing_on() {
    let (mut r, _sel) = selection_rig("hello", Some(Box::new(Shout)), &settings());
    r.coord.handle_right_key_tap();
    r.coord.handle_right_shift_tap();
    assert_eq!(r.coord.kind(), SessionKind::Dictation);
    assert_eq!(r.coord.state(), RecordingState::Recording);

    let mut off = settings();
    off.edit_commands_enabled = false;
    let (mut r, _sel) = selection_rig("hello", Some(Box::new(Shout)), &off);
    r.coord.handle_right_shift_tap();
    assert_eq!(r.coord.state(), RecordingState::Idle);
}

#[test]
fn right_shift_tap_with_nothing_to_edit_never_records() {
    let (mut r, _sel) = selection_rig("hello", Some(Box::new(Shout)), &settings());
    r.coord.handle_right_shift_tap();
    assert_eq!(r.coord.state(), RecordingState::Error);
    assert_eq!(r.coord.status().error.as_deref(), Some("Select text first"));
    assert!(!r.coord.has_pending());
}

// ---- Records + status ---------------------------------------------------------------

#[test]
fn outcome_maps_to_a_full_history_record() {
    let mut r = rig(
        "hello there friend",
        MockTextInjector::new(),
        MemoryService::new(),
    );
    r.coord.handle_hotkey_down();
    r.run(1500, &tone(0.05, 0.2));
    r.coord.handle_hotkey_up();
    assert!(r.coord.status().estimate_secs.unwrap() >= 1);
    let out = r.coord.finish_pending().unwrap();
    let rec = out.to_record(1_700_000_000).unwrap();
    assert_eq!(rec.text, "hello there friend");
    assert_eq!(rec.raw_text.as_deref(), Some("hello there friend"));
    assert_eq!(rec.app.as_deref(), Some("TestApp"));
    assert_eq!(rec.audio_device.as_deref(), Some("mock-default"));
    assert_eq!(rec.injection_method.as_deref(), Some("typed"));
    assert_eq!(rec.entry_kind, "dictation");
    assert_eq!(rec.word_count, 3);
    assert!(rec.duration_ms >= 1500);
    assert_eq!(rec.model_id, "mock");

    let store = Store::open_in_memory().unwrap();
    let id = store.insert_transcription(&rec).unwrap();
    let back = store.get_transcription(id).unwrap().unwrap();
    assert_eq!(back.injection_method.as_deref(), Some("typed"));
}

#[test]
fn status_reports_elapsed_mode_and_target() {
    let mut r = rig("x", MockTextInjector::new(), MemoryService::new());
    r.coord.toggle();
    r.run(2100, &[]);
    let st = r.coord.status();
    assert_eq!(st.elapsed_secs, 2);
    assert_eq!(st.mode, RecordingMode::HandsFree);
    assert_eq!(st.target_app.as_deref(), Some("TestApp"));
}

#[test]
fn reset_audio_only_when_idle() {
    let mut r = rig("x", MockTextInjector::new(), MemoryService::new());
    r.coord.toggle();
    assert!(r.coord.reset_audio().is_err());
    r.coord.cancel();
    assert!(r.coord.reset_audio().is_ok());
}

// ---- Memory + store (unchanged behaviour) ---------------------------------------------

#[test]
fn memory_preserves_punctuation_and_is_case_insensitive() {
    let mut memory = MemoryService::new();
    memory.add_term("npm", "NPM");
    memory.add_snippet("addr", "123 Main St");

    assert_eq!(memory.apply("i love Npm."), "i love NPM.");
    assert_eq!(memory.apply("my addr, please"), "my 123 Main St, please");
    assert_eq!(memory.apply("nothing here"), "nothing here");
}

#[test]
fn store_round_trips_records() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.count().unwrap(), 0);
    let id = store
        .insert_transcription(&TranscriptionRecord::new(
            "hello there",
            Some("Firefox".into()),
            1200,
            1_700_000_000,
        ))
        .unwrap();
    assert!(id > 0);
    store
        .insert_transcription(&TranscriptionRecord::new(
            "second one",
            None,
            800,
            1_700_000_100,
        ))
        .unwrap();
    assert_eq!(store.count().unwrap(), 2);
    let recent = store.recent(10).unwrap();
    assert_eq!(recent[0].text, "second one");
    assert_eq!(recent[1].word_count, 2);
}

#[test]
fn memory_persistence_and_stats() {
    let store = Store::open_in_memory().unwrap();
    store.add_memory("dictionary", "npm", "NPM").unwrap();
    store.add_memory("snippet", "addr", "123 Main St").unwrap();
    assert_eq!(store.list_memory().unwrap().len(), 2);
    let service = store.load_memory_service().unwrap();
    assert_eq!(service.apply("i use npm daily"), "i use NPM daily");
    store
        .insert_transcription(&TranscriptionRecord::new(
            "hello there world",
            Some("Firefox".into()),
            1000,
            1_700_000_000,
        ))
        .unwrap();
    let stats = store.stats(None).unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.total_words, 3);
    assert_eq!(stats.top_apps[0].app, "Firefox");
}

/// Small helper so tests read the mock clipboard without the trait in scope.
trait ClipNow {
    fn get_text_now(&self) -> Option<String>;
}
impl ClipNow for Arc<MockClipboard> {
    fn get_text_now(&self) -> Option<String> {
        use orttaai_core::clipboard::Clipboard;
        self.get_text().unwrap()
    }
}

#[allow(dead_code)]
fn _assert_send(_: &Mutex<DictationCoordinator>) {}

// ---- Target window (Mac TextInjectionService waitForReturn) --------------------

use orttaai_core::clipboard::Clipboard as _;
use orttaai_core::focus::ScriptedWindowFocus;

/// A rig whose dictation starts in window 1, then focus moves to window 2.
fn moved_focus(
    activation_works: bool,
    returns_after: Option<usize>,
) -> (Rig, Arc<ScriptedWindowFocus>) {
    let mut r = rig("hello world", MockTextInjector::new(), MemoryService::new());
    let mut windows = ScriptedWindowFocus::new(Some(1));
    windows.activation_works = activation_works;
    windows.returns_after = returns_after.map(|n| (1, n));
    let windows = Arc::new(windows);
    r.coord.set_window_focus(Box::new(windows.clone()));
    r.coord.on_press().unwrap();
    *windows.front.lock().unwrap() = Some(2);
    (r, windows)
}

#[test]
fn text_goes_back_to_the_window_dictation_started_in() {
    let (mut r, windows) = moved_focus(true, None);
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(r.log.last().as_deref(), Some("hello world"));
    assert_eq!(
        *windows.front.lock().unwrap(),
        Some(1),
        "brought back first"
    );
}

#[test]
fn waits_for_the_window_to_come_back() {
    let (mut r, windows) = moved_focus(false, Some(20));
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(r.log.last().as_deref(), Some("hello world"));
    assert!(windows.waits.load(std::sync::atomic::Ordering::SeqCst) >= 20);
}

#[test]
fn never_types_into_another_window() {
    let (mut r, _windows) = moved_focus(false, None);
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Failed);
    assert!(
        r.log.all().is_empty(),
        "nothing typed into the wrong window"
    );
    assert_eq!(outcome.injection_method, Some(InjectionMethod::Clipboard));
    assert_eq!(
        r.clip.get_text().unwrap().as_deref(),
        Some("hello world"),
        "left on the clipboard"
    );
    assert_eq!(
        outcome.transcript.as_deref(),
        Some("hello world"),
        "kept in History"
    );
    assert_eq!(
        r.coord.status().error.as_deref(),
        Some("Couldn't insert text. Press Ctrl+V to paste it.")
    );
}

/// Orttaai's own window 7 (Setup's Quick Test, Chat AI): it takes text itself.
struct OwnWindow(Mutex<Vec<String>>);

impl orttaai_core::focus::WindowFocus for OwnWindow {
    fn current(&self) -> Option<u64> {
        Some(7)
    }
    fn activate(&self, _window: u64) -> bool {
        true
    }
    fn insert_into_own(&self, window: u64, text: &str) -> bool {
        self.0.lock().unwrap().push(format!("{window}:{text}"));
        window == 7
    }
}

#[test]
fn a_dictation_into_orttaais_own_window_is_handed_to_it_not_typed() {
    let mut r = rig(
        "hello from orttaai",
        MockTextInjector::new(),
        MemoryService::new(),
    );
    let own = Arc::new(OwnWindow(Mutex::new(vec![])));
    r.coord.set_window_focus(Box::new(own.clone()));
    r.coord.on_press().unwrap();
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(
        *own.0.lock().unwrap(),
        vec!["7:hello from orttaai".to_string()]
    );
    assert!(r.log.all().is_empty(), "no keystrokes into ourselves");
}

// ---- Decoding while speaking (Mac live session) ---------------------------------

/// Records the length of every decode and names each result after its turn.
struct PieceCounter(Arc<Mutex<Vec<usize>>>);

impl orttaai_core::transcription::Transcriber for PieceCounter {
    fn load_model(&mut self, _id: &str) -> orttaai_core::Result<()> {
        Ok(())
    }
    fn loaded_model(&self) -> Option<&str> {
        Some("pieces")
    }
    fn transcribe(&self, samples: &[f32], _o: &DecodeOptions) -> orttaai_core::Result<String> {
        let mut calls = self.0.lock().unwrap();
        calls.push(samples.len());
        Ok(format!("part{}", calls.len()))
    }
    fn live_mode(&self) -> orttaai_core::live::LiveMode {
        orttaai_core::live::LiveMode::Clips
    }
}

fn live_rig(live: bool) -> (Rig, Arc<Mutex<Vec<usize>>>) {
    let mut s = settings();
    s.vocabulary_bias = false;
    let mut r = rig_with(
        "unused",
        MockTextInjector::new(),
        MockAudioCapture::new(1.0),
        MemoryService::new(),
        &s,
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    r.coord
        .replace_transcriber(Box::new(PieceCounter(calls.clone())));
    let mut config = CoordinatorConfig::from_settings(&s);
    config.live_decoding = live;
    r.coord.set_config(config);
    (r, calls)
}

#[test]
fn speech_before_a_pause_is_decoded_while_still_recording() {
    let (mut r, calls) = live_rig(true);
    r.coord.on_press().unwrap();
    r.feed.push(&tone(3.0, 0.2));
    r.feed.push(&silence(0.8));
    r.advance(300);
    // The first piece decodes on its own thread while recording goes on.
    for _ in 0..200 {
        if !calls.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(calls.lock().unwrap().len(), 1, "decoded before release");
    r.feed.push(&tone(2.0, 0.2));
    let outcome = r.coord.on_release().unwrap();

    assert_eq!(outcome.transcript.as_deref(), Some("part1 part2"));
    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        2,
        "the first piece plus the tail, never everything again"
    );
    let first = calls[0] as f32 / SR as f32;
    let tail = calls[1] as f32 / SR as f32;
    assert!((3.0..3.5).contains(&first), "first piece {first}s");
    assert!((2.0..2.6).contains(&tail), "tail {tail}s");
}

fn wait_for_calls(calls: &Arc<Mutex<Vec<usize>>>, n: usize) {
    for _ in 0..400 {
        if calls.lock().unwrap().len() >= n {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_decode_started_at_the_pause_is_used_when_only_silence_follows() {
    // Without live pieces (Express, or Whisper with live decoding off), the
    // whole recording is decoded once the speaker pauses, and that decode
    // stands in for the one at release.
    let (mut r, calls) = live_rig(false);
    r.coord.on_press().unwrap();
    r.feed.push(&tone(3.0, 0.2));
    r.feed.push(&silence(0.8));
    r.advance(300);
    wait_for_calls(&calls, 1);
    assert_eq!(calls.lock().unwrap().len(), 1, "decoded before release");
    r.feed.push(&silence(0.4));
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.transcript.as_deref(), Some("part1"));
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "nothing decoded again at release"
    );
}

#[test]
fn speech_after_the_pause_decodes_the_whole_recording_at_release() {
    let (mut r, calls) = live_rig(false);
    r.coord.on_press().unwrap();
    r.feed.push(&tone(3.0, 0.2));
    r.feed.push(&silence(0.8));
    r.advance(300);
    wait_for_calls(&calls, 1);
    r.feed.push(&tone(2.0, 0.2));
    let outcome = r.coord.on_release().unwrap();
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "the early decode, then all of it");
    assert_eq!(outcome.transcript.as_deref(), Some("part2"));
    let whole = calls[1] as f32 / SR as f32;
    assert!((5.7..6.0).contains(&whole), "whole recording {whole}s");
}

#[test]
fn a_short_last_stretch_is_decoded_at_the_pause_too() {
    // With live pieces, the stretch too short for a piece of its own is
    // decoded at the pause and reused at release.
    let (mut r, calls) = live_rig(true);
    r.coord.on_press().unwrap();
    r.feed.push(&tone(1.0, 0.2));
    r.feed.push(&silence(0.6));
    r.advance(300);
    wait_for_calls(&calls, 1);
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "the short stretch, before release"
    );
    r.feed.push(&silence(0.3));
    let outcome = r.coord.on_release().unwrap();
    assert_eq!(outcome.transcript.as_deref(), Some("part1"));
    assert_eq!(calls.lock().unwrap().len(), 1);
}
