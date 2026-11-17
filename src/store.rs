//! Candle storage: one file per ticker, `candles/<SYMBOL>.csv` in the data directory, with
//! every completed 1-minute candle appended. A ticker's file is read the first time the
//! ticker shows up in a run. Nothing is deleted automatically: Config -> Storage lists the
//! files and deletes what the user no longer wants.
//!
//! Columns (no header): `start_epoch_secs,open,high,low,close,volume`.
//!
//! Older versions wrote one file per day (`candles-YYYY-MM-DD.csv`, with the symbol as the
//! first column); `migrate_day_files` moves those into the per-ticker files once.

use crate::engine::Candle;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::America::New_York;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Sub-folder of the data directory holding the per-ticker files.
pub const DIR: &str = "candles";

/// New York trading date of an epoch-seconds instant.
pub fn ny_date(epoch_secs: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap_or_default().with_timezone(&New_York).date_naive()
}

/// File of `symbol`. Letters, digits, `.` and `-` are kept; anything else (e.g. the space in
/// "BRK B") becomes `_` followed by its code, so different symbols never share a file.
pub fn path(dir: &Path, symbol: &str) -> PathBuf {
    let mut safe = String::new();
    for c in symbol.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            safe.push(c);
        } else {
            safe.push_str(&format!("_{:x}", c as u32));
        }
    }
    dir.join(format!("{safe}.csv"))
}

/// One file line: `start,open,high,low,close,volume`.
pub(crate) fn format_line(c: &Candle) -> String {
    format!("{},{},{},{},{},{}\n", c.start, c.open, c.high, c.low, c.close, c.volume)
}

fn parse_fields(f: &[&str]) -> Option<Candle> {
    let num = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite());
    Some(Candle { start: f[0].parse().ok()?, open: num(f[1])?, high: num(f[2])?, low: num(f[3])?, close: num(f[4])?, volume: f[5].parse().ok()? })
}

/// A line written by `format_line`; None when malformed.
pub(crate) fn parse_line(line: &str) -> Option<Candle> {
    let f: Vec<&str> = line.trim_end_matches('\r').split(',').collect();
    if f.len() != 6 {
        return None;
    }
    parse_fields(&f)
}

/// Complete lines of a file: a last line without its newline (a write cut short) is dropped.
fn complete_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    match text.rfind('\n') {
        Some(i) => text[..i].split('\n').map(str::to_string).collect(),
        None => Vec::new(),
    }
}

/// The last `max` candles of `symbol`, oldest first. Malformed lines are skipped; of rows
/// with the same start the last written wins; a missing file gives an empty list. A file with
/// rows out of order or repeated (backfilled minutes appended later) is rewritten sorted.
pub fn load(dir: &Path, symbol: &str, max: usize) -> io::Result<Vec<Candle>> {
    let p = path(dir, symbol);
    let bytes = match fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut v: Vec<Candle> = complete_lines(&bytes).iter().filter_map(|l| parse_line(l)).collect();
    let messy = v.windows(2).any(|w| w[1].start <= w[0].start);
    // the stable sort keeps the write order of equal starts; the later row replaces the kept one
    v.sort_by_key(|c| c.start);
    v.dedup_by(|later, kept| {
        let same = later.start == kept.start;
        if same {
            *kept = *later;
        }
        same
    });
    if messy {
        let text: String = v.iter().map(format_line).collect();
        let tmp = p.with_extension("tmp");
        let _ = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &p));
    }
    let extra = v.len().saturating_sub(max);
    Ok(v.split_off(extra))
}

/// Moves old per-day files (`candles-YYYY-MM-DD.csv` in `data`) into the per-ticker files in
/// `candles` and deletes them. Returns how many day files were moved.
pub fn migrate_day_files(data: &Path, candles: &Path) -> io::Result<usize> {
    let Ok(entries) = fs::read_dir(data) else { return Ok(0) };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let is_day = name.strip_prefix("candles-").and_then(|s| s.strip_suffix(".csv")).is_some_and(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
        if !is_day || !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let mut by_sym: BTreeMap<String, Vec<Candle>> = BTreeMap::new();
        for line in complete_lines(&fs::read(e.path())?) {
            let f: Vec<&str> = line.trim_end_matches('\r').split(',').collect();
            if f.len() == 7 && !f[0].is_empty() {
                if let Some(c) = parse_fields(&f[1..]) {
                    by_sym.entry(f[0].to_string()).or_default().push(c);
                }
            }
        }
        let mut store = CandleStore::new(candles);
        for (sym, bars) in by_sym {
            let rows: Vec<(String, Candle)> = bars.into_iter().map(|c| (sym.clone(), c)).collect();
            store.append(&rows)?;
        }
        fs::remove_file(e.path())?;
        n += 1;
    }
    Ok(n)
}

/// Appends completed candles to their ticker's file.
pub struct CandleStore {
    dir: PathBuf,
    /// Files whose possibly cut-off last line has been checked this run.
    repaired: HashSet<String>,
}

impl CandleStore {
    pub fn new(dir: &Path) -> CandleStore {
        CandleStore { dir: dir.to_path_buf(), repaired: HashSet::new() }
    }

    /// Appends the rows, each to its symbol's file. A partially written last line left by a
    /// crash is cut off the first time a file is written in a run.
    pub fn append(&mut self, rows: &[(String, Candle)]) -> io::Result<()> {
        let mut by_sym: BTreeMap<&str, String> = BTreeMap::new();
        for (sym, c) in rows {
            by_sym.entry(sym.as_str()).or_default().push_str(&format_line(c));
        }
        fs::create_dir_all(&self.dir)?;
        for (sym, text) in by_sym {
            let p = path(&self.dir, sym);
            if self.repaired.insert(sym.to_string()) {
                if let Ok(bytes) = fs::read(&p) {
                    if bytes.last().is_some_and(|b| *b != b'\n') {
                        let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
                        OpenOptions::new().write(true).open(&p)?.set_len(keep as u64)?;
                    }
                }
            }
            OpenOptions::new().create(true).append(true).open(&p)?.write_all(text.as_bytes())?;
        }
        Ok(())
    }

    /// Appends backfilled candles of `symbol` (any minutes, also ones before rows already in the
    /// file); `load` puts the file back in order.
    pub fn insert(&mut self, symbol: &str, bars: &[Candle]) -> io::Result<()> {
        let rows: Vec<(String, Candle)> = bars.iter().map(|c| (symbol.to_string(), *c)).collect();
        self.append(&rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = PathBuf::from("target").join("test-store").join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn candle(start: i64, p: f64, vol: i64) -> Candle {
        Candle { start, open: p, high: p + 0.5, low: p - 0.25, close: p + 0.1, volume: vol }
    }

    #[test]
    fn one_file_per_ticker_round_trip() {
        let d = dir("roundtrip");
        let mut s = CandleStore::new(&d);
        s.append(&[("AAPL".to_string(), candle(60, 10.125, 1000)), ("XYZ".to_string(), candle(120, 0.0123, 7))]).unwrap();
        s.append(&[("AAPL".to_string(), candle(180, 10.2, 0))]).unwrap();
        assert!(d.join("AAPL.csv").is_file() && d.join("XYZ.csv").is_file());
        assert_eq!(load(&d, "AAPL", 100).unwrap(), vec![candle(60, 10.125, 1000), candle(180, 10.2, 0)]);
        // only the newest `max` are returned
        assert_eq!(load(&d, "AAPL", 1).unwrap(), vec![candle(180, 10.2, 0)]);
        assert!(load(&d, "NONE", 10).unwrap().is_empty());
    }

    #[test]
    fn symbols_map_to_distinct_safe_file_names() {
        let d = Path::new("x");
        assert_eq!(path(d, "BRK.B"), d.join("BRK.B.csv"));
        assert_ne!(path(d, "BRK B"), path(d, "BRK_B"));
        assert!(!path(d, "A/B").file_name().unwrap().to_string_lossy().contains('/'));
    }

    #[test]
    fn cut_off_lines_are_dropped_and_repaired() {
        let d = dir("truncated");
        let good = format_line(&candle(60, 1.0, 5));
        fs::write(path(&d, "A"), format!("{good}garbage\n120,1,1,1,1,1")).unwrap();
        assert_eq!(load(&d, "A", 10).unwrap(), vec![candle(60, 1.0, 5)]);
        let mut s = CandleStore::new(&d);
        s.append(&[("A".to_string(), candle(180, 3.0, 9))]).unwrap();
        assert_eq!(load(&d, "A", 10).unwrap(), vec![candle(60, 1.0, 5), candle(180, 3.0, 9)]);
    }

    #[test]
    fn load_sorts_dedupes_last_wins_and_compacts() {
        let d = dir("compact");
        let mut s = CandleStore::new(&d);
        s.append(&[("A".to_string(), candle(120, 1.0, 1)), ("A".to_string(), candle(180, 2.0, 2))]).unwrap();
        // backfilled minutes appended after newer ones, one of them a repeat
        s.insert("A", &[candle(60, 0.5, 3), candle(120, 1.5, 4)]).unwrap();
        let want = vec![candle(60, 0.5, 3), candle(120, 1.5, 4), candle(180, 2.0, 2)];
        assert_eq!(load(&d, "A", 10).unwrap(), want);
        // the file was rewritten in order
        let text = fs::read_to_string(path(&d, "A")).unwrap();
        assert_eq!(text, want.iter().map(format_line).collect::<String>());
        assert!(!path(&d, "A").with_extension("tmp").exists());
        assert_eq!(load(&d, "A", 2).unwrap(), want[1..].to_vec());
    }

    #[test]
    fn day_files_move_into_ticker_files() {
        let d = dir("migrate");
        let line = |s: &str, c: &Candle| format!("{s},{}", format_line(c));
        let day = format!("{}{}{}", line("A", &candle(60, 1.0, 1)), line("B", &candle(60, 2.0, 2)), line("A", &candle(120, 1.5, 3)));
        fs::write(d.join("candles-2026-11-17.csv"), day).unwrap();
        fs::write(d.join("candles-latest.csv"), "x").unwrap();
        let c = d.join(DIR);
        assert_eq!(migrate_day_files(&d, &c).unwrap(), 1);
        assert!(!d.join("candles-2026-11-17.csv").exists());
        assert!(d.join("candles-latest.csv").exists());
        assert_eq!(load(&c, "A", 10).unwrap(), vec![candle(60, 1.0, 1), candle(120, 1.5, 3)]);
        assert_eq!(load(&c, "B", 10).unwrap().len(), 1);
    }
}
