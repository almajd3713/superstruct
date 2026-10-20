//! Path modifiers MOD-01..09 (`levels.md` §5W.5, §5W.20): drawn independently of the type and
//! built on top of a horizontal leg. None of them narrows the walkway below the type's own
//! width or lowers the headroom under 2.2 m on the centre line, so walkability is unchanged.
//!
//! Drawn per segment from `hash(seed, L2, cell, MODS)`: eight words, always.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::orient::{Frame, LocalShape};
use crate::paths::{Build, Leg, Params};
use crate::paths_v::flat_yaw;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::tissue_grow::{Emitter, quanta};

/// Enclosures (MOD-02).
pub const OPEN: u8 = 0;
pub const RAILED: u8 = 1;
pub const CAGED: u8 = 2;
pub const WALLED: u8 = 3;
pub const GLAZED: u8 = 4;
pub const TUNNEL: u8 = 5;

/// The modifiers of one segment. Each field is an index; 0 is the plain case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mods {
    /// MOD-02: open, railed, caged, walled, glazed, tunnel.
    pub enclosure: u8,
    /// MOD-03: intact, missing rails (rails stand in alternate 6 m stretches).
    pub condition: u8,
    /// MOD-04: dark, lamp row.
    pub light: u8,
    /// MOD-05: open, crushing 2.2 m, normal 3 m, cathedral 6 m (under a roof).
    pub ceiling: u8,
    /// MOD-06: none, frames, arches, ribs.
    pub rhythm: u8,
    /// MOD-07: concrete, metal, warm.
    pub material: u8,
    /// MOD-08: none, cables, pipes overhead, signage glyphs.
    pub dressing: u8,
    /// MOD-09: on-grid, off-grid, free.
    pub angle: u8,
    /// NEAR-01..05: none, grille, slit view, doorway framing a far light, gaps in the grating,
    /// windows.
    pub near: u8,
    /// Spacing and sizes, in `[0, 1)`.
    pub k: f64,
}

impl Mods {
    pub const PLAIN: Self = Self {
        enclosure: OPEN,
        condition: 0,
        light: 0,
        ceiling: 0,
        rhythm: 0,
        material: 0,
        dressing: 0,
        angle: 0,
        near: 0,
        k: 0.0,
    };

    /// The height of the roof when there is one.
    pub fn roof(&self) -> Option<f64> {
        matches!(self.enclosure, GLAZED | TUNNEL).then(|| match self.ceiling {
            1 => 2.2,
            3 => 6.0,
            _ => 3.0,
        })
    }
}

/// Types that take an enclosure: open decks on a flat line.
const ENCLOSABLE: [u8; 7] = [1, 3, 6, 7, 16, 17, 18];
/// Types that take a rhythm of frames overhead.
const RHYTHMIC: [u8; 8] = [1, 6, 7, 8, 13, 16, 17, 18];
/// Types that already have a ceiling or walls where dressing would sit.
const NO_DRESSING: [u8; 5] = [2, 9, 10, 11, 12];

fn frac(word: u64) -> f64 {
    (word >> 40) as f64 / f64::from(1u32 << 24)
}

/// Draws the modifiers of a segment of type `id` from eight words. `weights` are the
/// stratum's four material weights.
pub fn draw(words: &[u64; 9], id: u8, weights: [u8; 4]) -> Mods {
    let enclosure = if ENCLOSABLE.contains(&id) {
        [0, 0, 0, 0, 1, 1, 2, 3, 4, 5][(words[0] % 10) as usize]
    } else {
        OPEN
    };
    let roofed = matches!(enclosure, GLAZED | TUNNEL);
    let w = |i: usize| u64::from(weights[i]) + 1;
    let total = w(0) + w(1) + w(2);
    let pick = words[5] % total;
    let material = if pick < w(0) {
        0
    } else if pick < w(0) + w(1) {
        1
    } else {
        2
    };
    Mods {
        enclosure,
        condition: u8::from(matches!(enclosure, RAILED | CAGED) && words[1].is_multiple_of(4)),
        light: u8::from(words[2] & 1 == 1),
        ceiling: if roofed { 1 + (words[3] % 3) as u8 } else { 0 },
        rhythm: if RHYTHMIC.contains(&id) {
            [0, 0, 0, 1, 2, 3][(words[4] % 6) as usize]
        } else {
            0
        },
        material,
        dressing: if NO_DRESSING.contains(&id) {
            0
        } else {
            [0, 0, 1, 2, 3][(words[6] % 5) as usize]
        },
        angle: [0, 0, 0, 1, 1, 2][(words[7] % 6) as usize],
        near: match [0, 0, 0, 0, 0, 1, 2, 3, 4, 5][(words[8] % 10) as usize] {
            // Types that wander sideways would run into a detail beside the centre line.
            _ if matches!(id, 5 | 19 | 20) => 0,
            4 if id != 1 => 0,
            n => n,
        },
        k: frac(words[7] >> 3 | words[6] << 20),
    }
}

/// An oriented box centred on a lattice-frame point, turned about the vertical by `yaw`
/// headings (needs a flat lattice frame): `size` is along the leg, up and across.
fn turned(b: &mut Build<'_, '_>, centre: [f64; 3], size: [f64; 3], dy: u8, m: MaterialId) -> bool {
    let Some(base) = flat_yaw(b.e.lat.frame) else {
        return false;
    };
    let leg = b.leg;
    let along =
        i32::from(base) + if leg.axis == 2 { 16 } else { 0 } + if leg.sign < 0.0 { 32 } else { 0 };
    let yaw = (along + i32::from(dy)).rem_euclid(64) as u8;
    let half = QVec3::new(
        quanta(size[0] / 2.0),
        quanta(size[1] / 2.0),
        quanta(size[2] / 2.0),
    );
    let aabb = QAabb::new(
        QVec3::new(-half.x, -half.y, -half.z),
        QVec3::new(half.x, half.y, half.z),
    );
    let origin = Emitter::qv(b.e.at(centre));
    b.e.out.push(Primitive {
        shape: Shape::oriented(
            Frame::table(yaw, 0),
            origin,
            LocalShape::Box { aabb },
            b.e.cell,
        ),
        material: m,
        kind: FeatureKind::BRIDGE,
        flags: PrimFlags::NONE,
    });
    true
}

/// Builds the modifiers of `m` on the leg of a type `id` path that began at `first` in
/// `e.out`.
#[allow(clippy::too_many_lines)] // one block per modifier
pub(crate) fn apply(e: &mut Emitter<'_>, leg: Leg, p: &Params, m: &Mods, first: usize) {
    let (len, h) = (leg.len, p.width / 2.0);
    let roof = m.roof();
    let mut b = Build {
        e,
        leg,
        clear: Vec::new(),
    };
    // MOD-02, MOD-03: enclosure and rails.
    let rail = |b: &mut Build<'_, '_>, side: f64| {
        let mut s = 0.0;
        let mut on = true;
        while s < len {
            let e = (s + 6.0).min(len);
            if on || m.condition == 0 {
                b.rail((s, e), (side * h - 0.05, side * h + 0.05), (0.0, 1.1));
            }
            on = !on;
            s = e;
        }
    };
    match m.enclosure {
        RAILED => {
            rail(&mut b, -1.0);
            rail(&mut b, 1.0);
        }
        CAGED => {
            rail(&mut b, -1.0);
            rail(&mut b, 1.0);
            let mut s = 0.0;
            while s <= len {
                for side in [-1.0, 1.0] {
                    b.rail((s, s + 0.1), (side * h - 0.05, side * h + 0.05), (1.1, 2.6));
                }
                b.rail((s, s + 0.1), (-h, h), (2.6, 2.7));
                s += 8.0;
            }
        }
        WALLED => {
            for side in [-1.0, 1.0] {
                b.side_solid(
                    (0.0, len),
                    (side * h, side * (h + 0.3)),
                    (0.0, 2.6),
                    MaterialId::CONCRETE,
                );
            }
        }
        GLAZED | TUNNEL => {
            let top = roof.unwrap_or(3.0);
            for side in [-1.0, 1.0] {
                let t = (side * h, side * (h + 0.3));
                if m.enclosure == TUNNEL {
                    b.side_solid((0.0, len), t, (0.0, top), MaterialId::CONCRETE);
                } else {
                    b.side_solid((0.0, len), t, (0.0, 0.9), MaterialId::CONCRETE);
                    b.side_solid((0.0, len), t, (0.9, top), MaterialId::GRATING);
                }
            }
            b.solid(
                (0.0, len),
                (-h - 0.3, h + 0.3),
                (top, top + 0.3),
                MaterialId::CONCRETE,
            );
        }
        _ => {}
    }
    let top = roof.map_or(3.0, |r| r + 0.3);
    // MOD-04: a lamp row on the wall side.
    if m.light == 1 {
        let mut s = 4.0;
        while s < len {
            let t = p.side * (h + 0.35);
            b.bx(
                (s - 0.15, s + 0.15),
                (t - 0.15, t + 0.15),
                (2.0, 2.3),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
            s += 24.0;
        }
    }
    // MOD-06, MOD-09: frames, arches or ribs overhead; the lintels turn when off-grid.
    if m.rhythm > 0 {
        let step = if m.rhythm == 3 {
            8.0 + 4.0 * m.k
        } else {
            16.0 + 16.0 * m.k
        };
        let high = if roof.is_some() {
            top
        } else {
            3.0 + (p.width / 2.0).min(5.0)
        };
        let dy = [
            0,
            [2, 4, 6][(m.k * 3.0) as usize % 3],
            [3, 5, 7, 9, 11][(m.k * 5.0) as usize % 5],
        ][usize::from(m.angle)];
        let thick = if m.rhythm == 3 { 0.12 } else { 0.4 };
        let mut s = step / 2.0;
        while s < len {
            for side in [-1.0, 1.0] {
                let t = side * (h + 0.7);
                b.solid(
                    (s - thick / 2.0, s + thick / 2.0),
                    (t - thick / 2.0, t + thick / 2.0),
                    (0.0, high),
                    MaterialId::METAL,
                );
                if m.rhythm == 2 {
                    // Haunches: a stepped brace in each corner.
                    for k in 0..3 {
                        let d = 0.4 * f64::from(k + 1);
                        b.solid(
                            (s - 0.2, s + 0.2),
                            (side * (h + 0.7 - d), side * (h + 0.7)),
                            (high - d, high - d + 0.4),
                            MaterialId::METAL,
                        );
                    }
                }
            }
            let lintel_len = 2.0 * (h + 0.7) + 0.4;
            let c = b.leg.map(s, 0.0, high + thick / 2.0);
            if dy == 0 || !turned(&mut b, c, [thick, thick, lintel_len], dy, MaterialId::METAL) {
                b.solid(
                    (s - thick / 2.0, s + thick / 2.0),
                    (-lintel_len / 2.0, lintel_len / 2.0),
                    (high, high + thick),
                    MaterialId::METAL,
                );
            }
            s += step;
        }
    }
    // MOD-08: dressing overhead.
    match m.dressing {
        1 => {
            for (t, y) in [(-0.5_f64, 3.4_f64), (0.0, 3.6), (0.7, 3.5)] {
                let y = y.max(top);
                b.bx(
                    (0.0, len),
                    (t - 0.03, t + 0.03),
                    (y, y + 0.06),
                    MaterialId::PIPE,
                    FeatureKind::CABLE,
                    PrimFlags::NO_COLLIDE,
                );
            }
        }
        2 => {
            for t in [-(h * 0.5 + 0.4), h * 0.5 + 0.4] {
                b.bx(
                    (0.0, len),
                    (t - 0.12, t + 0.12),
                    (top, top + 0.25),
                    MaterialId::PIPE,
                    FeatureKind::PIPE,
                    PrimFlags::NONE,
                );
            }
        }
        3 => {
            let mut s = 7.0;
            let mut i = 0;
            while s < len {
                let t = p.side * (h + 0.4);
                let (w, g) = if i % 2 == 0 { (0.9, 0.5) } else { (0.6, 0.6) };
                b.bx(
                    (s - w / 2.0, s + w / 2.0),
                    (t - 0.03, t + 0.03),
                    (2.0, 2.0 + g),
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    PrimFlags::NO_COLLIDE,
                );
                i += 1;
                s += 28.0;
            }
        }
        _ => {}
    }
    // NEAR-01..05: details at the scale of a few metres, beside the walkway.
    let side = p.side;
    let mid = len / 2.0;
    match m.near {
        1 => {
            // A grille over an opening: bars every quarter metre in a frame.
            let t = side * (h + 0.6);
            b.rail((mid - 1.5, mid + 1.5), (t - 0.1, t + 0.1), (0.0, 0.1));
            b.rail((mid - 1.5, mid + 1.5), (t - 0.1, t + 0.1), (2.5, 2.6));
            let mut s = mid - 1.5;
            while s <= mid + 1.5 {
                b.rail((s, s + 0.04), (t - 0.03, t + 0.03), (0.1, 2.5));
                s += 0.25;
            }
        }
        2 => {
            // A slit view: a thick wall with a gap 0.3 m wide, full height.
            let t = (side * (h + 0.4), side * (h + 1.0));
            b.solid((mid - 2.0, mid - 0.15), t, (0.0, 3.0), MaterialId::CONCRETE);
            b.solid((mid + 0.15, mid + 2.0), t, (0.0, 3.0), MaterialId::CONCRETE);
        }
        3 if len >= 40.0 => {
            // A doorway frames a lamp at the far end.
            let s = len * 0.3;
            for sd in [-1.0, 1.0] {
                b.solid(
                    (s - 0.3, s + 0.3),
                    (sd * (h + 0.1), sd * (h + 0.6)),
                    (0.0, 3.4),
                    MaterialId::CONCRETE,
                );
            }
            b.solid(
                (s - 0.3, s + 0.3),
                (-h - 0.6, h + 0.6),
                (3.0, 3.4),
                MaterialId::CONCRETE,
            );
            b.bx(
                (len - 0.7, len - 0.2),
                (-0.25, 0.25),
                (1.4, 1.9),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
        }
        5 => {
            // Windows: lit panels in a row along the façade side.
            let t = side * (h + 0.45);
            let mut s = 3.0;
            while s < len {
                b.bx(
                    (s - 0.6, s + 0.6),
                    (t - 0.03, t + 0.03),
                    (1.2, 2.4),
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    PrimFlags::NO_COLLIDE,
                );
                s += 5.0;
            }
        }
        _ => {}
    }
    // MOD-07: the structure takes the stratum's material.
    retint(b.e, m, first);
}

/// Changes the structural material of a rise built since `first` (MOD-07 on a vertical run).
pub(crate) fn retint(e: &mut Emitter<'_>, m: &Mods, first: usize) {
    let mat = [MaterialId::CONCRETE, MaterialId::METAL, MaterialId::WARM][usize::from(m.material)];
    for q in &mut e.out[first..] {
        if q.material == MaterialId::CONCRETE || q.material == MaterialId::METAL {
            q.material = mat;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{IDS, walk_line, width_range};
    use crate::rng::Rng;
    use crate::tissue::Lattice;

    fn solid_at(prims: &[Primitive], p: glam::Vec3) -> bool {
        prims
            .iter()
            .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
    }

    fn words(seed: u64) -> [u64; 9] {
        let mut r = Rng::new(seed);
        std::array::from_fn(|_| r.next_u64())
    }

    /// No modifier takes the floor from under a type's walking line or puts anything solid in
    /// the first 1.85 m above it (1.15 m for the crawl duct), in either axis.
    #[test]
    fn modifiers_keep_every_type_walkable() {
        let lat = Lattice::new(Frame::Id);
        for id in IDS {
            for seed in 0..40u64 {
                let m = draw(&words(seed * 7919 + u64::from(id)), id, [3, 1, 2, 0]);
                let (lo, hi) = width_range(id);
                let f = (seed % 5) as f64 / 4.0;
                let p = Params {
                    width: lo + (hi - lo) * f,
                    rails: (seed % 3) as u8,
                    windowed: seed % 2 == 1,
                    side: if seed % 2 == 0 { 1.0 } else { -1.0 },
                    k: f,
                    j: 1.0 - f,
                    gaps: m.near == 4,
                };
                for (axis, sign) in [(0, 1.0), (2, -1.0)] {
                    let mut start = [200.0, 100.0, 200.0];
                    start[axis] = if sign > 0.0 { 100.0 } else { 400.0 };
                    let leg = Leg {
                        trim: [0.0; 2],
                        start,
                        axis,
                        sign,
                        len: 100.0 + 20.0 * f,
                    };
                    let mut e = Emitter::new(&lat);
                    crate::paths::build_leg(&mut e, leg, id, &p);
                    apply(&mut e, leg, &p, &m, 0);
                    let heads: &[f64] = if id == 12 {
                        &[0.25, 0.9, 1.15]
                    } else {
                        &[0.25, 0.9, 1.5, 1.85]
                    };
                    for [s, t, y] in walk_line(id, leg.len, &p) {
                        let at = |dy: f64| {
                            let q = e.at(leg.map(s, t, y + dy));
                            glam::Vec3::new(q.x as f32, q.y as f32, q.z as f32)
                        };
                        assert!(
                            solid_at(&e.out, at(-0.1)),
                            "PH-{id:02} {m:?}: no floor at {s:.1}"
                        );
                        for &dy in heads {
                            assert!(
                                !solid_at(&e.out, at(dy)),
                                "PH-{id:02} {m:?}: blocked {dy} m above {s:.1}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Every value of every modifier is drawn, and a type that cannot take one never gets it.
    #[test]
    fn every_modifier_value_occurs() {
        let mut seen = [[false; 6]; 9];
        for seed in 0..4000u64 {
            let m = draw(&words(seed), 1, [3, 1, 2, 0]);
            for (row, v) in [
                m.enclosure,
                m.condition,
                m.light,
                m.ceiling,
                m.rhythm,
                m.material,
                m.dressing,
                m.angle,
                m.near,
            ]
            .into_iter()
            .enumerate()
            {
                seen[row][usize::from(v)] = true;
            }
            assert_eq!(m.roof().is_some(), matches!(m.enclosure, GLAZED | TUNNEL));
            let d = draw(&words(seed), 2, [3, 1, 2, 0]);
            assert!(
                d.enclosure == OPEN && d.rhythm == 0 && d.dressing == 0,
                "{d:?}"
            );
        }
        let counts = [6, 2, 2, 4, 4, 3, 4, 3, 6];
        for (row, n) in counts.into_iter().enumerate() {
            assert!(
                seen[row][..n].iter().all(|&b| b),
                "modifier {row}: {:?}",
                seen[row]
            );
        }
    }
}
