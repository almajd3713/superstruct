//! L2 column features: lift shafts `[LIFT_SHAFT]`. Spec: `docs/spec/levels.md` §5.2.
//!
//! A shaft is keyed by `ColumnSpanKey(L2)`: every L2 cell of the span (the eight cells inside
//! one L1 parent) derives the same shaft and emits its own slice. It stands on the parent's
//! decks and rises above each to a viewing platform; stops are at the deck top and the platform.

use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::key::ColumnSpanKey;
use crate::l1::{Rect, lift_pieces};
use crate::l1_port::PORT_CLEARANCE;
use crate::l2_mass::{Parent, inflate};
use crate::prim::{
    CellOutput, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape,
};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
const CELL: Q = 512 * M;
/// Keeps shafts clear of the 64 m port corridors along the cell faces, plus 8 m.
const RIM: Q = 72 * M;
/// Width of the viewing platform around the shaft.
const PLATFORM: Q = 4 * M;
/// Extra room kept around the platform from other L1 content.
const GAP: Q = 8 * M;

/// A lift shaft of one L2 column span, in the parent L1 cell's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftColumn {
    pub rect: Rect,
    /// `(deck top, platform height)` for each deck of the parent the shaft stands on.
    pub rises: Vec<(Q, Q)>,
}

/// The lift shaft of `span`, whose L1 parent is `parent_key` with output `parent`.
#[allow(clippy::cast_precision_loss)] // verticality is a u8
pub fn lift_column(
    seed: WorldSeed,
    span: ColumnSpanKey,
    parent: &CellOutput,
) -> Option<LiftColumn> {
    debug_assert_eq!(span.level, Level::L2);
    let l0 = glam::I64Vec3::new(
        floor_div_pow2(span.x, 6),
        floor_div_pow2(span.sy, 3),
        floor_div_pow2(span.z, 6),
    );
    let v = f32::from(regime(seed, l0).verticality) / 255.0;
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L2,
        span.hash_coords(),
        salt::LIFT_SHAFT,
    ));
    let exists = rng.chance(0.10 + (0.30 - 0.10) * v);
    let wx = 4 * M * rng.range_i32(2, 5);
    let wz = 4 * M * rng.range_i32(2, 5);
    let ux = rng.range_i32(0, (CELL - 2 * RIM - wx) / (4 * M) + 1);
    let uz = rng.range_i32(0, (CELL - 2 * RIM - wz) / (4 * M) + 1);
    let rise = 4 * M * rng.range_i32(4, 17);
    if !exists {
        return None;
    }
    // The span's column inside its L1 parent.
    let (cx, cz) = (
        Q::try_from(floor_mod_pow2(span.x, 3)).ok()? * CELL,
        Q::try_from(floor_mod_pow2(span.z, 3)).ok()? * CELL,
    );
    let x0 = cx + RIM + 4 * M * ux;
    let z0 = cz + RIM + 4 * M * uz;
    let rect = Rect {
        x0,
        z0,
        x1: x0 + wx,
        z1: z0 + wz,
    };
    let ctx = Parent::new(parent, [0, 0, 0]);
    let tops = ctx.all_deck_tops();
    if tops.is_empty() {
        return None;
    }
    if rises_exceed(&tops, rise) {
        return None;
    }
    let room = PLATFORM + GAP;
    let rises: Vec<(Q, Q)> = tops.iter().map(|&t| (t, t + rise)).collect();
    let fits = rises.iter().all(|&(t, top)| {
        ctx.fits(&QAabb::new(
            QVec3::new(rect.x0 - room, t, rect.z0 - room),
            QVec3::new(rect.x1 + room, top + PORT_CLEARANCE, rect.z1 + room),
        ))
    });
    fits.then_some(LiftColumn { rect, rises })
}

/// Whether any platform would stand above the top of the L1 cell.
fn rises_exceed(tops: &[Q], rise: Q) -> bool {
    tops.iter()
        .any(|&t| t + rise + PORT_CLEARANCE > crate::l1::CELL)
}

impl LiftColumn {
    /// The slice of the shaft in the L2 cell at `origin` (in the parent's frame): shaft,
    /// walls and platforms, and the volume children must leave alone. All in cell-local
    /// quanta.
    pub fn slice(&self, origin: [Q; 3]) -> (Vec<Primitive>, Vec<QAabb>) {
        let [ox, oy, oz] = origin;
        let rect = Rect {
            x0: self.rect.x0 - ox,
            z0: self.rect.z0 - oz,
            x1: self.rect.x1 - ox,
            z1: self.rect.z1 - oz,
        };
        let mut prims = Vec::new();
        let mut keepouts = Vec::new();
        for &(t, top) in &self.rises {
            let (lo, hi) = ((t - oy).max(0), (top + PORT_CLEARANCE - oy).min(CELL));
            if lo >= hi {
                continue;
            }
            prims.extend(lift_pieces(rect, lo, hi, &[t - oy, top - oy]));
            // Platform ring: four boxes around the shaft, top at the stop height.
            let thin = 8;
            if (0..=CELL).contains(&(top - oy)) {
                let (y0, y1) = ((top - oy - thin).max(0), top - oy);
                let o = PLATFORM;
                let ring = [
                    (rect.x0 - o, rect.z0 - o, rect.x1 + o, rect.z0),
                    (rect.x0 - o, rect.z1, rect.x1 + o, rect.z1 + o),
                    (rect.x0 - o, rect.z0, rect.x0, rect.z1),
                    (rect.x1, rect.z0, rect.x1 + o, rect.z1),
                ];
                if y0 < y1 {
                    prims.extend(ring.into_iter().map(|(x0, z0, x1, z1)| Primitive {
                        shape: Shape::Box {
                            aabb: QAabb::new(QVec3::new(x0, y0, z0), QVec3::new(x1, y1, z1)),
                        },
                        material: MaterialId::GRATING,
                        kind: FeatureKind::WALKWAY,
                        flags: PrimFlags::NONE,
                    }));
                }
            }
            keepouts.push(inflate_xz(
                &QAabb::new(
                    QVec3::new(rect.x0, lo, rect.z0),
                    QVec3::new(rect.x1, hi, rect.z1),
                ),
                PLATFORM,
            ));
        }
        (prims, keepouts)
    }
}

fn inflate_xz(a: &QAabb, by: Q) -> QAabb {
    let big = inflate(a, by);
    QAabb::new(
        QVec3::new(big.min.x, a.min.y, big.min.z),
        QVec3::new(big.max.x, a.max.y, big.max.z),
    )
}

/// The shaft's span key for `cell`.
pub fn span_of(cell: CellKey) -> ColumnSpanKey {
    ColumnSpanKey::of(cell)
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::l2::l2_cell_in;
    use crate::l2_port::L1Cache;
    use crate::walk::{Nav, cell_window};

    #[allow(clippy::too_many_lines)] // one scan with two assertions per shaft
    /// P0.8.5: the shaft is the same from every L2 cell it passes through, and its slices tile
    /// the shaft's height. Where one cell holds a deck and its platform, they are connected.
    #[test]
    fn shaft_is_identical_across_its_span() {
        let seed = WorldSeed(0x0805);
        let (mut shafts, mut joined) = (0, 0);
        for px in -25..25 {
            for (py, pz) in [(0, 1), (2, -3)] {
                let parent_key = CellKey::new(Level::L1, I64Vec3::new(px, py, pz));
                let cache = L1Cache::default();
                let parent = cache.get(seed, parent_key);
                for (cx, cz) in [(0, 0), (3, 5), (7, 2)] {
                    let span = ColumnSpanKey {
                        level: Level::L2,
                        x: px * 8 + cx,
                        z: pz * 8 + cz,
                        sy: py,
                    };
                    let Some(col) = lift_column(seed, span, &parent) else {
                        continue;
                    };
                    shafts += 1;
                    let mut covered: Vec<(Q, Q)> = Vec::new();
                    for cy in 0..8i32 {
                        let cell = CellKey::new(
                            Level::L2,
                            I64Vec3::new(span.x, py * 8 + i64::from(cy), span.z),
                        );
                        assert_eq!(
                            lift_column(seed, span_of(cell), &parent).as_ref(),
                            Some(&col)
                        );
                        let out = l2_cell_in(seed, &cache, cell);
                        let oy = cy * CELL;
                        for p in &out.primitives {
                            if let Shape::LiftShaft { aabb } = p.shape {
                                let (ox, oz) = (
                                    Q::try_from(floor_mod_pow2(span.x, 3)).unwrap() * CELL,
                                    Q::try_from(floor_mod_pow2(span.z, 3)).unwrap() * CELL,
                                );
                                // Only the L2 shafts: the L1 core is not in an L2 output.
                                assert_eq!(
                                    (
                                        aabb.min.x + ox,
                                        aabb.min.z + oz,
                                        aabb.max.x + ox,
                                        aabb.max.z + oz
                                    ),
                                    (col.rect.x0, col.rect.z0, col.rect.x1, col.rect.z1)
                                );
                                covered.push((aabb.min.y + oy, aabb.max.y + oy));
                            }
                        }
                    }
                    covered.sort_unstable();
                    for &(t, top) in &col.rises {
                        let (lo, hi) = (t, top + PORT_CLEARANCE);
                        let mut at = lo;
                        for &(a, b) in &covered {
                            if a <= at {
                                at = at.max(b);
                            }
                        }
                        assert!(at >= hi, "{span:?}: slices stop at {at}, want {hi}");
                    }
                    // A cell that holds a deck top and its platform joins them.
                    for &(t, top) in &col.rises {
                        if (t / CELL) != (top / CELL) || top % CELL == 0 {
                            continue;
                        }
                        let cy = t / CELL;
                        let cell = CellKey::new(
                            Level::L2,
                            I64Vec3::new(span.x, py * 8 + i64::from(cy), span.z),
                        );
                        let out = l2_cell_in(seed, &cache, cell);
                        let origin = [
                            Q::try_from(floor_mod_pow2(span.x, 3)).unwrap() * CELL,
                            cy * CELL,
                            Q::try_from(floor_mod_pow2(span.z, 3)).unwrap() * CELL,
                        ];
                        let mut prims = cell_window(&parent.primitives, origin, CELL);
                        let n = prims.len();
                        prims.extend(out.primitives);
                        let nav = Nav::build(&prims);
                        let ring = (0..nav.plates.len()).find(|&i| {
                            nav.plates[i].source >= n
                                && nav.plates[i].kind == FeatureKind::WALKWAY
                                && nav.plates[i].y0 == top - origin[1]
                        });
                        let Some(ring) = ring else {
                            panic!("{span:?}: missing platform plate");
                        };
                        let reaches_deck = (0..nav.plates.len()).any(|i| {
                            nav.plates[i].source < n
                                && nav.plates[i].kind == FeatureKind::DECK
                                && nav.connected(i, ring)
                        });
                        assert!(reaches_deck, "{span:?}: platform cut off");
                        joined += 1;
                    }
                }
            }
        }
        assert!(
            shafts > 20 && joined > 10,
            "{shafts} shafts, {joined} joined"
        );
    }
}
