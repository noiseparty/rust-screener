//! Mov (rank change on the full board) and the table sort order.

use super::{AlertKind, Engine};
use crate::scraper::{Mov, Ticker};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

impl Engine {
    /// Mov: rank change on the full board ranked by % change, over `rank_interval`. Independent of
    /// the user's sort and filters; blacklisted symbols are left out of both boards.
    pub(super) fn rank_board(&mut self, now: Instant) {
        if self.latest.is_empty() {
            return;
        }
        let mut board: Vec<&Ticker> = self.latest.iter().collect();
        board.sort_by(|a, b| board_order(a, b));
        let order: Vec<String> = board.iter().map(|t| t.symbol.clone()).collect();

        // Baseline: the board in force `rank_interval` ago, if we have been running that long.
        let cutoff = now.checked_sub(self.th.rank_interval);
        let old = cutoff.and_then(|c| self.rank_ring.iter().rev().find(|(at, _)| *at <= c)).map(|(_, o)| o.as_slice());
        let movs = movements(old, &order, &self.blacklist);
        for t in &mut self.latest {
            t.mov = movs.get(t.symbol.as_str()).copied().unwrap_or_default();
        }

        if self.rank_ring.back().is_none_or(|(_, last)| *last != order) {
            self.rank_ring.push_back((now, order));
        }
        // Keep exactly one entry at or before the cutoff: it is the baseline.
        if let Some(c) = cutoff {
            while self.rank_ring.get(1).is_some_and(|(at, _)| *at <= c) {
                self.rank_ring.pop_front();
            }
        }
    }
}

/// Full-board order: % change, then volume (both descending), then symbol.
fn board_order(a: &Ticker, b: &Ticker) -> Ordering {
    b.change_ratio
        .partial_cmp(&a.change_ratio)
        .unwrap_or(Ordering::Equal)
        .then(b.volume.cmp(&a.volume))
        .then_with(|| a.symbol.cmp(&b.symbol))
}

/// Rank movement of every symbol of `new` against `old` (both best first), blacklisted symbols
/// removed from both. No `old` board gives blanks; a symbol missing from it is a new entrant.
fn movements<'a>(old: Option<&[String]>, new: &'a [String], blacklist: &BTreeSet<String>) -> HashMap<&'a str, Mov> {
    let old_rank: Option<HashMap<&str, i32>> =
        old.map(|o| o.iter().filter(|s| !blacklist.contains(*s)).enumerate().map(|(i, s)| (s.as_str(), i as i32 + 1)).collect());
    new.iter()
        .filter(|s| !blacklist.contains(*s))
        .enumerate()
        .map(|(i, s)| {
            let mov = match &old_rank {
                None => Mov::None,
                Some(r) => r.get(s.as_str()).map_or(Mov::New, |&was| Mov::Moved(was - (i as i32 + 1))),
            };
            (s.as_str(), mov)
        })
        .collect()
}

/// Places gained on the board; `None` for blank and new-entrant rows.
fn mov_places(t: &Ticker) -> Option<i32> {
    match t.mov {
        Mov::Moved(n) => Some(n),
        Mov::None | Mov::New => None,
    }
}

/// Sort order for `key`. Rows without a Vol/m rate, or without a Mov rank change (blank or a
/// new entrant), go last in both directions.
pub(super) fn ordered(a: &Ticker, b: &Ticker, key: &str, desc: bool) -> Ordering {
    let blanks = match key {
        "volrate" => Some((a.vol_rate.is_some(), b.vol_rate.is_some())),
        "mov" => Some((mov_places(a).is_some(), mov_places(b).is_some())),
        _ => None,
    };
    match blanks {
        Some((false, true)) => return Ordering::Greater,
        Some((true, false)) => return Ordering::Less,
        Some((false, false)) => return a.symbol.cmp(&b.symbol),
        _ => {}
    }
    let o = compare(a, b, key);
    if desc {
        o.reverse()
    } else {
        o
    }
}

pub fn compare(a: &Ticker, b: &Ticker, key: &str) -> Ordering {
    let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    let primary = match key {
        "mov" => mov_places(a).unwrap_or(0).cmp(&mov_places(b).unwrap_or(0)),
        "symbol" => a.symbol.cmp(&b.symbol),
        "price" => f(a.price, b.price),
        "change" => f(a.change_ratio, b.change_ratio),
        "volume" => a.volume.cmp(&b.volume),
        "mcap" => f(a.market_value, b.market_value),
        "float" => f(a.float, b.float),
        "shares" => f(a.total_shares, b.total_shares),
        "volrate" => f(a.vol_rate.unwrap_or(0.0), b.vol_rate.unwrap_or(0.0)),
        "status" => a.status.map(AlertKind::label).cmp(&b.status.map(AlertKind::label)),
        _ => f(a.change_ratio, b.change_ratio).then(a.volume.cmp(&b.volume)),
    };
    primary.then_with(|| a.symbol.cmp(&b.symbol))
}
