use super::*;

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
    assert!(out.active.iter().all(|t| t.status.is_none()));
    // the same list again, and ticks in between, stay silent
    assert_eq!(e.recompute(t0 + Duration::from_secs(1)), 0);
    assert_eq!(list(&mut e, t0, 5, &["A", "B", "C"]).new_alerts, 0);
    // a symbol that joins afterwards is NEW
    let out = list(&mut e, t0, 10, &["A", "B", "C", "D"]);
    assert_eq!(out.new_alerts, 1);
    assert_eq!(kinds(&e), vec![AlertKind::New]);
    assert_eq!(e.alerts().back().unwrap().symbol, "D");
    assert_eq!(out.active.iter().find(|t| t.symbol == "D").unwrap().status, Some(AlertKind::New));
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
        let mut v = vec![Ticker { ticker_id: 1, ..tk("A", 1.0, 1000, 0.1, 0.0) }, Ticker { ticker_id: 2, ..tk("B", 5.0, 1000, 0.1, 0.0) }];
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
