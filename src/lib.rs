//! MarketGUI: the GUI app (`MarketGUI.exe`) and the console app
//! (`MarketGUI-cli.exe`: terminal UI and `--probe`) share everything here.

mod app;
mod art;
mod backfill;
mod barcache;
mod chart;
mod config;
mod engine;
mod fmt;
mod fsutil;
mod gui;
mod health;
mod history;
mod launch;
mod levels;
mod limiter;
mod net;
mod nytime;
mod paths;
mod presets;
mod probe;
mod scraper;
mod series;
mod session;
mod sound;
mod storage;
mod store;
mod stream;
mod sysmem;
mod theme;
mod trace;
mod ui;

pub use launch::{cli_main, gui_main};
