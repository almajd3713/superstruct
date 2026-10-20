//! Corridor kinds of L3 interiors (`docs/spec/levels.md` §6W.5, L3-14..20). They are drawn after
//! the solve, along skeleton edges, from the tiles the solver chose, and never change a tile, so
//! they cannot affect solvability. Every addition keeps the walk links of the block: a rail has
//! a gap where a passage is, a hump is walked over, and nothing collidable stands in a
//! passage.

#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::enum_glob_use,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::match_same_arms,
    clippy::needless_range_loop,
    clippy::manual_is_multiple_of,
    clippy::trivially_copy_pass_by_ref,
    clippy::verbose_bit_mask
)]

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l3_solver::{Dims, opposite};
use crate::l3_tiles::{self as tiles, D, FLOOR, Family, O, SLOT, STOREY, W};
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// The corridor kinds, numbered as the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorridorKind {
    /// L3-14: a rail along a skeleton slot that looks over a double-height room.
    Gallery = 14,
    /// L3-15: a corridor that climbs steps and comes down again.
    Stepped = 15,
    /// L3-16: rooms in a line, each doorway framed.
    Enfilade = 16,
    /// L3-17: a window at the bend of a dog-leg.
    DogLeg = 17,
    /// L3-18: a corridor open to an atrium, railed with a gap.
    OpenToAtrium = 18,
    /// L3-19: a chicane of diagonal steps.
    Skew = 19,
    /// L3-20: a ladder well through the floor above.
    Ladder = 20,
}

/// What the dressing needs to know of a slot.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlotView {
    /// The tile of a Core slot, `u16::MAX` elsewhere.
    pub tile: u16,
    pub core: bool,
    pub skeleton: bool,
    /// Skeleton edge bits over the four horizontal sides.
    pub edges: u8,
    /// Side `k` faces something that is not a Core slot of the same mass.
    pub outer: [bool; 4],
}

/// A piece in block-local quanta.
#[derive(Debug, Clone, Copy)]
pub struct Added {
    pub shape: Shape,
    pub material: MaterialId,
    pub kind: FeatureKind,
    pub flags: PrimFlags,
}

#[derive(Debug, Default)]
pub struct Dress {
    pub pieces: Vec<Added>,
    /// Slots whose floor slab is replaced by a ring round a 1 m square hole (ladder wells).
    pub holes: Vec<usize>,
    /// The kinds drawn, with counts, for the census.
    pub kinds: Vec<(CorridorKind, u16)>,
}

const DIMS: Dims = Dims {
    nx: 8,
    nt: 16,
    nz: 8,
};

fn origin(i: usize) -> [Q; 3] {
    let (x, t, z) = DIMS.coords(i);
    [x as Q * SLOT, t as Q * STOREY, z as Q * SLOT]
}

fn bx(
    o: [Q; 3],
    x: (Q, Q),
    y: (Q, Q),
    z: (Q, Q),
    m: MaterialId,
    kind: FeatureKind,
    flags: PrimFlags,
) -> Added {
    Added {
        shape: Shape::Box {
            aabb: QAabb::new(
                QVec3::new(o[0] + x.0, o[1] + y.0, o[2] + z.0),
                QVec3::new(o[0] + x.1, o[1] + y.1, o[2] + z.1),
            ),
        },
        material: m,
        kind,
        flags,
    }
}

fn family(tile: u16) -> Option<Family> {
    (usize::from(tile) < tiles::N_TILES).then(|| tiles::table()[usize::from(tile)].family)
}

fn sockets(tile: u16) -> Option<[u8; 6]> {
    (usize::from(tile) < tiles::N_TILES).then(|| tiles::table()[usize::from(tile)].sockets())
}

/// The mouths of a straight slot: along x (sides 1 and 3 open, 0 and 2 closed) or along z.
fn straight_axis(s: &[u8; 6]) -> Option<usize> {
    let open = |k: usize| s[k] != W;
    match (open(0), open(1), open(2), open(3)) {
        (true, false, true, false) => Some(2),
        (false, true, false, true) => Some(0),
        _ => None,
    }
}

/// A rail along side `k` of the slot at `o` with a 4 m gap in the middle.
fn gapped_rail(o: [Q; 3], k: usize, out: &mut Vec<Added>) {
    let (along, band) = match k {
        0 => (true, (WALL_IN, WALL_IN + 2)),
        2 => (true, (SLOT - WALL_IN - 2, SLOT - WALL_IN)),
        1 => (false, (SLOT - WALL_IN - 2, SLOT - WALL_IN)),
        _ => (false, (WALL_IN, WALL_IN + 2)),
    };
    let y = (FLOOR, FLOOR + 18);
    for seg in [(0, 32), (96, SLOT)] {
        let piece = if along {
            bx(
                o,
                seg,
                y,
                band,
                MaterialId::METAL,
                FeatureKind::RAILING,
                PrimFlags::NONE,
            )
        } else {
            bx(
                o,
                band,
                y,
                seg,
                MaterialId::METAL,
                FeatureKind::RAILING,
                PrimFlags::NONE,
            )
        };
        out.push(piece);
    }
}

/// How far in from the slot edge a rail stands: past the 5 Q wall band.
const WALL_IN: Q = 6;

/// The corridor kinds of a block's skeleton slots, one draw per slot in index order.
#[allow(clippy::too_many_lines)] // one pass, a kind per arm
pub fn corridor_kinds(seed: WorldSeed, block: CellKey, view: &[SlotView]) -> Dress {
    debug_assert_eq!(block.level, Level::L3);
    let mut rng = Rng::new(hash(seed.0, Level::L3, block.coords, salt::L3_CORR));
    let mut dress = Dress::default();
    let mut counts: std::collections::BTreeMap<CorridorKind, u16> =
        std::collections::BTreeMap::new();
    let nb = |i: usize, side: usize| DIMS.neighbour(i, side);
    let straight_ok = |j: usize| -> bool {
        view[j].core
            && view[j].skeleton
            && sockets(view[j].tile)
                .and_then(|s| straight_axis(&s))
                .is_some()
    };
    for i in 0..DIMS.len() {
        let roll = rng.next_u64();
        let v = view[i];
        if !v.core {
            continue;
        }
        let Some(s) = sockets(v.tile) else {
            continue;
        };
        let Some(fam) = family(v.tile) else {
            continue;
        };
        let walkable = !matches!(fam, Family::Solid | Family::Void(_));
        if !walkable || fam == Family::Atrium && !v.skeleton {
            continue;
        }
        let o = origin(i);
        let mut candidates: Vec<(CorridorKind, u64)> = Vec::new();
        // Sides that look onto an open neighbour without a skeleton edge: rails go there.
        let mut rail_sides: Vec<(usize, CorridorKind)> = Vec::new();
        if v.skeleton && !matches!(fam, Family::Atrium) {
            for k in 0..4 {
                if s[k] != O || v.edges >> k & 1 == 1 {
                    continue;
                }
                let Some(j) = nb(i, k) else {
                    continue;
                };
                match family(view[j].tile) {
                    Some(Family::Atrium) => rail_sides.push((k, CorridorKind::OpenToAtrium)),
                    Some(Family::Void(_)) => rail_sides.push((k, CorridorKind::Gallery)),
                    _ => {}
                }
            }
            // Over a double-height room: the slot below is an atrium or a void.
            if let Some(j) = nb(i, 4)
                && matches!(family(view[j].tile), Some(Family::Atrium | Family::Void(_)))
            {
                for k in 0..4 {
                    if s[k] == O && v.edges >> k & 1 == 0 && !rail_sides.iter().any(|r| r.0 == k) {
                        rail_sides.push((k, CorridorKind::Gallery));
                    }
                }
            }
        }
        if !rail_sides.is_empty() {
            candidates.push((rail_sides[0].1, 6));
        }
        if let Some(axis) = straight_axis(&s)
            && v.skeleton
            && matches!(fam, Family::Corridor | Family::Room)
            && s[4] == tiles::F
            && s[5] == tiles::F
        {
            if fam == Family::Corridor {
                candidates.push((CorridorKind::Stepped, 3));
                candidates.push((CorridorKind::Skew, 3));
            }
            // Rooms in a line: three straight skeleton slots one after the other.
            let (a, b) = if axis == 0 { (3, 1) } else { (0, 2) };
            if nb(i, a).is_some_and(straight_ok) && nb(i, b).is_some_and(straight_ok) {
                candidates.push((CorridorKind::Enfilade, 4));
            }
        }
        // A bend: two skeleton edges at a right angle, with an outside wall to put a window in.
        let edge_count = v.edges.count_ones();
        if v.skeleton
            && edge_count == 2
            && matches!(v.edges, 0b0011 | 0b0110 | 0b1100 | 0b1001)
            && (0..4).any(|k| v.outer[k] && s[k] == W)
        {
            candidates.push((CorridorKind::DogLeg, 5));
        }
        // A ladder well: a room with a room above, joined by nothing else.
        if matches!(fam, Family::Room)
            && let Some(up) = nb(i, 5)
            && view[up].core
            && matches!(family(view[up].tile), Some(Family::Room))
            && sockets(view[up].tile).is_some_and(|u| u[4] == tiles::F)
            && !dress.holes.contains(&up)
            && roll >> 40 & 15 == 0
        {
            candidates.push((CorridorKind::Ladder, 2));
        }
        if candidates.is_empty() {
            continue;
        }
        // One slot in four gets a kind at all, the rest stay plain corridors.
        if roll % 4 != 0 {
            continue;
        }
        let ws: Vec<u32> = candidates.iter().map(|c| c.1 as u32).collect();
        let Some(pick) = Rng::new(roll >> 8).pick_weighted(&ws) else {
            continue;
        };
        let kind = candidates[pick].0;
        match kind {
            CorridorKind::Gallery | CorridorKind::OpenToAtrium => {
                for &(k, _) in &rail_sides {
                    gapped_rail(o, k, &mut dress.pieces);
                }
            }
            CorridorKind::Stepped => {
                let axis = straight_axis(&s).unwrap_or(2);
                hump(o, axis, &mut dress.pieces);
            }
            CorridorKind::Enfilade => {
                let axis = straight_axis(&s).unwrap_or(2);
                lintel(o, axis, &mut dress.pieces);
            }
            CorridorKind::DogLeg => {
                if let Some(k) = (0..4).find(|&k| v.outer[k] && s[k] == W) {
                    window(o, k, &mut dress.pieces);
                }
            }
            CorridorKind::Skew => {
                let axis = straight_axis(&s).unwrap_or(2);
                chicane(o, axis, &mut dress.pieces);
            }
            CorridorKind::Ladder => {
                if let Some(up) = nb(i, 5) {
                    ladder(o, &mut dress.pieces);
                    dress.holes.push(up);
                }
            }
        }
        *counts.entry(kind).or_default() += 1;
    }
    dress.kinds = counts.into_iter().collect();
    let _ = (D, opposite(0), Dir4::PosX);
    dress
}

/// L3-15: steps up 0.75 m (4 of 0.1875 m, 5/16 m treads: a 3 m corridor ceiling leaves 2 m), a landing, and steps down, along
/// the corridor's axis (`axis` 0 along x, 2 along z), inside its 2 m passage.
fn hump(o: [Q; 3], axis: usize, out: &mut Vec<Added>) {
    let steps = 4;
    let rise: Q = 3 * steps; // quanta of 1/16 m
    let run: Q = 5 * steps;
    let (lo, hi) = (48, 80);
    let aabb = |a: (Q, Q), y: (Q, Q)| {
        if axis == 0 {
            QAabb::new(
                QVec3::new(o[0] + a.0, o[1] + y.0, o[2] + lo),
                QVec3::new(o[0] + a.1, o[1] + y.1, o[2] + hi),
            )
        } else {
            QAabb::new(
                QVec3::new(o[0] + lo, o[1] + y.0, o[2] + a.0),
                QVec3::new(o[0] + hi, o[1] + y.1, o[2] + a.1),
            )
        }
    };
    let (up, down) = if axis == 0 {
        (Dir4::PosX, Dir4::NegX)
    } else {
        (Dir4::PosZ, Dir4::NegZ)
    };
    let stairs = |a: (Q, Q), rise_dir: Dir4| Added {
        shape: Shape::StairRun {
            aabb: aabb(a, (FLOOR, FLOOR + rise)),
            rise: rise_dir,
            steps: steps as u16,
        },
        material: MaterialId::CONCRETE,
        kind: FeatureKind::STAIR_TOWER,
        flags: PrimFlags::NONE,
    };
    out.push(stairs((0, run), up));
    out.push(Added {
        shape: Shape::Box {
            aabb: aabb((run, SLOT - run), (FLOOR, FLOOR + rise)),
        },
        material: MaterialId::CONCRETE,
        kind: FeatureKind::WALKWAY,
        flags: PrimFlags::NONE,
    });
    out.push(stairs((SLOT - run, SLOT), down));
}

/// L3-16: a lintel across the doorway at the slot's far end, above 2.65 m.
fn lintel(o: [Q; 3], axis: usize, out: &mut Vec<Added>) {
    let y = (46, 52);
    let piece = if axis == 0 {
        bx(
            o,
            (SLOT - 4, SLOT),
            y,
            (40, 88),
            MaterialId::CONCRETE,
            FeatureKind::MASS,
            PrimFlags::NONE,
        )
    } else {
        bx(
            o,
            (40, 88),
            y,
            (SLOT - 4, SLOT),
            MaterialId::CONCRETE,
            FeatureKind::MASS,
            PrimFlags::NONE,
        )
    };
    out.push(piece);
}

/// L3-17: a lit window pane on the inner face of the outer wall on side `k`.
fn window(o: [Q; 3], k: usize, out: &mut Vec<Added>) {
    let y = (20, 44);
    let along = (40, 88);
    let (xb, zb) = match k {
        0 => (along, (5, 6)),
        2 => (along, (SLOT - 6, SLOT - 5)),
        1 => ((SLOT - 6, SLOT - 5), along),
        _ => ((5, 6), along),
    };
    out.push(bx(
        o,
        xb,
        y,
        zb,
        MaterialId::LIT_WINDOW,
        FeatureKind::WINDOW,
        PrimFlags::NO_COLLIDE,
    ));
}

/// L3-19: two diagonals of 1 m steps, narrowing the passage to 1.5 m and shifting it, which
/// are stand-ins for a corridor at an angle (a turned slab cannot be made in the lattice
/// axes). They stay within 0.25 m of the corridor's own walls.
fn chicane(o: [Q; 3], axis: usize, out: &mut Vec<Added>) {
    // Along the corridor in 8 steps of 1 m; the left wall bulges 0..0.25 m, so only a
    // sawtooth of 0.25 m teeth is added (the collision rule of §7).
    for s in 0..8i32 {
        let a = (s * 16, s * 16 + 16);
        let tooth = 2 + s % 3;
        let (left, right) = ((48 - tooth, 48), (80, 80 + tooth));
        for side in [left, right] {
            let piece = if axis == 0 {
                bx(
                    o,
                    a,
                    (FLOOR, 44),
                    side,
                    MaterialId::CONCRETE,
                    FeatureKind::MASS,
                    PrimFlags::NONE,
                )
            } else {
                bx(
                    o,
                    side,
                    (FLOOR, 44),
                    a,
                    MaterialId::CONCRETE,
                    FeatureKind::MASS,
                    PrimFlags::NONE,
                )
            };
            out.push(piece);
        }
    }
}

/// L3-20: the ladder: two rails and rungs up through the floor above, in the corner of the
/// slot, not collidable (climbing is the ladder mechanic of PW.5.1).
fn ladder(o: [Q; 3], out: &mut Vec<Added>) {
    let x = (100, 102);
    for z in [98, 108] {
        out.push(bx(
            o,
            x,
            (FLOOR, STOREY + 12),
            (z, z + 2),
            MaterialId::METAL,
            FeatureKind::MISC,
            PrimFlags::NO_COLLIDE,
        ));
    }
    let mut y = FLOOR + 6;
    while y < STOREY + 12 {
        out.push(bx(
            o,
            x,
            (y, y + 1),
            (98, 110),
            MaterialId::METAL,
            FeatureKind::MISC,
            PrimFlags::NO_COLLIDE,
        ));
        y += 8;
    }
}

/// The floor slab of a slot with a 1 m hole at `(96..112, 96..112)`: four boxes.
pub fn holed_floor(o: [Q; 3]) -> Vec<Added> {
    let mat = MaterialId::CONCRETE;
    let k = FeatureKind::WALKWAY;
    let f = PrimFlags::NONE;
    let y = (0, FLOOR);
    vec![
        bx(o, (0, 96), y, (0, SLOT), mat, k, f),
        bx(o, (112, SLOT), y, (0, SLOT), mat, k, f),
        bx(o, (96, 112), y, (0, 96), mat, k, f),
        bx(o, (96, 112), y, (112, SLOT), mat, k, f),
    ]
}
