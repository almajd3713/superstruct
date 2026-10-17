# Handoff: Phase 2 built, awaiting the user's playtest (2026-10-18)

Read `CLAUDE.md`, then this file, then `CHECKLIST.md`.

## State

- Phase 0 is done (P0.11.4c, cutting a busy L2 cell under 0.5 ms, is deferred by the user). Phase 1 is done except the user's confirmation of P1.5.2.
- Phase 2 is built through P2.7.1 and committed; nothing is pushed. Open items need the user: **P2.4.4** (movement feel), **P2.5.3** (death feel), **P2.7.2** (playtest, `docs/playtests/phase2.md` is a skeleton).
- `scripts/check.sh` passes in WSL; the 4 headless render tests pass on the RTX 4070 via `scripts/win.sh test -p engine --test render`.
- The world is the real generator at seed 0, but only L1 and L2 exist, so it looks sparse: no rooms, modules or dressing until Phase 3 and later.

## New engine modules (`crates/engine/src`)

| File | Role |
|---|---|
| `chunkmesh.rs` | meshes a cell's primitives; box and stair-step faces buried against other boxes are removed (exact integer rects, 64 m grid broadphase) |
| `meshpool.rs` | worker threads: generate, mesh, return `ChunkMesh` (+ primitives for collision); priority queue, `retarget` cancels stale jobs; per-worker L1 cache |
| `stream.rs` | `ChunkId` (cell + `Detail::Full/Far`), `StreamConfig`, `desired_set`. L1 structure is drawn at all distances and L2 adds on top, so bands overlap by design |
| `streamer.rs` | residency, per-frame upload byte budget (hard cap, oversize chunks rejected), LRU eviction under a memory budget, far-to-full swap visibility, `CollisionWorld`, debug L1 consistency check |
| `renderer.rs` | per-chunk GPU buffers with handles, hidden chunks, fade, fog and ambient uniforms |
| `collision.rs` | `Solid`, SDF union and gradient, `SolidSource`, per-chunk `CollisionChunk`, `CollisionWorld` |
| `controller.rs` | capsule as 5 spheres: step-up, step-down snap, 40 degree limit, accel curves. Pure, headless tests |
| `respawn.rs` | safe ground, fall detection, death/fade state machine |
| `debugcmd.rs` | teleport to a coordinate, nearest port, next sanctuary; run on a worker thread |

## Controls and env switches

F4 fly/walk, `+`/`-` fly speed (1 to 1000 m/s), Shift sprint (run, or x4 in fly), F5 nearest port, F6 next sanctuary, Esc frees the cursor and lets you use the overlay teleport box. Env: `SUPERSTRUCT_SECTOR`, `SUPERSTRUCT_LOCAL`, `SUPERSTRUCT_CELL`, `SUPERSTRUCT_WALK=1` (see CLAUDE.md §7; `scripts/win.sh` forwards them).
Example on a real deck: `SUPERSTRUCT_WALK=1 SUPERSTRUCT_SECTOR=-2,-2,-3 SUPERSTRUCT_LOCAL=10,745,584`.

## Known gaps and judgement calls

- Not verified by hand (only screenshots and headless tests): overlay buttons, F5/F6, the teleport text box, the feel of any movement, the visible death fade in motion, a Tracy capture (no profiler here).
- P2.2.4 uses per-chunk `create_buffer_init`, not `wgpu::util::StagingBelt`. P2.2.5 was checked with a simulated 900-frame fast flythrough against a small budget, not a 10-minute run.
- The new `crossbeam-channel` dependency was added (pre-approved crossbeam family).
- Ramp and stair SDFs are bounds, not exact; fine for walking, not for deep-penetration cases.
- Worker mesh time in a debug build is about 77 ms for a busy L2 cell; no release number yet on Windows.
- `SUPERSTRUCT_CELL` was not forwarded by `win.sh` before; fixed.

## Next

After the playtest: P2.7.2 notes and agreed next steps, then Phase 3 (see CHECKLIST). Good first jobs if the user is away: a release-build mesh/gen timing on Windows, a long flythrough bot, and pop-in review of the far-to-full swap.
