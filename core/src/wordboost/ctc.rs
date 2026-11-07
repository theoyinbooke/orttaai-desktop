//! The Word Boost model: Parakeet CTC 110M (`nvidia/parakeet-tdt_ctc-110m`,
//! CTC branch, int8, as exported by sherpa-onnx) run on the ONNX Runtime that
//! sherpa-onnx already links into the app (through `ort-sys` bindings, so no
//! second runtime). Input: NeMo log-mel features `[1, 80, T]` and their length;
//! output: per-frame log-probabilities `[1, T/8, 1025]`, blank last.

use super::features::{Featurizer, N_MELS};
use crate::error::{CoreError, Result};
use ort_sys as ort;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

/// Seconds per output frame: 10 ms features, subsampled 8 times.
pub const FRAME_SECONDS: f64 = 0.08;
/// The blank symbol, after the 1024 SentencePiece tokens.
pub const BLANK_ID: usize = 1024;
pub const CLASSES: usize = 1025;

fn api() -> Result<&'static ort::OrtApi> {
    // SAFETY: OrtGetApiBase is provided by the ONNX Runtime linked through
    // sherpa-onnx; GetApi returns a static table or null for an unsupported
    // version.
    unsafe {
        let base = ort::OrtGetApiBase();
        if base.is_null() {
            return Err(CoreError::Transcription("ONNX Runtime unavailable".into()));
        }
        let api = ((*base).GetApi)(ort::ORT_API_VERSION);
        api.as_ref()
            .ok_or_else(|| CoreError::Transcription("ONNX Runtime too old for Word Boost".into()))
    }
}

/// Turn an `OrtStatus` into a `Result`, releasing it.
fn check(api: &ort::OrtApi, status: ort::OrtStatusPtr) -> Result<()> {
    if status.0.is_null() {
        return Ok(());
    }
    // SAFETY: a non-null status carries a message and must be released once.
    let message = unsafe {
        let m = CStr::from_ptr((api.GetErrorMessage)(status.0))
            .to_string_lossy()
            .into_owned();
        (api.ReleaseStatus)(status.0);
        m
    };
    Err(CoreError::Transcription(format!(
        "Word Boost model: {message}"
    )))
}

/// A loaded CTC model. Runs are thread-safe in ONNX Runtime.
pub struct CtcModel {
    api: &'static ort::OrtApi,
    env: *mut ort::OrtEnv,
    session: *mut ort::OrtSession,
    memory: *mut ort::OrtMemoryInfo,
    featurizer: Featurizer,
}

// SAFETY: OrtSession::Run may be called concurrently; the other handles are
// only read after construction and released in Drop.
unsafe impl Send for CtcModel {}
unsafe impl Sync for CtcModel {}

impl CtcModel {
    pub fn load(model: &Path, threads: i32) -> Result<Self> {
        let api = api()?;
        let mut env = ptr::null_mut();
        let mut options = ptr::null_mut();
        let mut session = ptr::null_mut();
        let mut memory = ptr::null_mut();
        let id = CString::new("orttaai-word-boost").expect("no NUL");
        #[cfg(windows)]
        let path: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            model.as_os_str().encode_wide().chain(Some(0)).collect()
        };
        #[cfg(not(windows))]
        let path = CString::new(model.to_string_lossy().as_bytes())
            .map_err(|_| CoreError::Transcription("bad model path".into()))?;
        // SAFETY: every out-pointer is checked through `check`; partial
        // construction is released below on error.
        let result = unsafe {
            (|| -> Result<()> {
                check(
                    api,
                    (api.CreateEnv)(
                        ort::OrtLoggingLevel::ORT_LOGGING_LEVEL_ERROR,
                        id.as_ptr(),
                        &mut env,
                    ),
                )?;
                check(api, (api.CreateSessionOptions)(&mut options))?;
                if threads > 0 {
                    check(api, (api.SetIntraOpNumThreads)(options, threads))?;
                }
                check(
                    api,
                    (api.CreateSession)(env, path.as_ptr() as *const _, options, &mut session),
                )?;
                check(
                    api,
                    (api.CreateCpuMemoryInfo)(
                        ort::OrtAllocatorType::OrtArenaAllocator,
                        ort::OrtMemType::OrtMemTypeDefault,
                        &mut memory,
                    ),
                )?;
                Ok(())
            })()
        };
        // SAFETY: options are copied into the session and no longer needed.
        unsafe {
            if !options.is_null() {
                (api.ReleaseSessionOptions)(options);
            }
        }
        let model = Self {
            api,
            env,
            session,
            memory,
            featurizer: Featurizer::new(),
        };
        result.map(|()| model)
    }

    /// Per-frame log-probabilities (`CLASSES` per frame) for 16 kHz audio.
    pub fn log_probs(&self, samples: &[f32]) -> Result<Vec<Vec<f32>>> {
        let mel = self.featurizer.compute(samples);
        let frames = mel.first().map_or(0, Vec::len);
        let mut input: Vec<f32> = Vec::with_capacity(N_MELS * frames);
        for row in &mel {
            input.extend_from_slice(row);
        }
        let mut length = [frames as i64];
        let input_shape = [1i64, N_MELS as i64, frames as i64];
        let length_shape = [1i64];
        let api = self.api;
        let names_in = [c"audio_signal".as_ptr(), c"length".as_ptr()];
        let names_out = [c"logprobs".as_ptr()];
        let mut values: [*mut ort::OrtValue; 2] = [ptr::null_mut(); 2];
        let mut output: *mut ort::OrtValue = ptr::null_mut();
        // SAFETY: tensors borrow `input` and `length`, which outlive the run;
        // every OrtValue is released before returning.
        unsafe {
            let result = (|| -> Result<Vec<Vec<f32>>> {
                check(
                    api,
                    (api.CreateTensorWithDataAsOrtValue)(
                        self.memory,
                        input.as_mut_ptr() as *mut _,
                        input.len() * std::mem::size_of::<f32>(),
                        input_shape.as_ptr(),
                        input_shape.len(),
                        ort::ONNXTensorElementDataType::ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT,
                        &mut values[0],
                    ),
                )?;
                check(
                    api,
                    (api.CreateTensorWithDataAsOrtValue)(
                        self.memory,
                        length.as_mut_ptr() as *mut _,
                        std::mem::size_of::<i64>(),
                        length_shape.as_ptr(),
                        length_shape.len(),
                        ort::ONNXTensorElementDataType::ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64,
                        &mut values[1],
                    ),
                )?;
                check(
                    api,
                    (api.Run)(
                        self.session,
                        ptr::null(),
                        names_in.as_ptr(),
                        values.as_ptr() as *const *const ort::OrtValue,
                        2,
                        names_out.as_ptr(),
                        1,
                        &mut output,
                    ),
                )?;
                let mut info = ptr::null_mut();
                check(api, (api.GetTensorTypeAndShape)(output, &mut info))?;
                let mut dims = [0i64; 3];
                let shape = check(api, (api.GetDimensions)(info, dims.as_mut_ptr(), 3));
                (api.ReleaseTensorTypeAndShapeInfo)(info);
                shape?;
                let (steps, classes) = (dims[1].max(0) as usize, dims[2].max(0) as usize);
                if classes != CLASSES {
                    return Err(CoreError::Transcription(format!(
                        "Word Boost model has {classes} classes, expected {CLASSES}"
                    )));
                }
                let mut data: *mut std::ffi::c_void = ptr::null_mut();
                check(api, (api.GetTensorMutableData)(output, &mut data))?;
                let flat = std::slice::from_raw_parts(data as *const f32, steps * classes);
                Ok(flat.chunks_exact(classes).map(<[f32]>::to_vec).collect())
            })();
            for v in values.iter().chain([&output]) {
                if !v.is_null() {
                    (api.ReleaseValue)(*v);
                }
            }
            result
        }
    }
}

impl Drop for CtcModel {
    fn drop(&mut self) {
        // SAFETY: each handle was created by this model and is released once.
        unsafe {
            if !self.memory.is_null() {
                (self.api.ReleaseMemoryInfo)(self.memory);
            }
            if !self.session.is_null() {
                (self.api.ReleaseSession)(self.session);
            }
            if !self.env.is_null() {
                (self.api.ReleaseEnv)(self.env);
            }
        }
    }
}

/// Greedy CTC decode (repeats collapsed, blanks dropped), for checks.
pub fn greedy_tokens(log_probs: &[Vec<f32>]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev = BLANK_ID;
    for frame in log_probs {
        let best = frame
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(BLANK_ID, |(i, _)| i);
        if best != prev && best != BLANK_ID {
            out.push(best);
        }
        prev = best;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real model check: `ORTTAAI_WORD_BOOST_DIR` holds model.int8.onnx and
    /// tokens.txt, `ORTTAAI_TEST_WAV` a 16 kHz mono clip, and
    /// `ORTTAAI_TEST_EXPECT` a word the greedy transcript must contain.
    #[test]
    #[ignore]
    fn ctc_model_transcribes_a_clip() {
        let dir = std::path::PathBuf::from(std::env::var("ORTTAAI_WORD_BOOST_DIR").unwrap());
        let wav = std::env::var("ORTTAAI_TEST_WAV").unwrap();
        let expect = std::env::var("ORTTAAI_TEST_EXPECT").unwrap_or_else(|_| "quarterly".into());
        let mut reader = hound::WavReader::open(wav).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let model = CtcModel::load(&dir.join("model.int8.onnx"), 4).unwrap();
        let lp = model.log_probs(&samples).unwrap();
        let expected_frames = super::super::features::frame_count(samples.len()).div_ceil(8);
        assert!(
            lp.len().abs_diff(expected_frames) <= 1,
            "{} frames",
            lp.len()
        );
        let vocab: Vec<String> = std::fs::read_to_string(dir.join("tokens.txt"))
            .unwrap()
            .lines()
            .map(|l| l.split(' ').next().unwrap_or("").to_string())
            .collect();
        let text: String = greedy_tokens(&lp)
            .into_iter()
            .map(|t| vocab[t].replace('▁', " "))
            .collect();
        eprintln!("greedy: {text}");
        assert!(text.to_lowercase().contains(&expect), "{text}");
    }
}
