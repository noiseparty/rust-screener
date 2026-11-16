use crate::app::{App, View, BL_COLS, BOOT_ITEMS, COLUMNS, FIELDS, SETUP_ITEMS};
use crate::config::FILTER_NAMES;
use crate::engine::{AlertKind, RATE_WINDOWS};
use crate::scraper::{Mov, Ticker};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use std::time::{Duration, Instant};

// Palette (highlowtick.com tokens)
const GREEN: Color = Color::Rgb(34, 197, 94);
const RED: Color = Color::Rgb(239, 68, 68);
const CYAN: Color = Color::Rgb(34, 211, 238);
const BORDER: Color = Color::Rgb(22, 120, 140);
const TEXT: Color = Color::Rgb(238, 242, 239);
const DIM: Color = Color::Rgb(159, 179, 166);
const FAINT: Color = Color::Rgb(70, 84, 76);
const ZEBRA: Color = Color::Rgb(14, 22, 17);
const SELECT: Color = Color::Rgb(30, 58, 138);

fn kind_style(kind: AlertKind) -> Style {
    match kind {
        AlertKind::New => Style::new().fg(Color::Black).bg(Color::Rgb(234, 179, 8)),
        AlertKind::Pump => Style::new().fg(Color::Black).bg(Color::Rgb(249, 115, 22)),
        AlertKind::Spike => Style::new().fg(Color::Black).bg(Color::Rgb(244, 114, 182)),
        AlertKind::Setup => Style::new().fg(Color::White).bg(Color::Rgb(147, 51, 234)).bold(),
    }
}

fn status_kind(s: &str) -> Option<AlertKind> {
    match s {
        "NEW" => Some(AlertKind::New),
        "PUMP" => Some(AlertKind::Pump),
        "SPIKE" => Some(AlertKind::Spike),
        "SETUP" => Some(AlertKind::Setup),
        _ => None,
    }
}

fn panel(title: Line<'static>) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(BORDER))
        .title(title)
}

fn title(s: &str) -> Line<'static> {
    Line::from(Span::styled(format!(" {s} "), Style::new().fg(CYAN).bold()))
}

pub fn compact(v: f64) -> String {
    let a = v.abs();
    let (n, suf) = if a >= 1e12 {
        (v / 1e12, "T")
    } else if a >= 1e9 {
        (v / 1e9, "B")
    } else if a >= 1e6 {
        (v / 1e6, "M")
    } else if a >= 1e3 {
        (v / 1e3, "K")
    } else {
        return format!("{v:.0}");
    };
    if n.abs() >= 100.0 { format!("{n:.1}{suf}") } else { format!("{n:.2}{suf}") }
}

fn keyhint(k: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {k} "), Style::new().fg(Color::Black).bg(DIM)),
        Span::styled(format!(" {label}  "), Style::new().fg(DIM)),
    ]
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.view {
        View::Boot => boot(f, app),
        View::Health => health(f, app),
        View::Dashboard => dashboard(f, app),
        View::SetupMenu => setup_menu(f, app),
        View::SetupConfig => setup_config(f, app),
        View::SetupBlacklist => setup_blacklist(f, app),
        View::SetupFilters => setup_filters(f, app),
        View::Columns => columns(f, app),
        View::Chart => chart_view(f, app),
    }
}

// ───────────────────────────── boot / health ─────────────────────────────

fn boot(f: &mut Frame, app: &App) {
    let area = f.area();
    scene(f, area, app.started_at.elapsed().as_secs_f64());

    let menu_w = 34u16.min(area.width);
    let menu_h = (BOOT_ITEMS.len() as u16 + 5).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - menu_w) / 2,
        area.y + area.height.saturating_sub(menu_h + 1),
        menu_w,
        menu_h,
    );
    let mut lines = vec![Line::raw("")];
    for (i, item) in BOOT_ITEMS.iter().enumerate() {
        lines.push(if i == app.boot_idx {
            Line::from(Span::styled(format!(" ▶ {item:<28}"), Style::new().fg(Color::Black).bg(GREEN).bold()))
        } else {
            Line::from(Span::styled(format!("   {item:<28}"), Style::new().fg(TEXT)))
        });
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(" ↑/↓ move  enter select  q quit", Style::new().fg(DIM))));
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Rgb(142, 166, 200)))
        .style(Style::new().bg(Color::Rgb(10, 16, 36)))
        .title(Line::from(Span::styled(" VOLUME SCREENER ", Style::new().fg(Color::Rgb(255, 253, 242)).bold())).centered());
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// Moonlit mausoleum scaled to fit `area`, two pixels per cell via the upper half block.
fn scene(f: &mut Frame, area: Rect, t: f64) {
    use crate::art::{mausoleum, SCENE_H, SCENE_W};
    let px = mausoleum().frame(t);
    let (cw, ch) = (area.width as f64, area.height as f64 * 2.0);
    let scale = (cw / SCENE_W as f64).min(ch / SCENE_H as f64);
    let (ox, oy) = ((cw - SCENE_W as f64 * scale) / 2.0, (ch - SCENE_H as f64 * scale) / 2.0);
    let step = (1.0 / scale).ceil().max(1.0) as usize;
    let sample = |x: f64, y: f64| -> [f64; 3] {
        let (sx, sy) = ((x - ox) / scale, (y - oy) / scale);
        if sx < 0.0 || sy < 0.0 || sx >= SCENE_W as f64 || sy >= SCENE_H as f64 {
            return [3.0, 5.0, 10.0];
        }
        // box-average the source pixels this target pixel covers
        let (x0, y0) = (sx as usize, sy as usize);
        let (mut acc, mut n) = ([0.0; 3], 0.0);
        for yy in y0..(y0 + step).min(SCENE_H) {
            for xx in x0..(x0 + step).min(SCENE_W) {
                let p = px[yy * SCENE_W + xx];
                acc = [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]];
                n += 1.0;
            }
        }
        [acc[0] / n, acc[1] / n, acc[2] / n]
    };
    let rgb = |c: [f64; 3]| Color::Rgb(c[0] as u8, c[1] as u8, c[2] as u8);
    let buf = f.buffer_mut();
    for cy in 0..area.height {
        for cx in 0..area.width {
            let top = sample(cx as f64 + 0.5, cy as f64 * 2.0 + 0.5);
            let bot = sample(cx as f64 + 0.5, cy as f64 * 2.0 + 1.5);
            buf[(area.x + cx, area.y + cy)].set_symbol("▀").set_fg(rgb(top)).set_bg(rgb(bot));
        }
    }
}

/// Gear and wrench animation, centred in `area`.
fn gear(f: &mut Frame, area: Rect, t: f64) {
    use crate::art::{gear_frame, GEAR_H, GEAR_W};
    let cells = gear_frame(t);
    let ox = area.x + area.width.saturating_sub(GEAR_W as u16) / 2;
    let oy = area.y + area.height.saturating_sub(GEAR_H as u16) / 2;
    let buf = f.buffer_mut();
    for y in 0..GEAR_H.min(area.height as usize) {
        for x in 0..GEAR_W.min(area.width as usize) {
            let (ch, c) = cells[y * GEAR_W + x];
            if ch != ' ' {
                buf[(ox + x as u16, oy + y as u16)].set_char(ch).set_fg(Color::Rgb(c[0] as u8, c[1] as u8, c[2] as u8));
            }
        }
    }
}

fn chart_view(f: &mut Frame, app: &App) {
    let [body, footer] = Layout::vertical([Constraint::Min(10), Constraint::Length(1)]).areas(f.area());
    let block = panel(title("CHART"));
    let inner = block.inner(body).inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(block, body);
    let sym = app.chart_symbol.as_str();
    let candles: Vec<_> = app.engine.candles(sym).map(|c| c.iter().copied().collect()).unwrap_or_default();
    let pct = app
        .tickers
        .iter()
        .chain(&app.blacklisted)
        .find(|t| t.symbol == sym)
        .map_or(0.0, |t| t.change_ratio * 100.0);
    crate::chart::draw(f.buffer_mut(), inner, sym, &candles, pct);
    let mut spans = Vec::new();
    for (k, l) in [("↑↓", "prev/next ticker"), ("r", "refresh"), ("esc", "back"), ("q", "quit")] {
        spans.extend(keyhint(k, l));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), footer);
}

fn health(f: &mut Frame, app: &App) {
    let rect = centered(f.area(), 56, 11);
    let mut lines = vec![Line::raw("")];
    if app.health_running {
        lines.push(Line::styled("  Running checks...", Style::new().fg(DIM)));
    } else if app.health.is_empty() {
        lines.push(Line::styled("  Press Enter to run health checks", Style::new().fg(DIM)));
    } else {
        for r in &app.health {
            let (dot, color) = if r.ok { ("●", GREEN) } else { ("●", RED) };
            lines.push(Line::from(vec![
                Span::styled(format!("  {dot} "), Style::new().fg(color)),
                Span::styled(format!("{:<16}", r.name), Style::new().fg(TEXT)),
                Span::styled(r.message.clone(), Style::new().fg(color)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    let mut hint = keyhint("enter", "run checks");
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Paragraph::new(lines).block(panel(title("CONNECTION HEALTH"))), rect);
}

// ───────────────────────────── dashboard ─────────────────────────────

fn dashboard(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let show_rates = area.height >= 26;
    let show_tape = area.height >= 16;
    let [header, tape, rates, main, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(if show_tape { 1 } else { 0 }),
        Constraint::Length(if show_rates { 6 } else { 0 }),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);

    header_bar(f, app, header);
    if show_tape {
        ticker_tape(f, app, tape);
    }
    if show_rates {
        rate_bars(f, app, rates);
    }

    let show_alerts = main.width >= 100;
    let [left, right] = if show_alerts {
        let right_w = (main.width * 2 / 5).clamp(44, 64);
        Layout::horizontal([Constraint::Min(40), Constraint::Length(right_w)]).areas(main)
    } else {
        [main, Rect::default()]
    };
    gainers_table(f, app, left);
    if show_alerts {
        let bl_h = if app.blacklisted.is_empty() { 0 } else { (app.blacklisted.len() as u16 + 2).min(right.height / 3) };
        let [alerts, bl] = Layout::vertical([Constraint::Min(4), Constraint::Length(bl_h)]).areas(right);
        alerts_table(f, app, alerts);
        if bl_h > 0 {
            blacklist_box(f, app, bl);
        }
    }
    footer_bar(f, app, footer);
}

fn header_bar(f: &mut Frame, app: &App, area: Rect) {
    let mut left = vec![
        Span::styled(" VOLUME SCREENER ", Style::new().fg(Color::Black).bg(GREEN).bold()),
        Span::raw("  "),
        Span::styled(app.session.code(), Style::new().fg(CYAN).bold()),
    ];
    if !app.group.is_empty() {
        left.push(Span::styled(format!(" · {}", app.group), Style::new().fg(DIM)));
    }
    left.push(Span::styled("   alerts ", Style::new().fg(FAINT)));
    left.push(Span::styled(app.filter_name(), Style::new().fg(TEXT).bold()));

    let amber = Color::Rgb(245, 158, 11);
    let st = app.stream_status();
    let wss = format!("WSS {} subs {:.0} msg/s", st.subs, st.msgs_per_sec);
    let (dot, color, status) = match (&app.error, &app.last_update) {
        (Some(e), _) if app.stream_ok() => ("●", amber, format!("{wss}, list: {e}")),
        (Some(e), _) => ("●", RED, format!("error: {e}")),
        (None, Some(_)) if app.stream_ok() => ("●", GREEN, wss),
        (None, Some(_)) if app.stream_enabled() => ("●", amber, "REST, stream down".to_string()),
        (None, Some(t)) => ("●", GREEN, format!("live  {t}")),
        (None, None) => ("●", amber, "connecting".to_string()),
    };
    let right = Line::from(vec![
        Span::styled(format!("{dot} "), Style::new().fg(color)),
        Span::styled(format!("{status} "), Style::new().fg(DIM)),
    ]);
    let rw = (right.width() as u16).min(area.width / 2);
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

fn ticker_tape(f: &mut Frame, app: &App, area: Rect) {
    let mut cells: Vec<(char, Style)> = Vec::new();
    let push = |cells: &mut Vec<(char, Style)>, s: &str, st: Style| cells.extend(s.chars().map(|c| (c, st)));
    for t in app.tickers.iter().take(30) {
        let (arrow, color) = if t.change_ratio < 0.0 { ("▼", RED) } else { ("▲", GREEN) };
        push(&mut cells, "   ", Style::new());
        match status_kind(&t.status) {
            Some(k) => push(&mut cells, &format!(" {} ", t.symbol), kind_style(k)),
            None => push(&mut cells, &t.symbol, Style::new().fg(TEXT).bold()),
        }
        push(&mut cells, &format!(" {arrow}{:.2} ({:+.2}%)", t.price, t.change_ratio * 100.0), Style::new().fg(color));
    }
    if cells.is_empty() {
        push(&mut cells, "   Waiting for data...", Style::new().fg(FAINT));
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

fn rate_bars(f: &mut Frame, app: &App, area: Rect) {
    let rates = app.engine.tick_rates(Instant::now());
    let max = rates.iter().flat_map(|&(d, u)| [d, u]).max().unwrap_or(0).max(1);
    // count(4) sp bar sp label(3) sp bar sp count(4)
    let bar_w = (area.width.saturating_sub(16) / 2).clamp(4, 36) as usize;
    let bar = |v: u32| {
        let filled = ((v as f64 / max as f64) * bar_w as f64).round() as usize;
        (filled.min(bar_w), bar_w - filled.min(bar_w))
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:>w$}", "Downticks", w = bar_w + 5), Style::new().fg(DIM)),
        Span::styled("  ·  ", Style::new().fg(FAINT)),
        Span::styled(format!("{:<w$}", "Upticks", w = bar_w + 5), Style::new().fg(DIM)),
    ])];
    for (i, &(down, up)) in rates.iter().enumerate() {
        let (df, de) = bar(down);
        let (uf, ue) = bar(up);
        lines.push(Line::from(vec![
            Span::styled(format!("{down:>4} "), Style::new().fg(DIM)),
            Span::styled("░".repeat(de), Style::new().fg(Color::Rgb(60, 24, 24))),
            Span::styled("█".repeat(df), Style::new().fg(RED)),
            Span::styled(format!(" {:>3} ", RATE_WINDOWS[i].0), Style::new().fg(TEXT).bold()),
            Span::styled("█".repeat(uf), Style::new().fg(GREEN)),
            Span::styled("░".repeat(ue), Style::new().fg(Color::Rgb(20, 52, 32))),
            Span::styled(format!(" {up:<4}"), Style::new().fg(DIM)),
        ]));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn fit_columns(app: &mut App, width: u16) {
    let enabled: Vec<usize> = (0..COLUMNS.len()).filter(|&i| app.cfg.column_visible(COLUMNS[i].key)).collect();
    let mut prio = 3;
    let cols = loop {
        let cols: Vec<usize> = enabled.iter().copied().filter(|&i| COLUMNS[i].priority <= prio).collect();
        let need: u16 = cols.iter().map(|&i| COLUMNS[i].width + 1).sum();
        if need <= width || prio == 1 {
            break cols;
        }
        prio -= 1;
    };
    app.visible_cols = if cols.is_empty() { vec![1] } else { cols };
    app.sel_col = app.sel_col.min(app.visible_cols.len() - 1);
}

fn cell(t: &Ticker, key: &str) -> Cell<'static> {
    match key {
        "mov" => match t.mov {
            Mov::None => Cell::from(""),
            Mov::New => Cell::from(Span::styled("+", Style::new().fg(CYAN).bold())),
            Mov::Moved(0) => Cell::from(Span::styled("  ·", Style::new().fg(FAINT))),
            Mov::Moved(r) if r > 0 => Cell::from(Span::styled(format!("▲{r}"), Style::new().fg(GREEN))),
            Mov::Moved(r) => Cell::from(Span::styled(format!("▼{}", -r), Style::new().fg(RED))),
        },
        "symbol" => Cell::from(Span::styled(t.symbol.clone(), Style::new().bold())),
        "price" => Cell::from(Line::from(format!("{:.2}", t.price)).alignment(Alignment::Right)),
        "change" => {
            let pct = t.change_ratio * 100.0;
            let c = if pct >= 0.0 { GREEN } else { RED };
            Cell::from(Line::from(Span::styled(format!("{pct:+.2}%"), Style::new().fg(c))).alignment(Alignment::Right))
        }
        "volume" => Cell::from(Line::from(compact(t.volume as f64)).alignment(Alignment::Right)),
        "mcap" => Cell::from(
            Line::from(if t.market_value == 0.0 { "N/A".into() } else { compact(t.market_value) })
                .alignment(Alignment::Right),
        ),
        "volrate" => {
            let (s, c) = match t.vol_rate {
                _ if t.vol_stale => ("—".to_string(), DIM),
                None => ("…".to_string(), FAINT),
                Some(r) if r < 0.5 => ("0".to_string(), FAINT),
                Some(r) => (compact(r), if t.vol_partial { DIM } else { CYAN }),
            };
            Cell::from(Line::from(Span::styled(s, Style::new().fg(c))).alignment(Alignment::Right))
        }
        "status" => match status_kind(&t.status) {
            Some(k) => Cell::from(Span::styled(format!("{:^7}", k.label()), kind_style(k))),
            None => Cell::from(""),
        },
        _ => Cell::from(""),
    }
}

/// Column header for the Vol/m window: 30s, 1m, 3m, 90s.
fn vol_m_header(secs: u64) -> String {
    if secs >= 60 && secs % 60 == 0 { format!("Vol/m {}m", secs / 60) } else { format!("Vol/m {secs}s") }
}

fn gainers_table(f: &mut Frame, app: &mut App, area: Rect) {
    fit_columns(app, area.width.saturating_sub(4));
    let vol_m_label = vol_m_header(app.cfg.vol_m_window_seconds);

    let header = Row::new(app.visible_cols.iter().enumerate().map(|(vi, &ci)| {
        let col = &COLUMNS[ci];
        let mut label = if col.key == "volrate" { vol_m_label.clone() } else { col.title.to_string() };
        if app.sort_key == col.key {
            label.push(if app.sort_desc { '▼' } else { '▲' });
        }
        let st = if vi == app.sel_col {
            Style::new().fg(Color::Black).bg(CYAN).bold()
        } else {
            Style::new().fg(DIM).bold()
        };
        let line = Line::from(Span::styled(label, st));
        let line = if matches!(col.key, "mov" | "symbol" | "status") { line } else { line.alignment(Alignment::Right) };
        Cell::from(line)
    }))
    .height(1);

    let rows = app.tickers.iter().enumerate().map(|(i, t)| {
        let row = Row::new(app.visible_cols.iter().map(|&ci| cell(t, COLUMNS[ci].key)));
        let mut style = Style::new();
        if i % 2 == 1 {
            style = style.bg(ZEBRA);
        }
        // Filter grace: the row no longer passes a display filter and will vanish soon.
        if t.dim {
            style = style.fg(FAINT).add_modifier(Modifier::DIM);
        }
        row.style(style)
    });

    let widths: Vec<Constraint> = app.visible_cols.iter().map(|&ci| Constraint::Length(COLUMNS[ci].width)).collect();
    let count = Span::styled(format!("{} ", app.tickers.len()), Style::new().fg(DIM));
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(panel(Line::from(vec![Span::styled(" Top gainers ", Style::new().fg(CYAN).bold()), count])))
        .row_highlight_style(Style::new().bg(SELECT).fg(TEXT))
        .highlight_symbol("▌")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(if app.tickers.is_empty() { None } else { Some(app.row) });
    f.render_stateful_widget(table, area, &mut state);
    if app.tickers.is_empty() {
        let msg = if app.is_fetching() { "Fetching..." } else { "No tickers match the current filters" };
        let r = centered(area, msg.len() as u16 + 2, 1);
        f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(FAINT))), r);
    }
}

fn alerts_table(f: &mut Frame, app: &App, area: Rect) {
    let rows_fit = area.height.saturating_sub(3) as usize;
    let flash_window = Duration::from_millis(1500);
    let fresh = app.last_alert_at.is_some_and(|t| t.elapsed() < flash_window);

    let visible: Vec<_> = app.engine.alerts().iter().rev().filter(|a| app.alert_visible(a.kind)).take(rows_fit).collect();
    let rows = visible.iter().enumerate().map(|(i, a)| {
        let pct = a.change_ratio * 100.0;
        let mut row = Row::new(vec![
            Cell::from(Span::styled(a.clock.clone(), Style::new().fg(DIM))),
            Cell::from(Span::styled(a.symbol.clone(), Style::new().fg(TEXT).bold())),
            Cell::from(Span::styled(format!("{:^7}", a.kind.label()), kind_style(a.kind))),
            Cell::from(
                Line::from(Span::styled(format!("{pct:+.1}%"), Style::new().fg(if pct >= 0.0 { GREEN } else { RED })))
                    .alignment(Alignment::Right),
            ),
            Cell::from(Span::styled(a.detail(), Style::new().fg(DIM))),
        ]);
        if i == 0 && fresh && app.last_alert_at == Some(a.at) {
            row = row.style(Style::new().bg(Color::Rgb(20, 83, 45)));
        } else if i % 2 == 1 {
            row = row.style(Style::new().bg(ZEBRA));
        }
        row
    });

    let header = Row::new(["Time", "Symbol", "Type", "% Chg", "Detail"].map(|h| {
        let l = Line::from(Span::styled(h, Style::new().fg(DIM).bold()));
        Cell::from(if h == "% Chg" { l.alignment(Alignment::Right) } else { l })
    }));
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Min(8),
    ];
    let t = Table::new(rows, widths).header(header).column_spacing(1).block(panel(Line::from(vec![
        Span::styled(" Alerts ", Style::new().fg(CYAN).bold()),
        Span::styled(format!("{} ", app.filter_name()), Style::new().fg(DIM)),
    ])));
    f.render_widget(t, area);
    if visible.is_empty() {
        let msg = format!("No {} alerts", app.filter_name());
        f.render_widget(
            Paragraph::new(Span::styled(msg.clone(), Style::new().fg(FAINT))),
            centered(area, msg.len() as u16, 1),
        );
    }
}

fn blacklist_box(f: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .blacklisted
        .iter()
        .map(|t| {
            Line::styled(
                format!("{:<6} {:>8.2} {:>+8.1}%  {:>8}", t.symbol, t.price, t.change_ratio * 100.0, compact(t.volume as f64)),
                Style::new().fg(FAINT),
            )
        })
        .collect();
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(FAINT))
        .title(Span::styled(" Blacklisted ", Style::new().fg(FAINT)));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn footer_bar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (k, l) in [("enter", "chart"), ("←→", "column"), ("s", "sort"), ("f", "alerts"), ("v", "columns"), ("r", "refresh"), ("o", "setup"), ("q", "quit")] {
        spans.extend(keyhint(k, l));
    }
    let interval = app.poll_interval();
    let left = app.countdown().min(interval);
    let gw = 16usize;
    let filled = (((interval - left) as f64 / interval as f64) * gw as f64).round() as usize;
    let gauge = Line::from(vec![
        Span::styled(if app.is_fetching() { "fetching ".into() } else { format!("next {left:>2}s ") }, Style::new().fg(DIM)),
        Span::styled("━".repeat(filled.min(gw)), Style::new().fg(GREEN)),
        Span::styled("━".repeat(gw - filled.min(gw)), Style::new().fg(FAINT)),
    ]);
    let gw_total = gauge.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(gw_total + 1)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(spans)), l);
    f.render_widget(Paragraph::new(gauge), r);
}

// ───────────────────────────── setup ─────────────────────────────

fn setup_menu(f: &mut Frame, app: &App) {
    let area = f.area();
    let menu_area = if area.width >= 125 {
        let [g, m] = Layout::horizontal([Constraint::Length(52), Constraint::Min(70)]).areas(area);
        gear(f, g, app.started_at.elapsed().as_secs_f64());
        m
    } else {
        area
    };
    let rect = centered(menu_area, 70, SETUP_ITEMS.len() as u16 * 3 + 5);
    let mut lines = vec![Line::raw("")];
    for (i, (name, desc)) in SETUP_ITEMS.iter().enumerate() {
        let st = if i == app.setup_idx { Style::new().fg(Color::Black).bg(GREEN).bold() } else { Style::new().fg(TEXT) };
        let mark = if i == app.setup_idx { "▶" } else { " " };
        lines.push(Line::from(Span::styled(format!(" {mark} {name:<22}"), st)));
        lines.push(Line::styled(format!("     {desc}"), Style::new().fg(DIM).italic()));
        lines.push(Line::raw(""));
    }
    let mut hint = keyhint("enter", "open");
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(panel(title("SETUP"))), rect);
}

fn setup_config(f: &mut Frame, app: &App) {
    let area = f.area();
    let area = if area.width >= 150 {
        let [main, g] = Layout::horizontal([Constraint::Min(98), Constraint::Length(52)]).areas(area);
        gear(f, g, app.started_at.elapsed().as_secs_f64());
        main
    } else {
        area
    };
    let [form, preview] = Layout::horizontal([Constraint::Length(48), Constraint::Min(30)]).areas(area);

    let mut lines = vec![Line::raw("")];
    for (i, fld) in FIELDS.iter().enumerate() {
        if i == 10 {
            lines.push(Line::styled("  ─ display filters ─", Style::new().fg(FAINT)));
        }
        let focused = i == app.focus;
        let label_st = if focused { Style::new().fg(CYAN).bold() } else { Style::new().fg(DIM) };
        let mark = if focused { "▶" } else { " " };
        let mut val = app.inputs[i].clone();
        if focused {
            val.push('▏');
        }
        let val_st = if focused { Style::new().fg(TEXT).bg(Color::Rgb(24, 36, 30)) } else { Style::new().fg(TEXT) };
        lines.push(Line::from(vec![
            Span::styled(format!(" {mark} {:<20}", fld.label), label_st),
            Span::styled(format!(" {val:<20}"), val_st),
        ]));
    }
    lines.push(Line::raw(""));
    let mut text = Text::from(lines);
    if let Some(e) = &app.config_err {
        text.push_line(Line::styled(format!("  {e}"), Style::new().fg(RED).bold()));
    }
    let mut hint = keyhint("tab/↑↓", "field");
    hint.extend(keyhint("enter", "save"));
    hint.extend(keyhint("esc", "cancel"));
    text.push_line(Line::from(hint));

    let [form_top, tip] = Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).areas(form);
    f.render_widget(Paragraph::new(text).block(panel(title("CONFIGURATION"))), form_top);
    f.render_widget(
        Paragraph::new(FIELDS[app.focus].desc)
            .style(Style::new().fg(Color::Rgb(250, 230, 160)))
            .wrap(Wrap { trim: true })
            .block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(FAINT))),
        tip,
    );

    let filters = app.preview_filters();
    let passing = app.snapshot.iter().filter(|t| filters.matches(t)).count();
    let rows = app.snapshot.iter().map(|t| {
        let st = if filters.matches(t) { Style::new().fg(TEXT) } else { Style::new().fg(FAINT) };
        Row::new(vec![
            Cell::from(t.symbol.clone()),
            Cell::from(Line::from(format!("{:.2}", t.price)).alignment(Alignment::Right)),
            Cell::from(Line::from(format!("{:+.1}%", t.change_ratio * 100.0)).alignment(Alignment::Right)),
            Cell::from(Line::from(compact(t.volume as f64)).alignment(Alignment::Right)),
            Cell::from(Line::from(if t.market_value == 0.0 { "N/A".into() } else { compact(t.market_value) }).alignment(Alignment::Right)),
        ])
        .style(st)
    });
    let header = Row::new(["Symbol", "Price", "% Chg", "Volume", "Mkt Cap"].map(|h| {
        let l = Line::from(Span::styled(h, Style::new().fg(DIM).bold()));
        Cell::from(if h == "Symbol" { l } else { l.alignment(Alignment::Right) })
    }));
    let widths = [Constraint::Length(7), Constraint::Length(9), Constraint::Length(9), Constraint::Length(9), Constraint::Length(9)];
    let ttl = Line::from(vec![
        Span::styled(" Filter preview ", Style::new().fg(CYAN).bold()),
        Span::styled(format!("{passing}/{} pass ", app.snapshot.len()), Style::new().fg(DIM)),
    ]);
    f.render_widget(Table::new(rows, widths).header(header).column_spacing(1).block(panel(ttl)), preview);
    if app.snapshot.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No data yet", Style::new().fg(FAINT))), centered(preview, 11, 1));
    }
}

fn setup_blacklist(f: &mut Frame, app: &App) {
    let area = f.area();
    let mut lines = vec![
        Line::styled(" Blacklisted tickers are still tracked but listed separately, not ranked.", Style::new().fg(DIM)),
        Line::raw(""),
    ];
    if let Some(buf) = &app.bl_input {
        lines.push(Line::from(vec![
            Span::styled(" Toggle ticker: ", Style::new().fg(CYAN)),
            Span::styled(format!("{buf}▏"), Style::new().fg(TEXT).bg(Color::Rgb(24, 36, 30))),
        ]));
        if let Some(e) = &app.bl_err {
            lines.push(Line::styled(format!(" {e}"), Style::new().fg(RED)));
        }
        lines.push(Line::raw(""));
    }
    for (r, chunk) in app.snapshot.chunks(BL_COLS).enumerate() {
        let spans: Vec<Span> = chunk
            .iter()
            .enumerate()
            .map(|(c, t)| {
                let idx = r * BL_COLS + c;
                let on = app.engine.blacklist.contains(&t.symbol);
                let txt = format!(" [{}] {:<6} ", if on { "x" } else { " " }, t.symbol);
                let st = if idx == app.bl_cursor {
                    Style::new().fg(Color::Black).bg(CYAN).bold()
                } else if on {
                    Style::new().fg(FAINT)
                } else {
                    Style::new().fg(TEXT)
                };
                Span::styled(txt, st)
            })
            .collect();
        lines.push(Line::from(spans));
    }
    if app.snapshot.is_empty() {
        lines.push(Line::styled("  No tickers available. Start the screener and wait for a refresh.", Style::new().fg(FAINT)));
    }
    lines.push(Line::raw(""));
    let mut hint = keyhint("arrows", "move");
    hint.extend(keyhint("enter", "toggle"));
    hint.extend(keyhint("a", "type symbol"));
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Paragraph::new(lines).block(panel(title("TICKER BLACKLIST"))), area);
}

fn setup_filters(f: &mut Frame, app: &App) {
    let descs = [
        "Show every enabled alert type",
        "Ticker just appeared in the gainers list",
        "Volume ratio and price change both past their SETUP limits",
    ];
    let rect = centered(f.area(), 76, 9);
    let mut lines = vec![Line::raw("")];
    for (i, name) in FILTER_NAMES.iter().enumerate() {
        let on = app.cfg.filter_on(name);
        let badge = if on { Span::styled(" ON  ", Style::new().fg(Color::Black).bg(GREEN)) } else { Span::styled(" OFF ", Style::new().fg(TEXT).bg(FAINT)) };
        let name_st = if i == app.filter_cursor { Style::new().fg(Color::Black).bg(CYAN).bold() } else { Style::new().fg(TEXT) };
        lines.push(Line::from(vec![
            Span::raw(if i == app.filter_cursor { " ▶ " } else { "   " }),
            badge,
            Span::raw(" "),
            Span::styled(format!(" {name:<6}"), name_st),
            Span::styled(format!("  {}", descs[i]), Style::new().fg(DIM).italic()),
        ]));
    }
    lines.push(Line::raw(""));
    let mut hint = keyhint("enter", "toggle");
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(panel(title("ALERT FILTERS"))), rect);
}

fn columns(f: &mut Frame, app: &App) {
    let rect = centered(f.area(), 56, COLUMNS.len() as u16 + 6);
    let mut lines = vec![Line::raw("")];
    for (i, c) in COLUMNS.iter().enumerate() {
        let on = app.cfg.column_visible(c.key);
        let badge = if on { Span::styled(" ON  ", Style::new().fg(Color::Black).bg(GREEN)) } else { Span::styled(" OFF ", Style::new().fg(TEXT).bg(FAINT)) };
        let st = if i == app.col_cursor { Style::new().fg(Color::Black).bg(CYAN).bold() } else { Style::new().fg(TEXT) };
        let prio = ["", "core", "important", "standard"][c.priority as usize];
        lines.push(Line::from(vec![
            Span::raw(if i == app.col_cursor { " ▶ " } else { "   " }),
            badge,
            Span::styled(format!("  {:<9}", c.title), st),
            Span::styled(format!("  {prio}"), Style::new().fg(DIM).italic()),
        ]));
    }
    lines.push(Line::raw(""));
    let mut hint = keyhint("enter", "toggle");
    hint.extend(keyhint("esc", "back"));
    lines.push(Line::from(hint));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(panel(title("COLUMNS"))), rect);
}


