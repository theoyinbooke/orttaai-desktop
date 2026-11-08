//! The semantic index pipeline (Mac `indexPendingTranscriptions`): queued
//! dictations are chunked (140 words, 24 overlap), hashed, mined for signals,
//! and embedded with `semantic_embedding_model` (f32 LE blobs). Failures are
//! retried up to [`MAX_ATTEMPTS`]; with keyword fallback on, a dictation the
//! embedding server can't take is embedded lexically so search still finds
//! it. Changing the embedding model re-indexes everything and switches the
//! active index (kv `semantic_active_index_model`) once complete.

use super::findings;
use super::memory_graph;
use super::signals::{self, HEURISTIC_MODEL_ID};
use super::util::Clock;
use super::vector::{self, LEXICAL_MODEL_ID, LEXICAL_PROVIDER_NAME};
use crate::error::{CoreError, Result};
use crate::llm::LlmClient;
use crate::settings::Settings;
use crate::store::{SemanticChunk, Store};
use serde::Serialize;

pub const MAX_ATTEMPTS: i64 = 5;
pub const ACTIVE_MODEL_KEY: &str = "semantic_active_index_model";
/// Set while some dictations only carry fallback embeddings.
pub const BACKFILL_KEY: &str = "semantic_backfill_pending";
/// Last UTC offset the UI reported, for background local-time math.
pub const TZ_KEY: &str = "semantic_tz_offset_minutes";
const EMBED_BATCH: usize = 16;
const QUEUE_BATCH: i64 = 16;
/// Chunks refined by the local model per run (bounded cost).
const REFINE_PER_RUN: usize = 24;

/// Settings the pipeline needs.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub enabled: bool,
    pub auto_index: bool,
    pub embedding_model: String,
    pub keyword_fallback: bool,
    pub summary_enabled: bool,
    pub summary_model: String,
}

impl IndexConfig {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            enabled: s.semantic_enabled,
            auto_index: s.semantic_auto_index,
            embedding_model: s.semantic_embedding_model.trim().to_string(),
            keyword_fallback: s.semantic_keyword_fallback,
            summary_enabled: s.semantic_summary_enabled,
            summary_model: s.semantic_summary_model.trim().to_string(),
        }
    }

    /// The model new embeddings are made with.
    pub fn target_model(&self) -> &str {
        if self.embedding_model.is_empty() {
            LEXICAL_MODEL_ID
        } else {
            &self.embedding_model
        }
    }
}

/// Progress of a running index pass (`semantic-progress` payload).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexProgress {
    /// `indexing | graph | findings | refining | done`.
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

/// Outcome of one pass (Mac `SemanticIndexRunResult`).
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct IndexRunResult {
    pub source_count: usize,
    pub chunk_count: usize,
    pub embedded_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub signal_count: usize,
    pub refined_count: usize,
    pub graph_node_count: usize,
    pub graph_edge_count: usize,
    pub finding_count: usize,
    pub provider_name: String,
    pub model_id: String,
    pub used_fallback: bool,
    pub remaining: i64,
    pub error_message: Option<String>,
}

impl IndexRunResult {
    /// One line for the status strip.
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error_message {
            if self.embedded_count == 0 {
                return e.clone();
            }
        }
        let fallback = if self.used_fallback {
            " using lexical fallback"
        } else {
            ""
        };
        format!(
            "Indexed {} new chunk{} with {}{fallback}.",
            self.embedded_count,
            if self.embedded_count == 1 { "" } else { "s" },
            self.model_id
        )
    }
}

/// Index state for the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexStatus {
    pub enabled: bool,
    pub auto_index: bool,
    pub configured_model: String,
    /// The model search currently reads (empty before the first index).
    pub active_model: String,
    /// The configured model differs from the active index; a re-index is under way or due.
    pub needs_reindex: bool,
    /// The active index is the lexical fallback.
    pub using_fallback: bool,
    pub pending: i64,
    pub failed: i64,
    pub last_error: Option<String>,
    pub transcription_count: i64,
    pub indexed_transcription_count: i64,
    pub chunk_count: i64,
    pub embedded_chunk_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub signal_count: i64,
    pub latest_indexed_at: Option<i64>,
}

/// The model the index currently answers searches with.
pub fn active_model(store: &Store, cfg: &IndexConfig) -> Result<String> {
    Ok(store
        .kv_get(ACTIVE_MODEL_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.target_model().to_string()))
}

fn stored_active(store: &Store) -> Result<Option<String>> {
    Ok(store
        .kv_get(ACTIVE_MODEL_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

pub fn status(store: &Store, cfg: &IndexConfig) -> Result<IndexStatus> {
    let stored = stored_active(store)?;
    let active = stored.clone().unwrap_or_default();
    let counts = store.semantic_counts(if active.is_empty() {
        cfg.target_model()
    } else {
        &active
    })?;
    let (pending, failed, last_error) = store.semantic_queue_counts(MAX_ATTEMPTS)?;
    Ok(IndexStatus {
        enabled: cfg.enabled,
        auto_index: cfg.auto_index,
        configured_model: cfg.target_model().to_string(),
        needs_reindex: stored.is_some() && active != cfg.target_model(),
        using_fallback: active == LEXICAL_MODEL_ID && cfg.target_model() != LEXICAL_MODEL_ID,
        active_model: active,
        pending,
        failed,
        last_error,
        transcription_count: counts.transcription_count,
        indexed_transcription_count: counts.indexed_transcription_count,
        chunk_count: counts.chunk_count,
        embedded_chunk_count: counts.embedded_chunk_count,
        node_count: counts.node_count,
        edge_count: counts.edge_count,
        signal_count: counts.signal_count,
        latest_indexed_at: counts.latest_indexed_at,
    })
}

/// After a dictation is saved: queue it when semantic memory is on.
pub fn enqueue_after_save(store: &Store, cfg: &IndexConfig, transcription_id: i64) -> Result<bool> {
    if !cfg.enabled {
        return Ok(false);
    }
    Ok(store.semantic_enqueue(&[transcription_id])? > 0)
}

/// "Index Now": queue everything not yet indexed for the configured model
/// and give failed items another round. Returns how many are pending.
pub fn index_now(store: &Store, cfg: &IndexConfig) -> Result<i64> {
    store.semantic_queue_reset_failures()?;
    store.semantic_enqueue_missing(cfg.target_model())?;
    Ok(store.semantic_queue_counts(MAX_ATTEMPTS)?.0)
}

/// "Clear Index": drop the whole semantic memory.
pub fn clear(store: &Store) -> Result<()> {
    store.semantic_clear()?;
    store.kv_set(ACTIVE_MODEL_KEY, None)?;
    store.kv_set(BACKFILL_KEY, None)?;
    Ok(())
}

/// Saved UI offset (minutes east of UTC) for background local-time math.
pub fn clock(store: &Store, now: i64) -> Clock {
    let minutes = store
        .kv_get(TZ_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    Clock::new(now, minutes)
}

pub fn remember_tz(store: &Store, minutes: i64) -> Result<()> {
    store.kv_set(TZ_KEY, Some(&minutes.to_string()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    Primary,
    Lexical,
}

fn embed_texts(
    client: &dyn LlmClient,
    provider: Provider,
    model: &str,
    texts: &[String],
) -> Result<Vec<Vec<f32>>> {
    match provider {
        Provider::Lexical => Ok(texts.iter().map(|t| vector::lexical_embedding(t)).collect()),
        Provider::Primary => {
            let out = client.embed(model, texts)?;
            if out.len() != texts.len() {
                return Err(CoreError::Llm(
                    "Embedding count did not match chunk count.".into(),
                ));
            }
            if out.iter().any(|v| v.is_empty()) {
                return Err(CoreError::Llm(
                    "The embedding model returned an empty vector.".into(),
                ));
            }
            Ok(out)
        }
    }
}

fn embed_chunks(
    store: &Store,
    client: &dyn LlmClient,
    provider: Provider,
    model: &str,
    provider_name: &str,
    chunks: &[&SemanticChunk],
) -> Result<usize> {
    let mut count = 0;
    for batch in chunks.chunks(EMBED_BATCH) {
        let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
        let vectors = embed_texts(client, provider, model, &texts)?;
        for (c, v) in batch.iter().zip(vectors) {
            let n = vector::normalized(&v);
            store.semantic_save_embedding(
                c.id,
                model,
                provider_name,
                &vector::encode(&n),
                n.len(),
            )?;
            count += 1;
        }
    }
    Ok(count)
}

fn store_heuristic_signals(store: &Store, chunks: &[SemanticChunk]) -> Result<usize> {
    let done = store.semantic_signal_chunk_ids(HEURISTIC_MODEL_ID)?;
    let mut added = 0;
    for c in chunks.iter().filter(|c| !done.contains(&c.id)) {
        let rows: Vec<(String, String, f64)> = signals::signals(&c.text)
            .into_iter()
            .map(|s| (s.family.as_str().to_string(), s.value, s.confidence))
            .collect();
        added += store.semantic_insert_signals(c.id, HEURISTIC_MODEL_ID, &rows)?;
    }
    Ok(added)
}

/// Process the queue. Stops after `max_items` dictations (the rest stays
/// queued) or on an unrecoverable embedding error without fallback.
pub fn process_queue(
    store: &Store,
    client: &dyn LlmClient,
    cfg: &IndexConfig,
    clock: Clock,
    max_items: usize,
    progress: &mut dyn FnMut(IndexProgress),
) -> Result<IndexRunResult> {
    let target = cfg.target_model().to_string();
    let mut result = IndexRunResult {
        model_id: target.clone(),
        provider_name: client.provider_name().to_string(),
        ..Default::default()
    };
    if !cfg.enabled {
        result.provider_name = "Disabled".into();
        return Ok(result);
    }
    let mut provider = if target == LEXICAL_MODEL_ID {
        Provider::Lexical
    } else {
        Provider::Primary
    };
    let stored = stored_active(store)?;
    let backfill = store.kv_get(BACKFILL_KEY)?.is_some();
    let model_changed = stored.as_deref().is_some_and(|a| a != target);
    if provider == Provider::Primary && (model_changed || backfill) {
        // Re-index for the new model (or retry fallback-only items) once the
        // server answers; a dead server shouldn't requeue the whole history.
        match embed_texts(client, provider, &target, &["probe".to_string()]) {
            Ok(_) => {
                store.semantic_enqueue_missing(&target)?;
            }
            Err(e) if cfg.keyword_fallback => {
                provider = Provider::Lexical;
                result.used_fallback = true;
                result.error_message = Some(e.to_string());
            }
            Err(e) => {
                result.error_message = Some(e.to_string());
            }
        }
    }

    let total_pending = store.semantic_queue_counts(MAX_ATTEMPTS)?.0.max(0) as usize;
    let total = total_pending.min(max_items);
    let mut processed_chunks: Vec<SemanticChunk> = Vec::new();
    let mut processed = 0usize;
    let mut aborted = false;
    progress(IndexProgress {
        phase: "indexing".into(),
        done: 0,
        total,
        message: format!("Building semantic memory… 0 of {total}"),
    });

    'outer: while processed < max_items {
        let items = store.semantic_queue(
            QUEUE_BATCH.min((max_items - processed) as i64),
            MAX_ATTEMPTS,
        )?;
        if items.is_empty() {
            break;
        }
        for item in items {
            let tid = item.transcription_id;
            let Some(record) = store.get_transcription(tid)? else {
                store.semantic_dequeue(tid)?;
                continue;
            };
            if record.entry_kind == "edit" || record.text.trim().is_empty() {
                store.semantic_delete_chunks_for(tid)?;
                store.semantic_dequeue(tid)?;
                continue;
            }
            let drafts = vector::make_chunks(&record.text);
            let chunks = store.semantic_upsert_chunks(&record, &drafts)?;
            result.source_count += 1;
            result.chunk_count += chunks.len();
            result.signal_count += store_heuristic_signals(store, &chunks)?;

            let model = match provider {
                Provider::Primary => target.as_str(),
                Provider::Lexical => LEXICAL_MODEL_ID,
            };
            let embedded = store.semantic_embedded_chunk_ids(model)?;
            let missing: Vec<&SemanticChunk> = chunks
                .iter()
                .filter(|c| !embedded.contains(&c.id))
                .collect();
            result.skipped_count += chunks.len() - missing.len();
            let provider_name = match provider {
                Provider::Primary => format!("{} Embeddings", client.provider_name()),
                Provider::Lexical => LEXICAL_PROVIDER_NAME.to_string(),
            };
            match embed_chunks(store, client, provider, model, &provider_name, &missing) {
                Ok(n) => {
                    result.embedded_count += n;
                    store.semantic_dequeue(tid)?;
                }
                Err(e) => {
                    let message = e.to_string();
                    store.semantic_queue_failed(tid, &message)?;
                    result.error_message = Some(message);
                    if cfg.keyword_fallback {
                        provider = Provider::Lexical;
                        result.used_fallback = true;
                        store.kv_set(BACKFILL_KEY, Some("1"))?;
                        let lexical = store.semantic_embedded_chunk_ids(LEXICAL_MODEL_ID)?;
                        let missing: Vec<&SemanticChunk> =
                            chunks.iter().filter(|c| !lexical.contains(&c.id)).collect();
                        result.embedded_count += embed_chunks(
                            store,
                            client,
                            Provider::Lexical,
                            LEXICAL_MODEL_ID,
                            LEXICAL_PROVIDER_NAME,
                            &missing,
                        )?;
                        store.semantic_dequeue(tid)?;
                    } else {
                        result.failed_count += 1;
                        aborted = true;
                        processed_chunks.extend(chunks);
                        processed += 1;
                        break 'outer;
                    }
                }
            }
            processed_chunks.extend(chunks);
            processed += 1;
            progress(IndexProgress {
                phase: "indexing".into(),
                done: processed,
                total: total.max(processed),
                message: format!(
                    "Building semantic memory… {processed} of {}",
                    total.max(processed)
                ),
            });
        }
    }

    let (pending, failed, _) = store.semantic_queue_counts(MAX_ATTEMPTS)?;
    result.remaining = pending;
    result.failed_count = result.failed_count.max(failed.max(0) as usize);
    let used_model = match provider {
        Provider::Primary => target.clone(),
        Provider::Lexical => LEXICAL_MODEL_ID.to_string(),
    };
    result.model_id = used_model.clone();
    if provider == Provider::Lexical {
        result.provider_name = LEXICAL_PROVIDER_NAME.into();
    }

    // Switch the active index once the configured model covers everything.
    if !aborted && pending == 0 && provider == Provider::Primary {
        if stored.as_deref() != Some(target.as_str()) || backfill {
            store.kv_set(ACTIVE_MODEL_KEY, Some(&target))?;
        }
        store.kv_set(BACKFILL_KEY, None)?;
        store.semantic_delete_embeddings_except(&target)?;
    } else if stored.is_none() && result.embedded_count > 0 {
        store.kv_set(ACTIVE_MODEL_KEY, Some(&used_model))?;
    }

    if processed > 0 || model_changed {
        let active = active_model(store, cfg)?;
        progress(IndexProgress {
            phase: "graph".into(),
            done: processed,
            total: total.max(processed),
            message: "Linking concepts…".into(),
        });
        let graph = memory_graph::rebuild(store, &active, clock)?;
        result.graph_node_count = graph.nodes.len();
        result.graph_edge_count = graph.edges.len();
        progress(IndexProgress {
            phase: "findings".into(),
            done: processed,
            total: total.max(processed),
            message: "Finding patterns…".into(),
        });
        result.finding_count = findings::compute_and_store(store, clock)?.len();
    }

    // Optional local-model refinement of the newest chunks' signals.
    if cfg.summary_enabled
        && !cfg.summary_model.is_empty()
        && provider == Provider::Primary
        && !processed_chunks.is_empty()
    {
        let done = store.semantic_signal_chunk_ids(&cfg.summary_model)?;
        let mut todo: Vec<&SemanticChunk> = processed_chunks
            .iter()
            .filter(|c| !done.contains(&c.id) && c.word_count >= 8)
            .collect();
        todo.sort_by_key(|a| std::cmp::Reverse(a.source_created_at));
        todo.truncate(REFINE_PER_RUN);
        let n = todo.len();
        for (i, c) in todo.into_iter().enumerate() {
            progress(IndexProgress {
                phase: "refining".into(),
                done: i,
                total: n,
                message: format!("Reading signals with {}… {i} of {n}", cfg.summary_model),
            });
            match signals::refine_with_model(client, &cfg.summary_model, &c.text) {
                Ok(found) => {
                    let mut rows: Vec<(String, String, f64)> = found
                        .into_iter()
                        .map(|s| (s.family.as_str().to_string(), s.value, s.confidence))
                        .collect();
                    if rows.is_empty() {
                        // Marks the chunk as read so it isn't asked again.
                        rows.push(("intent".into(), "reviewed".into(), 0.0));
                    }
                    store.semantic_insert_signals(c.id, &cfg.summary_model, &rows)?;
                    result.refined_count += 1;
                }
                Err(_) => break,
            }
        }
        if result.refined_count > 0 {
            result.finding_count = findings::compute_and_store(store, clock)?.len();
        }
    }

    progress(IndexProgress {
        phase: "done".into(),
        done: processed,
        total: total.max(processed),
        message: result.summary(),
    });
    Ok(result)
}

/// Whether the local server answers and has the models semantic memory uses.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LlmStatus {
    pub reachable: bool,
    pub provider: String,
    pub models: Vec<String>,
    pub embedding_models: Vec<String>,
    pub embedding_model_installed: bool,
    pub summary_model_installed: bool,
    pub error: Option<String>,
}

pub fn canonical_model_name(name: &str) -> String {
    let t = name.trim().to_lowercase();
    if t.is_empty() || t.contains(':') {
        t
    } else {
        format!("{t}:latest")
    }
}

pub fn llm_status(client: &dyn LlmClient, cfg: &IndexConfig) -> LlmStatus {
    match client.list_models() {
        Ok(models) => {
            let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
            let canon: Vec<String> = names.iter().map(|n| canonical_model_name(n)).collect();
            let has = |m: &str| !m.is_empty() && canon.contains(&canonical_model_name(m));
            LlmStatus {
                reachable: true,
                provider: client.provider_name().to_string(),
                embedding_models: models
                    .iter()
                    .filter(|m| m.embedding)
                    .map(|m| m.name.clone())
                    .collect(),
                embedding_model_installed: cfg.target_model() == LEXICAL_MODEL_ID
                    || has(&cfg.embedding_model),
                summary_model_installed: has(&cfg.summary_model),
                models: names,
                error: None,
            }
        }
        Err(e) => LlmStatus {
            reachable: false,
            provider: client.provider_name().to_string(),
            models: Vec::new(),
            embedding_models: Vec::new(),
            embedding_model_installed: false,
            summary_model_installed: false,
            error: Some(e.to_string()),
        },
    }
}

/// First of `candidates` installed on the server (Mac `ConceptBriefWriter.localModel`).
pub fn pick_installed(client: &dyn LlmClient, candidates: &[String]) -> Result<Option<String>> {
    let installed: Vec<String> = client
        .list_models()?
        .into_iter()
        .map(|m| canonical_model_name(&m.name))
        .collect();
    Ok(candidates
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .find(|c| installed.contains(&canonical_model_name(c)))
        .map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatDelta, ChatMessage, ChatOptions, ChatReply, LlmModel, MockLlm};
    use crate::store::TranscriptionRecord;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Embeddings that can be switched off to simulate the server going down.
    struct Flaky {
        inner: MockLlm,
        down: AtomicBool,
        embeds: AtomicUsize,
    }
    impl LlmClient for Flaky {
        fn provider_name(&self) -> &str {
            "Ollama"
        }
        fn list_models(&self) -> Result<Vec<LlmModel>> {
            if self.down.load(Ordering::SeqCst) {
                return Err(CoreError::Llm("connection refused".into()));
            }
            self.inner.list_models()
        }
        fn chat(
            &self,
            m: &[ChatMessage],
            o: &ChatOptions,
            d: Option<&mut dyn FnMut(ChatDelta)>,
        ) -> Result<ChatReply> {
            self.inner.chat(m, o, d)
        }
        fn embed(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            self.embeds.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                return Err(CoreError::Llm("connection refused".into()));
            }
            self.inner.embed(model, inputs)
        }
    }

    fn flaky(down: bool) -> Flaky {
        Flaky {
            inner: MockLlm::fixed(
                r#"{"commitments":["I will ship it Friday"],"questions":[],"decisions":[],"blockers":[]}"#,
            ),
            down: AtomicBool::new(down),
            embeds: AtomicUsize::new(0),
        }
    }

    fn cfg() -> IndexConfig {
        IndexConfig {
            enabled: true,
            auto_index: true,
            embedding_model: "mock-embed".into(),
            keyword_fallback: true,
            summary_enabled: false,
            summary_model: String::new(),
        }
    }

    fn add(store: &Store, text: &str, at: i64) -> i64 {
        let r = TranscriptionRecord::new(text, Some("Code".into()), 4000, at);
        store.insert_transcription(&r).unwrap()
    }

    #[test]
    fn processes_queue_with_mock_embeddings() {
        let store = Store::open_in_memory().unwrap();
        let cfg = cfg();
        let a = add(&store, "I will fix the memory graph layout tomorrow.", 1000);
        let b = add(&store, &"word ".repeat(300), 2000);
        assert!(enqueue_after_save(&store, &cfg, a).unwrap());
        enqueue_after_save(&store, &cfg, b).unwrap();
        let client = flaky(false);
        let mut events = Vec::new();
        let r = process_queue(&store, &client, &cfg, Clock::utc(3000), 100, &mut |p| {
            events.push(p)
        })
        .unwrap();
        assert_eq!(r.source_count, 2);
        assert_eq!(r.chunk_count, 1 + 3);
        assert_eq!(r.embedded_count, 4);
        assert_eq!(r.remaining, 0);
        assert!(!r.used_fallback);
        assert!(r.signal_count > 0);
        assert!(r.graph_node_count > 0);
        assert_eq!(events.last().unwrap().phase, "done");
        assert!(events.iter().any(|e| e.phase == "graph"));
        let st = status(&store, &cfg).unwrap();
        assert_eq!(st.active_model, "mock-embed");
        assert_eq!(st.embedded_chunk_count, 4);
        assert_eq!(st.indexed_transcription_count, 2);
        assert!(!st.needs_reindex);
        // Re-running is a no-op.
        let again =
            process_queue(&store, &client, &cfg, Clock::utc(3000), 100, &mut |_| {}).unwrap();
        assert_eq!(again.source_count, 0);
        // Disabled config does nothing.
        let off = IndexConfig {
            enabled: false,
            ..cfg.clone()
        };
        assert!(!enqueue_after_save(&store, &off, a).unwrap());
        assert_eq!(
            process_queue(&store, &client, &off, Clock::utc(0), 10, &mut |_| {})
                .unwrap()
                .provider_name,
            "Disabled"
        );
    }

    #[test]
    fn falls_back_to_lexical_then_backfills_when_server_returns() {
        let store = Store::open_in_memory().unwrap();
        let cfg = cfg();
        let a = add(
            &store,
            "Deploy the docker container to production servers.",
            1000,
        );
        store.semantic_enqueue(&[a]).unwrap();
        let client = flaky(true);
        let r = process_queue(&store, &client, &cfg, Clock::utc(2000), 100, &mut |_| {}).unwrap();
        assert!(r.used_fallback);
        assert_eq!(r.model_id, LEXICAL_MODEL_ID);
        assert_eq!(r.remaining, 0);
        assert_eq!(status(&store, &cfg).unwrap().active_model, LEXICAL_MODEL_ID);
        assert!(status(&store, &cfg).unwrap().using_fallback);
        // Server back: the dictation is re-embedded with the real model.
        client.down.store(false, Ordering::SeqCst);
        let r2 = process_queue(&store, &client, &cfg, Clock::utc(3000), 100, &mut |_| {}).unwrap();
        assert!(!r2.used_fallback);
        assert_eq!(r2.embedded_count, 1);
        let st = status(&store, &cfg).unwrap();
        assert_eq!(st.active_model, "mock-embed");
        assert!(!st.using_fallback);
        assert_eq!(
            store.semantic_embedding_models().unwrap(),
            vec!["mock-embed".to_string()]
        );
    }

    #[test]
    fn retries_without_fallback_then_gives_up() {
        let store = Store::open_in_memory().unwrap();
        let cfg = IndexConfig {
            keyword_fallback: false,
            ..cfg()
        };
        let a = add(&store, "Some words to embed here.", 1000);
        store.semantic_enqueue(&[a]).unwrap();
        let client = flaky(true);
        for _ in 0..MAX_ATTEMPTS {
            let r =
                process_queue(&store, &client, &cfg, Clock::utc(2000), 100, &mut |_| {}).unwrap();
            assert!(r.error_message.is_some());
        }
        let st = status(&store, &cfg).unwrap();
        assert_eq!(st.pending, 0);
        assert_eq!(st.failed, 1);
        assert!(st.last_error.unwrap().contains("refused"));
        // Index Now resets failures.
        client.down.store(false, Ordering::SeqCst);
        assert_eq!(index_now(&store, &cfg).unwrap(), 1);
        let r = process_queue(&store, &client, &cfg, Clock::utc(2000), 100, &mut |_| {}).unwrap();
        assert_eq!(r.embedded_count, 1);
        assert_eq!(status(&store, &cfg).unwrap().failed, 0);
    }

    #[test]
    fn model_change_reindexes_then_switches() {
        let store = Store::open_in_memory().unwrap();
        let cfg = cfg();
        for i in 0..3 {
            let id = add(
                &store,
                &format!("dictation number {i} about the garden"),
                1000 + i,
            );
            store.semantic_enqueue(&[id]).unwrap();
        }
        let client = flaky(false);
        process_queue(&store, &client, &cfg, Clock::utc(5000), 100, &mut |_| {}).unwrap();
        let next = IndexConfig {
            embedding_model: "nomic-embed-text".into(),
            ..cfg.clone()
        };
        let st = status(&store, &next).unwrap();
        assert!(st.needs_reindex);
        assert_eq!(st.active_model, "mock-embed");
        // A partial pass keeps the old index active.
        let r = process_queue(&store, &client, &next, Clock::utc(5000), 2, &mut |_| {}).unwrap();
        assert_eq!(r.remaining, 1);
        assert_eq!(status(&store, &next).unwrap().active_model, "mock-embed");
        process_queue(&store, &client, &next, Clock::utc(5000), 100, &mut |_| {}).unwrap();
        let st = status(&store, &next).unwrap();
        assert_eq!(st.active_model, "nomic-embed-text");
        assert!(!st.needs_reindex);
        assert_eq!(
            store.semantic_embedding_models().unwrap(),
            vec!["nomic-embed-text".to_string()]
        );
        clear(&store).unwrap();
        assert_eq!(status(&store, &next).unwrap().chunk_count, 0);
        assert_eq!(status(&store, &next).unwrap().active_model, "");
    }

    #[test]
    fn refines_signals_with_summary_model() {
        let store = Store::open_in_memory().unwrap();
        let cfg = IndexConfig {
            summary_enabled: true,
            summary_model: "mock-chat".into(),
            ..cfg()
        };
        let a = add(
            &store,
            "We talked through the release plan and the Friday ship date in detail.",
            1000,
        );
        store.semantic_enqueue(&[a]).unwrap();
        let client = flaky(false);
        let r = process_queue(&store, &client, &cfg, Clock::utc(2000), 100, &mut |_| {}).unwrap();
        assert_eq!(r.refined_count, 1);
        let sigs = store
            .semantic_signals(Some(&["commitment"]), None, 10)
            .unwrap();
        assert!(sigs.iter().any(|s| s.model_id == "mock-chat"));
    }

    #[test]
    fn llm_status_and_model_picking() {
        let mock = MockLlm::fixed("x");
        let st = llm_status(&mock, &cfg());
        assert!(st.reachable);
        assert!(st.embedding_model_installed);
        assert_eq!(st.embedding_models, vec!["mock-embed".to_string()]);
        let off = llm_status(&flaky(true), &cfg());
        assert!(!off.reachable);
        assert!(off.error.is_some());
        assert_eq!(
            pick_installed(&mock, &["missing".into(), "mock-chat:latest".into()]).unwrap(),
            Some("mock-chat:latest".into())
        );
        assert_eq!(canonical_model_name("Gemma"), "gemma:latest");
    }
}
