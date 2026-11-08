//! Calendar arithmetic in the person's own time zone (the Mac's
//! `Calendar.current`): days, weeks starting on Monday, and calendar months,
//! all as Unix seconds. Tests use a fixed offset so they never depend on
//! where they run.

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, Offset, TimeZone,
    Timelike, Utc,
};

#[derive(Debug, Clone, Copy, Default)]
pub enum CoachCalendar {
    /// The machine's local time zone, DST and all.
    #[default]
    Local,
    /// A fixed UTC offset in seconds (tests, or a zone the UI reports).
    Fixed(i32),
}

/// A half-open span of time `[start, end)`, in Unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Interval {
    pub start: i64,
    pub end: i64,
}

impl Interval {
    pub fn new(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    /// Swift's `DateInterval.contains`, which includes the end.
    pub fn contains(&self, t: i64) -> bool {
        t >= self.start && t <= self.end
    }
}

impl CoachCalendar {
    fn offset_at_local(&self, naive: NaiveDateTime) -> i64 {
        match self {
            Self::Fixed(secs) => *secs as i64,
            Self::Local => Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|d| d.offset().fix().local_minus_utc() as i64)
                .unwrap_or(0),
        }
    }

    fn naive(&self, t: i64) -> NaiveDateTime {
        let utc = DateTime::<Utc>::from_timestamp(t, 0).unwrap_or_default();
        match self {
            Self::Fixed(secs) => {
                let offset = FixedOffset::east_opt(*secs)
                    .unwrap_or_else(|| FixedOffset::east_opt(0).expect("utc"));
                utc.with_timezone(&offset).naive_local()
            }
            Self::Local => utc.with_timezone(&Local).naive_local(),
        }
    }

    /// The Unix second of a local wall-clock time; gaps (DST) move forward.
    fn unix(&self, naive: NaiveDateTime) -> i64 {
        match self {
            Self::Fixed(secs) => naive.and_utc().timestamp() - *secs as i64,
            Self::Local => match Local.from_local_datetime(&naive).earliest() {
                Some(d) => d.timestamp(),
                None => {
                    let later = naive + Duration::hours(1);
                    later.and_utc().timestamp() - self.offset_at_local(later)
                }
            },
        }
    }

    fn midnight(date: NaiveDate) -> NaiveDateTime {
        date.and_hms_opt(0, 0, 0).expect("midnight")
    }

    pub fn start_of_day(&self, t: i64) -> i64 {
        self.unix(Self::midnight(self.naive(t).date()))
    }

    /// The same wall-clock time `days` days later (negative goes back).
    pub fn add_days(&self, t: i64, days: i64) -> i64 {
        let n = self.naive(t);
        self.unix(n + Duration::days(days))
    }

    /// The same wall-clock time `months` months later, clamped to the month's end.
    pub fn add_months(&self, t: i64, months: i32) -> i64 {
        let n = self.naive(t);
        let total = n.year() * 12 + n.month0() as i32 + months;
        let (year, month0) = (total.div_euclid(12), total.rem_euclid(12) as u32);
        let mut day = n.day();
        let date = loop {
            if let Some(d) = NaiveDate::from_ymd_opt(year, month0 + 1, day) {
                break d;
            }
            day -= 1;
        };
        self.unix(date.and_time(n.time()))
    }

    /// The calendar month holding `t`.
    pub fn month(&self, t: i64) -> Interval {
        let n = self.naive(t);
        let first = NaiveDate::from_ymd_opt(n.year(), n.month(), 1).expect("first of month");
        let start = self.unix(Self::midnight(first));
        Interval::new(start, self.add_months(start, 1))
    }

    /// The week holding `t`, Monday first.
    pub fn week_start(&self, t: i64) -> i64 {
        let date = self.naive(t).date();
        let back = date.weekday().num_days_from_monday() as i64;
        self.unix(Self::midnight(date - Duration::days(back)))
    }

    /// Day of the year, 1-based.
    pub fn day_of_year(&self, t: i64) -> u32 {
        self.naive(t).ordinal()
    }

    pub fn hour(&self, t: i64) -> u32 {
        self.naive(t).hour()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_weeks_and_months_in_a_fixed_zone() {
        let cal = CoachCalendar::Fixed(3_600);
        // 2027-01-15 12:00 UTC is a Friday, 13:00 local.
        let t = 1_800_000_000 - (1_800_000_000 % 86_400) + 12 * 3_600;
        let day = cal.start_of_day(t);
        assert_eq!((day + 3_600) % 86_400, 0);
        assert_eq!(cal.add_days(day, 1) - day, 86_400);
        let week = cal.week_start(t);
        assert!(week <= day && day - week < 7 * 86_400);
        let month = cal.month(t);
        assert!(month.start <= t && t < month.end);
        assert_eq!(cal.month(month.end).start, month.end);
        assert_eq!(
            cal.add_months(month.start, -1),
            cal.month(month.start - 1).start
        );
        assert_eq!(cal.hour(t), cal.hour(cal.add_days(t, 3)));
    }

    #[test]
    fn month_ends_clamp() {
        let cal = CoachCalendar::Fixed(0);
        // 2027-01-31 → 2027-02-28.
        let jan31 = NaiveDate::from_ymd_opt(2027, 1, 31)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let feb28 = NaiveDate::from_ymd_opt(2027, 2, 28)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(cal.add_months(jan31, 1), feb28);
    }
}
