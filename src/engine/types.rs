use super::MIN_VOL_M_WINDOW_SECS;
use crate::config::Config;
use crate::scraper::Ticker;
use std::time::{Duration, Instant};

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
    pub(super) fn slot(self) -> usize {
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
    /// Wall-clock time it fired (epoch seconds), shown in the chosen time format.
    pub epoch: i64,
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
        if self.prev_vol > 0 {
            self.curr_vol as f64 / self.prev_vol as f64
        } else {
            0.0
        }
    }

    pub fn price_pct(&self) -> f64 {
        if self.prev_price > 0.0 {
            (self.curr_price - self.prev_price) / self.prev_price * 100.0
        } else {
            0.0
        }
    }

    pub fn detail(&self) -> String {
        match self.kind {
            AlertKind::New => "appeared".into(),
            AlertKind::Spike => format!("vol {:.1}x", self.vol_ratio()),
            AlertKind::Pump => format!("price +{:.1}%", self.price_pct()),
            // a previous window without trades has no ratio
            AlertKind::Setup if self.prev_vol == 0 => format!("vol from 0  +{:.1}%", self.price_pct()),
            AlertKind::Setup => format!("vol {:.1}x  +{:.1}%", self.vol_ratio(), self.price_pct()),
        }
    }
}

/// Display filters. A zero bound means "no bound". An unknown float or share count (0) passes
/// its bounds, so a ticker is not hidden before its float is fetched.
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
    pub min_float: f64,
    pub max_float: f64,
    pub min_shares: f64,
    pub max_shares: f64,
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
            min_float: c.min_float,
            max_float: c.max_float,
            min_shares: c.min_shares,
            max_shares: c.max_shares,
        }
    }

    pub fn matches(&self, t: &Ticker) -> bool {
        if self.min_mcap > 0.0 || self.max_mcap > 0.0 {
            if t.market_value == 0.0 {
                return false;
            }
            if (self.min_mcap > 0.0 && t.market_value < self.min_mcap) || (self.max_mcap > 0.0 && t.market_value > self.max_mcap) {
                return false;
            }
        }
        let outside = |v: f64, min: f64, max: f64| v > 0.0 && ((min > 0.0 && v < min) || (max > 0.0 && v > max));
        if outside(t.float, self.min_float, self.max_float) || outside(t.total_shares, self.min_shares, self.max_shares) {
            return false;
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

/// SETUP baseline: shares traded in the previous and in the last SETUP window, and the price
/// one window ago.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetupBase {
    pub prev: u64,
    pub cur: u64,
    pub px: f64,
}
