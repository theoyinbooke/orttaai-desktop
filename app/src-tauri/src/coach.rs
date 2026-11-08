//! Coach commands. (Coach area.)
//!
//! Thin wrappers over `orttaai_core::coach`: every command that touches the
//! store or the model server runs on a blocking thread. Events:
//! - `coach-changed` — counts, judgements or reps changed; refetch the snapshot.
//! - `coach-judge-state` — the judge check's progress and result (`CoachJudgeState`).
//! - `coach-rep-phase` — `{ message }` while a rep or scene is scored.
//! - `coach-scene-delta` — `{ request, text }`: the partner's reply so far.
//! - `coach-capture` — `{ token, raw, text, duration_ms }`: a dictation taken as practice.
//! - `coach-compare-progress` — `{ model }` while judges are compared.
//! - `coach-setup-step` — `SetupStep` while models are set up.

use crate::err;
use orttaai_core::coach::calendar::CoachCalendar;
use orttaai_core::coach::center::{self, SetupStatus, SetupStep};
use orttaai_core::coach::mode::{self, CoachAppRule, CoachMode};
use orttaai_core::coach::models::{self, CoachJudgeState, CoachModelComparison, CoachModelSetup};
use orttaai_core::coach::month::CoachMonthReport;
use orttaai_core::coach::persona::CoachPersona;
use orttaai_core::coach::practice::{self, CoachRepPlan, CoachRepResult, ScoreContext};
use orttaai_core::coach::records::{CoachHabitTarget, CoachInput};
use orttaai_core::coach::scene::{CoachScene, PartnerMood, SceneTurn};
use orttaai_core::coach::skill::{all_skills, CoachRepKind, CoachSkill, CoachTrack};
use orttaai_core::coach::snapshot::{self, CoachSnapshot};
use orttaai_core::coach::{self as core_coach, calibration};
use orttaai_core::llm::LlmClient;
use orttaai_core::settings::Settings;
use orttaai_core::store::{now_unix, Store};
use serde::Serialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter};

const CAL: CoachCalendar = CoachCalendar::Local;

/// Registers the core's listeners once, so background work reaches the UI.
fn wire(app: &AppHandle) {
    static WIRED: OnceLock<()> = OnceLock::new();
    WIRED.get_or_init(|| {
        let a = app.clone();
        core_coach::set_change_listener(move || {
            let _ = a.emit("coach-changed", ());
        });
        let a = app.clone();
        core_coach::set_judge_check_listener(move |state| {
            let _ = a.emit("coach-judge-state", state);
        });
        let a = app.clone();
        core_coach::set_capture_listener(move |token, capture| {
            let _ = a.emit(
                "coach-capture",
                json!({ "token": token, "raw": capture.raw, "text": capture.text, "duration_ms": capture.duration_ms }),
            );
        });
    });
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

fn store() -> Result<Store, String> {
    Store::open_default().map_err(err)
}

fn llm(settings: &Settings) -> Option<Arc<dyn LlmClient>> {
    core_coach::llm_client(settings)
}

/// Load, change and save settings, telling the UI and the other areas.
fn patch_settings(app: &AppHandle, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let current = Settings::load_or_default();
    let mut next = current.clone();
    change(&mut next);
    next.save().map_err(err)?;
    let _ = app.emit("settings-changed", &next);
    crate::on_settings_changed(app, &current, &next);
    Ok(next)
}

// ---- Catalog -------------------------------------------------------------

#[derive(Serialize)]
struct SkillInfo {
    id: &'static str,
    track: CoachTrack,
    title: &'static str,
    why: &'static str,
    drill: &'static str,
    check: String,
    check_label: &'static str,
    checks: Vec<String>,
    measure: Option<&'static str>,
    speak_only: bool,
    habit: Option<String>,
}

fn skill_info(s: &CoachSkill) -> SkillInfo {
    SkillInfo {
        id: s.id,
        track: s.track,
        title: s.title,
        why: s.why,
        drill: s.drill,
        check: s.check.id(),
        check_label: s.check.label(),
        checks: s.checks().iter().map(|c| c.id()).collect(),
        measure: s.measure().map(|m| m.id()),
        speak_only: s.speak_only(),
        habit: CoachHabitTarget::for_skill(s).map(CoachHabitTarget::stored),
    }
}

/// Everything static Coach shows: tracks, skills, scenes, coaches, measures,
/// questions, ways of talking and rep kinds, worded as on the Mac.
#[tauri::command]
pub fn coach_catalog() -> serde_json::Value {
    use orttaai_core::coach::measure::CoachMeasure;
    use orttaai_core::coach::skill::{CoachCheck, CoachQuestion};
    let checks: Vec<_> = CoachTrack::ALL
        .iter()
        .flat_map(|t| t.checks())
        .chain(all_skills().iter().map(|s| s.check.clone()))
        .map(|c: CoachCheck| json!({ "id": c.id(), "label": c.label(), "judged": c.is_judged() }))
        .collect();
    json!({
        "tracks": CoachTrack::ALL.iter().map(|t| json!({
            "id": t, "title": t.title(), "mode": t.mode(), "open_brief": t.open_brief(),
            "skills": t.skills().iter().map(|s| s.id).collect::<Vec<_>>(),
            "checks": t.checks().iter().map(|c| c.id()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "skills": all_skills().iter().map(skill_info).collect::<Vec<_>>(),
        "checks": checks,
        "scenes": orttaai_core::coach::scene::SCENES,
        "personas": CoachPersona::ALL.iter().map(|p| {
            let (rate, pitch) = p.voice();
            json!({ "id": p, "name": p.name(), "trait": p.trait_line(), "sample": p.sample(), "rate": rate, "pitch": pitch })
        }).collect::<Vec<_>>(),
        "measures": CoachMeasure::ALL.iter().map(|m| json!({
            "id": m, "axis_title": m.axis_title(), "habit_title": m.habit_title(), "hit_title": m.hit_title(), "unit": m.unit(),
        })).collect::<Vec<_>>(),
        "questions": CoachQuestion::ALL.iter().map(|q| json!({
            "id": q.id(), "label": q.label(), "axis_title": q.axis_title(),
        })).collect::<Vec<_>>(),
        "modes": CoachMode::ALL.iter().map(|m| json!({ "id": m, "title": m.title(), "short_title": m.short_title() })).collect::<Vec<_>>(),
        "rep_kinds": CoachRepKind::ALL.iter().map(|k| json!({ "id": k, "title": k.title(), "time_limit": k.time_limit_seconds() })).collect::<Vec<_>>(),
        "calibration": {
            "examples": calibration::EXAMPLES.len(),
            "pass_accuracy": calibration::PASS_ACCURACY,
            "pass_recall": calibration::PASS_RECALL,
        },
        "full_shape_dictations": snapshot::FULL_SHAPE_DICTATIONS,
        "habit_window": orttaai_core::coach::habit::WINDOW,
        "minimum_baseline": orttaai_core::coach::habit::MINIMUM_BASELINE,
        "reps_to_master": orttaai_core::coach::scoring::REPS_TO_MASTER,
        "max_scene_turns": orttaai_core::coach::scene::MAX_TURNS,
    })
}

// ---- Snapshot, habits, apps ----------------------------------------------

fn kick_background(app: &AppHandle, settings: &Settings) {
    core_coach::center::judge_in_background(settings.clone());
    refresh_judge_state(app);
}

/// Works out the judge's state off the UI thread and emits it; starts the
/// judge check the first time the judge in use is seen.
fn refresh_judge_state(app: &AppHandle) {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let settings = Settings::load_or_default();
        let client = llm(&settings);
        if let Ok(store) = Store::open_default() {
            let state = center::judge_state(&store, &settings, client.as_deref());
            let _ = app.emit("coach-judge-state", &state);
            if let (CoachJudgeState::Unchecked { model }, Some(client)) =
                (&state, client.as_deref())
            {
                center::check_judges(&store, &settings, client, std::slice::from_ref(model));
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// The Coach page's snapshot, after counting anything new; `null` while Coach is off.
#[tauri::command]
pub async fn coach_snapshot(app: AppHandle) -> Result<Option<CoachSnapshot>, String> {
    wire(&app);
    let a = app.clone();
    blocking(move || {
        let settings = Settings::load_or_default();
        let snap = center::evaluate(&store()?, &settings, now_unix(), &CAL).map_err(err)?;
        if snap.is_some() {
            kick_background(&a, &settings);
        }
        Ok(snap)
    })
    .await
}

#[tauri::command]
pub async fn coach_judge_state(app: AppHandle) -> Result<CoachJudgeState, String> {
    wire(&app);
    blocking(|| {
        let settings = Settings::load_or_default();
        let client = llm(&settings);
        Ok(center::judge_state(&store()?, &settings, client.as_deref()))
    })
    .await
}

/// Grades each model on the judge check in the background (`coach-judge-state`).
#[tauri::command]
pub fn coach_check_judges(app: AppHandle, models: Vec<String>) -> Result<(), String> {
    wire(&app);
    if models.is_empty() || center::checking().is_some() {
        return Ok(());
    }
    std::thread::spawn(move || {
        let settings = Settings::load_or_default();
        if let (Some(client), Ok(store)) = (llm(&settings), Store::open_default()) {
            center::check_judges(&store, &settings, client.as_ref(), &models);
        } else {
            let _ = app.emit("coach-judge-state", CoachJudgeState::NoServer);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn coach_start_habit(
    app: AppHandle,
    target: String,
    mode: Option<String>,
) -> Result<(), String> {
    let target =
        CoachHabitTarget::from_stored(&target).ok_or_else(|| format!("Unknown habit {target}"))?;
    let mode = mode.as_deref().and_then(CoachMode::from_id);
    blocking(move || {
        store()?
            .start_coach_habit(target, mode, now_unix())
            .map(|_| ())
            .map_err(err)
    })
    .await?;
    let _ = app.emit("coach-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn coach_end_habit(app: AppHandle, id: i64) -> Result<(), String> {
    blocking(move || store()?.end_coach_habit(id, now_unix()).map_err(err)).await?;
    let _ = app.emit("coach-changed", ());
    Ok(())
}

/// Files an app under a way of talking, or switches it off, which also
/// forgets what Coach counted in it. `key` is the app's Coach key.
#[tauri::command]
pub async fn coach_set_app_rule(
    app: AppHandle,
    key: String,
    app_id: Option<String>,
    app_name: Option<String>,
    rule: String,
) -> Result<(), String> {
    let rule = CoachAppRule::from_id(&rule).ok_or_else(|| format!("Unknown rule {rule}"))?;
    let key = if key.trim().is_empty() {
        mode::app_key(app_id.as_deref(), app_name.as_deref()).ok_or("Unknown app")?
    } else {
        key.trim().to_lowercase()
    };
    let builtin = mode::builtin_mode(app_id.as_deref(), app_name.as_deref());
    let k = key.clone();
    patch_settings(&app, move |s| {
        if rule == CoachAppRule::from_mode(builtin) {
            s.coach_app_overrides.remove(&k);
        } else {
            s.coach_app_overrides.insert(k, rule.id().to_string());
        }
    })?;
    if rule == CoachAppRule::Off {
        blocking(move || {
            store()?
                .delete_coach_metrics_for_keys(&[key])
                .map(|_| ())
                .map_err(err)
        })
        .await?;
    }
    let _ = app.emit("coach-changed", ());
    Ok(())
}

/// Forgets an override added by hand (Settings › Coach).
#[tauri::command]
pub fn coach_remove_app_rule(app: AppHandle, key: String) -> Result<(), String> {
    patch_settings(&app, |s| {
        s.coach_app_overrides.remove(&key);
    })?;
    let _ = app.emit("coach-changed", ());
    Ok(())
}

/// The coach for one way of talking, or for every one when `mode` is null.
#[tauri::command]
pub fn coach_set_persona(
    app: AppHandle,
    persona: String,
    mode: Option<String>,
) -> Result<(), String> {
    let persona =
        CoachPersona::from_id(&persona).ok_or_else(|| format!("Unknown coach {persona}"))?;
    let mode = mode.as_deref().and_then(CoachMode::from_id);
    patch_settings(&app, |s| match mode {
        Some(m) => {
            s.coach_personas
                .insert(m.id().to_string(), persona.id().to_string());
        }
        None => {
            s.coach_persona = persona.id().to_string();
            s.coach_personas = CoachMode::ALL
                .iter()
                .map(|m| (m.id().to_string(), persona.id().to_string()))
                .collect();
        }
    })?;
    Ok(())
}

#[tauri::command]
pub async fn coach_month_report(month_of: i64) -> Result<CoachMonthReport, String> {
    blocking(move || {
        let settings = Settings::load_or_default();
        center::month_report(&store()?, &settings, month_of, now_unix(), &CAL).map_err(err)
    })
    .await
}

// ---- Practice ------------------------------------------------------------

fn recent_inputs(store: &Store, now: i64) -> Result<Vec<CoachInput>, String> {
    let since = now - 21 * 86_400;
    Ok(store
        .coach_inputs()
        .map_err(err)?
        .into_iter()
        .filter(|i| i.created_at >= since)
        .collect())
}

fn plan_rep(
    store: &Store,
    settings: &Settings,
    skill: &'static CoachSkill,
    kind: CoachRepKind,
    spoken: bool,
    avoiding: &[i64],
) -> Result<CoachRepPlan, String> {
    let now = now_unix();
    let overrides = models::overrides(settings);
    let mut kind = kind;
    let mut source = None;
    if kind == CoachRepKind::Redo {
        let inputs = recent_inputs(store, now)?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let found = snapshot::redo_source(skill, &inputs, &overrides, avoiding, now, seed)
            .and_then(|id| store.coach_quotes(&[id]).ok()?.remove(&id));
        match found {
            Some(q) => source = Some(q.into()),
            None => kind = CoachRepKind::Drill,
        }
    }
    let mode = skill
        .track
        .mode()
        .or_else(|| busiest_mode(store, settings, now));
    Ok(CoachRepPlan {
        skill_id: skill.id.to_string(),
        kind,
        spoken: spoken || skill.speak_only(),
        instruction: practice::instruction(skill, kind, source.is_some()),
        source,
        persona: models::persona_for(settings, mode),
        mode,
    })
}

fn busiest_mode(store: &Store, settings: &Settings, now: i64) -> Option<CoachMode> {
    let overrides = models::overrides(settings);
    let since = CAL.add_days(now, -orttaai_core::coach::shape::DAYS);
    let mut counts = std::collections::BTreeMap::new();
    for i in store.coach_inputs().ok()? {
        if i.created_at >= since {
            if let Some(m) = i.mode(&overrides) {
                *counts.entry(m).or_insert(0usize) += 1;
            }
        }
    }
    Some(snapshot::busiest(&counts))
}

/// Sets up a rep (a redo falls back to a drill when there's nothing of yours to redo).
#[tauri::command]
pub async fn coach_begin_rep(
    app: AppHandle,
    skill_id: String,
    kind: String,
    spoken: bool,
) -> Result<CoachRepPlan, String> {
    wire(&app);
    let skill = CoachSkill::by_id(&skill_id).ok_or_else(|| format!("Unknown skill {skill_id}"))?;
    let kind: CoachRepKind = serde_json::from_value(json!(kind)).map_err(err)?;
    core_coach::set_session_active(true);
    blocking(move || {
        plan_rep(
            &store()?,
            &Settings::load_or_default(),
            skill,
            kind,
            spoken,
            &[],
        )
    })
    .await
}

/// The next rep: the same skill, set up another way.
#[tauri::command]
pub async fn coach_next_rep(plan: CoachRepPlan) -> Result<CoachRepPlan, String> {
    let skill = plan.skill().ok_or("Unknown skill")?;
    let avoid: Vec<i64> = plan.source.iter().map(|s| s.id).collect();
    blocking(move || {
        plan_rep(
            &store()?,
            &Settings::load_or_default(),
            skill,
            practice::next_kind(plan.kind),
            plan.spoken,
            &avoid,
        )
    })
    .await
}

/// Leaving a rep or scene: background judging may resume, nothing stays armed.
#[tauri::command]
pub fn coach_end_session(app: AppHandle) {
    core_coach::disarm();
    core_coach::set_session_active(false);
    core_coach::center::judge_in_background(Settings::load_or_default());
    let _ = app.emit("coach-changed", ());
}

/// What scoring reads: the client, the jobs, the grades and the store.
struct ScoreParts {
    client: Option<Arc<dyn LlmClient>>,
    setup: CoachModelSetup,
    grades: std::collections::BTreeMap<String, calibration::JudgeGrade>,
    store: Store,
}

fn score_context_parts(settings: &Settings) -> Result<ScoreParts, String> {
    let store = store()?;
    let grades = models::judge_grades(&store);
    let setup = CoachModelSetup::current(settings, &grades, models::memory());
    Ok(ScoreParts {
        client: llm(settings),
        setup,
        grades,
        store,
    })
}

/// Scores a rep, saves its numbers, and returns the reveal. `coach-rep-phase`
/// reports each step.
#[tauri::command]
pub async fn coach_score_rep(
    app: AppHandle,
    plan: CoachRepPlan,
    heard: String,
    written: String,
    duration_ms: i64,
    spoken: bool,
) -> Result<CoachRepResult, String> {
    let a = app.clone();
    let result = blocking(move || {
        let settings = Settings::load_or_default();
        let ScoreParts {
            client,
            setup,
            grades,
            store,
        } = score_context_parts(&settings)?;
        let installed = client.as_deref().and_then(models::installed);
        let now = now_unix();
        let snap = center::evaluate(&store, &settings, now, &CAL).map_err(err)?;
        let skill = plan.skill().ok_or("Unknown skill")?;
        let usual = snap.as_ref().and_then(|s| s.progress(skill).usual);
        let summary = snap
            .as_ref()
            .map(|s| s.shape(plan.mode).now_summary.clone());
        let cx = ScoreContext {
            llm: client.as_deref(),
            setup: &setup,
            grades: &grades,
            installed: installed.as_deref(),
            usual,
            usual_summary: summary.as_ref(),
        };
        let mut phase = |m: &str| {
            let _ = a.emit("coach-rep-phase", json!({ "message": m }));
        };
        let result = practice::score_rep(
            &cx,
            &plan,
            &heard,
            &written,
            duration_ms,
            spoken,
            &mut phase,
        )?;
        store.save_coach_rep(&result.record(now)).map_err(err)?;
        Ok(result)
    })
    .await?;
    let _ = app.emit("coach-changed", ());
    Ok(result)
}

/// The next dictation becomes practice capture `token` (`coach-capture`).
/// `polish: false` for scene turns, which the partner answers as said.
#[tauri::command]
pub fn coach_practice_arm(app: AppHandle, token: u64, polish: bool) {
    wire(&app);
    core_coach::arm(token, polish);
}

#[tauri::command]
pub fn coach_practice_disarm() {
    core_coach::disarm();
}

/// The dictation in flight was started for practice but nobody spoke: drop it.
#[tauri::command]
pub fn coach_practice_discard() {
    core_coach::arm_discard();
}

// ---- Scenes --------------------------------------------------------------

#[derive(Serialize)]
pub struct SceneStart {
    scene: &'static CoachScene,
    skill: &'static str,
    persona: CoachPersona,
    usual: Option<f64>,
    partner_model: String,
    /// Why the scene can't run, when it can't.
    problem: Option<String>,
}

fn scene_by_id(id: &str) -> Result<&'static CoachScene, String> {
    CoachScene::by_id(id).ok_or_else(|| format!("Unknown scene {id}"))
}

/// Opens a scene: the skill it practises, the coach listening, and whether
/// the partner's model is ready.
#[tauri::command]
pub async fn coach_scene_begin(app: AppHandle, scene_id: String) -> Result<SceneStart, String> {
    wire(&app);
    let scene = scene_by_id(&scene_id)?;
    core_coach::set_session_active(true);
    blocking(move || {
        let settings = Settings::load_or_default();
        let store = store()?;
        let snap = center::evaluate(&store, &settings, now_unix(), &CAL).map_err(err)?;
        let track = scene.track;
        let skill = snap
            .as_ref()
            .map(|s| s.current_skill(track))
            .unwrap_or(track.skills()[0]);
        let mode = track
            .mode()
            .or_else(|| snap.as_ref().map(|s| s.busiest_mode));
        let grades = models::judge_grades(&store);
        let setup = CoachModelSetup::current(&settings, &grades, models::memory());
        let installed = llm(&settings).as_deref().and_then(models::installed);
        let problem = match installed {
            None => Some(
                "Scenes need your local model server. Start Ollama or LM Studio, then try again."
                    .to_string(),
            ),
            Some(list) if !list.contains(&setup.scene_model) => Some(format!(
                "{} isn't installed. Pick a scene partner in Cast › Models.",
                setup.scene_model
            )),
            Some(_) => None,
        };
        Ok(SceneStart {
            scene,
            skill: skill.id,
            persona: models::persona_for(&settings, mode),
            usual: snap.as_ref().and_then(|s| s.progress(skill).usual),
            partner_model: setup.scene_model,
            problem,
        })
    })
    .await
}

/// Loads the partner's model so its first reply isn't kept waiting.
#[tauri::command]
pub async fn coach_scene_warm() -> Result<(), String> {
    blocking(|| {
        let settings = Settings::load_or_default();
        if let Some(client) = llm(&settings) {
            let store = store()?;
            let setup = CoachModelSetup::current(
                &settings,
                &models::judge_grades(&store),
                models::memory(),
            );
            models::warm_partner(client.as_ref(), &setup);
        }
        Ok(())
    })
    .await
}

/// The partner's next line, streamed as `coach-scene-delta` `{ request, text }`.
/// `null` when the model didn't answer.
#[tauri::command]
pub async fn coach_scene_reply(
    app: AppHandle,
    scene_id: String,
    turns: Vec<SceneTurn>,
    request: u64,
) -> Result<Option<String>, String> {
    let scene = scene_by_id(&scene_id)?;
    blocking(move || {
        let settings = Settings::load_or_default();
        let Some(client) = llm(&settings) else {
            return Ok(None);
        };
        let store = store()?;
        let setup =
            CoachModelSetup::current(&settings, &models::judge_grades(&store), models::memory());
        let mut on_text = |text: &str| {
            let _ = app.emit(
                "coach-scene-delta",
                json!({ "request": request, "text": text }),
            );
        };
        Ok(models::partner_reply(
            client.as_ref(),
            &setup,
            scene,
            &turns,
            &mut on_text,
        ))
    })
    .await
}

#[tauri::command]
pub async fn coach_scene_mood(
    scene_id: String,
    turns: Vec<SceneTurn>,
) -> Result<Option<PartnerMood>, String> {
    let scene = scene_by_id(&scene_id)?;
    blocking(move || {
        let settings = Settings::load_or_default();
        let Some(client) = llm(&settings) else {
            return Ok(None);
        };
        let store = store()?;
        let setup =
            CoachModelSetup::current(&settings, &models::judge_grades(&store), models::memory());
        Ok(models::partner_mood(client.as_ref(), &setup, scene, &turns))
    })
    .await
}

/// Ends a scene: scores your turns together, plus the goal, and saves the rep.
#[tauri::command]
pub async fn coach_scene_end(
    app: AppHandle,
    scene_id: String,
    skill_id: String,
    persona: String,
    usual: Option<f64>,
    turns: Vec<SceneTurn>,
) -> Result<CoachRepResult, String> {
    let scene = scene_by_id(&scene_id)?;
    let skill = CoachSkill::by_id(&skill_id).ok_or_else(|| format!("Unknown skill {skill_id}"))?;
    let persona = CoachPersona::from_id(&persona).unwrap_or(CoachPersona::Mentor);
    let a = app.clone();
    let result = blocking(move || {
        let settings = Settings::load_or_default();
        let ScoreParts {
            client,
            setup,
            grades,
            store,
        } = score_context_parts(&settings)?;
        let installed = client.as_deref().and_then(models::installed);
        let cx = ScoreContext {
            llm: client.as_deref(),
            setup: &setup,
            grades: &grades,
            installed: installed.as_deref(),
            usual,
            usual_summary: None,
        };
        let mut phase = |m: &str| {
            let _ = a.emit("coach-rep-phase", json!({ "message": m }));
        };
        let result = practice::score_scene(&cx, scene, skill, persona, &turns, &mut phase)?;
        store
            .save_coach_rep(&result.record(now_unix()))
            .map_err(err)?;
        Ok(result)
    })
    .await?;
    let _ = app.emit("coach-changed", ());
    Ok(result)
}

// ---- Models --------------------------------------------------------------

#[derive(Serialize)]
pub struct ModelsInfo {
    server_running: bool,
    installed: Vec<String>,
    chat_models: Vec<String>,
    setup: CoachModelSetup,
    /// Grades against the current examples, passed first, then most accurate.
    grades: Vec<calibration::JudgeGrade>,
    /// Why each failed grade failed, by model.
    why_failed: std::collections::BTreeMap<String, &'static str>,
    best_passing: Option<String>,
    insights_model: String,
    judge_state: CoachJudgeState,
}

#[tauri::command]
pub async fn coach_models() -> Result<ModelsInfo, String> {
    blocking(|| {
        let settings = Settings::load_or_default();
        let store = store()?;
        let client = llm(&settings);
        let installed = client.as_deref().and_then(models::installed);
        let all = models::judge_grades(&store);
        let mut grades: Vec<calibration::JudgeGrade> =
            all.values().filter(|g| g.is_current()).cloned().collect();
        grades.sort_by(|a, b| {
            (b.passes(), b.accuracy)
                .partial_cmp(&(a.passes(), a.accuracy))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let why_failed = grades
            .iter()
            .filter(|g| !g.passes())
            .map(|g| (g.model.clone(), g.why_failed()))
            .collect();
        Ok(ModelsInfo {
            server_running: installed.is_some(),
            chat_models: models::chat_models(installed.as_deref().unwrap_or(&[])),
            installed: installed.unwrap_or_default(),
            setup: CoachModelSetup::current(&settings, &all, models::memory()),
            best_passing: models::best_passing_judge(&all).map(|g| g.model.clone()),
            grades,
            why_failed,
            insights_model: settings.insights_model.clone(),
            judge_state: center::judge_state(&store, &settings, client.as_deref()),
        })
    })
    .await
}

#[derive(Serialize)]
pub struct Comparison {
    question: Option<String>,
    rows: Vec<CoachModelComparison>,
}

/// Each installed model (up to four) judges the same 20 of your recent
/// dictations on one question. `coach-compare-progress` names the model asked.
#[tauri::command]
pub async fn coach_compare_models(app: AppHandle) -> Result<Comparison, String> {
    blocking(move || {
        let settings = Settings::load_or_default();
        let store = store()?;
        let Some(client) = llm(&settings) else {
            return Err("Start Ollama or LM Studio first.".into());
        };
        let now = now_unix();
        let busiest = busiest_mode(&store, &settings, now).unwrap_or(CoachMode::Ai);
        let (question, texts) =
            center::comparison_sample(&store, &settings, busiest, now, 20).map_err(err)?;
        if texts.is_empty() {
            return Ok(Comparison {
                question: None,
                rows: Vec::new(),
            });
        }
        let chat = models::chat_models(&models::installed(client.as_ref()).unwrap_or_default());
        let chosen: Vec<String> = chat.into_iter().take(4).collect();
        let setup =
            CoachModelSetup::current(&settings, &models::judge_grades(&store), models::memory());
        let mut progress = |m: &str| {
            let _ = app.emit("coach-compare-progress", json!({ "model": m }));
        };
        let rows = models::compare(
            client.as_ref(),
            &setup,
            &chosen,
            &texts,
            question,
            &mut progress,
        );
        Ok(Comparison {
            question: Some(question.id().to_string()),
            rows,
        })
    })
    .await
}

#[tauri::command]
pub async fn coach_setup_status() -> Result<SetupStatus, String> {
    blocking(|| {
        let settings = Settings::load_or_default();
        let client = llm(&settings);
        Ok(center::setup_status(
            &store()?,
            &settings,
            client.as_deref(),
        ))
    })
    .await
}

/// Downloads what's missing, checks the judge and sets the jobs, reporting
/// each step as `coach-setup-step`.
#[tauri::command]
pub fn coach_run_setup(app: AppHandle) -> Result<(), String> {
    wire(&app);
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    std::thread::spawn(move || {
        let settings = Settings::load_or_default();
        let client = llm(&settings);
        match Store::open_default() {
            Ok(store) => {
                let a = app.clone();
                let mut on_step = |s: &SetupStep| {
                    let _ = a.emit("coach-setup-step", s);
                };
                if let Ok(patch) =
                    center::set_up_models(&store, &settings, client.as_deref(), &mut on_step)
                {
                    let _ = patch_settings(&app, |s| {
                        if let Ok(next) = s.merged(&patch) {
                            *s = next;
                        }
                    });
                    refresh_judge_state(&app);
                    let _ = app.emit("coach-changed", ());
                }
            }
            Err(e) => {
                let _ = app.emit(
                    "coach-setup-step",
                    SetupStep::Failed {
                        message: e.to_string(),
                    },
                );
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// Starts `ollama serve` when Ollama is installed but not running. Returns
/// whether it could be started; the UI offers the download page otherwise.
#[tauri::command]
pub fn coach_start_ollama() -> bool {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("ollama");
    cmd.arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | DETACHED_PROCESS: no console flashes up.
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    cmd.spawn().is_ok()
}

/// Saves a share card PNG into Pictures (else Downloads) and returns its path.
#[tauri::command]
pub async fn coach_save_card(file_name: String, png: Vec<u8>) -> Result<String, String> {
    blocking(move || {
        center::save_card(&file_name, &png)
            .map(|p| p.display().to_string())
            .map_err(err)
    })
    .await
}
