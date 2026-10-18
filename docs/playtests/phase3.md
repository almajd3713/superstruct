# Phase 3 exit: performance pass and playtest (P3.5.1)

Machine: RTX 4070 Laptop GPU, native Windows (GNU toolchain), release build, 1600x900 window with vsync on (144 Hz panel). Numbers below were taken by me with the F1 overlay's GPU timestamp zones, the headless GPU tests and `SUPERSTRUCT_AUTOFLY`; **no Tracy capture has been taken** (no profiler on this machine), so that part is yours.

## Budgets from PLAN.md §11.4

| Pass | Budget | Measured | How |
|---|---|---|---|
| Culling (frustum, occlusion phase 2, Hi-Z) | 0.5 ms | 0.07 ms (cull 0.003-0.005, occlusion 0.004, hiz 0.057) | overlay at the test spot |
| Geometry (depth prepass and shading) | 6 ms | 0.4 ms (prepass 0.014, scene 0.41 including the horizon test) | overlay at the test spot |
| SDF horizon, full resolution | 2 ms | 1.0 ms at 2560x1440, clear air, every pixel hits | `horizon_cost` |
| SDF horizon, half resolution | 2 ms | 0.5 ms (march 0.32 + upsample 0.17) | `horizon_cost` |
| Horizon cell-record refill | n/a | 0.54 ms, once per 4 km of travel | `horizon_cost` |
| Volumetric fog | 2 ms | not built (Phase 5) | |
| Post-processing and stylization | 1.5 ms | not built (Phase 4.3) | |
| CPU main thread | 4 ms | `Renderer::render` 0.55 ms recording 19.6k far cells; `Streamer::update` under 0.001 ms steady, about 7 ms on the frame a re-rank happens (every 32 m of travel or a cell change) | `far_band_frame_cost`, `far_band_reaches_fifty_km_within_its_time_budget` |

## Riding at 100 m/s (the phase exit)

`SUPERSTRUCT_AUTOFLY=100` flies level along the view at 100 m/s through the real streamer at the usual test spot (sector -2 -2 -3, local 10 745 584), 1500 frames, release, overlay on. The game log prints each second's frame rate and worst frame (`worst_ms`):

| Seconds | fps | worst frame |
|---|---|---|
| 1 (loading 11k chunks) | 0.2 | 86.5 ms |
| 2 | 47.7 | 250 ms |
| 3 to 12 | 143.6 to 144.0 | 9.6, 9.7, 9.8, 9.8, 9.8, 10.0, 11.0, 11.3, 12.3, 16.3 ms |

After the initial load of the whole 51 km far band, a 100 m/s flight holds 144 fps (the panel's limit) and the worst frame in each second stays between 9.6 and 16.3 ms, under the 20 ms hitch criterion of P3.4.4. The 16.3 ms second is the likely re-rank frame (about 7 ms of streamer work on top of a normal frame). The first two seconds are the one-off load after start-up, not a hitch while riding.

This is a level fly-through, not a tram: trams do not exist yet (P3.4.3), and the 60 fps at 1440p criterion is covered only by the GPU numbers above (this window is 900p at 144 fps). A run at 2560x1440 with a Tracy capture is the remaining part of the gate.

## Still yours (manual)

- [ ] Tracy capture of a 100 m/s ride at 1440p, saved here (frame times, the re-rank hitch, worker occupancy).
- [ ] P3.2.3: fly across the 1 km and 4 km band radii and look for pops (the cross-fade is built and unit-tested; the flythrough test found no visible swaps in fog).
- [ ] P3.4.2: ride 4 km of lift shaft and say whether it feels good.
- [ ] The earlier open items: P1.5.2, P2.4.4, P2.5.3, P2.7.2 (`docs/playtests/phase2.md`).

## Notes

(player's notes here)
