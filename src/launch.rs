//! Startup shared by the GUI and the console app: app home, config, worker threads.

use crate::app::{self, App, Msg};
use crate::config::Config;
use crate::{backfill, barcache, gui, health, history, levels, paths, probe, scraper, stream, ui};
use crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// One app tick (input, messages, recompute) every this long.
pub const FRAME: Duration = Duration::from_millis(80);
const PAGE_SIZE: usize = 100;

const USAGE: &str = "usage: MarketGUI-cli [--probe [--probe-dir <dir>]]";

#[derive(Debug, PartialEq)]
enum Cli {
    Tui,
    Probe { dir: Option<PathBuf> },
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut probe = false;
    let mut dir = None;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--probe" => probe = true,
            "--probe-dir" => dir = Some(PathBuf::from(args.next().ok_or("--probe-dir needs a folder")?)),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    match (probe, dir) {
        (true, dir) => Ok(Cli::Probe { dir }),
        (false, None) => Ok(Cli::Tui),
        (false, Some(_)) => Err("--probe-dir only works with --probe".into()),
    }
}

/// Prepares the app home, loads the config and starts every worker thread. Returns the app
/// and the channel the workers report on.
fn start() -> Result<(App, mpsc::Receiver<Msg>), String> {
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
    // No config yet: a new user sets the display filters before the screener starts fetching.
    let first_run = !paths.config.exists();
    let cfg = Config::load(&paths.config).map_err(|e| format!("Error loading {}: {e}", paths.config.display()))?;

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    let (fetch_tx, fetch_rx) = mpsc::channel::<()>();
    let tx = msg_tx.clone();
    thread::spawn(move || {
        let agent = scraper::agent();
        for () in fetch_rx {
            if tx.send(Msg::Fetched(scraper::fetch_top_gainers(&agent, PAGE_SIZE))).is_err() {
                break;
            }
        }
    });

    let stream_tx = cfg.stream_enabled.then(|| {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        stream::spawn(msg_tx.clone(), cmd_rx);
        cmd_tx
    });

    let (backfill_tx, backfill_rx) = mpsc::channel::<backfill::Job>();
    let tx = msg_tx.clone();
    thread::spawn(move || {
        let agent = scraper::agent();
        for job in backfill_rx {
            let (bars, failed) = backfill::fetch(&agent, &job);
            if tx.send(Msg::Backfill { symbol: job.symbol, bars, failed }).is_err() {
                break;
            }
        }
    });

    // History workers share one queue; the app keeps at most `app::HISTORY_WORKERS` requests in flight.
    let (history_tx, history_rx) = mpsc::channel::<history::Req>();
    let history_rx = std::sync::Arc::new(std::sync::Mutex::new(history_rx));
    for _ in 0..app::HISTORY_WORKERS {
        let (tx, rx) = (msg_tx.clone(), history_rx.clone());
        thread::spawn(move || {
            let agent = scraper::agent();
            loop {
                let Ok(req) = rx.lock().map_err(|_| ()).and_then(|r| r.recv().map_err(|_| ())) else { break };
                let part = |bars| {
                    let _ = tx.send(Msg::Bars { symbol: req.symbol.clone(), tf: req.tf, mode: req.mode, done: false, failed: false, bars });
                };
                let (bars, failed) = history::fetch(&agent, &req, chrono::Utc::now().timestamp(), part);
                if tx.send(Msg::Bars { symbol: req.symbol, tf: req.tf, mode: req.mode, done: true, failed, bars }).is_err() {
                    break;
                }
            }
        });
    }

    // 4h swing levels: one batch at a time, each fetched on a few threads.
    let (levels_tx, levels_rx) = mpsc::channel::<levels::Req>();
    let tx = msg_tx.clone();
    thread::spawn(move || {
        for req in levels_rx {
            let bars = levels::fetch(&req);
            if tx.send(Msg::Levels { symbol: req.symbol, from: req.from, to: req.to, bars }).is_err() {
                break;
            }
        }
    });

    // Float and total shares: one batch of ticker ids at a time.
    let (floats_tx, floats_rx) = mpsc::channel::<Vec<u64>>();
    let tx = msg_tx.clone();
    thread::spawn(move || {
        let agent = scraper::agent();
        for ids in floats_rx {
            if tx.send(Msg::Floats(scraper::fetch_floats(&agent, &ids))).is_err() {
                break;
            }
        }
    });

    let (health_tx, health_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        for () in health_rx {
            if msg_tx.send(Msg::Health(health::run_all())).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(cfg, paths.config.clone(), fetch_tx, health_tx, stream_tx);
    app.attach_candle_store(&paths.data);
    app.home = Some(paths.home.clone());
    // the boot screen shows the results under the menu
    app.run_health_check();
    app.attach_backfill(backfill_tx);
    app.attach_history(history_tx);
    app.attach_levels(levels_tx, &paths.data.join("levels"));
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
    gui::run(start());
}

/// `MarketGUI-cli.exe`: the terminal UI, or `--probe`.
pub fn cli_main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Tui) => {}
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
    }
    let (mut app, msg_rx) = match start() {
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
        assert_eq!(args(&[]), Ok(Cli::Tui));
        assert_eq!(args(&["--probe"]), Ok(Cli::Probe { dir: None }));
        assert_eq!(args(&["--probe", "--probe-dir", "x"]), Ok(Cli::Probe { dir: Some(PathBuf::from("x")) }));
        assert!(args(&["--probe-dir", "x"]).is_err());
        assert!(args(&["--probe", "--probe-dir"]).is_err());
        assert!(args(&["--bogus"]).is_err());
    }
}
