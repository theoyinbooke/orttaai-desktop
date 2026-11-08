//! Coach metrics, habits, reps and judgements. Owned by the coach area.
//!
//! Coach stores numbers keyed by the dictation's id, never text of its own;
//! quotes and the judge's input are read live from `transcription`.

use super::Store;
use crate::coach::counting::DictationMeasures;
use crate::coach::mode::{self, CoachMode};
use crate::coach::records::{
    CoachAppUsage, CoachHabitRecord, CoachHabitTarget, CoachIndexSource, CoachInput, CoachQuote,
    CoachRepRecord,
};
use crate::coach::scoring::JudgeAnswer;
use crate::coach::skill::CoachQuestion;
use crate::error::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::{BTreeMap, HashMap, HashSet};

/// kv key for the judge grades (`model → JudgeGrade` JSON).
pub const COACH_JUDGE_GRADES_KEY: &str = "coach.judge_grades";

fn placeholders(n: usize) -> String {
    vec!["?"; n.max(1)].join(",")
}

fn index_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<CoachIndexSource> {
    Ok(CoachIndexSource {
        id: r.get(0)?,
        created_at: r.get(1)?,
        text: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        raw_text: r.get(3)?,
        recording_ms: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
        app_id: r.get(5)?,
        app_name: r.get(6)?,
    })
}

impl Store {
    // ---- Counting ----------------------------------------------------------

    /// Dictations (not edit commands) with no counts yet, or counts from an
    /// older version of the rules, newest first, in pages from a cursor
    /// `(created_at, id)` exclusive. Apps switched off are skipped by the caller.
    pub fn coach_backlog_page(
        &self,
        before: Option<(i64, i64)>,
        limit: i64,
    ) -> Result<Vec<CoachIndexSource>> {
        let (at, id) = before.unwrap_or((i64::MAX, i64::MAX));
        let mut stmt = self.conn().prepare(
            "SELECT t.id, t.created_at, t.text, t.raw_text, t.recording_ms, t.app_id, t.app_name
             FROM transcription t
             LEFT JOIN coach_metric m ON m.transcription_id = t.id
             WHERE t.entry_kind != 'edit'
               AND (m.transcription_id IS NULL OR m.metrics_version < ?1)
               AND (t.created_at < ?2 OR (t.created_at = ?2 AND t.id < ?3))
             ORDER BY t.created_at DESC, t.id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![DictationMeasures::VERSION, at, id, limit],
            index_source,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One dictation as Coach counts it, or `None` for an edit command or a missing row.
    pub fn coach_index_source(&self, id: i64) -> Result<Option<CoachIndexSource>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, created_at, text, raw_text, recording_ms, app_id, app_name
                 FROM transcription WHERE id = ?1 AND entry_kind != 'edit'",
                [id],
                index_source,
            )
            .optional()?)
    }

    pub fn save_coach_metrics(&self, metrics: &[(i64, DictationMeasures)]) -> Result<()> {
        if metrics.is_empty() {
            return Ok(());
        }
        let tx = self.conn().unchecked_transaction()?;
        for (id, m) in metrics {
            // The dictation may have been deleted while it was counted.
            let exists: Option<i64> = tx
                .query_row("SELECT 1 FROM transcription WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                continue;
            }
            let lengths = m
                .sentence_lengths
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                "INSERT OR REPLACE INTO coach_metric
                 (transcription_id, metrics_version, words, um_uh, phrase_fillers, wpm, sentence_lengths,
                  repeats, polish_change_ratio, raw_known)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    DictationMeasures::VERSION,
                    m.words,
                    m.um_uh,
                    m.phrase_fillers,
                    m.wpm,
                    lengths,
                    m.repeats,
                    m.polish_change_ratio,
                    m.raw_known
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every counted dictation, oldest first, with its app.
    pub fn coach_inputs(&self) -> Result<Vec<CoachInput>> {
        let mut stmt = self.conn().prepare(
            "SELECT t.id, t.created_at, t.app_id, t.app_name,
                    m.words, m.um_uh, m.phrase_fillers, m.wpm, m.sentence_lengths,
                    m.repeats, m.polish_change_ratio, m.raw_known
             FROM coach_metric m
             JOIN transcription t ON t.id = m.transcription_id
             ORDER BY t.created_at ASC, t.id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            let lengths: String = r.get(8)?;
            Ok(CoachInput {
                id: r.get(0)?,
                created_at: r.get(1)?,
                app_id: r.get(2)?,
                app_name: r.get(3)?,
                measures: DictationMeasures {
                    words: r.get(4)?,
                    um_uh: r.get(5)?,
                    phrase_fillers: r.get(6)?,
                    wpm: r.get(7)?,
                    sentence_lengths: lengths
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect(),
                    repeats: r.get(9)?,
                    polish_change_ratio: r.get(10)?,
                    raw_known: r.get(11)?,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn coach_quotes(&self, ids: &[i64]) -> Result<HashMap<i64, CoachQuote>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "SELECT id, text, created_at, app_name FROM transcription WHERE id IN ({})",
            placeholders(ids.len())
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok(CoachQuote {
                id: r.get(0)?,
                text: r.get(1)?,
                created_at: r.get(2)?,
                app_name: r.get(3)?,
            })
        })?;
        let mut quotes = HashMap::new();
        for q in rows {
            let q = q?;
            quotes.insert(q.id, q);
        }
        Ok(quotes)
    }

    /// A dictation's text, for the judge. Read live; never kept by Coach.
    pub fn coach_text(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT text FROM transcription WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Every app dictated into (edit commands aside), most used first.
    pub fn coach_apps(&self) -> Result<Vec<CoachAppUsage>> {
        let mut stmt = self.conn().prepare(
            "SELECT app_id, app_name, COUNT(*), MIN(created_at) FROM transcription
             WHERE entry_kind != 'edit' GROUP BY app_id, app_name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut by_key: BTreeMap<String, CoachAppUsage> = BTreeMap::new();
        for row in rows {
            let (app_id, app_name, count, first) = row?;
            let key = mode::app_key(app_id.as_deref(), app_name.as_deref()).unwrap_or_default();
            let entry = by_key.entry(key.clone()).or_insert_with(|| CoachAppUsage {
                key,
                app_id: None,
                app_name: None,
                dictations: 0,
                first_at: None,
            });
            entry.dictations += count;
            entry.first_at = match (entry.first_at, first) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            // Like the Mac's MAX(): keep the greatest non-empty spelling seen.
            if app_id
                .as_deref()
                .is_some_and(|v| Some(v) > entry.app_id.as_deref())
            {
                entry.app_id = app_id;
            }
            if app_name
                .as_deref()
                .is_some_and(|v| Some(v) > entry.app_name.as_deref())
            {
                entry.app_name = app_name;
            }
        }
        let mut apps: Vec<CoachAppUsage> = by_key.into_values().collect();
        apps.sort_by(|a, b| {
            b.dictations
                .cmp(&a.dictations)
                .then_with(|| a.key.cmp(&b.key))
        });
        Ok(apps)
    }

    /// Forgets the counts (and the judge's answers) of apps just switched off.
    pub fn delete_coach_metrics_for_keys(&self, keys: &[String]) -> Result<usize> {
        if keys.is_empty() {
            return Ok(0);
        }
        let wanted: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let mut stmt = self
            .conn()
            .prepare("SELECT id, app_id, app_name FROM transcription WHERE app_id IS NOT NULL OR app_name IS NOT NULL")?;
        let ids: Vec<i64> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .filter_map(|row| row.ok())
            .filter(|(_, id, name)| {
                mode::app_key(id.as_deref(), name.as_deref())
                    .is_some_and(|k| wanted.contains(k.as_str()))
            })
            .map(|(id, _, _)| id)
            .collect();
        let tx = self.conn().unchecked_transaction()?;
        for id in &ids {
            tx.execute("DELETE FROM coach_metric WHERE transcription_id = ?1", [id])?;
            tx.execute(
                "DELETE FROM coach_judgement WHERE transcription_id = ?1",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// Answers for dictations that have since been deleted.
    pub fn prune_coach_judgements(&self) -> Result<()> {
        self.conn().execute(
            "DELETE FROM coach_judgement WHERE transcription_id NOT IN (SELECT id FROM transcription)",
            [],
        )?;
        Ok(())
    }

    // ---- Habits ------------------------------------------------------------

    /// Every habit, newest first.
    pub fn coach_habits(&self) -> Result<Vec<CoachHabitRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, measure, mode, started_at, ended_at FROM coach_habit ORDER BY started_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CoachHabitRecord {
                id: r.get(0)?,
                measure: r.get(1)?,
                mode: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn start_coach_habit(
        &self,
        target: CoachHabitTarget,
        mode: Option<CoachMode>,
        at: i64,
    ) -> Result<CoachHabitRecord> {
        self.conn().execute(
            "INSERT INTO coach_habit (measure, mode, started_at) VALUES (?1, ?2, ?3)",
            params![target.stored(), mode.map(CoachMode::id), at],
        )?;
        Ok(CoachHabitRecord {
            id: self.conn().last_insert_rowid(),
            measure: target.stored(),
            mode: mode.map(|m| m.id().to_string()),
            started_at: at,
            ended_at: None,
        })
    }

    pub fn end_coach_habit(&self, id: i64, at: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE coach_habit SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
            params![at, id],
        )?;
        Ok(())
    }

    // ---- Reps --------------------------------------------------------------

    pub fn save_coach_rep(&self, rep: &CoachRepRecord) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO coach_rep (skill_id, kind, spoken, created_at, score, skill_met, duration_ms, words, checks, persona)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                rep.skill_id,
                rep.kind,
                rep.spoken,
                rep.created_at,
                rep.score,
                rep.skill_met,
                rep.duration_ms,
                rep.words,
                rep.checks,
                rep.persona
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Every rep, oldest first.
    pub fn coach_reps(&self) -> Result<Vec<CoachRepRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, skill_id, kind, spoken, created_at, score, skill_met, duration_ms, words, checks, persona
             FROM coach_rep ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CoachRepRecord {
                id: Some(r.get(0)?),
                skill_id: r.get(1)?,
                kind: r.get(2)?,
                spoken: r.get(3)?,
                created_at: r.get(4)?,
                score: r.get(5)?,
                skill_met: r.get(6)?,
                duration_ms: r.get(7)?,
                words: r.get(8)?,
                checks: r.get(9)?,
                persona: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Judgements --------------------------------------------------------

    /// Every answer from one model as points (yes 1, partly 0.5, no 0), by
    /// question and dictation, for live dictations only.
    pub fn coach_judgement_scores(
        &self,
        model: &str,
    ) -> Result<HashMap<CoachQuestion, HashMap<i64, f64>>> {
        let mut stmt = self.conn().prepare(
            "SELECT j.transcription_id, j.question, j.answer FROM coach_judgement j
             JOIN transcription t ON t.id = j.transcription_id WHERE j.model = ?1",
        )?;
        let rows = stmt.query_map([model], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut result: HashMap<CoachQuestion, HashMap<i64, f64>> = HashMap::new();
        for row in rows {
            let (id, question, answer) = row?;
            let (Some(q), Some(a)) = (
                CoachQuestion::from_id(&question),
                JudgeAnswer::from_id(&answer),
            ) else {
                continue;
            };
            result.entry(q).or_default().insert(id, a.points());
        }
        Ok(result)
    }

    pub fn save_coach_judgement(
        &self,
        transcription_id: i64,
        question: CoachQuestion,
        model: &str,
        answer: JudgeAnswer,
        confidence: f64,
        at: i64,
    ) -> Result<()> {
        if self.coach_text(transcription_id)?.is_none() {
            return Ok(());
        }
        self.conn().execute(
            "INSERT OR REPLACE INTO coach_judgement (transcription_id, question, model, answer, confidence, judged_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![transcription_id, question.id(), model, answer.id(), confidence, at],
        )?;
        Ok(())
    }

    pub fn count_coach_judgements_since(&self, since: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM coach_judgement WHERE judged_at >= ?1",
            [since],
            |r| r.get(0),
        )?)
    }

    /// Settings → Coach "forget": every count, habit, rep and answer.
    pub fn reset_coach(&self) -> Result<()> {
        self.conn().execute_batch(
            "DELETE FROM coach_metric; DELETE FROM coach_judgement; DELETE FROM coach_habit; DELETE FROM coach_rep;",
        )?;
        self.kv_set(COACH_JUDGE_GRADES_KEY, None)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    fn dictation(store: &Store, text: &str, raw: Option<&str>, app: Option<&str>, at: i64) -> i64 {
        let mut r = TranscriptionRecord::new(text, app.map(str::to_string), 6_000, at);
        r.raw_text = raw.map(str::to_string);
        store.insert_transcription(&r).unwrap()
    }

    fn m(words: i64) -> DictationMeasures {
        DictationMeasures {
            words,
            um_uh: 1,
            phrase_fillers: 0,
            wpm: Some(150),
            sentence_lengths: vec![8, 9],
            repeats: 0,
            polish_change_ratio: Some(0.1),
            raw_known: true,
        }
    }

    #[test]
    fn metrics_round_trip_and_follow_the_dictation() {
        let s = Store::open_in_memory().unwrap();
        let a = dictation(
            &s,
            "hello there",
            Some("um hello there"),
            Some("slack"),
            100,
        );
        let b = dictation(&s, "second", None, None, 200);
        assert_eq!(s.coach_backlog_page(None, 10).unwrap().len(), 2);
        s.save_coach_metrics(&[(a, m(3)), (b, m(1)), (999, m(1))])
            .unwrap();
        assert!(s.coach_backlog_page(None, 10).unwrap().is_empty());
        let inputs = s.coach_inputs().unwrap();
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].id, a);
        assert_eq!(inputs[0].measures, m(3));
        assert_eq!(inputs[0].app_name.as_deref(), Some("slack"));
        s.delete_transcription(a).unwrap();
        assert_eq!(s.coach_inputs().unwrap().len(), 1);
        assert_eq!(s.coach_quotes(&[a, b]).unwrap().len(), 1);
    }

    #[test]
    fn backlog_pages_by_cursor_and_skips_edits() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..5 {
            dictation(&s, "x", None, None, 100 + i);
        }
        let mut edit = TranscriptionRecord::new("make it shorter", None, 1_000, 500);
        edit.entry_kind = "edit".into();
        s.insert_transcription(&edit).unwrap();
        let first = s.coach_backlog_page(None, 3).unwrap();
        assert_eq!(
            first.iter().map(|r| r.created_at).collect::<Vec<_>>(),
            vec![104, 103, 102]
        );
        let last = first.last().unwrap();
        let next = s
            .coach_backlog_page(Some((last.created_at, last.id)), 3)
            .unwrap();
        assert_eq!(next.len(), 2);
    }

    #[test]
    fn apps_group_by_key_and_switching_off_forgets() {
        let s = Store::open_in_memory().unwrap();
        let a = dictation(&s, "a", None, Some("Slack"), 10);
        dictation(&s, "b", None, Some("slack"), 20);
        dictation(&s, "c", None, Some("Firefox"), 5);
        dictation(&s, "d", None, None, 30);
        let apps = s.coach_apps().unwrap();
        assert_eq!(apps[0].key, "name:slack");
        assert_eq!(apps[0].dictations, 2);
        assert_eq!(apps[0].first_at, Some(10));
        assert!(apps.iter().any(|a| a.key.is_empty()));
        s.save_coach_metrics(&[(a, m(1))]).unwrap();
        s.save_coach_judgement(a, CoachQuestion::Tone, "j", JudgeAnswer::Yes, 0.9, 50)
            .unwrap();
        assert_eq!(
            s.delete_coach_metrics_for_keys(&["name:slack".into()])
                .unwrap(),
            2
        );
        assert!(s.coach_inputs().unwrap().is_empty());
        assert!(s.coach_judgement_scores("j").unwrap().is_empty());
    }

    #[test]
    fn habits_reps_and_judgements() {
        let s = Store::open_in_memory().unwrap();
        let h = s
            .start_coach_habit(
                CoachHabitTarget::Question(CoachQuestion::FinishLine),
                Some(CoachMode::Ai),
                100,
            )
            .unwrap();
        s.start_coach_habit(
            CoachHabitTarget::Measure(crate::coach::measure::CoachMeasure::Pace),
            None,
            200,
        )
        .unwrap();
        let habits = s.coach_habits().unwrap();
        assert_eq!(habits.len(), 2);
        assert_eq!(
            habits[1].target(),
            Some(CoachHabitTarget::Question(CoachQuestion::FinishLine))
        );
        assert_eq!(habits[1].coach_mode(), Some(CoachMode::Ai));
        s.end_coach_habit(h.id, 300).unwrap();
        s.end_coach_habit(h.id, 400).unwrap();
        assert_eq!(s.coach_habits().unwrap()[1].ended_at, Some(300));

        let rep = CoachRepRecord {
            id: None,
            skill_id: "task-first".into(),
            kind: "drill".into(),
            spoken: false,
            created_at: 10,
            score: Some(4.2),
            skill_met: Some(true),
            duration_ms: 0,
            words: 30,
            checks: "[]".into(),
            persona: "mentor".into(),
        };
        let id = s.save_coach_rep(&rep).unwrap();
        let reps = s.coach_reps().unwrap();
        assert_eq!(reps[0].id, Some(id));
        assert_eq!(reps[0].skill_met, Some(true));

        let t = dictation(&s, "Fix the paste.", None, None, 1_000);
        s.save_coach_judgement(
            t,
            CoachQuestion::FinishLine,
            "j",
            JudgeAnswer::Partly,
            0.6,
            2_000,
        )
        .unwrap();
        s.save_coach_judgement(
            999,
            CoachQuestion::FinishLine,
            "j",
            JudgeAnswer::Yes,
            0.6,
            2_000,
        )
        .unwrap();
        let scores = s.coach_judgement_scores("j").unwrap();
        assert_eq!(scores[&CoachQuestion::FinishLine][&t], 0.5);
        assert_eq!(s.count_coach_judgements_since(1_500).unwrap(), 1);
        assert!(s.coach_judgement_scores("other").unwrap().is_empty());
        s.reset_coach().unwrap();
        assert!(s.coach_reps().unwrap().is_empty());
    }
}
