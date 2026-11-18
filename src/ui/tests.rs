use super::boot::version_line;
use super::draw;
use super::setup::{FILTERS_W, SIDEBAR_W};
use super::tables::{count_or_na, fit_columns, gainers_width};
use crate::app::{App, Hits, View, COLUMNS, SETUP_ITEMS, SETUP_VIEWS};
use crate::config::{Config, Layout as Dash};
use crate::fmt;
use crate::scraper::Ticker;
use crossterm::event::KeyCode;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::time::Duration;

#[test]
fn format_pane_previews_the_draft_and_the_dashboard_keeps_the_saved_formats() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.setup_idx = SETUP_VIEWS.iter().position(|&v| v == View::SetupFormat).unwrap();
    app.view = View::SetupFormat;
    app.fmt_draft = crate::fmt::Formats { date: "YYYY/MM/DD".into(), decimal: ',', ..Default::default() };
    let mut term = Terminal::new(TestBackend::new(160, 40)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    let today = fmt::date(chrono::Utc::now().timestamp());
    assert!(today.len() == 10 && today.contains('/') && text.contains(&today), "{today}");
    assert!(text.contains("1234,50") && text.contains("18,40M"));
    // the dashboard draws with the saved formats
    app.view = View::Dashboard;
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert_eq!(fmt::price(1.5), "1.50");
}

#[test]
fn float_columns_are_off_by_default_and_fit_when_switched_on() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    let key = |app: &App| app.visible_cols.iter().map(|&i| COLUMNS[i].key).collect::<Vec<_>>();
    fit_columns(&mut app, 200);
    assert!(!key(&app).contains(&"float") && !key(&app).contains(&"shares"));
    app.cfg.column_visibility.insert("float".into(), true);
    app.cfg.column_visibility.insert("shares".into(), true);
    let full = gainers_width(&app) - 2;
    fit_columns(&mut app, full);
    assert!(key(&app).contains(&"float") && key(&app).contains(&"shares"));
    // one cell short: the priority 3 columns go first
    fit_columns(&mut app, full - 1);
    assert!(!key(&app).contains(&"float") && key(&app).contains(&"price"));
    // the largest compact counts and N/A fit the column width
    let w = COLUMNS.iter().find(|c| c.key == "float").unwrap().width as usize;
    for v in [0.0, 999_990.0, 14_272_214_729.0, 999_000_000_000.0] {
        assert!(count_or_na(v).chars().count() <= w, "{}", count_or_na(v));
    }
}

#[test]
fn config_shell_draws_every_item_at_any_size() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.presets = vec![crate::presets::BUILTIN.to_string()];
    app.preset_inputs = None;
    for (w, h) in [(200, 50), (120, 40), (60, 15), (20, 5)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        for (i, &v) in SETUP_VIEWS.iter().enumerate() {
            for view in [View::SetupMenu, v] {
                app.setup_idx = i;
                app.view = view;
                term.draw(|f| draw(f, &mut app)).unwrap();
                assert_eq!(app.hits.setup_items.first().map(|r| r.1), (h > 3).then_some(0));
            }
        }
    }
    // the sidebar shows every item, with the gear under them on a tall screen
    let mut term = Terminal::new(TestBackend::new(200, 50)).unwrap();
    app.view = View::SetupMenu;
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert_eq!(app.hits.setup_items.len(), SETUP_ITEMS.len());
    let buf = term.backend().buffer();
    let side: String = (0..50).flat_map(|y| (0..SIDEBAR_W).map(move |x| (x, y))).map(|(x, y)| buf[(x, y)].symbol().to_string()).collect();
    assert!(side.contains("Presets") && side.contains('●'));
    let screen = |term: &Terminal<TestBackend>| -> String {
        let buf = term.backend().buffer();
        (0..50).flat_map(|y| (0..200).map(move |x| (x, y))).map(|(x, y)| buf[(x, y)].symbol().to_string()).collect()
    };
    // Theme: list, sample gainers rows with badges and a sample chart while there is no data
    app.setup_idx = SETUP_VIEWS.iter().position(|&v| v == View::SetupTheme).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let text = screen(&term);
    assert!(text.contains("Dark Noir") && text.contains("Preview: Dark Noir") && text.contains("SETUP") && text.contains("SMPL"));
    // Presets: the save row is listed last
    app.setup_idx = 1;
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert!(screen(&term).contains("Save current config as"));
    // the window app gets the cog as dots instead, over blank cells
    app.pixel_chart = true;
    term.draw(|f| draw(f, &mut app)).unwrap();
    let (r, ht) = &app.halftones[0];
    assert_eq!((ht.w, ht.h), (r.width as usize, r.height as usize * 2));
    assert_eq!(term.backend().buffer()[(r.x + r.width / 2, r.y + r.height / 2)].symbol(), " ");
}

/// Every mouse target of the last draw that has a size.
fn hit_rects(h: &Hits) -> Vec<Rect> {
    let mut v = vec![h.gainer_rows, h.alert_rows, h.chart_plot, h.chart_axis];
    v.extend(h.gainer_header.iter().map(|r| r.0));
    v.extend(h.tf_tabs.iter().map(|r| r.0));
    v.extend(h.keys.iter().map(|r| r.0));
    v.extend(h.layout_tabs.iter().map(|r| r.0));
    v.retain(|r| !r.is_empty());
    v
}

#[test]
fn every_layout_draws_at_any_size_with_targets_inside_and_apart() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.view = View::Dashboard;
    // a list, then 20 more symbols: 20 NEW alerts
    let mk = |i: u64| Ticker {
        symbol: format!("S{i}"),
        ticker_id: i,
        price: 2.0 + i as f64,
        volume: 1000 * i as i64,
        change_ratio: 0.1,
        market_value: 1e6,
        ..Default::default()
    };
    let t0 = std::time::Instant::now();
    app.engine.process((1..=10).map(mk).collect(), "change", true, t0);
    let out = app.engine.process((1..=30).map(mk).collect(), "change", true, t0 + Duration::from_secs(10));
    app.tickers = out.active;
    assert_eq!(app.engine.alerts().len(), 20);
    for layout in Dash::ALL {
        for (tape, hints) in [(true, true), (false, false)] {
            (app.cfg.layout, app.cfg.ticker_tape, app.cfg.key_hints) = (layout, tape, hints);
            for w in [40, 60, 80, 120, 200] {
                for h in 10..=50 {
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    term.draw(|f| draw(f, &mut app)).unwrap();
                    let area = Rect::new(0, 0, w, h);
                    let rects = hit_rects(&app.hits);
                    for (i, a) in rects.iter().enumerate() {
                        assert_eq!(a.intersection(area), *a, "{a:?} outside {w}x{h} in {layout:?}");
                        for b in &rects[i + 1..] {
                            assert!(!a.intersects(*b), "{a:?} overlaps {b:?} at {w}x{h} in {layout:?}");
                        }
                    }
                    // footer hints on the last row but the tape's, the tape on the last
                    let tape_on = tape && h >= 16;
                    let footer_y = h - 1 - u16::from(tape_on);
                    assert_eq!(app.hits.keys.first().map(|k| k.0.y), hints.then_some(footer_y));
                    if tape_on {
                        let buf = term.backend().buffer();
                        assert!((0..w).any(|x| buf[(x, h - 1)].symbol() != " "));
                    }
                    // the chart only in Full (when wide enough) and Chart; alerts not in Chart
                    assert_eq!(app.hits.chart_cols > 0, layout == Dash::Chart || (layout == Dash::Full && w >= 120));
                    if layout == Dash::Alerts {
                        assert_eq!(app.hits.alert_rows.height, (h - 4 - u16::from(hints) - u16::from(tape_on)).min(20));
                    }
                }
            }
        }
    }
}

#[test]
fn alerts_layout_scrolls_and_tabs_switch_layout() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "target/test-config/unused-layout-config.json".into(), tx, htx, None);
    app.view = View::Dashboard;
    let mk = |i: u64| Ticker { symbol: format!("S{i}"), ticker_id: i, price: 2.0, market_value: 1e6, ..Default::default() };
    let t0 = std::time::Instant::now();
    app.engine.process((1..=2).map(mk).collect(), "change", true, t0);
    app.engine.process((1..=30).map(mk).collect(), "change", true, t0 + Duration::from_secs(10));
    let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    // the header tabs: click "Alerts"
    let (r, _) = *app.hits.layout_tabs.iter().find(|t| t.1 == Dash::Alerts).unwrap();
    let click = |app: &mut App, x, y| {
        app.on_mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: x,
            row: y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
    };
    click(&mut app, r.x, r.y);
    assert_eq!(app.cfg.layout, Dash::Alerts);
    term.draw(|f| draw(f, &mut app)).unwrap();
    let first = app.hits.alert_syms[0].clone();
    let key = |app: &mut App, c| app.on_key(crossterm::event::KeyEvent::new(c, crossterm::event::KeyModifiers::NONE));
    key(&mut app, KeyCode::Down);
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert_eq!(app.alert_scroll, 1);
    assert_ne!(app.hits.alert_syms[0], first);
    // scrolled no further than the last page
    key(&mut app, KeyCode::PageDown);
    key(&mut app, KeyCode::PageDown);
    key(&mut app, KeyCode::PageDown);
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert_eq!(app.alert_scroll + app.hits.alert_syms.len(), 28);
    // chart keys do nothing without a chart; w cycles to Full
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.timeframe, crate::history::Tf::M1);
    key(&mut app, KeyCode::Char('w'));
    assert_eq!((app.cfg.layout, app.alert_scroll), (Dash::Full, 0));
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.timeframe, crate::history::Tf::M5);
}

fn screen_text(term: &Terminal<TestBackend>) -> Vec<String> {
    let buf = term.backend().buffer();
    (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect()
}

#[test]
fn health_pane_aligns_connections_and_system_groups() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.health = vec![
        crate::health::CheckResult { name: "Network", ok: true, message: "OK (12ms)".into() },
        crate::health::CheckResult { name: "Stock Data Source", ok: false, message: "HTTP 500".into() },
    ];
    app.setup_idx = SETUP_VIEWS.iter().position(|&v| v == View::Health).unwrap();
    app.view = View::Health;
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let lines = screen_text(&term);
    let col = |s: &str| lines.iter().find_map(|l| l.find(s).map(|i| l[..i].chars().count())).unwrap();
    assert!(lines.iter().any(|l| l.contains("SYSTEM")) && lines.iter().any(|l| l.contains("Webull requests")));
    // every message starts in one column, after the longest name
    assert_eq!(col("OK (12ms)"), col("HTTP 500"));
    assert_eq!(col("OK (12ms)"), col("per minute") - 2);
    // a blank line separates the groups
    let sys = lines.iter().position(|l| l.contains("SYSTEM")).unwrap();
    assert!(lines[sys - 1].trim_matches(|c| c == ' ' || c == '│').is_empty());
}

#[test]
fn alert_filters_preview_shows_rows_of_the_enabled_kinds() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut cfg = Config { alert_rows: 7, ..Config::default() };
    cfg.filter_enabled.insert("NEW".into(), false);
    let mut app = App::new(cfg, "unused-test-config.json".into(), tx, htx, None);
    app.setup_idx = SETUP_VIEWS.iter().position(|&v| v == View::SetupFilters).unwrap();
    app.view = View::SetupFilters;
    let mut term = Terminal::new(TestBackend::new(200, 40)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let lines = screen_text(&term);
    let preview: Vec<&str> = lines.iter().map(|l| &l[l.char_indices().nth(SIDEBAR_W as usize + FILTERS_W as usize).unwrap().0..]).collect();
    let rows = preview
        .iter()
        .filter(|l| l.contains("SMPL") || l.contains("DEMO") || l.contains("TEST") || l.contains("ABCD") || l.contains("WXYZ"))
        .count();
    assert_eq!(rows, 7);
    assert!(!preview.iter().any(|l| l.contains(" NEW ")) && preview.iter().any(|l| l.contains("SETUP")));
    assert!(lines.iter().any(|l| l.contains("Rows on dashboard") && l.contains(" 7 ")));
}

#[test]
fn boot_boxes_fit_their_content() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.health = vec![
        crate::health::CheckResult { name: "Network", ok: true, message: "OK (12ms)".into() },
        crate::health::CheckResult { name: "Stock Data Source", ok: true, message: "OK (80ms)".into() },
    ];
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let lines = screen_text(&term);
    let col = |s: &str| lines.iter().find_map(|l| l.find(s).map(|i| l[..i].chars().count())).unwrap();
    assert_eq!(col("OK (12ms)"), col("OK (80ms)"));
    // the health box: border, " ● ", 17-char name, 2 spaces, 9-char message, 1 space, border
    let row = lines.iter().find(|l| l.contains("OK (12ms)")).unwrap();
    let (l, r) = (row.find('│').unwrap(), row.rfind('│').unwrap());
    assert_eq!(row[l..r].chars().count() + 1, 2 + 3 + 17 + 2 + 9 + 1);
}

#[test]
fn boot_shows_name_and_version() {
    assert_eq!(version_line(), format!("MarketGUI v{} (Taj Mahal)", env!("CARGO_PKG_VERSION")));
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    let mut term = Terminal::new(TestBackend::new(100, 40)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer();
    let text: String = (0..40).flat_map(|y| (0..100).map(move |x| (x, y))).map(|(x, y)| buf[(x, y)].symbol().to_string()).collect();
    assert!(text.contains(&version_line()) && text.contains("Start MarketGUI"));
    // the scene is drawn as dot glyphs on the night background
    assert!(text.contains('●') || text.contains('•'));
}

#[test]
fn debug_overlay_draws_at_any_size_and_shows_flows_and_the_newest_lines() {
    use crate::trace::Log;
    use std::time::Instant;
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.view = View::Dashboard;
    let mut log = Log::new(None);
    let t0 = Instant::now();
    for i in 0..40 {
        log.record("rest.gainers", format!("{i} rows in 120 ms"), true, t0, chrono::Local::now());
    }
    log.record("stream.batch", "7 quotes".into(), false, t0, chrono::Local::now());
    let snap = log.snapshot(t0 + Duration::from_secs(3));
    let text = |term: &Terminal<TestBackend>| term.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>();
    for (w, h) in [(200, 60), (120, 40), (60, 15), (20, 5), (1, 1)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        for scroll in [0, 5, 1000] {
            app.debug.scroll = scroll;
            term.draw(|f| super::debug::overlay(f, &app, &snap)).unwrap();
        }
    }
    // 21 log lines fit
    let mut term = Terminal::new(TestBackend::new(160, 30)).unwrap();
    app.debug.scroll = 0;
    term.draw(|f| super::debug::overlay(f, &app, &snap)).unwrap();
    let t = text(&term);
    assert!(t.contains("stream.batch") && t.contains("7 quotes") && t.contains("3.0s"), "flow table");
    assert!(t.contains("39 rows in 120 ms") && t.contains("of 40"), "newest line");
    assert!(!t.contains(" 0 rows in"), "oldest line scrolled off");
    // scrolled to the top: the oldest line shows
    app.debug.scroll = 1000;
    term.draw(|f| super::debug::overlay(f, &app, &snap)).unwrap();
    assert!(text(&term).contains(" 0 rows in"));
    assert_eq!(super::debug::age(Duration::from_millis(400)), "0.4s");
    assert_eq!(super::debug::age(Duration::from_secs(150)), "2m");
}

#[test]
fn trace_mode_shows_in_the_header_and_footer_and_the_overlay_hides_the_chart() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    let mut app = App::new(Config::default(), "unused-test-config.json".into(), tx, htx, None);
    app.view = View::Dashboard;
    let mut term = Terminal::new(TestBackend::new(200, 50)).unwrap();
    let text = |term: &Terminal<TestBackend>| term.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>();
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert!(!text(&term).contains("TRACE") && !text(&term).contains("F12"));
    app.debug.on = true;
    term.draw(|f| draw(f, &mut app)).unwrap();
    assert!(text(&term).contains(" TRACE ") && text(&term).contains(" F12 "));
    assert!(app.hits.keys.iter().any(|&(_, k)| k == KeyCode::F(12)));
    app.debug.open = true;
    for (w, h) in [(200, 50), (80, 24), (20, 5)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        assert!(app.plot.is_none() && app.hits.keys.is_empty());
    }
}
