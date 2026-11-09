//! The sync engine (`docs/orttaai-link.md` §6, §9): one thread sends local
//! changes a moment after they happen, another holds a request open on the
//! Mac so its changes arrive within moments. When the Mac can't be reached,
//! changes wait in the outbox and go on the next connection.

use super::client::{ClientError, LinkClient};
use super::discovery::DiscoveredMac;
use super::model::{DeviceKind, Envelope, RecordKind};
use super::store::{now_ms, ApplyResult, DeviceInfo, LinkedMac, SyncCounts};
use super::wire::*;
use crate::store::Store;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// What the Devices page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub linked: bool,
    pub mac_name: Option<String>,
    /// `idle` (up to date), `syncing`, `offline`, `unlinked`, `updateMac`, `error`.
    pub state: String,
    pub message: Option<String>,
    /// The last time the Mac answered, in ms.
    pub last_sync_ms: Option<i64>,
    pub via_tailscale: bool,
    pub host: Option<String>,
    pub counts: SyncCounts,
    pub icloud: Option<ICloudStatus>,
    pub device_id: Option<String>,
    pub tailscale_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("{0}")]
    Client(#[from] ClientError),
    #[error("{0}")]
    Store(#[from] crate::error::CoreError),
    #[error("that link isn't from Orttaai on a Mac")]
    BadInvite,
}

/// Links this PC to a Mac with the 6-digit code shown there.
pub fn link_with_code(
    store: &Store,
    mac: &DiscoveredMac,
    code: &str,
    device: &DeviceInfo,
) -> Result<LinkedMac, LinkError> {
    let invite = LinkInvite {
        mac_device_id: mac.mac_device_id.clone(),
        name: mac.name.clone(),
        hosts: mac.hosts.clone(),
        port: mac.port,
        fingerprint: mac.fingerprint.clone(),
        tailscale_name: None,
        secret: String::new(),
    };
    link(
        store,
        &invite,
        None,
        Some(code.chars().filter(char::is_ascii_digit).collect()),
        device,
    )
}

/// Links this PC with the link copied on the Mac (works away from home).
pub fn link_with_invite(
    store: &Store,
    text: &str,
    device: &DeviceInfo,
) -> Result<LinkedMac, LinkError> {
    let invite = LinkInvite::parse(text).ok_or(LinkError::BadInvite)?;
    let secret = invite.secret.clone();
    link(store, &invite, Some(secret), None, device)
}

fn link(
    store: &Store,
    invite: &LinkInvite,
    secret: Option<String>,
    code: Option<String>,
    device: &DeviceInfo,
) -> Result<LinkedMac, LinkError> {
    let device_id = store.sync_device_id()?;
    let client = LinkClient::new(&invite.fingerprint, None, &device.app_version);
    let ask = PairRequest {
        secret,
        code,
        device: LinkDeviceInfo {
            device_id,
            kind: DeviceKind::this_platform(),
            name: device.name.clone(),
            model: device.model.clone(),
            app: "Orttaai".into(),
            app_version: Some(device.app_version.clone()),
            os_version: Some(device.os_version.clone()),
        },
    };
    let mut hosts = invite.hosts.clone();
    if let Some(ts) = &invite.tailscale_name {
        hosts.push(ts.clone());
    }
    let mut last_error = None;
    for host in &hosts {
        match client.post::<_, PairResponse>(
            host,
            invite.port,
            "/link/v1/pair",
            &ask,
            Duration::from_secs(10),
        ) {
            Ok(answer) => {
                let mac = LinkedMac {
                    mac_device_id: answer.mac_device_id,
                    name: answer.name,
                    fingerprint: invite.fingerprint.clone(),
                    token: answer.token,
                    hosts: invite.hosts.clone(),
                    port: invite.port,
                    tailscale_name: answer
                        .tailscale_name
                        .or_else(|| invite.tailscale_name.clone()),
                    last_host: Some(host.clone()),
                };
                store.sync_link(&mac, device)?;
                return Ok(mac);
            }
            Err(e) if e.is_unreachable() => last_error = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    Err(last_error
        .unwrap_or_else(|| ClientError::Unreachable("the Mac".into(), "no address to try".into()))
        .into())
}

struct Shared {
    status: Mutex<LinkStatus>,
    wake: Condvar,
    wake_flag: Mutex<bool>,
    stop: AtomicBool,
    /// Bumped on link and unlink so loops pick up the new Mac.
    generation: Mutex<u64>,
}

/// Runs sync in the background while this PC is linked.
#[derive(Clone)]
pub struct LinkEngine {
    db_path: PathBuf,
    device: DeviceInfo,
    shared: Arc<Shared>,
    on_change: Arc<dyn Fn(&ApplyResult) + Send + Sync>,
    on_status: Arc<dyn Fn(&LinkStatus) + Send + Sync>,
}

impl LinkEngine {
    /// `on_change` runs after records from the Mac changed something here;
    /// `on_status` whenever the status changes.
    pub fn start(
        db_path: PathBuf,
        device: DeviceInfo,
        on_change: impl Fn(&ApplyResult) + Send + Sync + 'static,
        on_status: impl Fn(&LinkStatus) + Send + Sync + 'static,
    ) -> Self {
        let engine = Self {
            db_path,
            device,
            shared: Arc::new(Shared {
                status: Mutex::new(LinkStatus {
                    state: "idle".into(),
                    ..Default::default()
                }),
                wake: Condvar::new(),
                wake_flag: Mutex::new(false),
                stop: AtomicBool::new(false),
                generation: Mutex::new(0),
            }),
            on_change: Arc::new(on_change),
            on_status: Arc::new(on_status),
        };
        let sender = engine.clone();
        std::thread::Builder::new()
            .name("orttaai-link-send".into())
            .spawn(move || sender.send_loop())
            .ok();
        let receiver = engine.clone();
        std::thread::Builder::new()
            .name("orttaai-link-receive".into())
            .spawn(move || receiver.receive_loop())
            .ok();
        engine.refresh_status();
        engine
    }

    pub fn status(&self) -> LinkStatus {
        self.shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Something changed here (or the user asked to sync now): send soon.
    pub fn nudge(&self) {
        if let Ok(mut flag) = self.shared.wake_flag.lock() {
            *flag = true;
        }
        self.shared.wake.notify_all();
    }

    /// The link changed (linked or unlinked): start over with it.
    pub fn relinked(&self) {
        if let Ok(mut g) = self.shared.generation.lock() {
            *g += 1;
        }
        self.refresh_status();
        self.nudge();
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.nudge();
    }

    fn generation(&self) -> u64 {
        self.shared.generation.lock().map(|g| *g).unwrap_or(0)
    }

    fn open(&self) -> Option<Store> {
        Store::open(&self.db_path).ok()
    }

    fn update(&self, change: impl FnOnce(&mut LinkStatus)) {
        let snapshot = {
            let Ok(mut status) = self.shared.status.lock() else {
                return;
            };
            let before = status.clone();
            change(&mut status);
            if *status == before {
                return;
            }
            status.clone()
        };
        (self.on_status)(&snapshot);
    }

    /// Rereads the link and counts from the database.
    pub fn refresh_status(&self) {
        let Some(store) = self.open() else { return };
        let mac = store.sync_linked_mac().ok().flatten();
        let counts = store.sync_counts().unwrap_or_default();
        let device_id = store.sync_get("device_id").ok().flatten();
        self.update(|s| {
            s.linked = mac.is_some();
            s.mac_name = mac.as_ref().map(|m| m.name.clone());
            s.tailscale_name = mac.as_ref().and_then(|m| m.tailscale_name.clone());
            s.counts = counts;
            s.device_id = device_id;
            if mac.is_none() {
                s.state = "idle".into();
                s.message = None;
                s.host = None;
                s.icloud = None;
            }
        });
    }

    /// Waits up to `timeout` for a nudge (or a stop).
    fn sleep(&self, timeout: Duration) {
        let Ok(flag) = self.shared.wake_flag.lock() else {
            return;
        };
        let (mut flag, _) = self
            .shared
            .wake
            .wait_timeout_while(flag, timeout, |woken| {
                !*woken && !self.shared.stop.load(Ordering::SeqCst)
            })
            .unwrap_or_else(|e| e.into_inner());
        *flag = false;
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    /// The addresses to try, best first.
    fn hosts(mac: &LinkedMac) -> Vec<String> {
        let mut hosts = Vec::new();
        if let Some(last) = &mac.last_host {
            hosts.push(last.clone());
        }
        for h in mac.hosts.iter().chain(mac.tailscale_name.iter()) {
            if !hosts.contains(h) {
                hosts.push(h.clone());
            }
        }
        hosts
    }

    /// Runs `call` against the first address that answers, remembering it.
    fn with_mac<T>(
        &self,
        store: &Store,
        mac: &mut LinkedMac,
        call: impl Fn(&LinkClient, &str) -> Result<T, ClientError>,
    ) -> Result<T, ClientError> {
        let client = LinkClient::new(
            &mac.fingerprint,
            Some(mac.token.clone()),
            &self.device.app_version,
        );
        let mut last = None;
        for host in Self::hosts(mac) {
            match call(&client, &host) {
                Ok(value) => {
                    if mac.last_host.as_deref() != Some(host.as_str()) {
                        mac.last_host = Some(host.clone());
                        let _ = store.sync_save_linked_mac(mac);
                    }
                    let via_tailscale = mac.tailscale_name.as_deref() == Some(host.as_str())
                        || host.starts_with("100.");
                    self.update(|s| {
                        s.host = Some(host.clone());
                        s.via_tailscale = via_tailscale;
                        s.last_sync_ms = Some(now_ms());
                    });
                    return Ok(value);
                }
                Err(e) if e.is_unreachable() => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| ClientError::Unreachable("the Mac".into(), "no address".into())))
    }

    fn note_error(&self, error: &ClientError) {
        let (state, message) = match error {
            ClientError::Unreachable(..) => (
                "offline",
                "Can't reach the Mac right now. Changes wait here and sync when it's back."
                    .to_string(),
            ),
            ClientError::Unlinked => (
                "unlinked",
                "This PC was removed on the Mac. Link it again to keep syncing.".to_string(),
            ),
            ClientError::UpdateMac => (
                "updateMac",
                "Update Orttaai on the Mac to keep syncing. Changes wait here.".to_string(),
            ),
            ClientError::WrongKey(_) => ("error", error.to_string()),
            _ => ("error", error.to_string()),
        };
        self.update(|s| {
            s.state = state.into();
            s.message = Some(message);
        });
    }

    fn note_ok(&self, store: &Store) {
        let counts = store.sync_counts().unwrap_or_default();
        self.update(|s| {
            s.state = if counts.waiting > 0 {
                "syncing".into()
            } else {
                "idle".into()
            };
            s.message = None;
            s.counts = counts;
        });
    }

    // ---- Sending ---------------------------------------------------------------

    fn send_loop(&self) {
        let mut backoff = 0usize;
        let mut last_device_check = Instant::now() - Duration::from_secs(3600);
        while !self.stopped() {
            let pause = match self.send_once(&mut last_device_check) {
                Ok(true) => Duration::from_millis(50),
                Ok(false) => {
                    backoff = 0;
                    Duration::from_secs(2)
                }
                Err(e) => {
                    self.note_error(&e);
                    if matches!(e, ClientError::Unlinked) {
                        Duration::from_secs(300)
                    } else {
                        backoff = (backoff + 1).min(4);
                        Duration::from_secs([5, 15, 60, 300][backoff - 1])
                    }
                }
            };
            self.sleep(pause);
        }
    }

    /// Sends one batch. `Ok(true)` when more are waiting.
    fn send_once(&self, last_device_check: &mut Instant) -> Result<bool, ClientError> {
        let Some(store) = self.open() else {
            return Ok(false);
        };
        let Some(mut mac) = store.sync_linked_mac().ok().flatten() else {
            return Ok(false);
        };
        let _ = store.sync_collect();
        if last_device_check.elapsed() > Duration::from_secs(60) {
            *last_device_check = Instant::now();
            let _ = store.sync_publish_device(&self.device);
        }
        let batch = store.sync_outgoing(MAXIMUM_BATCH).unwrap_or_default();
        if batch.is_empty() {
            let counts = store.sync_counts().unwrap_or_default();
            self.update(|s| s.counts = counts);
            return Ok(false);
        }
        self.update(|s| s.state = "syncing".into());
        let request = PushRequest {
            records: batch.iter().map(LinkRecord::from).collect(),
        };
        let port = mac.port;
        let answer: PushResponse = self.with_mac(&store, &mut mac, |client, host| {
            client.post(
                host,
                port,
                "/sync/v1/push",
                &request,
                Duration::from_secs(60),
            )
        })?;
        for accepted in &answer.accepted {
            let _ = store.sync_acknowledge(&accepted.record_name, &accepted.revision.revision());
        }
        for rejected in &answer.rejected {
            let _ = store.sync_mark_failed(&rejected.record_name, &rejected.problem);
        }
        if !answer.newer.is_empty() {
            let newer: Vec<Envelope> = answer
                .newer
                .iter()
                .filter_map(|r| r.envelope().ok())
                .collect();
            if let Ok(result) = store.sync_apply(&newer) {
                if result.changed > 0 {
                    (self.on_change)(&result);
                }
            }
        }
        self.note_ok(&store);
        Ok(batch.len() >= MAXIMUM_BATCH)
    }

    // ---- Receiving -------------------------------------------------------------

    fn receive_loop(&self) {
        let mut backoff = 0usize;
        let mut last_status = Instant::now() - Duration::from_secs(3600);
        while !self.stopped() {
            let generation = self.generation();
            match self.receive_once(&mut last_status) {
                Ok(()) => backoff = 0,
                Err(e) => {
                    self.note_error(&e);
                    backoff = (backoff + 1).min(4);
                    let pause = if matches!(e, ClientError::Unlinked) {
                        300
                    } else {
                        [5, 15, 60, 300][backoff - 1]
                    };
                    // Wait, unless the link changed meanwhile.
                    let until = Instant::now() + Duration::from_secs(pause);
                    while Instant::now() < until
                        && !self.stopped()
                        && self.generation() == generation
                    {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            if !self.status().linked {
                // Not linked: check again shortly (linking bumps the generation).
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(5)
                    && self.generation() == generation
                    && !self.stopped()
                {
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }

    fn receive_once(&self, last_status: &mut Instant) -> Result<(), ClientError> {
        let Some(store) = self.open() else {
            std::thread::sleep(Duration::from_secs(2));
            return Ok(());
        };
        let Some(mut mac) = store.sync_linked_mac().ok().flatten() else {
            return Ok(());
        };
        if last_status.elapsed() > Duration::from_secs(60) {
            *last_status = Instant::now();
            let port = mac.port;
            let status: StatusResponse = self.with_mac(&store, &mut mac, |client, host| {
                client.get(host, port, "/link/v1/status", Duration::from_secs(10))
            })?;
            if status.tailscale_name != mac.tailscale_name && status.tailscale_name.is_some() {
                mac.tailscale_name = status.tailscale_name.clone();
                let _ = store.sync_save_linked_mac(&mac);
            }
            self.update(|s| {
                s.icloud = Some(status.icloud.clone());
                s.mac_name = Some(status.name.clone());
                s.tailscale_name = mac.tailscale_name.clone();
            });
        }
        loop {
            let cursor = store.sync_cursor().unwrap_or_default();
            let path = format!("/sync/v1/changes?since={cursor}&limit={MAXIMUM_BATCH}&wait=25");
            let port = mac.port;
            let page: ChangesResponse = self.with_mac(&store, &mut mac, |client, host| {
                client.get(host, port, &path, Duration::from_secs(40))
            })?;
            let envelopes: Vec<Envelope> = page
                .records
                .iter()
                .filter_map(|r| r.envelope().ok())
                .collect();
            if !envelopes.is_empty() {
                if let Ok(result) = store.sync_apply(&envelopes) {
                    if result.changed > 0 {
                        (self.on_change)(&result);
                    }
                }
            }
            let _ = store.sync_set_cursor(&page.cursor);
            self.note_ok(&store);
            if !page.more || self.stopped() {
                return Ok(());
            }
        }
    }
}

/// Whether a change touched what the History and Home screens show.
pub fn touches_history(result: &ApplyResult) -> bool {
    result.kinds.contains(&RecordKind::Dictation)
}
