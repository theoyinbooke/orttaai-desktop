//! Decoding while the speaker is still talking (Mac `TranscriptionService` live
//! session): the recording is committed in pieces as it goes, so releasing the
//! key leaves only a short tail to decode however long the dictation ran.
//!
//! - Whisper commits a clip at a speech pause (0.7 s of quiet after at least
//!   2 s of audio), or at 15 s, cut at the latest 200 ms gap in its final 3 s.
//!   Each clip is decoded with the dictionary bias and the text so far as its
//!   prompt.
//! - Express (the Mac's "Decode while I speak", off by default) commits
//!   segments of at least 8 s at a 0.7 s pause, or past 12 s at the latest
//!   200 ms gap between words (the Mac waits 20 s; 12 s leaves a slow PC less
//!   to decode at release, and matched the whole decode just as well). It
//!   never cuts a word: continuous speech waits. Each segment is decoded with the 3 s before it and the 2 s after it, and
//!   the decodes are joined where they agree ([`crate::stitch`], the Mac's
//!   round 4). The pause is judged on audio at least 2 s old. A segment that
//!   decodes to nothing or doesn't agree with the one before ends segments
//!   for that recording: it's then decoded whole at release.
//!
//! Only one piece decodes at a time; a Whisper clip that fails stays
//! uncommitted and is simply decoded with the tail at the end.

use crate::transcription::{RawDecode, Transcriber};
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
/// Audio before an Express segment, and after it, decoded with it (Mac
/// `expressSegmentContextSampleCount`, `expressSegmentLookaheadSampleCount`).
pub const SEGMENT_CONTEXT: usize = 3 * SAMPLE_RATE;
pub const SEGMENT_LOOKAHEAD: usize = 2 * SAMPLE_RATE;
const SEGMENT_GAP_SEARCH: usize = 12 * SAMPLE_RATE;

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

/// Where a segment's decode window starts: the full 3 s of context before
/// `committed` (the recording's start for the first segment).
pub fn window_start(from: usize, committed: usize) -> usize {
    committed.saturating_sub(SEGMENT_CONTEXT).max(from)
}

/// The start to retry a window from when it decoded to nothing or didn't
/// join: the quietest 100 ms between 3 s and 1 s before `committed`, so
/// the window doesn't open in the middle of a word. Fluid Speech Mini
/// sometimes decodes a window that does to nothing, and the same audio from
/// a pause before it correctly. `None` when that is the same start. `audio`
/// is the recording from sample `from` on.
pub fn retry_start(from: usize, audio: &[f32], committed: usize) -> Option<usize> {
    let start = quiet_start(from, audio, committed);
    (start != window_start(from, committed)).then_some(start)
}

fn quiet_start(from: usize, audio: &[f32], committed: usize) -> usize {
    let earliest = committed.saturating_sub(SEGMENT_CONTEXT).max(from);
    let latest = committed.saturating_sub(SEGMENT_CONTEXT / 3).max(earliest);
    if earliest == 0 && committed <= SEGMENT_CONTEXT {
        return 0;
    }
    let mut best = (f32::MAX, earliest);
    let mut at = earliest;
    while at + FRAME_SAMPLES <= latest && at + FRAME_SAMPLES <= from + audio.len() {
        let rms = frame_rms(&audio[at - from..at - from + FRAME_SAMPLES]);
        if rms < best.0 {
            best = (rms, at);
        }
        at += FRAME_SAMPLES / 2;
    }
    best.1
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

/// Live pieces are decoded without a prompt, like the Mac
/// (`currentLivePromptTokens` returns nil): its ASR corpus measured prompted
/// live clips at 6.60% WER against 4.36% without, with median latency doubled
/// and dictionary words hallucinated into unrelated speech. The whole-recording
/// path keeps the dictionary bias.
fn live_options(options: &DecodeOptions) -> DecodeOptions {
    let mut opts = options.clone();
    opts.initial_prompt = None;
    opts
}

struct Inflight {
    /// The committed samples it continues from, and where its piece ends.
    start: usize,
    end: usize,
    /// The audio it decodes (for a segment, with its context), from
    /// `window_start` in the recording.
    clip: Vec<f32>,
    window_start: usize,
    handle: PieceHandle,
    /// A segment's second window (see [`retry_start`]): its start and audio.
    retry: Option<(usize, Vec<f32>)>,
}

enum PieceHandle {
    Text(JoinHandle<crate::error::Result<String>>),
    Raw(JoinHandle<crate::error::Result<RawDecode>>),
}

impl PieceHandle {
    fn is_finished(&self) -> bool {
        match self {
            Self::Text(h) => h.is_finished(),
            Self::Raw(h) => h.is_finished(),
        }
    }
}

/// What Express's live segments left at release.
pub struct Segments {
    /// The segments joined so far, timed from the recording's start.
    pub joined: RawDecode,
    /// Samples they cover.
    pub covered: usize,
    /// A segment didn't decode or didn't join: decode the whole recording.
    pub failed: bool,
}

/// What a decode started at a pause produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Early {
    /// The transcript of the audio it decoded (no live segments, or Whisper).
    Text(String),
    /// Express with live segments: the raw decode of the tail window, timed
    /// from the recording's start, to join at release.
    Raw(RawDecode),
}

/// Quiet after speech that starts a decode before the key comes up (Mac
/// `expressEarlyDecodeSilenceSampleCount`: in October 2026 its dictations
/// ended with a median 1.3 s of silence before release, at least 0.5 s in 234
/// of 239), so the decode is done, or well under way, at release.
pub const EARLY_DECODE_SILENCE: usize = SAMPLE_RATE / 2;
/// The latest audio searched for that pause.
pub const EARLY_DECODE_PROBE: usize = 3 * SAMPLE_RATE;

/// Where the speaker's last speech ended in `recent` (the latest audio), when
/// they have been quiet for at least [`EARLY_DECODE_SILENCE`] since; `None`
/// while they're talking or when there's no speech in it.
pub fn paused_after_speech(recent: &[f32]) -> Option<Pause> {
    if !has_speech(recent, SPEECH_ENERGY_THRESHOLD) {
        return None;
    }
    let quiet = pause_threshold(recent);
    let end = last_speech_end(recent, quiet)?;
    (recent.len() - end >= EARLY_DECODE_SILENCE).then_some(Pause { end, quiet })
}

/// A pause after speech: where the speech ended and the level below which
/// the audio counted as quiet (the room's noise, see [`pause_threshold`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pause {
    pub end: usize,
    pub quiet: f32,
}

/// A decode of the rest of the recording, started at a pause while the key is
/// still down (Mac: Express early decode, Whisper speculative tail). At
/// release it stands in for decoding that audio, when nothing was committed
/// since it started and only silence was recorded after what it covers.
pub struct EarlyDecode {
    /// Samples already committed by the live session when it started.
    base: usize,
    /// Samples it covers (base + the audio it decodes).
    covered: usize,
    /// Where the speech it saw ended, so a later pause after new speech
    /// starts a fresh one.
    speech_end: usize,
    /// The level the pause was judged quiet by. Audio after it at this level
    /// is the same quiet, so the decode still stands for the recording.
    quiet: f32,
    stop: Arc<crate::cancel::Token>,
    handle: Option<JoinHandle<crate::error::Result<Early>>>,
}

impl EarlyDecode {
    /// Decode `audio` (the recording from `base` on) in the background.
    pub fn start(
        base: usize,
        audio: Vec<f32>,
        pause: Pause,
        transcriber: &Arc<dyn Transcriber>,
        options: DecodeOptions,
    ) -> Option<Self> {
        let covered = base + audio.len();
        let transcriber = Arc::clone(transcriber);
        Self::run(base, covered, pause, move || {
            transcriber.transcribe(&audio, &options).map(Early::Text)
        })
    }

    /// Decode the tail of a recording with Express live segments: `window`
    /// starts at sample `window_start` (the segments' context before
    /// `base`) and runs to the end of what was recorded.
    pub fn start_raw(
        base: usize,
        window_start: usize,
        window: Vec<f32>,
        pause: Pause,
        transcriber: &Arc<dyn Transcriber>,
    ) -> Option<Self> {
        let covered = window_start + window.len();
        let transcriber = Arc::clone(transcriber);
        let offset = window_start as f32 / SAMPLE_RATE as f32;
        Self::run(base, covered, pause, move || {
            transcriber
                .decode_raw(&window)
                .map(|raw| Early::Raw(crate::stitch::shifted(raw, offset)))
        })
    }

    fn run(
        base: usize,
        covered: usize,
        pause: Pause,
        job: impl FnOnce() -> crate::error::Result<Early> + Send + 'static,
    ) -> Option<Self> {
        let stop = crate::cancel::Token::new();
        let token = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("orttaai-early-decode".into())
            .spawn(move || crate::cancel::scope(Some(token), job))
            .ok()?;
        Some(Self {
            base,
            covered,
            speech_end: pause.end,
            quiet: pause.quiet.max(FAINT_ENERGY_FLOOR),
            stop,
            handle: Some(handle),
        })
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Whether it already covers speech up to `speech_end` from `base`.
    pub fn covers(&self, base: usize, speech_end: usize) -> bool {
        self.base == base && self.speech_end >= speech_end
    }

    /// Samples from the recording's start that it decodes.
    pub fn covered(&self) -> usize {
        self.covered
    }

    /// Whether it no longer stands for the recording: something was
    /// committed since it started (`base` moved), or the speaker talked
    /// again in `after`, the audio recorded after what it covers.
    pub fn is_stale(&self, base: usize, after: &[f32]) -> bool {
        self.base != base || has_speech(after, self.quiet)
    }

    /// Its result when it still stands for the rest of `samples` (the whole
    /// recording) after `base` committed samples, waiting for it if it's
    /// still running; `None` when the recording moved on since (it is then
    /// stopped, so the decode at release doesn't wait behind it).
    pub fn take_if_current(mut self, base: usize, samples: &[f32]) -> Option<Early> {
        if self.base != base || self.covered > samples.len() {
            return None;
        }
        if has_speech(&samples[self.covered..], self.quiet) {
            return None;
        }
        self.handle.take()?.join().ok()?.ok()
    }
}

impl Drop for EarlyDecode {
    /// A decode nobody will use is stopped: it would keep the speech model
    /// busy for the next one.
    fn drop(&mut self) {
        if self.handle.as_ref().is_some_and(|h| !h.is_finished()) {
            self.stop.cancel();
        }
    }
}

/// One recording's committed pieces.
pub struct LiveSession {
    mode: LiveMode,
    /// Whisper's committed clips.
    committed: Vec<String>,
    /// Express's segments, joined.
    joined: RawDecode,
    /// A segment failed to decode or join: no more for this recording.
    failed: bool,
    committed_samples: usize,
    inflight: Option<Inflight>,
    /// What decodes Express segments (kept for a retry).
    transcriber: Option<Arc<dyn Transcriber>>,
}

impl LiveSession {
    pub fn new(mode: LiveMode) -> Self {
        Self {
            mode,
            committed: Vec::new(),
            joined: RawDecode::default(),
            failed: false,
            committed_samples: 0,
            inflight: None,
            transcriber: None,
        }
    }

    pub fn mode(&self) -> LiveMode {
        self.mode
    }

    /// Committed text so far (Whisper clips), and how many samples the
    /// committed pieces cover.
    pub fn committed(&self) -> (&[String], usize) {
        (&self.committed, self.committed_samples)
    }

    /// Where the audio the next piece needs starts: the committed samples,
    /// or for a segment the context before them.
    pub fn context_start(&self) -> usize {
        match self.mode {
            LiveMode::Segments => self.committed_samples.saturating_sub(SEGMENT_CONTEXT),
            _ => self.committed_samples,
        }
    }

    /// Smallest uncommitted length worth looking at.
    pub fn min_pending(&self) -> usize {
        match self.mode {
            LiveMode::Clips => PAUSE_MIN_CLIP + PAUSE_SILENCE,
            LiveMode::Segments if self.failed => usize::MAX,
            LiveMode::Segments => SEGMENT_MIN + PAUSE_SILENCE + SEGMENT_LOOKAHEAD,
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
        match piece.handle {
            PieceHandle::Text(handle) => match handle.join() {
                Ok(Ok(decoded)) => {
                    if piece.start == self.committed_samples {
                        if let Some(text) = commit_text(&decoded, &piece.clip) {
                            if !text.is_empty() {
                                self.committed.push(text);
                            }
                            self.committed_samples = piece.end;
                        }
                    }
                }
                Ok(Err(e)) => tracing::debug!("live piece not committed: {e}"),
                Err(_) => tracing::warn!("live decode thread panicked"),
            },
            PieceHandle::Raw(handle) => {
                let decoded = match handle.join() {
                    Ok(Ok(raw)) => Some(raw),
                    Ok(Err(e)) => {
                        tracing::debug!("live segment didn't decode: {e}");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("live decode thread panicked");
                        None
                    }
                };
                if piece.start != self.committed_samples {
                    return;
                }
                let offset = piece.window_start as f32 / SAMPLE_RATE as f32;
                // A segment is only cut after speech, so one that decodes to
                // nothing, or doesn't agree with the one before where they
                // overlap, means the segments can't be trusted here (after a
                // second try from a pause before the cut).
                let joined = decoded
                    .filter(|raw| !raw.text.trim().is_empty() && crate::stitch::stitchable(raw))
                    .and_then(|raw| {
                        crate::stitch::stitch(&self.joined, &crate::stitch::shifted(raw, offset))
                    });
                if joined.is_none() {
                    if let Some((start, window)) = piece.retry {
                        self.spawn_segment(piece.start, piece.end, start, window, None);
                        if block {
                            self.collect(true);
                        }
                        return;
                    }
                }
                match joined {
                    Some(joined) => {
                        self.joined = joined;
                        self.committed_samples = piece.end;
                    }
                    None => {
                        tracing::info!("live segments stopped: a segment didn't join");
                        self.failed = true;
                    }
                }
            }
        }
    }

    /// Whether a piece is decoding.
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// Start decoding the next piece, when one is due. `audio` is the
    /// recording from sample `from` on, where `from` is at most
    /// [`LiveSession::context_start`].
    pub fn start_next(
        &mut self,
        from: usize,
        audio: &[f32],
        transcriber: &Arc<dyn Transcriber>,
        options: &DecodeOptions,
    ) -> bool {
        if self.inflight.is_some() || from > self.context_start() {
            return false;
        }
        let committed = self.committed_samples;
        let Some(pending) = audio.get(committed - from..) else {
            return false;
        };
        let transcriber = Arc::clone(transcriber);
        let (piece, end, window_start, handle) = if self.mode == LiveMode::Segments {
            if self.failed || pending.len() <= SEGMENT_LOOKAHEAD {
                return false;
            }
            // The cut is decided on audio at least 2 s old, which is then
            // decoded with the segment as its lookahead.
            let decided = &pending[..pending.len() - SEGMENT_LOOKAHEAD];
            let Some(len) = segment_len(decided) else {
                return false;
            };
            let start = window_start(from, committed);
            let end = (committed + len + SEGMENT_LOOKAHEAD).min(from + audio.len());
            let window = audio[start - from..end - from].to_vec();
            let retry = retry_start(from, audio, committed)
                .map(|r| (r, audio[r - from..end - from].to_vec()));
            self.transcriber = Some(Arc::clone(&transcriber));
            return self.spawn_segment(committed, committed + len, start, window, retry);
        } else {
            let Some(len) = next_commit(pending, self.mode) else {
                return false;
            };
            let clip = pending[..len].to_vec();
            let opts = live_options(options);
            let audio = clip.clone();
            let handle = std::thread::Builder::new()
                .name("orttaai-live-decode".into())
                .spawn(move || transcriber.transcribe(&audio, &opts))
                .map(PieceHandle::Text);
            (clip, committed + len, committed, handle)
        };
        match handle {
            Ok(handle) => {
                self.inflight = Some(Inflight {
                    start: committed,
                    end,
                    clip: piece,
                    window_start,
                    handle,
                    retry: None,
                });
                true
            }
            Err(e) => {
                tracing::warn!("couldn't start a live decode: {e}");
                false
            }
        }
    }

    /// Decode a segment's `window` (from sample `window_start`) for the
    /// piece `start..end`.
    fn spawn_segment(
        &mut self,
        start: usize,
        end: usize,
        window_start: usize,
        window: Vec<f32>,
        retry: Option<(usize, Vec<f32>)>,
    ) -> bool {
        let Some(transcriber) = self.transcriber.clone() else {
            return false;
        };
        let clip = window.clone();
        let handle = std::thread::Builder::new()
            .name("orttaai-live-decode".into())
            .spawn(move || transcriber.decode_raw(&clip));
        match handle {
            Ok(handle) => {
                self.inflight = Some(Inflight {
                    start,
                    end,
                    clip: window,
                    window_start,
                    handle: PieceHandle::Raw(handle),
                    retry,
                });
                true
            }
            Err(e) => {
                tracing::warn!("couldn't start a live decode: {e}");
                false
            }
        }
    }

    /// Wait for the piece in flight and hand back what was committed
    /// (Whisper clips).
    pub fn finish(mut self) -> (Vec<String>, usize) {
        self.collect(true);
        (self.committed, self.committed_samples)
    }

    /// Wait for the segment in flight and hand back the joined segments
    /// (Express).
    pub fn finish_segments(mut self) -> Segments {
        self.collect(true);
        Segments {
            joined: self.joined,
            covered: self.committed_samples,
            failed: self.failed,
        }
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

    /// Decodes slowly, a step at a time, stopping when cancelled, like
    /// Express's model calls.
    struct SlowTranscriber {
        stopped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Transcriber for SlowTranscriber {
        fn load_model(&mut self, _: &str) -> crate::error::Result<()> {
            Ok(())
        }
        fn loaded_model(&self) -> Option<&str> {
            Some("slow")
        }
        fn transcribe(&self, _: &[f32], _: &DecodeOptions) -> crate::error::Result<String> {
            for _ in 0..100 {
                if crate::cancel::cancelled() {
                    self.stopped
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    return Err(crate::cancel::error());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok("slow".into())
        }
    }

    /// Low noise (a fan, a room) at `rms`.
    fn noise(secs: f32, rms: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f32) as usize;
        let amp = rms * std::f32::consts::SQRT_2;
        (0..n).map(|i| amp * (i as f32 * 1.3).sin()).collect()
    }

    #[test]
    fn a_pause_in_room_noise_is_still_a_pause_at_release() {
        // Speech, then the room's noise: the pause is judged by that noise,
        // so more of the same noise after the decode doesn't make it stale.
        let mut recording = noise(1.0, 0.007);
        recording.extend(tone(1.5, 0.2));
        recording.extend(noise(0.7, 0.007));
        let pause = paused_after_speech(&recording).expect("a pause after speech");
        assert!(
            pause.quiet > 0.007,
            "quiet level {} is above the noise",
            pause.quiet
        );
        let t: Arc<dyn Transcriber> = Arc::new(MockTranscriber::new("hello"));
        let early =
            EarlyDecode::start(0, recording.clone(), pause, &t, DecodeOptions::default()).unwrap();
        recording.extend(noise(0.6, 0.007));
        assert_eq!(
            early.take_if_current(0, &recording),
            Some(Early::Text("hello".into()))
        );
    }

    #[test]
    fn speech_after_the_pause_makes_it_stale() {
        let mut recording = tone(1.5, 0.2);
        recording.extend(quiet(0.7));
        let pause = paused_after_speech(&recording).unwrap();
        let t: Arc<dyn Transcriber> = Arc::new(MockTranscriber::new("hello"));
        let early =
            EarlyDecode::start(0, recording.clone(), pause, &t, DecodeOptions::default()).unwrap();
        let covered = early.covered();
        recording.extend(tone(0.5, 0.2));
        assert!(early.is_stale(0, &recording[covered..]));
        assert!(
            early.is_stale(16_000, &[]),
            "a committed piece moved the base"
        );
        assert_eq!(early.take_if_current(0, &recording), None);
    }

    #[test]
    fn a_stale_decode_is_stopped_not_waited_for() {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let t: Arc<dyn Transcriber> = Arc::new(SlowTranscriber {
            stopped: stopped.clone(),
        });
        let mut recording = tone(1.5, 0.2);
        recording.extend(quiet(0.7));
        let pause = paused_after_speech(&recording).unwrap();
        let early =
            EarlyDecode::start(0, recording.clone(), pause, &t, DecodeOptions::default()).unwrap();
        recording.extend(tone(0.5, 0.2));
        let started = std::time::Instant::now();
        assert_eq!(early.take_if_current(0, &recording), None);
        while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "the stale decode kept running"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_current_decode_is_waited_for_not_stopped() {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let t: Arc<dyn Transcriber> = Arc::new(SlowTranscriber {
            stopped: stopped.clone(),
        });
        let mut recording = tone(1.5, 0.2);
        recording.extend(quiet(0.7));
        let pause = paused_after_speech(&recording).unwrap();
        let early =
            EarlyDecode::start(0, recording.clone(), pause, &t, DecodeOptions::default()).unwrap();
        recording.extend(quiet(0.3));
        assert_eq!(
            early.take_if_current(0, &recording),
            Some(Early::Text("slow".into()))
        );
        assert!(!stopped.load(std::sync::atomic::Ordering::SeqCst));
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
    fn live_pieces_are_never_prompted() {
        let o = DecodeOptions {
            initial_prompt: Some("Orttaai.".into()),
            ..DecodeOptions::default()
        };
        assert_eq!(live_options(&o).initial_prompt, None);
    }

    #[test]
    fn session_commits_pieces_in_order() {
        let t: Arc<dyn Transcriber> = Arc::new(MockTranscriber::new("piece"));
        let mut s = LiveSession::new(LiveMode::Clips);
        let mut audio = tone(2.5, 0.2);
        audio.extend(quiet(0.8));
        assert!(s.start_next(0, &audio, &t, &DecodeOptions::default()));
        assert!(
            !s.start_next(0, &audio, &t, &DecodeOptions::default()),
            "one at a time"
        );
        let (texts, covered) = s.finish();
        assert_eq!(texts, vec!["piece".to_string()]);
        assert!(covered > 2 * SAMPLE_RATE && covered <= audio.len());
    }

    /// Speech made of "words": 0.3 s of a steady level each, word `k` at
    /// level 0.1 + 0.002 k, with 0.1 s between words and a 0.8 s pause
    /// after every `per_pause` words.
    fn spoken(words: usize, per_pause: usize) -> Vec<f32> {
        let mut audio = Vec::new();
        for k in 0..words {
            audio.extend(std::iter::repeat_n(
                0.1 + 0.002 * k as f32,
                SAMPLE_RATE * 3 / 10,
            ));
            audio.extend(quiet(if (k + 1) % per_pause == 0 { 0.8 } else { 0.1 }));
        }
        audio
    }

    /// Reads [`spoken`] words back, as a recognizer would: a token per
    /// stretch of sound, named by its level, timed from the window's start.
    /// `garble_from` names every word in windows starting after that sample
    /// count wrongly, for a decode that disagrees.
    struct WordReader {
        garble_after: Option<usize>,
        seen: std::sync::Mutex<usize>,
    }

    impl Transcriber for WordReader {
        fn load_model(&mut self, _: &str) -> crate::error::Result<()> {
            Ok(())
        }
        fn loaded_model(&self) -> Option<&str> {
            Some("reader")
        }
        fn transcribe(&self, samples: &[f32], _: &DecodeOptions) -> crate::error::Result<String> {
            Ok(self.decode_raw(samples)?.text)
        }
        fn decode_raw(&self, samples: &[f32]) -> crate::error::Result<RawDecode> {
            let calls = {
                let mut n = self.seen.lock().unwrap();
                *n += 1;
                *n
            };
            let garble = self.garble_after.is_some_and(|after| calls > after);
            let mut raw = RawDecode::default();
            let mut i = 0;
            while i < samples.len() {
                if samples[i] == 0.0 {
                    i += 1;
                    continue;
                }
                let level = samples[i];
                let start = i;
                while i < samples.len() && samples[i] == level {
                    i += 1;
                }
                let k = ((level - 0.1) / 0.002).round() as i32;
                let name = if garble {
                    format!("x{k}")
                } else {
                    format!("w{k}")
                };
                raw.tokens.push(format!("\u{2581}{name}"));
                raw.starts.push(start as f32 / SAMPLE_RATE as f32);
                raw.durations.push((i - start) as f32 / SAMPLE_RATE as f32);
            }
            raw.text = raw
                .tokens
                .iter()
                .map(|t| t.trim_start_matches('\u{2581}'))
                .collect::<Vec<_>>()
                .join(" ");
            Ok(raw)
        }
    }

    /// Record `audio` 250 ms at a time, as the coordinator polls a live
    /// session, and return what the segments committed.
    fn record(s: &mut LiveSession, audio: &[f32], t: &Arc<dyn Transcriber>) -> usize {
        let mut started = 0;
        let mut captured = 0;
        while captured < audio.len() {
            captured = (captured + SAMPLE_RATE / 4).min(audio.len());
            s.collect(true);
            let (_, covered) = s.committed();
            if captured < covered.saturating_add(s.min_pending()) {
                continue;
            }
            let from = s.context_start();
            if s.start_next(from, &audio[from..captured], t, &DecodeOptions::default()) {
                started += 1;
            }
        }
        started
    }

    fn words_named(n: usize) -> String {
        (0..n)
            .map(|k| format!("w{k}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn segments_join_into_the_whole_recording() {
        let audio = spoken(80, 25);
        let t: Arc<dyn Transcriber> = Arc::new(WordReader {
            garble_after: None,
            seen: Default::default(),
        });
        let mut s = LiveSession::new(LiveMode::Segments);
        let started = record(&mut s, &audio, &t);
        assert!(started >= 2, "{started} segments");
        let seg = s.finish_segments();
        assert!(!seg.failed);
        assert!(seg.covered > 0 && seg.covered < audio.len());
        // The tail, decoded with the segments' context, as at release.
        let window_start = window_start(0, seg.covered);
        let tail = crate::stitch::shifted(
            t.decode_raw(&audio[window_start..]).unwrap(),
            window_start as f32 / SAMPLE_RATE as f32,
        );
        let joined = crate::stitch::stitch(&seg.joined, &tail).unwrap();
        assert_eq!(joined.text, words_named(80));
        // Every token keeps its time in the recording: word k starts at
        // 0.4 k s plus 0.7 s for each pause before it.
        for (k, start) in joined.starts.iter().enumerate() {
            let want = 0.4 * k as f32 + 0.7 * (k / 25) as f32;
            assert!(
                (start - want).abs() < 0.02,
                "word {k} at {start}, want {want}"
            );
        }
    }

    #[test]
    fn a_segment_that_disagrees_ends_segments() {
        let audio = spoken(80, 25);
        let t: Arc<dyn Transcriber> = Arc::new(WordReader {
            garble_after: Some(1),
            seen: Default::default(),
        });
        let mut s = LiveSession::new(LiveMode::Segments);
        let started = record(&mut s, &audio, &t);
        assert_eq!(started, 2, "no segment after the one that didn't join");
        assert_eq!(s.min_pending(), usize::MAX);
        assert!(s.finish_segments().failed);
    }

    #[test]
    fn short_recordings_commit_no_segment() {
        let audio = spoken(20, 25);
        let t: Arc<dyn Transcriber> = Arc::new(WordReader {
            garble_after: None,
            seen: Default::default(),
        });
        let mut s = LiveSession::new(LiveMode::Segments);
        assert_eq!(record(&mut s, &audio, &t), 0);
        let seg = s.finish_segments();
        assert!(!seg.failed);
        assert_eq!(seg.covered, 0);
        assert!(seg.joined.text.is_empty());
    }

    /// Word edits between two texts (case and punctuation ignored).
    #[cfg(feature = "express")]
    fn word_edits(a: &str, b: &str) -> usize {
        let key = |t: &str| -> Vec<String> {
            t.split_whitespace()
                .map(|w| {
                    w.to_lowercase()
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .collect()
                })
                .filter(|w: &String| !w.is_empty())
                .collect()
        };
        let (a, b) = (key(a), key(b));
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, wa) in a.iter().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for (j, wb) in b.iter().enumerate() {
                let cur = row[j + 1];
                row[j + 1] = if wa == wb {
                    prev
                } else {
                    1 + prev.min(cur).min(row[j])
                };
                prev = cur;
            }
        }
        row[b.len()]
    }

    /// Live segments with the real Fluid Speech Mini against one decode of
    /// the whole recording. Prints counts and times only, never the words.
    /// `ORTTAAI_FLUID_DIR=<fluid-speech-mini-onnx> ORTTAAI_SEGMENT_WAVS=a.wav,b.wav
    /// cargo test --release -p orttaai-core --features express -- --ignored
    /// segments_match_the_whole_decode --nocapture`
    #[cfg(feature = "express")]
    #[test]
    #[ignore]
    fn segments_match_the_whole_decode() {
        let dir = std::env::var("ORTTAAI_FLUID_DIR").expect("ORTTAAI_FLUID_DIR");
        let wavs = std::env::var("ORTTAAI_SEGMENT_WAVS").expect("ORTTAAI_SEGMENT_WAVS");
        let t: Arc<dyn Transcriber> = Arc::new(
            crate::transcription::ExpressTranscriber::from_dir(std::path::Path::new(&dir), 4)
                .unwrap(),
        );
        let (mut words, mut edits) = (0, 0);
        for wav in wavs.split(',') {
            let mut reader = hound::WavReader::open(wav).unwrap();
            let audio: Vec<f32> = reader
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect();
            let started = std::time::Instant::now();
            let whole = crate::stitch::normalized(&t.decode_raw(&audio).unwrap());
            let whole_time = started.elapsed();
            let mut s = LiveSession::new(LiveMode::Segments);
            let pieces = record(&mut s, &audio, &t);
            let seg = s.finish_segments();
            assert!(!seg.failed, "{wav}: a segment didn't join");
            let started = std::time::Instant::now();
            let window_start = window_start(0, seg.covered);
            let tail = crate::stitch::shifted(
                t.decode_raw(&audio[window_start..]).unwrap(),
                window_start as f32 / SAMPLE_RATE as f32,
            );
            let joined = crate::stitch::stitch(&seg.joined, &tail).expect("the tail joins");
            let tail_time = started.elapsed();
            let n = whole.text.split_whitespace().count();
            let e = word_edits(&whole.text, &joined.text);
            eprintln!(
                "{}: {:.1}s, {pieces} segments, {n} words, {e} differ; whole decode {whole_time:?}, tail at release {tail_time:?} ({:.1}s of audio)",
                std::path::Path::new(wav).file_name().unwrap().to_string_lossy(),
                audio.len() as f32 / SAMPLE_RATE as f32,
                (audio.len() - window_start) as f32 / SAMPLE_RATE as f32,
            );
            words += n;
            edits += e;
        }
        eprintln!("all: {edits} of {words} words differ");
        assert!(edits * 100 <= words * 2, "{edits} of {words}");
    }
}
