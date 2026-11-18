//! Per-ticker series of (receive time, cumulative day volume, price), keyed by tickerId.
//!
//! One time axis: the local receive `Instant` is used for every window. The exchange
//! timestamp only orders messages and decides which session/date a message belongs to.
//! Ingest and clear rules follow `docs/stream-design.md` 1.3 and 1.4.
//!
//! Consumers (Vol/m, SETUP, probe) read through `vol_at` / `px_at` (value at or before a
//! time), `oldest` and `latest`. All mutation goes through `ingest`, `set_members`,
//! `expire` and `clear_all`.

use crate::session::Session;
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::America::New_York;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// A ticker that left the list keeps its series this long, in case it comes back.
pub const LINGER: Duration = Duration::from_secs(60);
/// Messages closer together than this share one point.
const COALESCE: Duration = Duration::from_secs(1);
/// Consecutive lower cumulative-volume readings after which the lower value is believed.
const LOWER_ACCEPT: u8 = 3;

/// Where a sample came from. A ticker switching source starts a new series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Stream,
    Rest,
}

/// One point of a series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub t: Instant,
    pub vol: u64,
    pub px: f64,
}

/// One observation offered to the store.
#[derive(Debug, Clone, Copy)]
pub struct Obs {
    pub id: u64,
    pub src: Source,
    /// Exchange timestamp in ms for stream messages; wall-clock ms for REST rows. 0 = unknown.
    pub ms: i64,
    /// Exchange sequence number (stream only; breaks ties between equal timestamps).
    pub seq: u64,
    pub px: Option<f64>,
    /// Cumulative day volume. A missing value drops the observation; it is never read as 0.
    pub vol: Option<u64>,
    /// Receive time.
    pub at: Instant,
}

/// What `ingest` did with an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingest {
    /// Stored as a new point or merged into the last point.
    Stored,
    /// Valid and in order, but identical to the last point. Nothing stored.
    Unchanged,
    /// Rejected: unknown id, missing data, out of order, older session, or a lone lower volume.
    Dropped,
}

/// Session and New York date of a timestamp, ordered by time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Epoch(NaiveDate, u8);

impl Epoch {
    fn of(ms: i64) -> Option<Epoch> {
        let ny = DateTime::<Utc>::from_timestamp_millis(ms)?.with_timezone(&New_York);
        let rank = match Session::at(ny) {
            Session::Closed if ny.hour() < 12 => 0,
            Session::Pre => 1,
            Session::Reg => 2,
            Session::After => 3,
            Session::Closed => 4,
        };
        Some(Epoch(ny.date_naive(), rank))
    }
}

#[derive(Debug, Default)]
struct Series {
    pts: VecDeque<Sample>,
    lower: u8,
    src: Option<Source>,
    /// (tradeStamp ms, seq) of the last accepted stream message.
    last: Option<(i64, u64)>,
    /// When the ticker left the list; `None` while it is a member.
    left_at: Option<Instant>,
}

impl Series {
    fn clear_points(&mut self) {
        self.pts.clear();
        self.lower = 0;
        self.last = None;
    }

    /// Last point with time <= `t`.
    fn at(&self, t: Instant) -> Option<Sample> {
        let i = self.pts.partition_point(|p| p.t <= t);
        i.checked_sub(1).map(|i| self.pts[i])
    }
}

pub struct SeriesStore {
    map: HashMap<u64, Series>,
    horizon: Duration,
    epoch: Option<Epoch>,
    resets: u64,
}

impl SeriesStore {
    /// `horizon`: how far back lookups must reach. One anchor point at or before
    /// `now - horizon` is always kept.
    pub fn new(horizon: Duration) -> SeriesStore {
        SeriesStore { map: HashMap::new(), horizon, epoch: None, resets: 0 }
    }

    pub fn set_horizon(&mut self, horizon: Duration) {
        self.horizon = horizon;
    }

    /// Number of all-series clears so far (session/date change, reconnect). Other state that
    /// must reset at the same moments (rank ring, badges) can watch this counter.
    #[cfg(test)]
    pub fn resets(&self) -> u64 {
        self.resets
    }

    /// Declares the tickers currently on the list. Unknown ids get an empty series; ids that
    /// left start their linger clock; ids that came back stop it.
    pub fn set_members(&mut self, ids: impl IntoIterator<Item = u64>, now: Instant) {
        let mut current = std::collections::HashSet::new();
        for id in ids {
            if id != 0 && current.insert(id) {
                self.map.entry(id).or_default().left_at = None;
            }
        }
        for (id, s) in self.map.iter_mut() {
            if !current.contains(id) && s.left_at.is_none() {
                s.left_at = Some(now);
            }
        }
    }

    /// Evicts series whose linger has run out. Returns true when any was removed.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.map.len();
        self.map.retain(|_, s| s.left_at.is_none_or(|at| now.saturating_duration_since(at) < LINGER));
        self.map.len() != before
    }

    /// Ids that left the list and are still lingering.
    pub fn lingering(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.map.iter().filter(|(_, s)| s.left_at.is_some()).map(|(&id, _)| id).collect();
        v.sort_unstable();
        v
    }

    /// Clears every series (stream reconnect). Membership and linger clocks stay.
    pub fn clear_all(&mut self) {
        for s in self.map.values_mut() {
            s.clear_points();
            s.src = None;
        }
        self.resets += 1;
    }

    pub fn ingest(&mut self, o: Obs) -> Ingest {
        if !self.map.contains_key(&o.id) {
            return Ingest::Dropped;
        }
        // Missing or unusable data is dropped, never read as zero.
        let (Some(px), Some(vol)) = (o.px.filter(|p| p.is_finite() && *p > 0.0), o.vol) else {
            return Ingest::Dropped;
        };
        // Session / date: a later one clears everything, an earlier one is a stale message.
        if let Some(e) = (o.ms > 0).then(|| Epoch::of(o.ms)).flatten() {
            match self.epoch {
                None => self.epoch = Some(e),
                Some(cur) if e > cur => {
                    self.epoch = Some(e);
                    self.clear_all();
                }
                Some(cur) if e < cur => return Ingest::Dropped,
                _ => {}
            }
        }
        let horizon = self.horizon;
        let s = self.map.get_mut(&o.id).expect("checked above");
        if s.src != Some(o.src) {
            s.clear_points();
            s.src = Some(o.src);
        }
        // 1. Order (stream only): by (tradeStamp, seq), never by seq alone.
        if o.src == Source::Stream {
            if let Some((ms, seq)) = s.last {
                if !(o.ms > ms || (o.ms == ms && o.seq > seq)) {
                    return Ingest::Dropped;
                }
            }
        }
        // Only messages that get past the checks below advance the order state.
        let stamp = (o.src == Source::Stream).then_some((o.ms, o.seq));
        let new = Sample { t: o.at, vol, px };
        let Some(back) = s.pts.back().copied() else {
            s.last = stamp;
            s.pts.push_back(new);
            return Ingest::Stored;
        };
        // 3. Volume going down: drop; believe it after LOWER_ACCEPT in a row. Never clamp.
        if vol < back.vol {
            s.lower += 1;
            if s.lower < LOWER_ACCEPT {
                return Ingest::Dropped;
            }
            s.clear_points();
            s.last = stamp;
            s.pts.push_back(new);
            return Ingest::Stored;
        }
        s.lower = 0;
        s.last = stamp;
        // 4. No change.
        if vol == back.vol && px == back.px {
            return Ingest::Unchanged;
        }
        // 5. Coalesce within 1 s into the last point (keeping its time), else append.
        if o.at.saturating_duration_since(back.t) < COALESCE {
            let b = s.pts.back_mut().expect("non-empty");
            b.vol = vol;
            b.px = px;
        } else {
            s.pts.push_back(new);
        }
        // 6. Prune, keeping exactly one anchor at or before `now - horizon`.
        if let Some(cut) = o.at.checked_sub(horizon) {
            while s.pts.len() > 1 && s.pts[1].t <= cut {
                s.pts.pop_front();
            }
        }
        Ingest::Stored
    }

    /// Cumulative volume of the last point at or before `t`; `None` if `t` precedes the series.
    pub fn vol_at(&self, id: u64, t: Instant) -> Option<u64> {
        self.sample_at(id, t).map(|p| p.vol)
    }

    /// Price of the last point at or before `t`; `None` if `t` precedes the series.
    pub fn px_at(&self, id: u64, t: Instant) -> Option<f64> {
        self.sample_at(id, t).map(|p| p.px)
    }

    pub fn sample_at(&self, id: u64, t: Instant) -> Option<Sample> {
        self.map.get(&id)?.at(t)
    }

    /// Oldest retained point (fallback when history is shorter than a window).
    pub fn oldest(&self, id: u64) -> Option<Sample> {
        self.map.get(&id)?.pts.front().copied()
    }

    #[cfg(test)]
    pub fn latest(&self, id: u64) -> Option<Sample> {
        self.map.get(&id)?.pts.back().copied()
    }

    #[cfg(test)]
    pub fn len(&self, id: u64) -> usize {
        self.map.get(&id).map_or(0, |s| s.pts.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 2026-01-12 14:30 UTC = 09:30 New York (Reg opens).
    fn ms(h: u32, m: u32, s: u32) -> i64 {
        New_York.with_ymd_and_hms(2026, 1, 12, h, m, s).unwrap().timestamp_millis()
    }

    fn store() -> (SeriesStore, Instant) {
        let mut st = SeriesStore::new(Duration::from_secs(120));
        let t0 = Instant::now();
        st.set_members([1, 2], t0);
        (st, t0)
    }

    fn obs(id: u64, src: Source, ms: i64, seq: u64, px: f64, vol: u64, at: Instant) -> Obs {
        Obs { id, src, ms, seq, px: Some(px), vol: Some(vol), at }
    }

    fn s(id: u64, ms_: i64, seq: u64, px: f64, vol: u64, at: Instant) -> Obs {
        obs(id, Source::Stream, ms_, seq, px, vol, at)
    }

    fn secs(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_secs(n)
    }

    #[test]
    fn stores_and_looks_up_step_values() {
        let (mut st, t0) = store();
        let base = ms(10, 0, 0);
        for (i, v) in [100u64, 200, 300].iter().enumerate() {
            let n = i as u64 * 10;
            assert_eq!(st.ingest(s(1, base + n as i64 * 1000, 1, 1.0 + i as f64, *v, secs(t0, n))), Ingest::Stored);
        }
        assert_eq!(st.len(1), 3);
        // before the first point: None
        assert_eq!(st.vol_at(1, t0.checked_sub(Duration::from_secs(1)).unwrap()), None);
        assert_eq!(st.vol_at(1, t0), Some(100));
        assert_eq!(st.vol_at(1, secs(t0, 9)), Some(100));
        assert_eq!(st.vol_at(1, secs(t0, 10)), Some(200));
        assert_eq!(st.vol_at(1, secs(t0, 500)), Some(300));
        assert_eq!(st.px_at(1, secs(t0, 15)), Some(2.0));
        assert_eq!(st.oldest(1).unwrap().vol, 100);
        assert_eq!(st.latest(1).unwrap().vol, 300);
        assert_eq!(st.vol_at(2, t0), None);
        assert_eq!(st.vol_at(99, t0), None);
    }

    #[test]
    fn unknown_ids_and_missing_data_are_dropped() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        assert_eq!(st.ingest(s(99, b, 1, 1.0, 1, t0)), Ingest::Dropped);
        let mut o = s(1, b, 1, 1.0, 1, t0);
        o.vol = None;
        assert_eq!(st.ingest(o), Ingest::Dropped);
        let mut o = s(1, b, 1, 1.0, 1, t0);
        o.px = None;
        assert_eq!(st.ingest(o), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b, 1, 0.0, 1, t0)), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b, 1, f64::NAN, 1, t0)), Ingest::Dropped);
        assert_eq!(st.len(1), 0);
        // a real zero volume is a valid reading
        assert_eq!(st.ingest(s(1, b, 1, 1.0, 0, t0)), Ingest::Stored);
    }

    #[test]
    fn ordering_by_stamp_then_seq() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        assert_eq!(st.ingest(s(1, b, 5, 1.0, 100, t0)), Ingest::Stored);
        assert_eq!(st.ingest(s(1, b - 1, 9, 1.0, 200, secs(t0, 2))), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b, 5, 1.0, 200, secs(t0, 2))), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b, 4, 1.0, 200, secs(t0, 2))), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b, 6, 1.0, 200, secs(t0, 2))), Ingest::Stored);
        // a newer stamp wins whatever the seq
        assert_eq!(st.ingest(s(1, b + 1, 0, 1.0, 300, secs(t0, 4))), Ingest::Stored);
        assert_eq!(st.latest(1).unwrap().vol, 300);
        // ordering is per ticker
        assert_eq!(st.ingest(s(2, b - 5000, 1, 1.0, 10, secs(t0, 4))), Ingest::Stored);
    }

    #[test]
    fn lower_volume_dropped_then_believed_after_three() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(1, b, 1, 1.0, 1000, t0));
        assert_eq!(st.ingest(s(1, b + 1000, 1, 1.0, 50, secs(t0, 2))), Ingest::Dropped);
        assert_eq!(st.ingest(s(1, b + 2000, 1, 1.0, 50, secs(t0, 3))), Ingest::Dropped);
        assert_eq!(st.latest(1).unwrap().vol, 1000);
        // the third in a row clears the series and restarts from this sample
        assert_eq!(st.ingest(s(1, b + 3000, 1, 1.0, 50, secs(t0, 4))), Ingest::Stored);
        assert_eq!(st.len(1), 1);
        assert_eq!(st.oldest(1).unwrap().vol, 50);
        assert_eq!(st.vol_at(1, secs(t0, 3)), None);
    }

    #[test]
    fn a_good_reading_resets_the_lower_count() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(1, b, 1, 1.0, 1000, t0));
        st.ingest(s(1, b + 1000, 1, 1.0, 50, secs(t0, 2)));
        st.ingest(s(1, b + 2000, 1, 1.0, 50, secs(t0, 3)));
        // equal volume and price counts as a good reading
        assert_eq!(st.ingest(s(1, b + 3000, 1, 1.0, 1000, secs(t0, 4))), Ingest::Unchanged);
        st.ingest(s(1, b + 4000, 1, 1.0, 50, secs(t0, 5)));
        st.ingest(s(1, b + 5000, 1, 1.0, 50, secs(t0, 6)));
        assert_eq!(st.latest(1).unwrap().vol, 1000);
        assert_eq!(st.len(1), 1);
    }

    #[test]
    fn unchanged_is_skipped_and_close_messages_coalesce() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        assert_eq!(st.ingest(s(1, b, 1, 2.0, 100, t0)), Ingest::Stored);
        assert_eq!(st.ingest(s(1, b + 100, 1, 2.0, 100, secs(t0, 5))), Ingest::Unchanged);
        assert_eq!(st.len(1), 1);
        // price-only change within 1 s overwrites the last point, keeping its time
        assert_eq!(st.ingest(s(1, b + 200, 1, 2.5, 100, t0 + Duration::from_millis(400))), Ingest::Stored);
        assert_eq!(st.len(1), 1);
        let l = st.latest(1).unwrap();
        assert_eq!((l.t, l.px, l.vol), (t0, 2.5, 100));
        // a steady stream of messages 0.6 s apart still makes progress (new point every >= 1 s)
        for i in 1..=10u64 {
            let at = t0 + Duration::from_millis(600 * i);
            st.ingest(s(1, b + 1000 + i as i64, 1, 2.5, 100 + i, at));
        }
        assert!(st.len(1) >= 4);
    }

    #[test]
    fn prune_keeps_one_anchor_before_horizon() {
        let (mut st, t0) = store(); // horizon 120 s
        let b = ms(10, 0, 0);
        for i in 0..=200u64 {
            st.ingest(s(1, b + i as i64 * 1000, 1, 1.0, 100 + i, secs(t0, i)));
        }
        let now = secs(t0, 200);
        let oldest = st.oldest(1).unwrap();
        assert!(oldest.t <= now - Duration::from_secs(120));
        assert!(oldest.t > now - Duration::from_secs(122)); // anchor within a second of the horizon
                                                            // lookup at exactly now - horizon still resolves
        assert!(st.vol_at(1, now - Duration::from_secs(120)).is_some());
        assert_eq!(st.len(1), 121);
    }

    #[test]
    fn session_change_clears_all_series() {
        let (mut st, t0) = store();
        st.ingest(s(1, ms(9, 29, 58), 1, 1.0, 100, t0));
        st.ingest(s(2, ms(9, 29, 58), 1, 1.0, 100, t0));
        let r = st.resets();
        // first Reg message: both series are wiped, this one starts anew
        assert_eq!(st.ingest(s(1, ms(9, 30, 1), 1, 1.0, 5, secs(t0, 3))), Ingest::Stored);
        assert_eq!(st.resets(), r + 1);
        assert_eq!(st.len(1), 1);
        assert_eq!(st.oldest(1).unwrap().vol, 5);
        assert_eq!(st.len(2), 0);
        // ordering state was cleared too: an older stamp from the new session is accepted for id 2
        assert_eq!(st.ingest(s(2, ms(9, 30, 0), 1, 1.0, 7, secs(t0, 4))), Ingest::Stored);
        // a late message from the previous session is stale
        assert_eq!(st.ingest(s(2, ms(9, 29, 59), 9, 1.0, 9, secs(t0, 5))), Ingest::Dropped);
        assert_eq!(st.resets(), r + 1);
    }

    #[test]
    fn date_change_clears_all_series() {
        let (mut st, t0) = store();
        let day1 = New_York.with_ymd_and_hms(2026, 1, 12, 10, 0, 0).unwrap().timestamp_millis();
        let day2 = New_York.with_ymd_and_hms(2026, 1, 13, 10, 0, 0).unwrap().timestamp_millis();
        st.ingest(s(1, day1, 1, 1.0, 100, t0));
        assert_eq!(st.ingest(s(1, day2, 1, 1.0, 40, secs(t0, 5))), Ingest::Stored);
        assert_eq!(st.len(1), 1);
        assert_eq!(st.oldest(1).unwrap().vol, 40);
        assert_eq!(st.resets(), 1);
    }

    #[test]
    fn closed_gaps_rank_between_sessions() {
        let (mut st, t0) = store();
        st.ingest(s(1, ms(19, 59, 0), 1, 1.0, 100, t0)); // After
                                                         // 20:01 same day (Closed) is later than After: clears
        assert_eq!(st.ingest(s(1, ms(20, 1, 0), 1, 1.0, 100, secs(t0, 2))), Ingest::Stored);
        assert_eq!(st.resets(), 1);
        // 21:00 same Closed period: no clear
        st.ingest(s(1, ms(21, 0, 0), 1, 1.0, 110, secs(t0, 4)));
        assert_eq!(st.resets(), 1);
    }

    #[test]
    fn reconnect_clears_all_but_keeps_membership() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(1, b, 5, 1.0, 100, t0));
        st.ingest(s(2, b, 5, 1.0, 100, t0));
        st.clear_all();
        assert_eq!((st.len(1), st.len(2)), (0, 0));
        assert_eq!(st.resets(), 1);
        // ordering state is gone: the new connection may restart sequence numbers
        assert_eq!(st.ingest(s(1, b + 1000, 1, 1.0, 100, secs(t0, 5))), Ingest::Stored);
        assert_eq!(st.len(1), 1);
    }

    #[test]
    fn source_switch_clears_that_ticker_only() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(1, b, 1, 1.0, 100, t0));
        st.ingest(s(2, b, 1, 1.0, 100, t0));
        let rest = obs(1, Source::Rest, b + 6000, 0, 1.1, 150, secs(t0, 6));
        assert_eq!(st.ingest(rest), Ingest::Stored);
        assert_eq!(st.len(1), 1);
        assert_eq!(st.oldest(1).unwrap().vol, 150);
        assert_eq!(st.len(2), 1);
        // REST rows need no ordering: same stamp again is accepted
        assert_eq!(st.ingest(obs(1, Source::Rest, b + 6000, 0, 1.2, 160, secs(t0, 11))), Ingest::Stored);
        // back to the stream: cleared again, stream ordering starts fresh
        assert_eq!(st.ingest(s(1, b + 1000, 1, 1.0, 100, secs(t0, 12))), Ingest::Stored);
        assert_eq!(st.len(1), 1);
        assert_eq!(st.oldest(1).unwrap().vol, 100);
        assert_eq!(st.resets(), 0);
    }

    #[test]
    fn linger_then_evict() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(1, b, 1, 1.0, 100, t0));
        st.ingest(s(2, b, 1, 1.0, 100, t0));
        // 2 leaves the list
        st.set_members([1], secs(t0, 10));
        assert_eq!(st.lingering(), vec![2]);
        assert!(!st.expire(secs(t0, 69)));
        assert_eq!(st.len(2), 1);
        // still accepts data while lingering
        assert_eq!(st.ingest(s(2, b + 1000, 1, 1.0, 120, secs(t0, 30))), Ingest::Stored);
        assert!(st.expire(secs(t0, 70)));
        assert_eq!(st.len(2), 0);
        assert!(st.lingering().is_empty());
        assert_eq!(st.ingest(s(2, b + 2000, 1, 1.0, 130, secs(t0, 71))), Ingest::Dropped);
        assert_eq!(st.len(1), 1);
    }

    #[test]
    fn returning_member_keeps_its_series() {
        let (mut st, t0) = store();
        let b = ms(10, 0, 0);
        st.ingest(s(2, b, 1, 1.0, 100, t0));
        st.set_members([1], secs(t0, 10));
        st.set_members([1, 2], secs(t0, 40));
        assert!(st.lingering().is_empty());
        assert!(!st.expire(secs(t0, 500)));
        assert_eq!(st.len(2), 1);
        // leaving again restarts the clock
        st.set_members([1], secs(t0, 500));
        assert!(!st.expire(secs(t0, 559)));
        assert!(st.expire(secs(t0, 560)));
    }
}
