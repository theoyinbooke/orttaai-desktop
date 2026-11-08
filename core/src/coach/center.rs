//! Keeps Coach's counts current and works out what the Coach page shows
//! (Mac `CoachCenter`): counting new dictations, the snapshot, the month
//! report, the judge check, background judging and model setup. Everything
//! here blocks; callers run it off the UI thread.

use super::calendar::CoachCalendar;
use super::calibration::{JudgeGrade, EXAMPLES};
use super::counting::DictationMeasures;
use super::mode::{self, CoachMode};
use super::models::{self, CoachJudgeState, CoachModelSetup};
use super::month::{self, CoachMonthReport, MonthInputs};
use super::records::CoachHabitTarget;
use super::shape::{self, judging, DatedMeasures};
use super::skill::{CoachCheck, CoachQuestion, CoachTrack};
use super::snapshot::{self, CoachSnapshot, SnapshotInputs};
use crate::error::Result;
use crate::llm::LlmClient;
use crate::settings::Settings;
use crate::store::{now_unix, Store};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

// ---- Wiring --------------------------------------------------------------

type Factory = Box<dyn Fn(&Settings) -> Option<Arc<dyn LlmClient>> + Send + Sync>;
static FACTORY: RwLock<Option<Factory>> = RwLock::new(None);
type ChangeListener = Box<dyn Fn() + Send + Sync>;
static ON_CHANGE: Mutex<Option<ChangeListener>> = Mutex::new(None);
type CheckListener = Box<dyn Fn(&CoachJudgeState) + Send + Sync>;
static ON_CHECK: Mutex<Option<CheckListener>> = Mutex::new(None);

/// How Coach reaches the local model server. Tests (and the shell, if it
/// wants a custom client) register a constructor here; otherwise Coach uses
/// the shared [`crate::llm::client_from_settings`] client.
pub fn set_llm_factory(
    f: impl Fn(&Settings) -> Option<Arc<dyn LlmClient>> + Send + Sync + 'static,
) {
    *FACTORY.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

/// The client for the configured server, or `None` when there is none.
pub fn llm_client(settings: &Settings) -> Option<Arc<dyn LlmClient>> {
    if let Some(factory) = FACTORY.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return factory(settings);
    }
    fallback_client(settings)
}

/// Without a registered factory, use the shared local client (Ollama or LM
/// Studio, local endpoints only).
fn fallback_client(settings: &Settings) -> Option<Arc<dyn LlmClient>> {
    if !crate::llm::policy::is_local_endpoint(settings.llm_endpoint()) {
        return None;
    }
    Some(crate::llm::client_from_settings(settings))
}

/// Called whenever background work changes what Coach shows.
pub fn set_change_listener(f: impl Fn() + Send + Sync + 'static) {
    *ON_CHANGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

/// Called as the judge check progresses and when it ends.
pub fn set_judge_check_listener(f: impl Fn(&CoachJudgeState) + Send + Sync + 'static) {
    *ON_CHECK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

fn notify_changed() {
    if let Some(f) = ON_CHANGE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        f();
    }
}

fn notify_check(state: &CoachJudgeState) {
    if let Some(f) = ON_CHECK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        f(state);
    }
}

static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
static JUDGING: AtomicBool = AtomicBool::new(false);
static CHECKING: Mutex<Option<(String, usize, usize)>> = Mutex::new(None);

/// A rep or scene is open: background judging waits so it never competes.
pub fn set_session_active(active: bool) {
    SESSION_ACTIVE.store(active, Ordering::SeqCst);
}

/// The judge check in progress, if any.
pub fn checking() -> Option<CoachJudgeState> {
    CHECKING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .map(|(model, done, total)| CoachJudgeState::Checking { model, done, total })
}

// ---- Counting ------------------------------------------------------------

pub const BATCH_SIZE: i64 = 500;

/// Counts dictations that have no counts yet, newest first, in batches,
/// never reading apps switched off. Returns how many were counted.
pub fn catch_up(store: &Store, settings: &Settings) -> Result<usize> {
    let off: HashSet<String> = mode::off_keys(&models::overrides(settings))
        .into_iter()
        .collect();
    let mut cursor = None;
    let mut counted = 0;
    loop {
        let page = store.coach_backlog_page(cursor, BATCH_SIZE)?;
        let Some(last) = page.last() else { break };
        cursor = Some((last.created_at, last.id));
        let metrics: Vec<(i64, DictationMeasures)> = page
            .iter()
            .filter(|s| !s.key().is_some_and(|k| off.contains(&k)))
            .map(|s| (s.id, DictationMeasures::measuring(&s.dictation())))
            .collect();
        counted += metrics.len();
        store.save_coach_metrics(&metrics)?;
        if (page.len() as i64) < BATCH_SIZE {
            break;
        }
    }
    Ok(counted)
}

/// The per-save hook: count the new dictation now, then let the judge look
/// in the background. Does nothing while Coach is off or the app is off.
pub fn record(store: &Store, id: i64, settings: &Settings) -> Result<bool> {
    if !settings.coach_enabled {
        return Ok(false);
    }
    let Some(source) = store.coach_index_source(id)? else {
        return Ok(false);
    };
    let overrides = models::overrides(settings);
    if source
        .key()
        .is_some_and(|k| overrides.get(&k) == Some(&mode::CoachAppRule::Off))
    {
        return Ok(false);
    }
    store.save_coach_metrics(&[(id, DictationMeasures::measuring(&source.dictation()))])?;
    Ok(true)
}

/// `persist.rs` calls this after every save.
pub fn after_save(store: &Store, id: i64) {
    let settings = Settings::load_or_default();
    match record(store, id, &settings) {
        Ok(true) => judge_in_background(settings),
        Ok(false) => {}
        Err(e) => tracing::warn!("Coach couldn't count dictation {id}: {e}"),
    }
}

// ---- What the page shows -------------------------------------------------

fn judge_model(store: &Store, settings: &Settings) -> String {
    CoachModelSetup::current(settings, &models::judge_grades(store), models::memory()).judge_model
}

/// The snapshot, after counting anything new; `None` while Coach is off.
pub fn evaluate(
    store: &Store,
    settings: &Settings,
    now: i64,
    cal: &CoachCalendar,
) -> Result<Option<CoachSnapshot>> {
    if !settings.coach_enabled {
        return Ok(None);
    }
    catch_up(store, settings)?;
    store.prune_coach_judgements()?;
    let inputs = store.coach_inputs()?;
    let habits = store.coach_habits()?;
    let apps = store.coach_apps()?;
    let reps = store.coach_reps()?;
    let scores = store.coach_judgement_scores(&judge_model(store, settings))?;
    let overrides = models::overrides(settings);
    let d = SnapshotInputs {
        inputs: &inputs,
        habits: &habits,
        apps: &apps,
        overrides: &overrides,
        reps: &reps,
        scores: &scores,
    };
    let seed = now as u64 / 3_600;
    Ok(Some(snapshot::build(
        &d,
        |ids| store.coach_quotes(ids).unwrap_or_default(),
        now,
        cal,
        seed,
    )))
}

/// The report for the calendar month holding `month_of`.
pub fn month_report(
    store: &Store,
    settings: &Settings,
    month_of: i64,
    now: i64,
    cal: &CoachCalendar,
) -> Result<CoachMonthReport> {
    let inputs = store.coach_inputs()?;
    let habits = store.coach_habits()?;
    let reps = store.coach_reps()?;
    let scores = store.coach_judgement_scores(&judge_model(store, settings))?;
    let overrides = models::overrides(settings);
    let d = MonthInputs {
        inputs: &inputs,
        habits: &habits,
        reps: &reps,
        scores: &scores,
        overrides: &overrides,
    };
    Ok(month::build(
        month_of,
        &d,
        |ids| store.coach_quotes(ids).unwrap_or_default(),
        now,
        cal,
    ))
}

/// Up to `limit` recent dictations in your busiest way of talking, for
/// comparing judges, with the question they're asked. Read live; never kept.
pub fn comparison_sample(
    store: &Store,
    settings: &Settings,
    busiest: CoachMode,
    now: i64,
    limit: usize,
) -> Result<(CoachQuestion, Vec<String>)> {
    let question = CoachTrack::for_mode(busiest)
        .checks()
        .into_iter()
        .find_map(|c| match c {
            CoachCheck::Judged(q) => Some(q),
            _ => None,
        })
        .unwrap_or(CoachQuestion::TaskFirst);
    let overrides = models::overrides(settings);
    let since = now - 21 * 86_400;
    let texts = store
        .coach_inputs()?
        .into_iter()
        .rev()
        .filter(|i| {
            i.created_at >= since && i.measures.words >= 12 && i.mode(&overrides) == Some(busiest)
        })
        .take(limit)
        .filter_map(|i| store.coach_text(i.id).ok().flatten())
        .collect();
    Ok((question, texts))
}

// ---- The judge check -----------------------------------------------------

/// Grades each model in turn, keeping every grade. Reports progress through
/// the judge-check listener.
pub fn check_judges(
    store: &Store,
    settings: &Settings,
    llm: &dyn LlmClient,
    models_to_check: &[String],
) -> Vec<JudgeGrade> {
    let setup = CoachModelSetup::current(settings, &models::judge_grades(store), models::memory());
    let mut grades = Vec::new();
    for model in models_to_check {
        let total = EXAMPLES.len();
        *CHECKING.lock().unwrap_or_else(|e| e.into_inner()) = Some((model.clone(), 0, total));
        notify_check(&CoachJudgeState::Checking {
            model: model.clone(),
            done: 0,
            total,
        });
        let mut progress = |done: usize, total: usize| {
            *CHECKING.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((model.clone(), done, total));
            notify_check(&CoachJudgeState::Checking {
                model: model.clone(),
                done,
                total,
            });
        };
        let grade = models::grade(llm, &setup, model, &mut progress, now_unix());
        if let Err(e) = models::save_judge_grade(store, &grade) {
            tracing::warn!("Coach couldn't keep a judge grade: {e}");
        }
        grades.push(grade);
    }
    *CHECKING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let state = judge_state(store, settings, Some(llm));
    notify_check(&state);
    notify_changed();
    grades
}

/// The judge's state now (the check in progress wins).
pub fn judge_state(
    store: &Store,
    settings: &Settings,
    llm: Option<&dyn LlmClient>,
) -> CoachJudgeState {
    if let Some(checking) = checking() {
        return checking;
    }
    let grades = models::judge_grades(store);
    let setup = CoachModelSetup::current(settings, &grades, models::memory());
    let installed = llm.and_then(models::installed);
    models::judge_state(installed.as_deref(), &setup, &grades)
}

// ---- Background judging --------------------------------------------------

/// Asks the judge about real dictations: first for habits on judged skills
/// (the 20 before each started and every one since), then to fill the
/// judged axes of the voice shape. At most 10 a pass and 40 a day, each
/// answer cached. Returns how many were asked.
pub fn judge_pass(
    store: &Store,
    settings: &Settings,
    llm: &dyn LlmClient,
    now: i64,
    cal: &CoachCalendar,
) -> usize {
    let grades = models::judge_grades(store);
    let setup = CoachModelSetup::current(settings, &grades, models::memory());
    if !models::judge_trusted(&setup, &grades)
        || !models::installed(llm).is_some_and(|i| i.contains(&setup.judge_model))
    {
        return 0;
    }
    let day_start = cal.start_of_day(now);
    let used = store.count_coach_judgements_since(day_start).unwrap_or(40);
    let budget = 10.min(40 - used);
    if budget <= 0 {
        return 0;
    }
    let budget = budget as usize;
    let known = store
        .coach_judgement_scores(&setup.judge_model)
        .unwrap_or_default();
    let Ok(inputs) = store.coach_inputs() else {
        return 0;
    };
    let habits = store.coach_habits().unwrap_or_default();
    let overrides = models::overrides(settings);

    let recent_since = now - 21 * 86_400;
    let recent: Vec<&super::records::CoachInput> = inputs
        .iter()
        .filter(|i| i.created_at >= recent_since)
        .collect();
    let mut queue: Vec<(CoachQuestion, i64)> = Vec::new();
    for habit in habits.iter().filter(|h| h.ended_at.is_none()) {
        let Some(CoachHabitTarget::Question(question)) = habit.target() else {
            continue;
        };
        let wanted = habit.coach_mode();
        let in_mode: Vec<&&super::records::CoachInput> = recent
            .iter()
            .filter(|i| {
                i.measures.words >= judging::MINIMUM_WORDS
                    && i.mode(&overrides)
                        .is_some_and(|m| wanted.is_none_or(|w| m == w))
            })
            .collect();
        let before: Vec<i64> = in_mode
            .iter()
            .filter(|i| i.created_at < habit.started_at)
            .map(|i| i.id)
            .collect();
        let before = &before[before.len().saturating_sub(super::habit::WINDOW)..];
        let since: Vec<i64> = in_mode
            .iter()
            .filter(|i| i.created_at >= habit.started_at)
            .map(|i| i.id)
            .collect();
        // Newest first since the start, then the baseline.
        queue.extend(since.iter().rev().map(|&id| (question, id)));
        queue.extend(before.iter().rev().map(|&id| (question, id)));
    }

    let shape_start = now - (shape::DAYS * 2 + 1) * 86_400;
    let samples: Vec<DatedMeasures> = snapshot::dated(&inputs, &overrides)
        .into_iter()
        .filter(|d| d.created_at >= shape_start)
        .collect();
    let mut recent_by_mode: BTreeMap<CoachMode, usize> = BTreeMap::new();
    let recent_start = cal.add_days(now, -shape::DAYS);
    for s in samples.iter().filter(|s| s.created_at >= recent_start) {
        *recent_by_mode.entry(s.mode).or_default() += 1;
    }
    let mut modes = CoachMode::SHAPED.to_vec();
    modes.sort_by_key(|m| std::cmp::Reverse(recent_by_mode.get(m).copied().unwrap_or(0)));
    queue.extend(judging::queue(&samples, &modes, &known, now, cal));

    let mut asked = 0;
    let mut seen = HashSet::new();
    for (question, id) in queue {
        if known.get(&question).is_some_and(|a| a.contains_key(&id)) {
            continue;
        }
        if asked >= budget || SESSION_ACTIVE.load(Ordering::SeqCst) {
            break;
        }
        if !seen.insert((question, id)) {
            continue;
        }
        let Ok(Some(text)) = store.coach_text(id) else {
            continue;
        };
        let Some((answer, confidence)) = models::judge(llm, &setup, &grades, question, &text, None)
        else {
            continue;
        };
        if store
            .save_coach_judgement(id, question, &setup.judge_model, answer, confidence, now)
            .is_ok()
        {
            asked += 1;
        }
    }
    asked
}

/// Starts a judging pass on its own thread unless one is running, a rep or
/// scene is open, or Coach is off. Tells the change listener when it asked.
pub fn judge_in_background(settings: Settings) {
    if !settings.coach_enabled
        || SESSION_ACTIVE.load(Ordering::SeqCst)
        || JUDGING.swap(true, Ordering::SeqCst)
    {
        return;
    }
    std::thread::spawn(move || {
        let asked = (|| {
            let llm = llm_client(&settings)?;
            let store = Store::open_default().ok()?;
            Some(judge_pass(
                &store,
                &settings,
                llm.as_ref(),
                now_unix(),
                &CoachCalendar::Local,
            ))
        })()
        .unwrap_or(0);
        JUDGING.store(false, Ordering::SeqCst);
        if asked > 0 {
            notify_changed();
        }
    });
}

// ---- Setting Coach up ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum SetupStep {
    Idle,
    Downloading {
        model: String,
        fraction: Option<f32>,
    },
    CheckingJudge {
        model: String,
        done: usize,
        total: usize,
    },
    Done,
    Failed {
        message: String,
    },
}

/// Where model setup stands: the recommended models, which are installed,
/// the recommended judge and whether everything is in place.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetupStatus {
    pub server_running: bool,
    pub installed: Vec<String>,
    pub models: Vec<models::CoachStarterModel>,
    pub missing: Vec<&'static str>,
    pub judge: &'static str,
    pub judge_grade: Option<JudgeGrade>,
    pub is_set_up: bool,
    pub provider_name: &'static str,
    pub supports_install: bool,
    pub memory: u64,
}

pub fn setup_status(
    store: &Store,
    settings: &Settings,
    llm: Option<&dyn LlmClient>,
) -> SetupStatus {
    let memory = models::memory();
    let grades = models::judge_grades(store);
    let setup = CoachModelSetup::current(settings, &grades, memory);
    let installed = llm.and_then(models::installed);
    let recommended = models::recommended_models(memory);
    let judge = models::recommended_judge(memory);
    let have = installed.clone().unwrap_or_default();
    let missing: Vec<&'static str> = recommended
        .iter()
        .map(|m| m.name)
        .filter(|n| !have.iter().any(|h| h == n))
        .collect();
    let judge_grade = models::judge_grade(&grades, judge).cloned();
    let is_set_up = installed.is_some()
        && missing.is_empty()
        && judge_grade.is_some()
        && setup.scene_model == models::STARTER_PARTNER
        && models::judge_trusted(&setup, &grades);
    SetupStatus {
        server_running: installed.is_some(),
        installed: have,
        models: recommended,
        missing,
        judge,
        judge_grade,
        is_set_up,
        provider_name: setup.provider_name,
        supports_install: setup.supports_install(),
        memory,
    }
}

/// Downloads what's missing, checks the recommended judge, and points each
/// job at its model. Returns the settings patch to apply on success.
pub fn set_up_models(
    store: &Store,
    settings: &Settings,
    llm: Option<&dyn LlmClient>,
    on_step: &mut dyn FnMut(&SetupStep),
) -> std::result::Result<serde_json::Value, String> {
    let status = setup_status(store, settings, llm);
    let fail = |message: String, on_step: &mut dyn FnMut(&SetupStep)| {
        on_step(&SetupStep::Failed {
            message: message.clone(),
        });
        Err(message)
    };
    let Some(llm) = llm.filter(|_| status.server_running) else {
        return fail("Start Ollama first.".into(), on_step);
    };
    for model in &status.missing {
        if !status.supports_install {
            return fail(
                format!(
                    "Download {model} in {}, then try again.",
                    status.provider_name
                ),
                on_step,
            );
        }
        on_step(&SetupStep::Downloading {
            model: model.to_string(),
            fraction: None,
        });
        let mut progress = |f: f32| {
            on_step(&SetupStep::Downloading {
                model: model.to_string(),
                fraction: Some(f.clamp(0.0, 1.0)),
            })
        };
        if let Err(e) = llm.pull(model, &mut progress) {
            return fail(format!("{model} didn't download: {e}"), on_step);
        }
    }
    if status.judge_grade.is_none() {
        let total = EXAMPLES.len();
        on_step(&SetupStep::CheckingJudge {
            model: status.judge.to_string(),
            done: 0,
            total,
        });
        check_judges(store, settings, llm, &[status.judge.to_string()]);
    }
    on_step(&SetupStep::Done);
    // Automatic: the best model that passed the judge check.
    Ok(serde_json::json!({
        "coach_partner_model": models::STARTER_PARTNER,
        "coach_model": "",
        "coach_judge_model": "",
        "coach_model_setup_seen": true,
    }))
}

// ---- Share cards ---------------------------------------------------------

/// Saves a share card (PNG) into Pictures, else Downloads, else home, and
/// returns where it went. The name is kept to safe characters.
pub fn save_card(file_name: &str, png: &[u8]) -> Result<std::path::PathBuf> {
    let folder = directories::UserDirs::new()
        .and_then(|d| {
            d.picture_dir()
                .or(d.download_dir())
                .map(std::path::Path::to_path_buf)
        })
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
        .ok_or_else(|| std::io::Error::other("no folder to save into"))?;
    let mut name: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    if name.trim_matches('.').is_empty() {
        name = "Orttaai-Coach".into();
    }
    if !name.to_ascii_lowercase().ends_with(".png") {
        name.push_str(".png");
    }
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(name);
    std::fs::write(&path, png)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::calibration;
    use crate::llm::{LlmModel, MockLlm};
    use crate::store::TranscriptionRecord;

    fn settings() -> Settings {
        Settings {
            coach_enabled: true,
            ..Settings::default()
        }
    }

    fn add(store: &Store, text: &str, app: &str, at: i64) -> i64 {
        let mut r = TranscriptionRecord::new(text, Some(app.into()), 8_000, at);
        r.raw_text = Some(text.into());
        store.insert_transcription(&r).unwrap()
    }

    fn pass(store: &Store, model: &str) {
        let g = JudgeGrade {
            model: model.into(),
            accuracy: 1.0,
            yes_recall: 1.0,
            no_recall: 1.0,
            answered: 52,
            total: 52,
            ms_per_answer: Some(10),
            version: calibration::VERSION,
            graded_at: 0,
        };
        models::save_judge_grade(store, &g).unwrap();
    }

    const TEXT: &str = "Um so add a retry to the paste step, it fails when the app is busy and I want it to try once more.";

    #[test]
    fn counting_skips_apps_switched_off_and_edits() {
        let store = Store::open_in_memory().unwrap();
        let mut s = settings();
        s.coach_app_overrides
            .insert("name:slack".into(), "off".into());
        let a = add(&store, TEXT, "Claude", 100);
        let b = add(&store, TEXT, "Slack", 200);
        assert_eq!(catch_up(&store, &s).unwrap(), 1);
        assert_eq!(catch_up(&store, &s).unwrap(), 0, "skipped rows don't loop");
        assert!(!record(&store, b, &s).unwrap());
        assert!(record(&store, a, &s).unwrap());
        let inputs = store.coach_inputs().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].measures.um_uh, 1);
        s.coach_enabled = false;
        assert!(!record(&store, a, &s).unwrap());
        assert!(evaluate(&store, &s, 1_000, &CoachCalendar::Fixed(0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn evaluate_and_the_month_from_the_store() {
        let store = Store::open_in_memory().unwrap();
        let now = 1_800_000_000;
        for i in 0..12 {
            add(&store, TEXT, "Claude", now - i * 3_600);
        }
        store
            .start_coach_habit(
                CoachHabitTarget::Measure(crate::coach::measure::CoachMeasure::Fillers),
                Some(CoachMode::Ai),
                now - 86_400,
            )
            .unwrap();
        let s = settings();
        let snap = evaluate(&store, &s, now, &CoachCalendar::Fixed(0))
            .unwrap()
            .unwrap();
        assert_eq!(snap.dictations, 12);
        assert_eq!(snap.recent_by_mode[&CoachMode::Ai], 12);
        assert_eq!(snap.habits.len(), 1);
        assert_eq!(snap.apps[0].display_name, "Claude");
        let report = month_report(&store, &s, now, now, &CoachCalendar::Fixed(0)).unwrap();
        assert!(report.dictations >= 1);
        let (question, texts) = comparison_sample(&store, &s, CoachMode::Ai, now, 20).unwrap();
        assert_eq!(question, CoachQuestion::TaskFirst);
        assert_eq!(texts.len(), 12);
    }

    #[test]
    fn background_judging_respects_the_budget_and_caches() {
        let store = Store::open_in_memory().unwrap();
        let now = 1_800_000_000;
        for i in 0..30 {
            add(&store, TEXT, "Claude", now - i * 3_600);
        }
        let s = settings();
        catch_up(&store, &s).unwrap();
        let llm = MockLlm::fixed(r#"{"answer":"yes","confidence":0.9}"#);
        let cal = CoachCalendar::Fixed(0);
        // Not trusted yet: nothing is asked.
        assert_eq!(judge_pass(&store, &s, &llm, now, &cal), 0);
        let judge = CoachModelSetup::current(&s, &BTreeMap::new(), models::memory()).judge_model;
        pass(&store, &judge);
        let llm = MockLlm::with(|_, _| Ok(r#"{"answer":"yes","confidence":0.9}"#.into()));
        let llm = InstalledMock {
            inner: llm,
            model: judge.clone(),
        };
        assert_eq!(judge_pass(&store, &s, &llm, now, &cal), 10);
        assert_eq!(judge_pass(&store, &s, &llm, now, &cal), 10);
        assert_eq!(store.count_coach_judgements_since(0).unwrap(), 20);
        // 40 a day at most.
        judge_pass(&store, &s, &llm, now, &cal);
        judge_pass(&store, &s, &llm, now, &cal);
        assert_eq!(judge_pass(&store, &s, &llm, now, &cal), 0);
        assert_eq!(store.count_coach_judgements_since(0).unwrap(), 40);
        let scores = store.coach_judgement_scores(&judge).unwrap();
        assert!(scores.contains_key(&CoachQuestion::TaskFirst));
    }

    /// A mock whose server has the judge installed.
    struct InstalledMock {
        inner: MockLlm,
        model: String,
    }

    impl LlmClient for InstalledMock {
        fn provider_name(&self) -> &str {
            "mock"
        }
        fn list_models(&self) -> crate::Result<Vec<LlmModel>> {
            Ok(vec![LlmModel {
                name: self.model.clone(),
                size: None,
                embedding: false,
            }])
        }
        fn chat(
            &self,
            m: &[crate::llm::ChatMessage],
            o: &crate::llm::ChatOptions,
            d: Option<&mut dyn FnMut(crate::llm::ChatDelta)>,
        ) -> crate::Result<crate::llm::ChatReply> {
            self.inner.chat(m, o, d)
        }
        fn embed(&self, model: &str, inputs: &[String]) -> crate::Result<Vec<Vec<f32>>> {
            self.inner.embed(model, inputs)
        }
        fn pull(&self, _model: &str, on_progress: &mut dyn FnMut(f32)) -> crate::Result<()> {
            on_progress(0.5);
            on_progress(1.0);
            Ok(())
        }
    }

    /// Tests that run the judge check share its global progress.
    static CHECK_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn judge_check_saves_grades_and_reports_state() {
        let _guard = CHECK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        let s = settings();
        let judge = CoachModelSetup::current(&s, &BTreeMap::new(), models::memory()).judge_model;
        let llm = InstalledMock {
            inner: MockLlm::with(|msgs, _| {
                let prompt = &msgs.last().unwrap().content;
                let e = EXAMPLES.iter().find(|e| prompt.contains(e.text)).unwrap();
                Ok(format!(
                    r#"{{"answer":"{}","confidence":1}}"#,
                    e.expected.id()
                ))
            }),
            model: judge.clone(),
        };
        assert!(matches!(
            judge_state(&store, &s, Some(&llm)),
            CoachJudgeState::Unchecked { .. }
        ));
        let grades = check_judges(&store, &s, &llm, std::slice::from_ref(&judge));
        assert!(grades[0].passes());
        assert!(judge_state(&store, &s, Some(&llm)).is_trusted());
        assert_eq!(judge_state(&store, &s, None), CoachJudgeState::NoServer);
        assert!(checking().is_none());
    }

    #[test]
    fn model_setup_pulls_what_is_missing() {
        let _guard = CHECK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        let s = settings();
        let mut steps = Vec::new();
        assert!(set_up_models(&store, &s, None, &mut |st| steps.push(st.clone())).is_err());
        assert_eq!(
            steps,
            vec![SetupStep::Failed {
                message: "Start Ollama first.".into()
            }]
        );

        // A server without pull support names the model.
        let no_pull = MockLlm::fixed(r#"{"answer":"yes","confidence":1}"#);
        let err = set_up_models(&store, &s, Some(&no_pull), &mut |_| {}).unwrap_err();
        assert!(err.contains(models::STARTER_PARTNER), "{err}");

        let judge = models::recommended_judge(models::memory());
        let llm = InstalledMock {
            inner: MockLlm::with(|msgs, _| {
                let prompt = &msgs.last().unwrap().content;
                let e = EXAMPLES.iter().find(|e| prompt.contains(e.text)).unwrap();
                Ok(format!(
                    r#"{{"answer":"{}","confidence":1}}"#,
                    e.expected.id()
                ))
            }),
            model: "something-else".into(),
        };
        let mut steps = Vec::new();
        let patch =
            set_up_models(&store, &s, Some(&llm), &mut |st| steps.push(st.clone())).unwrap();
        assert!(steps
            .iter()
            .any(|st| matches!(st, SetupStep::Downloading { fraction: Some(f), .. } if *f == 1.0)));
        assert_eq!(steps.last(), Some(&SetupStep::Done));
        assert_eq!(patch["coach_partner_model"], models::STARTER_PARTNER);
        assert!(models::judge_grade(&models::judge_grades(&store), judge).is_some());
    }
}
