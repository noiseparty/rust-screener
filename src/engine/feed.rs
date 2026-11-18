//! Stream quotes, the live overlay on REST rows, the receive-time series and Vol/m.

use super::{Engine, Live, Thresholds, LIVE_FRESH, MAX_SUBS, MIN_VOL_M_WINDOW_SECS};
use crate::scraper::Ticker;
use crate::series::{Ingest, Obs, SeriesStore, Source};
use crate::session::Session;
use crate::stream::Update;
use std::time::{Duration, Instant};

/// Series history kept beyond the longest window a consumer looks back over.
const SERIES_MARGIN: Duration = Duration::from_secs(60);

impl Engine {
    /// Receive-time series of every listed (or lingering) ticker, keyed by tickerId.
    pub fn series(&self) -> &SeriesStore {
        &self.series
    }

    /// The stream reconnected: everything it delivered before is stale.
    pub fn on_stream_reconnect(&mut self) {
        self.series.clear_all();
        self.refresh_vol_rates(Instant::now());
    }

    /// Whether the data feed is silent; Vol/m shows a dash while it is.
    pub fn set_feed_stale(&mut self, stale: bool) {
        self.feed_stale = stale;
    }

    /// Recomputes Vol/m of every row at `now`: a quiet ticker decays, a cleared series blanks.
    pub fn refresh_vol_rates(&mut self, now: Instant) {
        for t in &mut self.latest {
            (t.vol_rate, t.vol_partial, t.vol_stale) = vol_m(&self.series, t.ticker_id, now, self.th.vol_m_window, self.feed_stale);
        }
    }

    /// REST rows write to the series only while the stream is not healthy; otherwise the
    /// series holds stream samples only and a quiet ticker keeps its history.
    pub fn set_rest_feeds_series(&mut self, on: bool) {
        self.rest_feeds_series = on;
    }

    /// Drops lingering series whose 60 s ran out. Returns true when the subscription set changed.
    pub fn expire_series(&mut self, now: Instant) -> bool {
        self.series.expire(now)
    }

    /// Tickers to subscribe: the current list first, then lingering ones, up to `MAX_SUBS`.
    pub fn subscribe_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.latest.iter().map(|t| t.ticker_id).filter(|&i| i != 0).collect();
        ids.truncate(MAX_SUBS);
        let room = MAX_SUBS - ids.len();
        ids.extend(self.series.lingering().into_iter().take(room));
        ids
    }

    /// Folds a batch of stream quotes into the last processed rows (price, volume, change %, market cap).
    /// Records no alerts or history. Returns true when any row changed.
    pub fn apply_quotes(&mut self, updates: &[Update], session: Session, now: Instant) -> bool {
        let mut changed = false;
        for u in updates {
            // Extended-hours price is taken from pPrice when present; regular hours use close.
            let ext = matches!(session, Session::Pre | Session::After);
            let px = if ext { u.p_price.filter(|p| *p > 0.0).or(u.close) } else { u.close };
            // A message without price or volume is dropped, never read as zero.
            let Some(px) = px.filter(|p| *p > 0.0) else { continue };
            // The series store owns ordering, lower-volume and session/date rules.
            let ingest = u.volume.map(|vol| {
                let obs = Obs { id: u.id, src: Source::Stream, ms: u.trade_ms, seq: u.seq, px: Some(px), vol: Some(vol), at: u.recv };
                self.series.ingest(obs)
            });
            let (Some(vol), Some(ingest)) = (u.volume, ingest) else { continue };
            match ingest {
                Ingest::Dropped => continue,
                Ingest::Unchanged => {
                    // Nothing new, but the stream is alive for this ticker.
                    if let Some(l) = self.live.get_mut(&u.id) {
                        l.at = now;
                    }
                    continue;
                }
                Ingest::Stored => {}
            }
            let vol = vol as i64;
            let rate = vol_m(&self.series, u.id, now, self.th.vol_m_window, self.feed_stale);
            self.live.insert(u.id, Live { price: px, volume: vol, at: now });
            let Some(&i) = self.index.get(&u.id) else { continue };
            let t = &mut self.latest[i];
            t.price = px;
            (t.vol_rate, t.vol_partial, t.vol_stale) = rate;
            t.volume = vol;
            t.change_ratio = change_from(t.prev_close, px).or(u.change_ratio).unwrap_or(t.change_ratio);
            if let Some(m) = u.mcap.filter(|m| *m > 0.0) {
                t.market_value = m;
            }
            let sym = t.symbol.clone();
            self.record_sample(&sym, chrono::Local::now().timestamp(), px, vol);
            changed = true;
        }
        changed
    }

    /// Replaces a REST row's price/volume/change by the freshest stream quote, if there is one.
    /// Returns true when it did.
    pub(super) fn overlay_live(&self, t: &mut Ticker, now: Instant) -> bool {
        let Some(l) = self.live.get(&t.ticker_id).filter(|l| now.saturating_duration_since(l.at) <= LIVE_FRESH) else {
            return false;
        };
        t.price = l.price;
        t.volume = l.volume;
        if let Some(c) = change_from(t.prev_close, l.price) {
            t.change_ratio = c;
        }
        true
    }
}

/// Series history to keep: the longest look-back (Vol/m, or two SETUP windows) plus a margin.
pub(super) fn series_horizon(th: &Thresholds) -> Duration {
    th.vol_m_window.max(th.setup_window * 2) + SERIES_MARGIN
}

/// Vol/m of one ticker at `now`: (shares per minute, window only partly covered, feed stale).
///
/// The window ends at `now` and starts at `now - window`, or at the first point if the series is
/// younger. Under 30 s of coverage, or no series, gives no rate; a stale feed hides it.
pub(super) fn vol_m(series: &SeriesStore, id: u64, now: Instant, window: Duration, stale: bool) -> (Option<f64>, bool, bool) {
    if stale {
        return (None, false, true);
    }
    let window = window.max(Duration::from_secs(MIN_VOL_M_WINDOW_SECS));
    let Some(first) = series.oldest(id) else { return (None, false, false) };
    let start = now.checked_sub(window).map_or(first.t, |s| s.max(first.t));
    let covered = now.saturating_duration_since(start);
    if covered < Duration::from_secs(MIN_VOL_M_WINDOW_SECS) {
        return (None, false, false);
    }
    let (Some(end), Some(begin)) = (series.vol_at(id, now), series.vol_at(id, start)) else {
        return (None, false, false);
    };
    let rate = end.saturating_sub(begin) as f64 * 60.0 / covered.as_secs_f64();
    (Some(rate), covered < window, false)
}

/// Change ratio versus the previous close, when that is known.
fn change_from(prev_close: f64, price: f64) -> Option<f64> {
    (prev_close > 0.0).then(|| price / prev_close - 1.0)
}
