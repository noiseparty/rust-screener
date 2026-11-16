# Volume Screener (Rust)

Rust port of `volume_checker_v1.2` (Go). Polls Webull top gainers, detects NEW / SPIKE / PUMP / SETUP alerts, and renders a
highlowtick-style TUI with ratatui.

## Build / run

```
cargo build --release
target\release\volume_screener.exe
cargo test
```

## App home

Config and data live in `%LOCALAPPDATA%\VolumeScreener\`:

```
config.json   settings (contains the proxy key; never commit it)
data\         saved data (candle storage comes in a later step)
probe\        probe recordings
```

The folders are created on first run. If `config.json` exists in the current directory and not in the app home, it is
moved there once. No installer is needed.

`rust-toolchain.toml` pins the `stable` toolchain (needs 1.85+).

## Probe mode

```
volume_screener.exe --probe
```

Headless recorder (no TUI) for checking how the stream behaves across session changes: extended-hours price fields,
volume/high/low resets, status codes, `tradeStamp`, and the subscription cap. It refreshes the gainers list over REST
every 20 s (asking for 200 rows, so more than 100 are subscribed), subscribes to all of them and appends every raw
message and connection event to `probe\probe-YYYY-MM-DD.csv` in the app home (date = local start date; a restart on the
same day continues the same file). It reconnects like the app and idles while the market is closed. A status line is
printed every 30 s; `Ctrl+C` stops it and flushes the file (the file is also flushed every second).

Leave it running from before 11:00 Riga (pre-market) through 23:00 and, to see the after-hours session, past it. Then
share the CSV. `--probe-dir <folder>` writes somewhere other than `%LOCALAPPDATA%\VolumeScreener\probe\`.

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
header   VOLUME SCREENER  session · webull group   alert filter        ● live hh:mm:ss
tape     scrolling top-30 gainers (alerted symbols highlighted)
rates    New Highs/Lows (default) or Downticks | Upticks over 20m / 5m / 1m / 30s; `t` switches, remembered in config
main     Top gainers table (zebra, sortable)   |  Alerts feed (+ blacklisted box)
footer   key hints                                          next Ns ━━━━ refresh gauge
```

Highlight colors: NEW yellow, PUMP orange, SPIKE pink, SETUP purple, newest alert flashes green.
Alerts panel hides below 100 columns, rate bars below 26 rows, tape below 16 rows.

Boot screen: animated moonlit mausoleum. Setup screens: animated gear and wrench (shown when the terminal is wide
enough). Art ported from ASCII Magic, MIT, see `THIRD_PARTY_LICENSES.md`.

## Chart

`Enter` on a row opens a 1-minute candlestick chart for that ticker (↑/↓ switches ticker, Esc back). Candles are built
in memory from every price/volume sample the app receives (up to 390 per ticker, about 4 KB per ticker per hour), so the
chart only covers time since the ticker was first seen. Nothing is written to disk yet.

## Keys

| Key | Action |
|-----|--------|
| `Enter` | candlestick chart for selected row |
| `←/→` `h/l` | select column |
| `s` | sort by selected column (again to flip) |
| `↑/↓` `j/k`, PgUp/PgDn, `g` | move row cursor |
| `f` | cycle alert filter (ALL/NEW/PUMP/SPIKE/SETUP, enabled ones only) |
| `v` | column visibility |
| `o` / `c` | setup (config with live filter preview, blacklist, alert filters) |
| `r` | refresh now |
| `q` / `Ctrl+C` | quit |

Config screen: Tab/↑/↓ to move, type to edit, Enter or Ctrl+S to save, Esc to cancel.

## Changes vs the Go version

- Market session is computed in America/New_York time (was Riga). Fixes the ~3 weeks/year where US and EU DST differ.
- Sorting is keyed by column, not visible-column index (Go sorted the wrong field when columns were hidden).
- Config filter preview uses the raw scrape, so loosening a filter shows tickers that are currently filtered out.
- Volume/price history pruned by time window instead of a fixed 200-sample cap.
- Fetching keeps running while in setup screens; previews use a snapshot taken on entry.
- Finviz columns dropped (they were never populated); `finviz_*` config keys are kept and the proxy is still in Health Check.
- `column_order` and `finviz_*_seconds` config keys removed (unused).
- Live quotes come from Webull's push stream (MQTT over WSS, `src/stream.rs`); the REST list is refreshed every 20 s and `refresh_interval_seconds` is the poll used only while the stream is down. Set `"stream_enabled": false` in `config.json` to use REST polling only. The header shows `● WSS n subs m msg/s` or `● REST, stream down`.
