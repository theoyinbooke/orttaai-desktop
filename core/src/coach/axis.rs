//! An axis of the voice shape: a counted measure, or a question the judge
//! answers about each dictation. Each way of talking has its own six; the
//! all-modes shape keeps the six counted ones so it compares across them.

use super::measure::CoachMeasure;
use super::mode::CoachMode;
use super::skill::CoachQuestion;
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CoachAxis {
    Measure(CoachMeasure),
    Question(CoachQuestion),
}

impl CoachAxis {
    pub fn id(self) -> String {
        match self {
            Self::Measure(m) => m.id().to_string(),
            Self::Question(q) => format!("q:{}", q.id()),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Measure(m) => m.axis_title(),
            Self::Question(q) => q.axis_title(),
        }
    }

    pub fn is_judged(self) -> bool {
        matches!(self, Self::Question(_))
    }

    pub fn question(self) -> Option<CoachQuestion> {
        match self {
            Self::Question(q) => Some(q),
            Self::Measure(_) => None,
        }
    }

    /// The six axes for a way of talking, judged first. `None`, and
    /// "elsewhere", get the six counted measures.
    pub fn axes(mode: Option<CoachMode>) -> Vec<CoachAxis> {
        use CoachMeasure as M;
        use CoachQuestion as Q;
        let (q, m) = (Self::Question, Self::Measure);
        match mode {
            Some(CoachMode::Ai) => vec![
                q(Q::TaskFirst),
                q(Q::Context),
                q(Q::FinishLine),
                q(Q::OneAsk),
                m(M::Clarity),
                m(M::Pace),
            ],
            Some(CoachMode::People) => vec![
                q(Q::AskFirst),
                q(Q::WhenWhy),
                q(Q::Tone),
                m(M::Sentences),
                m(M::Clarity),
                m(M::Pace),
            ],
            Some(CoachMode::Writing) => vec![
                q(Q::PointFirst),
                q(Q::OneIdea),
                q(Q::Summary),
                q(Q::PlainWords),
                m(M::Fillers),
                m(M::Sentences),
            ],
            Some(CoachMode::Elsewhere) | None => CoachMeasure::ALL.into_iter().map(m).collect(),
        }
    }

    /// The judged questions on a way of talking's shape.
    pub fn questions(mode: Option<CoachMode>) -> Vec<CoachQuestion> {
        Self::axes(mode)
            .into_iter()
            .filter_map(Self::question)
            .collect()
    }
}

impl Serialize for CoachAxis {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn each_way_of_talking_has_six_axes_from_the_design() {
        let titles: Vec<_> = CoachAxis::axes(Some(CoachMode::Ai))
            .into_iter()
            .map(CoachAxis::title)
            .collect();
        assert_eq!(
            titles,
            [
                "Task first",
                "Context",
                "Finish line",
                "One ask",
                "Said it clearly",
                "Steady pace"
            ]
        );
        assert_eq!(
            CoachAxis::questions(Some(CoachMode::People)),
            vec![
                CoachQuestion::AskFirst,
                CoachQuestion::WhenWhy,
                CoachQuestion::Tone
            ]
        );
        assert_eq!(
            CoachAxis::questions(Some(CoachMode::Writing)),
            vec![
                CoachQuestion::PointFirst,
                CoachQuestion::OneIdea,
                CoachQuestion::Summary,
                CoachQuestion::PlainWords
            ]
        );
        assert_eq!(
            CoachAxis::axes(None),
            CoachMeasure::ALL
                .into_iter()
                .map(CoachAxis::Measure)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            CoachAxis::axes(Some(CoachMode::Elsewhere)),
            CoachAxis::axes(None)
        );
        for mode in CoachMode::ALL {
            let axes = CoachAxis::axes(Some(mode));
            assert_eq!(axes.len(), 6);
            assert_eq!(axes.iter().collect::<HashSet<_>>().len(), 6);
        }
    }
}
