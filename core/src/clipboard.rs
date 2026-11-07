//! Clipboard save/restore, used by paste injection, selection capture for
//! voice edit, and the "preserve clipboard" setting (Mac `ClipboardManager`).
//!
//! The real backend is `arboard` (Windows, X11, XWayland). On a Wayland session
//! with `wl-clipboard` installed it shells out to `wl-copy`/`wl-paste`, which
//! talk to the compositor directly (arboard's X11 path can't see native Wayland
//! apps' clipboard). Only plain text is saved and restored — rich clipboard
//! content (images, files) is not preserved (see `docs/gaps.md`).

use crate::error::{CoreError, Result};
use std::sync::Mutex;

pub trait Clipboard: Send + Sync {
    /// Snapshot the current clipboard so it can be restored later.
    fn save(&self) -> Result<()>;
    /// Put `text` on the clipboard.
    fn set_text(&self, text: &str) -> Result<()>;
    /// Read the clipboard's current text, if any.
    fn get_text(&self) -> Result<Option<String>>;
    /// Restore the clipboard to the last `save()` (clears it when it was empty).
    fn restore(&self) -> Result<()>;
}

/// In-memory clipboard for tests and `demo`.
#[derive(Default)]
pub struct MockClipboard {
    current: Mutex<Option<String>>,
    saved: Mutex<Option<String>>,
    /// Every value written by `set_text`/`restore`, for assertions.
    history: Mutex<Vec<Option<String>>>,
}

impl MockClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// A clipboard that already holds `text` (the user's own copy).
    pub fn with_text(text: &str) -> Self {
        let c = Self::default();
        *c.current.lock().unwrap() = Some(text.to_string());
        c
    }

    pub fn history(&self) -> Vec<Option<String>> {
        self.history.lock().unwrap().clone()
    }
}

impl Clipboard for MockClipboard {
    fn save(&self) -> Result<()> {
        *self.saved.lock().unwrap() = self.current.lock().unwrap().clone();
        Ok(())
    }

    fn set_text(&self, text: &str) -> Result<()> {
        *self.current.lock().unwrap() = Some(text.to_string());
        self.history.lock().unwrap().push(Some(text.to_string()));
        Ok(())
    }

    fn get_text(&self) -> Result<Option<String>> {
        Ok(self.current.lock().unwrap().clone())
    }

    fn restore(&self) -> Result<()> {
        let saved = self.saved.lock().unwrap().clone();
        *self.current.lock().unwrap() = saved.clone();
        self.history.lock().unwrap().push(saved);
        Ok(())
    }
}

/// `Arc<C>` is a clipboard too, so a test can keep a handle to a mock it gave away.
impl<C: Clipboard + ?Sized> Clipboard for std::sync::Arc<C> {
    fn save(&self) -> Result<()> {
        (**self).save()
    }
    fn set_text(&self, text: &str) -> Result<()> {
        (**self).set_text(text)
    }
    fn get_text(&self) -> Result<Option<String>> {
        (**self).get_text()
    }
    fn restore(&self) -> Result<()> {
        (**self).restore()
    }
}

/// The real OS clipboard (text only).
pub struct SystemClipboard {
    saved: Mutex<Option<Option<String>>>,
    #[cfg(feature = "injection")]
    board: Mutex<Option<arboard::Clipboard>>,
    /// Native Wayland session with `wl-copy`/`wl-paste` on PATH.
    #[cfg(target_os = "linux")]
    wayland_tools: bool,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self {
            saved: Mutex::new(None),
            #[cfg(feature = "injection")]
            board: Mutex::new(None),
            #[cfg(target_os = "linux")]
            wayland_tools: wayland_session() && on_path("wl-copy") && on_path("wl-paste"),
        }
    }

    /// Human-readable backend, for diagnostics / the setup platform check.
    pub fn backend_name(&self) -> &'static str {
        #[cfg(target_os = "linux")]
        if self.wayland_tools {
            return "wl-clipboard (Wayland)";
        }
        if cfg!(feature = "injection") {
            "arboard"
        } else {
            "unavailable (build with --features injection)"
        }
    }

    fn read(&self) -> Result<Option<String>> {
        #[cfg(target_os = "linux")]
        if self.wayland_tools {
            return wl::paste();
        }
        self.board_get()
    }

    fn write(&self, text: Option<&str>) -> Result<()> {
        #[cfg(target_os = "linux")]
        if self.wayland_tools {
            return wl::copy(text);
        }
        self.board_set(text)
    }

    #[cfg(feature = "injection")]
    fn with_board<T>(&self, f: impl FnOnce(&mut arboard::Clipboard) -> Result<T>) -> Result<T> {
        let mut guard = self.board.lock().unwrap();
        if guard.is_none() {
            // Kept alive for the app's lifetime: on X11 the clipboard owner must
            // stay around to serve the pasted text to the target app.
            *guard = Some(
                arboard::Clipboard::new()
                    .map_err(|e| CoreError::Injection(format!("clipboard unavailable: {e}")))?,
            );
        }
        f(guard.as_mut().expect("clipboard initialised"))
    }

    #[cfg(feature = "injection")]
    fn board_get(&self) -> Result<Option<String>> {
        self.with_board(|b| match b.get_text() {
            Ok(t) => Ok(Some(t)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(CoreError::Injection(format!("read clipboard: {e}"))),
        })
    }

    #[cfg(feature = "injection")]
    fn board_set(&self, text: Option<&str>) -> Result<()> {
        self.with_board(|b| {
            match text {
                Some(t) => b.set_text(t.to_string()),
                None => b.clear(),
            }
            .map_err(|e| CoreError::Injection(format!("write clipboard: {e}")))
        })
    }

    #[cfg(not(feature = "injection"))]
    fn board_get(&self) -> Result<Option<String>> {
        Err(CoreError::NotImplemented(
            "system clipboard — rebuild with --features injection",
        ))
    }

    #[cfg(not(feature = "injection"))]
    fn board_set(&self, _text: Option<&str>) -> Result<()> {
        Err(CoreError::NotImplemented(
            "system clipboard — rebuild with --features injection",
        ))
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard for SystemClipboard {
    fn save(&self) -> Result<()> {
        // A clipboard that can't be read (empty, non-text, owner not answering)
        // is saved as "nothing", so restore clears our staged text instead of
        // leaving it behind.
        let current = self.read().unwrap_or(None);
        *self.saved.lock().unwrap() = Some(current);
        Ok(())
    }

    fn set_text(&self, text: &str) -> Result<()> {
        self.write(Some(text))
    }

    fn get_text(&self) -> Result<Option<String>> {
        self.read()
    }

    fn restore(&self) -> Result<()> {
        let saved = self.saved.lock().unwrap().take();
        match saved {
            Some(previous) => self.write(previous.as_deref()),
            None => Ok(()),
        }
    }
}

#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

/// Whether `bin` is an executable on `PATH`.
pub fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(bin);
                p.is_file() || (cfg!(windows) && dir.join(format!("{bin}.exe")).is_file())
            })
        })
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
mod wl {
    use crate::error::{CoreError, Result};
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// How long a clipboard read may take before we give up (Mac
    /// `ClipboardManager.saveLimitMs`): a source app that never answers must
    /// not hold up a paste.
    const READ_LIMIT: Duration = Duration::from_millis(1000);

    pub fn paste() -> Result<Option<String>> {
        let mut child = Command::new("wl-paste")
            .args(["--no-newline", "--type", "text"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| CoreError::Injection(format!("wl-paste: {e}")))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        });
        let deadline = Instant::now() + READ_LIMIT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let buf = reader.join().unwrap_or_default();
        match status {
            // wl-paste exits non-zero when nothing (or no text) is copied.
            Some(s) if s.success() => Ok(Some(String::from_utf8_lossy(&buf).into_owned())),
            Some(_) => Ok(None),
            None => Err(CoreError::Injection("clipboard read timed out".into())),
        }
    }

    pub fn copy(text: Option<&str>) -> Result<()> {
        let mut cmd = Command::new("wl-copy");
        if text.is_none() {
            cmd.arg("--clear");
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| CoreError::Injection(format!("wl-copy: {e}")))?;
        if let (Some(mut stdin), Some(t)) = (child.stdin.take(), text) {
            stdin
                .write_all(t.as_bytes())
                .map_err(|e| CoreError::Injection(format!("wl-copy: {e}")))?;
        }
        // wl-copy forks a server and the parent exits once it owns the
        // selection; wait for that so the paste that follows sees our text.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(s)) if s.success() => return Ok(()),
                Ok(Some(s)) => {
                    return Err(CoreError::Injection(format!("wl-copy exited with {s}")))
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_save_set_restore_round_trips() {
        let c = MockClipboard::with_text("user's own copy");
        c.save().unwrap();
        c.set_text("dictated").unwrap();
        assert_eq!(c.get_text().unwrap().as_deref(), Some("dictated"));
        c.restore().unwrap();
        assert_eq!(c.get_text().unwrap().as_deref(), Some("user's own copy"));
    }

    #[test]
    fn restoring_an_empty_clipboard_clears_it() {
        let c = MockClipboard::new();
        c.save().unwrap();
        c.set_text("dictated").unwrap();
        c.restore().unwrap();
        assert_eq!(c.get_text().unwrap(), None);
    }

    #[test]
    fn on_path_finds_nothing_bogus() {
        assert!(!on_path("definitely-not-a-real-binary-orttaai"));
    }
}
