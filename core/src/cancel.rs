//! Stopping a decode that is no longer wanted.
//!
//! A decode started at a pause (see [`crate::live::EarlyDecode`]) goes stale
//! as soon as the speaker talks again. Left running, it keeps the speech model
//! busy, so the decode at release waits for it before it can start. A
//! [`Token`] lets the coordinator stop it: the decode runs inside
//! [`scope`], and the model code asks [`cancelled`] between steps or hooks
//! its own stop (ONNX Runtime's terminate flag) with [`on_cancel`].
//!
//! The token travels with the thread rather than through every call, so the
//! engines' `transcribe` signature stays the same. A thread a decode spawns
//! (Word Boost's scorer) takes it along with [`current`] and [`scope`].

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type Hook = Box<dyn Fn() + Send>;

/// Asks a decode to stop.
#[derive(Default)]
pub struct Token {
    stopped: AtomicBool,
    next_hook: AtomicU64,
    hooks: Mutex<Vec<(u64, Hook)>>,
}

impl Token {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Stop the decode: steps not yet started are skipped and running model
    /// calls are told to end early.
    pub fn cancel(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        for (_, hook) in self.hooks.lock().unwrap().iter() {
            hook();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Token>>> = const { RefCell::new(None) };
}

/// Run `f` with `token` as this thread's stop signal (none: `f` can't be
/// stopped). The previous signal is restored afterwards.
pub fn scope<R>(token: Option<Arc<Token>>, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT.with(|c| c.replace(token));
    struct Restore(Option<Arc<Token>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = previous);
        }
    }
    let _restore = Restore(previous);
    f()
}

/// This thread's stop signal, to hand to a thread it spawns.
pub fn current() -> Option<Arc<Token>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Whether this thread's decode has been asked to stop.
pub fn cancelled() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(|t| t.is_cancelled()))
}

/// Removes its hook when dropped.
pub struct HookGuard {
    token: Option<Arc<Token>>,
    id: u64,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        if let Some(t) = &self.token {
            t.hooks.lock().unwrap().retain(|(id, _)| *id != self.id);
        }
    }
}

/// Call `hook` if this thread's decode is stopped while the guard lives (at
/// once if it already is). Without a signal nothing is registered.
pub fn on_cancel(hook: impl Fn() + Send + 'static) -> HookGuard {
    let token = current();
    let mut id = 0;
    if let Some(t) = &token {
        id = t.next_hook.fetch_add(1, Ordering::SeqCst);
        let mut hooks = t.hooks.lock().unwrap();
        // Checked under the lock `cancel` takes, so a stop can't slip between
        // the check and the registration.
        if t.is_cancelled() {
            hook();
        }
        hooks.push((id, Box::new(hook)));
    }
    HookGuard { token, id }
}

/// The error a stopped decode returns.
pub fn error() -> crate::error::CoreError {
    crate::error::CoreError::Transcription("decode cancelled".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_thread_without_a_token_is_never_cancelled() {
        assert!(!cancelled());
        assert!(current().is_none());
    }

    #[test]
    fn scope_sets_and_restores_the_token() {
        let t = Token::new();
        scope(Some(t.clone()), || {
            assert!(!cancelled());
            t.cancel();
            assert!(cancelled());
            scope(None, || assert!(!cancelled()));
            assert!(cancelled());
        });
        assert!(!cancelled());
    }

    #[test]
    fn hooks_run_on_cancel_and_go_when_dropped() {
        let t = Token::new();
        let calls = Arc::new(AtomicUsize::new(0));
        scope(Some(t.clone()), || {
            let c = calls.clone();
            let guard = on_cancel(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
            drop(guard);
            let c = calls.clone();
            let _kept = on_cancel(move || {
                c.fetch_add(10, Ordering::SeqCst);
            });
            t.cancel();
        });
        assert_eq!(calls.load(Ordering::SeqCst), 10);
        assert!(t.hooks.lock().unwrap().is_empty());
    }

    #[test]
    fn a_hook_added_after_cancel_runs_at_once() {
        let t = Token::new();
        t.cancel();
        let calls = Arc::new(AtomicUsize::new(0));
        scope(Some(t), || {
            let c = calls.clone();
            let _g = on_cancel(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_spawned_thread_can_carry_the_token() {
        let t = Token::new();
        t.cancel();
        let seen = scope(Some(t), || {
            let token = current();
            std::thread::spawn(move || scope(token, cancelled))
                .join()
                .unwrap()
        });
        assert!(seen);
    }
}
