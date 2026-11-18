use super::*;

#[test]
fn cursor_follows_selected_symbol_across_resort() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
    let mk = |s: &str| Ticker { symbol: s.into(), ..Default::default() };
    app.tickers = vec![mk("A"), mk("B"), mk("C")];
    app.move_row(1);
    assert_eq!(app.sel_symbol.as_deref(), Some("B"));
    app.tickers = vec![mk("C"), mk("A"), mk("B")];
    app.resync_row();
    assert_eq!(app.row, 2);
    app.tickers = vec![mk("C"), mk("A")];
    app.resync_row();
    assert_eq!(app.row, 1);
    assert_eq!(app.sel_symbol.as_deref(), Some("A"));
}

#[test]
fn first_run_opens_the_filters_and_starts_after_saving() {
    let (mut app, fetch_rx) = test_app(Config::default());
    app.first_run = true;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // Start
    assert!(app.view == View::SetupConfig);
    assert_eq!(app.focus, field_index(FieldId::MinChange));
    assert!(!app.running);
    // one list fetch for the preview, and it fills the preview
    assert!(fetch_rx.try_recv().is_ok());
    let row = Ticker { symbol: "A".into(), price: 1.0, ..Default::default() };
    let fetched = Fetched { tickers: vec![row], group: String::new(), session: Session::now() };
    app.on_msg(Msg::Fetched(Ok(fetched)));
    assert_eq!(app.snapshot.len(), 1);
    // Esc backs out to the boot menu; Start comes back here
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.view == View::Boot);
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.view == View::SetupConfig);
    // saving starts the screener
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.first_run);
    assert!(app.view == View::Dashboard);
    assert!(app.running);
}

#[test]
fn debug_overlay_only_in_trace_mode_and_takes_the_keys_while_open() {
    let (mut app, _rx) = test_app(Config::default());
    let key = |app: &mut App, c| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    app.view = View::Dashboard;
    // tracing off: F12 and the backtick do nothing
    key(&mut app, KeyCode::F(12));
    key(&mut app, KeyCode::Char('`'));
    assert!(!app.debug.open);
    app.debug.on = true;
    key(&mut app, KeyCode::F(12));
    assert!(app.debug.open);
    // the arrows scroll the log instead of moving the row; w does not change the layout
    let layout = app.cfg.layout;
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('w'));
    assert_eq!((app.debug.scroll, app.cfg.layout), (1, layout));
    key(&mut app, KeyCode::Esc);
    assert!(!app.debug.open);
    key(&mut app, KeyCode::Char('`'));
    assert!(app.debug.open && app.debug.scroll == 0);
    key(&mut app, KeyCode::Char('`'));
    assert!(!app.debug.open);
    // only on the dashboard
    app.view = View::Chart;
    key(&mut app, KeyCode::F(12));
    assert!(!app.debug.open);
}
