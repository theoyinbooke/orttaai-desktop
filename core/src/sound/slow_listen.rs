//! Sound slowed down with its pitch kept, for listening again to a word the
//! speech model may have heard too fast ("emailer" written "email"). The Mac
//! and iPhone use `AVAudioUnitTimePitch`; this is WSOLA (waveform-similarity
//! overlap-add) in plain Rust: windows of the sound are laid down further
//! apart than they were taken, each one nudged to where it best continues
//! the one before, so the pitch stays and only the pace changes.

/// 32 ms windows at 16 kHz, laid down every 16 ms.
const WINDOW: usize = 512;
const SYNTHESIS_HOP: usize = WINDOW / 2;
/// How far a window may be nudged either way to line up with the last: 10 ms,
/// a whole period of the lowest voices.
const SEARCH: usize = 160;
/// The comparison skips samples: the best lag is found on every second one.
const COMPARE_STEP: usize = 2;

/// 16 kHz mono samples played at `rate` (0.6 = 60% of the speed), pitch kept.
/// `None` for an empty clip or a rate outside 0.25...4.
pub fn stretched(samples: &[f32], rate: f32) -> Option<Vec<f32>> {
    if samples.is_empty() || !rate.is_finite() || !(0.25..=4.0).contains(&rate) {
        return None;
    }
    if samples.len() < WINDOW + 2 * SEARCH {
        // Too short to line anything up: nothing to slow.
        return Some(samples.to_vec());
    }
    let out_len = (samples.len() as f64 / f64::from(rate)).round() as usize;
    // Silence after the end, so the last windows are taken from where they
    // belong instead of repeating the clip's final 32 ms.
    let mut padded = Vec::with_capacity(samples.len() + WINDOW);
    padded.extend_from_slice(samples);
    padded.resize(samples.len() + WINDOW, 0.0);
    let samples = &padded[..];
    let window: Vec<f32> = (0..WINDOW)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / WINDOW as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();
    let mut out = vec![0.0f32; out_len + WINDOW];
    let mut weight = vec![0.0f32; out_len + WINDOW];
    let last_start = samples.len() - WINDOW;
    // Where the last window was taken from: its natural continuation is what
    // the next one is lined up with.
    let mut previous = 0usize;
    let mut frame = 0usize;
    loop {
        let out_at = frame * SYNTHESIS_HOP;
        if out_at >= out_len {
            break;
        }
        let ideal = (out_at as f64 * f64::from(rate)).round() as usize;
        let taken = if frame == 0 {
            0
        } else {
            let natural = (previous + SYNTHESIS_HOP).min(last_start);
            best_start(samples, natural, ideal.min(last_start), last_start)
        };
        for i in 0..WINDOW {
            out[out_at + i] += samples[taken + i] * window[i];
            weight[out_at + i] += window[i];
        }
        previous = taken;
        frame += 1;
    }
    out.truncate(out_len);
    for (sample, w) in out.iter_mut().zip(&weight) {
        if *w > 1e-3 {
            *sample /= w;
        }
    }
    Some(out)
}

/// The start within `SEARCH` of `ideal` whose window is most like the one at
/// `natural` (normalised cross-correlation).
fn best_start(samples: &[f32], natural: usize, ideal: usize, last_start: usize) -> usize {
    let lower = ideal.saturating_sub(SEARCH);
    let upper = (ideal + SEARCH).min(last_start);
    if (lower..=upper).contains(&natural) {
        // The continuation itself is in reach: nothing lines up better.
        return natural;
    }
    let reference = &samples[natural..natural + WINDOW];
    let mut best = ideal;
    let mut best_score = f32::NEG_INFINITY;
    for start in lower..=upper {
        let candidate = &samples[start..start + WINDOW];
        let (mut dot, mut energy) = (0.0f32, 1e-9f32);
        let mut i = 0;
        while i < WINDOW {
            dot += reference[i] * candidate[i];
            energy += candidate[i] * candidate[i];
            i += COMPARE_STEP;
        }
        let score = dot / energy.sqrt();
        if score > best_score {
            best_score = score;
            best = start;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| 0.4 * (2.0 * std::f32::consts::PI * hz * i as f32 / 16_000.0).sin())
            .collect()
    }

    /// Upward zero crossings a second: the pitch of a pure tone.
    fn pitch(samples: &[f32]) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f32 / (samples.len() as f32 / 16_000.0)
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn slowed_sound_is_longer_and_keeps_its_pitch_and_level() {
        for hz in [110.0, 220.0, 440.0, 1_000.0] {
            let clip = tone(hz, 1.5);
            for rate in SLOW_RATES {
                let slow = stretched(&clip, rate).unwrap();
                let expected = (clip.len() as f32 / rate).round() as usize;
                assert_eq!(slow.len(), expected, "{hz} Hz at {rate}");
                let heard = pitch(&slow[2_000..slow.len() - 2_000]);
                assert!((heard - hz).abs() < hz * 0.03, "{hz} Hz came out {heard}");
                let level = rms(&slow[2_000..slow.len() - 2_000]) / rms(&clip);
                assert!((0.9..1.1).contains(&level), "{hz} Hz level {level}");
            }
        }
    }
    const SLOW_RATES: [f32; 2] = crate::sound::judge::SLOW_LISTEN_RATES;

    #[test]
    fn silence_stays_silent_and_edges_are_whole() {
        let slow = stretched(&vec![0.0; 16_000], 0.6).unwrap();
        assert!(slow.iter().all(|s| *s == 0.0));
        // The first and last samples aren't faded away by the windows.
        let clip = tone(300.0, 1.0);
        let slow = stretched(&clip, 0.7).unwrap();
        assert!(rms(&slow[..800]) > 0.2 && rms(&slow[slow.len() - 800..]) > 0.2);
        assert!(slow.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    #[test]
    fn a_click_is_not_doubled() {
        // One short burst in silence comes out as one burst, later.
        let mut clip = vec![0.0f32; 16_000];
        for (i, s) in clip[8_000..8_080].iter_mut().enumerate() {
            *s = if i % 2 == 0 { 0.8 } else { -0.8 };
        }
        let slow = stretched(&clip, 0.6).unwrap();
        let loud: Vec<usize> = (0..slow.len()).filter(|&i| slow[i].abs() > 0.2).collect();
        let (first, last) = (*loud.first().unwrap(), *loud.last().unwrap());
        assert!(last - first < 700, "burst spread over {}", last - first);
        assert!((12_500..14_200).contains(&first), "burst at {first}");
    }

    #[test]
    fn odd_input_is_refused_or_returned() {
        assert!(stretched(&[], 0.6).is_none());
        assert!(stretched(&[0.1; 4_000], 0.0).is_none());
        assert!(stretched(&[0.1; 4_000], f32::NAN).is_none());
        assert_eq!(stretched(&[0.1; 300], 0.6).unwrap().len(), 300);
        // Speeding up works too (never used, but never wrong).
        assert_eq!(stretched(&tone(200.0, 1.0), 2.0).unwrap().len(), 8_000);
    }
}
