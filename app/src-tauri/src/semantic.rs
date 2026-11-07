//! Semantic memory / Memory Graph commands. (Semantic area.)
//!
//! A background worker indexes queued dictations (enqueued from
//! `persist::after_save` when `semantic_enabled && semantic_auto_index`) off
//! the UI thread and broadcasts `semantic-progress` / `semantic-updated`. A
//! second worker reads working sessions with a local model for the Activity
//! tab (`semantic-activity`). Everything talks to a local LLM server through
//! the shared core `LlmClient` (local endpoints only).

use crate::err;
use orttaai_core::llm::LlmClient;
use orttaai_core::semantic::activity::{self, ActivityReport};
use orttaai_core::semantic::concept_graph::{ConceptGraph, ConceptTimeRange};
use orttaai_core::semantic::findings::RankedFinding;
use orttaai_core::semantic::index::{
    self, IndexConfig, IndexProgress, IndexRunResult, IndexStatus, LlmStatus,
};
use orttaai_core::semantic::report::{InsightReport, OverlayConfig};
use orttaai_core::semantic::service::{self, ConceptDetail, SignalsOverview, StoredReport};
use orttaai_core::semantic::util::Clock;
use orttaai_core::semantic::{self, SemanticHit, SemanticOptions};
use orttaai_core::settings::Settings;
use orttaai_core::store::{now_unix, InsightFinding, Store};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener};

// ---- Local LLM client ---------------------------------------------------------

/// The shared local LLM client (Ollama / LM Studio; non-local endpoints and
/// cloud models are refused inside the client).
fn client(settings: &Settings) -> std::sync::Arc<dyn LlmClient> {
    orttaai_core::llm::client_from_settings(settings)
}

// ---- Shared state --------------------------------------------------------------

static APP: OnceLock<AppHandle> = OnceLock::new();
static DICTATING: AtomicBool = AtomicBool::new(false);
static INDEX_RUNNING: AtomicBool = AtomicBool::new(false);
static READ_STOP: AtomicBool = AtomicBool::new(false);
static READ_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Shared {
    progress: Option<IndexProgress>,
    last_run: Option<IndexRunResult>,
    activity: ActivityState,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared::default()))
}

fn emit<T: Serialize + Clone>(event: &str, payload: T) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

/// Remember the app handle (for events) and start watching dictation state.
fn attach(app: &AppHandle) {
    if APP.set(app.clone()).is_ok() {
        app.listen_any("engine-state", |e| {
            let state: String = serde_json::from_str(e.payload()).unwrap_or_default();
            DICTATING.store(
                matches!(state.as_str(), "recording" | "processing" | "injecting"),
                Ordering::SeqCst,
            );
        });
        // Drain any backlog left from an earlier run.
        let settings = Settings::load_or_default();
        if settings.semantic_enabled && settings.semantic_auto_index {
            kick(Job::Index);
        }
    }
}

fn open_store() -> std::result::Result<Store, String> {
    Store::open_default().map_err(err)
}

fn clock_for(store: &Store, tz_offset: Option<i64>) -> Clock {
    if let Some(m) = tz_offset {
        let _ = index::remember_tz(store, m);
        Clock::new(now_unix(), m)
    } else {
        index::clock(store, now_unix())
    }
}

fn wait_while_dictating() {
    // Dictation (and polish on the same local model) always goes first.
    let mut waited = 0;
    while DICTATING.load(Ordering::SeqCst) && waited < 600 {
        std::thread::sleep(Duration::from_millis(500));
        waited += 1;
    }
}

// ---- Index worker --------------------------------------------------------------

enum Job {
    Index,
    Rebuild,
}

fn sender() -> &'static Mutex<Sender<Job>> {
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("semantic-index".into())
            .spawn(move || worker_loop(rx))
            .expect("spawn semantic worker");
        Mutex::new(tx)
    })
}

fn kick(job: Job) {
    if let Ok(tx) = sender().lock() {
        let _ = tx.send(job);
    }
}

fn worker_loop(rx: Receiver<Job>) {
    while let Ok(first) = rx.recv() {
        // Coalesce a burst of saves into one pass; a rebuild wins.
        let mut rebuild = matches!(first, Job::Rebuild);
        while let Ok(next) = rx.try_recv() {
            rebuild |= matches!(next, Job::Rebuild);
        }
        INDEX_RUNNING.store(true, Ordering::SeqCst);
        let outcome = run_index(rebuild);
        INDEX_RUNNING.store(false, Ordering::SeqCst);
        if let Err(e) = outcome {
            let result = IndexRunResult {
                provider_name: "Semantic Memory".into(),
                error_message: Some(e.clone()),
                ..Default::default()
            };
            if let Ok(mut s) = shared().lock() {
                s.progress = None;
                s.last_run = Some(result.clone());
            }
            emit("semantic-updated", result);
        }
    }
}

fn run_index(rebuild: bool) -> std::result::Result<(), String> {
    let settings = Settings::load_or_default();
    let cfg = IndexConfig::from_settings(&settings);
    if !cfg.enabled {
        return Ok(());
    }
    let store = open_store()?;
    let llm = client(&settings);
    let clock = index::clock(&store, now_unix());
    let mut on_progress = |p: IndexProgress| {
        if let Ok(mut s) = shared().lock() {
            s.progress = Some(p.clone());
        }
        emit("semantic-progress", p);
    };
    if rebuild {
        on_progress(IndexProgress {
            phase: "graph".into(),
            done: 0,
            total: 0,
            message: "Re-reading your dictations for the concept map…".into(),
        });
        store
            .semantic_clear_concept_cache()
            .and_then(|_| {
                service::concept_corpus(&store, clock.now, &mut |done, total| {
                    emit(
                        "semantic-graph-progress",
                        serde_json::json!({"done": done, "total": total}),
                    );
                })
            })
            .map_err(err)?;
    }
    let mut last: IndexRunResult;
    let mut total = IndexRunResult::default();
    loop {
        wait_while_dictating();
        let r = index::process_queue(&store, llm.as_ref(), &cfg, clock, 200, &mut on_progress)
            .map_err(err)?;
        let progressed = r.source_count > 0;
        total.source_count += r.source_count;
        total.chunk_count += r.chunk_count;
        total.embedded_count += r.embedded_count;
        total.skipped_count += r.skipped_count;
        total.signal_count += r.signal_count;
        total.refined_count += r.refined_count;
        last = r;
        if last.remaining == 0
            || !progressed
            || (last.error_message.is_some() && !last.used_fallback)
        {
            break;
        }
    }
    if rebuild && total.source_count == 0 {
        // Nothing new to index: still refresh the stored graph and findings.
        let active = index::active_model(&store, &cfg).map_err(err)?;
        let g =
            orttaai_core::semantic::memory_graph::rebuild(&store, &active, clock).map_err(err)?;
        last.graph_node_count = g.nodes.len();
        last.graph_edge_count = g.edges.len();
        last.finding_count = orttaai_core::semantic::findings::compute_and_store(&store, clock)
            .map_err(err)?
            .len();
    }
    let result = IndexRunResult {
        source_count: total.source_count,
        chunk_count: total.chunk_count,
        embedded_count: total.embedded_count,
        skipped_count: total.skipped_count,
        signal_count: total.signal_count,
        refined_count: total.refined_count,
        ..last
    };
    if let Ok(mut s) = shared().lock() {
        s.progress = None;
        s.last_run = Some(result.clone());
    }
    emit("semantic-updated", result);
    Ok(())
}

/// Post-save hook (`persist::after_save`): queue the dictation and wake the worker.
pub fn after_save(store: &Store, id: i64) {
    let settings = Settings::load_or_default();
    let cfg = IndexConfig::from_settings(&settings);
    match index::enqueue_after_save(store, &cfg, id) {
        Ok(true) if cfg.auto_index => kick(Job::Index),
        Ok(_) => {}
        Err(e) => eprintln!("orttaai: semantic enqueue failed: {e}"),
    }
}

/// React to settings changes: turning semantic memory on (or changing the
/// embedding model) indexes the history; turning it off stops reading.
pub fn on_settings_changed(app: &AppHandle, old: &Settings, new: &Settings) {
    attach(app);
    if !new.semantic_enabled {
        READ_STOP.store(true, Ordering::SeqCst);
        return;
    }
    let turned_on = !old.semantic_enabled && new.semantic_enabled;
    let model_changed = old.semantic_embedding_model.trim() != new.semantic_embedding_model.trim();
    let auto_on = !old.semantic_auto_index && new.semantic_auto_index;
    if turned_on || model_changed || auto_on || old.llm_provider != new.llm_provider {
        if let Ok(store) = Store::open_default() {
            let _ = index::index_now(&store, &IndexConfig::from_settings(new));
        }
        if new.semantic_auto_index || turned_on || model_changed {
            kick(Job::Index);
        }
    }
}

// ---- Activity reading ------------------------------------------------------------

/// Mac `WorkActivityStore.AnalysisState`.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct ActivityState {
    /// `idle | checking | running | paused | finished | unavailable`.
    pub state: String,
    pub done: usize,
    pub total: usize,
    pub model: Option<String>,
    pub message: Option<String>,
}

fn set_activity(state: ActivityState) {
    if let Ok(mut s) = shared().lock() {
        s.activity = state.clone();
    }
    emit("semantic-activity", state);
}

fn read_sessions(horizon_days: Option<i64>) {
    let settings = Settings::load_or_default();
    let llm = client(&settings);
    let provider = llm.provider_name().to_string();
    set_activity(ActivityState {
        state: "checking".into(),
        ..Default::default()
    });
    let unavailable = |message: String| {
        set_activity(ActivityState {
            state: "unavailable".into(),
            message: Some(message),
            ..Default::default()
        })
    };
    let model =
        match index::pick_installed(llm.as_ref(), &service::local_model_candidates(&settings)) {
            Err(_) => {
                return unavailable(format!(
                    "{provider} isn't running. Open it to read sessions with local AI."
                ))
            }
            Ok(None) => {
                return unavailable(format!(
                "No suitable model is installed in {provider}. Install one in Models › AI Features."
            ))
            }
            Ok(Some(m)) => m,
        };
    let Ok(store) = Store::open_default() else {
        return unavailable("The history database is unavailable.".into());
    };
    let clock = index::clock(&store, now_unix());
    let pending = match service::unread_sessions(&store, horizon_days, clock) {
        Ok(p) => p,
        Err(e) => return unavailable(format!("Couldn't read your sessions: {e}")),
    };
    let total = pending.len();
    let mut failures = 0;
    for (i, session) in pending.iter().enumerate() {
        if READ_STOP.load(Ordering::SeqCst) {
            set_activity(ActivityState {
                state: "idle".into(),
                ..Default::default()
            });
            return;
        }
        while DICTATING.load(Ordering::SeqCst) && !READ_STOP.load(Ordering::SeqCst) {
            set_activity(ActivityState {
                state: "paused".into(),
                done: i,
                total,
                model: Some(model.clone()),
                message: None,
            });
            std::thread::sleep(Duration::from_secs(1));
        }
        set_activity(ActivityState {
            state: "running".into(),
            done: i,
            total,
            model: Some(model.clone()),
            message: None,
        });
        match activity::analyze_with_model(llm.as_ref(), &model, session) {
            Ok(Some(a)) => {
                failures = 0;
                let _ = service::save_session_reading(&store, session, &a);
            }
            Ok(None) => failures = 0,
            Err(_) => {
                failures += 1;
                if failures >= 3 {
                    emit("semantic-updated", serde_json::json!({"activity": true}));
                    return unavailable(format!(
                        "{provider} stopped responding. Reading will resume next time."
                    ));
                }
            }
        }
        if (i + 1) % 6 == 0 {
            emit("semantic-activity-revision", i + 1);
        }
    }
    emit("semantic-activity-revision", total);
    set_activity(ActivityState {
        state: "finished".into(),
        done: total,
        total,
        model: Some(model),
        message: None,
    });
}

// ---- Commands --------------------------------------------------------------------

/// Everything the status strip needs.
#[derive(Serialize)]
pub struct SemanticStatus {
    pub index: IndexStatus,
    pub running: bool,
    pub progress: Option<IndexProgress>,
    pub last_run: Option<IndexRunResult>,
    pub activity: ActivityState,
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> std::result::Result<T, String> + Send + 'static,
) -> std::result::Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

#[tauri::command]
pub async fn semantic_status(
    app: AppHandle,
    tz_offset: Option<i64>,
) -> Result<SemanticStatus, String> {
    attach(&app);
    blocking(move || {
        let store = open_store()?;
        let _ = clock_for(&store, tz_offset);
        let cfg = IndexConfig::from_settings(&Settings::load_or_default());
        let status = index::status(&store, &cfg).map_err(err)?;
        let s = shared().lock().map_err(err)?;
        Ok(SemanticStatus {
            index: status,
            running: INDEX_RUNNING.load(Ordering::SeqCst),
            progress: s.progress.clone(),
            last_run: s.last_run.clone(),
            activity: s.activity.clone(),
        })
    })
    .await
}

#[tauri::command]
pub async fn semantic_llm_status(app: AppHandle) -> Result<LlmStatus, String> {
    attach(&app);
    blocking(|| {
        let settings = Settings::load_or_default();
        Ok(index::llm_status(
            client(&settings).as_ref(),
            &IndexConfig::from_settings(&settings),
        ))
    })
    .await
}

#[tauri::command]
pub async fn semantic_search(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SemanticHit>, String> {
    attach(&app);
    blocking(move || {
        let settings = Settings::load_or_default();
        let store = open_store()?;
        semantic::search(
            &store,
            client(&settings).as_ref(),
            &SemanticOptions::from_settings(&settings),
            &query,
            limit.unwrap_or(8).clamp(1, 50),
        )
        .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_index_now(app: AppHandle) -> Result<i64, String> {
    attach(&app);
    let pending = blocking(|| {
        let store = open_store()?;
        index::index_now(
            &store,
            &IndexConfig::from_settings(&Settings::load_or_default()),
        )
        .map_err(err)
    })
    .await?;
    kick(Job::Index);
    Ok(pending)
}

#[tauri::command]
pub async fn semantic_clear_index(app: AppHandle) -> Result<(), String> {
    attach(&app);
    READ_STOP.store(true, Ordering::SeqCst);
    blocking(|| {
        let store = open_store()?;
        index::clear(&store).map_err(err)
    })
    .await?;
    if let Ok(mut s) = shared().lock() {
        s.last_run = None;
        s.progress = None;
    }
    emit("semantic-updated", IndexRunResult::default());
    Ok(())
}

#[tauri::command]
pub fn semantic_rebuild_graph(app: AppHandle) {
    attach(&app);
    kick(Job::Rebuild);
}

#[tauri::command]
pub async fn semantic_concept_graph(
    app: AppHandle,
    range: String,
    kind: Option<String>,
    query: Option<String>,
    tz_offset: Option<i64>,
) -> Result<ConceptGraph, String> {
    attach(&app);
    blocking(move || {
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        service::concept_graph(
            &store,
            ConceptTimeRange::parse(&range),
            kind.as_deref().unwrap_or("all"),
            query.as_deref().unwrap_or(""),
            clock,
            &mut |done, total| {
                if total > 40 {
                    emit(
                        "semantic-graph-progress",
                        serde_json::json!({"done": done, "total": total}),
                    );
                }
            },
        )
        .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_concept_detail(
    range: String,
    id: String,
    tz_offset: Option<i64>,
) -> Result<Option<ConceptDetail>, String> {
    blocking(move || {
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        service::concept_detail(&store, ConceptTimeRange::parse(&range), &id, clock).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_concept_brief(
    range: String,
    id: String,
    tz_offset: Option<i64>,
) -> Result<String, String> {
    blocking(move || {
        let settings = Settings::load_or_default();
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        service::concept_brief(
            &store,
            client(&settings).as_ref(),
            &service::local_model_candidates(&settings),
            ConceptTimeRange::parse(&range),
            &id,
            clock,
        )
        .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_activity(
    app: AppHandle,
    range: String,
    tz_offset: Option<i64>,
) -> Result<ActivityReport, String> {
    attach(&app);
    blocking(move || {
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        service::activity_report(&store, ConceptTimeRange::parse(&range), clock).map_err(err)
    })
    .await
}

/// Read unread sessions with local AI, newest first; `range` sets the
/// horizon (twice the range, so period-over-period changes compare readings).
#[tauri::command]
pub fn semantic_activity_start(app: AppHandle, range: String) -> ActivityState {
    attach(&app);
    if READ_RUNNING.swap(true, Ordering::SeqCst) {
        return shared()
            .lock()
            .map(|s| s.activity.clone())
            .unwrap_or_default();
    }
    READ_STOP.store(false, Ordering::SeqCst);
    let horizon = ConceptTimeRange::parse(&range).days().map(|d| d * 2);
    let _ = std::thread::Builder::new()
        .name("semantic-sessions".into())
        .spawn(move || {
            read_sessions(horizon);
            READ_RUNNING.store(false, Ordering::SeqCst);
        });
    ActivityState {
        state: "checking".into(),
        ..Default::default()
    }
}

#[tauri::command]
pub fn semantic_activity_stop() -> ActivityState {
    READ_STOP.store(true, Ordering::SeqCst);
    let state = ActivityState {
        state: "idle".into(),
        ..Default::default()
    };
    set_activity(state.clone());
    state
}

#[tauri::command]
pub async fn semantic_insight_report() -> Result<Option<StoredReport>, String> {
    blocking(|| {
        let store = open_store()?;
        service::latest_report(&store).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_generate_insights(tz_offset: Option<i64>) -> Result<InsightReport, String> {
    blocking(move || {
        let settings = Settings::load_or_default();
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        let overlay = OverlayConfig {
            enabled: settings.semantic_summary_enabled,
            model: settings.semantic_summary_model.clone(),
            context_tokens: settings.insights_context_tokens,
            thinking: settings.insights_thinking,
        };
        let report = service::generate_report(
            &store,
            client(&settings).as_ref(),
            &IndexConfig::from_settings(&settings),
            &overlay,
            clock,
            8,
        )
        .map_err(err)?;
        emit("semantic-updated", serde_json::json!({"insights": true}));
        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn semantic_findings(
    kinds: Option<Vec<String>>,
    status: Option<String>,
) -> Result<Vec<InsightFinding>, String> {
    blocking(move || {
        let store = open_store()?;
        let status = status.unwrap_or_else(|| "active".into());
        store
            .semantic_findings(kinds.as_deref(), &[status.as_str()], 200)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn semantic_open_loops() -> Result<Vec<InsightFinding>, String> {
    blocking(|| {
        let store = open_store()?;
        service::open_loops(&store, 12).map_err(err)
    })
    .await
}

/// Resolve or dismiss a finding (never resurrected by recomputation).
#[tauri::command]
pub async fn semantic_set_finding_status(id: i64, status: String) -> Result<bool, String> {
    if !matches!(status.as_str(), "resolved" | "dismissed" | "active") {
        return Err(format!("unknown finding status: {status}"));
    }
    let changed = blocking(move || {
        let store = open_store()?;
        store.semantic_set_finding_status(id, &status).map_err(err)
    })
    .await?;
    emit("semantic-updated", serde_json::json!({"findings": true}));
    Ok(changed)
}

#[tauri::command]
pub async fn semantic_signals(since_days: Option<i64>) -> Result<SignalsOverview, String> {
    blocking(move || {
        let store = open_store()?;
        let since = since_days
            .filter(|d| *d > 0)
            .map(|d| now_unix() - d * 86_400);
        service::signals_overview(&store, since, 12).map_err(err)
    })
    .await
}

/// One number the Home ticker cycles through (Mac `InsightStat`).
#[derive(Serialize, Deserialize, Clone)]
pub struct InsightStat {
    pub id: String,
    pub icon: String,
    pub number: Option<f64>,
    pub text: Option<String>,
    pub label: String,
}

#[derive(Serialize)]
pub struct HomeInsights {
    pub enabled: bool,
    pub stats: Vec<InsightStat>,
    pub findings: Vec<RankedFinding>,
    pub summary: Vec<String>,
    pub report_generated_at: Option<i64>,
    pub report_model: Option<String>,
    pub stale: bool,
    pub index: IndexStatus,
}

/// The Home insights panel: ticker stats, ranked findings, the report summary.
#[tauri::command]
pub async fn semantic_home_insights(
    app: AppHandle,
    tz_offset: Option<i64>,
) -> Result<HomeInsights, String> {
    attach(&app);
    blocking(move || {
        let settings = Settings::load_or_default();
        let cfg = IndexConfig::from_settings(&settings);
        let store = open_store()?;
        let clock = clock_for(&store, tz_offset);
        let week = store
            .semantic_transcriptions_between(clock.now - 7 * 86_400, clock.now + 60)
            .map_err(err)?;
        let words: i64 = week
            .iter()
            .map(|r| r.text.split_whitespace().count() as i64)
            .sum();
        let (speech_words, speech_ms) =
            week.iter()
                .filter(|r| r.duration_ms > 2000)
                .fold((0i64, 0i64), |(w, ms), r| {
                    (
                        w + r.text.split_whitespace().count() as i64,
                        ms + r.duration_ms,
                    )
                });
        let mut stats = vec![
            InsightStat {
                id: "dictations".into(),
                icon: "mic".into(),
                number: Some(week.len() as f64),
                text: None,
                label: "dictations this week".into(),
            },
            InsightStat {
                id: "words".into(),
                icon: "edit".into(),
                number: Some(words as f64),
                text: None,
                label: "words this week".into(),
            },
        ];
        if speech_ms > 0 {
            stats.push(InsightStat {
                id: "wpm".into(),
                icon: "bolt".into(),
                number: Some((speech_words as f64 / (speech_ms as f64 / 60_000.0)).round()),
                text: None,
                label: "words per minute".into(),
            });
        }
        let mut findings = Vec::new();
        let mut summary = Vec::new();
        let mut generated = None;
        let mut model = None;
        let mut stale = false;
        if cfg.enabled {
            let loops = service::open_loops(&store, 50).map_err(err)?;
            stats.push(InsightStat {
                id: "open-loops".into(),
                icon: "target".into(),
                number: Some(loops.len() as f64),
                text: None,
                label: "open loops".into(),
            });
            findings = service::home_findings(&store, clock.now, 6).map_err(err)?;
            if let Some(top) = findings.iter().find(|f| f.finding.kind == "rhythm") {
                stats.push(InsightStat {
                    id: "rhythm".into(),
                    icon: "flame".into(),
                    number: None,
                    text: Some(top.finding.title.replace("Your voice peaks from ", "")),
                    label: "peak dictation hours".into(),
                });
            }
            if let Some(stored) = service::latest_report(&store).map_err(err)? {
                summary = stored.report.summary.clone();
                generated = Some(stored.report.generated_at);
                model = stored.report.summary_model_name.clone();
                stale = stored.stale;
            }
        }
        stats.truncate(6);
        Ok(HomeInsights {
            enabled: cfg.enabled,
            stats,
            findings,
            summary,
            report_generated_at: generated,
            report_model: model,
            stale,
            index: index::status(&store, &cfg).map_err(err)?,
        })
    })
    .await
}
