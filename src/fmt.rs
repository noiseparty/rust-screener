//! Time & number formats (Config -> Time & Format): time zone, date pattern, 12/24 h clock,
//! currency, separators, price decimals and volume style.
//!
//! Drawing code formats through `date`, `time`, `price`, `money` and `count`; `ui::draw` sets the
//! formats once per frame with `set` (like `theme::set`). Session phases stay New York based.

use chrono::{Local, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// Time zones offered with left/right; any other IANA name can be typed.
pub const ZONES: [&str; 12] = [
    "Local",
    "America/New_York",
    "America/Chicago",
    "America/Los_Angeles",
    "Europe/London",
    "Europe/Berlin",
    "Europe/Riga",
    "Asia/Tokyo",
    "Asia/Hong_Kong",
    "Asia/Singapore",
    "Australia/Sydney",
    "UTC",
];
/// Date patterns offered with left/right; any other pattern of `DD MM YY YYYY` can be typed.
pub const DATES: [&str; 5] = ["DD.MM.YY", "DD/MM/YYYY", "MM/DD/YY", "MM-DD-YYYY", "YYYY-MM-DD"];
pub const CURRENCIES: [&str; 4] = ["$", "€", "£", ""];
pub const DECIMALS: [char; 2] = ['.', ','];
pub const THOUSANDS: [&str; 4] = ["", ",", ".", " "];
/// Price decimals: 0 = Auto (2, or 4 under 1), 2, 4.
pub const PRICE_DECIMALS: [u8; 3] = [0, 2, 4];
/// Longest typed time zone or date pattern.
pub const INPUT_MAX: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Formats {
    /// "Local" or an IANA name such as "Europe/Riga".
    pub timezone: String,
    /// Tokens `DD MM YY YYYY` with any separators, e.g. "DD.MM.YY".
    pub date: String,
    pub clock_12h: bool,
    /// "$", "€", "£" or "" (none), before or after the number.
    pub currency: String,
    pub currency_after: bool,
    pub decimal: char,
    /// "" (none), ",", "." or " ".
    pub thousands: String,
    pub price_decimals: u8,
    /// Volume and market cap in full digits instead of K/M/B.
    pub volume_full: bool,
}

impl Default for Formats {
    fn default() -> Self {
        Formats {
            timezone: "Local".into(),
            date: DATES[0].into(),
            clock_12h: false,
            currency: "$".into(),
            currency_after: false,
            decimal: '.',
            thousands: String::new(),
            price_decimals: 0,
            volume_full: false,
        }
    }
}

impl Formats {
    /// Why these formats cannot be used, if they cannot.
    pub fn problem(&self) -> Option<&'static str> {
        if zone(&self.timezone).is_none() {
            Some("Unknown time zone: use Local or an IANA name like Europe/Riga")
        } else if pattern(&self.date).is_none() {
            Some("Date needs DD and MM (YY or YYYY optional), separators only")
        } else if self.thousands.starts_with(self.decimal) {
            Some("Decimal and thousands separators must differ")
        } else {
            None
        }
    }

    /// Each value that cannot be used goes back to its default.
    pub fn validate(&mut self) {
        let d = Formats::default();
        if zone(&self.timezone).is_none() {
            self.timezone = d.timezone;
        }
        if pattern(&self.date).is_none() {
            self.date = d.date;
        }
        if !DECIMALS.contains(&self.decimal) {
            self.decimal = d.decimal;
        }
        if !THOUSANDS.contains(&self.thousands.as_str()) || self.thousands.starts_with(self.decimal) {
            self.thousands = d.thousands;
        }
        if !CURRENCIES.contains(&self.currency.as_str()) {
            self.currency = d.currency;
        }
        if !PRICE_DECIMALS.contains(&self.price_decimals) {
            self.price_decimals = 0;
        }
    }
}

/// Rows of the Time & Format pane: label and help text. The first two are typed as well as picked.
pub const ROWS: [(&str, &str); 9] = [
    ("Time zone", "Zone of shown clock times and dates. Left/right picks one, or type any IANA name (Europe/Riga); Del clears. Session phases and the countdown stay New York time."),
    ("Date", "Left/right picks a format, or type your own with DD, MM, YY or YYYY and any separators (DD-MM-YYYY); Del clears."),
    ("Clock", "24h (14:05) or 12h (2:05pm)."),
    ("Currency", "Currency symbol on prices: $, €, £ or none."),
    ("Symbol", "Currency symbol before ($3.86) or after (3.86$) the number."),
    ("Decimal", "Decimal separator: dot or comma."),
    ("Thousands", "Thousands separator: none, comma, dot or space."),
    ("Price decimals", "Auto: 2 decimals, 4 under 1. Or always 2, or always 4."),
    ("Volume", "Volume and market cap abbreviated (18.40M) or in full digits (18400000)."),
];
/// Rows that take typing.
pub const TYPED_ROWS: usize = 2;

/// Next (or previous) of `opts` after `cur`; an unknown value goes to the first.
fn cycle<T: PartialEq + Clone>(opts: &[T], cur: &T, back: bool) -> T {
    let n = opts.len();
    let i = match opts.iter().position(|o| o == cur) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None => 0,
    };
    opts[i].clone()
}

impl Formats {
    /// Row `row`'s value as the pane shows it.
    pub fn row_text(&self, row: usize) -> String {
        match row {
            0 => self.timezone.clone(),
            1 => self.date.clone(),
            2 => if self.clock_12h { "12h" } else { "24h" }.into(),
            3 if self.currency.is_empty() => "none".into(),
            3 => self.currency.clone(),
            4 => if self.currency_after { "after" } else { "before" }.into(),
            5 => self.decimal.to_string(),
            6 => match self.thousands.as_str() {
                "" => "none".into(),
                " " => "space".into(),
                s => s.into(),
            },
            7 if self.price_decimals == 0 => "Auto".into(),
            7 => self.price_decimals.to_string(),
            _ => if self.volume_full { "full" } else { "abbreviated" }.into(),
        }
    }

    /// Steps row `row` to its next (or previous) choice.
    pub fn step(&mut self, row: usize, back: bool) {
        let zones: Vec<String> = ZONES.iter().map(|s| s.to_string()).collect();
        let dates: Vec<String> = DATES.iter().map(|s| s.to_string()).collect();
        let currencies: Vec<String> = CURRENCIES.iter().map(|s| s.to_string()).collect();
        let thousands: Vec<String> = THOUSANDS.iter().map(|s| s.to_string()).collect();
        match row {
            0 => self.timezone = cycle(&zones, &self.timezone, back),
            1 => self.date = cycle(&dates, &self.date, back),
            2 => self.clock_12h = !self.clock_12h,
            3 => self.currency = cycle(&currencies, &self.currency, back),
            4 => self.currency_after = !self.currency_after,
            5 => self.decimal = cycle(&DECIMALS, &self.decimal, back),
            6 => self.thousands = cycle(&thousands, &self.thousands, back),
            7 => self.price_decimals = cycle(&PRICE_DECIMALS, &self.price_decimals, back),
            _ => self.volume_full = !self.volume_full,
        }
    }

    /// The typed text of row 0 (time zone) or 1 (date); None on the other rows.
    pub fn typed(&mut self, row: usize) -> Option<&mut String> {
        match row {
            0 => Some(&mut self.timezone),
            1 => Some(&mut self.date),
            _ => None,
        }
    }

    /// One line showing these formats, e.g. "Local · 17.11.26 14:05 · $1234.50 · 18.40M".
    pub fn summary(&self) -> String {
        let a = Active::new(self);
        let ts = chrono::Utc::now().timestamp();
        format!("{} · {} {} · {} · {}", self.timezone, a.date_with(ts, &a.date), a.clock(ts, false), a.money(1234.5), a.count(18_400_000.0))
    }
}

#[derive(Clone, Copy)]
enum Zone {
    Local,
    Tz(Tz),
}

fn zone(name: &str) -> Option<Zone> {
    if name == "Local" { Some(Zone::Local) } else { name.parse::<Tz>().ok().map(Zone::Tz) }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Day,
    Month,
    Year2,
    Year4,
    Sep(char),
}

/// The tokens of a date pattern; None unless it holds DD and MM once each, at most one year
/// and no other letters or digits.
fn pattern(p: &str) -> Option<Vec<Tok>> {
    let mut toks = Vec::new();
    let mut rest = p;
    while let Some(c) = rest.chars().next() {
        let (tok, len) = if rest.starts_with("YYYY") {
            (Tok::Year4, 4)
        } else if rest.starts_with("YY") {
            (Tok::Year2, 2)
        } else if rest.starts_with("DD") {
            (Tok::Day, 2)
        } else if rest.starts_with("MM") {
            (Tok::Month, 2)
        } else if c.is_alphanumeric() {
            return None;
        } else {
            (Tok::Sep(c), c.len_utf8())
        };
        toks.push(tok);
        rest = &rest[len..];
    }
    let n = |f: fn(&Tok) -> bool| toks.iter().filter(|t| f(t)).count();
    let ok = n(|t| *t == Tok::Day) == 1 && n(|t| *t == Tok::Month) == 1 && n(|t| matches!(t, Tok::Year2 | Tok::Year4)) <= 1;
    (ok && p.chars().count() <= INPUT_MAX).then_some(toks)
}

/// Day and month only (chart time axis on 4h and D): the year and the separators beside it go.
fn short(toks: &[Tok]) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    for &t in toks.iter().filter(|t| !matches!(t, Tok::Year2 | Tok::Year4)) {
        let sep = matches!(t, Tok::Sep(_));
        if sep && (out.is_empty() || matches!(out.last(), Some(Tok::Sep(_)))) {
            continue;
        }
        out.push(t);
    }
    while matches!(out.last(), Some(Tok::Sep(_))) {
        out.pop();
    }
    out
}

struct Active {
    f: Formats,
    zone: Zone,
    date: Vec<Tok>,
    day: Vec<Tok>,
}

impl Active {
    fn new(f: &Formats) -> Active {
        let mut f = f.clone();
        f.validate();
        let date = pattern(&f.date).unwrap_or_default();
        Active { zone: zone(&f.timezone).unwrap_or(Zone::Local), day: short(&date), date, f }
    }

    fn at(&self, ts: i64) -> Option<NaiveDateTime> {
        match self.zone {
            Zone::Local => Local.timestamp_opt(ts, 0).single().map(|t| t.naive_local()),
            Zone::Tz(z) => z.timestamp_opt(ts, 0).single().map(|t| t.naive_local()),
        }
    }

    fn date_with(&self, ts: i64, toks: &[Tok]) -> String {
        let Some(t) = self.at(ts) else { return String::new() };
        toks.iter()
            .map(|tok| match tok {
                Tok::Day => t.format("%d").to_string(),
                Tok::Month => t.format("%m").to_string(),
                Tok::Year2 => t.format("%y").to_string(),
                Tok::Year4 => t.format("%Y").to_string(),
                Tok::Sep(c) => c.to_string(),
            })
            .collect()
    }

    fn clock(&self, ts: i64, secs: bool) -> String {
        let spec = match (self.f.clock_12h, secs) {
            (false, false) => "%H:%M",
            (false, true) => "%H:%M:%S",
            (true, false) => "%-I:%M%P",
            (true, true) => "%-I:%M:%S%P",
        };
        self.at(ts).map_or_else(String::new, |t| t.format(spec).to_string())
    }

    fn num(&self, v: f64, d: usize) -> String {
        let s = format!("{:.d$}", v.abs());
        let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
        let mut out = String::new();
        for (i, c) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                out.push_str(&self.f.thousands);
            }
            out.push(c);
        }
        if !frac.is_empty() {
            out.push(self.f.decimal);
            out.push_str(frac);
        }
        if v < 0.0 && s.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
            out.insert(0, '-');
        }
        out
    }

    fn price(&self, v: f64) -> String {
        let d = match self.f.price_decimals {
            0 if v.abs() < 1.0 => 4,
            0 => 2,
            d => usize::from(d),
        };
        self.num(v, d)
    }

    fn money(&self, v: f64) -> String {
        let (n, c) = (self.price(v), &self.f.currency);
        if self.f.currency_after { format!("{n}{c}") } else { format!("{c}{n}") }
    }

    fn count(&self, v: f64) -> String {
        if self.f.volume_full {
            return self.num(v.round(), 0);
        }
        let a = v.abs();
        let (n, suf) = match a {
            _ if a >= 1e12 => (v / 1e12, "T"),
            _ if a >= 1e9 => (v / 1e9, "B"),
            _ if a >= 1e6 => (v / 1e6, "M"),
            _ if a >= 1e3 => (v / 1e3, "K"),
            _ => return format!("{v:.0}"),
        };
        format!("{}{suf}", self.num(n, if n.abs() >= 100.0 { 1 } else { 2 }))
    }
}

thread_local! {
    static CUR: RefCell<Active> = RefCell::new(Active::new(&Formats::default()));
}

/// Uses `f` for everything formatted on this thread from now on (values that cannot be used
/// fall back to their defaults).
pub fn set(f: &Formats) {
    CUR.with(|c| {
        if c.borrow().f != *f {
            *c.borrow_mut() = Active::new(f);
        }
    });
}

fn with<T>(g: impl FnOnce(&Active) -> T) -> T {
    CUR.with(|c| g(&c.borrow()))
}

/// The date of `ts` (epoch seconds) in the date pattern, e.g. "17.11.26".
pub fn date(ts: i64) -> String {
    with(|a| a.date_with(ts, &a.date))
}

/// Day and month of `ts` in the pattern's order, e.g. "17.11" (chart time axis on 4h and D).
pub fn day(ts: i64) -> String {
    with(|a| a.date_with(ts, &a.day))
}

/// The clock time of `ts`: "14:05" or "2:05pm".
pub fn time(ts: i64) -> String {
    with(|a| a.clock(ts, false))
}

/// The clock time of `ts` with seconds: "14:05:09" or "2:05:09pm".
pub fn time_s(ts: i64) -> String {
    with(|a| a.clock(ts, true))
}

/// `v` with `d` decimals and the chosen separators.
pub fn num(v: f64, d: usize) -> String {
    with(|a| a.num(v, d))
}

/// A price: 2 decimals, or 4 under 1 (Auto), or the fixed count chosen.
pub fn price(v: f64) -> String {
    with(|a| a.price(v))
}

/// A price with the currency symbol, e.g. "$3.86" or "3,86€".
pub fn money(v: f64) -> String {
    with(|a| a.money(v))
}

/// A volume or market cap: abbreviated ("18.40M") or in full digits ("18,400,000").
pub fn count(v: f64) -> String {
    with(|a| a.count(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::New_York;

    /// 2026-11-17 14:05:09 UTC.
    const TS: i64 = 1_794_924_309;

    fn utc(f: Formats) -> Formats {
        Formats { timezone: "UTC".into(), ..f }
    }

    #[test]
    fn every_date_preset_formats() {
        let want = ["17.11.26", "17/11/2026", "11/17/26", "11-17-2026", "2026-11-17"];
        for (p, w) in DATES.iter().zip(want) {
            set(&utc(Formats { date: p.to_string(), ..Formats::default() }));
            assert_eq!(date(TS), w, "{p}");
        }
        set(&Formats::default());
    }

    #[test]
    fn custom_patterns_parse_and_short_dates_drop_the_year() {
        set(&utc(Formats { date: "YYYY.MM.DD".into(), ..Formats::default() }));
        assert_eq!((date(TS), day(TS)), ("2026.11.17".into(), "11.17".into()));
        set(&utc(Formats { date: "DD MM".into(), ..Formats::default() }));
        assert_eq!((date(TS), day(TS)), ("17 11".into(), "17 11".into()));
        set(&utc(Formats { date: "DD/MM/YYYY".into(), ..Formats::default() }));
        assert_eq!(day(TS), "17/11");
        set(&Formats::default());
        for bad in ["", "DD.YY", "DD.MM.YY.YY", "DD.MM.XX", "D.M.Y", "DD.MM.2026", "DDMMDD"] {
            assert!(pattern(bad).is_none(), "{bad}");
        }
        assert!(pattern("[DD-MM]").is_some());
    }

    #[test]
    fn time_zones_convert_and_clock_styles() {
        set(&utc(Formats::default()));
        assert_eq!((time(TS), time_s(TS)), ("14:05".into(), "14:05:09".into()));
        set(&Formats { timezone: "America/New_York".into(), clock_12h: true, ..Formats::default() });
        assert_eq!((time(TS), time_s(TS)), ("9:05am".into(), "9:05:09am".into()));
        assert_eq!(New_York.timestamp_opt(TS, 0).unwrap().format("%H").to_string(), "09");
        set(&Formats { timezone: "Asia/Tokyo".into(), ..Formats::default() });
        assert_eq!((time(TS), date(TS)), ("23:05".into(), "17.11.26".into()));
        set(&Formats { timezone: "Australia/Sydney".into(), ..Formats::default() });
        assert_eq!(date(TS), "18.11.26");
        set(&Formats::default());
        assert!(zone("Europe/Riga").is_some() && zone("Local").is_some());
        assert!(zone("Mars/Base").is_none() && zone("").is_none());
    }

    #[test]
    fn separators_currency_and_decimals() {
        set(&Formats::default());
        assert_eq!((price(3.856), price(0.12345), money(1234.5)), ("3.86".into(), "0.1235".into(), "$1234.50".into()));
        set(&Formats { decimal: ',', thousands: ".".into(), currency: "€".into(), currency_after: true, ..Formats::default() });
        assert_eq!((money(1234567.891), money(0.5), num(-1234.5, 1)), ("1.234.567,89€".into(), "0,5000€".into(), "-1.234,5".into()));
        set(&Formats { thousands: " ".into(), price_decimals: 4, currency: String::new(), ..Formats::default() });
        assert_eq!((money(12.5), price(-0.00001)), ("12.5000".into(), "0.0000".into()));
        set(&Formats { thousands: ",".into(), price_decimals: 2, ..Formats::default() });
        assert_eq!((price(0.1234), num(999.0, 0), num(1000.0, 0)), ("0.12".into(), "999".into(), "1,000".into()));
        set(&Formats::default());
    }

    #[test]
    fn counts_abbreviated_or_full() {
        set(&Formats::default());
        assert_eq!((count(950.0), count(18_400_000.0), count(123_456_789.0), count(2.5e9)), ("950".into(), "18.40M".into(), "123.5M".into(), "2.50B".into()));
        set(&Formats { volume_full: true, thousands: ",".into(), ..Formats::default() });
        assert_eq!(count(18_400_000.4), "18,400,000");
        set(&Formats { decimal: ',', ..Formats::default() });
        assert_eq!(count(18_400_000.0), "18,40M");
        set(&Formats::default());
    }

    #[test]
    fn pane_rows_step_and_take_typing() {
        let mut f = Formats::default();
        f.step(0, false);
        assert_eq!(f.timezone, "America/New_York");
        f.step(0, true);
        f.step(0, true);
        assert_eq!(f.timezone, "UTC");
        // a typed zone is checked; stepping from it goes back to the first choice
        *f.typed(0).unwrap() = "Europe/Vilnius".into();
        assert_eq!(f.problem(), None);
        f.step(0, false);
        assert_eq!(f.timezone, "Local");
        f.step(1, true);
        assert_eq!(f.row_text(1), "YYYY-MM-DD");
        f.step(6, false);
        assert_eq!(f.row_text(6), ",");
        f.step(6, false);
        assert_eq!(f.row_text(6), ".");
        assert!(f.problem().is_some());
        f.step(7, false);
        f.step(8, false);
        f.step(3, true);
        assert_eq!((f.row_text(7), f.row_text(8), f.row_text(3)), ("2".into(), "full".into(), "none".into()));
        assert!(f.typed(2).is_none());
    }

    #[test]
    fn problems_and_validation() {
        assert_eq!(Formats::default().problem(), None);
        assert!(Formats { timezone: "Nowhere".into(), ..Formats::default() }.problem().is_some());
        assert!(Formats { date: "MM".into(), ..Formats::default() }.problem().is_some());
        assert!(Formats { thousands: ".".into(), ..Formats::default() }.problem().is_some());
        let mut f = Formats { timezone: "x".into(), date: "y".into(), decimal: ';', thousands: "_".into(), currency: "¥".into(), price_decimals: 3, ..Formats::default() };
        f.validate();
        assert_eq!(f, Formats::default());
        // an old config without the key gets the defaults
        let f: Formats = serde_json::from_str("{}").unwrap();
        assert_eq!(f, Formats::default());
    }
}
