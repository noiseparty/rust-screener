//! Candlestick chart for one ticker. Layout and glyphs follow ASCII Magic's
//! "candlestick-chart" piece (MIT, see THIRD_PARTY_LICENSES.md), drawn from real 1-minute candles.

use crate::engine::Candle;
use crate::fmt;
use crate::history::{self, Tf};
use crate::levels::Level;
use crate::theme::{mix, pal};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

pub(crate) fn put(buf: &mut Buffer, area: Rect, x: i32, y: i32, ch: &str, fg: Color) {
    if x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
        return;
    }
    buf[(area.x + x as u16, area.y + y as u16)].set_symbol(ch).set_style(Style::new().fg(fg));
}

pub(crate) fn text(buf: &mut Buffer, area: Rect, x: i32, y: i32, s: &str, fg: Color) {
    for (i, ch) in s.chars().enumerate() {
        put(buf, area, x + i as i32, y, ch.encode_utf8(&mut [0; 4]), fg);
    }
}

/// `s` on a filled block of `w` cells from `x` (padded with spaces).
fn filled(buf: &mut Buffer, area: Rect, x: i32, y: i32, w: i32, s: &str, fg: Color, bg: Color) {
    if y < 0 || y >= area.height as i32 {
        return;
    }
    for (i, ch) in format!("{s:<w$}", w = w.max(0) as usize).chars().enumerate() {
        let x = x + i as i32;
        if x >= 0 && x < area.width as i32 {
            buf[(area.x + x as u16, area.y + y as u16)].set_symbol(ch.encode_utf8(&mut [0; 4])).set_style(Style::new().fg(fg).bg(bg));
        }
    }
}

/// Black or white, whichever reads better on `bg`.
fn on(bg: Color) -> Color {
    match bg {
        Color::Rgb(r, g, b) if 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b) <= 160.0 => Color::White,
        _ => Color::Black,
    }
}

/// Time left in a candle as the price axis shows it: `mm:ss` (1m, 5m), `hh:mm:ss` (1h, 4h),
/// `Nd hh:mm` (D).
pub fn countdown(left: i64, tf: Tf) -> String {
    let left = left.max(0);
    let (m, s) = (left / 60 % 60, left % 60);
    match tf {
        Tf::M1 | Tf::M5 => format!("{:02}:{s:02}", left / 60),
        Tf::H1 | Tf::H4 => crate::fmt::hms(left),
        Tf::D1 => format!("{}d {:02}:{m:02}", left / 86_400, left / 3600 % 24),
    }
}

fn get(buf: &Buffer, area: Rect, x: i32, y: i32) -> String {
    if x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
        return String::new();
    }
    buf[(area.x + x as u16, area.y + y as u16)].symbol().to_string()
}

/// How a chart is labelled: its timeframe, whether the time axis shows dates, and the text shown while
/// there are no candles.
pub struct Spec<'a> {
    pub tf: Tf,
    /// Time axis in day and month (4h, D) instead of clock times.
    pub day_axis: bool,
    pub empty: &'a str,
    /// Bars the right edge sits back from the newest; negative leaves empty space after it.
    pub pan: i64,
    /// Price range scale around the fitted range's middle (1 = fit, 2 = twice as tall a range).
    pub zoom: f64,
    /// Price range moved up by this fraction of its height (dragging the plot down).
    pub shift: f64,
    /// 4h swing lines, drawn from the candle holding their bar to the right edge.
    pub levels: &'a [Level],
    /// Status text under the header (e.g. while the levels load).
    pub note: &'a str,
    /// Time zoom: index into `ZOOMS`.
    pub scale: usize,
    /// Leave candles and volume to the caller (`Plot`), which paints them at pixel resolution.
    pub pixel: bool,
    /// Unix time now, for the countdown of the newest candle.
    pub now: i64,
}

/// Candles and volume of a drawn chart, for painting at pixel resolution. Cells are absolute
/// buffer positions; price `v` sits at row `top + 0.5 + (hi - v) / (hi - lo) * span` (row units).
#[derive(Debug, Clone)]
pub struct Plot {
    /// Column of the first candle and the row of the plot's top.
    pub x: u16,
    pub top: u16,
    /// Rows from the top row to the bottom row of the price plot.
    pub span: f64,
    /// Columns per candle.
    pub step: u16,
    pub lo: f64,
    pub hi: f64,
    pub candles: Vec<Candle>,
    /// First of the two volume rows.
    pub vol_row: u16,
    pub max_vol: f64,
    pub up: Color,
    pub down: Color,
}

/// Time zoom levels, zoomed in to zoomed out: (columns per shown candle, bars merged into one
/// shown candle).
pub const ZOOMS: [(u16, usize); 11] = [(4, 1), (3, 1), (2, 1), (1, 1), (1, 2), (1, 3), (1, 4), (1, 6), (1, 8), (1, 12), (1, 16)];
/// One candle every two columns.
pub const DEFAULT_SCALE: usize = 2;

/// Plot columns of a chart area of `cols` columns: candles at x = 0, 2, ... up to here.
pub fn plot_cols(cols: u16) -> u16 {
    cols.saturating_sub(10)
}

/// How many candles are shown in a chart area `cols` columns wide at time zoom `scale`.
fn shown(cols: u16, scale: usize) -> usize {
    (plot_cols(cols) as usize).div_ceil(ZOOMS[scale].0 as usize)
}

/// How many bars a chart area `cols` columns wide covers at time zoom `scale`.
pub fn fit(cols: u16, scale: usize) -> usize {
    shown(cols, scale) * ZOOMS[scale].1
}

/// One candle from consecutive bars (oldest first).
fn merged(bars: &[Candle]) -> Candle {
    let (first, last) = (bars[0], bars[bars.len() - 1]);
    Candle {
        start: first.start,
        open: first.open,
        close: last.close,
        high: bars.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max),
        low: bars.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
        volume: bars.iter().map(|c| c.volume).sum(),
    }
}

/// Draws `symbol`'s candles into `area`. `day_change_pct` is the ticker's % change on the day.
/// Returns the candles to paint when `spec.pixel` (they are not drawn into the buffer then).
pub fn draw(buf: &mut Buffer, area: Rect, symbol: &str, candles: &[Candle], day_change_pct: f64, spec: &Spec) -> Option<Plot> {
    let (cols, rows) = (area.width as i32, area.height as i32);
    if cols < 30 || rows < 10 {
        text(buf, area, 0, 0, "terminal too small for chart", pal().chart_muted);
        return None;
    }
    text(buf, area, 0, 0, symbol, pal().chart_text);
    text(buf, area, symbol.len() as i32 + 2, 0, spec.tf.label(), pal().chart_muted);
    let Some(live) = candles.last() else {
        text(buf, area, 0, 2, spec.empty, pal().chart_muted);
        return None;
    };

    let x1 = plot_cols(area.width) as i32 - 1;
    let (plot_top, plot_bot) = (2, rows - 5);
    let (step, merge) = ZOOMS[spec.scale];
    let step = step as i32;
    // the plot covers bars start..end (end = the right edge, `pan` bars back from the newest);
    // bars outside what is held leave empty slots, and at least one bar stays in view
    let (n, width) = (candles.len() as i64, fit(area.width, spec.scale) as i64);
    let end = (n - spec.pan).clamp(1, n + width - 1);
    let start = end - width;
    let (first, last) = (start.max(0) as usize, end.min(n) as usize);
    // zoomed out, each shown candle merges the `merge` bars of its slot
    let m = merge as i64;
    let mut view = Vec::new();
    let mut b = first;
    while b < last {
        let next = (start + ((b as i64 - start) / m + 1) * m).min(last as i64) as usize;
        view.push(merged(&candles[b..next]));
        b = next;
    }
    let view = view.as_slice();
    // column of the first shown candle
    let left = ((first as i64 - start) / m) as i32 * step;

    let mut lo = view.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let mut hi = view.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let pad = ((hi - lo) * 0.05).max(hi.abs() * 0.001).max(1e-4);
    lo -= pad;
    hi += pad;
    let half = (hi - lo) / 2.0 * spec.zoom;
    let mid = (hi + lo) / 2.0 + spec.shift * 2.0 * half;
    (lo, hi) = (mid - half, mid + half);
    let to_y = |v: f64| (plot_top as f64 + (1.0 - (v - lo) / (hi - lo)) * (plot_bot - plot_top) as f64).round() as i32;
    let decimals = if hi < 1.0 { 4 } else { 2 };

    // header: O H L C and day change
    let up = live.close >= live.open;
    let hx = symbol.len() as i32 + 6;
    let mut x = hx;
    for (label, v, col) in [
        ("O", live.open, pal().chart_muted),
        ("H", live.high, pal().chart_muted),
        ("L", live.low, pal().chart_muted),
        ("C", live.close, if up { pal().up } else { pal().down }),
    ] {
        let s = format!("{label} {}", fmt::num(v, decimals));
        text(buf, area, x, 0, &s, col);
        x += s.len() as i32 + 2;
    }
    text(buf, area, x, 0, &format!("{day_change_pct:+.2}%"), if day_change_pct >= 0.0 { pal().up } else { pal().down });

    // grid and price axis
    for q in 0..=4 {
        let v = lo + q as f64 / 4.0 * (hi - lo);
        let y = to_y(v);
        for gx in (0..=x1).step_by(2) {
            put(buf, area, gx, y, "·", pal().grid);
        }
        text(buf, area, x1 + 3, y, &fmt::num(v, decimals), pal().chart_muted);
    }
    for y in plot_top..=rows - 2 {
        put(buf, area, x1 + 1, y, "│", pal().grid);
    }
    for gx in 0..=x1 {
        put(buf, area, gx, plot_bot + 1, "─", pal().grid);
    }

    // 4h swing lines: the 3 open rows nearest the live price in white, every taken one (price
    // traded above it) dimmed; at the line's start the date and price of the high that set it,
    // in the Time & Format settings, by default "17.11.26 ($3.86)", 4 decimals under $1 (candles drawn later win, open lines over
    // taken ones). Lines that fall on one text row are one line: the newest gives the tag, the
    // others add " +N", so close lines do not overwrite each other's tag as the price moves. An
    // open row off the plot is a tag on the top or bottom edge instead, e.g. "▲ 17.11.26 ($3.86)".
    // a line set after the shown range (scrolled back) is not drawn
    let shown = spec.levels.iter().filter(|l| !candles.get(last).is_some_and(|c| c.start <= l.at));
    let (open, taken): (Vec<&Level>, Vec<&Level>) = shown.partition(|l| !l.taken);
    let mut open = level_rows(&open, &to_y);
    open.sort_by(|a, b| (a.price - live.close).abs().total_cmp(&(b.price - live.close).abs()).then(a.y.cmp(&b.y)));
    open.truncate(3);
    let taken: Vec<LevelRow> = level_rows(&taken, &to_y).into_iter().filter(|t| open.iter().all(|o| o.y != t.y)).collect();
    let dim = mix(pal().chart_muted, pal().bg, 0.5);
    let mut edge_tags: Vec<(i32, String)> = Vec::new();
    let (mut n_above, mut n_below) = (0, 0);
    for (r, is_open) in taken.iter().map(|r| (r, false)).chain(open.iter().map(|r| (r, true))) {
        let y = r.y;
        let more = if r.n > 1 { format!(" +{}", r.n - 1) } else { String::new() };
        let tag = format!("{} ({}){more}", fmt::date(r.at), fmt::money(r.price));
        if !(plot_top..=plot_bot).contains(&y) {
            if is_open {
                let above = y < plot_top;
                let row = if above { plot_top + n_above } else { plot_bot - n_below };
                *(if above { &mut n_above } else { &mut n_below }) += 1;
                edge_tags.push((row, format!("{} {tag}", if above { "▲" } else { "▼" })));
            }
            continue;
        }
        let x0 = left + view.partition_point(|c| c.start <= r.first).saturating_sub(1) as i32 * step;
        let col = if is_open { pal().chart_text } else { dim };
        for gx in x0..=x1 {
            put(buf, area, gx, y, "─", col);
        }
        text(buf, area, (x0 + 1).min(x1 - tag.chars().count() as i32).max(0), y, &tag, col);
    }

    // candles and volume
    let max_vol = view.iter().map(|c| c.volume).max().unwrap_or(0).max(1) as f64;
    for (i, c) in view.iter().enumerate().filter(|_| !spec.pixel) {
        let x = left + i as i32 * step;
        let col = if c.close >= c.open { pal().up } else { pal().down };
        // a narrowed price range clips candles to the plot
        let clip = |y: i32| y.clamp(plot_top, plot_bot);
        let (yh, yl) = (clip(to_y(c.high)), clip(to_y(c.low)));
        let (yt, yb) = (clip(to_y(c.open.max(c.close))), clip(to_y(c.open.min(c.close))));
        for y in yh..=yl {
            put(buf, area, x, y, "│", col);
        }
        if yt == yb {
            put(buf, area, x, yt, "━", col);
        } else {
            for y in yt..=yb {
                put(buf, area, x, y, "█", col);
            }
        }
        if c.volume > 0 {
            let vh = ((c.volume as f64 / max_vol) * 4.0).ceil() as i32; // half-rows, 2 rows tall
            put(buf, area, x, rows - 2, if vh >= 2 { "█" } else { "▄" }, col);
            if vh > 2 {
                put(buf, area, x, rows - 3, if vh >= 4 { "█" } else { "▄" }, col);
            }
        }
    }

    // live price line and its tag on the axis (a filled block), when the live price is inside
    // the shown range; under the tag the time left in the newest candle while it is in view
    let ly = to_y(live.close);
    if (plot_top..=plot_bot).contains(&ly) {
        for gx in (0..x1).step_by(3) {
            let cur = get(buf, area, gx, ly);
            if cur == " " || cur == "·" {
                put(buf, area, gx, ly, "╌", pal().live);
            }
        }
        let col = if up { pal().up } else { pal().down };
        put(buf, area, x1 + 1, ly, "◄", col);
        let w = cols - x1 - 2;
        filled(buf, area, x1 + 2, ly, w, &format!(" {}", fmt::num(live.close, decimals)), on(col), col);
        let left = history::bar_end(live.start, spec.tf) - spec.now;
        if last == candles.len() && left > 0 {
            filled(buf, area, x1 + 2, ly + 1, w, &format!(" {}", countdown(left, spec.tf)), on(col), col);
        }
    }
    // edge tags over the candles, right-aligned to the plot
    for (row, tag) in &edge_tags {
        text(buf, area, (x1 + 1 - tag.chars().count() as i32).max(0), *row, tag, pal().chart_text);
    }
    text(buf, area, 0, 1, spec.note, pal().chart_muted);

    // time axis
    for (i, c) in view.iter().enumerate().step_by((20 / step as usize).max(1)) {
        text(
            buf,
            area,
            left + i as i32 * step,
            rows - 1,
            &if spec.day_axis { fmt::day(c.start) } else { fmt::time(c.start) },
            pal().chart_muted,
        );
    }
    spec.pixel.then(|| Plot {
        x: area.x + left as u16,
        top: area.y + plot_top as u16,
        span: f64::from(plot_bot - plot_top),
        step: step as u16,
        lo,
        hi,
        candles: view.to_vec(),
        vol_row: area.y + (rows - 3) as u16,
        max_vol,
        up: pal().up,
        down: pal().down,
    })
}

/// The swing lines that fall on one text row, drawn as one.
struct LevelRow {
    y: i32,
    /// Date and price of the newest line on the row (its tag).
    at: i64,
    price: f64,
    /// Start of the oldest line on the row (where the drawn line starts).
    first: i64,
    /// Lines on the row.
    n: usize,
}

/// `lines` grouped by the row `to_y` puts them on, top to bottom.
fn level_rows(lines: &[&Level], to_y: &dyn Fn(f64) -> i32) -> Vec<LevelRow> {
    let mut rows: Vec<LevelRow> = Vec::new();
    for l in lines {
        let y = to_y(l.price);
        match rows.iter_mut().find(|r| r.y == y) {
            Some(r) => {
                if (l.at, l.price) > (r.at, r.price) {
                    (r.at, r.price) = (l.at, l.price);
                }
                r.first = r.first.min(l.at);
                r.n += 1;
            }
            None => rows.push(LevelRow { y, at: l.at, price: l.price, first: l.at, n: 1 }),
        }
    }
    rows.sort_by_key(|r| r.y);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(i: i64, low: f64, high: f64) -> Candle {
        Candle { start: i * 60, open: low, high, low, close: high, volume: 0 }
    }

    fn spec(pan: i64, levels: &[Level], pixel: bool) -> Spec<'_> {
        Spec { tf: Tf::M1, day_axis: false, empty: "", pan, zoom: 1.0, shift: 0.0, levels, note: "", scale: DEFAULT_SCALE, pixel, now: 0 }
    }

    /// Columns of the plot (row by row) holding a candle body.
    fn body_cols(buf: &Buffer) -> Vec<u16> {
        let mut cols: Vec<u16> = (0..40).filter(|&x| (2..=15).any(|y| buf[(x, y)].symbol() == "█")).collect();
        cols.dedup();
        cols
    }

    #[test]
    fn a_short_series_sits_at_the_right_edge_and_pans_into_empty_space() {
        // 50 columns: 40 plot columns, a candle every 2 -> 20 slots
        let area = Rect::new(0, 0, 50, 20);
        let candles: Vec<Candle> = (0..5).map(|i| candle(i, 1.0, 2.0)).collect();
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec(0, &[], false));
        assert_eq!(body_cols(&buf), vec![30, 32, 34, 36, 38]);
        // dragged left: 4 empty slots after the newest bar
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec(-4, &[], false));
        assert_eq!(body_cols(&buf), vec![22, 24, 26, 28, 30]);
        // dragged right: the newest bars leave the plot
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec(2, &[], false));
        assert_eq!(body_cols(&buf), vec![34, 36, 38]);
        // the pixel plot starts at the first candle's column
        let plot = draw(&mut Buffer::empty(area), area, "A", &candles, 0.0, &spec(-4, &[], true)).unwrap();
        assert_eq!((plot.x, plot.candles.len()), (22, 5));
    }

    #[test]
    fn only_the_three_open_levels_nearest_the_price_are_drawn_and_taken_ones_dimmed() {
        let area = Rect::new(0, 0, 50, 20);
        let mut candles: Vec<Candle> = (0..20).map(|i| candle(i, 1.0, 10.0)).collect();
        candles[19].close = 5.5;
        let level = |price, taken| Level { price, at: 0, taken };
        let levels = [
            level(2.0, false),
            level(3.0, false),
            level(4.0, false),
            level(5.0, false),
            level(6.0, false),
            level(7.0, true),
            level(9.0, true),
        ];
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec(0, &levels, false));
        // the right end of each line, past the last candle
        let lines = |fg: Color| (2..=15).filter(|&y| buf[(39, y)].symbol() == "─" && buf[(39, y)].fg == fg).count();
        assert_eq!(lines(pal().chart_text), 3);
        assert_eq!(lines(mix(pal().chart_muted, pal().bg, 0.5)), 2);
    }

    #[test]
    fn lines_on_one_row_are_one_line_whichever_side_the_price_is() {
        let area = Rect::new(0, 0, 50, 20);
        let level = |price, at| Level { price, at, taken: false };
        // 5.0 and 5.1 share a row (~0.76 per row); 2.0 is the 4th row from the price
        let levels = [level(5.0, 0), level(5.1, 60), level(7.0, 0), level(3.0, 0), level(2.0, 0)];
        // (the live price line runs through the tag's gap)
        let tag = "($5.10) +1";
        for close in [5.04, 5.06] {
            let mut candles: Vec<Candle> = (0..20).map(|i| candle(i, 1.0, 10.0)).collect();
            candles[19].close = close;
            let mut buf = Buffer::empty(area);
            draw(&mut buf, area, "A", &candles, 0.0, &spec(0, &levels, true));
            let rows: Vec<String> = (0..20).map(|y| (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
            assert_eq!(rows.iter().filter(|r| r.contains(tag)).count(), 1, "{close}: {rows:#?}");
            // three open rows: 5.x, 7 and 3
            let open = (2..=15).filter(|&y| buf[(39, y)].symbol() == "─" && buf[(39, y)].fg == pal().chart_text).count();
            assert_eq!(open, 3, "{close}");
            assert!(!rows.iter().any(|r| r.contains("($2.00)")), "{close}");
        }
    }

    #[test]
    fn open_levels_off_the_plot_are_tagged_on_its_edges() {
        let area = Rect::new(0, 0, 50, 20);
        let candles: Vec<Candle> = (0..20).map(|i| candle(i, 4.0, 5.0)).collect();
        let level = |price, taken| Level { price, at: 0, taken };
        let levels = [level(9.0, false), level(2.0, false), level(1.5, false), level(12.0, true)];
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec(0, &levels, false));
        let row = |y: u16| (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        // the nearest below on the bottom row, the next one stacked above it; taken lines get no tag
        assert!(row(2).ends_with(&format!("▲ {} ($9.00)", fmt::date(0))), "{}", row(2));
        assert!(row(15).contains('▼') && row(15).ends_with("($2.00)"), "{}", row(15));
        assert!(row(14).contains('▼') && row(14).ends_with("($1.50)"), "{}", row(14));
        assert!(!row(3).contains('▲'));
    }

    #[test]
    fn level_tags_show_the_full_date_and_four_decimals_under_a_dollar() {
        let area = Rect::new(0, 0, 50, 20);
        let candles: Vec<Candle> = (0..20).map(|i| candle(i, 0.5, 0.9)).collect();
        let levels = [Level { price: 0.7, at: 0, taken: false }];
        let mut buf = Buffer::empty(area);
        // pixel mode leaves the candles out of the buffer, so the tag is not drawn over
        draw(&mut buf, area, "A", &candles, 0.0, &spec(0, &levels, true));
        let rows: Vec<String> = (0..20).map(|y| (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        let tag = format!("{} ($0.7000)", fmt::date(0));
        assert!(rows.iter().any(|r| r.contains(&tag)), "{rows:#?}");
    }

    #[test]
    fn countdown_text_per_timeframe() {
        assert_eq!(countdown(59, Tf::M1), "00:59");
        assert_eq!(countdown(4 * 60 + 5, Tf::M5), "04:05");
        assert_eq!(countdown(3599, Tf::H1), "00:59:59");
        assert_eq!(countdown(3 * 3600 + 25 * 60 + 7, Tf::H4), "03:25:07");
        assert_eq!(countdown(26 * 3600 + 5 * 60 + 30, Tf::D1), "1d 02:05");
        assert_eq!(countdown(-5, Tf::M1), "00:00");
    }

    #[test]
    fn price_tag_and_countdown_are_filled_blocks_hidden_when_scrolled_back() {
        let area = Rect::new(0, 0, 50, 20);
        let candles: Vec<Candle> = (0..5).map(|i| candle(i, 1.0, 2.0)).collect();
        // 15 s into the newest 1m candle (it starts at 4:00): 45 s left
        let spec = Spec { now: 4 * 60 + 15, ..spec(0, &[], false) };
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &spec);
        let row = |buf: &Buffer, y: u16| (41..50).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        let ly = (2..=15).find(|&y| buf[(40, y)].symbol() == "◄").expect("price tag");
        assert_eq!(row(&buf, ly), " 2.00    ");
        assert_eq!(row(&buf, ly + 1), " 00:45   ");
        assert_eq!(buf[(49, ly)].bg, pal().up);
        assert_eq!(buf[(49, ly + 1)].bg, pal().up);
        // scrolled away from the newest bar: the price tag stays, the countdown goes
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &Spec { pan: 1, ..spec });
        assert!(!row(&buf, ly + 1).contains("00:"));
        // the pixel chart gets the same blocks
        let mut buf = Buffer::empty(area);
        draw(&mut buf, area, "A", &candles, 0.0, &Spec { pixel: true, ..spec });
        assert_eq!(row(&buf, ly + 1), " 00:45   ");
    }
}
