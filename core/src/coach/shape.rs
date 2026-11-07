//! The voice shape: six axes from 0 to 1 for the last 28 days, drawn over the
//! 28 days before. Counted axes come from the dictations' counts; judged
//! axes from the judge's answers about them.

use super::axis::CoachAxis;
use super::calendar::{CoachCalendar, Interval};
use super::counting::DictationMeasures;
use super::measure::{CoachMeasure, CoachSummary};
use super::mode::CoachMode;
use super::skill::CoachQuestion;
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};

/// A dictation's counts with when and how it was said.
#[derive(Debug, Clone)]
pub struct DatedMeasures {
    pub id: i64,
    pub created_at: i64,
    pub mode: CoachMode,
    pub measures: DictationMeasures,
}

/// Judge answers as points (yes 1, partly 0.5, no 0), by question and dictation.
pub type Judgements = HashMap<CoachQuestion, HashMap<i64, f64>>;

pub const DAYS: i64 = 28;

#[derive(Debug, Clone, PartialEq)]
pub struct CoachShape {
    /// `None` is every mode together.
    pub mode: Option<CoachMode>,
    /// The six axes drawn for this mode.
    pub axes: Vec<CoachAxis>,
    /// Axes with enough dictations to draw.
    pub now_axes: BTreeMap<CoachAxis, f64>,
    pub then_axes: BTreeMap<CoachAxis, f64>,
    /// Every counted measure, drawn or not, for picking a focus.
    pub now: BTreeMap<CoachMeasure, f64>,
    pub then: BTreeMap<CoachMeasure, f64>,
    pub now_summary: CoachSummary,
    pub then_summary: CoachSummary,
    /// The axis that grew most, when one grew by at least 3 points.
    pub biggest_gain: Option<CoachAxis>,
}

impl CoachShape {
    pub fn is_complete(&self) -> bool {
        self.axes.iter().all(|a| self.now_axes.contains_key(a))
    }

    pub fn then_is_complete(&self) -> bool {
        self.axes.iter().all(|a| self.then_axes.contains_key(a))
    }

    /// Only judged axes are still missing.
    pub fn waits_on_judge(&self) -> bool {
        !self.is_complete()
            && self
                .axes
                .iter()
                .all(|a| self.now_axes.contains_key(a) || a.is_judged())
    }

    /// Each axis's change from then to now, largest first, where both are known.
    pub fn changes(&self) -> Vec<(CoachAxis, f64)> {
        let mut changes: Vec<(CoachAxis, f64)> = self
            .axes
            .iter()
            .filter_map(|a| Some((*a, self.now_axes.get(a)? - self.then_axes.get(a)?)))
            .collect();
        changes.sort_by(|a, b| {
            b.1.abs()
                .partial_cmp(&a.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        changes
    }

    /// The last 28 days against the 28 before.
    pub fn make(
        samples: &[DatedMeasures],
        mode: Option<CoachMode>,
        now: i64,
        judgements: &Judgements,
        cal: &CoachCalendar,
    ) -> Self {
        let start = cal.add_days(now, -DAYS);
        let earlier = cal.add_days(now, -DAYS * 2);
        Self::make_window(
            samples,
            mode,
            Interval::new(start, now),
            Interval::new(earlier, start),
            judgements,
        )
    }

    /// Any window against the one before it, such as a month and the last.
    /// A dictation counts in a window when start <= created_at < end, except
    /// that the current window also takes one made at its end.
    pub fn make_window(
        samples: &[DatedMeasures],
        mode: Option<CoachMode>,
        window: Interval,
        before: Interval,
        judgements: &Judgements,
    ) -> Self {
        let mine: Vec<&DatedMeasures> = samples
            .iter()
            .filter(|s| mode.is_none_or(|m| s.mode == m))
            .collect();
        let recent: Vec<&DatedMeasures> = mine
            .iter()
            .copied()
            .filter(|s| s.created_at >= window.start && s.created_at <= window.end)
            .collect();
        let older: Vec<&DatedMeasures> = mine
            .iter()
            .copied()
            .filter(|s| s.created_at >= before.start && s.created_at < before.end)
            .collect();
        let now_summary = CoachSummary::new(recent.iter().map(|s| &s.measures));
        let then_summary = CoachSummary::new(older.iter().map(|s| &s.measures));
        let mut now = BTreeMap::new();
        let mut then = BTreeMap::new();
        for measure in CoachMeasure::ALL {
            if let Some(v) = measure.axis_score(&now_summary) {
                now.insert(measure, v);
            }
            if let Some(v) = measure.axis_score(&then_summary) {
                then.insert(measure, v);
            }
        }

        let axes = CoachAxis::axes(mode);
        let mut now_axes = BTreeMap::new();
        let mut then_axes = BTreeMap::new();
        let empty = HashMap::new();
        for axis in &axes {
            let (n, t) = match axis {
                CoachAxis::Measure(m) => (now.get(m).copied(), then.get(m).copied()),
                CoachAxis::Question(q) => {
                    let answers = judgements.get(q).unwrap_or(&empty);
                    (
                        judged_score(&recent, answers),
                        judged_score(&older, answers),
                    )
                }
            };
            if let Some(n) = n {
                now_axes.insert(*axis, n);
            }
            if let Some(t) = t {
                then_axes.insert(*axis, t);
            }
        }

        // The largest gain; on a tie the first axis wins, like Swift's `max(by:)`.
        let mut gain: Option<(CoachAxis, f64)> = None;
        for axis in &axes {
            if let (Some(n), Some(t)) = (now_axes.get(axis), then_axes.get(axis)) {
                let change = n - t;
                if change >= 0.03 && gain.is_none_or(|(_, best)| change > best) {
                    gain = Some((*axis, change));
                }
            }
        }
        Self {
            mode,
            axes,
            now_axes,
            then_axes,
            now,
            then,
            now_summary,
            then_summary,
            biggest_gain: gain.map(|g| g.0),
        }
    }
}

/// The average answer over the judged dictations, once there are enough.
fn judged_score(samples: &[&DatedMeasures], answers: &HashMap<i64, f64>) -> Option<f64> {
    let scores: Vec<f64> = samples
        .iter()
        .filter_map(|s| answers.get(&s.id).copied())
        .collect();
    if scores.len() < CoachMeasure::MINIMUM_SAMPLES {
        return None;
    }
    Some(scores.iter().sum::<f64>() / scores.len() as f64)
}

struct AxisRow<'a> {
    axis: CoachAxis,
    shape: &'a CoachShape,
}

impl Serialize for AxisRow<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Axis", 5)?;
        st.serialize_field("id", &self.axis.id())?;
        st.serialize_field("title", self.axis.title())?;
        st.serialize_field("judged", &self.axis.is_judged())?;
        st.serialize_field("now", &self.shape.now_axes.get(&self.axis))?;
        st.serialize_field("then", &self.shape.then_axes.get(&self.axis))?;
        st.end()
    }
}

struct MeasureMap<'a>(&'a BTreeMap<CoachMeasure, f64>);

impl Serialize for MeasureMap<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k.id(), v)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct Change {
    axis: String,
    change: f64,
}

impl Serialize for CoachShape {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("CoachShape", 11)?;
        st.serialize_field("mode", &self.mode)?;
        let rows: Vec<AxisRow> = self
            .axes
            .iter()
            .map(|&axis| AxisRow { axis, shape: self })
            .collect();
        st.serialize_field("axes", &rows)?;
        st.serialize_field("now", &MeasureMap(&self.now))?;
        st.serialize_field("then", &MeasureMap(&self.then))?;
        st.serialize_field("now_summary", &self.now_summary)?;
        st.serialize_field("then_summary", &self.then_summary)?;
        st.serialize_field("biggest_gain", &self.biggest_gain.map(CoachAxis::id))?;
        st.serialize_field("is_complete", &self.is_complete())?;
        st.serialize_field("then_is_complete", &self.then_is_complete())?;
        st.serialize_field("waits_on_judge", &self.waits_on_judge())?;
        let changes: Vec<Change> = self
            .changes()
            .into_iter()
            .map(|(a, change)| Change {
                axis: a.id(),
                change,
            })
            .collect();
        st.serialize_field("changes", &changes)?;
        st.end()
    }
}

/// Which dictations to ask the judge about so the judged axes fill in: up to
/// `PER_WINDOW` per axis in the last 28 days, then in the 28 before, the
/// busiest way of talking first, axes taking turns.
pub mod judging {
    use super::*;

    pub const PER_WINDOW: usize = 12;
    /// Shorter dictations aren't worth a judge's time.
    pub const MINIMUM_WORDS: i64 = 8;

    pub fn queue(
        samples: &[DatedMeasures],
        modes: &[CoachMode],
        known: &Judgements,
        now: i64,
        cal: &CoachCalendar,
    ) -> Vec<(CoachQuestion, i64)> {
        let start = cal.add_days(now, -DAYS);
        let earlier = cal.add_days(now, -DAYS * 2);
        // The same windows as the shape: the latest includes its end.
        let windows: [Box<dyn Fn(i64) -> bool>; 2] = [
            Box::new(move |t| t >= start && t <= now),
            Box::new(move |t| t >= earlier && t < start),
        ];
        let mut newest_first: Vec<&DatedMeasures> = samples
            .iter()
            .filter(|s| s.measures.words >= MINIMUM_WORDS)
            .collect();
        newest_first.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        let empty = HashMap::new();
        let mut queue = Vec::new();
        for in_window in &windows {
            for &mode in modes {
                let candidates: Vec<&&DatedMeasures> = newest_first
                    .iter()
                    .filter(|s| s.mode == mode && in_window(s.created_at))
                    .collect();
                let lists: Vec<Vec<(CoachQuestion, i64)>> = CoachAxis::questions(Some(mode))
                    .into_iter()
                    .map(|question| {
                        let answers = known.get(&question).unwrap_or(&empty);
                        let have = candidates
                            .iter()
                            .filter(|s| answers.contains_key(&s.id))
                            .count();
                        candidates
                            .iter()
                            .filter(|s| !answers.contains_key(&s.id))
                            .take(PER_WINDOW.saturating_sub(have))
                            .map(|s| (question, s.id))
                            .collect()
                    })
                    .collect();
                let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
                for index in 0..longest {
                    for list in &lists {
                        if let Some(item) = list.get(index) {
                            queue.push(*item);
                        }
                    }
                }
            }
        }
        queue
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coach::measure::measures;

    pub const NOW: i64 = 1_800_000_000;
    pub const CAL: CoachCalendar = CoachCalendar::Fixed(0);

    pub fn sample(id: i64, days_ago: f64, mode: CoachMode, words: i64) -> DatedMeasures {
        DatedMeasures {
            id,
            created_at: NOW - (days_ago * 86_400.0) as i64,
            mode,
            measures: measures(words, 0, 0, Some(145), &[10, 12], 0, Some(0.05), true),
        }
    }

    #[test]
    fn a_judged_axis_averages_the_answers_once_there_are_eight() {
        let mut samples = Vec::new();
        let mut answers = HashMap::new();
        for i in 0..10 {
            samples.push(sample(i, i as f64, CoachMode::Ai, 40));
            samples.push(sample(100 + i, (30 + i) as f64, CoachMode::Ai, 40));
            if i < 6 {
                answers.insert(i, 1.0);
            } else if i < 8 {
                answers.insert(i, 0.5);
            }
            if i < 7 {
                answers.insert(100 + i, 0.0);
            }
        }
        let judgements: Judgements = [(CoachQuestion::TaskFirst, answers)].into_iter().collect();
        let shape = CoachShape::make(&samples, Some(CoachMode::Ai), NOW, &judgements, &CAL);
        assert_eq!(
            shape
                .now_axes
                .get(&CoachAxis::Question(CoachQuestion::TaskFirst)),
            Some(&(7.0 / 8.0))
        );
        assert_eq!(
            shape
                .then_axes
                .get(&CoachAxis::Question(CoachQuestion::TaskFirst)),
            None
        );
        assert_eq!(
            shape
                .now_axes
                .get(&CoachAxis::Question(CoachQuestion::Context)),
            None
        );
        assert!(shape
            .now_axes
            .contains_key(&CoachAxis::Measure(CoachMeasure::Pace)));
        assert_eq!(shape.now.len(), CoachMeasure::ALL.len());
        assert!(!shape.is_complete());
        assert!(shape.waits_on_judge());
        assert_eq!(shape.biggest_gain, None);
    }

    #[test]
    fn the_biggest_gain_can_be_a_judged_axis() {
        let mut samples = Vec::new();
        let mut finish = HashMap::new();
        for i in 0..10 {
            samples.push(sample(i, i as f64, CoachMode::Ai, 40));
            samples.push(sample(100 + i, (30 + i) as f64, CoachMode::Ai, 40));
            finish.insert(i, if i < 6 { 1.0 } else { 0.0 });
            finish.insert(100 + i, if i < 3 { 1.0 } else { 0.0 });
        }
        let judgements: Judgements = [(CoachQuestion::FinishLine, finish)].into_iter().collect();
        let shape = CoachShape::make(&samples, Some(CoachMode::Ai), NOW, &judgements, &CAL);
        let axis = CoachAxis::Question(CoachQuestion::FinishLine);
        assert_eq!(shape.biggest_gain, Some(axis));
        assert_eq!(shape.changes()[0].0, axis);
        assert!((shape.changes()[0].1 - 0.3).abs() < 0.0001);
    }

    #[test]
    fn a_window_can_be_any_interval_against_the_one_before() {
        let month = CAL.month(NOW);
        let previous = Interval::new(CAL.add_months(month.start, -1), month.start);
        let mut samples = Vec::new();
        for i in 0..9 {
            samples.push(DatedMeasures {
                id: i,
                created_at: month.start + i * 3_600,
                mode: CoachMode::Ai,
                measures: measures(100, 1, 0, Some(145), &[10], 0, Some(0.05), true),
            });
            samples.push(DatedMeasures {
                id: 50 + i,
                created_at: previous.start + i * 3_600,
                mode: CoachMode::Ai,
                measures: measures(100, 4, 0, Some(145), &[10], 0, Some(0.05), true),
            });
        }
        let shape = CoachShape::make_window(&samples, None, month, previous, &Judgements::new());
        assert_eq!(shape.now_summary.dictations, 9);
        assert_eq!(shape.then_summary.dictations, 9);
        assert_eq!(
            shape.biggest_gain,
            Some(CoachAxis::Measure(CoachMeasure::Fillers))
        );
    }

    #[test]
    fn shape_compares_the_last_28_days_with_the_28_before() {
        let mut samples = Vec::new();
        for day in 0..10 {
            let recent = NOW - day * 86_400;
            let older = NOW - (day + 30) * 86_400;
            let m = |um| measures(100, um, 0, Some(145), &[10, 12], 0, Some(0.05), true);
            samples.push(DatedMeasures {
                id: day,
                created_at: recent,
                mode: CoachMode::Writing,
                measures: m(1),
            });
            samples.push(DatedMeasures {
                id: day + 100,
                created_at: older,
                mode: CoachMode::Writing,
                measures: m(4),
            });
            samples.push(DatedMeasures {
                id: day + 200,
                created_at: recent,
                mode: CoachMode::People,
                measures: m(5),
            });
        }
        let writing = CoachShape::make(
            &samples,
            Some(CoachMode::Writing),
            NOW,
            &Judgements::new(),
            &CAL,
        );
        assert!((writing.now[&CoachMeasure::Fillers] - 0.8).abs() < 0.0001);
        assert!((writing.then[&CoachMeasure::Fillers] - 0.2).abs() < 0.0001);
        assert_eq!(
            writing.biggest_gain,
            Some(CoachAxis::Measure(CoachMeasure::Fillers))
        );
        let people = CoachShape::make(
            &samples,
            Some(CoachMode::People),
            NOW,
            &Judgements::new(),
            &CAL,
        );
        assert_eq!(people.then.get(&CoachMeasure::Fillers), None);
        assert_eq!(people.biggest_gain, None);
        let json = serde_json::to_value(&writing).unwrap();
        assert_eq!(json["axes"].as_array().unwrap().len(), 6);
        assert_eq!(json["biggest_gain"], "fillers");
    }

    #[test]
    fn judging_fills_the_latest_window_first_with_axes_taking_turns() {
        let mut samples = Vec::new();
        for i in 0..15 {
            samples.push(sample(i, i as f64, CoachMode::Ai, 40));
            samples.push(sample(100 + i, (30 + i) as f64, CoachMode::Ai, 40));
        }
        samples.push(sample(500, 1.0, CoachMode::Ai, 3));
        samples.push(sample(600, 1.0, CoachMode::People, 40));
        let known: Judgements = [(
            CoachQuestion::TaskFirst,
            (0..12).map(|i| (i, 1.0)).collect(),
        )]
        .into_iter()
        .collect();
        let queue = judging::queue(
            &samples,
            &[CoachMode::Ai, CoachMode::People],
            &known,
            NOW,
            &CAL,
        );
        assert_eq!(
            queue[..3].iter().map(|q| q.0).collect::<Vec<_>>(),
            vec![
                CoachQuestion::Context,
                CoachQuestion::FinishLine,
                CoachQuestion::OneAsk
            ]
        );
        assert_eq!(
            queue[..3].iter().map(|q| q.1).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(!queue
            .iter()
            .any(|q| q.0 == CoachQuestion::TaskFirst && q.1 < 100));
        assert_eq!(
            queue
                .iter()
                .filter(|q| q.0 == CoachQuestion::Context && q.1 < 100)
                .count(),
            12
        );
        assert!(!queue.iter().any(|q| q.1 == 500));
        let first_people = queue.iter().position(|q| q.1 == 600).unwrap();
        let first_older = queue.iter().position(|q| q.1 >= 100 && q.1 < 200).unwrap();
        assert!(first_people < first_older);
        assert_eq!(
            queue
                .iter()
                .filter(|q| q.0 == CoachQuestion::TaskFirst && q.1 >= 100)
                .count(),
            12
        );
    }
}
