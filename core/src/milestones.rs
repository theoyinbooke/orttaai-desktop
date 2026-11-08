//! Milestones: badge catalog, progress evaluation and unlocks.
//!
//! A port of the macOS `MilestoneCatalog`, `MilestoneStats`,
//! `MilestoneComparisons`, `MilestoneCopy` and `MilestoneCenter`. The engine
//! reads counts, durations, dates, app names and the dictionary size — never
//! what was said. Unlocks are stored (`milestone_unlock`), so deleting history
//! never takes a badge back.

use crate::error::Result;
use crate::store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// The five groups the Milestones page lays badges out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Words,
    Streaks,
    SpeedAndTime,
    Dictations,
    Special,
}

impl Family {
    pub const ALL: [Family; 5] = [
        Family::Words,
        Family::Streaks,
        Family::SpeedAndTime,
        Family::Dictations,
        Family::Special,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Family::Words => "Words",
            Family::Streaks => "Streaks",
            Family::SpeedAndTime => "Speed & time",
            Family::Dictations => "Dictations",
            Family::Special => "Special",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Family::Words => "Total words dictated",
            Family::Streaks => "Days in a row with a dictation",
            Family::SpeedAndTime => "Pace, and typing time you got back",
            Family::Dictations => "Times you held the key and spoke",
            Family::Special => "How and where you dictate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
}

/// The number a badge is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Words,
    Dictations,
    /// The longest run of local days with a dictation, ever.
    BestStreakDays,
    BestWpm,
    MinutesSaved,
    DistinctApps,
    LongestDictationSeconds,
    NightOwlDictations,
    DictionaryWords,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Badge {
    pub id: &'static str,
    pub name: &'static str,
    /// What it takes, in a few words ("10 dictionary words").
    pub rule: &'static str,
    pub family: Family,
    pub tier: Tier,
    /// The Mac's SF Symbol name; the UI draws a matching glyph.
    pub symbol: &'static str,
    pub metric: Metric,
    pub threshold: i64,
}

#[allow(clippy::too_many_arguments)] // one row of the catalog table
const fn badge(
    id: &'static str,
    name: &'static str,
    rule: &'static str,
    family: Family,
    tier: Tier,
    symbol: &'static str,
    metric: Metric,
    threshold: i64,
) -> Badge {
    Badge {
        id,
        name,
        rule,
        family,
        tier,
        symbol,
        metric,
        threshold,
    }
}

use Family as F;
use Metric as M;
use Tier as T;

/// The 22 badges, in page order (Mac `MilestoneCatalog.badges`).
pub const CATALOG: [Badge; 22] = [
    badge(
        "first-words",
        "First Words",
        "Your first dictation",
        F::Words,
        T::Bronze,
        "waveform",
        M::Dictations,
        1,
    ),
    badge(
        "words-1k",
        "1,000 Words",
        "About 4 pages",
        F::Words,
        T::Bronze,
        "text.alignleft",
        M::Words,
        1_000,
    ),
    badge(
        "words-5k",
        "5,000 Words",
        "A long essay",
        F::Words,
        T::Silver,
        "text.alignleft",
        M::Words,
        5_000,
    ),
    badge(
        "words-10k",
        "10,000 Words",
        "A short story",
        F::Words,
        T::Silver,
        "text.alignleft",
        M::Words,
        10_000,
    ),
    badge(
        "words-50k",
        "50,000 Words",
        "A novel draft",
        F::Words,
        T::Gold,
        "book.closed",
        M::Words,
        50_000,
    ),
    badge(
        "words-100k",
        "100,000 Words",
        "A full novel",
        F::Words,
        T::Gold,
        "books.vertical",
        M::Words,
        100_000,
    ),
    badge(
        "words-1m",
        "A Million Words",
        "A lifetime of typing",
        F::Words,
        T::Gold,
        "crown",
        M::Words,
        1_000_000,
    ),
    badge(
        "streak-3",
        "3-Day Streak",
        "3 days running",
        F::Streaks,
        T::Bronze,
        "flame",
        M::BestStreakDays,
        3,
    ),
    badge(
        "streak-7",
        "7-Day Streak",
        "A full week",
        F::Streaks,
        T::Silver,
        "flame",
        M::BestStreakDays,
        7,
    ),
    badge(
        "streak-30",
        "30-Day Streak",
        "A month of talking",
        F::Streaks,
        T::Gold,
        "flame",
        M::BestStreakDays,
        30,
    ),
    badge(
        "streak-100",
        "100-Day Streak",
        "A habit for good",
        F::Streaks,
        T::Gold,
        "flame.fill",
        M::BestStreakDays,
        100,
    ),
    badge(
        "quick-talker",
        "Quick Talker",
        "150 WPM in a dictation",
        F::SpeedAndTime,
        T::Gold,
        "bolt",
        M::BestWpm,
        150,
    ),
    badge(
        "lightning",
        "Lightning",
        "200 WPM in a dictation",
        F::SpeedAndTime,
        T::Gold,
        "bolt.fill",
        M::BestWpm,
        200,
    ),
    badge(
        "hour-saved",
        "An Hour Saved",
        "1 hour of typing back",
        F::SpeedAndTime,
        T::Bronze,
        "clock",
        M::MinutesSaved,
        60,
    ),
    badge(
        "ten-hours-saved",
        "Ten Hours Saved",
        "10 hours of typing back",
        F::SpeedAndTime,
        T::Silver,
        "clock.badge.checkmark",
        M::MinutesSaved,
        600,
    ),
    badge(
        "dictations-10",
        "Ten Dictations",
        "10 dictations",
        F::Dictations,
        T::Bronze,
        "square.stack",
        M::Dictations,
        10,
    ),
    badge(
        "dictations-50",
        "50 Dictations",
        "50 dictations",
        F::Dictations,
        T::Silver,
        "square.stack.3d.up",
        M::Dictations,
        50,
    ),
    badge(
        "dictations-500",
        "500 Dictations",
        "500 dictations",
        F::Dictations,
        T::Gold,
        "square.stack.3d.up.fill",
        M::Dictations,
        500,
    ),
    badge(
        "three-apps",
        "Three Apps",
        "Dictated in 3 apps",
        F::Special,
        T::Bronze,
        "square.grid.2x2",
        M::DistinctApps,
        3,
    ),
    badge(
        "marathon",
        "Marathon",
        "One dictation over a minute",
        F::Special,
        T::Silver,
        "stopwatch",
        M::LongestDictationSeconds,
        60,
    ),
    badge(
        "wordsmith",
        "Wordsmith",
        "10 dictionary words",
        F::Special,
        T::Silver,
        "character.book.closed",
        M::DictionaryWords,
        10,
    ),
    badge(
        "night-owl",
        "Night Owl",
        "Dictated after midnight",
        F::Special,
        T::Bronze,
        "moon",
        M::NightOwlDictations,
        1,
    ),
];

pub fn badge_with_id(id: &str) -> Option<&'static Badge> {
    CATALOG.iter().find(|b| b.id == id)
}

/// `1234567` → `"1,234,567"` (the Mac's `formatted()` in English).
pub fn fmt_int(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

// ---- Comparisons -------------------------------------------------------------

/// Turns word counts and speeds into things people can picture.
pub struct Comparisons;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Landmark {
    pub name: &'static str,
    pub metres: f64,
}

impl Landmark {
    /// To the nearest 250 words, or 1,000 past 100,000.
    pub fn words_to_reach(&self) -> i64 {
        let words = self.metres / Comparisons::METRES_PER_WORD;
        let step = if words >= 100_000.0 { 1_000.0 } else { 250.0 };
        ((words / step).round() * step) as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Book {
    pub title: &'static str,
    pub words: i64,
}

impl Comparisons {
    /// One word printed in 12-point type, with its space, is about 1.2 cm.
    pub const METRES_PER_WORD: f64 = 0.012;
    pub const WORDS_PER_PAGE: i64 = 250;
    /// An average English word is 4.7 letters, plus a space.
    pub const KEY_PRESSES_PER_WORD: f64 = 5.7;
    pub const TYPING_WPM: f64 = 40.0;
    pub const HANDWRITING_WPM: f64 = 13.0;
    pub const CONVERSATION_WPM: f64 = 150.0;
    pub const AUCTIONEER_WPM: f64 = 250.0;

    pub const LANDMARKS: [Landmark; 5] = [
        Landmark {
            name: "Statue of Liberty",
            metres: 93.0,
        },
        Landmark {
            name: "Eiffel Tower",
            metres: 330.0,
        },
        Landmark {
            name: "Burj Khalifa",
            metres: 828.0,
        },
        Landmark {
            name: "Kilimanjaro",
            metres: 5_895.0,
        },
        Landmark {
            name: "Everest",
            metres: 8_849.0,
        },
    ];

    pub const BOOKS: [Book; 3] = [
        Book {
            title: "The Great Gatsby",
            words: 47_000,
        },
        Book {
            title: "an average novel",
            words: 90_000,
        },
        Book {
            title: "War and Peace",
            words: 587_000,
        },
    ];

    pub fn height_metres(words: i64) -> f64 {
        words.max(0) as f64 * Self::METRES_PER_WORD
    }

    /// "24 m", "1.6 km".
    pub fn height_text(words: i64) -> String {
        let m = Self::height_metres(words);
        if m >= 1_000.0 {
            format!("{:.1} km", m / 1_000.0)
        } else {
            format!("{} m", m.round() as i64)
        }
    }

    pub fn last_landmark_passed(words: i64) -> Option<Landmark> {
        Self::LANDMARKS
            .iter()
            .rev()
            .find(|l| words >= l.words_to_reach())
            .copied()
    }

    pub fn next_landmark(words: i64) -> Option<Landmark> {
        Self::LANDMARKS
            .iter()
            .find(|l| words < l.words_to_reach())
            .copied()
    }

    pub fn pages(words: i64) -> i64 {
        (words.max(0) as f64 / Self::WORDS_PER_PAGE as f64).round() as i64
    }

    pub fn key_presses(words: i64) -> i64 {
        (words.max(0) as f64 * Self::KEY_PRESSES_PER_WORD).round() as i64
    }

    /// "~11,400": an estimate, so to the nearest hundred.
    pub fn key_presses_text(words: i64) -> String {
        let presses = Self::key_presses(words);
        let rounded = if presses >= 1_000 {
            ((presses as f64 / 100.0).round() as i64) * 100
        } else {
            presses
        };
        format!("~{}", fmt_int(rounded))
    }

    pub fn typing_minutes(words: i64) -> i64 {
        (words.max(0) as f64 / Self::TYPING_WPM).round() as i64
    }

    /// How many times faster than `baseline` a pace is: one decimal below 10
    /// ("4.5"), whole above it ("14").
    pub fn multiple(wpm: i64, baseline: f64) -> String {
        if wpm <= 0 {
            return "0".into();
        }
        let value = wpm as f64 / baseline;
        if value >= 10.0 {
            return (value.round() as i64).to_string();
        }
        let rounded = (value * 10.0).round() / 10.0;
        if rounded == rounded.round() {
            (rounded as i64).to_string()
        } else {
            format!("{rounded:.1}")
        }
    }

    /// The longest book the words already outrun, if any.
    pub fn longest_book_passed(words: i64) -> Option<Book> {
        Self::BOOKS.iter().rev().find(|b| words >= b.words).copied()
    }

    /// "39 min", "1 hour", "2 h 5 min", "21 hours".
    pub fn duration_text(minutes: i64) -> String {
        if minutes < 60 {
            return format!("{minutes} min");
        }
        let hours = minutes / 60;
        let rest = minutes % 60;
        if hours >= 10 || rest == 0 {
            let whole = (minutes as f64 / 60.0).round() as i64;
            return if whole == 1 {
                "1 hour".into()
            } else {
                format!("{whole} hours")
            };
        }
        format!("{hours} h {rest} min")
    }

    /// A screen-time equivalent of the time given back.
    pub fn time_back_equivalent(minutes: i64) -> String {
        match minutes {
            m if m < 15 => "a coffee break".into(),
            m if m < 75 => "a TV episode".into(),
            m if m < 180 => "a feature film".into(),
            m if m < 600 => "a working day".into(),
            m => format!("{} working days", (m / 480).max(2)),
        }
    }
}

// ---- Stats -------------------------------------------------------------------

/// What the engine needs from one dictation: counts and times, never text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MilestoneDictation {
    pub created_at: i64,
    /// Local UTC offset (seconds) at `created_at`.
    pub local_offset: i64,
    pub words: i64,
    pub recording_ms: i64,
    pub app: Option<String>,
}

impl MilestoneDictation {
    fn local_seconds(&self) -> i64 {
        self.created_at + self.local_offset
    }
    fn local_day(&self) -> i64 {
        self.local_seconds().div_euclid(86_400)
    }
    fn local_hour(&self) -> i64 {
        self.local_seconds().rem_euclid(86_400) / 3_600
    }
}

/// Running totals behind every badge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MilestoneStats {
    pub total_words: i64,
    pub dictations: i64,
    pub speaking_ms: i64,
    pub best_wpm: i64,
    pub best_streak_days: i64,
    /// Days in the streak still running today or yesterday; 0 once broken.
    pub current_streak_days: i64,
    pub distinct_apps: i64,
    pub longest_dictation_ms: i64,
    pub night_owl_dictations: i64,
    pub dictionary_words: i64,
    pub first_dictation_at: Option<i64>,
    // Derived, so the UI and share cards read the same numbers.
    pub minutes_saved: i64,
    pub speaking_minutes: i64,
    pub average_wpm: i64,
}

impl MilestoneStats {
    /// A dictation must have this many words and seconds before its pace
    /// counts, so a clipped two-word note can't set a speed record.
    pub const MIN_WORDS_FOR_PACE: i64 = 10;
    pub const MIN_MS_FOR_PACE: i64 = 3_000;
    /// Local hours that count as after midnight for Night Owl (0..<5).
    pub const NIGHT_OWL_END_HOUR: i64 = 5;

    fn refresh_derived(&mut self) {
        self.minutes_saved = crate::analytics::minutes_saved(self.total_words, self.speaking_ms);
        self.speaking_minutes = (self.speaking_ms as f64 / 60_000.0).round() as i64;
        self.average_wpm = crate::analytics::calculate_wpm(self.total_words, self.speaking_ms);
    }

    pub fn value(&self, metric: Metric) -> i64 {
        match metric {
            Metric::Words => self.total_words,
            Metric::Dictations => self.dictations,
            Metric::BestStreakDays => self.best_streak_days,
            Metric::BestWpm => self.best_wpm,
            Metric::MinutesSaved => self.minutes_saved,
            Metric::DistinctApps => self.distinct_apps,
            Metric::LongestDictationSeconds => self.longest_dictation_ms / 1_000,
            Metric::NightOwlDictations => self.night_owl_dictations,
            Metric::DictionaryWords => self.dictionary_words,
        }
    }
}

/// Stats plus when each badge's threshold was first crossed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Computed {
    pub stats: MilestoneStats,
    /// Badge id → Unix seconds of the dictation (or dictionary entry) that crossed it.
    pub crossings: BTreeMap<&'static str, i64>,
}

/// Replay dictations oldest first. `today` is the local day number of now.
pub fn compute(
    dictations: &[MilestoneDictation],
    dictionary_entry_dates: &[i64],
    badges: &[Badge],
    today: i64,
) -> Computed {
    let mut stats = MilestoneStats::default();
    let mut crossings: BTreeMap<&'static str, i64> = BTreeMap::new();
    let mut apps: HashSet<String> = HashSet::new();
    let mut last_day: Option<i64> = None;
    let mut run = 0i64;

    let mut sorted: Vec<&MilestoneDictation> = dictations.iter().collect();
    sorted.sort_by_key(|d| d.created_at);
    for d in sorted {
        stats.dictations += 1;
        stats.total_words += d.words.max(0);
        stats.speaking_ms += d.recording_ms.max(0);
        stats.longest_dictation_ms = stats.longest_dictation_ms.max(d.recording_ms);
        stats.first_dictation_at.get_or_insert(d.created_at);

        if d.words >= MilestoneStats::MIN_WORDS_FOR_PACE
            && d.recording_ms >= MilestoneStats::MIN_MS_FOR_PACE
        {
            let pace = d.words as f64 / (d.recording_ms as f64 / 60_000.0);
            stats.best_wpm = stats.best_wpm.max(pace.round() as i64);
        }
        if let Some(app) = d.app.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            apps.insert(app.to_lowercase());
            stats.distinct_apps = apps.len() as i64;
        }
        if (0..MilestoneStats::NIGHT_OWL_END_HOUR).contains(&d.local_hour()) {
            stats.night_owl_dictations += 1;
        }
        let day = d.local_day();
        match last_day {
            Some(prev) if prev != day => run = if day - prev == 1 { run + 1 } else { 1 },
            None => run = 1,
            _ => {}
        }
        last_day = Some(day);
        stats.best_streak_days = stats.best_streak_days.max(run);
        stats.refresh_derived();

        for b in badges {
            if b.metric != Metric::DictionaryWords
                && !crossings.contains_key(b.id)
                && stats.value(b.metric) >= b.threshold
            {
                crossings.insert(b.id, d.created_at);
            }
        }
    }
    if let Some(last) = last_day {
        stats.current_streak_days = if today - last <= 1 { run } else { 0 };
    }

    let mut entries = dictionary_entry_dates.to_vec();
    entries.sort_unstable();
    stats.dictionary_words = entries.len() as i64;
    for b in badges {
        if b.metric == Metric::DictionaryWords
            && b.threshold > 0
            && entries.len() as i64 >= b.threshold
        {
            crossings.insert(b.id, entries[(b.threshold - 1) as usize]);
        }
    }
    stats.refresh_derived();
    Computed { stats, crossings }
}

// ---- Copy --------------------------------------------------------------------

/// The sentence under a badge in its celebration and toast (Mac `MilestoneCopy`).
pub fn celebration_line(badge: &Badge, stats: &MilestoneStats) -> String {
    if badge.id == "first-words" {
        return "Your first dictation is in. Welcome to talking instead of typing.".into();
    }
    match badge.metric {
        Metric::Words => {
            let words = stats.total_words;
            let height = Comparisons::height_text(words);
            match Comparisons::longest_book_passed(words) {
                Some(book) => format!(
                    "{} words: longer than {}, and {height} tall stood on end.",
                    fmt_int(words),
                    book.title
                ),
                None => format!(
                    "{} words: {} pages of a book, and {height} tall stood on end.",
                    fmt_int(words),
                    fmt_int(Comparisons::pages(words))
                ),
            }
        }
        Metric::Dictations => format!(
            "{} dictations, every word spoken instead of typed.",
            fmt_int(stats.dictations)
        ),
        Metric::BestStreakDays => {
            format!("{} days in a row with a dictation.", stats.best_streak_days)
        }
        Metric::BestWpm => format!(
            "{} words a minute, {}× faster than typing.",
            stats.best_wpm,
            Comparisons::multiple(stats.best_wpm, Comparisons::TYPING_WPM)
        ),
        Metric::MinutesSaved => format!(
            "{} of typing back, with both hands free. About {}.",
            Comparisons::duration_text(stats.minutes_saved),
            Comparisons::time_back_equivalent(stats.minutes_saved)
        ),
        Metric::DistinctApps => {
            format!("Orttaai has typed for you in {} apps.", stats.distinct_apps)
        }
        Metric::LongestDictationSeconds => {
            "One dictation ran over a minute. Hands free the whole time.".into()
        }
        Metric::DictionaryWords => format!(
            "Your dictionary has {} words, so names come out your way.",
            stats.dictionary_words
        ),
        Metric::NightOwlDictations => {
            "Dictating after midnight. Your keyboard got the night off.".into()
        }
    }
}

/// "2,005 of 5,000", "39 of 60 min", "1 of 3 days".
pub fn progress_text(badge: &Badge, current: i64) -> String {
    let (c, t) = (fmt_int(current), fmt_int(badge.threshold));
    match badge.metric {
        Metric::MinutesSaved => format!("{c} of {t} min"),
        Metric::BestStreakDays => format!("{c} of {t} days"),
        Metric::BestWpm => format!("{c} of {t} WPM"),
        Metric::LongestDictationSeconds => format!("{c} of {t} s"),
        Metric::DictionaryWords => format!("{c} of {t} words"),
        Metric::DistinctApps => format!("{c} of {t} apps"),
        _ => format!("{c} of {t}"),
    }
}

// ---- State -------------------------------------------------------------------

/// A badge earned on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MilestoneUnlock {
    pub badge_id: String,
    /// When the dictation that crossed its threshold was made.
    pub unlocked_at: i64,
    /// Set once the person has seen its celebration.
    pub celebrated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BadgeState {
    Unlocked {
        at: i64,
    },
    /// Some progress: the current value and the share of the threshold (≤ 0.99).
    InProgress {
        current: i64,
        fraction: f64,
    },
    Locked,
}

pub fn state_for(
    badge: &Badge,
    stats: &MilestoneStats,
    unlocks: &BTreeMap<String, MilestoneUnlock>,
) -> BadgeState {
    if let Some(u) = unlocks.get(badge.id) {
        return BadgeState::Unlocked { at: u.unlocked_at };
    }
    let current = stats.value(badge.metric);
    if current <= 0 || badge.threshold <= 0 {
        return BadgeState::Locked;
    }
    BadgeState::InProgress {
        current,
        fraction: (current as f64 / badge.threshold as f64).min(0.99),
    }
}

fn fraction(state: BadgeState) -> f64 {
    match state {
        BadgeState::Unlocked { .. } => 1.0,
        BadgeState::InProgress { fraction, .. } => fraction,
        BadgeState::Locked => 0.0,
    }
}

/// A badge with everything the UI shows about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BadgeView {
    #[serde(flatten)]
    pub badge: Badge,
    pub state: BadgeState,
    pub current: i64,
    pub fraction: f64,
    pub unlocked_at: Option<i64>,
    pub celebrated: bool,
    pub progress_text: String,
    pub celebration_line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FamilyInfo {
    pub id: Family,
    pub title: &'static str,
    pub note: &'static str,
}

/// The Milestones page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Overview {
    pub stats: MilestoneStats,
    pub badges: Vec<BadgeView>,
    pub families: Vec<FamilyInfo>,
    /// The next words badge, else the in-progress badge closest to done.
    pub next_goal: Option<String>,
    pub next_up_detail: Option<String>,
    pub unlocked_count: usize,
    pub total: usize,
}

pub fn badge_view(
    badge: &Badge,
    stats: &MilestoneStats,
    unlocks: &BTreeMap<String, MilestoneUnlock>,
) -> BadgeView {
    let state = state_for(badge, stats, unlocks);
    let unlock = unlocks.get(badge.id);
    BadgeView {
        badge: *badge,
        state,
        current: stats.value(badge.metric),
        fraction: fraction(state),
        unlocked_at: unlock.map(|u| u.unlocked_at),
        celebrated: unlock.map(|u| u.celebrated_at.is_some()).unwrap_or(false),
        progress_text: progress_text(badge, stats.value(badge.metric)),
        celebration_line: celebration_line(badge, stats),
    }
}

pub fn next_goal(
    stats: &MilestoneStats,
    unlocks: &BTreeMap<String, MilestoneUnlock>,
) -> Option<&'static Badge> {
    let open: Vec<&'static Badge> = CATALOG
        .iter()
        .filter(|b| !unlocks.contains_key(b.id))
        .collect();
    if let Some(w) = open.iter().find(|b| b.metric == Metric::Words) {
        return Some(w);
    }
    // `max` keeps the last of equals, as Swift's `max(by:)` does.
    open.into_iter()
        .fold(None, |best: Option<&'static Badge>, b| match best {
            Some(cur)
                if fraction(state_for(b, stats, unlocks))
                    < fraction(state_for(cur, stats, unlocks)) =>
            {
                Some(cur)
            }
            _ => Some(b),
        })
}

/// The line under the next goal's progress bar.
pub fn next_up_detail(goal: &Badge, stats: &MilestoneStats) -> String {
    let current = stats.value(goal.metric);
    let mut text = progress_text(goal, current);
    if goal.metric == Metric::Words && stats.dictations > 0 {
        let per = (stats.total_words / stats.dictations).max(1);
        let remaining = (goal.threshold - current).max(0);
        let more = (remaining as f64 / per as f64).ceil() as i64;
        if more > 0 {
            text.push_str(&format!(
                " · about {} more dictation{} at your pace",
                fmt_int(more),
                if more == 1 { "" } else { "s" }
            ));
        }
    } else if current == 0 {
        text = goal.rule.to_string();
    }
    text
}

pub fn overview(stats: &MilestoneStats, unlocks: &BTreeMap<String, MilestoneUnlock>) -> Overview {
    let goal = next_goal(stats, unlocks);
    Overview {
        stats: stats.clone(),
        badges: CATALOG
            .iter()
            .map(|b| badge_view(b, stats, unlocks))
            .collect(),
        families: Family::ALL
            .iter()
            .map(|f| FamilyInfo {
                id: *f,
                title: f.title(),
                note: f.note(),
            })
            .collect(),
        next_goal: goal.map(|g| g.id.to_string()),
        next_up_detail: goal.map(|g| next_up_detail(g, stats)),
        unlocked_count: CATALOG
            .iter()
            .filter(|b| unlocks.contains_key(b.id))
            .count(),
        total: CATALOG.len(),
    }
}

/// Earned but not yet celebrated, oldest first.
pub fn awaiting_celebration(
    stats: &MilestoneStats,
    unlocks: &BTreeMap<String, MilestoneUnlock>,
) -> Vec<BadgeView> {
    let mut waiting: Vec<(&Badge, i64)> = CATALOG
        .iter()
        .filter_map(|b| {
            unlocks
                .get(b.id)
                .filter(|u| u.celebrated_at.is_none())
                .map(|u| (b, u.unlocked_at))
        })
        .collect();
    waiting.sort_by_key(|(_, at)| *at);
    waiting
        .into_iter()
        .map(|(b, _)| badge_view(b, stats, unlocks))
        .collect()
}

/// A badge earned this long before it is noticed is not "just now".
pub const FRESH_UNLOCK_WINDOW_SECS: i64 = 120;

/// The outcome of one evaluation pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub stats: MilestoneStats,
    pub unlocks: BTreeMap<String, MilestoneUnlock>,
    /// Unlocks newly written by this pass.
    pub inserted: Vec<MilestoneUnlock>,
}

impl Evaluation {
    /// Badges earned by a dictation in the last two minutes, highest tier
    /// first — the ones a toast announces.
    pub fn fresh(&self, now: i64) -> Vec<BadgeView> {
        let mut fresh: Vec<&'static Badge> = self
            .inserted
            .iter()
            .filter(|u| now - u.unlocked_at <= FRESH_UNLOCK_WINDOW_SECS)
            .filter_map(|u| badge_with_id(&u.badge_id))
            .collect();
        fresh.sort_by_key(|b| std::cmp::Reverse(b.tier));
        fresh
            .into_iter()
            .map(|b| badge_view(b, &self.stats, &self.unlocks))
            .collect()
    }

    pub fn overview(&self) -> Overview {
        overview(&self.stats, &self.unlocks)
    }

    pub fn awaiting_celebration(&self) -> Vec<BadgeView> {
        awaiting_celebration(&self.stats, &self.unlocks)
    }
}

/// Read the history, replay it, and store any badge newly earned.
pub fn evaluate(store: &Store, now: i64) -> Result<Evaluation> {
    let inputs = store.milestone_inputs()?;
    let today = (now + store.local_offset_at(now)?).div_euclid(86_400);
    let computed = compute(
        &inputs.dictations,
        &inputs.dictionary_entry_dates,
        &CATALOG,
        today,
    );
    let stored = store.milestone_unlocks()?;
    let known: HashSet<&str> = stored.iter().map(|u| u.badge_id.as_str()).collect();
    let fresh: Vec<MilestoneUnlock> = computed
        .crossings
        .iter()
        .filter(|(id, _)| !known.contains(*id))
        .map(|(id, at)| MilestoneUnlock {
            badge_id: id.to_string(),
            unlocked_at: *at,
            celebrated_at: None,
        })
        .collect();
    let inserted = store.insert_milestone_unlocks(&fresh)?;
    let mut unlocks: BTreeMap<String, MilestoneUnlock> = BTreeMap::new();
    for u in stored.into_iter().chain(inserted.iter().cloned()) {
        unlocks.entry(u.badge_id.clone()).or_insert(u);
    }
    Ok(Evaluation {
        stats: computed.stats,
        unlocks,
        inserted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    const T0: i64 = 1_773_100_800; // 2026-03-10 00:00 UTC
    const DAY: i64 = 86_400;

    fn d(at: i64, words: i64, ms: i64, app: Option<&str>) -> MilestoneDictation {
        MilestoneDictation {
            created_at: at,
            local_offset: 0,
            words,
            recording_ms: ms,
            app: app.map(String::from),
        }
    }

    #[test]
    fn catalog_is_the_mac_set() {
        assert_eq!(CATALOG.len(), 22);
        let ids: HashSet<&str> = CATALOG.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), 22);
        assert_eq!(badge_with_id("lightning").unwrap().threshold, 200);
        assert_eq!(badge_with_id("night-owl").unwrap().family, Family::Special);
        for f in Family::ALL {
            assert!(CATALOG.iter().any(|b| b.family == f));
        }
    }

    #[test]
    fn formatting_and_comparisons() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(999), "999");
        assert_eq!(fmt_int(1_000), "1,000");
        assert_eq!(fmt_int(1_234_567), "1,234,567");
        assert_eq!(fmt_int(-4_500), "-4,500");
        assert_eq!(Comparisons::height_text(2_005), "24 m");
        assert_eq!(Comparisons::height_text(150_000), "1.8 km");
        assert_eq!(Comparisons::pages(2_005), 8);
        assert_eq!(Comparisons::key_presses_text(2_005), "~11,400");
        assert_eq!(Comparisons::key_presses_text(100), "~570");
        assert_eq!(Comparisons::multiple(180, 40.0), "4.5");
        assert_eq!(Comparisons::multiple(160, 40.0), "4");
        assert_eq!(Comparisons::multiple(560, 40.0), "14");
        assert_eq!(Comparisons::multiple(0, 40.0), "0");
        assert_eq!(Comparisons::duration_text(39), "39 min");
        assert_eq!(Comparisons::duration_text(60), "1 hour");
        assert_eq!(Comparisons::duration_text(125), "2 h 5 min");
        assert_eq!(Comparisons::duration_text(1_270), "21 hours");
        assert_eq!(Comparisons::time_back_equivalent(10), "a coffee break");
        assert_eq!(Comparisons::time_back_equivalent(70), "a TV episode");
        assert_eq!(Comparisons::time_back_equivalent(1_500), "3 working days");
        assert_eq!(Comparisons::LANDMARKS[3].words_to_reach(), 491_000);
        assert_eq!(Comparisons::LANDMARKS[0].words_to_reach(), 7_750);
        assert_eq!(
            Comparisons::next_landmark(100).unwrap().name,
            "Statue of Liberty"
        );
        assert_eq!(Comparisons::last_landmark_passed(100), None);
        assert_eq!(
            Comparisons::longest_book_passed(100_000).unwrap().words,
            90_000
        );
    }

    #[test]
    fn replay_counts_streaks_pace_apps_and_night_owl() {
        let ds = vec![
            // Mar 6 02:00 local: night owl; 9 words — too short for pace.
            d(T0 - 4 * DAY + 2 * 3_600, 9, 2_000, Some("Code")),
            d(T0 - 3 * DAY + 12 * 3_600, 50, 20_000, Some("code ")), // 150 WPM
            d(T0 - 2 * DAY + 12 * 3_600, 40, 10_000, Some("Slack")), // 240 WPM
            d(T0 + 12 * 3_600, 100, 61_000, Some("Firefox")),        // gap → new run
        ];
        let c = compute(&ds, &[], &CATALOG, T0 / DAY);
        let s = &c.stats;
        assert_eq!(s.dictations, 4);
        assert_eq!(s.total_words, 199);
        assert_eq!(s.best_wpm, 240);
        assert_eq!(s.best_streak_days, 3);
        assert_eq!(s.current_streak_days, 1);
        assert_eq!(s.distinct_apps, 3);
        assert_eq!(s.night_owl_dictations, 1);
        assert_eq!(s.longest_dictation_ms, 61_000);
        assert_eq!(s.first_dictation_at, Some(T0 - 4 * DAY + 2 * 3_600));
        assert_eq!(c.crossings["first-words"], T0 - 4 * DAY + 2 * 3_600);
        assert_eq!(c.crossings["night-owl"], T0 - 4 * DAY + 2 * 3_600);
        assert_eq!(c.crossings["quick-talker"], T0 - 3 * DAY + 12 * 3_600);
        assert_eq!(c.crossings["lightning"], T0 - 2 * DAY + 12 * 3_600);
        assert_eq!(c.crossings["streak-3"], T0 - 2 * DAY + 12 * 3_600);
        assert_eq!(c.crossings["three-apps"], T0 + 12 * 3_600);
        assert_eq!(c.crossings["marathon"], T0 + 12 * 3_600);
        assert!(!c.crossings.contains_key("words-1k"));
        assert!(!c.crossings.contains_key("streak-7"));
    }

    #[test]
    fn streak_uses_local_days_and_time_saved_never_negative() {
        // 23:30 and 00:30 local (UTC+1) are two days in a row.
        let mut a = d(T0 - 30 * 60 - 3_600, 5, 1_000, None);
        a.local_offset = 3_600;
        let mut b = d(T0 + 30 * 60 - 3_600, 5, 1_000, None);
        b.local_offset = 3_600;
        let c = compute(&[a, b], &[], &CATALOG, T0 / DAY);
        assert_eq!(c.stats.best_streak_days, 2);
        // 10 words spoken over 10 minutes: typing would have taken 15 s.
        let c = compute(&[d(T0, 10, 600_000, None)], &[], &CATALOG, T0 / DAY);
        assert_eq!(c.stats.minutes_saved, 0);
        // 4,000 words in 20 min speaking: 100 typing − 20 = 80 saved.
        let c = compute(&[d(T0, 4_000, 1_200_000, None)], &[], &CATALOG, T0 / DAY);
        assert_eq!(c.stats.minutes_saved, 80);
        assert!(c.crossings.contains_key("hour-saved"));
        assert!(c.crossings.contains_key("words-1k"));
    }

    #[test]
    fn dictionary_badge_dates_from_the_tenth_entry() {
        let dates: Vec<i64> = (0..12).rev().map(|i| T0 + i * 60).collect();
        let c = compute(&[], &dates, &CATALOG, T0 / DAY);
        assert_eq!(c.stats.dictionary_words, 12);
        assert_eq!(c.crossings["wordsmith"], T0 + 9 * 60);
        let c = compute(&[], &dates[..9], &CATALOG, T0 / DAY);
        assert!(!c.crossings.contains_key("wordsmith"));
    }

    #[test]
    fn copy_reads_from_the_numbers() {
        let mut s = MilestoneStats {
            total_words: 2_005,
            dictations: 40,
            best_wpm: 180,
            best_streak_days: 7,
            distinct_apps: 3,
            dictionary_words: 10,
            ..Default::default()
        };
        s.minutes_saved = 70;
        let line = |id: &str| celebration_line(badge_with_id(id).unwrap(), &s);
        assert_eq!(
            line("first-words"),
            "Your first dictation is in. Welcome to talking instead of typing."
        );
        assert_eq!(
            line("words-1k"),
            "2,005 words: 8 pages of a book, and 24 m tall stood on end."
        );
        assert_eq!(
            line("lightning"),
            "180 words a minute, 4.5× faster than typing."
        );
        assert_eq!(line("streak-7"), "7 days in a row with a dictation.");
        assert_eq!(
            line("hour-saved"),
            "1 h 10 min of typing back, with both hands free. About a TV episode."
        );
        assert_eq!(
            line("dictations-10"),
            "40 dictations, every word spoken instead of typed."
        );
        s.total_words = 100_000;
        assert_eq!(
            celebration_line(badge_with_id("words-50k").unwrap(), &s),
            "100,000 words: longer than an average novel, and 1.2 km tall stood on end."
        );
        assert_eq!(
            progress_text(badge_with_id("words-5k").unwrap(), 2_005),
            "2,005 of 5,000"
        );
        assert_eq!(
            progress_text(badge_with_id("hour-saved").unwrap(), 39),
            "39 of 60 min"
        );
        assert_eq!(
            progress_text(badge_with_id("streak-3").unwrap(), 1),
            "1 of 3 days"
        );
    }

    #[test]
    fn states_next_goal_and_detail() {
        let s = MilestoneStats {
            total_words: 2_005,
            dictations: 20,
            ..Default::default()
        };
        let mut unlocks = BTreeMap::new();
        for id in ["first-words", "words-1k"] {
            unlocks.insert(
                id.to_string(),
                MilestoneUnlock {
                    badge_id: id.into(),
                    unlocked_at: T0,
                    celebrated_at: None,
                },
            );
        }
        let five = badge_with_id("words-5k").unwrap();
        assert_eq!(
            state_for(five, &s, &unlocks),
            BadgeState::InProgress {
                current: 2_005,
                fraction: 0.401
            }
        );
        assert_eq!(
            state_for(badge_with_id("streak-3").unwrap(), &s, &unlocks),
            BadgeState::Locked
        );
        assert_eq!(next_goal(&s, &unlocks).unwrap().id, "words-5k");
        assert_eq!(
            next_up_detail(five, &s),
            "2,005 of 5,000 · about 30 more dictations at your pace"
        );
        assert_eq!(
            next_up_detail(badge_with_id("streak-3").unwrap(), &s),
            "3 days running"
        );
        let o = overview(&s, &unlocks);
        assert_eq!((o.unlocked_count, o.total), (2, 22));
        assert_eq!(o.families.len(), 5);
        let waiting = awaiting_celebration(&s, &unlocks);
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0].badge.id, "first-words");
        // Nearly done is capped below 1 until stored as unlocked.
        let near = MilestoneStats {
            dictations: 10,
            ..Default::default()
        };
        match state_for(
            badge_with_id("dictations-10").unwrap(),
            &near,
            &BTreeMap::new(),
        ) {
            BadgeState::InProgress { fraction, .. } => assert_eq!(fraction, 0.99),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn evaluate_persists_unlocks_once_and_reports_fresh() {
        let store = Store::open_in_memory().unwrap();
        // Local noon on a fixed day, whatever this machine's time zone, so
        // Night Owl stays out of it.
        let now = T0 + 12 * 3_600 - store.local_offset_at(T0).unwrap();
        let mut old =
            TranscriptionRecord::new("one two three", Some("code".into()), 2_000, now - DAY);
        old.word_count = 3;
        store.insert_transcription(&old).unwrap();
        // Edits don't count as dictations.
        let mut edit = TranscriptionRecord::new("x ".repeat(2_000), None, 1_000, now - 30);
        edit.entry_kind = "edit".into();
        store.insert_transcription(&edit).unwrap();

        let e = evaluate(&store, now).unwrap();
        assert_eq!(e.stats.dictations, 1);
        assert_eq!(e.inserted.len(), 1);
        assert_eq!(e.inserted[0].badge_id, "first-words");
        // Earned a day ago: not fresh, so no toast — it waits for the window.
        assert!(e.fresh(now).is_empty());
        assert_eq!(e.awaiting_celebration().len(), 1);

        // A new 10-minute dictation with 1,000 words crosses words-1k and
        // Marathon now; the toast leads with the higher tier.
        let rec = TranscriptionRecord::new("w ".repeat(1_000), Some("slack".into()), 600_000, now);
        store.insert_transcription(&rec).unwrap();
        let e = evaluate(&store, now + 5).unwrap();
        let fresh: Vec<&str> = e.fresh(now + 5).iter().map(|b| b.badge.id).collect();
        assert_eq!(fresh, vec!["marathon", "words-1k"]);
        // Running again inserts nothing.
        assert!(evaluate(&store, now + 6).unwrap().inserted.is_empty());

        store
            .mark_milestones_celebrated(&["first-words".to_string()], now + 7)
            .unwrap();
        let e = evaluate(&store, now + 8).unwrap();
        let waiting: Vec<&str> = e
            .awaiting_celebration()
            .iter()
            .map(|b| b.badge.id)
            .collect();
        assert_eq!(waiting, vec!["words-1k", "marathon"]);

        // Deleting history never takes a badge back.
        store.clear_history().unwrap();
        let e = evaluate(&store, now + 9).unwrap();
        assert_eq!(e.stats.dictations, 0);
        assert!(e.unlocks.contains_key("words-1k"));
        assert!(e
            .overview()
            .badges
            .iter()
            .any(|b| b.badge.id == "words-1k" && matches!(b.state, BadgeState::Unlocked { .. })));
    }
}
