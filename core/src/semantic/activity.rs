//! Work activity (Mac `WorkActivity`, `WorkSessionBuilder`, `SessionHeuristics`,
//! `SessionModelAnalyzer`, `ActivityReportBuilder`): dictations grouped into
//! working sessions, each read by a local model once (cached in
//! `session_analysis`) or estimated on-device from wording, then summarized
//! for a time range — kind of work, what got in the way, projects, decisions
//! and open questions.

use super::analyzer;
use super::concept_graph::ConceptTimeRange;
use super::concepts::{normalized_key, ConceptKind, ExtractedConcept};
use super::util::{clipped, sha256_hex, Clock};
use crate::error::Result;
use crate::llm::{ChatMessage, ChatOptions, LlmClient};
use crate::store::ConceptSource;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkActivity {
    Building,
    Fixing,
    Designing,
    Planning,
    Researching,
    Writing,
    Communicating,
    Setup,
    Reviewing,
    Other,
}

impl WorkActivity {
    pub const ALL: [WorkActivity; 10] = [
        WorkActivity::Building,
        WorkActivity::Fixing,
        WorkActivity::Designing,
        WorkActivity::Planning,
        WorkActivity::Researching,
        WorkActivity::Writing,
        WorkActivity::Communicating,
        WorkActivity::Setup,
        WorkActivity::Reviewing,
        WorkActivity::Other,
    ];

    pub fn raw(self) -> &'static str {
        match self {
            WorkActivity::Building => "building",
            WorkActivity::Fixing => "fixing",
            WorkActivity::Designing => "designing",
            WorkActivity::Planning => "planning",
            WorkActivity::Researching => "researching",
            WorkActivity::Writing => "writing",
            WorkActivity::Communicating => "communicating",
            WorkActivity::Setup => "setup",
            WorkActivity::Reviewing => "reviewing",
            WorkActivity::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|a| a.raw() == s.trim().to_lowercase())
            .unwrap_or(WorkActivity::Other)
    }

    pub fn title(self) -> &'static str {
        match self {
            WorkActivity::Building => "Building features",
            WorkActivity::Fixing => "Fixing problems",
            WorkActivity::Designing => "Designing the interface",
            WorkActivity::Planning => "Planning and deciding",
            WorkActivity::Researching => "Researching and learning",
            WorkActivity::Writing => "Writing and content",
            WorkActivity::Communicating => "Communicating",
            WorkActivity::Setup => "Setup and tooling",
            WorkActivity::Reviewing => "Reviewing and testing",
            WorkActivity::Other => "Other",
        }
    }

    /// Lowercase phrase for sentences ("most of it was fixing problems").
    pub fn phrase(self) -> String {
        let t = self.title();
        let mut c = t.chars();
        match c.next() {
            Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FrictionType {
    Bugs,
    Layout,
    Performance,
    Tooling,
    AiQuality,
    Integration,
    Direction,
    Other,
}

impl FrictionType {
    pub fn raw(self) -> &'static str {
        match self {
            FrictionType::Bugs => "bugs",
            FrictionType::Layout => "layout",
            FrictionType::Performance => "performance",
            FrictionType::Tooling => "tooling",
            FrictionType::AiQuality => "ai_quality",
            FrictionType::Integration => "integration",
            FrictionType::Direction => "direction",
            FrictionType::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            FrictionType::Bugs,
            FrictionType::Layout,
            FrictionType::Performance,
            FrictionType::Tooling,
            FrictionType::AiQuality,
            FrictionType::Integration,
            FrictionType::Direction,
            FrictionType::Other,
        ]
        .into_iter()
        .find(|f| f.raw() == s.trim().to_lowercase())
    }

    pub fn title(self) -> &'static str {
        match self {
            FrictionType::Bugs => "Bugs and errors",
            FrictionType::Layout => "UI and layout",
            FrictionType::Performance => "Speed and performance",
            FrictionType::Tooling => "Tools and setup",
            FrictionType::AiQuality => "AI output quality",
            FrictionType::Integration => "Integrations and APIs",
            FrictionType::Direction => "Unclear direction",
            FrictionType::Other => "Other",
        }
    }
}

/// What one working session was about.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionAnalysis {
    pub activity: WorkActivity,
    pub subject: Option<String>,
    pub goal: Option<String>,
    pub friction: Option<String>,
    pub friction_type: Option<FrictionType>,
    pub decision: Option<String>,
    pub open_question: Option<String>,
    /// Model name, or `None` for the on-device estimate.
    pub model: Option<String>,
}

impl SessionAnalysis {
    pub fn is_estimate(&self) -> bool {
        self.model.is_none()
    }
}

/// Consecutive dictations in one app, close together in time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkSession {
    pub key: String,
    pub dictation_ids: Vec<i64>,
    pub start: i64,
    pub end: i64,
    pub app_name: Option<String>,
    pub text: String,
    pub concept_keys: Vec<String>,
}

pub const SESSION_GAP: i64 = 30 * 60;
pub const MAX_SESSION_TEXT: usize = 3600;
/// Sessions shorter than this carry too little to read with a model.
pub const MIN_SESSION_LENGTH: usize = 120;
pub const PROMPT_VERSION: i64 = 2;

fn session_group(s: &ConceptSource) -> String {
    match s.app_name.as_deref().filter(|a| !a.is_empty()) {
        Some(app) => format!("app:{app}"),
        None => "device:".into(),
    }
}

/// Group dictations into working sessions (Mac session gap rule: same app,
/// ≤ 30 minutes since the previous dictation in that app).
pub fn sessions(
    sources: &[ConceptSource],
    concepts: &HashMap<i64, Vec<ExtractedConcept>>,
) -> Vec<WorkSession> {
    let mut ordered: Vec<&ConceptSource> = sources.iter().collect();
    ordered.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut groups: Vec<Vec<&ConceptSource>> = Vec::new();
    let mut open: HashMap<String, (usize, i64)> = HashMap::new();
    for s in ordered {
        let g = session_group(s);
        match open.get(&g) {
            Some(&(idx, last)) if s.created_at - last <= SESSION_GAP => {
                groups[idx].push(s);
                open.insert(g, (idx, s.created_at));
            }
            _ => {
                groups.push(vec![s]);
                open.insert(g, (groups.len() - 1, s.created_at));
            }
        }
    }
    groups
        .into_iter()
        .map(|group| {
            let mut seen = HashSet::new();
            let keys = group
                .iter()
                .flat_map(|s| concepts.get(&s.id).cloned().unwrap_or_default())
                .filter(|c| c.kind != ConceptKind::Topic || c.key.contains(' '))
                .map(|c| c.key)
                .filter(|k| seen.insert(k.clone()))
                .collect();
            WorkSession {
                key: session_key(&group),
                dictation_ids: group.iter().map(|s| s.id).collect(),
                start: group[0].created_at,
                end: group[group.len() - 1].created_at,
                app_name: group[0].app_name.clone(),
                text: condensed(&group.iter().map(|s| s.text.as_str()).collect::<Vec<_>>()),
                concept_keys: keys,
            }
        })
        .collect()
}

/// Changes whenever any dictation in the session changes.
pub fn session_key(group: &[&ConceptSource]) -> String {
    let mut material = String::new();
    for s in group {
        material.push_str(&format!(
            "{}\u{1F}{}\u{1F}{}\u{1F}",
            s.id,
            s.created_at,
            session_group(s)
        ));
        material.push_str(&s.text);
        material.push('\u{1E}');
    }
    format!(
        "{}-{}-{}",
        group.first().map(|s| s.id).unwrap_or(0),
        group.len(),
        sha256_hex(&material, 10)
    )
}

/// Long sessions keep their start and end, where goals and conclusions are.
pub fn condensed(texts: &[&str]) -> String {
    let joined = texts
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let n = joined.chars().count();
    if n <= MAX_SESSION_TEXT {
        return joined;
    }
    let head: String = joined.chars().take(MAX_SESSION_TEXT * 2 / 3).collect();
    let tail: String = joined.chars().skip(n - MAX_SESSION_TEXT / 3).collect();
    format!("{head}\n\n[…]\n\n{tail}")
}

// ---- On-device estimate ------------------------------------------------------

const ACTIVITY_CUES: &[(WorkActivity, &[&str])] = &[
    (
        WorkActivity::Fixing,
        &[
            "bug",
            "error",
            "crash",
            "broken",
            "not working",
            "doesn't work",
            "does not work",
            "fix",
            "issue",
            "wrong",
            "failing",
            "fails",
            "stuck",
        ],
    ),
    (
        WorkActivity::Designing,
        &[
            "layout", "design", "color", "spacing", "padding", "font", "icon", "align", "button",
            "screen", "look", "ui", "compact", "card",
        ],
    ),
    (
        WorkActivity::Building,
        &[
            "add ",
            "implement",
            "build",
            "create",
            "feature",
            "integrate",
            "support for",
            "new page",
            "port",
        ],
    ),
    (
        WorkActivity::Planning,
        &[
            "plan",
            "strategy",
            "priorit",
            "roadmap",
            "should we",
            "decide",
            "approach",
            "next step",
            "goal",
        ],
    ),
    (
        WorkActivity::Researching,
        &[
            "how does",
            "what is",
            "explain",
            "research",
            "learn",
            "understand",
            "compare",
            "difference between",
            "look up",
        ],
    ),
    (
        WorkActivity::Writing,
        &[
            "write",
            "post",
            "caption",
            "script",
            "blog",
            "article",
            "email",
            "tweet",
            "newsletter",
            "copy",
        ],
    ),
    (
        WorkActivity::Communicating,
        &[
            "tell him",
            "tell her",
            "reply",
            "message",
            "let them know",
            "meeting",
            "call with",
        ],
    ),
    (
        WorkActivity::Setup,
        &[
            "install",
            "configure",
            "setup",
            "set up",
            "deploy",
            "account",
            "domain",
            "sign in",
            "permission",
            "dns",
        ],
    ),
    (
        WorkActivity::Reviewing,
        &[
            "test",
            "verify",
            "check",
            "review",
            "confirm",
            "screenshot",
            "try it",
            "works now",
        ],
    ),
];

const FRICTION_CUES: &[&str] = &[
    "not working",
    "doesn't work",
    "does not work",
    "isn't working",
    "is not working",
    "still not",
    "still doesn't",
    "still broken",
    "broken",
    "error",
    "crash",
    "failed",
    "fails",
    "stuck",
    "bug",
    "not showing",
    "doesn't show",
    "not loading",
    "frustrat",
    "annoying",
    "keeps breaking",
    "keeps failing",
    "went wrong",
    "is wrong",
];

const FRICTION_TYPE_CUES: &[(FrictionType, &[&str])] = &[
    (
        FrictionType::Layout,
        &[
            "layout",
            "align",
            "spacing",
            "padding",
            "overlap",
            "too big",
            "too small",
            "cut off",
            "ui",
            "look",
        ],
    ),
    (
        FrictionType::Performance,
        &[
            "slow",
            "lag",
            "fast",
            "performance",
            "freeze",
            "hang",
            "memory",
            "cpu",
        ],
    ),
    (
        FrictionType::Tooling,
        &[
            "install",
            "build",
            "xcode",
            "config",
            "setup",
            "deploy",
            "sign",
            "permission",
            "terminal",
            "cargo",
            "npm",
        ],
    ),
    (
        FrictionType::AiQuality,
        &[
            "model",
            "hallucinat",
            "ai ",
            "agent",
            "prompt",
            "response",
            "output",
            "chatgpt",
            "claude",
            "codex",
        ],
    ),
    (
        FrictionType::Integration,
        &[
            "api",
            "endpoint",
            "integration",
            "sync",
            "server",
            "webhook",
            "auth",
        ],
    ),
    (
        FrictionType::Direction,
        &["not sure", "confus", "unclear", "don't know", "which one"],
    ),
];

const SESSION_DECISION_CUES: &[&str] = &[
    "let's go with",
    "we'll go with",
    "we are going with",
    "we're going with",
    "decided",
    "i've decided",
    "going forward",
    "from now on",
    "the plan is",
    "let's stick with",
];

/// Occurrences of `cue` in `text`. Short cues ("ui", "fix", "plan") must
/// start a word so "build" never counts as "ui" and "explain" never as "plan".
fn occurrences(cue: &str, text: &str) -> usize {
    let word = cue.trim();
    if word.chars().count() > 4 {
        return text.matches(cue).count();
    }
    text.match_indices(word)
        .filter(|(i, _)| {
            text[..*i]
                .chars()
                .next_back()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        })
        .count()
}

fn cue_in(cue: &str, text: &str) -> bool {
    occurrences(cue, text) > 0
}

/// Wording-based estimate used until (or instead of) a local model.
pub fn heuristic_analysis(session: &WorkSession) -> SessionAnalysis {
    let lowered = session.text.to_lowercase().replace('’', "'");
    let mut best: Option<(WorkActivity, usize)> = None;
    for (activity, cues) in ACTIVITY_CUES {
        let score: usize = cues.iter().map(|c| occurrences(c, &lowered)).sum();
        if score == 0 {
            continue;
        }
        best = match best {
            Some((a, s)) if s > score || (s == score && a.raw() < activity.raw()) => Some((a, s)),
            _ => Some((*activity, score)),
        };
    }
    let sentences = analyzer::sentences(&session.text);
    let friction_sentence = sentences.iter().find(|s| {
        let l = s.to_lowercase().replace('’', "'");
        FRICTION_CUES.iter().any(|c| cue_in(c, &l))
    });
    let friction_type = friction_sentence.map(|s| {
        let l = s.to_lowercase();
        FRICTION_TYPE_CUES
            .iter()
            .find(|(_, cues)| cues.iter().any(|c| cue_in(c, &l)))
            .map(|(t, _)| *t)
            .unwrap_or(FrictionType::Bugs)
    });
    let decision = sentences.iter().find(|s| {
        let l = s.to_lowercase().replace('’', "'");
        SESSION_DECISION_CUES.iter().any(|c| l.contains(c))
    });
    let question = sentences
        .iter()
        .rev()
        .find(|s| s.ends_with('?') && s.chars().count() > 20);
    SessionAnalysis {
        activity: best.map(|b| b.0).unwrap_or(WorkActivity::Other),
        subject: None,
        goal: None,
        friction: friction_sentence.map(|s| clipped(s, 140)),
        friction_type,
        decision: decision.map(|s| clipped(s, 140)),
        open_question: question.map(|s| clipped(s, 140)),
        model: None,
    }
}

// ---- Local model ---------------------------------------------------------------

pub fn model_prompt(session: &WorkSession) -> String {
    format!(
        "Below is one working session of a person's voice dictation, spoken while they were in {}. Describe what was going on in it.\n\n\
Return JSON with:\n\
- activity: the main kind of work. building = adding or extending features; fixing = something is broken or wrong and they want it repaired; designing = how things look or are laid out; planning = strategy, priorities, deciding what to do next; researching = learning or asking how something works; writing = content, posts, emails, docs; communicating = messages to other people; setup = installing, configuring, deploying, accounts; reviewing = testing or checking results; other.\n\
- subject: the project or thing it was about, 1 to 4 words, using its real name when one is said.\n\
- goal: what they were trying to get done, specific, under 12 words.\n\
- friction: a problem that actually got in their way: something failed, broke, errored, behaved wrongly, was slow, or blocked them. A change they simply want is NOT friction. Under 12 words, or null.\n\
- friction_type (null when friction is null): bugs = errors, crashes, broken behavior; layout = something displays wrongly (misaligned, cut off, overlapping, cluttered); performance = slow, lagging, heavy; tooling = build, install, signing, developer tools; ai_quality = an AI assistant or model gave poor or wrong output; integration = an API, sync, service, or account misbehaved; direction = they said they were confused or unsure what to do; other.\n\
- decision: a choice they clearly made (\"let's use X\", \"we'll drop Y\"), under 12 words, or null.\n\
- open_question: a question they asked that is still open, written as a question, under 14 words, or null.\n\n\
Session:\n{}",
        session.app_name.as_deref().unwrap_or("an app"),
        session.text
    )
}

pub fn model_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "activity": {"type": "string", "enum": ["building","fixing","designing","planning","researching","writing","communicating","setup","reviewing","other"]},
            "subject": {"type": "string"},
            "goal": {"type": "string"},
            "friction": {"type": ["string", "null"]},
            "friction_type": {"type": ["string", "null"], "enum": ["bugs","layout","performance","tooling","ai_quality","integration","direction","other", null]},
            "decision": {"type": ["string", "null"]},
            "open_question": {"type": ["string", "null"]}
        },
        "required": ["activity","subject","goal","friction","friction_type","decision","open_question"]
    })
}

#[derive(Deserialize)]
struct Payload {
    activity: String,
    subject: Option<String>,
    goal: Option<String>,
    friction: Option<String>,
    friction_type: Option<String>,
    decision: Option<String>,
    open_question: Option<String>,
}

/// Decode a model reply; `None` when it isn't the JSON we asked for.
pub fn decode_model_reply(reply: &str, model: &str) -> Option<SessionAnalysis> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end < start {
        return None;
    }
    let payload: Payload = serde_json::from_str(&reply[start..=end]).ok()?;
    let clean = |v: Option<String>| {
        v.map(|s| {
            s.trim()
                .trim_matches(|c: char| c == '.' || c.is_whitespace())
                .to_string()
        })
        .filter(|s| {
            !s.is_empty() && !s.eq_ignore_ascii_case("null") && !s.eq_ignore_ascii_case("none")
        })
        .map(|s| clipped(&s, 120))
    };
    let friction = clean(payload.friction);
    Some(SessionAnalysis {
        activity: WorkActivity::parse(&payload.activity),
        subject: clean(payload.subject),
        goal: clean(payload.goal),
        friction_type: friction.as_ref().map(|_| {
            payload
                .friction_type
                .as_deref()
                .and_then(FrictionType::parse)
                .unwrap_or(FrictionType::Other)
        }),
        friction,
        decision: clean(payload.decision),
        open_question: clean(payload.open_question),
        model: Some(model.to_string()),
    })
}

/// Read one session with a local model.
pub fn analyze_with_model(
    client: &dyn LlmClient,
    model: &str,
    session: &WorkSession,
) -> Result<Option<SessionAnalysis>> {
    let opts = ChatOptions {
        temperature: Some(0.1),
        think: Some(false),
        json_schema: Some(model_schema()),
        max_tokens: Some(240),
        num_ctx: Some(4096),
        timeout_ms: Some(90_000),
        ..ChatOptions::model(model)
    };
    let reply = client.chat(&[ChatMessage::user(model_prompt(session))], &opts, None)?;
    Ok(decode_model_reply(&reply.content, model))
}

// ---- Report --------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnalyzedSession {
    pub session: WorkSession,
    pub analysis: SessionAnalysis,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivityShare {
    pub activity: WorkActivity,
    pub title: String,
    pub sessions: usize,
    pub share: f64,
    pub previous_share: Option<f64>,
    /// Change in percentage points against the previous period.
    pub change: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionNote {
    pub id: String,
    pub text: String,
    pub date: i64,
    pub app_name: Option<String>,
    pub project: Option<String>,
    pub is_estimate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrictionGroup {
    #[serde(rename = "type")]
    pub kind: FrictionType,
    pub title: String,
    pub sessions: usize,
    pub examples: Vec<SessionNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectBreakdown {
    pub id: String,
    pub title: String,
    pub sessions: usize,
    pub last_active: i64,
    pub activities: Vec<ActivityShare>,
    pub friction_sessions: usize,
    pub top_friction: Option<FrictionType>,
    pub top_friction_title: Option<String>,
    pub latest_goal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivityReport {
    pub range: ConceptTimeRange,
    pub session_count: usize,
    pub day_count: usize,
    pub model_analyzed_count: usize,
    pub headline: Vec<String>,
    pub activities: Vec<ActivityShare>,
    pub friction_rate: f64,
    pub frictions: Vec<FrictionGroup>,
    pub projects: Vec<ProjectBreakdown>,
    pub decisions: Vec<SessionNote>,
    pub open_questions: Vec<SessionNote>,
}

impl ActivityReport {
    pub fn empty(range: ConceptTimeRange) -> Self {
        Self {
            range,
            session_count: 0,
            day_count: 0,
            model_analyzed_count: 0,
            headline: Vec::new(),
            activities: Vec::new(),
            friction_rate: 0.0,
            frictions: Vec::new(),
            projects: Vec::new(),
            decisions: Vec::new(),
            open_questions: Vec::new(),
        }
    }
}

/// A named thing the concept graph knows about.
#[derive(Debug, Clone)]
pub struct ProjectCandidate {
    pub key: String,
    pub title: String,
    pub salience: f64,
}

fn read_by_model(items: &[&AnalyzedSession]) -> Option<bool> {
    let share = items.iter().filter(|i| !i.analysis.is_estimate()).count() as f64
        / items.len().max(1) as f64;
    if share >= 0.8 {
        Some(true)
    } else if share <= 0.2 {
        Some(false)
    } else {
        None
    }
}

pub fn activity_shares(
    items: &[&AnalyzedSession],
    previous: Option<&[&AnalyzedSession]>,
) -> Vec<ActivityShare> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut counts: BTreeMap<WorkActivity, usize> = BTreeMap::new();
    for i in items {
        *counts.entry(i.analysis.activity).or_default() += 1;
    }
    let prev_counts = previous.map(|p| {
        let mut m: HashMap<WorkActivity, usize> = HashMap::new();
        for i in p {
            *m.entry(i.analysis.activity).or_default() += 1;
        }
        (m, p.len())
    });
    let mut out: Vec<ActivityShare> = counts
        .into_iter()
        .map(|(activity, count)| {
            let share = count as f64 / items.len() as f64;
            let previous_share = prev_counts.as_ref().map(|(m, total)| {
                m.get(&activity).copied().unwrap_or(0) as f64 / (*total).max(1) as f64
            });
            ActivityShare {
                activity,
                title: activity.title().into(),
                sessions: count,
                share,
                previous_share,
                change: previous_share.map(|p| ((share - p) * 100.0).round() as i64),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.activity.raw().cmp(b.activity.raw()))
    });
    out
}

fn note(item: &AnalyzedSession, text: &str, field: &str, project: Option<String>) -> SessionNote {
    SessionNote {
        id: format!("{}-{field}", item.session.key),
        text: text.into(),
        date: item.session.end,
        app_name: item.session.app_name.clone(),
        project,
        is_estimate: item.analysis.is_estimate(),
    }
}

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round() as i64)
}

/// Build the report for `range` (Mac `ActivityReportBuilder.build`).
pub fn build_report(
    sessions: &[AnalyzedSession],
    range: ConceptTimeRange,
    catalog: &[ProjectCandidate],
    aliases: &HashMap<String, String>,
    clock: Clock,
) -> ActivityReport {
    let now = clock.now;
    let start = range.days().map(|d| now - d * 86_400);
    let current: Vec<&AnalyzedSession> = sessions
        .iter()
        .filter(|s| start.map(|st| s.session.end >= st).unwrap_or(true))
        .collect();
    if current.is_empty() {
        return ActivityReport::empty(range);
    }
    let previous: Vec<&AnalyzedSession> = match (range.days(), start) {
        (Some(days), Some(st)) => {
            let ps = st - days * 86_400;
            sessions
                .iter()
                .filter(|s| s.session.end >= ps && s.session.end < st)
                .collect()
        }
        _ => Vec::new(),
    };
    let by_key: HashMap<&str, &ProjectCandidate> =
        catalog.iter().map(|c| (c.key.as_str(), c)).collect();
    let project_for = |item: &AnalyzedSession| -> Option<&ProjectCandidate> {
        let resolve = |k: &str| -> Option<&ProjectCandidate> {
            let k = aliases.get(k).map(String::as_str).unwrap_or(k);
            by_key.get(k).copied()
        };
        let best = item
            .session
            .concept_keys
            .iter()
            .filter_map(|k| resolve(k))
            .max_by(|a, b| a.salience.total_cmp(&b.salience));
        if best.is_some() {
            return best;
        }
        item.analysis
            .subject
            .as_deref()
            .and_then(|s| resolve(&normalized_key(s, ConceptKind::Product)))
    };

    let comparable = previous.len() >= 10 && read_by_model(&current) == read_by_model(&previous);
    let activities = activity_shares(&current, comparable.then_some(previous.as_slice()));
    let with_friction: Vec<&AnalyzedSession> = current
        .iter()
        .copied()
        .filter(|s| s.analysis.friction.is_some())
        .collect();
    let friction_rate = with_friction.len() as f64 / current.len() as f64;

    // Friction groups.
    let mut grouped: BTreeMap<FrictionType, Vec<&AnalyzedSession>> = BTreeMap::new();
    for s in &with_friction {
        grouped
            .entry(s.analysis.friction_type.unwrap_or(FrictionType::Other))
            .or_default()
            .push(s);
    }
    let mut frictions: Vec<FrictionGroup> = grouped
        .into_iter()
        .map(|(kind, mut members)| {
            members.sort_by_key(|a| std::cmp::Reverse(a.session.end));
            FrictionGroup {
                kind,
                title: kind.title().into(),
                sessions: members.len(),
                examples: members
                    .iter()
                    .take(3)
                    .map(|m| {
                        note(
                            m,
                            m.analysis.friction.as_deref().unwrap_or(""),
                            "friction",
                            project_for(m).map(|p| p.title.clone()),
                        )
                    })
                    .collect(),
            }
        })
        .collect();
    frictions.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.kind.raw().cmp(b.kind.raw()))
    });

    // Projects.
    let mut by_project: BTreeMap<String, (&ProjectCandidate, Vec<&AnalyzedSession>)> =
        BTreeMap::new();
    for s in &current {
        if let Some(p) = project_for(s) {
            by_project
                .entry(p.key.clone())
                .or_insert((p, Vec::new()))
                .1
                .push(s);
        }
    }
    let mut projects: Vec<ProjectBreakdown> = by_project
        .into_values()
        .filter(|(_, m)| m.len() >= 2)
        .map(|(p, mut members)| {
            members.sort_by_key(|a| std::cmp::Reverse(a.session.end));
            let friction_items: Vec<&&AnalyzedSession> = members
                .iter()
                .filter(|m| m.analysis.friction.is_some())
                .collect();
            let mut fcounts: BTreeMap<FrictionType, usize> = BTreeMap::new();
            for f in &friction_items {
                *fcounts
                    .entry(f.analysis.friction_type.unwrap_or(FrictionType::Other))
                    .or_default() += 1;
            }
            let top = fcounts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.raw().cmp(a.0.raw())))
                .map(|(k, _)| *k);
            ProjectBreakdown {
                id: p.key.clone(),
                title: p.title.clone(),
                sessions: members.len(),
                last_active: members.first().map(|m| m.session.end).unwrap_or(now),
                activities: activity_shares(&members, None)
                    .into_iter()
                    .take(3)
                    .collect(),
                friction_sessions: friction_items.len(),
                top_friction: top,
                top_friction_title: top.map(|t| t.title().to_string()),
                latest_goal: members.iter().find_map(|m| m.analysis.goal.clone()),
            }
        })
        .collect();
    projects.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.title.cmp(&b.title))
    });
    projects.truncate(8);

    let mut recent_first = current.clone();
    recent_first.sort_by_key(|a| std::cmp::Reverse(a.session.end));
    // The same sentence said in several sessions is listed once (newest).
    let mut seen_decisions: HashSet<String> = HashSet::new();
    let mut seen_questions: HashSet<String> = HashSet::new();
    let decisions: Vec<SessionNote> = recent_first
        .iter()
        .filter_map(|s| {
            s.analysis
                .decision
                .as_deref()
                .map(|d| note(s, d, "decision", project_for(s).map(|p| p.title.clone())))
        })
        .filter(|n| seen_decisions.insert(n.text.to_lowercase()))
        .take(8)
        .collect();
    let open_questions: Vec<SessionNote> = recent_first
        .iter()
        .filter_map(|s| {
            s.analysis
                .open_question
                .as_deref()
                .map(|q| note(s, q, "question", project_for(s).map(|p| p.title.clone())))
        })
        .filter(|n| seen_questions.insert(n.text.to_lowercase()))
        .take(8)
        .collect();
    let days: HashSet<i64> = current.iter().map(|s| clock.day(s.session.end)).collect();

    let mut report = ActivityReport {
        range,
        session_count: current.len(),
        day_count: days.len(),
        model_analyzed_count: current.iter().filter(|s| !s.analysis.is_estimate()).count(),
        headline: Vec::new(),
        activities,
        friction_rate,
        frictions,
        projects,
        decisions,
        open_questions,
    };
    report.headline = headline(&report);
    report
}

/// A few plain sentences that say what the numbers mean.
pub fn headline(r: &ActivityReport) -> Vec<String> {
    let mut lines = Vec::new();
    let period = if r.range == ConceptTimeRange::All {
        "Overall".to_string()
    } else {
        format!("In the last {}", r.range.title())
    };
    lines.push(format!(
        "{period}, you dictated through {} working session{} on {} day{}.",
        r.session_count,
        if r.session_count == 1 { "" } else { "s" },
        r.day_count,
        if r.day_count == 1 { "" } else { "s" }
    ));
    let meaningful: Vec<&ActivityShare> = r
        .activities
        .iter()
        .filter(|a| a.activity != WorkActivity::Other)
        .collect();
    if let Some(first) = meaningful.first() {
        match meaningful.get(1) {
            Some(second) if second.share >= 0.12 => lines.push(format!(
                "Most of it was {} ({}), then {} ({}).",
                first.activity.phrase(),
                percent(first.share),
                second.activity.phrase(),
                percent(second.share)
            )),
            _ => lines.push(format!(
                "Most of it was {} ({}).",
                first.activity.phrase(),
                percent(first.share)
            )),
        }
    }
    if let Some((share, change)) = r
        .activities
        .iter()
        .filter_map(|a| a.change.map(|c| (a, c)))
        .filter(|(_, c)| c.abs() >= 8)
        .max_by_key(|(_, c)| c.abs())
    {
        lines.push(format!(
            "{} is {} {} points from the {} before.",
            share.activity.title(),
            if change > 0 { "up" } else { "down" },
            change.abs(),
            r.range.title()
        ));
    }
    if let Some(top) = r.projects.first() {
        match r.projects.get(1) {
            Some(next) => lines.push(format!(
                "{} took the most attention ({} sessions), then {} ({}).",
                top.title, top.sessions, next.title, next.sessions
            )),
            None => lines.push(format!(
                "{} took the most attention ({} sessions).",
                top.title, top.sessions
            )),
        }
    }
    if r.friction_rate >= 0.05 {
        if let Some(f) = r.frictions.first() {
            lines.push(format!(
                "Something got in the way in {} of sessions, most often {}.",
                percent(r.friction_rate),
                f.kind.title().to_lowercase()
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    fn src(id: i64, at: i64, app: &str, text: &str) -> ConceptSource {
        ConceptSource {
            id,
            text: text.into(),
            created_at: at,
            app_name: Some(app.into()),
        }
    }

    #[test]
    fn sessions_follow_gap_rule_and_key_changes() {
        let sources = vec![
            src(1, 0, "Code", "Fix the login bug please"),
            src(2, 600, "Code", "The login bug is still broken."),
            src(3, 700, "Mail", "Reply to Sam about the invoice"),
            src(4, 600 + 31 * 60, "Code", "New session here"),
        ];
        let s = sessions(&sources, &HashMap::new());
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].dictation_ids, vec![1, 2]);
        assert_eq!(s[0].start, 0);
        assert_eq!(s[0].end, 600);
        let mut edited = sources.clone();
        edited[1].text = "The login bug is fixed now.".into();
        let s2 = sessions(&edited, &HashMap::new());
        assert_ne!(s[0].key, s2[0].key);
        assert_eq!(s[1].key, s2[1].key);
        let long = "word ".repeat(2000);
        let c = condensed(&[&long]);
        assert!(c.contains("[…]"));
        assert!(c.chars().count() < 3700);
    }

    #[test]
    fn heuristics_read_wording() {
        let s = WorkSession {
            key: "k".into(),
            dictation_ids: vec![1],
            start: 0,
            end: 0,
            app_name: Some("Code".into()),
            text: "The build is broken again with an error in the terminal. \
                   Let's go with the older toolchain. Why does cargo keep failing on Windows?"
                .into(),
            concept_keys: vec![],
        };
        let a = heuristic_analysis(&s);
        assert_eq!(a.activity, WorkActivity::Fixing);
        assert_eq!(a.friction_type, Some(FrictionType::Tooling));
        assert!(a.decision.as_deref().unwrap().starts_with("Let's go with"));
        assert!(a.open_question.as_deref().unwrap().ends_with('?'));
        assert!(a.is_estimate());
    }

    #[test]
    fn model_reply_decodes_and_cleans() {
        let a = decode_model_reply(
            r#"sure {"activity":"designing","subject":"Orttaai","goal":"Tighten card spacing.","friction":"null","friction_type":"layout","decision":"Use 8px radius","open_question":null}"#,
            "gemma",
        )
        .unwrap();
        assert_eq!(a.activity, WorkActivity::Designing);
        assert_eq!(a.goal.as_deref(), Some("Tighten card spacing"));
        assert_eq!(a.friction, None);
        assert_eq!(a.friction_type, None);
        assert_eq!(a.model.as_deref(), Some("gemma"));
        assert!(decode_model_reply("nope", "m").is_none());
        let mock = MockLlm::fixed(
            r#"{"activity":"fixing","subject":"x","goal":"y","friction":"Crash on launch","friction_type":"weird","decision":null,"open_question":null}"#,
        );
        let s = WorkSession {
            key: "k".into(),
            dictation_ids: vec![],
            start: 0,
            end: 0,
            app_name: None,
            text: "t".into(),
            concept_keys: vec![],
        };
        let r = analyze_with_model(&mock, "m", &s).unwrap().unwrap();
        assert_eq!(r.friction_type, Some(FrictionType::Other));
    }

    #[test]
    fn report_counts_projects_frictions_and_headline() {
        let now = 40 * 86_400;
        let mk = |i: i64, activity: WorkActivity, friction: bool, project: &str| AnalyzedSession {
            session: WorkSession {
                key: format!("s{i}"),
                dictation_ids: vec![i],
                start: now - i * 86_400,
                end: now - i * 86_400,
                app_name: Some("Code".into()),
                text: String::new(),
                concept_keys: vec![project.into()],
            },
            analysis: SessionAnalysis {
                activity,
                subject: None,
                goal: Some(format!("goal {i}")),
                friction: friction.then(|| "It crashed".to_string()),
                friction_type: friction.then_some(FrictionType::Bugs),
                decision: (i == 1).then(|| "Use SQLite".to_string()),
                open_question: (i == 2).then(|| "Should we ship Friday?".to_string()),
                model: None,
            },
        };
        let sessions = vec![
            mk(1, WorkActivity::Fixing, true, "orttaai"),
            mk(2, WorkActivity::Fixing, false, "orttaai"),
            mk(3, WorkActivity::Building, false, "orttaai"),
            mk(4, WorkActivity::Designing, false, "atlas"),
            mk(5, WorkActivity::Designing, true, "atlas"),
        ];
        let catalog = vec![
            ProjectCandidate {
                key: "orttaai".into(),
                title: "Orttaai".into(),
                salience: 5.0,
            },
            ProjectCandidate {
                key: "atlas".into(),
                title: "Atlas".into(),
                salience: 3.0,
            },
        ];
        let r = build_report(
            &sessions,
            ConceptTimeRange::Month,
            &catalog,
            &HashMap::new(),
            Clock::utc(now),
        );
        assert_eq!(r.session_count, 5);
        assert_eq!(r.day_count, 5);
        assert_eq!(r.activities[0].activity, WorkActivity::Designing);
        assert!((r.friction_rate - 0.4).abs() < 1e-9);
        assert_eq!(r.frictions[0].sessions, 2);
        assert_eq!(r.projects[0].title, "Orttaai");
        assert_eq!(r.projects[0].sessions, 3);
        assert_eq!(r.projects[0].latest_goal.as_deref(), Some("goal 1"));
        assert_eq!(r.decisions[0].project.as_deref(), Some("Orttaai"));
        assert_eq!(r.open_questions.len(), 1);
        assert!(r.headline[0]
            .starts_with("In the last 30 days, you dictated through 5 working sessions"));
        assert!(r
            .headline
            .iter()
            .any(|l| l.contains("took the most attention")));
        assert!(r
            .headline
            .iter()
            .any(|l| l.contains("Something got in the way in 40%")));
        assert_eq!(
            build_report(
                &[],
                ConceptTimeRange::Week,
                &[],
                &HashMap::new(),
                Clock::utc(now)
            ),
            ActivityReport::empty(ConceptTimeRange::Week)
        );
    }
}
