use crate::net::USER_AGENT;
use std::time::{Duration, Instant};

pub struct CheckResult {
    pub name: &'static str,
    pub ok: bool,
    pub message: String,
}

fn check(name: &'static str, url: &str, expect: u16) -> CheckResult {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).user_agent(USER_AGENT).build();
    let start = Instant::now();
    // Webull counts against the shared request budget (and reports its 403 pause)
    let res =
        if url.contains("webullfintech.com") { crate::limiter::get(&agent, url) } else { agent.get(url).call().map_err(|e| e.to_string()) };
    let (ok, message) = match res {
        Ok(r) if r.status() == expect => (true, format!("OK ({}ms)", start.elapsed().as_millis())),
        Ok(r) => (false, format!("HTTP {}", r.status())),
        Err(e) => (false, e),
    };
    CheckResult { name, ok, message }
}

/// Connection checks: internet access and the stock data source (Webull).
pub fn run_all() -> Vec<CheckResult> {
    vec![
        check("Network", "http://connectivitycheck.gstatic.com/generate_204", 204),
        check("Stock Data Source", "https://quotes-gw.webullfintech.com/api/bgw/market/pcIndex?regionId=6&pageSize=1", 200),
    ]
}
