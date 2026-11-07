//! Global hotkeys: `RegisterHotKey` (Windows), `XGrabKey` (X11), and the XDG
//! GlobalShortcuts portal (Wayland). Each manager owns every binding the app
//! needs (push-to-talk and the voice-edit shortcut) in one event loop —
//! `global-hotkey` delivers all events through a single global channel, so two
//! independent loops would steal each other's events.
//!
//! Every binding reports key-down and key-up; repeated downs while held and
//! stray ups are filtered here, so callers see clean press/release pairs.

use crate::error::{CoreError, Result};
use crate::types::HotkeyCombo;

/// Fired on key-down and key-up.
pub type HotkeyCallback = Box<dyn Fn() + Send + 'static>;

/// Stable ids (also the GlobalShortcuts portal shortcut ids).
pub const PUSH_TO_TALK_ID: &str = "push-to-talk";
pub const EDIT_SELECTION_ID: &str = "edit-selection";

/// One global shortcut.
pub struct HotkeyBinding {
    pub id: &'static str,
    /// Shown in the portal's bind dialog / GNOME Settings.
    pub description: &'static str,
    pub combo: HotkeyCombo,
    pub on_down: HotkeyCallback,
    pub on_up: HotkeyCallback,
}

impl HotkeyBinding {
    pub fn push_to_talk(
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Self {
        Self {
            id: PUSH_TO_TALK_ID,
            description: "Dictate — hold to talk, tap for hands-free",
            combo,
            on_down,
            on_up,
        }
    }

    pub fn edit_selection(
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Self {
        Self {
            id: EDIT_SELECTION_ID,
            description: "Edit selection with your voice",
            combo,
            on_down,
            on_up,
        }
    }
}

/// Filters a raw key stream into clean press/release pairs: auto-repeat
/// presses while held, and releases with no press, are dropped.
#[derive(Debug, Default, Clone, Copy)]
pub struct PressFilter {
    pressed: bool,
}

impl PressFilter {
    /// `true` when this down should be delivered.
    pub fn down(&mut self) -> bool {
        !std::mem::replace(&mut self.pressed, true)
    }
    /// `true` when this up should be delivered.
    pub fn up(&mut self) -> bool {
        std::mem::replace(&mut self.pressed, false)
    }
}

pub trait HotkeyManager: Send {
    /// Register every binding (replacing any previous registration).
    fn register_all(&mut self, bindings: Vec<HotkeyBinding>) -> Result<()>;

    /// Register a single push-to-talk combo (legacy one-binding API).
    fn register(
        &mut self,
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Result<()> {
        self.register_all(vec![HotkeyBinding::push_to_talk(combo, on_down, on_up)])
    }

    /// Remove every registered hotkey.
    fn unregister(&mut self) -> Result<()>;

    fn backend_name(&self) -> &'static str;
}

/// Mock manager whose press/release are fired manually (tests, `demo`).
#[derive(Default)]
pub struct MockHotkeyManager {
    bindings: Vec<HotkeyBinding>,
}

impl MockHotkeyManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn find(&self, id: &str) -> Option<&HotkeyBinding> {
        self.bindings.iter().find(|b| b.id == id)
    }

    /// Simulate the push-to-talk hotkey being pressed.
    pub fn fire_down(&self) {
        self.fire_down_id(PUSH_TO_TALK_ID);
    }

    /// Simulate the push-to-talk hotkey being released.
    pub fn fire_up(&self) {
        self.fire_up_id(PUSH_TO_TALK_ID);
    }

    pub fn fire_down_id(&self, id: &str) {
        if let Some(b) = self.find(id) {
            (b.on_down)();
        }
    }

    pub fn fire_up_id(&self, id: &str) {
        if let Some(b) = self.find(id) {
            (b.on_up)();
        }
    }

    /// Registered combos by id.
    pub fn combos(&self) -> Vec<(&'static str, HotkeyCombo)> {
        self.bindings
            .iter()
            .map(|b| (b.id, b.combo.clone()))
            .collect()
    }
}

impl HotkeyManager for MockHotkeyManager {
    fn register_all(&mut self, bindings: Vec<HotkeyBinding>) -> Result<()> {
        check_distinct(&bindings)?;
        self.bindings = bindings;
        Ok(())
    }

    fn unregister(&mut self) -> Result<()> {
        self.bindings.clear();
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "mock"
    }
}

/// Two bindings on one chord would make every press ambiguous.
fn check_distinct(bindings: &[HotkeyBinding]) -> Result<()> {
    for (i, a) in bindings.iter().enumerate() {
        for b in &bindings[i + 1..] {
            if a.combo.same_chord(&b.combo) {
                return Err(CoreError::Hotkey(format!(
                    "{} is assigned to two shortcuts",
                    a.combo.display()
                )));
            }
        }
    }
    Ok(())
}

/// Real OS-selected hotkey manager (`global-hotkey`: Windows `RegisterHotKey`,
/// macOS Carbon, Linux X11). A dedicated thread owns the manager and drives the
/// platform event source — pumping the win32 message queue on Windows, which
/// `global-hotkey` requires. Without the `hotkey` feature, `register_all`
/// returns [`CoreError::NotImplemented`].
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
    fn register_all(&mut self, bindings: Vec<HotkeyBinding>) -> Result<()> {
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        use std::sync::atomic::{AtomicBool, Ordering};

        self.unregister()?;
        check_distinct(&bindings)?;
        let mut resolved = Vec::with_capacity(bindings.len());
        for b in bindings {
            let hotkey = to_global_hotkey(&b.combo)?;
            resolved.push((hotkey, b, PressFilter::default()));
        }
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

        // The manager + event loop must live on one thread (win32 message loop
        // on Windows); global-hotkey self-manages X11 on Linux.
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
                for (hotkey, b, _) in &resolved {
                    if let Err(e) = manager.register(*hotkey) {
                        for (h, _, _) in &resolved {
                            let _ = manager.unregister(*h);
                        }
                        let _ = ready_tx.send(Err(CoreError::Hotkey(format!(
                            "{} is unavailable — another app may be using it ({e})",
                            b.combo.display()
                        ))));
                        return;
                    }
                }
                let _ = ready_tx.send(Ok(()));

                let receiver = GlobalHotKeyEvent::receiver();
                while !stop.load(Ordering::Relaxed) {
                    #[cfg(windows)]
                    pump_win32_messages();
                    while let Ok(event) = receiver.try_recv() {
                        if let Some((_, b, filter)) =
                            resolved.iter_mut().find(|(h, _, _)| h.id() == event.id)
                        {
                            match event.state {
                                HotKeyState::Pressed => {
                                    if filter.down() {
                                        (b.on_down)();
                                    }
                                }
                                HotKeyState::Released => {
                                    if filter.up() {
                                        (b.on_up)();
                                    }
                                }
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                for (h, _, _) in &resolved {
                    let _ = manager.unregister(*h);
                }
            })
            .map_err(|e| CoreError::Hotkey(format!("spawn hotkey thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.thread = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err(CoreError::Hotkey(
                "hotkey thread exited before registering".to_string(),
            )),
        }
    }

    #[cfg(not(feature = "hotkey"))]
    fn register_all(&mut self, _bindings: Vec<HotkeyBinding>) -> Result<()> {
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

/// Whether hotkeys on this session go through the GlobalShortcuts portal.
pub fn uses_portal() -> bool {
    cfg!(all(feature = "portal", target_os = "linux")) && crate::injection::is_wayland_session()
}

/// The best hotkey manager for the current session: the GlobalShortcuts portal
/// on Wayland (where X11 key grabs aren't delivered), otherwise the system
/// global hotkey (`RegisterHotKey` / `XGrabKey`).
pub fn default_manager() -> Box<dyn HotkeyManager> {
    #[cfg(all(feature = "portal", target_os = "linux"))]
    if uses_portal() {
        return Box::new(portal::PortalHotkeyManager::new());
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

/// Whether `combo` can be registered as a global hotkey on this build (used by
/// the shortcut recorder to reject keys the OS backend can't grab).
pub fn validate_combo(combo: &HotkeyCombo) -> Result<()> {
    if combo.key.trim().is_empty() {
        return Err(CoreError::Hotkey("press a key".into()));
    }
    if combo.modifiers.is_empty() && !combo.key.to_ascii_uppercase().starts_with('F') {
        return Err(CoreError::Hotkey(
            "add Ctrl, Alt, Shift or Super — a bare key would fire while you type".into(),
        ));
    }
    #[cfg(feature = "hotkey")]
    key_to_code(&combo.key)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn press_filter_drops_repeats_and_stray_ups() {
        let mut f = PressFilter::default();
        assert!(!f.up());
        assert!(f.down());
        assert!(!f.down());
        assert!(f.up());
        assert!(!f.up());
    }

    #[test]
    fn mock_routes_by_id_and_rejects_duplicate_chords() {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let mut m = MockHotkeyManager::new();
        m.register_all(vec![
            HotkeyBinding::push_to_talk(HotkeyCombo::default(), Box::new(|| {}), Box::new(|| {})),
            HotkeyBinding::edit_selection(
                HotkeyCombo::parse("Ctrl+Shift+E"),
                Box::new(move || {
                    h.fetch_add(1, Ordering::SeqCst);
                }),
                Box::new(|| {}),
            ),
        ])
        .unwrap();
        m.fire_down_id(EDIT_SELECTION_ID);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(m.combos().len(), 2);
        let dup = m.register_all(vec![
            HotkeyBinding::push_to_talk(HotkeyCombo::default(), Box::new(|| {}), Box::new(|| {})),
            HotkeyBinding::edit_selection(
                HotkeyCombo::parse("Shift+Ctrl+Space"),
                Box::new(|| {}),
                Box::new(|| {}),
            ),
        ]);
        assert!(dup.is_err());
    }

    #[test]
    fn validate_rejects_bare_keys() {
        assert!(validate_combo(&HotkeyCombo::parse("A")).is_err());
        assert!(validate_combo(&HotkeyCombo::parse("F8")).is_ok());
        assert!(validate_combo(&HotkeyCombo::default()).is_ok());
    }

    #[cfg(feature = "hotkey")]
    #[test]
    fn maps_common_keys() {
        use global_hotkey::hotkey::Code;
        assert_eq!(key_to_code("space").unwrap(), Code::Space);
        assert_eq!(key_to_code("Space").unwrap(), Code::Space);
        assert_eq!(key_to_code("a").unwrap(), Code::KeyA);
        assert_eq!(key_to_code("Z").unwrap(), Code::KeyZ);
        assert_eq!(key_to_code("1").unwrap(), Code::Digit1);
        assert_eq!(key_to_code("f5").unwrap(), Code::F5);
        assert!(key_to_code("definitely-not-a-key").is_err());
    }

    #[cfg(feature = "hotkey")]
    #[test]
    fn builds_combo_deterministically() {
        let combo = HotkeyCombo::default();
        let a = to_global_hotkey(&combo).unwrap();
        let b = to_global_hotkey(&combo).unwrap();
        assert_eq!(a.id(), b.id());
    }
}
