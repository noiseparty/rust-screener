//! Probe mode (`MarketGUI-cli.exe --probe`): a headless recorder for checking stream behaviour
//! across session changes. Writes every raw message and connection event to
//! `probe\probe-YYYY-MM-DD.csv` and prints a short status line now and then. Stops on Ctrl+C.

use crate::app::{Msg, LIST_REFRESH};
use crate::paths::Paths;
use crate::scraper::{self, Fetched};
use crate::session::Session;
use crate::stream::{self, Cmd, Event, RawMsg, Stamped, StreamStatus};
use chrono::{DateTime, Local, NaiveDate, Utc};
use chrono_tz::America::New_York;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

/// The probe asks for more rows than the app's page of 100, so the subscription cap can be seen.
const PAGE_SIZE: usize = 200;
const STATUS_EVERY: Duration = Duration::from_secs(30);
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(200);

pub const COLUMNS: [&str; 38] = [
    "recv_utc",
    "recv_ny",
    "recv_session",
    "kind",
    "ticker_id",
    "symbol",
    "list_pos",
    "trans_id",
    "trd_seq",
    "status",
    "trade_stamp_ms",
    "trade_utc",
    "trade_session",
    "close",
    "change",
    "change_ratio",
    "p_price",
    "p_change",
    "p_ch_ratio",
    "market_value",
    "hlt_rsn",
    "sale_inited",
    "trade_time",
    "last_price_vol",
    "volume",
    "deal_amount",
    "open",
    "high",
    "low",
    "wk52_high",
    "wk52_low",
    "stats_inited",
    "topic",
    "pub_id",
    "base_inited",
    "modules",
    "extra",
    "detail",
];

const BASE_KNOWN: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 9];
const SALE_KNOWN: [u32; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
const STATS_KNOWN: [u32; 13] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 29];

/// File name for a probe run started on `date`.
pub fn file_name(date: NaiveDate) -> String {
    format!("probe-{}.csv", date.format("%Y-%m-%d"))
}

/// One CSV line (no newline). Fields containing a comma, quote or line break are quoted.
pub fn csv_line<S: AsRef<str>>(fields: &[S]) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let f = f.as_ref();
        if f.contains([',', '"', '\n', '\r']) {
            out.push('"');
            out.push_str(&f.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(f);
        }
    }
    out
}

fn blank_row(at: DateTime<Utc>, kind: &str) -> Vec<String> {
    let mut r = vec![String::new(); COLUMNS.len()];
    r[0] = at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    r[1] = at.with_timezone(&New_York).format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    r[2] = Session::at(at).code().to_string();
    r[3] = kind.to_string();
    r
}

fn col(name: &str) -> usize {
    COLUMNS.iter().position(|c| *c == name).expect("known column")
}

/// "b8=1;s52=3" for fields outside the documented layout, prefixed b/s/t for base/sale/stats.
fn extras(m: &RawMsg) -> String {
    let mut parts = Vec::new();
    for (tag, map, known) in [("b", &m.base, &BASE_KNOWN[..]), ("s", &m.sale, &SALE_KNOWN[..]), ("t", &m.stats, &STATS_KNOWN[..])] {
        for (n, v) in map {
            if !known.contains(n) {
                parts.push(format!("{tag}{n}={v}"));
            }
        }
    }
    parts.join(";")
}

/// CSV row for one raw stream message. `at` is the local receive time.
pub fn msg_row(at: DateTime<Utc>, m: &RawMsg, symbol: &str, list_pos: Option<usize>) -> Vec<String> {
    let mut r = blank_row(at, "msg");
    let mut set = |name: &str, v: String| r[col(name)] = v;
    let get = |map: &std::collections::BTreeMap<u32, String>, n: u32| map.get(&n).cloned().unwrap_or_default();
    set("ticker_id", m.id.to_string());
    set("symbol", symbol.to_string());
    set("list_pos", list_pos.map(|p| p.to_string()).unwrap_or_default());

    set("trans_id", get(&m.base, 1));
    set("trd_seq", get(&m.base, 2));
    set("status", get(&m.base, 4));
    let stamp = get(&m.base, 5);
    if let Some(t) = stamp.parse::<i64>().ok().and_then(DateTime::from_timestamp_millis) {
        set("trade_utc", t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        set("trade_session", Session::at(t).code().to_string());
    }
    set("trade_stamp_ms", stamp);
    set("topic", get(&m.base, 6));
    set("pub_id", get(&m.base, 7));
    set("base_inited", get(&m.base, 9));

    for (name, n) in [
        ("close", 1),
        ("change", 2),
        ("change_ratio", 3),
        ("p_price", 4),
        ("p_change", 5),
        ("p_ch_ratio", 6),
        ("market_value", 7),
        ("sale_inited", 8),
        ("hlt_rsn", 9),
    ] {
        set(name, get(&m.sale, n));
    }
    for (name, n) in [
        ("trade_time", 1),
        ("last_price_vol", 2),
        ("volume", 3),
        ("deal_amount", 4),
        ("open", 8),
        ("high", 9),
        ("low", 10),
        ("wk52_high", 11),
        ("wk52_low", 12),
        ("stats_inited", 29),
    ] {
        set(name, get(&m.stats, n));
    }
    set("modules", m.modules.iter().map(|(n, len)| format!("{n}:{len}")).collect::<Vec<_>>().join(";"));
    set("extra", extras(m));
    r
}

/// CSV row for a non-message record (`kind` is the event name).
pub fn event_row(at: DateTime<Utc>, kind: &str, detail: &str) -> Vec<String> {
    let mut r = blank_row(at, kind);
    r[col("detail")] = detail.to_string();
    r
}

/// Event name and detail for connection events. Raw messages are formatted by `msg_row` instead.
pub fn event_fields(e: &Event) -> Option<(&'static str, String)> {
    Some(match e {
        Event::Connected { n } => ("connect", format!("connection #{n}")),
        Event::Subscribe { id, count, total } => ("subscribe", format!("id={id} count={count} total={total}")),
        Event::Unsubscribe { id, count, total } => ("unsubscribe", format!("id={id} count={count} total={total}")),
        Event::SubAck { id, codes } => {
            let codes: Vec<String> = codes.iter().map(|c| format!("0x{c:02x}")).collect();
            ("suback", format!("id={id} codes={}", codes.join(" ")))
        }
        Event::Disconnected { reason } => ("disconnect", reason.clone()),
        Event::Undecodable { topic, len } => ("undecodable", format!("topic={topic} payload_len={len}")),
        Event::Raw(_) => return None,
    })
}

/// CSV writer: header first when the file is empty, one line per row.
pub struct Recorder<W: Write> {
    out: W,
    pub rows: u64,
}

impl<W: Write> Recorder<W> {
    /// `fresh` says whether the header still has to be written.
    pub fn new(mut out: W, fresh: bool) -> io::Result<Self> {
        if fresh {
            writeln!(out, "{}", csv_line(&COLUMNS))?;
        }
        Ok(Recorder { out, rows: 0 })
    }

    pub fn write(&mut self, row: &[String]) -> io::Result<()> {
        self.rows += 1;
        writeln!(self.out, "{}", csv_line(row))
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Opens `<dir>/probe-<date>.csv` for appending (a restart on the same day continues the file).
pub fn open_csv(dir: &Path, date: NaiveDate) -> io::Result<(Recorder<BufWriter<File>>, PathBuf)> {
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name(date));
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let fresh = file.metadata()?.len() == 0;
    Ok((Recorder::new(BufWriter::new(file), fresh)?, path))
}

fn stream_state(st: &StreamStatus, session: Session) -> String {
    if st.connected {
        "up".to_string()
    } else if let Some(n) = st.retry_in {
        format!("retry in {n}s")
    } else if session == Session::Closed {
        "idle (closed)".to_string()
    } else {
        "down".to_string()
    }
}

fn say(text: &str) {
    println!("{} {text}", Local::now().format("%H:%M:%S"));
}

/// Runs until Ctrl+C. `dir` overrides the app home's `probe\` folder.
pub fn run(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = match dir {
        Some(d) => d,
        None => {
            let p = Paths::from_env().map_err(|e| e.to_string())?;
            // move the old app home first, or creating `probe\` would block it
            if let Err(e) = p.migrate_home() {
                eprintln!("could not rename the old app home: {e}");
            }
            p.probe
        }
    };
    let (mut rec, path) = open_csv(&dir, Local::now().date_naive()).map_err(|e| format!("{}: {e}", dir.display()))?;

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (ev_tx, ev_rx) = mpsc::channel::<Stamped>();
    stream::spawn_with(msg_tx, cmd_rx, Some(ev_tx));

    let (list_tx, list_rx) = mpsc::channel::<Result<Fetched, String>>();
    thread::spawn(move || {
        let agent = scraper::agent();
        while list_tx.send(scraper::fetch_top_gainers(&agent, PAGE_SIZE)).is_ok() {
            thread::sleep(Duration::from_secs(LIST_REFRESH));
        }
    });

    let started = Utc::now();
    let _ = rec.write(&event_row(started, "start", &format!("probe v{} page_size={PAGE_SIZE} list_refresh_s={LIST_REFRESH}", env!("CARGO_PKG_VERSION"))));
    say(&format!("probe recording to {} (Ctrl+C to stop)", path.display()));

    let mut symbols: HashMap<u64, (String, usize)> = HashMap::new();
    let mut st = StreamStatus::default();
    let (mut msgs, mut bad) = (0u64, 0u64);
    let (mut last_flush, mut last_status) = (Instant::now(), Instant::now());
    let mut write_failed = false;

    while !stop.load(Ordering::SeqCst) {
        let first = ev_rx.recv_timeout(TICK).ok();
        for (at, ev) in first.into_iter().chain(ev_rx.try_iter()) {
            let row = match &ev {
                Event::Raw(m) => {
                    msgs += 1;
                    let (sym, pos) = symbols.get(&m.id).map(|(s, p)| (s.as_str(), Some(*p))).unwrap_or(("", None));
                    msg_row(at, m, sym, pos)
                }
                e => {
                    let (kind, detail) = event_fields(e).expect("non-raw event");
                    if matches!(e, Event::Undecodable { .. }) {
                        bad += 1;
                    }
                    if !matches!(e, Event::Undecodable { .. }) {
                        say(&format!("{kind}: {detail}"));
                    }
                    event_row(at, kind, &detail)
                }
            };
            if let Err(e) = rec.write(&row) {
                if !write_failed {
                    write_failed = true;
                    eprintln!("write failed: {e}");
                }
            }
        }
        while let Ok(m) = msg_rx.try_recv() {
            if let Msg::Stream(s) = m {
                st = s; // quotes are not needed here: raw messages come through the event channel
            }
        }
        while let Ok(r) = list_rx.try_recv() {
            let row = match r {
                Ok(f) => {
                    symbols = f.tickers.iter().enumerate().map(|(i, t)| (t.ticker_id, (t.symbol.clone(), i + 1))).collect();
                    let ids: Vec<u64> = f.tickers.iter().map(|t| t.ticker_id).filter(|&i| i != 0).collect();
                    let detail = format!("rows={} ids={} group={} session={}", f.tickers.len(), ids.len(), f.group, f.session.code());
                    let _ = cmd_tx.send(Cmd::SetTickers(ids));
                    event_row(Utc::now(), "list", &detail)
                }
                Err(e) => event_row(Utc::now(), "list_error", &e),
            };
            let _ = rec.write(&row);
        }

        let now = Instant::now();
        if now.duration_since(last_flush) >= FLUSH_EVERY {
            last_flush = now;
            if let Err(e) = rec.flush() {
                if !write_failed {
                    write_failed = true;
                    eprintln!("flush failed: {e}");
                }
            }
        }
        if now.duration_since(last_status) >= STATUS_EVERY {
            last_status = now;
            let session = Session::now();
            say(&format!(
                "{} stream={} subs={} {:.1} msg/s | msgs={msgs} bad={bad} rows={} connections={}",
                session.code(),
                stream_state(&st, session),
                st.subs,
                st.msgs_per_sec,
                rec.rows,
                st.connections
            ));
        }
    }

    let _ = rec.write(&event_row(Utc::now(), "stop", &format!("msgs={msgs} bad={bad}")));
    rec.flush().map_err(|e| e.to_string())?;
    say(&format!("stopped: {msgs} messages, {} rows in {}", rec.rows, path.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::BTreeMap;

    fn cell<'a>(row: &'a [String], name: &str) -> &'a str {
        &row[col(name)]
    }

    fn map(pairs: &[(u32, &str)]) -> BTreeMap<u32, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    fn sample() -> RawMsg {
        RawMsg {
            id: 950978197,
            base: map(&[(1, "5"), (2, "77"), (3, "950978197"), (4, "T"), (5, "1763400000123"), (8, "9")]),
            sale: map(&[(1, "1.25"), (3, "0.25"), (4, "1.30"), (6, "0.30"), (7, "12,500,000")]),
            stats: map(&[(3, "1234567"), (9, "1.4"), (10, "0.9"), (52, "3")]),
            modules: vec![(6, 7)],
        }
    }

    #[test]
    fn file_name_uses_iso_date() {
        assert_eq!(file_name(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()), "probe-2026-11-07.csv");
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_line(&["a", "b c", ""]), "a,b c,");
        assert_eq!(csv_line(&["x,y", "say \"hi\"", "l1\nl2"]), "\"x,y\",\"say \"\"hi\"\"\",\"l1\nl2\"");
    }

    #[test]
    fn msg_row_has_one_cell_per_column() {
        // 2025-11-17 14:00:00.500 UTC is 09:00 in New York: pre-market.
        let at = Utc.with_ymd_and_hms(2025, 11, 17, 14, 0, 0).unwrap() + chrono::Duration::milliseconds(500);
        let row = msg_row(at, &sample(), "ABC", Some(3));
        assert_eq!(row.len(), COLUMNS.len());
        assert_eq!(cell(&row, "recv_utc"), "2025-11-17T14:00:00.500Z");
        assert_eq!(cell(&row, "recv_ny"), "2025-11-17 09:00:00.500");
        assert_eq!(cell(&row, "recv_session"), "PRE");
        assert_eq!(cell(&row, "kind"), "msg");
        assert_eq!(cell(&row, "ticker_id"), "950978197");
        assert_eq!(cell(&row, "symbol"), "ABC");
        assert_eq!(cell(&row, "list_pos"), "3");
        assert_eq!(cell(&row, "trd_seq"), "77");
        assert_eq!(cell(&row, "status"), "T");
        assert_eq!(cell(&row, "trade_stamp_ms"), "1763400000123");
        assert_eq!(cell(&row, "trade_utc"), "2025-11-17T17:20:00.123Z");
        assert_eq!(cell(&row, "trade_session"), "REG"); // 12:20 New York
        assert_eq!(cell(&row, "close"), "1.25");
        assert_eq!(cell(&row, "p_price"), "1.30");
        assert_eq!(cell(&row, "p_ch_ratio"), "0.30");
        assert_eq!(cell(&row, "market_value"), "12,500,000"); // raw, with the comma preserved (and quoted in the file)
        assert_eq!(cell(&row, "volume"), "1234567");
        assert_eq!(cell(&row, "high"), "1.4");
        assert_eq!(cell(&row, "low"), "0.9");
        assert_eq!(cell(&row, "change"), "");
        assert_eq!(cell(&row, "modules"), "6:7");
        assert_eq!(cell(&row, "extra"), "b8=9;t52=3");
        assert_eq!(cell(&row, "detail"), "");
    }

    #[test]
    fn msg_row_tolerates_missing_stamp_and_unlisted_symbol() {
        let mut m = sample();
        m.base.remove(&5);
        let row = msg_row(Utc::now(), &m, "", None);
        assert_eq!(cell(&row, "trade_stamp_ms"), "");
        assert_eq!(cell(&row, "trade_utc"), "");
        assert_eq!(cell(&row, "trade_session"), "");
        assert_eq!(cell(&row, "list_pos"), "");
    }

    #[test]
    fn event_rows_and_descriptions() {
        let at = Utc.with_ymd_and_hms(2025, 11, 17, 3, 0, 0).unwrap();
        let (kind, detail) = event_fields(&Event::SubAck { id: 4, codes: vec![0, 0x80] }).unwrap();
        let row = event_row(at, kind, &detail);
        assert_eq!(row.len(), COLUMNS.len());
        assert_eq!(cell(&row, "kind"), "suback");
        assert_eq!(cell(&row, "detail"), "id=4 codes=0x00 0x80");
        assert_eq!(cell(&row, "recv_session"), "CLOSED"); // 22:00 New York
        assert_eq!(event_fields(&Event::Subscribe { id: 2, count: 100, total: 200 }).unwrap().1, "id=2 count=100 total=200");
        assert_eq!(event_fields(&Event::Disconnected { reason: "no data from stream".into() }).unwrap().0, "disconnect");
        assert!(event_fields(&Event::Raw(sample())).is_none());
    }

    #[test]
    fn recorder_writes_header_once_and_rows() {
        let mut buf = Vec::new();
        let mut rec = Recorder::new(&mut buf, true).unwrap();
        rec.write(&msg_row(Utc::now(), &sample(), "A,B", None)).unwrap();
        rec.write(&event_row(Utc::now(), "stop", "done")).unwrap();
        assert_eq!(rec.rows, 2);
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], COLUMNS.join(","));
        assert!(lines[1].contains("\"A,B\""));
        assert!(lines[1].contains("\"12,500,000\""));
        assert!(lines[2].ends_with(",done"));
    }

    #[test]
    fn open_csv_appends_without_second_header() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-probe");
        let _ = fs::remove_dir_all(&dir);
        let date = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        for note in ["first", "second"] {
            let (mut rec, path) = open_csv(&dir, date).unwrap();
            assert_eq!(path, dir.join("probe-2026-11-17.csv"));
            rec.write(&event_row(Utc::now(), "start", note)).unwrap();
            rec.flush().unwrap();
        }
        let text = fs::read_to_string(dir.join("probe-2026-11-17.csv")).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert_eq!(text.matches("recv_utc").count(), 1);
    }
}
