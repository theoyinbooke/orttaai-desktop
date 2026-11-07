//! Whether a habit stuck: 20 dictations before it started against the latest
//! 20 since, as the share that met it, with a seeded bootstrap so the answer
//! is the same every time, and a comparison with the measures nobody
//! practised, so a good week doesn't take the credit.

use super::counting::DictationMeasures;
use super::measure::CoachMeasure;
use serde::Serialize;

/// One dictation a habit is judged on.
#[derive(Debug, Clone)]
pub struct HabitSample {
    pub id: i64,
    pub created_at: i64,
    pub measures: DictationMeasures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HabitStatus {
    /// Not enough dictations since it started, or before it, to tell yet.
    Watching,
    Working,
    NotMoving,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HabitVerdict {
    pub status: HabitStatus,
    /// Met or not, oldest first, for the dots.
    pub before: Vec<bool>,
    pub after: Vec<bool>,
    /// Dictations since the habit started that the measure could read.
    pub since_count: usize,
    pub before_share: Option<f64>,
    pub after_share: Option<f64>,
    /// The measure's average, in its unit.
    pub before_value: Option<f64>,
    pub after_value: Option<f64>,
    /// How far the share moved, in points, on the measures nobody practised.
    pub control_drift: Option<f64>,
    /// The latest dictation since the start that met it, 12 words or more.
    pub best_example_id: Option<i64>,
    /// Dictations still needed before a verdict.
    pub still_needed: usize,
}

pub const WINDOW: usize = 20;
/// Fewer than this before the start and there's no baseline to beat.
pub const MINIMUM_BASELINE: usize = 10;
const RESAMPLES: usize = 1_000;
pub const DEFAULT_SEED: u64 = 0x0C0AC4;

/// A counted measure's verdict; `samples` are the habit's mode, in any order.
pub fn verdict_for_measure(
    measure: CoachMeasure,
    started_at: i64,
    samples: &[HabitSample],
    seed: u64,
) -> HabitVerdict {
    let controls: Vec<CoachMeasure> = CoachMeasure::ALL
        .into_iter()
        .filter(|&m| m != measure)
        .collect();
    verdict(
        started_at,
        samples,
        |s| measure.is_hit(&s.measures),
        |s| measure.value(&s.measures),
        &controls,
        seed,
    )
}

/// Any notion of met: `is_hit` is `None` when a dictation can't be told, and
/// `controls` are the measures nobody practised.
pub fn verdict(
    started_at: i64,
    samples: &[HabitSample],
    is_hit: impl Fn(&HabitSample) -> Option<bool>,
    value: impl Fn(&HabitSample) -> Option<f64>,
    controls: &[CoachMeasure],
    seed: u64,
) -> HabitVerdict {
    let mut sorted: Vec<&HabitSample> = samples.iter().collect();
    sorted.sort_by_key(|s| s.created_at);
    let readable: Vec<&HabitSample> = sorted
        .iter()
        .copied()
        .filter(|s| is_hit(s).is_some())
        .collect();
    let before_all: Vec<&HabitSample> = readable
        .iter()
        .copied()
        .filter(|s| s.created_at < started_at)
        .collect();
    let before_samples: Vec<&HabitSample> = suffix(&before_all, WINDOW);
    let since_samples: Vec<&HabitSample> = readable
        .iter()
        .copied()
        .filter(|s| s.created_at >= started_at)
        .collect();
    let after_samples: Vec<&HabitSample> = suffix(&since_samples, WINDOW);

    let before: Vec<bool> = before_samples
        .iter()
        .map(|s| is_hit(s).unwrap_or(false))
        .collect();
    let after: Vec<bool> = after_samples
        .iter()
        .map(|s| is_hit(s).unwrap_or(false))
        .collect();
    let best = since_samples
        .iter()
        .rev()
        .find(|s| s.measures.words >= 12 && is_hit(s) == Some(true));

    let mut v = HabitVerdict {
        status: HabitStatus::Watching,
        before_share: share(&before),
        after_share: share(&after),
        before_value: mean(
            &before_samples
                .iter()
                .filter_map(|s| value(s))
                .collect::<Vec<_>>(),
        ),
        after_value: mean(
            &after_samples
                .iter()
                .filter_map(|s| value(s))
                .collect::<Vec<_>>(),
        ),
        before,
        after,
        since_count: since_samples.len(),
        control_drift: None,
        best_example_id: best.map(|s| s.id),
        still_needed: WINDOW.saturating_sub(since_samples.len()),
    };

    let (Some(before_share), Some(after_share)) = (v.before_share, v.after_share) else {
        return v;
    };
    if v.before.len() < MINIMUM_BASELINE || v.after.len() < WINDOW {
        return v;
    }
    v.control_drift = control_drift(controls, &before_samples, &after_samples, &sorted);
    let change = after_share - before_share;
    let (low, _) = bootstrap_interval(&v.before, &v.after, seed);
    let beats_drift = change > v.control_drift.unwrap_or(0.0);
    v.status = if low > 0.0 && beats_drift {
        HabitStatus::Working
    } else {
        HabitStatus::NotMoving
    };
    v
}

fn suffix<'a>(items: &[&'a HabitSample], n: usize) -> Vec<&'a HabitSample> {
    items[items.len().saturating_sub(n)..].to_vec()
}

/// The other measures' change in share over the same stretches of time.
fn control_drift(
    controls: &[CoachMeasure],
    before: &[&HabitSample],
    after: &[&HabitSample],
    all: &[&HabitSample],
) -> Option<f64> {
    let (b0, b1) = (before.first()?.created_at, before.last()?.created_at);
    let (a0, a1) = (after.first()?.created_at, after.last()?.created_at);
    let before_span: Vec<&&HabitSample> = all
        .iter()
        .filter(|s| s.created_at >= b0 && s.created_at <= b1)
        .collect();
    let after_span: Vec<&&HabitSample> = all
        .iter()
        .filter(|s| s.created_at >= a0 && s.created_at <= a1)
        .collect();
    let mut changes = Vec::new();
    for other in controls {
        let b: Vec<bool> = before_span
            .iter()
            .filter_map(|s| other.is_hit(&s.measures))
            .collect();
        let a: Vec<bool> = after_span
            .iter()
            .filter_map(|s| other.is_hit(&s.measures))
            .collect();
        if b.len() < MINIMUM_BASELINE || a.len() < MINIMUM_BASELINE {
            continue;
        }
        if let (Some(bs), Some(as_)) = (share(&b), share(&a)) {
            changes.push(as_ - bs);
        }
    }
    mean(&changes)
}

/// The 5th and 95th percentiles of the change in share.
pub fn bootstrap_interval(before: &[bool], after: &[bool], seed: u64) -> (f64, f64) {
    let mut rng = SplitMix64::new(seed);
    let b: Vec<f64> = before.iter().map(|&x| if x { 1.0 } else { 0.0 }).collect();
    let a: Vec<f64> = after.iter().map(|&x| if x { 1.0 } else { 0.0 }).collect();
    let mut changes = Vec::with_capacity(RESAMPLES);
    for _ in 0..RESAMPLES {
        let mut rb = 0.0;
        for _ in 0..b.len() {
            rb += b[(rng.next() % b.len() as u64) as usize];
        }
        rb /= b.len() as f64;
        let mut ra = 0.0;
        for _ in 0..a.len() {
            ra += a[(rng.next() % a.len() as u64) as usize];
        }
        ra /= a.len() as f64;
        changes.push(ra - rb);
    }
    changes.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    (
        changes[RESAMPLES * 5 / 100],
        changes[RESAMPLES * 95 / 100 - 1],
    )
}

pub(crate) fn share(hits: &[bool]) -> Option<f64> {
    if hits.is_empty() {
        None
    } else {
        Some(hits.iter().filter(|&&h| h).count() as f64 / hits.len() as f64)
    }
}

pub(crate) fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

/// A small seeded generator, so a verdict never changes between launches.
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_800_000_000;

    fn samples(before: &[bool], after: &[bool]) -> Vec<HabitSample> {
        let make = |hit: bool, offset: i64, id: i64| HabitSample {
            id,
            created_at: START + offset * 3_600,
            measures: DictationMeasures {
                words: 100,
                um_uh: if hit { 0 } else { 4 },
                phrase_fillers: 0,
                wpm: Some(140),
                sentence_lengths: vec![10],
                repeats: 0,
                polish_change_ratio: Some(0.05),
                raw_known: true,
            },
        };
        let n = before.len() as i64;
        let mut all: Vec<HabitSample> = before
            .iter()
            .enumerate()
            .map(|(i, &h)| make(h, -n + i as i64, i as i64))
            .collect();
        all.extend(
            after
                .iter()
                .enumerate()
                .map(|(i, &h)| make(h, i as i64, 1_000 + i as i64)),
        );
        all
    }

    fn fillers(before: &[bool], after: &[bool]) -> HabitVerdict {
        verdict_for_measure(
            CoachMeasure::Fillers,
            START,
            &samples(before, after),
            DEFAULT_SEED,
        )
    }

    #[test]
    fn watching_until_twenty_since() {
        let v = fillers(&[false; 20], &[true; 11]);
        assert_eq!(v.status, HabitStatus::Watching);
        assert_eq!(v.still_needed, 9);
        assert_eq!(v.since_count, 11);
        assert_eq!(v.control_drift, None);
    }

    #[test]
    fn no_baseline_means_watching() {
        assert_eq!(
            fillers(&[false, false], &[true; 25]).status,
            HabitStatus::Watching
        );
    }

    #[test]
    fn clear_improvement_is_working() {
        let before: Vec<bool> = (0..20).map(|i| i % 5 < 2).collect();
        let after: Vec<bool> = (0..20).map(|i| i % 10 != 0).collect();
        let v = fillers(&before, &after);
        assert_eq!(v.status, HabitStatus::Working);
        assert_eq!(v.before_share, Some(0.4));
        assert_eq!(v.after_share, Some(0.9));
        assert_eq!(v.control_drift, Some(0.0));
        assert_eq!(v.before.len(), 20);
        assert_eq!(v.after.len(), 20);
        assert_eq!(v.best_example_id, Some(1_019));
    }

    #[test]
    fn no_change_is_not_moving() {
        let same: Vec<bool> = (0..20).map(|i| i % 2 == 0).collect();
        assert_eq!(fillers(&same, &same).status, HabitStatus::NotMoving);
    }

    #[test]
    fn uses_the_latest_twenty_since() {
        let after: Vec<bool> = [vec![false; 10], vec![true; 20]].concat();
        let v = fillers(&[false; 20], &after);
        assert_eq!(v.after, vec![true; 20]);
        assert_eq!(v.since_count, 30);
        assert_eq!(v.status, HabitStatus::Working);
    }

    #[test]
    fn same_answer_every_time() {
        let before: Vec<bool> = (0..20).map(|i| i % 3 == 0).collect();
        let after: Vec<bool> = (0..20).map(|i| i % 3 != 0).collect();
        let first = bootstrap_interval(&before, &after, 7);
        assert_eq!(first, bootstrap_interval(&before, &after, 7));
        assert!(first.0 < first.1);
    }

    #[test]
    fn habits_work_on_any_notion_of_met() {
        let sample = |id: i64, offset: i64| HabitSample {
            id,
            created_at: START + offset * 600,
            measures: DictationMeasures {
                words: 30,
                um_uh: 0,
                phrase_fillers: 0,
                wpm: Some(140),
                sentence_lengths: vec![10],
                repeats: 0,
                polish_change_ratio: Some(0.0),
                raw_known: true,
            },
        };
        let mut all: Vec<HabitSample> = (0..20).map(|i| sample(i, -20 + i)).collect();
        all.extend((0..20).map(|i| sample(100 + i, i)));
        // Judged answers: none before, all since; one not judged yet.
        let answer = |s: &HabitSample| if s.id == 105 { None } else { Some(s.id >= 100) };
        let v = verdict(
            START,
            &all,
            answer,
            |s| answer(s).map(|a| if a { 1.0 } else { 0.0 }),
            &CoachMeasure::ALL,
            DEFAULT_SEED,
        );
        assert_eq!(v.before.len(), 20);
        assert_eq!(v.since_count, 19);
        assert_eq!(v.status, HabitStatus::Watching);
        assert_eq!(v.still_needed, 1);
    }
}
