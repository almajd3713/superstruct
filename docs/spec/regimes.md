# Spec: L0 regimes and rare events

An L0 cell (32 km) is a **regime**: a region with one architectural character and one atmosphere. Regimes are the main tool against monotony (PLAN.md §8). They supply the contrast that makes the world oppressive in one place and hollow and vast in another. Implementation: `worldgen::regime` (P0.6).

## 1. Fields

```rust
pub struct Regime {
    pub style: StyleId,      // u8, §2
    pub density: u8,         // 0 = hollow … 255 = packed
    pub verticality: u8,     // 0 = slabs/decks … 255 = pillars/shafts
    pub palette: PaletteId,  // u8, §3
    pub visibility_m: u16,   // fog visibility, multiple of 250 m
    pub light: u8,           // baseline light level, 0 = black
    pub event: RareEvent,    // §5
    pub web: WebFields,      // §7: the Web redesign (PLAN.md §10.4), drawn after the fields above
}
```

- `density` and `verticality` drive L1 and L2 content (`levels.md` §2: `d`, `v`). `style` will select material weights and module-kit variants (Phase 4).
- `palette`, `visibility_m` and `light` are **atmosphere**. The engine reads them through `atmosphere_at` (§4).
- Fog extinction is `σ = 3 / visibility_m` per metre. This is Koschmieder's relation: contrast drops to 5% at the visibility distance.

**Drawing.** An RNG seeded with `hash(seed, 0, cell, REGIME)` draws the fields in this fixed order:

1. `style` = `pick_weighted(style weights)`.
2. `density` = `range` within the style's range, inclusive.
3. `verticality`, the same way.
4. `palette` = uniform among the style's palettes.
5. `visibility_m` = 250 × `range` over the style's range in 250 m steps.
6. `light` = `range` within the style's range.

`event` is drawn separately (§5). The Web fields (§7) are drawn from a third RNG (`REGIME_WEB`), so steps 1–6 and the event keep their draw sequences and their values.

## 2. Styles

| Id | Name | Weight | density | verticality | Palettes | Visibility | light | Character |
|---|---|---|---|---|---|---|---|---|
| 0 | Concrete Strata | 30 | 96–200 | 40–140 | GREY, COLD_BLUE | 4–12 km | 40–90 | Stacked brutalist decks, long horizontals |
| 1 | Pipeworks | 25 | 140–255 | 60–180 | SEPIA, RUST | 2–6 km | 30–70 | Dense, industrial, close fog, machinery |
| 2 | Pillar Forest | 20 | 60–160 | 170–255 | GREY, BONE | 6–20 km | 50–100 | Huge trunks receding into haze |
| 3 | Hollow Expanse | 15 | 0–80 | 0–120 | COLD_BLUE, BONE | 15–30 km | 20–60 | Vast voids, sparse decks, far horizons |
| 4 | Overgrowth | 10 | 100–220 | 80–200 | SICK_GREEN, SEPIA | 3–8 km | 25–60 | Organic encrustation over structure, sickly light |

- Every range is inclusive.
- **Contrast:** pairs of styles that differ strongly, such as Pipeworks next to Hollow Expanse, give the "compression then release" rhythm at regime scale.
- Adjacent L0 cells are independent, so such contrasts arise naturally.

## 3. Palettes

The palettes are near-monochrome, with one tint each and one emissive colour, in the spirit of Nihei's ink work. The values are sRGB, and blending happens in linear space (§4).

| Id | Name | Fog / void | Ambient | Emissive | Accent |
|---|---|---|---|---|---|
| 0 | GREY | `#3A3C40` | `#8A8D93` | `#C9D4E0` | `#E8E8E8` |
| 1 | COLD_BLUE | `#1E2A36` | `#6E8496` | `#9FD3FF` | `#CFE8FF` |
| 2 | SEPIA | `#3B3127` | `#9A8670` | `#FFC27A` | `#F2D7B0` |
| 3 | RUST | `#3A2219` | `#8E5B45` | `#FF7A3D` | `#E9B39A` |
| 4 | BONE | `#5A5850` | `#B8B3A3` | `#FFF4D6` | `#FFFFFF` |
| 5 | SICK_GREEN | `#1F2B22` | `#6F8A72` | `#B6FF9E` | `#D8F2C8` |

- Sanctuary warmth is local (the `WARM` material and local lights), not a regime palette. That keeps the contrast with the surrounding regime.
- The palettes are first guesses. P4.3.4 (art-direction review) fixes them, and the ids are append-only.

## 4. Atmosphere blending: `atmosphere_at(WorldPos)`

Only atmosphere blends across L0 faces. Generator values never do: `style`, `density`, `verticality` and events are per cell (`levels.md` §3).

```rust
pub struct Atmosphere {
    pub fog_color: Vec3,     // linear RGB
    pub sigma: f32,          // extinction per metre
    pub ambient: Vec3,       // linear RGB
    pub emissive: Vec3,      // linear RGB
    pub accent: Vec3,        // linear RGB
    pub light: f32,          // light / 255
    pub haze: f32,           // multiplies sigma; the style's aerial perspective
    pub height_k: f32,       // fog height falloff per metre
}
```

**Blend band:** `B = 4096 m` on each side of every L0 face, the width of one L1 cell. Per axis, take the position `a ∈ [0, 32768)` inside the L0 cell. It is computed exactly from `sector.rem_euclid(32) × 1024 + local`. Then:

- `a < B`: blend with the lower neighbour, with `w_neighbour = 0.5 · s(1 − a / B)`.
- `a ≥ 32768 − B`: blend with the upper neighbour, with `w_neighbour = 0.5 · s(1 − (32768 − a) / B)`.
- Otherwise: `w_neighbour = 0`.
- `s(t) = t²(3 − 2t)`, the smoothstep polynomial (allowed operations only, `hashing.md` §6).

The three axes are combined as a tensor product. The resulting weights over the 2×2×2 block of cells sum to 1, and the result is the weighted sum of each cell's atmosphere (palette colours converted to linear, `sigma`, `light`).

At a face, both sides evaluate to a 50/50 mix, so the field is **continuous** everywhere (tested along a line in P0.6.3). It is also smooth across the band edges, because `s′(0) = 0`.

Example (x only): at 1024 m inside cell X from its lower face, `w_lower = 0.5 · s(0.75) = 0.5 · 0.84375 = 0.421875` and `w_self = 0.578125`. At the face itself, `a = 0` gives 0.5 / 0.5.

Blackout cells (§5) blend like any other cell, so darkness falls gradually as you approach one. A Blackout cell's own atmosphere has `light = 0` and `emissive = 0` ("emissive suppressed").

**Aerial perspective per style (P5.1.5, first guesses for the art review).** The engine uses `sigma · haze` and `height_k`; both blend like the other fields.

| Style | haze | height_k (per m) |
|---|---|---|
| Concrete Strata | 1.0 | 1/500 |
| Pipeworks | 3.0 | 1/150 |
| Pillar Forest | 1.2 | 1/800 |
| Hollow Expanse | 0.5 | 1/2000 |
| Overgrowth | 2.0 | 1/250 |

**Linear colours** are pinned as constants in `worldgen::atmosphere`, because the sRGB curve needs `powf`, which `hashing.md` §6 bans. A test re-derives them from this table's hex values.

## 5. Rare events

### 5.1 Per-cell event `[RARE_EVENT]`

The event is drawn with `pick_weighted` from an RNG seeded with `hash(seed, 0, cell, RARE_EVENT)`. The weights are out of 1000:

| Event | Weight | Effect |
|---|---|---|
| `None` | 920 | — |
| `GiantCavity` | 30 | An irregular, roughly ellipsoidal void of whole L1 cells (4–12 km radii) with rounded rim bites (`levels.md` §3, §4.6). Its rim is walkable (P5.2.2). BLAME!'s Jupiter-sized room, at the scale this world can afford |
| `Blackout` | 30 | `light = 0`, emissive suppressed, visibility forced to 1–3 km. Total darkness except the player's surroundings. Mind-breaking through absence |
| `GravityRegion` | 20 | **Flag only** until P5.2.4 decides whether gravity can change direction. It has no effect on generation or play until then |

**Blackout overrides.** After the pick, the same `RARE_EVENT` RNG draws the forced visibility as 250 × `range(4, 12)` inclusive (1–3 km), and `light` becomes 0. These replace the values drawn in §1, so the `REGIME` draw sequence is the same in every cell.

Over 10⁵ cells, each frequency must fall within ±10% of its weight / 1000 (P0.6.2).

### 5.2 Per-column event: infinite staircase `[RARE_STAIR]`

- Keyed by `ColumnKey(L0, x, z)`, so it runs through **every** L0 cell in that column, without end.
- Present with probability 4/1000 per L0 column: the first draw is `below(1000) < 4`, an exact integer test. A random L0 cell therefore lies in a staircase column with probability 0.004.
- Placement uses an RNG seeded with the same hash:
  - one L1 column within the L0 column, chosen uniformly from the 8 × 8 (`l1_x = below(8)`, then `l1_z = below(8)`), and
  - a 64 × 64 m footprint under the L1 separation rule (`levels.md` §4.0): the min corner is drawn on a 16 m step, x then z, with the 64 m margin. If the footprint comes within 64 m of the L1 core, the staircase is absent from that L1 column.
- Placement is drawn **before** the trunk, shaft and chasm, so those avoid it.
- A switchback of `StairRun`s ascends without end, with landings joined to every deck it crosses. It is an L1 keyed feature.
- The generator reserves the footprint as a full-height `KeepOut` in every cell of the column, cuts it out of every deck, and routes bridges around it (`l1_bridge`).
- **As implemented** (`worldgen::l1_stair`, P5.2.3, `GEN_VERSION` 6): a square spiral inside the footprint, identical in every L1 cell of the column. Each side carries one 4 m wide `StairRun` of 128 steps (0.25 m rise, 0.375 m tread: 32 m up over 48 m), and an 8 m landing slab in each corner joins one flight to the next. A turn is 128 m, so a cell holds 32 turns and its top landing is the bottom landing of the cell above. The flights hug the footprint's edge, so a deck around the hole meets the flight that passes its height. Cavity cells hold no staircase.

### 5.3 Not events

- **Slow distant movers** (light trains, Builder silhouettes) are rendering features (P5.2.5), not generator events.
- **Self-similarity** ("the same grammar at every scale", PLAN.md §7) is deferred. It needs its own design pass.

## 6. Salts (original block)

| Salt | Value | Key | Use |
|---|---|---|---|
| `REGIME` | `0x0101` | cell (L0) | regime field draws (§1) |
| `RARE_EVENT` | `0x0102` | cell (L0) | event choice (§5.1) |
| `RARE_EVENT_PLACE` | `0x0103` | cell (L0) | cavity placement |
| `RARE_STAIR` | `0x0104` | ColumnKey (L0) | staircase presence and placement (§5.2) |
| `CAVITY_NOISE` | `0x0105` | cell (L1) | cavity voxelization noise (`levels.md` §3) |

Salts added for the Web are in §12.

## 7. The Web fields (PW.0.1)

*Owners: REG-01..04, ORI-12..16 (PW.2.1); STR-01..02, BTW-01..08 (PW.2.2); SC-01..11 (PW.2.10). Architecture: `docs/proposals/the-web.md` §3, §4.1, §7.2; character: `the-web-ideas.md` §1.*

```rust
pub struct WebFields {
    pub fill_crust: u8,       // §7.1, mean built fraction inside crusts, 255 = 100 %
    pub fill_between: u8,     // §7.1, mean built fraction inside Betweens
    pub weave: u8,            // §7.2, 0 = axes only … 255 = mostly oblique
    pub grain_log2: u8,       // §7.3, grain size = 2^grain_log2 metres, 6..=12
    pub orientation: OrientationPolicy, // §7.4
    pub stack: StackGeometry, // §8.1
    pub crust_units: (u8, u8),   // §8.2, crust thickness range in 64 m units
    pub between_mix: [u8; 5], // §8.3, weights over the depth/content kinds
    pub forms: [u8; 7],       // §9, weights 0..=8 over form families
    pub supports: [u8; 11],   // §9, weights 0..=8 over SUP-01..11
    pub paths: [u8; 6],       // §9, weights 0..=8 over path families
    pub quarters: [u8; 16],   // §9, weights 0..=8 over QTR-01..16
    pub mutations: [u8; 6],   // §9, weights 0..=8 over mutation groups
}
```

All weights are integers (`hashing.md` §3.1). Fraction fields are `u8 / 255`, as `density` already is.

### 7.1 Fill (REG-01, SC-06)

Fill is the share of the volume that is structure. It is the mean the fill field (FILL-01, `levels.md` §4) aims at.

- `fill_crust` is drawn within the style's range (§7.6). Targets across styles: crust 70–90 % (the-web.md §4.1), so `178..=230`.
- `fill_between` is drawn within the style's range, 2–10 % (`5..=26`).
- Fill is a **mean**. Voids set the local value to zero, the stratum character shifts it (§10.2, SC-06), and L2 fills its cell to within ±10 % of the field.
- `density` (§1) keeps its meaning for the old L1 content until PW.2.8 replaces it. After that it is used only as the `d` input of the lerp-based features that survive (shaft and chasm odds, sanctuary odds), and it is derived: `density = lerp(fill_between, fill_crust, 1/2)` rounded down. Until PW.2.8 the drawn value from §1 is used unchanged. *(Open point for PW.2.8: whether to keep `density` at all.)*

### 7.2 Weave (REG-01)

`weave` is the bias of beams, routes and arteries towards axes versus angles. Frame, routes and arteries each take their step or member direction from a table of directions, and weave sets the odds.

- Direction classes: **axis** (6 directions), **face diagonal** (12), **space diagonal** (8), and **skew** (the other table entries, `primitives.md`, PW.0.2).
- For a choice between classes: `w_axis = 255 − weave`, `w_diag = weave / 2`, `w_space = weave / 4`, `w_skew = weave / 8`, as integer weights for `pick_weighted`. At `weave = 0` only axes appear; at 255 the weights are 0, 127, 63, 31, so 56 % face diagonals, 28 % space diagonals and 14 % skew.
- Level 1 oblique beams are only drawn when `weave ≥ 16`. Below that, frame members are axis-aligned.
- Ranges per style are in §7.6.

### 7.3 Grain (REG-02, ORI-14)

`grain_log2 ∈ 6..=12`: the grain size is 64 m (chaotic) up to 4096 m (calm).

- A **grain cell** is a cell of the cubic lattice of side `2^grain_log2` m, in world coordinates.
- A grain cell with side below 512 m (an L2 cell) cannot change the frame inside one L2 cell without breaking the local slot lattice. So the **frame of a structure** is the frame of the grain cell that contains its anchor point, and the structure is clipped to its owning L2 cell as always. Small grains therefore make neighbouring structures in one cell lean at different angles (collisions, ORI-14), while routes keep one lattice per L2 cell (the grain cell containing the cell's centre).
- Grain only matters for the policies `GrainYaw` and `TiltedGrain` (§7.4). With `Axis` it is ignored, and its value still counts for scale effects (SC-05: tissue scale and grain move together).

### 7.4 Orientation policy (REG-04, ORI-12..16)

```rust
pub enum OrientationPolicy {
    Axis,                       // ORI-12: the calm default
    YawPerL1,                   // ORI-13: one yaw per L1 cell
    GrainYaw,                   // ORI-14: one yaw per grain cell (§7.3); with grain 512 m this is one per L2 cell
    TiltedGrain { max_tilt: u8 }, // ORI-15: yaw and pitch, tilt index into the tilt table
    Radial,                     // ORI-16: around a gravity well; falls back to Axis outside any well's reach
}
```

- **Frames are integer keys.** A frame is `(yaw: u8, tilt: u8)`: a heading from the table of 64 headings and a tilt index from the table of tilts. The tables are constants in `primitives.md` (PW.0.2). No trigonometric function runs in `gen` (`hashing.md` §6).
- **Keys.**
  - `YawPerL1`: `yaw = hash(seed, 1, l1_cell, ORI_YAW) % 64`. Rule: `yaw` is quantised to a multiple of 4 (a 16-heading subset) when the style's `weave < 64`, to keep calm regimes tidy; the rule is written here so the policy is fully specified.
  - `GrainYaw`: `yaw` is drawn from `hash(seed, 0, grain_cell, ORI_YAW)`, where `grain_cell` is the integer lattice cell at the grain size (an `I64Vec3`; the level argument is 0 and the size is folded into the salt through `absorb(hash, grain_log2)`). Never a float.
  - `TiltedGrain`: as `GrainYaw`, plus `tilt = 1 + below(max_tilt)` from the same RNG (second draw), using the table's index (0 = upright).
  - `Radial`: the frame is given by the nearest well (`gravity.md`, PW.0.6): the local up is the integer direction away from the centre, from the well's direction table.
- **Cell inheritance.** L1 and L2 cells read the frame from the regime's policy and their own key. An L3 block inherits the frame of its L2 cell. Within a structure everything is in one frame.
- **Seams.** Where frames differ the structures collide. This is intended (ORI-14).
- **Drawing the policy.** The policy is drawn by `pick_weighted` over the style's orientation weights (§7.6). `max_tilt` is `range_i32(1, T+1)` where `T` is the style's tilt cap.
- **Axis-aligned output stays bit-identical.** `Axis` and frames with `yaw = 0, tilt = 0` produce the same primitives as the old generator (`primitives.md` PW.0.2 proves it for the shapes).

### 7.5 Draw order for the Web fields

An RNG seeded with `hash(seed, 0, cell, REGIME_WEB)`, after the regime has its `style` (§1 step 1). Draws, in this order:

1. `fill_crust` = `range` over the style's range, inclusive.
2. `fill_between` = `range` likewise.
3. `weave` = `range` likewise.
4. `grain_log2` = `range` likewise.
5. `orientation` = `pick_weighted(orientation weights)`, then, for `TiltedGrain`, `max_tilt` (draw only when picked; the extra draw is the last of this step).
6. `stack` = `pick_weighted(stack weights)` (§8.1), then `tilt` = `range` over the style's tilt column (§8.4), always drawn and used only for `Tilted`. Tilted and Curved are redrawn as Flat when the geometry cannot apply: `Curved` requires a gravity well within 16 km of the cell, else Flat (no further draw).
7. `crust_units.0` and `.1`: two draws from the style's low and high ranges (§8.2).
8. `between_mix[0..5]`: five draws, each `range` over the style's range (§8.3).
9. `forms[0..7]`, `supports[0..11]`, `paths[0..6]`, `quarters[0..16]`, `mutations[0..6]`: in this order, one draw per entry: `max(0, base + range(−2, 3))` clamped to 8, where `base` is the style's entry in §9. A style entry of 0 is a **ban** and stays 0 without a draw being skipped (the draw is still made, the result is discarded), so the draw count never depends on the table.

Unused draws are still made when a field does not apply, except for the one conditional draw named in step 5, so the sequence is stable. Steps 6–8 are nine draws in all (stack 1, tilt 1, crust bounds 2, Between weights 5). Until PW.2.2 implements them, `regime_web::draw_web` burns those nine draws (`RESERVED_STACK_DRAWS`) so step 9 already has its final values.

**Implementation (PW.2.1).** `worldgen::regime_web`: `WebFields`, `OrientationPolicy`, the per-style table `WEB_STYLES` (§7.6, §9.1) and `draw_web`. After step 9 the fallback rules of §9 are applied without a draw: WALK and STAIR path weights are at least 1, and forms and supports keep at least one nonzero entry. Frames: `WebFields::frame_at` (a structure's anchor in integer metres), `frame_l1`, `frame_l2`. Grain cells use `div_euclid` at `2^grain_log2` m; the grain size is folded into the key with `absorb(hash, grain_log2)`. `Radial` returns the identity until wells exist (PW.6). The golden digest of L0 keeps the version-6 fields only; the Web fields have their own pinned test, because no generator output reads them yet.

### 7.6 Per-style ranges (inclusive)

| Style | fill_crust | fill_between | weave | grain_log2 | Orientation weights (Axis, YawL1, GrainYaw, TiltedGrain, Radial) | tilt cap T |
|---|---|---|---|---|---|---|
| Concrete Strata | 190–225 | 5–15 | 0–40 | 11–12 | 70, 20, 5, 5, 0 | 2 |
| Pipeworks | 205–230 | 10–26 | 20–110 | 8–11 | 25, 35, 25, 15, 0 | 3 |
| Pillar Forest | 178–210 | 5–12 | 30–120 | 10–12 | 30, 30, 15, 25, 0 | 4 |
| Hollow Expanse | 178–200 | 5–10 | 40–180 | 9–12 | 20, 20, 15, 40, 5 | 6 |
| Overgrowth | 190–228 | 8–26 | 60–220 | 6–10 | 10, 20, 40, 30, 0 | 5 |

- `Radial` has weight 5 in Hollow Expanse only because wells are rarer than other regime choices; otherwise it falls back to Axis.
- Tilt index units are set in `primitives.md`; each step is a further 5° and T=6 is 30°.

## 8. Strata and the Between (PW.0.1; geometry and content in PW.0.3)

*Owners: STR-01..02, BTW-01..08.*

A regime's volume is a **stack** of strata: dense **crusts** separated by **Betweens**. The stack is defined along a **stack coordinate** `u` (§8.1), in integer units of 64 m (one L3 cell).

### 8.1 Stack geometry (BTW-06..08)

```rust
pub enum StackGeometry {
    Flat,                 // u = y (default)
    Tilted { tilt: u8 },  // BTW-06: u = n·p for the integer normal of tilt index `tilt` (5–30° off vertical, yaw from the regime)
    Vertical { axis: u8 },// BTW-07: u = x (axis 0) or z (axis 1): the strata are walls
    Curved,               // BTW-08: concentric around a gravity well; u = integer radial distance (octagonal norm)
}
```

- Draw weights per style are in §8.4.
- `u` is computed from integers: `Flat` is `y_units`. `Tilted` uses the normal from the tilt table with an integer dot product, divided by the table's fixed normalisation constant (all in `i64`). `Vertical` is the local `x` or `z` in 64 m units. `Curved` uses the octagonal radial distance `max(|a|, |b|, (|a| + |b|) · 3/4)` style integer norm over the plane through the well (exact form in `gravity.md`). No `sqrt` is needed.
- **Seams.** A regime is one L0 cell, and its stack is computed from its own key, so across a **horizontal** L0 face (when `u = y`) the two stacks need not match. The rule is that the cell with a crust at that height closes its side with a skin (SURF-04) or a section cliff (SURF-03), and when both have crust the two crusts simply join. Across the faces perpendicular to the stack coordinate the stacks do match (§8.2). For `Tilted` and `Curved` the faces cut the slabs obliquely, which is also a section cliff (SURF-03) and is intended.

### 8.2 The sequence along the stack (STR-01, STR-02)

- Units are **64 m** (so a 32768 m cell is 512 units).
- The stack in a regime is an alternating list `Between₀, Crust₁, Between₁, Crust₂, …, Betweenₙ`. It starts and ends with a Between.
- Each piece has a depth in units:
  - **Crust:** `crust_units.0..=crust_units.1` (inclusive), where the ranges per style are in §8.4 (8 to 64 units: 0.5 to 4 km).
  - **Between:** by its kind (§8.3). Kinds give a depth range: 0.5 to 4 km in general (`8..=64` units), with **Shallow** at `4..=6` units (256–384 m, nominally 300 m).
- **Seam Between.** The Between at the very start and end of the stack straddles the L0 face. Its depth is drawn from the **face key**, so both regimes derive the same depth and the same kind without communicating (invariant 4):
  - key: `hash(seed, 0, owner, SEAM_BETWEEN_Y)`, where `owner` is the L0 cell above the face (Y axis); for `Vertical` stacks use the salts `SEAM_BETWEEN_X` or `_Z` with the owner on the +side;
  - depth `D = 2 · range(max(2, lo/2), hi/2 + 1)` units, with `lo..=hi` the range of the seam kind, so `D` is even;
  - kind: `pick_weighted(between_mix)` using the **lower** cell's `between_mix`. The upper cell can evaluate it because a regime is a pure function of its own key;
  - half of the Seam Between (`D / 2` units) is in each cell, so no crust ever touches an L0 face on the stack axis.
- **Interior sequence** (draw order, from the RNG `hash(seed, 0, cell, STACK)`), starting after the lower half-seam:
  1. a crust depth, 2. the following Between kind and depth, 3. the next crust, and so on, until fewer than `crust_min + 3` units remain before the upper half-seam. The last Between is then extended to the upper half-seam. The sequence therefore has at most ~35 pieces.
- **Stratum index.** The crusts are numbered `k = 0, 1, …` from the bottom. A stratum is `(L0 cell, k)`; the next section gives it a character (§10).
- The sequence is cheap (≤ 35 draws), but queries run per L2 cell, so `worldgen` keeps a small cache keyed by L0 cell. The function itself stays pure.
- **Fixed-point rule:** every boundary is an integer number of units, so crust surfaces sit on the 64 m grid and never need float arithmetic.

**Implementation (PW.2.2, `worldgen::stack`).**
- `u` is taken in integer metres relative to the L0 cell, floor-divided by 64. `Tilted` uses the plane through the cell's centre with slope `s/64` towards the heading `yaw`: `shift = floor(s · (a·cx + b·cz) / (64 · isqrt(a² + b²)))` and `u = floor((y − shift) / 64)`, where `(a, b)` is the heading vector and `(cx, cz)` the offset from the cell centre. `isqrt` is the integer square root, so the lean is the tabled slope along the heading and no angle is computed. `Curved` uses `oct3(p − well) / 64` (`gravity.md` §2) and is `Flat` until a well is passed in (PW.6).
- **Wrapping.** `u` is reduced modulo 512 before the lookup. A `Tilted` or `Curved` stack therefore has crusts right up to the L0 faces, where it is cut obliquely (SURF-03), and the top and bottom seam halves meet as one full Between when `u` wraps.
- **Interior algorithm.** After the lower half-seam: repeat — if fewer than `crust_lo + 4` units remain before the upper half-seam, the last Between runs on to 512 and takes the upper seam's kind and flooding; otherwise draw a crust (trimmed to leave 4 units), then a Between kind, depth and flooding. If after that Between fewer than `crust_lo + 4` units would remain, it is replaced by the final Between described above. All draws are still made. The last Between of a stack can therefore be longer than its kind's range, and the Between before a final trim keeps its kind's range.
- **Tilt and wall axis.** The lean heading of a `Tilted` stack and the axis of a `Vertical` one come from `hash(seed, 0, cell, STACK_YAW)` (§12), not from the main RNG, whose nine draws are fixed.
- **Census tags.** `BetweenKind::tag()` (`BTW-01` Shallow, `BTW-02` Abyssal, `BTW-03` Pillared, `BTW-04` Webbed, `STR-02` Normal), `StackGeometry::tag()` (`BTW-06..08`) and `BTW-05` for flooded.
- **Slices.** `slice --stack --axis z --extent 131072` draws crusts and Betweens by kind; see `out/stack-*.png`. A `Vertical` stack seen in a plane parallel to its walls is one flat colour.

### 8.3 Between kinds (BTW-01..05)

A Between's kind decides depth and what fills it. The kinds are chosen per Between by `pick_weighted` over `between_mix` (index in the order below):

| Idx | Kind | Depth (units) | Content (what PW.2.3 builds) |
|---|---|---|---|
| 0 | Shallow (BTW-01) | 4–6 | forest of tethers; roofscape and ceiling-scape almost touch |
| 1 | Normal | 8–32 (0.5–2 km) | the standard mix of tethers, drips, spires, kisses, islands, tubes, cable nets (STR-03..11) |
| 2 | Abyssal (BTW-02) | 48–64 (3–4 km) | few tethers; the far crust is a faint pattern of lights |
| 3 | Pillared (BTW-03) | 8–40 | a hypostyle of tethers on a grid, and nothing else |
| 4 | Webbed (BTW-04) | 8–40 | cables and catwalks only, no tethers |

- **Flooded with fog (BTW-05)** is a modifier, not a depth kind: any Between can be flooded. `flooded = chance(1/6)` per Between, drawn in the sequence after kind and depth. A flooded Between sets local `visibility_m` to 300 m within it (the atmosphere blends through it) and weights lamp content up.
- **Tilted, Vertical and Curved (BTW-06..08)** are not Between kinds but stack geometries (§8.1): they change the shape of every Between and crust in the regime.
- Per-style weights for `between_mix` are in §8.4. The **seam Between** is drawn from the lower cell's `between_mix` and may be any kind.

### 8.4 Per-style stack parameters

| Style | stack weights (Flat, Tilted, Vertical, Curved) | tilt (for Tilted) | crust_units lo range | crust_units hi range | between_mix (Shallow, Normal, Abyssal, Pillared, Webbed) |
|---|---|---|---|---|---|
| Concrete Strata | 80, 8, 12, 0 | 1–3 | 12–20 | 28–48 | 15, 40, 15, 20, 10 |
| Pipeworks | 85, 5, 10, 0 | 1–2 | 16–28 | 40–64 | 40, 35, 3, 12, 10 |
| Pillar Forest | 70, 12, 15, 3 | 2–4 | 10–18 | 24–40 | 10, 30, 10, 45, 5 |
| Hollow Expanse | 55, 20, 15, 10 | 2–6 | 8–12 | 14–26 | 5, 25, 55, 5, 10 |
| Overgrowth | 70, 15, 10, 5 | 2–5 | 12–20 | 30–52 | 20, 35, 5, 10, 30 |

- Each `between_mix` entry is drawn `range(max(0, v − 5), v + 6)` around the table value `v`, then the five are not renormalised (`pick_weighted` takes integer weights). If all five are zero the draw falls back to Normal.
- A tilted stack draws `tilt` as `range_i32(lo, hi+1)` from this table's tilt column; tilt index units are 5°.
- Targets: Pipeworks has thick crusts and shallow Betweens; Hollow Expanse thin crusts and abyssal Betweens (the-web.md §4.1).

## 9. Style palettes of forms, supports, paths, quarters and mutations (REG-03, SC-03, SC-09)

A style supplies base weights for each category. The regime draws its own weights near them (§7.5, step 9). Weights are 0–8; 0 is a ban.

**Families.** A weight applies to a family; the family expands to its members, which are specified where the IDs are (PW.3.x). Each member's own weight inside the family is 1 unless the item that specifies it sets another.

| Category | Family | Members |
|---|---|---|
| Forms (7) | TOWER | FORM-01, 03, 04, 05, 06, 08 |
| | STEP | FORM-02, 10 |
| | FRAMEWORK | FORM-07, 11, 13 |
| | PLANT | FORM-12, 14 |
| | ROUND | FORM-09, 16, 19, 20 |
| | RUIN | FORM-15 |
| | ANGLED | FORM-17, 18, 21, 22, 23 |
| Paths (6) | WALK | PH-01, 03, 04, 05, 06, 07, 08, 16, 17, 18 |
| | ENCLOSED | PH-02, 09, 10, 11, 12, 20 |
| | STREET | PH-13, 14, 15, 19 |
| | STAIR | PV-01..10 |
| | LIFTSLOPE | PV-11..19 (bridges, ramps, wraps, lifts) |
| | LADDER | ladders (the-web.md §9; ART-10) |
| Mutations (6) | POSE | MUT-01, 02, 03 (inverted, tilted, twisted) |
| | DAMAGE | MUT-04, 05, 15 (broken, overgrown, swallowed) |
| | COPY | MUT-06, 07 (doubled, stretched) |
| | HOLLOW | MUT-08, 09, 10 (hollowed, caged, pierced) |
| | LIGHT | MUT-11, 12 (dark, lit) |
| | SCALE | MUT-13, 14 (scaled, fused) |

Supports have 11 entries, one per SUP-01..11 in order. Quarters have 16 entries, QTR-01..16 in order.

**Rules that keep routes possible (connectivity, `ports.md`).**
- The universal route types **PH-01 (catwalk)** and **PV-01 (straight flight)** can never be banned or weighted 0 by a stratum, so routing always has a fallback.
- Every stratum keeps at least one nonzero weight in each of WALK, STAIR, and in forms and supports.
- Bans apply to what structures are built, never to the routes the artery and L2 order need.

### 9.1 Base weights per style

Forms (TOWER, STEP, FRAMEWORK, PLANT, ROUND, RUIN, ANGLED):

| Style | Weights |
|---|---|
| Concrete Strata | 8, 6, 3, 2, 3, 2, 2 |
| Pipeworks | 3, 2, 7, 8, 4, 3, 2 |
| Pillar Forest | 5, 3, 4, 1, 6, 3, 4 |
| Hollow Expanse | 4, 4, 2, 1, 5, 4, 6 |
| Overgrowth | 3, 2, 3, 3, 3, 8, 5 |

Supports (Standing, Hanging, Spanning, Sideways, Collar, Embedded, Suspended, Piled, Nested, Colliding, Radial):

| Style | Weights |
|---|---|
| Concrete Strata | 8, 4, 3, 3, 3, 5, 2, 2, 4, 2, 1 |
| Pipeworks | 5, 3, 4, 4, 6, 3, 4, 6, 3, 3, 1 |
| Pillar Forest | 4, 5, 2, 3, 8, 2, 3, 2, 2, 2, 1 |
| Hollow Expanse | 3, 6, 6, 5, 2, 2, 6, 1, 3, 3, 1 |
| Overgrowth | 4, 4, 3, 3, 4, 6, 5, 8, 5, 4, 1 |

Paths (WALK, ENCLOSED, STREET, STAIR, LIFTSLOPE, LADDER):

| Style | Weights |
|---|---|
| Concrete Strata | 4, 5, 8, 8, 4, 2 |
| Pipeworks | 6, 6, 4, 4, 5, 8 |
| Pillar Forest | 5, 3, 3, 6, 6, 3 |
| Hollow Expanse | 8, 3, 2, 5, 6, 2 |
| Overgrowth | 4, 6, 5, 4, 4, 6 |

Mutations (POSE, DAMAGE, COPY, HOLLOW, LIGHT, SCALE):

| Style | Weights |
|---|---|
| Concrete Strata | 1, 2, 4, 3, 3, 2 |
| Pipeworks | 2, 4, 3, 4, 4, 2 |
| Pillar Forest | 3, 3, 2, 3, 3, 4 |
| Hollow Expanse | 5, 3, 2, 3, 2, 5 |
| Overgrowth | 3, 8, 3, 3, 2, 3 |

Quarters (QTR-01 Dwelling warren, 02 Foundry, 03 Data, 04 Transit, 05 Archive, 06 Hydroponic, 07 Necropolis, 08 Administrative, 09 Bastion, 10 Dock, 11 Waterworks, 12 Observatory, 13 Market, 14 Chapel, 15 Hospital, 16 Machine):

| Style | Weights |
|---|---|
| Concrete Strata | 4, 3, 5, 6, 5, 1, 5, 8, 7, 3, 3, 3, 4, 5, 4, 3 |
| Pipeworks | 5, 8, 3, 4, 2, 2, 2, 2, 4, 6, 8, 1, 3, 1, 2, 8 |
| Pillar Forest | 2, 2, 3, 3, 4, 4, 6, 2, 5, 3, 4, 7, 2, 8, 2, 2 |
| Hollow Expanse | 1, 1, 4, 5, 6, 3, 8, 3, 4, 4, 4, 8, 3, 6, 3, 1 |
| Overgrowth | 8, 2, 1, 2, 3, 8, 4, 2, 2, 2, 6, 3, 6, 4, 6, 2 |

These are first guesses for the art review (P4.3.4), like the palettes in §3. The ids and the order of every family are append-only.

## 10. Stratum character (SC-01..SC-11)

*Owner: PW.2.10 implements; this section specifies.*

### 10.1 The record

Each stratum `(L0 cell, k)` draws a **character** once. Every L1, L2 and L3 cell inside it reads the character from its stratum (found from the cell's stack coordinate `u`), so no cell redraws it.

```rust
pub struct Character {
    pub quarters: [u8; 4],    // SC-02: 2–4 dominant QTR ids; unused slots 0xFF
    pub forms: [u8; 7],       // SC-03, weights 0..=8
    pub supports: [u8; 11],   // SC-03
    pub paths: [u8; 6],       // SC-03
    pub mutations: [u8; 6],   // SC-09
    pub grain_log2: u8,       // SC-04, 6..=12
    pub orientation: OrientationPolicy, // SC-04
    pub scale: u8,            // SC-05, index 0..=6 into the table below
    pub fill: u8,             // SC-06, built fraction in this crust
    pub materials: [u8; 4],   // SC-07, weights over four material slots
    pub palette: PaletteId,   // SC-07, tint chosen among the style's palettes (§3)
    pub light: u8,            // SC-08, baseline light of this stratum
    pub lamps: u8,            // SC-08, lamp density, 0 = none … 255
}
```

**Scale table (SC-05).** `scale` indexes `[×½, ×¾, ×1, ×1½, ×2, ×3, ×4]` stored as the integer pair `(num, den)`: `(1,2), (3,4), (1,1), (3,2), (2,1), (3,1), (4,1)`. It multiplies the **tissue** sizes in L2 and the corridor/room module of L3 (width, floor height, door height of non-person-sized openings). The player-scale openings (doors, handrails, step height) never scale. Scale ≠ 1 is rare: weights `(2, 4, 40, 4, 3, 1, 1)`.

### 10.2 Draw order (SC-01)

An RNG seeded with `absorb(hash(seed, 0, cell, STRATUM_CHAR), k)` (`k` is the stratum index as `u64`), after the stack is known. Draws in this order:

1. `quarters`: the count is `range_i32(2, 5)` (2–4); then for each, `pick_weighted(regime quarters)` with already picked ids set to 0, so they are distinct (SC-02).
2. `forms`, `supports`, `paths`, `mutations`: for each entry, the regime weight `w` perturbed to `clamp(w + range_i32(−3, 4), 0, 8)` (SC-03, SC-09). After all entries of one category:
   - **boost:** `pick_weighted` among the entries picks 1–3 (the count is `range_i32(1, 4)`) and sets them to 8;
   - **ban:** the count is `range_i32(0, 3)` (0–2); each pick is among entries whose regime weight is ≤ 4, and sets them to 0, subject to the connectivity rules of §9. A ban that would break a rule is skipped.
   - the number of draws is fixed by the counts; skipped bans consume their draw.
3. `grain_log2`: a perturbation `range_i32(−2, 3)` of the regime value, clamped to 6..=12 (SC-04).
4. `orientation`: with probability 3/4 the regime's policy; otherwise `pick_weighted` over the style's orientation weights (§7.6). So one stratum may be calm and the next shattered (SC-04).
5. `scale` (SC-05): `pick_weighted` over the scale table's weights.
6. `fill` (SC-06): the regime `fill_crust` ± `range_i32(−12, 13)`, clamped to 178..=230 (the 70–90 % crust range).
7. `materials` (SC-07): four draws `range_i32(0, 9)`. Slots are defined by `PW.4.x` (materials); index 0 is the structural base. The palette: with probability 1/3 a different palette from the style's list.
8. `light` (SC-08): the style's light range, perturbed by `range_i32(−15, 16)` and clamped to 0..=255. `lamps` is `range_i32(0, 256)`.

### 10.3 Seams and cliffs (SC-10, SC-11)

- **Stratum seams (SC-10).** Where two crusts meet along a tether or a Spine (a vertical or oblique feature that links two strata), the cells in contact read **both** characters. A structure keeps its own stratum's character, and the tether's feature takes its form weights from the stratum that owns the cell it stands in, with the adjacent stratum's `quarters[0]` forms allowed to grow onto it with probability 1/4. This makes one stratum's forms grow onto the other's. The adjacency is derived from the stack (the crust above and below a Between), so both cells agree.
- **Section cliffs (SC-11).** A section cliff (SURF-03) shows the crust's floors from outside. The rendering uses each stratum's `light`, `palette` and `scale`, so characters stack visibly "like rock strata".
- **Swallowed (MUT-15)** is a mutation (§9, DAMAGE), not part of the character.

### 10.4 Lookups

`character_at(cell, u)` returns the `Character` of the stratum whose crust contains `u`, or `None` inside a Between. Between content (tethers, drips) uses the character of the **nearer** crust (ties go to the lower one) for its materials and light.

### 10.5 As implemented (PW.2.10, `worldgen::character`)

- `character(seed, l0, k)`, `character_at` (crust only) and `character_near` (the nearer crust inside a Between, ties to the lower one). Salt `STRATUM_CHAR` (0x0115) with `k` absorbed.
- A regime weight of 0 stays 0 (the regime's ban is not lifted by the perturbation); the draw is still made. Boost picks are weighted by the current weights. A ban is skipped when it would zero every entry or the WALK/STAIR path families.
- The orientation exception draws the policy and its tilt cap every time (three draws), then keeps the regime's policy with probability 3/4.
- `light` stays 0 in blackout cells. The palette is a different one of the style's list with probability 1/3.
- Not yet consumed: L2/L3 read the record from PW.3 on; the stratum weights on port types (§4W.5.1) and the seam rule (SC-10) wait for those consumers.

## 11. Census tags

Every Web field has a tag for the census (MET-11): the registry ID (`REG-01`, `BTW-03`, `SC-05`, …). The regime tags the cell with its stack geometry, its orientation policy and its Between kinds; each stratum tags its character fields. `web-report --census` counts the tags in the sample region.

## 12. Salts added for the Web

| Salt | Value | Key | Use |
|---|---|---|---|
| `REGIME_WEB` | `0x0110` | cell (L0) | Web fields (§7.5) |
| `STACK` | `0x0111` | cell (L0) | interior stack sequence (§8.2) |
| `SEAM_BETWEEN_Y` | `0x0112` | cell (L0), the cell above the face | seam Between along Y (§8.2) |
| `SEAM_BETWEEN_X` | `0x0113` | cell (L0), the cell on the + side | seam along X (Vertical stacks) |
| `SEAM_BETWEEN_Z` | `0x0114` | cell (L0), the cell on the + side | seam along Z |
| `STRATUM_CHAR` | `0x0115` | cell (L0) with `k` absorbed | stratum character (§10.2) |
| `ORI_YAW` | `0x0116` | grain cell or L1 cell | frame yaw (§7.4) |
| `STACK_YAW` | `0x0118` | cell (L0) | lean heading of a Tilted stack, wall axis of a Vertical one (§8.2) |
| `BETWEEN_FLOODED` | `0x0117` | cell (L0) | flooded modifier (§8.3), drawn from the `STACK` RNG; this salt is reserved for the standalone query |

New salts are append-only and recorded in `worldgen::salt` when the first user of each lands (PW.2.1 and PW.2.2).
