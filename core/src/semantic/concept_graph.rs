//! The Map tab's concept graph (Mac `ConceptGraphBuilder`): which concepts
//! matter, how they connect (Ochiai co-occurrence per working session), how
//! they group into themes (Louvain modularity), and what changed (trends and
//! highlights). Pure and deterministic, so it is cheap to rerun per range.

use super::concepts::{ConceptKind, ExtractedConcept};
use super::util::Clock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The concepts found in one dictation, with the context the graph needs.
#[derive(Debug, Clone)]
pub struct DictationConcepts {
    pub dictation_id: i64,
    pub date: i64,
    pub app_name: Option<String>,
    pub concepts: Vec<ExtractedConcept>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConceptTimeRange {
    Week,
    #[default]
    Month,
    Quarter,
    All,
}

impl ConceptTimeRange {
    pub fn title(self) -> &'static str {
        match self {
            ConceptTimeRange::Week => "7 days",
            ConceptTimeRange::Month => "30 days",
            ConceptTimeRange::Quarter => "90 days",
            ConceptTimeRange::All => "All time",
        }
    }

    pub fn days(self) -> Option<i64> {
        match self {
            ConceptTimeRange::Week => Some(7),
            ConceptTimeRange::Month => Some(30),
            ConceptTimeRange::Quarter => Some(90),
            ConceptTimeRange::All => None,
        }
    }

    /// Parses "week" | "month" | "quarter" | "all" (also "7d", "30d", "90d").
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "week" | "7d" | "7" => ConceptTimeRange::Week,
            "quarter" | "90d" | "90" => ConceptTimeRange::Quarter,
            "all" | "all_time" | "alltime" => ConceptTimeRange::All,
            _ => ConceptTimeRange::Month,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConceptTrend {
    New,
    Rising,
    Steady,
    Fading,
}

impl ConceptTrend {
    pub fn label(self) -> Option<&'static str> {
        match self {
            ConceptTrend::New => Some("New"),
            ConceptTrend::Rising => Some("Rising"),
            ConceptTrend::Fading => Some("Quiet lately"),
            ConceptTrend::Steady => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConceptAppShare {
    pub app_name: String,
    pub count: usize,
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConceptNode {
    pub id: String,
    pub kind: ConceptKind,
    pub kind_label: String,
    pub title: String,
    /// Dictations mentioning it inside the selected range.
    pub mention_count: usize,
    /// Distinct local days it came up inside the range.
    pub day_count: usize,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Mentions in the last 7 days, and in the 4 weeks before that (all-time data).
    pub recent_count: usize,
    pub previous_count: usize,
    pub trend: ConceptTrend,
    pub trend_label: Option<String>,
    pub apps: Vec<ConceptAppShare>,
    /// Most recent first, capped; the evidence behind the concept.
    pub dictation_ids: Vec<i64>,
    pub theme_id: Option<String>,
    pub salience: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConceptEdge {
    pub source_id: String,
    pub target_id: String,
    /// Working sessions (same app, within 30 minutes) that mention both.
    pub shared_sessions: usize,
    /// Ochiai coefficient in 0..=1.
    pub strength: f64,
}

impl ConceptEdge {
    pub fn id(&self) -> String {
        format!("{}|{}", self.source_id, self.target_id)
    }
    fn score(&self) -> f64 {
        self.strength * (1.0 + self.shared_sessions as f64).log2()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConceptTheme {
    pub id: String,
    pub title: String,
    pub concept_ids: Vec<String>,
    pub mention_count: usize,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConceptHighlightKind {
    Rising,
    New,
    Pair,
    Person,
    Fading,
    Focus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConceptHighlight {
    pub kind: ConceptHighlightKind,
    pub title: String,
    pub detail: String,
    pub concept_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConceptGraph {
    pub range: ConceptTimeRange,
    pub dictation_count: usize,
    pub nodes: Vec<ConceptNode>,
    pub edges: Vec<ConceptEdge>,
    pub themes: Vec<ConceptTheme>,
    pub highlights: Vec<ConceptHighlight>,
    /// Variant key → the key it was merged into ("mitumo" → "meetumo").
    pub aliases: BTreeMap<String, String>,
    pub generated_at: i64,
}

impl ConceptGraph {
    pub fn empty(range: ConceptTimeRange, now: i64) -> Self {
        Self {
            range,
            dictation_count: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            themes: Vec::new(),
            highlights: Vec::new(),
            aliases: BTreeMap::new(),
            generated_at: now,
        }
    }

    pub fn node(&self, id: &str) -> Option<&ConceptNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Neighbors with their shared-session counts, strongest first.
    pub fn related(&self, id: &str) -> Vec<(&ConceptNode, &ConceptEdge)> {
        let by_id: HashMap<&str, &ConceptNode> =
            self.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut out: Vec<(&ConceptNode, &ConceptEdge)> = self
            .edges
            .iter()
            .filter_map(|e| {
                let other = if e.source_id == id {
                    &e.target_id
                } else if e.target_id == id {
                    &e.source_id
                } else {
                    return None;
                };
                by_id.get(other.as_str()).map(|n| (*n, e))
            })
            .collect();
        out.sort_by(|a, b| {
            b.1.shared_sessions
                .cmp(&a.1.shared_sessions)
                .then(b.1.strength.total_cmp(&a.1.strength))
        });
        out
    }
}

/// Builder knobs (Mac `ConceptGraphBuilder.Options`).
#[derive(Debug, Clone)]
pub struct GraphOptions {
    pub max_nodes: usize,
    /// Dictations in the same app this close together are one session.
    pub session_gap: i64,
    pub max_edges_per_node: usize,
    pub min_edge_mentions: usize,
    pub min_edge_strength: f64,
    pub pair_concepts_per_dictation: usize,
    pub evidence_per_concept: usize,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            max_nodes: 80,
            session_gap: 30 * 60,
            max_edges_per_node: 6,
            min_edge_mentions: 2,
            min_edge_strength: 0.1,
            pair_concepts_per_dictation: 14,
            evidence_per_concept: 40,
        }
    }
}

#[derive(Default)]
struct Accumulator {
    kind_votes: HashMap<ConceptKind, usize>,
    title_votes: HashMap<String, usize>,
    dictation_ids: Vec<(i64, i64)>,
    days: HashSet<i64>,
    apps: HashMap<String, usize>,
    first_seen: i64,
    last_seen: i64,
    recent: usize,
    previous: usize,
    all_time_first_seen: i64,
}

/// Build the graph for `range` from every dictation's concepts.
pub fn build(
    dictations: &[DictationConcepts],
    range: ConceptTimeRange,
    clock: Clock,
    preferred_keys: &HashSet<String>,
    options: &GraphOptions,
) -> ConceptGraph {
    let now = clock.now;
    let range_start = range.days().map(|d| now - d * 86_400);
    let in_range = |d: &DictationConcepts| range_start.map(|s| d.date >= s).unwrap_or(true);
    let in_range_list: Vec<&DictationConcepts> =
        dictations.iter().filter(|d| in_range(d)).collect();
    if in_range_list.is_empty() {
        return ConceptGraph::empty(range, now);
    }

    let aliases = alias_map(dictations, preferred_keys);
    let recent_start = now - 7 * 86_400;
    let previous_start = now - 35 * 86_400;

    let mut accumulators: HashMap<String, Accumulator> = HashMap::new();
    let mut keys_by_session: HashMap<usize, HashSet<String>> = HashMap::new();
    let session_by_dictation = sessions(&in_range_list, options.session_gap);

    for dictation in dictations {
        let is_in_range = in_range(dictation);
        let mut seen = HashSet::new();
        for concept in &dictation.concepts {
            let key = aliases
                .get(&concept.key)
                .cloned()
                .unwrap_or_else(|| concept.key.clone());
            if !seen.insert(key.clone()) {
                continue;
            }
            let acc = accumulators
                .entry(key.clone())
                .or_insert_with(|| Accumulator {
                    first_seen: i64::MAX,
                    last_seen: i64::MIN,
                    all_time_first_seen: i64::MAX,
                    ..Default::default()
                });
            acc.all_time_first_seen = acc.all_time_first_seen.min(dictation.date);
            if dictation.date >= recent_start {
                acc.recent += 1;
            } else if dictation.date >= previous_start {
                acc.previous += 1;
            }
            if is_in_range {
                *acc.kind_votes.entry(concept.kind).or_default() += 1;
                // A merged variant's spelling never names the concept; the
                // canonical key's own mentions do, and a dictionary spelling wins.
                let weight = if preferred_keys.contains(&concept.key) {
                    1000
                } else if concept.key == key {
                    10
                } else {
                    1
                };
                *acc.title_votes.entry(concept.title.clone()).or_default() += weight;
                acc.dictation_ids
                    .push((dictation.dictation_id, dictation.date));
                acc.days.insert(clock.day(dictation.date));
                if let Some(app) = normalized_app(dictation.app_name.as_deref()) {
                    *acc.apps.entry(app).or_default() += 1;
                }
                acc.first_seen = acc.first_seen.min(dictation.date);
                acc.last_seen = acc.last_seen.max(dictation.date);
            }
        }
        if is_in_range {
            if let Some(session) = session_by_dictation.get(&dictation.dictation_id) {
                keys_by_session.entry(*session).or_default().extend(seen);
            }
        }
    }

    let document_count = in_range_list.len();
    let min_mentions = if document_count < 12 { 1 } else { 2 };
    let generic_share = if document_count >= 40 { 0.3 } else { 1.01 };

    struct Candidate {
        key: String,
        kind: ConceptKind,
        title: String,
        salience: f64,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (key, acc) in &accumulators {
        let mentions = acc.dictation_ids.len();
        if mentions < min_mentions || mentions == 0 {
            continue;
        }
        let kind = resolved_kind(&acc.kind_votes, !key.contains(' '));
        if kind == ConceptKind::Topic && mentions as f64 / document_count as f64 > generic_share {
            continue;
        }
        if kind == ConceptKind::Product && mentions < 2 && document_count >= 12 {
            continue;
        }
        let idf = ((document_count + 1) as f64 / (mentions + 1) as f64).ln() + 1.0;
        let recent_share = acc.recent as f64 / mentions.max(1) as f64;
        let spread = (acc.days.len() as f64 / 5.0).min(1.0);
        let specificity = if kind == ConceptKind::Topic && !key.contains(' ') {
            0.55
        } else {
            1.0
        };
        let salience = mentions as f64
            * idf
            * kind.rank_boost()
            * specificity
            * (1.0 + 0.5 * recent_share)
            * (0.75 + 0.25 * spread);
        let title = acc
            .title_votes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(t, _)| t.clone())
            .unwrap_or_else(|| key.clone());
        candidates.push(Candidate {
            key: key.clone(),
            kind,
            title,
            salience,
        });
    }
    candidates.sort_by(|a, b| {
        b.salience
            .total_cmp(&a.salience)
            .then_with(|| a.key.cmp(&b.key))
    });
    candidates.truncate(options.max_nodes);
    let selected_keys: HashSet<&str> = candidates.iter().map(|c| c.key.as_str()).collect();
    let salience_by_key: HashMap<&str, f64> = candidates
        .iter()
        .map(|c| (c.key.as_str(), c.salience))
        .collect();

    // Co-occurrence per working session.
    let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();
    let mut session_counts: HashMap<String, usize> = HashMap::new();
    let mut sessions_sorted: Vec<(&usize, &HashSet<String>)> = keys_by_session.iter().collect();
    sessions_sorted.sort_by_key(|(s, _)| **s);
    for (_, keys) in sessions_sorted {
        for key in keys.iter().filter(|k| selected_keys.contains(k.as_str())) {
            *session_counts.entry(key.clone()).or_default() += 1;
        }
        let mut present: Vec<&String> = keys
            .iter()
            .filter(|k| selected_keys.contains(k.as_str()))
            .collect();
        present.sort_by(|a, b| {
            salience_by_key[b.as_str()]
                .total_cmp(&salience_by_key[a.as_str()])
                .then_with(|| a.cmp(b))
        });
        present.truncate(options.pair_concepts_per_dictation);
        present.sort();
        if present.len() < 2 {
            continue;
        }
        for i in 0..present.len() - 1 {
            for j in i + 1..present.len() {
                *pair_counts
                    .entry((present[i].clone(), present[j].clone()))
                    .or_default() += 1;
            }
        }
    }

    let mut candidate_edges: Vec<ConceptEdge> = Vec::new();
    for ((a, b), count) in &pair_counts {
        if *count < options.min_edge_mentions || overlaps(a, b) {
            continue;
        }
        let (Some(lhs), Some(rhs)) = (session_counts.get(a), session_counts.get(b)) else {
            continue;
        };
        let strength = *count as f64 / ((lhs * rhs) as f64).sqrt();
        if strength < options.min_edge_strength {
            continue;
        }
        candidate_edges.push(ConceptEdge {
            source_id: a.clone(),
            target_id: b.clone(),
            shared_sessions: *count,
            strength: strength.min(1.0),
        });
    }
    candidate_edges.sort_by_key(|e| e.id());

    // Keep each concept's strongest links so hubs don't swallow the map.
    let mut edges_by_node: HashMap<&str, Vec<&ConceptEdge>> = HashMap::new();
    for e in &candidate_edges {
        edges_by_node.entry(&e.source_id).or_default().push(e);
        edges_by_node.entry(&e.target_id).or_default().push(e);
    }
    let mut kept: HashSet<String> = HashSet::new();
    for list in edges_by_node.values_mut() {
        list.sort_by(|a, b| {
            b.score()
                .total_cmp(&a.score())
                .then_with(|| a.id().cmp(&b.id()))
        });
        for e in list.iter().take(options.max_edges_per_node) {
            kept.insert(e.id());
        }
    }
    let mut edges: Vec<ConceptEdge> = candidate_edges
        .into_iter()
        .filter(|e| kept.contains(&e.id()))
        .collect();
    edges.sort_by(|a, b| {
        b.score()
            .total_cmp(&a.score())
            .then_with(|| a.id().cmp(&b.id()))
    });

    let keys: Vec<String> = candidates.iter().map(|c| c.key.clone()).collect();
    let salience_owned: HashMap<String, f64> = candidates
        .iter()
        .map(|c| (c.key.clone(), c.salience))
        .collect();
    let theme_by_key = detect_themes(&keys, &salience_owned, &edges);

    let mut nodes: Vec<ConceptNode> = candidates
        .iter()
        .map(|c| {
            let acc = &accumulators[&c.key];
            let mentions = acc.dictation_ids.len();
            let mut apps: Vec<(&String, &usize)> = acc.apps.iter().collect();
            apps.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            let apps = apps
                .into_iter()
                .take(4)
                .map(|(name, count)| ConceptAppShare {
                    app_name: name.clone(),
                    count: *count,
                    share: *count as f64 / mentions.max(1) as f64,
                })
                .collect();
            let mut evidence = acc.dictation_ids.clone();
            evidence.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
            let trend = trend(
                acc.recent,
                acc.previous,
                acc.all_time_first_seen,
                recent_start,
            );
            ConceptNode {
                id: c.key.clone(),
                kind: c.kind,
                kind_label: c.kind.display_name().to_string(),
                title: c.title.clone(),
                mention_count: mentions,
                day_count: acc.days.len(),
                first_seen: acc.first_seen,
                last_seen: acc.last_seen,
                recent_count: acc.recent,
                previous_count: acc.previous,
                trend,
                trend_label: trend.label().map(String::from),
                apps,
                dictation_ids: evidence
                    .into_iter()
                    .take(options.evidence_per_concept)
                    .map(|(id, _)| id)
                    .collect(),
                theme_id: theme_by_key.get(&c.key).map(|l| format!("theme:{l}")),
                salience: c.salience,
            }
        })
        .collect();

    let themes = make_themes(&nodes, &theme_by_key);
    let theme_ids: HashSet<&str> = themes.iter().map(|t| t.id.as_str()).collect();
    for node in &mut nodes {
        // Only real themes (3+ members) keep an assignment.
        if node
            .theme_id
            .as_deref()
            .map(|t| !theme_ids.contains(t))
            .unwrap_or(false)
        {
            node.theme_id = None;
        }
    }

    let mut graph = ConceptGraph {
        range,
        dictation_count: document_count,
        nodes,
        edges,
        themes,
        highlights: Vec::new(),
        aliases: aliases.into_iter().collect(),
        generated_at: now,
    };
    graph.highlights = highlights(&graph, clock);
    graph
}

/// Merges keys that name the same thing: ASR spelling variants of a name
/// (preferring dictionary spellings, then the most common), spacing variants
/// ("chat gpt" → "chatgpt"), and a short name onto its one full name.
pub fn alias_map(
    dictations: &[DictationConcepts],
    preferred_keys: &HashSet<String>,
) -> HashMap<String, String> {
    let mut named: HashMap<String, ConceptKind> = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for d in dictations {
        for c in &d.concepts {
            *counts.entry(c.key.clone()).or_default() += 1;
            if c.kind == ConceptKind::Topic {
                continue;
            }
            if let Some(existing) = named.get(&c.key) {
                if existing.precedence() >= c.kind.precedence() {
                    continue;
                }
            }
            named.insert(c.key.clone(), c.kind);
        }
    }
    let rank = |k: &str| {
        (
            usize::from(preferred_keys.contains(k)),
            counts.get(k).copied().unwrap_or(0),
        )
    };
    let mut map: HashMap<String, String> = HashMap::new();
    let mut buckets: BTreeMap<char, Vec<&String>> = BTreeMap::new();
    for key in counts.keys().filter(|k| k.chars().count() >= 5) {
        if let Some(first) = key.chars().next() {
            buckets.entry(first).or_default().push(key);
        }
    }
    for keys in buckets.values_mut() {
        keys.sort_by(|a, b| rank(b).cmp(&rank(a)).then_with(|| a.cmp(b)));
        let mut canonicals: Vec<&String> = Vec::new();
        for key in keys.iter() {
            let squashed: String = key.chars().filter(|c| *c != ' ').collect();
            let target = canonicals.iter().find(|canonical| {
                let other: String = canonical.chars().filter(|c| *c != ' ').collect();
                if other == squashed {
                    return true;
                }
                let (lo, ls) = (other.chars().count(), squashed.chars().count());
                let shorter = lo.min(ls);
                if lo.abs_diff(ls) > 2 || shorter < 5 {
                    return false;
                }
                let allowed = if shorter >= 6 { 2 } else { 1 };
                edit_distance(&other, &squashed, allowed) <= allowed
            });
            if let Some(t) = target {
                map.insert((*key).clone(), (*t).clone());
            } else if named.contains_key(key.as_str()) {
                canonicals.push(key);
            }
        }
    }
    // Short names onto their unique full name.
    let mut expansions: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for key in named.keys().filter(|k| !map.contains_key(*k)) {
        let tokens: Vec<&str> = key.split(' ').collect();
        if tokens.len() < 2 {
            continue;
        }
        for token in [tokens[0], tokens[tokens.len() - 1]] {
            if token.chars().count() >= 3 {
                expansions
                    .entry(token.to_string())
                    .or_default()
                    .insert(key.clone());
            }
        }
    }
    for (short, fulls) in expansions {
        if fulls.len() != 1 || map.contains_key(&short) {
            continue;
        }
        if matches!(
            named.get(&short),
            Some(ConceptKind::Person | ConceptKind::Product)
        ) {
            if let Some(full) = fulls.into_iter().next() {
                map.insert(short, full);
            }
        }
    }
    map
}

/// Levenshtein distance, giving up once it exceeds `limit`.
pub fn edit_distance(lhs: &str, rhs: &str, limit: usize) -> usize {
    let a: Vec<char> = lhs.chars().collect();
    let b: Vec<char> = rhs.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            row_min = row_min.min(current[j]);
        }
        if row_min > limit {
            return limit + 1;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Consecutive dictations in the same app with no more than `gap` between
/// them share a session id.
pub fn sessions(dictations: &[&DictationConcepts], gap: i64) -> HashMap<i64, usize> {
    let mut ordered: Vec<&&DictationConcepts> = dictations.iter().collect();
    ordered.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.dictation_id.cmp(&b.dictation_id))
    });
    let mut result = HashMap::new();
    let mut last_by_app: HashMap<String, (i64, usize)> = HashMap::new();
    let mut next = 0;
    for d in ordered {
        let app = d.app_name.clone().unwrap_or_default();
        match last_by_app.get(&app) {
            Some(&(last, session)) if d.date - last <= gap => {
                result.insert(d.dictation_id, session);
                last_by_app.insert(app, (d.date, session));
            }
            _ => {
                result.insert(d.dictation_id, next);
                last_by_app.insert(app, (d.date, next));
                next += 1;
            }
        }
    }
    result
}

/// Louvain modularity communities, labelled by their most salient member.
pub fn detect_themes(
    keys: &[String],
    salience: &HashMap<String, f64>,
    edges: &[ConceptEdge],
) -> HashMap<String, String> {
    let mut ordered: Vec<&String> = keys.iter().collect();
    ordered.sort_by(|a, b| {
        let (l, r) = (
            salience.get(*a).copied().unwrap_or(0.0),
            salience.get(*b).copied().unwrap_or(0.0),
        );
        r.total_cmp(&l).then_with(|| a.cmp(b))
    });
    let index: HashMap<&str, usize> = ordered
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    let n = ordered.len();
    let mut adjacency: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];
    for e in edges {
        let (Some(&a), Some(&b)) = (
            index.get(e.source_id.as_str()),
            index.get(e.target_id.as_str()),
        ) else {
            continue;
        };
        if a == b {
            continue;
        }
        let w = e.score();
        *adjacency[a].entry(b).or_default() += w;
        *adjacency[b].entry(a).or_default() += w;
    }
    let mut level_adjacency = adjacency;
    let mut level_members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    for _ in 0..4 {
        let assignment = louvain_local_moving(&level_adjacency, 1.15);
        let mut communities: Vec<usize> = assignment.clone();
        communities.sort_unstable();
        communities.dedup();
        if communities.len() >= level_adjacency.len() {
            break;
        }
        let remap: HashMap<usize, usize> = communities
            .iter()
            .enumerate()
            .map(|(i, c)| (*c, i))
            .collect();
        let mut next_members = vec![Vec::new(); communities.len()];
        for (super_node, community) in assignment.iter().enumerate() {
            next_members[remap[community]].extend(level_members[super_node].iter().copied());
        }
        let mut next_adjacency: Vec<BTreeMap<usize, f64>> =
            vec![BTreeMap::new(); communities.len()];
        for (node, neighbors) in level_adjacency.iter().enumerate() {
            let from = remap[&assignment[node]];
            for (other, w) in neighbors {
                let to = remap[&assignment[*other]];
                *next_adjacency[from].entry(to).or_default() += w;
            }
        }
        level_members = next_members;
        level_adjacency = next_adjacency;
    }
    let mut membership = vec![0usize; n];
    for (community, members) in level_members.iter().enumerate() {
        for m in members {
            membership[*m] = community;
        }
    }
    let mut leader: HashMap<usize, &String> = HashMap::new();
    for key in &ordered {
        leader.entry(membership[index[key.as_str()]]).or_insert(key);
    }
    ordered
        .iter()
        .map(|k| ((*k).clone(), leader[&membership[index[k.as_str()]]].clone()))
        .collect()
}

fn louvain_local_moving(adjacency: &[BTreeMap<usize, f64>], resolution: f64) -> Vec<usize> {
    let count = adjacency.len();
    let mut community: Vec<usize> = (0..count).collect();
    let degree: Vec<f64> = adjacency.iter().map(|m| m.values().sum()).collect();
    let total_weight = degree.iter().sum::<f64>() / 2.0;
    if total_weight <= 0.0 {
        return community;
    }
    let mut community_total = degree.clone();
    for _ in 0..30 {
        let mut moved = false;
        for node in 0..count {
            let current = community[node];
            community_total[current] -= degree[node];
            let mut weight_to: BTreeMap<usize, f64> = BTreeMap::new();
            for (other, w) in &adjacency[node] {
                if *other != node {
                    *weight_to.entry(community[*other]).or_default() += w;
                }
            }
            let mut best = current;
            let mut best_gain = weight_to.get(&current).copied().unwrap_or(0.0)
                - resolution * community_total[current] * degree[node] / (2.0 * total_weight);
            for (candidate, w) in &weight_to {
                let gain = w - resolution * community_total[*candidate] * degree[node]
                    / (2.0 * total_weight);
                if gain > best_gain + 1e-12 {
                    best = *candidate;
                    best_gain = gain;
                }
            }
            community[node] = best;
            community_total[best] += degree[node];
            if best != current {
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    community
}

fn make_themes(nodes: &[ConceptNode], theme_by_key: &HashMap<String, String>) -> Vec<ConceptTheme> {
    let mut grouped: BTreeMap<String, Vec<&ConceptNode>> = BTreeMap::new();
    for n in nodes {
        let label = theme_by_key
            .get(&n.id)
            .cloned()
            .unwrap_or_else(|| n.id.clone());
        grouped.entry(label).or_default().push(n);
    }
    let mut themes: Vec<ConceptTheme> = grouped
        .into_iter()
        .filter(|(_, m)| m.len() >= 3)
        .map(|(label, mut members)| {
            members.sort_by(|a, b| b.salience.total_cmp(&a.salience));
            ConceptTheme {
                id: format!("theme:{label}"),
                title: members
                    .iter()
                    .take(2)
                    .map(|m| m.title.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
                concept_ids: members.iter().map(|m| m.id.clone()).collect(),
                mention_count: members.iter().map(|m| m.mention_count).sum(),
                last_seen: members.iter().map(|m| m.last_seen).max().unwrap_or(0),
            }
        })
        .collect();
    themes.sort_by(|a, b| {
        b.mention_count
            .cmp(&a.mention_count)
            .then_with(|| a.id.cmp(&b.id))
    });
    themes
}

pub fn trend(recent: usize, previous: usize, first_seen: i64, recent_start: i64) -> ConceptTrend {
    let weekly_baseline = previous as f64 / 4.0;
    if first_seen >= recent_start && recent >= 2 {
        return ConceptTrend::New;
    }
    if recent >= 4 && recent as f64 >= 2.5 * weekly_baseline.max(1.0) {
        return ConceptTrend::Rising;
    }
    if previous >= 6 && recent == 0 {
        return ConceptTrend::Fading;
    }
    ConceptTrend::Steady
}

/// The few findings worth reading first.
pub fn highlights(graph: &ConceptGraph, clock: Clock) -> Vec<ConceptHighlight> {
    let mut result = Vec::new();
    let nodes = &graph.nodes;
    let rising_score =
        |n: &ConceptNode| (n.recent_count + 1) as f64 / (n.previous_count as f64 / 4.0 + 1.0);

    if let Some(r) = nodes
        .iter()
        .filter(|n| n.trend == ConceptTrend::Rising)
        .max_by(|a, b| rising_score(a).total_cmp(&rising_score(b)))
    {
        let weekly = r.previous_count as f64 / 4.0;
        let baseline = if weekly < 1.0 {
            "barely at all before".to_string()
        } else {
            format!("up from about {} a week", weekly.round() as i64)
        };
        result.push(ConceptHighlight {
            kind: ConceptHighlightKind::Rising,
            title: format!("{} is heating up", r.title),
            detail: format!("{} mentions this week, {baseline}.", r.recent_count),
            concept_ids: vec![r.id.clone()],
        });
    }
    if let Some(f) = nodes
        .iter()
        .filter(|n| n.trend == ConceptTrend::New)
        .max_by_key(|n| n.recent_count)
    {
        result.push(ConceptHighlight {
            kind: ConceptHighlightKind::New,
            title: format!("New this week: {}", f.title),
            detail: format!(
                "First came up {}, and {} times since.",
                relative_day(f.first_seen, clock),
                f.recent_count
            ),
            concept_ids: vec![f.id.clone()],
        });
    }
    let by_id: HashMap<&str, &ConceptNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    if let Some(pair) = graph
        .edges
        .iter()
        .filter(|e| {
            e.strength >= 0.25
                && e.shared_sessions >= 3
                && !overlaps(&e.source_id, &e.target_id)
                && (is_specific(by_id.get(e.source_id.as_str()).copied())
                    || is_specific(by_id.get(e.target_id.as_str()).copied()))
        })
        .max_by(|a, b| {
            a.shared_sessions
                .cmp(&b.shared_sessions)
                .then(a.strength.total_cmp(&b.strength))
        })
    {
        if let (Some(l), Some(r)) = (
            by_id.get(pair.source_id.as_str()),
            by_id.get(pair.target_id.as_str()),
        ) {
            result.push(ConceptHighlight {
                kind: ConceptHighlightKind::Pair,
                title: format!("{} and {} go together", l.title, r.title),
                detail: format!(
                    "They come up in the same working session {} times.",
                    pair.shared_sessions
                ),
                concept_ids: vec![l.id.clone(), r.id.clone()],
            });
        }
    }
    if let Some(p) = nodes
        .iter()
        .filter(|n| n.kind == ConceptKind::Person && n.mention_count >= 3)
        .max_by_key(|n| n.mention_count)
    {
        result.push(ConceptHighlight {
            kind: ConceptHighlightKind::Person,
            title: format!("{} comes up most", p.title),
            detail: format!(
                "Mentioned in {} dictations across {} day{}.",
                p.mention_count,
                p.day_count,
                if p.day_count == 1 { "" } else { "s" }
            ),
            concept_ids: vec![p.id.clone()],
        });
    }
    if let Some(f) = nodes
        .iter()
        .filter(|n| n.trend == ConceptTrend::Fading)
        .max_by_key(|n| n.previous_count)
    {
        result.push(ConceptHighlight {
            kind: ConceptHighlightKind::Fading,
            title: format!("{} has gone quiet", f.title),
            detail: format!(
                "{} mentions in the weeks before, none in the last 7 days.",
                f.previous_count
            ),
            concept_ids: vec![f.id.clone()],
        });
    }
    if let Some(focused) = nodes
        .iter()
        .filter(|n| {
            n.mention_count >= 5
                && n.apps.first().map(|a| a.share).unwrap_or(0.0) >= 0.75
                && n.kind != ConceptKind::Topic
        })
        .max_by_key(|n| n.mention_count)
    {
        if let Some(app) = focused.apps.first() {
            result.push(ConceptHighlight {
                kind: ConceptHighlightKind::Focus,
                title: format!("{} lives in {}", focused.title, app.app_name),
                detail: format!(
                    "{}% of its mentions happen while dictating in {}.",
                    (app.share * 100.0).round() as i64,
                    app.app_name
                ),
                concept_ids: vec![focused.id.clone()],
            });
        }
    }
    result.truncate(4);
    result
}

/// A named thing or a multi-word topic, rather than one generic word.
pub fn is_specific(node: Option<&ConceptNode>) -> bool {
    node.map(|n| n.kind != ConceptKind::Topic || n.id.contains(' '))
        .unwrap_or(false)
}

/// "design" and "design system" co-occur by construction.
pub fn overlaps(lhs: &str, rhs: &str) -> bool {
    let left: HashSet<&str> = lhs.split(' ').collect();
    rhs.split(' ').any(|t| left.contains(t))
}

pub fn relative_day(ts: i64, clock: Clock) -> String {
    let days = clock.day(clock.now) - clock.day(ts);
    match days {
        d if d < 1 => "today".to_string(),
        1 => "yesterday".to_string(),
        _ => format!("on {}", clock.weekday_name(ts)),
    }
}

fn resolved_kind(votes: &HashMap<ConceptKind, usize>, single_word: bool) -> ConceptKind {
    let total: usize = votes.values().sum();
    let named_total: usize = votes
        .iter()
        .filter(|(k, _)| **k != ConceptKind::Topic)
        .map(|(_, v)| *v)
        .sum();
    // A multi-word name needs a third of its mentions tagged as a name; a
    // single word needs most, or ordinary words capitalized once pass.
    let threshold = if single_word { 0.5 } else { 1.0 / 3.0 };
    let best = votes
        .iter()
        .filter(|(k, _)| **k != ConceptKind::Topic)
        .max_by(|a, b| a.1.cmp(b.1).then(a.0.precedence().cmp(&b.0.precedence())));
    match best {
        Some((k, _)) if named_total as f64 >= total as f64 * threshold => *k,
        _ => ConceptKind::Topic,
    }
}

fn normalized_app(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept(key: &str, kind: ConceptKind) -> ExtractedConcept {
        ExtractedConcept {
            key: key.into(),
            title: super::super::analyzer::capitalize_words(key),
            kind,
        }
    }

    fn dictation(id: i64, date: i64, app: &str, keys: &[(&str, ConceptKind)]) -> DictationConcepts {
        DictationConcepts {
            dictation_id: id,
            date,
            app_name: Some(app.into()),
            concepts: keys.iter().map(|(k, kind)| concept(k, *kind)).collect(),
        }
    }

    const DAY: i64 = 86_400;

    #[test]
    fn aliases_merge_variants_and_short_names() {
        let ds = vec![
            dictation(1, 0, "a", &[("meetumo", ConceptKind::Product)]),
            dictation(2, 0, "a", &[("meetumo", ConceptKind::Product)]),
            dictation(3, 0, "a", &[("mitumo", ConceptKind::Topic)]),
            dictation(4, 0, "a", &[("sam rivera", ConceptKind::Person)]),
            dictation(5, 0, "a", &[("sam", ConceptKind::Person)]),
            dictation(6, 0, "a", &[("chat gpt", ConceptKind::Product)]),
            dictation(7, 0, "a", &[("chatgpt", ConceptKind::Product)]),
            dictation(8, 0, "a", &[("chatgpt", ConceptKind::Product)]),
        ];
        let map = alias_map(&ds, &HashSet::new());
        assert_eq!(map.get("mitumo").map(String::as_str), Some("meetumo"));
        assert_eq!(map.get("sam").map(String::as_str), Some("sam rivera"));
        assert_eq!(map.get("chat gpt").map(String::as_str), Some("chatgpt"));
        assert_eq!(edit_distance("kitten", "sitting", 5), 3);
        assert_eq!(edit_distance("abcdef", "uvwxyz", 1), 2);
    }

    #[test]
    fn sessions_split_by_app_and_gap() {
        let a = dictation(1, 0, "code", &[]);
        let b = dictation(2, 10 * 60, "code", &[]);
        let c = dictation(3, 11 * 60, "mail", &[]);
        let d = dictation(4, 2 * 3600, "code", &[]);
        let s = sessions(&[&a, &b, &c, &d], 30 * 60);
        assert_eq!(s[&1], s[&2]);
        assert_ne!(s[&2], s[&3]);
        assert_ne!(s[&1], s[&4]);
    }

    #[test]
    fn builds_nodes_edges_themes_and_trends() {
        let now = 100 * DAY;
        let clock = Clock::utc(now);
        let mut ds = Vec::new();
        let mut id = 0;
        // Two clusters of co-occurring concepts across many sessions.
        for day in 0..10 {
            let t = now - day * DAY;
            id += 1;
            ds.push(dictation(
                id,
                t,
                "code",
                &[
                    ("orttaai", ConceptKind::Product),
                    ("memory graph", ConceptKind::Topic),
                    ("concept map", ConceptKind::Topic),
                ],
            ));
            id += 1;
            ds.push(dictation(
                id,
                t + 3 * 3600,
                "mail",
                &[
                    ("sarah chen", ConceptKind::Person),
                    ("budget review", ConceptKind::Topic),
                    ("quarterly plan", ConceptKind::Topic),
                ],
            ));
        }
        // A brand-new concept this week.
        for k in 0..3 {
            id += 1;
            ds.push(dictation(
                id,
                now - k * 3600 - 7200,
                "notes",
                &[("vercel", ConceptKind::Product)],
            ));
        }
        let g = build(
            &ds,
            ConceptTimeRange::Month,
            clock,
            &HashSet::new(),
            &GraphOptions::default(),
        );
        assert_eq!(g.dictation_count, 23);
        let orttaai = g.node("orttaai").expect("orttaai node");
        assert_eq!(orttaai.kind, ConceptKind::Product);
        assert_eq!(orttaai.mention_count, 10);
        assert_eq!(orttaai.day_count, 10);
        assert_eq!(orttaai.apps[0].app_name, "code");
        assert!(g.edges.iter().any(|e| {
            (e.source_id == "memory graph" && e.target_id == "orttaai")
                || (e.source_id == "orttaai" && e.target_id == "memory graph")
        }));
        assert!(!g.edges.iter().any(|e| {
            [e.source_id.as_str(), e.target_id.as_str()].contains(&"orttaai")
                && [e.source_id.as_str(), e.target_id.as_str()].contains(&"sarah chen")
        }));
        assert_eq!(g.themes.len(), 2, "{:?}", g.themes);
        assert_eq!(
            g.node("orttaai").unwrap().theme_id,
            g.node("concept map").unwrap().theme_id
        );
        assert_ne!(
            g.node("orttaai").unwrap().theme_id,
            g.node("sarah chen").unwrap().theme_id
        );
        assert_eq!(g.node("vercel").unwrap().trend, ConceptTrend::New);
        assert!(g
            .highlights
            .iter()
            .any(|h| h.kind == ConceptHighlightKind::New));
        assert!(g
            .highlights
            .iter()
            .any(|h| h.kind == ConceptHighlightKind::Person));
        assert!(g.highlights.len() <= 4);
        let related = g.related("orttaai");
        assert!(!related.is_empty());
        // A week range excludes older dictations.
        let w = build(
            &ds,
            ConceptTimeRange::Week,
            clock,
            &HashSet::new(),
            &GraphOptions::default(),
        );
        assert!(w.dictation_count < g.dictation_count);
    }

    #[test]
    fn trend_rules() {
        let rs = 1000;
        assert_eq!(trend(2, 0, 2000, rs), ConceptTrend::New);
        assert_eq!(trend(5, 4, 0, rs), ConceptTrend::Rising);
        assert_eq!(trend(0, 6, 0, rs), ConceptTrend::Fading);
        assert_eq!(trend(1, 4, 0, rs), ConceptTrend::Steady);
        assert!(overlaps("design", "design system"));
        assert!(!overlaps("design", "system"));
        assert_eq!(ConceptTimeRange::parse("90d"), ConceptTimeRange::Quarter);
    }
}
