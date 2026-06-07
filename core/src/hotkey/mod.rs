//! Global push-to-talk hotkey. Real backends land in Phase 1:
//! `RegisterHotKey` (Windows), `XGrabKey` (X11), XDG GlobalShortcuts portal (Wayland).
//! The Wayland path is brittle — registration may fail and callers degrade gracefully.

use crate::error::{CoreError, Result};
use crate::types::HotkeyCombo;

/// Fired on key-down (start recording) and key-up (stop + transcribe).
pub type HotkeyCallback = Box<dyn Fn() + Send + 'static>;

pub trait HotkeyManager: Send {
    /// Register `combo`, invoking `on_down` when held and `on_up` when released.
    fn register(
        &mut self,
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Result<()>;

    /// Remove any registered hotkey.
    fn unregister(&mut self) -> Result<()>;

    fn backend_name(&self) -> &'static str;
}

/// Mock manager whose press/release are fired manually (tests, `demo`).
#[derive(Default)]
pub struct MockHotkeyManager {
    on_down: Option<HotkeyCallback>,
    on_up: Option<HotkeyCallback>,
}

impl MockHotkeyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate the hotkey being pressed.
    pub fn fire_down(&self) {
        if let Some(cb) = &self.on_down {
            cb();
        }
    }

    /// Simulate the hotkey being released.
    pub fn fire_up(&self) {
        if let Some(cb) = &self.on_up {
            cb();
        }
    }
}

impl HotkeyManager for MockHotkeyManager {
    fn register(
        &mut self,
        _combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Result<()> {
        self.on_down = Some(on_down);
        self.on_up = Some(on_up);
        Ok(())
    }

    fn unregister(&mut self) -> Result<()> {
        self.on_down = None;
        self.on_up = None;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "mock"
    }
}

/// Real OS-selected hotkey manager. Phase 1 fills these in.
#[derive(Default)]
pub struct SystemHotkeyManager;

impl SystemHotkeyManager {
    pub fn new() -> Self {
        Self
    }
}

impl HotkeyManager for SystemHotkeyManager {
    fn register(
        &mut self,
        _combo: HotkeyCombo,
        _on_down: HotkeyCallback,
        _on_up: HotkeyCallback,
    ) -> Result<()> {
        Err(CoreError::NotImplemented("system global hotkey (Phase 1)"))
    }

    fn unregister(&mut self) -> Result<()> {
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        #[cfg(target_os = "windows")]
        {
            "RegisterHotKey (stub)"
        }
        #[cfg(target_os = "linux")]
        {
            "XGrabKey/portal (stub)"
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            "unsupported-host (stub)"
        }
    }
}
