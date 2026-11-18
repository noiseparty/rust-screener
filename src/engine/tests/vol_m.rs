use super::*;
use crate::engine::feed::series_horizon;
use crate::engine::ranking::ordered;
use crate::session::Session;

#[test]
fn series_horizon_follows_the_longer_of_vol_m_and_setup_windows() {
    let mut cfg = Config::default();
    cfg.vol_m_window_seconds = 30;
    cfg.setup_window_sec = 200;
    assert_eq!(series_horizon(&Thresholds::from_config(&cfg)), Duration::from_secs(460));
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
    // two 30 s SETUP windows + 60 s margin
    assert!(e.series().len(1) < 125);
    assert!(e.series().vol_at(1, t0 + Duration::from_secs(110)).is_some());
}
