# Float data from Webull (research)

Date: 2026-11-17, during pre-market (status `F`). Research only; no code changed.

## Answer

Yes, Webull gives float anonymously. Webull calls it **`outstandingShares`**. Total shares are in **`totalShares`**.
Both come from the anonymous REST quote endpoints, and several tickers can be fetched in one request. The push stream does
not send a share count, but StatsItem field 7 `negMarketValue` divided by SaleItem field 1 `close` gives the same number
exactly (from the REST data; not yet confirmed on the stream).

## Sources checked

| Source | Float info |
|---|---|
| `src/scraper.rs`, gainers `GET /api/bgw/market/pcIndex?regionId=6&pageSize=N` | Rows have `marketValue` (parsed), plus `turnoverRate`, `volume`, `close`, `preClose` (`turnoverRate` is ignored). The rows have no `outstandingShares`, `totalShares` or `negMarketValue`. |
| `src/stream.rs` `decode_items_combo` / `decode_raw`, `docs/plan.md` | SaleItem 7 `marketValue` is decoded into `Update.mcap`. StatsItem 6 `turnoverRate` and 7 `negMarketValue` are documented but not decoded into `Update`; `decode_raw` (probe) keeps them. No probe CSV exists locally, so I could not check whether the stream sends field 7 on every message. |
| `GET /api/stock/tickerRealTime/getQuote?tickerId=ID&includeSecu=1&includeQuote=1&more=1` | One ticker. Returns `totalShares`, `outstandingShares`, `negMarketValue`, `turnoverRate`, `avgVol10D`, `avgVol3M`. |
| `GET /api/bgw/quote/realtime?ids=ID1,ID2,...&includeSecu=1&delay=0&more=1` | **Several tickers in one request.** Returns a JSON array with the same fields as getQuote. |
| Web search | No public documentation for these endpoints. I confirmed the field names with the live requests below. |

Live requests: 4 GETs to `quotes-gw.webullfintech.com` with a generic UA, spaced 3 s apart. All returned 200.

## Example values (from the live requests)

| Ticker | close | totalShares | outstandingShares | negMarketValue | negMV / close | turnoverRate |
|---|---|---|---|---|---|---|
| AAPL 913256135 | 352.78 | 14,532,182,000 | 14,272,214,729 | 5,034,951,912,096.62 | 14,272,214,729 | 0.0020 |
| TSLA 913255598 | 379.74 | 3,952,757,946 | 3,245,325,768 | 1,232,380,007,140.32 | 3,245,325,768 | 0.0064 |
| GP 950155382 (top pre-market gainer) | 1.410 | 2,100,709 | 1,297,073 | 1,828,872.93 | 1,297,073 | 17.3706 |

- `outstandingShares` is the float. For TSLA it is about 82% of `totalShares`, which fits insider holdings being left out.
- `negMarketValue = outstandingShares × close`, where `close` is the **regular-session** close (in Pre/After it is the
  previous close, not `pPrice`). So `float = negMarketValue / close` is exact.
- `marketValue / close = totalShares`. GP: 2,961,999.69 / 1.41 = 2,100,709.
- **`turnoverRate` does not work as an estimate.** In pre-market it stays at the previous regular session's ratio. For
  GP, 17.3706 × 1,297,073 = 22.53M, but `volume` was 29.27M (pre-market volume). The gainers row shows the same 17.3706.
  It also has only 4 decimals, so large caps lose precision (AAPL 0.0020). Use it only as a last resort during the
  regular session.

## Freshness and cost

- Float changes when Webull's share data updates (filings, offerings, reverse splits), not during the day. **One fetch per
  ticker per day is enough.**
- Batch endpoint: 1 request per batch of ids, and it goes through `limiter::get` like every other REST call. The largest
  batch size was not tested (2 ids worked). Start at 20–50 ids per request. A full gainers list costs 1–3 requests at startup,
  then about 1 request whenever new tickers appear.
- Stream route: costs no extra requests, but only once field 7 has been seen for that ticker.

## Caveats

- Webull's float can lag a fresh offering or reverse split on small caps. Show it as Webull's figure, not as a guaranteed float.
- ETFs and funds (`template: "etf"`) have no `outstandingShares`/`negMarketValue`. Index rows have `netAsset`/`totalAsset`
  instead. Show `N/A`.
- Some rows return `"0"` or no field. Treat 0 as unknown (same rule as `market_value == 0.0` → `N/A`).
- `negMarketValue / close` needs the regular close. In Pre/After, use `Update.close` or `preClose` from the stream/REST,
  never `pPrice`.
- `/api/bgw/quote/realtime` is undocumented. If it starts returning 403, the limiter's existing pause covers it.

## Implementation outline

1. **Fetch (REST batch, primary):** add `fetch_floats(agent, ids: &[u64]) -> HashMap<u64, (f64 float, f64 total)>`
   to `src/scraper.rs` (`/api/bgw/quote/realtime?ids=..&includeSecu=1&delay=0&more=1`, through `limiter::get`), parsed
   with the existing `num()`.
2. **Worker:** start a `float` thread in `src/launch.rs` the same way as the levels worker (`mpsc::Receiver<Vec<u64>>` →
   `Msg::Floats(HashMap<..>)`). Add the `Msg` variant where `Msg` is defined.
3. **Cache:** keep `HashMap<tickerId, f64>` in `App`. Persist it to `data\floats-YYYY-MM-DD.json` (New York date) next to
   the candle files, reload today's file on start, and let the existing old-day purge delete it. When a REST list arrives
   (`Msg::Fetched`), send ids with no cached value to the worker, in batches.
4. **Stream fallback (optional):** decode StatsItem 7 into a new `Update.neg_mcap: Option<f64>` in `src/stream.rs`. In
   `engine.rs` around line 346 (where `mcap` is applied), set `float = neg_mcap / close` when there is no cached value.
   Run `--probe` first to confirm that field 7 arrives.
5. **Ticker field:** add `pub float: f64` (0 = unknown) to `Ticker` in `src/scraper.rs`, filled from the cache in the engine.
6. **Column:** add `Column { key: "float", title: "Float", width: 8, priority: 3 }` to `src/app.rs` (line 36 area). Render it
   in `src/ui.rs` like `mcap` (lines 590 and 976): `compact(t.float)` or `N/A`. Add the sort key in `src/engine.rs` (line 870).
7. **Filter:** add `min_float` / `max_float` to `src/config.rs` (default 0 = off) and to `Filters` in `src/engine.rs`
   (copy the `min_mcap` logic at lines 129-135, where unknown fails only when a bound is set). Add `FieldId::MinFloat/MaxFloat`
   with `FieldKind::Smart` (K/M/B suffixes) in `src/app.rs` next to `MinMcap`. Update config/engine tests.
8. Add one line about the Float column and filter to the README.
