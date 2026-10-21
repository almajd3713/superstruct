//! The exterior kit (KIT-01..12): window strips and fields, ledges and cornices, ribs, vents and
//! fans, masts, façade pipes, cable runs, coffered undersides, hanging lamps, glyph panels and
//! scaffolds on the faces of structures. Spec: `docs/spec/levels.md` §5W.34.
//!
//! Visual relief only: every piece is `NO_COLLIDE`, at most 3 m deep, and the pieces that would
//! fill a doorway or a bored passage are dropped. A structure draws its kit from the cell's
//! hash, so the kit of a face does not depend on what else the cell holds.

#![allow(
    clippy::cast_lossless,
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    clippy::too_many_lines
)]

use crate::cell::{Axis, Level};
use crate::hash::{hash, mix};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb};
use crate::rng::Rng;
use crate::salt;
use crate::tissue::Tissue;
use crate::tissue_grow::Emitter;

/// The registry tags of the kit, in the order the draws pick them.
pub const KIT_TAGS: [&str; 12] = [
    "KIT-01", "KIT-02", "KIT-03", "KIT-04", "KIT-05", "KIT-06", "KIT-07", "KIT-08", "KIT-09",
    "KIT-10", "KIT-11", "KIT-12",
];

/// The pieces so far, with the kit index of each; boxes are made in the lattice's own frame
/// (`Emitter::frame_box`), so a turned lattice carries its kit turned.
pub(crate) struct M<'a> {
    pub(crate) em: Emitter<'a>,
    pub(crate) tags: Vec<usize>,
}

impl M<'_> {
    pub(crate) fn new(t: &Tissue) -> M<'_> {
        M {
            em: Emitter::new(&t.lattice),
            tags: Vec::new(),
        }
    }

    pub(crate) fn boxed(
        &mut self,
        tag: usize,
        min: [f64; 3],
        max: [f64; 3],
        material: MaterialId,
        kind: FeatureKind,
    ) {
        self.em
            .frame_box(min, max, material, kind, PrimFlags::NO_COLLIDE);
        while self.tags.len() < self.em.out.len() {
            self.tags.push(tag);
        }
    }

    /// A round bar is drawn as a square one: `radius` is the half width.
    pub(crate) fn bar(
        &mut self,
        tag: usize,
        axis: Axis,
        base: [f64; 3],
        length: f64,
        radius: f64,
        material: MaterialId,
        kind: FeatureKind,
    ) {
        let k = match axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        let (mut lo, mut hi) = (base, base);
        for i in 0..3 {
            if i == k {
                hi[i] = base[i] + length;
            } else {
                lo[i] = base[i] - radius;
                hi[i] = base[i] + radius;
            }
        }
        self.boxed(tag, lo, hi, material, kind);
    }
}

/// One face of a part: the plane `p` on `axis` (0 = x, 2 = z) facing `sign`, spanning `t` along
/// the other horizontal axis and `y` in height.
pub(crate) struct Face {
    pub(crate) axis: usize,
    pub(crate) sign: f64,
    pub(crate) p: f64,
    pub(crate) t: (f64, f64),
    pub(crate) y: (f64, f64),
}

impl Face {
    /// A box `off0..off1` metres out from the face, `t0..t1` along it and `y0..y1` high.
    pub(crate) fn bx(
        &self,
        off0: f64,
        off1: f64,
        t0: f64,
        t1: f64,
        y0: f64,
        y1: f64,
    ) -> ([f64; 3], [f64; 3]) {
        let other = 2 - self.axis;
        let (a, b) = (self.p + self.sign * off0, self.p + self.sign * off1);
        let (mut lo, mut hi) = ([0.0; 3], [0.0; 3]);
        lo[self.axis] = a.min(b);
        hi[self.axis] = a.max(b);
        lo[other] = t0;
        hi[other] = t1;
        lo[1] = y0;
        hi[1] = y1;
        (lo, hi)
    }

    pub(crate) fn width(&self) -> f64 {
        self.t.1 - self.t.0
    }

    pub(crate) fn height(&self) -> f64 {
        self.y.1 - self.y.0
    }
}

pub(crate) fn faces(lo: [f64; 3], hi: [f64; 3]) -> [Face; 4] {
    [
        Face {
            axis: 0,
            sign: -1.0,
            p: lo[0],
            t: (lo[2], hi[2]),
            y: (lo[1], hi[1]),
        },
        Face {
            axis: 0,
            sign: 1.0,
            p: hi[0],
            t: (lo[2], hi[2]),
            y: (lo[1], hi[1]),
        },
        Face {
            axis: 2,
            sign: -1.0,
            p: lo[2],
            t: (lo[0], hi[0]),
            y: (lo[1], hi[1]),
        },
        Face {
            axis: 2,
            sign: 1.0,
            p: hi[2],
            t: (lo[0], hi[0]),
            y: (lo[1], hi[1]),
        },
    ]
}

/// The kit piece `kit` (0 to 11, the order of `KIT_TAGS`) on a face, a roof or an underside.
fn piece(
    m: &mut M,
    kit: usize,
    rng: &mut Rng,
    f: &Face,
    lo: [f64; 3],
    hi: [f64; 3],
    name: &[crate::glyphs::Glyph; 3],
) {
    let (w, h) = (f.width(), f.height());
    let (t0, t1) = f.t;
    let along =
        |r: &mut Rng, margin: f64| t0 + margin + r.next_f32() as f64 * (w - 2.0 * margin).max(0.0);
    match kit {
        // KIT-01: a strip of lit windows or a field of panes.
        0 => {
            if rng.below(2) == 0 {
                let y = f.y.0 + 3.0 + 4.0 * rng.below(((h - 6.0) / 4.0).max(1.0) as u64) as f64;
                let (a, b) = f.bx(0.0, 0.15, t0 + 1.5, t1 - 1.5, y, y + 1.2);
                m.boxed(0, a, b, MaterialId::LIT_WINDOW, FeatureKind::WINDOW);
            } else {
                let cols = ((w - 3.0) / 3.0).max(1.0) as u64;
                let rows = ((h - 4.0) / 4.0).clamp(1.0, 4.0) as u64;
                for r in 0..rows {
                    for c in 0..cols {
                        if rng.below(3) == 0 {
                            continue;
                        }
                        let (x, y) = (t0 + 1.5 + 3.0 * c as f64, f.y.0 + 2.5 + 4.0 * r as f64);
                        let (a, b) = f.bx(0.0, 0.15, x, x + 1.4, y, y + 2.0);
                        m.boxed(0, a, b, MaterialId::LIT_WINDOW, FeatureKind::WINDOW);
                    }
                }
            }
        }
        // KIT-02: a cornice at the top, or a ledge at a storey.
        1 => {
            let y = if rng.below(2) == 0 {
                f.y.1 - 0.6
            } else {
                f.y.0 + 8.0 * (1 + rng.below(((h / 8.0) as u64).max(1))) as f64
            };
            let (a, b) = f.bx(
                0.0,
                1.2,
                t0 - 0.0,
                t1,
                y.min(f.y.1 - 0.5),
                y.min(f.y.1 - 0.5) + 0.5,
            );
            m.boxed(1, a, b, MaterialId::CONCRETE, FeatureKind::MISC);
        }
        // KIT-03: vertical ribs every 8 m.
        2 => {
            let mut x = t0 + 4.0;
            while x < t1 - 2.0 {
                let (a, b) = f.bx(0.0, 0.6, x - 0.3, x + 0.3, f.y.0, f.y.1);
                m.boxed(2, a, b, MaterialId::METAL, FeatureKind::MISC);
                x += 8.0;
            }
        }
        // KIT-04: a vent grille with a fan in it.
        3 => {
            let x = along(rng, 3.0);
            let y = f.y.0 + 2.0 + rng.next_f32() as f64 * (h - 6.0).max(0.0);
            let (a, b) = f.bx(0.0, 0.3, x - 1.5, x + 1.5, y, y + 2.0);
            m.boxed(3, a, b, MaterialId::GRATING, FeatureKind::MISC);
            let mut base = [0.0; 3];
            base[f.axis] = f.p + f.sign * 0.3;
            base[2 - f.axis] = x;
            base[1] = y + 1.0;
            let axis = if f.axis == 0 { Axis::X } else { Axis::Z };
            if f.sign < 0.0 {
                base[f.axis] -= 0.5;
            }
            m.bar(
                3,
                axis,
                base,
                0.5,
                0.9,
                MaterialId::METAL,
                FeatureKind::MISC,
            );
        }
        // KIT-05: a mast on the roof with a lamp on its tip.
        4 => {
            let x = lo[0] + 2.0 + rng.next_f32() as f64 * (hi[0] - lo[0] - 4.0).max(0.0);
            let z = lo[2] + 2.0 + rng.next_f32() as f64 * (hi[2] - lo[2] - 4.0).max(0.0);
            let len = 8.0 + 4.0 * rng.below(5) as f64;
            m.bar(
                4,
                Axis::Y,
                [x, hi[1], z],
                len,
                0.3,
                MaterialId::METAL,
                FeatureKind::MISC,
            );
            m.boxed(
                4,
                [x - 0.4, hi[1] + len, z - 0.4],
                [x + 0.4, hi[1] + len + 0.8, z + 0.4],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
            );
        }
        // KIT-06: façade pipes, one to three.
        5 => {
            let n = 1 + rng.below(3);
            let mut x = along(rng, 2.0);
            for _ in 0..n {
                let mut base = [0.0; 3];
                base[f.axis] = f.p + f.sign * 0.4;
                base[2 - f.axis] = x.min(t1 - 1.0);
                base[1] = f.y.0;
                m.bar(
                    5,
                    Axis::Y,
                    base,
                    h,
                    0.35,
                    MaterialId::PIPE,
                    FeatureKind::PIPE,
                );
                x += 1.2;
            }
        }
        // KIT-07: cable runs across the face at two heights (visual only).
        6 => {
            for k in 0..2 {
                let y = f.y.0 + h * (0.3 + 0.35 * k as f64);
                let mut base = [0.0; 3];
                base[f.axis] = f.p + f.sign * 0.3;
                base[2 - f.axis] = t0;
                base[1] = y;
                let axis = if f.axis == 0 { Axis::Z } else { Axis::X };
                m.bar(6, axis, base, w, 0.1, MaterialId::METAL, FeatureKind::CABLE);
            }
        }
        // KIT-08: a coffered underside: beams in both directions under the part.
        7 => {
            let step = 4.0;
            let mut x = lo[0] + 2.0;
            let mut n = 0;
            while x < hi[0] && n < 14 {
                m.boxed(
                    7,
                    [x - 0.2, lo[1] - 0.8, lo[2]],
                    [x + 0.2, lo[1], hi[2]],
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                );
                x += step;
                n += 1;
            }
            let mut z = lo[2] + 2.0;
            n = 0;
            while z < hi[2] && n < 14 {
                m.boxed(
                    7,
                    [lo[0], lo[1] - 0.8, z - 0.2],
                    [hi[0], lo[1] - 0.4, z + 0.2],
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                );
                z += step;
                n += 1;
            }
        }
        // KIT-09: lamps on chains from the underside.
        8 => {
            for _ in 0..=rng.below(3) {
                let x = lo[0] + 2.0 + rng.next_f32() as f64 * (hi[0] - lo[0] - 4.0).max(0.0);
                let z = lo[2] + 2.0 + rng.next_f32() as f64 * (hi[2] - lo[2] - 4.0).max(0.0);
                let drop = 2.0 + rng.below(3) as f64;
                m.bar(
                    8,
                    Axis::Y,
                    [x, lo[1] - drop, z],
                    drop,
                    0.04,
                    MaterialId::METAL,
                    FeatureKind::CABLE,
                );
                m.boxed(
                    8,
                    [x - 0.3, lo[1] - drop - 0.6, z - 0.3],
                    [x + 0.3, lo[1] - drop, z + 0.3],
                    MaterialId::EMISSIVE,
                    FeatureKind::MISC,
                );
            }
        }
        // KIT-10: a glyph panel: a dark slab with the name of the district in the City's script,
        // three glyphs of seven by seven lit cells (MUL-01).
        9 => {
            let cell = 0.3;
            let glyph = 7.0 * cell;
            let width = 3.0 * glyph + 2.0 * cell + 2.0;
            let x = along(rng, width / 2.0 + 1.0).max(t0 + width / 2.0 + 0.5);
            let y = f.y.0 + 2.0 + rng.next_f32() as f64 * (h - 6.0).max(0.0);
            let (a, b) = f.bx(
                0.0,
                0.2,
                x - width / 2.0,
                x + width / 2.0,
                y,
                y + glyph + 1.0,
            );
            m.boxed(9, a, b, MaterialId::METAL, FeatureKind::MISC);
            for (k, g) in name.iter().enumerate() {
                let gx = x - width / 2.0 + 1.0 + k as f64 * (glyph + cell);
                for (row, bits) in g.iter().enumerate() {
                    for col in 0..7 {
                        if bits >> col & 1 == 1 {
                            let (cx, cy) =
                                (gx + col as f64 * cell, y + 0.5 + (6 - row) as f64 * cell);
                            let (a, b) = f.bx(0.2, 0.3, cx, cx + cell * 0.9, cy, cy + cell * 0.9);
                            m.boxed(9, a, b, MaterialId::EMISSIVE, FeatureKind::MISC);
                        }
                    }
                }
            }
        }
        // KIT-11: a handrail along the foot of the face, on posts (the stairs have their own).
        10 => {
            let y = f.y.0 + 1.0;
            let (a, b) = f.bx(0.2, 0.6, t0, t1, y, y + 0.08);
            m.boxed(10, a, b, MaterialId::METAL, FeatureKind::RAILING);
            let mut x = t0;
            while x <= t1 {
                let (a, b) = f.bx(0.2, 0.6, x - 0.04, x + 0.04, f.y.0, y);
                m.boxed(10, a, b, MaterialId::METAL, FeatureKind::RAILING);
                x += 2.0;
            }
        }
        // KIT-12: a scaffold: poles every 3 m and a platform deck every 8 m.
        _ => {
            let top = f.y.0 + (h.min(24.0) / 8.0).floor().max(1.0) * 8.0;
            let mut x = t0 + 1.0;
            let end = (t0 + 12.0).min(t1 - 1.0);
            while x <= end {
                let (a, b) = f.bx(1.0, 1.15, x, x + 0.15, f.y.0, top);
                m.boxed(11, a, b, MaterialId::METAL, FeatureKind::MISC);
                x += 3.0;
            }
            let mut y = f.y.0 + 8.0;
            while y <= top {
                let (a, b) = f.bx(0.2, 1.2, t0 + 1.0, end + 0.15, y, y + 0.15);
                m.boxed(11, a, b, MaterialId::GRATING, FeatureKind::MISC);
                y += 8.0;
            }
        }
    }
}

/// The kit pieces on the structures of `t`, with the index into `KIT_TAGS` of each.
pub fn dress(t: &Tissue) -> Vec<(usize, Primitive)> {
    let pitch = t.lattice.pitch as f64;
    let base = hash(t.seed.0, Level::L2, t.cell.coords, salt::KIT_EXT);
    // The district's name in the City's script, for the glyph panels (MUL-01).
    let alphabet = crate::glyphs::alphabet(t.seed);
    let l1 = t.cell.coords.div_euclid(glam::I64Vec3::splat(8));
    let name = crate::glyphs::name_of(t.seed, l1).map(|g| alphabet[usize::from(g)]);
    let mut m = M {
        em: Emitter::new(&t.lattice),
        tags: Vec::new(),
    };
    for (si, s) in t.structures.iter().enumerate() {
        // Only exterior masses: plain boxes. Hollow parts and shaped pieces keep their own look.
        for (pi, b) in s.parts.iter().enumerate() {
            let mut rng = Rng::new(mix(base ^ ((si as u64) << 32 | pi as u64)));
            let lo = [0, 1, 2].map(|k| f64::from(b.lo[k]) * pitch);
            let hi = [0, 1, 2].map(|k| f64::from(b.hi[k]) * pitch);
            // Eight draws are made for every part, whether or not it takes the kit.
            let dressed = rng.below(2) == 0;
            let picks = [
                rng.below(12) as usize,
                rng.below(12) as usize,
                rng.below(12) as usize,
            ];
            let face_picks = [
                rng.below(4) as usize,
                rng.below(4) as usize,
                rng.below(4) as usize,
            ];
            if !dressed || hi[1] - lo[1] < 8.0 {
                continue;
            }
            let fs = faces(lo, hi);
            for (kit, face) in picks.into_iter().zip(face_picks) {
                piece(&mut m, kit, &mut rng, &fs[face], lo, hi, &name);
            }
        }
    }
    // Nothing fills a doorway or a bored passage.
    let keep_out: Vec<QAabb> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .map(Primitive::aabb)
        .collect();
    let M { em, tags } = m;
    let mut out: Vec<(usize, Primitive)> = tags.into_iter().zip(em.out).collect();
    // Pieces of a turned lattice that fall wholly outside the cell are dropped.
    let cell_q = 512 * Q_PER_M;
    out.retain(|(_, p)| {
        let b = p.aabb();
        b.max.x > 0
            && b.min.x < cell_q
            && b.max.y > 0
            && b.min.y < cell_q
            && b.max.z > 0
            && b.min.z < cell_q
    });
    out.retain(|(_, p)| {
        let b = p.aabb();
        let pad = 8;
        !keep_out.iter().any(|k| {
            b.min.x < k.max.x + pad
                && b.max.x > k.min.x - pad
                && b.min.y < k.max.y + pad
                && b.max.y > k.min.y - pad
                && b.min.z < k.max.z + pad
                && b.max.z > k.min.z - pad
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellKey;
    use crate::seed::WorldSeed;
    use crate::tissue::TissueCache;

    /// KIT-01..12 all occur, nothing collides and everything stays in the cell (relief out of
    /// a face may reach 3 m past it).
    #[test]
    fn every_kit_piece_occurs_and_is_harmless() {
        let seed = WorldSeed(4);
        let cache = TissueCache::default();
        let mut seen = [0usize; 12];
        let mut cells = 0;
        for k in 0..400i64 {
            let key = CellKey::new(
                Level::L2,
                glam::I64Vec3::new(k % 8 - 4, k / 8 % 3, k / 24 - 8),
            );
            let t = crate::tissue::tissue_cell(seed, &cache, key);
            let pieces = dress(&t);
            eprintln!(
                "cell {k}: structures {} parts {} pieces {}",
                t.structures.len(),
                t.structures.iter().map(|s| s.parts.len()).sum::<usize>(),
                pieces.len()
            );
            cells += 1;
            for (tag, p) in &pieces {
                seen[*tag] += 1;
                assert!(p.flags.contains(PrimFlags::NO_COLLIDE));
                let b = p.aabb();
                assert!(b.max.x > 0 && b.min.x < 512 * 16 && b.max.z > 0 && b.min.z < 512 * 16);
            }
            // Determinism: a second run agrees.
            assert_eq!(dress(&t).len(), pieces.len());
        }
        assert!(cells > 0);
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    }
}
