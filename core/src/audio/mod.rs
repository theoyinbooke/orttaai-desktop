//! Microphone capture. The real backend (`cpal` + streaming `rubato`
//! resampling to 16 kHz mono f32) lives behind the `cpal-audio` feature.
//!
//! Mirrors the macOS `AudioCaptureService` contract: start on a device chosen
//! by its persistent name (falling back to the system default when it is gone),
//! a live input level, recovery from a stream that errors or stalls mid-recording
//! that **keeps every sample already captured**, and a user-triggered reset.

use crate::error::{CoreError, Result};
use crate::types::{AudioDevice, DeviceId};
use std::sync::{Arc, Mutex};

/// Startup retry pauses (Mac `startupRetryDelays`), in milliseconds.
pub const STARTUP_RETRY_DELAYS_MS: [u64; 3] = [0, 200, 500];
/// Startup pauses after an audio reset / wake, when the stack may be stale
/// (Mac `postWakeStartupRetryDelays`).
pub const STALE_STARTUP_RETRY_DELAYS_MS: [u64; 4] = [0, 250, 600, 1000];
/// Pauses between attempts to bring capture back mid-recording (Mac
/// `recoveryRetryDelays`). A Bluetooth headset can take a couple of seconds.
pub const RECOVERY_RETRY_DELAYS_MS: [u64; 6] = [0, 300, 600, 1000, 1500, 2000];
/// A running capture that delivers no frame for this long is restarted
/// (Mac `stallRestartInterval`).
pub const STALL_RESTART_MS: u64 = 1500;

/// Map an RMS level to the 0..=1 meter scale (Mac: `min(rms * 5, 1)`).
pub fn meter_level(rms: f32) -> f32 {
    (rms * 5.0).clamp(0.0, 1.0)
}

pub trait AudioCapture: Send + Sync {
    /// Begin capturing from `device` (or the system default when `None`). A
    /// named device that is unavailable falls back to the system default.
    fn start(&mut self, device: Option<DeviceId>) -> Result<()>;

    /// Current input level in `0.0..=1.0`, for the waveform meter.
    fn level(&self) -> f32;

    /// Stop capturing and return the buffered 16 kHz mono `f32` samples.
    fn stop(&mut self) -> Result<Vec<f32>>;

    /// A non-destructive snapshot of all buffered 16 kHz samples.
    fn snapshot(&self) -> Vec<f32>;

    /// Enumerate available input devices.
    fn devices(&self) -> Result<Vec<DeviceId>>;

    /// Samples captured so far (16 kHz). Cheap.
    fn captured_samples(&self) -> usize {
        self.snapshot().len()
    }

    /// The recording from absolute 16 kHz index `start` onward. Cheap: lets the
    /// hands-free monitor scan only what arrived since its last poll.
    fn samples_from(&self, start: usize) -> Vec<f32> {
        let all = self.snapshot();
        all.get(start..).map(<[f32]>::to_vec).unwrap_or_default()
    }

    /// Name of the device actually recording (after any fallback).
    fn active_device(&self) -> Option<String> {
        None
    }

    /// Mid-recording restarts in the current recording (diagnostics).
    fn restart_count(&self) -> u32 {
        0
    }

    /// Whether the current capture is mid-recovery (stream lost, retrying).
    fn is_recovering(&self) -> bool {
        false
    }

    /// "Reset Audio": tear down any capture and treat the audio stack as stale
    /// so the next start uses the longer retry schedule.
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Shared handle for feeding audio into a running [`MockAudioCapture`] — lets a
/// test drive hands-free silence detection over (virtual) time.
#[derive(Clone, Default)]
pub struct MockAudioFeed(Arc<Mutex<Vec<f32>>>);

impl MockAudioFeed {
    pub fn push(&self, samples: &[f32]) {
        self.0.lock().unwrap().extend_from_slice(samples);
    }
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Mock capture for tests and the `demo` CLI on any OS. On `stop()` it returns
/// whatever was fed during the recording, or `seconds` of canned silence when
/// nothing was fed.
pub struct MockAudioCapture {
    feed: MockAudioFeed,
    canned: Vec<f32>,
    capturing: Mutex<bool>,
    fail_start: bool,
    started_with: Arc<Mutex<Vec<Option<DeviceId>>>>,
}

impl MockAudioCapture {
    /// `seconds` of canned (silent) audio is returned on `stop()`.
    pub fn new(seconds: f32) -> Self {
        let n = (seconds * crate::types::TARGET_SAMPLE_RATE as f32) as usize;
        Self::with_samples(vec![0.0; n])
    }

    /// Return exactly `samples` on `stop()` (unless audio is fed).
    pub fn with_samples(samples: Vec<f32>) -> Self {
        Self {
            feed: MockAudioFeed::default(),
            canned: samples,
            capturing: Mutex::new(false),
            fail_start: false,
            started_with: Arc::default(),
        }
    }

    /// A capture whose `start()` always fails (no microphone).
    pub fn failing() -> Self {
        Self {
            fail_start: true,
            ..Self::new(0.0)
        }
    }

    /// Handle to push live samples during a recording.
    pub fn feed(&self) -> MockAudioFeed {
        self.feed.clone()
    }

    /// Devices each `start()` was asked for.
    pub fn starts(&self) -> Arc<Mutex<Vec<Option<DeviceId>>>> {
        self.started_with.clone()
    }
}

impl Default for MockAudioCapture {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl AudioCapture for MockAudioCapture {
    fn start(&mut self, device: Option<DeviceId>) -> Result<()> {
        self.started_with.lock().unwrap().push(device);
        if self.fail_start {
            return Err(CoreError::Audio("no microphone".to_string()));
        }
        *self.capturing.lock().unwrap() = true;
        self.feed.0.lock().unwrap().clear();
        Ok(())
    }

    fn level(&self) -> f32 {
        if *self.capturing.lock().unwrap() {
            0.2
        } else {
            0.0
        }
    }

    fn stop(&mut self) -> Result<Vec<f32>> {
        *self.capturing.lock().unwrap() = false;
        let fed = std::mem::take(&mut *self.feed.0.lock().unwrap());
        Ok(if fed.is_empty() {
            self.canned.clone()
        } else {
            fed
        })
    }

    fn snapshot(&self) -> Vec<f32> {
        self.feed.0.lock().unwrap().clone()
    }

    fn captured_samples(&self) -> usize {
        self.feed.len()
    }

    fn devices(&self) -> Result<Vec<DeviceId>> {
        Ok(vec![DeviceId("mock-default".to_string())])
    }

    fn active_device(&self) -> Option<String> {
        Some("mock-default".to_string())
    }
}

/// Input devices with the system default flagged. Mock list without the
/// `cpal-audio` feature.
pub fn list_input_devices() -> Result<Vec<AudioDevice>> {
    #[cfg(feature = "cpal-audio")]
    {
        cpal_backend::list_input_devices()
    }
    #[cfg(not(feature = "cpal-audio"))]
    {
        Ok(vec![AudioDevice {
            name: "mock-default".to_string(),
            is_default: true,
        }])
    }
}

#[cfg(feature = "cpal-audio")]
mod cpal_backend;
#[cfg(feature = "cpal-audio")]
pub use cpal_backend::{resample_to_target, CpalAudioCapture, LevelMonitor};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_fed_audio_or_canned() {
        let mut m = MockAudioCapture::new(0.5);
        m.start(None).unwrap();
        assert_eq!(m.stop().unwrap().len(), 8000);
        let feed = m.feed();
        m.start(Some(DeviceId("USB".into()))).unwrap();
        feed.push(&[0.1; 100]);
        assert_eq!(m.captured_samples(), 100);
        assert_eq!(m.samples_from(40).len(), 60);
        assert_eq!(m.stop().unwrap().len(), 100);
        assert_eq!(m.starts().lock().unwrap().len(), 2);
    }

    #[test]
    fn meter_scale_matches_mac() {
        assert_eq!(meter_level(0.1), 0.5);
        assert_eq!(meter_level(0.5), 1.0);
        assert_eq!(meter_level(-1.0), 0.0);
    }
}
