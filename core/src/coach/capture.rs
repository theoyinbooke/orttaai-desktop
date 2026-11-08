//! Practice capture: while a rep or a spoken scene turn is waiting, the next
//! dictation is handed to Coach instead of being typed into the focused app
//! or saved to History (the Mac's `startPracticeRecording`).
//!
//! **Engine contract.** After a dictation has been transcribed (and polished,
//! when [`practice_wants_polish`] says so) and *before* it is injected or
//! persisted, the engine calls:
//!
//! ```ignore
//! let capture = orttaai_core::coach::PracticeCapture { raw, text, duration_ms };
//! if orttaai_core::coach::take_practice(capture) {
//!     // Coach has it: don't type it, don't save it, don't emit `transcript`.
//! }
//! ```
//!
//! [`practice_armed`] can be checked earlier (for example to skip Polish for
//! a scene turn). `take_practice` is the only call that matters: it returns
//! `false` when nothing is armed, so normal dictation is untouched.

use std::sync::Mutex;

/// One dictation captured for practice.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PracticeCapture {
    /// What the recognizer heard, before cleanup and Polish.
    pub raw: String,
    /// The final text, as it would have been typed.
    pub text: String,
    /// How long the person spoke.
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Copy)]
struct Armed {
    token: u64,
    polish: bool,
    /// Swallow the next dictation without handing it on (a turn nobody spoke in).
    discard: bool,
}

type Listener = Box<dyn Fn(u64, PracticeCapture) + Send + Sync>;

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);

/// Where captures go (the desktop shell emits them to the UI).
pub fn set_capture_listener(listener: impl Fn(u64, PracticeCapture) + Send + Sync + 'static) {
    *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(listener));
}

/// The next dictation becomes practice rep `token`. `polish`: run Polish as
/// usual (reps) or skip it (scene turns, where the partner answers what you said).
pub fn arm(token: u64, polish: bool) {
    *ARMED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Armed {
        token,
        polish,
        discard: false,
    });
}

/// The next dictation is dropped: it was started for practice but nobody spoke.
pub fn arm_discard() {
    *ARMED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Armed {
        token: 0,
        polish: false,
        discard: true,
    });
}

/// Back to normal dictation.
pub fn disarm() {
    *ARMED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Whether the next dictation goes to Coach.
pub fn practice_armed() -> bool {
    ARMED.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Whether the armed capture wants Polish (false when nothing is armed).
pub fn practice_wants_polish() -> bool {
    ARMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|a| a.polish)
}

/// The armed rep's token, if any.
pub fn armed_token() -> Option<u64> {
    ARMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .filter(|a| !a.discard)
        .map(|a| a.token)
}

/// Hands a finished dictation to Coach. Returns `true` when Coach took it
/// (the engine must then neither type nor save it), `false` when nothing
/// was armed. Disarms either way, so only one dictation is ever taken.
pub fn take_practice(capture: PracticeCapture) -> bool {
    let Some(armed) = ARMED.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return false;
    };
    if !armed.discard {
        if let Some(listener) = LISTENER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            listener(armed.token, capture);
        }
    }
    true
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn the_next_dictation_goes_to_the_armed_rep_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let got: Arc<StdMutex<Vec<(u64, String)>>> = Arc::default();
        let sink = got.clone();
        set_capture_listener(move |token, c| sink.lock().unwrap().push((token, c.text)));
        let capture = |t: &str| PracticeCapture {
            raw: t.into(),
            text: t.into(),
            duration_ms: 1_000,
        };

        disarm();
        assert!(!take_practice(capture("typed as usual")));
        arm(7, true);
        assert!(practice_armed());
        assert!(practice_wants_polish());
        assert_eq!(armed_token(), Some(7));
        assert!(take_practice(capture("my rep")));
        assert!(!practice_armed());
        assert!(!take_practice(capture("back to normal")));

        arm_discard();
        assert!(!practice_wants_polish());
        assert!(take_practice(capture("nobody spoke")));
        assert_eq!(*got.lock().unwrap(), vec![(7, "my rep".to_string())]);
        disarm();
    }
}
