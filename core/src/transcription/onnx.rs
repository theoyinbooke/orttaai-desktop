//! A small ONNX Runtime session on the runtime sherpa-onnx already links into
//! the app (through `ort-sys` bindings, as Word Boost does), for models that
//! aren't in sherpa-onnx's format: Fluid Speech Mini's frontend, encoder,
//! decoder and joint network.

use crate::error::{CoreError, Result};
use ort_sys as ort;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

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
            .ok_or_else(|| CoreError::Transcription("ONNX Runtime is too old for Express".into()))
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
        "Express model: {message}"
    )))
}

/// How a session runs.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Intra-op threads; 0 leaves ONNX Runtime's default.
    pub threads: i32,
    /// Worker threads wait without spinning (small models run per token).
    pub spin: bool,
}

/// An input tensor borrowed for one run.
pub enum Input<'a> {
    F32(&'a [f32], &'a [i64]),
    I64(&'a [i64], &'a [i64]),
}

/// A float output, copied out of ONNX Runtime.
#[derive(Debug, Clone)]
pub struct Output {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// A loaded model. Runs are thread-safe in ONNX Runtime.
pub struct Session {
    api: &'static ort::OrtApi,
    env: *mut ort::OrtEnv,
    session: *mut ort::OrtSession,
    memory: *mut ort::OrtMemoryInfo,
}

// SAFETY: OrtSession::Run may be called concurrently; the other handles are
// only read after construction and released in Drop.
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Session {
    pub fn load(model: &Path, config: SessionConfig) -> Result<Self> {
        let api = api()?;
        let mut env = ptr::null_mut();
        let mut options = ptr::null_mut();
        let mut session = ptr::null_mut();
        let mut memory = ptr::null_mut();
        let id = CString::new("orttaai-express").expect("no NUL");
        #[cfg(windows)]
        let path: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            model.as_os_str().encode_wide().chain(Some(0)).collect()
        };
        #[cfg(not(windows))]
        let path = CString::new(model.to_string_lossy().as_bytes())
            .map_err(|_| CoreError::Transcription("bad model path".into()))?;
        // SAFETY: every out-pointer is checked through `check`; whatever was
        // built is released by Drop on error.
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
                if config.threads > 0 {
                    check(api, (api.SetIntraOpNumThreads)(options, config.threads))?;
                }
                // Activations are freed between runs rather than kept in an
                // arena: the encoder's would hold hundreds of MB while idle.
                check(api, (api.DisableCpuMemArena)(options))?;
                check(api, (api.DisableMemPattern)(options))?;
                if !config.spin {
                    check(
                        api,
                        (api.AddSessionConfigEntry)(
                            options,
                            c"session.intra_op.allow_spinning".as_ptr(),
                            c"0".as_ptr(),
                        ),
                    )?;
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
        let loaded = Self {
            api,
            env,
            session,
            memory,
        };
        result.map(|()| loaded)
    }

    /// Run with named inputs; returns the named float outputs in order.
    pub fn run(&self, inputs: &[(&CStr, Input<'_>)], outputs: &[&CStr]) -> Result<Vec<Output>> {
        let api = self.api;
        let names_in: Vec<*const std::ffi::c_char> =
            inputs.iter().map(|(n, _)| n.as_ptr()).collect();
        let names_out: Vec<*const std::ffi::c_char> = outputs.iter().map(|n| n.as_ptr()).collect();
        let mut values: Vec<*mut ort::OrtValue> = vec![ptr::null_mut(); inputs.len()];
        let mut results: Vec<*mut ort::OrtValue> = vec![ptr::null_mut(); outputs.len()];
        // SAFETY: input tensors borrow caller slices that outlive the run (ONNX
        // Runtime never writes inputs); every OrtValue is released below.
        unsafe {
            let result = (|| -> Result<Vec<Output>> {
                for ((_, input), value) in inputs.iter().zip(values.iter_mut()) {
                    let (data, bytes, shape, kind) = match input {
                        Input::F32(d, s) => (
                            d.as_ptr() as *mut std::ffi::c_void,
                            std::mem::size_of_val(*d),
                            *s,
                            ort::ONNXTensorElementDataType::ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT,
                        ),
                        Input::I64(d, s) => (
                            d.as_ptr() as *mut std::ffi::c_void,
                            std::mem::size_of_val(*d),
                            *s,
                            ort::ONNXTensorElementDataType::ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64,
                        ),
                    };
                    check(
                        api,
                        (api.CreateTensorWithDataAsOrtValue)(
                            self.memory,
                            data,
                            bytes,
                            shape.as_ptr(),
                            shape.len(),
                            kind,
                            value,
                        ),
                    )?;
                }
                check(
                    api,
                    (api.Run)(
                        self.session,
                        ptr::null(),
                        names_in.as_ptr(),
                        values.as_ptr() as *const *const ort::OrtValue,
                        values.len(),
                        names_out.as_ptr(),
                        names_out.len(),
                        results.as_mut_ptr(),
                    ),
                )?;
                results.iter().map(|&value| read_f32(api, value)).collect()
            })();
            for v in values.iter().chain(results.iter()) {
                if !v.is_null() {
                    (api.ReleaseValue)(*v);
                }
            }
            result
        }
    }
}

/// Copy a float tensor's shape and data.
///
/// # Safety
/// `value` must be a live tensor OrtValue.
unsafe fn read_f32(api: &ort::OrtApi, value: *mut ort::OrtValue) -> Result<Output> {
    let mut info = ptr::null_mut();
    check(api, (api.GetTensorTypeAndShape)(value, &mut info))?;
    let mut count = 0usize;
    let mut rank = 0usize;
    let mut dims = Vec::new();
    let shape = (|| -> Result<()> {
        check(api, (api.GetTensorShapeElementCount)(info, &mut count))?;
        check(api, (api.GetDimensionsCount)(info, &mut rank))?;
        dims = vec![0i64; rank];
        check(api, (api.GetDimensions)(info, dims.as_mut_ptr(), rank))
    })();
    (api.ReleaseTensorTypeAndShapeInfo)(info);
    shape?;
    let mut data: *mut std::ffi::c_void = ptr::null_mut();
    check(api, (api.GetTensorMutableData)(value, &mut data))?;
    let data = if count == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data as *const f32, count).to_vec()
    };
    Ok(Output {
        shape: dims.into_iter().map(|d| d.max(0) as usize).collect(),
        data,
    })
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: each handle was created by this session and is released once.
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
