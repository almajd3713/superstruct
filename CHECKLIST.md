# CHECKLIST.md: Superstruct

The ordered list of atomic tasks for building Superstruct. Rules for working through it are in `CLAUDE.md` §8. In short:
- Do items in order unless the user says otherwise.
- Meet each item's **Done when** criterion.
- Tick it as `- [x] … (YYYY-MM-DD)`.
- Add discovered follow-ups as new items rather than expanding the current one.

**Notation**
- Each ID has the form `P<phase>.<group>.<item>`.
- **Done when** is the verifiable completion criterion.
- **Needs** lists dependencies outside the obvious earlier items.
- 🧭 marks a design decision to make with the user.
- 🎮 marks a playtest or feel check that needs a human playing natively on Windows.

---

## Phase 0: Specification and generator

*Phase exit:* port connectivity is verified on 10⁶ random cells, and generation is deterministic across runs and threads.

### P0.1 Repository and tooling
- [x] **P0.1.1** Initialize git and add `.gitignore` (`target/`, `out/`, editor files). *Done when:* `git status` shows only intended files. (2026-10-17)
- [x] **P0.1.2** Add `rust-toolchain.toml` pinning a current stable version, with `rustfmt` and `clippy` components. *Done when:* `rustup show` resolves the pinned toolchain in the repo. (2026-10-17)
- [x] **P0.1.3** Create the Cargo workspace with empty crates `gen` (lib), `engine` (lib), `game` (bin), `tools/slice` (bin) and `tools/bots` (bin), all on edition 2024. *Done when:* `cargo build --workspace` succeeds. (2026-10-17)
- [x] **P0.1.4** Add workspace lints (`[workspace.lints]`: `unsafe_code = "deny"` in `gen`, clippy pedantic subset) and `rustfmt.toml`. *Done when:* `cargo clippy --workspace --all-targets -- -D warnings` passes. (2026-10-17)
- [x] **P0.1.5** Add release and dev profiles: release with `lto = "thin"` and `codegen-units = 1`, dev with `opt-level = 1` for `gen` and deps. *Done when:* both profiles build. (2026-10-17)
- [x] **P0.1.6** Write `scripts/check.sh` (fmt check, clippy `-D warnings`, `cargo test --workspace`). *Done when:* the script exits 0 on the empty workspace and non-zero on a deliberate fmt violation. (2026-10-17)
- [x] **P0.1.7** Create the `docs/spec/`, `docs/playtests/` and `out/` directories, each with a `.gitkeep` where needed. *Done when:* the directories exist and `out/` is ignored. (2026-10-17)
- [x] **P0.1.8** Update docs that say `gen::…` (e.g. CLAUDE.md §6 `gen::salt`) to the library name `worldgen`: `gen` is a reserved keyword in edition 2024, so the `gen` package exposes its lib as `worldgen`. CLAUDE.md edits need user agreement. *Done when:* no doc refers to a `gen::` path. (2026-10-17)

### P0.2 Specification documents (spec before code)
- [x] **P0.2.1** `docs/spec/coordinates.md` covers:
  - units and axes
  - `WorldPos` (sector of 1024 m, i64; local f32)
  - normalization
  - level sizes L0–L4
  - world→cell math with floor semantics
  - cell→AABB math
  - camera-relative conversion
  - overflow policy at extreme coordinates

  *Done when:* each formula has a worked numeric example that includes negative coordinates. (2026-10-17)
- [x] **P0.2.2** `docs/spec/hashing.md` covers the hash algorithm choice (a 64-bit mix of `seed, level, x, y, z, salt`), the RNG choice (for example PCG32 or SplitMix64), the salt registry rules and **pinned test vectors**. *Done when:* the vectors are listed and the user has reviewed the doc. (2026-10-17)
- [x] **P0.2.3** `docs/spec/primitives.md` covers:
  - primitive types (Box, Ramp, Cylinder, StairRun, LiftShaft placeholder)
  - their fields, in cell-local space
  - material and tag ids
  - the walkable-surface definition
  - the cell output struct (primitives, ports, child constraints, metadata)

  *Done when:* the doc is complete. (2026-10-17)
- [x] **P0.2.4** 🧭 Choose the **feature ownership rule**:
  - option (a): features are clipped to the owning cell and continue across faces only through ports
  - option (b): bounded overflow, where features may extend up to radius R into neighbours, so evaluation queries neighbours within R
  - plus **column and layer keys** for vertical features (trunks, lift shafts) and horizontal ones (decks)

  Record the choice in `docs/spec/levels.md`. *Done when:* the decision is recorded with its rationale. (2026-10-17)
- [x] **P0.2.5** `docs/spec/levels.md` covers the content, parameters, probabilities and parent-constraint inputs and outputs of each level:
  - L0 regimes
  - L1 megaforms: trunks, decks, shafts, chasms, giant cavities
  - L2 districts: masses, bridges, stair towers, ramps, lift placement, sanctuaries, vistas
  - L3 and L4: outline only, detailed in Phase 4

  *Done when:* each level lists its inputs, outputs and the salts it uses. (2026-10-17)
- [x] **P0.2.6** `docs/spec/ports.md` covers:
  - canonical `FaceKey`: the face is owned by the lower cell on the axis
  - port derivation from the face hash
  - port types: walkway, deck continuation, bridge end, stair landing, lift stop
  - port geometry: position on the face, width, clearance
  - the backbone lattice rule
  - L1→L2→L3 cascade rules
  - an argument that global connectivity follows from local connectivity

  *Done when:* the connectivity argument is written and the user has reviewed it. (2026-10-17)
- [x] **P0.2.7** `docs/spec/regimes.md` covers L0 fields (density, verticality, style id, fog colour and density, light level, palette), boundary blending of atmosphere only, and the rare-events table (giant cavity, infinite staircase, gravity region, …) with probabilities. *Done when:* the doc is complete. (2026-10-17)
- [x] **P0.2.8** 🧭 `docs/spec/traversal.md` covers:
  - player capsule dimensions and speeds (defaults in CLAUDE.md §4)
  - step-up and max slope
  - **jump or no jump**
  - fall-death threshold
  - safe-ground rules
  - lift and tram speeds
  - port widths and clearances derived from the capsule

  *Done when:* the user has approved the numbers. (2026-10-17)

### P0.3 `gen` core types
- [x] **P0.3.1** `WorldPos { sector: I64Vec3, local: Vec3 }` with `normalize()`, `offset(Vec3)` and `delta(&WorldPos) -> DVec3`. *Done when:* unit tests cover crossing sector boundaries in both directions on every axis. (2026-10-17)
- [x] **P0.3.2** Floor-division helpers for i64 and `CellKey { level, coords: I64Vec3 }` with `WorldPos → CellKey` for each level. *Done when:* tests pass at 0, −1, sector edges and ±2⁵⁰. (2026-10-17)
- [x] **P0.3.3** `CellKey::parent()`, `children()`, `aabb_world()` and `neighbor(dir)`. *Done when:* proptest checks that `child.parent() == cell` for every child, and that a neighbour's neighbour in the opposite direction is the original cell. (2026-10-17)
- [x] **P0.3.4** `FaceKey` canonicalization: the cell plus axis, owned by the lower-coordinate cell. *Done when:* proptest checks that both adjacent cells produce the identical key for their shared face. (2026-10-17)
- [x] **P0.3.5** Column and layer key helpers (`ColumnKey(level, x, z)`, `LayerKey(level, y, region)`), if chosen in P0.2.4. *Done when:* tests pass. (2026-10-17)
- [x] **P0.3.6** Overflow policy: every coordinate arithmetic path uses checked or explicitly wrapping ops, per the spec. *Done when:* tests at ±(2⁶²) neither panic nor silently corrupt values. (2026-10-17)
- [x] **P0.3.7** `ColumnSpanKey(level, x, z, sy = y div 8)` helper per `levels.md` §1.2. *Done when:* tests confirm every cell in a span maps to the same key, and adjacent spans map to different keys. (2026-10-17)

### P0.4 Hashing and RNG
- [x] **P0.4.1** Implement `hash(seed, level, cell, salt) -> u64` per `hashing.md`. *Done when:* the pinned test vectors pass. (2026-10-17)
- [x] **P0.4.2** Implement the in-house RNG with `next_u32`, `next_f32` (in [0,1)), `range_i32`, `chance(p)` and `pick_weighted`. *Done when:* pinned sequence tests pass, plus a basic uniformity test (chi-square over 10⁶ samples). (2026-10-17)
- [x] **P0.4.3** Add the `worldgen::salt` registry module with a test that asserts all salts are unique. *Done when:* the test passes. (2026-10-17)
- [x] **P0.4.4** Add `GEN_VERSION` and `WorldSeed` types, with the seed parsed from a string or number. *Done when:* the same string always gives the same seed (tested). (2026-10-17)
- [x] **P0.4.5** Hash quality test: single-bit avalanche over every input field (each output bit flips with p ≈ 0.5 within a stated tolerance), per `hashing.md` §2. *Done when:* the test passes. (2026-10-17)

### P0.5 Primitives and cell output
- [x] **P0.5.1** Primitive structs per `primitives.md`, with an AABB per primitive. *Done when:* AABB tests pass for each type. (2026-10-17)
- [x] **P0.5.2** `CellOutput { primitives, ports, child_constraints, meta }`, with optional `serde` behind a feature flag. *Done when:* a round-trip serialization test passes. (2026-10-17)
- [x] **P0.5.3** Point-in-solid and signed-distance functions for each primitive type, used later by collision, the SDF port and validation. *Done when:* tests at known points pass. (2026-10-17)

### P0.6 L0 regimes
- [x] **P0.6.1** `regime(seed, L0 cell) -> Regime` with all fields from `regimes.md`. *Done when:* the output is stable (golden test), and a 64×64 histogram of fields matches the specified distributions. (2026-10-17)
- [x] **P0.6.2** Rare-event assignment per L0 cell. *Done when:* event frequencies over 10⁵ cells are within ±10% of the spec. (2026-10-17)
- [x] **P0.6.3** `atmosphere_at(WorldPos)`, which blends fog and palette across L0 boundaries. *Done when:* the output is continuous across a boundary (tested along a line). (2026-10-17)

### P0.7 L1 megaforms
- [x] **P0.7.1** Trunks: vertical pillars from column keys, with radius, vertical extent and style. *Done when:* a trunk spanning several L1 cells vertically is identical when generated from each of them (test). (2026-10-17)
- [x] **P0.7.2** Decks: horizontal slabs from layer keys or ports, with thickness and extent. *Done when:* decks continue across faces exactly as the ports specify (test). (2026-10-17)
- [x] **P0.7.3** Shafts, chasms and voids, including giant-cavity rare events that carve out L1 content. *Done when:* a cavity region contains no L1 solids (test). (2026-10-17)
- [x] **P0.7.4** L1 face ports, including the backbone lattice. *Done when:* backbone faces always carry at least one walkable port (proptest). (2026-10-17)
- [x] **P0.7.5** Bridges joining trunks and decks along L1 ports. *Done when:* every L1 port has a bridge or deck reaching it inside the cell (test). (2026-10-17)
- [x] **P0.7.6** `l1_far_primitives(cell)`: the reduced set used by the far LOD band. *Done when:* it is a strict subset of the full L1 output (test). (2026-10-17)
- [x] **P0.7.7** Lift core per L1 column (`levels.md` §4.0.1): footprint from `ColumnKey`, stops at every deck top and bridge height, absent in cavity cells. *Done when:* the core is identical from every cell in a column (test), and every deck and bridge in a cell has a stop (test). (2026-10-17)
- [x] **P0.7.8** Giant-cavity rim bites (`levels.md` §4.6): tile-grid bites with the connectivity shrink loop. The blob itself (voxelization and pocket filling, `worldgen::cavity`) landed in P0.7.3. *Done when:* every rim cell's deck still reaches all its port-carrying faces and core stops (test), and the no-pocket test still passes over 10⁴ L0 cells. (2026-10-17)
- [x] **P0.7.9** Infinite-staircase footprint (`regimes.md` §5.2): `l1::stair_footprint` already places it (P0.7.1, since trunks avoid it). Make shafts and chasms avoid it, and emit it as a `KeepOut` until P5.2.3. *Done when:* the `KeepOut` is identical from every L1 cell in the column, and no L1 solid intersects it (tests). (2026-10-17)
- [x] **P0.7.10** L1 generation cost: `l1_cell` rebuilds the L0 cavity mask about 13 times (once per `is_cavity` call). Compute it once per L0 cell and pass it through, behind an L1 context struct. *Done when:* `l1_cell` output is unchanged (golden), and a criterion bench shows the speed-up. (2026-10-17)

### P0.8 L2 districts
- [ ] **P0.8.1** An L1 context query for an L2 cell: which volume is structure, void, deck surface or trunk interior. *Done when:* the query agrees with the L1 primitives at 10⁴ random points (test).
- [ ] **P0.8.2** Building masses on deck surfaces (trunk-face masses deferred to P0.8.9 per `levels.md` §5.1). *Done when:* no mass intersects L1 voids, and every mass sits on a supporting surface (test).
- [ ] **P0.8.3** L2 ports: those inherited by cascade from L1, plus the cell's own face-hash ports. *Done when:* every L1 port that crosses an L2 face appears as an L2 port (test).
- [ ] **P0.8.4** Internal connectivity: build a port graph and generate walkways, stairs and ramps that connect all ports while respecting the slope and step limits. *Done when:* the walkability validator (P0.9) reports every port reachable in 10⁴ random cells.
- [ ] **P0.8.5** Stair towers and lift shafts as column features, with lift stops emitted as ports. *Done when:* a shaft is identical from every L2 cell it passes through (test).
- [ ] **P0.8.6** Sanctuary placement: chosen at the L1 level so they sit a few km apart along walkable routes, and marked in L2 metadata. *Done when:* the spacing distribution over 10⁴ L1 cells matches the spec.
- [ ] **P0.8.7** Vista rule: occasionally guarantee an opening that frames a view (an opening facing an L1 void). *Done when:* the vista frequency matches the spec, and the openings face a void (test).
- [ ] **P0.8.8** Railing and edge flags: mark walkable edges with a drop greater than the fall threshold as guarded or deliberately unguarded. *Done when:* the unguarded fraction matches the spec.
- [ ] **P0.8.9** Trunk-attached masses, with walkway routing to the L1 network so they stay reachable (`ports.md` Lemma D). *Done when:* the validator reports every trunk-attached mass reachable in 10⁴ random cells.

### P0.9 Walkability validator
- [ ] **P0.9.1** Extract walkable surfaces from primitives: top faces with clearance of at least the capsule height. *Done when:* tests on hand-built cells pass.
- [ ] **P0.9.2** Build the per-cell nav graph (surfaces linked by step, slope and stair rules). *Done when:* tests on hand-built cells pass.
- [ ] **P0.9.3** Cross-cell linking through ports, plus a reachability query. *Done when:* a 3×3×3 block of L2 cells is reachable from the backbone (test).
- [ ] **P0.9.4** Connectivity check over N random cells, multithreaded, with a report of failures and their seeds and cells. *Done when:* it runs on 10⁴ cells in CI time (< 60 s).

### P0.10 Slice tool
- [ ] **P0.10.1** CLI arguments: `--seed --level --axis --at --center --extent --res --out` and overlay toggles. *Done when:* `--help` documents everything.
- [ ] **P0.10.2** Render primitive occupancy as a PNG coloured by type, with overlays for the cell grid, ports and walkable surfaces. *Done when:* the PNG has been inspected visually and matches expectations for a known cell.
- [ ] **P0.10.3** `--report` mode that runs the P0.9.4 check and writes JSON stats. *Done when:* the JSON includes counts, failures and timing.
- [ ] **P0.10.4** Tune the L1 and L2 parameters using slice images: density, variety and void ratio. 🧭 Review a set of slices with the user. *Done when:* the user approves the look of the cross-sections.

### P0.11 Determinism, performance and phase exit
- [ ] **P0.11.1** Golden tests: hashes of `CellOutput` for 100 fixed cells at each level, tied to `GEN_VERSION`. *Done when:* the test fails on any output change.
- [ ] **P0.11.2** Thread-order test: generate the same cells serially and in a shuffled order across rayon threads, then compare. *Done when:* identical.
- [ ] **P0.11.3** Far-coordinate test: generate cells near ±2⁵⁰ sectors. *Done when:* they are valid, with no panics and connectivity intact.
- [ ] **P0.11.4** Criterion benchmarks for L1 and L2 cell generation. Budgets: L2 cell under 2 ms, L1 cell under 5 ms, single thread, release build. *Done when:* the benchmarks exist and the numbers are recorded in `PLAN.md`.
- [ ] **P0.11.5** **Phase exit:** the connectivity check on 10⁶ random cells reports zero backbone failures. *Done when:* the report has been saved to `docs/playtests/phase0.md`.

---

## Phase 1: Engine skeleton

*Phase exit:* a static greybox scene built from `gen` primitives renders on Windows, with GPU timings visible.

### P1.1 Window, device and loop
- [ ] **P1.1.1** Window with `winit`: resize, borderless-fullscreen toggle (F11), close. *Done when:* it runs on Windows.
- [ ] **P1.1.2** wgpu instance, adapter and device. Request `MULTI_DRAW_INDIRECT`, `INDIRECT_FIRST_INSTANCE` and `TIMESTAMP_QUERY` (and `MULTI_DRAW_INDIRECT_COUNT` if available). Log what was granted and give a clear error message if a required feature is missing. *Done when:* the log shows the expected adapter (RTX 4070) and features on Windows.
- [ ] **P1.1.3** Surface configuration with a vsync toggle (Fifo / Mailbox / Immediate). *Done when:* the toggle works at runtime.
- [ ] **P1.1.4** Main loop: fixed-step simulation at 120 Hz with variable rendering and interpolation. *Done when:* a moving test object is smooth at both 60 Hz and uncapped.
- [ ] **P1.1.5** Logging via `tracing` with an env-filter. *Done when:* log levels can be set from the environment.
- [ ] **P1.1.6** A `settings.toml` loader (resolution, FOV, sensitivity, vsync) with defaults written on first run. *Done when:* edits are applied on restart.

### P1.2 Input and camera
- [ ] **P1.2.1** Keyboard and mouse state, with raw mouse motion from `DeviceEvent::MouseMotion`. *Done when:* look input has no acceleration.
- [ ] **P1.2.2** Cursor grab and lock, released on Esc and on focus loss. *Done when:* it behaves correctly on Windows.
- [ ] **P1.2.3** An action-binding table (keys mapped to actions), so later rebinding is possible. *Done when:* game code reads actions, not raw keys.
- [ ] **P1.2.4** First-person camera: yaw, clamped pitch, FOV. *Done when:* look works.
- [ ] **P1.2.5** Reversed-Z infinite perspective matrix. *Done when:* a unit test confirms the near plane maps to depth 1 and depth approaches 0 at very large distances.
- [ ] **P1.2.6** Camera-relative transform: `chunk WorldPos − camera WorldPos → Vec3`, computed in f64 or i64 then narrowed. *Done when:* the test shows sub-millimetre error at sector 2⁴⁰.

### P1.3 Rendering basics
- [ ] **P1.3.1** Depth32Float buffer with compare `GreaterEqual`, cleared to 0. *Done when:* a test scene sorts correctly at 1 m and 10 km.
- [ ] **P1.3.2** Vertex format: position f32×3, octahedral-encoded normal, material id u16. *Done when:* the size and alignment are documented and tested.
- [ ] **P1.3.3** Basic forward pipeline with a camera uniform and a per-draw transform storage buffer. *Done when:* a grid of boxes renders.
- [ ] **P1.3.4** Greybox shader: Lambert lighting plus a world-aligned 1 m / 8 m grid so scale is readable, computed from camera-relative position plus a sector offset uniform. *Done when:* the grid does not swim at large coordinates.
- [ ] **P1.3.5** Debug-line renderer for AABBs, cell grids and ports. *Done when:* toggles show L1 and L2 cell bounds.
- [ ] **P1.3.6** Screenshot key F12, saving a PNG to `out/`. *Done when:* the file is written.

### P1.4 Shaders and tooling
- [ ] **P1.4.1** Load shaders from files with hot reload via `notify` in debug builds, and embed them with `include_str!` in release builds. *Done when:* an edit appears live.
- [ ] **P1.4.2** On a shader error, keep the old pipeline and show the error in the overlay. *Done when:* a deliberate syntax error doesn't crash the game.
- [ ] **P1.4.3** `egui` overlay (F1): FPS, frame-time graph, camera sector and local position, seed. *Done when:* it is visible and toggles.
- [ ] **P1.4.4** Tracy CPU zones behind a cargo feature flag. *Done when:* a capture shows frame zones.
- [ ] **P1.4.5** GPU timestamp queries per pass, shown in the overlay, with a fallback when unsupported. *Done when:* per-pass milliseconds are visible.
- [ ] **P1.4.6** Headless offscreen render harness: render to a texture, read it back and compare against a reference with a tolerance. It runs under lavapipe in WSL. *Done when:* one reference test passes in WSL.

### P1.5 Windows workflow and phase exit
- [ ] **P1.5.1** Document and verify the Windows build: a native `cargo` on Windows, or cross-compilation from WSL. *Done when:* the steps are in CLAUDE.md §7 and the binary runs.
- [ ] **P1.5.2** **Phase exit:** mesh one generated L2 cell (temporary simple box mesher) and render it with timings. 🎮 *Done when:* there's a screenshot and the user confirms it on Windows.

---

## Phase 2: Greybox walker

*Phase exit:* you walk to a horizon structure and confirm it matches what you saw from afar. 🎮 Answer the key question: does walking in it feel good?

### P2.1 Meshing
- [ ] **P2.1.1** Box mesher with per-face normals and material ids. *Done when:* tests pass on triangle counts and winding.
- [ ] **P2.1.2** Ramp, cylinder (segment count by size) and stair-run meshers. *Done when:* visually checked via screenshots.
- [ ] **P2.1.3** Remove internal faces between touching boxes in the same chunk. *Done when:* the triangle count drops on test cells, with no visible holes.
- [ ] **P2.1.4** Run meshing on worker threads, producing CPU-side `ChunkMesh` objects. *Done when:* the main thread never meshes (verified in Tracy).

### P2.2 Streaming
- [ ] **P2.2.1** Chunk identity: L2 cells for the near and mid bands, L1 cells for the far band. *Done when:* types exist.
- [ ] **P2.2.2** Desired-set computation: nested shells per level around the camera, with configurable radii. *Done when:* unit tests confirm the set changes correctly when the camera moves across cells.
- [ ] **P2.2.3** Job queue prioritized by distance and view direction, with cancellation of stale jobs. *Done when:* fast camera movement leaves no stale jobs running (stat counter).
- [ ] **P2.2.4** Staging belt with a per-frame upload byte budget. *Done when:* uploads per frame never exceed the budget (overlay stat).
- [ ] **P2.2.5** LRU eviction with a GPU memory budget. *Done when:* memory stays under budget during a 10-minute flythrough.
- [ ] **P2.2.6** Streaming overlay: resident chunks, queue length, memory, uploads per frame. *Done when:* visible.

### P2.3 Floating origin
- [ ] **P2.3.1** Render origin re-bases when the camera changes sector. Chunk offsets are recomputed with integer math. *Done when:* there is no visible jump while walking across sector boundaries.
- [ ] **P2.3.2** Debug teleport to arbitrary sector coordinates. *Done when:* teleporting to sector 2⁴⁰ shows no jitter (screenshot comparison with the origin).

### P2.4 Collision and controller
- [ ] **P2.4.1** Broadphase: collect `gen` primitives overlapping the capsule's swept AABB, cached per resident chunk. *Done when:* the query is under 0.1 ms in a typical scene.
- [ ] **P2.4.2** Capsule against Box, Ramp, Cylinder and StairRun using the SDFs from P0.5.3: depenetration plus sliding. *Done when:* unit tests pass for corners, edges and ramps.
- [ ] **P2.4.3** Grounded detection, step-up and slope limit per `traversal.md`. *Done when:* the controller climbs stairs, slides down slopes that are too steep, and doesn't jitter on edges (headless tests).
- [ ] **P2.4.4** Walk and run with acceleration and deceleration curves (no jump, per P0.2.8). 🎮 *Done when:* the user signs off on how movement feels.
- [ ] **P2.4.5** Noclip/fly debug mode with adjustable speed up to 1 km/s. *Done when:* the toggle works.
- [ ] **P2.4.6** Debug commands: teleport to a coordinate, to the nearest port, or to a random sanctuary. *Done when:* each works.

### P2.5 Death and respawn
- [ ] **P2.5.1** Safe-ground tracker per `traversal.md` (grounded ≥1 s, walkable, edge-probe radius clear, not on a moving platform), storing a `WorldPos`. *Done when:* headless tests cover each rule.
- [ ] **P2.5.2** Fall detection: height exceeding the threshold, or falling for a set time with no ground below. *Done when:* headless tests pass.
- [ ] **P2.5.3** Death sequence: let the fall play for a moment, fade to black, respawn at the safe ground, fade in. 🎮 *Done when:* the user signs off on how it feels.

### P2.6 Simple atmosphere
- [ ] **P2.6.1** Exponential height fog in the forward shader, with parameters from `atmosphere_at(camera)`. *Done when:* distant forms fade correctly.
- [ ] **P2.6.2** Sky and void colour from the regime palette. *Done when:* regimes are visibly distinct.

### P2.7 Phase exit
- [ ] **P2.7.1** Consistency assertion: in debug builds, check that the far-band L1 primitives equal the L1 primitives used by the near band for the same cell. *Done when:* the assertion is in place and passes during a flythrough.
- [ ] **P2.7.2** 🎮 Playtest: pick a structure on the horizon, walk or fly to it, and confirm it is the same. Then judge the feel of walking. Write `docs/playtests/phase2.md`. *Done when:* the notes exist and next steps have been agreed.

---

## Phase 3: GPU-driven pipeline and far field

*Phase exit:* 60 fps at 1440p on the RTX 4070 Laptop while riding at 100 m/s, with no popping (Tracy captures saved).

### P3.1 GPU-driven geometry
- [ ] **P3.1.1** Shared vertex and index buffers with an offset sub-allocator and free list. *Done when:* allocator tests pass, and fragmentation stays under 20% after a churn test.
- [ ] **P3.1.2** Instance storage buffer: per-chunk position relative to the render origin, mesh range, LOD band, bounds. *Done when:* the scene renders with one indirect draw per material bucket.
- [ ] **P3.1.3** Compute frustum culling that writes indirect draw arguments. *Done when:* visible counts match a CPU reference on a test scene.
- [ ] **P3.1.4** Draw with `multi_draw_indirect`, or `multi_draw_indirect_count` if available, with a fallback path. *Done when:* both paths render identically.
- [ ] **P3.1.5** Depth prepass. *Done when:* GPU time shows shading cost drop in dense scenes.
- [ ] **P3.1.6** Hi-Z depth pyramid built by compute from the depth buffer. *Done when:* the pyramid can be visualized in debug.
- [ ] **P3.1.7** Two-phase occlusion culling: draw last frame's visible set, build Hi-Z, cull the rest, then draw what was newly visible. *Done when:* instances drawn inside a dense interior drop by at least 5×, with no visible popping.
- [ ] **P3.1.8** Freeze-culling debug toggle. *Done when:* you can fly out and inspect the culled set.

### P3.2 LOD bands and transitions
- [ ] **P3.2.1** Mid band: merged, simplified L2 meshes (exterior masses only). *Done when:* the mid-band triangle budget is met.
- [ ] **P3.2.2** Far band: instanced L1 megaform primitives (unit box and cylinder meshes, scaled per instance). *Done when:* the far band reaches 50 km within its time budget.
- [ ] **P3.2.3** Blue-noise dithered cross-fade between bands. *Done when:* no hard pops are visible in a recorded flythrough.
- [ ] **P3.2.4** Band radii exposed in the overlay and settings. *Done when:* they can be tuned live.

### P3.3 SDF horizon
- [ ] **P3.3.1** WGSL port of the L0 regime and L1 megaform rules (hash, RNG, trunks, decks, voids), using integer sector math in the shader. *Done when:* the code exists.
- [ ] **P3.3.2** CPU-vs-GPU comparison test: evaluate occupancy at 10⁴ points through compute readback and compare with `gen`. *Done when:* it matches 100% on solid/void classification, within tolerance at boundaries.
- [ ] **P3.3.3** Full-screen raymarch pass beyond the far band, stepping cell by cell and writing depth and colour. *Done when:* the horizon shows megaforms beyond 50 km.
- [ ] **P3.3.4** Half-resolution option with depth-aware upsampling. *Done when:* the SDF pass is ≤ 2 ms at 1440p.
- [ ] **P3.3.5** Seamless handoff from the SDF horizon to the far band. *Done when:* no visible seam appears while flying outward.

### P3.4 Transport
- [ ] **P3.4.1** Moving-platform support in the controller: the player inherits platform velocity, and the safe-ground tracker ignores platforms. *Done when:* headless tests pass.
- [ ] **P3.4.2** Lifts on lift-shaft column features, with a call and ride interaction and stops at ports. 🎮 *Done when:* riding 4 km of shaft feels good.
- [ ] **P3.4.3** Tram lines generated along L1 decks or bridges (a spec addition first), with stations as ports. *Done when:* lines are consistent across cells (test).
- [ ] **P3.4.4** Tram ride at up to 100 m/s, with streaming prefetch along the line. *Done when:* no hitches above 20 ms appear during a 10 km ride (Tracy).

### P3.5 Phase exit
- [ ] **P3.5.1** Performance pass on Windows: the 1440p budgets in PLAN.md §11.4 are met while riding a tram at 100 m/s. *Done when:* Tracy captures and a summary are saved to `docs/playtests/phase3.md`.

---

## Phase 4: Detail

*Phase exit:* near-band spaces read as an architected megastructure at human scale.

### P4.1 L3 blocks (bounded WFC)
- [ ] **P4.1.1** Spec: an L3 tileset (rooms, corridors, stairwells, pipe runs, voids), connector types and the ports-to-boundary-conditions mapping. Write it in `docs/spec/levels.md`. *Done when:* the spec is complete.
- [ ] **P4.1.2** WFC/model-synthesis solver over a fixed 8×8×8 block with fixed boundary conditions. *Done when:* unit tests on small tilesets pass.
- [ ] **P4.1.3** Retry with sub-seeds (up to K attempts), then a fallback template that is always solvable. *Done when:* ≥ 99.9% of blocks solve before needing the fallback (stats over 10⁵ blocks).
- [ ] **P4.1.4** L3 output is wired into the walkability validator. *Done when:* L3 blocks keep every port reachable (10⁴ blocks).
- [ ] **P4.1.5** L3 performance: a block solves in under 5 ms on a worker thread. *Done when:* a benchmark records it.

### P4.2 L4 module kit (generated in code)
- [ ] **P4.2.1** Module mesh generators: wall panel variants, railing, grating floor, pipe (straight, bend), cable bundle, door frame, lamp, stair. *Done when:* a gallery screenshot has been inspected.
- [ ] **P4.2.2** Module placement rules on L3 surfaces and edges, including railings from the P0.8.8 edge flags. *Done when:* guarded edges have railings and unguarded edges don't (test).
- [ ] **P4.2.3** Module instancing through the GPU-driven path. *Done when:* the near-band budget is met in a dense block.

### P4.3 Materials and stylization
- [ ] **P4.3.1** Material table (material id → base tone, roughness-ish, emissive). The palette stays narrow. *Done when:* materials are applied.
- [ ] **P4.3.2** Emissive panels and windows driven by module and regime parameters. *Done when:* they are visible in dark scenes.
- [ ] **P4.3.3** Prototype post passes behind toggles: posterize, ordered dither, ink outlines from depth and normals, film grain. *Done when:* all four can be toggled.
- [ ] **P4.3.4** 🧭 Art-direction review with the user: choose a combination of stylization passes and fix the palette. *Done when:* the choice is recorded in PLAN.md.

---

## Phase 5: Atmosphere and audio

*Phase exit:* the game feels oppressive, sanctuaries feel like relief, and rare events land.

### P5.1 Lighting and fog
- [ ] **P5.1.1** Clustered forward+: build the cluster grid and light lists in compute. *Done when:* 1000 point lights stay under budget.
- [ ] **P5.1.2** Light emission from emissive modules, within a per-view light budget. *Done when:* the budget is respected.
- [ ] **P5.1.3** Froxel volumetric fog: density injection from regime and local volumes, light injection, temporal reprojection, integration. *Done when:* it is ≤ 2 ms at 1440p, with visible light shafts.
- [ ] **P5.1.4** Ambient occlusion: analytic AO from a nearby-primitive buffer, or SSAO. 🧭 Pick after comparing both. *Done when:* the choice is recorded and implemented.
- [ ] **P5.1.5** Aerial perspective tuned per regime. *Done when:* the user reviews it.

### P5.2 Sanctuaries, landmarks and rare events
- [ ] **P5.2.1** Sanctuary interiors: warm low light, props (bench, plants, lamp), framed vista. *Done when:* 🎮 the user reviews one in game.
- [ ] **P5.2.2** Giant-cavity rare event rendered and traversable around its rim. *Done when:* found by teleport and reviewed.
- [ ] **P5.2.3** Infinite-staircase event. *Done when:* implemented and reviewed.
- [ ] **P5.2.4** 🧭 Gravity regions: decide whether gravity can change direction (this affects the controller and the ports spec). *Done when:* the decision is recorded and, if yes, broken into new items.
- [ ] **P5.2.5** Distant slow movers: light trains, Builder silhouettes and trams in the far band and horizon. *Done when:* visible movement at kilometre scale.

### P5.3 Audio
- [ ] **P5.3.1** `kira` setup with mixer buses (ambience, footsteps, machinery, UI). *Done when:* buses are controllable.
- [ ] **P5.3.2** Footsteps by material and speed. *Done when:* they play correctly.
- [ ] **P5.3.3** Ambient drone beds per regime, with crossfades. *Done when:* audible transitions between regimes.
- [ ] **P5.3.4** Reverb probes: estimate the size of the surrounding space by ray-casting against primitives, and drive reverb parameters from it. *Done when:* the change is audible between a duct and a void.
- [ ] **P5.3.5** Sanctuary sound bed. *Done when:* audible relief on entering.

### P5.4 Quiet tools
- [ ] **P5.4.1** Altimeter and coordinates readout toggle (absurd numbers welcome). *Done when:* it toggles.
- [ ] **P5.4.2** Droppable beacons, visible from distance through fog, persisted in the save file. *Done when:* they survive a restart.
- [ ] **P5.4.3** Photo mode: free camera within a radius, HUD hidden, FOV and exposure controls. *Done when:* it works.

---

## Phase 6: Polish and ship

### P6.1 Automated quality
- [ ] **P6.1.1** Bots: headless walkers that follow the nav graph across N km and report getting stuck, unexpected falls or unreachable ports. *Done when:* `cargo run -p bots` produces a report.
- [ ] **P6.1.2** Add the bots and the 10⁴-cell connectivity check to `scripts/check.sh` (a quick mode) and a nightly script (full mode). *Done when:* both modes run.
- [ ] **P6.1.3** Automated far/near consistency test across random cells. *Done when:* it passes.

### P6.2 Player-facing
- [ ] **P6.2.1** Settings menu: resolution, FOV, sensitivity, vsync, fog and SDF quality, head-bob off/on, motion comfort. *Done when:* every setting applies live or on restart, as labelled.
- [ ] **P6.2.2** Save file: seed, player position, safe ground, beacons. *Done when:* a round trip restores the state exactly.
- [ ] **P6.2.3** Start flow: new world (random or typed seed), or continue. *Done when:* both paths work.
- [ ] **P6.2.4** Spawn in a guaranteed calm starting location (for example near a sanctuary). *Done when:* 100 random seeds all spawn safely (test).

### P6.3 Performance and release
- [ ] **P6.3.1** Final performance pass against all budgets, with frame-time histograms from a 30-minute session. *Done when:* the results are in `docs/playtests/release.md`.
- [ ] **P6.3.2** Release build: embedded shaders, LTO, stripped symbols, app icon, version string. *Done when:* a single folder runs on a clean Windows machine.
- [ ] **P6.3.3** 🎮 Final playtest: a 1-hour zen session with notes. *Done when:* the notes are written.
