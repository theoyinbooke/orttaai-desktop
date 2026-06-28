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
use ashpd::desktop::PersistMode;

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
                    let session = match rd.create_session().await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("create session: {e}")));
                            return;
                        }
                    };
                    let saved = Settings::load_or_default().wayland_restore_token;
                    if let Err(e) = rd
                        .select_devices(
                            &session,
                            DeviceType::Keyboard.into(),
                            saved.as_deref(),
                            PersistMode::ExplicitlyRevoked,
                        )
                        .await
                    {
                        let _ = ready_tx.send(Err(format!("select devices: {e}")));
                        return;
                    }
                    let response = match rd.start(&session, None).await {
                        Ok(req) => match req.response() {
                            Ok(r) => r,
                            Err(e) => {
                                let _ =
                                    ready_tx.send(Err(format!("permission denied/dismissed: {e}")));
                                return;
                            }
                        },
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("start: {e}")));
                            return;
                        }
                    };
                    if !response.devices().contains(DeviceType::Keyboard) {
                        let _ = ready_tx.send(Err("keyboard control was not granted".into()));
                        return;
                    }
                    if let Some(token) = response.restore_token() {
                        save_restore_token(token);
                    }
                    let _ = ready_tx.send(Ok(()));

                    // ---- serve type requests for the session's lifetime -------
                    while let Some(job) = rx.recv().await {
                        let mut result = Ok(());
                        for ch in job.text.chars() {
                            let keysym = char_to_keysym(ch);
                            if let Err(e) = rd
                                .notify_keyboard_keysym(&session, keysym, KeyState::Pressed)
                                .await
                            {
                                result = Err(e.to_string());
                                break;
                            }
                            if let Err(e) = rd
                                .notify_keyboard_keysym(&session, keysym, KeyState::Released)
                                .await
                            {
                                result = Err(e.to_string());
                                break;
                            }
                            // A small gap helps the compositor register each key.
                            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                        }
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
