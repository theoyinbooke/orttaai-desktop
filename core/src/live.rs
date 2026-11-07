//! Decoding while the speaker is still talking (Mac `TranscriptionService` live
//! session): the recording is committed in pieces as it goes, so releasing the
//! key leaves only a short tail to decode however long the dictation ran.
//!
//! - Whisper commits a clip at a speech pause (0.7 s of quiet after at least
//!   2 s of audio), or at 15 s, cut at the latest 200 ms gap in its final 3 s.
//!   Each clip is decoded with the dictionary bias and the text so far as its
//!   prompt.
//! - Express (the Mac's "Decode while I speak", off by default) commits
//!   segments of at least 8 s at a 0.7 s pause, or past 20 s at the latest
//!   200 ms gap between words. It never cuts a word: continuous speech waits.
//!
//! Only one piece decodes at a time; a piece that fails stays uncommitted and
//! is simply decoded with the tail at the end.

use crate::transcription::{bias_prompt_for, Transcriber};
use crate::types::DecodeOptions;
use crate::vad::{frame_rms, FRAME_SAMPLES, SAMPLE_RATE, SPEECH_ENERGY_THRESHOLD};
use std::sync::Arc;
use std::thread::JoinHandle;

/// How a transcriber commits pieces while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveMode {
    /// Whisper: clips of up to 15 s and pause commits.
    Clips,
    /// Express: long pause-bounded segments.
    Segments,
    /// Decode the whole recording at the end.
    Off,
}

/// Below this RMS a frame is dead silence (muted or absent mic).
pub const FAINT_ENERGY_FLOOR: f32 = 0.005;
const MAX_PAUSE_ENERGY: f32 = 0.009;
pub const CLIP_SAMPLES: usize = 15 * SAMPLE_RATE;
const BOUNDARY_SEARCH: usize = 3 * SAMPLE_RATE;
const BOUNDARY_GAP: usize = SAMPLE_RATE / 5;
pub const PAUSE_SILENCE: usize = SAMPLE_RATE * 7 / 10;
const SILENCE_PAD: usize = SAMPLE_RATE * 3 / 10;
pub const PAUSE_MIN_CLIP: usize = 2 * SAMPLE_RATE;
pub const SEGMENT_MIN: usize = 8 * SAMPLE_RATE;
const SEGMENT_GAP_SEARCH: usize = 20 * SAMPLE_RATE;
/// The committed text carried into the next clip's prompt (about 100 tokens).
const CONTEXT_CHARS: usize = 400;

/// The pause threshold for `samples`: the quietest tenth of its 100 ms frames,
/// with a small margin for room noise, capped below quiet speech.
pub fn pause_threshold(samples: &[f32]) -> f32 {
    let mut energies: Vec<f32> = samples.chunks_exact(FRAME_SAMPLES).map(frame_rms).collect();
    if energies.is_empty() {
        return FAINT_ENERGY_FLOOR;
    }
    energies.sort_by(f32::total_cmp);
    let background = energies[(energies.len() - 1) / 10];
    (background * 1.8).clamp(FAINT_ENERGY_FLOOR, MAX_PAUSE_ENERGY)
}

/// Samples up to the end of the last 100 ms frame at or above `threshold`.
pub fn last_speech_end(samples: &[f32], threshold: f32) -> Option<usize> {
    let mut end = samples.len();
    while end > 0 {
        let start = end.saturating_sub(FRAME_SAMPLES);
        if frame_rms(&samples[start..end]) >= threshold {
            return Some(end);
        }
        end = start;
    }
    None
}

pub fn has_speech(samples: &[f32], threshold: f32) -> bool {
    last_speech_end(samples, threshold).is_some()
}

/// Where to end a regular clip of at least 15 s: the latest 200 ms gap (with
/// speech resuming after it) in its final 3 s, else the 15 s mark.
pub fn clip_len(pending: &[f32]) -> usize {
    let limit = pending.len().min(CLIP_SAMPLES);
    if pending.len() < CLIP_SAMPLES {
        return limit;
    }
    let threshold = pause_threshold(&pending[..limit]);
    let mut quiet = 0;
    let mut boundary = limit;
    let mut frame = limit - BOUNDARY_SEARCH;
    while frame + FRAME_SAMPLES <= limit {
        if frame_rms(&pending[frame..frame + FRAME_SAMPLES]) < threshold {
            quiet += FRAME_SAMPLES;
        } else {
            if quiet >= BOUNDARY_GAP {
                boundary = frame - BOUNDARY_GAP / 2;
            }
            quiet = 0;
        }
        frame += FRAME_SAMPLES;
    }
    boundary
}

/// A clip ending at the pause the speaker is making now, or `None` to wait.
pub fn pause_commit_len(pending: &[f32]) -> Option<usize> {
    if pending.len() < PAUSE_MIN_CLIP + PAUSE_SILENCE
        || !has_speech(pending, SPEECH_ENERGY_THRESHOLD)
    {
        return None;
    }
    let last = last_speech_end(pending, pause_threshold(pending))?;
    if pending.len() - last < PAUSE_SILENCE {
        return None;
    }
    let len = pending.len().min(last + SILENCE_PAD);
    (len >= PAUSE_MIN_CLIP).then_some(len)
}

/// Where to end an Express segment, or `None` to wait.
pub fn segment_len(pending: &[f32]) -> Option<usize> {
    if pending.len() < SEGMENT_MIN + PAUSE_SILENCE {
        return None;
    }
    if let Some(pause) = pause_commit_len(pending).filter(|&p| p >= SEGMENT_MIN) {
        return Some(pause);
    }
    if pending.len() < SEGMENT_GAP_SEARCH {
        return None;
    }
    let threshold = pause_threshold(pending);
    let mut quiet = 0;
    let mut boundary = None;
    let mut frame = SEGMENT_MIN;
    while frame + FRAME_SAMPLES <= pending.len() {
        if frame_rms(&pending[frame..frame + FRAME_SAMPLES]) < threshold {
            quiet += FRAME_SAMPLES;
        } else {
            if quiet >= BOUNDARY_GAP {
                boundary = Some(frame - quiet / 2);
            }
            quiet = 0;
        }
        frame += FRAME_SAMPLES;
    }
    boundary
}

/// How much of `pending` to commit now, if any.
pub fn next_commit(pending: &[f32], mode: LiveMode) -> Option<usize> {
    match mode {
        LiveMode::Clips if pending.len() >= CLIP_SAMPLES => Some(clip_len(pending)),
        LiveMode::Clips => pause_commit_len(pending),
        LiveMode::Segments => segment_len(pending),
        LiveMode::Off => None,
    }
}

/// The text a decoded piece commits: its words, or empty for a silent piece.
/// `None` when a piece with sound in it decoded to nothing, so it stays
/// uncommitted and is decoded again with the tail (Mac `liveCommitText`).
pub fn commit_text(decoded: &str, audio: &[f32]) -> Option<String> {
    let text = crate::coordinator::strip_nonspeech_markers(decoded.trim());
    if !text.is_empty() {
        return Some(text);
    }
    (!has_speech(audio, FAINT_ENERGY_FLOOR)).then(String::new)
}

/// The prompt for a piece: the dictionary bias, then the end of what has been
/// committed so far, so the decoder continues the same sentence.
pub fn piece_prompt(vocabulary: &[String], clip: &[f32], committed: &[String]) -> Option<String> {
    let bias = bias_prompt_for(vocabulary, clip);
    let context = committed.join(" ");
    let context = context.trim();
    let tail = if context.chars().count() > CONTEXT_CHARS {
        let skip = context.chars().count() - CONTEXT_CHARS;
        let rest: String = context.chars().skip(skip).collect();
        // Start at a word.
        rest.split_once(' ')
            .map(|(_, r)| r.to_string())
            .unwrap_or(rest)
    } else {
        context.to_string()
    };
    match (bias, tail.is_empty()) {
        (Some(b), false) => Some(format!("{b} {tail}")),
        (Some(b), true) => Some(b),
        (None, false) => Some(tail),
        (None, true) => None,
    }
}

struct Inflight {
    start: usize,
    clip: Vec<f32>,
    handle: JoinHandle<crate::error::Result<String>>,
}

/// One recording's committed pieces.
pub struct LiveSession {
    mode: LiveMode,
    committed: Vec<String>,
    committed_samples: usize,
    inflight: Option<Inflight>,
}

impl LiveSession {
    pub fn new(mode: LiveMode) -> Self {
        Self {
            mode,
            committed: Vec::new(),
            committed_samples: 0,
            inflight: None,
        }
    }

    pub fn mode(&self) -> LiveMode {
        self.mode
    }

    /// Committed text so far, and how many samples it covers.
    pub fn committed(&self) -> (&[String], usize) {
        (&self.committed, self.committed_samples)
    }

    /// Smallest uncommitted length worth looking at.
    pub fn min_pending(&self) -> usize {
        match self.mode {
            LiveMode::Clips => PAUSE_MIN_CLIP + PAUSE_SILENCE,
            LiveMode::Segments => SEGMENT_MIN + PAUSE_SILENCE,
            LiveMode::Off => usize::MAX,
        }
    }

    /// Fold in a finished piece; with `block`, wait for the one in flight.
    pub fn collect(&mut self, block: bool) {
        let done = self
            .inflight
            .as_ref()
            .is_some_and(|f| block || f.handle.is_finished());
        if !done {
            return;
        }
        let Some(piece) = self.inflight.take() else {
            return;
        };
        match piece.handle.join() {
            Ok(Ok(decoded)) => {
                if piece.start == self.committed_samples {
                    if let Some(text) = commit_text(&decoded, &piece.clip) {
                        if !text.is_empty() {
                            self.committed.push(text);
                        }
                        self.committed_samples = piece.start + piece.clip.len();
                    }
                }
            }
            Ok(Err(e)) => tracing::debug!("live piece not committed: {e}"),
            Err(_) => tracing::warn!("live decode thread panicked"),
        }
    }

    /// Whether a piece is decoding.
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// Start decoding the next piece of `pending` (the audio after the committed
    /// part), when one is due.
    pub fn start_next(
        &mut self,
        pending: &[f32],
        transcriber: &Arc<dyn Transcriber>,
        options: &DecodeOptions,
        vocabulary: &[String],
    ) -> bool {
        if self.inflight.is_some() {
            return false;
        }
        let Some(len) = next_commit(pending, self.mode) else {
            return false;
        };
        let clip = pending[..len].to_vec();
        let mut opts = options.clone();
        opts.initial_prompt = piece_prompt(vocabulary, &clip, &self.committed);
        let transcriber = Arc::clone(transcriber);
        let audio = clip.clone();
        let handle = std::thread::Builder::new()
            .name("orttaai-live-decode".into())
            .spawn(move || transcriber.transcribe(&audio, &opts));
        match handle {
            Ok(handle) => {
                self.inflight = Some(Inflight {
                    start: self.committed_samples,
                    clip,
                    handle,
                });
                true
            }
            Err(e) => {
                tracing::warn!("couldn't start a live decode: {e}");
                false
            }
        }
    }

    /// Wait for the piece in flight and hand back what was committed.
    pub fn finish(mut self) -> (Vec<String>, usize) {
        self.collect(true);
        (self.committed, self.committed_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcription::MockTranscriber;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f32) as usize;
        (0..n).map(|i| amp * (i as f32 * 0.2).sin()).collect()
    }

    fn quiet(secs: f32) -> Vec<f32> {
        vec![0.0; (secs * SAMPLE_RATE as f32) as usize]
    }

    #[test]
    fn commits_at_a_pause_after_two_seconds() {
        let mut audio = tone(2.5, 0.2);
        audio.extend(quiet(0.5));
        assert_eq!(pause_commit_len(&audio), None, "pause still too short");
        audio.extend(quiet(0.3));
        let len = pause_commit_len(&audio).expect("0.8 s pause commits");
        assert!(len >= 2 * SAMPLE_RATE && len <= audio.len());
        assert!(
            (len as f32 / SAMPLE_RATE as f32 - 2.8).abs() < 0.15,
            "speech + 0.3 s pad"
        );
    }

    #[test]
    fn short_or_silent_audio_never_commits() {
        assert_eq!(pause_commit_len(&quiet(5.0)), None);
        let mut short = tone(1.0, 0.2);
        short.extend(quiet(1.0));
        assert_eq!(pause_commit_len(&short), None);
    }

    #[test]
    fn long_clips_cut_at_a_gap_near_fifteen_seconds() {
        let mut audio = tone(13.0, 0.2);
        audio.extend(quiet(0.4));
        audio.extend(tone(4.0, 0.2));
        let len = clip_len(&audio);
        let secs = len as f32 / SAMPLE_RATE as f32;
        assert!(secs > 13.0 && secs < 13.4, "cut inside the gap, at {secs}");
        let unbroken = tone(17.0, 0.2);
        assert_eq!(clip_len(&unbroken), CLIP_SAMPLES, "no gap: the 15 s mark");
    }

    #[test]
    fn express_segments_wait_for_eight_seconds_and_a_pause() {
        let mut audio = tone(6.0, 0.2);
        audio.extend(quiet(1.0));
        assert_eq!(segment_len(&audio), None, "under 8 s");
        let mut long = tone(9.0, 0.2);
        long.extend(quiet(1.0));
        assert!(segment_len(&long).is_some());
    }

    #[test]
    fn silent_pieces_commit_empty_and_failed_ones_wait() {
        assert_eq!(commit_text("", &quiet(2.0)).as_deref(), Some(""));
        assert_eq!(commit_text("[BLANK_AUDIO]", &tone(2.0, 0.2)), None);
        assert_eq!(
            commit_text(" hello ", &tone(2.0, 0.2)).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn prompt_carries_the_end_of_the_committed_text() {
        let committed = vec!["a ".repeat(300)];
        let p = piece_prompt(&[], &tone(1.0, 0.2), &committed).unwrap();
        assert!(p.chars().count() <= CONTEXT_CHARS);
        assert_eq!(piece_prompt(&[], &tone(1.0, 0.2), &[]), None);
    }

    #[test]
    fn session_commits_pieces_in_order() {
        let t: Arc<dyn Transcriber> = Arc::new(MockTranscriber::new("piece"));
        let mut s = LiveSession::new(LiveMode::Clips);
        let mut audio = tone(2.5, 0.2);
        audio.extend(quiet(0.8));
        assert!(s.start_next(&audio, &t, &DecodeOptions::default(), &[]));
        assert!(
            !s.start_next(&audio, &t, &DecodeOptions::default(), &[]),
            "one at a time"
        );
        let (texts, covered) = s.finish();
        assert_eq!(texts, vec!["piece".to_string()]);
        assert!(covered > 2 * SAMPLE_RATE && covered <= audio.len());
    }
}
