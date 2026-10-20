//! Interior set-piece rooms of L3 (`docs/spec/levels.md` §6W.5, INT-01..19). Like the corridor
//! kinds they are drawn after the solve, from the tiles chosen, and change no tile. A room
//! piece stands only in the four corners of a slot (its middle cross, 1.5 m wide, stays clear,
//! so every door of the slot still joins every other) and only in slots off the skeleton or in
//! open volumes; corridors take the two rooms that are about their walls.

#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::match_same_arms,
    clippy::needless_range_loop,
    clippy::manual_is_multiple_of
)]

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l3_dress::{Added, SlotView};
use crate::l3_solver::Dims;
use crate::l3_tiles::{self as tiles, Family, SLOT, STOREY, W};
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// The rooms, numbered as the registry (INT-01..19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RoomKind {
    Stairs = 1,
    Doors = 2,
    Cubicles = 3,
    Parking = 4,
    LiftLobby = 5,
    Ward = 6,
    Auditorium = 7,
    Stadium = 8,
    ServerHall = 9,
    Cistern = 10,
    Bunks = 11,
    Vessel = 12,
    Shrinking = 13,
    Dining = 14,
    MapRoom = 15,
    GlassRoom = 16,
    ClockRoom = 17,
    Washrooms = 18,
    MirrorCorridor = 19,
}

const DIMS: Dims = Dims {
    nx: 8,
    nt: 16,
    nz: 8,
};
const FLOOR: Q = tiles::FLOOR;

/// A corner block: `x`, `z` ranges of a slot corner, clear of the walls and the middle cross.
fn corner(c: usize) -> ((Q, Q), (Q, Q)) {
    let lo = (8, 52);
    let hi = (76, SLOT - 8);
    match c {
        0 => (lo, lo),
        1 => (hi, lo),
        2 => (lo, hi),
        _ => (hi, hi),
    }
}

struct Put<'a> {
    o: [Q; 3],
    out: &'a mut Vec<Added>,
}

impl Put<'_> {
    fn bx(
        &mut self,
        x: (Q, Q),
        y: (Q, Q),
        z: (Q, Q),
        m: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        if x.0 < x.1 && y.0 < y.1 && z.0 < z.1 {
            self.out.push(Added {
                shape: Shape::Box {
                    aabb: QAabb::new(
                        QVec3::new(self.o[0] + x.0, self.o[1] + y.0, self.o[2] + z.0),
                        QVec3::new(self.o[0] + x.1, self.o[1] + y.1, self.o[2] + z.1),
                    ),
                },
                material: m,
                kind,
                flags,
            });
        }
    }

    fn solid(&mut self, x: (Q, Q), y: (Q, Q), z: (Q, Q), m: MaterialId) {
        self.bx(x, y, z, m, FeatureKind::MISC, PrimFlags::NONE);
    }

    fn free(&mut self, x: (Q, Q), y: (Q, Q), z: (Q, Q), m: MaterialId) {
        self.bx(x, y, z, m, FeatureKind::MISC, PrimFlags::NO_COLLIDE);
    }

    fn stairs(&mut self, x: (Q, Q), y0: Q, z: (Q, Q), rise: Dir4, steps: u16, rise_q: Q) {
        self.out.push(Added {
            shape: Shape::StairRun {
                aabb: QAabb::new(
                    QVec3::new(self.o[0] + x.0, self.o[1] + y0, self.o[2] + z.0),
                    QVec3::new(self.o[0] + x.1, self.o[1] + y0 + rise_q, self.o[2] + z.1),
                ),
                rise,
                steps,
            },
            material: MaterialId::CONCRETE,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        });
    }

    fn pillar(&mut self, cx: Q, cz: Q, r: Q, y: (Q, Q), m: MaterialId) {
        self.out.push(Added {
            shape: Shape::cylinder(
                crate::cell::Axis::Y,
                QVec3::new(self.o[0] + cx, self.o[1] + y.0, self.o[2] + cz),
                y.1 - y.0,
                r,
            ),
            material: m,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        });
    }
}

fn family(tile: u16) -> Option<Family> {
    (usize::from(tile) < tiles::N_TILES).then(|| tiles::table()[usize::from(tile)].family)
}

/// The room of `kind` in the slot at `o`; `col_roll` is the draw of the slot's column, so
/// that slots stacked in one column are dressed alike (a server hall 30 m tall).
fn draw_room(kind: RoomKind, o: [Q; 3], col_roll: u64, roll: u64, out: &mut Vec<Added>) {
    let mut p = Put { o, out };
    let concrete = MaterialId::CONCRETE;
    let metal = MaterialId::METAL;
    let y0 = FLOOR;
    match kind {
        RoomKind::Stairs => {
            // Four flights up to platforms, in the corners, each facing the middle.
            for c in 0..4 {
                let (x, z) = corner(c);
                let dir = [Dir4::PosX, Dir4::NegX, Dir4::PosX, Dir4::NegX][c];
                p.stairs((x.0, x.0 + 30), y0, z, dir, 6, 18);
                p.solid((x.0 + 30, x.1), (y0, y0 + 18), z, concrete);
            }
        }
        RoomKind::Doors => {
            // Door frames, closed, all along the four walls: no collision (they are in the wall).
            for side in 0..4 {
                for k in 0..4 {
                    let a = 16 + k * 28;
                    let (x, z) = match side {
                        0 => ((a, a + 18), (5, 7)),
                        1 => ((SLOT - 7, SLOT - 5), (a, a + 18)),
                        2 => ((a, a + 18), (SLOT - 7, SLOT - 5)),
                        _ => ((5, 7), (a, a + 18)),
                    };
                    p.free(x, (y0, y0 + 40), z, metal);
                    p.free(x, (y0 + 40, y0 + 44), z, concrete);
                }
            }
        }
        RoomKind::Cubicles => {
            for c in 0..4 {
                let (x, z) = corner(c);
                for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (x0, z0) = (x.0 + a * 22, z.0 + b * 22);
                    p.solid((x0, x0 + 2), (y0, y0 + 20), (z0, z0 + 22), concrete);
                    p.solid((x0, x0 + 22), (y0, y0 + 20), (z0, z0 + 2), concrete);
                    p.solid(
                        (x0 + 6, x0 + 18),
                        (y0 + 8, y0 + 10),
                        (z0 + 6, z0 + 16),
                        metal,
                    );
                }
            }
        }
        RoomKind::Parking => {
            for c in 0..4 {
                let (x, z) = corner(c);
                p.solid((x.0, x.0 + 40), (y0, y0 + 22), (z.0 + 4, z.0 + 24), metal);
                p.solid(
                    (x.0 + 6, x.0 + 30),
                    (y0 + 22, y0 + 32),
                    (z.0 + 6, z.0 + 22),
                    metal,
                );
                p.free(
                    (x.0, x.1),
                    (y0, y0 + 1),
                    (z.0 + 30, z.0 + 32),
                    MaterialId::EMISSIVE,
                );
            }
        }
        RoomKind::LiftLobby => {
            // Forty doors: ten on each wall, narrow frames with a lit panel above.
            for side in 0..4 {
                for k in 0..10 {
                    let a = 4 + k * 12;
                    let (x, z) = match side {
                        0 => ((a, a + 8), (5, 7)),
                        1 => ((SLOT - 7, SLOT - 5), (a, a + 8)),
                        2 => ((a, a + 8), (SLOT - 7, SLOT - 5)),
                        _ => ((5, 7), (a, a + 8)),
                    };
                    p.free(x, (y0, y0 + 36), z, metal);
                    p.free(x, (y0 + 38, y0 + 41), z, MaterialId::LIT_WINDOW);
                }
            }
        }
        RoomKind::Ward => {
            // Sixteen identical beds, four in each corner.
            for c in 0..4 {
                let (x, z) = corner(c);
                for k in 0..4 {
                    let z0 = z.0 + k * 11;
                    p.solid(
                        (x.0, x.0 + 34),
                        (y0 + 4, y0 + 11),
                        (z0, z0 + 9),
                        MaterialId::GRATING,
                    );
                    p.solid((x.0, x.0 + 4), (y0, y0 + 16), (z0, z0 + 9), metal);
                }
            }
        }
        RoomKind::Auditorium => {
            // Tiered seats in two corners facing the others: steps with a row of seats on each.
            for c in [2usize, 3] {
                let (x, z) = corner(c);
                p.stairs((x.0, x.1), y0, (z.0, z.0 + 30), Dir4::NegZ, 5, 15);
                for k in 0..5 {
                    let zz = z.1 - 6 - k * 6;
                    p.solid(
                        (x.0 + 2, x.1 - 2),
                        (y0 + 3 * (4 - k) + 3, y0 + 3 * (4 - k) + 8),
                        (zz, zz + 2),
                        metal,
                    );
                }
            }
        }
        RoomKind::Stadium => {
            for c in 0..4 {
                let (x, z) = corner(c);
                let dir = if c % 2 == 0 { Dir4::PosX } else { Dir4::NegX };
                p.stairs((x.0, x.0 + 40), y0, z, dir, 8, 24);
            }
        }
        RoomKind::ServerHall => {
            // Racks in each corner, a stack per storey that lines up with the one above.
            for c in 0..4 {
                let (x, z) = corner(c);
                if (col_roll >> c) & 1 == 0 {
                    continue;
                }
                for k in 0..3 {
                    let x0 = x.0 + k * 14;
                    p.solid((x0, x0 + 11), (0, STOREY), (z.0, z.1), metal);
                    p.free(
                        (x0 + 1, x0 + 10),
                        (20, 22),
                        (z.0 - 1, z.0),
                        MaterialId::LIT_WINDOW,
                    );
                }
            }
        }
        RoomKind::Cistern => {
            for c in 0..4 {
                let (x, z) = corner(c);
                for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    p.pillar(
                        x.0 + 11 + a * 22,
                        z.0 + 11 + b * 22,
                        6,
                        (0, STOREY),
                        concrete,
                    );
                }
            }
        }
        RoomKind::Bunks => {
            for c in 0..4 {
                let (x, z) = corner(c);
                for k in 0..2 {
                    let z0 = z.0 + k * 22;
                    for tier in 0..3 {
                        let y = y0 + 6 + tier * 14;
                        p.solid(
                            (x.0, x.0 + 38),
                            (y, y + 3),
                            (z0, z0 + 16),
                            MaterialId::GRATING,
                        );
                    }
                    p.solid((x.0, x.0 + 3), (y0, y0 + 48), (z0, z0 + 3), metal);
                    p.solid(
                        (x.0 + 35, x.0 + 38),
                        (y0, y0 + 48),
                        (z0 + 13, z0 + 16),
                        metal,
                    );
                }
            }
        }
        RoomKind::Vessel => {
            // A vessel on one corner: plates of cylinders that swell and narrow.
            let (x, z) = corner((roll % 4) as usize);
            let (cx, cz) = (i32::midpoint(x.0, x.1), i32::midpoint(z.0, z.1));
            for (k, r) in [8, 14, 20, 22, 20, 14, 8].into_iter().enumerate() {
                let y = y0 + k as Q * 8;
                p.pillar(cx, cz, r.min(22), (y, y + 8), metal);
            }
        }
        RoomKind::Shrinking => {}
        RoomKind::Dining => {
            for c in 0..4 {
                let (x, z) = corner(c);
                p.solid(
                    (x.0 + 4, x.1 - 4),
                    (y0 + 12, y0 + 15),
                    (z.0 + 12, z.0 + 22),
                    concrete,
                );
                p.solid(
                    (x.0 + 8, x.1 - 8),
                    (y0, y0 + 12),
                    (z.0 + 15, z.0 + 19),
                    metal,
                );
                for zz in [z.0 + 2, z.0 + 26] {
                    p.solid(
                        (x.0 + 4, x.1 - 4),
                        (y0 + 6, y0 + 8),
                        (zz, zz + 6),
                        MaterialId::GRATING,
                    );
                }
            }
        }
        RoomKind::MapRoom => {
            // A plinth with a relief of the surroundings: boxes of hashed heights.
            let (x, z) = corner((roll % 4) as usize);
            p.solid(x, (y0, y0 + 12), z, concrete);
            let mut r = Rng::new(roll);
            for a in 0..6 {
                for b in 0..6 {
                    let h = 1 + r.below(6) as Q;
                    let (x0, z0) = (x.0 + 2 + a * 7, z.0 + 2 + b * 7);
                    p.solid(
                        (x0, x0 + 6),
                        (y0 + 12, y0 + 12 + h),
                        (z0, z0 + 6),
                        MaterialId::WARM,
                    );
                }
            }
        }
        RoomKind::GlassRoom => {
            for side in 0..4 {
                let (x, z) = match side {
                    0 => ((8, SLOT - 8), (6, 7)),
                    1 => ((SLOT - 7, SLOT - 6), (8, SLOT - 8)),
                    2 => ((8, SLOT - 8), (SLOT - 7, SLOT - 6)),
                    _ => ((6, 7), (8, SLOT - 8)),
                };
                p.free(x, (y0 + 4, y0 + 44), z, MaterialId::LIT_WINDOW);
            }
        }
        RoomKind::ClockRoom => {
            // A clock face on the -z wall: twelve ticks round a ring, two hands.
            let (cx, cy) = (64, y0 + 30);
            // The twelve ticks at radius 18 (a table: the generator uses no trigonometry).
            for (dx, dy) in [
                (0, 18),
                (9, 16),
                (16, 9),
                (18, 0),
                (16, -9),
                (9, -16),
                (0, -18),
                (-9, -16),
                (-16, -9),
                (-18, 0),
                (-16, 9),
                (-9, 16),
            ] {
                p.free(
                    (cx + dx - 1, cx + dx + 1),
                    (cy + dy - 1, cy + dy + 1),
                    (6, 8),
                    MaterialId::EMISSIVE,
                );
            }
            p.free((cx - 1, cx + 1), (cy, cy + 14), (8, 9), metal);
            p.free((cx, cx + 10), (cy - 1, cy + 1), (8, 9), metal);
        }
        RoomKind::Washrooms => {
            for c in 0..4 {
                let (x, z) = corner(c);
                for k in 0..3 {
                    let x0 = x.0 + k * 15;
                    p.solid(
                        (x0, x0 + 2),
                        (y0, y0 + 36),
                        (z.0, z.1 - 8),
                        MaterialId::GRATING,
                    );
                }
                p.pillar(x.0 + 8, z.1 - 4, 2, (y0 + 28, y0 + 40), metal);
            }
        }
        RoomKind::MirrorCorridor => {
            for z0 in [4, SLOT - 5] {
                p.free(
                    (8, SLOT - 8),
                    (y0 + 3, y0 + 44),
                    (z0, z0 + 1),
                    MaterialId::LIT_WINDOW,
                );
            }
        }
    }
}

/// What a kind may be put in: an open volume, a room or a corridor.
fn kinds_for(fam: Family) -> &'static [(RoomKind, u32)] {
    use RoomKind::{
        Auditorium, Bunks, Cistern, ClockRoom, Cubicles, Dining, Doors, GlassRoom, LiftLobby,
        MapRoom, MirrorCorridor, Parking, ServerHall, Shrinking, Stadium, Stairs, Vessel, Ward,
        Washrooms,
    };
    match fam {
        Family::Atrium => &[
            (Stadium, 3),
            (ServerHall, 4),
            (Cistern, 3),
            (Vessel, 2),
            (Parking, 3),
            (Auditorium, 2),
        ],
        Family::Room => &[
            (Stairs, 2),
            (Doors, 3),
            (Cubicles, 4),
            (LiftLobby, 2),
            (Ward, 3),
            (Bunks, 3),
            (Dining, 3),
            (MapRoom, 2),
            (GlassRoom, 3),
            (ClockRoom, 2),
            (Washrooms, 3),
        ],
        Family::Corridor => &[(Shrinking, 3), (MirrorCorridor, 3)],
        _ => &[],
    }
}

/// The set-piece rooms of a block: one in about one eligible slot in five, drawn per slot in
/// index order, with the kind drawn from the slot's column so that stacked slots agree.
pub fn rooms(
    seed: WorldSeed,
    block: CellKey,
    view: &[SlotView],
    skip: &[usize],
) -> (Vec<Added>, Vec<(RoomKind, u16)>) {
    debug_assert_eq!(block.level, Level::L3);
    let mut rng = Rng::new(hash(seed.0, Level::L3, block.coords, salt::L3_PAYOFF));
    let mut out = Vec::new();
    let mut counts: std::collections::BTreeMap<RoomKind, u16> = std::collections::BTreeMap::new();
    for i in 0..DIMS.len() {
        let roll = rng.next_u64();
        let v = view[i];
        if !v.core {
            continue;
        }
        let Some(fam) = family(v.tile) else {
            continue;
        };
        let options = kinds_for(fam);
        // Rooms and atria off the skeleton; corridors on it have their walls dressed only.
        let eligible = match fam {
            Family::Corridor => v.skeleton,
            _ => !v.skeleton,
        };
        if options.is_empty() || !eligible || skip.contains(&i) {
            continue;
        }
        let (x, t, z) = DIMS.coords(i);
        // The kind is the column's (and the storey group's), the place is the slot's.
        let col_roll = hash(
            seed.0,
            Level::L3,
            block.coords + glam::I64Vec3::new(x as i64, (t / 4) as i64, z as i64),
            salt::L3_PAYOFF,
        );
        if roll % 12 != 0 {
            continue;
        }
        let ws: Vec<u32> = options.iter().map(|o| o.1).collect();
        let Some(k) = Rng::new(col_roll).pick_weighted(&ws) else {
            continue;
        };
        let kind = options[k].0;
        let sockets = tiles::table()[usize::from(v.tile)].sockets();
        if kind == RoomKind::Shrinking || kind == RoomKind::MirrorCorridor {
            // Along straight corridors only.
            let straight =
                (sockets[0] != W && sockets[2] != W && sockets[1] == W && sockets[3] == W)
                    || (sockets[1] != W && sockets[3] != W && sockets[0] == W && sockets[2] == W);
            if !straight {
                continue;
            }
        }
        let o = [x as Q * SLOT, t as Q * STOREY, z as Q * SLOT];
        if kind == RoomKind::Shrinking {
            shrink(o, sockets[0] != W, x, z, &mut out);
        } else if kind == RoomKind::MirrorCorridor && sockets[0] != W {
            // A corridor along z: the mirrors drawn for one along x, turned.
            let mut tmp = Vec::new();
            draw_room(kind, o, col_roll, roll >> 8, &mut tmp);
            out.extend(tmp.into_iter().map(|a| transpose(a, o)));
        } else {
            draw_room(kind, o, col_roll, roll >> 8, &mut out);
        }
        *counts.entry(kind).or_default() += 1;
    }
    (out, counts.into_iter().collect())
}

/// L3-INT-13: the corridor's walls step inward along it, a tooth a little longer each slot
/// (up to 0.5 m a side, leaving a 1 m passage).
fn shrink(o: [Q; 3], along_z: bool, x: usize, z: usize, out: &mut Vec<Added>) {
    let idx = if along_z { z } else { x } as Q;
    let mut p = Put { o, out };
    for s in 0..8 {
        let a = (s * 16, s * 16 + 16);
        let grow = 2 + (idx * 8 + s) % 7;
        for side in [(48 - grow, 48), (80, 80 + grow)] {
            if along_z {
                p.solid(side, (FLOOR, 44), a, MaterialId::CONCRETE);
            } else {
                p.solid(a, (FLOOR, 44), side, MaterialId::CONCRETE);
            }
        }
    }
}

/// An added box with x and z exchanged about the slot (a piece drawn for a corridor along z
/// put on one along x).
fn transpose(a: Added, o: [Q; 3]) -> Added {
    match a.shape {
        Shape::Box { aabb } => Added {
            shape: Shape::Box {
                aabb: QAabb::new(
                    QVec3::new(
                        o[0] + (aabb.min.z - o[2]),
                        aabb.min.y,
                        o[2] + (aabb.min.x - o[0]),
                    ),
                    QVec3::new(
                        o[0] + (aabb.max.z - o[2]),
                        aabb.max.y,
                        o[2] + (aabb.max.x - o[0]),
                    ),
                ),
            },
            ..a
        },
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No collidable piece of any room stands in the middle cross of its slot (1.5 m wide,
    /// from the floor to 2 m), so the doors of a slot stay joined; corridors keep 1 m.
    #[test]
    fn rooms_leave_the_middle_of_a_slot_clear() {
        let kinds = [
            RoomKind::Stairs,
            RoomKind::Doors,
            RoomKind::Cubicles,
            RoomKind::Parking,
            RoomKind::LiftLobby,
            RoomKind::Ward,
            RoomKind::Auditorium,
            RoomKind::Stadium,
            RoomKind::ServerHall,
            RoomKind::Cistern,
            RoomKind::Bunks,
            RoomKind::Vessel,
            RoomKind::Dining,
            RoomKind::MapRoom,
            RoomKind::GlassRoom,
            RoomKind::ClockRoom,
            RoomKind::Washrooms,
        ];
        for kind in kinds {
            for roll in 0..8u64 {
                let mut out = Vec::new();
                draw_room(kind, [0, 0, 0], u64::MAX, roll, &mut out);
                assert!(!out.is_empty(), "{kind:?} draws nothing");
                for a in &out {
                    if a.flags.contains(PrimFlags::NO_COLLIDE) {
                        continue;
                    }
                    let b = a.shape.aabb();
                    if b.min.y >= FLOOR + 32 {
                        continue;
                    }
                    let clear_x = b.max.x <= 52 || b.min.x >= 76;
                    let clear_z = b.max.z <= 52 || b.min.z >= 76;
                    assert!(clear_x && clear_z, "{kind:?} blocks the middle: {b:?}");
                    assert!(
                        b.min.x >= 0 && b.max.x <= SLOT && b.min.z >= 0 && b.max.z <= SLOT,
                        "{kind:?} leaves its slot: {b:?}"
                    );
                }
            }
        }
    }
}
