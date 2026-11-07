//! Local AI commands: Chat AI, polish, voice edit, writing insights, tone of
//! voice. (AI area.)

use orttaai_core::settings::Settings;

// ---- Ollama (Chat AI) -------------------------------------------------------

#[tauri::command]
pub async fn ollama_models() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = Settings::load_or_default();
        orttaai_core::llm::list_models(&settings.ollama_endpoint).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn ollama_chat(prompt: String, model: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = Settings::load_or_default();
        let model = if model.trim().is_empty() {
            "llama3.2".to_string()
        } else {
            model
        };
        orttaai_core::llm::generate(&settings.ollama_endpoint, &model, &prompt)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
