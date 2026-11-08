//! A rep's score from its checks, the judge's one-question prompt and how its
//! reply is read, and the guard that keeps a coach's note to the facts.

use super::counting::DictationMeasures;
use super::skill::{CoachCheck, CoachCount, CoachQuestion};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JudgeAnswer {
    Yes,
    Partly,
    No,
}

impl JudgeAnswer {
    pub fn points(self) -> f64 {
        match self {
            Self::Yes => 1.0,
            Self::Partly => 0.5,
            Self::No => 0.0,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Partly => "partly",
            Self::No => "no",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "yes" => Some(Self::Yes),
            "partly" => Some(Self::Partly),
            "no" => Some(Self::No),
            _ => None,
        }
    }
}

/// How one check went. A `None` answer couldn't be told and isn't scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: CoachCheck,
    pub answer: Option<JudgeAnswer>,
    /// The judge's confidence, 0 to 1; `None` for counted checks.
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl CheckResult {
    pub fn new(check: CoachCheck, answer: Option<JudgeAnswer>, confidence: Option<f64>) -> Self {
        Self {
            check,
            answer,
            confidence,
        }
    }

    /// A counted check, from the attempt's counts.
    pub fn counted(count: CoachCount, m: &DictationMeasures) -> Self {
        Self::new(
            CoachCheck::Counted(count),
            count.is_met(m).map(|met| {
                if met {
                    JudgeAnswer::Yes
                } else {
                    JudgeAnswer::No
                }
            }),
            None,
        )
    }
}

/// A skill counts a rep towards mastery at 4 or more with its own check met.
pub const MASTERY_SCORE: f64 = 4.0;
pub const REPS_TO_MASTER: usize = 3;

/// 1 + 4 × points ÷ answered checks, to one decimal; `None` when nothing
/// could be answered.
pub fn score(results: &[CheckResult]) -> Option<f64> {
    let answered: Vec<JudgeAnswer> = results.iter().filter_map(|r| r.answer).collect();
    if answered.is_empty() {
        return None;
    }
    let points: f64 = answered.iter().map(|a| a.points()).sum();
    Some(((1.0 + 4.0 * points / answered.len() as f64) * 10.0).round() / 10.0)
}

/// The judge gets one question per call and must answer in JSON.
pub mod judge_prompt {
    use super::*;

    pub const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"answer":{"type":"string","enum":["yes","partly","no"]},"confidence":{"type":"number","minimum":0,"maximum":1}},"required":["answer","confidence"]}"#;

    pub fn for_question(question: CoachQuestion, text: &str, context: Option<&str>) -> String {
        prompt(question.question(), text, context)
    }

    pub fn prompt(question_text: &str, text: &str, context: Option<&str>) -> String {
        let mut lines: Vec<String> = vec![
            "You judge one thing about a short piece of speech that was dictated.".into(),
            "Answer only the question, strictly. Partly means it does some of it.".into(),
            String::new(),
        ];
        if let Some(context) = context.filter(|c| !c.is_empty()) {
            lines.push(format!("What the speaker was asked to do: {context}"));
            lines.push(String::new());
        }
        lines.push("The speech, between the markers:".into());
        lines.push("<<<".into());
        lines.push(text.to_string());
        lines.push(">>>".into());
        lines.push(String::new());
        lines.push(format!("Question: {question_text}"));
        lines.push(r#"Reply with JSON only: {"answer": "yes" | "partly" | "no", "confidence": a number from 0 to 1}"#.into());
        lines.join("\n")
    }

    /// The JSON object between the first `{` and the last `}`, as the Mac reads it.
    pub(crate) fn object(reply: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        let start = reply.find('{')?;
        let end = reply.rfind('}')?;
        if start >= end {
            return None;
        }
        match serde_json::from_str::<serde_json::Value>(&reply[start..=end]).ok()? {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        }
    }

    /// The answer and confidence, or `None` when the reply can't be read.
    pub fn parse(reply: &str) -> Option<(JudgeAnswer, f64)> {
        let object = object(reply)?;
        let raw = object.get("answer")?.as_str()?.to_lowercase();
        let answer = JudgeAnswer::from_id(raw.trim())?;
        let confidence = match object.get("confidence") {
            Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.5).clamp(0.0, 1.0),
            Some(serde_json::Value::Bool(b)) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Some(serde_json::Value::String(s)) => match s.parse::<f64>() {
                Ok(v) => v.clamp(0.0, 1.0),
                Err(_) => 0.5,
            },
            _ => 0.5,
        };
        Some((answer, confidence))
    }
}

/// Keeps a model's note to what Coach told it, as Orttakey's CoachWriter does.
pub mod note_guard {
    use super::*;

    pub const MAX_CHARACTERS: usize = 280;

    /// Numbers a note may use: each figure whole and to one decimal, plus a
    /// few small counting words' digits.
    pub fn allowed_numbers(figures: &[f64]) -> BTreeSet<String> {
        let mut allowed: BTreeSet<String> = ["1", "2", "3"].iter().map(|s| s.to_string()).collect();
        for &figure in figures {
            allowed.insert(format!("{}", figure.round() as i64));
            allowed.insert(format!("{figure:.1}"));
        }
        allowed
    }

    fn quotes(c: char) -> bool {
        matches!(c, '"' | '“' | '”')
    }

    pub fn cleaned(note: &str) -> String {
        note.trim().trim_matches(quotes).trim().to_string()
    }

    fn number_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?").expect("number pattern"))
    }

    pub fn accepts(note: &str, allowed: &BTreeSet<String>) -> bool {
        let note = cleaned(note);
        if note.is_empty() || note.chars().count() > MAX_CHARACTERS {
            return false;
        }
        number_re()
            .find_iter(&note)
            .all(|m| allowed.contains(m.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::measure::CoachMeasure;

    #[test]
    fn score_is_one_plus_four_times_the_share_of_points() {
        let results = vec![
            CheckResult::new(
                CoachCheck::Judged(CoachQuestion::TaskFirst),
                Some(JudgeAnswer::Yes),
                None,
            ),
            CheckResult::new(
                CoachCheck::Judged(CoachQuestion::Context),
                Some(JudgeAnswer::Partly),
                None,
            ),
            CheckResult::new(
                CoachCheck::Judged(CoachQuestion::FinishLine),
                Some(JudgeAnswer::No),
                None,
            ),
            CheckResult::new(CoachCheck::Judged(CoachQuestion::OneAsk), None, None),
        ];
        assert_eq!(score(&results), Some(3.0));
        assert_eq!(
            score(&[CheckResult::new(
                CoachCheck::Judged(CoachQuestion::Tone),
                None,
                None
            )]),
            None
        );
        assert_eq!(
            score(&[CheckResult::new(
                CoachCheck::Judged(CoachQuestion::Tone),
                Some(JudgeAnswer::Yes),
                None
            )]),
            Some(5.0)
        );
    }

    #[test]
    fn counted_results_read_the_attempt() {
        let m = DictationMeasures {
            words: 70,
            um_uh: 0,
            phrase_fillers: 0,
            wpm: None,
            sentence_lengths: vec![10],
            repeats: 1,
            polish_change_ratio: Some(0.0),
            raw_known: true,
        };
        assert_eq!(
            CheckResult::counted(CoachCount::BriefWords, &m).answer,
            Some(JudgeAnswer::No)
        );
        assert_eq!(
            CheckResult::counted(CoachCount::Measure(CoachMeasure::Fillers), &m).answer,
            Some(JudgeAnswer::Yes)
        );
        assert_eq!(
            CheckResult::counted(CoachCount::Measure(CoachMeasure::Repeats), &m).answer,
            Some(JudgeAnswer::No)
        );
        assert_eq!(
            CheckResult::counted(CoachCount::Measure(CoachMeasure::Pace), &m).answer,
            None
        );
    }

    #[test]
    fn judge_replies_are_read_strictly() {
        assert_eq!(
            judge_prompt::parse(r#"{"answer":"yes","confidence":0.92}"#),
            Some((JudgeAnswer::Yes, 0.92))
        );
        assert_eq!(
            judge_prompt::parse("Sure! {\"answer\": \"Partly\", \"confidence\": \"0.6\"} done"),
            Some((JudgeAnswer::Partly, 0.6))
        );
        assert_eq!(
            judge_prompt::parse(r#"{"answer":"no","confidence":7}"#).map(|p| p.1),
            Some(1.0)
        );
        assert_eq!(
            judge_prompt::parse(r#"{"answer":"maybe","confidence":0.5}"#),
            None
        );
        assert_eq!(judge_prompt::parse("no json here"), None);
        let prompt = judge_prompt::for_question(
            CoachQuestion::FinishLine,
            "Fix the paste.",
            Some("Redo your prompt"),
        );
        assert!(prompt.contains("<<<\nFix the paste.\n>>>"));
        assert!(prompt.contains(CoachQuestion::FinishLine.question()));
        assert!(
            judge_prompt::prompt("Was it clear?", "Hi.", None).contains("Question: Was it clear?")
        );
    }

    #[test]
    fn notes_may_only_use_numbers_they_were_given() {
        let allowed = note_guard::allowed_numbers(&[4.2, 152.0]);
        assert!(note_guard::accepts(
            "Strong rep: 4.2 and a pace of 152.",
            &allowed
        ));
        assert!(note_guard::accepts("Nice. Do it 2 more times.", &allowed));
        assert!(!note_guard::accepts("You hit 97% of checks.", &allowed));
        assert!(!note_guard::accepts(&"a".repeat(300), &allowed));
        assert!(!note_guard::accepts("  ", &allowed));
        assert_eq!(note_guard::cleaned("  “Good work.” "), "Good work.");
    }

    #[test]
    fn goal_results_round_trip() {
        let results = vec![CheckResult::new(
            CoachCheck::Goal("newcomer".into()),
            Some(JudgeAnswer::Yes),
            Some(0.8),
        )];
        let json = serde_json::to_string(&results).unwrap();
        let back: Vec<CheckResult> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].check, CoachCheck::Goal("newcomer".into()));
    }
}
