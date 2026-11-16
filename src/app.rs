use crate::config::{Config, FILTER_NAMES};
use crate::engine::{AlertKind, Engine, Filters};
use crate::health::CheckResult;
use crate::scraper::{Fetched, Ticker};
use crate::session::Session;
use crate::stream::{self, StreamStatus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub struct Column {
    pub key: &'static str,
    pub title: &'static str,
    pub width: u16,
    pub priority: u8,
}

pub const COLUMNS: [Column; 8] = [
    Column { key: "mov", title: "Mov", width: 5, priority: 2 },
    Column { key: "symbol", title: "Symbol", width: 7, priority: 1 },
    Column { key: "price", title: "Price", width: 9, priority: 1 },
    Column { key: "change", title: "% Chg", width: 9, priority: 1 },
    Column { key: "volume", title: "Volume", width: 9, priority: 2 },
    Column { key: "mcap", title: "Mkt Cap", width: 9, priority: 3 },
    Column { key: "delta", title: "Δ Vol", width: 10, priority: 3 },
    Column { key: "status", title: "Status", width: 7, priority: 1 },
];

#[derive(Clone, Copy)]
pub enum FieldKind {
    Smart,
    Float,
    Int,
}

pub struct Field {
    pub label: &'static str,
    pub desc: &'static str,
    pub kind: FieldKind,
}

pub const FIELDS: [Field; 16] = [
    Field { label: "Min Market Cap", desc: "Minimum market cap to display. K/M/B suffixes allowed (500K, 1M). 0 disables.", kind: FieldKind::Smart },
    Field { label: "Max Market Cap", desc: "Maximum market cap to display. K/M/B suffixes allowed (1B). 0 disables.", kind: FieldKind::Smart },
    Field { label: "Spike Vol Ratio", desc: "Volume multiple over the baseline window that fires SPIKE. 2.0 = volume doubled.", kind: FieldKind::Float },
    Field { label: "Pump Price Pct", desc: "Price return over the baseline window that fires PUMP. 0.05 = +5%.", kind: FieldKind::Float },
    Field { label: "Refresh Rate (s)", desc: "How often to poll Webull, in seconds.", kind: FieldKind::Int },
    Field { label: "Rank Track (s)", desc: "Window for rank movement (Mov column). 60 = compare to rank 60s ago.", kind: FieldKind::Int },
    Field { label: "Baseline Window (s)", desc: "Lookback used as the baseline for SPIKE and PUMP detection.", kind: FieldKind::Int },
    Field { label: "Min Vol Alert", desc: "Minimum volume required before SPIKE/PUMP can fire.", kind: FieldKind::Int },
    Field { label: "Min Price Alert ($)", desc: "Minimum price required before SPIKE/PUMP can fire.", kind: FieldKind::Float },
    Field { label: "Delta Window (s)", desc: "Window for the volume delta column. 180 = volume added in the last 3 minutes.", kind: FieldKind::Int },
    Field { label: "Min % Change", desc: "Only show tickers up at least this %. 0 disables.", kind: FieldKind::Float },
    Field { label: "Max % Change", desc: "Only show tickers up at most this %. 0 disables.", kind: FieldKind::Float },
    Field { label: "Min Volume", desc: "Only show tickers with at least this volume. 0 disables.", kind: FieldKind::Int },
    Field { label: "Max Volume", desc: "Only show tickers with at most this volume. 0 disables.", kind: FieldKind::Int },
    Field { label: "Min Price", desc: "Only show tickers priced at least this. 0 disables.", kind: FieldKind::Float },
    Field { label: "Max Price", desc: "Only show tickers priced at most this. 0 disables.", kind: FieldKind::Float },
];

pub const BOOT_ITEMS: [&str; 3] = ["Start Volume Screener", "Health Check", "Settings"];
pub const SETUP_ITEMS: [(&str, &str); 3] = [
    ("Configuration", "Thresholds, filters and windows, with live filter preview"),
    ("Ticker Blacklist", "Hide tickers from ranking (still tracked, listed separately)"),
    ("Alert Filters", "Which alert types appear under ALL and in the f cycle"),
];
pub const BL_COLS: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Boot,
    Health,
    Dashboard,
    SetupMenu,
    SetupConfig,
    SetupBlacklist,
    SetupFilters,
    Columns,
    Chart,
}

/// While the stream is healthy the REST list is only refreshed this often (membership, market cap, prev close).
pub const LIST_REFRESH: u64 = 20;
/// No stream message or status for this long means the stream is down.
const STREAM_STALE: Duration = Duration::from_secs(15);

pub enum Msg {
    Fetched(Result<Fetched, String>),
    Quotes(Vec<stream::Update>),
    Stream(StreamStatus),
    Health(Vec<CheckResult>),
}

pub struct App {
    pub cfg: Config,
    pub config_path: PathBuf,
    pub engine: Engine,
    pub view: View,
    pub quit: bool,
    pub started_at: Instant,

    fetch_tx: Sender<()>,
    health_tx: Sender<()>,
    stream_tx: Option<Sender<stream::Cmd>>,
    stream: StreamStatus,
    stream_status_at: Option<Instant>,
    last_fetch: Option<Instant>,
    fetching: bool,
    running: bool,
    pub next_refresh: Instant,

    pub tickers: Vec<Ticker>,
    pub blacklisted: Vec<Ticker>,
    pub all: Vec<Ticker>,
    pub group: String,
    pub session: Session,
    pub last_update: Option<String>,
    pub last_alert_at: Option<Instant>,
    pub error: Option<String>,

    pub sort_key: &'static str,
    pub sort_desc: bool,
    pub sel_col: usize,
    pub visible_cols: Vec<usize>,
    pub row: usize,
    pub alert_filter: usize,

    pub boot_idx: usize,
    pub health_running: bool,
    pub health: Vec<CheckResult>,

    pub setup_idx: usize,
    pub inputs: Vec<String>,
    pub focus: usize,
    pub config_err: Option<String>,
    pub snapshot: Vec<Ticker>,
    pub bl_cursor: usize,
    pub bl_input: Option<String>,
    pub bl_err: Option<String>,
    pub filter_cursor: usize,
    pub col_cursor: usize,
    pub chart_symbol: String,
    pub sel_symbol: Option<String>,
}

impl App {
    pub fn new(
        cfg: Config,
        config_path: PathBuf,
        fetch_tx: Sender<()>,
        health_tx: Sender<()>,
        stream_tx: Option<Sender<stream::Cmd>>,
    ) -> App {
        let now = Instant::now();
        App {
            engine: Engine::new(&cfg),
            inputs: inputs_from(&cfg),
            cfg,
            config_path,
            view: View::Boot,
            quit: false,
            started_at: now,
            fetch_tx,
            health_tx,
            stream_tx,
            stream: StreamStatus::default(),
            stream_status_at: None,
            last_fetch: None,
            fetching: false,
            running: false,
            next_refresh: now,
            tickers: Vec::new(),
            blacklisted: Vec::new(),
            all: Vec::new(),
            group: String::new(),
            session: Session::now(),
            last_update: None,
            last_alert_at: None,
            error: None,
            sort_key: "change",
            sort_desc: true,
            sel_col: 3,
            visible_cols: (0..COLUMNS.len()).collect(),
            row: 0,
            alert_filter: 0,
            boot_idx: 0,
            health_running: false,
            health: Vec::new(),
            setup_idx: 0,
            focus: 0,
            config_err: None,
            snapshot: Vec::new(),
            bl_cursor: 0,
            bl_input: None,
            bl_err: None,
            filter_cursor: 0,
            col_cursor: 0,
            chart_symbol: String::new(),
            sel_symbol: None,
        }
    }

    pub fn countdown(&self) -> u64 {
        self.next_refresh.saturating_duration_since(Instant::now()).as_secs()
    }

    pub fn is_fetching(&self) -> bool {
        self.fetching
    }

    pub fn stream_enabled(&self) -> bool {
        self.stream_tx.is_some()
    }

    /// True while the stream is connected and delivering (or just connected and not yet silent too long).
    pub fn stream_ok(&self) -> bool {
        let now = Instant::now();
        let fresh = |t: Instant| now.saturating_duration_since(t) <= STREAM_STALE;
        self.stream_enabled()
            && self.stream.connected
            && self.stream_status_at.is_some_and(fresh)
            && self.stream.last_msg.is_none_or(fresh)
    }

    pub fn stream_status(&self) -> &StreamStatus {
        &self.stream
    }

    /// Seconds between REST list fetches: slow while the stream is healthy, the fallback poll otherwise.
    pub fn poll_interval(&self) -> u64 {
        if self.stream_ok() { LIST_REFRESH } else { self.cfg.refresh_interval_seconds.max(1) }
    }

    pub fn on_tick(&mut self, now: Instant) {
        if let Some(last) = self.last_fetch {
            self.next_refresh = last + Duration::from_secs(self.poll_interval());
        }
        if self.running && !self.fetching && now >= self.next_refresh {
            self.request_fetch();
        }
    }

    fn request_fetch(&mut self) {
        if !self.fetching && self.fetch_tx.send(()).is_ok() {
            self.fetching = true;
        }
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Fetched(res) => {
                self.fetching = false;
                self.last_fetch = Some(Instant::now());
                match res {
                    Err(e) => self.error = Some(e),
                    Ok(f) => {
                        self.error = None;
                        if let Some(tx) = &self.stream_tx {
                            let ids = f.tickers.iter().map(|t| t.ticker_id).filter(|&i| i != 0).collect();
                            let _ = tx.send(stream::Cmd::SetTickers(ids));
                        }
                        self.all = f.tickers.clone();
                        self.group = f.group;
                        self.session = f.session;
                        let now = Instant::now();
                        let out = self.engine.process(f.tickers, self.sort_key, self.sort_desc, now);
                        self.tickers = out.active;
                        self.blacklisted = out.blacklisted;
                        if out.new_alerts > 0 {
                            self.last_alert_at = Some(now);
                        }
                        self.resync_row();
                        self.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                    }
                }
            }
            Msg::Quotes(updates) => {
                if self.engine.apply_quotes(&updates, Session::now(), Instant::now()) {
                    self.reapply_local();
                    self.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
            }
            Msg::Stream(st) => {
                self.stream = st;
                self.stream_status_at = Some(Instant::now());
            }
            Msg::Health(results) => {
                self.health_running = false;
                self.health = results;
            }
        }
    }

    pub fn filter_name(&self) -> &'static str {
        FILTER_NAMES[self.alert_filter]
    }

    pub fn alert_visible(&self, kind: AlertKind) -> bool {
        match self.filter_name() {
            "ALL" => self.cfg.filter_on(kind.label()),
            name => kind.label() == name,
        }
    }

    /// Filters built from the unsaved config inputs, for the live preview.
    pub fn preview_filters(&self) -> Filters {
        let v = |i: usize| parse_field(&self.inputs[i], FIELDS[i].kind).unwrap_or(0.0);
        Filters {
            min_mcap: v(0),
            max_mcap: v(1),
            min_change_pct: v(10),
            max_change_pct: v(11),
            min_volume: v(12) as i64,
            max_volume: v(13) as i64,
            min_price: v(14),
            max_price: v(15),
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
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
        }
    }

    fn start(&mut self) {
        self.view = View::Dashboard;
        if !self.running {
            self.running = true;
            self.request_fetch();
        }
    }

    fn save_config(&mut self) {
        self.cfg.blacklisted_tickers = self.engine.blacklist.iter().cloned().collect();
        if let Err(e) = self.cfg.save(&self.config_path) {
            self.error = Some(format!("save config: {e}"));
        }
    }

    fn back_from_setup(&mut self) {
        self.view = if self.running { View::Dashboard } else { View::Boot };
    }

    fn key_boot(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.boot_idx = self.boot_idx.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.boot_idx = (self.boot_idx + 1).min(BOOT_ITEMS.len() - 1),
            KeyCode::Enter | KeyCode::Char(' ') => match self.boot_idx {
                0 => self.start(),
                1 => self.view = View::Health,
                _ => self.open_setup(),
            },
            _ => {}
        }
    }

    fn key_health(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') => {
                self.view = View::Boot;
                self.health.clear();
            }
            KeyCode::Enter | KeyCode::Char(' ') if !self.health_running => {
                if self.health_tx.send(()).is_ok() {
                    self.health_running = true;
                }
            }
            _ => {}
        }
    }

    fn key_dashboard(&mut self, k: KeyEvent) {
        let ncols = self.visible_cols.len();
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('r') => self.request_fetch(),
            KeyCode::Left | KeyCode::Char('h') => self.sel_col = self.sel_col.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => self.sel_col = (self.sel_col + 1).min(ncols.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => self.move_row(self.row.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_row(self.row + 1)
            }
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
            KeyCode::Char('o') | KeyCode::Char('c') => self.open_setup(),
            KeyCode::Char('v') => {
                self.view = View::Columns;
                self.col_cursor = 0;
            }
            _ => {}
        }
    }

    fn move_row(&mut self, row: usize) {
        self.row = row.min(self.tickers.len().saturating_sub(1));
        self.sel_symbol = self.tickers.get(self.row).map(|t| t.symbol.clone());
    }

    /// Re-derive the cursor row from the selected symbol after the list changes.
    fn resync_row(&mut self) {
        let pos = self
            .sel_symbol
            .as_ref()
            .and_then(|s| self.tickers.iter().position(|t| &t.symbol == s));
        match pos {
            Some(i) => self.row = i,
            None => self.move_row(self.row),
        }
    }

    fn key_chart(&mut self, k: KeyEvent) {
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
            _ => {}
        }
    }

    fn open_setup(&mut self) {
        self.view = View::SetupMenu;
        self.setup_idx = 0;
    }

    fn key_setup_menu(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') => self.back_from_setup(),
            KeyCode::Up | KeyCode::Char('k') => self.setup_idx = self.setup_idx.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.setup_idx = (self.setup_idx + 1).min(SETUP_ITEMS.len() - 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.snapshot = self.all.clone();
                match self.setup_idx {
                    0 => {
                        self.inputs = inputs_from(&self.cfg);
                        self.focus = 0;
                        self.config_err = None;
                        self.view = View::SetupConfig;
                    }
                    1 => {
                        self.bl_cursor = 0;
                        self.bl_input = None;
                        self.bl_err = None;
                        self.view = View::SetupBlacklist;
                    }
                    _ => {
                        self.filter_cursor = 0;
                        self.view = View::SetupFilters;
                    }
                }
            }
            _ => {}
        }
    }

    fn key_setup_config(&mut self, k: KeyEvent) {
        let n = FIELDS.len();
        match k.code {
            KeyCode::Esc => self.view = View::SetupMenu,
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + n - 1) % n,
            KeyCode::Char('s') if k.modifiers.contains(KeyModifiers::CONTROL) => self.commit_config(),
            KeyCode::Enter => self.commit_config(),
            KeyCode::Backspace => {
                self.inputs[self.focus].pop();
            }
            KeyCode::Char(c) if self.inputs[self.focus].len() < 20 && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.inputs[self.focus].push(c)
            }
            _ => {}
        }
    }

    fn commit_config(&mut self) {
        let mut vals = [0f64; 16];
        for (i, f) in FIELDS.iter().enumerate() {
            match parse_field(&self.inputs[i], f.kind) {
                Some(v) => vals[i] = v,
                None => {
                    self.focus = i;
                    self.config_err = Some(format!("Invalid {}", f.label));
                    return;
                }
            }
        }
        let c = &mut self.cfg;
        c.min_market_cap = vals[0];
        c.max_market_cap = vals[1];
        c.spike_vol_ratio = vals[2];
        c.pump_price_pct = vals[3];
        c.refresh_interval_seconds = vals[4] as u64;
        c.rank_tracking_interval_seconds = vals[5] as u64;
        c.baseline_window_sec = vals[6] as u64;
        c.min_volume_for_alert = vals[7] as i64;
        c.min_price_for_alert = vals[8];
        c.delta_window_seconds = vals[9] as u64;
        c.min_change_percent = vals[10];
        c.max_change_percent = vals[11];
        c.min_volume = vals[12] as i64;
        c.max_volume = vals[13] as i64;
        c.min_price = vals[14];
        c.max_price = vals[15];
        c.validate();
        self.engine.apply_config(&self.cfg);
        self.inputs = inputs_from(&self.cfg);
        self.config_err = None;
        self.save_config();
        self.view = View::SetupMenu;
        self.reapply_local();
    }

    /// Re-filter and re-sort the last fetched data without a network fetch.
    fn reapply_local(&mut self) {
        let out = self.engine.reapply(self.sort_key, self.sort_desc);
        self.tickers = out.active;
        self.blacklisted = out.blacklisted;
        self.resync_row();
    }

    fn key_blacklist(&mut self, k: KeyEvent) {
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

    fn key_filters(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Up | KeyCode::Char('k') => self.filter_cursor = self.filter_cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.filter_cursor = (self.filter_cursor + 1).min(FILTER_NAMES.len() - 1)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let name = FILTER_NAMES[self.filter_cursor];
                let on = self.cfg.filter_on(name);
                self.cfg.filter_enabled.insert(name.to_string(), !on);
                self.save_config();
            }
            _ => {}
        }
    }

    fn key_columns(&mut self, k: KeyEvent) {
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

fn inputs_from(c: &Config) -> Vec<String> {
    vec![
        format!("{:.0}", c.min_market_cap),
        format!("{:.0}", c.max_market_cap),
        format!("{:.2}", c.spike_vol_ratio),
        format!("{:.2}", c.pump_price_pct),
        c.refresh_interval_seconds.to_string(),
        c.rank_tracking_interval_seconds.to_string(),
        c.baseline_window_sec.to_string(),
        c.min_volume_for_alert.to_string(),
        format!("{:.2}", c.min_price_for_alert),
        c.delta_window_seconds.to_string(),
        format!("{:.1}", c.min_change_percent),
        format!("{:.1}", c.max_change_percent),
        c.min_volume.to_string(),
        c.max_volume.to_string(),
        format!("{:.2}", c.min_price),
        format!("{:.2}", c.max_price),
    ]
}

pub fn parse_field(s: &str, kind: FieldKind) -> Option<f64> {
    let s = s.trim();
    match kind {
        FieldKind::Smart => parse_smart(s),
        FieldKind::Float => s.parse::<f64>().ok().filter(|v| v.is_finite()),
        FieldKind::Int => s.parse::<u64>().ok().map(|v| v as f64),
    }
}

/// Parses numbers with optional K/M/B/T suffix and thousands separators.
pub fn parse_smart(s: &str) -> Option<f64> {
    let s = s.trim().to_uppercase().replace([',', ' '], "");
    if s.is_empty() {
        return Some(0.0);
    }
    let (num, mult) = match s.chars().last() {
        Some('K') => (&s[..s.len() - 1], 1e3),
        Some('M') => (&s[..s.len() - 1], 1e6),
        Some('B') => (&s[..s.len() - 1], 1e9),
        Some('T') => (&s[..s.len() - 1], 1e12),
        _ => (s.as_str(), 1.0),
    };
    num.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v * mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_parse() {
        assert_eq!(parse_smart("500K"), Some(500_000.0));
        assert_eq!(parse_smart("1.5m"), Some(1_500_000.0));
        assert_eq!(parse_smart("2,000"), Some(2000.0));
        assert_eq!(parse_smart(""), Some(0.0));
        assert_eq!(parse_smart("abc"), None);
    }

    #[test]
    fn int_fields_reject_negative_and_decimals() {
        assert_eq!(parse_field("5", FieldKind::Int), Some(5.0));
        assert_eq!(parse_field("-5", FieldKind::Int), None);
        assert_eq!(parse_field("5.5", FieldKind::Int), None);
    }

    #[test]
    fn cursor_follows_selected_symbol_across_resort() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
        let mk = |s: &str| Ticker { symbol: s.into(), ..Default::default() };
        app.tickers = vec![mk("A"), mk("B"), mk("C")];
        app.move_row(1);
        assert_eq!(app.sel_symbol.as_deref(), Some("B"));
        app.tickers = vec![mk("C"), mk("A"), mk("B")];
        app.resync_row();
        assert_eq!(app.row, 2);
        app.tickers = vec![mk("C"), mk("A")];
        app.resync_row();
        assert_eq!(app.row, 1);
        assert_eq!(app.sel_symbol.as_deref(), Some("A"));
    }
}
