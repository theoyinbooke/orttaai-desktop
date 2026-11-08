//! "Tap Right Ctrl to dictate" (the Mac's "Tap Right Option to Dictate"): a
//! listen-only watch on the keyboard that reports each clean tap of Right Ctrl
//! (see [`TapDetector`](super::tap::TapDetector)). Keys are never swallowed, so Ctrl shortcuts keep
//! working everywhere.
//!
//! - Windows: low-level keyboard and mouse hooks (no permission needed).
//! - Linux on X11: XInput2 raw key and button events on the root window.
//! - Wayland doesn't let an app watch keys it doesn't own, so the option is
//!   unavailable there; the normal shortcut still works.

use crate::error::{CoreError, Result};

/// Called on each clean tap, on the listener's own thread.
pub type TapCallback = Box<dyn Fn() + Send + Sync + 'static>;

/// Why the key can't be watched on this system, if it can't.
pub fn unavailable_reason() -> Option<&'static str> {
    if cfg!(not(all(
        feature = "hotkey",
        any(windows, target_os = "linux")
    ))) {
        return Some("Tap Right Ctrl isn't available on this system.");
    }
    if crate::injection::is_wayland_session() {
        return Some(
            "Wayland doesn't let apps watch a single key, so this works on X11 and Windows. Use your shortcut instead.",
        );
    }
    None
}

/// Watches Right Ctrl until dropped.
pub struct RightCtrlListener {
    #[allow(dead_code)]
    inner: imp::Listener,
}

impl RightCtrlListener {
    pub fn start(on_tap: TapCallback) -> Result<Self> {
        if let Some(reason) = unavailable_reason() {
            return Err(CoreError::Hotkey(reason.into()));
        }
        Ok(Self {
            inner: imp::start(on_tap)?,
        })
    }
}

#[cfg(all(feature = "hotkey", windows))]
mod imp {
    use super::TapCallback;
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

    /// Modifiers that, already held, make a Right Ctrl press part of a combo.
    const OTHER_MODIFIERS: [u16; 7] = [
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ];

    struct Hook {
        detector: TapDetector,
        on_tap: Arc<dyn Fn() + Send + Sync>,
    }

    /// Hook procedures are plain functions, so their state lives here. One
    /// listener at a time.
    static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub struct Listener {
        thread_id: u32,
        handle: Option<JoinHandle<()>>,
    }

    pub fn start(on_tap: TapCallback) -> Result<Listener> {
        {
            let mut slot = HOOK.lock().unwrap();
            if slot.is_some() {
                return Err(CoreError::Hotkey(
                    "Right Ctrl is already being watched".into(),
                ));
            }
            *slot = Some(Hook {
                detector: TapDetector::default(),
                on_tap: Arc::from(on_tap),
            });
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("orttaai-right-ctrl".into())
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
            Ok(Ok(thread_id)) => Ok(Listener {
                thread_id,
                handle: Some(handle),
            }),
            other => {
                let _ = handle.join();
                *HOOK.lock().unwrap() = None;
                let why = match other {
                    Ok(Err(e)) => e,
                    _ => "the listener stopped".into(),
                };
                Err(CoreError::Hotkey(format!(
                    "couldn't watch Right Ctrl: {why}"
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

    fn with_hook(f: impl FnOnce(&mut Hook) -> bool) {
        let fire = {
            let Ok(mut slot) = HOOK.lock() else { return };
            match slot.as_mut() {
                Some(hook) => f(hook).then(|| Arc::clone(&hook.on_tap)),
                None => None,
            }
        };
        if let Some(on_tap) = fire {
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
                    if vk == u32::from(VK_RCONTROL) {
                        if up {
                            return hook.detector.key_up(time);
                        }
                        let others = OTHER_MODIFIERS
                            .iter()
                            .any(|&k| GetAsyncKeyState(i32::from(k)) < 0);
                        hook.detector.key_down(others, time);
                    } else if !up {
                        hook.detector.other_input();
                    }
                    false
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
                hook.detector.other_input();
                false
            });
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

#[cfg(all(feature = "hotkey", target_os = "linux"))]
mod imp {
    use super::TapCallback;
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

    /// The `Control_R` keysym.
    const CONTROL_R: u32 = 0xffe4;
    /// `XIAllMasterDevices`.
    const ALL_MASTER_DEVICES: u16 = 1;

    pub struct Listener {
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    pub fn start(on_tap: TapCallback) -> Result<Listener> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop2 = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("orttaai-right-ctrl".into())
            .spawn(move || match connect() {
                Ok(watch) => {
                    let _ = tx.send(Ok(()));
                    watch.run(&stop2, &*on_tap);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })?;
        match rx.recv() {
            Ok(Ok(())) => Ok(Listener {
                stop,
                handle: Some(handle),
            }),
            other => {
                let _ = handle.join();
                let why = match other {
                    Ok(Err(e)) => e,
                    _ => "the listener stopped".into(),
                };
                Err(CoreError::Hotkey(format!(
                    "couldn't watch Right Ctrl: {why}"
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

    struct Watch {
        conn: RustConnection,
        /// Keycodes that produce Right Ctrl.
        targets: Vec<u8>,
        /// Every other modifier key's keycode.
        modifiers: Vec<u8>,
    }

    fn connect() -> std::result::Result<Watch, String> {
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
        let targets: Vec<u8> = map
            .keysyms
            .chunks(per)
            .enumerate()
            .filter(|(_, syms)| syms.contains(&CONTROL_R))
            .map(|(i, _)| min + i as u8)
            .collect();
        if targets.is_empty() {
            return Err("this keyboard layout has no Right Ctrl".into());
        }
        let modifiers = conn
            .get_modifier_mapping()
            .map_err(|e| s(&e))?
            .reply()
            .map_err(|e| s(&e))?
            .keycodes
            .into_iter()
            .filter(|k| *k != 0 && !targets.contains(k))
            .collect();
        Ok(Watch {
            conn,
            targets,
            modifiers,
        })
    }

    impl Watch {
        fn run(self, stop: &AtomicBool, on_tap: &(dyn Fn() + Send + Sync)) {
            let mut detector = TapDetector::default();
            // Other modifiers held right now, followed from the events
            // themselves: asking the server when an event is handled can be
            // too late (Shift may already be up again).
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
                        tracing::warn!("Right Ctrl listener lost the X server: {e}");
                        return;
                    }
                };
                match event {
                    Event::XinputRawKeyPress(e) if self.is_target(e.detail) => {
                        detector.key_down(!held.is_empty(), e.time);
                    }
                    Event::XinputRawKeyPress(e) => {
                        if let Some(k) = self.modifier(e.detail) {
                            if !held.contains(&k) {
                                held.push(k);
                            }
                        }
                        detector.other_input();
                    }
                    Event::XinputRawButtonPress(_) => detector.other_input(),
                    // `key_up` settles the press either way; only a tap fires.
                    Event::XinputRawKeyRelease(e)
                        if self.is_target(e.detail) && detector.key_up(e.time) =>
                    {
                        on_tap();
                    }
                    Event::XinputRawKeyRelease(e) => {
                        if let Some(k) = self.modifier(e.detail) {
                            held.retain(|h| *h != k);
                        }
                    }
                    _ => {}
                }
            }
        }

        fn is_target(&self, keycode: u32) -> bool {
            u8::try_from(keycode).is_ok_and(|k| self.targets.contains(&k))
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
    use super::TapCallback;
    use crate::error::{CoreError, Result};

    pub struct Listener;

    pub fn start(_on_tap: TapCallback) -> Result<Listener> {
        Err(CoreError::NotImplemented("Tap Right Ctrl"))
    }
}

#[cfg(all(test, feature = "hotkey", target_os = "linux"))]
mod x11_tests {
    use super::RightCtrlListener;
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
        let _listener = RightCtrlListener::start(Box::new(move || {
            let _ = tx.send(());
        }))
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
}
