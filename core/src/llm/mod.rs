//! Local LLM access for every AI feature (polish, voice edit, Chat AI, writing
//! insights, tone of voice, semantic memory, Coach).
//!
//! Everything talks to the [`LlmClient`] trait, which is always compiled, so
//! feature logic is unit-testable with [`MockLlm`]. The real HTTP clients
//! (Ollama, LM Studio) live behind the `ollama` feature. Only local endpoints
//! are allowed — see [`policy`].

use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub mod policy;

#[cfg(feature = "ollama")]
mod ollama_legacy;
#[cfg(feature = "ollama")]
pub use ollama_legacy::{generate, list_models};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(c: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: c.into(),
        }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: c.into(),
        }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: c.into(),
        }
    }
}

/// Per-request options. `None` fields use the server's defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatOptions {
    pub model: String,
    pub temperature: Option<f32>,
    /// Ask reasoning models to think (`Some(false)` disables it where supported).
    pub think: Option<bool>,
    /// Constrain output to this JSON schema (structured output).
    pub json_schema: Option<serde_json::Value>,
    pub max_tokens: Option<u32>,
    /// Context window (Ollama `num_ctx`).
    pub num_ctx: Option<u32>,
    /// Whole-request timeout.
    pub timeout_ms: Option<u64>,
}

impl ChatOptions {
    pub fn model(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }
}

/// A streamed piece of a reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum ChatDelta {
    Content(String),
    Reasoning(String),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChatReply {
    pub content: String,
    pub reasoning: Option<String>,
}

/// A model installed on the local server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmModel {
    pub name: String,
    /// Bytes on disk, when the server reports it.
    pub size: Option<u64>,
    /// True for embedding-only models.
    pub embedding: bool,
}

/// A local LLM server. Implementations must be cheap to clone/share.
pub trait LlmClient: Send + Sync {
    /// `"ollama"`, `"lmstudio"`, `"mock"` …
    fn provider_name(&self) -> &str;

    /// Installed models.
    fn list_models(&self) -> Result<Vec<LlmModel>>;

    /// Run a chat completion. When `on_delta` is given the reply is streamed
    /// through it as it arrives; the full reply is returned either way.
    fn chat(
        &self,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    ) -> Result<ChatReply>;

    /// Embed each input with `model`.
    fn embed(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Download `model` onto the server (Ollama `/api/pull`), reporting the
    /// fraction done, 0 to 1. Servers that can't install models keep the default.
    fn pull(&self, model: &str, on_progress: &mut dyn FnMut(f32)) -> Result<()> {
        let _ = (model, on_progress);
        Err(CoreError::NotImplemented(
            "downloading models from this server",
        ))
    }

    /// One-shot helper: system + user prompt → reply text.
    fn complete(&self, system: &str, user: &str, opts: &ChatOptions) -> Result<String> {
        let mut msgs = Vec::new();
        if !system.is_empty() {
            msgs.push(ChatMessage::system(system));
        }
        msgs.push(ChatMessage::user(user));
        Ok(self.chat(&msgs, opts, None)?.content)
    }
}

/// Strip a ```json fence (or any prose before the first `{`/`[`) and parse.
pub fn parse_json_reply<T: serde::de::DeserializeOwned>(reply: &str) -> Result<T> {
    let trimmed = reply.trim();
    let start = trimmed.find(['{', '[']).unwrap_or(0);
    let end = trimmed
        .rfind(['}', ']'])
        .map(|i| i + 1)
        .unwrap_or(trimmed.len());
    let body = if start < end {
        &trimmed[start..end]
    } else {
        trimmed
    };
    serde_json::from_str(body)
        .map_err(|e| CoreError::Llm(format!("model returned invalid JSON: {e}")))
}

type Responder = Box<dyn Fn(&[ChatMessage], &ChatOptions) -> Result<String> + Send + Sync>;

/// Deterministic client for tests. Replies come from a closure; embeddings are
/// a cheap bag-of-letters vector so similarity behaves sensibly.
pub struct MockLlm {
    responder: Responder,
    models: Vec<LlmModel>,
    /// Every request seen, for assertions.
    pub calls: Mutex<Vec<(Vec<ChatMessage>, ChatOptions)>>,
}

impl MockLlm {
    /// Always reply with `text`.
    pub fn fixed(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::with(move |_, _| Ok(text.clone()))
    }

    /// Reply via a closure over the request.
    pub fn with(
        f: impl Fn(&[ChatMessage], &ChatOptions) -> Result<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            responder: Box::new(f),
            models: vec![
                LlmModel {
                    name: "mock-chat".into(),
                    size: None,
                    embedding: false,
                },
                LlmModel {
                    name: "mock-embed".into(),
                    size: None,
                    embedding: true,
                },
            ],
            calls: Mutex::new(Vec::new()),
        }
    }

    /// A client whose every call fails (server down).
    pub fn offline() -> Self {
        Self::with(|_, _| Err(CoreError::Llm("connection refused (mock offline)".into())))
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl LlmClient for MockLlm {
    fn provider_name(&self) -> &str {
        "mock"
    }

    fn list_models(&self) -> Result<Vec<LlmModel>> {
        Ok(self.models.clone())
    }

    fn chat(
        &self,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    ) -> Result<ChatReply> {
        self.calls
            .lock()
            .unwrap()
            .push((messages.to_vec(), opts.clone()));
        let text = (self.responder)(messages, opts)?;
        if let Some(cb) = on_delta {
            for word in text.split_inclusive(' ') {
                cb(ChatDelta::Content(word.to_string()));
            }
        }
        Ok(ChatReply {
            content: text,
            reasoning: None,
        })
    }

    fn embed(&self, _model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|s| letter_vector(s)).collect())
    }
}

/// 26-dim normalized letter-frequency vector (mock embeddings).
fn letter_vector(s: &str) -> Vec<f32> {
    let mut v = vec![0f32; 26];
    for c in s.chars().filter(|c| c.is_ascii_alphabetic()) {
        v[(c.to_ascii_lowercase() as u8 - b'a') as usize] += 1.0;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_reply_strips_fences() {
        #[derive(Deserialize)]
        struct T {
            a: i32,
        }
        let t: T = parse_json_reply("```json\n{\"a\": 3}\n```").unwrap();
        assert_eq!(t.a, 3);
        assert!(parse_json_reply::<T>("nope").is_err());
    }

    #[test]
    fn mock_streams_and_records() {
        let m = MockLlm::fixed("hello big world");
        let mut seen = String::new();
        let mut cb = |d: ChatDelta| {
            if let ChatDelta::Content(t) = d {
                seen.push_str(&t)
            }
        };
        let r = m
            .chat(
                &[ChatMessage::user("hi")],
                &ChatOptions::model("x"),
                Some(&mut cb),
            )
            .unwrap();
        assert_eq!(r.content, "hello big world");
        assert_eq!(seen, "hello big world");
        assert_eq!(m.call_count(), 1);
        assert!(MockLlm::offline()
            .complete("", "x", &ChatOptions::default())
            .is_err());
    }
}
