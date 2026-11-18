//! App tests by topic; the shared fixtures are here.

use super::fields::{parse_field, parse_smart, RECOMPUTE_CHOICES};
use super::*;

mod chart;
mod feeds;
mod fields;
mod keys;
pub(super) mod replay;
mod setup;

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

fn set(inputs: &mut [String], id: FieldId, v: &str) {
    inputs[field_index(id)] = v.to_string();
}

fn test_app(cfg: Config) -> (App, std::sync::mpsc::Receiver<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (htx, _hrx) = std::sync::mpsc::channel();
    (App::new(cfg, PathBuf::from("target/test-config/unused-app-config.json"), tx, htx, None), rx)
}
