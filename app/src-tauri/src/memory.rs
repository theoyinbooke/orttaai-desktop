//! Personal Memory commands: dictionary, snippets, learning suggestions, edit
//! checks, import/export and the live text-pipeline preview. (Text-pipeline
//! area.) Every change emits `memory-changed` so the engine reloads its text
//! pipeline and other windows refresh.

use crate::err;
use orttaai_core::error::{CoreError, Result as CoreResult};
use orttaai_core::llm::{
    policy, ChatDelta, ChatMessage, ChatOptions, ChatReply, LlmClient, LlmModel, Role,
};
use orttaai_core::settings::{LlmProvider, Settings, SpeechEngine};
use orttaai_core::store::{
    AiAnalyzer, DictionaryEntry, EditCheck, ImportSummary, LearningRunResult, LearningSuggestion,
    SnippetEntry, Store, STATUS_PENDING,
};
use orttaai_core::text::{format_numbers, style_instruction};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Tell the engine (and other windows) that memory changed so the live
/// dictation pipeline reloads it without a restart.
fn changed(app: &AppHandle) {
    let _ = app.emit("memory-changed", ());
}

fn store() -> Result<Store, String> {
    Store::open_default().map_err(err)
}

// ---- Dictionary ------------------------------------------------------------------

#[tauri::command]
pub fn list_dictionary() -> Result<Vec<DictionaryEntry>, String> {
    store()?.list_dictionary().map_err(err)
}

#[tauri::command]
pub fn save_dictionary(
    app: AppHandle,
    id: Option<i64>,
    source: String,
    target: String,
    case_sensitive: bool,
    active: Option<bool>,
) -> Result<(), String> {
    if source.trim().is_empty() || target.trim().is_empty() {
        return Err("Both fields are required.".into());
    }
    let store = store()?;
    match id {
        Some(id) => {
            store.update_dictionary(id, &source, &target, case_sensitive, active.unwrap_or(true))
        }
        None => store
            .upsert_dictionary(&source, &target, case_sensitive)
            .and_then(|id| match active {
                Some(false) => store.update_dictionary(id, &source, &target, case_sensitive, false),
                _ => Ok(()),
            }),
    }
    .map_err(|e| friendly_unique(e, "dictionary entry"))?;
    changed(&app);
    Ok(())
}

/// Turn one dictionary entry on or off.
#[tauri::command]
pub fn set_dictionary_active(app: AppHandle, id: i64, active: bool) -> Result<(), String> {
    let store = store()?;
    let entry = store
        .list_dictionary()
        .map_err(err)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or("That dictionary entry no longer exists.")?;
    store
        .update_dictionary(
            id,
            &entry.source,
            &entry.target,
            entry.is_case_sensitive,
            active,
        )
        .map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_dictionary(app: AppHandle, id: i64) -> Result<(), String> {
    store()?.delete_dictionary(id).map_err(err)?;
    changed(&app);
    Ok(())
}

// ---- Snippets --------------------------------------------------------------------

#[tauri::command]
pub fn list_snippets() -> Result<Vec<SnippetEntry>, String> {
    store()?.list_snippets().map_err(err)
}

#[tauri::command]
pub fn save_snippet(
    app: AppHandle,
    id: Option<i64>,
    trigger: String,
    expansion: String,
    active: Option<bool>,
) -> Result<(), String> {
    if trigger.trim().is_empty() || expansion.trim().is_empty() {
        return Err("Both fields are required.".into());
    }
    let store = store()?;
    match id {
        Some(id) => store.update_snippet(id, &trigger, &expansion, active.unwrap_or(true)),
        None => store
            .upsert_snippet(&trigger, &expansion)
            .and_then(|id| match active {
                Some(false) => store.update_snippet(id, &trigger, &expansion, false),
                _ => Ok(()),
            }),
    }
    .map_err(|e| friendly_unique(e, "snippet"))?;
    changed(&app);
    Ok(())
}

/// Turn one snippet on or off.
#[tauri::command]
pub fn set_snippet_active(app: AppHandle, id: i64, active: bool) -> Result<(), String> {
    let store = store()?;
    let entry = store
        .list_snippets()
        .map_err(err)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("That snippet no longer exists.")?;
    store
        .update_snippet(id, &entry.trigger, &entry.expansion, active)
        .map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_snippet(app: AppHandle, id: i64) -> Result<(), String> {
    store()?.delete_snippet(id).map_err(err)?;
    changed(&app);
    Ok(())
}

/// Renaming an entry onto another one's source/trigger trips the unique index.
fn friendly_unique(e: CoreError, what: &str) -> String {
    let text = e.to_string();
    if text.contains("UNIQUE") {
        format!("Another {what} already uses that phrase.")
    } else {
        text
    }
}

// ---- Suggestions -------------------------------------------------------------------

/// Pending suggestions, newest first (Mac shows up to 150).
#[tauri::command]
pub fn list_suggestions() -> Result<Vec<LearningSuggestion>, String> {
    store()?
        .list_learning_suggestions(Some(STATUS_PENDING), 150)
        .map_err(err)
}

/// Accept a suggestion: its dictionary entry (or snippet) is created.
#[tauri::command]
pub fn accept_suggestion(app: AppHandle, id: i64) -> Result<(), String> {
    store()?
        .accept_learning_suggestion(id)
        .map_err(err)?
        .ok_or("That suggestion no longer exists.")?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn dismiss_suggestion(app: AppHandle, id: i64) -> Result<(), String> {
    store()?.dismiss_learning_suggestion(id).map_err(err)?;
    changed(&app);
    Ok(())
}

/// The last few edit checks (what Learn from My Edits found; never text).
#[tauri::command]
pub fn list_edit_checks() -> Result<Vec<EditCheck>, String> {
    store()?.list_edit_checks(8).map_err(err)
}

#[derive(Serialize)]
pub struct AnalysisReport {
    #[serde(flatten)]
    pub result: LearningRunResult,
    pub message: String,
}

/// Analyze recent history for suggestions ("Analyze History"): the local AI
/// model when AI suggestions are on, the heuristic analyzer otherwise or as a
/// fallback.
#[tauri::command]
pub async fn analyze_history(app: AppHandle) -> Result<AnalysisReport, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let settings = Settings::load_or_default();
        let store = store()?;
        let client = settings
            .ai_suggestions
            .then(|| LocalModelClient::for_settings(&settings))
            .flatten();
        let ai = client.as_ref().map(|(c, model)| AiAnalyzer {
            llm: c,
            model: model.clone(),
        });
        store.analyze_history(ai, 120).map_err(err)
    })
    .await
    .map_err(err)??;
    if result.inserted_count > 0 {
        changed(&app);
    }
    Ok(AnalysisReport {
        message: result.message(),
        result,
    })
}

// ---- Import / export ---------------------------------------------------------------

/// Write the dictionary and snippets to `orttaai-memory-YYYY-MM-DD.json` in
/// the Downloads folder (or the app data folder) and return its path.
#[tauri::command]
pub fn export_memory(app: AppHandle) -> Result<String, String> {
    let json = store()?.export_memory_json().map_err(err)?;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| Store::data_dir().map_err(err))
        .map_err(err)?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let base = format!("orttaai-memory-{}", today());
    let mut path = dir.join(format!("{base}.json"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.json"));
        n += 1;
    }
    std::fs::write(&path, json).map_err(err)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Merge a memory export (or a JSON array of `{source, target}`) into memory.
#[tauri::command]
pub fn import_memory(app: AppHandle, json: String) -> Result<ImportSummary, String> {
    let summary = store()?
        .import_memory_json(&json)
        .map_err(|e| format!("That file isn't an Orttaai memory export ({e})."))?;
    changed(&app);
    Ok(summary)
}

/// `YYYY-MM-DD` (UTC) for file names.
fn today() -> String {
    let days = orttaai_core::store::now_unix().div_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// ---- Live preview ------------------------------------------------------------------

#[derive(Serialize)]
pub struct PipelinePreview {
    pub text: String,
    pub changes: Vec<String>,
    pub dictionary_hits: Vec<String>,
    pub snippet_hits: Vec<String>,
    /// The polish instruction the current style mode adds, if any.
    pub style_instruction: Option<String>,
}

/// Run `text` through the text pipeline with the current settings and
/// memory, exactly as a dictation would be (without counting usage).
#[tauri::command]
pub async fn preview_text_pipeline(
    text: String,
    app_name: Option<String>,
) -> Result<PipelinePreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = Settings::load_or_default();
        let processor = store()?.text_processor(&settings).map_err(err)?;
        let input = if settings.engine == SpeechEngine::Express {
            format_numbers(&text)
        } else {
            text
        };
        let out = processor.process_for_app(&input, app_name.as_deref());
        Ok(PipelinePreview {
            text: out.text,
            changes: out.changes,
            dictionary_hits: out.dictionary_hits,
            snippet_hits: out.snippet_hits,
            style_instruction: style_instruction(settings.style_mode).map(str::to_string),
        })
    })
    .await
    .map_err(err)?
}

// ---- Local model for AI suggestions ------------------------------------------------

/// A one-shot client for the configured local LLM host (Ollama's
/// `/api/generate`, or LM Studio's OpenAI-compatible chat endpoint), used by
/// AI suggestions. Only local endpoints and non-cloud models are allowed.
struct LocalModelClient {
    provider: LlmProvider,
    endpoint: String,
}

impl LocalModelClient {
    /// The client and model for AI suggestions, or `None` when no model is
    /// configured or the endpoint is not local.
    fn for_settings(settings: &Settings) -> Option<(Self, String)> {
        let model = [
            &settings.insights_model,
            &settings.chat_model,
            &settings.polish_model,
        ]
        .into_iter()
        .map(|m| m.trim())
        .find(|m| !m.is_empty())?
        .to_string();
        let endpoint = settings.llm_endpoint().to_string();
        if !policy::is_local_endpoint(&endpoint) || policy::is_cloud_model(&model) {
            return None;
        }
        Some((
            Self {
                provider: settings.llm_provider,
                endpoint,
            },
            model,
        ))
    }

    fn base(&self) -> &str {
        self.endpoint.trim_end_matches('/')
    }
}

impl LlmClient for LocalModelClient {
    fn provider_name(&self) -> &str {
        match self.provider {
            LlmProvider::Ollama => "ollama",
            LlmProvider::LmStudio => "lmstudio",
        }
    }

    fn list_models(&self) -> CoreResult<Vec<LlmModel>> {
        match self.provider {
            LlmProvider::Ollama => Ok(orttaai_core::llm::list_models(&self.endpoint)?
                .into_iter()
                .map(|name| LlmModel {
                    name,
                    size: None,
                    embedding: false,
                })
                .collect()),
            LlmProvider::LmStudio => {
                #[derive(serde::Deserialize)]
                struct Model {
                    id: String,
                }
                #[derive(serde::Deserialize)]
                struct Models {
                    data: Vec<Model>,
                }
                let parsed: Models = ureq::get(&format!("{}/v1/models", self.base()))
                    .call()
                    .map_err(|e| {
                        CoreError::Llm(format!("request failed (is LM Studio running?): {e}"))
                    })?
                    .into_body()
                    .read_json()
                    .map_err(|e| CoreError::Llm(format!("bad response: {e}")))?;
                Ok(parsed
                    .data
                    .into_iter()
                    .map(|m| LlmModel {
                        name: m.id,
                        size: None,
                        embedding: false,
                    })
                    .collect())
            }
        }
    }

    fn chat(
        &self,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    ) -> CoreResult<ChatReply> {
        let content = match self.provider {
            LlmProvider::Ollama => {
                let prompt = messages
                    .iter()
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                orttaai_core::llm::generate(&self.endpoint, &opts.model, &prompt)?
            }
            LlmProvider::LmStudio => {
                #[derive(serde::Deserialize)]
                struct Message {
                    content: String,
                }
                #[derive(serde::Deserialize)]
                struct Choice {
                    message: Message,
                }
                #[derive(serde::Deserialize)]
                struct Reply {
                    choices: Vec<Choice>,
                }
                let msgs: Vec<serde_json::Value> = messages
                    .iter()
                    .map(|m| {
                        let role = match m.role {
                            Role::System => "system",
                            Role::Assistant => "assistant",
                            Role::User => "user",
                        };
                        serde_json::json!({ "role": role, "content": m.content })
                    })
                    .collect();
                let mut payload = serde_json::json!({
                    "model": opts.model,
                    "messages": msgs,
                    "stream": false,
                });
                if let Some(t) = opts.temperature {
                    payload["temperature"] = serde_json::json!(t);
                }
                let parsed: Reply = ureq::post(&format!("{}/v1/chat/completions", self.base()))
                    .send_json(payload)
                    .map_err(|e| {
                        CoreError::Llm(format!("request failed (is LM Studio running?): {e}"))
                    })?
                    .into_body()
                    .read_json()
                    .map_err(|e| CoreError::Llm(format!("bad response: {e}")))?;
                parsed
                    .choices
                    .into_iter()
                    .next()
                    .map(|c| c.message.content)
                    .unwrap_or_default()
            }
        };
        if let Some(cb) = on_delta {
            cb(ChatDelta::Content(content.clone()));
        }
        Ok(ChatReply {
            content,
            reasoning: None,
        })
    }

    fn embed(&self, _model: &str, _inputs: &[String]) -> CoreResult<Vec<Vec<f32>>> {
        Err(CoreError::Llm(
            "embeddings are not used by AI suggestions".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_a_calendar_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
        assert_eq!(&d[4..5], "-");
    }

    #[test]
    fn ai_client_needs_a_local_endpoint_and_a_model() {
        let mut s = Settings::default();
        let (_, model) = LocalModelClient::for_settings(&s).expect("default polish model");
        assert_eq!(model, s.polish_model);
        s.insights_model = "qwen3:4b".into();
        assert_eq!(LocalModelClient::for_settings(&s).unwrap().1, "qwen3:4b");
        s.ollama_endpoint = "https://api.example.com".into();
        assert!(LocalModelClient::for_settings(&s).is_none());
    }
}
