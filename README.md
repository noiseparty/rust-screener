# Volume Screener (Rust)

Rust port of `volume_checker_v1.2` (Go). Shows Webull top gainers live and raises NEW and SETUP alerts in a
highlowtick-style TUI built with ratatui. Quotes come from Webull's push stream (MQTT over WSS); REST polling is the
automatic fallback.

## Build / run

```
cargo build --release
target\release\volume_screener.exe
cargo test
```

`rust-toolchain.toml` pins the `stable` toolchain (needs 1.85+). Usage: `volume_screener [--probe [--probe-dir <dir>]]`;
any other argument prints the usage line and exits.

## Stream mode

- The REST top-gainers list is refreshed every 20 s and says which tickers to watch. The app subscribes to those
  tickers (up to 100, current list first, recently dropped ones linger briefly) on `wss://wspush.webullfintech.com`
  and updates price, % change, volume, market cap and day high/low from each push message.
- If the stream is down, the app polls the REST list every `Fallback Poll (s)` seconds until it reconnects. Set
  `"stream_enabled": false` in `config.json` to use REST polling only.
- The stream is an unofficial Webull API and may change without notice. It carries no bid/ask, no individual trades
  and no 52-week high/low.
- Header indicator (top right): green `● WSS n subs m msg/s` is the stream with n subscribed tickers and m messages
  per second; amber `● REST, stream down` is the REST fallback; amber `● WSS ..., list: <error>` is the stream up
  with a failed list refresh; red `● error: ...` is neither working; amber `● connecting` is startup;
  `● live hh:mm:ss` is REST-only mode (stream disabled).

## App home

Config and data live in `%LOCALAPPDATA%\VolumeScreener\`:

```
config.json   settings (contains the proxy key; never commit it)
data\         candles-YYYY-MM-DD.csv, saved 1-minute candles
probe\        probe recordings
```

The folders are created on first run. If `config.json` exists in the current directory and not in the app home, it is
moved there once. No installer is needed.

## Candle storage

Completed 1-minute candles are appended to `data\candles-YYYY-MM-DD.csv` (date = New York trading date; columns
`symbol,start_epoch_secs,open,high,low,close,volume`, no header). Today's file is reloaded on start so charts keep
their history across restarts, and files from previous days are deleted on launch. A failed write is retried with the
next batch.

## Probe mode

```
volume_screener.exe --probe [--probe-dir <folder>]
```

Headless recorder (no TUI) for checking how the stream behaves across session changes: extended-hours price fields,
volume/high/low resets, status codes, `tradeStamp`, and the subscription cap. It refreshes the gainers list over REST
every 20 s (asking for 200 rows, so more than 100 are subscribed), subscribes to all of them and appends every raw
message and connection event to `probe\probe-YYYY-MM-DD.csv` in the app home (date = local start date; a restart on the
same day continues the same file). It reconnects like the app and idles while the market is closed. A status line is
printed every 30 s; `Ctrl+C` stops it and flushes the file (the file is also flushed every second).

Leave it running from before 11:00 Riga (pre-market) through 23:00 and, to see the after-hours session, past it. Then
share the CSV. `--probe-dir` writes somewhere other than `%LOCALAPPDATA%\VolumeScreener\probe\`.

CSV columns: `recv_utc, recv_ny, recv_session` (receive time and the session it falls in), `kind` (`msg` or an event:
`start`, `list`, `list_error`, `connect`, `subscribe`, `unsubscribe`, `suback`, `disconnect`, `undecodable`, `stop`),
`ticker_id, symbol, list_pos` (rank in the latest REST list), then the raw fields as sent: `trans_id, trd_seq, status,
trade_stamp_ms, trade_utc, trade_session` (session of the message's own timestamp), `close, change, change_ratio,
p_price, p_change, p_ch_ratio, market_value, hlt_rsn, sale_inited`, `trade_time, last_price_vol, volume, deal_amount,
open, high, low, wk52_high, wk52_low, stats_inited`, `topic, pub_id, base_inited`, `modules` (other message parts as
`field:length`), `extra` (undocumented fields as `b`/`s`/`t` + number = value), `detail` (event text, e.g. SUBACK return
codes).

## Layout

```
header   VOLUME SCREENER  session · webull group   alerts <filter>        ● stream status
tape     scrolling top-30 gainers (alerted symbols highlighted)
rates    New Highs/Lows (default) or Downticks | Upticks over 20m / 5m / 1m / 30s; `t` switches, remembered in config
main     Top gainers table (zebra, sortable)   |  Alerts feed (+ blacklisted box)
footer   key hints                                          next Ns ━━━━ REST poll gauge
```

Alerts panel hides below 100 columns, rate bars below 26 rows, tape below 16 rows.

Boot screen: animated moonlit mausoleum. Setup screens: animated gear and wrench (shown when the terminal is wide
enough). Art ported from ASCII Magic, MIT, see `THIRD_PARTY_LICENSES.md`.

### Gainers table

Columns: Mov, Symbol, Price, % Chg, Volume, Mkt Cap, Vol/m, Status.

- **Mov**: change in rank on the full gainers board (ranked by % change) over `Rank Track (s)` (default 60), independent
  of your sort and display filters. A green `▲n` / red `▼n` is places gained / lost, a dim `·` is unchanged, a cyan `+` is
  a new entrant, blank is no history yet.
- **Vol/m**: average shares per minute over the last `Vol/m Window (s)` seconds (default 180; header shows e.g.
  `Vol/m 3m`), computed from the stream's cumulative volume. `…` means too little history (needs 30 s), `—` means the
  feed is stale, a dim value means the window is only partly covered.
- **Status**: the alert label (NEW or SETUP) of the row. It is held for the whole alert cooldown and at least 60 s.
- The table re-ranks and re-sorts every `Recompute (ms)` (500 or 1000). A row that stops passing a display filter stays
  visible, dimmed, for 60 s before it hides.

### Alerts

- **SETUP** (the main signal): fires when, versus the value `SETUP Window (s)` ago, volume is at least
  `SETUP Volume Ratio` times higher **and** price is up at least `SETUP Price %`. Defaults: 300 s, 2.2x, +10%. It also
  needs `Min Vol Alert` and `Min Price Alert ($)`. When history is shorter than the window, the oldest sample is used.
  Checked on every recompute tick.
- **NEW**: the symbol is on the gainers list now and was not on it in the last 30 minutes. The first list after
  startup, a session change or a display-filter change only seeds the memory, so nothing fires then.
- **SPIKE and PUMP are hidden for now.** They are not raised and do not appear in the alert filter cycle or the Alert
  Filters screen; the enum variants remain in the code so they can return.
- Each symbol and alert kind has a cooldown (`Alert Cooldown (s)`, default 120). Blacklisted symbols raise no alerts.
- Highlight colors: NEW yellow, SETUP purple, newest alert flashes green.

### Rate panel

`t` switches between two tabs, and the choice is saved in `config.json`:

- **New Highs/Lows** (default): count of session new highs and lows across the board in the last 20m / 5m / 1m / 30s.
  A ticker's first quote of a session only seeds its high and low, and it raises no event for its first 60 s.
- **Up/Down ticks**: price upticks and downticks over the same windows.

## Chart

`Enter` on a row opens a 1-minute candlestick chart for that ticker (↑/↓ switches ticker, Esc back). Candles are built
from every price/volume sample the app receives (up to 390 per ticker) and are saved to disk (see Candle storage), so
after a restart the chart covers today's saved candles plus the time since. There is no backfill from before the app
first saw the ticker.

## Keys

| Key | Action |
|-----|--------|
| `Enter` | candlestick chart for selected row |
| `←/→` `h/l` | select column |
| `s` | sort by selected column (again to flip); re-sorts locally, no fetch |
| `↑/↓` `j/k`, PgUp/PgDn, `g` | move row cursor (follows the selected symbol) |
| `f` | cycle alert filter (ALL/NEW/SETUP, enabled ones only) |
| `t` | rate panel tab: New Highs/Lows or Up/Down ticks |
| `v` | column visibility |
| `o` / `c` | setup (config with live filter preview, blacklist, alert filters) |
| `r` | refresh the gainers list now |
| `q` / `Ctrl+C` | quit |

Config screen: Tab/↑/↓ to move, type to edit, Enter or Ctrl+S to save, Esc to cancel. Saving re-sorts and re-filters
locally without fetching.

## Settings

| Setting | Meaning (default) |
|---------|-------------------|
| Min / Max Market Cap | display filter; K/M/B suffixes allowed (0 = off) |
| Fallback Poll (s) | REST poll interval, used only while the stream is down (5) |
| Rank Track (s) | window for the Mov column (60, min 5) |
| Recompute (ms) | re-rank / re-sort tick, 500 or 1000 (1000) |
| Vol/m Window (s) | window of the Vol/m column (180, min 30) |
| SETUP Window (s) | look-back for SETUP (300) |
| SETUP Volume Ratio | volume multiple needed (2.2) |
| SETUP Price % | price rise needed, 10 = +10% (10) |
| Min Vol Alert / Min Price Alert ($) | gates before SETUP can fire (10000 / 1.0) |
| Alert Cooldown (s) | per symbol and kind (120) |
| Min / Max % Change, Volume, Price | display filters (0 = off) |

Old config keys are migrated on load: `spike_vol_ratio` to `setup_vol_ratio`, `baseline_window_sec` to
`setup_window_sec`, `pump_price_pct` (a fraction) to `setup_price_pct` (percent), `delta_window_seconds` to
`vol_m_window_seconds`.

Sessions are shown from New York time: pre-market from 04:00, regular from 09:30, after-hours from 16:00 ET (in Riga
winter time 11:00, 16:30 and 23:00).

## Changes vs the Go version

- Market session is computed in America/New_York time (was Riga). Fixes the ~3 weeks/year where US and EU DST differ.
- Sorting is keyed by column, not visible-column index (Go sorted the wrong field when columns were hidden).
- Config filter preview uses the raw scrape, so loosening a filter shows tickers that are currently filtered out.
- Volume/price history pruned by time window instead of a fixed 200-sample cap.
- Fetching keeps running while in setup screens; previews use a snapshot taken on entry.
- Finviz columns dropped (they were never populated); `finviz_*` config keys are kept and the proxy is still in Health Check.
- `column_order` and `finviz_*_seconds` config keys removed (unused).
- The Δ volume column became Vol/m; SPIKE and PUMP alerts are hidden; the live push stream replaces 5 s polling.
