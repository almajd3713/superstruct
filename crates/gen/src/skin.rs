//! Skin coverage (TIS-06 with the dead-end rule, HOR-02): the crust's skin plates are covered
//! by structures that grew against them, or by cladding where nothing did. Spec:
//! `docs/spec/levels.md` §5W.7.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::match_same_arms,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::type_complexity
)]

use glam::{DVec3, I64Vec3, Vec3};

use crate::cell::Level;
use crate::hash::{absorb, hash};
use crate::l1_surface::SURF_SKIN;
use crate::orient::{Frame, LocalShape};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue::{Lattice, Reserved, Tissue};

/// Edge of a skin tile in metres.
pub const TILE_M: i64 = 32;

#[derive(Debug, Clone, Default)]
pub struct Skin {
    /// Tiles whose centre lies in this cell.
    pub tiles: usize,
    /// Tiles a structure already covered.
    pub covered: usize,
    pub cladding: Vec<Primitive>,
    /// The 16 probe points of every tile, cell-local metres, for the coverage test.
    #[cfg(test)]
    pub samples: Vec<Vec<Vec3>>,
}

/// A skin plate piece in its frame: origin (L1-local quanta) and local box.
fn plate_of(r: &Reserved) -> Option<(Frame, QVec3, QAabb)> {
    if r.piece.tag != SURF_SKIN {
        return None;
    }
    match r.piece.prim.shape {
        Shape::Box { aabb } => Some((Frame::Id, QVec3::default(), aabb)),
        Shape::Oriented(o) => match o.local {
            LocalShape::Box { aabb } => Some((o.frame, o.origin, aabb)),
            _ => None,
        },
        _ => None,
    }
}

fn to_m(q: i32) -> f64 {
    f64::from(q) / f64::from(Q_PER_M)
}

/// The skin pass of one L2 cell. `parent_pieces` are the L1 pieces, `local` the cell's index
/// in its L1 cell.
pub fn skin_pass(
    seed: WorldSeed,
    t: &Tissue,
    parent_pieces: &[Reserved],
    local: I64Vec3,
    structures: &[Primitive],
) -> Skin {
    let cell_m = (local * 512).as_dvec3();
    let mut skin = Skin::default();
    let cell_clip = QAabb::new(
        QVec3::new(0, 0, 0),
        QVec3::new(512 * Q_PER_M, 512 * Q_PER_M, 512 * Q_PER_M),
    );
    let mut rng = Rng::new(absorb(
        hash(seed.0, Level::L2, t.cell.coords, salt::SKIN_WINDOWS),
        0x5C1,
    ));
    for r in parent_pieces {
        let Some((frame, origin, a)) = plate_of(r) else {
            continue;
        };
        // Quick rejection by the piece's bounds in the L1 cell.
        let b = r.piece.prim.aabb();
        let q = f64::from(Q_PER_M);
        let (blo, bhi) = (
            DVec3::new(f64::from(b.min.x), f64::from(b.min.y), f64::from(b.min.z)) / q,
            DVec3::new(f64::from(b.max.x), f64::from(b.max.y), f64::from(b.max.z)) / q,
        );
        if bhi.cmple(cell_m).any() || blo.cmpge(cell_m + DVec3::splat(512.0)).any() {
            continue;
        }
        let size = [
            to_m(a.max.x - a.min.x),
            to_m(a.max.y - a.min.y),
            to_m(a.max.z - a.min.z),
        ];
        let k = (0..3)
            .min_by(|&i, &j| size[i].total_cmp(&size[j]))
            .unwrap_or(1);
        if size[k] > 16.0 {
            continue;
        }
        let (u, v) = match k {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let axes = frame.axes();
        let lo = [a.min.x, a.min.y, a.min.z].map(to_m);
        let hi = [a.max.x, a.max.y, a.max.z].map(to_m);
        let o = DVec3::new(to_m(origin.x), to_m(origin.y), to_m(origin.z)) - cell_m;
        // Frame-local metres to cell-local metres.
        let at = |l: [f64; 3]| o + axes[0] * l[0] + axes[1] * l[1] + axes[2] * l[2];
        // The outside is the side that is not crust.
        let mid = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let probe = |sign: f64| {
            let mut l = mid;
            l[k] += sign * (size[k] / 2.0 + 20.0);
            t.lattice.slot_of(at(l)).map(|i| t.in_crust[i])
        };
        let sign = match (probe(1.0), probe(-1.0)) {
            (Some(false), Some(true)) => 1.0,
            (Some(true), Some(false)) => -1.0,
            (Some(false), None) => 1.0,
            (None, Some(false)) => -1.0,
            _ => continue,
        };
        let face = if sign > 0.0 { hi[k] } else { lo[k] };
        let tile = TILE_M as f64;
        // The cell's corners in the frame bound the tiles to look at.
        let (mut cu, mut cv) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
        for i in 0..8 {
            let c = DVec3::new(
                if i & 1 == 0 { 0.0 } else { 512.0 },
                if i & 2 == 0 { 0.0 } else { 512.0 },
                if i & 4 == 0 { 0.0 } else { 512.0 },
            ) - o;
            let (du, dv) = (c.dot(axes[u]), c.dot(axes[v]));
            cu = (cu.0.min(du), cu.1.max(du));
            cv = (cv.0.min(dv), cv.1.max(dv));
        }
        let (ulo, uhi) = (lo[u].max(cu.0), hi[u].min(cu.1));
        let (vlo, vhi) = (lo[v].max(cv.0), hi[v].min(cv.1));
        if uhi <= ulo || vhi <= vlo {
            continue;
        }
        let (iu0, iu1) = (
            (ulo / tile).floor() as i64,
            ((uhi - 1e-6) / tile).floor() as i64,
        );
        let (iv0, iv1) = (
            (vlo / tile).floor() as i64,
            ((vhi - 1e-6) / tile).floor() as i64,
        );
        let mut uncovered: Vec<(i64, i64, [f64; 4])> = Vec::new();
        for iu in iu0..=iu1 {
            for iv in iv0..=iv1 {
                let (u0, u1) = (
                    (iu as f64 * tile).max(lo[u]),
                    ((iu + 1) as f64 * tile).min(hi[u]),
                );
                let (v0, v1) = (
                    (iv as f64 * tile).max(lo[v]),
                    ((iv + 1) as f64 * tile).min(hi[v]),
                );
                if u1 <= u0 || v1 <= v0 {
                    continue;
                }
                let mut l = [0.0; 3];
                l[u] = (u0 + u1) / 2.0;
                l[v] = (v0 + v1) / 2.0;
                l[k] = face;
                let c = at(l);
                if !(c.cmpge(DVec3::ZERO).all() && c.cmplt(DVec3::splat(512.0)).all()) {
                    continue;
                }
                skin.tiles += 1;
                let mut hit = 0;
                #[cfg(test)]
                let mut probes = Vec::new();
                for a in 0..4 {
                    for b in 0..4 {
                        let mut p = [0.0; 3];
                        p[u] = u0 + (u1 - u0) * (f64::from(a) + 0.5) / 4.0;
                        p[v] = v0 + (v1 - v0) * (f64::from(b) + 0.5) / 4.0;
                        p[k] = face + sign * 0.5;
                        let w = at(p);
                        let w = Vec3::new(w.x as f32, w.y as f32, w.z as f32);
                        #[cfg(test)]
                        probes.push(w);
                        hit += i32::from(structures.iter().any(|s| s.shape.contains(w)));
                    }
                }
                #[cfg(test)]
                skin.samples.push(probes);
                if hit * 4 >= 3 * 16 {
                    skin.covered += 1;
                } else {
                    uncovered.push((iu, iv, [u0, u1, v0, v1]));
                }
            }
        }
        // Cladding: runs of uncovered tiles along u, merged along v when they match.
        uncovered.sort_by_key(|&(iu, iv, _)| (iv, iu));
        let mut runs: Vec<[f64; 4]> = Vec::new();
        let mut last: Option<(i64, i64)> = None;
        for &(iu, iv, r) in &uncovered {
            match (last, runs.last_mut()) {
                (Some((pu, pv)), Some(run)) if pv == iv && pu + 1 == iu => run[1] = r[1],
                _ => runs.push(r),
            }
            last = Some((iu, iv));
        }
        let mut merged: Vec<[f64; 4]> = Vec::new();
        for r in runs {
            match merged
                .iter_mut()
                .find(|m| m[0] == r[0] && m[1] == r[1] && m[3] == r[2])
            {
                Some(m) => m[3] = r[3],
                None => merged.push(r),
            }
        }
        for m in merged {
            let depth = 2.0 + f64::from(rng.range_i32(0, 15));
            let (d0, d1) = if sign > 0.0 {
                (face, face + depth)
            } else {
                (face - depth, face)
            };
            let mut l0 = [0.0; 3];
            let mut l1 = [0.0; 3];
            l0[u] = m[0];
            l1[u] = m[1];
            l0[v] = m[2];
            l1[v] = m[3];
            l0[k] = d0;
            l1[k] = d1;
            let q = |x: f64| (x * f64::from(Q_PER_M)).round() as i32;
            let local_box = QAabb::new(
                QVec3::new(q(l0[0]), q(l0[1]), q(l0[2])),
                QVec3::new(q(l1[0]), q(l1[1]), q(l1[2])),
            );
            let origin_q = QVec3::new(q(o.x), q(o.y), q(o.z));
            skin.cladding.push(Primitive {
                shape: Shape::oriented(
                    frame,
                    origin_q,
                    LocalShape::Box { aabb: local_box },
                    cell_clip,
                ),
                material: MaterialId::CONCRETE,
                kind: FeatureKind::SKIN,
                flags: PrimFlags::FAR_VISIBLE,
            });
        }
    }
    skin
}

impl Lattice {
    /// The present slot whose cube holds the cell-local point `p` (metres), if any.
    pub fn slot_of(&self, p: DVec3) -> Option<usize> {
        let axes = self.frame.axes();
        let d = p - DVec3::splat(256.0);
        let n = self.n as i64;
        let f = |k: usize| ((d.dot(axes[k]) + 256.0) / self.pitch as f64).floor() as i64;
        let c = [f(0), f(1), f(2)];
        if c.iter().any(|&v| !(0..n).contains(&v)) {
            return None;
        }
        let i = self.index([c[0] as usize, c[1] as usize, c[2] as usize]);
        self.present[i].then_some(i)
    }
}
