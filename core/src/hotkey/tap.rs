//! Decides whether a press of the Right Ctrl key, on its own, was a tap: the
//! key went down and up quickly with nothing else pressed in between. Using
//! Ctrl in a shortcut (Ctrl+C) or with a click is never a tap.
//!
//! Port of the macOS `RightOptionTapDetector` (the Mac uses Right Option; on
//! Windows and Linux Right Alt is AltGr on many keyboards, so Right Ctrl takes
//! its place). Timestamps are the platform's millisecond event clock, injected
//! so the logic is deterministic under test.

use crate::gesture::TAP_MAX;

#[derive(Debug, Default)]
pub struct TapDetector {
    /// The key is down (auto-repeat sends more key-downs while it's held).
    down: bool,
    /// When a clean press started; `None` when it began as a combo.
    press_start: Option<u32>,
    spoiled: bool,
}

impl TapDetector {
    /// Right Ctrl went down. `others_held`: another key or modifier was
    /// already down, so this is the start of a combo.
    pub fn key_down(&mut self, others_held: bool, at_ms: u32) {
        if self.down {
            return; // auto-repeat
        }
        self.down = true;
        self.press_start = (!others_held).then_some(at_ms);
        self.spoiled = false;
    }

    /// Right Ctrl came up. Returns true when this completes a clean tap.
    pub fn key_up(&mut self, at_ms: u32) -> bool {
        if !std::mem::take(&mut self.down) {
            return false;
        }
        let start = self.press_start.take();
        let spoiled = std::mem::take(&mut self.spoiled);
        let Some(start) = start else { return false };
        // The event clock is 32-bit milliseconds and wraps every ~49 days.
        let held = at_ms.wrapping_sub(start);
        !spoiled && u128::from(held) < TAP_MAX.as_millis()
    }

    /// Any other key or a mouse button went down. While Right Ctrl is held
    /// this turns the press into a combo, never a tap.
    pub fn other_input(&mut self) {
        if self.press_start.is_some() {
            self.spoiled = true;
        }
    }

    /// Forget any press in flight, e.g. when the listener restarts.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_press_on_its_own_is_a_tap() {
        let mut d = TapDetector::default();
        d.key_down(false, 1_000);
        assert!(d.key_up(1_120));
    }

    #[test]
    fn a_long_press_is_not_a_tap() {
        let mut d = TapDetector::default();
        d.key_down(false, 1_000);
        assert!(!d.key_up(1_500), "half a second is a hold");
        d.key_down(false, 2_000);
        assert!(d.key_up(2_499));
    }

    #[test]
    fn shortcuts_and_clicks_are_never_taps() {
        let mut d = TapDetector::default();
        // Ctrl+C: another key while Right Ctrl is down.
        d.key_down(false, 0);
        d.other_input();
        assert!(!d.key_up(100));
        // Shift held first, then Right Ctrl.
        d.key_down(true, 200);
        assert!(!d.key_up(260));
        // Ctrl+click.
        d.key_down(false, 400);
        d.other_input();
        assert!(!d.key_up(450));
        // The next clean tap still counts.
        d.key_down(false, 600);
        assert!(d.key_up(700));
    }

    #[test]
    fn auto_repeat_and_stray_releases_are_ignored() {
        let mut d = TapDetector::default();
        d.key_down(false, 0);
        d.key_down(false, 30); // repeat: the press still started at 0
        d.key_down(false, 60);
        assert!(!d.key_up(520), "held 520 ms");
        assert!(!d.key_up(530), "a release with no press");
        // Typing before the press doesn't spoil it.
        d.other_input();
        d.key_down(false, 1_000);
        assert!(d.key_up(1_050));
    }

    #[test]
    fn the_millisecond_clock_wrapping_is_handled() {
        let mut d = TapDetector::default();
        d.key_down(false, u32::MAX - 50);
        assert!(d.key_up(100));
    }

    #[test]
    fn reset_forgets_a_press() {
        let mut d = TapDetector::default();
        d.key_down(false, 0);
        d.reset();
        assert!(!d.key_up(50));
    }
}
