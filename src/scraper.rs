use crate::session::Session;
use chrono::NaiveDate;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const API: &str = "https://quotes-gw.webullfintech.com/api/bgw/market/pcIndex?regionId=6&pageSize=";
const QUOTE_API: &str = "https://quotes-gw.webullfintech.com/api/bgw/quote/realtime?ids=";
/// Ticker ids per float request.
pub const FLOAT_BATCH: usize = 20;
/// Float cache rows older than this many days are dropped when the cache is read.
const FLOAT_KEEP_DAYS: i64 = 30;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ticker {
    /// Webull tickerId (0 when the row had none); the key of the push stream.
    pub ticker_id: u64,
    pub symbol: String,
    pub price: f64,
    pub volume: i64,
    pub change_ratio: f64,
    pub market_value: f64,
    /// Float (Webull's `outstandingShares`) and total shares; 0 = unknown.
    pub float: f64,
    pub total_shares: f64,
    /// Previous close, derived from price and change ratio; basis for locally computed change %.
    pub prev_close: f64,
    /// Shares per minute over the Vol/m window; `None` while there is too little history.
    pub vol_rate: Option<f64>,
    /// The Vol/m window is not yet fully covered by history.
    pub vol_partial: bool,
    /// The feed is silent, so `vol_rate` is not shown.
    pub vol_stale: bool,
    /// Rank movement on the full board; set by the engine's recompute tick.
    pub mov: Mov,
    /// Row no longer passes a display filter and is on its way out (dimmed).
    pub dim: bool,
    pub status: String,
}

/// Rank change of a ticker on the full gainers board over the tracking interval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mov {
    /// No history yet: shown blank.
    #[default]
    None,
    /// Not on the board at the start of the interval: shown as a cyan `+`.
    New,
    /// Places gained (positive) or lost (negative).
    Moved(i32),
}

pub struct Fetched {
    pub tickers: Vec<Ticker>,
    pub group: String,
    pub session: Session,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
}

pub fn fetch_top_gainers(agent: &ureq::Agent, page_size: usize) -> Result<Fetched, String> {
    let body: Value = crate::limiter::get(agent, &format!("{API}{page_size}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let session = Session::now();
    parse(&body, session)
}

pub fn parse(body: &Value, session: Session) -> Result<Fetched, String> {
    let groups = body
        .get("groups")
        .or_else(|| body.get("data"))
        .and_then(Value::as_array)
        .ok_or("could not find groups or data in response")?;

    let mut found: Option<(&Vec<Value>, String)> = None;
    'outer: for target in session.group_targets() {
        for g in groups {
            let name = g.get("name").and_then(Value::as_str).unwrap_or("");
            if !name.trim().to_lowercase().contains(target) {
                continue;
            }
            let items = g
                .get("data")
                .and_then(Value::as_array)
                .or_else(|| g.get("items").and_then(Value::as_array));
            if let Some(items) = items.filter(|i| !i.is_empty()) {
                found = Some((items, name.to_string()));
                break 'outer;
            }
        }
    }
    let (items, group) =
        found.ok_or_else(|| format!("no gainers group for session {}", session.code()))?;

    let tickers = items
        .iter()
        .filter(|i| i.is_object())
        .map(|item| {
            let data = item.get("ticker").filter(|t| t.is_object()).unwrap_or(item);
            let vals = item.get("values");
            let mut price = num(vals, "price");
            if price == 0.0 {
                price = num(Some(data), "close");
            }
            let mut change = num(vals, "changeRatio");
            if change == 0.0 {
                change = num(Some(data), "changeRatio");
            }
            Ticker {
                ticker_id: num(Some(data), "tickerId") as u64,
                prev_close: if price > 0.0 && change > -1.0 { price / (1.0 + change) } else { 0.0 },
                symbol: data.get("symbol").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                price,
                volume: num(Some(data), "volume") as i64,
                change_ratio: change,
                market_value: num(Some(data), "marketValue"),
                ..Default::default()
            }
        })
        .collect();

    Ok(Fetched { tickers, group, session })
}

/// Float and total shares of a ticker, and the New York date they were fetched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Float {
    pub float: f64,
    pub total: f64,
    pub date: NaiveDate,
}

/// Float and total shares of `ids` (at most `FLOAT_BATCH`) in one request: `(id, float, total)`.
pub fn fetch_floats(agent: &ureq::Agent, ids: &[u64]) -> Result<Vec<(u64, f64, f64)>, String> {
    let ids: Vec<String> = ids.iter().map(u64::to_string).collect();
    let url = format!("{QUOTE_API}{}&includeSecu=1&delay=0&more=1", ids.join(","));
    let body: Value = crate::limiter::get(agent, &url)?.into_json().map_err(|e| e.to_string())?;
    parse_floats(&body)
}

/// Rows of a `quote/realtime` answer (a JSON array) as `(id, float, total)`.
pub fn parse_floats(body: &Value) -> Result<Vec<(u64, f64, f64)>, String> {
    let rows = body.as_array().ok_or("quote answer is not a list")?;
    Ok(rows
        .iter()
        .map(|r| (num(Some(r), "tickerId") as u64, num(Some(r), "outstandingShares"), num(Some(r), "totalShares")))
        .filter(|r| r.0 != 0)
        .collect())
}

/// Sets each ticker's float and total shares from `floats` (unknown when it has none).
pub fn fill_floats(tickers: &mut [Ticker], floats: &HashMap<u64, Float>) {
    for t in tickers {
        let f = floats.get(&t.ticker_id);
        t.float = f.map_or(0.0, |f| f.float);
        t.total_shares = f.map_or(0.0, |f| f.total);
    }
}

/// The float cache file (`id,float,total,YYYY-MM-DD` per line); rows older than
/// `FLOAT_KEEP_DAYS` before `today` are left out. Empty when there is no file.
pub fn load_floats(path: &Path, today: NaiveDate) -> HashMap<u64, Float> {
    let Ok(text) = fs::read_to_string(path) else { return HashMap::new() };
    text.lines()
        .filter_map(|l| {
            let mut p = l.trim().split(',');
            let id = p.next()?.parse().ok()?;
            let float = p.next()?.parse().ok()?;
            let total = p.next()?.parse().ok()?;
            let date = NaiveDate::parse_from_str(p.next()?, "%Y-%m-%d").ok()?;
            ((today - date).num_days() <= FLOAT_KEEP_DAYS).then_some((id, Float { float, total, date }))
        })
        .collect()
}

/// Writes the whole float cache.
pub fn save_floats(path: &Path, floats: &HashMap<u64, Float>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text: String = floats.iter().map(|(id, f)| format!("{id},{},{},{}\n", f.float, f.total, f.date.format("%Y-%m-%d"))).collect();
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

fn num(obj: Option<&Value>, key: &str) -> f64 {
    match obj.and_then(|o| o.get(key)) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.replace(',', "").trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_nested_and_string_fields() {
        let body = json!({"groups": [
            {"name": "Losers", "data": [{"ticker": {"symbol": "X"}}]},
            {"name": "Top Gainers", "data": [
                {"ticker": {"symbol": "ABC", "close": "1.50", "volume": "1,234", "changeRatio": "0.25", "marketValue": "5000000"},
                 "values": {"price": "1.75", "changeRatio": "0.30"}},
                {"symbol": "DEF", "close": 2.0, "volume": 10, "changeRatio": 0.1}
            ]}
        ]});
        let f = parse(&body, Session::Reg).unwrap();
        assert_eq!(f.group, "Top Gainers");
        assert_eq!(f.tickers.len(), 2);
        assert_eq!(f.tickers[0].symbol, "ABC");
        assert_eq!(f.tickers[0].price, 1.75);
        assert_eq!(f.tickers[0].change_ratio, 0.30);
        assert_eq!(f.tickers[0].volume, 1234);
        assert_eq!(f.tickers[1].price, 2.0);
    }

    #[test]
    fn prefers_session_group() {
        let body = json!({"data": [
            {"name": "Top Gainers", "items": [{"symbol": "REG"}]},
            {"name": "Pre-Market Gainers", "items": [{"symbol": "PRE"}]}
        ]});
        assert_eq!(parse(&body, Session::Pre).unwrap().tickers[0].symbol, "PRE");
        assert_eq!(parse(&body, Session::Reg).unwrap().tickers[0].symbol, "REG");
    }

    #[test]
    fn parses_float_batch() {
        // shape of a live `quote/realtime` answer (trimmed); the ETF has no share counts
        let body = json!([
            {"tickerId": 950155382, "symbol": "GP", "close": "1.410", "totalShares": "2100709", "outstandingShares": "1297073", "negMarketValue": "1828872.93"},
            {"tickerId": 913256135, "symbol": "AAPL", "totalShares": "14532182000", "outstandingShares": "14272214729"},
            {"tickerId": 913243251, "symbol": "SPY", "template": "etf"},
            {"symbol": "NOID"}
        ]);
        let rows = parse_floats(&body).unwrap();
        assert_eq!(rows, vec![(950155382, 1_297_073.0, 2_100_709.0), (913256135, 14_272_214_729.0, 14_532_182_000.0), (913243251, 0.0, 0.0)]);
        assert!(parse_floats(&json!({"code": 403})).is_err());
    }

    #[test]
    fn float_cache_round_trip_drops_old_rows() {
        let d = std::path::PathBuf::from("target").join("test-floats");
        let p = d.join("floats.csv");
        let _ = fs::remove_file(&p);
        let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let old = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let mut m = HashMap::new();
        m.insert(1, Float { float: 1_297_073.0, total: 2_100_709.0, date: today });
        m.insert(2, Float { float: 0.0, total: 0.0, date: today });
        save_floats(&p, &m).unwrap();
        assert_eq!(load_floats(&p, today), m);
        m.insert(3, Float { float: 5.0, total: 6.0, date: old });
        save_floats(&p, &m).unwrap();
        assert_eq!(load_floats(&p, today).len(), 2);
        assert!(load_floats(&d.join("missing.csv"), today).is_empty());
        let mut t = vec![Ticker { ticker_id: 1, ..Default::default() }, Ticker { ticker_id: 9, float: 7.0, ..Default::default() }];
        fill_floats(&mut t, &m);
        assert_eq!((t[0].float, t[0].total_shares, t[1].float), (1_297_073.0, 2_100_709.0, 0.0));
    }
}
