use super::*;

#[test]
fn listed_tickers_are_preloaded_in_list_order_leaving_a_worker_free() {
    let (mut app, qrx) = chart_app();
    let t0 = Instant::now();
    fetches(&mut app, t0);
    let first = sent(&qrx);
    let full = history::Mode::Full;
    assert_eq!(first, vec![("A".into(), Tf::D1, full), ("A".into(), Tf::M5, full), ("A".into(), Tf::H4, full)]);
    // nothing more until one answers
    fetches(&mut app, t0);
    assert!(sent(&qrx).is_empty());
    answer(&mut app, "A", Tf::D1, full, vec![bar(0, 1.0)]);
    // an empty answer is not held, so it is tried again later
    answer(&mut app, "A", Tf::M5, full, vec![]);
    fetches(&mut app, t0);
    assert_eq!(sent(&qrx), vec![("A".into(), Tf::H1, full), ("B".into(), Tf::D1, full)]);
    let later = Instant::now() + HISTORY_RETRY;
    answer(&mut app, "A", Tf::H1, full, vec![bar(0, 1.0)]);
    fetches(&mut app, later);
    assert_eq!(sent(&qrx), vec![("A".into(), Tf::M5, full)]);
}

#[test]
fn charted_ticker_gets_its_chart_refreshes_and_older_bars() {
    use history::Mode::{Before, Full, Since};
    let (mut app, qrx) = chart_app();
    app.tickers.truncate(1);
    let t0 = Instant::now();
    let charted = |rx: &std::sync::mpsc::Receiver<history::Req>| {
        sent(rx).into_iter().filter(|(s, tf, _)| s == "A" && *tf == Tf::H1).map(|r| r.2).collect::<Vec<_>>()
    };
    app.on_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
    assert_eq!(app.timeframe, Tf::H1);
    fetches(&mut app, t0);
    // 1h is preloaded last (D, 5m and 4h fill the preload slots); the charted request
    // waits for the dwell and then takes the free worker
    assert!(charted(&qrx).is_empty());
    fetches(&mut app, t0 + BACKFILL_DWELL);
    assert_eq!(charted(&qrx), vec![Full]);
    assert!(app.bars_loading("A"));
    // a progress message shows bars but keeps the request pending
    app.on_msg(Msg::Bars { symbol: "A".into(), tf: Tf::H1, mode: Full, done: false, failed: false, bars: vec![bar(3600, 1.0)] });
    assert!(app.bars_loading("A"));
    fetches(&mut app, t0 + BACKFILL_DWELL);
    assert!(charted(&qrx).is_empty());
    answer(&mut app, "A", Tf::H1, Full, (0..48).map(|i| bar(i * 3600, 1.0)).collect());
    assert!(!app.bars_loading("A"));
    app.hits.chart_fit = 20;
    // shown and fresh: nothing; after HISTORY_REFRESH the newest bars back to the last held
    fetches(&mut app, Instant::now() + BACKFILL_DWELL);
    assert!(charted(&qrx).is_empty());
    fetches(&mut app, Instant::now() + HISTORY_REFRESH);
    assert_eq!(charted(&qrx), vec![Since(47 * 3600)]);
    answer(&mut app, "A", Tf::H1, Since(47 * 3600), vec![bar(47 * 3600, 2.0), bar(48 * 3600, 3.0)]);
    assert_eq!(app.bars[&("A".to_string(), Tf::H1)].len(), 49);
    // scrolled back near the oldest bar: older bars
    app.pan = 30;
    fetches(&mut app, Instant::now() + BACKFILL_DWELL);
    assert_eq!(charted(&qrx), vec![Before(0)]);
    answer(&mut app, "A", Tf::H1, Before(0), vec![bar(-3600, 0.5)]);
    assert_eq!(app.bars[&("A".to_string(), Tf::H1)][0].start, -3600);
    fetches(&mut app, Instant::now() + BACKFILL_DWELL);
    assert_eq!(charted(&qrx), vec![Before(-3600)]);
    // nothing older: no more asking for older bars, only refreshes
    answer(&mut app, "A", Tf::H1, Before(-3600), vec![]);
    fetches(&mut app, Instant::now() + HISTORY_REFRESH);
    assert_eq!(charted(&qrx), vec![Since(48 * 3600)]);
}

#[test]
fn dragging_the_chart_pans_and_the_price_axis_scales() {
    let (mut app, _qrx) = chart_app();
    for i in 0..40 {
        app.engine.record_sample("A", i * 60, 1.0, i);
    }
    app.hits.chart_plot = Rect::new(60, 2, 40, 20);
    app.hits.chart_axis = Rect::new(100, 2, 10, 20);
    app.hits.chart_fit = 20;
    let ev = |kind, x, y| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70, 5));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 80, 5));
    assert_eq!(app.pan, 5);
    // one column at two columns per bar rounds to a bar (no dead zone around the press)
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 71, 5));
    assert_eq!(app.pan, 1);
    // dragging left past the newest bar leaves empty space, up to half the plot; dragging
    // right stops with a quarter of the plot still showing bars
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 40, 5));
    assert_eq!(app.pan, -10);
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 200, 5));
    assert_eq!(app.pan, 35);
    // dragging down moves the price range up with the pointer (14 plot rows)
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 200, 12));
    assert_eq!(app.shift, 0.5);
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 200, 12));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 10, 5));
    assert_eq!((app.pan, app.shift), (35, 0.5));
    // the wheel zooms in time and never moves the row cursor; zooming in keeps the scroll
    // inside what is held, zooming stops at both ends
    let row = app.row;
    app.on_mouse(ev(MouseEventKind::ScrollDown, 70, 5));
    assert_eq!(app.scale, crate::chart::DEFAULT_SCALE + 1);
    app.hits.chart_fit = 30;
    app.on_mouse(ev(MouseEventKind::ScrollUp, 70, 5));
    assert_eq!((app.scale, app.pan), (crate::chart::DEFAULT_SCALE, 33));
    for _ in 0..20 {
        app.on_mouse(ev(MouseEventKind::ScrollUp, 70, 5));
    }
    assert_eq!(app.scale, 0);
    for _ in 0..20 {
        app.on_mouse(ev(MouseEventKind::ScrollDown, 70, 5));
    }
    assert_eq!(app.scale, crate::chart::ZOOMS.len() - 1);
    assert_eq!(app.row, row);
    // dragging a zoomed-out chart moves more bars per column
    app.hits.chart_fit = 20;
    app.pan = 0;
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70, 5));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 71, 5));
    assert_eq!(app.pan, crate::chart::ZOOMS[app.scale].1 as i64);
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 71, 5));
    app.on_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
    assert_eq!((app.pan, app.scale, app.shift), (0, crate::chart::DEFAULT_SCALE, 0.0));
    // price axis: down widens the range
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 105, 5));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 105, 8));
    assert!(app.zoom > 1.0);
    app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 105, 8));
    // double click on the axis fits again (and drops a vertical move); `0` resets the whole view
    app.shift = 0.3;
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 105, 5));
    assert_eq!((app.zoom, app.shift), (1.0, 0.0));
    app.zoom = 3.0;
    app.on_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
    assert_eq!((app.pan, app.zoom), (0, 1.0));
    // a new timeframe starts at the newest bars, fitted
    (app.pan, app.shift) = (7, 0.2);
    app.set_timeframe(Tf::D1);
    assert_eq!((app.pan, app.shift), (0, 0.0));
}

#[test]
fn a_history_shorter_than_the_plot_still_drags() {
    let (mut app, _qrx) = chart_app();
    for i in 0..10 {
        app.engine.record_sample("A", i * 60, 1.0, i);
    }
    app.hits.chart_plot = Rect::new(60, 2, 40, 20);
    app.hits.chart_fit = 20;
    let ev = |kind, x| MouseEvent { kind, column: x, row: 5, modifiers: KeyModifiers::NONE };
    app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70));
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 76));
    assert_eq!(app.pan, 3);
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 100));
    assert_eq!(app.pan, 5);
    app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 60));
    assert_eq!(app.pan, -5);
}

#[test]
fn wheel_zooms_only_over_the_chart_and_t_cycles_timeframes() {
    let (mut app, _qrx) = chart_app();
    app.hits.chart_plot = Rect::new(60, 2, 40, 20);
    app.hits.chart_axis = Rect::new(100, 2, 10, 20);
    app.hits.chart_cols = 50;
    let wheel = |app: &mut App, kind, x| app.on_mouse(MouseEvent { kind, column: x, row: 5, modifiers: KeyModifiers::NONE });
    let (row, scale) = (app.row, app.scale);
    wheel(&mut app, MouseEventKind::ScrollDown, 10); // over the list: nothing
    assert_eq!((app.row, app.scale), (row, scale));
    wheel(&mut app, MouseEventKind::ScrollDown, 105); // over the price axis: zoom out
    assert_eq!(app.scale, scale + 1);
    assert_eq!(app.hits.chart_fit, crate::chart::fit(50, scale + 1));
    app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert_eq!(app.timeframe, Tf::M5);
    app.set_timeframe(Tf::D1);
    app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert_eq!(app.timeframe, Tf::M1);
}

#[test]
fn failed_requests_are_retried_not_taken_as_no_data() {
    use history::Mode::{Before, Full};
    let (mut app, qrx) = chart_app();
    app.tickers.truncate(1);
    app.set_timeframe(Tf::H1);
    let t0 = Instant::now();
    fetches(&mut app, t0);
    let _ = sent(&qrx);
    answer(&mut app, "A", Tf::H1, Full, (0..48).map(|i| bar(i * 3600, 1.0)).collect());
    app.hits.chart_fit = 20;
    app.pan = 30;
    fetches(&mut app, Instant::now() + BACKFILL_DWELL);
    assert!(sent(&qrx).iter().any(|r| r.2 == Before(0)));
    // a failed empty answer does not mark the chart as having nothing older
    app.on_msg(Msg::Bars { symbol: "A".into(), tf: Tf::H1, mode: Before(0), done: true, failed: true, bars: vec![] });
    fetches(&mut app, Instant::now() + BACKFILL_DWELL);
    assert!(sent(&qrx).iter().any(|r| r.2 == Before(0)));
}

#[test]
fn clicking_an_alert_for_a_ticker_off_the_list_opens_its_chart() {
    let (mut app, _qrx) = chart_app();
    app.hits.alert_rows = Rect::new(1, 2, 40, 1);
    app.hits.alert_syms = vec!["ZZZ".into()];
    app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 3, row: 2, modifiers: KeyModifiers::NONE });
    assert!(app.view == View::Chart);
    assert_eq!(app.chart_symbol, "ZZZ");
}

#[test]
fn mouse_selects_rows_tabs_and_footer_keys() {
    let (mut app, _qrx) = chart_app();
    app.hits.gainer_rows = Rect::new(1, 5, 40, 2);
    app.hits.tf_tabs = vec![(Rect::new(50, 0, 4, 1), Tf::D1)];
    app.hits.keys = vec![(Rect::new(0, 20, 5, 1), KeyCode::Char('v'))];
    app.hits.alert_rows = Rect::new(1, 2, 40, 1);
    app.hits.alert_syms = vec!["B".into()];
    let click = |x, y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE };
    app.on_mouse(click(3, 6));
    assert_eq!(app.row, 1);
    app.on_mouse(click(3, 5));
    assert_eq!(app.row, 0);
    app.on_mouse(click(3, 2)); // alert row for B
    assert_eq!(app.row, 1);
    app.on_mouse(click(51, 0));
    assert_eq!(app.timeframe, Tf::D1);
    // clicking the selected row again keeps the dashboard
    app.on_mouse(click(3, 6));
    assert!(app.view == View::Dashboard);
    assert_eq!(app.row, 1);
    app.on_mouse(click(2, 20));
    assert!(app.view == View::Columns);
}
