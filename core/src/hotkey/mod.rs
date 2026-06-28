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

/// Real OS-selected hotkey manager.
///
/// With the `hotkey` feature it uses `global-hotkey` (Windows `RegisterHotKey`,
/// macOS Carbon, Linux X11) and emits push-to-talk **down/up** events. A dedicated
/// thread owns the manager and drives the platform event source — pumping the
/// win32 message queue on Windows, which `global-hotkey` requires. Without the
/// feature, `register` returns [`CoreError::NotImplemented`].
///
/// Wayland note: native global shortcuts there go through the XDG portal and are
/// not covered here yet; X11 (incl. XWayland) works. See `docs/gaps.md`.
pub struct SystemHotkeyManager {
    #[cfg(feature = "hotkey")]
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "hotkey")]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SystemHotkeyManager {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "hotkey")]
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "hotkey")]
            thread: None,
        }
    }
}

impl Default for SystemHotkeyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyManager for SystemHotkeyManager {
    #[cfg(feature = "hotkey")]
    fn register(
        &mut self,
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Result<()> {
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        use std::sync::atomic::Ordering;

        let hotkey = to_global_hotkey(&combo)?;
        let id = hotkey.id();
        let stop = self.stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

        // The manager + event loop must live on one thread (win32 message loop on
        // Windows); global-hotkey self-manages X11 on Linux.
        let handle = std::thread::Builder::new()
            .name("orttaai-hotkey".to_string())
            .spawn(move || {
                let manager = match GlobalHotKeyManager::new() {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = ready_tx.send(Err(CoreError::Hotkey(format!("manager init: {e}"))));
                        return;
                    }
                };
                if let Err(e) = manager.register(hotkey) {
                    let _ = ready_tx.send(Err(CoreError::Hotkey(format!("register: {e}"))));
                    return;
                }
                let _ = ready_tx.send(Ok(()));

                let receiver = GlobalHotKeyEvent::receiver();
                while !stop.load(Ordering::Relaxed) {
                    #[cfg(windows)]
                    pump_win32_messages();
                    while let Ok(event) = receiver.try_recv() {
                        if event.id == id {
                            match event.state {
                                HotKeyState::Pressed => on_down(),
                                HotKeyState::Released => on_up(),
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                let _ = manager.unregister(hotkey);
            })
            .map_err(|e| CoreError::Hotkey(format!("spawn hotkey thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.thread = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CoreError::Hotkey(
                "hotkey thread exited before registering".to_string(),
            )),
        }
    }

    #[cfg(not(feature = "hotkey"))]
    fn register(
        &mut self,
        _combo: HotkeyCombo,
        _on_down: HotkeyCallback,
        _on_up: HotkeyCallback,
    ) -> Result<()> {
        Err(CoreError::NotImplemented(
            "system global hotkey — rebuild with --features hotkey",
        ))
    }

    fn unregister(&mut self) -> Result<()> {
        #[cfg(feature = "hotkey")]
        {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_return)]
    fn backend_name(&self) -> &'static str {
        #[cfg(not(feature = "hotkey"))]
        return "system (stub — build with --features hotkey)";
        #[cfg(all(feature = "hotkey", target_os = "windows"))]
        return "RegisterHotKey";
        #[cfg(all(feature = "hotkey", target_os = "macos"))]
        return "Carbon hotkey";
        #[cfg(all(feature = "hotkey", target_os = "linux"))]
        return "XGrabKey (X11)";
        #[cfg(all(
            feature = "hotkey",
            not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
        ))]
        return "global-hotkey";
    }
}

#[cfg(feature = "hotkey")]
impl Drop for SystemHotkeyManager {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

/// The best hotkey manager for the current session: the GlobalShortcuts portal
/// on Wayland (where X11 key grabs aren't delivered), otherwise the system
/// global hotkey (`RegisterHotKey` / `XGrabKey`).
pub fn default_manager() -> Box<dyn HotkeyManager> {
    #[cfg(all(feature = "portal", target_os = "linux"))]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|s| s.eq_ignore_ascii_case("wayland"))
                .unwrap_or(false);
        if wayland {
            return Box::new(portal::PortalHotkeyManager::new());
        }
    }
    Box::new(SystemHotkeyManager::new())
}

#[cfg(all(feature = "portal", target_os = "linux"))]
mod portal;

#[cfg(feature = "hotkey")]
fn to_global_hotkey(combo: &HotkeyCombo) -> Result<global_hotkey::hotkey::HotKey> {
    use crate::types::Modifier;
    use global_hotkey::hotkey::{HotKey, Modifiers};

    let mut mods = Modifiers::empty();
    for modifier in &combo.modifiers {
        mods |= match modifier {
            Modifier::Ctrl => Modifiers::CONTROL,
            Modifier::Shift => Modifiers::SHIFT,
            Modifier::Alt => Modifiers::ALT,
            Modifier::Meta => Modifiers::META,
        };
    }
    let code = key_to_code(&combo.key)?;
    Ok(HotKey::new(
        if mods.is_empty() { None } else { Some(mods) },
        code,
    ))
}

/// Map our key names to W3C UI Events code strings, then parse:
/// `"a"` → `KeyA`, `"1"` → `Digit1`, `"space"` → `Space`, `"f5"` → `F5`.
#[cfg(feature = "hotkey")]
fn key_to_code(key: &str) -> Result<global_hotkey::hotkey::Code> {
    let normalized = if key.chars().count() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            format!("Key{}", c.to_ascii_uppercase())
        } else if c.is_ascii_digit() {
            format!("Digit{c}")
        } else {
            key.to_string()
        }
    } else {
        let mut s = key.to_string();
        if let Some(first) = s.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        s
    };
    normalized
        .parse::<global_hotkey::hotkey::Code>()
        .map_err(|_| CoreError::Hotkey(format!("unsupported hotkey key: {key}")))
}

#[cfg(all(feature = "hotkey", windows))]
fn pump_win32_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(all(test, feature = "hotkey"))]
mod tests {
    use super::{key_to_code, to_global_hotkey};
    use crate::types::{HotkeyCombo, Modifier};
    use global_hotkey::hotkey::Code;

    #[test]
    fn maps_common_keys() {
        assert_eq!(key_to_code("space").unwrap(), Code::Space);
        assert_eq!(key_to_code("Space").unwrap(), Code::Space);
        assert_eq!(key_to_code("a").unwrap(), Code::KeyA);
        assert_eq!(key_to_code("Z").unwrap(), Code::KeyZ);
        assert_eq!(key_to_code("1").unwrap(), Code::Digit1);
        assert_eq!(key_to_code("f5").unwrap(), Code::F5);
        assert!(key_to_code("definitely-not-a-key").is_err());
    }

    #[test]
    fn builds_combo_deterministically() {
        let combo = HotkeyCombo {
            key: "Space".to_string(),
            modifiers: vec![Modifier::Ctrl, Modifier::Shift],
        };
        let a = to_global_hotkey(&combo).unwrap();
        let b = to_global_hotkey(&combo).unwrap();
        assert_eq!(a.id(), b.id());
    }
}
