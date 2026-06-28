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
