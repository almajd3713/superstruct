# Handoff: end of Phase 1 (2026-10-17)

Read `CLAUDE.md`, then this file, then `CHECKLIST.md` (Phase 2 starts at P2.1).

## State

- Phase 0 (generator) is complete. P0.11.4c (cut busy L2 cell cost to under 0.5 ms) is deferred to a later optimization pass by the user.
- Phase 1 (engine skeleton) is built and committed through `c701054`. Every item is ticked except **P1.5.2**, the phase exit: it needs the user to look at the scene on Windows and confirm. Nothing has been pushed.
- `scripts/check.sh` passes in WSL. The 4 headless render tests pass natively on the RTX 4070.
- The user is remote and has said to use judgement; issues they find will be fixed later.

## Still unverified by hand (built and run, but only the user can eyeball)

- F11 fullscreen and window resize; cursor grab and release (click, Esc, focus loss).
- F2 vsync toggle at runtime; smoothness at 60 Hz and uncapped.
- Live shader edit under `crates/engine/shaders/`.
- A Tracy capture (`--features game/tracy`); no profiler on this machine.
- P1.5.2: run `SUPERSTRUCT_OVERLAY=1 scripts/win.sh run`, look at the generated L2 cell, tick the item.

## How to run things

- WSL: `scripts/check.sh` (fmt, clippy, tests).
- Windows: `scripts/win.sh build|test|run [cargo args]`. GNU toolchain (no MSVC linker), target dir `%USERPROFILE%\ss-target`. Details in CLAUDE.md §7.
- Remote screenshots: `SUPERSTRUCT_AUTOSHOT=<frames> SUPERSTRUCT_OVERLAY=1 SUPERSTRUCT_DEBUG_BOXES=1 SUPERSTRUCT_SETTINGS=/tmp/ssrun/settings.toml scripts/win.sh run`, then read `out/screenshot-*.png`. Without a settings path the game writes `settings.toml` into the working directory (git-ignored).
- `SUPERSTRUCT_CELL=x,y,z` picks the L2 cell; default is the busiest within 4 cells of the origin (4 -4 4 at seed 0).
- Keys: WASD/Space/Ctrl/Shift fly, F1 overlay, F2 vsync, F3 cell outlines, F11 fullscreen, F12 screenshot, Esc releases the cursor.

## Engine layout (`crates/engine/src`)

| File | Role |
|---|---|
| `app.rs` | winit loop, fixed 120 Hz step, `draw`, screenshot, env switches |
| `gpu.rs` | instance (Vulkan/DX12/Metal only), adapter, device, surface, vsync |
| `renderer.rs` | depth (reversed-Z, `GreaterEqual`, clear 0), scene and line pipelines, `FrameInput`, `reload_shaders` |
| `shaders.rs` + `shaders/*.wgsl` | disk loading and `notify` watcher in debug, embedded in release |
| `camera.rs`, `input.rs` | camera maths and camera-relative transform; action bindings |
| `vertex.rs`, `mesh.rs`, `primmesh.rs`, `cellscene.rs` | 20-byte vertex, CPU mesh, temporary primitive mesher, one generated L2 cell |
| `overlay.rs`, `gpu_timer.rs`, `profile.rs` | egui overlay, timestamp zones, Tracy `zone!` |
| `headless.rs`, `readback.rs` | offscreen render and PNG output; tests in `tests/render.rs` |
| `demo.rs` | throwaway pillar scene, used only by tests; delete once tests use real cells |

## Decisions and gotchas

- GL is excluded (`Backends::PRIMARY` in `gpu.rs` and `headless.rs`): Mesa GL-on-D3D12 in WSL segfaults intermittently. Headless tests therefore skip in WSL. A `WGPU_BACKEND` env switch was discussed but not built.
- wgpu 30 / egui 0.37 / glam 0.34 APIs differ from older docs: projection is `glam::camera::rh::proj::directx::perspective_infinite_reverse`; `queue.present(frame)`; pipeline layouts take `Option<&BindGroupLayout>`; read the registry source when unsure.
- The overlay draws through a non-sRGB view (surface has `view_formats`); egui asserts every `TexturesDelta` is consumed.
- Grid phase is sector-independent because 1024 is a multiple of 8; the shader only needs the in-sector camera offset.
- Reference image `crates/engine/tests/reference/demo.png` was rendered on the RTX 4070; regenerate with `UPDATE_REFERENCE=1` (via `WSLENV=UPDATE_REFERENCE scripts/win.sh test ...`) after shader palette changes.
- Beyond the pre-approved crates, added: `bytemuck`, `toml`, `tracing-subscriber`, `egui-winit`, `notify` (justified in commit messages).
- Scene is not visually interesting yet; shading is flat greybox. Stylization is Phase 2 and later.

## Next

Phase 2 (greybox walker), in order: P2.1 meshing (internal-face removal, worker threads), P2.2 streaming, P2.3 floating origin, P2.4 collision and controller, P2.5 death and respawn, P2.6 atmosphere, P2.7 exit. Sign-off needed from the user at P2.4.4 (movement feel), P2.5.3 (death feel) and P2.7.2 (playtest).
