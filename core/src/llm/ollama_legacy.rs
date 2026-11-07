//! Minimal Ollama client (behind the `ollama` feature) for local AI features —
//! transcript polish, suggestions, and the Chat AI panel. HTTP via `ureq`; all
//! local-first. Degrades with a clear error when no Ollama instance is running.

use crate::error::{CoreError, Result};
use serde::Deserialize;

fn base(endpoint: &str) -> String {
    endpoint.trim_end_matches('/').to_string()
}

/// One-shot generation against Ollama's `/api/generate` (non-streaming).
pub fn generate(endpoint: &str, model: &str, prompt: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct GenResponse {
        response: String,
    }

    let url = format!("{}/api/generate", base(endpoint));
    let payload = serde_json::json!({ "model": model, "prompt": prompt, "stream": false });
    let parsed: GenResponse = ureq::post(&url)
        .send_json(payload)
        .map_err(|e| CoreError::Llm(format!("request failed (is Ollama running?): {e}")))?
        .into_body()
        .read_json()
        .map_err(|e| CoreError::Llm(format!("bad response: {e}")))?;
    Ok(parsed.response.trim().to_string())
}

/// Locally installed Ollama model names (`/api/tags`).
pub fn list_models(endpoint: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }

    let url = format!("{}/api/tags", base(endpoint));
    let parsed: Tags = ureq::get(&url)
        .call()
        .map_err(|e| CoreError::Llm(format!("request failed (is Ollama running?): {e}")))?
        .into_body()
        .read_json()
        .map_err(|e| CoreError::Llm(format!("bad response: {e}")))?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}
