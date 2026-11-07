//! LM Studio's server (Mac `LMStudioClient`; default `http://127.0.0.1:1234`):
//! native `GET /api/v1/models` (type + size, local files only) with the
//! OpenAI-compatible `GET /v1/models` fallback (llama.cpp, mlx_lm, vLLM…),
//! `POST /v1/chat/completions` streamed as SSE (incl. `reasoning_content` /
//! `reasoning` deltas and `response_format: json_schema`), `POST
//! /v1/completions` for raw prompts, `POST /v1/embeddings`, and `POST
//! /api/v1/models/download` + status polling.
//!
//! LM Studio silently substitutes whatever model is loaded when the requested
//! id doesn't exist, so every request first verifies the model against a
//! short-lived model-list cache and fails loudly instead.

use super::{ollama::finish_reply, ChatDelta, ChatReply, LlmModel, PullProgress};
use crate::error::{CoreError, Result};
use serde_json::Value;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:1234";

/// One native `/api/v1/models` entry kept as local.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModel {
    pub key: String,
    /// `llm` or `embedding`.
    pub kind: String,
    pub size: Option<u64>,
}

/// An `/api/v1/models` entry is local when LM Studio reports files on disk for
/// it; hosted (Secure Cloud) models have none.
pub fn is_local_entry(entry: &Value) -> bool {
    let has_key = entry
        .get("key")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.trim().is_empty());
    let size = entry.get("size_bytes").and_then(Value::as_u64).unwrap_or(0);
    has_key && size > 0
}

/// Local models from a native `/api/v1/models` body; `None` when the body
/// isn't the native shape (older LM Studio / other OpenAI-compatible servers).
pub fn parse_native_models(json: &Value) -> Option<Vec<NativeModel>> {
    let entries = json.get("models")?.as_array()?;
    Some(
        entries
            .iter()
            .filter(|e| is_local_entry(e))
            .filter_map(|e| {
                Some(NativeModel {
                    key: e.get("key")?.as_str()?.trim().to_string(),
                    kind: e
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("llm")
                        .to_string(),
                    size: e.get("size_bytes").and_then(Value::as_u64),
                })
            })
            .collect(),
    )
}

/// Model ids from an OpenAI-compatible `/v1/models` body.
pub fn parse_openai_models(json: &Value) -> Vec<String> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|d| {
            d.iter()
                .filter_map(|e| e.get("id").and_then(Value::as_str))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && !super::policy::is_cloud_model(s))
                .collect()
        })
        .unwrap_or_default()
}

/// Models for the trait from either list shape.
pub fn to_models(native: Option<&[NativeModel]>, openai: &[String]) -> Vec<LlmModel> {
    match native {
        Some(n) => n
            .iter()
            .map(|m| LlmModel {
                name: m.key.clone(),
                size: m.size,
                embedding: m.kind == "embedding",
            })
            .collect(),
        None => openai
            .iter()
            .map(|id| LlmModel {
                name: id.clone(),
                size: None,
                embedding: super::ollama::looks_like_embedding(id, None),
            })
            .collect(),
    }
}

/// `"5m"` → 300, `"90s"` → 90, `"1h"` → 3600, `"300"` → 300 (LM Studio `ttl`).
pub fn ttl_seconds(keep_alive: &str) -> Option<u64> {
    let t = keep_alive.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<u64>() {
        return (n > 0).then_some(n);
    }
    let (num, unit) = t.split_at(t.len() - 1);
    let n: u64 = num.parse().ok().filter(|n| *n > 0)?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3_600),
        _ => None,
    }
}

/// `response_format` for a JSON schema (strict structured output).
pub fn response_format(schema: &Value) -> Value {
    serde_json::json!({
        "type": "json_schema",
        "json_schema": { "name": "structured_response", "strict": true, "schema": schema },
    })
}

/// Read a streamed `/v1/chat/completions` body (Server-Sent Events). Content
/// comes in `choices[0].delta.content`; reasoning models add
/// `delta.reasoning_content` (LM Studio, DeepSeek) or `delta.reasoning`.
pub fn read_sse_stream(
    reader: impl BufRead,
    mut on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    cancel: &AtomicBool,
) -> Result<ChatReply> {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut finish: Option<String> = None;
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Llm(super::CANCELLED.into()));
        }
        let line = line.map_err(|e| super::ollama::stream_io_error("LM Studio", e))?;
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(err) = json.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| err.as_str())
                .unwrap_or("unknown error");
            return Err(CoreError::Llm(format!("LM Studio error: {msg}")));
        }
        let Some(choice) = json
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            continue;
        };
        let delta = choice.get("delta").or_else(|| choice.get("message"));
        if let Some(delta) = delta {
            for key in ["reasoning_content", "reasoning"] {
                if let Some(piece) = delta
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                {
                    thinking.push_str(piece);
                    if let Some(cb) = on_delta.as_mut() {
                        cb(ChatDelta::Reasoning(piece.to_string()));
                    }
                    break;
                }
            }
            if let Some(piece) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            {
                content.push_str(piece);
                if let Some(cb) = on_delta.as_mut() {
                    cb(ChatDelta::Content(piece.to_string()));
                }
            }
        }
        if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
            finish = Some(r.to_string());
        }
    }
    finish_reply("LM Studio", content, thinking).map_err(|e| match finish {
        Some(r) => CoreError::Llm(format!("{} finish_reason={r}.", super::error_text(&e))),
        None => e,
    })
}

/// `choices[0].text` from a `/v1/completions` body.
pub fn parse_completion_text(json: &Value) -> Option<String> {
    let text = json
        .get("choices")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    let t = super::strip_think_tags(text).trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Vectors from a `/v1/embeddings` body, ordered by `index`.
pub fn parse_embeddings(json: &Value) -> Vec<Vec<f32>> {
    let Some(data) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut rows: Vec<(u64, Vec<f32>)> = data
        .iter()
        .filter_map(|e| {
            let v: Vec<f32> = e
                .get("embedding")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_f64().map(|f| f as f32))
                .collect();
            (!v.is_empty()).then(|| (e.get("index").and_then(Value::as_u64).unwrap_or(0), v))
        })
        .collect();
    rows.sort_by_key(|(i, _)| *i);
    rows.into_iter().map(|(_, v)| v).collect()
}

/// A download job's status + progress, from the start response or a poll.
pub fn download_state(json: &Value, model: &str) -> (String, PullProgress) {
    let status = json
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("downloading")
        .to_string();
    let total = json.get("total_size_bytes").and_then(Value::as_u64);
    let done_status = status == "completed" || status == "already_downloaded";
    let done = json
        .get("downloaded_bytes")
        .and_then(Value::as_u64)
        .or(if done_status { total } else { None });
    let label = match status.as_str() {
        "paused" => "Paused in LM Studio",
        "completed" | "already_downloaded" => "Downloaded",
        _ => "Downloading in LM Studio",
    };
    (
        status,
        PullProgress {
            model: model.to_string(),
            status: label.to_string(),
            completed: done,
            total,
        },
    )
}

#[cfg(feature = "ollama")]
pub use client::LmStudioClient;

#[cfg(feature = "ollama")]
mod client {
    use super::super::http::{self, Ctx};
    use super::super::{
        policy, ChatDelta, ChatMessage, ChatOptions, ChatReply, GenerateRequest, LlmClient,
        LlmModel, PullProgress,
    };
    use super::*;
    use std::collections::HashMap;
    use std::io::BufReader;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const CACHE_LIFETIME: Duration = Duration::from_secs(30);

    /// Model list per endpoint, shared by every client instance (clients are
    /// rebuilt from settings per request).
    #[derive(Clone)]
    struct Cached {
        at: Instant,
        models: Vec<LlmModel>,
        /// `llm` / `embedding` per key, when the native list answered.
        types: HashMap<String, String>,
    }

    fn cache() -> &'static Mutex<HashMap<String, Cached>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// LM Studio (or any OpenAI-compatible server) on this computer or the LAN.
    pub struct LmStudioClient {
        endpoint: String,
    }

    impl LmStudioClient {
        pub fn new(endpoint: &str) -> Self {
            let trimmed = endpoint.trim().trim_end_matches('/');
            let trimmed = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
            let trimmed = trimmed.strip_suffix("/api/v1").unwrap_or(trimmed);
            Self {
                endpoint: if trimmed.is_empty() {
                    DEFAULT_ENDPOINT.to_string()
                } else {
                    trimmed.to_string()
                },
            }
        }

        fn ctx(&self, timeout_ms: Option<u64>) -> Result<Ctx<'_>> {
            policy::require_local_endpoint(&self.endpoint, "LM Studio")?;
            Ok(Ctx {
                provider: "LM Studio",
                endpoint: &self.endpoint,
                timeout_ms,
            })
        }

        fn url(&self, path: &str) -> String {
            http::url(&self.endpoint, path)
        }

        fn fetch(&self, timeout_ms: u64) -> Result<Cached> {
            let ctx = self.ctx(Some(timeout_ms))?;
            let native = http::get(&ctx, &self.url("/api/v1/models"))
                .ok()
                .filter(|r| r.is_success())
                .and_then(|r| r.text(&ctx).ok())
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|j| parse_native_models(&j));
            let cached = match native {
                Some(n) => Cached {
                    at: Instant::now(),
                    models: to_models(Some(&n), &[]),
                    types: n.into_iter().map(|m| (m.key, m.kind)).collect(),
                },
                None => {
                    let json = http::get_json(&ctx, &self.url("/v1/models"))?;
                    Cached {
                        at: Instant::now(),
                        models: to_models(None, &parse_openai_models(&json)),
                        types: HashMap::new(),
                    }
                }
            };
            cache()
                .lock()
                .unwrap()
                .insert(self.endpoint.clone(), cached.clone());
            Ok(cached)
        }

        fn cached(&self) -> Option<Cached> {
            cache()
                .lock()
                .unwrap()
                .get(&self.endpoint)
                .filter(|c| c.at.elapsed() < CACHE_LIFETIME && !c.models.is_empty())
                .cloned()
        }

        /// Fail loudly instead of letting LM Studio swap in another model.
        fn ensure_model(&self, model: &str) -> Result<String> {
            let m = model.trim();
            if m.is_empty() {
                return Err(CoreError::Llm("Missing LM Studio model name.".into()));
            }
            policy::require_local_model(m, "LM Studio")?;
            let list = match self.cached() {
                Some(c) => c,
                None => self.fetch(4_000)?,
            };
            if list.models.iter().any(|x| x.name.eq_ignore_ascii_case(m)) {
                Ok(m.to_string())
            } else {
                Err(CoreError::Llm(format!(
                    "Model \u{201C}{m}\u{201D} isn't a local model in LM Studio. {} Download it in LM Studio, then pick it from the model list.",
                    policy::REFUSAL_MESSAGE
                )))
            }
        }

        fn ttl() -> u64 {
            ttl_seconds("5m").unwrap_or(300)
        }
    }

    impl LlmClient for LmStudioClient {
        fn provider_name(&self) -> &str {
            "lmstudio"
        }

        fn endpoint(&self) -> &str {
            &self.endpoint
        }

        fn list_models(&self) -> Result<Vec<LlmModel>> {
            Ok(self.fetch(2_400)?.models)
        }

        fn model_capabilities(&self, model: &str) -> Option<Vec<String>> {
            let c = self.cached().or_else(|| self.fetch(2_000).ok())?;
            match c.types.get(model).map(String::as_str) {
                Some("llm") => Some(vec!["completion".into()]),
                Some("embedding") => Some(vec!["embedding".into()]),
                _ => None,
            }
        }

        fn chat(
            &self,
            messages: &[ChatMessage],
            opts: &ChatOptions,
            on_delta: Option<&mut dyn FnMut(ChatDelta)>,
        ) -> Result<ChatReply> {
            self.chat_cancellable(messages, opts, on_delta, &AtomicBool::new(false))
        }

        fn chat_cancellable(
            &self,
            messages: &[ChatMessage],
            opts: &ChatOptions,
            on_delta: Option<&mut dyn FnMut(ChatDelta)>,
            cancel: &AtomicBool,
        ) -> Result<ChatReply> {
            // `think` and `num_ctx` are Ollama concepts: LM Studio reasoning
            // models decide for themselves and context is fixed at load time.
            let msgs: Vec<Value> = messages
                .iter()
                .filter(|m| !m.content.trim().is_empty())
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content.trim() }))
                .collect();
            if msgs.is_empty() {
                return Err(CoreError::Llm(
                    "Missing chat messages for LM Studio.".into(),
                ));
            }
            let model = self.ensure_model(&opts.model)?;
            let mut payload = serde_json::json!({
                "model": model,
                "messages": msgs,
                "stream": true,
                "temperature": opts.temperature.unwrap_or(0.0),
                "ttl": Self::ttl(),
            });
            if let Some(n) = opts.max_tokens {
                payload["max_tokens"] = serde_json::json!(n.clamp(1, 16_000));
            }
            if let Some(schema) = &opts.json_schema {
                payload["response_format"] = response_format(schema);
            }
            let ctx = self.ctx(opts.timeout_ms)?;
            let resp = http::post_json(&ctx, &self.url("/v1/chat/completions"), &payload)?;
            if !resp.is_success() {
                return Err(ctx.status_error(resp));
            }
            read_sse_stream(BufReader::new(resp.body), on_delta, cancel)
        }

        fn generate(&self, req: &GenerateRequest, opts: &ChatOptions) -> Result<String> {
            if !req.raw {
                return Ok(self
                    .chat(&[ChatMessage::user(req.prompt.trim())], opts, None)?
                    .content);
            }
            let model = self.ensure_model(&opts.model)?;
            let mut payload = serde_json::json!({
                "model": model,
                "prompt": req.prompt,
                "stream": false,
                "temperature": opts.temperature.unwrap_or(0.0),
                "ttl": Self::ttl(),
            });
            if let Some(n) = opts.max_tokens {
                payload["max_tokens"] = serde_json::json!(n.clamp(1, 16_000));
            }
            if !req.stop.is_empty() {
                payload["stop"] = serde_json::json!(req.stop);
            }
            let ctx = self.ctx(opts.timeout_ms)?;
            let json = http::post_json_value(&ctx, &self.url("/v1/completions"), &payload)?;
            parse_completion_text(&json)
                .ok_or_else(|| CoreError::Llm("LM Studio returned an empty response.".into()))
        }

        fn embed(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            let inputs: Vec<&str> = inputs
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if inputs.is_empty() {
                return Err(CoreError::Llm(
                    "Missing text for LM Studio embeddings.".into(),
                ));
            }
            let model = self.ensure_model(model)?;
            let payload =
                serde_json::json!({ "model": model, "input": inputs, "ttl": Self::ttl() });
            let ctx = self.ctx(Some(120_000))?;
            let json = http::post_json_value(&ctx, &self.url("/v1/embeddings"), &payload)?;
            let rows = parse_embeddings(&json);
            if rows.len() != inputs.len() {
                return Err(CoreError::Llm(format!(
                    "LM Studio returned {} embeddings for {} input(s).",
                    rows.len(),
                    inputs.len()
                )));
            }
            Ok(rows)
        }

        fn pull_model(&self, model: &str, on_progress: &mut dyn FnMut(PullProgress)) -> Result<()> {
            let ctx = self.ctx(Some(30_000))?;
            let mut json = http::post_json_value(
                &ctx,
                &self.url("/api/v1/models/download"),
                &serde_json::json!({ "model": model }),
            )?;
            let job = json
                .get("job_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            loop {
                let (status, progress) = download_state(&json, model);
                on_progress(progress);
                match status.as_str() {
                    "completed" | "already_downloaded" => {
                        cache().lock().unwrap().remove(&self.endpoint);
                        return Ok(());
                    }
                    "failed" => {
                        return Err(CoreError::Llm(format!(
                            "LM Studio couldn't download {model}."
                        )))
                    }
                    _ => {
                        let Some(job) = job.as_deref() else {
                            return Err(CoreError::Llm(
                                "LM Studio didn't start the download.".into(),
                            ));
                        };
                        std::thread::sleep(Duration::from_secs(1));
                        let poll = self.ctx(Some(15_000))?;
                        json = http::get_json(
                            &poll,
                            &self.url(&format!("/api/v1/models/download/status/{job}")),
                        )?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn native_list_keeps_only_local_files() {
        let json: Value = serde_json::from_str(
            r#"{"models":[
                {"key":"qwen3.5-0.8b","type":"llm","size_bytes":812000000},
                {"key":"glm-5.3","type":"llm","size_bytes":0},
                {"key":"text-embedding-nomic-embed-text-v1.5","type":"embedding","size_bytes":84000000},
                {"type":"llm","size_bytes":5}
            ]}"#,
        )
        .unwrap();
        let native = parse_native_models(&json).unwrap();
        assert_eq!(native.len(), 2);
        let models = to_models(Some(&native), &[]);
        assert_eq!(models[0].name, "qwen3.5-0.8b");
        assert!(!models[0].embedding);
        assert!(models[1].embedding);
        assert!(parse_native_models(&serde_json::json!({"data": []})).is_none());
    }

    #[test]
    fn openai_list_fallback() {
        let json: Value = serde_json::from_str(
            r#"{"data":[{"id":"llama-3.2-1b"},{"id":"nomic-embed-text"},{"id":"x-cloud"},{"id":""}]}"#,
        )
        .unwrap();
        let ids = parse_openai_models(&json);
        assert_eq!(ids, vec!["llama-3.2-1b", "nomic-embed-text"]);
        let models = to_models(None, &ids);
        assert!(models[1].embedding);
    }

    #[test]
    fn sse_stream_with_reasoning() {
        let body = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Think. \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning\":\"More.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ignored\"}}]}\n\n",
        );
        let mut deltas = Vec::new();
        let mut cb = |d: ChatDelta| deltas.push(d);
        let r = read_sse_stream(Cursor::new(body), Some(&mut cb), &AtomicBool::new(false)).unwrap();
        assert_eq!(r.content, "Hi there");
        assert_eq!(r.reasoning.as_deref(), Some("Think. More."));
        assert_eq!(deltas.len(), 4);
    }

    #[test]
    fn sse_errors_and_empty() {
        let err = read_sse_stream(
            Cursor::new("data: {\"error\":{\"message\":\"Model unloaded\"}}\n\n"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Model unloaded"));
        let empty = read_sse_stream(
            Cursor::new("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(empty.to_string().contains("finish_reason=length"));
        let think = read_sse_stream(
            Cursor::new(
                "data: {\"choices\":[{\"delta\":{\"content\":\"<think>x</think>Done\"}}]}\n",
            ),
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(think.content, "Done");
        assert_eq!(think.reasoning.as_deref(), Some("x"));
    }

    #[test]
    fn helpers() {
        assert_eq!(ttl_seconds("5m"), Some(300));
        assert_eq!(ttl_seconds("90s"), Some(90));
        assert_eq!(ttl_seconds("1h"), Some(3_600));
        assert_eq!(ttl_seconds("300"), Some(300));
        assert_eq!(ttl_seconds("0"), None);
        assert_eq!(ttl_seconds("abc"), None);
        assert_eq!(ttl_seconds(""), None);

        let rf = response_format(&serde_json::json!({"type":"object"}));
        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["json_schema"]["strict"], true);

        let emb: Value = serde_json::from_str(
            r#"{"data":[{"index":1,"embedding":[0.5]},{"index":0,"embedding":[0.25]}]}"#,
        )
        .unwrap();
        assert_eq!(parse_embeddings(&emb), vec![vec![0.25], vec![0.5]]);

        let comp: Value =
            serde_json::from_str(r#"{"choices":[{"text":"  Cleaned text.  "}]}"#).unwrap();
        assert_eq!(
            parse_completion_text(&comp).as_deref(),
            Some("Cleaned text.")
        );
        assert!(parse_completion_text(&serde_json::json!({"choices":[{"text":" "}]})).is_none());
    }

    #[test]
    fn download_states() {
        let (s, p) = download_state(
            &serde_json::json!({"job_id":"j1","status":"downloading","total_size_bytes":100,"downloaded_bytes":40}),
            "m",
        );
        assert_eq!(s, "downloading");
        assert_eq!(p.fraction(), Some(0.4));
        let (s, p) = download_state(
            &serde_json::json!({"status":"already_downloaded","total_size_bytes":100}),
            "m",
        );
        assert_eq!(s, "already_downloaded");
        assert_eq!(p.status, "Downloaded");
        assert_eq!(p.fraction(), Some(1.0));
        assert_eq!(
            download_state(&serde_json::json!({"status":"paused"}), "m")
                .1
                .status,
            "Paused in LM Studio"
        );
    }
}
