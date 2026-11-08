//! Native Wayland text injection via the XDG `RemoteDesktop` portal.
//!
//! GNOME's Wayland compositor implements neither the X11 key grab nor the
//! virtual-keyboard protocol that `wtype` relies on — but it DOES implement
//! `org.freedesktop.portal.RemoteDesktop`. We open one persistent session (the
//! user grants permission once; we save a restore token so later launches don't
//! re-prompt) and type each transcript as keysym press/release events. The
//! same session sends the paste/copy chords for paste injection and voice-edit
//! selection capture.
//!
//! All portal/D-Bus work runs on a dedicated thread with its own async runtime;
//! the synchronous calls hand work to it over a channel and wait for the result.

use crate::error::{CoreError, Result};
use crate::settings::Settings;
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
use ashpd::desktop::{PersistMode, Session};

pub const CONTROL_L: i32 = 0xFFE3;
pub const SHIFT_L: i32 = 0xFFE1;
pub const INSERT: i32 = 0xFF63;
pub const KEY_V: i32 = 0x76;
pub const KEY_C: i32 = 0x63;

enum Keys {
    Text(String),
    Chord { modifiers: Vec<i32>, key: i32 },
}

struct Job {
    keys: Keys,
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

                    // GNOME closes a RemoteDesktop session after use/idle, so a
                    // later request fails with "Invalid session". When that
                    // happens *before any key is sent*, transparently re-open
                    // with the saved restore token (no dialog) and retry once.
                    // Never retried mid-text, so a partial type can't double up.
                    while let Some(job) = rx.recv().await {
                        let result = match send_keys(&rd, &session, &job.keys).await {
                            Ok(()) => Ok(()),
                            Err((0, msg)) => {
                                tracing::warn!("portal session unusable ({msg}); re-opening");
                                match open_keyboard_session(&rd).await {
                                    Ok(s) => {
                                        session = s;
                                        send_keys(&rd, &session, &job.keys)
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

    fn run(&self, keys: Keys) -> Result<()> {
        if let Err(e) = &self.ready {
            return Err(CoreError::Injection(e.clone()));
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.tx
            .send(Job {
                keys,
                reply: reply_tx,
            })
            .map_err(|_| CoreError::Injection("portal worker stopped".into()))?;
        match reply_rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(CoreError::Injection(e)),
            Err(_) => Err(CoreError::Injection("portal worker dropped".into())),
        }
    }

    /// Type `text` into the focused window. Blocks until done.
    pub fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.run(Keys::Text(text.to_string()))
    }

    /// Press `modifiers`, click `key`, release the modifiers (keysyms).
    pub fn chord(&self, modifiers: &[i32], key: i32) -> Result<()> {
        self.run(Keys::Chord {
            modifiers: modifiers.to_vec(),
            key,
        })
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

async fn key<'a>(
    rd: &RemoteDesktop<'a>,
    session: &Session<'a, RemoteDesktop<'a>>,
    keysym: i32,
    state: KeyState,
) -> std::result::Result<(), String> {
    rd.notify_keyboard_keysym(session, keysym, state)
        .await
        .map_err(|e| e.to_string())
}

/// Send `keys`. On error, returns how many keys were already sent so the caller
/// can avoid retrying mid-text (which would double-type). A failure at index 0
/// means nothing was sent — safe to retry.
async fn send_keys<'a>(
    rd: &RemoteDesktop<'a>,
    session: &Session<'a, RemoteDesktop<'a>>,
    keys: &Keys,
) -> std::result::Result<(), (usize, String)> {
    match keys {
        Keys::Text(text) => {
            for (i, ch) in text.chars().enumerate() {
                let keysym = char_to_keysym(ch);
                key(rd, session, keysym, KeyState::Pressed)
                    .await
                    .map_err(|e| (i, e))?;
                key(rd, session, keysym, KeyState::Released)
                    .await
                    .map_err(|e| (i, e))?;
                // A small gap helps the compositor register each key.
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            Ok(())
        }
        Keys::Chord { modifiers, key: k } => {
            for &m in modifiers {
                key(rd, session, m, KeyState::Pressed)
                    .await
                    .map_err(|e| (0, e))?;
            }
            let sent = async {
                key(rd, session, *k, KeyState::Pressed).await?;
                tokio::time::sleep(std::time::Duration::from_millis(4)).await;
                key(rd, session, *k, KeyState::Released).await
            }
            .await;
            // Always release the modifiers, even if the key failed.
            for &m in modifiers.iter().rev() {
                let _ = key(rd, session, m, KeyState::Released).await;
            }
            sent.map_err(|e| (1, e))
        }
    }
}

/// Map a character to an X11 keysym for the portal.
fn char_to_keysym(ch: char) -> i32 {
    match ch {
        '\n' | '\r' => 0xFF0D,                               // Return
        '\t' => 0xFF09,                                      // Tab
        c if ('\u{20}'..='\u{7e}').contains(&c) => c as i32, // ASCII == Latin-1 keysym
        c => 0x0100_0000 + c as i32,                         // Unicode keysym range
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
