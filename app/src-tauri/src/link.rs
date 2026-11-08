//! Orttaai Link in the app: the sync engine that keeps this PC in step with
//! Orttaai on a Mac, and the commands behind the Devices page.

use orttaai_core::store::Store;
use orttaai_core::sync::model::{DeviceKind, DeviceTotals, RecordKind};
use orttaai_core::sync::{self, DeviceInfo, DiscoveredMac, LinkEngine, LinkStatus};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub struct LinkState(pub LinkEngine);

fn database_path() -> Result<std::path::PathBuf, String> {
    Ok(Store::data_dir()
        .map_err(|e| e.to_string())?
        .join("history.db"))
}

/// What this PC says about itself to the Mac.
pub fn device_info() -> DeviceInfo {
    DeviceInfo {
        name: computer_name(),
        model: None,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_version: os_version(),
    }
}

fn computer_name() -> String {
    let name = if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .ok()
    };
    name.map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "This PC".into())
}

fn os_version() -> String {
    if cfg!(windows) {
        return "Windows".into();
    }
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "Linux".into())
}

/// Starts the engine at launch; it waits quietly until this PC is linked.
pub fn start(app: &AppHandle) {
    let Ok(path) = database_path() else { return };
    // Make sure the database (and its sync tables) exists before the engine opens it.
    if Store::open(&path).is_err() {
        return;
    }
    let changes = app.clone();
    let statuses = app.clone();
    let engine = LinkEngine::start(
        path,
        device_info(),
        move |result| {
            if result.kinds.contains(&RecordKind::Dictation)
                || result.kinds.contains(&RecordKind::Device)
            {
                let _ = changes.emit("history-changed", ());
            }
            if result.kinds.iter().any(|k| {
                matches!(
                    k,
                    RecordKind::DictionaryEntry
                        | RecordKind::Snippet
                        | RecordKind::LearningSuggestion
                )
            }) {
                let _ = changes.emit("memory-changed", ());
            }
            if result.kinds.contains(&RecordKind::WritingInsight) {
                let _ = changes.emit("writing-insights-updated", ());
            }
        },
        move |status| {
            let _ = statuses.emit("link-status", status);
        },
    );
    app.manage(LinkState(engine));
}

fn engine(app: &AppHandle) -> Result<LinkEngine, String> {
    app.try_state::<LinkState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| "Sync isn't running".to_string())
}

#[tauri::command]
pub fn link_status(app: AppHandle) -> Result<LinkStatus, String> {
    let engine = engine(&app)?;
    engine.refresh_status();
    Ok(engine.status())
}

#[tauri::command]
pub async fn link_discover() -> Result<Vec<DiscoveredMac>, String> {
    tauri::async_runtime::spawn_blocking(|| sync::discover(Duration::from_secs(5)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn link_with_code(
    app: AppHandle,
    mac: DiscoveredMac,
    code: String,
) -> Result<String, String> {
    let linked = tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&database_path()?).map_err(|e| e.to_string())?;
        sync::link_with_code(&store, &mac, &code, &device_info()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    engine(&app)?.relinked();
    Ok(linked.name)
}

#[tauri::command]
pub async fn link_with_invite(app: AppHandle, text: String) -> Result<String, String> {
    let linked = tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&database_path()?).map_err(|e| e.to_string())?;
        sync::link_with_invite(&store, &text, &device_info()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    engine(&app)?.relinked();
    Ok(linked.name)
}

#[tauri::command]
pub fn link_unlink(app: AppHandle) -> Result<(), String> {
    let store = Store::open(&database_path()?).map_err(|e| e.to_string())?;
    store.sync_unlink().map_err(|e| e.to_string())?;
    engine(&app)?.relinked();
    Ok(())
}

#[tauri::command]
pub fn link_sync_now(app: AppHandle) -> Result<(), String> {
    engine(&app)?.nudge();
    Ok(())
}

/// A device in this archive, for the Devices page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub app: String,
    pub this_device: bool,
    pub totals: DeviceTotals,
    /// ms since 1970.
    pub last_seen_at: i64,
}

#[tauri::command]
pub fn link_devices() -> Result<Vec<DeviceView>, String> {
    let store = Store::open(&database_path()?).map_err(|e| e.to_string())?;
    let me = store.sync_get("device_id").map_err(|e| e.to_string())?;
    let mut devices: Vec<DeviceView> = store
        .sync_devices()
        .map_err(|e| e.to_string())?
        .into_values()
        .map(|d| {
            let this_device = me.as_deref() == Some(d.device_id.as_str());
            DeviceView {
                // Counted here for every device, so the numbers match History.
                totals: store.device_totals(&d.device_id).unwrap_or(d.totals),
                device_id: d.device_id,
                name: d.name,
                kind: d.kind,
                app: d.app,
                this_device,
                last_seen_at: d.last_seen_at,
            }
        })
        .collect();
    devices.sort_by(|a, b| {
        b.this_device
            .cmp(&a.this_device)
            .then(b.totals.words.cmp(&a.totals.words))
    });
    Ok(devices)
}
