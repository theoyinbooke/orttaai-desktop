//! Semantic memory: chunking, embeddings, search, concept graph, signals,
//! insight findings and work-activity summaries — the port of the macOS
//! `Core/SemanticMemory` stack.
//!
//! - [`index`] — the index queue, embeddings, active-model switching, status.
//! - [`analyzer`] / [`concepts`] — the portable NaturalLanguage replacement and
//!   per-dictation concept extraction.
//! - [`concept_graph`] — the Map's concept graph (themes, trends, highlights).
//! - [`memory_graph`] — the stored chunk/topic/entity/app graph.
//! - [`signals`] — commitments, questions, decisions, blockers, tone, intent.
//! - [`findings`] — the deterministic pattern engine and Home ranking.
//! - [`report`] — the Insights report (+ optional local-model phrasing).
//! - [`activity`] — working sessions, their readings and the Activity report.
//! - [`service`] — store-backed orchestration used by the app.
//!
//! CONTRACT (used by Chat AI): [`search`] keeps this exact signature.

pub mod activity;
pub mod analyzer;
pub mod concept_graph;
pub mod concepts;
pub mod findings;
pub mod index;
pub mod memory_graph;
pub mod report;
pub mod service;
pub mod signals;
pub mod util;
pub mod vector;

use crate::error::Result;
use crate::llm::LlmClient;
use crate::store::Store;
use serde::Serialize;
use std::collections::HashSet;

/// Options for semantic retrieval, derived from settings.
#[derive(Debug, Clone)]
pub struct SemanticOptions {
    pub enabled: bool,
    pub embedding_model: String,
    pub keyword_fallback: bool,
}

impl SemanticOptions {
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        Self {
            enabled: s.semantic_enabled,
            embedding_model: s.semantic_embedding_model.clone(),
            keyword_fallback: s.semantic_keyword_fallback,
        }
    }
}

/// One retrieved passage of the user's own dictations.
#[derive(Debug, Clone, Serialize)]
pub struct SemanticHit {
    pub chunk_id: Option<i64>,
    pub transcription_id: i64,
    pub text: String,
    pub created_at: i64,
    pub app: Option<String>,
    /// Higher is more relevant (cosine similarity or keyword score, 0..=1).
    pub score: f32,
}

/// Cosine floor for a vector hit (Mac `minimumScore`).
pub const MIN_VECTOR_SCORE: f64 = 0.16;
/// Chunks scanned per search (Mac `fetchEmbeddedSemanticChunks(limit: 1500)`).
const SEARCH_CHUNKS: i64 = 1500;

/// Score desc; near-ties (< 0.0001) go to the more recent passage.
fn rank(hits: &mut [SemanticHit]) {
    hits.sort_by(|a, b| {
        if (a.score - b.score).abs() < 0.0001 {
            b.created_at.cmp(&a.created_at)
        } else {
            b.score.total_cmp(&a.score)
        }
    });
}

fn vector_hits(
    store: &Store,
    client: &dyn LlmClient,
    model: &str,
    query: &str,
) -> Result<Vec<SemanticHit>> {
    let chunks = store.semantic_embedded_chunks(model, SEARCH_CHUNKS)?;
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let q = if model == vector::LEXICAL_MODEL_ID {
        vector::lexical_embedding(query)
    } else {
        client
            .embed(model, &[query.to_string()])?
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    if q.is_empty() {
        return Ok(Vec::new());
    }
    Ok(chunks
        .into_iter()
        .filter_map(|ec| {
            let v = vector::decode(&ec.vector, ec.dimension)?;
            let score = vector::cosine(&q, &v);
            (score >= MIN_VECTOR_SCORE).then_some(SemanticHit {
                chunk_id: Some(ec.chunk.id),
                transcription_id: ec.chunk.transcription_id,
                text: ec.chunk.text,
                created_at: ec.chunk.source_created_at,
                app: ec.chunk.app_name,
                score: score as f32,
            })
        })
        .collect())
}

/// Share of query terms each passage contains (indexed chunks when there
/// are any, otherwise raw dictations).
fn keyword_hits(store: &Store, query: &str) -> Result<Vec<SemanticHit>> {
    let mut terms: Vec<String> = vector::lexical_tokens(query);
    if terms.is_empty() {
        terms = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.chars().count() > 2)
            .map(|t| t.to_lowercase())
            .collect();
    }
    let mut seen = HashSet::new();
    terms.retain(|t| seen.insert(t.clone()));
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let score = |text: &str| {
        let lower = text.to_lowercase();
        let matched = terms
            .iter()
            .filter(|t| concepts::contains_bounded(&lower, t) || lower.contains(t.as_str()))
            .count();
        matched as f32 / terms.len() as f32
    };
    let chunks = store.semantic_chunks(4000)?;
    let hits: Vec<SemanticHit> = if chunks.is_empty() {
        store
            .recent(2000)?
            .into_iter()
            .filter(|r| r.entry_kind != "edit")
            .filter_map(|r| {
                let s = score(&r.text);
                (s > 0.0).then_some(SemanticHit {
                    chunk_id: None,
                    transcription_id: r.id.unwrap_or(0),
                    text: r.text,
                    created_at: r.created_at,
                    app: r.app,
                    score: s,
                })
            })
            .collect()
    } else {
        chunks
            .into_iter()
            .filter_map(|c| {
                let s = score(&c.text);
                (s > 0.0).then_some(SemanticHit {
                    chunk_id: Some(c.id),
                    transcription_id: c.transcription_id,
                    text: c.text,
                    created_at: c.source_created_at,
                    app: c.app_name,
                    score: s,
                })
            })
            .collect()
    };
    Ok(hits)
}

/// Retrieve the `limit` passages most relevant to `query`: cosine search over
/// the active index, topped up with keyword matches when
/// `semantic_keyword_fallback` is on (also used alone when the embedding
/// server is unreachable or semantic memory is off).
pub fn search(
    store: &Store,
    client: &dyn LlmClient,
    opts: &SemanticOptions,
    query: &str,
    limit: usize,
) -> Result<Vec<SemanticHit>> {
    let query = query.trim();
    if query.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    if opts.enabled {
        let cfg = index::IndexConfig {
            enabled: true,
            auto_index: true,
            embedding_model: opts.embedding_model.trim().to_string(),
            keyword_fallback: opts.keyword_fallback,
            summary_enabled: false,
            summary_model: String::new(),
        };
        let model = index::active_model(store, &cfg)?;
        match vector_hits(store, client, &model, query) {
            Ok(v) => hits = v,
            Err(e) if opts.keyword_fallback => {
                tracing::warn!("semantic search fell back to keywords: {e}");
            }
            Err(e) => return Err(e),
        }
        rank(&mut hits);
        hits.truncate(limit);
    }
    if hits.len() < limit && opts.keyword_fallback {
        let have: HashSet<(Option<i64>, i64)> = hits
            .iter()
            .map(|h| (h.chunk_id, h.transcription_id))
            .collect();
        let have_tx: HashSet<i64> = hits.iter().map(|h| h.transcription_id).collect();
        let mut extra: Vec<SemanticHit> = keyword_hits(store, query)?
            .into_iter()
            .filter(|h| !have.contains(&(h.chunk_id, h.transcription_id)))
            .filter(|h| h.chunk_id.is_some() || !have_tx.contains(&h.transcription_id))
            .collect();
        rank(&mut extra);
        hits.extend(extra.into_iter().take(limit - hits.len()));
    }
    Ok(hits)
}

/// Numbered context block for prompts (Mac `contextBlock`).
pub fn context_block(hits: &[SemanticHit], clock: util::Clock) -> String {
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            let app = h
                .app
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .unwrap_or("Unknown App");
            format!(
                "[{}] {app}, {}, relevance {}%: {}",
                i + 1,
                clock.medium(h.created_at),
                (h.score * 100.0).round() as i64,
                h.text.replace('\n', " ").trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;
    use crate::store::TranscriptionRecord;

    fn opts(enabled: bool, fallback: bool) -> SemanticOptions {
        SemanticOptions {
            enabled,
            embedding_model: "mock-embed".into(),
            keyword_fallback: fallback,
        }
    }

    fn seed(store: &Store) {
        for (i, t) in [
            "deploy the docker container to the production cluster",
            "grandma's birthday cake recipe with lemon frosting",
            "docker compose file for the staging cluster deploy",
        ]
        .iter()
        .enumerate()
        {
            let r = TranscriptionRecord::new(*t, Some("Code".into()), 1000, 1000 + i as i64 * 10);
            store.insert_transcription(&r).unwrap();
        }
    }

    #[test]
    fn vector_search_ranks_by_similarity_with_recency_tiebreak() {
        let store = Store::open_in_memory().unwrap();
        seed(&store);
        let client = MockLlm::fixed("x");
        let cfg = index::IndexConfig::from_settings(&crate::settings::Settings {
            semantic_enabled: true,
            semantic_embedding_model: "mock-embed".into(),
            ..Default::default()
        });
        index::index_now(&store, &cfg).unwrap();
        index::process_queue(
            &store,
            &client,
            &cfg,
            util::Clock::utc(5000),
            100,
            &mut |_| {},
        )
        .unwrap();
        let hits = search(
            &store,
            &client,
            &opts(true, false),
            "docker cluster deploy",
            3,
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].chunk_id.is_some());
        assert!(hits[0].text.contains("docker"));
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score - 0.0001));
        // Identical passages tie → the newer one wins.
        let mut tied = vec![
            SemanticHit {
                chunk_id: Some(1),
                transcription_id: 1,
                text: "a".into(),
                created_at: 1,
                app: None,
                score: 0.5,
            },
            SemanticHit {
                chunk_id: Some(2),
                transcription_id: 2,
                text: "a".into(),
                created_at: 9,
                app: None,
                score: 0.50001,
            },
        ];
        rank(&mut tied);
        assert_eq!(tied[0].created_at, 9);
        assert!(search(&store, &client, &opts(true, false), "  ", 3)
            .unwrap()
            .is_empty());
        let block = context_block(&hits[..1], util::Clock::utc(0));
        assert!(block.starts_with("[1] Code, "));
    }

    #[test]
    fn keyword_fallback_without_index_or_when_disabled() {
        let store = Store::open_in_memory().unwrap();
        seed(&store);
        let client = MockLlm::fixed("x");
        // Disabled + fallback → keyword hits over raw dictations.
        let hits = search(&store, &client, &opts(false, true), "docker cluster", 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].chunk_id.is_none());
        assert!((hits[0].score - 1.0).abs() < 1e-6);
        assert_eq!(hits[0].created_at, 1020, "recency breaks the tie");
        // Disabled without fallback → nothing.
        assert!(search(&store, &client, &opts(false, false), "docker", 5)
            .unwrap()
            .is_empty());
        // Enabled but never indexed → keyword fallback fills in.
        let hits = search(&store, &client, &opts(true, true), "lemon cake", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("lemon"));
    }
}
