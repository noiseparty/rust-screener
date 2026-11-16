mod app;
mod art;
mod chart;
mod config;
mod engine;
mod health;
mod paths;
mod scraper;
mod series;
mod session;
mod stream;
mod ui;

use app::{App, Msg};
use config::Config;
use crossterm::event::{self, Event, KeyEventKind};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const PAGE_SIZE: usize = 100;

fn main() {
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
