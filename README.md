# MarketGUI (Rust)

Rust port of `volume_checker_v1.2` (Go). Shows Webull top gainers live and raises NEW and SETUP alerts in a
highlowtick-style UI built with ratatui, in its own window (or a terminal). Quotes come from Webull's push stream (MQTT over WSS); REST polling is the
automatic fallback.

## Build / run

```
cargo build --release
target\release\MarketGUI.exe        the app window
target\release\MarketGUI-cli.exe    the same UI in a terminal, and --probe
cargo test
```

`rust-toolchain.toml` pins Rust 1.88. `MarketGUI.exe` opens a window (no console): the UI is drawn with Cascadia
Mono (else Consolas); Ctrl+mouse wheel or Ctrl +/- change the text size, Ctrl+0 resets it (saved as `font_size`). Data
keeps updating while the window is minimized. Usage of the console build:
`MarketGUI-cli [--probe [--probe-dir <dir>]]`; any other argument prints the usage line and exits.

## Stream mode

- The REST top-gainers list is refreshed every 20 s and says which tickers to watch. The app subscribes to those
  tickers (up to 100, current list first, recently dropped ones linger briefly) on `wss://wspush.webullfintech.com`
  and updates price, % change, volume, market cap and day high/low from each push message.
- If the stream is down, the app polls the REST list every `Fallback Poll (s)` seconds until it reconnects. Set
  `"stream_enabled": false` in `config.json` to use REST polling only.
- The stream is an unofficial Webull API and may change without notice. It carries no bid/ask, no individual trades
  and no 52-week high/low.
- Header indicator (top right): green `● online m Hz` is the stream receiving m messages per second; amber
  `● online m Hz` is the stream up with a failed list refresh; amber `● REST` is the REST fallback; red `● offline`
  is neither working; amber `● connecting` is startup; green `● online` (no Hz) is REST-only mode (stream disabled).

## App home

Config and data live in `%LOCALAPPDATA%\MarketGUI\`. On launch, if that folder is missing and the
old `%LOCALAPPDATA%\VolumeScreener\` exists (the app's name before MarketGUI), the old folder is renamed, so config
and data carry over; if the rename fails the old folder is left as it is and the error shows in the header:

```
config.json   settings (contains the proxy key; never commit it)
data\candles\ <SYMBOL>.csv, saved 1-minute candles (one file per ticker)
data\bars\<tf>\ <SYMBOL>.csv, cached 5m / 1h / 4h / D chart bars (tf = 5m, 1h, 4h, D)
data\levels\  <SYMBOL>.csv, 4h swing-level bar cache
presets\     <name>.json, saved Config presets
probe\        probe recordings
```

The folders are created on first run. If `config.json` exists in the current directory and not in the app home, it is
moved there once. No installer is needed.

## Candle storage

Completed 1-minute candles are appended to one file per ticker, `data\candles\<SYMBOL>.csv` (columns
`start_epoch_secs,open,high,low,close,volume`, no header). A ticker's file is read the first time it is on the list in
a run (the newest 2000 candles, about two extended-hours sessions), so its 1m chart goes back across restarts and days.
Nothing is deleted automatically and there is no size cap: Config -> Storage shows the files and their sizes and
deletes what is no longer needed. Day files from older versions (`candles-YYYY-MM-DD.csv`) are moved into the
per-ticker files once. A failed write is retried with the next batch.

## Config

`o` / `c` on the dashboard (or Config on the start screen) opens one screen: a sidebar on the left (Configuration,
Presets, Theme, Time & Format, Blacklist, Alert Filters, Sound, Storage, Health Check, with the selected item's description and the animated
gear at its bottom), the selected item's settings next to it, and a preview on the right for Configuration (the filter
preview), Presets (the preset's values and its filter preview), Theme (a mini dashboard), Time & Format (sample
alert row, chart labels, level tag, prices and volumes in the formats being edited), Blacklist (the
blacklisted tickers with price and change) and Alert Filters (the dashboard alerts panel with those filters and rows:
the recent alerts, or sample alerts of each kind before any fired). Up/down in the sidebar shows each item live; Enter, -> or Tab moves into
its settings, Esc goes back to the sidebar (in Configuration it also drops unsaved changes, in Theme a previewed
theme), and Esc in the sidebar leaves.
Clicking a sidebar item selects it; clicking the selected one opens it. One hint bar at the bottom shows the keys of
whatever has focus.

### Presets

Presets are named Configuration settings, saved as `presets\<name>.json` in the app home (a whole config; only the
Configuration fields and the Time & Format settings are taken when one is loaded). The built-in `Recommended` preset is listed first and cannot be
deleted or overwritten; saved ones follow by name. Up/down selects (the preview shows its values), Enter loads it into
the Configuration form (previewed with its formats, Enter there saves both, Esc drops them; the theme is never changed
by a preset; Recommended keeps the formats in use). The preview's last line shows the preset's formats. The
last row, **Save current config as…**, previews the config in use; Enter on it asks a name (letters, digits, space,
`_` and `-`, up to 24 characters; an existing name asks y to replace). `d` deletes the selected preset after a y. `p`
in Configuration still loads Recommended.

### Time & Format

How times, dates and numbers are shown. Up/down moves, <- / -> picks (space too on the choice rows), Enter saves
(`formats` in the config), Esc drops the changes; while the item is selected the preview and the whole Config screen
use the formats being edited.

- **Time zone**: Local (default), New York, Chicago, Los Angeles, London, Berlin, Riga, Tokyo, Hong Kong, Singapore,
  Sydney, UTC, or type any IANA name (e.g. `Europe/Vilnius`; Del clears, an unknown name cannot be saved). It applies
  to alert times, the chart time axis, level dates and Storage dates. Session phases, the countdown, candle
  boundaries and the daily candle files stay New York based.
- **Date**: `DD.MM.YY` (default), `DD/MM/YYYY`, `MM/DD/YY`, `MM-DD-YYYY`, `YYYY-MM-DD`, or type a pattern with `DD`,
  `MM`, an optional `YY` or `YYYY` and any separators. The chart's 4h/D axis shows day and month in that order.
- **Clock**: 24h (`14:05`) or 12h (`2:05pm`).
- **Currency** `$`, `€`, `£` or none, **Symbol** before or after the number (level tags).
- **Decimal** `.` or `,` and **Thousands** none, `,`, `.` or space (prices, chart axis, volumes); the two must differ.
- **Price decimals**: Auto (2, or 4 under 1), always 2 or always 4 (price cells, ticker tape, level tags).
- **Volume**: abbreviated (`18.40M`) or full digits (volume, market cap and Vol/m cells).

### Alert Filters

On/off for the alert types shown under ALL and in the `f` cycle, and a last row **Rows on dashboard ◂ 5 ▸**
(5-15; <- / -> or space change it, saved at once). It is the same setting as Alert Rows in Configuration.

### Sound

The alert sound (`assets\alert.wav`, built into the exe) plays once per tick when new alerts appear and at least one
of them passes the current alert filter (the `f` filter, and under ALL the Alert Filters toggles). **Mute** (Enter or
space toggles) and **Volume** 0-100 (default 70; <- / -> step by 5, drawn as a bar; Enter or space plays a test at
that volume, even when muted). Both are saved at once (`sound_muted`, `sound_volume` in the config).

### Health Check

Two groups with aligned columns. **Connections**: the Network and Stock Data Source checks (Enter runs them again).
**System**: app uptime, RAM used / total, this app's RAM, CPU % (system-wide, sampled every second), logical cores,
the `data` folder size, free disk space on the app home's drive, Webull requests in the last minute (or the time left
of a 403 pause), the stream's rate and subscriptions, and the OS version. Values that cannot be read show `n/a`.

## Storage

Config -> Storage is a file browser over the app home: folders first, files largest first, with sizes and dates.
Enter/-> opens a folder, Backspace/<- goes up (at the top: back to the sidebar), d or Delete deletes the file or folder under the cursor after a y to
confirm. `config.json` is never deleted. The header shows the size of the `data` folder in MB (`storage 12 MB`).

## Probe mode

```
MarketGUI-cli.exe --probe [--probe-dir <folder>]
```

Headless recorder (no TUI) for checking how the stream behaves across session changes: extended-hours price fields,
volume/high/low resets, status codes, `tradeStamp`, and the subscription cap. It refreshes the gainers list over REST
every 20 s (asking for 200 rows, so more than 100 are subscribed), subscribes to all of them and appends every raw
message and connection event to `probe\probe-YYYY-MM-DD.csv` in the app home (date = local start date; a restart on the
same day continues the same file). It reconnects like the app and idles while the market is closed. A status line is
printed every 30 s; `Ctrl+C` stops it and flushes the file (the file is also flushed every second).

Leave it running from before 11:00 Riga (pre-market) through 23:00 and, to see the after-hours session, past it. Then
share the CSV. `--probe-dir` writes somewhere other than `%LOCALAPPDATA%\MarketGUI\probe\`.

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
header   Full List Chart Alerts  <phase>   <until> HH:MM:SS   ● online n Hz  up 3h 12m  storage n MB  RAM n%
main     Alerts (last 5-15)                    |  Chart of the selected row
         Gainers table (zebra, sortable)       |  (+ blacklisted box)
footer   key hints
tape     scrolling top-30 gainers (alerted symbols highlighted)
```

The tabs at the left of the header (or `w`, which cycles them) pick the dashboard layout, saved as `layout` in
`config.json`: **Full** (alerts and list, the chart beside them), **List** (alerts and list, full width), **Chart**
(the chart only) and **Alerts** (the alerts only, full height; ↑/↓, PgUp/PgDn, `g` and the wheel scroll them). The
chart keys (`1`-`5`, `t`, `0`) act only in Full and Chart. Configuration -> LAYOUT switches the ticker tape and the key
hints off (their rows go to the main area) and sets the alert panel's rows (5-15, default 5).

Header phases and countdowns (New York time, Riga winter time in brackets): `Pre Market` 04:00-09:30 (11:00-16:30),
`open in`; `Regular Market` 09:30-10:30 (16:30-17:30), `zombie hour in`; `Regular - Zombie Hour` 10:30-11:30
(17:30-18:30), `ends in`; `Regular Market` 11:30-16:00 (18:30-23:00), `close in`; `After Market` 16:00-20:00
(23:00-03:00), `closes in`; otherwise `Market Closed`, `pre-market in`; weekends count to Monday's pre-market (US
holidays are not known). The gainers panel title shows only the ticker count. RAM is the PC's physical memory in use,
in percent (amber from 75%, red from 90%).

In Full the chart hides when the window is narrower than the list plus 44 columns; the alerts panel hides when the
list would keep fewer than 5 rows; the tape hides below 16 rows. A narrow header drops its status items from the end,
then the phase name, then the tabs: at the narrowest only the countdown and the status dot stay.

Boot screen: animated moonlit mausoleum. Config: animated gear and wrench at the bottom of the sidebar, scaled to fit
(box-averaged, down to a quarter size; hidden below that). Both are halftone dots: dot size follows brightness (the
scene) or density (the gear). The window paints them as smooth round dots, two per text cell; the terminal shows one
dot glyph per cell (` · ∙ • ●`). Art ported from ASCII Magic, MIT, see `THIRD_PARTY_LICENSES.md`.

### Gainers table

Columns: Mov, Symbol, Price, % Chg, Volume, Mkt Cap, Float, Shares, Vol/m, Status. Float and Shares are off by default
(switch them on with `v`).

- **Mov**: change in rank on the full gainers board (ranked by % change) over `Rank Track (s)` (default 60), independent
  of your sort and display filters. A green `▲n` / red `▼n` is places gained / lost, a dim `·` is unchanged, a cyan `+` is
  a new entrant, blank is no history yet.
- **Vol/m**: average shares per minute over the last `Vol/m Window (s)` seconds (default 180; header shows e.g.
  `Vol/m 3m`), computed from the stream's cumulative volume. `…` means too little history (needs 30 s), `—` means the
  feed is stale, a dim value means the window is only partly covered.
- **Float** / **Shares**: Webull's float (`outstandingShares`) and total shares, `N/A` when unknown (ETFs, or not fetched
  yet). Fetched for the gainers list in batches of 20 ids per request, once per New York day, and cached in
  `data\floats.csv` (`id,float,total,date`; rows older than 30 days are dropped). Webull's float can lag a fresh
  offering or reverse split.
- **Status**: the alert label (NEW or SETUP) of the row. It is held for the whole alert cooldown and at least 60 s.
- The table re-ranks and re-sorts every `Recompute (ms)` (500 or 1000). A row that stops passing a display filter stays
  visible, dimmed, for 60 s before it hides.

### Alerts

- **SETUP** (the main signal): fires when the shares traded in the last `SETUP Window (s)` are at least
  `SETUP Volume Ratio` times the shares traded in the window before it **and** price is up at least `SETUP Price %`
  on one window ago. Defaults: 300 s, 2.2x, +10% (the last 5 minutes versus the 5 minutes before). Windows, not the
  day's total volume, so a ticker that spikes again late in the day still fires. After a window without trades any
  volume counts (Detail shows "vol from 0"). It also needs `Min Vol Alert` and `Min Price Alert ($)`. A previous
  window only partly seen (the app watched the ticker for less than two windows) is scaled to a full window; with
  less than one window plus a tenth of one, nothing fires. Detail shows the window ratio. Checked on every recompute tick.
- **NEW**: the symbol is on the gainers list now and was not on it in the last 30 minutes. The first list after
  startup, a session change or a display-filter change only seeds the memory, so nothing fires then.
- **SPIKE and PUMP are hidden for now.** They are not raised and do not appear in the alert filter cycle or the Alert
  Filters screen; the enum variants remain in the code so they can return.
- Each symbol and alert kind has a cooldown (`Alert Cooldown (s)`, default 120). Blacklisted symbols raise no alerts.
- Highlight colors: NEW yellow, SETUP purple, newest alert flashes green.

## Chart

The right side of the dashboard charts the selected row; `Enter` opens the same chart full screen (↑/↓ switches
ticker, Esc back). Timeframes 1m / 5m / 1h / 4h / D: click the tabs in the chart title or press `1`-`5`.

- **1m** candles are built from every price/volume sample the app receives (up to 390 per ticker) and are saved to disk
  (see Candle storage), so after a restart the chart covers today's saved candles plus the time since.
- **5m / 1h / 4h / D** come from Webull's chart endpoint (48 bars per chart). Every ticker on the filtered list is
  preloaded in the background, top to bottom, D / 1h / 4h / 5m, on 3 worker threads; a ticker that joins the list is
  loaded when its turn comes. A 4th worker serves the charted ticker first: its chart once it has been selected for
  1 s, then its newest bars every 15 s; the live price keeps the bar in progress current. Intraday bars cost one
  request each anonymously (about 10 s for a 48-bar chart, filled in 8 bars at a time); daily bars come in one
  request. Bars are cached on disk (`data\bars\<tf>\<SYMBOL>.csv`, rewritten when a fetch is merged), so after a
  launch a cached chart shows at once and is only topped up with the bars since.
- **Move the chart** like TradingView: drag the plot sideways and up/down, and it follows the pointer. Dragging right
  shows older bars until a quarter of the plot still shows bars; dragging left leaves empty space after the newest bar
  (up to half the plot). A history shorter than the plot sits at the right edge. Near the oldest bar held, 48 older
  bars are fetched (1m too, from Webull). Drag the price axis down/up to widen/narrow the price range; double-click it
  to fit again (also drops a vertical move). `0` resets the view; switching ticker or timeframe starts at the newest
  bars, fitted.
- **4h swing levels** (every timeframe but D): only the 3 untaken levels nearest the live price are drawn; levels price
  has already traded above are all drawn dimmed. There is no distance window: an untaken level outside the shown price
  range is a tag on the plot's top or bottom edge instead (e.g. `▲ 17.11.26 ($3.86)`). Each line is labelled with
  the date and price of the high that set it, by default `DD.MM.YY ($D.CC)` (4 decimals under $1, e.g. `($0.1234)`;
  date, currency and decimals follow Time & Format). A failed
  request drops its batch, which is asked again later, so the cached year has no holes.
- **Price tag and candle countdown**: the live price sits on the price axis as a filled block in the up/down colour
  (like TradingView), and under it a second block with the time left in the newest candle: `mm:ss` on 1m/5m,
  `hh:mm:ss` on 1h/4h, `Nd hh:mm` on D (to the next New York midnight). Bars cut at the sessions (e.g. the 4h bar
  08:00-09:30) count down to the cut. The countdown is hidden while the chart is scrolled back past the newest bar.

## Themes

`o` → **Theme** (sidebar row after Presets): up/down browses the list and the whole screen previews each theme, with
a mini dashboard on the right (gainers rows with alert badges and the charted or first ticker's chart with volume;
sample rows and candles while there is no data). Enter keeps and saves it; Esc, or moving to another sidebar item,
drops the preview. Stored as `theme` in `config.json`. **Dark Noir** (`dark-noir`, default) is the original look;
the other 39 come from [opaline](https://github.com/hyperb1iss/opaline) (Catppuccin, Tokyo Night, Gruvbox, Nord,
Dracula, Rose Pine, Solarized, ...; ids like `catppuccin-mocha`). Themes paint the app background; Dark Noir keeps
the terminal's. The boot animation and the setup gear keep their own colours under every theme.

## First run

Without a `config.json` the screener does not start fetching right away: Start opens Configuration with the display
filters focused and a live preview from one list fetch. Every ticker that passes the filters is preloaded, so set them
to keep the list short. `p` loads the Recommended preset. Enter saves and starts (that first save creates
`config.json`); Esc goes back to the start screen, and the first-run setup comes again next launch.

## Start screen
The menu (Start MarketGUI, Config) is centred over the animated Taj Mahal. The connection checks (Network, Stock Data Source) run at launch and their results show under the menu in aligned columns, with the version (`MarketGUI v1.0.0 (Taj Mahal)`). They can be run again from Config -> Health Check. Each box is only as wide as its content, so more of the scene shows around them.

## Window

`MarketGUI.exe` has no Windows title bar; the app draws its own, thin, in the theme's style: drag it to move the
window, double-click to maximize, minimize/maximize/close buttons on the right, drag any window edge to resize. The
text rows share the rest of the window height, so the grid fills it exactly (the ticker tape is flush with the bottom
edge). The window can be made as small as 300x200 px.

## Mouse

Click a row to select and chart it (`Enter` opens the full chart). Click a column header to sort by it (again
to flip), an alert to jump to its ticker, a layout tab or a timeframe tab to switch, a footer hint to run it. Drag the chart to move
it (both ways), the price axis to scale. The wheel over the chart zooms it in time like TradingView (up = closer, down = further
out, down to 16 bars per column); over the alerts it scrolls them; elsewhere the wheel does nothing. A click on an alert for a ticker that is no longer
on the list opens its full chart. In Config, a click on a sidebar item selects it (on the selected one, opens it).

Backfill: for every alerted symbol, every ticker on the filtered list and the selected row, the app fetches up to 10
one-minute bars from before its first candle (Webull chart endpoint, one bar per request anonymously, same trading day
only). Each symbol is backfilled once per run; a failed request is tried again after 60 s. This gives a new spiker a few minutes of chart before it
reached the list.

Launch gap fill: when a ticker's saved 1m candles end before its first live candle today (the app was closed), the
minutes in between are fetched, walking back from the first live candle to the newest saved one (or to 04:00 New York
when nothing is saved for today); charted ticker first, then the list top to bottom, one job at a time. Backfilled
and gap-filled bars are written to the ticker's candle file at once (out-of-order rows are sorted and the file
compacted when it is next read).

## Keys

| Key | Action |
|-----|--------|
| `Enter` | candlestick chart for selected row |
| `←/→` `h/l` | select column |
| `s` | sort by selected column (again to flip); re-sorts locally, no fetch |
| `↑/↓` `j/k`, PgUp/PgDn, `g` | move row cursor (follows the selected symbol) |
| `f` | cycle alert filter (ALL/NEW/SETUP, enabled ones only) |
| `w` | next layout: Full, List, Chart, Alerts |
| `1`-`5` | chart timeframe: 1m, 5m, 1h, 4h, D (`t` = next) |
| `0` | reset chart scroll, price scale and time zoom |
| `v` | column visibility |
| `o` / `c` | Config (sidebar: configuration with live filter preview, presets, blacklist, alert filters, storage, health check) |
| `r` | refresh the gainers list now |
| `q` / `Ctrl+C` | quit |

Configuration: Tab/↑/↓ to move, type to edit, ←/→ for choices, `p` for the Recommended preset, Enter or Ctrl+S to save, Esc to cancel (back to the sidebar). Saving re-sorts and re-filters
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
| SETUP Volume Ratio | last window's volume versus the window before it (2.2) |
| SETUP Price % | price rise needed, 10 = +10% (10) |
| Min Vol Alert / Min Price Alert ($) | gates before SETUP can fire (10000 / 1.0) |
| Alert Cooldown (s) | per symbol and kind (120) |
| Min / Max % Change, Volume, Price | display filters (0 = off) |
| Min / Max Float, Shares | display filters on float / total shares; K/M/B suffixes allowed (0 = off). A ticker whose count is unknown passes |
| Ticker Tape / Key Hints | LAYOUT: show the tape / the key hints on the dashboard (On) |
| Alert Rows | LAYOUT: rows of the dashboard alerts panel, 5-15 (5) |

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
- Finviz columns dropped (they were never populated); `finviz_*` config keys are kept but no longer checked by Health Check.
- `column_order` and `finviz_*_seconds` config keys removed (unused).
- The Δ volume column became Vol/m; SPIKE and PUMP alerts are hidden; the live push stream replaces 5 s polling.
