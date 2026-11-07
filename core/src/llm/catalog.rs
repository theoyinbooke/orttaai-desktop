//! Curated local models (Mac `OllamaClient.curated*Models`) and the S1-mini
//! polish model, which the desktop serves through the local LLM host.

use crate::settings::LlmProvider;
use serde::Serialize;

/// What a curated model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Chat,
    Embedding,
}

/// One curated download.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogModel {
    pub name: &'static str,
    pub size: Option<u64>,
    pub kind: CatalogKind,
}

const fn chat(name: &'static str, size: Option<u64>) -> CatalogModel {
    CatalogModel {
        name,
        size,
        kind: CatalogKind::Chat,
    }
}

const fn embedding(name: &'static str, size: Option<u64>) -> CatalogModel {
    CatalogModel {
        name,
        size,
        kind: CatalogKind::Embedding,
    }
}

/// Lightweight chat models for polish, insights, chat (Mac order).
pub const CURATED_CHAT: &[CatalogModel] = &[
    chat("gemma4:e2b", Some(7_162_000_000)),
    chat("gemma4:e4b", Some(9_608_000_000)),
    chat("gemma3:1b", None),
    chat("gemma3:4b", Some(8_600_000_000)),
    chat("qwen3.5:0.8b", None),
    chat("qwen3.5:2b", None),
    chat("qwen3.5:4b", None),
    chat("ministral-3:3b", Some(4_670_000_000)),
    chat("granite4:1b", None),
    chat("granite4:3b", None),
];

/// Embedding models for semantic memory (Mac order).
pub const CURATED_EMBEDDING: &[CatalogModel] = &[
    embedding("all-minilm", Some(67_000_000)),
    embedding("nomic-embed-text", Some(274_000_000)),
    embedding("embeddinggemma", Some(622_000_000)),
    embedding("qwen3-embedding:0.6b", Some(639_000_000)),
];

/// The full curated catalog, de-duplicated (Mac `fetchLibraryModels`).
pub fn curated() -> Vec<CatalogModel> {
    let mut seen = std::collections::HashSet::new();
    CURATED_CHAT
        .iter()
        .chain(CURATED_EMBEDDING)
        .filter(|m| seen.insert(m.name.to_ascii_lowercase()))
        .cloned()
        .collect()
}

/// Download size from the curated catalog (`:latest` ignored).
pub fn catalog_size(model: &str) -> Option<u64> {
    let target = model.to_ascii_lowercase().replace(":latest", "");
    curated()
        .into_iter()
        .find(|m| m.name.eq_ignore_ascii_case(&target))
        .and_then(|m| m.size)
}

/// Whether `model` is installed among `installed` (exact, or `name:latest`).
pub fn is_installed(model: &str, installed: &[String]) -> bool {
    let want = model.trim().to_ascii_lowercase();
    let want_latest = if want.contains(':') {
        want.clone()
    } else {
        format!("{want}:latest")
    };
    installed.iter().any(|m| {
        let m = m.to_ascii_lowercase();
        m == want || m == want_latest
    })
}

/// The embedding model Orttaai recommends for semantic memory.
pub const RECOMMENDED_EMBEDDING: &str = "nomic-embed-text";

/// What to ask the server to download for the recommended embedding model.
pub fn recommended_embedding_download(p: LlmProvider) -> &'static str {
    match p {
        LlmProvider::Ollama => "nomic-embed-text",
        LlmProvider::LmStudio => "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF",
    }
}

// ---- S1-mini ----------------------------------------------------------------

/// S1-mini by Superwhisper (Qwen3 0.6B fine-tuned for dictation clean-up,
/// Apache 2.0), the Hugging Face repository the Mac downloads.
pub const S1_MINI_REPO: &str = "superwhisper/s1-mini-GGUF";
/// The quantisation the Mac ships (`s1-mini-q4_k_m.gguf`).
pub const S1_MINI_QUANT: &str = "Q4_K_M";
/// Download size of `s1-mini-q4_k_m.gguf`.
pub const S1_MINI_BYTES: u64 = 484_219_808;
/// Credit its licence asks for.
pub const S1_MINI_CREDIT: &str =
    "S1-mini by Superwhisper (Apache 2.0), built on Qwen3 0.6B. It runs on your own computer through your local AI server.";

/// Ollama pulls GGUFs straight from Hugging Face with `hf.co/<repo>:<quant>`.
pub fn s1_mini_ollama_model() -> String {
    format!("hf.co/{S1_MINI_REPO}:{S1_MINI_QUANT}")
}

/// What to ask the provider to download for S1-mini.
pub fn s1_mini_download(p: LlmProvider) -> String {
    match p {
        LlmProvider::Ollama => s1_mini_ollama_model(),
        LlmProvider::LmStudio => format!("https://huggingface.co/{S1_MINI_REPO}"),
    }
}

/// The installed S1-mini, under whatever name the server gave it
/// (`hf.co/superwhisper/s1-mini-GGUF:Q4_K_M`, `s1-mini`, …).
pub fn installed_s1_mini(installed: &[String]) -> Option<String> {
    installed
        .iter()
        .find(|m| m.to_ascii_lowercase().contains("s1-mini"))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lookup() {
        assert_eq!(catalog_size("gemma4:e2b"), Some(7_162_000_000));
        assert_eq!(catalog_size("all-minilm:latest"), Some(67_000_000));
        assert_eq!(catalog_size("nope"), None);
        assert_eq!(
            curated().len(),
            CURATED_CHAT.len() + CURATED_EMBEDDING.len()
        );
        let installed = vec!["all-minilm:latest".to_string(), "gemma4:e2b".to_string()];
        assert!(is_installed("all-minilm", &installed));
        assert!(is_installed("gemma4:e2b", &installed));
        assert!(!is_installed("gemma4:e4b", &installed));
    }

    #[test]
    fn s1_mini_names() {
        assert_eq!(
            s1_mini_ollama_model(),
            "hf.co/superwhisper/s1-mini-GGUF:Q4_K_M"
        );
        assert!(s1_mini_download(LlmProvider::LmStudio).starts_with("https://huggingface.co/"));
        let installed = vec![
            "gemma4:e2b".to_string(),
            "hf.co/superwhisper/s1-mini-GGUF:Q4_K_M".to_string(),
        ];
        assert_eq!(
            installed_s1_mini(&installed).as_deref(),
            Some("hf.co/superwhisper/s1-mini-GGUF:Q4_K_M")
        );
        assert!(installed_s1_mini(&installed[..1]).is_none());
    }
}
