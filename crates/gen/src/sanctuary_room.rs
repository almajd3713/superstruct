//! The sanctuary room (`docs/spec/levels.md` §5.5): a walled `WARM` room inside the sanctuary
//! zone, with one door and a wide opening on the opposite wall that frames the view outward.
//!
//! The room stands 4 m inside the zone's footprint. The zone lies fully on deck, so the ring
//! around the room is deck too and the door always opens onto walkable floor.

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::prim::{
    CellOutput, ChildConstraint, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3,
    Shape, ZoneKind,
};
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// Gap between the zone's edge and the room's walls.
const INSET: Q = 4 * M;
const SIDE: Q = 24 * M;
const WALL: Q = M / 2;
pub const HEIGHT: Q = 6 * M;
const DOOR_W: Q = 3 * M;
const DOOR_H: Q = 11 * M / 4;
const WINDOW_W: Q = 12 * M;
const WINDOW_SILL: Q = M;
const WINDOW_TOP: Q = 4 * M;

/// The room's footprint and which wall holds the door, in the L2 cell's local quanta. Sides are
/// numbered -z, +x, +z, -x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Room {
    /// Footprint at floor height.
    pub floor: QAabb,
    pub door_side: u8,
}

impl Room {
    /// The wall holding the vista opening, opposite the door.
    pub const fn window_side(&self) -> u8 {
        (self.door_side + 2) % 4
    }

    pub fn centre(&self) -> [Q; 2] {
        [
            i32::midpoint(self.floor.min.x, self.floor.max.x),
            i32::midpoint(self.floor.min.z, self.floor.max.z),
        ]
    }

    /// Walls with their openings, and the roof. All collide.
    pub fn primitives(&self) -> Vec<Primitive> {
        let f = self.floor;
        let mut out = Vec::new();
        let mut add = |lo: [Q; 3], hi: [Q; 3]| {
            out.push(Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(
                        QVec3::new(lo[0], lo[1], lo[2]),
                        QVec3::new(hi[0], hi[1], hi[2]),
                    ),
                },
                material: MaterialId::WARM,
                kind: FeatureKind::SANCTUARY,
                flags: PrimFlags::NONE,
            });
        };
        add(
            [f.min.x, f.min.y + HEIGHT, f.min.z],
            [f.max.x, f.min.y + HEIGHT + WALL, f.max.z],
        );
        for side in 0..4u8 {
            // Position along the wall and its extent.
            let (u0, u1) = if side % 2 == 0 {
                (f.min.x, f.max.x)
            } else {
                (f.min.z + WALL, f.max.z - WALL)
            };
            let mid = i32::midpoint(u0, u1);
            let (hole_w, lo, hi) = if side == self.door_side {
                (DOOR_W, 0, DOOR_H)
            } else if side == self.window_side() {
                (WINDOW_W, WINDOW_SILL, WINDOW_TOP)
            } else {
                (0, 0, 0)
            };
            let mut wall = |a: Q, b: Q, y0: Q, y1: Q| {
                if a >= b || y0 >= y1 {
                    return;
                }
                let y = f.min.y;
                match side {
                    0 => add([a, y + y0, f.min.z], [b, y + y1, f.min.z + WALL]),
                    2 => add([a, y + y0, f.max.z - WALL], [b, y + y1, f.max.z]),
                    3 => add([f.min.x, y + y0, a], [f.min.x + WALL, y + y1, b]),
                    _ => add([f.max.x - WALL, y + y0, a], [f.max.x, y + y1, b]),
                }
            };
            if hole_w == 0 {
                wall(u0, u1, 0, HEIGHT);
                continue;
            }
            wall(u0, mid - hole_w / 2, 0, HEIGHT);
            wall(mid + hole_w / 2, u1, 0, HEIGHT);
            wall(mid - hole_w / 2, mid + hole_w / 2, 0, lo);
            wall(mid - hole_w / 2, mid + hole_w / 2, hi, HEIGHT);
        }
        out
    }
}

/// The room of the sanctuary zone `out` carries, if it carries one.
pub fn room_of(seed: WorldSeed, out: &CellOutput) -> Option<Room> {
    let zone = out.child_constraints.iter().find_map(|c| match c {
        ChildConstraint::Zone {
            aabb,
            kind: ZoneKind::Sanctuary,
        } => Some(*aabb),
        _ => None,
    })?;
    debug_assert_eq!(out.key.level, Level::L2);
    Some(room_in(seed, out.key, zone))
}

fn room_in(seed: WorldSeed, cell: CellKey, zone: QAabb) -> Room {
    let h = hash(seed.0, Level::L2, cell.coords, salt::SANCTUARY_ROOM);
    let (x, z) = (zone.min.x + INSET, zone.min.z + INSET);
    Room {
        floor: QAabb::new(
            QVec3::new(x, zone.min.y, z),
            QVec3::new(x + SIDE, zone.min.y + HEIGHT + WALL, z + SIDE),
        ),
        door_side: (h % 4) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_fits_the_zone_and_has_a_door_and_a_window() {
        let zone = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(32 * M, 16 * M, 32 * M));
        for x in 0..16 {
            let cell = CellKey::new(Level::L2, glam::I64Vec3::new(x, 0, 0));
            let room = room_in(WorldSeed(1), cell, zone);
            let prims = room.primitives();
            assert!(prims.iter().all(|p| {
                let b = p.aabb();
                b.min.x >= zone.min.x
                    && b.max.x <= zone.max.x
                    && b.min.z >= zone.min.z
                    && b.max.z <= zone.max.z
                    && b.max.y <= zone.max.y
            }));
            // A walker's capsule fits through the door.
            let c = room.centre();
            let (dx, dz) = match room.door_side {
                0 => (c[0], room.floor.min.z + WALL / 2),
                2 => (c[0], room.floor.max.z - WALL / 2),
                3 => (room.floor.min.x + WALL / 2, c[1]),
                _ => (room.floor.max.x - WALL / 2, c[1]),
            };
            let y = room.floor.min.y + M;
            assert!(!prims.iter().any(|p| {
                let b = p.aabb();
                (b.min.x..b.max.x).contains(&dx)
                    && (b.min.z..b.max.z).contains(&dz)
                    && (b.min.y..b.max.y).contains(&y)
            }));
        }
    }
}
