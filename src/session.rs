use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;

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
        let ny = t.with_timezone(&New_York);
        let m = ny.hour() * 60 + ny.minute();
        match m {
            m if m < 4 * 60 => Session::Closed,
            m if m < 9 * 60 + 30 => Session::Pre,
            m if m < 16 * 60 => Session::Reg,
            m if m < 20 * 60 => Session::After,
            _ => Session::Closed,
        }
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
