//! Config screen: sidebar, presets, Configuration form, theme, blacklist, alert filters,
//! sound and formats, and saving the config.

use super::*;

impl App {
    /// Window text size (GUI only), kept in the config.
    pub fn set_font_size(&mut self, size: f32) {
        self.cfg.font_size = size;
        self.save_config();
    }

    pub(super) fn save_config(&mut self) {
        self.cfg.blacklisted_tickers = self.engine.blacklist.iter().cloned().collect();
        if let Err(e) = self.cfg.save(&self.config_path) {
            self.error = Some(format!("save config: {e}"));
        }
    }

    pub(super) fn back_from_setup(&mut self) {
        self.view = if self.running { View::Dashboard } else { View::Boot };
    }

    pub(super) fn open_setup(&mut self) {
        self.select_setup(0);
        self.view = View::SetupMenu;
    }

    /// Selects sidebar item `i` and readies its settings pane, which shows it live while the
    /// sidebar has focus.
    pub(super) fn select_setup(&mut self, i: usize) {
        self.setup_idx = i.min(SETUP_ITEMS.len() - 1);
        self.snapshot = self.all.clone();
        // unsaved Configuration inputs and a previewed theme are dropped when the selection moves
        self.palette = theme::palette(&self.cfg.theme);
        self.theme_cursor = theme::names().iter().position(|(id, _)| *id == self.cfg.theme).unwrap_or(0);
        self.preset_formats = None;
        match SETUP_VIEWS[self.setup_idx] {
            View::SetupConfig => {
                self.inputs = inputs_from(&self.cfg);
                self.focus = 0;
                self.config_err = None;
            }
            View::Presets => {
                self.preset_input = None;
                self.preset_ask = None;
                self.preset_msg = None;
                self.refresh_presets();
            }
            View::SetupBlacklist => {
                self.bl_cursor = 0;
                self.bl_input = None;
                self.bl_err = None;
            }
            View::SetupFilters => self.filter_cursor = 0,
            View::SetupSound => self.sound_cursor = 0,
            View::SetupLayouts => self.reset_layouts_draft(),
            View::SetupFormat => {
                self.fmt_draft = self.cfg.formats.clone();
                self.fmt_cursor = 0;
            }
            View::Storage => self.storage = self.home.as_deref().map(storage::Browser::new),
            _ => {}
        }
    }

    pub(super) fn key_setup_menu(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') => self.back_from_setup(),
            KeyCode::Up | KeyCode::Char('k') => self.select_setup(self.setup_idx.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select_setup(self.setup_idx + 1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') | KeyCode::Char(' ') => {
                self.view = SETUP_VIEWS[self.setup_idx]
            }
            _ => {}
        }
    }

    pub(super) fn presets_dir(&self) -> Option<PathBuf> {
        self.home.as_deref().map(presets::dir)
    }

    /// Re-reads the preset list, keeping the cursor in it.
    pub(super) fn refresh_presets(&mut self) {
        self.presets = match self.presets_dir() {
            Some(d) => presets::list(&d),
            None => vec![presets::BUILTIN.to_string()],
        };
        self.select_preset(self.preset_cursor);
    }

    /// Moves the Presets cursor and reads the preset there for the preview; the last row ("Save
    /// current config as") previews the config in use.
    pub(super) fn select_preset(&mut self, i: usize) {
        self.preset_cursor = i.min(self.presets.len());
        let c = if self.preset_cursor == self.presets.len() { Some(self.cfg.clone()) } else { self.load_preset().ok() };
        self.preset_inputs = c.as_ref().map(inputs_from);
        self.preset_summary = c.map(|c| c.formats.summary());
    }

    pub(super) fn load_preset(&self) -> Result<Config, String> {
        let name = self.presets.get(self.preset_cursor).ok_or("no preset")?;
        presets::load(&self.presets_dir().unwrap_or_default(), name, &self.cfg).map_err(|e| format!("could not read {name}: {e}"))
    }

    /// Saves the config in use as preset `name`.
    pub(super) fn save_preset(&mut self, name: &str) {
        let Some(dir) = self.presets_dir() else {
            self.preset_msg = Some("no app folder to save presets in".into());
            return;
        };
        // the proxy key is a secret: it stays in config.json only
        let cfg = Config { finviz_proxy_key: String::new(), ..self.cfg.clone() };
        self.preset_msg = Some(match presets::save(&dir, name, &cfg) {
            Ok(()) => format!("saved {name}"),
            Err(e) => format!("could not save {name}: {e}"),
        });
        self.refresh_presets();
        if let Some(i) = self.presets.iter().position(|p| p.eq_ignore_ascii_case(name)) {
            self.select_preset(i);
        }
    }

    pub(super) fn key_presets(&mut self, k: KeyEvent) {
        if let Some(buf) = self.preset_input.as_mut() {
            match k.code {
                KeyCode::Esc => {
                    self.preset_input = None;
                    self.preset_msg = None;
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Enter => match presets::sanitize(buf) {
                    None => self.preset_msg = Some("the name needs a letter or digit".into()),
                    Some(name) if presets::is_builtin(&name) => {
                        self.preset_msg = Some(format!("{} is built in: pick another name", presets::BUILTIN))
                    }
                    Some(name) => {
                        self.preset_input = None;
                        self.preset_msg = None;
                        let taken = self.presets_dir().is_some_and(|d| presets::exists(&d, &name));
                        if taken {
                            self.preset_ask = Some(PresetAsk::Overwrite(name));
                        } else {
                            self.save_preset(&name);
                        }
                    }
                },
                KeyCode::Char(c) if buf.chars().count() < presets::NAME_MAX && !k.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
                _ => {}
            }
            return;
        }
        if let Some(ask) = self.preset_ask.take() {
            self.preset_msg = None;
            if k.code == KeyCode::Char('y') {
                match ask {
                    PresetAsk::Overwrite(name) => self.save_preset(&name),
                    PresetAsk::Delete(name) => {
                        let Some(dir) = self.presets_dir() else { return };
                        self.preset_msg = Some(match presets::delete(&dir, &name) {
                            Ok(()) => format!("deleted {name}"),
                            Err(e) => format!("could not delete {name}: {e}"),
                        });
                        self.refresh_presets();
                        self.select_preset(self.preset_cursor.min(self.presets.len().saturating_sub(1)));
                    }
                }
            }
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.view = View::SetupMenu,
            KeyCode::Up | KeyCode::Char('k') => self.select_preset(self.preset_cursor.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select_preset(self.preset_cursor + 1),
            // the last row: name prompt for saving the config in use
            KeyCode::Enter | KeyCode::Char(' ') if self.preset_cursor == self.presets.len() => {
                self.preset_input = Some(String::new());
                self.preset_msg = None;
            }
            // into the Configuration form, previewed with its formats; Enter there saves both (the
            // theme stays)
            KeyCode::Enter | KeyCode::Char(' ') => match self.load_preset() {
                Ok(c) => {
                    self.inputs = inputs_from(&c);
                    self.preset_formats = Some(c.formats);
                    self.focus = 0;
                    self.config_err = None;
                    self.preset_msg = None;
                    self.setup_idx = 0;
                    self.view = View::SetupConfig;
                }
                Err(e) => self.preset_msg = Some(e),
            },
            KeyCode::Char('d') => match self.presets.get(self.preset_cursor) {
                Some(name) if presets::is_builtin(name) => self.preset_msg = Some(format!("{name} is built in")),
                Some(name) => self.preset_ask = Some(PresetAsk::Delete(name.clone())),
                None => {}
            },
            _ => {}
        }
    }

    pub(super) fn key_setup_config(&mut self, k: KeyEvent) {
        let n = FIELDS.len();
        let picked = matches!(FIELDS[self.focus].kind, FieldKind::Choice(_) | FieldKind::Toggle);
        match k.code {
            KeyCode::Esc => {
                // unsaved inputs are dropped
                self.inputs = inputs_from(&self.cfg);
                self.preset_formats = None;
                self.config_err = None;
                self.view = if self.first_run { View::Boot } else { View::SetupMenu };
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + n - 1) % n,
            KeyCode::Char('s') if k.modifiers.contains(KeyModifiers::CONTROL) => self.commit_config(),
            KeyCode::Enter => self.commit_config(),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if picked => match FIELDS[self.focus].kind {
                FieldKind::Choice(opts) => self.inputs[self.focus] = cycle_choice(&self.inputs[self.focus], opts, k.code == KeyCode::Left),
                FieldKind::Toggle => self.inputs[self.focus] = on_off(self.inputs[self.focus] != "On").into(),
                _ => {}
            },
            // Recommended preset into the inputs (previewed, saved with Enter), whatever field has
            // focus; no field takes a "p"
            KeyCode::Char('p') => {
                self.inputs = inputs_from(&Config::recommended(&self.cfg));
                self.config_err = None;
            }
            // A choice field takes no typing.
            KeyCode::Backspace | KeyCode::Char(_) if picked => {}
            KeyCode::Backspace => {
                self.inputs[self.focus].pop();
            }
            KeyCode::Char(c) if self.inputs[self.focus].len() < 20 && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.inputs[self.focus].push(c)
            }
            _ => {}
        }
    }

    pub(super) fn commit_config(&mut self) {
        let mut base = self.cfg.clone();
        if let Some(f) = &self.preset_formats {
            base.formats = f.clone();
        }
        match config_from_inputs(&base, &self.inputs) {
            Ok(cfg) => {
                self.cfg = cfg;
                self.preset_formats = None;
            }
            Err(i) => {
                self.focus = i;
                self.config_err = Some(format!("Invalid {}", FIELDS[i].label));
                return;
            }
        }
        self.engine.apply_config(&self.cfg);
        self.inputs = inputs_from(&self.cfg);
        self.config_err = None;
        self.save_config();
        self.view = View::SetupMenu;
        self.reapply_local();
        if self.first_run {
            self.first_run = false;
            self.start();
        }
    }

    /// Config -> Theme: up/down previews a theme, Enter keeps and saves it, Esc reverts.
    pub(super) fn key_theme(&mut self, k: KeyEvent) {
        let names = theme::names();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.theme_cursor = self.theme_cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.theme_cursor = (self.theme_cursor + 1).min(names.len() - 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.cfg.theme = names[self.theme_cursor].0.to_string();
                self.save_config();
                self.view = View::SetupMenu;
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                self.select_setup(self.setup_idx);
                self.view = View::SetupMenu;
                return;
            }
            _ => return,
        }
        self.palette = theme::palette(names[self.theme_cursor].0);
    }

    pub(super) fn key_blacklist(&mut self, k: KeyEvent) {
        if let Some(buf) = self.bl_input.as_mut() {
            match k.code {
                KeyCode::Esc => {
                    self.bl_input = None;
                    self.bl_err = None;
                }
                KeyCode::Backspace => {
                    if buf.pop().is_none() {
                        self.bl_input = None;
                        self.bl_err = None;
                    }
                }
                KeyCode::Enter => {
                    let sym = buf.trim().to_uppercase();
                    if sym.is_empty() {
                        self.bl_input = None;
                    } else if !self.snapshot.iter().any(|t| t.symbol == sym) {
                        self.bl_err = Some(format!("Ticker '{sym}' not found in current data"));
                    } else {
                        self.engine.toggle_blacklist(&sym);
                        self.save_config();
                        self.bl_input = None;
                        self.bl_err = None;
                    }
                }
                KeyCode::Char(c) if buf.len() < 10 => buf.push(c),
                _ => {}
            }
            return;
        }
        let max = self.snapshot.len().saturating_sub(1);
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Left | KeyCode::Char('h') => self.bl_cursor = self.bl_cursor.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => self.bl_cursor = (self.bl_cursor + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => self.bl_cursor = self.bl_cursor.saturating_sub(BL_COLS),
            KeyCode::Down | KeyCode::Char('j') if self.bl_cursor + BL_COLS <= max => self.bl_cursor += BL_COLS,
            KeyCode::Char('a') => {
                self.bl_input = Some(String::new());
                self.bl_err = None;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(sym) = self.snapshot.get(self.bl_cursor).map(|t| t.symbol.clone()) {
                    self.engine.toggle_blacklist(&sym);
                    self.save_config();
                }
            }
            _ => {}
        }
    }

    pub(super) fn key_filters(&mut self, k: KeyEvent) {
        match k.code {
            // the last row, "Rows on dashboard": left steps down, right/space/enter up, wrapping
            // like the Alert Rows choice in Configuration
            KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') if self.filter_cursor == FILTER_NAMES.len() => {
                let r = u64::from(self.cfg.alert_rows);
                self.cfg.alert_rows = crate::fmt::cycle(fields::ALERT_ROW_CHOICES, &r, k.code == KeyCode::Left) as u16;
                self.save_config();
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Up | KeyCode::Char('k') => self.filter_cursor = self.filter_cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.filter_cursor = (self.filter_cursor + 1).min(FILTER_NAMES.len()),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let name = FILTER_NAMES[self.filter_cursor];
                let on = self.cfg.filter_on(name);
                self.cfg.filter_enabled.insert(name.to_string(), !on);
                self.save_config();
            }
            _ => {}
        }
    }

    /// Config -> Sound: Mute row (enter/space toggles), Volume row (left/right by `VOLUME_STEP`,
    /// enter/space plays a test at that volume, even when muted).
    pub(super) fn key_sound(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Left | KeyCode::Right if self.sound_cursor == 1 => {
                let v = self.cfg.sound_volume;
                self.cfg.sound_volume = if k.code == KeyCode::Left { v.saturating_sub(VOLUME_STEP) } else { (v + VOLUME_STEP).min(100) };
                self.save_config();
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Up | KeyCode::Char('k') => self.sound_cursor = 0,
            KeyCode::Down | KeyCode::Char('j') => self.sound_cursor = 1,
            KeyCode::Enter | KeyCode::Char(' ') if self.sound_cursor == 0 => {
                self.cfg.sound_muted = !self.cfg.sound_muted;
                self.save_config();
            }
            // unit tests never play audio
            KeyCode::Enter | KeyCode::Char(' ') if !cfg!(test) => crate::sound::play_alert(self.cfg.sound_volume, false),
            _ => {}
        }
    }

    /// Config -> Time & Format: up/down moves, left/right picks (space too, except on the typed
    /// rows), typing edits the time zone and date rows (Del clears), Enter saves when every
    /// value is usable, Esc drops the changes.
    pub(super) fn key_format(&mut self, k: KeyEvent) {
        let row = self.fmt_cursor;
        let typed = row < fmt::TYPED_ROWS;
        match k.code {
            KeyCode::Up => self.fmt_cursor = row.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.fmt_cursor = (row + 1).min(fmt::ROWS.len() - 1),
            KeyCode::Left | KeyCode::Right => self.fmt_draft.step(row, k.code == KeyCode::Left),
            KeyCode::Char(' ') if !typed => self.fmt_draft.step(row, false),
            KeyCode::Enter if self.fmt_draft.problem().is_none() => {
                self.cfg.formats = self.fmt_draft.clone();
                self.save_config();
                self.view = View::SetupMenu;
            }
            KeyCode::Esc => {
                self.fmt_draft = self.cfg.formats.clone();
                self.view = View::SetupMenu;
            }
            KeyCode::Backspace if !typed => {
                self.fmt_draft = self.cfg.formats.clone();
                self.view = View::SetupMenu;
            }
            _ => {
                let Some(text) = self.fmt_draft.typed(row) else { return };
                match k.code {
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Delete => text.clear(),
                    KeyCode::Char(c) if text.chars().count() < fmt::INPUT_MAX && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                        text.push(c)
                    }
                    _ => {}
                }
            }
        }
    }
}
