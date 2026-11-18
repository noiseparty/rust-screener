use super::*;
use crate::session::Session;

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
