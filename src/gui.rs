//! The app in its own window (eframe/egui). The ratatui UI is drawn into an in-memory grid
//! (`TestBackend`) and painted cell by cell: block and box-drawing characters as exact
//! rectangles (no seams), everything else as text in a monospace system font.
//!
//! The screener logic runs on its own thread, so fetching and alerts go on while the window
//! is minimized; the window locks the app only to pass input and draw.

use crate::app::{App, Msg, APP_NAME};
use crate::launch::FRAME;
use crate::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Vec2};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

const FONT_MIN: f32 = 8.0;
const FONT_MAX: f32 = 40.0;
/// What `Color::Reset` stands for (the Windows Terminal defaults).
const DEFAULT_FG: Color32 = Color32::from_rgb(204, 204, 204);
const DEFAULT_BG: Color32 = Color32::from_rgb(12, 12, 12);

fn lock(app: &Mutex<App>) -> MutexGuard<'_, App> {
    app.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn run(started: Result<(App, Receiver<Msg>), String>) {
    let icon = egui::IconData { rgba: include_bytes!("../assets/icon64.rgba").to_vec(), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([300.0, 200.0])
            // the title bar is drawn in the app's own style (`title_bar`)
            .with_decorations(false)
            .with_icon(icon),
        ..Default::default()
    };
    let (app, rx) = match started {
        Ok(s) => s,
        Err(e) => {
            // the error window keeps the system frame, so it can be closed
            let options = eframe::NativeOptions { viewport: options.viewport.with_decorations(true), ..options };
            let _ = eframe::run_native(APP_NAME, options, Box::new(|_| Ok(Box::new(ErrorWindow(e)))));
            return;
        }
    };
    let font_size = app.cfg.font_size.clamp(FONT_MIN, FONT_MAX);
    let mut app = app;
    app.pixel_chart = true;
    let app = Arc::new(Mutex::new(app));

    // Screener logic: messages from the workers and the tick, whether or not the window draws.
    let logic = app.clone();
    std::thread::spawn(move || loop {
        {
            let mut app = lock(&logic);
            while let Ok(msg) = rx.try_recv() {
                app.on_msg(msg);
            }
            app.on_tick(Instant::now());
        }
        std::thread::sleep(FRAME);
    });

    let _ = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(fonts());
            // Ctrl +/- change the text size (kept in the config) instead of egui's zoom.
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            Ok(Box::new(Window {
                app,
                term: Terminal::new(TestBackend::new(80, 24)).expect("in-memory backend"),
                font_size,
                origin: Pos2::ZERO,
                cell: Vec2::new(8.0, 16.0),
                mouse_down: false,
                last_cell: None,
                hover: None,
                wheel: 0.0,
                icon: None,
            }))
        }),
    );
}

/// Startup failed: the window shows why.
struct ErrorWindow(String);

impl eframe::App for ErrorWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(egui::RichText::new(&self.0).color(Color32::LIGHT_RED).size(16.0));
        });
    }
}

/// Cascadia Mono, else Consolas, then egui's own monospace; Segoe UI Symbol for symbols the
/// others lack.
fn fonts() -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::default();
    let dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    let mut loaded = |name: &str, file: &str| -> Option<String> {
        let bytes = std::fs::read(dir.join(file)).ok()?;
        defs.font_data.insert(name.to_string(), Arc::new(egui::FontData::from_owned(bytes)));
        Some(name.to_string())
    };
    let main = loaded("cascadia", "CascadiaMono.ttf").or_else(|| loaded("consolas", "consola.ttf"));
    let symbols = loaded("symbols", "seguisym.ttf");
    let mono = defs.families.entry(FontFamily::Monospace).or_default();
    if let Some(m) = main {
        mono.insert(0, m);
    }
    mono.extend(symbols);
    defs
}

struct Window {
    app: Arc<Mutex<App>>,
    term: Terminal<TestBackend>,
    font_size: f32,
    /// Top-left of the grid and the size of one cell, in points (from the last frame): the
    /// glyph width by the row pitch (grid height / rows, at least a text row).
    origin: Pos2,
    cell: Vec2,
    mouse_down: bool,
    last_cell: Option<(u16, u16)>,
    /// Cell under the pointer.
    hover: Option<(u16, u16)>,
    /// Wheel movement not yet turned into whole scroll steps, in lines.
    wheel: f32,
    /// The window icon, for the title bar.
    icon: Option<egui::TextureHandle>,
}

impl eframe::App for Window {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // egui may run a frame twice with the same input; act on it once.
        let events = if ctx.current_pass_index() == 0 { ctx.input(|i| i.events.clone()) } else { Vec::new() };
        let font = FontId::monospace(self.font_size);
        let (cw, ch) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));

        let shared = self.app.clone();
        let mut app = lock(&shared);
        for ev in events {
            if let Some(size) = handle(ev, &mut app, self) {
                self.font_size = size;
                app.set_font_size(size);
            }
        }
        if app.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // the boot screen keeps the original colours, the window chrome with it
        let pal = if app.view == crate::app::View::Boot { crate::theme::NOIR } else { app.palette };
        let bg = match pal.bg {
            Color::Reset => DEFAULT_BG,
            c => color(c, DEFAULT_BG),
        };
        let bar_bg = match pal.zebra {
            Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
            _ => Color32::from_rgb(24, 24, 24),
        };
        let fg = color(pal.text, DEFAULT_FG);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(bg)).show(ctx, |ui| {
            let window = ui.max_rect();
            // a thin title bar; the rows share the rest, so the grid fills the window exactly
            let bar_h = (0.6 * ch + 4.0).round().max(18.0);
            let mut area = window;
            area.min.y += bar_h;
            let rows = ((area.height() / ch).floor() as u16).max(1);
            self.cell = Vec2::new(cw, (area.height() / f32::from(rows)).max(ch));
            self.origin = area.min;
            let cols = ((area.width() / cw).floor() as u16).max(1);
            let size = self.term.backend().buffer().area;
            if (size.width, size.height) != (cols, rows) {
                self.term.backend_mut().resize(cols, rows);
            }
            let _ = self.term.draw(|f| ui::draw(f, &mut app));
            let ppp = ctx.pixels_per_point();
            // halftone dots under the cells (the cells over them are blank)
            for (r, ht) in &app.halftones {
                paint_halftone(ui.painter(), *r, ht, area.min, self.cell);
            }
            paint(ui.painter(), self.term.backend().buffer(), area.min, self.cell, ch, ppp, &font);
            if let Some(plot) = &app.plot {
                paint_plot(ui.painter(), plot, area.min, self.cell, ppp);
            }
            paint_bubbles(ui.painter(), &app.bubbles, area.min, self.cell, &font);
            let icon = self.icon.get_or_insert_with(|| {
                let img = egui::ColorImage::from_rgba_unmultiplied([64, 64], include_bytes!("../assets/icon64.rgba"));
                ctx.load_texture("icon", img, egui::TextureOptions::LINEAR)
            });
            // resize handles first, so the title bar buttons (registered later) win their clicks
            resize_edges(ui, window);
            title_bar(ui, Rect::from_min_max(window.min, Pos2::new(window.max.x, window.min.y + bar_h)), bar_bg, fg, &font, icon);
        });
        drop(app);
        ctx.request_repaint_after(FRAME);
    }
}

/// The window's title bar in the app's style: icon and name; drag moves the window,
/// double-click maximizes; minimize, maximize and close buttons on the right.
fn title_bar(ui: &mut egui::Ui, bar: Rect, bg: Color32, fg: Color32, font: &FontId, icon: &egui::TextureHandle) {
    let ctx = ui.ctx().clone();
    let painter = ui.painter();
    painter.rect_filled(bar, 0.0, bg);
    let pad = 4.0;
    // icon and name a little smaller than the bar, centred in it
    let side = (bar.height() - pad).round();
    let font = &FontId::monospace((font.size * 0.6).max(FONT_MIN));
    let icon_r = Rect::from_min_size(Pos2::new(bar.min.x + pad + 2.0, (bar.center().y - side / 2.0).round()), Vec2::splat(side));
    painter.image(icon.id(), icon_r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    painter.text(Pos2::new(icon_r.max.x + 8.0, bar.center().y), Align2::LEFT_CENTER, APP_NAME, font.clone(), fg);

    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let btn_w = (bar.height() * 2.0).round();
    let buttons = [
        ("─", egui::ViewportCommand::Minimized(true)),
        (if maximized { "❐" } else { "□" }, egui::ViewportCommand::Maximized(!maximized)),
        ("✕", egui::ViewportCommand::Close),
    ];
    let mut right = bar.max.x;
    let mut drag_area = bar;
    for (i, (glyph, cmd)) in buttons.into_iter().enumerate().rev() {
        let r = Rect::from_min_max(Pos2::new(right - btn_w, bar.min.y), Pos2::new(right, bar.max.y));
        right -= btn_w;
        let resp = ui.interact(r, ui.id().with(("title-btn", i)), egui::Sense::click());
        if resp.hovered() {
            let hover = if i == 2 { Color32::from_rgb(196, 43, 28) } else { fg.gamma_multiply(0.15) };
            ui.painter().rect_filled(r, 0.0, hover);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, font.clone(), fg);
        if resp.clicked() {
            ctx.send_viewport_cmd(cmd);
        }
    }
    drag_area.max.x = right;
    let resp = ui.interact(drag_area, ui.id().with("title-drag"), egui::Sense::click_and_drag());
    if resp.double_clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if resp.drag_started() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Resize handles along the window's edges and corners (the window has no system frame).
fn resize_edges(ui: &mut egui::Ui, w: Rect) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let e = 5.0;
    let (l, r, t, b) = (w.min.x, w.max.x, w.min.y, w.max.y);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
    let edges = [
        (rect(l, t, l + e, t + e), D::NorthWest, C::ResizeNorthWest),
        (rect(r - e, t, r, t + e), D::NorthEast, C::ResizeNorthEast),
        (rect(l, b - e, l + e, b), D::SouthWest, C::ResizeSouthWest),
        (rect(r - e, b - e, r, b), D::SouthEast, C::ResizeSouthEast),
        (rect(l + e, t, r - e, t + e), D::North, C::ResizeNorth),
        (rect(l + e, b - e, r - e, b), D::South, C::ResizeSouth),
        (rect(l, t + e, l + e, b - e), D::West, C::ResizeWest),
        (rect(r - e, t + e, r, b - e), D::East, C::ResizeEast),
    ];
    for (i, (area, dir, cursor)) in edges.into_iter().enumerate() {
        let resp = ui.interact(area, ui.id().with(("resize", i)), egui::Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
}

/// Candles, wicks and volume of the chart at pixel resolution, over the cells the chart left
/// empty for them.
fn paint_plot(painter: &egui::Painter, plot: &crate::chart::Plot, origin: Pos2, cell: Vec2, ppp: f32) {
    let snap = |v: f32| (v * ppp).round() / ppp;
    let px = 1.0 / ppp;
    let range = (plot.hi - plot.lo).max(f64::EPSILON);
    let y_of = |v: f64| origin.y + (f64::from(plot.top) + 0.5 + (plot.hi - v) / range * plot.span) as f32 * cell.y;
    let top = origin.y + f32::from(plot.top) * cell.y;
    let bottom = origin.y + (f32::from(plot.top) + plot.span as f32 + 1.0) * cell.y;
    let left = origin.x + f32::from(plot.x) * cell.x;
    let slot = f32::from(plot.step) * cell.x;
    let clip = Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + slot * plot.candles.len() as f32, bottom));
    let price = painter.with_clip_rect(clip);
    let body_w = snap((slot * 0.7).max(px)).max(px);
    let wick_w = px.max(snap(cell.x * 0.1));
    let vol_top = origin.y + f32::from(plot.vol_row) * cell.y;
    let vol_h = 2.0 * cell.y;
    for (i, c) in plot.candles.iter().enumerate() {
        let col = color(if c.close >= c.open { plot.up } else { plot.down }, DEFAULT_FG);
        let cx = snap(left + slot * i as f32 + slot / 2.0);
        let (yh, yl) = (snap(y_of(c.high)), snap(y_of(c.low)));
        price.rect_filled(Rect::from_min_max(Pos2::new(cx - wick_w / 2.0, yh), Pos2::new(cx + wick_w / 2.0, yl.max(yh + px))), 0.0, col);
        let (yt, yb) = (snap(y_of(c.open.max(c.close))), snap(y_of(c.open.min(c.close))));
        let body = Rect::from_min_max(Pos2::new(cx - body_w / 2.0, yt), Pos2::new(cx + body_w / 2.0, yb.max(yt + px)));
        price.rect_filled(body, 0.0, col);
        if c.volume > 0 {
            let h = snap((c.volume as f64 / plot.max_vol) as f32 * vol_h).max(px);
            let r = Rect::from_min_max(Pos2::new(cx - body_w / 2.0, vol_top + vol_h - h), Pos2::new(cx + body_w / 2.0, vol_top + vol_h));
            painter.rect_filled(r, 0.0, col.gamma_multiply(0.6));
        }
    }
}

/// Bubbles of the Bubbles layout over the cells (blank there): radius in columns, scaled by the
/// smaller of a column and half a row so neighbours never touch; the label lines centred.
fn paint_bubbles(painter: &egui::Painter, bubbles: &[crate::bubbles::Painted], origin: Pos2, cell: Vec2, font: &FontId) {
    let unit = cell.x.min(cell.y / 2.0);
    for b in bubbles {
        let centre = origin + Vec2::new(b.cx * cell.x, b.cy * cell.y);
        painter.circle_filled(centre, b.r * unit, color(b.fill, DEFAULT_FG));
        let fg = color(b.text, DEFAULT_FG);
        for (i, s) in b.lines.iter().enumerate() {
            let pos = centre + Vec2::new(0.0, (i as f32 - 1.0) * cell.y);
            painter.text(pos, Align2::CENTER_CENTER, s, font.clone(), fg);
        }
    }
}

/// Halftone dots over the cells of `area`: anti-aliased circles, two per cell (square pitch),
/// radius sqrt(coverage) times half the pitch.
fn paint_halftone(painter: &egui::Painter, area: ratatui::layout::Rect, ht: &crate::art::Halftone, origin: Pos2, cell: Vec2) {
    let rgb = |c: crate::art::Rgb| Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8);
    let top_left = origin + Vec2::new(f32::from(area.x) * cell.x, f32::from(area.y) * cell.y);
    if let Some(bg) = ht.bg {
        let size = Vec2::new(f32::from(area.width) * cell.x, f32::from(area.height) * cell.y);
        painter.rect_filled(Rect::from_min_size(top_left, size), 0.0, rgb(bg));
    }
    let pitch = Vec2::new(cell.x, cell.y / 2.0);
    let r_max = pitch.x.min(pitch.y) / 2.0;
    for y in 0..ht.h {
        for x in 0..ht.w {
            let (c, k) = ht.dots[y * ht.w + x];
            let r = k.sqrt() as f32 * r_max;
            if r >= 0.3 {
                let centre = top_left + Vec2::new((x as f32 + 0.5) * pitch.x, (y as f32 + 0.5) * pitch.y);
                painter.circle_filled(centre, r, rgb(c));
            }
        }
    }
}

/// Cell under a window position, if inside the grid.
fn cell_at(w: &Window, pos: Pos2, buf: &Buffer) -> Option<(u16, u16)> {
    let rel = pos - w.origin;
    if rel.x < 0.0 || rel.y < 0.0 {
        return None;
    }
    let (x, y) = ((rel.x / w.cell.x) as u16, (rel.y / w.cell.y) as u16);
    (x < buf.area.width && y < buf.area.height).then_some((x, y))
}

fn mouse(kind: MouseEventKind, (x, y): (u16, u16), modifiers: KeyModifiers) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers }
}

/// Passes one window event to the app as the key or mouse event the terminal UI gets.
/// Returns a new text size when the event asks for one.
fn handle(ev: egui::Event, app: &mut App, w: &mut Window) -> Option<f32> {
    let buf = w.term.backend().buffer().clone();
    match ev {
        egui::Event::Text(s) => {
            for c in s.chars().filter(|c| !c.is_control()) {
                app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            }
        }
        egui::Event::Key { key, pressed: true, modifiers, .. } => {
            if modifiers.ctrl {
                match key {
                    egui::Key::Plus | egui::Key::Equals => return Some((w.font_size + 1.0).min(FONT_MAX)),
                    egui::Key::Minus => return Some((w.font_size - 1.0).max(FONT_MIN)),
                    egui::Key::Num0 => return Some(15.0),
                    _ => {}
                }
            }
            if let Some(k) = key_event(key, modifiers) {
                app.on_key(k);
            }
        }
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } => {
            w.mouse_down = pressed;
            let cell = cell_at(w, pos, &buf);
            w.last_cell = cell;
            if let Some(c) = cell {
                let kind = if pressed { MouseEventKind::Down(MouseButton::Left) } else { MouseEventKind::Up(MouseButton::Left) };
                app.on_mouse(mouse(kind, c, KeyModifiers::NONE));
            } else if !pressed {
                app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), (0, 0), KeyModifiers::NONE));
            }
        }
        egui::Event::PointerMoved(pos) => {
            let cell = cell_at(w, pos, &buf);
            // the wheel acts where the pointer is (it zooms only over the chart)
            w.hover = cell;
            if w.mouse_down && cell.is_some() && cell != w.last_cell {
                w.last_cell = cell;
                app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), cell?, KeyModifiers::NONE));
            }
        }
        egui::Event::MouseWheel { unit, delta, modifiers, .. } => {
            let lines = match unit {
                egui::MouseWheelUnit::Point => delta.y / w.cell.y,
                egui::MouseWheelUnit::Line => delta.y,
                egui::MouseWheelUnit::Page => delta.y * 10.0,
            };
            // a horizontal scroll has no vertical part (and 0.0.signum() is 1.0)
            if lines == 0.0 {
                return None;
            }
            if modifiers.ctrl {
                return Some((w.font_size + lines.signum()).clamp(FONT_MIN, FONT_MAX));
            }
            w.wheel += lines;
            let Some(pos) = w.hover else {
                w.wheel = 0.0;
                return None;
            };
            while w.wheel.abs() >= 1.0 {
                // positive delta moves the content down: the list scrolls up
                let kind = if w.wheel > 0.0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown };
                app.on_mouse(mouse(kind, pos, KeyModifiers::NONE));
                w.wheel -= w.wheel.signum();
            }
        }
        _ => {}
    }
    None
}

/// Non-character keys, and Ctrl+letter (Ctrl+S saves the config).
fn key_event(key: egui::Key, m: egui::Modifiers) -> Option<KeyEvent> {
    use egui::Key as K;
    let code = match key {
        K::ArrowUp => KeyCode::Up,
        K::ArrowDown => KeyCode::Down,
        K::ArrowLeft => KeyCode::Left,
        K::ArrowRight => KeyCode::Right,
        K::Enter => KeyCode::Enter,
        K::Escape => KeyCode::Esc,
        K::Tab if m.shift => KeyCode::BackTab,
        K::Tab => KeyCode::Tab,
        K::Backspace => KeyCode::Backspace,
        K::Delete => KeyCode::Delete,
        K::Home => KeyCode::Home,
        K::End => KeyCode::End,
        K::PageUp => KeyCode::PageUp,
        K::PageDown => KeyCode::PageDown,
        // trace mode's debug overlay
        K::F12 => KeyCode::F(12),
        k if m.ctrl => {
            let name = k.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => {
                    return Some(KeyEvent::new(KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL));
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let mods = if m.shift { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
    Some(KeyEvent::new(code, mods))
}

fn color(c: Color, default: Color32) -> Color32 {
    let rgb = Color32::from_rgb;
    match c {
        Color::Reset => default,
        Color::Black => rgb(12, 12, 12),
        Color::Red => rgb(197, 15, 31),
        Color::Green => rgb(19, 161, 14),
        Color::Yellow => rgb(193, 156, 0),
        Color::Blue => rgb(0, 55, 218),
        Color::Magenta => rgb(136, 23, 152),
        Color::Cyan => rgb(58, 150, 221),
        Color::Gray => rgb(204, 204, 204),
        Color::DarkGray => rgb(118, 118, 118),
        Color::LightRed => rgb(231, 72, 86),
        Color::LightGreen => rgb(22, 198, 12),
        Color::LightYellow => rgb(249, 241, 165),
        Color::LightBlue => rgb(59, 120, 255),
        Color::LightMagenta => rgb(180, 0, 158),
        Color::LightCyan => rgb(97, 214, 214),
        Color::White => rgb(242, 242, 242),
        Color::Rgb(r, g, b) => rgb(r, g, b),
        Color::Indexed(i) => indexed(i),
    }
}

/// xterm 256-colour palette.
fn indexed(i: u8) -> Color32 {
    const BASIC: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    match i {
        0..=15 => color(BASIC[i as usize], DEFAULT_FG),
        16..=231 => {
            let v = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            let n = i - 16;
            Color32::from_rgb(v(n / 36), v(n / 6 % 6), v(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
    }
}

/// Lines of a box-drawing character: (left, right, up, down, heavy).
fn box_lines(s: &str) -> Option<(bool, bool, bool, bool, bool)> {
    Some(match s {
        "─" => (true, true, false, false, false),
        "━" => (true, true, false, false, true),
        "│" => (false, false, true, true, false),
        "┃" => (false, false, true, true, true),
        "┌" | "╭" => (false, true, false, true, false),
        "┐" | "╮" => (true, false, false, true, false),
        "└" | "╰" => (false, true, true, false, false),
        "┘" | "╯" => (true, false, true, false, false),
        "├" => (false, true, true, true, false),
        "┤" => (true, false, true, true, false),
        "┬" => (true, true, false, true, false),
        "┴" => (true, true, true, false, false),
        "┼" => (true, true, true, true, false),
        _ => return None,
    })
}

/// The grid's cells; text (`text_h` = the font's row height) is centred in the row pitch.
fn paint(painter: &egui::Painter, buf: &Buffer, origin: Pos2, cell: Vec2, text_h: f32, ppp: f32, font: &FontId) {
    // Cell edges snapped to device pixels, so neighbouring cells meet without gaps.
    let snap = |v: f32| (v * ppp).round() / ppp;
    let text_dy = snap((cell.y - text_h).max(0.0) / 2.0);
    let px = 1.0 / ppp;
    let x_at = |c: u16| snap(origin.x + c as f32 * cell.x);
    let y_at = |r: u16| snap(origin.y + r as f32 * cell.y);
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            let rect = Rect::from_min_max(Pos2::new(x_at(x), y_at(y)), Pos2::new(x_at(x + 1), y_at(y + 1)));
            let mut fg = color(c.fg, DEFAULT_FG);
            let mut bg = color(c.bg, Color32::TRANSPARENT);
            if c.modifier.contains(Modifier::REVERSED) {
                let back = if bg == Color32::TRANSPARENT { DEFAULT_BG } else { bg };
                (fg, bg) = (back, fg);
            }
            if c.modifier.contains(Modifier::DIM) {
                let under = if bg == Color32::TRANSPARENT { DEFAULT_BG } else { bg };
                fg = fg.lerp_to_gamma(under, 0.5);
            }
            if bg != Color32::TRANSPARENT {
                painter.rect_filled(rect, 0.0, bg);
            }
            let sym = c.symbol();
            if sym == " " || sym.is_empty() {
                continue;
            }
            let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
            let mid_y = snap((t + b) / 2.0);
            let mid_x = snap((l + r) / 2.0);
            let block = |x0: f32, y0: f32, x1: f32, y1: f32| {
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)), 0.0, fg);
            };
            match sym {
                "█" => block(l, t, r, b),
                "▀" => block(l, t, r, mid_y),
                "▄" => block(l, mid_y, r, b),
                "▌" => block(l, t, mid_x, b),
                "▐" => block(mid_x, t, r, b),
                s => {
                    if let Some((left, right, up, down, heavy)) = box_lines(s) {
                        let th = (if heavy { 2.0 } else { 1.0 }) * (cell.y / 14.0).round().max(1.0) * px;
                        let (hy0, hy1) = (mid_y - snap(th / 2.0), mid_y - snap(th / 2.0) + th);
                        let (vx0, vx1) = (mid_x - snap(th / 2.0), mid_x - snap(th / 2.0) + th);
                        if left {
                            block(l, hy0, vx1, hy1);
                        }
                        if right {
                            block(vx0, hy0, r, hy1);
                        }
                        if up {
                            block(vx0, t, vx1, hy1);
                        }
                        if down {
                            block(vx0, hy0, vx1, b);
                        }
                        continue;
                    }
                    let pos = Pos2::new(l, t + text_dy);
                    painter.text(pos, Align2::LEFT_TOP, s, font.clone(), fg);
                    if c.modifier.contains(Modifier::BOLD) {
                        painter.text(pos + Vec2::new(px.max(cell.x * 0.06), 0.0), Align2::LEFT_TOP, s, font.clone(), fg);
                    }
                }
            }
        }
    }
}
