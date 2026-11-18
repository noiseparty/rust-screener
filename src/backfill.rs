//! Minute-bar backfill, so a ticker that just appeared (a new spiker) shows the minutes
//! before it reached the list, and a ticker with saved candles gets the minutes the app was
//! closed for. Bars come from `history` one at a time, walking back.

use crate::engine::Candle;
use crate::history::{self, Tf};
use crate::nytime::{self, date as ny_date};

/// Minutes looked back before the first candle we hold.
pub const MINUTES: i64 = 10;

/// Backfill `symbol` with the bars that start in `[from, before)` (epoch seconds, minute starts).
pub struct Job {
    pub symbol: String,
    pub ticker_id: u64,
    pub from: i64,
    pub before: i64,
}

impl Job {
    /// The `MINUTES` minutes before `before` (a ticker with no saved candles).
    pub fn recent(symbol: String, ticker_id: u64, before: i64) -> Job {
        Job { symbol, ticker_id, from: before - MINUTES * 60, before }
    }
}

/// Start of the gap between the newest saved candle (`saved`) and the first live one
/// (`first_live`): the minute after the saved one when it is from the same New York day,
/// else that day's 04:00. None when there is no gap.
pub fn gap_from(saved: i64, first_live: i64) -> Option<i64> {
    let day = ny_date(first_live);
    let open = nytime::ts(day, 4, 0)?;
    let from = if ny_date(saved) == day { (saved + 60).max(open) } else { open };
    (from < first_live).then_some(from)
}

/// Bars of `job`, oldest first, and whether a request failed (then the bars fetched so far
/// are returned and the job is asked again later). Stops at a gap into an earlier trading
/// date.
pub fn fetch(agent: &ureq::Agent, job: &Job) -> (Vec<Candle>, bool) {
    let date = ny_date(job.before);
    let mut out = Vec::new();
    // The bar ending at `before` is the last minute that started before it.
    let failed = history::walk_back(agent, job.ticker_id, Tf::M1, job.before, |c| {
        let keep = c.start >= job.from && c.start < job.before && ny_date(c.start) == date;
        if keep {
            out.push(c);
        }
        keep
    });
    out.reverse();
    (out, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny(d: u32, h: u32, m: u32) -> i64 {
        crate::nytime::test::ny(2026, 11, d, h, m)
    }

    #[test]
    fn gap_runs_from_the_saved_candle_or_the_day_start() {
        // saved today: from the minute after it
        assert_eq!(gap_from(ny(17, 9, 40), ny(17, 11, 0)), Some(ny(17, 9, 41)));
        // saved on an earlier day: from 04:00 today
        assert_eq!(gap_from(ny(16, 19, 59), ny(17, 11, 0)), Some(ny(17, 4, 0)));
        // no gap: the live candles follow the saved one, or the day has not opened
        assert_eq!(gap_from(ny(17, 10, 59), ny(17, 11, 0)), None);
        assert_eq!(gap_from(ny(16, 19, 59), ny(17, 3, 30)), None);
        // the 10-minute job before the first candle
        let j = Job::recent("A".into(), 1, ny(17, 11, 0));
        assert_eq!((j.from, j.before), (ny(17, 10, 50), ny(17, 11, 0)));
    }
}
