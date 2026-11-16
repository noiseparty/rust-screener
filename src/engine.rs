use crate::config::Config;
use crate::scraper::Ticker;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

pub const MAX_ROWS: usize = 100;
pub const MAX_CANDLES: usize = 390;
pub const RATE_WINDOWS: [(&str, u64); 4] = [("20m", 1200), ("5m", 300), ("1m", 60), ("30s", 30)];

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
    pub spike_vol_ratio: f64,
    pub pump_price_pct: f64,
    pub min_volume_for_alert: i64,
    pub min_price_for_alert: f64,
    pub baseline_window: Duration,
    pub rank_interval: Duration,
    pub delta_window: Duration,
    pub max_alerts: usize,
}

impl Thresholds {
    pub fn from_config(c: &Config) -> Thresholds {
        Thresholds {
            spike_vol_ratio: c.spike_vol_ratio,
            pump_price_pct: c.pump_price_pct,
            min_volume_for_alert: c.min_volume_for_alert,
            min_price_for_alert: c.min_price_for_alert,
            baseline_window: Duration::from_secs(c.baseline_window_sec),
            rank_interval: Duration::from_secs(c.rank_tracking_interval_seconds),
            delta_window: Duration::from_secs(c.delta_window_seconds),
            max_alerts: c.alert_history_max,
        }
    }
}

pub struct Output {
    pub active: Vec<Ticker>,
    pub blacklisted: Vec<Ticker>,
    pub new_alerts: usize,
}

pub struct Engine {
    pub filters: Filters,
    pub th: Thresholds,
    pub blacklist: BTreeSet<String>,
    last: HashMap<String, Ticker>,
    vol_hist: HashMap<String, Vec<(Instant, i64)>>,
    price_hist: HashMap<String, Vec<(Instant, f64)>>,
    rank_hist: HashMap<String, HashMap<String, Vec<(Instant, usize)>>>,
    alerts: VecDeque<Alert>,
    /// Price tick direction events across the board: (time, is_up).
    ticks: VecDeque<(Instant, bool)>,
    candles: HashMap<String, VecDeque<Candle>>,
    last_cum_vol: HashMap<String, i64>,
}

impl Engine {
    pub fn new(cfg: &Config) -> Engine {
        Engine {
            filters: Filters::from_config(cfg),
            th: Thresholds::from_config(cfg),
            blacklist: cfg.blacklisted_tickers.iter().cloned().collect(),
            last: HashMap::new(),
            vol_hist: HashMap::new(),
            price_hist: HashMap::new(),
            rank_hist: HashMap::new(),
            alerts: VecDeque::new(),
            ticks: VecDeque::new(),
            candles: HashMap::new(),
            last_cum_vol: HashMap::new(),
        }
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

    pub fn apply_config(&mut self, cfg: &Config) {
        self.filters = Filters::from_config(cfg);
        self.th = Thresholds::from_config(cfg);
        while self.alerts.len() > self.th.max_alerts {
            self.alerts.pop_front();
        }
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
        let clock = wall.format("%H:%M:%S").to_string();
        let epoch = wall.timestamp();
        let th = self.th;
        let mut active = Vec::with_capacity(tickers.len());
        let mut blacklisted = Vec::new();
        let mut new_alerts = 0;

        for mut t in tickers {
            let is_bl = self.blacklist.contains(&t.symbol);
            if !is_bl && !self.filters.matches(&t) {
                continue;
            }

            let vh = self.vol_hist.entry(t.symbol.clone()).or_default();
            if let Some(base) = baseline(vh, now, th.delta_window) {
                t.vol_delta = t.volume - base;
            }
            let base_vol = baseline(vh, now, th.baseline_window);
            vh.push((now, t.volume));
            prune(vh, now, th.delta_window.max(th.baseline_window));

            let ph = self.price_hist.entry(t.symbol.clone()).or_default();
            let base_price = baseline(ph, now, th.baseline_window);
            ph.push((now, t.price));
            prune(ph, now, th.baseline_window);

            let kind = match self.last.get(&t.symbol) {
                None => Some(AlertKind::New),
                Some(prev) => {
                    if t.price > prev.price {
                        self.ticks.push_back((now, true));
                    } else if t.price < prev.price {
                        self.ticks.push_back((now, false));
                    }
                    let eligible = t.volume >= th.min_volume_for_alert && t.price >= th.min_price_for_alert;
                    let spike = eligible
                        && matches!(base_vol, Some(b) if b > 0 && t.volume as f64 / b as f64 >= th.spike_vol_ratio);
                    let pump = eligible
                        && matches!(base_price, Some(b) if b > 0.0 && (t.price - b) / b >= th.pump_price_pct);
                    match (spike, pump) {
                        (true, true) => Some(AlertKind::Setup),
                        (true, false) => Some(AlertKind::Spike),
                        (false, true) => Some(AlertKind::Pump),
                        _ => None,
                    }
                }
            };

            if let Some(kind) = kind {
                self.alerts.push_back(Alert {
                    at: now,
                    clock: clock.clone(),
                    symbol: t.symbol.clone(),
                    kind,
                    prev_vol: base_vol.unwrap_or(-1),
                    curr_vol: t.volume,
                    prev_price: base_price.unwrap_or(-1.0),
                    curr_price: t.price,
                    change_ratio: t.change_ratio,
                });
                new_alerts += 1;
                t.status = kind.label().to_string();
            }

            self.record_sample(&t.symbol, epoch, t.price, t.volume);
            self.last.insert(t.symbol.clone(), t.clone());
            if is_bl { blacklisted.push(t) } else { active.push(t) }
        }

        let horizon = Duration::from_secs(RATE_WINDOWS[0].1);
        while self.ticks.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) >= horizon) {
            self.ticks.pop_front();
        }

        active.sort_by(|a, b| {
            let o = compare(a, b, sort_key);
            if sort_desc { o.reverse() } else { o }
        });
        active.truncate(MAX_ROWS);

        self.track_ranks(&mut active, sort_key, sort_desc, now);

        while self.alerts.len() > th.max_alerts {
            self.alerts.pop_front();
        }

        Output { active, blacklisted, new_alerts }
    }

    fn track_ranks(&mut self, active: &mut [Ticker], sort_key: &str, sort_desc: bool, now: Instant) {
        let interval = self.th.rank_interval;
        let ctx = self.rank_hist.entry(format!("{sort_key}_{sort_desc}")).or_default();
        for (i, t) in active.iter_mut().enumerate() {
            let rank = i + 1;
            let hist = ctx.entry(t.symbol.clone()).or_default();
            let last = hist.last().map(|&(_, r)| r);
            hist.retain(|&(at, _)| now.saturating_duration_since(at) < interval * 2);
            let base = hist
                .iter()
                .find(|&&(at, _)| now.saturating_duration_since(at) <= interval)
                .map(|&(_, r)| r)
                .or(last);
            t.rank_change = base.map_or(0, |b| b as i32 - rank as i32);
            hist.push((now, rank));
        }
        ctx.retain(|_, h| h.last().is_some_and(|&(at, _)| now.saturating_duration_since(at) < Duration::from_secs(300)));
    }
}

/// Oldest sample inside the window, falling back to the oldest sample overall.
fn baseline<T: Copy>(hist: &[(Instant, T)], now: Instant, window: Duration) -> Option<T> {
    hist.iter()
        .find(|(at, _)| now.saturating_duration_since(*at) <= window)
        .or(hist.first())
        .map(|&(_, v)| v)
}

fn prune<T>(hist: &mut Vec<(Instant, T)>, now: Instant, window: Duration) {
    let keep_from = hist
        .iter()
        .position(|(at, _)| now.saturating_duration_since(*at) <= window)
        .unwrap_or(hist.len().saturating_sub(1));
    hist.drain(..keep_from);
}

pub fn compare(a: &Ticker, b: &Ticker, key: &str) -> Ordering {
    let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    match key {
        "mov" => a.rank_change.cmp(&b.rank_change),
        "symbol" => a.symbol.cmp(&b.symbol),
        "price" => f(a.price, b.price),
        "change" => f(a.change_ratio, b.change_ratio),
        "volume" => a.volume.cmp(&b.volume),
        "mcap" => f(a.market_value, b.market_value),
        "delta" => a.vol_delta.cmp(&b.vol_delta),
        "status" => a.status.cmp(&b.status),
        _ => f(a.change_ratio, b.change_ratio).then(a.vol_delta.cmp(&b.vol_delta)),
    }
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

    #[test]
    fn new_then_setup() {
        let mut e = engine();
        let t0 = Instant::now();
        let out = e.process(vec![tk("A", 1.0, 1000, 0.1, 0.0)], "change", true, t0);
        assert_eq!(out.active[0].status, "NEW");
        let out = e.process(vec![tk("A", 1.2, 3000, 0.3, 0.0)], "change", true, t0 + Duration::from_secs(5));
        assert_eq!(out.active[0].status, "SETUP");
        assert_eq!(e.alerts().back().unwrap().kind, AlertKind::Setup);
        assert_eq!(out.active[0].vol_delta, 2000);
    }

    #[test]
    fn spike_only_and_pump_only() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(vec![tk("S", 1.0, 1000, 0.1, 0.0), tk("P", 1.0, 1000, 0.1, 0.0)], "change", true, t0);
        let out = e.process(
            vec![tk("S", 1.0, 2500, 0.1, 0.0), tk("P", 1.10, 1100, 0.2, 0.0)],
            "symbol",
            false,
            t0 + Duration::from_secs(5),
        );
        assert_eq!(out.active[0].status, "PUMP");
        assert_eq!(out.active[1].status, "SPIKE");
        assert_eq!(e.tick_rates(t0 + Duration::from_secs(6))[3], (0, 1));
    }

    #[test]
    fn delta_uses_delta_window() {
        let mut cfg = Config::default();
        cfg.max_market_cap = 0.0;
        cfg.delta_window_seconds = 10;
        let mut e = Engine::new(&cfg);
        let t0 = Instant::now();
        for (i, v) in [100, 200, 300, 400].iter().enumerate() {
            let out = e.process(vec![tk("A", 1.0, *v, 0.1, 0.0)], "change", true, t0 + Duration::from_secs(i as u64 * 5));
            if i == 3 {
                // window [5s, 15s] -> oldest sample in window is 200
                assert_eq!(out.active[0].vol_delta, 200);
            }
        }
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
    fn rank_change_tracks_movement() {
        let mut e = engine();
        let t0 = Instant::now();
        e.process(vec![tk("A", 1.0, 1, 0.5, 0.0), tk("B", 1.0, 1, 0.3, 0.0)], "change", true, t0);
        let out = e.process(vec![tk("A", 1.0, 1, 0.2, 0.0), tk("B", 1.0, 1, 0.4, 0.0)], "change", true, t0 + Duration::from_secs(5));
        assert_eq!(out.active[0].symbol, "B");
        assert_eq!(out.active[0].rank_change, 1);
        assert_eq!(out.active[1].rank_change, -1);
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
        e.process(ts, "change", true, Instant::now());
        assert_eq!(e.alerts().len(), 10);
    }
}
