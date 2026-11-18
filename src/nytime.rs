//! New York time: every session boundary, trading date and "local time to timestamp"
//! conversion goes through here, so DST is handled in one place.

use chrono::{DateTime, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use chrono_tz::Tz;

/// Session boundaries, minutes after New York midnight.
pub const PRE_OPEN: u32 = 4 * 60;
pub const REG_OPEN: u32 = 9 * 60 + 30;
pub const REG_CLOSE: u32 = 16 * 60;
pub const AFTER_CLOSE: u32 = 20 * 60;

/// `ts` (epoch seconds) in New York time.
pub fn at(ts: i64) -> Option<DateTime<Tz>> {
    New_York.timestamp_opt(ts, 0).single()
}

/// Any instant in New York time.
pub fn of<Z: TimeZone>(t: &DateTime<Z>) -> DateTime<Tz> {
    t.with_timezone(&New_York)
}

/// Minutes after midnight of a New York time.
pub fn minute_of_day(t: &DateTime<Tz>) -> u32 {
    t.hour() * 60 + t.minute()
}

/// New York trading date of an epoch-seconds instant.
pub fn date(ts: i64) -> NaiveDate {
    of(&DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default()).date_naive()
}

/// Epoch seconds of `h:m` New York time on `day` (the earlier one if the clock repeats).
pub fn ts(day: NaiveDate, h: u32, m: u32) -> Option<i64> {
    New_York.from_local_datetime(&day.and_hms_opt(h, m, 0)?).earliest().map(|t| t.timestamp())
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    /// Epoch seconds of a New York wall-clock time.
    pub fn ny(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        New_York.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp()
    }

    #[test]
    fn date_and_ts_round_trip_across_dst() {
        // 2026-03-08 is the US spring-forward day
        for (mo, d) in [(1, 12), (3, 8), (3, 9), (11, 1), (11, 2)] {
            let day = NaiveDate::from_ymd_opt(2026, mo, d).unwrap();
            let t = ts(day, 4, 0).unwrap();
            assert_eq!(t, ny(2026, mo, d, 4, 0));
            assert_eq!(date(t), day);
            assert_eq!(minute_of_day(&at(t).unwrap()), PRE_OPEN);
        }
    }
}
