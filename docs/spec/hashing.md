# Spec: hashing and randomness

**Status: approved by the user on 2026-10-17 (P0.2.2).**

Every random decision in `worldgen` comes from the hash and RNG defined here. Both are pinned bit for bit: changing either changes every world, which requires a `GEN_VERSION` bump (CLAUDE.md §3.8). The executable reference is `docs/spec/hashing_reference.py` (standard-library Python), which generated every vector below.

## 1. Building block: the SplitMix64 finalizer

```rust
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15; // 2^64 / golden ratio, odd

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
```

- This is the output function of Vigna's SplitMix64 (Stafford's "Mix13" variant). Our implementation reproduces the reference generator's published outputs (§3.3, seed 0).
- `mix` is a bijection on `u64`, so two different inputs always give different outputs.
- `mix(0) == 0`. That is why every use below adds `GAMMA` before mixing, so all-zero inputs don't hash to zero.

## 2. The hash

```rust
fn absorb(h: u64, v: u64) -> u64 {
    mix((h ^ v).wrapping_add(GAMMA))
}

pub fn hash(seed: u64, level: u8, cell: I64Vec3, salt: u64) -> u64 {
    let mut h = mix(seed.wrapping_add(GAMMA));
    h = absorb(h, u64::from(level));
    h = absorb(h, cell.x as u64); // two's-complement bit cast, lossless
    h = absorb(h, cell.y as u64);
    h = absorb(h, cell.z as u64);
    absorb(h, salt)
}
```

**Properties**

- **Integer inputs only** (CLAUDE.md §3.3). Negative coordinates are bit-cast, never offset or clamped, so any `i64` is accepted, including values outside the coordinate domain.
- **Order-sensitive:** `(x, y, z)` and `(y, x, z)` give unrelated hashes, because each field is mixed in before the next one arrives.
- **Collision-free per field:** for a fixed prefix, each `absorb` step is a bijection in its new field. Two keys that differ in exactly one field can never collide. Keys that differ in several fields collide with probability about 2⁻⁶⁴.
- **Avalanche:** flipping any single input bit flips each output bit with probability about 0.5. Over 300 random keys per input bit and all 6 fields, the mean flip rate was 0.4997 and the worst per-bit deviation was 0.117, which is within sampling noise for about 22 000 measured cells. The test (`hash::tests::avalanche`, P0.4.5) uses 2000 random keys per input bit, covering all 64 bits of seed, salt and each cell axis, plus five single-bit level pairs. It requires every per-bit flip rate within 0.06 of 0.5 (about 5σ) and a mean within 0.002 of 0.5. At pinned RNG seed `0xA5A5A5A5` it measures a worst deviation of 0.0455 and a mean of 0.49998.
- **`GEN_VERSION` is not hashed.** A version bump marks an intentional output change. It should not also reshuffle every unrelated decision, because that would make golden-test diffs useless.
- **Single entry point:** shared keys map onto these arguments rather than adding new hash functions. A face key hashes the owner cell with one salt per axis. A column key `(level, x, z)` hashes the cell `(x, 0, z)` with a column salt. Distinct salts keep these from colliding with ordinary cells.

### 2.1 Pinned hash vectors

| seed | level | cell (x, y, z) | salt | `hash` |
|---|---|---|---|---|
| 0 | 0 | (0, 0, 0) | 0x0000 | `0xCBD37AD29B93B094` |
| 0 | 0 | (0, 0, 0) | 0x0001 | `0x1C728780035A0397` |
| 1 | 0 | (0, 0, 0) | 0x0000 | `0xFC1F9D47C1A5266C` |
| 0 | 1 | (0, 0, 0) | 0x0000 | `0x75BD1CB46E5EF9B8` |
| 42 | 2 | (−1, 0, 6) | 0x0301 | `0x61F0BC6254485B97` |
| 42 | 2 | (0, −1, 6) | 0x0301 | `0x2C12A530C9FC9D86` |
| 42 | 2 | (6, 0, −1) | 0x0301 | `0xA93A3B02AA52D299` |
| 42 | 4 | (−3, 0, 386) | 0x0501 | `0x5EF3A63462E0ABA8` |
| 0xFFFFFFFFFFFFFFFF | 0 | (−2⁵², 2⁵², −1) | 0x0101 | `0x682F0B3AFE1BAE22` |
| 0xDEADBEEF | 1 | (2⁵⁰, −2⁵⁰, 123456789) | 0x0201 | `0x3ED842A88A148678` |
| 7 | 3 | (i64::MIN, i64::MAX, 0) | 0xFFFFFFFFFFFFFFFF | `0x2B755AF0E8D0291B` |

The cells (−1, 0, 6) and (−3, 0, 386) are the L2 and L4 cells from the worked example in `coordinates.md` §4.

## 3. The RNG: SplitMix64

Per-cell sequential randomness, for when one decision needs many draws (e.g. placing N masses in a cell):

```rust
pub struct Rng { state: u64 }

impl Rng {
    pub fn new(seed: u64) -> Self { Self { state: seed } } // typically seed = hash(...)
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix(self.state)
    }
}
```

**Why SplitMix64 over PCG32:**
- It passes BigCrush.
- Any seed is valid, including 0.
- It has one `u64` of state and reuses the hash's own `mix`.
- It gives 64-bit outputs natively.

Its 2⁶⁴ period is irrelevant at a few hundred draws per cell. PCG32 would need a second, different mixer and only gives 32 bits per step.

### 3.1 Derived operations

All operations are defined on top of `next_u64` so that the draw count is part of the spec.

| Operation | Definition | Draws |
|---|---|---|
| `next_u32()` | `(next_u64() >> 32) as u32` | 1 |
| `next_f32()` | `(next_u32() >> 8) as f32 * 2⁻²⁴`, in [0, 1) and exact (24-bit mantissa) | 1 |
| `below(n: u64)` | uniform in [0, n), n > 0. Lemire's multiply-shift with rejection, see below | ≥ 1 |
| `range_i32(lo, hi)` | `lo + below((hi − lo) as u64) as i32`, half-open [lo, hi), requires lo < hi | ≥ 1 |
| `chance(p: f32)` | `next_f32() < p`, so p ≤ 0 never fires and p ≥ 1 always does | 1 |
| `pick_weighted(w: &[u32])` | `None` if Σw = 0. Otherwise `r = below(Σw)` (sum in `u64`) and the result is the first index i with r < w₀ + … + wᵢ. Zero-weight entries are never picked | ≥ 1 |

`below(n)`:

```rust
let mut m = u128::from(self.next_u64()) * u128::from(n);
if (m as u64) < n {
    let t = n.wrapping_neg() % n;          // (2^64 − n) mod n
    while (m as u64) < t {
        m = u128::from(self.next_u64()) * u128::from(n);
    }
}
(m >> 64) as u64
```

The rejection makes the result exactly uniform. For small `n` it almost never fires (probability below n / 2⁶⁴), so the draw count is 1 in practice but stays deterministic either way.

- **Weights are integers.** Float weights would make the result depend on summation order and rounding.
- `range_i32` with `lo >= hi` is a caller bug. It `debug_assert`s, and in release it returns `lo` without drawing.

### 3.2 Pinned RNG vectors

Raw `next_u64`, first four outputs:

| seed | outputs |
|---|---|
| 0 | `0xE220A8397B1DCDAF`, `0x6E789E6AA1B965F4`, `0x06C45D188009454F`, `0xF88BB8A8724C81EC` |
| 1 | `0x910A2DEC89025CC1`, `0xBEEB8DA1658EEC67`, `0xF893A2EEFB32555E`, `0x71C18690EE42C90B` |
| `0x61F0BC6254485B97` | `0xA99FB48F16F1CB2D`, `0x334890D401E44B6F`, `0x66C0506433B0D742`, `0x98516F89099BD6CB` |

Seed 0 matches the published reference SplitMix64 sequence.

Derived operations, each starting from a fresh `Rng::new(0x61F0BC6254485B97)` (= `hash(42, 2, (−1, 0, 6), 0x0301)`):

| Call, repeated | Results |
|---|---|
| `next_u32` ×4 | `0xA99FB48F`, `0x334890D4`, `0x66C05064`, `0x98516F89` |
| `next_f32` ×4 | 11116468·2⁻²⁴, 3360912·2⁻²⁴, 6733904·2⁻²⁴, 9982319·2⁻²⁴ (≈ 0.662593, 0.200326, 0.401372, 0.594993) |
| `range_i32(−10, 10)` ×8 | 3, −6, −2, 1, 6, 1, 5, 8 |
| `range_i32(i32::MIN, i32::MAX)` ×3 | 698332302, −1287089965, −423604125 |
| `chance(0.25)` ×8 | false, true, false, false, false, false, false, false |
| `pick_weighted(&[1, 0, 3, 6])` ×8 | 3, 2, 3, 3, 3, 3, 3, 3 |

## 4. Salt registry

All salts live in `worldgen::salt` as named `pub const NAME: u64` values.

1. **One salt, one purpose.** Never share a salt between two decisions, even if they look unrelated.
2. **No inline salts.** Every `hash(...)` call names a registry constant.
3. **Values are explicit literals, grouped in blocks of 0x100:**

   | Block | Owner |
   |---|---|
   | `0x0100–0x01FF` | L0 regime |
   | `0x0200–0x02FF` | L1 megaforms |
   | `0x0300–0x03FF` | L2 districts |
   | `0x0400–0x04FF` | L3 blocks |
   | `0x0500–0x05FF` | L4 modules |
   | `0x0600–0x06FF` | L5 dressing |
   | `0x0F00–0x0FFF` | cross-level: the port face salts `PORT_FACE_X/Y/Z` (`0x0F01–0x0F03`), keyed by `FaceKey` at every level. Level-owned features stay in their level's block even when they are rare or cross cells: `RARE_EVENT`/`RARE_EVENT_PLACE` are `0x0102`/`0x0103` (L0), `SANCTUARY` is `0x0206` (L1) |

   The value is not derived from the name, so renaming a constant never changes output.
4. **Append-only.** A salt that is no longer used moves to a `RETIRED` list in the same module, and its value is never reassigned.
5. **Uniqueness test (P0.4.3):** a test collects every active and retired salt and asserts they are distinct.

## 5. World seed

`WorldSeed(u64)`. `WorldSeed::parse(text)`:

1. Trim ASCII whitespace (space, `\t`, `\n`, `\x0C`, `\r`, as in Rust's `is_ascii_whitespace`). If the result is empty, return `Err(SeedError::Empty)`. The start-flow UI picks a random seed instead.
2. If it is 1–20 ASCII digits whose value fits in `u64`, the seed is that number. Leading zeros are allowed, so `"0042"` equals `"42"`.
3. Otherwise the seed is **FNV-1a 64** of the trimmed UTF-8 bytes (offset basis `0xCBF29CE484222325`, prime `0x100000001B3`).

| Input | Seed |
|---|---|
| `"42"`, `" 42\n"`, `"0042"` | 42 |
| `"18446744073709551615"` | `0xFFFFFFFFFFFFFFFF` |
| `"18446744073709551616"` (overflows u64) | `0xEDF2AA6B38FC416D` (FNV) |
| `"superstruct"` | `0xE7E3863EEE77F165` |
| `"BLAME!"` | `0x3E16C973059C069F` |
| `"Ω"` | `0x0AD41307B7283D68` |
| `"-5"` (not all digits) | `0x07D00F07B497D7F7` (FNV) |
| `"1e3"` | `0x44BB4218178916D8` (FNV) |
| `""`, `"  "` | `Err(Empty)` |

## 6. Determinism rules for floating point in `worldgen`

The hash and RNG are integer-only, but generator code turns their outputs into `f32` sizes and positions. Those must be bit-identical on every machine, or the golden tests (P0.11.1) and the horizon promise break.

- **Allowed:** `+ − × ÷`, `sqrt`, comparisons, `floor`/`ceil`/`round`/`trunc`, `abs`, `min`/`max`, and integer↔float conversions. These are exactly specified by IEEE 754, and Rust does not enable fast-math.
- **Banned in anything that affects output:** `sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `powf`, `powi`, `hypot`, `cbrt` and other libm functions, whose results can differ between platforms and libm versions (`powi` can even differ between debug and release builds). Where one is needed, write an in-house polynomial or a lookup table, pinned by tests.
- **Fixed evaluation order:** no parallel float reductions, and no iterating a `HashMap`. Sums run in a defined order.

### 6.1 Rotations and the Web (PW.0.2)

The Web adds oriented primitives (`primitives.md` §9). The determinism rules above are enough for them, with the following explicit list.

- **No trigonometric function runs in `gen`.** Every angle is a table index. The tables are constants:
  - 64 headings as integer pairs `(a, b)`, first quadrant pinned, other quadrants by the exact map `(a, b) → (−b, a)`;
  - 7 tilt slopes `s`;
  - 49 direction vectors with components in `−2..=2`;
  - the discretised catenary tilts.
  A test re-derives the first two with `f64::tan` from the formulas in `primitives.md` and checks them. That test is the only place a trig function appears, and it is not generator code.
- **Operations used to build a frame:** integer arithmetic, `+ − × ÷` and `sqrt` in `f64`, and `floor(x + 0.5)` for rounding positions to Q. All are exactly specified (§6 above), so equal integer keys give identical bits.
- **No FMA.** Rust does not contract `a * b + c` into a fused operation unless `mul_add` is called, and `gen` never calls it (PW.1.1 adds a test that scans the non-test source of `crates/gen/src` for `mul_add`, `sin`, `cos`, `tan`, `atan`, `exp`, `ln`, `powf`, `powi`, `hypot` and `cbrt`; `atmosphere.rs` is the only known user, in a constant-derivation test).
- **Order:** matrices are built and applied in the order written in `primitives.md` §9.1. Dot products and cross products sum left to right.
- **Integers first.** Whatever decides *what exists* (counts, indices, which cell, which line key) is integer. Floats only place the vertices of a feature already decided, and a geometric test whose result changes what exists (a `KeepOut` overlap) is evaluated on the **integer AABB** (`primitives.md` §9.5), not on the float solid.
- **New salts** are append-only in the usual blocks. The orientation salts are listed in `regimes.md` §12.

## 7. GPU port (for P3.3)

WGSL has no 64-bit integers unless `wgpu::Features::SHADER_INT64` is enabled. That feature is available on Vulkan and DX12 (SM 6.0+), and therefore on the RTX 4070 target.

- **Plan:** require `SHADER_INT64` for the SDF horizon and port `mix`, `absorb`, `hash` and the RNG directly.
- **Fallback:** if the feature is missing, emulate `u64` as `vec2<u32>`. `wrapping_mul` then needs a 32×32→64 multiply built from 16-bit halves. This is slower but exact.
- **Testing:** the P3.3.2 CPU-vs-GPU test compares hash outputs bit for bit and float occupancy with a tolerance. GPU float math may contract to FMA and flush denormals, so float results are not expected to match exactly.

## 8. Review decisions (2026-10-17)

1. **Hash design:** approved as drafted, a chained SplitMix64 finalizer. The GPU port relies on `SHADER_INT64`, with a `vec2<u32>` emulation as fallback (§7).
2. **Seed text:** any all-digit string that fits in `u64` is numeric, and leading zeros are ignored, so `"0042"` and `"42"` give the same world.
3. **Salt layout:** blocks of 0x100 per level, append-only, with a retired list (§4).
