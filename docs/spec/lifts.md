# Spec: lifts

**Status: draft, 2026-10-18, for P3.4.2. Speeds and the ride acceleration come from `traversal.md` §5 (approved). Numbers that are new here and affect feel are marked "playtest" in §12 and need the user's agreement before they change (CLAUDE.md §8).**

How the lift cars in the L1 lift core work at runtime: when a car exists, where it can go, how the player calls and rides it, and what keeps the player from falling into an empty shaft. Shaft geometry is generator output (`levels.md` §4.0.1, `primitives.md` §2). Everything in this file except §2.2 is **game state**: it lives in the engine, is never persisted, and is never fed back into the generator (CLAUDE.md §3.1).

## 1. Principles

1. **The generator stays pure.** The engine reads the shaft and its stops through pure functions of `(seed, cell)` (§2.2) on a worker thread. Car position, gate state and plans never influence generated output.
2. **One car per shaft, made on demand.** Nobody else lives in the structure, so a car exists only where the player needs one. There is no simulated fleet and nothing to remember.
3. **No open drops.** A core opening never leads into an empty shaft. Openings are closed by runtime gates (§5) unless a car is parked at that stop.
4. **No friction.** A call takes a few seconds, rides go stop to stop with the gentle traversal numbers, and feedback is a single short caption line (§6.4), never a HUD.
5. **Small coordinates only.** Car heights are `f32` metres above a base `WorldPos` that is an L1 cell corner, and the base is moved when the car changes cell (§3.5). The GPU only sees camera-relative origins (CLAUDE.md §3.6).

## 2. The shaft

### 2.1 Geometry recap (L1 core, as implemented)

All in the cell-local frame of an L1 cell, in quanta (Q, 1/16 m) unless stated.

| Name | Definition |
|---|---|
| Footprint `F` | `l1::core_footprint(seed, ColumnKey::of(cell))`: a square, 16–32 m, identical in every cell of the column |
| Wall zone | the outer 1 m of `F` (`CORE_WALL`). Walls are solid except from each stop `s` up to `s + PORT_CLEARANCE` (4 m), on all four sides |
| Strip | at a stop, the wall boxes below the opening end exactly at `s`, so the wall zone is a 1 m floor strip flush with the deck |
| Inner rect `I` | `F` shrunk by 1 m on each side: the empty shaft interior |
| Stops | `l1::core_stops(decks, bridges)`: every deck top and every bridge floor in the cell, sorted, unique |
| Absent | in cavity cells (`cavity::is_cavity`): no core, no walls, no stops |

Decks have `F` as a hole, so at a deck stop the deck edge meets the strip on all four sides. A bridge stop is a core floating between decks: only the side (or the part of a side) where a bridge arrives has floor outside.

### 2.2 Cell plan (pure, engine side)

A new engine module `liftplan` provides one pure function, built only from public `worldgen` functions. It adds nothing to the generator and changes no output, so no `GEN_VERSION` bump.

```rust
pub enum CellPlan {
    OutOfDomain,              // !cell.in_domain()
    Cavity,                   // is_cavity(seed, cell): the core is absent
    Core { footprint: Rect, stops: Vec<StopPlan> },
}
pub struct StopPlan {
    pub y: Q,                         // cell-local stop height
    pub served: [Vec<(Q, Q)>; 4],     // per side -x, +x, -z, +z: open intervals along the side
}
pub fn plan_cell(seed: WorldSeed, cell: CellKey) -> CellPlan;
```

- **Stops.** Run `l1::l1_cell(seed, cell)`. Take the decks from `l1::decks(seed, LayerKey::of(cell, DECK_REGION_LOG2))` and the bridge boxes (kind `BRIDGE`) from the output, then call `l1::core_stops` on them. That is the same function and the same inputs `l1_cell` uses to cut the openings, so the stops match the openings exactly. The test in §11.2 checks this against the wall boxes.
- **Served intervals.** For each stop `s` and side, take the 0.5 m band just outside that side of `F` (for −x: `x ∈ [F.x0 − 8 Q, F.x0)`, `z ∈ [F.z0, F.z1)`). The served intervals are the union, along the side, of every `DECK` or `BRIDGE` box with top `== s` whose plan rectangle overlaps the band. Intervals are clipped to the side and merged. At a deck stop every side is fully served. At a bridge stop only the bridge width is served.
- Cost: one `l1_cell` per cell, a few milliseconds, so it always runs on a worker (§3.3).

## 3. Sessions and plans

### 3.1 Session

A **session** is the runtime state of one core column `(x, z)`: its plan, its gates and at most one car. Sessions are keyed by the L1 column.

| Event | Rule |
|---|---|
| Attach | the plan distance from the player's feet to `F` of a column is ≤ `ATTACH_RADIUS` (256 m). Up to 4 sessions can exist, because cores keep only 64 m from the cell faces and four columns can meet near a corner |
| Detach | that distance is > `DETACH_RADIUS` (384 m) and the player is not on the session's car. Plan, gates and car are dropped |
| Teleport | any debug teleport (F5, F6, overlay command, `SUPERSTRUCT_SECTOR`) drops every session. They re-attach from the new position |

### 3.2 Planned range

- The **focus cell** is the L1 cell y that contains the car while the player is on it, and the cell that contains the player's feet otherwise.
- The **wanted range** is `[focus − 1, focus + 1]` (`PLAN_HALF_EXTENT` = 1 cell, 12 km in all). It is cut at the first `Cavity` or `OutOfDomain` cell in each direction, because the core does not continue through a cavity. Such a cell makes that end **terminal**.
- The plan keeps cells within `PLAN_KEEP` (2) cells of the focus. Beyond that it drops them, except the cell holding the car and the cell holding the car's target.
- If the focus cell itself is a cavity (the player is inside a cavity), the plan is empty and complete. Nothing in that column is reachable on foot.

### 3.3 Requests and workers

- A plan request is a column plus a list of cell y values that are not yet planned. It runs on a new thread through the same pattern as `debugcmd::spawn`: `plan_cell` per cell, with the result sent over a crossbeam channel. The main thread never calls `plan_cell`.
- Each request carries a session generation number. A result for a dropped or re-created session is discarded.
- Requests are made when a session attaches, when the focus cell changes, and for extension (§3.4). The plan always covers at least one full cell beyond the car's cell in the direction of travel, which is at least 2 minutes of ride at cruise and far more than a request takes.
- **Readiness guard.** While a session whose `F` is within `PLAN_READY_RADIUS` (64 m) of the player has no plan for the player's cell yet, the composite solid source (§7.3) reports `ready() == false`, so the controller holds still. This guarantees that gates exist before the player can reach an opening, even after a teleport or spawn beside a core.

### 3.4 Extension: no stop in the requested direction

A deck layer can have 0 decks (`levels.md` §4.2), so whole cells of a core may have no stop. When `go(up)` (or down) finds no stop beyond the car in the plan:

1. If that end of the plan is terminal, nothing happens and the caption reads "the shaft ends here".
2. Otherwise the press is held as **pending**, and the planner requests the next cell in that direction. This repeats one cell at a time until a stop appears, a terminal cell is found, or `PLAN_SEARCH` (8) cells beyond the car's cell have been scanned (one L0 cell height). In the last case that end counts as terminal for this session.
3. When a stop appears, the pending press is applied as `go(dir)`. Only one pending press exists; a new press replaces it.

### 3.5 Lift stops and rebasing

- The `Lift` is given `base` = the min corner of the car's current L1 cell, and the footprint `F` converted to metres. Its stop list is every planned stop as `f32` metres above `base`: `(cell_y − base_cell_y) × 4096 + s / 16`. Stops are multiples of 1/16 m below 2¹³ m in magnitude, so they are exact in `f32`.
- When the car's height leaves `[0, 4096)`, the lift is **rebased** between steps: `base` moves one cell up or down, and the height, target, previous height and every stop shift by exactly ∓4096 m. These subtractions are exact (Sterbenz), so the car's world position is unchanged to the bit and the controller sees no jump.
- When a plan result arrives, the stop list is rebuilt the same way, keeping the car's height and target. The target's cell is never dropped (§3.2), so the target is always still in the list.

## 4. The car

### 4.1 State

`engine::lift::Lift` (already written) holds the kinematics. A session's car adds:

| Field | Meaning |
|---|---|
| `lift: Lift` | height, velocity, target, stops |
| `prev_y: f32` | height at the start of the last step: used for the platform velocity (§7.2) and render interpolation (§7.4) |
| `phase` | `Arriving` (empty, travelling to the call stop), `Parked { stop }`, `Closing { stop }`, `Riding`, `Holding` (stream hold, §8.2) |
| `pending: Option<bool>` | a held `go` direction (§3.4) |

Changes the implementation needs in `Lift`:
- `new_at(height)`, to start between stops.
- `go_to(stop)`, to target one stop directly.
- Acceleration per instance instead of the `ACCEL` constant.
- `rebase(shift)`.
- A hold flag that brakes at the instance acceleration to zero and keeps the target.

### 4.2 Kinematics

| Use | Cruise | Acceleration | Notes |
|---|---|---|---|
| Ride (player aboard) | 30 m/s (`LONG_CRUISE`) | 1.5 m/s² (`ACCEL`) | `traversal.md` §5. 4096 m in about 156 s |
| Empty arrival | 30 m/s | 4.0 m/s² (`EMPTY_ACCEL`) | nobody aboard, so the comfort limit does not apply. 32 m in about 6 s |

The existing braking rule (`BRAKE` = 0.9 of the acceleration, exact landing on the target) and the queueing rule of `go` stay as they are. All accelerations are far below g, so a passenger never leaves the floor, whether the car is braking upwards or accelerating downwards.

### 4.3 Calling: create, travel or keep

A call at stop `S` of a session (§6.2) resolves as follows:

| Situation | Result |
|---|---|
| No car | a new car is created at `S + ARRIVAL_DISTANCE` (32 m), or at `S − 32 m` if `S + 32 m` is above the top of the plan's highest non-terminal cell. It is `Arriving` with `go_to(S)` and `EMPTY_ACCEL` |
| Car `Parked` at `S`, or `Arriving` at `S` | nothing. The caption reads "the lift is here" or "the lift is coming" |
| Car idle or arriving elsewhere, within 32 m of `S` | it travels: `go_to(S)`, `Arriving` |
| Car elsewhere, farther than 32 m, player not aboard | it is discarded, and a new car is created as in the first row |
| Player aboard | the call is ignored (riding uses Space/Ctrl) |

**Why travel from 32 m and not teleport, and why not let a far car travel.** A teleport into the opening is a pop in front of the player. A far car would take minutes (a 600 m approach takes about 40 s at ride acceleration), which is friction. Creating the car 32 m away gives a short, visible descent through the bars (§5) that reads as "the lift answered". This is accepted: from a few spots pressed against the gate, looking steeply up, the moment the car appears 32 m up an unlit shaft can be seen. The playtest decides whether 32 m and 4 m/s² feel right.

### 4.4 Discard

A car is discarded, with no trace and no memory of where it was, when any of these happens:
- its session detaches
- the player is not aboard and the vertical distance between the player's feet and the car top is over `CAR_KEEP` (1024 m, the `l2_radius_m` streaming radius, beyond which its surroundings may not be collidable)
- a debug teleport happens

When the player comes back, the openings are gated, and a call creates a fresh car. Nothing else observes the shaft, so this is indistinguishable from a car that waited.

## 5. Gates

Each stop in a session's plan has a **gate ring**: runtime solids that close the core openings at that stop. Gates are not generator output.

### 5.1 Geometry

- The gate plane lies 0.8 m inside `F` (`GATE_OFFSET`) and is 0.1 m thick (`GATE_THICKNESS`). For side −x the gate is `x ∈ [F.x0 + 0.8, F.x0 + 0.9]`, with `z` running across the ring's extent `[F.z0 + 0.8, F.z1 − 0.8]`. The four sides form a closed square ring that overlaps at the corners.
- Its height runs from `s` to `s + 4 m` (`PORT_CLEARANCE`): the whole opening.
- The 0.8 m strip outside the gate stays walkable from the deck, as a ledge from which to look into the shaft through the bars.
- The gate sits inside the 1 m wall zone, 0.1 m back from the wall's inner face. When it is sunk into the wall below the opening (§5.2), it is fully hidden and has no coplanar faces.
- **Collision:** one box per closed segment. **Drawn** as `METAL` bars 0.1 × 0.1 m at a 0.5 m pitch, standing on a 0.1 m bottom rail. The 0.4 m gaps are narrower than the 0.6 m capsule, so the single collision box is honest.
- Each side is split at its served intervals (§2.2) into **served** segments and **fixed** segments. Fixed segments never open: on a bridge stop, the sides with air outside stay closed even with the car parked.

### 5.2 Opening and closing

| Transition | Rule |
|---|---|
| Car arrives (`Arriving`/`Riding` → `Parked { stop }`) | served segments of that stop sink into the wall below over `GATE_TIME` (1.0 s). Collision is removed at the **end** |
| Departure (`go` pressed while `Parked`) | served segments rise over 1.0 s with collision added at the **start**, then the car moves (`Closing` → `Riding`). A gate box that appears around a capsule depenetrates it towards the car: the capsule's axis is on the car side of the gate plane, because "on the car" (§6.3) puts the feet inside `C`, which lies 0.16 m inside the gate |
| Car discarded or not parked | every segment is closed |

Consequences: the player can never walk into an empty shaft, and can never leave a moving car at a passing stop. Every stop it passes is gated. Rides go stop to stop, and the queueing rule lets the player choose where to stop.

## 6. Interaction

### 6.1 Input

| Action | Default key | Effect (walk mode, cursor grabbed, `Survival::controllable()`) |
|---|---|---|
| `Interact` (new) | E | call the lift at the opening in reach (§6.2) |
| `MoveUp` | Space | while on the car: `go(up)` |
| `MoveDown` | Ctrl | while on the car: `go(down)` |

These are edge-triggered (`InputState::pressed`), never held. In fly mode none of them affects lifts, since Space and Ctrl fly. Toggling to fly mode leaves cars and gates as they are, and the normal rules apply afterwards.

### 6.2 Call reach

A call at stop `s` of session `K` is accepted when:
- the player's feet are within 0.5 m (`CALL_REACH_Y`) of `s`
- the plan point of the feet is within `CALL_REACH` (2.0 m) of a **served** segment, measured to the segment's line on the boundary of `F`. Points on the strip count as distance 0.

If several stops or sessions qualify, the nearest segment wins. No facing check is made. Outside reach, E does nothing and shows no caption.

### 6.3 On the car

The player is **on the car** when `Controller::on_platform()` holds, the plan point of the feet lies inside the car rect `C` (§7.1), and the feet are within 0.1 m (`ON_CAR_TOL`) of the car top. Only then do Space and Ctrl send the car, and only then does the car's cell become the focus cell.

### 6.4 Captions

There is no HUD. A single centred text line near the bottom of the screen is drawn even when the debug overlay is hidden. It stays for `CAPTION_TIME` (2.5 s), fades over 0.5 s, and a new caption replaces the old one.

| When | Text |
|---|---|
| Call accepted | "the lift is coming" |
| Call while parked here | "the lift is here" |
| First three boardings per session of the game | "Space up · Ctrl down" |
| `go` with a terminal end and no stop | "the shaft ends here" |

A stream hold (§8.2) shows no caption: it is an engine fault, logged at `warn` with a Tracy message. Sound cues (call chime, gate, motor hum) are a follow-up once audio exists.

## 7. Geometry, collision and rendering

### 7.1 Car

- **Collision:** one `Shape::Box`, the car rect `C` = `I` shrunk by 1 Q (0.0625 m) on each side, 0.5 m thick (`CAR_THICKNESS`), with its top at the car height.
- **Why the inner rect, not the full footprint.** The wall strip already makes the floor flush from the deck to the shaft interior at every stop. A full-footprint slab would overlap the walls and z-fight with the strip top. The 1 Q inset keeps the car's sides off the wall's inner faces. The gap from the car to the gate is 0.16 m and from the car to the wall is 0.06 m. Both are far narrower than the capsule, so no railing is needed on the car: it is always surrounded by walls or gates.
- **Drawn** as five boxes tiling the same slab: a `METAL` centre and a 0.25 m `EMISSIVE` border, so the car reads in the dark shaft and its approach is visible. Collision is the single slab.
- The car may overlap the lower parts of static walls only in the 1 Q seams. Nothing collides the car against the world: it is kinematic.

### 7.2 Platform velocity

The car's `Solid` carries `platform: Some(Vec3::Y × (y − prev_y) / dt)`: the displacement over the step just taken, not `Lift::velocity()`. That keeps the arrival step (which snaps onto the stop) consistent with the controller's ride-along. While the car is parked, the value is `Some(Vec3::ZERO)`.

### 7.3 Composite solid source

`LiftSolids { world: &CollisionWorld, sessions: &Sessions }` implements `SolidSource`:
- `gather`: the world's solids, plus every closed gate box and every car within `half`. Their offsets are computed from `WorldPos::delta` to the query centre, so nothing large reaches `f32`.
- `ready`: `world.ready(pos)` and the readiness guard (§3.3).

Gate and car `Solid`s are kept in per-session buffers that are rebuilt only on gate or plan events. They are not allocated per step. The safe-ground edge probe (`Survival::edge_clear`) must ignore solids with `platform.is_some()`, so a parked car never makes a spot next to an opening look safe. Gates do count, because they are static.

### 7.4 Rendering

- The car is one dynamic draw. Its mesh is built once from its five boxes with `primmesh::add_primitive`, and its origin is a `WorldPos` set every frame. The renderer already uses per-draw camera-relative origins.
- The drawn height is `prev_y + (y − prev_y) × alpha`, with the same `alpha` the camera interpolation uses. Otherwise the floor under the player would judder by up to 0.25 m per step at cruise.
- Gates are built once per stop as two meshes: fixed and served segments. The served mesh's origin is lowered by `open_fraction × 4 m` while it moves. Meshes are uploaded through the staging belt when a plan arrives.
- GPU timestamp zone "lifts", CPU Tracy zones "lift update" and "lift plan" (worker).

## 8. Simulation order and streaming

### 8.1 Fixed step (120 Hz)

1. Sessions: attach and detach, apply finished plan results, discard cars (§3, §4.4).
2. For each car: `prev_y = y`, stream check (§8.2), `lift.update(dt)`, rebase if needed (§3.5), and phase and gate transitions (§5.2).
3. `controller.step(dt, input, &LiftSolids)`. The car is already at its end-of-step position, as P3.4.1 requires.
4. `survival.update(dt, &controller, yaw, &LiftSolids)`.
5. Interact, MoveUp and MoveDown presses take effect at the start of the next step.

### 8.2 Stream hold

The controller does nothing while `ready(feet)` is false, so a car that kept moving would leave its passenger floating. While a car is moving, before each update, the step checks `world.ready(probe)`. The probe is the car's centre at `y + dir × (v² / (2 × BRAKE × a) + 32 m)`, clamped to the target. If the probe is not ready, the car enters `Holding`: it brakes at its acceleration to zero and waits with its target kept, then resumes when the probe is ready. At the default `l2_radius_m` of 1024 m and a stopping distance of 333 m, a hold means streaming fell more than 30 s behind, so it is a performance bug to report (CLAUDE.md §4).

**Failsafe.** If `ready(feet)` is false while the player is on the car, the car's update is skipped for that step too, so the car and the player freeze together.

## 9. Edge cases

| Case | Behaviour |
|---|---|
| Walking into an opening with no car | blocked by the gate (§5). The ledge strip is walkable |
| Leaving mid-ride at a passing stop | impossible: passing stops are gated |
| Fall death on the car | grounded throughout (|a| ≤ 4 m/s² ≪ g), so `last_ground` follows the car and no fall is counted |
| Safe ground | never recorded on the car (`on_platform`, `traversal.md` §4). The edge probe ignores platform solids (§7.3). During a ride, the respawn point stays where it was before boarding |
| Death or respawn while a car exists | sessions and cars follow the normal distance rules from the respawn position. A player can only die off the car, so no passenger is ever left |
| Debug teleport | drops every session (§3.1). The F5 nearest-port target of a Y-face `LiftStop` port is the middle of the shaft in mid-air. It should skip `LiftStop` ports (follow-up) |
| Plan runs out of stops | §3.4: pending, extension, then "the shaft ends here". The car never moves beyond the highest or lowest planned stop, so it never enters a cavity cell |
| Speed cap | the car never exceeds its cruise speed. Queued presses only change the target, never the speed |
| Two stops less than 4 m apart | kept as separate stops. Their gate rings overlap in height, which is harmless |
| Fly mode | §6.1. A car can be left parked or riding empty to its target, then idles there |

## 10. L2 lift shafts: follow-up (P3.4.2b)

L2 shafts (`levels.md` §5.2, `l2_column::lift_column`) are **out of scope for the first implementation**:
- They are optional, short (a rise of 16–64 m) and local, while the L1 core is the backbone every deck relies on (`ports.md` Lemma A).
- Their data needs the parent's full `CellOutput` (`lift_column(seed, span_of(cell), &l1_cell(parent))`).
- The same session, gate and car rules apply unchanged once the core works.

When they are added:
- **Data:** stops are the `rises` pairs `(deck top, platform)` of `LiftColumn`, in the parent L1 frame, and `F` is `LiftColumn::rect`.
- **Speed:** cruise is `SHORT_CRUISE` (6 m/s) with `ACCEL`, so a 64 m rise takes about 15 s.
- **Bottom stop:** the shaft stands on the deck with no hole. The car's slab rests inside the deck top, which is invisible and flush.
- **Top stop:** the top is open to the sky, so the gate ring and the open opening above it are the only enclosure. The 4 m `GRATING` ring serves all four sides.
- **Session key:** `ColumnSpanKey(L2)`.

## 11. Tests that define "done"

All are headless (no GPU) and use seed 0. Tests that need a real cell **search** for it in a fixed order: L1 cells with `y` from −4 to 4, then `z` from 0 to 7, then `x` from 0 to 7. They take the first match and print its key, so a `GEN_VERSION` change moves the cell instead of breaking the test. A failed search is a test failure. Time step 1/120 s.

### 11.1 Kinematics (`lift.rs`)

1. The existing four tests keep passing.
2. Empty arrival: from 32 m above, with `EMPTY_ACCEL`, `go_to(S)` lands exactly on `S` in 5.0–7.0 s, with peak acceleration ≤ 4.0 + 0.01 m/s².
3. Rebase: a car riding from 4000 m to 4200 m above `base`, rebased at 4096 m. The world-space car origin advances every step by `v·dt` within 1 mm, with no step of more than 0.3 m, and it lands with exact `f32` equality at the stop relative to the new base.
4. Hold: with the hold set at cruise, the car stops within 340 m, |a| ≤ 1.5 + 0.01 m/s², and the target is kept. After the hold is cleared it lands on the target.

### 11.2 Stop extraction against the generator (`liftplan`)

Search for a non-cavity cell with at least two decks.
1. `plan_cell` returns `Core`, and `footprint == core_footprint(seed, ColumnKey::of(cell))`.
2. Stops are ascending and include every deck top.
3. Against the `SHAFT_WALL` boxes of `l1_cell` inside `F`, every stop `s` has no wall box intersecting `(s, s + 4 m)`, and four wall boxes with top `== s`, one per side, unless an earlier stop's opening covers `s`. Every height outside the openings is covered by walls on all four sides.
4. Every deck stop has all four sides served over their whole length.
5. If the searched range has a bridge stop, each of its served intervals is at least 8 m long and lies on one side.
6. `plan_cell` called twice, and on two threads, gives equal results.
7. A cavity cell from the search range gives `Cavity`.

### 11.3 Scripted ride (`CollisionWorld` + `Controller` + `Survival` + sessions)

Search for a cell whose two lowest stops `s0 < s1` are both deck tops. The collision world holds the cell's L1 output, with an empty `CollisionChunk` for each L2 cell the script touches (so `ready` holds). The planner runs inline.

| Step | Script | Pass criterion |
|---|---|---|
| 1 | The player stands on the deck at `s0`, 3 m outside the middle of the −x side, and walks +x for 3 s | the feet never pass the gate plane: `x ≤ F.x0 + 0.8 − 0.3 + 0.01` |
| 2 | Press Interact | accepted. The car is `Parked` at `s0` within 7.0 s and the gates are open 1.0 s later (±1 step) |
| 3 | Walk +x for 4 s | on the car; feet within 0.03 m of the car top |
| 4 | Press MoveUp | gates closed after 1.0 s, then the car rides. On every step of the ride: on the car, feet within 0.03 m of the car top, grounded. The ride takes `(s1 − s0)/30 + 20 s ± 4 s` (when `s1 − s0 ≥ 600 m`, which deck spacing guarantees) and ends exactly on `s1` |
| 5 | After the gates open, walk −x for 8 s | off the car, not on a platform, grounded, feet at `s1 ± 0.03`, at least 2 m outside `F` |
| 6 | Stand 1.5 s | `Survival::safe` updated to this deck. It never changed during steps 3–4, while on the car |

### 11.4 Ride across an L1 cell boundary

Search for two vertically adjacent non-cavity cells `c`, `c + 1` that both have stops. Load both L1 outputs. Start parked at the highest stop of `c` with the player on board, then press MoveUp once. Pass criteria:
- the plan extends to `c + 2`, or marks it terminal, while riding
- exactly one rebase happens
- the player stays on the car within 0.03 m on every step
- the car top ends within 1 mm of the lowest stop of `c + 1` as a world position
- no step's world displacement differs from `v·dt` by more than 1 mm
- if the gap is at least 600 m, the duration is `d/30 + 20 s ± 4 s`

### 11.5 Safe ground and gates

1. Parked car, player standing on it for 5 s: `safe` is unchanged.
2. Player on the strip at the opening of a parked car for 5 s: `safe` is not set there, because the edge probe ignores the car.
3. No car: walking into each of the four openings at `s0` is blocked. The feet stay outside `C` for 10 s of pushing.
4. Bridge stop (if found in 11.2): with the car parked, walking into a fixed segment is blocked.

### 11.6 Session rules

1. Attach at 256 m, detach beyond 384 m (hysteresis checked both ways).
2. Car discarded when the player is not aboard and over 1024 m away vertically.
3. A teleport drops all sessions.
4. A call with a car 500 m away discards it and creates one 32 m away.
5. The readiness guard holds the controller within 64 m of a core until the plan exists.

## 12. Manual playtest (user only)

These are judged by the user on a native Windows run and recorded in `docs/playtests/`. They are the 🎮 part of P3.4.2:
- whether a full 4 km ride (about 2.5 min) feels like a set piece and not a wait: speed, acceleration, the view through passing gates
- the call: does a 32 m arrival in about 6 s feel answered, and is the car's appearance noticeable
- gate timing (1.0 s) and the caption wording
- whether the emissive car border reads well in the dark shaft
- any motion discomfort at 30 m/s with dense geometry close by

## 13. Numbers

| Constant | Value | Used in |
|---|---|---|
| `LONG_CRUISE` | 30 m/s | ride cruise (§4.2), `traversal.md` §5 |
| `ACCEL` | 1.5 m/s² | ride acceleration (§4.2) |
| `BRAKE` | 0.9 | share of acceleration in the braking curve (`lift.rs`) |
| `STOP_TOLERANCE` | 0.5 m | next-stop search in `go` (`lift.rs`) |
| `SHORT_CRUISE` | 6 m/s | L2 shafts, follow-up (§10) |
| `EMPTY_ACCEL` | 4.0 m/s² | empty arrival (§4.2). Playtest |
| `ARRIVAL_DISTANCE` | 32 m | where a new car appears, and the travel-or-recreate limit (§4.3). Playtest |
| `CAR_THICKNESS` | 0.5 m | car slab (§7.1) |
| Car inset | 1 m wall + 1 Q (0.0625 m) | `C` from `F` (§7.1) |
| Car border | 0.25 m, `EMISSIVE` | car drawing (§7.1). Playtest |
| `CORE_WALL` | 1 m | generator, recap (§2.1) |
| `PORT_CLEARANCE` | 4 m | opening and gate height (§2.1, §5.1) |
| Served band | 0.5 m (8 Q) | served intervals (§2.2) |
| `GATE_OFFSET` | 0.8 m inside `F` | gate plane (§5.1) |
| `GATE_THICKNESS` | 0.1 m | gate collision and bars (§5.1) |
| Bar size and pitch | 0.1 m, 0.5 m | gate drawing (§5.1) |
| `GATE_TIME` | 1.0 s | open and close (§5.2). Playtest |
| `CALL_REACH` | 2.0 m (plan) | call (§6.2) |
| `CALL_REACH_Y` | 0.5 m | call (§6.2) |
| `ON_CAR_TOL` | 0.1 m | on the car (§6.3) |
| `CAPTION_TIME` | 2.5 s + 0.5 s fade | captions (§6.4) |
| `ATTACH_RADIUS` | 256 m | session attach (§3.1) |
| `DETACH_RADIUS` | 384 m | session detach (§3.1) |
| `PLAN_READY_RADIUS` | 64 m | readiness guard (§3.3) |
| `PLAN_HALF_EXTENT` | 1 L1 cell | wanted range (§3.2) |
| `PLAN_KEEP` | 2 L1 cells | plan trimming (§3.2) |
| `PLAN_SEARCH` | 8 L1 cells | extension limit (§3.4) |
| `CAR_KEEP` | 1024 m vertical | car discard (§4.4) |
| `MAX_SESSIONS` | 4 | sessions (§3.1) |
| Stream probe margin | 32 m beyond the stopping distance | stream hold (§8.2) |
| Test tolerances | 0.03 m on the car, 1 mm world continuity, ±4 s ride time, 5.0–7.0 s arrival | §11 |
