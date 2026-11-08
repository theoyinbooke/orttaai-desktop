//! Word Boost's per-frame CTC scores in the same 15 s chunks with 2 s of
//! overlap that FluidAudio uses for long audio, so a recording can be scored
//! chunk by chunk while the user is still speaking. Stitched together, the
//! chunks give the scores FluidAudio computes for the whole recording at once.
//! A port of the Mac's `WordBoostChunks`.

use std::hash::{Hash, Hasher};
use std::ops::Range;

/// FluidAudio's `ASRConstants.maxModelSamples`: 15 s at 16 kHz.
pub const CHUNK_SAMPLE_COUNT: usize = 240_000;
/// FluidAudio's `CtcKeywordSpotter.chunkOverlapSamples`: 2 s.
pub const OVERLAP_SAMPLE_COUNT: usize = 32_000;
pub const STRIDE_SAMPLE_COUNT: usize = CHUNK_SAMPLE_COUNT - OVERLAP_SAMPLE_COUNT;
const SAMPLE_RATE: f64 = 16_000.0;

/// The chunks FluidAudio scores for a recording of `sample_count` samples:
/// one chunk up to 15 s, otherwise 15 s chunks every 13 s, the last one
/// ending at the end of the recording.
pub fn chunks(sample_count: usize) -> Vec<Range<usize>> {
    if sample_count == 0 {
        return Vec::new();
    }
    if sample_count <= CHUNK_SAMPLE_COUNT {
        return std::iter::once(0..sample_count).collect();
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < sample_count {
        let end = (start + CHUNK_SAMPLE_COUNT).min(sample_count);
        out.push(start..end);
        if end >= sample_count {
            break;
        }
        start += STRIDE_SAMPLE_COUNT;
    }
    out
}

/// The full 15 s chunks a recording that has reached `sample_count` samples
/// already contains. Each is also a chunk of the finished recording, however
/// long it grows.
pub fn complete_chunks(sample_count: usize) -> Vec<Range<usize>> {
    if sample_count < CHUNK_SAMPLE_COUNT {
        return Vec::new();
    }
    let count = (sample_count - CHUNK_SAMPLE_COUNT) / STRIDE_SAMPLE_COUNT + 1;
    (0..count)
        .map(|index| {
            let start = index * STRIDE_SAMPLE_COUNT;
            start..start + CHUNK_SAMPLE_COUNT
        })
        .collect()
}

/// Joins chunk scores the way FluidAudio does: frames where two chunks
/// overlap are averaged in probability space, and the first chunk's frame
/// duration sets the overlap length.
pub fn stitch(chunks: &[Vec<Vec<f32>>], frame_duration: f64) -> Vec<Vec<f32>> {
    if frame_duration.is_nan() || frame_duration <= 0.0 {
        return Vec::new();
    }
    let overlap_frames = (OVERLAP_SAMPLE_COUNT as f64 / SAMPLE_RATE / frame_duration) as usize;
    let mut stitched: Vec<Vec<f32>> = Vec::new();
    for (index, frames) in chunks.iter().enumerate() {
        if frames.is_empty() {
            continue;
        }
        if index == 0 {
            stitched.extend(frames.iter().cloned());
            continue;
        }
        let overlap = overlap_frames.min(stitched.len()).min(frames.len());
        let existing_start = stitched.len() - overlap;
        for offset in 0..overlap {
            let merged = averaged_frame(&stitched[existing_start + offset], &frames[offset]);
            stitched[existing_start + offset] = merged;
        }
        stitched.extend(frames[overlap..].iter().cloned());
    }
    stitched
}

/// `log((exp(a) + exp(b)) / 2)` for each class, computed stably. Like the Mac,
/// it keeps only as many classes as the shorter frame has.
pub fn averaged_frame(existing: &[f32], incoming: &[f32]) -> Vec<f32> {
    let count = existing.len().min(incoming.len());
    if count == 0 {
        return existing.to_vec();
    }
    use std::f32::consts::LN_2;
    (0..count)
        .map(|index| {
            let a = existing[index];
            let b = incoming[index];
            let m = a.max(b);
            if m == f32::NEG_INFINITY {
                return f32::NEG_INFINITY;
            }
            m + ((a - m).exp() + (b - m).exp()).ln() - LN_2
        })
        .collect()
}

/// Identifies a chunk's audio, so scores computed during recording are reused
/// only for exactly the same samples (the quiet-tail lift can change the last
/// 1.5 s before the final decode). Stable within a process.
pub fn fingerprint(samples: &[f32]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    samples.len().hash(&mut hasher);
    for sample in samples {
        sample.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    const SECOND: usize = 16_000;

    #[test]
    fn short_recording_is_one_chunk() {
        assert_eq!(chunks(5 * SECOND), vec![0..5 * SECOND]);
        assert_eq!(chunks(15 * SECOND), vec![0..15 * SECOND]);
        assert!(chunks(0).is_empty());
    }

    #[test]
    fn long_recording_is_fifteen_second_chunks_every_thirteen() {
        assert_eq!(
            chunks(30 * SECOND),
            vec![
                0..15 * SECOND,
                13 * SECOND..28 * SECOND,
                26 * SECOND..30 * SECOND
            ]
        );
    }

    #[test]
    fn every_complete_chunk_during_recording_is_a_chunk_of_the_finished_recording() {
        for recorded in (0..=90 * SECOND).step_by(3_333) {
            let complete = complete_chunks(recorded);
            for finished in [recorded, recorded + 1, recorded + 20 * SECOND] {
                let all = chunks(finished);
                for chunk in &complete {
                    assert!(
                        all.contains(chunk),
                        "{chunk:?} recorded at {recorded}, finished at {finished}"
                    );
                }
            }
        }
        assert!(complete_chunks(15 * SECOND - 1).is_empty());
        assert_eq!(
            complete_chunks(28 * SECOND),
            vec![0..15 * SECOND, 13 * SECOND..28 * SECOND]
        );
    }

    #[test]
    fn stitch_averages_overlapping_frames_in_probability_space() {
        // 0.5 s frames: the 2 s overlap is 4 frames.
        let first: Vec<Vec<f32>> = (0..6).map(|i| vec![-(i as f32), -1.0]).collect();
        let second: Vec<Vec<f32>> = (0..5).map(|i| vec![-10.0 - i as f32, -2.0]).collect();
        let stitched = stitch(&[first.clone(), second.clone()], 0.5);

        assert_eq!(stitched.len(), 6 + 5 - 4);
        assert_eq!(stitched[0], first[0]);
        assert_eq!(stitched[1], first[1]);
        for offset in 0..4 {
            let merged = &stitched[2 + offset];
            let expected = ((first[2 + offset][0].exp() + second[offset][0].exp()) / 2.0).ln();
            assert!((merged[0] - expected).abs() < 1e-5);
            let expected = (((-1f32).exp() + (-2f32).exp()) / 2.0).ln();
            assert!((merged[1] - expected).abs() < 1e-5);
        }
        assert_eq!(stitched[6], second[4]);
    }

    #[test]
    fn stitch_keeps_a_single_chunk_as_is() {
        let frames: Vec<Vec<f32>> = (0..3).map(|i| vec![i as f32]).collect();
        let one = std::slice::from_ref(&frames);
        assert_eq!(stitch(one, 0.08), frames);
        assert!(stitch(&[], 0.08).is_empty());
        assert!(stitch(one, 0.0).is_empty());
        assert!(stitch(one, f64::NAN).is_empty());
    }

    #[test]
    fn averaged_frame_handles_impossible_and_ragged_frames() {
        let inf = f32::NEG_INFINITY;
        assert_eq!(averaged_frame(&[inf, 0.0], &[inf, 0.0]), vec![inf, 0.0]);
        assert_eq!(averaged_frame(&[0.0, 0.0, 0.0], &[0.0]).len(), 1);
        assert_eq!(averaged_frame(&[1.0], &[]), vec![1.0]);
    }

    #[test]
    fn fingerprint_tells_changed_audio_apart() {
        let audio: Vec<f32> = (0..32_000).map(|i| (i as f32 * 0.07).sin() * 0.2).collect();
        let mut changed = audio.clone();
        *changed.last_mut().unwrap() += 0.001;
        assert_eq!(fingerprint(&audio), fingerprint(&audio.clone()));
        assert_ne!(fingerprint(&audio), fingerprint(&changed));
    }
}
