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
- [x] 8. Stream client (`0d056c3`)
- [x] 9. Series store (`c8163e9`)
- [x] 10. Probe mode (`1b27d6f`)
- [x] 11. Vol/m column (`8b9d374`)
- [x] 12. Ranking (`c49d618`)
- [x] 13. SETUP (`e6dd601`)
- [x] 14. Cooldown + sticky badge (`859037e`)

## Unfinished: build steps in order

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
