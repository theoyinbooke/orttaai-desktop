//! What there is to practise: four tracks, each a path of skills. A skill is
//! checked by counting (Delivery) or by a judge answering one yes/no
//! question; every rep in a track is scored on the track's checks, with the
//! skill's own check marked as the one that matters.

use super::counting::DictationMeasures;
use super::measure::CoachMeasure;
use super::mode::CoachMode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachTrack {
    Prompting,
    Messages,
    Delivery,
    Explaining,
}

impl CoachTrack {
    pub const ALL: [CoachTrack; 4] = [
        Self::Prompting,
        Self::Messages,
        Self::Delivery,
        Self::Explaining,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Prompting => "prompting",
            Self::Messages => "messages",
            Self::Delivery => "delivery",
            Self::Explaining => "explaining",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Prompting => "Prompting AI",
            Self::Messages => "Messages to people",
            Self::Delivery => "Delivery",
            Self::Explaining => "Explaining",
        }
    }

    /// The way of talking whose dictations it draws on, `None` for any.
    pub fn mode(self) -> Option<CoachMode> {
        match self {
            Self::Prompting => Some(CoachMode::Ai),
            Self::Messages => Some(CoachMode::People),
            Self::Delivery => None,
            Self::Explaining => Some(CoachMode::Writing),
        }
    }

    /// The track for a way of talking.
    pub fn for_mode(mode: CoachMode) -> Self {
        match mode {
            CoachMode::Ai => Self::Prompting,
            CoachMode::People => Self::Messages,
            CoachMode::Writing => Self::Explaining,
            CoachMode::Elsewhere => Self::Delivery,
        }
    }

    pub fn skills(self) -> Vec<&'static CoachSkill> {
        all_skills().iter().filter(|s| s.track == self).collect()
    }

    /// The checks every rep in the track is scored on.
    pub fn checks(self) -> Vec<CoachCheck> {
        use CoachCheck::*;
        use CoachQuestion as Q;
        match self {
            Self::Prompting => vec![
                Judged(Q::TaskFirst),
                Judged(Q::Context),
                Judged(Q::FinishLine),
                Judged(Q::OneAsk),
            ],
            Self::Messages => vec![
                Judged(Q::AskFirst),
                Judged(Q::WhenWhy),
                Judged(Q::Tone),
                Counted(CoachCount::BriefWords),
            ],
            Self::Delivery => vec![
                Counted(CoachCount::Measure(CoachMeasure::Fillers)),
                Counted(CoachCount::Measure(CoachMeasure::Phrases)),
                Counted(CoachCount::Measure(CoachMeasure::Sentences)),
                Counted(CoachCount::Measure(CoachMeasure::Repeats)),
            ],
            Self::Explaining => vec![
                Judged(Q::PointFirst),
                Judged(Q::OneIdea),
                Judged(Q::Summary),
                Judged(Q::PlainWords),
            ],
        }
    }

    /// A brief to speak or type when there's nothing of your own to redo.
    pub fn open_brief(self) -> &'static str {
        match self {
            Self::Prompting => "Brief an agent on something you're working on today.",
            Self::Messages => "Ask a teammate for something you need from them this week.",
            Self::Delivery => "Explain what you're working on and why it matters.",
            Self::Explaining => "Explain one idea from your work to someone new to it.",
        }
    }
}

/// A question a judge model answers yes, partly or no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoachQuestion {
    TaskFirst,
    Context,
    FilesAndErrors,
    FinishLine,
    OneAsk,
    Colleague,
    AskFirst,
    WhenWhy,
    Tone,
    PointFirst,
    OneIdea,
    Summary,
    PlainWords,
}

impl CoachQuestion {
    pub const ALL: [CoachQuestion; 13] = [
        Self::TaskFirst,
        Self::Context,
        Self::FilesAndErrors,
        Self::FinishLine,
        Self::OneAsk,
        Self::Colleague,
        Self::AskFirst,
        Self::WhenWhy,
        Self::Tone,
        Self::PointFirst,
        Self::OneIdea,
        Self::Summary,
        Self::PlainWords,
    ];

    /// The raw value kept in `coach_judgement.question` (the Mac's rawValue).
    pub fn id(self) -> &'static str {
        match self {
            Self::TaskFirst => "taskFirst",
            Self::Context => "context",
            Self::FilesAndErrors => "filesAndErrors",
            Self::FinishLine => "finishLine",
            Self::OneAsk => "oneAsk",
            Self::Colleague => "colleague",
            Self::AskFirst => "askFirst",
            Self::WhenWhy => "whenWhy",
            Self::Tone => "tone",
            Self::PointFirst => "pointFirst",
            Self::OneIdea => "oneIdea",
            Self::Summary => "summary",
            Self::PlainWords => "plainWords",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TaskFirst => "Task stated first",
            Self::Context => "Gives the context needed",
            Self::FilesAndErrors => "Names the file, screen or error",
            Self::FinishLine => "Says what done looks like",
            Self::OneAsk => "One request, not several",
            Self::Colleague => "A newcomer would follow it",
            Self::AskFirst => "The ask comes first",
            Self::WhenWhy => "Says when, and why it matters",
            Self::Tone => "Friendly and direct",
            Self::PointFirst => "Main point first",
            Self::OneIdea => "One idea at a time",
            Self::Summary => "Ends with a summary or next step",
            Self::PlainWords => "Plain words",
        }
    }

    /// What the judge is asked, about the text it is shown.
    pub fn question(self) -> &'static str {
        match self {
            Self::TaskFirst => "Is the task the speaker wants done stated in the first sentence?",
            Self::Context => "Does it give the context needed to do the task, such as what, where and any constraints?",
            Self::FilesAndErrors => "Does it name a specific file, screen, error message or measurement involved?",
            Self::FinishLine => "Does it say how anyone will know the task is finished, such as a test that passes or a result to check?",
            Self::OneAsk => "Does it ask for one thing, rather than several unrelated things?",
            Self::Colleague => "Would a new colleague with no background know exactly what to do from it?",
            Self::AskFirst => "Is the request or question in the first sentence?",
            Self::WhenWhy => "Does it say when the thing is needed and why it matters?",
            Self::Tone => "Is the tone friendly and direct, neither curt nor over-apologetic?",
            Self::PointFirst => "Does the first sentence state the main point?",
            Self::OneIdea => "Does each sentence carry a single idea?",
            Self::Summary => "Does it end with a short summary or a next step?",
            Self::PlainWords => "Is it in plain words a newcomer would follow, without unexplained jargon?",
        }
    }

    /// The voice shape's axis label.
    pub fn axis_title(self) -> &'static str {
        match self {
            Self::TaskFirst => "Task first",
            Self::Context => "Context",
            Self::FilesAndErrors => "Specifics",
            Self::FinishLine => "Finish line",
            Self::OneAsk => "One ask",
            Self::Colleague => "Easy to follow",
            Self::AskFirst => "Ask first",
            Self::WhenWhy => "When and why",
            Self::Tone => "Tone",
            Self::PointFirst => "Point first",
            Self::OneIdea => "One idea",
            Self::Summary => "Summary",
            Self::PlainWords => "Plain words",
        }
    }
}

/// A check that's counted, not judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoachCount {
    Measure(CoachMeasure),
    /// 60 words or fewer.
    BriefWords,
}

impl CoachCount {
    pub fn label(self) -> &'static str {
        match self {
            Self::Measure(m) => m.hit_title(),
            Self::BriefWords => "60 words or fewer",
        }
    }

    /// Whether the attempt meets it; `None` when it can't be told (typed pace).
    pub fn is_met(self, m: &DictationMeasures) -> Option<bool> {
        match self {
            Self::Measure(measure) => measure.is_hit(m),
            Self::BriefWords => Some(m.words <= 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoachCheck {
    Judged(CoachQuestion),
    Counted(CoachCount),
    /// Whether a scene's goal was reached, by the scene's id.
    Goal(String),
}

impl CoachCheck {
    pub fn id(&self) -> String {
        match self {
            Self::Judged(q) => format!("q-{}", q.id()),
            Self::Counted(CoachCount::Measure(m)) => format!("m-{}", m.id()),
            Self::Counted(CoachCount::BriefWords) => "m-brief".to_string(),
            Self::Goal(scene) => format!("g-{scene}"),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        if let Some(q) = id.strip_prefix("q-") {
            return CoachQuestion::from_id(q).map(Self::Judged);
        }
        if id == "m-brief" {
            return Some(Self::Counted(CoachCount::BriefWords));
        }
        if let Some(m) = id.strip_prefix("m-") {
            return CoachMeasure::from_id(m).map(|m| Self::Counted(CoachCount::Measure(m)));
        }
        id.strip_prefix("g-").map(|s| Self::Goal(s.to_string()))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Judged(q) => q.label(),
            Self::Counted(c) => c.label(),
            Self::Goal(_) => "Reached the goal",
        }
    }

    pub fn is_judged(&self) -> bool {
        matches!(self, Self::Judged(_))
    }
}

/// Checks travel (and are stored in `coach_rep.checks`) by their id.
impl Serialize for CoachCheck {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.id())
    }
}

impl<'de> Deserialize<'de> for CoachCheck {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let id = String::deserialize(d)?;
        Self::from_id(&id).ok_or_else(|| serde::de::Error::custom(format!("unknown check {id}")))
    }
}

/// How a rep is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachRepKind {
    /// Say one of your own dictations again, better.
    Redo,
    /// Meet one constraint.
    Drill,
    /// A minute to brief or explain.
    Timed,
    /// A roleplay scene.
    Scene,
}

impl CoachRepKind {
    pub const ALL: [CoachRepKind; 4] = [Self::Redo, Self::Drill, Self::Timed, Self::Scene];
    /// The kinds a skill offers on its own; scenes start from Studio.
    pub const PRACTICE: [CoachRepKind; 3] = [Self::Redo, Self::Drill, Self::Timed];

    pub fn id(self) -> &'static str {
        match self {
            Self::Redo => "redo",
            Self::Drill => "drill",
            Self::Timed => "timed",
            Self::Scene => "scene",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Redo => "Redo one of yours",
            Self::Drill => "Drill",
            Self::Timed => "60-second brief",
            Self::Scene => "Scene",
        }
    }

    pub fn time_limit_seconds(self) -> i64 {
        match self {
            Self::Redo => 90,
            Self::Drill => 60,
            Self::Timed => 60,
            Self::Scene => 90,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoachSkill {
    pub id: &'static str,
    pub track: CoachTrack,
    pub title: &'static str,
    /// The check that matters most for this skill.
    pub check: CoachCheck,
    /// One line on why, shown on the path.
    pub why: &'static str,
    /// The constraint a drill sets.
    pub drill: &'static str,
}

impl CoachSkill {
    /// Delivery skills map to a counted measure, so they can become habits.
    pub fn measure(&self) -> Option<CoachMeasure> {
        match self.check {
            CoachCheck::Counted(CoachCount::Measure(m)) => Some(m),
            _ => None,
        }
    }

    /// Pace needs a voice.
    pub fn speak_only(&self) -> bool {
        self.measure() == Some(CoachMeasure::Pace)
    }

    pub fn checks(&self) -> Vec<CoachCheck> {
        let mut all = self.track.checks();
        if !all.contains(&self.check) {
            all.insert(0, self.check.clone());
        }
        all
    }

    pub fn by_id(id: &str) -> Option<&'static CoachSkill> {
        all_skills().iter().find(|s| s.id == id)
    }
}

pub fn all_skills() -> &'static [CoachSkill] {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<CoachSkill>> = OnceLock::new();
    ALL.get_or_init(|| {
        use CoachCheck::*;
        use CoachQuestion as Q;
        use CoachTrack::*;
        let s = |id, track, title, check, why, drill| CoachSkill {
            id,
            track,
            title,
            check,
            why,
            drill,
        };
        let m = |m| Counted(CoachCount::Measure(m));
        vec![
            // Prompting AI.
            s(
                "task-first",
                Prompting,
                "Task first",
                Judged(Q::TaskFirst),
                "Agents act on the first thing you say.",
                "Say the task in your first sentence, then everything else.",
            ),
            s(
                "context",
                Prompting,
                "Give the context it needs",
                Judged(Q::Context),
                "What, where and any limits save a round of questions.",
                "Give the task, then where it lives and one constraint.",
            ),
            s(
                "files-errors",
                Prompting,
                "Name the files and errors",
                Judged(Q::FilesAndErrors),
                "A file name or the exact error beats a description.",
                "Name one file and one error or measurement.",
            ),
            s(
                "finish-line",
                Prompting,
                "Say what done looks like",
                Judged(Q::FinishLine),
                "Agents stop sooner and ask less when they know the finish line.",
                "End with how you'll know it's finished.",
            ),
            s(
                "one-ask",
                Prompting,
                "One request per prompt",
                Judged(Q::OneAsk),
                "One clear ask gets one clear result.",
                "Ask for one thing only.",
            ),
            s(
                "colleague",
                Prompting,
                "Pass the colleague test",
                Judged(Q::Colleague),
                "If someone new could follow it, an agent can.",
                "Brief it as if to someone who joined today.",
            ),
            // Messages to people.
            s(
                "ask-first",
                Messages,
                "Ask first",
                Judged(Q::AskFirst),
                "People read the first line, then decide.",
                "Put the ask in your first sentence.",
            ),
            s(
                "keep-short",
                Messages,
                "Keep it short",
                Counted(CoachCount::BriefWords),
                "Short messages get answered sooner.",
                "Three sentences, 60 words at most.",
            ),
            s(
                "when-why",
                Messages,
                "Say when and why",
                Judged(Q::WhenWhy),
                "A deadline and a reason make it easy to say yes.",
                "Say when you need it and why it matters.",
            ),
            s(
                "tone",
                Messages,
                "A tone that fits",
                Judged(Q::Tone),
                "Friendly and direct beats curt or over-apologetic.",
                "Ask warmly, without apologising.",
            ),
            // Delivery.
            s(
                "fewer-umuh",
                Delivery,
                "Fewer ums and uhs",
                m(CoachMeasure::Fillers),
                "A silent pause reads better, and writes nothing.",
                "Say it with no um or uh. Pause instead.",
            ),
            s(
                "fewer-phrases",
                Delivery,
                "Fewer filler phrases",
                m(CoachMeasure::Phrases),
                "\"Basically\" and \"you know\" fill time, not meaning.",
                "No \"basically\", \"actually\" or \"you know\".",
            ),
            s(
                "short-sentences",
                Delivery,
                "Shorter sentences",
                m(CoachMeasure::Sentences),
                "Short sentences give Polish clear places to stop.",
                "Sentences of 14 words or fewer.",
            ),
            s(
                "steady-pace",
                Delivery,
                "A steadier pace",
                m(CoachMeasure::Pace),
                "Names and numbers come out cleaner nearer 140 words a minute.",
                "Stay at 160 words a minute or slower.",
            ),
            s(
                "no-repeats",
                Delivery,
                "No words said twice",
                m(CoachMeasure::Repeats),
                "Starting a sentence a beat slower stops the stutter.",
                "Say each word once.",
            ),
            // Explaining.
            s(
                "point-first",
                Explaining,
                "Point first",
                Judged(Q::PointFirst),
                "Lead with the point; the detail follows it.",
                "Open with the main point.",
            ),
            s(
                "one-idea",
                Explaining,
                "One idea at a time",
                Judged(Q::OneIdea),
                "One idea per sentence is easy to follow.",
                "One idea per sentence.",
            ),
            s(
                "sum-up",
                Explaining,
                "Sum it up",
                Judged(Q::Summary),
                "A last line that sums up is what people remember.",
                "End with a one-line summary or next step.",
            ),
            s(
                "plain-words",
                Explaining,
                "Plain words",
                Judged(Q::PlainWords),
                "Plain words travel further than jargon.",
                "Explain it without jargon.",
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_skill_belongs_to_its_track_and_is_checked() {
        let ids: HashSet<_> = all_skills().iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), all_skills().len());
        for track in CoachTrack::ALL {
            assert!(!track.skills().is_empty());
            assert_eq!(track.checks().len(), 4);
        }
        for skill in all_skills() {
            assert!(skill.checks().contains(&skill.check));
            assert!(
                skill.checks()[0] == skill.check || skill.track.checks().contains(&skill.check)
            );
        }
        assert!(CoachSkill::by_id("steady-pace").unwrap().speak_only());
        assert_eq!(
            CoachSkill::by_id("fewer-umuh").unwrap().measure(),
            Some(CoachMeasure::Fillers)
        );
        assert_eq!(CoachSkill::by_id("finish-line").unwrap().measure(), None);
        assert_eq!(
            CoachTrack::for_mode(CoachMode::People),
            CoachTrack::Messages
        );
    }

    #[test]
    fn checks_round_trip_by_id() {
        for check in [
            CoachCheck::Judged(CoachQuestion::FinishLine),
            CoachCheck::Counted(CoachCount::BriefWords),
            CoachCheck::Counted(CoachCount::Measure(CoachMeasure::Pace)),
            CoachCheck::Goal("newcomer".into()),
        ] {
            let json = serde_json::to_string(&check).unwrap();
            assert_eq!(serde_json::from_str::<CoachCheck>(&json).unwrap(), check);
        }
        assert_eq!(CoachCheck::Goal("newcomer".into()).id(), "g-newcomer");
        assert_eq!(
            CoachCheck::Goal("newcomer".into()).label(),
            "Reached the goal"
        );
    }

    #[test]
    fn counted_checks_read_the_attempt() {
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
        assert_eq!(CoachCount::BriefWords.is_met(&m), Some(false));
        assert_eq!(
            CoachCount::Measure(CoachMeasure::Fillers).is_met(&m),
            Some(true)
        );
        assert_eq!(
            CoachCount::Measure(CoachMeasure::Repeats).is_met(&m),
            Some(false)
        );
        // A typed attempt has no pace: it can't be told.
        assert_eq!(CoachCount::Measure(CoachMeasure::Pace).is_met(&m), None);
    }
}
