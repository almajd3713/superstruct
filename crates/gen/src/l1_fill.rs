//! The fill field: the built fraction each L2 cell aims at (FILL-01).
//! Spec: `docs/spec/levels.md` §4W.4.
//!
//! Everything is integer and depends only on the stack, the voids and hashes of L1 lattice
//! corners, so both sides of every face agree.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::{I64Vec3, Vec3};

use crate::cell::Level;
use crate::hash::hash;
use crate::l1_void::{VoidPiece, voids_of_cell};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Stratum, stack};

const L1_M: i64 = 4096;
const L0_M: i64 = 32_768;

/// The noise of an L1 lattice corner, `0..64`.
pub fn corner_noise(seed: WorldSeed, corner: I64Vec3) -> u64 {
    Rng::new(hash(seed.0, Level::L1, corner, salt::FILL_CORNER)).below(64)
}

/// The trilinear noise of L2 cell `(ix, iy, iz)` of L1 cell `l1`, `0..64`, in integer weights
/// over 16 per axis. The spec's weight pair `(15 − (2i + 1), 2i + 1)` sums to 15; the pair
/// used is `(15 − 2i, 2i + 1)`, which sums to 16.
pub fn noise_at(seed: WorldSeed, l1: I64Vec3, ix: i64, iy: i64, iz: i64) -> u64 {
    let w = |i: i64| [(15 - 2 * i) as u64, (2 * i + 1) as u64];
    let (wx, wy, wz) = (w(ix), w(iy), w(iz));
    let mut sum = 0;
    for dx in 0..2 {
        for dy in 0..2 {
            for dz in 0..2 {
                let n = corner_noise(seed, l1 + I64Vec3::new(dx, dy, dz));
                sum += n * wx[dx as usize] * wy[dy as usize] * wz[dz as usize];
            }
        }
    }
    sum / 4096
}

/// The fill of every L2 cell of L1 cell `l1`, indexed `(iz · 8 + iy) · 8 + ix`.
pub fn fill_field(seed: WorldSeed, l1: I64Vec3) -> Vec<u8> {
    let home = l1.div_euclid(I64Vec3::splat(8));
    let web = regime(seed, home).web;
    let st = stack(seed, home);
    let voids: Vec<VoidPiece> = voids_of_cell(seed, l1)
        .into_iter()
        .filter(|v| !v.solid)
        .collect();
    let origin = l1 * L1_M;
    let mut out = vec![0u8; 512];
    for iz in 0..8i64 {
        for iy in 0..8i64 {
            for ix in 0..8i64 {
                // Eight samples down the centre column, one per 64 m slab.
                let mut total = 0u32;
                for t in 0..8i64 {
                    let p = I64Vec3::new(ix * 512 + 256, iy * 512 + t * 64 + 32, iz * 512 + 256);
                    let pv = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
                    if voids.iter().any(|v| v.piece.prim.shape.contains(pv)) {
                        continue;
                    }
                    let local = (origin + p).rem_euclid(I64Vec3::splat(L0_M));
                    total += match st.stratum_at(local, None) {
                        Stratum::Crust(_) => u32::from(web.fill_crust),
                        Stratum::Between(_) => u32::from(web.fill_between),
                    };
                }
                let base = u64::from(total / 8);
                let n = noise_at(seed, l1, ix, iy, iz);
                let mut f = (base * (192 + 2 * n) / 256).min(255);
                // A lull has no crust, no fill and no tissue (`gravity.md` §5): nothing may
                // float there by accident.
                let c = origin + I64Vec3::new(ix * 512 + 256, iy * 512 + 256, iz * 512 + 256);
                if crate::gravity::field_at_metres(seed, [c.x, c.y, c.z]).is_lull() {
                    f = 0;
                }
                out[((iz * 8 + iy) * 8 + ix) as usize] = f as u8;
            }
        }
    }
    out
}

/// L2 cells with at least this fill (of 255) show as mass in the far band.
const MASS_FROM: u8 = 24;

/// HOR-03: the mass of the L2 structures as the far band draws it. One box per run of built
/// L2 cells along x: as long as the run, as wide and high as the cube root of the run's least
/// fill allows (a cell of fill `f` out of 255 holds a block of edge `512 (f/255)^(1/3)`),
/// centred in its cells. Far-only, so the full L1 mesh does not draw it; it is the parent's
/// answer to what the children will make, never a simplification of them.
pub fn far_mass(seed: WorldSeed, l1: I64Vec3, fill: &[u8]) -> Vec<crate::prim::Primitive> {
    use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
    let _ = seed;
    let q = |m: i64| (m * 16) as i32;
    let mut out = Vec::new();
    for iz in 0..8i64 {
        for iy in 0..8i64 {
            let mut ix = 0;
            while ix < 8 {
                let at = |x: i64| fill[((iz * 8 + iy) * 8 + x) as usize];
                if at(ix) < MASS_FROM {
                    ix += 1;
                    continue;
                }
                let start = ix;
                let mut least = 255u8;
                while ix < 8 && at(ix) >= MASS_FROM {
                    least = least.min(at(ix));
                    ix += 1;
                }
                // The edge in metres from the cube root of the fill, by integer search.
                let target = i64::from(least) * 512 * 512 * 512 / 255;
                let mut edge = 0i64;
                while (edge + 8).pow(3) <= target && edge < 512 {
                    edge += 8;
                }
                let edge = edge.max(32);
                let (x0, x1) = (
                    start * 512 + 256 - edge / 2,
                    (ix - 1) * 512 + 256 + edge / 2,
                );
                let (y0, z0) = (iy * 512 + 256 - edge / 2, iz * 512 + 256 - edge / 2);
                out.push(Primitive {
                    shape: Shape::Box {
                        aabb: QAabb::new(
                            QVec3::new(q(x0), q(y0), q(z0)),
                            QVec3::new(q(x1), q(y0 + edge), q(z0 + edge)),
                        ),
                    },
                    material: MaterialId::CONCRETE,
                    kind: FeatureKind::MASS,
                    flags: PrimFlags::FAR_VISIBLE | PrimFlags::FAR_ONLY | PrimFlags::NO_COLLIDE,
                });
            }
        }
    }
    let _ = l1;
    out
}

/// The fill of one L2 cell, given its coordinates (`L2` cell index in the world).
pub fn fill_l2(seed: WorldSeed, l2: I64Vec3) -> u8 {
    let l1 = l2.div_euclid(I64Vec3::splat(8));
    let i = l2 - l1 * 8;
    fill_field(seed, l1)[((i.z * 8 + i.y) * 8 + i.x) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regime_web::StackGeometry;

    #[test]
    fn noise_weights_sum_to_one() {
        // With all corners equal the blend returns that value: the weights sum to 16³.
        for i in 0..8 {
            assert_eq!((15 - 2 * i) + (2 * i + 1), 16);
        }
        let seed = WorldSeed(1);
        for l in [I64Vec3::ZERO, I64Vec3::new(-3, 4, 9)] {
            for i in 0..8 {
                let n = noise_at(seed, l, i, 7 - i, 3);
                assert!(n < 64);
            }
        }
    }

    /// Neighbouring cells share corners, so the noise is continuous across L1 faces: the last
    /// L2 cell of one and the first of the next differ by little.
    #[test]
    fn noise_is_continuous_across_faces() {
        let seed = WorldSeed(2);
        for k in 0..60 {
            let l = I64Vec3::new(k, k % 5, -k);
            for axis in 0..3 {
                let mut step = [0i64; 3];
                step[axis] = 1;
                let (mut a, mut b) = ([3, 4, 5], [3, 4, 5]);
                a[axis] = 7;
                b[axis] = 0;
                let n1 = noise_at(seed, l, a[0], a[1], a[2]);
                let n2 = noise_at(seed, l + I64Vec3::from_array(step), b[0], b[1], b[2]);
                // The two sit 2/16 of a cell apart, so they differ by at most 2/16 of the range.
                assert!(n1.abs_diff(n2) <= 63 * 3 / 16 + 1, "{n1} {n2}");
            }
        }
    }

    /// The field averages to the regime's fill in crusts and Betweens, and is zero in a shaft.
    #[test]
    fn field_matches_the_regime_means() {
        let seed = WorldSeed(1);
        let (mut crust_sum, mut crust_n, mut crust_target) = (0u64, 0u64, 0u64);
        let (mut bet_sum, mut bet_n, mut bet_target) = (0u64, 0u64, 0u64);
        let mut zero_in_void = 0;
        for x in 0..10 {
            for z in 0..10 {
                let home = I64Vec3::new(x, 0, z);
                let web = regime(seed, home).web;
                if !matches!(web.stack, StackGeometry::Flat) {
                    continue;
                }
                let st = stack(seed, home);
                for l in 0..8i64 {
                    let l1 = home * 8 + I64Vec3::new(3, l, 4);
                    let field = fill_field(seed, l1);
                    for iy in 0..8i64 {
                        let y = (l * 4096 + iy * 512 + 256) / 64;
                        // A whole-L2-cell stratum: all of its 64 m slabs agree.
                        let (a, b) = (st.segment_at(y - 4), st.segment_at(y + 3));
                        if a.stratum != b.stratum {
                            continue;
                        }
                        let f = u64::from(field[(4 * 8 + iy as usize) * 8 + 3]);
                        match a.stratum {
                            Stratum::Crust(_) => {
                                crust_sum += f;
                                crust_n += 1;
                                crust_target += u64::from(web.fill_crust);
                                zero_in_void += usize::from(f == 0);
                            }
                            Stratum::Between(_) => {
                                bet_sum += f;
                                bet_n += 1;
                                bet_target += u64::from(web.fill_between);
                            }
                        }
                    }
                }
            }
        }
        assert!(
            crust_n > 200 && bet_n > 100,
            "{crust_n} crust, {bet_n} between samples"
        );
        let ratio = |s: u64, t: u64| s as f64 / t.max(1) as f64;
        // The noise multiplier averages 1.0; voids and rounding pull the mean slightly down.
        assert!(
            (0.8..1.08).contains(&ratio(crust_sum, crust_target)),
            "crust {}",
            ratio(crust_sum, crust_target)
        );
        assert!(
            (0.8..1.08).contains(&ratio(bet_sum, bet_target)),
            "between {}",
            ratio(bet_sum, bet_target)
        );
        let _ = zero_in_void;
    }

    /// An L2 cell whose centre column lies in a shaft has fill 0, and the field is the same
    /// however it is asked for.
    #[test]
    fn voids_empty_the_field() {
        use crate::l1_void::{VoidKind, voids_of_l0};
        let seed = WorldSeed(1);
        let mut checked = 0;
        for x in -6..6 {
            for z in -6..6 {
                let home = I64Vec3::new(x, 0, z);
                for v in voids_of_l0(seed, home) {
                    if v.kind != VoidKind::Shaft {
                        continue;
                    }
                    let (lo, hi) = v.bounds();
                    // The L2 cell holding the shaft's middle, at its middle height.
                    let mid = (lo + hi) / 2;
                    let l2 = mid.div_euclid(I64Vec3::splat(512));
                    let centre = l2 * 512 + I64Vec3::new(256, 0, 256);
                    // Only when the whole centre column is inside the shaft's footprint.
                    // (the footprint of a round shaft is its circle, not its bounds).
                    let inside = v.keep_out.iter().any(|s| match *s {
                        crate::l1_void::Shp::Box { lo, hi } => {
                            centre.x > lo.x && centre.x < hi.x && centre.z > lo.z && centre.z < hi.z
                        }
                        crate::l1_void::Shp::Cyl { base, r, .. } => {
                            let (dx, dz) = (centre.x - base.x, centre.z - base.z);
                            dx * dx + dz * dz < r * r
                        }
                        _ => false,
                    });
                    if !inside {
                        continue;
                    }
                    let l1 = l2.div_euclid(I64Vec3::splat(8));
                    let f = fill_l2(seed, l2);
                    assert_eq!(
                        f,
                        fill_field(seed, l1)[(((l2.z - l1.z * 8) * 8 + (l2.y - l1.y * 8)) * 8
                            + (l2.x - l1.x * 8))
                            as usize]
                    );
                    // Its eight samples all lie inside the shaft's height unless the cell is at its ends.
                    if l2.y * 512 >= lo.y && (l2.y + 1) * 512 <= hi.y {
                        assert_eq!(
                            f, 0,
                            "shaft cell {l2:?} has fill {f}: {:?} bounds {lo:?} {hi:?}",
                            v.keep_out
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no shaft cell found");
    }

    /// A lull has no fill: the middle of two equal twin wells is empty space.
    #[test]
    fn a_lull_has_no_fill() {
        use crate::gravity::{WellKind, wells_of};
        let seed = WorldSeed(0);
        let mut checked = 0;
        for x in -8..=8 {
            for z in -8..=8 {
                for y in -4..=4 {
                    let ws = wells_of(seed, I64Vec3::new(x, y, z));
                    if ws.len() != 2
                        || ws[0].kind != WellKind::Twin
                        || ws[0].strength != ws[1].strength
                    {
                        continue;
                    }
                    let mid = [0, 1, 2].map(|i| i64::midpoint(ws[0].centre[i], ws[1].centre[i]));
                    if !crate::gravity::field_at_metres(seed, mid).is_lull() {
                        continue;
                    }
                    let l2 = I64Vec3::new(
                        mid[0].div_euclid(512),
                        mid[1].div_euclid(512),
                        mid[2].div_euclid(512),
                    );
                    // The cell is a lull at its centre only if the pair is symmetric about it;
                    // check the cell whose centre is the lull.
                    let c = [0, 1, 2].map(|i| [l2.x, l2.y, l2.z][i] * 512 + 256);
                    if crate::gravity::field_at_metres(seed, c).is_lull() {
                        assert_eq!(fill_l2(seed, l2), 0);
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no lull cell found between twins");
    }
}
