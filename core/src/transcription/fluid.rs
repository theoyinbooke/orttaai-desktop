//! Fluid Speech Mini (FluidVoice), the Mac's Express model, as its ONNX build
//! (`FluidVoice-Labs/fluid-speech-mini-onnx`): Parakeet TDT 0.6B v3 fine-tuned
//! for dictation, with weights packed at 2 and 4 bits (`MatMulNBits`).
//!
//! A port of the model's reference `transcribe.py`: the Hugging Face Parakeet
//! log-mel features (128 bands from `mel_filters.npy`), the convolutional
//! front end in time chunks, the encoder, then greedy token-and-duration
//! decoding with the decoder and joint network, one token at a time. Audio
//! longer than `max_single_pass_seconds` is cut at quiet points into pieces of
//! about 30 s, as the reference does.
//!
//! Fluid Speech Mini by FluidVoice. Built on NVIDIA Parakeet TDT 0.6B v3 and
//! Moondream Parakeet Ultra. CC BY 4.0.

use super::onnx::{Input, Session, SessionConfig};
use crate::error::{CoreError, Result};
use rustfft::num_complex::Complex64;
use rustfft::FftPlanner;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LOG_ZERO_GUARD: f32 = 5.960_464_5e-8; // 2^-24
const NORM_EPS: f32 = 1e-5;

/// The files the model folder holds.
pub const FILES: [&str; 8] = [
    "frontend.onnx",
    "encoder.onnx",
    "encoder.onnx.data",
    "decoder.onnx",
    "joint.onnx",
    "tokenizer.json",
    "config.json",
    "mel_filters.npy",
];

/// Whether `dir` holds a Fluid Speech model (rather than a sherpa-onnx one).
pub fn is_model_dir(dir: &Path) -> bool {
    files(dir).iter().all(|f| f.is_file())
}

pub fn files(dir: &Path) -> Vec<PathBuf> {
    FILES.iter().map(|f| dir.join(f)).collect()
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    vocab_size: usize,
    blank_token_id: usize,
    pad_token_id: usize,
    eos_token_id: Option<usize>,
    durations: Vec<usize>,
    max_symbols_per_step: usize,
    decoder_layers: usize,
    decoder_hidden_size: usize,
    encoder_hidden_size: usize,
    sampling_rate: usize,
    n_fft: usize,
    hop_length: usize,
    win_length: usize,
    preemphasis: Option<f32>,
    num_mel_bins: usize,
    max_single_pass_seconds: f32,
    subsampling_kernel: i64,
    subsampling_stride: i64,
    subsampling_layers: u32,
    frontend_chunk_frames: usize,
}

/// One decoded clip: the text and, for Word Boost, every token's piece and
/// its time in seconds from the start of the audio.
#[derive(Debug, Clone, Default)]
pub struct Decoded {
    pub text: String,
    pub tokens: Vec<String>,
    pub starts: Vec<f32>,
    pub durations: Vec<f32>,
}

struct Tokenizer {
    pieces: Vec<String>,
    skip: HashSet<usize>,
    space: String,
}

impl Tokenizer {
    fn load(path: &Path, cfg: &Config) -> Result<Self> {
        let bad = |why: &str| CoreError::Transcription(format!("Express tokenizer: {why}"));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path)?).map_err(|e| bad(&e.to_string()))?;
        let vocab = json["model"]["vocab"]
            .as_object()
            .ok_or_else(|| bad("no vocabulary"))?;
        let mut pieces = vec![String::new(); cfg.vocab_size];
        let mut put = |id: usize, piece: &str| {
            if id >= pieces.len() {
                pieces.resize(id + 1, String::new());
            }
            pieces[id] = piece.to_string();
        };
        for (piece, id) in vocab {
            if let Some(id) = id.as_u64() {
                put(id as usize, piece);
            }
        }
        let mut skip: HashSet<usize> = [cfg.blank_token_id, cfg.pad_token_id].into();
        for added in json["added_tokens"].as_array().into_iter().flatten() {
            let (Some(id), Some(content)) = (added["id"].as_u64(), added["content"].as_str())
            else {
                continue;
            };
            put(id as usize, content);
            // Control tokens (<|nospeech|>, language tags) are never text.
            let control = content.starts_with("<|") && content.ends_with("|>");
            if added["special"].as_bool() == Some(true) || control {
                skip.insert(id as usize);
            }
        }
        let space = json["decoder"]["replacement"]
            .as_str()
            .unwrap_or("\u{2581}")
            .to_string();
        Ok(Self {
            pieces,
            skip,
            space,
        })
    }

    fn is_text(&self, id: usize) -> bool {
        !self.skip.contains(&id) && id < self.pieces.len()
    }

    fn decode(&self, ids: &[usize]) -> String {
        let text: String = ids
            .iter()
            .filter(|&&id| self.is_text(id))
            .map(|&id| self.pieces[id].as_str())
            .collect::<String>()
            .replace(&self.space, " ");
        text.strip_prefix(' ').map(str::to_owned).unwrap_or(text)
    }
}

/// `mel_filters.npy`: a little-endian float32 `[mels, n_fft / 2 + 1]` array.
fn read_filters(path: &Path, mels: usize, bins: usize) -> Result<Vec<f32>> {
    let bad = || CoreError::Transcription("Express model: mel_filters.npy is unreadable".into());
    let bytes = std::fs::read(path)?;
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(bad());
    }
    let (header_len, start) = if bytes[6] == 1 {
        (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10)
    } else {
        if bytes.len() < 12 {
            return Err(bad());
        }
        (
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
            12,
        )
    };
    let header = std::str::from_utf8(bytes.get(start..start + header_len).ok_or_else(bad)?)
        .map_err(|_| bad())?;
    let shape = format!("({mels}, {bins})");
    if !header.contains("'<f4'")
        || header.contains("'fortran_order': True")
        || !header.contains(&shape)
    {
        return Err(bad());
    }
    let data = &bytes[start + header_len..];
    if data.len() != mels * bins * 4 {
        return Err(bad());
    }
    Ok(data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

pub struct FluidModel {
    cfg: Config,
    /// `[mels][bins]`, row-major.
    filters: Vec<f32>,
    window: Vec<f64>,
    fft: Arc<dyn rustfft::Fft<f64>>,
    tokenizer: Tokenizer,
    frontend: Option<Session>,
    encoder: Session,
    decoder: Session,
    joint: Session,
}

impl FluidModel {
    pub fn load(dir: &Path, threads: i32) -> Result<Self> {
        for f in files(dir).iter().filter(|f| !f.ends_with("frontend.onnx")) {
            if !f.is_file() {
                return Err(CoreError::Transcription(format!(
                    "Express model file missing: {}",
                    f.display()
                )));
            }
        }
        let cfg: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)
            .map_err(|e| CoreError::Transcription(format!("Express model config: {e}")))?;
        let bins = cfg.n_fft / 2 + 1;
        let filters = read_filters(&dir.join("mel_filters.npy"), cfg.num_mel_bins, bins)?;
        let tokenizer = Tokenizer::load(&dir.join("tokenizer.json"), &cfg)?;
        // A periodic=False Hann window of win_length, centred in n_fft, kept
        // at float32 precision as the reference does.
        let mut window = vec![0.0f64; cfg.n_fft];
        let left = (cfg.n_fft - cfg.win_length) / 2;
        for n in 0..cfg.win_length {
            let w = 0.5
                - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (cfg.win_length - 1) as f64).cos();
            window[left + n] = f64::from(w as f32);
        }
        let big = SessionConfig {
            threads,
            spin: true,
        };
        // Run once per token: one thread, no spinning between tokens.
        let small = SessionConfig {
            threads: 1,
            spin: false,
        };
        let front = dir.join("frontend.onnx");
        let frontend = if front.is_file() {
            Some(Session::load(&front, big)?)
        } else {
            None
        };
        Ok(Self {
            fft: FftPlanner::<f64>::new().plan_fft_forward(cfg.n_fft),
            encoder: Session::load(&dir.join("encoder.onnx"), big)?,
            decoder: Session::load(&dir.join("decoder.onnx"), small)?,
            joint: Session::load(&dir.join("joint.onnx"), small)?,
            frontend,
            filters,
            window,
            tokenizer,
            cfg,
        })
    }

    /// Seconds per encoder frame (10 ms features, subsampled 8 times).
    fn frame_seconds(&self) -> f32 {
        let factor = self.cfg.subsampling_stride.pow(self.cfg.subsampling_layers) as f32;
        self.cfg.hop_length as f32 * factor / self.cfg.sampling_rate as f32
    }

    /// Transcribe 16 kHz mono audio.
    pub fn transcribe(&self, wav: &[f32]) -> Result<Decoded> {
        let rate = self.cfg.sampling_rate;
        if wav.len() < self.cfg.hop_length * 2 {
            return Ok(Decoded::default());
        }
        if wav.len() as f32 <= self.cfg.max_single_pass_seconds * rate as f32 {
            return self.transcribe_pass(wav, 0.0);
        }
        let mut out = Decoded::default();
        let mut texts = Vec::new();
        let mut start = 0usize;
        for piece in self.split_long(wav) {
            let offset = start as f32 / rate as f32;
            start += piece.len();
            let part = self.transcribe_pass(piece, offset)?;
            let text = part.text.trim();
            if !text.is_empty() {
                texts.push(text.to_string());
            }
            out.tokens.extend(part.tokens);
            out.starts.extend(part.starts);
            out.durations.extend(part.durations);
        }
        out.text = texts.join(" ");
        Ok(out)
    }

    fn transcribe_pass(&self, wav: &[f32], offset: f32) -> Result<Decoded> {
        let (feats, mask, frames) = self.log_mel(wav);
        let (enc, enc_frames, valid) = self.encode(&feats, &mask, frames)?;
        drop(feats);
        let steps = self.greedy_tdt(&enc, enc_frames, valid)?;
        let seconds = self.frame_seconds();
        let ids: Vec<usize> = steps.iter().map(|s| s.token).collect();
        let mut out = Decoded {
            text: self.tokenizer.decode(&ids),
            ..Decoded::default()
        };
        for s in steps.iter().filter(|s| self.tokenizer.is_text(s.token)) {
            out.tokens.push(self.tokenizer.pieces[s.token].clone());
            out.starts.push(offset + s.frame as f32 * seconds);
            out.durations.push(s.duration as f32 * seconds);
        }
        Ok(out)
    }

    /// The Hugging Face ParakeetFeatureExtractor's features: `[frames][mels]`
    /// row-major, the attention mask and the frame count.
    fn log_mel(&self, wav: &[f32]) -> (Vec<f32>, Vec<i64>, usize) {
        let cfg = &self.cfg;
        let (n_fft, hop, mels) = (cfg.n_fft, cfg.hop_length, cfg.num_mel_bins);
        let bins = n_fft / 2 + 1;
        let pad = n_fft / 2;
        let mut padded = vec![0.0f32; wav.len() + 2 * pad];
        match cfg.preemphasis {
            Some(p) if p != 0.0 && !wav.is_empty() => {
                padded[pad] = wav[0];
                for i in 1..wav.len() {
                    padded[pad + i] = wav[i] - p * wav[i - 1];
                }
            }
            _ => padded[pad..pad + wav.len()].copy_from_slice(wav),
        }
        let frames = 1 + wav.len() / hop;
        let mut mel = vec![0.0f32; frames * mels];
        let mut buf = vec![Complex64::new(0.0, 0.0); n_fft];
        let mut power = vec![0.0f32; bins];
        for f in 0..frames {
            let start = f * hop;
            for (i, c) in buf.iter_mut().enumerate() {
                *c = Complex64::new(f64::from(padded[start + i]) * self.window[i], 0.0);
            }
            self.fft.process(&mut buf);
            for (p, c) in power.iter_mut().zip(&buf) {
                *p = c.norm_sqr() as f32;
            }
            let row = &mut mel[f * mels..(f + 1) * mels];
            for (m, out) in row.iter_mut().enumerate() {
                let weights = &self.filters[m * bins..(m + 1) * bins];
                let e: f32 = weights.iter().zip(&power).map(|(w, p)| w * p).sum();
                *out = (e + LOG_ZERO_GUARD).ln();
            }
        }
        // Each band normalised over the real frames (unbiased variance); the
        // trailing frame past the audio is zeroed and masked out.
        let valid = (wav.len() / hop).min(frames);
        for m in 0..mels {
            let column = || (0..valid).map(|f| mel[f * mels + m]);
            let mean = column().sum::<f32>() / valid.max(1) as f32;
            let var = column().map(|v| (v - mean) * (v - mean)).sum::<f32>()
                / valid.saturating_sub(1).max(1) as f32;
            let std = var.sqrt() + NORM_EPS;
            for f in 0..frames {
                let v = &mut mel[f * mels + m];
                *v = if f < valid { (*v - mean) / std } else { 0.0 };
            }
        }
        let mask = (0..frames).map(|f| i64::from(f < valid)).collect();
        (mel, mask, frames)
    }

    /// Frames after the convolutional subsampling.
    fn subsampled(&self, mut frames: i64) -> i64 {
        let cfg = &self.cfg;
        let pad = (cfg.subsampling_kernel - 1) / 2 * 2 - cfg.subsampling_kernel;
        for _ in 0..cfg.subsampling_layers {
            frames = ((frames + pad) as f64 / cfg.subsampling_stride as f64 + 1.0).floor() as i64;
        }
        frames
    }

    /// The encoder's output `[frames][hidden]`, its frame count and how many
    /// are real audio. The front end runs in time chunks (same result, far
    /// smaller activations), each with one output frame of context per side.
    fn encode(
        &self,
        feats: &[f32],
        mask: &[i64],
        frames: usize,
    ) -> Result<(Vec<f32>, usize, usize)> {
        let cfg = &self.cfg;
        let mels = cfg.num_mel_bins;
        let hidden_size = cfg.encoder_hidden_size;
        let Some(frontend) = &self.frontend else {
            let out = self.encoder.run(
                &[
                    (
                        c"input_features",
                        Input::F32(feats, &[1, frames as i64, mels as i64]),
                    ),
                    (c"attention_mask", Input::I64(mask, &[1, frames as i64])),
                ],
                &[c"encoder_out"],
            )?;
            let enc = first(out)?;
            let n = enc.shape.get(1).copied().unwrap_or(0);
            let valid = self.subsampled(mask.iter().sum::<i64>()).max(0) as usize;
            return Ok((enc.data, n, valid.min(n)));
        };
        let factor = cfg.subsampling_stride.pow(cfg.subsampling_layers) as usize;
        let total = self.subsampled(frames as i64).max(0) as usize;
        let step = cfg.frontend_chunk_frames.max(1);
        let mut hidden: Vec<f32> = Vec::with_capacity(total * hidden_size);
        let mut a = 0;
        while a < total {
            let b = (a + step).min(total);
            let s = if a > 0 { factor * (a - 1) } else { 0 };
            let e = frames.min(factor * (b + 1));
            let n = e - s;
            let out = frontend.run(
                &[
                    (
                        c"input_features",
                        Input::F32(&feats[s * mels..e * mels], &[1, n as i64, mels as i64]),
                    ),
                    (c"attention_mask", Input::I64(&mask[s..e], &[1, n as i64])),
                ],
                &[c"hidden_states"],
            )?;
            let y = first(out)?;
            let rows = y.shape.get(1).copied().unwrap_or(0);
            let skip = usize::from(a > 0);
            let take = (b - a).min(rows.saturating_sub(skip));
            hidden.extend_from_slice(&y.data[skip * hidden_size..(skip + take) * hidden_size]);
            a = b;
        }
        let rows = hidden.len() / hidden_size;
        let valid = (self.subsampled(mask.iter().sum::<i64>()).max(0) as usize).min(rows);
        let out = self.encoder.run(
            &[
                (
                    c"hidden_states",
                    Input::F32(&hidden, &[1, rows as i64, hidden_size as i64]),
                ),
                (c"length", Input::I64(&[valid as i64], &[1])),
            ],
            &[c"encoder_out"],
        )?;
        let enc = first(out)?;
        let n = enc.shape.get(1).copied().unwrap_or(0);
        Ok((enc.data, n, valid))
    }

    /// Greedy token-and-duration decoding: the loop of transformers'
    /// `ParakeetForTDT.generate` for one clip. Blanks are included.
    fn greedy_tdt(&self, enc: &[f32], frames: usize, valid: usize) -> Result<Vec<Step>> {
        let cfg = &self.cfg;
        let hidden = cfg.encoder_hidden_size;
        if frames == 0 {
            return Ok(Vec::new());
        }
        let (vocab, blank) = (cfg.vocab_size, cfg.blank_token_id);
        let state_shape = [cfg.decoder_layers as i64, 1, cfg.decoder_hidden_size as i64];
        let zeros = vec![0.0f32; cfg.decoder_layers * cfg.decoder_hidden_size];
        let predict =
            |token: usize, h: &[f32], c: &[f32]| -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
                let mut out = self.decoder.run(
                    &[
                        (c"token", Input::I64(&[token as i64], &[1, 1])),
                        (c"h_in", Input::F32(h, &state_shape)),
                        (c"c_in", Input::F32(c, &state_shape)),
                    ],
                    &[c"decoder_out", c"h_out", c"c_out"],
                )?;
                let c = out.pop().map(|o| o.data).unwrap_or_default();
                let h = out.pop().map(|o| o.data).unwrap_or_default();
                let d = out.pop().map(|o| o.data).unwrap_or_default();
                Ok((d, h, c))
            };
        let (mut dec_out, mut h, mut c) = predict(blank, &zeros, &zeros)?;
        let budget = (cfg.max_symbols_per_step * frames).saturating_sub(1).max(1);
        let mut steps = Vec::new();
        let mut t = 0usize;
        loop {
            let frame = t.min(frames - 1);
            let out = self.joint.run(
                &[
                    (
                        c"encoder_frame",
                        Input::F32(
                            &enc[frame * hidden..(frame + 1) * hidden],
                            &[1, hidden as i64],
                        ),
                    ),
                    (
                        c"decoder_out",
                        Input::F32(&dec_out, &[1, cfg.decoder_hidden_size as i64]),
                    ),
                ],
                &[c"logits"],
            )?;
            let logits = first(out)?.data;
            if logits.len() < vocab + cfg.durations.len() {
                return Err(CoreError::Transcription(
                    "Express model: unexpected joint output".into(),
                ));
            }
            let token = argmax(&logits[..vocab]);
            let mut duration = cfg.durations[argmax(&logits[vocab..vocab + cfg.durations.len()])];
            if token == blank && duration == 0 {
                duration = 1;
            }
            steps.push(Step {
                token,
                frame: t,
                duration,
            });
            t += duration;
            if t >= valid || steps.len() >= budget || Some(token) == cfg.eos_token_id {
                return Ok(steps);
            }
            if token != blank {
                (dec_out, h, c) = predict(token, &h, &c)?;
            }
        }
    }

    /// Cut audio longer than one pass at the quietest moment 25–35 s into
    /// each piece.
    fn split_long<'a>(&self, wav: &'a [f32]) -> Vec<&'a [f32]> {
        let rate = self.cfg.sampling_rate;
        let (target, search) = (30.0f32, 10.0f32);
        let hop = rate / 50;
        let energy: Vec<f64> = wav
            .chunks(hop)
            .map(|c| {
                (c.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / hop as f64).sqrt()
            })
            .collect();
        // np.convolve(energy, ones(10) / 10, "same"): the mean of
        // energy[i - 5 ..= i + 4], zeros past either end.
        let smooth: Vec<f64> = (0..energy.len())
            .map(|i| {
                let lo = i.saturating_sub(5);
                let hi = (i + 5).min(energy.len());
                energy[lo..hi].iter().sum::<f64>() / 10.0
            })
            .collect();
        let max = (self.cfg.max_single_pass_seconds * rate as f32) as usize;
        let mut pieces = Vec::new();
        let mut start = 0usize;
        while wav.len() - start > max {
            let lo = (start + ((target - search / 2.0) * rate as f32) as usize) / hop;
            let hi = ((start + ((target + search / 2.0) * rate as f32) as usize) / hop)
                .min(smooth.len());
            let quietest = (lo..hi)
                .min_by(|&x, &y| smooth[x].total_cmp(&smooth[y]))
                .unwrap_or(lo);
            let cut = (quietest * hop).clamp(start + 1, wav.len());
            pieces.push(&wav[start..cut]);
            start = cut;
        }
        pieces.push(&wav[start..]);
        pieces
    }
}

struct Step {
    token: usize,
    frame: usize,
    duration: usize,
}

fn first(outputs: Vec<super::onnx::Output>) -> Result<super::onnx::Output> {
    outputs
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::Transcription("Express model: no output".into()))
}

/// The first index of the largest value (numpy's argmax).
fn argmax(values: &[f32]) -> usize {
    let mut best = 0;
    for (i, &v) in values.iter().enumerate() {
        if v > values[best] {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_takes_the_first_of_equals() {
        assert_eq!(argmax(&[0.1, 0.5, 0.5, 0.2]), 1);
        assert_eq!(argmax(&[3.0]), 0);
    }

    /// Real decode against the reference. Run with
    /// `ORTTAAI_FLUID_DIR=<fluid-speech-mini-onnx> ORTTAAI_FLUID_WAVS=<a.wav,b.wav> ORTTAAI_FLUID_EXPECT=<a text|b text>
    /// cargo test -p orttaai-core --features express -- --ignored fluid_matches`
    #[test]
    #[ignore]
    fn fluid_matches_the_reference() {
        let dir = std::env::var("ORTTAAI_FLUID_DIR").expect("ORTTAAI_FLUID_DIR");
        let wavs = std::env::var("ORTTAAI_FLUID_WAVS").expect("ORTTAAI_FLUID_WAVS");
        let expect = std::env::var("ORTTAAI_FLUID_EXPECT").expect("ORTTAAI_FLUID_EXPECT");
        let model = FluidModel::load(Path::new(&dir), 4).unwrap();
        for (wav, want) in wavs.split(',').zip(expect.split('|')) {
            let mut reader = hound::WavReader::open(wav).unwrap();
            assert_eq!(reader.spec().sample_rate, 16_000);
            let samples: Vec<f32> = match reader.spec().sample_format {
                hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
                hound::SampleFormat::Int => reader
                    .samples::<i16>()
                    .map(|s| s.unwrap() as f32 / 32768.0)
                    .collect(),
            };
            let started = std::time::Instant::now();
            let out = model.transcribe(&samples).unwrap();
            eprintln!("{:?} {}", started.elapsed(), out.text);
            assert_eq!(out.text, want);
            assert_eq!(out.tokens.len(), out.starts.len());
            assert!(out.starts.windows(2).all(|w| w[0] <= w[1]));
            assert!(out
                .starts
                .last()
                .is_some_and(|&s| s <= samples.len() as f32 / 16_000.0));
        }
    }
}
