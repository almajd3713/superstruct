//! Archetypes of L3 masses (`docs/spec/levels.md` §6W.4, L3-04..13): how an interior suits the
//! form it is in. An archetype never changes the skeleton's connectedness or the boundary
//! conditions; it changes the tile weights of the solver slot by slot, and for three of them
//! the shape of the skeleton's tree and the number of spines.

#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::enum_glob_use,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::naive_bytecount,
    clippy::match_same_arms
)]

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l3_tiles::{D, Family, O, TileInfo};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use glam::I64Vec3;

/// The archetypes, numbered as the registry (`Plain` is the solver without one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Archetype {
    Plain = 0,
    /// L3-04: closed offices, one door each, off corridors.
    Office = 4,
    /// L3-05: rows of one-slot cells off a corridor.
    CellBlock = 5,
    /// L3-06: a double-height hall with galleries.
    Mezzanine = 6,
    /// L3-07: a core of more stairs (a helical ramp is the same climb).
    RampCore = 7,
    /// L3-08: wide open storeys alternating with partitioned ones.
    Lofts = 8,
    /// L3-09: mostly voids, floors only on the skeleton.
    OpenCage = 9,
    /// L3-10: big open volumes with catwalks on the skeleton.
    MachineFloors = 10,
    /// L3-11: long aisles between solid racks.
    RackAisles = 11,
    /// L3-12: spokes from the spine.
    Radial = 12,
    /// L3-13: corridors that turn a quarter each storey about the spine.
    Twisted = 13,
}

impl Archetype {
    pub const ALL: [Archetype; 11] = [
        Self::Plain,
        Self::Office,
        Self::CellBlock,
        Self::Mezzanine,
        Self::RampCore,
        Self::Lofts,
        Self::OpenCage,
        Self::MachineFloors,
        Self::RackAisles,
        Self::Radial,
        Self::Twisted,
    ];

    pub fn id(self) -> u8 {
        self as u8
    }

    /// Spines more than the plain count.
    pub fn extra_spines(self) -> usize {
        usize::from(self == Self::RampCore)
    }
}

/// The archetypes a structure form allows, with weights. `form` is `Structure::form`: a plain
/// form 0 to 22 (`forms.rs`, FORM-01 to FORM-23), a hub from 100, a set piece from 200 and a
/// local place from 300.
pub fn applicable(form: u16) -> Vec<(Archetype, u32)> {
    use Archetype::*;
    let mut v = vec![(Plain, 3)];
    match form {
        // Slab and tower forms: FORM-01 slab, 03 needle, 04 tube, 05 pair, 06 comb, 08 cluster.
        0 | 2 | 3 | 4 | 5 | 7 => v.extend([
            (Office, 4),
            (CellBlock, 3),
            (Mezzanine, 2),
            (RampCore, 2),
            (Lofts, 2),
        ]),
        // FORM-02 ziggurat.
        1 => v.extend([(Office, 3), (Lofts, 3), (Mezzanine, 2)]),
        // FORM-07 cage, FORM-11 gantry.
        6 | 10 => v.extend([(OpenCage, 6), (Mezzanine, 2), (MachineFloors, 1)]),
        // FORM-12 machine, FORM-14 tank farm.
        11 | 13 => v.extend([(MachineFloors, 6), (OpenCage, 1)]),
        // FORM-13 rack canyon.
        12 => v.extend([(RackAisles, 8), (MachineFloors, 2)]),
        // FORM-09 drum, FORM-10, rotunda 16, dome 19.
        8 | 9 | 15 | 18 => v.extend([(Radial, 5), (RampCore, 2), (CellBlock, 2)]),
        // FORM-17 twisted tower.
        16 => v.extend([(Twisted, 8), (Radial, 1)]),
        // Leaning, angled forms: lofts on their side.
        17 | 20 | 21 | 22 => v.extend([(Lofts, 4), (Office, 2)]),
        // Ruins and the rest: a plain interior, sometimes a hall.
        14 => v.extend([(Mezzanine, 2), (OpenCage, 2)]),
        // Hubs: halls, canyons, cathedrals.
        100..=199 => v.extend([
            (Mezzanine, 3),
            (MachineFloors, 2),
            (RackAisles, 1),
            (Radial, 1),
        ]),
        _ => v.extend([(Office, 1), (Mezzanine, 1)]),
    }
    v
}

/// The archetype of mass `index` of L2 cell `cell`.
pub fn pick(seed: WorldSeed, cell: CellKey, index: usize, form: u16) -> Archetype {
    debug_assert_eq!(cell.level, Level::L2);
    let key = cell.coords + I64Vec3::new(index as i64, 0, 0);
    let mut rng = Rng::new(hash(seed.0, Level::L2, key, salt::L3_ARCH));
    let options = applicable(form);
    let ws: Vec<u32> = options.iter().map(|o| o.1).collect();
    rng.pick_weighted(&ws)
        .map_or(Archetype::Plain, |i| options[i].0)
}

fn count(info: &TileInfo, socket: u8) -> usize {
    info.sides.iter().filter(|&&s| s == socket).count()
}

/// The multiplier, in eighths, an archetype puts on a tile's weight at storey `rel` (counted
/// from the bottom of the mass).
pub fn weight_mult(a: Archetype, info: &TileInfo, rel: i32) -> u32 {
    let (o, d) = (count(info, O), count(info, D));
    let closed = o == 0;
    match (a, info.family) {
        (Archetype::Plain | Archetype::Radial | Archetype::Twisted, _) => 8,
        (_, Family::SpiralBase | Family::Spiral | Family::SpiralTop) => 8,
        (Archetype::Office, Family::Room) => match (closed, d) {
            (true, 1) => 64,
            (true, _) => 24,
            _ => 3,
        },
        (Archetype::Office, Family::Corridor) => 24,
        (Archetype::Office, Family::Atrium) => 2,
        (Archetype::Office, Family::Void(_)) => 3,
        (Archetype::CellBlock, Family::Room) => {
            if closed && d == 1 {
                120
            } else {
                2
            }
        }
        (Archetype::CellBlock, Family::Corridor) => 40,
        (Archetype::CellBlock, Family::Solid) => 16,
        (Archetype::CellBlock, Family::Atrium | Family::Void(_)) => 1,
        (Archetype::Mezzanine, Family::Atrium) => 100,
        (Archetype::Mezzanine, Family::Void(_)) => 20,
        (Archetype::Mezzanine, Family::Room) => {
            if o >= 2 {
                24
            } else if closed {
                3
            } else {
                8
            }
        }
        (Archetype::Mezzanine, Family::Solid) => 3,
        (Archetype::RampCore, Family::Atrium) => 16,
        (Archetype::RampCore, Family::Solid) => 6,
        (Archetype::Lofts, _) if rel % 2 == 0 => match info.family {
            Family::Room if o >= 3 => 40,
            Family::Atrium => 24,
            Family::Solid => 2,
            _ => 8,
        },
        (Archetype::Lofts, _) => match info.family {
            Family::Solid => 24,
            Family::Room if closed => 24,
            Family::Corridor => 24,
            Family::Room => 3,
            _ => 4,
        },
        (Archetype::OpenCage, Family::Void(_)) => 160,
        (Archetype::OpenCage, Family::Atrium) => 20,
        (Archetype::OpenCage, Family::Solid) => 1,
        (Archetype::OpenCage, Family::Room) => 2,
        (Archetype::OpenCage, Family::Corridor) => 4,
        (Archetype::MachineFloors, Family::Atrium) => 120,
        (Archetype::MachineFloors, Family::Void(_)) => 40,
        (Archetype::MachineFloors, Family::Corridor) => 16,
        (Archetype::MachineFloors, Family::Room) => {
            if o >= 2 {
                16
            } else {
                2
            }
        }
        (Archetype::MachineFloors, Family::Solid) => 2,
        (Archetype::RackAisles, Family::Solid) => 40,
        (Archetype::RackAisles, Family::Corridor) => {
            // A straight aisle: mouths on two opposite sides.
            let straight = d == 2
                && ((info.sides[0] == D && info.sides[2] == D)
                    || (info.sides[1] == D && info.sides[3] == D));
            if straight { 120 } else { 4 }
        }
        (Archetype::RackAisles, Family::Room) => 1,
        (Archetype::RackAisles, Family::Atrium) => 4,
        (Archetype::RackAisles, Family::Void(_)) => 1,
        _ => 8,
    }
}
