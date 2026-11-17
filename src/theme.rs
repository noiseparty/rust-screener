//! Colour themes: Dark Noir (the original look, default) and every opaline builtin theme.
//!
//! Drawing code reads the palette through `pal()`; `ui::draw` sets it once per frame. The
//! boot screen and the setup gear keep their own colours whatever the theme.

use opaline::OpalineColor;
use ratatui::style::Color;
use std::cell::Cell;

pub const DEFAULT: &str = "dark-noir";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// App background; `Reset` keeps the terminal's own.
    pub bg: Color,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    /// Titles, selected header and tab.
    pub accent: Color,
    pub border: Color,
    pub green: Color,
    pub red: Color,
    pub amber: Color,
    pub zebra: Color,
    pub select: Color,
    /// Text on accent / green / badge backgrounds.
    pub on_accent: Color,
    pub new_bg: Color,
    pub pump_bg: Color,
    pub spike_bg: Color,
    pub setup_bg: Color,
    pub on_setup: Color,
    /// Newest alert row, briefly.
    pub flash: Color,
    /// Focused input field.
    pub input_bg: Color,
    /// Field help text.
    pub tip: Color,
    // chart
    pub chart_text: Color,
    pub chart_muted: Color,
    pub up: Color,
    pub down: Color,
    pub grid: Color,
    pub live: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The original colours (highlowtick.com tokens).
pub const NOIR: Palette = Palette {
    bg: Color::Reset,
    text: rgb(238, 242, 239),
    dim: rgb(159, 179, 166),
    faint: rgb(70, 84, 76),
    accent: rgb(34, 211, 238),
    border: rgb(22, 120, 140),
    green: rgb(34, 197, 94),
    red: rgb(239, 68, 68),
    amber: rgb(245, 158, 11),
    zebra: rgb(14, 22, 17),
    select: rgb(30, 58, 138),
    on_accent: Color::Black,
    new_bg: rgb(234, 179, 8),
    pump_bg: rgb(249, 115, 22),
    spike_bg: rgb(244, 114, 182),
    setup_bg: rgb(147, 51, 234),
    on_setup: Color::White,
    flash: rgb(20, 83, 45),
    input_bg: rgb(24, 36, 30),
    tip: rgb(250, 230, 160),
    chart_text: rgb(230, 237, 243),
    chart_muted: rgb(139, 148, 158),
    up: rgb(63, 185, 80),
    down: rgb(248, 81, 73),
    grid: rgb(48, 54, 61),
    live: rgb(121, 192, 255),
};

fn from_opaline(t: &opaline::Theme) -> Palette {
    let c = |token: &str| {
        let OpalineColor { r, g, b } = t.color(token);
        rgb(r, g, b)
    };
    Palette {
        bg: c("bg.base"),
        text: c("text.primary"),
        dim: c("text.muted"),
        faint: c("text.dim"),
        accent: c("accent.primary"),
        border: c("border.unfocused"),
        green: c("success"),
        red: c("error"),
        amber: c("warning"),
        zebra: c("bg.panel"),
        select: c("bg.selection"),
        on_accent: c("bg.base"),
        new_bg: c("warning"),
        pump_bg: c("code.number"),
        spike_bg: c("accent.tertiary"),
        setup_bg: c("code.keyword"),
        on_setup: c("bg.base"),
        flash: c("bg.highlight"),
        input_bg: c("bg.highlight"),
        tip: c("text.secondary"),
        chart_text: c("text.primary"),
        chart_muted: c("text.muted"),
        up: c("success"),
        down: c("error"),
        grid: c("border.unfocused"),
        live: c("info"),
    }
}

/// Every theme as (id, display name): Dark Noir first, then opaline's builtins.
pub fn names() -> Vec<(&'static str, &'static str)> {
    let mut v = vec![(DEFAULT, "Dark Noir")];
    v.extend(opaline::builtins::builtin_names().iter().copied());
    v
}

/// The palette of theme `id`; an unknown id gets Dark Noir.
pub fn palette(id: &str) -> Palette {
    if id == DEFAULT {
        return NOIR;
    }
    opaline::builtins::load_by_name(id).map_or(NOIR, |t| from_opaline(&t))
}

thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(NOIR) };
}

/// `a` with `t` (0..=1) of `b` blended in. `Reset` counts as black (the dark default
/// background); any other non-RGB colour leaves `a` as it is.
pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let rgb_of = |c: Color| match c {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        Color::Reset | Color::Black => Some([0, 0, 0]),
        _ => None,
    };
    let (Some(x), Some(y)) = (rgb_of(a), rgb_of(b)) else { return a };
    let ch = |i: usize| (f64::from(x[i]) * (1.0 - t) + f64::from(y[i]) * t).round() as u8;
    rgb(ch(0), ch(1), ch(2))
}

/// Sets the palette drawing code reads.
pub fn set(p: Palette) {
    CURRENT.with(|c| c.set(p));
}

/// The current palette.
pub fn pal() -> Palette {
    CURRENT.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noir_is_default_and_every_listed_theme_loads() {
        assert_eq!(palette(DEFAULT), NOIR);
        assert_eq!(palette("no-such-theme"), NOIR);
        let names = names();
        assert_eq!(names[0], ("dark-noir", "Dark Noir"));
        assert!(names.len() > 30);
        for (id, _) in &names[1..] {
            let p = palette(id);
            assert_ne!(p, NOIR, "{id}");
            assert!(matches!(p.bg, Color::Rgb(..)), "{id}");
        }
        assert!(names.iter().any(|(id, _)| *id == "catppuccin-mocha"));
    }

    #[test]
    fn mix_blends_rgb_and_treats_reset_as_black() {
        assert_eq!(mix(rgb(200, 100, 0), rgb(0, 100, 200), 0.5), rgb(100, 100, 100));
        assert_eq!(mix(rgb(200, 100, 0), Color::Reset, 0.5), rgb(100, 50, 0));
        assert_eq!(mix(rgb(200, 100, 0), Color::Blue, 0.5), rgb(200, 100, 0));
    }
}
