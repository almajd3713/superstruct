# Spec: primitives and cell output

What a generator level emits for one cell. The same data drives meshing, collision, walkability validation and the SDF horizon, so there is one geometric truth (PLAN.md §11.3). Implementation: `worldgen::prim` (P0.5).

## 1. Units: integer quanta

Primitive geometry is stored in **integer quanta of 1/16 m** (`Q = 0.0625 m`) as `i32`, in the owning cell's **local space**. Local space puts the origin at the cell's minimum corner (`coordinates.md` §5.1), with the axes and metres of `coordinates.md` §1.

```rust
pub type Q = i32;                 // 1 quantum = 1/16 m
pub struct QVec3 { pub x: Q, pub y: Q, pub z: Q }
pub struct QAabb { pub min: QVec3, pub max: QVec3 } // half-open on every axis, min < max
```

Why integers rather than `f32` metres:

- **Exact contact.** Touching boxes share bit-identical faces. That makes internal-face removal (P2.1.3) and coplanar walkable-surface merging exact, with no hairline gaps.
- **Determinism.** Every generator decision is integer arithmetic on hash and RNG outputs (`hashing.md` §6). Conversion to `f32` happens once, at meshing and collision time. It is exact, because any multiple of 1/16 below 2²⁰ m is representable.
- **Range.** An L0 cell (32768 m) is 524 288 Q, and an L1 cell (4096 m) is 65 536 Q. Both fit easily in `i32`, including overflow into neighbours.
- **Granularity.** 6.25 cm is finer than anything the player perceives at structural scale. Standard stair rises (3 Q = 18.75 cm) and treads (5 Q = 31.25 cm) are whole quanta. L5 dressing, which is visual only, may use `f32` offsets inside an L4 cell.

A child cell's offset in its parent's local space is `(child.coords − 8 × parent.coords) × child_size`, which is an exact integer in Q. Converting between parent-local and child-local space is a single integer subtraction.

Geometry normally lies inside the cell, `[0, size)`. It may extend outside only as the feature ownership rule in `levels.md` (P0.2.4) allows.

## 2. Orientation

All primitives are **axis-aligned**. Ramps and stairs rise along one of four horizontal directions, and cylinders lie along one of the three axes.

```rust
pub enum Dir4 { PosX, NegX, PosZ, NegZ }   // direction of ascent
pub enum Axis3 { X, Y, Z }
```

The megastructure's language is orthogonal: slabs, shafts, pillars, ducts. Axis-alignment keeps collision analytic and exact, and keeps the SDF horizon cheap. Arbitrary rotation (diagonal bridges, for example) would be a later addition and a `GEN_VERSION` bump. **The user confirmed axis-aligned only on 2026-10-17**, over yaw rotation and 45° steps.

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
    Cylinder { axis: Axis3, base: QVec3, length: Q, radius: Q },
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
- For Ramp and StairRun it is exact or a conservative bound; the implementation documents which. A bound never overestimates distance, so the raymarcher stays safe.
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

pub struct CellMeta {
    // Level-specific summary defined in levels.md: the regime id and style at L0, feature
    // counts, sanctuary/rare-event flags, and a content hash for golden tests (P0.11.1).
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
