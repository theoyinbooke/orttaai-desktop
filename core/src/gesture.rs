//! Hotkey gesture interpreter: hold = push-to-talk, short tap = hands-free.
//!
//! Port of the macOS `HotkeyGestureInterpreter`. Recording always starts on
//! key-down (never waiting for disambiguation); this only decides what the
//! matching key-up means:
//! - released before [`TAP_MAX`] with hands-free enabled → the press was a
//!   **tap**: keep recording and promote it to hands-free mode;
//! - released at/after the threshold (or hands-free disabled) → a **hold**:
//!   stop and finalize, exactly like classic push-to-talk.
//!
//! Timestamps are injected so the logic is deterministic under test.

use std::time::Duration;

/// A press-and-release shorter than this reads as a tap. It matches the
/// coordinator's minimum recording length: a hold any shorter would be thrown
/// away as too short, so a press that brief is a tap.
pub const TAP_MAX: Duration = Duration::from_millis(500);

/// What a key-up means for the recording in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAction {
    /// Hold released: stop recording and finalize.
    StopRecording,
    /// Tap detected: keep recording, switch to hands-free mode.
    PromoteToHandsFree,
    /// No press was recorded for this release (e.g. the key-up following the
    /// tap that stopped a hands-free session) — do nothing.
    Ignore,
}

#[derive(Debug, Clone, Default)]
pub struct GestureInterpreter {
    press_start: Option<Duration>,
}

impl GestureInterpreter {
    pub fn new() -> Self {
        Self::default()
    }

    /// When the in-flight press began, if any.
    pub fn press_start(&self) -> Option<Duration> {
        self.press_start
    }

    /// Records the key-down that started (or attempted to start) recording.
    /// `at` is any monotonic timestamp.
    pub fn record_press(&mut self, at: Duration) {
        self.press_start = Some(at);
    }

    /// Forgets any in-flight press, e.g. when recording failed to start or a
    /// key-down was consumed as a hands-free stop.
    pub fn reset(&mut self) {
        self.press_start = None;
    }

    /// Consumes the recorded press and classifies the release. A release with no
    /// recorded press is ignored. A release timestamped before its press
    /// classifies as a hold so recording always has a way to stop.
    pub fn evaluate_release(&mut self, at: Duration, hands_free_enabled: bool) -> ReleaseAction {
        let Some(start) = self.press_start.take() else {
            return ReleaseAction::Ignore;
        };
        match at.checked_sub(start) {
            Some(held) if hands_free_enabled && held < TAP_MAX => ReleaseAction::PromoteToHandsFree,
            _ => ReleaseAction::StopRecording,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn short_press_is_a_tap_when_hands_free_enabled() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(1000));
        assert_eq!(
            g.evaluate_release(ms(1200), true),
            ReleaseAction::PromoteToHandsFree
        );
    }

    #[test]
    fn tap_threshold_is_exclusive() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(0));
        assert_eq!(
            g.evaluate_release(ms(499), true),
            ReleaseAction::PromoteToHandsFree
        );
        g.record_press(ms(0));
        assert_eq!(
            g.evaluate_release(ms(500), true),
            ReleaseAction::StopRecording
        );
    }

    #[test]
    fn short_press_is_a_hold_when_hands_free_disabled() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(0));
        assert_eq!(
            g.evaluate_release(ms(100), false),
            ReleaseAction::StopRecording
        );
    }

    #[test]
    fn long_press_stops() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(0));
        assert_eq!(
            g.evaluate_release(ms(3000), true),
            ReleaseAction::StopRecording
        );
    }

    #[test]
    fn release_without_press_is_ignored_and_press_is_consumed() {
        let mut g = GestureInterpreter::new();
        assert_eq!(g.evaluate_release(ms(10), true), ReleaseAction::Ignore);
        g.record_press(ms(0));
        let _ = g.evaluate_release(ms(100), true);
        assert_eq!(g.evaluate_release(ms(200), true), ReleaseAction::Ignore);
    }

    #[test]
    fn reset_forgets_press() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(0));
        g.reset();
        assert!(g.press_start().is_none());
        assert_eq!(g.evaluate_release(ms(100), true), ReleaseAction::Ignore);
    }

    #[test]
    fn non_monotonic_release_is_a_hold() {
        let mut g = GestureInterpreter::new();
        g.record_press(ms(1000));
        assert_eq!(
            g.evaluate_release(ms(900), true),
            ReleaseAction::StopRecording
        );
    }
}
