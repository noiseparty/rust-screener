# Stream rework plan: user decisions (authoritative)

`docs/stream-design.md` is the reviewed technical design. **Where this file differs, this file wins.**

## User decisions

| # | Item | Decision |
|---|------|----------|
| 1 | Alert flash bug | Fix (single `now` for alert time and `last_alert_at`). |
| 2 | Blacklisted symbols raise alerts | Fix: no alerts for blacklisted. |
| 3 | History lost while filtered | Fix: record history for every ticker before display filters. |
| 4 | Sorting by Mov reads zeros | Fix. |
| 5 | Cursor jumps after re-sort | Selection follows the symbol (`selected_symbol`). |
| 6 | `s` / config save trigger fetch | Re-sort locally, no fetch. |
| 7 | Live probe | `--probe` mode in the main exe: headless recorder the user leaves running a full day. Sessions (Riga, winter): pre 11:00, regular 16:30, after 23:00. Sessions are computed in America/New_York so DST is automatic. |
| 8 | Stream client | Yes, with automatic REST fallback. |
| 9 | Vol/m column | Yes, replaces the Δ volume column (design 2.1). |
| 10, 11 | New rate SPIKE / robust PUMP | **Not built.** SPIKE and PUMP alerts are **hidden for now**: not emitted, not in the alert filter cycle or Alert Filters screen. Keep the enum variants so they can return later. |
| 13 | SETUP | **Keep the user's proven SETUP logic**, not design 2.5: SETUP fires when, versus the value N seconds ago, volume ratio ≥ `setup_vol_ratio` **and** price change ≥ `setup_price_pct`. All three adjustable in Settings: **SETUP Window (s)** (default 300), **SETUP Volume Ratio** (default 2.2 — note the user's config has `spike_vol_ratio` 2.2), **SETUP Price %** (default from user's `pump_price_pct`, entered as percent e.g. 10 = +10%). Migrate old keys `spike_vol_ratio`→`setup_vol_ratio`, `pump_price_pct`→`setup_price_pct` (×100 if stored as fraction ≤1), `baseline_window_sec`→`setup_window_sec`. With the stream, "value N seconds ago" uses the series store (receive-time lookup `vol_at(now-N)`, `px_at(now-N)`), with the existing fallback to the oldest sample when history is shorter than N. Eligibility gates `min_volume_for_alert`, `min_price_for_alert` stay. |
| 12 | Cooldown | Per symbol+kind cooldown, `alert_cooldown_sec` default 120, adjustable. **While cooling down, the Status column still shows the label.** |
| 14 | NEW | Design 2.6: NEW = listed now and not listed in the last 1800 s; first list after startup / session change / filter change seeds silently. |
| 15 | Sticky badge | Status label held 60 s after an alert (and for the whole cooldown, per 12). |
| 16 | New highs/lows | Rate panel gets **tabs**, toggled with `t`: **New Highs/Lows** (default, design 2.8) and **Up/Down ticks** (existing). Remember the chosen tab in config. |
| 17 | Recompute | Recompute/re-rank tick, adjustable **0.5 s or 1 s** (`recompute_ms`, default 1000; setting accepts 500 or 1000). |
| 18 | Mov | Design 2.9(b): Mov = change in rank on the full gainers board ranked by % change, over `rank_tracking_interval_seconds`, independent of user sort and display filters. New entrant shows cyan `+`, no history shows blank. |
| 19 | Filter grace | A row that stops passing a display filter stays visible **dimmed for 60 s**, then hides. |
| 20 | Settings | Add: SETUP Window (s), SETUP Volume Ratio, SETUP Price %, Alert Cooldown (s), Recompute (ms). Rename "Refresh Rate (s)" → "Fallback Poll (s)" (REST poll used only when the stream is down). Remove Spike Vol Ratio / Pump Price Pct / Baseline Window labels (replaced by SETUP fields). Keep `preview_filters` indices correct (it hard-codes indices; switch it to named lookups). |
| 21 | Storage | No installer. App home = `%LOCALAPPDATA%\VolumeScreener\` (config.json, `data\`, `probe\`). On first run, if `config.json` exists in the current directory and not in the app home, move it there. Completed 1-minute candles appended to `data\candles-YYYY-MM-DD.csv` (New York trading date) and reloaded on start; on launch, delete data files from previous days. |

## Stream protocol facts (verified live)

- WSS `wss://wspush.webullfintech.com:443/mqtt`, WebSocket subprotocol `mqtt`, header `Origin: https://webull.com`, MQTT 3.1.1, client id = random 32-hex `did`, username `test`, password `test`, keepalive 30 s.
- After CONNACK, SUBSCRIBE (QoS 0) to topic string `{"header":{"did":"<did>","hl":"en","app":"global","os":"web","osv":"h5","ver":"1.0.0","platform":"webull-us"}}`, then to `{"tickerIds":[...],"type":"102","flag":"1"}`. Only type 102 with flag "1" returns data anonymously. Unsubscribe uses the same topic string form.
- PUBLISH topic is JSON like `{"type":102,"tickerId":950978197,"modules":[""],"flag":"1"}`; payload is protobuf `ItemsCombo`:
  - ItemsCombo: 1 base(BaseItem), 2 sale(SaleItem), 3 stats(StatsItem), 4 deal, 5 quote, 6 ntv, 7 noii
  - BaseItem: 1 transId u64, 2 trdSeq u64, 3 tickerId u64, 4 status string, 5 tradeStamp u64 (ms), 6 topic u32, 7 pubId u64, 9 inited bool (unknown fields 8,11,12,13 appear; skip unknown)
  - SaleItem: 1 close, 2 change, 3 changeRatio, 4 pPrice, 5 pChange, 6 pChRatio, 7 marketValue (all strings), 8 inited bool, 9 hltRsn string
  - StatsItem: 1 tradeTime, 2 lastestPriceVol, 3 volume (cumulative day), 4 dealAmount, 5 vibrateRatio, 6 turnoverRate, 7 negMarketValue, 8 open, 9 high, 10 low, 11 fiftyTwoWkHigh, 12 fiftyTwoWkLow (strings), 29 inited bool; skip unknown fields (field 52 seen)
- Observed ~0.5–1 msg/s per hot ticker. REST gainers rows include `ticker.tickerId`.
- A Python reference decoder that worked: `docs/stream-design.md` section 1 and the field list above. Hand-write the protobuf varint/length-delimited reader (no codegen).

## Engineering rules

- Rust stable 1.85 (`rust-toolchain.toml`). Avoid crates needing newer rustc; keep `resolver = "3"`.
- Prefer sync code and threads (no tokio). Suggested: `tungstenite` with rustls, hand-written minimal MQTT 3.1.1 framing.
- Minimum complexity; no speculative features. Root-cause fixes.
- Every step: `cargo build` with zero warnings, `cargo test` passing, add unit tests for new logic, then commit to local git (no remote) with message ending in the attribution line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Do not touch `src/art.rs` / `src/chart.rs` drawing except to feed data.
