//! The six things Coach watches, how each reads one dictation, what counts
//! as meeting it, and how a group of dictations scores on the voice shape.

use super::counting::DictationMeasures;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachMeasure {
    Fillers,
    Phrases,
    Pace,
    Sentences,
    Repeats,
    Clarity,
}

impl CoachMeasure {
    pub const ALL: [CoachMeasure; 6] = [
        Self::Fillers,
        Self::Phrases,
        Self::Pace,
        Self::Sentences,
        Self::Repeats,
        Self::Clarity,
    ];

    /// Dictations needed before an axis is drawn.
    pub const MINIMUM_SAMPLES: usize = 8;

    pub fn id(self) -> &'static str {
        match self {
            Self::Fillers => "fillers",
            Self::Phrases => "phrases",
            Self::Pace => "pace",
            Self::Sentences => "sentences",
            Self::Repeats => "repeats",
            Self::Clarity => "clarity",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The voice shape's axis label.
    pub fn axis_title(self) -> &'static str {
        match self {
            Self::Fillers => "Fewer fillers",
            Self::Phrases => "Clean phrases",
            Self::Pace => "Steady pace",
            Self::Sentences => "Short sentences",
            Self::Repeats => "No repeats",
            Self::Clarity => "Said it clearly",
        }
    }

    /// The habit's name.
    pub fn habit_title(self) -> &'static str {
        match self {
            Self::Fillers => "Fewer ums and uhs",
            Self::Phrases => "Fewer filler phrases",
            Self::Pace => "A steadier pace",
            Self::Sentences => "Shorter sentences",
            Self::Repeats => "No words said twice",
            Self::Clarity => "Say it right first time",
        }
    }

    /// What meeting it means, for a dictation.
    pub fn hit_title(self) -> &'static str {
        match self {
            Self::Fillers => "1 um or uh per 100 words or fewer",
            Self::Phrases => "1 filler phrase per 100 words or fewer",
            Self::Pace => "160 words a minute or slower",
            Self::Sentences => "Sentences of 14 words or fewer",
            Self::Repeats => "No word said twice",
            Self::Clarity => "Polish changed 15% or less",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::Fillers | Self::Phrases | Self::Repeats => "per 100 words",
            Self::Pace => "words a minute",
            Self::Sentences => "words a sentence",
            Self::Clarity => "rewritten",
        }
    }

    /// The measure for one dictation, or `None` when it can't be told.
    pub fn value(self, m: &DictationMeasures) -> Option<f64> {
        match self {
            Self::Fillers => {
                (m.raw_known && m.words >= 5).then(|| m.um_uh as f64 / m.words as f64 * 100.0)
            }
            Self::Phrases => (m.raw_known && m.words >= 5)
                .then(|| m.phrase_fillers as f64 / m.words as f64 * 100.0),
            Self::Pace => m.wpm.map(|w| w as f64),
            Self::Sentences => {
                if m.raw_known {
                    m.median_sentence().map(|v| v as f64)
                } else {
                    None
                }
            }
            Self::Repeats => {
                (m.raw_known && m.words >= 5).then(|| m.repeats as f64 / m.words as f64 * 100.0)
            }
            Self::Clarity => m.polish_change_ratio,
        }
    }

    /// Whether a dictation's value meets the measure.
    pub fn is_hit_value(self, value: f64) -> bool {
        match self {
            Self::Fillers | Self::Phrases => value <= 1.0,
            Self::Pace => value <= 160.0,
            Self::Sentences => value <= 14.0,
            Self::Repeats => value == 0.0,
            Self::Clarity => value <= 0.15,
        }
    }

    pub fn is_hit(self, m: &DictationMeasures) -> Option<bool> {
        self.value(m).map(|v| self.is_hit_value(v))
    }

    /// 0 to 1 on the voice shape, 1 best; `None` until there's enough to tell.
    pub fn axis_score(self, s: &CoachSummary) -> Option<f64> {
        let clamp = |x: f64| x.clamp(0.0, 1.0);
        let enough = s.raw_dictations >= Self::MINIMUM_SAMPLES;
        match self {
            Self::Fillers => enough.then(|| clamp(1.0 - s.um_uh_rate() / 5.0)),
            Self::Phrases => enough.then(|| clamp(1.0 - s.phrase_rate() / 4.0)),
            Self::Pace => {
                if s.paced_dictations < Self::MINIMUM_SAMPLES {
                    return None;
                }
                let p = s.pace? as f64;
                Some(if p < 130.0 {
                    clamp(1.0 - (130.0 - p) / 60.0)
                } else if p > 160.0 {
                    clamp(1.0 - (p - 160.0) / 60.0)
                } else {
                    1.0
                })
            }
            Self::Sentences => {
                if !enough {
                    return None;
                }
                let median = s.median_sentence?;
                Some(if median <= 14 {
                    1.0
                } else {
                    clamp(1.0 - (median - 14) as f64 / 16.0)
                })
            }
            Self::Repeats => enough.then(|| clamp(1.0 - s.repeat_rate() / 2.0)),
            Self::Clarity => (s.clarity_dictations >= Self::MINIMUM_SAMPLES)
                .then(|| clamp(1.0 - s.polish_fix_share())),
        }
    }

    /// The figure shown for a group of dictations, in `unit`.
    pub fn figure(self, s: &CoachSummary) -> Option<f64> {
        match self {
            Self::Fillers => (s.raw_dictations > 0).then(|| s.um_uh_rate()),
            Self::Phrases => (s.raw_dictations > 0).then(|| s.phrase_rate()),
            Self::Pace => s.pace.map(|p| p as f64),
            Self::Sentences => s.median_sentence.map(|m| m as f64),
            Self::Repeats => (s.raw_dictations > 0).then(|| s.repeat_rate()),
            Self::Clarity => (s.clarity_dictations > 0).then(|| s.polish_fix_share()),
        }
    }
}

/// Counts over many dictations, as Orttakey's CoachStats adds them up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachSummary {
    pub dictations: usize,
    pub words: i64,
    /// Words in dictations whose raw transcript is known.
    pub raw_words: i64,
    pub raw_dictations: usize,
    pub um_uh: i64,
    pub phrase_fillers: i64,
    pub repeats: i64,
    pub paced_dictations: usize,
    pub pace: Option<i64>,
    /// The pace a quarter of dictations run above.
    pub fast_pace: Option<i64>,
    pub median_sentence: Option<i64>,
    pub longest_sentence: Option<i64>,
    pub clarity_dictations: usize,
    pub polish_fixes: usize,
}

impl CoachSummary {
    pub fn new<'a>(all: impl IntoIterator<Item = &'a DictationMeasures>) -> Self {
        let all: Vec<&DictationMeasures> = all.into_iter().collect();
        let raw: Vec<&&DictationMeasures> = all.iter().filter(|m| m.raw_known).collect();
        let mut paces: Vec<i64> = all.iter().filter_map(|m| m.wpm).collect();
        paces.sort_unstable();
        let mut lengths: Vec<i64> = raw
            .iter()
            .flat_map(|m| m.sentence_lengths.iter().copied())
            .collect();
        lengths.sort_unstable();
        let ratios: Vec<f64> = all.iter().filter_map(|m| m.polish_change_ratio).collect();
        Self {
            dictations: all.len(),
            words: all.iter().map(|m| m.words).sum(),
            raw_words: raw.iter().map(|m| m.words).sum(),
            raw_dictations: raw.len(),
            um_uh: raw.iter().map(|m| m.um_uh).sum(),
            phrase_fillers: raw.iter().map(|m| m.phrase_fillers).sum(),
            repeats: raw.iter().map(|m| m.repeats).sum(),
            paced_dictations: paces.len(),
            pace: paces.get(paces.len() / 2).copied(),
            fast_pace: if paces.len() < 4 {
                None
            } else {
                Some(paces[paces.len() * 3 / 4])
            },
            median_sentence: lengths.get(lengths.len() / 2).copied(),
            longest_sentence: lengths.last().copied(),
            clarity_dictations: ratios.len(),
            polish_fixes: ratios.iter().filter(|&&r| r > 0.15).count(),
        }
    }

    /// um and uh per 100 words.
    pub fn um_uh_rate(&self) -> f64 {
        if self.raw_words == 0 {
            0.0
        } else {
            self.um_uh as f64 / self.raw_words as f64 * 100.0
        }
    }

    pub fn phrase_rate(&self) -> f64 {
        if self.raw_words == 0 {
            0.0
        } else {
            self.phrase_fillers as f64 / self.raw_words as f64 * 100.0
        }
    }

    pub fn repeat_rate(&self) -> f64 {
        if self.raw_words == 0 {
            0.0
        } else {
            self.repeats as f64 / self.raw_words as f64 * 100.0
        }
    }

    /// Share of dictations Polish rewrote by more than 15%.
    pub fn polish_fix_share(&self) -> f64 {
        if self.clarity_dictations == 0 {
            0.0
        } else {
            self.polish_fixes as f64 / self.clarity_dictations as f64
        }
    }
}

/// What to work on next, most needed first (Mac `CoachFocus`).
pub mod focus {
    use super::*;

    pub const UM_UH_GOAL: f64 = 1.0;

    pub fn needs(s: &CoachSummary) -> Vec<(CoachMeasure, f64)> {
        let mut needs: Vec<(CoachMeasure, f64)> = Vec::new();
        let enough = s.raw_dictations >= CoachMeasure::MINIMUM_SAMPLES;
        if enough && s.um_uh_rate() > UM_UH_GOAL {
            needs.push((CoachMeasure::Fillers, s.um_uh_rate() / UM_UH_GOAL));
        }
        let median = s.median_sentence.unwrap_or(0) as f64;
        let longest = s.longest_sentence.unwrap_or(0) as f64;
        if enough && median > 14.0 {
            needs.push((
                CoachMeasure::Sentences,
                0.9 * (median / 14.0).max(longest / 40.0),
            ));
        }
        if s.paced_dictations >= CoachMeasure::MINIMUM_SAMPLES {
            if let Some(fast) = s.fast_pace.filter(|&f| f > 160) {
                needs.push((CoachMeasure::Pace, 0.8 * fast as f64 / 160.0));
            }
        }
        if enough && s.phrase_rate() > 1.0 {
            needs.push((CoachMeasure::Phrases, 0.85 * s.phrase_rate()));
        }
        if enough && s.repeat_rate() > 0.5 {
            needs.push((CoachMeasure::Repeats, 0.7 * s.repeat_rate() / 0.5));
        }
        if s.clarity_dictations >= CoachMeasure::MINIMUM_SAMPLES && s.polish_fix_share() > 0.3 {
            needs.push((CoachMeasure::Clarity, 0.7 * s.polish_fix_share() / 0.3));
        }
        // Stable, like Swift's sort on these small lists.
        needs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        needs
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn measures(
    words: i64,
    um_uh: i64,
    phrases: i64,
    wpm: Option<i64>,
    sentences: &[i64],
    repeats: i64,
    polish: Option<f64>,
    raw: bool,
) -> DictationMeasures {
    DictationMeasures {
        words,
        um_uh,
        phrase_fillers: phrases,
        wpm,
        sentence_lengths: sentences.to_vec(),
        repeats,
        polish_change_ratio: polish,
        raw_known: raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_m() -> DictationMeasures {
        measures(40, 0, 0, Some(145), &[10, 12], 0, Some(0.05), true)
    }

    #[test]
    fn summary_adds_up_like_orttakey() {
        let all = [
            measures(100, 3, 0, Some(120), &[10, 20], 0, Some(0.05), true),
            measures(100, 1, 0, Some(150), &[30], 0, Some(0.05), true),
            measures(50, 9, 0, Some(200), &[5], 0, Some(0.05), false),
            measures(100, 0, 0, Some(170), &[14], 0, Some(0.05), true),
        ];
        let s = CoachSummary::new(&all);
        assert_eq!(s.dictations, 4);
        assert_eq!(s.words, 350);
        assert_eq!(s.raw_words, 300);
        assert_eq!(s.um_uh, 4);
        assert!((s.um_uh_rate() - 4.0 / 3.0).abs() < 0.0001);
        assert_eq!(s.pace, Some(170));
        assert_eq!(s.fast_pace, Some(200));
        assert_eq!(s.median_sentence, Some(20));
        assert_eq!(s.longest_sentence, Some(30));
    }

    #[test]
    fn axes_need_eight_dictations() {
        let few = CoachSummary::new(&vec![std_m(); 7]);
        assert_eq!(CoachMeasure::Fillers.axis_score(&few), None);
        let enough = CoachSummary::new(&vec![std_m(); 8]);
        assert_eq!(CoachMeasure::Fillers.axis_score(&enough), Some(1.0));
        assert_eq!(CoachMeasure::Pace.axis_score(&enough), Some(1.0));
        assert_eq!(CoachMeasure::Sentences.axis_score(&enough), Some(1.0));
    }

    #[test]
    fn axis_scales_are_fixed() {
        let s = CoachSummary::new(&vec![
            measures(
                100,
                2,
                2,
                Some(190),
                &[22],
                1,
                Some(0.4),
                true
            );
            10
        ]);
        let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 0.0001;
        assert!(close(CoachMeasure::Fillers.axis_score(&s), 0.6));
        assert!(close(CoachMeasure::Phrases.axis_score(&s), 0.5));
        assert!(close(CoachMeasure::Pace.axis_score(&s), 0.5));
        assert!(close(CoachMeasure::Sentences.axis_score(&s), 0.5));
        assert!(close(CoachMeasure::Repeats.axis_score(&s), 0.5));
        assert_eq!(CoachMeasure::Clarity.axis_score(&s), Some(0.0));
    }

    #[test]
    fn focus_uses_orttakey_scores_first() {
        let s = CoachSummary::new(&vec![
            measures(
                100,
                3,
                0,
                Some(180),
                &[16],
                0,
                Some(0.05),
                true
            );
            10
        ]);
        let needs = focus::needs(&s);
        assert_eq!(
            needs.iter().map(|n| n.0).collect::<Vec<_>>(),
            vec![
                CoachMeasure::Fillers,
                CoachMeasure::Sentences,
                CoachMeasure::Pace
            ]
        );
        assert!((needs[0].1 - 3.0).abs() < 0.0001);
        assert!((needs[1].1 - 0.9 * 16.0 / 14.0).abs() < 0.0001);
        assert!((needs[2].1 - 0.8 * 180.0 / 160.0).abs() < 0.0001);
        assert!(focus::needs(&CoachSummary::new(&vec![std_m(); 10])).is_empty());
        let one_long = CoachSummary::new(&vec![
            measures(
                40,
                0,
                0,
                Some(145),
                &[8, 10, 45],
                0,
                Some(0.05),
                true
            );
            10
        ]);
        assert!(!focus::needs(&one_long)
            .iter()
            .any(|n| n.0 == CoachMeasure::Sentences));
    }

    #[test]
    fn unknown_raw_leaves_text_measures_out() {
        let m = super::super::counting::DictationMeasures::measuring(
            &super::super::counting::CoachDictation {
                created_at: 0,
                heard: "The plan is done and we ship it on Friday after the review.".into(),
                written: "The plan is done and we ship it on Friday after the review.".into(),
                recording_ms: 5_000,
                raw_known: false,
            },
        );
        assert_eq!(m.polish_change_ratio, None);
        assert_eq!(CoachMeasure::Fillers.value(&m), None);
        assert_eq!(CoachMeasure::Sentences.value(&m), None);
        assert_eq!(CoachMeasure::Repeats.value(&m), None);
        assert_eq!(CoachMeasure::Pace.value(&m), Some(156.0));
    }
}
