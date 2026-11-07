//! Writing completed dictations to the store and fanning out post-save work
//! (semantic indexing, milestones, coach metrics). Each area hooks in via
//! [`after_save`].

use orttaai_core::store::{now_unix, Store, TranscriptionRecord};

/// Best-effort write of a plain dictation (legacy path; the engine now saves
/// full records via [`save_record`]).
#[allow(dead_code)]
pub fn save_transcript(text: &str, duration_ms: i64, source: Option<String>) -> Option<i64> {
    let record = TranscriptionRecord::new(text, source, duration_ms, now_unix());
    save_record(&record)
}

/// Persist a fully populated record; returns its id.
pub fn save_record(record: &TranscriptionRecord) -> Option<i64> {
    match Store::open_default() {
        Ok(store) => match store.insert_transcription(record) {
            Ok(id) => {
                after_save(&store, id);
                Some(id)
            }
            Err(e) => {
                eprintln!("orttaai: failed to persist transcription: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("orttaai: failed to open history store: {e}");
            None
        }
    }
}

/// Post-save hooks. Areas append their own calls here (one line each).
fn after_save(_store: &Store, _id: i64) {
    // == semantic: enqueue for indexing ==
    // == milestones: evaluate unlocks ==
    crate::insights::after_save(_store, _id);
    // == coach: record metrics ==
}
