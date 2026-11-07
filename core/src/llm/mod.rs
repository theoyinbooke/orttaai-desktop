//! Local LLM access for every AI feature (polish, voice edit, Chat AI, writing
//! insights, tone of voice, semantic memory, Coach).
//!
//! Everything talks to the [`LlmClient`] trait, which is always compiled, so
//! feature logic is unit-testable with [`MockLlm`]. The real HTTP clients
//! (Ollama, LM Studio) live behind the `ollama` feature. Only local endpoints
//! are allowed — see [`policy`].

use crate::error::{CoreError, Result};
use crate::settings::{LlmProvider, Settings};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub mod catalog;
#[cfg(feature = "ollama")]
mod http;
pub mod lmstudio;
pub mod ollama;
pub mod policy;
#[cfg(all(test, feature = "ollama"))]
mod test_server;

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

    // ---- Additions (all defaulted, so existing implementations keep working) ----

    /// The server address this client talks to (empty for test clients).
    fn endpoint(&self) -> &str {
        ""
    }

    /// Reachability + installed models, with a user-facing message (Mac `checkHealth`).
    fn health(&self) -> LlmHealth {
        match self.list_models() {
            Ok(models) => LlmHealth::reachable(self.provider_name(), models),
            Err(e) => LlmHealth::unreachable(self.provider_name(), &e),
        }
    }

    /// What a model can do as the server reports it (`completion`, `thinking`,
    /// `embedding`, …), or `None` when the server doesn't say.
    fn model_capabilities(&self, _model: &str) -> Option<Vec<String>> {
        None
    }

    /// Prompt-only generation (Mac `generate`). `raw` sends the prompt exactly
    /// as written (already in the model's chat format, e.g. S1-mini); otherwise
    /// the server applies the model's template. Defaults to a one-message chat.
    fn generate(&self, req: &GenerateRequest, opts: &ChatOptions) -> Result<String> {
        self.complete("", &req.prompt, opts)
    }

    /// [`LlmClient::chat`] that stops early (with an error carrying
    /// [`CANCELLED`]) once `cancel` is set. Defaults to an uncancellable chat.
    fn chat_cancellable(
        &self,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        on_delta: Option<&mut dyn FnMut(ChatDelta)>,
        _cancel: &AtomicBool,
    ) -> Result<ChatReply> {
        self.chat(messages, opts, on_delta)
    }

    /// Ask the server to download `model` (Ollama pull / LM Studio download),
    /// reporting progress until it's done.
    fn pull_model(&self, model: &str, _on_progress: &mut dyn FnMut(PullProgress)) -> Result<()> {
        Err(CoreError::Llm(format!(
            "{} can't download {model} from Orttaai.",
            provider_display(self.provider_name())
        )))
    }

    /// Load a model into memory with a tiny request; returns elapsed ms.
    fn warm(&self, model: &str, timeout_ms: u64) -> Result<u64> {
        let started = std::time::Instant::now();
        let opts = ChatOptions {
            temperature: Some(0.0),
            think: Some(false),
            max_tokens: Some(8),
            timeout_ms: Some(timeout_ms),
            ..ChatOptions::model(model)
        };
        self.generate(&GenerateRequest::new("Reply with OK."), &opts)?;
        Ok(started.elapsed().as_millis() as u64)
    }
}

/// A prompt-only request for [`LlmClient::generate`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub prompt: String,
    /// Send the prompt verbatim, without the server's chat template.
    pub raw: bool,
    /// Extra stop sequences.
    pub stop: Vec<String>,
}

impl GenerateRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            ..Default::default()
        }
    }
    pub fn raw(prompt: impl Into<String>, stop: &[&str]) -> Self {
        Self {
            prompt: prompt.into(),
            raw: true,
            stop: stop.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Download progress for [`LlmClient::pull_model`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PullProgress {
    pub model: String,
    pub status: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
}

impl PullProgress {
    /// 0..=1 when both byte counts are known.
    pub fn fraction(&self) -> Option<f64> {
        match (self.completed, self.total) {
            (Some(c), Some(t)) if t > 0 => Some((c as f64 / t as f64).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

/// Mac `OllamaHealthStatus`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlmHealth {
    pub reachable: bool,
    /// Installed model names, sorted.
    pub models: Vec<String>,
    pub message: String,
}

impl LlmHealth {
    pub fn reachable(provider: &str, models: Vec<LlmModel>) -> Self {
        let mut names: Vec<String> = models.into_iter().map(|m| m.name).collect();
        names.sort();
        let server = provider_display(provider);
        let message = if names.is_empty() {
            if provider == "lmstudio" {
                format!("{server} is reachable, but no local models are downloaded yet. Add models in LM Studio.")
            } else {
                format!("{server} is reachable, but no local models are installed yet.")
            }
        } else {
            format!(
                "{server} is reachable. {} local model{} available.",
                names.len(),
                if names.len() == 1 { "" } else { "s" }
            )
        };
        Self {
            reachable: true,
            models: names,
            message,
        }
    }

    pub fn unreachable(provider: &str, e: &CoreError) -> Self {
        let server = provider_display(provider);
        let detail = error_text(e);
        let message = if provider == "lmstudio" {
            format!("{server} unreachable: {detail} Start the server from LM Studio's Developer tab, or run `lms server start`.")
        } else {
            format!("{server} unreachable: {detail}")
        };
        Self {
            reachable: false,
            models: Vec::new(),
            message,
        }
    }
}

/// `"ollama"` → `"Ollama"`, `"lmstudio"` → `"LM Studio"`.
pub fn provider_display(provider: &str) -> &'static str {
    match provider {
        "ollama" => "Ollama",
        "lmstudio" => "LM Studio",
        "mock" => "Mock",
        _ => "Local AI",
    }
}

/// Display name for a configured provider.
pub fn provider_display_name(p: LlmProvider) -> &'static str {
    match p {
        LlmProvider::Ollama => "Ollama",
        LlmProvider::LmStudio => "LM Studio",
    }
}

/// The message inside an LLM error, without the `llm error:` prefix.
pub fn error_text(e: &CoreError) -> String {
    match e {
        CoreError::Llm(m) => m.clone(),
        other => other.to_string(),
    }
}

/// Marker carried by the error a cancelled generation returns.
pub const CANCELLED: &str = "generation stopped";

/// Why a request failed, so callers pick the right circuit-breaker action
/// (Mac `LLMRequestErrorClassifier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFailure {
    /// The server is up but slow.
    Timeout,
    /// No server is listening (connection refused / host down).
    Unreachable,
    /// The server is up but the model isn't installed (HTTP 404).
    ModelMissing,
    /// Stopped by the user.
    Cancelled,
    Other,
}

/// Classify an error produced by the HTTP clients (or `MockLlm`).
pub fn classify_error(e: &CoreError) -> LlmFailure {
    let m = error_text(e).to_ascii_lowercase();
    if m.contains(CANCELLED) {
        LlmFailure::Cancelled
    } else if m.contains("timed out") || m.contains("timeout") {
        LlmFailure::Timeout
    } else if m.contains("unreachable")
        || m.contains("connection refused")
        || m.contains("could not connect")
        || m.contains("host not found")
    {
        LlmFailure::Unreachable
    } else if m.contains("error 404") || m.contains("isn't a local model") {
        LlmFailure::ModelMissing
    } else {
        LlmFailure::Other
    }
}

/// Every balanced top-level `{…}` in `text`, string-aware (Mac
/// `extractJSONObjects`) — models sometimes wrap JSON in prose or emit two.
pub fn extract_json_objects(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut depth, mut start, mut escaped, mut in_string) = (0usize, None, false, false);
    for (i, &c) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && in_string {
            escaped = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == '{' {
            if depth == 0 {
                start = Some(i);
            }
            depth += 1;
        } else if c == '}' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                if let Some(s) = start.take() {
                    out.push(chars[s..=i].iter().collect());
                }
            }
        }
    }
    out
}

/// Remove `<think>…</think>` blocks (reasoning some models inline in the
/// reply). An unclosed `<think>` hides everything after it.
pub fn strip_think_tags(text: &str) -> String {
    split_inline_thinking(text).0
}

/// Split a (possibly partial) reply with inline `<think>` tags into the
/// visible text and the reasoning (Mac `ChatAIReplyStatus.splitInlineThinking`,
/// extended to several blocks).
pub fn split_inline_thinking(text: &str) -> (String, Option<String>) {
    if !text.contains("<think>") {
        return (text.to_string(), None);
    }
    let mut visible = String::new();
    let mut thought = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("<think>") {
        visible.push_str(&rest[..open]);
        let after = &rest[open + "<think>".len()..];
        match after.find("</think>") {
            Some(close) => {
                thought.push_str(&after[..close]);
                rest = &after[close + "</think>".len()..];
            }
            None => {
                thought.push_str(after);
                rest = "";
            }
        }
    }
    visible.push_str(rest);
    (visible, Some(thought))
}

/// The client used when this binary has no local-AI support compiled in.
#[cfg(not(feature = "ollama"))]
struct UnavailableClient;

#[cfg(not(feature = "ollama"))]
impl LlmClient for UnavailableClient {
    fn provider_name(&self) -> &str {
        "unavailable"
    }
    fn list_models(&self) -> Result<Vec<LlmModel>> {
        Err(CoreError::Llm(
            "local AI isn't built into this binary (feature `ollama`)".into(),
        ))
    }
    fn chat(
        &self,
        _messages: &[ChatMessage],
        _opts: &ChatOptions,
        _on_delta: Option<&mut dyn FnMut(ChatDelta)>,
    ) -> Result<ChatReply> {
        self.list_models().map(|_| ChatReply::default())
    }
    fn embed(&self, _model: &str, _inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        self.list_models().map(|_| Vec::new())
    }
}

/// The client for the configured provider and endpoint (Ollama or LM Studio).
/// Only local endpoints and local models are ever used ([`policy`]).
pub fn client_from_settings(s: &Settings) -> Arc<dyn LlmClient> {
    client_for(s.llm_provider, s.llm_endpoint())
}

/// A client for an explicit provider + endpoint (e.g. a connection test
/// before saving).
pub fn client_for(provider: LlmProvider, endpoint: &str) -> Arc<dyn LlmClient> {
    #[cfg(feature = "ollama")]
    {
        match provider {
            LlmProvider::Ollama => Arc::new(ollama::OllamaClient::new(endpoint)),
            LlmProvider::LmStudio => Arc::new(lmstudio::LmStudioClient::new(endpoint)),
        }
    }
    #[cfg(not(feature = "ollama"))]
    {
        let _ = (provider, endpoint);
        Arc::new(UnavailableClient)
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
