//! Disk cache of the fetched 5m / 1h / 4h / D chart bars: `bars/<tf>/<SYMBOL>.csv` in the data
//! directory, same columns as the candle files. A cached chart is only topped up after a
//! launch instead of fetched again bar by bar.

use crate::engine::Candle;
use crate::history::{self, Tf};
use crate::store::{self, format_line, parse_line};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Sub-folder of the data directory holding the cache.
pub const DIR: &str = "bars";

fn path(dir: &Path, tf: Tf, symbol: &str) -> PathBuf {
    store::path(&dir.join(tf.label()), symbol)
}

/// The cached bars of `symbol` in `tf`, oldest first; empty when there are none.
pub fn load(dir: &Path, tf: Tf, symbol: &str) -> Vec<Candle> {
    let Ok(text) = fs::read_to_string(path(dir, tf, symbol)) else { return Vec::new() };
    let mut v: Vec<Candle> = text.lines().filter_map(parse_line).collect();
    v.sort_by_key(|c| c.start);
    v.dedup_by_key(|c| c.start);
    v
}

/// Writes the newest `history::MAX_HELD` of `bars` (oldest first) as the whole file.
pub fn save(dir: &Path, tf: Tf, symbol: &str, bars: &[Candle]) -> io::Result<()> {
    let p = path(dir, tf, symbol);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let text: String = bars[bars.len().saturating_sub(history::MAX_HELD)..].iter().map(format_line).collect();
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(start: i64, p: f64) -> Candle {
        Candle { start, open: p, high: p + 1.0, low: p - 0.5, close: p, volume: 7 }
    }

    fn dir(name: &str) -> PathBuf {
        let d = PathBuf::from("target").join("test-barcache").join(name);
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trip_per_timeframe() {
        let d = dir("roundtrip");
        let bars = vec![bar(300, 1.25), bar(600, 2.5)];
        save(&d, Tf::M5, "BRK B", &bars).unwrap();
        assert_eq!(load(&d, Tf::M5, "BRK B"), bars);
        assert!(load(&d, Tf::H1, "BRK B").is_empty());
        assert!(d.join("5m").is_dir());
    }

    #[test]
    fn merge_then_save_keeps_order_and_the_cap() {
        let d = dir("merge");
        save(&d, Tf::D1, "A", &[bar(0, 1.0), bar(86_400, 2.0)]).unwrap();
        let mut held = load(&d, Tf::D1, "A");
        history::merge(&mut held, &[bar(86_400, 3.0), bar(172_800, 4.0)]);
        save(&d, Tf::D1, "A", &held).unwrap();
        assert_eq!(load(&d, Tf::D1, "A"), vec![bar(0, 1.0), bar(86_400, 3.0), bar(172_800, 4.0)]);
        let many: Vec<Candle> = (0..history::MAX_HELD as i64 + 5).map(|i| bar(i * 60, 1.0)).collect();
        save(&d, Tf::D1, "A", &many).unwrap();
        let back = load(&d, Tf::D1, "A");
        assert_eq!((back.len(), back[0].start), (history::MAX_HELD, 5 * 60));
    }
}
