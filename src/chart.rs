//! Candlestick chart for one ticker. Layout and glyphs follow ASCII Magic's
//! "candlestick-chart" piece (MIT, see THIRD_PARTY_LICENSES.md), drawn from real 1-minute candles.

use crate::engine::Candle;
use chrono::{Local, TimeZone};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

const TEXT: Color = Color::Rgb(230, 237, 243);
const MUTED: Color = Color::Rgb(139, 148, 158);
const UP: Color = Color::Rgb(63, 185, 80);
const DOWN: Color = Color::Rgb(248, 81, 73);
const GRID: Color = Color::Rgb(48, 54, 61);
const LIVE: Color = Color::Rgb(121, 192, 255);

fn put(buf: &mut Buffer, area: Rect, x: i32, y: i32, ch: &str, fg: Color) {
    if x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
        return;
    }
    buf[(area.x + x as u16, area.y + y as u16)].set_symbol(ch).set_style(Style::new().fg(fg));
}

fn text(buf: &mut Buffer, area: Rect, x: i32, y: i32, s: &str, fg: Color) {
    for (i, ch) in s.chars().enumerate() {
        put(buf, area, x + i as i32, y, ch.encode_utf8(&mut [0; 4]), fg);
    }
}

fn get(buf: &Buffer, area: Rect, x: i32, y: i32) -> String {
    if x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
        return String::new();
    }
    buf[(area.x + x as u16, area.y + y as u16)].symbol().to_string()
}

fn hhmm(epoch: i64) -> String {
    Local.timestamp_opt(epoch, 0).single().map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// Draws `symbol`'s candles into `area`. `day_change_pct` is the ticker's % change on the day.
pub fn draw(buf: &mut Buffer, area: Rect, symbol: &str, candles: &[Candle], day_change_pct: f64) {
    let (cols, rows) = (area.width as i32, area.height as i32);
    if cols < 30 || rows < 10 {
        text(buf, area, 0, 0, "terminal too small for chart", MUTED);
        return;
    }
    text(buf, area, 0, 0, symbol, TEXT);
    text(buf, area, symbol.len() as i32 + 2, 0, "1m", MUTED);
    let Some(live) = candles.last() else {
        text(buf, area, 0, 2, "Collecting data. Candles appear after the first refresh in each minute.", MUTED);
        return;
    };

    let x1 = cols - 11;
    let (plot_top, plot_bot) = (2, rows - 5);
    let nc = ((x1 + 1) / 2) as usize;
    let view = &candles[candles.len().saturating_sub(nc)..];

    let mut lo = view.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let mut hi = view.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let pad = ((hi - lo) * 0.05).max(hi.abs() * 0.001).max(1e-4);
    lo -= pad;
    hi += pad;
    let to_y = |v: f64| (plot_top as f64 + (1.0 - (v - lo) / (hi - lo)) * (plot_bot - plot_top) as f64).round() as i32;
    let decimals = if hi < 1.0 { 4 } else { 2 };

    // header: O H L C and day change
    let up = live.close >= live.open;
    let hx = symbol.len() as i32 + 6;
    let mut x = hx;
    for (label, v, col) in [("O", live.open, MUTED), ("H", live.high, MUTED), ("L", live.low, MUTED), ("C", live.close, if up { UP } else { DOWN })] {
        let s = format!("{label} {v:.decimals$}");
        text(buf, area, x, 0, &s, col);
        x += s.len() as i32 + 2;
    }
    text(buf, area, x, 0, &format!("{day_change_pct:+.2}%"), if day_change_pct >= 0.0 { UP } else { DOWN });

    // grid and price axis
    for q in 0..=4 {
        let v = lo + q as f64 / 4.0 * (hi - lo);
        let y = to_y(v);
        for gx in (0..=x1).step_by(2) {
            put(buf, area, gx, y, "·", GRID);
        }
        text(buf, area, x1 + 3, y, &format!("{v:.decimals$}"), MUTED);
    }
    for y in plot_top..=rows - 2 {
        put(buf, area, x1 + 1, y, "│", GRID);
    }
    for gx in 0..=x1 {
        put(buf, area, gx, plot_bot + 1, "─", GRID);
    }

    // candles and volume
    let max_vol = view.iter().map(|c| c.volume).max().unwrap_or(0).max(1) as f64;
    for (i, c) in view.iter().enumerate() {
        let x = i as i32 * 2;
        let col = if c.close >= c.open { UP } else { DOWN };
        let (yh, yl) = (to_y(c.high), to_y(c.low));
        let (yt, yb) = (to_y(c.open.max(c.close)), to_y(c.open.min(c.close)));
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

    // live price line and tag
    let ly = to_y(live.close);
    for gx in (0..x1).step_by(3) {
        let cur = get(buf, area, gx, ly);
        if cur == " " || cur == "·" {
            put(buf, area, gx, ly, "╌", LIVE);
        }
    }
    let col = if up { UP } else { DOWN };
    put(buf, area, x1 + 1, ly, "◄", col);
    text(buf, area, x1 + 2, ly, &format!(" {:.decimals$} ", live.close), col);

    // time axis
    for (i, c) in view.iter().enumerate().step_by(10) {
        text(buf, area, i as i32 * 2, rows - 1, &hhmm(c.start), MUTED);
    }
}
