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
use crate::selection::{SelectOutcome, SelectionPeek};

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
/// - **Linux**: AT-SPI, the accessibility bus (X11 and Wayland alike): the
///   editable object that last took focus, through its `Text` interface. GTK,
///   Qt, LibreOffice and Firefox fields are readable; apps that don't expose
///   accessibility read as `Unreadable`.
#[cfg(target_os = "windows")]
pub fn focused_field_read() -> FieldRead {
    uia_field::read()
}

#[cfg(all(target_os = "linux", feature = "portal"))]
pub fn focused_field_read() -> FieldRead {
    atspi_field::read()
}

#[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "portal"))))]
pub fn focused_field_read() -> FieldRead {
    FieldRead::Unreadable
}

// ---- Selection, through accessibility (see `crate::selection`) ------------------

/// What's selected in the focused field, read without the clipboard.
#[cfg(target_os = "windows")]
pub fn peek_selection() -> SelectionPeek {
    uia_field::peek_selection()
}
#[cfg(all(target_os = "linux", feature = "portal"))]
pub fn peek_selection() -> SelectionPeek {
    atspi_field::peek_selection()
}
#[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "portal"))))]
pub fn peek_selection() -> SelectionPeek {
    SelectionPeek::Nothing
}

/// Find `text` (typed by Orttaai) in the focused field and select it.
#[cfg(target_os = "windows")]
pub fn select_inserted(text: &str) -> SelectOutcome {
    uia_field::select_inserted(text)
}
#[cfg(all(target_os = "linux", feature = "portal"))]
pub fn select_inserted(text: &str) -> SelectOutcome {
    atspi_field::select_inserted(text)
}
#[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "portal"))))]
pub fn select_inserted(_text: &str) -> SelectOutcome {
    SelectOutcome::NotFound
}

/// The focused field's selected text; `None` when it can't be read.
#[cfg(target_os = "windows")]
pub fn selected_text() -> Option<String> {
    uia_field::selected_text()
}
#[cfg(all(target_os = "linux", feature = "portal"))]
pub fn selected_text() -> Option<String> {
    atspi_field::selected_text()
}
#[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "portal"))))]
pub fn selected_text() -> Option<String> {
    None
}

/// If `text` is still selected, put the caret just after it.
#[cfg(target_os = "windows")]
pub fn deselect(text: &str) {
    uia_field::deselect(text)
}
#[cfg(all(target_os = "linux", feature = "portal"))]
pub fn deselect(text: &str) {
    atspi_field::deselect(text)
}
#[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "portal"))))]
pub fn deselect(_text: &str) {}

/// Linux: the focused field over AT-SPI. A worker thread joins the
/// accessibility bus, asks apps to report focus changes, and remembers the
/// object that last gained focus; reads go to that object. Every step fails
/// safe to `Unreadable`, and each call to an app is capped so a hung app can't
/// stall a dictation.
#[cfg(all(target_os = "linux", feature = "portal"))]
mod atspi_field {
    use crate::injection::edit_watch::FieldRead;
    use crate::selection::{locate_inserted, same_text, SelectOutcome, SelectionPeek};
    use std::sync::mpsc;
    use std::sync::OnceLock;
    use std::time::Duration;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

    /// AT-SPI state bits (`AtspiStateType`) and roles (`AtspiRole`).
    const STATE_EDITABLE: u32 = 7;
    const STATE_FOCUSED: u32 = 12;
    const STATE_SINGLE_LINE: u32 = 26;
    const ROLE_PASSWORD_TEXT: u32 = 40;
    const ROLE_TERMINAL: u32 = 60;
    const TEXT: &str = "org.a11y.atspi.Text";
    /// Reads while waiting for the app to show a new selection.
    const CONFIRM_ATTEMPTS: u32 = 5;
    const CONFIRM_WAIT: Duration = Duration::from_millis(30);
    /// A single call to an app.
    const CALL_TIMEOUT: Duration = Duration::from_millis(800);
    /// A whole read, as the watcher sees it.
    const READ_TIMEOUT: Duration = Duration::from_millis(2500);
    /// Fields longer than this (a whole document) aren't read.
    const MAX_CHARS: i32 = 100_000;

    /// One job for the worker; each answers on its own channel.
    enum Request {
        Read(mpsc::Sender<FieldRead>),
        Peek(mpsc::Sender<SelectionPeek>),
        /// When it was asked: a select whose caller has stopped waiting is
        /// never started.
        Select(String, mpsc::Sender<SelectOutcome>, std::time::Instant),
        Selected(mpsc::Sender<Option<String>>),
        Deselect(String, mpsc::Sender<()>),
    }

    static WORKER: OnceLock<Option<tokio::sync::mpsc::UnboundedSender<Request>>> = OnceLock::new();

    /// Start listening for focus changes (once). Called at startup so the
    /// field you're in is known before the first dictation.
    pub fn start() {
        worker();
    }

    fn worker() -> Option<&'static tokio::sync::mpsc::UnboundedSender<Request>> {
        WORKER
            .get_or_init(|| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
                std::thread::Builder::new()
                    .name("orttaai-atspi".into())
                    .spawn(move || {
                        let rt = match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(rt) => rt,
                            Err(_) => return,
                        };
                        rt.block_on(run(rx));
                    })
                    .ok()?;
                Some(tx)
            })
            .as_ref()
    }

    /// Send one job and wait for its answer; `fallback` when the bus is
    /// unavailable or the app doesn't answer in time.
    fn ask<T>(job: impl FnOnce(mpsc::Sender<T>) -> Request, fallback: T) -> T {
        let Some(worker) = worker() else {
            return fallback;
        };
        let (tx, rx) = mpsc::channel();
        if worker.send(job(tx)).is_err() {
            return fallback;
        }
        rx.recv_timeout(READ_TIMEOUT).unwrap_or(fallback)
    }

    pub fn read() -> FieldRead {
        ask(Request::Read, FieldRead::Unreadable)
    }

    pub fn peek_selection() -> SelectionPeek {
        ask(Request::Peek, SelectionPeek::Nothing)
    }

    pub fn select_inserted(text: &str) -> SelectOutcome {
        ask(
            |tx| Request::Select(text.to_string(), tx, std::time::Instant::now()),
            SelectOutcome::NotFound,
        )
    }

    pub fn selected_text() -> Option<String> {
        ask(Request::Selected, None)
    }

    pub fn deselect(text: &str) {
        ask(|tx| Request::Deselect(text.to_string(), tx), ())
    }

    /// The object that last gained focus: its app's bus name and path.
    type Focus = std::sync::Arc<std::sync::Mutex<Option<(String, OwnedObjectPath)>>>;

    async fn run(mut requests: tokio::sync::mpsc::UnboundedReceiver<Request>) {
        let bus = match connect().await {
            Ok(bus) => bus,
            Err(e) => {
                tracing::info!("accessibility bus unavailable, edits can't be read: {e}");
                // Answer every job so callers never wait out the timeout.
                while let Some(request) = requests.recv().await {
                    answer_unavailable(request);
                }
                return;
            }
        };
        let focus: Focus = Default::default();
        tokio::spawn(follow_focus(bus.clone(), focus.clone()));
        while let Some(request) = requests.recv().await {
            let Some((app, path)) = focus.lock().unwrap().clone() else {
                answer_unavailable(request);
                continue;
            };
            let field = Field {
                bus: &bus,
                app: &app,
                path: &path,
            };
            match request {
                Request::Read(reply) => {
                    let read = read_field(&bus, &app, &path)
                        .await
                        .unwrap_or(FieldRead::Unreadable);
                    let _ = reply.send(read);
                }
                Request::Peek(reply) => {
                    let _ = reply.send(field.peek().await.unwrap_or(SelectionPeek::Nothing));
                }
                Request::Select(text, reply, asked) => {
                    // Selected after Orttaai stopped waiting, nothing would
                    // edit it and the next key typed would replace it.
                    if asked.elapsed() >= READ_TIMEOUT {
                        let _ = reply.send(SelectOutcome::NotFound);
                        continue;
                    }
                    let outcome = field.select(&text).await.unwrap_or(SelectOutcome::NotFound);
                    let selected = matches!(outcome, SelectOutcome::Selected(_));
                    if reply.send(outcome).is_err() && selected {
                        field.deselect(&text).await;
                    }
                }
                Request::Selected(reply) => {
                    let _ = reply.send(field.selected_text().await);
                }
                Request::Deselect(text, reply) => {
                    field.deselect(&text).await;
                    let _ = reply.send(());
                }
            }
        }
    }

    fn answer_unavailable(request: Request) {
        match request {
            Request::Read(r) => drop(r.send(FieldRead::Unreadable)),
            Request::Peek(r) => drop(r.send(SelectionPeek::Nothing)),
            Request::Select(_, r, _) => drop(r.send(SelectOutcome::NotFound)),
            Request::Selected(r) => drop(r.send(None)),
            Request::Deselect(_, r) => drop(r.send(())),
        }
    }

    /// The focused field: its text, selection and caret through AT-SPI's
    /// `Text` interface. Offsets are characters.
    struct Field<'a> {
        bus: &'a zbus::Connection,
        app: &'a str,
        path: &'a OwnedObjectPath,
    }

    /// What kind of field has focus.
    enum Kind {
        Text {
            single_line: bool,
        },
        Secure,
        /// Not focused any more, not editable, or a terminal.
        Other,
    }

    impl Field<'_> {
        async fn kind(&self) -> Option<Kind> {
            const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
            let states: Vec<u32> =
                call(self.bus, self.app, self.path, ACCESSIBLE, "GetState", &()).await?;
            let has = |bit: u32| {
                states
                    .get((bit / 32) as usize)
                    .is_some_and(|word| word & (1 << (bit % 32)) != 0)
            };
            if !has(STATE_FOCUSED) || !has(STATE_EDITABLE) {
                return Some(Kind::Other);
            }
            let role: u32 = call(self.bus, self.app, self.path, ACCESSIBLE, "GetRole", &()).await?;
            Some(match role {
                ROLE_PASSWORD_TEXT => Kind::Secure,
                ROLE_TERMINAL => Kind::Other,
                _ => Kind::Text {
                    single_line: has(STATE_SINGLE_LINE),
                },
            })
        }

        async fn int_property(&self, name: &str) -> Option<i32> {
            let v: OwnedValue = call(
                self.bus,
                self.app,
                self.path,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(TEXT, name),
            )
            .await?;
            i32::try_from(v).ok()
        }

        async fn text(&self, start: i32, end: i32) -> Option<String> {
            call(
                self.bus,
                self.app,
                self.path,
                TEXT,
                "GetText",
                &(start, end),
            )
            .await
        }

        /// The first selection's offsets, `None` when nothing is selected.
        async fn selection(&self) -> Option<(i32, i32)> {
            let count: i32 =
                call(self.bus, self.app, self.path, TEXT, "GetNSelections", &()).await?;
            if count < 1 {
                return None;
            }
            let (start, end): (i32, i32) = call(
                self.bus,
                self.app,
                self.path,
                TEXT,
                "GetSelection",
                &(0i32,),
            )
            .await?;
            (end > start).then_some((start, end))
        }

        async fn peek(&self) -> Option<SelectionPeek> {
            let single_line = match self.kind().await? {
                Kind::Secure => return Some(SelectionPeek::Secure),
                Kind::Other => return Some(SelectionPeek::Nothing),
                Kind::Text { single_line } => single_line,
            };
            let Some((start, end)) = self.selection().await else {
                return Some(SelectionPeek::Nothing);
            };
            let text = self.text(start, end).await?;
            Some(if text.trim().is_empty() {
                SelectionPeek::Nothing
            } else {
                SelectionPeek::Selected { text, single_line }
            })
        }

        async fn selected_text(&self) -> Option<String> {
            match self.kind().await? {
                Kind::Text { .. } => {}
                _ => return None,
            }
            match self.selection().await {
                Some((start, end)) => self.text(start, end).await,
                None => Some(String::new()),
            }
        }

        async fn set_caret(&self, offset: i32) {
            let _: Option<bool> = call(
                self.bus,
                self.app,
                self.path,
                TEXT,
                "SetCaretOffset",
                &(offset,),
            )
            .await;
        }

        async fn select(&self, needle: &str) -> Option<SelectOutcome> {
            match self.kind().await? {
                Kind::Secure => return Some(SelectOutcome::Secure),
                Kind::Other => return Some(SelectOutcome::NotFound),
                Kind::Text { .. } => {}
            }
            let count = self.int_property("CharacterCount").await?;
            if !(0..=MAX_CHARS).contains(&count) {
                return Some(SelectOutcome::NotFound);
            }
            let field = self.text(0, count).await?;
            let caret = self.int_property("CaretOffset").await;
            let Some((start, end)) =
                locate_inserted(needle, &field, caret.and_then(|c| usize::try_from(c).ok()))
            else {
                return Some(SelectOutcome::NotFound);
            };
            let (start, end) = (i32::try_from(start).ok()?, i32::try_from(end).ok()?);
            let had_selection = self.selection().await.is_some();
            let method = if had_selection {
                "SetSelection"
            } else {
                "AddSelection"
            };
            let taken: Option<bool> = if had_selection {
                call(
                    self.bus,
                    self.app,
                    self.path,
                    TEXT,
                    method,
                    &(0i32, start, end),
                )
                .await
            } else {
                call(self.bus, self.app, self.path, TEXT, method, &(start, end)).await
            };
            if taken == Some(true) {
                // Only a selection the app confirms is edited: the result is
                // pasted over whatever is selected.
                for attempt in 0..CONFIRM_ATTEMPTS {
                    if attempt > 0 {
                        tokio::time::sleep(CONFIRM_WAIT).await;
                    }
                    if self.selection().await == Some((start, end)) {
                        let text = self.text(start, end).await?;
                        if same_text(&text, needle) {
                            return Some(SelectOutcome::Selected(text));
                        }
                    }
                }
            }
            if let Some(caret) = caret {
                self.set_caret(caret).await;
            }
            Some(SelectOutcome::NotFound)
        }

        async fn deselect(&self, needle: &str) {
            let Some((start, end)) = self.selection().await else {
                return;
            };
            if !self
                .text(start, end)
                .await
                .is_some_and(|t| same_text(&t, needle))
            {
                return;
            }
            let _: Option<bool> = call(
                self.bus,
                self.app,
                self.path,
                TEXT,
                "RemoveSelection",
                &(0i32,),
            )
            .await;
            self.set_caret(end).await;
        }
    }

    /// The accessibility bus (a separate bus; its address comes from the
    /// session bus), with focus events switched on.
    async fn connect() -> zbus::Result<zbus::Connection> {
        let session = zbus::Connection::session().await?;
        let address: String = session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .await?
            .body()
            .deserialize()?;
        let bus = zbus::connection::Builder::address(address.as_str())?
            .build()
            .await?;
        // Toolkits only send the events someone has asked the registry for.
        // The call's signature changed in at-spi2-core 2.46; try both.
        let registry = |args: &'static str| {
            let bus = bus.clone();
            async move {
                let call = if args == "s" {
                    bus.call_method(
                        Some("org.a11y.atspi.Registry"),
                        "/org/a11y/atspi/registry",
                        Some("org.a11y.atspi.Registry"),
                        "RegisterEvent",
                        &("object:state-changed:focused",),
                    )
                    .await
                } else {
                    bus.call_method(
                        Some("org.a11y.atspi.Registry"),
                        "/org/a11y/atspi/registry",
                        Some("org.a11y.atspi.Registry"),
                        "RegisterEvent",
                        &("object:state-changed:focused", Vec::<String>::new(), ""),
                    )
                    .await
                };
                call.is_ok()
            }
        };
        if !registry("sass").await {
            registry("s").await;
        }
        Ok(bus)
    }

    /// Remember each object that gains focus.
    async fn follow_focus(bus: zbus::Connection, focus: Focus) {
        use futures_util::StreamExt;
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.a11y.atspi.Event.Object")
            .and_then(|r| r.member("StateChanged"))
            .map(|r| r.build())
        else {
            return;
        };
        let Ok(mut events) = zbus::MessageStream::for_match_rule(rule, &bus, None).await else {
            return;
        };
        while let Some(Ok(msg)) = events.next().await {
            let header = msg.header();
            let (Some(sender), Some(path)) = (header.sender(), header.path()) else {
                continue;
            };
            let body = msg.body();
            let Ok(body) = body.deserialize::<Structure>() else {
                continue;
            };
            let fields = body.fields();
            let gained = matches!(
                (fields.first(), fields.get(1)),
                (Some(Value::Str(kind)), Some(Value::I32(1))) if kind.as_str() == "focused"
            );
            if gained {
                *focus.lock().unwrap() = Some((sender.to_string(), path.to_owned().into()));
            }
        }
    }

    async fn call<R>(
        bus: &zbus::Connection,
        app: &str,
        path: &OwnedObjectPath,
        iface: &str,
        method: &str,
        args: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> Option<R>
    where
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let reply = tokio::time::timeout(
            CALL_TIMEOUT,
            bus.call_method(Some(app), path.as_ref(), Some(iface), method, args),
        )
        .await
        .ok()?
        .ok()?;
        reply.body().deserialize().ok()
    }

    async fn read_field(
        bus: &zbus::Connection,
        app: &str,
        path: &OwnedObjectPath,
    ) -> Option<FieldRead> {
        const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
        let states: Vec<u32> = call(bus, app, path, ACCESSIBLE, "GetState", &()).await?;
        let has = |bit: u32| {
            states
                .get((bit / 32) as usize)
                .is_some_and(|word| word & (1 << (bit % 32)) != 0)
        };
        // Focus moved on to something we weren't told about, or not a field.
        if !has(STATE_FOCUSED) || !has(STATE_EDITABLE) {
            return Some(FieldRead::Unreadable);
        }
        let role: u32 = call(bus, app, path, ACCESSIBLE, "GetRole", &()).await?;
        if role == ROLE_PASSWORD_TEXT {
            return Some(FieldRead::Secure);
        }
        let count: OwnedValue = call(
            bus,
            app,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &("org.a11y.atspi.Text", "CharacterCount"),
        )
        .await?;
        let count = i32::try_from(count).ok()?;
        if !(0..=MAX_CHARS).contains(&count) {
            return Some(FieldRead::Unreadable);
        }
        let text: String = call(
            bus,
            app,
            path,
            "org.a11y.atspi.Text",
            "GetText",
            &(0i32, count),
        )
        .await?;
        Some(FieldRead::Text(text))
    }
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

    // ---- Selection (see `crate::selection`) ----

    use crate::selection::{same_text, SelectOutcome, SelectionPeek};
    use windows::core::{Interface, BSTR};
    use windows::Win32::UI::Accessibility::{
        IUIAutomation2, IUIAutomationElement, IUIAutomationTextRange, TextPatternRangeEndpoint_End,
        TextPatternRangeEndpoint_Start, UIA_EditControlTypeId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, ES_MULTILINE, GWL_STYLE};

    /// How long one UI Automation call may wait on the app. These calls run
    /// on the dictation engine's thread, and a hung app would otherwise hold
    /// it for UI Automation's own 20 s.
    const CONNECTION_TIMEOUT_MS: u32 = 1_000;
    const TRANSACTION_TIMEOUT_MS: u32 = 2_000;

    /// Reads while waiting for the app to show a new selection.
    const CONFIRM_ATTEMPTS: u32 = 5;
    const CONFIRM_WAIT: std::time::Duration = std::time::Duration::from_millis(30);

    enum Focused {
        Secure,
        Text(IUIAutomationElement, IUIAutomationTextPattern),
        Other,
    }

    unsafe fn focused() -> windows::core::Result<Focused> {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(bounded) = automation.cast::<IUIAutomation2>() {
            let _ = bounded.SetConnectionTimeout(CONNECTION_TIMEOUT_MS);
            let _ = bounded.SetTransactionTimeout(TRANSACTION_TIMEOUT_MS);
        }
        let element = automation.GetFocusedElement()?;
        if element.CurrentIsPassword()?.as_bool() {
            return Ok(Focused::Secure);
        }
        // Text that can't be typed into (a web page, a PDF) is never edited:
        // the result would be pasted into nothing.
        if let Ok(value) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            if value.CurrentIsReadOnly().is_ok_and(|r| r.as_bool()) {
                return Ok(Focused::Other);
            }
        }
        Ok(
            match element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                Ok(pattern) => Focused::Text(element, pattern),
                Err(_) => Focused::Other,
            },
        )
    }

    /// Whether the edit box takes more than one line: a Win32 edit with the
    /// multi-line style, a web text box marked multiline (Chromium and
    /// Electron say so in their ARIA properties), or text with a line break.
    unsafe fn is_multiline(element: &IUIAutomationElement, text: &str) -> bool {
        if text.contains(['\n', '\r']) {
            return true;
        }
        if element.CurrentAriaProperties().is_ok_and(|aria| {
            aria.to_string()
                .to_ascii_lowercase()
                .contains("multiline=true")
        }) {
            return true;
        }
        match element.CurrentNativeWindowHandle() {
            Ok(hwnd) if !hwnd.is_invalid() => GetWindowLongW(hwnd, GWL_STYLE) & ES_MULTILINE != 0,
            _ => false,
        }
    }

    /// The first selected range; a caret is an empty one.
    unsafe fn first_selection(
        pattern: &IUIAutomationTextPattern,
    ) -> Option<IUIAutomationTextRange> {
        let ranges = pattern.GetSelection().ok()?;
        if ranges.Length().ok()? < 1 {
            return None;
        }
        ranges.GetElement(0).ok()
    }

    unsafe fn text_of(range: &IUIAutomationTextRange) -> Option<String> {
        range.GetText(MAX_CHARS).ok().map(|t| t.to_string())
    }

    pub fn peek_selection() -> SelectionPeek {
        unsafe { peek_inner() }.unwrap_or(SelectionPeek::Nothing)
    }

    unsafe fn peek_inner() -> windows::core::Result<SelectionPeek> {
        let (element, pattern) = match focused()? {
            Focused::Secure => return Ok(SelectionPeek::Secure),
            Focused::Other => return Ok(SelectionPeek::Nothing),
            Focused::Text(e, p) => (e, p),
        };
        let Some(text) = first_selection(&pattern).and_then(|r| text_of(&r)) else {
            return Ok(SelectionPeek::Nothing);
        };
        if text.trim().is_empty() {
            return Ok(SelectionPeek::Nothing);
        }
        // Address bars and search boxes select all their text when clicked.
        // UI Automation has no "one line" flag, so: an edit box, everything
        // selected, and nothing saying it's multi-line.
        let edit_box = element.CurrentControlType()? == UIA_EditControlTypeId;
        let all = pattern.DocumentRange().ok().and_then(|d| text_of(&d));
        let whole = all.as_deref().is_some_and(|all| same_text(all, &text));
        let single_line =
            edit_box && whole && !is_multiline(&element, all.as_deref().unwrap_or(&text));
        Ok(SelectionPeek::Selected { text, single_line })
    }

    pub fn selected_text() -> Option<String> {
        unsafe {
            match focused().ok()? {
                Focused::Text(_, pattern) => Some(
                    first_selection(&pattern)
                        .and_then(|r| text_of(&r))
                        .unwrap_or_default(),
                ),
                _ => None,
            }
        }
    }

    pub fn select_inserted(text: &str) -> SelectOutcome {
        unsafe { select_inner(text) }.unwrap_or(SelectOutcome::NotFound)
    }

    /// `needle` as the field may store it: Windows fields keep line breaks
    /// as `\r\n` or `\r`.
    fn spellings(needle: &str) -> Vec<String> {
        let mut out = vec![needle.to_string()];
        if needle.contains('\n') {
            out.push(needle.replace('\n', "\r\n"));
            out.push(needle.replace('\n', "\r"));
        }
        out
    }

    /// The copy nearest before the caret, or the only copy in the field.
    unsafe fn find(
        pattern: &IUIAutomationTextPattern,
        caret: Option<&IUIAutomationTextRange>,
        needle: &str,
    ) -> Option<IUIAutomationTextRange> {
        let doc = pattern.DocumentRange().ok()?;
        for spelling in spellings(needle) {
            let wanted = BSTR::from(spelling.as_str());
            if let Some(caret) = caret {
                let before = doc.Clone().ok()?;
                if before
                    .MoveEndpointByRange(
                        TextPatternRangeEndpoint_End,
                        caret,
                        TextPatternRangeEndpoint_End,
                    )
                    .is_ok()
                {
                    if let Ok(found) = before.FindText(&wanted, true, false) {
                        return Some(found);
                    }
                }
            }
            if let (Ok(first), Ok(last)) = (
                doc.FindText(&wanted, false, false),
                doc.FindText(&wanted, true, false),
            ) {
                let same = first
                    .CompareEndpoints(
                        TextPatternRangeEndpoint_Start,
                        &last,
                        TextPatternRangeEndpoint_Start,
                    )
                    .ok()?
                    == 0;
                return same.then_some(first);
            }
        }
        None
    }

    unsafe fn select_inner(text: &str) -> windows::core::Result<SelectOutcome> {
        let pattern = match focused()? {
            Focused::Secure => return Ok(SelectOutcome::Secure),
            Focused::Other => return Ok(SelectOutcome::NotFound),
            Focused::Text(_, p) => p,
        };
        let needle = text.trim();
        if needle.is_empty() {
            return Ok(SelectOutcome::NotFound);
        }
        let caret = first_selection(&pattern);
        let Some(range) = find(&pattern, caret.as_ref(), needle) else {
            return Ok(SelectOutcome::NotFound);
        };
        range.Select()?;
        // Only a selection the app confirms is edited: the result is pasted
        // over whatever is selected.
        for attempt in 0..CONFIRM_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(CONFIRM_WAIT);
            }
            let now = first_selection(&pattern).and_then(|r| text_of(&r));
            if let Some(now) = now.filter(|t| same_text(t, needle)) {
                return Ok(SelectOutcome::Selected(now));
            }
        }
        if let Some(caret) = caret {
            let _ = caret.Select();
        }
        Ok(SelectOutcome::NotFound)
    }

    pub fn deselect(text: &str) {
        unsafe {
            let Ok(Focused::Text(_, pattern)) = focused() else {
                return;
            };
            let Some(range) = first_selection(&pattern) else {
                return;
            };
            if !text_of(&range).is_some_and(|t| same_text(&t, text)) {
                return;
            }
            if range
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_Start,
                    &range,
                    TextPatternRangeEndpoint_End,
                )
                .is_ok()
            {
                let _ = range.Select();
            }
        }
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
    /// Put `text` straight into `window` when it's one of Orttaai's own (Setup's
    /// Quick Test, Chat AI), instead of synthesizing keystrokes into ourselves,
    /// which Windows doesn't reliably deliver. `true` when it was handed over.
    fn insert_into_own(&self, _window: u64, _text: &str) -> bool {
        false
    }
}

/// Whether `window` belongs to this process (Windows; elsewhere `false`).
pub fn is_own_window(window: u64) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(
                HWND(window as usize as *mut core::ffi::c_void),
                Some(&mut pid),
            )
        };
        pid != 0 && pid == std::process::id()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        false
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
    fn insert_into_own(&self, window: u64, text: &str) -> bool {
        (**self).insert_into_own(window, text)
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

/// The window in front, unless it's the taskbar (clicked to reach Orttaai's
/// tray icon) or a window of Orttaai's you can't type into (the pill): a
/// dictation must never take those as the app to type into. Orttaai's main
/// window counts, so dictating into Chat AI works. `None` then means
/// "whatever is in front when the text is typed", as on a system without
/// window tracking.
#[cfg(target_os = "windows")]
pub fn focused_window() -> Option<u64> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    win::is_app_window(hwnd).then_some(hwnd.0 as usize as u64)
}

/// Start following focus in the background. Windows: which app window is in
/// front, so a click on the tray icon (which brings the taskbar forward) can
/// hand focus back to it. Linux: which text field has focus, so edits to a
/// dictation can be read (learn from edits).
pub fn track_foreground() {
    #[cfg(target_os = "windows")]
    win::track_foreground();
    #[cfg(all(target_os = "linux", feature = "portal"))]
    atspi_field::start();
}

/// After a click on Orttaai's tray icon or its menu, bring back the window that
/// was in front before the taskbar took it, so the dictation goes there and its
/// caret shows. Returns whether an app window is in front afterwards (always
/// `true` off Windows, where clicking the tray doesn't move focus).
pub fn return_from_tray() -> bool {
    #[cfg(target_os = "windows")]
    {
        win::return_from_tray()
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

/// NOTE: `#[cfg(windows)]`-only; verified by the Windows cross-check build.
#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
        GetWindowThreadProcessId, IsWindow, IsWindowVisible, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE,
        MSG, WINEVENT_OUTOFCONTEXT, WS_EX_NOACTIVATE,
    };

    /// The taskbar (main and secondary monitors), the notification area's
    /// overflow (Windows 10 and 11) and the shell's flyout hosts.
    const SHELL_CLASSES: &[&str] = &[
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "XamlExplorerHostIslandWindow",
    ];

    /// The last app window that was in front (an HWND; 0 = none yet).
    static LAST_APP_WINDOW: AtomicU64 = AtomicU64::new(0);

    fn is_shell_surface(hwnd: HWND) -> bool {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        n > 0 && SHELL_CLASSES.contains(&String::from_utf16_lossy(&buf[..n as usize]).as_str())
    }

    fn is_own(hwnd: HWND) -> bool {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid == std::process::id()
    }

    /// One of Orttaai's windows you can type into (the main window, for Chat
    /// AI or the Memory fields), not the pill (`WS_EX_NOACTIVATE`, never in
    /// front) or the tray's hidden message window.
    fn is_own_typing_window(hwnd: HWND) -> bool {
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        unsafe { IsWindowVisible(hwnd) }.as_bool() && ex & WS_EX_NOACTIVATE.0 == 0
    }

    /// A window someone could be typing into: any app's, Orttaai's main window
    /// included, but not the taskbar.
    pub(super) fn is_app_window(hwnd: HWND) -> bool {
        !hwnd.0.is_null()
            && !is_shell_surface(hwnd)
            && (!is_own(hwnd) || is_own_typing_window(hwnd))
    }

    fn note(hwnd: HWND) {
        if is_app_window(hwnd) && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            LAST_APP_WINDOW.store(hwnd.0 as usize as u64, Ordering::SeqCst);
        }
    }

    unsafe extern "system" fn on_foreground(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        note(hwnd);
    }

    pub(super) fn track_foreground() {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("orttaai-foreground".into())
                .spawn(|| unsafe {
                    // Out-of-context events arrive through this thread's queue.
                    let hook = SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(on_foreground),
                        0,
                        0,
                        // Own windows too: the main window can be the one
                        // to return to (dictating into Chat AI).
                        WINEVENT_OUTOFCONTEXT,
                    );
                    if hook.is_invalid() {
                        tracing::warn!("couldn't watch the foreground window");
                        return;
                    }
                    note(GetForegroundWindow());
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        DispatchMessageW(&msg);
                    }
                });
        });
    }

    pub(super) fn return_from_tray() -> bool {
        let front = unsafe { GetForegroundWindow() };
        if is_app_window(front) {
            return true;
        }
        let last = LAST_APP_WINDOW.load(Ordering::SeqCst);
        let hwnd = HWND(last as usize as *mut core::ffi::c_void);
        if last == 0 || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return false;
        }
        super::activate_window(last);
        for _ in 0..10 {
            if unsafe { GetForegroundWindow() } == hwnd {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        false
    }
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
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let hwnd = HWND(window as usize as *mut core::ffi::c_void);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // Refused by the foreground lock: share the front window's input
        // state for a moment, the way Windows lets the app in front hand over.
        let front = GetForegroundWindow();
        if front == hwnd {
            return true;
        }
        let front_thread = GetWindowThreadProcessId(front, None);
        let me = GetCurrentThreadId();
        if front_thread == 0
            || front_thread == me
            || !AttachThreadInput(me, front_thread, true).as_bool()
        {
            return false;
        }
        let _ = BringWindowToTop(hwnd);
        let done = SetForegroundWindow(hwnd).as_bool();
        let _ = AttachThreadInput(me, front_thread, false);
        done
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

/// The app's name from its executable path: the file name without `.exe`.
#[cfg(any(target_os = "windows", test))]
fn exe_stem(path: &str) -> Option<String> {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file
        .strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(file);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Store (UWP) apps draw inside a frame owned by `ApplicationFrameHost`, so
/// the frame's process names the host, not the app (Sticky Notes, Calculator,
/// Photos).
#[cfg(any(target_os = "windows", test))]
fn is_frame_host(stem: &str) -> bool {
    stem.eq_ignore_ascii_case("ApplicationFrameHost")
}

/// A Store app's name: the executable's description ("Microsoft Sticky
/// Notes"), else the frame's title, else the executable's name.
#[cfg(any(target_os = "windows", test))]
fn store_app_name(
    description: Option<String>,
    title: Option<String>,
    stem: Option<String>,
) -> Option<String> {
    let clean = |s: Option<String>| {
        s.map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.chars().count() <= 60)
    };
    clean(description)
        .or_else(|| clean(title))
        .or_else(|| clean(stem))
}

/// NOTE: `#[cfg(windows)]`-only — cannot be compiled on the Linux dev host; it is
/// verified by the Windows cross-check build. Fails safe to `None`.
#[cfg(target_os = "windows")]
fn focused_app_windows() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let pid = win_app::window_pid(hwnd)?;
    let path = win_app::process_path(pid);
    let stem = path.as_deref().and_then(exe_stem);
    if stem.as_deref().is_some_and(is_frame_host) {
        // The app is the process behind the frame's content.
        let inner = win_app::content_pid(hwnd, pid).and_then(win_app::process_path);
        return store_app_name(
            inner.as_deref().and_then(win_app::file_description),
            win_app::window_title(hwnd),
            inner.as_deref().and_then(exe_stem),
        );
    }
    // A process Orttaai may not open (an elevated one): its window's title.
    stem.or_else(|| win_app::window_title(hwnd))
}

#[cfg(target_os = "windows")]
mod win_app {
    use windows::core::{BOOL, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH};
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetWindowTextW, GetWindowThreadProcessId,
    };

    pub fn window_pid(hwnd: HWND) -> Option<u32> {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid)
    }

    pub fn process_path(pid: u32) -> Option<String> {
        unsafe {
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
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    pub fn window_title(hwnd: HWND) -> Option<String> {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }

    /// The first child window that belongs to another process than the frame.
    pub fn content_pid(frame: HWND, frame_pid: u32) -> Option<u32> {
        struct Search {
            frame_pid: u32,
            found: Option<u32>,
        }
        unsafe extern "system" fn visit(child: HWND, data: LPARAM) -> BOOL {
            let search = &mut *(data.0 as *mut Search);
            if let Some(pid) = window_pid(child) {
                if pid != search.frame_pid {
                    search.found = Some(pid);
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
        let mut search = Search {
            frame_pid,
            found: None,
        };
        unsafe {
            let _ = EnumChildWindows(
                Some(frame),
                Some(visit),
                LPARAM(&mut search as *mut Search as isize),
            );
        }
        search.found
    }

    /// The executable's FileDescription ("Microsoft Sticky Notes").
    pub fn file_description(path: &str) -> Option<String> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
            if size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast())
                .ok()?;
            let query = |key: &str| -> Option<(*const u16, u32)> {
                let key: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                VerQueryValueW(
                    data.as_ptr().cast(),
                    PCWSTR(key.as_ptr()),
                    &mut ptr,
                    &mut len,
                )
                .as_bool()
                .then_some((ptr as *const u16, len))
            };
            let (translation, len) = query("\\VarFileInfo\\Translation")?;
            if len < 4 {
                return None;
            }
            let pair = translation;
            let (lang, page) = (*pair, *pair.add(1));
            let (text, chars) = query(&format!(
                "\\StringFileInfo\\{lang:04x}{page:04x}\\FileDescription"
            ))?;
            let slice = std::slice::from_raw_parts(text, chars as usize);
            let value = String::from_utf16_lossy(slice)
                .trim_end_matches('\0')
                .trim()
                .to_string();
            (!value.is_empty()).then_some(value)
        }
    }
}

#[cfg(test)]
mod app_name_tests {
    use super::*;

    #[test]
    fn executable_names() {
        assert_eq!(
            exe_stem("C:\\Program Files\\Mozilla Firefox\\firefox.exe").as_deref(),
            Some("firefox")
        );
        assert_eq!(
            exe_stem("C:\\Windows\\System32\\NOTEPAD.EXE").as_deref(),
            Some("NOTEPAD")
        );
        assert_eq!(exe_stem("").as_deref(), None);
        assert!(is_frame_host("ApplicationFrameHost"));
        assert!(is_frame_host("applicationframehost"));
        assert!(!is_frame_host("firefox"));
    }

    #[test]
    fn a_store_app_is_named_for_itself_not_its_frame() {
        assert_eq!(
            store_app_name(
                Some("Microsoft Sticky Notes".into()),
                Some("Sticky Notes".into()),
                Some("Microsoft.Notes".into())
            )
            .as_deref(),
            Some("Microsoft Sticky Notes")
        );
        assert_eq!(
            store_app_name(None, Some("Sticky Notes".into()), None).as_deref(),
            Some("Sticky Notes")
        );
        assert_eq!(
            store_app_name(Some("  ".into()), None, Some("Microsoft.Notes".into())).as_deref(),
            Some("Microsoft.Notes")
        );
        assert_eq!(store_app_name(None, None, None), None);
    }
}
