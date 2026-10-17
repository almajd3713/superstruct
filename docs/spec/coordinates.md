# Spec: coordinates

How positions, cells and camera-relative vectors are represented and converted. Every formula below has a worked example with negative coordinates. Implementation: `worldgen::coord` (P0.3).

## 1. Units and axes

- Units are **metres**.
- Axes are **right-handed, +Y up**. Gravity points along −Y.
- Camera convention: at yaw 0 the camera looks along **−Z**, and +X is to its right. This matches glam's `look_to_rh` and wgpu's clip space.
- Integer division of a possibly negative value always uses **floor semantics** (`div_euclid` / `rem_euclid` with a positive divisor, or `>>` for powers of two). Truncating division (`/`, `%` on signed integers) is a bug in coordinate code.

  | Expression | Floor (correct) | Truncation (wrong) |
  |---|---|---|
  | −1 ÷ 4 | −1 | 0 |
  | −24 ÷ 512 | −1 | 0 |
  | −3 ÷ 8 | −1 | 0 |
  | −3 mod 128 | 125 | −3 |

## 2. `WorldPos`

```rust
pub struct WorldPos {
    pub sector: I64Vec3, // which 1024 m sector
    pub local: Vec3,     // offset inside that sector
}
```

- `SECTOR_SIZE = 1024.0` m (2¹⁰).
- **Invariant:** after normalization, `0.0 <= local.c < 1024.0` and `local.c` is finite on every axis `c`.
- The absolute position along an axis is `sector.c × 1024 + local.c`. That sum is never computed in floating point outside tests and debug display.
- Precision: `local` is an `f32`, so its worst-case step is 2⁻¹⁴ m ≈ 0.06 mm (in [512, 1024)).

### 2.1 Normalization

Per axis, in this order:

```
k      = floor(local / 1024)          // as i64
local  = local − k × 1024             // f32
if local >= 1024 { local −= 1024; k += 1 }   // f32 rounding fix-up
sector = sector + k                   // checked, see §6
```

The fix-up is required. A tiny negative `local` such as −1×10⁻⁷ gives `k = −1`, and −1×10⁻⁷ + 1024 rounds to exactly `1024.0` in `f32`, which breaks the invariant. A non-finite `local` (NaN or ±∞) is rejected with `CoordError::NonFinite`.

| Input sector | Input local | k | Output sector | Output local | Note |
|---|---|---|---|---|---|
| 0 | −0.5 | −1 | **−1** | 1023.5 | negative crossing |
| 0 | 1030.25 | 1 | 1 | 6.25 | positive crossing |
| 0 | 2048.0 | 2 | 2 | 0.0 | exact multiple |
| 0 | −1×10⁻⁷ | −1 → 0 | 0 | 0.0 | fix-up fires: 1024.0 → 0.0, k → 0 |
| 7 | 1023.99999 | 1 | 8 | 0.0 | the input already rounds to 1024.0 in f32 |

### 2.2 `offset(d: Vec3) -> Result<WorldPos, CoordError>`

`local + d`, then normalize. The sum is formed in `f32`, so `offset` is meant for per-frame motion. At 1 km/s and a 120 Hz step, |d| ≈ 8.3 m. For |d| up to 1024 m the result keeps the 0.06 mm precision of `local`. Large jumps (teleports, debug commands) build a new `WorldPos` from an integer sector instead.

Example: `{sector: (0, 0, 0), local: (3.0, 1.65, 0.25)}.offset((−5.0, 0.0, −0.5))` gives the local sum (−2.0, 1.65, −0.25), which normalizes to `{sector: (−1, 0, −1), local: (1022.0, 1.65, 1023.75)}`.

### 2.3 `delta(&self, other) -> DVec3` (self − other)

```
delta.c = (self.sector.c − other.sector.c) as f64 × 1024 + (self.local.c − other.local.c) as f64
```

The integer subtraction happens first and is exact. The `f64` conversion is exact while |Δsector| < 2⁴³ (that is, Δ < 9×10¹⁵ m). Beyond that the result only matters as "very far away".

Example (x axis only): `a = {sector −1, local 1000.0}` is −24 m and `b = {sector 2, local 10.0}` is 2058 m.
`a.delta(b) = (−1 − 2) × 1024 + (1000 − 10) = −3072 + 990 = −2082 m`. Check: −24 − 2058 = −2082.

## 3. Hierarchy levels

Every level is 8× its child on each axis, so a parent has 8³ = 512 children.

| Level | Name | Cell size `s` | log₂ s | Relation to a 1024 m sector |
|---|---|---|---|---|
| L0 | Regime | 32768 m | 15 | 32 sectors per cell per axis |
| L1 | Megaforms | 4096 m | 12 | 4 sectors per cell per axis |
| L2 | Districts | 512 m | 9 | 2 cells per sector per axis |
| L3 | Blocks | 64 m | 6 | 16 cells per sector per axis |
| L4 | Modules | 8 m | 3 | 128 cells per sector per axis |
| L5 | Dressing | — | — | not a grid level; placed inside L4 cells |

```rust
pub struct CellKey { pub level: u8, pub coords: I64Vec3 }
```

A cell with coordinate `c` covers the half-open interval `[c × s, (c + 1) × s)` on each axis. The minimum corner belongs to the cell and the maximum corner belongs to the next one.

## 4. World → cell

Per axis, for level size `s = 2^k`:

- **`k >= 10`** (L0, L1; a cell spans `m = s / 1024` sectors): `c = sector.div_euclid(m)`. The local part is irrelevant because `0 <= local < 1024`.
- **`k < 10`** (L2–L4; `n = 1024 / s` cells per sector): `c = sector × n + floor(local / s)`. Since `s` is a power of two, `local / s` is exact in `f32` and `floor(local / s)` lies in `[0, n − 1]`.

No float coordinate is ever converted to a cell any other way, and no float ever reaches the hash (CLAUDE.md §3.3).

**Worked example.** `P = {sector: (−1, 0, 3), local: (1000.0, 5.0, 17.5)}`, which is the absolute position (−24, 5, 3089.5) m.

| Level | x | y | z | Check against floor(abs / s) |
|---|---|---|---|---|
| L0 | −1 div 32 = **−1** | 0 div 32 = 0 | 3 div 32 = 0 | ⌊−24/32768⌋ = −1 ✓ |
| L1 | −1 div 4 = **−1** | 0 | 3 div 4 = 0 | ⌊3089.5/4096⌋ = 0 ✓ |
| L2 | −1×2 + ⌊1000/512⌋ = −2 + 1 = **−1** | 0 + ⌊5/512⌋ = 0 | 3×2 + ⌊17.5/512⌋ = **6** | ⌊−24/512⌋ = −1 ✓ |
| L3 | −1×16 + ⌊1000/64⌋ = −16 + 15 = **−1** | 0 | 3×16 + 0 = **48** | ⌊3089.5/64⌋ = 48 ✓ |
| L4 | −1×128 + ⌊1000/8⌋ = −128 + 125 = **−3** | 0 | 3×128 + ⌊17.5/8⌋ = **386** | ⌊−24/8⌋ = −3 ✓ (exactly on the min corner) |

With truncation, L0–L3 x would all come out as 0, which is the wrong cell.

## 5. Cell → AABB and parent/child

### 5.1 Cell minimum corner as a `WorldPos`

Per axis:

- **`k >= 10`:** `sector = c × m`, `local = 0`.
- **`k < 10`:** `sector = c.div_euclid(n)`, `local = c.rem_euclid(n) × s`.

The maximum corner is the minimum corner of cell `c + 1`. Computing it that way keeps it exact and normalized.

| Cell | Formula | Min corner | Absolute |
|---|---|---|---|
| L2 x = −1 (n = 2) | sector = −1 div 2 = −1, local = (−1 mod 2) × 512 = 512 | {−1, 512.0} | −512 m |
| L2 z = 6 | sector = 6 div 2 = 3, local = 0 | {3, 0.0} | 3072 m |
| L1 x = −1 (m = 4) | sector = −4, local = 0 | {−4, 0.0} | −4096 m |
| L3 x = −1 (n = 16) | sector = −1 div 16 = −1, local = 15 × 64 = 960 | {−1, 960.0} | −64 m |
| L4 x = −3 (n = 128) | sector = −3 div 128 = −1, local = (−3 mod 128) × 8 = 125 × 8 = 1000 | {−1, 1000.0} | −24 m |

The last row round-trips to the world → cell example in §4.

### 5.2 Parent, children and neighbours

- `parent(c) = c.div_euclid(8)` (equivalently `c >> 3`).
- `children(c) = { 8c + (i, j, l) | i, j, l ∈ 0..8 }`, 512 cells.
- `neighbor(c, dir) = c + dir` for a unit axis direction `dir ∈ {±X, ±Y, ±Z}`.

Examples: the L4 cell x = −3 has L3 parent ⌊−3/8⌋ = −1, and L3 x = −1 has L2 parent ⌊−1/8⌋ = −1, which matches §4. The L2 cell x = −1 has L3 children x ∈ {−8, …, −1}. The L2 cell z = 6 has L1 parent ⌊6/8⌋ = 0.

## 6. Camera-relative conversion

The GPU never sees large coordinates (CLAUDE.md §3.6).

- The **render origin** is the minimum corner of the camera's current sector, `{sector: camera.sector, local: 0}`. It changes only when the camera changes sector (P2.3.1).
- A chunk's transform is `(chunk_origin.delta(render_origin)) as Vec3`. It is computed on the CPU in `f64` from the integer sector difference and only then cast to `f32`.
- Vertex positions are stored relative to their chunk origin. The camera's position for the view matrix is its `local`, which is already relative to the render origin.

Worked example: the camera is at `{sector: (0, 0, 3), local: (100.0, 1.65, 200.0)}`, so the render origin is `{(0, 0, 3), (0, 0, 0)}`. The chunk is the L2 cell (−1, 0, 6) with origin `{(−1, 0, 3), (512.0, 0.0, 0.0)}`.

- Chunk relative to render origin: Δsector = (−1, 0, 0), so (−1024 + 512, 0, 0) = **(−512, 0, 0)**.
- Camera relative to render origin: (100, 1.65, 200).
- Chunk origin as seen from the eye: (−612, −1.65, −200). Check in absolute metres: (−512, 0, 3072) − (100, 1.65, 3272) = (−612, −1.65, −200) ✓.

Precision: at 50 km from the render origin an `f32` steps in 3.9 mm, which is fine for the far band. Near geometry is within a few hundred metres and stays sub-millimetre.

## 7. Overflow policy at extreme coordinates

**Domain.** A valid sector satisfies `|sector.c| <= MAX_SECTOR = 2⁵²` on every axis.

- That is ±4.6×10¹⁸ m (about 487 light years) per axis. Nihei's City is about 1.6×10¹² m across. Walking at 4 m/s, reaching the edge takes about 3.7×10¹⁰ years, and at 1 km/s noclip about 1.5×10⁸ years. The world is unbounded in practice.
- The bound leaves headroom. An L4 coordinate is `sector × 128 + 127`, so |c| < 2⁵⁹. One more ×8 subdivision stays under 2⁶², neighbours ±1 cannot overflow, and the sector difference of two valid positions (|Δ| ≤ 2⁵³) fits in `i64` and converts to `f64` exactly.

**Rules.**

1. Coordinate arithmetic (`sector`, cell coordinates, neighbours, children, parents) uses `checked_*` operations and the domain check. Leaving the domain returns `CoordError::OutOfDomain`. It never wraps, saturates or panics.
2. `wrapping_*` operations are allowed only inside the hash and RNG (`hashing.md`), where wrap-around is part of the algorithm.
3. Entry points that accept external coordinates (teleport commands, save files, `WorldPos::new`) validate them and return `Result`. `normalize` and `offset` return `Result` because a carry can step out of the domain.
4. `delta` between two valid positions cannot overflow, so it is infallible.
5. World → cell for a valid `WorldPos` cannot overflow, so it is infallible. `CellKey::neighbor` and `children` are checked against the level's domain: `|c| <= MAX_SECTOR × n` for k < 10, or `MAX_SECTOR div m` for k ≥ 10.
6. **Gameplay at the edge:** cells outside the domain are never generated or streamed. The controller treats the domain boundary as a solid wall. Nobody will reach it, but the behaviour is defined.

**Examples.**

- `WorldPos::new((2⁵², 0, 0), (1023.5, 0, 0))` is valid (sector 2⁵², local 1023.5).
- `.offset((1.0, 0, 0))` carries to sector 2⁵² + 1 and returns `Err(OutOfDomain)`.
- `WorldPos::new((−2⁶², 0, 0), …)` returns `Err(OutOfDomain)`. This is the ±2⁶² test in P0.3.6: no panic, no wrapped value.
- At sector −2⁵², local (−0.5, …), normalizing would give sector −2⁵² − 1, so it returns `Err(OutOfDomain)`.
