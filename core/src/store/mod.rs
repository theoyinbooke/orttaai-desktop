//! Persistence via SQLite (`rusqlite`, bundled). Mirrors the macOS GRDB schema so
//! analytics/insights logic ports directly. Migrations are applied on open.

use crate::error::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single dictation, mirroring the macOS `TranscriptionRecord`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionRecord {
    pub id: Option<i64>,
    pub text: String,
    pub app: Option<String>,
    pub duration_ms: i64,
    pub word_count: i64,
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
            id: None,
            text,
            app,
            duration_ms,
            word_count,
            created_at,
        }
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open an in-memory database (tests, ephemeral use).
    pub fn open_in_memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory()?,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open (or create) the history database at the platform data directory
    /// (`~/.local/share/orttaai` on Linux, `%APPDATA%\orttaai` on Windows).
    pub fn open_default() -> Result<Self> {
        let dir = directories::ProjectDirs::from("org", "orttaai", "Orttaai")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .ok_or_else(|| std::io::Error::other("no data directory available"))?;
        Self::open(&dir.join("history.db"))
    }

    /// Open (or create) a database file, creating parent directories as needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self {
            conn: Connection::open(path)?,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                text        TEXT NOT NULL,
                app         TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                word_count  INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_transcriptions_created_at
                ON transcriptions (created_at DESC);
            CREATE TABLE IF NOT EXISTS personal_memory (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                kind        TEXT NOT NULL,
                trigger     TEXT NOT NULL,
                replacement TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Insert a record, returning its new row id.
    pub fn insert_transcription(&self, record: &TranscriptionRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO transcriptions (text, app, duration_ms, word_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.text,
                record.app,
                record.duration_ms,
                record.word_count,
                record.created_at
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Most-recent records, newest first.
    pub fn recent(&self, limit: i64) -> Result<Vec<TranscriptionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, app, duration_ms, word_count, created_at
             FROM transcriptions ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(TranscriptionRecord {
                id: Some(row.get(0)?),
                text: row.get(1)?,
                app: row.get(2)?,
                duration_ms: row.get(3)?,
                word_count: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Total number of stored transcriptions.
    pub fn count(&self) -> Result<i64> {
        let n = self
            .conn
            .query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0))?;
        Ok(n)
    }

    // ---- Analytics ----------------------------------------------------------

    /// Aggregate stats for the Home/Analytics dashboards.
    pub fn stats(&self) -> Result<DashboardStats> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0))?;
        let total_words: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(word_count), 0) FROM transcriptions",
            [],
            |r| r.get(0),
        )?;
        let avg_words = if total > 0 {
            total_words as f64 / total as f64
        } else {
            0.0
        };

        let mut stmt = self.conn.prepare(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day, COUNT(*)
             FROM transcriptions
             WHERE created_at >= strftime('%s', 'now', '-7 days')
             GROUP BY day ORDER BY day",
        )?;
        let last7_days = stmt
            .query_map([], |r| {
                Ok(DailyCount {
                    day: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(app, 'unknown') AS a, COUNT(*) AS c
             FROM transcriptions GROUP BY a ORDER BY c DESC LIMIT 5",
        )?;
        let top_apps = stmt
            .query_map([], |r| {
                Ok(AppCount {
                    app: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(DashboardStats {
            total,
            total_words,
            avg_words,
            last7_days,
            top_apps,
        })
    }

    // ---- Personal Memory (dictionary + snippets) ----------------------------

    pub fn list_memory(&self) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, trigger, replacement FROM personal_memory ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(MemoryEntry {
                id: Some(r.get(0)?),
                kind: r.get(1)?,
                trigger: r.get(2)?,
                replacement: r.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn add_memory(&self, kind: &str, trigger: &str, replacement: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO personal_memory (kind, trigger, replacement) VALUES (?1, ?2, ?3)",
            params![kind, trigger, replacement],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_memory(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM personal_memory WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Build a [`MemoryService`](crate::memory::MemoryService) from the stored
    /// entries, ready to apply during dictation.
    pub fn load_memory_service(&self) -> Result<crate::memory::MemoryService> {
        let mut service = crate::memory::MemoryService::new();
        for entry in self.list_memory()? {
            if entry.kind == "snippet" {
                service.add_snippet(&entry.trigger, entry.replacement);
            } else {
                service.add_term(&entry.trigger, entry.replacement);
            }
        }
        Ok(service)
    }
}

/// A day → count pair for the activity sparkline.
#[derive(Debug, Clone, Serialize)]
pub struct DailyCount {
    pub day: String,
    pub count: i64,
}

/// An app → count pair for "top apps".
#[derive(Debug, Clone, Serialize)]
pub struct AppCount {
    pub app: String,
    pub count: i64,
}

/// Aggregate dashboard statistics.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardStats {
    pub total: i64,
    pub total_words: i64,
    pub avg_words: f64,
    pub last7_days: Vec<DailyCount>,
    pub top_apps: Vec<AppCount>,
}

/// A Personal Memory entry (`kind` is `"dictionary"` or `"snippet"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Option<i64>,
    pub kind: String,
    pub trigger: String,
    pub replacement: String,
}
