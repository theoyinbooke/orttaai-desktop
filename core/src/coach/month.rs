//! A calendar month looked back on: how much you dictated, your voice shape
//! against the month before, what moved, the practice you did, where each
//! habit stood at the month's end, and the month's best moment.

use super::axis::CoachAxis;
use super::calendar::{CoachCalendar, Interval};
use super::mode::{CoachMode, Overrides};
use super::records::{CoachHabitRecord, CoachInput, CoachQuote, CoachRepRecord};
use super::scoring::{JudgeAnswer, MASTERY_SCORE, REPS_TO_MASTER};
use super::shape::{CoachShape, DatedMeasures, Judgements};
use super::skill::CoachTrack;
use super::snapshot::{habit_verdict, yes_answers, CoachHabitState};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The smallest change worth reporting, in axis points.
pub const NOTABLE_CHANGE: f64 = 0.03;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachMonthMove {
    pub axis: CoachAxis,
    pub title: &'static str,
    pub mode: Option<CoachMode>,
    pub now: f64,
    pub then: f64,
    pub change: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachMonthTrack {
    pub track: CoachTrack,
    pub reps: usize,
    /// Skill ids whose mastering rep fell in the month.
    pub mastered: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachMonthBestRep {
    pub skill: &'static str,
    pub score: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachMonthReport {
    /// The calendar month; its end is the first moment of the next.
    pub month: Interval,
    pub previous: Interval,
    /// Still running: the month we're in.
    pub is_current: bool,
    /// The first month there's anything to look back on, for stepping back.
    pub first_month: Option<i64>,
    pub dictations: usize,
    pub words: i64,
    pub days_dictated: usize,
    pub reps: usize,
    pub dictations_before: usize,
    pub shapes: BTreeMap<CoachMode, CoachShape>,
    pub all_shape: CoachShape,
    /// Every axis that moved by 3 points or more, gains first, by lens.
    pub moves: Vec<CoachMonthMove>,
    pub tracks: Vec<CoachMonthTrack>,
    pub best_rep: Option<CoachMonthBestRep>,
    /// Habits that ran during the month, judged on dictations up to its end.
    pub habits: Vec<CoachHabitState>,
    /// A dictation that met the month's top gain.
    pub best_moment: Option<CoachQuote>,
    pub best_moment_axis: Option<CoachAxis>,
    pub is_empty: bool,
    pub mastered_count: usize,
    /// The biggest gain anywhere, for the headline.
    pub top_gain: Option<CoachMonthMove>,
}

pub struct MonthInputs<'a> {
    pub inputs: &'a [CoachInput],
    pub habits: &'a [CoachHabitRecord],
    pub reps: &'a [CoachRepRecord],
    pub scores: &'a Judgements,
    pub overrides: &'a Overrides,
}

fn top_gain(moves: &[CoachMonthMove]) -> Option<&CoachMonthMove> {
    let mut best: Option<&CoachMonthMove> = None;
    for m in moves.iter().filter(|m| m.change > 0.0) {
        if best.is_none_or(|b| m.change > b.change) {
            best = Some(m);
        }
    }
    best
}

pub fn build(
    month_of: i64,
    d: &MonthInputs<'_>,
    quotes: impl Fn(&[i64]) -> HashMap<i64, CoachQuote>,
    now: i64,
    cal: &CoachCalendar,
) -> CoachMonthReport {
    let month = cal.month(month_of);
    let previous = Interval::new(cal.add_months(month.start, -1), month.start);
    let is_current = month.contains(now) && now < month.end;

    let dated: Vec<DatedMeasures> = d
        .inputs
        .iter()
        .filter(|i| i.created_at < month.end)
        .filter_map(|i| {
            Some(DatedMeasures {
                id: i.id,
                created_at: i.created_at,
                mode: i.mode(d.overrides)?,
                measures: i.measures.clone(),
            })
        })
        .collect();
    let in_month: Vec<&DatedMeasures> = dated
        .iter()
        .filter(|i| i.created_at >= month.start)
        .collect();
    let window = Interval::new(month.start, month.start.max(now.min(month.end)));

    let mut shapes = BTreeMap::new();
    for mode in CoachMode::SHAPED {
        shapes.insert(
            mode,
            CoachShape::make_window(&dated, Some(mode), window, previous, d.scores),
        );
    }
    let all_shape = CoachShape::make_window(&dated, None, window, previous, d.scores);

    let mut moves = Vec::new();
    let lenses: Vec<(Option<CoachMode>, &CoachShape)> = std::iter::once((None, &all_shape))
        .chain(
            CoachMode::SHAPED
                .iter()
                .filter_map(|m| shapes.get(m).map(|s| (Some(*m), s))),
        )
        .collect();
    for (mode, shape) in lenses {
        for axis in &shape.axes {
            let (Some(n), Some(t)) = (shape.now_axes.get(axis), shape.then_axes.get(axis)) else {
                continue;
            };
            if (n - t).abs() < NOTABLE_CHANGE {
                continue;
            }
            moves.push(CoachMonthMove {
                axis: *axis,
                title: axis.title(),
                mode,
                now: *n,
                then: *t,
                change: n - t,
            });
        }
    }
    moves.sort_by(|a, b| {
        b.change
            .partial_cmp(&a.change)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Practice: reps by track, and skills whose mastering rep fell in the month.
    let month_reps: Vec<&CoachRepRecord> = d
        .reps
        .iter()
        .filter(|r| month.contains(r.created_at) && r.created_at < month.end)
        .collect();
    let mut sorted_reps: Vec<&CoachRepRecord> = d.reps.iter().collect();
    sorted_reps.sort_by_key(|r| r.created_at);
    let mut mastered_at: HashMap<&str, i64> = HashMap::new();
    let mut strong: HashMap<&str, usize> = HashMap::new();
    for rep in sorted_reps {
        if rep.score.unwrap_or(0.0) >= MASTERY_SCORE && rep.skill_met == Some(true) {
            let n = strong.entry(rep.skill_id.as_str()).or_default();
            *n += 1;
            if *n == REPS_TO_MASTER {
                mastered_at.insert(rep.skill_id.as_str(), rep.created_at);
            }
        }
    }
    let tracks: Vec<CoachMonthTrack> = CoachTrack::ALL
        .iter()
        .map(|&track| {
            let skills = track.skills();
            let ids: BTreeSet<&str> = skills.iter().map(|s| s.id).collect();
            let mastered = skills
                .iter()
                .filter(|s| {
                    mastered_at
                        .get(s.id)
                        .is_some_and(|&t| t >= month.start && t < month.end)
                })
                .map(|s| s.id)
                .collect();
            CoachMonthTrack {
                track,
                reps: month_reps
                    .iter()
                    .filter(|r| ids.contains(r.skill_id.as_str()))
                    .count(),
                mastered,
            }
        })
        .collect();
    let mut best_rep: Option<CoachMonthBestRep> = None;
    for rep in &month_reps {
        let (Some(skill), Some(score)) = (rep.skill(), rep.score) else {
            continue;
        };
        if best_rep.as_ref().is_none_or(|b| score > b.score) {
            best_rep = Some(CoachMonthBestRep {
                skill: skill.id,
                score,
                created_at: rep.created_at,
            });
        }
    }

    // Habits that ran during the month, as they stood at its end.
    let judged = yes_answers(d.scores);
    let habits: Vec<CoachHabitState> = d
        .habits
        .iter()
        .filter(|h| h.started_at < month.end && h.ended_at.unwrap_or(i64::MAX) >= month.start)
        .filter_map(|h| {
            let target = h.target()?;
            let verdict = habit_verdict(h, target, &dated, &judged);
            Some(super::snapshot::habit_state(h.clone(), target, verdict))
        })
        .collect();

    // The best moment: the latest dictation in the month that met the top gain.
    let gain = top_gain(&moves).cloned();
    let mut moment_id = None;
    if let Some(g) = &gain {
        let pool: Vec<&&DatedMeasures> = in_month
            .iter()
            .filter(|i| g.mode.is_none_or(|m| i.mode == m) && i.measures.words >= 12)
            .collect();
        moment_id = match g.axis {
            CoachAxis::Measure(measure) => pool
                .iter()
                .rev()
                .find(|i| measure.is_hit(&i.measures) == Some(true))
                .map(|i| i.id),
            CoachAxis::Question(q) => {
                let empty = HashMap::new();
                let answers = d.scores.get(&q).unwrap_or(&empty);
                pool.iter()
                    .rev()
                    .find(|i| answers.get(&i.id) == Some(&JudgeAnswer::Yes.points()))
                    .map(|i| i.id)
            }
        };
    }
    let found = moment_id.map(|id| quotes(&[id])).unwrap_or_default();

    let first_dictation = d.inputs.iter().map(|i| i.created_at).min();
    let first_rep = d.reps.iter().map(|r| r.created_at).min();
    let first = [first_dictation, first_rep].into_iter().flatten().min();

    let dictations = in_month.len();
    let reps = month_reps.len();
    let mastered_count = tracks.iter().map(|t| t.mastered.len()).sum();
    CoachMonthReport {
        month,
        previous,
        is_current,
        first_month: first.map(|t| cal.month(t).start),
        dictations,
        words: in_month.iter().map(|i| i.measures.words).sum(),
        days_dictated: in_month
            .iter()
            .map(|i| cal.start_of_day(i.created_at))
            .collect::<BTreeSet<_>>()
            .len(),
        reps,
        dictations_before: dated
            .iter()
            .filter(|i| previous.contains(i.created_at) && i.created_at < previous.end)
            .count(),
        shapes,
        all_shape,
        moves,
        tracks,
        best_rep,
        habits,
        best_moment: moment_id.and_then(|id| found.get(&id).cloned()),
        best_moment_axis: if moment_id.is_some() {
            gain.as_ref().map(|g| g.axis)
        } else {
            None
        },
        is_empty: dictations == 0 && reps == 0,
        mastered_count,
        top_gain: gain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::measure::{measures, CoachMeasure};

    const CAL: CoachCalendar = CoachCalendar::Fixed(0);

    fn input(id: i64, at: i64, um: i64) -> CoachInput {
        CoachInput {
            id,
            created_at: at,
            app_id: None,
            app_name: Some("Claude".into()),
            measures: measures(100, um, 0, Some(145), &[10], 0, Some(0.05), true),
        }
    }

    #[test]
    fn a_month_against_the_month_before() {
        let now = 1_800_000_000;
        let month = CAL.month(now);
        let previous_start = CAL.add_months(month.start, -1);
        let mut inputs = Vec::new();
        for i in 0..9 {
            inputs.push(input(50 + i, previous_start + i * 86_400, 4));
        }
        for i in 0..9 {
            inputs.push(input(i, month.start + i * 3_600, 1));
        }
        let reps: Vec<CoachRepRecord> = (0..3)
            .map(|i| CoachRepRecord {
                id: None,
                skill_id: "fewer-umuh".into(),
                kind: "drill".into(),
                spoken: true,
                created_at: month.start + 100 + i,
                score: Some(4.0 + i as f64 * 0.5),
                skill_met: Some(true),
                duration_ms: 20_000,
                words: 40,
                checks: "[]".into(),
                persona: "mentor".into(),
            })
            .collect();
        let habits = vec![CoachHabitRecord {
            id: 1,
            measure: "fillers".into(),
            mode: None,
            started_at: month.start,
            ended_at: None,
        }];
        let overrides = Overrides::new();
        let scores = Judgements::new();
        let d = MonthInputs {
            inputs: &inputs,
            habits: &habits,
            reps: &reps,
            scores: &scores,
            overrides: &overrides,
        };
        let quotes = |ids: &[i64]| -> HashMap<i64, CoachQuote> {
            ids.iter()
                .map(|&id| {
                    (
                        id,
                        CoachQuote {
                            id,
                            text: "x".into(),
                            created_at: 0,
                            app_name: None,
                        },
                    )
                })
                .collect()
        };
        let r = build(now, &d, quotes, now, &CAL);
        assert!(r.is_current);
        assert_eq!(r.dictations, 9);
        assert_eq!(r.words, 900);
        assert_eq!(r.days_dictated, 1);
        assert_eq!(r.dictations_before, 9);
        assert_eq!(r.reps, 3);
        assert_eq!(r.mastered_count, 1);
        assert_eq!(r.tracks[2].mastered, vec!["fewer-umuh"]);
        assert_eq!(r.best_rep.as_ref().map(|b| b.score), Some(5.0));
        let gain = r.top_gain.as_ref().unwrap();
        assert_eq!(gain.axis, CoachAxis::Measure(CoachMeasure::Fillers));
        assert!(gain.change > 0.5);
        assert_eq!(r.best_moment.as_ref().map(|q| q.id), Some(8));
        assert_eq!(r.habits.len(), 1);
        assert_eq!(r.first_month, Some(CAL.month(previous_start).start));
        assert!(!r.is_empty);
        serde_json::to_string(&r).unwrap();

        // Two months on, nothing happened.
        let later = build(
            CAL.add_months(now, 2),
            &d,
            |_| HashMap::new(),
            CAL.add_months(now, 3),
            &CAL,
        );
        assert!(later.is_empty);
        assert!(!later.is_current);
    }
}
