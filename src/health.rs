use crate::scraper::USER_AGENT;
use std::time::{Duration, Instant};

pub struct CheckResult {
    pub name: &'static str,
    pub ok: bool,
    pub message: String,
}

fn check(name: &'static str, url: &str, expect: u16) -> CheckResult {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build();
    let start = Instant::now();
    let (ok, message) = match agent.get(url).call() {
        Ok(r) if r.status() == expect => (true, format!("OK ({}ms)", start.elapsed().as_millis())),
        Ok(r) => (false, format!("HTTP {}", r.status())),
        Err(ureq::Error::Status(code, _)) => (false, format!("HTTP {code}")),
        Err(e) => (false, e.kind().to_string()),
    };
    CheckResult { name, ok, message }
}

pub fn run_all(finviz_proxy_url: &str) -> Vec<CheckResult> {
    let mut out = vec![
        check("Network", "http://connectivitycheck.gstatic.com/generate_204", 204),
        check(
            "Webull API",
            "https://quotes-gw.webullfintech.com/api/bgw/market/pcIndex?regionId=6&pageSize=1",
            200,
        ),
    ];
    let proxy = finviz_proxy_url.trim_end_matches('/');
    out.push(if proxy.is_empty() {
        CheckResult { name: "Finviz Proxy", ok: false, message: "Not configured".into() }
    } else {
        check("Finviz Proxy", &format!("{proxy}/api/health"), 200)
    });
    out
}
