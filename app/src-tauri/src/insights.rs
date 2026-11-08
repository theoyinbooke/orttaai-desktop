//! Home, analytics, history, milestones and What's New commands.
//! (Analytics/home area.)
//!
//! Events emitted:
//! - `history-changed` — a dictation was deleted here.
//! - `milestone-unlocked` — `Vec<BadgeView>` earned by a dictation in the last
//!   two minutes (highest tier first), for the corner toast. Only sent while
//!   Settings › Celebrate milestones is on.
//! - `milestones-changed` — unlocks or celebrations changed (refresh views).

use crate::err;
use orttaai_core::analytics::{
    self, AnalyticsSnapshot, Clock, DashboardPayload, GithubPromptDecision, TimeRange,
};
use orttaai_core::milestones::{self, BadgeView, Overview};
use orttaai_core::settings::Settings;
use orttaai_core::store::{now_unix, HistoryPage, HistoryQuery, Store, TranscriptionRecord};
use orttaai_core::whatsnew::{self, Decision, Release};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

/// The app, for events raised from the post-save hook (which runs on the
/// dictation worker and has no handle of its own). Every command here that
/// gets one records it; the UI calls them as soon as it loads.
static APP: OnceLock<AppHandle> = OnceLock::new();

fn remember(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Run store work off the main thread.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

fn open() -> Result<Store, String> {
    Store::open_default().map_err(err)
}

fn clock(store: &Store) -> Result<Clock, String> {
    let now = now_unix();
    Ok(Clock {
        now,
        offset: store.local_offset_at(now).map_err(err)?,
    })
}

/// The model the Performance card names, and whether it is Express.
fn current_model(settings: &Settings) -> (Option<String>, bool) {
    if settings.express_active() {
        (Some("Express (Parakeet TDT)".to_string()), true)
    } else {
        (Some(settings.model_id.clone()), false)
    }
}

fn load_payload(store: &Store) -> Result<DashboardPayload, String> {
    let settings = Settings::load_or_default();
    let (model, express) = current_model(&settings);
    let samples = store.analytics_samples(None).map_err(err)?;
    let recent = store.recent(analytics::RECENT_COUNT as i64).map_err(err)?;
    Ok(analytics::dashboard_payload(
        &samples,
        &recent,
        clock(store)?,
        model.as_deref(),
        express,
    ))
}

// ---- Legacy (kept for the CLI-era UI and other areas) ------------------------

#[tauri::command]
pub fn dashboard_stats(days: Option<i64>) -> Result<orttaai_core::store::DashboardStats, String> {
    Store::open_default()
        .and_then(|s| s.stats(days))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_transcription(app: AppHandle, id: i64) -> Result<(), String> {
    remember(&app);
    blocking(move || open()?.delete_transcription(id).map_err(err)).await?;
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

// ---- Home + Analytics ------------------------------------------------------

/// Home (Mac `HomeView`): today, performance, 30-day trend, top apps, recent.
#[tauri::command]
pub async fn home_summary(app: AppHandle) -> Result<DashboardPayload, String> {
    remember(&app);
    blocking(|| load_payload(&open()?)).await
}

/// Analytics › Dashboard for `range` (`30d | 90d | all`; default: the saved
/// `analytics_time_range`).
#[tauri::command]
pub async fn analytics_dashboard(
    app: AppHandle,
    range: Option<String>,
) -> Result<AnalyticsSnapshot, String> {
    remember(&app);
    blocking(move || {
        let settings = Settings::load_or_default();
        let range = TimeRange::parse(range.as_deref().unwrap_or(&settings.analytics_time_range));
        let (model, express) = current_model(&settings);
        let store = open()?;
        let samples = store.analytics_samples(None).map_err(err)?;
        let recent = store.recent(analytics::RECENT_COUNT as i64).map_err(err)?;
        Ok(analytics::analytics_snapshot(
            &samples,
            &recent,
            clock(&store)?,
            range,
            model.as_deref(),
            express,
        ))
    })
    .await
}

/// Whether Home should show the "Enjoying Orttaai?" prompt now; records the
/// showing (at most three, a week apart).
#[tauri::command]
pub async fn github_prompt_check(app: AppHandle) -> Result<bool, String> {
    remember(&app);
    let payload = blocking(|| load_payload(&open()?)).await?;
    let s = Settings::load_or_default();
    let now = now_unix();
    match analytics::github_prompt_decision(
        &payload,
        s.github_star_prompt_completed,
        s.github_star_prompt_shown_count,
        s.github_star_prompt_last_shown_at,
        now,
    ) {
        GithubPromptDecision::Show => {
            crate::settings_cmds::update_settings(
                app,
                serde_json::json!({
                    "github_star_prompt_shown_count": s.github_star_prompt_shown_count + 1,
                    "github_star_prompt_last_shown_at": now,
                }),
            )?;
            Ok(true)
        }
        GithubPromptDecision::Exhausted => {
            crate::settings_cmds::update_settings(
                app,
                serde_json::json!({ "github_star_prompt_completed": true }),
            )?;
            Ok(false)
        }
        GithubPromptDecision::Skip => Ok(false),
    }
}

// ---- History ---------------------------------------------------------------

/// Search/filter/paginate history (newest first).
#[tauri::command]
pub async fn history_page(query: HistoryQuery) -> Result<HistoryPage, String> {
    blocking(move || open()?.history(&query).map_err(err)).await
}

/// App names seen in history, most used first (for the filter).
#[tauri::command]
pub async fn history_apps() -> Result<Vec<String>, String> {
    blocking(|| open()?.history_apps().map_err(err)).await
}

#[tauri::command]
pub async fn get_transcription(id: i64) -> Result<Option<TranscriptionRecord>, String> {
    blocking(move || open()?.get_transcription(id).map_err(err)).await
}

/// Put a dictation's exact text on the clipboard. Returns the text so the UI
/// can also copy it through the webview, which is the reliable path on
/// Windows; on Linux the native tools are tried first.
#[tauri::command]
pub async fn copy_transcription(id: i64) -> Result<String, String> {
    let record = blocking(move || open()?.get_transcription(id).map_err(err))
        .await?
        .ok_or_else(|| "That dictation is no longer in your history.".to_string())?;
    let _ = native_copy(record.text.as_bytes(), None);
    Ok(record.text)
}

// ---- Milestones ------------------------------------------------------------

fn evaluate_now(store: &Store) -> Result<milestones::Evaluation, String> {
    milestones::evaluate(store, now_unix()).map_err(err)
}

/// The Milestones page: every badge with its state and progress.
#[tauri::command]
pub async fn milestones_overview(app: AppHandle) -> Result<Overview, String> {
    remember(&app);
    let (overview, changed) = blocking(|| {
        let e = evaluate_now(&open()?)?;
        Ok((e.overview(), !e.inserted.is_empty()))
    })
    .await?;
    if changed {
        let _ = app.emit("milestones-changed", ());
    }
    Ok(overview)
}

/// Badges earned but not yet celebrated, oldest first. Evaluates first, so
/// at launch everything already earned unlocks with its historical date and
/// waits here for the celebration (never a toast).
#[tauri::command]
pub async fn milestones_pending(app: AppHandle) -> Result<Vec<BadgeView>, String> {
    remember(&app);
    blocking(|| Ok(evaluate_now(&open()?)?.awaiting_celebration())).await
}

/// Re-evaluate now (e.g. after the dictionary changed) and announce anything
/// just earned. Returns the fresh badges.
#[tauri::command]
pub async fn milestones_evaluate(app: AppHandle) -> Result<Vec<BadgeView>, String> {
    remember(&app);
    let now = now_unix();
    let e = blocking(move || milestones::evaluate(&open()?, now).map_err(err)).await?;
    announce(&app, &e, now);
    Ok(e.fresh(now))
}

#[tauri::command]
pub async fn milestones_mark_celebrated(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    remember(&app);
    blocking(move || {
        open()?
            .mark_milestones_celebrated(&ids, now_unix())
            .map_err(err)
    })
    .await?;
    let _ = app.emit("milestones-changed", ());
    Ok(())
}

fn announce(app: &AppHandle, e: &milestones::Evaluation, now: i64) {
    if e.inserted.is_empty() {
        return;
    }
    let _ = app.emit("milestones-changed", ());
    let fresh = e.fresh(now);
    if !fresh.is_empty() && Settings::load_or_default().celebrate_milestones {
        let _ = app.emit("milestone-unlocked", fresh);
    }
}

/// Post-save hook (see `persist::after_save`): evaluate milestones after each
/// dictation, off the dictation worker so typing is never held up.
pub fn after_save(_store: &Store, _id: i64) {
    std::thread::spawn(|| {
        let now = now_unix();
        match Store::open_default().and_then(|s| milestones::evaluate(&s, now)) {
            Ok(e) => {
                if let Some(app) = APP.get() {
                    announce(app, &e, now);
                }
            }
            Err(e) => eprintln!("orttaai: milestone evaluation failed: {e}"),
        }
    });
}

// ---- What's New ------------------------------------------------------------

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether this person was using Orttaai before this launch: setup done and
/// a dictation from over an hour ago. A fresh install opens What's New only
/// after setup (when `has_completed_setup` is already true), so setup alone
/// can't tell the two apart.
fn is_existing_user(settings: &Settings, store: &Store) -> bool {
    settings.has_completed_setup
        && store
            .transcriptions_since(i64::MIN, 1)
            .ok()
            .and_then(|v| v.first().map(|r| r.created_at))
            .is_some_and(|first| first < now_unix() - 3_600)
}

/// The release notes to show once after an update, if any. A fresh install,
/// a downgrade or a release without notes is recorded as seen silently.
#[tauri::command]
pub async fn whats_new_pending(app: AppHandle) -> Result<Option<Release>, String> {
    remember(&app);
    let settings = Settings::load_or_default();
    let existing = {
        let s = settings.clone();
        blocking(move || Ok(is_existing_user(&s, &open()?))).await?
    };
    match whatsnew::evaluate(
        VERSION,
        settings.last_whats_new_version.as_deref(),
        existing,
    ) {
        Decision::Nothing => Ok(None),
        Decision::MarkSeen => {
            crate::settings_cmds::update_settings(
                app,
                serde_json::json!({ "last_whats_new_version": VERSION }),
            )?;
            Ok(None)
        }
        Decision::Show(release) => Ok(Some(*release)),
    }
}

/// The card was closed: this version's notes count as seen.
#[tauri::command]
pub fn whats_new_mark_seen(app: AppHandle, version: Option<String>) -> Result<(), String> {
    crate::settings_cmds::update_settings(
        app,
        serde_json::json!({ "last_whats_new_version": version.unwrap_or_else(|| VERSION.to_string()) }),
    )?;
    Ok(())
}

// ---- Share cards -----------------------------------------------------------

fn decode_png(png_base64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let data = png_base64
        .split_once("base64,")
        .map(|(_, d)| d)
        .unwrap_or(png_base64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(err)?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The card image isn't a PNG.".into());
    }
    Ok(bytes)
}

/// `Pictures/Orttaai`, else `Downloads`, else home.
fn share_dir() -> Result<PathBuf, String> {
    let dirs = directories::UserDirs::new().ok_or("No home folder to save into.")?;
    let base = dirs
        .picture_dir()
        .map(|p| p.join("Orttaai"))
        .or_else(|| dirs.download_dir().map(PathBuf::from))
        .unwrap_or_else(|| dirs.home_dir().to_path_buf());
    std::fs::create_dir_all(&base).map_err(err)?;
    Ok(base)
}

/// A file name with nothing but safe characters, ending in `.png`.
fn safe_file_name(name: &str) -> String {
    let stem: String = name
        .trim_end_matches(".png")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    format!(
        "{}.png",
        if stem.is_empty() {
            "Orttaai-card"
        } else {
            stem
        }
    )
}

/// Save a rendered share card (PNG, base64) into Pictures/Orttaai; returns
/// the full path. An existing file of that name gets a numbered sibling.
#[tauri::command]
pub async fn save_share_card(file_name: String, png_base64: String) -> Result<String, String> {
    blocking(move || {
        let bytes = decode_png(&png_base64)?;
        let dir = share_dir()?;
        let name = safe_file_name(&file_name);
        let mut path = dir.join(&name);
        let stem = name.trim_end_matches(".png").to_string();
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem}-{n}.png"));
            n += 1;
        }
        std::fs::write(&path, bytes).map_err(err)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}

/// Copy a rendered card to the clipboard natively (Linux: `wl-copy`, then
/// `xclip`). The UI tries the webview clipboard first, which covers Windows.
#[tauri::command]
pub async fn copy_share_card(png_base64: String) -> Result<(), String> {
    blocking(move || native_copy(&decode_png(&png_base64)?, Some("image/png"))).await
}

/// Show a saved card in the file manager.
#[tauri::command]
pub fn reveal_share_card(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}

/// Pipe bytes into the platform clipboard tool (`mime` for images).
#[cfg(target_os = "linux")]
fn native_copy(bytes: &[u8], mime: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // Only the tools for this session: wl-copy needs Wayland, xclip/xsel X11.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    let mut candidates: Vec<(&str, Vec<&str>)> = Vec::new();
    match mime {
        Some(m) => {
            if wayland {
                candidates.push(("wl-copy", vec!["--type", m]));
            }
            if x11 {
                candidates.push(("xclip", vec!["-selection", "clipboard", "-t", m]));
            }
        }
        None => {
            if wayland {
                candidates.push(("wl-copy", vec![]));
            }
            if x11 {
                candidates.push(("xclip", vec!["-selection", "clipboard"]));
                candidates.push(("xsel", vec!["--clipboard", "--input"]));
            }
        }
    }
    let mut failure = None;
    for (bin, args) in candidates {
        let Ok(mut child) = Command::new(bin)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(bytes).map_err(err)?;
        }
        // These tools keep running to serve the clipboard, so a tool that's
        // still running a moment later has it. One that exits at once either
        // handed it to a helper (success) or failed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    failure = Some(format!("{bin} failed ({status})"));
                    break;
                }
                Ok(None) if std::time::Instant::now() >= deadline => return Ok(()),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(e) => {
                    failure = Some(format!("{bin}: {e}"));
                    break;
                }
            }
        }
    }
    Err(failure.unwrap_or_else(|| {
        "No clipboard tool found (install wl-clipboard or xclip).".into()
    }))
}

#[cfg(not(target_os = "linux"))]
fn native_copy(_bytes: &[u8], _mime: Option<&str>) -> Result<(), String> {
    Err("Copy from the app window instead.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(
            safe_file_name("Orttaai-words-1k-mountain-portrait.png"),
            "Orttaai-words-1k-mountain-portrait.png"
        );
        assert_eq!(safe_file_name("../../etc/passwd"), "etc-passwd.png");
        assert_eq!(safe_file_name(""), "Orttaai-card.png");
    }

    #[test]
    fn png_payloads_are_checked() {
        use base64::Engine;
        let png = b"\x89PNG\r\n\x1a\nrest";
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        assert_eq!(
            decode_png(&format!("data:image/png;base64,{b64}")).unwrap(),
            png
        );
        assert!(decode_png(&base64::engine::general_purpose::STANDARD.encode(b"GIF89a")).is_err());
    }
}
