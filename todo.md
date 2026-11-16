# TODO

Status as of 2026-11-16. Spec: `docs/plan.md` (user decisions, authoritative) and `docs/stream-design.md`.

## Done (committed)

- [x] 1. Alert flash fix (`0072564`)
- [x] 2. No alerts for blacklisted symbols (`c3a2ee2`)
- [x] 3. History and candles recorded before display filters (`bf67fb1`)
- [x] 4. Mov sort fix + symbol tiebreak (`9908be1`)
- [x] 5. Cursor follows selected symbol (`c915e4b`)
- [x] 6. `s` / config save re-sort locally, no fetch (`d8e8a5e`)
- [x] 7. App home dir (`5fc469a`)

## Unfinished: build steps in order

- [ ] 8. **Stream client** (item 8): `src/stream.rs`, MQTT over WSS, protobuf decode, subscribe unfiltered gainers by tickerId, 250 ms batches, reconnect/backoff, REST list every 20 s, REST fallback poll when stream down, header `● WSS n subs m msg/s` / `● REST, stream down`, `stream_enabled` config.
- [ ] 9. **Series store**: per-ticker receive-time series with ingest rules, `vol_at` / `px_at`, clear rules (session/date change, reconnect, source switch, 60 s linger).
- [ ] 10. **Probe mode** (item 7): `volume_screener.exe --probe` headless recorder → `probe\probe-YYYY-MM-DD.csv`.
- [ ] 11. **Vol/m column** (item 9): replaces Δ volume column.
- [ ] 12. **Ranking** (items 17, 18, 19): recompute tick 500/1000 ms (setting), Mov = rank change on the full board by % change (cyan `+` for new entrants), filter grace: dimmed 60 s then hidden.
- [ ] 13. **SETUP** (item 13): user's proven logic (vol ratio + price % vs N seconds ago), all three adjustable (defaults 300 s / 2.2x / +10%), config key migration, hide SPIKE and PUMP.
- [ ] 14. **Cooldown + sticky badge** (items 12, 15): 2 min cooldown per symbol+kind, status label stays visible during cooldown and ≥ 60 s.
- [ ] 15. **NEW rework** (item 14): listed now and not in last 30 min; silent seeding on startup / session change / filter change.
- [ ] 16. **Rate panel tabs** (item 16): `t` toggles New Highs/Lows (default) and Up/Down ticks; tab remembered in config.
- [ ] 17. **Settings consolidation** (item 20): Fallback Poll, SETUP fields, Alert Cooldown, Recompute (ms), Vol/m Window, named field lookups in preview.
- [ ] 18. **Candle storage** (item 21 part B): append 1-minute candles to `data\candles-YYYY-MM-DD.csv`, reload today's on start, delete previous days on launch.
- [ ] 19. **Final review + release**: check against plan, clippy, README, `cargo build --release`, copy exe to project root.

## Resume

Re-run the build workflow from step 7. Saved script:
`C:\Users\Testweek2\.claude\projects\C--Users-Testweek2-Desktop-Web-TOOLS-volume-screener-rust\0665e736-f904-4931-96fb-f3e901274c41\workflows\scripts\stream-rework-build-wf_6ff531bb-2fc.js`
(resume run `wf_6ff531bb-2fc` replays steps 1–6 from cache, same session only; in a new session, remove steps 1–6 from the script or let them no-op on the committed tree.)

## User actions

- [ ] Close the running `volume_screener.exe` before the release step; the copy in the project root is the original pre-UI build and is locked while running.
- [ ] After step 10: run `volume_screener.exe --probe` one day from before 11:00 to after 23:00 Riga time, then share the CSV so stream behaviour at session changes can be checked (extended-hours price, volume/high/low resets, status codes, subscription cap).

## Open questions / later

- Revisit rate-based SPIKE and robust PUMP (design 2.2 / 2.3) if the user wants them back; currently hidden.
- Historical candle backfill from a Webull chart endpoint (unverified) so charts cover time before the app started.
- Stream is an unofficial Webull API; REST fallback must stay working.
