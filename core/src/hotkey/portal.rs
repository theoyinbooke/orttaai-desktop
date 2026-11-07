//! Wayland hotkeys via the XDG `GlobalShortcuts` portal.
//!
//! GNOME's Wayland compositor doesn't deliver X11 key grabs to clients, so the
//! `global-hotkey` (XGrabKey) backend never fires there. The GlobalShortcuts
//! portal lets us register shortcuts the user binds once (in GNOME's dialog /
//! Settings) and then receive `Activated`/`Deactivated` (press/release) events
//! even when our window isn't focused — exactly what push-to-talk needs. Both
//! of the app's shortcuts (push-to-talk and edit-selection) are bound in one
//! session.

use crate::error::{CoreError, Result};
use crate::hotkey::{HotkeyBinding, HotkeyManager, PressFilter};
use crate::types::{HotkeyCombo, Modifier};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Once autorepeat is seen, a gap this long with no further `Activated` means
/// the key was released (covers a missed `Deactivated`).
const WATCHDOG_GAP: Duration = Duration::from_millis(1200);
/// Stop a hold after this long regardless (a dropped release can never strand
/// recording on; the coordinator's own caps are shorter).
const MAX_HOLD: Duration = Duration::from_secs(330);

/// Hotkeys backed by the GlobalShortcuts portal (GNOME/KDE Wayland).
pub struct PortalHotkeyManager {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PortalHotkeyManager {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl Default for PortalHotkeyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-shortcut hold tracking.
struct Held {
    binding: HotkeyBinding,
    filter: PressFilter,
    started_at: Option<Instant>,
    last_activated: Option<Instant>,
    repeats: u32,
}

impl Held {
    fn activated(&mut self, now: Instant) {
        self.last_activated = Some(now);
        if self.filter.down() {
            self.started_at = Some(now);
            self.repeats = 0;
            (self.binding.on_down)();
        } else {
            self.repeats += 1; // autorepeat while held — ignore
        }
    }

    fn deactivated(&mut self) {
        if self.filter.up() {
            self.started_at = None;
            (self.binding.on_up)();
        }
    }

    fn tick(&mut self, now: Instant) {
        let Some(start) = self.started_at else { return };
        let gap = self.last_activated.map(|t| now.duration_since(t));
        if now.duration_since(start) > MAX_HOLD
            || (self.repeats >= 1 && gap.is_some_and(|g| g > WATCHDOG_GAP))
        {
            tracing::info!("portal hotkey {}: release inferred", self.binding.id);
            self.deactivated();
        }
    }
}

impl HotkeyManager for PortalHotkeyManager {
    fn register_all(&mut self, bindings: Vec<HotkeyBinding>) -> Result<()> {
        use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
        use futures_util::StreamExt;

        self.unregister()?;
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

        let handle = std::thread::Builder::new()
            .name("orttaai-portal-hotkey".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(CoreError::Hotkey(format!("runtime: {e}"))));
                        return;
                    }
                };
                rt.block_on(async move {
                    let gs = match GlobalShortcuts::new().await {
                        Ok(g) => g,
                        Err(e) => {
                            let _ = ready_tx
                                .send(Err(CoreError::Hotkey(format!("portal unavailable: {e}"))));
                            return;
                        }
                    };
                    let session = match gs.create_session().await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ready_tx
                                .send(Err(CoreError::Hotkey(format!("create session: {e}"))));
                            return;
                        }
                    };
                    // Subscribe before binding so no early events are missed.
                    let mut activated = match gs.receive_activated().await {
                        Ok(s) => Box::pin(s),
                        Err(e) => {
                            let _ =
                                ready_tx.send(Err(CoreError::Hotkey(format!("activated: {e}"))));
                            return;
                        }
                    };
                    let mut deactivated = match gs.receive_deactivated().await {
                        Ok(s) => Box::pin(s),
                        Err(e) => {
                            let _ =
                                ready_tx.send(Err(CoreError::Hotkey(format!("deactivated: {e}"))));
                            return;
                        }
                    };
                    let triggers: Vec<String> =
                        bindings.iter().map(|b| combo_to_trigger(&b.combo)).collect();
                    let shortcuts: Vec<NewShortcut> = bindings
                        .iter()
                        .zip(&triggers)
                        .map(|(b, t)| {
                            NewShortcut::new(b.id, b.description).preferred_trigger(Some(t.as_str()))
                        })
                        .collect();
                    match gs
                        .bind_shortcuts(&session, &shortcuts, None)
                        .await
                        .and_then(|r| r.response())
                    {
                        Ok(resp) => {
                            // GNOME often ignores `preferred_trigger` and binds no
                            // usable trigger (the user assigns the key in
                            // Settings → Keyboard). Log what it bound so a
                            // non-firing shortcut can be diagnosed.
                            if resp.shortcuts().is_empty() {
                                tracing::warn!("GlobalShortcuts bind returned no shortcuts");
                            }
                            for sc in resp.shortcuts() {
                                tracing::info!(
                                    "bound shortcut id={:?} trigger={:?}",
                                    sc.id(),
                                    sc.trigger_description()
                                );
                            }
                        }
                        Err(e) => {
                            let _ = ready_tx
                                .send(Err(CoreError::Hotkey(format!("bind shortcut: {e}"))));
                            return;
                        }
                    }
                    let _ = ready_tx.send(Ok(()));

                    // Hold-to-talk. The portal emits one `Activated` on press,
                    // REPEATS it at the key auto-repeat rate while held, then one
                    // `Deactivated` on release. The first Activated is the press;
                    // repeats are ignored. Safety nets (watchdog + hard cap)
                    // guarantee a dropped release can't strand a key "down".
                    let mut held: Vec<Held> = bindings
                        .into_iter()
                        .map(|binding| Held {
                            binding,
                            filter: PressFilter::default(),
                            started_at: None,
                            last_activated: None,
                            repeats: 0,
                        })
                        .collect();
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        tokio::select! {
                            ev = activated.next() => {
                                if let Some(e) = ev {
                                    let now = Instant::now();
                                    if let Some(h) = held.iter_mut().find(|h| h.binding.id == e.shortcut_id()) {
                                        h.activated(now);
                                    }
                                }
                            }
                            ev = deactivated.next() => {
                                if let Some(e) = ev {
                                    if let Some(h) = held.iter_mut().find(|h| h.binding.id == e.shortcut_id()) {
                                        h.deactivated();
                                    }
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                                let now = Instant::now();
                                for h in held.iter_mut() {
                                    h.tick(now);
                                }
                            }
                        }
                    }
                });
            })
            .map_err(|e| CoreError::Hotkey(format!("spawn portal hotkey thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.thread = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CoreError::Hotkey(
                "portal hotkey thread exited before binding".to_string(),
            )),
        }
    }

    fn unregister(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "GlobalShortcuts portal (Wayland)"
    }
}

impl Drop for PortalHotkeyManager {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

/// Build an XDG accelerator string (e.g. `CTRL+SHIFT+space`) from our combo, as
/// a preferred default for the bind dialog. GNOME may let the user override it.
fn combo_to_trigger(combo: &HotkeyCombo) -> String {
    let mut parts: Vec<String> = combo
        .modifiers
        .iter()
        .map(|m| {
            match m {
                Modifier::Ctrl => "CTRL",
                Modifier::Shift => "SHIFT",
                Modifier::Alt => "ALT",
                Modifier::Meta => "LOGO",
            }
            .to_string()
        })
        .collect();
    parts.push(combo.key.to_lowercase());
    parts.join("+")
}
