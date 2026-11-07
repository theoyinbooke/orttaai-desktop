//! Home, analytics, history, milestones and What's New commands.
//! (Analytics/home area.)

use orttaai_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

// ---- Analytics + Personal Memory --------------------------------------------

#[tauri::command]
pub fn dashboard_stats(days: Option<i64>) -> Result<orttaai_core::store::DashboardStats, String> {
    Store::open_default()
        .and_then(|s| s.stats(days))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_transcription(app: AppHandle, id: i64) -> Result<(), String> {
    Store::open_default()
        .and_then(|s| s.delete_transcription(id))
        .map_err(|e| e.to_string())?;
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[derive(Serialize)]
pub struct HistoryItem {
    id: i64,
    text: String,
    app: Option<String>,
    word_count: i64,
    created_at: i64,
}

#[tauri::command]
pub fn recent_history(limit: i64) -> Result<Vec<HistoryItem>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let records = store
        .recent(limit.clamp(1, 500))
        .map_err(|e| e.to_string())?;
    Ok(records
        .into_iter()
        .map(|r| HistoryItem {
            id: r.id.unwrap_or(0),
            text: r.text,
            app: r.app,
            word_count: r.word_count,
            created_at: r.created_at,
        })
        .collect())
}
