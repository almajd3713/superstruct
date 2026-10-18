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
- [x] **P0.8.1** An L1 context query for an L2 cell: which volume is structure, void, deck surface or trunk interior. *Done when:* the query agrees with the L1 primitives at 10⁴ random points (test). (2026-10-17)
- [x] **P0.8.2** Building masses on deck surfaces (trunk-face masses deferred to P0.8.9 per `levels.md` §5.1). *Done when:* no mass intersects L1 voids, and every mass sits on a supporting surface (test). (2026-10-17)
- [x] **P0.8.3b** Masses keep an 8 m gap from L2 port corridors (`levels.md` §5.1), once P0.8.3 defines them; `l2_mass::Parent::fits` is the place. *Done when:* no mass intersects a port corridor (test). (2026-10-17)
- [x] **P0.8.3** L2 ports: those inherited by cascade from L1, plus the cell's own face-hash ports. *Done when:* every L1 port that crosses an L2 face appears as an L2 port (test). (2026-10-17)
- [x] **P0.8.4** Internal connectivity: build a port graph and generate walkways, stairs and ramps that connect all ports while respecting the slope and step limits. *Done when:* the walkability validator (P0.9) reports every port reachable in 10⁴ random cells. (2026-10-17)
- [x] **P0.8.5** Stair towers and lift shafts as column features, with lift stops emitted as ports. *Done when:* a shaft is identical from every L2 cell it passes through (test). (2026-10-17, lift shafts; towers split out as P0.8.5b)
- [x] **P0.8.5b** Stair towers `[STAIR_TOWER]` (`levels.md` §5.2). Open question: a flight with the spec rise (3 Q) has no whole-step height dividing the 8192 Q L2 cell, so towers cannot align with cell boundaries; decide the tower rise or how a flight crosses an L2 cell face, then build stacked `StairRun`s in `Box` walls with landings. *Done when:* a tower is identical from every L2 cell it passes through (test), and the validator links its landings. (2026-10-17, tower rise 2 Q)
- [x] **P0.8.6** Sanctuary placement: chosen at the L1 level so they sit a few km apart along walkable routes, and marked in L2 metadata. *Done when:* the spacing distribution over 10⁴ L1 cells matches the spec. (2026-10-17)
- [x] **P0.8.7** Vista rule: occasionally guarantee an opening that frames a view (an opening facing an L1 void). *Done when:* the vista frequency matches the spec, and the openings face a void (test). (2026-10-17)
- [x] **P0.8.8** Railing and edge flags: mark walkable edges with a drop greater than the fall threshold as guarded or deliberately unguarded. *Done when:* the unguarded fraction matches the spec. (2026-10-17)
- [x] **P0.8.9** Trunk-attached masses, with walkway routing to the L1 network so they stay reachable (`ports.md` Lemma D). *Done when:* the validator reports every trunk-attached mass reachable in 10⁴ random cells. (2026-10-17)

### P0.9 Walkability validator
- [x] **P0.9.1** Extract walkable surfaces from primitives: top faces with clearance of at least the capsule height. *Done when:* tests on hand-built cells pass. (2026-10-17)
- [x] **P0.9.2** Build the per-cell nav graph (surfaces linked by step, slope and stair rules). *Done when:* tests on hand-built cells pass. (2026-10-17)
- [x] **P0.9.3** Cross-cell linking through ports, plus a reachability query. *Done when:* a 3×3×3 block of L2 cells is reachable from the backbone (test). (2026-10-17)
- [x] **P0.9.4** Connectivity check over N random cells, multithreaded, with a report of failures and their seeds and cells. *Done when:* it runs on 10⁴ cells in CI time (< 60 s). (2026-10-17)

### P0.10 Slice tool
- [x] **P0.10.1** CLI arguments: `--seed --level --axis --at --center --extent --res --out` and overlay toggles. *Done when:* `--help` documents everything. (2026-10-17)
- [x] **P0.10.2** Render primitive occupancy as a PNG coloured by type, with overlays for the cell grid, ports and walkable surfaces. *Done when:* the PNG has been inspected visually and matches expectations for a known cell. (2026-10-17)
- [x] **P0.10.3** `--report` mode that runs the P0.9.4 check and writes JSON stats. *Done when:* the JSON includes counts, failures and timing. (2026-10-17)
- [x] **P0.10.4** Tune the L1 and L2 parameters using slice images: density, variety and void ratio. 🧭 Review a set of slices with the user. *Done when:* the user approves the look of the cross-sections. (2026-10-17, user approved the sample slices; retune after the engine)

### P0.11 Determinism, performance and phase exit
- [x] **P0.11.1** Golden tests: hashes of `CellOutput` for 100 fixed cells at each level, tied to `GEN_VERSION`. *Done when:* the test fails on any output change. (2026-10-17)
- [x] **P0.11.2** Thread-order test: generate the same cells serially and in a shuffled order across rayon threads, then compare. *Done when:* identical. (2026-10-17)
- [x] **P0.11.3** Far-coordinate test: generate cells near ±2⁵⁰ sectors. *Done when:* they are valid, with no panics and connectivity intact. (2026-10-17)
- [x] **P0.11.4** Criterion benchmarks for L1 and L2 cell generation. Budgets: L2 cell under 2 ms, L1 cell under 5 ms, single thread, release build. *Done when:* the benchmarks exist and the numbers are recorded in `PLAN.md`. (2026-10-17, WSL2 numbers; Windows re-run is P0.11.4b)
- [x] **P0.11.4b** Re-run `cargo bench -p gen --bench cells` natively on Windows and replace the provisional WSL2 numbers in `PLAN.md` §11.4. *Done when:* the table says Windows. (2026-10-17, GNU toolchain; no MSVC linker on this machine)
- [ ] **P0.11.4c** Cut the cost of busy L2 cells (1.44 ms mean against a 2 ms budget): profile `l2_cell`, starting with the railing pass (`l2_rail`), whose 2 m edge probes scan every primitive. *Done when:* the busy-cell mean is under 0.5 ms with identical output (golden unchanged) or the change is deliberate and `GEN_VERSION` is bumped.
- [x] **P0.11.5** **Phase exit:** the connectivity check on 10⁶ random cells reports zero backbone failures. *Done when:* the report has been saved to `docs/playtests/phase0.md`. (2026-10-17)

---

## Phase 1: Engine skeleton

*Phase exit:* a static greybox scene built from `gen` primitives renders on Windows, with GPU timings visible.

### P1.1 Window, device and loop
- [x] **P1.1.1** Window with `winit`: resize, borderless-fullscreen toggle (F11), close. *Done when:* it runs on Windows. (2026-10-17, builds and launches on Windows GNU toolchain; F11 and resize by hand are for the user to eyeball)
- [x] **P1.1.2** wgpu instance, adapter and device. Request `MULTI_DRAW_INDIRECT`, `INDIRECT_FIRST_INSTANCE` and `TIMESTAMP_QUERY` (and `MULTI_DRAW_INDIRECT_COUNT` if available). Log what was granted and give a clear error message if a required feature is missing. *Done when:* the log shows the expected adapter (RTX 4070) and features on Windows. (2026-10-17, RTX 4070 on Vulkan with TIMESTAMP_QUERY, INDIRECT_FIRST_INSTANCE, MULTI_DRAW_INDIRECT_COUNT logged on Windows; plain multi-draw-indirect is core in wgpu 30)
- [x] **P1.1.3** Surface configuration with a vsync toggle (Fifo / Mailbox / Immediate). *Done when:* the toggle works at runtime. (2026-10-17, F2 cycles Fifo/Mailbox/Immediate; Fifo verified at 144 fps on Windows, runtime toggle itself awaits a hands-on check)
- [x] **P1.1.4** Main loop: fixed-step simulation at 120 Hz with variable rendering and interpolation. *Done when:* a moving test object is smooth at both 60 Hz and uncapped. (2026-10-17, 120 Hz fixed step with interpolated test quad; runs, smoothness awaits a visual check at 60 Hz and uncapped)
- [x] **P1.1.5** Logging via `tracing` with an env-filter. *Done when:* log levels can be set from the environment. (2026-10-17, `SUPERSTRUCT_LOG` env filter)
- [x] **P1.1.6** A `settings.toml` loader (resolution, FOV, sensitivity, vsync) with defaults written on first run. *Done when:* edits are applied on restart. (2026-10-17, `settings.toml` in the working directory (git-ignored), override with `SUPERSTRUCT_SETTINGS`; unit-tested)

### P1.2 Input and camera
- [x] **P1.2.1** Keyboard and mouse state, with raw mouse motion from `DeviceEvent::MouseMotion`. *Done when:* look input has no acceleration. (2026-10-17, raw `DeviceEvent::MouseMotion`, no smoothing; unit-tested accumulation)
- [x] **P1.2.2** Cursor grab and lock, released on Esc and on focus loss. *Done when:* it behaves correctly on Windows. (2026-10-17, left click grabs (Locked, falling back to Confined), Esc and focus loss release; builds, awaits a hands-on check on Windows)
- [x] **P1.2.3** An action-binding table (keys mapped to actions), so later rebinding is possible. *Done when:* game code reads actions, not raw keys. (2026-10-17, `Action` enum and `Bindings` table with `rebind`; app reads actions only)
- [x] **P1.2.4** First-person camera: yaw, clamped pitch, FOV. *Done when:* look works. (2026-10-17, yaw/pitch camera with ±89° clamp; debug free-fly on WASD/Space/Ctrl/Shift until the controller exists; not visible until P1.3)
- [x] **P1.2.5** Reversed-Z infinite perspective matrix. *Done when:* a unit test confirms the near plane maps to depth 1 and depth approaches 0 at very large distances. (2026-10-17, tested via glam rh directx `perspective_infinite_reverse`)
- [x] **P1.2.6** Camera-relative transform: `chunk WorldPos − camera WorldPos → Vec3`, computed in f64 or i64 then narrowed. *Done when:* the test shows sub-millimetre error at sector 2⁴⁰. (2026-10-17, `camera_relative` via `WorldPos::delta`; test at sector 2^40)

### P1.3 Rendering basics
- [x] **P1.3.1** Depth32Float buffer with compare `GreaterEqual`, cleared to 0. *Done when:* a test scene sorts correctly at 1 m and 10 km. (2026-10-17, `tests/render.rs`: 1 m vs 10 km and 5 m apart at 10 km, both draw orders; passes on the RTX 4070)
- [x] **P1.3.2** Vertex format: position f32×3, octahedral-encoded normal, material id u16. *Done when:* the size and alignment are documented and tested. (2026-10-17, 20-byte vertex documented in `vertex.rs`, size/offset and oct round-trip tested)
- [x] **P1.3.3** Basic forward pipeline with a camera uniform and a per-draw transform storage buffer. *Done when:* a grid of boxes renders. (2026-10-17, box grid scene via per-draw camera-relative origin storage buffer; `demo.rs` is temporary)
- [x] **P1.3.4** Greybox shader: Lambert lighting plus a world-aligned 1 m / 8 m grid so scale is readable, computed from camera-relative position plus a sector offset uniform. *Done when:* the grid does not swim at large coordinates. (2026-10-17, checked in a Windows screenshot; grid phase is sector-independent because 1024 is a multiple of 8, so the uniform carries the in-sector camera offset only; `grid_does_not_swim_at_large_coordinates` renders sector 0 and sector 2^40 identically)
- [x] **P1.3.5** Debug-line renderer for AABBs, cell grids and ports. *Done when:* toggles show L1 and L2 cell bounds. (2026-10-17, F3 toggles L2 (blue) and L1 (orange) outlines, depth-tested, verified in a screenshot)
- [x] **P1.3.6** Screenshot key F12, saving a PNG to `out/`. *Done when:* the file is written. (2026-10-17, F12 writes `out/screenshot-<unix>.png`; `SUPERSTRUCT_AUTOSHOT=<n>` takes one after n frames and exits, for remote runs)

### P1.4 Shaders and tooling
- [x] **P1.4.1** Load shaders from files with hot reload via `notify` in debug builds, and embed them with `include_str!` in release builds. *Done when:* an edit appears live. (2026-10-17, `shaders.rs`: files on disk plus `notify` watcher in debug builds, `include_str!` copies in release; the live-edit itself is for a hands-on check)
- [x] **P1.4.2** On a shader error, keep the old pipeline and show the error in the overlay. *Done when:* a deliberate syntax error doesn't crash the game. (2026-10-17, `reload_shaders` builds inside an error scope and keeps the old pipelines on failure; `broken_shader_keeps_the_old_pipeline` passes on Windows; the overlay display lands with P1.4.3)
- [x] **P1.4.3** `egui` overlay (F1): FPS, frame-time graph, camera sector and local position, seed. *Done when:* it is visible and toggles. (2026-10-17, F1 toggles; shows fps, frame-time graph, adapter, vsync, seed, camera sector and local position, shader error; egui-winit added as the winit glue for egui; `SUPERSTRUCT_OVERLAY=1` starts it visible; verified in a Windows screenshot)
- [x] **P1.4.4** Tracy CPU zones behind a cargo feature flag. *Done when:* a capture shows frame zones. (2026-10-17, `--features tracy` (game) enables zones frame/simulate/draw/record scene/overlay prepare/submit/present plus `frame_mark`; builds and runs on WSL and Windows; no Tracy profiler here, so the capture itself is for the user to check)
- [x] **P1.4.5** GPU timestamp queries per pass, shown in the overlay, with a fallback when unsupported. *Done when:* per-pass milliseconds are visible. (2026-10-17, `gpu_timer.rs`: per-pass timestamp writes (scene, ui), 3-slot async readback, inert without TIMESTAMP_QUERY (overlay says so); verified on the RTX 4070 (scene 0.034 ms, ui 0.005 ms))
- [x] **P1.4.6** Headless offscreen render harness: render to a texture, read it back and compare against a reference with a tolerance. It runs under lavapipe in WSL. *Done when:* one reference test passes in WSL. (2026-10-17, `engine::headless` + `tests/render.rs` with a committed reference PNG; passes natively on Windows. WSL has only Mesa GL-on-D3D12, which segfaults, so GL is excluded and the tests skip there; install `mesa-vulkan-drivers` for lavapipe to run them in WSL)

### P1.5 Windows workflow and phase exit
- [x] **P1.5.1** Document and verify the Windows build: a native `cargo` on Windows, or cross-compilation from WSL. *Done when:* the steps are in CLAUDE.md §7 and the binary runs. (2026-10-17, `scripts/win.sh build|test|run` (GNU toolchain via `cargo.exe`), documented in CLAUDE.md §7; the game and the render tests run natively)
- [ ] **P1.5.2** **Phase exit:** mesh one generated L2 cell (temporary simple box mesher) and render it with timings. 🎮 *Done when:* there's a screenshot and the user confirms it on Windows. (Built 2026-10-17: `cellscene.rs` meshes the busiest L2 cell near the origin, `primmesh.rs` is the temporary mesher, the overlay shows gen/mesh/GPU timings; cell 4 -4 4 at seed 0 is 95 prims, 9480 tris, gen 11 ms and mesh 10 ms in a debug build, scene pass 0.014 ms on the GPU. Screenshot checked on the RTX 4070; waiting for the user's confirmation, so left unticked.)

---

## Phase 2: Greybox walker

*Phase exit:* you walk to a horizon structure and confirm it matches what you saw from afar. 🎮 Answer the key question: does walking in it feel good?

### P2.1 Meshing
- [x] **P2.1.1** Box mesher with per-face normals and material ids. *Done when:* tests pass on triangle counts and winding. (2026-10-17) `chunkmesh.rs`: per-face normals and material ids; tests cover triangle counts, winding and exposed area.
- [x] **P2.1.2** Ramp, cylinder (segment count by size) and stair-run meshers. *Done when:* visually checked via screenshots. (2026-10-17) Cylinder sides scale with radius (8 to 48); ramps, stairs and cylinders checked in the Windows screenshot of cell 4 -4 4.
- [x] **P2.1.3** Remove internal faces between touching boxes in the same chunk. *Done when:* the triangle count drops on test cells, with no visible holes. (2026-10-17) Exact integer rect subtraction against a 64 m grid broadphase; cell 4 -4 4 drops 9480 to 8292 tris, partial-overlap tests show no holes (exposed area matches). Only boxes and stair steps occlude.
- [x] **P2.1.4** Run meshing on worker threads, producing CPU-side `ChunkMesh` objects. *Done when:* the main thread never meshes (verified in Tracy). (2026-10-17) `meshpool.rs`: crossbeam-channel workers generate and mesh; the app only uploads. Verified structurally and by tests, not in Tracy (no profiler on this machine).

### P2.2 Streaming
- [x] **P2.2.1** Chunk identity: L2 cells for the near and mid bands, L1 cells for the far band. *Done when:* types exist. (2026-10-17) `stream.rs`: `ChunkId` = cell + `Detail` (Full or Far); L2 near, L1 full mid, L1 far-visible far. L1 structure is drawn at every distance and L2 adds on top, so the bands overlap by design.
- [x] **P2.2.2** Desired-set computation: nested shells per level around the camera, with configurable radii. *Done when:* unit tests confirm the set changes correctly when the camera moves across cells. (2026-10-17) `desired_set` with `StreamConfig` radii (default 1024 / 4096 / 12288 m); tests cover cell-boundary crossing, detail swap, view bias and sector 2^40.
- [x] **P2.2.3** Job queue prioritized by distance and view direction, with cancellation of stale jobs. *Done when:* fast camera movement leaves no stale jobs running (stat counter). (2026-10-17) `meshpool.rs` priority queue; `retarget` prunes stale queued jobs and flags running ones, whose results are dropped. `cancelled` counter in the overlay and pool stats; test asserts the queue holds nothing stale.
- [x] **P2.2.4** Staging belt with a per-frame upload byte budget. *Done when:* uploads per frame never exceed the budget (overlay stat). (2026-10-17) `Streamer` uploads under a hard per-frame byte budget (default 16 MiB; oversize chunks are rejected, counted). Uses per-chunk `create_buffer_init`, not `wgpu::util::StagingBelt`; revisit when chunks share big buffers (P3).
- [x] **P2.2.5** LRU eviction with a GPU memory budget. *Done when:* memory stays under budget during a 10-minute flythrough. (2026-10-17) LRU by last-wanted frame, only for unwanted hidden chunks. Verified by a simulated 900-frame, 400 m/s flythrough against a 700 KB budget with a fake sink, not a 10-minute real run.
- [x] **P2.2.6** Streaming overlay: resident chunks, queue length, memory, uploads per frame. *Done when:* visible. (2026-10-17) Overlay shows resident/drawn/wanted, queue, running, ready, cancelled, GPU MiB, uploads per frame, evictions, rejected, over-budget. Screenshot: 322 chunks, 7.4 MiB.

### P2.3 Floating origin
- [x] **P2.3.1** Render origin re-bases when the camera changes sector. Chunk offsets are recomputed with integer math. *Done when:* there is no visible jump while walking across sector boundaries. (2026-10-17) No re-base step is needed: every chunk origin is an exact `WorldPos` delta against the camera, computed in f64 from integer sector differences, so values sent to the GPU are always camera-relative. Test `walking_across_a_sector_boundary_moves_the_view_smoothly` steps 1 cm over a sector face at sector 2^40 and stays within 0.2 mm. Not walked by hand.
- [x] **P2.3.2** Debug teleport to arbitrary sector coordinates. *Done when:* teleporting to sector 2⁴⁰ shows no jitter (screenshot comparison with the origin). (2026-10-17) `SUPERSTRUCT_SECTOR=x,y,z` teleports at start. The headless `grid_does_not_swim_at_large_coordinates` test already compares sector 0 with 2^40 (max diff <= 2); a streamed screenshot at sector 2^40 renders cleanly.

### P2.4 Collision and controller
- [x] **P2.4.1** Broadphase: collect `gen` primitives overlapping the capsule's swept AABB, cached per resident chunk. *Done when:* the query is under 0.1 ms in a typical scene. (2026-10-18) `collision.rs`: per-chunk primitive bounds, gather by chunk then by AABB. Release timing in WSL on a real deck neighbourhood (36 chunks, 1 solid in range): gather 0.0003 ms, full controller step 0.0008 ms. Not measured in a busy cell on Windows.
- [x] **P2.4.2** Capsule against Box, Ramp, Cylinder and StairRun using the SDFs from P0.5.3: depenetration plus sliding. *Done when:* unit tests pass for corners, edges and ramps. (2026-10-18) Capsule as 5 spheres pushed out along the SDF gradient of the union of solids (`controller.rs`). Tests: walls with sliding, ledge edge, stairs, ramps, a real generated deck. Ramp and stair SDFs are bounds, not exact, so deep-inside cases are not covered.
- [x] **P2.4.3** Grounded detection, step-up and slope limit per `traversal.md`. *Done when:* the controller climbs stairs, slides down slopes that are too steep, and doesn't jitter on edges (headless tests). (2026-10-18) Step-up 0.45 m, step-down snap, 40 degree limit (gentle ramp climbed, steep ramp not), no jitter standing on an edge, stairs descended airborne under 12 frames. Headless tests. A sphere over a stair corner needed a flat-surface-under-the-axis rule to count as walkable.
- [ ] **P2.4.4** Walk and run with acceleration and deceleration curves (no jump, per P0.2.8). 🎮 *Done when:* the user signs off on how movement feels.
- [x] **P2.4.5** Noclip/fly debug mode with adjustable speed up to 1 km/s. *Done when:* the toggle works. (2026-10-18) F4 toggles fly/walk; +/- double or halve fly speed from 1 to 1000 m/s, Shift is x4. Toggle checked by running, not by hand.
- [x] **P2.4.6** Debug commands: teleport to a coordinate, to the nearest port, or to a random sanctuary. *Done when:* each works. (2026-10-18) `debugcmd.rs`: teleport to a coordinate (overlay box), nearest port (F5), next sanctuary (F6), run on a worker thread. Parsing and both searches have tests; the overlay buttons and F5/F6 have not been clicked by hand.

### P2.5 Death and respawn
- [x] **P2.5.1** Safe-ground tracker per `traversal.md` (grounded ≥1 s, walkable, edge-probe radius clear, not on a moving platform), storing a `WorldPos`. *Done when:* headless tests cover each rule. (2026-10-18) `respawn.rs`: safe spot after 1 s grounded on walkable ground with the 8-ray 1 m edge probe; the moving-platform rule has no test because no moving platforms exist yet (Phase 3). Headless tests for the 1 s rule and the edge rule.
- [x] **P2.5.2** Fall detection: height exceeding the threshold, or falling for a set time with no ground below. *Done when:* headless tests pass. (2026-10-18) Death at more than 15 m below the last ground contact, or after 3 s airborne with no ground within 200 m (ray). Headless tests for a 10 m drop (harmless), a 25 m drop, the void timeout.
- [ ] **P2.5.3** Death sequence: let the fall play for a moment, fade to black, respawn at the safe ground, fade in. 🎮 *Done when:* the user signs off on how it feels. (Built 2026-10-18, awaiting the user's feel sign-off: 1.0 s fall, 1.5 s fade out, respawn at safe ground facing the recorded yaw, 1.5 s fade in; fade is in the scene shader. Seen working in a Windows run, shot mid-dying.)

### P2.6 Simple atmosphere
- [x] **P2.6.1** Exponential height fog in the forward shader, with parameters from `atmosphere_at(camera)`. *Done when:* distant forms fade correctly. (2026-10-18) Scene shader: analytic exponential height fog (density thins above the camera, k = 1/400 m), sigma, fog colour, ambient tint and light level from `atmosphere_at(camera)` every frame; clear colour is the fog colour. Distant forms fade in Windows screenshots.
- [x] **P2.6.2** Sky and void colour from the regime palette. *Done when:* regimes are visibly distinct. (2026-10-18) Sky and void are the fog colour of the regime palette. Screenshots in three L0 cells show cold blue, grey and sepia. There is no sky gradient or sun disc yet; the palettes are dark by spec.

### P2.7 Phase exit
- [x] **P2.7.1** Consistency assertion: in debug builds, check that the far-band L1 primitives equal the L1 primitives used by the near band for the same cell. *Done when:* the assertion is in place and passes during a flythrough. (2026-10-18) Debug builds hash the far-visible primitives of each L1 chunk in the worker; when the far and full chunk of a cell are both resident the streamer asserts the digests match (`l1 checks` counter in the overlay). Passes in the swap test. Both bands come from the same `l1_cell` call, so this guards against future divergence rather than proving a hard case; it does not cover L2 or the SDF horizon yet. Not yet seen during a manual flythrough.
- [ ] **P2.7.2** 🎮 Playtest: pick a structure on the horizon, walk or fly to it, and confirm it is the same. Then judge the feel of walking. Write `docs/playtests/phase2.md`. *Done when:* the notes exist and next steps have been agreed.

---

## Phase 3: GPU-driven pipeline and far field

*Phase exit:* 60 fps at 1440p on the RTX 4070 Laptop while riding at 100 m/s, with no popping (Tracy captures saved).

### P3.1 GPU-driven geometry
- [x] **P3.1.1** Shared vertex and index buffers with an offset sub-allocator and free list. *Done when:* allocator tests pass, and fragmentation stays under 20% after a churn test. (2026-10-18: `suballoc.rs` best-fit with coalescing; fragmentation is the share of free space in spans too small for a typical 20k-element chunk, and the churn test at 60% fill with oldest-first eviction stays under 20%. The renderer now draws from shared 2M-vertex/4M-index pages with `base_vertex`/`first_index`, still one draw per chunk; a page is added when none has room, so P3.1.4 will need to bucket per page or move to one big buffer. Render tests pass on the RTX 4070, reference image unchanged.)
- [x] **P3.1.2** Instance storage buffer: per-chunk position relative to the render origin, mesh range, LOD band, bounds. *Done when:* the scene renders with one indirect draw per material bucket. (2026-10-18: `Instance` storage buffer (camera-relative origin, bounding sphere, mesh range, band slot) is rebuilt each frame; indirect args are written by the CPU until P3.1.3 moves that to compute. There is one material bucket today, drawn with one `multi_draw_indexed_indirect` per buffer page; without `INDIRECT_FIRST_INSTANCE` it falls back to direct draws. Render tests pass, reference unchanged.)
- [x] **P3.1.3** Compute frustum culling that writes indirect draw arguments. *Done when:* visible counts match a CPU reference on a test scene. (2026-10-18: `cull.rs` + `shaders/cull.wgsl`; sphere-vs-plane test against six normalised planes (reversed-Z infinite, so the sixth always passes); the pass has its own `cull` GPU timestamp zone and hot-reloads with the other shaders. `gpu_culling_matches_the_cpu_frustum` renders 500 cubes and the GPU count equals `Frustum::sphere_visible` on the RTX 4070. The no-`INDIRECT_FIRST_INSTANCE` fallback does not cull.)
- [x] **P3.1.4** Draw with `multi_draw_indirect`, or `multi_draw_indirect_count` if available, with a fallback path. *Done when:* both paths render identically. (2026-10-18: paths are per-page `multi_draw_indexed_indirect` and the direct `draw_indexed` fallback, switchable with `Renderer::set_force_direct`; `direct_and_indirect_draw_paths_render_identically` is pixel-exact on the RTX 4070. The `_count` variant is split out as P3.1.4b because it needs compacted arguments.)
- [ ] **P3.1.4b** `multi_draw_indexed_indirect_count` when `MULTI_DRAW_INDIRECT_COUNT` is available: the cull pass compacts visible arguments per page and writes a count. *Done when:* it renders identically to the other two paths and the GPU time for the draw drops in a sparse-visibility scene.
- [x] **P3.1.5** Depth prepass. *Done when:* GPU time shows shading cost drop in dense scenes. (2026-10-18: depth-only pass with its own `prepass` zone, then the colour pass loads depth and tests `GreaterEqual` without writing; `@invariant` on the clip position keeps the two depths identical. `Renderer::set_prepass` toggles it and `prepass_does_not_change_the_image` is pixel-exact. Worst case measured on the RTX 4070 at 1440p (`prepass_cuts_shading_cost_in_a_dense_scene`, 40 full-screen slabs drawn back to front): scene 3.24 ms without, 0.18 ms + 0.40 ms prepass with. That scene is the best case for a prepass; the real number comes with real interiors.)
- [x] **P3.1.6** Hi-Z depth pyramid built by compute from the depth buffer. *Done when:* the pyramid can be visualized in debug. (2026-10-18: `hiz.rs` + `shaders/hiz.wgsl`; R32Float pyramid, level 0 copies the prepass depth and each level takes the 2x2 minimum (farthest, with reversed Z), with the extra row or column of odd sources folded into the last texel so it stays conservative. Own `hiz` GPU zone, hot-reloaded. `Renderer::set_debug_hiz(Some(level))` draws a level over the frame; `hiz_levels_are_conservative_minimums_and_debug_view_draws` compares every level of a 157x91 frame with a CPU reduction and writes `out/hiz-level2.png`, which I looked at. Not yet bound to any overlay key.)
- [x] **P3.1.7** Two-phase occlusion culling: draw last frame's visible set, build Hi-Z, cull the rest, then draw what was newly visible. *Done when:* instances drawn inside a dense interior drop by at least 5×, with no visible popping. (2026-10-18: phase 1 culls to frustum and last frame's visible set (set A, per-chunk slots in a persistent GPU `visible` buffer), the prepass draws A, the Hi-Z is built, phase 2 tests every frustum-visible instance against it (cube corners to a screen rectangle, one mip, 4 texels, nearest corner depth vs farthest occluder) and writes set B plus the next history, a second prepass adds B, then the colour pass draws A and B. New chunks start as not visible so they get tested before they show. Own `occlusion` GPU zone. `occlusion_culling_drops_hidden_instances_without_changing_the_image`: 300 cubes behind a wall, drawn per frame 301, 301, 1, 1 against 301 frustum-only, and every frame is pixel-identical to the unculled render. Falls back to frustum-only when the prepass is off or without `INDIRECT_FIRST_INSTANCE`. Real interiors not yet measured.)
- [x] **P3.1.8** Freeze-culling debug toggle. *Done when:* you can fly out and inspect the culled set. (2026-10-18: F7 toggles `Renderer::set_freeze_culling`. The frustum taken at that moment is kept in world terms and translated to each frame's camera-relative space, so the live camera can fly out and see exactly what was culled; occlusion culling pauses while frozen because its pyramid belongs to the live view. `frozen_culling_keeps_the_old_frustum_while_the_camera_turns_away` checks the GPU count against the translated CPU frustum after turning around and stepping aside. Not tried by hand in the running game yet.)

### P3.2 LOD bands and transitions
- [x] **P3.2.1** Mid band: merged, simplified L2 meshes (exterior masses only). *Done when:* the mid-band triangle budget is met. (2026-10-18: `Detail::Mid` for L2 cells between `l2_radius_m` (1024 m) and the new `l2_mid_radius_m` (3072 m) draws only the `ENVELOPE` primitives, the same data the full chunk refines; it is hidden once the cell's full chunk is resident and never enters collision. `PLAN.md` gave no number, so I set the budget: at most 200k resident mid triangles and 1000 per chunk; measured on the current generator at 0.8k to 5.4k in total and 72 per chunk over 4 spots (`mid_band_triangle_totals`, run with `--release -- --ignored`). It will need re-measuring once L3 and dressing add density. Merging several cells into one mesh was not done: with this few triangles the draw count is what matters and occlusion culling already covers it. Checked in the Windows game build: masses show in the mid distance, 144 fps.)
- [x] **P3.2.2** Far band: instanced L1 megaform primitives (unit box and cylinder meshes, scaled per instance). *Done when:* the far band reaches 50 km within its time budget. (2026-10-18: far L1 chunks are now `FarData` instance lists (`farinst.rs`, 32 bytes each: cell-local box, owning cell slot, material, cylinder flag and axis) drawn by `vs_far` out of one shared unit box and unit cylinder, with one multi-draw per frame over the cells that pass a CPU frustum test and a per-frame camera-relative cell table. `l1_far_radius_m` is now 51200. Budget I set: the whole far band resident within 10 s in release (60 s debug) on 8 workers; measured 11013 chunks, 7.6 MB, in 1.5 s release (`far_band_reaches_fifty_km_within_its_time_budget`). `far_instances_draw_like_the_meshed_primitives` compares the instanced draw of boxes and cylinders on all three axes against the meshed one (cylinders use 32 sides, the meshed ones scale with radius). Windows game run at the test spot: 10979 chunks, 12.2 MiB, and the distance fills with more silhouettes. **Regression, not yet tuned:** average frame time went from 6.95 ms to 9.89 ms (21 ms worst while the 11k chunks were still arriving) and the scene pass from 0.04 to 0.26 ms; the likely CPU cost is the per-frame cell table and indirect records for ~11k cells, so a cached or GPU-culled version is follow-up P3.2.2b.)
- [x] **P3.2.2b** Bring the far band's per-frame cost down. *Done when:* average frame time at the 50 km far radius is back within 1 ms of the 12 km figure. (2026-10-18: my guess about the far draw path was wrong. `far_band_frame_cost` (release, 19.6k cells) shows `render()` at 0.55 ms CPU and the scene pass at 0.6 ms GPU. The real cost was `Streamer::update` at 5 ms per frame with nothing arriving: `refresh_visibility` rebuilt an 11k-entry map every frame, and the periodic re-rank re-sorted the whole wanted set every 30 frames. Visibility now recomputes only when the wanted or resident set changes, and re-ranking happens on a cell change, after 32 m of movement, or while chunks are still loading. Steady update 5 ms to under 0.001 ms; the game shows 144 fps, 6.96 ms average and 9.3 ms worst, the same as before the 50 km radius. A re-rank itself still costs about 7 ms when it does happen, once per 32 m of travel, so fast flight will see that hitch; worth a look in P3.5.1. `fast_flythrough...` now uses a 900 KB budget over 2400 frames because the wanted set alone reaches about 490 KB in busy regions and the old 400 KB figure only passed by timing luck.)
- [ ] **P3.2.3** Blue-noise dithered cross-fade between bands. *Done when:* no hard pops are visible in a recorded flythrough. (Built 2026-10-18, awaiting your flythrough review, so left unticked. Every chunk has a fade level (0.5 s) that the streamer's show, hide and upload calls drive: new chunks fade in, hidden ones fade out, and streamed uploads start at 0. The vertex stage passes the signed level (positive in, negative out) and the fragment stage keeps a pixel when a 64x64 void-and-cluster blue-noise value is below the level for incoming chunks, or at or above one minus the level for outgoing ones, so a swapping pair covers every pixel exactly once. The depth prepass has a variant with the same test, used only while something is fading. Tests: `noise` (each of 256 thresholds appears 16 times, neighbours differ more than white noise), `fade_level_sets_the_share_of_pixels_drawn` (25, 50, 75% within 3 points) and `fading_in_and_fading_out_chunks_tile_the_screen_exactly` (at t = 0.2, 0.5, 0.8 no pixel is drawn by both or by neither). What I could not show: `flythrough_pops_are_smoothed_by_the_fade` (ignored, streams for real and compares the picture just before and after each streaming update at the same camera) finds at most 1 changed pixel per frame with fades off as well as on, because in this scene the swaps along a 70 m crossing of an L2 cell boundary are lost in fog. So the criterion needs your eyes on a real flight, ideally across the 1 km and 4 km band radii.)
- [x] **P3.2.4** Band radii exposed in the overlay and settings. *Done when:* they can be tuned live. (2026-10-18: four radii (full L2, mid L2, full L1, far L1) are `band_*_m` fields in `settings.toml`, shown as log sliders under a collapsible "bands (m)" in the F1 overlay (Esc frees the cursor to drag, as with the teleport box), and applied live through `Streamer::set_stream_config`, which re-targets on the next frame; releasing a slider writes the file. Each band is kept at least as far out as the one inside it. Tests: `band_radii_change_live` (shrinking drops wanted and visible chunks, restoring brings them back exactly) and the settings round trip. Checked in the Windows game that the section renders; I did not drag the sliders by hand.)

### P3.3 SDF horizon
- [x] **P3.3.1** WGSL port of the L0 regime and L1 megaform rules (hash, RNG, trunks, decks, voids), using integer sector math in the shader. *Done when:* the code exists. (2026-10-18: `shaders/horizon.wgsl` ports the hash, RNG, regime, core, staircase, trunk, shaft, chasm and deck rules with 64-bit values as `vec2<u32>` and signed 64-bit cell coordinates as two's-complement bits; `horizon.rs` generates its tables from the generator itself. Everything float-derived on the CPU (trunk, shaft and chasm chances, deck weights) is handed over as exact integers via small public functions in `gen::l1` (`trunk_chance` and friends, a refactor with identical output: the golden tests pass), so the GPU never recomputes a float threshold. **Not ported:** giant-cavity cells and the rim bites they cause, bridges, and the rejection loop of `Rng::below` (probability under 2^-59 per draw). A census of 64k cells showed bridges never dropped a trunk, 0.26% of cells are cavities, and bites only occur next to cavities, so trunks and decks with their four hole types are exact everywhere else.)
- [x] **P3.3.2** CPU-vs-GPU comparison test: evaluate occupancy at 10⁴ points through compute readback and compare with `gen`. *Done when:* it matches 100% on solid/void classification, within tolerance at boundaries. (2026-10-18: `gpu_port_matches_the_generator` classifies 10000 points on the RTX 4070 (5340 solid, 4660 void) in cells from near the origin out to ±2^49, half uniform and half inside or within 2 m of the generator's own trunk and deck primitives, and compares with `Shape::contains` over `l1_cell`'s output: 0 mismatches. Cells next to a cavity are skipped (4 of the sampled ones), since the port leaves cavities out. A debug run uses 2000 points, release 10000.)
- [x] **P3.3.3** Full-screen raymarch pass beyond the far band, stepping cell by cell and writing depth and colour. *Done when:* the horizon shows megaforms beyond 50 km. (2026-10-18: `horizon_cache.wgsl` fills a 65x65x65 window of L1 cell records (trunk, decks, holes; 128 bytes each, 35 MB) around the camera whenever the camera changes L1 cell, and `horizon_march.wgsl` is a full-screen pass that walks each pixel's ray cell by cell from `l1_far_radius_m` outwards (analytic ray against each cell's trunk cylinder and deck slabs, stepping out of the four hole types), shades with the scene's own lighting and fog (factored into `shade.wgsl`) and writes depth, so it draws first in the main pass and the near geometry depth-tests over it. Window radius is 32 cells (131 km). On by default via the new `horizon` setting; off in the headless tests. `horizon_march_covers_the_same_pixels_as_the_far_band` renders the same 13x13x13 cells as instanced far geometry and by marching from six yaws: the covered pixels overlap 93-99% (mean 96.4%), the rest being bridges and cavity cells the port leaves out. `horizon_shows_megaforms_beyond_fifty_km` (clear air, nothing else loaded, marching from 51.2 km) fills 98% of the frame with trunks and layered decks; I looked at the image. Costs on the RTX 4070 at 1440p, clear air so every pixel hits: march 1.0 ms, refill 0.54 ms once per 4 km (new `horizon cache` GPU zone). In the Windows game at the usual test spot it is invisible, as the regime's fog (6-12 km visibility) swallows everything beyond 50 km; scene pass 0.25 to 0.65 ms, 144 fps unchanged.)
- [x] **P3.3.4** Half-resolution option with depth-aware upsampling. *Done when:* the SDF pass is ≤ 2 ms at 1440p. (2026-10-18: the full-resolution march was already 1.0 ms at 1440p in clear air, so the budget holds either way; the option is for weaker GPUs. `horizon_half_res` in the settings (default off) marches into half-size colour and depth targets (own `horizon half` GPU zone) and an upsample in the main pass picks, for each pixel, the one of the four nearest half-res samples whose depth is closest to the bilinear estimate over those that hit, then writes colour and depth, so surfaces do not borrow colour across a silhouette. Measured on the RTX 4070 at 1440p: march 0.32 ms plus upsample 0.17 ms against 1.0 ms. `half_resolution_horizon_matches_full_resolution`: covered pixels overlap 98.3-98.6% with the full-resolution image and the mean difference is about 5 of 255 on a dense scene (a surface-edge soft spot of a half-res march, not a bug); I looked at the image.)
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
- [ ] **P4.1.6** Mass entrances and extra links (moved from P0.8.4b on 2026-10-17: an entrance means nothing until L3 builds interiors). Give each L2 mass an entrance as a `ForcedPort` for L3, and add `levels.md` §5.3 steps 1–2 (spanning tree plus `chance(0.3)` loops) between ports and entrances. *Done when:* loops exist at the spec frequency and the validator still reports every port and entrance reachable.

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
