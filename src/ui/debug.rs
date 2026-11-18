//! Trace mode's debug overlay over the dashboard: the flow table (how many messages each step
//! of the data path handled, how long ago the last one was, and what it said) over the newest
//! log lines.

use super::{keyhint, panel, title};
use crate::app::App;
use crate::theme::pal;
use crate::trace::Snapshot;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::time::Duration;

/// Category column width.
const CAT_W: usize = 17;

/// "0.4s", "12s", "3m", "2h": how long ago a category last saw a message.
pub(super) fn age(d: Duration) -> String {
    match d.as_secs() {
        0..=9 => format!("{:.1}s", d.as_secs_f64()),
        s @ 10..=119 => format!("{s}s"),
        s @ 120..=7199 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

pub(super) fn overlay(f: &mut Frame, app: &App, snap: &Snapshot) {
    let area = f.area();
    // a margin round the box where there is room, so the dashboard still shows behind it
    let mx = if area.width >= 60 { area.width / 20 } else { 0 };
    let my = if area.height >= 20 { 1 } else { 0 };
    let rect = Rect::new(area.x + mx, area.y + my, area.width - 2 * mx, area.height - 2 * my);
    let block = panel(title("DEBUG  how data travels"));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    // the flow table takes what it needs, up to half; the log the rest, then the key hints
    let flows_h = (snap.flows.len() as u16 + 1).min(inner.height / 2);
    let [flows, log, hints] = Layout::vertical([Constraint::Length(flows_h), Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let dim = Style::new().fg(pal().dim);
    let mut rows = vec![Line::styled(format!(" {:<CAT_W$} {:>8} {:>6}  last", "category", "count", "age"), dim.bold())];
    for fl in &snap.flows {
        // recent activity stands out; quiet categories fade
        let st = if fl.age < Duration::from_secs(2) { Style::new().fg(pal().green) } else { Style::new().fg(pal().text) };
        rows.push(Line::from(vec![
            Span::styled(format!(" {:<CAT_W$}", fl.cat), st.bold()),
            Span::styled(format!(" {:>8} {:>6}  ", fl.count, age(fl.age)), dim),
            Span::styled(fl.summary.clone(), Style::new().fg(pal().text)),
        ]));
    }
    if snap.flows.is_empty() {
        rows.push(Line::styled(" nothing traced yet", Style::new().fg(pal().faint)));
    }
    f.render_widget(Paragraph::new(rows), flows);

    // the newest lines that fit, `scroll` lines back from the end
    let h = log.height.saturating_sub(1) as usize;
    let end = snap.lines.len().saturating_sub(app.debug.scroll.min(snap.lines.len().saturating_sub(h)));
    let start = end.saturating_sub(h);
    let head = format!(" log: lines {}-{} of {} (newest last)", start + usize::from(end > start), end, snap.lines.len());
    let mut lines = vec![Line::styled(head, dim.bold())];
    lines.extend(snap.lines[start..end].iter().map(|l| Line::styled(format!(" {l}"), Style::new().fg(pal().text))));
    f.render_widget(Paragraph::new(lines), log);

    let mut keys = keyhint("F12/`", "close");
    keys.extend(keyhint("↑↓ pgup/pgdn", "scroll"));
    keys.extend(keyhint("end", "newest"));
    f.render_widget(Paragraph::new(Line::from(keys)), hints);
}
