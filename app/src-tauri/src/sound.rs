//! Sound anchors in the app: which anchors the engine listens with, and
//! learning from what you do to a dictation after it was typed
//! (`orttaai_core::sound`, `docs/sound-anchors.md`).
//!
//! The anchors ride on Express's Word Boost: its model scores the sound, so
//! they are on while Express dictates in English with Word Boost on and its
//! model installed. Otherwise everything works as it did before them.

use orttaai_core::injection::edit_watch::{EditCheckOutcome as Watched, EditObservation};
use orttaai_core::settings::Settings;
use orttaai_core::sound::learn::{self, Learned};
use orttaai_core::sound::{guesses, SoundAnchorBook, SoundMemory};
use orttaai_core::store::Store;
use orttaai_core::text::learning::dictation_edits;
use orttaai_core::transcription::express;
use std::path::PathBuf;
use std::sync::Arc;

/// Whether dictation judges words by sound with these settings.
pub fn active(settings: &Settings) -> bool {
    settings.express_english() && settings.express_word_boost && express::word_boost_installed()
}

fn path() -> Option<PathBuf> {
    Store::data_dir().ok().map(|d| d.join(learn::FILE_NAME))
}

/// The active dictionary's words (single words, each once).
fn dictionary_words(settings: &Settings) -> Vec<String> {
    if !settings.dictionary_enabled {
        return Vec::new();
    }
    let Ok(store) = Store::open_default() else {
        return Vec::new();
    };
    let mut words: Vec<String> = Vec::new();
    for entry in store.list_dictionary().unwrap_or_default() {
        let target = entry.target.trim();
        if entry.is_active
            && !target.is_empty()
            && !target.contains(' ')
            && !words.iter().any(|w| w.eq_ignore_ascii_case(target))
        {
            words.push(target.to_string());
        }
    }
    words
}

/// The anchors dictation listens with: what your fixes taught, for words
/// still in the dictionary, plus starting guesses. Empty while the sound
/// check is off. Works out guesses for new words, so call it when memory or
/// settings change, never while a dictation waits.
pub fn book(settings: &Settings) -> Arc<SoundAnchorBook> {
    if !active(settings) {
        return Arc::default();
    }
    let stored = path()
        .map(|p| SoundAnchorBook::load(&p))
        .unwrap_or_default();
    Arc::new(learn::active_book(&stored, &dictionary_words(settings)))
}

/// A word left the dictionary: what was learned about its sound goes too.
pub fn forget_unless_listed(term: &str) {
    let Some(path) = path() else {
        return;
    };
    let still_listed = Store::open_default()
        .and_then(|s| s.list_dictionary())
        .map(|entries| {
            entries
                .iter()
                .any(|e| e.target.trim().eq_ignore_ascii_case(term.trim()))
        })
        .unwrap_or(true);
    if !still_listed {
        learn::update_stored(&path, |book| book.forget(term.trim()));
    }
}

/// Reset App Data: the anchors go with the rest.
pub fn reset() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Learn from one watched dictation: fixes to your words by sound, fixes
/// back, and the heard words you left as they were. Returns what was
/// learned; its `handled` fixes must not become dictionary pairs.
pub fn learn_from_watch(
    memory: Option<Arc<SoundMemory>>,
    settings: &Settings,
    observation: &EditObservation,
) -> Learned {
    // Only a dictation the engine judged by sound has kept sound: without
    // one (Whisper, or Express that fell back to it), fixes go to the text
    // learner as they always did.
    if !active(settings) || observation.recording.is_none() {
        return Learned::default();
    }
    let Some(path) = path() else {
        return Learned::default();
    };
    // What happened to the words is known only when the field could be read
    // at both ends of the watch.
    let edits = match (observation.outcome, &observation.before, &observation.after) {
        (Watched::Edited | Watched::NoChange, Some(before), Some(after)) => {
            dictation_edits(&observation.inserted, before, None, after)
        }
        _ => None,
    };
    let Some(edits) = edits else {
        return Learned::default();
    };
    let recording = observation
        .recording
        .and_then(|id| memory.as_ref().and_then(|m| m.get(id)));
    let dictionary = dictionary_words(settings);
    let guessed = SoundAnchorBook::new(guesses::anchors(&dictionary));
    let learned = learn::update_stored(&path, |book| {
        learn::learn(
            book,
            &guessed,
            &dictionary,
            recording.as_ref(),
            Some(&edits),
        )
    });
    if learned.anything() {
        // Counts, never words.
        tracing::info!(
            "sound anchors: learned from a dictation ({} said, {} fixed back, {} kept)",
            learned.said,
            learned.fixed_back,
            learned.kept
        );
    }
    learned
}
