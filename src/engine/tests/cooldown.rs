use super::*;

#[test]
fn cooldown_is_per_symbol_and_kind() {
    let mut e = engine();
    let t0 = Instant::now();
    let at = |s: u64| t0 + Duration::from_secs(s);
    let a = tk("A", 1.0, 1000, 0.1, 0.0);
    let b = tk("B", 1.0, 1000, 0.1, 0.0);
    assert!(e.raise(AlertKind::Setup, &a, None, at(0), 0));
    // same symbol and kind: held back for the whole cooldown
    assert!(!e.raise(AlertKind::Setup, &a, None, at(1), 0));
    assert!(!e.raise(AlertKind::Setup, &a, None, at(119), 0));
    // another kind of the same symbol, and the same kind of another symbol, are independent
    assert!(e.raise(AlertKind::New, &a, None, at(2), 0));
    assert!(e.raise(AlertKind::Setup, &b, None, at(3), 0));
    assert_eq!(e.alerts().len(), 3);
    // the cooldown counts from the last raise
    assert!(e.raise(AlertKind::Setup, &a, None, at(120), 0));
    assert!(!e.raise(AlertKind::Setup, &a, None, at(239), 0));
    assert!(e.raise(AlertKind::Setup, &a, None, at(240), 0));
}

#[test]
fn cooldown_follows_the_configured_seconds() {
    let mut e = engine();
    e.apply_config(&Config { alert_cooldown_sec: 10, max_market_cap: 0.0, ..Config::default() });
    let t0 = Instant::now();
    let a = tk("A", 1.0, 1000, 0.1, 0.0);
    assert!(e.raise(AlertKind::Setup, &a, None, t0, 0));
    assert!(!e.raise(AlertKind::Setup, &a, None, t0 + Duration::from_secs(9), 0));
    assert!(e.raise(AlertKind::Setup, &a, None, t0 + Duration::from_secs(10), 0));
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
    assert_eq!(out.active[0].status, Some(AlertKind::New));
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
    assert!(e.raise(AlertKind::Setup, &a, None, at(30), 0));
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
    assert!(!e.raise(AlertKind::Setup, &a, None, Instant::now(), 0));
    assert!(e.alerts().is_empty() && e.fired.is_empty());
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
