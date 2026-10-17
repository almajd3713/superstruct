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

`event` is drawn separately (§5).

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
- Geometry details are in P5.2.3. Until then, the generator reserves the footprint as a `KeepOut`.

### 5.3 Not events

- **Slow distant movers** (light trains, Builder silhouettes) are rendering features (P5.2.5), not generator events.
- **Self-similarity** ("the same grammar at every scale", PLAN.md §7) is deferred. It needs its own design pass.

## 6. Salts

| Salt | Value | Key | Use |
|---|---|---|---|
| `REGIME` | `0x0101` | cell (L0) | regime field draws (§1) |
| `RARE_EVENT` | `0x0102` | cell (L0) | event choice (§5.1) |
| `RARE_EVENT_PLACE` | `0x0103` | cell (L0) | cavity placement |
| `RARE_STAIR` | `0x0104` | ColumnKey (L0) | staircase presence and placement (§5.2) |
| `CAVITY_NOISE` | `0x0105` | cell (L1) | cavity voxelization noise (`levels.md` §3) |
