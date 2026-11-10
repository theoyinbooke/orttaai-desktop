//! Reading and selecting text in the focused field through accessibility,
//! never the clipboard (Mac `SelectionCaptureService.peekSelection` and
//! `selectInsertedText`).
//!
//! - The dictation shortcut looks at what's selected when a recording starts,
//!   so an instruction ("make this shorter") can edit it. A Ctrl+C there would
//!   run on every dictation, so only accessibility is used: UI Automation on
//!   Windows, AT-SPI on Linux. Apps that don't expose their text read as
//!   nothing selected, and the dictation works as it always did.
//! - With nothing selected, the edit key finds the text Orttaai just typed in
//!   the field and selects it, so "make that more formal" needs no mouse.
//!
//! Everything fails safe: an unreadable field is "nothing selected", and a
//! selection the app doesn't confirm is never edited.

use std::sync::{Arc, Mutex};

/// What a quick look at the focused field's selection found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionPeek {
    /// `single_line`: a one-line field (an address bar, a search box), which
    /// usually selects all its text when clicked.
    Selected {
        text: String,
        single_line: bool,
    },
    Nothing,
    /// A password field: its text was not read.
    Secure,
}

/// Result of selecting text Orttaai typed earlier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectOutcome {
    /// Selected, and the app confirmed it: the selected text.
    Selected(String),
    /// The field can't be read, no longer holds the text (or holds it more
    /// than once away from the caret), or didn't take the selection.
    NotFound,
    Secure,
}

/// The focused field's selection, through accessibility.
pub trait FieldSelector: Send + Sync {
    fn peek(&self) -> SelectionPeek;
    /// Find `text` in the focused field and select it.
    fn select_inserted(&self, text: &str) -> SelectOutcome;
    /// The current selection; `None` when the field can't be read.
    fn selected_text(&self) -> Option<String>;
    /// Undo `select_inserted` when the edit didn't land: if `text` is still
    /// selected, put the caret just after it.
    fn deselect(&self, text: &str);
}

/// Whether `text` is still what's selected, right before an edit is pasted
/// over it. True when the field can't be read: nothing more is known then.
pub fn is_still_selected(selector: &dyn FieldSelector, text: &str) -> bool {
    match selector.selected_text() {
        None => true,
        Some(now) => same_text(&now, text),
    }
}

/// The same text, ignoring surrounding space and line-ending style (Windows
/// fields report `\r\n` or `\r` where Orttaai typed `\n`).
pub fn same_text(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.replace("\r\n", "\n")
            .replace('\r', "\n")
            .trim()
            .to_string()
    };
    norm(a) == norm(b)
}

/// Where `needle` is in `field`, as character offsets `(start, end)`. The
/// caret sits just after a fresh insertion, so the copy nearest before the
/// caret wins when the same words appear more than once. With no copy before
/// the caret, only a single copy in the whole field is trusted.
pub fn locate_inserted(needle: &str, field: &str, caret: Option<usize>) -> Option<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    let char_at = |byte: usize| field[..byte].chars().count();
    let span = |byte: usize| {
        let start = char_at(byte);
        (start, start + needle.chars().count())
    };
    if let Some(caret) = caret {
        let caret_byte = field
            .char_indices()
            .nth(caret)
            .map(|(i, _)| i)
            .unwrap_or(field.len());
        if let Some(byte) = field[..caret_byte].rfind(needle) {
            return Some(span(byte));
        }
    }
    let first = field.find(needle)?;
    let last = field.rfind(needle)?;
    (first == last).then(|| span(first))
}

/// The OS selector: [`crate::focus`]'s UI Automation (Windows) and AT-SPI
/// (Linux) helpers.
pub struct SystemFieldSelector;

impl FieldSelector for SystemFieldSelector {
    fn peek(&self) -> SelectionPeek {
        crate::focus::peek_selection()
    }
    fn select_inserted(&self, text: &str) -> SelectOutcome {
        crate::focus::select_inserted(text)
    }
    fn selected_text(&self) -> Option<String> {
        crate::focus::selected_text()
    }
    fn deselect(&self, text: &str) {
        crate::focus::deselect(text)
    }
}

/// Scripted selector for tests and `demo`. Clones share their state.
#[derive(Clone, Default)]
pub struct MockFieldSelector {
    state: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    peek: Option<SelectionPeek>,
    peek_delay: std::time::Duration,
    peeks: usize,
    /// Whether `select_inserted` finds the text.
    selectable: bool,
    selected: Vec<String>,
    /// What `selected_text` reports; `None` = unreadable.
    current: Option<String>,
    deselected: Vec<String>,
}

impl MockFieldSelector {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_peek(&self, peek: SelectionPeek) {
        self.state.lock().unwrap().peek = Some(peek);
    }
    pub fn set_peek_delay(&self, delay: std::time::Duration) {
        self.state.lock().unwrap().peek_delay = delay;
    }
    pub fn set_selectable(&self, selectable: bool) {
        self.state.lock().unwrap().selectable = selectable;
    }
    pub fn set_current(&self, current: Option<&str>) {
        self.state.lock().unwrap().current = current.map(str::to_string);
    }
    pub fn peeks(&self) -> usize {
        self.state.lock().unwrap().peeks
    }
    pub fn selected(&self) -> Vec<String> {
        self.state.lock().unwrap().selected.clone()
    }
    pub fn deselected(&self) -> Vec<String> {
        self.state.lock().unwrap().deselected.clone()
    }
}

impl FieldSelector for MockFieldSelector {
    fn peek(&self) -> SelectionPeek {
        let delay = {
            let mut s = self.state.lock().unwrap();
            s.peeks += 1;
            s.peek_delay
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let s = self.state.lock().unwrap();
        let peek = s.peek.clone().unwrap_or(SelectionPeek::Nothing);
        if let SelectionPeek::Selected { text, .. } = &peek {
            drop(s);
            self.state
                .lock()
                .unwrap()
                .current
                .get_or_insert(text.clone());
        }
        peek
    }
    fn select_inserted(&self, text: &str) -> SelectOutcome {
        let mut s = self.state.lock().unwrap();
        s.selected.push(text.to_string());
        if s.selectable {
            s.current = Some(text.trim().to_string());
            SelectOutcome::Selected(text.trim().to_string())
        } else {
            SelectOutcome::NotFound
        }
    }
    fn selected_text(&self) -> Option<String> {
        self.state.lock().unwrap().current.clone()
    }
    fn deselect(&self, text: &str) {
        self.state.lock().unwrap().deselected.push(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_copy_before_the_caret_wins() {
        let field = "Intro. OK. Middle text. OK. End.";
        assert_eq!(locate_inserted("OK.", field, Some(10)), Some((7, 10)));
        assert_eq!(
            locate_inserted("OK.", field, Some(field.len())),
            Some((24, 27))
        );
    }

    #[test]
    fn several_copies_after_the_caret_are_not_guessed() {
        assert_eq!(locate_inserted("OK.", "OK. and OK.", Some(0)), None);
        assert_eq!(
            locate_inserted("and", "OK. and OK.", Some(0)),
            Some((4, 7)),
            "a single copy is trusted wherever the caret is"
        );
        assert_eq!(locate_inserted("OK.", "OK. and OK.", None), None);
        assert_eq!(locate_inserted("gone", "OK.", Some(3)), None);
        assert_eq!(locate_inserted("  ", "OK.", Some(3)), None);
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let field = "Café 👋 See you.";
        let caret = field.chars().count();
        assert_eq!(
            locate_inserted(" See you. ", field, Some(caret)),
            Some((7, 15))
        );
    }

    #[test]
    fn line_endings_dont_matter() {
        assert!(same_text("one\r\ntwo ", "one\ntwo"));
        assert!(same_text("one\rtwo", "one\ntwo"));
        assert!(!same_text("one two", "one\ntwo"));
    }

    #[test]
    fn unreadable_selection_counts_as_still_there() {
        let mock = MockFieldSelector::new();
        assert!(is_still_selected(&mock, "words"));
        mock.set_current(Some(" words "));
        assert!(is_still_selected(&mock, "words"));
        mock.set_current(Some(""));
        assert!(!is_still_selected(&mock, "words"));
    }
}
