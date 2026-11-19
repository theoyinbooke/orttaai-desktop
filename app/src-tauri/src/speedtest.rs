//! Settings › Model › Performance › Speed Test: times the speech model this
//! computer dictates with at a few thread counts and sets Decode Threads to
//! the fastest (core `speedtest`). It loads its own copy of the model, so
//! dictation keeps working while it runs.
//!
//! Events: `speed-test-progress` `{threads, index, total, ms?}`.

use crate::err;
use orttaai_core::cpu;
use orttaai_core::settings::Settings;
use orttaai_core::speedtest;
use orttaai_core::transcription::{
    decode_options_from_settings, express, ExpressTranscriber, Transcriber, WhisperTranscriber,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
pub struct Trial {
    pub threads: usize,
    pub ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeedTestResult {
    /// The model that was timed, as the UI names it.
    pub model: String,
    pub trials: Vec<Trial>,
    /// The fastest thread count, now saved as Decode Threads.
    pub best: usize,
    /// What Auto would use.
    pub recommended: usize,
    pub sample_seconds: f32,
}

/// Run the speed test and save the fastest thread count as Decode Threads.
#[tauri::command]
pub async fn run_speed_test(app: AppHandle) -> Result<SpeedTestResult, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("The speed test is already running.".into());
    }
    let progress_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || measure(&progress_app))
        .await
        .map_err(err);
    RUNNING.store(false, Ordering::SeqCst);
    let result = result??;
    crate::settings_cmds::update_settings(
        app,
        serde_json::json!({ "decode": { "n_threads": result.best } }),
    )?;
    Ok(result)
}

fn measure(app: &AppHandle) -> Result<SpeedTestResult, String> {
    let settings = Settings::load_or_default();
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let recommended = cpu::speech_threads() as usize;
    let candidates = speedtest::candidates(recommended, cpu::core_counts(), logical);
    let samples = speedtest::sample_audio(speedtest::SAMPLE_SECONDS);
    let total = candidates.len();
    let report = |index: usize, threads: usize, ms: Option<u64>| {
        let _ = app.emit(
            "speed-test-progress",
            serde_json::json!({ "threads": threads, "index": index, "total": total, "ms": ms }),
        );
    };

    let mut trials: Vec<(usize, Duration)> = Vec::new();
    let model;
    if settings.express_active() {
        // Express's ONNX sessions fix their thread count when they load, so
        // each count loads the model once.
        let id = settings.express_model.trim().to_string();
        model = express::spec(&id).map_or_else(|| id.clone(), |s| s.name.to_string());
        let dir = express::model_dir(&id).map_err(err)?;
        let opts = decode_options_from_settings(&settings);
        for (index, &threads) in candidates.iter().enumerate() {
            report(index, threads, None);
            let t = ExpressTranscriber::from_dir(&dir, threads as i32).map_err(err)?;
            let took = speedtest::time_decode(|s| t.transcribe(s, &opts).map(|_| ()), &samples)
                .map_err(err)?;
            report(index, threads, Some(took.as_millis() as u64));
            trials.push((threads, took));
        }
    } else {
        // Whisper takes its thread count per decode: one load serves all.
        let path = crate::engine::resolve_model(&settings, None)?;
        model = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Whisper".into());
        let t = WhisperTranscriber::from_path(&path).map_err(err)?;
        for (index, &threads) in candidates.iter().enumerate() {
            report(index, threads, None);
            let mut opts = decode_options_from_settings(&settings);
            opts.n_threads = threads as i32;
            let took = speedtest::time_decode(|s| t.transcribe(s, &opts).map(|_| ()), &samples)
                .map_err(err)?;
            report(index, threads, Some(took.as_millis() as u64));
            trials.push((threads, took));
        }
    }
    let best = speedtest::fastest(&trials).ok_or("The speed test didn't run.")?;
    tracing::info!("Speed test on {model}: {trials:?}; fastest {best} threads");
    Ok(SpeedTestResult {
        model,
        trials: trials
            .into_iter()
            .map(|(threads, d)| Trial {
                threads,
                ms: d.as_millis() as u64,
            })
            .collect(),
        best,
        recommended,
        sample_seconds: speedtest::SAMPLE_SECONDS,
    })
}
