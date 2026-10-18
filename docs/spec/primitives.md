# Spec: primitives and cell output

What a generator level emits for one cell. The same data drives meshing, collision, walkability validation and the SDF horizon, so there is one geometric truth (PLAN.md §11.3). Implementation: `worldgen::prim` (P0.5).

## 1. Units: integer quanta

Primitive geometry is stored in **integer quanta of 1/16 m** (`Q = 0.0625 m`) as `i32`, in the owning cell's **local space**. Local space puts the origin at the cell's minimum corner (`coordinates.md` §5.1), with the axes and metres of `coordinates.md` §1.

```rust
pub type Q = i32;                 // 1 quantum = 1/16 m
pub struct QVec3 { pub x: Q, pub y: Q, pub z: Q }
pub struct QAabb { pub min: QVec3, pub max: QVec3 } // half-open on every axis, min < max
pub struct QRect { pub u0: Q, pub v0: Q, pub u1: Q, pub v1: Q } // face-plane rect (ports.md), half-open
```

Why integers rather than `f32` metres:

- **Exact contact.** Touching boxes share bit-identical faces. That makes internal-face removal (P2.1.3) and coplanar walkable-surface merging exact, with no hairline gaps.
- **Determinism.** Every generator decision is integer arithmetic on hash and RNG outputs (`hashing.md` §6). Conversion to `f32` happens once, at meshing and collision time. It is exact, because any multiple of 1/16 below 2²⁰ m is representable.
- **Range.** An L0 cell (32768 m) is 524 288 Q, and an L1 cell (4096 m) is 65 536 Q. Both fit easily in `i32`, including overflow into neighbours.
- **Granularity.** 6.25 cm is finer than anything the player perceives at structural scale. Standard stair rises (3 Q = 18.75 cm) and treads (5 Q = 31.25 cm) are whole quanta. L5 dressing, which is visual only, may use `f32` offsets inside an L4 cell.

A child cell's offset in its parent's local space is `(child.coords − 8 × parent.coords) × child_size`, which is an exact integer in Q. Converting between parent-local and child-local space is a single integer subtraction.

Geometry normally lies inside the cell, `[0, size)`. It may extend outside only as the feature ownership rule in `levels.md` (P0.2.4) allows.

## 2. Orientation

Primitives are **axis-aligned by default**: ramps and stairs rise along one of four horizontal directions, and cylinders lie along one of the three axes. The shapes in §3 stay exactly as they are.

```rust
pub enum Dir4 { PosX, NegX, PosZ, NegZ }   // direction of ascent
pub enum Axis { X, Y, Z }                // shared with cell faces (worldgen::cell)
```

**Revised 2026-10-18 (PW.0.2).** The user reversed the axis-aligned-only decision of 2026-10-17: the Web design allows any angle (`docs/proposals/the-web.md` §7). Oriented shapes are specified in §9. They wrap the shapes of §3 in a frame, so an axis-aligned primitive is simply the case with the identity frame, and its output is unchanged.

## 3. Primitive types

```rust
pub struct Primitive {
    pub shape: Shape,
    pub material: MaterialId, // u16
    pub kind: FeatureKind,    // u8
    pub flags: PrimFlags,     // u8 bitflags
}

pub enum Shape {
    Box      { aabb: QAabb },
    Ramp     { aabb: QAabb, rise: Dir4 },
    Cylinder { axis: Axis, base: QVec3, length: Q, radius: Q },
    StairRun { aabb: QAabb, rise: Dir4, steps: u16 },
    LiftShaft{ aabb: QAabb },
}
```

| Shape | Solid region | AABB | Notes |
|---|---|---|---|
| **Box** | the whole `aabb` | `aabb` | Slabs, walls, masses, trunk cores |
| **Ramp** | wedge in `aabb`. With `t ∈ [0, 1]` the normalized position along `rise` (0 at the low edge), solid where `y − min.y ≤ (max.y − min.y) · t` | `aabb` | Zero thickness at the low edge. Slope angle = atan(height / run) |
| **Cylinder** | `axis`-aligned solid cylinder. `base` is the centre of the cap at the axis minimum, it spans `length` along the axis, and its circular cross-section has `radius` | `base ± radius` across the axis, `[base, base + length]` along it | Vertical: trunks, columns. Horizontal: pipes, ducts, tram tubes |
| **StairRun** | a staircase of `steps` steps filling `aabb`, ascending along `rise`. Step *i* (0-based) is solid up to height `(i + 1) · h / steps` over its tread `[i · r / steps, (i + 1) · r / steps)`, where `h` is the height and `r` the run | `aabb` | `h` and `r` must both be divisible by `steps`, so every step is in whole quanta. This is checked in debug builds |
| **LiftShaft** | **not solid.** It reserves a vertical void for a lift platform | `aabb` | Placeholder until P3.4.2. Shaft walls are separate Box primitives. Stops come from lift-stop ports (`ports.md`) |

Each shape provides `aabb()`, `contains(p)` (point-in-solid) and `sdf(p)` (signed distance, negative inside), all over `f32` points in cell-local metres (P0.5.3).
- `sdf` is exact for Box and Cylinder.
- For Ramp and StairRun it is exact or a conservative bound. A bound never overestimates distance, so the raymarcher stays safe. As implemented (`worldgen::sdf`, P0.5.3):
  - Ramp is `max(box, incline plane)`: exact inside, a lower bound outside.
  - StairRun is the minimum over its step boxes: exact outside, and it underestimates depth inside.
  - Both are evaluated in a mirrored "rise frame" (an isometry), so all four directions share one formula.
- LiftShaft's `sdf` is `+∞`, because it has no surface.
- LiftShaft's `contains` is always false.

**No subtractive primitives.** The solid world is the **union** of all primitives. Holes, shafts and chasms are expressed by not placing solid there, and finer levels are kept out of those regions by `KeepOut` constraints (§6). This keeps collision a simple "any primitive contains p" query.

## 4. Material, kind and flags

**`MaterialId(u16)`** indexes the material table (P4.3.1). Ids are append-only, because renumbering changes output. The initial ids:

| Id | Name | Use |
|---|---|---|
| 0 | `CONCRETE` | default structure |
| 1 | `METAL` | panels, shaft walls |
| 2 | `GRATING` | catwalks, bridge decks (footsteps, see-through later) |
| 3 | `EMISSIVE` | light panels, windows |
| 4 | `PIPE` | pipes and ducts |
| 5 | `WARM` | sanctuary interiors |

**`FeatureKind(u8)`**: what a primitive belongs to. Rendering bands, dressing, audio and debug colouring in slice images all use it.
- `Trunk`, `Deck`, `ShaftWall`, `Bridge`, `Mass`, `StairTower`, `Ramp`, `Walkway`, `Railing`, `Pipe`, `Sanctuary`, `Misc`.
- Append-only.

**`PrimFlags`** bitflags:

| Flag | Meaning |
|---|---|
| `ENVELOPE` | A placeholder that the child level refines (§5). Drawn in this level's LOD band and replaced when the children are resident |
| `NO_COLLIDE` | Visual only (cables, thin dressing). Collision and walkability ignore it |
| `FAR_VISIBLE` | Included in the far-band reduction (`l1_far_primitives`, P0.7.6) and in the SDF horizon |

## 5. Structural vs envelope primitives

The horizon promise depends on this distinction.

- A **structural** primitive (no `ENVELOPE` flag) is final. It exists at every distance, is rendered by its owning level's chunks, and is collided with directly. Finer levels may **add** to it (detail on a trunk's surface, a doorway frame) but never remove or move it.
- An **envelope** primitive stands in for content a finer level will generate. An L2 building mass, for example, is drawn as a box in the mid band and refined into rooms by L3 nearby. The child level must:
  1. keep all its solid geometry **inside** the envelope, apart from features attached through ports, and
  2. **fill the envelope's outer surface** closely enough that the silhouette and massing seen from afar match. Exterior walls follow the envelope faces, with openings at ports and windows.

  The exact tolerance for (2) is defined per level in `levels.md`. The consistency check (P2.7.1, P6.1.3) enforces it.
- Collision and walkability always use the finest resident level. When a child level is resident, its parent's envelope primitives are ignored. Structural primitives always count.

## 6. Walkable surface

A **walkable surface** is a region of a primitive's upward-facing boundary that meets all of these:

1. **Slope:** its normal is within `max_slope` (40°, `traversal.md`) of +Y.
   - Box tops and vertical-cylinder caps: 0°.
   - Ramp: atan(height / run).
   - StairRun: treated as its equivalent ramp, provided each step's rise is ≤ `step_up` (0.45 m) and each tread is ≥ 0.25 m.
   - Horizontal cylinders and cylinder sides are never walkable.
2. **Uncovered:** no other solid, collidable primitive occupies the space directly above it. Coplanar touching tops merge into one surface.
3. **Clearance:** the column from the surface up to `capsule_height` (1.8 m) is free of collidable solid. Parts with less headroom are removed.
4. **Width:** what remains must contain a disk of radius `capsule_radius` (0.3 m). Narrower slivers are dropped.

**As implemented** (`worldgen::walk`, P0.9.1–P0.9.2). Surfaces are plan rectangles (`Plate`), flat or sloping along one axis: box tops, ramps within 40°, and stair runs with rise ≤ 7 Q and tread ≥ 4 Q (as their equivalent ramp). A plate loses the plan footprint of every other collidable solid that meets its headroom column (29 Q above the surface); vertical cylinders are cut as 48 strips; pieces narrower than 10 Q (2 × radius) are dropped. Two plates are **linked** when their plan rectangles overlap, or touch along an edge of at least 10 Q, and their heights come within 7 Q (the step-up) somewhere on that contact. Each stop of a `LiftShaft` gives a flat platform over the shaft footprint (a stop is the top of a box inside the footprint, or the height of a flat plate meeting the footprint where every wall is open for 29 Q); a shaft's platforms are linked to each other. `Nav` holds the plates, the links and the connected components.

**Validator** (`worldgen::validate`, P0.9.3–P0.9.4): `check_cell` builds a cell's `Nav` from the parent's window (`walk::cell_window`: L1 primitives met by the cell, boxes cut to it in plan) plus everything L2 emitted, and requires of each non-lift port that floor exists at its opening (`walk::plates_at_port`) and connects to a plate of L1 origin (the network N) inside the cell, and that no primitive leaves the cell. `BlockNav` joins the graphs of neighbouring cells through the ports on their shared faces (lift platforms over a shared lift footprint join too) and answers `reaches_backbone`. `check_random` runs `check_cell` over runs of 512 sibling cells on several scoped threads and returns counts, failures in cell order and timing; 10⁴ cells take about 0.1 s in release.

The walkability extractor (P0.9.1) computes this per cell from primitives across the levels that are resident at that point. It does not trust any flag. Nav graph links between surfaces (steps, slopes, stairs, ports) are P0.9.2.

## 7. Cell output

```rust
pub struct CellOutput {
    pub key: CellKey,
    pub primitives: Vec<Primitive>,           // owning cell's local space, deterministic order
    pub ports: Vec<Port>,                     // ports.md
    pub child_constraints: Vec<ChildConstraint>,
    pub meta: CellMeta,
}

pub enum ChildConstraint {
    /// Children may not place collidable solid here (shafts, chasms, vistas, port corridors).
    KeepOut { aabb: QAabb },
    /// Children refine primitive `prim` (an ENVELOPE) per §5.
    Envelope { prim: u32 },
    /// A port the children must realize on their boundary (cascade, ports.md).
    ForcedPort { port: u32 },
    /// A region with a purpose children must honour (sanctuary interior, vista, …).
    Zone { aabb: QAabb, kind: ZoneKind },
}

pub enum ZoneKind { Sanctuary, Vista, GiantCavity } // append-only

pub struct CellMeta {
    pub flags: MetaFlags, // SANCTUARY, VISTA
    // Further level-specific fields from levels.md (the regime id and style at L0, feature
    // counts, a content hash for golden tests in P0.11.1) are added by the items that produce them.
}
```

- **Coordinates** in `primitives` and in `child_constraints` are in the emitting cell's local space. Children convert them with the integer offset from §1.
- **Determinism.** Vectors are filled in a fixed order defined by the code, never by `HashMap` iteration or thread scheduling. The golden-test content hash covers the whole `CellOutput` in that order.
- **Optional `serde`**, behind a feature flag (P0.5.2), so the slice tool and tests can dump outputs.

## 8. Open points (resolved in later specs)

- How far geometry may extend past the cell (P0.2.4 ownership rule): `levels.md`.
- Envelope-fill tolerance per level: `levels.md`.
- `Port` fields: `ports.md`.
- Stair rise and tread defaults, and the lift platform: `traversal.md`.

## 9. Oriented primitives (PW.0.2)

*Owners: TEC-01..04 (specified here, built in PW.1.1); TEC-05, TEC-07 (PW.1.2); TEC-06 (PW.1.3). Architecture: `the-web.md` §7.3.*

### 9.1 Integer orientation (TEC-01)

A rotation is never computed from an angle. It is a **key into constant tables of integer vectors**. Matrices are then built with `+ − × ÷` and `sqrt` only, which IEEE 754 rounds correctly, so equal keys give equal bits on every machine (`hashing.md` §6).

```rust
pub enum Frame {
    /// Axis-aligned. The identity.
    Id,
    /// A yaw heading (§9.1.1) then a tilt (§9.1.2).
    Table { yaw: u8, tilt: u8 },          // yaw 0..64, tilt 0..7
    /// The local X axis along an integer direction (§9.1.3), with a derived roll.
    Dir { d: [i8; 3] },
}
```

`Frame::Table { yaw: 0, tilt: 0 }` and `Frame::Id` are the same frame (a constructor normalises it to `Id`, and the shape code treats `Id` as "emit the old axis-aligned primitive").

#### 9.1.1 Headings

There are 64 headings. Heading `k` is the integer vector `(a, b)` and the unit vectors are `X = (a, 0, b) / L`, `Z = (−b, 0, a) / L` with `L = sqrt(a² + b²)`, computed in `f64`.

- First quadrant (`k = 0..16`), derived once with `round(64 · tan(5.625° · r))` and then **pinned as a constant table**. The derivation is documentation only; the table is the definition:

| k | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| (a, b) | (64,0) | (64,6) | (64,13) | (64,19) | (64,27) | (64,34) | (64,43) | (64,53) | (64,64) | (53,64) | (43,64) | (34,64) | (27,64) | (19,64) | (13,64) | (6,64) |

- Other quadrants: heading `k = 16q + r` is the first-quadrant vector `r` turned `q` quarter turns by the exact integer map `(a, b) → (−b, a)`.
- Heading 0 is +X, 16 is +Z, 32 is −X, 48 is −Z. The headings are 5.625° apart to within about 0.3°. They need not be exactly even.
- `yaw + 32` is the exact opposite direction; `yaw + 16` is the exact right angle. Both follow from the quadrant rule, so perpendicular and antiparallel facets are exact.
- At heading 0, `X = (1, 0, 0)` and `Z = (0, 0, 1)` come out **exactly**, because `64/64 = 1` and `0/64 = 0`.

#### 9.1.2 Tilts

`tilt ∈ 0..7` selects a slope `s = [0, 6, 11, 17, 23, 30, 37][tilt]` (`64 · tan` of 0°, 5°, …, 30°, rounded). The frame axes are:

```
m   = sqrt(64² + s²)
ex  = (64 / m) · X + (s / m) · Y        // local X, tilted up by atan(s/64)
ey  = (−s / m) · X + (64 / m) · Y       // local up
ez  = Z
```

`tilt = 0` gives `s = 0`, `m = 64`, and `ex = X`, `ey = Y` exactly. A tilt is about the frame's horizontal axis `Z`, so the heading also says in which direction the slab leans.

#### 9.1.3 Direction frames

For beams and other members at an arbitrary angle, `Frame::Dir { d }` with `d` a component of the **direction table**:

- The table lists every vector `(x, y, z)` with components in `−2..=2`, not all zero, `gcd(|x|, |y|, |z|) = 1`, and whose first nonzero component is positive. There are **49** entries, enumerated in lexicographic order of `(x, y, z)` from `(0,0,1)`. Since a beam is an undirected line, the sign does not matter.
- Classes (the classes of `weave`, `regimes.md` §7.2): **axis** 3, **face diagonal** 6 (two components of magnitude 1, one zero), **space diagonal** 4 (all three of magnitude 1), **skew** 36 (the rest).
- Axes: `ex = d / |d|`, `|d| = sqrt(x² + y² + z²)`. For the second axis take `w = Y` if `x ≠ 0 or z ≠ 0`, else `w = X`; then `ey = normalize(w − (w · ex) ex)` and `ez = ex × ey`. All of this is `+ − × ÷ sqrt`. If `d` is an axis, the result is the identity (up to a permutation of axes, which the constructor performs exactly, so a beam along Y is an axis-aligned cylinder).

Frames from an up vector (`Frame::from_up`, used by gravity wells and the radial policy, PW.0.6) follow the same Gram–Schmidt recipe with `ey = up / |up|`. They are specified in `gravity.md`.

#### 9.1.4 Using a frame

A frame plus an integer **origin** `o: QVec3` (cell-local Q) maps frame-local integer Q coordinates `l` to cell-local coordinates `p = o + R · l`, where `R = [ex ey ez]`. The mapping is computed in `f64` and **rounded to the nearest Q** with `floor(x + 0.5)` whenever a position must become an integer (for example the centre of a helix step). Everything that stays in frame-local integers (extents, step counts) is exact.

### 9.2 Oriented shapes (TEC-03)

```rust
pub struct Oriented {
    pub frame: Frame,
    pub origin: QVec3,          // frame origin, cell-local Q
    pub shape: Shape,           // any §3 shape, in frame-local Q (axis-aligned there)
    pub clip: QAabb,            // §9.4, cell-local Q
}
```

`Shape` is exactly the §3 enum (`Box`, `Ramp`, `StairRun`, `Cylinder`). So the new shapes are:

| New shape | Is |
|---|---|
| Oriented box | `Box` in a frame |
| Oriented ramp | `Ramp` in a frame (rises along ±frame X or ±frame Z) |
| Oriented stair run | `StairRun` in a frame |
| Cylinder on any axis | `Cylinder` along frame X, Y or Z, so with `Frame::Dir` it lies along any of the 49 directions |
| Faceted helix, polygons | built from the above (§9.3) |

`Primitive.shape` becomes `enum Shape { …§3…, Oriented(Box<Oriented>) }` (append-only). A primitive whose frame is `Id` is never stored as `Oriented`. It keeps the old variant, so old output is bit-identical and the golden tests stay valid (§9.6).

### 9.3 Faceted helix and curves (TEC-04)

Curved things are made from straight pieces, so every piece keeps an analytic shape.

- **Helical stair**: `n` steps per turn from the set `{8, 16, 32}` (so each step is a whole number of headings: `64 / n` = 8, 4, 2), `turns` a half-integer count, an inner radius `r0`, a width `w`, a rise per step `h` (whole Q, ≤ 7 Q so the stair is walkable), a handedness, and a base heading `k0`.
- Step `j` is a `Box` (tread) with frame `Table { yaw: k0 + j · (64/n) ± 16, tilt: 0 }`: a heading turned a quarter turn from the radial direction, so its long axis is tangent. Its origin is the centre of the tread on the circle of radius `r0 + w/2`, computed as `round(centre + radius · (a, b) / L)` per §9.1.4, and its vertical position is `base_y + j · h`.
- The tread length along the tangent is `ceil(2 · radius · tan(π/n))` rounded up to a whole Q using the integer ratio table of §9.1.1 (the facet chord), plus an overlap of 1 Q so adjacent treads share a face and the stair has no cracks.
- **Helical ramp** uses a `Ramp` per facet with the rise along the tangent. Its `rise` per facet equals `h`, so the slope is `h / chord`; the slope limit (40°, `traversal.md`) applies per facet.
- **Polygonal ring, dome, rotunda, curved gallery**: a ring of `n ∈ {8, 16, 32, 64}` box facets at the headings `k0 + j · (64/n)`, each facing inwards; a dome adds rings at tilts from the tilt table, with the radius per ring from the integer ratios of the table. The walls of a curved gallery (PH-20) and a rotunda (FORM-16) use the same recipe.
- **Catenary** (PH-04) is a chain of oriented boxes whose per-segment tilt index follows a pinned table of integer tilts (a discretised parabola), so no `cosh` is ever evaluated.
- **Cones and funnels** (FORM-23, VOID-13): a stack of coaxial rings (cylinders on the Y axis, or facet rings) with radius decreasing by a whole number of Q per ring.

### 9.4 Clipping instead of shape rules (TEC-02)

Every oriented primitive carries a `clip: QAabb`, the box of its owning cell (or a tighter one), in the emitting cell's local space.

- `contains(p) = shape.contains(p) ∧ clip.contains(p)`.
- `sdf(p) = max(shape.sdf(p), clip.sdf(p))`. It is a valid lower bound: it never overestimates distance, so the raymarcher stays safe, and it is exact inside both.
- **Meshing** clips each output polygon to the six clip planes (Sutherland–Hodgman against a box). No cap faces are generated on the clip planes: the neighbouring cell emits its own clipped piece of the same solid, and the faces on the shared plane are interior.
- **Why this is enough for keyed features.** A beam, a staircase or any feature whose description is a pure function of a key is evaluated by every cell it crosses. Each cell emits the whole solid **with its own clip**, so the pieces tile the solid exactly at the cell planes, and nothing needs to know about neighbours. This keeps the ownership rule (`levels.md` §1.1) and removes the old restriction that only boxes can cross faces.
- To limit work, a cell first discards a feature whose integer AABB (§9.5) misses the clip box.
- The clip is part of the primitive's description and hence of the golden hash.

### 9.5 Bounds

`aabb()` of an oriented primitive is the **integer AABB** of its eight frame-box corners mapped to cell-local Q (rounded outwards: `floor` for the minimum, `ceil` for the maximum), intersected with `clip`. It is conservative (never smaller than the solid), and it is what constraints, `KeepOut` tests and overlap queries use. For `Id` it is exactly the old AABB.

### 9.6 Axis-aligned output is unchanged

- `Frame::Id` maps to the old variants. Nothing about those is touched: no matrix, no rounding.
- `Table { yaw: 0, tilt: 0 }` produces `ex = X = (1,0,0)`, `ey = Y`, `ez = Z` exactly (§9.1.1 and §9.1.2), and the constructor turns it into `Id` anyway.
- `Table { yaw: 16 · q, tilt: 0 }` (quarter turns) has `(a, b)` = `(64, 0)` turned `q` times by `(a, b) → (−b, a)`: `(0, 64)`, `(−64, 0)`, `(0, −64)`, so the unit vectors are `±1` and `0` exactly. Mapping integer Q through it is exact (no rounding), so quarter-turned boxes equal the old `Dir4`-style boxes.
- A test (PW.1.1) asserts this and that the golden content hashes of every existing cell are identical until the first generator change after it (invariant 8).

### 9.7 Collision, walkability and SDF (summary; built in PW.1.2 and PW.1.3)

- **Collision (TEC-05).** To test a capsule against an oriented primitive, transform the capsule's two end points into the frame (`R^T (p − o)`), run the existing axis-aligned test there (the capsule keeps its radius under a rotation), and rotate the contact normal back. The clip box is an additional axis-aligned test in cell space. A vertical capsule becomes a slanted capsule in the frame, so the test is segment-versus-box, which already exists for the horizontal-cylinder case.
- **Walkability (TEC-06).** A plate is an oriented rectangle with a normal. A surface is walkable if the angle between its normal and the local up (usually +Y, `gravity.md` otherwise) is ≤ 40°, which is checked with the integer slope tables, not with an angle. Links join plates whose overlap and height difference meet the same tests, in the plate's frame. `walk.rs` keeps its axis-aligned fast path for `Id` plates.
- **SDF (TEC-07).** The SDF of an oriented primitive is the axis-aligned SDF evaluated at `R^T (p − o)`. A rotation is an isometry, so exactness and the lower-bound property carry over unchanged. For the clip the formula in §9.4 applies.
