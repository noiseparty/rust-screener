//! Replay harness: runs a recorded market session through a real `App` on a simulated clock and
//! shows what the user would see (rows, their order, Vol/m, Status badges, alerts).
//!
//! The fixture (`replay::fixture`) holds what the outside world did: the REST server's list over
//! time, stream quotes and connection events. The harness plays the parts of `launch.rs` and the
//! worker threads: it ticks the app every `FRAME` (80 ms), answers the app's own list fetches
//! (`FETCH_LATENCY` later, with the list in force then), sends a stream status every second like
//! the stream thread, and delivers quotes as `Msg::Quotes` batches. Nothing sleeps: `app::now()`
//! reads the clock set here, and time jumps from one due item to the next.
//!
//! Wall time: the engine stamps REST rows with the real clock, and the series store clears all
//! history when a message's session or New York date is later than the last one. Quote stamps are
//! therefore placed on the real clock inside the current session (see `wall_base`), so a replay
//! behaves the same at any time of day. (A run that crosses a session boundary in its own few
//! milliseconds could still see one clear.)

use super::*;
use crate::launch::FRAME;
use std::cell::Cell;
use std::sync::mpsc::Receiver;

mod fixture;
mod scenarios;

use fixture::{Ev, Fixture};

thread_local! {
    /// The simulated "now" of the replay running on this thread (each test has its own thread).
    static CLOCK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The simulated clock, when a replay runs on this thread.
pub(in crate::app) fn sim_now() -> Option<Instant> {
    CLOCK.with(Cell::get)
}

/// Time from the app asking for the list to the answer.
const FETCH_LATENCY: Duration = Duration::from_millis(200);
/// The stream thread sends its status this often (`stream::STATUS_EVERY`).
const STATUS_EVERY: Duration = Duration::from_secs(1);

/// The Vol/m cell as drawn (`ui::tables`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VolM {
    /// `—`: the feed is stale.
    Stale,
    /// `…`: too little history.
    Pending,
    /// A rate; `partial` (drawn dim) while the window is not yet covered.
    Rate { per_min: f64, partial: bool },
}

/// One dashboard row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub symbol: String,
    pub price: f64,
    pub change_pct: f64,
    pub volume: i64,
    pub vol_m: VolM,
    pub dim: bool,
    pub status: Option<&'static str>,
}

/// One alert of the alerts panel; `at` is the time after t=0.
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub at: Duration,
    pub symbol: String,
    pub kind: &'static str,
    pub detail: String,
}

pub struct Replay {
    pub app: App,
    fetch_rx: Receiver<()>,
    t0: Instant,
    /// Simulated time after t0.
    now: Duration,
    events: Vec<(u64, Ev)>,
    next_ev: usize,
    /// What the REST server answers now (None before the fixture's first list).
    server: Option<Result<Vec<Ticker>, String>>,
    fetch_due: Option<Duration>,
    stream: StreamStatus,
    /// The stream thread is running (it sends a status every second from its first connect).
    stream_running: bool,
    next_status: Duration,
    next_tick: Duration,
    /// Real wall-clock ms that fixture time 0 maps to, for quote trade stamps.
    wall_base: i64,
}

impl Replay {
    /// Loads `tests/fixtures/<name>` and starts the app on it, as if the user had pressed Enter
    /// on the boot screen at t=0 (screener running, dashboard shown).
    pub fn load(name: &str) -> Replay {
        Replay::new(fixture::load(name))
    }

    pub fn new(f: Fixture) -> Replay {
        let t0 = Instant::now();
        CLOCK.with(|c| c.set(Some(t0)));
        let (fetch_tx, fetch_rx) = std::sync::mpsc::channel();
        let (health_tx, _health_rx) = std::sync::mpsc::channel();
        // A stream sender makes the app stream-enabled; the subscription commands are not needed.
        let (stream_tx, _) = std::sync::mpsc::channel();
        let path = PathBuf::from("target/test-config/unused-replay-config.json");
        let mut app = App::new(f.cfg, path, fetch_tx, health_tx, Some(stream_tx));
        app.view = View::Dashboard;
        app.running = true;
        let span = f.events.last().map_or(0, |(t, _)| *t as i64);
        Replay {
            app,
            fetch_rx,
            t0,
            now: Duration::ZERO,
            events: f.events,
            next_ev: 0,
            server: None,
            fetch_due: None,
            stream: StreamStatus::default(),
            stream_running: false,
            next_status: Duration::ZERO,
            next_tick: Duration::ZERO,
            wall_base: wall_base(chrono::Utc::now().timestamp_millis(), span + 60_000),
        }
    }

    /// Runs everything due up to and including `t` (after t=0), in time order: fixture events,
    /// then a fetch answer, then a stream status, then the app tick.
    pub fn run_to(&mut self, t: Duration) {
        loop {
            let ev = self.events.get(self.next_ev).map(|(ms, _)| Duration::from_millis(*ms));
            let status = self.stream_running.then_some(self.next_status);
            let next = [ev, self.fetch_due, status, Some(self.next_tick)].into_iter().flatten().min().expect("tick");
            if next > t {
                break;
            }
            self.now = next;
            CLOCK.with(|c| c.set(Some(self.t0 + next)));
            if ev == Some(next) {
                self.play_events();
            }
            if self.fetch_due == Some(next) {
                self.answer_fetch();
            }
            if status == Some(next) {
                self.next_status = next + STATUS_EVERY;
                self.app.on_msg(Msg::Stream(self.stream.clone()));
            }
            if self.next_tick == next {
                self.next_tick = next + FRAME;
                self.app.on_tick(self.t0 + next);
                if self.fetch_rx.try_recv().is_ok() && self.fetch_due.is_none() {
                    self.fetch_due = Some(next + FETCH_LATENCY);
                }
            }
        }
        self.now = t;
        CLOCK.with(|c| c.set(Some(self.t0 + t)));
    }

    /// `run_to` in seconds.
    pub fn run_to_secs(&mut self, secs: f64) {
        self.run_to(Duration::from_secs_f64(secs));
    }

    /// Plays the fixture events due now. Quotes due together go as one batch; REST rows due
    /// together make up the server's new list.
    fn play_events(&mut self) {
        let now_ms = self.now.as_millis() as u64;
        let mut batch = Vec::new();
        let mut list: Option<Vec<Ticker>> = None;
        while let Some((t, ev)) = self.events.get(self.next_ev).cloned() {
            if t != now_ms {
                break;
            }
            self.next_ev += 1;
            match ev {
                Ev::Rest(r) => list.get_or_insert_with(Vec::new).push(Ticker {
                    ticker_id: r.id,
                    symbol: r.symbol,
                    price: r.price,
                    volume: r.volume,
                    change_ratio: r.change_ratio,
                    market_value: r.mcap,
                    // as `scraper` derives it
                    prev_close: if r.price > 0.0 && r.change_ratio > -1.0 { r.price / (1.0 + r.change_ratio) } else { 0.0 },
                    ..Default::default()
                }),
                Ev::RestError(e) => self.server = Some(Err(e)),
                Ev::Quote(q) => batch.push(stream::Update {
                    id: q.id,
                    trade_ms: self.wall_base + q.trade_ms,
                    seq: q.seq,
                    close: q.close,
                    p_price: q.p_price,
                    change_ratio: q.change_ratio,
                    volume: q.volume,
                    mcap: q.mcap,
                    high: None,
                    low: None,
                    status: String::new(),
                    recv: self.t0 + self.now,
                }),
                Ev::StreamUp => {
                    self.stream.connected = true;
                    self.stream.connections += 1;
                    self.stream.last_msg = None;
                    self.stream.error = None;
                    self.stream_running = true;
                    self.next_status = self.now + STATUS_EVERY;
                    self.app.on_msg(Msg::Stream(self.stream.clone()));
                }
                Ev::StreamDown => {
                    self.stream.connected = false;
                    self.stream.error = Some("no data from stream".into());
                    self.app.on_msg(Msg::Stream(self.stream.clone()));
                }
            }
        }
        if let Some(list) = list {
            self.server = Some(Ok(list));
        }
        if !batch.is_empty() {
            self.stream.last_msg = Some(self.t0 + self.now);
            self.app.on_msg(Msg::Quotes(batch));
        }
    }

    /// Answers the pending fetch with the server's list; before the first list the request waits.
    fn answer_fetch(&mut self) {
        let Some(server) = self.server.clone() else {
            self.fetch_due = self.events[self.next_ev..]
                .iter()
                .find(|(_, e)| matches!(e, Ev::Rest(_) | Ev::RestError(_)))
                .map(|(t, _)| Duration::from_millis(*t));
            return;
        };
        self.fetch_due = None;
        let session = self.app.session;
        let res = server.map(|tickers| Fetched { tickers, group: "replay".into(), session });
        self.app.on_msg(Msg::Fetched(res));
    }

    /// The dashboard rows, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        self.app
            .tickers
            .iter()
            .map(|t| Row {
                symbol: t.symbol.clone(),
                price: t.price,
                change_pct: t.change_ratio * 100.0,
                volume: t.volume,
                vol_m: match t.vol_rate {
                    _ if t.vol_stale => VolM::Stale,
                    None => VolM::Pending,
                    Some(per_min) => VolM::Rate { per_min, partial: t.vol_partial },
                },
                dim: t.dim,
                status: t.status.map(AlertKind::label),
            })
            .collect()
    }

    pub fn row(&self, sym: &str) -> Option<Row> {
        self.rows().into_iter().find(|r| r.symbol == sym)
    }

    /// The dashboard symbols, top to bottom.
    pub fn symbols(&self) -> Vec<String> {
        self.app.tickers.iter().map(|t| t.symbol.clone()).collect()
    }

    /// Every alert raised so far, oldest first.
    pub fn alerts(&self) -> Vec<Fired> {
        self.app
            .engine
            .alerts()
            .iter()
            .map(|a| Fired { at: a.at - self.t0, symbol: a.symbol.clone(), kind: a.kind.label(), detail: a.detail() })
            .collect()
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        CLOCK.with(|c| c.set(None));
    }
}

/// Real wall-clock ms for fixture time 0: `span_ms` of quotes placed so they start and end in the
/// session (and New York date) of `now_ms`, the one REST rows are stamped in; ending at
/// `now_ms` when that fits, else starting there.
fn wall_base(now_ms: i64, span_ms: i64) -> i64 {
    let epoch = |ms: i64| {
        let t = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms).expect("valid time");
        (crate::nytime::date(ms / 1000), Session::at(t))
    };
    if epoch(now_ms - span_ms) == epoch(now_ms) {
        now_ms - span_ms
    } else {
        now_ms
    }
}

#[test]
fn wall_base_keeps_the_fixture_inside_one_session() {
    use chrono::TimeZone;
    let ny = |h, m| chrono_tz::America::New_York.with_ymd_and_hms(2026, 1, 12, h, m, 0).unwrap().timestamp_millis();
    let ten_min = 600_000;
    // mid-session: the fixture ends now
    assert_eq!(wall_base(ny(11, 0), ten_min), ny(10, 50));
    // just after the open: it starts now
    assert_eq!(wall_base(ny(9, 32), ten_min), ny(9, 32));
}
