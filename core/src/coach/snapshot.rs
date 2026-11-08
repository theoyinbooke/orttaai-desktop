//! Everything Coach shows, worked out in one pass from the counted
//! dictations: a voice shape per way of talking, this week's focus, the
//! habits and their proof, practice progress, and a quote caught in the wild.

use super::calendar::CoachCalendar;
use super::habit::{self, HabitSample, HabitVerdict};
use super::measure::{focus, CoachMeasure};
use super::mode::{CoachMode, Overrides};
use super::records::{
    CoachAppUsage, CoachHabitRecord, CoachHabitTarget, CoachInput, CoachQuote, CoachRepRecord,
};
use super::scoring::{MASTERY_SCORE, REPS_TO_MASTER};
use super::shape::{CoachShape, DatedMeasures, Judgements};
use super::skill::{all_skills, CoachQuestion, CoachRepKind, CoachSkill, CoachTrack};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Below this many dictations in the last 28 days a shape is still filling in.
pub const FULL_SHAPE_DICTATIONS: usize = 30;

/// Something worth working on: a measure, in one way of talking or all.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachFocusPick {
    pub measure: CoachMeasure,
    /// `None` is every way of talking.
    pub mode: Option<CoachMode>,
    /// Where it stands now, in the measure's unit.
    pub figure: Option<f64>,
}

impl CoachFocusPick {
    pub fn id(&self) -> String {
        format!(
            "{}-{}",
            self.measure.id(),
            self.mode.map(CoachMode::id).unwrap_or("all")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachHabitState {
    pub habit: CoachHabitRecord,
    pub target: CoachHabitTarget,
    pub mode: Option<CoachMode>,
    pub verdict: HabitVerdict,
    pub example: Option<CoachQuote>,
    pub title: String,
    pub hit_title: &'static str,
    pub measure: Option<CoachMeasure>,
    /// The unit of `verdict.before_value` and `after_value`.
    pub unit: &'static str,
}

impl CoachHabitState {
    fn new(habit: CoachHabitRecord, target: CoachHabitTarget, verdict: HabitVerdict) -> Self {
        let measure = target.measure();
        Self {
            mode: habit.coach_mode(),
            title: target.title(),
            hit_title: target.hit_title(),
            unit: measure.map(CoachMeasure::unit).unwrap_or("of dictations"),
            measure,
            habit,
            target,
            verdict,
            example: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachWeekDay {
    /// Local midnight, Unix seconds.
    pub date: i64,
    pub dictated: bool,
    pub practised: bool,
    pub is_today: bool,
    pub is_future: bool,
}

/// A recent dictation that did the thing being worked on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachCaught {
    /// `None` for a judged habit.
    pub measure: Option<CoachMeasure>,
    pub quote: CoachQuote,
    /// In a row, most recent first, in that way of talking.
    pub streak: usize,
}

/// Progress on one skill, from its reps.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachSkillProgress {
    pub reps: usize,
    /// Reps at 4 or more with the skill's own check met.
    pub strong_reps: usize,
    pub last_at: Option<i64>,
    /// The average of the last five scores.
    pub usual: Option<f64>,
    pub mastered: bool,
    pub review_due: bool,
}

impl CoachSkillProgress {
    pub fn empty() -> Self {
        Self {
            reps: 0,
            strong_reps: 0,
            last_at: None,
            usual: None,
            mastered: false,
            review_due: false,
        }
    }

    pub fn make(reps: &[&CoachRepRecord], now: i64) -> Self {
        let scores: Vec<f64> = reps.iter().filter_map(|r| r.score).collect();
        let recent = &scores[scores.len().saturating_sub(5)..];
        let strong = reps
            .iter()
            .filter(|r| r.score.unwrap_or(0.0) >= MASTERY_SCORE && r.skill_met == Some(true))
            .count();
        let last_at = reps.last().map(|r| r.created_at);
        let mastered = strong >= REPS_TO_MASTER;
        Self {
            reps: reps.len(),
            strong_reps: strong,
            last_at,
            usual: habit::mean(recent),
            mastered,
            review_due: mastered && last_at.is_some_and(|t| now - t > 7 * 86_400),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachTrackProgress {
    pub track: CoachTrack,
    pub level: usize,
    pub mastered: usize,
    pub skills: usize,
    pub reps: usize,
    pub current_skill: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachAppRow {
    #[serde(flatten)]
    pub usage: CoachAppUsage,
    pub display_name: String,
    /// The rule in force: the person's choice, else the table.
    pub rule: super::mode::CoachAppRule,
    pub builtin: CoachMode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachSnapshot {
    pub built_at: i64,
    /// Counted dictations, apps switched off left out.
    pub dictations: usize,
    pub shapes: BTreeMap<CoachMode, CoachShape>,
    pub all_shape: CoachShape,
    /// Dictations in the last 28 days for each way of talking.
    pub recent_by_mode: BTreeMap<CoachMode, usize>,
    pub busiest_mode: CoachMode,
    pub focus: Option<CoachFocusPick>,
    /// Up to three habits worth starting, for an empty Proof.
    pub suggestions: Vec<CoachFocusPick>,
    pub week: Vec<CoachWeekDay>,
    pub dictations_this_week: usize,
    pub words_this_week: i64,
    pub habits: Vec<CoachHabitState>,
    pub headline_habit: Option<i64>,
    pub caught: Option<CoachCaught>,
    pub apps: Vec<CoachAppRow>,
    pub first_dictation_at: Option<i64>,
    /// Progress on each skill, by skill id.
    pub skill_progress: BTreeMap<String, CoachSkillProgress>,
    pub tracks: Vec<CoachTrackProgress>,
    pub reps_this_week: usize,
    /// What Coach suggests practising today.
    pub todays_skill: &'static str,
    pub todays_kind: CoachRepKind,
}

impl CoachSnapshot {
    pub fn shape(&self, mode: Option<CoachMode>) -> &CoachShape {
        mode.and_then(|m| self.shapes.get(&m))
            .unwrap_or(&self.all_shape)
    }

    pub fn progress(&self, skill: &CoachSkill) -> CoachSkillProgress {
        self.skill_progress
            .get(skill.id)
            .cloned()
            .unwrap_or_else(CoachSkillProgress::empty)
    }

    /// The skill to work on next in a track: one due for review, else the
    /// first not yet mastered, else the first.
    pub fn current_skill(&self, track: CoachTrack) -> &'static CoachSkill {
        current_skill(&self.skill_progress, track)
    }
}

fn current_skill(
    progress: &BTreeMap<String, CoachSkillProgress>,
    track: CoachTrack,
) -> &'static CoachSkill {
    let skills = track.skills();
    let get = |s: &CoachSkill| {
        progress
            .get(s.id)
            .cloned()
            .unwrap_or_else(CoachSkillProgress::empty)
    };
    if let Some(due) = skills.iter().find(|s| get(s).review_due) {
        return due;
    }
    skills
        .iter()
        .find(|s| !get(s).mastered)
        .copied()
        .unwrap_or(skills[0])
}

fn mastered_count(progress: &BTreeMap<String, CoachSkillProgress>, track: CoachTrack) -> usize {
    track
        .skills()
        .iter()
        .filter(|s| progress.get(s.id).is_some_and(|p| p.mastered))
        .count()
}

/// The mode with the most dictations lately; the first on a tie, like Swift's `max`.
pub fn busiest(recent: &BTreeMap<CoachMode, usize>) -> CoachMode {
    let mut best = CoachMode::Ai;
    let mut count = None;
    for mode in CoachMode::SHAPED {
        let n = recent.get(&mode).copied().unwrap_or(0);
        if count.is_none_or(|c| n > c) {
            best = mode;
            count = Some(n);
        }
    }
    best
}

/// Everything the builder reads.
pub struct SnapshotInputs<'a> {
    pub inputs: &'a [CoachInput],
    pub habits: &'a [CoachHabitRecord],
    pub apps: &'a [CoachAppUsage],
    pub overrides: &'a Overrides,
    pub reps: &'a [CoachRepRecord],
    /// The judge's answers as points, by question and dictation.
    pub scores: &'a Judgements,
}

/// Judge answers as yes or not, for habits.
pub fn yes_answers(scores: &Judgements) -> HashMap<CoachQuestion, HashMap<i64, bool>> {
    scores
        .iter()
        .map(|(q, a)| (*q, a.iter().map(|(id, p)| (*id, *p == 1.0)).collect()))
        .collect()
}

pub fn dated(inputs: &[CoachInput], overrides: &Overrides) -> Vec<DatedMeasures> {
    inputs
        .iter()
        .filter_map(|i| {
            Some(DatedMeasures {
                id: i.id,
                created_at: i.created_at,
                mode: i.mode(overrides)?,
                measures: i.measures.clone(),
            })
        })
        .collect()
}

pub fn build(
    d: &SnapshotInputs<'_>,
    quotes: impl Fn(&[i64]) -> HashMap<i64, CoachQuote>,
    now: i64,
    cal: &CoachCalendar,
    redo_seed: u64,
) -> CoachSnapshot {
    let dated = dated(d.inputs, d.overrides);
    let judged = yes_answers(d.scores);

    let mut shapes = BTreeMap::new();
    for mode in CoachMode::SHAPED {
        shapes.insert(
            mode,
            CoachShape::make(&dated, Some(mode), now, d.scores, cal),
        );
    }
    let all_shape = CoachShape::make(&dated, None, now, &Judgements::new(), cal);

    let recent_start = cal.add_days(now, -super::shape::DAYS);
    let mut recent_by_mode: BTreeMap<CoachMode, usize> = BTreeMap::new();
    for item in dated.iter().filter(|i| i.created_at >= recent_start) {
        *recent_by_mode.entry(item.mode).or_default() += 1;
    }

    let active: Vec<&CoachHabitRecord> = d.habits.iter().filter(|h| h.ended_at.is_none()).collect();
    let busy: BTreeSet<String> = active
        .iter()
        .map(|h| format!("{}-{}", h.measure, h.mode.as_deref().unwrap_or("all")))
        .collect();
    let picks: Vec<CoachFocusPick> = focus_picks(&shapes, &recent_by_mode)
        .into_iter()
        .filter(|p| !busy.contains(&p.id()))
        .collect();

    // The week so far.
    let week_start = cal.week_start(now);
    let today = cal.start_of_day(now);
    let this_week: Vec<&DatedMeasures> = dated
        .iter()
        .filter(|i| i.created_at >= week_start)
        .collect();
    let days_dictated: BTreeSet<i64> = this_week
        .iter()
        .map(|i| cal.start_of_day(i.created_at))
        .collect();
    let week_reps: Vec<&CoachRepRecord> = d
        .reps
        .iter()
        .filter(|r| r.created_at >= week_start)
        .collect();
    let rep_days: BTreeSet<i64> = week_reps
        .iter()
        .map(|r| cal.start_of_day(r.created_at))
        .collect();
    let week: Vec<CoachWeekDay> = (0..7)
        .map(|offset| {
            let start = cal.start_of_day(cal.add_days(week_start, offset));
            CoachWeekDay {
                date: start,
                dictated: days_dictated.contains(&start),
                practised: rep_days.contains(&start),
                is_today: start == today,
                is_future: start > today,
            }
        })
        .collect();

    // Habits, judged on the dictations in their way of talking.
    let mut states: Vec<CoachHabitState> = active
        .iter()
        .filter_map(|h| {
            let target = h.target()?;
            let verdict = habit_verdict(h, target, &dated, &judged);
            Some(CoachHabitState::new((*h).clone(), target, verdict))
        })
        .collect();

    // Caught in the wild: from a habit, else from this week's focus.
    let mut caught_id = None;
    let mut caught_measure = None;
    let mut caught_mode = None;
    let mut caught_streak = None;
    if let Some(state) = states.iter().find(|s| s.verdict.best_example_id.is_some()) {
        caught_id = state.verdict.best_example_id;
        caught_measure = state.measure;
        caught_mode = state.mode;
        if state.measure.is_none() {
            caught_streak = Some(state.verdict.after.iter().rev().take_while(|&&h| h).count());
        }
    } else if let Some(f) = picks.first() {
        let since = cal.add_days(now, -14);
        let found = dated.iter().rev().find(|i| {
            i.created_at >= since
                && f.mode.is_none_or(|m| i.mode == m)
                && i.measures.words >= 12
                && f.measure.is_hit(&i.measures).unwrap_or(false)
        });
        caught_id = found.map(|i| i.id);
        caught_measure = Some(f.measure);
        caught_mode = f.mode;
    }

    let mut wanted: Vec<i64> = states
        .iter()
        .filter_map(|s| s.verdict.best_example_id)
        .collect();
    wanted.extend(caught_id);
    wanted.sort_unstable();
    wanted.dedup();
    let found = quotes(&wanted);
    for state in &mut states {
        state.example = state
            .verdict
            .best_example_id
            .and_then(|id| found.get(&id).cloned());
    }
    let caught =
        caught_id
            .and_then(|id| found.get(&id).cloned())
            .map(|quote| match caught_measure {
                Some(measure) => {
                    let run = dated
                        .iter()
                        .filter(|i| caught_mode.is_none_or(|m| i.mode == m))
                        .rev()
                        .filter_map(|i| measure.is_hit(&i.measures))
                        .take_while(|&h| h)
                        .count();
                    CoachCaught {
                        measure: Some(measure),
                        quote,
                        streak: run,
                    }
                }
                None => CoachCaught {
                    measure: None,
                    quote,
                    streak: caught_streak.unwrap_or(0),
                },
            });

    // Practice: progress per skill, this week's reps, and today's rep.
    let mut by_skill: HashMap<&str, Vec<&CoachRepRecord>> = HashMap::new();
    for rep in d.reps {
        by_skill.entry(rep.skill_id.as_str()).or_default().push(rep);
    }
    let skill_progress: BTreeMap<String, CoachSkillProgress> = by_skill
        .iter()
        .map(|(id, reps)| (id.to_string(), CoachSkillProgress::make(reps, now)))
        .collect();
    let tracks = CoachTrack::ALL
        .iter()
        .map(|&track| {
            let mastered = mastered_count(&skill_progress, track);
            CoachTrackProgress {
                track,
                level: 1 + mastered / 2,
                mastered,
                skills: track.skills().len(),
                reps: track
                    .skills()
                    .iter()
                    .map(|s| skill_progress.get(s.id).map_or(0, |p| p.reps))
                    .sum(),
                current_skill: current_skill(&skill_progress, track).id,
            }
        })
        .collect();

    let busiest_mode = busiest(&recent_by_mode);
    // Today: the busiest way of talking's track, or every other day the
    // Delivery skill for this week's focus when it's a counted one.
    let day = cal.day_of_year(now);
    let mut skill = current_skill(&skill_progress, CoachTrack::for_mode(busiest_mode));
    if day % 2 == 0 {
        if let Some(f) = picks.first() {
            if let Some(delivery) = CoachTrack::Delivery
                .skills()
                .into_iter()
                .find(|s| s.measure() == Some(f.measure))
            {
                if !skill_progress.get(delivery.id).is_some_and(|p| p.mastered) {
                    skill = delivery;
                }
            }
        }
    }
    let has_source = redo_source(skill, d.inputs, d.overrides, &[], now, redo_seed).is_some();

    let headline_habit = {
        let mut best: Option<(&CoachHabitState, f64)> = None;
        for s in &states {
            if let Some(share) = s.verdict.after_share {
                if best.is_none_or(|(_, b)| share > b) {
                    best = Some((s, share));
                }
            }
        }
        best.map(|(s, _)| s.habit.id)
    };

    let apps = d
        .apps
        .iter()
        .map(|a| CoachAppRow {
            display_name: a.display_name(),
            rule: super::mode::rule_for(a.app_id.as_deref(), a.app_name.as_deref(), d.overrides),
            builtin: super::mode::builtin_mode(a.app_id.as_deref(), a.app_name.as_deref()),
            usage: a.clone(),
        })
        .collect();

    CoachSnapshot {
        built_at: now,
        dictations: dated.len(),
        shapes,
        all_shape,
        recent_by_mode,
        busiest_mode,
        focus: picks.first().cloned(),
        suggestions: picks.iter().take(3).cloned().collect(),
        week,
        dictations_this_week: this_week.len(),
        words_this_week: this_week.iter().map(|i| i.measures.words).sum(),
        habits: states,
        headline_habit,
        caught,
        apps,
        first_dictation_at: d.inputs.first().map(|i| i.created_at),
        skill_progress,
        tracks,
        reps_this_week: week_reps.len(),
        todays_skill: skill.id,
        todays_kind: if has_source {
            CoachRepKind::Redo
        } else {
            CoachRepKind::Drill
        },
    }
}

/// A habit's state from its record and verdict, quote not yet read.
pub fn habit_state(
    habit: CoachHabitRecord,
    target: CoachHabitTarget,
    verdict: HabitVerdict,
) -> CoachHabitState {
    CoachHabitState::new(habit, target, verdict)
}

/// A habit judged on the dictations in its way of talking.
pub fn habit_verdict(
    habit: &CoachHabitRecord,
    target: CoachHabitTarget,
    dated: &[DatedMeasures],
    judgements: &HashMap<CoachQuestion, HashMap<i64, bool>>,
) -> HabitVerdict {
    let mode = habit.coach_mode();
    let samples: Vec<HabitSample> = dated
        .iter()
        .filter(|i| mode.is_none_or(|m| i.mode == m))
        .map(|i| HabitSample {
            id: i.id,
            created_at: i.created_at,
            measures: i.measures.clone(),
        })
        .collect();
    match target {
        CoachHabitTarget::Measure(measure) => {
            habit::verdict_for_measure(measure, habit.started_at, &samples, habit::DEFAULT_SEED)
        }
        CoachHabitTarget::Question(question) => {
            let empty = HashMap::new();
            let answers = judgements.get(&question).unwrap_or(&empty);
            habit::verdict(
                habit.started_at,
                &samples,
                |s| answers.get(&s.id).copied(),
                |s| answers.get(&s.id).map(|&a| if a { 1.0 } else { 0.0 }),
                &CoachMeasure::ALL,
                habit::DEFAULT_SEED,
            )
        }
    }
}

/// Needs across the ways of talking, most needed first, one per measure;
/// when nothing stands out, the weakest axes of the busiest mode, so there's
/// always something to try.
pub fn focus_picks(
    shapes: &BTreeMap<CoachMode, CoachShape>,
    recent: &BTreeMap<CoachMode, usize>,
) -> Vec<CoachFocusPick> {
    let mut scored: Vec<(CoachFocusPick, f64)> = Vec::new();
    for mode in CoachMode::SHAPED {
        let Some(shape) = shapes.get(&mode) else {
            continue;
        };
        for (measure, score) in focus::needs(&shape.now_summary) {
            scored.push((
                CoachFocusPick {
                    measure,
                    mode: Some(mode),
                    figure: measure.figure(&shape.now_summary),
                },
                score,
            ));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut picks = Vec::new();
    let mut seen = BTreeSet::new();
    for (pick, _) in scored {
        if seen.insert(pick.measure) {
            picks.push(pick);
        }
    }
    if picks.len() < 3 {
        let busiest = busiest(recent);
        if let Some(shape) = shapes.get(&busiest) {
            let mut weakest: Vec<(CoachMeasure, f64)> = shape
                .now
                .iter()
                .filter(|(m, v)| **v < 1.0 && !seen.contains(*m))
                .map(|(m, v)| (*m, *v))
                .collect();
            weakest.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (measure, _) in weakest {
                if picks.len() >= 3 {
                    break;
                }
                picks.push(CoachFocusPick {
                    measure,
                    mode: Some(busiest),
                    figure: measure.figure(&shape.now_summary),
                });
                seen.insert(measure);
            }
        }
    }
    picks
}

/// A dictation of yours to say again: recent, in the track's way of talking,
/// long enough to improve, and for Delivery one that missed. One of the 12
/// newest that fit, picked by `seed`.
pub fn redo_source(
    skill: &CoachSkill,
    inputs: &[CoachInput],
    overrides: &Overrides,
    avoiding: &[i64],
    now: i64,
    seed: u64,
) -> Option<i64> {
    let since = now - 21 * 86_400;
    let candidates: Vec<i64> = inputs
        .iter()
        .rev()
        .filter(|i| {
            if i.created_at < since
                || avoiding.contains(&i.id)
                || !(15..=140).contains(&i.measures.words)
            {
                return false;
            }
            let Some(mode) = i.mode(overrides) else {
                return false;
            };
            if skill.track.mode().is_some_and(|wanted| mode != wanted) {
                return false;
            }
            if let Some(measure) = skill.measure() {
                if measure.is_hit(&i.measures) != Some(false) {
                    return false;
                }
            }
            true
        })
        .take(12)
        .map(|i| i.id)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pick = habit::SplitMix64::new(seed).next() % candidates.len() as u64;
    Some(candidates[pick as usize])
}

/// Every skill's id in path order, for the UI catalog.
pub fn skill_ids() -> Vec<&'static str> {
    all_skills().iter().map(|s| s.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::counting::DictationMeasures;
    use crate::coach::measure::measures;
    use crate::coach::mode::CoachAppRule;

    const NOW: i64 = 1_800_000_000;
    const CAL: CoachCalendar = CoachCalendar::Fixed(0);

    fn input(id: i64, at: i64, app: &str, m: DictationMeasures) -> CoachInput {
        CoachInput {
            id,
            created_at: at,
            app_id: None,
            app_name: Some(app.into()),
            measures: m,
        }
    }

    fn rep(skill: &str, at: i64, score: f64, met: bool) -> CoachRepRecord {
        CoachRepRecord {
            id: None,
            skill_id: skill.into(),
            kind: "drill".into(),
            spoken: false,
            created_at: at,
            score: Some(score),
            skill_met: Some(met),
            duration_ms: 0,
            words: 30,
            checks: "[]".into(),
            persona: "mentor".into(),
        }
    }

    fn fixture() -> (Vec<CoachInput>, Vec<CoachAppUsage>) {
        let mut inputs = Vec::new();
        // 30 AI dictations over the last 20 days, heavy on fillers and long.
        for i in 0..30 {
            inputs.push(input(
                i,
                NOW - (30 - i) * 60_000,
                "Claude",
                measures(40, 3, 0, Some(180), &[18, 20], 0, Some(0.05), true),
            ));
        }
        // 10 people dictations, clean.
        for i in 0..10 {
            inputs.push(input(
                100 + i,
                NOW - (10 - i) * 70_000,
                "Slack",
                measures(30, 0, 0, Some(140), &[9], 0, Some(0.05), true),
            ));
        }
        inputs.sort_by_key(|i| i.created_at);
        let apps = vec![
            CoachAppUsage {
                key: "name:claude".into(),
                app_id: None,
                app_name: Some("Claude".into()),
                dictations: 30,
                first_at: Some(NOW - 1_800_000),
            },
            CoachAppUsage {
                key: "name:slack".into(),
                app_id: None,
                app_name: Some("Slack".into()),
                dictations: 10,
                first_at: Some(NOW - 700_000),
            },
        ];
        (inputs, apps)
    }

    #[test]
    fn snapshot_from_fixed_data() {
        let (inputs, apps) = fixture();
        let reps = vec![
            rep("task-first", NOW - 3_000, 4.5, true),
            rep("task-first", NOW - 2_000, 4.0, true),
            rep("task-first", NOW - 1_000, 5.0, true),
            rep("context", NOW - 500, 2.0, false),
        ];
        let overrides = Overrides::new();
        let scores = Judgements::new();
        let d = SnapshotInputs {
            inputs: &inputs,
            habits: &[],
            apps: &apps,
            overrides: &overrides,
            reps: &reps,
            scores: &scores,
        };
        let s = build(&d, |_| HashMap::new(), NOW, &CAL, 1);
        assert_eq!(s.dictations, 40);
        assert_eq!(s.recent_by_mode[&CoachMode::Ai], 30);
        assert_eq!(s.busiest_mode, CoachMode::Ai);
        // Fillers at 7.5 per 100 words lead the focus.
        assert_eq!(
            s.focus.as_ref().map(|f| f.measure),
            Some(CoachMeasure::Fillers)
        );
        assert_eq!(s.focus.as_ref().and_then(|f| f.mode), Some(CoachMode::Ai));
        assert!(s.suggestions.len() <= 3);
        let task = &s.skill_progress["task-first"];
        assert!(task.mastered);
        assert_eq!(task.strong_reps, 3);
        assert!((task.usual.unwrap() - 4.5).abs() < 1e-9);
        // Prompting moves on to the next skill once task-first is mastered.
        assert_eq!(s.current_skill(CoachTrack::Prompting).id, "context");
        assert_eq!(s.tracks[0].level, 1);
        assert_eq!(s.week.len(), 7);
        assert_eq!(s.week.iter().filter(|d| d.is_today).count(), 1);
        assert_eq!(s.apps[0].rule, CoachAppRule::Ai);
        assert_eq!(s.apps[1].builtin, CoachMode::People);
        serde_json::to_string(&s).unwrap();
    }

    #[test]
    fn switched_off_apps_are_left_out() {
        let (inputs, apps) = fixture();
        let mut overrides = Overrides::new();
        overrides.insert("name:slack".into(), CoachAppRule::Off);
        let scores = Judgements::new();
        let d = SnapshotInputs {
            inputs: &inputs,
            habits: &[],
            apps: &apps,
            overrides: &overrides,
            reps: &[],
            scores: &scores,
        };
        let s = build(&d, |_| HashMap::new(), NOW, &CAL, 1);
        assert_eq!(s.dictations, 30);
        assert_eq!(s.apps[1].rule, CoachAppRule::Off);
    }

    #[test]
    fn habits_and_caught_in_the_wild() {
        let mut inputs = Vec::new();
        let start = NOW - 10 * 86_400;
        for i in 0..20 {
            inputs.push(input(
                i,
                start - (20 - i) * 3_600,
                "Claude",
                measures(40, 3, 0, Some(150), &[10], 0, Some(0.05), true),
            ));
        }
        for i in 0..20 {
            inputs.push(input(
                100 + i,
                start + (i + 1) * 3_600,
                "Claude",
                measures(40, 0, 0, Some(150), &[10], 0, Some(0.05), true),
            ));
        }
        let habit = CoachHabitRecord {
            id: 7,
            measure: "fillers".into(),
            mode: Some("ai".into()),
            started_at: start,
            ended_at: None,
        };
        let overrides = Overrides::new();
        let scores = Judgements::new();
        let d = SnapshotInputs {
            inputs: &inputs,
            habits: std::slice::from_ref(&habit),
            apps: &[],
            overrides: &overrides,
            reps: &[],
            scores: &scores,
        };
        let quote = |ids: &[i64]| -> HashMap<i64, CoachQuote> {
            ids.iter()
                .map(|&id| {
                    (
                        id,
                        CoachQuote {
                            id,
                            text: format!("dictation {id}"),
                            created_at: NOW,
                            app_name: Some("Claude".into()),
                        },
                    )
                })
                .collect()
        };
        let s = build(&d, quote, NOW, &CAL, 1);
        let state = &s.habits[0];
        assert_eq!(state.verdict.status, habit::HabitStatus::Working);
        assert_eq!(state.verdict.before_share, Some(0.0));
        assert_eq!(state.verdict.after_share, Some(1.0));
        assert_eq!(state.example.as_ref().map(|q| q.id), Some(119));
        assert_eq!(s.headline_habit, Some(7));
        let caught = s.caught.as_ref().unwrap();
        assert_eq!(caught.quote.id, 119);
        assert_eq!(caught.streak, 20);
        // A habit already running isn't suggested again.
        assert!(!s.suggestions.iter().any(|p| p.id() == "fillers-ai"));
    }

    #[test]
    fn redo_picks_a_recent_dictation_that_missed() {
        let (inputs, _) = fixture();
        let overrides = Overrides::new();
        let fillers = CoachSkill::by_id("fewer-umuh").unwrap();
        let id = redo_source(fillers, &inputs, &overrides, &[], NOW, 3).unwrap();
        assert!(
            id < 30,
            "an AI dictation with fillers, not a clean Slack one"
        );
        let ask = CoachSkill::by_id("ask-first").unwrap();
        let id = redo_source(ask, &inputs, &overrides, &[], NOW, 3).unwrap();
        assert!((100..110).contains(&id));
        assert_eq!(
            redo_source(ask, &inputs, &overrides, &[], NOW + 30 * 86_400, 3),
            None
        );
    }
}
