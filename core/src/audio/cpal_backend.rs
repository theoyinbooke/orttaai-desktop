//! Real microphone capture: `cpal` (WASAPI / ALSA-PipeWire / CoreAudio) with
//! streaming `rubato` resampling to 16 kHz mono f32. Enabled by `cpal-audio`.
//!
//! `cpal::Stream` is `!Send`, so it is owned by a dedicated audio thread; the
//! command channel is wrapped in a `Mutex` so this type stays `Send + Sync`.
//!
//! Resampling happens in the stream callback as audio arrives, so the 16 kHz
//! buffer is always current: the hands-free monitor reads only the new samples
//! each poll instead of resampling a ten-minute recording four times a second.
//!
//! The audio thread also watches the stream's health (Mac
//! `AudioCaptureService`): a stream error, a stall (no frames for 1.5 s), or a
//! stream that never delivers its first frame tears the stream down and
//! re-opens it on the Mac recovery schedule — keeping every captured sample and
//! falling back to the system default device when the chosen one is gone.

use super::{
    meter_level, AudioCapture, RECOVERY_RETRY_DELAYS_MS, STALE_STARTUP_RETRY_DELAYS_MS,
    STALL_RESTART_MS, STARTUP_RETRY_DELAYS_MS,
};
use crate::error::{CoreError, Result};
use crate::types::{AudioDevice, DeviceId, TARGET_SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use rubato::{FftFixedIn, Resampler};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Resampler input chunk size (frames at the source rate).
const RESAMPLE_CHUNK: usize = 1024;
/// A fresh stream that has produced nothing by now is stale (Mac health check).
const FIRST_FRAME_TIMEOUT_MS: u64 = 800;

/// The streaming 16 kHz conversion state, shared with the stream callback.
#[derive(Default)]
struct Converter {
    resampler: Option<FftFixedIn<f32>>,
    /// Mono source-rate audio waiting for a full resampler chunk.
    pending: Vec<f32>,
    /// The recording so far, at 16 kHz.
    out: Vec<f32>,
}

impl Converter {
    /// Prepare for a stream at `source_rate`, keeping (or clearing) the output.
    fn configure(&mut self, source_rate: u32, keep: bool) -> Result<()> {
        self.flush();
        if !keep {
            self.out.clear();
        }
        self.resampler = if source_rate == TARGET_SAMPLE_RATE {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(
                    source_rate as usize,
                    TARGET_SAMPLE_RATE as usize,
                    RESAMPLE_CHUNK,
                    1,
                    1,
                )
                .map_err(|e| CoreError::Audio(format!("resampler init: {e}")))?,
            )
        };
        Ok(())
    }

    fn push(&mut self, mono: &[f32]) {
        let Some(resampler) = self.resampler.as_mut() else {
            self.out.extend_from_slice(mono);
            return;
        };
        self.pending.extend_from_slice(mono);
        loop {
            let need = resampler.input_frames_next();
            if self.pending.len() < need {
                break;
            }
            match resampler.process(&[&self.pending[..need]], None) {
                Ok(chunk) => self.out.extend_from_slice(&chunk[0]),
                Err(e) => tracing::warn!("resample: {e}"),
            }
            self.pending.drain(..need);
        }
    }

    /// Resample whatever partial chunk is left.
    fn flush(&mut self) {
        if let Some(resampler) = self.resampler.as_mut() {
            if !self.pending.is_empty() {
                if let Ok(chunk) = resampler.process_partial(Some(&[&self.pending]), None) {
                    self.out.extend_from_slice(&chunk[0]);
                }
            }
        }
        self.pending.clear();
    }
}

struct Shared {
    conv: Mutex<Converter>,
    /// Meter level as f32 bits.
    level: AtomicU32,
    /// Callback invocations since the stream was (re)built.
    frames: AtomicU64,
    stream_error: AtomicBool,
    active_device: Mutex<Option<String>>,
    restarts: AtomicU32,
    recovering: AtomicBool,
}

impl Shared {
    fn set_level(&self, v: f32) {
        self.level.store(v.to_bits(), Ordering::Relaxed);
    }
}

enum Cmd {
    Start {
        device: Option<DeviceId>,
        reply: Sender<Result<()>>,
    },
    Stop {
        reply: Sender<Vec<f32>>,
    },
    Reset {
        reply: Sender<()>,
    },
    Shutdown,
}

pub struct CpalAudioCapture {
    cmd_tx: Mutex<Sender<Cmd>>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl CpalAudioCapture {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            conv: Mutex::new(Converter::default()),
            level: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            stream_error: AtomicBool::new(false),
            active_device: Mutex::new(None),
            restarts: AtomicU32::new(0),
            recovering: AtomicBool::new(false),
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
        let (reply, rx) = channel();
        self.send(Cmd::Start { device, reply })?;
        rx.recv()
            .map_err(|_| CoreError::Audio("audio thread did not reply".to_string()))?
    }

    fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    fn stop(&mut self) -> Result<Vec<f32>> {
        let (reply, rx) = channel();
        self.send(Cmd::Stop { reply })?;
        rx.recv()
            .map_err(|_| CoreError::Audio("audio thread did not reply".to_string()))
    }

    fn snapshot(&self) -> Vec<f32> {
        self.shared.conv.lock().unwrap().out.clone()
    }

    fn captured_samples(&self) -> usize {
        self.shared.conv.lock().unwrap().out.len()
    }

    fn samples_from(&self, start: usize) -> Vec<f32> {
        let conv = self.shared.conv.lock().unwrap();
        conv.out
            .get(start..)
            .map(<[f32]>::to_vec)
            .unwrap_or_default()
    }

    #[allow(deprecated)]
    fn devices(&self) -> Result<Vec<DeviceId>> {
        Ok(list_input_devices()?
            .into_iter()
            .map(|d| DeviceId(d.name))
            .collect())
    }

    fn active_device(&self) -> Option<String> {
        self.shared.active_device.lock().unwrap().clone()
    }

    fn restart_count(&self) -> u32 {
        self.shared.restarts.load(Ordering::Relaxed)
    }

    fn is_recovering(&self) -> bool {
        self.shared.recovering.load(Ordering::Relaxed)
    }

    fn reset(&mut self) -> Result<()> {
        let (reply, rx) = channel();
        self.send(Cmd::Reset { reply })?;
        rx.recv()
            .map_err(|_| CoreError::Audio("audio thread did not reply".to_string()))
    }
}

/// Mid-recording recovery in progress.
struct Recovery {
    attempt: usize,
    next_at: Instant,
    device: Option<DeviceId>,
}

fn audio_thread(cmd_rx: std::sync::mpsc::Receiver<Cmd>, shared: Arc<Shared>) {
    // The `!Send` stream lives only here.
    let mut stream: Option<cpal::Stream> = None;
    let mut recording = false;
    let mut requested: Option<DeviceId> = None;
    let mut recovery: Option<Recovery> = None;
    let mut stale = false;
    let mut started_at = Instant::now();
    let mut last_frames = 0u64;
    let mut last_progress = Instant::now();

    loop {
        match cmd_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Cmd::Start { device, reply }) => {
                stream = None;
                recovery = None;
                shared.recovering.store(false, Ordering::Relaxed);
                shared.restarts.store(0, Ordering::Relaxed);
                shared.set_level(0.0);
                let delays: &[u64] = if stale {
                    &STALE_STARTUP_RETRY_DELAYS_MS
                } else {
                    &STARTUP_RETRY_DELAYS_MS
                };
                stale = false;
                let mut attempt_device = device.clone();
                let mut last_err = CoreError::Audio("no audio input".to_string());
                let mut ok = false;
                for (i, &delay) in delays.iter().enumerate() {
                    if delay > 0 {
                        std::thread::sleep(Duration::from_millis(delay));
                    }
                    match open_stream(attempt_device.as_ref(), &shared, false) {
                        Ok(s) => {
                            if i > 0 {
                                tracing::info!("audio capture started on retry {}", i + 1);
                            }
                            stream = Some(s);
                            ok = true;
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("audio start attempt {} failed: {e}", i + 1);
                            last_err = e;
                            // The chosen mic may have disconnected or been
                            // renamed; record from the system default instead.
                            if attempt_device.take().is_some() {
                                tracing::warn!("preferred input failed; using system default");
                            }
                        }
                    }
                }
                if ok {
                    recording = true;
                    requested = device;
                    started_at = Instant::now();
                    last_frames = 0;
                    last_progress = started_at;
                    let _ = reply.send(Ok(()));
                } else {
                    recording = false;
                    let _ = reply.send(Err(last_err));
                }
            }
            Ok(Cmd::Stop { reply }) => {
                stream = None; // drop → stops capture
                recording = false;
                recovery = None;
                shared.recovering.store(false, Ordering::Relaxed);
                shared.set_level(0.0);
                let samples = {
                    let mut conv = shared.conv.lock().unwrap();
                    conv.flush();
                    std::mem::take(&mut conv.out)
                };
                let _ = reply.send(samples);
            }
            Ok(Cmd::Reset { reply }) => {
                stream = None;
                recording = false;
                recovery = None;
                stale = true;
                shared.recovering.store(false, Ordering::Relaxed);
                shared.set_level(0.0);
                {
                    let mut conv = shared.conv.lock().unwrap();
                    *conv = Converter::default();
                }
                let _ = reply.send(());
            }
            Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        if !recording {
            continue;
        }
        let now = Instant::now();

        if let Some(rec) = recovery.as_mut() {
            if now >= rec.next_at {
                match open_stream(rec.device.as_ref(), &shared, true) {
                    Ok(s) => {
                        tracing::info!(
                            "capture recovered on attempt {}; recording continues",
                            rec.attempt + 1
                        );
                        stream = Some(s);
                        shared.restarts.fetch_add(1, Ordering::Relaxed);
                        shared.recovering.store(false, Ordering::Relaxed);
                        recovery = None;
                        started_at = now;
                        last_frames = 0;
                        last_progress = now;
                    }
                    Err(e) => {
                        tracing::warn!("capture recovery attempt {} failed: {e}", rec.attempt + 1);
                        // The chosen mic may be gone; carry on from the default.
                        rec.device = None;
                        rec.attempt += 1;
                        match RECOVERY_RETRY_DELAYS_MS.get(rec.attempt) {
                            Some(&d) => rec.next_at = now + Duration::from_millis(d),
                            None => {
                                tracing::error!(
                                    "capture could not be recovered; the recording keeps what it had"
                                );
                                shared.recovering.store(false, Ordering::Relaxed);
                                recovery = None;
                                recording = false;
                            }
                        }
                    }
                }
            }
            continue;
        }

        let frames = shared.frames.load(Ordering::Relaxed);
        if frames != last_frames {
            last_frames = frames;
            last_progress = now;
        }
        let errored = shared.stream_error.swap(false, Ordering::Relaxed);
        let stalled =
            frames > 0 && now.duration_since(last_progress).as_millis() as u64 >= STALL_RESTART_MS;
        let never_started = frames == 0
            && now.duration_since(started_at).as_millis() as u64 >= FIRST_FRAME_TIMEOUT_MS;
        if errored || stalled || never_started {
            let reason = if errored {
                "stream error"
            } else if stalled {
                "no audio for 1.5 s"
            } else {
                "no samples after 800 ms"
            };
            let kept = shared.conv.lock().unwrap().out.len();
            tracing::warn!("capture interrupted ({reason}) after {kept} samples; restarting");
            stream = None;
            shared.set_level(0.0);
            shared.recovering.store(true, Ordering::Relaxed);
            recovery = Some(Recovery {
                attempt: 0,
                next_at: now + Duration::from_millis(RECOVERY_RETRY_DELAYS_MS[0]),
                device: requested.clone(),
            });
        }
    }
    drop(stream);
}

/// Find an input device by its persistent name.
#[allow(deprecated)]
fn find_device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    host.input_devices()
        .ok()?
        .find(|d| d.name().map(|n| n == name).unwrap_or(false))
}

/// Open and start a stream; configures the shared converter for its rate.
#[allow(deprecated)]
fn open_stream(
    device: Option<&DeviceId>,
    shared: &Arc<Shared>,
    keep: bool,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = match device {
        Some(id) => find_device(&host, &id.0)
            .ok_or_else(|| CoreError::Audio(format!("input device not found: {}", id.0)))?,
        None => host
            .default_input_device()
            .ok_or_else(|| CoreError::Audio("no default input device".to_string()))?,
    };
    let name = device.name().ok();

    let supported = device
        .default_input_config()
        .map_err(|e| CoreError::Audio(format!("default input config: {e}")))?;
    let sample_format = supported.sample_format();
    let channels = supported.channels() as usize;
    let source_rate = supported.sample_rate();
    let config = supported.config();

    shared.conv.lock().unwrap().configure(source_rate, keep)?;
    shared.frames.store(0, Ordering::Relaxed);
    shared.stream_error.store(false, Ordering::Relaxed);

    let stream = match sample_format {
        SampleFormat::F32 => build_typed_stream::<f32>(&device, &config, channels, shared.clone()),
        SampleFormat::I16 => build_typed_stream::<i16>(&device, &config, channels, shared.clone()),
        SampleFormat::U16 => build_typed_stream::<u16>(&device, &config, channels, shared.clone()),
        SampleFormat::I32 => build_typed_stream::<i32>(&device, &config, channels, shared.clone()),
        other => Err(CoreError::Audio(format!(
            "unsupported sample format: {other:?}"
        ))),
    }?;
    stream
        .play()
        .map_err(|e| CoreError::Audio(format!("play stream: {e}")))?;
    *shared.active_device.lock().unwrap() = name;
    Ok(stream)
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
    let err_shared = shared.clone();
    let mut mono: Vec<f32> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                mono.clear();
                let mut sum_sq = 0.0f32;
                for frame in data.chunks(channels.max(1)) {
                    let value = frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>()
                        / frame.len() as f32;
                    sum_sq += value * value;
                    mono.push(value);
                }
                if !mono.is_empty() {
                    let rms = (sum_sq / mono.len() as f32).sqrt();
                    shared.set_level(meter_level(rms));
                    shared.conv.lock().unwrap().push(&mono);
                }
                shared.frames.fetch_add(1, Ordering::Relaxed);
            },
            move |err| {
                tracing::error!("audio stream error: {err}");
                err_shared.stream_error.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| CoreError::Audio(format!("build input stream: {e}")))
}

/// Input devices with the system default flagged (Settings › Audio).
#[allow(deprecated)]
pub fn list_input_devices() -> Result<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    let mut out: Vec<AudioDevice> = Vec::new();
    let devices = host
        .input_devices()
        .map_err(|e| CoreError::Audio(format!("enumerate devices: {e}")))?;
    for device in devices {
        if let Ok(name) = device.name() {
            if out.iter().any(|d| d.name == name) {
                continue;
            }
            out.push(AudioDevice {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
            });
        }
    }
    Ok(out)
}

/// A live input-level meter outside of dictation (Settings › Audio, Setup mic
/// check). Opens its own stream on a dedicated thread; dropping it stops it.
pub struct LevelMonitor {
    level: Arc<AtomicU32>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
    device: Option<String>,
}

impl LevelMonitor {
    /// Start metering `device` (by name; `None` = system default). A missing
    /// named device falls back to the default, like recording does.
    #[allow(deprecated)]
    pub fn start(device: Option<String>) -> Result<Self> {
        let level = Arc::new(AtomicU32::new(0));
        let (stop_tx, stop_rx) = channel::<()>();
        let (ready_tx, ready_rx) = channel::<Result<Option<String>>>();
        let thread_level = level.clone();
        let thread = std::thread::Builder::new()
            .name("orttaai-level".into())
            .spawn(move || {
                let host = cpal::default_host();
                let dev = device
                    .as_deref()
                    .and_then(|n| find_device(&host, n))
                    .or_else(|| host.default_input_device());
                let Some(dev) = dev else {
                    let _ = ready_tx.send(Err(CoreError::Audio("no input device".into())));
                    return;
                };
                let name = dev.name().ok();
                let built = (|| -> Result<cpal::Stream> {
                    let supported = dev
                        .default_input_config()
                        .map_err(|e| CoreError::Audio(format!("default input config: {e}")))?;
                    let channels = supported.channels() as usize;
                    let config = supported.config();
                    let lvl = thread_level.clone();
                    let stream = match supported.sample_format() {
                        SampleFormat::F32 => meter_stream::<f32>(&dev, &config, channels, lvl),
                        SampleFormat::I16 => meter_stream::<i16>(&dev, &config, channels, lvl),
                        SampleFormat::U16 => meter_stream::<u16>(&dev, &config, channels, lvl),
                        SampleFormat::I32 => meter_stream::<i32>(&dev, &config, channels, lvl),
                        other => Err(CoreError::Audio(format!(
                            "unsupported sample format: {other:?}"
                        ))),
                    }?;
                    stream
                        .play()
                        .map_err(|e| CoreError::Audio(format!("play stream: {e}")))?;
                    Ok(stream)
                })();
                match built {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(name));
                        let _ = stop_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| CoreError::Audio(format!("spawn level monitor: {e}")))?;
        let device = ready_rx
            .recv()
            .map_err(|_| CoreError::Audio("level monitor exited".into()))??;
        Ok(Self {
            level,
            stop: Some(stop_tx),
            thread: Some(thread),
            device,
        })
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// The device actually being metered.
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }
}

impl Drop for LevelMonitor {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn meter_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    level: Arc<AtomicU32>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut sum_sq = 0.0f32;
                let mut n = 0usize;
                for frame in data.chunks(channels.max(1)) {
                    let v = frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>()
                        / frame.len() as f32;
                    sum_sq += v * v;
                    n += 1;
                }
                if n > 0 {
                    let rms = (sum_sq / n as f32).sqrt();
                    level.store(meter_level(rms).to_bits(), Ordering::Relaxed);
                }
            },
            move |err| tracing::error!("level monitor stream error: {err}"),
            None,
        )
        .map_err(|e| CoreError::Audio(format!("build input stream: {e}")))
}

/// One-shot resample of mono `f32` from `source_rate` to 16 kHz (used for WAV
/// files and tests; live capture resamples as it streams).
pub fn resample_to_target(mono: &[f32], source_rate: u32) -> Result<Vec<f32>> {
    if mono.is_empty() {
        return Ok(Vec::new());
    }
    let mut conv = Converter::default();
    conv.configure(source_rate, false)?;
    for chunk in mono.chunks(4096) {
        conv.push(chunk);
    }
    conv.flush();
    Ok(conv.out)
}

#[cfg(test)]
mod tests {
    use super::{resample_to_target, Converter};

    fn sine(rate: usize, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn resamples_48k_to_16k() {
        let out = resample_to_target(&sine(48_000, 1.0), 48_000).unwrap();
        assert!(
            (out.len() as i32 - 16_000).abs() < 1024,
            "expected ~16000 samples, got {}",
            out.len()
        );
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

    #[test]
    fn streaming_in_small_callbacks_matches_length() {
        let mut conv = Converter::default();
        conv.configure(44_100, false).unwrap();
        let input = sine(44_100, 2.0);
        for chunk in input.chunks(441) {
            conv.push(chunk);
        }
        conv.flush();
        assert!(
            (conv.out.len() as i32 - 32_000).abs() < 1500,
            "{}",
            conv.out.len()
        );
    }

    #[test]
    fn reconfigure_keeps_samples_across_a_restart() {
        let mut conv = Converter::default();
        conv.configure(16_000, false).unwrap();
        conv.push(&[0.5; 1000]);
        // Recovery onto a 48 kHz fallback device keeps what was captured.
        conv.configure(48_000, true).unwrap();
        conv.push(&sine(48_000, 0.5));
        conv.flush();
        assert!(conv.out.len() > 1000 + 7000, "{}", conv.out.len());
        assert_eq!(conv.out[0], 0.5);
    }
}
