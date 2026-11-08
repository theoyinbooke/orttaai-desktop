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

/// Tries before a history save gives up, and the wait between them (Mac
/// `historySaveAttempts` / `historySaveRetryDelayNs`): a busy or locked
/// database usually frees up within a moment.
const SAVE_ATTEMPTS: u32 = 3;
const SAVE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Persist a fully populated record; returns its id, or `None` when every
/// attempt failed.
pub fn save_record(record: &TranscriptionRecord) -> Option<i64> {
    for attempt in 1..=SAVE_ATTEMPTS {
        let saved = Store::open_default().and_then(|store| {
            let id = store.insert_transcription(record)?;
            Ok((store, id))
        });
        match saved {
            Ok((store, id)) => {
                after_save(&store, id);
                return Some(id);
            }
            Err(e) => {
                tracing::warn!("history save attempt {attempt} failed: {e}");
                if attempt < SAVE_ATTEMPTS {
                    std::thread::sleep(SAVE_RETRY_DELAY);
                }
            }
        }
    }
    None
}

/// Post-save hooks. Areas append their own calls here (one line each).
fn after_save(_store: &Store, _id: i64) {
    // == semantic: enqueue for indexing ==
    crate::semantic::after_save(_store, _id);
    // == milestones: evaluate unlocks ==
    crate::insights::after_save(_store, _id);
    // == coach: record metrics ==
    orttaai_core::coach::after_save(_store, _id);
}
