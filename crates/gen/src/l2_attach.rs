//! Masses attached to trunks. Spec: `docs/spec/levels.md` §5.1 and `docs/spec/ports.md` Lemma D.
//!
//! A mass hangs off a trunk's side, its floor 4–28 m above a deck, with an entrance skywalk on
//! its outer face: a grating landing and a stair down to the deck. The skywalk's corridor must
//! fit, so the mass is reachable from the network by construction.

use crate::cell::Axis;
use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l2_mass::{Parent, gap_ok, overlaps};
use crate::l2_walk::Skywalk;
use crate::prim::{
    CellOutput, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape,
};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
const CELL: Q = 512 * M;
/// Keeps the mass clear of the 64 m port corridors along the cell faces, plus 8 m.
const RIM: Q = 72 * M;
const GAP: Q = 8 * M;
/// How far the mass reaches into the trunk's concrete.
const EMBED: Q = 4 * M;
const TRIES: u32 = 8;
/// Entrance landing width.
const ENTRANCE: Q = 8 * M;

/// A trunk-attached mass and its entrance skywalk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attached {
    pub mass: Primitive,
    pub walk: Skywalk,
}

fn isqrt(n: i64) -> i64 {
    let mut x = 0i64;
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// The attached masses of `cell`. `origin` is its offset in the parent; `avoid` and `placed`
/// are cell-local volumes to keep clear of (corridors and zones) and the other masses.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
pub fn attached_masses(
    seed: WorldSeed,
    cell: CellKey,
    parent: &CellOutput,
    origin: [Q; 3],
    avoid: &[QAabb],
    placed: &[QAabb],
) -> Vec<Attached> {
    debug_assert_eq!(cell.level, Level::L2);
    let full = Parent::new(parent, origin);
    let trunks: Vec<(Q, Q, Q)> = full.trunk_circles().to_vec();
    let mut rng = Rng::new(hash(seed.0, Level::L2, cell.coords, salt::TRUNK_MASS));
    let l0 = cell.coords.map(|v| crate::cell::floor_div_pow2(v, 6));
    let v = f32::from(regime(seed, l0).verticality) / 255.0;
    let max_h = ((64.0 + (256.0 - 64.0) * v) / 4.0) as i32;
    let attempts = rng.below(4);
    let tops = full.deck_tops();
    let body = Parent::new(parent, origin).without_trunks();

    let mut out: Vec<Attached> = Vec::new();
    let mut taken: Vec<QAabb> = placed.to_vec();
    let mut blocked: Vec<QAabb> = avoid.to_vec();
    for _ in 0..attempts {
        let dir = rng.below(4);
        let w = 4 * M * rng.range_i32(4, 17);
        let depth = 4 * M * rng.range_i32(4, 17);
        let h = 4 * M * rng.range_i32(4, max_h + 1);
        for _ in 0..TRIES {
            let pick_trunk = rng.below(trunks.len().max(1) as u64) as usize;
            let pick_top = rng.below(tops.len().max(1) as u64) as usize;
            let k = rng.range_i32(1, 8);
            let tau = 4 * M * rng.range_i32(-8, 9);
            let (Some(&(cx, cz, r)), Some(&deck)) = (trunks.get(pick_trunk), tops.get(pick_top))
            else {
                continue;
            };
            let along_x = dir < 2;
            let sign: Q = if dir.is_multiple_of(2) { 1 } else { -1 };
            let (centre, tc) = if along_x {
                (cx, cz + tau)
            } else {
                (cz, cx + tau)
            };
            let reach = tau.abs() + depth / 2;
            if i64::from(reach) * 100 > i64::from(r) * 85 {
                continue;
            }
            // Nearest the trunk's axis the surface is at `chord`; stay inside it by EMBED.
            let chord =
                isqrt(i64::from(r) * i64::from(r) - i64::from(reach) * i64::from(reach)) as Q;
            let near = (chord - EMBED) / (4 * M) * (4 * M);
            let (n0, n1) = if sign > 0 {
                (centre + near, centre + near + w)
            } else {
                (centre - near - w, centre - near)
            };
            let floor = deck + 4 * M * k;
            let (t0, t1) = (tc - depth / 2, tc + depth / 2);
            let aabb = if along_x {
                QAabb::new(QVec3::new(n0, floor, t0), QVec3::new(n1, floor + h, t1))
            } else {
                QAabb::new(QVec3::new(t0, floor, n0), QVec3::new(t1, floor + h, n1))
            };
            let inside = |a: Q, b: Q| a >= RIM && b <= CELL - RIM;
            if !(inside(aabb.min.x, aabb.max.x)
                && inside(aabb.min.z, aabb.max.z)
                && aabb.max.y <= CELL - GAP)
            {
                continue;
            }
            let walk = Skywalk {
                axis: if along_x { Axis::X } else { Axis::Z },
                plane: if sign > 0 { n1 } else { n0 },
                inward: sign,
                u0: tc - ENTRANCE / 2,
                u1: tc + ENTRANCE / 2,
                floor,
                height: 4 * M * k,
            };
            let corridor = walk.corridor();
            let clear = |a: &QAabb| {
                !body.keepouts.iter().any(|b| overlaps(b, a))
                    && !body.solids().iter().any(|b| overlaps(b, a))
                    && !blocked.iter().any(|b| overlaps(b, a))
                    && taken.iter().all(|b| gap_ok(b, a))
            };
            if clear(&aabb)
                && full.fits(&corridor)
                && !blocked.iter().any(|b| overlaps(b, &corridor))
                && !taken.iter().any(|b| overlaps(b, &corridor))
            {
                taken.push(aabb);
                blocked.push(corridor);
                out.push(Attached {
                    mass: Primitive {
                        shape: Shape::Box { aabb },
                        material: MaterialId::CONCRETE,
                        kind: FeatureKind::MASS,
                        flags: PrimFlags::ENVELOPE,
                    },
                    walk,
                });
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::cell::floor_mod_pow2;
    use crate::l2::l2_cell_in;
    use crate::l2_port::L1Cache;
    use crate::walk::{Nav, cell_window};

    /// P0.8.9: every trunk-attached mass touches its trunk and has an entrance landing that
    /// the validator links to the L1 network, over 10⁴ cells.
    #[test]
    #[ignore = "reads the v6 L1 (lift core, decks, bridges), replaced in version 7; rebuilt or deleted with PW.3"]
    fn attached_masses_are_reachable() {
        let seed = WorldSeed(0x0809);
        let (mut cells, mut attached) = (0u32, 0u32);
        'scan: for x in -600..600 {
            let parent_key = CellKey::new(Level::L1, I64Vec3::new(x, x % 4, 2 * x % 7));
            let cache = L1Cache::default();
            let parent = cache.get(seed, parent_key);
            if !parent
                .primitives
                .iter()
                .any(|p| p.kind == FeatureKind::TRUNK)
            {
                continue;
            }
            for k in 0..512i64 {
                let idx = [k % 8, k / 64, k / 8 % 8];
                let cell = CellKey::new(
                    Level::L2,
                    parent_key.coords * 8 + I64Vec3::new(idx[0], idx[1], idx[2]),
                );
                cells += 1;
                let origin = idx.map(|i| Q::try_from(i).unwrap() * CELL);
                let out = l2_cell_in(seed, &cache, cell);
                let mut prims = cell_window(&parent.primitives, origin, CELL);
                let n = prims.len();
                prims.extend(out.primitives.iter().copied());
                let ctx = Parent::new(&parent, origin);
                let tops = ctx.all_deck_tops();
                let mut nav = None;
                for m in out
                    .primitives
                    .iter()
                    .filter(|p| p.kind == FeatureKind::MASS)
                {
                    let bb = m.aabb();
                    if tops.contains(&bb.min.y) {
                        continue;
                    }
                    attached += 1;
                    // It reaches into a trunk.
                    assert!(
                        ctx.trunk_circles().iter().any(|&(cx, cz, r)| {
                            let dx = i64::from((bb.min.x - cx).max(cx - bb.max.x).max(0));
                            let dz = i64::from((bb.min.z - cz).max(cz - bb.max.z).max(0));
                            dx * dx + dz * dz < i64::from(r) * i64::from(r)
                        }),
                        "{cell:?}: mass {bb:?} touches no trunk"
                    );
                    let slab = out.primitives.iter().position(|p| {
                        let s = p.aabb();
                        p.kind == FeatureKind::WALKWAY
                            && s.max.y == bb.min.y
                            && s.min.x <= bb.max.x
                            && bb.min.x <= s.max.x
                            && s.min.z <= bb.max.z
                            && bb.min.z <= s.max.z
                    });
                    let slab = slab.unwrap_or_else(|| panic!("{cell:?}: mass without a landing"));
                    let nav = nav.get_or_insert_with(|| Nav::build(&prims));
                    let landing: Vec<usize> = (0..nav.plates.len())
                        .filter(|&i| nav.plates[i].source == n + slab)
                        .collect();
                    assert!(!landing.is_empty(), "{cell:?}: landing is not walkable");
                    let reaches = landing.iter().any(|&i| {
                        (0..nav.plates.len())
                            .any(|j| nav.plates[j].source < n && nav.connected(i, j))
                    });
                    assert!(reaches, "{cell:?}: landing of {bb:?} is cut off");
                }
                let _ = floor_mod_pow2;
                if cells >= 10_000 && attached >= 50 {
                    break 'scan;
                }
            }
        }
        assert!(
            cells >= 10_000 && attached >= 50,
            "{cells} cells, {attached} attached"
        );
    }
}
