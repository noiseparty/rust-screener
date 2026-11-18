//! The screener engine: filters, Vol/m, NEW and SETUP alerts, Mov ranks and 1-minute candles.
//!
//! `Engine` and its constants live here; its methods are spread over the child modules by topic.

mod alerts;
mod candles;
mod feed;
mod process;
mod ranking;
#[cfg(test)]
mod tests;
mod types;

pub use types::{Alert, AlertKind, Candle, Filters, Output, SetupBase, Thresholds};

use crate::config::Config;
use crate::scraper::Ticker;
use crate::series::SeriesStore;
use feed::series_horizon;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

pub const MAX_ROWS: usize = 100;
/// 1-minute candles kept in memory per ticker (about two full extended-hours sessions).
pub const MAX_CANDLES: usize = 2000;
/// A stream quote older than this no longer overrides the REST row.
pub const LIVE_FRESH: Duration = Duration::from_secs(15);
/// Assumed stream subscription cap (to be confirmed by the probe); members come first.
pub const MAX_SUBS: usize = 100;
/// Shortest Vol/m window, and the least history a rate needs.
pub const MIN_VOL_M_WINDOW_SECS: u64 = 30;
/// A row that stops passing a display filter stays visible, dimmed, this long.
pub const FILTER_GRACE: Duration = Duration::from_secs(60);
/// An alert's Status label stays at least this long (and for the whole cooldown if that is longer).
pub const BADGE_HOLD: Duration = Duration::from_secs(60);
/// NEW: a symbol on the list now that was not on it for this long.
pub const NEW_QUIET: Duration = Duration::from_secs(1800);

/// Latest accepted stream quote of one ticker.
#[derive(Debug, Clone, Copy)]
struct Live {
    price: f64,
    volume: i64,
    at: Instant,
}

pub struct Engine {
    pub filters: Filters,
    pub th: Thresholds,
    pub blacklist: BTreeSet<String>,
    /// Every ticker from the most recent process() call, for local re-filter/re-sort.
    latest: Vec<Ticker>,
    /// Full-board order by % change (every listed symbol, blacklisted included), pushed
    /// whenever it changes; the entry in force at `now - rank_interval` is the Mov baseline.
    rank_ring: VecDeque<(Instant, Vec<String>)>,
    /// Symbols that passed the display filters at the last recompute.
    passing: HashSet<String>,
    /// Symbols that stopped passing, and when. Visible (dimmed) for `FILTER_GRACE`, then hidden.
    failing_since: HashMap<String, Instant>,
    alerts: VecDeque<Alert>,
    /// When each symbol was last on the list (passing the display filters, blacklisted included).
    /// Drives NEW; entries older than `LISTED_KEEP` are dropped.
    listed: HashMap<String, Instant>,
    /// The next fetched list only seeds `listed` and raises no NEW (startup, session change).
    reseed: bool,
    /// When each alert kind (by `AlertKind::slot`) last fired for a symbol. Drives the cooldown
    /// and the Status badge; entries older than the badge hold are dropped.
    fired: HashMap<String, [Option<Instant>; 4]>,
    candles: HashMap<String, VecDeque<Candle>>,
    /// Start of the newest candle per symbol already handed to storage (or loaded from it).
    stored_upto: HashMap<String, i64>,
    last_cum_vol: HashMap<String, i64>,
    live: HashMap<u64, Live>,
    /// tickerId -> position in `latest`.
    index: HashMap<u64, usize>,
    series: SeriesStore,
    /// Whether REST rows may write to the series (false while the stream is healthy).
    rest_feeds_series: bool,
    /// No fresh data is arriving (stream silent and REST not answering): Vol/m shows a dash.
    feed_stale: bool,
}

impl Engine {
    pub fn new(cfg: &Config) -> Engine {
        Engine {
            filters: Filters::from_config(cfg),
            th: Thresholds::from_config(cfg),
            blacklist: cfg.blacklisted_tickers.iter().cloned().collect(),
            latest: Vec::new(),
            rank_ring: VecDeque::new(),
            passing: HashSet::new(),
            failing_since: HashMap::new(),
            alerts: VecDeque::new(),
            listed: HashMap::new(),
            reseed: true,
            fired: HashMap::new(),
            candles: HashMap::new(),
            stored_upto: HashMap::new(),
            last_cum_vol: HashMap::new(),
            live: HashMap::new(),
            index: HashMap::new(),
            series: SeriesStore::new(series_horizon(&Thresholds::from_config(cfg))),
            rest_feeds_series: true,
            feed_stale: false,
        }
    }
}
