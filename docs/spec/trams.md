# Spec: trams

**Status: draft, 2026-10-18, for P3.4.3 (lines, stations) and P3.4.4 (ride, prefetch). Speeds and acceleration come from `traversal.md` §5 (approved). New numbers that affect feel are marked "playtest" in §11 and change only with the user's agreement (CLAUDE.md §8).**

Where tram lines are, what the generator emits for them, how the cars run, and how the streamer keeps up at 100 m/s. Keys follow `levels.md` §1.2, hashing follows `hashing.md`, geometry follows `primitives.md`. The runtime half (§5–§7) follows the pattern of `lifts.md`: game state that is never persisted and never fed back into the generator.

## 1. Decisions

| # | Decision | Reason |
|---|---|---|
| D1 | **No `GEN_VERSION` bump.** Trams come from a new module `worldgen::tram`. `l1_cell`, `l2_cell`, ports, the validator and every golden digest stay exactly as they are | All static tram geometry is `NO_COLLIDE` and lies in a volume that existing L1 and L2 content provably never enters (§2.5). Nothing existing needs to avoid it, so nothing existing changes |
| D2 | Lines run **at grade on an L1 deck**, along an **internal L2 face plane**, inside one deck region (at most 16 km) | The 8 m on each side of an L2 face is free of every L2 solid, and the first 3.5 m above a deck is free of every bridge. Deck heights only agree inside a region |
| D3 | Rejected: elevated viaducts, lines on bridges, hanging lines under decks | §2.6 |
| D4 | A line exists only where its whole corridor is clear of holes and trunks in every cell it crosses, checked from keyed features alone. Otherwise the next candidate plane is tried, then no line | A deterministic rule over keys, cheap, and provable. Routing around obstacles would need bends, which axis-aligned trams at 100 m/s cannot take |
| D5 | Stations are **not** `Port`s. Each is a `TramStation` record: an internal stop on the deck, like a lift-core stop | A `Port` needs a `FaceKey` and joins two cells (Lemma B). A station joins nothing across a face, and adding it to `cell_ports` would change output |
| D6 | Cars run on a **pure periodic schedule** of (line, game time): two lanes, a loop with a hidden turnaround at each end, constant headway | No state, no calls, and future positions are exactly known, which is what the prefetch needs (§7) |
| D7 | No ride control. You board by walking in through open doors and get off at any station | The schedule is pure, so a player cannot hold or send a car. Zen: nothing to operate |
| D8 | The car is the only collidable tram object, and it exists only at runtime | Keeps D1 true |

## 2. Where lines exist (generator, P3.4.3)

### 2.1 Key and draws: `LayerKey(L1, y, rx, rz)`, W = 4, `[TRAM_LINE]`

New salt `TRAM_LINE = 0x0209` (L1 block, keyed by the deck `LayerKey`), appended to `worldgen::salt` and to `levels.md` §8. The RNG is `Rng::new(hash(seed, L1, layer.hash_coords(), TRAM_LINE))`. Let `decks(seed, layer)` give `n` decks. For each deck `i = 0..n`, bottom to top, these draws are **always** made, whatever earlier draws said, so a deck's line never depends on another deck's outcome:

| Order | Draw | Meaning |
|---|---|---|
| 1 | `chance(0.5)` (`TRAM_CHANCE`) | the deck has a line |
| 2 | `below(2)` | axis of travel: 0 = X, 1 = Z |
| 3 | `chance(0.5)` | the line spans the whole region (`a = 0, b = 3`) |
| 4 | `range_i32(0, 4)` → `a'` | first cell along the axis, if not full |
| 5 | `range_i32(a', 4)` → `b'` | last cell along the axis, if not full |
| 6 | `below(28)` → `start` | first candidate plane (§2.3) |
| 7 | `below(600_000)` → `phase_ms` | schedule phase (§5.2) |

Indices `a, b, column` are region-relative (0..4). Absolute L1 coordinates are `4 · rx + index` (or `rz`). A line has `a ≤ b`, so it covers 1 to 4 cells along its axis. At most one line exists per deck and region layer, so two lines never cross at grade. Lines on different decks are at least 600 m apart vertically.

### 2.2 Frame

For axis Z, *along* is z and *cross* is x. For axis X, along is x and cross is z. All values are in Q in the cell-local frame unless stated.

| Name | Definition |
|---|---|
| Column | the cell column across the axis, `column ∈ 0..4` (region-relative) |
| Plane `P` | the internal L2 face at cross = `512 m · k`, `k ∈ 1..=7`. Never an L1 face |
| Run | cells `(along ∈ a..=b, layer y, cross = column)`, all in the region |
| `top` | the deck's top, identical in every cell of the region layer |
| Along `s` | measured from the min corner of cell `a` along the axis, so `s ∈ [0, (b − a + 1) · 4096 m)` |
| Lane 0 | cross `[P − 6.75 m, P − 2.25 m)`, centre `P − 4.5 m`. Cars travel towards +s |
| Lane 1 | cross `[P + 2.25 m, P + 6.75 m)`, centre `P + 4.5 m`. Cars travel towards −s |
| Median | cross `[P − 2.25 m, P + 2.25 m)`: the boarding strip, and where the L2 face lies |
| Strip | cross `[P − 8 m, P + 8 m)`: the volume reserved for trams (§2.5) |
| Envelope `E` | strip × along `[e_A − 16 m, e_B + 16 m)` × y `[top, top + 3.5 m)`, per cell clipped to the cell |

### 2.3 Candidate scan and clearance rule

Candidates are `c ∈ 0..28` with `column = c div 7` and `k = c mod 7 + 1`. Try `c = (start + i) mod 28` for `i = 0, 1, …, 27`. The first candidate that is **clear** becomes the line. If none is clear, the deck has no line.

A candidate is clear when, for **every** cell of its run:
1. the cell is not a cavity cell, and not a rim cell (`l1_rim::is_rim`). Only computed if the L0 cell has a cavity at all;
2. let `R` = strip cross range × the line's along extent clipped to the cell, in plan. Every footprint in `l1::deck_holes(seed, cell)` (core, staircase, shaft, chasm) keeps at least `TRAM_HOLE_CLEAR` (16 m) from `R` (`Rect::dist2_rect ≥ (16 m)²`);
3. if `l1::trunk(seed, ColumnSpanKey::of(cell))` has a piece in this cell (`trunk_piece(..).is_some()`), the trunk circle keeps at least `r + 16 m` from `R` (`dist2_point ≥ (r + 16 m)²`). The keyed trunk is used even if the bridge router later omits the piece, which is conservative.

Everything read here is a keyed feature (`levels.md` §4.0), so every cell computes the same result without reading any cell's output. Cost: at most 28 × 4 cell checks of keyed features per deck, well under a millisecond. The engine caches lines per `LayerKey`.

**The line never floats or cuts a trunk.** Decks cover the whole cell minus `deck_holes` (and minus rim bites, which rule 1 excludes). `R` misses every hole, so `R` lies on deck pieces at `top` in every cell of the run. Rule 3 keeps every trunk away.

### 2.4 Stations

Station positions are in whole L2 cells along the line, `j` counted from cell `a`'s min corner, with centres at `s = 512 m · j + 256 m` (the middle of an L2 cell):
- first station `j_0 = 1`, last `j_m = 8 (b − a) + 6`, so `J = j_m − j_0 = 8 (b − a) + 5`;
- hops `h = max(1, J div 14)`, and station `i ∈ 0..=h` is at `j_i = j_0 + (i · J) div h`.

Consecutive stations are at least 14 L2 cells (7168 m) apart whenever `h ≥ 2`. That exceeds `V²/A` = 1667 m (a hop of 2560 m or more reaches cruise, and every hop the rule allows is longer), so every such hop reaches cruise.

| Run `b − a` | Stations | Hops (m) | Hop times |
|---|---|---|---|
| 0 | 2 | 2560 | 42.3 s |
| 1 | 2 | 6656 | 83.2 s |
| 2 | 2 | 10 752 | 124.2 s |
| 3 | 3 | 7168, 7680 | 88.3 s, 93.5 s |

Line ends (car-centre turnaround points) are `e_A = s_0 − 64 m` and `e_B = s_m + 64 m`. The track runs from `e_A − 16 m` to `e_B + 16 m`, so the line stays at least 688 m from any L1 face along its axis, and so from the region's cliff edges.

### 2.5 Why the envelope is always empty

`E` has height 3.5 m (56 Q) and the strip is 16 m wide. Every existing producer of geometry stays out of `E`:

| Content | Why it never enters `E` |
|---|---|
| The line's own deck slab | it lies below `top` |
| Other decks | at least 600 m away. `top + 3.5 m` is below the cell top, since deck tops are at most about 3.75 km |
| Core, staircase, shaft, chasm, trunk | rules 2–3 of §2.3. Core walls lie inside the core footprint |
| Rim bites, cavity | rule 1 of §2.3 |
| L1 bridges and trunk landings | a bridge floor is at least 8 m clear of every slab of both cells of its face (`l1_port`, `BRIDGE_SLAB_CLEARANCE`), and its slab is 4 m thick. The lowest bridge underside is therefore `top + 4 m`. The fallback height is a gap midpoint, hundreds of metres from any slab |
| L1 sanctuary zone | inside a 72 m margin of its L2 child |
| L2 masses | `x0 ≥ 8 m`, `x1 ≤ 504 m` in L2-local space (`l2_mass`, `GAP`), and `P` is an L2 face |
| L2 skywalks on the plane `P` | the floor is `top + 4 m · k`. `k = 0` emits nothing. For `k ≥ 1` the 0.5 m slab's underside is at least `top + 3.5 m`, and the stair starts 8 m from the face (`LANDING`) |
| L2 skywalks on faces across the line | `u` keeps a 64 m margin from the tile's edges, so they stay at least 64 m from `P` |
| Trunk-attached masses and their skywalks | masses keep 72 m (`RIM`). A stair ends at most 55 m from the mass |
| L2 lift shafts, stair towers, vistas | 72 m margin |
| L2 railings | they stand on plate edges: hole edges (at least 16 m away), slab edges (above `E`). Edges on L2 faces are skipped |
| L3 | it refines envelopes (masses) only |
| L4, L5 (future) | **new obligation**: place nothing, collidable or not, inside `tram::envelopes(seed, cell)`. To be written into the P4.2 spec |

The car (top at `top + 3.25 m`) therefore never meets static geometry. It passes 0.25 m under skywalk slabs and at least 0.75 m under bridges. The §9.1 tests check this over real cells.

### 2.6 Rejected alternatives, and the version question

- **Elevated viaduct** (the first candidate): it needs at least 6.4 m (headroom, beam, car), so it meets bridges and needs stairs at stations. Worse, L2 masses and skywalk corridors may occupy any deck spot more than 8 m from an L2 face. A collidable viaduct must therefore be visible to L2 as an L1 solid or `KeepOut`, which changes L2 output in every cell near a line, plus L1 output and the golden digests. That route **would** need a `GEN_VERSION` bump.
- **On bridges**: half-bridges join a port to a core, so a straight chain is at most two cells long, with axis-aligned bends.
- **Under decks**: there is no way down through a slab without new deck holes, which would change `deck_pieces`.
- **Across region faces**: deck heights differ by up to kilometres there. A line could only continue as a structure over a cliff.
- **Line state depending on L2 output** (placing a line where L2 happens to have left room): it inverts the hierarchy, costs dozens of L2 cells per line, and makes an L1-scale feature depend on fine detail.

**Limitation accepted:** lines are at most about 16 km long (15.6 km of track). Longer trips change lines on foot or by lift.

## 3. Stations and connectivity

```rust
pub struct TramStation {
    pub line: TramLineId,   // { layer: LayerKey, deck: u8 }
    pub index: u8,          // 0..=h along +s
    pub cell: CellKey,      // L1 cell holding the station centre
    pub s: i32,             // along position in Q (line frame)
    pub boarding: Rect,     // cell-local plan: median × [s − 16 m, s + 16 m)
}
```

- **Footprint.** Boarding happens from the median, over the length of the car. Every station lies in the middle of one L2 cell, at least 240 m from that cell's faces along the line.
- **Connected by construction.** The boarding rect lies on a deck piece at `top` (§2.3). That deck piece is part of N, and Lemma A (`ports.md` §6) connects it to every port of the cell. Static tram pieces are `NO_COLLIDE`, so they cut no plate. The car is runtime and never generator output. Nothing about Lemmas A–D changes.
- **Finer levels.** No `KeepOut` is emitted for L2: §2.5 proves L2 never enters `E`, and a new constraint would change L2 output. L4 and L5 honour `tram::envelopes` (the obligation in §2.5).
- **Why "stations as ports" holds in spirit.** A station is a stop node of the transport graph with a walkable access rect on N, like an internal lift-core stop. It is derived from keys only, so every cell agrees on it.

## 4. Static geometry and API

All pieces are `Box` primitives running along the axis, so a piece clipped at an L1 face gives two boxes with bit-identical shared faces (`levels.md` §1.1 rule 2). All have kind `FeatureKind::TRAM` (new, id 12, append-only) and flags `NO_COLLIDE`. None is `FAR_VISIBLE`. Heights are above `top`.

| Piece | Cross | Along | Height | Material |
|---|---|---|---|---|
| Rails, 2 per lane | 0.25 m wide, centred at lane centre ± 1.5 m | `[e_A − 16, e_B + 16)` | 0–2 Q | `METAL` |
| Median light strips | `[P − 2.25, P − 2.0)` and `[P + 2.0, P + 2.25)` | the same | 0–1 Q | `EMISSIVE` |
| Station plate | the median | `[s − 16, s + 16)` | 0–1 Q | `METAL` |
| Station bars | the median | 0.25 m at `s ± 16 m` | 1–2 Q | `EMISSIVE` |
| Traversers at each end | `[P − 6.75, P + 6.75)` | `[e − 16, e + 16)` | 0–1 Q | `METAL` |

```rust
pub fn tram_lines(seed: WorldSeed, layer: LayerKey) -> Vec<TramLine>;      // ≤ 1 per deck, bottom up
pub fn lines_in_cell(seed: WorldSeed, cell: CellKey) -> Vec<TramLine>;     // lines whose run holds cell
pub fn tram_pieces(seed: WorldSeed, cell: CellKey) -> Vec<Primitive>;      // §4 table, clipped, cell-local
pub fn envelopes(seed: WorldSeed, cell: CellKey) -> Vec<QAabb>;            // E per line, clipped
pub struct TramLine { pub id: TramLineId, pub axis: Axis, pub column: i64 /* absolute */,
    pub plane: Q, pub cells: (i64, i64) /* absolute a..=b */, pub top: Q,
    pub stations: Vec<TramStation>, pub ends: (i32, i32) /* e_A, e_B in Q */, pub phase_ms: u32 }
```

The order is deterministic: lines by deck, then pieces in table order, lane 0 before lane 1, then by along position. The engine adds `tram_pieces` to the mesh of the L1 **full** chunk of the same cell, on the worker that builds it. Collision chunks ignore them because they are `NO_COLLIDE`.

## 5. The car and the schedule (runtime)

### 5.1 Car

Car-local frame: centre at `s_car` on its lane, heights above `top`. Length 32 m (`CAR_LENGTH`), width 4.5 m (the lane).

| Part | Geometry |
|---|---|
| Floor | 3–7 Q (0.19–0.44 m). The step up from the deck is 7 Q = 0.4375 m, under the 0.45 m step-up, so you walk in |
| Walls | 0.25 m thick, from 7 Q to 48 Q (3.0 m), on all four sides. Interior headroom 2.56 m ≥ 2.4 m |
| Roof | 48–52 Q (3.25 m). Below the 3.5 m envelope |
| Doors | two openings on the **median side**, 2 m wide, centred at ±8 m from the car centre, full interior height. Closed by runtime door solids (§5.4) |
| Drawn | `METAL` body, `EMISSIVE` head- and tail-light bands and a ceiling strip. Window bands are visual only: the collision is the wall |

Every part is a `DynamicSolid` with `platform: Some(v)`, where `v` is the displacement over the last step divided by `dt` (as `lifts.md` §7.2). It is kinematic: nothing collides the car against the world.

### 5.2 Schedule as a pure function

Inputs: the `TramLine` and game time `t` (§5.3). The engine computes it in `f64`, never in the generator.

- **Hop** of length `d` at `A` = 6 m/s² and `V` = 100 m/s: if `d ≥ V²/A`, a trapezoid lasting `d/V + V/A`; otherwise a triangle lasting `2 √(d/A)`. Position within the hop is closed-form.
- **Cycle**, per lane:
  - Lane 0: `e_A → s_0`, dwell, `s_0 → s_1`, dwell, …, `s_m`, dwell, then `→ e_B`, followed by the transfer.
  - Lane 1: the mirror image, from `e_B` back to `e_A`, followed by the transfer.

  `DWELL` is 20 s. The **transfer** at an end moves the car across from one lane centre to the other (9 m) in 12 s (`TRANSFER_TIME`), with a smoothstep profile, at rest along the line. `T_cycle` is the sum of both legs.
- **Fleet:** `K = max(1, round(T_cycle / HEADWAY))` with `HEADWAY` = 60 s, and `H = T_cycle / K`. Because the shortest cycle is about 215 s (K = 4), `H` lies in [53, 68] s, which is the longest wait at a station in one direction.
- **Car k ∈ 0..K** is in cycle phase `τ_k = rem_euclid(t + phase_ms / 1000 + k · H, T_cycle)`.
- **Spacing.** Every car follows the same profile `H` apart. A follower reaches a station at least `H − DWELL ≥ 33 s` after the leader left it, when the leader is at least 2400 m ahead (833 m of acceleration plus 100 m/s for the remaining 16 s). Lanes never overlap. A transfer (12 s) ends long before the next car arrives. §9.2 checks the spacing numerically.
- **Example:** a full line has a 15.6 km cycle leg of 267 s (6.5 + 20 + 88.3 + 20 + 93.5 + 20 + 6.5 + 12), `T_cycle` of 534 s and `K` = 9 (`H` = 59.3 s). Riding from the first to the last station covers 14.85 km in 202 s, including the middle dwell.

### 5.3 Time origin

`t` = `sim_tick / 120` seconds, where `sim_tick` is the fixed-step counter (`u64`). It starts at 0 when the world is loaded and stops while the game is paused. It is not saved and has no wall-clock input. The generator supplies only `phase_ms`, so lines run out of step with each other. Every caller that evaluates the schedule at the same `t` gets the same answer, on any thread.

### 5.4 Doors

Doors open over 1.0 s (`DOOR_TIME`), starting 1.0 s after the car stops. They begin closing 2.0 s before departure, and collision is added at the start of closing (as gates do in `lifts.md` §5.2). If a door box appears around a capsule, it depenetrates the capsule towards the side its axis is on: into the car if the axis is inside the interior rect, otherwise into the median. Doors are always closed while `v ≠ 0`.

### 5.5 Clearing the lane (the push rule)

A walker can stand on the rails, and a car at 100 m/s moves 0.83 m per step, more than the capsule's radius. Each step, before the controller runs, for every car that is moving and not carrying the player:
- Sweep the car's plan rect from its start-of-step to its end-of-step position, inflate it by the capsule radius, and take the height range `[top − 0.5 m, top + 3.25 m]`.
- If the capsule overlaps that volume, move the player:
  - for a lane car, across to the median side of that lane, 0.05 m clear of the inflated rect (lane 0 pushes to +cross, lane 1 to −cross);
  - for a car in transfer, along towards the line's interior.

  The velocity component along the push is zeroed. There is no damage and no fall.

The median and the line are inside `E`, which has no static solid (§2.5), so the target spot is always free. A walker in the median is never touched, even with cars passing in both lanes. A sound cue comes with audio (follow-up).

## 6. Riding

### 6.1 Interaction

| Action | Rule |
|---|---|
| Board | walk through an open door. "Aboard" means: `Controller::on_platform()`, the feet's plan point inside the car's interior rect, and the feet within 0.1 m of the floor top |
| Alight | walk out at any dwell. Passing stations are closed because doors are shut while moving |
| `Interact` (E) | on a station's boarding rect, shows a caption: "the tram is here" (a car is dwelling), "a tram is coming" (next arrival in either lane ≤ 30 s), "the tram is far" (otherwise). E means "call" for lifts, and here it means "ask". Nothing else, because nothing can be called |
| Ride control | none (D7). Space and Ctrl do nothing aboard |

Captions use the single line of `lifts.md` §6.4. The first three boardings in a game show "the tram stops at every station". On reaching the last station while aboard, the caption is "the end of the line". A passenger who stays aboard rides the transfer and comes back along the other lane.

### 6.2 What the player sees

Feet 0.44 m above the deck, eye at 2.1 m, the deck sliding by underneath, median light strips streaking at 100 m/s, skywalk slabs and bridges flashing overhead, and stations marked by bars of light. Acceleration takes 17 s (833 m), so the speed arrives quickly and the cruise is the long part.

### 6.3 Safety, death and respawn

| Case | Behaviour |
|---|---|
| Falling aboard | impossible. The acceleration is horizontal and the controller carries the player by the car's displacement, so it never lifts the player off the floor, and `last_ground` follows the car. Walls, roof and closed doors enclose the car |
| Safe ground | never recorded aboard (`traversal.md` §4). The edge probe ignores platform solids (`lifts.md` §7.3). Respawn returns the player to the last spot before boarding |
| Death off the car | the schedule is unaffected. Cars are where `t` puts them |
| Stream readiness aboard | while aboard, the composite solid source reports `ready = true` and gathers the car solids plus whatever world chunks are resident. The enclosure is the car, so the pure schedule never needs a stream hold. Off the car, normal readiness applies |
| Debug teleport, fly mode | the player leaves the car. The car carries on |
| Cavity | lines never touch cavity or rim cells (§2.3). A line always ends in a turnaround |
| End of the line | the car dwells, runs 64 m on, transfers and returns (§5.2). No plan runs out, because the whole line is known |

### 6.4 Sessions

A line gets a session when the plan distance from the player to its envelope is at most 4096 m (`TRAM_ATTACH`, the L1 full radius) and its deck is within 4096 m vertically. It is dropped beyond 5120 m (`TRAM_DETACH`). Lines come from `lines_in_cell` for the L1 cells of the L1 full ring, computed on the chunk workers and cached per `LayerKey`. A session holds no state beyond the line and its cached `T_cycle`, `K` and `H`. Car solids are gathered only for cars within 64 m of the player. Cars are drawn for every session (§8).

### 6.5 Simulation order (120 Hz fixed step)

1. Sessions attach and detach.
2. Per car: previous pose, new pose at `t + dt`, door state.
3. The push rule (§5.5).
4. `controller.step` with the composite source: world, lift solids and tram solids.
5. `survival.update`.

## 7. Streaming prefetch (P3.4.4)

At 100 m/s the camera crosses the 1024 m L2 full radius in about 10 s, and a 512 m L2 cell in 5 s. The schedule gives the car's **exact** future positions, so the streamer is told where the camera will be.

- **Probes.** While aboard, every 256 m of along distance from the car's position to `min(LOOKAHEAD_S` (30 s) of schedule, the end of the cycle leg`)` is a `Probe { pos: WorldPos, lead_s }`. That is at most 12 probes at cruise (3 km ahead). Dwell time counts in `lead_s`.
- **`desired_set(cfg, cam, view_dir, probes, out)`.** Besides the existing rings, it adds the L2 **full** chunks within 512 m (`PROBE_L2_RADIUS`) of each probe, with priority `100 · lead_s + d`, where `d` is the distance from the probe. Duplicates keep the lowest priority: sort by `(id, priority)`, dedup by id, then sort by priority as now. L1 full (4 km) and the mid band (3 km) already reach past the look-ahead, so probes add no L1 cells.
- **Re-rank** while aboard and moving happens every 256 m of camera travel (`TRAM_RERANK_M`) instead of 32 m, or when the probe list changes length. That is 0.4 re-ranks per second instead of 3, so the roughly 7 ms re-rank (`docs/playtests/phase3.md`) lands less often. Probe cells stay in the wanted set until passed, so they are never evicted early.
- **Budgets:**
  - Upload stays at 16 MB per frame and memory at 768 MB. Probes add at most about 120 L2 cells to the wanted set.
  - Main thread: `Streamer::update` including a re-rank stays ≤ 8 ms. If a native run shows more, moving `desired_set` to a worker is the follow-up, and the overrun is reported (CLAUDE.md §4).
  - Throughput at cruise is about 37 new chunks per second across all bands (L2 full about 3, L2 mid about 22, L1 far about 12).
- **Readiness targets** (checked in §9.3):
  - every L2 full chunk within 256 m (`RIDE_COLLISION_RADIUS`) of the camera is resident on every frame of the ride;
  - every L2 full chunk within 512 m of a station is resident at least 10 s before the car stops there.

**Hitch criterion.** No frame above 20 ms during a ride of at least 10 km, from the first to the last station of a full line (14.85 km), measured natively on Windows.

## 8. Horizon and far band

Rails, strips and station marks are `NO_COLLIDE` and not `FAR_VISIBLE`. They are drawn only by the L1 full chunk (within 4 km), never by the far band or the SDF horizon. Cars are drawn only for attached sessions (within 4 km). **Explicit limitation:** a line cannot be seen from more than 4 km away. This is consistent with the horizon promise, which covers far-visible structure: nothing seen from afar changes up close, and a line only appears as near detail, like narrow bridges or core walls. Slow lights moving far away are P5.2.5.

## 9. Tests that define "done"

Generator tests are headless and run in release. They sample with `hash(0x7A11, …)` so they are reproducible, and report counts.

### 9.1 Line consistency (P3.4.3)

"Lines are consistent across cells" means all of 1–4 below.

1. **Agreement.** Sample 2000 region layers with at least one deck (`y ∈ −16..16`, `rx, rz ∈ −64..64`). For each line, list the cells of its run, plus the cells across the plane's neighbour columns and the cells one step past each end. Shuffle them with a fixed permutation and evaluate `lines_in_cell` from scratch, with no cache, on 8 scoped threads. Every run cell must report a line equal (`==`, every field) to `tram_lines(layer)`'s line. No other cell may report it. Two runs in different orders give identical `Debug` text.
2. **Seams.** For each pair of consecutive run cells, the pieces of cell `c` ending at the shared face and the pieces of `c + 1` starting there must match as multisets of `(cross min, cross max, y min, y max, material, kind, flags)`, exactly in Q. No piece leaves its cell, and the pieces' along union covers `[e_A − 16 m, e_B + 16 m)` with no gap.
3. **Clearance.**
   - For every line of the 2000 sampled layers and every run cell: no collidable, non-`LiftShaft` primitive of `l1_cell` overlaps `E` (strict overlap). `DECK` boxes with top `== top` cover the lane-and-median plan rect exactly, by area as in `Parent::fits`. No hole or trunk comes within 16 m of the strip.
   - For 100 lines: every L2 cell meeting `E` (`l2_cell_in`) has no primitive overlapping `E`.
   - Zero failures are allowed.
4. **Stations on N.** For each of those 100 lines and each station, the validator's `check_cell` passes for the L2 cell holding the station. The boarding rect's centre at `top` lies on a plate of L1 origin, in the component that holds the cell's ports.
5. **No output change.** `golden_cells_are_unchanged` passes with `GEN_VERSION` still 3, and the salt uniqueness test passes with `TRAM_LINE`. New golden file `tests/golden/trams.txt`: digests of `tram_lines` for 100 layers and of `tram_pieces` for 100 run cells. From now on a tram output change is a `GEN_VERSION` bump.
6. **Stats** over 10⁴ layers (printed, bounded): share of decks with a line within [0.4, 0.55] (the chance is 0.5 and almost every deck finds a clear plane among 28), share of lines with `b − a = 3` within [0.3, 0.7], and every station spacing as in §2.4.

### 9.2 Schedule (engine, `tram.rs`)

On 50 sampled lines, at 1/120 s steps over two cycles:
- `s` is continuous: each step moves at most `100 · dt + 1e-6` m;
- `|a| ≤ 6 + 0.01` m/s² along the line, and `v ≤ 100 + 1e-3` m/s;
- every dwell holds the car exactly at the station centre (to 1 mm) with `v = 0`;
- doors are open only when `v = 0`;
- `state(t) == state(t + T_cycle)` to 1 mm;
- same-lane cars stay at least `2 · CAR_LENGTH` apart (sampled every 0.1 s);
- results are bit-identical when computed on two threads in different orders.

### 9.3 Ride and streaming (P3.4.4)

1. **Scripted ride** (CollisionWorld + Controller + Survival + tram sessions, synchronous chunk loading, fake clock). Use the first full line found in the §9.1 sample order.

   | Step | Script | Pass criterion |
   |---|---|---|
   | 1 | Stand in lane 0 ahead of an arriving car | pushed into the median. The feet never enter the car rect |
   | 2 | Wait in the median at `s_0`, walk in when the doors are open | aboard within the dwell |
   | 3 | Ride to `s_m` | on every step: aboard, grounded, feet within 0.03 m of the floor. `safe` unchanged. Duration 202 s ± 2 s |
   | 4 | Walk out | on the deck at `top ± 0.03 m`, not on a platform. `safe` updates after 1.5 s |

2. **Coverage** (real `Streamer`, `MeshPool` with synchronous generation, fake sink, fake clock). Ride from `s_0` to `s_m`, calling `update` with probes every 1/60 s. The §7 readiness targets hold on every frame. A control run with probes disabled and the same synchronous pool is reported, to show what the probes add.
3. **Paced smoke test** (`#[ignore]`, release): real worker threads, frames paced so wall time keeps up with the fake clock (about 3.5 minutes). It reports the per-frame main-thread time of `Streamer::update` and the tram update, the number of re-ranks and the misses against the targets. Zero misses is required. Times from WSL are a regression signal only, never a performance number (CLAUDE.md §7).
4. **Native, the actual gate.**
   - `SUPERSTRUCT_TRAM=auto` boards the nearest full line's first station on the first dwell and rides to the last station. `SUPERSTRUCT_TRAM=<rx,y,rz,deck>` picks a line. Then it exits.
   - The log prints each second's worst frame, as `SUPERSTRUCT_AUTOFLY` does.
   - Pass: no frame above 20 ms after the initial load. The Tracy capture goes into `docs/playtests/phase3.md`.
   - Only this run shows real frame times, GPU upload stalls, staging-belt pressure, worker contention with the render thread, re-rank spikes and driver hitches.

## 10. Manual playtest (user only)

Judged natively and recorded in `docs/playtests/`:
- the feel of 100 m/s at 0.44 m above the deck (speed sensation, motion comfort, whether the 6 m/s² (0.6 g) start and stop feel brisk or harsh, and the 17 s to cruise)
- the wait (up to about 68 s) and the dwell (20 s)
- whether being pushed aside reads as gentle
- car size and look, the light strips, and whether a station can be found from afar
- caption wording

## 11. Numbers

| Constant | Value | Used in |
|---|---|---|
| `TRAM_LINE` salt | `0x0209` | §2.1 |
| `TRAM_CHANCE` | 0.5 per deck | §2.1 |
| Full-region chance | 0.5 | §2.1 |
| Candidate planes | 28 (4 columns × `k` 1..=7) | §2.3 |
| `TRAM_HOLE_CLEAR` | 16 m | §2.3 |
| Strip half-width | 8 m (the L2 mass margin) | §2.2, §2.5 |
| Envelope height | 3.5 m (56 Q) | §2.2, §2.5 |
| Lane centres | `P ± 4.5 m` | §2.2 |
| Median | 4.5 m | §2.2 |
| Station spacing | `j` step ≥ 14 L2 cells (7168 m) when `h ≥ 2` | §2.4 |
| End overrun | 64 m past the terminal station. Track +16 m | §2.4 |
| Max line length | about 15.6 km of track | §2.4, §2.6 |
| Rail gauge, rail size | 3 m; 0.25 m × 2 Q | §4 |
| `CAR_LENGTH` × width | 32 m × 4.5 m | §5.1. Playtest |
| Car floor top, roof top | 7 Q (0.4375 m), 52 Q (3.25 m) | §5.1 |
| Doors | 2 × 2 m, median side, ±8 m | §5.1 |
| `V`, `A` | 100 m/s, 6 m/s² | §5.2, `traversal.md` §5. `A` is a playtest number |
| `DWELL` | 20 s | §5.2. Playtest |
| `TRANSFER_TIME` | 12 s | §5.2 |
| `HEADWAY` target | 60 s (actual 53–68 s) | §5.2. Playtest |
| `DOOR_TIME` | 1.0 s, closing starts 2.0 s before departure | §5.4. Playtest |
| Push clearance | 0.05 m | §5.5 |
| Aboard tolerance | 0.1 m | §6.1 |
| Interact caption threshold | 30 s | §6.1 |
| `TRAM_ATTACH` / `TRAM_DETACH` | 4096 m / 5120 m | §6.4 |
| Car solid gather radius | 64 m | §6.4 |
| `LOOKAHEAD_S` | 30 s (about 3 km) | §7 |
| Probe spacing | 256 m | §7 |
| `PROBE_L2_RADIUS` | 512 m | §7 |
| `TRAM_RERANK_M` | 256 m | §7 |
| `RIDE_COLLISION_RADIUS` | 256 m | §7, §9.3 |
| Station readiness lead | 10 s | §7, §9.3 |
| Main-thread streamer budget | ≤ 8 ms per frame | §7 |
| Hitch limit | 20 ms, ride ≥ 10 km | §7, §9.3 |
| Test samples | 2000 layers; 100 lines for L2 and validator checks; 50 lines for the schedule; 10⁴ layers for stats | §9 |
