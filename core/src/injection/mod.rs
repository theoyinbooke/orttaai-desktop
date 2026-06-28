//! Text injection at the cursor. Real backends land in Phase 1:
//! `enigo`/`SendInput` (Windows), `xdotool` (X11), `wtype` (Wayland).
//!
//! Secure-field detection has no reliable Wayland equivalent — the trait returns
//! `SecureFieldStatus::Unknown` there (see `docs/gaps.md`).

use crate::error::{CoreError, Result};
use crate::types::{InjectionResult, SecureFieldStatus};
use std::sync::{Arc, Mutex};

pub trait TextInjector: Send + Sync {
    /// Make `text` appear at the cursor in the focused application.
    fn inject(&self, text: &str) -> Result<InjectionResult>;

    /// Best-effort secure/password-field check for the focused element.
    fn is_secure_field_focused(&self) -> SecureFieldStatus;

    /// Human-readable backend name, for diagnostics.
    fn backend_name(&self) -> &'static str;
}

/// Shared, cloneable record of what a [`MockTextInjector`] "typed" — lets a test
/// inspect injected text after handing ownership of the injector to the coordinator.
#[derive(Clone, Default)]
pub struct InjectionLog(Arc<Mutex<Vec<String>>>);

impl InjectionLog {
    pub fn last(&self) -> Option<String> {
        self.0.lock().unwrap().last().cloned()
    }

    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Records injected text instead of touching the OS. Used by tests and `demo`.
pub struct MockTextInjector {
    log: InjectionLog,
    secure: SecureFieldStatus,
    fail: bool,
}

impl MockTextInjector {
    pub fn new() -> Self {
        Self {
            log: InjectionLog::default(),
            secure: SecureFieldStatus::NotSecure,
            fail: false,
        }
    }

    /// Simulate a focused secure field (so the coordinator blocks injection).
    pub fn secure() -> Self {
        Self {
            secure: SecureFieldStatus::Secure,
            ..Self::new()
        }
    }

    /// Simulate an unverifiable field (the honest Linux/Wayland answer) so the
    /// `strict_secure` policy can be exercised.
    pub fn unknown() -> Self {
        Self {
            secure: SecureFieldStatus::Unknown,
            ..Self::new()
        }
    }

    /// Simulate injection that fails (e.g. wtype/portal can't type into the
    /// focused app) so the transcript-preserving fallback can be exercised.
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new()
        }
    }

    /// A handle to inspect what was injected.
    pub fn log(&self) -> InjectionLog {
        self.log.clone()
    }
}

impl Default for MockTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for MockTextInjector {
    fn inject(&self, text: &str) -> Result<InjectionResult> {
        if self.fail {
            return Err(CoreError::Injection("mock injection failure".to_string()));
        }
        self.log.0.lock().unwrap().push(text.to_string());
        Ok(InjectionResult::Success)
    }

    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        self.secure
    }

    fn backend_name(&self) -> &'static str {
        "mock"
    }
}

/// The real, OS-selected injector.
///
/// With the `injection` feature it types text via `enigo` (Windows `SendInput`,
/// macOS `CGEvent`, Linux X11) and falls back to `wtype` on Linux/Wayland, where
/// `enigo` is unreliable. Without the feature it returns
/// [`CoreError::NotImplemented`] so behavior stays honest.
///
/// Secure/password-field detection uses UI Automation on Windows; Linux has no
/// reliable per-field equivalent, so
/// [`is_secure_field_focused`](TextInjector::is_secure_field_focused) returns
/// [`SecureFieldStatus::Unknown`] there (pair it with the `strict_secure`
/// setting — see `docs/gaps.md`).
pub struct SystemTextInjector {
    #[cfg(all(feature = "injection", target_os = "linux"))]
    wayland: bool,
    #[cfg(all(feature = "portal", target_os = "linux"))]
    portal: Option<portal::WaylandPortalInjector>,
}

impl SystemTextInjector {
    pub fn new() -> Self {
        Self {
            #[cfg(all(feature = "injection", target_os = "linux"))]
            wayland: is_wayland_session(),
            #[cfg(all(feature = "portal", target_os = "linux"))]
            portal: init_portal(),
        }
    }
}

/// On Wayland, try to open a RemoteDesktop portal session (shows the permission
/// dialog the first time). Returns `None` if unavailable or denied so we fall
/// back to `wtype`/clipboard.
#[cfg(all(feature = "portal", target_os = "linux"))]
fn init_portal() -> Option<portal::WaylandPortalInjector> {
    if !is_wayland_session() {
        return None;
    }
    match portal::WaylandPortalInjector::new() {
        Ok(p) if p.is_ready() => {
            tracing::info!("Wayland injection via the RemoteDesktop portal");
            Some(p)
        }
        Ok(_) => {
            tracing::warn!("RemoteDesktop portal not granted; falling back to wtype");
            None
        }
        Err(e) => {
            tracing::warn!("RemoteDesktop portal unavailable: {e}");
            None
        }
    }
}

impl Default for SystemTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for SystemTextInjector {
    fn inject(&self, text: &str) -> Result<InjectionResult> {
        if text.is_empty() {
            return Ok(InjectionResult::NoTranscript);
        }
        self.type_text(text)
    }

    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        detect_secure_field()
    }

    #[allow(clippy::needless_return)]
    fn backend_name(&self) -> &'static str {
        #[cfg(not(feature = "injection"))]
        return "system (stub — build with --features injection)";
        #[cfg(all(feature = "injection", target_os = "windows"))]
        return "enigo/SendInput";
        #[cfg(all(feature = "injection", target_os = "macos"))]
        return "enigo/CGEvent";
        #[cfg(all(feature = "injection", target_os = "linux"))]
        return if self.wayland {
            "wtype (Wayland)"
        } else {
            "enigo (X11)"
        };
        #[cfg(all(
            feature = "injection",
            not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
        ))]
        return "enigo";
    }
}

#[cfg(feature = "injection")]
impl SystemTextInjector {
    fn type_text(&self, text: &str) -> Result<InjectionResult> {
        #[cfg(all(feature = "portal", target_os = "linux"))]
        if let Some(portal) = &self.portal {
            return portal.inject(text);
        }
        #[cfg(target_os = "linux")]
        if self.wayland {
            return type_via_wtype(text);
        }
        type_via_enigo(text)
    }
}

#[cfg(not(feature = "injection"))]
impl SystemTextInjector {
    fn type_text(&self, _text: &str) -> Result<InjectionResult> {
        Err(CoreError::NotImplemented(
            "system text injection — rebuild with --features injection",
        ))
    }
}

#[cfg(feature = "injection")]
fn type_via_enigo(text: &str) -> Result<InjectionResult> {
    use enigo::{Enigo, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| CoreError::Injection(format!("enigo init: {e}")))?;
    enigo
        .text(text)
        .map_err(|e| CoreError::Injection(format!("type text: {e}")))?;
    Ok(InjectionResult::Success)
}

#[cfg(all(feature = "injection", target_os = "linux"))]
fn type_via_wtype(text: &str) -> Result<InjectionResult> {
    match std::process::Command::new("wtype").arg(text).status() {
        Ok(status) if status.success() => Ok(InjectionResult::Success),
        Ok(status) => Err(CoreError::Injection(format!("wtype exited with {status}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(CoreError::Injection(
            "Wayland typing needs the `wtype` tool, which isn't installed. \
             Install it with `sudo apt install wtype` (Debian/Ubuntu) or \
             `sudo dnf install wtype` (Fedora), then try again."
                .to_string(),
        )),
        Err(e) => Err(CoreError::Injection(format!("failed to run wtype: {e}"))),
    }
}

/// Whether the `wtype` binary is available on `PATH` (used for first-run checks).
#[cfg(all(feature = "injection", target_os = "linux"))]
pub fn wtype_available() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("wtype").is_file()))
        .unwrap_or(false)
}

#[cfg(all(feature = "injection", target_os = "linux"))]
fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

#[cfg(all(feature = "portal", target_os = "linux"))]
mod portal;

// ---- Secure-field detection -------------------------------------------------

/// Best-effort check of whether keyboard focus is in a password/secure field.
///
/// - **Windows**: UI Automation's `IsPassword` on the focused element — reliable.
/// - **Linux/Wayland & others**: no reliable per-field API exists, so this is
///   `Unknown` (honest). The coordinator's `strict_secure` policy decides whether
///   `Unknown` should block injection.
///
/// Always fails toward `Unknown` (never panics) so a detection hiccup can't brick
/// dictation.
#[cfg(all(feature = "injection", target_os = "windows"))]
fn detect_secure_field() -> SecureFieldStatus {
    secure_windows::focused_field_status()
}

#[cfg(not(all(feature = "injection", target_os = "windows")))]
fn detect_secure_field() -> SecureFieldStatus {
    SecureFieldStatus::Unknown
}

/// Windows secure-field detection via UI Automation.
///
/// NOTE: this is `#[cfg(windows)]`-only and cannot be compiled on the Linux dev
/// host — it is verified only by the Windows CI build. Every step fails safe to
/// `Unknown`.
#[cfg(all(feature = "injection", target_os = "windows"))]
mod secure_windows {
    use crate::types::SecureFieldStatus;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    pub fn focused_field_status() -> SecureFieldStatus {
        match unsafe { query_focused_is_password() } {
            Ok(true) => SecureFieldStatus::Secure,
            Ok(false) => SecureFieldStatus::NotSecure,
            Err(_) => SecureFieldStatus::Unknown,
        }
    }

    unsafe fn query_focused_is_password() -> windows::core::Result<bool> {
        // Idempotent on this (long-lived worker) thread; a repeat call returns
        // S_FALSE and a different-apartment call returns RPC_E_CHANGED_MODE —
        // both harmless, so the HRESULT is ignored.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let focused = automation.GetFocusedElement()?;
        Ok(focused.CurrentIsPassword()?.as_bool())
    }
}
