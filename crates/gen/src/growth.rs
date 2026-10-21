//! Overgrowth (GROW-01..05, `docs/proposals/the-web-ideas.md` §13.6): moss on north faces,
//! hanging roots, pale glowing fungus, vines in hydroponic quarters and a tree of pipes that is
//! a tree. Spec: `docs/spec/levels.md` §5W.39.
//!
//! Like the exterior kit it is visual relief on the box parts of structures (`NO_COLLIDE`, in the
//! lattice's frame), and it is more where the air is damp: in a stratum of the Hydroponic and
//! Waterworks quarters and near voids.

#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::features::{Content, M};
use crate::hash::{hash, mix};
use crate::kit_ext::{M as Pieces, faces};
use crate::orient::{Frame, LocalShape, heading};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue::Tissue;

pub const TAGS: [&str; 4] = ["GROW-01", "GROW-02", "GROW-03", "GROW-04"];

/// The growth on the structures of `t`, with the index into `TAGS` of each piece.
pub fn dress(t: &Tissue) -> Vec<(usize, Primitive)> {
    let Some(chr) = t.character else {
        return Vec::new();
    };
    let l1 = t.cell.coords.div_euclid(I64Vec3::splat(8));
    // Damp quarters grow more: hydroponic (5) and waterworks (10) take twice the chance.
    let quarter = crate::quarters::quarter_of(t.seed, l1, &chr);
    let damp = matches!(quarter, 5 | 10);
    let pitch = t.lattice.pitch as f64;
    let base = hash(t.seed.0, Level::L2, t.cell.coords, salt::GROWTH);
    let mut m = Pieces::new(t);
    for (si, s) in t.structures.iter().enumerate() {
        for (pi, b) in s.parts.iter().enumerate() {
            let mut rng = Rng::new(mix(base ^ ((si as u64) << 32 | pi as u64)));
            let lo = [0, 1, 2].map(|k| f64::from(b.lo[k]) * pitch);
            let hi = [0, 1, 2].map(|k| f64::from(b.hi[k]) * pitch);
            // The draws are made for every part, whatever it takes.
            let moss = rng.below(if damp { 2 } else { 4 }) == 0;
            let roots = rng.below(if damp { 3 } else { 6 }) == 0;
            let fungus = rng.below(if damp { 3 } else { 8 }) == 0;
            let vines = damp && rng.below(2) == 0;
            let seeds = [
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64(),
            ];
            if hi[1] - lo[1] < 8.0 {
                continue;
            }
            let fs = faces(lo, hi);
            if moss {
                // GROW-01: patches on the north face (−z): grids of 2 m slabs, in clumps.
                let f = &fs[2];
                let mut r = Rng::new(seeds[0]);
                let (mut n, mut y) = (0, f.y.0 + 1.0);
                while y < f.y.1 - 2.0 && n < 24 {
                    let mut x = f.t.0 + 1.0;
                    while x < f.t.1 - 3.0 && n < 24 {
                        if r.below(3) != 0 {
                            let (a, bb) = f.bx(0.0, 0.25, x, x + 2.0, y, y + 1.5);
                            m.boxed(0, a, bb, MaterialId::MOSS, FeatureKind::MISC);
                            n += 1;
                        }
                        x += 2.0;
                    }
                    y += 2.0;
                }
            }
            if roots {
                // GROW-02: roots hanging from the underside, 3 to 12 m.
                let mut r = Rng::new(seeds[1]);
                for _ in 0..6 {
                    let x = lo[0] + 1.0 + r.next_f32() as f64 * (hi[0] - lo[0] - 2.0).max(0.0);
                    let z = lo[2] + 1.0 + r.next_f32() as f64 * (hi[2] - lo[2] - 2.0).max(0.0);
                    let len = 3.0 + r.below(10) as f64;
                    m.boxed(
                        1,
                        [x, lo[1] - len, z],
                        [x + 0.2, lo[1], z + 0.2],
                        MaterialId::MOSS,
                        FeatureKind::CABLE,
                    );
                }
            }
            if fungus {
                // GROW-03: pale fungus along the foot of a face: clusters of caps that glow.
                let mut r = Rng::new(seeds[2]);
                let f = &fs[r.below(4) as usize];
                for _ in 0..5 {
                    let x = f.t.0 + 1.0 + r.next_f32() as f64 * (f.width() - 2.0).max(0.0);
                    let (a, bb) = f.bx(0.0, 0.5, x, x + 0.6, f.y.0, f.y.0 + 0.4);
                    m.boxed(2, a, bb, MaterialId::FUNGUS, FeatureKind::MISC);
                }
            }
            if vines {
                // GROW-04: vines hanging in strips of 3 to 6 m from the top edge of a face.
                let mut r = Rng::new(seeds[3]);
                let f = &fs[r.below(4) as usize];
                let mut x = f.t.0 + 2.0;
                while x < f.t.1 - 2.0 {
                    let len = 3.0 + r.below(4) as f64;
                    let (a, bb) = f.bx(0.0, 0.3, x, x + 0.3, f.y.1 - len, f.y.1);
                    m.boxed(3, a, bb, MaterialId::MOSS, FeatureKind::CABLE);
                    x += 2.5;
                }
            }
        }
    }
    let Pieces { em, tags } = m;
    let cell_q = 512 * Q_PER_M;
    let keep_out: Vec<QAabb> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .map(Primitive::aabb)
        .collect();
    tags.into_iter()
        .zip(em.out)
        .filter(|(_, p)| {
            let b = p.aabb();
            b.max.x > 0
                && b.min.x < cell_q
                && b.max.y > 0
                && b.min.y < cell_q
                && b.max.z > 0
                && b.min.z < cell_q
                && !keep_out.iter().any(|k| {
                    b.min.x < k.max.x + 8
                        && b.max.x > k.min.x - 8
                        && b.min.y < k.max.y + 8
                        && b.max.y > k.min.y - 8
                        && b.min.z < k.max.z + 8
                        && b.max.z > k.min.z - 8
                })
        })
        .collect()
}

/// A box of pipe `size` metres from `at`, in the table frame `(yaw, tilt)`.
fn turned(
    out: &mut Vec<Primitive>,
    at: [i32; 3],
    size: [i32; 3],
    yaw: u8,
    tilt: u8,
    material: MaterialId,
) {
    let q = |v: i32| v * M;
    let clip = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(65536, 65536, 65536));
    out.push(Primitive {
        shape: Shape::oriented(
            Frame::table(yaw, tilt),
            QVec3::new(q(at[0]), q(at[1]), q(at[2])),
            LocalShape::Box {
                aabb: QAabb::new(
                    QVec3::new(0, 0, 0),
                    QVec3::new(q(size[0]), q(size[1]), q(size[2])),
                ),
            },
            clip,
        ),
        material,
        kind: FeatureKind::PIPE,
        flags: PrimFlags::NONE,
    });
}

/// An L1 cell holds a tree of pipes with chance 1 in this (GROW-05).
const TREE_ONE_IN: u64 = 40;

/// Whether L1 cell `l1` holds the tree.
pub fn has_tree(seed: WorldSeed, l1: I64Vec3) -> bool {
    Rng::new(hash(seed.0, Level::L1, l1, salt::GROWTH)).below(TREE_ONE_IN) == 0
}

/// GROW-05: a tree of pipes that is a tree: a trunk of 12 m, three tiers of boughs (turned
/// beams of pipe that thin and rise) and crowns of moss, on a base of roots.
pub fn tree_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    if !has_tree(seed, l1) {
        return Content::default();
    }
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::GROWTH));
    rng.below(TREE_ONE_IN);
    let c = 2048;
    let mut out: Vec<Primitive> = Vec::new();
    let base = 1500;
    let q = |v: i32| v * M;
    // The trunk, and roots that spread from its foot.
    out.push(Primitive {
        shape: Shape::cylinder(
            crate::cell::Axis::Y,
            QVec3::new(q(c), q(base), q(c)),
            q(160),
            q(6),
        ),
        material: MaterialId::PIPE,
        kind: FeatureKind::PIPE,
        flags: PrimFlags::FAR_VISIBLE,
    });
    for k in 0..8u8 {
        turned(
            &mut out,
            [c, base, c],
            [26, 3, 3],
            k * 8,
            0,
            MaterialId::PIPE,
        );
    }
    // Three tiers of six boughs, rising 15° to 30° and thinning.
    for tier in 0..3i32 {
        let y = base + 70 + tier * 30;
        let len = 60 - tier * 14;
        let thick = 4 - tier;
        for k in 0..6u8 {
            let yaw = k * 10 + (tier as u8) * 3 + (rng.below(4) as u8);
            turned(
                &mut out,
                [c, y, c],
                [len, thick, thick],
                yaw,
                3 + tier as u8,
                MaterialId::PIPE,
            );
            // The crown at the end of the bough.
            let (a, b) = heading(yaw);
            let norm = (f64::from(a) * f64::from(a) + f64::from(b) * f64::from(b)).sqrt();
            let (ex, ez) = (
                (f64::from(len) * f64::from(a) / norm).round() as i32,
                (f64::from(len) * f64::from(b) / norm).round() as i32,
            );
            let top = y + (f64::from(len) * 0.35) as i32;
            out.push(Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(
                        QVec3::new(q(c + ex - 8), q(top), q(c + ez - 8)),
                        QVec3::new(q(c + ex + 8), q(top + 6), q(c + ez + 8)),
                    ),
                },
                material: MaterialId::MOSS,
                kind: FeatureKind::MISC,
                flags: PrimFlags::NO_COLLIDE,
            });
        }
    }
    Content {
        prims: out,
        keep_out: vec![QAabb::new(
            QVec3::new(q(c - 90), q(base - 4), q(c - 90)),
            QVec3::new(q(c + 90), q(base + 260), q(c + 90)),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellKey;
    use crate::tissue::TissueCache;

    /// GROW-01..04 all occur on real structures, harmlessly, and moss is on north faces only.
    #[test]
    fn every_growth_occurs_and_is_harmless() {
        let seed = WorldSeed(8);
        let cache = TissueCache::default();
        let mut seen = [0usize; 4];
        for k in 0..400i64 {
            let key = CellKey::new(Level::L2, I64Vec3::new(k % 8 - 4, k / 8 % 3, k / 24 - 8));
            let t = crate::tissue::tissue_cell(seed, &cache, key);
            for (tag, p) in dress(&t) {
                seen[tag] += 1;
                assert!(p.flags.contains(PrimFlags::NO_COLLIDE));
            }
        }
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    }

    /// GROW-05: a tree in about one cell in forty.
    #[test]
    fn a_tree_of_pipes_stands_in_some_cells() {
        let seed = WorldSeed(8);
        let mut trees = 0;
        for x in -20..20 {
            for z in -20..20 {
                let l1 = I64Vec3::new(x, 1, z);
                if has_tree(seed, l1) {
                    trees += 1;
                    let c = tree_content(seed, l1);
                    assert!(c.prims.len() > 30);
                    assert!(c.prims.iter().any(|p| p.material == MaterialId::MOSS));
                    assert!(!c.keep_out.is_empty());
                }
            }
        }
        assert!((15..80).contains(&trees), "{trees} of 1600");
    }
}
