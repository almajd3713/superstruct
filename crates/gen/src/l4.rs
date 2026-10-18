//! L4 module placement (`docs/spec/levels.md` §7): where the code-generated kit pieces go on L3
//! surfaces and edges. This module only decides placements, as plain data; the engine turns each
//! [`Module`] into a mesh instance.
//!
//! Placement is a pure function of the block's layout and primitives (and, for railings, of the
//! L2 edge runs). Every module is `NO_COLLIDE` in this version: modules that replace the look of
//! existing collidable boxes (railings, pipes) leave the boxes to carry collision, and the rest
//! stay well inside the 0.25 m wall allowance, so no walk link of `levels.md` Lemma L3-4 changes.

#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // slot indices are below 16

use crate::cell::{Axis, CellKey, Level, floor_div_pow2};
use crate::hash::hash;
use crate::l3::{L3Block, SlotClass, VESTIBULE};
use crate::l3_tiles::{self as tiles, D, FLOOR, Family, SLOT, STOREY, W, WALL};
use crate::prim::{FeatureKind, Primitive, Q, QVec3, Shape};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Full brightness.
pub const GLOW_MAX: u8 = 15;

/// Wall panel variants; the draw maps to them with plain walls the most common.
pub const PANEL_VARIANTS: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    /// 8 m × 4 m panel on the inner face of a wall. `variant` is 0 plain, 1 ribbed, 2 vent,
    /// 3 window.
    WallPanel,
    /// `len` along the piece's x, standing on `pos`.
    Railing,
    /// `len` by `width` grating overlay centred on `pos`, on the floor top.
    Grating,
    /// Frame of a door opening, on the inner face of a door side.
    DoorFrame,
    /// Hangs from `pos` on the ceiling.
    Lamp,
    /// A straight pipe of `len` along the piece's x, centred on `pos`.
    Pipe,
    /// A cable bundle of `len` along the piece's x, centred on `pos`.
    Cable,
}

/// A placed kit piece. Positions are in quanta in the frame of the cell the placement came from
/// (the L3 block, or the L2 cell for railings), at the middle of the piece's base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Module {
    pub kind: ModuleKind,
    pub pos: QVec3,
    /// Quarter turns about +Y taking the piece's front (+Z) to +z (0), +x (1), -z (2), -x (3);
    /// its local x axis goes to +x, -z, -x, +z.
    pub facing: u8,
    pub len: Q,
    pub width: Q,
    pub variant: u8,
    /// Brightness of the piece's emissive parts, 0 (dead) to 15, already scaled by the block's
    /// regime: a Blackout block has every module dark. Pieces without emissive parts carry 15.
    pub glow: u8,
}

fn module(kind: ModuleKind, pos: [Q; 3], facing: u8) -> Module {
    Module {
        kind,
        pos: QVec3::new(pos[0], pos[1], pos[2]),
        facing,
        len: 0,
        width: 0,
        variant: 0,
        glow: GLOW_MAX,
    }
}

/// The side numbers of a slot are -z, +x, +z, -x. A wall on side `s` faces into the slot along
/// the facing returned here, and its inner face lies this far from the slot's centre line.
const SIDE_FACING: [u8; 4] = [0, 3, 2, 1];

/// Centre of the inner face of side `side` of the slot at `(x, t, z)`, at the storey line.
fn side_point(x: Q, t: Q, z: Q, side: usize) -> [Q; 3] {
    let (x0, z0) = (x * SLOT, z * SLOT);
    let y = t * STOREY;
    match side {
        0 => [x0 + SLOT / 2, y, z0 + WALL],
        1 => [x0 + SLOT - WALL, y, z0 + SLOT / 2],
        2 => [x0 + SLOT / 2, y, z0 + SLOT - WALL],
        _ => [x0 + WALL, y, z0 + SLOT / 2],
    }
}

/// The modules of an L3 block, in slot order and, inside a slot, panels, doors, grating, lamp;
/// then pipes and cables from the block's primitives. Positions are block-local.
pub fn block_modules(seed: WorldSeed, block: CellKey, l3: &L3Block) -> Vec<Module> {
    debug_assert_eq!(block.level, Level::L3);
    let mut out = Vec::new();
    let b = block.coords;
    // A Blackout block suppresses every emissive part (`regimes.md` §5).
    let lit_regime = regime(seed, b.map(|v| floor_div_pow2(v, 9))).light > 0;
    for (i, slot) in l3.layout.slots.iter().enumerate() {
        if slot.class != SlotClass::Core || slot.sealed || slot.tile == u16::MAX {
            continue;
        }
        let (x, t, z) = (i % 8, i / 64, i / 8 % 8);
        let (xq, tq, zq) = (x as Q, t as Q, z as Q);
        // Variants come from fixed draws per L4 cell, whatever the slots hold.
        let l4 = glam::I64Vec3::new(
            b.x * 8 + x as i64,
            b.y * 8 + (t / 2) as i64,
            b.z * 8 + z as i64,
        );
        let mut rng = Rng::new(hash(seed.0, Level::L4, l4, salt::MODULE));
        let draws: [u64; 18] = std::array::from_fn(|_| rng.below(8));
        let draw = |side: usize| draws[(t % 2) * 4 + side];
        // Window brightness per side, then the lamp of each slot of the cell.
        let window_glow = |side: usize| 5 + (draws[10 + (t % 2) * 4 + side] * 2).min(10) as u8;
        let lamp_glow = match draws[8 + t % 2] {
            0..=4 => GLOW_MAX,
            5 | 6 => 7,
            _ => 0,
        };

        let vestibule = slot.tile == VESTIBULE;
        let info = (!vestibule).then(|| tiles::table()[usize::from(slot.tile)]);
        if let Some(info) = info.filter(tiles::TileInfo::walkable) {
            for (side, &socket) in info.sides.iter().enumerate() {
                let p = side_point(xq, tq, zq, side);
                match socket {
                    W => {
                        let mut m = module(ModuleKind::WallPanel, p, SIDE_FACING[side]);
                        m.variant = match draw(side) {
                            0..=3 => 0,
                            4 | 5 => 1,
                            6 => 2,
                            _ => 3,
                        };
                        if m.variant == 3 {
                            m.glow = if lit_regime { window_glow(side) } else { 0 };
                        }
                        out.push(m);
                    }
                    D => {
                        let p = [p[0], p[1] + FLOOR, p[2]];
                        let mut m = module(ModuleKind::DoorFrame, p, SIDE_FACING[side]);
                        m.glow = if lit_regime { GLOW_MAX } else { 0 };
                        out.push(m);
                    }
                    _ => {}
                }
            }
            let flat = matches!(
                info.family,
                Family::Corridor | Family::SpiralBase | Family::SpiralTop
            );
            if flat {
                let mut m = module(
                    ModuleKind::Grating,
                    [
                        xq * SLOT + SLOT / 2,
                        tq * STOREY + FLOOR + 1,
                        zq * SLOT + SLOT / 2,
                    ],
                    0,
                );
                m.len = SLOT - 2 * WALL;
                m.width = SLOT - 2 * WALL;
                out.push(m);
            }
        }
        let lit =
            vestibule || info.is_some_and(|i| matches!(i.family, Family::Room | Family::Corridor));
        if lit {
            let mut m = module(
                ModuleKind::Lamp,
                [
                    xq * SLOT + SLOT / 2,
                    (tq + 1) * STOREY,
                    zq * SLOT + SLOT / 2,
                ],
                0,
            );
            m.glow = if lit_regime { lamp_glow } else { 0 };
            out.push(m);
        }
    }
    pipes_and_cables(&l3.out.primitives, &mut out);
    out
}

/// Pipes come as one module per 8 m of each L3 pipe cylinder, with a cable bundle beside the
/// first pipe of each run (cylinders that start at the same place along the same axis).
fn pipes_and_cables(prims: &[Primitive], out: &mut Vec<Module>) {
    let mut last_run: Option<(Axis, Q, Q)> = None;
    for p in prims.iter().filter(|p| p.kind == FeatureKind::PIPE) {
        let Shape::Cylinder {
            axis,
            base,
            length,
            radius,
        } = p.shape
        else {
            continue;
        };
        let facing = u8::from(axis == Axis::Z);
        let mut at = 0;
        while at < length {
            let seg = SLOT.min(length - at);
            let centre = match axis {
                Axis::X => [base.x + at + seg / 2, base.y, base.z],
                _ => [base.x, base.y, base.z + at + seg / 2],
            };
            let mut m = module(ModuleKind::Pipe, centre, facing);
            m.len = seg;
            m.width = radius;
            out.push(m);
            at += seg;
        }
        let run = (axis, base.x.min(base.z), length);
        if last_run != Some(run) {
            last_run = Some(run);
            let centre = match axis {
                Axis::X => [base.x + length / 2, base.y + 8, base.z],
                _ => [base.x, base.y + 8, base.z + length / 2],
            };
            let mut m = module(ModuleKind::Cable, centre, facing);
            m.len = length;
            out.push(m);
        }
    }
}

/// Railing modules for the railing boxes of an L2 cell, in primitive order. The boxes keep
/// their collision; the module is the look.
pub fn railing_modules(prims: &[Primitive]) -> Vec<Module> {
    prims
        .iter()
        .filter(|p| p.kind == FeatureKind::RAILING)
        .map(|p| {
            let bb = p.aabb();
            let along_x = bb.max.x - bb.min.x >= bb.max.z - bb.min.z;
            let mid = |a: Q, b: Q| i32::midpoint(a, b);
            let mut m = module(
                ModuleKind::Railing,
                [mid(bb.min.x, bb.max.x), bb.min.y, mid(bb.min.z, bb.max.z)],
                u8::from(!along_x),
            );
            m.len = if along_x {
                bb.max.x - bb.min.x
            } else {
                bb.max.z - bb.min.z
            };
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::l2::l2_cell_in;
    use crate::l2_port::L1Cache;
    use crate::l2_rail::railings_with_runs;
    use crate::l3::{l3_block_with, l3_blocks_of, l3_parent};
    use crate::prim::Q;
    use crate::walk::cell_window;

    const SEED: WorldSeed = WorldSeed(0x0422);

    /// P4.2.2: guarded edge runs have a railing module and unguarded ones have none.
    #[test]
    fn guarded_edges_have_railings_and_unguarded_ones_do_not() {
        let cache = L1Cache::default();
        let (mut guarded, mut unguarded) = (0, 0);
        for x in -8..8 {
            let parent_key = CellKey::new(Level::L1, I64Vec3::new(x, x % 3, 2 * x % 5));
            let parent = cache.get(SEED, parent_key);
            for k in (0..512i64).step_by(3) {
                let idx = [k % 8, k / 64, k / 8 % 8];
                let cell = CellKey::new(
                    Level::L2,
                    parent_key.coords * 8 + I64Vec3::new(idx[0], idx[1], idx[2]),
                );
                let out = l2_cell_in(SEED, &cache, cell);
                let origin = idx.map(|i| Q::try_from(i).unwrap() * crate::l2::CELL);
                let mut seen = cell_window(&parent.primitives, origin, crate::l2::CELL);
                seen.extend(
                    out.primitives
                        .iter()
                        .filter(|p| p.kind != FeatureKind::RAILING)
                        .copied(),
                );
                let (_, _, runs) = railings_with_runs(SEED, cell, &seen);
                let mods = railing_modules(&out.primitives);
                let on_run = |m: &Module, r: &crate::l2_rail::EdgeRun| {
                    let (across, along) = if r.along_x {
                        (m.pos.z, m.pos.x)
                    } else {
                        (m.pos.x, m.pos.z)
                    };
                    // Railings stand just inside the edge, within their thickness.
                    m.pos.y == r.y
                        && (across - r.fixed).abs() <= 2
                        && (m.facing == 0) == r.along_x
                        && (along - i32::midpoint(r.a, r.b)).abs() <= 1
                        && m.len == r.b - r.a
                };
                assert_eq!(
                    mods.len(),
                    runs.iter().filter(|r| r.guarded).count(),
                    "{cell:?}"
                );
                for r in &runs {
                    let n = mods.iter().filter(|m| on_run(m, r)).count();
                    if r.guarded {
                        assert_eq!(n, 1, "{cell:?}: guarded run {r:?}");
                        guarded += 1;
                    } else {
                        assert_eq!(n, 0, "{cell:?}: unguarded run {r:?}");
                        unguarded += 1;
                    }
                }
            }
        }
        assert!(
            guarded > 50 && unguarded > 5,
            "{guarded} guarded, {unguarded} unguarded"
        );
    }

    fn sample_blocks(count: usize) -> Vec<(CellKey, L3Block)> {
        let cache = L1Cache::default();
        let mut out = Vec::new();
        'scan: for x in -40..40i64 {
            let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 4, 2 * x % 7));
            for k in (0..512i64).step_by(7) {
                let cell = CellKey::new(
                    Level::L2,
                    l1.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                );
                let blocks = l3_blocks_of(SEED, &cache, cell);
                if blocks.is_empty() {
                    continue;
                }
                let parent = l3_parent(SEED, &cache, cell);
                let key = blocks[blocks.len() / 2].0;
                out.push((key, l3_block_with(SEED, &parent, key)));
                if out.len() == count {
                    break 'scan;
                }
            }
        }
        out
    }

    /// P4.2.2: panels on W sides, frames on D sides, gratings on corridors, lamps on lit slots,
    /// nothing in solid slots; placement is pure and stays inside the block.
    #[test]
    fn modules_follow_the_slot_rules() {
        let blocks = sample_blocks(60);
        assert!(blocks.len() >= 30, "{} blocks", blocks.len());
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for (key, b) in &blocks {
            let mods = block_modules(SEED, *key, b);
            assert_eq!(mods, block_modules(SEED, *key, b));
            for m in &mods {
                *counts.entry(format!("{:?}", m.kind)).or_default() += 1;
                assert!(
                    (-1..=1025).contains(&m.pos.x)
                        && (0..=1024).contains(&m.pos.y)
                        && (-1..=1025).contains(&m.pos.z),
                    "{m:?} outside its block"
                );
            }
            for (i, slot) in b.layout.slots.iter().enumerate() {
                let (x, t, z) = (i % 8, i / 64, i / 8 % 8);
                let origin = [x as Q * SLOT, t as Q * STOREY, z as Q * SLOT];
                let inside = |m: &&Module| {
                    m.kind != ModuleKind::Pipe
                        && m.kind != ModuleKind::Cable
                        && (origin[0]..=origin[0] + SLOT).contains(&m.pos.x)
                        && (origin[1]..origin[1] + STOREY).contains(&m.pos.y)
                        && (origin[2]..=origin[2] + SLOT).contains(&m.pos.z)
                };
                let mine: Vec<&Module> = mods.iter().filter(inside).collect();
                let walkable = slot.class == SlotClass::Core
                    && !slot.sealed
                    && slot.tile != u16::MAX
                    && slot.tile != VESTIBULE
                    && tiles::table()[usize::from(slot.tile)].walkable();
                if !walkable {
                    continue;
                }
                let info = tiles::table()[usize::from(slot.tile)];
                #[allow(clippy::needless_range_loop)] // the side indexes three tables
                for side in 0..4 {
                    let p = side_point(x as Q, t as Q, z as Q, side);
                    let at_side = |kind: ModuleKind, y: Q| {
                        mods.iter()
                            .filter(|m| {
                                m.kind == kind
                                    && m.pos == QVec3::new(p[0], y, p[2])
                                    && m.facing == SIDE_FACING[side]
                            })
                            .count()
                    };
                    assert_eq!(
                        at_side(ModuleKind::WallPanel, p[1]),
                        usize::from(info.sides[side] == W),
                        "{key:?} slot {i} side {side}"
                    );
                    assert_eq!(
                        at_side(ModuleKind::DoorFrame, p[1] + FLOOR),
                        usize::from(info.sides[side] == D),
                        "{key:?} slot {i} side {side}"
                    );
                }
                let grates = mine
                    .iter()
                    .filter(|m| m.kind == ModuleKind::Grating)
                    .count();
                let flat = matches!(
                    info.family,
                    Family::Corridor | Family::SpiralBase | Family::SpiralTop
                );
                assert_eq!(grates, usize::from(flat), "{key:?} slot {i}");
            }
        }
        // Emissive parts vary in brightness: some lamps are dim or dead, windows are never dark
        // in a lit regime, and nothing exceeds the scale.
        let all: Vec<Module> = blocks
            .iter()
            .flat_map(|(k, b)| block_modules(SEED, *k, b))
            .collect();
        assert!(all.iter().all(|m| m.glow <= GLOW_MAX));
        let lamps: Vec<u8> = all
            .iter()
            .filter(|m| m.kind == ModuleKind::Lamp)
            .map(|m| m.glow)
            .collect();
        assert!(lamps.contains(&GLOW_MAX) && lamps.contains(&0) && lamps.contains(&7));
        assert!(
            all.iter()
                .filter(|m| m.kind == ModuleKind::WallPanel && m.variant == 3)
                .all(|m| (5..=15).contains(&m.glow))
        );
        for kind in ["WallPanel", "DoorFrame", "Grating", "Lamp"] {
            assert!(counts.get(kind).copied().unwrap_or(0) > 20, "{counts:?}");
        }
        println!("{counts:?}");
    }
}
