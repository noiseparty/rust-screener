use crate::app::{App, APP_NAME, FieldId, FieldKind, Hits, PresetAsk, View, BL_COLS, BOOT_ITEMS, COLUMNS, FIELDS, SETUP_ITEMS, SETUP_VIEWS};
use crate::engine::Candle;
use crate::fmt;
use crate::history::Tf;
use crate::theme::{self, pal};
use crossterm::event::KeyCode;
use crate::config::{Layout as Dash, FILTER_NAMES};
use crate::engine::AlertKind;
use crate::scraper::{Mov, Ticker};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use std::time::Duration;


fn kind_style(kind: AlertKind) -> Style {
    match kind {
        AlertKind::New => Style::new().fg(pal().on_accent).bg(pal().new_bg),
        AlertKind::Pump => Style::new().fg(pal().on_accent).bg(pal().pump_bg),
        AlertKind::Spike => Style::new().fg(pal().on_accent).bg(pal().spike_bg),
        AlertKind::Setup => Style::new().fg(pal().on_setup).bg(pal().setup_bg).bold(),
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
        .border_style(Style::new().fg(pal().border))
        .title(title)
}

fn title(s: &str) -> Line<'static> {
    Line::from(Span::styled(format!(" {s} "), Style::new().fg(pal().accent).bold()))
}

fn keyhint(k: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {k} "), Style::new().fg(pal().on_accent).bg(pal().dim)),
        Span::styled(format!(" {label}  "), Style::new().fg(pal().dim)),
    ]
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.hits = Hits::default();
    app.plot = None;
    app.halftones.clear();
    // The boot screen keeps the original colours; everything else follows the theme.
    theme::set(if app.view == View::Boot { theme::NOIR } else { app.palette });
    fmt::set(app.formats());
    if app.view != View::Boot {
        f.render_widget(Block::new().style(Style::new().bg(pal().bg).fg(pal().text)), f.area());
    }
    match app.view {
        View::Boot => boot(f, app),
        View::Dashboard => dashboard(f, app),
        View::Columns => columns(f, app),
        View::Chart => chart_view(f, app),
        View::SetupMenu
        | View::SetupConfig
        | View::SetupBlacklist
        | View::SetupFilters
        | View::Storage
        | View::Health
        | View::Presets
        | View::SetupTheme
        | View::SetupSound
        | View::SetupFormat => setup_shell(f, app),
    }
}

// ───────────────────────────── boot / health ─────────────────────────────

fn boot(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let t = app.started_at.elapsed().as_secs_f64();
    scene(f, app, area, t);

    let boxed = |title: &str| {
        let b = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Rgb(142, 166, 200)))
            .style(Style::new().bg(Color::Rgb(10, 16, 36)));
        if title.is_empty() {
            b
        } else {
            b.title(Line::from(Span::styled(format!(" {title} "), Style::new().fg(Color::Rgb(255, 253, 242)).bold())).centered())
        }
    };
    // Each box is only as wide as its content (+ borders), so the scene shows around it.
    let hint = " ↑/↓ move  enter select  q quit ";
    let item_w = BOOT_ITEMS.iter().map(|s| s.chars().count() + 4).max().unwrap_or(0);
    let menu_iw = item_w.max(hint.chars().count()).max(APP_NAME.len() + 4);
    // connection checks, run at startup: dot, name padded to the longest, message
    let name_w = app.health.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    let health_iw = if app.health.is_empty() {
        25
    } else {
        app.health.iter().map(|r| 3 + name_w + 2 + r.message.chars().count() + 1).max().unwrap_or(0)
    };
    let menu_w = (menu_iw as u16 + 2).min(area.width);
    let health_w = (health_iw as u16 + 2).min(area.width);
    let menu_h = BOOT_ITEMS.len() as u16 + 5;
    let health_h = app.health.len().max(1) as u16 + 2;
    let mid = |w: u16| area.x + (area.width - w) / 2;
    // the menu itself is centred; the health box and version hang below it
    let y = area.y + area.height.saturating_sub(menu_h) / 2;
    let menu = Rect::new(mid(menu_w), y, menu_w, menu_h).intersection(area);
    let health_r = Rect::new(mid(health_w), y + menu_h, health_w, health_h).intersection(area);

    let mut lines = vec![Line::raw("")];
    for (i, item) in BOOT_ITEMS.iter().enumerate() {
        lines.push(if i == app.boot_idx {
            Line::from(Span::styled(format!(" ▶ {item:<iw$}", iw = menu_iw - 3), Style::new().fg(pal().on_accent).bg(pal().green).bold()))
        } else {
            Line::from(Span::styled(format!("   {item:<iw$}", iw = menu_iw - 3), Style::new().fg(pal().text)))
        });
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(hint, Style::new().fg(pal().dim))));
    f.render_widget(Clear, menu);
    f.render_widget(Paragraph::new(lines).block(boxed(APP_NAME)), menu);

    let health: Vec<Line> = if app.health.is_empty() {
        let msg = if app.health_running { " checking connections..." } else { " connection checks not run" };
        vec![Line::styled(msg, Style::new().fg(pal().dim))]
    } else {
        app.health
            .iter()
            .map(|r| {
                let color = if r.ok { pal().green } else { pal().red };
                Line::from(vec![
                    Span::styled(" ● ", Style::new().fg(color)),
                    Span::styled(format!("{:<name_w$}  ", r.name), Style::new().fg(pal().text)),
                    // long messages (e.g. a proxy URL) are cut by the box on a narrow screen
                    Span::styled(r.message.clone(), Style::new().fg(color)),
                ])
            })
            .collect()
    };
    f.render_widget(Clear, health_r);
    f.render_widget(Paragraph::new(health).block(boxed("")), health_r);
    let version = format!(" {} ", version_line());
    let vw = (version.chars().count() as u16).min(area.width);
    let version_r = Rect::new(mid(vw), y + menu_h + health_h, vw, 1).intersection(area);
    f.render_widget(
        Paragraph::new(Span::styled(version, Style::new().fg(Color::Rgb(200, 212, 235)).bg(Color::Rgb(10, 16, 36)))),
        version_r,
    );
}

/// Config -> Storage: the app home as a file list with sizes; d deletes (after y).
fn storage_view(f: &mut Frame, app: &App, body: Rect) {
    let Some(b) = app.storage.as_ref() else { return };
    let total: u64 = b.entries.iter().map(|e| e.bytes).sum();
    let rows: Vec<Row> = b
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let name = if e.dir { format!("▸ {}/", e.name) } else { format!("  {}", e.name) };
            let when = e
                .modified
                .map(|m| {
                    let ts = chrono::DateTime::<chrono::Utc>::from(m).timestamp();
                    format!("{} {}", fmt::date(ts), fmt::time(ts))
                })
                .unwrap_or_default();
            let st = if e.dir { Style::new().fg(pal().accent) } else { Style::new().fg(pal().text) };
            let row = Row::new(vec![
                Cell::from(Span::styled(name, st)),
                Cell::from(Line::from(crate::storage::human(e.bytes)).alignment(Alignment::Right)),
                Cell::from(Span::styled(when, Style::new().fg(pal().dim))),
            ]);
            if i % 2 == 1 { row.style(Style::new().bg(pal().zebra)) } else { row }
        })
        .collect();
    let header = Row::new([
        Cell::from(Span::styled("Name", Style::new().fg(pal().dim).bold())),
        Cell::from(Line::from(Span::styled("Size", Style::new().fg(pal().dim).bold())).alignment(Alignment::Right)),
        Cell::from(Span::styled("Modified", Style::new().fg(pal().dim).bold())),
    ]);
    let ttl = Line::from(vec![
        Span::styled(" STORAGE ", Style::new().fg(pal().accent).bold()),
        Span::styled(format!("{} · {} ", b.rel(), crate::storage::human(total)), Style::new().fg(pal().dim)),
    ]);
    let width = body.width.min(110);
    let [list, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(Rect { width, ..body });
    let table = Table::new(rows, [Constraint::Fill(1), Constraint::Length(10), Constraint::Length(20)])
        .header(header)
        .column_spacing(2)
        .block(panel(ttl))
        .row_highlight_style(Style::new().bg(pal().select).fg(pal().text))
        .highlight_symbol("▌")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    let mut state = TableState::default().with_selected((!b.entries.is_empty()).then_some(b.cursor));
    f.render_stateful_widget(table, list, &mut state);
    if b.entries.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("empty folder", Style::new().fg(pal().faint))), centered(list, 12, 1));
    }
    let line = match (b.confirm, b.selected(), &b.msg) {
        (true, Some(e), _) => Line::styled(
            format!(" Delete {}{} ({})? y = yes, any other key = no", e.name, if e.dir { "/ and everything in it" } else { "" }, crate::storage::human(e.bytes)),
            Style::new().fg(pal().amber).bold(),
        ),
        (_, _, Some(m)) => Line::styled(format!(" {m}"), Style::new().fg(pal().dim)),
        _ => Line::styled(format!(" {}", b.cwd.display()), Style::new().fg(pal().faint)),
    };
    f.render_widget(Paragraph::new(line).block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(pal().faint))), status);
}

/// Release name shown under the start menu.
const CODENAME: &str = "Taj Mahal";

/// Name, version and release name, e.g. "MarketGUI v1.0.0 (Taj Mahal)".
fn version_line() -> String {
    format!("{APP_NAME} v{} ({CODENAME})", env!("CARGO_PKG_VERSION"))
}

/// Moonlit mausoleum fitted to `area` as halftone dots (two per cell, square).
fn scene(f: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let ht = crate::art::scene_halftone(t, area.width as usize, area.height as usize * 2);
    halftone(f, app, area, ht);
}

/// Gear and wrench animation at the bottom of `area`, centred, as halftone dots: full size where
/// it fits, else box-averaged down to fit, down to a quarter size; else nothing.
fn gear(f: &mut Frame, app: &mut App, area: Rect, t: f64) {
    use crate::art::{gear_halftone, GEAR_H, GEAR_W};
    let s = (f64::from(area.width) / GEAR_W as f64).min(f64::from(area.height) / GEAR_H as f64).min(1.0);
    let (w, h) = ((GEAR_W as f64 * s) as u16, (GEAR_H as f64 * s) as u16);
    if w < (GEAR_W / 4) as u16 || h < (GEAR_H / 4) as u16 {
        return;
    }
    let r = Rect::new(area.x + (area.width - w) / 2, area.bottom() - h, w, h);
    halftone(f, app, r, gear_halftone(t, w as usize, h as usize * 2));
}

/// Draws `ht` (`area.width` x `2 * area.height` dots) over `area`. The window app paints the dots
/// itself (`App::halftones`), so the cells are left blank; the terminal gets one glyph per cell
/// (` · ∙ • ●` by coverage) in the dots' colour.
fn halftone(f: &mut Frame, app: &mut App, area: Rect, ht: crate::art::Halftone) {
    let rgb = |c: crate::art::Rgb| Color::Rgb(c[0] as u8, c[1] as u8, c[2] as u8);
    let buf = f.buffer_mut();
    for cy in 0..area.height {
        for cx in 0..area.width {
            let cell = &mut buf[(area.x + cx, area.y + cy)];
            if app.pixel_chart {
                cell.reset();
                continue;
            }
            let (x, y) = (cx as usize, cy as usize * 2);
            let ((a, ka), (b, kb)) = (ht.dots[y * ht.w + x], ht.dots[(y + 1) * ht.w + x]);
            let k = (ka + kb) / 2.0;
            let col = if k > 0.0 { [0, 1, 2].map(|i| (a[i] * ka + b[i] * kb) / (ka + kb)) } else { [0.0; 3] };
            let glyph = match k {
                k if k < 0.06 => ' ',
                k if k < 0.2 => '·',
                k if k < 0.4 => '∙',
                k if k < 0.65 => '•',
                _ => '●',
            };
            cell.set_char(glyph).set_fg(rgb(col));
            if let Some(bg) = ht.bg {
                cell.set_bg(rgb(bg));
            }
        }
    }
    if app.pixel_chart {
        app.halftones.push((area, ht));
    }
}

fn chart_view(f: &mut Frame, app: &mut App) {
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

/// Key hints in one line; hints with a key code are clickable.
fn hint_bar(f: &mut Frame, app: &mut App, area: Rect, hints: &[(&str, &str, Option<KeyCode>)]) {
    let mut spans = Vec::new();
    let mut x = area.x;
    for &(k, l, code) in hints {
        let hint = keyhint(k, l);
        let w = hint.iter().map(|s| s.width() as u16).sum::<u16>();
        if let Some(code) = code {
            app.hits.keys.push((Rect::new(x, area.y, w, 1).intersection(area), code));
        }
        x = x.saturating_add(w);
        spans.extend(hint);
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Health Check (#2): the connection checks, then the System group; one name column padded to
/// the longest name, messages aligned after it, a blank line between the groups.
fn health(f: &mut Frame, app: &App, area: Rect) {
    let conn: Vec<(&str, String, Color)> = if app.health_running {
        vec![("Checks", "running...".into(), pal().dim)]
    } else if app.health.is_empty() {
        vec![("Checks", "press Enter to run them".into(), pal().dim)]
    } else {
        app.health.iter().map(|r| (r.name, r.message.clone(), if r.ok { pal().green } else { pal().red })).collect()
    };
    let sys = system_rows(app);
    let name_w = conn.iter().chain(&sys).map(|r| r.0.chars().count()).max().unwrap_or(0);
    let row = |(name, msg, col): &(&str, String, Color)| {
        Line::from(vec![
            Span::styled("  ● ", Style::new().fg(*col)),
            Span::styled(format!("{name:<name_w$}  "), Style::new().fg(pal().dim)),
            Span::styled(msg.clone(), Style::new().fg(*col)),
        ])
    };
    let head = |s: &str| Line::styled(format!("  {s}"), Style::new().fg(pal().accent).bold());
    let mut lines = vec![Line::raw(""), head("CONNECTIONS")];
    lines.extend(conn.iter().map(row));
    lines.push(Line::raw(""));
    lines.push(head("SYSTEM"));
    lines.extend(sys.iter().map(row));
    lines.push(Line::raw(""));
    let w = lines.iter().map(|l| l.width() as u16).max().unwrap_or(0) + 4;
    let rect = Rect { width: area.width.min(w.max(40)), height: area.height.min(lines.len() as u16 + 2), ..area };
    f.render_widget(Paragraph::new(lines).block(panel(title("HEALTH CHECK"))), rect);
}

/// The System group of the health pane: (name, value, colour); "n/a" where a value cannot be read.
fn system_rows(app: &App) -> Vec<(&'static str, String, Color)> {
    use crate::sysmem;
    let gb = |b: u64| format!("{:.1} GB", b as f64 / 1_073_741_824.0);
    let level = |pct: f64| if pct >= 90.0 { pal().red } else if pct >= 75.0 { pal().amber } else { pal().text };
    let na = || ("n/a".to_string(), pal().faint);
    let ram = sysmem::ram().map_or_else(na, |(used, total)| {
        let pct = used as f64 / total as f64 * 100.0;
        (format!("{} / {} ({pct:.0}%)", gb(used), gb(total)), level(pct))
    });
    let app_ram = sysmem::process_ram().map_or_else(na, |b| (format!("{:.0} MB", b as f64 / 1_048_576.0), pal().text));
    let cpu = match app.cpu_pct {
        Some(p) => (format!("{p:.0}%"), level(p)),
        None if cfg!(windows) => ("measuring...".into(), pal().dim),
        None => na(),
    };
    let data = app.storage_bytes.map_or_else(na, |b| (format!("{:.0} MB", b as f64 / 1_048_576.0), pal().text));
    let disk = app.home.as_deref().and_then(sysmem::disk).map_or_else(na, |(free, total)| {
        let col = if (free as f64) < total as f64 * 0.05 { pal().red } else { pal().text };
        (format!("{} free of {}", gb(free), gb(total)), col)
    });
    let requests = match crate::limiter::paused() {
        Some(d) => (format!("paused after HTTP 403, {} min left", d.as_secs().div_ceil(60)), pal().red),
        None => (format!("{} per minute", crate::limiter::per_minute()), pal().text),
    };
    let st = app.stream_status();
    let stream = if !app.stream_enabled() {
        ("off".into(), pal().dim)
    } else if app.stream_ok() {
        (format!("{:.0} Hz, {} subscriptions", st.msgs_per_sec, st.subs), pal().green)
    } else {
        ("disconnected".into(), pal().amber)
    };
    let os = app.os_version.clone().map_or_else(na, |s| (s, pal().text));
    [
        ("Uptime", (uptime(app.started_at.elapsed()), pal().text)),
        ("RAM", ram),
        ("App RAM", app_ram),
        ("CPU", cpu),
        ("Cores", (format!("{} logical", sysmem::cores()), pal().text)),
        ("Data folder", data),
        ("Disk", disk),
        ("Webull requests", requests),
        ("Stream", stream),
        ("OS", os),
    ]
    .into_iter()
    .map(|(n, (v, c))| (n, v, c))
    .collect()
}

// ───────────────────────────── dashboard ─────────────────────────────

/// The dashboard in the chosen layout (header tabs or `w`). The tape and the key hints can be
/// switched off in Configuration; their rows then go to the main area.
fn dashboard(f: &mut Frame, app: &mut App) {
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
    }
    if app.cfg.key_hints {
        footer_bar(f, app, footer);
    }
    if show_tape {
        ticker_tape(f, app, tape);
    }
}

/// Alerts panel (`alert_rows` rows plus header and borders) over the gainers list; the panel
/// is shown only while the list keeps at least 5 rows.
fn alerts_and_list(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.cfg.alert_rows;
    let alerts_h = if area.height >= rows + 3 + 8 { rows + 3 } else { 0 };
    let [alerts, gainers] = Layout::vertical([Constraint::Length(alerts_h), Constraint::Min(0)]).areas(area);
    if alerts_h > 0 {
        alerts_table(f, app, alerts);
    }
    gainers_table(f, app, gainers);
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

/// Bordered chart of `sym` in the current timeframe, with clickable timeframe tabs in the
/// title. 1m comes from the candles collected today (plus backfill); the others from Webull.
fn chart_panel(f: &mut Frame, app: &mut App, area: Rect, sym: &str) {
    let tf = app.timeframe;
    let mut ttl = vec![Span::styled(" CHART ", Style::new().fg(pal().accent).bold()), Span::raw(" ")];
    let mut x = area.x + 1 + 8;
    for t in Tf::ALL {
        let label = format!(" {} ", t.label());
        let w = label.len() as u16;
        let st = if t == tf { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        app.hits.tf_tabs.push((Rect::new(x, area.y, w, 1).intersection(area), t));
        ttl.push(Span::styled(label, st));
        ttl.push(Span::raw(" "));
        x += w + 1;
    }
    let block = panel(Line::from(ttl));
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
    // Fetched bars are refreshed every few seconds; the live price keeps the bar in progress current.
    if let (Tf::M5 | Tf::H1 | Tf::H4 | Tf::D1, Some(t), Some(last)) = (tf, ticker, candles.last_mut()) {
        if t.price > 0.0 && chrono::Utc::now().timestamp() < last.start + tf.secs() {
            last.close = t.price;
            last.high = last.high.max(t.price);
            last.low = last.low.min(t.price);
        }
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
    let spec = crate::chart::Spec { tf, day_axis, empty: &empty, pan: app.pan, zoom: app.zoom, shift: app.shift, levels, note: &note, scale: app.scale, pixel: app.pixel_chart, now };
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
    let clock = Span::styled(countdown(secs), Style::new().fg(pal().text).bold());
    let full = Line::from(vec![
        Span::styled(format!(" {phase}"), Style::new().fg(pal().accent).bold()),
        Span::styled(format!("   {until} "), Style::new().fg(pal().dim)),
        clock.clone(),
    ]);
    let short = Line::from(vec![Span::raw(" "), clock]);
    let tabs: Vec<Span> = Dash::ALL
        .iter()
        .map(|&l| {
            let st = if l == app.cfg.layout { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
            Span::styled(format!(" {} ", l.label()), st)
        })
        .collect();
    // one space before the strip and after each tab
    let tabs_w = 1 + tabs.iter().map(|s| s.width() as u16 + 1).sum::<u16>();
    // the status dot and a space before it
    let dot_w = 3;
    let show_tabs = area.width >= tabs_w + short.width() as u16 + dot_w;
    if show_tabs {
        let mut x = area.x + 1;
        for (s, &l) in tabs.iter().zip(&Dash::ALL) {
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
        vec![Span::styled(format!("up {}  ", uptime(app.started_at.elapsed())), Style::new().fg(pal().faint))],
    ];
    // Config -> Storage lists and deletes this data
    if let Some(bytes) = app.storage_bytes {
        let mb = (bytes as f64 / 1_048_576.0).round();
        items.push(vec![Span::styled(format!("storage {mb:.0} MB  "), Style::new().fg(pal().dim))]);
    }
    if let Some((used, total)) = crate::sysmem::ram() {
        let pct = used as f64 / total as f64 * 100.0;
        let col = if pct >= 90.0 { pal().red } else if pct >= 75.0 { pal().amber } else { pal().dim };
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
fn countdown(secs: i64) -> String {
    let s = secs.max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// App uptime: `42s`, `5m 07s`, `3h 12m`.
fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

fn ticker_tape(f: &mut Frame, app: &App, area: Rect) {
    let mut cells: Vec<(char, Style)> = Vec::new();
    let push = |cells: &mut Vec<(char, Style)>, s: &str, st: Style| cells.extend(s.chars().map(|c| (c, st)));
    for t in app.tickers.iter().take(30) {
        let (arrow, color) = if t.change_ratio < 0.0 { ("▼", pal().red) } else { ("▲", pal().green) };
        push(&mut cells, "   ", Style::new());
        match status_kind(&t.status) {
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

/// A share count or market cap, "N/A" when unknown (0).
fn count_or_na(v: f64) -> String {
    if v == 0.0 { "N/A".into() } else { fmt::count(v) }
}

fn cell(t: &Ticker, key: &str) -> Cell<'static> {
    match key {
        "mov" => match t.mov {
            Mov::None => Cell::from(""),
            Mov::New => Cell::from(Span::styled("+", Style::new().fg(pal().accent).bold())),
            Mov::Moved(0) => Cell::from(Span::styled("  ·", Style::new().fg(pal().faint))),
            Mov::Moved(r) if r > 0 => Cell::from(Span::styled(format!("▲{r}"), Style::new().fg(pal().green))),
            Mov::Moved(r) => Cell::from(Span::styled(format!("▼{}", -r), Style::new().fg(pal().red))),
        },
        "symbol" => Cell::from(Span::styled(t.symbol.clone(), Style::new().bold())),
        "price" => Cell::from(Line::from(fmt::price(t.price)).alignment(Alignment::Right)),
        "change" => {
            let pct = t.change_ratio * 100.0;
            let c = if pct >= 0.0 { pal().green } else { pal().red };
            Cell::from(Line::from(Span::styled(format!("{pct:+.2}%"), Style::new().fg(c))).alignment(Alignment::Right))
        }
        "volume" => Cell::from(Line::from(fmt::count(t.volume as f64)).alignment(Alignment::Right)),
        "mcap" => Cell::from(Line::from(count_or_na(t.market_value)).alignment(Alignment::Right)),
        "float" => Cell::from(Line::from(count_or_na(t.float)).alignment(Alignment::Right)),
        "shares" => Cell::from(Line::from(count_or_na(t.total_shares)).alignment(Alignment::Right)),
        "volrate" => {
            let (s, c) = match t.vol_rate {
                _ if t.vol_stale => ("—".to_string(), pal().dim),
                None => ("…".to_string(), pal().faint),
                Some(r) if r < 0.5 => ("0".to_string(), pal().faint),
                Some(r) => (fmt::count(r), if t.vol_partial { pal().dim } else { pal().accent }),
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

/// Width the gainers panel needs for its enabled columns (borders, cursor column, spacing).
fn gainers_width(app: &App) -> u16 {
    let cols: u16 = COLUMNS.iter().filter(|c| app.cfg.column_visible(c.key)).map(|c| c.width + 1).sum();
    // borders 2, cursor column 1, columns with one space between: sum(width + 1) - 1 + 3
    cols + 2
}

fn gainers_table(f: &mut Frame, app: &mut App, area: Rect) {
    fit_columns(app, area.width.saturating_sub(2));
    let vol_m_label = vol_m_header(app.cfg.vol_m_window_seconds);

    let header = Row::new(app.visible_cols.iter().enumerate().map(|(vi, &ci)| {
        let col = &COLUMNS[ci];
        let mut label = if col.key == "volrate" { vol_m_label.clone() } else { col.title.to_string() };
        if app.sort_key == col.key {
            label.push(if app.sort_desc { '▼' } else { '▲' });
        }
        let st = if vi == app.sel_col {
            Style::new().fg(pal().on_accent).bg(pal().accent).bold()
        } else {
            Style::new().fg(pal().dim).bold()
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
            style = style.bg(pal().zebra);
        }
        // Filter grace: the row no longer passes a display filter and will vanish soon.
        if t.dim {
            style = style.fg(pal().faint).add_modifier(Modifier::DIM);
        }
        row.style(style)
    });

    let widths: Vec<Constraint> = app.visible_cols.iter().map(|&ci| Constraint::Length(COLUMNS[ci].width)).collect();
    let count = Span::styled(format!(" {} ", app.tickers.len()), Style::new().fg(pal().dim));
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(panel(Line::from(count)))
        .row_highlight_style(Style::new().bg(pal().select).fg(pal().text))
        .highlight_symbol("▌")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(if app.tickers.is_empty() { None } else { Some(app.row) });
    f.render_stateful_widget(table, area, &mut state);

    // Mouse targets: header cells (after the border and the 1-wide highlight column) and data rows.
    let mut x = area.x + 2;
    for (vi, &ci) in app.visible_cols.iter().enumerate() {
        let w = COLUMNS[ci].width;
        app.hits.gainer_header.push((Rect::new(x, area.y + 1, w, 1).intersection(area), vi));
        x += w + 1;
    }
    let shown = (area.height.saturating_sub(3) as usize).min(app.tickers.len().saturating_sub(state.offset()));
    app.hits.gainer_rows = Rect::new(area.x + 1, area.y + 2, area.width.saturating_sub(2), shown as u16).intersection(area);
    app.hits.gainer_offset = state.offset();
    if app.tickers.is_empty() {
        let msg = if app.is_fetching() { "Fetching..." } else { "No tickers match the current filters" };
        let r = centered(area, msg.len() as u16 + 2, 1);
        f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(pal().faint))), r);
    }
}

fn alerts_table(f: &mut Frame, app: &mut App, area: Rect) {
    let rows_fit = area.height.saturating_sub(3) as usize;
    let flash_window = Duration::from_millis(1500);
    let fresh = app.last_alert_at.is_some_and(|t| t.elapsed() < flash_window);
    // scrolled no further than the last page
    let total = app.engine.alerts().iter().filter(|a| app.alert_visible(a.kind)).count();
    app.alert_scroll = app.alert_scroll.min(total.saturating_sub(rows_fit));
    let skip = app.alert_scroll;

    let syms: Vec<String> =
        app.engine.alerts().iter().rev().filter(|a| app.alert_visible(a.kind)).skip(skip).take(rows_fit).map(|a| a.symbol.clone()).collect();
    app.hits.alert_rows = Rect::new(area.x + 1, area.y + 2, area.width.saturating_sub(2), syms.len() as u16).intersection(area);
    app.hits.alert_syms = syms;
    let app = &*app;
    let visible: Vec<_> = app.engine.alerts().iter().rev().filter(|a| app.alert_visible(a.kind)).skip(skip).take(rows_fit).collect();
    let flash = app.last_alert_at.filter(|_| fresh);
    draw_alerts(f, area, &visible, flash, app.filter_name());
}

/// The alerts panel with `visible` (newest first); the newest row flashes when its alert is the
/// one fired at `flash`.
fn draw_alerts(f: &mut Frame, area: Rect, visible: &[&crate::engine::Alert], flash: Option<std::time::Instant>, filter: &str) {
    let rows = visible.iter().enumerate().map(|(i, a)| {
        let pct = a.change_ratio * 100.0;
        let mut row = Row::new(vec![
            Cell::from(Span::styled(fmt::time_s(a.epoch), Style::new().fg(pal().dim))),
            Cell::from(Span::styled(a.symbol.clone(), Style::new().fg(pal().text).bold())),
            Cell::from(Span::styled(format!("{:^7}", a.kind.label()), kind_style(a.kind))),
            Cell::from(
                Line::from(Span::styled(format!("{pct:+.1}%"), Style::new().fg(if pct >= 0.0 { pal().green } else { pal().red })))
                    .alignment(Alignment::Right),
            ),
            Cell::from(Span::styled(a.detail(), Style::new().fg(pal().dim))),
        ]);
        if i == 0 && flash == Some(a.at) {
            row = row.style(Style::new().bg(pal().flash));
        } else if i % 2 == 1 {
            row = row.style(Style::new().bg(pal().zebra));
        }
        row
    });

    let header = Row::new(["Time", "Symbol", "Type", "% Chg", "Detail"].map(|h| {
        let l = Line::from(Span::styled(h, Style::new().fg(pal().dim).bold()));
        Cell::from(if h == "% Chg" { l.alignment(Alignment::Right) } else { l })
    }));
    let widths = [
        // 12h times are wider
        Constraint::Length(visible.iter().map(|a| fmt::time_s(a.epoch).len() as u16).max().unwrap_or(0).max(8)),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Min(8),
    ];
    let t = Table::new(rows, widths).header(header).column_spacing(1).block(panel(Line::from(vec![
        Span::styled(" Alerts ", Style::new().fg(pal().accent).bold()),
        Span::styled(format!("{filter} "), Style::new().fg(pal().dim)),
    ])));
    f.render_widget(t, area);
    if visible.is_empty() {
        let msg = format!("No {filter} alerts");
        f.render_widget(
            Paragraph::new(Span::styled(msg.clone(), Style::new().fg(pal().faint))),
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
    hint_bar(
        f,
        app,
        area,
        &[
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
        ],
    );
}

// ───────────────────────────── setup ─────────────────────────────

/// Width of the Config sidebar (#1).
const SIDEBAR_W: u16 = 26;
/// Settings label column and value column widths in Configuration.
const LABEL_W: usize = 22;
const VALUE_W: usize = 26;
/// Settings pane (#2) widths of the items with a preview (#3) beside it.
const FORM_W: u16 = (LABEL_W + VALUE_W + 10) as u16;
const PRESETS_W: u16 = 36;
const THEME_W: u16 = 34;
const BLACKLIST_W: u16 = BL_COLS as u16 * 12 + 2;
const FILTERS_W: u16 = 80;
const FORMAT_W: u16 = 64;

/// Config: the sidebar (#1), the selected item's settings (#2, shown live while the sidebar has
/// focus), its preview (#3) for Configuration, Presets, Theme and Blacklist, and one hint bar.
fn setup_shell(f: &mut Frame, app: &mut App) {
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let [side, rest] = Layout::horizontal([Constraint::Length(SIDEBAR_W), Constraint::Min(0)]).areas(body);
    sidebar(f, app, side);
    let item = SETUP_VIEWS[app.setup_idx];
    let active = app.view != View::SetupMenu;
    let pane_w = match item {
        View::SetupConfig => Some(FORM_W),
        View::Presets => Some(PRESETS_W),
        View::SetupTheme => Some(THEME_W),
        View::SetupBlacklist => Some(BLACKLIST_W),
        View::SetupFilters => Some(FILTERS_W),
        View::SetupFormat => Some(FORMAT_W),
        _ => None,
    };
    let (pane, preview) = match pane_w {
        Some(w) if rest.width >= w + 30 => {
            let [a, b] = Layout::horizontal([Constraint::Length(w), Constraint::Min(0)]).areas(rest);
            (a, Some(b))
        }
        _ => (rest, None),
    };
    match item {
        View::SetupConfig => config_form(f, app, pane, active),
        View::Presets => presets_list(f, app, pane),
        View::SetupTheme => theme_list(f, app, pane),
        View::SetupBlacklist => setup_blacklist(f, app, pane),
        View::SetupFilters => setup_filters(f, app, pane),
        View::SetupSound => setup_sound(f, app, pane),
        View::SetupFormat => setup_format(f, app, pane, active),
        View::Storage => storage_view(f, app, pane),
        _ => health(f, app, pane),
    }
    match (item, preview) {
        (View::SetupConfig, Some(p)) => filter_preview(f, &app.snapshot, &app.preview_filters(), p),
        (View::Presets, Some(p)) => preset_preview(f, app, p),
        (View::SetupTheme, Some(p)) => theme_preview(f, app, p),
        (View::SetupBlacklist, Some(p)) => blacklisted_preview(f, app, p),
        (View::SetupFilters, Some(p)) => alerts_preview(f, app, p),
        (View::SetupFormat, Some(p)) => format_preview(f, p),
        _ => {}
    }
    let esc = ("esc", "back", Some(KeyCode::Esc));
    let hints: Vec<(&str, &str, Option<KeyCode>)> = match app.view {
        View::SetupMenu => vec![("↑↓", "item", None), ("enter/→", "open", Some(KeyCode::Enter)), esc],
        View::SetupConfig => vec![
            ("tab/↑↓", "field", None),
            ("←→", "choose", None),
            ("p", "recommended preset", Some(KeyCode::Char('p'))),
            ("enter", "save", Some(KeyCode::Enter)),
            ("esc", "cancel", Some(KeyCode::Esc)),
        ],
        View::Presets if app.preset_input.is_some() => vec![("enter", "save", Some(KeyCode::Enter)), ("esc", "cancel", Some(KeyCode::Esc))],
        View::Presets if app.preset_cursor == app.presets.len() => {
            vec![("↑↓", "move", None), ("enter", "save as…", Some(KeyCode::Enter)), esc]
        }
        View::Presets => vec![
            ("↑↓", "move", None),
            ("enter", "load", Some(KeyCode::Enter)),
            ("d", "delete", Some(KeyCode::Char('d'))),
            esc,
        ],
        View::SetupTheme => vec![("↑↓", "preview", None), ("enter", "keep", Some(KeyCode::Enter)), ("esc", "revert", Some(KeyCode::Esc))],
        View::SetupBlacklist => vec![("arrows", "move", None), ("enter", "toggle", Some(KeyCode::Enter)), ("a", "type symbol", Some(KeyCode::Char('a'))), esc],
        View::SetupFilters if app.filter_cursor == FILTER_NAMES.len() => vec![("↑↓", "move", None), ("←→", "rows", None), esc],
        View::SetupFilters => vec![("↑↓", "move", None), ("enter", "toggle", Some(KeyCode::Enter)), esc],
        View::SetupSound if app.sound_cursor == 1 => vec![("↑↓", "move", None), ("←→", "volume", None), ("enter", "test", Some(KeyCode::Enter)), esc],
        View::SetupSound => vec![("↑↓", "move", None), ("enter", "mute on/off", Some(KeyCode::Enter)), esc],
        View::SetupFormat if app.fmt_cursor < fmt::TYPED_ROWS => {
            vec![("↑↓", "move", None), ("←→", "choose", None), ("type", "custom", None), ("enter", "save", Some(KeyCode::Enter)), ("esc", "cancel", Some(KeyCode::Esc))]
        }
        View::SetupFormat => vec![("↑↓", "move", None), ("←→", "choose", None), ("enter", "save", Some(KeyCode::Enter)), ("esc", "cancel", Some(KeyCode::Esc))],
        View::Storage => vec![
            ("↑↓", "move", None),
            ("enter/→", "open folder", None),
            ("←/bksp", "up", None),
            ("d/del", "delete", Some(KeyCode::Char('d'))),
            ("r", "refresh", Some(KeyCode::Char('r'))),
            esc,
        ],
        _ => vec![("enter", "run again", Some(KeyCode::Enter)), esc],
    };
    hint_bar(f, app, footer, &hints);
}

/// Config sidebar (#1): the items (clickable), the selected one's description, and the gear at
/// the bottom when it fits.
fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.view == View::SetupMenu;
    let block = panel(title("CONFIG"));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut y = inner.y + 1;
    for (i, (name, _)) in SETUP_ITEMS.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        let sel = i == app.setup_idx;
        let st = match (sel, focused) {
            (true, true) => Style::new().fg(pal().on_accent).bg(pal().green).bold(),
            (true, false) => Style::new().fg(pal().accent).bg(pal().select).bold(),
            _ => Style::new().fg(pal().text),
        };
        let r = Rect::new(inner.x, y, inner.width, 1);
        let text = format!(" {} {name:<w$}", if sel { "▶" } else { " " }, w = inner.width.saturating_sub(3) as usize);
        f.render_widget(Paragraph::new(Span::styled(text, st)), r);
        app.hits.setup_items.push((r, i));
        y += 2;
    }
    let desc_h = 4;
    let desc = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), desc_h).intersection(inner);
    let text = Paragraph::new(SETUP_ITEMS[app.setup_idx].1).style(Style::new().fg(pal().dim).italic()).wrap(Wrap { trim: true });
    f.render_widget(text, desc);
    let below = (y + desc_h + 1).min(inner.bottom());
    let t = app.started_at.elapsed().as_secs_f64();
    gear(f, app, Rect::new(inner.x, below, inner.width, inner.bottom() - below), t);
}

/// Configuration (#2): the settings form and the focused field's description; `active` = the
/// form has focus (else no field is marked).
fn config_form(f: &mut Frame, app: &App, form: Rect, active: bool) {
    let section = |name: &str| {
        Line::from(vec![
            Span::styled(format!("  {name} "), Style::new().fg(pal().accent).bold()),
            Span::styled("─".repeat(LABEL_W + VALUE_W - name.len()), Style::new().fg(pal().faint)),
        ])
    };
    let mut lines = vec![Line::raw("")];
    if app.first_run {
        let amber = Style::new().fg(pal().amber).bold();
        lines.push(Line::styled("  First run: set the display filters so the list holds", amber));
        lines.push(Line::styled("  only the tickers you want; charts are loaded for every", amber));
        lines.push(Line::styled("  listed ticker. p loads the recommended preset.", amber));
        lines.push(Line::raw(""));
    }
    lines.push(section("SCREENER & ALERTS"));
    for (i, fld) in FIELDS.iter().enumerate() {
        match fld.id {
            FieldId::MinChange => {
                lines.push(Line::raw(""));
                lines.push(section("DISPLAY FILTERS"));
            }
            FieldId::TickerTape => {
                lines.push(Line::raw(""));
                lines.push(section("LAYOUT"));
            }
            _ => {}
        }
        let focused = active && i == app.focus;
        let label_st = if focused { Style::new().fg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        let mark = if focused { "▶" } else { " " };
        let raw = app.inputs[i].clone();
        // every value starts in the same column; choices put their arrows around it
        let picked = matches!(fld.kind, FieldKind::Choice(_) | FieldKind::Toggle);
        let shown = if focused && !picked { format!("{raw}▏") } else { raw };
        let (open, close) = if picked { ("◂ ", " ▸") } else { ("  ", "  ") };
        let val_st = if focused { Style::new().fg(pal().text).bg(pal().input_bg) } else { Style::new().fg(pal().text) };
        let arrow_st = Style::new().fg(if focused { pal().accent } else { pal().faint });
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} {:<w$}", fld.label, w = LABEL_W), label_st),
            Span::styled(open, arrow_st),
            Span::styled(format!("{shown:<w$}", w = VALUE_W - 4), val_st),
            Span::styled(close, arrow_st),
        ]));
    }
    if let Some(e) = &app.config_err {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("  {e}"), Style::new().fg(pal().red).bold()));
    }

    let [form_top, tip] = Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).areas(form);
    f.render_widget(Paragraph::new(lines).block(panel(title("CONFIGURATION"))), form_top);
    f.render_widget(
        Paragraph::new(FIELDS[app.focus].desc)
            .style(Style::new().fg(pal().tip))
            .wrap(Wrap { trim: true })
            .block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(pal().faint))),
        tip,
    );
}

/// Filter preview (#3): every ticker of `snapshot`, those failing `filters` dimmed.
fn filter_preview(f: &mut Frame, snapshot: &[Ticker], filters: &crate::engine::Filters, preview: Rect) {
    let passing = snapshot.iter().filter(|t| filters.matches(t)).count();
    let rows = snapshot.iter().map(|t| {
        let st = if filters.matches(t) { Style::new().fg(pal().text) } else { Style::new().fg(pal().faint) };
        Row::new(vec![
            Cell::from(t.symbol.clone()),
            Cell::from(Line::from(fmt::price(t.price)).alignment(Alignment::Right)),
            Cell::from(Line::from(format!("{:+.1}%", t.change_ratio * 100.0)).alignment(Alignment::Right)),
            Cell::from(Line::from(fmt::count(t.volume as f64)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.market_value)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.float)).alignment(Alignment::Right)),
            Cell::from(Line::from(count_or_na(t.total_shares)).alignment(Alignment::Right)),
        ])
        .style(st)
    });
    let header = Row::new(["Symbol", "Price", "% Chg", "Volume", "Mkt Cap", "Float", "Shares"].map(|h| {
        let l = Line::from(Span::styled(h, Style::new().fg(pal().dim).bold()));
        Cell::from(if h == "Symbol" { l } else { l.alignment(Alignment::Right) })
    }));
    let widths = [8, 10, 9, 10, 10, 9, 9].map(Constraint::Length);
    let preview = Rect { width: preview.width.min(90), ..preview };
    let ttl = Line::from(vec![
        Span::styled(" Filter preview ", Style::new().fg(pal().accent).bold()),
        Span::styled(format!("{passing}/{} pass ", snapshot.len()), Style::new().fg(pal().dim)),
    ]);
    f.render_widget(Table::new(rows, widths).header(header).column_spacing(1).block(panel(ttl)), preview);
    if snapshot.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No data yet", Style::new().fg(pal().faint))), centered(preview, 11, 1));
    }
}

/// Presets (#2): the built-in preset and the saved ones, the "Save current config as" row, the
/// name prompt, a question or message.
fn presets_list(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![Line::raw("")];
    for (i, name) in app.presets.iter().enumerate() {
        let sel = i == app.preset_cursor;
        let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        lines.push(Line::from(vec![
            Span::raw(if sel { " ▶ " } else { "   " }),
            Span::styled(format!(" {name} "), st),
            Span::styled(if crate::presets::is_builtin(name) { " built in" } else { "" }, Style::new().fg(pal().dim).italic()),
        ]));
    }
    let sel = app.preset_cursor == app.presets.len();
    let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().accent) };
    lines.push(Line::from(vec![Span::raw(if sel { " ▶ " } else { "   " }), Span::styled(" + Save current config as… ", st)]));
    lines.push(Line::raw(""));
    if let Some(buf) = &app.preset_input {
        lines.push(Line::styled(" Save the current config as:", Style::new().fg(pal().accent)));
        lines.push(Line::from(vec![Span::raw(" "), Span::styled(format!("{buf}▏"), Style::new().fg(pal().text).bg(pal().input_bg))]));
    }
    let amber = Style::new().fg(pal().amber).bold();
    match &app.preset_ask {
        Some(PresetAsk::Overwrite(n)) => lines.push(Line::styled(format!(" Replace preset {n}? y = yes, any other key = no"), amber)),
        Some(PresetAsk::Delete(n)) => lines.push(Line::styled(format!(" Delete preset {n}? y = yes, any other key = no"), amber)),
        None => {}
    }
    if let Some(m) = &app.preset_msg {
        lines.push(Line::styled(format!(" {m}"), Style::new().fg(pal().dim)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title("PRESETS"))), area);
}

/// Presets preview (#3): the selected preset's (or, on the save row, the config in use's)
/// Configuration values and its filter preview.
fn preset_preview(f: &mut Frame, app: &App, area: Rect) {
    let name = app.presets.get(app.preset_cursor).cloned().unwrap_or_else(|| "Current config".to_string());
    let Some(inputs) = &app.preset_inputs else {
        let msg = Line::styled(" this preset could not be read", Style::new().fg(pal().faint));
        f.render_widget(Paragraph::new(msg).block(panel(title(&name))), area);
        return;
    };
    let [vals, prev] = Layout::vertical([Constraint::Length(FIELDS.len() as u16 + 3), Constraint::Min(3)]).areas(area);
    let lines: Vec<Line> = FIELDS
        .iter()
        .zip(inputs)
        .map(|(fld, v)| {
            Line::from(vec![
                Span::styled(format!("  {:<w$}", fld.label, w = LABEL_W), Style::new().fg(pal().dim)),
                Span::styled(v.clone(), Style::new().fg(pal().text)),
            ])
        })
        .collect();
    let mut lines = lines;
    if let Some(s) = &app.preset_summary {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<w$}", "Time & Format", w = LABEL_W), Style::new().fg(pal().dim)),
            Span::styled(s.clone(), Style::new().fg(pal().text)),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(panel(title(&name))), Rect { width: vals.width.min(90), ..vals });
    filter_preview(f, &app.snapshot, &crate::app::filters_from_inputs(inputs), prev);
}

/// Theme (#2): every theme, the highlighted one previewed on the whole screen, the saved one marked.
fn theme_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = theme::names()
        .iter()
        .map(|(id, name)| {
            let saved = if *id == app.cfg.theme { " in use" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {name} "), Style::new().fg(pal().text)),
                Span::styled(saved, Style::new().fg(pal().dim).italic()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(panel(title("THEME")))
        .highlight_symbol(" ▶")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
        .highlight_style(Style::new().fg(pal().on_accent).bg(pal().accent).bold());
    let mut state = ListState::default().with_selected(Some(app.theme_cursor));
    f.render_stateful_widget(list, area, &mut state);
}

/// Theme preview (#3): a small dashboard in the previewed theme: a few gainers rows and the
/// charted (or first) ticker's chart; sample rows and candles while there is no data.
fn theme_preview(f: &mut Frame, app: &App, area: Rect) {
    let area = Rect { width: area.width.min(90), ..area };
    let mut rows: Vec<Ticker> = if app.tickers.is_empty() { app.snapshot.clone() } else { app.tickers.clone() };
    if rows.is_empty() {
        let mk = |s: &str, price, chg, volume, status: &str| Ticker { symbol: s.into(), price, change_ratio: chg, volume, status: status.into(), ..Default::default() };
        rows = vec![
            mk("SMPL", 3.42, 0.524, 18_400_000, "SETUP"),
            mk("DEMO", 1.18, 0.317, 6_200_000, "NEW"),
            mk("TEST", 7.05, 0.211, 2_900_000, "PUMP"),
            mk("ABCD", 0.62, -0.048, 950_000, ""),
        ];
    }
    rows.truncate(6);
    let [list, chart] = Layout::vertical([Constraint::Length(rows.len() as u16 + 3), Constraint::Min(0)]).areas(area);
    let keys = ["mov", "symbol", "price", "change", "volume", "status"];
    let widths: Vec<Constraint> = keys.iter().map(|k| Constraint::Length(COLUMNS.iter().find(|c| c.key == *k).map_or(8, |c| c.width))).collect();
    let header = Row::new(keys.map(|k| {
        let ttl = COLUMNS.iter().find(|c| c.key == k).map_or("", |c| c.title);
        Cell::from(Span::styled(ttl, Style::new().fg(pal().dim).bold()))
    }));
    let body = rows.iter().enumerate().map(|(i, t)| {
        let st = if i == 0 { Style::new().bg(pal().select).fg(pal().text) } else if i % 2 == 1 { Style::new().bg(pal().zebra) } else { Style::new() };
        Row::new(keys.map(|k| cell(t, k))).style(st)
    });
    let name = theme::names().get(app.theme_cursor).map_or(String::new(), |(_, n)| n.to_string());
    f.render_widget(Table::new(body, widths).header(header).column_spacing(1).block(panel(title(&format!("Preview: {name}")))), list);

    let sym = app.charted_symbol().unwrap_or(&rows[0].symbol).to_string();
    let tf = app.timeframe;
    let mut candles: Vec<Candle> = match tf {
        Tf::M1 => app.engine.candles(&sym).map(|c| c.iter().copied().collect()).unwrap_or_default(),
        _ => app.bars.get(&(sym.clone(), tf)).cloned().unwrap_or_default(),
    };
    if candles.is_empty() {
        candles = sample_candles(tf.secs());
    }
    let pct = rows.iter().find(|t| t.symbol == sym).map_or(0.0, |t| t.change_ratio * 100.0);
    let block = panel(title("CHART"));
    let inner = block.inner(chart).inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(block, chart);
    let day_axis = matches!(tf, Tf::H4 | Tf::D1);
    let now = chrono::Utc::now().timestamp();
    let spec = crate::chart::Spec { tf, day_axis, empty: "", pan: 0, zoom: 1.0, shift: 0.0, levels: &[], note: "", scale: crate::chart::DEFAULT_SCALE, pixel: false, now };
    crate::chart::draw(f.buffer_mut(), inner, &sym, &candles, pct, &spec);
}

/// Made-up candles (a steady climb with pullbacks) for previews without data, `secs` apart.
fn sample_candles(secs: i64) -> Vec<Candle> {
    let now = chrono::Utc::now().timestamp() / secs * secs;
    let mut close = 2.0;
    (0..80)
        .map(|i| {
            let open = close;
            close = (open + ((i as f64) * 0.7).sin() * 0.06 + 0.012).max(0.5);
            let (hi, lo) = (open.max(close) + 0.03, open.min(close) - 0.03);
            let volume = 20_000 + ((i * 7919) % 50) as i64 * 2_000;
            Candle { start: now - (79 - i) * secs, open, high: hi, low: lo, close, volume }
        })
        .collect()
}

/// Blacklist preview (#3): the blacklisted tickers with price and change from the last list
/// (dashes for one not on it).
fn blacklisted_preview(f: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .engine
        .blacklist
        .iter()
        .map(|s| match app.snapshot.iter().find(|t| &t.symbol == s) {
            Some(t) => Line::styled(format!(" {s:<6} {:>9} {:>+8.1}%", fmt::price(t.price), t.change_ratio * 100.0), Style::new().fg(pal().text)),
            None => Line::styled(format!(" {s:<6} {:>9} {:>9}", "—", "—"), Style::new().fg(pal().faint)),
        })
        .collect();
    let area = Rect { width: area.width.min(40), ..area };
    let ttl = Line::from(vec![
        Span::styled(" Blacklisted ", Style::new().fg(pal().accent).bold()),
        Span::styled(format!("{} ", lines.len()), Style::new().fg(pal().dim)),
    ]);
    let empty = lines.is_empty();
    f.render_widget(Paragraph::new(lines).block(panel(ttl)), area);
    if empty {
        f.render_widget(Paragraph::new(Span::styled("none", Style::new().fg(pal().faint))), centered(area, 4, 1));
    }
}

fn setup_blacklist(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![
        Line::styled(" Blacklisted tickers are still tracked but listed separately, not ranked.", Style::new().fg(pal().dim)),
        Line::raw(""),
    ];
    if let Some(buf) = &app.bl_input {
        lines.push(Line::from(vec![
            Span::styled(" Toggle ticker: ", Style::new().fg(pal().accent)),
            Span::styled(format!("{buf}▏"), Style::new().fg(pal().text).bg(pal().input_bg)),
        ]));
        if let Some(e) = &app.bl_err {
            lines.push(Line::styled(format!(" {e}"), Style::new().fg(pal().red)));
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
                    Style::new().fg(pal().on_accent).bg(pal().accent).bold()
                } else if on {
                    Style::new().fg(pal().faint)
                } else {
                    Style::new().fg(pal().text)
                };
                Span::styled(txt, st)
            })
            .collect();
        lines.push(Line::from(spans));
    }
    if app.snapshot.is_empty() {
        lines.push(Line::styled("  No tickers available. Start the screener and wait for a refresh.", Style::new().fg(pal().faint)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title("TICKER BLACKLIST"))), area);
}

fn setup_filters(f: &mut Frame, app: &App, area: Rect) {
    let descs = [
        "Show every enabled alert type",
        "Ticker just appeared in the gainers list",
        "Volume ratio and price change both past their SETUP limits",
    ];
    let rect = Rect { width: area.width.min(FILTERS_W), height: area.height.min(FILTER_NAMES.len() as u16 + 6), ..area };
    let mut lines = vec![Line::raw("")];
    for (i, name) in FILTER_NAMES.iter().enumerate() {
        let on = app.cfg.filter_on(name);
        let badge = if on { Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().green)) } else { Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint)) };
        let name_st = if i == app.filter_cursor { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        lines.push(Line::from(vec![
            Span::raw(if i == app.filter_cursor { " ▶ " } else { "   " }),
            badge,
            Span::raw(" "),
            Span::styled(format!(" {name:<6}"), name_st),
            Span::styled(format!("  {}", descs[i]), Style::new().fg(pal().dim).italic()),
        ]));
    }
    // the dashboard's alert rows (same setting as Alert Rows in Configuration)
    let sel = app.filter_cursor == FILTER_NAMES.len();
    let label_st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
    let arrow_st = Style::new().fg(if sel { pal().accent } else { pal().faint });
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw(if sel { " ▶ " } else { "   " }),
        Span::styled(" Rows on dashboard ", label_st),
        Span::styled("  ◂ ", arrow_st),
        Span::styled(format!("{:>2}", app.cfg.alert_rows), Style::new().fg(pal().text).bold()),
        Span::styled(" ▸", arrow_st),
    ]));
    f.render_widget(Paragraph::new(lines).block(panel(title("ALERT FILTERS"))), rect);
}

/// Config -> Sound (#2): the Mute badge and the Volume slider (a bar of 20 steps).
fn setup_sound(f: &mut Frame, app: &App, area: Rect) {
    let rect = Rect { width: area.width.min(FILTERS_W), height: area.height.min(8), ..area };
    let label = |i: usize, name: &str| {
        let sel = i == app.sound_cursor;
        let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        [Span::raw(if sel { " ▶ " } else { "   " }), Span::styled(format!(" {name:<7}"), st), Span::raw("  ")]
    };
    let muted = app.cfg.sound_muted;
    let badge = if muted { Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().amber)) } else { Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint)) };
    let vol = app.cfg.sound_volume;
    let filled = usize::from(vol) / 5;
    let bar_col = if muted { pal().faint } else { pal().green };
    let arrow_st = Style::new().fg(if app.sound_cursor == 1 { pal().accent } else { pal().faint });
    let mut mute = label(0, "Mute").to_vec();
    mute.extend([badge, Span::styled("  Enter plays a test on the Volume row", Style::new().fg(pal().dim).italic())]);
    let mut volume = label(1, "Volume").to_vec();
    volume.extend([
        Span::styled("◂ ", arrow_st),
        Span::styled("█".repeat(filled), Style::new().fg(bar_col)),
        Span::styled("░".repeat(20 - filled), Style::new().fg(pal().faint)),
        Span::styled(" ▸", arrow_st),
        Span::styled(format!(" {vol:>3}%"), Style::new().fg(pal().text).bold()),
    ]);
    let lines = vec![Line::raw(""), Line::from(mute), Line::raw(""), Line::from(volume)];
    f.render_widget(Paragraph::new(lines).block(panel(title("SOUND"))), rect);
}

/// Config -> Time & Format (#2): one row per setting, choices between arrows, the typed rows
/// with a cursor; the focused row's help and, when a value cannot be used, why.
fn setup_format(f: &mut Frame, app: &App, area: Rect, active: bool) {
    let d = &app.fmt_draft;
    let mut lines = vec![Line::raw("")];
    for (i, (label, _)) in fmt::ROWS.iter().enumerate() {
        let focused = active && i == app.fmt_cursor;
        let label_st = if focused { Style::new().fg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        let arrow_st = Style::new().fg(if focused { pal().accent } else { pal().faint });
        let val_st = if focused { Style::new().fg(pal().text).bg(pal().input_bg) } else { Style::new().fg(pal().text) };
        let mut v = d.row_text(i);
        if focused && i < fmt::TYPED_ROWS {
            v.push('▏');
        }
        lines.push(Line::from(vec![
            Span::styled(format!("  {} {label:<16}", if focused { "▶" } else { " " }), label_st),
            Span::styled("◂ ", arrow_st),
            Span::styled(format!("{v:<34}"), val_st),
            Span::styled(" ▸", arrow_st),
        ]));
    }
    if let Some(p) = d.problem() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("  {p}"), Style::new().fg(pal().red).bold()));
    }
    let [top, tip] = Layout::vertical([Constraint::Min(0), Constraint::Length(5)]).areas(area);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title("TIME & FORMAT"))), top);
    f.render_widget(
        Paragraph::new(fmt::ROWS[app.fmt_cursor].1)
            .style(Style::new().fg(pal().tip))
            .wrap(Wrap { trim: true })
            .block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(pal().faint))),
        tip,
    );
}

/// Time & Format preview (#3): a sample alert row, chart time-axis labels, a level tag, prices
/// and volumes, all in the formats being edited (`ui::draw` set them).
fn format_preview(f: &mut Frame, area: Rect) {
    let area = Rect { width: area.width.min(70), ..area };
    let now = chrono::Utc::now().timestamp();
    let alerts = sample_alerts(1);
    let [table, rest] = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);
    draw_alerts(f, table, &alerts.iter().collect::<Vec<_>>(), None, "ALL");
    let row = |name: &str, v: String| {
        Line::from(vec![Span::styled(format!("  {name:<16}"), Style::new().fg(pal().dim)), Span::styled(v, Style::new().fg(pal().text))])
    };
    let mins: Vec<String> = (0..4).map(|i| fmt::time(now / 300 * 300 - (3 - i) * 1800)).collect();
    let days: Vec<String> = (0..4).map(|i| fmt::day(now - (3 - i) * 86_400)).collect();
    let lines = vec![
        Line::raw(""),
        row("Chart 1m-1h", mins.join("   ")),
        row("Chart 4h, D", days.join("   ")),
        row("Level tag", format!("{} ({})   {} ({})", fmt::date(now - 86_400), fmt::money(3.86), fmt::date(now - 7 * 86_400), fmt::money(0.4521))),
        row("Price", format!("{}   {}   {}", fmt::price(1234.5), fmt::price(3.86), fmt::price(0.4521))),
        row("Volume", format!("{}   {}", fmt::count(18_400_000.0), fmt::count(950_000.0))),
        row("Market cap", fmt::count(2_450_000_000.0)),
        row("Date and time", format!("{} {}", fmt::date(now), fmt::time_s(now))),
    ];
    f.render_widget(Paragraph::new(lines).block(panel(title("PREVIEW"))), rest);
}

/// Alert Filters preview (#3): the dashboard alerts panel as the filters and rows set it, from the
/// recent alerts, else from sample alerts of each kind.
fn alerts_preview(f: &mut Frame, app: &App, area: Rect) {
    use crate::engine::Alert;
    let visible = |a: &&Alert| app.cfg.filter_on(a.kind.label());
    let rows = app.cfg.alert_rows as usize;
    let real: Vec<&Alert> = app.engine.alerts().iter().rev().filter(visible).take(rows).collect();
    let samples: Vec<Alert>;
    let (shown, note) = if app.engine.alerts().is_empty() {
        samples = sample_alerts(rows * 4);
        (samples.iter().filter(visible).take(rows).collect(), "sample alerts (none fired yet)")
    } else {
        (real, "recent alerts")
    };
    let [table, caption] = Layout::vertical([Constraint::Length(app.cfg.alert_rows + 3), Constraint::Length(1)]).areas(area);
    draw_alerts(f, table, &shown, None, "ALL");
    f.render_widget(Paragraph::new(Span::styled(format!(" Preview: {note}, {rows} rows"), Style::new().fg(pal().faint))), caption);
}

/// `n` made-up alerts cycling through the kinds, newest first.
fn sample_alerts(n: usize) -> Vec<crate::engine::Alert> {
    let syms = ["SMPL", "DEMO", "TEST", "ABCD", "WXYZ"];
    let kinds = [AlertKind::Setup, AlertKind::New, AlertKind::Pump, AlertKind::Spike];
    (0..n)
        .map(|i| {
            let kind = kinds[i % kinds.len()];
            let now = chrono::Utc::now().timestamp();
            crate::engine::Alert {
                at: std::time::Instant::now(),
                epoch: now - i as i64 * 37,
                symbol: syms[i % syms.len()].to_string(),
                kind,
                prev_vol: 400_000,
                curr_vol: 400_000 + (i as i64 % 5 + 2) * 300_000,
                prev_price: 2.0,
                curr_price: 2.0 + (i % 4) as f64 * 0.12,
                change_ratio: 0.18 + i as f64 * 0.013,
            }
        })
        .collect()
}

fn columns(f: &mut Frame, app: &App) {
    let rect = centered(f.area(), 56, COLUMNS.len() as u16 + 6);
    let mut lines = vec![Line::raw("")];
    for (i, c) in COLUMNS.iter().enumerate() {
        let on = app.cfg.column_visible(c.key);
        let badge = if on { Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().green)) } else { Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint)) };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
        let mk = |i: u64| Ticker { symbol: format!("S{i}"), ticker_id: i, price: 2.0 + i as f64, volume: 1000 * i as i64, change_ratio: 0.1, market_value: 1e6, ..Default::default() };
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
        let rows = preview.iter().filter(|l| l.contains("SMPL") || l.contains("DEMO") || l.contains("TEST") || l.contains("ABCD") || l.contains("WXYZ")).count();
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
}
