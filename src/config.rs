use crate::layouts::{self, CustomLayout, Panel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Alert kinds that can be raised and filtered. SPIKE and PUMP are hidden for now (their enum
/// variants stay in the engine so they can return).
pub const FILTER_NAMES: [&str; 3] = ["ALL", "NEW", "SETUP"];

/// Old config keys and the SETUP keys that replaced them (value copied as is).
const SETUP_KEY_MIGRATION: [(&str, &str); 2] = [("spike_vol_ratio", "setup_vol_ratio"), ("baseline_window_sec", "setup_window_sec")];

/// Dashboard layout, picked with the header tabs or `w`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Alerts and the list, the chart beside them when the window is wide enough.
    #[default]
    Full,
    /// Alerts and the list, full width.
    List,
    /// The chart only.
    Chart,
    /// The alerts only, full height and scrollable.
    Alerts,
    /// The listed tickers as bubbles sized by their volume over the timeframe.
    Bubbles,
    /// The user's own tabs (`Config::custom_layouts`, Config -> Layouts).
    Custom1,
    Custom2,
    Custom3,
}

impl Layout {
    /// The built-in tabs.
    pub const ALL: [Layout; 5] = [Layout::Full, Layout::List, Layout::Chart, Layout::Bubbles, Layout::Alerts];
    pub const CUSTOM: [Layout; layouts::MAX] = [Layout::Custom1, Layout::Custom2, Layout::Custom3];

    /// Which custom tab this is (0-based).
    pub fn custom_index(self) -> Option<usize> {
        Layout::CUSTOM.iter().position(|&l| l == self)
    }
}

/// Rows of the dashboard alerts panel: least, most.
pub const ALERT_ROWS_MIN: u16 = 5;
pub const ALERT_ROWS_MAX: u16 = 15;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub refresh_interval_seconds: u64,
    /// Use the Webull push stream (falls back to REST polling when it is down).
    pub stream_enabled: bool,
    pub alert_history_max: usize,
    pub min_market_cap: f64,
    pub max_market_cap: f64,
    pub rank_tracking_interval_seconds: u64,
    /// Re-rank / re-sort tick in milliseconds: 500 or 1000.
    pub recompute_ms: u64,

    // SETUP: volume ratio and price change versus `setup_window_sec` ago
    /// Window of the SETUP look-back, in seconds. Old files call it `baseline_window_sec`.
    pub setup_window_sec: u64,
    /// Volume of the last `setup_window_sec` divided by the volume of the window before it.
    /// Old files: `spike_vol_ratio`.
    pub setup_vol_ratio: f64,
    /// Price change in percent versus `setup_window_sec` ago (10 = +10%). Old files: `pump_price_pct`
    /// (a fraction, 0.05 = 5%).
    pub setup_price_pct: f64,
    pub min_volume_for_alert: i64,
    pub min_price_for_alert: f64,
    /// Seconds before the same alert kind can be raised again for one symbol.
    pub alert_cooldown_sec: u64,

    // Display filters (0 = disabled)
    pub min_change_percent: f64,
    pub max_change_percent: f64,
    pub min_volume: i64,
    pub max_volume: i64,
    pub min_price: f64,
    pub max_price: f64,
    /// Float and total shares bounds; a ticker whose count is unknown passes.
    pub min_float: f64,
    pub max_float: f64,
    pub min_shares: f64,
    pub max_shares: f64,

    /// Window of the Vol/m column. Old files call it `delta_window_seconds`.
    #[serde(alias = "delta_window_seconds")]
    pub vol_m_window_seconds: u64,
    /// Colour theme id: "dark-noir" (the original look) or an opaline builtin such as
    /// "catppuccin-mocha". An unknown id shows Dark Noir.
    pub theme: String,
    /// Text size of the app window (GUI), in points. Ctrl+mouse wheel changes it.
    pub font_size: f32,
    pub layout: Layout,
    /// The user's own tabs (`Layout::Custom1..3`), at most `layouts::MAX`.
    pub custom_layouts: Vec<CustomLayout>,
    /// The scrolling ticker tape and the key hints at the bottom of the dashboard.
    pub ticker_tape: bool,
    pub key_hints: bool,
    /// Rows of the dashboard alerts panel (5-15).
    pub alert_rows: u16,
    /// Alert sound: silent when muted; volume in percent (0-100).
    pub sound_muted: bool,
    pub sound_volume: u8,
    /// Time zone, date, clock and number formats (Config -> Time & Format).
    pub formats: crate::fmt::Formats,
    pub blacklisted_tickers: Vec<String>,
    pub filter_enabled: BTreeMap<String, bool>,
    pub column_visibility: BTreeMap<String, bool>,

    pub finviz_enabled: bool,
    pub finviz_proxy_url: String,
    pub finviz_proxy_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            refresh_interval_seconds: 5,
            stream_enabled: true,
            alert_history_max: 50,
            min_market_cap: 0.0,
            max_market_cap: 1e15,
            rank_tracking_interval_seconds: 60,
            recompute_ms: 1000,
            setup_window_sec: 300,
            setup_vol_ratio: 2.2,
            setup_price_pct: 10.0,
            min_volume_for_alert: 10_000,
            min_price_for_alert: 1.0,
            alert_cooldown_sec: 120,
            min_change_percent: 0.0,
            max_change_percent: 0.0,
            min_volume: 0,
            max_volume: 0,
            min_price: 0.0,
            max_price: 0.0,
            min_float: 0.0,
            max_float: 0.0,
            min_shares: 0.0,
            max_shares: 0.0,
            vol_m_window_seconds: 180,
            theme: "dark-noir".to_string(),
            font_size: 15.0,
            layout: Layout::Full,
            custom_layouts: Vec::new(),
            ticker_tape: true,
            key_hints: true,
            alert_rows: ALERT_ROWS_MIN,
            sound_muted: false,
            sound_volume: 70,
            formats: crate::fmt::Formats::default(),
            blacklisted_tickers: Vec::new(),
            filter_enabled: FILTER_NAMES.iter().map(|f| (f.to_string(), true)).collect(),
            column_visibility: BTreeMap::new(),
            finviz_enabled: false,
            finviz_proxy_url: String::new(),
            finviz_proxy_key: String::new(),
        }
    }
}

impl Config {
    /// The "Recommended" preset (`p` in Configuration): the owner's tuned settings for
    /// small-cap momentum. Only the Configuration fields change; everything else is kept.
    pub fn recommended(base: &Config) -> Config {
        Config {
            refresh_interval_seconds: 1,
            min_market_cap: 0.0,
            max_market_cap: 800_000_000.0,
            rank_tracking_interval_seconds: 60,
            recompute_ms: 1000,
            vol_m_window_seconds: 120,
            setup_window_sec: 300,
            setup_vol_ratio: 2.2,
            setup_price_pct: 10.0,
            min_volume_for_alert: 10_000,
            min_price_for_alert: 1.0,
            alert_cooldown_sec: 120,
            min_change_percent: 15.0,
            max_change_percent: 0.0,
            min_volume: 10_000,
            max_volume: 0,
            min_price: 0.3,
            max_price: 25.0,
            min_float: 0.0,
            max_float: 0.0,
            min_shares: 0.0,
            max_shares: 0.0,
            theme: "everforest-dark".to_string(),
            ..base.clone()
        }
    }

    /// The config in `path`, or the defaults when there is none yet. A missing file is not
    /// written here: the first save (the first-run Configuration) creates it, so a user who
    /// quits before saving still gets the first-run setup next time.
    pub fn load(path: &Path) -> io::Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let data = fs::read_to_string(path)?;
        let mut cfg = Config::from_json(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        cfg.validate();
        Ok(cfg)
    }

    /// Parses a config file, first moving the pre-SETUP keys to their new names. A new key
    /// already in the file wins over the old one.
    fn from_json(text: &str) -> Result<Config, serde_json::Error> {
        let mut v: serde_json::Value = serde_json::from_str(text)?;
        if let Some(obj) = v.as_object_mut() {
            for (old, new) in SETUP_KEY_MIGRATION {
                if let Some(val) = obj.remove(old) {
                    obj.entry(new).or_insert(val);
                }
            }
            // The old pump threshold was a fraction (0.05); SETUP Price % is entered as a percent.
            if let Some(val) = obj.remove("pump_price_pct") {
                let pct = match val.as_f64() {
                    Some(f) if f <= 1.0 => serde_json::json!((f * 100.0 * 1e6).round() / 1e6),
                    _ => val,
                };
                obj.entry("setup_price_pct").or_insert(pct);
            }
        }
        serde_json::from_value(v)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let data = serde_json::to_string_pretty(self).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        crate::fsutil::write_atomic(path, data)
    }

    /// Clamps out-of-range values; unusable ones fall back to the defaults.
    pub fn validate(&mut self) {
        let d = Config::default();
        if self.refresh_interval_seconds < 1 {
            self.refresh_interval_seconds = 1;
        }
        if self.alert_history_max < 10 {
            self.alert_history_max = 10;
        }
        if self.rank_tracking_interval_seconds == 0 {
            self.rank_tracking_interval_seconds = d.rank_tracking_interval_seconds;
        } else if self.rank_tracking_interval_seconds < 5 {
            self.rank_tracking_interval_seconds = 5;
        }
        if self.recompute_ms != 500 {
            self.recompute_ms = d.recompute_ms;
        }
        if !(self.setup_vol_ratio > 0.0 && self.setup_vol_ratio.is_finite()) {
            self.setup_vol_ratio = d.setup_vol_ratio;
        }
        if !(self.setup_price_pct > 0.0 && self.setup_price_pct.is_finite()) {
            self.setup_price_pct = d.setup_price_pct;
        }
        if self.setup_window_sec == 0 {
            self.setup_window_sec = d.setup_window_sec;
        }
        if self.alert_cooldown_sec == 0 {
            self.alert_cooldown_sec = d.alert_cooldown_sec;
        }
        if self.vol_m_window_seconds == 0 {
            self.vol_m_window_seconds = d.vol_m_window_seconds;
        } else if self.vol_m_window_seconds < 30 {
            self.vol_m_window_seconds = 30;
        }
        self.alert_rows = self.alert_rows.clamp(ALERT_ROWS_MIN, ALERT_ROWS_MAX);
        self.sound_volume = self.sound_volume.min(100);
        self.formats.validate();
        // The "delta" column became "volrate" (Vol/m).
        if let Some(v) = self.column_visibility.remove("delta") {
            self.column_visibility.entry("volrate".to_string()).or_insert(v);
        }
        for f in FILTER_NAMES {
            self.filter_enabled.entry(f.to_string()).or_insert(true);
        }
        self.custom_layouts.truncate(layouts::MAX);
        self.custom_layouts.iter_mut().for_each(CustomLayout::normalize);
        if self.layout.custom_index().is_some() && self.custom(self.layout).is_none() {
            self.layout = Layout::Full;
        }
    }

    /// The custom tab `l`, when it is set up and switched on.
    pub fn custom(&self, l: Layout) -> Option<&CustomLayout> {
        self.custom_layouts.get(l.custom_index()?).filter(|c| c.is_used())
    }

    /// The header tabs: the built-in ones, then the custom ones in use.
    pub fn tabs(&self) -> Vec<Layout> {
        Layout::ALL.into_iter().chain(Layout::CUSTOM.into_iter().filter(|&l| self.custom(l).is_some())).collect()
    }

    /// The tab after the current one.
    pub fn next_layout(&self) -> Layout {
        let tabs = self.tabs();
        tabs[(tabs.iter().position(|&l| l == self.layout).map_or(0, |i| i + 1)) % tabs.len()]
    }

    /// A tab's name: the built-in name, or the custom one's (else "Custom N").
    pub fn layout_label(&self, l: Layout) -> String {
        match l {
            Layout::Full => "Full".into(),
            Layout::List => "List".into(),
            Layout::Chart => "Chart".into(),
            Layout::Alerts => "Alerts".into(),
            Layout::Bubbles => "Bubbles".into(),
            _ => {
                let i = l.custom_index().unwrap_or(0);
                self.custom(l).map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Custom {}", i + 1))
            }
        }
    }

    /// Whether the current tab shows `p` (built-in tabs: as they are drawn).
    pub fn shows(&self, p: Panel) -> bool {
        match self.layout {
            Layout::Full => matches!(p, Panel::List | Panel::Alerts | Panel::Chart | Panel::Bubbles | Panel::Volume | Panel::Blacklist),
            Layout::List => matches!(p, Panel::List | Panel::Alerts | Panel::Bubbles | Panel::Volume),
            Layout::Chart => matches!(p, Panel::Chart | Panel::Blacklist),
            Layout::Bubbles => matches!(p, Panel::Bubbles | Panel::Volume),
            Layout::Alerts => p == Panel::Alerts,
            l => self.custom(l).is_some_and(|c| c.contains(p)),
        }
    }

    /// Whether the current tab uses the timeframe (the chart, the bubbles).
    pub fn uses_timeframe(&self) -> bool {
        self.shows(Panel::Chart) || self.shows(Panel::Bubbles)
    }

    pub fn filter_on(&self, name: &str) -> bool {
        self.filter_enabled.get(name).copied().unwrap_or(true)
    }

    /// Whether a list column is shown; Float and Shares are off until switched on.
    pub fn column_visible(&self, key: &str) -> bool {
        self.column_visibility.get(key).copied().unwrap_or(!matches!(key, "float" | "shares"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-config");
        fs::create_dir_all(&d).unwrap();
        d.join(format!("vs_cfg_test_{}_{}.json", name, std::process::id()))
    }

    #[test]
    fn missing_file_gives_defaults_without_writing() {
        let p = tmp("missing");
        let _ = fs::remove_file(&p);
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(!p.exists());
    }

    #[test]
    fn validation_fixes_bad_values() {
        let p = tmp("validate");
        fs::write(
            &p,
            r#"{"refresh_interval_seconds":0,"alert_history_max":2,"setup_vol_ratio":-1,"delta_window_seconds":0,"filter_enabled":{"ALL":false}}"#,
        )
        .unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.refresh_interval_seconds, 1);
        assert_eq!(cfg.alert_history_max, 10);
        assert_eq!(cfg.setup_vol_ratio, 2.2);
        assert_eq!(cfg.vol_m_window_seconds, 180);
        assert!(!cfg.filter_on("ALL"));
        assert!(cfg.filter_on("SETUP"));
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn old_delta_window_key_and_column_migrate() {
        let p = tmp("migrate");
        fs::write(&p, r#"{"delta_window_seconds":120,"column_visibility":{"delta":false,"mcap":true}}"#).unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.vol_m_window_seconds, 120);
        assert!(!cfg.column_visible("volrate"));
        assert!(!cfg.column_visibility.contains_key("delta"));
        // saved under the new name
        cfg.save(&p).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("vol_m_window_seconds") && !text.contains("delta_window_seconds"));
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn setup_defaults() {
        let c = Config::default();
        assert_eq!((c.setup_window_sec, c.setup_vol_ratio, c.setup_price_pct), (300, 2.2, 10.0));
        // an old file without any of the keys gets the defaults
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!((c.setup_window_sec, c.setup_vol_ratio, c.setup_price_pct), (300, 2.2, 10.0));
    }

    #[test]
    fn old_spike_pump_baseline_keys_migrate_to_setup_keys() {
        let p = tmp("setup_migrate");
        fs::write(&p, r#"{"spike_vol_ratio":3.5,"pump_price_pct":0.07,"baseline_window_sec":120,"min_price":0.3}"#).unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.setup_vol_ratio, 3.5);
        assert_eq!(cfg.setup_price_pct, 7.0); // fraction became percent, no float noise
        assert_eq!(cfg.setup_window_sec, 120);
        assert_eq!(cfg.min_price, 0.3);
        // saved under the new names only
        cfg.save(&p).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        for old in ["spike_vol_ratio", "pump_price_pct", "baseline_window_sec"] {
            assert!(!text.contains(old), "{old} still saved");
        }
        assert_eq!(Config::load(&p).unwrap(), cfg);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn old_pump_pct_above_one_is_already_a_percent() {
        let cfg = Config::from_json(r#"{"pump_price_pct":12}"#).unwrap();
        assert_eq!(cfg.setup_price_pct, 12.0);
        let cfg = Config::from_json(r#"{"pump_price_pct":1}"#).unwrap();
        assert_eq!(cfg.setup_price_pct, 100.0);
    }

    #[test]
    fn new_setup_keys_win_over_old_ones() {
        let cfg = Config::from_json(
            r#"{"spike_vol_ratio":9,"setup_vol_ratio":2.5,"pump_price_pct":0.5,"setup_price_pct":8,"baseline_window_sec":9,"setup_window_sec":60}"#,
        )
        .unwrap();
        assert_eq!((cfg.setup_vol_ratio, cfg.setup_price_pct, cfg.setup_window_sec), (2.5, 8.0, 60));
    }

    #[test]
    fn bad_setup_values_fall_back() {
        let mut cfg = Config { setup_vol_ratio: 0.0, setup_price_pct: -3.0, setup_window_sec: 0, ..Config::default() };
        cfg.validate();
        assert_eq!((cfg.setup_window_sec, cfg.setup_vol_ratio, cfg.setup_price_pct), (300, 2.2, 10.0));
    }

    #[test]
    fn spike_and_pump_are_not_alert_filters() {
        assert_eq!(FILTER_NAMES, ["ALL", "NEW", "SETUP"]);
    }

    #[test]
    fn vol_m_window_is_clamped_to_30() {
        let mut cfg = Config { vol_m_window_seconds: 5, ..Config::default() };
        cfg.validate();
        assert_eq!(cfg.vol_m_window_seconds, 30);
    }

    #[test]
    fn recompute_ms_accepts_only_500_or_1000() {
        assert_eq!(Config::default().recompute_ms, 1000);
        for (given, want) in [(500, 500), (1000, 1000), (0, 1000), (250, 1000), (750, 1000), (60_000, 1000)] {
            let mut cfg = Config { recompute_ms: given, ..Config::default() };
            cfg.validate();
            assert_eq!(cfg.recompute_ms, want, "given {given}");
        }
        // an old file without the key gets the default
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.recompute_ms, 1000);
    }

    #[test]
    fn alert_cooldown_defaults_to_120_and_zero_resets() {
        assert_eq!(Config::default().alert_cooldown_sec, 120);
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.alert_cooldown_sec, 120);
        let mut cfg = Config { alert_cooldown_sec: 0, ..Config::default() };
        cfg.validate();
        assert_eq!(cfg.alert_cooldown_sec, 120);
        let mut cfg = Config { alert_cooldown_sec: 45, ..Config::default() };
        cfg.validate();
        assert_eq!(cfg.alert_cooldown_sec, 45);
    }

    #[test]
    fn rank_interval_has_a_minimum_of_5() {
        let mut cfg = Config { rank_tracking_interval_seconds: 2, ..Config::default() };
        cfg.validate();
        assert_eq!(cfg.rank_tracking_interval_seconds, 5);
    }

    #[test]
    fn old_rate_tab_key_is_ignored() {
        let cfg: Config = serde_json::from_str(r#"{"rate_tab":"ticks"}"#).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn layout_toggles_and_alert_rows() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!((c.layout, c.ticker_tape, c.key_hints, c.alert_rows), (Layout::Full, true, true, 5));
        let mut c = Config::from_json(r#"{"layout":"alerts","ticker_tape":false,"alert_rows":40}"#).unwrap();
        c.validate();
        assert_eq!((c.layout, c.ticker_tape, c.key_hints, c.alert_rows), (Layout::Alerts, false, true, 15));
        assert_eq!(c.next_layout(), Layout::Full);
        c.layout = Layout::Full;
        assert_eq!(c.next_layout(), Layout::List);
    }

    #[test]
    fn custom_tabs_follow_the_built_in_ones_and_fall_back_when_gone() {
        let on = |name: &str| CustomLayout { on: true, name: name.into(), ..CustomLayout::default() };
        let mut c = Config { custom_layouts: vec![on("Mine"), CustomLayout::default(), on("")], ..Config::default() };
        c.validate();
        assert_eq!(&c.tabs()[5..], [Layout::Custom1, Layout::Custom3]);
        assert_eq!((c.layout_label(Layout::Custom1), c.layout_label(Layout::Custom3)), ("Mine".into(), "Custom 3".into()));
        c.layout = Layout::Alerts;
        assert_eq!(c.next_layout(), Layout::Custom1);
        c.layout = Layout::Custom3;
        assert_eq!(c.next_layout(), Layout::Full);
        // the default custom tab: chart | list
        c.layout = Layout::Custom1;
        assert!(c.shows(Panel::Chart) && c.uses_timeframe() && !c.shows(Panel::Alerts));
        // a saved tab that is no longer set up: Full
        let mut c = Config::from_json(r#"{"layout":"custom2"}"#).unwrap();
        c.validate();
        assert_eq!(c.layout, Layout::Full);
        // round trip
        let c = Config { custom_layouts: vec![on("Mine")], layout: Layout::Custom1, ..Config::default() };
        let back = Config::from_json(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!((back.layout, back.custom_layouts), (Layout::Custom1, c.custom_layouts));
    }

    #[test]
    fn formats_default_validate_and_roundtrip() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c.formats, crate::fmt::Formats::default());
        let mut c = Config::from_json(r#"{"formats":{"timezone":"Nowhere","date":"YYYY-MM-DD","clock_12h":true}}"#).unwrap();
        c.validate();
        assert_eq!((c.formats.timezone.as_str(), c.formats.date.as_str(), c.formats.clock_12h), ("Local", "YYYY-MM-DD", true));
        let p = tmp("formats");
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn float_columns_off_and_filters_zero_by_default() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert!(!c.column_visible("float") && !c.column_visible("shares") && c.column_visible("mcap"));
        assert_eq!((c.min_float, c.max_float, c.min_shares, c.max_shares), (0.0, 0.0, 0.0, 0.0));
        let c = Config::from_json(r#"{"column_visibility":{"float":true},"max_float":20000000}"#).unwrap();
        assert!(c.column_visible("float") && !c.column_visible("shares"));
        assert_eq!(c.max_float, 2e7);
    }

    #[test]
    fn save_roundtrip() {
        let p = tmp("roundtrip");
        let mut cfg = Config::default();
        cfg.min_price = 0.3;
        cfg.blacklisted_tickers = vec!["ABC".into()];
        cfg.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), cfg);
        fs::remove_file(&p).unwrap();
    }
}
