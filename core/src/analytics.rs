//! Analytics: dashboard metrics, trends, streaks, latency health.
//!
//! A port of the macOS `DashboardStatsService` (Home) and
//! `AnalyticsDashboardSnapshot` (Analytics › Dashboard). Everything here is a
//! pure function over [`Sample`]s, so the numbers are testable with fixed
//! timestamps; [`crate::store::Store`] reads the samples (with each one's
//! local UTC offset, so days are bucketed in local time, as on the Mac).

use crate::milestones::{self, Comparisons};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Performance reads this many of the latest dictations: a change shows
/// within a day of dictating.
pub const PERFORMANCE_SAMPLE_COUNT: usize = 50;
/// Typical (p50) total latency under this is "Fast".
pub const FAST_MS: i64 = 1_200;
/// Typical total latency under this is "Normal"; at or above it, "Slow".
pub const NORMAL_MS: i64 = 3_000;
/// Home's Recent Dictations card shows this many.
pub const RECENT_COUNT: usize = 12;
/// Home's Top Apps card shows this many.
pub const TOP_APPS_HOME: usize = 4;
/// Analytics' Words by App card shows this many.
pub const TOP_APPS_ANALYTICS: usize = 5;

const DAY: i64 = 86_400;

/// One dictation as the dashboards see it: counts and timings, never text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    pub id: i64,
    /// Unix seconds (UTC).
    pub created_at: i64,
    /// The machine's local UTC offset (seconds) at `created_at`, which puts
    /// the dictation on its local calendar day.
    pub local_offset: i64,
    /// UTC offset where it was said, when the record's time zone is a fixed
    /// offset; hour and weekday use it ("9 a.m. on a trip is still 9 a.m.").
    pub said_offset: Option<i64>,
    pub words: i64,
    pub recording_ms: i64,
    pub processing_ms: i64,
    pub transcription_ms: Option<i64>,
    pub text_processing_ms: Option<i64>,
    pub injection_ms: Option<i64>,
    pub app: Option<String>,
    pub model_id: String,
    pub entry_kind: String,
}

impl Sample {
    /// Days since 1970-01-01 in local time.
    pub fn local_day(&self) -> i64 {
        (self.created_at + self.local_offset).div_euclid(DAY)
    }

    fn said_seconds(&self) -> i64 {
        self.created_at + self.said_offset.unwrap_or(self.local_offset)
    }

    /// Local hour of day (0–23) where it was said.
    pub fn hour(&self) -> u32 {
        (self.said_seconds().rem_euclid(DAY) / 3_600) as u32
    }

    /// Weekday where it was said, 1 = Sunday … 7 = Saturday (Mac `Calendar`).
    pub fn weekday(&self) -> u32 {
        weekday_of_day(self.said_seconds().div_euclid(DAY))
    }
}

/// "Now" for the dashboards: the instant plus the local UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub now: i64,
    pub offset: i64,
}

impl Clock {
    pub fn today(&self) -> i64 {
        (self.now + self.offset).div_euclid(DAY)
    }
}

/// 1 = Sunday … 7 = Saturday for a day number (1970-01-01 was a Thursday).
pub fn weekday_of_day(day: i64) -> u32 {
    ((day + 4).rem_euclid(7) + 1) as u32
}

/// `YYYY-MM-DD` for a day number (proleptic Gregorian, H. Hinnant's
/// civil-from-days).
pub fn day_string(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse a fixed UTC offset time zone ("+01:00", "-0530", "UTC+2",
/// "GMT-03:30") into seconds. Named zones return `None` (the machine's local
/// time is used instead).
pub fn parse_utc_offset(tz: &str) -> Option<i64> {
    let t = tz.trim();
    let t = t
        .strip_prefix("UTC")
        .or_else(|| t.strip_prefix("GMT"))
        .unwrap_or(t);
    if t.is_empty() || t == "Z" {
        return if tz.trim().is_empty() { None } else { Some(0) };
    }
    let (sign, rest) = match t.as_bytes()[0] {
        b'+' => (1, &t[1..]),
        b'-' => (-1, &t[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = if digits.len() <= 2 {
        (digits.parse::<i64>().ok()?, 0)
    } else {
        let split = digits.len() - 2;
        (
            digits[..split].parse::<i64>().ok()?,
            digits[split..].parse::<i64>().ok()?,
        )
    };
    if h > 14 || m >= 60 {
        return None;
    }
    Some(sign * (h * 3_600 + m * 60))
}

/// Words per minute, rounded; 0 without words or speaking time.
pub fn calculate_wpm(words: i64, recording_ms: i64) -> i64 {
    if words <= 0 || recording_ms <= 0 {
        return 0;
    }
    (words as f64 / (recording_ms as f64 / 60_000.0)).round() as i64
}

/// Typing the same words at the Mac's assumed 40 WPM, minus the time spent
/// speaking them; never below 0 (Mac `MilestoneStats.minutesSaved`).
pub fn minutes_saved(words: i64, recording_ms: i64) -> i64 {
    let typing = words.max(0) as f64 / Comparisons::TYPING_WPM;
    let speaking = recording_ms.max(0) as f64 / 60_000.0;
    ((typing - speaking).floor() as i64).max(0)
}

fn normalized_app_name(raw: Option<&str>) -> String {
    match raw.map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "Unknown App".to_string(),
    }
}

/// "Whisper large v3 turbo_954mb" → without the size suffix (`_\d+(mb|gb)$`).
pub fn canonical_model_id(model_id: &str) -> String {
    let trimmed = model_id.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("mb") || lower.ends_with("gb") {
        let body = &trimmed[..trimmed.len() - 2];
        let digits = body
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if digits > 0 {
            let before = &body[..body.len() - digits];
            if let Some(stripped) = before.strip_suffix('_') {
                return stripped.to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Single line, at most `limit` characters, then "...".
pub fn preview_text(text: &str, limit: usize) -> String {
    let single = text.replace(['\n', '\r'], " ");
    let single = single.trim();
    if single.chars().count() <= limit {
        return single.to_string();
    }
    let cut: String = single.chars().take(limit).collect();
    format!("{cut}...")
}

/// Mean, rounded; `None` without values.
pub fn average_latency(values: &[i64]) -> Option<i64> {
    let valid: Vec<i64> = values.iter().copied().filter(|v| *v >= 0).collect();
    if valid.is_empty() {
        return None;
    }
    Some((valid.iter().sum::<i64>() as f64 / valid.len() as f64).round() as i64)
}

/// Linear-interpolated percentile (0…1), rounded, as on the Mac.
pub fn percentile_latency(values: &[i64], percentile: f64) -> Option<i64> {
    let mut valid: Vec<i64> = values.iter().copied().filter(|v| *v >= 0).collect();
    if valid.is_empty() {
        return None;
    }
    valid.sort_unstable();
    if valid.len() == 1 {
        return Some(valid[0]);
    }
    let p = percentile.clamp(0.0, 1.0);
    let rank = p * (valid.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return Some(valid[lower]);
    }
    let weight = rank - lower as f64;
    let (lo, hi) = (valid[lower] as f64, valid[upper] as f64);
    Some((lo + (hi - lo) * weight).round() as i64)
}

// ---- Output types ----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct HeaderStats {
    pub active_days: i64,
    pub total_words: i64,
    pub average_wpm: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TodaySnapshot {
    pub words: i64,
    pub sessions: i64,
    pub active_minutes: i64,
    pub average_wpm: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TrendPoint {
    /// Local calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub words: i64,
    pub sessions: i64,
    pub average_wpm: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopApp {
    pub name: String,
    pub sessions: i64,
    pub words: i64,
    pub session_share: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceLevel {
    #[default]
    NoData,
    Fast,
    Normal,
    Slow,
}

impl PerformanceLevel {
    /// By the typical dictation: one stuck paste mustn't mark Orttaai slow.
    pub fn for_typical_ms(p50: Option<i64>) -> Self {
        match p50 {
            None => Self::NoData,
            Some(ms) if ms < FAST_MS => Self::Fast,
            Some(ms) if ms < NORMAL_MS => Self::Normal,
            Some(_) => Self::Slow,
        }
    }
}

/// How long the latest dictations took from key release to text in place,
/// and each stage: transcription, text processing (rules, dictionary and
/// Polish), then insertion.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PerformanceHealth {
    pub level: PerformanceLevel,
    pub sample_count: i64,
    pub average_processing_ms: Option<i64>,
    pub processing_p50_ms: Option<i64>,
    pub processing_p95_ms: Option<i64>,
    pub average_transcription_ms: Option<i64>,
    pub transcription_p50_ms: Option<i64>,
    pub transcription_p95_ms: Option<i64>,
    pub average_text_processing_ms: Option<i64>,
    pub text_processing_p50_ms: Option<i64>,
    pub text_processing_p95_ms: Option<i64>,
    pub average_injection_ms: Option<i64>,
    pub injection_p50_ms: Option<i64>,
    pub injection_p95_ms: Option<i64>,
    pub current_model_id: String,
    /// The Express (Parakeet) engine is the current engine.
    pub is_express: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecentDictation {
    pub id: i64,
    pub created_at: i64,
    pub full_text: String,
    pub preview_text: String,
    pub app_name: String,
    pub word_count: i64,
    pub processing_ms: i64,
}

/// Everything Home shows (Mac `DashboardStatsPayload`).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct DashboardPayload {
    pub header: HeaderStats,
    pub today: TodaySnapshot,
    pub trend30d: Vec<TrendPoint>,
    pub top_apps30d: Vec<TopApp>,
    pub performance: PerformanceHealth,
    pub recent: Vec<RecentDictation>,
    /// Average words on the other active days of the last 30, for the Today
    /// card's comparison; `None` without earlier history.
    pub usual_daily_words: Option<i64>,
    pub streaks: Streaks,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HourlyPoint {
    pub hour: u32,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WeekdayPoint {
    /// 1 = Sunday.
    pub weekday: u32,
    pub short_name: &'static str,
    pub name: &'static str,
    pub sessions: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DurationBucket {
    pub label: &'static str,
    /// How the bucket reads in a sentence ("5 to 15 seconds").
    pub phrase: &'static str,
    pub count: i64,
    pub order: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppWordCount {
    pub name: String,
    pub words: i64,
}

/// Day-run streaks, counted in local calendar days.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Streaks {
    /// Days in the run still going today or yesterday; 0 once broken.
    pub current_days: i64,
    pub best_days: i64,
    pub dictated_today: bool,
}

/// Totals for one window, for "vs previous period".
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PeriodTotals {
    pub sessions: i64,
    pub words: i64,
    pub recording_minutes: i64,
    pub average_wpm: i64,
    pub active_days: i64,
    pub minutes_saved: i64,
}

/// The range against the window just before it; changes are fractions
/// (`0.25` = 25% more), `None` when the earlier window had none.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PeriodTrend {
    pub previous: PeriodTotals,
    pub words_change: Option<f64>,
    pub sessions_change: Option<f64>,
    pub active_minutes_change: Option<f64>,
    pub wpm_change: Option<f64>,
}

/// Analytics' range picker (Mac `AnalyticsTimeRange`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeRange {
    #[default]
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "90d")]
    NinetyDays,
    #[serde(rename = "all")]
    AllTime,
}

impl TimeRange {
    /// Accepts the stored setting (`30d | 90d | all`) and the Mac's labels.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "90d" | "90 days" | "90" => Self::NinetyDays,
            "all" | "all time" | "alltime" | "all_time" => Self::AllTime,
            _ => Self::ThirtyDays,
        }
    }

    pub fn day_count(self) -> Option<i64> {
        match self {
            Self::ThirtyDays => Some(30),
            Self::NinetyDays => Some(90),
            Self::AllTime => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ThirtyDays => "30d",
            Self::NinetyDays => "90d",
            Self::AllTime => "all",
        }
    }
}

/// Everything Analytics › Dashboard shows for one range.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AnalyticsSnapshot {
    pub range: TimeRange,
    pub payload: DashboardPayload,
    pub trend_points: Vec<TrendPoint>,
    /// Seconds spoken on each day of `trend_points`, same order.
    pub daily_active_seconds: Vec<i64>,
    pub hourly_activity: Vec<HourlyPoint>,
    pub weekday_activity: Vec<WeekdayPoint>,
    pub duration_buckets: Vec<DurationBucket>,
    pub words_by_app: Vec<AppWordCount>,
    pub median_duration_ms: Option<i64>,
    pub total_sessions: i64,
    pub total_words: i64,
    pub total_recording_minutes: i64,
    pub active_days: i64,
    pub average_wpm: i64,
    /// Typing time given back in the range, at 40 WPM.
    pub minutes_saved: i64,
    pub streaks: Streaks,
    /// `None` for All Time.
    pub previous: Option<PeriodTrend>,
}

// ---- Builders --------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct DayAggregate {
    words: i64,
    sessions: i64,
    recording_ms: i64,
}

fn header_stats(samples: &[Sample]) -> HeaderStats {
    let words: i64 = samples.iter().map(|s| s.words.max(0)).sum();
    let recording: i64 = samples.iter().map(|s| s.recording_ms.max(0)).sum();
    let days: HashSet<i64> = samples.iter().map(Sample::local_day).collect();
    HeaderStats {
        active_days: days.len() as i64,
        total_words: words,
        average_wpm: calculate_wpm(words, recording),
    }
}

fn today_snapshot(samples: &[&Sample]) -> TodaySnapshot {
    let words: i64 = samples.iter().map(|s| s.words.max(0)).sum();
    let recording: i64 = samples.iter().map(|s| s.recording_ms.max(0)).sum();
    TodaySnapshot {
        words,
        sessions: samples.len() as i64,
        active_minutes: (recording as f64 / 60_000.0).round() as i64,
        average_wpm: calculate_wpm(words, recording),
    }
}

fn day_aggregates<'a>(samples: impl Iterator<Item = &'a Sample>) -> HashMap<i64, DayAggregate> {
    let mut by_day: HashMap<i64, DayAggregate> = HashMap::new();
    for s in samples {
        let a = by_day.entry(s.local_day()).or_default();
        a.words += s.words.max(0);
        a.sessions += 1;
        a.recording_ms += s.recording_ms.max(0);
    }
    by_day
}

fn trend_points(by_day: &HashMap<i64, DayAggregate>, start: i64, end: i64) -> Vec<TrendPoint> {
    (start..=end)
        .map(|day| {
            let a = by_day.get(&day).copied().unwrap_or_default();
            TrendPoint {
                day: day_string(day),
                words: a.words,
                sessions: a.sessions,
                average_wpm: calculate_wpm(a.words, a.recording_ms),
            }
        })
        .collect()
}

fn top_apps(samples: &[&Sample], limit: usize) -> Vec<TopApp> {
    let mut by_app: HashMap<String, (i64, i64)> = HashMap::new();
    for s in samples {
        let e = by_app
            .entry(normalized_app_name(s.app.as_deref()))
            .or_default();
        e.0 += 1;
        e.1 += s.words.max(0);
    }
    let total = samples.len().max(1) as f64;
    let mut apps: Vec<TopApp> = by_app
        .into_iter()
        .map(|(name, (sessions, words))| TopApp {
            name,
            sessions,
            words,
            session_share: sessions as f64 / total,
        })
        .collect();
    apps.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then(b.words.cmp(&a.words))
            .then(a.name.cmp(&b.name))
    });
    apps.truncate(limit);
    apps
}

/// `samples` newest first; the latest [`PERFORMANCE_SAMPLE_COUNT`] are measured.
pub fn performance(
    samples_newest_first: &[Sample],
    current_model_id: Option<&str>,
    is_express: bool,
) -> PerformanceHealth {
    let scoped: Vec<&Sample> = samples_newest_first
        .iter()
        .take(PERFORMANCE_SAMPLE_COUNT)
        .collect();
    let processing: Vec<i64> = scoped.iter().map(|s| s.processing_ms.max(0)).collect();
    let transcription: Vec<i64> = scoped.iter().filter_map(|s| s.transcription_ms).collect();
    let text: Vec<i64> = scoped.iter().filter_map(|s| s.text_processing_ms).collect();
    let injection: Vec<i64> = scoped.iter().filter_map(|s| s.injection_ms).collect();
    let processing_p50 = percentile_latency(&processing, 0.5);

    let model = current_model_id
        .map(canonical_model_id)
        .filter(|m| !m.is_empty())
        .or_else(|| {
            samples_newest_first
                .first()
                .map(|s| canonical_model_id(&s.model_id))
                .filter(|m| !m.is_empty())
        })
        .unwrap_or_else(|| "Not set".to_string());

    PerformanceHealth {
        level: PerformanceLevel::for_typical_ms(processing_p50),
        sample_count: scoped.len() as i64,
        average_processing_ms: average_latency(&processing),
        processing_p50_ms: processing_p50,
        processing_p95_ms: percentile_latency(&processing, 0.95),
        average_transcription_ms: average_latency(&transcription),
        transcription_p50_ms: percentile_latency(&transcription, 0.5),
        transcription_p95_ms: percentile_latency(&transcription, 0.95),
        average_text_processing_ms: average_latency(&text),
        text_processing_p50_ms: percentile_latency(&text, 0.5),
        text_processing_p95_ms: percentile_latency(&text, 0.95),
        average_injection_ms: average_latency(&injection),
        injection_p50_ms: percentile_latency(&injection, 0.5),
        injection_p95_ms: percentile_latency(&injection, 0.95),
        current_model_id: model,
        is_express,
    }
}

/// Current and best runs of consecutive local days with a dictation.
pub fn streaks(samples: &[Sample], clock: Clock) -> Streaks {
    let mut days: Vec<i64> = samples.iter().map(Sample::local_day).collect();
    days.sort_unstable();
    days.dedup();
    let (mut best, mut run) = (0i64, 0i64);
    let mut last: Option<i64> = None;
    for d in &days {
        run = match last {
            Some(prev) if d - prev == 1 => run + 1,
            _ => 1,
        };
        best = best.max(run);
        last = Some(*d);
    }
    let today = clock.today();
    let current = match last {
        Some(l) if today - l <= 1 => run,
        _ => 0,
    };
    Streaks {
        current_days: current,
        best_days: best,
        dictated_today: last == Some(today),
    }
}

/// Mac `RecentDictationsCard` rows from records (newest first).
pub fn recent_dictations(records: &[crate::store::TranscriptionRecord]) -> Vec<RecentDictation> {
    records
        .iter()
        .take(RECENT_COUNT)
        .map(|r| RecentDictation {
            id: r.id.unwrap_or(r.created_at * 1_000),
            created_at: r.created_at,
            full_text: r.text.clone(),
            preview_text: preview_text(&r.text, 120),
            app_name: normalized_app_name(r.app.as_deref()),
            word_count: r.text.split_whitespace().count() as i64,
            processing_ms: r.processing_ms.max(0),
        })
        .collect()
}

/// Home's payload. `samples` may be in any order.
pub fn dashboard_payload(
    samples: &[Sample],
    recent: &[crate::store::TranscriptionRecord],
    clock: Clock,
    current_model_id: Option<&str>,
    is_express: bool,
) -> DashboardPayload {
    let today = clock.today();
    let thirty_start = today - 29;
    let todays: Vec<&Sample> = samples.iter().filter(|s| s.local_day() == today).collect();
    let thirty: Vec<&Sample> = samples
        .iter()
        .filter(|s| (thirty_start..=today).contains(&s.local_day()))
        .collect();
    let trend30d = trend_points(&day_aggregates(thirty.iter().copied()), thirty_start, today);
    let mut newest: Vec<Sample> = samples.to_vec();
    newest.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));

    let earlier: Vec<&TrendPoint> = trend30d[..trend30d.len().saturating_sub(1)]
        .iter()
        .filter(|p| p.sessions > 0)
        .collect();
    let usual_daily_words = if earlier.is_empty() {
        None
    } else {
        Some(earlier.iter().map(|p| p.words).sum::<i64>() / earlier.len() as i64)
    };

    DashboardPayload {
        header: header_stats(samples),
        today: today_snapshot(&todays),
        top_apps30d: top_apps(&thirty, TOP_APPS_HOME),
        trend30d,
        performance: performance(&newest, current_model_id, is_express),
        recent: recent_dictations(recent),
        usual_daily_words,
        streaks: streaks(samples, clock),
    }
}

const WEEKDAYS: [(&str, &str); 7] = [
    ("Sun", "Sunday"),
    ("Mon", "Monday"),
    ("Tue", "Tuesday"),
    ("Wed", "Wednesday"),
    ("Thu", "Thursday"),
    ("Fri", "Friday"),
    ("Sat", "Saturday"),
];

const BUCKETS: [(&str, &str, i64); 7] = [
    ("<5s", "under 5 seconds", 5_000),
    ("5–15s", "5 to 15 seconds", 15_000),
    ("15–30s", "15 to 30 seconds", 30_000),
    ("30s–1m", "30 seconds to a minute", 60_000),
    ("1–2m", "1 to 2 minutes", 120_000),
    ("2–5m", "2 to 5 minutes", 300_000),
    ("5m+", "over 5 minutes", i64::MAX),
];

fn period_totals(samples: &[&Sample]) -> PeriodTotals {
    let words: i64 = samples.iter().map(|s| s.words.max(0)).sum();
    let recording: i64 = samples.iter().map(|s| s.recording_ms.max(0)).sum();
    let days: HashSet<i64> = samples.iter().map(|s| s.local_day()).collect();
    PeriodTotals {
        sessions: samples.len() as i64,
        words,
        recording_minutes: (recording as f64 / 60_000.0).round() as i64,
        average_wpm: calculate_wpm(words, recording),
        active_days: days.len() as i64,
        minutes_saved: minutes_saved(words, recording),
    }
}

fn change(current: i64, previous: i64) -> Option<f64> {
    (previous > 0).then(|| (current - previous) as f64 / previous as f64)
}

/// Analytics › Dashboard for a range. `samples` is the whole archive (the
/// previous period and Home's payload need data outside the range).
pub fn analytics_snapshot(
    samples: &[Sample],
    recent: &[crate::store::TranscriptionRecord],
    clock: Clock,
    range: TimeRange,
    current_model_id: Option<&str>,
    is_express: bool,
) -> AnalyticsSnapshot {
    let today = clock.today();
    let in_range: Vec<&Sample> = match range.day_count() {
        Some(n) => samples
            .iter()
            .filter(|s| (today - (n - 1)..=today).contains(&s.local_day()))
            .collect(),
        None => samples.iter().collect(),
    };
    let totals = period_totals(&in_range);

    // Trend: the range's days, or from the oldest dictation for All Time.
    let start = match range.day_count() {
        Some(n) => Some(today - (n - 1)),
        None => in_range.iter().map(|s| s.local_day()).min(),
    };
    let by_day = day_aggregates(in_range.iter().copied());
    let (trend, daily_active_seconds) = match start {
        Some(start) => {
            let start = start.min(today);
            let pts = trend_points(&by_day, start, today);
            let secs = (start..=today)
                .map(|d| by_day.get(&d).map(|a| a.recording_ms / 1_000).unwrap_or(0))
                .collect();
            (pts, secs)
        }
        None => (Vec::new(), Vec::new()),
    };

    let mut by_hour = [0i64; 24];
    let mut by_weekday = [0i64; 8];
    for s in &in_range {
        by_hour[s.hour() as usize] += 1;
        by_weekday[s.weekday() as usize] += 1;
    }
    let hourly_activity = (0..24)
        .map(|h| HourlyPoint {
            hour: h as u32,
            count: by_hour[h],
        })
        .collect();
    let weekday_activity = (1..=7u32)
        .map(|w| WeekdayPoint {
            weekday: w,
            short_name: WEEKDAYS[(w - 1) as usize].0,
            name: WEEKDAYS[(w - 1) as usize].1,
            sessions: by_weekday[w as usize],
        })
        .collect();

    let mut durations: Vec<i64> = in_range.iter().map(|s| s.recording_ms.max(0)).collect();
    durations.sort_unstable();
    let median_duration_ms = (!durations.is_empty()).then(|| durations[durations.len() / 2]);

    let mut counts = [0i64; BUCKETS.len()];
    for ms in &durations {
        let index = BUCKETS
            .iter()
            .position(|b| *ms < b.2)
            .unwrap_or(BUCKETS.len() - 1);
        counts[index] += 1;
    }
    let duration_buckets = BUCKETS
        .iter()
        .enumerate()
        .map(|(i, b)| DurationBucket {
            label: b.0,
            phrase: b.1,
            count: counts[i],
            order: i,
        })
        .collect();

    let mut app_words: BTreeMap<String, i64> = BTreeMap::new();
    for s in &in_range {
        let name = match s.app.as_deref().map(str::trim) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => "Unknown".to_string(),
        };
        *app_words.entry(name).or_default() += s.words.max(0);
    }
    let mut words_by_app: Vec<AppWordCount> = app_words
        .into_iter()
        .map(|(name, words)| AppWordCount { name, words })
        .collect();
    words_by_app.sort_by(|a, b| b.words.cmp(&a.words).then(a.name.cmp(&b.name)));
    words_by_app.truncate(TOP_APPS_ANALYTICS);

    let previous = range.day_count().map(|n| {
        let prev_end = today - n;
        let prev_start = prev_end - (n - 1);
        let prev: Vec<&Sample> = samples
            .iter()
            .filter(|s| (prev_start..=prev_end).contains(&s.local_day()))
            .collect();
        let p = period_totals(&prev);
        PeriodTrend {
            words_change: change(totals.words, p.words),
            sessions_change: change(totals.sessions, p.sessions),
            active_minutes_change: change(totals.recording_minutes, p.recording_minutes),
            wpm_change: change(totals.average_wpm, p.average_wpm),
            previous: p,
        }
    });

    AnalyticsSnapshot {
        range,
        payload: dashboard_payload(samples, recent, clock, current_model_id, is_express),
        trend_points: trend,
        daily_active_seconds,
        hourly_activity,
        weekday_activity,
        duration_buckets,
        words_by_app,
        median_duration_ms,
        total_sessions: totals.sessions,
        total_words: totals.words,
        total_recording_minutes: totals.recording_minutes,
        active_days: totals.active_days,
        average_wpm: totals.average_wpm,
        minutes_saved: totals.minutes_saved,
        streaks: streaks(samples, clock),
        previous,
    }
}

// ---- GitHub star prompt (Mac HomeViewModel) --------------------------------

pub const GITHUB_PROMPT_COOLDOWN_SECS: i64 = 7 * DAY;
pub const GITHUB_PROMPT_MIN_SESSIONS_TODAY: i64 = 3;
pub const GITHUB_PROMPT_MIN_WORDS: i64 = 200;
pub const GITHUB_PROMPT_MAX_SHOWS: u32 = 3;

/// What Home does about the "Enjoying Orttaai?" prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubPromptDecision {
    /// Show it now (and record the showing).
    Show,
    /// Shown enough times: mark it completed for good.
    Exhausted,
    /// Not now.
    Skip,
}

pub fn github_prompt_decision(
    payload: &DashboardPayload,
    completed: bool,
    shown_count: u32,
    last_shown_at: i64,
    now: i64,
) -> GithubPromptDecision {
    if completed {
        return GithubPromptDecision::Skip;
    }
    let enough = payload.today.sessions >= GITHUB_PROMPT_MIN_SESSIONS_TODAY
        || payload.header.total_words >= GITHUB_PROMPT_MIN_WORDS;
    if !enough {
        return GithubPromptDecision::Skip;
    }
    if shown_count >= GITHUB_PROMPT_MAX_SHOWS {
        return GithubPromptDecision::Exhausted;
    }
    if last_shown_at > 0 && now - last_shown_at < GITHUB_PROMPT_COOLDOWN_SECS {
        return GithubPromptDecision::Skip;
    }
    GithubPromptDecision::Show
}

/// Re-exported so callers can format the same way the copy does.
pub use milestones::fmt_int;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    // 2026-03-10 00:00:00 UTC (a Tuesday).
    const T0: i64 = 1_773_100_800;
    const H: i64 = 3_600;

    fn sample(id: i64, at: i64, offset: i64, words: i64, rec_ms: i64) -> Sample {
        Sample {
            id,
            created_at: at,
            local_offset: offset,
            words,
            recording_ms: rec_ms,
            processing_ms: 800,
            model_id: "ggml-base.en-q5_1".into(),
            entry_kind: "dictation".into(),
            ..Default::default()
        }
    }

    #[test]
    fn calendar_helpers() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(T0 / DAY), "2026-03-10");
        assert_eq!(weekday_of_day(0), 5); // Thursday
        assert_eq!(weekday_of_day(T0 / DAY), 3); // Tuesday
        assert_eq!(day_string(-1), "1969-12-31");
        assert_eq!(day_string(11_016), "2000-02-29");
    }

    #[test]
    fn offsets_parse() {
        assert_eq!(parse_utc_offset("+01:00"), Some(3_600));
        assert_eq!(parse_utc_offset("-0530"), Some(-(5 * 3_600 + 30 * 60)));
        assert_eq!(parse_utc_offset("UTC+2"), Some(7_200));
        assert_eq!(parse_utc_offset("GMT"), Some(0));
        assert_eq!(parse_utc_offset("Europe/London"), None);
        assert_eq!(parse_utc_offset(""), None);
    }

    #[test]
    fn days_bucket_in_local_time() {
        // 23:30 UTC on Mar 9 is 00:30 Mar 10 at UTC+1, but 18:30 Mar 9 at UTC-5.
        let at = T0 - 30 * 60;
        assert_eq!(day_string(sample(1, at, H, 1, 1).local_day()), "2026-03-10");
        assert_eq!(
            day_string(sample(1, at, -5 * H, 1, 1).local_day()),
            "2026-03-09"
        );
        assert_eq!(sample(1, at, H, 1, 1).hour(), 0);
        assert_eq!(sample(1, at, -5 * H, 1, 1).hour(), 18);
        let mut s = sample(1, at, -5 * H, 1, 1);
        s.said_offset = Some(H);
        assert_eq!(s.hour(), 0);
        assert_eq!(s.weekday(), 3);
        assert_eq!(day_string(s.local_day()), "2026-03-09");
    }

    #[test]
    fn wpm_time_saved_and_model() {
        assert_eq!(calculate_wpm(150, 60_000), 150);
        assert_eq!(calculate_wpm(0, 60_000), 0);
        assert_eq!(calculate_wpm(10, 0), 0);
        // 400 words typed at 40 WPM = 10 min; spoken in 3 min → 7 saved.
        assert_eq!(minutes_saved(400, 180_000), 7);
        assert_eq!(minutes_saved(10, 600_000), 0);
        assert_eq!(
            canonical_model_id(" whisper-large-v3_954mb "),
            "whisper-large-v3"
        );
        assert_eq!(canonical_model_id("ggml-base.en-q5_1"), "ggml-base.en-q5_1");
        assert_eq!(canonical_model_id("tiny_75GB"), "tiny");
        assert_eq!(preview_text("a\nb  ", 120), "a b");
        assert_eq!(preview_text("abcdef", 3), "abc...");
    }

    #[test]
    fn percentiles_match_mac() {
        assert_eq!(percentile_latency(&[], 0.5), None);
        assert_eq!(percentile_latency(&[700], 0.95), Some(700));
        assert_eq!(percentile_latency(&[100, 200, 300, 400], 0.5), Some(250));
        assert_eq!(percentile_latency(&[100, 200, 300, 400], 0.95), Some(385));
        assert_eq!(average_latency(&[100, 201, -5]), Some(151));
    }

    #[test]
    fn latency_health_thresholds() {
        let mk = |ms: i64| {
            let mut s = sample(1, T0, 0, 5, 1_000);
            s.processing_ms = ms;
            s.transcription_ms = Some(ms / 2);
            s
        };
        assert_eq!(
            performance(&[], None, false).level,
            PerformanceLevel::NoData
        );
        assert_eq!(performance(&[], None, false).current_model_id, "Not set");
        assert_eq!(
            performance(&[mk(1_199)], None, false).level,
            PerformanceLevel::Fast
        );
        assert_eq!(
            performance(&[mk(1_200)], None, false).level,
            PerformanceLevel::Normal
        );
        assert_eq!(
            performance(&[mk(2_999)], None, false).level,
            PerformanceLevel::Normal
        );
        assert_eq!(
            performance(&[mk(3_000)], None, false).level,
            PerformanceLevel::Slow
        );
        // One stuck paste doesn't make it slow: the median decides.
        let mixed = [mk(500), mk(600), mk(9_000)];
        let h = performance(&mixed, Some("tiny_75mb"), true);
        assert_eq!(h.level, PerformanceLevel::Fast);
        assert_eq!(h.processing_p50_ms, Some(600));
        assert_eq!(h.transcription_p50_ms, Some(300));
        assert_eq!(h.text_processing_p50_ms, None);
        assert_eq!(h.current_model_id, "tiny");
        assert!(h.is_express);
        // Only the latest 50 are measured.
        let many: Vec<Sample> = (0..80)
            .map(|i| mk(if i < 50 { 400 } else { 9_000 }))
            .collect();
        let h = performance(&many, None, false);
        assert_eq!(h.sample_count, 50);
        assert_eq!(h.processing_p95_ms, Some(400));
        assert_eq!(h.current_model_id, "ggml-base.en-q5_1");
    }

    #[test]
    fn streaks_count_local_days() {
        let clock = Clock {
            now: T0 + 10 * H,
            offset: 0,
        };
        // Mar 6, 7, 8 (run of 3), then Mar 9 and today Mar 10 (run of... 5).
        let s: Vec<Sample> = [-4, -3, -2, -1, 0]
            .iter()
            .enumerate()
            .map(|(i, d)| sample(i as i64, T0 + d * DAY + H, 0, 10, 5_000))
            .collect();
        let st = streaks(&s, clock);
        assert_eq!(
            (st.current_days, st.best_days, st.dictated_today),
            (5, 5, true)
        );
        // A gap breaks the run; yesterday still counts as running.
        let s = vec![
            sample(1, T0 - 10 * DAY, 0, 1, 1),
            sample(2, T0 - 9 * DAY, 0, 1, 1),
            sample(3, T0 - 8 * DAY, 0, 1, 1),
            sample(4, T0 - DAY, 0, 1, 1),
        ];
        let st = streaks(&s, clock);
        assert_eq!(
            (st.current_days, st.best_days, st.dictated_today),
            (1, 3, false)
        );
        let st = streaks(&s[..3], clock);
        assert_eq!(st.current_days, 0);
        // Two dictations either side of local midnight are two days.
        let s = vec![
            sample(1, T0 - 30 * 60 - H, H, 1, 1),
            sample(2, T0 + 30 * 60 - H, H, 1, 1),
        ];
        assert_eq!(streaks(&s, clock).best_days, 2);
    }

    #[test]
    fn home_payload_today_trend_and_apps() {
        let clock = Clock {
            now: T0 + 20 * H,
            offset: 0,
        };
        let mut s = vec![
            sample(1, T0 + 9 * H, 0, 100, 60_000),
            sample(2, T0 + 10 * H, 0, 50, 30_000),
            sample(3, T0 - DAY, 0, 30, 20_000),
            sample(4, T0 - 40 * DAY, 0, 1_000, 400_000),
        ];
        s[0].app = Some("Firefox".into());
        s[1].app = Some(" Firefox ".into());
        s[2].app = Some("Code".into());
        let recent = vec![TranscriptionRecord {
            id: Some(2),
            text: "hello\nworld".into(),
            app: None,
            processing_ms: 900,
            created_at: T0 + 10 * H,
            ..Default::default()
        }];
        let p = dashboard_payload(&s, &recent, clock, Some("tiny"), false);
        assert_eq!(p.header.total_words, 1_180);
        assert_eq!(p.header.active_days, 3);
        assert_eq!(p.today.words, 150);
        assert_eq!(p.today.sessions, 2);
        assert_eq!(p.today.active_minutes, 2); // 1.5 min rounds up
        assert_eq!(p.today.average_wpm, 100);
        assert_eq!(p.trend30d.len(), 30);
        assert_eq!(p.trend30d.last().unwrap().day, "2026-03-10");
        assert_eq!(p.trend30d.last().unwrap().words, 150);
        assert_eq!(p.trend30d[28].words, 30);
        assert_eq!(p.top_apps30d[0].name, "Firefox");
        assert_eq!(p.top_apps30d[0].sessions, 2);
        assert!((p.top_apps30d[0].session_share - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(p.top_apps30d[1].name, "Code");
        assert_eq!(p.usual_daily_words, Some(30));
        assert_eq!(p.recent[0].preview_text, "hello world");
        assert_eq!(p.recent[0].app_name, "Unknown App");
        assert_eq!(p.recent[0].word_count, 2);
        assert_eq!(p.streaks.current_days, 2);
    }

    #[test]
    fn analytics_ranges_buckets_and_previous_period() {
        let clock = Clock {
            now: T0 + 20 * H,
            offset: 0,
        };
        let mut s = vec![
            sample(1, T0 + 9 * H, 0, 100, 4_000),        // today, Tue 09h
            sample(2, T0 - DAY + 22 * H, 0, 50, 16_000), // Mon 22h
            sample(3, T0 - 20 * DAY, 0, 40, 70_000),     // in 30d
            sample(4, T0 - 45 * DAY, 0, 400, 400_000),   // previous 30d / in 90d
            sample(5, T0 - 200 * DAY, 0, 10, 1_000),     // only all-time
        ];
        s[0].app = Some("Slack".into());
        let a = analytics_snapshot(&s, &[], clock, TimeRange::ThirtyDays, None, false);
        assert_eq!(a.total_sessions, 3);
        assert_eq!(a.total_words, 190);
        assert_eq!(a.active_days, 3);
        assert_eq!(a.trend_points.len(), 30);
        assert_eq!(a.daily_active_seconds.len(), 30);
        assert_eq!(*a.daily_active_seconds.last().unwrap(), 4);
        assert_eq!(a.hourly_activity[9].count, 1);
        assert_eq!(a.hourly_activity[22].count, 1);
        assert_eq!(a.weekday_activity[2].name, "Tuesday");
        assert_eq!(a.weekday_activity[2].sessions, 1);
        assert_eq!(a.weekday_activity[1].sessions, 1);
        let counts: Vec<i64> = a.duration_buckets.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![1, 0, 1, 0, 1, 0, 0]);
        assert_eq!(a.median_duration_ms, Some(16_000));
        assert_eq!(a.words_by_app[0].name, "Slack");
        assert_eq!(a.words_by_app[1].name, "Unknown");
        let prev = a.previous.as_ref().unwrap();
        assert_eq!(prev.previous.sessions, 1);
        assert_eq!(prev.previous.words, 400);
        assert_eq!(prev.sessions_change, Some(2.0));
        assert!((prev.words_change.unwrap() - (190.0 - 400.0) / 400.0).abs() < 1e-9);

        let a = analytics_snapshot(&s, &[], clock, TimeRange::NinetyDays, None, false);
        assert_eq!(a.total_sessions, 4);
        assert_eq!(a.trend_points.len(), 90);
        assert_eq!(a.previous.unwrap().previous.sessions, 0);

        let a = analytics_snapshot(&s, &[], clock, TimeRange::AllTime, None, false);
        assert_eq!(a.total_sessions, 5);
        assert_eq!(a.trend_points.len(), 201);
        assert_eq!(a.trend_points[0].day, day_string(T0 / DAY - 200));
        assert!(a.previous.is_none());
        assert_eq!(a.minutes_saved, minutes_saved(600, 491_000));

        let empty = analytics_snapshot(&[], &[], clock, TimeRange::AllTime, None, false);
        assert!(empty.trend_points.is_empty());
        assert_eq!(empty.median_duration_ms, None);
        assert_eq!(empty.payload.performance.level, PerformanceLevel::NoData);
    }

    #[test]
    fn range_parsing() {
        assert_eq!(TimeRange::parse("90d"), TimeRange::NinetyDays);
        assert_eq!(TimeRange::parse("All Time"), TimeRange::AllTime);
        assert_eq!(TimeRange::parse("nonsense"), TimeRange::ThirtyDays);
        assert_eq!(TimeRange::AllTime.key(), "all");
    }

    #[test]
    fn github_prompt_gating() {
        let mut p = DashboardPayload::default();
        let now = T0;
        assert_eq!(
            github_prompt_decision(&p, false, 0, 0, now),
            GithubPromptDecision::Skip
        );
        p.header.total_words = 200;
        assert_eq!(
            github_prompt_decision(&p, false, 0, 0, now),
            GithubPromptDecision::Show
        );
        assert_eq!(
            github_prompt_decision(&p, true, 0, 0, now),
            GithubPromptDecision::Skip
        );
        assert_eq!(
            github_prompt_decision(&p, false, 1, now - DAY, now),
            GithubPromptDecision::Skip
        );
        assert_eq!(
            github_prompt_decision(&p, false, 1, now - 8 * DAY, now),
            GithubPromptDecision::Show
        );
        assert_eq!(
            github_prompt_decision(&p, false, 3, 0, now),
            GithubPromptDecision::Exhausted
        );
        p.header.total_words = 0;
        p.today.sessions = 3;
        assert_eq!(
            github_prompt_decision(&p, false, 0, 0, now),
            GithubPromptDecision::Show
        );
    }
}
