//! Coach's model jobs on the person's own local server (Ollama or LM Studio,
//! whichever Orttaai uses), all through [`LlmClient`]: the judge, one yes/no
//! question per call; the coach, who writes the note; and the scene partner.
//! A judge is only asked once it has passed Coach's judge check; without
//! one, reps are scored on the counted checks and the coach's own lines.

use super::calibration::{self, JudgeGrade, EXAMPLES};
use super::mode::{self, CoachMode, Overrides};
use super::persona::CoachPersona;
use super::scene::{partner_prompt, CoachScene, SceneTurn};
use super::scoring::{judge_prompt, note_guard, JudgeAnswer};
use super::skill::CoachQuestion;
use crate::llm::{ChatDelta, ChatMessage, ChatOptions, LlmClient};
use crate::settings::{LlmProvider, Settings};
use crate::store::{Store, COACH_JUDGE_GRADES_KEY};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

// ---- Preferences (from Settings) -----------------------------------------

pub fn overrides(settings: &Settings) -> Overrides {
    mode::parse_overrides(&settings.coach_app_overrides)
}

/// The coach for a way of talking: its own pick, else the AI seat's, else
/// the default coach (Mentor unless chosen).
pub fn persona_for(settings: &Settings, mode: Option<CoachMode>) -> CoachPersona {
    let pick = |m: CoachMode| {
        settings
            .coach_personas
            .get(m.id())
            .and_then(|p| CoachPersona::from_id(p))
    };
    pick(mode.unwrap_or(CoachMode::Elsewhere))
        .or_else(|| pick(CoachMode::Ai))
        .or_else(|| CoachPersona::from_id(&settings.coach_persona))
        .unwrap_or(CoachPersona::Mentor)
}

/// How each model did on Coach's judge check, by model name.
pub fn judge_grades(store: &Store) -> BTreeMap<String, JudgeGrade> {
    store
        .kv_get(COACH_JUDGE_GRADES_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save_judge_grade(store: &Store, grade: &JudgeGrade) -> crate::Result<()> {
    let mut all = judge_grades(store);
    all.insert(grade.model.clone(), grade.clone());
    store.kv_set(COACH_JUDGE_GRADES_KEY, Some(&serde_json::to_string(&all)?))
}

/// A model's grade against the examples in use now.
pub fn judge_grade<'a>(
    grades: &'a BTreeMap<String, JudgeGrade>,
    model: &str,
) -> Option<&'a JudgeGrade> {
    grades.get(model).filter(|g| g.is_current())
}

/// The most accurate model that passed, then the fastest.
pub fn best_passing_judge(grades: &BTreeMap<String, JudgeGrade>) -> Option<&JudgeGrade> {
    let key = |g: &JudgeGrade| (g.accuracy, -(g.ms_per_answer.unwrap_or(i64::MAX) as f64));
    let mut best: Option<&JudgeGrade> = None;
    for g in grades.values().filter(|g| g.is_current() && g.passes()) {
        if best.is_none_or(|b| key(g).partial_cmp(&key(b)) == Some(std::cmp::Ordering::Greater)) {
            best = Some(g);
        }
    }
    best
}

// ---- Recommended models --------------------------------------------------

/// A model Coach recommends, and the jobs it does.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachStarterModel {
    pub name: &'static str,
    pub jobs: Vec<&'static str>,
    pub size: &'static str,
}

/// Quick enough to feel like talking: first sentence in about 0.05 s.
pub const STARTER_PARTNER: &str = "nemotron-3.5-bolt:e4b";
/// The best judge on the judge check (98%), but it needs the room.
pub const STARTER_BIG_JUDGE: &str = "gemma4:26b";
/// Memory for the big judge and the partner to stay loaded together.
pub const ROOM_FOR_BIG_JUDGE: u64 = 48 << 30;

/// Bolt plays the scene partners everywhere. With room for both, Gemma 4
/// 26B judges and writes the notes; otherwise Bolt does every job.
pub fn recommended_models(memory: u64) -> Vec<CoachStarterModel> {
    if memory >= ROOM_FOR_BIG_JUDGE {
        vec![
            CoachStarterModel {
                name: STARTER_PARTNER,
                jobs: vec!["Scene partner"],
                size: "7.2 GB",
            },
            CoachStarterModel {
                name: STARTER_BIG_JUDGE,
                jobs: vec!["Judge", "Coach"],
                size: "18.7 GB",
            },
        ]
    } else {
        vec![CoachStarterModel {
            name: STARTER_PARTNER,
            jobs: vec!["Scene partner", "Judge", "Coach"],
            size: "7.2 GB",
        }]
    }
}

pub fn recommended_judge(memory: u64) -> &'static str {
    if memory >= ROOM_FOR_BIG_JUDGE {
        STARTER_BIG_JUDGE
    } else {
        STARTER_PARTNER
    }
}

/// Total physical memory in bytes, or `None` when it can't be read.
pub fn total_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        meminfo_total(&info)
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: `status` is a properly sized, initialised MEMORYSTATUSEX.
        unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
        Some(status.ullTotalPhys)
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        None
    }
}

/// `MemTotal:  16314544 kB` → bytes.
pub fn meminfo_total(info: &str) -> Option<u64> {
    let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Memory to plan with: what's there, or 16 GB when it can't be told.
pub fn memory() -> u64 {
    total_memory().unwrap_or(16 << 30)
}

// ---- The setup in use ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachModelSetup {
    /// `ollama | lm_studio`.
    pub provider: &'static str,
    pub provider_name: &'static str,
    pub endpoint: String,
    pub coach_model: String,
    pub judge_model: String,
    /// Plays the scene partners; the coach model unless one was picked.
    pub partner_model: String,
    pub scene_model: String,
    /// The coach and judge were left on automatic.
    pub coach_automatic: bool,
    pub judge_automatic: bool,
    /// Ask reasoning models not to think (Ollama understands the flag).
    #[serde(skip)]
    pub think: Option<bool>,
}

impl CoachModelSetup {
    /// Unless the person picked them, both jobs go to the best model that
    /// passed the judge check, else the one recommended for this memory.
    pub fn current(
        settings: &Settings,
        grades: &BTreeMap<String, JudgeGrade>,
        memory: u64,
    ) -> Self {
        let automatic = best_passing_judge(grades)
            .map(|g| g.model.clone())
            .unwrap_or_else(|| recommended_judge(memory).to_string());
        let coach = settings.coach_model.trim().to_string();
        let judge = settings.coach_judge_model.trim().to_string();
        let partner = settings.coach_partner_model.trim().to_string();
        let coach_model = if coach.is_empty() {
            automatic.clone()
        } else {
            coach.clone()
        };
        let (provider, provider_name, think) = match settings.llm_provider {
            LlmProvider::Ollama => ("ollama", "Ollama", Some(false)),
            LlmProvider::LmStudio => ("lm_studio", "LM Studio", None),
        };
        Self {
            provider,
            provider_name,
            endpoint: settings.llm_endpoint().to_string(),
            scene_model: if partner.is_empty() {
                coach_model.clone()
            } else {
                partner.clone()
            },
            coach_model,
            judge_model: if judge.is_empty() {
                automatic
            } else {
                judge.clone()
            },
            partner_model: partner,
            coach_automatic: coach.is_empty(),
            judge_automatic: judge.is_empty(),
            think,
        }
    }

    pub fn supports_install(&self) -> bool {
        self.provider == "ollama"
    }

    fn opts(
        &self,
        model: &str,
        temperature: f32,
        max_tokens: u32,
        num_ctx: u32,
        timeout_ms: u64,
    ) -> ChatOptions {
        ChatOptions {
            model: model.to_string(),
            temperature: Some(temperature),
            think: self.think,
            json_schema: None,
            max_tokens: Some(max_tokens),
            num_ctx: Some(num_ctx),
            timeout_ms: Some(timeout_ms),
            keep_alive_secs: None,
        }
    }
}

/// The judge passed Coach's check against the current examples.
pub fn judge_trusted(setup: &CoachModelSetup, grades: &BTreeMap<String, JudgeGrade>) -> bool {
    judge_grade(grades, &setup.judge_model).is_some_and(JudgeGrade::passes)
}

// ---- The jobs ------------------------------------------------------------

/// Models on the server, or `None` when it isn't running.
pub fn installed(llm: &dyn LlmClient) -> Option<Vec<String>> {
    llm.list_models()
        .ok()
        .map(|models| models.into_iter().map(|m| m.name).collect())
}

/// Installed models that can chat (embedding models left out).
pub fn chat_models(installed: &[String]) -> Vec<String> {
    installed
        .iter()
        .filter(|m| !m.to_lowercase().contains("embed"))
        .cloned()
        .collect()
}

fn json_opts(
    setup: &CoachModelSetup,
    model: &str,
    schema: &str,
    max_tokens: u32,
    timeout_ms: u64,
) -> ChatOptions {
    let mut opts = setup.opts(model, 0.0, max_tokens, 8_192, timeout_ms);
    opts.json_schema = serde_json::from_str(schema).ok();
    opts
}

/// Any model's answer, trusted or not: for grading and comparing.
pub fn ask(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    question_text: &str,
    text: &str,
    context: Option<&str>,
    model: &str,
) -> Option<(JudgeAnswer, f64)> {
    let prompt = judge_prompt::prompt(question_text, text, context);
    let opts = json_opts(setup, model, judge_prompt::JSON_SCHEMA, 60, 60_000);
    match llm.complete("", &prompt, &opts) {
        Ok(reply) => judge_prompt::parse(&reply),
        Err(e) => {
            tracing::info!("Coach judge skipped a check: {e}");
            None
        }
    }
}

/// The judge's answer to a question asked in its own words, or `None` when
/// it couldn't say or hasn't passed the judge check.
pub fn judge_text(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    grades: &BTreeMap<String, JudgeGrade>,
    question_text: &str,
    text: &str,
    context: Option<&str>,
) -> Option<(JudgeAnswer, f64)> {
    if !judge_trusted(setup, grades) {
        return None;
    }
    ask(llm, setup, question_text, text, context, &setup.judge_model)
}

pub fn judge(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    grades: &BTreeMap<String, JudgeGrade>,
    question: CoachQuestion,
    text: &str,
    context: Option<&str>,
) -> Option<(JudgeAnswer, f64)> {
    judge_text(llm, setup, grades, question.question(), text, context)
}

/// Grades a model on every calibration example, one question at a time.
pub fn grade(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    model: &str,
    progress: &mut dyn FnMut(usize, usize),
    now: i64,
) -> JudgeGrade {
    let mut answers = Vec::with_capacity(EXAMPLES.len());
    let mut timings: Vec<i64> = Vec::new();
    for (index, example) in EXAMPLES.iter().enumerate() {
        progress(index, EXAMPLES.len());
        let start = Instant::now();
        let verdict = ask(
            llm,
            setup,
            example.question.question(),
            example.text,
            None,
            model,
        );
        // The first answer also loads the model; it isn't a fair time.
        if verdict.is_some() && index > 0 {
            timings.push(start.elapsed().as_millis() as i64);
        }
        answers.push((example, verdict.map(|v| v.0)));
    }
    progress(EXAMPLES.len(), EXAMPLES.len());
    let ms = if timings.is_empty() {
        None
    } else {
        Some(timings.iter().sum::<i64>() / timings.len() as i64)
    };
    calibration::grade(model, &answers, ms, now)
}

/// The coach model's note, if it wrote one that keeps to the facts.
pub fn note(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    prompt: &str,
    allowed: &BTreeSet<String>,
) -> Option<String> {
    let opts = setup.opts(&setup.coach_model, 0.6, 120, 4_096, 25_000);
    match llm.complete("", prompt, &opts) {
        Ok(reply) => {
            let note = note_guard::cleaned(&reply);
            note_guard::accepts(&note, allowed).then_some(note)
        }
        Err(e) => {
            tracing::info!("Coach note fell back to the coach's line: {e}");
            None
        }
    }
}

/// The scene partner's reply, streamed: `on_text` gets the reply so far as it
/// grows, without a name the model added. `None` when the model didn't answer.
pub fn partner_reply(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    scene: &CoachScene,
    turns: &[SceneTurn],
    on_text: &mut dyn FnMut(&str),
) -> Option<String> {
    let messages = partner_prompt::chat(scene, turns);
    let opts = setup.opts(&setup.scene_model, 0.7, 160, 8_192, 30_000);
    let mut so_far = String::new();
    let mut on_delta = |delta: ChatDelta| {
        if let ChatDelta::Content(piece) = delta {
            so_far.push_str(&piece);
            on_text(&partner_prompt::without_name(&so_far, scene.partner_name));
        }
    };
    match llm.chat(&messages, &opts, Some(&mut on_delta)) {
        Ok(reply) => {
            let text = partner_prompt::without_name(&reply.content, scene.partner_name);
            (!text.is_empty()).then_some(text)
        }
        Err(e) => {
            tracing::info!("Coach scene partner didn't answer: {e}");
            None
        }
    }
}

/// How the partner feels after its last line.
pub fn partner_mood(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    scene: &CoachScene,
    turns: &[SceneTurn],
) -> Option<super::scene::PartnerMood> {
    let opts = json_opts(
        setup,
        &setup.scene_model,
        partner_prompt::MOOD_SCHEMA,
        20,
        15_000,
    );
    let reply = llm
        .complete("", &partner_prompt::mood_prompt(scene, turns), &opts)
        .ok()?;
    partner_prompt::parse_mood(&reply)
}

/// Loads the scene model so its first reply isn't kept waiting.
pub fn warm_partner(llm: &dyn LlmClient, setup: &CoachModelSetup) {
    let opts = setup.opts(&setup.scene_model, 0.0, 1, 8_192, 60_000);
    let _ = llm.chat(&[ChatMessage::user("Hello.")], &opts, None);
}

/// How each model does as judge on the same dictations: how often it agrees
/// with the others, and how long an answer takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachModelComparison {
    pub model: String,
    /// Share of dictations where its answer matched the majority.
    pub agreement: Option<f64>,
    pub ms_per_answer: Option<i64>,
    pub answered: usize,
}

pub fn compare(
    llm: &dyn LlmClient,
    setup: &CoachModelSetup,
    models: &[String],
    texts: &[String],
    question: CoachQuestion,
    progress: &mut dyn FnMut(&str),
) -> Vec<CoachModelComparison> {
    let mut answers: BTreeMap<&str, Vec<Option<JudgeAnswer>>> = BTreeMap::new();
    let mut timings: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for model in models {
        progress(model);
        let mut mine = Vec::new();
        for text in texts {
            let start = Instant::now();
            let verdict = ask(llm, setup, question.question(), text, None, model);
            mine.push(verdict.map(|v| v.0));
            if verdict.is_some() {
                timings
                    .entry(model)
                    .or_default()
                    .push(start.elapsed().as_millis() as i64);
            }
        }
        answers.insert(model, mine);
    }
    // The most common answer per dictation; on a tie, the first answer to reach it.
    let majority: Vec<Option<JudgeAnswer>> = (0..texts.len())
        .map(|i| {
            let votes: Vec<JudgeAnswer> = models
                .iter()
                .filter_map(|m| answers.get(m.as_str())?.get(i).copied().flatten())
                .collect();
            let mut best: Option<(JudgeAnswer, usize)> = None;
            for v in &votes {
                let n = votes.iter().filter(|x| *x == v).count();
                if best.is_none_or(|(_, c)| n > c) {
                    best = Some((*v, n));
                }
            }
            best.map(|b| b.0)
        })
        .collect();
    let mut rows: Vec<CoachModelComparison> = models
        .iter()
        .map(|model| {
            let mine = answers.get(model.as_str()).cloned().unwrap_or_default();
            let pairs: Vec<bool> = mine
                .iter()
                .zip(&majority)
                .filter_map(|(a, m)| Some(a.as_ref()? == m.as_ref()?))
                .collect();
            let times = timings.get(model.as_str()).cloned().unwrap_or_default();
            CoachModelComparison {
                model: model.clone(),
                agreement: if pairs.is_empty() {
                    None
                } else {
                    Some(pairs.iter().filter(|&&p| p).count() as f64 / pairs.len() as f64)
                },
                ms_per_answer: if times.is_empty() {
                    None
                } else {
                    Some(times.iter().sum::<i64>() / times.len() as i64)
                },
                answered: pairs.len(),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        let key = |r: &CoachModelComparison| {
            (
                r.agreement.unwrap_or(-1.0),
                -(r.ms_per_answer.unwrap_or(i64::MAX) as f64),
            )
        };
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// Whether Coach has a judge it can trust.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CoachJudgeState {
    Unknown,
    NoServer,
    NotInstalled {
        model: String,
    },
    Checking {
        model: String,
        done: usize,
        total: usize,
    },
    Passed {
        grade: JudgeGrade,
    },
    Failed {
        grade: JudgeGrade,
    },
    /// Installed but never checked against the current examples.
    Unchecked {
        model: String,
    },
}

impl CoachJudgeState {
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }
}

/// The judge's state without running anything (the caller starts a check
/// when it comes back `Unchecked`).
pub fn judge_state(
    installed: Option<&[String]>,
    setup: &CoachModelSetup,
    grades: &BTreeMap<String, JudgeGrade>,
) -> CoachJudgeState {
    let Some(installed) = installed else {
        return CoachJudgeState::NoServer;
    };
    if !installed.contains(&setup.judge_model) {
        return CoachJudgeState::NotInstalled {
            model: setup.judge_model.clone(),
        };
    }
    match judge_grade(grades, &setup.judge_model) {
        Some(g) if g.passes() => CoachJudgeState::Passed { grade: g.clone() },
        Some(g) => CoachJudgeState::Failed { grade: g.clone() },
        None => CoachJudgeState::Unchecked {
            model: setup.judge_model.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    fn grades_with(model: &str, pass: bool) -> BTreeMap<String, JudgeGrade> {
        let g = JudgeGrade {
            model: model.into(),
            accuracy: if pass { 0.95 } else { 0.5 },
            yes_recall: if pass { 1.0 } else { 0.0 },
            no_recall: 1.0,
            answered: 52,
            total: 52,
            ms_per_answer: Some(200),
            version: calibration::VERSION,
            graded_at: 0,
        };
        [(model.to_string(), g)].into_iter().collect()
    }

    #[test]
    fn recommendations_follow_memory() {
        assert_eq!(recommended_models(64 << 30).len(), 2);
        assert_eq!(recommended_judge(64 << 30), STARTER_BIG_JUDGE);
        assert_eq!(
            recommended_models(16 << 30)[0].jobs,
            vec!["Scene partner", "Judge", "Coach"]
        );
        assert_eq!(recommended_judge(16 << 30), STARTER_PARTNER);
        assert_eq!(
            meminfo_total("MemTotal:       16314544 kB\nMemFree: 1 kB"),
            Some(16_314_544 * 1024)
        );
        assert_eq!(meminfo_total("nothing"), None);
    }

    #[test]
    fn automatic_jobs_use_the_best_passing_judge() {
        let mut settings = Settings::default();
        let none = BTreeMap::new();
        let setup = CoachModelSetup::current(&settings, &none, 16 << 30);
        assert_eq!(setup.judge_model, STARTER_PARTNER);
        assert_eq!(setup.coach_model, STARTER_PARTNER);
        assert!(setup.judge_automatic);
        let grades = grades_with("gemma4:26b", true);
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        assert_eq!(setup.judge_model, "gemma4:26b");
        assert!(judge_trusted(&setup, &grades));
        settings.coach_judge_model = "other".into();
        settings.coach_partner_model = String::new();
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        assert_eq!(setup.judge_model, "other");
        assert_eq!(setup.scene_model, "gemma4:26b");
        assert!(!judge_trusted(&setup, &grades));
    }

    #[test]
    fn personas_fall_back_seat_by_seat() {
        let mut settings = Settings::default();
        assert_eq!(
            persona_for(&settings, Some(CoachMode::People)),
            CoachPersona::Mentor
        );
        settings.coach_persona = "peer".into();
        assert_eq!(persona_for(&settings, None), CoachPersona::Peer);
        settings.coach_personas.insert("ai".into(), "editor".into());
        settings
            .coach_personas
            .insert("people".into(), "sergeant".into());
        assert_eq!(
            persona_for(&settings, Some(CoachMode::People)),
            CoachPersona::Sergeant
        );
        assert_eq!(
            persona_for(&settings, Some(CoachMode::Writing)),
            CoachPersona::Editor
        );
    }

    #[test]
    fn untrusted_judges_are_never_asked() {
        let llm = MockLlm::fixed(r#"{"answer":"yes","confidence":0.9}"#);
        let settings = Settings::default();
        let failed = grades_with(STARTER_PARTNER, false);
        let setup = CoachModelSetup::current(&settings, &failed, 16 << 30);
        assert_eq!(
            judge(&llm, &setup, &failed, CoachQuestion::Tone, "Hi.", None),
            None
        );
        assert_eq!(llm.call_count(), 0);
        let passed = grades_with(STARTER_PARTNER, true);
        assert_eq!(
            judge(&llm, &setup, &passed, CoachQuestion::Tone, "Hi.", None),
            Some((JudgeAnswer::Yes, 0.9))
        );
        let (_, opts) = llm.calls.lock().unwrap()[0].clone();
        assert_eq!(opts.temperature, Some(0.0));
        assert!(opts.json_schema.is_some());
        assert_eq!(opts.think, Some(false));
    }

    #[test]
    fn grading_a_perfect_judge() {
        // Answer each example as the examples expect, from the prompt.
        let llm = MockLlm::with(|msgs, _| {
            let prompt = &msgs.last().unwrap().content;
            let example = EXAMPLES.iter().find(|e| prompt.contains(e.text)).unwrap();
            Ok(format!(
                r#"{{"answer":"{}","confidence":0.9}}"#,
                example.expected.id()
            ))
        });
        let setup = CoachModelSetup::current(&Settings::default(), &BTreeMap::new(), 16 << 30);
        let mut seen = 0;
        let g = grade(&llm, &setup, "m", &mut |done, _| seen = done, 5);
        assert!(g.passes());
        assert_eq!(seen, EXAMPLES.len());
        assert_eq!(llm.call_count(), EXAMPLES.len());
        let offline = MockLlm::offline();
        assert!(!grade(&offline, &setup, "m", &mut |_, _| {}, 5).passes());
    }

    #[test]
    fn notes_keep_to_the_facts() {
        let setup = CoachModelSetup::current(&Settings::default(), &BTreeMap::new(), 16 << 30);
        let allowed = note_guard::allowed_numbers(&[4.2]);
        assert_eq!(
            note(&MockLlm::fixed("\"Good: 4.2.\""), &setup, "p", &allowed).as_deref(),
            Some("Good: 4.2.")
        );
        assert_eq!(
            note(&MockLlm::fixed("You hit 97%."), &setup, "p", &allowed),
            None
        );
        assert_eq!(note(&MockLlm::offline(), &setup, "p", &allowed), None);
    }

    #[test]
    fn partner_streams_without_its_name() {
        let llm = MockLlm::fixed("Ola: Fine, the calendar first. Then the export.");
        let setup = CoachModelSetup::current(&Settings::default(), &BTreeMap::new(), 16 << 30);
        let scene = CoachScene::by_id("scope-push").unwrap();
        let mut seen = Vec::new();
        let reply = partner_reply(
            &llm,
            &setup,
            scene,
            &[SceneTurn::new(
                super::super::scene::Speaker::Partner,
                scene.opening,
            )],
            &mut |t| seen.push(t.to_string()),
        );
        assert_eq!(
            reply.as_deref(),
            Some("Fine, the calendar first. Then the export.")
        );
        assert!(seen.last().unwrap().starts_with("Fine"));
        let mood = MockLlm::fixed(r#"{"mood":"weighing"}"#);
        assert_eq!(
            partner_mood(&mood, &setup, scene, &[]),
            Some(super::super::scene::PartnerMood::Weighing)
        );
    }

    #[test]
    fn comparison_ranks_agreement_then_speed() {
        let llm = MockLlm::with(|_, opts| {
            Ok(if opts.model == "contrary" {
                r#"{"answer":"no","confidence":1}"#
            } else {
                r#"{"answer":"yes","confidence":1}"#
            }
            .to_string())
        });
        let setup = CoachModelSetup::current(&Settings::default(), &BTreeMap::new(), 16 << 30);
        let models = vec!["contrary".to_string(), "a".to_string(), "b".to_string()];
        let rows = compare(
            &llm,
            &setup,
            &models,
            &["one".into(), "two".into()],
            CoachQuestion::TaskFirst,
            &mut |_| {},
        );
        assert_eq!(rows.last().unwrap().model, "contrary");
        assert_eq!(rows.last().unwrap().agreement, Some(0.0));
        assert_eq!(rows[0].agreement, Some(1.0));
    }

    #[test]
    fn judge_state_from_server_and_grades() {
        let settings = Settings::default();
        let grades = grades_with(STARTER_PARTNER, true);
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        assert_eq!(
            judge_state(None, &setup, &grades),
            CoachJudgeState::NoServer
        );
        assert!(matches!(
            judge_state(Some(&[]), &setup, &grades),
            CoachJudgeState::NotInstalled { .. }
        ));
        assert!(judge_state(Some(&[STARTER_PARTNER.to_string()]), &setup, &grades).is_trusted());
        assert!(matches!(
            judge_state(
                Some(&[STARTER_PARTNER.to_string()]),
                &setup,
                &BTreeMap::new()
            ),
            CoachJudgeState::Unchecked { .. }
        ));
    }
}
