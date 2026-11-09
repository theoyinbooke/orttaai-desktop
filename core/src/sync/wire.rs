//! Records and request bodies as Orttaai Link carries them
//! (`docs/orttaai-link.md` §4–§6); the Swift side is `LinkRecord` and
//! `LinkAPI.swift` in the orttaai repo.

use super::model::{DeviceKind, Envelope, Origin, RecordId, Revision, Zone};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: i64 = 1;
pub const DEFAULT_PORT: u16 = 47821;
pub const BONJOUR_TYPE: &str = "_orttaai._tcp.local.";
pub const MAXIMUM_BATCH: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRevision {
    pub counter: i64,
    #[serde(rename = "modifiedAt")]
    pub modified_at: i64,
    pub writer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOrigin {
    #[serde(rename = "deviceID", default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    pub kind: DeviceKind,
}

/// One record on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRecord {
    pub zone: String,
    pub record_name: String,
    pub schema_version: i64,
    pub revision: WireRevision,
    pub generation: i64,
    pub is_deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
    /// Base64 of the payload JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<WireOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    #[error("unknown zone {0}")]
    UnknownZone(String),
    #[error("payload of {0} isn't base64")]
    UnreadablePayload(String),
}

impl From<&Revision> for WireRevision {
    fn from(r: &Revision) -> Self {
        Self {
            counter: r.counter,
            modified_at: r.modified_ms,
            writer: r.writer.clone(),
        }
    }
}

impl WireRevision {
    pub fn revision(&self) -> Revision {
        Revision {
            counter: self.counter,
            modified_ms: self.modified_at,
            writer: self.writer.clone(),
        }
    }
}

impl From<&Envelope> for LinkRecord {
    fn from(e: &Envelope) -> Self {
        Self {
            zone: e.id.zone.name(),
            record_name: e.id.name.clone(),
            schema_version: e.schema_version,
            revision: (&e.revision).into(),
            generation: e.generation,
            is_deleted: e.is_deleted,
            deleted_at: e.deleted_at_ms,
            payload: e
                .payload
                .as_ref()
                .map(|p| base64::engine::general_purpose::STANDARD.encode(p)),
            origin: e.origin.as_ref().map(|o| WireOrigin {
                device_id: o.device_id.clone(),
                kind: o.kind,
            }),
            created_at: e.created_at_ms,
        }
    }
}

impl LinkRecord {
    pub fn envelope(&self) -> Result<Envelope, WireError> {
        let zone =
            Zone::parse(&self.zone).ok_or_else(|| WireError::UnknownZone(self.zone.clone()))?;
        let payload = match &self.payload {
            Some(p) => Some(
                base64::engine::general_purpose::STANDARD
                    .decode(p)
                    .map_err(|_| WireError::UnreadablePayload(self.record_name.clone()))?,
            ),
            None => None,
        };
        Ok(Envelope {
            id: RecordId {
                zone,
                name: self.record_name.clone(),
            },
            schema_version: self.schema_version,
            revision: self.revision.revision(),
            generation: self.generation,
            is_deleted: self.is_deleted,
            deleted_at_ms: self.deleted_at,
            payload,
            origin: self.origin.as_ref().map(|o| Origin {
                device_id: o.device_id.clone(),
                kind: o.kind,
            }),
            created_at_ms: self.created_at,
        })
    }
}

// ---- Request and response bodies -------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkDeviceInfo {
    #[serde(rename = "deviceID")]
    pub device_id: String,
    pub kind: DeviceKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub device: LinkDeviceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub token: String,
    #[serde(rename = "macDeviceID")]
    pub mac_device_id: String,
    pub name: String,
    #[serde(default)]
    pub tailscale_name: Option<String>,
    pub protocol: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushRequest {
    pub records: Vec<LinkRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub record_name: String,
    pub revision: WireRevision,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejected {
    pub record_name: String,
    pub problem: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushResponse {
    #[serde(default)]
    pub accepted: Vec<Accepted>,
    #[serde(default)]
    pub newer: Vec<LinkRecord>,
    #[serde(default)]
    pub rejected: Vec<Rejected>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChangesResponse {
    pub records: Vec<LinkRecord>,
    pub cursor: String,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ICloudStatus {
    pub state: String,
    #[serde(default)]
    pub last_sent_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub name: String,
    #[serde(rename = "macDeviceID")]
    pub mac_device_id: String,
    pub version: String,
    pub protocol: i64,
    #[serde(rename = "iCloud")]
    pub icloud: ICloudStatus,
    #[serde(default)]
    pub tailscale_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

/// What the Mac's QR code or "Copy Link" carries
/// (`orttaai://link?v=1&id=…&n=…&h=…&p=…&fp=…[&ts=…]#secret`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInvite {
    pub mac_device_id: String,
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub fingerprint: String,
    pub tailscale_name: Option<String>,
    pub secret: String,
}

impl LinkInvite {
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.trim().strip_prefix("orttaai://link")?;
        let (query, secret) = rest.split_once('#')?;
        let query = query.strip_prefix('?')?;
        let mut invite = LinkInvite {
            mac_device_id: String::new(),
            name: String::new(),
            hosts: vec![],
            port: DEFAULT_PORT,
            fingerprint: String::new(),
            tailscale_name: None,
            secret: secret.to_string(),
        };
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = percent_decode(value);
            match key {
                "v" if value != "1" => return None,
                "id" => invite.mac_device_id = value,
                "n" => invite.name = value,
                "h" => {
                    invite.hosts = value
                        .split(',')
                        .filter(|h| !h.is_empty())
                        .map(String::from)
                        .collect()
                }
                "p" => invite.port = value.parse().ok()?,
                "fp" => invite.fingerprint = value.to_lowercase(),
                "ts" if !value.is_empty() => invite.tailscale_name = Some(value),
                _ => {}
            }
        }
        let ok = !invite.mac_device_id.is_empty()
            && invite.fingerprint.len() == 64
            && !invite.secret.is_empty()
            && (!invite.hosts.is_empty() || invite.tailscale_name.is_some());
        ok.then_some(invite)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
