//! Microphone capture. The real backend (`cpal` + `rubato` resampling to 16 kHz
//! mono f32) lands in Phase 1 behind the `cpal-audio` feature.

use crate::error::Result;
use crate::types::DeviceId;
use std::sync::Mutex;

pub trait AudioCapture: Send + Sync {
    /// Begin capturing from `device` (or the system default when `None`).
    fn start(&mut self, device: Option<DeviceId>) -> Result<()>;

    /// Current input level in `0.0..=1.0`, for the waveform meter.
    fn level(&self) -> f32;

    /// Stop capturing and return the buffered 16 kHz mono `f32` samples.
    fn stop(&mut self) -> Result<Vec<f32>>;

    /// A non-destructive snapshot of buffered samples (for live transcription).
    fn snapshot(&self) -> Vec<f32>;

    /// Enumerate available input devices.
    fn devices(&self) -> Result<Vec<DeviceId>>;
}

/// Mock capture that yields a fixed buffer of silence-length samples. Lets the
/// coordinator and CLI run end-to-end on any OS without a real microphone.
pub struct MockAudioCapture {
    samples: Mutex<Vec<f32>>,
    canned: Vec<f32>,
    capturing: Mutex<bool>,
}

impl MockAudioCapture {
    /// `seconds` of canned (silent) audio is returned on `stop()`.
    pub fn new(seconds: f32) -> Self {
        let n = (seconds * crate::types::TARGET_SAMPLE_RATE as f32) as usize;
        Self {
            samples: Mutex::new(Vec::new()),
            canned: vec![0.0; n],
            capturing: Mutex::new(false),
        }
    }
}

impl Default for MockAudioCapture {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl AudioCapture for MockAudioCapture {
    fn start(&mut self, _device: Option<DeviceId>) -> Result<()> {
        *self.capturing.lock().unwrap() = true;
        self.samples.lock().unwrap().clear();
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
        let mut buf = self.samples.lock().unwrap();
        *buf = self.canned.clone();
        Ok(buf.clone())
    }

    fn snapshot(&self) -> Vec<f32> {
        self.samples.lock().unwrap().clone()
    }

    fn devices(&self) -> Result<Vec<DeviceId>> {
        Ok(vec![DeviceId("mock-default".to_string())])
    }
}

// Phase 1:
// #[cfg(feature = "cpal-audio")]
// pub mod cpal_backend; // CpalAudioCapture: WASAPI/ALSA-PipeWire + rubato resample.
