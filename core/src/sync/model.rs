//! The sync model shared with Orttaai on the Mac and Orttakey: device kinds,
//! zones, record names, revisions, the conflict policy and the payloads.
//! A port of the `OrttaaiSyncCore` Swift package (orttaai repo,
//! `LocalPackages/OrttaaiSync`); `docs/orttaai-link.md` is the contract.
//! The sample records in `tests/fixtures/link` keep the two in step.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use unicode_normalization::UnicodeNormalization;

/// The newest schema this version reads (see `SyncEnvelope.currentSchemaVersion`).
pub const CURRENT_SCHEMA_VERSION: i64 = 2;
/// CloudKit's 1 MB record limit, with room for the plain fields.
pub const MAXIMUM_PAYLOAD_BYTES: usize = 900_000;

// ---- Device kinds --------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Mac,
    Iphone,
    Ipad,
    Windows,
    Linux,
    Android,
    Unknown,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mac => "mac",
            Self::Iphone => "iphone",
            Self::Ipad => "ipad",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Android => "android",
            Self::Unknown => "unknown",
        }
    }

    /// A kind this version doesn't know reads as `Unknown`.
    pub fn parse(s: &str) -> Self {
        match s {
            "mac" => Self::Mac,
            "iphone" => Self::Iphone,
            "ipad" => Self::Ipad,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            "android" => Self::Android,
            _ => Self::Unknown,
        }
    }

    /// The schema version that introduced the kind.
    pub fn schema_version(self) -> i64 {
        match self {
            Self::Windows | Self::Linux | Self::Android => 2,
            _ => 1,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Mac => "Mac",
            Self::Iphone => "iPhone",
            Self::Ipad => "iPad",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Android => "Android",
            Self::Unknown => "Unknown device",
        }
    }

    /// This computer's kind.
    pub fn this_platform() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

impl Serialize for DeviceKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DeviceKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}

// ---- Zones and record names ----------------------------------------------

const DICTATIONS_PREFIX: &str = "Dictations-";
const LEGACY: &str = "legacy";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Zone {
    Library,
    MacOnly,
    Dictations(String),
    LegacyDictations,
}

impl Zone {
    pub fn name(&self) -> String {
        match self {
            Self::Library => "Library".into(),
            Self::MacOnly => "MacOnly".into(),
            Self::Dictations(id) => format!("{DICTATIONS_PREFIX}{id}"),
            Self::LegacyDictations => format!("{DICTATIONS_PREFIX}{LEGACY}"),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Library" => Some(Self::Library),
            "MacOnly" => Some(Self::MacOnly),
            _ => {
                let id = name.strip_prefix(DICTATIONS_PREFIX)?;
                if id == LEGACY {
                    Some(Self::LegacyDictations)
                } else if is_valid_device_id(id) {
                    Some(Self::Dictations(id.to_string()))
                } else {
                    None
                }
            }
        }
    }

    pub fn is_dictation_zone(&self) -> bool {
        matches!(self, Self::Dictations(_) | Self::LegacyDictations)
    }
}

/// Lowercase UUID-shaped identifiers (letters, digits, dashes; at most 64).
pub fn is_valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != LEGACY
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordKind {
    Dictation,
    DictionaryEntry,
    Snippet,
    Device,
    MacSummary,
    MacPreferences,
    ToneProfile,
    ChatConversation,
    LearningSuggestion,
    WritingInsight,
}

impl RecordKind {
    pub const ALL: [RecordKind; 10] = [
        Self::Dictation,
        Self::DictionaryEntry,
        Self::Snippet,
        Self::Device,
        Self::MacSummary,
        Self::MacPreferences,
        Self::ToneProfile,
        Self::ChatConversation,
        Self::LearningSuggestion,
        Self::WritingInsight,
    ];

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Dictation => "dictation.",
            Self::DictionaryEntry => "dict.",
            Self::Snippet => "snippet.",
            Self::Device => "device.",
            Self::MacSummary => "summary.",
            Self::MacPreferences => "prefs.",
            Self::ToneProfile => "tone.",
            Self::ChatConversation => "chat.",
            Self::LearningSuggestion => "suggestion.",
            Self::WritingInsight => "insight.",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dictation => "Dictation",
            Self::DictionaryEntry => "DictionaryEntry",
            Self::Snippet => "Snippet",
            Self::Device => "Device",
            Self::MacSummary => "MacSummary",
            Self::MacPreferences => "MacPreferences",
            Self::ToneProfile => "ToneProfile",
            Self::ChatConversation => "ChatConversation",
            Self::LearningSuggestion => "LearningSuggestion",
            Self::WritingInsight => "WritingInsight",
        }
    }

    pub fn of(record_name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| record_name.starts_with(k.prefix()))
    }

    /// A deleted dictation never comes back; a deleted word can.
    pub fn deletion_is_terminal(self) -> bool {
        self == Self::Dictation
    }

    fn fixed_zone(self) -> Option<Zone> {
        match self {
            Self::Dictation => None,
            Self::DictionaryEntry | Self::Device | Self::MacSummary => Some(Zone::Library),
            _ => Some(Zone::MacOnly),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordId {
    pub zone: Zone,
    pub name: String,
}

impl RecordId {
    pub fn kind(&self) -> Option<RecordKind> {
        RecordKind::of(&self.name)
    }

    pub fn local_key(&self) -> &str {
        match self.kind() {
            Some(k) => &self.name[k.prefix().len()..],
            None => &self.name,
        }
    }

    pub fn is_well_formed(&self) -> bool {
        let Some(kind) = self.kind() else {
            return false;
        };
        if self.local_key().is_empty() || self.name.len() > 255 {
            return false;
        }
        match kind.fixed_zone() {
            Some(zone) => self.zone == zone,
            None => self.zone.is_dictation_zone(),
        }
    }

    pub fn dictation(sync_id: &str, origin_device_id: Option<&str>) -> Self {
        let zone = match origin_device_id {
            Some(id) if is_valid_device_id(id) => Zone::Dictations(id.to_string()),
            _ => Zone::LegacyDictations,
        };
        Self {
            zone,
            name: format!("{}{sync_id}", RecordKind::Dictation.prefix()),
        }
    }

    pub fn dictionary_entry(normalized_source: &str) -> Self {
        Self {
            zone: Zone::Library,
            name: format!(
                "{}{}",
                RecordKind::DictionaryEntry.prefix(),
                digest(normalized_source)
            ),
        }
    }

    pub fn snippet(normalized_trigger: &str) -> Self {
        Self {
            zone: Zone::MacOnly,
            name: format!(
                "{}{}",
                RecordKind::Snippet.prefix(),
                digest(normalized_trigger)
            ),
        }
    }

    pub fn device(device_id: &str) -> Self {
        Self {
            zone: Zone::Library,
            name: format!("{}{device_id}", RecordKind::Device.prefix()),
        }
    }

    pub fn learning_suggestion(sync_id: &str) -> Self {
        Self {
            zone: Zone::MacOnly,
            name: format!("{}{sync_id}", RecordKind::LearningSuggestion.prefix()),
        }
    }

    pub fn writing_insight(sync_id: &str) -> Self {
        Self {
            zone: Zone::MacOnly,
            name: format!("{}{sync_id}", RecordKind::WritingInsight.prefix()),
        }
    }
}

/// `SyncNormalization.normalizedKey`: trimmed, lowercased, inner whitespace
/// collapsed to one space.
pub fn normalized_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `SyncNormalization.digest`: lowercase hex SHA-256 of the key's NFC form.
pub fn digest(normalized_key: &str) -> String {
    let canonical: String = normalized_key.nfc().collect();
    hex(&Sha256::digest(canonical.as_bytes()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `WordCount.of`: whitespace-separated tokens.
pub fn word_count(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

// ---- Revisions -----------------------------------------------------------

/// A Lamport counter, then the wall clock, then the writer: every device
/// orders two versions the same way.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Revision {
    pub counter: i64,
    /// Milliseconds since 1970.
    pub modified_ms: i64,
    pub writer: String,
}

impl Ord for Revision {
    fn cmp(&self, other: &Self) -> Ordering {
        self.counter
            .cmp(&other.counter)
            .then(self.modified_ms.cmp(&other.modified_ms))
            .then_with(|| self.writer.cmp(&other.writer))
    }
}

impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---- Envelopes ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub device_id: Option<String>,
    pub kind: DeviceKind,
}

/// One record as every app sees it; `payload` is the kind's JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub id: RecordId,
    pub schema_version: i64,
    pub revision: Revision,
    pub generation: i64,
    pub is_deleted: bool,
    pub deleted_at_ms: Option<i64>,
    pub payload: Option<Vec<u8>>,
    pub origin: Option<Origin>,
    pub created_at_ms: Option<i64>,
}

impl Envelope {
    /// The lowest schema version that carries this record.
    pub fn required_schema_version(
        id: &RecordId,
        payload: Option<&[u8]>,
        origin: Option<&Origin>,
    ) -> i64 {
        let mut version = 1;
        if let Some(origin) = origin {
            version = version.max(origin.kind.schema_version());
        }
        if id.kind() == Some(RecordKind::Device) {
            if let Some(device) =
                payload.and_then(|p| serde_json::from_slice::<DevicePayload>(p).ok())
            {
                version = version.max(device.kind.schema_version());
            }
        }
        version
    }

    pub fn kind(&self) -> Option<RecordKind> {
        self.id.kind()
    }

    /// Why this record can't be applied (`SyncEnvelope.problem`), or `None`.
    pub fn problem(&self) -> Option<String> {
        let Some(kind) = self.kind() else {
            return Some("malformed record name or zone".into());
        };
        if !self.id.is_well_formed() || self.schema_version < 1 {
            return Some("malformed record name or zone".into());
        }
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Some(format!(
                "schema version {} is newer than this app",
                self.schema_version
            ));
        }
        if self.is_deleted {
            if kind.deletion_is_terminal() && self.payload.is_some() {
                return Some("deleted dictation still carrying text".into());
            }
        } else {
            let Some(payload) = self.payload.as_deref().filter(|p| !p.is_empty()) else {
                return Some("live record without a payload".into());
            };
            if payload.len() > MAXIMUM_PAYLOAD_BYTES {
                return Some(format!(
                    "payload of {} bytes is over the limit",
                    payload.len()
                ));
            }
            if !payload_decodes(kind, payload) {
                return Some("payload could not be decoded".into());
            }
        }
        if kind == RecordKind::Dictation {
            let Some(origin) = &self.origin else {
                return Some("dictation without an origin".into());
            };
            if RecordId::dictation("x", origin.device_id.as_deref()).zone != self.id.zone {
                return Some("dictation in another device's zone".into());
            }
        }
        None
    }

    /// A schema newer than this app reads: keep it for after an update.
    pub fn is_from_newer_schema(&self) -> bool {
        self.schema_version > CURRENT_SCHEMA_VERSION
    }
}

fn payload_decodes(kind: RecordKind, payload: &[u8]) -> bool {
    match kind {
        RecordKind::Dictation => serde_json::from_slice::<DictationPayload>(payload).is_ok(),
        RecordKind::DictionaryEntry => {
            serde_json::from_slice::<DictionaryEntryPayload>(payload).is_ok()
        }
        RecordKind::Snippet => serde_json::from_slice::<SnippetPayload>(payload).is_ok(),
        RecordKind::Device => serde_json::from_slice::<DevicePayload>(payload).is_ok(),
        RecordKind::LearningSuggestion => {
            serde_json::from_slice::<LearningSuggestionPayload>(payload).is_ok()
        }
        RecordKind::WritingInsight => {
            serde_json::from_slice::<WritingInsightPayload>(payload).is_ok()
        }
        // Kept as they came; this app doesn't read them.
        RecordKind::MacSummary
        | RecordKind::MacPreferences
        | RecordKind::ToneProfile
        | RecordKind::ChatConversation => {
            serde_json::from_slice::<serde_json::Value>(payload).is_ok()
        }
    }
}

// ---- Conflict policy ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalState {
    pub revision: Revision,
    pub is_deleted: bool,
    pub generation: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AcceptRemote,
    KeepLocal,
    AlreadyCurrent,
}

/// `ConflictPolicy.decide`.
pub fn decide(local: Option<&LocalState>, remote: &Envelope) -> Decision {
    let Some(local) = local else {
        return Decision::AcceptRemote;
    };
    let Some(kind) = remote.kind() else {
        return Decision::KeepLocal;
    };
    if kind.deletion_is_terminal() {
        match (local.is_deleted, remote.is_deleted) {
            (true, true) => return Decision::AlreadyCurrent,
            (true, false) => return Decision::KeepLocal,
            (false, true) => return Decision::AcceptRemote,
            (false, false) => {}
        }
    } else {
        if remote.generation != local.generation {
            return if remote.generation > local.generation {
                Decision::AcceptRemote
            } else {
                Decision::KeepLocal
            };
        }
        if local.is_deleted != remote.is_deleted {
            return if remote.is_deleted {
                Decision::AcceptRemote
            } else {
                Decision::KeepLocal
            };
        }
    }
    match remote.revision.cmp(&local.revision) {
        Ordering::Greater => Decision::AcceptRemote,
        Ordering::Less => Decision::KeepLocal,
        Ordering::Equal => Decision::AlreadyCurrent,
    }
}

// ---- Payloads -------------------------------------------------------------
//
// Swift writes dates as milliseconds since 1970 (a number that may carry a
// fraction) and leaves out optional fields with no value.

mod ms {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(*v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        Ok(f64::deserialize(d)?.round() as i64)
    }

    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(v) => s.serialize_i64(*v),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
            Ok(Option::<f64>::deserialize(d)?.map(|v| v.round() as i64))
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationPayload {
    #[serde(with = "ms")]
    pub created_at: i64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_app_name: Option<String>,
    #[serde(
        default,
        rename = "targetAppBundleID",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_app_bundle_id: Option<String>,
    #[serde(default, rename = "modelID", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_processing_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injection_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_sync_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_activation_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard_restore_delay_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injection_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone_identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntryPayload {
    pub source: String,
    pub target: String,
    pub normalized_source: String,
    pub is_case_sensitive: bool,
    pub is_active: bool,
    #[serde(with = "ms")]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPayload {
    pub trigger: String,
    pub expansion: String,
    pub normalized_trigger: String,
    pub is_active: bool,
    #[serde(with = "ms")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTotals {
    pub dictations: i64,
    pub words: i64,
    pub recording_ms: i64,
    #[serde(default, with = "ms::option", skip_serializing_if = "Option::is_none")]
    pub first_at: Option<i64>,
    #[serde(default, with = "ms::option", skip_serializing_if = "Option::is_none")]
    pub last_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePayload {
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
    #[serde(with = "ms")]
    pub first_seen_at: i64,
    #[serde(with = "ms")]
    pub last_seen_at: i64,
    pub totals: DeviceTotals,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningSuggestionPayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub candidate_source: String,
    pub candidate_target: String,
    pub normalized_source: String,
    pub confidence: f64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(with = "ms")]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingInsightPayload {
    #[serde(with = "ms")]
    pub generated_at: i64,
    pub analyzer_name: String,
    pub used_fallback: bool,
    pub is_pinned: bool,
    pub sample_count: i64,
    #[serde(rename = "requestJSON")]
    pub request_json: String,
    #[serde(rename = "snapshotJSON")]
    pub snapshot_json: String,
}

/// Payload JSON as every app writes it: keys sorted, like Swift's encoder.
pub fn encode_payload<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    // serde_json's Value keeps keys sorted (BTreeMap), so a round trip
    // through it sorts them.
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&value)
}
