//! What a gravity well builds in the L2 cells around it: a planetoid core or a hollow shell, an
//! axis cable with rings, a deck, a membrane with its gate holes, a corridor, drifting debris.
//! Spec: `docs/spec/gravity.md` §3.4.

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_range_loop
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::gates::gates_in_l2;
use crate::gravity::{Well, WellKind, dir, wells_of};
use crate::hash::hash;
use crate::orient::beam_cylinder;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L2_SIZE: i64 = 512;
const L0_SIZE: i64 = 32768;

/// Half size of a Point well's cube planetoid, in metres: an eighth of the reach.
pub fn core_half(w: &Well) -> i64 {
    (w.reach / 8).clamp(128, 1024) / 16 * 16
}

/// Half size of a Shell well's hollow cube, in metres: a quarter of the reach.
pub fn shell_half(w: &Well) -> i64 {
    (w.reach / 4).clamp(512, 4096) / 16 * 16
}

/// Thickness of the walls of a core or a shell, in metres.
const WALL: i64 = 24;
/// The seam's plate and the plane's deck, in metres.
const PLATE: i64 = 2;
const DECK: i64 = 4;
/// Half the clear width of a fold's corridor, and its wall thickness, in metres.
const HALL: i64 = 6;
const HALL_WALL: i64 = 2;

struct Cell {
    lo: [i64; 3],
}

impl Cell {
    /// `[min, max)` in metres intersected with the cell, as a box in cell-local Q.
    fn clip(&self, min: [i64; 3], max: [i64; 3]) -> Option<QAabb> {
        let mut lo = [0i32; 3];
        let mut hi = [0i32; 3];
        for i in 0..3 {
            let a = min[i].max(self.lo[i]);
            let b = max[i].min(self.lo[i] + L2_SIZE);
            if a >= b {
                return None;
            }
            lo[i] = ((a - self.lo[i]) * 16) as i32;
            hi[i] = ((b - self.lo[i]) * 16) as i32;
        }
        Some(QAabb::new(
            QVec3::new(lo[0], lo[1], lo[2]),
            QVec3::new(hi[0], hi[1], hi[2]),
        ))
    }

    fn solid(&self, min: [i64; 3], max: [i64; 3], material: MaterialId) -> Option<Primitive> {
        Some(Primitive {
            shape: Shape::Box {
                aabb: self.clip(min, max)?,
            },
            material,
            kind: FeatureKind::MASS,
            flags: PrimFlags::NONE,
        })
    }

    fn light(&self, min: [i64; 3], max: [i64; 3]) -> Option<Primitive> {
        Some(Primitive {
            shape: Shape::Box {
                aabb: self.clip(min, max)?,
            },
            material: MaterialId::EMISSIVE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NO_COLLIDE | PrimFlags::FAR_VISIBLE,
        })
    }
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Six slabs of a cube of half size `half` and wall thickness `WALL` round `c`. The slabs on
/// x take the whole face, those on y stop short of the x walls, those on z of both.
fn cube_walls(cell: &Cell, c: [i64; 3], half: i64, out: &mut Vec<Primitive>) {
    for a in 0..3 {
        for sign in [-1i64, 1] {
            let mut min = [0i64; 3];
            let mut max = [0i64; 3];
            for i in 0..3 {
                let inset = if i < a { WALL } else { 0 };
                min[i] = c[i] - half + inset;
                max[i] = c[i] + half - inset;
            }
            if sign < 0 {
                max[a] = c[a] - half + WALL;
            } else {
                min[a] = c[a] + half - WALL;
            }
            if let Some(p) = cell.solid(min, max, MaterialId::CONCRETE) {
                out.push(p);
            }
        }
    }
}

fn line_pieces(cell: &Cell, w: &Well, out: &mut Vec<Primitive>) {
    let d = dir(w.axis);
    let half = w.extent[1].max(256);
    let n = d.iter().map(|v| v * v).sum::<i64>();
    let len = {
        let mut x = n;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = i64::midpoint(x, n / x);
        }
        x.max(1)
    };
    // The cable is drawn in the cells that contain a point of the axis.
    let mut hit = false;
    let mut t = -half;
    while t <= half {
        let p = [0, 1, 2].map(|i| w.centre[i] + d[i] * t / len);
        if (0..3).all(|i| p[i] >= cell.lo[i] && p[i] < cell.lo[i] + L2_SIZE) {
            hit = true;
            break;
        }
        t += 64;
    }
    if !hit {
        return;
    }
    let start = [0, 1, 2].map(|i| (w.centre[i] - d[i] * half / len - cell.lo[i]) * 16);
    let clip = QAabb::new(
        QVec3::new(0, 0, 0),
        QVec3::new(512 * 16, 512 * 16, 512 * 16),
    );
    let dd = [d[0] as i8, d[1] as i8, d[2] as i8];
    out.push(Primitive {
        shape: beam_cylinder(
            dd,
            QVec3::new(start[0] as i32, start[1] as i32, start[2] as i32),
            (2 * half * 16) as i32,
            8,
            clip,
        ),
        material: MaterialId::EMISSIVE,
        kind: FeatureKind::CABLE,
        flags: PrimFlags::NO_COLLIDE | PrimFlags::FAR_VISIBLE,
    });
    // Rings of light every 128 m on an axis-aligned well.
    let Some(a) = (0..3).find(|&i| d[i] != 0 && d[(i + 1) % 3] == 0 && d[(i + 2) % 3] == 0) else {
        return;
    };
    let radius = if w.kind == WellKind::Tube { 48 } else { 12 };
    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
    let mut pos = (w.centre[a] - half).div_euclid(128) * 128;
    while pos <= w.centre[a] + half {
        for (along, across, sign) in [(b, c, -1), (b, c, 1), (c, b, -1), (c, b, 1)] {
            let mut min = [0i64; 3];
            let mut max = [0i64; 3];
            min[a] = pos;
            max[a] = pos + 1;
            let edge = w.centre[across] + sign * radius;
            min[across] = edge.min(edge + sign);
            max[across] = edge.max(edge + sign) + 1;
            min[along] = w.centre[along] - radius;
            max[along] = w.centre[along] + radius + 1;
            if let Some(p) = cell.light(min, max) {
                out.push(p);
            }
        }
        pos += 128;
    }
}

/// A plate of `[lo, hi)` across the extent in the axes other than `a`, with holes where the
/// gates of the cell pass through.
fn plate(
    cell: &Cell,
    w: &Well,
    a: usize,
    lo: i64,
    hi: i64,
    holes: &[([i64; 3], [i64; 3])],
    out: &mut Vec<Primitive>,
) {
    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
    let region = |i: usize| (w.centre[i] - w.extent[i], w.centre[i] + w.extent[i]);
    let mut rects: Vec<[(i64, i64); 2]> = vec![[region(b), region(c)]];
    for (hmin, hmax) in holes {
        let hole = [(hmin[b], hmax[b]), (hmin[c], hmax[c])];
        let mut next = Vec::new();
        for r in rects {
            // `r` minus `hole`: left and right in `b`, then front and back in `c`.
            let (ob0, ob1) = (r[0].0.max(hole[0].0), r[0].1.min(hole[0].1));
            let (oc0, oc1) = (r[1].0.max(hole[1].0), r[1].1.min(hole[1].1));
            if ob0 >= ob1 || oc0 >= oc1 {
                next.push(r);
                continue;
            }
            if r[0].0 < ob0 {
                next.push([(r[0].0, ob0), r[1]]);
            }
            if ob1 < r[0].1 {
                next.push([(ob1, r[0].1), r[1]]);
            }
            if r[1].0 < oc0 {
                next.push([(ob0, ob1), (r[1].0, oc0)]);
            }
            if oc1 < r[1].1 {
                next.push([(ob0, ob1), (oc1, r[1].1)]);
            }
        }
        rects = next;
    }
    for r in rects {
        let mut min = [0i64; 3];
        let mut max = [0i64; 3];
        min[a] = lo;
        max[a] = hi;
        min[b] = r[0].0;
        max[b] = r[0].1;
        min[c] = r[1].0;
        max[c] = r[1].1;
        if let Some(p) = cell.solid(min, max, MaterialId::METAL) {
            out.push(p);
        }
    }
}

fn fold_pieces(cell: &Cell, w: &Well, out: &mut Vec<Primitive>) {
    for s in &w.fold {
        let up_axis = (0..3).find(|&i| s.up[i] != 0).unwrap_or(1);
        let side = (0..3).find(|&i| i != s.axis && i != up_axis).unwrap_or(0);
        let mut lo = s.start;
        let mut hi = s.start;
        let end = s.start[s.axis] + s.sign * s.len;
        lo[s.axis] = s.start[s.axis].min(end) - HALL;
        hi[s.axis] = s.start[s.axis].max(end) + HALL;
        // Floor, ceiling and the two walls round the corridor's centre line.
        for (axis, sign) in [(up_axis, -1i64), (up_axis, 1), (side, -1), (side, 1)] {
            let mut min = [lo[0], lo[1], lo[2]];
            let mut max = [hi[0], hi[1], hi[2]];
            for i in 0..3 {
                if i != s.axis {
                    min[i] = s.start[i] - HALL - HALL_WALL;
                    max[i] = s.start[i] + HALL + HALL_WALL;
                }
            }
            if sign < 0 {
                min[axis] = s.start[axis] - HALL - HALL_WALL;
                max[axis] = s.start[axis] - HALL;
            } else {
                min[axis] = s.start[axis] + HALL;
                max[axis] = s.start[axis] + HALL + HALL_WALL;
            }
            if let Some(p) = cell.solid(min, max, MaterialId::CONCRETE) {
                out.push(p);
            }
        }
    }
}

fn debris(seed: WorldSeed, cell: &Cell, l2: I64Vec3, w: &Well, out: &mut Vec<Primitive>) {
    let mid = [0, 1, 2].map(|i| cell.lo[i] + L2_SIZE / 2);
    let r = [0, 1, 2].map(|i| mid[i] - w.centre[i]);
    let a = r.map(i64::abs);
    let d = a[0].max(a[1]).max(a[2]).max(4 * (a[0] + a[1] + a[2]) / 7);
    if d >= w.reach {
        return;
    }
    let mut rng = Rng::new(hash(seed.0, Level::L2, l2, salt::WELL_PATCH));
    let n = 3 + rng.below(4);
    for _ in 0..n {
        let size = [0, 1, 2].map(|_| 2 + 2 * rng.below(5) as i64);
        let at = [0, 1, 2].map(|i| cell.lo[i] + 16 + rng.below(480) as i64);
        let max = add(at, size);
        if let Some(p) = cell.solid(at, max, MaterialId::CONCRETE) {
            out.push(p);
        }
    }
}

/// KIT-15: the furnace's machinery in the L2 cell that holds a well's centre: eight boilers on a
/// ring of radius 200 m with a pipe run joining them, round the hub.
fn furnace_machinery(cell: &Cell, w: &Well, out: &mut Vec<Primitive>) {
    let c = w.centre;
    if !(0..3).all(|i| c[i] >= cell.lo[i] && c[i] < cell.lo[i] + L2_SIZE) {
        return;
    }
    // Eight points of a square ring: the corners and the middles of its sides.
    let r = 200i64;
    let spots: [(i64, i64); 8] = [
        (-r, -r),
        (0, -r),
        (r, -r),
        (r, 0),
        (r, r),
        (0, r),
        (-r, r),
        (-r, 0),
    ];
    for (dx, dz) in spots {
        let (x, z) = (c[0] + dx, c[2] + dz);
        let min = [x - 8, c[1] - 8, z - 8];
        let max = [x + 8, c[1] + 28, z + 8];
        if let Some(p) = cell.solid(min, max, MaterialId::PIPE) {
            out.push(p);
        }
    }
    // The pipe run: a square frame of 1 m pipes at 10 m up, no collision.
    for (a, b) in [(0usize, 2usize), (2, 4), (4, 6), (6, 0)] {
        let (p, q) = (spots[a], spots[b]);
        let min = [c[0] + p.0.min(q.0) - 1, c[1] + 10, c[2] + p.1.min(q.1) - 1];
        let max = [c[0] + p.0.max(q.0) + 1, c[1] + 12, c[2] + p.1.max(q.1) + 1];
        if let Some(mut piece) = cell.solid(min, max, MaterialId::PIPE) {
            piece.kind = FeatureKind::PIPE;
            piece.flags = PrimFlags::NO_COLLIDE;
            out.push(piece);
        }
    }
}

/// The pieces the wells near L2 cell `l2` build inside it (cell-local, 1/16 m).
pub fn cell_pieces(seed: WorldSeed, l2: I64Vec3) -> Vec<Primitive> {
    let lo = [l2.x * L2_SIZE, l2.y * L2_SIZE, l2.z * L2_SIZE];
    let cell = Cell { lo };
    let home = I64Vec3::new(
        lo[0].div_euclid(L0_SIZE),
        lo[1].div_euclid(L0_SIZE),
        lo[2].div_euclid(L0_SIZE),
    );
    let mut out = Vec::new();
    let mut gates = None;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                for w in wells_of(seed, home + I64Vec3::new(dx, dy, dz)).iter() {
                    if w.quiet {
                        continue;
                    }
                    furnace_machinery(&cell, w, &mut out);
                    match w.kind {
                        WellKind::Point => cube_walls(&cell, w.centre, core_half(w), &mut out),
                        WellKind::Twin => {
                            cube_walls(&cell, w.centre, (core_half(w) / 2).max(64), &mut out);
                        }
                        WellKind::Shell => cube_walls(&cell, w.centre, shell_half(w), &mut out),
                        WellKind::Line | WellKind::Tube => line_pieces(&cell, w, &mut out),
                        WellKind::Plane => {
                            let a = dir(w.axis).iter().position(|&v| v != 0).unwrap_or(1);
                            plate(&cell, w, a, w.centre[a] - DECK, w.centre[a], &[], &mut out);
                        }
                        WellKind::Seam => {
                            let a = dir(w.axis).iter().position(|&v| v != 0).unwrap_or(1);
                            let g = gates.get_or_insert_with(|| gates_in_l2(seed, l2));
                            let holes: Vec<_> = g.iter().map(|z| (z.min, z.max)).collect();
                            plate(
                                &cell,
                                w,
                                a,
                                w.centre[a] - PLATE / 2,
                                w.centre[a] + PLATE / 2,
                                &holes,
                                &mut out,
                            );
                        }
                        WellKind::Fold => fold_pieces(&cell, w, &mut out),
                        WellKind::Damaged => debris(seed, &cell, l2, w, &mut out),
                        WellKind::Tidal => {}
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gravity::find_well;

    const SEED: WorldSeed = WorldSeed(0);

    fn l2_of(p: [i64; 3]) -> I64Vec3 {
        I64Vec3::new(
            p[0].div_euclid(L2_SIZE),
            p[1].div_euclid(L2_SIZE),
            p[2].div_euclid(L2_SIZE),
        )
    }

    /// Whether metre point `p` lies inside a solid piece the wells built in its L2 cell.
    fn solid_at(p: [i64; 3]) -> bool {
        let l2 = l2_of(p);
        let local = glam::Vec3::new(
            (p[0] - l2.x * L2_SIZE) as f32 + 0.5,
            (p[1] - l2.y * L2_SIZE) as f32 + 0.5,
            (p[2] - l2.z * L2_SIZE) as f32 + 0.5,
        );
        cell_pieces(SEED, l2)
            .iter()
            .filter(|q| !q.flags.contains(PrimFlags::NO_COLLIDE))
            .any(|q| q.shape.contains(local))
    }

    #[test]
    fn a_point_well_has_a_hollow_cube_core() {
        let (_, w) = find_well(SEED, WellKind::Point, 8).expect("a point well");
        let h = core_half(&w);
        let c = w.centre;
        assert!(solid_at([c[0] + h - 4, c[1], c[2]]));
        assert!(solid_at([c[0], c[1] - h + 4, c[2]]));
        assert!(
            !solid_at([c[0] + h - WALL - 8, c[1], c[2]]),
            "the core is hollow"
        );
        assert!(!solid_at([c[0] + h + 8, c[1], c[2]]));
    }

    #[test]
    fn a_shell_well_has_a_hollow_cube_to_stand_inside() {
        let (_, w) = find_well(SEED, WellKind::Shell, 8).expect("a shell well");
        let h = shell_half(&w);
        let c = w.centre;
        assert!(solid_at([c[0], c[1], c[2] + h - 4]));
        assert!(!solid_at([c[0], c[1], c[2] + h - WALL - 8]));
        assert!(!solid_at(c));
    }

    #[test]
    fn a_plane_has_a_deck_and_a_seam_a_plate_with_holes() {
        let (_, w) = find_well(SEED, WellKind::Plane, 20).expect("a plane well");
        let a = dir(w.axis).iter().position(|&v| v != 0).unwrap();
        let mut p = w.centre;
        p[a] -= 2;
        assert!(solid_at(p), "deck under the plane coordinate");
        p[a] += 6;
        assert!(!solid_at(p), "nothing above it");

        let (_, s) = find_well(SEED, WellKind::Seam, 40).expect("a seam well");
        let a = dir(s.axis).iter().position(|&v| v != 0).unwrap();
        let mut found_plate = false;
        let mut found_hole = false;
        for dz in -6..6 {
            for dy in -6..6 {
                for dx in -6..6 {
                    let l2 = l2_of(s.centre) + I64Vec3::new(dx, dy, dz);
                    for z in crate::gates::gates_in_l2(SEED, l2).iter() {
                        let mid = [0, 1, 2].map(|i| i64::midpoint(z.min[i], z.max[i]));
                        found_hole |= !solid_at(mid);
                        let mut off = mid;
                        let b = (a + 1) % 3;
                        off[b] = z.max[b] + 8;
                        found_plate |= solid_at([off[0], off[1], off[2]]);
                    }
                }
            }
        }
        assert!(found_hole, "a gate has an opening in the plate");
        assert!(found_plate, "the plate stands beside the hole");
    }

    #[test]
    fn lines_have_cables_folds_corridors_and_damage_has_debris() {
        let (_, l) = find_well(SEED, WellKind::Line, 8).expect("a line well");
        assert!(
            cell_pieces(SEED, l2_of(l.centre))
                .iter()
                .any(|p| p.kind == FeatureKind::CABLE)
        );
        let (_, f) = find_well(SEED, WellKind::Fold, 60).expect("a fold well");
        let s = f.fold[0];
        let mut p = s.start;
        let up_axis = (0..3).find(|&i| s.up[i] != 0).unwrap();
        p[up_axis] -= HALL + 1;
        assert!(solid_at(p), "the corridor floor");
        p[up_axis] += HALL;
        assert!(!solid_at(p), "the corridor is clear inside");
        let (_, d) = find_well(SEED, WellKind::Damaged, 60).expect("a damaged well");
        let mut any = false;
        for dz in -3..3 {
            for dy in -3..3 {
                for dx in -3..3 {
                    let l2 = l2_of(d.centre) + I64Vec3::new(dx, dy, dz);
                    any |= !cell_pieces(SEED, l2).is_empty();
                }
            }
        }
        assert!(any);
    }
}
