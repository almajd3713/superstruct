//! L1 megaforms (4096 m cells). Spec: `docs/spec/levels.md` §4.
//!
//! Keyed features are pure functions of their keys, so every cell sharing a key agrees.
//! Geometry is in quanta in the L1 cell's local space.

use glam::I64Vec3;

use crate::cell::{Axis, CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::key::{ColumnKey, ColumnSpanKey, LayerKey};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
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

/// log₂ of the deck region width in L1 cells (W = 4, 16 km).
pub const DECK_REGION_LOG2: u32 = 2;

/// One deck slab: its top surface height and its thickness below that, in cell-local quanta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Deck {
    pub top: Q,
    pub thickness: Q,
}

/// The decks of one layer cell in a deck region (§4.2), bottom to top. Every cell sharing
/// the layer key gets the same decks.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)] // weights in [0, 4], n < 4
pub fn decks(seed: WorldSeed, layer: LayerKey) -> Vec<Deck> {
    debug_assert_eq!(layer.level, Level::L1);
    let shift = 3 - DECK_REGION_LOG2;
    let l0 = I64Vec3::new(
        floor_div_pow2(layer.rx, shift),
        floor_div_pow2(layer.y, 3),
        floor_div_pow2(layer.rz, shift),
    );
    let d = f32::from(regime(seed, l0).density) / 255.0;
    let mut rng = Rng::new(hash(seed.0, Level::L1, layer.hash_coords(), salt::DECK));
    // Integer weights round toward zero (§2).
    let weights = [lerp(4.0, 1.0, d), 4.0, lerp(1.0, 3.0, d), lerp(0.0, 2.0, d)].map(|w| w as u32);
    let n = rng.pick_weighted(&weights).unwrap_or(0) as Q;
    let step = 4 * M;
    (0..n)
        .map(|i| {
            let centre = (2 * i + 1) * CELL / (2 * n) / step * step;
            let jitter = CELL / (4 * n) / step;
            let top = centre + step * rng.range_i32(-jitter, jitter + 1);
            let thickness = rng.range_i32(2, 13) * step;
            Deck { top, thickness }
        })
        .collect()
}

/// The holes cut through this cell's decks. Shafts, chasms and rim bites join in later items.
pub fn deck_holes(seed: WorldSeed, cell: CellKey) -> Vec<Rect> {
    vec![core_footprint(seed, ColumnKey::of(cell))]
}

/// The deck slabs of `cell` as boxes: the cell rectangle minus `holes`, per deck.
pub fn deck_pieces(decks: &[Deck], holes: &[Rect]) -> Vec<Primitive> {
    let rects = decompose(holes);
    decks
        .iter()
        .flat_map(|deck| {
            rects.iter().map(move |r| Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(
                        QVec3::new(r.x0, deck.top - deck.thickness, r.z0),
                        QVec3::new(r.x1, deck.top, r.z1),
                    ),
                },
                material: MaterialId::CONCRETE,
                kind: FeatureKind::DECK,
                flags: PrimFlags::FAR_VISIBLE,
            })
        })
        .collect()
}

/// The cell square `[0, CELL)²` minus `holes`, as disjoint rectangles (§4.2).
///
/// Cuts at the sorted hole edges, merges filled grid cells into runs along x within each z
/// row, then extends a run from the previous row when its x-extent is identical. Output is
/// sorted by `(z0, x0)`.
pub fn decompose(holes: &[Rect]) -> Vec<Rect> {
    let clip = |v: Q| v.clamp(0, CELL);
    let mut xs: Vec<Q> = vec![0, CELL];
    let mut zs: Vec<Q> = vec![0, CELL];
    for h in holes {
        xs.extend([clip(h.x0), clip(h.x1)]);
        zs.extend([clip(h.z0), clip(h.z1)]);
    }
    xs.sort_unstable();
    xs.dedup();
    zs.sort_unstable();
    zs.dedup();

    let filled = |x0: Q, z0: Q| {
        !holes
            .iter()
            .any(|h| h.x0 <= x0 && x0 < h.x1 && h.z0 <= z0 && z0 < h.z1)
    };
    let mut done: Vec<Rect> = Vec::new();
    // Runs that are still open, from the previous row.
    let mut open: Vec<Rect> = Vec::new();
    for zw in zs.windows(2) {
        let (z0, z1) = (zw[0], zw[1]);
        let mut row: Vec<(Q, Q)> = Vec::new();
        for xw in xs.windows(2) {
            if filled(xw[0], z0) {
                match row.last_mut() {
                    Some(run) if run.1 == xw[0] => run.1 = xw[1],
                    _ => row.push((xw[0], xw[1])),
                }
            }
        }
        let mut next = Vec::with_capacity(row.len());
        for (x0, x1) in row {
            if let Some(i) = open.iter().position(|r| r.x0 == x0 && r.x1 == x1) {
                let mut r = open.remove(i);
                r.z1 = z1;
                next.push(r);
            } else {
                next.push(Rect { x0, z0, x1, z1 });
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    done.sort_unstable_by_key(|r| (r.z0, r.x0));
    done
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

    fn rect(x0: Q, z0: Q, x1: Q, z1: Q) -> Rect {
        Rect { x0, z0, x1, z1 }
    }

    fn area(r: &Rect) -> i64 {
        i64::from(r.x1 - r.x0) * i64::from(r.z1 - r.z0)
    }

    /// The pieces tile the cell minus the holes: no overlaps, no hole coverage, full area.
    fn assert_partition(pieces: &[Rect], holes: &[Rect]) {
        for (i, a) in pieces.iter().enumerate() {
            assert!(a.x0 < a.x1 && a.z0 < a.z1 && a.x0 >= 0 && a.x1 <= CELL);
            for b in &pieces[i + 1..] {
                assert!(a.x1 <= b.x0 || b.x1 <= a.x0 || a.z1 <= b.z0 || b.z1 <= a.z0);
            }
            for h in holes {
                assert!(a.x1 <= h.x0 || h.x1 <= a.x0 || a.z1 <= h.z0 || h.z1 <= a.z0);
            }
        }
        let cell = i64::from(CELL) * i64::from(CELL);
        let total: i64 = pieces.iter().map(area).sum();
        assert_eq!(total, cell - holes.iter().map(area).sum::<i64>());
    }

    /// Pinned decomposition: a change here changes generator output.
    #[test]
    fn decompose_golden() {
        assert_eq!(decompose(&[]), vec![rect(0, 0, CELL, CELL)]);
        let hole = rect(1000, 2000, 3000, 2500);
        assert_eq!(
            decompose(&[hole]),
            vec![
                rect(0, 0, CELL, 2000),
                rect(0, 2000, 1000, 2500),
                rect(3000, 2000, CELL, 2500),
                rect(0, 2500, CELL, CELL),
            ]
        );
        let holes = [rect(1000, 1000, 2000, 2000), rect(1500, 3000, 2500, 4000)];
        assert_eq!(
            decompose(&holes),
            vec![
                rect(0, 0, CELL, 1000),
                rect(0, 1000, 1000, 2000),
                rect(2000, 1000, CELL, 2000),
                rect(0, 2000, CELL, 3000),
                rect(0, 3000, 1500, 4000),
                rect(2500, 3000, CELL, 4000),
                rect(0, 4000, CELL, CELL),
            ]
        );
    }

    #[test]
    fn decompose_partitions_random_holes() {
        let mut rng = Rng::new(0xDEC);
        for _ in 0..500 {
            let mut holes: Vec<Rect> = Vec::new();
            for _ in 0..rng.range_i32(0, 6) {
                let side = rng.range_i32(1, 32) * 4 * M;
                let x0 = step_range(&mut rng, SEPARATION, CELL - SEPARATION - side, M);
                let z0 = step_range(&mut rng, SEPARATION, CELL - SEPARATION - side, M);
                let h = Rect::square(x0, z0, side);
                if holes.iter().all(|o| h.dist2_rect(o) > 0) {
                    holes.push(h);
                }
            }
            assert_partition(&decompose(&holes), &holes);
        }
    }

    #[test]
    fn decks_are_spaced_and_inside_the_cell() {
        let seed = WorldSeed(0xDE);
        let mut counts = [0u32; 4];
        for rx in -20..20 {
            for y in -10..10 {
                for rz in [-3, 0, 4] {
                    let layer = LayerKey {
                        level: Level::L1,
                        y,
                        rx,
                        rz,
                    };
                    let ds = decks(seed, layer);
                    counts[ds.len()] += 1;
                    for d in &ds {
                        assert!((8 * M..=48 * M).contains(&d.thickness));
                        assert!(d.top - d.thickness >= 0 && d.top <= CELL, "{d:?}");
                        assert_eq!(d.top % (4 * M), 0);
                    }
                    for w in ds.windows(2) {
                        assert!(w[1].top - w[0].top >= 600 * M, "{ds:?}");
                    }
                }
            }
        }
        assert!(counts.iter().all(|&c| c > 0), "deck counts {counts:?}");
    }

    /// P0.7.2: inside a region, neighbours have the same decks, and both sides cover the shared
    /// face over its full width at the same heights, so each deck continues flush.
    #[test]
    fn decks_continue_across_faces_inside_a_region() {
        let seed = WorldSeed(0xDE);
        let mut region_faces = 0;
        let mut differing_region_faces = 0;
        for x in -12..12 {
            for y in [-1, 0, 3] {
                for z in [-5, 2] {
                    let a = l1(x, y, z);
                    let b = l1(x + 1, y, z);
                    let la = LayerKey::of(a, DECK_REGION_LOG2);
                    let lb = LayerKey::of(b, DECK_REGION_LOG2);
                    if la != lb {
                        region_faces += 1;
                        differing_region_faces += u32::from(decks(seed, la) != decks(seed, lb));
                        continue;
                    }
                    let ds = decks(seed, la);
                    assert_eq!(ds, decks(seed, lb));
                    let face = |cell: CellKey, at: Q| -> Vec<(Q, Q, Q, Q)> {
                        let mut spans: Vec<_> = deck_pieces(&ds, &deck_holes(seed, cell))
                            .iter()
                            .map(Primitive::aabb)
                            .filter(|bb| {
                                if at == 0 {
                                    bb.min.x == 0
                                } else {
                                    bb.max.x == CELL
                                }
                            })
                            .map(|bb| (bb.min.y, bb.max.y, bb.min.z, bb.max.z))
                            .collect();
                        spans.sort_unstable();
                        // Piece boundaries depend on each cell's own holes; compare coverage.
                        let mut merged: Vec<(Q, Q, Q, Q)> = Vec::new();
                        for sp in spans {
                            match merged.last_mut() {
                                Some(m) if (m.0, m.1) == (sp.0, sp.1) && m.3 == sp.2 => m.3 = sp.3,
                                _ => merged.push(sp),
                            }
                        }
                        merged
                    };
                    let (fa, fb) = (face(a, CELL), face(b, 0));
                    assert_eq!(fa, fb, "{a:?} | {b:?}");
                    for d in &ds {
                        let width: Q = fa
                            .iter()
                            .filter(|s| (s.0, s.1) == (d.top - d.thickness, d.top))
                            .map(|s| s.3 - s.2)
                            .sum();
                        assert_eq!(width, CELL, "deck {d:?} is not flush across the face");
                    }
                }
            }
        }
        assert!(
            differing_region_faces * 2 > region_faces,
            "region faces should mostly differ"
        );
    }

    #[test]
    fn deck_pieces_avoid_the_core() {
        let seed = WorldSeed(0xDE);
        for x in -10..10 {
            let cell = l1(x, 2, -x);
            let ds = decks(seed, LayerKey::of(cell, DECK_REGION_LOG2));
            let holes = deck_holes(seed, cell);
            let rects: Vec<Rect> = deck_pieces(&ds, &holes)
                .iter()
                .take(decompose(&holes).len())
                .map(|p| {
                    let bb = p.aabb();
                    rect(bb.min.x, bb.min.z, bb.max.x, bb.max.z)
                })
                .collect();
            if !ds.is_empty() {
                assert_partition(&rects, &holes);
            }
        }
    }
}
