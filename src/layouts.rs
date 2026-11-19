//! The user's own dashboard tabs (Config -> Layouts): an arrangement (`Template`) of slots, a
//! panel in each slot, and the split sizes.

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

/// Most custom tabs.
pub const MAX: usize = 3;
/// Split sizes the keys step through: the first part's share, in percent. Dragging a border
/// sets any size in `SPLIT_MIN..=SPLIT_MAX`.
pub const SPLITS: [u8; 7] = [25, 33, 40, 50, 60, 67, 75];
pub const SPLIT_MIN: u8 = 10;
pub const SPLIT_MAX: u8 = 90;
/// Longest tab name.
pub const NAME_MAX: usize = 12;

/// What a slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    #[default]
    Empty,
    List,
    Alerts,
    Chart,
    Bubbles,
    Volume,
    Blacklist,
}

impl Panel {
    pub const ALL: [Panel; 7] = [Panel::Empty, Panel::List, Panel::Alerts, Panel::Chart, Panel::Bubbles, Panel::Volume, Panel::Blacklist];

    pub fn label(self) -> &'static str {
        match self {
            Panel::Empty => "Empty",
            Panel::List => "List",
            Panel::Alerts => "Alerts",
            Panel::Chart => "Chart",
            Panel::Bubbles => "Bubbles",
            Panel::Volume => "Volume",
            Panel::Blacklist => "Blacklist",
        }
    }
}

/// How the slots are arranged (`|` side by side, `/` stacked).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    /// A
    Single,
    /// A | B
    #[default]
    Columns,
    /// A / B
    Rows,
    /// A | (B / C)
    LeftRight2,
    /// (A / B) | C
    Left2Right,
    /// A / (B | C)
    Top2Bottom,
    /// (A | B) / C
    TopBottom2,
    /// A | B | C
    Columns3,
    /// A / B / C
    Rows3,
    /// (A | B) / (C | D)
    Grid,
}

impl Template {
    pub const ALL: [Template; 10] = [
        Template::Single,
        Template::Columns,
        Template::Rows,
        Template::LeftRight2,
        Template::Left2Right,
        Template::Top2Bottom,
        Template::TopBottom2,
        Template::Columns3,
        Template::Rows3,
        Template::Grid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Template::Single => "one panel",
            Template::Columns => "2 columns",
            Template::Rows => "2 rows",
            Template::LeftRight2 => "left | right split",
            Template::Left2Right => "left split | right",
            Template::Top2Bottom => "top / bottom split",
            Template::TopBottom2 => "top split / bottom",
            Template::Columns3 => "3 columns",
            Template::Rows3 => "3 rows",
            Template::Grid => "2 x 2 grid",
        }
    }

    pub fn slots(self) -> usize {
        match self {
            Template::Single => 1,
            Template::Columns | Template::Rows => 2,
            Template::Grid => 4,
            _ => 3,
        }
    }

    pub fn splits(self) -> usize {
        match self {
            Template::Single => 0,
            Template::Columns | Template::Rows => 1,
            _ => 2,
        }
    }

    /// What each split sets, for the Config pane.
    pub fn split_label(self, i: usize) -> &'static str {
        match (self, i) {
            (Template::Columns | Template::Columns3 | Template::LeftRight2 | Template::Left2Right, 0) => "left width",
            (Template::Rows | Template::Rows3 | Template::Top2Bottom | Template::TopBottom2, 0) => "top height",
            (Template::Grid, 0) => "top height",
            (Template::Columns3, _) => "middle width",
            (Template::Rows3, _) => "middle height",
            (Template::LeftRight2 | Template::Left2Right, _) => "upper height",
            (Template::Top2Bottom | Template::TopBottom2 | Template::Grid, _) => "left width",
            _ => "",
        }
    }

    /// The splits' borders in `area`: for each split, the area it divides and whether it divides
    /// it into columns (else rows). Its border sits at its share of that area.
    pub fn handles(self, area: Rect, splits: &[u8]) -> Vec<(usize, Rect, bool)> {
        let s = |i: usize| splits.get(i).copied().unwrap_or(50);
        match self {
            Template::Single => vec![],
            Template::Columns => vec![(0, area, true)],
            Template::Rows => vec![(0, area, false)],
            Template::LeftRight2 => vec![(0, area, true), (1, cols(area, s(0))[1], false)],
            Template::Left2Right => vec![(0, area, true), (1, cols(area, s(0))[0], false)],
            Template::Top2Bottom => vec![(0, area, false), (1, rows(area, s(0))[1], true)],
            Template::TopBottom2 => vec![(0, area, false), (1, rows(area, s(0))[0], true)],
            Template::Columns3 => vec![(0, area, true), (1, cols(area, s(0))[1], true)],
            Template::Rows3 => vec![(0, area, false), (1, rows(area, s(0))[1], false)],
            Template::Grid => vec![(0, area, false), (1, area, true)],
        }
    }

    /// The slots' areas in `area`, A first. Splits (percent of the first part; for the
    /// second split of three in a row, of what the first left) default to 50.
    pub fn areas(self, area: Rect, splits: &[u8]) -> Vec<Rect> {
        let s = |i: usize| splits.get(i).copied().unwrap_or(50);
        match self {
            Template::Single => vec![area],
            Template::Columns => cols(area, s(0)).to_vec(),
            Template::Rows => rows(area, s(0)).to_vec(),
            Template::LeftRight2 => {
                let [a, r] = cols(area, s(0));
                let [b, c] = rows(r, s(1));
                vec![a, b, c]
            }
            Template::Left2Right => {
                let [l, c] = cols(area, s(0));
                let [a, b] = rows(l, s(1));
                vec![a, b, c]
            }
            Template::Top2Bottom => {
                let [a, bot] = rows(area, s(0));
                let [b, c] = cols(bot, s(1));
                vec![a, b, c]
            }
            Template::TopBottom2 => {
                let [top, c] = rows(area, s(0));
                let [a, b] = cols(top, s(1));
                vec![a, b, c]
            }
            Template::Columns3 => {
                let [a, r] = cols(area, s(0));
                let [b, c] = cols(r, s(1));
                vec![a, b, c]
            }
            Template::Rows3 => {
                let [a, r] = rows(area, s(0));
                let [b, c] = rows(r, s(1));
                vec![a, b, c]
            }
            Template::Grid => {
                let [top, bot] = rows(area, s(0));
                let [a, b] = cols(top, s(1));
                let [c, d] = cols(bot, s(1));
                vec![a, b, c, d]
            }
        }
    }
}

/// `area` cut into a left part `pct` percent wide and the rest.
fn cols(area: Rect, pct: u8) -> [Rect; 2] {
    let w = (u32::from(area.width) * u32::from(pct) / 100) as u16;
    [Rect { width: w, ..area }, Rect { x: area.x + w, width: area.width - w, ..area }]
}

/// `area` cut into a top part `pct` percent high and the rest.
fn rows(area: Rect, pct: u8) -> [Rect; 2] {
    let h = (u32::from(area.height) * u32::from(pct) / 100) as u16;
    [Rect { height: h, ..area }, Rect { y: area.y + h, height: area.height - h, ..area }]
}

/// One custom tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomLayout {
    /// Shown as a header tab.
    pub on: bool,
    pub name: String,
    pub template: Template,
    pub slots: Vec<Panel>,
    pub splits: Vec<u8>,
}

impl Default for CustomLayout {
    fn default() -> Self {
        CustomLayout { on: false, name: String::new(), template: Template::Columns, slots: vec![Panel::Chart, Panel::List], splits: vec![50] }
    }
}

impl CustomLayout {
    /// Slots and splits as many as the template has, each panel at most once, splits from
    /// `SPLITS`, a name of at most `NAME_MAX` characters.
    pub fn normalize(&mut self) {
        self.slots.resize(self.template.slots(), Panel::Empty);
        for i in 0..self.slots.len() {
            if self.slots[i] != Panel::Empty && self.slots[..i].contains(&self.slots[i]) {
                self.slots[i] = Panel::Empty;
            }
        }
        self.splits.resize(self.template.splits(), 50);
        for s in &mut self.splits {
            *s = (*s).clamp(SPLIT_MIN, SPLIT_MAX);
        }
        self.name = self.name.trim().chars().take(NAME_MAX).collect::<String>().trim_end().to_string();
    }

    pub fn contains(&self, p: Panel) -> bool {
        self.slots.contains(&p)
    }

    /// Whether it is a header tab: switched on with something in a slot.
    pub fn is_used(&self) -> bool {
        self.on && self.slots.iter().any(|&p| p != Panel::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_tiles_the_area() {
        for t in Template::ALL {
            for (w, h) in [(120, 40), (81, 23), (7, 3)] {
                for s in [25, 50, 75] {
                    let area = Rect::new(3, 2, w, h);
                    let a = t.areas(area, &[s, 100 - s]);
                    assert_eq!(a.len(), t.slots(), "{t:?}");
                    let total: u32 = a.iter().map(|r| u32::from(r.width) * u32::from(r.height)).sum();
                    assert_eq!(total, u32::from(w) * u32::from(h), "{t:?} {w}x{h} {s}");
                    for (i, r) in a.iter().enumerate() {
                        assert_eq!(r.intersection(area), *r, "{t:?}");
                        assert!(a[i + 1..].iter().all(|o| !o.intersects(*r)), "{t:?} {a:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn left_right_split_puts_a_on_the_left() {
        let a = Template::LeftRight2.areas(Rect::new(0, 0, 100, 40), &[40, 25]);
        assert_eq!(a, vec![Rect::new(0, 0, 40, 40), Rect::new(40, 0, 60, 10), Rect::new(40, 10, 60, 30)]);
    }

    #[test]
    fn handles_sit_on_the_borders_between_slots() {
        let area = Rect::new(0, 0, 100, 40);
        for t in Template::ALL {
            let h = t.handles(area, &[40, 25]);
            assert_eq!(h.len(), t.splits(), "{t:?}");
        }
        // left | right split: the column border, then the right part's row border
        let h = Template::LeftRight2.handles(area, &[40, 25]);
        assert_eq!(h, vec![(0, area, true), (1, Rect::new(40, 0, 60, 40), false)]);
    }

    #[test]
    fn normalize_fits_the_template() {
        let mut l = CustomLayout {
            on: true,
            name: "  a very long tab name  ".into(),
            template: Template::Rows3,
            slots: vec![Panel::Chart, Panel::Chart],
            splits: vec![52, 95, 10],
        };
        l.normalize();
        assert_eq!(l.slots, vec![Panel::Chart, Panel::Empty, Panel::Empty]);
        assert_eq!(l.splits, vec![52, 90]);
        assert_eq!(l.name, "a very long");
        assert!(l.contains(Panel::Chart) && l.is_used());
    }
}
