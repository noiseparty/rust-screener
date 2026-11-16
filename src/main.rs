mod app;
mod art;
mod chart;
mod config;
mod engine;
mod health;
mod scraper;
mod session;
mod ui;

use app::{App, Msg};
use config::{Config, CONFIG_PATH};
use crossterm::event::{self, Event, KeyEventKind};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const PAGE_SIZE: usize = 100;

fn main() {
    let cfg = match Config::load(Path::new(CONFIG_PATH)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading {CONFIG_PATH}: {e}");
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

    let (health_tx, health_rx) = mpsc::channel::<()>();
    let proxy = cfg.finviz_proxy_url.clone();
    thread::spawn(move || {
        for () in health_rx {
            if msg_tx.send(Msg::Health(health::run_all(&proxy))).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(cfg, fetch_tx, health_tx);
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
