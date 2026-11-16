# Continue here

Handoff (updated 2026-11-17, after step 19). Read this, then `todo.md`, then `docs/plan.md`.

## What this is

Rust TUI stock screener (`volume_screener`), a port of the Go `volume_checker_v1.2`. It shows Webull top gainers from the live push stream (MQTT over WSS, REST fallback), raises NEW and SETUP alerts and renders a highlowtick.com-style dashboard. `README.md` describes the finished behaviour.

- Repo: `C:\Users\ZX202\kodi\rust-screener`, branch `master`. Not pushed; push only when the user asks. The GitHub remote https://github.com/noiseparty/rust-screener is **public**.
- Specs: `docs/plan.md` = user decisions (**authoritative**); `docs/stream-design.md` = technical design. Where they differ, `plan.md` wins.

## State

All build steps 1-19 are done and committed (see `todo.md`). `cargo build --release` has 0 warnings and all tests pass. The release exe is `volume_screener.exe` in the project root (gitignored; rebuild with `cargo build --release` and copy `target\release\volume_screener.exe`). Clippy is not installed for the toolchain, so no clippy pass was done (installing it is a `rustup component add` outside the project and needs the user's approval).

## Untested on screen

Only unit tests have run. The TUI and the live stream have not been run against real data in the final build:

- Header indicator, Vol/m column, Mov, dimmed rows, sticky Status badge, rate tabs (`t`), Settings screen with the new fields.
- NEW / SETUP alerts firing live, including the cooldown.
- Candle files being written to `%LOCALAPPDATA%\VolumeScreener\data\`, reloaded on restart and purged the next day.
- Config move from the current directory to the app home on first run.
- Stream reconnect and REST fallback behaviour over a long session.

## Waiting on the user

1. Run `volume_screener.exe --probe` for a full day (before 11:00 to after 23:00 Riga time) and share `probe\probe-YYYY-MM-DD.csv`.
2. Use the probe data to check:
   - `MAX_SUBS = 100` (src/engine.rs) is an assumption; an earlier probe showed 200 subscriptions accepted. Raise or keep the cap accordingly.
   - The step-9 rule that drops quotes from an older session (series store ingest rules) against what the stream sends at session changes.
   - Whether the regular-hours price field is right (`close` is used in the regular session and `pPrice` in extended hours; the regular-hours choice is unverified).
3. Decide whether the public repo should keep the Webull stream details in `docs/`, or become private.

## Key user decisions (short form; full table in docs/plan.md)

- **SETUP is the main signal**: volume ratio >= X **and** price change >= Y% versus N seconds ago (defaults 300 s / 2.2x / +10%).
- **SPIKE and PUMP alerts are hidden for now.** The enum variants stay.
- 2 min alert cooldown; Status label held for the cooldown and at least 60 s.
- NEW = on the gainers list now and not in the last 30 min; no startup flood.
- Rate panel tabs with `t`: New Highs/Lows (default) and Up/Down ticks.
- Recompute/re-rank every 500 or 1000 ms. Mov = rank change on the full board. A row that stops passing a filter stays dimmed 60 s.
- No installer. Config and data in `%LOCALAPPDATA%\VolumeScreener\`; 1-minute candles saved daily, previous days deleted on launch.
- Sessions in Riga winter time: pre 11:00, regular 16:30, after 23:00; code uses America/New_York so DST is automatic.

## Stream facts (verified live)

`wss://wspush.webullfintech.com:443/mqtt`, subprotocol `mqtt`, `Origin: https://webull.com`, MQTT 3.1.1, user/pass `test`/`test`. Subscribe to the header topic first, then to `{"tickerIds":[...],"type":"102","flag":"1"}`. Payload is protobuf `ItemsCombo`; the field map is in `docs/plan.md`.

Anonymously the stream gives price, change, % change, cumulative day volume, open/high/low, market cap and a ms timestamp. It gives no bid/ask, individual trades or 52-week hi/lo. The REST gainers response includes `tickerId`. It is an unofficial API; the REST fallback must keep working.

## Environment gotchas

- The default rustc is 1.70. `rust-toolchain.toml` selects `stable` (1.85). Keep dependencies compatible with 1.85; `resolver = "3"` is set.
- Many `src` files and the docs use CRLF line endings; keep them (use the Edit tool, not `sed -i`).
- `config.json` is gitignored: it contains the Finviz proxy key. Never commit it.
- Art in `src/art.rs` / `src/chart.rs` is ported from ASCII Magic (MIT); the notice is in `THIRD_PARTY_LICENSES.md`.
- Nothing outside `C:\Users\ZX202\kodi\` may be changed without the user's permission (see the user's global CLAUDE.md).

## Later ideas

- Rate-based SPIKE and robust PUMP (design 2.2 / 2.3) if the user wants them back.
- Historical candle backfill from a Webull chart endpoint (unverified), so charts cover time before the app started.
