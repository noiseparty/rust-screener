//! Animated scenes ported from ASCII Magic (https://www.ascii-magic.com), MIT License,
//! Copyright (c) 2026 ASCII Magic. See THIRD_PARTY_LICENSES.md.
//! - moonlit-mausoleum: boot screen, rendered as half-block pixels.
//! - maintenance-mode: the gear + wrench only, for the settings screens.

use std::f64::consts::PI;
use std::sync::OnceLock;

pub type Rgb = [f64; 3];

// ───────────────────────────── kit.js helpers ─────────────────────────────

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}
/// JS Math.round (half rounds toward +inf).
fn jround(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

pub fn hex(h: &str) -> Rgb {
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    [p(1), p(3), p(5)]
}

fn hash2(x: i64, y: i64, s: i64) -> f64 {
    let mut h = (x as i32)
        .wrapping_mul(374761393)
        .wrapping_add((y as i32).wrapping_mul(668265263))
        .wrapping_add((s as i32).wrapping_mul(2147483647));
    h = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1274126177);
    ((h ^ ((h as u32) >> 16) as i32) as u32) as f64 / 4294967296.0
}

pub fn noise2(x: f64, y: f64, seed: i64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (xf, yf) = (smooth(x - xi), smooth(y - yi));
    let (xi, yi) = (xi as i64, yi as i64);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    lerp(lerp(a, b, xf), lerp(c, d, xf), yf)
}

pub fn fbm(x: f64, y: f64, octaves: u32, seed: i64) -> f64 {
    let (mut v, mut amp, mut f, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for i in 0..octaves {
        v += amp * noise2(x * f, y * f, seed + i as i64 * 17);
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    v / norm
}

fn gradient(stops: &[&str], n: usize) -> Vec<Rgb> {
    let rgb: Vec<Rgb> = stops.iter().map(|s| hex(s)).collect();
    (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64 * (rgb.len() - 1) as f64;
            let j = (t.floor() as usize).min(rgb.len() - 2);
            let c = mix(rgb[j], rgb[j + 1], t - j as f64);
            [c[0].round(), c[1].round(), c[2].round()]
        })
        .collect()
}

// ───────────────────────────── moonlit mausoleum ─────────────────────────────

pub const SCENE_W: usize = 192;
pub const SCENE_H: usize = 108;
const SKY: u8 = 1;
const TREE: u8 = 2;
const BUILD: u8 = 3;
const GROUND: u8 = 4;
const POOL: u8 = 5;
const CX: f64 = 96.0;
const BASE: usize = 62;

struct Pal {
    lit: Rgb, lit2: Rgb, mid: Rgb, soft: Rgb, sh: Rgb, sh2: Rgb, sh3: Rgb,
    arch: Rgb, arch2: Rgb, arch3: Rgb, black: Rgb, tree: Rgb, tree2: Rgb, lawn: Rgb, lawn2: Rgb,
    path: Rgb, path2: Rgb, water: Rgb, water2: Rgb, moon: Rgb, moon2: Rgb, halo: Rgb,
    cloud: Rgb, cloud2: Rgb, bird: Rgb,
}

fn pal() -> Pal {
    Pal {
        lit: hex("#e9eef6"), lit2: hex("#dbe3ef"), mid: hex("#c9d3e4"), soft: hex("#b7c3d8"),
        sh: hex("#94a2bd"), sh2: hex("#7f8eab"), sh3: hex("#6d7c99"),
        arch: hex("#1e2740"), arch2: hex("#2b3756"), arch3: hex("#3c4a6b"), black: hex("#03050a"),
        tree: hex("#03050a"), tree2: hex("#0a0f1c"), lawn: hex("#0a0f1c"), lawn2: hex("#24344e"),
        path: hex("#6a7898"), path2: hex("#a5b2ca"), water: hex("#2a3e70"), water2: hex("#34497c"),
        moon: hex("#fffdf2"), moon2: hex("#f4f6fb"), halo: hex("#8ea6c8"),
        cloud: hex("#4a6098"), cloud2: hex("#5e74a8"), bird: hex("#0a1024"),
    }
}

pub struct Mausoleum {
    rgb: Vec<Rgb>,
    refl: Vec<Rgb>,
    mask: Vec<u8>,
    c: Pal,
}

pub fn mausoleum() -> &'static Mausoleum {
    static SCENE: OnceLock<Mausoleum> = OnceLock::new();
    SCENE.get_or_init(Mausoleum::build)
}

impl Mausoleum {
    fn build() -> Mausoleum {
        let (w, h) = (SCENE_W, SCENE_H);
        let mut s = Mausoleum { rgb: vec![[0.0; 3]; w * h], refl: vec![[0.0; 3]; w * h], mask: vec![0; w * h], c: pal() };
        let c = pal();
        let moon = (150.0, 19.0);
        let mr = 8.5;

        // 1. sky with the moon's halo
        for y in 0..BASE {
            for x in 0..w {
                let v = y as f64 / BASE as f64;
                let mut col = if v < 0.5 {
                    mix(hex("#1a2850"), hex("#2e4680"), v / 0.5)
                } else {
                    mix(hex("#2e4680"), hex("#5a78b0"), (v - 0.5) / 0.5)
                };
                let d = ((x as f64 - moon.0).powi(2) + (y as f64 - moon.1).powi(2)).sqrt();
                col = mix(col, c.halo, clamp01(1.0 - (d - mr) / 30.0).powi(2) * 0.55);
                let px = if d < mr { if d < mr - 1.5 { c.moon } else { c.moon2 } } else { col };
                s.put(x as f64, y as f64, px, SKY);
            }
        }
        // 2. distant tree lines
        for x in 0..w {
            let xf = x as f64;
            let top = 50.0 + 3.0 * fbm(xf / 7.0, 1.0, 3, 4) + if (xf - CX).abs() < 50.0 { 6.0 } else { 0.0 };
            for y in (top.floor() as usize)..BASE {
                s.put(xf, y as f64, if (y as f64) < top + 2.0 { c.tree2 } else { c.tree }, TREE);
            }
        }
        // 3. the mausoleum
        let shade = |x: f64, x0: f64, x1: f64| clamp01((x - x0) / (x1 - x0));
        let marble = |u: f64, n: f64| {
            let v = clamp01(u + n);
            if v < 0.5 { mix(c.sh3, c.soft, v / 0.5) } else { mix(c.soft, c.lit, (v - 0.5) / 0.5) }
        };
        for y in 56..BASE {
            for x in 50..=142 {
                let xf = x as f64;
                let col = if y == 56 { c.lit2 } else { mix(c.sh2, c.mid, 0.3 + 0.5 * shade(xf, 50.0, 142.0)) };
                s.put(xf, y as f64, col, BUILD);
            }
        }
        for y in 33..56 {
            for x in 66..=126 {
                let (xf, yf) = (x as f64, y as f64);
                let corner = x < 75 || x > 117;
                let u = if corner { if x < 75 { 0.45 } else { 0.9 } } else { 0.55 + 0.3 * shade(xf, 75.0, 117.0) };
                s.put(xf, yf, marble(u, (noise2(xf / 3.0, yf / 3.0, 2) - 0.5) * 0.06), BUILD);
            }
        }
        for x in (66..=126).step_by(3) {
            s.put(x as f64, 32.0, c.lit2, BUILD);
        }
        for y in 34..56 {
            for x in 84..=108 {
                s.put(x as f64, y as f64, marble(0.68 + 0.25 * shade(x as f64, 84.0, 108.0), 0.0), BUILD);
            }
        }
        s.arch(87, 105, 37, 55, c.arch2);
        s.arch(91, 101, 45, 55, c.arch);
        for (x0, x1) in [(68, 73), (77, 82), (110, 115), (119, 124)] {
            s.arch(x0, x1, 37, 44, c.arch3);
            s.arch(x0, x1, 47, 55, c.arch2);
        }
        for y in 27..33 {
            for x in 82..=110 {
                s.put(x as f64, y as f64, marble(0.45 + 0.45 * shade(x as f64, 82.0, 110.0), 0.0), BUILD);
            }
        }
        let (r, db, dh) = (16.0, 27.0, 23.0);
        for y in (27 - 23)..=27 {
            let v = (db - y as f64) / dh;
            let mut wd = if v < 0.35 {
                r * (0.84 + 0.16 * ((v / 0.35) * PI / 2.0).sin())
            } else {
                r * (1.0 - ((v - 0.35) / 0.65).powi(2)).max(0.0).sqrt()
            };
            if v > 0.8 {
                wd *= 1.0 - (v - 0.8) * 1.6;
            }
            let mut x = (CX - wd).floor();
            while x <= CX + wd {
                let u = (x - (CX - wd)) / (2.0 * wd + 0.001);
                let lit = 0.42 + 0.5 * u - 0.12 * (1.0 - (PI * u).sin()) + 0.08 * v;
                s.put(x, y as f64, marble(lit, 0.0), BUILD);
                x += 1.0;
            }
        }
        for y in 0..4 {
            s.put(CX, y as f64, c.mid, BUILD);
        }
        s.put(CX - 1.0, 2.0, c.sh, BUILD);
        s.put(CX + 1.0, 2.0, c.lit2, BUILD);
        s.cupola(73.0, 30.0, 4.0, 6.0);
        s.cupola(119.0, 30.0, 4.0, 6.0);
        s.minaret(58.5, 20.0, 56.0, 1.6);
        s.minaret(133.5, 20.0, 56.0, 1.6);

        // 4. terrace, gardens and the pool narrowing into the distance
        let hw = |y: f64| lerp(9.0, 42.0, (y - 64.0) / (h as f64 - 64.0));
        for y in BASE..h {
            for x in 0..w {
                let (xf, yf) = (x as f64, y as f64);
                let d = (xf + 0.5 - CX).abs();
                let hh = if y >= 64 { hw(yf) } else { -1.0 };
                if y < 64 {
                    s.put(xf, yf, if y == BASE { c.path2 } else { c.path }, GROUND);
                } else if d < hh {
                    s.put(xf, yf, c.water, POOL);
                } else if d < hh + 1.5 + (yf - 64.0) * 0.08 {
                    s.put(xf, yf, c.path2, GROUND);
                } else if d < hh + 3.0 + (yf - 64.0) * 0.18 {
                    s.put(xf, yf, c.path, GROUND);
                } else {
                    let t = clamp01(fbm(xf / 6.0, yf / 4.0, 3, 8) * 1.6 - 0.75) * 0.6;
                    s.put(xf, yf, mix(c.lawn, c.lawn2, t), GROUND);
                }
            }
        }
        // cypress rows flanking the pool
        let cyp = hex("#3a5070");
        for k in 1..6 {
            let kf = k as f64;
            let y = 62.0 + kf * kf * 0.8 + kf * 2.0;
            let ht = 3.0 + (y - 62.0) * 0.62;
            let wd = ht * 0.22;
            for side in [-1.0, 1.0] {
                let cx = CX + side * (hw(y.max(64.0)) + 8.0 + (y - 64.0) * 0.4);
                let mut yy = (y - ht).floor();
                while yy <= y {
                    let v = (y - yy) / ht;
                    let ww = wd
                        * (PI * ((1.0 - v) * 0.9 + 0.1).min(1.0)).sin()
                        * if v > 0.85 { (1.0 - v) / 0.15 } else { 1.0 };
                    let mut x = (cx - ww).floor();
                    while x <= cx + ww {
                        s.put(x, yy, if x > cx { cyp } else { c.black }, TREE);
                        x += 1.0;
                    }
                    yy += 1.0;
                }
            }
        }
        // 5. mirrored image in the pool
        for y in 64..h {
            for x in 0..w {
                let i = y * w + x;
                if s.mask[i] != POOL {
                    continue;
                }
                let sy = 2 * BASE as i64 - 1 - y as i64 + 6;
                let mut col = mix(c.water, c.water2, (y as f64 - 64.0) / (h as f64 - 64.0));
                if sy >= 0 {
                    let j = sy as usize * w + x;
                    match s.mask[j] {
                        BUILD => col = mix(s.rgb[j], c.water, 0.3),
                        TREE => col = c.tree,
                        SKY => col = mix(s.rgb[j], c.water, 0.4),
                        _ => {}
                    }
                }
                s.refl[i] = col;
                s.put(x as f64, y as f64, col, POOL);
            }
        }
        s
    }

    fn put(&mut self, x: f64, y: f64, c: Rgb, m: u8) {
        let (x, y) = (jround(x), jround(y));
        if x < 0 || y < 0 || x >= SCENE_W as i64 || y >= SCENE_H as i64 {
            return;
        }
        let i = y as usize * SCENE_W + x as usize;
        self.rgb[i] = c;
        self.mask[i] = m;
    }

    fn arch(&mut self, x0: i64, x1: i64, y0: i64, y1: i64, col: Rgb) {
        let cx = (x0 + x1) as f64 / 2.0;
        let hw = (x1 - x0) as f64 / 2.0;
        let ah = hw * 1.1;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dy = (y - y0) as f64;
                if dy < ah {
                    let k = 1.0 - dy / ah;
                    if (x as f64 - cx).abs() > hw * (1.0 - k * k).sqrt() * 1.02 - 0.2 {
                        continue;
                    }
                }
                self.put(x as f64, y as f64, col, BUILD);
            }
        }
    }

    fn marble(&self, u: f64) -> Rgb {
        let v = clamp01(u);
        if v < 0.5 { mix(self.c.sh3, self.c.soft, v / 0.5) } else { mix(self.c.soft, self.c.lit, (v - 0.5) / 0.5) }
    }

    fn cupola(&mut self, cx: f64, base: f64, r: f64, h: f64) {
        for y in (base as i64)..(base as i64 + 3) {
            self.put(cx - r + 1.0, y as f64, self.c.sh, BUILD);
            self.put(cx + r - 1.0, y as f64, self.c.lit2, BUILD);
        }
        let mut y = base - h;
        while y < base {
            let v = (base - y) / h;
            let w = r * (1.0 - v * v).max(0.0).sqrt() * if v > 0.7 { 1.0 - (v - 0.7) * 1.5 } else { 1.0 };
            let mut x = (cx - w).floor();
            while x <= cx + w {
                let col = self.marble(0.3 + 0.7 * (x - cx + w) / (2.0 * w + 0.01));
                self.put(x, y, col, BUILD);
                x += 1.0;
            }
            y += 1.0;
        }
        self.put(cx, base - h - 1.0, self.c.mid, BUILD);
    }

    fn minaret(&mut self, cx: f64, top: f64, bottom: f64, w: f64) {
        let mut y = top;
        while y < bottom {
            let ww = w + (y - top) / (bottom - top) * 0.6;
            let mut x = (cx - ww).floor();
            while x <= cx + ww {
                let col = if x < cx { self.c.sh } else if x == cx.floor() { self.c.mid } else { self.c.lit2 };
                self.put(x, y, col, BUILD);
                x += 1.0;
            }
            y += 1.0;
        }
        for f in [0.28, 0.55, 0.8] {
            let y = jround(lerp(top, bottom, f)) as f64;
            let mut x = (cx - w - 1.5).floor();
            while x <= cx + w + 1.5 {
                self.put(x, y, self.c.lit, BUILD);
                self.put(x, y + 1.0, self.c.sh3, BUILD);
                x += 1.0;
            }
        }
        self.cupola(jround(cx) as f64, top, jround(w + 1.5) as f64, 4.0);
    }

    /// One animation frame at time `t` seconds: SCENE_W x SCENE_H pixels.
    pub fn frame(&self, t: f64) -> Vec<Rgb> {
        let (w, h) = (SCENE_W, SCENE_H);
        let c = &self.c;
        let mut out = self.rgb.clone();
        // ripples rolling down the pool, bending the reflection
        for y in 64..h {
            let ph = 70.0 / (y as f64 - 58.0) - t * 0.9;
            let s = (ph * 6.28).sin();
            let dx = jround(s * (0.4 + (y as f64 - 64.0) * 0.035));
            let dark = if s > 0.8 { 0.45 } else { 0.0 };
            for x in 0..w {
                let i = y * w + x;
                if self.mask[i] != POOL {
                    continue;
                }
                let sx = x as i64 + dx;
                let sx = if sx >= 0 && (sx as usize) < w && self.mask[y * w + sx as usize] == POOL { sx as usize } else { x };
                out[i] = mix(self.refl[y * w + sx], c.water, dark);
            }
        }
        let add = |out: &mut Vec<Rgb>, x: i64, y: i64, col: Rgb, a: f64| {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return;
            }
            let i = y as usize * w + x as usize;
            if self.mask[i] != SKY {
                return;
            }
            out[i] = mix(out[i], col, a);
        };
        // thin clouds drifting across the sky
        for (u, l, cy, th, col, sp) in [
            (0.1f64, 50.0f64, 14.0f64, 1.4f64, c.cloud2, 0.9f64),
            (0.6, 70.0, 34.0, 1.1, c.cloud, 0.6),
            (0.35, 40.0, 8.0, 0.9, c.cloud, 0.5),
        ] {
            let span = w as f64 + l * 2.0;
            let cx = (((u * w as f64 + t * sp * 1.4) % span) + span) % span - l;
            let mut y = (cy - th * 3.0).floor();
            while y <= cy + th * 3.0 {
                let mut x = (cx - l / 2.0).floor();
                while x < cx + l / 2.0 {
                    let along = 1.0 - ((x - cx) / (l / 2.0)).abs();
                    let wob = (x * 0.13 + cy).sin() * th;
                    let a = clamp01(along * 2.0) * clamp01(1.0 - (y - cy - wob).abs() / (th * 1.5 * (0.4 + along)));
                    if a > 0.05 {
                        add(&mut out, x as i64, y as i64, col, a * 0.75);
                    }
                    x += 1.0;
                }
                y += 1.0;
            }
        }
        // birds crossing in front of the moon every 20 s
        let bt = (t % 20.0) / 20.0;
        let fx = lerp(-30.0, w as f64 + 10.0, bt);
        let fy = 24.0 - bt * 10.0;
        for (dx, dy, ph) in [(0.0, 0.0, 0.0), (6.0, 2.0, 1.1), (11.0, -1.0, 2.3), (17.0, 3.0, 0.6), (23.0, 1.0, 1.7)] {
            let x = jround(fx + dx);
            let y = jround(fy + dy + (t * 1.6 + ph).sin() * 0.8);
            let up = if (t * 8.0 + ph).sin() > 0.0 { -1 } else { 0 };
            add(&mut out, x, y, c.bird, 1.0);
            add(&mut out, x - 1, y + up, c.bird, 1.0);
            add(&mut out, x + 1, y + up, c.bird, 1.0);
        }
        out
    }
}

// ───────────────────────────── gear + wrench ─────────────────────────────

pub const GEAR_W: usize = 48;
pub const GEAR_H: usize = 25;

/// Gear and wrench from maintenance-mode, as (char, color) cells; ' ' means empty.
pub fn gear_frame(t: f64) -> Vec<(char, Rgb)> {
    static PALS: OnceLock<(Vec<Rgb>, Vec<Rgb>)> = OnceLock::new();
    let (gear, steel) = PALS.get_or_init(|| {
        (
            gradient(&["#7a3e0c", "#d9822b", "#ffa657", "#ffd8a8"], 8),
            gradient(&["#484f58", "#8b949e", "#c9d1d9", "#ffffff"], 6),
        )
    });
    // source rows 1..=24 map to our rows 0..=23 (hazard stripes dropped)
    const H: i64 = 26;
    let w = GEAR_W as i64;
    let mut cells = vec![(' ', [0.0; 3]); GEAR_W * GEAR_H];
    let mut set = |x: i64, y: i64, ch: char, col: Rgb| {
        let y = y - 1;
        if x >= 0 && x < w && y >= 0 && y < GEAR_H as i64 {
            cells[(y * w + x) as usize] = (ch, col);
        }
    };
    let (gx, gy) = (17.0, 13.0);
    let rot = t * 0.9;
    let ang = 1.68 + (t * 2.2).sin() * 0.12;
    let (ca, sa) = (ang.cos(), ang.sin());
    let (px, py) = (39.0, 5.0);
    let mut mask = vec![0u8; (w * H) as usize];
    for y in 1..H - 1 {
        for x in 28..48 {
            let dx = (x as f64 - px) * 0.5;
            let dy = y as f64 - py;
            let u = dx * ca + dy * sa;
            let v = -dx * sa + dy * ca;
            let mut e = -1.0;
            if u > 1.8 && u < 13.5 && v.abs() < 1.0 {
                e = v.abs();
            }
            let hr = u.hypot(v);
            if hr < 3.0 && !(u < 0.4 && v.abs() < 1.2) {
                e = hr / 3.0;
            }
            let tr = (u - 14.6).hypot(v);
            if tr < 2.0 && tr > 0.9 {
                e = tr / 2.0;
            }
            if e < 0.0 {
                continue;
            }
            let k = (4.6 - e * 2.6 + if v < 0.0 { 0.8 } else { -0.8 }).round().clamp(0.0, 5.0) as u8;
            mask[(x + y * w) as usize] = 1 + k;
        }
    }
    let masked = |x: i64, y: i64| x >= 0 && x < w && y >= 0 && y < H && mask[(x + y * w) as usize] != 0;
    for y in (gy as i64 - 9)..=(gy as i64 + 9) {
        for x in (gx as i64 - 18)..=(gx as i64 + 18) {
            if x < 0 || y < 1 || y >= H - 1 {
                continue;
            }
            if (-1..=1).any(|oy| (-1..=1).any(|ox| masked(x + ox, y + oy))) {
                continue;
            }
            let dx = (x as f64 - gx) * 0.5;
            let dy = y as f64 - gy;
            let rr = dx.hypot(dy);
            if rr > 8.4 {
                continue;
            }
            let a = dy.atan2(dx) - rot;
            let tooth = if (a * 8.0).cos() > 0.1 { 8.2 } else { 6.5 };
            if rr > tooth || rr < 2.0 {
                continue;
            }
            let lit = (-dx * 0.6 - dy * 0.8) / (rr + 0.01);
            let k = ((0.55 + 0.4 * lit - if rr < 2.8 { 0.15 } else { 0.0 }) * 8.0).floor().clamp(0.0, 7.0) as usize;
            let ring = rr > 4.1 && rr < 4.9;
            set(x, y, if ring { '▓' } else { '█' }, gear[if ring { k.saturating_sub(3) } else { k }]);
        }
    }
    for y in 1..H - 1 {
        for x in 28..48 {
            let m = mask[(x + y * w) as usize];
            if m != 0 {
                set(x, y, if m > 4 { '█' } else if m > 2 { '▓' } else { '▒' }, steel[m as usize - 1]);
            }
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_matches_kit_js_range_and_determinism() {
        let a = noise2(3.7, 1.2, 4);
        assert!((0.0..=1.0).contains(&a));
        assert_eq!(a, noise2(3.7, 1.2, 4));
        let f = fbm(10.0 / 7.0, 1.0, 3, 4);
        assert!((0.0..=1.0).contains(&f));
    }

    #[test]
    fn scene_builds_and_animates() {
        let m = mausoleum();
        let f0 = m.frame(0.0);
        let f1 = m.frame(3.3);
        assert_eq!(f0.len(), SCENE_W * SCENE_H);
        assert_ne!(f0, f1);
        // moon centre is near-white
        assert!(f0[19 * SCENE_W + 150][0] > 240.0);
    }

    #[test]
    fn gear_has_both_parts() {
        let g = gear_frame(1.0);
        assert!(g.iter().filter(|c| c.0 != ' ').count() > 200);
        assert!(g.iter().any(|c| c.0 == '▒' || c.0 == '▓'));
    }
}
