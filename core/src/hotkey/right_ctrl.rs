//! "Tap Right Ctrl to dictate" (the Mac's "Tap Right Option to Dictate") and
//! "Tap Right Shift to edit" (the Mac's "Tap Right Command to Edit"): a
//! listen-only watch on the keyboard that reports each clean tap of either key
//! (see [`TapDetector`](super::tap::TapDetector)). Keys are never swallowed, so
//! Ctrl and Shift keep working everywhere.
//!
//! - Windows: low-level keyboard and mouse hooks (no permission needed).
//! - Linux on X11: XInput2 raw key and button events on the root window.
//! - Wayland doesn't let an app watch keys it doesn't own, so the option is
//!   unavailable there; the normal shortcut still works.

use crate::error::{CoreError, Result};

/// Called on each clean tap, on the listener's own thread.
pub type TapCallback = Box<dyn Fn() + Send + Sync + 'static>;

/// What to call for each key's taps; a key with no callback isn't watched.
#[derive(Default)]
pub struct Taps {
    pub right_ctrl: Option<TapCallback>,
    pub right_shift: Option<TapCallback>,
}

/// Why the key can't be watched on this system, if it can't.
pub fn unavailable_reason() -> Option<&'static str> {
    if cfg!(not(all(
        feature = "hotkey",
        any(windows, target_os = "linux")
    ))) {
        return Some("Tapping Right Ctrl or Right Shift isn't available on this system.");
    }
    if crate::injection::is_wayland_session() {
        return Some(
            "Wayland doesn't let apps watch a single key, so this works on X11 and Windows. Use your shortcut instead.",
        );
    }
    None
}

/// One of the watched keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightKey {
    Ctrl,
    Shift,
}

impl RightKey {
    pub fn name(self) -> &'static str {
        match self {
            RightKey::Ctrl => "Right Ctrl",
            RightKey::Shift => "Right Shift",
        }
    }
}

/// Watches Right Ctrl and/or Right Shift until dropped.
pub struct RightKeyListener {
    #[allow(dead_code)]
    inner: imp::Listener,
    missing: Vec<RightKey>,
}

impl RightKeyListener {
    /// Starts when at least one asked-for key can be watched; a key the
    /// keyboard layout doesn't have (Right Ctrl remapped to Compose, say) is
    /// left out and reported by [`missing`](Self::missing), so the other one
    /// still works.
    pub fn start(taps: Taps) -> Result<Self> {
        if let Some(reason) = unavailable_reason() {
            return Err(CoreError::Hotkey(reason.into()));
        }
        if taps.right_ctrl.is_none() && taps.right_shift.is_none() {
            return Err(CoreError::Hotkey("no key to watch".into()));
        }
        let (inner, missing) = imp::start(taps)?;
        Ok(Self { inner, missing })
    }

    /// Keys that were asked for but this keyboard layout doesn't have.
    pub fn missing(&self) -> &[RightKey] {
        &self.missing
    }
}

#[cfg(all(feature = "hotkey", windows))]
mod imp {
    use super::{RightKey, Taps};
    use crate::error::{CoreError, Result};
    use crate::hotkey::tap::TapDetector;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU,
        VK_RSHIFT, VK_RWIN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEHWHEEL,
        WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN,
    };

    /// Every modifier; any other one already held makes a press a combo.
    const MODIFIERS: [u16; 8] = [
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ];

    struct Watched {
        vk: u16,
        detector: TapDetector,
        on_tap: Arc<dyn Fn() + Send + Sync>,
    }

    struct Hook {
        keys: Vec<Watched>,
    }

    /// Hook procedures are plain functions, so their state lives here. One
    /// listener at a time.
    static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub struct Listener {
        thread_id: u32,
        handle: Option<JoinHandle<()>>,
    }

    pub fn start(taps: Taps) -> Result<(Listener, Vec<RightKey>)> {
        {
            let mut slot = HOOK.lock().unwrap();
            if slot.is_some() {
                return Err(CoreError::Hotkey(
                    "the right-side keys are already being watched".into(),
                ));
            }
            let keys = [
                (VK_RCONTROL, taps.right_ctrl),
                (VK_RSHIFT, taps.right_shift),
            ]
            .into_iter()
            .filter_map(|(vk, cb)| {
                cb.map(|cb| Watched {
                    vk,
                    detector: TapDetector::default(),
                    on_tap: Arc::from(cb),
                })
            })
            .collect();
            *slot = Some(Hook { keys });
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("orttaai-right-keys".into())
            .spawn(move || {
                // SAFETY: the hooks are installed and removed on this thread,
                // which pumps messages until WM_QUIT so they get called.
                unsafe {
                    let mut msg: MSG = std::mem::zeroed();
                    // Create this thread's message queue before anyone posts
                    // WM_QUIT to it.
                    PeekMessageW(
                        &mut msg,
                        std::ptr::null_mut(),
                        WM_USER,
                        WM_USER,
                        PM_NOREMOVE,
                    );
                    let module = GetModuleHandleW(std::ptr::null());
                    let keyboard =
                        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
                    if keyboard.is_null() {
                        let _ = tx.send(Err(std::io::Error::last_os_error().to_string()));
                        return;
                    }
                    // Ctrl+click is a combo too; without this hook it would
                    // only miss that case.
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
                    let _ = tx.send(Ok(GetCurrentThreadId()));
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
                    UnhookWindowsHookEx(keyboard);
                    if !mouse.is_null() {
                        UnhookWindowsHookEx(mouse);
                    }
                }
            });
        let handle = match spawned {
            Ok(h) => h,
            Err(e) => {
                *HOOK.lock().unwrap() = None;
                return Err(e.into());
            }
        };
        match rx.recv() {
            // Every PC keyboard has both keys.
            Ok(Ok(thread_id)) => Ok((
                Listener {
                    thread_id,
                    handle: Some(handle),
                },
                Vec::new(),
            )),
            other => {
                let _ = handle.join();
                *HOOK.lock().unwrap() = None;
                let why = match other {
                    Ok(Err(e)) => e,
                    _ => "the listener stopped".into(),
                };
                Err(CoreError::Hotkey(format!(
                    "couldn't watch the right-side keys: {why}"
                )))
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // SAFETY: posting to a thread id we own; its queue exists.
            unsafe {
                PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            }
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            *HOOK.lock().unwrap() = None;
        }
    }

    /// Run `f` on the hook state; it returns the callbacks to fire, which run
    /// after the lock is released.
    fn with_hook(f: impl FnOnce(&mut Hook) -> Vec<Arc<dyn Fn() + Send + Sync>>) {
        let fire = {
            let Ok(mut slot) = HOOK.lock() else { return };
            match slot.as_mut() {
                Some(hook) => f(hook),
                None => Vec::new(),
            }
        };
        for on_tap in fire {
            on_tap();
        }
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let key = &*(lparam as *const KBDLLHOOKSTRUCT);
            // Keys other apps (or Orttaai's own typing) inject aren't the user's.
            if key.flags & LLKHF_INJECTED == 0 {
                let up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
                let (vk, time) = (key.vkCode, key.time);
                with_hook(|hook| {
                    let mut fire = Vec::new();
                    for key in &mut hook.keys {
                        if vk == u32::from(key.vk) {
                            if up {
                                if key.detector.key_up(time) {
                                    fire.push(Arc::clone(&key.on_tap));
                                }
                            } else {
                                let others = MODIFIERS
                                    .iter()
                                    .filter(|&&k| k != key.vk)
                                    .any(|&k| GetAsyncKeyState(i32::from(k)) < 0);
                                key.detector.key_down(others, time);
                            }
                        } else if !up {
                            key.detector.other_input();
                        }
                    }
                    fire
                });
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32
            && matches!(
                wparam as u32,
                WM_LBUTTONDOWN
                    | WM_RBUTTONDOWN
                    | WM_MBUTTONDOWN
                    | WM_XBUTTONDOWN
                    | WM_MOUSEWHEEL
                    | WM_MOUSEHWHEEL
            )
        {
            with_hook(|hook| {
                for key in &mut hook.keys {
                    key.detector.other_input();
                }
                Vec::new()
            });
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

#[cfg(all(feature = "hotkey", target_os = "linux"))]
mod imp {
    use super::{RightKey, TapCallback, Taps};
    use crate::error::{CoreError, Result};
    use crate::hotkey::tap::TapDetector;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::JoinHandle;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::xinput::{self, ConnectionExt as _, XIEventMask};
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    /// The `Control_R` and `Shift_R` keysyms.
    const CONTROL_R: u32 = 0xffe4;
    const SHIFT_R: u32 = 0xffe2;
    /// `XIAllMasterDevices`.
    const ALL_MASTER_DEVICES: u16 = 1;

    pub struct Listener {
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    pub fn start(taps: Taps) -> Result<(Listener, Vec<RightKey>)> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop2 = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("orttaai-right-keys".into())
            .spawn(move || match connect(taps) {
                Ok((watch, missing)) => {
                    let _ = tx.send(Ok(missing));
                    watch.run(&stop2);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })?;
        match rx.recv() {
            Ok(Ok(missing)) => Ok((
                Listener {
                    stop,
                    handle: Some(handle),
                },
                missing,
            )),
            other => {
                let _ = handle.join();
                let why = match other {
                    Ok(Err(e)) => e,
                    _ => "the listener stopped".into(),
                };
                Err(CoreError::Hotkey(format!(
                    "couldn't watch the right-side keys: {why}"
                )))
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    struct Watched {
        /// Keycodes that produce the key.
        keycodes: Vec<u8>,
        detector: TapDetector,
        on_tap: TapCallback,
    }

    struct Watch {
        conn: RustConnection,
        keys: Vec<Watched>,
        /// Every modifier key's keycode, the watched keys included.
        modifiers: Vec<u8>,
    }

    fn connect(taps: Taps) -> std::result::Result<(Watch, Vec<RightKey>), String> {
        let s = |e: &dyn std::fmt::Display| e.to_string();
        let (conn, screen) = x11rb::connect(None).map_err(|e| s(&e))?;
        let root = conn.setup().roots[screen].root;
        // 2.2 so raw events reach every client that asks, not only the
        // grabbing one.
        let version = conn
            .xinput_xi_query_version(2, 2)
            .map_err(|e| s(&e))?
            .reply()
            .map_err(|e| s(&e))?;
        if (version.major_version, version.minor_version) < (2, 1) {
            return Err("this X server's XInput is too old".into());
        }
        let mask = xinput::EventMask {
            deviceid: ALL_MASTER_DEVICES,
            mask: vec![
                XIEventMask::RAW_KEY_PRESS
                    | XIEventMask::RAW_KEY_RELEASE
                    | XIEventMask::RAW_BUTTON_PRESS,
            ],
        };
        conn.xinput_xi_select_events(root, &[mask])
            .map_err(|e| s(&e))?
            .check()
            .map_err(|e| s(&e))?;

        let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| s(&e))?
            .reply()
            .map_err(|e| s(&e))?;
        let per = usize::from(map.keysyms_per_keycode).max(1);
        let keycodes_of = |keysym: u32| -> Vec<u8> {
            map.keysyms
                .chunks(per)
                .enumerate()
                .filter(|(_, syms)| syms.contains(&keysym))
                .map(|(i, _)| min + i as u8)
                .collect()
        };
        let mut keys = Vec::new();
        let mut missing = Vec::new();
        for (keysym, key, cb) in [
            (CONTROL_R, RightKey::Ctrl, taps.right_ctrl),
            (SHIFT_R, RightKey::Shift, taps.right_shift),
        ] {
            let Some(on_tap) = cb else { continue };
            let keycodes = keycodes_of(keysym);
            if keycodes.is_empty() {
                missing.push(key);
                continue;
            }
            keys.push(Watched {
                keycodes,
                detector: TapDetector::default(),
                on_tap,
            });
        }
        let modifiers = conn
            .get_modifier_mapping()
            .map_err(|e| s(&e))?
            .reply()
            .map_err(|e| s(&e))?
            .keycodes
            .into_iter()
            .filter(|k| *k != 0)
            .collect();
        if keys.is_empty() {
            let names: Vec<&str> = missing.iter().map(|k| k.name()).collect();
            return Err(format!(
                "this keyboard layout has no {}",
                names.join(" or ")
            ));
        }
        Ok((
            Watch {
                conn,
                keys,
                modifiers,
            },
            missing,
        ))
    }

    impl Watch {
        fn run(mut self, stop: &AtomicBool) {
            // Modifiers held right now, followed from the events themselves:
            // asking the server when an event is handled can be too late
            // (Shift may already be up again).
            let mut held = self.modifiers_down_now();
            while !stop.load(Ordering::Relaxed) {
                let event = match self.conn.poll_for_event() {
                    Ok(Some(e)) => e,
                    Ok(None) => {
                        // The detector uses the events' own timestamps, so
                        // this pause doesn't skew a tap's length.
                        std::thread::sleep(Duration::from_millis(15));
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!("right-side key listener lost the X server: {e}");
                        return;
                    }
                };
                match event {
                    Event::XinputRawKeyPress(e) => {
                        let code = u8::try_from(e.detail).ok();
                        for key in &mut self.keys {
                            if code.is_some_and(|c| key.keycodes.contains(&c)) {
                                let others = held.iter().any(|h| !key.keycodes.contains(h));
                                key.detector.key_down(others, e.time);
                            } else {
                                key.detector.other_input();
                            }
                        }
                        if let Some(k) = self.modifier(e.detail) {
                            if !held.contains(&k) {
                                held.push(k);
                            }
                        }
                    }
                    Event::XinputRawButtonPress(_) => {
                        for key in &mut self.keys {
                            key.detector.other_input();
                        }
                    }
                    Event::XinputRawKeyRelease(e) => {
                        let code = u8::try_from(e.detail).ok();
                        for key in &mut self.keys {
                            // `key_up` settles the press either way; only a
                            // tap fires.
                            if code.is_some_and(|c| key.keycodes.contains(&c))
                                && key.detector.key_up(e.time)
                            {
                                (key.on_tap)();
                            }
                        }
                        if let Some(k) = self.modifier(e.detail) {
                            held.retain(|h| *h != k);
                        }
                    }
                    _ => {}
                }
            }
        }

        fn modifier(&self, keycode: u32) -> Option<u8> {
            u8::try_from(keycode)
                .ok()
                .filter(|k| self.modifiers.contains(k))
        }

        /// The other modifiers already down when the listener starts.
        fn modifiers_down_now(&self) -> Vec<u8> {
            let Ok(keys) = self
                .conn
                .query_keymap()
                .map_err(|e| e.to_string())
                .and_then(|c| c.reply().map_err(|e| e.to_string()))
            else {
                return Vec::new();
            };
            self.modifiers
                .iter()
                .copied()
                .filter(|&k| keys.keys[usize::from(k / 8)] & (1 << (k % 8)) != 0)
                .collect()
        }
    }
}

#[cfg(not(all(feature = "hotkey", any(windows, target_os = "linux"))))]
mod imp {
    use super::{RightKey, Taps};
    use crate::error::{CoreError, Result};

    pub struct Listener;

    pub fn start(_taps: Taps) -> Result<(Listener, Vec<RightKey>)> {
        Err(CoreError::NotImplemented("Tap Right Ctrl or Right Shift"))
    }
}

#[cfg(all(test, feature = "hotkey", target_os = "linux"))]
mod x11_tests {
    use super::{RightKeyListener, Taps};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Presses and releases keycodes through XTEST, the way a keyboard does
    /// (`xdotool key Control_R` also holds Left Ctrl, so it can't be used).
    /// "105" = press and release, "+105" = press, "-105" = release, "~0.8" =
    /// wait 0.8 s.
    const KEYS: &str = r#"
import sys, time
from Xlib import X, display
from Xlib.ext import xtest
d = display.Display()
for step in sys.argv[1:]:
    if step.startswith("~"):
        d.sync(); time.sleep(float(step[1:])); continue
    code = int(step.lstrip("+-"))
    if not step.startswith("-"): xtest.fake_input(d, X.KeyPress, code)
    if not step.startswith("+"): xtest.fake_input(d, X.KeyRelease, code)
    d.sync(); time.sleep(0.03)
"#;

    /// Real X server check: needs `DISPLAY` with the evdev keymap (Right Ctrl
    /// = 105, Left Ctrl = 37, Left Shift = 50, A = 38) and python3-xlib.
    #[test]
    #[ignore]
    fn x11_reports_clean_taps_only() {
        let (tx, rx) = mpsc::channel();
        let _listener = RightKeyListener::start(Taps {
            right_ctrl: Some(Box::new(move || {
                let _ = tx.send(());
            })),
            right_shift: None,
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let keys = |steps: &[&str]| {
            let ok = std::process::Command::new("python3")
                .args(["-c", KEYS])
                .args(steps)
                .status()
                .unwrap()
                .success();
            assert!(ok, "{steps:?}");
        };
        let tapped = |within_ms: u64| rx.recv_timeout(Duration::from_millis(within_ms)).is_ok();

        keys(&["105"]);
        assert!(tapped(2000), "a clean tap");
        keys(&["+50", "105", "-50"]);
        assert!(!tapped(800), "Shift held first");
        keys(&["+105", "38", "-105"]);
        assert!(!tapped(800), "Right Ctrl+A");
        keys(&["+105", "~0.8", "-105"]);
        assert!(!tapped(800), "a long press");
        keys(&["37"]);
        assert!(!tapped(800), "Left Ctrl");
        keys(&["105"]);
        assert!(tapped(2000), "a clean tap again");
    }

    /// Right Shift = 62 in the evdev keymap. Typing a capital with it is
    /// never a tap, and a Right Ctrl tap isn't one either.
    #[test]
    #[ignore]
    fn x11_reports_right_shift_taps_separately() {
        let (tx, rx) = mpsc::channel();
        let _listener = RightKeyListener::start(Taps {
            right_ctrl: None,
            right_shift: Some(Box::new(move || {
                let _ = tx.send(());
            })),
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let keys = |steps: &[&str]| {
            let ok = std::process::Command::new("python3")
                .args(["-c", KEYS])
                .args(steps)
                .status()
                .unwrap()
                .success();
            assert!(ok, "{steps:?}");
        };
        let tapped = |within_ms: u64| rx.recv_timeout(Duration::from_millis(within_ms)).is_ok();

        keys(&["62"]);
        assert!(tapped(2000), "a clean tap");
        keys(&["+62", "38", "-62"]);
        assert!(!tapped(800), "Shift+A while typing");
        keys(&["105"]);
        assert!(!tapped(800), "Right Ctrl is not Right Shift");
        keys(&["62"]);
        assert!(tapped(2000), "a clean tap again");
    }
}
