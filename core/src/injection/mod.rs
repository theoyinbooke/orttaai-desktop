//! Text injection at the cursor (Mac `TextInjectionService`,
//! `SelectionCaptureService`, `KeyEventPoster`).
//!
//! Two layers:
//! - [`TextInjector`] — the OS-specific key sender: type text, send the paste
//!   and copy chords, and report whether the focused field is a password field.
//!   Backends: `enigo` (Windows `SendInput`, X11 XTest), `wtype` (wlroots
//!   Wayland), the XDG RemoteDesktop portal (GNOME/KDE Wayland).
//! - [`InjectionService`] — the portable delivery policy on top: `auto` /
//!   `type` / `paste` modes, clipboard save → paste → restore when
//!   `preserve_clipboard`, the honest fallback that leaves the text on the
//!   clipboard when everything fails, the secure-field guard, and selection
//!   capture for voice edit (copy chord into a saved-and-restored clipboard).
//!
//! Secure-field detection has no reliable Linux equivalent — the trait returns
//! [`SecureFieldStatus::Unknown`] there (see `docs/gaps.md`).

use crate::clipboard::Clipboard;
use crate::error::{CoreError, Result};
use crate::settings::InjectionMode;
use crate::types::{InjectionMethod, InjectionResult, SecureFieldStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod edit_watch;

/// Which key chord asks the focused app to paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteChord {
    /// Ctrl+V — every GUI app.
    CtrlV,
    /// Shift+Insert — terminals (where Ctrl+V inserts a literal ^V) and most
    /// GTK/Qt/Win32 edit controls.
    ShiftInsert,
}

pub trait TextInjector: Send + Sync {
    /// Make `text` appear at the cursor via synthesized keystrokes.
    fn type_text(&self, text: &str) -> Result<()>;

    /// Send the paste chord to the focused app.
    fn paste_chord(&self, chord: PasteChord) -> Result<()>;

    /// Send the copy chord (Ctrl+C) to the focused app.
    fn copy_chord(&self) -> Result<()>;

    /// Best-effort secure/password-field check for the focused element.
    fn is_secure_field_focused(&self) -> SecureFieldStatus;

    /// Human-readable backend name, for diagnostics.
    fn backend_name(&self) -> &'static str;

    /// How typed text is recorded in history.
    fn typing_method(&self) -> InjectionMethod {
        InjectionMethod::Typed
    }

    /// Type `text` (legacy one-shot API used by the CLI).
    fn inject(&self, text: &str) -> Result<InjectionResult> {
        if text.is_empty() {
            return Ok(InjectionResult::NoTranscript);
        }
        self.type_text(text)?;
        Ok(InjectionResult::Success)
    }
}

impl<T: TextInjector + ?Sized> TextInjector for Box<T> {
    fn type_text(&self, text: &str) -> Result<()> {
        (**self).type_text(text)
    }
    fn paste_chord(&self, chord: PasteChord) -> Result<()> {
        (**self).paste_chord(chord)
    }
    fn copy_chord(&self) -> Result<()> {
        (**self).copy_chord()
    }
    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        (**self).is_secure_field_focused()
    }
    fn backend_name(&self) -> &'static str {
        (**self).backend_name()
    }
    fn typing_method(&self) -> InjectionMethod {
        (**self).typing_method()
    }
}

// ---- Delivery policy --------------------------------------------------------

/// In `auto` mode, text this long (or multi-line) is pasted instead of typed:
/// pasting is near-instant and keeps line breaks, typing is per-character.
pub const AUTO_PASTE_MIN_CHARS: usize = 40;
/// How long the target app gets to read our clipboard before it's restored.
/// Longer than the Mac's verified 90 ms: without a post-paste verification
/// read, X11/Wayland/Win32 apps that fetch the selection asynchronously need
/// the margin, or they'd paste the restored (old) clipboard.
pub const RESTORE_DELAY_MS: u64 = 250;
/// Low Latency Mode's restore delay.
pub const LOW_LATENCY_RESTORE_DELAY_MS: u64 = 150;
/// Settle time between staging the clipboard and sending the chord.
const CLIPBOARD_SETTLE_MS: u64 = 20;
/// Pause before the copy chord so the user's fingers are off the edit
/// shortcut's modifiers (a held Shift would turn Ctrl+C into Ctrl+Shift+C).
pub const SELECTION_SETTLE_MS: u64 = 120;
/// Bounded wait for a copy to land (Mac `copyPollAttempts` × `copyPollInterval`).
pub const COPY_POLL_ATTEMPTS: u32 = 6;
pub const COPY_POLL_INTERVAL_MS: u64 = 50;

/// How text reaches the focused app.
#[derive(Debug, Clone)]
pub struct InjectionConfig {
    pub mode: InjectionMode,
    pub preserve_clipboard: bool,
    pub low_latency: bool,
    /// Treat an unverifiable field (`Unknown`) as secure.
    pub strict_secure: bool,
}

impl Default for InjectionConfig {
    fn default() -> Self {
        Self {
            mode: InjectionMode::Auto,
            preserve_clipboard: true,
            low_latency: false,
            strict_secure: false,
        }
    }
}

/// What one delivery did.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub result: InjectionResult,
    pub method: InjectionMethod,
    /// Why delivery failed, when it did.
    pub error: Option<String>,
    pub ms: i64,
}

/// Result of reading the focused app's selection (Mac `SelectionCaptureResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionCapture {
    Captured(String),
    NoSelection,
    /// The focused element is a password field. Its contents are never read.
    BlockedSecureField,
}

/// Terminal-like apps get Shift+Insert, where Ctrl+V means "literal next".
pub fn paste_chord_for(app: Option<&str>) -> PasteChord {
    const TERMINALS: &[&str] = &[
        "term",
        "konsole",
        "alacritty",
        "kitty",
        "wezterm",
        "foot",
        "tilix",
        "terminator",
        "guake",
        "yakuake",
        "tilda",
        "ghostty",
        "cmd",
        "powershell",
        "pwsh",
        "conhost",
        "windowsterminal",
        "mintty",
        "putty",
        "warp",
    ];
    match app {
        Some(a) => {
            let a = a.to_ascii_lowercase();
            if TERMINALS.iter().any(|t| a.contains(t)) {
                PasteChord::ShiftInsert
            } else {
                PasteChord::CtrlV
            }
        }
        None => PasteChord::CtrlV,
    }
}

/// Whether `auto` mode pastes `text`.
pub fn auto_pastes(text: &str) -> bool {
    text.contains('\n') || text.chars().count() >= AUTO_PASTE_MIN_CHARS
}

/// Portable delivery policy over a key sender and a clipboard.
pub struct InjectionService {
    injector: Box<dyn TextInjector>,
    clipboard: Box<dyn Clipboard>,
    config: InjectionConfig,
    /// Test seam: real sleeps by default.
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
}

impl InjectionService {
    pub fn new(injector: Box<dyn TextInjector>, clipboard: Box<dyn Clipboard>) -> Self {
        Self {
            injector,
            clipboard,
            config: InjectionConfig::default(),
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Replace the sleep function (tests skip real waits).
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    pub fn config(&self) -> &InjectionConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: InjectionConfig) {
        self.config = config;
    }

    pub fn backend_name(&self) -> &'static str {
        self.injector.backend_name()
    }

    pub fn secure_status(&self) -> SecureFieldStatus {
        self.injector.is_secure_field_focused()
    }

    /// Whether the secure-field policy refuses the focused field.
    pub fn secure_blocked(&self) -> bool {
        match self.secure_status() {
            SecureFieldStatus::Secure => true,
            SecureFieldStatus::Unknown => self.config.strict_secure,
            SecureFieldStatus::NotSecure => false,
        }
    }

    /// Put `text` on the clipboard and leave it there (manual-paste recovery).
    pub fn leave_on_clipboard(&self, text: &str) -> bool {
        self.clipboard.set_text(text).is_ok()
    }

    /// Deliver `text` to the focused app (whose name, when known, picks the
    /// paste chord). Checks the secure-field policy first. On total failure the
    /// text is left on the clipboard so it can be pasted by hand.
    pub fn deliver(&self, text: &str, app: Option<&str>) -> Delivery {
        let started = Instant::now();
        let done = |result, method, error: Option<String>| Delivery {
            result,
            method,
            error,
            ms: started.elapsed().as_millis() as i64,
        };
        if text.is_empty() {
            return done(InjectionResult::NoTranscript, InjectionMethod::Failed, None);
        }
        if self.secure_blocked() {
            return done(
                InjectionResult::BlockedSecureField,
                InjectionMethod::Blocked,
                None,
            );
        }
        let paste = match self.config.mode {
            InjectionMode::Paste => true,
            InjectionMode::Type => false,
            InjectionMode::Auto => auto_pastes(text),
        };
        let error = if paste {
            match self.paste(text, paste_chord_for(app)) {
                Ok(()) => return done(InjectionResult::Success, InjectionMethod::Paste, None),
                Err(paste_err) => {
                    tracing::warn!("paste failed ({paste_err}); typing instead");
                    match self.injector.type_text(text) {
                        Ok(()) => {
                            return done(
                                InjectionResult::Success,
                                self.injector.typing_method(),
                                None,
                            )
                        }
                        Err(e) => e.to_string(),
                    }
                }
            }
        } else {
            match self.injector.type_text(text) {
                Ok(()) => {
                    return done(
                        InjectionResult::Success,
                        self.injector.typing_method(),
                        None,
                    )
                }
                Err(e) => e.to_string(),
            }
        };
        let method = if self.leave_on_clipboard(text) {
            InjectionMethod::Clipboard
        } else {
            InjectionMethod::Failed
        };
        done(InjectionResult::Failed, method, Some(error))
    }

    fn paste(&self, text: &str, chord: PasteChord) -> Result<()> {
        let preserve = self.config.preserve_clipboard;
        if preserve {
            if let Err(e) = self.clipboard.save() {
                tracing::warn!("couldn't save the clipboard before pasting: {e}");
            }
        }
        self.clipboard.set_text(text)?;
        // Shift+Insert pastes the PRIMARY selection, not the clipboard, in xterm
        // and others: without this they'd paste whatever was last highlighted.
        let primary = (chord == PasteChord::ShiftInsert).then(|| {
            let held = self.clipboard.get_primary().unwrap_or(None);
            if let Err(e) = self.clipboard.set_primary(Some(text)) {
                tracing::warn!("couldn't stage the selection: {e}");
            }
            held
        });
        (self.sleep)(Duration::from_millis(CLIPBOARD_SETTLE_MS));
        if let Err(e) = self.injector.paste_chord(chord) {
            if preserve {
                let _ = self.clipboard.restore();
            }
            if let Some(held) = &primary {
                let _ = self.clipboard.set_primary(held.as_deref());
            }
            return Err(e);
        }
        if preserve || primary.is_some() {
            let delay = if self.config.low_latency {
                LOW_LATENCY_RESTORE_DELAY_MS
            } else {
                RESTORE_DELAY_MS
            };
            (self.sleep)(Duration::from_millis(delay));
        }
        if preserve {
            if let Err(e) = self.clipboard.restore() {
                tracing::warn!("couldn't restore the clipboard: {e}");
            }
        }
        if let Some(held) = &primary {
            if let Err(e) = self.clipboard.set_primary(held.as_deref()) {
                tracing::warn!("couldn't restore the selection: {e}");
            }
        }
        Ok(())
    }

    /// Read the focused app's selection for a voice edit: secure-field guard
    /// first (a password's contents must never reach a model), then the copy
    /// chord into a saved-and-restored clipboard. The clipboard is staged with a
    /// unique sentinel so a copy that lands is detectable even when the
    /// selection equals what was already on the clipboard.
    pub fn capture_selection(&self) -> SelectionCapture {
        if self.secure_blocked() {
            return SelectionCapture::BlockedSecureField;
        }
        (self.sleep)(Duration::from_millis(SELECTION_SETTLE_MS));
        let _ = self.clipboard.save();
        let sentinel = format!(
            "\u{2063}orttaai-selection-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        if self.clipboard.set_text(&sentinel).is_err() {
            return SelectionCapture::NoSelection;
        }
        let mut copied: Option<String> = None;
        if self.injector.copy_chord().is_ok() {
            for _ in 0..COPY_POLL_ATTEMPTS {
                (self.sleep)(Duration::from_millis(COPY_POLL_INTERVAL_MS));
                match self.clipboard.get_text() {
                    Ok(Some(t)) if t != sentinel => {
                        copied = Some(t);
                        break;
                    }
                    _ => {}
                }
            }
        }
        let _ = self.clipboard.restore();
        match copied {
            Some(t) if !t.trim().is_empty() => SelectionCapture::Captured(t),
            _ => SelectionCapture::NoSelection,
        }
    }
}

// ---- Mock -------------------------------------------------------------------

/// Shared, cloneable record of what a [`MockTextInjector`] delivered — lets a
/// test inspect injected text after handing ownership to the coordinator.
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

/// Records delivered text instead of touching the OS. `paste_chord` "pastes"
/// whatever is on the attached clipboard; `copy_chord` copies the scripted
/// selection onto it. Used by tests and `demo`.
pub struct MockTextInjector {
    log: InjectionLog,
    chords: Arc<Mutex<Vec<String>>>,
    secure: SecureFieldStatus,
    fail: bool,
    fail_paste: bool,
    clipboard: Option<Arc<dyn Clipboard>>,
    selection: Arc<Mutex<Option<String>>>,
}

impl MockTextInjector {
    pub fn new() -> Self {
        Self {
            log: InjectionLog::default(),
            chords: Arc::default(),
            secure: SecureFieldStatus::NotSecure,
            fail: false,
            fail_paste: false,
            clipboard: None,
            selection: Arc::default(),
        }
    }

    /// Simulate a focused secure field (so injection is blocked).
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

    /// Simulate key injection that fails entirely (e.g. no virtual keyboard).
    pub fn failing() -> Self {
        Self {
            fail: true,
            fail_paste: true,
            ..Self::new()
        }
    }

    /// Only the paste chord fails.
    pub fn failing_paste(mut self) -> Self {
        self.fail_paste = true;
        self
    }

    /// Attach the clipboard that paste reads from and copy writes to.
    pub fn with_clipboard(mut self, clipboard: Arc<dyn Clipboard>) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    /// The text currently "selected" in the focused app.
    pub fn with_selection(self, text: &str) -> Self {
        *self.selection.lock().unwrap() = Some(text.to_string());
        self
    }

    /// A handle to inspect what was delivered.
    pub fn log(&self) -> InjectionLog {
        self.log.clone()
    }

    /// Chords sent, e.g. `["copy", "paste:ctrl+v"]`.
    pub fn chords(&self) -> Arc<Mutex<Vec<String>>> {
        self.chords.clone()
    }
}

impl Default for MockTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for MockTextInjector {
    fn type_text(&self, text: &str) -> Result<()> {
        if self.fail {
            return Err(CoreError::Injection("mock injection failure".to_string()));
        }
        self.log.0.lock().unwrap().push(text.to_string());
        Ok(())
    }

    fn paste_chord(&self, chord: PasteChord) -> Result<()> {
        self.chords.lock().unwrap().push(match chord {
            PasteChord::CtrlV => "paste:ctrl+v".into(),
            PasteChord::ShiftInsert => "paste:shift+insert".into(),
        });
        if self.fail_paste {
            return Err(CoreError::Injection("mock paste failure".to_string()));
        }
        let pasted = self
            .clipboard
            .as_ref()
            .and_then(|c| c.get_text().ok().flatten())
            .ok_or_else(|| CoreError::Injection("nothing to paste".into()))?;
        self.log.0.lock().unwrap().push(pasted);
        Ok(())
    }

    fn copy_chord(&self) -> Result<()> {
        self.chords.lock().unwrap().push("copy".into());
        if self.fail {
            return Err(CoreError::Injection("mock copy failure".to_string()));
        }
        if let (Some(c), Some(sel)) = (&self.clipboard, self.selection.lock().unwrap().clone()) {
            c.set_text(&sel)?;
        }
        Ok(())
    }

    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        self.secure
    }

    fn backend_name(&self) -> &'static str {
        "mock"
    }
}

// ---- Real backend -----------------------------------------------------------

/// The real, OS-selected key sender.
///
/// With the `injection` feature it types via `enigo` (Windows `SendInput`,
/// macOS `CGEvent`, Linux X11) and uses `wtype` on Linux/Wayland, or the
/// RemoteDesktop portal when it was granted (`portal` feature). Without the
/// feature it returns [`CoreError::NotImplemented`] so behaviour stays honest.
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

    /// Whether typing goes through the RemoteDesktop portal.
    pub fn uses_portal(&self) -> bool {
        #[cfg(all(feature = "portal", target_os = "linux"))]
        {
            self.portal.is_some()
        }
        #[cfg(not(all(feature = "portal", target_os = "linux")))]
        {
            false
        }
    }
}

/// On Wayland, try to open a RemoteDesktop portal session (shows the permission
/// dialog the first time). `None` if unavailable or denied → `wtype`/clipboard.
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
    fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.send(Keys::Text(text))
    }

    fn paste_chord(&self, chord: PasteChord) -> Result<()> {
        self.send(Keys::Paste(chord))
    }

    fn copy_chord(&self) -> Result<()> {
        self.send(Keys::Copy)
    }

    fn is_secure_field_focused(&self) -> SecureFieldStatus {
        detect_secure_field()
    }

    fn typing_method(&self) -> InjectionMethod {
        if self.uses_portal() {
            InjectionMethod::Portal
        } else {
            InjectionMethod::Typed
        }
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
        return if self.uses_portal() {
            "RemoteDesktop portal (Wayland)"
        } else if self.wayland {
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

/// One key-sending request.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(feature = "injection"), allow(dead_code))]
enum Keys<'a> {
    Text(&'a str),
    Paste(PasteChord),
    Copy,
}

#[cfg(feature = "injection")]
impl SystemTextInjector {
    fn send(&self, keys: Keys<'_>) -> Result<()> {
        #[cfg(all(feature = "portal", target_os = "linux"))]
        if let Some(portal) = &self.portal {
            return match keys {
                Keys::Text(t) => portal.type_text(t),
                Keys::Paste(PasteChord::CtrlV) => portal.chord(&[portal::CONTROL_L], portal::KEY_V),
                Keys::Paste(PasteChord::ShiftInsert) => {
                    portal.chord(&[portal::SHIFT_L], portal::INSERT)
                }
                Keys::Copy => portal.chord(&[portal::CONTROL_L], portal::KEY_C),
            };
        }
        #[cfg(target_os = "linux")]
        if self.wayland {
            return wtype(keys);
        }
        enigo_send(keys)
    }
}

#[cfg(not(feature = "injection"))]
impl SystemTextInjector {
    fn send(&self, _keys: Keys<'_>) -> Result<()> {
        Err(CoreError::NotImplemented(
            "system text injection — rebuild with --features injection",
        ))
    }
}

#[cfg(feature = "injection")]
fn enigo_send(keys: Keys<'_>) -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| CoreError::Injection(format!("enigo init: {e}")))?;
    let err = |e: enigo::InputError| CoreError::Injection(format!("send keys: {e}"));
    match keys {
        Keys::Text(t) => enigo.text(t).map_err(err),
        Keys::Paste(_) | Keys::Copy => {
            // The user may still be holding the hotkey's modifiers; release the
            // ones that would turn Ctrl+C into Ctrl+Shift+C (devtools in a
            // browser) before sending the chord.
            for m in [Key::Shift, Key::Alt, Key::Meta] {
                let _ = enigo.key(m, Direction::Release);
            }
            let (modifier, key) = match keys {
                Keys::Paste(PasteChord::ShiftInsert) => (Key::Shift, insert_key()),
                Keys::Paste(PasteChord::CtrlV) => (Key::Control, letter_key('v')),
                _ => (Key::Control, letter_key('c')),
            };
            enigo.key(modifier, Direction::Press).map_err(err)?;
            let clicked = enigo.key(key, Direction::Click).map_err(err);
            enigo.key(modifier, Direction::Release).map_err(err)?;
            clicked
        }
    }
}

#[cfg(all(feature = "injection", target_os = "windows"))]
fn letter_key(c: char) -> enigo::Key {
    // Windows shortcuts need virtual-key codes, not Unicode input.
    match c {
        'v' => enigo::Key::V,
        _ => enigo::Key::C,
    }
}

#[cfg(all(feature = "injection", not(target_os = "windows")))]
fn letter_key(c: char) -> enigo::Key {
    enigo::Key::Unicode(c)
}

#[cfg(all(feature = "injection", any(target_os = "windows", target_os = "linux")))]
fn insert_key() -> enigo::Key {
    enigo::Key::Insert
}

#[cfg(all(
    feature = "injection",
    not(any(target_os = "windows", target_os = "linux"))
))]
fn insert_key() -> enigo::Key {
    // macOS has no Insert key; Shift+Insert is never chosen there in practice.
    enigo::Key::Unicode('v')
}

#[cfg(all(feature = "injection", target_os = "linux"))]
fn wtype(keys: Keys<'_>) -> Result<()> {
    let args: Vec<&str> = match keys {
        Keys::Text(t) => vec!["--", t],
        Keys::Paste(PasteChord::CtrlV) => vec!["-M", "ctrl", "v", "-m", "ctrl"],
        Keys::Paste(PasteChord::ShiftInsert) => vec!["-M", "shift", "-k", "Insert", "-m", "shift"],
        Keys::Copy => vec!["-M", "ctrl", "c", "-m", "ctrl"],
    };
    match std::process::Command::new("wtype").args(&args).status() {
        Ok(status) if status.success() => Ok(()),
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
pub fn wtype_available() -> bool {
    crate::clipboard::on_path("wtype")
}

/// Whether this is a Wayland session (Linux).
pub fn is_wayland_session() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|s| s.eq_ignore_ascii_case("wayland"))
                .unwrap_or(false))
}

#[cfg(all(feature = "portal", target_os = "linux"))]
mod portal;

// ---- Secure-field detection -------------------------------------------------

/// Best-effort check of whether keyboard focus is in a password/secure field.
///
/// - **Windows**: UI Automation's `IsPassword` on the focused element — reliable.
/// - **Linux/Wayland & others**: no reliable per-field API exists, so this is
///   `Unknown` (honest). The `strict_secure` policy decides whether `Unknown`
///   blocks injection.
#[cfg(all(feature = "injection", target_os = "windows"))]
fn detect_secure_field() -> SecureFieldStatus {
    secure_windows::focused_field_status()
}

#[cfg(not(all(feature = "injection", target_os = "windows")))]
fn detect_secure_field() -> SecureFieldStatus {
    SecureFieldStatus::Unknown
}

/// Windows secure-field detection via UI Automation. Every step fails safe to
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::MockClipboard;

    fn service(injector: MockTextInjector, clip: Arc<MockClipboard>) -> InjectionService {
        InjectionService::new(Box::new(injector), Box::new(clip)).with_sleep(|_| {})
    }

    fn cfg(mode: InjectionMode, preserve: bool) -> InjectionConfig {
        InjectionConfig {
            mode,
            preserve_clipboard: preserve,
            ..Default::default()
        }
    }

    #[test]
    fn paste_mode_restores_the_users_clipboard() {
        let clip = Arc::new(MockClipboard::with_text("user's copy"));
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let log = inj.log();
        let mut s = service(inj, clip.clone());
        s.set_config(cfg(InjectionMode::Paste, true));
        let d = s.deliver("hello world", Some("firefox"));
        assert_eq!(d.result, InjectionResult::Success);
        assert_eq!(d.method, InjectionMethod::Paste);
        assert_eq!(log.last().as_deref(), Some("hello world"));
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("user's copy"));
    }

    #[test]
    fn terminal_paste_stages_and_restores_the_primary_selection() {
        let clip = Arc::new(MockClipboard::new());
        clip.set_primary(Some("highlighted")).unwrap();
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let mut s = service(inj, clip.clone());
        s.set_config(cfg(InjectionMode::Paste, true));
        s.deliver("dictated", Some("xterm"));
        assert_eq!(
            clip.primary_history(),
            vec![
                Some("highlighted".to_string()),
                Some("dictated".to_string()),
                Some("highlighted".to_string()),
            ],
            "terminals paste PRIMARY on Shift+Insert, so it must hold the text"
        );
    }

    #[test]
    fn ordinary_paste_leaves_the_primary_selection_alone() {
        let clip = Arc::new(MockClipboard::new());
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let mut s = service(inj, clip.clone());
        s.set_config(cfg(InjectionMode::Paste, true));
        s.deliver("dictated", Some("firefox"));
        assert!(clip.primary_history().is_empty());
    }

    #[test]
    fn paste_without_preserve_leaves_the_text() {
        let clip = Arc::new(MockClipboard::with_text("user's copy"));
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let mut s = service(inj, clip.clone());
        s.set_config(cfg(InjectionMode::Paste, false));
        s.deliver("dictated", None);
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("dictated"));
    }

    #[test]
    fn auto_types_short_and_pastes_long_text() {
        let clip = Arc::new(MockClipboard::new());
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let chords = inj.chords();
        let mut s = service(inj, clip);
        s.set_config(cfg(InjectionMode::Auto, true));
        assert_eq!(s.deliver("short", None).method, InjectionMethod::Typed);
        assert!(chords.lock().unwrap().is_empty());
        let long = "a considerably longer sentence that is worth pasting quickly";
        assert_eq!(s.deliver(long, None).method, InjectionMethod::Paste);
        assert_eq!(
            s.deliver("line one\nline two", None).method,
            InjectionMethod::Paste
        );
    }

    #[test]
    fn terminals_get_shift_insert() {
        assert_eq!(
            paste_chord_for(Some("gnome-terminal-server")),
            PasteChord::ShiftInsert
        );
        assert_eq!(
            paste_chord_for(Some("WindowsTerminal")),
            PasteChord::ShiftInsert
        );
        assert_eq!(paste_chord_for(Some("firefox")), PasteChord::CtrlV);
        assert_eq!(paste_chord_for(None), PasteChord::CtrlV);
    }

    #[test]
    fn failed_paste_falls_back_to_typing_and_restores() {
        let clip = Arc::new(MockClipboard::with_text("mine"));
        let inj = MockTextInjector::new()
            .with_clipboard(clip.clone())
            .failing_paste();
        let log = inj.log();
        let mut s = service(inj, clip.clone());
        s.set_config(cfg(InjectionMode::Paste, true));
        let d = s.deliver("fallback text", None);
        assert_eq!(d.method, InjectionMethod::Typed);
        assert_eq!(log.last().as_deref(), Some("fallback text"));
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("mine"));
    }

    #[test]
    fn total_failure_leaves_text_on_clipboard() {
        let clip = Arc::new(MockClipboard::with_text("mine"));
        let mut s = service(MockTextInjector::failing(), clip.clone());
        s.set_config(cfg(InjectionMode::Type, true));
        let d = s.deliver("keep me", None);
        assert_eq!(d.result, InjectionResult::Failed);
        assert_eq!(d.method, InjectionMethod::Clipboard);
        assert!(d.error.is_some());
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("keep me"));
    }

    #[test]
    fn secure_and_strict_unknown_fields_are_blocked() {
        let clip = Arc::new(MockClipboard::new());
        let s = service(MockTextInjector::secure(), clip.clone());
        assert_eq!(s.deliver("pw", None).method, InjectionMethod::Blocked);
        let mut s = service(MockTextInjector::unknown(), clip);
        assert_eq!(s.deliver("ok", None).result, InjectionResult::Success);
        s.set_config(InjectionConfig {
            strict_secure: true,
            ..Default::default()
        });
        assert_eq!(
            s.deliver("no", None).result,
            InjectionResult::BlockedSecureField
        );
    }

    #[test]
    fn selection_capture_copies_then_restores_clipboard() {
        let clip = Arc::new(MockClipboard::with_text("user's copy"));
        let inj = MockTextInjector::new()
            .with_clipboard(clip.clone())
            .with_selection("the selected words");
        let s = service(inj, clip.clone());
        assert_eq!(
            s.capture_selection(),
            SelectionCapture::Captured("the selected words".into())
        );
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("user's copy"));
    }

    #[test]
    fn selection_equal_to_clipboard_is_still_detected() {
        let clip = Arc::new(MockClipboard::with_text("same"));
        let inj = MockTextInjector::new()
            .with_clipboard(clip.clone())
            .with_selection("same");
        let s = service(inj, clip);
        assert_eq!(
            s.capture_selection(),
            SelectionCapture::Captured("same".into())
        );
    }

    #[test]
    fn no_selection_and_secure_selection() {
        let clip = Arc::new(MockClipboard::with_text("x"));
        let inj = MockTextInjector::new().with_clipboard(clip.clone());
        let s = service(inj, clip.clone());
        assert_eq!(s.capture_selection(), SelectionCapture::NoSelection);
        assert_eq!(clip.get_text().unwrap().as_deref(), Some("x"));
        let s = service(MockTextInjector::secure().with_selection("pw"), clip);
        assert_eq!(s.capture_selection(), SelectionCapture::BlockedSecureField);
    }
}
