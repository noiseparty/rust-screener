use crate::backfill;
use crate::barcache;
use crate::config::{Config, Layout, ALERT_ROWS_MAX, ALERT_ROWS_MIN, FILTER_NAMES};
use crate::engine::MAX_CANDLES;
use crate::engine::{AlertKind, Candle, Engine, Filters};
use crate::fmt::{self, Formats};
use crate::health::CheckResult;
use crate::history::{self, Tf};
use crate::levels;
use crate::presets;
use crate::theme::{self, Palette};
use crate::scraper::{self, Fetched, Float, Ticker};
use crate::session::Session;
use crate::storage;
use crate::store::{self, CandleStore};
use crate::stream::{self, StreamStatus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub struct Column {
    pub key: &'static str,
    pub title: &'static str,
    pub width: u16,
    pub priority: u8,
}

pub const COLUMNS: [Column; 10] = [
    Column { key: "mov", title: "Mov", width: 5, priority: 2 },
    Column { key: "symbol", title: "Symbol", width: 7, priority: 1 },
    Column { key: "price", title: "Price", width: 9, priority: 1 },
    Column { key: "change", title: "% Chg", width: 9, priority: 1 },
    Column { key: "volume", title: "Volume", width: 9, priority: 2 },
    Column { key: "mcap", title: "Mkt Cap", width: 9, priority: 3 },
    Column { key: "float", title: "Float", width: 8, priority: 3 },
    Column { key: "shares", title: "Shares", width: 8, priority: 3 },
    Column { key: "volrate", title: "Vol/m", width: 10, priority: 3 },
    Column { key: "status", title: "Status", width: 7, priority: 1 },
];

#[derive(Clone, Copy, PartialEq)]
pub enum FieldKind {
    Smart,
    Float,
    Int,
    /// One of a fixed list of values, picked with left/right/space instead of typed.
    Choice(&'static [u64]),
    /// On or Off, switched with left/right/space.
    Toggle,
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
    MinFloat,
    MaxFloat,
    MinShares,
    MaxShares,
    TickerTape,
    KeyHints,
    AlertRows,
}

pub struct Field {
    pub id: FieldId,
    pub label: &'static str,
    pub desc: &'static str,
    pub kind: FieldKind,
}

pub const RECOMPUTE_CHOICES: &[u64] = &[500, 1000];
pub const ALERT_ROW_CHOICES: &[u64] = &[5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

pub const FIELDS: [Field; 25] = [
    Field { id: FieldId::MinMcap, label: "Min Market Cap", desc: "Minimum market cap to display. K/M/B suffixes allowed (500K, 1M). 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::MaxMcap, label: "Max Market Cap", desc: "Maximum market cap to display. K/M/B suffixes allowed (1B). 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::FallbackPoll, label: "Fallback Poll (s)", desc: "How often to poll Webull over REST, in seconds. Used only while the live stream is down.", kind: FieldKind::Int },
    Field { id: FieldId::RankTrack, label: "Rank Track (s)", desc: "Window for rank movement (Mov column). 60 = compare to rank 60s ago. Min 5.", kind: FieldKind::Int },
    Field { id: FieldId::RecomputeMs, label: "Recompute (ms)", desc: "How often the table re-ranks and re-sorts. Left/right/space to switch between 500 and 1000.", kind: FieldKind::Choice(RECOMPUTE_CHOICES) },
    Field { id: FieldId::VolMWindow, label: "Vol/m Window (s)", desc: "Window for the Vol/m column (min 30). 180 = average shares per minute over the last 3 minutes.", kind: FieldKind::Int },
    Field { id: FieldId::SetupWindow, label: "SETUP Window (s)", desc: "SETUP window. 300 = volume of the last 5 minutes versus the 5 minutes before, price versus 5 minutes ago.", kind: FieldKind::Int },
    Field { id: FieldId::SetupVolRatio, label: "SETUP Volume Ratio", desc: "SETUP needs the volume of the last window at least this multiple of the window before it. 2.2 = 2.2x.", kind: FieldKind::Float },
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
    Field { id: FieldId::MinFloat, label: "Min Float", desc: "Only show tickers with at least this float (shares). K/M/B suffixes allowed (5M). Unknown floats pass. 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::MaxFloat, label: "Max Float", desc: "Only show tickers with at most this float (shares). K/M/B suffixes allowed (20M). Unknown floats pass. 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::MinShares, label: "Min Shares", desc: "Only show tickers with at least this many total shares. K/M/B suffixes allowed. Unknown counts pass. 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::MaxShares, label: "Max Shares", desc: "Only show tickers with at most this many total shares. K/M/B suffixes allowed. Unknown counts pass. 0 disables.", kind: FieldKind::Smart },
    Field { id: FieldId::TickerTape, label: "Ticker Tape", desc: "The scrolling ticker tape at the bottom of the dashboard. Off gives its row to the list.", kind: FieldKind::Toggle },
    Field { id: FieldId::KeyHints, label: "Key Hints", desc: "The key hints at the bottom of the dashboard. Off gives their row to the list.", kind: FieldKind::Toggle },
    Field { id: FieldId::AlertRows, label: "Alert Rows", desc: "Rows of the alerts panel above the list (5-15). Left/right/space to change.", kind: FieldKind::Choice(ALERT_ROW_CHOICES) },
];

/// Position of a field in `FIELDS` (and in the input list).
pub fn field_index(id: FieldId) -> usize {
    FIELDS.iter().position(|f| f.id == id).expect("every FieldId is listed in FIELDS")
}

/// The app's name: window, title bar, boot screen.
pub const APP_NAME: &str = "MarketGUI";
pub const BOOT_ITEMS: [&str; 2] = ["Start MarketGUI", "Config"];
/// Config sidebar items and their descriptions.
pub const SETUP_ITEMS: [(&str, &str); 9] = [
    ("Configuration", "Thresholds, filters and windows, with live filter preview"),
    ("Presets", "Saved configurations: load one into Configuration, save the current one"),
    ("Theme", "Colour theme: browse with a live dashboard preview, Enter keeps it"),
    ("Time & Format", "Time zone, date and clock, currency and number formats, with a live preview"),
    ("Blacklist", "Hide tickers from ranking (still tracked, listed separately)"),
    ("Alert Filters", "Which alert types appear under ALL and in the f cycle"),
    ("Sound", "Alert sound: mute it, set its volume, play a test"),
    ("Storage", "Ticker data on disk: browse it and delete what you no longer need"),
    ("Health Check", "Connection checks for the network and the stock data source"),
];
/// The settings pane (#2) of each sidebar item, in `SETUP_ITEMS` order.
pub const SETUP_VIEWS: [View; 9] = [
    View::SetupConfig,
    View::Presets,
    View::SetupTheme,
    View::SetupFormat,
    View::SetupBlacklist,
    View::SetupFilters,
    View::SetupSound,
    View::Storage,
    View::Health,
];
/// Volume step of the Sound pane's slider (left/right).
pub const VOLUME_STEP: u8 = 5;
pub const BL_COLS: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Boot,
    Health,
    Dashboard,
    /// Config with the sidebar focused; the other setup views are its focused settings pane.
    SetupMenu,
    SetupConfig,
    SetupBlacklist,
    SetupFilters,
    Columns,
    Chart,
    Storage,
    Presets,
    /// Config -> Theme: the theme list, previewed while browsing.
    SetupTheme,
    /// Config -> Sound: mute and volume of the alert sound.
    SetupSound,
    /// Config -> Time & Format: time zone, date, clock and number formats.
    SetupFormat,
}

/// A Presets action waiting for `y`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresetAsk {
    Overwrite(String),
    Delete(String),
}

/// While the stream is healthy the REST list is only refreshed this often (membership, market cap, prev close).
pub const LIST_REFRESH: u64 = 20;
/// No stream message or status for this long means the stream is down.
const STREAM_STALE: Duration = Duration::from_secs(15);

/// The charted row is backfilled once it has stayed selected this long, so scrolling past rows fetches nothing.
const BACKFILL_DWELL: Duration = Duration::from_secs(1);

/// The header's ticker-data size is re-measured this often.
const STORAGE_POLL: Duration = Duration::from_secs(10);
/// Shortest CPU sample interval (shorter deltas are too noisy to read).
const CPU_POLL: Duration = Duration::from_secs(1);

/// While a 5m/1h/4h/D chart is shown, its newest bars are refetched this often.
const HISTORY_REFRESH: Duration = Duration::from_secs(15);
/// Chart history requests run on this many worker threads.
pub const HISTORY_WORKERS: usize = 4;
/// Preloading may use all workers but one, which stays free for the charted ticker.
const PRELOAD_IN_FLIGHT: usize = HISTORY_WORKERS - 1;
/// Timeframes preloaded for every ticker on the filtered list, in this order (D is one request).
/// (1m is the stream's, plus `queue_backfills` for the minutes before and the gap since the last
/// run; 4h levels have their own queue.)
const PRELOAD_TFS: [Tf; 4] = [Tf::D1, Tf::M5, Tf::H4, Tf::H1];
/// A preload or full fetch that came back empty is tried again after this long.
const HISTORY_RETRY: Duration = Duration::from_secs(60);
/// Older bars are fetched when the chart is scrolled back to within this many bars of the oldest held.
const OLDER_MARGIN: usize = 4;

/// A 4h levels batch that came back empty is asked again after this long (a ticker with no
/// 4h bars would otherwise cost a batch of requests every minute).
const LEVELS_RETRY: Duration = Duration::from_secs(600);

/// Float cache file in the data directory (`scraper::load_floats`).
pub const FLOATS_FILE: &str = "floats.csv";

/// A mouse drag on the chart: moving the plot (from where it was pressed) or scaling the price axis.
#[derive(Debug, Clone, Copy)]
enum Drag {
    Plot { x: u16, y: u16, pan: i64, shift: f64 },
    Axis { y: u16, zoom: f64 },
}

/// Screen areas the mouse can act on, recorded by the last draw.
#[derive(Default)]
pub struct Hits {
    /// Gainers data rows and the index of the first row shown in them.
    pub gainer_rows: Rect,
    pub gainer_offset: usize,
    /// Gainers header cells and their position in `visible_cols`.
    pub gainer_header: Vec<(Rect, usize)>,
    /// Alerts data rows and the symbol of each, top to bottom.
    pub alert_rows: Rect,
    pub alert_syms: Vec<String>,
    /// Header layout tabs.
    pub layout_tabs: Vec<(Rect, Layout)>,
    pub tf_tabs: Vec<(Rect, Tf)>,
    /// Footer key hints.
    pub keys: Vec<(Rect, KeyCode)>,
    /// Chart plot (drag to scroll back) and price axis (drag to scale), and how many bars fit.
    pub chart_plot: Rect,
    pub chart_axis: Rect,
    pub chart_fit: usize,
    /// Width of the chart area in cells (to recompute the fit when the zoom changes).
    pub chart_cols: u16,
    /// Config sidebar rows and their index in `SETUP_ITEMS`.
    pub setup_items: Vec<(Rect, usize)>,
}

pub enum Msg {
    Fetched(Result<Fetched, String>),
    /// Chart bars for a symbol and timeframe, oldest first, answering a request of `mode`;
    /// `done` = last message of the request (earlier ones carry the bars so far).
    Bars { symbol: String, tf: Tf, mode: history::Mode, done: bool, failed: bool, bars: Vec<Candle> },
    /// Backfilled minute bars of a symbol, oldest first, and whether a request failed (the job
    /// is asked again later).
    Backfill { symbol: String, bars: Vec<Candle>, failed: bool },
    /// 4h bars ending in `(from, to]` for a symbol's swing levels, oldest first; None when a
    /// request failed or a 403 pause cut the batch short.
    Levels { symbol: String, from: i64, to: i64, bars: Option<Vec<Candle>> },
    /// Float and total shares of the last batch of ticker ids sent to the float worker.
    Floats(Result<Vec<(u64, f64, f64)>, String>),
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
    /// Alerts skipped at the top of the alerts panel (scrolled in the Alerts layout).
    pub alert_scroll: usize,

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
    /// Config -> Sound: 0 = the Mute row, 1 = the Volume row.
    pub sound_cursor: usize,
    /// Config -> Time & Format: the formats being edited (shown everywhere while the item is
    /// selected) and the focused row of `fmt::ROWS`.
    pub fmt_draft: Formats,
    pub fmt_cursor: usize,
    /// Formats of a preset loaded into Configuration, saved with it (Enter) or dropped (Esc).
    pub preset_formats: Option<Formats>,
    /// Config -> Presets: the names (built-in first), the selected one (`presets.len()` = the
    /// "Save current config as" row), its Configuration inputs (None when it cannot be read), the
    /// name being typed, a question and a message.
    pub presets: Vec<String>,
    pub preset_cursor: usize,
    pub preset_inputs: Option<Vec<String>>,
    /// Its Time & Format settings in one line.
    pub preset_summary: Option<String>,
    pub preset_input: Option<String>,
    pub preset_ask: Option<PresetAsk>,
    pub preset_msg: Option<String>,
    /// Config -> Theme: the highlighted entry of `theme::names()` (previewed in `palette`).
    pub theme_cursor: usize,
    pub col_cursor: usize,
    pub chart_symbol: String,
    pub sel_symbol: Option<String>,
    /// Candle storage; None until `attach_candle_store` (tests never write to disk).
    candle_store: Option<CandleStore>,
    /// Backfill worker; None until `attach_backfill` (tests make no network calls).
    backfill_tx: Option<Sender<backfill::Job>>,
    /// Symbols already sent for backfill this run (each is backfilled once).
    backfill_asked: HashSet<String>,
    /// Backfills whose request failed, and when: asked again after `HISTORY_RETRY`.
    backfill_failed: HashMap<String, Instant>,
    /// The charted symbol and since when it has been charted.
    charted_since: Option<(String, Instant)>,
    pub timeframe: Tf,
    /// Chart history worker; None until `attach_history`.
    history_tx: Option<Sender<history::Req>>,
    /// Fetched bars per symbol and timeframe (not 1m: those are the engine's candles).
    pub bars: HashMap<(String, Tf), Vec<Candle>>,
    bars_pending: HashSet<(String, Tf)>,
    bars_asked_at: HashMap<(String, Tf), Instant>,
    /// Charts with no older bars left to fetch.
    bars_exhausted: HashSet<(String, Tf)>,
    /// Charts brought up to date this run (a cached chart is topped up once before anything else).
    bars_topped: HashSet<(String, Tf)>,
    /// 4h swing levels worker; None until `attach_levels`.
    levels_tx: Option<Sender<levels::Req>>,
    /// A year of 4h bars per symbol and the swing lines drawn from them.
    pub levels: HashMap<String, levels::Data>,
    /// Their disk cache; None in tests.
    levels_dir: Option<PathBuf>,
    /// The symbol whose 4h bars are being fetched.
    pub levels_pending: Option<String>,
    /// When a symbol's levels may be asked again: `LEVELS_RETRY` after an empty answer,
    /// `HISTORY_RETRY` after a failed batch.
    levels_retry_at: HashMap<String, Instant>,
    /// New York date the level lines were last drawn for; a new day draws them again.
    levels_day: Option<chrono::NaiveDate>,
    /// Float worker; None until `attach_floats`.
    floats_tx: Option<Sender<Vec<u64>>>,
    /// Float and total shares per ticker id, fetched once per New York day; its file (None in tests).
    floats: HashMap<u64, Float>,
    floats_path: Option<PathBuf>,
    /// The ids being fetched, and when a failed batch may be asked again.
    floats_pending: Vec<u64>,
    floats_retry_at: Option<Instant>,
    /// Ticker ids of every symbol listed this run, so a ticker that left the list still charts.
    ids: HashMap<String, u64>,
    /// Start of the newest saved 1m candle of each symbol read from storage (gap backfill).
    saved_newest: HashMap<String, i64>,
    /// The symbol whose 1m gap since the last run is being backfilled (one at a time).
    gap_pending: Option<String>,
    /// Symbols whose gap was filled (or had none) this run; kept apart from `backfill_asked`, so a
    /// symbol backfilled before its saved candles loaded still gets its gap.
    gap_done: HashSet<String>,
    /// 5m/1h/4h/D bar cache (`barcache`); None in tests. Charts whose cache was read this run.
    bars_dir: Option<PathBuf>,
    bars_cached: HashSet<(String, Tf)>,
    pub hits: Hits,
    /// Chart view: bars the right edge sits back from the newest (negative: empty space after
    /// it), and price range scale (1 = fit).
    pub pan: i64,
    /// Time zoom: index into `chart::ZOOMS` (mouse wheel).
    pub scale: usize,
    pub zoom: f64,
    /// Price range moved up by this fraction of its height (vertical plot drag; 0 = fitted).
    pub shift: f64,
    drag: Option<Drag>,
    last_axis_click: Option<Instant>,
    /// No config existed at launch: the user sets the filters before anything is fetched.
    pub first_run: bool,
    /// Colours of the configured theme (or the one being previewed in Settings).
    pub palette: Palette,
    /// App home (config, data); None in tests.
    pub home: Option<PathBuf>,
    /// Config -> Storage browser, while open.
    pub storage: Option<storage::Browser>,
    /// Bytes of ticker data on disk (the `data` folder), refreshed every `STORAGE_POLL`.
    pub storage_bytes: Option<u64>,
    storage_at: Option<Instant>,
    /// System-wide CPU busy % (Health Check), sampled on the tick at most every `CPU_POLL`.
    pub cpu_pct: Option<f64>,
    cpu: crate::sysmem::CpuMeter,
    cpu_at: Option<Instant>,
    /// OS name and build, read once.
    pub os_version: Option<String>,
    /// Per-ticker candle files, and the tickers whose file was read this run.
    candle_dir: Option<PathBuf>,
    candles_loaded: HashSet<String>,
    /// The window app paints candles itself at pixel resolution (`chart::Plot`).
    pub pixel_chart: bool,
    /// What the window app paints over the last drawn chart.
    pub plot: Option<crate::chart::Plot>,
    /// Halftone art the window app paints as dots under the cells (boot scene, cog).
    pub halftones: Vec<(Rect, crate::art::Halftone)>,
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
        let palette = theme::palette(&cfg.theme);
        let fmt_draft = cfg.formats.clone();
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
            alert_scroll: 0,
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
            sound_cursor: 0,
            fmt_draft,
            fmt_cursor: 0,
            preset_formats: None,
            presets: Vec::new(),
            preset_cursor: 0,
            preset_inputs: None,
            preset_summary: None,
            preset_input: None,
            preset_ask: None,
            preset_msg: None,
            theme_cursor: 0,
            col_cursor: 0,
            chart_symbol: String::new(),
            sel_symbol: None,
            candle_store: None,
            backfill_tx: None,
            backfill_asked: HashSet::new(),
            backfill_failed: HashMap::new(),
            charted_since: None,
            timeframe: Tf::M1,
            history_tx: None,
            bars: HashMap::new(),
            bars_pending: HashSet::new(),
            bars_asked_at: HashMap::new(),
            bars_exhausted: HashSet::new(),
            bars_topped: HashSet::new(),
            levels_tx: None,
            levels: HashMap::new(),
            levels_dir: None,
            levels_pending: None,
            levels_retry_at: HashMap::new(),
            levels_day: None,
            floats_tx: None,
            floats: HashMap::new(),
            floats_path: None,
            floats_pending: Vec::new(),
            floats_retry_at: None,
            ids: HashMap::new(),
            saved_newest: HashMap::new(),
            gap_pending: None,
            gap_done: HashSet::new(),
            bars_dir: None,
            bars_cached: HashSet::new(),
            hits: Hits::default(),
            pan: 0,
            scale: crate::chart::DEFAULT_SCALE,
            zoom: 1.0,
            shift: 0.0,
            drag: None,
            last_axis_click: None,
            first_run: false,
            palette,
            home: None,
            storage: None,
            storage_bytes: None,
            storage_at: None,
            cpu_pct: None,
            cpu: crate::sysmem::CpuMeter::default(),
            cpu_at: None,
            os_version: crate::sysmem::os_version(),
            candle_dir: None,
            candles_loaded: HashSet::new(),
            pixel_chart: false,
            plot: None,
            halftones: Vec::new(),
        }
    }

    pub fn set_timeframe(&mut self, tf: Tf) {
        if tf != self.timeframe {
            self.timeframe = tf;
            self.reset_chart_view();
        }
    }

    fn over_chart(&self, p: Position) -> bool {
        self.hits.chart_plot.contains(p) || self.hits.chart_axis.contains(p)
    }

    /// Time zoom level; the scroll stays within what is held.
    fn set_scale(&mut self, scale: usize) {
        self.scale = scale.min(crate::chart::ZOOMS.len() - 1);
        // bars that fit at the new zoom, before the scroll is clamped to them
        if self.hits.chart_cols > 0 {
            self.hits.chart_fit = crate::chart::fit(self.hits.chart_cols, self.scale);
        }
        let (lo, hi) = self.pan_range();
        self.pan = self.pan.clamp(lo, hi);
    }

    /// How far the chart moves: forward until half the plot is empty space after the newest
    /// bar, back until a quarter of the plot still shows bars (older ones are fetched near the
    /// oldest, which moves this further back).
    fn pan_range(&self) -> (i64, i64) {
        let (len, _, _) = self.charted_symbol().map(|s| self.held_bars(s)).unwrap_or_default();
        let fit = self.hits.chart_fit as i64;
        // whole merged candles, so the grouping does not shift at the limits
        let merge = crate::chart::ZOOMS[self.scale].1 as i64;
        (-(fit / 2 / merge * merge), ((len as i64 - fit / 4).max(0)) / merge * merge)
    }

    fn reset_chart_view(&mut self) {
        self.pan = 0;
        self.zoom = 1.0;
        self.shift = 0.0;
        self.drag = None;
    }

    /// Bars held for `sym` in the current timeframe.
    fn held_bars(&self, sym: &str) -> (usize, Option<i64>, Option<i64>) {
        match self.timeframe {
            Tf::M1 => {
                let c = self.engine.candles(sym);
                (c.map_or(0, |c| c.len()), c.and_then(|c| c.front()).map(|c| c.start), c.and_then(|c| c.back()).map(|c| c.start))
            }
            tf => {
                let b = self.bars.get(&(sym.to_string(), tf));
                (b.map_or(0, |b| b.len()), b.and_then(|b| b.first()).map(|c| c.start), b.and_then(|b| b.last()).map(|c| c.start))
            }
        }
    }

    pub fn attach_history(&mut self, tx: Sender<history::Req>) {
        self.history_tx = Some(tx);
    }

    /// Whether bars for `sym` in the current timeframe are being fetched.
    pub fn bars_loading(&self, sym: &str) -> bool {
        self.bars_pending.contains(&(sym.to_string(), self.timeframe))
    }

    /// Ticker id of `sym`: from the list, else from any list fetched this run.
    fn ticker_id(&self, sym: &str) -> Option<u64> {
        self.all.iter().find(|t| t.symbol == sym).map(|t| t.ticker_id).filter(|&i| i != 0).or_else(|| self.ids.get(sym).copied())
    }

    /// The 5m/1h/4h/D bar cache folder.
    pub fn attach_bar_cache(&mut self, dir: &Path) {
        self.bars_dir = Some(dir.to_path_buf());
    }

    /// Reads the cached bars of `key` the first time they are needed this run.
    fn load_cached_bars(&mut self, key: &(String, Tf)) {
        if key.1 == Tf::M1 || !self.bars_cached.insert(key.clone()) {
            return;
        }
        let Some(dir) = &self.bars_dir else { return };
        let bars = barcache::load(dir, key.1, &key.0);
        if !bars.is_empty() && !self.bars.contains_key(key) {
            self.bars.insert(key.clone(), bars);
        }
    }

    fn send_history(&mut self, tx: &Sender<history::Req>, symbol: String, ticker_id: u64, tf: Tf, mode: history::Mode, now: Instant) {
        let key = (symbol, tf);
        if tx.send(history::Req { symbol: key.0.clone(), ticker_id, tf, mode }).is_ok() {
            self.bars_asked_at.insert(key.clone(), now);
            self.bars_pending.insert(key);
        }
    }

    /// The request the charted ticker needs now, if any: its chart in the current timeframe
    /// (once selected for `BACKFILL_DWELL`), older bars when scrolled back near the oldest one,
    /// or its newest bars every `HISTORY_REFRESH`.
    fn charted_request(&self, now: Instant) -> Option<(String, u64, Tf, history::Mode)> {
        let (sym, since) = self.charted_since.as_ref()?;
        if now.saturating_duration_since(*since) < BACKFILL_DWELL {
            return None;
        }
        let tf = self.timeframe;
        let key = (sym.clone(), tf);
        if self.bars_pending.contains(&key) {
            return None;
        }
        let id = self.ticker_id(sym)?;
        let asked_ago = self.bars_asked_at.get(&key).map(|&t| now.saturating_duration_since(t));
        let (len, first, last) = self.held_bars(sym);
        let cap = if tf == Tf::M1 { MAX_CANDLES } else { history::MAX_HELD };
        let near_oldest = self.pan + (self.hits.chart_fit + OLDER_MARGIN) as i64 >= len as i64;
        let mode = match (first, last) {
            // a cached chart not refreshed yet this run: its newest bars first
            (Some(_), Some(last)) if tf != Tf::M1 && !self.bars_topped.contains(&key) && asked_ago.is_none_or(|a| a >= HISTORY_RETRY) => {
                history::Mode::Since(last)
            }
            (Some(first), _) if near_oldest && len < cap && !self.bars_exhausted.contains(&key) => history::Mode::Before(first),
            // 1m candles are the app's own; only older ones are fetched
            _ if tf == Tf::M1 => return None,
            (None, _) if asked_ago.is_none_or(|a| a >= HISTORY_REFRESH) => history::Mode::Full,
            (Some(_), Some(last)) if asked_ago.is_none_or(|a| a >= HISTORY_REFRESH) => history::Mode::Since(last),
            _ => return None,
        };
        Some((sym.clone(), id, tf, mode))
    }

    /// Chart history: the charted ticker first, then every timeframe of every ticker on the
    /// filtered list (top to bottom) in the remaining worker slots. A chart with cached bars is
    /// topped up (`Since` its newest bar) instead of fetched in full.
    fn dispatch_history(&mut self, now: Instant) {
        let Some(tx) = self.history_tx.clone() else { return };
        if let Some((sym, _)) = self.charted_since.clone() {
            self.load_cached_bars(&(sym, self.timeframe));
        }
        if self.bars_pending.len() < HISTORY_WORKERS {
            if let Some((sym, id, tf, mode)) = self.charted_request(now) {
                self.send_history(&tx, sym, id, tf, mode, now);
            }
        }
        let mut wanted = Vec::new();
        'list: for i in 0..self.tickers.len() {
            let (sym, id) = (self.tickers[i].symbol.clone(), self.tickers[i].ticker_id);
            for tf in PRELOAD_TFS {
                if self.bars_pending.len() + wanted.len() >= PRELOAD_IN_FLIGHT {
                    break 'list;
                }
                let key = (sym.clone(), tf);
                if id == 0 || self.bars_pending.contains(&key) {
                    continue;
                }
                self.load_cached_bars(&key);
                let asked = self.bars_asked_at.get(&key);
                let mode = match (self.bars.get(&key).and_then(|b| b.last()), asked) {
                    (None, a) if a.is_none_or(|&a| now.saturating_duration_since(a) >= HISTORY_RETRY) => history::Mode::Full,
                    // cached bars not topped up yet this run (a failed top-up is asked again)
                    (Some(last), a) if !self.bars_topped.contains(&key) && a.is_none_or(|&a| now.saturating_duration_since(a) >= HISTORY_RETRY) => {
                        history::Mode::Since(last.start)
                    }
                    _ => continue,
                };
                wanted.push((key.0, id, tf, mode));
            }
        }
        for (sym, id, tf, mode) in wanted {
            self.send_history(&tx, sym, id, tf, mode, now);
        }
    }

    fn on_bars(&mut self, symbol: String, tf: Tf, mode: history::Mode, done: bool, failed: bool, bars: Vec<Candle>) {
        let key = (symbol, tf);
        if done {
            self.bars_pending.remove(&key);
            // the next refresh counts from the answer, not from the (slow) request
            self.bars_asked_at.insert(key.clone(), Instant::now());
            // only a clean empty answer means there is nothing older; a failed one is asked again
            if matches!(mode, history::Mode::Before(_)) && bars.is_empty() && !failed {
                self.bars_exhausted.insert(key.clone());
            }
            if matches!(mode, history::Mode::Since(_) | history::Mode::Full) && !failed {
                self.bars_topped.insert(key.clone());
            }
        }
        match (tf, mode) {
            (Tf::M1, history::Mode::Before(_)) => self.insert_candles(&key.0, &bars),
            (Tf::M1, _) => {}
            // an empty answer leaves nothing, so it is tried again after HISTORY_RETRY
            (_, history::Mode::Full) if !bars.is_empty() => {
                self.bars.insert(key.clone(), bars);
            }
            (_, history::Mode::Full) => {}
            // a top-up that did not reach the held bars (an old cache) replaces them, so no hole
            // is left; a failed one is dropped and the next refresh asks again
            (_, history::Mode::Since(s)) if bars.first().is_some_and(|b| b.start > s) => {
                if !failed {
                    self.bars.insert(key.clone(), bars);
                }
            }
            _ => {
                if let Some(held) = self.bars.get_mut(&key) {
                    history::merge(held, &bars);
                }
            }
        }
        if done && tf != Tf::M1 {
            if let (Some(dir), Some(held)) = (&self.bars_dir, self.bars.get(&key)) {
                let _ = barcache::save(dir, tf, &key.0, held);
            }
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.view, View::Dashboard | View::Chart) && !self.in_setup() {
            return;
        }
        let p = Position::new(m.column, m.row);
        match m.kind {
            // the wheel zooms the chart in time, like TradingView: up = closer, down = further out
            // only over the chart; elsewhere the wheel does nothing
            MouseEventKind::ScrollUp if self.over_chart(p) => self.set_scale(self.scale.saturating_sub(1)),
            MouseEventKind::ScrollDown if self.over_chart(p) => self.set_scale(self.scale + 1),
            // over the alerts: scroll them (the draw keeps the last page in view)
            MouseEventKind::ScrollUp if self.hits.alert_rows.contains(p) => self.alert_scroll = self.alert_scroll.saturating_sub(1),
            MouseEventKind::ScrollDown if self.hits.alert_rows.contains(p) => self.alert_scroll += 1,
            MouseEventKind::Down(MouseButton::Left) => self.click(p),
            MouseEventKind::Drag(MouseButton::Left) => self.drag_to(p),
            MouseEventKind::Up(MouseButton::Left) => self.drag = None,
            _ => {}
        }
    }

    /// Plot drag moves the chart with the pointer: right shows older bars (to the nearest bar),
    /// down shows higher prices. Axis drag: down widens the price range, up narrows it.
    fn drag_to(&mut self, p: Position) {
        match self.drag {
            Some(Drag::Plot { x, y, pan, shift }) => {
                let (step, merge) = crate::chart::ZOOMS[self.scale];
                let bars = (f64::from(i32::from(p.x) - i32::from(x)) * merge as f64 / f64::from(step)).round() as i64;
                let (lo, hi) = self.pan_range();
                self.pan = (pan + bars).clamp(lo, hi);
                // the price plot spans the plot area less its header, time axis and volume rows
                let span = (f64::from(self.hits.chart_plot.height) - 6.0).max(1.0);
                self.shift = shift + f64::from(i32::from(p.y) - i32::from(y)) / span;
            }
            Some(Drag::Axis { y, zoom }) => {
                let rows = i32::from(p.y) - i32::from(y);
                self.zoom = (zoom * 1.06f64.powi(rows)).clamp(0.05, 20.0);
            }
            None => {}
        }
    }

    fn click(&mut self, p: Position) {
        if let Some(&(_, l)) = self.hits.layout_tabs.iter().find(|(r, _)| r.contains(p)) {
            self.set_layout(l);
            return;
        }
        if let Some(&(_, tf)) = self.hits.tf_tabs.iter().find(|(r, _)| r.contains(p)) {
            self.set_timeframe(tf);
            return;
        }
        if self.hits.chart_plot.contains(p) {
            self.drag = Some(Drag::Plot { x: p.x, y: p.y, pan: self.pan, shift: self.shift });
            return;
        }
        if self.hits.chart_axis.contains(p) {
            // a double click on the price axis fits the range again
            let double = self.last_axis_click.is_some_and(|t| t.elapsed() < Duration::from_millis(400));
            self.last_axis_click = Some(Instant::now());
            if double {
                self.zoom = 1.0;
                self.shift = 0.0;
            }
            self.drag = Some(Drag::Axis { y: p.y, zoom: self.zoom });
            return;
        }
        if let Some(&(_, i)) = self.hits.setup_items.iter().find(|(r, _)| r.contains(p)) {
            // another item is shown with the sidebar focused; the one shown is opened
            if i == self.setup_idx {
                self.view = SETUP_VIEWS[i];
            } else {
                self.select_setup(i);
                self.view = View::SetupMenu;
            }
            return;
        }
        if let Some(&(_, code)) = self.hits.keys.iter().find(|(r, _)| r.contains(p)) {
            self.on_key(KeyEvent::new(code, KeyModifiers::NONE));
            return;
        }
        if self.view != View::Dashboard {
            return;
        }
        if let Some(&(_, vi)) = self.hits.gainer_header.iter().find(|(r, _)| r.contains(p)) {
            self.sel_col = vi;
            self.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        } else if self.hits.gainer_rows.contains(p) {
            let i = self.hits.gainer_offset + (p.y - self.hits.gainer_rows.y) as usize;
            if i < self.tickers.len() {
                self.move_row(i);
            }
        } else if self.hits.alert_rows.contains(p) {
            let i = (p.y - self.hits.alert_rows.y) as usize;
            if let Some(sym) = self.hits.alert_syms.get(i).cloned() {
                match self.tickers.iter().position(|t| t.symbol == sym) {
                    Some(row) => self.move_row(row),
                    // not on the list any more (filtered out, blacklisted): its full chart
                    None => {
                        self.chart_symbol = sym;
                        self.view = View::Chart;
                    }
                }
            }
        }
    }

    pub fn attach_backfill(&mut self, tx: Sender<backfill::Job>) {
        self.backfill_tx = Some(tx);
    }

    /// The levels worker and the cache folder (pruned here: files unused for 30 days).
    pub fn attach_levels(&mut self, tx: Sender<levels::Req>, dir: &Path) {
        let _ = levels::prune(dir);
        self.levels_tx = Some(tx);
        self.levels_dir = Some(dir.to_path_buf());
    }

    /// 4h level bars for `sym`, from memory, else the disk cache, else empty.
    fn level_data(&mut self, sym: &str, now: i64) -> &levels::Data {
        if !self.levels.contains_key(sym) {
            let d = self.levels_dir.as_deref().and_then(|dir| levels::load(dir, sym, now)).unwrap_or_default();
            self.levels.insert(sym.to_string(), d);
        }
        &self.levels[sym]
    }

    /// The next batch of 4h level bars, one batch at a time, most urgent ticker first:
    /// the charted one (after `BACKFILL_DWELL`), SETUP alerts, list tickers whose year is not
    /// fetched yet (top to bottom), then the list's newly finished bars.
    fn dispatch_levels(&mut self, now: Instant) {
        let Some(tx) = self.levels_tx.clone() else { return };
        let t = chrono::Utc::now().timestamp();
        // a new day: today's bars no longer grey out or remove lines the same way
        let day = store::ny_date(t);
        if self.levels_day != Some(day) {
            self.levels_day = Some(day);
            self.levels.values_mut().for_each(|d| d.redraw(t));
        }
        if self.levels_pending.is_some() || crate::limiter::paused().is_some() {
            return;
        }
        let mut urgent: Vec<String> = Vec::new();
        if let Some((s, since)) = &self.charted_since {
            if now.saturating_duration_since(*since) >= BACKFILL_DWELL {
                urgent.push(s.clone());
            }
        }
        urgent.extend(self.engine.alerts().iter().rev().filter(|a| a.kind == AlertKind::Setup).map(|a| a.symbol.clone()));
        let list: Vec<String> = self.tickers.iter().map(|t| t.symbol.clone()).collect();
        let passes: [(&[String], bool); 3] = [(&urgent, false), (&list, true), (&list, false)];
        for (syms, only_new) in passes {
            for sym in syms {
                if self.levels_retry_at.get(sym).is_some_and(|&at| now < at) {
                    continue;
                }
                let Some(ticker_id) = self.ticker_id(sym) else { continue };
                let d = self.level_data(sym, t);
                if only_new && d.complete(t) {
                    continue;
                }
                if let Some((from, to)) = d.next_range(t) {
                    if tx.send(levels::Req { symbol: sym.clone(), ticker_id, from, to }).is_ok() {
                        self.levels_pending = Some(sym.clone());
                    }
                    return;
                }
            }
        }
    }

    fn on_levels(&mut self, symbol: String, from: i64, to: i64, bars: Option<Vec<Candle>>) {
        self.levels_pending = None;
        let now = chrono::Utc::now().timestamp();
        let d = self.levels.entry(symbol.clone()).or_default();
        // None: a request failed (or a 403 pause started, which dispatch also waits out)
        let Some(bars) = bars else {
            self.levels_retry_at.insert(symbol, Instant::now() + HISTORY_RETRY);
            return;
        };
        if !d.apply(from, to, &bars, now) {
            self.levels_retry_at.insert(symbol, Instant::now() + LEVELS_RETRY);
            return;
        }
        self.levels_retry_at.remove(&symbol);
        if let Some(dir) = &self.levels_dir {
            let _ = levels::save(dir, &symbol, d);
        }
    }

    /// The float worker and the cache file (read here).
    pub fn attach_floats(&mut self, tx: Sender<Vec<u64>>, path: &Path) {
        self.floats = scraper::load_floats(path, store::ny_date(chrono::Utc::now().timestamp()));
        self.floats_tx = Some(tx);
        self.floats_path = Some(path.to_path_buf());
    }

    /// The next batch of gainers-list ids without a float fetched today (New York), one batch
    /// at a time; a failed batch waits `HISTORY_RETRY`.
    fn dispatch_floats(&mut self, now: Instant) {
        let Some(tx) = &self.floats_tx else { return };
        if !self.floats_pending.is_empty() || self.floats_retry_at.is_some_and(|at| now < at) || crate::limiter::paused().is_some() {
            return;
        }
        let today = store::ny_date(chrono::Utc::now().timestamp());
        let ids: Vec<u64> = self
            .all
            .iter()
            .map(|t| t.ticker_id)
            .filter(|id| *id != 0 && self.floats.get(id).is_none_or(|f| f.date != today))
            .take(scraper::FLOAT_BATCH)
            .collect();
        if !ids.is_empty() && tx.send(ids.clone()).is_ok() {
            self.floats_pending = ids;
        }
    }

    /// A float answer: stored (an id the answer left out is unknown for today), saved and shown.
    fn on_floats(&mut self, res: Result<Vec<(u64, f64, f64)>, String>) {
        let asked = std::mem::take(&mut self.floats_pending);
        let Ok(rows) = res else {
            self.floats_retry_at = Some(Instant::now() + HISTORY_RETRY);
            return;
        };
        let date = store::ny_date(chrono::Utc::now().timestamp());
        for id in asked {
            self.floats.insert(id, Float { float: 0.0, total: 0.0, date });
        }
        for (id, float, total) in rows {
            self.floats.insert(id, Float { float, total, date });
        }
        if let Some(p) = &self.floats_path {
            let _ = scraper::save_floats(p, &self.floats);
        }
        scraper::fill_floats(&mut self.all, &self.floats);
        scraper::fill_floats(&mut self.snapshot, &self.floats);
        self.engine.set_floats(&self.floats);
        self.reapply_local();
    }

    /// The symbol the chart shows: the full-screen chart's, else the selected row's.
    pub fn charted_symbol(&self) -> Option<&str> {
        match self.view {
            View::Chart => Some(self.chart_symbol.as_str()),
            _ => self.tickers.get(self.row).map(|t| t.symbol.as_str()),
        }
    }

    /// Notes when the charted symbol changed, for the fetch dwell.
    fn track_charted(&mut self, now: Instant) {
        let charted = self.charted_symbol().map(str::to_string);
        if charted != self.charted_since.as_ref().map(|(s, _)| s.clone()) {
            self.charted_since = charted.map(|s| (s, now));
            self.reset_chart_view();
        }
    }

    /// 1m backfill, each symbol once per run (a failed job is asked again after
    /// `HISTORY_RETRY`). A ticker with saved candles gets the minutes between its newest saved
    /// candle and its first live one (the time the app was closed): the charted ticker first,
    /// then the list top to bottom, one job at a time. Any other alerted, charted (after
    /// `BACKFILL_DWELL`) or listed ticker gets the minutes before the first candle we hold.
    fn queue_backfills(&mut self, now: Instant) {
        let Some(tx) = self.backfill_tx.clone() else { return };
        let charted = self.charted_since.as_ref().filter(|(_, since)| now.saturating_duration_since(*since) >= BACKFILL_DWELL).map(|(s, _)| s.clone());
        let list: Vec<String> = self.tickers.iter().map(|t| t.symbol.clone()).collect();
        let waiting = |app: &App, sym: &str| app.backfill_asked.contains(sym) || app.backfill_failed.get(sym).is_some_and(|&t| now.saturating_duration_since(t) < HISTORY_RETRY);
        if self.gap_pending.is_none() {
            for sym in charted.iter().chain(&list) {
                let Some(&saved) = self.saved_newest.get(sym) else { continue };
                let retry_wait = self.backfill_failed.get(sym).is_some_and(|&t| now.saturating_duration_since(t) < HISTORY_RETRY);
                if self.gap_done.contains(sym) || retry_wait {
                    continue;
                }
                let Some(id) = self.ticker_id(sym) else { continue };
                // the first live candle; none yet: wait for it
                let Some(first) = self.engine.candles(sym).and_then(|c| c.iter().find(|c| c.start > saved)).map(|c| c.start) else { continue };
                let Some(from) = backfill::gap_from(saved, first) else {
                    self.gap_done.insert(sym.clone());
                    continue;
                };
                if tx.send(backfill::Job { symbol: sym.clone(), ticker_id: id, from, before: first }).is_ok() {
                    self.gap_done.insert(sym.clone());
                    self.gap_pending = Some(sym.clone());
                }
                break;
            }
        }
        let mut want: Vec<String> = self.engine.alerts().iter().map(|a| a.symbol.clone()).collect();
        want.extend(charted);
        want.extend(list);
        let minute = chrono::Utc::now().timestamp().div_euclid(60) * 60;
        for sym in want {
            if self.saved_newest.contains_key(&sym) || waiting(self, &sym) {
                continue;
            }
            let Some(id) = self.ticker_id(&sym) else { continue };
            let before = self.engine.first_candle_start(&sym).unwrap_or(minute);
            if tx.send(backfill::Job::recent(sym.clone(), id, before)).is_ok() {
                self.backfill_asked.insert(sym);
            }
        }
    }

    /// Puts fetched 1m bars among the symbol's candles. Those at or before the newest stored
    /// candle are written now (`store_completed` only writes newer ones); `store::load` puts
    /// the file back in order.
    fn insert_candles(&mut self, sym: &str, bars: &[Candle]) {
        let added = self.engine.insert_candles(sym, bars);
        let Some(upto) = self.engine.stored_upto(sym) else { return };
        let old: Vec<Candle> = added.into_iter().filter(|c| c.start <= upto).collect();
        if let (Some(store), false) = (self.candle_store.as_mut(), old.is_empty()) {
            let _ = store.insert(sym, &old);
        }
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

    /// Turns on candle storage in `dir`: deletes earlier days' candle files, reloads today's
    /// candles into the engine, and from then on appends each candle once its minute ends.
    /// Storage problems never stop the screener; the missing history is simply not there.
    pub fn attach_candle_store(&mut self, dir: &Path) {
        let candles = dir.join(store::DIR);
        let _ = store::migrate_day_files(dir, &candles);
        self.candle_store = Some(CandleStore::new(&candles));
        self.candle_dir = Some(candles);
    }

    /// Reads a ticker's saved candles the first time it is on the list this run, in front of
    /// whatever was collected since, and marks them as stored.
    fn load_saved_candles(&mut self) {
        let Some(dir) = self.candle_dir.clone() else { return };
        let fresh: Vec<String> = self.all.iter().map(|t| t.symbol.clone()).filter(|s| !self.candles_loaded.contains(s)).collect();
        for sym in fresh {
            self.candles_loaded.insert(sym.clone());
            let Ok(bars) = store::load(&dir, &sym, MAX_CANDLES) else { continue };
            if let Some(last) = bars.last() {
                self.saved_newest.insert(sym.clone(), last.start);
                self.engine.insert_candles(&sym, &bars);
                let rows: Vec<(String, Candle)> = bars.into_iter().map(|c| (sym.clone(), c)).collect();
                self.engine.mark_stored(&rows);
            }
        }
    }

    /// Appends candles whose minute has ended. Failed writes are retried with the next batch.
    fn store_completed(&mut self) {
        let Some(store) = self.candle_store.as_mut() else { return };
        let rows = self.engine.completed_candles(chrono::Utc::now().timestamp());
        if !rows.is_empty() && store.append(&rows).is_ok() {
            self.engine.mark_stored(&rows);
        }
    }

    pub fn on_tick(&mut self, now: Instant) {
        self.load_saved_candles();
        self.store_completed();
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
        if self.home.is_some() && self.storage_at.is_none_or(|t| now.saturating_duration_since(t) >= STORAGE_POLL) {
            self.storage_at = Some(now);
            self.storage_bytes = self.home.as_deref().map(|h| storage::size(&h.join("data")));
        }
        if self.cpu_at.is_none_or(|t| now.saturating_duration_since(t) >= CPU_POLL) {
            self.cpu_at = Some(now);
            if let Some(p) = self.cpu.sample() {
                self.cpu_pct = Some(p);
            }
        }
        // the filter preview uses floats too, so they are fetched before the screener starts
        self.dispatch_floats(now);
        if self.running {
            self.track_charted(now);
            self.queue_backfills(now);
            self.dispatch_history(now);
            self.dispatch_levels(now);
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
        let n = self.engine.recompute(now);
        self.on_new_alerts(n, now);
        self.reapply_local();
    }

    /// After a tick raised `n` alerts (the newest `n` in the engine): flash them, and play the
    /// alert sound once if any of them passes the current alert filter.
    fn on_new_alerts(&mut self, n: usize, now: Instant) {
        if n == 0 {
            return;
        }
        self.last_alert_at = Some(now);
        let kinds: Vec<AlertKind> = self.engine.alerts().iter().rev().take(n).map(|a| a.kind).collect();
        // unit tests never play audio
        if self.sound_due(&kinds) && !cfg!(test) {
            crate::sound::play_alert(self.cfg.sound_volume, self.cfg.sound_muted);
        }
    }

    /// Whether new alerts of `kinds` sound the alert: one of them passes the alert filter and
    /// the sound is not muted or at volume 0.
    fn sound_due(&self, kinds: &[AlertKind]) -> bool {
        !self.cfg.sound_muted && self.cfg.sound_volume > 0 && kinds.iter().any(|&k| self.alert_visible(k))
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
                    Ok(mut f) => {
                        scraper::fill_floats(&mut f.tickers, &self.floats);
                        self.error = None;
                        self.last_ok_fetch = Some(Instant::now());
                        self.all = f.tickers.clone();
                        self.ids.extend(f.tickers.iter().filter(|t| t.ticker_id != 0).map(|t| (t.symbol.clone(), t.ticker_id)));
                        // the first-run filter preview waits on this list
                        if self.first_run && matches!(self.view, View::SetupMenu | View::SetupConfig | View::Presets) {
                            self.snapshot = self.all.clone();
                        }
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
                        self.on_new_alerts(out.new_alerts, now);
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
            Msg::Backfill { symbol, bars, failed } => {
                let gap = self.gap_pending.as_deref() == Some(symbol.as_str());
                if gap {
                    self.gap_pending = None;
                }
                self.insert_candles(&symbol, &bars);
                if failed {
                    if gap {
                        self.gap_done.remove(&symbol);
                    } else {
                        self.backfill_asked.remove(&symbol);
                    }
                    self.backfill_failed.insert(symbol, Instant::now());
                }
            }
            Msg::Levels { symbol, from, to, bars } => self.on_levels(symbol, from, to, bars),
            Msg::Floats(res) => self.on_floats(res),
            Msg::Bars { symbol, tf, mode, done, failed, bars } => self.on_bars(symbol, tf, mode, done, failed, bars),
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
            View::Storage => self.key_storage(k),
            View::Presets => self.key_presets(k),
            View::SetupTheme => self.key_theme(k),
            View::SetupSound => self.key_sound(k),
            View::SetupFormat => self.key_format(k),
        }
    }

    /// Whether the Config screen is shown (sidebar or a settings pane focused).
    pub fn in_setup(&self) -> bool {
        self.view == View::SetupMenu || SETUP_VIEWS.contains(&self.view)
    }

    /// The formats to draw with: the ones being edited while Time & Format is selected, a loaded
    /// preset's in Configuration, else the saved ones.
    pub fn formats(&self) -> &Formats {
        match (self.in_setup().then(|| SETUP_VIEWS[self.setup_idx]), &self.preset_formats) {
            (Some(View::SetupFormat), _) => &self.fmt_draft,
            (Some(View::SetupConfig), Some(f)) => f,
            _ => &self.cfg.formats,
        }
    }

    fn start(&mut self) {
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

    /// Window text size (GUI only), kept in the config.
    pub fn set_font_size(&mut self, size: f32) {
        self.cfg.font_size = size;
        self.save_config();
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

    fn key_storage(&mut self, k: KeyEvent) {
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

    fn key_health(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('d') => self.view = View::SetupMenu,
            KeyCode::Enter | KeyCode::Char(' ') => self.run_health_check(),
            _ => {}
        }
    }

    /// Dashboard layout, saved.
    fn set_layout(&mut self, l: Layout) {
        self.cfg.layout = l;
        self.alert_scroll = 0;
        self.save_config();
    }

    fn key_dashboard(&mut self, k: KeyEvent) {
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
            KeyCode::Char(c @ '1'..='5') => self.set_timeframe(Tf::ALL[c as usize - '1' as usize]),
            KeyCode::Char('t') => self.set_timeframe(self.timeframe.next()),
            KeyCode::Char('0') => {
                self.reset_chart_view();
                self.scale = crate::chart::DEFAULT_SCALE;
            }
            _ => {}
        }
    }

    fn open_setup(&mut self) {
        self.select_setup(0);
        self.view = View::SetupMenu;
    }

    /// Selects sidebar item `i` and readies its settings pane, which shows it live while the
    /// sidebar has focus.
    fn select_setup(&mut self, i: usize) {
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
            View::SetupFormat => {
                self.fmt_draft = self.cfg.formats.clone();
                self.fmt_cursor = 0;
            }
            View::Storage => self.storage = self.home.as_deref().map(storage::Browser::new),
            _ => {}
        }
    }

    fn key_setup_menu(&mut self, k: KeyEvent) {
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

    fn presets_dir(&self) -> Option<PathBuf> {
        self.home.as_deref().map(presets::dir)
    }

    /// Re-reads the preset list, keeping the cursor in it.
    fn refresh_presets(&mut self) {
        self.presets = match self.presets_dir() {
            Some(d) => presets::list(&d),
            None => vec![presets::BUILTIN.to_string()],
        };
        self.select_preset(self.preset_cursor);
    }

    /// Moves the Presets cursor and reads the preset there for the preview; the last row ("Save
    /// current config as") previews the config in use.
    fn select_preset(&mut self, i: usize) {
        self.preset_cursor = i.min(self.presets.len());
        let c = if self.preset_cursor == self.presets.len() { Some(self.cfg.clone()) } else { self.load_preset().ok() };
        self.preset_inputs = c.as_ref().map(inputs_from);
        self.preset_summary = c.map(|c| c.formats.summary());
    }

    fn load_preset(&self) -> Result<Config, String> {
        let name = self.presets.get(self.preset_cursor).ok_or("no preset")?;
        presets::load(&self.presets_dir().unwrap_or_default(), name, &self.cfg).map_err(|e| format!("could not read {name}: {e}"))
    }

    /// Saves the config in use as preset `name`.
    fn save_preset(&mut self, name: &str) {
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

    fn key_presets(&mut self, k: KeyEvent) {
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
                KeyCode::Char(c) if buf.chars().count() < presets::NAME_MAX && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.push(c)
                }
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

    fn key_setup_config(&mut self, k: KeyEvent) {
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
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if picked => {
                match FIELDS[self.focus].kind {
                    FieldKind::Choice(opts) => {
                        self.inputs[self.focus] = cycle_choice(&self.inputs[self.focus], opts, k.code == KeyCode::Left)
                    }
                    FieldKind::Toggle => self.inputs[self.focus] = on_off(self.inputs[self.focus] != "On").into(),
                    _ => {}
                }
            }
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

    fn commit_config(&mut self) {
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
    fn key_theme(&mut self, k: KeyEvent) {
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
            // the last row, "Rows on dashboard": left steps down, right/space/enter up, wrapping
            // like the Alert Rows choice in Configuration
            KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') if self.filter_cursor == FILTER_NAMES.len() => {
                let r = self.cfg.alert_rows;
                self.cfg.alert_rows = match k.code {
                    KeyCode::Left if r <= ALERT_ROWS_MIN => ALERT_ROWS_MAX,
                    KeyCode::Left => r - 1,
                    _ if r >= ALERT_ROWS_MAX => ALERT_ROWS_MIN,
                    _ => r + 1,
                };
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
    fn key_sound(&mut self, k: KeyEvent) {
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
    fn key_format(&mut self, k: KeyEvent) {
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
                    KeyCode::Char(c) if text.chars().count() < fmt::INPUT_MAX && !k.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                    _ => {}
                }
            }
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
        FieldId::MinFloat => format!("{:.0}", c.min_float),
        FieldId::MaxFloat => format!("{:.0}", c.max_float),
        FieldId::MinShares => format!("{:.0}", c.min_shares),
        FieldId::MaxShares => format!("{:.0}", c.max_shares),
        FieldId::TickerTape => on_off(c.ticker_tape).into(),
        FieldId::KeyHints => on_off(c.key_hints).into(),
        FieldId::AlertRows => c.alert_rows.to_string(),
    }
}

/// Text of a Toggle field.
fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
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
        FieldId::MinFloat => c.min_float = v,
        FieldId::MaxFloat => c.max_float = v,
        FieldId::MinShares => c.min_shares = v,
        FieldId::MaxShares => c.max_shares = v,
        FieldId::TickerTape => c.ticker_tape = v != 0.0,
        FieldId::KeyHints => c.key_hints = v != 0.0,
        FieldId::AlertRows => c.alert_rows = v as u16,
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
pub fn filters_from_inputs(inputs: &[String]) -> Filters {
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
        min_float: v(FieldId::MinFloat),
        max_float: v(FieldId::MaxFloat),
        min_shares: v(FieldId::MinShares),
        max_shares: v(FieldId::MaxShares),
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
        FieldKind::Toggle => match s {
            "On" => Some(1.0),
            "Off" => Some(0.0),
            _ => None,
        },
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

    /// The fetch part of `on_tick`.
    fn fetches(app: &mut App, now: Instant) {
        app.track_charted(now);
        app.queue_backfills(now);
        app.dispatch_history(now);
    }

    fn sent(rx: &std::sync::mpsc::Receiver<history::Req>) -> Vec<(String, Tf, history::Mode)> {
        rx.try_iter().map(|r| (r.symbol, r.tf, r.mode)).collect()
    }

    fn bar(start: i64, p: f64) -> Candle {
        Candle { start, open: p, high: p, low: p, close: p, volume: 1 }
    }

    fn answer(app: &mut App, sym: &str, tf: Tf, mode: history::Mode, bars: Vec<Candle>) {
        app.on_msg(Msg::Bars { symbol: sym.into(), tf, mode, done: true, failed: false, bars });
    }

    fn chart_app() -> (App, std::sync::mpsc::Receiver<history::Req>) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let (qtx, qrx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
        app.attach_history(qtx);
        app.view = View::Dashboard;
        let mk = |s: &str, id| Ticker { symbol: s.into(), ticker_id: id, price: 2.0, ..Default::default() };
        app.tickers = vec![mk("A", 1), mk("B", 2)];
        app.all = app.tickers.clone();
        (app, qrx)
    }

    #[test]
    fn listed_tickers_are_preloaded_in_list_order_leaving_a_worker_free() {
        let (mut app, qrx) = chart_app();
        let t0 = Instant::now();
        fetches(&mut app, t0);
        let first = sent(&qrx);
        let full = history::Mode::Full;
        assert_eq!(first, vec![("A".into(), Tf::D1, full), ("A".into(), Tf::M5, full), ("A".into(), Tf::H4, full)]);
        // nothing more until one answers
        fetches(&mut app, t0);
        assert!(sent(&qrx).is_empty());
        answer(&mut app, "A", Tf::D1, full, vec![bar(0, 1.0)]);
        // an empty answer is not held, so it is tried again later
        answer(&mut app, "A", Tf::M5, full, vec![]);
        fetches(&mut app, t0);
        assert_eq!(sent(&qrx), vec![("A".into(), Tf::H1, full), ("B".into(), Tf::D1, full)]);
        let later = Instant::now() + HISTORY_RETRY;
        answer(&mut app, "A", Tf::H1, full, vec![bar(0, 1.0)]);
        fetches(&mut app, later);
        assert_eq!(sent(&qrx), vec![("A".into(), Tf::M5, full)]);
    }

    #[test]
    fn charted_ticker_gets_its_chart_refreshes_and_older_bars() {
        use history::Mode::{Before, Full, Since};
        let (mut app, qrx) = chart_app();
        app.tickers.truncate(1);
        let t0 = Instant::now();
        let charted = |rx: &std::sync::mpsc::Receiver<history::Req>| {
            sent(rx).into_iter().filter(|(s, tf, _)| s == "A" && *tf == Tf::H1).map(|r| r.2).collect::<Vec<_>>()
        };
        app.on_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.timeframe, Tf::H1);
        fetches(&mut app, t0);
        // 1h is preloaded last (D, 5m and 4h fill the preload slots); the charted request
        // waits for the dwell and then takes the free worker
        assert!(charted(&qrx).is_empty());
        fetches(&mut app, t0 + BACKFILL_DWELL);
        assert_eq!(charted(&qrx), vec![Full]);
        assert!(app.bars_loading("A"));
        // a progress message shows bars but keeps the request pending
        app.on_msg(Msg::Bars { symbol: "A".into(), tf: Tf::H1, mode: Full, done: false, failed: false, bars: vec![bar(3600, 1.0)] });
        assert!(app.bars_loading("A"));
        fetches(&mut app, t0 + BACKFILL_DWELL);
        assert!(charted(&qrx).is_empty());
        answer(&mut app, "A", Tf::H1, Full, (0..48).map(|i| bar(i * 3600, 1.0)).collect());
        assert!(!app.bars_loading("A"));
        app.hits.chart_fit = 20;
        // shown and fresh: nothing; after HISTORY_REFRESH the newest bars back to the last held
        fetches(&mut app, Instant::now() + BACKFILL_DWELL);
        assert!(charted(&qrx).is_empty());
        fetches(&mut app, Instant::now() + HISTORY_REFRESH);
        assert_eq!(charted(&qrx), vec![Since(47 * 3600)]);
        answer(&mut app, "A", Tf::H1, Since(47 * 3600), vec![bar(47 * 3600, 2.0), bar(48 * 3600, 3.0)]);
        assert_eq!(app.bars[&("A".to_string(), Tf::H1)].len(), 49);
        // scrolled back near the oldest bar: older bars
        app.pan = 30;
        fetches(&mut app, Instant::now() + BACKFILL_DWELL);
        assert_eq!(charted(&qrx), vec![Before(0)]);
        answer(&mut app, "A", Tf::H1, Before(0), vec![bar(-3600, 0.5)]);
        assert_eq!(app.bars[&("A".to_string(), Tf::H1)][0].start, -3600);
        fetches(&mut app, Instant::now() + BACKFILL_DWELL);
        assert_eq!(charted(&qrx), vec![Before(-3600)]);
        // nothing older: no more asking for older bars, only refreshes
        answer(&mut app, "A", Tf::H1, Before(-3600), vec![]);
        fetches(&mut app, Instant::now() + HISTORY_REFRESH);
        assert_eq!(charted(&qrx), vec![Since(48 * 3600)]);
    }

    #[test]
    fn dragging_the_chart_pans_and_the_price_axis_scales() {
        let (mut app, _qrx) = chart_app();
        for i in 0..40 {
            app.engine.record_sample("A", i * 60, 1.0, i);
        }
        app.hits.chart_plot = Rect::new(60, 2, 40, 20);
        app.hits.chart_axis = Rect::new(100, 2, 10, 20);
        app.hits.chart_fit = 20;
        let ev = |kind, x, y| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70, 5));
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 80, 5));
        assert_eq!(app.pan, 5);
        // one column at two columns per bar rounds to a bar (no dead zone around the press)
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 71, 5));
        assert_eq!(app.pan, 1);
        // dragging left past the newest bar leaves empty space, up to half the plot; dragging
        // right stops with a quarter of the plot still showing bars
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 40, 5));
        assert_eq!(app.pan, -10);
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 200, 5));
        assert_eq!(app.pan, 35);
        // dragging down moves the price range up with the pointer (14 plot rows)
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 200, 12));
        assert_eq!(app.shift, 0.5);
        app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 200, 12));
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 10, 5));
        assert_eq!((app.pan, app.shift), (35, 0.5));
        // the wheel zooms in time and never moves the row cursor; zooming in keeps the scroll
        // inside what is held, zooming stops at both ends
        let row = app.row;
        app.on_mouse(ev(MouseEventKind::ScrollDown, 70, 5));
        assert_eq!(app.scale, crate::chart::DEFAULT_SCALE + 1);
        app.hits.chart_fit = 30;
        app.on_mouse(ev(MouseEventKind::ScrollUp, 70, 5));
        assert_eq!((app.scale, app.pan), (crate::chart::DEFAULT_SCALE, 33));
        for _ in 0..20 {
            app.on_mouse(ev(MouseEventKind::ScrollUp, 70, 5));
        }
        assert_eq!(app.scale, 0);
        for _ in 0..20 {
            app.on_mouse(ev(MouseEventKind::ScrollDown, 70, 5));
        }
        assert_eq!(app.scale, crate::chart::ZOOMS.len() - 1);
        assert_eq!(app.row, row);
        // dragging a zoomed-out chart moves more bars per column
        app.hits.chart_fit = 20;
        app.pan = 0;
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70, 5));
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 71, 5));
        assert_eq!(app.pan, crate::chart::ZOOMS[app.scale].1 as i64);
        app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 71, 5));
        app.on_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
        assert_eq!((app.pan, app.scale, app.shift), (0, crate::chart::DEFAULT_SCALE, 0.0));
        // price axis: down widens the range
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 105, 5));
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 105, 8));
        assert!(app.zoom > 1.0);
        app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 105, 8));
        // double click on the axis fits again (and drops a vertical move); `0` resets the whole view
        app.shift = 0.3;
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 105, 5));
        assert_eq!((app.zoom, app.shift), (1.0, 0.0));
        app.zoom = 3.0;
        app.on_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
        assert_eq!((app.pan, app.zoom), (0, 1.0));
        // a new timeframe starts at the newest bars, fitted
        (app.pan, app.shift) = (7, 0.2);
        app.set_timeframe(Tf::D1);
        assert_eq!((app.pan, app.shift), (0, 0.0));
    }

    #[test]
    fn a_history_shorter_than_the_plot_still_drags() {
        let (mut app, _qrx) = chart_app();
        for i in 0..10 {
            app.engine.record_sample("A", i * 60, 1.0, i);
        }
        app.hits.chart_plot = Rect::new(60, 2, 40, 20);
        app.hits.chart_fit = 20;
        let ev = |kind, x| MouseEvent { kind, column: x, row: 5, modifiers: KeyModifiers::NONE };
        app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), 70));
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 76));
        assert_eq!(app.pan, 3);
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 100));
        assert_eq!(app.pan, 5);
        app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 60));
        assert_eq!(app.pan, -5);
    }

    #[test]
    fn theme_previews_while_browsing_esc_drops_it_and_enter_saves_it() {
        let (mut app, _rx) = test_app(Config::default());
        assert_eq!(app.palette, theme::NOIR);
        let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupTheme).unwrap();
        assert_eq!(SETUP_ITEMS[item].0, "Theme");
        assert!(FIELDS.iter().all(|f| f.label != "Theme"));
        app.select_setup(item);
        app.view = View::SetupTheme;
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(app.theme_cursor, 0);
        app.on_key(key(KeyCode::Down));
        let (second, third) = (theme::names()[1].0, theme::names()[2].0);
        assert_eq!(app.palette, theme::palette(second));
        // up stops at the first
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Up));
        assert_eq!((app.theme_cursor, app.palette), (0, theme::NOIR));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Esc));
        assert!(app.view == View::SetupMenu);
        assert_eq!((app.palette, app.theme_cursor), (theme::NOIR, 0));
        assert_eq!(app.cfg.theme, theme::DEFAULT);
        // leaving the item drops a preview too
        app.view = View::SetupTheme;
        app.on_key(key(KeyCode::Down));
        app.select_setup(item + 1);
        assert_eq!(app.palette, theme::NOIR);
        // pick and save; the cursor starts on the saved theme next time
        app.select_setup(item);
        app.view = View::SetupTheme;
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.cfg.theme, second);
        assert_eq!(app.palette, theme::palette(second));
        app.select_setup(item);
        assert_eq!(app.theme_cursor, 1);
        app.view = View::SetupTheme;
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.palette, theme::palette(second));
        assert_ne!(app.palette, theme::palette(third));
    }

    #[test]
    fn wheel_zooms_only_over_the_chart_and_t_cycles_timeframes() {
        let (mut app, _qrx) = chart_app();
        app.hits.chart_plot = Rect::new(60, 2, 40, 20);
        app.hits.chart_axis = Rect::new(100, 2, 10, 20);
        app.hits.chart_cols = 50;
        let wheel = |app: &mut App, kind, x| app.on_mouse(MouseEvent { kind, column: x, row: 5, modifiers: KeyModifiers::NONE });
        let (row, scale) = (app.row, app.scale);
        wheel(&mut app, MouseEventKind::ScrollDown, 10); // over the list: nothing
        assert_eq!((app.row, app.scale), (row, scale));
        wheel(&mut app, MouseEventKind::ScrollDown, 105); // over the price axis: zoom out
        assert_eq!(app.scale, scale + 1);
        assert_eq!(app.hits.chart_fit, crate::chart::fit(50, scale + 1));
        app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.timeframe, Tf::M5);
        app.set_timeframe(Tf::D1);
        app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.timeframe, Tf::M1);
    }

    #[test]
    fn failed_requests_are_retried_not_taken_as_no_data() {
        use history::Mode::{Before, Full};
        let (mut app, qrx) = chart_app();
        app.tickers.truncate(1);
        app.set_timeframe(Tf::H1);
        let t0 = Instant::now();
        fetches(&mut app, t0);
        let _ = sent(&qrx);
        answer(&mut app, "A", Tf::H1, Full, (0..48).map(|i| bar(i * 3600, 1.0)).collect());
        app.hits.chart_fit = 20;
        app.pan = 30;
        fetches(&mut app, Instant::now() + BACKFILL_DWELL);
        assert!(sent(&qrx).iter().any(|r| r.2 == Before(0)));
        // a failed empty answer does not mark the chart as having nothing older
        app.on_msg(Msg::Bars { symbol: "A".into(), tf: Tf::H1, mode: Before(0), done: true, failed: true, bars: vec![] });
        fetches(&mut app, Instant::now() + BACKFILL_DWELL);
        assert!(sent(&qrx).iter().any(|r| r.2 == Before(0)));
    }

    #[test]
    fn a_failed_backfill_is_asked_again_after_the_retry_delay() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let (btx, brx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
        app.attach_backfill(btx);
        app.view = View::Dashboard;
        app.tickers = vec![Ticker { symbol: "A".into(), ticker_id: 1, ..Default::default() }];
        app.all = app.tickers.clone();
        let t0 = Instant::now();
        fetches(&mut app, t0);
        assert_eq!(brx.try_recv().unwrap().symbol, "A");
        app.on_msg(Msg::Backfill { symbol: "A".into(), bars: vec![], failed: true });
        fetches(&mut app, Instant::now());
        assert!(brx.try_recv().is_err());
        fetches(&mut app, Instant::now() + HISTORY_RETRY);
        assert_eq!(brx.try_recv().unwrap().symbol, "A");
    }

    #[test]
    fn clicking_an_alert_for_a_ticker_off_the_list_opens_its_chart() {
        let (mut app, _qrx) = chart_app();
        app.hits.alert_rows = Rect::new(1, 2, 40, 1);
        app.hits.alert_syms = vec!["ZZZ".into()];
        app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 3, row: 2, modifiers: KeyModifiers::NONE });
        assert!(app.view == View::Chart);
        assert_eq!(app.chart_symbol, "ZZZ");
    }

    #[test]
    fn p_loads_the_recommended_preset_into_the_inputs_and_enter_saves_it() {
        let (mut app, _rx) = test_app(Config::default());
        app.inputs = inputs_from(&app.cfg);
        app.view = View::SetupConfig;
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        // previewed only
        assert_eq!(app.cfg.min_change_percent, 0.0);
        assert_eq!(app.inputs[field_index(FieldId::MinChange)], "15.0");
        // presets apply the Configuration fields only, never the theme
        assert_eq!(app.palette, theme::NOIR);
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let want = Config::recommended(&Config::default());
        assert_eq!((app.cfg.min_change_percent, app.cfg.max_price, app.cfg.max_market_cap), (15.0, 25.0, 800_000_000.0));
        assert_eq!((app.cfg.vol_m_window_seconds, app.cfg.refresh_interval_seconds), (want.vol_m_window_seconds, 1));
        assert_eq!(app.cfg.theme, theme::DEFAULT);
        assert_eq!(app.palette, theme::NOIR);
    }

    #[test]
    fn first_run_opens_the_filters_and_starts_after_saving() {
        let (mut app, fetch_rx) = test_app(Config::default());
        app.first_run = true;
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // Start
        assert!(app.view == View::SetupConfig);
        assert_eq!(app.focus, field_index(FieldId::MinChange));
        assert!(!app.running);
        // one list fetch for the preview, and it fills the preview
        assert!(fetch_rx.try_recv().is_ok());
        let row = Ticker { symbol: "A".into(), price: 1.0, ..Default::default() };
        let fetched = Fetched { tickers: vec![row], group: String::new(), session: Session::now() };
        app.on_msg(Msg::Fetched(Ok(fetched)));
        assert_eq!(app.snapshot.len(), 1);
        // Esc backs out to the boot menu; Start comes back here
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.view == View::Boot);
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.view == View::SetupConfig);
        // saving starts the screener
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.first_run);
        assert!(app.view == View::Dashboard);
        assert!(app.running);
    }

    #[test]
    fn config_sidebar_shows_items_live_and_focuses_their_pane() {
        let (mut app, _rx) = test_app(Config::default());
        app.view = View::Dashboard;
        app.running = true;
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        app.on_key(key(KeyCode::Char('o')));
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.setup_idx, 0);
        // moving the sidebar keeps it focused; the pane follows
        app.on_key(key(KeyCode::Down));
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.setup_idx, 1);
        assert_eq!(app.presets, vec![presets::BUILTIN.to_string()]);
        for _ in 0..10 {
            app.on_key(key(KeyCode::Down));
        }
        assert_eq!(app.setup_idx, SETUP_ITEMS.len() - 1);
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::Health);
        app.on_key(key(KeyCode::Esc));
        assert!(app.view == View::SetupMenu);
        // Configuration: Esc drops unsaved inputs and goes back to the sidebar
        app.select_setup(0);
        app.on_key(key(KeyCode::Right));
        assert!(app.view == View::SetupConfig);
        app.inputs[field_index(FieldId::MinPrice)] = "7".into();
        app.on_key(key(KeyCode::Esc));
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.inputs, inputs_from(&app.cfg));
        // a click on another item selects it, on the shown one opens it
        let filters = SETUP_VIEWS.iter().position(|&v| v == View::SetupFilters).unwrap();
        app.hits.setup_items = vec![(Rect::new(0, 3, 24, 1), 0), (Rect::new(0, 5, 24, 1), filters)];
        let click = |y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 2, row: y, modifiers: KeyModifiers::NONE };
        app.on_mouse(click(5));
        assert!(app.view == View::SetupMenu);
        assert_eq!(app.setup_idx, filters);
        app.on_mouse(click(5));
        assert!(app.view == View::SetupFilters);
        app.on_key(key(KeyCode::Esc));
        app.on_key(key(KeyCode::Esc));
        assert!(app.view == View::Dashboard);
    }

    #[test]
    fn presets_save_load_overwrite_and_delete() {
        let home = std::env::temp_dir().join(format!("vs-app-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let (mut app, _rx) = test_app(Config { min_price: 2.0, ..Config::default() });
        app.home = Some(home.clone());
        app.running = true;
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let typed = |app: &mut App, s: &str| s.chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
        app.open_setup();
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::Presets);
        // the built-in preset is listed first and cannot be deleted
        app.on_key(key(KeyCode::Char('d')));
        assert!(app.preset_ask.is_none());
        // the last row ("Save current config as") previews the config in use; Enter asks a name
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.preset_cursor, 1);
        assert_eq!(app.preset_inputs, Some(inputs_from(&app.cfg)));
        app.on_key(key(KeyCode::Char('n'))); // no key of its own any more
        assert!(app.preset_input.is_none());
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.preset_input.as_deref(), Some(""));
        typed(&mut app, "Mine/1");
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.presets, vec![presets::BUILTIN.to_string(), "Mine1".into()]);
        assert_eq!(app.preset_cursor, 1);
        assert_eq!(app.preset_inputs.as_ref().unwrap()[field_index(FieldId::MinPrice)], "2.00");
        // the same name asks before replacing; any key but y keeps the old one
        app.cfg.min_price = 3.0;
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Enter));
        typed(&mut app, "Mine1");
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.preset_ask, Some(PresetAsk::Overwrite("Mine1".into())));
        app.on_key(key(KeyCode::Char('x')));
        assert_eq!(presets::load(&presets::dir(&home), "Mine1", &app.cfg).unwrap().min_price, 2.0);
        app.on_key(key(KeyCode::Enter));
        typed(&mut app, "Mine1");
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Char('y')));
        assert_eq!(presets::load(&presets::dir(&home), "Mine1", &app.cfg).unwrap().min_price, 3.0);
        // Enter loads it into the Configuration form, previewed; Enter there saves it
        app.cfg.min_price = 0.0;
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::SetupConfig);
        assert_eq!(app.setup_idx, 0);
        assert_eq!(app.inputs[field_index(FieldId::MinPrice)], "3.00");
        assert_eq!(app.cfg.min_price, 0.0);
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.cfg.min_price, 3.0);
        // delete after y (the cursor stayed on Mine1); the cursor then stays on a preset
        app.select_setup(1);
        app.view = View::Presets;
        assert_eq!(app.preset_cursor, 1);
        app.on_key(key(KeyCode::Char('d')));
        assert_eq!(app.preset_ask, Some(PresetAsk::Delete("Mine1".into())));
        app.on_key(key(KeyCode::Char('y')));
        assert_eq!(app.presets, vec![presets::BUILTIN.to_string()]);
        assert_eq!(app.preset_cursor, 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn mouse_selects_rows_tabs_and_footer_keys() {
        let (mut app, _qrx) = chart_app();
        app.hits.gainer_rows = Rect::new(1, 5, 40, 2);
        app.hits.tf_tabs = vec![(Rect::new(50, 0, 4, 1), Tf::D1)];
        app.hits.keys = vec![(Rect::new(0, 20, 5, 1), KeyCode::Char('v'))];
        app.hits.alert_rows = Rect::new(1, 2, 40, 1);
        app.hits.alert_syms = vec!["B".into()];
        let click = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        app.on_mouse(click(3, 6));
        assert_eq!(app.row, 1);
        app.on_mouse(click(3, 5));
        assert_eq!(app.row, 0);
        app.on_mouse(click(3, 2)); // alert row for B
        assert_eq!(app.row, 1);
        app.on_mouse(click(51, 0));
        assert_eq!(app.timeframe, Tf::D1);
        // clicking the selected row again keeps the dashboard
        app.on_mouse(click(3, 6));
        assert!(app.view == View::Dashboard);
        assert_eq!(app.row, 1);
        app.on_mouse(click(2, 20));
        assert!(app.view == View::Columns);
    }

    #[test]
    fn backfill_every_listed_ticker_once_per_symbol() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (htx, _hrx) = std::sync::mpsc::channel();
        let (btx, brx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), PathBuf::from("unused-test-config.json"), tx, htx, None);
        app.attach_backfill(btx);
        app.view = View::Dashboard;
        let mk = |s: &str, id| Ticker { symbol: s.into(), ticker_id: id, ..Default::default() };
        app.tickers = vec![mk("A", 1), mk("B", 2), mk("Z", 0)];
        app.all = app.tickers.clone();
        let jobs = |rx: &std::sync::mpsc::Receiver<backfill::Job>| rx.try_iter().map(|j| (j.symbol, j.ticker_id, j.before)).collect::<Vec<_>>();
        let t0 = Instant::now();
        // list order, no dwell; Z has no tickerId so it cannot be backfilled
        fetches(&mut app, t0);
        let first = jobs(&brx);
        assert_eq!(first.iter().map(|j| (j.0.as_str(), j.1, j.2 % 60)).collect::<Vec<_>>(), vec![("A", 1, 0), ("B", 2, 0)]);
        // once per symbol, also once charted
        fetches(&mut app, t0 + BACKFILL_DWELL * 3);
        assert!(jobs(&brx).is_empty());
        // the first candle we hold bounds the backfill
        app.engine.record_sample("C", 6000, 1.0, 10);
        app.tickers.push(mk("C", 3));
        app.all = app.tickers.clone();
        fetches(&mut app, t0 + BACKFILL_DWELL * 4);
        assert_eq!(jobs(&brx), vec![("C".to_string(), 3, 6000)]);
        // backfilled bars go in front of the candles we built
        let bar = Candle { start: 5940, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 5 };
        app.on_msg(Msg::Backfill { symbol: "C".into(), bars: vec![bar], failed: false });
        assert_eq!(app.engine.first_candle_start("C"), Some(5940));
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
            ticker_tape: false,
            key_hints: false,
            alert_rows: 12,
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
    fn layout_fields_switch_with_keys_and_save() {
        use crossterm::event::KeyEvent;
        let (mut app, _rx) = test_app(Config::default());
        app.view = View::SetupConfig;
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        app.focus = field_index(FieldId::TickerTape);
        assert_eq!(app.inputs[app.focus], "On");
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Right));
        assert_eq!(app.inputs[app.focus], "Off");
        app.focus = field_index(FieldId::AlertRows);
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.inputs[app.focus], "15");
        app.on_key(key(KeyCode::Enter));
        assert_eq!((app.cfg.ticker_tape, app.cfg.key_hints, app.cfg.alert_rows), (false, true, 15));
        assert_eq!(parse_field("on", FieldKind::Toggle), None);
    }

    #[test]
    fn alert_filters_pane_sets_dashboard_rows() {
        use crossterm::event::KeyEvent;
        let (mut app, _rx) = test_app(Config::default());
        app.view = View::SetupFilters;
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        for _ in 0..5 {
            app.on_key(key(KeyCode::Down));
        }
        assert_eq!(app.filter_cursor, FILTER_NAMES.len());
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.cfg.alert_rows, 15);
        app.on_key(key(KeyCode::Right));
        app.on_key(key(KeyCode::Char(' ')));
        assert_eq!(app.cfg.alert_rows, 6);
        // on a filter row left still goes back to the sidebar
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Left));
        assert!(app.view == View::SetupMenu);
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

    #[test]
    fn alert_sound_follows_the_alert_filter_mute_and_volume() {
        let (mut app, _rx) = test_app(Config::default());
        // a tick without alerts is silent; one with several sounds once
        assert!(!app.sound_due(&[]));
        assert!(app.sound_due(&[AlertKind::New, AlertKind::Setup]));
        // ALL with NEW switched off in Alert Filters
        app.cfg.filter_enabled.insert("NEW".into(), false);
        assert!(!app.sound_due(&[AlertKind::New]));
        assert!(app.sound_due(&[AlertKind::New, AlertKind::Setup]));
        // the SETUP filter (f cycle) passes only SETUP alerts
        app.alert_filter = FILTER_NAMES.iter().position(|&n| n == "SETUP").unwrap();
        assert!(!app.sound_due(&[AlertKind::New]));
        assert!(app.sound_due(&[AlertKind::Setup]));
        app.cfg.sound_muted = true;
        assert!(!app.sound_due(&[AlertKind::Setup]));
        app.cfg.sound_muted = false;
        app.cfg.sound_volume = 0;
        assert!(!app.sound_due(&[AlertKind::Setup]));
    }

    #[test]
    fn sound_pane_toggles_mute_and_steps_volume() {
        use crossterm::event::KeyEvent;
        let (mut app, _rx) = test_app(Config::default());
        let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupSound).unwrap();
        assert_eq!(SETUP_ITEMS[item].0, "Sound");
        app.view = View::SetupSound;
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        app.on_key(key(KeyCode::Enter));
        assert!(app.cfg.sound_muted);
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Right));
        assert_eq!(app.cfg.sound_volume, 75);
        for _ in 0..10 {
            app.on_key(key(KeyCode::Right));
        }
        assert_eq!(app.cfg.sound_volume, 100);
        for _ in 0..25 {
            app.on_key(key(KeyCode::Left));
        }
        assert_eq!(app.cfg.sound_volume, 0);
        // Enter on the Volume row plays a test and changes nothing
        app.on_key(key(KeyCode::Enter));
        assert!(app.cfg.sound_muted && app.view == View::SetupSound);
        // on the Mute row left goes back to the sidebar
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Left));
        assert!(app.view == View::SetupMenu);
    }

    #[test]
    fn format_pane_previews_validates_saves_and_reverts() {
        let (mut app, _rx) = test_app(Config::default());
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        let item = SETUP_VIEWS.iter().position(|&v| v == View::SetupFormat).unwrap();
        assert_eq!(SETUP_ITEMS[item].0, "Time & Format");
        app.select_setup(item);
        app.view = View::SetupFormat;
        // a typed zone: shown at once, saved only when it is a real one
        app.on_key(key(KeyCode::Delete));
        "Asia/Nowhere".chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
        assert_eq!(app.formats().timezone, "Asia/Nowhere");
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::SetupFormat && app.cfg.formats.timezone == "Local");
        for _ in 0.."Nowhere".len() {
            app.on_key(key(KeyCode::Backspace));
        }
        "Tokyo".chars().for_each(|c| app.on_key(key(KeyCode::Char(c))));
        // the clock row: space switches it, Enter saves everything
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Char(' ')));
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::SetupMenu);
        assert_eq!((app.cfg.formats.timezone.as_str(), app.cfg.formats.clock_12h), ("Asia/Tokyo", true));
        // Esc drops unsaved changes; away from the pane the saved formats are drawn
        app.view = View::SetupFormat;
        app.on_key(key(KeyCode::Right));
        assert_ne!(app.formats().clock_12h, app.cfg.formats.clock_12h);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.fmt_draft, app.cfg.formats);
        app.view = View::Dashboard;
        assert_eq!(app.formats(), &app.cfg.formats);
    }

    #[test]
    fn presets_carry_their_formats_into_configuration() {
        let home = std::env::temp_dir().join(format!("vs-app-preset-fmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let saved = Config { formats: Formats { date: "YYYY-MM-DD".into(), ..Formats::default() }, ..Config::default() };
        presets::save(&presets::dir(&home), "Iso", &saved).unwrap();
        let (mut app, _rx) = test_app(Config::default());
        app.home = Some(home.clone());
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        app.select_setup(1);
        app.view = View::Presets;
        app.on_key(key(KeyCode::Down));
        assert!(app.preset_summary.as_deref().is_some_and(|s| s.contains('-')));
        // loaded: the form previews them, Esc drops them, loading again and Enter saves them
        app.on_key(key(KeyCode::Enter));
        assert!(app.view == View::SetupConfig);
        assert_eq!(app.formats().date, "YYYY-MM-DD");
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.formats().date, "DD.MM.YY");
        // the Presets cursor stays on Iso
        app.select_setup(1);
        app.view = View::Presets;
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.cfg.formats.date, "YYYY-MM-DD");
        assert!(app.preset_formats.is_none());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
