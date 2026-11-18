use super::*;

#[test]
fn a_failed_backfill_is_asked_again_after_the_retry_delay() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let (btx, brx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
    app.attach_backfill(btx);
    app.view = View::Dashboard;
    app.tickers = vec![Ticker { symbol: "A".into(), ticker_id: 1, ..Default::default() }];
    app.all = app.tickers.clone();
    let t0 = Instant::now();
    fetches(&mut app, t0);
    assert_eq!(brx.try_recv().unwrap().symbol, "A");
    app.on_msg(Msg::Backfill { symbol: "A".into(), bars: vec![], failed: true });
    fetches(&mut app, Instant::now());
    assert!(brx.try_recv().is_err());
    fetches(&mut app, Instant::now() + HISTORY_RETRY);
    assert_eq!(brx.try_recv().unwrap().symbol, "A");
}

#[test]
fn backfill_every_listed_ticker_once_per_symbol() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let (btx, brx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
    app.attach_backfill(btx);
    app.view = View::Dashboard;
    let mk = |s: &str, id| Ticker { symbol: s.into(), ticker_id: id, ..Default::default() };
    app.tickers = vec![mk("A", 1), mk("B", 2), mk("Z", 0)];
    app.all = app.tickers.clone();
    let jobs = |rx: &std::sync::mpsc::Receiver<backfill::Job>| rx.try_iter().map(|j| (j.symbol, j.ticker_id, j.before)).collect::<Vec<_>>();
    let t0 = Instant::now();
    // list order, no dwell; Z has no tickerId so it cannot be backfilled
    fetches(&mut app, t0);
    let first = jobs(&brx);
    assert_eq!(first.iter().map(|j| (j.0.as_str(), j.1, j.2 % 60)).collect::<Vec<_>>(), vec![("A", 1, 0), ("B", 2, 0)]);
    // once per symbol, also once charted
    fetches(&mut app, t0 + BACKFILL_DWELL * 3);
    assert!(jobs(&brx).is_empty());
    // the first candle we hold bounds the backfill
    app.engine.record_sample("C", 6000, 1.0, 10);
    app.tickers.push(mk("C", 3));
    app.all = app.tickers.clone();
    fetches(&mut app, t0 + BACKFILL_DWELL * 4);
    assert_eq!(jobs(&brx), vec![("C".to_string(), 3, 6000)]);
    // backfilled bars go in front of the candles we built
    let bar = Candle { start: 5940, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 5 };
    app.on_msg(Msg::Backfill { symbol: "C".into(), bars: vec![bar], failed: false });
    assert_eq!(app.engine.first_candle_start("C"), Some(5940));
}

#[test]
fn stream_batches_wait_for_the_recompute_tick() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let cfg = Config { recompute_ms: 500, max_market_cap: 0.0, ..Config::default() };
    let mut app = App::new(cfg, PathBuf::from("unused-test-config.json"), tx, htx, None);
    let row = |s: &str, id: u64, chg: f64| Ticker {
        ticker_id: id,
        symbol: s.into(),
        price: 1.0 + chg,
        prev_close: 1.0,
        volume: 1000,
        change_ratio: chg,
        ..Default::default()
    };
    let base = app.last_recompute;
    let out = app.engine.process(vec![row("A", 1, 0.10), row("B", 2, 0.05)], "change", true, base);
    app.tickers = out.active;
    assert_eq!(app.tickers[0].symbol, "A");
    // B jumps past A on the stream: the data changes, the table does not
    let quote = stream::Update {
        id: 2,
        trade_ms: chrono::Utc::now().timestamp_millis(),
        seq: 1,
        close: Some(2.0),
        p_price: None,
        change_ratio: None,
        volume: Some(2000),
        mcap: None,
        high: None,
        low: None,
        status: String::new(),
        recv: Instant::now(),
    };
    app.on_msg(Msg::Quotes(vec![quote]));
    assert_eq!(app.tickers[0].symbol, "A");
    app.on_tick(base + Duration::from_millis(400));
    assert_eq!(app.tickers[0].symbol, "A");
    // the tick (500 ms here) re-sorts
    app.on_tick(base + Duration::from_millis(500));
    assert_eq!(app.tickers[0].symbol, "B");
    assert_eq!(app.last_recompute, base + Duration::from_millis(500));
}

#[test]
fn alert_sound_follows_the_alert_filter_mute_and_volume() {
    let (mut app, _rx) = test_app(Config::default());
    // a tick without alerts is silent; one with several sounds once
    assert!(!app.sound_due(&[]));
    assert!(app.sound_due(&[AlertKind::New, AlertKind::Setup]));
    // ALL with NEW switched off in Alert Filters
    app.cfg.filter_enabled.insert("NEW".into(), false);
    assert!(!app.sound_due(&[AlertKind::New]));
    assert!(app.sound_due(&[AlertKind::New, AlertKind::Setup]));
    // the SETUP filter (f cycle) passes only SETUP alerts
    app.alert_filter = FILTER_NAMES.iter().position(|&n| n == "SETUP").unwrap();
    assert!(!app.sound_due(&[AlertKind::New]));
    assert!(app.sound_due(&[AlertKind::Setup]));
    app.cfg.sound_muted = true;
    assert!(!app.sound_due(&[AlertKind::Setup]));
    app.cfg.sound_muted = false;
    app.cfg.sound_volume = 0;
    assert!(!app.sound_due(&[AlertKind::Setup]));
}
