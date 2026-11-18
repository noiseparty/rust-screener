use crate::nytime::{self, AFTER_CLOSE, PRE_OPEN, REG_CLOSE, REG_OPEN};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};

/// US equity session, derived from New York time so DST is always correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Pre,
    Reg,
    After,
    Closed,
}

impl Session {
    pub fn now() -> Session {
        Session::at(Utc::now())
    }

    pub fn at<Tz: TimeZone>(t: DateTime<Tz>) -> Session {
        match nytime::minute_of_day(&nytime::of(&t)) {
            m if m < PRE_OPEN => Session::Closed,
            m if m < REG_OPEN => Session::Pre,
            m if m < REG_CLOSE => Session::Reg,
            m if m < AFTER_CLOSE => Session::After,
            _ => Session::Closed,
        }
    }

    /// The phase of the trading day at `t`: its header label, the countdown label and the seconds
    /// until it ends. Phases, New York time (Riga winter time in brackets): pre-market
    /// 04:00-09:30 (11:00-16:30), regular 09:30-10:30 (16:30-17:30), zombie hour 10:30-11:30
    /// (17:30-18:30), regular to 16:00 (23:00), after-market to 20:00 (03:00). Outside them it
    /// counts to the next weekday's pre-market (US holidays are not known).
    pub fn phase<Tz: TimeZone>(t: DateTime<Tz>) -> (&'static str, &'static str, i64) {
        let ny = nytime::of(&t);
        let weekday = ny.weekday().number_from_monday() <= 5;
        let m = nytime::minute_of_day(&ny);
        let phases: [(u32, &str, &str); 5] = [
            (REG_OPEN, "Pre Market", "open in"),
            (10 * 60 + 30, "Regular Market", "zombie hour in"),
            (11 * 60 + 30, "Regular - Zombie Hour", "ends in"),
            (REG_CLOSE, "Regular Market", "close in"),
            (AFTER_CLOSE, "After Market", "closes in"),
        ];
        if weekday && m >= PRE_OPEN {
            if let Some(&(end, label, until)) = phases.iter().find(|(end, _, _)| m < *end) {
                let secs = i64::from(end * 60) - i64::from(m * 60 + ny.second());
                return (label, until, secs);
            }
        }
        // next weekday 04:00
        let mut day = ny.date_naive();
        if m >= PRE_OPEN || !weekday {
            day = day.succ_opt().unwrap_or(day);
        }
        while day.weekday().number_from_monday() > 5 {
            day = day.succ_opt().unwrap_or(day);
        }
        let open = nytime::ts(day, PRE_OPEN / 60, 0).map_or(0, |o| o - t.with_timezone(&Utc).timestamp());
        ("Market Closed", "pre-market in", open.max(0))
    }

    pub fn code(self) -> &'static str {
        match self {
            Session::Pre => "PRE",
            Session::Reg => "REG",
            Session::After => "AFT",
            Session::Closed => "CLOSED",
        }
    }

    /// Webull group names to try, in order of preference.
    pub fn group_targets(self) -> &'static [&'static str] {
        match self {
            Session::Pre => &["pre-market gainers", "pre-market", "top gainers"],
            Session::After => &["after-hours gainers", "after-market gainers", "after-hours", "top gainers"],
            _ => &["top gainers"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::New_York;

    fn ny(h: u32, m: u32) -> DateTime<chrono_tz::Tz> {
        New_York.with_ymd_and_hms(2026, 1, 12, h, m, 0).unwrap()
    }

    #[test]
    fn boundaries() {
        assert_eq!(Session::at(ny(3, 59)), Session::Closed);
        assert_eq!(Session::at(ny(4, 0)), Session::Pre);
        assert_eq!(Session::at(ny(9, 29)), Session::Pre);
        assert_eq!(Session::at(ny(9, 30)), Session::Reg);
        assert_eq!(Session::at(ny(15, 59)), Session::Reg);
        assert_eq!(Session::at(ny(16, 0)), Session::After);
        assert_eq!(Session::at(ny(19, 59)), Session::After);
        assert_eq!(Session::at(ny(20, 0)), Session::Closed);
    }

    #[test]
    fn phases() {
        // 2026-01-12 is a Monday
        let at = |h, m, s| New_York.with_ymd_and_hms(2026, 1, 12, h, m, s).unwrap();
        assert_eq!(Session::phase(at(8, 0, 0)), ("Pre Market", "open in", 90 * 60));
        assert_eq!(Session::phase(at(9, 29, 30)), ("Pre Market", "open in", 30));
        assert_eq!(Session::phase(at(9, 30, 0)), ("Regular Market", "zombie hour in", 3600));
        assert_eq!(Session::phase(at(10, 45, 0)), ("Regular - Zombie Hour", "ends in", 45 * 60));
        assert_eq!(Session::phase(at(11, 30, 0)), ("Regular Market", "close in", 270 * 60));
        assert_eq!(Session::phase(at(16, 0, 0)), ("After Market", "closes in", 4 * 3600));
        // after 20:00 and before 04:00: the next pre-market
        assert_eq!(Session::phase(at(20, 0, 0)), ("Market Closed", "pre-market in", 8 * 3600));
        assert_eq!(Session::phase(at(3, 0, 0)), ("Market Closed", "pre-market in", 3600));
        // Riga 16:30 in winter is the open
        let riga = chrono_tz::Europe::Riga.with_ymd_and_hms(2026, 1, 12, 16, 0, 0).unwrap();
        assert_eq!(Session::phase(riga), ("Pre Market", "open in", 30 * 60));
        // Friday evening and the weekend count to Monday 04:00
        let fri = New_York.with_ymd_and_hms(2026, 1, 16, 21, 0, 0).unwrap();
        assert_eq!(Session::phase(fri), ("Market Closed", "pre-market in", (3 + 48 + 4) * 3600));
        let sun = New_York.with_ymd_and_hms(2026, 1, 18, 12, 0, 0).unwrap();
        assert_eq!(Session::phase(sun), ("Market Closed", "pre-market in", 16 * 3600));
    }

    #[test]
    fn riga_equivalents_in_winter() {
        // Riga 16:30 == NY 09:30 in January (UTC+2 vs UTC-5)
        let riga = chrono_tz::Europe::Riga.with_ymd_and_hms(2026, 1, 12, 16, 30, 0).unwrap();
        assert_eq!(Session::at(riga), Session::Reg);
        let riga = chrono_tz::Europe::Riga.with_ymd_and_hms(2026, 1, 13, 2, 59, 0).unwrap();
        assert_eq!(Session::at(riga), Session::After);
    }

    #[test]
    fn us_dst_gap_handled() {
        // 2026-03-09: US already on EDT, EU not yet on summer time.
        // NY 09:30 is Riga 15:30 that week; the old Riga-based rule said PRE.
        let t = New_York.with_ymd_and_hms(2026, 3, 9, 9, 30, 0).unwrap();
        assert_eq!(Session::at(t), Session::Reg);
    }
}
