//! Writing insights and tone-of-voice analysis (Mac `WritingInsightsService`,
//! `HeuristicWritingInsightAnalyzer`, `OllamaWritingInsightAnalyzer`,
//! `ToneOfVoiceService`). A local LLM writes JSON reports; a deterministic
//! analysis of the same history stands in whenever no model is configured or
//! the model fails. Snapshots persist in `writing_insight_snapshot` (kind
//! `writing` / `tone`) with pinning and history.

use crate::chat::clock;
use crate::error::Result;
use crate::llm::{
    error_text, extract_json_objects, provider_display_name, ChatOptions, GenerateRequest,
    LlmClient,
};
use crate::settings::Settings;
use crate::store::{InsightSnapshotRow, Store, TranscriptionRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};

pub const KIND_WRITING: &str = "writing";
pub const KIND_TONE: &str = "tone";

// ============================================================================
// Writing insights — models
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeRange {
    Days7,
    Days14,
    #[default]
    Days30,
    Days90,
    All,
}

impl TimeRange {
    pub fn title(self) -> &'static str {
        match self {
            TimeRange::Days7 => "7d",
            TimeRange::Days14 => "14d",
            TimeRange::Days30 => "30d",
            TimeRange::Days90 => "90d",
            TimeRange::All => "All",
        }
    }

    /// Start of the window, or `None` for all time.
    pub fn start(self, now: i64) -> Option<i64> {
        let days = match self {
            TimeRange::Days7 => 7,
            TimeRange::Days14 => 14,
            TimeRange::Days30 => 30,
            TimeRange::Days90 => 90,
            TimeRange::All => return None,
        };
        Some(now - days * 86_400)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GenerationMode {
    #[default]
    Balanced,
    Deep,
}

impl GenerationMode {
    pub fn history_limit(self) -> usize {
        match self {
            GenerationMode::Balanced => 600,
            GenerationMode::Deep => 2_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppFilterMode {
    #[default]
    AllApps,
    IncludeOnly,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WritingInsightsRequest {
    pub time_range: TimeRange,
    pub generation_mode: GenerationMode,
    pub app_filter_mode: AppFilterMode,
    pub selected_apps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightSignal {
    pub label: String,
    pub value: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightPattern {
    pub title: String,
    pub detail: String,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    Dictionary,
    Snippet,
}

/// A suggested dictionary entry or snippet ("Add to Dictionary").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightRecommendation {
    pub kind: RecommendationKind,
    pub source: String,
    pub target: String,
    pub rationale: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritingInsightSnapshot {
    pub generated_at: i64,
    pub sample_count: usize,
    pub analyzer_name: String,
    pub used_fallback: bool,
    pub request: WritingInsightsRequest,
    pub summary: String,
    pub signals: Vec<InsightSignal>,
    pub patterns: Vec<InsightPattern>,
    pub strengths: Vec<String>,
    pub opportunities: Vec<String>,
    #[serde(default)]
    pub recommendations: Vec<InsightRecommendation>,
}

/// What a generation produced (Mac `WritingInsightsRunResult`).
#[derive(Debug, Clone, Serialize)]
pub struct WritingInsightsRun {
    pub snapshot: Option<WritingInsightSnapshot>,
    pub snapshot_id: Option<i64>,
    pub sample_count: usize,
    pub analyzer_name: String,
    pub used_fallback: bool,
    pub error: Option<String>,
    pub persistence_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightHistoryItem {
    pub id: i64,
    pub is_pinned: bool,
    pub snapshot: WritingInsightSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightsComparison {
    pub older_id: i64,
    pub newer_id: i64,
    pub headline: String,
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    Fresh,
    Aging,
    Stale,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightFreshness {
    pub new_session_count: i64,
    pub latest_session_at: Option<i64>,
    pub status: FreshnessStatus,
    pub should_auto_refresh: bool,
}

/// The analysed part of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct InsightPayload {
    pub summary: String,
    pub signals: Vec<InsightSignal>,
    pub patterns: Vec<InsightPattern>,
    pub strengths: Vec<String>,
    pub opportunities: Vec<String>,
}

// ============================================================================
// Heuristic analyzer (deterministic fallback)
// ============================================================================

pub const HEURISTIC_ANALYZER: &str = "Heuristic Analyzer";
pub const QUICK_LOOK: &str = "Quick look";

fn app_name(app: Option<&str>) -> String {
    app.map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("Unknown App")
        .to_string()
}

/// Unicode punctuation (Foundation `.punctuationCharacters`): unlike
/// `is_ascii_punctuation`, symbols such as `< > $ + = ^ | ~` are not included.
fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        "!\"#%&'()*,-./:;?@[\\]_{}".contains(c)
    } else {
        matches!(
            c,
            '\u{2010}'..='\u{2027}' | '\u{2030}'..='\u{205E}' | '\u{3001}'..='\u{3003}' | '¡' | '¿' | '«' | '»' | '§' | '¶'
        )
    }
}

fn time_bucket(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Morning",
        12..=16 => "Afternoon",
        17..=21 => "Evening",
        _ => "Night",
    }
}

fn opening_phrase(text: &str) -> Option<String> {
    let lower = text.to_lowercase().replace('\n', " ");
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let phrase = words[..words.len().min(3)].join(" ");
    let phrase = phrase
        .trim_matches(|c: char| is_punctuation(c) || c.is_whitespace())
        .to_string();
    (phrase.chars().count() >= 4).then_some(phrase)
}

/// Group-by max with a deterministic tie-break (first key alphabetically).
fn top(counts: &HashMap<String, usize>) -> Option<(&String, usize)> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(k, v)| (k, *v))
}

fn fmt_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Mac `HeuristicWritingInsightAnalyzer` over newest-first history.
pub fn heuristic_insights(
    history: &[TranscriptionRecord],
    utc_offset_secs: i64,
) -> Option<InsightPayload> {
    if history.is_empty() {
        return None;
    }
    let n = history.len();
    let total_words: usize = history
        .iter()
        .map(|r| r.text.split_whitespace().count())
        .sum();
    let total_ms: i64 = history.iter().map(|r| r.duration_ms.max(0)).sum();
    let avg_words = (total_words as f64 / n as f64).round() as usize;
    let avg_secs = (total_ms as f64 / (n as f64 * 1_000.0)).round() as i64;

    let mut apps: HashMap<String, usize> = HashMap::new();
    let mut buckets: HashMap<String, usize> = ["Morning", "Afternoon", "Evening", "Night"]
        .iter()
        .map(|b| (b.to_string(), 0))
        .collect();
    let mut openings: HashMap<String, usize> = HashMap::new();
    let mut fillers: HashMap<String, usize> = HashMap::new();
    for r in history {
        *apps.entry(app_name(r.app.as_deref())).or_default() += 1;
        *buckets
            .entry(time_bucket(clock::local_hour(r.created_at, utc_offset_secs)).to_string())
            .or_default() += 1;
        if let Some(o) = opening_phrase(&r.text) {
            *openings.entry(o).or_default() += 1;
        }
        let lowered = format!(" {} ", r.text.to_lowercase());
        for f in ["um", "uh", "like", "you know", "basically", "actually"] {
            if lowered.contains(&format!(" {f} ")) {
                *fillers.entry(f.to_string()).or_default() += 1;
            }
        }
    }
    let (top_app, top_app_n) = top(&apps)
        .map(|(k, v)| (k.clone(), v))
        .unwrap_or(("Unknown App".into(), 0));
    let (top_bucket, top_bucket_n) = top(&buckets)
        .map(|(k, v)| (k.clone(), v))
        .unwrap_or(("Morning".into(), 0));
    let top_opening = top(&openings).map(|(k, v)| (k.clone(), v));
    let top_filler = top(&fillers).map(|(k, v)| (k.clone(), v));

    let summary = format!(
        "You are most active in {} sessions, averaging {avg_words} words per dictation.\n{top_app} is your primary context ({top_app_n} of {n} sessions).",
        top_bucket.to_lowercase()
    );
    let mut patterns = vec![
        InsightPattern {
            title: "Primary writing context".into(),
            detail: format!("Most of your recent dictations happen in {top_app}."),
            evidence: Some(format!("{top_app_n} out of {n} sessions")),
        },
        InsightPattern {
            title: "Session length".into(),
            detail: format!(
                "You average {avg_words} words over about {} seconds per session.",
                avg_secs.max(1)
            ),
            evidence: Some(format!("{total_words} words across {n} sessions")),
        },
        InsightPattern {
            title: "Peak writing window".into(),
            detail: format!(
                "Your strongest activity is in the {}.",
                top_bucket.to_lowercase()
            ),
            evidence: Some(format!("{top_bucket_n} sessions")),
        },
    ];
    if let Some((o, c)) = top_opening.as_ref().filter(|(_, c)| *c > 1) {
        patterns.push(InsightPattern {
            title: "Frequent opening phrase".into(),
            detail: format!("You often start with \u{201C}{o}\u{201D}."),
            evidence: Some(format!("Appeared {c}x")),
        });
    }
    if let Some((f, c)) = top_filler.as_ref().filter(|(_, c)| *c > 1) {
        patterns.push(InsightPattern {
            title: "Filler habit".into(),
            detail: format!("You frequently use \u{201C}{f}\u{201D}."),
            evidence: Some(format!("Detected in {c}x transcripts")),
        });
    }
    let signals = vec![
        InsightSignal {
            label: "Sessions".into(),
            value: n.to_string(),
            detail: "Analyzed from recent history".into(),
        },
        InsightSignal {
            label: "Words".into(),
            value: fmt_thousands(total_words),
            detail: "Total dictated words".into(),
        },
        InsightSignal {
            label: "Avg/session".into(),
            value: format!("{avg_words} words"),
            detail: "Typical dictation length".into(),
        },
        InsightSignal {
            label: "Avg duration".into(),
            value: format!("{}s", avg_secs.max(1)),
            detail: "Estimated speaking time".into(),
        },
    ];
    let mut strengths = vec![
        "You have consistent dictation activity in one main writing context.".to_string(),
        "Your sessions are frequent enough to build personal writing patterns quickly.".to_string(),
    ];
    strengths.push(if avg_words >= 20 {
        "You sustain longer-form dictation, which helps idea development.".into()
    } else {
        "Your short sessions are efficient for quick capture workflows.".into()
    });
    let mut opportunities = Vec::new();
    if let Some((f, c)) = &top_filler {
        if *c > 2 {
            opportunities.push(format!(
                "Reduce \u{201C}{f}\u{201D} in final drafts to improve clarity."
            ));
        }
    }
    if top_opening.as_ref().is_some_and(|(_, c)| *c > 3) {
        opportunities.push("Vary your opening line to avoid repetitive starts.".into());
    }
    if opportunities.is_empty() {
        opportunities = vec![
            "Create snippets for your most repeated openings to save time.".into(),
            "Review history weekly to spot recurring phrasing you want to refine.".into(),
        ];
    }
    patterns.truncate(6);
    strengths.truncate(4);
    opportunities.truncate(4);
    Some(InsightPayload {
        summary,
        signals,
        patterns,
        strengths,
        opportunities,
    })
}

// ============================================================================
// LLM analyzer
// ============================================================================

/// Echoed prompt placeholders are never shown (Mac `InsightReplyCleaner`).
pub fn is_placeholder(text: &str) -> bool {
    let n = text
        .trim_matches(|c: char| c.is_whitespace() || is_punctuation(c))
        .to_lowercase();
    [
        "short bullet",
        "short note",
        "pattern title",
        "what this means",
        "proof",
        "one short paragraph",
        "string",
        "bullet",
        "note",
        "title",
        "detail",
        "evidence",
    ]
    .contains(&n.as_str())
        || (n.starts_with('<') && n.ends_with('>'))
}

/// The structured-output schema for writing insights.
pub fn writing_insights_schema() -> serde_json::Value {
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "signals": { "type": "array", "items": {
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "value": { "type": "string" },
                    "detail": { "type": "string" }
                },
                "required": ["label", "value", "detail"],
                "additionalProperties": false
            }},
            "patterns": { "type": "array", "items": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "detail": { "type": "string" },
                    "evidence": { "type": "string" }
                },
                "required": ["title", "detail", "evidence"],
                "additionalProperties": false
            }},
            "strengths": strings,
            "opportunities": strings
        },
        "required": ["summary", "signals", "patterns", "strengths", "opportunities"],
        "additionalProperties": false
    })
}

/// History lines within the prompt budget (`max(24 000, ctx × 3)` chars).
pub fn insight_history_lines(history: &[TranscriptionRecord], context_tokens: u32) -> Vec<String> {
    let mut remaining = (context_tokens as i64 * 3).max(24_000);
    let mut lines = Vec::new();
    for r in history {
        let text = r.text.trim().replace(['\n', '\r'], " ");
        if text.is_empty() {
            continue;
        }
        let prefix = format!("{}: ", app_name(r.app.as_deref()));
        let available = remaining - prefix.chars().count() as i64 - 12;
        if available < 80 {
            break;
        }
        let len = text.chars().count() as i64;
        let budget = len.min(available);
        let included = if len > budget {
            format!(
                "{}...",
                text.chars()
                    .take((budget - 3).max(0) as usize)
                    .collect::<String>()
            )
        } else {
            text
        };
        let line = format!("{prefix}{included}");
        remaining -= line.chars().count() as i64 + 8;
        lines.push(line);
    }
    lines
}

/// The Mac prompt for the local-LLM analyzer.
pub fn writing_insights_prompt(lines: &[String]) -> String {
    let history = lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("[{}] {l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Analyze this user's dictation history and produce concise speaking and writing insights.

Return ONLY valid JSON with these keys:
- \"summary\": string, two or three sentences that begin with \"You\"
- \"signals\": array of {{\"label\": string, \"value\": string, \"detail\": string}}, each a number or fact you counted in the history
- \"patterns\": array of {{\"title\": string, \"detail\": string, \"evidence\": string}}, evidence quoting or pointing at the history
- \"strengths\": array of strings
- \"opportunities\": array of strings
Write every value yourself from the history. Never output placeholder text; leave an array empty rather than guess.

Rules:
- Speak to the user directly as \"you\"; never call them \"the user\".
- Keep summary under 55 words.
- Max 4 signals, 6 patterns, 4 strengths, 4 opportunities.
- Focus on speaking clarity, pacing cues, and repeated phrasing habits.
- Use practical, actionable language.

History:
{history}"
    )
}

#[derive(Deserialize)]
struct RawInsights {
    summary: String,
    signals: Vec<RawSignal>,
    patterns: Vec<RawPattern>,
    strengths: Vec<String>,
    opportunities: Vec<String>,
}

#[derive(Deserialize)]
struct RawSignal {
    label: String,
    value: String,
    detail: String,
}

#[derive(Deserialize)]
struct RawPattern {
    title: String,
    detail: String,
    evidence: Option<String>,
}

/// Decode the first JSON object in `reply` that has the report's shape, and
/// clean it (limits, placeholders, empty fields).
pub fn parse_insights_reply(reply: &str) -> Option<InsightPayload> {
    let raw = extract_json_objects(reply)
        .into_iter()
        .find_map(|c| serde_json::from_str::<RawInsights>(&c).ok())?;
    let summary = raw.summary.trim().to_string();
    let signals = raw
        .signals
        .into_iter()
        .take(4)
        .filter_map(|s| {
            let (label, value, detail) = (
                s.label.trim().to_string(),
                s.value.trim().to_string(),
                s.detail.trim().to_string(),
            );
            (!label.is_empty() && !value.is_empty() && !is_placeholder(&detail)).then_some(
                InsightSignal {
                    label,
                    value,
                    detail,
                },
            )
        })
        .collect();
    let patterns = raw
        .patterns
        .into_iter()
        .take(6)
        .filter_map(|p| {
            let (title, detail) = (p.title.trim().to_string(), p.detail.trim().to_string());
            if title.is_empty()
                || detail.is_empty()
                || is_placeholder(&title)
                || is_placeholder(&detail)
            {
                return None;
            }
            let evidence = p
                .evidence
                .map(|e| e.trim().to_string())
                .filter(|e| !e.is_empty() && !is_placeholder(e));
            Some(InsightPattern {
                title,
                detail,
                evidence,
            })
        })
        .collect();
    let clean = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !is_placeholder(s))
            .take(4)
            .collect()
    };
    Some(InsightPayload {
        summary: if summary.is_empty() || is_placeholder(&summary) {
            "Insights generated from your recent dictation sessions.".into()
        } else {
            summary
        },
        signals,
        patterns,
        strengths: clean(raw.strengths),
        opportunities: clean(raw.opportunities),
    })
}

/// Models the LLM analyzer tries in order: the insights model when insights
/// are on, then the polish model when polish is on (Mac
/// `localLLMInsightCandidateModels`).
pub fn insight_candidate_models(s: &Settings) -> Vec<String> {
    let norm = |m: &str| {
        let t = m.trim();
        if t.is_empty() || crate::llm::policy::is_cloud_model(t) {
            "gemma4:e2b".to_string()
        } else {
            t.to_string()
        }
    };
    let mut out: Vec<String> = Vec::new();
    if s.insights_enabled {
        out.push(norm(&s.insights_model));
    }
    if s.polish_enabled {
        out.push(norm(&s.polish_model));
    }
    let mut seen = HashSet::new();
    out.retain(|m| seen.insert(m.to_lowercase()));
    out
}

/// A model that hasn't answered in two minutes isn't coming back with better.
const INSIGHTS_TIMEOUT_MS: u64 = 120_000;

fn llm_insights(
    client: &dyn LlmClient,
    settings: &Settings,
    history: &[TranscriptionRecord],
) -> Option<InsightPayload> {
    let models = insight_candidate_models(settings);
    let lines = insight_history_lines(history, settings.insights_context_tokens);
    if models.is_empty() || lines.is_empty() {
        return None;
    }
    let prompt = writing_insights_prompt(&lines);
    models.iter().find_map(|model| {
        let opts = ChatOptions {
            temperature: Some(0.15),
            think: Some(settings.insights_thinking),
            json_schema: Some(writing_insights_schema()),
            max_tokens: Some(if settings.insights_thinking {
                4_000
            } else {
                1_500
            }),
            num_ctx: Some(settings.insights_context_tokens),
            timeout_ms: Some(INSIGHTS_TIMEOUT_MS),
            ..ChatOptions::model(model)
        };
        let reply = client
            .generate(&GenerateRequest::new(&prompt), &opts)
            .ok()?;
        parse_insights_reply(&reply)
    })
}

// ============================================================================
// Writing insights — service
// ============================================================================

/// History matching the request, newest first, capped by the mode's limit.
pub fn filtered_history(
    store: &Store,
    req: &WritingInsightsRequest,
    now: i64,
) -> Result<Vec<TranscriptionRecord>> {
    let selected: HashSet<String> = req
        .selected_apps
        .iter()
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty())
        .collect();
    let start = req.time_range.start(now);
    let filtered = store
        .recent(100_000)?
        .into_iter()
        .filter(|r| start.is_none_or(|s| r.created_at >= s))
        .filter(|r| {
            if req.app_filter_mode == AppFilterMode::AllApps || selected.is_empty() {
                return true;
            }
            let app = r.app.as_deref().unwrap_or("").trim().to_lowercase();
            match req.app_filter_mode {
                AppFilterMode::AllApps => true,
                AppFilterMode::IncludeOnly => selected.contains(&app),
                AppFilterMode::Exclude => !selected.contains(&app),
            }
        })
        .take(req.generation_mode.history_limit())
        .collect();
    Ok(filtered)
}

/// The analyzer the chain starts with ("Ollama (Local)" or the heuristic).
pub fn primary_analyzer_name(s: &Settings) -> String {
    if insight_candidate_models(s).is_empty() {
        HEURISTIC_ANALYZER.to_string()
    } else {
        format!("{} (Local)", provider_display_name(s.llm_provider))
    }
}

fn make_snapshot(
    payload: InsightPayload,
    analyzer: &str,
    used_fallback: bool,
    req: &WritingInsightsRequest,
    sample_count: usize,
    now: i64,
) -> WritingInsightSnapshot {
    WritingInsightSnapshot {
        generated_at: now,
        sample_count,
        analyzer_name: analyzer.to_string(),
        used_fallback,
        request: req.clone(),
        summary: payload.summary,
        signals: payload.signals,
        patterns: payload.patterns,
        strengths: payload.strengths,
        opportunities: payload.opportunities,
        recommendations: Vec::new(),
    }
}

/// An instant non-AI reading shown while the AI analysis runs. Not saved.
pub fn quick_insights(
    store: &Store,
    req: &WritingInsightsRequest,
    now: i64,
    utc_offset_secs: i64,
) -> Result<Option<WritingInsightSnapshot>> {
    let history = filtered_history(store, req, now)?;
    Ok(heuristic_insights(&history, utc_offset_secs)
        .map(|p| make_snapshot(p, QUICK_LOOK, false, req, history.len(), now)))
}

/// Generate (LLM → heuristic fallback) and persist a writing-insights snapshot.
pub fn generate_writing_insights(
    store: &Store,
    client: &dyn LlmClient,
    settings: &Settings,
    req: &WritingInsightsRequest,
    now: i64,
    utc_offset_secs: i64,
) -> WritingInsightsRun {
    let primary = primary_analyzer_name(settings);
    let history = match filtered_history(store, req, now) {
        Ok(h) => h,
        Err(_) => {
            return WritingInsightsRun {
                snapshot: None,
                snapshot_id: None,
                sample_count: 0,
                analyzer_name: primary,
                used_fallback: false,
                error: Some("Couldn't generate insights yet.".into()),
                persistence_warning: None,
            }
        }
    };
    if history.is_empty() {
        return WritingInsightsRun {
            snapshot: None,
            snapshot_id: None,
            sample_count: 0,
            analyzer_name: primary,
            used_fallback: false,
            error: None,
            persistence_warning: None,
        };
    }
    let uses_llm = primary != HEURISTIC_ANALYZER;
    let (payload, analyzer, used_fallback) = if uses_llm {
        match llm_insights(client, settings, &history) {
            Some(p) => (Some(p), primary.clone(), false),
            None => match heuristic_insights(&history, utc_offset_secs) {
                Some(p) => (Some(p), HEURISTIC_ANALYZER.to_string(), true),
                None => (None, primary.clone(), false),
            },
        }
    } else {
        (
            heuristic_insights(&history, utc_offset_secs),
            primary.clone(),
            false,
        )
    };
    let Some(payload) = payload else {
        return WritingInsightsRun {
            snapshot: None,
            snapshot_id: None,
            sample_count: history.len(),
            analyzer_name: analyzer,
            used_fallback,
            error: Some("Couldn't generate insights yet.".into()),
            persistence_warning: None,
        };
    };
    let snapshot = make_snapshot(payload, &analyzer, used_fallback, req, history.len(), now);
    let saved = serde_json::to_string(&snapshot).ok().and_then(|snap_json| {
        store
            .insert_insight_snapshot(
                KIND_WRITING,
                now,
                &analyzer,
                used_fallback,
                history.len() as i64,
                &serde_json::to_string(req).unwrap_or_else(|_| "{}".into()),
                &snap_json,
            )
            .ok()
    });
    WritingInsightsRun {
        persistence_warning: saved
            .is_none()
            .then(|| "Insights generated, but couldn't persist snapshot.".into()),
        snapshot: Some(snapshot),
        snapshot_id: saved,
        sample_count: history.len(),
        analyzer_name: analyzer,
        used_fallback,
        error: None,
    }
}

fn decode_writing(row: InsightSnapshotRow) -> Option<InsightHistoryItem> {
    let snapshot = serde_json::from_str(&row.snapshot_json).ok()?;
    Some(InsightHistoryItem {
        id: row.id,
        is_pinned: row.is_pinned,
        snapshot,
    })
}

pub fn latest_writing_insights(store: &Store) -> Result<Option<InsightHistoryItem>> {
    Ok(store
        .latest_insight_snapshot(KIND_WRITING)?
        .and_then(decode_writing))
}

pub fn writing_insights_history(store: &Store, limit: i64) -> Result<Vec<InsightHistoryItem>> {
    Ok(store
        .insight_snapshot_history(KIND_WRITING, limit)?
        .into_iter()
        .filter_map(decode_writing)
        .collect())
}

pub fn get_writing_insights(store: &Store, id: i64) -> Result<Option<InsightHistoryItem>> {
    Ok(store
        .get_insight_snapshot(id)?
        .filter(|r| r.kind == KIND_WRITING)
        .and_then(decode_writing))
}

/// Fresh / aging (6+ new sessions) / stale (14+); auto-refresh at 20+.
pub fn freshness(store: &Store, generated_at: i64) -> Result<InsightFreshness> {
    let n = store.count_transcriptions_since(generated_at)?;
    let status = if n >= 14 {
        FreshnessStatus::Stale
    } else if n >= 6 {
        FreshnessStatus::Aging
    } else {
        FreshnessStatus::Fresh
    };
    Ok(InsightFreshness {
        new_session_count: n,
        latest_session_at: store.latest_transcription_at()?,
        status,
        should_auto_refresh: n >= 20,
    })
}

/// Compare two snapshots (Mac `recomputeInsightsComparison`).
pub fn compare(
    a: &InsightHistoryItem,
    b: &InsightHistoryItem,
    utc_offset_secs: i64,
) -> InsightsComparison {
    let (older, newer) = if a.snapshot.generated_at <= b.snapshot.generated_at {
        (a, b)
    } else {
        (b, a)
    };
    let (o, n) = (&older.snapshot, &newer.snapshot);
    let delta = n.sample_count as i64 - o.sample_count as i64;
    let delta_text = match delta {
        0 => "no change".to_string(),
        d if d > 0 => format!("+{d}"),
        d => d.to_string(),
    };
    let mut bullets = vec![format!(
        "Sessions analyzed: {} -> {} ({delta_text}).",
        o.sample_count, n.sample_count
    )];
    let old_ops: HashSet<&String> = o.opportunities.iter().collect();
    let new_ops: HashSet<&String> = n.opportunities.iter().collect();
    let added = new_ops.difference(&old_ops).count();
    let resolved = old_ops.difference(&new_ops).count();
    if added > 0 || resolved > 0 {
        bullets.push(format!(
            "Opportunity changes: {added} added, {resolved} resolved."
        ));
    }
    let old_patterns: HashSet<&String> = o.patterns.iter().map(|p| &p.title).collect();
    let mut new_patterns: Vec<&String> = n
        .patterns
        .iter()
        .map(|p| &p.title)
        .filter(|t| !old_patterns.contains(t))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    new_patterns.sort();
    if !new_patterns.is_empty() {
        bullets.push(format!(
            "New patterns detected: {}.",
            new_patterns
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    bullets.truncate(4);
    InsightsComparison {
        older_id: older.id,
        newer_id: newer.id,
        headline: format!(
            "Comparing {} vs {}",
            clock::medium_date_time(o.generated_at, utc_offset_secs),
            clock::medium_date_time(n.generated_at, utc_offset_secs)
        ),
        bullets,
    }
}

/// "Add to Dictionary" / "Add as Snippet".
pub fn apply_recommendation(store: &Store, rec: &InsightRecommendation) -> Result<i64> {
    match rec.kind {
        RecommendationKind::Dictionary => store.upsert_dictionary(&rec.source, &rec.target, false),
        RecommendationKind::Snippet => store.upsert_snippet(&rec.source, &rec.target),
    }
}

// ============================================================================
// Tone of voice
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToneMetric {
    pub name: String,
    pub value: f64,
    pub label: String,
    pub detail: String,
}

/// Mac `ToneOfVoiceProfile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToneProfile {
    pub generated_at: i64,
    /// The model that refined it; empty for local metrics only.
    pub model: String,
    pub sample_count: usize,
    pub word_count: usize,
    pub sentence_count: usize,
    pub overall_score: i64,
    pub confidence: f64,
    pub summary: String,
    pub descriptors: Vec<String>,
    pub signature_phrases: Vec<String>,
    pub avoidances: Vec<String>,
    pub signature_approaches: Vec<String>,
    pub recommendations: Vec<String>,
    pub metrics: Vec<ToneMetric>,
    pub prompt_guide: String,
    pub sample_excerpts: Vec<String>,
}

impl ToneProfile {
    pub fn confidence_percent(&self) -> i64 {
        (self.confidence.clamp(0.0, 1.0) * 100.0).round() as i64
    }
}

/// What a tone analysis produced.
#[derive(Debug, Clone, Serialize)]
pub struct ToneRun {
    pub profile: ToneProfile,
    pub snapshot_id: Option<i64>,
    /// The model answered (its JSON may still have fallen back to metrics).
    pub used_llm: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToneHistoryItem {
    pub id: i64,
    pub is_pinned: bool,
    pub profile: ToneProfile,
}

const TONE_METRICS: [&str; 6] = [
    "Formality",
    "Warmth",
    "Directness",
    "Enthusiasm",
    "Complexity",
    "Conversation",
];
const CONTRACTIONS: &[&str] = &[
    "n't", "'re", "'ll", "'ve", "'d", "'m", " it's ", " i'm ", " don't ", " can't ",
];
const HEDGES: &[&str] = &[
    " maybe ",
    " perhaps ",
    " i think ",
    " sort of ",
    " kind of ",
    " probably ",
    " might ",
    " could ",
    " somewhat ",
];
const WARM: &[&str] = &[
    " thanks ",
    " thank you ",
    " appreciate ",
    " happy ",
    " love ",
    " great ",
    " glad ",
    " please ",
    " excited ",
];
const INCLUSIVE: &[&str] = &[" we ", " us ", " our ", " together "];

fn tone_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn tone_sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn rate(markers: &[&str], lower: &str, denom: usize) -> f64 {
    if denom == 0 {
        return 0.0;
    }
    markers
        .iter()
        .map(|m| lower.matches(m).count())
        .sum::<usize>() as f64
        / denom as f64
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

fn metric(name: &str, value: f64, low: &str, mid: &str, high: &str, detail: &str) -> ToneMetric {
    let label = if value < 0.38 {
        low
    } else if value > 0.64 {
        high
    } else {
        mid
    };
    ToneMetric {
        name: name.into(),
        value,
        label: label.into(),
        detail: detail.into(),
    }
}

fn signature_phrases(samples: &[String]) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "that", "this", "with", "from", "have", "has", "had", "are", "was",
        "were", "you", "your", "but", "not", "can", "will", "just",
    ];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in samples {
        let w = tone_words(s);
        if w.len() < 2 {
            continue;
        }
        for len in 2..=3 {
            if w.len() < len {
                continue;
            }
            for win in w.windows(len) {
                if win
                    .iter()
                    .any(|x| !STOP.contains(&x.as_str()) && x.chars().count() > 3)
                {
                    *counts.entry(win.join(" ")).or_default() += 1;
                }
            }
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().filter(|(_, c)| *c > 1).collect();
    v.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.chars().count().cmp(&a.0.chars().count()))
            .then(a.0.cmp(&b.0))
    });
    v.into_iter().take(8).map(|(k, _)| k).collect()
}

fn excerpt(text: &str, max: usize) -> String {
    let cleaned = text.replace(['\n', '\r'], " ").trim().to_string();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        format!(
            "{}...",
            cleaned
                .chars()
                .take(max.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn pct(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

/// Local-metrics profile (Mac `makeHeuristicProfile`).
pub fn heuristic_tone_profile(samples: &[String], model: &str, now: i64) -> ToneProfile {
    let combined = samples.join("\n\n");
    let words = tone_words(&combined);
    let sentences = tone_sentences(&combined);
    let lower = format!(" {} ", combined.to_lowercase());
    let wc = words.len();
    let sc = sentences.len();
    let unique = words.iter().collect::<HashSet<_>>().len() as f64 / wc.max(1) as f64;
    let avg_word_len = if wc == 0 {
        0.0
    } else {
        words.iter().map(|w| w.chars().count()).sum::<usize>() as f64 / wc as f64
    };
    let avg_sentence = wc as f64 / sc.max(1) as f64;
    let contraction = rate(CONTRACTIONS, &lower, wc);
    let hedge = rate(HEDGES, &lower, sc);
    let warm = rate(WARM, &lower, sc);
    let inclusive = rate(INCLUSIVE, &lower, wc);
    let punct = |p: char| {
        if sc == 0 {
            0.0
        } else {
            combined.matches(p).count() as f64 / sc as f64
        }
    };
    let question = punct('?');
    let exclamation = punct('!');

    let formality = clamp01(0.62 - contraction * 4.0 + (avg_word_len - 4.2) * 0.12);
    let warmth = clamp01(0.38 + warm * 2.2 + inclusive * 6.0);
    let directness = clamp01(0.74 - hedge * 1.8 + (avg_sentence / 180.0).min(0.16));
    let enthusiasm = clamp01(0.32 + exclamation * 1.8 + warm);
    let complexity =
        clamp01(0.28 + unique * 0.7 + (avg_sentence - 10.0) / 42.0 + (avg_word_len - 4.0) / 10.0);
    let conversational = clamp01(0.5 + contraction * 5.0 + question * 0.7 - formality * 0.2);

    let mut descriptors: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        if !descriptors.iter().any(|d| d == s) {
            descriptors.push(s.into());
        }
    };
    push(if formality > 0.64 {
        "professional"
    } else if formality < 0.36 {
        "casual"
    } else {
        "balanced"
    });
    push(if warmth > 0.62 { "warm" } else { "measured" });
    push(if directness > 0.62 {
        "direct"
    } else {
        "diplomatic"
    });
    if enthusiasm > 0.62 {
        push("energetic");
    }
    if complexity > 0.62 {
        push("layered");
    }
    if conversational > 0.62 {
        push("conversational");
    }
    descriptors.truncate(5);

    let phrases = signature_phrases(samples);
    let metrics = vec![
        metric(
            "Formality",
            formality,
            "Casual",
            "Balanced",
            "Formal",
            "Based on contractions, word choice, and sentence shape.",
        ),
        metric(
            "Warmth",
            warmth,
            "Reserved",
            "Neutral",
            "Warm",
            "Based on appreciation, inclusive language, and positive markers.",
        ),
        metric(
            "Directness",
            directness,
            "Diplomatic",
            "Balanced",
            "Direct",
            "Based on hedging, sentence focus, and assertive phrasing.",
        ),
        metric(
            "Enthusiasm",
            enthusiasm,
            "Measured",
            "Moderate",
            "Energetic",
            "Based on punctuation and energetic language.",
        ),
        metric(
            "Complexity",
            complexity,
            "Simple",
            "Clear",
            "Layered",
            "Based on vocabulary variety and average sentence length.",
        ),
        metric(
            "Conversation",
            conversational,
            "Composed",
            "Natural",
            "Conversational",
            "Based on contractions, questions, and spoken rhythm.",
        ),
    ];
    let overall = (metrics.iter().map(|m| m.value).sum::<f64>() / metrics.len() as f64 * 100.0)
        .round() as i64;
    let confidence = clamp01((wc as f64 / 650.0).min(0.92));
    let sentence_label = if avg_sentence < 12.0 {
        "short"
    } else if avg_sentence > 20.0 {
        "longer, more layered"
    } else {
        "moderate-length"
    };
    let summary = format!(
        "Your writing voice reads as {}. It tends to use {sentence_label} sentences with {}.",
        descriptors
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        if contraction > 0.015 {
            "natural contractions"
        } else {
            "more complete phrasing"
        }
    );

    let mut recommendations = vec![
        "Use the detected tone profile in ChatAI's My Tone mode for drafts that sound closer to your writing.".to_string(),
        if directness > 0.64 {
            "Keep generated drafts concise and action-oriented."
        } else {
            "Let drafts keep some nuance instead of forcing overly blunt phrasing."
        }
        .to_string(),
        if warmth > 0.62 {
            "Preserve friendly relational language when rewriting."
        } else {
            "Keep warmth intentional so generated text does not sound artificially cheerful."
        }
        .to_string(),
        if complexity > 0.62 {
            "Allow layered explanations, but ask ChatAI to tighten final delivery when needed."
        } else {
            "Prefer plain language and short transitions in generated content."
        }
        .to_string(),
    ];
    if formality < 0.36 {
        recommendations.push("Let contractions remain when the audience is informal.".into());
    } else if formality > 0.64 {
        recommendations.push("Avoid slang when generating professional content.".into());
    }
    if enthusiasm < 0.38 {
        recommendations.push("Use exclamation points sparingly.".into());
    }
    recommendations.truncate(6);

    let mut avoidances = Vec::new();
    if enthusiasm < 0.42 {
        avoidances.push("Avoid excessive exclamation points or hype.".to_string());
    }
    if formality > 0.64 {
        avoidances.push("Avoid slang and overly casual shortcuts.".into());
    }
    if directness > 0.64 {
        avoidances.push("Avoid excessive hedging or long caveats.".into());
    }
    if contraction < 0.006 {
        avoidances.push("Avoid forcing contractions when the context is formal.".into());
    }
    if avoidances.is_empty() {
        avoidances.push("Avoid overusing signature phrases; keep them natural.".into());
    }

    let mut approaches = vec![if avg_sentence > 20.0 {
        "Builds ideas in fuller sentences."
    } else {
        "Keeps sentences compact and easy to scan."
    }
    .to_string()];
    if question > 0.12 {
        approaches.push("Uses questions to frame or advance the point.".into());
    }
    if let Some(first) = phrases.first() {
        approaches.push(format!("Repeats phrases like \"{first}\" across samples."));
    }

    let metric_lines = metrics
        .iter()
        .map(|m| {
            format!(
                "- {}: {} ({}%). {}",
                m.name,
                m.label,
                pct(m.value),
                m.detail
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let phrase_line = if phrases.is_empty() {
        "- Do not force catchphrases; prioritize natural rhythm.".to_string()
    } else {
        format!(
            "- Signature phrases to consider lightly: {}.",
            phrases
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let contraction_line = if contraction > 0.015 {
        "- Use contractions naturally when context allows."
    } else {
        "- Prefer complete forms unless the user asks for a casual rewrite."
    };
    let sentence_line = if avg_sentence > 20.0 {
        "- Use fuller sentences and allow nuance."
    } else {
        "- Prefer concise sentences and clean transitions."
    };
    let prompt_guide = format!(
        "Write in the user's tone of voice.\nVoice descriptors: {}.\n{metric_lines}\n{sentence_line}\n{contraction_line}\n{phrase_line}\nSignature approaches:\n{}\nAvoid:\n{}\nKeep the content useful and natural. Do not imitate personal facts or exaggerate style markers.",
        descriptors.join(", "),
        approaches.iter().map(|a| format!("- {a}")).collect::<Vec<_>>().join("\n"),
        avoidances.iter().map(|a| format!("- {a}")).collect::<Vec<_>>().join("\n"),
    );

    ToneProfile {
        generated_at: now,
        model: model.to_string(),
        sample_count: samples.len(),
        word_count: wc,
        sentence_count: sc,
        overall_score: overall,
        confidence,
        summary,
        descriptors,
        signature_phrases: phrases,
        avoidances,
        signature_approaches: approaches,
        recommendations,
        metrics,
        prompt_guide,
        sample_excerpts: samples.iter().take(4).map(|s| excerpt(s, 180)).collect(),
    }
}

/// The Mac stylometry prompt, seeded with the local metrics.
pub fn tone_prompt(samples: &[String], baseline: &ToneProfile) -> String {
    let sample_text = samples
        .iter()
        .take(140)
        .enumerate()
        .map(|(i, s)| format!("[{}] {}", i + 1, excerpt(s, 900)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let metrics = baseline
        .metrics
        .iter()
        .map(|m| format!("{}={} {}%", m.name, m.label, pct(m.value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "You are a computational stylometry analyst. Analyze this user's writing and dictation samples to create a tone-of-voice profile that can guide a local writing assistant.

Return ONLY valid JSON with this structure:
{{
  \"summary\": \"one concise paragraph\",
  \"descriptors\": [\"word\", \"word\", \"word\"],
  \"metrics\": [
    {{\"name\": \"Formality\", \"value\": 0.0, \"label\": \"Casual|Balanced|Formal\", \"detail\": \"short note\"}},
    {{\"name\": \"Warmth\", \"value\": 0.0, \"label\": \"Reserved|Neutral|Warm\", \"detail\": \"short note\"}},
    {{\"name\": \"Directness\", \"value\": 0.0, \"label\": \"Diplomatic|Balanced|Direct\", \"detail\": \"short note\"}},
    {{\"name\": \"Enthusiasm\", \"value\": 0.0, \"label\": \"Measured|Moderate|Energetic\", \"detail\": \"short note\"}},
    {{\"name\": \"Complexity\", \"value\": 0.0, \"label\": \"Simple|Clear|Layered\", \"detail\": \"short note\"}},
    {{\"name\": \"Conversation\", \"value\": 0.0, \"label\": \"Composed|Natural|Conversational\", \"detail\": \"short note\"}}
  ],
  \"signaturePhrases\": [\"phrase\"],
  \"avoidances\": [\"Avoid ...\"],
  \"signatureApproaches\": [\"approach\"],
  \"recommendations\": [\"practical recommendation\"],
  \"promptGuide\": \"Instruction block for an AI assistant to write in this user's tone.\",
  \"confidence\": 0.0
}}

Rules:
- Score metric values from 0.0 to 1.0.
- Keep the summary under 70 words.
- Use the provided local metrics as a starting point, but refine with the samples.
- The promptGuide must be direct instructions, not analysis commentary.
- Do not invent personal facts.

Local metric baseline:
Summary: {}
Metrics: {metrics}
Signature phrases: {}

Writing samples:
{sample_text}",
        baseline.summary,
        baseline.signature_phrases.join(", ")
    )
}

/// Structured-output schema matching the tone prompt.
pub fn tone_schema() -> serde_json::Value {
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "descriptors": strings,
            "metrics": { "type": "array", "items": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "enum": TONE_METRICS },
                    "value": { "type": "number" },
                    "label": { "type": "string" },
                    "detail": { "type": "string" }
                },
                "required": ["name", "value", "label", "detail"],
                "additionalProperties": false
            }},
            "signaturePhrases": strings,
            "avoidances": strings,
            "signatureApproaches": strings,
            "recommendations": strings,
            "promptGuide": { "type": "string" },
            "confidence": { "type": "number" }
        },
        "required": ["summary", "descriptors", "metrics", "signaturePhrases", "avoidances",
                     "signatureApproaches", "recommendations", "promptGuide", "confidence"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTone {
    summary: Option<String>,
    descriptors: Option<Vec<String>>,
    metrics: Option<Vec<RawToneMetric>>,
    signature_phrases: Option<Vec<String>>,
    avoidances: Option<Vec<String>>,
    signature_approaches: Option<Vec<String>>,
    recommendations: Option<Vec<String>>,
    prompt_guide: Option<String>,
    confidence: Option<f64>,
}

#[derive(Deserialize)]
struct RawToneMetric {
    name: String,
    value: f64,
    label: String,
    detail: String,
}

fn strings_or(values: Option<Vec<String>>, fallback: &[String], limit: usize) -> Vec<String> {
    let v: Vec<String> = values
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(limit)
        .collect();
    if v.is_empty() {
        fallback.iter().take(limit).cloned().collect()
    } else {
        v
    }
}

/// Merge the model's JSON over the local-metrics profile (Mac `decodeAIProfile`).
pub fn parse_tone_reply(
    reply: &str,
    fallback: &ToneProfile,
    model: &str,
    now: i64,
) -> Option<ToneProfile> {
    let raw = extract_json_objects(reply)
        .into_iter()
        .find_map(|c| serde_json::from_str::<RawTone>(&c).ok())?;
    let metrics = match raw.metrics.filter(|m| !m.is_empty()) {
        None => fallback.metrics.clone(),
        Some(ms) => {
            let mut by_name: HashMap<String, ToneMetric> = HashMap::new();
            for m in ms {
                let name = m.name.trim().to_string();
                if !TONE_METRICS.contains(&name.as_str()) {
                    continue;
                }
                let label = m.label.trim().to_string();
                let fallback_label = fallback
                    .metrics
                    .iter()
                    .find(|f| f.name == name)
                    .map(|f| f.label.clone())
                    .unwrap_or_else(|| name.clone());
                by_name.insert(
                    name.clone(),
                    ToneMetric {
                        value: clamp01(m.value),
                        label: if label.is_empty() {
                            fallback_label
                        } else {
                            label
                        },
                        detail: m.detail.trim().to_string(),
                        name,
                    },
                );
            }
            TONE_METRICS
                .iter()
                .filter_map(|n| {
                    by_name
                        .remove(*n)
                        .or_else(|| fallback.metrics.iter().find(|f| f.name == *n).cloned())
                })
                .collect()
        }
    };
    let overall = (metrics.iter().map(|m| m.value).sum::<f64>() / metrics.len().max(1) as f64
        * 100.0)
        .round() as i64;
    let summary = raw
        .summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.summary.clone());
    let guide = raw
        .prompt_guide
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.prompt_guide.clone());
    Some(ToneProfile {
        generated_at: now,
        model: model.to_string(),
        sample_count: fallback.sample_count,
        word_count: fallback.word_count,
        sentence_count: fallback.sentence_count,
        overall_score: overall,
        confidence: clamp01(raw.confidence.unwrap_or(fallback.confidence)),
        summary,
        descriptors: strings_or(raw.descriptors, &fallback.descriptors, 5),
        signature_phrases: strings_or(raw.signature_phrases, &fallback.signature_phrases, 8),
        avoidances: strings_or(raw.avoidances, &fallback.avoidances, 6),
        signature_approaches: strings_or(
            raw.signature_approaches,
            &fallback.signature_approaches,
            6,
        ),
        recommendations: strings_or(raw.recommendations, &fallback.recommendations, 6),
        metrics,
        prompt_guide: guide,
        sample_excerpts: fallback.sample_excerpts.clone(),
    })
}

/// Default model for tone analysis (Mac: the insights model).
pub fn default_tone_model(s: &Settings) -> String {
    let m = s.insights_model.trim();
    if m.is_empty() || crate::llm::policy::is_cloud_model(m) {
        "gemma4:e2b".into()
    } else {
        m.to_string()
    }
}

/// Analyze the 800 latest dictations; `None` when there's no history. An
/// empty `model` uses local metrics only. The profile is saved as a `tone`
/// snapshot.
pub fn analyze_tone(
    store: &Store,
    client: &dyn LlmClient,
    settings: &Settings,
    model: &str,
    now: i64,
) -> Result<Option<ToneRun>> {
    let samples: Vec<String> = store
        .recent(800)?
        .into_iter()
        .map(|r| r.text.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if samples.is_empty() {
        return Ok(None);
    }
    let model = model.trim();
    let mut profile = heuristic_tone_profile(&samples, model, now);
    let mut used_llm = false;
    let mut error = None;
    if !model.is_empty() {
        let opts = ChatOptions {
            temperature: Some(0.12),
            think: Some(settings.insights_thinking),
            json_schema: Some(tone_schema()),
            max_tokens: Some(4_000),
            num_ctx: Some(settings.insights_context_tokens),
            timeout_ms: Some(600_000),
            ..ChatOptions::model(model)
        };
        match client.generate(
            &GenerateRequest::new(tone_prompt(&samples, &profile)),
            &opts,
        ) {
            Ok(reply) => {
                used_llm = true;
                if let Some(ai) = parse_tone_reply(&reply, &profile, model, now) {
                    profile = ai;
                }
            }
            Err(e) => {
                tracing::warn!("tone analysis failed: {}", error_text(&e));
                error = Some(format!(
                    "{} could not complete tone analysis, so Orttaai used local metrics.",
                    provider_display_name(settings.llm_provider)
                ));
            }
        }
    }
    if !used_llm {
        profile.model = String::new();
    }
    let snapshot_id = store
        .insert_insight_snapshot(
            KIND_TONE,
            now,
            if used_llm { model } else { "Local metrics" },
            !used_llm,
            samples.len() as i64,
            &json!({ "model": model }).to_string(),
            &serde_json::to_string(&profile)?,
        )
        .ok();
    Ok(Some(ToneRun {
        profile,
        snapshot_id,
        used_llm,
        error,
    }))
}

fn decode_tone(row: InsightSnapshotRow) -> Option<ToneHistoryItem> {
    Some(ToneHistoryItem {
        profile: serde_json::from_str(&row.snapshot_json).ok()?,
        id: row.id,
        is_pinned: row.is_pinned,
    })
}

/// The saved tone profile (used by Chat AI's My Tone mode).
pub fn latest_tone_profile(store: &Store) -> Option<ToneProfile> {
    store
        .latest_insight_snapshot(KIND_TONE)
        .ok()
        .flatten()
        .and_then(decode_tone)
        .map(|t| t.profile)
}

pub fn latest_tone(store: &Store) -> Result<Option<ToneHistoryItem>> {
    Ok(store
        .latest_insight_snapshot(KIND_TONE)?
        .and_then(decode_tone))
}

pub fn tone_history(store: &Store, limit: i64) -> Result<Vec<ToneHistoryItem>> {
    Ok(store
        .insight_snapshot_history(KIND_TONE, limit)?
        .into_iter()
        .filter_map(decode_tone)
        .collect())
}

pub fn get_tone(store: &Store, id: i64) -> Result<Option<ToneHistoryItem>> {
    Ok(store
        .get_insight_snapshot(id)?
        .filter(|r| r.kind == KIND_TONE)
        .and_then(decode_tone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    const NOW: i64 = 1_792_596_720; // 2026-10-21 15:32 UTC

    fn store_with(texts: &[(&str, &str, i64)]) -> Store {
        let s = Store::open_in_memory().unwrap();
        for (text, app, ago) in texts {
            let mut r = TranscriptionRecord::new(*text, Some(app.to_string()), 6_000, NOW - ago);
            r.duration_ms = 6_000;
            s.insert_transcription(&r).unwrap();
        }
        s
    }

    fn sample_store() -> Store {
        store_with(&[
            ("so um the release should ship on friday", "Slack", 3_600),
            (
                "so um the budget needs a second review before monday",
                "Slack",
                7_200,
            ),
            ("so um the deck goes to the client today", "Mail", 10_800),
            (
                "thanks so much for the quick turnaround, you know it helps",
                "Slack",
                14_400,
            ),
            ("old note from long ago", "Notes", 200 * 86_400),
        ])
    }

    #[test]
    fn heuristic_insights_match_mac_copy() {
        let s = sample_store();
        let req = WritingInsightsRequest::default();
        let history = filtered_history(&s, &req, NOW).unwrap();
        assert_eq!(history.len(), 4, "30-day window drops the old note");
        let p = heuristic_insights(&history, 0).unwrap();
        assert!(p
            .summary
            .starts_with("You are most active in afternoon sessions"));
        assert!(p
            .summary
            .contains("Slack is your primary context (3 of 4 sessions)."));
        assert_eq!(p.signals[0].value, "4");
        assert_eq!(p.signals[3].value, "6s");
        let titles: Vec<&str> = p.patterns.iter().map(|x| x.title.as_str()).collect();
        assert!(titles.contains(&"Frequent opening phrase"));
        assert!(titles.contains(&"Filler habit"));
        assert!(p.opportunities[0].contains("Reduce \u{201C}um\u{201D}"));
        assert_eq!(fmt_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn app_filters_and_ranges() {
        let s = sample_store();
        let only_mail = WritingInsightsRequest {
            app_filter_mode: AppFilterMode::IncludeOnly,
            selected_apps: vec!["mail".into()],
            time_range: TimeRange::All,
            ..Default::default()
        };
        assert_eq!(filtered_history(&s, &only_mail, NOW).unwrap().len(), 1);
        let not_slack = WritingInsightsRequest {
            app_filter_mode: AppFilterMode::Exclude,
            selected_apps: vec!["Slack".into()],
            time_range: TimeRange::All,
            ..Default::default()
        };
        assert_eq!(filtered_history(&s, &not_slack, NOW).unwrap().len(), 2);
        assert_eq!(TimeRange::Days7.start(NOW), Some(NOW - 7 * 86_400));
        assert_eq!(GenerationMode::Deep.history_limit(), 2_000);
    }

    #[test]
    fn llm_report_parsed_and_cleaned() {
        let reply = r#"Sure! {"summary":"You write in short bursts.","signals":[{"label":"Sessions","value":"4","detail":"counted"},{"label":"","value":"x","detail":"y"},{"label":"A","value":"1","detail":"short note"}],"patterns":[{"title":"Openers","detail":"You start with so um.","evidence":"[1] so um"},{"title":"pattern title","detail":"x","evidence":"proof"}],"strengths":["Clear asks"," "],"opportunities":["Drop um","bullet"]}"#;
        let p = parse_insights_reply(reply).unwrap();
        assert_eq!(p.summary, "You write in short bursts.");
        assert_eq!(p.signals.len(), 1);
        assert_eq!(p.patterns.len(), 1);
        assert_eq!(p.patterns[0].evidence.as_deref(), Some("[1] so um"));
        assert_eq!(p.strengths, vec!["Clear asks"]);
        assert_eq!(p.opportunities, vec!["Drop um"]);
        assert!(parse_insights_reply("no json here").is_none());
        assert!(parse_insights_reply(r#"{"summary":"x"}"#).is_none());
        let empty = parse_insights_reply(r#"{"summary":"<summary>","signals":[],"patterns":[],"strengths":[],"opportunities":[]}"#).unwrap();
        assert_eq!(
            empty.summary,
            "Insights generated from your recent dictation sessions."
        );
        assert!(is_placeholder(" Short bullet. "));
    }

    #[test]
    fn generate_uses_llm_then_falls_back_and_persists() {
        let s = sample_store();
        let settings = Settings {
            insights_enabled: true,
            insights_model: "gemma4:e2b".into(),
            ..Settings::default()
        };
        let req = WritingInsightsRequest::default();
        let good = MockLlm::with(|msgs, o| {
            assert!(msgs[0]
                .content
                .contains("History:\n[1] Slack: so um the release"));
            assert_eq!(o.temperature, Some(0.15));
            assert_eq!(o.max_tokens, Some(1_500));
            assert!(o.json_schema.is_some());
            Ok(r#"{"summary":"You plan releases in Slack.","signals":[],"patterns":[],"strengths":["Direct"],"opportunities":[]}"#.into())
        });
        let run = generate_writing_insights(&s, &good, &settings, &req, NOW, 0);
        assert_eq!(run.analyzer_name, "Ollama (Local)");
        assert!(!run.used_fallback);
        assert_eq!(
            run.snapshot.as_ref().unwrap().summary,
            "You plan releases in Slack."
        );
        let id = run.snapshot_id.unwrap();

        let run2 = generate_writing_insights(&s, &MockLlm::offline(), &settings, &req, NOW + 60, 0);
        assert_eq!(run2.analyzer_name, HEURISTIC_ANALYZER);
        assert!(run2.used_fallback);
        assert!(run2.snapshot.is_some());

        let none =
            generate_writing_insights(&s, &MockLlm::offline(), &Settings::default(), &req, NOW, 0);
        assert_eq!(none.analyzer_name, HEURISTIC_ANALYZER);
        assert!(!none.used_fallback);

        let hist = writing_insights_history(&s, 10).unwrap();
        assert_eq!(hist.len(), 3);
        s.set_insight_snapshot_pinned(id, true).unwrap();
        assert_eq!(writing_insights_history(&s, 10).unwrap()[0].id, id);
        assert_eq!(latest_writing_insights(&s).unwrap().unwrap().id, hist[0].id);
        assert!(get_writing_insights(&s, id).unwrap().is_some());

        let empty = store_with(&[]);
        let r = generate_writing_insights(&empty, &good, &settings, &req, NOW, 0);
        assert!(r.snapshot.is_none() && r.error.is_none() && r.sample_count == 0);
        assert!(
            quick_insights(&s, &req, NOW, 0)
                .unwrap()
                .unwrap()
                .analyzer_name
                == QUICK_LOOK
        );
    }

    #[test]
    fn candidate_models_and_freshness() {
        let s = Settings {
            insights_enabled: true,
            insights_model: "".into(),
            polish_enabled: true,
            polish_model: "GEMMA4:E2B".into(),
            ..Settings::default()
        };
        assert_eq!(insight_candidate_models(&s), vec!["gemma4:e2b"]);
        assert!(insight_candidate_models(&Settings::default()).is_empty());

        let st = sample_store();
        let f = freshness(&st, NOW - 5 * 3_600).unwrap();
        assert_eq!(f.new_session_count, 4);
        assert_eq!(f.status, FreshnessStatus::Fresh);
        assert_eq!(f.latest_session_at, Some(NOW - 3_600));
    }

    #[test]
    fn comparison_bullets() {
        let snap = |at: i64, n: usize, ops: &[&str], pats: &[&str]| WritingInsightSnapshot {
            generated_at: at,
            sample_count: n,
            analyzer_name: "x".into(),
            used_fallback: false,
            request: WritingInsightsRequest::default(),
            summary: String::new(),
            signals: vec![],
            patterns: pats
                .iter()
                .map(|t| InsightPattern {
                    title: t.to_string(),
                    detail: "d".into(),
                    evidence: None,
                })
                .collect(),
            strengths: vec![],
            opportunities: ops.iter().map(|s| s.to_string()).collect(),
            recommendations: vec![],
        };
        let a = InsightHistoryItem {
            id: 1,
            is_pinned: false,
            snapshot: snap(NOW - 86_400, 10, &["a", "b"], &["P1"]),
        };
        let b = InsightHistoryItem {
            id: 2,
            is_pinned: false,
            snapshot: snap(NOW, 14, &["b", "c", "d"], &["P1", "Z2", "A3"]),
        };
        let c = compare(&b, &a, 0);
        assert_eq!(c.older_id, 1);
        assert_eq!(c.bullets[0], "Sessions analyzed: 10 -> 14 (+4).");
        assert_eq!(c.bullets[1], "Opportunity changes: 2 added, 1 resolved.");
        assert_eq!(c.bullets[2], "New patterns detected: A3, Z2.");
        assert!(c
            .headline
            .starts_with("Comparing Oct 20, 2026 at 3:32 PM vs Oct 21"));
    }

    #[test]
    fn recommendations_apply() {
        let s = store_with(&[]);
        apply_recommendation(
            &s,
            &InsightRecommendation {
                kind: RecommendationKind::Dictionary,
                source: "orta".into(),
                target: "Orttaai".into(),
                rationale: "".into(),
                confidence: 0.9,
            },
        )
        .unwrap();
        apply_recommendation(
            &s,
            &InsightRecommendation {
                kind: RecommendationKind::Snippet,
                source: "sig".into(),
                target: "Best, Theo".into(),
                rationale: "".into(),
                confidence: 0.9,
            },
        )
        .unwrap();
        assert_eq!(s.list_dictionary().unwrap().len(), 1);
        assert_eq!(s.list_snippets().unwrap().len(), 1);
    }

    #[test]
    fn heuristic_tone_profile_metrics() {
        let samples: Vec<String> = [
            "Thanks so much, we'll ship it on Friday!",
            "I think we could maybe review the budget together?",
            "Thanks so much for the review, we're nearly there.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let p = heuristic_tone_profile(&samples, "", NOW);
        assert_eq!(p.metrics.len(), 6);
        assert_eq!(
            p.metrics
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            TONE_METRICS
        );
        assert_eq!(p.sample_count, 3);
        assert!(p.word_count > 20);
        assert!(p.signature_phrases.contains(&"thanks so much".to_string()));
        assert!(p.summary.starts_with("Your writing voice reads as "));
        assert!(p
            .prompt_guide
            .starts_with("Write in the user's tone of voice."));
        assert!(p.recommendations[0].contains("My Tone mode"));
        assert!(p.confidence < 0.1);
        assert!(p.descriptors.contains(&"warm".to_string()));
        assert_eq!(p.sample_excerpts.len(), 3);
        // Deterministic.
        assert_eq!(p, heuristic_tone_profile(&samples, "", NOW));
    }

    #[test]
    fn tone_reply_merges_over_metrics() {
        let base = heuristic_tone_profile(&["Hello there, thanks a lot.".to_string()], "m", NOW);
        let reply = r#"{"summary":"You sound warm and brisk.","descriptors":["warm","brisk"],"metrics":[{"name":"Warmth","value":1.4,"label":"Warm","detail":"thanks"},{"name":"Bogus","value":0.5,"label":"x","detail":"y"}],"signaturePhrases":[],"promptGuide":"Write warmly.","confidence":0.7}"#;
        let p = parse_tone_reply(reply, &base, "gemma4:e2b", NOW).unwrap();
        assert_eq!(p.summary, "You sound warm and brisk.");
        assert_eq!(p.descriptors, vec!["warm", "brisk"]);
        assert_eq!(p.metrics.len(), 6);
        let warmth = p.metrics.iter().find(|m| m.name == "Warmth").unwrap();
        assert_eq!(warmth.value, 1.0);
        assert_eq!(p.prompt_guide, "Write warmly.");
        assert_eq!(p.confidence_percent(), 70);
        assert_eq!(
            p.avoidances, base.avoidances,
            "missing fields keep the metrics"
        );
        assert!(parse_tone_reply("nope", &base, "m", NOW).is_none());
    }

    #[test]
    fn analyze_tone_end_to_end() {
        let s = sample_store();
        let settings = Settings::default();
        let ok = MockLlm::with(|msgs, o| {
            assert!(msgs[0]
                .content
                .starts_with("You are a computational stylometry analyst."));
            assert!(msgs[0].content.contains("Local metric baseline:"));
            assert_eq!(o.temperature, Some(0.12));
            assert_eq!(o.max_tokens, Some(4_000));
            Ok(r#"{"summary":"You are brisk.","confidence":0.5}"#.into())
        });
        let run = analyze_tone(&s, &ok, &settings, "gemma4:e2b", NOW)
            .unwrap()
            .unwrap();
        assert!(run.used_llm);
        assert_eq!(run.profile.summary, "You are brisk.");
        assert_eq!(run.profile.model, "gemma4:e2b");

        let down = analyze_tone(&s, &MockLlm::offline(), &settings, "gemma4:e2b", NOW + 5)
            .unwrap()
            .unwrap();
        assert!(!down.used_llm);
        assert!(down
            .error
            .unwrap()
            .starts_with("Ollama could not complete tone analysis"));
        assert_eq!(down.profile.model, "");

        let local = analyze_tone(&s, &MockLlm::offline(), &settings, "", NOW + 10)
            .unwrap()
            .unwrap();
        assert!(local.error.is_none());

        assert_eq!(tone_history(&s, 10).unwrap().len(), 3);
        assert_eq!(
            latest_tone(&s).unwrap().unwrap().id,
            local.snapshot_id.unwrap()
        );
        assert!(latest_tone_profile(&s).is_some());
        assert!(get_tone(&s, run.snapshot_id.unwrap()).unwrap().is_some());
        assert!(analyze_tone(&store_with(&[]), &ok, &settings, "m", NOW)
            .unwrap()
            .is_none());
        assert_eq!(default_tone_model(&settings), "gemma4:e2b");
    }
}
