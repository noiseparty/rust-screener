//! Fetched lists, the recompute tick, config changes and the filtered, sorted table.

use super::feed::{series_horizon, vol_m};
use super::ranking::ordered;
use super::{Engine, Filters, Output, Thresholds, FILTER_GRACE, MAX_ROWS};
use crate::config::Config;
use crate::scraper::Ticker;
use crate::series::{Obs, Source};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

impl Engine {
    pub fn apply_config(&mut self, cfg: &Config) {
        let new_filters = Filters::from_config(cfg);
        let filters_changed = new_filters != self.filters;
        self.filters = new_filters;
        self.th = Thresholds::from_config(cfg);
        self.series.set_horizon(series_horizon(&self.th));
        while self.alerts.len() > self.th.max_alerts {
            self.alerts.pop_front();
        }
        // New filters apply at once: nothing lingers from the old ones.
        self.failing_since.clear();
        self.passing = self.latest.iter().filter(|t| self.shown(t)).map(|t| t.symbol.clone()).collect();
        // Whatever the new filters let through is the list as of now, not a flood of new symbols.
        if filters_changed {
            self.seed_listed(Instant::now());
        }
    }

    /// Passes the display filters now (blacklisted rows are listed separately, never here).
    pub(super) fn shown(&self, t: &Ticker) -> bool {
        !self.blacklist.contains(&t.symbol) && self.filters.matches(t)
    }

    /// Sets the float and total shares of every row from the float cache.
    pub fn set_floats(&mut self, floats: &HashMap<u64, crate::scraper::Float>) {
        crate::scraper::fill_floats(&mut self.latest, floats);
    }

    pub fn toggle_blacklist(&mut self, sym: &str) {
        if !self.blacklist.remove(sym) {
            self.blacklist.insert(sym.to_string());
        }
    }

    pub fn process(&mut self, tickers: Vec<Ticker>, sort_key: &str, sort_desc: bool, now: Instant) -> Output {
        let wall = chrono::Local::now();
        let epoch = wall.timestamp();
        let th = self.th;
        let mut latest = Vec::with_capacity(tickers.len());

        self.series.set_members(tickers.iter().map(|t| t.ticker_id), now);
        self.series.expire(now);
        let wall_ms = wall.timestamp_millis();

        for mut t in tickers {
            let overlaid = self.overlay_live(&mut t, now);
            // A row still carrying a fresh stream quote adds nothing; otherwise REST feeds the
            // series when the stream is down. REST has no volume of 0 for a trading ticker, so 0 means missing.
            if self.rest_feeds_series && !overlaid {
                self.series.ingest(Obs {
                    id: t.ticker_id,
                    src: Source::Rest,
                    ms: wall_ms,
                    seq: 0,
                    px: Some(t.price),
                    vol: u64::try_from(t.volume).ok().filter(|v| *v > 0),
                    at: now,
                });
            }
            (t.vol_rate, t.vol_partial, t.vol_stale) = vol_m(&self.series, t.ticker_id, now, th.vol_m_window, self.feed_stale);

            // NEW and SETUP are evaluated on the recompute tick.
            self.record_sample(&t.symbol, epoch, t.price, t.volume);
            latest.push(t);
        }

        while self.alerts.len() > th.max_alerts {
            self.alerts.pop_front();
        }

        self.index = latest.iter().enumerate().filter(|(_, t)| t.ticker_id != 0).map(|(i, t)| (t.ticker_id, i)).collect();
        self.live.retain(|id, _| self.index.contains_key(id));
        self.latest = latest;

        // A fetch is rare and changes membership: rank and filter at once instead of at the next tick.
        let new_alerts = self.recompute_at(now, true);
        let mut out = self.reapply(sort_key, sort_desc, now);
        out.new_alerts = new_alerts;
        out
    }

    /// The recompute tick: re-ranks the full board (Mov), moves rows into or out of the filter
    /// grace and raises NEW and SETUP alerts from the stream-fed rows. Stream quotes only change
    /// data; the display follows on this tick. Returns the number of alerts raised.
    pub fn recompute(&mut self, now: Instant) -> usize {
        self.recompute_at(now, false)
    }

    /// `fetched`: the list was just replaced by a fetch (the first one after a reseed seeds NEW).
    fn recompute_at(&mut self, now: Instant, fetched: bool) -> usize {
        self.rank_board(now);
        self.update_display(now);
        let raised = self.evaluate_new(now, fetched) + self.evaluate_setups(now);
        let hold = self.badge_hold();
        self.fired.retain(|_, f| {
            for at in f.iter_mut() {
                if at.is_some_and(|a| now.saturating_duration_since(a) >= hold) {
                    *at = None;
                }
            }
            f.iter().any(Option::is_some)
        });
        raised
    }

    /// Tracks which symbols pass the display filters; a symbol that stops passing starts its grace.
    fn update_display(&mut self, now: Instant) {
        let mut passing = HashSet::new();
        let mut failing = HashMap::new();
        for t in &self.latest {
            if self.blacklist.contains(&t.symbol) {
                continue;
            }
            if self.filters.matches(t) {
                passing.insert(t.symbol.clone());
            } else if let Some(&since) = self.failing_since.get(&t.symbol) {
                failing.insert(t.symbol.clone(), since);
            } else if self.passing.contains(&t.symbol) {
                failing.insert(t.symbol.clone(), now);
            }
        }
        self.passing = passing;
        self.failing_since = failing;
    }

    /// Whether a row is on the board at `now`, and if so whether it is dimmed (filter grace).
    fn display_state(&self, t: &Ticker, now: Instant) -> Option<bool> {
        if self.filters.matches(t) {
            return Some(false);
        }
        match self.failing_since.get(&t.symbol) {
            Some(&since) => (now.saturating_duration_since(since) < FILTER_GRACE).then_some(true),
            // Failing since a moment ago; the next recompute starts its grace.
            None => self.passing.contains(&t.symbol).then_some(true),
        }
    }

    /// Re-filter and re-sort the last processed tickers with the current filters, blacklist and
    /// sort. Records no history, ticks, alerts or rank samples and leaves Mov as last computed.
    pub fn reapply(&self, sort_key: &str, sort_desc: bool, now: Instant) -> Output {
        let mut active = Vec::new();
        let mut blacklisted = Vec::new();
        for t in &self.latest {
            if self.blacklist.contains(&t.symbol) {
                blacklisted.push(t.clone());
            } else if let Some(dim) = self.display_state(t, now) {
                let status = self.badge(&t.symbol, now);
                active.push(Ticker { dim, status, ..t.clone() });
            }
        }
        active.sort_by(|a, b| ordered(a, b, sort_key, sort_desc));
        active.truncate(MAX_ROWS);
        Output { active, blacklisted, new_alerts: 0 }
    }
}
