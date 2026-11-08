//! The Insights tab report (Mac `SemanticInsightReport`): deterministic cards,
//! clusters, comparisons and charts computed from the stored memory graph and
//! chunks, merged with the pattern-engine findings, then optionally phrased by
//! a local model ("deterministic spine, LLM voice"). Every claim stays linked
//! to evidence chunks.

use super::findings::{kind_action, kind_label};
use super::memory_graph::{self, node_key, MemoryGraph};
use super::util::{excerpt, Clock};
use super::vector;
use crate::llm::{ChatMessage, ChatOptions, LlmClient};
use crate::store::{GraphEdgeRecord, GraphNodeRecord, InsightFinding, SemanticChunk};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const FALLBACK_PREFIX: &str = "Local model fallback:";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightEvidence {
    pub id: String,
    pub chunk_id: i64,
    pub title: String,
    pub excerpt: String,
    pub source_app_name: Option<String>,
    pub source_created_at: Option<i64>,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightCard {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub action_text: String,
    pub related_node_ids: Vec<String>,
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightCluster {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub related_node_ids: Vec<String>,
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightComparison {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub trend: String,
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightChartPoint {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub detail: Option<String>,
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightChart {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: String,
    pub unit: String,
    pub points: Vec<InsightChartPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightReport {
    pub generated_at: i64,
    pub graph_signature: String,
    pub analyzer_name: String,
    pub used_fallback: bool,
    pub summary: Vec<String>,
    pub summary_model_name: Option<String>,
    pub clusters: Vec<InsightCluster>,
    pub comparisons: Vec<InsightComparison>,
    pub coverage_notes: Vec<String>,
    pub charts: Vec<InsightChart>,
    pub cards: Vec<InsightCard>,
    pub source_node_count: usize,
    pub source_edge_count: usize,
    pub source_chunk_count: usize,
}

struct NodeContext<'a> {
    node: &'a GraphNodeRecord,
    neighbor_ids: BTreeSet<String>,
    neighbor_kinds: BTreeSet<String>,
    chunks: Vec<&'a SemanticChunk>,
    app_names: BTreeSet<String>,
    latest_seen_at: Option<i64>,
    weighted_degree: f64,
}

impl NodeContext<'_> {
    fn degree(&self) -> usize {
        self.neighbor_ids.len()
    }
    fn evidence_count(&self) -> usize {
        self.chunks.len()
    }
    fn importance(&self) -> f64 {
        self.node.weight
            + self.weighted_degree
            + self.degree() as f64 * 0.32
            + self.app_names.len() as f64 * 0.4
    }
    fn bridge(&self) -> f64 {
        self.neighbor_kinds.len() as f64 * 1.4
            + self.app_names.len() as f64 * 1.1
            + self.weighted_degree
            + self.degree() as f64 * 0.18
    }
    fn is_concept(&self) -> bool {
        self.node.kind == "topic" || self.node.kind == "entity"
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn confidence(base: f64, evidence: usize, degree: usize) -> f64 {
    (base + evidence.min(5) as f64 * 0.07 + degree.min(8) as f64 * 0.025).clamp(0.25, 0.95)
}

fn app_name(c: &SemanticChunk) -> String {
    c.app_name
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("Dictation")
        .to_string()
}

fn evidence_items(chunks: &[&SemanticChunk], focus: &str, limit: usize) -> Vec<InsightEvidence> {
    let mut sorted: Vec<&&SemanticChunk> = chunks.iter().collect();
    sorted.sort_by(|a, b| {
        b.source_created_at
            .cmp(&a.source_created_at)
            .then(a.id.cmp(&b.id))
    });
    sorted
        .into_iter()
        .take(limit.max(1))
        .enumerate()
        .map(|(i, c)| InsightEvidence {
            id: format!("{}-{i}", c.id),
            chunk_id: c.id,
            title: if focus.is_empty() {
                memory_graph::chunk_title(&c.text)
            } else {
                focus.to_string()
            },
            excerpt: excerpt(&c.text, 180),
            source_app_name: c.app_name.clone(),
            source_created_at: Some(c.source_created_at),
            score: (1.0 - i as f64 * 0.12).max(0.25),
        })
        .collect()
}

pub fn contains_open_loop_cue(text: &str) -> bool {
    let n = format!(" {} ", text.to_lowercase().replace('’', "'"));
    [
        " need to ",
        " i need ",
        " we need ",
        " should ",
        " have to ",
        " must ",
        " follow up",
        " following up",
        " reminder",
        " remind ",
        " todo",
        " to-do",
        " action item",
        " next step",
        " make sure",
        " don't forget",
        " unresolved",
        " still need",
        " need this",
        " need that",
    ]
    .iter()
    .any(|c| n.contains(c))
}

pub fn contains_friction_cue(text: &str) -> bool {
    let n = format!(" {} ", text.to_lowercase().replace('’', "'"));
    [
        " stuck ",
        " blocked ",
        " blocker ",
        " frustrating ",
        " frustrated ",
        " annoying ",
        " broken ",
        " bug ",
        " issue ",
        " problem ",
        " confusing ",
        " can't ",
        " cannot ",
        " failed ",
        " failure ",
        " error ",
        " not working ",
        " unresolved ",
        " hard to ",
        " difficult to ",
    ]
    .iter()
    .any(|c| n.contains(c))
}

fn term_counts(chunks: &[&SemanticChunk]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for c in chunks {
        for t in vector::lexical_tokens(&c.text) {
            if t.chars().count() >= 4 {
                *counts.entry(t).or_default() += 1;
            }
        }
    }
    counts
}

fn top_counts(counts: HashMap<String, usize>, limit: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(limit.max(1));
    v
}

fn app_counts(chunks: &[&SemanticChunk]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for c in chunks {
        *m.entry(app_name(c)).or_default() += 1;
    }
    m
}

/// Recent (last 14 days) vs older chunks; halves when one side is empty.
fn comparison_windows<'a>(
    chunks: &[&'a SemanticChunk],
    now: i64,
) -> (Vec<&'a SemanticChunk>, Vec<&'a SemanticChunk>) {
    let mut sorted: Vec<&SemanticChunk> = chunks.to_vec();
    sorted.sort_by(|a, b| {
        b.source_created_at
            .cmp(&a.source_created_at)
            .then(a.id.cmp(&b.id))
    });
    if sorted.len() < 2 {
        return (sorted, Vec::new());
    }
    let cutoff = now - 14 * 86_400;
    let recent: Vec<&SemanticChunk> = sorted
        .iter()
        .copied()
        .filter(|c| c.source_created_at >= cutoff)
        .collect();
    let previous: Vec<&SemanticChunk> = sorted
        .iter()
        .copied()
        .filter(|c| c.source_created_at < cutoff)
        .collect();
    if !recent.is_empty() && !previous.is_empty() {
        return (recent, previous);
    }
    let split = (sorted.len() / 2).max(1);
    let (a, b) = sorted.split_at(split);
    (a.to_vec(), b.to_vec())
}

fn contexts<'a>(graph: &'a MemoryGraph, chunks: &'a [SemanticChunk]) -> Vec<NodeContext<'a>> {
    let node_by_id: HashMap<&str, &GraphNodeRecord> = graph
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n))
        .collect();
    let chunk_by_node: HashMap<String, &SemanticChunk> = chunks
        .iter()
        .map(|c| (format!("chunk:{}", c.id), c))
        .collect();
    let mut incident: HashMap<&str, Vec<&GraphEdgeRecord>> = HashMap::new();
    for e in &graph.edges {
        incident.entry(&e.source_node_id).or_default().push(e);
        incident.entry(&e.target_node_id).or_default().push(e);
    }
    graph
        .nodes
        .iter()
        .filter(|n| n.kind != "chunk")
        .map(|node| {
            let mut ctx = NodeContext {
                node,
                neighbor_ids: BTreeSet::new(),
                neighbor_kinds: BTreeSet::new(),
                chunks: Vec::new(),
                app_names: BTreeSet::new(),
                latest_seen_at: None,
                weighted_degree: 0.0,
            };
            let mut seen = HashSet::new();
            for e in incident
                .get(node.node_id.as_str())
                .cloned()
                .unwrap_or_default()
            {
                ctx.weighted_degree += e.weight;
                let other = if e.source_node_id == node.node_id {
                    &e.target_node_id
                } else {
                    &e.source_node_id
                };
                ctx.neighbor_ids.insert(other.clone());
                if let Some(o) = node_by_id.get(other.as_str()) {
                    ctx.neighbor_kinds.insert(o.kind.clone());
                }
                if let Some(c) = chunk_by_node.get(other.as_str()) {
                    if seen.insert(c.id) {
                        ctx.chunks.push(c);
                        if let Some(app) = c
                            .app_name
                            .as_deref()
                            .map(str::trim)
                            .filter(|a| !a.is_empty())
                        {
                            ctx.app_names.insert(app.to_string());
                        }
                    }
                }
            }
            ctx.chunks.sort_by(|a, b| {
                b.source_created_at
                    .cmp(&a.source_created_at)
                    .then(a.id.cmp(&b.id))
            });
            ctx.latest_seen_at = ctx
                .chunks
                .iter()
                .map(|c| c.source_created_at)
                .max()
                .or(node.last_seen_at);
            ctx
        })
        .collect()
}

fn app_summary(apps: &BTreeSet<String>) -> String {
    if apps.is_empty() {
        String::new()
    } else {
        format!(
            " across {}",
            apps.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )
    }
}

fn sorted_by<'a, F>(ctxs: &'a [NodeContext<'a>], filter: F) -> Vec<&'a NodeContext<'a>>
where
    F: Fn(&NodeContext<'a>) -> bool,
{
    let mut v: Vec<&NodeContext> = ctxs.iter().filter(|c| filter(c)).collect();
    v.sort_by(|a, b| {
        b.importance()
            .total_cmp(&a.importance())
            .then_with(|| a.node.node_id.cmp(&b.node.node_id))
    });
    v
}

/// The deterministic report (Mac `makeInsightReport`).
pub fn deterministic(
    graph: &MemoryGraph,
    chunks: &[SemanticChunk],
    clock: Clock,
    limit_cards: usize,
) -> InsightReport {
    let signature = memory_graph::signature(graph);
    let now = clock.now;
    let mut report = InsightReport {
        generated_at: now,
        graph_signature: signature,
        analyzer_name: "Heuristic Graph Signals".into(),
        used_fallback: true,
        summary: Vec::new(),
        summary_model_name: None,
        clusters: Vec::new(),
        comparisons: Vec::new(),
        coverage_notes: Vec::new(),
        charts: Vec::new(),
        cards: Vec::new(),
        source_node_count: graph.nodes.len(),
        source_edge_count: graph.edges.len(),
        source_chunk_count: chunks.len(),
    };
    if graph.nodes.is_empty() || graph.edges.is_empty() {
        report.summary = vec![
            "Build the semantic index to generate insight cards from your writing graph.".into(),
        ];
        report.coverage_notes = vec!["No semantic graph nodes or links are available yet.".into()];
        return report;
    }
    let ctxs = contexts(graph, chunks);
    let all: Vec<&SemanticChunk> = chunks.iter().collect();
    let mut cards: Vec<InsightCard> = Vec::new();
    let mut push = |card: Option<InsightCard>| {
        if let Some(c) = card {
            if !cards.iter().any(|x| x.id == c.id) {
                cards.push(c);
            }
        }
    };

    // Dominant theme.
    push(
        sorted_by(&ctxs, |c| c.is_concept() && c.evidence_count() > 0)
            .first()
            .map(|c| InsightCard {
                id: format!("dominant-theme-{}", c.node.node_id),
                kind: "Dominant Theme".into(),
                title: format!("Dominant theme: {}", c.node.title),
                body: format!(
                    "{} is one of the strongest recurring concepts in the current memory graph, connected to {}{}.",
                    c.node.title,
                    plural(c.evidence_count(), "segment"),
                    app_summary(&c.app_names)
                ),
                confidence: confidence(0.52, c.evidence_count(), c.degree()),
                action_text: "Use this as a working thread: collect decisions, tasks, and follow-up notes around it.".into(),
                related_node_ids: vec![c.node.node_id.clone()],
                evidence: evidence_items(&c.chunks, &c.node.title, 3),
            }),
    );
    // Active project.
    {
        let mut v: Vec<&NodeContext> = ctxs
            .iter()
            .filter(|c| {
                c.evidence_count() > 0
                    && (c.node.kind == "entity" || c.node.title.to_lowercase().contains("project"))
            })
            .collect();
        v.sort_by(|a, b| {
            b.app_names
                .len()
                .cmp(&a.app_names.len())
                .then(b.importance().total_cmp(&a.importance()))
                .then_with(|| a.node.node_id.cmp(&b.node.node_id))
        });
        push(v.first().map(|c| InsightCard {
            id: format!("active-project-{}", c.node.node_id),
            kind: "Active Project".into(),
            title: format!("Likely active project: {}", c.node.title),
            body: format!(
                "{} appears across {} and {}, which makes it look like a live workstream rather than a one-off mention.",
                c.node.title,
                plural(c.evidence_count(), "segment"),
                plural(c.app_names.len(), "app context")
            ),
            confidence: confidence(0.48, c.evidence_count(), c.degree() + c.app_names.len()),
            action_text: "Create or update a project note with the latest decisions, blockers, and next steps.".into(),
            related_node_ids: vec![c.node.node_id.clone()],
            evidence: evidence_items(&c.chunks, &c.node.title, 3),
        }));
    }
    // Hidden bridge.
    {
        let mut v: Vec<&NodeContext> = ctxs
            .iter()
            .filter(|c| c.degree() >= 3 && c.neighbor_kinds.len() >= 2 && c.evidence_count() > 0)
            .collect();
        v.sort_by(|a, b| {
            b.bridge()
                .total_cmp(&a.bridge())
                .then_with(|| a.node.node_id.cmp(&b.node.node_id))
        });
        push(v.first().map(|c| {
            let kinds = c.neighbor_kinds.iter().cloned().collect::<Vec<_>>().join(", ");
            let mut related = vec![c.node.node_id.clone()];
            related.extend(c.neighbor_ids.iter().take(6).cloned());
            InsightCard {
                id: format!("bridge-{}", c.node.node_id),
                kind: "Hidden Bridge".into(),
                title: format!("{} connects separate areas", c.node.title),
                body: format!(
                    "This node bridges {} ({kinds}), making it a useful place to inspect cross-project or cross-context overlap.",
                    plural(c.neighbor_kinds.len(), "graph layer")
                ),
                confidence: confidence(0.5, c.evidence_count(), c.degree()),
                action_text: "Review its connected evidence to find reusable ideas, dependencies, or duplicated effort.".into(),
                related_node_ids: related,
                evidence: evidence_items(&c.chunks, &c.node.title, 3),
            }
        }));
    }
    // Open loops.
    {
        let mut open: Vec<&SemanticChunk> = all
            .iter()
            .copied()
            .filter(|c| contains_open_loop_cue(&c.text))
            .collect();
        open.sort_by_key(|a| std::cmp::Reverse(a.source_created_at));
        open.truncate(5);
        if !open.is_empty() {
            push(Some(InsightCard {
                id: "open-loops".into(),
                kind: "Open Loops".into(),
                title: "Open loops are showing up in recent dictation".into(),
                body: format!(
                    "{} contain follow-up, need-to, or action language. These are likely commitments or unresolved work hiding inside dictation.",
                    plural(open.len(), "recent excerpt")
                ),
                confidence: confidence(0.5, open.len(), open.len()),
                action_text: "Turn these excerpts into explicit tasks before they disappear into the transcript history.".into(),
                related_node_ids: open.iter().map(|c| format!("chunk:{}", c.id)).collect(),
                evidence: evidence_items(&open, "open loop", 4),
            }));
        }
    }
    // Context shifts.
    {
        let mut groups: BTreeMap<String, Vec<&SemanticChunk>> = BTreeMap::new();
        for c in &all {
            let name = c
                .app_name
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .unwrap_or("Unknown App")
                .to_string();
            groups.entry(name).or_default().push(c);
        }
        let mut ranked: Vec<(String, Vec<&SemanticChunk>)> = groups.into_iter().collect();
        ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        if ranked.len() >= 2 {
            let top: Vec<String> = ranked.iter().take(3).map(|r| r.0.clone()).collect();
            let mut ev: Vec<&SemanticChunk> = ranked
                .iter()
                .take(3)
                .filter_map(|r| r.1.first().copied())
                .collect();
            ev.sort_by_key(|a| std::cmp::Reverse(a.source_created_at));
            push(Some(InsightCard {
                id: "context-shifts".into(),
                kind: "Context Shifts".into(),
                title: format!("Work is moving across {} app contexts", ranked.len()),
                body: format!(
                    "The strongest app contexts are {}. This can signal useful cross-tool flow, but it can also hide fragmentation if the same project is spread across multiple places.",
                    top.join(", ")
                ),
                confidence: confidence(0.46, ev.len(), ranked.len()),
                action_text: "Check whether the same project has decisions or todos scattered across these apps.".into(),
                related_node_ids: top.iter().map(|a| format!("app:{}", node_key(a))).collect(),
                evidence: evidence_items(&ev, "app context", 3),
            }));
        }
    }
    // Recent momentum.
    {
        let mut v: Vec<&NodeContext> = ctxs
            .iter()
            .filter(|c| {
                c.evidence_count() > 0 && c.latest_seen_at.is_some() && c.node.kind != "app"
            })
            .collect();
        v.sort_by(|a, b| {
            b.latest_seen_at
                .cmp(&a.latest_seen_at)
                .then(b.importance().total_cmp(&a.importance()))
                .then_with(|| a.node.node_id.cmp(&b.node.node_id))
        });
        push(v.first().map(|c| InsightCard {
            id: format!("recent-momentum-{}", c.node.node_id),
            kind: "Recent Momentum".into(),
            title: format!("Recent momentum: {}", c.node.title),
            body: format!(
                "{} is one of the freshest connected concepts in the graph. It is worth checking before older themes pull attention away from current work.",
                c.node.title
            ),
            confidence: confidence(0.44, c.evidence_count(), c.degree()),
            action_text: "Ask what changed recently and whether this should become today's priority.".into(),
            related_node_ids: vec![c.node.node_id.clone()],
            evidence: evidence_items(&c.chunks, &c.node.title, 3),
        }));
    }
    // Underdeveloped thread.
    {
        let mut v: Vec<&NodeContext> = ctxs
            .iter()
            .filter(|c| {
                c.is_concept() && c.evidence_count() > 0 && c.degree() <= 2 && c.node.weight >= 1.0
            })
            .collect();
        v.sort_by(|a, b| {
            b.node
                .weight
                .total_cmp(&a.node.weight)
                .then(b.latest_seen_at.cmp(&a.latest_seen_at))
                .then_with(|| a.node.node_id.cmp(&b.node.node_id))
        });
        push(v.first().map(|c| InsightCard {
            id: format!("underdeveloped-{}", c.node.node_id),
            kind: "Underdeveloped Area".into(),
            title: format!("{} has signal but few connections", c.node.title),
            body: format!(
                "This concept has enough weight to matter, but only {}. It may be an idea that has not yet been linked to the right project, app, or next step.",
                plural(c.degree(), "connection")
            ),
            confidence: confidence(0.42, c.evidence_count(), 3usize.saturating_sub(c.degree()).max(1)),
            action_text: "Expand this into a note or connect it to the project it belongs to.".into(),
            related_node_ids: vec![c.node.node_id.clone()],
            evidence: evidence_items(&c.chunks, &c.node.title, 3),
        }));
    }
    let (recent, previous) = comparison_windows(&all, now);
    let window_evidence = || {
        let mut ev: Vec<&SemanticChunk> = recent
            .iter()
            .take(2)
            .chain(previous.iter().take(2))
            .copied()
            .collect();
        ev.sort_by_key(|a| std::cmp::Reverse(a.source_created_at));
        ev
    };
    if !recent.is_empty() && !previous.is_empty() {
        // Temporal comparison.
        let recent_apps: Vec<String> = top_counts(app_counts(&recent), 3)
            .into_iter()
            .map(|x| x.0)
            .collect();
        let previous_apps: Vec<String> = top_counts(app_counts(&previous), 3)
            .into_iter()
            .map(|x| x.0)
            .collect();
        let changed: Vec<&String> = recent_apps
            .iter()
            .filter(|a| !previous_apps.contains(a))
            .collect();
        let body = if changed.is_empty() {
            format!(
                "Recent dictation is still concentrated in {}, which suggests continuity rather than a major context change.",
                recent_apps.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "Recent dictation has shifted toward {}, while older evidence leaned more toward {}.",
                changed.iter().take(2).map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                previous_apps.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
            )
        };
        let ev = window_evidence();
        push(Some(InsightCard {
            id: "temporal-comparison".into(),
            kind: "Temporal Comparison".into(),
            title: "Recent work is not identical to older graph evidence".into(),
            body,
            confidence: confidence(0.45, ev.len(), recent_apps.len() + previous_apps.len()),
            action_text: "Compare the recent thread against older commitments before choosing the next priority.".into(),
            related_node_ids: ev.iter().map(|c| format!("chunk:{}", c.id)).collect(),
            evidence: evidence_items(&ev, "recent vs older", 4),
        }));
        // Recurring vs fading.
        let rt = term_counts(&recent);
        let pt = term_counts(&previous);
        let mut rising: Vec<(String, i64)> = rt
            .iter()
            .map(|(t, c)| {
                (
                    t.clone(),
                    *c as i64 - pt.get(t).copied().unwrap_or(0) as i64,
                )
            })
            .filter(|x| x.1 > 0)
            .collect();
        rising.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut fading: Vec<(String, i64)> = pt
            .iter()
            .map(|(t, c)| {
                (
                    t.clone(),
                    *c as i64 - rt.get(t).copied().unwrap_or(0) as i64,
                )
            })
            .filter(|x| x.1 > 0)
            .collect();
        fading.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let rising: Vec<String> = rising.into_iter().take(3).map(|x| x.0).collect();
        let fading: Vec<String> = fading.into_iter().take(3).map(|x| x.0).collect();
        if !rising.is_empty() || !fading.is_empty() {
            let ev = window_evidence();
            push(Some(InsightCard {
                id: "recurring-fading".into(),
                kind: "Recurring vs Fading".into(),
                title: "Themes are changing over time".into(),
                body: format!(
                    "Recent language is leaning toward {}, while older evidence carried more {}. This is a useful signal for what is live now versus what may be slipping.",
                    if rising.is_empty() { "no clear rising term".into() } else { rising.join(", ") },
                    if fading.is_empty() { "no clear fading term".into() } else { fading.join(", ") }
                ),
                confidence: confidence(0.42, ev.len(), rising.len() + fading.len()),
                action_text: "Promote rising themes into active tasks and review fading themes for unfinished commitments.".into(),
                related_node_ids: rising.iter().chain(fading.iter()).map(|t| format!("topic:{}", node_key(t))).collect(),
                evidence: evidence_items(&ev, "theme movement", 4),
            }));
        }
    }
    cards.truncate(limit_cards.max(1));

    // Clusters.
    let mut clusters: Vec<InsightCluster> = Vec::new();
    {
        let mut groups: BTreeMap<String, Vec<&SemanticChunk>> = BTreeMap::new();
        for c in &all {
            groups.entry(app_name(c)).or_default().push(c);
        }
        let mut ranked: Vec<(String, Vec<&SemanticChunk>)> = groups.into_iter().collect();
        ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        for (app, group) in ranked.iter().take(3) {
            let terms: Vec<String> = top_counts(term_counts(group), 3)
                .into_iter()
                .map(|x| x.0)
                .collect();
            clusters.push(InsightCluster {
                id: format!("app-cluster-{}", node_key(app)),
                title: format!("{app} work area"),
                summary: if terms.is_empty() {
                    format!(
                        "{app} contains {}.",
                        plural(group.len(), "indexed transcript segment")
                    )
                } else {
                    format!(
                        "{app} is carrying {}, based on {}.",
                        terms.join(", "),
                        plural(group.len(), "indexed segment")
                    )
                },
                related_node_ids: vec![format!("app:{}", node_key(app))],
                evidence: evidence_items(group, app, 3),
            });
        }
        for c in sorted_by(&ctxs, |c| c.is_concept() && c.evidence_count() >= 2)
            .into_iter()
            .take(2)
        {
            clusters.push(InsightCluster {
                id: format!("theme-cluster-{}", c.node.node_id),
                title: format!("{} thread", c.node.title),
                summary: format!(
                    "{} links {} across {}, making it a candidate life/work area rather than a one-off mention.",
                    c.node.title,
                    plural(c.evidence_count(), "evidence segment"),
                    plural(c.app_names.len().max(1), "app context")
                ),
                related_node_ids: vec![c.node.node_id.clone()],
                evidence: evidence_items(&c.chunks, &c.node.title, 3),
            });
        }
        let mut seen = HashSet::new();
        clusters.retain(|c| !c.evidence.is_empty() && seen.insert(c.id.clone()));
    }

    // Comparisons.
    let mut comparisons: Vec<InsightComparison> = Vec::new();
    if !recent.is_empty() && !previous.is_empty() {
        let rt: Vec<String> = top_counts(term_counts(&recent), 3)
            .into_iter()
            .map(|x| x.0)
            .collect();
        let pt: Vec<String> = top_counts(term_counts(&previous), 3)
            .into_iter()
            .map(|x| x.0)
            .collect();
        if !rt.is_empty() || !pt.is_empty() {
            comparisons.push(InsightComparison {
                id: "comparison-theme-drift".into(),
                title: "Recent vs older theme mix".into(),
                detail: format!(
                    "Recent evidence emphasizes {}; older evidence emphasized {}.",
                    if rt.is_empty() {
                        "no dominant terms".into()
                    } else {
                        rt.join(", ")
                    },
                    if pt.is_empty() {
                        "no dominant terms".into()
                    } else {
                        pt.join(", ")
                    }
                ),
                trend: "mixed".into(),
                evidence: evidence_items(&window_evidence(), "theme drift", 4),
            });
        }
        let ra: BTreeSet<String> = recent.iter().map(|c| app_name(c)).collect();
        let pa: BTreeSet<String> = previous.iter().map(|c| app_name(c)).collect();
        let new_apps: Vec<&String> = ra.difference(&pa).collect();
        if !new_apps.is_empty() {
            comparisons.push(InsightComparison {
                id: "comparison-app-shift".into(),
                title: "App-context shift".into(),
                detail: format!(
                    "Recent dictation added {} compared with the older window, which can indicate a newer work surface or scattered follow-up context.",
                    new_apps.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ),
                trend: "rising".into(),
                evidence: evidence_items(&recent, "app shift", 3),
            });
        }
    }

    // Charts.
    let mut charts = Vec::new();
    {
        let mut v: Vec<&NodeContext> = ctxs
            .iter()
            .filter(|c| c.is_concept() && c.evidence_count() > 0)
            .collect();
        v.sort_by(|a, b| {
            b.evidence_count()
                .cmp(&a.evidence_count())
                .then(b.importance().total_cmp(&a.importance()))
                .then_with(|| a.node.node_id.cmp(&b.node.node_id))
        });
        let points: Vec<InsightChartPoint> = v
            .iter()
            .take(6)
            .map(|c| {
                let apps = c
                    .app_names
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                InsightChartPoint {
                    id: format!("life-area-{}", c.node.node_id),
                    label: c.node.title.clone(),
                    value: c.evidence_count() as f64,
                    detail: Some(if apps.is_empty() {
                        format!(
                            "{} connect to this area.",
                            plural(c.evidence_count(), "transcript segment")
                        )
                    } else {
                        format!("{} across {apps}.", plural(c.evidence_count(), "segment"))
                    }),
                    evidence: evidence_items(&c.chunks, &c.node.title, 2),
                }
            })
            .filter(|p| !p.evidence.is_empty())
            .collect();
        if points.len() >= 2 {
            charts.push(InsightChart {
                id: "chart-life-area-attention".into(),
                title: "Attention by Life/Work Area".into(),
                subtitle: "Areas are inferred from recurring topics and named contexts connected to transcript evidence.".into(),
                kind: "bar".into(),
                unit: "segments".into(),
                points,
            });
        }
        let mut groups: BTreeMap<String, Vec<&SemanticChunk>> = BTreeMap::new();
        for c in &all {
            groups.entry(app_name(c)).or_default().push(c);
        }
        let mut ranked: Vec<(String, Vec<&SemanticChunk>)> = groups.into_iter().collect();
        ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        let points: Vec<InsightChartPoint> = ranked
            .iter()
            .take(6)
            .map(|(app, group)| {
                let terms: Vec<String> = top_counts(term_counts(group), 3)
                    .into_iter()
                    .map(|x| x.0)
                    .collect();
                InsightChartPoint {
                    id: format!("app-context-{}", node_key(app)),
                    label: app.clone(),
                    value: group.len() as f64,
                    detail: Some(if terms.is_empty() {
                        format!(
                            "{} in this app context.",
                            plural(group.len(), "transcript segment")
                        )
                    } else {
                        format!(
                            "Carrying {} across {}.",
                            terms.join(", "),
                            plural(group.len(), "segment")
                        )
                    }),
                    evidence: evidence_items(group, app, 2),
                }
            })
            .collect();
        if points.len() >= 2 {
            charts.push(InsightChart {
                id: "chart-app-context".into(),
                title: "Where Your Work Happens".into(),
                subtitle: "This shows where graph evidence is being created, which can reveal cross-tool focus or fragmentation.".into(),
                kind: "bar".into(),
                unit: "segments".into(),
                points,
            });
        }
        let groups: [(&str, Vec<&SemanticChunk>, &str); 4] = [
            ("Recent open loops", recent.iter().copied().filter(|c| contains_open_loop_cue(&c.text)).collect(), "Recent evidence uses need-to, should, follow-up, or unresolved commitment language."),
            ("Older open loops", previous.iter().copied().filter(|c| contains_open_loop_cue(&c.text)).collect(), "Older evidence also carried commitment or follow-up language."),
            ("Recent friction cues", recent.iter().copied().filter(|c| contains_friction_cue(&c.text)).collect(), "Recent evidence includes explicit stuck, blocked, bug, issue, failed, confusing, or not-working language."),
            ("Older friction cues", previous.iter().copied().filter(|c| contains_friction_cue(&c.text)).collect(), "Older evidence included explicit blocker or friction language."),
        ];
        let points: Vec<InsightChartPoint> = groups
            .iter()
            .filter(|g| !g.1.is_empty())
            .map(|(label, group, detail)| InsightChartPoint {
                id: format!("signal-{}", node_key(label)),
                label: (*label).into(),
                value: group.len() as f64,
                detail: Some((*detail).into()),
                evidence: evidence_items(group, label, 2),
            })
            .collect();
        if points.len() >= 2 {
            charts.push(InsightChart {
                id: "chart-open-loop-friction".into(),
                title: "Open Loops and Friction Cues".into(),
                subtitle: "Counts come from explicit language in the transcript evidence, not inferred mood or diagnosis.".into(),
                kind: "comparison".into(),
                unit: "signals".into(),
                points,
            });
        }
    }

    // Coverage notes.
    let mut notes = Vec::new();
    if chunks.len() < 20 {
        notes.push(format!(
            "Evidence is still thin: only {} were available.",
            plural(chunks.len(), "embedded transcript chunk")
        ));
    }
    let app_count = chunks
        .iter()
        .filter_map(|c| {
            c.app_name
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
        })
        .collect::<HashSet<_>>()
        .len();
    if app_count < 2 {
        notes.push(format!(
            "Cross-app comparison is limited because evidence currently spans {}.",
            plural(app_count, "app context")
        ));
    }
    if ctxs.iter().filter(|c| c.evidence_count() >= 2).count() < 2 {
        notes.push(
            "Life/work area clustering will improve after more repeated themes are indexed.".into(),
        );
    }
    if graph.edges.len() < graph.nodes.len() / 2 {
        notes.push("The graph has relatively few links, so bridge insights should be treated as directional rather than definitive.".into());
    }

    // Summary lines.
    let mut summary = Vec::new();
    match sorted_by(&ctxs, |c| c.is_concept() && c.evidence_count() > 0).first() {
        Some(top) => summary.push(format!(
            "{} is the current center of gravity in your dictated work, with enough repeated evidence to treat it as an active thread.",
            top.node.title
        )),
        None => summary.push("The graph has enough structure to inspect, but it needs more repeated dictation before a clear workstream emerges.".into()),
    }
    if let Some(c) = cards.iter().find(|c| c.kind == "Open Loops") {
        summary.push(format!(
            "{}: capture these as explicit tasks so commitments do not stay buried inside transcripts.",
            c.title
        ));
    } else if let Some(c) = cards.iter().find(|c| c.kind == "Context Shifts") {
        summary.push(format!(
            "{}, so check whether decisions and todos are scattered across tools.",
            c.title
        ));
    }
    if let Some(c) = cards.iter().find(|c| c.kind == "Hidden Bridge") {
        summary.push(format!(
            "{}; this is the best place to look for cross-project overlap or reusable ideas.",
            c.title
        ));
    } else if let Some(c) = cards.iter().find(|c| c.kind == "Recent Momentum") {
        summary.push(format!(
            "{} is fresh enough to review before older themes pull attention away.",
            c.title
        ));
    }
    match cards.first() {
        Some(c) => summary.push(format!("Best next action: {}", c.action_text)),
        None => summary.push("Best next action: build more semantic memory, then regenerate insights for stronger evidence.".into()),
    }
    summary.truncate(4);

    report.summary = summary;
    report.clusters = clusters;
    report.comparisons = comparisons;
    report.coverage_notes = notes;
    report.charts = charts;
    report.cards = cards;
    report
}

/// Fold the top findings into the report (Mac `reportByMergingFindings`).
pub fn merge_findings(
    mut report: InsightReport,
    findings: &[InsightFinding],
    chunks: &[SemanticChunk],
) -> InsightReport {
    if findings.is_empty() {
        return report;
    }
    let by_id: HashMap<i64, &SemanticChunk> = chunks.iter().map(|c| (c.id, c)).collect();
    let mut ranked: Vec<&InsightFinding> = findings.iter().collect();
    ranked.sort_by(|a, b| {
        (b.magnitude * b.confidence)
            .total_cmp(&(a.magnitude * a.confidence))
            .then(a.id.cmp(&b.id))
    });
    let mut per_kind: HashMap<&str, usize> = HashMap::new();
    let mut cards = Vec::new();
    for f in ranked {
        let n = per_kind.entry(f.kind.as_str()).or_default();
        if *n >= 2 {
            continue;
        }
        *n += 1;
        let evidence = f
            .evidence_chunk_ids
            .iter()
            .take(3)
            .filter_map(|id| by_id.get(id))
            .map(|c| InsightEvidence {
                id: format!("finding-{}-{}", f.kind, c.id),
                chunk_id: c.id,
                title: c.app_name.clone().unwrap_or_else(|| "Dictation".into()),
                excerpt: c.text.chars().take(220).collect(),
                source_app_name: c.app_name.clone(),
                source_created_at: Some(c.source_created_at),
                score: f.confidence,
            })
            .collect();
        cards.push(InsightCard {
            id: format!("finding-{}-{}", f.kind, f.subject_key),
            kind: kind_label(&f.kind).into(),
            title: f.title.clone(),
            body: f.detail.clone(),
            confidence: f.confidence,
            action_text: kind_action(&f.kind).into(),
            related_node_ids: Vec::new(),
            evidence,
        });
        if cards.len() >= 6 {
            break;
        }
    }
    if let Some(h) = findings
        .iter()
        .find(|f| f.kind == super::findings::ANOMALY)
        .or_else(|| findings.iter().find(|f| f.kind == super::findings::RHYTHM))
    {
        report
            .summary
            .insert(0, format!("{}: {}", h.title, h.detail));
    }
    cards.extend(report.cards);
    report.cards = cards;
    report
}

pub fn with_fallback_reason(mut report: InsightReport, reason: &str) -> InsightReport {
    let mut notes: Vec<String> = vec![reason.to_string()];
    notes.extend(
        report
            .coverage_notes
            .into_iter()
            .filter(|n| !n.starts_with(FALLBACK_PREFIX)),
    );
    notes.truncate(5);
    report.coverage_notes = notes;
    report
}

// ---- Local model overlay ---------------------------------------------------------

fn clean_line(value: Option<&str>, max: usize) -> Option<String> {
    let mut s = value?.to_string();
    while let (Some(a), Some(b)) = (s.find("<think>"), s.find("</think>")) {
        if b < a {
            break;
        }
        s.replace_range(a..b + "</think>".len(), "");
    }
    let mut s = s.replace("**", "").replace('`', "");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = s.trim_start_matches(['-', '*', '•', ' ']).to_string();
    let t = match t.split_once(['.', ')']) {
        Some((num, rest)) if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) => {
            rest.trim().to_string()
        }
        _ => t,
    };
    let t = t.trim().to_string();
    if t.is_empty() {
        return None;
    }
    if t.chars().count() > max {
        let head: String = t.chars().take(max.saturating_sub(3)).collect();
        return Some(format!("{}...", head.trim_end()));
    }
    Some(t)
}

fn str_field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()))
}

fn f64_field(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|x| {
            x.as_f64().or_else(|| {
                x.as_str()
                    .and_then(|s| s.trim().trim_end_matches('%').parse().ok())
            })
        })
    })
}

fn ids_field(v: &serde_json::Value) -> Vec<i64> {
    let raw = [
        "evidenceChunkIDs",
        "evidenceChunkIds",
        "evidence_chunk_ids",
        "evidence",
    ]
    .iter()
    .find_map(|k| v.get(*k));
    let mut out: Vec<i64> = Vec::new();
    fn push_str(out: &mut Vec<i64>, s: &str) {
        for part in s.split(|c: char| !c.is_ascii_digit()) {
            if let Ok(n) = part.parse() {
                out.push(n);
            }
        }
    }
    match raw {
        Some(serde_json::Value::Array(items)) => {
            for i in items {
                if let Some(n) = i.as_i64() {
                    out.push(n);
                } else if let Some(s) = i.as_str() {
                    push_str(&mut out, s);
                }
            }
        }
        Some(serde_json::Value::String(s)) => push_str(&mut out, s),
        Some(serde_json::Value::Number(n)) => out.extend(n.as_i64()),
        _ => {}
    }
    let mut seen = HashSet::new();
    out.retain(|x| seen.insert(*x));
    out
}

fn arr<'a>(v: &'a serde_json::Value, key: &str) -> Vec<&'a serde_json::Value> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Balanced top-level JSON objects in a reply (fences and prose around them ignored).
pub fn extract_json_objects(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_str = false;
    let mut escape = false;
    for (i, c) in text.char_indices() {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' if depth > 0 => in_str = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        out.push(text[s..=i].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn decode_payload(reply: &str) -> Option<serde_json::Value> {
    extract_json_objects(reply).into_iter().find_map(|o| {
        let v: serde_json::Value = serde_json::from_str(&o).ok()?;
        let non_empty = ["summary", "cards", "clusters", "comparisons", "charts"]
            .iter()
            .any(|k| {
                v.get(*k)
                    .and_then(|x| x.as_array())
                    .is_some_and(|a| !a.is_empty())
            });
        non_empty.then_some(v)
    })
}

fn model_evidence(
    ids: &[i64],
    by_id: &HashMap<i64, &SemanticChunk>,
    focus: &str,
    limit: usize,
) -> Vec<InsightEvidence> {
    let chunks: Vec<&SemanticChunk> = ids.iter().filter_map(|i| by_id.get(i).copied()).collect();
    if chunks.is_empty() {
        return Vec::new();
    }
    evidence_items(&chunks, focus, limit)
}

/// Replace report sections with the model's phrasing where it cites real chunks.
pub fn apply_model_payload(
    report: InsightReport,
    payload: &serde_json::Value,
    chunks: &[SemanticChunk],
    model: &str,
    limit_cards: usize,
) -> InsightReport {
    let by_id: HashMap<i64, &SemanticChunk> = chunks.iter().map(|c| (c.id, c)).collect();
    let lines = |key: &str, fallback: &[String], limit: usize| -> Vec<String> {
        let cleaned: Vec<String> = arr(payload, key)
            .into_iter()
            .filter_map(|v| clean_line(v.as_str(), 240))
            .collect();
        if cleaned.is_empty() {
            fallback.to_vec()
        } else {
            cleaned.into_iter().take(limit.max(1)).collect()
        }
    };
    let summary = lines("summary", &report.summary, 4);
    let coverage = lines("coverageNotes", &report.coverage_notes, 4);
    let cards: Vec<InsightCard> = arr(payload, "cards")
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let title = clean_line(str_field(c, &["title"]), 96)?;
            let body = clean_line(str_field(c, &["body"]), 420)?;
            let action = clean_line(str_field(c, &["actionText", "action_text", "action"]), 220)?;
            let ids = ids_field(c);
            let evidence = model_evidence(&ids, &by_id, &title, 4);
            if evidence.is_empty() {
                return None;
            }
            Some(InsightCard {
                id: format!("model-card-{i}-{}", node_key(&title)),
                kind: clean_line(str_field(c, &["kind"]), 40)
                    .unwrap_or_else(|| "Deep Insight".into()),
                confidence: f64_field(c, &["confidence"])
                    .unwrap_or(0.64)
                    .clamp(0.25, 0.95),
                title,
                body,
                action_text: action,
                related_node_ids: ids.iter().map(|i| format!("chunk:{i}")).collect(),
                evidence,
            })
        })
        .take(limit_cards.max(1))
        .collect();
    let clusters: Vec<InsightCluster> = arr(payload, "clusters")
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let title = clean_line(str_field(c, &["title"]), 96)?;
            let summary = clean_line(str_field(c, &["summary"]), 360)?;
            let ids = ids_field(c);
            let evidence = model_evidence(&ids, &by_id, &title, 3);
            if evidence.is_empty() {
                return None;
            }
            Some(InsightCluster {
                id: format!("model-cluster-{i}-{}", node_key(&title)),
                title,
                summary,
                related_node_ids: ids.iter().map(|i| format!("chunk:{i}")).collect(),
                evidence,
            })
        })
        .collect();
    let comparisons: Vec<InsightComparison> = arr(payload, "comparisons")
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let title = clean_line(str_field(c, &["title"]), 96)?;
            let detail = clean_line(str_field(c, &["detail"]), 360)?;
            let ids = ids_field(c);
            let evidence = model_evidence(&ids, &by_id, &title, 3);
            if evidence.is_empty() {
                return None;
            }
            Some(InsightComparison {
                id: format!("model-comparison-{i}-{}", node_key(&title)),
                trend: clean_line(str_field(c, &["trend"]), 24).unwrap_or_else(|| "mixed".into()),
                title,
                detail,
                evidence,
            })
        })
        .collect();
    let charts: Vec<InsightChart> = arr(payload, "charts")
        .into_iter()
        .enumerate()
        .filter_map(|(ci, c)| {
            let title = clean_line(str_field(c, &["title"]), 96)?;
            let subtitle = clean_line(str_field(c, &["subtitle"]), 220)?;
            let points: Vec<InsightChartPoint> = arr(c, "points")
                .into_iter()
                .enumerate()
                .filter_map(|(pi, p)| {
                    let label = clean_line(str_field(p, &["label"]), 64)?;
                    let value = f64_field(p, &["value"]).filter(|v| v.is_finite())?;
                    let ids = ids_field(p);
                    let evidence = model_evidence(&ids, &by_id, &label, 2);
                    if evidence.is_empty() {
                        return None;
                    }
                    Some(InsightChartPoint {
                        id: format!("model-chart-{ci}-point-{pi}-{}", node_key(&label)),
                        label,
                        value: value.max(0.0),
                        detail: clean_line(str_field(p, &["detail"]), 160),
                        evidence,
                    })
                })
                .take(8)
                .collect();
            if points.len() < 2 {
                return None;
            }
            Some(InsightChart {
                id: format!("model-chart-{ci}-{}", node_key(&title)),
                kind: clean_line(str_field(c, &["kind"]), 24).unwrap_or_else(|| "bar".into()),
                unit: clean_line(str_field(c, &["unit"]), 32).unwrap_or_else(|| "segments".into()),
                title,
                subtitle,
                points,
            })
        })
        .take(3)
        .collect();
    InsightReport {
        analyzer_name: "Local Graph Analyst".into(),
        used_fallback: false,
        summary,
        summary_model_name: Some(model.to_string()),
        cards: if cards.is_empty() {
            report.cards.clone()
        } else {
            cards
        },
        clusters: if clusters.is_empty() {
            report.clusters.clone()
        } else {
            clusters
        },
        comparisons: if comparisons.is_empty() {
            report.comparisons.clone()
        } else {
            comparisons
        },
        charts: if charts.is_empty() {
            report.charts.clone()
        } else {
            charts
        },
        coverage_notes: coverage,
        ..report
    }
}

pub fn model_schema() -> serde_json::Value {
    let ids = serde_json::json!({"type": "array", "items": {"type": "integer"}});
    serde_json::json!({
        "type": "object",
        "properties": {
            "summary": {"type": "array", "items": {"type": "string"}},
            "cards": {"type": "array", "items": {"type": "object", "properties": {
                "kind": {"type": "string"}, "title": {"type": "string"}, "body": {"type": "string"},
                "actionText": {"type": "string"}, "confidence": {"type": "number"}, "evidenceChunkIDs": ids
            }, "required": ["kind", "title", "body", "actionText", "confidence", "evidenceChunkIDs"]}},
            "clusters": {"type": "array", "items": {"type": "object", "properties": {
                "title": {"type": "string"}, "summary": {"type": "string"}, "evidenceChunkIDs": ids
            }, "required": ["title", "summary", "evidenceChunkIDs"]}},
            "comparisons": {"type": "array", "items": {"type": "object", "properties": {
                "title": {"type": "string"}, "detail": {"type": "string"}, "trend": {"type": "string"}, "evidenceChunkIDs": ids
            }, "required": ["title", "detail", "trend", "evidenceChunkIDs"]}},
            "charts": {"type": "array", "items": {"type": "object", "properties": {
                "title": {"type": "string"}, "subtitle": {"type": "string"}, "kind": {"type": "string"},
                "unit": {"type": "string"}, "points": {"type": "array", "items": {"type": "object", "properties": {
                    "label": {"type": "string"}, "value": {"type": "number"}, "detail": {"type": "string"}, "evidenceChunkIDs": ids
                }, "required": ["label", "value", "evidenceChunkIDs"]}}
            }, "required": ["title", "subtitle", "kind", "unit", "points"]}},
            "coverageNotes": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["summary", "cards", "clusters", "comparisons", "charts", "coverageNotes"]
    })
}

pub fn evidence_pack(
    report: &InsightReport,
    graph: &MemoryGraph,
    chunks: &[SemanticChunk],
    clock: Clock,
) -> String {
    let by_id: HashMap<&str, &GraphNodeRecord> = graph
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n))
        .collect();
    let top_nodes = graph
        .nodes
        .iter()
        .filter(|n| n.kind != "chunk")
        .take(36)
        .map(|n| format!("- {}: {} weight {:.2}", n.kind, n.title, n.weight))
        .collect::<Vec<_>>()
        .join("\n");
    let mut edges: Vec<&GraphEdgeRecord> = graph.edges.iter().collect();
    edges.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let top_edges = edges
        .iter()
        .take(28)
        .map(|e| {
            let s = by_id
                .get(e.source_node_id.as_str())
                .map(|n| n.title.as_str())
                .unwrap_or(&e.source_node_id);
            let t = by_id
                .get(e.target_node_id.as_str())
                .map(|n| n.title.as_str())
                .unwrap_or(&e.target_node_id);
            let ev = e
                .evidence
                .as_deref()
                .filter(|x| !x.is_empty())
                .map(|x| format!(" evidence: {x}"))
                .unwrap_or_default();
            format!("- {s} -- {} {:.2} -- {t}{ev}", e.kind, e.weight)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let all: Vec<&SemanticChunk> = chunks.iter().collect();
    let apps = top_counts(app_counts(&all), 8)
        .into_iter()
        .map(|(a, n)| format!("- {a}: {}", plural(n, "segment")))
        .collect::<Vec<_>>()
        .join("\n");
    let (recent, previous) = comparison_windows(&all, report.generated_at);
    let count = |v: &[&SemanticChunk], f: fn(&str) -> bool| v.iter().filter(|c| f(&c.text)).count();
    let signals = format!(
        "- Recent window: {}, {}, {}\n- Older window: {}, {}, {}",
        plural(recent.len(), "segment"),
        plural(count(&recent, contains_open_loop_cue), "open-loop cue"),
        plural(count(&recent, contains_friction_cue), "friction cue"),
        plural(previous.len(), "segment"),
        plural(count(&previous, contains_open_loop_cue), "open-loop cue"),
        plural(count(&previous, contains_friction_cue), "friction cue"),
    );
    let chunk_lines = chunks
        .iter()
        .take(80)
        .map(|c| {
            format!(
                "[chunkID={}] {}, {}\n{}",
                c.id,
                app_name(c),
                clock.medium(c.source_created_at),
                excerpt(&c.text, 360)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let cards = report
        .cards
        .iter()
        .take(8)
        .map(|c| format!("- {}: {}. {}", c.kind, c.title, c.body))
        .collect::<Vec<_>>()
        .join("\n");
    let charts = report
        .charts
        .iter()
        .take(3)
        .map(|c| {
            let pts = c
                .points
                .iter()
                .take(8)
                .map(|p| format!("{}={:.1}", p.label, p.value))
                .collect::<Vec<_>>()
                .join("; ");
            format!("- {} ({}): {pts}", c.title, c.unit)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let or_none = |s: String| {
        if s.is_empty() {
            "- None".to_string()
        } else {
            s
        }
    };
    format!(
        "Graph counts: {} nodes, {} edges, {} embedded transcript chunks.\n\nTop graph nodes:\n{}\n\nTop graph links:\n{}\n\nApp distribution:\n{}\n\nTemporal/open-loop/friction counts:\n{signals}\n\nDeterministic pre-analysis:\n{}\n\nDeterministic chart candidates:\n{}\n\nTranscript chunks:\n{}",
        graph.nodes.len(),
        graph.edges.len(),
        chunks.len(),
        or_none(top_nodes),
        or_none(top_edges),
        or_none(apps),
        or_none(cards),
        or_none(charts),
        or_none(chunk_lines)
    )
}

pub fn model_prompt(
    report: &InsightReport,
    graph: &MemoryGraph,
    chunks: &[SemanticChunk],
    clock: Clock,
) -> String {
    format!(
        "You are a private local life/work graph analyst inside a desktop dictation app. Analyze only the provided transcript evidence, graph structure, app context, and timestamps.\n\n\
Your job is to go deeper than restating nodes and edges. Infer evidence-bounded patterns about:\n\
- what the user appears to be actively working on;\n\
- which life/work areas are recurring, rising, fading, or fragmented across apps;\n\
- explicit frustration, blockage, ambiguity, or open-loop signals when the transcript language supports them;\n\
- bridges between areas that may reveal duplicated effort, dependencies, or unresolved commitments;\n\
- chartable distributions that help the user see where attention, friction, and recurring themes are concentrated.\n\n\
Rules:\n\
- Return one JSON object only. No markdown and no prose wrapper.\n\
- Do not diagnose the user, infer protected traits, guess demographics, or make unsupported personality claims.\n\
- Treat \"frustration\" as a work-signal only when transcript wording supports it, such as stuck, blocked, confusing, annoying, issue, bug, failed, need to, should, follow up, unresolved, or similar language.\n\
- Every card, cluster, comparison, and chart point must cite evidenceChunkIDs from the provided chunks.\n\
- Use graph edges as weak evidence for relationships, not as the insight itself. Do not write \"A is connected to B\" unless you explain the deeper inferred implication for the user's work, friction, commitments, or attention.\n\
- Synthesize across at least two evidence types when possible: graph links, transcript wording, app context, time recency, repeated terms, and deterministic pre-analysis.\n\
- The deterministic pre-analysis entries (life areas, rhythms, open commitments/questions, emerging/fading themes, anomalies) are computed facts: keep their substance, rephrase them in a sharper voice if you like, build deeper synthesis on top of them, and never contradict them.\n\
- Prefer specific, comparative insights over generic productivity advice: recent vs older work, cross-app fragmentation, recurring vs fading themes, open loops, blockers, and bridges between life/work areas.\n\
- Charts should expose distributions or comparisons, not decoration.\n\
- If evidence is thin, say so in coverageNotes instead of inventing certainty.\n\n\
Required JSON shape: {{\"summary\": [\"3-4 concise, evidence-bounded bullets\"], \"cards\": [{{\"kind\": \"Active Work|Friction Signal|Open Loop|Bridge|Attention Shift|Deep Insight\", \"title\": \"string\", \"body\": \"specific evidence-bounded finding\", \"actionText\": \"concrete next action\", \"confidence\": 0.25, \"evidenceChunkIDs\": [123]}}], \"clusters\": [{{\"title\": \"life/work area name\", \"summary\": \"what this area contains and why it matters\", \"evidenceChunkIDs\": [123]}}], \"comparisons\": [{{\"title\": \"recent vs older comparison\", \"detail\": \"what increased, faded, or crossed app boundaries\", \"trend\": \"rising|fading|stable|mixed\", \"evidenceChunkIDs\": [123]}}], \"charts\": [{{\"title\": \"chart title\", \"subtitle\": \"what the chart reveals\", \"kind\": \"bar|comparison\", \"unit\": \"segments|mentions|signals|score\", \"points\": [{{\"label\": \"label\", \"value\": 3, \"detail\": \"why it matters\", \"evidenceChunkIDs\": [123]}}]}}], \"coverageNotes\": [\"limitations in the evidence\"]}}\n\n\
Evidence pack:\n{}",
        evidence_pack(report, graph, chunks, clock)
    )
}

/// Model-overlay settings for [`overlay`].
#[derive(Debug, Clone)]
pub struct OverlayConfig {
    pub enabled: bool,
    pub model: String,
    pub context_tokens: u32,
    pub thinking: bool,
}

fn canonical_model(name: &str) -> String {
    let t = name.trim().to_lowercase();
    if t.is_empty() || t.contains(':') {
        t
    } else {
        format!("{t}:latest")
    }
}

/// Phrase the report with a local model when enabled and installed; on any
/// failure the deterministic report is returned with the reason noted.
pub fn overlay(
    report: InsightReport,
    graph: &MemoryGraph,
    chunks: &[SemanticChunk],
    client: &dyn LlmClient,
    cfg: &OverlayConfig,
    clock: Clock,
    limit_cards: usize,
) -> InsightReport {
    if report.cards.is_empty() || !cfg.enabled {
        return report;
    }
    let model = cfg.model.trim();
    if model.is_empty() {
        return with_fallback_reason(
            report,
            &format!("{FALLBACK_PREFIX} no graph insight model is selected."),
        );
    }
    let provider = client.provider_name().to_string();
    match client.list_models() {
        Ok(models) => {
            let want = canonical_model(model);
            if !models.iter().any(|m| canonical_model(&m.name) == want) {
                return with_fallback_reason(
                    report,
                    &format!("{FALLBACK_PREFIX} {model} is not installed in {provider}."),
                );
            }
        }
        Err(e) => return with_fallback_reason(report, &format!("{FALLBACK_PREFIX} {e}")),
    }
    let opts = ChatOptions {
        temperature: Some(0.2),
        think: Some(cfg.thinking),
        json_schema: Some(model_schema()),
        max_tokens: Some(4800),
        num_ctx: Some(cfg.context_tokens),
        timeout_ms: Some(120_000),
        ..ChatOptions::model(model)
    };
    let prompt = model_prompt(&report, graph, chunks, clock);
    match client.chat(&[ChatMessage::user(prompt)], &opts, None) {
        Ok(reply) => match decode_payload(&reply.content) {
            Some(payload) => apply_model_payload(report, &payload, chunks, model, limit_cards),
            None => with_fallback_reason(
                report,
                &format!("{FALLBACK_PREFIX} {model} did not return usable structured JSON."),
            ),
        },
        Err(e) => with_fallback_reason(report, &format!("{FALLBACK_PREFIX} {e}")),
    }
}

/// Freshness of a stored report against the current graph.
pub fn is_stale(report: &InsightReport, graph: &MemoryGraph) -> bool {
    report.graph_signature != memory_graph::signature(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;
    use crate::store::EmbeddedChunk;

    fn chunk(id: i64, text: &str, app: &str, ts: i64) -> SemanticChunk {
        SemanticChunk {
            id,
            transcription_id: id,
            chunk_index: 0,
            text: text.into(),
            text_hash: String::new(),
            source_created_at: ts,
            app_name: Some(app.into()),
            word_count: text.split_whitespace().count() as i64,
        }
    }

    fn fixture(now: i64) -> (MemoryGraph, Vec<SemanticChunk>) {
        let texts = [
            (
                "I need to finish the memory graph layout for Orttaai.",
                "Code",
                now - 86_400,
            ),
            (
                "The memory graph layout is broken on Windows and I'm stuck.",
                "Code",
                now - 2 * 86_400,
            ),
            (
                "Email Sarah Chen about the memory graph demo.",
                "Mail",
                now - 3 * 86_400,
            ),
            (
                "Budget review notes for the quarterly plan.",
                "Notes",
                now - 30 * 86_400,
            ),
            (
                "Budget review again with Sarah Chen next week.",
                "Mail",
                now - 31 * 86_400,
            ),
        ];
        let chunks: Vec<SemanticChunk> = texts
            .iter()
            .enumerate()
            .map(|(i, (t, a, ts))| chunk(i as i64 + 1, t, a, *ts))
            .collect();
        let embedded: Vec<EmbeddedChunk> = chunks
            .iter()
            .map(|c| EmbeddedChunk {
                chunk: c.clone(),
                model_id: "m".into(),
                dimension: 256,
                vector: vector::encode(&vector::lexical_embedding(&c.text)),
            })
            .collect();
        (memory_graph::build(&embedded, Clock::utc(now)), chunks)
    }

    #[test]
    fn deterministic_report_has_cards_charts_and_notes() {
        let now = 100 * 86_400;
        let (graph, chunks) = fixture(now);
        let r = deterministic(&graph, &chunks, Clock::utc(now), 8);
        assert!(r.used_fallback);
        assert!(r.cards.iter().any(|c| c.kind == "Dominant Theme"));
        assert!(r.cards.iter().any(|c| c.kind == "Open Loops"));
        assert!(r.cards.iter().any(|c| c.kind == "Context Shifts"));
        assert!(r
            .cards
            .iter()
            .all(|c| c.evidence.iter().all(|e| e.chunk_id > 0)));
        assert!(r.summary.len() <= 4 && !r.summary.is_empty());
        assert!(r.summary.last().unwrap().starts_with("Best next action:"));
        assert!(r
            .coverage_notes
            .iter()
            .any(|n| n.starts_with("Evidence is still thin")));
        assert!(r.charts.iter().any(|c| c.id == "chart-app-context"));
        assert!(!r.clusters.is_empty());
        assert!(!r.comparisons.is_empty());
        assert!(!is_stale(&r, &graph));
        let empty = deterministic(&MemoryGraph::default(), &[], Clock::utc(now), 8);
        assert!(empty.cards.is_empty());
    }

    #[test]
    fn overlay_uses_model_cards_with_real_evidence_only() {
        let now = 100 * 86_400;
        let (graph, chunks) = fixture(now);
        let base = deterministic(&graph, &chunks, Clock::utc(now), 8);
        let reply = r#"```json
{"summary":["- **Memory graph** is your main thread."],
 "cards":[{"kind":"Friction Signal","title":"Layout keeps breaking","body":"The layout broke twice.","actionText":"Pin the Windows bug.","confidence":2,"evidenceChunkIDs":[2,"1"]},
          {"kind":"Ghost","title":"No evidence","body":"x","actionText":"y","confidence":0.5,"evidenceChunkIDs":[999]}],
 "clusters":[],"comparisons":[],"charts":[],"coverageNotes":[]}
```"#;
        let mock = MockLlm::with(move |_, _| Ok(reply.to_string()));
        let cfg = OverlayConfig {
            enabled: true,
            model: "mock-chat".into(),
            context_tokens: 8192,
            thinking: false,
        };
        let r = overlay(
            base.clone(),
            &graph,
            &chunks,
            &mock,
            &cfg,
            Clock::utc(now),
            8,
        );
        assert_eq!(r.summary_model_name.as_deref(), Some("mock-chat"));
        assert!(!r.used_fallback);
        assert_eq!(r.summary, vec!["Memory graph is your main thread."]);
        assert_eq!(r.cards.len(), 1);
        assert_eq!(r.cards[0].confidence, 0.95);
        assert_eq!(r.cards[0].evidence.len(), 2);
        assert_eq!(r.clusters, base.clusters);
        // Not installed → deterministic with a reason.
        let missing = OverlayConfig {
            model: "llama9".into(),
            ..cfg.clone()
        };
        let r2 = overlay(
            base.clone(),
            &graph,
            &chunks,
            &mock,
            &missing,
            Clock::utc(now),
            8,
        );
        assert!(r2.coverage_notes[0].contains("not installed"));
        // Offline → deterministic with a reason.
        let r3 = overlay(
            base.clone(),
            &graph,
            &chunks,
            &MockLlm::offline(),
            &cfg,
            Clock::utc(now),
            8,
        );
        assert!(r3.summary_model_name.is_none());
        assert!(r3.coverage_notes[0].starts_with(FALLBACK_PREFIX));
        // Junk → reason.
        let junk = MockLlm::fixed("I cannot help with that");
        let r4 = overlay(base, &graph, &chunks, &junk, &cfg, Clock::utc(now), 8);
        assert!(r4.coverage_notes[0].contains("structured JSON"));
    }

    #[test]
    fn merging_findings_prepends_cards_and_headline() {
        let now = 100 * 86_400;
        let (graph, chunks) = fixture(now);
        let base = deterministic(&graph, &chunks, Clock::utc(now), 8);
        let f = InsightFinding {
            id: 1,
            kind: "anomaly".into(),
            subject_key: "volume:today".into(),
            title: "Unusually vocal today".into(),
            detail: "12 dictations today.".into(),
            magnitude: 3.0,
            confidence: 0.8,
            window_start: None,
            window_end: None,
            evidence_chunk_ids: vec![1],
            first_seen_at: 0,
            last_computed_at: 0,
            last_shown_at: None,
            status: "active".into(),
        };
        let r = merge_findings(base.clone(), &[f], &chunks);
        assert_eq!(r.cards[0].kind, "Today");
        assert_eq!(r.cards[0].evidence[0].chunk_id, 1);
        assert_eq!(r.cards.len(), base.cards.len() + 1);
        assert!(r.summary[0].starts_with("Unusually vocal today: "));
        assert_eq!(extract_json_objects("a {\"x\":\"}\"} b {}").len(), 2);
    }
}
