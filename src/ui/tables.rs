//! The gainers table and the alerts table (also drawn by the Config previews).

use super::{centered, kind_style, panel};
use crate::app::{App, COLUMNS};
use crate::fmt;
use crate::scraper::{Mov, Ticker};
use crate::theme::pal;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use std::time::Duration;

pub(super) fn fit_columns(app: &mut App, width: u16) {
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
pub(super) fn count_or_na(v: f64) -> String {
    if v == 0.0 {
        "N/A".into()
    } else {
        fmt::count(v)
    }
}

pub(super) fn cell(t: &Ticker, key: &str) -> Cell<'static> {
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
        "status" => match t.status {
            Some(k) => Cell::from(Span::styled(format!("{:^7}", k.label()), kind_style(k))),
            None => Cell::from(""),
        },
        _ => Cell::from(""),
    }
}

/// Column header for the Vol/m window: 30s, 1m, 3m, 90s.
fn vol_m_header(secs: u64) -> String {
    if secs >= 60 && secs % 60 == 0 {
        format!("Vol/m {}m", secs / 60)
    } else {
        format!("Vol/m {secs}s")
    }
}

/// Width the gainers panel needs for its enabled columns (borders, cursor column, spacing).
pub(super) fn gainers_width(app: &App) -> u16 {
    let cols: u16 = COLUMNS.iter().filter(|c| app.cfg.column_visible(c.key)).map(|c| c.width + 1).sum();
    // borders 2, cursor column 1, columns with one space between: sum(width + 1) - 1 + 3
    cols + 2
}

pub(super) fn gainers_table(f: &mut Frame, app: &mut App, area: Rect) {
    fit_columns(app, area.width.saturating_sub(2));
    let vol_m_label = vol_m_header(app.cfg.vol_m_window_seconds);

    let header = Row::new(app.visible_cols.iter().enumerate().map(|(vi, &ci)| {
        let col = &COLUMNS[ci];
        let mut label = if col.key == "volrate" { vol_m_label.clone() } else { col.title.to_string() };
        if app.sort_key == col.key {
            label.push(if app.sort_desc { '▼' } else { '▲' });
        }
        let st =
            if vi == app.sel_col { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().dim).bold() };
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

pub(super) fn alerts_table(f: &mut Frame, app: &mut App, area: Rect) {
    let rows_fit = area.height.saturating_sub(3) as usize;
    let flash_window = Duration::from_millis(1500);
    let fresh = app.last_alert_at.is_some_and(|t| t.elapsed() < flash_window);
    // scrolled no further than the last page
    let total = app.engine.alerts().iter().filter(|a| app.alert_visible(a.kind)).count();
    app.alert_scroll = app.alert_scroll.min(total.saturating_sub(rows_fit));
    let skip = app.alert_scroll;

    let syms: Vec<String> = app
        .engine
        .alerts()
        .iter()
        .rev()
        .filter(|a| app.alert_visible(a.kind))
        .skip(skip)
        .take(rows_fit)
        .map(|a| a.symbol.clone())
        .collect();
    app.hits.alert_rows = Rect::new(area.x + 1, area.y + 2, area.width.saturating_sub(2), syms.len() as u16).intersection(area);
    app.hits.alert_syms = syms;
    let app = &*app;
    let visible: Vec<_> = app.engine.alerts().iter().rev().filter(|a| app.alert_visible(a.kind)).skip(skip).take(rows_fit).collect();
    let flash = app.last_alert_at.filter(|_| fresh);
    draw_alerts(f, area, &visible, flash, app.filter_name());
}

/// The alerts panel with `visible` (newest first); the newest row flashes when its alert is the
/// one fired at `flash`.
pub(super) fn draw_alerts(f: &mut Frame, area: Rect, visible: &[&crate::engine::Alert], flash: Option<std::time::Instant>, filter: &str) {
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
        f.render_widget(Paragraph::new(Span::styled(msg.clone(), Style::new().fg(pal().faint))), centered(area, msg.len() as u16, 1));
    }
}
