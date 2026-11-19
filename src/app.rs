use crate::backfill;
use crate::barcache;
use crate::config::{Config, Layout, FILTER_NAMES};
use crate::engine::MAX_CANDLES;
use crate::engine::{AlertKind, Candle, Engine, Filters};
use crate::fmt::{self, Formats};
use crate::health::CheckResult;
use crate::history::{self, Tf};
use crate::layouts::{CustomLayout, Panel};
use crate::levels;
use crate::presets;
use crate::scraper::{self, Fetched, Float, Ticker};
use crate::session::Session;
use crate::storage;
use crate::store::{self, CandleStore};
use crate::stream::{self, StreamStatus};
use crate::theme::{self, Palette};
use crate::trace::{tlog, tnote};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

mod chart_view;
mod feeds;
mod fields;
mod keys;
mod layout_edit;
mod setup;
mod views;

use chart_view::Drag;
pub use chart_view::Hits;
pub use layout_edit::{layout_rows, LayoutRow};
use fields::{config_from_inputs, cycle_choice, inputs_from, on_off};
pub use fields::{field_index, filters_from_inputs, FieldId, FieldKind, COLUMNS, FIELDS};
pub use views::{DebugView, PresetAsk, View, APP_NAME, BL_COLS, BOOT_ITEMS, SETUP_ITEMS, SETUP_VIEWS, VOLUME_STEP};

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

/// The current time. The replay tests (`tests::replay`) run the app on a simulated clock.
fn now() -> Instant {
    #[cfg(test)]
    {
        if let Some(t) = tests::replay::sim_now() {
            return t;
        }
    }
    Instant::now()
}

pub enum Msg {
    Fetched(Result<Fetched, String>),
    /// Chart bars for a symbol and timeframe, oldest first, answering a request of `mode`;
    /// `done` = last message of the request (earlier ones carry the bars so far).
    Bars {
        symbol: String,
        tf: Tf,
        mode: history::Mode,
        done: bool,
        failed: bool,
        bars: Vec<Candle>,
    },
    /// Backfilled minute bars of a symbol, oldest first, and whether a request failed (the job
    /// is asked again later).
    Backfill {
        symbol: String,
        bars: Vec<Candle>,
        failed: bool,
    },
    /// 4h bars ending in `(from, to]` for a symbol's swing levels, oldest first; None when a
    /// request failed or a 403 pause cut the batch short.
    Levels {
        symbol: String,
        from: i64,
        to: i64,
        bars: Option<Vec<Candle>>,
    },
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
    /// Config -> Layouts: the custom tabs being edited, which one, and the focused row.
    pub layouts_draft: Vec<CustomLayout>,
    pub layouts_tab: usize,
    pub layouts_cursor: usize,
    /// On-screen layout editor: the drafts to restore on Esc, the view to go back to, and the
    /// pointer while a widget is carried.
    layout_edit_undo: Option<Vec<CustomLayout>>,
    layout_edit_back: View,
    pub drag_pos: Option<Position>,
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
    /// Bubbles the window app paints over the cells (the Bubbles layout).
    pub bubbles: Vec<crate::bubbles::Painted>,
    /// Where the bubbles are, kept between frames, and when they last moved.
    pub bubble_field: crate::bubbles::Field,
    pub bubbles_at: Option<Instant>,
    /// Trace mode's debug overlay on the dashboard.
    pub debug: DebugView,
}

impl App {
    pub fn new(
        cfg: Config,
        config_path: PathBuf,
        fetch_tx: Sender<()>,
        health_tx: Sender<()>,
        stream_tx: Option<Sender<stream::Cmd>>,
    ) -> App {
        let now = now();
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
            layouts_draft: vec![CustomLayout::default(); crate::layouts::MAX],
            layouts_tab: 0,
            layouts_cursor: 0,
            layout_edit_undo: None,
            layout_edit_back: View::Dashboard,
            drag_pos: None,
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
            bubbles: Vec::new(),
            bubble_field: Default::default(),
            bubbles_at: None,
            debug: DebugView { on: crate::trace::enabled(), ..DebugView::default() },
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
        let now = now();
        let fresh = |t: Instant| now.saturating_duration_since(t) <= STREAM_STALE;
        self.stream_enabled() && self.stream.connected && self.stream_status_at.is_some_and(fresh) && self.stream.last_msg.is_none_or(fresh)
    }

    pub fn stream_status(&self) -> &StreamStatus {
        &self.stream
    }

    /// Seconds between REST list fetches: slow while the stream is healthy, the fallback poll otherwise.
    pub fn poll_interval(&self) -> u64 {
        if self.stream_ok() {
            LIST_REFRESH
        } else {
            self.cfg.refresh_interval_seconds.max(1)
        }
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
            self.storage_bytes = self.home.as_deref().map(|h| storage::size(&crate::paths::Paths::at(h.to_path_buf()).data));
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
        tnote!("engine.recompute", "{} ms, {} rows, {n} alerts", now.elapsed().as_millis(), self.tickers.len());
    }

    /// After a tick raised `n` alerts (the newest `n` in the engine): flash them, and play the
    /// alert sound once if any of them passes the current alert filter.
    fn on_new_alerts(&mut self, n: usize, now: Instant) {
        if n == 0 {
            return;
        }
        self.last_alert_at = Some(now);
        tlog!(
            "engine.alerts",
            "{n}: {}",
            self.engine.alerts().iter().rev().take(n).map(|a| format!("{} {}", a.symbol, a.kind.label())).collect::<Vec<_>>().join(", ")
        );
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
        crate::trace::dispatch(&msg);
        match msg {
            Msg::Fetched(res) => {
                self.fetching = false;
                self.last_fetch = Some(now());
                match res {
                    Err(e) => self.error = Some(e),
                    Ok(mut f) => {
                        scraper::fill_floats(&mut f.tickers, &self.floats);
                        self.error = None;
                        self.last_ok_fetch = Some(now());
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
                        let now = now();
                        let out = self.engine.process(f.tickers, self.sort_key, self.sort_desc, now);
                        tlog!(
                            "engine.process",
                            "{} ms, {} rows, {} blacklisted, {} alerts",
                            now.elapsed().as_millis(),
                            out.active.len(),
                            out.blacklisted.len(),
                            out.new_alerts
                        );
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
                if self.engine.apply_quotes(&updates, Session::now(), now()) {
                    self.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
            }
            Msg::Stream(st) => {
                if st.connections != self.stream.connections {
                    self.engine.on_stream_reconnect();
                }
                self.stream = st;
                self.stream_status_at = Some(now());
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

    /// Re-filter and re-sort the last fetched data without a network fetch.
    fn reapply_local(&mut self) {
        let out = self.engine.reapply(self.sort_key, self.sort_desc, now());
        self.tickers = out.active;
        self.blacklisted = out.blacklisted;
        self.resync_row();
    }
}

#[cfg(test)]
mod tests;
