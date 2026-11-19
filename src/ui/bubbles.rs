//! The Bubbles layout: the listed tickers as bubbles sized by their volume over the timeframe.

use super::dashboard::tf_title;
use super::panel;
use super::volume::{volume_panel, VOL_H};
use crate::app::App;
use crate::bubbles::{self, Bubble, Item, Painted, SHADES};
use crate::fmt;
use crate::history::Tf;
use crate::theme::{mix, pal};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Most bubbles shown (the largest volumes).
const MAX_BUBBLES: usize = 40;
/// Fewest rows the bubbles take.
const BUBBLES_MIN_H: u16 = 10;

/// The bubbles over the volume chart: the chart keeps its `VOL_H` rows, the bubbles take the
/// rest when there are at least `BUBBLES_MIN_H`; a smaller area shows the chart only.
pub(super) fn bubbles_and_volume(f: &mut Frame, app: &mut App, area: Rect) {
    if area.height < VOL_H + BUBBLES_MIN_H {
        volume_panel(f, app, area);
        return;
    }
    let [bubbles, volume] = Layout::vertical([Constraint::Min(0), Constraint::Length(VOL_H)]).areas(area);
    bubbles_panel(f, app, bubbles);
    volume_panel(f, app, volume);
}

/// Bordered panel with the timeframe tabs; each listed ticker's volume over the timeframe (the
/// last 1m/5m/1h/4h of its 1m candles, the day's volume on D) as a bubble labelled with the
/// ticker, that volume and the price. The window app paints the bubbles itself (`App::bubbles`); the
/// terminal colours the cells inside them.
pub(super) fn bubbles_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(tf_title(app, area, " BUBBLES "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let tf = app.timeframe;
    let now = chrono::Utc::now().timestamp();
    let mut items: Vec<Item> = app
        .tickers
        .iter()
        .map(|t| {
            let delta = match tf {
                Tf::D1 => t.volume,
                _ => app.engine.candles(&t.symbol).map_or(0, |c| bubbles::delta(c, tf.secs(), now)),
            };
            let label = fmt::count(delta as f64).chars().count().max(fmt::price(t.price).chars().count());
            // three label rows (six units) and the widest line
            let min_r = ((t.symbol.chars().count().max(label) + 2) as f32 / 2.0).max(3.5);
            Item { symbol: t.symbol.clone(), delta, min_r }
        })
        .filter(|i| i.delta > 0)
        .collect();
    items.sort_by(|a, b| b.delta.cmp(&a.delta));
    items.truncate(MAX_BUBBLES);
    // the bubbles move on by the time since the last frame (kept still in between)
    let t = std::time::Instant::now();
    let dt = app.bubbles_at.map_or(0.0, |at| t.saturating_duration_since(at).as_secs_f32().min(0.5));
    app.bubbles_at = Some(t);
    let placed = app.bubble_field.step(&items, f32::from(inner.width), f32::from(inner.height) * 2.0, dt).to_vec();
    if placed.is_empty() {
        let note = format!("No volume in the last {}", tf.label());
        f.render_widget(Paragraph::new(Span::styled(note, Style::new().fg(pal().faint))), inner);
        return;
    }
    let fill = |b: &Bubble| mix(pal().accent, pal().bg, SHADES[b.shade]);
    let price = |sym: &str| app.tickers.iter().find(|t| t.symbol == sym).map_or(0.0, |t| t.price);
    let lines = |b: &Bubble| [b.symbol.clone(), fmt::count(b.delta as f64), fmt::price(price(&b.symbol))];
    if app.pixel_chart {
        app.bubbles = placed
            .iter()
            .map(|b| Painted {
                cx: f32::from(inner.x) + b.cx,
                cy: f32::from(inner.y) + b.cy / 2.0,
                r: b.r,
                fill: fill(b),
                text: pal().chart_text,
                lines: lines(b),
            })
            .collect();
        return;
    }
    // terminal: cells whose centre is inside a bubble take its colour, the label on its centre
    let buf = f.buffer_mut();
    for y in 0..inner.height {
        for x in 0..inner.width {
            let (ux, uy) = (f32::from(x) + 0.5, (f32::from(y) + 0.5) * 2.0);
            if let Some(b) = placed.iter().find(|b| (b.cx - ux).hypot(b.cy - uy) <= b.r) {
                buf[(inner.x + x, inner.y + y)].set_bg(fill(b));
            }
        }
    }
    for b in &placed {
        let top = (b.cy / 2.0) as u16 - 1;
        for (i, s) in lines(b).iter().enumerate() {
            let x = (b.cx - s.chars().count() as f32 / 2.0).round().max(0.0) as u16;
            let style = Style::new().fg(pal().chart_text).bg(fill(b));
            buf.set_string(inner.x + x, inner.y + top + i as u16, s, style);
        }
    }
}
