//! L2 vistas `[VISTA]`. Spec: `docs/spec/levels.md` §5.6.
//!
//! A vista is a short grating balcony on the deck at the edge of an L1 shaft or chasm, with a
//! clear corridor out over the void. All geometry is in the L2 cell's local quanta.

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::key::ColumnSpanKey;
use crate::l1::{CELL as L1_CELL, Rect, chasm, shaft};
use crate::l2_mass::{Parent, overlaps};
use crate::prim::{
    CellOutput, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape,
};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
const CELL: Q = 512 * M;
/// Keeps the vista clear of the 64 m port corridors along the cell faces, plus 8 m.
const RIM: Q = 72 * M;
const WIDTH: Q = 16 * M;
/// Balcony length over the void, and the deck strip behind it.
const BALCONY: Q = 8 * M;
const CORRIDOR_MAX: Q = 64 * M;
const CORRIDOR_MIN: Q = 16 * M;
const CORRIDOR_HEIGHT: Q = 8 * M;
const SLAB: Q = 8;
const CHANCE: f32 = 0.25;

/// A vista in the L2 cell: the balcony and the volumes around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vista {
    pub balcony: Primitive,
    /// Clear volume out over the void; children keep it open.
    pub corridor: QAabb,
    /// Balcony and corridor together.
    pub zone: QAabb,
}

/// The vista of `cell`, if the draw says so and a spot fits. `origin` is the cell's offset in
/// its L1 parent; `avoid` lists volumes (cell-local) the vista must not meet.
pub fn vista(
    seed: WorldSeed,
    cell: CellKey,
    parent_key: CellKey,
    parent: &CellOutput,
    origin: [Q; 3],
    avoid: &[QAabb],
) -> Option<Vista> {
    debug_assert_eq!(cell.level, Level::L2);
    let mut rng = Rng::new(hash(seed.0, Level::L2, cell.coords, salt::VISTA));
    let present = rng.chance(CHANCE);
    let start = rng.below(4);
    let offsets: Vec<Q> = (0..8)
        .map(|_| Q::try_from(rng.below(1 << 16)).unwrap_or(0))
        .collect();
    if !present {
        return None;
    }

    let voids: Vec<Rect> = shaft(seed, ColumnSpanKey::of(parent_key))
        .into_iter()
        .chain(chasm(seed, parent_key))
        .collect();
    let mut ctx = Parent::new(parent, [0, 0, 0]);
    // The voids themselves are free for the balcony and corridor.
    ctx.keepouts.retain(|k| {
        !(k.min.y == 0
            && k.max.y == L1_CELL
            && voids
                .iter()
                .any(|v| (k.min.x, k.min.z, k.max.x, k.max.z) == (v.x0, v.z0, v.x1, v.z1)))
    });
    let mut tops = ctx.all_deck_tops();
    // The slab hangs 0.5 m below the deck top, so the top must be that far inside the cell.
    tops.retain(|&t| t >= origin[1] + SLAB && t + CORRIDOR_HEIGHT + 8 * M <= origin[1] + CELL);

    let inner = Rect {
        x0: origin[0] + RIM,
        z0: origin[2] + RIM,
        x1: origin[0] + CELL - RIM,
        z1: origin[2] + CELL - RIM,
    };
    let shift = |a: &QAabb| {
        QAabb::new(
            QVec3::new(
                a.min.x - origin[0],
                a.min.y - origin[1],
                a.min.z - origin[2],
            ),
            QVec3::new(
                a.max.x - origin[0],
                a.max.y - origin[1],
                a.max.z - origin[2],
            ),
        )
    };
    for (i, (void, top)) in voids
        .iter()
        .flat_map(|v| tops.iter().map(move |t| (v, *t)))
        .enumerate()
    {
        for k in 0..4u64 {
            let side = (start + k) % 4;
            let u = offsets[(i + usize::try_from(k).unwrap_or(0)) % offsets.len()];
            let Some(v) = place(void, &inner, side, u, top) else {
                continue;
            };
            let (strip, balcony, corridor) = (v.0, v.1, v.2);
            let free = |a: &QAabb| {
                !ctx.keepouts.iter().any(|k| overlaps(k, a))
                    && !ctx.solids().iter().any(|s| overlaps(s, a))
                    && !avoid.iter().any(|b| overlaps(&shift(a), b))
            };
            if !(ctx.fits(&strip) && free(&balcony) && free(&corridor)) {
                continue;
            }
            let slab = QAabb::new(
                QVec3::new(balcony.min.x, top - SLAB, balcony.min.z),
                QVec3::new(balcony.max.x, top, balcony.max.z),
            );
            let zone = QAabb::new(
                QVec3::new(
                    balcony.min.x.min(corridor.min.x),
                    top - SLAB,
                    balcony.min.z.min(corridor.min.z),
                ),
                QVec3::new(
                    balcony.max.x.max(corridor.max.x),
                    corridor.max.y,
                    balcony.max.z.max(corridor.max.z),
                ),
            );
            return Some(Vista {
                balcony: Primitive {
                    shape: Shape::Box { aabb: shift(&slab) },
                    material: MaterialId::GRATING,
                    kind: FeatureKind::WALKWAY,
                    flags: PrimFlags::NONE,
                },
                corridor: shift(&corridor),
                zone: shift(&zone),
            });
        }
    }
    None
}

/// The deck strip, balcony and corridor on one side of `void`, in the parent's frame. Side 0
/// is the void's -x edge (the balcony points +x), 1 its +x edge, 2 -z, 3 +z.
fn place(void: &Rect, inner: &Rect, side: u64, u: Q, top: Q) -> Option<(QAabb, QAabb, QAabb)> {
    let along_x = side >= 2;
    // Edge position, along-edge range, and the unit direction into the void.
    let (edge, dir, lo, hi, n_lo, n_hi) = if along_x {
        (
            if side == 2 { void.z0 } else { void.z1 },
            if side == 2 { 1 } else { -1 },
            void.x0.max(inner.x0),
            void.x1.min(inner.x1),
            inner.z0,
            inner.z1,
        )
    } else {
        (
            if side == 0 { void.x0 } else { void.x1 },
            if side == 0 { 1 } else { -1 },
            void.z0.max(inner.z0),
            void.z1.min(inner.z1),
            inner.x0,
            inner.x1,
        )
    };
    let span = hi - WIDTH - lo;
    if span < 0 {
        return None;
    }
    let t0 = lo + 4 * M * (u % (span / (4 * M) + 1));
    // Distance from the edge to the far wall of the void, along the direction.
    let depth = if along_x {
        void.z1 - void.z0
    } else {
        void.x1 - void.x0
    };
    let corridor_len = (depth - BALCONY).min(CORRIDOR_MAX);
    if corridor_len < CORRIDOR_MIN {
        return None;
    }
    // Normal-direction extent: strip behind the edge, balcony and corridor in front.
    let back = edge - dir * BALCONY;
    let front = edge + dir * (BALCONY + corridor_len);
    let (n0, n1) = (back.min(front), back.max(front));
    if n0 < n_lo || n1 > n_hi {
        return None;
    }
    let boxed = |a: Q, b: Q, y0: Q, y1: Q| {
        let (a, b) = (a.min(b), a.max(b));
        if along_x {
            QAabb::new(QVec3::new(t0, y0, a), QVec3::new(t0 + WIDTH, y1, b))
        } else {
            QAabb::new(QVec3::new(a, y0, t0), QVec3::new(b, y1, t0 + WIDTH))
        }
    };
    let strip = boxed(back, edge, top, top + 4 * M);
    let balcony = boxed(edge, edge + dir * BALCONY, top - SLAB, top + 4 * M);
    let corridor = boxed(
        edge + dir * BALCONY,
        edge + dir * (BALCONY + corridor_len),
        top,
        top + CORRIDOR_HEIGHT,
    );
    Some((strip, balcony, corridor))
}

#[cfg(test)]
mod tests {
    use glam::{I64Vec3, Vec3};

    use super::*;
    use crate::cell::floor_mod_pow2;
    use crate::l1_query::{L1Context, Volume};
    use crate::l2::l2_cell_in;
    use crate::l2_port::L1Cache;
    use crate::prim::{ChildConstraint, MetaFlags, ZoneKind};

    /// P0.8.7: vista frequency among eligible cells is at most the spec's 0.25, and each
    /// vista's balcony stands on deck at the edge of a void with its corridor over that void.
    #[test]
    #[allow(
        clippy::too_many_lines,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss
    )] // one scan with several checks per vista
    fn vistas_face_voids() {
        let seed = WorldSeed(0x0807);
        let (mut eligible, mut vistas) = (0u32, 0u32);
        for x in -40..40 {
            for z in [-2, 3] {
                let parent_key = CellKey::new(Level::L1, I64Vec3::new(x, 0, z));
                let span = ColumnSpanKey::of(parent_key);
                let voids: Vec<Rect> = shaft(seed, span)
                    .into_iter()
                    .chain(chasm(seed, parent_key))
                    .collect();
                if voids.is_empty() {
                    continue;
                }
                let cache = L1Cache::default();
                let parent = cache.get(seed, parent_key);
                let ctx = L1Context::from_primitives(parent.primitives.clone());
                let tops = Parent::new(&parent, [0, 0, 0]).all_deck_tops();
                for k in 0..512i64 {
                    let idx = [k % 8, k / 64, k / 8 % 8];
                    let cell = CellKey::new(
                        Level::L2,
                        parent_key.coords * 8 + I64Vec3::new(idx[0], idx[1], idx[2]),
                    );
                    let oy = Q::try_from(idx[1]).unwrap() * CELL;
                    if !tops.iter().any(|&t| t >= oy && t + 256 <= oy + CELL) {
                        continue;
                    }
                    // A void within 64 m of the cell's deck.
                    let (ox, oz) = (
                        Q::try_from(idx[0]).unwrap() * CELL,
                        Q::try_from(idx[2]).unwrap() * CELL,
                    );
                    let near = |v: &Rect| {
                        v.x0 - 64 * M < ox + CELL
                            && ox < v.x1 + 64 * M
                            && v.z0 - 64 * M < oz + CELL
                            && oz < v.z1 + 64 * M
                    };
                    if !voids.iter().any(near) {
                        continue;
                    }
                    let out = l2_cell_in(seed, &cache, cell);
                    eligible += 1;
                    if !out.meta.flags.contains(MetaFlags::VISTA) {
                        continue;
                    }
                    vistas += 1;
                    let origin = [idx[0], idx[1], idx[2]].map(|i| Q::try_from(i).unwrap() * CELL);
                    let zone = out
                        .child_constraints
                        .iter()
                        .find_map(|c| match c {
                            ChildConstraint::Zone {
                                aabb,
                                kind: ZoneKind::Vista,
                            } => Some(*aabb),
                            _ => None,
                        })
                        .unwrap();
                    let balcony = out
                        .primitives
                        .iter()
                        .find(|p| p.kind == FeatureKind::WALKWAY && overlaps(&p.aabb(), &zone))
                        .unwrap()
                        .aabb();
                    // The corridor lies over a void.
                    let corridor = out
                        .child_constraints
                        .iter()
                        .filter_map(|c| match c {
                            ChildConstraint::KeepOut { aabb } => Some(*aabb),
                            _ => None,
                        })
                        .find(|a| overlaps(a, &zone) && a.max.y - a.min.y == CORRIDOR_HEIGHT)
                        .unwrap();
                    let mid = |a: &QAabb| {
                        Vec3::new(
                            (a.min.x + a.max.x) as f32 / 32.0,
                            a.min.y as f32 / 16.0,
                            (a.min.z + a.max.z) as f32 / 32.0,
                        )
                    };
                    let to_l1 = |p: Vec3| {
                        p + Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32) / 16.0
                    };
                    let over = to_l1(mid(&corridor));
                    assert!(
                        voids.iter().any(|v| {
                            let (px, pz) = ((over.x * 16.0) as Q, (over.z * 16.0) as Q);
                            v.x0 <= px && px < v.x1 && v.z0 <= pz && pz < v.z1
                        }),
                        "{cell:?}: corridor not over a void"
                    );
                    // Just under the balcony's midpoint: deck on the strip side only, so the
                    // slab hangs over open air; and deck is at its top height.
                    let under = to_l1(mid(&balcony)) - Vec3::new(0.0, 0.5, 0.0);
                    assert!(
                        matches!(ctx.classify(under), Volume::Void),
                        "{cell:?}: balcony not over a void"
                    );
                    let top = balcony.max.y;
                    assert!(
                        tops.contains(&(top + origin[1])),
                        "{cell:?}: balcony {balcony:?} zone {zone:?} tops {tops:?} origin {origin:?}"
                    );
                    let _ = floor_mod_pow2;
                }
            }
        }
        let rate = f64::from(vistas) / f64::from(eligible);
        assert!(
            vistas >= 20 && (0.03..=0.25).contains(&rate),
            "{vistas} of {eligible}"
        );
    }
}
