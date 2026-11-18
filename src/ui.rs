//! Drawing: `draw` picks the screen for `App::view`; the screens live in the child modules, the
//! small widgets they share are here.

mod boot;
mod dashboard;
mod debug;
mod samples;
mod setup;
mod setup_panes;
mod tables;
#[cfg(test)]
mod tests;

use crate::app::{App, Hits, View};
use crate::engine::AlertKind;
use crate::fmt;
use crate::theme::{self, pal};
use boot::boot;
use crossterm::event::KeyCode;
use dashboard::{chart_view, columns, dashboard};
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use setup::setup_shell;

fn kind_style(kind: AlertKind) -> Style {
    match kind {
        AlertKind::New => Style::new().fg(pal().on_accent).bg(pal().new_bg),
        AlertKind::Pump => Style::new().fg(pal().on_accent).bg(pal().pump_bg),
        AlertKind::Spike => Style::new().fg(pal().on_accent).bg(pal().spike_bg),
        AlertKind::Setup => Style::new().fg(pal().on_setup).bg(pal().setup_bg).bold(),
    }
}

fn panel(title: Line<'static>) -> Block<'static> {
    Block::new().borders(Borders::ALL).border_type(BorderType::Plain).border_style(Style::new().fg(pal().border)).title(title)
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
    if app.view == View::Dashboard && app.debug.open {
        debug::overlay(f, app, &crate::trace::snapshot().unwrap_or_default());
        // the window app would paint the chart's candles over the overlay, and the dashboard
        // under it takes no clicks
        app.plot = None;
        app.halftones.clear();
        app.hits = Hits::default();
    }
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
