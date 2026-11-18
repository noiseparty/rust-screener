use super::*;

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
fn float_and_share_filters_pass_unknown_values() {
    let known = Ticker { float: 5e6, total_shares: 8e6, ..tk("A", 5.0, 1000, 0.25, 1e6) };
    let unknown = tk("U", 5.0, 1000, 0.25, 1e6);
    for f in [
        Filters { min_float: 6e6, ..Default::default() },
        Filters { max_float: 4e6, ..Default::default() },
        Filters { min_shares: 9e6, ..Default::default() },
        Filters { max_shares: 7e6, ..Default::default() },
    ] {
        assert!(!f.matches(&known), "{f:?}");
        assert!(f.matches(&unknown), "{f:?}");
    }
    assert!(Filters { min_float: 1e6, max_float: 1e7, min_shares: 1e6, max_shares: 1e7, ..Default::default() }.matches(&known));
    // only the float is known: the share bounds still pass
    let half = Ticker { float: 5e6, ..unknown.clone() };
    assert!(Filters { max_shares: 1.0, ..Default::default() }.matches(&half));
    assert!(!Filters { max_float: 1.0, ..Default::default() }.matches(&half));
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
    assert!(e.candles("BL").is_some());
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
    e.process(vec![Ticker { ticker_id: 1, ..tk("A", 1.0, 1000, 0.1, 0.0) }, tk("B", 9.0, 1000, 0.2, 0.0)], "change", true, t0);
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
