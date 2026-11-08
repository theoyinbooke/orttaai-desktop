//! What Coach keeps and reads from the store (Mac `CoachRecords`): counts
//! keyed by the dictation, habits, reps and the judge's answers. Never any
//! text of its own; quotes are read live from `transcription`.

use super::counting::{CoachDictation, DictationMeasures};
use super::measure::CoachMeasure;
use super::mode::{self, CoachMode, Overrides};
use super::scoring::CheckResult;
use super::skill::{all_skills, CoachCheck, CoachQuestion, CoachSkill};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// From schema v2 (the Mac-parity foundation) the engine keeps the
/// recognizer's text in `raw_text` whenever it differs from what was typed,
/// so a NULL `raw_text` after this moment means "the same as the text".
/// Before it, a NULL can't be told apart, so those rows only give a pace.
pub const RAW_KEPT_SINCE: i64 = 1_794_030_285;

/// A dictation waiting to be counted.
#[derive(Debug, Clone)]
pub struct CoachIndexSource {
    pub id: i64,
    pub created_at: i64,
    pub text: String,
    pub raw_text: Option<String>,
    pub recording_ms: i64,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
}

impl CoachIndexSource {
    pub fn dictation(&self) -> CoachDictation {
        CoachDictation {
            created_at: self.created_at,
            heard: self.raw_text.clone().unwrap_or_else(|| self.text.clone()),
            written: self.text.clone(),
            recording_ms: self.recording_ms,
            raw_known: self.raw_text.is_some() || self.created_at >= RAW_KEPT_SINCE,
        }
    }

    pub fn key(&self) -> Option<String> {
        mode::app_key(self.app_id.as_deref(), self.app_name.as_deref())
    }
}

/// A counted dictation with the app it went to.
#[derive(Debug, Clone)]
pub struct CoachInput {
    pub id: i64,
    pub created_at: i64,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub measures: DictationMeasures,
}

impl CoachInput {
    pub fn mode(&self, overrides: &Overrides) -> Option<CoachMode> {
        mode::mode_for(self.app_id.as_deref(), self.app_name.as_deref(), overrides)
    }
}

/// An app dictated into, as Coach lists them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachAppUsage {
    /// `mode::app_key`; "" when the app isn't known.
    pub key: String,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub dictations: i64,
    pub first_at: Option<i64>,
}

impl CoachAppUsage {
    pub fn display_name(&self) -> String {
        if let Some(name) = self
            .app_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return name.to_string();
        }
        self.app_id
            .clone()
            .unwrap_or_else(|| "Unknown app".to_string())
    }
}

/// A dictation quoted in Coach, read live so deleting it removes the quote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachQuote {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
    pub app_name: Option<String>,
}

/// What a habit tracks: a counted measure, or a judged question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoachHabitTarget {
    Measure(CoachMeasure),
    Question(CoachQuestion),
}

impl CoachHabitTarget {
    /// As kept in `coach_habit.measure`: a measure's name, or "q:" and the question's.
    pub fn stored(self) -> String {
        match self {
            Self::Measure(m) => m.id().to_string(),
            Self::Question(q) => format!("q:{}", q.id()),
        }
    }

    pub fn from_stored(stored: &str) -> Option<Self> {
        match stored.strip_prefix("q:") {
            Some(q) => CoachQuestion::from_id(q).map(Self::Question),
            None => CoachMeasure::from_id(stored).map(Self::Measure),
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Measure(m) => m.habit_title().to_string(),
            Self::Question(q) => all_skills()
                .iter()
                .find(|s| s.check == CoachCheck::Judged(q))
                .map(|s| s.title.to_string())
                .unwrap_or_else(|| q.label().to_string()),
        }
    }

    pub fn hit_title(self) -> &'static str {
        match self {
            Self::Measure(m) => m.hit_title(),
            Self::Question(q) => q.label(),
        }
    }

    pub fn measure(self) -> Option<CoachMeasure> {
        match self {
            Self::Measure(m) => Some(m),
            Self::Question(_) => None,
        }
    }

    /// The skill as a habit: its counted measure, or its judged question.
    pub fn for_skill(skill: &CoachSkill) -> Option<Self> {
        match skill.check {
            CoachCheck::Counted(super::skill::CoachCount::Measure(m)) => Some(Self::Measure(m)),
            CoachCheck::Judged(q) => Some(Self::Question(q)),
            _ => None,
        }
    }
}

impl Serialize for CoachHabitTarget {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.stored())
    }
}

impl<'de> Deserialize<'de> for CoachHabitTarget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::from_stored(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown habit target {raw}")))
    }
}

/// A habit the person started in Coach.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachHabitRecord {
    pub id: i64,
    pub measure: String,
    /// `None` is every way of talking.
    pub mode: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

impl CoachHabitRecord {
    pub fn target(&self) -> Option<CoachHabitTarget> {
        CoachHabitTarget::from_stored(&self.measure)
    }

    pub fn coach_mode(&self) -> Option<CoachMode> {
        self.mode.as_deref().and_then(CoachMode::from_id)
    }
}

/// A practice rep, as kept: numbers and check results, never what was said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachRepRecord {
    pub id: Option<i64>,
    pub skill_id: String,
    pub kind: String,
    pub spoken: bool,
    pub created_at: i64,
    pub score: Option<f64>,
    /// Whether the skill's own check was met; `None` when it couldn't be told.
    pub skill_met: Option<bool>,
    pub duration_ms: i64,
    pub words: i64,
    /// JSON of the rep's `[CheckResult]`.
    pub checks: String,
    pub persona: String,
}

impl CoachRepRecord {
    pub fn skill(&self) -> Option<&'static CoachSkill> {
        CoachSkill::by_id(&self.skill_id)
    }

    pub fn check_results(&self) -> Vec<CheckResult> {
        serde_json::from_str(&self.checks).unwrap_or_default()
    }
}
