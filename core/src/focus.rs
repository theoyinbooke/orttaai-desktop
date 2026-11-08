//! Best-effort detection of the currently focused application — recorded as the
//! "source" each dictation is typed into.
//!
//! Reality of the platforms:
//! - **X11**: `_NET_ACTIVE_WINDOW` → the window's `WM_CLASS`. Reliable.
//! - **Windows**: `GetForegroundWindow` → owning process image name. Reliable.
//! - **Wayland (GNOME etc.)**: there is **no** API a normal client may use to
//!   learn the focused app — the compositor refuses it for security (GNOME's
//!   `Shell.Introspect.GetWindows` returns `AccessDenied`, and no portal exists).
//!   So this returns `None` there, and the dictation is stored without a source.

use crate::injection::edit_watch::FieldRead;

/// The focused text field's full value, for learn-from-edits. `None` when it
/// can't be read (Linux has no portable API; password fields are never read).
pub fn focused_field_text() -> Option<String> {
    match focused_field_read() {
        FieldRead::Text(t) => Some(t),
        _ => None,
    }
}

/// Read the focused field: its text, `Secure` for a password field (never
/// read), or `Unreadable`.
///
/// - **Windows**: UI Automation — `ValuePattern.CurrentValue`, falling back to
///   `TextPattern.DocumentRange.GetText` (rich edits, browsers).
/// - **Linux / others**: no API a normal client may use (AT-SPI is unreliable
///   and absent under Wayland sandboxes), so always `Unreadable`.
#[cfg(target_os = "windows")]
pub fn focused_field_read() -> FieldRead {
    uia_field::read()
}

#[cfg(not(target_os = "windows"))]
pub fn focused_field_read() -> FieldRead {
    FieldRead::Unreadable
}

/// NOTE: `#[cfg(windows)]`-only; verified by the Windows cross-check build.
/// Every step fails safe to `Unreadable`.
#[cfg(target_os = "windows")]
mod uia_field {
    use crate::injection::edit_watch::FieldRead;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationValuePattern,
        UIA_TextPatternId, UIA_ValuePatternId,
    };

    /// Fields longer than this are not worth diffing (a whole document).
    const MAX_CHARS: i32 = 20_000;

    pub fn read() -> FieldRead {
        unsafe { read_inner() }.unwrap_or(FieldRead::Unreadable)
    }

    unsafe fn read_inner() -> windows::core::Result<FieldRead> {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let focused = automation.GetFocusedElement()?;
        if focused.CurrentIsPassword()?.as_bool() {
            return Ok(FieldRead::Secure);
        }
        if let Ok(value) =
            focused.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            if let Ok(v) = value.CurrentValue() {
                return Ok(FieldRead::Text(v.to_string()));
            }
        }
        if let Ok(text) = focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            let range = text.DocumentRange()?;
            let v = range.GetText(MAX_CHARS)?;
            return Ok(FieldRead::Text(v.to_string()));
        }
        Ok(FieldRead::Unreadable)
    }
}

/// The window a dictation is typed into, so text never lands somewhere else
/// when focus moves while it's processed (Mac `TextInjectionService`'s target
/// app). Window ids are X11 window ids or Windows HWNDs; Wayland has none.
pub trait WindowFocus: Send + Sync {
    /// The focused top-level window, or `None` when it can't be known.
    fn current(&self) -> Option<u64>;
    /// Ask for `window` to come to the front. `false` when the request failed.
    fn activate(&self, window: u64) -> bool;
    /// Pause between checks while waiting for the window to come back.
    fn wait(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// The OS's windows.
pub struct SystemWindowFocus;

impl WindowFocus for SystemWindowFocus {
    fn current(&self) -> Option<u64> {
        focused_window()
    }

    fn activate(&self, window: u64) -> bool {
        activate_window(window)
    }
}

/// Scripted windows for tests: `front` changes when `activate` is honoured or
/// after `returns_after` waits.
pub struct ScriptedWindowFocus {
    pub front: std::sync::Mutex<Option<u64>>,
    /// Whether `activate` brings the window forward.
    pub activation_works: bool,
    /// The user brings `target` back after this many waits (None: never).
    pub returns_after: Option<(u64, usize)>,
    pub waits: std::sync::atomic::AtomicUsize,
}

impl ScriptedWindowFocus {
    pub fn new(front: Option<u64>) -> Self {
        Self {
            front: std::sync::Mutex::new(front),
            activation_works: false,
            returns_after: None,
            waits: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl WindowFocus for ScriptedWindowFocus {
    fn current(&self) -> Option<u64> {
        *self.front.lock().unwrap()
    }

    fn activate(&self, window: u64) -> bool {
        if self.activation_works {
            *self.front.lock().unwrap() = Some(window);
        }
        self.activation_works
    }

    fn wait(&self, _ms: u64) {
        let n = self.waits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if let Some((window, after)) = self.returns_after {
            if n >= after {
                *self.front.lock().unwrap() = Some(window);
            }
        }
    }
}

impl<W: WindowFocus + ?Sized> WindowFocus for std::sync::Arc<W> {
    fn current(&self) -> Option<u64> {
        (**self).current()
    }
    fn activate(&self, window: u64) -> bool {
        (**self).activate(window)
    }
    fn wait(&self, ms: u64) {
        (**self).wait(ms)
    }
}

/// The focused top-level window's id.
#[cfg(target_os = "linux")]
pub fn focused_window() -> Option<u64> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let net_active = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let active = conn
        .get_property(false, root, net_active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let win = active.value32().and_then(|mut it| it.next())?;
    (win != 0).then_some(win as u64)
}

#[cfg(target_os = "windows")]
pub fn focused_window() -> Option<u64> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as usize as u64)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn focused_window() -> Option<u64> {
    None
}

/// Ask the window manager to bring `window` forward (EWMH `_NET_ACTIVE_WINDOW`,
/// as a pager would).
#[cfg(target_os = "linux")]
pub fn activate_window(window: u64) -> bool {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let Some(net_active) = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
    else {
        return false;
    };
    // Source 2 = pager: window managers honour it without focus-stealing checks.
    let event = ClientMessageEvent::new(32, window as u32, net_active, [2, 0, 0, 0, 0]);
    let sent = conn
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .is_ok();
    // Wait for a reply before disconnecting: closing straight after the flush
    // can drop the request before the server has handed it to the window
    // manager (seen with fluxbox).
    sent && conn.flush().is_ok() && conn.get_input_focus().is_ok_and(|c| c.reply().is_ok())
}

/// NOTE: `#[cfg(windows)]`-only; verified by the Windows cross-check build.
#[cfg(target_os = "windows")]
pub fn activate_window(window: u64) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let hwnd = HWND(window as usize as *mut core::ffi::c_void);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn activate_window(_window: u64) -> bool {
    false
}

/// The focused app's name, or `None` when it can't be determined (e.g. Wayland).
#[cfg(target_os = "linux")]
pub fn focused_app() -> Option<String> {
    focused_app_x11()
}

#[cfg(target_os = "windows")]
pub fn focused_app() -> Option<String> {
    focused_app_windows()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn focused_app() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn focused_app_x11() -> Option<String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    // On Wayland there's no usable X active-window, so this connect-or-query
    // simply yields nothing — exactly the intended fallback.
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;

    let net_active = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let active = conn
        .get_property(false, root, net_active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let win = active.value32().and_then(|mut it| it.next())?;
    if win == 0 {
        return None;
    }

    // WM_CLASS is "instance\0class\0"; the class (second field) is the app name.
    let class = conn
        .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    let parts: Vec<&[u8]> = class
        .value
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    let name = parts.get(1).or_else(|| parts.first())?;
    let s = String::from_utf8_lossy(name).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// NOTE: `#[cfg(windows)]`-only — cannot be compiled on the Linux dev host; it is
/// verified by the Windows CI build. Fails safe to `None`.
#[cfg(target_os = "windows")]
fn focused_app_windows() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
        let stem = file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".EXE"))
            .unwrap_or(file);
        if stem.is_empty() {
            None
        } else {
            Some(stem.to_string())
        }
    }
}
