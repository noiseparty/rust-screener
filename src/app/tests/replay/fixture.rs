//! Replay fixture parser (`tests/fixtures/*.replay`).
//!
//! The probe CSV (`MarketGUI-cli.exe --probe`) records stream messages and connection events,
//! but of the REST gainers list only a `list` line with counts, so a fixture is its own small
//! line format. Probe CSV rows can be pasted in as they are; they become quotes and stream
//! events. One item per line; blank lines and lines starting with `#` are skipped.
//!
//! ```text
//! start 2026-01-12T15:00:00Z      optional, before any event: the wall time of t=0 (default
//!                                 2026-01-12 10:00 New York); probe rows are timed against it
//! config min_change_percent=5     config.rs field overrides (JSON values), before any event
//! t=<time> rest SYM,id,price,change_ratio,volume,mcap
//!                                 one REST list row; the rows sharing a t are the whole list
//!                                 the server returns from then on (every fetch sees it)
//! t=<time> rest-error <text>      the server fails fetches with <text> from then on
//! t=<time> quote id,trade_ms,seq,close,p_price,change_ratio,volume,mcap
//!                                 one stream quote; empty field = absent; empty trade_ms =
//!                                 start + t; quotes sharing a t arrive as one batch
//! t=<time> stream up|down         stream (re)connected / disconnected
//! <probe CSV row>                 kind `msg` -> quote at t = recv_utc - start; `connect` ->
//!                                 stream up; `disconnect` -> stream down; other kinds and the
//!                                 header line are skipped
//! ```
//!
//! `<time>` is milliseconds after t=0 (`t=1500`) or seconds with an `s` (`t=1.5s`, `t=610s`).
//! Lines must be in time order.

use crate::config::Config;
use crate::session::Session;
use chrono::{DateTime, Utc};

/// One fixture event.
#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    Rest(RestRow),
    RestError(String),
    Quote(Quote),
    StreamUp,
    StreamDown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestRow {
    pub symbol: String,
    pub id: u64,
    pub price: f64,
    pub change_ratio: f64,
    pub volume: i64,
    pub mcap: f64,
}

/// A stream quote. `trade_ms` is relative to `start` (the harness places it on the real clock).
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub id: u64,
    pub trade_ms: i64,
    pub seq: u64,
    pub close: Option<f64>,
    pub p_price: Option<f64>,
    pub change_ratio: Option<f64>,
    pub volume: Option<u64>,
    pub mcap: Option<f64>,
}

#[derive(Debug)]
pub struct Fixture {
    pub cfg: Config,
    /// (ms after t=0, event), in time order.
    pub events: Vec<(u64, Ev)>,
}

/// Default `start`: 2026-01-12 15:00 UTC = 10:00 New York, regular session.
const DEFAULT_START: &str = "2026-01-12T15:00:00Z";

/// Reads `tests/fixtures/<name>`.
pub fn load(name: &str) -> Fixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

pub fn parse(text: &str) -> Result<Fixture, String> {
    let mut start = parse_utc(DEFAULT_START)?;
    let mut cfg = serde_json::to_value(Config::default()).map_err(|e| e.to_string())?;
    let mut events: Vec<(u64, Ev)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        let err = |e: String| format!("line {}: {e}: {line}", n + 1);
        if line.is_empty() || line.starts_with('#') || line.starts_with("recv_utc,") {
            continue;
        }
        let parsed = if let Some(rest) = line.strip_prefix("start ") {
            if !events.is_empty() {
                return Err(err("start after events".into()));
            }
            start = parse_utc(rest.trim()).map_err(err)?;
            None
        } else if let Some(rest) = line.strip_prefix("config ") {
            if !events.is_empty() {
                return Err(err("config after events".into()));
            }
            for kv in rest.split_whitespace() {
                let (k, v) = kv.split_once('=').ok_or_else(|| err("expected key=value".into()))?;
                if cfg.get(k).is_none() {
                    return Err(err(format!("unknown config key {k}")));
                }
                cfg[k] = serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.into()));
            }
            None
        } else if let Some(rest) = line.strip_prefix("t=") {
            Some(event_line(rest, start).map_err(err)?)
        } else {
            probe_row(line, start).map_err(err)?
        };
        if let Some((t, ev)) = parsed {
            if events.last().is_some_and(|(last, _)| t < *last) {
                return Err(err("out of time order".into()));
            }
            events.push((t, ev));
        }
    }
    let cfg = serde_json::from_value(cfg).map_err(|e| format!("config: {e}"))?;
    Ok(Fixture { cfg, events })
}

fn parse_utc(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc)).map_err(|e| format!("bad time {s}: {e}"))
}

/// `1500` (ms) or `1.5s`.
fn parse_t(s: &str) -> Result<u64, String> {
    let ms = match s.strip_suffix('s') {
        Some(secs) => secs.parse::<f64>().map(|v| (v * 1000.0).round()),
        None => s.parse::<f64>(),
    };
    ms.ok().filter(|v| *v >= 0.0).map(|v| v as u64).ok_or_else(|| format!("bad time {s}"))
}

/// A number as the stream and REST send it (thousands separators allowed); empty = none.
fn num(s: &str) -> Result<Option<f64>, String> {
    let s = s.trim().replace(',', "");
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<f64>().map(Some).map_err(|_| format!("bad number {s}"))
}

fn need(s: &str, what: &str) -> Result<f64, String> {
    num(s)?.ok_or_else(|| format!("missing {what}"))
}

fn event_line(line: &str, start: DateTime<Utc>) -> Result<(u64, Ev), String> {
    let (t, rest) = line.split_once(' ').ok_or("expected t=<time> <event>")?;
    let t = parse_t(t)?;
    let (kind, args) = rest.trim().split_once(' ').unwrap_or((rest.trim(), ""));
    let f: Vec<&str> = args.split(',').map(str::trim).collect();
    let ev = match kind {
        "rest" => {
            if f.len() != 6 {
                return Err("rest wants SYM,id,price,change_ratio,volume,mcap".into());
            }
            Ev::Rest(RestRow {
                symbol: f[0].to_string(),
                id: need(f[1], "id")? as u64,
                price: need(f[2], "price")?,
                change_ratio: need(f[3], "change_ratio")?,
                volume: need(f[4], "volume")? as i64,
                mcap: need(f[5], "mcap")?,
            })
        }
        "rest-error" => Ev::RestError(args.trim().to_string()),
        "quote" => {
            if f.len() != 8 {
                return Err("quote wants id,trade_ms,seq,close,p_price,change_ratio,volume,mcap".into());
            }
            let trade_ms = num(f[1])?.map_or(t as i64, |v| v as i64);
            let q = Quote {
                id: need(f[0], "id")? as u64,
                trade_ms,
                seq: num(f[2])?.unwrap_or(0.0) as u64,
                close: num(f[3])?,
                p_price: num(f[4])?,
                change_ratio: num(f[5])?,
                volume: num(f[6])?.map(|v| v.round() as u64),
                mcap: num(f[7])?,
            };
            Ev::Quote(resolve_price(q, start))
        }
        "stream" => match args.trim() {
            "up" => Ev::StreamUp,
            "down" => Ev::StreamDown,
            other => return Err(format!("stream wants up|down, got {other}")),
        },
        other => return Err(format!("unknown event {other}")),
    };
    Ok((t, ev))
}

/// The app picks pPrice over close in extended hours by the real clock (`Session::now()`); a
/// replay must not depend on when it runs, so the price is chosen here by the fixture's time.
fn resolve_price(mut q: Quote, start: DateTime<Utc>) -> Quote {
    let at = start + chrono::Duration::milliseconds(q.trade_ms);
    let ext = matches!(Session::at(at), Session::Pre | Session::After);
    if ext {
        if let Some(p) = q.p_price.filter(|p| *p > 0.0) {
            q.close = Some(p);
        }
    }
    q.p_price = None;
    q
}

/// A probe CSV row (`probe::COLUMNS`): `msg` -> quote, `connect` / `disconnect` -> stream events.
fn probe_row(line: &str, start: DateTime<Utc>) -> Result<Option<(u64, Ev)>, String> {
    use crate::probe::COLUMNS;
    let cells = split_csv(line);
    if cells.len() != COLUMNS.len() {
        return Err(format!("not an event and not a probe row ({} cells, want {})", cells.len(), COLUMNS.len()));
    }
    let get = |name: &str| cells[COLUMNS.iter().position(|c| *c == name).expect("probe column")].as_str();
    let recv = parse_utc(get("recv_utc"))?;
    let t = (recv - start).num_milliseconds();
    if t < 0 {
        return Err("probe row before start".into());
    }
    let t = t as u64;
    let ev = match get("kind") {
        "connect" => Ev::StreamUp,
        "disconnect" => Ev::StreamDown,
        "msg" => {
            let trade_ms = num(get("trade_stamp_ms"))?.map_or(t as i64, |ms| ms as i64 - start.timestamp_millis());
            let q = Quote {
                id: need(get("ticker_id"), "ticker_id")? as u64,
                trade_ms,
                seq: num(get("trd_seq"))?.unwrap_or(0.0) as u64,
                close: num(get("close"))?,
                p_price: num(get("p_price"))?,
                change_ratio: num(get("change_ratio"))?,
                volume: num(get("volume"))?.filter(|v| *v >= 0.0).map(|v| v.round() as u64),
                mcap: num(get("market_value"))?,
            };
            Ev::Quote(resolve_price(q, start))
        }
        _ => return Ok(None),
    };
    Ok(Some((t, ev)))
}

/// Splits one CSV line as `probe::csv_line` writes it (quoted fields, `""` for a quote).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().expect("non-empty").push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(String::new()),
            c => out.last_mut().expect("non-empty").push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{csv_line, event_row, msg_row};
    use crate::stream::RawMsg;
    use std::collections::BTreeMap;

    #[test]
    fn parses_events_config_and_times() {
        let f = parse(
            "config min_change_percent=5 recompute_ms=500\nt=0 stream up\nt=1.5s rest ABC,7,2.5,0.25,1000,5e7\n\
             t=2000 quote 7,,3,2.6,,,1200,\nt=3s rest-error HTTP 503\n",
        )
        .unwrap();
        assert_eq!(f.cfg.min_change_percent, 5.0);
        assert_eq!(f.cfg.recompute_ms, 500);
        assert_eq!(f.events[0], (0, Ev::StreamUp));
        assert!(matches!(&f.events[1], (1500, Ev::Rest(r)) if r.symbol == "ABC" && r.id == 7 && r.volume == 1000));
        let (t, Ev::Quote(q)) = &f.events[2] else { panic!("quote") };
        assert_eq!((*t, q.trade_ms, q.seq, q.close, q.volume, q.mcap), (2000, 2000, 3, Some(2.6), Some(1200), None));
        assert_eq!(f.events[3], (3000, Ev::RestError("HTTP 503".into())));
        assert!(parse("t=5s stream up\nt=4s stream down\n").unwrap_err().contains("time order"));
        assert!(parse("config no_such_key=1\n").is_err());
    }

    #[test]
    fn extended_hours_price_is_taken_from_p_price_by_fixture_time() {
        // 08:00 New York: pre-market, pPrice wins; 10:00: regular, close wins
        let pre = parse("start 2026-01-12T13:00:00Z\nt=0 quote 1,,1,2.0,2.4,,100,\n").unwrap();
        let reg = parse("t=0 quote 1,,1,2.0,2.4,,100,\n").unwrap();
        for (f, px) in [(pre, 2.4), (reg, 2.0)] {
            let Ev::Quote(q) = &f.events[0].1 else { panic!("quote") };
            assert_eq!((q.close, q.p_price), (Some(px), None));
        }
    }

    #[test]
    fn probe_rows_become_quotes_and_stream_events() {
        let start = parse_utc("2026-01-12T15:00:00Z").unwrap();
        let at = start + chrono::Duration::milliseconds(2500);
        let map = |p: &[(u32, &str)]| p.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>();
        let stamp = (start.timestamp_millis() + 2400).to_string();
        let raw = RawMsg {
            id: 42,
            base: map(&[(2, "9"), (5, &stamp)]),
            sale: map(&[(1, "3.10"), (3, "0.12"), (7, "12,500,000")]),
            stats: map(&[(3, "1,234,567")]),
            modules: vec![],
        };
        let text = format!(
            "start 2026-01-12T15:00:00Z\n{}\n{}\n{}\n{}\n",
            csv_line(&crate::probe::COLUMNS),
            csv_line(&event_row(start, "connect", "connection #1")),
            csv_line(&msg_row(at, &raw, "ABC", Some(1))),
            csv_line(&event_row(at, "list", "rows=1")),
        );
        let f = parse(&text).unwrap();
        assert_eq!(f.events.len(), 2);
        assert_eq!(f.events[0], (0, Ev::StreamUp));
        let (t, Ev::Quote(q)) = &f.events[1] else { panic!("quote") };
        assert_eq!((*t, q.id, q.trade_ms, q.seq), (2500, 42, 2400, 9));
        assert_eq!((q.close, q.change_ratio, q.volume, q.mcap), (Some(3.10), Some(0.12), Some(1_234_567), Some(12_500_000.0)));
    }
}
