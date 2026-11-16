# Continue here

Handoff for the next session (written 2026-11-16). Read this, then `todo.md`, then `docs/plan.md`.

## What this is

Rust TUI stock screener (`volume_screener`), a port of the Go `volume_checker_v1.2`. It polls Webull top gainers, raises alerts and renders a highlowtick.com-style dashboard. The current work replaces 5 s polling with Webull's live push stream (WSS) and reworks the features around tick-level data.

- Repo: `C:\Users\Testweek2\Desktop\Web\TOOLS\volume_screener_rust`, branch `master`, remote https://github.com/noiseparty/rust-screener (**public**).
- Specs: `docs/plan.md` = user decisions (**authoritative**); `docs/stream-design.md` = reviewed technical design. Where they differ, `plan.md` wins.
- Task list: `todo.md` (steps 1–6 done, 7–19 open).

## Where we stopped

- Steps 1–6 (REST-path bug fixes) are committed and pushed.
- Step 7 (app home dir `%LOCALAPPDATA%\VolumeScreener\`) was interrupted. Its partial work is in `git stash` ("partial step app-home (interrupted)"). It had added `src/paths.rs` and edited `main.rs`, `config.rs`, `app.rs`, `README.md`. Decide: `git stash pop`, check that it builds and matches plan row 21, then finish and commit; or `git stash drop` and redo.
- The build workflow was stopped by the user, not by a failure.

## How to resume

1. `git status` must be clean (apart from the stash). Run `cargo build` and `cargo test`; both should pass.
2. Re-run the build workflow from step 7. Saved script:
   `C:\Users\Testweek2\.claude\projects\C--Users-Testweek2-Desktop-Web-TOOLS-volume-screener-rust\0665e736-f904-4931-96fb-f3e901274c41\workflows\scripts\stream-rework-build-wf_6ff531bb-2fc.js`
   - Cached replay (`resumeFromRunId: wf_6ff531bb-2fc`) only works in the same Claude session. In a new session, edit the script's `STEPS` array to remove the six `Fixes` entries, then run it.
   - Agreed setup: one **Sonnet** agent per step, **sequential** (the steps share `engine.rs` / `app.rs` / `ui.rs` / `config.rs`, so they can't run in parallel). Each step must build with zero warnings and pass tests, then commit locally. Stop on the first failure.
   - Expect about 2–4 hours for steps 7–19.
3. The final step builds the release exe and copies it to the project root. **The user must close the running `volume_screener.exe` first** (the file is locked while it runs).
4. Push to GitHub only when the user asks.

## Key user decisions (short form; full table in docs/plan.md)

- **SETUP is the main signal.** Keep the user's proven logic: volume ratio ≥ X **and** price change ≥ Y% versus N seconds ago. All three are adjustable (defaults 300 s / 2.2x / +10%).
- **SPIKE and PUMP alerts are hidden for now.** Keep the enum variants.
- 2 min alert cooldown; the Status label stays visible during the cooldown and for ≥ 60 s.
- NEW = joined the gainers list and wasn't on it in the last 30 min; no startup flood.
- Rate panel tabs, toggled with `t`: **New Highs/Lows** (default) and Up/Down ticks.
- Recompute/re-rank every 500 or 1000 ms, set in Settings.
- Mov = rank change on the full board by % change, independent of the user's sort and filters.
- A row that stops passing a filter stays visible **dimmed for 60 s**, then hides.
- No installer. Config and data live in `%LOCALAPPDATA%\VolumeScreener\`. 1-minute candles are saved daily, and previous days' files are deleted on launch.
- Sessions in Riga time (winter): pre 11:00, regular 16:30, after 23:00. Code computes sessions in America/New_York, so DST is automatic.

## Stream facts (verified live)

`wss://wspush.webullfintech.com:443/mqtt`, subprotocol `mqtt`, `Origin: https://webull.com`, MQTT 3.1.1, user/pass `test`/`test`.
1. Subscribe to the header topic first.
2. Then subscribe to `{"tickerIds":[...],"type":"102","flag":"1"}`.

The payload is protobuf `ItemsCombo`. Full field map is in `docs/plan.md`.

What the stream provides anonymously:
- Yes: price, change, % change, cumulative day volume, open/high/low, market cap, ms timestamp.
- No: bid/ask, individual trades, 52-week hi/lo.

The REST gainers response includes `tickerId`.

The working Python probes are in the old scratchpad and may be gone. `docs/plan.md` has everything needed to reimplement them.

## Environment gotchas

- The default rustc is 1.70. `rust-toolchain.toml` selects `stable` (1.85). Keep dependencies compatible with 1.85; `resolver = "3"` is set.
- Long inline Python heredocs in the Bash tool can get truncated. Write scripts to a file, or use the Edit tool.
- `config.json` is gitignored: it contains the Finviz proxy key. Never commit it.
- Art in `src/art.rs` / `src/chart.rs` is ported from ASCII Magic (MIT); the notice is in `THIRD_PARTY_LICENSES.md`.

## Waiting on the user

- After step 10 exists: run `volume_screener.exe --probe` for a full day (before 11:00 to after 23:00 Riga time) and share the CSV.
- Decide whether the public repo should keep the Webull stream details in `docs/`, or make the repo private.
