//! Payoffs, batch 2 (PAY-01..10, `docs/proposals/the-web-ideas.md` §10): installations at the end
//! of a dead end. Spec: `docs/spec/levels.md` §5W.42.
//!
//! A leaf of the route tree sometimes (one in four) ends in one of ten installations instead of
//! a balcony or a lamp. Each stands in the slot's 32 m cube, in metres from the slot's centre on
//! its floor, away from the middle where the walkway ends; most are not solid.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::glyphs::{Glyph, alphabet, name_of};
use crate::prim::{FeatureKind, MaterialId, PrimFlags};
use crate::rng::Rng;
use crate::tissue::Tissue;

pub const TAGS: [&str; 10] = [
    "PAY-01", "PAY-02", "PAY-03", "PAY-04", "PAY-05", "PAY-06", "PAY-07", "PAY-08", "PAY-09",
    "PAY-10",
];

pub type Spec = ([f64; 3], [f64; 3], MaterialId, FeatureKind, PrimFlags);

fn solid(lo: [f64; 3], hi: [f64; 3], m: MaterialId) -> Spec {
    (lo, hi, m, FeatureKind::MISC, PrimFlags::NONE)
}

fn free(lo: [f64; 3], hi: [f64; 3], m: MaterialId) -> Spec {
    (lo, hi, m, FeatureKind::MISC, PrimFlags::NO_COLLIDE)
}

/// The boxes of an anamorphic glyph (PAY-01): for every lit cell of `glyph`, a box on the ray
/// from `focus` to the cell's place on a plane `dist` metres away along `dir` (a horizontal unit
/// axis), at a depth of 0.5 to 1.25 of that distance and sized with the depth, so that from the
/// focal point the boxes cover the glyph's cells exactly and from anywhere else they are
/// scattered. `cell` is the angular size of a glyph cell at the plane, in metres there.
pub fn anamorphic(
    focus: [f64; 3],
    dir: [f64; 2],
    dist: f64,
    cell: f64,
    glyph: Glyph,
    rng: &mut Rng,
) -> Vec<([f64; 3], f64)> {
    let side = [-dir[1], dir[0]];
    let mut out = Vec::new();
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..7 {
            if bits >> col & 1 == 0 {
                continue;
            }
            // The cell's centre on the plane, left to right and top to bottom.
            let u = (f64::from(col) - 3.0) * cell;
            // Raised so that the lowest row is above the eye: nothing goes under the floor.
            let v = (3.0 - row as f64 + 3.5) * cell;
            let depth = 0.5 + 0.75 * f64::from(rng.next_u32() >> 8) / f64::from(1u32 << 24);
            let (k, size) = (depth, 0.9 * cell * depth);
            let p = [
                focus[0] + k * (dist * dir[0] + u * side[0]),
                focus[1] + k * v,
                focus[2] + k * (dist * dir[1] + u * side[1]),
            ];
            out.push((p, size));
        }
    }
    out
}

/// The installation `kind` (0..10, PAY-01..10) at the slot with centre `(cx, cz)` and floor `y`,
/// as specs in lattice metres, or `None` when it needs something the slot lacks.
pub fn installation(
    t: &Tissue,
    kind: usize,
    (cx, y, cz): (f64, f64, f64),
    rng: &mut Rng,
) -> Option<Vec<Spec>> {
    let mut out: Vec<Spec> = Vec::new();
    let conc = MaterialId::CONCRETE;
    let metal = MaterialId::METAL;
    let warm = MaterialId::WARM;
    let glow = MaterialId::EMISSIVE;
    let l1 = t.cell.coords.div_euclid(I64Vec3::splat(8));
    let glyphs = alphabet(t.seed);
    let name = name_of(t.seed, l1);
    // Away from the middle, where the walkway ends: the room's corner.
    let (ox, oz) = (cx + 9.0, cz + 9.0);
    match kind {
        // PAY-01: an anamorphic glyph: the first glyph of the district's name, resolving from
        // the point where the walkway ends, scattered in a space of 16 m.
        0 => {
            let focus = [cx, y + 1.65, cz];
            for (p, s) in anamorphic(
                focus,
                [0.0, -1.0],
                12.0,
                1.0,
                glyphs[usize::from(name[0])],
                rng,
            ) {
                out.push(free(
                    [p[0] - s / 2.0, p[1] - s / 2.0, p[2] - s / 2.0],
                    [p[0] + s / 2.0, p[1] + s / 2.0, p[2] + s / 2.0],
                    glow,
                ));
            }
        }
        // PAY-02: a cartographer's room: a table with a relief of the real fill of this level of
        // the district, one 1 m column to the L2 cell, heights from the fill field.
        1 => {
            let l2 = t.cell.coords - l1 * 8;
            let fill = crate::l1_fill::fill_field(t.seed, l1);
            out.push(solid(
                [ox - 5.0, y, oz - 5.0],
                [ox + 3.0, y + 0.9, oz + 3.0],
                metal,
            ));
            for i in 0..8usize {
                for j in 0..8usize {
                    let f = f64::from(fill[((j * 8 + l2.y as usize) * 8) + i]) / 255.0;
                    let h = 0.05 + 0.5 * f;
                    let (x, z) = (ox - 4.5 + i as f64, oz - 4.5 + j as f64);
                    out.push(solid(
                        [x, y + 0.9, z],
                        [x + 0.9, y + 0.9 + h, z + 0.9],
                        if f > 0.5 { warm } else { conc },
                    ));
                }
            }
        }
        // PAY-03: a focal window: a wall with a slit that frames the knot of the district.
        2 => {
            let sk = crate::l1_artery::skeleton(t.seed, l1)?;
            let k = crate::l1_artery::coords(usize::from(sk.knot));
            let here = [t.cell.coords.x - l1.x * 8, t.cell.coords.z - l1.z * 8];
            let (dx, dz) = (k[0] - here[0], k[2] - here[1]);
            // The wall stands on the side of the dominant horizontal direction to the knot.
            let (ax, az) = if dx.abs() >= dz.abs() {
                (dx.signum() as f64, 0.0)
            } else {
                (0.0, dz.signum() as f64)
            };
            if ax == 0.0 && az == 0.0 {
                return None;
            }
            let (wx, wz) = (cx + ax * 12.0, cz + az * 12.0);
            // 12 m wide along the other axis, 6 m tall, a hole of 2 by 2.5 m at eye height.
            let (px, pz) = (az.abs(), ax.abs());
            let c = |a: f64, b: f64, h0: f64, h1: f64| {
                solid(
                    [
                        wx + px * a - ax.abs() * 0.4,
                        y + h0,
                        wz + pz * a - az.abs() * 0.4,
                    ],
                    [
                        wx + px * b + ax.abs() * 0.4,
                        y + h1,
                        wz + pz * b + az.abs() * 0.4,
                    ],
                    conc,
                )
            };
            out.push(c(-6.0, -1.0, 0.0, 6.0));
            out.push(c(1.0, 6.0, 0.0, 6.0));
            out.push(c(-1.0, 1.0, 0.0, 0.4));
            out.push(c(-1.0, 1.0, 2.9, 6.0));
            out.push(free(
                [wx - px - ax.abs() * 0.5, y + 0.4, wz - pz - az.abs() * 0.5],
                [
                    wx + px * 1.0 + ax.abs() * 0.5,
                    y + 0.5,
                    wz + pz * 1.0 + az.abs() * 0.5,
                ],
                glow,
            ));
        }
        // PAY-04: the previous walker: a figure sitting against the corner, a lantern beside.
        3 => {
            out.push(solid(
                [ox - 0.3, y, oz - 0.3],
                [ox + 0.3, y + 0.45, oz + 0.5],
                conc,
            ));
            out.push(free(
                [ox - 0.2, y + 0.45, oz - 0.2],
                [ox + 0.2, y + 1.0, oz + 0.15],
                metal,
            ));
            out.push(free(
                [ox - 0.13, y + 1.0, oz - 0.13],
                [ox + 0.13, y + 1.26, oz + 0.13],
                metal,
            ));
            out.push(free(
                [ox - 0.2, y + 0.3, oz + 0.5],
                [ox + 0.2, y + 0.4, oz + 1.1],
                metal,
            ));
            out.push(free(
                [ox - 0.7, y, oz + 0.2],
                [ox - 0.5, y + 0.3, oz + 0.4],
                warm,
            ));
        }
        // PAY-05: a lit room in a dark district: walls, a lit window and a warm lamp inside.
        4 => {
            let (x0, z0) = (cx + 5.0, cz - 3.0);
            for (lo, hi) in [
                ([x0, 0.0, z0], [x0 + 6.0, 3.5, z0 + 0.3]),
                ([x0, 0.0, z0 + 5.7], [x0 + 6.0, 3.5, z0 + 6.0]),
                ([x0 + 5.7, 0.0, z0 + 0.3], [x0 + 6.0, 3.5, z0 + 5.7]),
                ([x0, 3.5, z0], [x0 + 6.0, 3.8, z0 + 6.0]),
                ([x0, 0.0, z0 + 0.3], [x0 + 0.3, 1.0, z0 + 5.7]),
                ([x0, 2.4, z0 + 0.3], [x0 + 0.3, 3.5, z0 + 5.7]),
            ] {
                out.push(solid(
                    [lo[0], y + lo[1], lo[2]],
                    [hi[0], y + hi[1], hi[2]],
                    conc,
                ));
            }
            out.push(free(
                [x0, y + 1.0, z0 + 1.0],
                [x0 + 0.2, y + 2.4, z0 + 5.0],
                MaterialId::LIT_WINDOW,
            ));
            out.push(free(
                [x0 + 2.5, y + 2.6, z0 + 2.5],
                [x0 + 3.5, y + 3.4, z0 + 3.5],
                warm,
            ));
        }
        // PAY-06: a collection: a room of 400 lamps hung at their own heights in a grid.
        5 => {
            let (x0, z0) = (cx - 10.0, cz - 10.0);
            out.push(solid(
                [x0, y + 8.0, z0],
                [x0 + 20.0, y + 8.4, z0 + 20.0],
                conc,
            ));
            for i in 0..20 {
                for j in 0..20 {
                    let drop = 0.5 + 3.0 * f64::from(rng.next_u32() >> 8) / f64::from(1u32 << 24);
                    let (x, z) = (x0 + f64::from(i) + 0.4, z0 + f64::from(j) + 0.4);
                    out.push(free(
                        [x + 0.1, y + 8.0 - drop, z + 0.1],
                        [x + 0.14, y + 8.0, z + 0.14],
                        metal,
                    ));
                    out.push(free(
                        [x, y + 8.0 - drop - 0.3, z],
                        [x + 0.3, y + 8.0 - drop, z + 0.3],
                        glow,
                    ));
                }
            }
        }
        // PAY-07: a message: a wall panel with eight glyphs in two rows (a line of the City).
        6 => {
            let (x0, z0) = (cx - 7.0, cz + 11.0);
            out.push(solid([x0, y, z0], [x0 + 14.0, y + 6.0, z0 + 0.4], metal));
            for k in 0..8 {
                let g = &glyphs[((name[k % 3] as usize + 7 * k) % 32).min(31)];
                let (gx, gy) = (
                    x0 + 0.8 + (k % 4) as f64 * 3.3,
                    y + 3.2 - (k / 4) as f64 * 2.6,
                );
                for (row, bits) in g.iter().enumerate() {
                    for col in 0..7 {
                        if bits >> col & 1 == 1 {
                            let (u, v) = (gx + f64::from(col) * 0.38, gy + (6 - row) as f64 * 0.3);
                            out.push(free([u, v, z0 - 0.1], [u + 0.34, v + 0.26, z0], glow));
                        }
                    }
                }
            }
        }
        // PAY-08: an echo chamber: a ring of eight wall slabs and a roof, one gap for a door.
        7 => {
            for k in 0..8u8 {
                if k == 0 {
                    continue;
                }
                let (a, b) = crate::orient::heading(k * 8);
                let n = f64::from(a * a + b * b).sqrt();
                let (px, pz) = (10.0 * f64::from(a) / n, 10.0 * f64::from(b) / n);
                out.push(solid(
                    [cx + px - 2.2, y, cz + pz - 2.2],
                    [cx + px + 2.2, y + 7.0, cz + pz + 2.2],
                    conc,
                ));
            }
            out.push(solid(
                [cx - 12.0, y + 7.0, cz - 12.0],
                [cx + 12.0, y + 7.6, cz + 12.0],
                conc,
            ));
        }
        // PAY-09: a seat: a bench and its back, facing out. Sometimes it is just that.
        8 => {
            out.push(solid(
                [ox - 1.2, y, oz - 0.3],
                [ox + 1.2, y + 0.45, oz + 0.3],
                conc,
            ));
            out.push(solid(
                [ox - 1.2, y + 0.45, oz + 0.25],
                [ox + 1.2, y + 1.1, oz + 0.35],
                conc,
            ));
        }
        // PAY-10: a bell in a shaft: four walls of 5 m, a beam, a rope and a bell of stacked boxes.
        _ => {
            let (x0, z0) = (cx - 12.0, cz - 12.0);
            for (lo, hi) in [
                ([x0, 0.0, z0], [x0 + 0.5, 22.0, z0 + 5.0]),
                ([x0 + 4.5, 0.0, z0], [x0 + 5.0, 22.0, z0 + 5.0]),
                ([x0 + 0.5, 0.0, z0], [x0 + 4.5, 22.0, z0 + 0.5]),
                ([x0 + 0.5, 0.0, z0 + 4.5], [x0 + 4.5, 22.0, z0 + 5.0]),
            ] {
                out.push(solid(
                    [lo[0], y + lo[1], lo[2]],
                    [hi[0], y + hi[1], hi[2]],
                    conc,
                ));
            }
            out.push(solid(
                [x0 + 0.5, y + 20.0, z0 + 2.2],
                [x0 + 4.5, y + 20.4, z0 + 2.8],
                metal,
            ));
            out.push(free(
                [x0 + 2.45, y + 14.0, z0 + 2.45],
                [x0 + 2.55, y + 20.0, z0 + 2.55],
                metal,
            ));
            for (k, half) in [0.5, 0.8, 1.1, 1.3].into_iter().enumerate() {
                let h = 13.0 - f64::from(k as u32) * 0.8;
                out.push(free(
                    [x0 + 2.5 - half, y + h, z0 + 2.5 - half],
                    [x0 + 2.5 + half, y + h + 0.8, z0 + 2.5 + half],
                    warm,
                ));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PAY-01: from the focal point the boxes sit on the rays through the glyph's cells, and
    /// cover them; from anywhere else they do not line up.
    #[test]
    fn an_anamorphic_glyph_resolves_from_its_focal_point() {
        let glyph = alphabet(crate::seed::WorldSeed(1))[5];
        let focus = [10.0, 1.65, 20.0];
        let (dist, cell) = (12.0, 1.0);
        let boxes = anamorphic(focus, [0.0, -1.0], dist, cell, glyph, &mut Rng::new(3));
        let lit: u32 = glyph.iter().map(|r| r.count_ones()).sum();
        assert_eq!(boxes.len() as u32, lit);
        for (p, size) in &boxes {
            // Project from the focus onto the plane `dist` away along (0, -1): the point lands
            // on a cell centre and the box fills 0.9 of the cell.
            let rel = [p[0] - focus[0], p[1] - focus[1], p[2] - focus[2]];
            let t = dist / -rel[2];
            let (u, v) = (rel[0] * t, rel[1] * t);
            // `side` is (-dir.z, dir.x) = (1, 0): u runs along +x.
            let (col, row) = (u / cell + 3.0, 3.0 + 3.5 - v / cell);
            assert!(
                (col - col.round()).abs() < 1e-9 && (row - row.round()).abs() < 1e-9,
                "{col} {row}"
            );
            assert!((size * t - 0.9 * cell).abs() < 1e-9);
            let (c, r) = (col.round() as usize, row.round() as usize);
            assert_eq!(glyph[r] >> c & 1, 1, "the box is not on a lit cell");
        }
        // Moved aside by 3 m, the boxes no longer fall on cells.
        let off = [focus[0] + 3.0, focus[1], focus[2]];
        let scattered = boxes
            .iter()
            .filter(|(p, _)| {
                let rel = [p[0] - off[0], p[1] - off[1], p[2] - off[2]];
                let t = dist / -rel[2];
                let col = rel[0] * t / cell + 3.0;
                (col - col.round()).abs() > 0.05
            })
            .count();
        assert!(
            scattered * 2 > boxes.len(),
            "{scattered} of {} off their cells",
            boxes.len()
        );
    }

    /// PAY-01..10 each build in a real slot and stay inside it (16 m round the middle, and the
    /// 32 m cube above the floor), and the cartographer's table shows the real fill.
    #[test]
    fn every_installation_builds_inside_its_slot() {
        use crate::cell::{CellKey, Level};
        use crate::tissue::{TissueCache, tissue_cell};
        let seed = crate::seed::WorldSeed(2);
        let cache = TissueCache::default();
        let mut made = [0usize; 10];
        for k in 0..6i64 {
            let key = CellKey::new(Level::L2, I64Vec3::new(k % 3 * 8 + 3, 2, k / 3 * 8 + 5));
            let t = tissue_cell(seed, &cache, key);
            for (kind, count) in made.iter_mut().enumerate() {
                let (cx, y, cz) = (256.0, 160.0, 256.0);
                let Some(specs) = installation(&t, kind, (cx, y, cz), &mut Rng::new(kind as u64))
                else {
                    continue;
                };
                *count += 1;
                assert!(!specs.is_empty(), "{}", TAGS[kind]);
                for (lo, hi, _, _, _) in specs {
                    for (a, c) in [(0, cx), (2, cz)] {
                        assert!(
                            lo[a] >= c - 16.5 && hi[a] <= c + 16.5,
                            "{} {lo:?} {hi:?}",
                            TAGS[kind]
                        );
                    }
                    assert!(
                        lo[1] >= y - 0.01 && hi[1] <= y + 32.0,
                        "{} {lo:?} {hi:?}",
                        TAGS[kind]
                    );
                }
            }
        }
        assert!(made.iter().all(|&n| n > 0), "{made:?}");
        // PAY-02: the table's columns follow the district's fill: more fill, taller column.
        let key = CellKey::new(Level::L2, I64Vec3::new(3, 2, 5));
        let t = tissue_cell(seed, &cache, key);
        let specs = installation(&t, 1, (256.0, 160.0, 256.0), &mut Rng::new(1)).unwrap();
        let l1 = key.coords.div_euclid(I64Vec3::splat(8));
        let fill = crate::l1_fill::fill_field(seed, l1);
        let layer = (key.coords - l1 * 8).y as usize;
        let heights: Vec<f64> = specs[1..]
            .iter()
            .map(|(lo, hi, ..)| hi[1] - lo[1])
            .collect();
        assert_eq!(heights.len(), 64);
        for j in 0..8usize {
            for i in 0..8usize {
                let f = f64::from(fill[(j * 8 + layer) * 8 + i]) / 255.0;
                assert!((heights[i * 8 + j] - (0.05 + 0.5 * f)).abs() < 1e-9);
            }
        }
    }
}
