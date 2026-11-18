//! 1-minute candles: built from samples, merged with backfilled or saved bars, handed to storage.

use super::{Candle, Engine, MAX_CANDLES};
use std::collections::VecDeque;

impl Engine {
    pub fn candles(&self, sym: &str) -> Option<&VecDeque<Candle>> {
        self.candles.get(sym)
    }

    /// Start of the first candle held for `sym`.
    pub fn first_candle_start(&self, sym: &str) -> Option<i64> {
        self.candles.get(sym)?.front().map(|c| c.start)
    }

    /// Puts backfilled or saved bars among the held candles, in order of start, before, between
    /// or after them. A minute already held is kept (our own candles are never replaced); past
    /// `MAX_CANDLES` the oldest go. Returns the bars taken and still held, oldest first.
    pub fn insert_candles(&mut self, sym: &str, bars: &[Candle]) -> Vec<Candle> {
        let held = self.candles.entry(sym.to_string()).or_default();
        let mut added = Vec::new();
        for c in bars {
            if let Err(i) = held.binary_search_by_key(&c.start, |h| h.start) {
                held.insert(i, *c);
                added.push(*c);
            }
        }
        let extra = held.len().saturating_sub(MAX_CANDLES);
        held.drain(..extra);
        let first = held.front().map_or(i64::MAX, |c| c.start);
        added.retain(|c| c.start >= first);
        added.sort_by_key(|c| c.start);
        added
    }

    /// Start of the newest candle of `sym` written to storage.
    pub fn stored_upto(&self, sym: &str) -> Option<i64> {
        self.stored_upto.get(sym).copied()
    }

    /// Candles whose minute has ended by `now_epoch` and that are not stored yet, oldest first
    /// per symbol. They are recorded before display filters, like all candles.
    pub fn completed_candles(&self, now_epoch: i64) -> Vec<(String, Candle)> {
        let mut out = Vec::new();
        for (sym, bars) in &self.candles {
            let done = self.stored_upto.get(sym).copied().unwrap_or(i64::MIN);
            let first_new = bars.iter().rposition(|c| c.start <= done).map_or(0, |i| i + 1);
            for c in bars.iter().skip(first_new).take_while(|c| c.start + 60 <= now_epoch) {
                out.push((sym.clone(), *c));
            }
        }
        out
    }

    /// Marks candles (as returned by `completed_candles`) as written to storage.
    pub fn mark_stored(&mut self, rows: &[(String, Candle)]) {
        for (sym, c) in rows {
            let up = self.stored_upto.entry(sym.clone()).or_insert(i64::MIN);
            *up = (*up).max(c.start);
        }
    }

    /// Folds one price/cumulative-volume sample into the symbol's 1-minute candles.
    pub fn record_sample(&mut self, sym: &str, epoch_secs: i64, price: f64, cum_vol: i64) {
        if price <= 0.0 {
            return;
        }
        let prev = self.last_cum_vol.insert(sym.to_string(), cum_vol);
        let traded = prev.map_or(0, |p| (cum_vol - p).max(0));
        let start = epoch_secs - epoch_secs.rem_euclid(60);
        let bars = self.candles.entry(sym.to_string()).or_default();
        match bars.back_mut() {
            Some(c) if c.start == start => {
                c.high = c.high.max(price);
                c.low = c.low.min(price);
                c.close = price;
                c.volume += traded;
            }
            Some(c) if c.start > start => {}
            _ => {
                bars.push_back(Candle { start, open: price, high: price, low: price, close: price, volume: traded });
                if bars.len() > MAX_CANDLES {
                    bars.pop_front();
                }
            }
        }
    }
}
