//! The sync ledger on this PC's database (`docs/orttaai-link.md` §8).
//!
//! While linked, triggers note every change to a synced table in
//! `sync_pending`. [`Store::sync_collect`] turns those into records with a new
//! revision and queues them in `sync_outbox`; the engine sends the outbox to
//! the Mac and applies what comes back with [`Store::sync_apply`], the same
//! conflict policy every device uses. Applying turns the triggers off, so
//! records from the Mac are never sent back as local changes.

use super::model::*;
use crate::error::Result;
use crate::store::{now_unix, Store};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The Mac this PC is linked to, as linking left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedMac {
    pub mac_device_id: String,
    pub name: String,
    /// Public-key fingerprint (lowercase hex SHA-256 of the X9.63 key).
    pub fingerprint: String,
    pub token: String,
    /// Home-network addresses to try, then the `.local` name.
    pub hosts: Vec<String>,
    pub port: u16,
    pub tailscale_name: Option<String>,
    /// The last address that answered.
    #[serde(default)]
    pub last_host: Option<String>,
}

/// What applying a batch did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyResult {
    /// Records whose content changed here.
    pub changed: usize,
    /// Kinds that changed, for refreshing the right screens.
    pub kinds: Vec<RecordKind>,
    /// Records set aside: from a newer schema, or unreadable.
    pub skipped: usize,
}

/// Counts for the Devices page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCounts {
    pub waiting: i64,
    pub failed: i64,
    pub last_failure: Option<String>,
}

const TABLE_TRANSCRIPTION: &str = "transcription";
const TABLE_DICTIONARY: &str = "dictionary_entry";
const TABLE_SNIPPET: &str = "snippet_entry";
const TABLE_SUGGESTION: &str = "learning_suggestion";
const TABLE_INSIGHT: &str = "writing_insight_snapshot";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct RecordRow {
    zone: String,
    local_table: Option<String>,
    local_id: Option<i64>,
    revision: Revision,
    generation: i64,
    is_deleted: bool,
    deleted_at_ms: Option<i64>,
    origin_device_id: Option<String>,
    origin_kind: Option<String>,
    created_at_ms: Option<i64>,
    payload: Option<Vec<u8>>,
}

impl RecordRow {
    const COLUMNS: &'static str =
        "zone, local_table, local_id, rev_counter, rev_modified_ms, rev_writer, generation, \
        is_deleted, deleted_at_ms, origin_device_id, origin_kind, created_at_ms, payload";

    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            zone: r.get(0)?,
            local_table: r.get(1)?,
            local_id: r.get(2)?,
            revision: Revision {
                counter: r.get(3)?,
                modified_ms: r.get(4)?,
                writer: r.get(5)?,
            },
            generation: r.get(6)?,
            is_deleted: r.get(7)?,
            deleted_at_ms: r.get(8)?,
            origin_device_id: r.get(9)?,
            origin_kind: r.get(10)?,
            created_at_ms: r.get(11)?,
            payload: r.get(12)?,
        })
    }

    fn local_state(&self) -> LocalState {
        LocalState {
            revision: self.revision.clone(),
            is_deleted: self.is_deleted,
            generation: self.generation,
        }
    }

    fn origin(&self) -> Option<Origin> {
        self.origin_kind.as_deref().map(|k| Origin {
            device_id: self.origin_device_id.clone(),
            kind: DeviceKind::parse(k),
        })
    }
}

impl Store {
    // ---- State -----------------------------------------------------------

    pub fn sync_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM sync_state WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?
            .flatten())
    }

    pub fn sync_set(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// This PC's sync identity, made the first time it's needed.
    pub fn sync_device_id(&self) -> Result<String> {
        if let Some(id) = self.sync_get("device_id")? {
            return Ok(id);
        }
        let id = self.new_uuid()?;
        self.sync_set("device_id", Some(&id))?;
        Ok(id)
    }

    fn new_uuid(&self) -> Result<String> {
        let bytes: Vec<u8> = self
            .conn()
            .query_row("SELECT randomblob(16)", [], |r| r.get(0))?;
        let h = hex(&bytes);
        Ok(format!(
            "{}-{}-4{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[13..16],
            &h[16..20],
            &h[20..32]
        ))
    }

    pub fn sync_linked_mac(&self) -> Result<Option<LinkedMac>> {
        Ok(self
            .sync_get("link")?
            .and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn sync_save_linked_mac(&self, mac: &LinkedMac) -> Result<()> {
        self.sync_set("link", Some(&serde_json::to_string(mac)?))
    }

    pub fn sync_cursor(&self) -> Result<String> {
        Ok(self.sync_get("cursor")?.unwrap_or_default())
    }

    pub fn sync_set_cursor(&self, cursor: &str) -> Result<()> {
        self.sync_set("cursor", Some(cursor))
    }

    fn sync_capture(&self, on: bool) -> Result<()> {
        self.sync_set("capture", Some(if on { "1" } else { "0" }))
    }

    // ---- Linking -----------------------------------------------------------

    /// Starts syncing with a Mac: notes every change from now on and queues
    /// everything already here, so the Mac gets the whole archive.
    pub fn sync_link(&self, mac: &LinkedMac, device: &DeviceInfo) -> Result<()> {
        self.sync_device_id()?;
        self.conn().execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            self.sync_save_linked_mac(mac)?;
            self.sync_set("cursor", None)?;
            self.sync_capture(true)?;
            self.conn().execute_batch(
                "DELETE FROM sync_pending;
                 INSERT INTO sync_pending (local_table, local_id) SELECT 'transcription', id FROM transcription ORDER BY id;
                 INSERT INTO sync_pending (local_table, local_id) SELECT 'dictionary_entry', id FROM dictionary_entry;
                 INSERT INTO sync_pending (local_table, local_id) SELECT 'snippet_entry', id FROM snippet_entry;
                 INSERT INTO sync_pending (local_table, local_id) SELECT 'learning_suggestion', id FROM learning_suggestion;
                 INSERT INTO sync_pending (local_table, local_id) SELECT 'writing_insight_snapshot', id FROM writing_insight_snapshot WHERE kind = 'writing';",
            )?;
            self.collect_pending()?;
            self.publish_device(device, true)?;
            Ok(())
        })();
        self.finish(result)
    }

    /// Stops syncing. Everything stays on this PC; linking again sends what
    /// changed meanwhile.
    pub fn sync_unlink(&self) -> Result<()> {
        self.sync_capture(false)?;
        self.conn().execute_batch(
            "DELETE FROM sync_state WHERE key IN ('link', 'cursor', 'device_published');
             DELETE FROM sync_pending; DELETE FROM sync_outbox;",
        )?;
        Ok(())
    }

    fn finish(&self, result: Result<()>) -> Result<()> {
        match result {
            Ok(()) => {
                self.conn().execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn().execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ---- Revisions -----------------------------------------------------------

    fn next_revision(&self) -> Result<Revision> {
        let counter = self
            .sync_get("counter")?
            .and_then(|c| c.parse::<i64>().ok())
            .unwrap_or(0)
            + 1;
        self.sync_set("counter", Some(&counter.to_string()))?;
        Ok(Revision {
            counter,
            modified_ms: now_ms(),
            writer: self.sync_device_id()?,
        })
    }

    fn observe(&self, revision: &Revision) -> Result<()> {
        let counter = self
            .sync_get("counter")?
            .and_then(|c| c.parse::<i64>().ok())
            .unwrap_or(0);
        if revision.counter > counter {
            self.sync_set("counter", Some(&revision.counter.to_string()))?;
        }
        Ok(())
    }

    // ---- Local changes -------------------------------------------------------

    /// Turns noted local changes into records waiting for the Mac. Returns
    /// how many were queued.
    pub fn sync_collect(&self) -> Result<usize> {
        let pending: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM sync_pending", [], |r| r.get(0))?;
        if pending == 0 {
            return Ok(0);
        }
        self.conn().execute_batch("BEGIN IMMEDIATE")?;
        let mut queued = 0;
        let result = (|| -> Result<()> {
            queued = self.collect_pending()?;
            Ok(())
        })();
        self.finish(result)?;
        Ok(queued)
    }

    fn collect_pending(&self) -> Result<usize> {
        let items: Vec<(i64, String, i64, bool, Option<String>)> = {
            let mut stmt = self.conn().prepare(
                "SELECT id, local_table, local_id, deleted, sync_key FROM sync_pending ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut queued = 0;
        let mut done_live = std::collections::HashSet::new();
        for (_, table, local_id, deleted, key) in &items {
            let changed = if *deleted {
                self.collect_deletion(table, key.as_deref())?
            } else if done_live.insert((table.clone(), *local_id)) {
                self.collect_live(table, *local_id)?
            } else {
                false
            };
            if changed {
                queued += 1;
            }
        }
        if let Some(last) = items.last() {
            self.conn()
                .execute("DELETE FROM sync_pending WHERE id <= ?1", [last.0])?;
        }
        Ok(queued)
    }

    fn collect_live(&self, table: &str, local_id: i64) -> Result<bool> {
        let me = self.sync_device_id()?;
        match table {
            TABLE_TRANSCRIPTION => {
                type Row = (Option<String>, Option<String>, Option<String>, i64);
                let row: Option<Row> = self
                    .conn()
                    .query_row(
                        "SELECT sync_id, source_device_id, source_device_kind, created_at FROM transcription WHERE id = ?1",
                        [local_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                let Some((sync_id, source_id, source_kind, created_at)) = row else {
                    return Ok(false);
                };
                let sync_id = match sync_id {
                    Some(id) => id,
                    None => self.new_uuid()?,
                };
                let source_id = source_id.unwrap_or_else(|| me.clone());
                let source_kind =
                    source_kind.unwrap_or_else(|| DeviceKind::this_platform().as_str().to_string());
                self.conn().execute(
                    "UPDATE transcription SET sync_id = ?2, source_device_id = ?3, source_device_kind = ?4 WHERE id = ?1",
                    params![local_id, sync_id, source_id, source_kind],
                )?;
                let id = RecordId::dictation(&sync_id, Some(&source_id));
                if self.record(&id.name)?.is_some_and(|r| r.is_deleted) {
                    return Ok(false);
                }
                let origin = Origin {
                    device_id: Some(source_id),
                    kind: DeviceKind::parse(&source_kind),
                };
                self.save_local(
                    &id,
                    TABLE_TRANSCRIPTION,
                    local_id,
                    0,
                    Some(&origin),
                    Some(created_at * 1000),
                )?;
                Ok(true)
            }
            TABLE_DICTIONARY | TABLE_SNIPPET => {
                let column = if table == TABLE_DICTIONARY {
                    "source"
                } else {
                    "trigger"
                };
                let source: Option<String> = self
                    .conn()
                    .query_row(
                        &format!("SELECT {column} FROM {table} WHERE id = ?1"),
                        [local_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(source) = source else {
                    return Ok(false);
                };
                let key = normalized_key(&source);
                let id = if table == TABLE_DICTIONARY {
                    RecordId::dictionary_entry(&key)
                } else {
                    RecordId::snippet(&key)
                };
                // Renamed: the old name's record is deleted.
                let previous: Vec<String> = {
                    let mut stmt = self.conn().prepare(
                        "SELECT record_name FROM sync_record WHERE local_table = ?1 AND local_id = ?2 AND record_name != ?3 AND is_deleted = 0",
                    )?;
                    let rows = stmt.query_map(params![table, local_id, id.name], |r| r.get(0))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                for name in previous {
                    self.tombstone(&name)?;
                }
                let generation = match self.record(&id.name)? {
                    Some(existing) if existing.is_deleted => existing.generation + 1,
                    Some(existing) => existing.generation,
                    None => 0,
                };
                self.save_local(&id, table, local_id, generation, None, None)?;
                Ok(true)
            }
            TABLE_SUGGESTION | TABLE_INSIGHT => {
                let sync_id: Option<Option<String>> = self
                    .conn()
                    .query_row(
                        &format!("SELECT sync_id FROM {table} WHERE id = ?1"),
                        [local_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(sync_id) = sync_id else {
                    return Ok(false);
                };
                let sync_id = match sync_id {
                    Some(id) => id,
                    None => {
                        let id = self.new_uuid()?;
                        self.conn().execute(
                            &format!("UPDATE {table} SET sync_id = ?2 WHERE id = ?1"),
                            params![local_id, id],
                        )?;
                        id
                    }
                };
                let id = if table == TABLE_SUGGESTION {
                    RecordId::learning_suggestion(&sync_id)
                } else {
                    RecordId::writing_insight(&sync_id)
                };
                self.save_local(&id, table, local_id, 0, None, None)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn collect_deletion(&self, table: &str, key: Option<&str>) -> Result<bool> {
        let Some(key) = key else { return Ok(false) };
        let name = match table {
            TABLE_TRANSCRIPTION => format!("{}{key}", RecordKind::Dictation.prefix()),
            TABLE_DICTIONARY => RecordId::dictionary_entry(&normalized_key(key)).name,
            TABLE_SNIPPET => RecordId::snippet(&normalized_key(key)).name,
            TABLE_SUGGESTION => RecordId::learning_suggestion(key).name,
            TABLE_INSIGHT => RecordId::writing_insight(key).name,
            _ => return Ok(false),
        };
        self.tombstone(&name)
    }

    /// Marks a live record deleted at a new revision. False if there's none.
    fn tombstone(&self, name: &str) -> Result<bool> {
        let Some(existing) = self.record(name)? else {
            return Ok(false);
        };
        if existing.is_deleted {
            return Ok(false);
        }
        let revision = self.next_revision()?;
        self.conn().execute(
            "UPDATE sync_record SET rev_counter = ?2, rev_modified_ms = ?3, rev_writer = ?4, is_deleted = 1,
                deleted_at_ms = ?3, local_table = NULL, local_id = NULL, payload = NULL WHERE record_name = ?1",
            params![name, revision.counter, revision.modified_ms, revision.writer],
        )?;
        self.enqueue(name, &revision)?;
        Ok(true)
    }

    fn save_local(
        &self,
        id: &RecordId,
        table: &str,
        local_id: i64,
        generation: i64,
        origin: Option<&Origin>,
        created_at_ms: Option<i64>,
    ) -> Result<()> {
        let revision = self.next_revision()?;
        let schema = Envelope::required_schema_version(id, None, origin);
        self.conn().execute(
            "INSERT INTO sync_record (record_name, zone, kind, local_table, local_id, rev_counter, rev_modified_ms, rev_writer,
                generation, is_deleted, deleted_at_ms, origin_device_id, origin_kind, created_at_ms, schema_version, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL, ?10, ?11, ?12, ?13, NULL)
             ON CONFLICT(record_name) DO UPDATE SET local_table = excluded.local_table, local_id = excluded.local_id,
                rev_counter = excluded.rev_counter, rev_modified_ms = excluded.rev_modified_ms, rev_writer = excluded.rev_writer,
                generation = excluded.generation, is_deleted = 0, deleted_at_ms = NULL,
                origin_device_id = COALESCE(sync_record.origin_device_id, excluded.origin_device_id),
                origin_kind = COALESCE(sync_record.origin_kind, excluded.origin_kind),
                created_at_ms = COALESCE(sync_record.created_at_ms, excluded.created_at_ms)",
            params![
                id.name,
                id.zone.name(),
                id.kind().map(|k| k.as_str()).unwrap_or(""),
                table,
                local_id,
                revision.counter,
                revision.modified_ms,
                revision.writer,
                generation,
                origin.and_then(|o| o.device_id.clone()),
                origin.map(|o| o.kind.as_str()),
                created_at_ms,
                schema,
            ],
        )?;
        self.enqueue(&id.name, &revision)
    }

    fn enqueue(&self, name: &str, revision: &Revision) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_outbox (record_name, rev_counter, rev_modified_ms, rev_writer, failed_reason, enqueued_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)
             ON CONFLICT(record_name) DO UPDATE SET rev_counter = excluded.rev_counter,
                rev_modified_ms = excluded.rev_modified_ms, rev_writer = excluded.rev_writer,
                failed_reason = NULL, enqueued_at = excluded.enqueued_at",
            params![name, revision.counter, revision.modified_ms, revision.writer, now_unix()],
        )?;
        Ok(())
    }

    fn record(&self, name: &str) -> Result<Option<RecordRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {} FROM sync_record WHERE record_name = ?1",
                    RecordRow::COLUMNS
                ),
                [name],
                RecordRow::from_row,
            )
            .optional()?)
    }

    // ---- This PC's own device record -------------------------------------------

    /// Queues this PC's device record (name, versions, its own totals) when
    /// it changed, or always with `force`.
    pub fn sync_publish_device(&self, device: &DeviceInfo) -> Result<bool> {
        self.publish_device(device, false)
    }

    fn publish_device(&self, device: &DeviceInfo, force: bool) -> Result<bool> {
        let me = self.sync_device_id()?;
        let totals = self.device_totals(&me)?;
        let fingerprint = format!(
            "{}|{}|{}|{}|{}|{}",
            device.name,
            device.app_version,
            device.os_version,
            totals.dictations,
            totals.words,
            totals.recording_ms
        );
        if !force && self.sync_get("device_published")?.as_deref() == Some(fingerprint.as_str()) {
            return Ok(false);
        }
        let id = RecordId::device(&me);
        let now = now_ms();
        let first_seen = self
            .sync_get("first_seen_ms")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(now);
        self.sync_set("first_seen_ms", Some(&first_seen.to_string()))?;
        let payload = encode_payload(&DevicePayload {
            device_id: me.clone(),
            kind: DeviceKind::this_platform(),
            name: device.name.clone(),
            model: device.model.clone(),
            app: "Orttaai".into(),
            app_version: Some(device.app_version.clone()),
            os_version: Some(device.os_version.clone()),
            first_seen_at: first_seen,
            last_seen_at: now,
            totals,
        })?;
        let revision = self.next_revision()?;
        self.conn().execute(
            "INSERT INTO sync_record (record_name, zone, kind, rev_counter, rev_modified_ms, rev_writer, schema_version, payload)
             VALUES (?1, 'Library', 'Device', ?2, ?3, ?4, 2, ?5)
             ON CONFLICT(record_name) DO UPDATE SET rev_counter = excluded.rev_counter, rev_modified_ms = excluded.rev_modified_ms,
                rev_writer = excluded.rev_writer, is_deleted = 0, payload = excluded.payload",
            params![id.name, revision.counter, revision.modified_ms, revision.writer, payload],
        )?;
        self.enqueue(&id.name, &revision)?;
        self.sync_set("device_published", Some(&fingerprint))?;
        Ok(true)
    }

    /// A device's own dictations, words and time (edits not counted).
    pub fn device_totals(&self, device_id: &str) -> Result<DeviceTotals> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(word_count), 0), COALESCE(SUM(recording_ms), 0), MIN(created_at), MAX(created_at)
             FROM transcription WHERE source_device_id = ?1 AND entry_kind != 'edit'",
            [device_id],
            |r| {
                Ok(DeviceTotals {
                    dictations: r.get(0)?,
                    words: r.get(1)?,
                    recording_ms: r.get(2)?,
                    first_at: r.get::<_, Option<i64>>(3)?.map(|s| s * 1000),
                    last_at: r.get::<_, Option<i64>>(4)?.map(|s| s * 1000),
                })
            },
        )?)
    }

    // ---- Sending -------------------------------------------------------------

    /// Up to `limit` records waiting for the Mac, as they are now.
    pub fn sync_outgoing(&self, limit: usize) -> Result<Vec<Envelope>> {
        let names: Vec<String> = {
            let mut stmt = self.conn().prepare(
                "SELECT record_name FROM sync_outbox WHERE failed_reason IS NULL ORDER BY enqueued_at, rev_counter LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit as i64], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            match self.current_envelope(&name)? {
                Some(envelope) => match envelope.problem() {
                    None => out.push(envelope),
                    Some(problem) => self.sync_mark_failed(&name, &problem)?,
                },
                // Nothing left to send (its content is gone).
                None => {
                    self.conn()
                        .execute("DELETE FROM sync_outbox WHERE record_name = ?1", [&name])?;
                }
            }
        }
        Ok(out)
    }

    /// The current local version of a record.
    fn current_envelope(&self, name: &str) -> Result<Option<Envelope>> {
        let Some(row) = self.record(name)? else {
            return Ok(None);
        };
        let Some(zone) = Zone::parse(&row.zone) else {
            return Ok(None);
        };
        let id = RecordId {
            zone,
            name: name.to_string(),
        };
        let payload = if row.is_deleted {
            None
        } else {
            match (&row.local_table, row.local_id) {
                (Some(table), Some(local_id)) => match self.content_payload(table, local_id)? {
                    Some(p) => Some(p),
                    None => return Ok(None),
                },
                _ => match row.payload.clone() {
                    Some(p) => Some(p),
                    None => return Ok(None),
                },
            }
        };
        let origin = if id.kind() == Some(RecordKind::Dictation) {
            row.origin()
        } else {
            None
        };
        Ok(Some(Envelope {
            schema_version: Envelope::required_schema_version(
                &id,
                payload.as_deref(),
                origin.as_ref(),
            ),
            id,
            revision: row.revision,
            generation: row.generation,
            is_deleted: row.is_deleted,
            deleted_at_ms: row.deleted_at_ms,
            payload,
            origin,
            created_at_ms: row.created_at_ms,
        }))
    }

    fn content_payload(&self, table: &str, local_id: i64) -> Result<Option<Vec<u8>>> {
        let c = self.conn();
        let payload = match table {
            TABLE_TRANSCRIPTION => c
                .query_row(
                    "SELECT created_at, text, raw_text, app_name, app_id, model_id, recording_ms, processing_ms,
                        transcription_ms, text_processing_ms, injection_ms, audio_device, injection_method,
                        entry_kind, edit_instruction, time_zone FROM transcription WHERE id = ?1",
                    [local_id],
                    |r| {
                        let text: String = r.get(1)?;
                        let raw: Option<String> = r.get(2)?;
                        let model: String = r.get(5)?;
                        let kind: String = r.get(13)?;
                        Ok(DictationPayload {
                            created_at: r.get::<_, i64>(0)? * 1000,
                            raw_text: raw.filter(|raw| *raw != text),
                            text,
                            target_app_name: r.get(3)?,
                            target_app_bundle_id: r.get(4)?,
                            model_id: (!model.is_empty()).then_some(model),
                            recording_ms: Some(r.get(6)?),
                            processing_ms: Some(r.get(7)?),
                            transcription_ms: r.get(8)?,
                            text_processing_ms: r.get(9)?,
                            injection_ms: r.get(10)?,
                            audio_device: r.get(11)?,
                            injection_method: r.get(12)?,
                            entry_kind: (kind == "edit").then_some(kind),
                            edit_instruction: r.get(14)?,
                            time_zone_identifier: r.get(15)?,
                            ..Default::default()
                        })
                    },
                )
                .optional()?
                .map(|p| encode_payload(&p))
                .transpose()?,
            TABLE_DICTIONARY => c
                .query_row(
                    "SELECT source, target, is_case_sensitive, is_active, created_at FROM dictionary_entry WHERE id = ?1",
                    [local_id],
                    |r| {
                        let source: String = r.get(0)?;
                        Ok(DictionaryEntryPayload {
                            normalized_source: normalized_key(&source),
                            source,
                            target: r.get(1)?,
                            is_case_sensitive: r.get(2)?,
                            is_active: r.get(3)?,
                            created_at: r.get::<_, i64>(4)? * 1000,
                        })
                    },
                )
                .optional()?
                .map(|p| encode_payload(&p))
                .transpose()?,
            TABLE_SNIPPET => c
                .query_row(
                    "SELECT trigger, expansion, is_active, created_at FROM snippet_entry WHERE id = ?1",
                    [local_id],
                    |r| {
                        let trigger: String = r.get(0)?;
                        Ok(SnippetPayload {
                            normalized_trigger: normalized_key(&trigger),
                            trigger,
                            expansion: r.get(1)?,
                            is_active: r.get(2)?,
                            created_at: r.get::<_, i64>(3)? * 1000,
                        })
                    },
                )
                .optional()?
                .map(|p| encode_payload(&p))
                .transpose()?,
            TABLE_SUGGESTION => c
                .query_row(
                    "SELECT kind, candidate_source, candidate_target, normalized_source, confidence, status, evidence, created_at
                     FROM learning_suggestion WHERE id = ?1",
                    [local_id],
                    |r| {
                        Ok(LearningSuggestionPayload {
                            kind: r.get(0)?,
                            candidate_source: r.get(1)?,
                            candidate_target: r.get(2)?,
                            normalized_source: r.get(3)?,
                            confidence: r.get(4)?,
                            status: r.get(5)?,
                            evidence: r.get(6)?,
                            created_at: r.get::<_, i64>(7)? * 1000,
                        })
                    },
                )
                .optional()?
                .map(|p| encode_payload(&p))
                .transpose()?,
            TABLE_INSIGHT => c
                .query_row(
                    "SELECT generated_at, analyzer_name, used_fallback, is_pinned, sample_count, request_json, snapshot_json
                     FROM writing_insight_snapshot WHERE id = ?1",
                    [local_id],
                    |r| {
                        Ok(WritingInsightPayload {
                            generated_at: r.get::<_, i64>(0)? * 1000,
                            analyzer_name: r.get(1)?,
                            used_fallback: r.get(2)?,
                            is_pinned: r.get(3)?,
                            sample_count: r.get(4)?,
                            request_json: r.get(5)?,
                            snapshot_json: r.get(6)?,
                        })
                    },
                )
                .optional()?
                .map(|p| encode_payload(&p))
                .transpose()?,
            _ => None,
        };
        Ok(payload)
    }

    /// The Mac saved this revision: its outbox entry is done (a newer local
    /// revision stays).
    pub fn sync_acknowledge(&self, name: &str, revision: &Revision) -> Result<()> {
        self.conn().execute(
            "DELETE FROM sync_outbox WHERE record_name = ?1 AND rev_counter = ?2 AND rev_modified_ms = ?3 AND rev_writer = ?4",
            params![name, revision.counter, revision.modified_ms, revision.writer],
        )?;
        Ok(())
    }

    /// The Mac refused it; kept, and shown, until it changes again.
    pub fn sync_mark_failed(&self, name: &str, reason: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE sync_outbox SET failed_reason = ?2 WHERE record_name = ?1",
            params![name, reason],
        )?;
        Ok(())
    }

    pub fn sync_counts(&self) -> Result<SyncCounts> {
        Ok(self.conn().query_row(
            "SELECT (SELECT COUNT(*) FROM sync_outbox WHERE failed_reason IS NULL) + (SELECT COUNT(*) FROM sync_pending),
                    (SELECT COUNT(*) FROM sync_outbox WHERE failed_reason IS NOT NULL),
                    (SELECT failed_reason FROM sync_outbox WHERE failed_reason IS NOT NULL LIMIT 1)",
            [],
            |r| Ok(SyncCounts { waiting: r.get(0)?, failed: r.get(1)?, last_failure: r.get(2)? }),
        )?)
    }

    // ---- Receiving -------------------------------------------------------------

    /// Applies records from the Mac in one transaction, with the conflict
    /// policy every device uses.
    pub fn sync_apply(&self, envelopes: &[Envelope]) -> Result<ApplyResult> {
        let mut result = ApplyResult::default();
        if envelopes.is_empty() {
            return Ok(result);
        }
        self.conn().execute_batch("BEGIN IMMEDIATE")?;
        let capture = self.sync_get("capture")?.as_deref() == Some("1");
        let outcome = (|| -> Result<()> {
            self.sync_capture(false)?;
            for envelope in envelopes {
                if envelope.problem().is_some() {
                    result.skipped += 1;
                    continue;
                }
                self.observe(&envelope.revision)?;
                let existing = self.record(&envelope.id.name)?;
                match decide(
                    existing.as_ref().map(|r| r.local_state()).as_ref(),
                    envelope,
                ) {
                    Decision::AcceptRemote => {
                        let local_id = self.apply_content(envelope, existing.as_ref())?;
                        self.store_remote(envelope, local_id)?;
                        self.conn().execute(
                            "DELETE FROM sync_outbox WHERE record_name = ?1",
                            [&envelope.id.name],
                        )?;
                        result.changed += 1;
                        if let Some(kind) = envelope.kind() {
                            if !result.kinds.contains(&kind) {
                                result.kinds.push(kind);
                            }
                        }
                    }
                    Decision::KeepLocal => {
                        if let Some(local) = &existing {
                            self.enqueue(&envelope.id.name, &local.revision)?;
                        }
                    }
                    Decision::AlreadyCurrent => {
                        self.sync_acknowledge(&envelope.id.name, &envelope.revision)?;
                    }
                }
            }
            Ok(())
        })();
        let restored = self.sync_capture(capture);
        self.finish(outcome)?;
        restored?;
        Ok(result)
    }

    fn store_remote(&self, e: &Envelope, local: Option<(&str, i64)>) -> Result<()> {
        let keeps_payload = local.is_none() && !e.is_deleted;
        self.conn().execute(
            "INSERT INTO sync_record (record_name, zone, kind, local_table, local_id, rev_counter, rev_modified_ms, rev_writer,
                generation, is_deleted, deleted_at_ms, origin_device_id, origin_kind, created_at_ms, schema_version, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(record_name) DO UPDATE SET zone = excluded.zone, local_table = excluded.local_table,
                local_id = excluded.local_id, rev_counter = excluded.rev_counter, rev_modified_ms = excluded.rev_modified_ms,
                rev_writer = excluded.rev_writer, generation = excluded.generation, is_deleted = excluded.is_deleted,
                deleted_at_ms = excluded.deleted_at_ms, origin_device_id = COALESCE(excluded.origin_device_id, sync_record.origin_device_id),
                origin_kind = COALESCE(excluded.origin_kind, sync_record.origin_kind),
                created_at_ms = COALESCE(sync_record.created_at_ms, excluded.created_at_ms),
                schema_version = excluded.schema_version, payload = excluded.payload",
            params![
                e.id.name,
                e.id.zone.name(),
                e.kind().map(|k| k.as_str()).unwrap_or(""),
                local.map(|l| l.0),
                local.map(|l| l.1),
                e.revision.counter,
                e.revision.modified_ms,
                e.revision.writer,
                e.generation,
                e.is_deleted,
                e.deleted_at_ms,
                e.origin.as_ref().and_then(|o| o.device_id.clone()),
                e.origin.as_ref().map(|o| o.kind.as_str()),
                e.created_at_ms,
                e.schema_version,
                if keeps_payload { e.payload.clone() } else { None },
            ],
        )?;
        Ok(())
    }

    /// Writes a record's content into its table. Returns the row it lives
    /// in, or `None` for records kept only in the ledger.
    fn apply_content(
        &self,
        e: &Envelope,
        existing: Option<&RecordRow>,
    ) -> Result<Option<(&'static str, i64)>> {
        let Some(kind) = e.kind() else {
            return Ok(None);
        };
        let existing_local = existing.and_then(|r| r.local_id);
        let c = self.conn();
        let now = now_unix();

        if e.is_deleted {
            match kind {
                RecordKind::Dictation => {
                    let sync_id = e.id.local_key().to_string();
                    let id: Option<i64> = c
                        .query_row(
                            "SELECT id FROM transcription WHERE sync_id = ?1",
                            [&sync_id],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if let Some(id) = id {
                        self.delete_transcription(id)?;
                    }
                }
                RecordKind::DictionaryEntry
                | RecordKind::Snippet
                | RecordKind::LearningSuggestion
                | RecordKind::WritingInsight => {
                    if let Some(id) = existing_local {
                        let table = table_for(kind);
                        c.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
                    }
                }
                _ => {}
            }
            return Ok(None);
        }

        let payload = e.payload.as_deref().unwrap_or_default();
        match kind {
            RecordKind::Dictation => {
                let p: DictationPayload = serde_json::from_slice(payload)?;
                let sync_id = e.id.local_key().to_string();
                let origin = e.origin.clone().unwrap_or(Origin {
                    device_id: None,
                    kind: DeviceKind::Unknown,
                });
                let entry_kind = p.entry_kind.clone().unwrap_or_else(|| "dictation".into());
                let found: Option<i64> = c
                    .query_row(
                        "SELECT id FROM transcription WHERE sync_id = ?1",
                        [&sync_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let values = params![
                    p.created_at / 1000,
                    p.text,
                    p.raw_text,
                    p.target_app_name,
                    p.target_app_bundle_id,
                    p.recording_ms.unwrap_or(0),
                    p.processing_ms.or(p.transcription_ms).unwrap_or(0),
                    p.transcription_ms,
                    p.text_processing_ms,
                    p.injection_ms,
                    p.model_id.clone().unwrap_or_default(),
                    p.audio_device,
                    p.injection_method,
                    entry_kind,
                    p.edit_instruction,
                    word_count(&p.text),
                    p.time_zone_identifier,
                    sync_id,
                    origin.device_id,
                    origin.kind.as_str(),
                ];
                let id = match found {
                    Some(id) => {
                        c.execute(
                            "UPDATE transcription SET created_at = ?1, text = ?2, raw_text = ?3, app_name = ?4, app_id = ?5,
                                recording_ms = ?6, processing_ms = ?7, transcription_ms = ?8, text_processing_ms = ?9,
                                injection_ms = ?10, model_id = ?11, audio_device = ?12, injection_method = ?13,
                                entry_kind = ?14, edit_instruction = ?15, word_count = ?16, time_zone = ?17,
                                source_device_id = COALESCE(source_device_id, ?19), source_device_kind = COALESCE(source_device_kind, ?20)
                             WHERE sync_id = ?18",
                            values,
                        )?;
                        id
                    }
                    None => {
                        c.execute(
                            "INSERT INTO transcription (created_at, text, raw_text, app_name, app_id, recording_ms, processing_ms,
                                transcription_ms, text_processing_ms, injection_ms, model_id, audio_device, injection_method,
                                entry_kind, edit_instruction, word_count, time_zone, sync_id, source_device_id, source_device_kind)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                            values,
                        )?;
                        c.last_insert_rowid()
                    }
                };
                // Indexed for Memory Graph and ChatAI like a dictation said here.
                c.execute(
                    "INSERT OR IGNORE INTO semantic_index_queue (transcription_id, enqueued_at, attempts)
                     SELECT id, ?2, 0 FROM transcription WHERE id = ?1 AND entry_kind != 'edit' AND length(trim(text)) > 0",
                    params![id, now],
                )?;
                Ok(Some((TABLE_TRANSCRIPTION, id)))
            }
            RecordKind::DictionaryEntry => {
                let p: DictionaryEntryPayload = serde_json::from_slice(payload)?;
                let key = normalized_key(&p.source);
                let id = self.upsert_keyed(
                    TABLE_DICTIONARY,
                    "normalized_source",
                    &key,
                    existing_local,
                    "UPDATE dictionary_entry SET source = ?2, target = ?3, normalized_source = ?4, is_case_sensitive = ?5,
                        is_active = ?6, updated_at = ?7 WHERE id = ?1",
                    "INSERT INTO dictionary_entry (source, target, normalized_source, is_case_sensitive, is_active, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (&p.source, &p.target, &key, p.is_case_sensitive, p.is_active, p.created_at / 1000, now),
                )?;
                Ok(Some((TABLE_DICTIONARY, id)))
            }
            RecordKind::Snippet => {
                let p: SnippetPayload = serde_json::from_slice(payload)?;
                let key = normalized_key(&p.trigger);
                let id = self.upsert_keyed(
                    TABLE_SNIPPET,
                    "normalized_trigger",
                    &key,
                    existing_local,
                    "UPDATE snippet_entry SET trigger = ?2, expansion = ?3, normalized_trigger = ?4, is_active = ?6,
                        updated_at = ?7 WHERE id = ?1",
                    "INSERT INTO snippet_entry (trigger, expansion, normalized_trigger, is_active, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?5, ?6, ?7)",
                    (&p.trigger, &p.expansion, &key, false, p.is_active, p.created_at / 1000, now),
                )?;
                Ok(Some((TABLE_SNIPPET, id)))
            }
            RecordKind::LearningSuggestion => {
                let p: LearningSuggestionPayload = serde_json::from_slice(payload)?;
                let sync_id = e.id.local_key().to_string();
                let found: Option<i64> = c
                    .query_row(
                        "SELECT id FROM learning_suggestion WHERE sync_id = ?1
                         OR (kind = ?2 AND normalized_source = ?3 AND candidate_target = ?4) ORDER BY sync_id IS NULL LIMIT 1",
                        params![sync_id, p.kind, p.normalized_source, p.candidate_target],
                        |r| r.get(0),
                    )
                    .optional()?;
                let id = match found {
                    Some(id) => {
                        c.execute(
                            "UPDATE learning_suggestion SET kind = ?2, candidate_source = ?3, candidate_target = ?4,
                                normalized_source = ?5, confidence = ?6, status = ?7, evidence = ?8, updated_at = ?9, sync_id = ?10
                             WHERE id = ?1",
                            params![id, p.kind, p.candidate_source, p.candidate_target, p.normalized_source, p.confidence, p.status, p.evidence, now, sync_id],
                        )?;
                        id
                    }
                    None => {
                        c.execute(
                            "INSERT INTO learning_suggestion (kind, candidate_source, candidate_target, normalized_source,
                                confidence, status, evidence, created_at, updated_at, sync_id)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            params![p.kind, p.candidate_source, p.candidate_target, p.normalized_source, p.confidence, p.status, p.evidence, p.created_at / 1000, now, sync_id],
                        )?;
                        c.last_insert_rowid()
                    }
                };
                Ok(Some((TABLE_SUGGESTION, id)))
            }
            RecordKind::WritingInsight => {
                let p: WritingInsightPayload = serde_json::from_slice(payload)?;
                let sync_id = e.id.local_key().to_string();
                let found: Option<i64> = c
                    .query_row(
                        "SELECT id FROM writing_insight_snapshot WHERE sync_id = ?1",
                        [&sync_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let id = match found {
                    Some(id) => {
                        c.execute(
                            "UPDATE writing_insight_snapshot SET generated_at = ?2, analyzer_name = ?3, used_fallback = ?4,
                                is_pinned = ?5, sample_count = ?6, request_json = ?7, snapshot_json = ?8 WHERE id = ?1",
                            params![id, p.generated_at / 1000, p.analyzer_name, p.used_fallback, p.is_pinned, p.sample_count, p.request_json, p.snapshot_json],
                        )?;
                        id
                    }
                    None => {
                        c.execute(
                            "INSERT INTO writing_insight_snapshot (kind, generated_at, analyzer_name, used_fallback, is_pinned,
                                sample_count, request_json, snapshot_json, sync_id)
                             VALUES ('writing', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![p.generated_at / 1000, p.analyzer_name, p.used_fallback, p.is_pinned, p.sample_count, p.request_json, p.snapshot_json, sync_id],
                        )?;
                        c.last_insert_rowid()
                    }
                };
                Ok(Some((TABLE_INSIGHT, id)))
            }
            // Devices, the Mac's summary, chats and Mac settings stay in the
            // ledger as they came.
            _ => Ok(None),
        }
    }

    /// Updates the row a word or snippet lives in (found by its record, or by
    /// its key), or inserts it.
    #[allow(clippy::too_many_arguments)]
    fn upsert_keyed(
        &self,
        table: &str,
        key_column: &str,
        key: &str,
        existing_local: Option<i64>,
        update_sql: &str,
        insert_sql: &str,
        v: (&String, &String, &String, bool, bool, i64, i64),
    ) -> Result<i64> {
        let c = self.conn();
        let found: Option<i64> = match existing_local {
            Some(id) => c
                .query_row(
                    &format!("SELECT id FROM {table} WHERE id = ?1"),
                    [id],
                    |r| r.get(0),
                )
                .optional()?,
            None => None,
        };
        let found = match found {
            Some(id) => Some(id),
            None => c
                .query_row(
                    &format!("SELECT id FROM {table} WHERE {key_column} = ?1"),
                    [key],
                    |r| r.get(0),
                )
                .optional()?,
        };
        match found {
            Some(id) => {
                c.execute(update_sql, params![id, v.0, v.1, v.2, v.3, v.4, v.6])?;
                Ok(id)
            }
            None => {
                c.execute(insert_sql, params![v.0, v.1, v.2, v.3, v.4, v.5, v.6])?;
                Ok(c.last_insert_rowid())
            }
        }
    }

    // ---- For screens -------------------------------------------------------------

    /// Device records this PC knows (its own and those from the Mac), by ID.
    pub fn sync_devices(&self) -> Result<HashMap<String, DevicePayload>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT payload FROM sync_record WHERE kind = 'Device' AND is_deleted = 0 AND payload IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        let mut devices = HashMap::new();
        for payload in rows {
            if let Ok(device) = serde_json::from_slice::<DevicePayload>(&payload?) {
                devices.insert(device.device_id.clone(), device);
            }
        }
        Ok(devices)
    }
}

/// What this PC says about itself in its device record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub model: Option<String>,
    pub app_version: String,
    pub os_version: String,
}

fn table_for(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Dictation => TABLE_TRANSCRIPTION,
        RecordKind::DictionaryEntry => TABLE_DICTIONARY,
        RecordKind::Snippet => TABLE_SNIPPET,
        RecordKind::LearningSuggestion => TABLE_SUGGESTION,
        RecordKind::WritingInsight => TABLE_INSIGHT,
        _ => "",
    }
}
