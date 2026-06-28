//! Native Wayland text injection via the XDG `RemoteDesktop` portal.
//!
//! GNOME's Wayland compositor implements neither the X11 key grab nor the
//! virtual-keyboard protocol that `wtype` relies on — but it DOES implement
//! `org.freedesktop.portal.RemoteDesktop`. We open one persistent session (the
//! user grants permission once; we save a restore token so later launches don't
//! re-prompt) and type each transcript as keysym press/release events.
//!
//! All portal/D-Bus work runs on a dedicated thread with its own async runtime;
//! the synchronous [`inject`](WaylandPortalInjector::inject) call hands text to
//! it over a channel and waits for the result.

use crate::error::{CoreError, Result};
use crate::settings::Settings;
use crate::types::InjectionResult;
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
use ashpd::desktop::{PersistMode, Session};

struct Job {
    text: String,
    reply: std::sync::mpsc::Sender<std::result::Result<(), String>>,
}

/// A long-lived RemoteDesktop portal session used to type into the focused app.
pub struct WaylandPortalInjector {
    tx: tokio::sync::mpsc::UnboundedSender<Job>,
    ready: std::result::Result<(), String>,
    _worker: std::thread::JoinHandle<()>,
}

impl WaylandPortalInjector {
    /// Open the portal session. Shows the permission dialog on first run; later
    /// runs reuse the saved restore token. Blocks until the user responds.
    pub fn new() -> Result<Self> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();

        let worker = std::thread::Builder::new()
            .name("orttaai-portal".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("runtime: {e}")));
                        return;
                    }
                };
                rt.block_on(async move {
                    // ---- set up the session (interactive on first run) --------
                    let rd = match RemoteDesktop::new().await {
                        Ok(rd) => rd,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("portal unavailable: {e}")));
                            return;
                        }
                    };
                    let mut session = match open_keyboard_session(&rd).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));

                    // ---- serve type requests, healing a dropped session -------
                    // GNOME closes a RemoteDesktop session after use/idle, so a
                    // later inject fails with "Invalid session". When that happens
                    // *before any key is typed*, transparently re-open with the
                    // saved restore token (no dialog) and retry once. We never
                    // retry mid-text, so a partial type can't double up in the
                    // user's app.
                    while let Some(job) = rx.recv().await {
                        let result = match type_keysyms(&rd, &session, &job.text).await {
                            Ok(()) => Ok(()),
                            Err((0, msg)) => {
                                eprintln!("orttaai: portal session unusable ({msg}); re-opening");
                                match open_keyboard_session(&rd).await {
                                    Ok(s) => {
                                        session = s;
                                        type_keysyms(&rd, &session, &job.text)
                                            .await
                                            .map_err(|(_, m)| m)
                                    }
                                    Err(e) => Err(format!("session re-open failed: {e}")),
                                }
                            }
                            Err((_, msg)) => Err(msg),
                        };
                        let _ = job.reply.send(result);
                    }
                });
            })
            .map_err(|e| CoreError::Injection(format!("spawn portal worker: {e}")))?;

        let ready = ready_rx
            .recv()
            .unwrap_or_else(|_| Err("portal worker exited during setup".into()));

        Ok(Self {
            tx,
            ready,
            _worker: worker,
        })
    }

    /// Whether the portal session was granted and is ready to type.
    pub fn is_ready(&self) -> bool {
        self.ready.is_ok()
    }

    /// Type `text` into the focused window via the portal. Blocks until done.
    pub fn inject(&self, text: &str) -> Result<InjectionResult> {
        if let Err(e) = &self.ready {
            return Err(CoreError::Injection(e.clone()));
        }
        if text.is_empty() {
            return Ok(InjectionResult::NoTranscript);
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.tx
            .send(Job {
                text: text.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| CoreError::Injection("portal worker stopped".into()))?;
        match reply_rx.recv() {
            Ok(Ok(())) => Ok(InjectionResult::Success),
            Ok(Err(e)) => Err(CoreError::Injection(e)),
            Err(_) => Err(CoreError::Injection("portal worker dropped".into())),
        }
    }
}

/// Open (or re-open) a keyboard-only RemoteDesktop session. The saved restore
/// token lets GNOME restore the grant without showing the permission dialog, so
/// this is safe to call again whenever the compositor drops the session.
async fn open_keyboard_session<'a>(
    rd: &RemoteDesktop<'a>,
) -> std::result::Result<Session<'a, RemoteDesktop<'a>>, String> {
    let session = rd
        .create_session()
        .await
        .map_err(|e| format!("create session: {e}"))?;
    let saved = Settings::load_or_default().wayland_restore_token;
    rd.select_devices(
        &session,
        DeviceType::Keyboard.into(),
        saved.as_deref(),
        PersistMode::ExplicitlyRevoked,
    )
    .await
    .map_err(|e| format!("select devices: {e}"))?;
    let response = rd
        .start(&session, None)
        .await
        .map_err(|e| format!("start: {e}"))?
        .response()
        .map_err(|e| format!("permission denied/dismissed: {e}"))?;
    if !response.devices().contains(DeviceType::Keyboard) {
        return Err("keyboard control was not granted".to_string());
    }
    if let Some(token) = response.restore_token() {
        save_restore_token(token);
    }
    Ok(session)
}

/// Type `text` as keysym press/release pairs. On error, returns how many chars
/// were already typed so the caller can avoid retrying mid-text (which would
/// double-type). A failure at index 0 means nothing was typed — safe to retry.
async fn type_keysyms<'a>(
    rd: &RemoteDesktop<'a>,
    session: &Session<'a, RemoteDesktop<'a>>,
    text: &str,
) -> std::result::Result<(), (usize, String)> {
    for (i, ch) in text.chars().enumerate() {
        let keysym = char_to_keysym(ch);
        rd.notify_keyboard_keysym(session, keysym, KeyState::Pressed)
            .await
            .map_err(|e| (i, e.to_string()))?;
        rd.notify_keyboard_keysym(session, keysym, KeyState::Released)
            .await
            .map_err(|e| (i, e.to_string()))?;
        // A small gap helps the compositor register each key.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    Ok(())
}

/// Map a character to an X11 keysym for the portal.
fn char_to_keysym(ch: char) -> i32 {
    match ch {
        '\n' | '\r' => 0xFF0D, // Return
        '\t' => 0xFF09,        // Tab
        c if ('\u{20}'..='\u{7e}').contains(&c) => c as i32, // ASCII == Latin-1 keysym
        c => 0x0100_0000 + c as i32, // Unicode keysym range
    }
}

/// Persist the restore token so the next launch doesn't re-prompt for permission.
fn save_restore_token(token: &str) {
    let mut settings = Settings::load_or_default();
    if settings.wayland_restore_token.as_deref() != Some(token) {
        settings.wayland_restore_token = Some(token.to_string());
        let _ = settings.save();
    }
}
