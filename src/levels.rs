//! 4h swing levels (horizontal support / resistance lines), ported from the SA_LIVE Pine
//! script's "4h swing lines".
//!
//! Webull's 4h bars follow the sessions (New York): 04:00-08:00, 08:00-09:30, 09:30-13:30,
//! 13:30-16:00, 16:00-20:00. Those end times are fixed, so a year of bars is fetched by
//! asking for each bar end directly (one request per bar; ~1300 for a year), in batches of
//! `BATCH`, newest first, so recent lines show within a minute and the app can switch to a
//! more urgent ticker between batches. Bars are cached per symbol on disk.

use crate::engine::Candle;
use crate::history::{self, Tf};
use chrono::{Datelike, Duration, NaiveTime, TimeZone, Timelike, Weekday};
use chrono_tz::America::New_York;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How far back 4h bars are fetched.
pub const LOOKBACK_DAYS: i64 = 365;
const LOOKBACK_SECS: i64 = LOOKBACK_DAYS * 86_400;
/// Bars per request (~12 s at the shared 5 requests/s budget).
const BATCH: usize = 60;
/// Threads per batch; the shared request budget (`limiter`) sets the actual rate.
const THREADS: usize = 2;
/// A bar is requested this long after its end, so Webull has finished it.
pub const SETTLE_SECS: i64 = 60;
/// Cache files untouched this long are deleted. There is no size cap: Config -> Storage lists the
/// files so the user can delete what they no longer need.
const CACHE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);
/// End times (New York) of the 4h bars of a weekday.
const BAR_ENDS: [(u32, u32); 5] = [(8, 0), (9, 30), (13, 30), (16, 0), (20, 0)];

/// Pine defaults: older long-term highs shown, wick / close throughs that remove a line and
/// the line cap. (The Pine distance window is not used: the chart picks the open lines
/// nearest the live price itself.)
const OLDER_LTH: usize = 6;
const WICK_REMOVE: u32 = 2;
const CLOSE_REMOVE: u32 = 1;
const MAX_LINES: usize = 200;

/// Fetch the 4h bars that end in `(from, to]`.
pub struct Req {
    pub symbol: String,
    pub ticker_id: u64,
    pub from: i64,
    pub to: i64,
}

/// One line: its price, the start of the 4h bar that set it, and whether price has traded
/// above it since (drawn grey).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub price: f64,
    pub at: i64,
    pub taken: bool,
}

/// End times of the 4h bars of the weekdays in `(from, to]` (holidays included; asking for
/// one returns the bar before it, which the caller drops as a duplicate).
pub fn bar_ends(from: i64, to: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let (Some(a), Some(b)) = (New_York.timestamp_opt(from, 0).single(), New_York.timestamp_opt(to, 0).single()) else {
        return out;
    };
    let mut d = a.date_naive();
    while d <= b.date_naive() {
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            for (h, m) in BAR_ENDS {
                let t = d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default());
                if let Some(ts) = New_York.from_local_datetime(&t).single().map(|t| t.timestamp()) {
                    if ts > from && ts <= to {
                        out.push(ts);
                    }
                }
            }
        }
        d += Duration::days(1);
    }
    out
}

/// A symbol's 4h bars (oldest first), the bar ends they cover, `(from, through]`, and the
/// swing lines drawn from them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Data {
    pub bars: Vec<Candle>,
    /// Covered bar ends; None before the first batch.
    pub cover: Option<(i64, i64)>,
    /// No older bars exist (a batch further back came back empty: e.g. a recent listing).
    pub oldest: bool,
    pub lines: Vec<Level>,
}

impl Data {
    /// The next bar ends to fetch at `now`, `(from, to]`: newer bars first (oldest of them
    /// first, so the cover stays one piece), then the year back, newest first.
    pub fn next_range(&self, now: i64) -> Option<(i64, i64)> {
        let to = now - SETTLE_SECS;
        let lower = to - LOOKBACK_SECS;
        // the newest `BATCH` ends of `(lower, top]`
        let back_from = |top: i64| {
            let ends = bar_ends(lower, top);
            let i = ends.len().checked_sub(1)?.saturating_sub(BATCH - 1);
            Some((if i == 0 { lower } else { ends[i] - 1 }, top))
        };
        let Some((from, through)) = self.cover else { return back_from(to) };
        let newer = bar_ends(through, to);
        if !newer.is_empty() {
            return Some((through, newer[BATCH.min(newer.len()) - 1]));
        }
        if self.oldest {
            return None;
        }
        back_from(from)
    }

    /// Whether the whole year is fetched (only newer bars can still be missing).
    pub fn complete(&self, now: i64) -> bool {
        let lower = now - SETTLE_SECS - LOOKBACK_SECS;
        self.cover.is_some_and(|(from, _)| self.oldest || bar_ends(lower, from).is_empty())
    }

    /// Share of the year covered, 0-100.
    pub fn percent(&self, now: i64) -> u32 {
        let Some((from, through)) = self.cover else { return 0 };
        if self.complete(now) {
            return 100;
        }
        ((through - from) as f64 / LOOKBACK_SECS as f64 * 100.0).clamp(0.0, 99.0) as u32
    }

    /// Folds in the bars fetched for `(from, to]` and redraws the lines. False when nothing
    /// came back for newer bars (a failed request, tried again later); an empty batch
    /// further back means there are no older bars.
    pub fn apply(&mut self, from: i64, to: i64, bars: &[Candle], now: i64) -> bool {
        if bars.is_empty() {
            match self.cover {
                Some((f, _)) if to <= f => {
                    self.oldest = true;
                    return true;
                }
                _ => return false,
            }
        }
        history::merge(&mut self.bars, bars);
        self.cover = Some(self.cover.map_or((from, to), |(f, t)| (f.min(from), t.max(to))));
        // keep a year
        let lower = now - SETTLE_SECS - LOOKBACK_SECS;
        self.bars.retain(|c| c.start >= lower - 4 * 3600);
        self.lines = swing_levels(&self.bars, now);
        true
    }

    /// Draws the lines again at `now` (a new day moves the window whose bars never grey out
    /// or remove a line).
    pub fn redraw(&mut self, now: i64) {
        self.lines = swing_levels(&self.bars, now);
    }
}

/// Cache file of `symbol` in `dir` (symbols like "BRK B" are made file-safe).
fn cache_path(dir: &Path, symbol: &str) -> PathBuf {
    let safe: String = symbol.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    dir.join(format!("{safe}.csv"))
}

/// Writes `data` as `from,through,oldest` then one `start,open,high,low,close,volume` line per bar.
pub fn save(dir: &Path, symbol: &str, data: &Data) -> io::Result<()> {
    let Some((from, through)) = data.cover else { return Ok(()) };
    fs::create_dir_all(dir)?;
    let mut s = format!("{from},{through},{}\n", u8::from(data.oldest));
    for c in &data.bars {
        s.push_str(&format!("{},{},{},{},{},{}\n", c.start, c.open, c.high, c.low, c.close, c.volume));
    }
    let path = cache_path(dir, symbol);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, s)?;
    fs::rename(tmp, path)
}

/// The cached bars of `symbol`, with their lines drawn at `now`.
pub fn load(dir: &Path, symbol: &str, now: i64) -> Option<Data> {
    let s = fs::read_to_string(cache_path(dir, symbol)).ok()?;
    let mut lines = s.lines();
    let head: Vec<i64> = lines.next()?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let [from, through, oldest] = head[..] else { return None };
    let bars: Vec<Candle> = lines
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let n = |i: usize| f.get(i)?.trim().parse::<f64>().ok();
            Some(Candle { start: f.first()?.trim().parse().ok()?, open: n(1)?, high: n(2)?, low: n(3)?, close: n(4)?, volume: f.get(5)?.trim().parse().ok()? })
        })
        .collect();
    let lines = swing_levels(&bars, now);
    Some(Data { bars, cover: Some((from, through)), oldest: oldest == 1, lines })
}

/// Deletes cache files untouched for `CACHE_MAX_AGE`. Returns how many were deleted.
pub fn prune(dir: &Path) -> io::Result<usize> {
    let now = std::time::SystemTime::now();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for e in fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "csv") {
            if let Ok(m) = e.metadata() {
                files.push((m.modified().unwrap_or(now), p));
            }
        }
    }
    let mut n = 0;
    for (modified, p) in files {
        let old = now.duration_since(modified).is_ok_and(|a| a > CACHE_MAX_AGE);
        if old && fs::remove_file(&p).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// The 4h bars ending in `(req.from, req.to]`, oldest first, fetched on `THREADS` threads.
/// None when any request failed or a 403 pause started meanwhile: the batch would have
/// holes, so it is dropped and asked again.
pub fn fetch(req: &Req) -> Option<Vec<Candle>> {
    let ends = bar_ends(req.from, req.to);
    let chunk = ends.len().div_ceil(THREADS).max(1);
    let id = req.ticker_id;
    let answers: Vec<Result<Option<Candle>, ()>> = std::thread::scope(|s| {
        let parts: Vec<_> = ends
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    let agent = crate::scraper::agent();
                    let mut v = Vec::new();
                    for &ts in part {
                        let a = history::try_bar_at(&agent, id, Tf::H4, ts);
                        let failed = a.is_err();
                        v.push(a);
                        if failed {
                            break;
                        }
                    }
                    v
                })
            })
            .collect();
        parts.into_iter().flat_map(|h| h.join().unwrap_or_else(|_| vec![Err(())])).collect()
    });
    if crate::limiter::paused().is_some() {
        return None;
    }
    gather(answers)
}

/// The bars of a batch's answers, oldest first without repeats; None when any failed.
fn gather(answers: Vec<Result<Option<Candle>, ()>>) -> Option<Vec<Candle>> {
    let mut out: Vec<Candle> = answers.into_iter().collect::<Result<Vec<_>, ()>>().ok()?.into_iter().flatten().collect();
    out.sort_by_key(|c| c.start);
    out.dedup_by_key(|c| c.start);
    Some(out)
}

fn ny_minute(ts: i64) -> Option<(chrono::DateTime<chrono_tz::Tz>, u32)> {
    let t = New_York.timestamp_opt(ts, 0).single()?;
    let m = t.hour() * 60 + t.minute();
    Some((t, m))
}

/// Start of today's window: the latest 16:00 New York market close at or before `now`
/// that has a market bar. Its candles never grey out or remove a line.
fn window_start(bars: &[Candle], now: i64) -> Option<i64> {
    bars.iter().rev().find_map(|c| {
        let (t, m) = ny_minute(c.start)?;
        if !(570..960).contains(&m) {
            return None;
        }
        let close = New_York.from_local_datetime(&t.date_naive().and_hms_opt(16, 0, 0)?).single()?.timestamp();
        (close <= now).then_some(close)
    })
}

/// Swing points of `vals` (price, time): a middle value strictly beyond the one before and
/// not behind the one after (`high` = highs, else lows).
fn swings(vals: &[(f64, i64)], high: bool) -> Vec<(f64, i64)> {
    vals.windows(3)
        .filter(|w| if high { w[1].0 > w[0].0 && w[1].0 >= w[2].0 } else { w[1].0 < w[0].0 && w[1].0 <= w[2].0 })
        .map(|w| w[1])
        .collect()
}

/// Next tier of highs: a high strictly above the highs on both sides; a repeat of the last
/// one's price is skipped.
fn tier(highs: &[(f64, i64)]) -> Vec<(f64, i64)> {
    let mut out: Vec<(f64, i64)> = Vec::new();
    for w in highs.windows(3) {
        if w[1].0 > w[0].0 && w[1].0 > w[2].0 && out.last().is_none_or(|l| l.0 != w[1].0) {
            out.push(w[1]);
        }
    }
    out
}

/// Taken / removed state of a line at `p` set by the bar starting at `at`, from the bars
/// after it and before `today`. Returns `(taken, removed)`.
fn evaluate(bars: &[Candle], p: f64, at: i64, today: Option<i64>) -> (bool, bool) {
    let tick = if p < 1.0 { 0.0001 } else { 0.01 } * 0.99;
    let (mut taken, mut above) = (false, false);
    let (mut wicks, mut trips) = (0, 0);
    for c in bars.iter().filter(|c| c.start > at && today.is_none_or(|t| c.start < t)) {
        if c.high - p >= tick {
            taken = true;
            if c.close <= p {
                wicks += 1;
            }
        }
        if !above && c.close > p {
            above = true;
        } else if above && c.close < p {
            above = false;
            trips += 1;
        }
        if wicks >= WICK_REMOVE || trips >= CLOSE_REMOVE {
            return (taken, true);
        }
    }
    (taken, false)
}

/// The swing lines to draw from finished 4h `bars` (oldest first) at time `now`.
///
/// Highs and lows are classed as short-term (a bar beyond both neighbours), intermediate
/// (a short-term high above the short-term highs on both sides) and long-term (the same one
/// tier up). Lines: the latest long-term high (LTH), the `OLDER_LTH` before it, and for each
/// short-term low since the LTH the highest short-term high up to the next short-term low,
/// ignoring highs above the highest confirmed top since the LTH. A line goes after
/// `WICK_REMOVE` wicks through or `CLOSE_REMOVE` closes through; one that only traded above
/// it is "taken". Lines are kept newest first, at most `MAX_LINES`, at any distance from the
/// price.
pub fn swing_levels(bars: &[Candle], now: i64) -> Vec<Level> {
    let highs: Vec<(f64, i64)> = bars.iter().map(|c| (c.high, c.start)).collect();
    let lows: Vec<(f64, i64)> = bars.iter().map(|c| (c.low, c.start)).collect();
    let sth = swings(&highs, true);
    let stl: Vec<i64> = swings(&lows, false).into_iter().map(|l| l.1).collect();
    let ith = tier(&sth);
    let lth = tier(&ith);
    let Some(&(pl, tl)) = lth.last() else { return Vec::new() };
    let cap = ith.iter().filter(|h| h.1 > tl).map(|h| h.0).fold(pl, f64::max);

    let mut cands = vec![(pl, tl)];
    cands.extend(lth.iter().rev().skip(1).take(OLDER_LTH));
    let mut last_t = None;
    for (j, &t0) in stl.iter().enumerate().filter(|(_, t)| **t >= tl) {
        let t1 = stl.get(j + 1).copied().unwrap_or(i64::MAX);
        // oldest first with >=, so of equal highs the newest wins
        let best = sth
            .iter()
            .filter(|h| h.1 >= t0 && h.1 <= t1 && h.1 > tl && h.0 <= cap)
            .fold(None, |b: Option<(f64, i64)>, &h| if b.is_some_and(|b| h.0 < b.0) { b } else { Some(h) });
        if let Some(b) = best.filter(|b| Some(b.1) != last_t) {
            cands.push(b);
            last_t = Some(b.1);
        }
    }
    cands.sort_by_key(|c| std::cmp::Reverse(c.1));

    let today = window_start(bars, now);
    let mut out = Vec::new();
    for (price, at) in cands {
        if out.len() >= MAX_LINES {
            break;
        }
        let (taken, removed) = evaluate(bars, price, at, today);
        if !removed {
            out.push(Level { price, at, taken });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        New_York.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp()
    }

    fn bar(start: i64, high: f64, low: f64, close: f64) -> Candle {
        Candle { start, open: close, high, low, close, volume: 1 }
    }

    #[test]
    fn bar_ends_are_the_weekday_session_ends() {
        // Fri 13 Nov 2026 12:00 to Mon 16 Nov 09:30 (New York)
        let ends = bar_ends(ny(2026, 11, 13, 12, 0), ny(2026, 11, 16, 9, 30));
        let want = [ny(2026, 11, 13, 13, 30), ny(2026, 11, 13, 16, 0), ny(2026, 11, 13, 20, 0), ny(2026, 11, 16, 8, 0), ny(2026, 11, 16, 9, 30)];
        assert_eq!(ends, want);
    }

    #[test]
    fn batches_go_newest_first_then_keep_up() {
        let now = ny(2026, 11, 17, 10, 0);
        let mut d = Data::default();
        // first batch: the newest BATCH bar ends
        let (f, t) = d.next_range(now).unwrap();
        assert_eq!(t, now - SETTLE_SECS);
        assert_eq!(bar_ends(f, t).len(), BATCH);
        let bars: Vec<Candle> = bar_ends(f, t).iter().map(|&e| bar(e - 3600, 2.0, 1.0, 1.5)).collect();
        assert!(d.apply(f, t, &bars, now));
        assert_eq!(d.cover, Some((f, t)));
        assert!(!d.complete(now));
        // next: the BATCH before it, ending where the cover starts
        let (f2, t2) = d.next_range(now).unwrap();
        assert_eq!((t2, bar_ends(f2, t2).len()), (f, BATCH));
        // an empty batch further back: no older bars exist, the year counts as complete
        assert!(d.apply(f2, t2, &[], now));
        assert!(d.complete(now) && d.oldest);
        assert_eq!(d.next_range(now), None);
        // a new bar ends: only it is asked for
        let later = ny(2026, 11, 17, 13, 31) + SETTLE_SECS;
        assert_eq!(d.next_range(later), Some((t, ny(2026, 11, 17, 13, 30))));
        // nothing back for newer bars = a failed request
        assert!(!d.apply(t, ny(2026, 11, 17, 13, 30), &[], later));
    }

    #[test]
    fn cache_round_trip_and_prune() {
        let dir = std::env::temp_dir().join(format!("vs-levels-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut d = Data { cover: Some((10, 20)), oldest: true, ..Data::default() };
        d.bars = vec![bar(14_400, 2.5, 1.25, 2.0)];
        save(&dir, "BRK B", &d).unwrap();
        assert!(dir.join("BRK_B.csv").exists());
        let back = load(&dir, "BRK B", 0).unwrap();
        assert_eq!((back.bars, back.cover, back.oldest), (d.bars, d.cover, true));
        assert!(load(&dir, "NONE", 0).is_none());
        // fresh and small: kept
        assert_eq!(prune(&dir).unwrap(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn swing_and_tier_rules() {
        let v = |xs: &[f64]| xs.iter().enumerate().map(|(i, &p)| (p, i as i64)).collect::<Vec<_>>();
        // strict before, not behind after: the first bar of a flat top counts
        assert_eq!(swings(&v(&[1.0, 3.0, 3.0, 2.0]), true), vec![(3.0, 1)]);
        assert_eq!(swings(&v(&[3.0, 1.0, 1.0, 2.0]), false), vec![(1.0, 1)]);
        // tier: strictly above both sides, a repeated price is skipped
        assert_eq!(tier(&v(&[1.0, 5.0, 2.0, 5.0, 1.0])), vec![(5.0, 1)]);
    }

    #[test]
    fn removal_counts_wicks_and_close_throughs() {
        let b = [bar(1, 10.5, 9.0, 9.5), bar(2, 10.4, 9.0, 9.8)];
        assert_eq!(evaluate(&b, 10.0, 0, None), (true, true)); // two wicks through
        assert_eq!(evaluate(&b[..1], 10.0, 0, None), (true, false)); // one: taken only
        let c = [bar(1, 11.0, 9.0, 10.5), bar(2, 10.6, 9.0, 9.9)];
        assert_eq!(evaluate(&c, 10.0, 0, None), (true, true)); // close above, then below
        assert_eq!(evaluate(&c, 10.0, 0, Some(2)), (true, false)); // today's bar ignored
    }

    #[test]
    fn a_batch_with_a_failed_request_is_dropped() {
        let b = |s| Ok(Some(bar(s, 2.0, 1.0, 1.5)));
        // a holiday's end repeats the bar before it; no bar is fine
        assert_eq!(gather(vec![b(14_400), Ok(None), b(0), b(14_400)]).map(|v| v.len()), Some(2));
        assert_eq!(gather(vec![b(0), Err(()), b(14_400)]), None);
        assert_eq!(gather(vec![]), Some(vec![]));
    }

    #[test]
    fn lines_near_the_price_are_kept() {
        // the long-term high at 20 with the last close just below it (no distance window)
        let hs = [10.0, 12.0, 11.0, 14.0, 12.0, 13.0, 11.0, 20.0, 12.0, 15.0, 12.0, 13.0, 11.0, 17.0, 12.0, 14.0, 12.0, 15.5, 11.0, 13.5, 10.0, 18.0, 9.0];
        let mut bars: Vec<Candle> = hs.iter().enumerate().map(|(i, &h)| bar(i as i64 * 14_400, h, h - 1.0, h - 0.5)).collect();
        bars.push(bar(23 * 14_400, 19.8, 19.0, 19.5));
        let lv = swing_levels(&bars, i64::MAX);
        assert!(lv.iter().any(|l| l.price == 20.0 && !l.taken), "{lv:?}");
    }

    #[test]
    fn picks_lth_and_the_highest_high_per_swing_low_segment() {
        // highs build a long-term high at 20, then a pullback with two segments
        let hs = [10.0, 12.0, 11.0, 14.0, 12.0, 13.0, 11.0, 20.0, 12.0, 15.0, 12.0, 13.0, 11.0, 17.0, 12.0, 14.0, 12.0, 15.5, 11.0, 13.5, 10.0, 18.0, 9.0];
        let bars: Vec<Candle> = hs.iter().enumerate().map(|(i, &h)| bar(i as i64 * 14_400, h, h - 1.0, h - 0.5)).collect();
        let lv = swing_levels(&bars, i64::MAX);
        assert!(lv.iter().any(|l| l.price == 20.0), "{lv:?}");
        // newest first
        assert!(lv.windows(2).all(|w| w[0].at >= w[1].at));
    }
}
