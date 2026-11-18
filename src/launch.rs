//! Startup shared by the GUI and the console app: app home, config, worker threads.

use crate::app::{self, App, Msg};
use crate::config::Config;
use crate::trace::{self, tlog};
use crate::{backfill, barcache, gui, health, history, levels, paths, probe, scraper, stream, ui};
use crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// One app tick (input, messages, recompute) every this long.
pub const FRAME: Duration = Duration::from_millis(80);
const PAGE_SIZE: usize = 100;

const USAGE: &str = "usage: MarketGUI-cli [--trace] | --probe [--probe-dir <dir>]";

#[derive(Debug, PartialEq)]
enum Cli {
    /// The app; `trace` = `--trace` was given.
    Tui {
        trace: bool,
    },
    Probe {
        dir: Option<PathBuf>,
    },
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut probe = false;
    let mut trace = false;
    let mut dir = None;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--probe" => probe = true,
            "--probe-dir" => dir = Some(PathBuf::from(args.next().ok_or("--probe-dir needs a folder")?)),
            "--trace" => trace = true,
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    match (probe, dir) {
        (true, _) if trace => Err("--trace does not work with --probe".into()),
        (true, dir) => Ok(Cli::Probe { dir }),
        (false, None) => Ok(Cli::Tui { trace }),
        (false, Some(_)) => Err("--probe-dir only works with --probe".into()),
    }
}

/// The window app has no console to report a bad argument on: it only looks for `--trace`.
fn gui_trace_arg(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|a| a == "--trace")
}

/// Runs `f` and returns its result with the milliseconds it took (for trace lines).
fn timed<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_millis())
}

/// The app home, ready to use, and the error of a failed `VolumeScreener` rename (shown in
/// the app).
fn prepare_home() -> Result<(paths::Paths, Option<String>), String> {
    let mut migrate_err = None;
    let paths = paths::Paths::from_env()
        .and_then(|p| {
            // the old `VolumeScreener` folder becomes the app home; if that fails it is left as it is
            if let Err(e) = p.migrate_home() {
                migrate_err = Some(format!("could not rename %LOCALAPPDATA%\\{} to {}: {e}", paths::OLD_APP_DIR_NAME, paths::APP_DIR_NAME));
            }
            // a failed rename: this run keeps using the old folder, so no new home blocks the next try
            let p = match (&migrate_err, p.old_home()) {
                (Some(_), Some(old)) if old.is_dir() => paths::Paths::at(old),
                _ => p,
            };
            let cwd = std::env::current_dir()?;
            p.init(&cwd)?;
            Ok(p)
        })
        .map_err(|e| format!("Error preparing app home: {e}"))?;
    Ok((paths, migrate_err))
}

/// A worker thread that answers each job from `rx` with a message, until either channel
/// closes. `agent` gives every worker its own HTTP agent.
fn spawn_worker<J: Send + 'static>(tx: mpsc::Sender<Msg>, rx: mpsc::Receiver<J>, run: impl Fn(&ureq::Agent, J) -> Msg + Send + 'static) {
    thread::spawn(move || {
        let agent = crate::net::agent();
        for job in rx {
            if tx.send(run(&agent, job)).is_err() {
                break;
            }
        }
    });
}

/// The chart history workers: they share one queue and hand over partial charts while
/// walking back. The app keeps at most `app::HISTORY_WORKERS` requests in flight.
fn spawn_history_workers(msg_tx: &mpsc::Sender<Msg>) -> mpsc::Sender<history::Req> {
    let (history_tx, history_rx) = mpsc::channel::<history::Req>();
    let history_rx = std::sync::Arc::new(std::sync::Mutex::new(history_rx));
    for _ in 0..app::HISTORY_WORKERS {
        let (tx, rx) = (msg_tx.clone(), history_rx.clone());
        thread::spawn(move || {
            let agent = crate::net::agent();
            loop {
                let Ok(req) = rx.lock().map_err(|_| ()).and_then(|r| r.recv().map_err(|_| ())) else { break };
                let part = |bars| {
                    let _ = tx.send(Msg::Bars { symbol: req.symbol.clone(), tf: req.tf, mode: req.mode, done: false, failed: false, bars });
                };
                let ((bars, failed), ms) = timed(|| history::fetch(&agent, &req, chrono::Utc::now().timestamp(), part));
                tlog!(
                    "rest.history",
                    "{} {} {:?}: {} bars in {ms} ms{}",
                    req.symbol,
                    req.tf.label(),
                    req.mode,
                    bars.len(),
                    if failed { " FAILED" } else { "" }
                );
                if tx.send(Msg::Bars { symbol: req.symbol, tf: req.tf, mode: req.mode, done: true, failed, bars }).is_err() {
                    break;
                }
            }
        });
    }
    history_tx
}

/// Prepares the app home, loads the config and starts every worker thread. Returns the app
/// and the channel the workers report on.
fn start(trace_arg: bool) -> Result<(App, mpsc::Receiver<Msg>), String> {
    let (paths, migrate_err) = prepare_home()?;
    trace::init(&paths.logs, trace::wanted(trace_arg));
    // No config yet: a new user sets the display filters before the screener starts fetching.
    let first_run = !paths.config.exists();
    let cfg = Config::load(&paths.config).map_err(|e| format!("Error loading {}: {e}", paths.config.display()))?;

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    let (fetch_tx, fetch_rx) = mpsc::channel::<()>();
    spawn_worker(msg_tx.clone(), fetch_rx, |agent, ()| {
        let (res, ms) = timed(|| scraper::fetch_top_gainers(agent, PAGE_SIZE));
        match &res {
            Ok(f) => tlog!("rest.gainers", "{} rows in {ms} ms", f.tickers.len()),
            Err(e) => tlog!("rest.gainers", "error after {ms} ms: {e}"),
        }
        Msg::Fetched(res)
    });

    let stream_tx = cfg.stream_enabled.then(|| {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        stream::spawn(msg_tx.clone(), cmd_rx);
        cmd_tx
    });

    let (backfill_tx, backfill_rx) = mpsc::channel::<backfill::Job>();
    spawn_worker(msg_tx.clone(), backfill_rx, |agent, job: backfill::Job| {
        let ((bars, failed), ms) = timed(|| backfill::fetch(agent, &job));
        tlog!(
            "rest.backfill",
            "{} {}..{}: {} bars in {ms} ms{}",
            job.symbol,
            job.from,
            job.before,
            bars.len(),
            if failed { " FAILED" } else { "" }
        );
        Msg::Backfill { symbol: job.symbol, bars, failed }
    });

    let history_tx = spawn_history_workers(&msg_tx);

    // 4h swing levels: one batch at a time, each fetched on a few threads (with their own agents).
    let (levels_tx, levels_rx) = mpsc::channel::<levels::Req>();
    spawn_worker(msg_tx.clone(), levels_rx, |_, req: levels::Req| {
        let (bars, ms) = timed(|| levels::fetch(&req));
        tlog!(
            "rest.levels",
            "{} {}..{}: {} in {ms} ms",
            req.symbol,
            req.from,
            req.to,
            bars.as_ref().map_or("FAILED".to_string(), |b| format!("{} bars", b.len()))
        );
        Msg::Levels { symbol: req.symbol, from: req.from, to: req.to, bars }
    });

    // Float and total shares: one batch of ticker ids at a time.
    let (floats_tx, floats_rx) = mpsc::channel::<Vec<u64>>();
    spawn_worker(msg_tx.clone(), floats_rx, |agent, ids: Vec<u64>| {
        let (res, ms) = timed(|| scraper::fetch_floats(agent, &ids));
        tlog!(
            "rest.floats",
            "{} ids: {} in {ms} ms",
            ids.len(),
            res.as_ref().map_or_else(|e| format!("error {e}"), |v| format!("{} floats", v.len()))
        );
        Msg::Floats(res)
    });

    let (health_tx, health_rx) = mpsc::channel::<()>();
    spawn_worker(msg_tx, health_rx, |_, ()| {
        let (res, ms) = timed(health::run_all);
        tlog!("health", "{}/{} checks ok in {ms} ms", res.iter().filter(|c| c.ok).count(), res.len());
        Msg::Health(res)
    });

    let mut app = App::new(cfg, paths.config.clone(), fetch_tx, health_tx, stream_tx);
    app.attach_candle_store(&paths.data);
    app.home = Some(paths.home.clone());
    // the boot screen shows the results under the menu
    app.run_health_check();
    app.attach_backfill(backfill_tx);
    app.attach_history(history_tx);
    app.attach_levels(levels_tx, &paths.data.join(crate::levels::DIR));
    app.attach_bar_cache(&paths.data.join(barcache::DIR));
    app.attach_floats(floats_tx, &paths.data.join(app::FLOATS_FILE));
    app.first_run = first_run;
    if let Some(e) = migrate_err {
        eprintln!("{e}");
        app.error = Some(e);
    }
    Ok((app, msg_rx))
}

/// `MarketGUI.exe`: the app in its own window.
pub fn gui_main() {
    gui::run(start(gui_trace_arg(std::env::args().skip(1))));
}

/// `MarketGUI-cli.exe`: the terminal UI, or `--probe`.
pub fn cli_main() {
    let trace = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Tui { trace }) => trace,
        Ok(Cli::Probe { dir }) => {
            if let Err(e) = probe::run(dir) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            return;
        }
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let (mut app, msg_rx) = match start(trace) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    let result = run_tui(&mut terminal, &mut app, &msg_rx);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run_tui(terminal: &mut ratatui::DefaultTerminal, app: &mut App, rx: &mpsc::Receiver<Msg>) -> std::io::Result<()> {
    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        let deadline = Instant::now() + FRAME;
        while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            if !event::poll(wait)? {
                break;
            }
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    app.on_key(k);
                    break;
                }
                // Pointer motion is ignored; it would only force redraws.
                Event::Mouse(m) if m.kind != event::MouseEventKind::Moved => {
                    app.on_mouse(m);
                    break;
                }
                _ => {}
            }
        }
        while let Ok(msg) = rx.try_recv() {
            app.on_msg(msg);
        }
        app.on_tick(Instant::now());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<Cli, String> {
        parse_args(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_arguments() {
        assert_eq!(args(&[]), Ok(Cli::Tui { trace: false }));
        assert_eq!(args(&["--trace"]), Ok(Cli::Tui { trace: true }));
        assert!(args(&["--probe", "--trace"]).is_err());
        assert!(args(&["--trace", "--probe-dir", "x"]).is_err());
        assert_eq!(args(&["--probe"]), Ok(Cli::Probe { dir: None }));
        assert_eq!(args(&["--probe", "--probe-dir", "x"]), Ok(Cli::Probe { dir: Some(PathBuf::from("x")) }));
        assert!(args(&["--probe-dir", "x"]).is_err());
        assert!(args(&["--probe", "--probe-dir"]).is_err());
        assert!(args(&["--bogus"]).is_err());
    }

    #[test]
    fn window_app_looks_only_for_trace() {
        let a = |a: &[&str]| gui_trace_arg(a.iter().map(|s| s.to_string()));
        assert!(a(&["--trace"]) && a(&["x", "--trace"]));
        assert!(!a(&[]) && !a(&["--probe"]) && !a(&["--tracex"]));
    }
}
