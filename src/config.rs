use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

pub const FILTER_NAMES: [&str; 5] = ["ALL", "NEW", "PUMP", "SPIKE", "SETUP"];

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

    // SPIKE / PUMP alert thresholds
    pub spike_vol_ratio: f64,
    pub pump_price_pct: f64,
    pub baseline_window_sec: u64,
    pub min_volume_for_alert: i64,
    pub min_price_for_alert: f64,

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
            spike_vol_ratio: 2.0,
            pump_price_pct: 0.05,
            baseline_window_sec: 300,
            min_volume_for_alert: 10_000,
            min_price_for_alert: 1.0,
            min_change_percent: 0.0,
            max_change_percent: 0.0,
            min_volume: 0,
            max_volume: 0,
            min_price: 0.0,
            max_price: 0.0,
            vol_m_window_seconds: 180,
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
        let mut cfg: Config = serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        cfg.validate();
        Ok(cfg)
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
        }
        if self.spike_vol_ratio <= 0.0 {
            self.spike_vol_ratio = 2.0;
        }
        if self.pump_price_pct <= 0.0 {
            self.pump_price_pct = 0.05;
        }
        if self.baseline_window_sec == 0 {
            self.baseline_window_sec = 300;
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
            r#"{"refresh_interval_seconds":0,"alert_history_max":2,"spike_vol_ratio":-1,"delta_window_seconds":0,"filter_enabled":{"ALL":false}}"#,
        )
        .unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.refresh_interval_seconds, 1);
        assert_eq!(cfg.alert_history_max, 10);
        assert_eq!(cfg.spike_vol_ratio, 2.0);
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
    fn vol_m_window_is_clamped_to_30() {
        let mut cfg = Config { vol_m_window_seconds: 5, ..Config::default() };
        cfg.validate();
        assert_eq!(cfg.vol_m_window_seconds, 30);
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
