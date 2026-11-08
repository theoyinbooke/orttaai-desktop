//! Learn-from-edits watcher (Mac `EditLearningWatcher`).
//!
//! Right after a dictation is inserted it reads the focused field (again for up
//! to ~1.5 s if the inserted text hasn't shown up yet), then re-reads it over
//! the next several minutes, and once more when the next dictation starts. A
//! field that empties — a chat box after the message is sent — or a move to
//! another field ends the watch with the text it held just before. Each watch
//! reports exactly once, through the observer callback the integrator wires to
//! the text pipeline's correction finder: the field text before and after the
//! user's fixes, never stored by this module. Secure fields are never read.
//!
//! Field reading is OS-specific ([`crate::focus::focused_field_read`]: UI
//! Automation on Windows, AT-SPI on Linux).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Timed reads, cumulative ~6.8 minutes (Mac `readDelays`).
pub const READ_DELAYS: [Duration; 6] = [
    Duration::from_secs(4),
    Duration::from_secs(12),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(180),
];
/// Re-reads while waiting for the inserted text to appear in the field.
pub const BASELINE_RETRIES: u32 = 10;
pub const BASELINE_RETRY_DELAY: Duration = Duration::from_millis(150);

/// One read of the focused field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldRead {
    Text(String),
    /// A password field: never read.
    Secure,
    /// The app (or OS) doesn't expose the field.
    Unreadable,
}

pub trait FocusedFieldReader: Send + Sync {
    fn read(&self) -> FieldRead;
}

/// The OS reader: [`crate::focus::focused_field_text`].
pub struct SystemFieldReader;

impl FocusedFieldReader for SystemFieldReader {
    fn read(&self) -> FieldRead {
        crate::focus::focused_field_read()
    }
}

/// How one watch ended (Mac `EditCheckOutcome`, minus the text-pipeline ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditCheckOutcome {
    /// The field changed after the dictation was inserted.
    Edited,
    /// The dictation was left as it was.
    NoChange,
    /// The field could be read, but the dictation wasn't in it.
    TextNotFound,
    /// The app didn't let Orttaai read the field.
    Unreadable,
    /// A password field: never read.
    SecureField,
}

/// What the observer receives once per watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditObservation {
    pub outcome: EditCheckOutcome,
    /// The dictated text that was inserted.
    pub inserted: String,
    /// Field text right after insertion (when readable).
    pub before: Option<String>,
    /// Field text at the end of the watch (when readable).
    pub after: Option<String>,
    pub app: Option<String>,
}

pub type EditObserver = Arc<dyn Fn(EditObservation) + Send + Sync>;

struct Watch {
    inserted: String,
    baseline: String,
    latest: String,
    app: Option<String>,
}

impl Watch {
    fn observation(self) -> EditObservation {
        EditObservation {
            outcome: if self.latest != self.baseline {
                EditCheckOutcome::Edited
            } else {
                EditCheckOutcome::NoChange
            },
            inserted: self.inserted,
            before: Some(self.baseline),
            after: Some(self.latest),
            app: self.app,
        }
    }
}

/// Word-set overlap: a re-read that shares less than half its words with the
/// watched text is another field (the user moved on).
fn same_field(latest: &str, now: &str) -> bool {
    let words = |s: &str| {
        s.split_whitespace()
            .map(|w| w.to_lowercase())
            .collect::<std::collections::HashSet<_>>()
    };
    let a = words(latest);
    let b = words(now);
    if a.is_empty() || b.is_empty() {
        return a.is_empty() && b.is_empty();
    }
    let shared = a.intersection(&b).count();
    shared * 2 >= a.len().min(b.len())
}

/// Watches each inserted dictation for the user's fixes.
pub struct EditWatcher {
    reader: Arc<dyn FocusedFieldReader>,
    observer: EditObserver,
    generation: Arc<AtomicU64>,
    current: Arc<Mutex<Option<Watch>>>,
    sleep: Arc<dyn Fn(Duration) + Send + Sync>,
    delays: Vec<Duration>,
}

impl EditWatcher {
    pub fn new(reader: Arc<dyn FocusedFieldReader>, observer: EditObserver) -> Self {
        Self {
            reader,
            observer,
            generation: Arc::new(AtomicU64::new(0)),
            current: Arc::new(Mutex::new(None)),
            sleep: Arc::new(std::thread::sleep),
            delays: READ_DELAYS.to_vec(),
        }
    }

    /// Test seam: replace sleeps and the read schedule.
    pub fn with_timing(
        mut self,
        sleep: impl Fn(Duration) + Send + Sync + 'static,
        delays: Vec<Duration>,
    ) -> Self {
        self.sleep = Arc::new(sleep);
        self.delays = delays;
        self
    }

    /// The next dictation is starting: read the watched field one last time,
    /// report, and stop watching (its insertion would mix into the field).
    pub fn finish_current(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let taken = self.current.lock().unwrap().take();
        if let Some(mut watch) = taken {
            if let FieldRead::Text(t) = self.reader.read() {
                if !t.is_empty() && same_field(&watch.latest, &t) {
                    watch.latest = t;
                }
            }
            (self.observer)(watch.observation());
        }
    }

    /// Start watching `inserted`, just delivered into `app`. Runs on its own
    /// thread; returns immediately.
    pub fn watch(&self, inserted: &str, app: Option<String>) {
        self.finish_current();
        let generation = self.generation.load(Ordering::SeqCst);
        let gen = self.generation.clone();
        let current = self.current.clone();
        let reader = self.reader.clone();
        let observer = self.observer.clone();
        let sleep = self.sleep.clone();
        let delays = self.delays.clone();
        let inserted = inserted.to_string();
        let alive = move || gen.load(Ordering::SeqCst) == generation;
        let spawned = std::thread::Builder::new()
            .name("orttaai-edit-watch".into())
            .spawn(move || {
                let report = |o: EditObservation| (observer)(o);
                // Baseline: wait for the inserted text to show up.
                let mut last = FieldRead::Unreadable;
                let mut baseline = None;
                for attempt in 0..BASELINE_RETRIES {
                    if attempt > 0 {
                        sleep(BASELINE_RETRY_DELAY);
                    }
                    if !alive() {
                        return;
                    }
                    last = reader.read();
                    match &last {
                        FieldRead::Text(t) if t.contains(inserted.trim()) => {
                            baseline = Some(t.clone());
                            break;
                        }
                        FieldRead::Secure => break,
                        _ => {}
                    }
                }
                let Some(baseline) = baseline else {
                    let outcome = match last {
                        FieldRead::Secure => EditCheckOutcome::SecureField,
                        FieldRead::Text(_) => EditCheckOutcome::TextNotFound,
                        FieldRead::Unreadable => EditCheckOutcome::Unreadable,
                    };
                    if alive() {
                        report(EditObservation {
                            outcome,
                            inserted,
                            before: None,
                            after: None,
                            app,
                        });
                    }
                    return;
                };
                *current.lock().unwrap() = Some(Watch {
                    inserted,
                    latest: baseline.clone(),
                    baseline,
                    app,
                });
                for delay in delays {
                    sleep(delay);
                    if !alive() {
                        return; // finish_current reported it
                    }
                    let mut guard = current.lock().unwrap();
                    let Some(watch) = guard.as_mut() else { return };
                    let ended = match reader.read() {
                        // A field that empties (message sent) ends the watch
                        // with the text it held just before.
                        FieldRead::Text(t) if t.trim().is_empty() => true,
                        FieldRead::Text(t) if same_field(&watch.latest, &t) => {
                            watch.latest = t;
                            false
                        }
                        // Another field, a password field, or unreadable.
                        _ => true,
                    };
                    if ended {
                        let watch = guard.take().expect("watch present");
                        drop(guard);
                        report(watch.observation());
                        return;
                    }
                }
                let taken = current.lock().unwrap().take();
                if let Some(watch) = taken {
                    if alive() {
                        report(watch.observation());
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the edit watcher: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Script(Mutex<VecDeque<FieldRead>>);
    impl FocusedFieldReader for Script {
        fn read(&self) -> FieldRead {
            let mut q = self.0.lock().unwrap();
            if q.len() > 1 {
                q.pop_front().unwrap()
            } else {
                q.front().cloned().unwrap_or(FieldRead::Unreadable)
            }
        }
    }

    fn text(s: &str) -> FieldRead {
        FieldRead::Text(s.to_string())
    }

    fn run(reads: Vec<FieldRead>, delays: usize) -> Vec<EditObservation> {
        let seen: Arc<Mutex<Vec<EditObservation>>> = Arc::default();
        let sink = seen.clone();
        let w = EditWatcher::new(
            Arc::new(Script(Mutex::new(reads.into()))),
            Arc::new(move |o| sink.lock().unwrap().push(o)),
        )
        .with_timing(|_| {}, vec![Duration::ZERO; delays]);
        w.watch("meet with jon", Some("Notepad".into()));
        for _ in 0..200 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let out = seen.lock().unwrap().clone();
        out
    }

    #[test]
    fn reports_an_edit_with_before_and_after() {
        let obs = run(
            vec![
                text("Notes: meet with jon"),
                text("Notes: meet with Jon"),
                text("Notes: meet with Jon tomorrow"),
            ],
            2,
        );
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].outcome, EditCheckOutcome::Edited);
        assert_eq!(obs[0].before.as_deref(), Some("Notes: meet with jon"));
        assert_eq!(
            obs[0].after.as_deref(),
            Some("Notes: meet with Jon tomorrow")
        );
    }

    #[test]
    fn untouched_dictation_reports_no_change() {
        let obs = run(vec![text("meet with jon")], 3);
        assert_eq!(obs[0].outcome, EditCheckOutcome::NoChange);
    }

    #[test]
    fn emptied_field_ends_with_the_text_just_before() {
        let obs = run(
            vec![text("meet with jon"), text("meet with Jon"), text("")],
            5,
        );
        assert_eq!(obs[0].outcome, EditCheckOutcome::Edited);
        assert_eq!(obs[0].after.as_deref(), Some("meet with Jon"));
    }

    #[test]
    fn moving_to_another_field_ends_the_watch() {
        let obs = run(
            vec![
                text("meet with jon"),
                text("completely different search box query"),
            ],
            3,
        );
        assert_eq!(obs[0].outcome, EditCheckOutcome::NoChange);
        assert_eq!(obs[0].after.as_deref(), Some("meet with jon"));
    }

    #[test]
    fn unreadable_secure_and_missing_text() {
        assert_eq!(
            run(vec![FieldRead::Unreadable], 1)[0].outcome,
            EditCheckOutcome::Unreadable
        );
        assert_eq!(
            run(vec![FieldRead::Secure], 1)[0].outcome,
            EditCheckOutcome::SecureField
        );
        assert_eq!(
            run(vec![text("something else")], 1)[0].outcome,
            EditCheckOutcome::TextNotFound
        );
    }

    #[test]
    fn finish_current_reports_once() {
        let seen: Arc<Mutex<Vec<EditObservation>>> = Arc::default();
        let sink = seen.clone();
        let w = EditWatcher::new(
            Arc::new(Script(Mutex::new(
                vec![text("meet with jon"), text("meet with Jon")].into(),
            ))),
            Arc::new(move |o| sink.lock().unwrap().push(o)),
        )
        .with_timing(
            |d| std::thread::sleep(d.min(Duration::from_millis(1))),
            vec![Duration::from_secs(60)],
        );
        w.watch("meet with jon", None);
        // Let the baseline land, then the next dictation starts.
        for _ in 0..200 {
            if w.current.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        w.finish_current();
        std::thread::sleep(Duration::from_millis(30));
        let obs = seen.lock().unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].after.as_deref(), Some("meet with Jon"));
    }
}
