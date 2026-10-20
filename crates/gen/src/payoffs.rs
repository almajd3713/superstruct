//! The dead-end rule and legibility (`levels.md` §5W.2 step 4, §5W.23; TIS-05, RULE-01,
//! RULE-02): every leaf of the route tree that is not a port, an anchor, the knot or a
//! structure's entrance ends at a payoff, and the knot carries a beacon.
//!
//! The pass runs after growth. For each leaf, in route order, it tries a set piece (chance
//! 1/3, in the free slots next to the leaf), then a view (a free ray of 7 slots, 224 m), then a
//! lift (chance 1/4, to another route slot of the same column), then a light, which always
//! fits. Every leaf draws four words whatever happens.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::cell::Dir6;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive};
use crate::rng::Rng;
use crate::sets::{SET_BASE, SETS};
use crate::tissue::{AnchorKind, Segment, SlotState, Surround, Tissue};
use crate::tissue_grow::{Emitter, Entrance, SlotBox, Structure};

/// What ends a dead end (each has its own id in the census).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoffKind {
    /// A set piece (SET id) that was grown against the leaf.
    Set(u8),
    /// A balcony facing a free ray.
    View(Dir6),
    /// A lift to another level of the tree (the segment is `Tissue::lift_segments`).
    Lift,
    /// A lamp.
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payoff {
    pub slot: u32,
    pub kind: PayoffKind,
}

/// The payoffs of a tissue and the pieces they add.
#[derive(Debug, Clone, Default)]
pub struct Payoffs {
    pub list: Vec<Payoff>,
    /// Balconies, lamps and the knot's beacon, cell-local.
    pub prims: Vec<Primitive>,
}

const HORIZONTAL: [Dir6; 4] = [Dir6::PosX, Dir6::PosZ, Dir6::NegX, Dir6::NegZ];
/// A view needs a free ray this long: 7 slots of 32 m is 224 m (the spec asks for 200 m).
const VIEW_SLOTS: i32 = 7;

type BoxSpec = ([f64; 3], [f64; 3], MaterialId, FeatureKind, PrimFlags);

fn add(c: [i32; 3], d: Dir6) -> [i32; 3] {
    let o = d.offset();
    [c[0] + o.x as i32, c[1] + o.y as i32, c[2] + o.z as i32]
}

fn at(t: &Tissue, c: [i32; 3]) -> Option<usize> {
    let n = t.lattice.n as i32;
    c.iter()
        .all(|&v| (0..n).contains(&v))
        .then(|| {
            t.lattice
                .index([c[0] as usize, c[1] as usize, c[2] as usize])
        })
        .filter(|&i| t.lattice.present[i])
}

/// The dead ends of a tissue: leaves of the route tree that are no port, anchor, knot or
/// entrance, in route order.
pub fn dead_ends(t: &Tissue) -> Vec<u32> {
    let mut on = vec![false; t.lattice.len()];
    for &i in &t.route {
        on[i as usize] = true;
    }
    let mut keep = vec![false; t.lattice.len()];
    for a in &t.anchors {
        keep[a.slot as usize] = true;
    }
    for s in &t.structures {
        for e in &s.entrances {
            keep[e.slot as usize] = true;
        }
    }
    t.route
        .iter()
        .copied()
        .filter(|&i| {
            let i = i as usize;
            !keep[i]
                && matches!(t.lattice.state[i], SlotState::Route | SlotState::Free)
                && t.lattice.neighbours(i).filter(|&j| on[j]).count() <= 1
        })
        .collect()
}

/// A set piece grown against `leaf`: the first kind that fits in the free slots beside it.
fn try_set(t: &mut Tissue, leaf: usize, words: &[u64; 4]) -> Option<u8> {
    let legal = crate::sets::legal(t);
    let ws: Vec<u32> = legal.iter().map(|k| k.1).collect();
    let first = Rng::new(words[0]).pick_weighted(&ws)?;
    let c = t.lattice.coords(leaf).map(|v| v as i32);
    let pitch = t.lattice.pitch as i32;
    for step in 0..legal.len().min(3) {
        let kind = legal[(first + step) % legal.len()].0;
        let def = &SETS[kind];
        let m = |v: i32| ((v + pitch - 1) / pitch).max(1);
        let size = [m(def.size_m.0[0]), m(def.size_m.0[2]), m(def.size_m.0[1])];
        let Some(out) = (def.build)(size, &mut Rng::new(words[0] ^ kind as u64)) else {
            continue;
        };
        for k in 0..4 {
            let dir = HORIZONTAL[(words[1] as usize + k) % 4];
            // The box starts beside the leaf, centred on it across the other axis.
            let lo = {
                let mut lo = c;
                let o = dir.offset();
                if o.x > 0 || o.z > 0 {
                    lo[if o.x != 0 { 0 } else { 2 }] += 1;
                } else {
                    lo[if o.x != 0 { 0 } else { 2 }] -= size[if o.x != 0 { 0 } else { 2 }];
                }
                let across = if o.x != 0 { 2 } else { 0 };
                lo[across] -= size[across] / 2;
                lo
            };
            let region = SlotBox {
                lo,
                hi: [lo[0] + size[0], lo[1] + size[1], lo[2] + size[2]],
            };
            if !region.inside(t.lattice.n) {
                continue;
            }
            let slots = region.slots(&t.lattice);
            let free = slots
                .iter()
                .all(|&i| t.lattice.present[i] && t.lattice.state[i] == SlotState::Free);
            if !free {
                continue;
            }
            let shifted = out.shifted(lo);
            let mut placed = shifted.boxes.clone();
            placed.extend(shifted.shaped.iter().map(|(_, b)| *b));
            let mut taken: Vec<usize> = placed.iter().flat_map(|b| b.slots(&t.lattice)).collect();
            taken.sort_unstable();
            taken.dedup();
            for &i in &taken {
                t.lattice.state[i] = SlotState::Built;
            }
            t.sets.push((kind as u8 + 1, t.structures.len() as u16));
            t.structures.push(Structure {
                support: 0,
                form: (SET_BASE + kind) as u16,
                variant: 0,
                mutations: Vec::new(),
                parts: shifted.boxes,
                oriented: Vec::new(),
                cables: shifted
                    .vines
                    .iter()
                    .map(|v| [v[0], v[1], v[2], v[3]])
                    .collect(),
                shaped: shifted.shaped,
                windows: Vec::new(),
                inner: Vec::new(),
                dark: false,
                lit: shifted.lit,
                slots: taken.iter().map(|&i| i as u32).collect(),
                entrances: vec![Entrance {
                    slot: leaf as u32,
                    into: dir,
                    engulf: false,
                }],
                sealed: false,
            });
            return Some(kind as u8 + 1);
        }
    }
    None
}

/// A direction in which a ray of free or void slots runs at least `VIEW_SLOTS`.
fn view_dir(t: &Tissue, leaf: usize, offset: usize) -> Option<Dir6> {
    let c = t.lattice.coords(leaf).map(|v| v as i32);
    (0..4).map(|k| HORIZONTAL[(offset + k) % 4]).find(|&d| {
        let mut p = c;
        (0..VIEW_SLOTS).all(|_| {
            p = add(p, d);
            at(t, p)
                .is_some_and(|i| matches!(t.lattice.state[i], SlotState::Free | SlotState::Void))
        })
    })
}

/// A route slot in the column of `leaf` at another height, with only free slots between.
fn lift_target(t: &Tissue, leaf: usize) -> Option<usize> {
    let c = t.lattice.coords(leaf).map(|v| v as i32);
    let mut on = vec![false; t.lattice.len()];
    for &i in &t.route {
        on[i as usize] = true;
    }
    let mut best: Option<(i32, usize)> = None;
    for dir in [Dir6::PosY, Dir6::NegY] {
        let mut p = c;
        loop {
            p = add(p, dir);
            let Some(i) = at(t, p) else { break };
            if on[i] {
                if t.lattice.state[i] == SlotState::Route
                    && best.is_none_or(|b| (p[1] - c[1]).abs() < b.0)
                {
                    best = Some(((p[1] - c[1]).abs(), i));
                }
                break;
            }
            if t.lattice.state[i] != SlotState::Free {
                break;
            }
        }
    }
    best.map(|b| b.1)
}

/// A balcony from the slot centre `c = (x, z, floor)` to the face towards `d`, 6 m wide, with
/// a rail along the free end.
fn balcony(specs: &mut Vec<BoxSpec>, c: (f64, f64, f64), d: Dir6, pitch: f64) {
    let (cx, cz, floor) = c;
    let o = d.offset();
    let (dx, dz) = (f64::from(o.x as i32), f64::from(o.z as i32));
    let end = (cx + dx * pitch / 2.0, cz + dz * pitch / 2.0);
    let (x0, x1) = (
        cx.min(end.0) - 3.0 * dz.abs(),
        cx.max(end.0) + 3.0 * dz.abs(),
    );
    let (z0, z1) = (
        cz.min(end.1) - 3.0 * dx.abs(),
        cz.max(end.1) + 3.0 * dx.abs(),
    );
    specs.push((
        [x0, floor - 0.5, z0],
        [x1, floor, z1],
        MaterialId::CONCRETE,
        FeatureKind::WALKWAY,
        PrimFlags::NONE,
    ));
    let (rx0, rx1, rz0, rz1) = if o.x == 0 {
        (
            x0,
            x1,
            end.1.min(end.1 - dz * 0.1),
            end.1.max(end.1 - dz * 0.1),
        )
    } else {
        (
            end.0.min(end.0 - dx * 0.1),
            end.0.max(end.0 - dx * 0.1),
            z0,
            z1,
        )
    };
    specs.push((
        [rx0, floor, rz0],
        [rx1, floor + 1.1, rz1],
        MaterialId::METAL,
        FeatureKind::RAILING,
        PrimFlags::NONE,
    ));
}

/// Joins `leaf` to `target` (the same column) with a new rising segment, to be built as a lift.
fn add_lift(t: &mut Tissue, leaf: usize, target: usize) {
    let (a, b) = (t.lattice.coords(leaf), t.lattice.coords(target));
    let step: i64 = if b[1] > a[1] { 1 } else { -1 };
    let mut y = a[1] as i64;
    let mut slots = vec![leaf as u32];
    while y != b[1] as i64 {
        y += step;
        let i = t.lattice.index([a[0], y as usize, a[2]]);
        if i != target {
            t.lattice.state[i] = SlotState::Route;
            t.route.push(i as u32);
        }
        slots.push(i as u32);
    }
    t.lift_segments.push(t.segments.len() as u16);
    t.segments.push(Segment {
        slots,
        surround: Surround::Rise,
        engulfable: false,
        walk: u16::MAX,
    });
}

/// Pays off every dead end of `t` and puts a beacon on the knot.
#[allow(clippy::too_many_lines)] // one pass, the four stages in order
pub fn payoff_pass(t: &mut Tissue, rng: &mut Rng) {
    let mut specs: Vec<BoxSpec> = Vec::new();
    let pitch = t.lattice.pitch as f64;
    let mut list: Vec<Payoff> = Vec::new();
    for leaf in dead_ends(t) {
        let words: [u64; 4] = std::array::from_fn(|_| rng.next_u64());
        let leaf = leaf as usize;
        let c = t.lattice.coords(leaf);
        let (cx, cz) = ((c[0] as f64 + 0.5) * pitch, (c[2] as f64 + 0.5) * pitch);
        let floor = c[1] as f64 * pitch;
        // 1. A set piece.
        if words[0].is_multiple_of(3)
            && let Some(id) = try_set(t, leaf, &words)
        {
            list.push(Payoff {
                slot: leaf as u32,
                kind: PayoffKind::Set(id),
            });
            continue;
        }
        // 2. A view: a balcony toward the ray.
        if let Some(d) = view_dir(t, leaf, words[2] as usize) {
            balcony(&mut specs, (cx, cz, floor), d, pitch);
            list.push(Payoff {
                slot: leaf as u32,
                kind: PayoffKind::View(d),
            });
            continue;
        }
        // 3. A lift to another level of the tree.
        if words[3].is_multiple_of(4)
            && let Some(target) = lift_target(t, leaf)
        {
            add_lift(t, leaf, target);
            list.push(Payoff {
                slot: leaf as u32,
                kind: PayoffKind::Lift,
            });
            continue;
        }
        // 4. A light: a lamp on a post, always.
        specs.push((
            [cx - 0.2, floor, cz - 0.2],
            [cx + 0.2, floor + 3.0, cz + 0.2],
            MaterialId::METAL,
            FeatureKind::MISC,
            PrimFlags::NONE,
        ));
        specs.push((
            [cx - 0.4, floor + 3.0, cz - 0.4],
            [cx + 0.4, floor + 3.8, cz + 0.4],
            MaterialId::EMISSIVE,
            FeatureKind::WINDOW,
            PrimFlags::NO_COLLIDE,
        ));
        list.push(Payoff {
            slot: leaf as u32,
            kind: PayoffKind::Light,
        });
    }
    // The knot's beacon: a spire in the corner of the knot's slot, clear of the route.
    if let Some(a) = t.anchors.iter().find(|a| a.kind == AnchorKind::Knot) {
        let c = t.lattice.coords(a.slot as usize);
        let (cx, cz) = ((c[0] as f64 + 0.5) * pitch, (c[2] as f64 + 0.5) * pitch);
        let floor = c[1] as f64 * pitch;
        specs.push((
            [cx + 10.0, floor, cz + 10.0],
            [cx + 12.0, floor + 24.0, cz + 12.0],
            MaterialId::BEACON,
            FeatureKind::MISC,
            PrimFlags::FAR_VISIBLE,
        ));
    }
    let mut e = Emitter::new(&t.lattice);
    for (lo, hi, m, k, f) in specs {
        e.frame_box(lo, hi, m, k, f);
    }
    t.payoffs = Payoffs { list, prims: e.out };
}

/// The rule check (RULE-01): dead ends with no payoff. Empty when the rule holds.
pub fn unpaid(t: &Tissue) -> Vec<u32> {
    dead_ends(t)
        .into_iter()
        .filter(|&l| !t.payoffs.list.iter().any(|p| p.slot == l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellKey, Level};
    use crate::seed::WorldSeed;
    use crate::tissue::{TissueCache, tissue_cell};
    use glam::I64Vec3;

    fn cells(n: i64) -> impl Iterator<Item = CellKey> {
        (0..n).map(|k| {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            )
        })
    }

    /// RULE-01, TIS-05: no dead end is left without a payoff, in any of 2 000 cells; every
    /// kind of payoff occurs; and RULE-02: the cell that holds the knot has a beacon.
    #[test]
    fn every_dead_end_is_paid_off() {
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let (mut sets, mut views, mut lifts, mut lights) = (0, 0, 0, 0);
        let mut leaves = 0;
        for key in cells(2000) {
            let t = tissue_cell(seed, &cache, key);
            assert!(t.connects_anchors(), "{key:?}");
            let bad = unpaid(&t);
            assert!(bad.is_empty(), "{key:?}: unpaid dead ends {bad:?}");
            for p in &t.payoffs.list {
                leaves += 1;
                match p.kind {
                    PayoffKind::Set(_) => sets += 1,
                    PayoffKind::View(_) => views += 1,
                    PayoffKind::Lift => lifts += 1,
                    PayoffKind::Light => lights += 1,
                }
            }
            for &si in &t.lift_segments {
                assert_eq!(t.segments[usize::from(si)].surround, Surround::Rise);
            }
            if t.anchors.iter().any(|a| a.kind == AnchorKind::Knot) {
                assert!(
                    t.payoffs
                        .prims
                        .iter()
                        .any(|p| p.material == MaterialId::BEACON),
                    "{key:?}: no beacon"
                );
            }
        }
        eprintln!("dead ends {leaves}: sets {sets}, views {views}, lifts {lifts}, lights {lights}");
        assert!(sets > 0 && views > 0 && lifts > 0 && lights > 0);
    }
}
