//! L1 megaforms (4096 m cells). Spec: `docs/spec/levels.md` §4.
//!
//! Keyed features are pure functions of their keys, so every cell sharing a key agrees.
//! Geometry is in quanta in the L1 cell's local space.

use glam::I64Vec3;

use crate::cell::{Axis, CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::key::{ColumnKey, ColumnSpanKey};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QVec3, Shape};
use crate::regime::{rare_stair, regime};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// One metre in quanta.
const M: Q = 16;
/// L1 cell edge in quanta.
pub const CELL: Q = 4096 * M;
/// Separation between hole and obstacle footprints, and from the cell faces (§4.0).
pub const SEPARATION: Q = 64 * M;

/// A horizontal footprint in cell-local quanta, half-open: `[x0, x1) × [z0, z1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x0: Q,
    pub z0: Q,
    pub x1: Q,
    pub z1: Q,
}

impl Rect {
    pub fn square(x0: Q, z0: Q, side: Q) -> Self {
        Self {
            x0,
            z0,
            x1: x0 + side,
            z1: z0 + side,
        }
    }

    /// Squared distance from the point to the rectangle (0 inside).
    fn dist2_point(&self, x: Q, z: Q) -> i64 {
        let dx = i64::from((self.x0 - x).max(x - self.x1).max(0));
        let dz = i64::from((self.z0 - z).max(z - self.z1).max(0));
        dx * dx + dz * dz
    }

    /// Squared distance between two rectangles (0 if they overlap or touch).
    fn dist2_rect(&self, o: &Rect) -> i64 {
        let dx = i64::from((o.x0 - self.x1).max(self.x0 - o.x1).max(0));
        let dz = i64::from((o.z0 - self.z1).max(self.z0 - o.z1).max(0));
        dx * dx + dz * dz
    }
}

/// The lift core footprint of L1 column `column` (§4.0.1): square, 16–32 m, step 4 m.
pub fn core_footprint(seed: WorldSeed, column: ColumnKey) -> Rect {
    debug_assert_eq!(column.level, Level::L1);
    let mut rng = Rng::new(hash(seed.0, Level::L1, column.hash_coords(), salt::CORE));
    let side = rng.range_i32(4, 9) * 4 * M;
    let x0 = step_range(&mut rng, SEPARATION, CELL - SEPARATION - side, 4 * M);
    let z0 = step_range(&mut rng, SEPARATION, CELL - SEPARATION - side, 4 * M);
    Rect::square(x0, z0, side)
}

/// Infinite-staircase footprint side (`regimes.md` §5.2).
pub const STAIR_SIDE: Q = 64 * M;

/// The infinite-staircase footprint in L1 column `column`, if its L0 column has a staircase
/// in this L1 column and the footprint keeps clear of the core.
pub fn stair_footprint(seed: WorldSeed, column: ColumnKey) -> Option<Rect> {
    debug_assert_eq!(column.level, Level::L1);
    let l0 = ColumnKey {
        level: Level::L0,
        x: floor_div_pow2(column.x, 3),
        z: floor_div_pow2(column.z, 3),
    };
    let stair = rare_stair(seed, l0)?;
    if i64::from(stair.l1_x) != floor_mod_pow2(column.x, 3)
        || i64::from(stair.l1_z) != floor_mod_pow2(column.z, 3)
    {
        return None;
    }
    // Continues the staircase's own RNG after presence and L1 column.
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0.hash_coords(), salt::RARE_STAIR));
    rng.below(1000);
    rng.below(8);
    rng.below(8);
    let hi = CELL - SEPARATION - STAIR_SIDE;
    let x0 = step_range(&mut rng, SEPARATION, hi, 16 * M);
    let z0 = step_range(&mut rng, SEPARATION, hi, 16 * M);
    let rect = Rect::square(x0, z0, STAIR_SIDE);
    let core = core_footprint(seed, column);
    let sep = i64::from(SEPARATION);
    (rect.dist2_rect(&core) >= sep * sep).then_some(rect)
}

/// A trunk over one column span (§4.1): a vertical cylinder through span cells `a..=b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Trunk {
    pub centre_x: Q,
    pub centre_z: Q,
    pub radius: Q,
    /// Bottom and top cell within the span, `0 ≤ a ≤ b ≤ 7`.
    pub a: u8,
    pub b: u8,
}

pub const TRUNK_MIN_RADIUS: Q = 96 * M;
/// 16 m in quanta, for stepping the radius down.
const RADIUS_STEP: usize = 256;

/// The trunk of a column span, if any.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // range(0, 8) < 8
pub fn trunk(seed: WorldSeed, span: ColumnSpanKey) -> Option<Trunk> {
    debug_assert_eq!(span.level, Level::L1);
    let l0 = I64Vec3::new(
        floor_div_pow2(span.x, 3),
        span.sy,
        floor_div_pow2(span.z, 3),
    );
    let v = f32::from(regime(seed, l0).verticality) / 255.0;
    let mut rng = Rng::new(hash(seed.0, Level::L1, span.hash_coords(), salt::TRUNK));
    if !rng.chance(lerp(0.30, 0.80, v)) {
        return None;
    }
    let drawn_radius = rng.range_i32(6, 26) * 16 * M;
    let lo = drawn_radius + SEPARATION;
    let centre_x = step_range(&mut rng, lo, CELL - lo, 16 * M);
    let centre_z = step_range(&mut rng, lo, CELL - lo, 16 * M);
    let (a, b) = if rng.chance(0.5) {
        (0, 7)
    } else {
        let a = rng.range_i32(0, 8);
        (a, rng.range_i32(a, 8))
    };

    let column = ColumnKey {
        level: Level::L1,
        x: span.x,
        z: span.z,
    };
    let earlier = [
        Some(core_footprint(seed, column)),
        stair_footprint(seed, column),
    ];
    let clear = |radius: Q| {
        let need = i64::from(radius + SEPARATION);
        earlier
            .iter()
            .flatten()
            .all(|r| r.dist2_point(centre_x, centre_z) >= need * need)
    };
    let radius = (TRUNK_MIN_RADIUS..=drawn_radius)
        .rev()
        .step_by(RADIUS_STEP)
        .find(|&r| clear(r))?;

    Some(Trunk {
        centre_x,
        centre_z,
        radius,
        a: a as u8,
        b: b as u8,
    })
}

/// The piece of `trunk` that L1 cell `cell` emits: the full cell height if the cell is
/// within `a..=b` of its span.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // floor_mod < 8
pub fn trunk_piece(trunk: &Trunk, cell: CellKey) -> Option<Primitive> {
    let j = floor_mod_pow2(cell.coords.y, 3) as u8;
    (trunk.a..=trunk.b).contains(&j).then(|| Primitive {
        shape: Shape::cylinder(
            Axis::Y,
            QVec3::new(trunk.centre_x, 0, trunk.centre_z),
            CELL,
            trunk.radius,
        ),
        material: MaterialId::CONCRETE,
        kind: FeatureKind::TRUNK,
        flags: PrimFlags::FAR_VISIBLE,
    })
}

/// `lo + step · range(0, n)`, uniform over the multiples of `step` in `[lo, hi]`.
fn step_range(rng: &mut Rng, lo: Q, hi: Q, step: Q) -> Q {
    debug_assert!(lo <= hi && (hi - lo) % step == 0, "{lo}..={hi} step {step}");
    lo + step * rng.range_i32(0, (hi - lo) / step + 1)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l1(x: i64, y: i64, z: i64) -> CellKey {
        CellKey::new(Level::L1, I64Vec3::new(x, y, z))
    }

    fn columns() -> impl Iterator<Item = ColumnKey> {
        (-30..30).flat_map(|x| {
            (-30..30).map(move |z| ColumnKey {
                level: Level::L1,
                x,
                z,
            })
        })
    }

    /// P0.7.1: every cell of a span derives the same trunk, and the pieces stack into one
    /// cylinder through cells `a..=b`.
    #[test]
    fn trunk_is_identical_from_every_cell_in_its_span() {
        let seed = WorldSeed(0x7C);
        let mut spanning = 0;
        for col in columns() {
            for sy in [-2i64, 0, 5] {
                let cells: Vec<CellKey> = (0..8).map(|j| l1(col.x, sy * 8 + j, col.z)).collect();
                let trunks: Vec<_> = cells
                    .iter()
                    .map(|&c| trunk(seed, ColumnSpanKey::of(c)))
                    .collect();
                assert!(trunks.windows(2).all(|w| w[0] == w[1]), "{col:?} sy {sy}");
                let Some(t) = trunks[0] else { continue };
                if t.b > t.a {
                    spanning += 1;
                }
                for (j, &cell) in cells.iter().enumerate() {
                    let piece = trunk_piece(&t, cell);
                    let inside = (usize::from(t.a)..=usize::from(t.b)).contains(&j);
                    assert_eq!(piece.is_some(), inside, "{t:?} cell {j}");
                    if let Some(p) = piece {
                        let bb = p.aabb();
                        assert_eq!((bb.min.y, bb.max.y), (0, CELL), "full cell height");
                    }
                }
            }
        }
        assert!(spanning > 1000, "only {spanning} multi-cell trunks");
    }

    /// Trunks keep the separation from the cell faces, the core and the staircase.
    #[test]
    fn trunk_respects_separation() {
        let seed = WorldSeed(0x7C);
        let sep = i64::from(SEPARATION);
        let mut found = 0;
        for col in columns() {
            let span = ColumnSpanKey {
                level: Level::L1,
                x: col.x,
                z: col.z,
                sy: 1,
            };
            let Some(t) = trunk(seed, span) else { continue };
            found += 1;
            assert!(t.radius >= TRUNK_MIN_RADIUS && t.radius <= 400 * M);
            assert_eq!(t.radius % (16 * M), 0);
            for c in [t.centre_x, t.centre_z] {
                assert!(c - t.radius >= SEPARATION && c + t.radius <= CELL - SEPARATION);
            }
            let need = i64::from(t.radius) + sep;
            let core = core_footprint(seed, col);
            assert!(
                core.dist2_point(t.centre_x, t.centre_z) >= need * need,
                "{t:?}"
            );
            if let Some(s) = stair_footprint(seed, col) {
                assert!(
                    s.dist2_point(t.centre_x, t.centre_z) >= need * need,
                    "{t:?}"
                );
            }
        }
        assert!(found > 1000, "only {found} trunks in 3600 columns");
    }

    /// Existence follows `lerp(0.30, 0.80, v)` with the span's regime.
    #[test]
    fn trunk_frequency_tracks_verticality() {
        let seed = WorldSeed(0x7C);
        let (mut expected, mut got) = (0.0f64, 0u32);
        let mut n = 0u32;
        for col in columns() {
            let span = ColumnSpanKey {
                level: Level::L1,
                x: col.x,
                z: col.z,
                sy: 0,
            };
            let l0 = I64Vec3::new(floor_div_pow2(col.x, 3), 0, floor_div_pow2(col.z, 3));
            let v = f64::from(regime(seed, l0).verticality) / 255.0;
            expected += 0.30 + 0.50 * v;
            got += u32::from(trunk(seed, span).is_some());
            n += 1;
        }
        // Core conflicts remove a few trunks, so allow a small shortfall.
        let ratio = f64::from(got) / expected;
        assert!(
            (0.93..1.04).contains(&ratio),
            "{got} trunks vs {expected:.0} of {n}"
        );
    }

    #[test]
    fn core_footprint_is_per_column_and_in_range() {
        let seed = WorldSeed(5);
        for col in columns() {
            let r = core_footprint(seed, col);
            let side = r.x1 - r.x0;
            assert_eq!(side, r.z1 - r.z0);
            assert!((16 * M..=32 * M).contains(&side) && side % (4 * M) == 0);
            assert!(r.x0 >= SEPARATION && r.x1 <= CELL - SEPARATION);
            assert!(r.z0 >= SEPARATION && r.z1 <= CELL - SEPARATION);
            for y in [-9, 0, 77] {
                assert_eq!(core_footprint(seed, ColumnKey::of(l1(col.x, y, col.z))), r);
            }
        }
    }

    /// Exactly one L1 column per staircase L0 column has a footprint (unless rejected).
    #[test]
    fn stair_footprint_lies_in_its_l1_column() {
        let seed = WorldSeed(0x57A1);
        let (mut l0_stairs, mut footprints) = (0, 0);
        for x0 in -60..60 {
            for z0 in -60..60 {
                let l0 = ColumnKey {
                    level: Level::L0,
                    x: x0,
                    z: z0,
                };
                let Some(s) = rare_stair(seed, l0) else {
                    continue;
                };
                l0_stairs += 1;
                for dx in 0..8 {
                    for dz in 0..8 {
                        let col = ColumnKey {
                            level: Level::L1,
                            x: x0 * 8 + dx,
                            z: z0 * 8 + dz,
                        };
                        let fp = stair_footprint(seed, col);
                        let here = (dx, dz) == (i64::from(s.l1_x), i64::from(s.l1_z));
                        assert!(here || fp.is_none());
                        if let Some(r) = fp {
                            footprints += 1;
                            assert_eq!(r.x1 - r.x0, STAIR_SIDE);
                            assert!(r.x0 >= SEPARATION && r.x1 <= CELL - SEPARATION);
                            assert!(r.z0 >= SEPARATION && r.z1 <= CELL - SEPARATION);
                        }
                    }
                }
            }
        }
        assert!(l0_stairs > 20, "{l0_stairs}");
        assert!(
            footprints * 10 >= l0_stairs * 9,
            "{footprints} of {l0_stairs}"
        );
    }
}
