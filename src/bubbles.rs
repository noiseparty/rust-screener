//! Bubble map of the listed tickers: each ticker's volume over the selected timeframe as a
//! circle whose area follows the volume. Bubbles keep their place between frames (`Field`).
//!
//! Layout works in square units: one unit is a cell's width, and a cell is two units tall.

use crate::engine::Candle;
use ratatui::style::Color;

/// Share of the panel the bubbles cover before the minimum sizes.
const FILL: f32 = 0.55;
/// Space between bubbles, in units.
const GAP: f32 = 0.5;
/// Shades (share of the background blended into the accent colour) a bubble picks from.
pub const SHADES: [f64; 5] = [0.35, 0.45, 0.55, 0.65, 0.75];

/// Shares traded in the `secs` before `now`: the 1m candles starting inside that window
/// (the minute in progress included).
pub fn delta<'a>(candles: impl IntoIterator<Item = &'a Candle>, secs: i64, now: i64) -> i64 {
    candles.into_iter().filter(|c| c.start >= now - secs && c.start <= now).map(|c| c.volume).sum()
}

/// One ticker to place: its volume and the least radius its label needs.
pub struct Item {
    pub symbol: String,
    pub delta: i64,
    pub min_r: f32,
}

/// A placed bubble: centre and radius in units from the panel's top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    pub symbol: String,
    pub delta: i64,
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    /// Index into `SHADES`, fixed per symbol so a ticker keeps its colour as ranks change.
    pub shade: usize,
}

/// What the window app paints: a bubble in cell coordinates of the whole grid (x in columns,
/// y in rows), its radius in columns, its colours and its label lines (ticker, volume, price).
#[derive(Debug, Clone)]
pub struct Painted {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    pub fill: Color,
    pub text: Color,
    pub lines: [String; 3],
}

/// How fast a bubble's size follows its volume (per second, exponential).
const GROW_RATE: f32 = 3.0;
/// How fast bubbles drift toward the centre (share of the distance per second).
const PULL_RATE: f32 = 0.15;
/// Overlap passes per step.
const PASSES: usize = 8;

/// Radius each item should have in a `w` x `h` panel: area follows `delta`, covering `FILL` of
/// the panel (less when the largest would not fit), at least `min_r`. None: does not fit at all.
fn targets(items: &[Item], w: f32, h: f32) -> Vec<Option<f32>> {
    let total: f64 = items.iter().map(|i| i.delta.max(0) as f64).sum();
    let r_max = w.min(h) / 2.0;
    let largest = items.iter().map(|i| i.delta).max().unwrap_or(0).max(1) as f64;
    let k = (f64::from(FILL * w * h) / (std::f64::consts::PI * total.max(1.0))).sqrt().min(f64::from(r_max) / largest.sqrt());
    items.iter().map(|i| Some(((k * (i.delta.max(0) as f64).sqrt()) as f32).max(i.min_r)).filter(|&r| r <= r_max)).collect()
}

/// A free spot for a bubble of radius `r` among `others`, searched on a spiral out from the
/// centre (stretched to the panel's shape, about one unit between tries).
fn free_spot(others: &[Bubble], r: f32, w: f32, h: f32) -> Option<(f32, f32)> {
    let (sx, sy) = (w / w.max(h), h / w.max(h));
    let reach = w.hypot(h) / 2.0;
    let (mut theta, mut rad) = (0.0f32, 0.0f32);
    loop {
        let (x, y) = (w / 2.0 + rad * theta.cos() * sx, h / 2.0 + rad * theta.sin() * sy);
        let inside = x - r >= 0.0 && x + r <= w && y - r >= 0.0 && y + r <= h;
        if inside && others.iter().all(|b| (b.cx - x).hypot(b.cy - y) >= b.r + r + GAP) {
            return Some((x, y));
        }
        if rad > reach {
            return None;
        }
        theta += 1.0 / rad.max(1.0);
        rad = theta / std::f32::consts::TAU * 2.0;
    }
}

/// The bubbles kept from frame to frame, so they stay where they are: a bubble only grows or
/// shrinks toward its volume, and its neighbours are nudged aside while all drift slowly
/// toward the centre. A new ticker appears small in a free spot; one that left the list goes.
#[derive(Debug, Default)]
pub struct Field {
    bubbles: Vec<Bubble>,
    /// Panel size of the last step (positions are scaled when it changes).
    size: (f32, f32),
}

impl Field {
    /// Moves the bubbles `dt` seconds on for `items` in a `w` x `h` unit panel and returns them.
    pub fn step(&mut self, items: &[Item], w: f32, h: f32, dt: f32) -> &[Bubble] {
        if w < 1.0 || h < 1.0 {
            return &[];
        }
        let (ow, oh) = self.size;
        if (ow, oh) != (w, h) && ow > 0.0 && oh > 0.0 {
            for b in &mut self.bubbles {
                (b.cx, b.cy) = (b.cx * w / ow, b.cy * h / oh);
            }
        }
        self.size = (w, h);
        let want = targets(items, w, h);
        let target = |sym: &str| items.iter().zip(&want).find(|(i, _)| i.symbol == sym).and_then(|(i, r)| r.map(|r| (i.delta, r)));
        // gone or too big: removed; the rest follow their volume
        self.bubbles.retain(|b| target(&b.symbol).is_some_and(|(d, _)| d > 0));
        let grow = 1.0 - (-GROW_RATE * dt).exp();
        for b in &mut self.bubbles {
            let (delta, r) = target(&b.symbol).unwrap_or((0, b.r));
            b.delta = delta;
            b.r += (r - b.r) * grow;
        }
        // new ones, largest first, start at a third of their size where they fit
        let mut fresh: Vec<(&Item, f32)> = items
            .iter()
            .zip(&want)
            .filter(|(i, _)| i.delta > 0 && self.bubbles.iter().all(|b| b.symbol != i.symbol))
            .filter_map(|(i, r)| r.map(|r| (i, r)))
            .collect();
        fresh.sort_by(|a, b| b.0.delta.cmp(&a.0.delta).then_with(|| a.0.symbol.cmp(&b.0.symbol)));
        for (it, r) in fresh {
            if let Some((cx, cy)) = free_spot(&self.bubbles, r, w, h) {
                let shade = it.symbol.bytes().map(usize::from).sum::<usize>() % SHADES.len();
                self.bubbles.push(Bubble { symbol: it.symbol.clone(), delta: it.delta, cx, cy, r: r / 3.0, shade });
            }
        }
        // drift toward the centre, weaker across the panel's long side
        let pull = 1.0 - (-PULL_RATE * dt).exp();
        let (px, py) = (pull * h.min(w) / w, pull * h.min(w) / h);
        for b in &mut self.bubbles {
            b.cx += (w / 2.0 - b.cx) * px;
            b.cy += (h / 2.0 - b.cy) * py;
        }
        // overlaps pushed apart, the smaller bubble moving more; then back inside
        let n = self.bubbles.len();
        for _ in 0..PASSES {
            for i in 0..n {
                for j in i + 1..n {
                    let (a, b) = (&self.bubbles[i], &self.bubbles[j]);
                    let (dx, dy) = (b.cx - a.cx, b.cy - a.cy);
                    let d = dx.hypot(dy);
                    let over = a.r + b.r + GAP - d;
                    if over <= 0.0 {
                        continue;
                    }
                    // the same spot: apart sideways (the same way every time)
                    let (ux, uy) = if d > 1e-3 { (dx / d, dy / d) } else { (1.0, 0.0) };
                    let (wa, wb) = (a.r * a.r, b.r * b.r);
                    let (ma, mb) = (over * wb / (wa + wb), over * wa / (wa + wb));
                    self.bubbles[i].cx -= ux * ma;
                    self.bubbles[i].cy -= uy * ma;
                    self.bubbles[j].cx += ux * mb;
                    self.bubbles[j].cy += uy * mb;
                }
            }
            for b in &mut self.bubbles {
                b.cx = b.cx.clamp(b.r.min(w / 2.0), (w - b.r).max(w / 2.0));
                b.cy = b.cy.clamp(b.r.min(h / 2.0), (h - b.r).max(h / 2.0));
            }
        }
        &self.bubbles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(symbol: &str, delta: i64) -> Item {
        Item { symbol: symbol.into(), delta, min_r: 3.0 }
    }

    #[test]
    fn delta_sums_the_minutes_in_the_window() {
        let c = |start, volume| Candle { start, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume };
        let candles = [c(0, 5), c(60, 7), c(120, 11)];
        assert_eq!(delta(&candles, 60, 150), 11);
        assert_eq!(delta(&candles, 300, 150), 23);
        // a minute starting after `now` is not counted
        assert_eq!(delta(&candles, 60, 30), 5);
    }

    /// `items` run for `secs` at 12 frames a second.
    fn settle(f: &mut Field, items: &[Item], w: f32, h: f32, secs: f32) -> Vec<Bubble> {
        for _ in 0..(secs * 12.0) as usize {
            f.step(items, w, h, 1.0 / 12.0);
        }
        f.step(items, w, h, 1.0 / 12.0).to_vec()
    }

    #[test]
    fn bubbles_settle_inside_the_panel_without_overlaps() {
        let items: Vec<Item> = (1..=30).map(|i| item(&format!("T{i}"), i * i * 1000)).collect();
        let (w, h) = (120.0, 60.0);
        let b = settle(&mut Field::default(), &items, w, h, 10.0);
        assert!(b.len() >= 20, "{}", b.len());
        for (i, x) in b.iter().enumerate() {
            assert!(x.cx - x.r >= -0.01 && x.cx + x.r <= w + 0.01 && x.cy - x.r >= -0.01 && x.cy + x.r <= h + 0.01, "{x:?}");
            for y in &b[i + 1..] {
                assert!((x.cx - y.cx).hypot(x.cy - y.cy) >= x.r + y.r - 0.5, "{x:?} {y:?}");
            }
        }
        let r = |s: &str| b.iter().find(|x| x.symbol == s).unwrap().r;
        assert!(r("T30") > r("T20") && r("T20") > r("T10"));
    }

    #[test]
    fn a_volume_change_grows_the_bubble_in_place() {
        let mut items: Vec<Item> = (1..=10).map(|i| item(&format!("T{i}"), i * 10_000)).collect();
        let (w, h) = (120.0, 60.0);
        let mut f = Field::default();
        let before = settle(&mut f, &items, w, h, 10.0);
        // T3 doubles: it grows gradually, and nothing jumps in a frame
        items[2].delta *= 2;
        let mut last = before.clone();
        for _ in 0..24 {
            let now = f.step(&items, w, h, 1.0 / 12.0).to_vec();
            for (a, b) in last.iter().zip(&now) {
                assert_eq!(a.symbol, b.symbol);
                assert!((a.cx - b.cx).hypot(a.cy - b.cy) < 1.0, "{a:?} -> {b:?}");
            }
            last = now;
        }
        let r = |v: &[Bubble], s: &str| v.iter().find(|x| x.symbol == s).unwrap().r;
        assert!(r(&last, "T3") > r(&before, "T3") * 1.2);
    }

    #[test]
    fn one_big_ticker_still_fits() {
        let b = settle(&mut Field::default(), &[item("BIG", 9_000_000), item("MID", 2_000_000)], 118.0, 52.0, 5.0);
        assert_eq!(b.iter().map(|b| b.symbol.as_str()).collect::<Vec<_>>(), ["BIG", "MID"]);
        assert!((b[0].r - 26.0).abs() < 0.1, "{}", b[0].r);
    }

    #[test]
    fn nothing_to_place() {
        let mut f = Field::default();
        assert!(f.step(&[], 100.0, 50.0, 0.1).is_empty());
        assert!(f.step(&[item("A", 0)], 100.0, 50.0, 0.1).is_empty());
        // too big for the panel
        assert!(f.step(&[Item { min_r: 40.0, ..item("A", 5) }], 60.0, 30.0, 0.1).is_empty());
        // a ticker that leaves the list goes
        assert_eq!(f.step(&[item("A", 5)], 100.0, 50.0, 0.1).len(), 1);
        assert!(f.step(&[item("B", 5)], 100.0, 50.0, 0.1).iter().all(|b| b.symbol == "B"));
    }
}
