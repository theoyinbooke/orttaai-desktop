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
}

impl MockTextInjector {
    pub fn new() -> Self {
        Self {
            log: InjectionLog::default(),
            secure: SecureFieldStatus::NotSecure,
        }
    }

    /// Simulate a focused secure field (so the coordinator blocks injection).
    pub fn secure() -> Self {
        Self {
            log: InjectionLog::default(),
            secure: SecureFieldStatus::Secure,
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
/// Secure/password-field detection has no reliable Linux equivalent, so
/// [`is_secure_field_focused`](TextInjector::is_secure_field_focused) returns
/// [`SecureFieldStatus::Unknown`] (see `docs/gaps.md`).
pub struct SystemTextInjector {
    #[cfg(all(feature = "injection", target_os = "linux"))]
    wayland: bool,
}

impl SystemTextInjector {
    pub fn new() -> Self {
        Self {
            #[cfg(all(feature = "injection", target_os = "linux"))]
            wayland: is_wayland_session(),
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
        // No reliable cross-platform detection — honest by default.
        SecureFieldStatus::Unknown
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
    let status = std::process::Command::new("wtype")
        .arg(text)
        .status()
        .map_err(|e| CoreError::Injection(format!("spawn wtype (install `wtype`): {e}")))?;
    if status.success() {
        Ok(InjectionResult::Success)
    } else {
        Err(CoreError::Injection(format!("wtype exited with {status}")))
    }
}

#[cfg(all(feature = "injection", target_os = "linux"))]
fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}
