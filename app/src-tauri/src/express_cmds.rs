//! Express engine (Parakeet via sherpa-onnx) model commands — the desktop
//! counterpart of the Mac's Express card in Settings › Models.
//!
//! Events: `express-progress` `{id, fraction}`, `express-done` `{id}`,
//! `express-error` `{id, error}`, `express-cancelled` `{id}`.

use crate::err;
use orttaai_core::transcription::express::{self, ExpressModelInfo};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

fn downloads() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static D: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

#[tauri::command]
pub fn express_models() -> Result<Vec<ExpressModelInfo>, String> {
    express::list().map_err(err)
}

/// Download + unpack an Express model in the background.
#[tauri::command]
pub fn express_download(app: AppHandle, id: String) -> Result<(), String> {
    if express::spec(&id).is_none() {
        return Err(format!("Unknown Express model '{id}'."));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut map = downloads().lock().unwrap();
        if map.contains_key(&id) {
            return Ok(()); // already downloading
        }
        map.insert(id.clone(), cancel.clone());
    }
    std::thread::spawn(move || {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let result = express::download(&id, &cancel, move |fraction| {
            let _ = progress_app.emit(
                "express-progress",
                serde_json::json!({ "id": progress_id, "fraction": fraction }),
            );
        });
        downloads().lock().unwrap().remove(&id);
        match result {
            Ok(_) => {
                let _ = app.emit("express-done", serde_json::json!({ "id": id }));
            }
            Err(_) if cancel.load(Ordering::SeqCst) => {
                let _ = app.emit("express-cancelled", serde_json::json!({ "id": id }));
            }
            Err(e) => {
                let _ = app.emit(
                    "express-error",
                    serde_json::json!({ "id": id, "error": e.to_string() }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn express_cancel(id: String) {
    if let Some(flag) = downloads().lock().unwrap().get(&id) {
        flag.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn express_delete(id: String) -> Result<(), String> {
    express::delete(&id).map_err(err)
}
