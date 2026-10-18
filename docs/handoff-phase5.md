# Handoff: Phases 4 and 5.1 built, interiors playable (2026-10-18)

Read `CLAUDE.md`, then this file, then `CHECKLIST.md`, then `docs/controls.md`. Nothing is pushed; HEAD is `b3c3ff3` on a detached `HEAD` branch state, one commit per checklist item.

## State

- Done and committed: P4.1.x (L3 blocks), P4.2.x (L4 kit, placement, instancing), P4.3.1 to P4.3.3 (materials, emissive, stylization prototypes), P4.4.1 (L3 streamed into the game), P5.1.1 to P5.1.4 (clustered lights, lamp lights, froxel fog, AO).
- Built but waiting on the user: **P5.1.5** (aerial perspective per style, review shots in `out/regime-*.png`). Deferred by the user on purpose: **P4.3.4** (art direction), see "Styling" below. Older sign-offs still open: P1.5.2, P2.4.4, P2.5.3, P2.7.2, P3.4.2, P3.4.4 (hitch gate), P3.5.1, and the Phase 0 Windows bench.
- Next unchecked items: P5.2.1 (sanctuary interiors), P5.2.2 and P5.2.3 (rare events), P5.2.4 (gravity decision), P5.2.5 (distant movers), P5.3.x (audio, `kira`), P5.4.x (quiet tools).
- `GEN_VERSION` is 4. Goldens regenerated at the bump.

## What you can see in the game now

Walk to a mass entrance and in. Viewpoints (print more with `cargo test -p engine --release --lib print_entrance_viewpoints -- --ignored --nocapture`):

`SUPERSTRUCT_SECTOR=-46,0,-8 SUPERSTRUCT_LOCAL=404.0,529.6,434.0 SUPERSTRUCT_YAW=0 SUPERSTRUCT_WALK=1 scripts/win.sh run --release` stands at a door; local z 422 is just inside. Screenshots: `out/interior-door.png`, `out/interior-inside.png`.

## Architecture added this session

| Area | Where | Notes |
|---|---|---|
| L3 blocks | `gen/src/l3.rs`, `l3_plan.rs`, `l3_tiles.rs`, `l3_solver.rs`, `l3_validate.rs` | model synthesis over 8×16×8 slots; retries then fallback; `clear_of_trunks` moves an attached mass's inner face off its trunk disc; `block_is_active` |
| Entrances and links | `gen/src/l2_entrance.rs`, `l2_links.rs`, `l2.rs` | `PortKind::Entrance { plane }` ports with `ForcedPort`s, spanning tree plus 30 % loops as `KeepOut`s; lift and tower nodes not linked yet |
| L4 placement | `gen/src/l4.rs` | `block_modules`, `railing_modules`; every module is `NO_COLLIDE`; `glow` 0 to 15, zero in a Blackout |
| Kit and instancing | `engine/src/kit.rs`, `modsys.rs`, `vs_module` in `scene.wgsl` | one shared kit mesh, 16-byte instances per chunk, indirect draw per (chunk, piece) |
| Materials | `engine/src/materials.rs` | table in the globals; `lit_glow` shows emissive in the chunk's glow colour |
| Lights | `lights.rs`, `light_cull.wgsl`, `lights.wgsl` | 64 px tiles × 24 slices, 64 lights per cluster, budget (default 512), point and spot |
| Fog | `volfog.rs`, `fog.wgsl`, `volfog.wgsl` | froxel grid out to 250 m, temporal reprojection, local `FogVolume`s, analytic beyond |
| AO | `ao.rs`, `ao.wgsl` | analytic box-distance chosen (PLAN.md §11.3), SSAO kept as fallback; needs the depth prepass |
| Post | `post.rs`, `post.wgsl` | posterize, dither, outlines, grain behind toggles (F8, F9, F10, G) |
| Streaming | `stream.rs`, `streamer.rs`, `meshpool.rs` | `ChunkId::l3`, `l3_radius_m` 192 m; chunks carry modules and AO boxes; L2 envelopes with an entrance become hollow shells with the door cut; envelopes no longer collide, L3 provides collision |

The scene pipelines have four bind groups: globals, draws, lights (plus AO texture), fog. Any new scene pipeline must set all four. GPU zones: scene, ui, cull, prepass, hiz, occlusion, horizon cache, horizon half, post, light cull, fog, ao.

## Measured (RTX 4070 laptop, native Windows, release, 2560×1440)

L3 block solve mean 0.83 ms (WSL); 3147 module instances scene +0.08 ms; 1000 lights: light cull 0.09 to 0.16 ms, scene 0.5 ms; froxel fog 0.12 ms; AO analytic 0.47 ms, SSAO 0.39 ms; post (all four) 0.15 ms. All inside PLAN.md §11.4.

## How to work

- WSL for `cargo test`; Windows for anything with a GPU: `scripts/win.sh test -p engine --release --test render <name>` and `scripts/win.sh run --release`. Env vars reach Windows only through `WSLENV` (the script forwards the `SUPERSTRUCT_*` list; for a test use `WSLENV=NAME NAME=1 scripts/win.sh test ...`, e.g. `UPDATE_REFERENCE`).
- **Always run `cargo test -p gen` in the debug profile too.** A debug-only integer overflow (`PlanGrid`, fixed in `175c6c7`) crashed the game at start while every release test passed.
- Pre-existing failure, release only: `streamer::tests::far_to_full_swap_keeps_the_old_chunk_drawn_until_the_new_one_is_resident` (its assertion needs a debug-only consistency check).
- Timing tests use `settled_times` (best of three) because other GPU tests running in parallel can push a budget over.
- Render outputs go to `out/` (git-ignored); look at images before claiming success.
- Project rules: one checklist item per commit, no AI attribution lines, no subagents, tick items in the same commit.

## Judgement calls to know about

- `levels.md` §5.3: the loop candidate rule ("unless a tree edge") left no candidates; changed to "nearest node not joined by a tree edge".
- The fine nav counts a 4 to 9 quantum strip as floor only when wide floor of the same level lies on both short sides (`bridges` in `walk.rs`).
- Per-style haze and height falloff are first guesses (`regimes.md` §4).
- Lights have no shadows, so fog beams are cones, not shafts cut by geometry.

## Known gaps

- Beyond 192 m a doorway is a see-through hole into an unlit shell; masses without L3 loaded have no collision; module instances pop in without a fade.
- Windows, facade panels, atrium balustrades, pipe bends and lift or tower link nodes are not placed yet.
- Module lights and AO boxes exist only inside loaded L3 chunks; the froxel fog's local volumes and lights come only from those.
- The sanctuary has no interior content yet (P5.2.1): props, warm lights and the framed vista still need writing, probably as new module kinds placed from the sanctuary zone.
- No audio at all; the controller and respawn have no sound hooks yet.

## Styling (agreed plan, not started)

The user judged the post filters "not that stylized, just postprocess filters". Direction agreed to explore, with the user, plus internet research: move the style into the shading (hard tonal steps, world-anchored hatching or screentone for shadow, variable line weight, ink-wash fog toward black or white), keep near-monochrome palettes, drop grain and global posterize. P4.3.4 stays open until then. Do it after P5 is feature complete, iterating with the user.

## Suggested order

1. Ask the user to review P5.1.5 (regime shots) and walk an interior.
2. P5.2.1 sanctuary interior (also a good test of module kinds and warm lights), then P5.3 audio, P5.4 tools, P5.2.x events.
3. L3 streaming polish: fade modules in, a plug or dark shell for far doorways, collision before the L3 radius.
4. Then the styling work above.
