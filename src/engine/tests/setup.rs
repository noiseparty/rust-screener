use super::*;
use crate::engine::alerts::setup_met;
use crate::session::Session;

#[test]
fn setup_fires_exactly_at_both_limits() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    let out = setup_row(&mut e, t0, 0, 1.0, 1000);
    assert_eq!(out.active[0].status, None);
    // 1000 traded in the previous 300 s window
    assert_eq!(setup_row(&mut e, t0, 300, 1.0, 2000).active[0].status, None);
    // 2200 in the last window (exactly 2.2x) and exactly +10% on one window ago
    let out = setup_row(&mut e, t0, 600, 1.10, 4200);
    assert_eq!(out.active[0].status, Some(AlertKind::Setup));
    let a = e.alerts().back().unwrap();
    assert_eq!(a.kind, AlertKind::Setup);
    assert_eq!((a.prev_vol, a.curr_vol, a.prev_price, a.curr_price), (1000, 2200, 1.0, 1.10));
    assert_eq!(a.detail(), "vol 2.2x  +10.0%");
}

#[test]
fn setup_needs_both_limits() {
    for (price, vol) in [(1.099, 4200), (1.10, 4199), (1.0, 20_000), (3.0, 2000)] {
        let mut e = setup_engine();
        let t0 = Instant::now();
        setup_row(&mut e, t0, 0, 1.0, 1000);
        setup_row(&mut e, t0, 300, 1.0, 2000);
        let out = setup_row(&mut e, t0, 600, price, vol);
        assert_eq!(out.new_alerts, 0, "price {price} vol {vol}");
        assert!(kinds(&e).is_empty());
    }
}

#[test]
fn spike_and_pump_are_never_raised() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    let rows = |s_vol: i64, p_px: f64| {
        vec![Ticker { ticker_id: 1, ..tk("S", 1.0, s_vol, 0.1, 0.0) }, Ticker { ticker_id: 2, ..tk("P", p_px, 1000, 0.5, 0.0) }]
    };
    e.process(rows(1000, 1.0), "change", true, t0);
    // S: volume x5, flat price (a SPIKE before). P: price +50%, flat volume (a PUMP before).
    let out = e.process(rows(5000, 1.5), "symbol", false, t0 + Duration::from_secs(5));
    assert_eq!(out.new_alerts, 0);
    // nothing was raised (the first list only seeds NEW)
    assert!(out.active.iter().all(|t| t.status.is_none()));
    assert!(kinds(&e).is_empty());
}

#[test]
fn setup_needs_part_of_a_previous_window_and_scales_it() {
    let t0 = Instant::now();
    // less history than one window: no previous window to compare with
    let mut e = setup_engine();
    setup_row(&mut e, t0, 0, 1.0, 1000);
    assert_eq!(setup_row(&mut e, t0, 20, 1.2, 3000).new_alerts, 0);
    // 20 s before the last window is under a tenth of a window: still nothing
    assert_eq!(setup_row(&mut e, t0, 320, 1.2, 90_000).new_alerts, 0);
    // 60 s before it, with 200 traded: 1000 per full window, so 2200 in the last one fires
    let mut e = setup_engine();
    setup_row(&mut e, t0, 0, 1.0, 1000);
    setup_row(&mut e, t0, 60, 1.0, 1200);
    assert_eq!(setup_row(&mut e, t0, 360, 1.1, 3399).new_alerts, 0);
    assert_eq!(setup_row(&mut e, t0, 360, 1.1, 3400).active[0].status, Some(AlertKind::Setup));
}

#[test]
fn setup_after_a_quiet_window_fires_on_any_volume() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    setup_row(&mut e, t0, 0, 1.0, 1000);
    setup_row(&mut e, t0, 300, 1.0, 1000);
    let out = setup_row(&mut e, t0, 600, 1.1, 1500);
    assert_eq!(out.active[0].status, Some(AlertKind::Setup));
    assert_eq!(e.alerts().back().unwrap().detail(), "vol from 0  +10.0%");
}

#[test]
fn setup_compares_windows_not_the_day_volume() {
    // a ticker that already traded a million shares spikes again: 2200 vs 1000 per window
    let mut e = setup_engine();
    let t0 = Instant::now();
    setup_row(&mut e, t0, 0, 1.0, 1_000_000);
    setup_row(&mut e, t0, 300, 1.0, 1_001_000);
    assert_eq!(setup_row(&mut e, t0, 600, 1.1, 1_003_199).new_alerts, 0);
    assert_eq!(setup_row(&mut e, t0, 600, 1.1, 1_003_200).active[0].status, Some(AlertKind::Setup));
}

#[test]
fn setup_without_a_series_never_fires() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    // ticker id 0 has no series
    let z = |p: f64, v: i64| vec![tk("Z", p, v, 0.3, 0.0)];
    e.process(z(1.0, 1000), "change", true, t0);
    e.process(z(1.0, 2000), "change", true, t0 + Duration::from_secs(300));
    let out = e.process(z(2.0, 9000), "change", true, t0 + Duration::from_secs(600));
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
    // History rebuilt after the clear works again: 1000 traded 300-600 s, 2200 600-900 s.
    assert_eq!(setup_row(&mut e, t0, 600, 1.5, 6000).new_alerts, 0);
    let out = setup_row(&mut e, t0, 900, 1.65, 8200);
    assert_eq!(out.active[0].status, Some(AlertKind::Setup));
}

#[test]
fn setup_price_baseline_is_one_window_ago() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    setup_row(&mut e, t0, 0, 1.0, 1000);
    setup_row(&mut e, t0, 300, 1.5, 2000);
    // +65% on t=0 but only +9.9% on one window ago (t=300): not a SETUP
    assert_eq!(setup_row(&mut e, t0, 600, 1.649, 5000).new_alerts, 0);
    assert_eq!(setup_row(&mut e, t0, 600, 1.65, 5000).active[0].status, Some(AlertKind::Setup));
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
    setup_row(&mut e, t0, 300, 1.0, 2000);
    assert_eq!(setup_row(&mut e, t0, 600, 1.2, 4999).new_alerts, 0);
    // price below the gate (volume above)
    let mut e = Engine::new(&cfg);
    setup_row(&mut e, t0, 0, 0.5, 10_000);
    setup_row(&mut e, t0, 300, 0.5, 11_000);
    assert_eq!(setup_row(&mut e, t0, 600, 0.99, 30_000).new_alerts, 0);
    // exactly at both gates
    let mut e = Engine::new(&cfg);
    setup_row(&mut e, t0, 0, 0.5, 1000);
    setup_row(&mut e, t0, 300, 0.5, 2000);
    assert_eq!(setup_row(&mut e, t0, 600, 1.0, 5000).new_alerts, 1);
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
    e.apply_quotes(&[at(upd(7, 301_000, 2, 1.0, 2000), 310)], Session::Reg, t0 + Duration::from_secs(310));
    e.apply_quotes(&[at(upd(7, 601_000, 3, 1.10, 4200), 610)], Session::Reg, t0 + Duration::from_secs(610));
    // a stale REST row; the fresh stream quote overrides it
    let rest = Ticker { ticker_id: 7, prev_close: 1.0, ..tk("A", 1.0, 1000, 0.0, 5e6) };
    let out = e.process(vec![rest], "change", true, t0 + Duration::from_secs(611));
    assert_eq!(out.active[0].status, Some(AlertKind::Setup));
    let a = e.alerts().back().unwrap();
    assert_eq!((a.prev_vol, a.curr_vol), (1000, 2200));
}

/// Lists row "A" (id 1) over REST at `t0`, then lets the stream trade 1000 in one 300 s window
/// and exactly 2.2x that (+10%) in the next; no REST fetch follows. Returns the time of that
/// last quote, 610 s on.
fn stream_setup_hit(e: &mut Engine, t0: Instant) -> Instant {
    setup_row(e, t0, 0, 1.0, 1000);
    e.set_rest_feeds_series(false);
    for (s, ms, seq, px, vol) in [(10, 10_000, 1, 1.0, 1000), (310, 310_000, 2, 1.0, 2000), (610, 610_000, 3, 1.10, 4200)] {
        let mut u = upd(1, ms, seq, px, vol);
        u.recv = t0 + Duration::from_secs(s);
        e.apply_quotes(&[u], Session::Reg, t0 + Duration::from_secs(s));
    }
    t0 + Duration::from_secs(610)
}

#[test]
fn setup_is_raised_by_the_tick_once_per_cooldown() {
    let mut e = setup_engine();
    let t0 = Instant::now();
    let at = |s: u64| t0 + Duration::from_secs(s);
    stream_setup_hit(&mut e, t0);
    // the stream data alone raises nothing; the tick does
    assert!(kinds(&e).is_empty());

    assert_eq!(e.recompute(at(610)), 1);
    assert_eq!(kinds(&e), vec![AlertKind::Setup]);
    assert_eq!(status_at(&e, at(610)), "SETUP");
    let a = e.alerts().back().unwrap();
    assert_eq!((a.at, a.prev_vol, a.curr_vol), (at(610), 1000, 2200));

    // the limits still hold on every following tick, but the cooldown keeps it to one alert
    for s in (611..730).step_by(3) {
        assert_eq!(e.recompute(at(s)), 0, "second {s}");
    }
    assert_eq!(e.recompute(at(729)), 0);
    assert_eq!(kinds(&e).len(), 1);
    // once the cooldown (120 s) is over and the condition still holds, it is raised again
    assert_eq!(e.recompute(at(730)), 1);
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
fn setup_met_edges() {
    let th = Thresholds::from_config(&Config::default());
    let b = |prev, cur, px| Some(SetupBase { prev, cur, px });
    assert!(setup_met(&th, b(1000, 2200, 1.0), 1.10));
    assert!(!setup_met(&th, b(1000, 2199, 1.0), 1.10));
    assert!(!setup_met(&th, b(1000, 2200, 1.0), 1.0999));
    assert!(!setup_met(&th, None, 1.10));
    // a quiet previous window: anything traded now counts, nothing traded does not
    assert!(setup_met(&th, b(0, 1, 1.0), 1.10));
    assert!(!setup_met(&th, b(0, 0, 1.0), 1.10));
    assert!(!setup_met(&th, b(1000, 2200, 0.0), 1.10));
    // a falling price never counts
    assert!(!setup_met(&th, b(1000, 5000, 1.0), 0.5));
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
    setup_row(&mut e, t0, 60, 1.0, 2000);
    let out = setup_row(&mut e, t0, 120, 1.05, 3500);
    assert_eq!(out.active[0].status, Some(AlertKind::Setup));
}
