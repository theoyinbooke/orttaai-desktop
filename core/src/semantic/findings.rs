//! Deterministic pattern mining (Mac `InsightPatternEngine`): rhythms, life
//! areas, emerging/fading/resurfacing themes, open commitments and questions,
//! and volume anomalies. Every finding is computed — never guessed — carries
//! evidence chunk ids and a sample-size confidence, and is reconciled into
//! `insight_finding` so insights have memory. [`rank_for_home`] orders them
//! for the Home panel.

use super::analyzer;
use super::memory_graph::{self, MemoryGraph};
use super::util::{djb2, Clock};
use crate::error::Result;
use crate::store::{FindingDraft, InsightFinding, SignalWithContext, Store};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const LIFE_AREA: &str = "life_area";
pub const RHYTHM: &str = "rhythm";
pub const EMERGING: &str = "emerging_theme";
pub const FADING: &str = "fading_theme";
pub const RESURFACING: &str = "resurfacing_theme";
pub const OPEN_COMMITMENT: &str = "open_commitment";
pub const OPEN_QUESTION: &str = "open_question";
pub const ANOMALY: &str = "anomaly";

pub const COMPUTED_KINDS: [&str; 8] = [
    LIFE_AREA,
    RHYTHM,
    EMERGING,
    FADING,
    RESURFACING,
    OPEN_COMMITMENT,
    OPEN_QUESTION,
    ANOMALY,
];

/// Card label per kind (Mac `cardLabels`).
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        LIFE_AREA => "Life Area",
        RHYTHM => "Rhythm",
        EMERGING => "Emerging",
        FADING => "Fading",
        RESURFACING => "Back Again",
        OPEN_COMMITMENT => "Open Commitment",
        OPEN_QUESTION => "Open Question",
        ANOMALY => "Today",
        _ => "Insight",
    }
}

/// Suggested next step per kind (Mac `cardActions`).
pub fn kind_action(kind: &str) -> &'static str {
    match kind {
        LIFE_AREA => "Review what this area needs next.",
        RHYTHM => "Protect this window for your hardest thinking.",
        EMERGING => "Give the new thread a decision or a deadline.",
        FADING => "Confirm it is finished or revive it.",
        RESURFACING => "Recurring loops usually hide an unmade decision.",
        OPEN_COMMITMENT => "Close the loop or consciously let it go.",
        OPEN_QUESTION => "Answer it or capture it as a task.",
        ANOMALY => "Check what's different about today.",
        _ => "Tap the evidence to revisit the source.",
    }
}

#[derive(Debug, Clone)]
pub struct ActivitySample {
    pub created_at: i64,
    pub word_count: i64,
    pub recording_ms: i64,
    pub app_name: Option<String>,
}

pub struct EngineInput<'a> {
    pub activity: &'a [ActivitySample],
    pub graph: &'a MemoryGraph,
    /// source_created_at per chunk id.
    pub chunk_dates: &'a HashMap<i64, i64>,
    pub signals: &'a [SignalWithContext],
    pub clock: Clock,
}

pub fn compute(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let mut out = rhythm_findings(input);
    out.extend(life_area_findings(input));
    out.extend(trajectory_findings(input));
    out.extend(commitment_findings(input));
    out.extend(question_findings(input));
    out.extend(anomaly_findings(input));
    out
}

/// Confidence grows with evidence and saturates.
pub fn confidence_from_samples(count: usize) -> f64 {
    (0.35 + 0.06 * count as f64).min(0.95)
}

pub fn hour_label(hour: i64) -> String {
    let h = hour.rem_euclid(24);
    match h {
        0 => "12am".into(),
        12 => "12pm".into(),
        1..=11 => format!("{h}am"),
        _ => format!("{}pm", h - 12),
    }
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    })
}

fn rhythm_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let samples = input.activity;
    if samples.len() < 12 {
        return Vec::new();
    }
    let mut count_by_hour: HashMap<i64, usize> = HashMap::new();
    let mut wpm_by_hour: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for s in samples {
        let hour = input.clock.hour(s.created_at) as i64;
        *count_by_hour.entry(hour).or_default() += 1;
        if s.recording_ms > 2000 && s.word_count > 4 {
            wpm_by_hour
                .entry(hour)
                .or_default()
                .push(s.word_count as f64 / (s.recording_ms as f64 / 60_000.0));
        }
    }
    let mut out = Vec::new();
    let total = samples.len();
    let (mut best_start, mut best_count) = (0i64, -1i64);
    for start in 0..24 {
        let count: usize = (0..3)
            .map(|o| count_by_hour.get(&((start + o) % 24)).copied().unwrap_or(0))
            .sum();
        if count as i64 > best_count {
            best_count = count as i64;
            best_start = start;
        }
    }
    let share = best_count as f64 / total as f64;
    if share >= 0.3 {
        let end = (best_start + 3) % 24;
        out.push(FindingDraft {
            kind: RHYTHM.into(),
            subject_key: "peak-hours".into(),
            title: format!(
                "Your voice peaks from {} to {}",
                hour_label(best_start),
                hour_label(end)
            ),
            detail: format!(
                "{}% of your {total} dictations happen between {} and {}. That window is when you think out loud the most.",
                (share * 100.0).round() as i64,
                hour_label(best_start),
                hour_label(end)
            ),
            magnitude: share,
            confidence: confidence_from_samples(total),
            window_start: samples.iter().map(|s| s.created_at).min(),
            window_end: samples.iter().map(|s| s.created_at).max(),
            evidence_chunk_ids: Vec::new(),
        });
    }
    let all: Vec<f64> = wpm_by_hour.values().flatten().copied().collect();
    if all.len() >= 10 {
        if let Some(overall) = median(&all) {
            let best = wpm_by_hour
                .iter()
                .filter(|(_, v)| v.len() >= 5)
                .filter_map(|(h, v)| median(v).map(|m| (*h, m)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((hour, m)) = best {
                if overall > 0.0 && m >= overall * 1.15 {
                    let lift = ((m / overall - 1.0) * 100.0).round() as i64;
                    out.push(FindingDraft {
                        kind: RHYTHM.into(),
                        subject_key: "fluency-peak".into(),
                        title: format!("You speak {lift}% faster around {}", hour_label(hour)),
                        detail: format!(
                            "Median dictation speed around {} is {} WPM vs your overall {} WPM. Protect this fluency peak for hard thinking.",
                            hour_label(hour),
                            m.round() as i64,
                            overall.round() as i64
                        ),
                        magnitude: m / overall,
                        confidence: confidence_from_samples(all.len()),
                        window_start: None,
                        window_end: None,
                        evidence_chunk_ids: Vec::new(),
                    });
                }
            }
        }
    }
    out
}

fn chunk_id_of(node_id: &str) -> Option<i64> {
    node_id.strip_prefix("chunk:").and_then(|s| s.parse().ok())
}

fn life_area_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let graph = input.graph;
    if graph.nodes.len() < 12 {
        return Vec::new();
    }
    let mut labels: HashMap<&str, String> = graph
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n.node_id.clone()))
        .collect();
    let mut adjacency: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
    for e in &graph.edges {
        adjacency
            .entry(&e.source_node_id)
            .or_default()
            .push((&e.target_node_id, e.weight));
        adjacency
            .entry(&e.target_node_id)
            .or_default()
            .push((&e.source_node_id, e.weight));
    }
    let mut ordered: Vec<&str> = graph.nodes.iter().map(|n| n.node_id.as_str()).collect();
    ordered.sort_unstable();
    for _ in 0..6 {
        let mut changed = false;
        for id in &ordered {
            let Some(neighbors) = adjacency.get(id).filter(|n| !n.is_empty()) else {
                continue;
            };
            let mut weight_by_label: BTreeMap<String, f64> = BTreeMap::new();
            for (n, w) in neighbors {
                if let Some(l) = labels.get(n) {
                    *weight_by_label.entry(l.clone()).or_default() += w;
                }
            }
            // Highest weight; ties → smallest label.
            let winner = weight_by_label
                .iter()
                .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(l, _)| l.clone());
            if let Some(w) = winner {
                if labels.get(id) != Some(&w) {
                    labels.insert(id, w);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let by_id: HashMap<&str, &crate::store::GraphNodeRecord> = graph
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n))
        .collect();
    let mut communities: BTreeMap<String, Vec<&crate::store::GraphNodeRecord>> = BTreeMap::new();
    for (id, label) in &labels {
        if let Some(n) = by_id.get(id) {
            communities.entry(label.clone()).or_default().push(n);
        }
    }
    let total_chunks = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "chunk")
        .count()
        .max(1);
    let mut ranked: Vec<(String, Vec<&crate::store::GraphNodeRecord>, usize)> = communities
        .into_iter()
        .map(|(l, m)| {
            let c = m.iter().filter(|n| n.kind == "chunk").count();
            (l, m, c)
        })
        .filter(|(_, m, c)| *c >= 3 && m.iter().any(|n| n.kind != "chunk"))
        .collect();
    ranked.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(4)
        .filter_map(|(_, members, chunk_count)| {
            let mut concepts: Vec<_> = members
                .iter()
                .filter(|n| n.kind == "topic" || n.kind == "entity")
                .collect();
            concepts.sort_by(|a, b| {
                b.weight
                    .total_cmp(&a.weight)
                    .then(a.node_id.cmp(&b.node_id))
            });
            let mut apps: Vec<_> = members.iter().filter(|n| n.kind == "app").collect();
            apps.sort_by(|a, b| {
                b.weight
                    .total_cmp(&a.weight)
                    .then(a.node_id.cmp(&b.node_id))
            });
            let anchor = concepts.first().or(apps.first())?;
            let names: Vec<&str> = concepts.iter().take(3).map(|n| n.title.as_str()).collect();
            let app_names: Vec<&str> = apps.iter().take(2).map(|n| n.title.as_str()).collect();
            let share = chunk_count as f64 / total_chunks as f64;
            let mut chunk_ids: Vec<i64> = members
                .iter()
                .filter(|n| n.kind == "chunk")
                .filter_map(|n| chunk_id_of(&n.node_id))
                .collect();
            chunk_ids.sort_unstable();
            let mut parts = Vec::new();
            if !names.is_empty() {
                parts.push(format!("Centers on {}", names.join(", ")));
            }
            if !app_names.is_empty() {
                parts.push(format!("lives in {}", app_names.join(" and ")));
            }
            parts.push(format!(
                "{}% of your indexed dictation",
                (share * 100.0).round() as i64
            ));
            Some(FindingDraft {
                kind: LIFE_AREA.into(),
                subject_key: format!("area:{}", anchor.node_id),
                title: format!(
                    "Life area: {}",
                    names.first().copied().unwrap_or(&anchor.title)
                ),
                detail: format!("{}.", parts.join(" · ")),
                magnitude: share,
                confidence: confidence_from_samples(chunk_count),
                window_start: None,
                window_end: None,
                evidence_chunk_ids: chunk_ids.into_iter().take(8).collect(),
            })
        })
        .collect()
}

fn trajectory_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let by_id: HashMap<&str, &crate::store::GraphNodeRecord> = input
        .graph
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n))
        .collect();
    let mut mentions: BTreeMap<&str, Vec<(i64, i64)>> = BTreeMap::new();
    for e in input
        .graph
        .edges
        .iter()
        .filter(|e| e.kind == "mentions" || e.kind == "entity")
    {
        let (concept, chunk) = if e.source_node_id.starts_with("chunk:") {
            (&e.target_node_id, &e.source_node_id)
        } else {
            (&e.source_node_id, &e.target_node_id)
        };
        let Some(cid) = chunk_id_of(chunk) else {
            continue;
        };
        let Some(date) = input.chunk_dates.get(&cid) else {
            continue;
        };
        mentions.entry(concept).or_default().push((cid, *date));
    }
    let now = input.clock.now;
    let recent_window = 14 * 86_400;
    let week = 7 * 86_400;
    let mut out = Vec::new();
    for (concept_id, list) in mentions {
        let Some(concept) = by_id.get(concept_id) else {
            continue;
        };
        if list.len() < 3 {
            continue;
        }
        let mut dates: Vec<i64> = list.iter().map(|m| m.1).collect();
        dates.sort_unstable();
        let recent = list.iter().filter(|m| now - m.1 <= recent_window).count();
        let older = list
            .iter()
            .filter(|m| {
                let age = now - m.1;
                age > recent_window && age <= recent_window * 2
            })
            .count();
        let mut ev = list.clone();
        ev.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let evidence: Vec<i64> = ev.iter().map(|m| m.0).take(6).collect();
        if recent >= 3 && recent >= older * 2 && older <= 1 {
            out.push(FindingDraft {
                kind: EMERGING.into(),
                subject_key: concept_id.to_string(),
                title: format!("{} is taking off", concept.title),
                detail: format!(
                    "{recent} mentions in the last two weeks vs {older} before. A new thread is forming."
                ),
                magnitude: recent as f64,
                confidence: confidence_from_samples(recent),
                window_start: dates.first().copied(),
                window_end: dates.last().copied(),
                evidence_chunk_ids: evidence,
            });
        } else if older >= 3 && recent == 0 {
            out.push(FindingDraft {
                kind: FADING.into(),
                subject_key: concept_id.to_string(),
                title: format!("{} went quiet", concept.title),
                detail: format!(
                    "Active with {older} mentions, then nothing in two weeks. Finished or abandoned?"
                ),
                magnitude: older as f64,
                confidence: confidence_from_samples(older),
                window_start: dates.first().copied(),
                window_end: dates.last().copied(),
                evidence_chunk_ids: evidence,
            });
        } else if dates.len() >= 3 {
            let last_gap = dates.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
            let last = dates.last().copied().unwrap_or(now);
            if last_gap >= week && now - last <= week {
                out.push(FindingDraft {
                    kind: RESURFACING.into(),
                    subject_key: concept_id.to_string(),
                    title: format!("{} is back", concept.title),
                    detail: format!(
                        "It went silent for {} days and resurfaced this week. Recurring loops like this are usually unfinished business.",
                        last_gap / 86_400
                    ),
                    magnitude: last_gap as f64 / 86_400.0,
                    confidence: confidence_from_samples(dates.len()),
                    window_start: dates.first().copied(),
                    window_end: dates.last().copied(),
                    evidence_chunk_ids: evidence,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        b.magnitude
            .total_cmp(&a.magnitude)
            .then_with(|| a.subject_key.cmp(&b.subject_key))
    });
    out.truncate(6);
    out
}

const COMPLETION_CUES: &[&str] = &[
    "done",
    "finished",
    "completed",
    "fixed",
    "shipped",
    "resolved",
    "works now",
    "working now",
    "it worked",
    "that worked",
    "solved",
];

fn dedup_key(value: &str) -> String {
    value.to_lowercase().chars().take(60).collect()
}

fn commitment_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let now = input.clock.now;
    let mut commitments: Vec<&SignalWithContext> = input
        .signals
        .iter()
        .filter(|s| s.family == "commitment" && now - s.source_created_at <= 30 * 86_400)
        .collect();
    commitments.sort_by(|a, b| {
        b.source_created_at
            .cmp(&a.source_created_at)
            .then(a.id.cmp(&b.id))
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in commitments {
        let key = dedup_key(&c.value);
        if !seen.insert(key.clone()) {
            continue;
        }
        // Resolved when a later chunk names the same top concept with a completion cue.
        let top = analyzer::topic_concepts(&c.value, 2)
            .into_iter()
            .next()
            .map(|t| t.key);
        let resolved = top.as_ref().is_some_and(|top| {
            input.signals.iter().any(|later| {
                later.source_created_at > c.source_created_at && {
                    let text = later.chunk_text.to_lowercase();
                    COMPLETION_CUES.iter().any(|cue| text.contains(cue))
                        && text.contains(top.as_str())
                }
            })
        });
        if resolved {
            continue;
        }
        let age = ((now - c.source_created_at) / 86_400).max(0);
        out.push(FindingDraft {
            kind: OPEN_COMMITMENT.into(),
            subject_key: format!("commitment:{}:{}", c.chunk_id, djb2(&key) & 0xFFFF),
            title: if age == 0 {
                "Said today".into()
            } else {
                format!("Said {age} day{} ago", if age == 1 { "" } else { "s" })
            },
            detail: format!(
                "\u{201C}{}\u{201D}{}",
                c.value,
                c.app_name
                    .as_deref()
                    .map(|a| format!(" in {a}"))
                    .unwrap_or_default()
            ),
            magnitude: age as f64,
            confidence: c.confidence,
            window_start: Some(c.source_created_at),
            window_end: Some(c.source_created_at),
            evidence_chunk_ids: vec![c.chunk_id],
        });
        if out.len() >= 6 {
            break;
        }
    }
    out
}

fn question_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let now = input.clock.now;
    let mut qs: Vec<&SignalWithContext> = input
        .signals
        .iter()
        .filter(|s| {
            s.family == "question"
                && s.confidence >= 0.9
                && now - s.source_created_at <= 14 * 86_400
        })
        .collect();
    qs.sort_by(|a, b| {
        b.source_created_at
            .cmp(&a.source_created_at)
            .then(a.id.cmp(&b.id))
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for q in qs {
        let key = dedup_key(&q.value);
        if !seen.insert(key.clone()) {
            continue;
        }
        out.push(FindingDraft {
            kind: OPEN_QUESTION.into(),
            subject_key: format!("question:{}:{}", q.chunk_id, djb2(&key) & 0xFFFF),
            title: "Still open?".into(),
            detail: format!(
                "\u{201C}{}\u{201D}{}",
                q.value,
                q.app_name
                    .as_deref()
                    .map(|a| format!(" asked in {a}"))
                    .unwrap_or_default()
            ),
            magnitude: (now - q.source_created_at) as f64 / 86_400.0,
            confidence: q.confidence,
            window_start: Some(q.source_created_at),
            window_end: Some(q.source_created_at),
            evidence_chunk_ids: vec![q.chunk_id],
        });
        if out.len() >= 4 {
            break;
        }
    }
    out
}

fn anomaly_findings(input: &EngineInput<'_>) -> Vec<FindingDraft> {
    let clock = input.clock;
    let today = clock.day(clock.now);
    let mut daily: HashMap<i64, i64> = HashMap::new();
    for s in input.activity {
        *daily.entry(clock.day(s.created_at)).or_default() += 1;
    }
    // Only days with dictation count toward the baseline (as on the Mac).
    let baseline: Vec<i64> = (1..=28)
        .filter_map(|o| daily.get(&(today - o)).copied())
        .collect();
    if baseline.len() < 7 {
        return Vec::new();
    }
    let today_count = daily.get(&today).copied().unwrap_or(0);
    let mean = baseline.iter().sum::<i64>() as f64 / baseline.len() as f64;
    let var = baseline
        .iter()
        .map(|c| (*c as f64 - mean).powi(2))
        .sum::<f64>()
        / baseline.len() as f64;
    let std = var.sqrt();
    if std <= 0.5 || mean < 1.0 {
        return Vec::new();
    }
    let z = (today_count as f64 - mean) / std;
    if z.abs() < 2.0 || !(today_count >= 3 || z < 0.0) {
        return Vec::new();
    }
    let direction = if z > 0.0 { "Far Above" } else { "Well Below" };
    vec![FindingDraft {
        kind: ANOMALY.into(),
        subject_key: "volume:today".into(),
        title: if z > 0.0 {
            "Unusually vocal today".into()
        } else {
            "Unusually quiet today".into()
        },
        detail: format!(
            "{today_count} dictations today vs your ~{}/day norm. {direction} baseline.",
            mean.round() as i64
        ),
        magnitude: z.abs(),
        confidence: confidence_from_samples(baseline.len()),
        window_start: Some(clock.start_of_day(clock.now)),
        window_end: Some(clock.now),
        evidence_chunk_ids: Vec::new(),
    }]
}

/// Run the engine over the store and reconcile the findings ledger.
pub fn compute_and_store(store: &Store, clock: Clock) -> Result<Vec<InsightFinding>> {
    let since = clock.now - 60 * 86_400;
    let activity: Vec<ActivitySample> = store
        .semantic_transcriptions_between(since, clock.now + 86_400)?
        .into_iter()
        .map(|r| ActivitySample {
            created_at: r.created_at,
            word_count: r.text.split_whitespace().count() as i64,
            recording_ms: r.duration_ms,
            app_name: r.app,
        })
        .collect();
    let signals = store.semantic_signals(Some(&["commitment", "question"]), None, 2000)?;
    let graph = memory_graph::load_default(store)?;
    let chunk_dates: HashMap<i64, i64> = store
        .semantic_chunks(4000)?
        .into_iter()
        .map(|c| (c.id, c.source_created_at))
        .collect();
    let drafts = compute(&EngineInput {
        activity: &activity,
        graph: &graph,
        chunk_dates: &chunk_dates,
        signals: &signals,
        clock,
    });
    store.semantic_reconcile_findings(&drafts, &COMPUTED_KINDS, clock.now)?;
    store.semantic_findings(None, &["active"], 60)
}

/// A finding ordered for the Home panel.
#[derive(Debug, Clone, Serialize)]
pub struct RankedFinding {
    #[serde(flatten)]
    pub finding: InsightFinding,
    pub label: String,
    pub action: String,
    pub score: f64,
}

/// novelty × confidence × coverage × actionability, at most two per kind.
pub fn rank_for_home(findings: &[InsightFinding], now: i64, limit: usize) -> Vec<RankedFinding> {
    let mut scored: Vec<RankedFinding> = findings
        .iter()
        .filter(|f| f.status == "active")
        .map(|f| {
            let coverage = match f.kind.as_str() {
                LIFE_AREA => f.magnitude.clamp(0.0, 1.0),
                RHYTHM => {
                    if f.subject_key == "fluency-peak" {
                        ((f.magnitude - 1.0) * 2.0).clamp(0.0, 1.0)
                    } else {
                        f.magnitude.clamp(0.0, 1.0)
                    }
                }
                EMERGING | FADING => (f.magnitude / 8.0).clamp(0.0, 1.0),
                RESURFACING => (f.magnitude / 21.0).clamp(0.0, 1.0),
                OPEN_COMMITMENT | OPEN_QUESTION => ((f.magnitude + 1.0) / 7.0).clamp(0.2, 1.0),
                ANOMALY => (f.magnitude / 4.0).clamp(0.0, 1.0),
                _ => 0.5,
            };
            let actionability = match f.kind.as_str() {
                OPEN_COMMITMENT => 1.0,
                OPEN_QUESTION => 0.9,
                ANOMALY => 0.85,
                EMERGING | RESURFACING => 0.8,
                FADING => 0.7,
                RHYTHM => 0.65,
                _ => 0.6,
            };
            let novelty = match f.last_shown_at {
                None => 1.0,
                Some(t) if now - t < 86_400 => 0.55,
                Some(_) => 0.85,
            };
            RankedFinding {
                finding: f.clone(),
                label: kind_label(&f.kind).into(),
                action: kind_action(&f.kind).into(),
                score: (0.25 + coverage) * f.confidence * novelty * actionability,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.finding.id.cmp(&b.finding.id))
    });
    let mut per_kind: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for r in scored {
        let n = per_kind.entry(r.finding.kind.clone()).or_default();
        if *n >= 2 {
            continue;
        }
        *n += 1;
        out.push(r);
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{GraphEdgeRecord, GraphNodeRecord};

    const DAY: i64 = 86_400;

    fn sig(
        id: i64,
        family: &str,
        value: &str,
        text: &str,
        at: i64,
        conf: f64,
    ) -> SignalWithContext {
        SignalWithContext {
            id,
            chunk_id: id,
            family: family.into(),
            value: value.into(),
            confidence: conf,
            model_id: "heuristic-v1".into(),
            extracted_at: at,
            chunk_text: text.into(),
            source_created_at: at,
            app_name: Some("Mail".into()),
            transcription_id: id,
        }
    }

    fn node(id: &str, kind: &str, weight: f64) -> GraphNodeRecord {
        GraphNodeRecord {
            node_id: id.into(),
            kind: kind.into(),
            title: id.split(':').nth(1).unwrap_or(id).into(),
            subtitle: None,
            weight,
            last_seen_at: None,
        }
    }

    fn edge(a: &str, b: &str, kind: &str) -> GraphEdgeRecord {
        GraphEdgeRecord {
            source_node_id: a.into(),
            target_node_id: b.into(),
            kind: kind.into(),
            weight: 0.6,
            evidence: None,
        }
    }

    #[test]
    fn rhythm_peak_and_fluency() {
        let now = 100 * DAY;
        let mut activity = Vec::new();
        for d in 0..14 {
            // 21:00 dictations are fast, others slow.
            activity.push(ActivitySample {
                created_at: now - d * DAY - 3 * 3600,
                word_count: 200,
                recording_ms: 60_000,
                app_name: None,
            });
            activity.push(ActivitySample {
                created_at: now - d * DAY - 12 * 3600,
                word_count: 100,
                recording_ms: 60_000,
                app_name: None,
            });
        }
        let graph = MemoryGraph::default();
        let input = EngineInput {
            activity: &activity,
            graph: &graph,
            chunk_dates: &HashMap::new(),
            signals: &[],
            clock: Clock::utc(now),
        };
        let f = rhythm_findings(&input);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].subject_key, "peak-hours");
        assert!(f[0].magnitude >= 0.3);
        assert_eq!(f[1].subject_key, "fluency-peak");
        assert!(
            f[1].title.starts_with("You speak 33% faster around 9pm"),
            "{}",
            f[1].title
        );
        assert_eq!(hour_label(0), "12am");
        assert_eq!(hour_label(13), "1pm");
    }

    #[test]
    fn commitments_resolve_and_questions_stay_open() {
        let now = 50 * DAY;
        let signals = vec![
            sig(
                1,
                "commitment",
                "I will fix the invoice export.",
                "I will fix the invoice export.",
                now - 3 * DAY,
                0.9,
            ),
            sig(
                2,
                "commitment",
                "I need to call the landlord about rent.",
                "x",
                now - DAY,
                0.7,
            ),
            sig(
                3,
                "question",
                "Should we raise prices?",
                "Should we raise prices?",
                now - 2 * DAY,
                0.95,
            ),
            sig(
                4,
                "question",
                "how do I reset it",
                "how do I reset it",
                now - 2 * DAY,
                0.6,
            ),
            sig(
                5,
                "commitment",
                "Invoice export is done and shipped.",
                "The invoice export is done and shipped.",
                now - DAY / 2,
                0.7,
            ),
        ];
        let graph = MemoryGraph::default();
        let input = EngineInput {
            activity: &[],
            graph: &graph,
            chunk_dates: &HashMap::new(),
            signals: &signals,
            clock: Clock::utc(now),
        };
        let c = commitment_findings(&input);
        assert!(
            c.iter().all(|f| !f.detail.contains("fix the invoice")),
            "{c:?}"
        );
        let landlord = c.iter().find(|f| f.detail.contains("landlord")).unwrap();
        assert_eq!(landlord.title, "Said 1 day ago");
        assert!(landlord.detail.ends_with(" in Mail"));
        let q = question_findings(&input);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].title, "Still open?");
    }

    #[test]
    fn trajectories_and_life_areas() {
        let now = 60 * DAY;
        let mut nodes = vec![
            node("topic:docker", "topic", 5.0),
            node("topic:garden", "topic", 4.0),
            node("app:code", "app", 3.0),
        ];
        let mut edges = Vec::new();
        let mut dates = HashMap::new();
        // docker: 4 recent mentions (emerging). garden: 3 mentions 3–4 weeks ago (fading).
        for i in 0..4 {
            let id = format!("chunk:{i}");
            nodes.push(node(&id, "chunk", 1.0));
            edges.push(edge(&id, "topic:docker", "mentions"));
            edges.push(edge("app:code", &id, "app-context"));
            dates.insert(i, now - (i + 1) * DAY);
        }
        for i in 10..13 {
            let id = format!("chunk:{i}");
            nodes.push(node(&id, "chunk", 1.0));
            edges.push(edge(&id, "topic:garden", "mentions"));
            dates.insert(i, now - (5 + i) * DAY);
        }
        for i in 20..25 {
            nodes.push(node(&format!("chunk:{i}"), "chunk", 1.0));
        }
        let graph = MemoryGraph { nodes, edges };
        let input = EngineInput {
            activity: &[],
            graph: &graph,
            chunk_dates: &dates,
            signals: &[],
            clock: Clock::utc(now),
        };
        let t = trajectory_findings(&input);
        assert!(t
            .iter()
            .any(|f| f.kind == EMERGING && f.subject_key == "topic:docker"));
        assert!(t
            .iter()
            .any(|f| f.kind == FADING && f.subject_key == "topic:garden"));
        let areas = life_area_findings(&input);
        assert!(!areas.is_empty());
        assert!(areas[0].title.starts_with("Life area: "));
        assert!(areas.iter().any(|a| a.detail.contains("docker")));
    }

    #[test]
    fn anomaly_on_a_loud_day() {
        let now = 100 * DAY + 20 * 3600;
        let mut activity = Vec::new();
        let mut push = |t: i64| {
            activity.push(ActivitySample {
                created_at: t,
                word_count: 10,
                recording_ms: 5000,
                app_name: None,
            })
        };
        for d in 1..=14 {
            for k in 0..(1 + d % 3) {
                push(100 * DAY - d * DAY + k * 60);
            }
        }
        for k in 0..12 {
            push(100 * DAY + k * 60);
        }
        let graph = MemoryGraph::default();
        let input = EngineInput {
            activity: &activity,
            graph: &graph,
            chunk_dates: &HashMap::new(),
            signals: &[],
            clock: Clock::utc(now),
        };
        let a = anomaly_findings(&input);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].title, "Unusually vocal today");
        assert!(a[0].detail.contains("Far Above baseline"));
    }

    #[test]
    fn ranking_prefers_actionable_and_novel() {
        let f = |id: i64, kind: &str, mag: f64, shown: Option<i64>| InsightFinding {
            id,
            kind: kind.into(),
            subject_key: format!("s{id}"),
            title: "t".into(),
            detail: "d".into(),
            magnitude: mag,
            confidence: 0.8,
            window_start: None,
            window_end: None,
            evidence_chunk_ids: vec![],
            first_seen_at: 0,
            last_computed_at: 0,
            last_shown_at: shown,
            status: "active".into(),
        };
        let now = 10 * DAY;
        let list = vec![
            f(1, LIFE_AREA, 0.2, None),
            f(2, OPEN_COMMITMENT, 5.0, None),
            f(3, OPEN_COMMITMENT, 4.0, None),
            f(4, OPEN_COMMITMENT, 3.0, None),
            f(5, ANOMALY, 3.0, Some(now - 100)),
        ];
        let r = rank_for_home(&list, now, 10);
        assert_eq!(r[0].finding.id, 2);
        assert_eq!(
            r.iter()
                .filter(|x| x.finding.kind == OPEN_COMMITMENT)
                .count(),
            2
        );
        assert_eq!(r[0].label, "Open Commitment");
        assert_eq!(r.len(), 4);
    }
}
