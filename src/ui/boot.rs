//! The start screen: the menu and connection checks over the halftone scene, and the halftone
//! drawing shared with the Config sidebar's gear.

use crate::app::{App, APP_NAME, BOOT_ITEMS};
use crate::theme::pal;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

pub(super) fn boot(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let t = app.started_at.elapsed().as_secs_f64();
    scene(f, app, area, t);

    let boxed = |title: &str| {
        let b = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Rgb(142, 166, 200)))
            .style(Style::new().bg(Color::Rgb(10, 16, 36)));
        if title.is_empty() {
            b
        } else {
            b.title(Line::from(Span::styled(format!(" {title} "), Style::new().fg(Color::Rgb(255, 253, 242)).bold())).centered())
        }
    };
    // Each box is only as wide as its content (+ borders), so the scene shows around it.
    let hint = " ↑/↓ move  enter select  q quit ";
    let item_w = BOOT_ITEMS.iter().map(|s| s.chars().count() + 4).max().unwrap_or(0);
    let menu_iw = item_w.max(hint.chars().count()).max(APP_NAME.len() + 4);
    // connection checks, run at startup: dot, name padded to the longest, message
    let name_w = app.health.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    let health_iw = if app.health.is_empty() {
        25
    } else {
        app.health.iter().map(|r| 3 + name_w + 2 + r.message.chars().count() + 1).max().unwrap_or(0)
    };
    let menu_w = (menu_iw as u16 + 2).min(area.width);
    let health_w = (health_iw as u16 + 2).min(area.width);
    let menu_h = BOOT_ITEMS.len() as u16 + 5;
    let health_h = app.health.len().max(1) as u16 + 2;
    let mid = |w: u16| area.x + (area.width - w) / 2;
    // the menu itself is centred; the health box and version hang below it
    let y = area.y + area.height.saturating_sub(menu_h) / 2;
    let menu = Rect::new(mid(menu_w), y, menu_w, menu_h).intersection(area);
    let health_r = Rect::new(mid(health_w), y + menu_h, health_w, health_h).intersection(area);

    let mut lines = vec![Line::raw("")];
    for (i, item) in BOOT_ITEMS.iter().enumerate() {
        lines.push(if i == app.boot_idx {
            Line::from(Span::styled(format!(" ▶ {item:<iw$}", iw = menu_iw - 3), Style::new().fg(pal().on_accent).bg(pal().green).bold()))
        } else {
            Line::from(Span::styled(format!("   {item:<iw$}", iw = menu_iw - 3), Style::new().fg(pal().text)))
        });
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(hint, Style::new().fg(pal().dim))));
    f.render_widget(Clear, menu);
    f.render_widget(Paragraph::new(lines).block(boxed(APP_NAME)), menu);

    let health: Vec<Line> = if app.health.is_empty() {
        let msg = if app.health_running { " checking connections..." } else { " connection checks not run" };
        vec![Line::styled(msg, Style::new().fg(pal().dim))]
    } else {
        app.health
            .iter()
            .map(|r| {
                let color = if r.ok { pal().green } else { pal().red };
                Line::from(vec![
                    Span::styled(" ● ", Style::new().fg(color)),
                    Span::styled(format!("{:<name_w$}  ", r.name), Style::new().fg(pal().text)),
                    // long messages (e.g. a proxy URL) are cut by the box on a narrow screen
                    Span::styled(r.message.clone(), Style::new().fg(color)),
                ])
            })
            .collect()
    };
    f.render_widget(Clear, health_r);
    f.render_widget(Paragraph::new(health).block(boxed("")), health_r);
    let version = format!(" {} ", version_line());
    let vw = (version.chars().count() as u16).min(area.width);
    let version_r = Rect::new(mid(vw), y + menu_h + health_h, vw, 1).intersection(area);
    f.render_widget(
        Paragraph::new(Span::styled(version, Style::new().fg(Color::Rgb(200, 212, 235)).bg(Color::Rgb(10, 16, 36)))),
        version_r,
    );
}

/// Release name shown under the start menu.
const CODENAME: &str = "Taj Mahal";

/// Name, version and release name, e.g. "MarketGUI v1.0.0 (Taj Mahal)".
pub(super) fn version_line() -> String {
    format!("{APP_NAME} v{} ({CODENAME})", env!("CARGO_PKG_VERSION"))
}

/// Moonlit mausoleum fitted to `area` as halftone dots (two per cell, square).
fn scene(f: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let ht = crate::art::scene_halftone(t, area.width as usize, area.height as usize * 2);
    halftone(f, app, area, ht);
}

/// Gear and wrench animation at the bottom of `area`, centred, as halftone dots: full size where
/// it fits, else box-averaged down to fit, down to a quarter size; else nothing.
pub(super) fn gear(f: &mut Frame, app: &mut App, area: Rect, t: f64) {
    use crate::art::{gear_halftone, GEAR_H, GEAR_W};
    let s = (f64::from(area.width) / GEAR_W as f64).min(f64::from(area.height) / GEAR_H as f64).min(1.0);
    let (w, h) = ((GEAR_W as f64 * s) as u16, (GEAR_H as f64 * s) as u16);
    if w < (GEAR_W / 4) as u16 || h < (GEAR_H / 4) as u16 {
        return;
    }
    let r = Rect::new(area.x + (area.width - w) / 2, area.bottom() - h, w, h);
    halftone(f, app, r, gear_halftone(t, w as usize, h as usize * 2));
}

/// Draws `ht` (`area.width` x `2 * area.height` dots) over `area`. The window app paints the dots
/// itself (`App::halftones`), so the cells are left blank; the terminal gets one glyph per cell
/// (` · ∙ • ●` by coverage) in the dots' colour.
fn halftone(f: &mut Frame, app: &mut App, area: Rect, ht: crate::art::Halftone) {
    let rgb = |c: crate::art::Rgb| Color::Rgb(c[0] as u8, c[1] as u8, c[2] as u8);
    let buf = f.buffer_mut();
    for cy in 0..area.height {
        for cx in 0..area.width {
            let cell = &mut buf[(area.x + cx, area.y + cy)];
            if app.pixel_chart {
                cell.reset();
                continue;
            }
            let (x, y) = (cx as usize, cy as usize * 2);
            let ((a, ka), (b, kb)) = (ht.dots[y * ht.w + x], ht.dots[(y + 1) * ht.w + x]);
            let k = (ka + kb) / 2.0;
            let col = if k > 0.0 { [0, 1, 2].map(|i| (a[i] * ka + b[i] * kb) / (ka + kb)) } else { [0.0; 3] };
            let glyph = match k {
                k if k < 0.06 => ' ',
                k if k < 0.2 => '·',
                k if k < 0.4 => '∙',
                k if k < 0.65 => '•',
                _ => '●',
            };
            cell.set_char(glyph).set_fg(rgb(col));
            if let Some(bg) = ht.bg {
                cell.set_bg(rgb(bg));
            }
        }
    }
    if app.pixel_chart {
        app.halftones.push((area, ht));
    }
}
