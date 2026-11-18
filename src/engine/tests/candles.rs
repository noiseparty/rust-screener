use super::*;

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
fn completed_candles_are_taken_once() {
    let mut e = engine();
    let take = |e: &mut Engine, now| {
        let rows = e.completed_candles(now);
        e.mark_stored(&rows);
        rows
    };
    e.record_sample("A", 600, 1.0, 1000);
    e.record_sample("A", 660, 2.0, 1100);
    e.record_sample("B", 610, 5.0, 10);
    assert!(take(&mut e, 650).is_empty()); // minute 600 still running
    let got = take(&mut e, 661);
    assert_eq!(got.len(), 2);
    assert!(got.iter().any(|(s, c)| s == "A" && c.start == 600) && got.iter().any(|(s, c)| s == "B" && c.start == 600));
    assert!(take(&mut e, 700).is_empty());
    let got = take(&mut e, 721); // minute 660 of A ended
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].0.as_str(), got[0].1.start), ("A", 660));
    assert!(take(&mut e, 10_000).is_empty());
}

#[test]
fn loaded_candles_are_not_stored_again() {
    let mut e = engine();
    let take = |e: &mut Engine, now| {
        let rows = e.completed_candles(now);
        e.mark_stored(&rows);
        rows
    };
    let c = |start| Candle { start, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1 };
    // saved candles go in front and are marked as stored (as App::load_saved_candles does)
    let saved = [c(600), c(660)];
    e.insert_candles("A", &saved);
    e.mark_stored(&saved.iter().map(|b| ("A".to_string(), *b)).collect::<Vec<_>>());
    assert_eq!(e.candles("A").unwrap().len(), 2);
    assert!(take(&mut e, 10_000).is_empty());
    e.record_sample("A", 780, 2.0, 5); // a later minute appends after the reloaded ones
    assert_eq!(e.candles("A").unwrap().len(), 3);
    let got = take(&mut e, 10_000);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1.start, 780);
}

#[test]
fn backfill_fills_gaps_and_never_replaces_our_candles() {
    let mut e = engine();
    let c = |start, p| Candle { start, open: p, high: p, low: p, close: p, volume: 1 };
    e.record_sample("A", 300, 5.0, 100);
    // 300 is a minute we hold: ours is kept; earlier bars go in front, in order
    let added = e.insert_candles("A", &[c(120, 1.0), c(180, 2.0), c(240, 3.0), c(300, 9.0)]);
    assert_eq!(added.iter().map(|c| c.start).collect::<Vec<_>>(), vec![120, 180, 240]);
    let starts = |e: &Engine| e.candles("A").unwrap().iter().map(|c| c.start).collect::<Vec<i64>>();
    assert_eq!(starts(&e), vec![120, 180, 240, 300]);
    assert_eq!(e.candles("A").unwrap()[3].close, 5.0);
    assert_eq!(e.first_candle_start("A"), Some(120));
    // a gap in the middle is filled in order, repeats are skipped
    e.record_sample("A", 600, 6.0, 200);
    assert_eq!(e.insert_candles("A", &[c(480, 4.0), c(420, 3.5), c(240, 0.1)]).len(), 2);
    assert_eq!(starts(&e), vec![120, 180, 240, 300, 420, 480, 600]);
    // a symbol with nothing yet takes the bars as they are
    e.insert_candles("B", &[c(60, 1.0), c(120, 2.0)]);
    assert_eq!(e.candles("B").unwrap().len(), 2);
    assert_eq!(e.first_candle_start("C"), None);
    assert_eq!(e.stored_upto("A"), None);
}
