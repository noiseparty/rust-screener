//! Recorded sessions replayed through the app (`tests/fixtures/*.replay`). Each fixture's header
//! explains the market it holds; the numbers below follow from the default config
//! (300 s SETUP window, 2.2x volume, +10% price, 120 s cooldown, 180 s Vol/m, 1 s recompute).

use super::*;

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

fn kinds(r: &Replay) -> Vec<(String, &'static str)> {
    r.alerts().into_iter().map(|a| (a.symbol, a.kind)).collect()
}

fn rate(r: &Replay, sym: &str) -> VolM {
    r.row(sym).unwrap_or_else(|| panic!("{sym} not on the list")).vol_m
}

#[test]
fn quiet_session_lists_sorted_rows_with_steady_vol_m_and_no_alerts() {
    let mut r = Replay::load("quiet.replay");
    // the list is fetched at the first tick and answered 200 ms later
    r.run_to_secs(0.1);
    assert!(r.rows().is_empty());
    r.run_to_secs(0.5);
    assert_eq!(r.symbols(), ["DDD", "BBB", "AAA", "CCC"]);
    assert!(r.app.stream_ok());
    // Vol/m needs 30 s of history; then it is partial until the 180 s window is covered
    r.run_to_secs(20.0);
    assert_eq!(rate(&r, "AAA"), VolM::Pending);
    r.run_to_secs(60.0);
    // AAA's first quote at 1 s, five more by 60 s: 5,000 shares over the 58-59 s up to the
    // last recompute
    match rate(&r, "AAA") {
        VolM::Rate { per_min, partial: true } => assert!((5000.0 * 60.0 / 59.0..5000.0 * 60.0 / 57.9).contains(&per_min), "{per_min}"),
        v => panic!("AAA Vol/m at 60 s: {v:?}"),
    }
    r.run_to_secs(300.0);
    for (sym, per_min) in [("AAA", 6000.0), ("BBB", 15000.0), ("CCC", 3000.0), ("DDD", 24000.0)] {
        assert_eq!(rate(&r, sym), VolM::Rate { per_min, partial: false }, "{sym}");
    }
    // prices wiggle a cent around the REST price; the order holds and nothing is flagged
    assert_eq!(r.symbols(), ["DDD", "BBB", "AAA", "CCC"]);
    assert!(r.rows().iter().all(|row| !row.dim && row.status.is_none()));
    assert_eq!(r.row("AAA").unwrap().volume, 900_000 + 29_000);
    assert!(r.alerts().is_empty(), "{:?}", r.alerts());
    assert!(r.app.error.is_none());
}

#[test]
fn volume_and_price_spike_raises_one_setup_held_for_the_cooldown() {
    let mut r = Replay::load("setup.replay");
    r.run_to_secs(610.0);
    assert!(r.alerts().is_empty(), "{:?}", r.alerts());
    assert_eq!(r.symbols(), ["BBB", "XYZ", "AAA"]);

    // the burst quote lands at 611 s; the next recompute (at most 1 s later) raises SETUP
    r.run_to_secs(612.1);
    let fired = r.alerts();
    assert_eq!(kinds(&r), [("XYZ".to_string(), "SETUP")]);
    let at = fired[0].at;
    assert!(at >= secs(611.0) && at <= secs(612.1), "{at:?}");
    // 70,000 shares in the last 300 s vs 30,000 before; 2.30 vs 2.00
    assert_eq!(fired[0].detail, "vol 2.3x  +15.0%");
    let a = &r.app.engine.alerts()[0];
    assert_eq!((a.prev_vol, a.curr_vol, a.prev_price, a.curr_price), (30_000, 70_000, 2.00, 2.30));
    // +43.75% on its 1.60 close: XYZ is re-sorted to the top on the same tick, with its badge
    assert_eq!(r.symbols(), ["XYZ", "BBB", "AAA"]);
    assert_eq!(r.row("XYZ").unwrap().status, Some("SETUP"));

    // the limits stay met until 711 s, but the cooldown holds: no second SETUP
    r.run_to(at + secs(119.0));
    assert_eq!(r.alerts().len(), 1);
    assert_eq!(r.row("XYZ").unwrap().status, Some("SETUP"));
    // the price eased at 711 s (+32.5% on the day): back under BBB
    assert_eq!(r.symbols(), ["BBB", "XYZ", "AAA"]);
    // the badge goes at the first recompute (ticks are 80 ms, so about every 1.04 s) once the
    // 120 s cooldown has ended
    r.run_to(at + secs(121.1));
    assert_eq!(r.row("XYZ").unwrap().status, None);
    r.run_to_secs(800.0);
    assert_eq!(kinds(&r), [("XYZ".to_string(), "SETUP")]);
}

#[test]
fn ticker_joining_the_list_after_startup_raises_one_new() {
    let mut r = Replay::load("new.replay");
    r.run_to_secs(94.0);
    assert_eq!(r.symbols(), ["BBB", "AAA", "CCC"]);
    // the startup list only seeds NEW
    assert!(r.alerts().is_empty(), "{:?}", r.alerts());

    // NEWW is on the server's list from 95 s; the app polls every 20 s while the stream is up
    r.run_to_secs(116.0);
    assert_eq!(kinds(&r), [("NEWW".to_string(), "NEW")]);
    let at = r.alerts()[0].at;
    assert!(at > secs(95.0) && at <= secs(116.0), "{at:?}");
    assert_eq!(r.alerts()[0].detail, "appeared");
    assert_eq!(r.symbols(), ["NEWW", "BBB", "AAA", "CCC"]);
    assert_eq!(r.row("NEWW").unwrap().status, Some("NEW"));

    // it leaves the list at 180 s (gone at the next poll, no grace: it is not on the list at all)
    r.run_to_secs(201.0);
    assert!(r.row("NEWW").is_none());
    // and comes back at 240 s, inside NEW_QUIET: no second NEW
    r.run_to_secs(300.0);
    assert_eq!(r.symbols(), ["NEWW", "BBB", "AAA", "CCC"]);
    assert_eq!(kinds(&r), [("NEWW".to_string(), "NEW")]);
}

#[test]
fn row_falling_below_a_filter_is_dimmed_for_a_minute_then_hidden() {
    let mut r = Replay::load("dim.replay");
    r.run_to_secs(123.0);
    assert_eq!(r.symbols(), ["AAA", "FADE"]);
    assert!(!r.row("FADE").unwrap().dim);

    // the quote at 124 s puts FADE at +8%, under Min Change 10%
    r.run_to_secs(125.5);
    let fade = r.row("FADE").unwrap();
    assert!(fade.dim);
    assert!((fade.change_pct - 8.0).abs() < 1e-6, "{}", fade.change_pct);
    // still dimmed through REST polls (the fresh quote beats the REST row's +12%)
    r.run_to_secs(183.0);
    assert!(r.row("FADE").unwrap().dim);
    // grace started at the first recompute after 124 s (by 125.1 s); the first recompute after
    // its 60 s hides the row
    r.run_to_secs(186.2);
    assert_eq!(r.symbols(), ["AAA"]);
    r.run_to_secs(240.0);
    assert_eq!(r.symbols(), ["AAA"]);
    assert!(r.alerts().is_empty(), "{:?}", r.alerts());
}

#[test]
fn silent_stream_falls_back_to_rest_and_a_dead_feed_hides_vol_m() {
    let mut r = Replay::load("stale.replay");
    r.run_to_secs(66.5);
    assert!(r.app.stream_ok());
    assert!(matches!(rate(&r, "AAA"), VolM::Rate { partial: true, .. }));
    assert_eq!(r.app.poll_interval(), LIST_REFRESH);

    // 15 s after BBB's last quote at 66 s the stream counts as down: REST every 5 s
    r.run_to_secs(81.5);
    assert!(!r.app.stream_ok());
    assert_eq!(r.app.poll_interval(), 5);
    // REST rows now feed the series, which starts over: Vol/m waits for 30 s of REST points
    r.run_to_secs(95.0);
    assert_eq!(rate(&r, "AAA"), VolM::Pending);
    assert_eq!(r.row("AAA").unwrap().price, 3.00);

    // the server's new list shows up at the next 5 s poll
    r.run_to_secs(105.5);
    let aaa = r.row("AAA").unwrap();
    assert_eq!((aaa.price, aaa.volume), (3.30, 1_050_000));
    assert!((aaa.change_pct - 32.0).abs() < 1e-6);
    r.run_to_secs(140.0);
    assert!(matches!(rate(&r, "AAA"), VolM::Rate { .. }), "{:?}", rate(&r, "AAA"));

    // REST fails from 150 s; Vol/m shows a dash once no list has come for 15 s
    // (the last good list came between 144.7 and 150 s)
    r.run_to_secs(159.0);
    assert_eq!(r.app.error.as_deref(), Some("HTTP 503"));
    assert!(r.rows().iter().all(|row| row.vol_m != VolM::Stale));
    r.run_to_secs(171.0);
    assert!(r.rows().iter().all(|row| row.vol_m == VolM::Stale), "{:?}", r.rows());
    // the rows stay listed with their last prices
    assert_eq!(r.symbols(), ["BBB", "AAA"]);
    assert_eq!(r.row("AAA").unwrap().price, 3.35);

    // REST is back at 200 s: the next poll (by 205.5 s) has the new prices; Vol/m returns on
    // the recompute after it
    r.run_to_secs(205.5);
    assert!(r.app.error.is_none());
    assert_eq!(r.row("AAA").unwrap().price, 3.40);
    r.run_to_secs(207.0);
    assert!(r.rows().iter().all(|row| row.vol_m != VolM::Stale));
    assert!(r.alerts().is_empty(), "{:?}", r.alerts());
}
