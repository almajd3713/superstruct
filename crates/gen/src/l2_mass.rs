//! L2 building masses on deck surfaces. Spec: `docs/spec/levels.md` §5.1.
//!
//! Geometry is in quanta in the L2 cell's local space. A mass reads only its parent L1 cell's
//! output, so it is a pure function of the L2 key.

use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::prim::{
    CellOutput, ChildConstraint, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3,
    Shape,
};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// L2 cell edge in quanta.
pub const CELL: Q = 512 * M;
/// Margin to the cell faces, and gap between masses.
const GAP: Q = 8 * M;
const TRIES: u32 = 8;

/// The L2 masses of `cell`, in order of attempt. `parent` is the output of the parent L1 cell
/// and `avoid` the volumes (port corridors) masses keep 8 m away from.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)] // weights ≤ 3, tries < 8
pub fn masses_in(
    seed: WorldSeed,
    cell: CellKey,
    parent: &CellOutput,
    avoid: &[QAabb],
) -> Vec<Primitive> {
    debug_assert_eq!(cell.level, Level::L2);
    let c = cell.coords;
    let l0 = c.map(|v| floor_div_pow2(v, 6));
    let reg = regime(seed, l0);
    let (d, v) = (
        f32::from(reg.density) / 255.0,
        f32::from(reg.verticality) / 255.0,
    );
    let mut rng = Rng::new(hash(seed.0, Level::L2, c, salt::MASS));
    let weights = [
        lerp(3.0, 0.0, d),
        2.0,
        2.0,
        2.0,
        lerp(1.0, 3.0, d),
        lerp(0.0, 3.0, d),
        lerp(0.0, 2.0, d),
    ]
    .map(|w| w as u32);
    let attempts = rng.pick_weighted(&weights).unwrap_or(0);
    let max_h = (lerp(64.0, 256.0, v) / 4.0) as i32;

    // The cell's offset inside its L1 parent.
    let o = c.map(|v| floor_mod_pow2(v, 3) * i64::from(CELL));
    let [ox, oy, oz] = [o.x, o.y, o.z].map(|v| v as Q);
    let mut ctx = Parent::new(parent, [ox, oy, oz]);
    ctx.keepouts.extend(avoid.iter().map(|a| inflate(a, GAP)));
    let tops = ctx.deck_tops();

    let mut placed: Vec<QAabb> = Vec::new();
    for _ in 0..attempts {
        let w = rng.range_i32(4, 33) * 4 * M;
        let depth = rng.range_i32(4, 33) * 4 * M;
        let h = rng.range_i32(4, max_h + 1) * 4 * M;
        for _ in 0..TRIES {
            let pick = rng.below(tops.len().max(1) as u64) as usize;
            let x0 = GAP + 4 * M * rng.range_i32(0, (CELL - 2 * GAP - w) / (4 * M) + 1);
            let z0 = GAP + 4 * M * rng.range_i32(0, (CELL - 2 * GAP - depth) / (4 * M) + 1);
            let Some(&bottom) = tops.get(pick) else {
                continue;
            };
            let aabb = QAabb::new(
                QVec3::new(x0, bottom, z0),
                QVec3::new(x0 + w, bottom + h, z0 + depth),
            );
            if bottom + h <= CELL - GAP
                && placed.iter().all(|p| gap_ok(p, &aabb))
                && ctx.fits(&aabb)
            {
                placed.push(aabb);
                break;
            }
        }
    }
    placed
        .into_iter()
        .map(|aabb| Primitive {
            shape: Shape::Box { aabb },
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags: PrimFlags::ENVELOPE,
        })
        .collect()
}

/// The parent's data shifted into the L2 cell's frame.
pub(crate) struct Parent {
    pub(crate) keepouts: Vec<QAabb>,
    solids: Vec<QAabb>,
    /// `(x, z, radius)` of each trunk.
    trunks: Vec<(Q, Q, Q)>,
    /// Deck pieces by top height: `(top, rect)`.
    decks: Vec<(Q, [Q; 4])>,
}

pub(crate) fn inflate(a: &QAabb, by: Q) -> QAabb {
    QAabb::new(
        QVec3::new(a.min.x - by, a.min.y - by, a.min.z - by),
        QVec3::new(a.max.x + by, a.max.y + by, a.max.z + by),
    )
}

pub(crate) fn shift(bb: &QAabb, [ox, oy, oz]: [Q; 3]) -> QAabb {
    QAabb::new(
        QVec3::new(bb.min.x - ox, bb.min.y - oy, bb.min.z - oz),
        QVec3::new(bb.max.x - ox, bb.max.y - oy, bb.max.z - oz),
    )
}

impl Parent {
    pub(crate) fn new(parent: &CellOutput, o: [Q; 3]) -> Self {
        let mut p = Self {
            keepouts: Vec::new(),
            solids: Vec::new(),
            trunks: Vec::new(),
            decks: Vec::new(),
        };
        for c in &parent.child_constraints {
            // Sanctuary zones are reserved like keep-outs for everything L2 adds.
            if let ChildConstraint::KeepOut { aabb } | ChildConstraint::Zone { aabb, .. } = c {
                p.keepouts.push(shift(aabb, o));
            }
        }
        for prim in &parent.primitives {
            if matches!(prim.shape, Shape::LiftShaft { .. })
                || prim.flags.contains(PrimFlags::NO_COLLIDE)
            {
                continue;
            }
            let bb = shift(&prim.aabb(), o);
            if let Shape::Cylinder { base, radius, .. } = prim.shape {
                // Tested against the circle, not the bounding box.
                p.trunks.push((base.x - o[0], base.z - o[2], radius));
                continue;
            }
            p.solids.push(bb);
            if prim.kind == FeatureKind::DECK {
                p.decks
                    .push((bb.max.y, [bb.min.x, bb.min.z, bb.max.x, bb.max.z]));
            }
        }
        p
    }

    /// Drops the trunk circles, for bodies that are meant to touch a trunk.
    pub(crate) fn without_trunks(mut self) -> Self {
        self.trunks.clear();
        self
    }

    /// `(x, z, radius)` of each trunk.
    pub(crate) fn trunk_circles(&self) -> &[(Q, Q, Q)] {
        &self.trunks
    }

    pub(crate) fn solids(&self) -> &[QAabb] {
        &self.solids
    }

    /// Every deck top of the parent, sorted, unique.
    pub(crate) fn all_deck_tops(&self) -> Vec<Q> {
        let mut tops: Vec<Q> = self.decks.iter().map(|d| d.0).collect();
        tops.sort_unstable();
        tops.dedup();
        tops
    }

    /// Deck tops with room above them for the smallest mass and the margin.
    pub(crate) fn deck_tops(&self) -> Vec<Q> {
        let mut tops: Vec<Q> = self
            .decks
            .iter()
            .map(|d| d.0)
            .filter(|&t| t >= 0 && t + 16 * M + GAP <= CELL)
            .collect();
        tops.sort_unstable();
        tops.dedup();
        tops
    }

    /// Whether `a` is fully supported by deck at its bottom, and clear of everything else.
    pub(crate) fn fits(&self, a: &QAabb) -> bool {
        let area = i64::from(a.max.x - a.min.x) * i64::from(a.max.z - a.min.z);
        let covered: i64 = self
            .decks
            .iter()
            .filter(|d| d.0 == a.min.y)
            .map(|&(_, [x0, z0, x1, z1])| {
                let w = (a.max.x.min(x1) - a.min.x.max(x0)).max(0);
                let d = (a.max.z.min(z1) - a.min.z.max(z0)).max(0);
                i64::from(w) * i64::from(d)
            })
            .sum();
        covered == area
            && !self.keepouts.iter().any(|k| overlaps(k, a))
            && !self.solids.iter().any(|s| overlaps(s, a))
            && self.trunks.iter().all(|&(cx, cz, r)| {
                let dx = i64::from((a.min.x - cx).max(cx - a.max.x).max(0));
                let dz = i64::from((a.min.z - cz).max(cz - a.max.z).max(0));
                dx * dx + dz * dz >= i64::from(r) * i64::from(r)
            })
    }
}

/// Strict 3D overlap: touching faces do not count.
pub fn overlaps(a: &QAabb, b: &QAabb) -> bool {
    a.min.x < b.max.x
        && b.min.x < a.max.x
        && a.min.y < b.max.y
        && b.min.y < a.max.y
        && a.min.z < b.max.z
        && b.min.z < a.max.z
}

/// Whether the boxes are at least `GAP` apart along some axis.
pub(crate) fn gap_ok(a: &QAabb, b: &QAabb) -> bool {
    let sep = |a0: Q, a1: Q, b0: Q, b1: Q| (b0 - a1).max(a0 - b1);
    sep(a.min.x, a.max.x, b.min.x, b.max.x) >= GAP
        || sep(a.min.y, a.max.y, b.min.y, b.max.y) >= GAP
        || sep(a.min.z, a.max.z, b.min.z, b.max.z) >= GAP
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use glam::{I64Vec3, Vec3};

    use super::*;
    use crate::l1::l1_cell;
    use crate::l1_query::{L1Context, Volume};

    /// P0.8.2: no mass touches an L1 solid, keep-out or void, and every mass stands on deck.
    #[test]
    fn masses_sit_on_supported_clear_surfaces() {
        let seed = WorldSeed(0x0802);
        let (mut total, mut parents) = (0, 0);
        for x in -12..12 {
            for y in [-4, 0, 2] {
                for z in [-5, 3] {
                    let l1 = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                    let out = l1_cell(seed, l1);
                    if out.primitives.is_empty() {
                        continue;
                    }
                    parents += 1;
                    let ctx = L1Context::from_primitives(out.primitives.clone());
                    for k in 0..512i64 {
                        let o = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
                        let cell = CellKey::new(Level::L2, l1.coords * 8 + o);
                        let ms = masses_in(seed, cell, &out, &[]);
                        total += ms.len();
                        let origin = Vec3::new(o.x as f32, o.y as f32, o.z as f32) * 512.0;
                        check(&ctx, &out, &ms, origin, CELL, cell);
                    }
                }
            }
        }
        assert!(
            parents > 100 && total > 200,
            "{parents} parents, {total} masses"
        );
    }

    #[allow(clippy::cast_possible_truncation)] // test origins are whole metres below 2²⁰
    fn check(
        ctx: &L1Context,
        out: &CellOutput,
        ms: &[Primitive],
        origin: Vec3,
        cell_q: Q,
        cell: CellKey,
    ) {
        let oq = [origin.x, origin.y, origin.z].map(|m| Q::try_from(m as i64 * 16).unwrap());
        let boxes: Vec<QAabb> = ms.iter().map(Primitive::aabb).collect();
        for (i, bb) in boxes.iter().enumerate() {
            assert!(
                bb.min.x >= GAP && bb.max.x <= cell_q - GAP,
                "{cell:?} margin"
            );
            assert!(
                bb.min.z >= GAP && bb.max.z <= cell_q - GAP,
                "{cell:?} margin"
            );
            assert!(bb.max.y <= cell_q - GAP, "{cell:?} margin");
            assert!(boxes[..i].iter().all(|o| gap_ok(o, bb)), "{cell:?} gap");
            let (lo, hi) = (bb.min_m(), bb.max_m());
            let at = |u: f32, v: f32, w: f32| lo + (hi - lo) * Vec3::new(u, v, w);
            // Interior points are open air; just under the base is deck.
            for u in [0.01, 0.25, 0.5, 0.75, 0.99] {
                for v in [0.01, 0.5, 0.99] {
                    for w in [0.01, 0.25, 0.5, 0.75, 0.99] {
                        let got = ctx.classify(origin + at(u, v, w));
                        assert!(
                            matches!(got, Volume::Void | Volume::DeckSurface),
                            "{cell:?} {got:?}"
                        );
                    }
                }
                for w in [0.01, 0.5, 0.99] {
                    let mut p = at(u, 0.0, w);
                    p.y -= 0.25;
                    assert_eq!(
                        ctx.classify(origin + p),
                        Volume::Structure,
                        "{cell:?} unsupported"
                    );
                }
            }
            for c in &out.child_constraints {
                if let ChildConstraint::KeepOut { aabb } = c {
                    let shifted = shift(aabb, oq);
                    assert!(!overlaps(&shifted, bb), "{cell:?} in a KeepOut");
                }
            }
        }
    }
}
