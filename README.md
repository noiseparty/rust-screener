# MarketGUI

A fast, keyboard-driven **US stock momentum screener** for the desktop, written in Rust. It watches the market's top
gainers in real time, ranks them, and alerts you when a ticker starts to move on unusual volume, with a live candlestick
chart, swing levels and everything kept on your own machine.

```
 Full  List  Chart  Alerts   Regular Market   close in 02:41:07          ● online 14 Hz   up 1h 12m   RAM 41%
┌ Alerts ─────────────────────────────────────┐┌ XYZ  1m 5m 1h 4h D ──────────────────────────────┐
│ 15:42:10  SETUP  XYZ   vol 2.3x  +15.0%      ││                                    ┃  ▲ 4.12   │
│ 15:39:55  NEW    ABCD  appeared              ││                              ┃ ┃ ┃┃┃           │
├ Gainers 42 ─────────────────────────────────┤│                 ┃┃   ┃ ┃ ┃┃┃┃┃┃┃      3.86 ── │
│ Mov Symbol  Price  % Chg  Volume  Vol/m  St ││        ┃ ┃┃┃┃┃┃┃┃┃┃┃┃                         │
│ ▲3  XYZ     4.05  +52.4%  18.40M  210K SETUP││  ┃┃┃┃┃┃┃                              04:12   │
│ ·   ABCD    1.18  +31.7%   6.20M   48K NEW  ││                                                 │
└─────────────────────────────────────────────┘└─────────────────────────────────────────────────┘
```

## Features

- **Live top gainers** from a push stream (quotes several times a second), with REST polling as automatic fallback.
- **SETUP alerts**: volume in the last window is a multiple of the window before **and** price is up by a set percent
  (defaults: last 5 min vs the 5 min before, 2.2x, +10%). **NEW alerts** for tickers that just joined the list.
  Per-symbol cooldowns, alert filters and an alert sound.
- **Gainers table** with rank movement (Mov), shares-per-minute (Vol/m), float and total shares, sortable columns,
  display filters (price, % change, volume, market cap, float) and a blacklist.
- **Candlestick chart** of the selected ticker in 1m / 5m / 1h / 4h / D, TradingView-style drag and zoom, live price
  tag, candle countdown and **4h swing levels** (support/resistance from a year of 4h bars).
- **Local history**: 1-minute candles saved per ticker, chart bars cached, so charts survive restarts.
- **Two front ends, one app**: a native window (`MarketGUI.exe`) and a terminal UI (`MarketGUI-cli.exe`).
- 40 colour themes, presets, configurable time zone / date / number formats, mouse support everywhere.
- **Interchangeable data source** (see [Data sources](#data-sources)): the screener, alerts, charts and UI never see a
  provider's formats.
- Built-in diagnostics: health check, trace mode with a live data-flow view, a probe recorder and replay tests.

## Quick start

Requires Windows 10/11 and [Rust](https://rustup.rs) (the toolchain version is pinned in `rust-toolchain.toml` and
installed automatically by rustup). Other platforms are untested.

```
git clone https://github.com/noiseparty/rust-screener
cd rust-screener
cargo build --release
target\release\MarketGUI.exe        the app window
target\release\MarketGUI-cli.exe    the same app in a terminal (use Windows Terminal)
```

No installer and no account are needed. On first start, **Start** opens Configuration with the display filters
focused and a live preview: set them so the list stays short (every listed ticker gets its chart preloaded), or press
`p` for the Recommended preset, then Enter to save and start.

Settings and data live in `%LOCALAPPDATA%\MarketGUI\`:

```
config.json         settings
presets\            saved presets (<name>.json)
data\candles\       saved 1-minute candles, one CSV per ticker
data\bars\<tf>\     cached 5m / 1h / 4h / D chart bars
data\levels\        cached 4h bars for swing levels
data\floats.csv     float / shares cache
probe\              probe recordings
logs\               trace logs (kept 7 days)
```

Nothing is deleted automatically except trace logs and level caches untouched for 30 days; **Config → Storage** browses
the folder and deletes what you no longer need.

## Data sources

MarketGUI ships with **Webull** as its data source, read anonymously (no login):

| Data | Webull source | Module |
|------|---------------|--------|
| Top gainers list (which tickers to watch) | REST, polled every 20 s (every *Fallback Poll* seconds while the stream is down) | `scraper.rs` |
| Live quotes: price, % change, volume, day high/low, market cap | push stream, MQTT 3.1.1 over WebSocket, protobuf payloads | `stream.rs`, `stream/` |
| Chart bars (5m / 1h / 4h / D), 1m backfill, 4h swing-level history | REST chart endpoint | `history.rs`, `backfill.rs`, `levels.rs` |
| Float and total shares | REST quote endpoint | `scraper.rs` |

These are unofficial, undocumented endpoints: they can change or disappear without notice, and they carry no bid/ask,
no individual trades and no 52-week range. All REST calls share one request budget (5 requests/s; a 15-minute pause
after an HTTP 403) in `limiter.rs`.

**The data source can be swapped.** Provider code sits at the edge of the app. Each source runs on its own worker
thread and hands the app plain values over a channel: a list of tickers, batches of quotes, lists of candles
(`app::Msg::Fetched`, `Quotes`, `Bars`, `Backfill`, `Levels`, `Floats`). The engine, alerts, ranking, charts, storage
and UI work only on those values. To use another provider (a broker or market-data API, your own feed, a recorded
file), write workers that produce the same messages and start them in `launch.rs` in place of the Webull ones. The push
stream is optional (`"stream_enabled": false` runs on the polled list alone), so a REST-only provider is enough to
start. The [replay tests](#replay-tests) already run the whole app on recorded data with no network at all.

## Using the app

### Dashboard

```
header   layout tabs   market phase + countdown   connection status   uptime   storage   RAM
main     Alerts (5-15 rows)                  |  chart of the selected row
         Gainers table                       |  (+ blacklisted tickers)
footer   key hints
tape     scrolling top-30 gainers
```

- **Layouts** (header tabs or `w`): Full (alerts + list + chart), List, Chart, Alerts (full-height, scrollable).
- **Market phase** in New York time with a countdown: Pre Market 04:00-09:30, Regular 09:30-16:00 (with the
  10:30-11:30 "zombie hour" marked), After Market 16:00-20:00, otherwise Market Closed (US holidays are not known).
- **Connection status**: green `● online n Hz` = stream receiving n messages/s; amber `● REST` = polling fallback;
  red `● offline` = neither; amber `● connecting` = startup.

### Gainers table

Columns: Mov, Symbol, Price, % Chg, Volume, Mkt Cap, Float, Shares, Vol/m, Status (Float and Shares are off by
default; `v` toggles columns).

- **Mov**: rank change on the full board (by % change) over *Rank Track* seconds, independent of your sort and filters.
  `▲n` / `▼n` places gained / lost, `·` unchanged, `+` new entrant.
- **Vol/m**: average shares per minute over the last *Vol/m Window* seconds. `…` = too little history (needs 30 s),
  `—` = feed stale, dim = window only partly covered.
- **Float / Shares**: fetched once per New York day per ticker; `N/A` when unknown (ETFs).
- **Status**: the row's alert badge, held for the cooldown (at least 60 s).
- The table re-ranks every *Recompute* tick (500 or 1000 ms). A row that stops passing a display filter stays dimmed for
  60 s before it hides.

### Alerts

- **SETUP** (main signal): shares traded in the last *SETUP Window* ≥ *SETUP Volume Ratio* × the window before, **and**
  price ≥ *SETUP Price %* above one window ago, plus the *Min Vol Alert* and *Min Price Alert* gates. Windows, not the
  day's total, so a ticker that spikes again late in the day still fires. A previous window only partly watched is
  scaled to a full one; with less than 1.1 windows of history nothing fires.
- **NEW**: on the gainers list now and not in the last 30 minutes. The first list after startup, a session change or a
  filter change only seeds the memory, so there is no flood.
- Cooldown per symbol and kind (*Alert Cooldown*, default 120 s). Blacklisted symbols never alert. `f` cycles the
  alert filter. SPIKE and PUMP alert types exist in the code but are switched off.

### Chart

The selected row is charted beside the list; `Enter` opens it full screen (↑/↓ switches ticker). Timeframes `1`-`5`
(1m, 5m, 1h, 4h, D) or the title tabs.

- **1m** candles are built from the live feed and saved to disk; on launch the minutes the app was closed are fetched
  (back to 04:00 New York), and new tickers get up to 10 minutes of history from before they appeared.
- **5m / 1h / 4h / D** come from the chart endpoint, 48 bars per view, preloaded for every listed ticker in the
  background and cached on disk, so a cached chart shows instantly and is only topped up.
- **Navigate** like TradingView: drag the plot, drag the price axis to scale (double-click to fit), mouse wheel to zoom
  time, `0` to reset. Dragging back past the oldest bar fetches older ones.
- **4h swing levels** (all timeframes except D): the 3 untaken levels nearest the price are drawn, levels already
  traded through are dimmed, levels off-screen become edge tags, each labelled with date and price.
- **Price tag and countdown** on the price axis, showing time left in the current candle.

### Configuration

`o` or `c` opens Config: a sidebar (Configuration, Presets, Theme, Time & Format, Blacklist, Alert Filters, Sound,
Storage, Health Check), the selected item's settings and a live preview. Enter / → / Tab moves into an item, Esc goes
back (and drops unsaved changes).

| Setting | Meaning (default) |
|---------|-------------------|
| Min / Max Market Cap | display filter; K/M/B suffixes allowed (0 = off) |
| Fallback Poll (s) | REST poll interval while the stream is down (5) |
| Rank Track (s) | window of the Mov column (60, min 5) |
| Recompute (ms) | re-rank / re-sort tick, 500 or 1000 (1000) |
| Vol/m Window (s) | window of the Vol/m column (180, min 30) |
| SETUP Window (s) | SETUP look-back (300) |
| SETUP Volume Ratio | last window's volume vs the window before (2.2) |
| SETUP Price % | price rise needed, 10 = +10% (10) |
| Min Vol Alert / Min Price Alert ($) | gates before SETUP can fire (10000 / 1.0) |
| Alert Cooldown (s) | per symbol and kind (120) |
| Min / Max % Change, Volume, Price | display filters (0 = off) |
| Min / Max Float, Shares | display filters; a ticker with unknown counts passes (0 = off) |
| Ticker Tape / Key Hints / Alert Rows | dashboard layout (On / On / 5) |

- **Presets**: named configurations (`presets\<name>.json`); the built-in *Recommended* is always first.
- **Theme**: 40 themes with a live mini-dashboard preview: the default *Dark Noir* plus the
  [opaline](https://github.com/hyperb1iss/opaline) set (Catppuccin, Tokyo Night, Gruvbox, Nord, Dracula, …).
- **Time & Format**: time zone (any IANA name), date pattern, 12/24 h clock, currency symbol, decimal and thousands
  separators, price decimals, abbreviated or full volumes. Market phases and candle boundaries always stay New York based.
- **Blacklist**: hide tickers from the ranking (still tracked, listed separately).
- **Alert Filters**: which alert types show, and the number of alert rows.
- **Sound**: mute and volume of the alert sound (built into the exe), with a test button.
- **Storage**: file browser over the app folder with sizes; delete files or folders (`config.json` is protected).
- **Health Check**: network and data-source checks, plus system stats (RAM, CPU, disk, request rate, stream rate).

Saving re-filters and re-sorts locally without a new fetch. Older config keys are migrated automatically on load.

### Keys

| Key | Action |
|-----|--------|
| `↑/↓` `j/k`, PgUp/PgDn, `g` | move the row cursor (it follows the selected symbol) |
| `←/→` `h/l`, `s` | select a column, sort by it (again to flip) |
| `Enter` | full-screen chart of the selected row |
| `1`-`5`, `t`, `0` | chart timeframe, next timeframe, reset the chart view |
| `w` | next layout |
| `f` | cycle the alert filter (ALL / NEW / SETUP) |
| `v` | column visibility |
| `o` / `c` | Config |
| `r` | refresh the gainers list now |
| `F12` or `` ` `` | debug overlay (trace mode only) |
| `q` / `Ctrl+C` | quit |

In the window: Ctrl + wheel or Ctrl +/- changes the text size, Ctrl+0 resets it. The window has its own thin title bar
(drag to move, double-click to maximize, drag edges to resize).

**Mouse**: click a row to chart it, a column header to sort, an alert to jump to its ticker, a tab or a footer hint to
use it; drag and wheel on the chart as above.

## Diagnostics and development

### Project layout

The app is one Rust library (`src/lib.rs`) with two binaries. A short map, the data-flow diagram and the coding rules
are in [`CLAUDE.md`](CLAUDE.md); research notes are in [`docs/`](docs/).

```
cargo test                          all tests (~15 s), including the replay tests
cargo build --profile debugrel      release speed, trace on, no LTO (faster to build)
cargo build --release               the release exes
cargo fmt                           formatting (rustfmt.toml)
```

### Trace mode

Trace mode logs how data moves through the app: stream, REST requests, worker answers, app messages and the engine.
It is on in debug builds (`cargo test`, `cargo build`, `--profile debugrel`), with `--trace`
(`MarketGUI.exe --trace`, `MarketGUI-cli.exe --trace`), or with the environment variable `MARKETGUI_TRACE=1`. Off,
the app behaves exactly as without it.

While tracing, the header shows a `TRACE` badge and `F12` (or `` ` ``) opens the debug overlay: a **flow table** with one
row per step of the data path, and the newest log lines below it (↑/↓, PgUp/PgDn scroll, End jumps to the newest, Esc
closes).

| Column | Meaning |
|--------|---------|
| category | the step: `stream` (dial, connect, disconnect), `stream.subs`, `stream.batch`, `stream.rate`, `rest.gainers` / `rest.history` / `rest.backfill` / `rest.levels` / `rest.floats` / `health` (worker answers: rows or bars, time, errors), `limiter.*` (requests, skips, 403 pauses), `app.*` (a message as the app handles it), `engine.process` / `engine.recompute` / `engine.alerts` |
| count | messages in that step since start |
| age | time since its newest message (green: under 2 s) |
| last | the newest message |

A worker row followed by its `app.*` row shows a request reaching the app; a count that stops growing or an age that
keeps rising shows where data stops. Frequent steps are only counted, not written as lines. Logs go to
`%LOCALAPPDATA%\MarketGUI\logs\trace-YYYY-MM-DD.log` (`HH:MM:SS.mmm category message`, kept 7 days).

### Probe mode

```
MarketGUI-cli.exe --probe [--probe-dir <folder>]
```

A headless recorder for studying the push stream: it subscribes to the whole gainers list (up to 200 tickers) and
appends every raw message and connection event to `probe\probe-YYYY-MM-DD.csv`, reconnecting like the app and idling
while the market is closed. A status line prints every 30 s; `Ctrl+C` stops it. Columns: receive time and session,
event kind (`msg`, `connect`, `subscribe`, `suback`, `disconnect`, `list`, …), ticker id, symbol, list position, then
every raw field of the message as sent, undocumented fields included.

### Replay tests

`cargo test replay` plays recorded market sessions (REST list, stream quotes, connection events) through the real app
on a simulated clock and checks what the dashboard would show: rows and order, Vol/m, NEW and SETUP alerts with
cooldown and badge, dimmed rows, REST fallback. They take well under a second and never touch the network.

Fixtures are `tests/fixtures/*.replay`, in a small line format (documented at the top of
`src/app/tests/replay/fixture.rs`):

```
# comment
config min_change_percent=10                      config.json keys, before any event
t=0s stream up
t=0s rest XYZ,201,2.00,0.25,500000,40000000       SYM,tickerId,price,change_ratio,volume,mcap; rows sharing a t are the list
t=1s quote 201,,1,2.00,,,500000,                  tickerId,trade_ms,seq,close,p_price,change_ratio,volume,mcap
t=150s rest-error HTTP 503                        fetches fail until the next list
```

To add a scenario, write `tests/fixtures/<name>.replay` and a test in `src/app/tests/replay/scenarios.rs` that calls
`Replay::load("<name>.replay")`, `run_to_secs(..)` and asserts on `rows()`, `symbols()`, `row(sym)` and `alerts()`.
Probe CSV rows can be pasted into a fixture unchanged (`msg` rows become quotes, `connect` / `disconnect` become
`stream up` / `down`; put `start <recv_utc of the first row>` at the top). The probe does not record the list rows, so
add `rest` lines by hand where the list changed.

## Disclaimer

MarketGUI is a personal tool, provided as is. It is not affiliated with or endorsed by Webull or any data provider, and
it uses their public web endpoints without an API agreement; check the provider's terms before relying on it. Market
data may be delayed, incomplete or wrong. Nothing here is financial advice.

## Credits

Boot and chart art ported from [ASCII Magic](https://www.ascii-magic.com) (MIT), see
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). Themes from [opaline](https://github.com/hyperb1iss/opaline).
UI built with [ratatui](https://ratatui.rs) and [egui](https://github.com/emilk/egui).
