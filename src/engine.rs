use crate::config::Config;
use crate::scraper::{Mov, Ticker};
use crate::series::{Ingest, Obs, SeriesStore, Source};
use crate::session::Session;
use crate::stream::Update;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

pub const MAX_ROWS: usize = 100;
pub const MAX_CANDLES: usize = 390;
/// A stream quote older than this no longer overrides the REST row.
pub const LIVE_FRESH: Duration = Duration::from_secs(15);
/// Assumed stream subscription cap (to be confirmed by the probe); members come first.
pub const MAX_SUBS: usize = 100;
/// Shortest Vol/m window, and the least history a rate needs.
pub const MIN_VOL_M_WINDOW_SECS: u64 = 30;
/// Series history kept beyond the longest window a consumer looks back over.
const SERIES_MARGIN: Duration = Duration::from_secs(60);
/// A row that stops passing a display filter stays visible, dimmed, this long.
pub const FILTER_GRACE: Duration = Duration::from_secs(60);
/// An alert's Status label stays at least this long (and for the whole cooldown if that is longer).
pub const BADGE_HOLD: Duration = Duration::from_secs(60);
/// NEW: a symbol on the list now that was not on it for this long.
pub const NEW_QUIET: Duration = Duration::from_secs(1800);
/// How long a symbol's last time on the list is remembered.
const LISTED_KEEP: Duration = Duration::from_secs(2400);
pub const RATE_WINDOWS: [(&str, u64); 4] = [("20m", 1200), ("5m", 300), ("1m", 60), ("30s", 30)];
/// Badge priority when several alert kinds of one symbol are held: first wins.
const BADGE_PRIORITY: [AlertKind; 4] = [AlertKind::Setup, AlertKind::Pump, AlertKind::Spike, AlertKind::New];

/// One-minute OHLCV bar built from observed price/volume samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    /// Epoch seconds at the start of the minute.
    pub start: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Shares traded inside the minute (difference of cumulative day volume).
    pub volume: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    New,
    Spike,
    Pump,
    Setup,
}

impl AlertKind {
    /// Index into the per-symbol fire times.
    fn slot(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            AlertKind::New => "NEW",
            AlertKind::Spike => "SPIKE",
            AlertKind::Pump => "PUMP",
            AlertKind::Setup => "SETUP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub at: Instant,
    pub clock: String,
    pub symbol: String,
    pub kind: AlertKind,
    pub prev_vol: i64,
    pub curr_vol: i64,
    pub prev_price: f64,
    pub curr_price: f64,
    pub change_ratio: f64,
}

impl Alert {
    pub fn vol_ratio(&self) -> f64 {
        if self.prev_vol > 0 { self.curr_vol as f64 / self.prev_vol as f64 } else { 0.0 }
    }

    pub fn price_pct(&self) -> f64 {
        if self.prev_price > 0.0 { (self.curr_price - self.prev_price) / self.prev_price * 100.0 } else { 0.0 }
    }

    pub fn detail(&self) -> String {
        match self.kind {
            AlertKind::New => "appeared".into(),
            AlertKind::Spike => format!("vol {:.1}x", self.vol_ratio()),
            AlertKind::Pump => format!("price +{:.1}%", self.price_pct()),
            AlertKind::Setup => format!("vol {:.1}x  +{:.1}%", self.vol_ratio(), self.price_pct()),
        }
    }
}

/// Display filters. A zero bound means "no bound".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Filters {
    pub min_mcap: f64,
    pub max_mcap: f64,
    pub min_change_pct: f64,
    pub max_change_pct: f64,
    pub min_volume: i64,
    pub max_volume: i64,
    pub min_price: f64,
    pub max_price: f64,
}

impl Filters {
    pub fn from_config(c: &Config) -> Filters {
        Filters {
            min_mcap: c.min_market_cap,
            max_mcap: c.max_market_cap,
            min_change_pct: c.min_change_percent,
            max_change_pct: c.max_change_percent,
            min_volume: c.min_volume,
            max_volume: c.max_volume,
            min_price: c.min_price,
            max_price: c.max_price,
        }
    }

    pub fn matches(&self, t: &Ticker) -> bool {
        if self.min_mcap > 0.0 || self.max_mcap > 0.0 {
            if t.market_value == 0.0 {
                return false;
            }
            if (self.min_mcap > 0.0 && t.market_value < self.min_mcap)
                || (self.max_mcap > 0.0 && t.market_value > self.max_mcap)
            {
                return false;
            }
        }
        let pct = t.change_ratio * 100.0;
        !((self.min_price > 0.0 && t.price < self.min_price)
            || (self.max_price > 0.0 && t.price > self.max_price)
            || (self.min_volume > 0 && t.volume < self.min_volume)
            || (self.max_volume > 0 && t.volume > self.max_volume)
            || (self.min_change_pct > 0.0 && pct < self.min_change_pct)
            || (self.max_change_pct > 0.0 && pct > self.max_change_pct))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// SETUP: volume now / volume `setup_window` ago must reach this ...
    pub setup_vol_ratio: f64,
    /// ... and price change in percent versus `setup_window` ago must reach this.
    pub setup_price_pct: f64,
    pub setup_window: Duration,
    pub min_volume_for_alert: i64,
    pub min_price_for_alert: f64,
    /// Minimum time between two alerts of the same kind for one symbol.
    pub alert_cooldown: Duration,
    pub rank_interval: Duration,
    pub vol_m_window: Duration,
    pub max_alerts: usize,
}

impl Thresholds {
    pub fn from_config(c: &Config) -> Thresholds {
        Thresholds {
            setup_vol_ratio: c.setup_vol_ratio,
            setup_price_pct: c.setup_price_pct,
            setup_window: Duration::from_secs(c.setup_window_sec),
            min_volume_for_alert: c.min_volume_for_alert,
            min_price_for_alert: c.min_price_for_alert,
            alert_cooldown: Duration::from_secs(c.alert_cooldown_sec),
            rank_interval: Duration::from_secs(c.rank_tracking_interval_seconds),
            vol_m_window: Duration::from_secs(c.vol_m_window_seconds.max(MIN_VOL_M_WINDOW_SECS)),
            max_alerts: c.alert_history_max,
        }
    }
}

pub struct Output {
    pub active: Vec<Ticker>,
    pub blacklisted: Vec<Ticker>,
    pub new_alerts: usize,
}

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
    last: HashMap<String, Ticker>,
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
    /// Price tick direction events across the board: (time, is_up).
    ticks: VecDeque<(Instant, bool)>,
    candles: HashMap<String, VecDeque<Candle>>,
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
            last: HashMap::new(),
            latest: Vec::new(),
            rank_ring: VecDeque::new(),
            passing: HashSet::new(),
            failing_since: HashMap::new(),
            alerts: VecDeque::new(),
            listed: HashMap::new(),
            reseed: true,
            fired: HashMap::new(),
            ticks: VecDeque::new(),
            candles: HashMap::new(),
            last_cum_vol: HashMap::new(),
            live: HashMap::new(),
            index: HashMap::new(),
            series: SeriesStore::new(series_horizon(&Thresholds::from_config(cfg))),
            rest_feeds_series: true,
            feed_stale: false,
        }
    }

    /// Receive-time series of every listed (or lingering) ticker, keyed by tickerId.
    pub fn series(&self) -> &SeriesStore {
        &self.series
    }

    /// The stream reconnected: everything it delivered before is stale.
    pub fn on_stream_reconnect(&mut self) {
        self.series.clear_all();
        self.refresh_vol_rates(Instant::now());
    }

    /// Cumulative volume and price of `id` as they were `setup_window` before `now`, from the
    /// series store. History shorter than the window falls back to the oldest sample.
    /// `None` when the series is empty or cleared.
    fn setup_base(&self, id: u64, now: Instant) -> Option<(u64, f64)> {
        let first = self.series().oldest(id)?;
        let at = now.checked_sub(self.th.setup_window).map_or(first.t, |s| s.max(first.t));
        Some((self.series().vol_at(id, at)?, self.series().px_at(id, at)?))
    }

    /// Whether the data feed is silent; Vol/m shows a dash while it is.
    pub fn set_feed_stale(&mut self, stale: bool) {
        self.feed_stale = stale;
    }

    /// Recomputes Vol/m of every row at `now`: a quiet ticker decays, a cleared series blanks.
    pub fn refresh_vol_rates(&mut self, now: Instant) {
        for t in &mut self.latest {
            (t.vol_rate, t.vol_partial, t.vol_stale) =
                vol_m(&self.series, t.ticker_id, now, self.th.vol_m_window, self.feed_stale);
        }
    }

    /// REST rows write to the series only while the stream is not healthy; otherwise the
    /// series holds stream samples only and a quiet ticker keeps its history.
    pub fn set_rest_feeds_series(&mut self, on: bool) {
        self.rest_feeds_series = on;
    }

    /// Drops lingering series whose 60 s ran out. Returns true when the subscription set changed.
    pub fn expire_series(&mut self, now: Instant) -> bool {
        self.series.expire(now)
    }

    /// Tickers to subscribe: the current list first, then lingering ones, up to `MAX_SUBS`.
    pub fn subscribe_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.latest.iter().map(|t| t.ticker_id).filter(|&i| i != 0).collect();
        ids.truncate(MAX_SUBS);
        let room = MAX_SUBS - ids.len();
        ids.extend(self.series.lingering().into_iter().take(room));
        ids
    }

    /// Folds a batch of stream quotes into the last processed rows (price, volume, change %, market cap).
    /// Records no alerts or history. Returns true when any row changed.
    pub fn apply_quotes(&mut self, updates: &[Update], session: Session, now: Instant) -> bool {
        let mut changed = false;
        for u in updates {
            // Extended-hours price is taken from pPrice when present; regular hours use close.
            let ext = matches!(session, Session::Pre | Session::After);
            let px = if ext { u.p_price.filter(|p| *p > 0.0).or(u.close) } else { u.close };
            // A message without price or volume is dropped, never read as zero.
            let (Some(px), Some(vol)) = (px.filter(|p| *p > 0.0), u.volume) else { continue };
            // The series store owns ordering, lower-volume and session/date rules.
            let obs = Obs { id: u.id, src: Source::Stream, ms: u.trade_ms, seq: u.seq, px: Some(px), vol: Some(vol), at: u.recv };
            match self.series.ingest(obs) {
                Ingest::Dropped => continue,
                Ingest::Unchanged => {
                    // Nothing new, but the stream is alive for this ticker.
                    if let Some(l) = self.live.get_mut(&u.id) {
                        l.at = now;
                    }
                    continue;
                }
                Ingest::Stored => {}
            }
            let vol = vol as i64;
            let rate = vol_m(&self.series, u.id, now, self.th.vol_m_window, self.feed_stale);
            self.live.insert(u.id, Live { price: px, volume: vol, at: now });
            let Some(&i) = self.index.get(&u.id) else { continue };
            let t = &mut self.latest[i];
            t.price = px;
            (t.vol_rate, t.vol_partial, t.vol_stale) = rate;
            t.volume = vol;
            t.change_ratio = change_from(t.prev_close, px).or(u.change_ratio).unwrap_or(t.change_ratio);
            if let Some(m) = u.mcap.filter(|m| *m > 0.0) {
                t.market_value = m;
            }
            let sym = t.symbol.clone();
            self.record_sample(&sym, chrono::Local::now().timestamp(), px, vol);
            changed = true;
        }
        changed
    }

    /// Replaces a REST row's price/volume/change by the freshest stream quote, if there is one.
    /// Returns true when it did.
    fn overlay_live(&self, t: &mut Ticker, now: Instant) -> bool {
        let Some(l) = self.live.get(&t.ticker_id).filter(|l| now.saturating_duration_since(l.at) <= LIVE_FRESH) else {
            return false;
        };
        t.price = l.price;
        t.volume = l.volume;
        if let Some(c) = change_from(t.prev_close, l.price) {
            t.change_ratio = c;
        }
        true
    }

    pub fn candles(&self, sym: &str) -> Option<&VecDeque<Candle>> {
        self.candles.get(sym)
    }

    /// Folds one price/cumulative-volume sample into the symbol's 1-minute candles.
    pub fn record_sample(&mut self, sym: &str, epoch_secs: i64, price: f64, cum_vol: i64) {
        if price <= 0.0 {
            return;
        }
        let prev = self.last_cum_vol.insert(sym.to_string(), cum_vol);
        let traded = prev.map_or(0, |p| (cum_vol - p).max(0));
        let start = epoch_secs - epoch_secs.rem_euclid(60);
        let bars = self.candles.entry(sym.to_string()).or_default();
        match bars.back_mut() {
            Some(c) if c.start == start => {
                c.high = c.high.max(price);
                c.low = c.low.min(price);
                c.close = price;
                c.volume += traded;
            }
            Some(c) if c.start > start => {}
            _ => {
                bars.push_back(Candle { start, open: price, high: price, low: price, close: price, volume: traded });
                if bars.len() > MAX_CANDLES {
                    bars.pop_front();
                }
            }
        }
    }

    /// The trading session changed: the next fetched list only seeds NEW, it raises none.
    pub fn on_session_change(&mut self) {
        self.reseed = true;
    }

    /// Records every row that passes the display filters as listed at `now`, raising nothing.
    fn seed_listed(&mut self, now: Instant) {
        for t in &self.latest {
            if self.filters.matches(t) {
                self.listed.insert(t.symbol.clone(), now);
            }
        }
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        let new_filters = Filters::from_config(cfg);
        let filters_changed = new_filters != self.filters;
        self.filters = new_filters;
        self.th = Thresholds::from_config(cfg);
        self.series.set_horizon(series_horizon(&self.th));
        while self.alerts.len() > self.th.max_alerts {
            self.alerts.pop_front();
        }
        // New filters apply at once: nothing lingers from the old ones.
        self.failing_since.clear();
        self.passing = self.latest.iter().filter(|t| self.shown(t)).map(|t| t.symbol.clone()).collect();
        // Whatever the new filters let through is the list as of now, not a flood of new symbols.
        if filters_changed {
            self.seed_listed(Instant::now());
        }
    }

    /// Passes the display filters now (blacklisted rows are listed separately, never here).
    fn shown(&self, t: &Ticker) -> bool {
        !self.blacklist.contains(&t.symbol) && self.filters.matches(t)
    }

    pub fn alerts(&self) -> &VecDeque<Alert> {
        &self.alerts
    }

    pub fn toggle_blacklist(&mut self, sym: &str) {
        if !self.blacklist.remove(sym) {
            self.blacklist.insert(sym.to_string());
        }
    }

    /// Down/up tick counts per rate window, ordered as RATE_WINDOWS.
    pub fn tick_rates(&self, now: Instant) -> [(u32, u32); 4] {
        let mut out = [(0u32, 0u32); 4];
        for &(at, up) in &self.ticks {
            let age = now.saturating_duration_since(at).as_secs();
            for (i, &(_, secs)) in RATE_WINDOWS.iter().enumerate() {
                if age < secs {
                    if up { out[i].1 += 1 } else { out[i].0 += 1 }
                }
            }
        }
        out
    }

    pub fn process(&mut self, tickers: Vec<Ticker>, sort_key: &str, sort_desc: bool, now: Instant) -> Output {
        let wall = chrono::Local::now();
        let epoch = wall.timestamp();
        let th = self.th;
        let mut latest = Vec::with_capacity(tickers.len());

        self.series.set_members(tickers.iter().map(|t| t.ticker_id), now);
        self.series.expire(now);
        let wall_ms = wall.timestamp_millis();

        for mut t in tickers {
            let overlaid = self.overlay_live(&mut t, now);
            // A row still carrying a fresh stream quote adds nothing; otherwise REST feeds the
            // series when the stream is down. REST has no volume of 0 for a trading ticker, so 0 means missing.
            if self.rest_feeds_series && !overlaid {
                self.series.ingest(Obs {
                    id: t.ticker_id,
                    src: Source::Rest,
                    ms: wall_ms,
                    seq: 0,
                    px: Some(t.price),
                    vol: u64::try_from(t.volume).ok().filter(|v| *v > 0),
                    at: now,
                });
            }
            let is_bl = self.blacklist.contains(&t.symbol);
            let shown = is_bl || self.filters.matches(&t);

            (t.vol_rate, t.vol_partial, t.vol_stale) =
                vol_m(&self.series, t.ticker_id, now, th.vol_m_window, self.feed_stale);

            // NEW and SETUP are evaluated on the recompute tick.
            if let Some(prev) = self.last.get(&t.symbol) {
                if shown && t.price > prev.price {
                    self.ticks.push_back((now, true));
                } else if shown && t.price < prev.price {
                    self.ticks.push_back((now, false));
                }
            }

            self.record_sample(&t.symbol, epoch, t.price, t.volume);
            self.last.insert(t.symbol.clone(), t.clone());
            latest.push(t);
        }

        let horizon = Duration::from_secs(RATE_WINDOWS[0].1);
        while self.ticks.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) >= horizon) {
            self.ticks.pop_front();
        }

        while self.alerts.len() > th.max_alerts {
            self.alerts.pop_front();
        }

        self.index = latest.iter().enumerate().filter(|(_, t)| t.ticker_id != 0).map(|(i, t)| (t.ticker_id, i)).collect();
        self.live.retain(|id, _| self.index.contains_key(id));
        self.latest = latest;

        // A fetch is rare and changes membership: rank and filter at once instead of at the next tick.
        let new_alerts = self.recompute_at(now, true);
        let mut out = self.reapply(sort_key, sort_desc, now);
        out.new_alerts = new_alerts;
        out
    }

    /// The recompute tick: re-ranks the full board (Mov), moves rows into or out of the filter
    /// grace and raises NEW and SETUP alerts from the stream-fed rows. Stream quotes only change
    /// data; the display follows on this tick. Returns the number of alerts raised.
    pub fn recompute(&mut self, now: Instant) -> usize {
        self.recompute_at(now, false)
    }

    /// `fetched`: the list was just replaced by a fetch (the first one after a reseed seeds NEW).
    fn recompute_at(&mut self, now: Instant, fetched: bool) -> usize {
        self.rank_board(now);
        self.update_display(now);
        let raised = self.evaluate_new(now, fetched) + self.evaluate_setups(now);
        let hold = self.badge_hold();
        self.fired.retain(|_, f| {
            for at in f.iter_mut() {
                if at.is_some_and(|a| now.saturating_duration_since(a) >= hold) {
                    *at = None;
                }
            }
            f.iter().any(Option::is_some)
        });
        raised
    }

    /// NEW: a row on the list (passing the display filters) now that was not on it in the last
    /// `NEW_QUIET`. While a reseed is pending (startup, session change) ticks do nothing and the
    /// next fetched list is only recorded. Blacklisted rows are recorded but never alert.
    fn evaluate_new(&mut self, now: Instant, fetched: bool) -> usize {
        if self.reseed {
            if fetched {
                self.reseed = false;
                self.seed_listed(now);
            }
            return 0;
        }
        let mut hits = Vec::new();
        for t in &self.latest {
            if !self.filters.matches(t) {
                continue;
            }
            let gone = self.listed.get(&t.symbol).is_none_or(|&at| now.saturating_duration_since(at) >= NEW_QUIET);
            if gone && !self.blacklist.contains(&t.symbol) {
                hits.push(t.clone());
            }
        }
        self.seed_listed(now);
        self.listed.retain(|_, at| now.saturating_duration_since(*at) < LISTED_KEEP);
        let clock = chrono::Local::now().format("%H:%M:%S").to_string();
        let mut raised = 0;
        for t in hits {
            let base = self.setup_base(t.ticker_id, now);
            if self.raise(AlertKind::New, &t, base, now, &clock) {
                raised += 1;
            }
        }
        raised
    }

    /// SETUP on every shown, eligible row at its current price and volume. The cooldown stops a
    /// row that keeps meeting the limits from being raised again at each tick.
    fn evaluate_setups(&mut self, now: Instant) -> usize {
        // Without fresh data the look-back window only slides over old samples; nothing is new.
        if self.feed_stale {
            return 0;
        }
        let th = self.th;
        let mut hits = Vec::new();
        for t in &self.latest {
            let eligible = t.volume >= th.min_volume_for_alert && t.price >= th.min_price_for_alert;
            if !eligible || !self.shown(t) || !self.can_fire(&t.symbol, AlertKind::Setup, now) {
                continue;
            }
            let base = self.setup_base(t.ticker_id, now);
            if setup_met(&th, base, t.volume, t.price) {
                hits.push((t.clone(), base));
            }
        }
        if hits.is_empty() {
            return 0;
        }
        let clock = chrono::Local::now().format("%H:%M:%S").to_string();
        let mut raised = 0;
        for (t, base) in hits {
            if self.raise(AlertKind::Setup, &t, base, now, &clock) {
                raised += 1;
            }
        }
        raised
    }

    /// How long an alert's Status label is held: the cooldown, but at least `BADGE_HOLD`.
    fn badge_hold(&self) -> Duration {
        self.th.alert_cooldown.max(BADGE_HOLD)
    }

    /// Whether the cooldown of `kind` for `sym` has run out.
    fn can_fire(&self, sym: &str, kind: AlertKind, now: Instant) -> bool {
        let last = self.fired.get(sym).and_then(|f| f[kind.slot()]);
        last.is_none_or(|at| now.saturating_duration_since(at) >= self.th.alert_cooldown)
    }

    /// Logs an alert unless the symbol is blacklisted or still cooling down for this kind, and
    /// starts the cooldown and the Status badge. `base` is the value one SETUP window ago.
    fn raise(&mut self, kind: AlertKind, t: &Ticker, base: Option<(u64, f64)>, now: Instant, clock: &str) -> bool {
        if self.blacklist.contains(&t.symbol) || !self.can_fire(&t.symbol, kind, now) {
            return false;
        }
        self.fired.entry(t.symbol.clone()).or_default()[kind.slot()] = Some(now);
        self.alerts.push_back(Alert {
            at: now,
            clock: clock.to_string(),
            symbol: t.symbol.clone(),
            kind,
            prev_vol: base.map_or(-1, |b| b.0 as i64),
            curr_vol: t.volume,
            prev_price: base.map_or(-1.0, |b| b.1),
            curr_price: t.price,
            change_ratio: t.change_ratio,
        });
        while self.alerts.len() > self.th.max_alerts {
            self.alerts.pop_front();
        }
        true
    }

    /// The alert label shown in the Status column: the highest-priority kind still held.
    fn badge(&self, sym: &str, now: Instant) -> Option<AlertKind> {
        let fired = self.fired.get(sym)?;
        let hold = self.badge_hold();
        BADGE_PRIORITY
            .into_iter()
            .find(|k| fired[k.slot()].is_some_and(|at| now.saturating_duration_since(at) < hold))
    }

    /// Mov: rank change on the full board ranked by % change, over `rank_interval`. Independent of
    /// the user's sort and filters; blacklisted symbols are left out of both boards.
    fn rank_board(&mut self, now: Instant) {
        if self.latest.is_empty() {
            return;
        }
        let mut board: Vec<&Ticker> = self.latest.iter().collect();
        board.sort_by(|a, b| board_order(a, b));
        let order: Vec<String> = board.iter().map(|t| t.symbol.clone()).collect();

        // Baseline: the board in force `rank_interval` ago, if we have been running that long.
        let cutoff = now.checked_sub(self.th.rank_interval);
        let old = cutoff.and_then(|c| self.rank_ring.iter().rev().find(|(at, _)| *at <= c)).map(|(_, o)| o.as_slice());
        let movs = movements(old, &order, &self.blacklist);
        for t in &mut self.latest {
            t.mov = movs.get(t.symbol.as_str()).copied().unwrap_or_default();
        }

        if self.rank_ring.back().is_none_or(|(_, last)| *last != order) {
            self.rank_ring.push_back((now, order));
        }
        // Keep exactly one entry at or before the cutoff: it is the baseline.
        if let Some(c) = cutoff {
            while self.rank_ring.get(1).is_some_and(|(at, _)| *at <= c) {
                self.rank_ring.pop_front();
            }
        }
    }

    /// Tracks which symbols pass the display filters; a symbol that stops passing starts its grace.
    fn update_display(&mut self, now: Instant) {
        let mut passing = HashSet::new();
        let mut failing = HashMap::new();
        for t in &self.latest {
            if self.blacklist.contains(&t.symbol) {
                continue;
            }
            if self.filters.matches(t) {
                passing.insert(t.symbol.clone());
            } else if let Some(&since) = self.failing_since.get(&t.symbol) {
                failing.insert(t.symbol.clone(), since);
            } else if self.passing.contains(&t.symbol) {
                failing.insert(t.symbol.clone(), now);
            }
        }
        self.passing = passing;
        self.failing_since = failing;
    }

    /// Whether a row is on the board at `now`, and if so whether it is dimmed (filter grace).
    fn display_state(&self, t: &Ticker, now: Instant) -> Option<bool> {
        if self.filters.matches(t) {
            return Some(false);
        }
        match self.failing_since.get(&t.symbol) {
            Some(&since) => (now.saturating_duration_since(since) < FILTER_GRACE).then_some(true),
            // Failing since a moment ago; the next recompute starts its grace.
            None => self.passing.contains(&t.symbol).then_some(true),
        }
    }

    /// Re-filter and re-sort the last processed tickers with the current filters, blacklist and
    /// sort. Records no history, ticks, alerts or rank samples and leaves Mov as last computed.
    pub fn reapply(&self, sort_key: &str, sort_desc: bool, now: Instant) -> Output {
        let mut active = Vec::new();
        let mut blacklisted = Vec::new();
        for t in &self.latest {
            if self.blacklist.contains(&t.symbol) {
                blacklisted.push(t.clone());
            } else if let Some(dim) = self.display_state(t, now) {
                let status = self.badge(&t.symbol, now).map_or_else(String::new, |k| k.label().to_string());
                active.push(Ticker { dim, status, ..t.clone() });
            }
        }
        active.sort_by(|a, b| ordered(a, b, sort_key, sort_desc));
        active.truncate(MAX_ROWS);
        Output { active, blacklisted, new_alerts: 0 }
    }
}

/// Full-board order: % change, then volume (both descending), then symbol.
fn board_order(a: &Ticker, b: &Ticker) -> Ordering {
    b.change_ratio
        .partial_cmp(&a.change_ratio)
        .unwrap_or(Ordering::Equal)
        .then(b.volume.cmp(&a.volume))
        .then_with(|| a.symbol.cmp(&b.symbol))
}

/// Rank movement of every symbol of `new` against `old` (both best first), blacklisted symbols
/// removed from both. No `old` board gives blanks; a symbol missing from it is a new entrant.
fn movements<'a>(old: Option<&[String]>, new: &'a [String], blacklist: &BTreeSet<String>) -> HashMap<&'a str, Mov> {
    let old_rank: Option<HashMap<&str, i32>> = old.map(|o| {
        o.iter().filter(|s| !blacklist.contains(*s)).enumerate().map(|(i, s)| (s.as_str(), i as i32 + 1)).collect()
    });
    new.iter()
        .filter(|s| !blacklist.contains(*s))
        .enumerate()
        .map(|(i, s)| {
            let mov = match &old_rank {
                None => Mov::None,
                Some(r) => r.get(s.as_str()).map_or(Mov::New, |&was| Mov::Moved(was - (i as i32 + 1))),
            };
            (s.as_str(), mov)
        })
        .collect()
}

/// Series history to keep: the longest look-back window (Vol/m or SETUP) plus a margin.
fn series_horizon(th: &Thresholds) -> Duration {
    th.vol_m_window.max(th.setup_window) + SERIES_MARGIN
}

/// SETUP: versus the value `setup_window` ago (`base`: cumulative volume, price), the volume ratio
/// is at least `setup_vol_ratio` and the price change is at least `setup_price_pct` percent.
/// Both limits are inclusive. No baseline, or a baseline of 0, never triggers.
fn setup_met(th: &Thresholds, base: Option<(u64, f64)>, vol: i64, price: f64) -> bool {
    // Slack for float noise, so a move of exactly 10% off 1.00 counts as 10%.
    const EPS: f64 = 1e-9;
    let Some((base_vol, base_px)) = base else { return false };
    if base_vol == 0 || !(base_px > 0.0) {
        return false;
    }
    let ratio = vol as f64 / base_vol as f64;
    let pct = (price - base_px) / base_px * 100.0;
    ratio + EPS >= th.setup_vol_ratio && pct + EPS >= th.setup_price_pct
}

/// Vol/m of one ticker at `now`: (shares per minute, window only partly covered, feed stale).
///
/// The window ends at `now` and starts at `now - window`, or at the first point if the series is
/// younger. Under 30 s of coverage, or no series, gives no rate; a stale feed hides it.
fn vol_m(series: &SeriesStore, id: u64, now: Instant, window: Duration, stale: bool) -> (Option<f64>, bool, bool) {
    if stale {
        return (None, false, true);
    }
    let window = window.max(Duration::from_secs(MIN_VOL_M_WINDOW_SECS));
    let Some(first) = series.oldest(id) else { return (None, false, false) };
    let start = now.checked_sub(window).map_or(first.t, |s| s.max(first.t));
    let covered = now.saturating_duration_since(start);
    if covered < Duration::from_secs(MIN_VOL_M_WINDOW_SECS) {
        return (None, false, false);
    }
    let (Some(end), Some(begin)) = (series.vol_at(id, now), series.vol_at(id, start)) else {
        return (None, false, false);
    };
    let rate = end.saturating_sub(begin) as f64 * 60.0 / covered.as_secs_f64();
    (Some(rate), covered < window, false)
}

/// Change ratio versus the previous close, when that is known.
fn change_from(prev_close: f64, price: f64) -> Option<f64> {
    (prev_close > 0.0).then(|| price / prev_close - 1.0)
}

/// Places gained on the board; `None` for blank and new-entrant rows.
fn mov_places(t: &Ticker) -> Option<i32> {
    match t.mov {
        Mov::Moved(n) => Some(n),
        Mov::None | Mov::New => None,
    }
}

/// Sort order for `key`. Rows without a Vol/m rate, or without a Mov rank change (blank or a
/// new entrant), go last in both directions.
fn ordered(a: &Ticker, b: &Ticker, key: &str, desc: bool) -> Ordering {
    let blanks = match key {
        "volrate" => Some((a.vol_rate.is_some(), b.vol_rate.is_some())),
        "mov" => Some((mov_places(a).is_some(), mov_places(b).is_some())),
        _ => None,
    };
    match blanks {
        Some((false, true)) => return Ordering::Greater,
        Some((true, false)) => return Ordering::Less,
        Some((false, false)) => return a.symbol.cmp(&b.symbol),
        _ => {}
    }
    let o = compare(a, b, key);
    if desc { o.reverse() } else { o }
}

pub fn compare(a: &Ticker, b: &Ticker, key: &str) -> Ordering {
    let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    let primary = match key {
        "mov" => mov_places(a).unwrap_or(0).cmp(&mov_places(b).unwrap_or(0)),
        "symbol" => a.symbol.cmp(&b.symbol),
        "price" => f(a.price, b.price),
        "change" => f(a.change_ratio, b.change_ratio),
        "volume" => a.volume.cmp(&b.volume),
        "mcap" => f(a.market_value, b.market_value),
        "volrate" => f(a.vol_rate.unwrap_or(0.0), b.vol_rate.unwrap_or(0.0)),
        "status" => a.status.cmp(&b.status),
        _ => f(a.change_ratio, b.change_ratio).then(a.volume.cmp(&b.volume)),
    };
    primary.then_with(|| a.symbol.cmp(&b.symbol))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tk(sym: &str, price: f64, vol: i64, chg: f64, mcap: f64) -> Ticker {
        Ticker { symbol: sym.into(), price, volume: vol, change_ratio: chg, market_value: mcap, ..Default::default() }
    }

    fn engine() -> Engine {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price_for_alert = 0.0;
        cfg.min_volume_for_alert = 0;
        Engine::new(&cfg)
    }

    #[test]
    fn filters_each_bound() {
        let t = tk("A", 5.0, 1000, 0.25, 1e6);
        assert!(Filters::default().matches(&t));
        assert!(!Filters { min_price: 6.0, ..Default::default() }.matches(&t));
        assert!(!Filters { max_price: 4.0, ..Default::default() }.matches(&t));
        assert!(!Filters { min_volume: 2000, ..Default::default() }.matches(&t));
        assert!(!Filters { max_volume: 500, ..Default::default() }.matches(&t));
        assert!(!Filters { min_change_pct: 30.0, ..Default::default() }.matches(&t));
        assert!(!Filters { max_change_pct: 20.0, ..Default::default() }.matches(&t));
        assert!(!Filters { min_mcap: 2e6, ..Default::default() }.matches(&t));
        assert!(!Filters { max_mcap: 5e5, ..Default::default() }.matches(&t));
        assert!(Filters { min_price: 1.0, max_price: 10.0, min_change_pct: 20.0, ..Default::default() }.matches(&t));
        // Unknown market cap is excluded when a cap filter is active
        assert!(!Filters { max_mcap: 1e9, ..Default::default() }.matches(&tk("B", 1.0, 1, 0.1, 0.0)));
    }

    /// Engine with SETUP at 300 s / 2.2x / +10% and no eligibility gates.
    fn setup_engine() -> Engine {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price_for_alert = 0.0;
        cfg.min_volume_for_alert = 0;
        cfg.setup_window_sec = 300;
        cfg.setup_vol_ratio = 2.2;
        cfg.setup_price_pct = 10.0;
        Engine::new(&cfg)
    }

    fn setup_row(e: &mut Engine, t0: Instant, secs: u64, price: f64, vol: i64) -> Output {
        let t = Ticker { ticker_id: 1, ..tk("A", price, vol, 0.3, 0.0) };
        e.process(vec![t], "change", true, t0 + Duration::from_secs(secs))
    }

    /// The first fetch (an empty list) only seeds NEW; rows listed after it are new.
    fn seed_empty(e: &mut Engine, now: Instant) {
        e.process(vec![], "change", true, now);
    }

    fn kinds(e: &Engine) -> Vec<AlertKind> {
        e.alerts().iter().map(|a| a.kind).collect()
    }

    #[test]
    fn setup_fires_exactly_at_both_limits() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        let out = setup_row(&mut e, t0, 0, 1.0, 1000);
        assert_eq!(out.active[0].status, "");
        // exactly 2.2x and exactly +10%, 300 s later
        let out = setup_row(&mut e, t0, 300, 1.10, 2200);
        assert_eq!(out.active[0].status, "SETUP");
        let a = e.alerts().back().unwrap();
        assert_eq!(a.kind, AlertKind::Setup);
        assert_eq!((a.prev_vol, a.curr_vol, a.prev_price, a.curr_price), (1000, 2200, 1.0, 1.10));
        assert_eq!(a.detail(), "vol 2.2x  +10.0%");
    }

    #[test]
    fn setup_needs_both_limits() {
        for (price, vol) in [(1.099, 2200), (1.10, 2199), (1.0, 9000), (3.0, 1000)] {
            let mut e = setup_engine();
            let t0 = Instant::now();
            setup_row(&mut e, t0, 0, 1.0, 1000);
            let out = setup_row(&mut e, t0, 300, price, vol);
            assert_eq!(out.new_alerts, 0, "price {price} vol {vol}");
            assert!(kinds(&e).is_empty());
        }
    }

    #[test]
    fn spike_and_pump_are_never_raised() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        let rows = |s_vol: i64, p_px: f64| {
            vec![
                Ticker { ticker_id: 1, ..tk("S", 1.0, s_vol, 0.1, 0.0) },
                Ticker { ticker_id: 2, ..tk("P", p_px, 1000, 0.5, 0.0) },
            ]
        };
        e.process(rows(1000, 1.0), "change", true, t0);
        // S: volume x5, flat price (a SPIKE before). P: price +50%, flat volume (a PUMP before).
        let out = e.process(rows(5000, 1.5), "symbol", false, t0 + Duration::from_secs(5));
        assert_eq!(out.new_alerts, 0);
        // nothing was raised (the first list only seeds NEW)
        assert!(out.active.iter().all(|t| t.status.is_empty()));
        assert!(kinds(&e).is_empty());
        // the tick counter still follows the price move
        assert_eq!(e.tick_rates(t0 + Duration::from_secs(6))[3], (0, 1));
    }

    #[test]
    fn setup_with_short_history_compares_to_the_oldest_sample() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        setup_row(&mut e, t0, 0, 1.0, 1000);
        // only 20 s of history, window is 300 s
        let out = setup_row(&mut e, t0, 20, 1.2, 3000);
        assert_eq!(out.active[0].status, "SETUP");
    }

    #[test]
    fn setup_without_a_series_never_fires() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        // ticker id 0 has no series
        let z = |p: f64, v: i64| vec![tk("Z", p, v, 0.3, 0.0)];
        e.process(z(1.0, 1000), "change", true, t0);
        let out = e.process(z(2.0, 9000), "change", true, t0 + Duration::from_secs(300));
        assert_eq!(out.new_alerts, 0);
        // a ticker's very first sample is its own baseline
        let mut e = setup_engine();
        assert_eq!(setup_row(&mut e, t0, 0, 5.0, 9000).new_alerts, 0);
        assert!(kinds(&e).is_empty());
    }

    #[test]
    fn cleared_series_needs_new_history() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        setup_row(&mut e, t0, 0, 1.0, 1000);
        e.on_stream_reconnect();
        // The jump is real, but the baseline is gone: the new series starts at this sample.
        let out = setup_row(&mut e, t0, 300, 1.5, 5000);
        assert_eq!(out.new_alerts, 0);
        // History rebuilt after the clear works again.
        let out = setup_row(&mut e, t0, 600, 1.65, 11_000);
        assert_eq!(out.active[0].status, "SETUP");
    }

    #[test]
    fn setup_baseline_is_the_value_one_window_ago_not_the_oldest() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        setup_row(&mut e, t0, 0, 1.0, 1000);
        setup_row(&mut e, t0, 100, 1.5, 5000);
        // Against t=0 this is 9x and +65%; against the point at or before t=100 (300 s ago) it is
        // 1.8x and +10%: not a SETUP.
        let out = setup_row(&mut e, t0, 400, 1.65, 9000);
        assert_eq!(out.new_alerts, 0);
        // 11000 / 5000 = 2.2x and 1.65 / 1.5 = +10%: SETUP
        let out = setup_row(&mut e, t0, 400, 1.65, 11_000);
        assert_eq!(out.active[0].status, "SETUP");
    }

    #[test]
    fn setup_respects_the_alert_gates() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_volume_for_alert = 5000;
        cfg.min_price_for_alert = 1.0;
        let t0 = Instant::now();
        // volume below the gate, price above
        let mut e = Engine::new(&cfg);
        setup_row(&mut e, t0, 0, 1.0, 1000);
        assert_eq!(setup_row(&mut e, t0, 300, 1.2, 4999).new_alerts, 0);
        // price below the gate (volume above)
        let mut e = Engine::new(&cfg);
        setup_row(&mut e, t0, 0, 0.5, 10_000);
        assert_eq!(setup_row(&mut e, t0, 300, 0.99, 30_000).new_alerts, 0);
        // exactly at both gates
        let mut e = Engine::new(&cfg);
        setup_row(&mut e, t0, 0, 0.5, 2000);
        assert_eq!(setup_row(&mut e, t0, 300, 1.0, 5000).new_alerts, 1);
    }

    #[test]
    fn setup_reads_the_stream_series_and_live_quote() {
        let (mut e, t0) = live_engine();
        e.set_rest_feeds_series(false);
        let at = |mut u: Update, s: u64| {
            u.recv = t0 + Duration::from_secs(s);
            u
        };
        e.apply_quotes(&[at(upd(7, 1000, 1, 1.0, 1000), 10)], Session::Reg, t0 + Duration::from_secs(10));
        e.apply_quotes(&[at(upd(7, 301_000, 2, 1.10, 2200), 310)], Session::Reg, t0 + Duration::from_secs(310));
        // a stale REST row; the fresh stream quote overrides it
        let rest = Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.0, 1000, 0.0, 5e6) };
        let out = e.process(vec![rest], "change", true, t0 + Duration::from_secs(311));
        assert_eq!(out.active[0].status, "SETUP");
        assert_eq!(e.alerts().back().unwrap().prev_vol, 1000);
    }

    /// Status of the only row as the table would show it at `now`.
    fn status_at(e: &Engine, now: Instant) -> String {
        e.reapply("change", true, now).active[0].status.clone()
    }

    /// Lists row "A" (id 1) over REST at `t0`, then lets the stream bring it to exactly 2.2x and
    /// +10% of 300 s ago; no REST fetch follows. Returns the time of that last quote, 310 s on.
    fn stream_setup_hit(e: &mut Engine, t0: Instant) -> Instant {
        setup_row(e, t0, 0, 1.0, 1000);
        e.set_rest_feeds_series(false);
        for (s, ms, seq, px, vol) in [(10, 10_000, 1, 1.0, 1000), (310, 310_000, 2, 1.10, 2200)] {
            let mut u = upd(1, ms, seq, px, vol);
            u.recv = t0 + Duration::from_secs(s);
            e.apply_quotes(&[u], Session::Reg, t0 + Duration::from_secs(s));
        }
        t0 + Duration::from_secs(310)
    }

    #[test]
    fn setup_is_raised_by_the_tick_once_per_cooldown() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        stream_setup_hit(&mut e, t0);
        // the stream data alone raises nothing; the tick does
        assert!(kinds(&e).is_empty());

        assert_eq!(e.recompute(at(310)), 1);
        assert_eq!(kinds(&e), vec![AlertKind::Setup]);
        assert_eq!(status_at(&e, at(310)), "SETUP");
        let a = e.alerts().back().unwrap();
        assert_eq!((a.at, a.prev_vol, a.curr_vol), (at(310), 1000, 2200));

        // the limits still hold on every following tick, but the cooldown keeps it to one alert
        for s in (311..430).step_by(3) {
            assert_eq!(e.recompute(at(s)), 0, "second {s}");
        }
        assert_eq!(e.recompute(at(429)), 0);
        assert_eq!(kinds(&e).len(), 1);
        // once the cooldown (120 s) is over and the condition still holds, it is raised again
        assert_eq!(e.recompute(at(430)), 1);
        assert_eq!(kinds(&e).len(), 2);
    }

    #[test]
    fn setup_tick_skips_blacklisted_filtered_and_stale_rows() {
        let t0 = Instant::now();
        let mut e = setup_engine();
        e.toggle_blacklist("A");
        let now = stream_setup_hit(&mut e, t0);
        assert_eq!(e.recompute(now), 0);

        let mut e = setup_engine();
        e.filters.min_price = 5.0;
        let now = stream_setup_hit(&mut e, t0);
        assert_eq!(e.recompute(now), 0);

        let mut e = setup_engine();
        let now = stream_setup_hit(&mut e, t0);
        e.set_feed_stale(true);
        assert_eq!(e.recompute(now), 0);
        e.set_feed_stale(false);
        assert_eq!(e.recompute(now), 1);
    }

    #[test]
    fn cooldown_is_per_symbol_and_kind() {
        let mut e = engine();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let a = tk("A", 1.0, 1000, 0.1, 0.0);
        let b = tk("B", 1.0, 1000, 0.1, 0.0);
        assert!(e.raise(AlertKind::Setup, &a, None, at(0), "00:00:00"));
        // same symbol and kind: held back for the whole cooldown
        assert!(!e.raise(AlertKind::Setup, &a, None, at(1), "00:00:01"));
        assert!(!e.raise(AlertKind::Setup, &a, None, at(119), "00:01:59"));
        // another kind of the same symbol, and the same kind of another symbol, are independent
        assert!(e.raise(AlertKind::New, &a, None, at(2), "00:00:02"));
        assert!(e.raise(AlertKind::Setup, &b, None, at(3), "00:00:03"));
        assert_eq!(e.alerts().len(), 3);
        // the cooldown counts from the last raise
        assert!(e.raise(AlertKind::Setup, &a, None, at(120), "00:02:00"));
        assert!(!e.raise(AlertKind::Setup, &a, None, at(239), "00:03:59"));
        assert!(e.raise(AlertKind::Setup, &a, None, at(240), "00:04:00"));
    }

    #[test]
    fn cooldown_follows_the_configured_seconds() {
        let mut e = engine();
        e.apply_config(&Config { alert_cooldown_sec: 10, max_market_cap: 0.0, ..Config::default() });
        let t0 = Instant::now();
        let a = tk("A", 1.0, 1000, 0.1, 0.0);
        assert!(e.raise(AlertKind::Setup, &a, None, t0, ""));
        assert!(!e.raise(AlertKind::Setup, &a, None, t0 + Duration::from_secs(9), ""));
        assert!(e.raise(AlertKind::Setup, &a, None, t0 + Duration::from_secs(10), ""));
    }

    #[test]
    fn badge_is_held_for_the_cooldown() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        seed_empty(&mut e, t0);
        setup_row(&mut e, t0, 0, 1.0, 1000);
        // NEW at 0 s, default cooldown 120 s: still shown until 120 s, gone at 120 s
        assert_eq!(status_at(&e, at(0)), "NEW");
        assert_eq!(status_at(&e, at(61)), "NEW");
        assert_eq!(status_at(&e, at(119)), "NEW");
        assert_eq!(status_at(&e, at(120)), "");
        // the badge survives the table being rebuilt by a fetch that raises nothing
        let out = setup_row(&mut e, t0, 100, 1.0, 1000);
        assert_eq!(out.new_alerts, 0);
        assert_eq!(out.active[0].status, "NEW");
    }

    #[test]
    fn badge_is_held_at_least_60_s_when_the_cooldown_is_shorter() {
        let mut e = engine();
        e.apply_config(&Config { alert_cooldown_sec: 20, max_market_cap: 0.0, ..Config::default() });
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        seed_empty(&mut e, t0);
        e.process(vec![tk("A", 1.0, 1000, 0.1, 0.0)], "change", true, t0);
        assert_eq!(status_at(&e, at(59)), "NEW");
        assert_eq!(status_at(&e, at(60)), "");
        // the cooldown itself is over long before the badge goes
        assert!(e.can_fire("A", AlertKind::New, at(20)));
    }

    #[test]
    fn badge_shows_the_highest_priority_kind_and_expiry_cleans_up() {
        let mut e = engine();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        seed_empty(&mut e, t0);
        e.process(vec![Ticker { ticker_id: 1, ..tk("A", 1.0, 1000, 0.1, 0.0) }], "change", true, t0);
        assert_eq!(status_at(&e, at(1)), "NEW");
        let a = tk("A", 1.0, 1000, 0.1, 0.0);
        assert!(e.raise(AlertKind::Setup, &a, None, at(30), ""));
        assert_eq!(status_at(&e, at(31)), "SETUP");
        // NEW (from 0 s) is covered by SETUP while both are held; SETUP outlasts it (30 s + 120 s)
        e.recompute(at(121));
        assert_eq!(status_at(&e, at(121)), "SETUP");
        e.recompute(at(149));
        assert_eq!(status_at(&e, at(149)), "SETUP");
        e.recompute(at(150));
        assert_eq!(status_at(&e, at(150)), "");
        assert!(e.fired.is_empty());
    }

    #[test]
    fn blacklisted_symbols_get_no_badge() {
        let mut e = engine();
        e.toggle_blacklist("A");
        let a = tk("A", 1.0, 1000, 0.1, 0.0);
        assert!(!e.raise(AlertKind::Setup, &a, None, Instant::now(), ""));
        assert!(e.alerts().is_empty() && e.fired.is_empty());
    }

    #[test]
    fn setup_met_edges() {
        let th = Thresholds::from_config(&Config::default());
        assert!(setup_met(&th, Some((1000, 1.0)), 2200, 1.10));
        assert!(!setup_met(&th, Some((1000, 1.0)), 2199, 1.10));
        assert!(!setup_met(&th, Some((1000, 1.0)), 2200, 1.0999));
        assert!(!setup_met(&th, None, 2200, 1.10));
        assert!(!setup_met(&th, Some((0, 1.0)), 2200, 1.10));
        assert!(!setup_met(&th, Some((1000, 0.0)), 2200, 1.10));
        // a falling price never counts
        assert!(!setup_met(&th, Some((1000, 1.0)), 5000, 0.5));
    }

    #[test]
    fn config_change_moves_the_setup_limits() {
        let mut e = setup_engine();
        let t0 = Instant::now();
        setup_row(&mut e, t0, 0, 1.0, 1000);
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price_for_alert = 0.0;
        cfg.min_volume_for_alert = 0;
        cfg.setup_window_sec = 60;
        cfg.setup_vol_ratio = 1.5;
        cfg.setup_price_pct = 5.0;
        e.apply_config(&cfg);
        let out = setup_row(&mut e, t0, 60, 1.05, 1500);
        assert_eq!(out.active[0].status, "SETUP");
    }

    /// Engine whose first fetch (an empty list) has seeded NEW; rows listed after it are new.
    fn seeded_engine(t0: Instant) -> Engine {
        let mut e = setup_engine();
        seed_empty(&mut e, t0);
        e
    }

    /// Lists the given symbols (ids 1..) over REST at `secs` after `t0`.
    fn list(e: &mut Engine, t0: Instant, secs: u64, syms: &[&str]) -> Output {
        let rows = syms.iter().enumerate().map(|(i, s)| Ticker { ticker_id: i as u64 + 1, ..tk(s, 1.0, 1000, 0.1, 0.0) });
        e.process(rows.collect(), "change", true, t0 + Duration::from_secs(secs))
    }

    #[test]
    fn startup_list_only_seeds_new() {
        let t0 = Instant::now();
        let mut e = setup_engine();
        let out = list(&mut e, t0, 0, &["A", "B", "C"]);
        assert_eq!(out.new_alerts, 0);
        assert!(kinds(&e).is_empty());
        assert!(out.active.iter().all(|t| t.status.is_empty()));
        // the same list again, and ticks in between, stay silent
        assert_eq!(e.recompute(t0 + Duration::from_secs(1)), 0);
        assert_eq!(list(&mut e, t0, 5, &["A", "B", "C"]).new_alerts, 0);
        // a symbol that joins afterwards is NEW
        let out = list(&mut e, t0, 10, &["A", "B", "C", "D"]);
        assert_eq!(out.new_alerts, 1);
        assert_eq!(kinds(&e), vec![AlertKind::New]);
        assert_eq!(e.alerts().back().unwrap().symbol, "D");
        assert_eq!(out.active.iter().find(|t| t.symbol == "D").unwrap().status, "NEW");
    }

    #[test]
    fn leaving_and_returning_within_30_min_is_not_new() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        assert_eq!(list(&mut e, t0, 0, &["A", "B"]).new_alerts, 2);
        // B leaves the list; it comes back 29:59 after it was last listed
        list(&mut e, t0, 60, &["A"]);
        assert_eq!(list(&mut e, t0, 1799, &["A", "B"]).new_alerts, 0);
        assert_eq!(kinds(&e).len(), 2);
    }

    #[test]
    fn returning_after_30_min_is_new() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        list(&mut e, t0, 0, &["A", "B"]);
        list(&mut e, t0, 60, &["A"]);
        // B was last listed at 0 s: back at 1799 s is still too soon, 1800 s is NEW
        assert_eq!(list(&mut e, t0, 1799, &["A"]).new_alerts, 0);
        let out = list(&mut e, t0, 1800, &["A", "B"]);
        assert_eq!(out.new_alerts, 1);
        assert_eq!(e.alerts().back().unwrap().symbol, "B");
        assert_eq!(kinds(&e).len(), 3);
    }

    #[test]
    fn staying_listed_keeps_the_clock_running() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        list(&mut e, t0, 0, &["A"]);
        // listed all the time, long past 30 min: never NEW again
        for s in (60..=3600).step_by(60) {
            assert_eq!(list(&mut e, t0, s, &["A"]).new_alerts, 0, "second {s}");
        }
        assert_eq!(kinds(&e).len(), 1);
    }

    #[test]
    fn session_change_reseeds_silently() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        list(&mut e, t0, 0, &["A"]);
        e.on_session_change();
        // ticks before the next fetch raise nothing and do not use up the reseed
        assert_eq!(e.recompute(t0 + Duration::from_secs(1)), 0);
        // a brand new board arrives: seeded, no alerts
        let out = list(&mut e, t0, 10, &["A", "X", "Y", "Z"]);
        assert_eq!(out.new_alerts, 0);
        assert_eq!(kinds(&e).len(), 1);
        // afterwards NEW works as before
        assert_eq!(list(&mut e, t0, 20, &["A", "X", "Y", "Z", "W"]).new_alerts, 1);
        assert_eq!(e.alerts().back().unwrap().symbol, "W");
    }

    #[test]
    fn filter_change_reseeds_silently() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        let rows = |e: &mut Engine, s: u64, extra: bool| {
            let mut v = vec![
                Ticker { ticker_id: 1, ..tk("A", 1.0, 1000, 0.1, 0.0) },
                Ticker { ticker_id: 2, ..tk("B", 5.0, 1000, 0.1, 0.0) },
            ];
            if extra {
                v.push(Ticker { ticker_id: 3, ..tk("C", 6.0, 1000, 0.1, 0.0) });
            }
            e.process(v, "change", true, t0 + Duration::from_secs(s))
        };
        // price filter: only A passes at first
        let mut cfg = Config { max_market_cap: 0.0, max_price: 2.0, ..Config::default() };
        e.apply_config(&cfg);
        assert_eq!(rows(&mut e, 0, false).new_alerts, 1);
        // the filter is widened: B now passes, but it is only a new view of the same list
        cfg.max_price = 0.0;
        e.apply_config(&cfg);
        assert_eq!(e.recompute(t0 + Duration::from_secs(1)), 0);
        assert_eq!(rows(&mut e, 2, true).new_alerts, 1, "only C, which was not on the list at the change");
        assert_eq!(kinds(&e).len(), 2);
        assert_eq!(e.alerts().back().unwrap().symbol, "C");
    }

    #[test]
    fn symbol_failing_the_filter_is_off_the_list() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        e.filters.min_price = 0.5;
        let row = |p: f64| Ticker { ticker_id: 1, ..tk("A", p, 1000, 0.1, 0.0) };
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(e.process(vec![row(1.0)], "change", true, at(0)).new_alerts, 1);
        // below the price filter for over 30 min (still in the REST list), then passing again
        assert_eq!(e.process(vec![row(0.4)], "change", true, at(60)).new_alerts, 0);
        assert_eq!(e.process(vec![row(0.4)], "change", true, at(1900)).new_alerts, 0);
        assert_eq!(e.process(vec![row(1.0)], "change", true, at(1901)).new_alerts, 1);
    }

    #[test]
    fn blacklisted_symbols_never_raise_new() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        e.toggle_blacklist("B");
        assert_eq!(list(&mut e, t0, 0, &["A", "B"]).new_alerts, 1);
        assert_eq!(kinds(&e).len(), 1);
        // taking it off the blacklist is not an appearance: it has been on the list all along
        e.toggle_blacklist("B");
        assert_eq!(list(&mut e, t0, 10, &["A", "B"]).new_alerts, 0);
    }

    #[test]
    fn new_follows_the_cooldown_and_badge() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        e.apply_config(&Config { alert_cooldown_sec: 4000, max_market_cap: 0.0, ..Config::default() });
        list(&mut e, t0, 0, &["A"]);
        list(&mut e, t0, 60, &[]);
        // back after 30 min, but the (long) cooldown of the first NEW has not run out
        assert_eq!(list(&mut e, t0, 1900, &["A"]).new_alerts, 0);
        assert_eq!(kinds(&e).len(), 1);
        // the badge from the first NEW is still held
        assert_eq!(status_at(&e, t0 + Duration::from_secs(1900)), "NEW");
    }

    #[test]
    fn listed_entries_are_pruned_after_40_min() {
        let t0 = Instant::now();
        let mut e = seeded_engine(t0);
        list(&mut e, t0, 0, &["A", "B"]);
        list(&mut e, t0, 2399, &["B"]);
        assert!(e.listed.contains_key("A"));
        list(&mut e, t0, 2400, &["B"]);
        assert!(!e.listed.contains_key("A") && e.listed.contains_key("B"));
    }

    #[test]
    fn series_horizon_follows_the_longer_of_vol_m_and_setup_windows() {
        let mut cfg = Config::default();
        cfg.vol_m_window_seconds = 30;
        cfg.setup_window_sec = 200;
        assert_eq!(series_horizon(&Thresholds::from_config(&cfg)), Duration::from_secs(260));
        cfg.vol_m_window_seconds = 300;
        cfg.setup_window_sec = 60;
        assert_eq!(series_horizon(&Thresholds::from_config(&cfg)), Duration::from_secs(360));
    }

    /// Engine whose REST rows (ticker 1) feed the series; Vol/m window `w` seconds.
    fn vol_engine(w: u64) -> Engine {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.vol_m_window_seconds = w;
        Engine::new(&cfg)
    }

    fn feed(e: &mut Engine, t0: Instant, secs: u64, vol: i64) {
        let t = Ticker { ticker_id: 1, ..tk("A", 1.0, vol, 0.1, 0.0) };
        e.process(vec![t], "change", true, t0 + Duration::from_secs(secs));
    }

    fn row(e: &Engine) -> Ticker {
        e.reapply("change", true, Instant::now()).active.remove(0)
    }

    #[test]
    fn vol_m_over_a_full_window() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        // 50 shares per second
        for i in 0..=20u64 {
            feed(&mut e, t0, i * 10, 1000 + 500 * i as i64);
        }
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_partial, t.vol_stale), (Some(3000.0), false, false));
    }

    #[test]
    fn vol_m_is_partial_until_the_window_is_covered() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        for i in 0..=6u64 {
            feed(&mut e, t0, i * 10, 1000 + 500 * i as i64);
        }
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_partial), (Some(3000.0), true));
    }

    #[test]
    fn vol_m_needs_30_seconds_of_history() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        feed(&mut e, t0, 0, 1000);
        assert_eq!(row(&e).vol_rate, None);
        feed(&mut e, t0, 29, 5000);
        assert_eq!(row(&e).vol_rate, None);
        feed(&mut e, t0, 30, 5000);
        assert!(row(&e).vol_rate.is_some());
        // a ticker without a tickerId has no series
        let mut e = vol_engine(180);
        e.process(vec![tk("B", 1.0, 1000, 0.1, 0.0)], "change", true, t0);
        e.process(vec![tk("B", 1.0, 9000, 0.1, 0.0)], "change", true, t0 + Duration::from_secs(60));
        assert_eq!(row(&e).vol_rate, None);
    }

    #[test]
    fn vol_m_of_a_quiet_ticker_decays_to_zero() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        for i in 0..=6u64 {
            feed(&mut e, t0, i * 10, 1000 + 500 * i as i64);
        }
        // 60 s later the burst is still inside the window: 3000 shares over 120 s
        e.refresh_vol_rates(t0 + Duration::from_secs(120));
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_partial), (Some(1500.0), true));
        // once the window has moved past the burst the rate is a true 0
        e.refresh_vol_rates(t0 + Duration::from_secs(300));
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_partial), (Some(0.0), false));
    }

    #[test]
    fn vol_m_is_hidden_while_the_feed_is_stale() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        for i in 0..=6u64 {
            feed(&mut e, t0, i * 10, 1000 + 500 * i as i64);
        }
        e.set_feed_stale(true);
        e.refresh_vol_rates(t0 + Duration::from_secs(61));
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_stale), (None, true));
        e.set_feed_stale(false);
        e.refresh_vol_rates(t0 + Duration::from_secs(61));
        let t = row(&e);
        assert_eq!((t.vol_rate.is_some(), t.vol_stale), (true, false));
    }

    #[test]
    fn vol_m_blanks_when_the_series_is_cleared() {
        let mut e = vol_engine(180);
        let t0 = Instant::now();
        for i in 0..=6u64 {
            feed(&mut e, t0, i * 10, 1000 + 500 * i as i64);
        }
        assert!(row(&e).vol_rate.is_some());
        e.on_stream_reconnect();
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_stale), (None, false));
    }

    #[test]
    fn vol_m_window_follows_config_and_is_clamped() {
        let mut e = vol_engine(60);
        let t0 = Instant::now();
        // 60 shares per second for 120 s, then 0 for 60 s
        for i in 0..=12u64 {
            feed(&mut e, t0, i * 10, 1000 + 600 * i as i64);
        }
        for i in 13..=18u64 {
            feed(&mut e, t0, i * 10, 1000 + 600 * 12);
        }
        assert_eq!(row(&e).vol_rate, Some(0.0));
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.vol_m_window_seconds = 120;
        e.apply_config(&cfg);
        e.refresh_vol_rates(t0 + Duration::from_secs(180));
        assert_eq!(row(&e).vol_rate, Some(1800.0));
        // below the minimum the window is 30 s
        assert_eq!(Thresholds::from_config(&Config { vol_m_window_seconds: 5, ..Config::default() }).vol_m_window.as_secs(), 30);
    }

    #[test]
    fn vol_m_from_stream_quotes() {
        let (mut e, t0) = live_engine();
        e.set_rest_feeds_series(false);
        let mut u = upd(7, 1000, 1, 1.5, 2000);
        u.recv = t0 + Duration::from_secs(10);
        e.apply_quotes(&[u], Session::Reg, t0 + Duration::from_secs(10));
        // one point only: nothing to measure yet
        assert_eq!(row(&e).vol_rate, None);
        let mut u = upd(7, 2000, 1, 1.6, 8000);
        u.recv = t0 + Duration::from_secs(70);
        e.apply_quotes(&[u], Session::Reg, t0 + Duration::from_secs(70));
        let t = row(&e);
        assert_eq!((t.vol_rate, t.vol_partial), (Some(6000.0), true));
    }

    #[test]
    fn vol_rate_sorts_with_blanks_last_in_both_directions() {
        let r = |s: &str, v: Option<f64>| Ticker { vol_rate: v, ..tk(s, 1.0, 1, 0.1, 0.0) };
        let rows = vec![r("N1", None), r("LOW", Some(10.0)), r("ZERO", Some(0.0)), r("HIGH", Some(500.0)), r("N2", None)];
        let sorted = |desc: bool| {
            let mut v = rows.clone();
            v.sort_by(|a, b| ordered(a, b, "volrate", desc));
            v.into_iter().map(|t| t.symbol).collect::<Vec<_>>()
        };
        assert_eq!(sorted(true), ["HIGH", "LOW", "ZERO", "N1", "N2"]);
        assert_eq!(sorted(false), ["ZERO", "LOW", "HIGH", "N1", "N2"]);
    }
    #[test]
    fn filtered_and_blacklisted_split() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price = 1.0;
        cfg.blacklisted_tickers = vec!["BL".into()];
        let mut e = Engine::new(&cfg);
        let out = e.process(
            vec![tk("LOW", 0.5, 1, 0.1, 0.0), tk("OK", 2.0, 1, 0.1, 0.0), tk("BL", 0.1, 1, 0.1, 0.0)],
            "change",
            true,
            Instant::now(),
        );
        assert_eq!(out.active.len(), 1);
        assert_eq!(out.active[0].symbol, "OK");
        assert_eq!(out.blacklisted[0].symbol, "BL");
    }

    #[test]
    fn blacklisted_symbols_raise_no_alerts() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.blacklisted_tickers = vec!["BL".into()];
        let mut e = Engine::new(&cfg);
        let out = e.process(vec![tk("BL", 1.0, 1000, 0.1, 0.0)], "change", true, Instant::now());
        assert_eq!(out.new_alerts, 0);
        assert!(e.alerts.is_empty());
        assert_eq!(out.blacklisted[0].symbol, "BL");
        assert!(e.last.contains_key("BL"));
    }

    #[test]
    fn filtered_tickers_keep_history_but_hide_and_raise_no_alerts() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price = 5.0;
        let mut e = Engine::new(&cfg);
        let f = Ticker { ticker_id: 9, ..tk("F", 1.0, 1000, 0.1, 0.0) };
        let out = e.process(vec![f], "change", true, Instant::now());
        assert!(out.active.is_empty());
        assert_eq!(out.new_alerts, 0);
        assert!(e.alerts.is_empty());
        assert_eq!(e.series().len(9), 1);
        assert!(e.candles("F").is_some());
    }

    #[test]
    fn reapply_refilters_and_resorts_without_recording() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(
            vec![Ticker { ticker_id: 1, ..tk("A", 1.0, 1000, 0.1, 0.0) }, tk("B", 9.0, 1000, 0.2, 0.0)],
            "change",
            true,
            t0,
        );
        let alerts = e.alerts.len();
        let samples = e.series().len(1);
        let out = e.reapply("price", true, Instant::now());
        assert_eq!(out.active[0].symbol, "B");
        e.apply_config(&Config { min_price: 5.0, max_market_cap: 0.0, ..Config::default() });
        let out = e.reapply("price", true, Instant::now());
        assert_eq!(out.active.len(), 1);
        assert_eq!(out.active[0].symbol, "B");
        assert_eq!(e.alerts.len(), alerts);
        assert_eq!(e.series().len(1), samples);
    }

    #[test]
    fn compare_ties_break_by_symbol() {
        let a = tk("AAA", 1.0, 10, 0.0, 1.0);
        let b = tk("BBB", 1.0, 10, 0.0, 1.0);
        for key in ["mov", "price", "change", "volume", "status"] {
            assert_eq!(compare(&a, &b, key), Ordering::Less);
        }
    }

    fn rank_tk(sym: &str, id: u64, chg: f64) -> Ticker {
        Ticker { ticker_id: id, prev_close: 1.0, ..tk(sym, 1.0 + chg, 1000, chg, 0.0) }
    }

    fn movs(out: &Output) -> Vec<(String, Mov)> {
        let mut v: Vec<_> = out.active.iter().map(|t| (t.symbol.clone(), t.mov)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn mv(s: &str, m: Mov) -> (String, Mov) {
        (s.to_string(), m)
    }

    #[test]
    fn mov_is_rank_change_over_the_interval() {
        let mut e = engine();
        let t0 = Instant::now();
        let board = |a: f64, b: f64, c: f64| vec![tk("A", 1.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0), tk("C", 1.0, 1, c, 0.0)];
        // no history yet: blank
        let out = e.process(board(0.5, 0.3, 0.1), "change", true, t0);
        assert!(out.active.iter().all(|t| t.mov == Mov::None));
        // 59 s in: the baseline is not a full interval old
        let out = e.process(board(0.2, 0.4, 0.1), "change", true, t0 + Duration::from_secs(59));
        assert!(out.active.iter().all(|t| t.mov == Mov::None));
        // 60 s in: ranks A1 B2 C3 became B1 A2 C3
        let out = e.process(board(0.2, 0.4, 0.1), "change", true, t0 + Duration::from_secs(60));
        assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(1)), mv("C", Mov::Moved(0))]);
        assert_eq!(out.active[0].symbol, "B");
    }

    #[test]
    fn mov_baseline_is_the_board_one_interval_ago() {
        let mut e = engine();
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let board = |a: f64, b: f64| vec![tk("A", 1.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0)];
        e.process(board(0.5, 0.3), "change", true, at(0)); // A B
        e.process(board(0.3, 0.5), "change", true, at(30)); // B A
        e.process(board(0.5, 0.3), "change", true, at(55)); // A B again
        // baseline for t=70 is the board at t=10: A B. Nothing moved, whatever happened in between.
        let out = e.process(board(0.5, 0.3), "change", true, at(70));
        assert_eq!(movs(&out), [mv("A", Mov::Moved(0)), mv("B", Mov::Moved(0))]);
        // baseline for t=95 is the board at t=35: B A
        let out = e.process(board(0.5, 0.3), "change", true, at(95));
        assert_eq!(movs(&out), [mv("A", Mov::Moved(1)), mv("B", Mov::Moved(-1))]);
    }

    #[test]
    fn mov_marks_new_entrants_and_ignores_the_blacklist() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(vec![tk("A", 1.0, 1, 0.3, 0.0), tk("B", 1.0, 1, 0.2, 0.0)], "change", true, t0);
        // D enters on top; blacklisted X sits above A and B but is not ranked at all
        e.blacklist.insert("X".into());
        let out = e.process(
            vec![tk("D", 1.0, 1, 0.9, 0.0), tk("X", 1.0, 1, 0.8, 0.0), tk("A", 1.0, 1, 0.3, 0.0), tk("B", 1.0, 1, 0.2, 0.0)],
            "change",
            true,
            t0 + Duration::from_secs(60),
        );
        // A and B each drop one place behind D; X has no Mov
        assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(-1)), mv("D", Mov::New)]);
        assert_eq!(out.blacklisted[0].mov, Mov::None);
        // X was not on the board at the start of the interval, so un-blacklisted it counts as a new entrant
        e.blacklist.clear();
        e.recompute(t0 + Duration::from_secs(61));
        let out = e.reapply("change", true, t0 + Duration::from_secs(61));
        assert!(out.active.iter().any(|t| t.symbol == "X" && t.mov == Mov::New));
    }

    #[test]
    fn mov_ignores_sort_and_display_filters() {
        let mut e = engine();
        let t0 = Instant::now();
        let board = |a: f64, b: f64, c: f64| vec![tk("A", 5.0, 1, a, 0.0), tk("B", 1.0, 1, b, 0.0), tk("C", 3.0, 1, c, 0.0)];
        e.process(board(0.5, 0.3, 0.1), "change", true, t0);
        let t1 = t0 + Duration::from_secs(60);
        let by_change = e.process(board(0.1, 0.3, 0.5), "change", true, t1);
        let by_price = e.reapply("price", false, t1);
        assert_eq!(movs(&by_change), movs(&by_price));
        assert_eq!(movs(&by_change), [mv("A", Mov::Moved(-2)), mv("B", Mov::Moved(0)), mv("C", Mov::Moved(2))]);
        // a filter that hides B and C leaves the full-board ranks untouched
        let cfg = Config { min_price: 4.0, max_market_cap: 0.0, ..Config::default() };
        e.apply_config(&cfg);
        let out = e.reapply("change", true, t1);
        assert_eq!(out.active.len(), 1);
        assert_eq!(out.active[0].mov, Mov::Moved(-2));
    }

    #[test]
    fn sorting_by_mov_puts_blank_and_new_rows_last() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(vec![tk("A", 1.0, 1, 0.5, 0.0), tk("B", 1.0, 1, 0.3, 0.0), tk("C", 1.0, 1, 0.1, 0.0)], "change", true, t0);
        let t1 = t0 + Duration::from_secs(60);
        // new board B C N A: A lost 3, B and C gained 1, N is new
        let out = e.process(
            vec![tk("A", 1.0, 1, 0.1, 0.0), tk("B", 1.0, 1, 0.5, 0.0), tk("C", 1.0, 1, 0.3, 0.0), tk("N", 1.0, 1, 0.2, 0.0)],
            "mov",
            true,
            t1,
        );
        let order = |o: &Output| o.active.iter().map(|t| t.symbol.as_str()).collect::<String>();
        assert_eq!(order(&out), "CBAN");
        assert_eq!(order(&e.reapply("mov", false, t1)), "ABCN");
        // before any history every row is blank: symbol order
        let mut fresh = engine();
        let out = fresh.process(vec![tk("B", 1.0, 1, 0.3, 0.0), tk("A", 1.0, 1, 0.5, 0.0)], "mov", true, t0);
        assert_eq!(order(&out), "AB");
    }

    #[test]
    fn recompute_tick_reranks_from_stream_quotes() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(vec![rank_tk("A", 1, 0.5), rank_tk("B", 2, 0.3)], "change", true, t0);
        // stream data arrives; Mov is not recomputed until the tick
        assert!(e.apply_quotes(&[upd(1, 1000, 1, 1.1, 2000), upd(2, 1000, 1, 1.9, 2000)], Session::Reg, t0));
        let t1 = t0 + Duration::from_secs(60);
        let out = e.reapply("change", true, t1);
        assert!(out.active.iter().all(|t| t.mov == Mov::None));
        assert_eq!(out.active[0].symbol, "B");
        e.recompute(t1);
        let out = e.reapply("change", true, t1);
        assert_eq!(movs(&out), [mv("A", Mov::Moved(-1)), mv("B", Mov::Moved(1))]);
    }

    fn filtered_engine(min_price: f64) -> Engine {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.min_price = min_price;
        Engine::new(&cfg)
    }

    fn rows_dim(e: &Engine, now: Instant) -> Vec<(String, bool)> {
        e.reapply("symbol", false, now).active.iter().map(|t| (t.symbol.clone(), t.dim)).collect()
    }

    fn row_dim(s: &str, dim: bool) -> (String, bool) {
        (s.to_string(), dim)
    }

    #[test]
    fn row_failing_a_filter_stays_dimmed_for_60s_then_hides() {
        let mut e = filtered_engine(2.0);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let a = |price: f64| tk("A", price, 1000, 0.1, 0.0);
        let out = e.process(vec![a(3.0), tk("B", 5.0, 1, 0.1, 0.0)], "symbol", false, t0);
        assert_eq!(out.active.iter().map(|t| t.dim).collect::<Vec<_>>(), [false, false]);
        // A drops under the price filter: still listed, dimmed
        e.process(vec![a(1.5), tk("B", 5.0, 1, 0.1, 0.0)], "symbol", false, at(10));
        assert_eq!(rows_dim(&e, at(10)), [row_dim("A", true), row_dim("B", false)]);
        // the grace runs from the recompute that noticed it (t=10)
        e.recompute(at(30));
        assert_eq!(rows_dim(&e, at(69)), [row_dim("A", true), row_dim("B", false)]);
        assert_eq!(rows_dim(&e, at(70)), [row_dim("B", false)]);
        e.recompute(at(80));
        assert_eq!(rows_dim(&e, at(80)), [row_dim("B", false)]);
        // once hidden it stays hidden while it keeps failing
        e.recompute(at(200));
        assert_eq!(rows_dim(&e, at(200)), [row_dim("B", false)]);
    }

    #[test]
    fn row_that_passes_again_during_grace_is_undimmed_and_regraces_later() {
        let mut e = filtered_engine(2.0);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let a = |price: f64| vec![tk("A", price, 1000, 0.1, 0.0)];
        e.process(a(3.0), "symbol", false, t0);
        e.process(a(1.0), "symbol", false, at(10));
        assert_eq!(rows_dim(&e, at(40)), [row_dim("A", true)]);
        e.process(a(3.0), "symbol", false, at(40));
        assert_eq!(rows_dim(&e, at(41)), [row_dim("A", false)]);
        // fails again: a fresh 60 s starts, not the remainder of the old one
        e.process(a(1.0), "symbol", false, at(50));
        assert_eq!(rows_dim(&e, at(109)), [row_dim("A", true)]);
        assert_eq!(rows_dim(&e, at(110)), []);
    }

    #[test]
    fn row_that_never_passed_is_not_shown_and_leaving_rows_are_forgotten() {
        let mut e = filtered_engine(2.0);
        let t0 = Instant::now();
        e.process(vec![tk("A", 1.0, 1, 0.1, 0.0)], "symbol", false, t0);
        assert_eq!(rows_dim(&e, t0), []);
        e.process(vec![tk("B", 3.0, 1, 0.1, 0.0)], "symbol", false, t0 + Duration::from_secs(1));
        assert!(e.passing.contains("B") && !e.passing.contains("A"));
        e.process(vec![], "symbol", false, t0 + Duration::from_secs(2));
        assert!(e.passing.is_empty() && e.failing_since.is_empty());
    }

    #[test]
    fn saving_config_clears_the_grace() {
        let mut e = filtered_engine(2.0);
        let t0 = Instant::now();
        e.process(vec![tk("A", 3.0, 1, 0.1, 0.0)], "symbol", false, t0);
        let tight = Config { min_price: 4.0, max_market_cap: 0.0, ..Config::default() };
        // the filters changed under the engine: A would be in grace
        e.filters = Filters::from_config(&tight);
        assert_eq!(rows_dim(&e, t0), [row_dim("A", true)]);
        // saving applies the new filters at once
        e.apply_config(&tight);
        assert_eq!(rows_dim(&e, t0), []);
        // and loosening brings the row back undimmed
        let loose = Config { min_price: 1.0, max_market_cap: 0.0, ..Config::default() };
        e.apply_config(&loose);
        assert_eq!(rows_dim(&e, t0), [row_dim("A", false)]);
    }

    #[test]
    fn blacklisted_rows_are_never_in_grace() {
        let mut e = filtered_engine(0.0);
        let t0 = Instant::now();
        e.process(vec![tk("A", 3.0, 1, 0.1, 0.0)], "symbol", false, t0);
        e.toggle_blacklist("A");
        e.recompute(t0 + Duration::from_secs(1));
        let out = e.reapply("symbol", false, t0 + Duration::from_secs(1));
        assert!(out.active.is_empty());
        assert_eq!(out.blacklisted.len(), 1);
    }

    #[test]
    fn candles_aggregate_per_minute() {
        let mut e = engine();
        e.record_sample("A", 600, 1.0, 1000);
        e.record_sample("A", 610, 1.5, 1500);
        e.record_sample("A", 650, 0.8, 1600);
        e.record_sample("A", 660, 0.9, 1700);
        e.record_sample("A", 655, 5.0, 1800); // late sample for a closed minute is ignored
        let c: Vec<_> = e.candles("A").unwrap().iter().copied().collect();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].open, c[0].high, c[0].low, c[0].close, c[0].volume), (1.0, 1.5, 0.8, 0.8, 600));
        assert_eq!((c[1].start, c[1].open, c[1].volume), (660, 0.9, 100));
    }

    #[test]
    fn alert_history_capped() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.alert_history_max = 10;
        let mut e = Engine::new(&cfg);
        let ts: Vec<_> = (0..25).map(|i| tk(&format!("T{i}"), 1.0, 1, 0.1, 0.0)).collect();
        let now = Instant::now();
        seed_empty(&mut e, now);
        e.process(ts, "change", true, now);
        assert_eq!(e.alerts().len(), 10);
    }

    /// Exchange stamps in tests are offsets from one fixed point near now, so they share a
    /// session with the REST rows that `process` stamps with the wall clock.
    fn base_ms() -> i64 {
        static BASE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
        *BASE.get_or_init(|| chrono::Utc::now().timestamp_millis())
    }

    fn upd(id: u64, ms: i64, seq: u64, px: f64, vol: u64) -> Update {
        Update {
            id,
            trade_ms: base_ms() + ms,
            seq,
            close: Some(px),
            p_price: None,
            change_ratio: None,
            volume: Some(vol),
            mcap: None,
            high: None,
            low: None,
            status: String::new(),
            recv: Instant::now(),
        }
    }

    fn live_engine() -> (Engine, Instant) {
        let mut e = engine();
        let t0 = Instant::now();
        let a = Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.1, 1000, 0.1, 5e6) };
        e.process(vec![a], "change", true, t0);
        (e, t0)
    }

    #[test]
    fn quotes_update_rows_and_change_is_local() {
        let (mut e, t0) = live_engine();
        assert!(e.apply_quotes(&[upd(7, 1000, 1, 1.5, 1600)], Session::Reg, t0));
        let t = &e.reapply("change", true, Instant::now()).active[0];
        assert_eq!((t.price, t.volume), (1.5, 1600));
        assert!((t.change_ratio - 0.5).abs() < 1e-9);
        // 5 s of history is too little for Vol/m
        assert_eq!(t.vol_rate, None);
        // unknown ids and ids without volume change nothing
        assert!(!e.apply_quotes(&[upd(99, 1, 1, 1.0, 1)], Session::Reg, t0));
        let mut nov = upd(7, 2000, 1, 2.0, 0);
        nov.volume = None;
        assert!(!e.apply_quotes(&[nov], Session::Reg, t0));
    }

    #[test]
    fn quotes_ordering_and_lower_volume() {
        let (mut e, t0) = live_engine();
        e.apply_quotes(&[upd(7, 1000, 5, 1.5, 2000)], Session::Reg, t0);
        // older timestamp, same ms with lower/equal seq: dropped
        assert!(!e.apply_quotes(&[upd(7, 999, 9, 1.9, 2100)], Session::Reg, t0));
        assert!(!e.apply_quotes(&[upd(7, 1000, 5, 1.9, 2100)], Session::Reg, t0));
        assert!(e.apply_quotes(&[upd(7, 1000, 6, 1.6, 2100)], Session::Reg, t0));
        // lower volume: dropped twice, believed on the third reading
        assert!(!e.apply_quotes(&[upd(7, 1100, 1, 1.6, 50)], Session::Reg, t0));
        assert!(!e.apply_quotes(&[upd(7, 1200, 1, 1.6, 50)], Session::Reg, t0));
        assert_eq!(e.reapply("change", true, Instant::now()).active[0].volume, 2100);
        assert!(e.apply_quotes(&[upd(7, 1300, 1, 1.6, 50)], Session::Reg, t0));
        assert_eq!(e.reapply("change", true, Instant::now()).active[0].volume, 50);
    }

    #[test]
    fn extended_hours_prefers_p_price() {
        let (mut e, t0) = live_engine();
        let mut u = upd(7, 1000, 1, 1.5, 1100);
        u.p_price = Some(1.8);
        e.apply_quotes(&[u.clone()], Session::Pre, t0);
        assert_eq!(e.reapply("change", true, Instant::now()).active[0].price, 1.8);
        u.trade_ms = base_ms() + 2000;
        e.apply_quotes(&[u], Session::Reg, t0);
        assert_eq!(e.reapply("change", true, Instant::now()).active[0].price, 1.5);
    }

    #[test]
    fn rest_refresh_keeps_fresh_stream_values() {
        let (mut e, t0) = live_engine();
        e.apply_quotes(&[upd(7, 1000, 1, 1.5, 1600)], Session::Reg, t0);
        let rest = Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.2, 1200, 0.2, 5e6) };
        let out = e.process(vec![rest.clone()], "change", true, t0 + Duration::from_secs(5));
        assert_eq!((out.active[0].price, out.active[0].volume), (1.5, 1600));
        assert!((out.active[0].change_ratio - 0.5).abs() < 1e-9);
        // once the stream goes quiet the REST row wins again
        let out = e.process(vec![rest], "change", true, t0 + LIVE_FRESH + Duration::from_secs(5));
        assert_eq!((out.active[0].price, out.active[0].volume), (1.2, 1200));
    }

    #[test]
    fn stream_quotes_feed_series() {
        let (mut e, t0) = live_engine();
        let mut u = upd(7, 1000, 1, 1.5, 1600);
        u.recv = t0 + Duration::from_secs(2);
        e.apply_quotes(&[u], Session::Reg, t0);
        let mut u = upd(7, 2000, 1, 1.6, 1800);
        u.recv = t0 + Duration::from_secs(6);
        e.apply_quotes(&[u], Session::Reg, t0);
        let s = e.series();
        // the REST sample taken at t0 was replaced by the source switch
        assert_eq!(s.len(7), 2);
        assert_eq!(s.vol_at(7, t0 + Duration::from_secs(4)), Some(1600));
        assert_eq!(s.px_at(7, t0 + Duration::from_secs(6)), Some(1.6));
        assert_eq!(s.vol_at(7, t0 + Duration::from_secs(1)), None);
    }

    #[test]
    fn series_uses_p_price_in_extended_hours_and_close_in_regular() {
        let (mut e, t0) = live_engine();
        let mut u = upd(7, 1000, 1, 0.0, 1100);
        u.close = None;
        u.p_price = Some(1.8);
        u.recv = t0;
        e.apply_quotes(&[u.clone()], Session::Pre, t0);
        assert_eq!(e.series().latest(7).unwrap().px, 1.8);
        // Reg ignores pPrice; without close the message is dropped
        u.trade_ms += 1000;
        e.apply_quotes(&[u], Session::Reg, t0);
        assert_eq!(e.series().len(7), 1);
        assert_eq!(e.series().latest(7).unwrap().px, 1.8);
    }

    #[test]
    fn dropped_quotes_do_not_reach_series_or_rows() {
        let (mut e, t0) = live_engine();
        let mut u = upd(7, 1000, 1, 1.5, 1600);
        u.recv = t0;
        e.apply_quotes(&[u], Session::Reg, t0);
        let mut old = upd(7, 500, 1, 9.0, 9000);
        old.recv = t0 + Duration::from_secs(3);
        assert!(!e.apply_quotes(&[old], Session::Reg, t0));
        assert_eq!(e.series().latest(7).unwrap().vol, 1600);
    }

    #[test]
    fn rest_rows_feed_series_only_while_enabled() {
        let (mut e, t0) = live_engine();
        // live_engine's first process() fed the series from REST
        assert_eq!(e.series().len(7), 1);
        assert_eq!(e.series().latest(7).unwrap().vol, 1000);
        e.set_rest_feeds_series(false);
        let rest = |v: i64| Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.2, v, 0.2, 5e6) };
        e.process(vec![rest(1500)], "change", true, t0 + Duration::from_secs(5));
        assert_eq!(e.series().latest(7).unwrap().vol, 1000);
        e.set_rest_feeds_series(true);
        e.process(vec![rest(1500)], "change", true, t0 + Duration::from_secs(10));
        assert_eq!(e.series().latest(7).unwrap().vol, 1500);
        assert_eq!(e.series().len(7), 2);
        assert_eq!(e.series().vol_at(7, t0 + Duration::from_secs(8)), Some(1000));
    }

    #[test]
    fn row_with_fresh_stream_quote_does_not_feed_rest_sample() {
        let (mut e, t0) = live_engine();
        let mut u = upd(7, 1000, 1, 1.5, 1600);
        u.recv = t0 + Duration::from_secs(1);
        e.apply_quotes(&[u], Session::Reg, t0 + Duration::from_secs(1));
        let rest = Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.2, 1200, 0.2, 5e6) };
        e.process(vec![rest], "change", true, t0 + Duration::from_secs(5));
        let s = e.series();
        assert_eq!(s.len(7), 1);
        assert_eq!(s.latest(7).unwrap().vol, 1600);
    }

    #[test]
    fn reconnect_clears_series() {
        let (mut e, _) = live_engine();
        assert_eq!(e.series().len(7), 1);
        e.on_stream_reconnect();
        assert_eq!(e.series().len(7), 0);
    }

    #[test]
    fn rest_zero_volume_and_unkeyed_rows_are_not_recorded() {
        let mut e = engine();
        let t0 = Instant::now();
        let z = Ticker { ticker_id: 5, ..tk("Z", 1.0, 0, 0.1, 0.0) };
        e.process(vec![z, tk("U", 1.0, 10, 0.1, 0.0)], "change", true, t0);
        assert_eq!(e.series().len(5), 0);
        assert_eq!(e.series().len(0), 0);
    }

    #[test]
    fn leaving_ticker_lingers_then_is_evicted_and_unsubscribed() {
        let mut e = engine();
        let t0 = Instant::now();
        let a = Ticker { ticker_id: 1, ..tk("A", 1.0, 100, 0.1, 0.0) };
        let b = Ticker { ticker_id: 2, ..tk("B", 1.0, 100, 0.1, 0.0) };
        e.process(vec![a.clone(), b], "change", true, t0);
        assert_eq!(e.subscribe_ids(), vec![1, 2]);
        e.process(vec![a.clone()], "change", true, t0 + Duration::from_secs(5));
        // members first, then the lingering one
        assert_eq!(e.subscribe_ids(), vec![1, 2]);
        assert_eq!(e.series().len(2), 1);
        assert!(!e.expire_series(t0 + Duration::from_secs(64)));
        assert!(e.expire_series(t0 + Duration::from_secs(65)));
        assert_eq!(e.subscribe_ids(), vec![1]);
        assert_eq!(e.series().len(2), 0);
        // coming back within the linger keeps the series
        e.process(vec![a.clone(), Ticker { ticker_id: 2, ..tk("B", 1.0, 150, 0.1, 0.0) }], "change", true, t0 + Duration::from_secs(70));
        e.process(vec![a], "change", true, t0 + Duration::from_secs(80));
        e.process(vec![Ticker { ticker_id: 2, ..tk("B", 1.0, 160, 0.1, 0.0) }], "change", true, t0 + Duration::from_secs(90));
        assert_eq!(e.series().len(2), 2);
    }

    #[test]
    fn subscribe_ids_cap_members_first() {
        let mut e = engine();
        let t0 = Instant::now();
        let rows: Vec<Ticker> = (1..=120u64).map(|i| Ticker { ticker_id: i, ..tk(&format!("T{i}"), 1.0, 10, 0.1, 0.0) }).collect();
        e.process(rows.clone(), "change", true, t0);
        e.process(rows[..90].to_vec(), "change", true, t0 + Duration::from_secs(5));
        let ids = e.subscribe_ids();
        assert_eq!(ids.len(), MAX_SUBS);
        assert_eq!(ids[..90], (1..=90u64).collect::<Vec<_>>()[..]);
    }

    #[test]
    fn config_change_updates_series_horizon() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.vol_m_window_seconds = 30;
        cfg.setup_window_sec = 30;
        let mut e = Engine::new(&cfg);
        let t0 = Instant::now();
        let a = |v: i64| Ticker { ticker_id: 1, ..tk("A", 1.0, v, 0.1, 0.0) };
        for i in 0..=200u64 {
            e.process(vec![a(100 + i as i64)], "change", true, t0 + Duration::from_secs(i));
        }
        // 30 s window + 60 s margin
        assert!(e.series().len(1) < 95);
        assert!(e.series().vol_at(1, t0 + Duration::from_secs(110)).is_some());
    }
}
