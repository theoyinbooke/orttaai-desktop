//! Clipboard save/restore, used by paste injection, selection capture for
//! voice edit, and the "preserve clipboard" setting (Mac `ClipboardManager`).
//!
//! The real backend is `arboard` (Windows, X11, XWayland). On a Wayland session
//! with `wl-clipboard` installed it shells out to `wl-copy`/`wl-paste`, which
//! talk to the compositor directly (arboard's X11 path can't see native Wayland
//! apps' clipboard).
//!
//! `save`/`restore` bring back what the user had, like the Mac:
//! - Windows: every memory-backed clipboard format, byte for byte (text, rich
//!   text, HTML, images as DIB, copied files), via the Win32 clipboard API.
//! - Linux: the text as before, or an image when one is on the clipboard
//!   (arboard on X11, `wl-paste --type image/...` on Wayland).
//! - Elsewhere: plain text only.

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
    /// The X11/Wayland PRIMARY selection (what's highlighted). Terminals paste it
    /// on Shift+Insert. Platforms without one report nothing.
    fn get_primary(&self) -> Result<Option<String>> {
        Ok(None)
    }
    /// Set the PRIMARY selection, or clear it. A no-op where there is none.
    fn set_primary(&self, _text: Option<&str>) -> Result<()> {
        Ok(())
    }
    /// The copy now on the clipboard came from a code editor with nothing
    /// selected: VS Code and its forks (Cursor, Windsurf, VSCodium) copy the
    /// whole line then, which is not a selection.
    fn copied_from_empty_selection(&self) -> bool {
        false
    }
}

/// VS Code tags its copies with JSON metadata, `"isFromEmptySelection":true`
/// when the line was copied because nothing was selected. It can't be its own
/// clipboard type, so Chromium packs it into its custom-data format, as
/// UTF-16; the bytes are searched directly, in UTF-8 and UTF-16.
pub fn marks_empty_selection(bytes: &[u8]) -> bool {
    const MARKER: &str = "\"isFromEmptySelection\":true";
    if bytes.len() > 256_000 {
        return false;
    }
    let utf8 = MARKER.as_bytes();
    let utf16: Vec<u8> = MARKER.encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.windows(utf8.len()).any(|w| w == utf8) || bytes.windows(utf16.len()).any(|w| w == utf16)
}

/// The clipboard type Chromium (and so VS Code) keeps that metadata in.
#[cfg(target_os = "linux")]
const CHROMIUM_CUSTOM_DATA_MIME: &str = "chromium/x-web-custom-data";

/// In-memory clipboard for tests and `demo`.
#[derive(Default)]
pub struct MockClipboard {
    current: Mutex<Option<String>>,
    saved: Mutex<Option<String>>,
    /// Every value written by `set_text`/`restore`, for assertions.
    history: Mutex<Vec<Option<String>>>,
    primary: Mutex<Option<String>>,
    /// Every value written by `set_primary`, for assertions.
    primary_history: Mutex<Vec<Option<String>>>,
    /// The next copy looks like a code editor's whole-line copy.
    empty_selection_copy: std::sync::atomic::AtomicBool,
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

    pub fn primary_history(&self) -> Vec<Option<String>> {
        self.primary_history.lock().unwrap().clone()
    }

    /// Make copies look like a code editor's copy with nothing selected.
    pub fn set_empty_selection_copy(&self, on: bool) {
        self.empty_selection_copy
            .store(on, std::sync::atomic::Ordering::SeqCst);
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

    fn get_primary(&self) -> Result<Option<String>> {
        Ok(self.primary.lock().unwrap().clone())
    }

    fn set_primary(&self, text: Option<&str>) -> Result<()> {
        *self.primary.lock().unwrap() = text.map(str::to_string);
        self.primary_history
            .lock()
            .unwrap()
            .push(text.map(str::to_string));
        Ok(())
    }

    fn copied_from_empty_selection(&self) -> bool {
        self.empty_selection_copy
            .load(std::sync::atomic::Ordering::SeqCst)
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
    fn get_primary(&self) -> Result<Option<String>> {
        (**self).get_primary()
    }
    fn set_primary(&self, text: Option<&str>) -> Result<()> {
        (**self).set_primary(text)
    }
    fn copied_from_empty_selection(&self) -> bool {
        (**self).copied_from_empty_selection()
    }
}

/// Largest clipboard snapshot kept for restoring. Anything bigger falls back to
/// saving just the text, so a huge copy can't balloon memory or stall a paste.
#[cfg(any(target_os = "linux", all(windows, feature = "injection"), test))]
const MAX_SNAPSHOT_BYTES: usize = 64 << 20;

/// Whether `add` more bytes still fit in a snapshot already holding `total`.
#[cfg(any(target_os = "linux", all(windows, feature = "injection"), test))]
fn fits_snapshot(total: usize, add: usize) -> bool {
    total
        .checked_add(add)
        .is_some_and(|n| n <= MAX_SNAPSHOT_BYTES)
}

/// What `save()` captured, for `restore()` to put back.
enum Snapshot {
    /// Plain text, or nothing (restore then clears the clipboard).
    Text(Option<String>),
    /// Every memory-backed Windows clipboard format, in enumeration order.
    #[cfg(all(windows, feature = "injection"))]
    Formats(Vec<win::Format>),
    /// An image on the X11 clipboard, decoded by arboard.
    #[cfg(all(target_os = "linux", feature = "injection"))]
    Image(arboard::ImageData<'static>),
    /// One MIME type's raw bytes on the Wayland clipboard (an image).
    #[cfg(target_os = "linux")]
    Mime { mime: String, bytes: Vec<u8> },
}

/// The real OS clipboard.
pub struct SystemClipboard {
    saved: Mutex<Option<Snapshot>>,
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

    #[cfg(all(feature = "injection", target_os = "linux"))]
    fn board_primary_get(&self) -> Result<Option<String>> {
        use arboard::{GetExtLinux, LinuxClipboardKind};
        self.with_board(
            |b| match b.get().clipboard(LinuxClipboardKind::Primary).text() {
                Ok(t) => Ok(Some(t)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(CoreError::Injection(format!("read selection: {e}"))),
            },
        )
    }

    #[cfg(all(feature = "injection", target_os = "linux"))]
    fn board_primary_set(&self, text: Option<&str>) -> Result<()> {
        use arboard::{LinuxClipboardKind, SetExtLinux};
        self.with_board(|b| {
            b.set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text.unwrap_or_default().to_string())
                .map_err(|e| CoreError::Injection(format!("write selection: {e}")))
        })
    }

    /// The X11 clipboard's image, if it holds one small enough to keep.
    #[cfg(all(feature = "injection", target_os = "linux"))]
    fn board_image(&self) -> Option<arboard::ImageData<'static>> {
        self.with_board(|b| Ok(b.get_image().ok()))
            .ok()
            .flatten()
            .filter(|img| !img.bytes.is_empty() && fits_snapshot(0, img.bytes.len()))
    }

    /// Capture the clipboard. Never fails: whatever can't be read is saved as
    /// "nothing", so restore clears our staged text instead of leaving it.
    fn snapshot(&self) -> Snapshot {
        #[cfg(target_os = "linux")]
        if self.wayland_tools {
            // Only one kind can be put back on Linux, so text wins when there
            // is some (copied spreadsheet cells also offer a picture of
            // them); an image comes back when it's all there was, like a
            // screenshot copy.
            let text = wl::paste().unwrap_or(None);
            if text.as_deref().is_some_and(|t| !t.is_empty()) {
                return Snapshot::Text(text);
            }
            if let Some((mime, bytes)) = wl::paste_image() {
                return Snapshot::Mime { mime, bytes };
            }
            return Snapshot::Text(text);
        }
        #[cfg(all(windows, feature = "injection"))]
        match win::save_formats() {
            Ok(formats) => return Snapshot::Formats(formats),
            Err(e) => tracing::debug!("full clipboard snapshot unavailable, saving text: {e}"),
        }
        let text = self.read().unwrap_or(None);
        // As on Wayland: text wins; an image only when it's all there was.
        #[cfg(all(target_os = "linux", feature = "injection"))]
        if text.as_deref().is_none_or(str::is_empty) {
            if let Some(image) = self.board_image() {
                return Snapshot::Image(image);
            }
        }
        Snapshot::Text(text)
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
        let snapshot = self.snapshot();
        *self.saved.lock().unwrap() = Some(snapshot);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn get_primary(&self) -> Result<Option<String>> {
        if self.wayland_tools {
            return wl::paste_primary();
        }
        #[cfg(feature = "injection")]
        return self.board_primary_get();
        #[cfg(not(feature = "injection"))]
        Ok(None)
    }

    #[cfg(target_os = "linux")]
    fn set_primary(&self, text: Option<&str>) -> Result<()> {
        if self.wayland_tools {
            return wl::copy_primary(text);
        }
        #[cfg(feature = "injection")]
        return self.board_primary_set(text);
        #[cfg(not(feature = "injection"))]
        Ok(())
    }

    fn set_text(&self, text: &str) -> Result<()> {
        self.write(Some(text))
    }

    fn get_text(&self) -> Result<Option<String>> {
        self.read()
    }

    fn copied_from_empty_selection(&self) -> bool {
        #[cfg(all(windows, feature = "injection"))]
        return win::any_registered_format(marks_empty_selection);
        #[cfg(target_os = "linux")]
        {
            let bytes = if self.wayland_tools {
                wl::paste_type(CHROMIUM_CUSTOM_DATA_MIME)
            } else {
                xclip_type(CHROMIUM_CUSTOM_DATA_MIME)
            };
            return bytes.is_some_and(|b| marks_empty_selection(&b));
        }
        #[allow(unreachable_code)]
        false
    }

    fn restore(&self) -> Result<()> {
        let saved = self.saved.lock().unwrap().take();
        match saved {
            None => Ok(()),
            Some(Snapshot::Text(previous)) => self.write(previous.as_deref()),
            #[cfg(all(windows, feature = "injection"))]
            Some(Snapshot::Formats(formats)) => win::restore_formats(&formats),
            #[cfg(all(target_os = "linux", feature = "injection"))]
            Some(Snapshot::Image(image)) => self.with_board(|b| {
                b.set_image(image)
                    .map_err(|e| CoreError::Injection(format!("restore clipboard image: {e}")))
            }),
            #[cfg(target_os = "linux")]
            Some(Snapshot::Mime { mime, bytes }) => wl::copy_bytes(&bytes, &mime),
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

/// One type's bytes off the X11 clipboard through `xclip`, when installed
/// (arboard reads text and images only). Capped at half a second: the copying
/// app answers the request itself and may be slow.
#[cfg(target_os = "linux")]
fn xclip_type(mime: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    if !on_path("xclip") {
        return None;
    }
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", mime, "-o"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_millis(500);
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let buf = reader.join().ok()?;
    ok.then_some(buf)
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
        read(&["--no-newline", "--type", "text"])
    }

    pub fn paste_primary() -> Result<Option<String>> {
        read(&["--primary", "--no-newline", "--type", "text"])
    }

    /// The clipboard's image as (MIME type, bytes), when it offers one that
    /// fits a snapshot. Text-only or empty clipboards give `None`.
    /// One MIME type's bytes, when the clipboard offers it.
    pub fn paste_type(mime: &str) -> Option<Vec<u8>> {
        read_bytes(&["--type", mime]).ok().flatten()
    }

    pub fn paste_image() -> Option<(String, Vec<u8>)> {
        let types = read_bytes(&["--list-types"]).ok().flatten()?;
        let types = String::from_utf8_lossy(&types);
        let mime = super::pick_image_mime(types.lines())?.to_string();
        let bytes = read_bytes(&["--type", &mime]).ok().flatten()?;
        (!bytes.is_empty() && super::fits_snapshot(0, bytes.len())).then_some((mime, bytes))
    }

    fn read(args: &[&str]) -> Result<Option<String>> {
        Ok(read_bytes(args)?.map(|buf| String::from_utf8_lossy(&buf).into_owned()))
    }

    /// Run `wl-paste` and collect its stdout; `None` when nothing matched.
    fn read_bytes(args: &[&str]) -> Result<Option<Vec<u8>>> {
        let mut child = Command::new("wl-paste")
            .args(args)
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
            Some(s) if s.success() => Ok(Some(buf)),
            Some(_) => Ok(None),
            None => Err(CoreError::Injection("clipboard read timed out".into())),
        }
    }

    pub fn copy(text: Option<&str>) -> Result<()> {
        write(text.map(str::as_bytes), None, false)
    }

    pub fn copy_primary(text: Option<&str>) -> Result<()> {
        write(text.map(str::as_bytes), None, true)
    }

    /// Offer `bytes` as `mime` on the clipboard (restoring a saved image).
    pub fn copy_bytes(bytes: &[u8], mime: &str) -> Result<()> {
        write(Some(bytes), Some(mime), false)
    }

    fn write(data: Option<&[u8]>, mime: Option<&str>, primary: bool) -> Result<()> {
        let mut cmd = Command::new("wl-copy");
        if primary {
            cmd.arg("--primary");
        }
        if let Some(mime) = mime {
            cmd.args(["--type", mime]);
        }
        if data.is_none() {
            cmd.arg("--clear");
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| CoreError::Injection(format!("wl-copy: {e}")))?;
        if let (Some(mut stdin), Some(d)) = (child.stdin.take(), data) {
            stdin
                .write_all(d)
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

/// The image type to save from a Wayland clipboard offering `types` (one MIME
/// type per item, as `wl-paste --list-types` prints them): PNG first, else the
/// first other `image/*`. `None` keeps the text-only behaviour.
#[cfg(any(target_os = "linux", test))]
fn pick_image_mime<'a>(types: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut other = None;
    for t in types.into_iter().map(str::trim) {
        if t.eq_ignore_ascii_case("image/png") {
            return Some(t);
        }
        let is_image = t.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("image/")) && t.len() > 6;
        if other.is_none() && is_image {
            other = Some(t);
        }
    }
    other
}

/// Which Windows clipboard formats can be copied as plain bytes. Standard
/// format ids (winuser.h) are spelled out so the filter is testable anywhere.
#[cfg(any(all(windows, feature = "injection"), test))]
mod win_formats {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_OWNERDISPLAY: u32 = 0x0080;
    const CF_DSPBITMAP: u32 = 0x0082;
    const CF_DSPMETAFILEPICT: u32 = 0x0083;
    const CF_DSPENHMETAFILE: u32 = 0x008E;
    const CF_PRIVATEFIRST: u32 = 0x0200;
    const CF_PRIVATELAST: u32 = 0x02FF;
    const CF_GDIOBJFIRST: u32 = 0x0300;
    const CF_GDIOBJLAST: u32 = 0x03FF;
    /// Registered (named) formats start here.
    const REGISTERED_FIRST: u32 = 0xC000;

    /// Whether `id` is a registered format, which has a name to look up.
    pub fn is_registered(id: u32) -> bool {
        id >= REGISTERED_FIRST
    }

    /// Whether `id`'s clipboard handle is an HGLOBAL whose bytes are the data.
    /// GDI handles (bitmaps, palettes, metafiles) and owner-drawn/private formats
    /// aren't; CF_DIB/CF_DIBV5 still carry a copied image, and Windows
    /// re-synthesises CF_BITMAP from them on restore.
    pub fn is_memory_format(id: u32) -> bool {
        id != 0
            && !matches!(
                id,
                CF_BITMAP
                    | CF_METAFILEPICT
                    | CF_PALETTE
                    | CF_ENHMETAFILE
                    | CF_OWNERDISPLAY
                    | CF_DSPBITMAP
                    | CF_DSPMETAFILEPICT
                    | CF_DSPENHMETAFILE
            )
            && !(CF_PRIVATEFIRST..=CF_PRIVATELAST).contains(&id)
            && !(CF_GDIOBJFIRST..=CF_GDIOBJLAST).contains(&id)
    }

    /// Registered formats that only mean something while the source app's OLE
    /// data object owns the clipboard (they hold its pointers); restoring them
    /// from a copy would hand OLE a stale object.
    pub fn is_skipped_name(name: &str) -> bool {
        matches!(name, "DataObject" | "Ole Private Data")
    }
}

/// Win32 clipboard snapshot: every memory-backed format, copied byte for byte.
#[cfg(all(windows, feature = "injection"))]
mod win {
    use super::{fits_snapshot, win_formats};
    use crate::error::{CoreError, Result};
    use std::time::Duration;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
        GetClipboardFormatNameW, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };

    /// Another app may hold the clipboard open for a moment (clipboard
    /// managers do right after every change): retry for up to ~200 ms.
    const OPEN_ATTEMPTS: u32 = 20;
    const OPEN_RETRY: Duration = Duration::from_millis(10);

    /// One saved clipboard format.
    pub struct Format {
        id: u32,
        bytes: Vec<u8>,
    }

    /// The clipboard, open for this thread until dropped.
    struct OpenClipboardGuard;

    impl OpenClipboardGuard {
        fn open() -> Result<Self> {
            for attempt in 0..OPEN_ATTEMPTS {
                // SAFETY: no pointers involved. A null owner is allowed; arboard
                // (clipboard-win) opens the clipboard the same way.
                if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                    return Ok(Self);
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    std::thread::sleep(OPEN_RETRY);
                }
            }
            Err(CoreError::Injection(
                "clipboard is held open by another app".into(),
            ))
        }
    }

    impl Drop for OpenClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: we opened it in `open`.
            unsafe { CloseClipboard() };
        }
    }

    /// The name of a registered format, e.g. "HTML Format".
    fn format_name(id: u32) -> Option<String> {
        if !win_formats::is_registered(id) {
            return None;
        }
        let mut buf = [0u16; 256];
        // SAFETY: `buf` is valid for `buf.len()` u16s; the call writes at most that.
        let n = unsafe { GetClipboardFormatNameW(id, buf.as_mut_ptr(), buf.len() as i32) };
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }

    /// Whether any registered (app-defined) format on the clipboard satisfies
    /// `test`, e.g. VS Code's metadata inside "Chromium Web Custom MIME Data
    /// Format". False when the clipboard can't be opened.
    pub fn any_registered_format(test: impl Fn(&[u8]) -> bool) -> bool {
        let Ok(_open) = OpenClipboardGuard::open() else {
            return false;
        };
        let mut id = 0u32;
        loop {
            // SAFETY: the clipboard is open on this thread. 0 = no more formats.
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                return false;
            }
            if !win_formats::is_registered(id) || !win_formats::is_memory_format(id) {
                continue;
            }
            // SAFETY: as in `save_formats`: the handle is only locked, read
            // and unlocked while the clipboard is open.
            let hit = unsafe {
                let handle = GetClipboardData(id);
                if handle.is_null() {
                    continue;
                }
                let size = GlobalSize(handle);
                let ptr = GlobalLock(handle);
                if size == 0 || ptr.is_null() {
                    continue;
                }
                let hit = test(std::slice::from_raw_parts(ptr.cast::<u8>(), size));
                GlobalUnlock(handle);
                hit
            };
            if hit {
                return true;
            }
        }
    }

    /// Copy every memory-backed format off the clipboard. Fails (so the caller
    /// saves text instead) when the clipboard can't be opened or the formats
    /// add up to more than the snapshot cap.
    pub fn save_formats() -> Result<Vec<Format>> {
        let _open = OpenClipboardGuard::open()?;
        let mut saved = Vec::new();
        let mut total = 0usize;
        let mut id = 0u32;
        loop {
            // SAFETY: the clipboard is open on this thread. 0 = no more formats.
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            if !win_formats::is_memory_format(id)
                || format_name(id).is_some_and(|n| win_formats::is_skipped_name(&n))
            {
                continue;
            }
            // SAFETY: the clipboard is open; the handle stays owned by the
            // clipboard and is only read (locked, copied, unlocked) here.
            // Delay-rendered formats are rendered by their owner on this call.
            unsafe {
                let handle = GetClipboardData(id);
                if handle.is_null() {
                    continue;
                }
                let size = GlobalSize(handle);
                if size == 0 {
                    continue;
                }
                if !fits_snapshot(total, size) {
                    return Err(CoreError::Injection(
                        "clipboard too large to snapshot".into(),
                    ));
                }
                let ptr = GlobalLock(handle);
                if ptr.is_null() {
                    continue;
                }
                let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec();
                GlobalUnlock(handle);
                total += size;
                saved.push(Format { id, bytes });
            }
        }
        Ok(saved)
    }

    /// Replace the clipboard with the saved formats (an empty snapshot clears
    /// it). A format that fails to set is skipped; the rest still go back.
    pub fn restore_formats(formats: &[Format]) -> Result<()> {
        let _open = OpenClipboardGuard::open()?;
        // SAFETY: the clipboard is open on this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(CoreError::Injection("couldn't empty the clipboard".into()));
        }
        for f in formats {
            // SAFETY: `mem` is a fresh movable block of `f.bytes.len()` bytes;
            // the copy stays inside it. After a successful SetClipboardData the
            // system owns `mem`; otherwise it is still ours and is freed.
            unsafe {
                let mem = GlobalAlloc(GMEM_MOVEABLE, f.bytes.len());
                if mem.is_null() {
                    continue;
                }
                let ptr = GlobalLock(mem);
                if ptr.is_null() {
                    GlobalFree(mem);
                    continue;
                }
                std::ptr::copy_nonoverlapping(f.bytes.as_ptr(), ptr.cast::<u8>(), f.bytes.len());
                GlobalUnlock(mem);
                if SetClipboardData(f.id, mem).is_null() {
                    GlobalFree(mem);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vs_code_marker_found_in_either_encoding() {
        let json = r#"{"version":1,"isFromEmptySelection":true,"mode":"rust"}"#;
        assert!(marks_empty_selection(json.as_bytes()));
        // Chromium custom data: a binary header, then UTF-16 strings.
        let mut custom = vec![0x50, 0, 0, 0, 1, 0, 0, 0];
        custom.extend(json.encode_utf16().flat_map(u16::to_le_bytes));
        assert!(marks_empty_selection(&custom));
        let selected = r#"{"version":1,"isFromEmptySelection":false}"#;
        assert!(!marks_empty_selection(selected.as_bytes()));
        assert!(!marks_empty_selection(b"plain text"));
    }

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

    #[test]
    fn snapshot_cap_allows_up_to_the_limit() {
        assert!(fits_snapshot(0, 0));
        assert!(fits_snapshot(0, MAX_SNAPSHOT_BYTES));
        assert!(fits_snapshot(MAX_SNAPSHOT_BYTES - 10, 10));
        assert!(!fits_snapshot(MAX_SNAPSHOT_BYTES - 10, 11));
        assert!(!fits_snapshot(0, MAX_SNAPSHOT_BYTES + 1));
        // No overflow panic on absurd sizes.
        assert!(!fits_snapshot(usize::MAX, 1));
    }

    #[test]
    fn image_mime_prefers_png() {
        let types = ["text/plain", "image/jpeg", "image/png", "text/html"];
        assert_eq!(pick_image_mime(types), Some("image/png"));
    }

    #[test]
    fn image_mime_falls_back_to_other_images() {
        let listed = "text/html\nimage/bmp\nimage/jpeg\n";
        assert_eq!(pick_image_mime(listed.lines()), Some("image/bmp"));
        assert_eq!(pick_image_mime([" IMAGE/PNG "]), Some("IMAGE/PNG"));
    }

    #[test]
    fn text_only_clipboard_picks_no_image() {
        let types = [
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "TEXT",
            "text/uri-list",
            "image/",
            "imagery/png",
        ];
        assert_eq!(pick_image_mime(types), None);
        assert_eq!(pick_image_mime([]), None);
    }

    #[test]
    fn windows_memory_formats_are_kept() {
        use win_formats::is_memory_format;
        // CF_TEXT, CF_UNICODETEXT, CF_LOCALE, CF_DIB, CF_DIBV5, CF_HDROP, CF_DSPTEXT.
        for id in [1, 13, 16, 8, 17, 15, 0x81] {
            assert!(is_memory_format(id), "format {id} should be saved");
        }
        // Registered formats ("HTML Format", "Rich Text Format", "PNG", ...).
        assert!(is_memory_format(0xC000));
        assert!(is_memory_format(0xC1F3));
        assert!(is_memory_format(0xFFFF));
    }

    #[test]
    fn windows_handle_formats_are_skipped() {
        use win_formats::is_memory_format;
        // Invalid, CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE,
        // CF_OWNERDISPLAY, CF_DSPBITMAP, CF_DSPMETAFILEPICT, CF_DSPENHMETAFILE.
        for id in [0, 2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E] {
            assert!(!is_memory_format(id), "format {id} should be skipped");
        }
        // Private and GDI-object ranges.
        for id in [0x200, 0x2AB, 0x2FF, 0x300, 0x3FF] {
            assert!(!is_memory_format(id), "format {id:#x} should be skipped");
        }
    }

    #[test]
    fn windows_registered_formats_start_at_c000() {
        assert!(!win_formats::is_registered(13));
        assert!(!win_formats::is_registered(0xBFFF));
        assert!(win_formats::is_registered(0xC000));
    }

    #[test]
    fn windows_ole_object_formats_are_skipped() {
        assert!(win_formats::is_skipped_name("DataObject"));
        assert!(win_formats::is_skipped_name("Ole Private Data"));
        assert!(!win_formats::is_skipped_name("HTML Format"));
        assert!(!win_formats::is_skipped_name("PNG"));
    }
}
