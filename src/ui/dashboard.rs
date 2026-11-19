//! The dashboard (header, list/alerts/chart in the chosen layout, footer, tape), the full-screen
//! chart and the Columns overlay.

use super::tables::{alerts_table, gainers_table, gainers_width};
use super::volume::VOL_H;
use super::{centered, hint_bar, keyhint, kind_style, panel, title};
use crate::app::{App, COLUMNS};
use crate::config::Layout as Dash;
use crate::engine::Candle;
use crate::fmt;
use crate::history::Tf;
use crate::theme::pal;
use crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// The dashboard in the chosen layout (header tabs or `w`). The tape and the key hints can be
/// switched off in Configuration; their rows then go to the main area.
pub(super) fn dashboard(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let show_tape = app.cfg.ticker_tape && area.height >= 16;
    let [header, main, footer, tape] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(u16::from(app.cfg.key_hints)),
        Constraint::Length(u16::from(show_tape)),
    ])
    .areas(area);

    header_bar(f, app, header);
    match app.cfg.layout {
        Dash::Full => {
            // the list keeps the width its columns need; the chart takes the rest, if 44 fit
            let list_w = gainers_width(app);
            if main.width >= list_w + 44 {
                let [left, right] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(44)]).areas(main);
                alerts_and_list(f, app, left);
                chart_column(f, app, right);
            } else {
                alerts_and_list(f, app, main);
            }
        }
        Dash::List => alerts_and_list(f, app, main),
        Dash::Chart => chart_column(f, app, main),
        Dash::Alerts => alerts_table(f, app, main),
        Dash::Bubbles => super::bubbles::bubbles_and_volume(f, app, main),
        l => {
            if let Some(c) = app.cfg.custom(l).cloned() {
                super::custom::custom(f, app, main, &c);
            }
        }
    }
    if app.cfg.key_hints {
        footer_bar(f, app, footer);
    }
    if show_tape {
        ticker_tape(f, app, tape);
    }
}

/// Alerts panel (`alert_rows` rows plus header and borders) over the gainers list; the panel
/// is shown only while the list keeps at least 5 rows. The rows the list does not need go to
/// the volume chart, and to the bubbles over it when there are enough.
fn alerts_and_list(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.cfg.alert_rows;
    let alerts_h = if area.height >= rows + 3 + 8 { rows + 3 } else { 0 };
    let [alerts, rest] = Layout::vertical([Constraint::Length(alerts_h), Constraint::Min(0)]).areas(area);
    if alerts_h > 0 {
        alerts_table(f, app, alerts);
    }
    // the list's rows, header and borders; the rest to the volume chart, then the bubbles
    let list_h = app.tickers.len() as u16 + 3;
    let spare = rest.height.saturating_sub(list_h);
    if spare < VOL_H {
        gainers_table(f, app, rest);
        return;
    }
    let [gainers, below] = Layout::vertical([Constraint::Min(0), Constraint::Length(spare)]).areas(rest);
    gainers_table(f, app, gainers);
    super::bubbles::bubbles_and_volume(f, app, below);
}

/// The chart of the charted row, with the blacklisted tickers under it.
fn chart_column(f: &mut Frame, app: &mut App, area: Rect) {
    let bl_h = if app.blacklisted.is_empty() { 0 } else { (app.blacklisted.len() as u16 + 2).min(area.height / 4) };
    let [chart, bl] = Layout::vertical([Constraint::Min(0), Constraint::Length(bl_h)]).areas(area);
    let sym = app.charted_symbol().unwrap_or("").to_string();
    chart_panel(f, app, chart, &sym);
    if bl_h > 0 {
        blacklist_box(f, app, bl);
    }
}

pub(super) fn chart_view(f: &mut Frame, app: &mut App) {
    let [body, footer] = Layout::vertical([Constraint::Min(10), Constraint::Length(1)]).areas(f.area());
    let sym = app.chart_symbol.clone();
    chart_panel(f, app, body, &sym);
    hint_bar(
        f,
        app,
        footer,
        &[
            ("↑↓", "prev/next ticker", None),
            ("1-5/t", "timeframe", Some(KeyCode::Char('t'))),
            ("r", "refresh", Some(KeyCode::Char('r'))),
            ("esc", "back", Some(KeyCode::Esc)),
            ("q", "quit", Some(KeyCode::Char('q'))),
        ],
    );
}

/// Panel title `name` followed by the timeframe tabs (clickable) of the panel at `area`.
pub(super) fn tf_title(app: &mut App, area: Rect, name: &'static str) -> Line<'static> {
    let mut ttl = vec![Span::styled(name, Style::new().fg(pal().accent).bold()), Span::raw(" ")];
    let mut x = area.x + 1 + name.len() as u16 + 1;
    for t in Tf::ALL {
        let label = format!(" {} ", t.label());
        let w = label.len() as u16;
        let st = if t == app.timeframe { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        app.hits.tf_tabs.push((Rect::new(x, area.y, w, 1).intersection(area), t));
        ttl.push(Span::styled(label, st));
        ttl.push(Span::raw(" "));
        x += w + 1;
    }
    Line::from(ttl)
}

/// Bordered chart of `sym` in the current timeframe, with clickable timeframe tabs in the
/// title. 1m comes from the candles collected today (plus backfill); the others from Webull.
pub(super) fn chart_panel(f: &mut Frame, app: &mut App, area: Rect, sym: &str) {
    let tf = app.timeframe;
    let block = panel(tf_title(app, area, " CHART "));
    let inner = block.inner(area).inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(block, area);
    if sym.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No ticker selected", Style::new().fg(pal().faint))), inner);
        return;
    }
    let ticker = app.tickers.iter().chain(&app.blacklisted).find(|t| t.symbol == sym);
    let mut candles: Vec<Candle> = match tf {
        Tf::M1 => app.engine.candles(sym).map(|c| c.iter().copied().collect()).unwrap_or_default(),
        _ => app.bars.get(&(sym.to_string(), tf)).cloned().unwrap_or_default(),
    };
    // Fetched bars are refreshed every few seconds; the live price and the 1m candles keep the
    // bar in progress current (and stand in for it until Webull has one).
    if tf != Tf::M1 && !candles.is_empty() {
        let minutes = app.engine.candles(sym).into_iter().flatten().copied();
        crate::history::extend_live(&mut candles, tf, minutes, ticker.map_or(0.0, |t| t.price), chrono::Utc::now().timestamp());
    }
    let empty = match tf {
        Tf::M1 => "Collecting data. Candles appear after the first refresh in each minute.".to_string(),
        _ if app.bars_loading(sym) => format!("Loading {} bars...", tf.label()),
        _ => format!("No {} bars yet.", tf.label()),
    };
    let day_axis = matches!(tf, Tf::H4 | Tf::D1);
    let pct = ticker.map_or(0.0, |t| t.change_ratio * 100.0);
    // 4h swing levels on every timeframe up to 4h (as the Pine script: not on the daily)
    let levels = match (tf, app.levels.get(sym)) {
        (Tf::D1, _) | (_, None) => &[][..],
        (_, Some(d)) => d.lines.as_slice(),
    };
    let now = chrono::Utc::now().timestamp();
    let note = match (app.levels.get(sym), crate::limiter::paused()) {
        (_, Some(p)) => format!("Webull paused after HTTP 403: {} min left", p.as_secs().div_ceil(60)),
        (Some(d), None) if !d.complete(now) => format!("4h levels: {}% of 1 year loaded", d.percent(now)),
        (Some(d), None) if d.lines.is_empty() => "4h levels: none".to_string(),
        (None, None) => "4h levels: queued".to_string(),
        _ => String::new(),
    };
    let spec = crate::chart::Spec {
        tf,
        day_axis,
        empty: &empty,
        pan: app.pan,
        zoom: app.zoom,
        shift: app.shift,
        levels,
        note: &note,
        scale: app.scale,
        pixel: app.pixel_chart,
        now,
    };
    app.plot = crate::chart::draw(f.buffer_mut(), inner, sym, &candles, pct, &spec);

    // Mouse targets: drag the plot to move the chart, drag the price axis to scale.
    let plot_w = crate::chart::plot_cols(inner.width);
    let body = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));
    app.hits.chart_plot = Rect::new(body.x, body.y, plot_w, body.height);
    app.hits.chart_axis = Rect::new(body.x + plot_w, body.y, inner.width.saturating_sub(plot_w), body.height);
    app.hits.chart_cols = inner.width;
    app.hits.chart_fit = crate::chart::fit(inner.width, app.scale);
}

/// Layout tabs, the phase of the trading day with its countdown, and the status items. When
/// narrow, the status items are dropped from the end, then the phase name, then the tabs: at
/// the narrowest only the countdown and the status dot stay.
fn header_bar(f: &mut Frame, app: &mut App, area: Rect) {
    // phase of the trading day and the countdown to its end
    let (phase, until, secs) = crate::session::Session::phase(chrono::Utc::now());
    let clock = Span::styled(fmt::hms(secs), Style::new().fg(pal().text).bold());
    let full = Line::from(vec![
        Span::styled(format!(" {phase}"), Style::new().fg(pal().accent).bold()),
        Span::styled(format!("   {until} "), Style::new().fg(pal().dim)),
        clock.clone(),
    ]);
    let short = Line::from(vec![Span::raw(" "), clock]);
    let layouts = app.cfg.tabs();
    let tabs: Vec<Span> = layouts
        .iter()
        .map(|&l| {
            let st =
                if l == app.cfg.layout { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
            Span::styled(format!(" {} ", app.cfg.layout_label(l)), st)
        })
        .collect();
    // one space before the strip and after each tab
    let tabs_w = 1 + tabs.iter().map(|s| s.width() as u16 + 1).sum::<u16>();
    // the status dot and a space before it
    let dot_w = 3;
    let show_tabs = area.width >= tabs_w + short.width() as u16 + dot_w;
    if show_tabs {
        let mut x = area.x + 1;
        for (s, &l) in tabs.iter().zip(&layouts) {
            let w = s.width() as u16;
            app.hits.layout_tabs.push((Rect::new(x, area.y, w, 1), l));
            x += w + 1;
        }
        let strip: Vec<Span> = std::iter::once(Span::raw(" ")).chain(tabs.into_iter().flat_map(|s| [s, Span::raw(" ")])).collect();
        f.render_widget(Paragraph::new(Line::from(strip)), Rect { width: tabs_w, ..area });
    }
    let area = if show_tabs { Rect { x: area.x + tabs_w, width: area.width - tabs_w, ..area } } else { area };
    let left = if area.width as usize >= full.width() + dot_w as usize { full } else { short };

    let amber = pal().amber;
    let online = format!("online {:.0} Hz", app.stream_status().msgs_per_sec);
    let (color, status) = match (&app.error, &app.last_update) {
        // the stream runs but the gainers list failed
        (Some(_), _) if app.stream_ok() => (amber, online),
        (Some(_), _) => (pal().red, "offline".to_string()),
        (None, Some(_)) if app.stream_ok() => (pal().green, online),
        (None, Some(_)) if app.stream_enabled() => (amber, "REST".to_string()),
        (None, Some(_)) => (pal().green, "online".to_string()),
        (None, None) => (amber, "connecting".to_string()),
    };
    // status dot, stream status, uptime, ticker data on disk, RAM: when the header is too narrow
    // the items at the end are dropped whole
    let room = (area.width as usize).saturating_sub(left.width() + 1);
    let mut right: Vec<Span> = Vec::new();
    let mut items = vec![
        vec![Span::styled("● ", Style::new().fg(color))],
        vec![Span::styled(format!("{status}  "), Style::new().fg(pal().dim))],
        vec![Span::styled(format!("up {}  ", fmt::uptime(app.started_at.elapsed())), Style::new().fg(pal().faint))],
    ];
    // trace mode (README "Debugging / trace mode")
    if app.debug.on {
        items.insert(2, vec![Span::styled(" TRACE ", Style::new().fg(pal().on_accent).bg(amber).bold()), Span::raw("  ")]);
    }
    // Config -> Storage lists and deletes this data
    if let Some(bytes) = app.storage_bytes {
        let mb = (bytes as f64 / 1_048_576.0).round();
        items.push(vec![Span::styled(format!("storage {mb:.0} MB  "), Style::new().fg(pal().dim))]);
    }
    if let Some((used, total)) = crate::sysmem::ram() {
        let pct = used as f64 / total as f64 * 100.0;
        let col = if pct >= 90.0 {
            pal().red
        } else if pct >= 75.0 {
            pal().amber
        } else {
            pal().dim
        };
        items.push(vec![Span::styled(format!("RAM {pct:.0}% "), Style::new().fg(col))]);
    }
    for item in items {
        let w: usize = item.iter().map(|s| s.width()).sum();
        if right.iter().map(|s| s.width()).sum::<usize>() + w > room {
            break;
        }
        right.extend(item);
    }
    let right = Line::from(right);
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

/// Session countdown as `HH:MM:SS`.

fn ticker_tape(f: &mut Frame, app: &App, area: Rect) {
    let mut cells: Vec<(char, Style)> = Vec::new();
    let push = |cells: &mut Vec<(char, Style)>, s: &str, st: Style| cells.extend(s.chars().map(|c| (c, st)));
    for t in app.tickers.iter().take(30) {
        let (arrow, color) = if t.change_ratio < 0.0 { ("▼", pal().red) } else { ("▲", pal().green) };
        push(&mut cells, "   ", Style::new());
        match t.status {
            Some(k) => push(&mut cells, &format!(" {} ", t.symbol), kind_style(k)),
            None => push(&mut cells, &t.symbol, Style::new().fg(pal().text).bold()),
        }
        push(&mut cells, &format!(" {arrow}{} ({:+.2}%)", fmt::price(t.price), t.change_ratio * 100.0), Style::new().fg(color));
    }
    if cells.is_empty() {
        push(&mut cells, "   Waiting for data...", Style::new().fg(pal().faint));
    }
    let n = cells.len();
    let offset = (app.started_at.elapsed().as_millis() / 110) as usize % n;
    let spans: Vec<Span> = (0..area.width as usize)
        .map(|i| {
            let (c, st) = cells[(offset + i) % n];
            Span::styled(c.to_string(), st)
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(super) fn blacklist_box(f: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .blacklisted
        .iter()
        .map(|t| {
            Line::styled(
                format!("{:<6} {:>8} {:>+8.1}%  {:>8}", t.symbol, fmt::price(t.price), t.change_ratio * 100.0, fmt::count(t.volume as f64)),
                Style::new().fg(pal().faint),
            )
        })
        .collect();
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal().faint))
        .title(Span::styled(" Blacklisted ", Style::new().fg(pal().faint)));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn footer_bar(f: &mut Frame, app: &mut App, area: Rect) {
    // trace mode: the debug overlay's key comes first, so a narrow footer still shows it
    let debug = app.debug.on.then_some(("F12", "debug", Some(KeyCode::F(12))));
    let hints: Vec<_> = debug
        .into_iter()
        .chain([
            ("enter", "chart", Some(KeyCode::Enter)),
            ("w", "layout", Some(KeyCode::Char('w'))),
            ("←→", "column", None),
            ("s", "sort", Some(KeyCode::Char('s'))),
            ("1-5/t", "timeframe", Some(KeyCode::Char('t'))),
            ("f", "alerts", Some(KeyCode::Char('f'))),
            ("v", "columns", Some(KeyCode::Char('v'))),
            ("r", "refresh", Some(KeyCode::Char('r'))),
            ("o", "config", Some(KeyCode::Char('o'))),
            ("q", "quit", Some(KeyCode::Char('q'))),
        ])
        .collect();
    hint_bar(f, app, area, &hints);
}

pub(super) fn columns(f: &mut Frame, app: &App) {
    let rect = centered(f.area(), 56, COLUMNS.len() as u16 + 6);
    let mut lines = vec![Line::raw("")];
    for (i, c) in COLUMNS.iter().enumerate() {
        let on = app.cfg.column_visible(c.key);
        let badge = if on {
            Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().green))
        } else {
            Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint))
        };
        let st = if i == app.col_cursor { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        let prio = ["", "core", "important", "standard"][c.priority as usize];
        lines.push(Line::from(vec![
            Span::raw(if i == app.col_cursor { " ▶ " } else { "   " }),
            badge,
            Span::styled(format!("  {:<9}", c.title), st),
            Span::styled(format!("  {prio}"), Style::new().fg(pal().dim).italic()),
        ]));
    }
    lines.push(Line::raw(""));
    let mut hint = keyhint("enter", "toggle");
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(panel(title("COLUMNS"))), rect);
}
