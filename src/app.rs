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
    Column { key: "volrate", title: "Vol/m", width: 10, priority: 3 },
    Column { key: "status", title: "Status", width: 7, priority: 1 },
];

#[derive(Clone, Copy)]
pub enum FieldKind {
    Smart,
    Float,
    Int,
    /// One of a fixed list of values, picked with left/right/space instead of typed.
    Choice(&'static [u64]),
}

/// Name of a Settings field. Everything that needs a field looks it up by name, never by position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldId {
    MinMcap,
    MaxMcap,
    FallbackPoll,
    RankTrack,
    RecomputeMs,
    VolMWindow,
    SetupWindow,
    SetupVolRatio,
    SetupPricePct,
    MinVolAlert,
    MinPriceAlert,
    AlertCooldown,
    MinChange,
    MaxChange,
    MinVolume,
    MaxVolume,
    MinPrice,
    MaxPrice,
}

pub struct Field {
    pub id: FieldId,
    pub label: &'static str,
    pub desc: &'static str,
    pub kind: FieldKind,
}

pub const RECOMPUTE_CHOICES: &[u64] = &[500, 1000];

pub const FIELDS: [Field; 18] = [
    Field { id: FieldId::MinMcap, label: "Min Market Cap", desc: "Minimum market cap to display. K/M/B suffixes allowed (500K, 1M). 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::MaxMcap, label: "Max Market Cap", desc: "Maximum market cap to display. K/M/B suffixes allowed (1B). 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::FallbackPoll, label: "Fallback Poll (s)", desc: "How often to poll Webull over REST, in seconds. Used only while the live stream is down.", kind: FieldKind::Int },
    Field { id: FieldId::RankTrack, label: "Rank Track (s)", desc: "Window for rank movement (Mov column). 60 = compare to rank 60s ago. Min 5.", kind: FieldKind::Int },
    Field { id: FieldId::RecomputeMs, label: "Recompute (ms)", desc: "How often the table re-ranks and re-sorts. Left/right/space to switch between 500 and 1000.", kind: FieldKind::Choice(RECOMPUTE_CHOICES) },
    Field { id: FieldId::VolMWindow, label: "Vol/m Window (s)", desc: "Window for the Vol/m column (min 30). 180 = average shares per minute over the last 3 minutes.", kind: FieldKind::Int },
    Field { id: FieldId::SetupWindow, label: "SETUP Window (s)", desc: "How far back SETUP compares volume and price. 300 = versus 5 minutes ago.", kind: FieldKind::Int },
    Field { id: FieldId::SetupVolRatio, label: "SETUP Volume Ratio", desc: "SETUP needs volume at least this multiple of the volume one SETUP window ago. 2.2 = 2.2x.", kind: FieldKind::Float },
    Field { id: FieldId::SetupPricePct, label: "SETUP Price %", desc: "SETUP also needs price up at least this percent over the SETUP window. 10 = +10%.", kind: FieldKind::Float },
    Field { id: FieldId::MinVolAlert, label: "Min Vol Alert", desc: "Minimum volume required before SETUP can fire.", kind: FieldKind::Int },
    Field { id: FieldId::MinPriceAlert, label: "Min Price Alert ($)", desc: "Minimum price required before SETUP can fire.", kind: FieldKind::Float },
    Field { id: FieldId::AlertCooldown, label: "Alert Cooldown (s)", desc: "Seconds before the same alert kind can fire again for one symbol. 120 = 2 minutes.", kind: FieldKind::Int },
    Field { id: FieldId::MinChange, label: "Min % Change", desc: "Only show tickers up at least this %. 0 disables.", kind: FieldKind::Float },
    Field { id: FieldId::MaxChange, label: "Max % Change", desc: "Only show tickers up at most this %. 0 disables.", kind: FieldKind::Float },
    Field { id: FieldId::MinVolume, label: "Min Volume", desc: "Only show tickers with at least this volume. 0 disables.", kind: FieldKind::Int },
    Field { id: FieldId::MaxVolume, label: "Max Volume", desc: "Only show tickers with at most this volume. 0 disables.", kind: FieldKind::Int },
    Field { id: FieldId::MinPrice, label: "Min Price", desc: "Only show tickers priced at least this. 0 disables.", kind: FieldKind::Float },
    Field { id: FieldId::MaxPrice, label: "Max Price", desc: "Only show tickers priced at most this. 0 disables.", kind: FieldKind::Float },
];

/// Position of a field in `FIELDS` (and in the input list).
pub fn field_index(id: FieldId) -> usize {
    FIELDS.iter().position(|f| f.id == id).expect("every FieldId is listed in FIELDS")
}

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
    last_ok_fetch: Option<Instant>,
    /// Last recompute tick (re-rank, filter grace, Vol/m, re-sort).
    last_recompute: Instant,
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
            last_ok_fetch: None,
            last_recompute: now,
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

    /// Tells the stream which tickers to hold: the list plus lingering ones.
    fn send_subscriptions(&self) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.send(stream::Cmd::SetTickers(self.engine.subscribe_ids()));
        }
    }

    /// No fresh data: the stream is down and the REST list has not answered for a while.
    fn feed_stale(&self, now: Instant) -> bool {
        if self.stream_ok() {
            return false;
        }
        let limit = Duration::from_secs((self.poll_interval() * 3).max(10));
        self.last_ok_fetch.is_none_or(|t| now.saturating_duration_since(t) > limit)
    }

    pub fn on_tick(&mut self, now: Instant) {
        self.engine.set_rest_feeds_series(!self.stream_ok());
        let stale = self.feed_stale(now);
        self.engine.set_feed_stale(stale);
        if now.saturating_duration_since(self.last_recompute) >= self.recompute_interval() {
            self.recompute(now);
        }
        if self.engine.expire_series(now) {
            self.send_subscriptions();
        }
        if let Some(last) = self.last_fetch {
            self.next_refresh = last + Duration::from_secs(self.poll_interval());
        }
        if self.running && !self.fetching && now >= self.next_refresh {
            self.request_fetch();
        }
    }

    fn recompute_interval(&self) -> Duration {
        Duration::from_millis(self.cfg.recompute_ms)
    }

    /// The recompute tick: Vol/m, Mov and filter grace are refreshed and the table re-sorted.
    /// Stream batches only update data; the display follows here.
    fn recompute(&mut self, now: Instant) {
        self.last_recompute = now;
        self.engine.refresh_vol_rates(now);
        // Alerts raised here carry `now`, so the alerts table can flash the newest one.
        if self.engine.recompute(now) > 0 {
            self.last_alert_at = Some(now);
        }
        self.reapply_local();
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
                        self.last_ok_fetch = Some(Instant::now());
                        self.all = f.tickers.clone();
                        self.group = f.group;
                        if f.session != self.session {
                            self.engine.on_session_change();
                        }
                        self.session = f.session;
                        let now = Instant::now();
                        let out = self.engine.process(f.tickers, self.sort_key, self.sort_desc, now);
                        self.tickers = out.active;
                        self.blacklisted = out.blacklisted;
                        // process() re-ranked the board, so this counts as a recompute tick.
                        self.last_recompute = now;
                        self.send_subscriptions();
                        if out.new_alerts > 0 {
                            self.last_alert_at = Some(now);
                        }
                        self.resync_row();
                        self.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                    }
                }
            }
            Msg::Quotes(updates) => {
                // Data only: the table is re-ranked and re-sorted on the recompute tick.
                if self.engine.apply_quotes(&updates, Session::now(), Instant::now()) {
                    self.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
            }
            Msg::Stream(st) => {
                if st.connections != self.stream.connections {
                    self.engine.on_stream_reconnect();
                }
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
        filters_from_inputs(&self.inputs)
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

    /// Switches the rate panel between New Highs/Lows and Up/Down ticks and remembers the choice.
    fn toggle_rate_tab(&mut self) {
        self.cfg.rate_tab = self.cfg.rate_tab.toggled();
        self.save_config();
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
            KeyCode::Char('t') => self.toggle_rate_tab(),
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
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if matches!(FIELDS[self.focus].kind, FieldKind::Choice(_)) => {
                if let FieldKind::Choice(opts) = FIELDS[self.focus].kind {
                    let back = k.code == KeyCode::Left;
                    self.inputs[self.focus] = cycle_choice(&self.inputs[self.focus], opts, back);
                }
            }
            // A choice field takes no typing.
            KeyCode::Backspace | KeyCode::Char(_) if matches!(FIELDS[self.focus].kind, FieldKind::Choice(_)) => {}
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
        match config_from_inputs(&self.cfg, &self.inputs) {
            Ok(cfg) => self.cfg = cfg,
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
    }

    /// Re-filter and re-sort the last fetched data without a network fetch.
    fn reapply_local(&mut self) {
        let out = self.engine.reapply(self.sort_key, self.sort_desc, Instant::now());
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

/// Text shown in the input box for one field.
fn input_text(c: &Config, id: FieldId) -> String {
    match id {
        FieldId::MinMcap => format!("{:.0}", c.min_market_cap),
        FieldId::MaxMcap => format!("{:.0}", c.max_market_cap),
        FieldId::FallbackPoll => c.refresh_interval_seconds.to_string(),
        FieldId::RankTrack => c.rank_tracking_interval_seconds.to_string(),
        FieldId::RecomputeMs => c.recompute_ms.to_string(),
        FieldId::VolMWindow => c.vol_m_window_seconds.to_string(),
        FieldId::SetupWindow => c.setup_window_sec.to_string(),
        FieldId::SetupVolRatio => format!("{:.2}", c.setup_vol_ratio),
        FieldId::SetupPricePct => format!("{:.2}", c.setup_price_pct),
        FieldId::MinVolAlert => c.min_volume_for_alert.to_string(),
        FieldId::MinPriceAlert => format!("{:.2}", c.min_price_for_alert),
        FieldId::AlertCooldown => c.alert_cooldown_sec.to_string(),
        FieldId::MinChange => format!("{:.1}", c.min_change_percent),
        FieldId::MaxChange => format!("{:.1}", c.max_change_percent),
        FieldId::MinVolume => c.min_volume.to_string(),
        FieldId::MaxVolume => c.max_volume.to_string(),
        FieldId::MinPrice => format!("{:.2}", c.min_price),
        FieldId::MaxPrice => format!("{:.2}", c.max_price),
    }
}

/// Stores one parsed field value in the config (validation happens afterwards, in `Config::validate`).
fn apply_field(c: &mut Config, id: FieldId, v: f64) {
    match id {
        FieldId::MinMcap => c.min_market_cap = v,
        FieldId::MaxMcap => c.max_market_cap = v,
        FieldId::FallbackPoll => c.refresh_interval_seconds = v as u64,
        FieldId::RankTrack => c.rank_tracking_interval_seconds = v as u64,
        FieldId::RecomputeMs => c.recompute_ms = v as u64,
        FieldId::VolMWindow => c.vol_m_window_seconds = v as u64,
        FieldId::SetupWindow => c.setup_window_sec = v as u64,
        FieldId::SetupVolRatio => c.setup_vol_ratio = v,
        FieldId::SetupPricePct => c.setup_price_pct = v,
        FieldId::MinVolAlert => c.min_volume_for_alert = v as i64,
        FieldId::MinPriceAlert => c.min_price_for_alert = v,
        FieldId::AlertCooldown => c.alert_cooldown_sec = v as u64,
        FieldId::MinChange => c.min_change_percent = v,
        FieldId::MaxChange => c.max_change_percent = v,
        FieldId::MinVolume => c.min_volume = v as i64,
        FieldId::MaxVolume => c.max_volume = v as i64,
        FieldId::MinPrice => c.min_price = v,
        FieldId::MaxPrice => c.max_price = v,
    }
}

fn inputs_from(c: &Config) -> Vec<String> {
    FIELDS.iter().map(|f| input_text(c, f.id)).collect()
}

/// The config after applying every input to `base` and validating. `Err(i)` is the first field
/// whose text does not parse.
fn config_from_inputs(base: &Config, inputs: &[String]) -> Result<Config, usize> {
    let mut c = base.clone();
    for (i, f) in FIELDS.iter().enumerate() {
        let v = parse_field(&inputs[i], f.kind).ok_or(i)?;
        apply_field(&mut c, f.id, v);
    }
    c.validate();
    Ok(c)
}

/// Display filters from the (possibly unsaved) inputs; unparseable fields count as 0 (off).
fn filters_from_inputs(inputs: &[String]) -> Filters {
    let v = |id: FieldId| {
        let i = field_index(id);
        parse_field(&inputs[i], FIELDS[i].kind).unwrap_or(0.0)
    };
    Filters {
        min_mcap: v(FieldId::MinMcap),
        max_mcap: v(FieldId::MaxMcap),
        min_change_pct: v(FieldId::MinChange),
        max_change_pct: v(FieldId::MaxChange),
        min_volume: v(FieldId::MinVolume) as i64,
        max_volume: v(FieldId::MaxVolume) as i64,
        min_price: v(FieldId::MinPrice),
        max_price: v(FieldId::MaxPrice),
    }
}

/// Next (or previous) option after the one in `cur`; an unknown value goes to the first option.
fn cycle_choice(cur: &str, opts: &[u64], back: bool) -> String {
    let n = opts.len();
    let next = match cur.trim().parse::<u64>().ok().and_then(|v| opts.iter().position(|&o| o == v)) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None => 0,
    };
    opts[next].to_string()
}

pub fn parse_field(s: &str, kind: FieldKind) -> Option<f64> {
    let s = s.trim();
    match kind {
        FieldKind::Smart => parse_smart(s),
        FieldKind::Float => s.parse::<f64>().ok().filter(|v| v.is_finite()),
        FieldKind::Int => s.parse::<u64>().ok().map(|v| v as f64),
        FieldKind::Choice(opts) => s.parse::<u64>().ok().filter(|v| opts.contains(v)).map(|v| v as f64),
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

    #[test]
    fn stream_batches_wait_for_the_recompute_tick() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let cfg = Config { recompute_ms: 500, max_market_cap: 0.0, ..Config::default() };
        let mut app = App::new(cfg, PathBuf::from("unused-test-config.json"), tx, htx, None);
        let row = |s: &str, id: u64, chg: f64| Ticker {
            ticker_id: id,
            symbol: s.into(),
            price: 1.0 + chg,
            prev_close: 1.0,
            volume: 1000,
            change_ratio: chg,
            ..Default::default()
        };
        let base = app.last_recompute;
        let out = app.engine.process(vec![row("A", 1, 0.10), row("B", 2, 0.05)], "change", true, base);
        app.tickers = out.active;
        assert_eq!(app.tickers[0].symbol, "A");
        // B jumps past A on the stream: the data changes, the table does not
        let quote = stream::Update {
            id: 2,
            trade_ms: chrono::Utc::now().timestamp_millis(),
            seq: 1,
            close: Some(2.0),
            p_price: None,
            change_ratio: None,
            volume: Some(2000),
            mcap: None,
            high: None,
            low: None,
            status: String::new(),
            recv: Instant::now(),
        };
        app.on_msg(Msg::Quotes(vec![quote]));
        assert_eq!(app.tickers[0].symbol, "A");
        app.on_tick(base + Duration::from_millis(400));
        assert_eq!(app.tickers[0].symbol, "A");
        // the tick (500 ms here) re-sorts
        app.on_tick(base + Duration::from_millis(500));
        assert_eq!(app.tickers[0].symbol, "B");
        assert_eq!(app.last_recompute, base + Duration::from_millis(500));
    }

    fn set(inputs: &mut [String], id: FieldId, v: &str) {
        inputs[field_index(id)] = v.to_string();
    }

    fn test_app(cfg: Config) -> (App, std::sync::mpsc::Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        (App::new(cfg, PathBuf::from("target/test-config/unused-app-config.json"), tx, htx, None), rx)
    }

    #[test]
    fn settings_labels_are_the_planned_ones() {
        let labels: Vec<&str> = FIELDS.iter().map(|f| f.label).collect();
        for want in [
            "Fallback Poll (s)",
            "SETUP Window (s)",
            "SETUP Volume Ratio",
            "SETUP Price %",
            "Alert Cooldown (s)",
            "Recompute (ms)",
            "Vol/m Window (s)",
        ] {
            assert!(labels.contains(&want), "missing {want}");
        }
        for gone in ["Refresh Rate (s)", "Spike Vol Ratio", "Pump Price Pct", "Baseline Window", "Delta Window (s)"] {
            assert!(!labels.contains(&gone), "{gone} still listed");
        }
    }

    #[test]
    fn every_field_id_is_listed_once_and_found_by_name() {
        for (i, f) in FIELDS.iter().enumerate() {
            assert_eq!(field_index(f.id), i);
            assert_eq!(FIELDS.iter().filter(|g| g.id == f.id).count(), 1);
        }
        assert_eq!(FIELDS[field_index(FieldId::RecomputeMs)].label, "Recompute (ms)");
        assert_eq!(FIELDS[field_index(FieldId::FallbackPoll)].label, "Fallback Poll (s)");
        assert_eq!(FIELDS[field_index(FieldId::AlertCooldown)].label, "Alert Cooldown (s)");
        assert_eq!(FIELDS[field_index(FieldId::MaxPrice)].label, "Max Price");
    }

    #[test]
    fn every_field_roundtrips_settings_to_config() {
        let c = Config {
            min_market_cap: 5e8,
            max_market_cap: 2e9,
            refresh_interval_seconds: 7,
            rank_tracking_interval_seconds: 45,
            recompute_ms: 500,
            vol_m_window_seconds: 90,
            setup_window_sec: 600,
            setup_vol_ratio: 3.5,
            setup_price_pct: 7.25,
            min_volume_for_alert: 25_000,
            min_price_for_alert: 0.5,
            alert_cooldown_sec: 45,
            min_change_percent: 2.5,
            max_change_percent: 80.5,
            min_volume: 1_000,
            max_volume: 90_000_000,
            min_price: 0.25,
            max_price: 20.0,
            ..Config::default()
        };
        let inputs = inputs_from(&c);
        assert_eq!(inputs.len(), FIELDS.len());
        // starting from different values, the inputs bring every field to `c`
        let got = config_from_inputs(&Config::default(), &inputs).unwrap();
        assert_eq!(got, c);
        // fields outside Settings are left alone
        let base = Config { blacklisted_tickers: vec!["ZZZ".into()], stream_enabled: false, ..Config::default() };
        let got = config_from_inputs(&base, &inputs).unwrap();
        assert_eq!(got.blacklisted_tickers, vec!["ZZZ".to_string()]);
        assert!(!got.stream_enabled);
    }

    #[test]
    fn default_config_survives_a_settings_roundtrip() {
        let c = Config::default();
        assert_eq!(config_from_inputs(&c, &inputs_from(&c)).unwrap(), c);
    }

    #[test]
    fn out_of_range_inputs_clamp_like_validate() {
        let mut inputs = inputs_from(&Config::default());
        set(&mut inputs, FieldId::FallbackPoll, "0");
        set(&mut inputs, FieldId::RankTrack, "2");
        set(&mut inputs, FieldId::VolMWindow, "5");
        set(&mut inputs, FieldId::SetupWindow, "0");
        set(&mut inputs, FieldId::AlertCooldown, "0");
        set(&mut inputs, FieldId::SetupVolRatio, "0");
        set(&mut inputs, FieldId::SetupPricePct, "-3");
        let c = config_from_inputs(&Config::default(), &inputs).unwrap();
        assert_eq!(c.refresh_interval_seconds, 1);
        assert_eq!(c.rank_tracking_interval_seconds, 5);
        assert_eq!(c.vol_m_window_seconds, 30);
        assert_eq!(c.setup_window_sec, 300);
        assert_eq!(c.alert_cooldown_sec, 120);
        assert_eq!(c.setup_vol_ratio, 2.2);
        assert_eq!(c.setup_price_pct, 10.0);
    }

    #[test]
    fn unparseable_inputs_are_rejected_with_the_field_index() {
        for (id, bad) in [
            (FieldId::RecomputeMs, "750"),
            (FieldId::RecomputeMs, "abc"),
            (FieldId::RecomputeMs, ""),
            (FieldId::FallbackPoll, "-1"),
            (FieldId::FallbackPoll, "1.5"),
            (FieldId::AlertCooldown, "x"),
            (FieldId::SetupVolRatio, "abc"),
            (FieldId::SetupWindow, ""),
            (FieldId::VolMWindow, "-5"),
        ] {
            let mut inputs = inputs_from(&Config::default());
            set(&mut inputs, id, bad);
            assert_eq!(config_from_inputs(&Config::default(), &inputs), Err(field_index(id)), "{id:?} = {bad:?}");
        }
    }

    #[test]
    fn recompute_is_a_two_way_choice() {
        assert_eq!(parse_field("500", FieldKind::Choice(RECOMPUTE_CHOICES)), Some(500.0));
        assert_eq!(parse_field("1000", FieldKind::Choice(RECOMPUTE_CHOICES)), Some(1000.0));
        assert_eq!(parse_field("250", FieldKind::Choice(RECOMPUTE_CHOICES)), None);
        assert!(matches!(FIELDS[field_index(FieldId::RecomputeMs)].kind, FieldKind::Choice(o) if o == [500, 1000]));
        assert_eq!(cycle_choice("1000", RECOMPUTE_CHOICES, false), "500");
        assert_eq!(cycle_choice("500", RECOMPUTE_CHOICES, false), "1000");
        assert_eq!(cycle_choice("500", RECOMPUTE_CHOICES, true), "1000");
        assert_eq!(cycle_choice("junk", RECOMPUTE_CHOICES, false), "500");
    }

    #[test]
    fn recompute_field_toggles_with_keys_and_ignores_typing() {
        use crossterm::event::KeyEvent;
        let (mut app, _rx) = test_app(Config::default());
        app.view = View::SetupConfig;
        app.focus = field_index(FieldId::RecomputeMs);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        app.on_key(key(KeyCode::Char('7')));
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.inputs[app.focus], "1000");
        app.on_key(key(KeyCode::Right));
        assert_eq!(app.inputs[app.focus], "500");
        app.on_key(key(KeyCode::Char(' ')));
        assert_eq!(app.inputs[app.focus], "1000");
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.inputs[app.focus], "500");
    }

    #[test]
    fn preview_filters_read_fields_by_name() {
        let mut inputs = inputs_from(&Config::default());
        set(&mut inputs, FieldId::MinMcap, "500K");
        set(&mut inputs, FieldId::MaxMcap, "2B");
        set(&mut inputs, FieldId::MinChange, "5");
        set(&mut inputs, FieldId::MaxChange, "90");
        set(&mut inputs, FieldId::MinVolume, "1000");
        set(&mut inputs, FieldId::MaxVolume, "5000000");
        set(&mut inputs, FieldId::MinPrice, "0.5");
        set(&mut inputs, FieldId::MaxPrice, "15");
        // alert and timing fields must not leak into the filters
        set(&mut inputs, FieldId::SetupVolRatio, "9");
        set(&mut inputs, FieldId::AlertCooldown, "77");
        let f = filters_from_inputs(&inputs);
        assert_eq!(f.min_mcap, 500_000.0);
        assert_eq!(f.max_mcap, 2e9);
        assert_eq!((f.min_change_pct, f.max_change_pct), (5.0, 90.0));
        assert_eq!((f.min_volume, f.max_volume), (1000, 5_000_000));
        assert_eq!((f.min_price, f.max_price), (0.5, 15.0));
        // a half-typed value counts as off
        set(&mut inputs, FieldId::MaxPrice, "1.");
        set(&mut inputs, FieldId::MinPrice, "x");
        assert_eq!(filters_from_inputs(&inputs).min_price, 0.0);
    }

    #[test]
    fn preview_filters_follow_the_app_inputs() {
        let (mut app, _rx) = test_app(Config::default());
        let i = field_index(FieldId::MinPrice);
        app.inputs[i] = "3".into();
        assert_eq!(app.preview_filters().min_price, 3.0);
    }

    #[test]
    fn saving_settings_applies_the_config_without_a_fetch() {
        let (mut app, rx) = test_app(Config { max_market_cap: 0.0, ..Config::default() });
        app.inputs = inputs_from(&app.cfg);
        set(&mut app.inputs, FieldId::AlertCooldown, "30");
        set(&mut app.inputs, FieldId::RecomputeMs, "500");
        set(&mut app.inputs, FieldId::FallbackPoll, "9");
        set(&mut app.inputs, FieldId::SetupWindow, "120");
        app.view = View::SetupConfig;
        app.commit_config();
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.cfg.alert_cooldown_sec, 30);
        assert_eq!(app.cfg.recompute_ms, 500);
        assert_eq!(app.cfg.refresh_interval_seconds, 9);
        assert_eq!(app.cfg.setup_window_sec, 120);
        assert!(rx.try_recv().is_err(), "saving settings must not trigger a fetch");

        // an invalid field keeps the screen open, focuses the field and changes nothing
        app.view = View::SetupConfig;
        set(&mut app.inputs, FieldId::RecomputeMs, "750");
        app.commit_config();
        assert!(app.view == View::SetupConfig);
        assert_eq!(app.focus, field_index(FieldId::RecomputeMs));
        assert_eq!(app.config_err.as_deref(), Some("Invalid Recompute (ms)"));
        assert_eq!(app.cfg.recompute_ms, 500);
    }
}
