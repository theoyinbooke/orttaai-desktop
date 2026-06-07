//! Real microphone capture: `cpal` (WASAPI / ALSA-PipeWire / CoreAudio) with
//! `rubato` resampling to 16 kHz mono f32. Enabled by the `cpal-audio` feature.
//!
//! `cpal::Stream` is `!Send`, so it is owned by a dedicated audio thread; the
//! command channel is wrapped in a `Mutex` so this type stays `Send + Sync` and
//! satisfies the [`AudioCapture`] bound.

use super::AudioCapture;
use crate::error::{CoreError, Result};
use crate::types::{DeviceId, TARGET_SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use rubato::{FftFixedIn, Resampler};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Resampler input chunk size (frames at the source rate).
const RESAMPLE_CHUNK: usize = 1024;

struct Shared {
    /// Mono f32 captured at the source sample rate.
    samples: Mutex<Vec<f32>>,
    /// Most-recent peak level in `0.0..=1.0`.
    level: Mutex<f32>,
}

enum Cmd {
    Start {
        device: Option<DeviceId>,
        reply: Sender<Result<u32>>,
    },
    Stop {
        reply: Sender<()>,
    },
    Shutdown,
}

pub struct CpalAudioCapture {
    cmd_tx: Mutex<Sender<Cmd>>,
    shared: Arc<Shared>,
    source_rate: u32,
    thread: Option<JoinHandle<()>>,
}

impl CpalAudioCapture {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::new()),
            level: Mutex::new(0.0),
        });
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("orttaai-audio".to_string())
            .spawn(move || audio_thread(cmd_rx, thread_shared))
            .expect("spawn audio thread");

        Self {
            cmd_tx: Mutex::new(cmd_tx),
            shared,
            source_rate: TARGET_SAMPLE_RATE,
            thread: Some(thread),
        }
    }

    fn send(&self, cmd: Cmd) -> Result<()> {
        self.cmd_tx
            .lock()
            .unwrap()
            .send(cmd)
            .map_err(|_| CoreError::Audio("audio thread is gone".to_string()))
    }
}

impl Default for CpalAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CpalAudioCapture {
    fn drop(&mut self) {
        let _ = self.send(Cmd::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl AudioCapture for CpalAudioCapture {
    fn start(&mut self, device: Option<DeviceId>) -> Result<()> {
        self.shared.samples.lock().unwrap().clear();
        *self.shared.level.lock().unwrap() = 0.0;

        let (reply, rx) = channel();
        self.send(Cmd::Start { device, reply })?;
        let rate = rx
            .recv()
            .map_err(|_| CoreError::Audio("audio thread did not reply".to_string()))??;
        self.source_rate = rate;
        Ok(())
    }

    fn level(&self) -> f32 {
        *self.shared.level.lock().unwrap()
    }

    fn stop(&mut self) -> Result<Vec<f32>> {
        let (reply, rx) = channel();
        self.send(Cmd::Stop { reply })?;
        rx.recv()
            .map_err(|_| CoreError::Audio("audio thread did not reply".to_string()))?;

        let mono = std::mem::take(&mut *self.shared.samples.lock().unwrap());
        resample_to_target(&mono, self.source_rate)
    }

    fn snapshot(&self) -> Vec<f32> {
        let mono = self.shared.samples.lock().unwrap().clone();
        resample_to_target(&mono, self.source_rate).unwrap_or(mono)
    }

    #[allow(deprecated)]
    fn devices(&self) -> Result<Vec<DeviceId>> {
        let host = cpal::default_host();
        let mut out = Vec::new();
        let devices = host
            .input_devices()
            .map_err(|e| CoreError::Audio(format!("enumerate devices: {e}")))?;
        for device in devices {
            if let Ok(name) = device.name() {
                out.push(DeviceId(name));
            }
        }
        Ok(out)
    }
}

fn audio_thread(cmd_rx: std::sync::mpsc::Receiver<Cmd>, shared: Arc<Shared>) {
    // The `!Send` stream lives only here.
    let mut stream: Option<cpal::Stream> = None;
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Cmd::Start { device, reply } => {
                let result = build_stream(device.as_ref(), &shared).and_then(|(s, rate)| {
                    s.play()
                        .map_err(|e| CoreError::Audio(format!("play stream: {e}")))?;
                    stream = Some(s);
                    Ok(rate)
                });
                let _ = reply.send(result);
            }
            Cmd::Stop { reply } => {
                stream = None; // drop → stops capture
                let _ = reply.send(());
            }
            Cmd::Shutdown => break,
        }
    }
}

// cpal's `Device::name()` is deprecated in favor of `id()`/`description()`;
// migrate when we add stable device persistence. For now the human name is fine.
#[allow(deprecated)]
fn build_stream(device: Option<&DeviceId>, shared: &Arc<Shared>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = match device {
        Some(id) => host
            .input_devices()
            .map_err(|e| CoreError::Audio(format!("enumerate devices: {e}")))?
            .find(|d| d.name().map(|n| n == id.0).unwrap_or(false))
            .ok_or_else(|| CoreError::Audio(format!("input device not found: {}", id.0)))?,
        None => host
            .default_input_device()
            .ok_or_else(|| CoreError::Audio("no default input device".to_string()))?,
    };

    let supported = device
        .default_input_config()
        .map_err(|e| CoreError::Audio(format!("default input config: {e}")))?;
    let sample_format = supported.sample_format();
    let channels = supported.channels() as usize;
    let source_rate = supported.sample_rate();
    let config = supported.config();

    let stream = match sample_format {
        SampleFormat::F32 => build_typed_stream::<f32>(&device, &config, channels, shared.clone()),
        SampleFormat::I16 => build_typed_stream::<i16>(&device, &config, channels, shared.clone()),
        SampleFormat::U16 => build_typed_stream::<u16>(&device, &config, channels, shared.clone()),
        other => Err(CoreError::Audio(format!(
            "unsupported sample format: {other:?}"
        ))),
    }?;

    Ok((stream, source_rate))
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    shared: Arc<Shared>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut peak = 0.0f32;
                let mut samples = shared.samples.lock().unwrap();
                for frame in data.chunks(channels) {
                    let mut sum = 0.0f32;
                    for &sample in frame {
                        sum += f32::from_sample(sample);
                    }
                    let value = sum / channels as f32;
                    peak = peak.max(value.abs());
                    samples.push(value);
                }
                drop(samples);
                *shared.level.lock().unwrap() = peak;
            },
            move |err| tracing::error!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| CoreError::Audio(format!("build input stream: {e}")))?;
    Ok(stream)
}

/// Resample mono `f32` from `source_rate` to 16 kHz.
fn resample_to_target(mono: &[f32], source_rate: u32) -> Result<Vec<f32>> {
    if mono.is_empty() {
        return Ok(Vec::new());
    }
    if source_rate == TARGET_SAMPLE_RATE {
        return Ok(mono.to_vec());
    }

    let mut resampler = FftFixedIn::<f32>::new(
        source_rate as usize,
        TARGET_SAMPLE_RATE as usize,
        RESAMPLE_CHUNK,
        1,
        1,
    )
    .map_err(|e| CoreError::Audio(format!("resampler init: {e}")))?;

    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let need = resampler.input_frames_next();
        if pos + need > mono.len() {
            break;
        }
        let resampled = resampler
            .process(&[&mono[pos..pos + need]], None)
            .map_err(|e| CoreError::Audio(format!("resample: {e}")))?;
        out.extend_from_slice(&resampled[0]);
        pos += need;
    }
    if pos < mono.len() {
        let resampled = resampler
            .process_partial(Some(&[&mono[pos..]]), None)
            .map_err(|e| CoreError::Audio(format!("resample tail: {e}")))?;
        out.extend_from_slice(&resampled[0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::resample_to_target;

    #[test]
    fn resamples_48k_to_16k() {
        // 1 second of a 440 Hz sine at 48 kHz → expect ~16 000 samples at 16 kHz.
        let src_rate = 48_000usize;
        let input: Vec<f32> = (0..src_rate)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / src_rate as f32).sin())
            .collect();

        let out = resample_to_target(&input, 48_000).unwrap();

        assert!(
            (out.len() as i32 - 16_000).abs() < 1024,
            "expected ~16000 samples, got {}",
            out.len()
        );
        // The sine survives resampling (not silence).
        let peak = out.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak > 0.5, "signal lost in resampling, peak = {peak}");
    }

    #[test]
    fn passthrough_when_already_16k() {
        let input = vec![0.1, -0.2, 0.3, -0.4];
        assert_eq!(resample_to_target(&input, 16_000).unwrap(), input);
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(resample_to_target(&[], 48_000).unwrap().is_empty());
    }
}
