//! Quarters (QTR-01..16, `docs/proposals/the-web-ideas.md` §4): the character of a district.
//! Spec: `docs/spec/levels.md` §5W.37.
//!
//! A stratum's character has two to four dominant quarters (`Character::quarters`, SC-02). Each
//! L1 cell picks one of them (`QUARTER`, `0x031A`), so a quarter spans 4 km, and the structures
//! of its L2 cells take that quarter's dressing on their faces: sixteen recipes of boxes in the
//! lattice's frame, visual only, in the manner of `kit_ext`.

#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::character::Character;
use crate::hash::{hash, mix};
use crate::kit_ext::{Face, M, faces};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue::Tissue;

pub const NAMES: [&str; 16] = [
    "Dwelling warren",
    "Foundry",
    "Data",
    "Transit",
    "Archive",
    "Hydroponic",
    "Necropolis",
    "Administrative",
    "Bastion",
    "Dock",
    "Waterworks",
    "Observatory",
    "Market",
    "Chapel",
    "Hospital",
    "Machine",
];

/// The quarter of L1 cell `l1` in a stratum of character `chr`: one of its dominant quarters.
pub fn quarter_of(seed: WorldSeed, l1: I64Vec3, chr: &Character) -> u8 {
    // A Library region is all Archive (STG-01).
    if crate::strange::strange_of(seed, l1.div_euclid(I64Vec3::splat(8)))
        == Some(crate::strange::Strange::Library)
    {
        return 4;
    }
    let dominant: Vec<u8> = chr.dominant().collect();
    if dominant.is_empty() {
        return 0;
    }
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::QUARTER));
    dominant[rng.below(dominant.len() as u64) as usize]
}

/// One recipe of the quarter `q` on `f` (a face of the part from `lo` to `hi`, in metres).
fn recipe(m: &mut M<'_>, q: usize, rng: &mut Rng, f: &Face, lo: [f64; 3], hi: [f64; 3]) {
    let (t0, t1) = f.t;
    let (w, h) = (f.width(), f.height());
    let rand = |r: &mut Rng, a: f64, b: f64| a + r.next_f32() as f64 * (b - a).max(0.0);
    let none = FeatureKind::MISC;
    let lit = MaterialId::LIT_WINDOW;
    let glow = MaterialId::EMISSIVE;
    let warm = MaterialId::WARM;
    let metal = MaterialId::METAL;
    let conc = MaterialId::CONCRETE;
    match q {
        // QTR-01 dwelling warren: balconies and small lit windows in a dense grid.
        0 => {
            let (mut n, mut y) = (0, f.y.0 + 2.0);
            while y < f.y.1 - 3.0 && n < 16 {
                let mut x = t0 + 2.0;
                while x < t1 - 3.0 && n < 16 {
                    let (a, b) = f.bx(0.0, 1.2, x, x + 2.0, y, y + 0.2);
                    m.boxed(0, a, b, conc, none);
                    let (a, b) = f.bx(0.0, 0.1, x + 0.3, x + 1.3, y + 0.4, y + 2.0);
                    m.boxed(0, a, b, lit, FeatureKind::WINDOW);
                    x += 4.0;
                    n += 1;
                }
                y += 3.5;
            }
        }
        // QTR-02 foundry: chimneys, a glowing channel along the foot, vertical pipes.
        1 => {
            for _ in 0..2 {
                let x = rand(rng, lo[0] + 3.0, hi[0] - 7.0);
                let z = rand(rng, lo[2] + 3.0, hi[2] - 7.0);
                let hgt = 24.0 + 4.0 * rng.below(5) as f64;
                m.boxed(
                    1,
                    [x, hi[1], z],
                    [x + 4.0, hi[1] + hgt, z + 4.0],
                    metal,
                    none,
                );
                m.boxed(
                    1,
                    [x - 0.3, hi[1] + hgt, z - 0.3],
                    [x + 4.3, hi[1] + hgt + 0.6, z + 4.3],
                    glow,
                    none,
                );
            }
            let (a, b) = f.bx(0.0, 0.3, t0 + 1.0, t1 - 1.0, f.y.0 + 0.2, f.y.0 + 0.8);
            m.boxed(1, a, b, glow, none);
            let x = rand(rng, t0 + 2.0, t1 - 4.0);
            let (a, b) = f.bx(0.0, 0.9, x, x + 0.9, f.y.0, f.y.1);
            m.boxed(1, a, b, MaterialId::PIPE, FeatureKind::PIPE);
        }
        // QTR-03 data: racks of small lights, antennas.
        2 => {
            let mut y = f.y.0 + 2.0;
            let mut n = 0;
            while y < f.y.1 - 1.0 && n < 8 {
                let x = rand(rng, t0 + 1.0, t1 - 3.0);
                let (a, b) = f.bx(0.0, 0.2, x, x + 2.0, y, y + 0.25);
                m.boxed(2, a, b, glow, none);
                let (a, b) = f.bx(0.0, 0.1, t0 + 0.5, t1 - 0.5, y - 0.3, y - 0.2);
                m.boxed(2, a, b, metal, none);
                y += 1.2;
                n += 1;
            }
            let x = rand(rng, lo[0] + 2.0, hi[0] - 2.0);
            let z = rand(rng, lo[2] + 2.0, hi[2] - 2.0);
            m.boxed(
                2,
                [x, hi[1], z],
                [x + 0.3, hi[1] + 14.0, z + 0.3],
                metal,
                none,
            );
        }
        // QTR-04 transit: a platform edge, signal boards and catenary posts.
        3 => {
            let (a, b) = f.bx(0.0, 1.5, t0, t1, f.y.0, f.y.0 + 0.25);
            m.boxed(3, a, b, warm, none);
            let mut x = t0 + 4.0;
            while x < t1 - 3.0 {
                let (a, b) = f.bx(0.0, 0.2, x, x + 3.0, f.y.0 + 3.0, f.y.0 + 4.0);
                m.boxed(3, a, b, glow, none);
                let (a, b) = f.bx(1.0, 1.2, x + 1.4, x + 1.6, f.y.0, f.y.0 + 6.0);
                m.boxed(3, a, b, metal, none);
                x += 16.0;
            }
        }
        // QTR-04 archive: shelving: dividers every 2 m and a shelf every 2.5 m.
        4 => {
            let mut x = t0 + 1.0;
            let mut n = 0;
            while x < t1 - 1.0 && n < 12 {
                let (a, b) = f.bx(0.0, 0.4, x, x + 0.2, f.y.0 + 0.5, f.y.1 - 0.5);
                m.boxed(4, a, b, conc, none);
                x += 2.0;
                n += 1;
            }
            let mut y = f.y.0 + 2.5;
            while y < f.y.1 - 1.0 {
                let (a, b) = f.bx(0.0, 0.4, t0 + 1.0, t1 - 1.0, y, y + 0.15);
                m.boxed(4, a, b, MaterialId::GRATING, none);
                y += 2.5;
            }
        }
        // QTR-06 hydroponic: planter strips with grow-lights above.
        5 => {
            let mut y = f.y.0 + 3.0;
            while y < f.y.1 - 3.0 {
                let (a, b) = f.bx(0.0, 0.9, t0 + 2.0, t1 - 2.0, y, y + 0.6);
                m.boxed(5, a, b, warm, none);
                let (a, b) = f.bx(0.0, 0.5, t0 + 2.0, t1 - 2.0, y + 2.4, y + 2.5);
                m.boxed(5, a, b, glow, none);
                y += 4.0;
            }
        }
        // QTR-07 necropolis: a grid of niches with the odd candle.
        6 => {
            let (mut n, mut y) = (0, f.y.0 + 1.0);
            while y < f.y.1 - 2.0 && n < 20 {
                let mut x = t0 + 1.0;
                while x < t1 - 2.0 && n < 20 {
                    let (a, b) = f.bx(0.0, 0.1, x, x + 1.0, y, y + 1.0);
                    m.boxed(6, a, b, metal, none);
                    if rng.below(8) == 0 {
                        let (a, b) = f.bx(0.1, 0.2, x + 0.4, x + 0.6, y + 0.1, y + 0.4);
                        m.boxed(6, a, b, warm, none);
                    }
                    x += 2.0;
                    n += 1;
                }
                y += 2.0;
            }
        }
        // QTR-08 administrative: a colonnade and an entablature.
        7 => {
            let mut x = t0 + 3.0;
            while x < t1 - 3.0 {
                let (a, b) = f.bx(2.0, 3.0, x, x + 1.0, f.y.0, f.y.0 + 8.0);
                m.boxed(7, a, b, conc, none);
                x += 6.0;
            }
            let (a, b) = f.bx(0.0, 3.2, t0 + 2.0, t1 - 2.0, f.y.0 + 8.0, f.y.0 + 9.0);
            m.boxed(7, a, b, conc, none);
        }
        // QTR-09 bastion: merlons along the roof edge and buttresses.
        8 => {
            let mut x = t0;
            while x < t1 - 2.0 {
                let (a, b) = f.bx(0.0, 2.0, x, x + 2.0, f.y.1, f.y.1 + 2.0);
                m.boxed(8, a, b, conc, none);
                x += 4.0;
            }
            let mut x = t0 + 5.0;
            while x < t1 - 5.0 {
                let (a, b) = f.bx(0.0, 3.0, x, x + 3.0, f.y.0, (f.y.0 + 8.0).min(f.y.1));
                m.boxed(8, a, b, conc, none);
                x += 10.0;
            }
        }
        // QTR-10 dock: a gantry crane on the roof with its jib out over the face.
        9 => {
            let x = rand(rng, t0 + 3.0, t1 - 3.0);
            let (a, b) = f.bx(-1.0, 1.0, x - 3.0, x - 1.0, f.y.1, f.y.1 + 12.0);
            m.boxed(9, a, b, metal, none);
            let (a, b) = f.bx(-1.0, 1.0, x + 1.0, x + 3.0, f.y.1, f.y.1 + 12.0);
            m.boxed(9, a, b, metal, none);
            let (a, b) = f.bx(-1.0, 22.0, x - 3.0, x + 3.0, f.y.1 + 12.0, f.y.1 + 13.0);
            m.boxed(9, a, b, metal, none);
            let (a, b) = f.bx(0.0, 0.6, t0 + 2.0, t0 + 2.6, f.y.0, f.y.0 + 1.0);
            m.boxed(9, a, b, metal, none);
        }
        // QTR-11 waterworks: tanks on the roof, manifold pipes at the foot.
        10 => {
            for _ in 0..3 {
                let x = rand(rng, lo[0] + 1.0, hi[0] - 9.0);
                let z = rand(rng, lo[2] + 1.0, hi[2] - 7.0);
                m.boxed(
                    10,
                    [x, hi[1], z],
                    [x + 8.0, hi[1] + 6.0, z + 6.0],
                    MaterialId::PIPE,
                    none,
                );
            }
            for k in 0..3 {
                let y = f.y.0 + 1.0 + 1.0 * k as f64;
                let (a, b) = f.bx(0.0, 0.5, t0 + 1.0, t1 - 1.0, y, y + 0.4);
                m.boxed(10, a, b, MaterialId::PIPE, FeatureKind::PIPE);
            }
        }
        // QTR-12 observatory: a stepped dome with a lit slit and a telescope.
        11 => {
            let (cx, cz) = (f64::midpoint(lo[0], hi[0]), f64::midpoint(lo[2], hi[2]));
            for (k, half) in [6.0, 4.5, 3.0].into_iter().enumerate() {
                let y = hi[1] + 2.0 * k as f64;
                m.boxed(
                    11,
                    [cx - half, y, cz - half],
                    [cx + half, y + 2.0, cz + half],
                    conc,
                    none,
                );
            }
            m.boxed(
                11,
                [cx - 0.5, hi[1] + 0.5, cz - 6.2],
                [cx + 0.5, hi[1] + 5.5, cz - 5.9],
                glow,
                none,
            );
            m.boxed(
                11,
                [cx - 0.3, hi[1] + 6.0, cz - 0.3],
                [cx + 0.3, hi[1] + 6.6, cz + 9.0],
                metal,
                none,
            );
        }
        // QTR-13 market: awnings over stalls, with lamps on strings.
        12 => {
            let mut x = t0 + 2.0;
            while x < t1 - 4.0 {
                let (a, b) = f.bx(0.0, 3.0, x, x + 3.0, f.y.0 + 3.2, f.y.0 + 3.5);
                m.boxed(12, a, b, warm, none);
                let (a, b) = f.bx(0.5, 2.5, x + 0.3, x + 2.7, f.y.0, f.y.0 + 1.0);
                m.boxed(12, a, b, conc, none);
                let (a, b) = f.bx(2.5, 2.9, x + 1.3, x + 1.7, f.y.0 + 2.6, f.y.0 + 3.0);
                m.boxed(12, a, b, glow, none);
                x += 4.0;
            }
        }
        // QTR-14 chapel: tall narrow windows and a spire.
        13 => {
            let mut x = t0 + 2.0;
            while x < t1 - 3.0 {
                let (a, b) = f.bx(
                    0.0,
                    0.15,
                    x,
                    x + 1.0,
                    f.y.0 + 2.0,
                    (f.y.0 + 10.0).min(f.y.1 - 1.0),
                );
                m.boxed(13, a, b, lit, FeatureKind::WINDOW);
                x += 4.0;
            }
            let (cx, cz) = (f64::midpoint(lo[0], hi[0]), f64::midpoint(lo[2], hi[2]));
            for (k, (half, up)) in [(3.0, 4.0), (2.0, 4.0), (1.0, 7.0)].into_iter().enumerate() {
                let y = hi[1] + 4.0 * k as f64;
                m.boxed(
                    13,
                    [cx - half, y, cz - half],
                    [cx + half, y + up, cz + half],
                    conc,
                    none,
                );
            }
        }
        // QTR-15 hospital: a lit cross and a ramp.
        14 => {
            let x = f64::midpoint(t0, t1);
            let y = f.y.0 + (h * 0.6).max(4.0).min(h - 4.0);
            let (a, b) = f.bx(0.0, 0.25, x - 3.0, x + 3.0, y - 0.5, y + 0.5);
            m.boxed(14, a, b, glow, none);
            let (a, b) = f.bx(0.0, 0.25, x - 0.5, x + 0.5, y - 3.0, y + 3.0);
            m.boxed(14, a, b, glow, none);
            let (a, b) = f.bx(0.0, 4.0, x - 2.0, x + 2.0, f.y.0, f.y.0 + 0.4);
            m.boxed(14, a, b, conc, none);
        }
        // QTR-16 machine: a great wheel on the face and a bank of pistons on the roof.
        _ => {
            let x = f64::midpoint(t0, t1);
            let y = f.y.0 + h / 2.0;
            let r = (w.min(h) / 4.0).clamp(2.0, 8.0);
            let (a, b) = f.bx(0.0, 0.8, x - r, x + r, y - 0.5, y + 0.5);
            m.boxed(15, a, b, metal, none);
            let (a, b) = f.bx(0.0, 0.8, x - 0.5, x + 0.5, y - r, y + r);
            m.boxed(15, a, b, metal, none);
            let (a, b) = f.bx(0.0, 1.0, x - 1.0, x + 1.0, y - 1.0, y + 1.0);
            m.boxed(15, a, b, warm, none);
            for k in 0..5 {
                let px = lo[0] + 2.0 + 2.0 * k as f64;
                m.boxed(
                    15,
                    [px, hi[1], lo[2] + 2.0],
                    [px + 1.0, hi[1] + 4.0 + (k % 3) as f64 * 2.0, lo[2] + 3.0],
                    metal,
                    none,
                );
            }
        }
    }
}

/// The dressing of the quarter on the structures of `t`, with the quarter of each piece.
pub fn dress(t: &Tissue) -> Vec<(usize, Primitive)> {
    let Some(chr) = t.character else {
        return Vec::new();
    };
    let l1 = t.cell.coords.div_euclid(I64Vec3::splat(8));
    dress_as(t, quarter_of(t.seed, l1, &chr) as usize)
}

/// The dressing of quarter `q` on the structures of `t`, whatever quarter the cell is in.
pub fn dress_as(t: &Tissue, q: usize) -> Vec<(usize, Primitive)> {
    let pitch = t.lattice.pitch as f64;
    let base = hash(t.seed.0, Level::L2, t.cell.coords, salt::QUARTER);
    let mut m = M::new(t);
    for (si, s) in t.structures.iter().enumerate() {
        for (pi, b) in s.parts.iter().enumerate() {
            let mut rng = Rng::new(mix(base ^ ((si as u64) << 32 | pi as u64)));
            let lo = [0, 1, 2].map(|k| f64::from(b.lo[k]) * pitch);
            let hi = [0, 1, 2].map(|k| f64::from(b.hi[k]) * pitch);
            // Draws are made for every part, whether or not it takes the dressing.
            let dressed = rng.below(2) == 0;
            let face_picks = [rng.below(4) as usize, rng.below(4) as usize];
            if !dressed || hi[1] - lo[1] < 8.0 {
                continue;
            }
            let fs = faces(lo, hi);
            for face in face_picks {
                recipe(&mut m, q, &mut rng, &fs[face], lo, hi);
            }
        }
    }
    let M { em, tags: _ } = m;
    let cell_q = 512 * Q_PER_M;
    let _: Option<Axis> = None;
    let _ = (PrimFlags::NONE, cell_q);
    let mut out: Vec<(usize, Primitive)> = em
        .out
        .into_iter()
        .filter(|p| {
            let b = p.aabb();
            b.max.x > 0
                && b.min.x < cell_q
                && b.max.y > 0
                && b.min.y < cell_q
                && b.max.z > 0
                && b.min.z < cell_q
        })
        .map(|p| (q, p))
        .collect();
    // Nothing fills a doorway or a bored passage.
    let keep_out: Vec<crate::prim::QAabb> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .map(Primitive::aabb)
        .collect();
    out.retain(|(_, p)| {
        let b = p.aabb();
        !keep_out.iter().any(|k| {
            b.min.x < k.max.x + 8
                && b.max.x > k.min.x - 8
                && b.min.y < k.max.y + 8
                && b.max.y > k.min.y - 8
                && b.min.z < k.max.z + 8
                && b.max.z > k.min.z - 8
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellKey;
    use crate::tissue::TissueCache;

    /// QTR-01..16: every quarter is a dominant quarter somewhere, and every recipe builds.
    #[test]
    fn every_quarter_occurs_and_dresses() {
        let seed = WorldSeed(5);
        let mut dominant = [false; 16];
        for l0x in -20..20 {
            for k in 0..4 {
                let chr = crate::character::character(seed, I64Vec3::new(l0x, 0, 3), k);
                for q in chr.dominant() {
                    dominant[usize::from(q)] = true;
                }
            }
        }
        assert!(dominant.iter().all(|&d| d), "{dominant:?}");
        let cache = TissueCache::default();
        let mut seen = [0usize; 16];
        for k in 0..6i64 {
            let key = CellKey::new(Level::L2, I64Vec3::new(k % 3 * 8 - 12, 1, k / 3 * 8 + 2));
            let t = crate::tissue::tissue_cell(seed, &cache, key);
            for (q, count) in seen.iter_mut().enumerate() {
                for (qq, p) in dress_as(&t, q) {
                    assert_eq!(qq, q);
                    *count += 1;
                    assert!(p.aabb().max.x > 0);
                }
            }
        }
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    }

    #[test]
    fn a_quarter_spans_its_l1_cell() {
        let seed = WorldSeed(5);
        let cache = TissueCache::default();
        let l1 = I64Vec3::new(3, 1, -2);
        let mut quarters = std::collections::HashSet::new();
        for k in 0..16 {
            let l2 = l1 * 8 + I64Vec3::new(k % 8, 2, k / 8 + 1);
            let t = crate::tissue::tissue_cell(seed, &cache, CellKey::new(Level::L2, l2));
            if let Some(c) = t.character {
                quarters.insert(quarter_of(seed, l1, &c));
            }
        }
        assert!(quarters.len() <= 1, "{quarters:?}");
    }
}
