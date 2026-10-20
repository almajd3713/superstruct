//! Z-fighting census (PW.3.21d): faces of different pieces that lie in one plane, face the
//! same way and are both visible. Test-only; boxes only (flat or turned), which are nearly all
//! the pieces. Such pairs are separated by the mesher (`engine::primmesh::nudge`); what no
//! offset can separate, identical pieces, must not occur, and the test says so.

#![allow(
    clippy::cast_possible_truncation,
    clippy::default_trait_access,
    clippy::too_many_lines
)]

use glam::{DVec3, Vec3};

use crate::carve::as_box;
use crate::prim::{FeatureKind, PrimFlags, Primitive, Shape};

/// A box piece's six faces: a corner, the two edge vectors and the outward normal (metres).
fn faces(p: &Primitive) -> Vec<(DVec3, DVec3, DVec3, DVec3)> {
    let Some((frame, origin, l)) = as_box(&p.shape) else {
        return Vec::new();
    };
    let [ex, ey, ez] = frame.axes();
    let o = origin.to_metres();
    let o = DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z));
    let (lo, hi) = (l.min.to_metres(), l.max.to_metres());
    let lo = DVec3::new(f64::from(lo.x), f64::from(lo.y), f64::from(lo.z));
    let hi = DVec3::new(f64::from(hi.x), f64::from(hi.y), f64::from(hi.z));
    let ax = [ex, ey, ez];
    let size = hi - lo;
    let at = |c: DVec3| o + ex * c.x + ey * c.y + ez * c.z;
    let mut out = Vec::with_capacity(6);
    for k in 0..3 {
        let (i, j) = ((k + 1) % 3, (k + 2) % 3);
        let mut u = DVec3::ZERO;
        u[i] = size[i];
        let mut v = DVec3::ZERO;
        v[j] = size[j];
        for (high, sign) in [(false, -1.0), (true, 1.0)] {
            let mut c = lo;
            if high {
                c[k] = hi[k];
            }
            out.push((at(c), ax_vec(&ax, u), ax_vec(&ax, v), ax[k] * sign));
        }
    }
    out
}

fn ax_vec(ax: &[DVec3; 3], v: DVec3) -> DVec3 {
    ax[0] * v.x + ax[1] * v.y + ax[2] * v.z
}

fn vec3(v: DVec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

/// Pairs of kinds that have a coplanar, same-facing, visible face, with the sample count.
pub fn census_with(
    prims: &[Primitive],
    each: &mut dyn FnMut(&Primitive, &Primitive),
) -> std::collections::BTreeMap<(u8, u8), usize> {
    const EPS: f64 = 0.003;
    let drawn: Vec<&Primitive> = prims
        .iter()
        .filter(|p| {
            !p.flags.contains(PrimFlags::ENVELOPE)
                && !matches!(p.shape, Shape::LiftShaft { .. })
                && p.kind != FeatureKind::VOID
        })
        .collect();
    let mut found = std::collections::BTreeMap::new();
    for (ai, a) in drawn.iter().enumerate() {
        for (corner, u, v, n) in faces(a) {
            if u.length() < 0.2 || v.length() < 0.2 {
                continue;
            }
            for (fu, fv) in [(0.23, 0.31), (0.67, 0.19), (0.41, 0.77), (0.83, 0.58)] {
                let p = corner + u * fu + v * fv;
                let (out, inn) = (vec3(p + n * EPS), vec3(p - n * EPS));
                if !a.shape.contains(inn) || a.shape.contains(out) {
                    continue;
                }
                let covered = drawn.iter().any(|q| q.shape.contains(out));
                if covered {
                    continue;
                }
                for (bi, b) in drawn.iter().enumerate() {
                    if bi != ai && b.shape.contains(inn) {
                        *found.entry((a.kind.0, b.kind.0)).or_default() += 1;
                        each(a, b);
                        break;
                    }
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carve::translated;
    use crate::cell::{CellKey, Level};
    use crate::l2_port::L1Cache;
    use crate::l2_web::l2_cell_web;
    use crate::seed::WorldSeed;
    use glam::I64Vec3;

    #[test]
    fn coplanar_faces() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let mut total = std::collections::BTreeMap::new();
        let (mut cells, mut faces_seen, mut duplicates) = (0, 0usize, 0usize);
        let mut dup_kinds = std::collections::BTreeMap::new();
        let n: i64 = std::env::var("ZF_CELLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6);
        let parents = std::env::var("ZF_PARENT").is_ok();
        let mut shown: std::collections::BTreeMap<(u8, u8), usize> = Default::default();
        for k in 0..n {
            let l2 = I64Vec3::new(k % 8 - 8, k / 8 % 4, (k * 3) % 8) + I64Vec3::new(0, 0, 0);
            let key = CellKey::new(Level::L2, l2);
            let local = l2.rem_euclid(I64Vec3::splat(8));
            let l1 = CellKey::new(Level::L1, l2.div_euclid(I64Vec3::splat(8)));
            let mut prims = l2_cell_web(seed, &cache, key).primitives;
            let d = [local.x, local.y, local.z].map(|v| -(v as i32) * 512 * 16);
            let parent = cache.get(seed, l1);
            let reach = |p: &Primitive| {
                let b = p.aabb();
                b.max.x > 0
                    && b.min.x < 8192
                    && b.max.y > 0
                    && b.min.y < 8192
                    && b.max.z > 0
                    && b.min.z < 8192
            };
            for p in parent.primitives.iter().filter(|_| parents) {
                let q = translated(p, d);
                if reach(&q) {
                    prims.push(q);
                }
            }
            faces_seen += prims.len();
            let mut seen = std::collections::HashSet::new();
            for p in &prims {
                if !seen.insert((p.shape, p.material)) {
                    duplicates += 1;
                    *dup_kinds.entry(p.kind.0).or_insert(0usize) += 1;
                }
            }
            let found = census_with(&prims, &mut |a, b| {
                let e = shown.entry((a.kind.0, b.kind.0)).or_default();
                if *e < 2 && std::env::var("ZF_SHOW").is_ok() {
                    *e += 1;
                    eprintln!(
                        "EX {:?}/{:?}:\n  A {:?}\n  B {:?}",
                        a.kind, b.kind, a.shape, b.shape
                    );
                }
            });
            for (kk, n) in found {
                *total.entry(kk).or_insert(0usize) += n;
            }
            cells += 1;
        }
        assert_eq!(
            duplicates, 0,
            "identical pieces cannot be nudged apart: {dup_kinds:?}"
        );
        eprintln!(
            "{cells} cells, {faces_seen} pieces; coplanar samples by (kind, kind): {total:?}"
        );
    }

    /// Every lit window panel of the structures stands on a face of a solid piece: its back is
    /// in one and its front is in none (PW.3.21e: they floated off walls in another frame).
    #[test]
    fn windows_lie_on_walls() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut panels, mut floating) = (0usize, 0usize);
        for k in 0..60i64 {
            let l2 = I64Vec3::new(k % 8 - 8, k / 8 % 4, (k * 3) % 8);
            let key = CellKey::new(Level::L2, l2);
            let local = l2.rem_euclid(I64Vec3::splat(8));
            let l1 = CellKey::new(Level::L1, l2.div_euclid(I64Vec3::splat(8)));
            let own = l2_cell_web(seed, &cache, key).primitives;
            let d = [local.x, local.y, local.z].map(|v| -(v as i32) * 512 * 16);
            let mut solid: Vec<Primitive> = own
                .iter()
                .filter(|p| {
                    !p.flags.contains(PrimFlags::NO_COLLIDE) || p.kind != FeatureKind::WINDOW
                })
                .copied()
                .collect();
            solid.extend(
                cache
                    .get(seed, l1)
                    .primitives
                    .iter()
                    .map(|p| translated(p, d)),
            );
            solid.retain(|p| {
                !p.flags.contains(PrimFlags::NO_COLLIDE) && p.kind != FeatureKind::WINDOW
            });
            for w in own
                .iter()
                .filter(|p| p.material == crate::prim::MaterialId::LIT_WINDOW)
            {
                let Some((frame, origin, l)) = as_box(&w.shape) else {
                    continue;
                };
                let [ex, ey, ez] = frame.axes();
                let ax = [ex, ey, ez];
                let (lo, hi) = (l.min.to_metres(), l.max.to_metres());
                let (lo, hi) = (
                    DVec3::new(f64::from(lo.x), f64::from(lo.y), f64::from(lo.z)),
                    DVec3::new(f64::from(hi.x), f64::from(hi.y), f64::from(hi.z)),
                );
                let size = hi - lo;
                let thin = (0..3)
                    .min_by(|&a, &b| size[a].total_cmp(&size[b]))
                    .unwrap_or(0);
                let mut dims = [size[0], size[1], size[2]];
                dims.sort_by(f64::total_cmp);
                if size[thin] > 0.2
                    || (dims[1] - 1.375).abs() > 0.01
                    || (dims[2] - 1.625).abs() > 0.01
                {
                    continue;
                }
                panels += 1;
                let o = origin.to_metres();
                let o = DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z));
                let centre = o
                    + ax[0] * (lo.x + hi.x) / 2.0
                    + ax[1] * (lo.y + hi.y) / 2.0
                    + ax[2] * (lo.z + hi.z) / 2.0;
                let n = ax[thin];
                let reach = size[thin] / 2.0 + 0.06;
                let stands = [-1.0, 1.0].into_iter().any(|side| {
                    let back = vec3(centre - n * reach * side);
                    let front = vec3(centre + n * reach * side);
                    solid.iter().any(|q| q.shape.contains(back))
                        && !solid.iter().any(|q| q.shape.contains(front))
                });
                if !stands {
                    floating += 1;
                }
            }
        }
        eprintln!("{panels} window panels, {floating} not on a wall");
        assert!(panels > 100, "{panels} panels");
        assert!(
            floating * 10 <= panels,
            "{floating} of {panels} panels float"
        );
    }
}
