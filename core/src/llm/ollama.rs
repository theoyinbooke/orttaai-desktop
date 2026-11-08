//! Ollama's native API (Mac `OllamaClient`): `/api/tags`, `/api/show`,
//! `/api/chat` (streamed NDJSON with `think`, `format` JSON schema and
//! `options.num_ctx/temperature`), `/api/generate`, `/api/embed` and
//! `/api/pull` (streamed progress).
//!
//! The parsers are plain functions over readers so they're tested on canned
//! bodies; the client itself is behind the `ollama` feature.

use super::{policy, split_inline_thinking, ChatDelta, ChatReply, LlmModel, PullProgress};
use crate::error::{CoreError, Result};
use serde_json::Value;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// An `/api/tags` entry is local when it isn't a cloud model by name and
/// carries no remote host (Ollama proxies those to ollama.com).
pub fn is_local_entry(entry: &Value) -> bool {
    let name = entry
        .get("name")
        .or_else(|| entry.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if name.trim().is_empty() || policy::is_cloud_model(name) {
        return false;
    }
    for key in ["remote_host", "remote_model"] {
        if entry
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            return false;
        }
    }
    true
}

/// Embedding-only models by name or family (`bert`, `nomic-bert`).
pub fn looks_like_embedding(name: &str, family: Option<&str>) -> bool {
    let n = name.to_ascii_lowercase();
    let fam = family.unwrap_or("").to_ascii_lowercase();
    n.contains("embed") || n.contains("minilm") || n.contains("bge-") || fam.contains("bert")
}

/// Parse an `/api/tags` body into local models.
pub fn parse_tags(json: &Value) -> Vec<LlmModel> {
    json.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter(|m| is_local_entry(m))
                .filter_map(|m| {
                    let name = m.get("name").and_then(Value::as_str)?.trim().to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let family = m
                        .get("details")
                        .and_then(|d| d.get("family"))
                        .and_then(Value::as_str);
                    Some(LlmModel {
                        embedding: looks_like_embedding(&name, family),
                        size: m.get("size").and_then(Value::as_u64),
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `capabilities` from an `/api/show` body.
pub fn parse_capabilities(json: &Value) -> Option<Vec<String>> {
    json.get("capabilities").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

/// Read a streamed `/api/chat` body: one JSON object per line, the reply in
/// `message.content` and a thinking model's reasoning in `message.thinking`.
/// Inline `<think>` blocks in the content are moved to the reasoning.
pub fn read_chat_stream(
    reader: impl BufRead,
    mut on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    cancel: &AtomicBool,
) -> Result<ChatReply> {
    let mut content = String::new();
    let mut thinking = String::new();
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Llm(super::CANCELLED.into()));
        }
        let line = line.map_err(|e| stream_io_error("Ollama", e))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(err) = json
            .get("error")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
        {
            return Err(CoreError::Llm(format!("Ollama error: {err}")));
        }
        if let Some(message) = json.get("message") {
            if let Some(piece) = message
                .get("thinking")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            {
                thinking.push_str(piece);
                if let Some(cb) = on_delta.as_mut() {
                    cb(ChatDelta::Reasoning(piece.to_string()));
                }
            }
            if let Some(piece) = message
                .get("content")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            {
                content.push_str(piece);
                if let Some(cb) = on_delta.as_mut() {
                    cb(ChatDelta::Content(piece.to_string()));
                }
            }
        } else if let Some(piece) = json
            .get("response")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
        {
            content.push_str(piece);
            if let Some(cb) = on_delta.as_mut() {
                cb(ChatDelta::Content(piece.to_string()));
            }
        }
        if json.get("done").and_then(Value::as_bool) == Some(true) {
            break;
        }
    }
    finish_reply("Ollama", content, thinking)
}

/// Build the final reply: inline `<think>` moves to reasoning; an empty
/// answer is an error that says whether reasoning came back.
pub(crate) fn finish_reply(provider: &str, content: String, thinking: String) -> Result<ChatReply> {
    let (visible, inline) = split_inline_thinking(&content);
    let mut reasoning = thinking;
    if let Some(t) = inline {
        if !reasoning.is_empty() && !t.trim().is_empty() {
            reasoning.push('\n');
        }
        reasoning.push_str(&t);
    }
    let reply = visible.trim().to_string();
    if reply.is_empty() {
        let note = if reasoning.trim().is_empty() {
            ""
        } else {
            " Thinking content was present, but no final response was returned."
        };
        return Err(CoreError::Llm(format!(
            "{provider} returned an empty chat response.{note}"
        )));
    }
    let reasoning = reasoning.trim().to_string();
    Ok(ChatReply {
        content: reply,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
    })
}

/// Read a streamed `/api/pull` body, reporting each status line.
pub fn read_pull_stream(
    reader: impl BufRead,
    model: &str,
    on_progress: &mut dyn FnMut(PullProgress),
) -> Result<()> {
    let mut received = false;
    let mut success = false;
    for line in reader.lines() {
        let line = line.map_err(|e| stream_io_error("Ollama", e))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(err) = json
            .get("error")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
        {
            return Err(CoreError::Llm(format!("Ollama error: {err}")));
        }
        let status = json
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Downloading model...")
            .to_string();
        let lower = status.to_ascii_lowercase();
        if lower.contains("success") || lower.contains("complete") {
            success = true;
        }
        on_progress(PullProgress {
            model: model.to_string(),
            status,
            completed: any_u64(json.get("completed")),
            total: any_u64(json.get("total")),
        });
        received = true;
    }
    if !received {
        return Err(CoreError::Llm("Invalid response from Ollama.".into()));
    }
    if !success {
        on_progress(PullProgress {
            model: model.to_string(),
            status: "Model download finished.".into(),
            completed: None,
            total: None,
        });
    }
    Ok(())
}

/// A read error mid-stream: a timeout (worded so the breaker sees it) or a drop.
pub(crate) fn stream_io_error(provider: &str, e: std::io::Error) -> CoreError {
    let text = e.to_string();
    if e.kind() == std::io::ErrorKind::TimedOut || text.to_ascii_lowercase().contains("timeout") {
        CoreError::Llm(format!("{provider} timed out while streaming"))
    } else {
        CoreError::Llm(format!("{provider} stream failed: {text}"))
    }
}

fn any_u64(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// `embeddings` (or a single `embedding`) from an `/api/embed` body.
pub fn parse_embeddings(json: &Value) -> Result<Vec<Vec<f32>>> {
    let rows: Vec<&Value> = if let Some(e) = json.get("embeddings").and_then(Value::as_array) {
        e.iter().collect()
    } else if let Some(e) = json.get("embedding") {
        vec![e]
    } else {
        return Err(CoreError::Llm("Invalid response from Ollama.".into()));
    };
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let v: Vec<f32> = row
                .as_array()?
                .iter()
                .filter_map(|x| match x {
                    Value::Number(n) => n.as_f64().map(|f| f as f32),
                    Value::String(s) => s.parse::<f32>().ok(),
                    _ => None,
                })
                .collect();
            (!v.is_empty()).then_some(v)
        })
        .collect())
}

/// `options` for chat/generate: Mac bounds on `num_predict` and `num_ctx`.
pub fn request_options(opts: &super::ChatOptions, stop: &[String]) -> Value {
    let mut o = serde_json::Map::new();
    o.insert(
        "temperature".into(),
        serde_json::json!(opts.temperature.unwrap_or(0.0)),
    );
    if let Some(n) = opts.max_tokens {
        o.insert("num_predict".into(), serde_json::json!(n.clamp(32, 16_000)));
    }
    if let Some(n) = opts.num_ctx {
        o.insert("num_ctx".into(), serde_json::json!(n.clamp(2_048, 262_144)));
    }
    if !stop.is_empty() {
        o.insert("stop".into(), serde_json::json!(stop));
    }
    Value::Object(o)
}

#[cfg(feature = "ollama")]
pub use client::OllamaClient;

#[cfg(feature = "ollama")]
mod client {
    use super::super::http::{self, Ctx};
    use super::super::{
        policy, ChatDelta, ChatMessage, ChatOptions, ChatReply, GenerateRequest, LlmClient,
        LlmModel, PullProgress,
    };
    use super::*;
    use std::io::BufReader;

    /// Ollama on this computer or the local network.
    pub struct OllamaClient {
        endpoint: String,
    }

    impl OllamaClient {
        pub fn new(endpoint: &str) -> Self {
            let trimmed = endpoint.trim().trim_end_matches('/');
            let trimmed = trimmed.strip_suffix("/api").unwrap_or(trimmed);
            Self {
                endpoint: if trimmed.is_empty() {
                    DEFAULT_ENDPOINT.to_string()
                } else {
                    trimmed.to_string()
                },
            }
        }

        fn ctx(&self, timeout_ms: Option<u64>) -> Result<Ctx<'_>> {
            policy::require_local_endpoint(&self.endpoint, "Ollama")?;
            Ok(Ctx {
                provider: "Ollama",
                endpoint: &self.endpoint,
                timeout_ms,
            })
        }

        fn url(&self, path: &str) -> String {
            http::url(&self.endpoint, path)
        }

        fn model_name(model: &str) -> Result<String> {
            let m = model.trim();
            if m.is_empty() {
                return Err(CoreError::Llm("Missing Ollama model name.".into()));
            }
            policy::require_local_model(m, "Ollama")?;
            Ok(m.to_string())
        }
    }

    impl LlmClient for OllamaClient {
        fn provider_name(&self) -> &str {
            "ollama"
        }

        fn endpoint(&self) -> &str {
            &self.endpoint
        }

        fn list_models(&self) -> Result<Vec<LlmModel>> {
            let ctx = self.ctx(Some(2_400))?;
            let json = http::get_json(&ctx, &self.url("/api/tags"))?;
            Ok(parse_tags(&json))
        }

        fn model_capabilities(&self, model: &str) -> Option<Vec<String>> {
            if policy::is_cloud_model(model) {
                return None;
            }
            let ctx = self.ctx(Some(2_000)).ok()?;
            let json = http::post_json_value(
                &ctx,
                &self.url("/api/show"),
                &serde_json::json!({ "model": model }),
            )
            .ok()?;
            parse_capabilities(&json)
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
            let model = Self::model_name(&opts.model)?;
            let msgs: Vec<Value> = messages
                .iter()
                .filter(|m| !m.content.trim().is_empty())
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content.trim() }))
                .collect();
            if msgs.is_empty() {
                return Err(CoreError::Llm("Missing chat messages for Ollama.".into()));
            }
            let mut payload = serde_json::json!({
                "model": model,
                "messages": msgs,
                "stream": true,
                "options": request_options(opts, &[]),
                "keep_alive": format!("{}s", opts.keep_alive_secs.unwrap_or(300)),
            });
            if let Some(think) = opts.think {
                payload["think"] = Value::Bool(think);
            }
            if let Some(schema) = &opts.json_schema {
                payload["format"] = schema.clone();
            }
            let ctx = self.ctx(opts.timeout_ms)?;
            let resp = http::post_json(&ctx, &self.url("/api/chat"), &payload)?;
            if !resp.is_success() {
                return Err(ctx.status_error(resp));
            }
            read_chat_stream(BufReader::new(resp.body), on_delta, cancel)
        }

        fn generate(&self, req: &GenerateRequest, opts: &ChatOptions) -> Result<String> {
            let model = Self::model_name(&opts.model)?;
            let prompt = if req.raw {
                req.prompt.clone()
            } else {
                req.prompt.trim().to_string()
            };
            if prompt.trim().is_empty() {
                return Err(CoreError::Llm("Missing prompt content for Ollama.".into()));
            }
            let mut payload = serde_json::json!({
                "model": model,
                "prompt": prompt,
                "stream": false,
                "raw": req.raw,
                "options": request_options(opts, &req.stop),
                "keep_alive": format!("{}s", opts.keep_alive_secs.unwrap_or(300)),
            });
            if !req.raw {
                if let Some(think) = opts.think {
                    payload["think"] = Value::Bool(think);
                }
            }
            if let Some(schema) = &opts.json_schema {
                payload["format"] = schema.clone();
            }
            let ctx = self.ctx(opts.timeout_ms)?;
            let json = http::post_json_value(&ctx, &self.url("/api/generate"), &payload)?;
            let text = json
                .get("response")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                let thinking = json
                    .get("thinking")
                    .and_then(Value::as_str)
                    .is_some_and(|t| !t.is_empty());
                let reason = json
                    .get("done_reason")
                    .and_then(Value::as_str)
                    .map(|r| format!(" done_reason={r}."))
                    .unwrap_or_default();
                let note = if thinking {
                    " Thinking content was present, but no final response was returned."
                } else {
                    ""
                };
                return Err(CoreError::Llm(format!(
                    "Ollama returned an empty generation response.{reason}{note}"
                )));
            }
            Ok(text)
        }

        fn embed(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            let model = Self::model_name(model)?;
            let inputs: Vec<&str> = inputs
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if inputs.is_empty() {
                return Err(CoreError::Llm("Missing text for Ollama embeddings.".into()));
            }
            let payload = serde_json::json!({
                "model": model,
                "input": inputs,
                "truncate": true,
                "keep_alive": "5m",
            });
            let ctx = self.ctx(Some(120_000))?;
            let json = http::post_json_value(&ctx, &self.url("/api/embed"), &payload)?;
            let rows = parse_embeddings(&json)?;
            if rows.len() != inputs.len() {
                return Err(CoreError::Llm(format!(
                    "Ollama returned {} embeddings for {} input(s).",
                    rows.len(),
                    inputs.len()
                )));
            }
            Ok(rows)
        }

        fn pull_model(&self, model: &str, on_progress: &mut dyn FnMut(PullProgress)) -> Result<()> {
            let model = Self::model_name(model)?;
            let ctx = self.ctx(Some(3_600_000))?;
            let resp = http::post_json(
                &ctx,
                &self.url("/api/pull"),
                &serde_json::json!({ "model": model, "stream": true }),
            )?;
            if !resp.is_success() {
                return Err(ctx.status_error(resp));
            }
            read_pull_stream(BufReader::new(resp.body), &model, on_progress)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn tags_filter_cloud_and_remote() {
        let json: Value = serde_json::from_str(
            r#"{"models":[
                {"name":"gemma4:e2b","size":7162000000,"details":{"family":"gemma3"}},
                {"name":"gpt-oss:120b-cloud","size":384},
                {"name":"kimi-k2:latest","remote_host":"https://ollama.com:443"},
                {"name":"all-minilm:latest","details":{"family":"bert"}},
                {"name":"  "}
            ]}"#,
        )
        .unwrap();
        let models = parse_tags(&json);
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["gemma4:e2b", "all-minilm:latest"]);
        assert!(!models[0].embedding);
        assert!(models[1].embedding);
        assert_eq!(models[0].size, Some(7_162_000_000));
    }

    #[test]
    fn chat_stream_with_thinking() {
        let body = concat!(
            "{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"thinking\":\"Let me think.\"},\"done\":false}\n",
            "\n",
            "not json\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"done\":false}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\" world\"},\"done\":false}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\"}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"ignored\"}}\n",
        );
        let mut seen = Vec::new();
        let mut cb = |d: ChatDelta| seen.push(d);
        let reply =
            read_chat_stream(Cursor::new(body), Some(&mut cb), &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.content, "Hello world");
        assert_eq!(reply.reasoning.as_deref(), Some("Let me think."));
        assert_eq!(
            seen,
            vec![
                ChatDelta::Reasoning("Let me think.".into()),
                ChatDelta::Content("Hello".into()),
                ChatDelta::Content(" world".into()),
            ]
        );
    }

    #[test]
    fn chat_stream_inline_think_and_errors() {
        let body = "{\"message\":{\"content\":\"<think>plan</think>Answer\"},\"done\":true}\n";
        let r = read_chat_stream(Cursor::new(body), None, &AtomicBool::new(false)).unwrap();
        assert_eq!(r.content, "Answer");
        assert_eq!(r.reasoning.as_deref(), Some("plan"));

        let err = read_chat_stream(
            Cursor::new("{\"error\":\"model \\\"x\\\" not found\"}\n"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));

        let empty = read_chat_stream(
            Cursor::new("{\"message\":{\"thinking\":\"hmm\"},\"done\":true}\n"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(empty.to_string().contains("Thinking content was present"));

        let cancelled = read_chat_stream(
            Cursor::new("{\"message\":{\"content\":\"x\"}}\n"),
            None,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(
            super::super::classify_error(&cancelled),
            super::super::LlmFailure::Cancelled
        );
    }

    #[test]
    fn pull_progress_stream() {
        let body = concat!(
            "{\"status\":\"pulling manifest\"}\n",
            "{\"status\":\"pulling 6a0746a1ec1a\",\"digest\":\"sha256:6a07\",\"total\":2000,\"completed\":500}\n",
            "{\"status\":\"verifying sha256 digest\"}\n",
            "{\"status\":\"success\"}\n",
        );
        let mut events = Vec::new();
        read_pull_stream(Cursor::new(body), "all-minilm", &mut |p| events.push(p)).unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[1].fraction(), Some(0.25));
        assert_eq!(events[3].status, "success");

        let err = read_pull_stream(
            Cursor::new("{\"error\":\"pull model manifest: file does not exist\"}\n"),
            "nope",
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
        assert!(read_pull_stream(Cursor::new(""), "x", &mut |_| {}).is_err());

        let mut last = None;
        read_pull_stream(
            Cursor::new("{\"status\":\"writing manifest\"}\n"),
            "x",
            &mut |p| last = Some(p.status),
        )
        .unwrap();
        assert_eq!(last.as_deref(), Some("Model download finished."));
    }

    #[test]
    fn embeddings_and_capabilities() {
        let json: Value =
            serde_json::from_str(r#"{"embeddings":[[0.1,0.2],[0.3,"0.4"]]}"#).unwrap();
        assert_eq!(
            parse_embeddings(&json).unwrap(),
            vec![vec![0.1, 0.2], vec![0.3, 0.4]]
        );
        let single: Value = serde_json::from_str(r#"{"embedding":[1,2,3]}"#).unwrap();
        assert_eq!(parse_embeddings(&single).unwrap().len(), 1);
        assert!(parse_embeddings(&serde_json::json!({})).is_err());

        let show: Value =
            serde_json::from_str(r#"{"capabilities":["completion","thinking"]}"#).unwrap();
        assert_eq!(
            parse_capabilities(&show).unwrap(),
            vec!["completion".to_string(), "thinking".to_string()]
        );
    }

    #[test]
    fn options_are_bounded() {
        let opts = super::super::ChatOptions {
            temperature: Some(0.35),
            max_tokens: Some(8),
            num_ctx: Some(1_000_000),
            ..super::super::ChatOptions::model("m")
        };
        let o = request_options(&opts, &["<|im_end|>".to_string()]);
        assert_eq!(o["num_predict"], 32);
        assert_eq!(o["num_ctx"], 262_144);
        assert_eq!(o["stop"][0], "<|im_end|>");
        assert!((o["temperature"].as_f64().unwrap() - 0.35).abs() < 1e-6);
    }
}
