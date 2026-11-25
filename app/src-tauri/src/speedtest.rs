//! The speed test (Settings › Model › Performance, or the card that offers it
//! once after setup or an update): times the speech model this computer
//! dictates with at a few thread counts and sets Decode Threads to the fastest
//! (core `speedtest`). It runs in the background with its own copy of the
//! model, so dictation keeps working and the window can close meanwhile.
//!
//! Events: `speed-test-progress` `{threads, index, total, ms?}`, then
//! `speed-test-done` `{result?, error?}`.

use crate::err;
use orttaai_core::cpu;
use orttaai_core::settings::Settings;
use orttaai_core::speedtest::{self, SpeedTestRecord};
use orttaai_core::transcription::{
    decode_options_from_settings, express, ExpressTranscriber, Transcriber, WhisperTranscriber,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static RUNNING: AtomicBool = AtomicBool::new(false);
/// The trial running now, for a window opened mid-test.
static PROGRESS: Mutex<Option<serde_json::Value>> = Mutex::new(None);

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

#[derive(Debug, Clone, Serialize)]
pub struct SpeedTestStatus {
    pub running: bool,
    pub progress: Option<serde_json::Value>,
    /// Show the card that offers the test.
    pub offer: bool,
    pub last: Option<SpeedTestRecord>,
}

/// Is the model the test would time downloaded?
fn model_ready(settings: &Settings) -> bool {
    if settings.express_active() {
        express::is_installed(settings.express_model.trim())
    } else {
        crate::engine::resolve_model(settings, None).is_ok()
    }
}

#[tauri::command]
pub fn speed_test_status() -> SpeedTestStatus {
    let settings = Settings::load_or_default();
    let running = RUNNING.load(Ordering::SeqCst);
    SpeedTestStatus {
        running,
        progress: if running {
            PROGRESS.lock().ok().and_then(|p| p.clone())
        } else {
            None
        },
        offer: !running
            && speedtest::should_offer(
                &settings,
                model_ready(&settings),
                cpu::core_counts().map(|c| c.performance),
            ),
        last: settings.last_speed_test,
    }
}

/// "Not now" on the offer card: it doesn't come back (the test stays in
/// Settings › Model › Performance).
#[tauri::command]
pub fn speed_test_offer_dismiss(app: AppHandle) -> Result<(), String> {
    crate::settings_cmds::update_settings(
        app,
        serde_json::json!({ "speed_test_offer_dismissed": true }),
    )?;
    Ok(())
}

/// Start the speed test in the background. When it finishes, Decode Threads
/// is the fastest count and `speed-test-done` says so.
#[tauri::command]
pub fn start_speed_test(app: AppHandle) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("The speed test is already running.".into());
    }
    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let measured = tauri::async_runtime::spawn_blocking(move || measure(&progress_app))
            .await
            .map_err(err)
            .and_then(|r| r);
        let outcome = measured.and_then(|result| {
            save(&app, &result)?;
            Ok(result)
        });
        if let Ok(mut p) = PROGRESS.lock() {
            *p = None;
        }
        RUNNING.store(false, Ordering::SeqCst);
        let payload = match &outcome {
            Ok(result) => serde_json::json!({ "result": result }),
            Err(e) => {
                tracing::warn!("Speed test failed: {e}");
                serde_json::json!({ "error": e })
            }
        };
        let _ = app.emit("speed-test-done", payload);
    });
    Ok(())
}

fn save(app: &AppHandle, result: &SpeedTestResult) -> Result<(), String> {
    let ms = |threads: usize| {
        result
            .trials
            .iter()
            .find(|t| t.threads == threads)
            .map(|t| t.ms)
    };
    let record = SpeedTestRecord {
        model: result.model.clone(),
        best: result.best,
        best_ms: ms(result.best).unwrap_or(0),
        recommended: result.recommended,
        recommended_ms: ms(result.recommended),
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
    };
    crate::settings_cmds::update_settings(
        app.clone(),
        serde_json::json!({ "decode": { "n_threads": result.best }, "last_speed_test": record }),
    )?;
    Ok(())
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
        let progress =
            serde_json::json!({ "threads": threads, "index": index, "total": total, "ms": ms });
        if let Ok(mut p) = PROGRESS.lock() {
            *p = Some(progress.clone());
        }
        let _ = app.emit("speed-test-progress", progress);
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
        // Timed as dictation runs: with Word Boost, which shares the cores.
        let boost = crate::engine::word_boost_terms(&settings);
        for (index, &threads) in candidates.iter().enumerate() {
            report(index, threads, None);
            let t = ExpressTranscriber::from_dir(&dir, threads as i32).map_err(err)?;
            t.set_word_boost_terms(&boost);
            let took = speedtest::time_decode(
                |s| {
                    // Word Boost keeps a dictation's scores; each trial
                    // scores afresh, as a new dictation would.
                    t.discard_prepared_word_boost();
                    t.transcribe(s, &opts).map(|_| ())
                },
                &samples,
            )
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
