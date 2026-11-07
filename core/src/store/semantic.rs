//! Semantic memory: chunks, embeddings, graph, signals, findings, index queue,
//! concept/session caches and insight snapshots. Owned by the semantic area.

use super::{now_unix, Store, TranscriptionRecord};
use crate::error::Result;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A persisted chunk of one dictation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticChunk {
    pub id: i64,
    pub transcription_id: i64,
    pub chunk_index: i64,
    pub text: String,
    pub text_hash: String,
    pub source_created_at: i64,
    pub app_name: Option<String>,
    pub word_count: i64,
}

/// A chunk joined with its embedding for one model.
#[derive(Debug, Clone)]
pub struct EmbeddedChunk {
    pub chunk: SemanticChunk,
    pub model_id: String,
    pub dimension: usize,
    pub vector: Vec<u8>,
}

/// A queued transcription awaiting (re)indexing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueueItem {
    pub transcription_id: i64,
    pub enqueued_at: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
}

/// A stored signal joined with its chunk's context.
#[derive(Debug, Clone, Serialize)]
pub struct SignalWithContext {
    pub id: i64,
    pub chunk_id: i64,
    pub family: String,
    pub value: String,
    pub confidence: f64,
    pub model_id: String,
    pub extracted_at: i64,
    pub chunk_text: String,
    pub source_created_at: i64,
    pub app_name: Option<String>,
    pub transcription_id: i64,
}

/// A node of the stored semantic memory graph (chunks, topics, entities, apps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNodeRecord {
    pub node_id: String,
    pub kind: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub weight: f64,
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdgeRecord {
    pub source_node_id: String,
    pub target_node_id: String,
    pub kind: String,
    pub weight: f64,
    pub evidence: Option<String>,
}

/// A typed, pre-computed pattern (Mac `InsightFinding`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InsightFinding {
    pub id: i64,
    pub kind: String,
    pub subject_key: String,
    pub title: String,
    pub detail: String,
    pub magnitude: f64,
    pub confidence: f64,
    pub window_start: Option<i64>,
    pub window_end: Option<i64>,
    pub evidence_chunk_ids: Vec<i64>,
    pub first_seen_at: i64,
    pub last_computed_at: i64,
    pub last_shown_at: Option<i64>,
    /// `active | expired | resolved | dismissed`.
    pub status: String,
}

/// Engine output before persistence identity is attached.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FindingDraft {
    pub kind: String,
    pub subject_key: String,
    pub title: String,
    pub detail: String,
    pub magnitude: f64,
    pub confidence: f64,
    pub window_start: Option<i64>,
    pub window_end: Option<i64>,
    pub evidence_chunk_ids: Vec<i64>,
}

/// One dictation as the concept graph reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptSource {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
    pub app_name: Option<String>,
}

/// Cached concept extraction for one dictation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptExtractionRecord {
    pub transcription_id: i64,
    pub text_hash: String,
    pub extractor_version: i64,
    pub concepts_json: String,
    pub extracted_at: i64,
}

/// Index counts for the status strip.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SemanticCounts {
    pub transcription_count: i64,
    pub indexed_transcription_count: i64,
    pub chunk_count: i64,
    pub embedded_chunk_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub signal_count: i64,
    pub latest_indexed_at: Option<i64>,
}

const CHUNK_COLS: &str =
    "c.id, c.transcription_id, c.chunk_index, c.text, c.text_hash, c.source_created_at, c.app_name, c.word_count";

fn chunk_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SemanticChunk> {
    Ok(SemanticChunk {
        id: r.get(0)?,
        transcription_id: r.get(1)?,
        chunk_index: r.get(2)?,
        text: r.get(3)?,
        text_hash: r.get(4)?,
        source_created_at: r.get(5)?,
        app_name: r.get(6)?,
        word_count: r.get(7)?,
    })
}

fn in_list(n: usize) -> String {
    vec!["?"; n.max(1)].join(", ")
}

impl Store {
    // ---- Queue -------------------------------------------------------------

    /// Queue transcriptions for (re)indexing; already-queued ones keep their place.
    pub fn semantic_enqueue(&self, ids: &[i64]) -> Result<usize> {
        let now = now_unix();
        let mut stmt = self.conn().prepare(
            "INSERT INTO semantic_index_queue (transcription_id, enqueued_at, attempts)
             VALUES (?1, ?2, 0) ON CONFLICT(transcription_id) DO NOTHING",
        )?;
        let mut added = 0;
        for id in ids {
            added += stmt.execute(params![id, now])?;
        }
        Ok(added)
    }

    /// Queue every plain dictation that has no chunks yet, or whose chunks
    /// lack an embedding for `model_id`. Returns how many were added.
    pub fn semantic_enqueue_missing(&self, model_id: &str) -> Result<usize> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO semantic_index_queue (transcription_id, enqueued_at, attempts)
             SELECT t.id, ?2, 0 FROM transcription t
             WHERE t.entry_kind != 'edit' AND length(trim(t.text)) > 0
               AND (NOT EXISTS (SELECT 1 FROM semantic_chunk c WHERE c.transcription_id = t.id)
                    OR EXISTS (SELECT 1 FROM semantic_chunk c WHERE c.transcription_id = t.id
                               AND NOT EXISTS (SELECT 1 FROM semantic_embedding e
                                               WHERE e.chunk_id = c.id AND e.model_id = ?1)))",
            params![model_id, now_unix()],
        )?;
        Ok(n)
    }

    /// Oldest-first queue entries with fewer than `max_attempts` failures.
    pub fn semantic_queue(&self, limit: i64, max_attempts: i64) -> Result<Vec<QueueItem>> {
        let mut stmt = self.conn().prepare(
            "SELECT transcription_id, enqueued_at, attempts, last_error FROM semantic_index_queue
             WHERE attempts < ?1 ORDER BY enqueued_at ASC, transcription_id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![max_attempts, limit.max(1)], |r| {
            Ok(QueueItem {
                transcription_id: r.get(0)?,
                enqueued_at: r.get(1)?,
                attempts: r.get(2)?,
                last_error: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn semantic_dequeue(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "DELETE FROM semantic_index_queue WHERE transcription_id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Record a failed attempt so the item is retried later (up to the cap).
    pub fn semantic_queue_failed(&self, id: i64, error: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE semantic_index_queue SET attempts = attempts + 1, last_error = ?2
             WHERE transcription_id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Give failed items another round of retries ("Index Now").
    pub fn semantic_queue_reset_failures(&self) -> Result<()> {
        self.conn().execute(
            "UPDATE semantic_index_queue SET attempts = 0, last_error = NULL",
            [],
        )?;
        Ok(())
    }

    /// (pending, failed, last error) with `max_attempts` as the failure cap.
    pub fn semantic_queue_counts(&self, max_attempts: i64) -> Result<(i64, i64, Option<String>)> {
        let pending: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM semantic_index_queue WHERE attempts < ?1",
            [max_attempts],
            |r| r.get(0),
        )?;
        let failed: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM semantic_index_queue WHERE attempts >= ?1",
            [max_attempts],
            |r| r.get(0),
        )?;
        let last: Option<String> = self
            .conn()
            .query_row(
                "SELECT last_error FROM semantic_index_queue WHERE last_error IS NOT NULL
                 ORDER BY attempts DESC, enqueued_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok((pending, failed, last))
    }

    // ---- Chunks & embeddings -------------------------------------------------

    /// Replace a transcription's chunks with `drafts`, keeping unchanged chunks
    /// (and their embeddings/signals) and dropping stale ones.
    pub fn semantic_upsert_chunks(
        &self,
        record: &TranscriptionRecord,
        drafts: &[crate::semantic::vector::ChunkDraft],
    ) -> Result<Vec<SemanticChunk>> {
        let Some(tid) = record.id else {
            return Ok(Vec::new());
        };
        let tx = self.conn().unchecked_transaction()?;
        let existing: Vec<SemanticChunk> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {CHUNK_COLS} FROM semantic_chunk c WHERE c.transcription_id = ?1"
            ))?;
            let rows = stmt.query_map([tid], chunk_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let draft_indexes: HashSet<i64> = drafts.iter().map(|d| d.chunk_index).collect();
        for stale in existing
            .iter()
            .filter(|c| !draft_indexes.contains(&c.chunk_index))
        {
            tx.execute(
                "DELETE FROM semantic_embedding WHERE chunk_id = ?1",
                [stale.id],
            )?;
            tx.execute(
                "DELETE FROM semantic_signal WHERE chunk_id = ?1",
                [stale.id],
            )?;
            tx.execute("DELETE FROM semantic_chunk WHERE id = ?1", [stale.id])?;
        }
        let now = now_unix();
        let mut out = Vec::new();
        for d in drafts {
            if let Some(ex) = existing.iter().find(|c| c.chunk_index == d.chunk_index) {
                if ex.text_hash != d.text_hash {
                    tx.execute(
                        "DELETE FROM semantic_embedding WHERE chunk_id = ?1",
                        [ex.id],
                    )?;
                    tx.execute("DELETE FROM semantic_signal WHERE chunk_id = ?1", [ex.id])?;
                }
                tx.execute(
                    "UPDATE semantic_chunk SET text = ?2, text_hash = ?3, source_created_at = ?4,
                     app_name = ?5, word_count = ?6, updated_at = ?7 WHERE id = ?1",
                    params![
                        ex.id,
                        d.text,
                        d.text_hash,
                        record.created_at,
                        record.app,
                        d.word_count,
                        now
                    ],
                )?;
                out.push(SemanticChunk {
                    id: ex.id,
                    transcription_id: tid,
                    chunk_index: d.chunk_index,
                    text: d.text.clone(),
                    text_hash: d.text_hash.clone(),
                    source_created_at: record.created_at,
                    app_name: record.app.clone(),
                    word_count: d.word_count,
                });
            } else {
                tx.execute(
                    "INSERT INTO semantic_chunk (transcription_id, chunk_index, text, text_hash,
                     source_created_at, app_name, word_count, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        tid,
                        d.chunk_index,
                        d.text,
                        d.text_hash,
                        record.created_at,
                        record.app,
                        d.word_count,
                        now
                    ],
                )?;
                out.push(SemanticChunk {
                    id: tx.last_insert_rowid(),
                    transcription_id: tid,
                    chunk_index: d.chunk_index,
                    text: d.text.clone(),
                    text_hash: d.text_hash.clone(),
                    source_created_at: record.created_at,
                    app_name: record.app.clone(),
                    word_count: d.word_count,
                });
            }
        }
        tx.commit()?;
        out.sort_by_key(|c| c.chunk_index);
        Ok(out)
    }

    /// Remove every chunk (and derived row) of one transcription.
    pub fn semantic_delete_chunks_for(&self, transcription_id: i64) -> Result<()> {
        self.conn().execute(
            "DELETE FROM semantic_signal WHERE chunk_id IN
             (SELECT id FROM semantic_chunk WHERE transcription_id = ?1)",
            [transcription_id],
        )?;
        self.conn().execute(
            "DELETE FROM semantic_embedding WHERE chunk_id IN
             (SELECT id FROM semantic_chunk WHERE transcription_id = ?1)",
            [transcription_id],
        )?;
        self.conn().execute(
            "DELETE FROM semantic_chunk WHERE transcription_id = ?1",
            [transcription_id],
        )?;
        Ok(())
    }

    /// Chunk ids that already carry an embedding for `model_id`.
    pub fn semantic_embedded_chunk_ids(&self, model_id: &str) -> Result<HashSet<i64>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT chunk_id FROM semantic_embedding WHERE model_id = ?1")?;
        let rows = stmt.query_map([model_id], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
    }

    pub fn semantic_save_embedding(
        &self,
        chunk_id: i64,
        model_id: &str,
        provider_name: &str,
        vector_le: &[u8],
        dimension: usize,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO semantic_embedding (chunk_id, model_id, provider_name, dimension, vector, generated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chunk_id, model_id) DO UPDATE SET provider_name = excluded.provider_name,
                dimension = excluded.dimension, vector = excluded.vector, generated_at = excluded.generated_at",
            params![chunk_id, model_id, provider_name, dimension as i64, vector_le, now_unix()],
        )?;
        Ok(())
    }

    /// Chunks embedded with `model_id`, newest first.
    pub fn semantic_embedded_chunks(
        &self,
        model_id: &str,
        limit: i64,
    ) -> Result<Vec<EmbeddedChunk>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {CHUNK_COLS}, e.model_id, e.dimension, e.vector FROM semantic_chunk c
             JOIN semantic_embedding e ON e.chunk_id = c.id WHERE e.model_id = ?1
             ORDER BY c.source_created_at DESC, c.chunk_index ASC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![model_id, limit.max(1)], |r| {
            Ok(EmbeddedChunk {
                chunk: chunk_from_row(r)?,
                model_id: r.get(8)?,
                dimension: r.get::<_, i64>(9)?.max(0) as usize,
                vector: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every chunk, newest first (keyword fallback and evidence lookups).
    pub fn semantic_chunks(&self, limit: i64) -> Result<Vec<SemanticChunk>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {CHUNK_COLS} FROM semantic_chunk c
             ORDER BY c.source_created_at DESC, c.chunk_index ASC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit.max(1)], chunk_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn semantic_chunks_by_ids(&self, ids: &[i64]) -> Result<HashMap<i64, SemanticChunk>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {CHUNK_COLS} FROM semantic_chunk c WHERE c.id IN ({})",
            in_list(ids.len())
        ))?;
        let rows = stmt.query_map(params_from_iter(ids.iter()), chunk_from_row)?;
        let mut out = HashMap::new();
        for c in rows {
            let c = c?;
            out.insert(c.id, c);
        }
        Ok(out)
    }

    /// Drop embeddings from every model but `keep` (after a completed re-index).
    pub fn semantic_delete_embeddings_except(&self, keep: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM semantic_embedding WHERE model_id != ?1",
            [keep],
        )?)
    }

    /// Distinct model ids present in the embedding table.
    pub fn semantic_embedding_models(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT DISTINCT model_id FROM semantic_embedding ORDER BY model_id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Signals -----------------------------------------------------------

    pub fn semantic_signal_chunk_ids(&self, model_id: &str) -> Result<HashSet<i64>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT DISTINCT chunk_id FROM semantic_signal WHERE model_id = ?1")?;
        let rows = stmt.query_map([model_id], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
    }

    /// Insert one chunk's signals from `model_id`, skipping exact duplicates.
    pub fn semantic_insert_signals(
        &self,
        chunk_id: i64,
        model_id: &str,
        signals: &[(String, String, f64)],
    ) -> Result<usize> {
        let now = now_unix();
        let mut added = 0;
        for (family, value, confidence) in signals {
            let exists: bool = self.conn().query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_signal WHERE chunk_id = ?1 AND family = ?2
                 AND value = ?3 AND model_id = ?4)",
                params![chunk_id, family, value, model_id],
                |r| r.get(0),
            )?;
            if exists {
                continue;
            }
            self.conn().execute(
                "INSERT INTO semantic_signal (chunk_id, family, value, confidence, model_id, extracted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![chunk_id, family, value, confidence, model_id, now],
            )?;
            added += 1;
        }
        Ok(added)
    }

    /// Signals joined with chunk context, newest first.
    pub fn semantic_signals(
        &self,
        families: Option<&[&str]>,
        since: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SignalWithContext>> {
        let mut sql = String::from(
            "SELECT s.id, s.chunk_id, s.family, s.value, s.confidence, s.model_id, s.extracted_at,
                    c.text, c.source_created_at, c.app_name, c.transcription_id
             FROM semantic_signal s JOIN semantic_chunk c ON c.id = s.chunk_id WHERE 1 = 1",
        );
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(f) = families.filter(|f| !f.is_empty()) {
            sql.push_str(&format!(" AND s.family IN ({})", in_list(f.len())));
            args.extend(
                f.iter()
                    .map(|x| rusqlite::types::Value::from(x.to_string())),
            );
        }
        if let Some(since) = since {
            sql.push_str(" AND c.source_created_at >= ?");
            args.push(since.into());
        }
        sql.push_str(" ORDER BY c.source_created_at DESC, s.id ASC LIMIT ?");
        args.push(limit.max(1).into());
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
            Ok(SignalWithContext {
                id: r.get(0)?,
                chunk_id: r.get(1)?,
                family: r.get(2)?,
                value: r.get(3)?,
                confidence: r.get(4)?,
                model_id: r.get(5)?,
                extracted_at: r.get(6)?,
                chunk_text: r.get(7)?,
                source_created_at: r.get(8)?,
                app_name: r.get(9)?,
                transcription_id: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Stored memory graph ----------------------------------------------

    pub fn semantic_replace_graph(
        &self,
        nodes: &[GraphNodeRecord],
        edges: &[GraphEdgeRecord],
    ) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM semantic_graph_edge", [])?;
        tx.execute("DELETE FROM semantic_graph_node", [])?;
        let now = now_unix();
        {
            let mut ns = tx.prepare(
                "INSERT INTO semantic_graph_node (node_id, kind, title, subtitle, weight, last_seen_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for n in nodes {
                ns.execute(params![
                    n.node_id,
                    n.kind,
                    n.title,
                    n.subtitle,
                    n.weight,
                    n.last_seen_at,
                    now
                ])?;
            }
            let mut es = tx.prepare(
                "INSERT OR REPLACE INTO semantic_graph_edge (source_node_id, target_node_id, kind, weight, evidence, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for e in edges {
                es.execute(params![
                    e.source_node_id,
                    e.target_node_id,
                    e.kind,
                    e.weight,
                    e.evidence,
                    now
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Heaviest nodes, then the heaviest edges between them.
    pub fn semantic_graph(
        &self,
        limit_nodes: i64,
        limit_edges: usize,
    ) -> Result<(Vec<GraphNodeRecord>, Vec<GraphEdgeRecord>)> {
        let mut stmt = self.conn().prepare(
            "SELECT node_id, kind, title, subtitle, weight, last_seen_at FROM semantic_graph_node
             ORDER BY weight DESC, title ASC LIMIT ?1",
        )?;
        let nodes = stmt
            .query_map([limit_nodes.max(1)], |r| {
                Ok(GraphNodeRecord {
                    node_id: r.get(0)?,
                    kind: r.get(1)?,
                    title: r.get(2)?,
                    subtitle: r.get(3)?,
                    weight: r.get(4)?,
                    last_seen_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ids: HashSet<&str> = nodes.iter().map(|n| n.node_id.as_str()).collect();
        let mut stmt = self.conn().prepare(
            "SELECT source_node_id, target_node_id, kind, weight, evidence FROM semantic_graph_edge
             ORDER BY weight DESC, id ASC",
        )?;
        let edges = stmt
            .query_map([], |r| {
                Ok(GraphEdgeRecord {
                    source_node_id: r.get(0)?,
                    target_node_id: r.get(1)?,
                    kind: r.get(2)?,
                    weight: r.get(3)?,
                    evidence: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|e| {
                ids.contains(e.source_node_id.as_str()) && ids.contains(e.target_node_id.as_str())
            })
            .take(limit_edges.max(1))
            .collect();
        Ok((nodes, edges))
    }

    // ---- Findings ----------------------------------------------------------

    /// Reconcile a fresh computation with stored findings: recomputed ones
    /// update in place (keeping first-seen, last-shown and user status),
    /// vanished active ones of the computed kinds expire, new ones insert.
    pub fn semantic_reconcile_findings(
        &self,
        drafts: &[FindingDraft],
        computed_kinds: &[&str],
        now: i64,
    ) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        let mut fresh: HashSet<(String, String)> = HashSet::new();
        for d in drafts {
            fresh.insert((d.kind.clone(), d.subject_key.clone()));
            let evidence = serde_json::to_string(&d.evidence_chunk_ids)?;
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT id, status FROM insight_finding WHERE kind = ?1 AND subject_key = ?2",
                    params![d.kind, d.subject_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, status)) = existing {
                let status = if status == "expired" {
                    "active".to_string()
                } else {
                    status
                };
                tx.execute(
                    "UPDATE insight_finding SET title = ?2, detail = ?3, magnitude = ?4, confidence = ?5,
                     window_start = ?6, window_end = ?7, evidence_chunk_ids = ?8, last_computed_at = ?9,
                     status = ?10 WHERE id = ?1",
                    params![
                        id,
                        d.title,
                        d.detail,
                        d.magnitude,
                        d.confidence,
                        d.window_start,
                        d.window_end,
                        evidence,
                        now,
                        status
                    ],
                )?;
            } else {
                tx.execute(
                    "INSERT INTO insight_finding (kind, subject_key, title, detail, magnitude, confidence,
                     window_start, window_end, evidence_chunk_ids, first_seen_at, last_computed_at, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 'active')",
                    params![
                        d.kind,
                        d.subject_key,
                        d.title,
                        d.detail,
                        d.magnitude,
                        d.confidence,
                        d.window_start,
                        d.window_end,
                        evidence,
                        now
                    ],
                )?;
            }
        }
        if !computed_kinds.is_empty() {
            let stale: Vec<(i64, String, String)> = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT id, kind, subject_key FROM insight_finding WHERE status = 'active'
                     AND kind IN ({})",
                    in_list(computed_kinds.len())
                ))?;
                let rows = stmt.query_map(params_from_iter(computed_kinds.iter()), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (id, kind, subject) in stale {
                if !fresh.contains(&(kind, subject)) {
                    tx.execute(
                        "UPDATE insight_finding SET status = 'expired', last_computed_at = ?2 WHERE id = ?1",
                        params![id, now],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Findings with the given statuses (default active), strongest first.
    pub fn semantic_findings(
        &self,
        kinds: Option<&[String]>,
        statuses: &[&str],
        limit: i64,
    ) -> Result<Vec<InsightFinding>> {
        let statuses: Vec<&str> = if statuses.is_empty() {
            vec!["active"]
        } else {
            statuses.to_vec()
        };
        let mut sql = format!(
            "SELECT id, kind, subject_key, title, detail, magnitude, confidence, window_start, window_end,
             evidence_chunk_ids, first_seen_at, last_computed_at, last_shown_at, status
             FROM insight_finding WHERE status IN ({})",
            in_list(statuses.len())
        );
        let mut args: Vec<rusqlite::types::Value> = statuses
            .iter()
            .map(|s| rusqlite::types::Value::from(s.to_string()))
            .collect();
        if let Some(k) = kinds.filter(|k| !k.is_empty()) {
            sql.push_str(&format!(" AND kind IN ({})", in_list(k.len())));
            args.extend(k.iter().map(|x| rusqlite::types::Value::from(x.clone())));
        }
        sql.push_str(" ORDER BY magnitude DESC, id ASC LIMIT ?");
        args.push(limit.max(1).into());
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
            let evidence: String = r.get(9)?;
            Ok(InsightFinding {
                id: r.get(0)?,
                kind: r.get(1)?,
                subject_key: r.get(2)?,
                title: r.get(3)?,
                detail: r.get(4)?,
                magnitude: r.get(5)?,
                confidence: r.get(6)?,
                window_start: r.get(7)?,
                window_end: r.get(8)?,
                evidence_chunk_ids: serde_json::from_str(&evidence).unwrap_or_default(),
                first_seen_at: r.get(10)?,
                last_computed_at: r.get(11)?,
                last_shown_at: r.get(12)?,
                status: r.get(13)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// User feedback on a finding (resolve/dismiss); never undone by recomputation.
    pub fn semantic_set_finding_status(&self, id: i64, status: &str) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE insight_finding SET status = ?2 WHERE id = ?1",
            params![id, status],
        )? > 0)
    }

    pub fn semantic_mark_findings_shown(&self, ids: &[i64], at: i64) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut args: Vec<rusqlite::types::Value> = vec![at.into()];
        args.extend(ids.iter().map(|i| rusqlite::types::Value::from(*i)));
        self.conn().execute(
            &format!(
                "UPDATE insight_finding SET last_shown_at = ? WHERE id IN ({})",
                in_list(ids.len())
            ),
            params_from_iter(args.iter()),
        )?;
        Ok(())
    }

    // ---- Concept cache -------------------------------------------------------

    /// Every plain dictation (edit commands excluded), oldest first.
    pub fn semantic_concept_sources(&self) -> Result<Vec<ConceptSource>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, text, created_at, app_name FROM transcription
             WHERE entry_kind != 'edit' AND length(trim(text)) > 0
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ConceptSource {
                id: r.get(0)?,
                text: r.get(1)?,
                created_at: r.get(2)?,
                app_name: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn semantic_concept_extractions(&self) -> Result<HashMap<i64, ConceptExtractionRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT transcription_id, text_hash, extractor_version, concepts_json, extracted_at
             FROM concept_extraction",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ConceptExtractionRecord {
                transcription_id: r.get(0)?,
                text_hash: r.get(1)?,
                extractor_version: r.get(2)?,
                concepts_json: r.get(3)?,
                extracted_at: r.get(4)?,
            })
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let r = r?;
            out.insert(r.transcription_id, r);
        }
        Ok(out)
    }

    pub fn semantic_save_concept_extractions(
        &self,
        records: &[ConceptExtractionRecord],
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let tx = self.conn().unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO concept_extraction (transcription_id, text_hash, extractor_version, concepts_json, extracted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(transcription_id) DO UPDATE SET text_hash = excluded.text_hash,
                   extractor_version = excluded.extractor_version, concepts_json = excluded.concepts_json,
                   extracted_at = excluded.extracted_at",
            )?;
            for r in records {
                stmt.execute(params![
                    r.transcription_id,
                    r.text_hash,
                    r.extractor_version,
                    r.concepts_json,
                    r.extracted_at
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Drop cached extractions whose dictation no longer exists.
    pub fn semantic_prune_concept_extractions(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM concept_extraction WHERE transcription_id NOT IN (SELECT id FROM transcription)",
            [],
        )?)
    }

    /// Forget every cached concept extraction ("Rebuild graph").
    pub fn semantic_clear_concept_cache(&self) -> Result<()> {
        self.conn().execute("DELETE FROM concept_extraction", [])?;
        Ok(())
    }

    /// Active dictionary targets (preferred spellings for concept names).
    pub fn semantic_dictionary_terms(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT target FROM dictionary_entry WHERE is_active = 1 ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Session analyses ----------------------------------------------------

    /// session_key → (model, analysis_json) for one prompt version.
    pub fn semantic_session_analyses(
        &self,
        prompt_version: i64,
    ) -> Result<HashMap<String, (String, String)>> {
        let mut stmt = self.conn().prepare(
            "SELECT session_key, model, analysis_json FROM session_analysis WHERE prompt_version = ?1",
        )?;
        let rows = stmt.query_map([prompt_version], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    pub fn semantic_save_session_analysis(
        &self,
        key: &str,
        prompt_version: i64,
        model: &str,
        analysis_json: &str,
        session_end: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO session_analysis (session_key, prompt_version, model, analysis_json, session_end, analyzed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_key) DO UPDATE SET prompt_version = excluded.prompt_version,
               model = excluded.model, analysis_json = excluded.analysis_json,
               session_end = excluded.session_end, analyzed_at = excluded.analyzed_at",
            params![key, prompt_version, model, analysis_json, session_end, now_unix()],
        )?;
        Ok(())
    }

    // ---- Insight snapshots ---------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn semantic_save_insight_snapshot(
        &self,
        generated_at: i64,
        graph_signature: &str,
        analyzer_name: &str,
        summary_model: Option<&str>,
        nodes: i64,
        edges: i64,
        chunks: i64,
        report_json: &str,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO semantic_insight_snapshot (generated_at, graph_signature, analyzer_name,
             summary_model_name, source_node_count, source_edge_count, source_chunk_count, report_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                generated_at,
                graph_signature,
                analyzer_name,
                summary_model,
                nodes,
                edges,
                chunks,
                report_json
            ],
        )?;
        let id = self.conn().last_insert_rowid();
        // Keep the 20 most recent snapshots.
        self.conn().execute(
            "DELETE FROM semantic_insight_snapshot WHERE id NOT IN
             (SELECT id FROM semantic_insight_snapshot ORDER BY generated_at DESC, id DESC LIMIT 20)",
            [],
        )?;
        Ok(id)
    }

    /// The latest snapshot's report JSON.
    pub fn semantic_latest_insight_snapshot(&self) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT report_json FROM semantic_insight_snapshot ORDER BY generated_at DESC, id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ---- Stats & reset -------------------------------------------------------

    pub fn semantic_counts(&self, model_id: &str) -> Result<SemanticCounts> {
        let one = |sql: &str| -> Result<i64> { Ok(self.conn().query_row(sql, [], |r| r.get(0))?) };
        Ok(SemanticCounts {
            transcription_count: one(
                "SELECT COUNT(*) FROM transcription WHERE entry_kind != 'edit' AND length(trim(text)) > 0",
            )?,
            indexed_transcription_count: self.conn().query_row(
                "SELECT COUNT(DISTINCT c.transcription_id) FROM semantic_chunk c
                 JOIN semantic_embedding e ON e.chunk_id = c.id WHERE e.model_id = ?1",
                [model_id],
                |r| r.get(0),
            )?,
            chunk_count: one("SELECT COUNT(*) FROM semantic_chunk")?,
            embedded_chunk_count: self.conn().query_row(
                "SELECT COUNT(*) FROM semantic_embedding WHERE model_id = ?1",
                [model_id],
                |r| r.get(0),
            )?,
            node_count: one("SELECT COUNT(*) FROM semantic_graph_node")?,
            edge_count: one("SELECT COUNT(*) FROM semantic_graph_edge")?,
            signal_count: one("SELECT COUNT(*) FROM semantic_signal")?,
            latest_indexed_at: self.conn().query_row(
                "SELECT MAX(generated_at) FROM semantic_embedding WHERE model_id = ?1",
                [model_id],
                |r| r.get(0),
            )?,
        })
    }

    /// Settings → Clear Index: drop the whole semantic memory (Mac
    /// `clearSemanticMemory`). Dictations themselves are untouched.
    pub fn semantic_clear(&self) -> Result<()> {
        self.conn().execute_batch(
            "DELETE FROM semantic_insight_snapshot; DELETE FROM semantic_graph_edge;
             DELETE FROM semantic_graph_node; DELETE FROM concept_extraction; DELETE FROM session_analysis;
             DELETE FROM semantic_signal; DELETE FROM semantic_embedding; DELETE FROM semantic_chunk;
             DELETE FROM insight_finding; DELETE FROM semantic_index_queue;",
        )?;
        Ok(())
    }

    /// Plain dictations (no edits) created in `[from, to)`, oldest first.
    pub fn semantic_transcriptions_between(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<TranscriptionRecord>> {
        let mut out: Vec<TranscriptionRecord> = self
            .transcriptions_since(from, 1_000_000)?
            .into_iter()
            .filter(|r| r.created_at < to && r.entry_kind != "edit")
            .collect();
        out.sort_by_key(|r| (r.created_at, r.id));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::vector::make_chunks;

    fn store_with(text: &str) -> (Store, TranscriptionRecord) {
        let s = Store::open_in_memory().unwrap();
        let mut r = TranscriptionRecord::new(text, Some("code".into()), 1000, 1_000);
        r.id = Some(s.insert_transcription(&r).unwrap());
        (s, r)
    }

    #[test]
    fn chunk_upsert_keeps_unchanged_and_drops_stale() {
        let long = (0..200)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let (s, mut r) = store_with(&long);
        let first = s.semantic_upsert_chunks(&r, &make_chunks(&long)).unwrap();
        assert_eq!(first.len(), 2);
        s.semantic_save_embedding(first[0].id, "m", "p", &[0; 8], 2)
            .unwrap();
        s.semantic_insert_signals(
            first[0].id,
            "h",
            &[("intent".into(), "reflect".into(), 0.6)],
        )
        .unwrap();
        // Same text → same ids, embedding kept.
        let again = s.semantic_upsert_chunks(&r, &make_chunks(&long)).unwrap();
        assert_eq!(again[0].id, first[0].id);
        assert!(s
            .semantic_embedded_chunk_ids("m")
            .unwrap()
            .contains(&first[0].id));
        // Shorter, different text → one chunk, its embedding/signals gone.
        r.text = "totally new words".into();
        let shrunk = s.semantic_upsert_chunks(&r, &make_chunks(&r.text)).unwrap();
        assert_eq!(shrunk.len(), 1);
        assert!(s.semantic_embedded_chunk_ids("m").unwrap().is_empty());
        assert!(s.semantic_signal_chunk_ids("h").unwrap().is_empty());
        assert_eq!(s.semantic_chunks(10).unwrap().len(), 1);
    }

    #[test]
    fn queue_retries_and_caps() {
        let (s, r) = store_with("hello world again");
        let id = r.id.unwrap();
        assert_eq!(s.semantic_enqueue(&[id]).unwrap(), 1);
        assert_eq!(s.semantic_enqueue(&[id]).unwrap(), 0);
        for _ in 0..3 {
            s.semantic_queue_failed(id, "boom").unwrap();
        }
        assert!(s.semantic_queue(10, 3).unwrap().is_empty());
        assert_eq!(
            s.semantic_queue_counts(3).unwrap(),
            (0, 1, Some("boom".into()))
        );
        s.semantic_queue_reset_failures().unwrap();
        assert_eq!(s.semantic_queue(10, 3).unwrap().len(), 1);
        s.semantic_dequeue(id).unwrap();
        assert_eq!(s.semantic_queue_counts(3).unwrap().0, 0);
        // Missing-embedding enqueue picks up unindexed dictations.
        assert_eq!(s.semantic_enqueue_missing("m").unwrap(), 1);
    }

    #[test]
    fn findings_reconcile_preserves_user_status() {
        let s = Store::open_in_memory().unwrap();
        let draft = |k: &str, sub: &str| FindingDraft {
            kind: k.into(),
            subject_key: sub.into(),
            title: "t".into(),
            detail: "d".into(),
            magnitude: 1.0,
            confidence: 0.5,
            window_start: None,
            window_end: None,
            evidence_chunk_ids: vec![1, 2],
        };
        s.semantic_reconcile_findings(
            &[draft("rhythm", "a"), draft("rhythm", "b")],
            &["rhythm"],
            10,
        )
        .unwrap();
        let all = s.semantic_findings(None, &[], 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].evidence_chunk_ids, vec![1, 2]);
        let b = all.iter().find(|f| f.subject_key == "b").unwrap().id;
        s.semantic_set_finding_status(b, "dismissed").unwrap();
        // "a" vanishes → expired; "b" recomputed but stays dismissed.
        s.semantic_reconcile_findings(&[draft("rhythm", "b")], &["rhythm"], 20)
            .unwrap();
        assert!(s
            .semantic_findings(None, &["active"], 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            s.semantic_findings(None, &["expired"], 10).unwrap().len(),
            1
        );
        assert_eq!(
            s.semantic_findings(None, &["dismissed"], 10).unwrap().len(),
            1
        );
        // "a" returns → re-activated with its first-seen kept.
        s.semantic_reconcile_findings(&[draft("rhythm", "a")], &["rhythm"], 30)
            .unwrap();
        let a = &s.semantic_findings(None, &["active"], 10).unwrap()[0];
        assert_eq!(a.first_seen_at, 10);
        s.semantic_mark_findings_shown(&[a.id], 40).unwrap();
        assert_eq!(
            s.semantic_findings(None, &[], 10).unwrap()[0].last_shown_at,
            Some(40)
        );
    }

    #[test]
    fn clear_removes_everything_semantic() {
        let (s, r) = store_with("some text to index");
        let chunks = s.semantic_upsert_chunks(&r, &make_chunks(&r.text)).unwrap();
        s.semantic_save_embedding(chunks[0].id, "m", "p", &[0; 4], 1)
            .unwrap();
        s.semantic_enqueue(&[r.id.unwrap()]).unwrap();
        s.semantic_clear().unwrap();
        let c = s.semantic_counts("m").unwrap();
        assert_eq!(c.chunk_count, 0);
        assert_eq!(c.embedded_chunk_count, 0);
        assert_eq!(c.transcription_count, 1);
        assert_eq!(s.semantic_queue_counts(5).unwrap().0, 0);
    }
}
