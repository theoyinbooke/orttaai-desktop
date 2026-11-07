//! Voice-activity detection for hands-free auto-stop: Silero (whisper.cpp VAD)
//! with an adaptive-RMS fallback.
//!
//! Port of the macOS `HandsFreeAutoStopPolicy` + `SileroVoiceDetector`.
//!
//! The policy uses the transcription pipeline's energy framing (100 ms frames)
//! and accounts for frames incrementally: [`AutoStopPolicy::advance`] scans only
//! the samples recorded since the previous call, so each poll costs a few frames
//! no matter how long the recording.
//!
//! What counts as speech adapts to the session. A fixed 0.02 RMS line cut people
//! off mid-sentence when their microphone ran quiet, so speech is a frame well
//! above the session's own background (2.5× its quietest 5%) and not far below
//! its loudest (a tenth of the peak), never stricter than 0.02 nor looser than
//! 0.0015. The recording arms only on sustained speech (300 ms — a click or a
//! cough never arms it) and stops only once the trailing silence since the last
//! speech frame reaches the configured window.
//!
//! Loudness can't tell a soft word from a quiet room, or a fan from a voice.
//! When the Silero voice detector is available its verdicts decide instead
//! ([`AutoStopPolicy::heard_voice`]); loudness stays as the fallback.

use crate::error::Result;
use std::time::Duration;

pub const SAMPLE_RATE: usize = 16_000;
/// 100 ms energy frames.
pub const FRAME_SAMPLES: usize = SAMPLE_RATE / 10;
/// The transcription pipeline's speech-onset energy (EnergyVAD threshold).
pub const SPEECH_ENERGY_THRESHOLD: f32 = 0.02;
/// Total speech-level audio required before the policy arms (300 ms).
pub const MIN_SPEECH_SAMPLES: usize = FRAME_SAMPLES * 3;
/// Never counts audio this quiet as speech: well above a muted or near-silent
/// input, below soft speech on a quiet microphone.
pub const QUIETEST_SPEECH: f32 = 0.0015;
const LOUDNESS_BINS: usize = 26;

/// Root-mean-square of a frame.
pub fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Whether any 100 ms frame of `samples` reaches the speech-onset energy —
/// used to skip conditioning (vocabulary-bias prompt) on silent audio, which
/// invites prompt bleed-through.
pub fn contains_speech_energy(samples: &[f32]) -> bool {
    samples
        .chunks(FRAME_SAMPLES)
        .any(|frame| frame_rms(frame) >= SPEECH_ENERGY_THRESHOLD)
}

/// Silence-based auto-stop policy for hands-free dictation, and the
/// per-session speech accounting it decides from.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoStopPolicy {
    scanned: usize,
    speech_frames: usize,
    peak_rms: f32,
    last_speech_end: Option<usize>,
    speech_threshold: f32,
    loudness: [u32; LOUDNESS_BINS],
    frames: usize,
    uses_voice_detector: bool,
    voice_scanned: usize,
    voice_speech: usize,
    last_voice_end: Option<usize>,
    voice_detector_failed: bool,
}

impl Default for AutoStopPolicy {
    fn default() -> Self {
        Self {
            scanned: 0,
            speech_frames: 0,
            peak_rms: 0.0,
            last_speech_end: None,
            speech_threshold: SPEECH_ENERGY_THRESHOLD,
            loudness: [0; LOUDNESS_BINS],
            frames: 0,
            uses_voice_detector: false,
            voice_scanned: 0,
            voice_speech: 0,
            last_voice_end: None,
            voice_detector_failed: false,
        }
    }
}

impl AutoStopPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples accounted for by loudness so far; always whole frames.
    pub fn scanned_samples(&self) -> usize {
        self.scanned
    }
    pub fn speech_frame_count(&self) -> usize {
        self.speech_frames
    }
    pub fn peak_frame_rms(&self) -> f32 {
        self.peak_rms
    }
    /// What counts as speech now.
    pub fn speech_threshold(&self) -> f32 {
        self.speech_threshold
    }
    /// True once the voice detector's verdicts decide, rather than loudness.
    pub fn uses_voice_detector(&self) -> bool {
        self.uses_voice_detector
    }
    /// Samples the voice detector has judged so far.
    pub fn voice_scanned_samples(&self) -> usize {
        self.voice_scanned
    }
    /// Audio the voice detector judged to be speech.
    pub fn voice_speech_samples(&self) -> usize {
        self.voice_speech
    }
    /// The voice detector failed during this recording; don't ask it again.
    pub fn voice_detector_failed(&self) -> bool {
        self.voice_detector_failed
    }

    /// True once enough speech has been heard for silence to mean "finished".
    pub fn is_armed(&self) -> bool {
        if self.uses_voice_detector {
            self.voice_speech >= MIN_SPEECH_SAMPLES
        } else {
            self.speech_frames * FRAME_SAMPLES >= MIN_SPEECH_SAMPLES
        }
    }

    /// Silence trailing the last speech (or the whole recording so far when
    /// none was heard), in milliseconds.
    pub fn trailing_silence_ms(&self) -> u64 {
        let (scanned, last) = if self.uses_voice_detector {
            (self.voice_scanned, self.last_voice_end.unwrap_or(0))
        } else {
            (self.scanned, self.last_speech_end.unwrap_or(0))
        };
        (scanned.saturating_sub(last) as u64) * 1000 / SAMPLE_RATE as u64
    }

    /// The voice detector's verdicts on consecutive chunks of `chunk_samples`,
    /// starting where its last verdict ended. From the first call on, these
    /// decide when the recording has gone quiet.
    pub fn heard_voice(&mut self, speech: &[bool], chunk_samples: usize) {
        self.uses_voice_detector = true;
        for &is_speech in speech {
            self.voice_scanned += chunk_samples;
            if is_speech {
                self.voice_speech += chunk_samples;
                self.last_voice_end = Some(self.voice_scanned);
            }
        }
    }

    /// Goes back to loudness for the rest of the recording, when the voice
    /// detector fails partway.
    pub fn abandon_voice_detector(&mut self) {
        self.uses_voice_detector = false;
        self.voice_detector_failed = true;
    }

    /// Accounts for the samples recorded since the last call. `samples_from`
    /// returns the recording from the given absolute sample index onward. A
    /// buffer that shrank (capture restarted) starts the accounting over. A
    /// trailing partial frame is left for the next call.
    pub fn advance(&mut self, total_samples: usize, samples_from: impl Fn(usize) -> Vec<f32>) {
        if total_samples < self.scanned {
            *self = AutoStopPolicy::default();
        }
        if total_samples - self.scanned < FRAME_SAMPLES {
            return;
        }
        let samples = samples_from(self.scanned);
        let mut start = 0;
        while start + FRAME_SAMPLES <= samples.len() {
            let rms = frame_rms(&samples[start..start + FRAME_SAMPLES]);
            self.peak_rms = self.peak_rms.max(rms);
            self.scanned += FRAME_SAMPLES;
            self.heard(rms);
            if rms >= self.speech_threshold {
                self.speech_frames += 1;
                self.last_speech_end = Some(self.scanned);
            }
            start += FRAME_SAMPLES;
        }
    }

    /// Counts the frame's loudness, and sets what counts as speech from the
    /// session so far.
    fn heard(&mut self, rms: f32) {
        let bin = if rms > 0.0001 {
            (((rms.log10() + 4.0) * 5.0) as usize).min(LOUDNESS_BINS - 1)
        } else {
            0
        };
        self.loudness[bin] += 1;
        self.frames += 1;
        if self.frames < 10 {
            return;
        }
        // The quietest 5% of frames: pauses and breaths, or the room.
        let mut seen = 0usize;
        let mut background = 0.0f32;
        for (index, &count) in self.loudness.iter().enumerate() {
            seen += count as usize;
            if seen * 20 >= self.frames {
                background = if index == 0 {
                    0.0
                } else {
                    10f32.powf((index as f32 + 0.5) / 5.0 - 4.0)
                };
                break;
            }
        }
        let adaptive = QUIETEST_SPEECH
            .max(background * 2.5)
            .max(self.peak_rms * 0.1);
        self.speech_threshold = SPEECH_ENERGY_THRESHOLD.min(adaptive);
    }

    /// True when the recording should stop because the trailing silence has
    /// lasted `silence_window`. Never fires before the policy is armed (a user
    /// still gathering their thoughts keeps the mic open; the hands-free cap
    /// remains the hard bound) or before `minimum_duration`.
    pub fn should_stop(
        &self,
        silence_window: Duration,
        recording_duration: Duration,
        minimum_duration: Duration,
    ) -> bool {
        if silence_window.is_zero() || recording_duration < minimum_duration || !self.is_armed() {
            return false;
        }
        self.trailing_silence_ms() >= silence_window.as_millis() as u64
    }
}

/// Hears whether stretches of a recording hold a voice (Mac
/// `VoiceActivityDetecting`).
pub trait VoiceDetector: Send {
    /// Samples behind each verdict.
    fn chunk_samples(&self) -> usize;
    /// Forgets the previous recording.
    fn reset(&mut self);
    /// One verdict per whole chunk in `samples`. `context` is the audio that
    /// immediately precedes `samples` (may be empty) so the detector's
    /// recurrent state can settle before the judged chunks.
    fn speech(&mut self, context: &[f32], samples: &[f32]) -> Result<Vec<bool>>;
}

/// Scripted detector for tests: returns verdicts from a queue, `false` after.
#[derive(Default)]
pub struct MockVoiceDetector {
    pub verdicts: std::collections::VecDeque<bool>,
    pub fail: bool,
}

impl VoiceDetector for MockVoiceDetector {
    fn chunk_samples(&self) -> usize {
        SILERO_CHUNK_SAMPLES
    }
    fn reset(&mut self) {}
    fn speech(&mut self, _context: &[f32], samples: &[f32]) -> Result<Vec<bool>> {
        if self.fail {
            return Err(crate::CoreError::Audio(
                "mock voice detector failure".into(),
            ));
        }
        let n = samples.len() / SILERO_CHUNK_SAMPLES;
        Ok((0..n)
            .map(|_| self.verdicts.pop_front().unwrap_or(false))
            .collect())
    }
}

/// Mac judges 256 ms chunks.
pub const SILERO_CHUNK_SAMPLES: usize = 4096;
/// Speech from this probability up: the threshold the Mac bench measured.
pub const SILERO_SPEECH_PROBABILITY: f32 = 0.5;
/// File name of the whisper.cpp Silero model (HF `ggml-org/whisper-vad`).
pub const SILERO_MODEL_FILE: &str = "ggml-silero-v5.1.2.bin";

/// Collapse whisper.cpp's per-window (32 ms) probabilities into one verdict per
/// 256 ms chunk: speech when at least a quarter of its windows (≥ 64 ms) are
/// above the speech probability. One loud window — a click — never counts.
pub fn chunk_verdicts(window_probs: &[f32], windows_per_chunk: usize) -> Vec<bool> {
    if windows_per_chunk == 0 {
        return Vec::new();
    }
    window_probs
        .chunks(windows_per_chunk)
        .filter(|c| c.len() == windows_per_chunk)
        .map(|c| {
            let voiced = c
                .iter()
                .filter(|&&p| p >= SILERO_SPEECH_PROBABILITY)
                .count();
            voiced * 4 >= windows_per_chunk
        })
        .collect()
}

#[cfg(feature = "whisper")]
pub use silero::SileroVad;

#[cfg(feature = "whisper")]
mod silero {
    use super::{chunk_verdicts, VoiceDetector, SILERO_CHUNK_SAMPLES};
    use crate::error::{CoreError, Result};
    use std::path::Path;
    use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

    /// whisper.cpp's Silero window (32 ms at 16 kHz).
    const WINDOW: usize = 512;
    /// Up to ~1 s of preceding audio lets the LSTM settle (whisper.cpp resets
    /// its state on every call).
    const MAX_CONTEXT: usize = WINDOW * 31;

    /// Silero VAD v5 through whisper.cpp (`WhisperVadContext`).
    pub struct SileroVad {
        ctx: WhisperVadContext,
    }

    impl SileroVad {
        pub fn load(model_path: &Path) -> Result<Self> {
            whisper_rs::install_logging_hooks();
            let path = model_path
                .to_str()
                .ok_or_else(|| CoreError::Audio("non-UTF-8 VAD model path".into()))?;
            if !model_path.exists() {
                return Err(CoreError::Audio(format!(
                    "voice detector model missing: {}",
                    model_path.display()
                )));
            }
            let mut params = WhisperVadContextParams::new();
            params.set_n_threads(1);
            params.set_use_gpu(false);
            let ctx = WhisperVadContext::new(path, params)
                .map_err(|e| CoreError::Audio(format!("load voice detector: {e}")))?;
            Ok(Self { ctx })
        }
    }

    impl VoiceDetector for SileroVad {
        fn chunk_samples(&self) -> usize {
            SILERO_CHUNK_SAMPLES
        }

        fn reset(&mut self) {}

        fn speech(&mut self, context: &[f32], samples: &[f32]) -> Result<Vec<bool>> {
            let whole = samples.len() / SILERO_CHUNK_SAMPLES * SILERO_CHUNK_SAMPLES;
            if whole == 0 {
                return Ok(Vec::new());
            }
            // Whole windows of context only, so judged windows stay aligned.
            let ctx_len = (context.len().min(MAX_CONTEXT) / WINDOW) * WINDOW;
            let mut audio = Vec::with_capacity(ctx_len + whole);
            audio.extend_from_slice(&context[context.len() - ctx_len..]);
            audio.extend_from_slice(&samples[..whole]);
            self.ctx
                .detect_speech(&audio)
                .map_err(|e| CoreError::Audio(format!("voice detector: {e}")))?;
            let probs = self.ctx.probabilities();
            let skip = ctx_len / WINDOW;
            let judged = probs.get(skip..).unwrap_or(&[]);
            Ok(chunk_verdicts(judged, SILERO_CHUNK_SAMPLES / WINDOW))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / 16_000.0).sin())
            .collect()
    }

    fn run(policy: &mut AutoStopPolicy, audio: &[f32]) {
        policy.advance(audio.len(), |from| audio[from..].to_vec());
    }

    #[test]
    fn rms_of_sine_is_amplitude_over_root_two() {
        let r = frame_rms(&tone(1600, 0.5));
        assert!((r - 0.3535).abs() < 0.01, "{r}");
        assert_eq!(frame_rms(&[]), 0.0);
    }

    #[test]
    fn silence_never_arms() {
        let mut p = AutoStopPolicy::new();
        run(&mut p, &vec![0.0; 16_000 * 10]);
        assert!(!p.is_armed());
        assert!(!p.should_stop(
            Duration::from_secs(2),
            Duration::from_secs(10),
            Duration::from_millis(500)
        ));
    }

    #[test]
    fn speech_then_silence_stops_after_window() {
        let mut audio = tone(16_000, 0.3); // 1 s speech
        audio.extend(vec![0.0; 16_000 * 2]); // 2 s silence
        let mut p = AutoStopPolicy::new();
        run(&mut p, &audio);
        assert!(p.is_armed());
        assert_eq!(p.trailing_silence_ms(), 2000);
        let d = Duration::from_secs(3);
        assert!(p.should_stop(Duration::from_secs(2), d, Duration::from_millis(500)));
        assert!(!p.should_stop(Duration::from_secs(4), d, Duration::from_millis(500)));
    }

    #[test]
    fn a_single_click_does_not_arm() {
        let mut audio = vec![0.0; 16_000];
        audio.extend(tone(1600, 0.5)); // one loud 100ms frame
        audio.extend(vec![0.0; 16_000 * 3]);
        let mut p = AutoStopPolicy::new();
        run(&mut p, &audio);
        assert!(!p.is_armed());
    }

    #[test]
    fn quiet_microphone_speech_still_counts() {
        // Soft speech peaking ~0.01 RMS over a near-silent room (0.0003):
        // below the fixed 0.02 line but well above the room.
        let mut audio = Vec::new();
        for _ in 0..20 {
            audio.extend(tone(1600, 0.0004));
        }
        for _ in 0..10 {
            audio.extend(tone(1600, 0.014));
        }
        audio.extend(vec![0.0002; 1600 * 30]);
        let mut p = AutoStopPolicy::new();
        run(&mut p, &audio);
        assert!(p.speech_threshold() < SPEECH_ENERGY_THRESHOLD);
        assert!(
            p.is_armed(),
            "frames={} thr={}",
            p.speech_frame_count(),
            p.speech_threshold()
        );
        assert!(p.trailing_silence_ms() >= 2900);
    }

    #[test]
    fn incremental_advance_matches_one_shot() {
        let mut audio = tone(8_000, 0.2);
        audio.extend(vec![0.0; 24_000]);
        audio.extend(tone(8_000, 0.2));
        let mut one = AutoStopPolicy::new();
        run(&mut one, &audio);
        let mut inc = AutoStopPolicy::new();
        let mut n = 0;
        while n < audio.len() {
            n = (n + 1234).min(audio.len());
            let prefix = &audio[..n];
            inc.advance(prefix.len(), |from| prefix[from..].to_vec());
        }
        assert_eq!(one.speech_frame_count(), inc.speech_frame_count());
        assert_eq!(one.trailing_silence_ms(), inc.trailing_silence_ms());
    }

    #[test]
    fn shrinking_buffer_restarts_accounting() {
        let audio = tone(16_000, 0.3);
        let mut p = AutoStopPolicy::new();
        run(&mut p, &audio);
        assert!(p.scanned_samples() > 0);
        let short = vec![0.0; 3200];
        p.advance(short.len(), |from| short[from..].to_vec());
        assert_eq!(p.scanned_samples(), 3200);
        assert_eq!(p.speech_frame_count(), 0);
    }

    #[test]
    fn minimum_duration_and_zero_window_never_stop() {
        let mut audio = tone(16_000, 0.3);
        audio.extend(vec![0.0; 16_000 * 3]);
        let mut p = AutoStopPolicy::new();
        run(&mut p, &audio);
        assert!(!p.should_stop(Duration::ZERO, Duration::from_secs(4), Duration::ZERO));
        assert!(!p.should_stop(
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(500)
        ));
    }

    #[test]
    fn voice_detector_verdicts_take_over_and_can_be_abandoned() {
        let mut p = AutoStopPolicy::new();
        // Loud audio that loudness would call speech…
        run(&mut p, &tone(16_000 * 2, 0.3));
        // …but the voice detector hears 2 chunks of voice then 6 of silence.
        p.heard_voice(
            &[true, true, false, false, false, false, false, false],
            SILERO_CHUNK_SAMPLES,
        );
        assert!(p.uses_voice_detector());
        assert!(p.is_armed());
        assert_eq!(p.trailing_silence_ms(), 6 * 4096 * 1000 / 16_000);
        p.abandon_voice_detector();
        assert!(!p.uses_voice_detector());
        assert!(p.voice_detector_failed());
        assert_eq!(p.trailing_silence_ms(), 0);
    }

    #[test]
    fn chunk_verdicts_need_a_quarter_of_windows() {
        let probs = [
            0.9, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
        ];
        assert_eq!(chunk_verdicts(&probs, 8), vec![true, false]);
        assert!(chunk_verdicts(&probs[..5], 8).is_empty());
    }

    #[test]
    fn speech_energy_check() {
        assert!(!contains_speech_energy(&vec![0.001; 32_000]));
        let mut a = vec![0.0; 16_000];
        a.extend(tone(1600, 0.2));
        assert!(contains_speech_energy(&a));
    }
}
