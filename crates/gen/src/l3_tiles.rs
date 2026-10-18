//! The L3 tile set: 205 tiles with sockets, weights and geometry (`docs/spec/levels.md` §6.5).
//!
//! A slot is 8 m × 4 m × 8 m: x and z in `[0, 128)` quanta, y in `[0, 64)`. Sides are numbered
//! -z, +x, +z, -x, down, up, as in the solver. Rotations are quarter-turns that carry side 0
//! (-z) to side `r`.

use std::sync::OnceLock;

use crate::l3_solver::TileSet;
use crate::prim::{Dir4, FeatureKind, MaterialId, Q, QAabb, QVec3, Shape};
use crate::regime::StyleId;

/// Slot plan size in quanta (8 m).
pub const SLOT: Q = 128;
/// Slot height in quanta (4 m).
pub const STOREY: Q = 64;
/// Wall thickness in quanta.
/// Wall thickness. Two back-to-back walls leave a 10 Q gap in the floor plan, which is the least
/// the walkability extractor keeps as a surface (`MIN_WIDTH`).
pub const WALL: Q = 5;
/// Floor slab thickness.
pub const FLOOR: Q = 4;
const DOOR: (Q, Q) = (48, 80);
const DOOR_TOP: Q = 48;

// Horizontal sockets.
pub const W: u8 = 0;
pub const O: u8 = 1;
pub const D: u8 = 2;
// Vertical sockets: floor, voids of depth k above a floor, spiral with rotation r.
pub const F: u8 = 0;
pub const fn void_socket(k: u8) -> u8 {
    k
}
pub const fn spiral_socket(r: u8) -> u8 {
    4 + r
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Solid,
    Room,
    Atrium,
    /// Depth 1 to 3.
    Void(u8),
    Corridor,
    SpiralBase,
    Spiral,
    SpiralTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileInfo {
    pub family: Family,
    /// Sockets on -z, +x, +z, -x.
    pub sides: [u8; 4],
    pub down: u8,
    pub up: u8,
    pub rot: u8,
}

impl TileInfo {
    /// Sockets in the solver's side order.
    pub fn sockets(&self) -> [u8; 6] {
        [
            self.sides[0],
            self.sides[1],
            self.sides[2],
            self.sides[3],
            self.down,
            self.up,
        ]
    }

    pub fn walkable(&self) -> bool {
        !matches!(self.family, Family::Solid | Family::Void(_))
    }

    pub fn is_spiral(&self) -> bool {
        matches!(
            self.family,
            Family::SpiralBase | Family::Spiral | Family::SpiralTop
        )
    }
}

pub const N_TILES: usize = 205;

/// Tile ids of the families, in table order.
pub const SOLID: usize = 0;
pub const ROOM0: usize = 1;
pub const ATRIUM0: usize = ROOM0 + 81;
pub const VOID1_0: usize = ATRIUM0 + 16;
pub const VOID2_0: usize = VOID1_0 + 32;
pub const VOID3_0: usize = VOID2_0 + 32;
pub const CORRIDOR0: usize = VOID3_0 + 16;
pub const SPIRAL_BASE0: usize = CORRIDOR0 + 15;
pub const SPIRAL0: usize = SPIRAL_BASE0 + 4;
pub const SPIRAL_TOP0: usize = SPIRAL0 + 4;

fn build_table() -> Vec<TileInfo> {
    let mut t = Vec::with_capacity(N_TILES);
    let tile = |family, sides, down, up, rot| TileInfo {
        family,
        sides,
        down,
        up,
        rot,
    };
    t.push(tile(Family::Solid, [W; 4], F, F, 0));
    for code in 0..81u32 {
        let s = |k: u32| u8::try_from(code / 3u32.pow(k) % 3).unwrap_or(0);
        t.push(tile(Family::Room, [s(0), s(1), s(2), s(3)], F, F, 0));
    }
    let wo = |code: u32| -> [u8; 4] {
        std::array::from_fn(|k| u8::try_from(code >> k & 1).unwrap_or(0))
    };
    for code in 0..16 {
        t.push(tile(Family::Atrium, wo(code), F, void_socket(1), 0));
    }
    for k in 1..=3u8 {
        let ups: &[u8] = if k < 3 { &[F, k + 1] } else { &[F] };
        for &up in ups {
            for code in 0..16 {
                t.push(tile(Family::Void(k), wo(code), void_socket(k), up, 0));
            }
        }
    }
    // Corridors by mouth mask: straight 2, bend 4, tee 4, cross 1, dead end 4.
    let masks: Vec<u8> = (1..16u8).collect();
    for m in masks {
        let sides = std::array::from_fn(|k| if m >> k & 1 == 1 { D } else { W });
        t.push(tile(Family::Corridor, sides, F, F, 0));
    }
    for (family, down, up_is_p) in [
        (Family::SpiralBase, Some(F), true),
        (Family::Spiral, None, true),
        (Family::SpiralTop, None, false),
    ] {
        for r in 0..4u8 {
            let sides = std::array::from_fn(|k| if k == usize::from(r) { D } else { W });
            let p = spiral_socket(r);
            let (d, u) = match family {
                Family::SpiralBase => (down.unwrap_or(F), p),
                Family::Spiral => (p, p),
                _ => (p, F),
            };
            let _ = up_is_p;
            t.push(tile(family, sides, d, u, r));
        }
    }
    t
}

/// Every tile, indexed by tile id.
pub fn table() -> &'static [TileInfo] {
    static TABLE: OnceLock<Vec<TileInfo>> = OnceLock::new();
    TABLE.get_or_init(build_table)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Weights per tile for a regime (§6.5): integers, rounded toward zero, at least 1.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // small positive values
pub fn weights(density: u8, verticality: u8, style: StyleId) -> Vec<u32> {
    let d = f32::from(density) / 255.0;
    let v = f32::from(verticality) / 255.0;
    let f = |s: u8| match s {
        O => 3.0,
        _ => 2.0,
    };
    table()
        .iter()
        .map(|t| {
            let sides: f32 = t.sides.iter().map(|&s| f(s)).product();
            let mut w = match t.family {
                Family::Solid => lerp(300.0, 3000.0, d),
                Family::Room => sides,
                Family::Atrium => sides * lerp(0.1, 0.6, v),
                Family::Void(k) => {
                    let variants = if k < 3 { 2.0 } else { 1.0 };
                    sides * lerp(0.6, 0.1, d) / variants
                }
                Family::Corridor => lerp(20.0, 160.0, d),
                _ => lerp(10.0, 60.0, v),
            };
            let doubled = match style {
                StyleId::PIPEWORKS => t.family == Family::Corridor,
                StyleId::HOLLOW_EXPANSE => matches!(t.family, Family::Void(_) | Family::Atrium),
                StyleId::PILLAR_FOREST => t.is_spiral(),
                _ => false,
            };
            if doubled {
                w *= 2.0;
            }
            if style == StyleId::HOLLOW_EXPANSE && t.family == Family::Solid {
                w *= 0.5;
            }
            (w as u32).max(1)
        })
        .collect()
}

/// The solver's tile set for a regime.
pub fn tileset(density: u8, verticality: u8, style: StyleId) -> TileSet {
    let w = weights(density, verticality, style);
    let tiles: Vec<([u8; 6], u32)> = table()
        .iter()
        .zip(w)
        .map(|(t, w)| (t.sockets(), w))
        .collect();
    TileSet::new(&tiles)
}

/// One piece of a tile in slot-local quanta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub shape: Shape,
    pub material: MaterialId,
    pub kind: FeatureKind,
}

/// A plan rectangle `[x0, x1) × [z0, z1)` and a height range.
#[derive(Clone, Copy)]
struct B {
    x: (Q, Q),
    z: (Q, Q),
    y: (Q, Q),
}

fn b(x: (Q, Q), z: (Q, Q), y: (Q, Q)) -> B {
    B { x, z, y }
}

fn rotate_box(bx: B, r: u8) -> B {
    let mut bx = bx;
    for _ in 0..r {
        // A quarter-turn carrying -z to +x: (x, z) -> (SLOT - z, x).
        bx = B {
            x: (SLOT - bx.z.1, SLOT - bx.z.0),
            z: bx.x,
            y: bx.y,
        };
    }
    bx
}

fn rotate_dir(d: Dir4, r: u8) -> Dir4 {
    let mut d = d;
    for _ in 0..r {
        d = match d {
            Dir4::PosX => Dir4::PosZ,
            Dir4::PosZ => Dir4::NegX,
            Dir4::NegX => Dir4::NegZ,
            Dir4::NegZ => Dir4::PosX,
        };
    }
    d
}

fn aabb(bx: B) -> QAabb {
    QAabb::new(
        QVec3::new(bx.x.0, bx.y.0, bx.z.0),
        QVec3::new(bx.x.1, bx.y.1, bx.z.1),
    )
}

struct Out {
    pieces: Vec<Piece>,
    kind: FeatureKind,
}

impl Out {
    fn add(&mut self, bx: B, material: MaterialId) {
        if bx.x.0 < bx.x.1 && bx.z.0 < bx.z.1 && bx.y.0 < bx.y.1 {
            self.pieces.push(Piece {
                shape: Shape::Box { aabb: aabb(bx) },
                material,
                kind: self.kind,
            });
        }
    }
}

/// The extent along a wall on `side` and the band it occupies across, in quanta.
fn wall_band(side: usize) -> ((Q, Q), (Q, Q)) {
    match side {
        0 | 3 => ((0, SLOT), (0, WALL)),
        _ => ((0, SLOT), (SLOT - WALL, SLOT)),
    }
}

/// The wall on `side` with sockets `socket`, between heights `y`. Walls on x sides are inset by
/// the thickness at both ends so corners are not doubled.
/// How far a doorway slab reaches into the room, in quanta. Together with the same slab of the
/// slot across the door it forms one floor strip at least `MIN_WIDTH` (10 Q) deep, which the
/// walkability extractor needs to link the two floors across the 8 Q of wall.
const THRESHOLD: Q = 10;

/// A floor slab through the doorway on `side`, `opening` wide.
fn threshold(out: &mut Out, side: usize, opening: (Q, Q)) {
    let depth = if side == 0 || side == 3 {
        (0, THRESHOLD)
    } else {
        (SLOT - THRESHOLD, SLOT)
    };
    let bx = if side.is_multiple_of(2) {
        b(opening, depth, (0, FLOOR))
    } else {
        b(depth, opening, (0, FLOOR))
    };
    out.add(bx, MaterialId::CONCRETE);
}

fn wall(out: &mut Out, side: usize, socket: u8, y: (Q, Q), material: MaterialId) {
    if socket == O {
        return;
    }
    // Along-wall range and the thickness band.
    let (along, band) = wall_band(side);
    let put = |out: &mut Out, a: (Q, Q), y: (Q, Q)| {
        let bx = if side.is_multiple_of(2) {
            b(a, band, y)
        } else {
            b(band, a, y)
        };
        out.add(bx, material);
    };
    if socket == D {
        put(out, (along.0, DOOR.0), y);
        put(out, (DOOR.1, along.1), y);
        put(out, DOOR, (DOOR_TOP.max(y.0), y.1));
        threshold(out, side, DOOR);
    } else {
        put(out, along, y);
    }
}

fn room_pieces(info: &TileInfo, out: &mut Out, wall_material: MaterialId) {
    out.add(b((0, SLOT), (0, SLOT), (0, FLOOR)), MaterialId::CONCRETE);
    for (side, &s) in info.sides.iter().enumerate() {
        wall(out, side, s, (0, STOREY), wall_material);
    }
}

fn corridor_pieces(info: &TileInfo, out: &mut Out) {
    out.add(b((0, SLOT), (0, SLOT), (0, FLOOR)), MaterialId::GRATING);
    out.add(
        b((0, SLOT), (0, SLOT), (DOOR_TOP, STOREY)),
        MaterialId::CONCRETE,
    );
    // The plan splits at 48 and 80 into nine cells. The middle and the arms towards mouths are
    // the passage; everything else is solid between the floor and the ceiling.
    let edges = [(0, DOOR.0), DOOR, (DOOR.1, SLOT)];
    for (iz, &z) in edges.iter().enumerate() {
        for (ix, &x) in edges.iter().enumerate() {
            let open = match (ix, iz) {
                (1, 1) => true,
                (1, 0) => info.sides[0] == D,
                (2, 1) => info.sides[1] == D,
                (1, 2) => info.sides[2] == D,
                (0, 1) => info.sides[3] == D,
                _ => false,
            };
            if !open {
                out.add(b(x, z, (FLOOR, DOOR_TOP)), MaterialId::CONCRETE);
            }
        }
    }
}

/// The spiral stair in rotation 0 (§6.5.1), added through `add` so the caller can rotate it.
fn spiral_pieces(info: &TileInfo, out: &mut Out) {
    let r = info.rot;
    let add = |out: &mut Out, bx: B, m: MaterialId| out.add(rotate_box(bx, r), m);
    let concrete = MaterialId::CONCRETE;
    let grating = MaterialId::GRATING;
    // Pillar.
    add(out, b((32, 96), (32, 96), (0, STOREY)), concrete);
    // Walls, with the door on side r (side 0 before rotation).
    let wall_rot = |out: &mut Out, side: usize, door: bool| {
        let (along, band) = wall_band(side);
        let put = |out: &mut Out, a: (Q, Q), y: (Q, Q)| {
            let bx = if side.is_multiple_of(2) {
                b(a, band, y)
            } else {
                b(band, a, y)
            };
            out.add(rotate_box(bx, r), concrete);
        };
        if door {
            put(out, (along.0, DOOR.0), (0, STOREY));
            put(out, (DOOR.1, along.1), (0, STOREY));
            put(out, DOOR, (DOOR_TOP, STOREY));
            let depth = (0, THRESHOLD);
            out.add(rotate_box(b(DOOR, depth, (0, FLOOR)), r), concrete);
        } else {
            put(out, along, (0, STOREY));
        }
    };
    for side in 0..4 {
        wall_rot(out, side, side == 0);
    }
    // Floor: the whole plan for the base, otherwise only c0 and the lane-0 flat.
    if info.family == Family::SpiralBase {
        add(out, b((0, SLOT), (0, SLOT), (0, FLOOR)), concrete);
    } else {
        add(out, b((WALL, 71), (WALL, 32), (0, FLOOR)), grating);
    }
    if info.family == Family::SpiralTop {
        // Arrival only: the rest is open down the well.
        add(out, b((0, WALL), (0, SLOT), (0, FLOOR)), concrete);
        return;
    }
    // Landings c0..c3 at 4, 20, 36, 52 and the lane flats; c0 and the lane-0 flat were added.
    if info.family == Family::SpiralBase {
        add(out, b((WALL, 71), (WALL, 32), (0, FLOOR)), grating);
    }
    add(out, b((96, 124), (WALL, 32), (16, 20)), grating);
    add(out, b((96, 124), (96, 124), (32, 36)), grating);
    add(out, b((WALL, 32), (96, 124), (48, 52)), grating);
    // Flats.
    add(out, b((96, 124), (32, 71), (16, 20)), grating);
    add(out, b((57, 96), (96, 124), (32, 36)), grating);
    add(out, b((WALL, 32), (52, 96), (48, 52)), grating);
    // Flights: 3 Q rises and 5 Q treads, the last one ending at the slot top.
    let flight = |out: &mut Out, bx: B, rise: Dir4, steps: u16| {
        let bx = rotate_box(bx, r);
        out.pieces.push(Piece {
            shape: Shape::stair_run(aabb(bx), rotate_dir(rise, r), steps),
            material: MaterialId::METAL,
            kind: out.kind,
        });
    };
    flight(out, b((71, 96), (WALL, 32), (FLOOR, 19)), Dir4::PosX, 5);
    flight(out, b((96, 124), (71, 96), (20, 35)), Dir4::PosZ, 5);
    flight(out, b((32, 57), (96, 124), (36, 51)), Dir4::NegX, 5);
    flight(out, b((WALL, 32), (32, 52), (52, 64)), Dir4::NegZ, 4);
}

/// A wall on `side` with an opening of the given `u` range from the floor to the door height.
fn wall_open(out: &mut Out, side: usize, opening: (Q, Q), y: (Q, Q), material: MaterialId) {
    let (along, band) = wall_band(side);
    for (a, y) in [
        ((along.0, opening.0), y),
        ((opening.1, along.1), y),
        (opening, (DOOR_TOP.max(y.0), y.1)),
    ] {
        let bx = if side.is_multiple_of(2) {
            b(a, band, y)
        } else {
            b(band, a, y)
        };
        out.add(bx, material);
    }
    threshold(out, side, opening);
}

/// The entrance hall: a room with an opening 4 m wide on `door_side` (towards the doorway in the
/// envelope), a door on every side in `inner_doors`, and walls elsewhere.
pub fn vestibule_pieces(door_side: usize, inner_doors: [bool; 4], style: StyleId) -> Vec<Piece> {
    let mut out = Out {
        pieces: Vec::new(),
        kind: FeatureKind::MASS,
    };
    let material = if style == StyleId::PIPEWORKS {
        MaterialId::METAL
    } else {
        MaterialId::CONCRETE
    };
    out.add(b((0, SLOT), (0, SLOT), (0, FLOOR)), MaterialId::CONCRETE);
    for (side, &inner) in inner_doors.iter().enumerate() {
        if side == door_side {
            wall_open(&mut out, side, (32, 96), (0, STOREY), material);
        } else if inner {
            wall(&mut out, side, D, (0, STOREY), material);
        } else {
            wall(&mut out, side, W, (0, STOREY), material);
        }
    }
    out.pieces
}

/// The pieces of tile `id` in slot-local quanta, in a fixed order. Pipeworks draws room
/// half-walls in metal.
pub fn pieces(id: usize, style: StyleId) -> Vec<Piece> {
    let info = table()[id];
    let kind = if info.is_spiral() {
        FeatureKind::STAIR_TOWER
    } else {
        FeatureKind::MASS
    };
    let mut out = Out {
        pieces: Vec::new(),
        kind,
    };
    let wall_material = if style == StyleId::PIPEWORKS {
        MaterialId::METAL
    } else {
        MaterialId::CONCRETE
    };
    match info.family {
        Family::Solid => out.add(b((0, SLOT), (0, SLOT), (0, STOREY)), MaterialId::CONCRETE),
        Family::Room | Family::Atrium => room_pieces(&info, &mut out, wall_material),
        Family::Void(_) => {
            for (side, &s) in info.sides.iter().enumerate() {
                wall(&mut out, side, s, (0, STOREY), wall_material);
            }
        }
        Family::Corridor => corridor_pieces(&info, &mut out),
        _ => spiral_pieces(&info, &mut out),
    }
    out.pieces
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::l3_solver::{Dims, Scratch, domain_of, opposite, solve, tile_of};
    use crate::rng::Rng;

    #[test]
    fn there_are_205_tiles_in_the_documented_order() {
        let t = table();
        assert_eq!(t.len(), N_TILES);
        assert_eq!(t[SOLID].family, Family::Solid);
        assert_eq!(t[ROOM0].family, Family::Room);
        assert_eq!(t[ATRIUM0].family, Family::Atrium);
        assert_eq!(t[VOID1_0].family, Family::Void(1));
        assert_eq!(t[VOID2_0].family, Family::Void(2));
        assert_eq!(t[VOID3_0].family, Family::Void(3));
        assert_eq!(t[CORRIDOR0].family, Family::Corridor);
        assert_eq!(t[SPIRAL_BASE0].family, Family::SpiralBase);
        assert_eq!(t[SPIRAL0].family, Family::Spiral);
        assert_eq!(t[SPIRAL_TOP0 + 3].family, Family::SpiralTop);
        assert_eq!(SPIRAL_TOP0 + 4, N_TILES);
    }

    #[test]
    fn weights_are_positive_integers_and_follow_the_regime() {
        for style in [
            StyleId::CONCRETE_STRATA,
            StyleId::PIPEWORKS,
            StyleId::HOLLOW_EXPANSE,
        ] {
            let w = weights(128, 128, style);
            assert!(w.iter().all(|&x| x >= 1));
        }
        let packed = weights(255, 0, StyleId::CONCRETE_STRATA);
        let hollow = weights(0, 255, StyleId::CONCRETE_STRATA);
        assert!(packed[SOLID] > hollow[SOLID]);
        assert!(hollow[ATRIUM0] > packed[ATRIUM0]);
        let pipes = weights(128, 128, StyleId::PIPEWORKS);
        let plain = weights(128, 128, StyleId::CONCRETE_STRATA);
        assert_eq!(pipes[CORRIDOR0], 2 * plain[CORRIDOR0]);
    }

    #[test]
    fn no_piece_leaves_its_slot_and_every_shape_is_valid() {
        for style in [StyleId::CONCRETE_STRATA, StyleId::PIPEWORKS] {
            for id in 0..N_TILES {
                for p in pieces(id, style) {
                    let bb = p.shape.aabb();
                    assert!(
                        bb.min.x >= 0
                            && bb.max.x <= SLOT
                            && bb.min.z >= 0
                            && bb.max.z <= SLOT
                            && bb.min.y >= 0
                            && bb.max.y <= STOREY,
                        "tile {id} {:?} leaves its slot: {bb:?}",
                        table()[id]
                    );
                }
            }
        }
    }

    #[test]
    fn solid_is_one_box_and_corridors_keep_a_passage() {
        let solid = pieces(SOLID, StyleId::CONCRETE_STRATA);
        assert_eq!(solid.len(), 1);
        // A straight corridor along z (mouths -z and +z): the arms are open, the sides solid.
        let straight = (CORRIDOR0..CORRIDOR0 + 15)
            .find(|&i| table()[i].sides == [D, W, D, W])
            .unwrap();
        let ps = pieces(straight, StyleId::CONCRETE_STRATA);
        let blocked = |x: Q, z: Q, y: Q| {
            ps.iter().any(|p| {
                let b = p.shape.aabb();
                b.min.x <= x
                    && x < b.max.x
                    && b.min.z <= z
                    && z < b.max.z
                    && b.min.y <= y
                    && y < b.max.y
            })
        };
        assert!(!blocked(64, 10, 20) && !blocked(64, 64, 20) && !blocked(64, 120, 20));
        assert!(blocked(10, 64, 20) && blocked(120, 64, 20) && blocked(64, 64, 50));
    }

    #[test]
    fn a_stack_of_spirals_has_matching_flights_and_steps_within_the_limit() {
        // Rise and tread of every flight (traversal.md §6: step at most 7 Q, tread at least 4 Q).
        for id in SPIRAL_BASE0..N_TILES {
            for p in pieces(id, StyleId::CONCRETE_STRATA) {
                if let Shape::StairRun { aabb, rise, steps } = p.shape {
                    let rise_total = aabb.max.y - aabb.min.y;
                    let run = aabb.size(rise.axis());
                    assert_eq!(rise_total % Q::from(steps), 0);
                    assert_eq!(run % Q::from(steps), 0);
                    assert!(rise_total / Q::from(steps) <= 7);
                    assert!(run / Q::from(steps) >= 4);
                }
            }
        }
    }

    #[test]
    fn rotation_carries_the_door_with_the_rotation() {
        for r in 0..4u8 {
            let id = SPIRAL_BASE0 + usize::from(r);
            assert_eq!(table()[id].sides[usize::from(r)], D);
            // The doorway is open at the middle of side r at y = 8.
            let ps = pieces(id, StyleId::CONCRETE_STRATA);
            let (x, z) = match r {
                0 => (64, 1),
                1 => (126, 64),
                2 => (64, 126),
                _ => (1, 64),
            };
            let blocked = ps.iter().any(|p| {
                let b = p.shape.aabb();
                b.min.x <= x
                    && x < b.max.x
                    && b.min.z <= z
                    && z < b.max.z
                    && b.min.y <= 8
                    && 8 < b.max.y
            });
            assert!(!blocked, "rotation {r}");
        }
    }

    #[test]
    fn the_free_tile_set_solves_a_block_sized_lattice() {
        let ts = tileset(128, 128, StyleId::CONCRETE_STRATA);
        let dims = Dims {
            nx: 8,
            nt: 16,
            nz: 8,
        };
        // Floors at the bottom and top, walls on the outside: the usual boundary conditions.
        let mut d = vec![domain_of(0..N_TILES); dims.len()];
        for i in 0..dims.len() {
            for side in 0..4 {
                if dims.neighbour(i, side).is_none() {
                    d[i][0] &= ts.with_socket(side, W)[0];
                    for w in 1..4 {
                        d[i][w] &= ts.with_socket(side, W)[w];
                    }
                }
            }
            if dims.neighbour(i, 4).is_none() {
                let m = ts.with_socket(4, F);
                for w in 0..4 {
                    d[i][w] &= m[w];
                }
            }
            if dims.neighbour(i, 5).is_none() {
                let m = ts.with_socket(5, F);
                for w in 0..4 {
                    d[i][w] &= m[w];
                }
            }
        }
        let mut ok = 0;
        for seed in 0..20 {
            let mut dd = d.clone();
            if solve(
                &ts,
                dims,
                &mut dd,
                &mut Rng::new(seed),
                &mut Scratch::default(),
            )
            .is_ok()
            {
                ok += 1;
                for i in 0..dims.len() {
                    let a = tile_of(&dd[i]).unwrap();
                    for side in 0..6 {
                        if let Some(j) = dims.neighbour(i, side) {
                            let b = tile_of(&dd[j]).unwrap();
                            assert_eq!(
                                table()[a].sockets()[side],
                                table()[b].sockets()[opposite(side)]
                            );
                        }
                    }
                }
            }
        }
        println!("{ok}/20 free blocks solved");
        assert!(ok >= 10, "only {ok}/20 solved");
    }
}
