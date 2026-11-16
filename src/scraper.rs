use crate::session::Session;
use serde_json::Value;
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const API: &str = "https://quotes-gw.webullfintech.com/api/bgw/market/pcIndex?regionId=6&pageSize=";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ticker {
    /// Webull tickerId (0 when the row had none); the key of the push stream.
    pub ticker_id: u64,
    pub symbol: String,
    pub price: f64,
    pub volume: i64,
    pub change_ratio: f64,
    pub market_value: f64,
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
    let body: Value = agent
        .get(&format!("{API}{page_size}"))
        .call()
        .map_err(|e| e.to_string())?
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
}
