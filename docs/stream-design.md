# Volume Screener: moving from REST polling to the WSS stream (design)

Scope: design only. No files were edited. Where the analyst and the skeptic disagreed, this uses the skeptic's revised version, and section 3 resolves the remaining conflicts between features. Line numbers refer to the current `src/`.

---

## 0. Live checks before writing any code (blocking)

Write a small probe binary that subscribes to 3–5 gainers and logs the raw protobuf fields. Run it across 04:00–04:10, 09:25–09:35, 15:55–16:05 and one after-hours window (all ET). It must answer:

1. **Extended-hours price:** is `sale.pPrice` (or `pChRatio`) filled in Pre/After, or does `sale.close` move? If neither does, the stream drives Reg only, and Pre/After stays on REST polling (see 3.6).
2. **`stats.volume`:** does it advance in Pre/After? Does it reset or jump at 09:30 or 16:00? Is it a plain integer string?
3. **`stats.high` / `stats.low`:** do they cover regular hours only, or extended hours too? When do they reset?
4. **`status` codes** in Pre/After (is it still "T"?).
5. **`trdSeq`:** is it per ticker, is it monotonic, does it reset each day or on reconnect? This is informational only; nothing in the design depends on it.
6. **Subscription cap:** the real limit, assumed to be 100.

---

## 1. Shared data model

### 1.1 Threads and messages
- A new **`src/stream.rs`** thread runs MQTT over WSS and decodes the protobuf. It sends `Msg::Quotes(Vec<Update>)` on the existing mpsc channel at most every **250 ms**, in arrival order. App owns all state, so no `Arc<Mutex>` is needed (same pattern as `Msg::Fetched`).
- `Update { id: u64, trade_ms: i64, seq: u64, px: Option<f64>, change_ratio: Option<f64>, volume: Option<u64>, mcap: Option<f64>, day_hi: f64, day_lo: f64, status: String, recv: Instant }`
- REST polls the **unfiltered** gainers list every `LIST_REFRESH` = 20 s while the stream is healthy. It supplies list membership, the symbol↔tickerId map, mcap and `prev_close`.

### 1.2 Per-ticker state (keyed by tickerId)
```
Quote  { symbol, price, prev_close, volume, mcap, last_ms, last_seq, updated: Instant, from_stream: bool }
Series { pts: VecDeque<(Instant, u64 vol, f64 px)>, lower_count: u8 }
```
- **One time axis:** the local receive `Instant` is used for every window. `tradeStamp` is used only for ordering, for the session/date of a message, and for the alert Time column.
- **`change_ratio`** is always computed locally as `price / prev_close - 1`, with `prev_close = close / (1 + changeRatio)` taken from the first REST row. REST and WS then can never disagree on change %.

### 1.3 Ingest rules (per Update, O(1))
1. **Order:** accept when `trade_ms > last_ms`, or when `trade_ms == last_ms && seq > last_seq`. Drop anything else. Never order on `trdSeq` alone.
2. **Missing data:** if volume is missing or unparseable, drop the message. It is never read as 0; this fixes the `num()` → 0 behaviour in `scraper.rs` for this path.
3. **Volume going down** (`vol < back.vol`): count it in `lower_count` and drop the message. On 3 lower readings in a row, clear that ticker's series and start a new one from this sample. Never reset on a single message, and never clamp (clamping freezes the deltas).
4. **No change:** if `vol == back.vol` and price is unchanged, skip the message.
5. **Otherwise:** set `lower_count = 0`. If `recv - back.t < 1 s`, overwrite the last point; otherwise push a new one.
6. **Prune:** drop old points but keep exactly one anchor at or before `now - HORIZON`, where `HORIZON = spike_short + spike_base + 60s`. The rule is to pop the front while `pts[1].t <= now - HORIZON`.
7. **Lookups:** `vol_at(t)` and `px_at(t)` return the last point with time ≤ t (`partition_point`). They return `None` if t is earlier than the first point.

### 1.4 When series are cleared
| Event | What is cleared |
|---|---|
| Session change (`Session::at(tradeStamp)`), or the New York date changes | All series, rank ring, filter-grace state, badges, `last_ms` and `last_seq` |
| WSS reconnect | All series. Coverage then restarts by itself, so no `live_since` and no reconnect gate are needed. |
| A ticker switches source (REST ↔ WS) | That ticker's series |
| Linger expires (60 s after the ticker leaves the REST list) | Unsubscribe the ticker; remove its Quote, Series and badge |

The first point of each session is a **base only**, so opening and closing auction prints never fall inside a delta.

### 1.5 What is recorded regardless of filters and blacklist
Series and quotes are updated for **every subscribed id**, whatever the display filters or blacklist say. This fixes the gaps caused by the filter check running first (`engine.rs:211-213`) and the stale `hist.first()` fallback (`engine.rs:311-324`).

- **Blacklisted ids** stay subscribed and keep their data. They are excluded from alerts, board rank and the high/low counts.
- **Display filters** only decide which rows are shown.

---

## 2. Per feature

### 2.1 Volume delta becomes Vol/m
**Problem now:** the delta is history that is gated by the filters and keeps stale anchors. A ticker that comes back after 20 minutes shows a delta covering 20 minutes under a "Δ180s" header. A new ticker shows a "0" that looks real. Negative deltas appear at volume resets.

**New calculation** (W = `delta_window_seconds`):
```
start   = max(now - W, first.t)
covered = now - start
vol_w   = vol_at(now) - vol_at(start)      // >= 0 by the ingest rules
rate    = vol_w * 60 / covered_secs        // shares per minute
rate    = None if covered < min(W, 30s) OR no WS message on the connection for > 10s (stale)
partial = covered < W
```
- The window always ends at **`now`**, so a quiet ticker's rate decays toward 0.
- `Ticker.vol_delta: i64` is replaced by `vol_rate: Option<f64>` and `vol_partial: bool`.
- Only **full-coverage** rates may feed SPIKE or relative-volume colouring.

**Config:** keep `delta_window_seconds` at the user's value and clamp it to at least 30. No new keys.

### 2.2 SPIKE: a rate surge, not growth of day volume
**Problem now:** SPIKE is day cumulative volume divided by the cumulative volume at the oldest sample (`engine.rs:237-238`). That measures growth of the day total. It fires at 04:00 on thin volume and almost never after 10:00: 30M + a 3M burst = 1.1x.

**New calculation** (S = `spike_short_sec` 60, B = `spike_base_sec` 600):
```
short      = vol_at(now) - vol_at(now - S)
base_start = max(now - S - B, first.t);   require (now - S - base_start) >= 120s, otherwise no SPIKE
base_rate  = (vol_at(now - S) - vol_at(base_start)) / (now - S - base_start)
ratio      = (short / S) / max(base_rate, 1.0)
fire if ratio >= spike_rate_ratio (3.0) && short >= spike_min_shares (20_000)
        && day_vol >= min_volume_for_alert && px >= min_price_for_alert
```
The baseline window ends where the short window starts, so a surge cannot dilute its own baseline. The day-average fallback is **dropped**: a ticker without enough history waits for S + 120 s of coverage, and NEW covers tickers that just joined.

The alert detail reads `vol 4.2x 85k/m`.

### 2.3 PUMP: run-up from a robust low
**Problem now:** PUMP is a return measured from a single price sample (`engine.rs:239-240`). A single odd-lot print at either end can trigger or hide it. A V-shaped move is measured from the price before the dip, not from the low.

**New calculation** (W = `pump_window_sec` 120; at least 30 s of coverage required):
```
low = 2nd-lowest px among points in [now - W, now] ∪ {px_at(now - W)}   // ignores one stray print
pct = (px - low) / low
fire if pct >= pump_price_pct (0.05) on 2 consecutive accepted messages, plus the eligibility gates
```
There is no "px ≥ previous px" check; the 2-message confirmation covers it.

The alert detail reads `+6.3% 1m40`, where the time is now minus the time of the low.

### 2.4 Debounce and re-arm (per symbol and kind)
**Problem now:** an alert fires again on every poll while its condition holds. At stream rate that becomes hundreds per minute.

```
Trig { armed, pending, last_fire: Instant, fire_val }
```
- On each message, check re-arm **before** checking fire.
- **Re-arm** only when `now - last_fire >= alert_cooldown_sec` (120) **and** the metric has fallen below half its threshold.
- **Fire** only while armed. Firing sets `armed = false`.
- **PUMP escalation:** while disarmed, PUMP fires again when `px >= fire_val * (1 + pump_price_pct)`. An escalation still needs the 2-message confirmation.
- SPIKE has no escalation. During a sustained surge its baseline catches up, so it re-arms on its own.

### 2.5 SETUP
**Problem now:** SETUP needs SPIKE and PUMP on the same poll. Volume usually leads price by tens of seconds, so real setups rarely show as SETUP.

**New rule:** when SPIKE or PUMP fires and the other kind fired within `spike_short_sec`, log **SETUP instead of** the second alert. That gives one row per event and needs no extra Trig. The detail reads `4.2x +6.3%`.

### 2.6 NEW
**Problem now:** NEW means "first time this ticker passed the filter". `self.last` is never pruned, so NEW fires once per process lifetime. Startup produces 25+ NEW alerts at once, and changing a filter floods it again.

**New rule:**
- Keep `last_listed: HashMap<String, Instant>` over the REST list.
- NEW fires when a symbol is listed now and was not listed in the past 1800 s (hardcoded).
- The first list after startup, after a session change, or after a filter change only seeds `last_listed`; it raises no alerts.
- Prune `last_listed` entries older than 40 minutes.

### 2.7 General alert fixes
- **Blacklisted symbols** raise no alerts. Today they reach the alert push at `engine.rs:250-263`.
- **New `Alert` struct:** `Alert { at: Instant, ts_ms, symbol, kind, metric, rate, span_ms, price, change_ratio }`. This drops `prev_vol` / `prev_price` and their -1 sentinels.
- **Time column** uses `ts_ms` converted to local HH:MM:SS.
- **Status badge:** per symbol, `(kind, until = fire + 60s)`. Priority is SETUP > SPIKE/PUMP > NEW.
- **Flash bug:** `app.rs:207` and `app.rs:211` read two separate `Instant::now()` values, so `last_alert_at == a.at` (`ui.rs:400`) is never true. Fix: pass one `now` and use it for both. Flash only when the alert's kind passes `alert_visible`.

**Alert config:**

| Change | Key | Default |
|---|---|---|
| Replaces `spike_vol_ratio` | `spike_rate_ratio` | 3.0 |
| New | `spike_short_sec` | 60 |
| New | `spike_base_sec` | 600 |
| New | `spike_min_shares` | 20 000 |
| Replaces `baseline_window_sec` | `pump_window_sec` | 120 |
| New | `alert_cooldown_sec` | 120 |

`validate()`:
- zero values fall back to the defaults;
- `spike_rate_ratio > 1`;
- `spike_base_sec >= 2 * spike_short_sec`.

These are **constants, not config:**

| Constant | Value |
|---|---|
| SETUP window | `spike_short_sec` |
| NEW re-entry | 1800 s |
| Badge hold | 60 s |
| Rate floor | 1 share/s |
| Confirmation | 2 messages |

### 2.8 Rate bars become session new highs / new lows
**Problem now:** a "tick" is the net direction between two 5 s polls, so the counts scale with list length and poll interval. Stale `last` values cause fake ticks. Blacklisted tickers are counted. The price source switches at session boundaries, causing bursts. All four bars share one max, so the 30 s and 1 m rows are invisible.

**New state:**
```
hl_session: Session
hl: HashMap<u64, HL { hi, lo, armed_at: Instant, last_hi_event: Instant }>
hl_events: VecDeque<(Instant, bool is_high)>
```

**`on_quote`**, using `px` per the live checks (pPrice in Pre/After if filled, close in Reg; skip if px ≤ 0 or the session is Closed):
- Blacklisted: return.
- Session changed: clear `hl`. The event deque is not cleared, so windows decay naturally.
- **No entry yet:** seed `hi = max(px, day_hi if Reg and > 0)` and `lo = min(px, day_lo if Reg and > 0)`, with `armed_at = now + 60s`. No event.
- **Otherwise:** compute `h = max(px, day_hi if Reg)` and `l = min(px, day_lo if Reg)`.
  - If `h > hi`: set `hi = h`.
  - Else if `l < lo`: set `lo = l`.
  - Push an event only when `now >= armed_at` **and** at least 1 s has passed since this ticker's last event.
- No trdSeq guard and no bust guard: the bust guard would create fake highs when messages arrive out of order.

**Reading the bars:**
- Count events per window (20 m / 5 m / 1 m / 30 s) with `partition_point`; age = W is excluded.
- Prune the deque at 1200 s.
- Bar length = `(count / W) / max over all rows and both sides`, so the units are comparable and short windows show acceleration.

**Remove:** `ticks` and the tick push (`engine.rs:231-235`). No new config keys.

### 2.9 Ranking, sorting, filters and membership
**Problems now:**
- Sorting by 'mov' reads `rank_change = 0`, because `track_ranks` runs after the sort (`engine.rs:275-281, 329`).
- Mov is keyed by the user's sort column and measured after filtering and truncation.
- The rank fallback can be up to 300 s old.
- Selection is a row index, so the cursor lands on a different ticker after a re-sort.
- `compare()` has no tiebreak.
- Pressing 's' or saving config triggers a fetch.
- The maps are never evicted.

**New: one fixed 1 s recompute tick** (re-sorting is never tied to the 80 ms frame or to each message):
- **(a) Universe:** members ∪ lingering ids. Board order = `change_ratio desc → volume desc → symbol asc`. Push `(now, Vec<id>)` onto `rank_ring` and keep `rank_interval + 5s` of it.
- **(b) Mov:** take the newest ring entry with time ≤ `now - rank_interval`. Remove ids that are blacklisted now from both the old and the new list, then rank both.
  - No old entry yet: blank.
  - Id not in the old list: cyan `+`.
  - Otherwise: `old_rank - new_rank`.
  - Mov ignores display filters and the sort column, so changing either never creates fake movement.
- **(c) Filter grace:** a row that starts failing a filter stays visible, dimmed, for 10 s and is then hidden. Saving config clears the grace state.
- **(d) Rows:** universe minus blacklist minus hidden. Sort with `compare(key).then(symbol)`; for 'mov', blank and `+` go last. Then truncate to `MAX_ROWS`.
- **(e) Linger expiry:** unsubscribe and evict the ticker.

**Subscriptions:** want = members first, then lingering ids, up to the verified cap. Subscribe to want minus subscribed. Unsubscribe only tickers whose linger has expired.

**Stream health:** `stream_ok = connected && last_msg_at < 15 s ago`, measured over all messages. When it is not ok in Pre/Reg/After:
- poll REST every `refresh_interval_seconds`, with REST allowed to write values;
- reconnect with backoff 1 → 30 s and resubscribe the full set;
- in the Closed session, make no reconnect attempts.

**Selection:** stored as `selected_id`; it follows the symbol after every recompute.

**Keys:**
- 's' and saving config re-run step (d) locally, with no fetch.
- 'r' forces a REST list fetch.

**Config:**
- `refresh_interval_seconds` now means the **REST fallback poll** (default 5).
- Add `stream_enabled: bool = true`, in config.json only.
- `rank_tracking_interval_seconds` has a minimum of 5.

**Constants:**

| Constant | Value |
|---|---|
| RECOMPUTE | 1 s |
| FILTER_GRACE | 10 s |
| LINGER | 60 s |
| STREAM_STALE | 15 s |
| QUOTE_STALE | 60 s |
| LIST_REFRESH | 20 s |

---

## 3. Cross-feature conflicts and how they are resolved

| # | Conflict | Decision |
|---|---|---|
| 1 | Time axis: tradeStamp (alerts analyst) vs receive Instant | **Receive Instant** for all windows. tradeStamp only for ordering, session/date and the Time column. |
| 2 | Message ordering: trdSeq (all analysts) vs none (critiques) | `(tradeStamp, trdSeq)` lexicographic order for quotes. Volume series also need volume to be monotonic. High/low tracking needs no ordering guard. Never trdSeq alone. |
| 3 | Volume decrease: clamp / drop at 0.5x / clear on any decrease / 3 lower readings | **Drop single lower readings; reset after 3 in a row.** Explicit clears on session change, date change, reconnect and source switch. No clamping (it freezes deltas). |
| 4 | Reconnect: keep series + gate (alerts) vs clear all (volume) | **Clear all volume series.** The coverage requirements then act as the gate. High/low state is **kept** (a monotonic max is safe across a gap). |
| 5 | Linger: 10 min (alerts) vs 60 s (volume, ranking) | **60 s.** NEW continuity comes from `last_listed` (40 min), not from keeping the series. |
| 6 | Badge hold: 60 s vs 30 s | **60 s** (the alerts group owns this). |
| 7 | List refresh / `refresh_interval_seconds` meaning | Constant LIST_REFRESH = 20 s while the stream is healthy. `refresh_interval_seconds` = fallback poll. |
| 8 | Config sprawl vs fixed FIELDS indices (`preview_filters` hard-codes 10–15) | Rename in place: index 2 `Spike Vol Ratio` → `Spike Rate Ratio`, index 6 `Baseline Window` → `Pump Window`, index 4 `Refresh Rate` → `Fallback Poll`. **Append** `Spike Min Shares`, `Alert Cooldown (s)`, `Spike Short (s)`, `Spike Base (s)` as indices 16–19, so indices 0–15 do not move. All other tunables are constants. |
| 9 | Partial-coverage rates feeding alerts | They never do. Vol/m shows partial values dimmed. SPIKE uses its own full-window coverage rule. |
| 10 | Rate ticks across a REST/WS source change | Moot: rate bars become high/low tracking fed **only** by stream messages. |
| 11 | change_ratio basis differs between REST and WS in Pre/After | Compute it locally from `prev_close` everywhere: rank, filters, PUMP display. |
| 12 | Pre/After price | Decided by live check 0.1. If the stream has no extended-hours price, Pre/After keeps REST 5 s polling for price, and high/low tracking stays empty in Pre/After rather than showing wrong data. |
| 13 | `$/min` via `dealAmount` | Dropped. Optional sort variant: `vol_rate * price`. |

---

## 4. UI changes

- **Vol/m column:**
  - Header `Vol/m {30s|1m|3m}`, width 8, no `+` sign and no red branch.
  - `None` shows a dim `…`; a stale feed shows `—`. Partial coverage is dimmed. A true 0 is a faint `0`.
  - Optional cell colour by relative volume (≥2x cyan, ≥4x bold cyan, full coverage only).
  - `None` sorts last in both directions.
- **Alerts panel:**
  - Details: `vol 4.2x 85k/m`, `+6.3% 1m40`, `4.2x +6.3%`, `listed +X%`.
  - Time column from tradeStamp.
  - Flash fixed.
  - New filter descriptions at `ui.rs:611-614`.
- **Status column and tape:**
  - Sticky badge (60 s), with priority as in 2.7.
  - Tape arrows coloured by the sign of `change_ratio`, not Mov.
- **Mov column:** blank until there is history, cyan `+` for a ticker that entered, ▲n / ▼n otherwise. Sorting by Mov works.
- **Rows:** dimmed during filter grace, while lingering, or when the quote is older than 60 s. Selection follows the symbol.
- **High/low panel:**
  - Title: `SESSION HIGHS/LOWS · <SESSION> · N subs`.
  - Labels `New Lows` / `New Highs`.
  - Bars use per-second rates; the text shows raw counts.
  - Rows whose window is not yet full are dimmed.
  - Shows `waiting for stream` before any data or when the session is Closed.
- **Header:**
  - `● WSS n subs m msg/s` in green;
  - `● REST 5s, stream down, retry Ns` in amber;
  - errors in red;
  - a list-fetch error while the stream is fine shows as a warning, not an error.
- **Footer gauge:** counts down to the list refresh (WSS mode) or to the next poll (REST mode).
- **Settings:** new labels and descriptions per 3.8. "Delta Window" is described as "Window for the Vol/m column (min 30s)".

---

## 5. Build order (each step can be approved on its own)

1. **Live probe** (separate bin, no app changes). Answers section 0.
2. **REST-path bug fixes, no stream needed:**
   - one `now` for the alert flash;
   - record history before the filter check;
   - no alerts for blacklisted symbols;
   - compute Mov before sorting;
   - tiebreak by symbol;
   - selection by `selected_id`;
   - 's' and saving config re-sort locally with no fetch.
3. **Stream client:** `src/stream.rs`, `Msg::Quotes` with 250 ms batches, subscribe/unsubscribe from the unfiltered list, `stream_enabled`, health state and header indicator. Display only: merge WS price, volume and mcap over the REST row, with local `change_ratio`.
4. **Series store:** ingest rules, the clear rules in 1.4, linger and eviction, plus unit tests for ordering, lower readings, coalescing, pruning and step lookup.
5. **Vol/m column** on the series store, with the new header, cell rendering and sorting.
6. **1 s recompute tick:** board-rank ring, new Mov, filter grace, `LIST_REFRESH` and REST fallback.
7. **SPIKE (rate) and PUMP (robust low):** new config keys, validate, settings rename/append and descriptions.
8. **Debounce, SETUP replacement, sticky badges, new `Alert` struct and detail text, tradeStamp Time column.**
9. **NEW from `last_listed`:** silent seeding and 1800 s re-entry.
10. **High/low bars:** replace ticks, new panel rendering.
11. **Tests across features:**
    - no SPIKE from late-day growth, after a reconnect, or on the 09:30 print;
    - a 3x surge fires once per cooldown;
    - one low outlier gives no PUMP;
    - PUMP escalation;
    - SPIKE then PUMP within 60 s gives a single SETUP row;
    - no NEW flood on startup;
    - NEW again after 30 min;
    - no high/low events while unarmed or on a session change;
    - window boundaries.

Steps 2 and 4 do not depend on each other. Steps 5–10 each depend only on steps 3–4, and step 8 also needs step 7.

Files referenced:
- `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust\src\engine.rs`
- `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust\src\app.rs`
- `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust\src\ui.rs`
- `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust\src\config.rs`
- `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust\src\scraper.rs`