//! NeMo's audio front end for Parakeet (`AudioToMelSpectrogramPreprocessor`
//! with the model's settings): 0.97 pre-emphasis, a 512-point STFT of 25 ms
//! Hann windows every 10 ms (centred, zero padded), power spectrum, 80 Slaney
//! mel bands from 0 to 8 kHz, natural log with a 2^-24 guard, then each band
//! normalised to zero mean and unit variance over the recording. The Mac's
//! Word Boost runs the same transform as a Core ML model (`MelSpectrogram`).

use rustfft::{num_complex::Complex32, FftPlanner};

pub const SAMPLE_RATE: usize = 16_000;
pub const N_FFT: usize = 512;
pub const WIN_LENGTH: usize = 400;
pub const HOP: usize = 160;
pub const N_MELS: usize = 80;
const PREEMPH: f32 = 0.97;
const LOG_GUARD: f32 = 5.960_464_5e-8; // 2^-24
const STD_EPS: f32 = 1e-5;

/// `librosa.filters.mel(sr=16000, n_fft=512, n_mels=80, fmin=0, fmax=8000,
/// norm="slaney")`, as `N_MELS` rows of `N_FFT / 2 + 1` weights.
pub fn mel_filterbank() -> Vec<Vec<f32>> {
    fn hz_to_mel(f: f64) -> f64 {
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = 6.4f64.ln() / 27.0;
        if f >= min_log_hz {
            min_log_mel + (f / min_log_hz).ln() / logstep
        } else {
            f / f_sp
        }
    }
    fn mel_to_hz(m: f64) -> f64 {
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = 6.4f64.ln() / 27.0;
        if m >= min_log_mel {
            min_log_hz * (logstep * (m - min_log_mel)).exp()
        } else {
            f_sp * m
        }
    }
    let bins = N_FFT / 2 + 1;
    let fmax = SAMPLE_RATE as f64 / 2.0;
    let fft_freqs: Vec<f64> = (0..bins)
        .map(|k| k as f64 * fmax / (bins - 1) as f64)
        .collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(fmax));
    let mel_f: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64))
        .collect();
    (0..N_MELS)
        .map(|i| {
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            fft_freqs
                .iter()
                .map(|&f| {
                    let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
                    let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
                    (lower.min(upper).max(0.0) * enorm) as f32
                })
                .collect()
        })
        .collect()
}

/// The number of feature frames NeMo produces for `samples` (centred STFT).
pub fn frame_count(samples: usize) -> usize {
    samples / HOP + 1
}

/// Log-mel features for 16 kHz mono audio, laid out `[N_MELS][frames]` (the
/// model's `[1, 80, T]` input).
pub struct Featurizer {
    filterbank: Vec<Vec<f32>>,
    window: Vec<f32>,
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl Default for Featurizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Featurizer {
    pub fn new() -> Self {
        // torch.hann_window(400, periodic=False), centred inside the 512-point frame.
        let mut window = vec![0.0f32; N_FFT];
        let offset = (N_FFT - WIN_LENGTH) / 2;
        for n in 0..WIN_LENGTH {
            let w =
                0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (WIN_LENGTH - 1) as f64).cos();
            window[offset + n] = w as f32;
        }
        Self {
            filterbank: mel_filterbank(),
            window,
            fft: FftPlanner::new().plan_fft_forward(N_FFT),
        }
    }

    pub fn compute(&self, samples: &[f32]) -> Vec<Vec<f32>> {
        let frames = frame_count(samples.len());
        if samples.is_empty() {
            return vec![vec![0.0; frames]; N_MELS];
        }
        // Pre-emphasis, then zero padding of n_fft / 2 on both sides (centre=True).
        let pad = N_FFT / 2;
        let mut x = vec![0.0f32; samples.len() + 2 * pad];
        x[pad] = samples[0];
        for t in 1..samples.len() {
            x[pad + t] = samples[t] - PREEMPH * samples[t - 1];
        }
        let bins = N_FFT / 2 + 1;
        let mut mel = vec![vec![0.0f32; frames]; N_MELS];
        let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut power = vec![0.0f32; bins];
        // Frames fill one column of every mel row, so index by frame.
        #[allow(clippy::needless_range_loop)]
        for f in 0..frames {
            let start = f * HOP;
            for (i, c) in buf.iter_mut().enumerate() {
                let s = x.get(start + i).copied().unwrap_or(0.0);
                *c = Complex32::new(s * self.window[i], 0.0);
            }
            self.fft.process(&mut buf);
            for (k, p) in power.iter_mut().enumerate() {
                *p = buf[k].norm_sqr();
            }
            for (m, row) in self.filterbank.iter().enumerate() {
                let e: f32 = row.iter().zip(&power).map(|(w, p)| w * p).sum();
                mel[m][f] = (e + LOG_GUARD).ln();
            }
        }
        // Per-feature normalisation over the whole recording (unbiased std).
        for row in &mut mel {
            let n = row.len() as f32;
            let mean = row.iter().sum::<f32>() / n;
            let var = if row.len() > 1 {
                row.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / (n - 1.0)
            } else {
                0.0
            };
            let std = var.sqrt() + STD_EPS;
            for v in row.iter_mut() {
                *v = (*v - mean) / std;
            }
        }
        mel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filterbank_matches_librosa_shape_and_norm() {
        let fb = mel_filterbank();
        assert_eq!(fb.len(), 80);
        assert_eq!(fb[0].len(), 257);
        // Slaney-normalised triangles: each band's peak is 2 / bandwidth (in Hz).
        assert!(fb.iter().all(|row| row.iter().all(|w| *w >= 0.0)));
        assert!(
            fb.iter().all(|row| row.iter().any(|w| *w > 0.0)),
            "no empty band"
        );
        // Reference values from librosa 0.11 for these exact parameters.
        let peak = |row: &[f32]| row.iter().cloned().fold(0.0f32, f32::max);
        for (band, want_peak, want_sum) in [
            (0usize, 0.022_534_56f32, 0.031_172f32),
            (10, f32::NAN, 0.033_053),
            (40, 0.014_444_176, 0.032_116),
            (79, 0.003_252_148_4, 0.031_931),
        ] {
            if !want_peak.is_nan() {
                assert!(
                    (peak(&fb[band]) - want_peak).abs() < 1e-6,
                    "band {band} peak {}",
                    peak(&fb[band])
                );
            }
            let sum: f32 = fb[band].iter().sum();
            assert!((sum - want_sum).abs() < 1e-5, "band {band} sum {sum}");
        }
        let total: f32 = fb.iter().flatten().sum();
        assert!((total - 2.558_260_7).abs() < 1e-4, "total {total}");
    }

    #[test]
    fn frames_and_normalisation() {
        let f = Featurizer::new();
        let samples: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        let mel = f.compute(&samples);
        assert_eq!(mel.len(), 80);
        assert_eq!(mel[0].len(), 101);
        for row in &mel {
            let mean: f32 = row.iter().sum::<f32>() / row.len() as f32;
            assert!(mean.abs() < 1e-3, "zero mean, got {mean}");
        }
    }
}
