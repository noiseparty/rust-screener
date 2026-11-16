mod app;
mod art;
mod chart;
mod config;
mod engine;
mod health;
mod paths;
mod probe;
mod scraper;
mod series;
mod session;
mod store;
mod stream;
mod ui;

use app::{App, Msg};
use config::Config;
use crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const PAGE_SIZE: usize = 100;

const USAGE: &str = "usage: volume_screener [--probe [--probe-dir <dir>]]";

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

fn main() {
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
            eprintln!("{e}
{USAGE}");
            std::process::exit(2);
        }
    }
    let paths = match paths::Paths::from_env().and_then(|p| {
        let cwd = std::env::current_dir()?;
        p.init(&cwd)?;
        Ok(p)
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error preparing app home: {e}");
            std::process::exit(1);
        }
    };
    let cfg = match Config::load(&paths.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading {}: {e}", paths.config.display());
            std::process::exit(1);
        }
    };

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

    let (health_tx, health_rx) = mpsc::channel::<()>();
    let proxy = cfg.finviz_proxy_url.clone();
    thread::spawn(move || {
        for () in health_rx {
            if msg_tx.send(Msg::Health(health::run_all(&proxy))).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(cfg, paths.config.clone(), fetch_tx, health_tx, stream_tx);
    app.attach_candle_store(&paths.data);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &msg_rx);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App, rx: &mpsc::Receiver<Msg>) -> std::io::Result<()> {
    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        let deadline = Instant::now() + FRAME;
        while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            if !event::poll(wait)? {
                break;
            }
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    app.on_key(k);
                    break;
                }
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
