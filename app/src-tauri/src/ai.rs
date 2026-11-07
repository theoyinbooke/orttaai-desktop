//! Local AI commands: provider status + model downloads, S1-mini, polish and
//! voice-edit previews, Chat AI (streamed), writing insights, tone of voice.
//! (AI area.)
//!
//! Events:
//! - `ai-pull-progress` `{model, purpose, status, completed, total, fraction, done, error}`
//! - `ai-models-changed` `()` after a download finishes
//! - `chat-delta` `{conversation_id, message_id, content, reasoning}` (accumulated text)
//! - `chat-done` `TurnResult`
//! - `chat-error` `{conversation_id, message_id, error}`
//! - `writing-insights-updated` `WritingInsightsRun`
//! - `tone-updated` `ToneRun`

use crate::err;
use orttaai_core::chat::{self, ChatGroup, ChatRequest, PreparedTurn, TurnResult};
use orttaai_core::edit;
use orttaai_core::llm::{
    self, catalog, client_for, client_from_settings, LlmClient, LlmHealth, PullProgress,
};
use orttaai_core::polish::{PolishOptions, PolishOutcome, Polisher};
use orttaai_core::settings::{LlmProvider, PolishEngine, Settings};
use orttaai_core::store::{now_unix, ChatMessageRow, Store};
use orttaai_core::writing::{
    self, InsightFreshness, InsightHistoryItem, InsightRecommendation, InsightsComparison,
    ToneHistoryItem, ToneRun, WritingInsightSnapshot, WritingInsightsRequest, WritingInsightsRun,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

// ---- Shared state -------------------------------------------------------------

#[derive(Default)]
struct AiState {
    /// Conversation id → stop flag for the reply being generated.
    inflight: Mutex<HashMap<i64, Arc<AtomicBool>>>,
    /// Models being downloaded right now.
    pulls: Mutex<HashSet<String>>,
}

fn state() -> &'static AiState {
    static STATE: OnceLock<AiState> = OnceLock::new();
    STATE.get_or_init(AiState::default)
}

fn settings() -> Settings {
    Settings::load_or_default()
}

fn store() -> Result<Store, String> {
    Store::open_default().map_err(err)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

// ---- Provider + models --------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub size: Option<u64>,
    pub embedding: bool,
    /// The server says the model can think (`None` = unknown).
    pub thinking: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiStatus {
    pub provider: LlmProvider,
    pub provider_name: String,
    pub endpoint: String,
    pub reachable: bool,
    pub message: String,
    pub models: Vec<ModelInfo>,
    pub can_download: bool,
}

/// Installed models with capabilities (Ollama `/api/show`, LM Studio types).
fn model_infos(client: &dyn LlmClient) -> Result<Vec<ModelInfo>, String> {
    let models = client.list_models().map_err(|e| llm::error_text(&e))?;
    let infos = std::thread::scope(|scope| {
        let handles: Vec<_> = models
            .into_iter()
            .map(|m| {
                scope.spawn(move || {
                    let caps = client.model_capabilities(&m.name);
                    let embedding = caps
                        .as_ref()
                        .map(|c| {
                            c.iter().any(|x| x == "embedding")
                                && !c.iter().any(|x| x == "completion")
                        })
                        .unwrap_or(m.embedding);
                    ModelInfo {
                        thinking: caps.as_ref().map(|c| c.iter().any(|x| x == "thinking")),
                        embedding,
                        size: m.size,
                        name: m.name,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect::<Vec<_>>()
    });
    let mut infos = infos;
    infos.sort_by_key(|a| a.name.to_lowercase());
    Ok(infos)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn ai_status() -> Result<AiStatus, String> {
    blocking(|| {
        let s = settings();
        let client = client_from_settings(&s);
        let health = client.health();
        let models = if health.reachable {
            model_infos(client.as_ref()).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(AiStatus {
            provider: s.llm_provider,
            provider_name: llm::provider_display_name(s.llm_provider).into(),
            endpoint: client.endpoint().to_string(),
            reachable: health.reachable,
            message: health.message,
            models,
            can_download: true,
        })
    })
    .await
}

/// Check a provider/endpoint (defaults: the saved ones).
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_test_connection(
    provider: Option<LlmProvider>,
    endpoint: Option<String>,
) -> Result<LlmHealth, String> {
    blocking(move || {
        let s = settings();
        let provider = provider.unwrap_or(s.llm_provider);
        let endpoint = endpoint.unwrap_or_else(|| match provider {
            LlmProvider::Ollama => s.ollama_endpoint.clone(),
            LlmProvider::LmStudio => s.lmstudio_endpoint.clone(),
        });
        Ok(client_for(provider, &endpoint).health())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn ai_models() -> Result<Vec<ModelInfo>, String> {
    blocking(|| model_infos(client_from_settings(&settings()).as_ref())).await
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub name: String,
    pub size: Option<u64>,
    pub kind: catalog::CatalogKind,
    pub installed: bool,
}

/// Curated lightweight models with whether each is installed.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_catalog() -> Result<Vec<CatalogEntry>, String> {
    blocking(|| {
        let installed = client_from_settings(&settings())
            .list_models()
            .map(|m| m.into_iter().map(|m| m.name).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(catalog::curated()
            .into_iter()
            .map(|m| CatalogEntry {
                installed: catalog::is_installed(m.name, &installed),
                name: m.name.to_string(),
                size: m.size,
                kind: m.kind,
            })
            .collect())
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
struct PullEvent {
    model: String,
    purpose: String,
    status: String,
    completed: Option<u64>,
    total: Option<u64>,
    fraction: Option<f64>,
    done: bool,
    error: Option<String>,
}

/// Download `model` on the provider in the background, emitting
/// `ai-pull-progress` (throttled) until it finishes.
fn spawn_pull(app: AppHandle, model: String, purpose: String) -> Result<(), String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("Pick a model to download.".into());
    }
    if !state().pulls.lock().unwrap().insert(model.clone()) {
        return Err(format!("{model} is already downloading."));
    }
    std::thread::spawn(move || {
        let client = client_from_settings(&settings());
        let mut last = Instant::now() - Duration::from_secs(1);
        let emit = |p: &PullProgress, done: bool, error: Option<String>| {
            let _ = app.emit(
                "ai-pull-progress",
                PullEvent {
                    model: model.clone(),
                    purpose: purpose.clone(),
                    status: p.status.clone(),
                    completed: p.completed,
                    total: p.total,
                    fraction: p.fraction(),
                    done,
                    error,
                },
            );
        };
        let result = client.pull_model(&model, &mut |p| {
            if last.elapsed() >= Duration::from_millis(150) {
                last = Instant::now();
                emit(&p, false, None);
            }
        });
        state().pulls.lock().unwrap().remove(&model);
        let final_progress = PullProgress {
            model: model.clone(),
            status: if result.is_ok() {
                "Downloaded".into()
            } else {
                "Download failed".into()
            },
            completed: None,
            total: None,
        };
        emit(
            &final_progress,
            true,
            result.err().map(|e| llm::error_text(&e)),
        );
        let _ = app.emit("ai-models-changed", ());
    });
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn ai_pull_model(app: AppHandle, model: String, purpose: Option<String>) -> Result<(), String> {
    spawn_pull(app, model, purpose.unwrap_or_else(|| "model".into()))
}

#[derive(Debug, Clone, Serialize)]
pub struct WarmResult {
    pub model: String,
    pub ms: u64,
}

/// Load the enabled polish and insights models into memory ("Warm Now").
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_warm_models() -> Result<Vec<WarmResult>, String> {
    blocking(|| {
        let s = settings();
        let client = client_from_settings(&s);
        let mut models: Vec<String> = Vec::new();
        if s.polish_enabled {
            match s.polish_engine {
                PolishEngine::LocalLlm => models.push(PolishOptions::from_settings(&s).model),
                PolishEngine::S1Mini => {
                    let names: Vec<String> = client
                        .list_models()
                        .map(|m| m.into_iter().map(|m| m.name).collect())
                        .unwrap_or_default();
                    if let Some(m) = catalog::installed_s1_mini(&names) {
                        models.push(m);
                    }
                }
            }
        }
        if s.insights_enabled {
            models.push(writing::default_tone_model(&s));
        }
        let mut seen = HashSet::new();
        models.retain(|m| seen.insert(m.to_lowercase()));
        if models.is_empty() {
            return Err("Enable local polish or local insights to warm a model.".into());
        }
        models
            .into_iter()
            .map(|m| {
                client
                    .warm(&m, 40_000)
                    .map(|ms| WarmResult { model: m, ms })
                    .map_err(|e| llm::error_text(&e))
            })
            .collect()
    })
    .await
}

// ---- S1-mini ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct S1MiniStatus {
    pub provider_name: String,
    pub reachable: bool,
    pub installed: bool,
    /// The installed name on the server.
    pub model: Option<String>,
    pub downloading: bool,
    pub download: String,
    pub size: u64,
    pub credit: String,
    pub message: String,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn s1_mini_status() -> Result<S1MiniStatus, String> {
    blocking(|| {
        let s = settings();
        let client = client_from_settings(&s);
        let health = client.health();
        let model = catalog::installed_s1_mini(&health.models);
        let download = catalog::s1_mini_download(s.llm_provider);
        let name = llm::provider_display_name(s.llm_provider);
        let message = if !health.reachable {
            format!("Start {name} to install and run S1-mini.")
        } else if let Some(m) = &model {
            format!("Installed on {name} as {m}.")
        } else {
            format!("{name} downloads S1-mini from Hugging Face and serves it on this computer.")
        };
        Ok(S1MiniStatus {
            provider_name: name.into(),
            reachable: health.reachable,
            installed: model.is_some(),
            downloading: state().pulls.lock().unwrap().contains(&download),
            model,
            download,
            size: catalog::S1_MINI_BYTES,
            credit: catalog::S1_MINI_CREDIT.into(),
            message,
        })
    })
    .await
}

/// One-click S1-mini install (Ollama `hf.co/…` pull / LM Studio download).
#[tauri::command(rename_all = "snake_case")]
pub fn s1_mini_install(app: AppHandle) -> Result<(), String> {
    let s = settings();
    spawn_pull(
        app,
        catalog::s1_mini_download(s.llm_provider),
        "s1-mini".into(),
    )
}

// ---- Polish + voice edit previews ---------------------------------------------

/// Polish `text` with the current settings (Settings → "Try it").
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_polish_test(text: String) -> Result<PolishOutcome, String> {
    blocking(move || {
        let s = Settings {
            polish_enabled: true,
            ..settings()
        };
        let protected = store()
            .and_then(|st| st.list_dictionary().map_err(err))
            .map(|d| {
                d.into_iter()
                    .filter(|e| e.is_active)
                    .map(|e| e.target)
                    .collect()
            })
            .unwrap_or_default();
        let polisher = Polisher::new(
            client_from_settings(&s),
            PolishOptions::from_settings(&s).with_protected_words(protected),
        );
        Ok(polisher.polish(&text, None))
    })
    .await
}

/// Rewrite text per a spoken/typed instruction with the edit model.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_rewrite_selection(
    selection: String,
    instruction: String,
) -> Result<String, String> {
    blocking(move || {
        let s = settings();
        let opts = edit::EditOptions::from_settings(&s);
        edit::rewrite_selection(
            client_from_settings(&s).as_ref(),
            &opts.model,
            &selection,
            &instruction,
            opts.timeout_ms,
            opts.max_chars,
        )
        .map_err(|e| llm::error_text(&e))
    })
    .await
}

// ---- Chat AI -------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_conversations(
    search: Option<String>,
    utc_offset: Option<i64>,
) -> Result<Vec<ChatGroup>, String> {
    blocking(move || {
        let list = store()?
            .chat_list(search.as_deref(), chat::MAX_CONVERSATIONS as i64)
            .map_err(err)?;
        Ok(chat::group_conversations(
            list,
            now_unix(),
            utc_offset.unwrap_or(0),
        ))
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_messages(conversation_id: i64) -> Result<Vec<ChatMessageRow>, String> {
    blocking(move || store()?.chat_messages(conversation_id).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_rename(conversation_id: i64, title: String) -> Result<(), String> {
    blocking(move || {
        let t = title.trim();
        if t.is_empty() {
            return Err("Give the chat a title.".into());
        }
        store()?.chat_rename(conversation_id, t).map_err(err)?;
        Ok(())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_delete(conversation_id: i64) -> Result<(), String> {
    if let Some(flag) = state().inflight.lock().unwrap().get(&conversation_id) {
        flag.store(true, Ordering::Relaxed);
    }
    blocking(move || {
        store()?.chat_delete(conversation_id).map_err(err)?;
        Ok(())
    })
    .await
}

/// Chat-capable installed models (embedding-only models left out).
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_models() -> Result<Vec<ModelInfo>, String> {
    blocking(|| {
        Ok(model_infos(client_from_settings(&settings()).as_ref())?
            .into_iter()
            .filter(|m| !m.embedding)
            .collect())
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatInfo {
    pub provider_name: String,
    pub endpoint: String,
    pub default_model: String,
    pub has_tone_profile: bool,
    pub semantic_enabled: bool,
    /// Conversations with a reply being generated.
    pub inflight: Vec<i64>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_info() -> Result<ChatInfo, String> {
    blocking(|| {
        let s = settings();
        Ok(ChatInfo {
            provider_name: llm::provider_display_name(s.llm_provider).into(),
            endpoint: s.llm_endpoint().to_string(),
            default_model: chat::default_chat_model(&s),
            has_tone_profile: store()
                .ok()
                .and_then(|st| writing::latest_tone_profile(&st))
                .is_some(),
            semantic_enabled: s.semantic_enabled,
            inflight: state().inflight.lock().unwrap().keys().copied().collect(),
        })
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
struct ChatDeltaEvent<'a> {
    conversation_id: i64,
    message_id: i64,
    content: &'a str,
    reasoning: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct ChatErrorEvent {
    conversation_id: i64,
    message_id: i64,
    error: String,
}

/// Store the message and stream the reply on a worker thread. Returns at
/// once with the ids the `chat-delta` / `chat-done` events refer to.
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_send(
    app: AppHandle,
    request: ChatRequest,
    utc_offset: Option<i64>,
) -> Result<PreparedTurn, String> {
    if let Some(id) = request.conversation_id {
        if state().inflight.lock().unwrap().contains_key(&id) {
            return Err("A reply is still being written. Stop it or wait for it to finish.".into());
        }
    }
    let s = settings();
    let turn = {
        let req = request.clone();
        let s = s.clone();
        blocking(move || chat::prepare_turn(&store()?, &s, &req).map_err(|e| llm::error_text(&e)))
            .await?
    };
    let cancel = Arc::new(AtomicBool::new(false));
    state()
        .inflight
        .lock()
        .unwrap()
        .insert(turn.conversation_id, cancel.clone());
    let out = turn.clone();
    std::thread::spawn(move || {
        let conversation_id = turn.conversation_id;
        let message_id = turn.assistant_message_id;
        let result: Result<TurnResult, String> = (|| {
            let st = store()?;
            let client = client_from_settings(&s);
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut on_update = |content: &str, reasoning: &str| {
                if last.elapsed() >= Duration::from_millis(50) {
                    last = Instant::now();
                    let _ = app.emit(
                        "chat-delta",
                        ChatDeltaEvent {
                            conversation_id,
                            message_id,
                            content,
                            reasoning,
                        },
                    );
                }
            };
            chat::run_turn(
                &st,
                client.as_ref(),
                &s,
                &turn,
                &request,
                now_unix(),
                utc_offset.unwrap_or(0),
                &cancel,
                &mut on_update,
            )
            .map_err(|e| llm::error_text(&e))
        })();
        state().inflight.lock().unwrap().remove(&conversation_id);
        match result {
            Ok(done) => {
                let _ = app.emit("chat-done", &done);
            }
            Err(error) => {
                let _ = app.emit(
                    "chat-error",
                    ChatErrorEvent {
                        conversation_id,
                        message_id,
                        error,
                    },
                );
            }
        }
    });
    Ok(out)
}

/// Stop the reply being generated for a conversation.
#[tauri::command(rename_all = "snake_case")]
pub fn chat_stop(conversation_id: i64) -> bool {
    match state().inflight.lock().unwrap().get(&conversation_id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

// ---- Writing insights -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WritingInsightsInfo {
    pub analyzer_name: String,
    pub uses_llm: bool,
    pub models: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn writing_insights_info() -> WritingInsightsInfo {
    let s = settings();
    let models = writing::insight_candidate_models(&s);
    WritingInsightsInfo {
        analyzer_name: writing::primary_analyzer_name(&s),
        uses_llm: !models.is_empty(),
        models,
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_generate(
    app: AppHandle,
    request: Option<WritingInsightsRequest>,
    utc_offset: Option<i64>,
) -> Result<WritingInsightsRun, String> {
    let run = blocking(move || {
        let s = settings();
        Ok(writing::generate_writing_insights(
            &store()?,
            client_from_settings(&s).as_ref(),
            &s,
            &request.unwrap_or_default(),
            now_unix(),
            utc_offset.unwrap_or(0),
        ))
    })
    .await?;
    let _ = app.emit("writing-insights-updated", &run);
    Ok(run)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_quick(
    request: Option<WritingInsightsRequest>,
    utc_offset: Option<i64>,
) -> Result<Option<WritingInsightSnapshot>, String> {
    blocking(move || {
        writing::quick_insights(
            &store()?,
            &request.unwrap_or_default(),
            now_unix(),
            utc_offset.unwrap_or(0),
        )
        .map_err(err)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_latest() -> Result<Option<InsightHistoryItem>, String> {
    blocking(|| writing::latest_writing_insights(&store()?).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_history(
    limit: Option<i64>,
) -> Result<Vec<InsightHistoryItem>, String> {
    blocking(move || writing::writing_insights_history(&store()?, limit.unwrap_or(24)).map_err(err))
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_get(id: i64) -> Result<Option<InsightHistoryItem>, String> {
    blocking(move || writing::get_writing_insights(&store()?, id).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_freshness(generated_at: i64) -> Result<InsightFreshness, String> {
    blocking(move || writing::freshness(&store()?, generated_at).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_compare(
    a: i64,
    b: i64,
    utc_offset: Option<i64>,
) -> Result<InsightsComparison, String> {
    blocking(move || {
        let st = store()?;
        let get = |id| {
            writing::get_writing_insights(&st, id)
                .map_err(err)?
                .ok_or_else(|| "That insight run no longer exists.".to_string())
        };
        Ok(writing::compare(
            &get(a)?,
            &get(b)?,
            utc_offset.unwrap_or(0),
        ))
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_insights_apps() -> Result<Vec<String>, String> {
    blocking(|| {
        let mut apps = store()?.history_apps().map_err(err)?;
        apps.truncate(60);
        Ok(apps)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn writing_apply_recommendation(
    app: AppHandle,
    recommendation: InsightRecommendation,
) -> Result<i64, String> {
    let id =
        blocking(move || writing::apply_recommendation(&store()?, &recommendation).map_err(err))
            .await?;
    let _ = app.emit("memory-changed", ());
    Ok(id)
}

/// Pin/unpin a writing-insight or tone snapshot.
#[tauri::command(rename_all = "snake_case")]
pub async fn insight_snapshot_pin(id: i64, pinned: bool) -> Result<bool, String> {
    blocking(move || {
        store()?
            .set_insight_snapshot_pinned(id, pinned)
            .map_err(err)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn insight_snapshot_delete(id: i64) -> Result<bool, String> {
    blocking(move || store()?.delete_insight_snapshot(id).map_err(err)).await
}

// ---- Tone of voice ----------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub async fn tone_analyze(
    app: AppHandle,
    model: Option<String>,
) -> Result<Option<ToneRun>, String> {
    let run = blocking(move || {
        let s = settings();
        let model = model.unwrap_or_else(|| writing::default_tone_model(&s));
        writing::analyze_tone(
            &store()?,
            client_from_settings(&s).as_ref(),
            &s,
            &model,
            now_unix(),
        )
        .map_err(err)
    })
    .await?;
    if let Some(r) = &run {
        let _ = app.emit("tone-updated", r);
    }
    Ok(run)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tone_latest() -> Result<Option<ToneHistoryItem>, String> {
    blocking(|| writing::latest_tone(&store()?).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tone_history(limit: Option<i64>) -> Result<Vec<ToneHistoryItem>, String> {
    blocking(move || writing::tone_history(&store()?, limit.unwrap_or(24)).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tone_get(id: i64) -> Result<Option<ToneHistoryItem>, String> {
    blocking(move || writing::get_tone(&store()?, id).map_err(err)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tone_sample_count() -> Result<i64, String> {
    blocking(|| Ok(store()?.count().map_err(err)?.min(800))).await
}
