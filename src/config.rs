use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Alert kinds that can be raised and filtered. SPIKE and PUMP are hidden for now (their enum
/// variants stay in the engine so they can return).
pub const FILTER_NAMES: [&str; 3] = ["ALL", "NEW", "SETUP"];

/// Old config keys and the SETUP keys that replaced them (value copied as is).
const SETUP_KEY_MIGRATION: [(&str, &str); 2] =
    [("spike_vol_ratio", "setup_vol_ratio"), ("baseline_window_sec", "setup_window_sec")];

/// The two panels of the rate area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateTab {
    /// Price up / down ticks.
    Ticks,
    /// Session new highs / new lows (the default; an unknown value in the file also lands here).
    #[default]
    #[serde(other)]
    HighsLows,
}

impl RateTab {
    pub fn toggled(self) -> RateTab {
        match self {
            RateTab::HighsLows => RateTab::Ticks,
            RateTab::Ticks => RateTab::HighsLows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            RateTab::HighsLows => "New Highs/Lows",
            RateTab::Ticks => "Up/Down ticks",
        }
    }
}

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
    /// Volume now divided by volume `setup_window_sec` ago. Old files: `spike_vol_ratio`.
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

    /// Window of the Vol/m column. Old files call it `delta_window_seconds`.
    #[serde(alias = "delta_window_seconds")]
    pub vol_m_window_seconds: u64,
    /// Which panel the rate area shows; `t` toggles it.
    pub rate_tab: RateTab,
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
            vol_m_window_seconds: 180,
            rate_tab: RateTab::default(),
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
    pub fn load(path: &Path) -> io::Result<Config> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save(path)?;
            return Ok(cfg);
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
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, data)
    }

    pub fn validate(&mut self) {
        if self.refresh_interval_seconds < 1 {
            self.refresh_interval_seconds = 1;
        }
        if self.alert_history_max < 10 {
            self.alert_history_max = 10;
        }
        if self.rank_tracking_interval_seconds == 0 {
            self.rank_tracking_interval_seconds = 60;
        } else if self.rank_tracking_interval_seconds < 5 {
            self.rank_tracking_interval_seconds = 5;
        }
        if self.recompute_ms != 500 {
            self.recompute_ms = 1000;
        }
        if !(self.setup_vol_ratio > 0.0 && self.setup_vol_ratio.is_finite()) {
            self.setup_vol_ratio = 2.2;
        }
        if !(self.setup_price_pct > 0.0 && self.setup_price_pct.is_finite()) {
            self.setup_price_pct = 10.0;
        }
        if self.setup_window_sec == 0 {
            self.setup_window_sec = 300;
        }
        if self.alert_cooldown_sec == 0 {
            self.alert_cooldown_sec = 120;
        }
        if self.vol_m_window_seconds == 0 {
            self.vol_m_window_seconds = 180;
        } else if self.vol_m_window_seconds < 30 {
            self.vol_m_window_seconds = 30;
        }
        // The "delta" column became "volrate" (Vol/m).
        if let Some(v) = self.column_visibility.remove("delta") {
            self.column_visibility.entry("volrate".to_string()).or_insert(v);
        }
        for f in FILTER_NAMES {
            self.filter_enabled.entry(f.to_string()).or_insert(true);
        }
    }

    pub fn filter_on(&self, name: &str) -> bool {
        self.filter_enabled.get(name).copied().unwrap_or(true)
    }

    pub fn column_visible(&self, key: &str) -> bool {
        self.column_visibility.get(key).copied().unwrap_or(true)
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
    fn missing_file_writes_defaults() {
        let p = tmp("missing");
        let _ = fs::remove_file(&p);
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(p.exists());
        fs::remove_file(&p).unwrap();
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
    fn rate_tab_defaults_to_highs_lows_and_roundtrips() {
        assert_eq!(Config::default().rate_tab, RateTab::HighsLows);
        // an old file without the key gets the default
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.rate_tab, RateTab::HighsLows);
        // an unknown value does not stop the file from loading
        let cfg: Config = serde_json::from_str(r#"{"rate_tab":"bogus"}"#).unwrap();
        assert_eq!(cfg.rate_tab, RateTab::HighsLows);

        let p = tmp("rate_tab");
        let cfg = Config { rate_tab: Config::default().rate_tab.toggled(), ..Config::default() };
        assert_eq!(cfg.rate_tab, RateTab::Ticks);
        cfg.save(&p).unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains(r#""rate_tab": "ticks""#));
        let back = Config::load(&p).unwrap();
        assert_eq!(back.rate_tab, RateTab::Ticks);
        assert_eq!(back.rate_tab.toggled(), RateTab::HighsLows);
        fs::remove_file(&p).unwrap();
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
