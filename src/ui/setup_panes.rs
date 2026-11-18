//! The other Config panes (#2) and previews (#3): Blacklist, Alert Filters, Sound, Time &
//! Format, Storage and Health Check.

use super::samples::sample_alerts;
use super::setup::FILTERS_W;
use super::tables::draw_alerts;
use super::{centered, panel, title};
use crate::app::{App, BL_COLS};
use crate::config::FILTER_NAMES;
use crate::fmt;
use crate::theme::pal;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;

/// Blacklist preview (#3): the blacklisted tickers with price and change from the last list
/// (dashes for one not on it).
pub(super) fn blacklisted_preview(f: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .engine
        .blacklist
        .iter()
        .map(|s| match app.snapshot.iter().find(|t| &t.symbol == s) {
            Some(t) => {
                Line::styled(format!(" {s:<6} {:>9} {:>+8.1}%", fmt::price(t.price), t.change_ratio * 100.0), Style::new().fg(pal().text))
            }
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

pub(super) fn setup_blacklist(f: &mut Frame, app: &App, area: Rect) {
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

pub(super) fn setup_filters(f: &mut Frame, app: &App, area: Rect) {
    let descs = [
        "Show every enabled alert type",
        "Ticker just appeared in the gainers list",
        "Volume ratio and price change both past their SETUP limits",
    ];
    let rect = Rect { width: area.width.min(FILTERS_W), height: area.height.min(FILTER_NAMES.len() as u16 + 6), ..area };
    let mut lines = vec![Line::raw("")];
    for (i, name) in FILTER_NAMES.iter().enumerate() {
        let on = app.cfg.filter_on(name);
        let badge = if on {
            Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().green))
        } else {
            Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint))
        };
        let name_st =
            if i == app.filter_cursor { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
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
pub(super) fn setup_sound(f: &mut Frame, app: &App, area: Rect) {
    let rect = Rect { width: area.width.min(FILTERS_W), height: area.height.min(8), ..area };
    let label = |i: usize, name: &str| {
        let sel = i == app.sound_cursor;
        let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        [Span::raw(if sel { " ▶ " } else { "   " }), Span::styled(format!(" {name:<7}"), st), Span::raw("  ")]
    };
    let muted = app.cfg.sound_muted;
    let badge = if muted {
        Span::styled(" ON  ", Style::new().fg(pal().on_accent).bg(pal().amber))
    } else {
        Span::styled(" OFF ", Style::new().fg(pal().text).bg(pal().faint))
    };
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
pub(super) fn setup_format(f: &mut Frame, app: &App, area: Rect, active: bool) {
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
pub(super) fn format_preview(f: &mut Frame, area: Rect) {
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
        row(
            "Level tag",
            format!("{} ({})   {} ({})", fmt::date(now - 86_400), fmt::money(3.86), fmt::date(now - 7 * 86_400), fmt::money(0.4521)),
        ),
        row("Price", format!("{}   {}   {}", fmt::price(1234.5), fmt::price(3.86), fmt::price(0.4521))),
        row("Volume", format!("{}   {}", fmt::count(18_400_000.0), fmt::count(950_000.0))),
        row("Market cap", fmt::count(2_450_000_000.0)),
        row("Date and time", format!("{} {}", fmt::date(now), fmt::time_s(now))),
    ];
    f.render_widget(Paragraph::new(lines).block(panel(title("PREVIEW"))), rest);
}

/// Alert Filters preview (#3): the dashboard alerts panel as the filters and rows set it, from the
/// recent alerts, else from sample alerts of each kind.
pub(super) fn alerts_preview(f: &mut Frame, app: &App, area: Rect) {
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

/// Config -> Storage: the app home as a file list with sizes; d deletes (after y).
pub(super) fn storage_view(f: &mut Frame, app: &App, body: Rect) {
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
            if i % 2 == 1 {
                row.style(Style::new().bg(pal().zebra))
            } else {
                row
            }
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
            format!(
                " Delete {}{} ({})? y = yes, any other key = no",
                e.name,
                if e.dir { "/ and everything in it" } else { "" },
                crate::storage::human(e.bytes)
            ),
            Style::new().fg(pal().amber).bold(),
        ),
        (_, _, Some(m)) => Line::styled(format!(" {m}"), Style::new().fg(pal().dim)),
        _ => Line::styled(format!(" {}", b.cwd.display()), Style::new().fg(pal().faint)),
    };
    f.render_widget(Paragraph::new(line).block(Block::new().borders(Borders::ALL).border_style(Style::new().fg(pal().faint))), status);
}

/// Health Check (#2): the connection checks, then the System group; one name column padded to
/// the longest name, messages aligned after it, a blank line between the groups.
pub(super) fn health(f: &mut Frame, app: &App, area: Rect) {
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
pub(super) fn system_rows(app: &App) -> Vec<(&'static str, String, Color)> {
    use crate::sysmem;
    let gb = |b: u64| format!("{:.1} GB", b as f64 / 1_073_741_824.0);
    let level = |pct: f64| {
        if pct >= 90.0 {
            pal().red
        } else if pct >= 75.0 {
            pal().amber
        } else {
            pal().text
        }
    };
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
        ("Uptime", (fmt::uptime(app.started_at.elapsed()), pal().text)),
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
