//! Candle storage: completed 1-minute candles are appended to `candles-YYYY-MM-DD.csv`
//! (New York trading date) in the data directory, reloaded on start, and files from
//! earlier days are deleted on launch. Every function takes the directory as a parameter.
//!
//! Columns (no header): `symbol,start_epoch_secs,open,high,low,close,volume`.

use crate::engine::Candle;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::America::New_York;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const PREFIX: &str = "candles-";
const SUFFIX: &str = ".csv";

/// New York trading date of an epoch-seconds instant.
pub fn ny_date(epoch_secs: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap_or_default().with_timezone(&New_York).date_naive()
}

/// File name for a trading date.
pub fn file_name(date: NaiveDate) -> String {
    format!("{PREFIX}{}{SUFFIX}", date.format("%Y-%m-%d"))
}

/// The date in a file name of exactly the form `candles-YYYY-MM-DD.csv`, else None.
fn parse_file_name(name: &str) -> Option<NaiveDate> {
    let d = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let date = NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?;
    (file_name(date) == name).then_some(date)
}

fn format_line(sym: &str, c: &Candle) -> String {
    format!("{sym},{},{},{},{},{},{}\n", c.start, c.open, c.high, c.low, c.close, c.volume)
}

fn parse_line(line: &str) -> Option<(String, Candle)> {
    let f: Vec<&str> = line.trim_end_matches('\r').split(',').collect();
    if f.len() != 7 || f[0].is_empty() {
        return None;
    }
    let num = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite());
    Some((
        f[0].to_string(),
        Candle {
            start: f[1].parse().ok()?,
            open: num(f[2])?,
            high: num(f[3])?,
            low: num(f[4])?,
            close: num(f[5])?,
            volume: f[6].parse().ok()?,
        },
    ))
}

/// Deletes `candles-YYYY-MM-DD.csv` files dated before `today` from `dir`; nothing else is
/// touched. Returns how many were deleted. A missing directory is not an error.
pub fn purge_old(dir: &Path, today: NaiveDate) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut n = 0;
    for entry in entries {
        let entry = entry?;
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        let old = entry.file_name().to_str().and_then(parse_file_name).is_some_and(|d| d < today);
        if is_file && old {
            fs::remove_file(entry.path())?;
            n += 1;
        }
    }
    Ok(n)
}

/// Reads `date`'s file. Lines that are incomplete or malformed are skipped, and a last line
/// without its newline (a write cut short) is dropped even if it happens to parse.
/// A missing file gives an empty list.
pub fn load(dir: &Path, date: NaiveDate) -> io::Result<Vec<(String, Candle)>> {
    let bytes = match fs::read(dir.join(file_name(date))) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&bytes);
    let complete = match text.rfind('\n') {
        Some(i) => &text[..i],
        None => return Ok(Vec::new()),
    };
    Ok(complete.split('\n').filter_map(parse_line).collect())
}

/// Appends candles to the file of their trading date, switching files when the date changes.
pub struct CandleStore {
    dir: PathBuf,
    open: Option<(NaiveDate, File)>,
}

impl CandleStore {
    pub fn new(dir: &Path) -> CandleStore {
        CandleStore { dir: dir.to_path_buf(), open: None }
    }

    /// Opens `date`'s file for appending. A partially written last line left by a crash is cut
    /// off first so new rows start on their own line.
    fn open_for(&mut self, date: NaiveDate) -> io::Result<&mut File> {
        if self.open.as_ref().is_none_or(|(d, _)| *d != date) {
            fs::create_dir_all(&self.dir)?;
            let path = self.dir.join(file_name(date));
            if let Ok(bytes) = fs::read(&path) {
                if bytes.last().is_some_and(|b| *b != b'\n') {
                    let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
                    OpenOptions::new().write(true).open(&path)?.set_len(keep as u64)?;
                }
            }
            let f = OpenOptions::new().create(true).append(true).open(&path)?;
            self.open = Some((date, f));
        }
        Ok(&mut self.open.as_mut().expect("just opened").1)
    }

    /// Appends the rows (each into the file of its own New York date). On a write error the
    /// file handle is dropped so the next call reopens it.
    pub fn append(&mut self, rows: &[(String, Candle)]) -> io::Result<()> {
        for (sym, c) in rows {
            let line = format_line(sym, c);
            let res = self.open_for(ny_date(c.start)).and_then(|f| f.write_all(line.as_bytes()));
            if let Err(e) = res {
                self.open = None;
                return Err(e);
            }
        }
        Ok(())
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

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Epoch seconds of 12:00 UTC (07:00/08:00 in New York) on the date.
    fn noon(d: NaiveDate) -> i64 {
        d.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp()
    }

    fn candle(start: i64, p: f64, vol: i64) -> Candle {
        Candle { start, open: p, high: p + 0.5, low: p - 0.25, close: p + 0.1, volume: vol }
    }

    #[test]
    fn append_and_reload_round_trip() {
        let d = dir("roundtrip");
        let day = date(2026, 3, 4);
        let rows = vec![
            ("AAPL".to_string(), candle(noon(day), 10.125, 1000)),
            ("XYZ".to_string(), candle(noon(day) + 60, 0.0123, 7)),
            ("AAPL".to_string(), candle(noon(day) + 120, 10.2, 0)),
        ];
        let mut s = CandleStore::new(&d);
        s.append(&rows[..2]).unwrap();
        s.append(&rows[2..]).unwrap();
        assert!(d.join("candles-2026-03-04.csv").is_file());
        assert_eq!(load(&d, day).unwrap(), rows);
        assert!(load(&d, date(2026, 3, 5)).unwrap().is_empty());
    }

    #[test]
    fn truncated_and_bad_lines_are_tolerated() {
        let d = dir("truncated");
        let day = date(2026, 3, 4);
        let good1 = format_line("A", &candle(noon(day), 1.0, 5));
        let good2 = format_line("B", &candle(noon(day) + 60, 2.0, 6));
        // Last line is cut mid-write but still parses; it must not be trusted.
        let text = format!("{good1}garbage line\nA,notanumber,1,1,1,1,1\n{good2}C,1,2,3\r\nD,{},1,1,1,1,12", noon(day) + 120);
        fs::write(d.join(file_name(day)), text).unwrap();
        let got = load(&d, day).unwrap();
        assert_eq!(got.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), ["A", "B"]);

        // Appending after the cut repairs the file: the partial line is gone, new rows stand alone.
        let mut s = CandleStore::new(&d);
        s.append(&[("E".to_string(), candle(noon(day) + 180, 3.0, 9))]).unwrap();
        let got = load(&d, day).unwrap();
        assert_eq!(got.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), ["A", "B", "E"]);
        assert_eq!(fs::read_to_string(d.join(file_name(day))).unwrap().lines().last().unwrap().split(',').next(), Some("E"));

        // A file with no complete line at all loads empty.
        fs::write(d.join(file_name(date(2026, 3, 6))), "A,1,2").unwrap();
        assert!(load(&d, date(2026, 3, 6)).unwrap().is_empty());
    }

    #[test]
    fn purge_removes_only_old_candle_files() {
        let d = dir("purge");
        let today = date(2026, 3, 4);
        for n in [
            "candles-2026-03-03.csv",
            "candles-2025-12-31.csv",
            "candles-2026-03-04.csv", // today: kept
            "candles-2026-03-05.csv", // not a previous day: kept
            "config.json",
            "candles-2026-03-03.csv.bak",
            "candles-2026-3-3.csv",
            "candles-latest.csv",
            "old-candles-2026-03-03.csv",
        ] {
            fs::write(d.join(n), "x").unwrap();
        }
        fs::create_dir(d.join("candles-2026-03-02.csv")).unwrap(); // a directory is never removed
        assert_eq!(purge_old(&d, today).unwrap(), 2);
        let mut left: Vec<String> = fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(
            left,
            [
                "candles-2026-03-02.csv".to_string(),
                "candles-2026-03-03.csv.bak".to_string(),
                "candles-2026-03-04.csv".to_string(),
                "candles-2026-03-05.csv".to_string(),
                "candles-2026-3-3.csv".to_string(),
                "candles-latest.csv".to_string(),
                "config.json".to_string(),
                "old-candles-2026-03-03.csv".to_string(),
            ]
        );
        assert_eq!(purge_old(&d.join("missing"), today).unwrap(), 0);
    }

    #[test]
    fn rollover_starts_a_new_file() {
        let d = dir("rollover");
        // 03:58 UTC on Mar 5 is 22:58 on Mar 4 in New York (EST); 05:01 UTC is 00:01 on Mar 5.
        let before = date(2026, 3, 5).and_hms_opt(3, 58, 0).unwrap().and_utc().timestamp();
        let after = date(2026, 3, 5).and_hms_opt(5, 1, 0).unwrap().and_utc().timestamp();
        assert_eq!(ny_date(before), date(2026, 3, 4));
        assert_eq!(ny_date(after), date(2026, 3, 5));
        let mut s = CandleStore::new(&d);
        s.append(&[("A".to_string(), candle(before, 1.0, 1))]).unwrap();
        s.append(&[("A".to_string(), candle(after, 2.0, 2))]).unwrap();
        assert_eq!(load(&d, date(2026, 3, 4)).unwrap().len(), 1);
        assert_eq!(load(&d, date(2026, 3, 5)).unwrap(), vec![("A".to_string(), candle(after, 2.0, 2))]);
    }
}
