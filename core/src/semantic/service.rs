//! Store-backed orchestration the app calls: the concept corpus (cached
//! per-dictation extraction, Mac `ConceptGraphStore`), Map graphs and
//! mentions, the Activity report and its model reading, Insights report
//! generation, signals and open loops, and concept briefs.

use super::activity::{
    self, ActivityReport, AnalyzedSession, ProjectCandidate, SessionAnalysis, WorkSession,
};
use super::concept_graph::{
    self, ConceptGraph, ConceptNode, ConceptTimeRange, DictationConcepts, GraphOptions,
};
use super::concepts::{
    self, ConceptKind, DictionaryTermMatcher, ExtractedConcept, EXTRACTOR_VERSION,
};
use super::findings::{self, RankedFinding};
use super::index::{self, IndexConfig};
use super::memory_graph;
use super::report::{self, InsightReport, OverlayConfig};
use super::util::{clipped, sha256_hex, Clock};
use crate::error::Result;
use crate::llm::{ChatOptions, LlmClient};
use crate::settings::Settings;
use crate::store::{
    ConceptExtractionRecord, ConceptSource, InsightFinding, SignalWithContext, Store,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Every dictation with its (cached) concepts.
pub struct ConceptCorpus {
    pub sources: Vec<ConceptSource>,
    pub concepts: HashMap<i64, Vec<ExtractedConcept>>,
    pub dictations: Vec<DictationConcepts>,
    pub preferred_keys: HashSet<String>,
}

const EXTRACT_BATCH: usize = 48;

fn extract_batch(batch: &[&ConceptSource], now: i64) -> Vec<ConceptExtractionRecord> {
    batch
        .iter()
        .map(|s| ConceptExtractionRecord {
            transcription_id: s.id,
            text_hash: sha256_hex(&s.text, 12),
            extractor_version: EXTRACTOR_VERSION,
            concepts_json: serde_json::to_string(&concepts::concepts(&s.text))
                .unwrap_or_else(|_| "[]".into()),
            extracted_at: now,
        })
        .collect()
}

/// Bring the extraction cache up to date (in parallel, saving each batch so an
/// interrupted first pass resumes) and return the corpus.
pub fn concept_corpus(
    store: &Store,
    now: i64,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ConceptCorpus> {
    let sources = store.semantic_concept_sources()?;
    let terms = store.semantic_dictionary_terms()?;
    let mut cached = store.semantic_concept_extractions()?;
    let pending: Vec<&ConceptSource> = sources
        .iter()
        .filter(|s| match cached.get(&s.id) {
            None => true,
            Some(r) => {
                r.text_hash != sha256_hex(&s.text, 12) || r.extractor_version != EXTRACTOR_VERSION
            }
        })
        .collect();
    if !pending.is_empty() {
        let width = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).max(1))
            .unwrap_or(1);
        let batches: Vec<&[&ConceptSource]> = pending.chunks(EXTRACT_BATCH).collect();
        let total = pending.len();
        let mut done = 0;
        progress(0, total);
        for wave in batches.chunks(width) {
            let results: Vec<Vec<ConceptExtractionRecord>> = std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .iter()
                    .map(|b| scope.spawn(move || extract_batch(b, now)))
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect()
            });
            for records in results {
                store.semantic_save_concept_extractions(&records)?;
                done += records.len();
                for r in records {
                    cached.insert(r.transcription_id, r);
                }
            }
            progress(done, total);
        }
    }
    if cached.len() > sources.len() {
        store.semantic_prune_concept_extractions()?;
    }
    let matcher = DictionaryTermMatcher::new(&terms);
    let mut concept_map = HashMap::new();
    let dictations = sources
        .iter()
        .map(|s| {
            let extracted: Vec<ExtractedConcept> = cached
                .get(&s.id)
                .and_then(|r| serde_json::from_str(&r.concepts_json).ok())
                .unwrap_or_default();
            let merged = matcher.merging(&extracted, &s.text);
            concept_map.insert(s.id, merged.clone());
            DictationConcepts {
                dictation_id: s.id,
                date: s.created_at,
                app_name: s.app_name.clone(),
                concepts: merged,
            }
        })
        .collect();
    Ok(ConceptCorpus {
        preferred_keys: matcher.preferred_keys(),
        sources,
        concepts: concept_map,
        dictations,
    })
}

/// Which concept kinds a Map filter shows.
pub fn kind_filter_includes(filter: &str, kind: ConceptKind) -> bool {
    match filter {
        "people" => kind == ConceptKind::Person,
        "names" => matches!(
            kind,
            ConceptKind::Product | ConceptKind::Organization | ConceptKind::Place
        ),
        "topics" => kind == ConceptKind::Topic,
        _ => true,
    }
}

/// The Map graph for `range`, optionally filtered by kind and search text.
pub fn concept_graph(
    store: &Store,
    range: ConceptTimeRange,
    kind_filter: &str,
    query: &str,
    clock: Clock,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ConceptGraph> {
    let corpus = concept_corpus(store, clock.now, progress)?;
    let mut graph = concept_graph::build(
        &corpus.dictations,
        range,
        clock,
        &corpus.preferred_keys,
        &GraphOptions::default(),
    );
    let q = query.trim().to_lowercase();
    if kind_filter != "all" || !q.is_empty() {
        let mut keep: HashSet<String> = graph
            .nodes
            .iter()
            .filter(|n| kind_filter_includes(kind_filter, n.kind))
            .filter(|n| q.is_empty() || n.title.to_lowercase().contains(&q) || n.id.contains(&q))
            .map(|n| n.id.clone())
            .collect();
        if !q.is_empty() {
            // A search shows its matches with their direct neighbors.
            let neighbors: Vec<String> = graph
                .edges
                .iter()
                .filter_map(|e| {
                    if keep.contains(&e.source_id) {
                        Some(e.target_id.clone())
                    } else if keep.contains(&e.target_id) {
                        Some(e.source_id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let by_kind: HashSet<String> = graph
                .nodes
                .iter()
                .filter(|n| kind_filter_includes(kind_filter, n.kind))
                .map(|n| n.id.clone())
                .collect();
            keep.extend(neighbors.into_iter().filter(|n| by_kind.contains(n)));
        }
        graph.nodes.retain(|n| keep.contains(&n.id));
        graph
            .edges
            .retain(|e| keep.contains(&e.source_id) && keep.contains(&e.target_id));
        graph
            .highlights
            .retain(|h| h.concept_ids.iter().all(|id| keep.contains(id)));
    }
    Ok(graph)
}

/// One dictation mentioning a concept, cut to the sentence that names it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConceptMention {
    pub dictation_id: i64,
    pub date: i64,
    pub app_name: Option<String>,
    pub excerpt: String,
}

/// The sentence that mentions `title`, or the opening of the dictation.
pub fn excerpt_mentioning(text: &str, title: &str, limit: usize) -> String {
    let cleaned = text.replace('\n', " ");
    let needle = title.to_lowercase();
    let chosen = super::analyzer::sentences(&cleaned)
        .into_iter()
        .find(|s| s.to_lowercase().contains(&needle))
        .unwrap_or_else(|| cleaned.trim().to_string());
    clipped(&chosen, limit)
}

pub fn mentions(store: &Store, node: &ConceptNode, limit: usize) -> Result<Vec<ConceptMention>> {
    let mut out = Vec::new();
    for id in node.dictation_ids.iter().take(limit) {
        if let Some(r) = store.get_transcription(*id)? {
            out.push(ConceptMention {
                dictation_id: *id,
                date: r.created_at,
                app_name: r.app,
                excerpt: excerpt_mentioning(&r.text, &node.title, 220),
            });
        }
    }
    Ok(out)
}

/// A concept's detail: the node, its neighbors, and what was said.
#[derive(Debug, Clone, Serialize)]
pub struct ConceptDetail {
    pub node: ConceptNode,
    pub related: Vec<RelatedConcept>,
    pub mentions: Vec<ConceptMention>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedConcept {
    pub id: String,
    pub title: String,
    pub kind: ConceptKind,
    pub shared_sessions: usize,
    pub strength: f64,
}

pub fn concept_detail(
    store: &Store,
    range: ConceptTimeRange,
    id: &str,
    clock: Clock,
) -> Result<Option<ConceptDetail>> {
    let graph = concept_graph(store, range, "all", "", clock, &mut |_, _| {})?;
    let Some(node) = graph.node(id).cloned() else {
        return Ok(None);
    };
    let related = graph
        .related(id)
        .into_iter()
        .take(6)
        .map(|(n, e)| RelatedConcept {
            id: n.id.clone(),
            title: n.title.clone(),
            kind: n.kind,
            shared_sessions: e.shared_sessions,
            strength: e.strength,
        })
        .collect();
    let mentions = mentions(store, &node, 4)?;
    Ok(Some(ConceptDetail {
        node,
        related,
        mentions,
    }))
}

/// Prompt for a concept brief (Mac `ConceptBriefWriter.prompt`).
pub fn brief_prompt(
    node: &ConceptNode,
    related: &[String],
    mentions: &[ConceptMention],
    clock: Clock,
) -> String {
    let excerpts = mentions
        .iter()
        .map(|m| {
            format!(
                "- [{}, {}] {}",
                clock.month_day(m.date),
                m.app_name.as_deref().unwrap_or("dictation"),
                m.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "These are excerpts from someone's own voice dictations that mention \"{t}\".\n\
It came up in {} dictations over {} days. It often comes up alongside: {}.\n\n\
Excerpts:\n{excerpts}\n\n\
In 2 or 3 short sentences, addressed to them as \"you\", say what \"{t}\" is to them, what they are doing or deciding about it, and anything that looks unresolved. Use only the excerpts. Be specific. No preamble, no bullet points.",
        node.mention_count,
        node.day_count,
        related.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
        t = node.title
    )
}

/// Candidates for local reading (polish model first: usually loaded already).
pub fn local_model_candidates(settings: &Settings) -> Vec<String> {
    vec![
        settings.polish_model.trim().to_string(),
        settings.semantic_summary_model.trim().to_string(),
    ]
}

/// Write a 2–3 sentence brief about a concept with a local model.
pub fn concept_brief(
    store: &Store,
    client: &dyn LlmClient,
    candidates: &[String],
    range: ConceptTimeRange,
    id: &str,
    clock: Clock,
) -> Result<String> {
    let graph = concept_graph(store, range, "all", "", clock, &mut |_, _| {})?;
    let node = graph.node(id).cloned().ok_or_else(|| {
        crate::error::CoreError::Llm("That concept is no longer on the map.".into())
    })?;
    let Some(model) = index::pick_installed(client, candidates)? else {
        return Err(crate::error::CoreError::Llm(format!(
            "No local model is installed in {}. Install one in Models › AI Features.",
            client.provider_name()
        )));
    };
    let related: Vec<String> = graph
        .related(id)
        .into_iter()
        .map(|(n, _)| n.title.clone())
        .collect();
    let mentions = mentions(store, &node, 8)?;
    let opts = ChatOptions {
        temperature: Some(0.2),
        think: Some(false),
        max_tokens: Some(220),
        num_ctx: Some(4096),
        timeout_ms: Some(60_000),
        ..ChatOptions::model(model)
    };
    let reply = client.complete("", &brief_prompt(&node, &related, &mentions, clock), &opts)?;
    Ok(strip_think(&reply).trim().to_string())
}

fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out
}

// ---- Activity ----------------------------------------------------------------

fn catalog(graph: &ConceptGraph) -> Vec<ProjectCandidate> {
    graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, ConceptKind::Product | ConceptKind::Organization))
        .map(|n| ProjectCandidate {
            key: n.id.clone(),
            title: n.title.clone(),
            salience: n.salience,
        })
        .collect()
}

fn cached_analyses(store: &Store) -> Result<HashMap<String, SessionAnalysis>> {
    Ok(store
        .semantic_session_analyses(activity::PROMPT_VERSION)?
        .into_iter()
        .filter_map(|(k, (_, json))| serde_json::from_str(&json).ok().map(|a| (k, a)))
        .collect())
}

/// The Activity tab report for `range`.
pub fn activity_report(
    store: &Store,
    range: ConceptTimeRange,
    clock: Clock,
) -> Result<ActivityReport> {
    let corpus = concept_corpus(store, clock.now, &mut |_, _| {})?;
    let all = concept_graph::build(
        &corpus.dictations,
        ConceptTimeRange::All,
        clock,
        &corpus.preferred_keys,
        &GraphOptions::default(),
    );
    let readings = cached_analyses(store)?;
    let analyzed: Vec<AnalyzedSession> = activity::sessions(&corpus.sources, &corpus.concepts)
        .into_iter()
        .map(|s| {
            let analysis = readings
                .get(&s.key)
                .cloned()
                .unwrap_or_else(|| activity::heuristic_analysis(&s));
            AnalyzedSession {
                session: s,
                analysis,
            }
        })
        .collect();
    let aliases: HashMap<String, String> = all.aliases.clone().into_iter().collect();
    Ok(activity::build_report(
        &analyzed,
        range,
        &catalog(&all),
        &aliases,
        clock,
    ))
}

/// Sessions a model hasn't read yet, newest first, within `horizon_days`.
pub fn unread_sessions(
    store: &Store,
    horizon_days: Option<i64>,
    clock: Clock,
) -> Result<Vec<WorkSession>> {
    let corpus = concept_corpus(store, clock.now, &mut |_, _| {})?;
    let done = store.semantic_session_analyses(activity::PROMPT_VERSION)?;
    let start = horizon_days.map(|d| clock.now - d * 86_400);
    let mut pending: Vec<WorkSession> = activity::sessions(&corpus.sources, &corpus.concepts)
        .into_iter()
        .filter(|s| {
            !done.contains_key(&s.key) && s.text.chars().count() >= activity::MIN_SESSION_LENGTH
        })
        .filter(|s| start.map(|st| s.end >= st).unwrap_or(true))
        .collect();
    pending.sort_by_key(|a| std::cmp::Reverse(a.end));
    Ok(pending)
}

pub fn save_session_reading(
    store: &Store,
    session: &WorkSession,
    analysis: &SessionAnalysis,
) -> Result<()> {
    store.semantic_save_session_analysis(
        &session.key,
        activity::PROMPT_VERSION,
        analysis.model.as_deref().unwrap_or(""),
        &serde_json::to_string(analysis)?,
        session.end,
    )
}

// ---- Insights ----------------------------------------------------------------

/// Generate (and persist) the Insights report: findings → deterministic
/// report → merged findings → optional local-model phrasing.
pub fn generate_report(
    store: &Store,
    client: &dyn LlmClient,
    cfg: &IndexConfig,
    overlay: &OverlayConfig,
    clock: Clock,
    limit_cards: usize,
) -> Result<InsightReport> {
    let active = index::active_model(store, cfg)?;
    let graph = memory_graph::load_default(store)?;
    let chunks: Vec<_> = store
        .semantic_embedded_chunks(&active, 900)?
        .into_iter()
        .map(|e| e.chunk)
        .collect();
    let found = findings::compute_and_store(store, clock)?;
    let base = report::deterministic(&graph, &chunks, clock, limit_cards);
    let merged = report::merge_findings(base, &found, &chunks);
    let out = report::overlay(merged, &graph, &chunks, client, overlay, clock, limit_cards);
    store.semantic_save_insight_snapshot(
        out.generated_at,
        &out.graph_signature,
        &out.analyzer_name,
        out.summary_model_name.as_deref(),
        out.source_node_count as i64,
        out.source_edge_count as i64,
        out.source_chunk_count as i64,
        &serde_json::to_string(&out)?,
    )?;
    Ok(out)
}

/// The last generated report and whether the graph has moved on since.
#[derive(Debug, Clone, Serialize)]
pub struct StoredReport {
    pub report: InsightReport,
    pub stale: bool,
}

pub fn latest_report(store: &Store) -> Result<Option<StoredReport>> {
    let Some(json) = store.semantic_latest_insight_snapshot()? else {
        return Ok(None);
    };
    let Ok(report) = serde_json::from_str::<InsightReport>(&json) else {
        return Ok(None);
    };
    let graph = memory_graph::load_default(store)?;
    Ok(Some(StoredReport {
        stale: report::is_stale(&report, &graph),
        report,
    }))
}

/// Open loops: commitments and questions still active, newest first.
pub fn open_loops(store: &Store, limit: i64) -> Result<Vec<InsightFinding>> {
    let mut v = store.semantic_findings(
        Some(&[
            findings::OPEN_COMMITMENT.to_string(),
            findings::OPEN_QUESTION.to_string(),
        ]),
        &["active"],
        limit.max(1),
    )?;
    v.sort_by(|a, b| b.window_start.cmp(&a.window_start).then(a.id.cmp(&b.id)));
    Ok(v)
}

/// Ranked findings for the Home panel; marks them shown.
pub fn home_findings(store: &Store, now: i64, limit: usize) -> Result<Vec<RankedFinding>> {
    let all = store.semantic_findings(None, &["active"], 200)?;
    let ranked = findings::rank_for_home(&all, now, limit);
    let ids: Vec<i64> = ranked.iter().map(|r| r.finding.id).collect();
    store.semantic_mark_findings_shown(&ids, now)?;
    Ok(ranked)
}

/// Signals grouped by family for the Insights tab.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SignalsOverview {
    pub commitments: Vec<SignalWithContext>,
    pub questions: Vec<SignalWithContext>,
    pub decisions: Vec<SignalWithContext>,
    pub blockers: Vec<SignalWithContext>,
    /// family → value → count over the window (tone and intent mix).
    pub mix: HashMap<String, HashMap<String, usize>>,
    pub chunk_count: usize,
}

pub fn signals_overview(
    store: &Store,
    since: Option<i64>,
    per_family: usize,
) -> Result<SignalsOverview> {
    let all = store.semantic_signals(None, since, 5000)?;
    let mut out = SignalsOverview::default();
    let mut chunks = HashSet::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for s in all {
        chunks.insert(s.chunk_id);
        if s.family == "tone" || s.family == "intent" {
            if s.value != "reviewed" {
                *out.mix
                    .entry(s.family.clone())
                    .or_default()
                    .entry(s.value.clone())
                    .or_default() += 1;
            }
            continue;
        }
        if !seen.insert((s.family.clone(), s.value.to_lowercase())) {
            continue;
        }
        let list = match s.family.as_str() {
            "commitment" => &mut out.commitments,
            "question" => &mut out.questions,
            "decision" => &mut out.decisions,
            "blocker" => &mut out.blockers,
            _ => continue,
        };
        if list.len() < per_family {
            list.push(s);
        }
    }
    out.chunk_count = chunks.len();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;
    use crate::store::TranscriptionRecord;

    const DAY: i64 = 86_400;

    fn seed(store: &Store, now: i64) {
        let texts = [
            (
                "We shipped Orttaai to Vercel today and the memory graph looks great.",
                "Code",
            ),
            (
                "The memory graph layout in Orttaai is broken on Windows. I'm stuck.",
                "Code",
            ),
            (
                "Let's go with the canvas renderer for Orttaai. Should we add zoom?",
                "Code",
            ),
            (
                "Email Sarah Chen about the Vercel invoice for Orttaai.",
                "Mail",
            ),
            (
                "Sarah Chen wants the budget review for the quarterly plan by Friday.",
                "Mail",
            ),
        ];
        for d in 0..4 {
            for (i, (t, app)) in texts.iter().enumerate() {
                let r = TranscriptionRecord::new(
                    *t,
                    Some((*app).into()),
                    8000,
                    now - d * DAY - (i as i64) * 600,
                );
                store.insert_transcription(&r).unwrap();
            }
        }
    }

    #[test]
    fn corpus_caches_and_graph_filters() {
        let store = Store::open_in_memory().unwrap();
        let now = 100 * DAY;
        seed(&store, now);
        let mut calls = Vec::new();
        let corpus = concept_corpus(&store, now, &mut |d, t| calls.push((d, t))).unwrap();
        assert_eq!(corpus.sources.len(), 20);
        assert_eq!(calls.last(), Some(&(20, 20)));
        // Second pass hits the cache.
        let mut again = Vec::new();
        concept_corpus(&store, now, &mut |d, t| again.push((d, t))).unwrap();
        assert!(again.is_empty());
        let clock = Clock::utc(now);
        let g = concept_graph(
            &store,
            ConceptTimeRange::Month,
            "all",
            "",
            clock,
            &mut |_, _| {},
        )
        .unwrap();
        let orttaai = g.node("orttaai").expect("orttaai on the map");
        assert_eq!(orttaai.kind, ConceptKind::Product);
        let people = concept_graph(
            &store,
            ConceptTimeRange::Month,
            "people",
            "",
            clock,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(people.nodes.iter().all(|n| n.kind == ConceptKind::Person));
        assert!(people.node("sarah chen").is_some());
        let searched = concept_graph(
            &store,
            ConceptTimeRange::Month,
            "all",
            "sarah",
            clock,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(searched.node("sarah chen").is_some());
        assert!(searched.nodes.len() < g.nodes.len());
        let detail = concept_detail(&store, ConceptTimeRange::Month, "orttaai", clock)
            .unwrap()
            .unwrap();
        assert_eq!(detail.mentions.len(), 4);
        assert!(detail.mentions[0]
            .excerpt
            .to_lowercase()
            .contains("orttaai"));
        assert!(!detail.related.is_empty());
        assert!(
            concept_detail(&store, ConceptTimeRange::Month, "nope", clock)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn activity_report_and_model_reading() {
        let store = Store::open_in_memory().unwrap();
        let now = 100 * DAY;
        seed(&store, now);
        let clock = Clock::utc(now);
        let r = activity_report(&store, ConceptTimeRange::Month, clock).unwrap();
        assert!(r.session_count > 0);
        assert_eq!(r.model_analyzed_count, 0);
        assert!(r.headline[0].contains("working session"));
        assert!(r.projects.iter().any(|p| p.title == "Orttaai"));
        let unread = unread_sessions(&store, Some(60), clock).unwrap();
        assert!(!unread.is_empty());
        let mock = MockLlm::fixed(
            r#"{"activity":"designing","subject":"Orttaai","goal":"Polish the map","friction":null,"friction_type":null,"decision":null,"open_question":null}"#,
        );
        let a = activity::analyze_with_model(&mock, "mock-chat", &unread[0])
            .unwrap()
            .unwrap();
        save_session_reading(&store, &unread[0], &a).unwrap();
        let r2 = activity_report(&store, ConceptTimeRange::Month, clock).unwrap();
        assert_eq!(r2.model_analyzed_count, 1);
        assert_eq!(
            unread_sessions(&store, Some(60), clock).unwrap().len(),
            unread.len() - 1
        );
    }

    #[test]
    fn report_generation_findings_and_signals() {
        let store = Store::open_in_memory().unwrap();
        let now = 100 * DAY;
        seed(&store, now);
        let cfg = IndexConfig {
            enabled: true,
            auto_index: true,
            embedding_model: "mock-embed".into(),
            keyword_fallback: true,
            summary_enabled: false,
            summary_model: String::new(),
        };
        index::index_now(&store, &cfg).unwrap();
        let client = MockLlm::fixed("{}");
        let clock = Clock::utc(now);
        index::process_queue(&store, &client, &cfg, clock, 1000, &mut |_| {}).unwrap();
        let overlay = OverlayConfig {
            enabled: false,
            model: String::new(),
            context_tokens: 8192,
            thinking: false,
        };
        let r = generate_report(&store, &client, &cfg, &overlay, clock, 8).unwrap();
        assert!(!r.cards.is_empty());
        let stored = latest_report(&store).unwrap().unwrap();
        assert_eq!(stored.report.generated_at, r.generated_at);
        assert!(!stored.stale);
        let loops = open_loops(&store, 12).unwrap();
        assert!(loops
            .iter()
            .all(|f| f.kind == "open_commitment" || f.kind == "open_question"));
        let home = home_findings(&store, now, 6).unwrap();
        assert!(home.iter().all(|f| f.finding.status == "active"));
        let sig = signals_overview(&store, None, 10).unwrap();
        assert!(!sig.decisions.is_empty());
        assert!(!sig.questions.is_empty());
        assert!(!sig.blockers.is_empty());
        assert!(sig.mix.contains_key("intent"));
        // Brief: no installed candidate → clear error; installed → text.
        let err = concept_brief(
            &store,
            &client,
            &["nope".into()],
            ConceptTimeRange::Month,
            "orttaai",
            clock,
        );
        assert!(err.unwrap_err().to_string().contains("No local model"));
        let brief_client = MockLlm::fixed("<think>hm</think> Orttaai is your app.");
        let brief = concept_brief(
            &store,
            &brief_client,
            &["mock-chat".into()],
            ConceptTimeRange::Month,
            "orttaai",
            clock,
        )
        .unwrap();
        assert_eq!(brief, "Orttaai is your app.");
        assert_eq!(
            excerpt_mentioning("First one. The Orttaai app works. Third.", "orttaai", 220),
            "The Orttaai app works."
        );
    }
}
