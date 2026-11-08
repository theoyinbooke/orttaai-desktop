//! Analytics queries (dashboard stats, trends, streaks). Owned by the
//! analytics/home area.

use super::Store;
use crate::analytics::{parse_utc_offset, Sample};
use crate::error::Result;
use crate::milestones::{MilestoneDictation, MilestoneUnlock};
use rusqlite::params;
use serde::Serialize;

/// A day → count pair for the activity chart.
#[derive(Debug, Clone, Serialize)]
pub struct DailyCount {
    pub day: String,
    pub count: i64,
}

/// An app → count pair for "top apps".
#[derive(Debug, Clone, Serialize)]
pub struct AppCount {
    pub app: String,
    pub count: i64,
}

/// Aggregate dashboard statistics.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardStats {
    pub total: i64,
    pub total_words: i64,
    pub avg_words: f64,
    /// Average dictation speed in words per minute, across all recordings.
    pub avg_wpm: f64,
    pub last7_days: Vec<DailyCount>,
    pub top_apps: Vec<AppCount>,
}

impl Store {
    /// Aggregate stats for the dashboards. `days` limits the window (e.g. `Some(7)`
    /// for the last week); `None` or `Some(0)` means all-time.
    pub fn stats(&self, days: Option<i64>) -> Result<DashboardStats> {
        let now = super::now_unix();
        let window = days.filter(|d| *d > 0);
        let cutoff = window.map(|d| now - d * 86_400).unwrap_or(0);
        let conn = self.conn();

        let (total, total_words, total_duration_ms): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(word_count), 0), COALESCE(SUM(recording_ms), 0)
             FROM transcription WHERE created_at >= ?1",
            [cutoff],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let avg_words = if total > 0 {
            total_words as f64 / total as f64
        } else {
            0.0
        };
        let avg_wpm = if total_duration_ms > 0 {
            total_words as f64 / (total_duration_ms as f64 / 60_000.0)
        } else {
            0.0
        };

        let chart_cutoff = now - window.unwrap_or(30) * 86_400;
        let mut stmt = conn.prepare(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day, COUNT(*)
             FROM transcription WHERE created_at >= ?1
             GROUP BY day ORDER BY day",
        )?;
        let last7_days = stmt
            .query_map([chart_cutoff], |r| {
                Ok(DailyCount {
                    day: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = conn.prepare(
            "SELECT app_name AS a, COUNT(*) AS c
             FROM transcription
             WHERE created_at >= ?1 AND app_name IS NOT NULL AND app_name != ''
             GROUP BY a ORDER BY c DESC LIMIT 5",
        )?;
        let top_apps = stmt
            .query_map([cutoff], |r| {
                Ok(AppCount {
                    app: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(DashboardStats {
            total,
            total_words,
            avg_words,
            avg_wpm,
            last7_days,
            top_apps,
        })
    }

    /// The machine's local UTC offset (seconds) at `unix`, from SQLite's
    /// `localtime` (the C library's zone rules, DST included).
    pub fn local_offset_at(&self, unix: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT CAST(strftime('%s', ?1, 'unixepoch', 'localtime') AS INTEGER) - ?1",
            [unix],
            |r| r.get(0),
        )?)
    }

    /// Every dictation at/after `since` as dashboard samples (counts and
    /// timings only), each with its local UTC offset, oldest first.
    pub fn analytics_samples(&self, since: Option<i64>) -> Result<Vec<Sample>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, created_at,
                    CAST(strftime('%s', created_at, 'unixepoch', 'localtime') AS INTEGER) - created_at,
                    time_zone, word_count, recording_ms, processing_ms, transcription_ms,
                    text_processing_ms, injection_ms, app_name, model_id, entry_kind
             FROM transcription WHERE created_at >= ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([since.unwrap_or(i64::MIN)], |r| {
            let tz: Option<String> = r.get(3)?;
            Ok(Sample {
                id: r.get(0)?,
                created_at: r.get(1)?,
                local_offset: r.get(2)?,
                said_offset: tz.as_deref().and_then(parse_utc_offset),
                words: r.get(4)?,
                recording_ms: r.get(5)?,
                processing_ms: r.get(6)?,
                transcription_ms: r.get(7)?,
                text_processing_ms: r.get(8)?,
                injection_ms: r.get(9)?,
                app: r.get(10)?,
                model_id: r.get(11)?,
                entry_kind: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- Milestones ------------------------------------------------------

    /// What the milestone engine reads: dictations (not voice edits), oldest
    /// first, and when each active dictionary entry was added.
    pub fn milestone_inputs(&self) -> Result<MilestoneInputs> {
        let mut stmt = self.conn().prepare(
            "SELECT created_at,
                    CAST(strftime('%s', created_at, 'unixepoch', 'localtime') AS INTEGER) - created_at,
                    word_count, recording_ms, app_name
             FROM transcription WHERE entry_kind != 'edit'
             ORDER BY created_at ASC, id ASC",
        )?;
        let dictations = stmt
            .query_map([], |r| {
                Ok(MilestoneDictation {
                    created_at: r.get(0)?,
                    local_offset: r.get(1)?,
                    words: r.get(2)?,
                    recording_ms: r.get(3)?,
                    app: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self.conn().prepare(
            "SELECT created_at FROM dictionary_entry WHERE is_active = 1 ORDER BY created_at ASC",
        )?;
        let dictionary_entry_dates = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(MilestoneInputs {
            dictations,
            dictionary_entry_dates,
        })
    }

    pub fn milestone_unlocks(&self) -> Result<Vec<MilestoneUnlock>> {
        let mut stmt = self.conn().prepare(
            "SELECT badge_id, unlocked_at, celebrated_at FROM milestone_unlock
             ORDER BY unlocked_at, badge_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MilestoneUnlock {
                badge_id: r.get(0)?,
                unlocked_at: r.get(1)?,
                celebrated_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Store new unlocks; returns the ones actually written (a badge another
    /// pass stored first is skipped, so it is never announced twice).
    pub fn insert_milestone_unlocks(
        &self,
        unlocks: &[MilestoneUnlock],
    ) -> Result<Vec<MilestoneUnlock>> {
        let mut inserted = Vec::new();
        for u in unlocks {
            let n = self.conn().execute(
                "INSERT OR IGNORE INTO milestone_unlock (badge_id, unlocked_at, celebrated_at)
                 VALUES (?1, ?2, ?3)",
                params![u.badge_id, u.unlocked_at, u.celebrated_at],
            )?;
            if n > 0 {
                inserted.push(u.clone());
            }
        }
        Ok(inserted)
    }

    /// Mark badges as seen in a celebration (first time only).
    pub fn mark_milestones_celebrated(&self, ids: &[String], at: i64) -> Result<()> {
        for id in ids {
            self.conn().execute(
                "UPDATE milestone_unlock SET celebrated_at = ?2
                 WHERE badge_id = ?1 AND celebrated_at IS NULL",
                params![id, at],
            )?;
        }
        Ok(())
    }
}

/// Inputs to [`crate::milestones::compute`].
#[derive(Debug, Clone, Default)]
pub struct MilestoneInputs {
    pub dictations: Vec<MilestoneDictation>,
    pub dictionary_entry_dates: Vec<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranscriptionRecord;

    #[test]
    fn stats_still_works_for_the_cli() {
        let s = Store::open_in_memory().unwrap();
        let now = super::super::now_unix();
        s.insert_transcription(&TranscriptionRecord::new(
            "one two three four",
            Some("code".into()),
            60_000,
            now,
        ))
        .unwrap();
        let st = s.stats(Some(7)).unwrap();
        assert_eq!((st.total, st.total_words), (1, 4));
        assert_eq!(st.avg_wpm, 4.0);
        assert_eq!(st.top_apps[0].app, "code");
    }

    #[test]
    fn samples_carry_local_day_offsets_and_timings() {
        let s = Store::open_in_memory().unwrap();
        let at = 1_773_100_800;
        let mut r = TranscriptionRecord::new("a b c", Some("Slack".into()), 3_000, at);
        r.processing_ms = 640;
        r.transcription_ms = Some(400);
        r.injection_ms = Some(90);
        r.time_zone = Some("+02:00".into());
        s.insert_transcription(&r).unwrap();
        let samples = s.analytics_samples(None).unwrap();
        assert_eq!(samples.len(), 1);
        let x = &samples[0];
        assert_eq!(x.words, 3);
        assert_eq!(x.transcription_ms, Some(400));
        assert_eq!(x.text_processing_ms, None);
        assert_eq!(x.said_offset, Some(7_200));
        assert_eq!(x.hour(), 2);
        assert_eq!(x.local_offset, s.local_offset_at(at).unwrap());
        // The day matches SQLite's own local-date bucketing.
        let day: String = s
            .conn()
            .query_row("SELECT date(?1, 'unixepoch', 'localtime')", [at], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(crate::analytics::day_string(x.local_day()), day);
        assert!(s.analytics_samples(Some(at + 1)).unwrap().is_empty());
    }

    #[test]
    fn unlocks_insert_once_and_celebrate_once() {
        let s = Store::open_in_memory().unwrap();
        let u = MilestoneUnlock {
            badge_id: "first-words".into(),
            unlocked_at: 10,
            celebrated_at: None,
        };
        let first = s
            .insert_milestone_unlocks(std::slice::from_ref(&u))
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(s.insert_milestone_unlocks(&[u]).unwrap().is_empty());
        s.mark_milestones_celebrated(&["first-words".into()], 20)
            .unwrap();
        s.mark_milestones_celebrated(&["first-words".into()], 30)
            .unwrap();
        assert_eq!(s.milestone_unlocks().unwrap()[0].celebrated_at, Some(20));
        s.upsert_dictionary("orttaai", "Orttaai", false).unwrap();
        assert_eq!(
            s.milestone_inputs().unwrap().dictionary_entry_dates.len(),
            1
        );
    }
}
