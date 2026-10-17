//! L1 megaforms (4096 m cells). Spec: `docs/spec/levels.md` §4.
//!
//! Keyed features are pure functions of their keys, so every cell sharing a key agrees.
//! Geometry is in quanta in the L1 cell's local space.

use glam::I64Vec3;

use crate::cavity::is_cavity;
use crate::cell::{Axis, CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::key::{ColumnKey, ColumnSpanKey, LayerKey};
use crate::l1_bridge::bridges;
use crate::l1_port::PORT_CLEARANCE;
use crate::l1_rim::rim_bites;
use crate::prim::{
    CellMeta, CellOutput, ChildConstraint, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb,
    QVec3, Shape,
};
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
    pub(crate) fn dist2_point(&self, x: Q, z: Q) -> i64 {
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

/// The holes cut through this cell's decks: core, shaft and chasm. Rim bites join in P0.7.8.
pub fn deck_holes(seed: WorldSeed, cell: CellKey) -> Vec<Rect> {
    let mut holes = vec![core_footprint(seed, ColumnKey::of(cell))];
    holes.extend(shaft(seed, ColumnSpanKey::of(cell)));
    holes.extend(chasm(seed, cell));
    holes
}

/// Squared separation, for comparing against `dist2_*`.
const SEP2: i64 = (SEPARATION as i64) * (SEPARATION as i64);

/// A rectangle of `w × d` with its min corner drawn on `step`, keeping the face margin.
fn place_rect(rng: &mut Rng, w: Q, d: Q, step: Q) -> Rect {
    let x0 = step_range(rng, SEPARATION, CELL - SEPARATION - w, step);
    let z0 = step_range(rng, SEPARATION, CELL - SEPARATION - d, step);
    Rect {
        x0,
        z0,
        x1: x0 + w,
        z1: z0 + d,
    }
}

/// Whether `r` keeps the separation from the footprints placed before it in the column.
fn clear_of_column(seed: WorldSeed, column: ColumnKey, span: ColumnSpanKey, r: &Rect) -> bool {
    let core = core_footprint(seed, column);
    let stair = stair_footprint(seed, column);
    let trunk_ok = trunk(seed, span).is_none_or(|t| {
        let need = i64::from(t.radius + SEPARATION);
        r.dist2_point(t.centre_x, t.centre_z) >= need * need
    });
    r.dist2_rect(&core) >= SEP2 && stair.is_none_or(|s| r.dist2_rect(&s) >= SEP2) && trunk_ok
}

/// The shaft of a column span (§4.3): a 32 km vertical void, 64–512 m per side.
pub fn shaft(seed: WorldSeed, span: ColumnSpanKey) -> Option<Rect> {
    debug_assert_eq!(span.level, Level::L1);
    let l0 = I64Vec3::new(
        floor_div_pow2(span.x, 3),
        span.sy,
        floor_div_pow2(span.z, 3),
    );
    let d = f32::from(regime(seed, l0).density) / 255.0;
    let mut rng = Rng::new(hash(seed.0, Level::L1, span.hash_coords(), salt::SHAFT));
    if !rng.chance(lerp(0.40, 0.10, d)) {
        return None;
    }
    let step = 16 * M;
    let w = rng.range_i32(4, 33) * step;
    let depth = rng.range_i32(4, 33) * step;
    let r = place_rect(&mut rng, w, depth, step);
    let column = ColumnKey {
        level: Level::L1,
        x: span.x,
        z: span.z,
    };
    clear_of_column(seed, column, span, &r).then_some(r)
}

/// The chasm of one cell (§4.4): 256–2048 m per side, through this cell's decks only.
pub fn chasm(seed: WorldSeed, cell: CellKey) -> Option<Rect> {
    debug_assert_eq!(cell.level, Level::L1);
    let l0 = cell.parent().map_or(I64Vec3::ZERO, |p| p.coords);
    let d = f32::from(regime(seed, l0).density) / 255.0;
    let mut rng = Rng::new(hash(seed.0, Level::L1, cell.coords, salt::CHASM));
    if !rng.chance(lerp(0.30, 0.05, d)) {
        return None;
    }
    let step = 64 * M;
    let w = rng.range_i32(4, 33) * step;
    let depth = rng.range_i32(4, 33) * step;
    let r = place_rect(&mut rng, w, depth, step);
    let span = ColumnSpanKey::of(cell);
    let clear = clear_of_column(seed, ColumnKey::of(cell), span, &r)
        && shaft(seed, span).is_none_or(|sh| r.dist2_rect(&sh) >= SEP2);
    clear.then_some(r)
}

/// Everything L1 emits for one cell so far: face ports, trunk piece, deck pieces, half-bridges,
/// the lift core, and `KeepOut` for bridges, the shaft and the chasm. Cavity cells emit nothing.
pub fn l1_cell(seed: WorldSeed, cell: CellKey) -> CellOutput {
    let mut out = CellOutput {
        key: cell,
        primitives: Vec::new(),
        ports: Vec::new(),
        child_constraints: Vec::new(),
        meta: CellMeta::default(),
    };
    if is_cavity(seed, cell) {
        return out;
    }
    out.ports = crate::l1_port::cell_ports(seed, cell);
    let span = ColumnSpanKey::of(cell);
    let ds = decks(seed, LayerKey::of(cell, DECK_REGION_LOG2));
    let core = core_footprint(seed, ColumnKey::of(cell));
    let t = trunk(seed, span).filter(|t| trunk_piece(t, cell).is_some());
    let br = bridges(cell, &out.ports, core, t.as_ref(), !ds.is_empty());
    if br.trunk_kept {
        out.primitives.extend(t.and_then(|t| trunk_piece(&t, cell)));
    }
    let holes = deck_holes(seed, cell);
    let bites = rim_bites(seed, cell, &ds, &holes, t.as_ref());
    out.primitives.extend(deck_pieces(&ds, &holes, &bites));
    // Children keep clear of the bitten slabs.
    for (deck, rects) in ds.iter().zip(&bites) {
        out.child_constraints
            .extend(rects.iter().map(|r| ChildConstraint::KeepOut {
                aabb: QAabb::new(
                    QVec3::new(r.x0, deck.top - deck.thickness, r.z0),
                    QVec3::new(r.x1, deck.top + PORT_CLEARANCE, r.z1),
                ),
            }));
    }
    // Children keep bridge decks and the opening above them clear.
    out.child_constraints.extend(br.primitives.iter().map(|p| {
        let bb = p.aabb();
        ChildConstraint::KeepOut {
            aabb: QAabb::new(
                bb.min,
                QVec3::new(bb.max.x, bb.max.y + PORT_CLEARANCE, bb.max.z),
            ),
        }
    }));
    let stops = core_stops(&ds, &br.primitives);
    out.primitives.extend(core_pieces(core, &stops));
    out.primitives.extend(br.primitives);
    let full_height = |r: Rect| ChildConstraint::KeepOut {
        aabb: QAabb::new(QVec3::new(r.x0, 0, r.z0), QVec3::new(r.x1, CELL, r.z1)),
    };
    out.child_constraints.extend(
        shaft(seed, span)
            .into_iter()
            .chain(chasm(seed, cell))
            .map(full_height),
    );
    out
}

/// Wall thickness of the lift core.
const CORE_WALL: Q = M;

/// Lift stop heights in a cell (§4.0.1): every deck top and every bridge floor, sorted, unique.
pub fn core_stops(decks: &[Deck], bridges: &[Primitive]) -> Vec<Q> {
    let mut stops: Vec<Q> = decks
        .iter()
        .map(|d| d.top)
        .chain(bridges.iter().map(|b| b.aabb().max.y))
        .collect();
    stops.sort_unstable();
    stops.dedup();
    stops
}

/// The lift core of one cell: the `LiftShaft` volume and its four `METAL` walls, open for
/// `PORT_CLEARANCE` above each stop.
pub fn core_pieces(core: Rect, stops: &[Q]) -> Vec<Primitive> {
    let mut prims = vec![Primitive {
        shape: Shape::LiftShaft {
            aabb: QAabb::new(
                QVec3::new(core.x0, 0, core.z0),
                QVec3::new(core.x1, CELL, core.z1),
            ),
        },
        material: MaterialId::METAL,
        kind: FeatureKind::SHAFT_WALL,
        flags: PrimFlags::NONE,
    }];
    // Solid wall spans are the gaps between openings.
    let mut y = 0;
    let mut spans: Vec<(Q, Q)> = Vec::new();
    for &s in stops {
        if s > y {
            spans.push((y, s.min(CELL)));
        }
        y = y.max(s + PORT_CLEARANCE);
    }
    if y < CELL {
        spans.push((y, CELL));
    }
    let t = CORE_WALL;
    let walls = [
        (core.x0, core.z0, core.x0 + t, core.z1),
        (core.x1 - t, core.z0, core.x1, core.z1),
        (core.x0 + t, core.z0, core.x1 - t, core.z0 + t),
        (core.x0 + t, core.z1 - t, core.x1 - t, core.z1),
    ];
    for (y0, y1) in spans {
        for (x0, z0, x1, z1) in walls {
            prims.push(Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(QVec3::new(x0, y0, z0), QVec3::new(x1, y1, z1)),
                },
                material: MaterialId::METAL,
                kind: FeatureKind::SHAFT_WALL,
                flags: PrimFlags::NONE,
            });
        }
    }
    prims
}

/// The deck slabs of `cell` as boxes: the cell rectangle minus `holes`, per deck.
pub fn deck_pieces(decks: &[Deck], holes: &[Rect], bites: &[Vec<Rect>]) -> Vec<Primitive> {
    let base = decompose(holes);
    decks
        .iter()
        .zip(bites)
        .flat_map(|(deck, bite)| {
            let rects = if bite.is_empty() {
                base.clone()
            } else {
                decompose(&[holes, bite].concat())
            };
            rects.into_iter().map(move |r| Primitive {
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

/// The far-band reduction (§4.8): the `FAR_VISIBLE` primitives of the full output, in order.
/// Trunks, deck pieces and bridges at least 16 m wide. The same data, never a simplification.
pub fn l1_far_primitives(seed: WorldSeed, cell: CellKey) -> Vec<Primitive> {
    let mut prims = l1_cell(seed, cell).primitives;
    prims.retain(|p| p.flags.contains(PrimFlags::FAR_VISIBLE));
    prims
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
                        let mut spans: Vec<_> =
                            deck_pieces(&ds, &deck_holes(seed, cell), &no_bites(&ds))
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
            let rects: Vec<Rect> = deck_pieces(&ds, &holes, &no_bites(&ds))
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

    /// Shafts and chasms keep the separation from everything placed before them, and shafts
    /// are identical through their span.
    #[test]
    fn shafts_and_chasms_respect_separation() {
        let seed = WorldSeed(0x5AF);
        let (mut shafts, mut chasms) = (0, 0);
        for col in columns() {
            let span = ColumnSpanKey {
                level: Level::L1,
                x: col.x,
                z: col.z,
                sy: -1,
            };
            let sh = shaft(seed, span);
            for j in 0..8 {
                let cell = l1(col.x, -8 + j, col.z);
                assert_eq!(shaft(seed, ColumnSpanKey::of(cell)), sh);
            }
            let cell = l1(col.x, -5, col.z);
            let ch = chasm(seed, cell);
            for r in sh.iter().chain(&ch) {
                assert!(r.x0 >= SEPARATION && r.x1 <= CELL - SEPARATION, "{r:?}");
                assert!(r.z0 >= SEPARATION && r.z1 <= CELL - SEPARATION, "{r:?}");
                assert!(clear_of_column(seed, col, span, r), "{r:?}");
            }
            if let (Some(a), Some(b)) = (sh, ch) {
                assert!(a.dist2_rect(&b) >= SEP2);
            }
            shafts += u32::from(sh.is_some());
            chasms += u32::from(ch.is_some());
        }
        assert!(
            shafts > 300 && chasms > 200,
            "{shafts} shafts, {chasms} chasms"
        );
    }

    /// Deck pieces never enter a hole, and every hole is a full-height `KeepOut` for children.
    #[test]
    fn decks_avoid_all_holes() {
        let seed = WorldSeed(0x5AF);
        for x in -20..20 {
            for z in [-7, 3] {
                let cell = l1(x, 1, z);
                let out = l1_cell(seed, cell);
                let holes = deck_holes(seed, cell);
                for p in out
                    .primitives
                    .iter()
                    .filter(|p| p.kind == FeatureKind::DECK)
                {
                    let bb = p.aabb();
                    for h in &holes {
                        let apart = bb.max.x <= h.x0
                            || h.x1 <= bb.min.x
                            || bb.max.z <= h.z0
                            || h.z1 <= bb.min.z;
                        assert!(apart, "{bb:?} in hole {h:?}");
                    }
                }
                let full_height = out
                    .child_constraints
                    .iter()
                    .filter(|c| {
                        matches!(c, ChildConstraint::KeepOut { aabb } if aabb.min.y == 0 && aabb.max.y == CELL)
                    })
                    .count();
                assert_eq!(full_height, holes.len() - 1, "one per shaft and chasm");
            }
        }
    }

    /// P0.7.3: a cavity region contains no L1 solids.
    #[test]
    fn cavity_cells_emit_nothing() {
        use crate::cavity::cavity_mask;
        use crate::regime::{RareEvent, rare_event};
        let seed = WorldSeed(0xCA7);
        let mut checked = 0;
        for i in 0..2000i64 {
            let l0 = I64Vec3::new(i % 50 - 25, i / 50 - 20, 3);
            if rare_event(seed, l0) != RareEvent::GiantCavity {
                continue;
            }
            let mask = cavity_mask(seed, l0);
            for z in 0..8 {
                for y in 0..8 {
                    for x in 0..8 {
                        let cell = CellKey::new(
                            Level::L1,
                            l0 * 8
                                + I64Vec3::from_array([x, y, z].map(|v| i64::try_from(v).unwrap())),
                        );
                        if mask.get([x, y, z]) {
                            checked += 1;
                            let out = l1_cell(seed, cell);
                            assert!(out.primitives.is_empty() && out.child_constraints.is_empty());
                            assert!(out.ports.is_empty());
                        }
                    }
                }
            }
        }
        assert!(checked > 500, "only {checked} cavity cells checked");
    }

    /// Cells of `columns()` in a few layers, skipping none.
    fn sample_cells() -> impl Iterator<Item = CellKey> {
        (-15..15).flat_map(|x| {
            [-4, 0, 3]
                .into_iter()
                .flat_map(move |y| [-6, 1].map(|z| l1(x, y, z)))
        })
    }

    /// Indices of the rects reachable from `start` through touching or overlapping rects.
    fn reachable(rects: &[Rect], start: usize) -> Vec<bool> {
        let touch =
            |a: &Rect, b: &Rect| a.x0 <= b.x1 && b.x0 <= a.x1 && a.z0 <= b.z1 && b.z0 <= a.z1;
        let mut seen = vec![false; rects.len()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            for j in 0..rects.len() {
                if !seen[j] && touch(&rects[i], &rects[j]) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        seen
    }

    /// The face-plane span `(u0, u1)` a box covers on the cell's face `face`, if it touches it.
    fn face_span(cell: CellKey, face: crate::cell::FaceKey, bb: &QAabb) -> Option<(Q, Q)> {
        let positive = face.owner == cell;
        match face.axis {
            Axis::X => {
                let at = if positive {
                    bb.max.x == CELL
                } else {
                    bb.min.x == 0
                };
                at.then_some((bb.min.z, bb.max.z))
            }
            Axis::Z => {
                let at = if positive {
                    bb.max.z == CELL
                } else {
                    bb.min.z == 0
                };
                at.then_some((bb.min.x, bb.max.x))
            }
            Axis::Y => None,
        }
    }

    /// P0.7.5: every L1 port has a deck or a bridge reaching it inside the cell, and every
    /// bridge reaches the core at its own height, clear of a kept trunk.
    #[test]
    fn every_port_is_reached() {
        use crate::port::PortKind;
        let seed = WorldSeed(0xB41D);
        let (mut bridges_checked, mut omitted) = (0, 0);
        for cell in sample_cells() {
            let out = l1_cell(seed, cell);
            let core = core_footprint(seed, ColumnKey::of(cell));
            let kept_trunk = out.primitives.iter().find(|p| p.kind == FeatureKind::TRUNK);
            let t = trunk(seed, ColumnSpanKey::of(cell)).filter(|t| trunk_piece(t, cell).is_some());
            omitted += u32::from(t.is_some() && kept_trunk.is_none());
            for port in &out.ports {
                let kind = match port.kind {
                    PortKind::DeckContinuation => FeatureKind::DECK,
                    PortKind::BridgeEnd => FeatureKind::BRIDGE,
                    _ => continue,
                };
                let at_floor: Vec<QAabb> = out
                    .primitives
                    .iter()
                    .filter(|p| p.kind == kind)
                    .map(Primitive::aabb)
                    .filter(|bb| bb.max.y == port.rect.v0)
                    .collect();
                let mut covered: Vec<(Q, Q)> = at_floor
                    .iter()
                    .filter_map(|bb| face_span(cell, port.face, bb))
                    .collect();
                covered.sort_unstable();
                let mut reach = port.rect.u0;
                for (a, b) in covered {
                    if a <= reach {
                        reach = reach.max(b);
                    }
                }
                assert!(reach >= port.rect.u1, "{cell:?} port {port:?} not reached");

                if kind == FeatureKind::BRIDGE {
                    bridges_checked += 1;
                    let rects: Vec<Rect> = at_floor
                        .iter()
                        .map(|bb| Rect {
                            x0: bb.min.x,
                            z0: bb.min.z,
                            x1: bb.max.x,
                            z1: bb.max.z,
                        })
                        .collect();
                    let start = at_floor
                        .iter()
                        .position(|bb| face_span(cell, port.face, bb).is_some())
                        .unwrap();
                    let seen = reachable(&rects, start);
                    let to_core = rects
                        .iter()
                        .zip(&seen)
                        .any(|(r, &s)| s && r.dist2_rect(&core) == 0);
                    assert!(to_core, "{cell:?} bridge from {port:?} misses the core");
                    for r in &rects {
                        let inside =
                            r.x0 < core.x1 && core.x0 < r.x1 && r.z0 < core.z1 && core.z0 < r.z1;
                        assert!(!inside, "bridge enters the core");
                    }
                }
            }
        }
        assert!(bridges_checked > 200, "{bridges_checked}");
        assert!(omitted * 20 < bridges_checked, "{omitted} trunks omitted");
    }

    /// A kept trunk is never crossed by a bridge route, apart from the landing.
    #[test]
    fn bridges_clear_kept_trunks() {
        let seed = WorldSeed(0xB41D);
        for cell in sample_cells() {
            let out = l1_cell(seed, cell);
            let Some(t) = trunk(seed, ColumnSpanKey::of(cell)) else {
                continue;
            };
            if !out.primitives.iter().any(|p| p.kind == FeatureKind::TRUNK) {
                continue;
            }
            let has_deck = out.primitives.iter().any(|p| p.kind == FeatureKind::DECK);
            let r2 = i64::from(t.radius) * i64::from(t.radius);
            let crossing = out
                .primitives
                .iter()
                .filter(|p| p.kind == FeatureKind::BRIDGE)
                .map(Primitive::aabb)
                .filter(|bb| {
                    let r = Rect {
                        x0: bb.min.x,
                        z0: bb.min.z,
                        x1: bb.max.x,
                        z1: bb.max.z,
                    };
                    r.dist2_point(t.centre_x, t.centre_z) < r2
                })
                .count();
            if has_deck {
                assert_eq!(crossing, 0, "{cell:?}");
            } else {
                assert!(
                    crossing >= 1,
                    "{cell:?}: trunk without deck needs a landing"
                );
            }
        }
    }

    /// P0.7.6: the far set is an ordered subset of the full output, drops only narrow bridges and core pieces,
    /// and is strictly smaller wherever there is a narrow bridge.
    #[test]
    fn far_primitives_are_a_subset() {
        use crate::l1_bridge::FAR_BRIDGE_WIDTH;
        let seed = WorldSeed(0xFA2);
        let mut strict = 0;
        for cell in sample_cells() {
            let full = l1_cell(seed, cell).primitives;
            let far = l1_far_primitives(seed, cell);
            let mut it = full.iter();
            for p in &far {
                assert!(it.any(|q| q == p), "{p:?} not in order in the full output");
            }
            for p in full.iter().filter(|p| !far.contains(p)) {
                if p.kind == FeatureKind::SHAFT_WALL {
                    continue;
                }
                assert_eq!(p.kind, FeatureKind::BRIDGE, "{p:?}");
                let bb = p.aabb();
                let narrow = (bb.max.x - bb.min.x).min(bb.max.z - bb.min.z) < FAR_BRIDGE_WIDTH;
                assert!(narrow, "{p:?}");
            }
            strict += u32::from(far.len() < full.len());
        }
        assert!(strict > 20, "only {strict} cells drop anything");
    }

    fn lift_shaft(out: &CellOutput) -> Vec<QAabb> {
        out.primitives
            .iter()
            .filter_map(|p| match p.shape {
                Shape::LiftShaft { aabb } => Some(aabb),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn core_identical_in_column_and_absent_in_cavities() {
        let seed = WorldSeed(11);
        let mut checked = 0;
        for column in columns().take(200) {
            let want = core_footprint(seed, column);
            for y in [-9, -1, 0, 5, 12] {
                let cell = l1(column.x, y, column.z);
                let shafts = lift_shaft(&l1_cell(seed, cell));
                if is_cavity(seed, cell) {
                    assert!(shafts.is_empty());
                    continue;
                }
                assert_eq!(shafts.len(), 1);
                let a = shafts[0];
                assert_eq!(
                    (a.min.x, a.min.z, a.max.x, a.max.z),
                    (want.x0, want.z0, want.x1, want.z1)
                );
                assert_eq!((a.min.y, a.max.y), (0, CELL));
                checked += 1;
            }
        }
        assert!(checked > 500);
    }

    #[test]
    fn every_deck_and_bridge_has_a_stop() {
        let seed = WorldSeed(11);
        let mut with_bridges = 0;
        for cell in sample_cells() {
            let out = l1_cell(seed, cell);
            if out.primitives.is_empty() {
                continue;
            }
            let walls: Vec<QAabb> = out
                .primitives
                .iter()
                .filter(|p| p.kind == FeatureKind::SHAFT_WALL)
                .filter_map(|p| match p.shape {
                    Shape::Box { aabb } => Some(aabb),
                    _ => None,
                })
                .collect();
            let floors = out.primitives.iter().filter_map(|p| match p.kind {
                FeatureKind::DECK | FeatureKind::BRIDGE => Some(p.aabb().max.y),
                _ => None,
            });
            for f in floors {
                assert!(
                    walls
                        .iter()
                        .all(|w| w.max.y <= f || w.min.y >= f + PORT_CLEARANCE),
                    "no opening at {f} in {cell:?}"
                );
                if out.primitives.iter().any(|p| p.kind == FeatureKind::BRIDGE) {
                    with_bridges += 1;
                }
            }
        }
        assert!(with_bridges > 0);
    }

    fn no_bites(ds: &[Deck]) -> Vec<Vec<Rect>> {
        vec![Vec::new(); ds.len()]
    }

    /// P0.7.8: bites never disconnect a deck from its port faces or the core.
    #[test]
    fn rim_decks_stay_connected() {
        use crate::cavity::cavity_mask;
        use crate::regime::{RareEvent, rare_event};
        let seed = WorldSeed(0xCA7);
        let (mut rims, mut bitten) = (0, 0);
        for i in 0..2000i64 {
            let l0 = I64Vec3::new(i % 50 - 25, i / 50 - 20, 3);
            if rare_event(seed, l0) != RareEvent::GiantCavity {
                continue;
            }
            let mask = cavity_mask(seed, l0);
            for z in 0..8usize {
                for y in 0..8usize {
                    for x in 0..8usize {
                        let cell = CellKey::new(
                            Level::L1,
                            l0 * 8
                                + I64Vec3::from_array([x, y, z].map(|v| i64::try_from(v).unwrap())),
                        );
                        if mask.get([x, y, z]) || !crate::l1_rim::is_rim(seed, cell) {
                            continue;
                        }
                        rims += 1;
                        let out = l1_cell(seed, cell);
                        let core = core_footprint(seed, ColumnKey::of(cell));
                        let ds = decks(seed, LayerKey::of(cell, DECK_REGION_LOG2));
                        let faces: Vec<usize> = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                            .iter()
                            .enumerate()
                            .filter(|(_, (dx, dz))| {
                                !is_cavity(
                                    seed,
                                    l1(cell.coords.x + dx, cell.coords.y, cell.coords.z + dz),
                                )
                            })
                            .map(|(s, _)| s)
                            .collect();
                        for d in &ds {
                            let rects: Vec<Rect> = out
                                .primitives
                                .iter()
                                .filter(|p| p.kind == FeatureKind::DECK && p.aabb().max.y == d.top)
                                .map(|p| {
                                    let bb = p.aabb();
                                    Rect {
                                        x0: bb.min.x,
                                        z0: bb.min.z,
                                        x1: bb.max.x,
                                        z1: bb.max.z,
                                    }
                                })
                                .collect();
                            let full = decompose(&deck_holes(seed, cell)).len();
                            bitten += usize::from(rects.len() != full);
                            // Edge-sharing connectivity (positive overlap on one axis).
                            let share = |a: &Rect, b: &Rect| {
                                (a.x1 == b.x0 || b.x1 == a.x0) && a.z0 < b.z1 && b.z0 < a.z1
                                    || (a.z1 == b.z0 || b.z1 == a.z0) && a.x0 < b.x1 && b.x0 < a.x1
                            };
                            let mut seen = vec![false; rects.len()];
                            let mut stack = vec![0];
                            seen[0] = true;
                            while let Some(a) = stack.pop() {
                                for b in 0..rects.len() {
                                    if !seen[b] && share(&rects[a], &rects[b]) {
                                        seen[b] = true;
                                        stack.push(b);
                                    }
                                }
                            }
                            assert!(seen.iter().all(|&s| s), "{cell:?} deck {d:?} split");
                            for &f in &faces {
                                let at = |r: &Rect| match f {
                                    0 => r.x0 == 0,
                                    1 => r.x1 == CELL,
                                    2 => r.z0 == 0,
                                    _ => r.z1 == CELL,
                                };
                                assert!(rects.iter().any(at), "{cell:?} deck misses face {f}");
                            }
                            assert!(
                                rects.iter().any(|r| r.dist2_rect(&core) == 0),
                                "{cell:?} deck misses core"
                            );
                        }
                    }
                }
            }
        }
        assert!(rims > 100, "{rims}");
        assert!(bitten > 10, "only {bitten} bitten decks");
    }
}
