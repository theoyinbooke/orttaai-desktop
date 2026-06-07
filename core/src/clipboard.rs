//! Clipboard save/restore, used by paste-based injectors and the
//! "preserve clipboard" setting. Real backend (`arboard`) lands in Phase 1.

use crate::error::{CoreError, Result};
use std::sync::Mutex;

pub trait Clipboard: Send + Sync {
    /// Snapshot the current clipboard so it can be restored later.
    fn save(&self) -> Result<()>;
    /// Put `text` on the clipboard.
    fn set_text(&self, text: &str) -> Result<()>;
    /// Read the clipboard's current text, if any.
    fn get_text(&self) -> Result<Option<String>>;
    /// Restore the clipboard to the last `save()`.
    fn restore(&self) -> Result<()>;
}

/// In-memory clipboard for tests and `demo`.
#[derive(Default)]
pub struct MockClipboard {
    current: Mutex<Option<String>>,
    saved: Mutex<Option<String>>,
}

impl MockClipboard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clipboard for MockClipboard {
    fn save(&self) -> Result<()> {
        *self.saved.lock().unwrap() = self.current.lock().unwrap().clone();
        Ok(())
    }

    fn set_text(&self, text: &str) -> Result<()> {
        *self.current.lock().unwrap() = Some(text.to_string());
        Ok(())
    }

    fn get_text(&self) -> Result<Option<String>> {
        Ok(self.current.lock().unwrap().clone())
    }

    fn restore(&self) -> Result<()> {
        *self.current.lock().unwrap() = self.saved.lock().unwrap().clone();
        Ok(())
    }
}

/// Real OS clipboard. Phase 1 wires `arboard`.
pub struct SystemClipboard;

impl SystemClipboard {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard for SystemClipboard {
    fn save(&self) -> Result<()> {
        Err(CoreError::NotImplemented("system clipboard (Phase 1)"))
    }
    fn set_text(&self, _text: &str) -> Result<()> {
        Err(CoreError::NotImplemented("system clipboard (Phase 1)"))
    }
    fn get_text(&self) -> Result<Option<String>> {
        Err(CoreError::NotImplemented("system clipboard (Phase 1)"))
    }
    fn restore(&self) -> Result<()> {
        Err(CoreError::NotImplemented("system clipboard (Phase 1)"))
    }
}
