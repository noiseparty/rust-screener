# MarketGUI (Rust; formerly volume_screener)

Read `CONTINUE.md` for the current state and `README.md` for behaviour.

## Layout

- `MarketGUI.exe` (`src/main.rs`, Windows GUI subsystem) opens its own window: `src/gui.rs` draws the ratatui UI
  into a `TestBackend` grid and paints it with egui (eframe 0.33). The screener logic runs on its own thread
  (`Arc<Mutex<App>>`), so data keeps flowing while the window is minimized.
- `MarketGUI-cli.exe` (`src/bin/MarketGUI-cli.rs`) is the terminal UI and `--probe`. The package is still named
  `volume_screener`; the exe names come from the `[[bin]]` entries in `Cargo.toml`.
- Everything else is the `screener` library (`src/lib.rs`); startup and worker threads are in `src/launch.rs`.

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
  global stable here is 1.85; `CONTINUE.md`'s "keep deps 1.85-compatible" note is superseded. opaline is used with
  `default-features = false` because its ratatui adapter targets ratatui 0.30 (`ratatui-core`); we are on 0.29 and
  convert `OpalineColor` (plain RGB) ourselves in `src/theme.rs`.
- **The root `MarketGUI.exe` is locked while the app runs.** `cargo build --release` still works; copying
  `target\release\MarketGUI.exe` to the root fails until the user closes the app.
