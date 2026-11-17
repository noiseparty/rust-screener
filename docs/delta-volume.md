# Delta volume (research, no code yet)

Delta = buy volume − sell volume. Buy = trades at the ask (buyer initiated), sell = at the bid.
This doc answers whether the app can show it, how accurate each method is, and how to build it.

## 1. What the data gives

**Stream** (`src/stream.rs`, anonymous `type 102, flag 1`): one `ItemsCombo` snapshot per update,
~0.5–1 msg/s per hot ticker (`docs/plan.md`). Used fields:

| Field | Meaning | Use for delta |
|---|---|---|
| base 5 `tradeStamp` | ms time of last trade | ordering |
| sale 1 `close` / 4 `pPrice` | last price (Reg / Pre+After) | price change between updates |
| stats 3 `volume` | cumulative day volume | volume increment between updates |
| stats 2 `lastestPriceVol` | probably last trade size | unverified, see section 5 |

No bid/ask, no individual trades, no trade side. Modules 4 `deal` and 5 `quote` are not delivered
anonymously (plan.md). `decode_items_combo` ignores stats 2; `decode_raw` and the probe keep it
(`last_price_vol` column), but no probe CSV exists yet (`%LOCALAPPDATA%\MarketGUI\probe\` is empty).

Unofficial client docs (tedchou12/webull wiki) describe topic 103 as `deal: {trdBs, volume, tradeTime, price}`
and 105 as 102+103. `trdBs` is Webull's own buy/sell flag per trade. Whether 103/105 deliver anything to
user `test` has not been checked specifically; see section 5.

**History API** (`src/history.rs`, `charts/query`): bars `label,open,close,high,low,prevClose,volume,vwap`.
OHLCV only, no side. Applies to fetched 5m/1h/4h/D bars and to backfilled 1m bars.

**Engine path**: `Engine::apply_quotes` -> `record_sample(sym, epoch, px, cum_vol)` for every accepted
(`Ingest::Stored`) stream update. `record_sample` already computes `traded = cum_vol − prev_cum_vol` per
update; the price before the update is the current candle's `close` (or the previous candle's). So every
input a tick rule needs is already at hand there; nothing in the stream client has to change.

## 2. Is true delta possible?

No. True delta needs every trade with its side, or every trade plus the bid/ask at that moment
(Lee-Ready). The anonymous stream has neither. Everything below is an estimate. Unless 103/105 turn
out to work anonymously, the best available is a tick rule on ~1 s increments.

## 3. Options

### A. Candle colour (the user's example)
Whole bar volume is buy if `close >= open`, sell otherwise. "100k green + 50k red = +50k over 2m".
- Accuracy: lowest. All-or-nothing per bar; a doji with a huge print counts in full on one side;
  a bar that ran up then faded counts as all sell. Known as "up/down volume". Fine as a trend hint
  on 1m bars summed over several minutes, poor on single bars or 4h/D bars.
- Cost: zero; no state, no storage, works on any bar from any source.

### B. Tick rule on stream increments
Per accepted update: `dV = cum_vol − prev`, `dP = px − prev_px`.
`dP > 0` -> dV is buy; `dP < 0` -> sell; `dP == 0` -> same side as the last non-zero tick
(zero-tick rule); no previous tick (first update of a run/session) -> unclassified.
- Accuracy: the tick rule on individual trades is ~75–80 % correct for US equities (Lee-Ready
  ~85–87 %; Carrion, Ko, Lay 2020; Chakrabarty, Pascual, Shkilko 2015). Here one increment can bundle
  several trades over ~1 s and is signed by the net price move, so per-increment accuracy is lower
  (not measured; no ground truth available). Errors partly cancel when summed over minutes, so net
  delta over 1–5 min should track real delta in sign and rough size on active tickers. Weak on thin
  tickers (few updates, long zero-tick runs) and on large prints without a price change
  (block / late / odd-lot reports inherit the last side).
- Pitfalls: opening/closing auction volume (09:30/16:00 jump) lands in one increment. The series
  store already makes the first point of a session a base only; the candle path must do the same
  (do not classify the first increment after a session change, reconnect or app start).
- Cost: two numbers per candle, one `prev_px`/`last_sign` per symbol. Only for candles built live
  from the stream. During REST fallback (5 s polls) the same rule runs on coarser increments.

### C. Bulk volume classification (BVC, Easley, López de Prado, O'Hara 2012)
Per bar: `buy = V · Φ(ΔP / σ)`, `sell = V − buy`, where `ΔP = close − previous close`, `σ` = standard
deviation of `ΔP` over the last N bars (e.g. 50) of the same timeframe, Φ = normal CDF.
- Accuracy: on equities below the trade-level tick rule (Chakrabarty et al. 2015), better than A
  because a small move splits the volume ~50/50 instead of 100/0. A is BVC with σ -> 0.
- Cost: computed at draw time from OHLCV; nothing stored. Needs a normal CDF (std has no `erf`;
  a 6-line Abramowitz–Stegun approximation is enough).

### D. Webull `trdBs` (only if topic 103/105 works anonymously)
Webull's own per-trade side. Closest to real delta the app could get. Every trade is a message, so
message volume rises a lot on hot tickers. Blocked on the probe check in section 5.

### Where each applies

| Bars | A colour | B tick rule | C BVC |
|---|---|---|---|
| 1m candles built live from the stream | yes | **yes (best)** | yes |
| 1m backfilled (gap/history API) and old 6-column CSV rows | yes | no (no stream data) | **yes** |
| fetched 5m/1h/4h/D | yes | only by summing stored 1m B-candles when the bar is fully covered | **yes** |

## 4. Storage

Per-ticker 1m CSV (`store.rs`, `candles\<SYMBOL>.csv`) gets two columns:
`start,open,high,low,close,volume,buy,sell`.
- `buy`/`sell` = tick-rule shares; `volume − buy − sell` = unclassified (first increment).
- Backward compatible on read: `parse_line` accepts 6 or 8 fields; a 6-field row (old files,
  backfilled bars) loads with `buy = sell = 0` and is shown with BVC. Rows may mix 6 and 8 fields.
- Not forward compatible: an older exe rejects 8-field rows (`f.len() != 6`). Acceptable (one user).
- Backfilled bars are written with 6 fields (no tick data); BVC is never stored, so a later better
  method needs no migration.
- `data\bars\<tf>\` (bar cache, round 3) is unchanged: fetched bars use BVC at draw time.

`Candle` gets `buy: i64, sell: i64` (0 = unknown). `chart::merged` sums them; `history::parse_bar`,
`record_sample`, `prepend_candles` and tests set 0 / accumulate.

## 5. Probe step

The probe already records everything needed (`last_price_vol`, `volume`, `close`, `p_price`,
`trade_stamp_ms`, `trd_seq` per message). Run `--probe` for at least 30 min of regular hours, then
check per ticker, over consecutive messages:
1. Is `last_price_vol` filled at all, and in which sessions?
2. `dV = volume[i] − volume[i−1]`. If `last_price_vol[i] <= dV` always, and `== dV` often when
   `trd_seq` advanced by 1, it is the last trade size.
3. Share of increments with `dP == 0` (zero ticks) and median `dV` per increment: tells how coarse
   B is per ticker.
4. If confirmed, use it only as a quality marker: `dV == last_price_vol` means the increment was one
   trade, so B is trade-level tick rule for it. It gives no side by itself.

Separate one-off check for option D: subscribe a few ids with `"type":"103"` and `"type":"105"`
(probe-only flag, `tickers_topic` type parameter) and record SUBACK codes and whether top-level
module 4 appears in `modules`. If it does, decode `deal` fields raw first before designing on it.

The analysis is a short script over the CSV (no app code).

## 6. UI

- **List column** `Δ` (key `delta`, header `Δ 2m`): net delta over the last W minutes from the
  symbol's 1m candles (current partial minute included), signed and compact (`+50k`, `−1.2M`),
  green/red; dim when any minute in the window is BVC-estimated or unclassified share > 50 %;
  blank when no candle covers the window. Sorts like Vol/m (blanks last).
- **Config**: `Δ Window (min)` (1–30, default 2). Do **not** reuse `delta_window_seconds`: it is
  already a serde alias of `vol_m_window_seconds`. New key `net_delta_window_min`.
- Minutes, not seconds: the delta lives on candles, so it survives reconnects (the series store is
  cleared on reconnect) and covers backfilled minutes. 1-minute granularity matches the user's "over 2m".
- **Chart**: the 2-row volume pane can show net delta instead of volume: bar height
  `|buy − sell| / max |buy − sell|`, coloured by sign; per-bar method as for the column
  (B where the candle has buy/sell, else C). Toggled with a free key in the dashboard and chart view
  (`d` is taken in the chart view; e.g. `x`) and remembered in config. TUI path in `chart::draw`,
  pixel path in `gui.rs` (reads `Plot.candles`).

## 7. Recommendation

1. Build B (tick rule) into live 1m candles now: it is the only method using information beyond OHLC,
   and the inputs already exist in `record_sample`.
2. Use C (BVC) for every bar without tick data (backfilled 1m, old rows, 5m/1h/4h/D). Use A only if
   the user explicitly prefers the simpler rule; it is C's worst case.
3. Run the probe (section 5) first or alongside; it does not block 1–2. If 103/105 deliver `deal`
   anonymously, D replaces B later with the same storage columns.
4. Label it as an estimate in the column description ("estimated from price ticks").

### Implementation outline (later round)

| File | Change |
|---|---|
| `src/engine.rs` | `Candle { buy, sell }`; `record_sample` keeps `prev_px`/`last_sign` per symbol, adds `traded` to buy or sell; no classification for the first sample of a symbol or after `on_session_change` / `on_stream_reconnect`; `prepend_candles`/gap inserts keep 0. `Ticker.net_delta` filled in `refresh_vol_rates`-style per recompute. |
| `src/delta.rs` (new) | `tick_sign`, `bvc_split(bars, i, sigma)`, `norm_cdf`, `window_delta(candles, now, minutes) -> Option<(i64, bool est)>`; unit tests (zero tick carry, first-sample unclassified, BVC symmetry, σ = 0, window edges). |
| `src/store.rs` | 8-column write, 6-or-8-column read; tests for mixed files. |
| `src/history.rs` | `parse_bar` sets `buy = sell = 0`. |
| `src/chart.rs` | `merged` sums buy/sell; delta mode for the volume rows; `Spec.delta: bool`. |
| `src/gui.rs` | pixel painting of delta bars. |
| `src/scraper.rs` | `Ticker.net_delta: Option<i64>`, `net_delta_est: bool`. |
| `src/app.rs`, `src/ui.rs` | `delta` column, `Δ Window (min)` field, toggle key, cell rendering, sort. |
| `src/config.rs` | `net_delta_window_min` (default 2, clamp 1–30), chart delta toggle. |
| `src/probe.rs`, `src/stream.rs` | optional: topic type flag for the 103/105 check. |
| `README.md` | column, key, method and its limits. |

## Sources

- [Chakrabarty, Pascual, Shkilko 2015: BVC vs tick rule vs Lee-Ready](https://papers.ssrn.com/abstract=2182819)
- [Same paper, RePEc](https://ideas.repec.org/a/eee/finmar/v25y2015icp52-79.html)
- [Easley, López de Prado, O'Hara: Bulk Classification of Trading Activity](https://www.academia.edu/124868502/Bulk_Classification_of_Trading_Activity)
- [Trade signing in fast markets (Carrion, Ko, Lay)](https://law-journals-books.vlex.com/vid/trade-signing-in-fast-855674305)
- [tedchou12/webull wiki: streaming topics 102/103/105](https://github.com/tedchou12/webull/wiki/How-to-use-Streaming-Quotes%3F)
