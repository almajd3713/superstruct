# Spec: gravity wells, the field `g(p)`, gates, lulls and drift

**Status: PW.0.6, 2026-10-18.** Numbers marked *(provisional)* are player-feel numbers for the user's playtest (CLAUDE.md §8). Approved by the user on 2026-10-18: wells about every 50 km, a smooth camera roll at gates, lulls with both handrail tubes and free drift. Architecture: `docs/proposals/the-web.md` §8; low gravity: `the-web-ideas.md` §8. Implementation: `worldgen::gravity` (PW.6.1, built 2026-10-21); the controller and engine parts are PW.6.2 and later.

**Owners:** WELL-01..10 (PW.6.5–PW.6.10), GRV-01..03, GRV-09 (PW.6.4), GRV-04..08 (PW.6.3), GRV-10, GRV-14 (PW.6.8), GRV-11, GRV-13 (PW.6.2), GRV-12 (PW.6.1), WGT-01..09 (their items in PW.6.x and PW.11).

## 1. The field

Gravity at a world position is a pure function of position and the world seed (GRV-12):

```rust
pub struct Gravity {
    pub up: IVec3,      // integer direction, max-norm 8, or (0,0,0) in a lull (§1.2)
    pub strength: u16,  // 0..=256, 256 = standard gravity 9.81 m/s²
}
pub fn gravity_at(seed: u64, p: WorldPos) -> Gravity
```

- `up` is the direction of **anti-gravity**: where "up" points. Down is `−up`.
- Outside every well, `up = (0, 8, 0)` and `strength = 256`: the world of version 6.
- The player's local gravity vector is `g = 9.81 · (strength/256) · (−up/|up|)`.

### 1.1 Integer directions

All directions in the field are integer vectors with **max-norm 8**: components in `−8..=8`, at least one of magnitude 8. A real vector `v` is quantised by `u_i = floor((16·v_i + m) / (2m))` with `m = max |v_i|` and floor division (`div_euclid`), which rounds to the nearest integer. This is exact integer arithmetic. The quantisation step is about 3.6°, so a planetoid is faceted at large scale (GRV-01), and a walker turns smoothly because the controller interpolates between steps (§6).

The unit vector for the frame is `up / sqrt(x² + y² + z²)` in `f64` (`primitives.md` §9.1.3 uses the same recipe, `Frame::from_up`).

### 1.2 Wells add; the default gives way

Every well `w` contributes a vector `c_w(p) · s_w · u_w(p)` where `c_w ∈ 0..=256` is its **coverage** at `p` (§2.2), `s_w ∈ 0..=256` its strength and `u_w` its direction (§2.1). The default gravity contributes `c_def · 256 · (0, 8, 0)` with `c_def = max(0, 256 − Σ c_w)`. If `Σ c_w > 256` the well coverages are scaled by `256 / Σ c_w`, integer division, in the order of the wells (§3.2). The sum is

`v(p) = Σ contributions` (all `i64`).

Then, with `m = max |v_i|`:
- `strength = min(256, m / 2048)`. A full contribution has coverage 256, strength 256 and direction magnitude 8, so `m = 256 · 256 · 8 = 2^19` and `m / 2^11 = 256`. Where contributions cancel, `m` falls to 0;
- `up = quantise(v)` if `strength ≥ 4`, else `(0, 0, 0)` (a **lull**).

Opposed wells (twin wells, shell wells at their centre) cancel smoothly, so **lulls** (GRV-10) arise from the sum with no special case. Aligned wells add, capped at 256.

## 2. Well kinds

```rust
pub struct Well {
    pub kind: WellKind,     // §2.1
    pub centre: IVec3,      // world metres, 16 m step, relative to the L0 cell
    pub axis: u8,           // index into the 49-entry direction table (line, tube, plane)
    pub reach: u32,         // metres, 2000..=16384
    pub strength: u16,      // 0..=256
    pub extent: IVec3,      // half-sizes for box-shaped regions (plane, seam, fold)
    pub l1: I64Vec3,        // the L1 cell that holds the furnace (knot) §3.3
}
```

### 2.1 Directions and metrics

Let `r = p − centre`, in metres (`i64`). `oct3(r) = max(m∞, 4·S/7)` with `m∞ = max |r_i|`, `S = Σ |r_i|`. `oct2` is the same for two components. Both are exact integer metrics whose unit ball is a faceted sphere or disc (error ≤ 14 %).

| Kind (ID) | `u_w(p)` (up) | Distance `d` used for coverage | Active region |
|---|---|---|---|
| **Point** (WELL-01) | away from the centre: `quantise(r)` | `oct3(r)` | the ball `d < reach` |
| **Shell** (WELL-02) | towards the centre: `quantise(−r)` | `oct3(r)` | the ball `d < reach`; at the centre `r = 0`, so there is no direction and the contribution is zero (a weightless hollow, §5) |
| **Line** (WELL-03) | away from the axis: `quantise(r_perp)` | `oct2(r_perp)` | a cylinder along the axis of half-length `extent.y · 16` |
| **Tube** (WELL-04) | towards the axis: `quantise(−r_perp)` | `oct2(r_perp)` | as the line well |
| **Plane** (WELL-05) | a fixed direction from the direction table (`axis`) | box distance | the box of half-sizes `extent` around the centre |
| **Twin** (WELL-06) | two Point wells, `w` and `w'` | each its own | the pair is one key (§3.1) |
| **Seam** (WELL-07) | `+n` on one side of a plane, `−n` on the other | box distance | two Plane wells with opposed directions, sharing a boundary plane; **no taper** on the shared plane; the membrane is a wall 16–64 m thick with a gate |
| **Fold** (WELL-08) | the up of the segment of a corridor polyline, turning by 90° about the corridor's axis from one segment to the next | tube distance to the polyline | a tube along the polyline |
| **Damaged** (WELL-09) | per 256 m patch: a table direction and strength from the patch's own key | patch cube | a ball of patches inside `reach` |
| **Tidal** (WELL-10) | towards the centre: `quantise(−r)` | `oct3(r)` | strength falls linearly with `d`: `s = s_w · (1 − d / reach)` |

- `r_perp` is `r` minus its component along the axis `a`: for an axis-aligned `a` it is `r` with that component zeroed; for another table direction `a = (ax, ay, az)` the unnormalised `|a|²·r − (r·a)·a` is used (all `i64`), then quantised.
- A Point well gives a planetoid of concentric shells with the furnace in the middle (WELL-01). A Shell well is its inverse: the inside of a sphere, the opposite wall overhead (WELL-02).
- **Fold** corridors are built by L1 as a polyline of straight segments (turns of 90° in the axis plane) and keyed by the well's key; the field makes the corridor's walls and floor swap at each bend. The corridor is a tube artery (`levels.md` §4W.5.3).
- **Damaged** has a *slow pulse* as a **rendering and sound effect only**; the field itself stays static, because `g(p)` is a function of position.

### 2.2 Coverage and gradients

For a well with reach `R` and distance `d`, with `t = clamp((R − d) · 4 / R, 0, 1)` as a fraction in 1/4096 (integer), the coverage is

`c = 256 · S(t)` where `S` is the smoothstep polynomial `t²(3 − 2t)` in fixed point,

so coverage is 256 in the inner three-quarters of the reach and tapers to 0 at `R`. In the taper the contributions of the well and the default gravity mix (§1.2): the field turns gradually (GRV-09). **A walker crossing a planetoid's curve sees no jump**, and the same holds at the outer taper: `up` rotates over `R/4` (500 m to 4 km) from the default up to the well's direction. The transitional strength is the vector sum, so it dips where the directions oppose: a well with a horizontal contribution beside default gravity never has a zero-strength seam unless the contributions are exactly opposite.

### 2.3 Strength

`strength` is drawn per well from 96..=256 in steps of 16 (Tidal: 128..=256 at the core). A weak well (below 160) is a low-gravity district (WGT-08).

## 3. Placement

### 3.1 Per L0 cell

An L0 cell holds a well with probability 1/4 (`chance(0.25)` from `hash(seed, 0, cell, WELL)`), which gives a mean spacing of about 51 km (32 km / 0.25^(1/3)). Draws, in order:

1. `kind` by `pick_weighted` over `[Point 20, Shell 8, Line 22, Tube 12, Plane 12, Twin 8, Seam 5, Fold 5, Damaged 5, Tidal 3]`;
2. `centre` within the inner 6×6×6 L1 cells of the L0 cell (x, y, z each on a 16 m step), redrawn up to 8 times if its L1 cell is a cavity cell (`levels.md` §3); if all 8 draws fall in cavity cells there is no well;
3. `reach`: `2000 + 1000 · range(0, 15)` (2–16 km), capped for Line and Tube at 8 km;
4. `strength` (§2.3);
5. kind-specific draws: `axis` (Line, Tube, Plane, Fold), `extent` (Plane, Seam, Fold: half sizes 256–4096 m on a 64 m step), the partner centre (Twin: 3–12 km away along a table direction, then an L1 liveness check as for the first), the corridor polyline (Fold: 2–6 segments of 1–4 km), the patch set (Damaged).

The regime changes the kind weights: Hollow Expanse doubles Shell and Plane, Pipeworks doubles Line and Tube, Overgrowth doubles Damaged. (Multipliers; first guesses.)

### 3.2 Query

`gravity_at(p)` needs the wells of the 27 L0 cells around the L0 cell of `p`. A well's reach is at most 16 384 m (half an L0 cell), and its centre is in the inner 6×6×6 L1 cells of its cell, so it cannot reach beyond the adjacent cells. The 27 well records are cached per L0 cell. The wells are sorted by `(L0 cell, kind index)` for the summation order of §1.2.

### 3.3 The furnace (knot) and what stands on it

- A well's centre lies in one live L1 cell. That cell's **knot** is forced to the L2 cell of the centre, with the hub kind **gravity furnace** (HUB-10) (`levels.md` §4W.2.2). The furnace is the source of the field; for Line and Tube wells it is a machine along the axis; for Plane, Seam and Fold wells it is a plant along the region's edge.
- GRV-01: a structure's up is the field direction at its anchor: `Frame::from_up(up(anchor))`, with the heading drawn from the grain (`regimes.md` §7.4, `Radial` policy). Forms are placed with their floors normal to it (`levels.md` §5W.4.1).
- GRV-02: strata in a regime with `StackGeometry::Curved` use the octagonal radial distance from the nearest Point or Shell well as `u` (`regimes.md` §8.1), so crusts wrap into concentric shells. For Line and Tube wells the same applies to `oct2(r_perp)`.
- GRV-03: arteries in a well's region run radially or along shells. In the artery walk (`levels.md` §4W.5.2) the weight of a move is multiplied by 3 if the move is within 30° of the local up (radial) or within 30° of perpendicular to it (along a shell), tested with integer dot products against `up`.
- A lull cell (§5) carries no crust, no fill and no tissue (fill 0), only lull arteries.

## 4. Gates

**Built in PW.6.3 (2026-10-21), `worldgen::gates`, `GEN_VERSION` 19.** What differs from the text below: gates are drawn only where the field has a jump, which today is a Seam's membrane (a Seam's normal is a coordinate axis); an L2 cell holding the membrane gets a gate with chance 1/2 (the draws are made in a fixed order, so the stream does not depend on whether a seam is there), at a lateral spot on a 16 m step at least 64 m from the cell edge and 16 m inside the seam's extent; the kinds are drawn Door 55, Stair 20, Ladder 15, Bridge 10; zones are 2 m thick (Door, Stair, Ladder) with a half opening of 3, 2 and 2 m, and the Bridge is a 16 m ring with half opening 8 m. `from_up` is `−n` on the low side and `+n` on the high side, so every gate at a seam turns by 180° and its strength dips to zero at the middle (`strength = min(256, max|v| / 8)`, so a lull below 4): that is the weightless point of a 2 m passage, and a 2 s drift for a bridge. Gates for Fold bends and Damaged patches wait for PW.6.9 and PW.6.10, because their field jumps are not on a plane. `GateZone` is in absolute metres and not in `CellOutput` (`ZoneKind` is unchanged); `gates_in_l2(seed, l2)` is the pure query and `gravity_at` applies it. The ring of light is four emissive bars with no collision, appended to the L2 primitives. The opening itself (the membrane wall with its hole) is built in PW.6.9. `near_well` in the artery draw is now `field_at_metres(face centre) != default`.

*GRV-04..08, built in PW.6.3.* A **gravity gate** is a marked threshold where "down" turns by a large angle over a short distance. It is a generated feature of L2 (a `THR-09` threshold) that overrides the field locally.

```rust
pub struct GateZone {
    pub aabb: QAabb,       // the gate's box, cell-local Q
    pub from_up: IVec3,    // max-norm-8 directions
    pub to_up: IVec3,
    pub axis: Axis,        // the box axis along which the up turns
    pub kind: GateKind,
}
pub enum GateKind { Door, StairTurnover, Ladder, Bridge }
```

- Inside the zone, `up` interpolates along `axis` from `from_up` at the box's low end to `to_up` at the high end, with the integer smoothstep: `v = (256 − k)·from_up + k·to_up`, `k = 256·S(t)`, quantised (§1.1). If `from_up` and `to_up` are opposite the vector passes through zero at the middle: the gate has a zero-gravity point, and the zone is made short (≤ 2 m) so the walker passes in under a second.
- The zone stores `strength = 256` throughout. Outside the zone the surrounding field applies: a gate sits where the field is steady (it connects two fields of different `up`, such as a Seam or the junction of two bridges).
- **Varieties**:
  - **Door** (GRV-08): a doorway 2 m thick whose two sides have different `up`. The zone is the doorway.
  - **Stair that turns over at its top** (GRV-05): the last 2 m of a stair, where `up` turns by 90° or 180°, so the landing is on a wall or the ceiling of the room below.
  - **Ladder** (GRV-06): the zone covers the top rung to the landing. You climb up, and at the top you are walking. (A ladder reorients with the field; see §6.)
  - **Bridge** (GRV-07): a bridge whose far half belongs to the other well. The zone is a 16 m ring of light mid-span; each end is in its own field.
- A gate is marked with a **ring of light** or a bulkhead frame, the same on all kinds, visible from 100 m: the player is told that "down" changes.
- A `GateZone` is part of the cell output (`ZoneKind::GravityGate`, append-only), so `g(p)` is `gravity_at(p)` overridden by the gate zones of the resident cell containing `p`.
- **Connectivity:** a gate is a threshold on a route, so the route is unchanged. The walk validator evaluates plate walkability against the local `up` at each end separately (§6), which are the two fields.

## 5. Lulls, drift lanes and drift

*GRV-10, GRV-14, built in PW.6.8.*

A **lull** is a region where `strength < 4/256` (§1.2). Rules:

- **Never walkable floor.** The generator places no tissue and no crust surface in a lull. The L1 fill field is 0 in lull cells (L2 cell centres where `gravity_at` is a lull), so no building floats there by accident. The exceptions are the **lull tubes** and drift lanes below, and structures that are in the lull on purpose (the Archipelago, WGT-05).
- **Lull tubes** (arteries, `levels.md` §4W.5.3, type *Gravity gate* on a tube): an enclosed tube of 3–6 m diameter, with **handrails** along it (rails every 90°, a 4 cm round bar) and light rings. Crossing a lull in one is the route guaranteed by the skeleton.
- **Drift lanes** (WGT-03): open channels in a lull with a current. A drift lane carries a velocity field of 0.5–2 m/s along its axis within a tube of 20–60 m diameter (a *current*); the current is a function of position, `current_at(p)`, in the same module. The current applies to the drifting player and to dust and debris.
- **Gravity rivers** (WGT-04): a long drift lane between two wells, marked by lights and dust, 1–10 km long.

### 5.1 Free drift (GRV-14)

In a lull (strength < 4/256) the player **floats**:
- the capsule is not pulled; the velocity decays with a time constant of 4 s *(provisional)*, so a stopped player almost stops;
- holding forward pushes along the **view direction** with an acceleration of 0.5 m/s², up to **1 m/s** *(provisional, the user's "about 1 m/s")*, backward pushes back, and there is no strafe;
- contact with any surface stops the player gently: the velocity into the surface goes to zero over 0.3 s, with no damage, no bounce;
- near a **handrail** (within 0.6 m of a rail axis) the player is pulled to the rail and carried along it at up to **2 m/s** in the direction of the view's component along the rail (the ladder mechanic laid flat); releasing forward holds position;
- there is no jump, no fall death, no run. The camera keeps the last `up` and does not roll.
- Between strength 4/256 and 64/256 the player is in **low gravity**: gravity is scaled, walking works on floors, falling is slow (§7), and the player can still use the drift push in the air (0.25 m/s²).

Void death (`traversal.md` §3) in a lull: if no surface (within 100 m) is found for 90 s of drifting, the player respawns with the usual fade *(provisional)*.

## 6. Orienting the player

The controller, camera, respawn, lifts and the walk validator take "up" from the field (GRV-13).

- **Controller.** The capsule's axis is the local `up` (unit vector from §1.1). Gravity is `−strength/256 · 9.81 · up`. The walkable-slope test uses the angle between the surface normal and `up` (≤ 40°). Step-up and step-down are along `up`. Yaw is a rotation about `up`; pitch is relative to the horizon plane perpendicular to `up`.
- **Re-orientation.** When `up` changes (a gradient or a gate), the player's orientation turns so that the new `up` is the camera's up, by the shortest rotation. The turning rate is capped at **90°/s** *(provisional)*, with an ease-in and ease-out of 0.15 s. A 180° gate flip therefore takes about 2 s; the camera rolls smoothly (the user's choice), and the capsule keeps contact with its floor. Controls stay relative to the view throughout.
- **Lifts and trams** use the `up` of their shaft frame, fixed at generation (the field at the shaft's anchor); a lift in a well region follows the local radial direction. Trams are on tubes whose frame is fixed at generation and are unaffected.
- **Respawn** stores `(WorldPos, up, facing)` at the safe position.
- **Walk validator.** A plate is walkable if the angle between its normal and the `up` at its centre is ≤ 40° and the strength there is ≥ 64/256 (low gravity allows walking, weightlessness does not). Links join plates in the same way; across a gate, a link is allowed between the plates at the two ends.

## 7. Falling (GRV-11)

Fall damage is measured along the local down and scaled by strength. The rule keeps the standard value of 15 m in standard gravity (`traversal.md` §3).

- Accumulate **fall energy** `e` in metres of standard fall: at each step, `Δe = (strength/256) · max(0, Δp · (−up))` (`Δp` the displacement, `up` the unit vector), starting from zero at the last ground contact.
- Death on landing if `e > 15 m`. For constant gravity this is exactly `15 m / (strength/256)` of falling, so in a half-strength field the fatal drop is 30 m; the speed at impact is the same 17.2 m/s.
- Under strength 64/256 (low gravity) the fatal drop is 60 m or more; below 4/256 there is no fall at all.
- The **void timeout** (3.0 s without ground within 200 m) becomes `3.0 s · sqrt(256/strength)`, capped at 12 s, because a fall takes longer in a weak field.
- **Featherfall shafts** (WGT-01..02) are shafts of weak strength where the fall speed is 1–3 m/s. The strength there is 4..=16/256 *(provisional)*; they are marked by rings of light, so a fall into them is safe by construction, and a featherfall is a fast, safe route down a stratum. They are generated as small Plane or Line wells inside a shaft void (`levels.md` §4W.3.2).

## 8. Low-gravity and drift features (summary)

These use the field, gates, and lulls above. Their geometry is specified where they are built.

| ID | Feature | Built from |
|---|---|---|
| WGT-01, 02 | Featherfall shafts: straight, helical, branching, with platforms to drift between, upward | a Line well of strength 4–16 along the shaft; variants by shaft shape |
| WGT-03, 04 | Drift lanes, gravity rivers | `current_at` and lights (§5) |
| WGT-05 | The Archipelago | a lull region of a Between with floating fragments, each its own small Point well with strength 32–128 and `reach` 200–600 m, turning slowly (the fragment's frame, a function of the world clock, is applied by the engine to its pieces; the field around it is static) |
| WGT-06 | Tumbler room | a Plane well whose direction switches every 3–8 minutes of world time; the table of directions is keyed, and the time is the only non-position input. It is an engine-side effect, outside the generator |
| WGT-07 | Orbiters | debris in circular orbit around a Point well on a fixed period (world clock); static geometry, moving instances |
| WGT-08 | Low-gravity districts | wells with strength below 160 (§2.3), tidal edges |
| WGT-09 | Weightless cathedral | a lull with a hall of floating lamps and a handrail |

Only `gravity_at(p)` is a function of position alone. `Tumbler`, `Orbiters` and the Archipelago rotation depend on the world clock and live in the engine, not in `gen`.

## 9. Tests

- `gravity_at` is a pure function: same inputs, same output across threads and runs; no float in the vector sum.
- Lulls appear between twin wells, at the centre of a Shell well, and nowhere on the default field.
- The coverage taper is continuous: `up` changes by at most one quantisation step per metre across the taper edge (property test).
- Walking round a Point well returns to the start (engine test, PW.6.2): six faces of a test block.
- Fall energy equals `h` for a vertical fall in default gravity, and `h · strength/256` in a constant weaker field.

## 10. Salts

| Salt | Value | Key | Use |
|---|---|---|---|
| `WELL` | `0x0122` | cell (L0) | well presence and draws (§3.1) |
| `WELL_PATCH` | `0x0123` | patch cell (L0, 256 m patches via absorb) | Damaged well patches |
| `GATE` | `0x0317` | cell (L2) | gate placement and kind |
