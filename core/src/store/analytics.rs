//! Analytics queries (dashboard stats, trends, streaks). Owned by the
//! analytics/home area.

use super::Store;
use crate::error::Result;
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
}
