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

/// The real, OS-selected injector. Phase 1 fills these in; for now each target
/// returns [`CoreError::NotImplemented`] so behavior is honest, not faked.
pub struct SystemTextInjector;

impl SystemTextInjector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for SystemTextInjector {
    fn inject(&self, _text: &str) -> Result<InjectionResult> {
        Err(CoreError::NotImplemented("system text injection (Phase 1)"))
    }

    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        // The honest default until real detection exists per platform.
        SecureFieldStatus::Unknown
    }

    fn backend_name(&self) -> &'static str {
        #[cfg(target_os = "windows")]
        {
            "SendInput (stub)"
        }
        #[cfg(target_os = "linux")]
        {
            "wtype/xdotool (stub)"
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            "unsupported-host (stub)"
        }
    }
}
