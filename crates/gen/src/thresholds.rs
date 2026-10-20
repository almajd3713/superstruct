//! Thresholds THR-01..08 (`levels.md` §5W.6, §5W.19): the frame, gate or hole where a route
//! enters a structure or a bored passage. THR-09, the gravity gate, waits for the gravity module
//! (PW.3.13b).
//!
//! A threshold is built in the frame of its face: `s` into the structure (negative outside),
//! `t` across and `y` up from the route slot's floor. Its opening is never narrower than 2 m
//! (the port minimum of `ports.md` §2), and a sill is at most 0.4 m (under the step-up). Each
//! also adds a clear box to `Thresholds::bores`: air to cut from the structure or the solid.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::cell::{Dir6, Level};
use crate::hash::hash;
use crate::paths::{Build, Clear, Leg};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue::{SlotState, Surround, Tissue};
use crate::tissue_grow::Emitter;

/// The ids of this module: THR-01..THR-08.
pub const IDS: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// What the thresholds of a tissue are.
#[derive(Debug, Clone, Default)]
pub struct Thresholds {
    /// `(route slot outside, THR id)` in the order they were placed.
    pub list: Vec<(u32, u8)>,
    /// Cell-local primitives.
    pub prims: Vec<Primitive>,
    /// Clear boxes: air to cut from the structure or solid the route enters.
    pub bores: Vec<Primitive>,
}

/// A face a route crosses: the slot outside, and the way in.
#[derive(Debug, Clone, Copy)]
struct Face {
    slot: u32,
    into: Dir6,
}

type S = (f64, f64);

/// The leg of a face: origin at the middle of the face on the route slot's floor (for a
/// horizontal face), or at the slot's centre on the floor or ceiling plane (for a vertical one).
fn leg_of(t: &Tissue, f: Face) -> Leg {
    let pitch = t.lattice.pitch as f64;
    let c = t.lattice.coords(f.slot as usize);
    let (cx, cz) = ((c[0] as f64 + 0.5) * pitch, (c[2] as f64 + 0.5) * pitch);
    let floor = c[1] as f64 * pitch;
    let (axis, sign, off) = match f.into {
        Dir6::PosX => (0, 1.0, pitch / 2.0),
        Dir6::NegX => (0, -1.0, pitch / 2.0),
        Dir6::PosZ => (2, 1.0, pitch / 2.0),
        Dir6::NegZ => (2, -1.0, pitch / 2.0),
        Dir6::PosY | Dir6::NegY => (0, 1.0, 0.0),
    };
    let mut start = [cx, floor, cz];
    start[axis] += sign * off;
    if f.into == Dir6::PosY {
        start[1] = floor + pitch;
    }
    Leg {
        start,
        axis,
        sign,
        len: 0.0,
        trim: [0.0; 2],
    }
}

/// A frame of two posts and a lintel round an opening `half` wide and `high` tall, over the
/// depth `s`.
fn portal(
    b: &mut Build<'_, '_>,
    s: S,
    half: f64,
    post: f64,
    high: f64,
    lintel: f64,
    m: MaterialId,
) {
    for side in [-1.0, 1.0] {
        b.solid(
            s,
            (side * half, side * (half + post)),
            (0.0, high + lintel),
            m,
        );
    }
    b.solid(s, (-half - post, half + post), (high, high + lintel), m);
}

/// The clear box of a door `half` wide and `high` tall, `depth` deep.
fn passage(b: &mut Build<'_, '_>, depth: f64, half: f64, high: f64) {
    b.cut((0.0, depth), (-half, half), (-0.5, high));
}

/// Builds threshold `id` at `face`; `k` in `[0, 1)` varies its sizes. Returns the clear boxes.
#[allow(clippy::too_many_lines)] // one arm per threshold
fn build(e: &mut Emitter<'_>, leg: Leg, id: u8, vertical: i8, k: f64) -> Vec<Clear> {
    let mut b = Build {
        e,
        leg,
        clear: Vec::new(),
    };
    let metal = MaterialId::METAL;
    let concrete = MaterialId::CONCRETE;
    if vertical > 0 {
        // A hatch in the ceiling: a collar round a 2.4 m square, and the way up.
        for side in [-1.0, 1.0] {
            b.solid((-1.2, 1.2), (side * 1.2, side * 1.7), (-0.45, 0.0), metal);
            b.solid((side * 1.2, side * 1.7), (-1.7, 1.7), (-0.45, 0.0), metal);
        }
        b.cut((-1.2, 1.2), (-1.2, 1.2), (-0.5, 4.0));
        return b.clear;
    }
    match id {
        // THR-01: a bulkhead hatch, a thick metal frame with a coaming to step over.
        1 => {
            portal(&mut b, (-0.3, 0.5), 1.0, 0.5, 2.4, 0.4, metal);
            b.solid((-0.3, 0.5), (-1.0, 1.0), (-0.1, 0.3), metal);
            passage(&mut b, 4.0, 1.4, 3.0);
        }
        // THR-02: a portal frame of heavy concrete.
        2 => {
            portal(&mut b, (-0.6, 0.6), 1.5, 1.2, 4.0, 1.5, concrete);
            passage(&mut b, 4.0, 1.6, 4.0);
        }
        // THR-03: a giant gate, shut, with a door cut into it for one person.
        3 => {
            let (wide, tall) = (4.0 + 2.0 * k, 11.0 + 3.0 * k);
            for side in [-1.0, 1.0] {
                b.solid(
                    (-1.0, 1.0),
                    (side * wide, side * (wide + 2.0)),
                    (0.0, tall),
                    concrete,
                );
                b.solid((-0.5, 0.5), (side * 1.1, side * wide), (0.0, 2.6), metal);
            }
            b.solid((-0.5, 0.5), (-wide, wide), (2.6, tall - 1.0), metal);
            passage(&mut b, 4.0, 1.4, 3.0);
        }
        // THR-04: a broken wall, stumps of different heights and rubble beside the gap.
        4 => {
            for side in [-1.0, 1.0] {
                for (i, h) in [1.5 + 3.0 * k, 2.8, 4.5 - 2.0 * k].into_iter().enumerate() {
                    let a = 1.6 + i as f64;
                    b.solid(
                        (-0.3, 0.5),
                        (side * a, side * (a + 1.0)),
                        (0.0, h),
                        concrete,
                    );
                }
                b.solid((-1.4, 0.8), (side * 1.4, side * 2.6), (0.0, 0.35), concrete);
            }
            passage(&mut b, 4.0, 1.6, 3.0);
        }
        // THR-05: a hole in the floor, a collar on three sides and a rubble stair down.
        5 => {
            b.solid((1.5, 2.0), (-2.0, 2.0), (0.0, 0.35), concrete);
            b.solid((5.0, 5.5), (-2.0, 2.0), (0.0, 0.35), concrete);
            for side in [-1.0, 1.0] {
                b.solid((2.0, 5.0), (side * 1.5, side * 2.0), (0.0, 0.35), concrete);
            }
            for i in 0..13 {
                let top = -0.3 * f64::from(i + 1);
                let s = 2.0 + 0.5 * f64::from(i);
                b.solid((s, s + 0.5), (-1.3, 1.3), (top - 2.0, top), concrete);
            }
            b.cut((2.0, 9.0), (-1.5, 1.5), (-4.3, 0.3));
        }
        // THR-06: an airlock, two hatches with a chamber between them.
        6 => {
            portal(&mut b, (-0.3, 0.5), 1.0, 0.5, 2.4, 0.4, metal);
            portal(&mut b, (5.7, 6.5), 1.0, 0.5, 2.4, 0.4, metal);
            for side in [-1.0, 1.0] {
                b.solid((0.5, 5.7), (side * 1.6, side * 2.0), (0.0, 3.0), metal);
            }
            b.solid((0.5, 5.7), (-2.0, 2.0), (3.0, 3.4), metal);
            passage(&mut b, 6.5, 2.2, 3.8);
        }
        // THR-07: a window to climb through, a sill under the step-up and a glass transom.
        7 => {
            b.solid((-0.3, 0.5), (-1.2, 1.2), (-0.1, 0.4), concrete);
            for side in [-1.0, 1.0] {
                b.solid((-0.3, 0.5), (side * 1.2, side * 1.5), (0.0, 3.2), concrete);
            }
            b.solid((-0.3, 0.5), (-1.5, 1.5), (2.8, 3.2), concrete);
            b.glass((-0.1, 0.3), (-1.2, 1.2), (2.4, 2.8));
            passage(&mut b, 3.0, 1.4, 3.4);
        }
        // THR-08: a threshold of light: a glowing frame, a lit floor and a panel beyond.
        _ => {
            for side in [-1.0, 1.0] {
                b.bx(
                    (-0.2, 0.4),
                    (side * 1.4, side * 1.7),
                    (0.0, 3.2),
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    PrimFlags::NONE,
                );
            }
            b.bx(
                (-0.2, 0.4),
                (-1.7, 1.7),
                (2.8, 3.2),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NONE,
            );
            b.bx(
                (-0.4, 0.4),
                (-1.4, 1.4),
                (0.0, 0.03),
                MaterialId::EMISSIVE,
                FeatureKind::WALKWAY,
                PrimFlags::NONE,
            );
            b.bx(
                (0.5, 3.5),
                (-1.5, 1.5),
                (3.2, 3.3),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
            passage(&mut b, 4.0, 1.5, 3.4);
        }
    }
    b.clear
}

/// The faces of a tissue that get a threshold, in order: structure entrances (structure
/// order), then the ends of bored passages (segment order).
fn faces(t: &Tissue) -> Vec<(Face, bool, bool)> {
    let mut out = Vec::new();
    for s in &t.structures {
        // Only the first two entrances of a structure get a frame: the rest are plain doors.
        for e in s.entrances.iter().take(2) {
            out.push((
                Face {
                    slot: e.slot,
                    into: e.into,
                },
                e.engulf,
                false,
            ));
        }
    }
    for seg in &t.segments {
        if seg.surround != Surround::Bored || seg.slots.len() < 2 {
            continue;
        }
        let at = |i: u32| t.lattice.state[i as usize];
        let n = seg.slots.len();
        for (out_i, in_i) in [(0, 1), (n - 1, n - 2)] {
            let (a, b) = (seg.slots[out_i], seg.slots[in_i]);
            if at(a) == SlotState::Solid || at(b) != SlotState::Solid {
                continue;
            }
            let (ca, cb) = (t.lattice.coords(a as usize), t.lattice.coords(b as usize));
            let d = [0, 1, 2].map(|k| cb[k] as i64 - ca[k] as i64);
            let into = match d {
                [1, 0, 0] => Dir6::PosX,
                [-1, 0, 0] => Dir6::NegX,
                [0, 1, 0] => Dir6::PosY,
                [0, -1, 0] => Dir6::NegY,
                [0, 0, 1] => Dir6::PosZ,
                [0, 0, -1] => Dir6::NegZ,
                _ => continue,
            };
            out.push((Face { slot: a, into }, false, true));
        }
    }
    out
}

/// Legal types and weights of a face: `(id, weight)`.
fn legal(into: Dir6, engulf: bool, bored: bool) -> Vec<(u8, u32)> {
    match (into, engulf, bored) {
        (Dir6::NegY, _, _) => vec![(5, 1)],
        (Dir6::PosY, _, _) => vec![(1, 1)],
        (_, true, _) => vec![(2, 3), (4, 3)],
        (_, _, true) => vec![(1, 3), (6, 1)],
        _ => vec![(1, 2), (2, 3), (3, 1), (4, 2), (6, 1), (7, 1), (8, 2)],
    }
}

/// Places a threshold on every structure entrance and at both ends of every bored passage.
pub fn thresholds_pass(seed: WorldSeed, t: &Tissue) -> Thresholds {
    let mut out = Thresholds::default();
    let mut e = Emitter::new(&t.lattice);
    let mut bores = Emitter::new(&t.lattice);
    for (face, engulf, bored) in faces(t) {
        // Three words per face, keyed by the face, so a face draws the same whatever else the
        // cell holds (the bored ends are known from the routes alone).
        let key = salt::THRESHOLDS + ((u64::from(face.slot) << 8) | face.into as u64) * 0x1_0000;
        let mut rng = Rng::new(hash(seed.0, Level::L2, t.cell.coords, key));
        let words: [u64; 3] = std::array::from_fn(|_| rng.next_u64());
        let cands = legal(face.into, engulf, bored);
        let ws: Vec<u32> = cands.iter().map(|c| c.1).collect();
        let id = Rng::new(words[0])
            .pick_weighted(&ws)
            .map_or(cands[0].0, |i| cands[i].0);
        let k = (words[1] >> 40) as f64 / f64::from(1u32 << 24);
        let vertical = match face.into {
            Dir6::PosY => 1,
            Dir6::NegY => -1,
            _ => 0,
        };
        let leg = leg_of(t, face);
        for (lo, hi) in build(&mut e, leg, id, vertical, k) {
            bores.frame_box(
                lo,
                hi,
                MaterialId::CONCRETE,
                FeatureKind::MISC,
                PrimFlags::NO_COLLIDE,
            );
        }
        out.list.push((face.slot, id));
    }
    out.prims = e.out;
    out.bores = bores.out;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orient::Frame;
    use crate::tissue::Lattice;

    fn solid_at(prims: &[Primitive], p: glam::Vec3) -> bool {
        prims
            .iter()
            .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
    }

    fn leg(axis: usize, sign: f64) -> Leg {
        Leg {
            start: [256.0, 100.0, 256.0],
            axis,
            sign,
            len: 0.0,
            trim: [0.0; 2],
        }
    }

    fn at(l: &Leg, s: f64, t: f64, y: f64) -> glam::Vec3 {
        let p = l.map(s, t, y);
        glam::Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
    }

    /// Every horizontal threshold leaves an opening 2 m wide (the port minimum) and 2.4 m
    /// high over its whole depth, with no sill over 0.4 m, in either axis and direction.
    #[test]
    fn openings_meet_the_port_minimum() {
        let lat = Lattice::new(Frame::Id);
        for id in IDS.into_iter().filter(|&i| i != 5) {
            for (axis, sign) in [(0, 1.0), (0, -1.0), (2, 1.0), (2, -1.0)] {
                for variant in 0..3 {
                    let l = leg(axis, sign);
                    let mut e = Emitter::new(&lat);
                    let clear = build(&mut e, l, id, 0, f64::from(variant) / 2.0);
                    assert!(!e.out.is_empty(), "THR-{id:02} emitted nothing");
                    assert!(!clear.is_empty(), "THR-{id:02} cuts nothing");
                    let mut s = -2.0;
                    while s < 3.0 {
                        for t in [-0.95, 0.0, 0.95] {
                            for y in [0.45, 1.0, 1.8, 2.35] {
                                assert!(
                                    !solid_at(&e.out, at(&l, s + 0.01, t, y)),
                                    "THR-{id:02} blocked at s {s:.2} t {t} y {y}"
                                );
                            }
                            // A sill is a step, never a wall.
                            assert!(
                                !solid_at(&e.out, at(&l, s + 0.01, t, 0.45)),
                                "THR-{id:02} sill over the step-up"
                            );
                        }
                        s += 0.25;
                    }
                }
            }
        }
    }

    /// The rubble stair of THR-05 has a floor under every step and headroom above.
    #[test]
    fn the_rubble_stair_is_walkable() {
        let lat = Lattice::new(Frame::Id);
        let l = leg(0, 1.0);
        let mut e = Emitter::new(&lat);
        build(&mut e, l, 5, -1, 0.5);
        for i in 0..13 {
            let s = 2.25 + 0.5 * f64::from(i);
            let top = -0.3 * f64::from(i + 1);
            assert!(solid_at(&e.out, at(&l, s, 0.0, top - 0.1)), "step {i}");
            for dy in [0.25, 0.9, 1.5, 1.85] {
                assert!(!solid_at(&e.out, at(&l, s, 0.0, top + dy)), "step {i} {dy}");
            }
        }
    }

    /// THR-01..08 all occur in real tissue, each at a face of the kind it is legal on.
    #[test]
    fn every_threshold_occurs() {
        use crate::cell::CellKey;
        use crate::tissue::{TissueCache, tissue_cell};
        use glam::I64Vec3;
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let mut seen = [0usize; 10];
        for k in 0..800i64 {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(seed, &cache, key);
            for &(slot, id) in &t.thresholds.list {
                assert!(t.lattice.present[slot as usize]);
                seen[id as usize] += 1;
            }
            if !t.thresholds.list.is_empty() {
                assert!(!t.thresholds.bores.is_empty(), "{key:?}");
            }
        }
        eprintln!("thresholds per type {:?}", &seen[1..9]);
        for id in IDS {
            assert!(seen[id as usize] > 0, "THR-{id:02} never occurs: {seen:?}");
        }
    }
}
