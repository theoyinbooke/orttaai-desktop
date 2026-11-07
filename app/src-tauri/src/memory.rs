//! Dictionary, snippets and learning-suggestion commands. (Text-pipeline area.)

use crate::err;
use orttaai_core::store::{DictionaryEntry, SnippetEntry, Store};
use tauri::{AppHandle, Emitter};

/// Tell the engine (and other windows) that memory changed so the live
/// dictation pipeline reloads it without a restart.
fn changed(app: &AppHandle) {
    let _ = app.emit("memory-changed", ());
}

#[tauri::command]
pub fn list_dictionary() -> Result<Vec<DictionaryEntry>, String> {
    Store::open_default()
        .and_then(|s| s.list_dictionary())
        .map_err(err)
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
    let store = Store::open_default().map_err(err)?;
    match id {
        Some(id) => {
            store.update_dictionary(id, &source, &target, case_sensitive, active.unwrap_or(true))
        }
        None => store
            .upsert_dictionary(&source, &target, case_sensitive)
            .map(|_| ()),
    }
    .map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_dictionary(app: AppHandle, id: i64) -> Result<(), String> {
    Store::open_default()
        .and_then(|s| s.delete_dictionary(id))
        .map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn list_snippets() -> Result<Vec<SnippetEntry>, String> {
    Store::open_default()
        .and_then(|s| s.list_snippets())
        .map_err(err)
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
    let store = Store::open_default().map_err(err)?;
    match id {
        Some(id) => store.update_snippet(id, &trigger, &expansion, active.unwrap_or(true)),
        None => store.upsert_snippet(&trigger, &expansion).map(|_| ()),
    }
    .map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_snippet(app: AppHandle, id: i64) -> Result<(), String> {
    Store::open_default()
        .and_then(|s| s.delete_snippet(id))
        .map_err(err)?;
    changed(&app);
    Ok(())
}
