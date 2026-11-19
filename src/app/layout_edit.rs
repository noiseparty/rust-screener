//! Config -> Layouts: the user's own dashboard tabs, edited on a draft that Enter saves; and
//! the on-screen editor (`View::LayoutEdit`): the tab full size with live data, widgets dragged
//! onto its slots and its borders dragged to resize.

use super::chart_view::Drag;
use super::*;
use crate::layouts::{self, Template, SPLITS, SPLIT_MAX, SPLIT_MIN};

/// A row of the Layouts pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutRow {
    /// Which of the custom tabs is edited.
    Tab,
    /// Shown as a header tab or not.
    Use,
    Name,
    Template,
    Slot(usize),
    Split(usize),
}

/// The rows for `l`: slots and splits as many as its template has.
pub fn layout_rows(l: &CustomLayout) -> Vec<LayoutRow> {
    let mut rows = vec![LayoutRow::Tab, LayoutRow::Use, LayoutRow::Name, LayoutRow::Template];
    rows.extend((0..l.template.slots()).map(LayoutRow::Slot));
    rows.extend((0..l.template.splits()).map(LayoutRow::Split));
    rows
}

/// The item after (or before) `cur` in `all`, wrapping.
fn cycle<T: Copy + PartialEq>(all: &[T], cur: T, back: bool) -> T {
    let i = all.iter().position(|&v| v == cur).unwrap_or(0);
    all[if back { (i + all.len() - 1) % all.len() } else { (i + 1) % all.len() }]
}

/// The next panel for slot `i` of `l`: Empty or one not shown in another slot.
fn next_panel(l: &CustomLayout, i: usize, back: bool) -> Panel {
    let free = |p: &Panel| *p == Panel::Empty || l.slots.iter().enumerate().all(|(j, s)| j == i || s != p);
    let all: Vec<Panel> = Panel::ALL.into_iter().filter(free).collect();
    cycle(&all, l.slots[i], back)
}

/// The next split size from `SPLITS` above (below) `cur`, which a drag may have set between them.
fn next_split(cur: u8, back: bool) -> u8 {
    if back {
        SPLITS.iter().rev().copied().find(|&v| v < cur).unwrap_or(SPLITS[0])
    } else {
        SPLITS.iter().copied().find(|&v| v > cur).unwrap_or(SPLITS[SPLITS.len() - 1])
    }
}

/// Puts `panel` in slot `to` of `l`; the slot that had it (dragged from there, or already
/// placed) gets what `to` had.
fn place(l: &mut CustomLayout, panel: Panel, to: usize) {
    if let Some(j) = l.slots.iter().position(|&p| p == panel && p != Panel::Empty) {
        l.slots[j] = l.slots[to];
    }
    l.slots[to] = panel;
}

impl App {
    /// The saved custom tabs as a draft of `layouts::MAX` entries.
    pub(super) fn reset_layouts_draft(&mut self) {
        let mut d = self.cfg.custom_layouts.clone();
        d.resize(layouts::MAX, CustomLayout::default());
        self.layouts_draft = d;
        self.layouts_cursor = 0;
    }

    /// The widget being carried in the on-screen editor.
    pub fn carried_widget(&self) -> Option<Panel> {
        match self.drag {
            Some(Drag::Widget { panel, .. }) => Some(panel),
            _ => None,
        }
    }

    /// The custom tab being edited.
    pub fn layout_draft(&self) -> &CustomLayout {
        &self.layouts_draft[self.layouts_tab]
    }

    /// Saves the drafts as the custom tabs (a shown tab that is gone falls back to Full).
    fn commit_layouts(&mut self) {
        self.cfg.custom_layouts = self.layouts_draft.clone();
        self.cfg.validate();
        self.save_config();
    }

    /// Opens the on-screen editor for the custom tab being edited; Esc there goes back to `back`.
    pub(super) fn open_layout_edit(&mut self, back: View) {
        self.layout_edit_undo = Some(self.layouts_draft.clone());
        self.layout_edit_back = back;
        self.drag = None;
        self.view = View::LayoutEdit;
    }

    /// The dashboard's `e` on a custom tab: edit it on screen.
    pub(super) fn edit_current_tab(&mut self) {
        if let Some(i) = self.cfg.layout.custom_index().filter(|_| self.cfg.custom(self.cfg.layout).is_some()) {
            self.reset_layouts_draft();
            self.layouts_tab = i;
            self.open_layout_edit(View::Dashboard);
        }
    }

    /// Config -> Layouts: up/down move, left/right (space) change the value, typing edits the
    /// name, `e` edits on screen, Enter saves, Esc drops the changes.
    pub(super) fn key_layouts(&mut self, k: KeyEvent) {
        let rows = layout_rows(self.layout_draft());
        let row = rows[self.layouts_cursor.min(rows.len() - 1)];
        let back = k.code == KeyCode::Left;
        let l = &mut self.layouts_draft[self.layouts_tab];
        match (k.code, row) {
            (KeyCode::Up, _) => self.layouts_cursor = self.layouts_cursor.saturating_sub(1),
            (KeyCode::Down | KeyCode::Tab, _) => self.layouts_cursor = (self.layouts_cursor + 1).min(rows.len() - 1),
            (KeyCode::Enter, _) => {
                self.commit_layouts();
                self.view = View::SetupMenu;
            }
            (KeyCode::Backspace, LayoutRow::Name) => {
                l.name.pop();
            }
            (KeyCode::Esc | KeyCode::Backspace, _) => {
                self.reset_layouts_draft();
                self.view = View::SetupMenu;
            }
            (KeyCode::Char(c), LayoutRow::Name) if !c.is_control() => {
                if l.name.chars().count() < layouts::NAME_MAX {
                    l.name.push(c);
                }
            }
            (KeyCode::Char('e'), _) => {
                l.on = true;
                self.open_layout_edit(View::SetupLayouts);
            }
            (KeyCode::Left | KeyCode::Right | KeyCode::Char(' '), r) => match r {
                LayoutRow::Tab => {
                    self.layouts_tab = cycle(&[0, 1, 2], self.layouts_tab, back);
                    let n = layout_rows(self.layout_draft()).len();
                    self.layouts_cursor = self.layouts_cursor.min(n - 1);
                }
                LayoutRow::Use => l.on = !l.on,
                LayoutRow::Name => {}
                LayoutRow::Template => {
                    l.template = cycle(&Template::ALL, l.template, back);
                    l.normalize();
                }
                LayoutRow::Slot(i) => l.slots[i] = next_panel(l, i, back),
                LayoutRow::Split(i) => l.splits[i] = next_split(l.splits[i], back),
            },
            (KeyCode::Char('k'), _) => self.layouts_cursor = self.layouts_cursor.saturating_sub(1),
            (KeyCode::Char('j'), _) => self.layouts_cursor = (self.layouts_cursor + 1).min(rows.len() - 1),
            _ => {}
        }
    }

    /// On-screen editor keys: Enter saves (and shows the tab), Esc drops what was changed here.
    pub(super) fn key_layout_edit(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => {
                self.layouts_draft[self.layouts_tab].on = true;
                self.commit_layouts();
                if self.layout_edit_back == View::Dashboard {
                    self.cfg.layout = Layout::CUSTOM[self.layouts_tab];
                    self.save_config();
                }
                self.view = self.layout_edit_back;
            }
            KeyCode::Esc => {
                if let Some(d) = self.layout_edit_undo.take() {
                    self.layouts_draft = d;
                }
                self.view = self.layout_edit_back;
            }
            _ => {}
        }
        self.drag = None;
    }

    /// On-screen editor mouse: press a widget chip or a filled slot to carry its widget, a border
    /// to resize, a template chip to switch; release over a slot to drop the widget there, else
    /// (if it came from a slot) to empty that slot. Right click empties a slot.
    pub(super) fn layout_mouse(&mut self, m: MouseEvent) {
        let p = Position::new(m.column, m.row);
        let hit = |v: &[(Rect, usize)]| v.iter().find(|(r, _)| r.contains(p)).map(|&(_, i)| i);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag_pos = Some(p);
                if let Some(&(_, code)) = self.hits.keys.iter().find(|(r, _)| r.contains(p)) {
                    self.key_layout_edit(KeyEvent::new(code, KeyModifiers::NONE));
                } else if let Some(&(_, t)) = self.hits.edit_templates.iter().find(|(r, _)| r.contains(p)) {
                    let l = &mut self.layouts_draft[self.layouts_tab];
                    l.template = t;
                    l.normalize();
                } else if let Some(&(_, panel)) = self.hits.edit_widgets.iter().find(|(r, _)| r.contains(p)) {
                    self.drag = Some(Drag::Widget { panel, from: None });
                } else if let Some(&(_, i, parent, cols)) = self.hits.edit_handles.iter().find(|h| h.0.contains(p)) {
                    self.drag = Some(Drag::Split { i, parent, cols });
                } else if let Some(i) = hit(&self.hits.edit_slots) {
                    let panel = self.layout_draft().slots[i];
                    if panel != Panel::Empty {
                        self.drag = Some(Drag::Widget { panel, from: Some(i) });
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved => {
                self.drag_pos = Some(p);
                self.drag_to(p);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(Drag::Widget { panel, from }) = self.drag {
                    let l = &mut self.layouts_draft[self.layouts_tab];
                    match hit(&self.hits.edit_slots) {
                        Some(to) => place(l, panel, to),
                        None => {
                            if let Some(i) = from {
                                l.slots[i] = Panel::Empty;
                            }
                        }
                    }
                }
                self.drag = None;
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(i) = hit(&self.hits.edit_slots) {
                    self.layouts_draft[self.layouts_tab].slots[i] = Panel::Empty;
                }
            }
            _ => {}
        }
    }

    /// A border dragged to `p`: its split becomes the share of `parent` before `p`.
    pub(super) fn drag_split(&mut self, i: usize, parent: Rect, cols: bool, p: Position) {
        let (at, len) = if cols { (p.x.saturating_sub(parent.x), parent.width) } else { (p.y.saturating_sub(parent.y), parent.height) };
        if len == 0 {
            return;
        }
        let pct = (u32::from(at) * 100 / u32::from(len)) as u8;
        if let Some(s) = self.layouts_draft[self.layouts_tab].splits.get_mut(i) {
            *s = pct.clamp(SPLIT_MIN, SPLIT_MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_choices_skip_panels_placed_elsewhere() {
        let l = CustomLayout {
            on: true,
            name: String::new(),
            template: Template::LeftRight2,
            slots: vec![Panel::Chart, Panel::Bubbles, Panel::Empty],
            splits: vec![50, 50],
        };
        // after Chart comes Volume: Bubbles is in B
        assert_eq!(next_panel(&l, 0, false), Panel::Volume);
        assert_eq!(next_panel(&l, 2, false), Panel::List);
        assert_eq!(next_panel(&l, 2, true), Panel::Blacklist);
    }

    #[test]
    fn splits_step_from_a_dragged_size() {
        assert_eq!((next_split(52, false), next_split(52, true)), (60, 50));
        assert_eq!((next_split(75, false), next_split(25, true)), (75, 25));
    }

    #[test]
    fn placing_swaps_with_the_slot_that_had_the_widget() {
        let mut l = CustomLayout { slots: vec![Panel::Chart, Panel::List], ..CustomLayout::default() };
        place(&mut l, Panel::Chart, 1);
        assert_eq!(l.slots, vec![Panel::List, Panel::Chart]);
        place(&mut l, Panel::Bubbles, 0);
        assert_eq!(l.slots, vec![Panel::Bubbles, Panel::Chart]);
    }
}
