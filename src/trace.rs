//! Trace mode: a log of how data travels through the app (stream, REST, workers, app messages,
//! engine). On in a debug-assertions build, with `--trace` or with `MARKETGUI_TRACE=1`; when off
//! every call is one atomic load and nothing is formatted.
//!
//! Lines go to `<home>\logs\trace-YYYY-MM-DD.log` (local date) and the newest `RING` stay in
//! memory for the debug overlay. Per category it counts messages and keeps the newest summary:
//! the overlay's flow table. `tlog!` counts and writes a line; `tnote!` only counts (for things
//! that happen several times a second, such as stream batches).

use chrono::{DateTime, Local, NaiveDate};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Log lines kept in memory.
pub const RING: usize = 500;
/// Trace files older than this many days are deleted at startup.
pub const KEEP_DAYS: i64 = 7;
/// Environment variable that turns tracing on (`1`).
pub const ENV: &str = "MARKETGUI_TRACE";

static ON: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Counts `cat` and writes a line: `tlog!("stream", "connected #{n}")`. Formats only when on.
macro_rules! tlog {
    ($cat:expr, $($arg:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::record($cat, format!($($arg)*), true)
        }
    };
}
/// Counts `cat` and keeps the summary, without a log line.
macro_rules! tnote {
    ($cat:expr, $($arg:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::record($cat, format!($($arg)*), false)
        }
    };
}
pub(crate) use {tlog, tnote};

struct Counter {
    count: u64,
    last: Instant,
    summary: String,
}

/// One category of the flow table.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub cat: &'static str,
    pub count: u64,
    /// Time since its newest message.
    pub age: Duration,
    pub summary: String,
}

/// What the overlay shows: the flow table (by category name) and the newest lines, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub flows: Vec<Flow>,
    pub lines: Vec<String>,
}

/// The ring of lines, the counters and the open log file.
pub struct Log {
    lines: VecDeque<String>,
    counters: BTreeMap<&'static str, Counter>,
    /// The logs folder; None keeps everything in memory (tests).
    dir: Option<PathBuf>,
    file: Option<(NaiveDate, File)>,
}

impl Log {
    pub fn new(dir: Option<PathBuf>) -> Log {
        Log { lines: VecDeque::with_capacity(RING), counters: BTreeMap::new(), dir, file: None }
    }

    /// Counts `msg` under `cat`; with `line` it is also kept in the ring and written to the file.
    pub fn record(&mut self, cat: &'static str, msg: String, line: bool, now: Instant, wall: DateTime<Local>) {
        if line {
            let text = format!("{} {cat:<15} {msg}", wall.format("%H:%M:%S%.3f"));
            self.write(&text, wall.date_naive());
            if self.lines.len() >= RING {
                self.lines.pop_front();
            }
            self.lines.push_back(text);
        }
        let c = self.counters.entry(cat).or_insert(Counter { count: 0, last: now, summary: String::new() });
        c.count += 1;
        c.last = now;
        c.summary = msg;
    }

    /// Appends `text` to the file of `day`, opening it on the first line and after midnight.
    /// A file that cannot be written is skipped; the ring keeps the line.
    fn write(&mut self, text: &str, day: NaiveDate) {
        let Some(dir) = &self.dir else { return };
        if self.file.as_ref().is_none_or(|(d, _)| *d != day) {
            let _ = fs::create_dir_all(dir);
            self.file = OpenOptions::new().create(true).append(true).open(file_path(dir, day)).ok().map(|f| (day, f));
        }
        if let Some((_, f)) = &mut self.file {
            if writeln!(f, "{text}").is_err() {
                self.file = None;
            }
        }
    }

    pub fn snapshot(&self, now: Instant) -> Snapshot {
        let flows = self
            .counters
            .iter()
            .map(|(&cat, c)| Flow { cat, count: c.count, age: now.saturating_duration_since(c.last), summary: c.summary.clone() })
            .collect();
        Snapshot { flows, lines: self.lines.iter().cloned().collect() }
    }
}

/// `trace-YYYY-MM-DD.log` in `dir`.
pub fn file_path(dir: &Path, day: NaiveDate) -> PathBuf {
    dir.join(format!("trace-{}.log", day.format("%Y-%m-%d")))
}

/// Whether tracing is on.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Whether this run traces: a debug-assertions build, the `--trace` argument (`flag`) or
/// `MARKETGUI_TRACE=1`.
pub fn wanted(flag: bool) -> bool {
    flag || cfg!(debug_assertions) || std::env::var(ENV).is_ok_and(|v| v.trim() == "1")
}

/// Deletes old trace files in `dir` and, with `on`, starts tracing into it.
pub fn init(dir: &Path, on: bool) {
    let purged = purge(dir, Local::now().date_naive());
    if !on {
        return;
    }
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Log::new(Some(dir.to_path_buf())));
    ON.store(true, Ordering::Relaxed);
    tlog!("trace", "on, v{}, logs in {} ({purged} old files deleted)", env!("CARGO_PKG_VERSION"), dir.display());
}

/// Records into the global log (use `tlog!` / `tnote!`, which check `enabled` first).
pub fn record(cat: &'static str, msg: String, line: bool) {
    if let Some(l) = LOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        l.record(cat, msg, line, Instant::now(), Local::now());
    }
}

/// The overlay's data; None while tracing is off.
pub fn snapshot() -> Option<Snapshot> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|l| l.snapshot(Instant::now()))
}

/// Deletes `trace-YYYY-MM-DD.log` files in `dir` dated more than `KEEP_DAYS` before `today`.
/// Nothing else in the folder is touched. Returns how many were deleted.
pub fn purge(dir: &Path, today: NaiveDate) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(day) = name.strip_prefix("trace-").and_then(|s| s.strip_suffix(".log")) else { continue };
        let Ok(day) = NaiveDate::parse_from_str(day, "%Y-%m-%d") else { continue };
        if (today - day).num_days() > KEEP_DAYS && e.path().is_file() && fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// One app message as it is dispatched: counted per kind; all but the frequent quotes and
/// stream status also get a line.
pub fn dispatch(m: &crate::app::Msg) {
    use crate::app::Msg;
    if !enabled() {
        return;
    }
    match m {
        Msg::Quotes(u) => tnote!("app.quotes", "{} quotes", u.len()),
        Msg::Stream(s) => tnote!(
            "app.stream",
            "{} #{}, {} subs, {:.0} msg/s{}",
            if s.connected { "up" } else { "down" },
            s.connections,
            s.subs,
            s.msgs_per_sec,
            s.retry_in.map_or(String::new(), |r| format!(", retry in {r}s"))
        ),
        Msg::Fetched(Ok(f)) => tlog!("app.fetched", "{} rows ({})", f.tickers.len(), f.group),
        Msg::Fetched(Err(e)) => tlog!("app.fetched", "error: {e}"),
        Msg::Bars { symbol, tf, done, failed, bars, .. } => {
            tlog!(
                "app.bars",
                "{symbol} {} {} bars{}{}",
                tf.label(),
                bars.len(),
                if *done { "" } else { " (part)" },
                if *failed { " FAILED" } else { "" }
            )
        }
        Msg::Backfill { symbol, bars, failed } => {
            tlog!("app.backfill", "{symbol} {} bars{}", bars.len(), if *failed { " FAILED" } else { "" })
        }
        Msg::Levels { symbol, bars, .. } => {
            tlog!("app.levels", "{symbol} {}", bars.as_ref().map_or("FAILED".to_string(), |b| format!("{} bars", b.len())))
        }
        Msg::Floats(Ok(v)) => tlog!("app.floats", "{} floats", v.len()),
        Msg::Floats(Err(e)) => tlog!("app.floats", "error: {e}"),
        Msg::Health(r) => tlog!("app.health", "{}/{} checks ok", r.iter().filter(|c| c.ok).count(), r.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::test::scratch;

    fn wall(day: u32) -> DateTime<Local> {
        use chrono::TimeZone;
        Local.with_ymd_and_hms(2026, 11, day, 9, 30, 0).single().unwrap()
    }

    #[test]
    fn ring_keeps_the_newest_lines_and_counts_every_message() {
        let mut log = Log::new(None);
        let t0 = Instant::now();
        for i in 0..RING + 20 {
            log.record("rest.gainers", format!("{i} rows"), true, t0, wall(18));
        }
        log.record("stream.batch", "12 quotes".into(), false, t0 + Duration::from_secs(2), wall(18));
        let s = log.snapshot(t0 + Duration::from_secs(5));
        assert_eq!(s.lines.len(), RING);
        assert!(s.lines[0].ends_with(" 20 rows") && s.lines[0].starts_with("09:30:00.000 rest.gainers"), "{}", s.lines[0]);
        assert!(s.lines.last().unwrap().ends_with(&format!(" {} rows", RING + 19)));
        // notes count but write no line
        assert_eq!(s.flows.len(), 2);
        assert_eq!(
            s.flows[0],
            Flow { cat: "rest.gainers", count: RING as u64 + 20, age: Duration::from_secs(5), summary: format!("{} rows", RING + 19) }
        );
        assert_eq!(s.flows[1], Flow { cat: "stream.batch", count: 1, age: Duration::from_secs(3), summary: "12 quotes".into() });
    }

    #[test]
    fn writes_one_file_per_day_and_purges_old_ones() {
        let dir = scratch("test-trace", "files");
        let mut log = Log::new(Some(dir.clone()));
        log.record("a", "one".into(), true, Instant::now(), wall(17));
        log.record("a", "two".into(), true, Instant::now(), wall(18));
        log.record("a", "note".into(), false, Instant::now(), wall(18));
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        assert!(fs::read_to_string(file_path(&dir, day(17))).unwrap().ends_with("a               one\n"));
        assert_eq!(fs::read_to_string(file_path(&dir, day(18))).unwrap().lines().count(), 1);
        // only trace files older than KEEP_DAYS go
        fs::write(file_path(&dir, day(1)), "old").unwrap();
        fs::write(dir.join("trace-notes.log"), "kept").unwrap();
        fs::write(dir.join("other-2026-11-01.log"), "kept").unwrap();
        assert_eq!(purge(&dir, day(10)), 1);
        assert_eq!(purge(&dir, day(10)), 0);
        assert!(dir.join("trace-notes.log").exists() && dir.join("other-2026-11-01.log").exists());
        assert!(file_path(&dir, day(17)).exists());
        assert_eq!(purge(&dir.join("missing"), day(10)), 0);
    }

    #[test]
    fn wanted_by_flag_or_debug_build() {
        assert!(wanted(true));
        assert_eq!(wanted(false), cfg!(debug_assertions) || std::env::var(ENV).is_ok_and(|v| v.trim() == "1"));
    }
}
