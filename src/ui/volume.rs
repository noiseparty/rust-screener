//! Live volume chart: the listed tickers' total volume per minute as a braille line scrolling
//! left, shaded below, with a count axis, time labels and the live minute's value as a tag.

use super::panel;
use crate::app::App;
use crate::chart::{put, text};
use crate::fmt;
use crate::theme::{mix, pal};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

/// Rows the chart takes, borders included.
pub(super) const VOL_H: u16 = 10;

/// Braille dot bits, by dot column (0, 1) and dot row (0-3, top down).
const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// The listed tickers' summed volume in each of the `n` minutes ending with the one `now` is
/// in, oldest first; 0 where there is none.
pub(super) fn minute_totals(app: &App, n: usize, now: i64) -> Vec<i64> {
    let cur = now - now.rem_euclid(60);
    let first = cur - (n as i64 - 1) * 60;
    let mut out = vec![0; n];
    for t in &app.tickers {
        let Some(candles) = app.engine.candles(&t.symbol) else { continue };
        for c in candles.iter().rev().take_while(|c| c.start >= first) {
            if c.start <= cur {
                out[((c.start - first) / 60) as usize] += c.volume;
            }
        }
    }
    out
}

/// Bordered panel with the max and average of the shown minutes in the title.
pub(super) fn volume_panel(f: &mut Frame, app: &App, area: Rect) {
    let now = chrono::Utc::now().timestamp();
    // the plot's dot columns, as `draw` lays them out
    let inner_w = area.width.saturating_sub(2);
    let n = usize::from(plot_cols(inner_w)) * 2;
    let totals = minute_totals(app, n.max(1), now);
    let max = totals.iter().copied().max().unwrap_or(0);
    let avg = totals.iter().sum::<i64>() / totals.len().max(1) as i64;
    let title = Line::from(vec![
        Span::styled(" VOLUME ", Style::new().fg(pal().accent).bold()),
        Span::styled("1m ", Style::new().fg(pal().dim)),
        Span::styled(format!(" max {}  avg {} ", fmt::count(max as f64), fmt::count(avg as f64)), Style::new().fg(pal().dim)),
    ]);
    let block = panel(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    draw(f.buffer_mut(), inner, &totals, now);
}

/// Width of the count labels left of the axis.
const LABEL_W: u16 = 7;
/// Width kept right of the plot for the live tag.
const TAG_W: u16 = 9;

/// Cell columns of the plot in an inner area `w` wide.
fn plot_cols(w: u16) -> u16 {
    w.saturating_sub(LABEL_W + 1 + TAG_W)
}

/// The chart of `totals` (one per braille dot column, newest at the right edge; the last is
/// the minute `now` is in) in `area`.
pub(super) fn draw(buf: &mut Buffer, area: Rect, totals: &[i64], now: i64) {
    let (w, h) = (area.width, area.height);
    let cols = plot_cols(w);
    if cols < 4 || h < 4 {
        return;
    }
    let max = totals.iter().copied().max().unwrap_or(0);
    if max <= 0 {
        text(buf, area, 1, 0, "No volume yet", pal().faint);
        return;
    }
    let plot_h = i32::from(h) - 2;
    let x0 = i32::from(LABEL_W) + 1;
    let top = max as f64 * 1.1;
    let dot_rows = plot_h * 4;
    let dot_y = |v: i64| ((1.0 - v as f64 / top) * f64::from(dot_rows - 1)).round() as i32;

    // gridlines and count labels at quarter heights
    for q in 1..=3 {
        let y = plot_h * q / 4;
        for x in (x0..x0 + i32::from(cols)).step_by(2) {
            put(buf, area, x, y, "┄", pal().grid);
        }
        let v = top * (1.0 - (f64::from(y) + 0.5) / f64::from(plot_h));
        let label = fmt::count(v);
        text(buf, area, i32::from(LABEL_W) - label.chars().count() as i32, y, &label, pal().chart_muted);
        put(buf, area, x0 - 1, y, "┤", pal().grid);
    }
    for y in 0..plot_h {
        if !(1..=3).any(|q| plot_h * q / 4 == y) {
            put(buf, area, x0 - 1, y, "│", pal().grid);
        }
    }
    put(buf, area, x0 - 1, plot_h, "└", pal().grid);
    for x in x0..x0 + i32::from(cols) {
        put(buf, area, x, plot_h, "─", pal().grid);
    }

    // the line: one dot column per minute, each joined to the one before
    let n = (usize::from(cols) * 2).min(totals.len());
    let shown = &totals[totals.len() - n..];
    let skip = usize::from(cols) * 2 - n; // empty dot columns on the left
    let mut bits = vec![0u32; usize::from(cols) * plot_h as usize];
    let mut low = vec![-1i32; usize::from(cols)]; // lowest line row per cell column
    let mut prev: Option<i32> = None;
    for (i, &v) in shown.iter().enumerate() {
        let d = skip + i;
        let y = dot_y(v);
        let (a, b) = prev.map_or((y, y), |p| (y.min(p), y.max(p)));
        for dy in a..=b {
            let (cx, cy) = (d / 2, (dy / 4) as usize);
            bits[cy * usize::from(cols) + cx] |= DOTS[d % 2][(dy % 4) as usize];
            low[cx] = low[cx].max(dy / 4);
        }
        prev = Some(y);
    }
    let shades = [("▓", 0.55), ("▒", 0.7), ("▒", 0.7), ("░", 0.82)];
    for cx in 0..usize::from(cols) {
        let x = x0 + cx as i32;
        if low[cx] >= 0 {
            for (k, y) in (low[cx] + 1..plot_h).enumerate() {
                let (s, t) = shades[k.min(shades.len() - 1)];
                put(buf, area, x, y, s, mix(pal().accent, pal().bg, t));
            }
        }
        for cy in 0..plot_h as usize {
            let b = bits[cy * usize::from(cols) + cx];
            if b != 0 {
                let ch = char::from_u32(0x2800 + b).unwrap_or(' ');
                put(buf, area, x, cy as i32, ch.encode_utf8(&mut [0; 4]), pal().accent);
            }
        }
    }

    // time labels on whole quarter hours, under a tick
    let cur = now - now.rem_euclid(60);
    let mut last_end = i32::MIN;
    for i in 0..n {
        let ts = cur - (n - 1 - i) as i64 * 60;
        if ts.rem_euclid(900) != 0 {
            continue;
        }
        let x = x0 + ((skip + i) / 2) as i32;
        put(buf, area, x, plot_h, "┴", pal().grid);
        let label = fmt::time(ts);
        if x > last_end && x + label.chars().count() as i32 <= x0 + i32::from(cols) {
            text(buf, area, x, plot_h + 1, &label, pal().chart_muted);
            last_end = x + label.chars().count() as i32;
        }
    }

    // live tag on the newest point's row
    if let Some(&v) = shown.last() {
        let tag = format!("◄ {}", fmt::count(v as f64));
        text(buf, area, x0 + i32::from(cols), dot_y(v) / 4, &tag, pal().accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    #[test]
    fn the_newest_minute_is_at_the_right_edge_with_its_tag() {
        let area = Rect::new(0, 0, 40, 10);
        let cols = plot_cols(40) as usize;
        // flat, then the newest minute at the top
        let mut totals = vec![4000; cols * 2];
        *totals.last_mut().unwrap() = 10_000;
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, &totals, 1_800_000_000);
        let x = (LABEL_W + 1) as usize + cols - 1;
        let is_braille = |s: &str| s.chars().next().is_some_and(|c| ('\u{2801}'..='\u{28FF}').contains(&c));
        // the top row holds the peak in the last column, and the tag
        assert!(is_braille(buf[(x as u16, 0)].symbol()), "{}", row(&buf, 0));
        assert!(row(&buf, 0).contains(&format!("◄ {}", fmt::count(10_000.0))), "{}", row(&buf, 0));
        // shaded under the flat line, near the bottom
        assert!((0..8).any(|y| row(&buf, y).contains('▓')));
        // the baseline and axis
        assert!(row(&buf, 8).contains('└') && row(&buf, 8).contains('─'));
    }

    #[test]
    fn no_volume_says_so() {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, &[0; 10], 0);
        assert!(row(&buf, 0).contains("No volume yet"));
    }

    #[test]
    fn totals_sum_the_listed_tickers_per_minute() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let mut app = App::new(crate::config::Config::default(), "unused-volume-config.json".into(), tx, htx, None);
        let c = |start, volume| crate::engine::Candle { start, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume };
        app.engine.insert_candles("A", &[c(600, 5), c(660, 7), c(780, 1)]);
        app.engine.insert_candles("B", &[c(660, 10)]);
        app.engine.insert_candles("OFF", &[c(660, 1000)]);
        let mk = |s: &str| crate::scraper::Ticker { symbol: s.into(), ..Default::default() };
        app.tickers = vec![mk("A"), mk("B")];
        // minutes 600..=780: 780 is the one in progress at 790
        assert_eq!(minute_totals(&app, 4, 790), vec![5, 17, 0, 1]);
    }
}
