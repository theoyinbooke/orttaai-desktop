//! A practice rep from plan to reveal: what to do, scoring it on the track's
//! checks (counted, and judged by the local model one question at a time),
//! the coach's note, and the numbers kept. Scenes end the same way.

use super::calibration::JudgeGrade;
use super::counting::{self, CoachDictation, DictationMeasures};
use super::measure::CoachSummary;
use super::mode::CoachMode;
use super::models::{self, CoachModelSetup};
use super::persona::CoachPersona;
use super::records::{CoachQuote, CoachRepRecord};
use super::scene::{CoachScene, SceneTurn, Speaker};
use super::scoring::{self, note_guard, CheckResult, JudgeAnswer};
use super::skill::{CoachCheck, CoachRepKind, CoachSkill, CoachTrack};
use crate::llm::LlmClient;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A rep, ready to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoachRepPlan {
    pub skill_id: String,
    pub kind: CoachRepKind,
    pub spoken: bool,
    /// What to do, in a sentence.
    pub instruction: String,
    /// For a redo: the dictation to say again.
    pub source: Option<PlanQuote>,
    pub persona: CoachPersona,
    /// The way of talking the rep counts for.
    pub mode: Option<CoachMode>,
}

/// A quote as a plan carries it (round-trips through the UI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanQuote {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
    pub app_name: Option<String>,
}

impl From<CoachQuote> for PlanQuote {
    fn from(q: CoachQuote) -> Self {
        Self {
            id: q.id,
            text: q.text,
            created_at: q.created_at,
            app_name: q.app_name,
        }
    }
}

impl CoachRepPlan {
    pub fn skill(&self) -> Option<&'static CoachSkill> {
        CoachSkill::by_id(&self.skill_id)
    }
}

pub fn instruction(skill: &CoachSkill, kind: CoachRepKind, has_source: bool) -> String {
    match kind {
        CoachRepKind::Redo if has_source => match skill.track {
            CoachTrack::Prompting => format!(
                "Say it again so the agent knows exactly what to do. {}",
                skill.drill
            ),
            CoachTrack::Messages => format!(
                "Say it again so they can answer at a glance. {}",
                skill.drill
            ),
            CoachTrack::Delivery | CoachTrack::Explaining => {
                format!("Say it again, better. {}", skill.drill)
            }
        },
        CoachRepKind::Redo | CoachRepKind::Drill => {
            format!("{} {}", skill.track.open_brief(), skill.drill)
        }
        CoachRepKind::Timed => format!(
            "{} You have a minute. {}",
            skill.track.open_brief(),
            skill.drill
        ),
        CoachRepKind::Scene => skill.drill.to_string(),
    }
}

/// The next rep: the same skill, set up another way.
pub fn next_kind(kind: CoachRepKind) -> CoachRepKind {
    let order = CoachRepKind::PRACTICE;
    let i = order.iter().position(|k| *k == kind).unwrap_or(0);
    order[(i + 1) % order.len()]
}

/// One counted figure on the reveal, beside your usual.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachRepFigure {
    pub label: &'static str,
    pub value: String,
    pub usual: Option<String>,
    /// True when the rep did better than usual, false worse, `None` level.
    pub better: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachRepResult {
    pub plan: CoachRepPlan,
    /// What the attempt said, after Polish. Shown, never stored.
    pub text: String,
    pub measures: DictationMeasures,
    pub checks: Vec<CheckResult>,
    pub score: Option<f64>,
    pub usual: Option<f64>,
    pub skill_met: Option<bool>,
    pub note: String,
    /// The model that judged, `None` when only counted checks ran.
    pub judge_model: Option<String>,
    pub figures: Vec<CoachRepFigure>,
    pub duration_ms: i64,
}

impl CoachRepResult {
    /// The rep as kept: numbers and check results, never what was said.
    pub fn record(&self, now: i64) -> CoachRepRecord {
        CoachRepRecord {
            id: None,
            skill_id: self.plan.skill_id.clone(),
            kind: self.plan.kind.id().to_string(),
            spoken: self.duration_ms > 0,
            created_at: now,
            score: self.score,
            skill_met: self.skill_met,
            duration_ms: self.duration_ms,
            words: self.measures.words,
            checks: serde_json::to_string(&self.checks).unwrap_or_else(|_| "[]".into()),
            persona: self.plan.persona.id().to_string(),
        }
    }
}

pub fn figures(
    m: &DictationMeasures,
    usual: Option<&CoachSummary>,
    spoken: bool,
) -> Vec<CoachRepFigure> {
    let mut out = Vec::new();
    let rate = if m.words == 0 {
        0.0
    } else {
        m.um_uh as f64 / m.words as f64 * 100.0
    };
    if spoken {
        let usual_rate = usual.and_then(|u| (u.raw_dictations > 0).then(|| u.um_uh_rate()));
        out.push(CoachRepFigure {
            label: "Ums and uhs",
            value: format!("{rate:.1}"),
            usual: usual_rate.map(|r| format!("{r:.1}")),
            better: usual_rate.and_then(|u| {
                if rate < u {
                    Some(true)
                } else if rate > u {
                    Some(false)
                } else {
                    None
                }
            }),
        });
        if let Some(wpm) = m.wpm {
            out.push(CoachRepFigure {
                label: "Pace",
                value: wpm.to_string(),
                usual: usual.and_then(|u| u.pace).map(|p| p.to_string()),
                better: None,
            });
        }
    }
    if let Some(longest) = m.longest_sentence() {
        out.push(CoachRepFigure {
            label: "Longest sentence",
            value: longest.to_string(),
            usual: usual
                .and_then(|u| u.median_sentence)
                .map(|s| format!("{s} typical")),
            better: None,
        });
    }
    out.push(CoachRepFigure {
        label: "Words",
        value: m.words.to_string(),
        usual: None,
        better: None,
    });
    out
}

/// What scoring needs besides the attempt.
pub struct ScoreContext<'a> {
    /// `None` when there is no model server at all.
    pub llm: Option<&'a dyn LlmClient>,
    pub setup: &'a CoachModelSetup,
    pub grades: &'a BTreeMap<String, JudgeGrade>,
    /// Models on the server, `None` when it isn't running.
    pub installed: Option<&'a [String]>,
    /// The average of the skill's last five scores.
    pub usual: Option<f64>,
    /// Your usual dictations in this way of talking, for the figures.
    pub usual_summary: Option<&'a CoachSummary>,
}

/// Scores a rep. `on_phase` hears each step ("Checking: …", "Your coach is
/// writing…"). Fails only when the attempt is too short to score.
pub fn score_rep(
    cx: &ScoreContext<'_>,
    plan: &CoachRepPlan,
    heard: &str,
    written: &str,
    duration_ms: i64,
    spoken: bool,
    on_phase: &mut dyn FnMut(&str),
) -> std::result::Result<CoachRepResult, String> {
    let skill = plan
        .skill()
        .ok_or_else(|| format!("Unknown skill {}", plan.skill_id))?;
    let measures = DictationMeasures::measuring(&CoachDictation {
        created_at: 0,
        heard: heard.to_string(),
        written: written.to_string(),
        recording_ms: if spoken { duration_ms } else { 0 },
        raw_known: true,
    });
    if measures.words < 3 {
        return Err("That was too short to score. Try a few sentences.".into());
    }
    // Judged checks only with a judge that passed the judge check.
    let has_judged = skill.checks().iter().any(CoachCheck::is_judged);
    let judge_available = has_judged
        && models::judge_trusted(cx.setup, cx.grades)
        && cx.llm.is_some()
        && cx
            .installed
            .is_some_and(|i| i.contains(&cx.setup.judge_model));

    let mut results = Vec::new();
    for check in skill.checks() {
        match &check {
            CoachCheck::Counted(count) => results.push(CheckResult::counted(*count, &measures)),
            CoachCheck::Judged(question) => {
                let (Some(llm), true) = (cx.llm, judge_available) else {
                    results.push(CheckResult::new(check, None, None));
                    continue;
                };
                on_phase(&format!("Checking: {}…", question.label().to_lowercase()));
                let verdict = models::judge(
                    llm,
                    cx.setup,
                    cx.grades,
                    *question,
                    written,
                    Some(&plan.instruction),
                );
                results.push(CheckResult::new(
                    check,
                    verdict.map(|v| v.0),
                    verdict.map(|v| v.1),
                ));
            }
            // Only scenes have a goal.
            CoachCheck::Goal(_) => {}
        }
    }
    let score = scoring::score(&results);
    let skill_met = results
        .iter()
        .find(|r| r.check == skill.check)
        .and_then(|r| r.answer)
        .map(|a| a == JudgeAnswer::Yes);
    let missing = results
        .iter()
        .find(|r| r.check != skill.check && r.answer == Some(JudgeAnswer::No))
        .map(|r| r.check.label());

    // The coach's note: the model's if it keeps to the facts, else the coach's own line.
    let mut facts = Vec::new();
    let mut numbers = Vec::new();
    if let Some(s) = score {
        facts.push(format!("score {s:.1}"));
        numbers.push(s);
    }
    if let Some(u) = cx.usual {
        facts.push(format!("usual score for this skill {u:.1}"));
        numbers.push(u);
    }
    facts.push(format!("{} words", measures.words));
    numbers.push(measures.words as f64);
    if let Some(wpm) = measures.wpm {
        facts.push(format!("{wpm} words a minute"));
        numbers.push(wpm as f64);
    }
    facts.push(format!("{} um or uh", measures.um_uh));
    numbers.push(measures.um_uh as f64);
    on_phase("Your coach is writing…");
    let fallback = plan.persona.line(skill.title, skill_met, missing);
    let coach_available = judge_available || cx.installed.is_some_and(|i| !i.is_empty());
    let note = match (cx.llm, coach_available) {
        (Some(llm), true) => {
            let prompt = plan.persona.note_prompt(skill, &results, &facts, score);
            models::note(
                llm,
                cx.setup,
                &prompt,
                &note_guard::allowed_numbers(&numbers),
            )
            .unwrap_or(fallback)
        }
        _ => fallback,
    };

    Ok(CoachRepResult {
        plan: plan.clone(),
        text: written.to_string(),
        figures: figures(&measures, cx.usual_summary, spoken),
        measures,
        checks: results,
        score,
        usual: cx.usual,
        skill_met,
        note,
        judge_model: judge_available.then(|| cx.setup.judge_model.clone()),
        duration_ms,
    })
}

/// Ends a scene: your turns scored together on the track's checks, plus
/// whether you reached the goal, judged on the whole conversation.
pub fn score_scene(
    cx: &ScoreContext<'_>,
    scene: &CoachScene,
    skill: &CoachSkill,
    persona: CoachPersona,
    turns: &[SceneTurn],
    on_phase: &mut dyn FnMut(&str),
) -> std::result::Result<CoachRepResult, String> {
    let yours: Vec<&SceneTurn> = turns.iter().filter(|t| t.speaker == Speaker::You).collect();
    if yours.is_empty() {
        return Err("Say something first, then end the scene.".into());
    }
    let written = yours
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let said = yours
        .iter()
        .map(|t| t.heard.as_deref().unwrap_or(&t.text))
        .collect::<Vec<_>>()
        .join("\n");
    let spoken_ms = if yours.iter().all(|t| t.duration_ms.is_some()) {
        yours.iter().map(|t| t.duration_ms.unwrap_or(0)).sum()
    } else {
        0
    };
    let measures = DictationMeasures::measuring(&CoachDictation {
        created_at: 0,
        heard: said,
        written: written.clone(),
        recording_ms: spoken_ms,
        raw_known: true,
    });
    let transcript = turns
        .iter()
        .map(|t| {
            format!(
                "{}: {}",
                if t.speaker == Speaker::You {
                    "Speaker"
                } else {
                    scene.partner_name
                },
                t.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut results = Vec::new();
    let context = format!("In a conversation: {}", scene.goal);
    for check in scene.track.checks() {
        match &check {
            CoachCheck::Counted(count) => results.push(CheckResult::counted(*count, &measures)),
            CoachCheck::Judged(question) => {
                on_phase(&format!("Checking: {}…", question.label().to_lowercase()));
                let verdict = cx.llm.and_then(|llm| {
                    models::judge(
                        llm,
                        cx.setup,
                        cx.grades,
                        *question,
                        &written,
                        Some(&context),
                    )
                });
                results.push(CheckResult::new(
                    check,
                    verdict.map(|v| v.0),
                    verdict.map(|v| v.1),
                ));
            }
            CoachCheck::Goal(_) => {}
        }
    }
    on_phase("Checking the goal…");
    let role = scene.partner_role.to_lowercase();
    let goal_context = format!("{}, {role}", scene.partner_name);
    let goal = cx.llm.and_then(|llm| {
        models::judge_text(
            llm,
            cx.setup,
            cx.grades,
            scene.goal_question,
            &transcript,
            Some(&goal_context),
        )
    });
    results.push(CheckResult::new(
        CoachCheck::Goal(scene.id.to_string()),
        goal.map(|g| g.0),
        goal.map(|g| g.1),
    ));

    let score = scoring::score(&results);
    let skill_met = results
        .iter()
        .find(|r| r.check == skill.check)
        .and_then(|r| r.answer)
        .map(|a| a == JudgeAnswer::Yes);
    let goal_met = goal.map(|g| g.0 == JudgeAnswer::Yes);
    let missing = results
        .iter()
        .find(|r| r.answer == Some(JudgeAnswer::No))
        .map(|r| r.check.label());
    let mut numbers = vec![yours.len() as f64, measures.words as f64];
    let mut facts = vec![
        format!("{} turns", yours.len()),
        format!("{} words", measures.words),
        format!(
            "goal reached: {}",
            match goal_met {
                Some(true) => "yes",
                Some(false) => "no",
                None => "not known",
            }
        ),
    ];
    if let Some(s) = score {
        facts.push(format!("score {s:.1}"));
        numbers.push(s);
    }
    on_phase("Your coach is writing…");
    let fallback = persona.line(scene.goal, goal_met, missing);
    let note = cx
        .llm
        .and_then(|llm| {
            models::note(
                llm,
                cx.setup,
                &persona.note_prompt(skill, &results, &facts, score),
                &note_guard::allowed_numbers(&numbers),
            )
        })
        .unwrap_or(fallback);

    let plan = CoachRepPlan {
        skill_id: skill.id.to_string(),
        kind: CoachRepKind::Scene,
        spoken: spoken_ms > 0,
        instruction: scene.goal.to_string(),
        source: None,
        persona,
        mode: scene.track.mode(),
    };
    Ok(CoachRepResult {
        plan,
        text: written,
        figures: figures(&measures, None, spoken_ms > 0),
        measures,
        checks: results,
        score,
        usual: cx.usual,
        skill_met,
        note,
        judge_model: models::judge_trusted(cx.setup, cx.grades)
            .then(|| cx.setup.judge_model.clone()),
        duration_ms: spoken_ms,
    })
}

/// Words and sentences so far, for the typing chips.
pub fn draft_counts(text: &str) -> (usize, usize) {
    (
        counting::word_count(text),
        counting::sentence_lengths(text).len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::calibration;
    use crate::llm::MockLlm;
    use crate::settings::Settings;

    fn passed(model: &str) -> BTreeMap<String, JudgeGrade> {
        let g = JudgeGrade {
            model: model.into(),
            accuracy: 1.0,
            yes_recall: 1.0,
            no_recall: 1.0,
            answered: 52,
            total: 52,
            ms_per_answer: Some(100),
            version: calibration::VERSION,
            graded_at: 0,
        };
        [(model.to_string(), g)].into_iter().collect()
    }

    fn plan(skill: &str, kind: CoachRepKind) -> CoachRepPlan {
        let s = CoachSkill::by_id(skill).unwrap();
        CoachRepPlan {
            skill_id: skill.into(),
            kind,
            spoken: false,
            instruction: instruction(s, kind, false),
            source: None,
            persona: CoachPersona::Editor,
            mode: s.track.mode(),
        }
    }

    #[test]
    fn instructions_and_the_next_rep() {
        let s = CoachSkill::by_id("finish-line").unwrap();
        assert_eq!(
            instruction(s, CoachRepKind::Redo, true),
            "Say it again so the agent knows exactly what to do. End with how you'll know it's finished."
        );
        assert_eq!(
            instruction(s, CoachRepKind::Timed, false),
            "Brief an agent on something you're working on today. You have a minute. End with how you'll know it's finished."
        );
        assert_eq!(next_kind(CoachRepKind::Redo), CoachRepKind::Drill);
        assert_eq!(next_kind(CoachRepKind::Timed), CoachRepKind::Redo);
    }

    #[test]
    fn a_typed_rep_without_a_judge_scores_its_counted_checks() {
        let settings = Settings::default();
        let grades = BTreeMap::new();
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        let cx = ScoreContext {
            llm: None,
            setup: &setup,
            grades: &grades,
            installed: None,
            usual: None,
            usual_summary: None,
        };
        let p = plan("keep-short", CoachRepKind::Drill);
        let r = score_rep(
            &cx,
            &p,
            "Can you review the paste fix by 3? It ships tonight.",
            "Can you review the paste fix by 3? It ships tonight.",
            0,
            false,
            &mut |_| {},
        )
        .unwrap();
        // Only "60 words or fewer" could be told: 5.0.
        assert_eq!(r.score, Some(5.0));
        assert_eq!(r.skill_met, Some(true));
        assert_eq!(r.judge_model, None);
        assert_eq!(r.note, "Keep it short: done. Nothing to cut.");
        assert_eq!(r.checks.iter().filter(|c| c.answer.is_none()).count(), 3);
        assert!(score_rep(&cx, &p, "too short", "too short", 0, false, &mut |_| {}).is_err());
        let rec = r.record(9);
        assert!(!rec.spoken);
        assert_eq!(rec.checks.matches("q-").count(), 3);
    }

    #[test]
    fn a_trusted_judge_answers_each_check_and_the_coach_writes() {
        let settings = Settings::default();
        let grades = passed(models::STARTER_PARTNER);
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        let llm = MockLlm::with(|msgs, opts| {
            let prompt = &msgs.last().unwrap().content;
            if opts.json_schema.is_some() {
                let yes = prompt.contains("Question: Is the task");
                Ok(format!(
                    r#"{{"answer":"{}","confidence":0.8}}"#,
                    if yes { "yes" } else { "no" }
                ))
            } else {
                Ok("Strong start at 2.0, add the context next.".into())
            }
        });
        let installed = vec![models::STARTER_PARTNER.to_string()];
        let cx = ScoreContext {
            llm: Some(&llm),
            setup: &setup,
            grades: &grades,
            installed: Some(&installed),
            usual: Some(3.0),
            usual_summary: None,
        };
        let p = plan("task-first", CoachRepKind::Drill);
        let mut phases = Vec::new();
        let text = "Add a retry to the paste step so it tries once more after a short wait.";
        let r = score_rep(&cx, &p, text, text, 0, false, &mut |s| {
            phases.push(s.to_string())
        })
        .unwrap();
        // Task first yes; context, finish line and one ask no: 1 + 4 × 1/4.
        assert_eq!(r.checks[0].answer, Some(JudgeAnswer::Yes));
        assert_eq!(r.skill_met, Some(true));
        assert_eq!(r.score, Some(2.0));
        assert_eq!(r.judge_model.as_deref(), Some(models::STARTER_PARTNER));
        assert!(phases.iter().any(|p| p == "Checking: task stated first…"));
        assert_eq!(phases.last().unwrap(), "Your coach is writing…");
        // The note used a number it was given (2.0 is the score).
        assert_eq!(r.note, "Strong start at 2.0, add the context next.");
    }

    #[test]
    fn figures_compare_with_your_usual() {
        let m = DictationMeasures {
            words: 50,
            um_uh: 1,
            phrase_fillers: 0,
            wpm: Some(150),
            sentence_lengths: vec![10, 20],
            repeats: 0,
            polish_change_ratio: Some(0.0),
            raw_known: true,
        };
        let usual = CoachSummary::new(&[DictationMeasures {
            um_uh: 3,
            ..m.clone()
        }]);
        let f = figures(&m, Some(&usual), true);
        assert_eq!(f[0].label, "Ums and uhs");
        assert_eq!(f[0].value, "2.0");
        assert_eq!(f[0].better, Some(true));
        assert_eq!(f[1].value, "150");
        assert_eq!(f[2].usual.as_deref(), Some("20 typical"));
        assert_eq!(figures(&m, None, false).len(), 2);
    }

    #[test]
    fn a_scene_is_scored_with_its_goal() {
        let settings = Settings::default();
        let grades = passed(models::STARTER_PARTNER);
        let setup = CoachModelSetup::current(&settings, &grades, 16 << 30);
        let llm = MockLlm::fixed(r#"{"answer":"yes","confidence":0.9}"#);
        let installed = vec![models::STARTER_PARTNER.to_string()];
        let cx = ScoreContext {
            llm: Some(&llm),
            setup: &setup,
            grades: &grades,
            installed: Some(&installed),
            usual: None,
            usual_summary: None,
        };
        let scene = CoachScene::by_id("review-ask").unwrap();
        let skill = CoachSkill::by_id("ask-first").unwrap();
        let turns = vec![
            SceneTurn::new(Speaker::Partner, scene.opening),
            SceneTurn {
                speaker: Speaker::You,
                text: "Could you review my fix by 3?".into(),
                duration_ms: Some(3_000),
                heard: Some("Um could you review my fix by 3?".into()),
            },
        ];
        let r = score_scene(&cx, scene, skill, CoachPersona::Peer, &turns, &mut |_| {}).unwrap();
        assert_eq!(
            r.checks.last().unwrap().check,
            CoachCheck::Goal("review-ask".into())
        );
        assert_eq!(r.score, Some(5.0));
        assert_eq!(r.plan.kind, CoachRepKind::Scene);
        assert_eq!(r.duration_ms, 3_000);
        assert_eq!(r.measures.um_uh, 1);
        assert!(score_scene(
            &cx,
            scene,
            skill,
            CoachPersona::Peer,
            &turns[..1],
            &mut |_| {}
        )
        .is_err());
    }
}
