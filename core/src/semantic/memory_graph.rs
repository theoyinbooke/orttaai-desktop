//! The stored semantic memory graph (Mac `SemanticMemoryService.rebuildGraph`):
//! chunk, app, topic and entity nodes joined by app-context / mentions /
//! entity edges plus embedding-similarity edges between chunks. Feeds the
//! pattern engine and the Insights report.

use super::analyzer;
use super::util::{sha256_hex, Clock};
use super::vector;
use crate::error::Result;
use crate::store::{EmbeddedChunk, GraphEdgeRecord, GraphNodeRecord, Store};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const MAX_NODES: usize = 240;
pub const MAX_EDGES: usize = 1200;
/// Similarity edges may use at most this share of the edge budget, so the
/// concept edges insights depend on are never crowded out by near-duplicates.
const SEMANTIC_EDGE_SHARE: f64 = 0.25;
pub const SOURCE_CHUNKS: i64 = 600;
const SIMILARITY_THRESHOLD: f64 = 0.74;
const MAX_NEIGHBORS_PER_CHUNK: usize = 3;

/// Nodes + edges of the stored graph.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryGraph {
    pub nodes: Vec<GraphNodeRecord>,
    pub edges: Vec<GraphEdgeRecord>,
}

/// "Orttaai Desktop" → "orttaai-desktop" (falls back to a hash for non-ASCII).
pub fn node_key(value: &str) -> String {
    let mut key = String::new();
    let mut dash = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            key.push(c);
            dash = false;
        } else if !dash {
            key.push('-');
            dash = true;
        }
    }
    let key = key.trim_matches('-').to_string();
    if key.is_empty() {
        sha256_hex(value, 6)
    } else {
        key
    }
}

/// First seven words, trimmed, capped at 64 chars.
pub fn chunk_title(text: &str) -> String {
    let words = text
        .split_whitespace()
        .take(7)
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = words.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    if trimmed.is_empty() {
        return "Dictation chunk".into();
    }
    if trimmed.chars().count() > 64 {
        format!("{}...", trimmed.chars().take(61).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

/// Top three topics of a chunk, a phrase standing in for the words it covers
/// ("onboarding flow" makes "flow" redundant).
fn graph_topics(text: &str, entity_keys: &HashSet<String>) -> Vec<analyzer::SemanticConcept> {
    let all: Vec<analyzer::SemanticConcept> = analyzer::topic_concepts(text, 8)
        .into_iter()
        .filter(|c| !entity_keys.contains(&c.key))
        .collect();
    let covered: HashSet<String> = all
        .iter()
        .filter(|c| c.key.contains(' '))
        .flat_map(|c| c.key.split(' ').map(String::from).collect::<Vec<_>>())
        .collect();
    all.into_iter()
        .filter(|c| c.key.contains(' ') || !covered.contains(&c.key))
        .take(3)
        .collect()
}

/// Build the graph from embedded chunks (newest first).
pub fn build(chunks: &[EmbeddedChunk], clock: Clock) -> MemoryGraph {
    let mut nodes: HashMap<String, GraphNodeRecord> = HashMap::new();
    let mut edges: HashMap<(String, String, String), GraphEdgeRecord> = HashMap::new();
    let mut mention_counts: HashMap<String, usize> = HashMap::new();
    let mut mention_days: HashMap<String, HashSet<i64>> = HashMap::new();
    let mut vectors: HashMap<i64, Vec<f32>> = HashMap::new();

    let upsert_node = |nodes: &mut HashMap<String, GraphNodeRecord>,
                       id: String,
                       kind: &str,
                       title: &str,
                       subtitle: Option<String>,
                       weight: f64,
                       last_seen: i64| {
        if id.is_empty() || title.is_empty() {
            return;
        }
        nodes
            .entry(id.clone())
            .and_modify(|n| {
                n.weight += weight;
                n.last_seen_at = Some(n.last_seen_at.map_or(last_seen, |l| l.max(last_seen)));
            })
            .or_insert(GraphNodeRecord {
                node_id: id,
                kind: kind.into(),
                title: title.into(),
                subtitle,
                weight,
                last_seen_at: Some(last_seen),
            });
    };
    let upsert_edge = |edges: &mut HashMap<(String, String, String), GraphEdgeRecord>,
                       a: &str,
                       b: &str,
                       kind: &str,
                       weight: f64,
                       evidence: Option<String>| {
        if a == b {
            return;
        }
        let (s, t) = if a < b { (a, b) } else { (b, a) };
        edges
            .entry((s.into(), t.into(), kind.into()))
            .and_modify(|e| e.weight = e.weight.max(weight))
            .or_insert(GraphEdgeRecord {
                source_node_id: s.into(),
                target_node_id: t.into(),
                kind: kind.into(),
                weight,
                evidence,
            });
    };

    for ec in chunks {
        let c = &ec.chunk;
        let chunk_node = format!("chunk:{}", c.id);
        upsert_node(
            &mut nodes,
            chunk_node.clone(),
            "chunk",
            &chunk_title(&c.text),
            Some(c.app_name.clone().unwrap_or_else(|| "Dictation".into())),
            (c.word_count as f64 / 18.0).max(1.0),
            c.source_created_at,
        );
        if let Some(app) = c
            .app_name
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            let app_node = format!("app:{}", node_key(app));
            upsert_node(
                &mut nodes,
                app_node.clone(),
                "app",
                app,
                Some("App context".into()),
                1.5,
                c.source_created_at,
            );
            upsert_edge(
                &mut edges,
                &app_node,
                &chunk_node,
                "app-context",
                0.62,
                Some(format!("Dictated in {app}")),
            );
        }
        let entities = analyzer::named_entities(&c.text, 3);
        let entity_keys: HashSet<String> = entities.iter().map(|e| e.key.clone()).collect();
        for concept in graph_topics(&c.text, &entity_keys) {
            let id = format!("topic:{}", node_key(&concept.key));
            upsert_node(
                &mut nodes,
                id.clone(),
                "topic",
                &concept.title,
                None,
                1.2,
                c.source_created_at,
            );
            *mention_counts.entry(id.clone()).or_default() += 1;
            mention_days
                .entry(id.clone())
                .or_default()
                .insert(clock.day(c.source_created_at));
            upsert_edge(
                &mut edges,
                &id,
                &chunk_node,
                "mentions",
                0.56,
                Some(concept.title.clone()),
            );
        }
        for entity in entities {
            let id = format!("entity:{}", node_key(&entity.key));
            upsert_node(
                &mut nodes,
                id.clone(),
                "entity",
                &entity.title,
                Some(entity.category.clone()),
                1.45,
                c.source_created_at,
            );
            *mention_counts.entry(id.clone()).or_default() += 1;
            mention_days
                .entry(id.clone())
                .or_default()
                .insert(clock.day(c.source_created_at));
            upsert_edge(
                &mut edges,
                &id,
                &chunk_node,
                "entity",
                0.64,
                Some(entity.title.clone()),
            );
        }
        if let Some(v) = vector::decode(&ec.vector, ec.dimension) {
            vectors.insert(c.id, v);
        }
    }

    // Recurrence is computed, not asserted.
    for (id, count) in &mention_counts {
        if let Some(node) = nodes.get_mut(id) {
            let days = mention_days.get(id).map(HashSet::len).unwrap_or(0);
            let recurrence = if *count >= 3 && days >= 2 {
                format!(
                    "Recurring · {count} mentions over {days} day{}",
                    if days == 1 { "" } else { "s" }
                )
            } else {
                format!("{count} mention{}", if *count == 1 { "" } else { "s" })
            };
            node.subtitle = Some(match node.subtitle.as_deref() {
                Some(cat) if !cat.is_empty() => format!("{cat} · {recurrence}"),
                _ => recurrence,
            });
        }
    }

    // Semantic similarity across the corpus, keeping each chunk's top matches.
    let comparable: Vec<&EmbeddedChunk> = chunks
        .iter()
        .filter(|c| vectors.contains_key(&c.chunk.id))
        .collect();
    for (i, left) in comparable.iter().enumerate() {
        let lv = &vectors[&left.chunk.id];
        let mut neighbors: Vec<(i64, f64)> = comparable[i + 1..]
            .iter()
            .filter_map(|right| {
                let score = vector::cosine(lv, &vectors[&right.chunk.id]);
                (score >= SIMILARITY_THRESHOLD).then_some((right.chunk.id, score))
            })
            .collect();
        neighbors.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (other, score) in neighbors.into_iter().take(MAX_NEIGHBORS_PER_CHUNK) {
            upsert_edge(
                &mut edges,
                &format!("chunk:{}", left.chunk.id),
                &format!("chunk:{other}"),
                "semantic",
                score.min(1.0),
                Some(format!(
                    "Semantic similarity {}%",
                    (score * 100.0).round() as i64
                )),
            );
        }
    }

    let mut ranked: Vec<GraphNodeRecord> = nodes.into_values().collect();
    ranked.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    ranked.truncate(MAX_NODES);
    let kept: HashSet<&str> = ranked.iter().map(|n| n.node_id.as_str()).collect();
    let mut ranked_edges: Vec<GraphEdgeRecord> = edges
        .into_values()
        .filter(|e| {
            kept.contains(e.source_node_id.as_str()) && kept.contains(e.target_node_id.as_str())
        })
        .collect();
    ranked_edges.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.source_node_id.cmp(&b.source_node_id))
            .then_with(|| a.target_node_id.cmp(&b.target_node_id))
    });
    let (semantic, structural): (Vec<GraphEdgeRecord>, Vec<GraphEdgeRecord>) =
        ranked_edges.into_iter().partition(|e| e.kind == "semantic");
    let semantic_cap = ((MAX_EDGES as f64 * SEMANTIC_EDGE_SHARE) as usize)
        .max(MAX_EDGES.saturating_sub(structural.len()));
    let semantic_take = semantic.len().min(semantic_cap);
    let structural_take = structural.len().min(MAX_EDGES - semantic_take);
    let mut ranked_edges: Vec<GraphEdgeRecord> =
        structural.into_iter().take(structural_take).collect();
    ranked_edges.extend(semantic.into_iter().take(semantic_take));
    ranked_edges.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.source_node_id.cmp(&b.source_node_id))
            .then_with(|| a.target_node_id.cmp(&b.target_node_id))
    });
    MemoryGraph {
        nodes: ranked,
        edges: ranked_edges,
    }
}

/// Rebuild and persist the stored graph from the active model's embeddings.
pub fn rebuild(store: &Store, model_id: &str, clock: Clock) -> Result<MemoryGraph> {
    let chunks = store.semantic_embedded_chunks(model_id, SOURCE_CHUNKS)?;
    let graph = build(&chunks, clock);
    store.semantic_replace_graph(&graph.nodes, &graph.edges)?;
    Ok(graph)
}

/// Load the stored graph.
pub fn load_default(store: &Store) -> Result<MemoryGraph> {
    load(store, MAX_NODES as i64, MAX_EDGES)
}

pub fn load(store: &Store, limit_nodes: i64, limit_edges: usize) -> Result<MemoryGraph> {
    let (nodes, edges) = store.semantic_graph(limit_nodes, limit_edges)?;
    Ok(MemoryGraph { nodes, edges })
}

/// Stable signature for report freshness (Mac `graphSignature`).
pub fn signature(graph: &MemoryGraph) -> String {
    let node_part = graph
        .nodes
        .iter()
        .map(|n| format!("{}:{}", n.node_id, n.weight))
        .collect::<Vec<_>>()
        .join("|");
    let edge_part = graph
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| format!("{i}:{}>{}:{}", e.source_node_id, e.target_node_id, e.weight))
        .collect::<Vec<_>>()
        .join("|");
    let raw = format!(
        "{}#{}#{node_part}#{edge_part}",
        graph.nodes.len(),
        graph.edges.len()
    );
    format!(
        "{}#{}#{}",
        graph.nodes.len(),
        graph.edges.len(),
        sha256_hex(&raw, 12)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SemanticChunk;

    pub(crate) fn embedded(id: i64, text: &str, app: &str, ts: i64, v: &[f32]) -> EmbeddedChunk {
        EmbeddedChunk {
            chunk: SemanticChunk {
                id,
                transcription_id: id,
                chunk_index: 0,
                text: text.into(),
                text_hash: String::new(),
                source_created_at: ts,
                app_name: Some(app.into()),
                word_count: text.split_whitespace().count() as i64,
            },
            model_id: "m".into(),
            dimension: v.len(),
            vector: vector::encode(v),
        }
    }

    #[test]
    fn builds_typed_nodes_and_edges() {
        let chunks = vec![
            embedded(
                1,
                "The memory graph needs a new layout for Sarah Chen.",
                "Code",
                3 * 86_400,
                &[1.0, 0.0],
            ),
            embedded(
                2,
                "Polish the memory graph with Sarah Chen again.",
                "Code",
                2 * 86_400,
                &[0.99, 0.05],
            ),
            embedded(
                3,
                "Groceries: milk and eggs for the weekend.",
                "Notes",
                86_400,
                &[0.0, 1.0],
            ),
            embedded(4, "The memory graph shipped today.", "Mail", 0, &[0.9, 0.1]),
        ];
        let g = build(&chunks, Clock::utc(4 * 86_400));
        let kinds: HashSet<&str> = g.nodes.iter().map(|n| n.kind.as_str()).collect();
        for k in ["chunk", "app", "topic", "entity"] {
            assert!(kinds.contains(k), "missing {k}: {kinds:?}");
        }
        let topic = g
            .nodes
            .iter()
            .find(|n| n.node_id == "topic:memory-graph")
            .unwrap();
        assert!(topic
            .subtitle
            .as_deref()
            .unwrap()
            .starts_with("Recurring · 3 mentions"));
        let entity = g
            .nodes
            .iter()
            .find(|n| n.node_id == "entity:sarah-chen")
            .unwrap();
        assert!(entity.subtitle.as_deref().unwrap().starts_with("Person · "));
        assert!(g.edges.iter().any(|e| e.kind == "semantic"
            && e.source_node_id == "chunk:1"
            && e.target_node_id == "chunk:2"));
        assert!(!g.edges.iter().any(|e| e.kind == "semantic"
            && (e.source_node_id == "chunk:3" || e.target_node_id == "chunk:3")));
        assert!(g.edges.iter().any(|e| e.kind == "app-context"));
        let sig = signature(&g);
        assert_eq!(sig, signature(&g));
        assert!(sig.starts_with(&format!("{}#", g.nodes.len())));
        assert_eq!(node_key("Orttaai Desktop!"), "orttaai-desktop");
        assert_eq!(chunk_title("  "), "Dictation chunk");
    }
}
