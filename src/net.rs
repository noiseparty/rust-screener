//! HTTP: the shared agent settings and JSON GETs inside the request budget (`limiter`).

use serde_json::Value;
use std::time::Duration;

pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).user_agent(USER_AGENT).build()
}

/// GET `url` inside the shared budget and parse the body as JSON. Errors as text.
pub fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    crate::limiter::get(agent, url)?.into_json().map_err(|e| e.to_string())
}
