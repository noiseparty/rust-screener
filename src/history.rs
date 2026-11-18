//! Chart bars from Webull's chart endpoint for every timeframe.
//!
//! Anonymously an intraday request (`m1`, `m5`, `m60`, `m240`) returns one bar: the latest
//! bar whose END is at or before `timestamp` (the in-progress bar when `timestamp` is past
//! now). Minutes or hours without trades have no bar. Daily (`d1`) bars come back in bulk,
//! newest first, labelled with the START of the New York day.

use crate::engine::Candle;
use crate::nytime::{self, AFTER_CLOSE, PRE_OPEN, REG_CLOSE, REG_OPEN};
use serde_json::Value;

const API: &str = "https://quotes-gw.webullfintech.com/api/quote/charts/query?extendTrading=1";
/// Bars fetched for a timeframe's chart.
pub const BARS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tf {
    M1,
    M5,
    H1,
    H4,
    D1,
}

impl Tf {
    pub const ALL: [Tf; 5] = [Tf::M1, Tf::M5, Tf::H1, Tf::H4, Tf::D1];

    /// The next timeframe, after D back to 1m.
    pub fn next(self) -> Tf {
        let i = Tf::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Tf::ALL[(i + 1) % Tf::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Tf::M1 => "1m",
            Tf::M5 => "5m",
            Tf::H1 => "1h",
            Tf::H4 => "4h",
            Tf::D1 => "D",
        }
    }

    fn api(self) -> &'static str {
        match self {
            Tf::M1 => "m1",
            Tf::M5 => "m5",
            Tf::H1 => "m60",
            Tf::H4 => "m240",
            Tf::D1 => "d1",
        }
    }

    pub fn secs(self) -> i64 {
        match self {
            Tf::M1 => 60,
            Tf::M5 => 300,
            Tf::H1 => 3600,
            Tf::H4 => 14_400,
            Tf::D1 => 86_400,
        }
    }
}

/// Which bars a request wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The newest `BARS` bars (a chart from scratch).
    Full,
    /// The newest bars back to the one starting at this time (refresh of a held or cached
    /// chart), at most `BARS`: an answer that does not reach it replaces what is held.
    Since(i64),
    /// `BARS` bars before the one starting at this time (scrolling back).
    Before(i64),
}

pub struct Req {
    pub symbol: String,
    pub ticker_id: u64,
    pub tf: Tf,
    pub mode: Mode,
}

/// Start of the intraday bar labelled with its end `label`. Bars are cut at the sessions
/// (New York 04:00, 09:30, 16:00) and counted from each session's start, so a 4h bar can be
/// 08:00-09:30 or 13:30-16:00 and a 1h bar 09:00-09:30.
fn bar_start(label: i64, secs: i64) -> i64 {
    use chrono::Timelike;
    let Some(t) = nytime::at(label) else { return label - secs };
    let m = i64::from(nytime::minute_of_day(&t));
    let sess = [REG_CLOSE, REG_OPEN, PRE_OPEN].into_iter().map(i64::from).find(|&s| s < m);
    match sess {
        Some(s) if m <= i64::from(AFTER_CLOSE) => {
            let from = label - (m - s) * 60 - i64::from(t.second());
            from + (label - 1 - from) / secs * secs
        }
        _ => label - secs,
    }
}

/// End of the bar starting at `start`: one timeframe later, but intraday bars stop at the
/// session cuts (New York 09:30, 16:00, 20:00, as `bar_start`) and a daily bar at the next
/// New York midnight.
pub fn bar_end(start: i64, tf: Tf) -> i64 {
    use chrono::{Days, Timelike};
    let Some(t) = nytime::at(start) else { return start + tf.secs() };
    if tf == Tf::D1 {
        let next = t.date_naive().checked_add_days(Days::new(1));
        return next.and_then(|d| nytime::ts(d, 0, 0)).unwrap_or(start + tf.secs());
    }
    let m = i64::from(nytime::minute_of_day(&t));
    let cut = [REG_OPEN, REG_CLOSE, AFTER_CLOSE].into_iter().map(i64::from).find(|&c| c > m);
    let full = start + tf.secs();
    cut.map_or(full, |c| full.min(start + (c - m) * 60 - i64::from(t.second())))
}

/// One bar line: `label,open,close,high,low,prevClose,volume,vwap`.
fn parse_bar(s: &str, tf: Tf) -> Option<Candle> {
    let f: Vec<&str> = s.split(',').collect();
    let num = |i: usize| f.get(i)?.trim().parse::<f64>().ok();
    let label: i64 = f.first()?.trim().parse().ok()?;
    Some(Candle {
        start: if tf == Tf::D1 { label } else { bar_start(label, tf.secs()) },
        open: num(1)?,
        close: num(2)?,
        high: num(3)?,
        low: num(4)?,
        volume: num(6)? as i64,
    })
}

fn query(agent: &ureq::Agent, ticker_id: u64, tf: Tf, extra: &str) -> Option<Vec<Candle>> {
    let url = format!("{API}&type={}&tickerIds={ticker_id}{extra}", tf.api());
    let body = crate::net::get_json(agent, &url).ok()?;
    // a valid answer without bar data (e.g. before a listing) means no bars, not a failed request
    // (an error object instead of the usual array still counts as a failure)
    body.as_array()?;
    let Some(data) = body.get(0).and_then(|b| b.get("data")).and_then(Value::as_array) else { return Some(Vec::new()) };
    Some(data.iter().filter_map(|v| parse_bar(v.as_str()?, tf)).collect())
}

/// The newest bar whose period ends at or before `ts`; `Err` when the request failed (network,
/// HTTP error, the shared limiter's 403 pause), `Ok(None)` when Webull has no such bar.
pub fn try_bar_at(agent: &ureq::Agent, ticker_id: u64, tf: Tf, ts: i64) -> Result<Option<Candle>, ()> {
    query(agent, ticker_id, tf, &format!("&count=1&timestamp={ts}")).map(|v| v.into_iter().next()).ok_or(())
}

/// Walks back one bar at a time from the bar ending at or before `ts`, newest first, handing
/// each bar to `keep` until it returns false or Webull has no older bar. True when a request
/// failed.
pub fn walk_back(agent: &ureq::Agent, ticker_id: u64, tf: Tf, mut ts: i64, mut keep: impl FnMut(Candle) -> bool) -> bool {
    loop {
        match try_bar_at(agent, ticker_id, tf, ts) {
            Ok(Some(c)) if keep(c) => ts = c.start,
            Ok(_) => return false,
            Err(()) => return true,
        }
    }
}

/// Intraday bars come one request each; the chart is handed over every this many bars.
const PROGRESS_EVERY: usize = 8;

/// Bars for `req`, oldest first, and whether a request failed (then whatever was fetched is
/// returned, and an empty answer does not mean "no more bars"). While walking back for a
/// `Full` or `Before` chart, `progress` gets the bars so far (oldest first) every
/// `PROGRESS_EVERY` (a `Since` answer is only merged once it reaches the held bars).
pub fn fetch(agent: &ureq::Agent, req: &Req, now: i64, mut progress: impl FnMut(Vec<Candle>)) -> (Vec<Candle>, bool) {
    if req.tf == Tf::D1 {
        let extra = match req.mode {
            Mode::Full => format!("&count={BARS}"),
            // enough days to reach the held bar (a cached chart can be days old)
            Mode::Since(s) => format!("&count={}", ((now - s) / 86_400 + 2).clamp(5, BARS as i64)),
            Mode::Before(s) => format!("&count={BARS}&timestamp={}", s - 1),
        };
        let Some(mut out) = query(agent, req.ticker_id, req.tf, &extra) else { return (Vec::new(), true) };
        out.sort_by_key(|c| c.start);
        return (out, false);
    }
    // Walk back bar by bar (newest first).
    let mut out: Vec<Candle> = Vec::new();
    let from = match req.mode {
        Mode::Full | Mode::Since(_) => now + req.tf.secs(),
        // the bar ending at `s` is the one before the bar starting at `s`
        Mode::Before(s) => s,
    };
    // a top-up walks back until it reaches the held bar, so a cache from days ago is not cut off
    let limit = if matches!(req.mode, Mode::Since(_)) { MAX_HELD } else { BARS };
    let failed = walk_back(agent, req.ticker_id, req.tf, from, |c| {
        if out.last().is_some_and(|l| c.start >= l.start) {
            return false;
        }
        out.push(c);
        if matches!(req.mode, Mode::Since(s) if c.start <= s) {
            return false;
        }
        if out.len() % PROGRESS_EVERY == 0 && out.len() < BARS && !matches!(req.mode, Mode::Since(_)) {
            progress(out.iter().rev().copied().collect());
        }
        out.len() < limit
    });
    out.reverse();
    (out, failed)
}

/// Most bars held per symbol and timeframe (scrolling back adds `BARS` at a time).
pub const MAX_HELD: usize = 2000;

/// Folds `fresh` bars into `bars` (both oldest first): a bar with the same start is replaced,
/// others are inserted in order.
pub fn merge(bars: &mut Vec<Candle>, fresh: &[Candle]) {
    for c in fresh {
        match bars.binary_search_by_key(&c.start, |b| b.start) {
            Ok(i) => bars[i] = *c,
            Err(i) => bars.insert(i, *c),
        }
    }
    let extra = bars.len().saturating_sub(MAX_HELD);
    bars.drain(..extra);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bar_lines_per_timeframe() {
        let line = "1794859200,352.22,352.43,352.45,352.21,352.22,37375,353.99";
        let c = parse_bar(line, Tf::M1).unwrap();
        assert_eq!(c, Candle { start: 1794859140, open: 352.22, high: 352.45, low: 352.21, close: 352.43, volume: 37375 });
        // 1794859200 = Mon 16 Nov 2026 15:00 New York: inside the 13:30-16:00 4h bar
        assert_eq!(parse_bar(line, Tf::H4).unwrap().start, 1794859200 - 90 * 60);
        // daily bars are labelled with the day start
        assert_eq!(parse_bar(line, Tf::D1).unwrap().start, 1794859200);
        assert!(parse_bar("1794859200,1,2", Tf::M1).is_none());
        assert!(parse_bar("", Tf::M1).is_none());
    }

    #[test]
    fn session_cut_bars_start_at_their_real_start() {
        // Webull end labels of Mon 16 Nov 2026 (New York, EST = UTC-5)
        let ny = |h: i64, m: i64| 1794805200 + h * 3600 + m * 60; // 00:00 NY
        let h4 = |end| bar_start(end, 14_400);
        assert_eq!(h4(ny(8, 0)), ny(4, 0));
        assert_eq!(h4(ny(9, 30)), ny(8, 0));
        assert_eq!(h4(ny(13, 30)), ny(9, 30));
        assert_eq!(h4(ny(16, 0)), ny(13, 30));
        assert_eq!(h4(ny(20, 0)), ny(16, 0));
        let h1 = |end| bar_start(end, 3600);
        assert_eq!(h1(ny(9, 30)), ny(9, 0));
        assert_eq!(h1(ny(10, 30)), ny(9, 30));
        assert_eq!(h1(ny(16, 0)), ny(15, 30));
        assert_eq!(h1(ny(17, 0)), ny(16, 0));
        assert_eq!(bar_start(ny(10, 1), 60), ny(10, 0));
    }

    #[test]
    fn bar_end_stops_at_session_cuts_and_midnight() {
        let ny = |h: i64, m: i64| 1794805200 + h * 3600 + m * 60; // Mon 16 Nov 2026 00:00 NY
        assert_eq!(bar_end(ny(4, 0), Tf::H4), ny(8, 0));
        assert_eq!(bar_end(ny(8, 0), Tf::H4), ny(9, 30));
        assert_eq!(bar_end(ny(13, 30), Tf::H4), ny(16, 0));
        assert_eq!(bar_end(ny(16, 0), Tf::H4), ny(20, 0));
        assert_eq!(bar_end(ny(9, 0), Tf::H1), ny(9, 30));
        assert_eq!(bar_end(ny(15, 30), Tf::H1), ny(16, 0));
        assert_eq!(bar_end(ny(10, 0), Tf::M5), ny(10, 5));
        assert_eq!(bar_end(ny(10, 0), Tf::M1), ny(10, 1));
        assert_eq!(bar_end(ny(0, 0), Tf::D1), ny(24, 0));
        // Sun 1 Nov 2026: clocks go back, so that New York day is 25 hours long
        let nov1 = 1793505600; // 00:00 EDT
        assert_eq!(bar_end(nov1, Tf::D1), nov1 + 25 * 3600);
    }

    #[test]
    fn merge_replaces_same_start_and_inserts_in_order() {
        let c = |start, p| Candle { start, open: p, high: p, low: p, close: p, volume: 1 };
        let mut bars = vec![c(0, 1.0), c(60, 2.0)];
        merge(&mut bars, &[c(60, 3.0), c(120, 4.0), c(30, 9.0), c(-60, 0.5)]);
        assert_eq!(bars, vec![c(-60, 0.5), c(0, 1.0), c(30, 9.0), c(60, 3.0), c(120, 4.0)]);
    }
}
