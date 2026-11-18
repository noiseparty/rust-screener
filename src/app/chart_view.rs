//! Chart view (time zoom, pan, price scale), chart history requests and mouse handling.

use super::*;

/// A mouse drag on the chart: moving the plot (from where it was pressed) or scaling the price axis.
#[derive(Debug, Clone, Copy)]
pub(super) enum Drag {
    Plot { x: u16, y: u16, pan: i64, shift: f64 },
    Axis { y: u16, zoom: f64 },
}

/// Screen areas the mouse can act on, recorded by the last draw.
#[derive(Default)]
pub struct Hits {
    /// Gainers data rows and the index of the first row shown in them.
    pub gainer_rows: Rect,
    pub gainer_offset: usize,
    /// Gainers header cells and their position in `visible_cols`.
    pub gainer_header: Vec<(Rect, usize)>,
    /// Alerts data rows and the symbol of each, top to bottom.
    pub alert_rows: Rect,
    pub alert_syms: Vec<String>,
    /// Header layout tabs.
    pub layout_tabs: Vec<(Rect, Layout)>,
    pub tf_tabs: Vec<(Rect, Tf)>,
    /// Footer key hints.
    pub keys: Vec<(Rect, KeyCode)>,
    /// Chart plot (drag to scroll back) and price axis (drag to scale), and how many bars fit.
    pub chart_plot: Rect,
    pub chart_axis: Rect,
    pub chart_fit: usize,
    /// Width of the chart area in cells (to recompute the fit when the zoom changes).
    pub chart_cols: u16,
    /// Config sidebar rows and their index in `SETUP_ITEMS`.
    pub setup_items: Vec<(Rect, usize)>,
}

impl App {
    pub fn set_timeframe(&mut self, tf: Tf) {
        if tf != self.timeframe {
            self.timeframe = tf;
            self.reset_chart_view();
        }
    }

    pub(super) fn over_chart(&self, p: Position) -> bool {
        self.hits.chart_plot.contains(p) || self.hits.chart_axis.contains(p)
    }

    /// Time zoom level; the scroll stays within what is held.
    pub(super) fn set_scale(&mut self, scale: usize) {
        self.scale = scale.min(crate::chart::ZOOMS.len() - 1);
        // bars that fit at the new zoom, before the scroll is clamped to them
        if self.hits.chart_cols > 0 {
            self.hits.chart_fit = crate::chart::fit(self.hits.chart_cols, self.scale);
        }
        let (lo, hi) = self.pan_range();
        self.pan = self.pan.clamp(lo, hi);
    }

    /// How far the chart moves: forward until half the plot is empty space after the newest
    /// bar, back until a quarter of the plot still shows bars (older ones are fetched near the
    /// oldest, which moves this further back).
    pub(super) fn pan_range(&self) -> (i64, i64) {
        let (len, _, _) = self.charted_symbol().map(|s| self.held_bars(s)).unwrap_or_default();
        let fit = self.hits.chart_fit as i64;
        // whole merged candles, so the grouping does not shift at the limits
        let merge = crate::chart::ZOOMS[self.scale].1 as i64;
        (-(fit / 2 / merge * merge), ((len as i64 - fit / 4).max(0)) / merge * merge)
    }

    pub(super) fn reset_chart_view(&mut self) {
        self.pan = 0;
        self.zoom = 1.0;
        self.shift = 0.0;
        self.drag = None;
    }

    /// Bars held for `sym` in the current timeframe.
    pub(super) fn held_bars(&self, sym: &str) -> (usize, Option<i64>, Option<i64>) {
        match self.timeframe {
            Tf::M1 => {
                let c = self.engine.candles(sym);
                (c.map_or(0, |c| c.len()), c.and_then(|c| c.front()).map(|c| c.start), c.and_then(|c| c.back()).map(|c| c.start))
            }
            tf => {
                let b = self.bars.get(&(sym.to_string(), tf));
                (b.map_or(0, |b| b.len()), b.and_then(|b| b.first()).map(|c| c.start), b.and_then(|b| b.last()).map(|c| c.start))
            }
        }
    }

    pub fn attach_history(&mut self, tx: Sender<history::Req>) {
        self.history_tx = Some(tx);
    }

    /// Whether bars for `sym` in the current timeframe are being fetched.
    pub fn bars_loading(&self, sym: &str) -> bool {
        self.bars_pending.contains(&(sym.to_string(), self.timeframe))
    }

    /// Ticker id of `sym`: from the list, else from any list fetched this run.
    pub(super) fn ticker_id(&self, sym: &str) -> Option<u64> {
        self.all.iter().find(|t| t.symbol == sym).map(|t| t.ticker_id).filter(|&i| i != 0).or_else(|| self.ids.get(sym).copied())
    }

    /// The 5m/1h/4h/D bar cache folder.
    pub fn attach_bar_cache(&mut self, dir: &Path) {
        self.bars_dir = Some(dir.to_path_buf());
    }

    /// Reads the cached bars of `key` the first time they are needed this run.
    pub(super) fn load_cached_bars(&mut self, key: &(String, Tf)) {
        if key.1 == Tf::M1 || !self.bars_cached.insert(key.clone()) {
            return;
        }
        let Some(dir) = &self.bars_dir else { return };
        let bars = barcache::load(dir, key.1, &key.0);
        if !bars.is_empty() && !self.bars.contains_key(key) {
            self.bars.insert(key.clone(), bars);
        }
    }

    pub(super) fn send_history(
        &mut self,
        tx: &Sender<history::Req>,
        symbol: String,
        ticker_id: u64,
        tf: Tf,
        mode: history::Mode,
        now: Instant,
    ) {
        let key = (symbol, tf);
        if tx.send(history::Req { symbol: key.0.clone(), ticker_id, tf, mode }).is_ok() {
            self.bars_asked_at.insert(key.clone(), now);
            self.bars_pending.insert(key);
        }
    }

    /// The request the charted ticker needs now, if any: its chart in the current timeframe
    /// (once selected for `BACKFILL_DWELL`), older bars when scrolled back near the oldest one,
    /// or its newest bars every `HISTORY_REFRESH`.
    pub(super) fn charted_request(&self, now: Instant) -> Option<(String, u64, Tf, history::Mode)> {
        let (sym, since) = self.charted_since.as_ref()?;
        if now.saturating_duration_since(*since) < BACKFILL_DWELL {
            return None;
        }
        let tf = self.timeframe;
        let key = (sym.clone(), tf);
        if self.bars_pending.contains(&key) {
            return None;
        }
        let id = self.ticker_id(sym)?;
        let asked_ago = self.bars_asked_at.get(&key).map(|&t| now.saturating_duration_since(t));
        let (len, first, last) = self.held_bars(sym);
        let cap = if tf == Tf::M1 { MAX_CANDLES } else { history::MAX_HELD };
        let near_oldest = self.pan + (self.hits.chart_fit + OLDER_MARGIN) as i64 >= len as i64;
        let mode = match (first, last) {
            // a cached chart not refreshed yet this run: its newest bars first
            (Some(_), Some(last)) if tf != Tf::M1 && !self.bars_topped.contains(&key) && asked_ago.is_none_or(|a| a >= HISTORY_RETRY) => {
                history::Mode::Since(last)
            }
            (Some(first), _) if near_oldest && len < cap && !self.bars_exhausted.contains(&key) => history::Mode::Before(first),
            // 1m candles are the app's own; only older ones are fetched
            _ if tf == Tf::M1 => return None,
            (None, _) if asked_ago.is_none_or(|a| a >= HISTORY_REFRESH) => history::Mode::Full,
            (Some(_), Some(last)) if asked_ago.is_none_or(|a| a >= HISTORY_REFRESH) => history::Mode::Since(last),
            _ => return None,
        };
        Some((sym.clone(), id, tf, mode))
    }

    /// Chart history: the charted ticker first, then every timeframe of every ticker on the
    /// filtered list (top to bottom) in the remaining worker slots. A chart with cached bars is
    /// topped up (`Since` its newest bar) instead of fetched in full.
    pub(super) fn dispatch_history(&mut self, now: Instant) {
        let Some(tx) = self.history_tx.clone() else { return };
        if let Some((sym, _)) = self.charted_since.clone() {
            self.load_cached_bars(&(sym, self.timeframe));
        }
        if self.bars_pending.len() < HISTORY_WORKERS {
            if let Some((sym, id, tf, mode)) = self.charted_request(now) {
                self.send_history(&tx, sym, id, tf, mode, now);
            }
        }
        let mut wanted = Vec::new();
        'list: for i in 0..self.tickers.len() {
            let (sym, id) = (self.tickers[i].symbol.clone(), self.tickers[i].ticker_id);
            for tf in PRELOAD_TFS {
                if self.bars_pending.len() + wanted.len() >= PRELOAD_IN_FLIGHT {
                    break 'list;
                }
                let key = (sym.clone(), tf);
                if id == 0 || self.bars_pending.contains(&key) {
                    continue;
                }
                self.load_cached_bars(&key);
                let asked = self.bars_asked_at.get(&key);
                let mode = match (self.bars.get(&key).and_then(|b| b.last()), asked) {
                    (None, a) if a.is_none_or(|&a| now.saturating_duration_since(a) >= HISTORY_RETRY) => history::Mode::Full,
                    // cached bars not topped up yet this run (a failed top-up is asked again)
                    (Some(last), a)
                        if !self.bars_topped.contains(&key) && a.is_none_or(|&a| now.saturating_duration_since(a) >= HISTORY_RETRY) =>
                    {
                        history::Mode::Since(last.start)
                    }
                    _ => continue,
                };
                wanted.push((key.0, id, tf, mode));
            }
        }
        for (sym, id, tf, mode) in wanted {
            self.send_history(&tx, sym, id, tf, mode, now);
        }
    }

    pub(super) fn on_bars(&mut self, symbol: String, tf: Tf, mode: history::Mode, done: bool, failed: bool, bars: Vec<Candle>) {
        let key = (symbol, tf);
        if done {
            self.bars_pending.remove(&key);
            // the next refresh counts from the answer, not from the (slow) request
            self.bars_asked_at.insert(key.clone(), Instant::now());
            // only a clean empty answer means there is nothing older; a failed one is asked again
            if matches!(mode, history::Mode::Before(_)) && bars.is_empty() && !failed {
                self.bars_exhausted.insert(key.clone());
            }
            if matches!(mode, history::Mode::Since(_) | history::Mode::Full) && !failed {
                self.bars_topped.insert(key.clone());
            }
        }
        match (tf, mode) {
            (Tf::M1, history::Mode::Before(_)) => self.insert_candles(&key.0, &bars),
            (Tf::M1, _) => {}
            // an empty answer leaves nothing, so it is tried again after HISTORY_RETRY
            (_, history::Mode::Full) if !bars.is_empty() => {
                self.bars.insert(key.clone(), bars);
            }
            (_, history::Mode::Full) => {}
            // a top-up that did not reach the held bars (an old cache) replaces them, so no hole
            // is left; a failed one is dropped and the next refresh asks again
            (_, history::Mode::Since(s)) if bars.first().is_some_and(|b| b.start > s) => {
                if !failed {
                    self.bars.insert(key.clone(), bars);
                }
            }
            _ => {
                if let Some(held) = self.bars.get_mut(&key) {
                    history::merge(held, &bars);
                }
            }
        }
        if done && tf != Tf::M1 {
            if let (Some(dir), Some(held)) = (&self.bars_dir, self.bars.get(&key)) {
                let _ = barcache::save(dir, tf, &key.0, held);
            }
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.view, View::Dashboard | View::Chart) && !self.in_setup() {
            return;
        }
        let p = Position::new(m.column, m.row);
        match m.kind {
            // the wheel zooms the chart in time, like TradingView: up = closer, down = further out
            // only over the chart; elsewhere the wheel does nothing
            MouseEventKind::ScrollUp if self.over_chart(p) => self.set_scale(self.scale.saturating_sub(1)),
            MouseEventKind::ScrollDown if self.over_chart(p) => self.set_scale(self.scale + 1),
            // over the alerts: scroll them (the draw keeps the last page in view)
            MouseEventKind::ScrollUp if self.hits.alert_rows.contains(p) => self.alert_scroll = self.alert_scroll.saturating_sub(1),
            MouseEventKind::ScrollDown if self.hits.alert_rows.contains(p) => self.alert_scroll += 1,
            MouseEventKind::Down(MouseButton::Left) => self.click(p),
            MouseEventKind::Drag(MouseButton::Left) => self.drag_to(p),
            MouseEventKind::Up(MouseButton::Left) => self.drag = None,
            _ => {}
        }
    }

    /// Plot drag moves the chart with the pointer: right shows older bars (to the nearest bar),
    /// down shows higher prices. Axis drag: down widens the price range, up narrows it.
    pub(super) fn drag_to(&mut self, p: Position) {
        match self.drag {
            Some(Drag::Plot { x, y, pan, shift }) => {
                let (step, merge) = crate::chart::ZOOMS[self.scale];
                let bars = (f64::from(i32::from(p.x) - i32::from(x)) * merge as f64 / f64::from(step)).round() as i64;
                let (lo, hi) = self.pan_range();
                self.pan = (pan + bars).clamp(lo, hi);
                // the price plot spans the plot area less its header, time axis and volume rows
                let span = (f64::from(self.hits.chart_plot.height) - 6.0).max(1.0);
                self.shift = shift + f64::from(i32::from(p.y) - i32::from(y)) / span;
            }
            Some(Drag::Axis { y, zoom }) => {
                let rows = i32::from(p.y) - i32::from(y);
                self.zoom = (zoom * 1.06f64.powi(rows)).clamp(0.05, 20.0);
            }
            None => {}
        }
    }

    pub(super) fn click(&mut self, p: Position) {
        if let Some(&(_, l)) = self.hits.layout_tabs.iter().find(|(r, _)| r.contains(p)) {
            self.set_layout(l);
            return;
        }
        if let Some(&(_, tf)) = self.hits.tf_tabs.iter().find(|(r, _)| r.contains(p)) {
            self.set_timeframe(tf);
            return;
        }
        if self.hits.chart_plot.contains(p) {
            self.drag = Some(Drag::Plot { x: p.x, y: p.y, pan: self.pan, shift: self.shift });
            return;
        }
        if self.hits.chart_axis.contains(p) {
            // a double click on the price axis fits the range again
            let double = self.last_axis_click.is_some_and(|t| t.elapsed() < Duration::from_millis(400));
            self.last_axis_click = Some(Instant::now());
            if double {
                self.zoom = 1.0;
                self.shift = 0.0;
            }
            self.drag = Some(Drag::Axis { y: p.y, zoom: self.zoom });
            return;
        }
        if let Some(&(_, i)) = self.hits.setup_items.iter().find(|(r, _)| r.contains(p)) {
            // another item is shown with the sidebar focused; the one shown is opened
            if i == self.setup_idx {
                self.view = SETUP_VIEWS[i];
            } else {
                self.select_setup(i);
                self.view = View::SetupMenu;
            }
            return;
        }
        if let Some(&(_, code)) = self.hits.keys.iter().find(|(r, _)| r.contains(p)) {
            self.on_key(KeyEvent::new(code, KeyModifiers::NONE));
            return;
        }
        if self.view != View::Dashboard {
            return;
        }
        if let Some(&(_, vi)) = self.hits.gainer_header.iter().find(|(r, _)| r.contains(p)) {
            self.sel_col = vi;
            self.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        } else if self.hits.gainer_rows.contains(p) {
            let i = self.hits.gainer_offset + (p.y - self.hits.gainer_rows.y) as usize;
            if i < self.tickers.len() {
                self.move_row(i);
            }
        } else if self.hits.alert_rows.contains(p) {
            let i = (p.y - self.hits.alert_rows.y) as usize;
            if let Some(sym) = self.hits.alert_syms.get(i).cloned() {
                match self.tickers.iter().position(|t| t.symbol == sym) {
                    Some(row) => self.move_row(row),
                    // not on the list any more (filtered out, blacklisted): its full chart
                    None => {
                        self.chart_symbol = sym;
                        self.view = View::Chart;
                    }
                }
            }
        }
    }
}
