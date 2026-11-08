//! Learning suggestions and edit checks (Mac `DatabaseManager` learning
//! section), plus the history analysis and edit-learning runs that feed them.
//! Owned by the text-pipeline area.

use super::{normalize_key, now_unix, Store};
use crate::error::Result;
use crate::llm::LlmClient;
use crate::memory::{DictionaryRule, MemoryService, SnippetRule};
use crate::settings::Settings;
use crate::text::learning::{
    self, edit_evidence, EditCheckOutcome, SuggestionDraft, SuggestionKind, EDIT_CONFIDENCE,
    EDIT_EVIDENCE_PREFIX, HEURISTIC_ANALYZER,
};
use crate::text::{TextOptions, TextProcessor};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// Suggestion status values stored in `learning_suggestion.status`.
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_ACCEPTED: &str = "accepted";
pub const STATUS_DISMISSED: &str = "dismissed";

/// Edit checks kept (newest first).
pub const MAX_EDIT_CHECKS: i64 = 30;

/// A stored suggestion (Mac `LearningSuggestion`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningSuggestion {
    pub id: i64,
    /// `dictionary | snippet`.
    pub kind: String,
    pub candidate_source: String,
    pub candidate_target: String,
    pub normalized_source: String,
    pub confidence: f64,
    /// `pending | accepted | dismissed`.
    pub status: String,
    pub evidence: Option<String>,
    /// `edit` when learned from the user's own fix, `history` otherwise.
    pub origin: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl LearningSuggestion {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        let evidence: Option<String> = r.get(7)?;
        let status: String = r.get(6)?;
        Ok(Self {
            id: r.get(0)?,
            kind: r.get(1)?,
            candidate_source: r.get(2)?,
            candidate_target: r.get(3)?,
            normalized_source: r.get(4)?,
            confidence: r.get(5)?,
            // Rows written as the Mac's "rejected" read as dismissed.
            status: if status == "rejected" {
                STATUS_DISMISSED.to_string()
            } else {
                status
            },
            origin: if evidence
                .as_deref()
                .is_some_and(|e| e.starts_with(EDIT_EVIDENCE_PREFIX))
            {
                "edit".into()
            } else {
                "history".into()
            },
            evidence,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
        })
    }
}

const SUGGESTION_COLUMNS: &str =
    "id, kind, candidate_source, candidate_target, normalized_source, \
     confidence, status, evidence, created_at, updated_at";

/// One logged check of a dictation's field (never any text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditCheck {
    pub id: i64,
    pub checked_at: i64,
    pub app_name: Option<String>,
    /// Snake-case [`EditCheckOutcome`].
    pub outcome: String,
    pub learned_count: i64,
    /// Human-readable outcome ("Learned a word from your fix").
    pub description: String,
}

/// What one history analysis did (Mac `MemoryLearningRunResult`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningRunResult {
    pub analyzer_name: String,
    pub inserted_count: usize,
    pub history_sample_count: usize,
    pub used_fallback: bool,
    /// Why the AI analyzer was not used, when it failed.
    pub ai_error: Option<String>,
}

impl LearningRunResult {
    /// The status line Memory › Suggestions shows after a run.
    pub fn message(&self) -> String {
        if self.history_sample_count == 0 {
            return "No history yet to analyze.".into();
        }
        if self.inserted_count == 0 {
            return format!(
                "No new suggestions found from {} transcripts.",
                self.history_sample_count
            );
        }
        let fallback = if self.used_fallback {
            " (fallback used)"
        } else {
            ""
        };
        format!(
            "Added {} suggestion{} using {}{fallback}.",
            self.inserted_count,
            if self.inserted_count == 1 { "" } else { "s" },
            self.analyzer_name
        )
    }
}

/// The field around one dictation, for [`Store::learn_from_edit`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditObservation {
    /// The text the dictation inserted.
    pub inserted: String,
    /// The field right after insertion.
    pub before: String,
    /// Caret (end of the insertion) in UTF-16 units, when known.
    pub caret_utf16: Option<usize>,
    /// The same field later.
    pub after: String,
    pub app_name: Option<String>,
}

/// Memory as exported to (and imported from) a JSON file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryExport {
    #[serde(default = "export_format")]
    pub format: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub exported_at: i64,
    #[serde(default)]
    pub dictionary: Vec<ExportedDictionaryEntry>,
    #[serde(default)]
    pub snippets: Vec<ExportedSnippet>,
}

fn export_format() -> String {
    "orttaai-memory".into()
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedDictionaryEntry {
    #[serde(alias = "heard", alias = "from")]
    pub source: String,
    #[serde(alias = "written", alias = "to", alias = "replacement")]
    pub target: String,
    #[serde(default, alias = "isCaseSensitive", alias = "is_case_sensitive")]
    pub case_sensitive: bool,
    #[serde(default = "yes", alias = "isActive", alias = "is_active")]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedSnippet {
    pub trigger: String,
    #[serde(alias = "text", alias = "replacement")]
    pub expansion: String,
    #[serde(default = "yes", alias = "isActive", alias = "is_active")]
    pub active: bool,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    pub dictionary_added: usize,
    pub dictionary_updated: usize,
    pub snippets_added: usize,
    pub snippets_updated: usize,
    /// Rows with an empty field.
    pub skipped: usize,
}

/// The local model used for AI suggestions.
pub struct AiAnalyzer<'a> {
    pub llm: &'a dyn LlmClient,
    pub model: String,
}

impl Store {
    // ---- Memory for the text pipeline ------------------------------------------

    /// Active dictionary and snippet rules with their ids and usage counts
    /// (usage orders the vocabulary-bias prompt).
    pub fn load_memory_rules(&self) -> Result<MemoryService> {
        let mut memory = MemoryService::new();
        for d in self.list_dictionary()?.into_iter().filter(|d| d.is_active) {
            memory.add_dictionary_rule(DictionaryRule {
                id: Some(d.id),
                source: d.source,
                target: d.target,
                case_sensitive: d.is_case_sensitive,
                usage_count: d.usage_count,
            });
        }
        for s in self.list_snippets()?.into_iter().filter(|s| s.is_active) {
            memory.add_snippet_rule(SnippetRule {
                id: Some(s.id),
                trigger: s.trigger,
                expansion: s.expansion,
                usage_count: s.usage_count,
            });
        }
        Ok(memory)
    }

    /// The transcript pipeline for `settings` and the stored memory.
    pub fn text_processor(&self, settings: &Settings) -> Result<TextProcessor> {
        Ok(TextProcessor::new(
            TextOptions::from_settings(settings),
            self.load_memory_rules()?,
        ))
    }

    // ---- Suggestions -----------------------------------------------------------

    /// Store drafts as pending suggestions. A pair the user already dismissed,
    /// or already has in the dictionary, is not suggested again; a pending
    /// duplicate keeps the higher confidence and the newest evidence. Returns
    /// how many rows were added or updated.
    pub fn save_learning_suggestions(&self, drafts: &[SuggestionDraft]) -> Result<usize> {
        let tx = self.conn().unchecked_transaction()?;
        let now = now_unix();
        let mut changed = 0usize;
        for draft in drafts {
            let source = draft.source.trim();
            let target = draft.target.trim();
            if source.is_empty() || target.is_empty() {
                continue;
            }
            let normalized = normalize_key(source);
            let kind = draft.kind.as_str();
            if draft.kind == SuggestionKind::Dictionary {
                let in_dictionary: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM dictionary_entry WHERE normalized_source = ?1 AND target = ?2",
                    params![normalized, target],
                    |r| r.get(0),
                )?;
                if in_dictionary > 0 {
                    continue;
                }
            }
            let existing: Option<(i64, String, f64)> = tx
                .query_row(
                    "SELECT id, status, confidence FROM learning_suggestion
                     WHERE kind = ?1 AND normalized_source = ?2 AND candidate_target = ?3",
                    params![kind, normalized, target],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let confidence = draft.confidence.clamp(0.0, 1.0);
            let evidence = draft.evidence.as_deref().filter(|e| !e.is_empty());
            match existing {
                Some((_, status, _)) if status == STATUS_DISMISSED || status == "rejected" => {}
                Some((id, status, old)) if status == STATUS_PENDING => {
                    tx.execute(
                        "UPDATE learning_suggestion SET confidence = ?2,
                            evidence = COALESCE(?3, evidence), updated_at = ?4 WHERE id = ?1",
                        params![id, old.max(confidence), evidence, now],
                    )?;
                    changed += 1;
                }
                // Accepted before, but the entry it made is gone: suggest again.
                Some((id, _, _)) => {
                    let still_there: i64 = if draft.kind == SuggestionKind::Snippet {
                        tx.query_row(
                            "SELECT COUNT(*) FROM snippet_entry WHERE normalized_trigger = ?1",
                            [&normalized],
                            |r| r.get(0),
                        )?
                    } else {
                        0
                    };
                    if still_there == 0 {
                        tx.execute(
                            "UPDATE learning_suggestion SET status = ?2, confidence = ?3,
                                evidence = COALESCE(?4, evidence), updated_at = ?5 WHERE id = ?1",
                            params![id, STATUS_PENDING, confidence, evidence, now],
                        )?;
                        changed += 1;
                    }
                }
                None => {
                    tx.execute(
                        "INSERT INTO learning_suggestion (kind, candidate_source, candidate_target,
                            normalized_source, confidence, status, evidence, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                        params![
                            kind,
                            source,
                            target,
                            normalized,
                            confidence,
                            STATUS_PENDING,
                            evidence,
                            now
                        ],
                    )?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Suggestions, newest first, optionally filtered by status.
    pub fn list_learning_suggestions(
        &self,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningSuggestion>> {
        let mut out = Vec::new();
        match status {
            Some(status) => {
                let statuses: &[&str] = if status == STATUS_DISMISSED {
                    &[STATUS_DISMISSED, "rejected"]
                } else {
                    &[status, status]
                };
                let mut stmt = self.conn().prepare(&format!(
                    "SELECT {SUGGESTION_COLUMNS} FROM learning_suggestion
                     WHERE status IN (?1, ?2) ORDER BY created_at DESC, id DESC LIMIT ?3"
                ))?;
                let rows = stmt.query_map(
                    params![statuses[0], statuses[1], limit],
                    LearningSuggestion::from_row,
                )?;
                for r in rows {
                    out.push(r?);
                }
            }
            None => {
                let mut stmt = self.conn().prepare(&format!(
                    "SELECT {SUGGESTION_COLUMNS} FROM learning_suggestion
                     ORDER BY created_at DESC, id DESC LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit], LearningSuggestion::from_row)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    pub fn get_learning_suggestion(&self, id: i64) -> Result<Option<LearningSuggestion>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {SUGGESTION_COLUMNS} FROM learning_suggestion WHERE id = ?1"),
                [id],
                LearningSuggestion::from_row,
            )
            .optional()?)
    }

    pub fn set_learning_suggestion_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE learning_suggestion SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status, now_unix()],
        )?;
        Ok(())
    }

    /// Accept a suggestion: create (or update) its dictionary entry or
    /// snippet, active, and mark it accepted. Returns the new entry's id, or
    /// `None` when the suggestion no longer exists.
    pub fn accept_learning_suggestion(&self, id: i64) -> Result<Option<i64>> {
        let Some(s) = self.get_learning_suggestion(id)? else {
            return Ok(None);
        };
        let entry = match SuggestionKind::parse(&s.kind) {
            SuggestionKind::Dictionary => {
                self.upsert_dictionary(&s.candidate_source, &s.candidate_target, false)?
            }
            SuggestionKind::Snippet => {
                self.upsert_snippet(&s.candidate_source, &s.candidate_target)?
            }
        };
        self.set_learning_suggestion_status(id, STATUS_ACCEPTED)?;
        Ok(Some(entry))
    }

    pub fn dismiss_learning_suggestion(&self, id: i64) -> Result<()> {
        self.set_learning_suggestion_status(id, STATUS_DISMISSED)
    }

    pub fn pending_learning_suggestion_count(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM learning_suggestion WHERE status = ?1",
            [STATUS_PENDING],
            |r| r.get(0),
        )?)
    }

    /// Delete suggestions with `status`, or all of them.
    pub fn clear_learning_suggestions(&self, status: Option<&str>) -> Result<()> {
        match status {
            Some(s) => self
                .conn()
                .execute("DELETE FROM learning_suggestion WHERE status = ?1", [s])?,
            None => self.conn().execute("DELETE FROM learning_suggestion", [])?,
        };
        Ok(())
    }

    // ---- Edit checks --------------------------------------------------------------

    /// Log how one check ended; only the newest [`MAX_EDIT_CHECKS`] are kept.
    pub fn insert_edit_check(
        &self,
        outcome: EditCheckOutcome,
        app_name: Option<&str>,
        learned_count: i64,
        at: i64,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO edit_check (checked_at, app_name, outcome, learned_count) VALUES (?1, ?2, ?3, ?4)",
            params![at, app_name, outcome.as_str(), learned_count],
        )?;
        let id = self.conn().last_insert_rowid();
        self.conn().execute(
            "DELETE FROM edit_check WHERE id NOT IN
                (SELECT id FROM edit_check ORDER BY checked_at DESC, id DESC LIMIT ?1)",
            [MAX_EDIT_CHECKS],
        )?;
        Ok(id)
    }

    pub fn list_edit_checks(&self, limit: i64) -> Result<Vec<EditCheck>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, checked_at, app_name, outcome, learned_count FROM edit_check
             ORDER BY checked_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            let outcome: String = r.get(3)?;
            let learned: i64 = r.get(4)?;
            Ok(EditCheck {
                id: r.get(0)?,
                checked_at: r.get(1)?,
                app_name: r.get(2)?,
                description: EditCheckOutcome::parse(&outcome)
                    .map_or_else(|| "Checked".to_string(), |o| o.description(learned)),
                outcome,
                learned_count: learned,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Import / export ---------------------------------------------------------------

    /// The whole dictionary and every snippet, as pretty JSON.
    pub fn export_memory_json(&self) -> Result<String> {
        let export = MemoryExport {
            format: export_format(),
            version: 1,
            exported_at: now_unix(),
            dictionary: self
                .list_dictionary()?
                .into_iter()
                .map(|d| ExportedDictionaryEntry {
                    source: d.source,
                    target: d.target,
                    case_sensitive: d.is_case_sensitive,
                    active: d.is_active,
                })
                .collect(),
            snippets: self
                .list_snippets()?
                .into_iter()
                .map(|s| ExportedSnippet {
                    trigger: s.trigger,
                    expansion: s.expansion,
                    active: s.is_active,
                })
                .collect(),
        };
        Ok(serde_json::to_string_pretty(&export)?)
    }

    /// Merge an export (or a bare array of dictionary entries) into memory.
    /// Entries match existing ones by their normalized source/trigger and
    /// replace them; nothing is deleted.
    pub fn import_memory_json(&self, json: &str) -> Result<ImportSummary> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let export: MemoryExport = if value.is_array() {
            MemoryExport {
                dictionary: serde_json::from_value(value)?,
                ..Default::default()
            }
        } else {
            serde_json::from_value(value)?
        };
        let mut summary = ImportSummary::default();
        let known_sources: std::collections::HashSet<String> = self
            .list_dictionary()?
            .into_iter()
            .map(|d| normalize_key(&d.source))
            .collect();
        let known_triggers: std::collections::HashSet<String> = self
            .list_snippets()?
            .into_iter()
            .map(|s| normalize_key(&s.trigger))
            .collect();
        let tx = self.conn().unchecked_transaction()?;
        for d in &export.dictionary {
            if d.source.trim().is_empty() || d.target.trim().is_empty() {
                summary.skipped += 1;
                continue;
            }
            let id = self.upsert_dictionary(&d.source, &d.target, d.case_sensitive)?;
            if !d.active {
                self.update_dictionary(id, &d.source, &d.target, d.case_sensitive, false)?;
            }
            if known_sources.contains(&normalize_key(&d.source)) {
                summary.dictionary_updated += 1;
            } else {
                summary.dictionary_added += 1;
            }
        }
        for sn in &export.snippets {
            if sn.trigger.trim().is_empty() || sn.expansion.trim().is_empty() {
                summary.skipped += 1;
                continue;
            }
            let id = self.upsert_snippet(&sn.trigger, &sn.expansion)?;
            if !sn.active {
                self.update_snippet(id, &sn.trigger, &sn.expansion, false)?;
            }
            if known_triggers.contains(&normalize_key(&sn.trigger)) {
                summary.snippets_updated += 1;
            } else {
                summary.snippets_added += 1;
            }
        }
        tx.commit()?;
        Ok(summary)
    }

    // ---- Runs ----------------------------------------------------------------------

    /// Learn from the user's own fix of a dictation (Mac `EditLearningWatcher`
    /// finish step): corrections inside the dictated text become pending
    /// dictionary suggestions and the outcome is logged, with no text.
    /// `english` is whether the dictation language is English.
    pub fn learn_from_edit(
        &self,
        observation: &EditObservation,
        english: bool,
    ) -> Result<(EditCheckOutcome, usize)> {
        let app = observation.app_name.as_deref();
        let (outcome, saved) = if !english {
            (EditCheckOutcome::NotEnglish, 0)
        } else if !learning::field_contains(&observation.before, &observation.inserted) {
            (EditCheckOutcome::TextNotFound, 0)
        } else {
            let found = learning::corrections(
                &observation.inserted,
                &observation.before,
                observation.caret_utf16,
                &observation.after,
            );
            if found.is_empty() {
                let edited = learning::comparable(&observation.after)
                    != learning::comparable(&observation.before);
                (
                    if edited {
                        EditCheckOutcome::EditedNoName
                    } else {
                        EditCheckOutcome::NoChange
                    },
                    0,
                )
            } else {
                let evidence = edit_evidence(app);
                let drafts: Vec<SuggestionDraft> = found
                    .into_iter()
                    .map(|c| SuggestionDraft {
                        kind: SuggestionKind::Dictionary,
                        source: c.heard,
                        target: c.corrected,
                        confidence: EDIT_CONFIDENCE,
                        evidence: Some(evidence.clone()),
                    })
                    .collect();
                let saved = self.save_learning_suggestions(&drafts)?;
                if saved > 0 {
                    (EditCheckOutcome::Learned, saved)
                } else {
                    (EditCheckOutcome::AlreadyKnown, 0)
                }
            }
        };
        self.insert_edit_check(outcome, app, saved as i64, now_unix())?;
        Ok((outcome, saved))
    }

    /// Analyze recent history for suggestions (Mac `analyzeRecentHistory`):
    /// the local model when `ai` is given, falling back to the heuristic
    /// analyzer when it fails or finds nothing.
    pub fn analyze_history(
        &self,
        ai: Option<AiAnalyzer<'_>>,
        limit: i64,
    ) -> Result<LearningRunResult> {
        let history: Vec<String> = self
            .recent(limit)?
            .into_iter()
            .filter(|r| r.entry_kind != "edit")
            .map(|r| r.text)
            .collect();
        if history.is_empty() {
            return Ok(LearningRunResult {
                analyzer_name: HEURISTIC_ANALYZER.into(),
                inserted_count: 0,
                history_sample_count: 0,
                used_fallback: false,
                ai_error: None,
            });
        }
        let sources: Vec<String> = self
            .list_dictionary()?
            .into_iter()
            .map(|d| d.source)
            .collect();
        let triggers: Vec<String> = self
            .list_snippets()?
            .into_iter()
            .map(|s| s.trigger)
            .collect();

        let mut ai_error = None;
        let mut used_fallback = false;
        let mut analyzer_name = HEURISTIC_ANALYZER.to_string();
        let mut drafts = Vec::new();
        if let Some(ai) = ai {
            match learning::llm_drafts(ai.llm, &ai.model, &history, &sources, &triggers) {
                Ok(d) => drafts = d,
                Err(e) => ai_error = Some(e.to_string()),
            }
            if drafts.is_empty() {
                used_fallback = true;
            } else {
                analyzer_name = format!("Local AI ({})", ai.model);
            }
        }
        if drafts.is_empty() {
            drafts = learning::heuristic_drafts(&history, &sources, &triggers);
        }
        let inserted_count = self.save_learning_suggestions(&drafts)?;
        Ok(LearningRunResult {
            analyzer_name,
            inserted_count,
            history_sample_count: history.len(),
            used_fallback,
            ai_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    fn draft(kind: SuggestionKind, s: &str, t: &str, c: f64) -> SuggestionDraft {
        SuggestionDraft {
            kind,
            source: s.into(),
            target: t.into(),
            confidence: c,
            evidence: Some("e".into()),
        }
    }

    #[test]
    fn stored_memory_builds_the_text_pipeline() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_dictionary("whispr", "Wispr", false).unwrap();
        let off = s.upsert_dictionary("old term", "Retired", false).unwrap();
        s.update_dictionary(off, "old term", "Retired", false, false)
            .unwrap();
        s.upsert_snippet("my sig", "Best, Theo").unwrap();
        s.record_memory_usage(&["whispr".into()], &[]).unwrap();
        let memory = s.load_memory_rules().unwrap();
        assert_eq!(memory.dictionary().len(), 1);
        assert_eq!(memory.dictionary()[0].usage_count, 1);
        assert!(memory.dictionary()[0].id.is_some());
        let p = s.text_processor(&Settings::default()).unwrap();
        assert_eq!(p.process("whispr flow").text, "Wispr flow");
        assert_eq!(p.process("insert my sig").text, "Best, Theo");
        assert_eq!(p.process("old term").text, "old term");
    }

    #[test]
    fn memory_round_trips_through_json() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_dictionary("whispr", "Wispr", true).unwrap();
        let off = s.upsert_snippet("my sig", "Best,\nTheo").unwrap();
        s.update_snippet(off, "my sig", "Best,\nTheo", false)
            .unwrap();
        let json = s.export_memory_json().unwrap();
        let other = Store::open_in_memory().unwrap();
        other.upsert_dictionary("WHISPR", "Old", false).unwrap();
        let summary = other.import_memory_json(&json).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                dictionary_added: 0,
                dictionary_updated: 1,
                snippets_added: 1,
                snippets_updated: 0,
                skipped: 0
            }
        );
        let d = &other.list_dictionary().unwrap()[0];
        assert_eq!((d.target.as_str(), d.is_case_sensitive), ("Wispr", true));
        let sn = &other.list_snippets().unwrap()[0];
        assert_eq!(sn.expansion, "Best,\nTheo");
        assert!(!sn.is_active);

        let bare = r#"[{"source":"otta ai","target":"Orttaai"},{"heard":"","written":"x"}]"#;
        let summary = other.import_memory_json(bare).unwrap();
        assert_eq!((summary.dictionary_added, summary.skipped), (1, 1));
        assert!(other.import_memory_json("not json").is_err());
    }

    #[test]
    fn suggestions_dedupe_and_respect_decisions() {
        let s = Store::open_in_memory().unwrap();
        let d = draft(SuggestionKind::Dictionary, "otta ai", "Orttaai", 0.5);
        assert_eq!(
            s.save_learning_suggestions(std::slice::from_ref(&d))
                .unwrap(),
            1
        );
        let higher = SuggestionDraft {
            confidence: 0.8,
            ..d.clone()
        };
        assert_eq!(s.save_learning_suggestions(&[higher]).unwrap(), 1);
        let pending = s
            .list_learning_suggestions(Some(STATUS_PENDING), 150)
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert!((pending[0].confidence - 0.8).abs() < 1e-9);
        assert_eq!(pending[0].origin, "history");

        s.dismiss_learning_suggestion(pending[0].id).unwrap();
        assert_eq!(
            s.save_learning_suggestions(&[d]).unwrap(),
            0,
            "dismissed stays dismissed"
        );
        assert_eq!(s.pending_learning_suggestion_count().unwrap(), 0);
        assert_eq!(
            s.list_learning_suggestions(Some(STATUS_DISMISSED), 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn accepting_creates_the_entry_and_blocks_resuggesting() {
        let s = Store::open_in_memory().unwrap();
        s.save_learning_suggestions(&[
            draft(SuggestionKind::Dictionary, "otta ai", "Orttaai", 0.8),
            draft(
                SuggestionKind::Snippet,
                "sign off",
                "Thanks so much, talk soon",
                0.7,
            ),
        ])
        .unwrap();
        for sug in s
            .list_learning_suggestions(Some(STATUS_PENDING), 10)
            .unwrap()
        {
            assert!(s.accept_learning_suggestion(sug.id).unwrap().is_some());
        }
        let dict = s.list_dictionary().unwrap();
        assert_eq!(dict.len(), 1);
        assert_eq!(dict[0].target, "Orttaai");
        assert!(dict[0].is_active);
        assert_eq!(s.list_snippets().unwrap()[0].trigger, "sign off");
        assert_eq!(
            s.save_learning_suggestions(&[draft(
                SuggestionKind::Dictionary,
                "Otta  AI",
                "Orttaai",
                0.9
            )])
            .unwrap(),
            0,
            "already in the dictionary"
        );
        assert!(s.accept_learning_suggestion(999).unwrap().is_none());
    }

    #[test]
    fn edit_checks_keep_the_newest_thirty() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..35 {
            s.insert_edit_check(EditCheckOutcome::NoChange, Some("Claude"), 0, 1_000 + i)
                .unwrap();
        }
        s.insert_edit_check(EditCheckOutcome::Learned, Some("Notes"), 2, 1_100)
            .unwrap();
        let checks = s.list_edit_checks(100).unwrap();
        assert_eq!(checks.len() as i64, MAX_EDIT_CHECKS);
        assert_eq!(checks[0].outcome, "learned");
        assert_eq!(checks[0].learned_count, 2);
        assert_eq!(checks[0].app_name.as_deref(), Some("Notes"));
        assert_eq!(checks[0].description, "Learned 2 words from your fixes");
    }

    #[test]
    fn learning_from_an_edit_saves_the_pair_and_logs_the_outcome() {
        let s = Store::open_in_memory().unwrap();
        let obs = EditObservation {
            inserted: "Ask Adayemi about the deck.".into(),
            before: "Ask Adayemi about the deck.".into(),
            caret_utf16: None,
            after: "Ask Adeyemi about the deck.".into(),
            app_name: Some("Slack".into()),
        };
        assert_eq!(
            s.learn_from_edit(&obs, true).unwrap(),
            (EditCheckOutcome::Learned, 1)
        );
        let sug = &s
            .list_learning_suggestions(Some(STATUS_PENDING), 10)
            .unwrap()[0];
        assert_eq!(
            (sug.candidate_source.as_str(), sug.candidate_target.as_str()),
            ("Adayemi", "Adeyemi")
        );
        assert_eq!(sug.origin, "edit");
        assert_eq!(
            sug.evidence.as_deref(),
            Some("Corrected after dictating into Slack")
        );
        assert!((sug.confidence - 0.9).abs() < 1e-9);
        // The same fix again is already known once dismissed.
        s.dismiss_learning_suggestion(sug.id).unwrap();
        assert_eq!(
            s.learn_from_edit(&obs, true).unwrap().0,
            EditCheckOutcome::AlreadyKnown
        );

        let unchanged = EditObservation {
            after: obs.before.clone(),
            ..obs.clone()
        };
        assert_eq!(
            s.learn_from_edit(&unchanged, true).unwrap().0,
            EditCheckOutcome::NoChange
        );
        let reworded = EditObservation {
            after: "Ask Adayemi about the slides.".into(),
            ..obs.clone()
        };
        assert_eq!(
            s.learn_from_edit(&reworded, true).unwrap().0,
            EditCheckOutcome::EditedNoName
        );
        let missing = EditObservation {
            before: "Something else".into(),
            ..obs.clone()
        };
        assert_eq!(
            s.learn_from_edit(&missing, true).unwrap().0,
            EditCheckOutcome::TextNotFound
        );
        assert_eq!(
            s.learn_from_edit(&obs, false).unwrap().0,
            EditCheckOutcome::NotEnglish
        );
        let outcomes: Vec<String> = s
            .list_edit_checks(10)
            .unwrap()
            .into_iter()
            .map(|c| c.outcome)
            .collect();
        assert_eq!(
            outcomes,
            vec![
                "not_english",
                "text_not_found",
                "edited_no_name",
                "no_change",
                "already_known",
                "learned"
            ]
        );
    }

    fn seed(s: &Store, texts: &[&str]) {
        for (i, t) in texts.iter().enumerate() {
            s.insert_transcription(&TranscriptionRecord::new(*t, None, 1000, 100 + i as i64))
                .unwrap();
        }
    }

    #[test]
    fn history_analysis_uses_the_heuristic_without_ai() {
        let s = Store::open_in_memory().unwrap();
        let r = s.analyze_history(None, 120).unwrap();
        assert_eq!(r.history_sample_count, 0);
        assert_eq!(r.message(), "No history yet to analyze.");
        let line = "Thanks so much for your help, talk soon";
        seed(
            &s,
            &[
                line,
                line,
                "Folasade joined.",
                "Ask Folasade now.",
                "Folashade said hi.",
            ],
        );
        let r = s.analyze_history(None, 120).unwrap();
        assert_eq!(r.analyzer_name, HEURISTIC_ANALYZER);
        assert_eq!(r.inserted_count, 2);
        assert_eq!(r.message(), "Added 2 suggestions using Heuristic Analyzer.");
        let again = s.analyze_history(None, 120).unwrap();
        assert_eq!(
            again.inserted_count, 2,
            "pending duplicates are refreshed, not added"
        );
        assert_eq!(s.pending_learning_suggestion_count().unwrap(), 2);
    }

    #[test]
    fn history_analysis_prefers_ai_and_falls_back() {
        let s = Store::open_in_memory().unwrap();
        seed(&s, &["I was testing otta ai today and otta ai was fast."]);
        let llm = crate::llm::MockLlm::fixed(
            r#"{"dictionary":[{"source":"otta ai","target":"Orttaai","confidence":0.8}],"snippets":[]}"#,
        );
        let r = s
            .analyze_history(
                Some(AiAnalyzer {
                    llm: &llm,
                    model: "gemma".into(),
                }),
                120,
            )
            .unwrap();
        assert_eq!(r.analyzer_name, "Local AI (gemma)");
        assert_eq!(r.inserted_count, 1);
        assert!(!r.used_fallback);

        let offline = crate::llm::MockLlm::offline();
        let r = s
            .analyze_history(
                Some(AiAnalyzer {
                    llm: &offline,
                    model: "gemma".into(),
                }),
                120,
            )
            .unwrap();
        assert!(r.used_fallback);
        assert!(r.ai_error.is_some());
        assert_eq!(r.analyzer_name, HEURISTIC_ANALYZER);
    }
}
