# MarketGUI (Rust; formerly volume_screener)

Open items: `todo.md`. User decisions (authoritative) and stream facts: `docs/plan.md`, design: `docs/stream-design.md` (both local only, gitignored: not in the public repo). Behaviour: `README.md`.

## Layout

- `MarketGUI.exe` (`src/main.rs`, Windows GUI subsystem) opens its own window: `src/gui.rs` draws the ratatui UI
  into a `TestBackend` grid and paints it with egui (eframe 0.33). The screener logic runs on its own thread
  (`Arc<Mutex<App>>`), so data keeps flowing while the window is minimized.
- `MarketGUI-cli.exe` (`src/bin/MarketGUI-cli.rs`) is the terminal UI and `--probe`. The package is still named
  `volume_screener`; the exe names come from the `[[bin]]` entries in `Cargo.toml`.
- Everything else is the `screener` library (`src/lib.rs`); startup and worker threads are in `src/launch.rs`.
- Big modules are directories with a root file: `app.rs` + `app/` (fields, views, chart_view, feeds, keys,
  setup, tests/), `engine.rs` + `engine/` (types, feed, candles, alerts, ranking, process, tests/), `ui.rs` +
  `ui/` (boot, dashboard, tables, setup, setup_panes, samples), `stream.rs` + `stream/` (mqtt, proto). Keep
  files under ~600 lines; put tests in the module's `tests` file(s), not inline in big files.
- Trace mode (`src/trace.rs`, overlay `src/ui/debug.rs`): `tlog!(cat, ...)` logs a line, `tnote!` only counts;
  both are a no-op unless tracing is on (debug assertions, `--trace`, `MARKETGUI_TRACE=1`). Log at boundaries,
  once per batch, never per quote. `cargo build --profile debugrel` = fast trace build (README "Debugging").
- Shared helpers (use them, don't re-implement): `nytime` (New York time, session boundary consts, trading
  date, local time to timestamp), `fsutil::write_atomic` (tmp + rename), `net` (HTTP agent, `get_json` inside
  the `limiter` budget), `store::{format_line, parse_line, parse_candles, path}` (candle CSV and file-safe
  symbol names, also used by `barcache` and `levels`), `history::walk_back`, `fmt::{hms, uptime, cycle}`.
- Tests write scratch files under `target/` (`fsutil::test::scratch`), never `%TEMP%`.

## Building without touching anything outside the workspace

While working, check changes with `cargo test` (debug, ~15 s), never `cargo build --release` (2-4 min with
LTO); build release only once at the end, when the exes are to be copied to the root.

Nothing may be written outside the workspace folder that contains this repo (the repo's parent). Run cargo with
the download cache there and rustup auto-install off (bash, from the repo root):
`CARGO_HOME="$(cd .. && pwd -W)/_cache/cargo-home" RUSTUP_AUTO_INSTALL=0 cargo test`. The pinned 1.88
toolchain is installed with rustfmt (approved 2026-11-18). Run `cargo fmt` before every commit (`rustfmt.toml`: width 140).
Clippy is not installed (installing it writes to the user's rustup folder and needs the user's OK).

## Gotchas

- **egui can run one frame twice with the same input** (multi-pass). `gui.rs` only acts on input when
  `ctx.current_pass_index() == 0`; keep it that way or keys fire twice (Enter skipped past the dashboard).

- **Launching the terminal UI from Claude's shell: clear `NO_COLOR` first.** The Claude Code shell sets `NO_COLOR=1`; crossterm
  honours it and drops every colour, so the dashboard renders monochrome and the boot castle (all coloured dot glyphs)
  vanishes. It looks like a broken build but is not. The window app (`MarketGUI.exe`) is not affected. Launch with:
  `Remove-Item Env:NO_COLOR; Start-Process wt.exe -ArgumentList "-d", "<project dir>", "<project dir>\MarketGUI-cli.exe"`
- **Launch the terminal UI in Windows Terminal, not the exe directly.** `Start-Process MarketGUI-cli.exe` opens a classic console
  window, which garbles 24-bit colours and box drawing.
- **Toolchain is pinned to Rust 1.88** (`rust-toolchain.toml`), because `opaline` (themes) uses let-chains. The
  global stable here is 1.85. opaline is used with
  `default-features = false` because its ratatui adapter targets ratatui 0.30 (`ratatui-core`); we are on 0.29 and
  convert `OpalineColor` (plain RGB) ourselves in `src/theme.rs`.
- **The root `MarketGUI.exe` is locked while the app runs.** `cargo build --release` still works; copying
  `target\release\MarketGUI.exe` to the root fails until the user closes the app.

## Architecture: how data travels

```
 Webull push stream (MQTT/WSS)          Webull REST (gainers list, every refresh / 20 s with stream up)
   stream.rs worker thread                 launch.rs fetch worker → scraper::fetch_top_gainers
   (mqtt.rs framing, proto.rs decode)        │
   250 ms batches                             │
        │ Msg::Quotes(Vec<Update>)             │ Msg::Fetched(rows)
        ▼                                      ▼
 ┌────────────────────── mpsc channel of app::Msg ──────────────────────┐
 │  also: Msg::Bars (history workers ×4), Backfill, Levels, Floats,      │
 │        Stream (status ~1/s), Health                                   │
 └───────────────────────────────┬──────────────────────────────────────┘
                                 ▼
   logic thread (gui.rs) every FRAME = 80 ms:  drain channel → App::on_msg,  then App::on_tick
     App (app.rs + app/)   owns Engine, config, UI state; on_tick dispatches worker requests
                           (history, backfill, levels, floats, next REST fetch, stream subscriptions)
     Engine (engine/)      apply_quotes → SeriesStore (series.rs) + live overlay;  process(rows);
                           recompute every 500/1000 ms: rank (Mov), Vol/m, NEW/SETUP alerts, dimming
                           → Output { active rows, blacklisted }, alerts deque, 1m candles
                                 │  Arc<Mutex<App>>
                                 ▼
   UI thread (gui.rs, eframe)  locks App, ui::draw renders ratatui into a TestBackend grid,
                               paints it with egui; input → App::on_key / on_mouse (pass 0 only)
   (MarketGUI-cli: the same App and ui::draw, driven by launch::run_tui in the terminal)

 Disk (%LOCALAPPDATA%\MarketGUI\): config.json, data\candles\ (store.rs), data\bars\ (barcache.rs),
 data\levels\ (levels.rs), data\floats.csv (scraper.rs), probe\ (probe.rs).
 Every Webull REST call goes through limiter.rs (shared 5 req/s budget, 15 min pause on HTTP 403).
```
