# CLAUDE.md: Superstruct

Instructions for any agent (Claude or other) working in this repository. Read this file first, then `PLAN.md` for the full research and design rationale, then `CHECKLIST.md` for what to do next.

---

## 1. What this project is

Superstruct is a **first-person walking and exploration game** set in a procedurally generated, **unbounded (x, y, z) cyberpunk megastructure** inspired by Tsutomu Nihei's *BLAME!*.

- The only verbs are walking and exploring. There is no combat, no inventory and no quests.
- The experience is zen, oppressive and sometimes mind-breaking because of its scale, with occasional sanctuaries to rest in.
- **The horizon promise:** anything visible, even on the far horizon, can eventually be reached, and when you get there it is the same structure you saw from far away.
- Visuals are stylized, gloomy and menacing, with a monochrome feel in the spirit of Nihei's ink work.
- Falling more than the threshold is death. You respawn at your last safe standing position.

**Team:** one human developer plus Claude. **Platform:** PC, with Windows as the shipping target.
**Engine:** custom, in Rust + wgpu. There is no third-party game engine, by deliberate choice: scale and performance are the main goals.

## 2. Key documents

| File | Purpose | Who edits |
|---|---|---|
| `CLAUDE.md` | This file: how to work here (rules, invariants, conventions) | Change only with the user's agreement |
| `PLAN.md` | Research, design rationale, architecture, decisions | Update when a decision changes, and say so in the commit |
| `CHECKLIST.md` | Ordered, atomic tasks with done-criteria | Tick items as they are completed |
| `docs/spec/*.md` | Generator and engine specifications, written in Phase 0 | Spec changes come before code changes |
| `docs/playtests/*.md` | Notes from playtests at phase exits | Append-only |

## 3. Non-negotiable invariants

Breaking any of these breaks the horizon promise or unbounded coordinates. **Do not change them without explicit user approval.**

1. **The generator is a pure function.** The output for a cell depends only on `(world_seed, GEN_VERSION, level, cell coords)` plus its parent cells and a bounded set of neighbour cells. Nothing may depend on:
   - generation order
   - which thread ran it
   - wall-clock time
   - player history
   - global mutable state
   - `HashMap` iteration order
   - `rand::thread_rng`
2. **Hierarchy is top-down.** A finer level refines what its parent decided and never contradicts it. Far LOD comes from the coarse levels' data, never from simplifying fine meshes.
3. **Hash on integers only.** Every random decision comes from the project's own stable hash of integer keys (§6). Never feed float coordinates into a hash.
4. **Neighbours agree without communicating.** Anything shared between two cells (face ports, features that cross a boundary) is derived from a key both cells compute identically: a canonical face key, or a column/layer key.
5. **Connectivity holds by construction.** Every cell connects all of its ports internally, and a backbone lattice of ports always exists. Code that can produce an unreachable backbone port is a bug.
6. **The GPU never sees large coordinates.** World positions are an `i64` sector plus an `f32` local offset. Everything sent to the GPU is relative to the camera or to the render origin.
7. **The `gen` crate has no GPU, windowing, audio or engine dependencies.** It must build and test headless on any machine.
8. **No silent output changes.** Any change that alters generator output must bump `GEN_VERSION` and update the golden tests deliberately.

## 4. Conventions

### Coordinates and units
- Units are **metres**. Axes are **right-handed, +Y up**, and gravity points along −Y.
- **Sector size is 1024 m.** A world position is `WorldPos { sector: I64Vec3, local: Vec3 }`, with `0 ≤ local < 1024` on each axis after normalization.
- Hierarchy cell sizes are powers of two:

  | Level | Name | Cell size |
  |---|---|---|
  | L0 | Regime | 32768 m |
  | L1 | Megaforms | 4096 m |
  | L2 | Districts | 512 m |
  | L3 | Blocks | 64 m |
  | L4 | Modules | 8 m |
  | L5 | Dressing | — |

- Integer division of negative coordinates uses **floor semantics** (`div_euclid` / `rem_euclid`), never truncation.
- Player defaults (tunable, defined in `docs/spec/traversal.md`):

  | Parameter | Value |
  |---|---|
  | Capsule height | 1.8 m |
  | Capsule radius | 0.3 m |
  | Eye height | 1.65 m |
  | Step-up | 0.45 m |
  | Max slope | 40° |
  | Walk speed | 1.4 m/s |
  | Run speed | 4.0 m/s |

### Rust
- Stable toolchain, pinned in `rust-toolchain.toml`, edition 2024.
- `cargo fmt` is mandatory. `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- Libraries (`gen`, `engine`) use `thiserror`. Binaries and tools use `anyhow`.
- No `unwrap()` or `expect()` in engine or game runtime paths unless the invariant is documented beside the call. Tests and tools may use them.
- Keep `unsafe` to a minimum. Each `unsafe` block needs a `// SAFETY:` comment.
- Prefer plain data structs and free functions over deep trait hierarchies.
- Comment sparingly. Explain *why*, not *what*. Spec-level reasoning belongs in `docs/spec/`, not in code comments.

### Dependencies
- The pre-approved crates are listed in `PLAN.md` §11.1: winit, wgpu, glam, rayon, crossbeam, kira, egui, egui-wgpu, tracy-client, thiserror, anyhow, serde (optional feature), proptest, criterion, image (tools), notify (hot reload), pollster.
- **Any other dependency needs a one-line justification in the PR or commit message.** Do not add a dependency to `gen` if it could affect determinism; the hash and RNG are implemented in-house on purpose.

### Shaders
- WGSL only, under `crates/engine/shaders/`, hot-reloaded in debug builds.
- A shader that fails to compile keeps the previous pipeline running and reports the error in the overlay. It must never crash the game.
- Any WGSL port of generator logic (the SDF horizon) must have a CPU-vs-GPU comparison test.

### Performance
These rules come from the budgets in `PLAN.md` §11.4.
- No heap allocation per frame in hot loops. Reuse buffers.
- Every new render pass gets a GPU timestamp zone. Every new system gets a Tracy CPU zone.
- Generation, meshing and other heavy work never run on the main thread.
- Uploads to the GPU go through the staging belt, which has a per-frame byte budget.
- If a change regresses a budget, say so explicitly. Never hide it.

## 5. Workspace layout (target)

```
Cargo.toml              workspace
rust-toolchain.toml
scripts/check.sh        fmt + clippy + test; run before declaring a task done
crates/
  gen/                  pure generator (no GPU deps)
  engine/               renderer, streaming, camera, input, audio wrappers
    shaders/            WGSL
  game/                 controller, respawn, transport, sanctuaries; the binary
tools/
  slice/                CLI: PNG cross-sections + connectivity reports from `gen`
  bots/                 headless walkers that traverse generated space
docs/
  spec/                 coordinates, hashing, levels, ports, regimes, traversal, primitives
  playtests/
out/                    tool and screenshot output (git-ignored)
```

## 6. Hashing and randomness

- A single hash entry point: `hash(seed, level, cell: I64Vec3, salt: u64) -> u64`. The exact algorithm and pinned test vectors live in `docs/spec/hashing.md`.
- Every distinct decision uses its own **salt constant** from a single registry module (`gen::salt`). Never reuse a salt for two different purposes, and never inline magic salts.
- Per-cell sequential randomness comes from a small in-house RNG seeded from that hash. Do not use the `rand` crate in `gen`.

## 7. Environment and running

- Development happens in **WSL2** (Linux) on a machine with an **RTX 4070 Laptop GPU**.
- In WSL, run and test `gen`, tools, bots and headless render tests. wgpu may fall back to software rendering (lavapipe) here, so results are valid for correctness only and **never for performance**.
- **Run, play and profile the game natively on Windows.** Either run `cargo` on the Windows side, or cross-compile from WSL. Every performance number must come from a native Windows run.
- Common commands (once the workspace exists):
  - `scripts/check.sh`: format check, clippy and all tests.
  - `cargo run -p slice -- --help`: generator cross-sections.
  - `cargo run -p game --release`: the game (on Windows).

## 8. How to work on a task

1. Pick the **first unchecked item** in `CHECKLIST.md`, unless the user names a different one. Respect the dependencies it lists.
2. If the item touches generator behaviour, read or update the relevant `docs/spec/` file **first**.
3. Make the smallest change that meets the item's **Done when** criterion. Do not bundle in unrelated refactors.
4. Verify:
   - `scripts/check.sh` passes
   - plus the item's own check (a test, slice PNG, screenshot, Tracy capture or playtest note)
   - for visual changes, look at the output image before claiming success
5. Tick the item in `CHECKLIST.md` as `- [x]` and append ` (YYYY-MM-DD)`. If you discovered follow-up work, add it as a new unchecked item in the right place. Do not silently expand the item you are working on.
6. Commit only when the user asks. Keep one checklist item per commit where practical, and reference its ID in the commit message (for example `P0.3.2: floor-div cell indexing`).
7. Report results faithfully. Failing tests, skipped steps and missed budgets are stated plainly.

**Ask the user before you:**
- change an invariant (§3) or a decision recorded in `PLAN.md`
- add a heavyweight dependency
- change player-feel numbers
- drop a checklist item

## 9. Design north stars

When unsure, these resolve the question.
- **Scale over detail.** A believable horizon is worth more than a perfect railing.
- **Consistency over variety.** A far structure that changes when you approach it is the worst possible bug.
- **Contrast creates feeling.** Oppression makes sanctuaries matter; compression makes voids overwhelming.
- **Zen means no friction.** Calm movement, no HUD clutter, no punishment beyond lost progress after a fall.
