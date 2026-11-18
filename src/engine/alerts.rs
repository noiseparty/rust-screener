//! NEW and SETUP alerts, the per-kind cooldown and the Status badge.

use super::{Alert, AlertKind, Engine, SetupBase, Thresholds, BADGE_HOLD, NEW_QUIET};
use crate::scraper::Ticker;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a symbol's last time on the list is remembered.
const LISTED_KEEP: Duration = Duration::from_secs(2400);
/// Badge priority when several alert kinds of one symbol are held: first wins.
const BADGE_PRIORITY: [AlertKind; 4] = [AlertKind::Setup, AlertKind::Pump, AlertKind::Spike, AlertKind::New];

impl Engine {
    /// SETUP baseline of `id` at `now` with cumulative volume `vol`, from the series store:
    /// the volume traded in the last `setup_window`, in the window before it, and the price one
    /// window ago. A previous window only partly covered (history shorter than two windows) is
    /// scaled to a full window; `None` when less than a tenth of it is covered, or no series.
    fn setup_base(&self, id: u64, now: Instant, vol: i64) -> Option<SetupBase> {
        let w = self.th.setup_window;
        let first = self.series().oldest(id)?;
        let mid = now.checked_sub(w)?;
        let from = mid.checked_sub(w).map_or(first.t, |s| s.max(first.t));
        let covered = mid.checked_duration_since(from)?;
        if covered.is_zero() || covered < w / 10 {
            return None;
        }
        let (mid_vol, from_vol) = (self.series().vol_at(id, mid)?, self.series().vol_at(id, from)?);
        let prev = mid_vol.saturating_sub(from_vol) as f64 * w.as_secs_f64() / covered.as_secs_f64();
        Some(SetupBase { prev: prev.round() as u64, cur: (vol.max(0) as u64).saturating_sub(mid_vol), px: self.series().px_at(id, mid)? })
    }

    /// The trading session changed: the next fetched list only seeds NEW, it raises none.
    pub fn on_session_change(&mut self) {
        self.reseed = true;
    }

    /// Records every row that passes the display filters as listed at `now`, raising nothing.
    pub(super) fn seed_listed(&mut self, now: Instant) {
        for t in &self.latest {
            if self.filters.matches(t) {
                self.listed.insert(t.symbol.clone(), now);
            }
        }
    }

    pub fn alerts(&self) -> &VecDeque<Alert> {
        &self.alerts
    }

    /// NEW: a row on the list (passing the display filters) now that was not on it in the last
    /// `NEW_QUIET`. While a reseed is pending (startup, session change) ticks do nothing and the
    /// next fetched list is only recorded. Blacklisted rows are recorded but never alert.
    pub(super) fn evaluate_new(&mut self, now: Instant, fetched: bool) -> usize {
        if self.reseed {
            if fetched {
                self.reseed = false;
                self.seed_listed(now);
            }
            return 0;
        }
        let mut hits = Vec::new();
        for t in &self.latest {
            if !self.filters.matches(t) {
                continue;
            }
            let gone = self.listed.get(&t.symbol).is_none_or(|&at| now.saturating_duration_since(at) >= NEW_QUIET);
            if gone && !self.blacklist.contains(&t.symbol) {
                hits.push(t.clone());
            }
        }
        self.seed_listed(now);
        self.listed.retain(|_, at| now.saturating_duration_since(*at) < LISTED_KEEP);
        let epoch = chrono::Utc::now().timestamp();
        let mut raised = 0;
        for t in hits {
            let base = self.setup_base(t.ticker_id, now, t.volume);
            if self.raise(AlertKind::New, &t, base, now, epoch) {
                raised += 1;
            }
        }
        raised
    }

    /// SETUP on every shown, eligible row at its current price and volume. The cooldown stops a
    /// row that keeps meeting the limits from being raised again at each tick.
    pub(super) fn evaluate_setups(&mut self, now: Instant) -> usize {
        // Without fresh data the look-back window only slides over old samples; nothing is new.
        if self.feed_stale {
            return 0;
        }
        let th = self.th;
        let mut hits = Vec::new();
        for t in &self.latest {
            let eligible = t.volume >= th.min_volume_for_alert && t.price >= th.min_price_for_alert;
            if !eligible || !self.shown(t) || !self.can_fire(&t.symbol, AlertKind::Setup, now) {
                continue;
            }
            let base = self.setup_base(t.ticker_id, now, t.volume);
            if setup_met(&th, base, t.price) {
                hits.push((t.clone(), base));
            }
        }
        if hits.is_empty() {
            return 0;
        }
        let epoch = chrono::Utc::now().timestamp();
        let mut raised = 0;
        for (t, base) in hits {
            if self.raise(AlertKind::Setup, &t, base, now, epoch) {
                raised += 1;
            }
        }
        raised
    }

    /// How long an alert's Status label is held: the cooldown, but at least `BADGE_HOLD`.
    pub(super) fn badge_hold(&self) -> Duration {
        self.th.alert_cooldown.max(BADGE_HOLD)
    }

    /// Whether the cooldown of `kind` for `sym` has run out.
    pub(super) fn can_fire(&self, sym: &str, kind: AlertKind, now: Instant) -> bool {
        let last = self.fired.get(sym).and_then(|f| f[kind.slot()]);
        last.is_none_or(|at| now.saturating_duration_since(at) >= self.th.alert_cooldown)
    }

    /// Logs an alert unless the symbol is blacklisted or still cooling down for this kind, and
    /// starts the cooldown and the Status badge. `base` holds the volume of the previous and the
    /// last SETUP window and the price one window ago.
    pub(super) fn raise(&mut self, kind: AlertKind, t: &Ticker, base: Option<SetupBase>, now: Instant, epoch: i64) -> bool {
        if self.blacklist.contains(&t.symbol) || !self.can_fire(&t.symbol, kind, now) {
            return false;
        }
        self.fired.entry(t.symbol.clone()).or_default()[kind.slot()] = Some(now);
        self.alerts.push_back(Alert {
            at: now,
            epoch,
            symbol: t.symbol.clone(),
            kind,
            prev_vol: base.map_or(-1, |b| b.prev as i64),
            curr_vol: base.map_or(t.volume, |b| b.cur as i64),
            prev_price: base.map_or(-1.0, |b| b.px),
            curr_price: t.price,
            change_ratio: t.change_ratio,
        });
        while self.alerts.len() > self.th.max_alerts {
            self.alerts.pop_front();
        }
        true
    }

    /// The alert label shown in the Status column: the highest-priority kind still held.
    pub(super) fn badge(&self, sym: &str, now: Instant) -> Option<AlertKind> {
        let fired = self.fired.get(sym)?;
        let hold = self.badge_hold();
        BADGE_PRIORITY.into_iter().find(|k| fired[k.slot()].is_some_and(|at| now.saturating_duration_since(at) < hold))
    }
}

/// SETUP: the volume of the last `setup_window` is at least `setup_vol_ratio` times the volume
/// of the window before it (after a window without trades any volume counts), and the price is
/// up at least `setup_price_pct` percent on one window ago. Both limits are inclusive. Windows,
/// not the day's total, so a ticker that spikes again later in the day still fires.
pub(super) fn setup_met(th: &Thresholds, base: Option<SetupBase>, price: f64) -> bool {
    // Slack for float noise, so a move of exactly 10% off 1.00 counts as 10%.
    const EPS: f64 = 1e-9;
    let Some(b) = base else { return false };
    if b.cur == 0 || !(b.px > 0.0) {
        return false;
    }
    let vol_ok = b.cur as f64 + EPS >= th.setup_vol_ratio * b.prev as f64;
    let pct = (price - b.px) / b.px * 100.0;
    vol_ok && pct + EPS >= th.setup_price_pct
}
