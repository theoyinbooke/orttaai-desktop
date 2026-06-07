//! End-to-end coordinator tests using mock backends — these run on any OS,
//! including the macOS dev host, and gate the build in CI.

use orttaai_core::audio::MockAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::injection::MockTextInjector;
use orttaai_core::memory::MemoryService;
use orttaai_core::store::{Store, TranscriptionRecord};
use orttaai_core::transcription::MockTranscriber;
use orttaai_core::types::{DecodeOptions, InjectionResult, RecordingState};

fn coordinator_with(
    transcript: &str,
    injector: MockTextInjector,
    memory: MemoryService,
) -> DictationCoordinator {
    DictationCoordinator::new(
        Box::new(MockTranscriber::new(transcript)),
        Box::new(MockAudioCapture::new(1.0)),
        Box::new(injector),
        memory,
        DecodeOptions::default(),
    )
}

#[test]
fn full_loop_injects_memory_applied_transcript() {
    let injector = MockTextInjector::new();
    let log = injector.log();

    let mut memory = MemoryService::new();
    memory.add_term("world", "WORLD");

    let mut coord = coordinator_with("hello world", injector, memory);

    assert_eq!(coord.state(), RecordingState::Idle);
    coord.on_press().unwrap();
    assert_eq!(coord.state(), RecordingState::Recording);

    let outcome = coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::Success);
    assert_eq!(outcome.transcript.as_deref(), Some("hello WORLD"));
    assert_eq!(log.last().as_deref(), Some("hello WORLD"));
    assert_eq!(coord.state(), RecordingState::Idle);
}

#[test]
fn secure_field_blocks_injection() {
    let injector = MockTextInjector::secure();
    let log = injector.log();

    let mut coord = coordinator_with("secret password", injector, MemoryService::new());
    coord.on_press().unwrap();
    let outcome = coord.on_release().unwrap();

    assert_eq!(outcome.result, InjectionResult::BlockedSecureField);
    assert!(outcome.transcript.is_none());
    assert!(
        log.all().is_empty(),
        "nothing should be injected into a secure field"
    );
}

#[test]
fn release_without_press_is_a_noop() {
    let mut coord = coordinator_with("ignored", MockTextInjector::new(), MemoryService::new());
    let outcome = coord.on_release().unwrap();
    assert_eq!(outcome.result, InjectionResult::NoTranscript);
}

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
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].text, "second one"); // newest first
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
    let stats = store.stats().unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.total_words, 3);
    assert_eq!(stats.top_apps[0].app, "Firefox");
}
