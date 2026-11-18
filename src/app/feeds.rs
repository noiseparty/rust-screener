//! Background feeds: 1m backfill, 4h levels, floats and the candle store.

use super::*;

impl App {
    pub fn attach_backfill(&mut self, tx: Sender<backfill::Job>) {
        self.backfill_tx = Some(tx);
    }

    /// The levels worker and the cache folder (pruned here: files unused for 30 days).
    pub fn attach_levels(&mut self, tx: Sender<levels::Req>, dir: &Path) {
        let _ = levels::prune(dir);
        self.levels_tx = Some(tx);
        self.levels_dir = Some(dir.to_path_buf());
    }

    /// 4h level bars for `sym`, from memory, else the disk cache, else empty.
    pub(super) fn level_data(&mut self, sym: &str, now: i64) -> &levels::Data {
        if !self.levels.contains_key(sym) {
            let d = self.levels_dir.as_deref().and_then(|dir| levels::load(dir, sym, now)).unwrap_or_default();
            self.levels.insert(sym.to_string(), d);
        }
        &self.levels[sym]
    }

    /// The next batch of 4h level bars, one batch at a time, most urgent ticker first:
    /// the charted one (after `BACKFILL_DWELL`), SETUP alerts, list tickers whose year is not
    /// fetched yet (top to bottom), then the list's newly finished bars.
    pub(super) fn dispatch_levels(&mut self, now: Instant) {
        let Some(tx) = self.levels_tx.clone() else { return };
        let t = chrono::Utc::now().timestamp();
        // a new day: today's bars no longer grey out or remove lines the same way
        let day = crate::nytime::date(t);
        if self.levels_day != Some(day) {
            self.levels_day = Some(day);
            self.levels.values_mut().for_each(|d| d.redraw(t));
        }
        if self.levels_pending.is_some() || crate::limiter::paused().is_some() {
            return;
        }
        let mut urgent: Vec<String> = Vec::new();
        if let Some((s, since)) = &self.charted_since {
            if now.saturating_duration_since(*since) >= BACKFILL_DWELL {
                urgent.push(s.clone());
            }
        }
        urgent.extend(self.engine.alerts().iter().rev().filter(|a| a.kind == AlertKind::Setup).map(|a| a.symbol.clone()));
        let list: Vec<String> = self.tickers.iter().map(|t| t.symbol.clone()).collect();
        let passes: [(&[String], bool); 3] = [(&urgent, false), (&list, true), (&list, false)];
        for (syms, only_new) in passes {
            for sym in syms {
                if self.levels_retry_at.get(sym).is_some_and(|&at| now < at) {
                    continue;
                }
                let Some(ticker_id) = self.ticker_id(sym) else { continue };
                let d = self.level_data(sym, t);
                if only_new && d.complete(t) {
                    continue;
                }
                if let Some((from, to)) = d.next_range(t) {
                    if tx.send(levels::Req { symbol: sym.clone(), ticker_id, from, to }).is_ok() {
                        self.levels_pending = Some(sym.clone());
                    }
                    return;
                }
            }
        }
    }

    pub(super) fn on_levels(&mut self, symbol: String, from: i64, to: i64, bars: Option<Vec<Candle>>) {
        self.levels_pending = None;
        let now = chrono::Utc::now().timestamp();
        let d = self.levels.entry(symbol.clone()).or_default();
        // None: a request failed (or a 403 pause started, which dispatch also waits out)
        let Some(bars) = bars else {
            self.levels_retry_at.insert(symbol, Instant::now() + HISTORY_RETRY);
            return;
        };
        if !d.apply(from, to, &bars, now) {
            self.levels_retry_at.insert(symbol, Instant::now() + LEVELS_RETRY);
            return;
        }
        self.levels_retry_at.remove(&symbol);
        if let Some(dir) = &self.levels_dir {
            let _ = levels::save(dir, &symbol, d);
        }
    }

    /// The float worker and the cache file (read here).
    pub fn attach_floats(&mut self, tx: Sender<Vec<u64>>, path: &Path) {
        self.floats = scraper::load_floats(path, crate::nytime::date(chrono::Utc::now().timestamp()));
        self.floats_tx = Some(tx);
        self.floats_path = Some(path.to_path_buf());
    }

    /// The next batch of gainers-list ids without a float fetched today (New York), one batch
    /// at a time; a failed batch waits `HISTORY_RETRY`.
    pub(super) fn dispatch_floats(&mut self, now: Instant) {
        let Some(tx) = &self.floats_tx else { return };
        if !self.floats_pending.is_empty() || self.floats_retry_at.is_some_and(|at| now < at) || crate::limiter::paused().is_some() {
            return;
        }
        let today = crate::nytime::date(chrono::Utc::now().timestamp());
        let ids: Vec<u64> = self
            .all
            .iter()
            .map(|t| t.ticker_id)
            .filter(|id| *id != 0 && self.floats.get(id).is_none_or(|f| f.date != today))
            .take(scraper::FLOAT_BATCH)
            .collect();
        if !ids.is_empty() && tx.send(ids.clone()).is_ok() {
            self.floats_pending = ids;
        }
    }

    /// A float answer: stored (an id the answer left out is unknown for today), saved and shown.
    pub(super) fn on_floats(&mut self, res: Result<Vec<(u64, f64, f64)>, String>) {
        let asked = std::mem::take(&mut self.floats_pending);
        let Ok(rows) = res else {
            self.floats_retry_at = Some(Instant::now() + HISTORY_RETRY);
            return;
        };
        let date = crate::nytime::date(chrono::Utc::now().timestamp());
        for id in asked {
            self.floats.insert(id, Float { float: 0.0, total: 0.0, date });
        }
        for (id, float, total) in rows {
            self.floats.insert(id, Float { float, total, date });
        }
        if let Some(p) = &self.floats_path {
            let _ = scraper::save_floats(p, &self.floats);
        }
        scraper::fill_floats(&mut self.all, &self.floats);
        scraper::fill_floats(&mut self.snapshot, &self.floats);
        self.engine.set_floats(&self.floats);
        self.reapply_local();
    }

    /// The symbol the chart shows: the full-screen chart's, else the selected row's.
    pub fn charted_symbol(&self) -> Option<&str> {
        match self.view {
            View::Chart => Some(self.chart_symbol.as_str()),
            _ => self.tickers.get(self.row).map(|t| t.symbol.as_str()),
        }
    }

    /// Notes when the charted symbol changed, for the fetch dwell.
    pub(super) fn track_charted(&mut self, now: Instant) {
        let charted = self.charted_symbol().map(str::to_string);
        if charted != self.charted_since.as_ref().map(|(s, _)| s.clone()) {
            self.charted_since = charted.map(|s| (s, now));
            self.reset_chart_view();
        }
    }

    /// 1m backfill, each symbol once per run (a failed job is asked again after
    /// `HISTORY_RETRY`). A ticker with saved candles gets the minutes between its newest saved
    /// candle and its first live one (the time the app was closed): the charted ticker first,
    /// then the list top to bottom, one job at a time. Any other alerted, charted (after
    /// `BACKFILL_DWELL`) or listed ticker gets the minutes before the first candle we hold.
    pub(super) fn queue_backfills(&mut self, now: Instant) {
        let Some(tx) = self.backfill_tx.clone() else { return };
        let charted = self
            .charted_since
            .as_ref()
            .filter(|(_, since)| now.saturating_duration_since(*since) >= BACKFILL_DWELL)
            .map(|(s, _)| s.clone());
        let list: Vec<String> = self.tickers.iter().map(|t| t.symbol.clone()).collect();
        let waiting = |app: &App, sym: &str| {
            app.backfill_asked.contains(sym)
                || app.backfill_failed.get(sym).is_some_and(|&t| now.saturating_duration_since(t) < HISTORY_RETRY)
        };
        if self.gap_pending.is_none() {
            for sym in charted.iter().chain(&list) {
                let Some(&saved) = self.saved_newest.get(sym) else { continue };
                let retry_wait = self.backfill_failed.get(sym).is_some_and(|&t| now.saturating_duration_since(t) < HISTORY_RETRY);
                if self.gap_done.contains(sym) || retry_wait {
                    continue;
                }
                let Some(id) = self.ticker_id(sym) else { continue };
                // the first live candle; none yet: wait for it
                let Some(first) = self.engine.candles(sym).and_then(|c| c.iter().find(|c| c.start > saved)).map(|c| c.start) else {
                    continue;
                };
                let Some(from) = backfill::gap_from(saved, first) else {
                    self.gap_done.insert(sym.clone());
                    continue;
                };
                if tx.send(backfill::Job { symbol: sym.clone(), ticker_id: id, from, before: first }).is_ok() {
                    self.gap_done.insert(sym.clone());
                    self.gap_pending = Some(sym.clone());
                }
                break;
            }
        }
        let mut want: Vec<String> = self.engine.alerts().iter().map(|a| a.symbol.clone()).collect();
        want.extend(charted);
        want.extend(list);
        let minute = chrono::Utc::now().timestamp().div_euclid(60) * 60;
        for sym in want {
            if self.saved_newest.contains_key(&sym) || waiting(self, &sym) {
                continue;
            }
            let Some(id) = self.ticker_id(&sym) else { continue };
            let before = self.engine.first_candle_start(&sym).unwrap_or(minute);
            if tx.send(backfill::Job::recent(sym.clone(), id, before)).is_ok() {
                self.backfill_asked.insert(sym);
            }
        }
    }

    /// Puts fetched 1m bars among the symbol's candles. Those at or before the newest stored
    /// candle are written now (`store_completed` only writes newer ones); `store::load` puts
    /// the file back in order.
    pub(super) fn insert_candles(&mut self, sym: &str, bars: &[Candle]) {
        let added = self.engine.insert_candles(sym, bars);
        let Some(upto) = self.engine.stored_upto(sym) else { return };
        let old: Vec<Candle> = added.into_iter().filter(|c| c.start <= upto).collect();
        if let (Some(store), false) = (self.candle_store.as_mut(), old.is_empty()) {
            let _ = store.insert(sym, &old);
        }
    }

    /// Turns on candle storage in `dir`: deletes earlier days' candle files, reloads today's
    /// candles into the engine, and from then on appends each candle once its minute ends.
    /// Storage problems never stop the screener; the missing history is simply not there.
    pub fn attach_candle_store(&mut self, dir: &Path) {
        let candles = dir.join(store::DIR);
        let _ = store::migrate_day_files(dir, &candles);
        self.candle_store = Some(CandleStore::new(&candles));
        self.candle_dir = Some(candles);
    }

    /// Reads a ticker's saved candles the first time it is on the list this run, in front of
    /// whatever was collected since, and marks them as stored.
    pub(super) fn load_saved_candles(&mut self) {
        let Some(dir) = self.candle_dir.clone() else { return };
        let fresh: Vec<String> = self.all.iter().map(|t| t.symbol.clone()).filter(|s| !self.candles_loaded.contains(s)).collect();
        for sym in fresh {
            self.candles_loaded.insert(sym.clone());
            let Ok(bars) = store::load(&dir, &sym, MAX_CANDLES) else { continue };
            if let Some(last) = bars.last() {
                self.saved_newest.insert(sym.clone(), last.start);
                self.engine.insert_candles(&sym, &bars);
                let rows: Vec<(String, Candle)> = bars.into_iter().map(|c| (sym.clone(), c)).collect();
                self.engine.mark_stored(&rows);
            }
        }
    }

    /// Appends candles whose minute has ended. Failed writes are retried with the next batch.
    pub(super) fn store_completed(&mut self) {
        let Some(store) = self.candle_store.as_mut() else { return };
        let rows = self.engine.completed_candles(chrono::Utc::now().timestamp());
        if !rows.is_empty() && store.append(&rows).is_ok() {
            self.engine.mark_stored(&rows);
        }
    }
}
