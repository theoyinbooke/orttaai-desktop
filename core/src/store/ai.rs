//! Chat conversations, writing-insight and tone snapshots. Owned by the AI area.

use super::{now_unix, Store};
use crate::error::Result;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// A saved Chat AI conversation (sidebar row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatConversation {
    pub id: i64,
    pub title: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

/// A passage of the user's own dictation a reply drew on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSource {
    pub transcription_id: i64,
    pub text: String,
    pub created_at: i64,
    pub app: Option<String>,
    pub score: f32,
}

/// One stored chat message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessageRow {
    pub id: i64,
    pub conversation_id: i64,
    /// `user | assistant`.
    pub role: String,
    pub content: String,
    pub reasoning: Option<String>,
    pub sources: Vec<ChatSource>,
    pub created_at: i64,
}

/// A stored writing-insight (`kind = writing`) or tone (`kind = tone`) snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightSnapshotRow {
    pub id: i64,
    pub kind: String,
    pub generated_at: i64,
    pub analyzer_name: String,
    pub used_fallback: bool,
    pub sample_count: i64,
    pub request_json: String,
    pub snapshot_json: String,
    pub is_pinned: bool,
}

const CONV_COLUMNS: &str = "c.id, c.title, c.model, c.created_at, c.updated_at, \
    (SELECT COUNT(*) FROM chat_message m WHERE m.conversation_id = c.id AND m.content != '')";

fn conv_from_row(r: &Row<'_>) -> rusqlite::Result<ChatConversation> {
    Ok(ChatConversation {
        id: r.get(0)?,
        title: r.get(1)?,
        model: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        message_count: r.get(5)?,
    })
}

fn message_from_row(r: &Row<'_>) -> rusqlite::Result<ChatMessageRow> {
    let sources: Option<String> = r.get(5)?;
    Ok(ChatMessageRow {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        role: r.get(2)?,
        content: r.get(3)?,
        reasoning: r.get(4)?,
        sources: sources
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        created_at: r.get(6)?,
    })
}

const SNAPSHOT_COLUMNS: &str =
    "id, kind, generated_at, analyzer_name, used_fallback, sample_count, \
    request_json, snapshot_json, is_pinned";

fn snapshot_from_row(r: &Row<'_>) -> rusqlite::Result<InsightSnapshotRow> {
    Ok(InsightSnapshotRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        generated_at: r.get(2)?,
        analyzer_name: r.get(3)?,
        used_fallback: r.get(4)?,
        sample_count: r.get(5)?,
        request_json: r.get(6)?,
        snapshot_json: r.get(7)?,
        is_pinned: r.get(8)?,
    })
}

fn sources_json(sources: &[ChatSource]) -> Option<String> {
    (!sources.is_empty()).then(|| serde_json::to_string(sources).unwrap_or_default())
}

impl Store {
    // ---- Chat conversations ------------------------------------------------

    pub fn chat_create(&self, title: &str, model: Option<&str>) -> Result<i64> {
        let now = now_unix();
        self.conn().execute(
            "INSERT INTO chat_conversation (title, model, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![title, model, now],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn chat_get(&self, id: i64) -> Result<Option<ChatConversation>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {CONV_COLUMNS} FROM chat_conversation c WHERE c.id = ?1"),
                [id],
                conv_from_row,
            )
            .optional()?)
    }

    /// Conversations with messages, most recently used first. `search`
    /// matches the title or any message (case-insensitive).
    pub fn chat_list(&self, search: Option<&str>, limit: i64) -> Result<Vec<ChatConversation>> {
        let q = search.map(str::trim).filter(|s| !s.is_empty());
        let mut sql = format!(
            "SELECT {CONV_COLUMNS} FROM chat_conversation c
             WHERE EXISTS (SELECT 1 FROM chat_message m WHERE m.conversation_id = c.id AND m.content != '')"
        );
        if q.is_some() {
            sql.push_str(
                " AND (lower(c.title) LIKE ?2 OR EXISTS (SELECT 1 FROM chat_message m
                   WHERE m.conversation_id = c.id AND lower(m.content) LIKE ?2))",
            );
        }
        sql.push_str(" ORDER BY c.updated_at DESC, c.id DESC LIMIT ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = match q {
            Some(q) => {
                let like = format!("%{}%", q.to_lowercase());
                stmt.query_map(params![limit, like], conv_from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => stmt
                .query_map(params![limit], conv_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    pub fn chat_rename(&self, id: i64, title: &str) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE chat_conversation SET title = ?2 WHERE id = ?1",
            params![id, title],
        )? > 0)
    }

    pub fn chat_set_model(&self, id: i64, model: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE chat_conversation SET model = ?2 WHERE id = ?1",
            params![id, model],
        )?;
        Ok(())
    }

    /// Delete a conversation and its messages.
    pub fn chat_delete(&self, id: i64) -> Result<bool> {
        self.conn()
            .execute("DELETE FROM chat_message WHERE conversation_id = ?1", [id])?;
        Ok(self
            .conn()
            .execute("DELETE FROM chat_conversation WHERE id = ?1", [id])?
            > 0)
    }

    /// Append a message and bump the conversation's `updated_at`.
    pub fn chat_add_message(
        &self,
        conversation_id: i64,
        role: &str,
        content: &str,
        reasoning: Option<&str>,
        sources: &[ChatSource],
    ) -> Result<i64> {
        let now = now_unix();
        self.conn().execute(
            "INSERT INTO chat_message (conversation_id, role, content, reasoning, sources_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![conversation_id, role, content, reasoning, sources_json(sources), now],
        )?;
        let id = self.conn().last_insert_rowid();
        self.conn().execute(
            "UPDATE chat_conversation SET updated_at = ?2 WHERE id = ?1",
            params![conversation_id, now],
        )?;
        Ok(id)
    }

    pub fn chat_update_message(
        &self,
        id: i64,
        content: &str,
        reasoning: Option<&str>,
        sources: &[ChatSource],
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE chat_message SET content = ?2, reasoning = ?3, sources_json = ?4 WHERE id = ?1",
            params![id, content, reasoning, sources_json(sources)],
        )?;
        self.conn().execute(
            "UPDATE chat_conversation SET updated_at = ?2
             WHERE id = (SELECT conversation_id FROM chat_message WHERE id = ?1)",
            params![id, now_unix()],
        )?;
        Ok(())
    }

    pub fn chat_delete_message(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM chat_message WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Messages of a conversation in order. Empty rows (a reply that never
    /// finished) are left out.
    pub fn chat_messages(&self, conversation_id: i64) -> Result<Vec<ChatMessageRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, conversation_id, role, content, reasoning, sources_json, created_at
             FROM chat_message WHERE conversation_id = ?1 AND content != '' ORDER BY id",
        )?;
        let rows = stmt.query_map([conversation_id], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn chat_message(&self, id: i64) -> Result<Option<ChatMessageRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, conversation_id, role, content, reasoning, sources_json, created_at
                 FROM chat_message WHERE id = ?1",
                [id],
                message_from_row,
            )
            .optional()?)
    }

    /// Keep the `keep` most recently used conversations (Mac keeps 80);
    /// conversations without messages are dropped too. `except` survives.
    pub fn chat_prune(&self, keep: usize, except: Option<i64>) -> Result<usize> {
        let except = except.unwrap_or(-1);
        let doomed: Vec<i64> = {
            let mut stmt = self.conn().prepare(
                "SELECT c.id FROM chat_conversation c WHERE c.id != ?2 AND (
                    NOT EXISTS (SELECT 1 FROM chat_message m WHERE m.conversation_id = c.id)
                    OR c.id NOT IN (SELECT id FROM chat_conversation ORDER BY updated_at DESC, id DESC LIMIT ?1))",
            )?;
            let rows = stmt.query_map(params![keep as i64, except], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in &doomed {
            self.chat_delete(*id)?;
        }
        Ok(doomed.len())
    }

    // ---- Writing insight / tone snapshots ------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_insight_snapshot(
        &self,
        kind: &str,
        generated_at: i64,
        analyzer_name: &str,
        used_fallback: bool,
        sample_count: i64,
        request_json: &str,
        snapshot_json: &str,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO writing_insight_snapshot (kind, generated_at, analyzer_name, used_fallback,
                sample_count, request_json, snapshot_json, is_pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                kind,
                generated_at,
                analyzer_name,
                used_fallback,
                sample_count,
                request_json,
                snapshot_json
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn latest_insight_snapshot(&self, kind: &str) -> Result<Option<InsightSnapshotRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {SNAPSHOT_COLUMNS} FROM writing_insight_snapshot WHERE kind = ?1
                     ORDER BY generated_at DESC, id DESC LIMIT 1"
                ),
                [kind],
                snapshot_from_row,
            )
            .optional()?)
    }

    pub fn get_insight_snapshot(&self, id: i64) -> Result<Option<InsightSnapshotRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {SNAPSHOT_COLUMNS} FROM writing_insight_snapshot WHERE id = ?1"),
                [id],
                snapshot_from_row,
            )
            .optional()?)
    }

    /// History of one kind: pinned first, then newest (Mac order).
    pub fn insight_snapshot_history(
        &self,
        kind: &str,
        limit: i64,
    ) -> Result<Vec<InsightSnapshotRow>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {SNAPSHOT_COLUMNS} FROM writing_insight_snapshot WHERE kind = ?1
             ORDER BY is_pinned DESC, generated_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![kind, limit], snapshot_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_insight_snapshot_pinned(&self, id: i64, pinned: bool) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE writing_insight_snapshot SET is_pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )? > 0)
    }

    pub fn delete_insight_snapshot(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM writing_insight_snapshot WHERE id = ?1", [id])?
            > 0)
    }

    // ---- Transcription helpers used by insights -------------------------------

    /// Dictations at/after `since` (Mac `countTranscriptions(since:)`).
    pub fn count_transcriptions_since(&self, since: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM transcription WHERE created_at > ?1",
            [since],
            |r| r.get(0),
        )?)
    }

    /// When the newest dictation was made.
    pub fn latest_transcription_at(&self) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row("SELECT MAX(created_at) FROM transcription", [], |r| {
                r.get(0)
            })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    #[test]
    fn chat_crud_search_and_prune() {
        let s = Store::open_in_memory().unwrap();
        let a = s
            .chat_create("Plan the launch", Some("gemma4:e2b"))
            .unwrap();
        let b = s.chat_create("New chat", None).unwrap();
        assert!(
            s.chat_list(None, 50).unwrap().is_empty(),
            "empty chats hidden"
        );

        s.chat_add_message(a, "user", "When do we ship?", None, &[])
            .unwrap();
        let src = ChatSource {
            transcription_id: 7,
            text: "ship friday".into(),
            created_at: 1,
            app: Some("Slack".into()),
            score: 0.8,
        };
        let reply = s.chat_add_message(a, "assistant", "", None, &[]).unwrap();
        s.chat_update_message(
            reply,
            "Friday.",
            Some("checked memory"),
            std::slice::from_ref(&src),
        )
        .unwrap();
        s.chat_add_message(b, "user", "Draft a haiku", None, &[])
            .unwrap();
        let pending = s.chat_add_message(b, "assistant", "", None, &[]).unwrap();

        let list = s.chat_list(None, 50).unwrap();
        assert_eq!(list.len(), 2);
        let conv_a = list.iter().find(|c| c.id == a).unwrap();
        assert_eq!(conv_a.message_count, 2);
        assert_eq!(s.chat_list(Some("FRIDAY"), 50).unwrap()[0].id, a);
        assert_eq!(s.chat_list(Some("haiku"), 50).unwrap()[0].id, b);
        assert!(s.chat_list(Some("zzz"), 50).unwrap().is_empty());

        let msgs = s.chat_messages(a).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].sources, vec![src]);
        assert_eq!(msgs[1].reasoning.as_deref(), Some("checked memory"));
        assert_eq!(
            s.chat_messages(b).unwrap().len(),
            1,
            "unfinished reply hidden"
        );
        s.chat_delete_message(pending).unwrap();

        assert!(s.chat_rename(a, "Launch date").unwrap());
        assert_eq!(s.chat_get(a).unwrap().unwrap().title, "Launch date");
        s.chat_set_model(a, "qwen3:1.7b").unwrap();
        assert_eq!(
            s.chat_get(a).unwrap().unwrap().model.as_deref(),
            Some("qwen3:1.7b")
        );

        let c = s.chat_create("New chat", None).unwrap();
        assert_eq!(s.chat_prune(80, Some(c)).unwrap(), 0, "in-flight chat kept");
        assert_eq!(s.chat_prune(80, None).unwrap(), 1, "empty chat dropped");
        assert_eq!(s.chat_prune(1, None).unwrap(), 1);
        assert_eq!(s.chat_list(None, 50).unwrap().len(), 1);

        assert!(s.chat_delete(a).unwrap() || s.chat_delete(b).unwrap());
    }

    #[test]
    fn snapshots_history_pin_delete() {
        let s = Store::open_in_memory().unwrap();
        let w1 = s
            .insert_insight_snapshot(
                "writing",
                100,
                "Heuristic Analyzer",
                false,
                3,
                "{}",
                "{\"a\":1}",
            )
            .unwrap();
        let w2 = s
            .insert_insight_snapshot(
                "writing",
                200,
                "Ollama (Local)",
                false,
                5,
                "{}",
                "{\"a\":2}",
            )
            .unwrap();
        let t1 = s
            .insert_insight_snapshot("tone", 150, "gemma4:e2b", false, 9, "{}", "{}")
            .unwrap();
        assert_eq!(
            s.latest_insight_snapshot("writing").unwrap().unwrap().id,
            w2
        );
        assert_eq!(s.latest_insight_snapshot("tone").unwrap().unwrap().id, t1);
        assert!(s.set_insight_snapshot_pinned(w1, true).unwrap());
        let h = s.insight_snapshot_history("writing", 10).unwrap();
        assert_eq!(h.iter().map(|r| r.id).collect::<Vec<_>>(), vec![w1, w2]);
        assert!(h[0].is_pinned);
        assert!(s.delete_insight_snapshot(w1).unwrap());
        assert!(!s.delete_insight_snapshot(w1).unwrap());
        assert!(s.get_insight_snapshot(w2).unwrap().is_some());

        assert_eq!(s.latest_transcription_at().unwrap(), None);
        s.insert_transcription(&TranscriptionRecord::new("hi there", None, 1000, 500))
            .unwrap();
        assert_eq!(s.count_transcriptions_since(400).unwrap(), 1);
        assert_eq!(s.count_transcriptions_since(500).unwrap(), 0);
        assert_eq!(s.latest_transcription_at().unwrap(), Some(500));
    }
}
