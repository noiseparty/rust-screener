//! Keys of the boot screen, dashboard, chart, columns, storage and health views.

use super::*;

impl App {
    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        // trace mode: the debug overlay takes the dashboard's keys while open
        if self.view == View::Dashboard && self.debug.on && (self.debug.open || is_debug_key(k.code)) {
            self.key_debug(k);
            return;
        }
        match self.view {
            View::Boot => self.key_boot(k),
            View::Health => self.key_health(k),
            View::Dashboard => self.key_dashboard(k),
            View::SetupMenu => self.key_setup_menu(k),
            View::SetupConfig => self.key_setup_config(k),
            View::SetupBlacklist => self.key_blacklist(k),
            View::SetupFilters => self.key_filters(k),
            View::Columns => self.key_columns(k),
            View::Chart => self.key_chart(k),
            View::Storage => self.key_storage(k),
            View::Presets => self.key_presets(k),
            View::SetupTheme => self.key_theme(k),
            View::SetupSound => self.key_sound(k),
            View::SetupFormat => self.key_format(k),
        }
    }

    /// The debug overlay: its key or Esc closes it, the arrows and pages scroll the log.
    fn key_debug(&mut self, k: KeyEvent) {
        let d = &mut self.debug;
        match k.code {
            c if is_debug_key(c) => {
                d.open = !d.open;
                d.scroll = 0;
            }
            KeyCode::Esc => d.open = false,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => d.scroll += 1,
            KeyCode::Down | KeyCode::Char('j') => d.scroll = d.scroll.saturating_sub(1),
            KeyCode::PageUp => d.scroll += 10,
            KeyCode::PageDown => d.scroll = d.scroll.saturating_sub(10),
            KeyCode::End | KeyCode::Home | KeyCode::Char('g') => d.scroll = 0,
            _ => {}
        }
    }

    pub(super) fn start(&mut self) {
        if self.first_run {
            // Filters first: one list fetch feeds the preview; chart data waits until saved.
            self.setup_idx = 0;
            self.inputs = inputs_from(&self.cfg);
            self.focus = field_index(FieldId::MinChange);
            self.config_err = None;
            self.snapshot = self.all.clone();
            self.view = View::SetupConfig;
            self.request_fetch();
            return;
        }
        self.view = View::Dashboard;
        if !self.running {
            self.running = true;
            self.request_fetch();
        }
    }

    pub(super) fn key_boot(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.boot_idx = self.boot_idx.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.boot_idx = (self.boot_idx + 1).min(BOOT_ITEMS.len() - 1),
            KeyCode::Enter | KeyCode::Char(' ') => match self.boot_idx {
                0 => self.start(),
                _ => self.open_setup(),
            },
            _ => {}
        }
    }

    /// Runs the connection checks (at startup, and from Config -> Health Check).
    pub fn run_health_check(&mut self) {
        if !self.health_running && self.health_tx.send(()).is_ok() {
            self.health_running = true;
        }
    }

    pub(super) fn key_storage(&mut self, k: KeyEvent) {
        let Some(b) = self.storage.as_mut() else {
            self.view = View::SetupMenu;
            return;
        };
        if b.confirm {
            match k.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    b.delete_selected();
                    self.storage_at = None; // the header total follows at the next tick
                }
                _ => {
                    b.confirm = false;
                    b.msg = None;
                }
            }
            return;
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => b.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => b.move_by(1),
            KeyCode::PageUp => b.move_by(-10),
            KeyCode::PageDown => b.move_by(10),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => b.enter(),
            KeyCode::Delete | KeyCode::Char('d') if b.selected().is_some() => b.confirm = true,
            KeyCode::Char('r') => b.refresh(),
            // at the root: back to the sidebar, which keeps showing the browser
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => {
                if !b.up() {
                    self.view = View::SetupMenu;
                }
            }
            _ => {}
        }
    }

    pub(super) fn key_health(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Enter | KeyCode::Char(' ') => self.run_health_check(),
            _ => {}
        }
    }

    /// Dashboard layout, saved.
    pub(super) fn set_layout(&mut self, l: Layout) {
        self.cfg.layout = l;
        self.alert_scroll = 0;
        self.save_config();
    }

    pub(super) fn key_dashboard(&mut self, k: KeyEvent) {
        let ncols = self.visible_cols.len();
        let chart = self.cfg.layout.has_chart();
        // the Alerts layout has no list: up/down scroll the alerts
        if self.cfg.layout == Layout::Alerts {
            let s = self.alert_scroll;
            let scrolled = match k.code {
                KeyCode::Up | KeyCode::Char('k') => Some(s.saturating_sub(1)),
                KeyCode::Down | KeyCode::Char('j') => Some(s + 1),
                KeyCode::PageUp => Some(s.saturating_sub(10)),
                KeyCode::PageDown => Some(s + 10),
                KeyCode::Home | KeyCode::Char('g') => Some(0),
                _ => None,
            };
            if let Some(s) = scrolled {
                self.alert_scroll = s;
                return;
            }
        }
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('r') => self.request_fetch(),
            KeyCode::Char('w') => self.set_layout(self.cfg.layout.next()),
            KeyCode::Left | KeyCode::Char('h') => self.sel_col = self.sel_col.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => self.sel_col = (self.sel_col + 1).min(ncols.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => self.move_row(self.row.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.move_row(self.row + 1),
            KeyCode::PageUp => self.move_row(self.row.saturating_sub(10)),
            KeyCode::PageDown => self.move_row(self.row + 10),
            KeyCode::Home | KeyCode::Char('g') => self.move_row(0),
            KeyCode::Enter => {
                if let Some(t) = self.tickers.get(self.row) {
                    self.chart_symbol = t.symbol.clone();
                    self.view = View::Chart;
                }
            }
            KeyCode::Char('s') => {
                if let Some(&ci) = self.visible_cols.get(self.sel_col) {
                    let key = COLUMNS[ci].key;
                    if self.sort_key == key {
                        self.sort_desc = !self.sort_desc;
                    } else {
                        self.sort_key = key;
                        self.sort_desc = true;
                    }
                    self.reapply_local();
                }
            }
            KeyCode::Char('f') => {
                for step in 1..=FILTER_NAMES.len() {
                    let i = (self.alert_filter + step) % FILTER_NAMES.len();
                    if self.cfg.filter_on(FILTER_NAMES[i]) {
                        self.alert_filter = i;
                        break;
                    }
                }
            }
            // chart keys only where the layout shows the chart
            KeyCode::Char(c @ '1'..='5') if chart => self.set_timeframe(Tf::ALL[c as usize - '1' as usize]),
            KeyCode::Char('t') if chart => self.set_timeframe(self.timeframe.next()),
            KeyCode::Char('0') if chart => {
                self.reset_chart_view();
                self.scale = crate::chart::DEFAULT_SCALE;
            }
            KeyCode::Char('o') | KeyCode::Char('c') => self.open_setup(),
            KeyCode::Char('v') => {
                self.view = View::Columns;
                self.col_cursor = 0;
            }
            _ => {}
        }
    }

    pub(super) fn move_row(&mut self, row: usize) {
        self.row = row.min(self.tickers.len().saturating_sub(1));
        self.sel_symbol = self.tickers.get(self.row).map(|t| t.symbol.clone());
    }

    /// Re-derive the cursor row from the selected symbol after the list changes.
    pub(super) fn resync_row(&mut self) {
        let pos = self.sel_symbol.as_ref().and_then(|s| self.tickers.iter().position(|t| &t.symbol == s));
        match pos {
            Some(i) => self.row = i,
            None => self.move_row(self.row),
        }
    }

    pub(super) fn key_chart(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter | KeyCode::Char('d') => self.view = View::Dashboard,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                let down = matches!(k.code, KeyCode::Down | KeyCode::Char('j'));
                self.move_row(if down { self.row + 1 } else { self.row.saturating_sub(1) });
                if let Some(t) = self.tickers.get(self.row) {
                    self.chart_symbol = t.symbol.clone();
                }
            }
            KeyCode::Char('r') => self.request_fetch(),
            KeyCode::Char(c @ '1'..='5') => self.set_timeframe(Tf::ALL[c as usize - '1' as usize]),
            KeyCode::Char('t') => self.set_timeframe(self.timeframe.next()),
            KeyCode::Char('0') => {
                self.reset_chart_view();
                self.scale = crate::chart::DEFAULT_SCALE;
            }
            _ => {}
        }
    }

    pub(super) fn key_columns(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') | KeyCode::Char('v') => self.view = View::Dashboard,
            KeyCode::Up | KeyCode::Char('k') => self.col_cursor = self.col_cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.col_cursor = (self.col_cursor + 1).min(COLUMNS.len() - 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let key = COLUMNS[self.col_cursor].key;
                let on = self.cfg.column_visible(key);
                self.cfg.column_visibility.insert(key.to_string(), !on);
                self.save_config();
            }
            _ => {}
        }
    }
}

/// F12, or the backtick (`) where F12 is taken: toggles the debug overlay in trace mode.
pub fn is_debug_key(c: KeyCode) -> bool {
    matches!(c, KeyCode::F(12) | KeyCode::Char('`'))
}
