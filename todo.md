# TODO

Status as of 2026-11-17: all build steps done. Spec: `docs/plan.md` (user decisions, authoritative) and `docs/stream-design.md`. Handoff: `CONTINUE.md`.

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
- [x] 15. NEW rework (`91007d7`)
- [x] 16. Rate panel tabs (`1590e93`)
- [x] 17. Settings consolidation (`e33d1bc`)
- [x] 18. Candle storage (`270b9f5`)
- [x] 19. Final review + release (`a6ac657`): plan check, README and CONTINUE.md updated, `cargo build --release` (0 warnings), 165 tests pass, exe copied to the project root. Clippy is not installed, so no clippy pass.

## User actions

- [ ] Run `volume_screener.exe --probe` one day from before 11:00 to after 23:00 Riga time, then share the CSV so stream behaviour at session changes can be checked (extended-hours price, volume/high/low resets, status codes, subscription cap).
- [ ] After the probe data: check `MAX_SUBS = 100` against the probe (200 subscriptions were accepted), the step-9 older-session drop rule, and the regular-hours price field (`close`).
- [ ] Run the TUI once and look at it: nothing in the final build has been seen on screen (header indicator, Vol/m, Mov, dimming, rate tabs, Settings, candle files in `%LOCALAPPDATA%\VolumeScreener\data\`).
- [ ] Decide whether the public repo keeps the Webull stream details in `docs/`, or becomes private. Nothing is pushed yet.

## Open questions / later

- Revisit rate-based SPIKE and robust PUMP (design 2.2 / 2.3) if the user wants them back; currently hidden.
- Historical candle backfill from a Webull chart endpoint (unverified) so charts cover time before the app started.
- Stream is an unofficial Webull API; REST fallback must stay working.
- Clippy pass once the component is installed (needs the user's approval: `rustup component add clippy` writes outside the project).
