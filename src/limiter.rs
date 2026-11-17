//! One request budget for every Webull REST call (gainers list, chart bars, backfill, 4h
//! levels), so their combined rate stays under Webull's CDN limit. ~30 requests/s for ~3
//! minutes got the IP a 403 on every quotes-gw endpoint; normal use never did.
//!
//! Requests get evenly spaced slots at `RATE` per second. A 403 pauses every request for
//! `PAUSE`, after which the rate is halved.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Requests per second across all threads.
const RATE: f64 = 5.0;
const MIN_RATE: f64 = 0.5;
/// How long everything stops after a 403.
const PAUSE: Duration = Duration::from_secs(15 * 60);

struct State {
    next: Instant,
    rate: f64,
    paused_until: Option<Instant>,
    /// Slot times of the requests of the last minute (for the health pane).
    recent: VecDeque<Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State, Instant) -> T) -> T {
    let now = Instant::now();
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| State { next: now, rate: RATE, paused_until: None, recent: VecDeque::new() });
    f(s, now)
}

/// Time left of a 403 pause, if one is running.
pub fn paused() -> Option<Duration> {
    with(|s, now| s.paused_until.and_then(|p| p.checked_duration_since(now)).filter(|d| !d.is_zero()))
}

/// Requests sent (or waiting for their slot) in the last minute.
pub fn per_minute() -> usize {
    with(|s, now| last_minute(&mut s.recent, now))
}

/// Drops times older than a minute and counts the rest.
fn last_minute(times: &mut VecDeque<Instant>, now: Instant) -> usize {
    while times.front().is_some_and(|&t| now.saturating_duration_since(t) >= Duration::from_secs(60)) {
        times.pop_front();
    }
    times.len()
}

/// Waits for the next free slot. False while paused: the request must be skipped.
fn take() -> bool {
    let wait = with(|s, now| {
        if s.paused_until.is_some_and(|p| now < p) {
            return None;
        }
        let slot = s.next.max(now);
        s.next = slot + Duration::from_secs_f64(1.0 / s.rate);
        last_minute(&mut s.recent, now);
        s.recent.push_back(slot);
        Some(slot - now)
    });
    match wait {
        Some(w) => {
            std::thread::sleep(w);
            true
        }
        None => false,
    }
}

/// A 403: pause everything and halve the rate (once per pause, however many threads saw it).
fn blocked() {
    with(|s, now| {
        if s.paused_until.is_none_or(|p| now >= p) {
            s.paused_until = Some(now + PAUSE);
            s.rate = (s.rate / 2.0).max(MIN_RATE);
        }
    });
}

/// GET `url` inside the budget. Errors as text; a 403 also starts the pause.
pub fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::Response, String> {
    if !take() {
        let m = paused().map_or(0, |d| d.as_secs().div_ceil(60));
        return Err(format!("Webull paused after HTTP 403 (rate limit), {m} min left"));
    }
    match agent.get(url).call() {
        Ok(r) => Ok(r),
        Err(ureq::Error::Status(403, _)) => {
            blocked();
            Err("HTTP 403 from Webull (rate limit): all requests paused 15 min".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::last_minute;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[test]
    fn counts_requests_of_the_last_minute() {
        let now = Instant::now() + Duration::from_secs(120);
        let s = |n| now - Duration::from_secs(n);
        let mut times: VecDeque<Instant> = [s(90), s(60), s(59), s(1), now].into();
        assert_eq!(last_minute(&mut times, now), 3);
        assert_eq!(times.len(), 3);
        assert_eq!(last_minute(&mut VecDeque::new(), now), 0);
    }
}
