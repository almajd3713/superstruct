# Handoff: Phase 5 without audio (2026-10-18)

Read `CLAUDE.md`, then this file, then `CHECKLIST.md`, then `docs/controls.md`. It follows `docs/handoff-phase5.md`. Nothing is pushed; one commit per checklist item on `main`.

## Done this session

| Item | Commit | What | Awaiting |
|---|---|---|---|
| P5.2.1 | `a853690` | `worldgen::sanctuary_room`: 24 m `WARM` room inside the zone, door on a hashed side, 12 m vista opening opposite; `l4::sanctuary_modules`: benches, plants, warm lamps (kit pieces `LampWarm`, `Bench`, `Plant`, warm point lights). `GEN_VERSION` 5 | user review in game |
| P5.4.1 | `e1af5aa` | Tab toggles an altitude and x/z readout (absolute metres) | done |
| P5.4.2 | `a7d36a0` | B drops a 4 m × 600 m `BEACON` pillar, saved to `save.toml` (`SUPERSTRUCT_SAVE`); shader draws material 6 mostly unfogged | done |
| P5.2.2 | `a930857` | Cavity and rim already existed; `print_cavity_rim_viewpoints` finds one | user review |
| P5.4.3 | `fbc4b52` | P photo mode: camera within 40 m, HUD hidden, PageUp/PageDown FOV, `.`/`,` exposure (post pass) | done |
| P5.2.3 | `f104c8e` | `l1_stair`: square spiral of 128-step flights and corner landings through every L1 cell of the column. `GEN_VERSION` 6 | user review |
| P5.2.5 | `f255b9c` | `fartram`: tram cars 3.5 to 60 km away drawn as growing light blocks (material 7) from the real schedule | user review |

Deferred by the user: **all audio (P5.3.x)**. A `kira` dependency was tried and reverted. WSL has no ALSA headers, so the plan is: `kira` with `default-features = false` (its `DefaultBackend` is then the mock backend), and a feature that turns on `kira/cpal` for Windows builds only.

Still open: **P5.2.4** (gravity regions, a decision for the user), **P4.3.4** and the styling work (see the previous handoff), and every older sign-off.

## Things to know

- Pre-existing flake: `streamer::tests::band_radii_change_live` fails now and then (`wanted` 229 against 228 after re-widening). It also failed on `e1af5aa`, which touched only the overlay, so it is not from these changes. `far_to_full_swap...` still fails in release only.
- `scripts/win.sh run` forwards `SUPERSTRUCT_READOUT`, `_SAVE` and `_PHOTO` now.
- Photo mode freezes the player but not lifts or trams, so a ride under the player carries it on.
- The far light trains need the tram clock, which only runs in walk mode.
- Staircase flights are solid wedges with no railings, and the far band and the SDF horizon draw the column as a plain block.
- Sanctuary vista opening faces a random side, not the nearest void.
- Viewpoints: `print_sanctuary_viewpoints`, `print_cavity_rim_viewpoints`, `print_staircase_viewpoints` in `meshpool.rs` tests (run with `cargo test -p engine --release --lib <name> -- --ignored --nocapture`).
