//! Semantic memory: chunking, embeddings, search, concept graph, signals,
//! insight findings and work-activity summaries.
//!
//! CONTRACT (used by Chat AI): [`search`] keeps this exact signature. This
//! initial version is a keyword fallback; the semantic area replaces the body
//! with embedding search + keyword fallback.

use crate::error::Result;
use crate::llm::LlmClient;
use crate::store::Store;
use serde::Serialize;

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

/// Retrieve the `limit` passages most relevant to `query`.
pub fn search(
    store: &Store,
    _client: &dyn LlmClient,
    _opts: &SemanticOptions,
    query: &str,
    limit: usize,
) -> Result<Vec<SemanticHit>> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_lowercase())
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits: Vec<SemanticHit> = store
        .recent(2000)?
        .into_iter()
        .filter_map(|r| {
            let lower = r.text.to_lowercase();
            let matched = terms.iter().filter(|t| lower.contains(t.as_str())).count();
            (matched > 0).then(|| SemanticHit {
                chunk_id: None,
                transcription_id: r.id.unwrap_or(0),
                text: r.text,
                created_at: r.created_at,
                app: r.app,
                score: matched as f32 / terms.len() as f32,
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.created_at.cmp(&a.created_at)));
    hits.truncate(limit);
    Ok(hits)
}
