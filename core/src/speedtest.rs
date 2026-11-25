//! The speed test in Settings › Model › Performance: times one speech decode
//! with a few thread counts on this computer and keeps the fastest. The
//! thread rule in [`crate::cpu`] is a good default, but PCs differ too much
//! (core mix, cache, power plan, thermal limits) for any rule to be right on
//! every one of them; measuring is.

use crate::cpu::CoreCounts;
use crate::settings::Settings;
use crate::types::TARGET_SAMPLE_RATE;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A finished speed test, kept in the settings so Performance can show it and
/// the offer card knows it ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedTestRecord {
    /// The model that was timed, as the UI names it.
    pub model: String,
    /// The fastest thread count, saved as Decode Threads.
    pub best: usize,
    pub best_ms: u64,
    /// What Auto would use, and how long it took.
    pub recommended: usize,
    pub recommended_ms: Option<u64>,
    /// Fluid Speech Mini with integer maths at the best thread count, when
    /// timed, and whether it was kept (see [`int8_math_wins`]).
    #[serde(default)]
    pub int8_math_ms: Option<u64>,
    #[serde(default)]
    pub int8_math: bool,
    /// Unix seconds.
    pub at: i64,
}

/// Integer maths is kept only when it saves at least a tenth of the time:
/// a closer result is within a run's noise, and the usual encoder is the
/// one every other test was made with.
pub fn int8_math_wins(usual: Duration, int8: Duration) -> bool {
    int8.as_secs_f64() <= usual.as_secs_f64() * 0.9
}

/// Whether to offer the speed test on a card: after setup, with a speech
/// model ready to time, once. Not after a test or "Not now". A Decode
/// Threads count chosen by hand is left alone unless it's more than the
/// computer's `fast_cores`: past them the extra threads land on slow or
/// shared cores, and decoding usually gets slower, not faster.
pub fn should_offer(settings: &Settings, model_ready: bool, fast_cores: Option<usize>) -> bool {
    let threads = settings.decode.n_threads;
    let auto_or_too_many = threads <= 0 || fast_cores.is_some_and(|fast| threads as usize > fast);
    settings.has_completed_setup
        && model_ready
        && !settings.speed_test_offer_dismissed
        && settings.last_speed_test.is_none()
        && auto_or_too_many
}

/// Seconds of sample audio each trial decodes: long enough that the encoder
/// dominates, as in a real dictation, short enough for the whole test to take
/// about a minute on a slow laptop.
pub const SAMPLE_SECONDS: f32 = 8.0;

/// Two results this close (5 %) are a tie, and the tie goes to fewer threads:
/// the same speed for less power and fewer cores kept busy.
const TIE_MARGIN: f64 = 1.05;

/// The thread counts to try: the recommended count, the fast and all
/// physical cores, and 2, 4, 6 and 8, within the logical CPUs and at most
/// eight, smallest first.
pub fn candidates(recommended: usize, counts: Option<CoreCounts>, logical: usize) -> Vec<usize> {
    let limit = logical.clamp(1, 8);
    let mut all = vec![2, 4, 6, 8, recommended];
    if let Some(c) = counts {
        all.extend([c.performance, c.total]);
    }
    let mut picked: Vec<usize> = all.into_iter().filter(|&n| n >= 1 && n <= limit).collect();
    if picked.is_empty() {
        picked.push(limit);
    }
    picked.sort_unstable();
    picked.dedup();
    picked
}

/// The fastest trial's thread count, preferring fewer threads within the
/// tie margin. `None` for no trials.
pub fn fastest(trials: &[(usize, Duration)]) -> Option<usize> {
    let best = trials.iter().map(|&(_, d)| d).min()?;
    let bar = best.as_secs_f64() * TIE_MARGIN;
    trials
        .iter()
        .filter(|&&(_, d)| d.as_secs_f64() <= bar)
        .map(|&(threads, _)| threads)
        .min()
}

/// Deterministic speech-like audio: voiced syllables (a pitch with formant
/// harmonics, four a second) with short gaps, at a normal speaking level.
/// What matters for timing is that the models do a real decode's work.
pub fn sample_audio(seconds: f32) -> Vec<f32> {
    let rate = TARGET_SAMPLE_RATE as f32;
    let n = (seconds * rate) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / rate;
        // Four syllables a second, each 0.18 s of voice then a short gap.
        let phase = (t * 4.0).fract();
        let envelope = if phase < 0.72 {
            (phase / 0.72 * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        // The pitch drifts between syllables like intonation.
        let syllable = (t * 4.0).floor();
        let pitch = 110.0 + 30.0 * (syllable * 1.7).sin();
        let mut voice = 0.0;
        for (harmonic, weight) in [(1.0, 1.0), (2.0, 0.6), (3.0, 0.45), (5.0, 0.3), (8.0, 0.15)] {
            voice += weight * (2.0 * std::f32::consts::PI * pitch * harmonic * t).sin();
        }
        out.push(0.08 * envelope * voice);
    }
    out
}

/// Times `decode` on `samples`: one untimed run to warm caches, then the
/// faster of two timed runs.
pub fn time_decode<E>(
    mut decode: impl FnMut(&[f32]) -> Result<(), E>,
    samples: &[f32],
) -> Result<Duration, E> {
    decode(samples)?;
    let mut best = Duration::MAX;
    for _ in 0..2 {
        let started = std::time::Instant::now();
        decode(samples)?;
        best = best.min(started.elapsed());
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cores(performance: usize, total: usize) -> Option<CoreCounts> {
        Some(CoreCounts { performance, total })
    }

    #[test]
    fn the_offer_shows_once_after_setup_with_a_model() {
        let mut s = Settings {
            has_completed_setup: true,
            ..Default::default()
        };
        let fast = Some(4);
        assert!(should_offer(&s, true, fast));
        assert!(!should_offer(&s, false, fast), "no model to time yet");
        s.has_completed_setup = false;
        assert!(!should_offer(&s, true, fast), "setup still running");
        s.has_completed_setup = true;
        s.decode.n_threads = 4;
        assert!(!should_offer(&s, true, fast), "threads already chosen");
        s.decode.n_threads = 8;
        assert!(
            should_offer(&s, true, fast),
            "8 threads on 4 fast cores: worth testing"
        );
        assert!(!should_offer(&s, true, None), "unknown cores: left alone");
        s.decode.n_threads = 0;
        s.speed_test_offer_dismissed = true;
        assert!(!should_offer(&s, true, fast), "Not now");
        s.speed_test_offer_dismissed = false;
        s.last_speed_test = Some(SpeedTestRecord {
            model: "Express".into(),
            best: 4,
            best_ms: 900,
            recommended: 4,
            recommended_ms: Some(900),
            int8_math_ms: None,
            int8_math: false,
            at: 0,
        });
        assert!(!should_offer(&s, true, fast), "already ran");
    }

    #[test]
    fn candidates_cover_the_useful_counts() {
        // A 2 + 8 laptop with 12 logical CPUs, recommended 4.
        assert_eq!(candidates(4, cores(2, 10), 12), vec![2, 4, 6, 8]);
        // A 6 + 8 chip: its six fast cores are tried.
        assert_eq!(candidates(6, cores(6, 14), 20), vec![2, 4, 6, 8]);
        // Never more than the logical CPUs.
        assert_eq!(candidates(4, cores(4, 4), 4), vec![2, 4]);
        assert_eq!(candidates(1, cores(1, 1), 1), vec![1]);
        // A 5-core CPU tries five.
        assert_eq!(candidates(5, cores(5, 5), 10), vec![2, 4, 5, 6, 8]);
    }

    #[test]
    fn the_fastest_wins_and_ties_go_to_fewer_threads() {
        let ms = Duration::from_millis;
        assert_eq!(
            fastest(&[(2, ms(900)), (4, ms(500)), (6, ms(420)), (8, ms(460))]),
            Some(6)
        );
        // 4 threads within 5 % of 8: same speed, fewer cores.
        assert_eq!(fastest(&[(4, ms(410)), (8, ms(400))]), Some(4));
        assert_eq!(fastest(&[]), None);
    }

    #[test]
    fn sample_audio_is_speech_length_and_level() {
        let audio = sample_audio(SAMPLE_SECONDS);
        assert_eq!(
            audio.len(),
            (SAMPLE_SECONDS * TARGET_SAMPLE_RATE as f32) as usize
        );
        let peak = audio.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.1 && peak < 0.3, "{peak}");
        // Deterministic: the same every time.
        assert_eq!(audio, sample_audio(SAMPLE_SECONDS));
    }

    #[test]
    fn time_decode_warms_up_then_keeps_the_faster_run() {
        let mut runs = 0;
        let took = time_decode(
            |_| {
                runs += 1;
                Ok::<(), ()>(())
            },
            &[0.0; 4],
        )
        .unwrap();
        assert_eq!(runs, 3, "one warm-up and two timed runs");
        assert!(took < Duration::from_secs(1));
        assert!(time_decode(|_| Err::<(), _>("broken"), &[0.0]).is_err());
    }

    #[test]
    fn integer_maths_must_save_a_tenth() {
        let ms = Duration::from_millis;
        assert!(int8_math_wins(ms(1000), ms(900)));
        assert!(int8_math_wins(ms(1000), ms(500)));
        assert!(!int8_math_wins(ms(1000), ms(950)), "within noise");
        assert!(!int8_math_wins(ms(1000), ms(1100)));
    }
}
