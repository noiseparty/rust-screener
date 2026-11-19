//! The user's own dashboard tabs (Config -> Layouts): each slot of the template draws its panel.

use super::bubbles::bubbles_panel;
use super::dashboard::{blacklist_box, chart_panel};
use super::tables::{alerts_table, gainers_table};
use super::volume::volume_panel;
use crate::app::{layout_rows, App, LayoutRow};
use crate::layouts::{CustomLayout, Panel, Template};
use crate::theme::pal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};
use ratatui::Frame;

/// The custom tab `layout` in `area`.
pub(super) fn custom(f: &mut Frame, app: &mut App, area: Rect, layout: &CustomLayout) {
    for (r, &p) in layout.template.areas(area, &layout.splits).iter().zip(&layout.slots) {
        match p {
            Panel::Empty => {}
            Panel::List => gainers_table(f, app, *r),
            Panel::Alerts => alerts_table(f, app, *r),
            Panel::Chart => {
                let sym = app.charted_symbol().unwrap_or("").to_string();
                chart_panel(f, app, *r, &sym);
            }
            Panel::Bubbles => bubbles_panel(f, app, *r),
            Panel::Volume => volume_panel(f, app, *r),
            Panel::Blacklist => blacklist_box(f, app, *r),
        }
    }
}

/// `layout` as boxes named after their panels; the slot `active` (being edited) highlighted.
pub(super) fn preview(buf: &mut Buffer, area: Rect, layout: &CustomLayout, active: Option<usize>) {
    for (i, (r, p)) in layout.template.areas(area, &layout.splits).iter().zip(&layout.slots).enumerate() {
        let on = active == Some(i);
        let col = if on { pal().accent } else { pal().border };
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Plain).border_style(Style::new().fg(col));
        let inner = block.inner(*r);
        block.render(*r, buf);
        if inner.height == 0 {
            continue;
        }
        let name = format!("{} {}", (b'A' + i as u8) as char, p.label());
        let st = if on { Style::new().fg(pal().accent).bold() } else { Style::new().fg(pal().dim) };
        let mid = Rect { y: inner.y + inner.height.saturating_sub(1) / 2, height: 1, ..inner };
        Paragraph::new(name).style(st).alignment(Alignment::Center).render(mid, buf);
    }
}

/// Width of the Layouts pane.
pub(super) const LAYOUTS_W: u16 = 46;

/// Config -> Layouts (#2): one row per setting of the custom tab being edited, choices
/// between arrows, the name with a cursor while its row is focused.
pub(super) fn setup_layouts(f: &mut Frame, app: &App, area: Rect, active: bool) {
    let l = app.layout_draft();
    let rows = layout_rows(l);
    let cursor = app.layouts_cursor.min(rows.len() - 1);
    let mut lines = vec![Line::raw("")];
    for (i, &row) in rows.iter().enumerate() {
        let sel = active && i == cursor;
        let st = if sel { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text) };
        let arrow = Style::new().fg(if sel { pal().accent } else { pal().faint });
        let (name, value) = match row {
            LayoutRow::Tab => ("Tab".to_string(), format!("Custom {}", app.layouts_tab + 1)),
            LayoutRow::Use => ("Show".to_string(), if l.on { "On" } else { "Off" }.to_string()),
            LayoutRow::Name => ("Name".to_string(), String::new()),
            LayoutRow::Template => ("Template".to_string(), l.template.label().to_string()),
            LayoutRow::Slot(s) => (format!("Slot {}", (b'A' + s as u8) as char), l.slots[s].label().to_string()),
            LayoutRow::Split(s) => (format!("Split {}", s + 1), format!("{}% {}", l.splits[s], l.template.split_label(s))),
        };
        let mut spans = vec![Span::raw(if sel { " ▶ " } else { "   " }), Span::styled(format!(" {name:<9}"), st), Span::raw("  ")];
        if row == LayoutRow::Name {
            let shown = if l.name.is_empty() && !sel { format!("Custom {}", app.layouts_tab + 1) } else { l.name.clone() };
            spans.push(Span::styled(shown, Style::new().fg(if l.name.is_empty() { pal().faint } else { pal().text })));
            if sel {
                spans.push(Span::styled("▏", Style::new().fg(pal().accent)));
            }
        } else {
            spans.extend([Span::styled("◂ ", arrow), Span::styled(value, Style::new().fg(pal().text).bold()), Span::styled(" ▸", arrow)]);
        }
        lines.push(Line::from(spans));
        // a gap after the tab rows and after the template
        if matches!(row, LayoutRow::Name | LayoutRow::Template) {
            lines.push(Line::raw(""));
        }
    }
    let rect = Rect { width: area.width.min(LAYOUTS_W), height: area.height.min(lines.len() as u16 + 2), ..area };
    f.render_widget(Paragraph::new(lines).block(super::panel(super::title("LAYOUTS"))), rect);
}

/// The custom tab being edited, as boxes; the focused slot highlighted.
pub(super) fn layouts_preview(f: &mut Frame, app: &App, area: Rect) {
    let block = super::panel(super::title("PREVIEW"));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = layout_rows(app.layout_draft());
    let active = match rows.get(app.layouts_cursor) {
        Some(LayoutRow::Slot(s)) if app.view == crate::app::View::SetupLayouts => Some(*s),
        _ => None,
    };
    preview(f.buffer_mut(), inner, app.layout_draft(), active);
}

/// The on-screen layout editor: a toolbar (templates, widgets), the tab full size with live data,
/// a box on each slot (dashed and named while empty, lit while a widget is carried over it),
/// grips on the borders, and the carried widget under the pointer.
pub(super) fn layout_edit(f: &mut Frame, app: &mut App) {
    let [bar1, bar2, main, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let l = app.layout_draft().clone();
    custom(f, app, main, &l);
    // the panels' own click areas do not act here
    app.hits = Default::default();

    // toolbar: the tab and the templates
    let name = if l.name.is_empty() { format!("Custom {}", app.layouts_tab + 1) } else { l.name.clone() };
    let buf = f.buffer_mut();
    let mut x = chip(buf, bar1, bar1.x, &format!(" EDIT {name} "), Style::new().fg(pal().on_accent).bg(pal().accent).bold()) + 1;
    x = chip(buf, bar1, x, "Template:", Style::new().fg(pal().dim)) + 1;
    for t in Template::ALL {
        let st = if t == l.template { Style::new().fg(pal().on_accent).bg(pal().accent).bold() } else { Style::new().fg(pal().text).bg(pal().zebra) };
        let at = x;
        x = chip(buf, bar1, x, &format!(" {} ", t.label()), st) + 1;
        app.hits.edit_templates.push((Rect::new(at, bar1.y, x - 1 - at, 1).intersection(bar1), t));
    }
    // widgets: the ones already placed are dimmed (dragging one moves it)
    let mut x = chip(buf, bar2, bar2.x, " Widgets:", Style::new().fg(pal().dim)) + 1;
    for p in Panel::ALL.into_iter().skip(1) {
        let st = if l.contains(p) { Style::new().fg(pal().dim).bg(pal().zebra) } else { Style::new().fg(pal().on_accent).bg(pal().green).bold() };
        let at = x;
        x = chip(buf, bar2, x, &format!(" {} ", p.label()), st) + 1;
        app.hits.edit_widgets.push((Rect::new(at, bar2.y, x - 1 - at, 1).intersection(bar2), p));
    }
    chip(buf, bar2, x + 1, "drag onto a slot · drag borders to resize · right click empties", Style::new().fg(pal().faint));

    // slots
    let carried = app.carried_widget();
    let over = |r: &Rect| app.drag_pos.is_some_and(|p| r.contains(p));
    for (i, (r, &p)) in l.template.areas(main, &l.splits).iter().zip(&l.slots).enumerate() {
        app.hits.edit_slots.push((*r, i));
        let lit = carried.is_some() && over(r);
        if p == Panel::Empty || lit {
            let col = if lit { pal().accent } else { pal().faint };
            let block = Block::new().borders(Borders::ALL).border_type(if lit { BorderType::Thick } else { BorderType::Plain }).border_style(Style::new().fg(col));
            let inner = block.inner(*r);
            block.render(*r, buf);
            if p == Panel::Empty && inner.height > 0 {
                let mid = Rect { y: inner.y + inner.height.saturating_sub(1) / 2, height: 1, ..inner };
                let text = format!("Slot {}: drop a widget here", (b'A' + i as u8) as char);
                Paragraph::new(text).style(Style::new().fg(col)).alignment(Alignment::Center).render(mid, buf);
            }
        }
    }
    // border grips
    for (i, parent, cols) in l.template.handles(main, &l.splits) {
        let s = u32::from(l.splits[i]);
        let r = if cols {
            let x = parent.x + (u32::from(parent.width) * s / 100) as u16;
            put_grip(buf, x.saturating_sub(1), parent.y + parent.height / 2, "◀▶");
            Rect::new(x.saturating_sub(1), parent.y, 2, parent.height)
        } else {
            let y = parent.y + (u32::from(parent.height) * s / 100) as u16;
            put_grip(buf, parent.x + parent.width / 2, y.saturating_sub(1), "▲");
            put_grip(buf, parent.x + parent.width / 2, y, "▼");
            Rect::new(parent.x, y.saturating_sub(1), parent.width, 2)
        };
        app.hits.edit_handles.push((r.intersection(main), i, parent, cols));
    }
    // the carried widget under the pointer
    if let (Some(p), Some(pos)) = (carried, app.drag_pos) {
        let area = f.area();
        chip(f.buffer_mut(), area, pos.x, &format!(" {} ", p.label()), Style::new().fg(pal().on_accent).bg(pal().accent).bold());
    }
    // the window app would paint candles and bubbles over the editor's marks; the live cells stay
    super::hint_bar(f, app, footer, &[("enter", "save", Some(crossterm::event::KeyCode::Enter)), ("esc", "cancel", Some(crossterm::event::KeyCode::Esc))]);
}

/// `s` at `x` on the row of `area` (clipped); returns the column after it.
fn chip(buf: &mut Buffer, area: Rect, x: u16, s: &str, st: Style) -> u16 {
    let y = area.y;
    if x >= area.right() {
        return x;
    }
    let (end, _) = buf.set_stringn(x, y, s, usize::from(area.right() - x), st);
    end.max(x)
}

/// A border grip in the accent colour.
fn put_grip(buf: &mut Buffer, x: u16, y: u16, s: &str) {
    if buf.area.contains(ratatui::layout::Position::new(x, y)) {
        buf.set_string(x, y, s, Style::new().fg(pal().accent).bold());
    }
}
