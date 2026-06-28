//! Wayland push-to-talk via the XDG `GlobalShortcuts` portal.
//!
//! GNOME's Wayland compositor doesn't deliver X11 key grabs to clients, so the
//! `global-hotkey` (XGrabKey) backend never fires there. The GlobalShortcuts
//! portal lets us register a shortcut the user binds once (in GNOME's dialog /
//! Settings) and then receive `Activated`/`Deactivated` (press/release) events
//! even when our window isn't focused — exactly what push-to-talk needs.

use crate::error::{CoreError, Result};
use crate::hotkey::{HotkeyCallback, HotkeyManager};
use crate::types::{HotkeyCombo, Modifier};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const SHORTCUT_ID: &str = "push-to-talk";

/// Push-to-talk backed by the GlobalShortcuts portal (GNOME/Wayland).
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

impl HotkeyManager for PortalHotkeyManager {
    fn register(
        &mut self,
        combo: HotkeyCombo,
        on_down: HotkeyCallback,
        on_up: HotkeyCallback,
    ) -> Result<()> {
        use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
        use futures_util::StreamExt;

        let stop = self.stop.clone();
        let trigger = combo_to_trigger(&combo);
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
                    let shortcut =
                        NewShortcut::new(SHORTCUT_ID, "Dictate — press to start, again to insert")
                            .preferred_trigger(Some(trigger.as_str()));
                    if let Err(e) = gs
                        .bind_shortcuts(&session, &[shortcut], None)
                        .await
                        .and_then(|r| r.response())
                    {
                        let _ =
                            ready_tx.send(Err(CoreError::Hotkey(format!("bind shortcut: {e}"))));
                        return;
                    }
                    let _ = ready_tx.send(Ok(()));

                    // GNOME reliably delivers Activated (press) but Deactivated
                    // (release) is flaky, so treat the shortcut as a TOGGLE:
                    // press to start, press again to stop + insert. Because it's
                    // a global shortcut, pressing it never steals focus from the
                    // user's target app, so the text lands there.
                    let mut active = false;
                    let mut last_activated: Option<std::time::Instant> = None;
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        tokio::select! {
                            ev = activated.next() => {
                                if matches!(&ev, Some(e) if e.shortcut_id() == SHORTCUT_ID) {
                                    let now = std::time::Instant::now();
                                    // GNOME re-emits Activated while the key is held
                                    // (and sometimes twice per press); collapse a
                                    // burst into one toggle. Sliding the window on
                                    // every event keeps a long hold as one toggle.
                                    let fresh = last_activated
                                        .is_none_or(|t| now.duration_since(t).as_millis() > 700);
                                    last_activated = Some(now);
                                    if fresh {
                                        eprintln!("orttaai: shortcut toggle (was_active={active})");
                                        if active {
                                            on_up();
                                        } else {
                                            on_down();
                                        }
                                        active = !active;
                                    }
                                }
                            }
                            ev = deactivated.next() => {
                                if matches!(&ev, Some(e) if e.shortcut_id() == SHORTCUT_ID) {
                                    eprintln!("orttaai: GlobalShortcut Deactivated");
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
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
