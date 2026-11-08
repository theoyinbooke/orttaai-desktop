//! Persistence via SQLite (`rusqlite`, bundled). Mirrors the macOS GRDB schema so
//! analytics/insights logic ports directly. Versioned migrations live in
//! [`schema`] and are applied on open.
//!
//! Feature areas extend [`Store`] with their own `impl Store` blocks in sibling
//! files (`ai.rs`, `semantic.rs`, `coach.rs`, `analytics.rs`, `learning.rs`) so the
//! core CRUD here stays small.

mod ai;
mod analytics;
mod coach;
mod learning;
pub mod schema;
mod semantic;

#[allow(unused_imports)]
pub use ai::*;
pub use analytics::*;
#[allow(unused_imports)]
pub use coach::*;
#[allow(unused_imports)]
pub use learning::*;
#[allow(unused_imports)]
pub use semantic::*;

use crate::error::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current Unix time in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// SQLITE_BUSY / SQLITE_LOCKED — another connection holds the lock.
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Lowercased, trimmed, inner-whitespace-collapsed key used for matching.
pub fn normalize_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A single dictation, mirroring the macOS `TranscriptionRecord`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscriptionRecord {
    pub id: Option<i64>,
    /// Final text that was typed.
    pub text: String,
    /// What the recognizer produced before cleanup/polish.
    pub raw_text: Option<String>,
    /// Focused app name (e.g. "firefox"); `None` when it can't be detected.
    pub app: Option<String>,
    pub app_id: Option<String>,
    /// How long the user spoke, in ms.
    pub duration_ms: i64,
    /// Release → text typed, in ms.
    pub processing_ms: i64,
    pub transcription_ms: Option<i64>,
    pub text_processing_ms: Option<i64>,
    pub polish_ms: Option<i64>,
    pub injection_ms: Option<i64>,
    pub model_id: String,
    pub audio_device: Option<String>,
    /// `typed | paste | portal | clipboard | failed | blocked`.
    pub injection_method: Option<String>,
    /// `dictation | edit`.
    pub entry_kind: String,
    pub edit_instruction: Option<String>,
    pub word_count: i64,
    pub time_zone: Option<String>,
    pub polished: bool,
    /// Unix seconds.
    pub created_at: i64,
}

impl TranscriptionRecord {
    /// Build a record from text + timing; computes the word count.
    pub fn new(
        text: impl Into<String>,
        app: Option<String>,
        duration_ms: i64,
        created_at: i64,
    ) -> Self {
        let text = text.into();
        let word_count = text.split_whitespace().count() as i64;
        Self {
            text,
            app,
            duration_ms,
            word_count,
            created_at,
            entry_kind: "dictation".to_string(),
            ..Default::default()
        }
    }

    const COLUMNS: &'static str = "id, text, raw_text, app_name, app_id, recording_ms, processing_ms, \
        transcription_ms, text_processing_ms, polish_ms, injection_ms, model_id, audio_device, \
        injection_method, entry_kind, edit_instruction, word_count, time_zone, polished, created_at";

    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: Some(r.get(0)?),
            text: r.get(1)?,
            raw_text: r.get(2)?,
            app: r.get(3)?,
            app_id: r.get(4)?,
            duration_ms: r.get(5)?,
            processing_ms: r.get(6)?,
            transcription_ms: r.get(7)?,
            text_processing_ms: r.get(8)?,
            polish_ms: r.get(9)?,
            injection_ms: r.get(10)?,
            model_id: r.get(11)?,
            audio_device: r.get(12)?,
            injection_method: r.get(13)?,
            entry_kind: r.get(14)?,
            edit_instruction: r.get(15)?,
            word_count: r.get(16)?,
            time_zone: r.get(17)?,
            polished: r.get(18)?,
            created_at: r.get(19)?,
        })
    }
}

/// Filter for paging through history.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HistoryQuery {
    /// Case-insensitive substring search over the text.
    pub search: Option<String>,
    /// Exact app-name filter.
    pub app: Option<String>,
    /// `dictation | edit`.
    pub entry_kind: Option<String>,
    /// Only records at/after this Unix second.
    pub since: Option<i64>,
    pub limit: i64,
    pub offset: i64,
}

/// One page of history plus the total matching count.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub items: Vec<TranscriptionRecord>,
    pub total: i64,
}

/// A dictionary replacement (Mac `DictionaryEntry`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub id: i64,
    pub source: String,
    pub target: String,
    pub is_case_sensitive: bool,
    pub is_active: bool,
    pub usage_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A snippet expansion (Mac `SnippetEntry`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetEntry {
    pub id: i64,
    pub trigger: String,
    pub expansion: String,
    pub is_active: bool,
    pub usage_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Legacy combined view kept for the CLI/tests (`kind` is `"dictionary"` or `"snippet"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Option<i64>,
    pub kind: String,
    pub trigger: String,
    pub replacement: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Platform data directory (`~/.local/share/orttaai` on Linux,
    /// `%APPDATA%\orttaai` on Windows). Overridable with `ORTTAAI_DATA_DIR`.
    pub fn data_dir() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os("ORTTAAI_DATA_DIR") {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("org", "orttaai", "Orttaai")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .ok_or_else(|| std::io::Error::other("no data directory available").into())
    }

    /// Open an in-memory database (tests, ephemeral use).
    pub fn open_in_memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory()?,
        };
        store.configure()?;
        store.migrate()?;
        Ok(store)
    }

    /// Open (or create) the history database at the platform data directory.
    pub fn open_default() -> Result<Self> {
        Self::open(&Self::data_dir()?.join("history.db"))
    }

    /// Open (or create) a database file, creating parent directories as needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // First launch opens the database from several commands at once; WAL
        // setup and migrations can briefly report "database is locked" (SQLite
        // doesn't always run the busy handler for those), so retry for up to ~5 s.
        let mut attempt = 0;
        loop {
            let opened = (|| -> Result<Self> {
                let store = Self {
                    conn: Connection::open(path)?,
                };
                store.configure()?;
                store.migrate()?;
                Ok(store)
            })();
            match opened {
                Err(crate::error::CoreError::Db(ref e)) if is_busy(e) && attempt < 50 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                other => return other,
            }
        }
    }

    fn configure(&self) -> Result<()> {
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL lets the UI read while the dictation worker writes.
        let _: String = self
            .conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        let latest = schema::MIGRATIONS.len() as i64;
        let current = |c: &Connection| -> Result<i64> {
            Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
        };
        if current(&self.conn)? >= latest {
            return Ok(());
        }
        // Several commands open the database at once on first launch. Take the
        // write lock *before* reading the version (BEGIN IMMEDIATE waits out the
        // busy timeout), so exactly one connection migrates and the others then
        // see the new version instead of re-running CREATE TABLE.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let version = current(&self.conn)?;
            for (i, sql) in schema::MIGRATIONS.iter().enumerate() {
                let target = i as i64 + 1;
                if version >= target {
                    continue;
                }
                self.conn.execute_batch(sql)?;
                self.conn
                    .execute_batch(&format!("PRAGMA user_version = {target}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Schema version after migrations (for diagnostics/tests).
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Raw connection for feature modules' `impl Store` blocks.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    // ---- Transcriptions ----------------------------------------------------

    /// Insert a record, returning its new row id.
    pub fn insert_transcription(&self, r: &TranscriptionRecord) -> Result<i64> {
        let word_count = if r.word_count > 0 {
            r.word_count
        } else {
            r.text.split_whitespace().count() as i64
        };
        let kind = if r.entry_kind.is_empty() {
            "dictation"
        } else {
            r.entry_kind.as_str()
        };
        self.conn.execute(
            "INSERT INTO transcription (text, raw_text, app_name, app_id, recording_ms, processing_ms,
                transcription_ms, text_processing_ms, polish_ms, injection_ms, model_id, audio_device,
                injection_method, entry_kind, edit_instruction, word_count, time_zone, polished, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                r.text,
                r.raw_text,
                r.app,
                r.app_id,
                r.duration_ms,
                r.processing_ms,
                r.transcription_ms,
                r.text_processing_ms,
                r.polish_ms,
                r.injection_ms,
                r.model_id,
                r.audio_device,
                r.injection_method,
                kind,
                r.edit_instruction,
                word_count,
                r.time_zone,
                r.polished,
                r.created_at
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_transcription(&self, id: i64) -> Result<Option<TranscriptionRecord>> {
        let sql = format!(
            "SELECT {} FROM transcription WHERE id = ?1",
            TranscriptionRecord::COLUMNS
        );
        Ok(self
            .conn
            .query_row(&sql, [id], TranscriptionRecord::from_row)
            .optional()?)
    }

    /// Most-recent records, newest first.
    pub fn recent(&self, limit: i64) -> Result<Vec<TranscriptionRecord>> {
        let sql = format!(
            "SELECT {} FROM transcription ORDER BY created_at DESC, id DESC LIMIT ?1",
            TranscriptionRecord::COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([limit], TranscriptionRecord::from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Records created at/after `since`, oldest first (for analysis pipelines).
    pub fn transcriptions_since(&self, since: i64, limit: i64) -> Result<Vec<TranscriptionRecord>> {
        let sql = format!(
            "SELECT {} FROM transcription WHERE created_at >= ?1 ORDER BY created_at ASC, id ASC LIMIT ?2",
            TranscriptionRecord::COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, limit], TranscriptionRecord::from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Search/filter/paginate history, newest first.
    pub fn history(&self, q: &HistoryQuery) -> Result<HistoryPage> {
        let mut clauses: Vec<&str> = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(s) = q.search.as_ref().filter(|s| !s.trim().is_empty()) {
            clauses.push("lower(text) LIKE ?");
            args.push(format!("%{}%", s.trim().to_lowercase()).into());
        }
        if let Some(app) = q.app.as_ref().filter(|s| !s.is_empty()) {
            clauses.push("app_name = ?");
            args.push(app.clone().into());
        }
        if let Some(kind) = q.entry_kind.as_ref().filter(|s| !s.is_empty()) {
            clauses.push("entry_kind = ?");
            args.push(kind.clone().into());
        }
        if let Some(since) = q.since {
            clauses.push("created_at >= ?");
            args.push(since.into());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM transcription {where_sql}"),
            rusqlite::params_from_iter(args.iter()),
            |r| r.get(0),
        )?;
        let limit = if q.limit <= 0 { 50 } else { q.limit.min(1000) };
        let sql = format!(
            "SELECT {} FROM transcription {where_sql} ORDER BY created_at DESC, id DESC LIMIT {limit} OFFSET {}",
            TranscriptionRecord::COLUMNS,
            q.offset.max(0)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let items = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter()),
                TranscriptionRecord::from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(HistoryPage { items, total })
    }

    /// Distinct app names seen in history (for filters).
    pub fn history_apps(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT app_name FROM transcription WHERE app_name IS NOT NULL AND app_name != ''
             GROUP BY app_name ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Delete a single transcription by id (and its derived semantic rows).
    pub fn delete_transcription(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM semantic_chunk WHERE transcription_id = ?1",
            params![id],
        )?;
        self.conn.execute(
            "DELETE FROM concept_extraction WHERE transcription_id = ?1",
            params![id],
        )?;
        self.conn.execute(
            "DELETE FROM semantic_index_queue WHERE transcription_id = ?1",
            params![id],
        )?;
        self.conn.execute(
            "DELETE FROM coach_metric WHERE transcription_id = ?1",
            params![id],
        )?;
        self.conn
            .execute("DELETE FROM transcription WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Settings → "Clear History": remove every dictation and derived data.
    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM semantic_embedding; DELETE FROM semantic_signal; DELETE FROM semantic_chunk;
             DELETE FROM semantic_graph_edge; DELETE FROM semantic_graph_node;
             DELETE FROM semantic_insight_snapshot; DELETE FROM insight_finding;
             DELETE FROM concept_extraction; DELETE FROM session_analysis; DELETE FROM semantic_index_queue;
             DELETE FROM coach_metric; DELETE FROM coach_judgement;
             DELETE FROM transcription;",
        )?;
        Ok(())
    }

    /// Settings → "Reset App Data": every table back to empty.
    pub fn reset_all(&self) -> Result<()> {
        self.clear_history()?;
        self.conn.execute_batch(
            "DELETE FROM dictionary_entry; DELETE FROM snippet_entry; DELETE FROM learning_suggestion;
             DELETE FROM edit_check; DELETE FROM writing_insight_snapshot; DELETE FROM chat_message;
             DELETE FROM chat_conversation; DELETE FROM milestone_unlock; DELETE FROM coach_habit;
             DELETE FROM coach_rep; DELETE FROM kv;",
        )?;
        Ok(())
    }

    /// Total number of stored transcriptions.
    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM transcription", [], |r| r.get(0))?)
    }

    // ---- Dictionary --------------------------------------------------------

    pub fn list_dictionary(&self) -> Result<Vec<DictionaryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, target, is_case_sensitive, is_active, usage_count, created_at, updated_at
             FROM dictionary_entry ORDER BY lower(source)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DictionaryEntry {
                id: r.get(0)?,
                source: r.get(1)?,
                target: r.get(2)?,
                is_case_sensitive: r.get(3)?,
                is_active: r.get(4)?,
                usage_count: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Insert or update (by normalized source) a dictionary entry.
    pub fn upsert_dictionary(
        &self,
        source: &str,
        target: &str,
        case_sensitive: bool,
    ) -> Result<i64> {
        let now = now_unix();
        self.conn.execute(
            "INSERT INTO dictionary_entry (source, target, normalized_source, is_case_sensitive, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(normalized_source) DO UPDATE SET
                source = excluded.source, target = excluded.target,
                is_case_sensitive = excluded.is_case_sensitive, is_active = 1, updated_at = excluded.updated_at",
            params![source.trim(), target.trim(), normalize_key(source), case_sensitive, now],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM dictionary_entry WHERE normalized_source = ?1",
            [normalize_key(source)],
            |r| r.get(0),
        )?)
    }

    pub fn update_dictionary(
        &self,
        id: i64,
        source: &str,
        target: &str,
        case_sensitive: bool,
        active: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE dictionary_entry SET source = ?2, target = ?3, normalized_source = ?4,
                is_case_sensitive = ?5, is_active = ?6, updated_at = ?7 WHERE id = ?1",
            params![
                id,
                source.trim(),
                target.trim(),
                normalize_key(source),
                case_sensitive,
                active,
                now_unix()
            ],
        )?;
        Ok(())
    }

    pub fn delete_dictionary(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM dictionary_entry WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- Snippets ----------------------------------------------------------

    pub fn list_snippets(&self) -> Result<Vec<SnippetEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, trigger, expansion, is_active, usage_count, created_at, updated_at
             FROM snippet_entry ORDER BY lower(trigger)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SnippetEntry {
                id: r.get(0)?,
                trigger: r.get(1)?,
                expansion: r.get(2)?,
                is_active: r.get(3)?,
                usage_count: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_snippet(&self, trigger: &str, expansion: &str) -> Result<i64> {
        let now = now_unix();
        self.conn.execute(
            "INSERT INTO snippet_entry (trigger, expansion, normalized_trigger, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(normalized_trigger) DO UPDATE SET
                trigger = excluded.trigger, expansion = excluded.expansion, is_active = 1,
                updated_at = excluded.updated_at",
            params![trigger.trim(), expansion, normalize_key(trigger), now],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM snippet_entry WHERE normalized_trigger = ?1",
            [normalize_key(trigger)],
            |r| r.get(0),
        )?)
    }

    pub fn update_snippet(
        &self,
        id: i64,
        trigger: &str,
        expansion: &str,
        active: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE snippet_entry SET trigger = ?2, expansion = ?3, normalized_trigger = ?4,
                is_active = ?5, updated_at = ?6 WHERE id = ?1",
            params![
                id,
                trigger.trim(),
                expansion,
                normalize_key(trigger),
                active,
                now_unix()
            ],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM snippet_entry WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Bump usage counters for entries that fired during a dictation.
    pub fn record_memory_usage(
        &self,
        dictionary_sources: &[String],
        snippet_triggers: &[String],
    ) -> Result<()> {
        for s in dictionary_sources {
            self.conn.execute(
                "UPDATE dictionary_entry SET usage_count = usage_count + 1 WHERE normalized_source = ?1",
                [normalize_key(s)],
            )?;
        }
        for t in snippet_triggers {
            self.conn.execute(
                "UPDATE snippet_entry SET usage_count = usage_count + 1 WHERE normalized_trigger = ?1",
                [normalize_key(t)],
            )?;
        }
        Ok(())
    }

    // ---- Legacy combined memory API (CLI + tests) --------------------------

    pub fn list_memory(&self) -> Result<Vec<MemoryEntry>> {
        let mut out: Vec<MemoryEntry> = self
            .list_dictionary()?
            .into_iter()
            .map(|d| MemoryEntry {
                id: Some(d.id),
                kind: "dictionary".into(),
                trigger: d.source,
                replacement: d.target,
            })
            .collect();
        out.extend(self.list_snippets()?.into_iter().map(|s| MemoryEntry {
            id: Some(s.id),
            kind: "snippet".into(),
            trigger: s.trigger,
            replacement: s.expansion,
        }));
        Ok(out)
    }

    pub fn add_memory(&self, kind: &str, trigger: &str, replacement: &str) -> Result<i64> {
        if kind == "snippet" {
            self.upsert_snippet(trigger, replacement)
        } else {
            self.upsert_dictionary(trigger, replacement, false)
        }
    }

    /// Build a [`MemoryService`](crate::memory::MemoryService) from the active
    /// stored entries, ready to apply during dictation.
    pub fn load_memory_service(&self) -> Result<crate::memory::MemoryService> {
        let mut service = crate::memory::MemoryService::new();
        for d in self.list_dictionary()?.into_iter().filter(|d| d.is_active) {
            service.add_term_with_case(&d.source, d.target, d.is_case_sensitive);
        }
        for s in self.list_snippets()?.into_iter().filter(|s| s.is_active) {
            service.add_snippet(&s.trigger, s.expansion);
        }
        Ok(service)
    }

    // ---- Key/value ---------------------------------------------------------

    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?
            .flatten())
    }

    pub fn kv_set(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now_unix()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_first_opens_migrate_once() {
        let dir = std::env::temp_dir().join(format!("orttaai-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let p = path.clone();
                std::thread::spawn(move || Store::open(&p).map(|s| s.count().unwrap()))
            })
            .collect();
        for h in handles {
            let r = h.join().unwrap();
            assert!(r.is_ok(), "open must not race: {:?}", r.err());
            assert_eq!(r.unwrap(), 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_fresh_db_to_latest() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), schema::MIGRATIONS.len() as i64);
    }

    #[test]
    fn upgrades_v0_2_database_in_place() {
        let dir = std::env::temp_dir().join(format!("orttaai-mig-{}", std::process::id()));
        let path = dir.join("history.db");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(schema::MIGRATIONS[0]).unwrap();
            c.execute(
                "INSERT INTO transcriptions (text, app, duration_ms, word_count, created_at) VALUES ('hi there', 'code', 1200, 2, 100)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO personal_memory (kind, trigger, replacement) VALUES ('dictionary', 'npm', 'NPM'), ('snippet', 'addr', '1 Main St')",
                [],
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let recs = s.recent(10).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].app.as_deref(), Some("code"));
        assert_eq!(recs[0].duration_ms, 1200);
        assert_eq!(s.list_dictionary().unwrap()[0].target, "NPM");
        assert_eq!(s.list_snippets().unwrap()[0].expansion, "1 Main St");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_search_and_paging() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..30 {
            let app = if i % 2 == 0 {
                Some("code".to_string())
            } else {
                None
            };
            s.insert_transcription(&TranscriptionRecord::new(
                format!("note {i} alpha"),
                app,
                1000,
                1000 + i,
            ))
            .unwrap();
        }
        s.insert_transcription(&TranscriptionRecord::new("Beta release", None, 1000, 5000))
            .unwrap();
        let page = s
            .history(&HistoryQuery {
                search: Some("beta".into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        let page = s
            .history(&HistoryQuery {
                app: Some("code".into()),
                limit: 5,
                offset: 5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 15);
        assert_eq!(page.items.len(), 5);
        assert_eq!(s.history_apps().unwrap(), vec!["code".to_string()]);
    }

    #[test]
    fn dictionary_upsert_is_unique_by_normalized_source() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_dictionary("Open AI", "OpenAI", false).unwrap();
        s.upsert_dictionary("open  ai", "OpenAI Inc", false)
            .unwrap();
        let d = s.list_dictionary().unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].target, "OpenAI Inc");
        s.record_memory_usage(&["OPEN AI".into()], &[]).unwrap();
        assert_eq!(s.list_dictionary().unwrap()[0].usage_count, 1);
    }

    #[test]
    fn kv_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.kv_get("a").unwrap(), None);
        s.kv_set("a", Some("1")).unwrap();
        assert_eq!(s.kv_get("a").unwrap().as_deref(), Some("1"));
    }
}
