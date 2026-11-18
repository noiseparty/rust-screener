//! Engine tests by topic; the fixtures they share are here.

mod alerts_new;
mod candles;
mod cooldown;
mod display;
mod quotes;
mod ranking;
mod setup;
mod vol_m;

use super::*;
use crate::stream::Update;

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

/// Status of the only row as the table would show it at `now`.
fn status_at(e: &Engine, now: Instant) -> String {
    e.reapply("change", true, now).active[0].status.map_or("", AlertKind::label).to_string()
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
