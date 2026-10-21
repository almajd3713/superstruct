//! L3 in the Web (`docs/spec/levels.md` §6W): the interiors of the box parts of the tissue's
//! structures, solved on the L2 cell's lattice frame.
//!
//! A mass is one box part of a structure. Its blocks are the 64 m cubes of the lattice (§6W.1:
//! the mass lattice is the lattice of the cell, in its frame), so the solve of `l3.rs` runs
//! unchanged in lattice coordinates and the result is turned into place by `frame_block`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;

use glam::DVec3;

use crate::cell::{CellKey, Dir6, Level, floor_mod_pow2};
use crate::l2_port::L1Cache;
use crate::l3::{L3Block, Parent, block_is_active, l3_block_with};
use crate::l3_plan::{COL, Entrance, EntranceKind, MassPlan};
use crate::orient::{Frame, LocalShape};
use crate::prim::{Primitive, Shape};
use crate::prim::{Q, QAabb, QVec3};
use crate::seed::WorldSeed;
use crate::tissue::tissue_cell;
use crate::tissue_grow::quanta;
use crate::tissue_grow::{SlotBox, Structure};

/// Quanta in one lattice slot (32 m).
const SLOT_Q: Q = 32 * 16;

/// A mass: a box part of a structure and the ways into it.
#[derive(Debug, Clone)]
pub struct MassSpec {
    pub structure: usize,
    pub part: usize,
    /// The structure's form, which decides its archetype.
    pub form: u16,
    /// The part's box in the lattice frame, in quanta from the cell's lattice corner.
    pub aabb: QAabb,
    pub entrances: Vec<Entrance>,
}

fn box_aabb(b: &SlotBox) -> QAabb {
    let q = |k: usize| (b.lo[k] * SLOT_Q, b.hi[k] * SLOT_Q);
    QAabb::new(
        QVec3::new(q(0).0, q(1).0, q(2).0),
        QVec3::new(q(0).1, q(1).1, q(2).1),
    )
}

fn contains(b: &SlotBox, c: [i32; 3]) -> bool {
    (0..3).all(|k| b.lo[k] <= c[k] && c[k] < b.hi[k])
}

/// A door on face `side` of `part`, through the slot cell `cell` (the slot outside the face
/// for an entrance, the slot inside for a join), at the storey of the cell's floor.
fn door(part: &SlotBox, side: u8, cell: [i32; 3]) -> Entrance {
    // The doorway is 4 m wide and centred on the slot, so it is half in each of the two middle
    // columns of the slot's four.
    let centre = |k: usize| cell[k] * SLOT_Q + SLOT_Q / 2;
    let (u_axis, col_axis) = if side.is_multiple_of(2) {
        (0, 2)
    } else {
        (2, 0)
    };
    let (u0, u1) = (centre(u_axis) - 32, centre(u_axis) + 32);
    let along = [cell[u_axis] * 4 + 1, cell[u_axis] * 4 + 2];
    let face_col = match side {
        0 => part.lo[2] * 4,
        1 => part.hi[0] * 4 - 1,
        2 => part.hi[2] * 4 - 1,
        _ => part.lo[0] * 4,
    };
    let _ = col_axis;
    let cols = along
        .iter()
        .map(|&a| {
            if side.is_multiple_of(2) {
                (a, face_col)
            } else {
                (face_col, a)
            }
        })
        .collect();
    Entrance {
        kind: EntranceKind::Door,
        side,
        u0,
        u1,
        cols,
        t: cell[1] * 8,
    }
}

/// The hatch through the slot column of `cell`, in the floor of `part` (`floor`) or its roof.
fn hatch(part: &SlotBox, cell: [i32; 3], floor: bool) -> Entrance {
    let col = (cell[0] * 4 + 2, cell[2] * 4 + 2);
    Entrance {
        kind: if floor {
            EntranceKind::FloorHatch
        } else {
            EntranceKind::RoofHatch
        },
        side: 0,
        u0: col.0 * COL + 32,
        u1: col.0 * COL + 96,
        cols: vec![col],
        t: if floor {
            part.lo[1] * 8
        } else {
            part.hi[1] * 8 - 1
        },
    }
}

/// The masses of a structure, with its entrances placed on the part each one leads into and a
/// door between every two parts that touch.
fn structure_masses(
    si: usize,
    s: &Structure,
    lat_coords: &dyn Fn(u32) -> [i32; 3],
    way_in: &dyn Fn(u32) -> bool,
) -> Vec<MassSpec> {
    let mut specs: Vec<MassSpec> = s
        .parts
        .iter()
        .enumerate()
        .map(|(pi, p)| MassSpec {
            structure: si,
            part: pi,
            form: s.form,
            aabb: box_aabb(p),
            entrances: Vec::new(),
        })
        .collect();
    let push = |spec: &mut MassSpec, e: Entrance| {
        if !spec.entrances.contains(&e) {
            spec.entrances.push(e);
        }
    };
    for e in &s.entrances {
        // A door needs a way to it (PW.4.17): a floor where the route passes outside.
        let horizontal = !matches!(e.into, Dir6::PosY | Dir6::NegY);
        if horizontal && !way_in(e.slot) {
            continue;
        }
        let r = lat_coords(e.slot);
        let d = e.into.offset();
        let inside = [r[0] + d.x as i32, r[1] + d.y as i32, r[2] + d.z as i32];
        let Some(pi) = s.parts.iter().position(|p| contains(p, inside)) else {
            continue;
        };
        let part = &s.parts[pi];
        let ent = match e.into {
            Dir6::PosX => door(part, 3, r),
            Dir6::NegX => door(part, 1, r),
            Dir6::PosZ => door(part, 0, r),
            Dir6::NegZ => door(part, 2, r),
            Dir6::PosY => hatch(part, r, true),
            Dir6::NegY => hatch(part, r, false),
        };
        push(&mut specs[pi], ent);
    }
    // Joins: where two parts share a face, one door through the lowest, first contact cell.
    for a in 0..s.parts.len() {
        for b in a + 1..s.parts.len() {
            let (pa, pb) = (&s.parts[a], &s.parts[b]);
            for (axis, side_a, side_b) in [(0usize, 1u8, 3u8), (2, 2, 0)] {
                let (first, second, sa, sb) = if pa.hi[axis] == pb.lo[axis] {
                    (pa, pb, side_a, side_b)
                } else if pb.hi[axis] == pa.lo[axis] {
                    (pb, pa, side_b, side_a)
                } else {
                    continue;
                };
                let o = 2 - axis;
                let lo = [0, 1, 2].map(|k| first.lo[k].max(second.lo[k]));
                let hi = [0, 1, 2].map(|k| first.hi[k].min(second.hi[k]));
                if lo[1] >= hi[1] || lo[o] >= hi[o] {
                    continue;
                }
                // The contact cell on each side of the shared plane.
                let mut cell_a = lo;
                cell_a[axis] = first.hi[axis] - 1;
                let mut cell_b = lo;
                cell_b[axis] = second.lo[axis];
                let (ia, ib) = if std::ptr::eq(first, pa) {
                    (a, b)
                } else {
                    (b, a)
                };
                push(&mut specs[ia], door(first, sa, shift(cell_a, axis, 1)));
                push(&mut specs[ib], door(second, sb, shift(cell_b, axis, -1)));
            }
        }
    }
    specs
}

/// `c` moved `d` slots along `axis`: the slot outside a face, from the slot inside it.
fn shift(mut c: [i32; 3], axis: usize, d: i32) -> [i32; 3] {
    c[axis] += d;
    c
}

/// The box, in lattice-frame metres, to take out of the shell of `part` for entrance `e`: a
/// doorway 3 m high through the wall, or a hatch through the floor or roof.
#[allow(clippy::single_match_else)] // a door and a hatch read side by side
pub fn entrance_cut(e: &Entrance, part: &SlotBox) -> ([f64; 3], [f64; 3]) {
    let m = |q: Q| f64::from(q) / 16.0;
    let slot = 32.0;
    let floor = f64::from(e.t) * 4.0;
    match e.kind {
        EntranceKind::Door => {
            let (u0, u1) = (m(e.u0), m(e.u1));
            let face =
                |k: usize, hi: bool| f64::from(if hi { part.hi[k] } else { part.lo[k] }) * slot;
            match e.side {
                0 => (
                    [u0, floor, face(2, false) - 1.0],
                    [u1, floor + 3.0, face(2, false) + 1.0],
                ),
                1 => (
                    [face(0, true) - 1.0, floor, u0],
                    [face(0, true) + 1.0, floor + 3.0, u1],
                ),
                2 => (
                    [u0, floor, face(2, true) - 1.0],
                    [u1, floor + 3.0, face(2, true) + 1.0],
                ),
                _ => (
                    [face(0, false) - 1.0, floor, u0],
                    [face(0, false) + 1.0, floor + 3.0, u1],
                ),
            }
        }
        _ => {
            let (c, z) = (e.cols[0], e.cols[0].1);
            let x0 = f64::from(c.0) * 8.0 + 2.0;
            let z0 = f64::from(z) * 8.0 + 2.0;
            let y = if e.kind == EntranceKind::FloorHatch {
                f64::from(part.lo[1]) * slot
            } else {
                f64::from(part.hi[1]) * slot
            };
            ([x0, y - 1.0, z0], [x0 + 4.0, y + 1.0, z0 + 4.0])
        }
    }
}

/// Whether two masses share volume.
fn overlap(a: &QAabb, b: &QAabb) -> bool {
    a.min.x < b.max.x
        && b.min.x < a.max.x
        && a.min.y < b.max.y
        && b.min.y < a.max.y
        && a.min.z < b.max.z
        && b.min.z < a.max.z
}

/// Whether a walker can reach the face of a structure from route slot `slot`: a floor under
/// the route where it passes the slot's centre (`floored_slots`).
pub fn way_in(t: &crate::tissue::Tissue, slot: u32) -> bool {
    t.floored_slots.contains(&slot)
}

/// The entrance slots of a tissue that have a floor under the route where it passes the slot's
/// centre, 2 m round it (a leg, a landing, a payoff): a route that only runs along the
/// structure (the mid-flight levels of a stair that passes it) has none. The thresholds are not
/// read, because they are placed after this is asked. Computed once, after the paths.
pub fn floored_slots(t: &crate::tissue::Tissue) -> Vec<u32> {
    let em = crate::tissue_grow::Emitter::new(&t.lattice);
    let pitch = t.lattice.pitch as f64;
    let pieces: Vec<&Primitive> = t
        .paths
        .prims
        .iter()
        .chain(&t.payoffs.prims)
        .filter(|p| !p.flags.contains(crate::prim::PrimFlags::NO_COLLIDE))
        .collect();
    let mut slots: Vec<u32> = t
        .structures
        .iter()
        .flat_map(|s| s.entrances.iter().map(|e| e.slot))
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots.retain(|&slot| {
        let c = t.lattice.coords(slot as usize);
        let centre = em.at([
            (c[0] as f64 + 0.5) * pitch,
            c[1] as f64 * pitch - 0.1,
            (c[2] as f64 + 0.5) * pitch,
        ]);
        let probes =
            [(0.0, 0.0), (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)].map(|(x, z)| {
                glam::Vec3::new(
                    (centre.x + x) as f32,
                    centre.y as f32,
                    (centre.z + z) as f32,
                )
            });
        let reach = glam::Vec3::splat(2.5);
        pieces.iter().any(|p| {
            let b = p.aabb();
            let (lo, hi) = (b.min_m(), b.max_m());
            probes.iter().any(|&q| {
                q.cmpge(lo - reach).all() && q.cmple(hi + reach).all() && p.shape.contains(q)
            })
        })
    });
    slots
}

/// The doorways of the masses as clear boxes (cell-local primitives): cut through whatever
/// else stands in them, the cladding and skin plates and neighbouring structures (PW.4.17).
pub fn door_cuts(t: &crate::tissue::Tissue) -> Vec<Primitive> {
    let mut em = crate::tissue_grow::Emitter::new(&t.lattice);
    for m in mass_specs(t) {
        let part = &t.structures[m.structure].parts[m.part];
        for e in &m.entrances {
            if e.kind == EntranceKind::Door {
                let (lo, hi) = entrance_cut(e, part);
                em.frame_box(
                    lo,
                    hi,
                    crate::prim::MaterialId::CONCRETE,
                    crate::prim::FeatureKind::MISC,
                    crate::prim::PrimFlags::NO_COLLIDE,
                );
            }
        }
    }
    em.out
}

/// The approach to every door of a route entrance (PW.4.17): a deck at the sill, as wide as the
/// doorway, from the middle of the route slot to the face, in lattice-frame metres `(lo, hi)`.
pub fn approach_decks(t: &crate::tissue::Tissue) -> Vec<([f64; 3], [f64; 3])> {
    let pitch = t.lattice.pitch as f64;
    let mut out = Vec::new();
    for s in &t.structures {
        for e in &s.entrances {
            let (axis, sign) = match e.into {
                Dir6::PosX => (0, 1.0),
                Dir6::NegX => (0, -1.0),
                Dir6::PosZ => (2, 1.0),
                Dir6::NegZ => (2, -1.0),
                _ => continue,
            };
            if !way_in(t, e.slot) {
                continue;
            }
            let c = t.lattice.coords(e.slot as usize);
            let centre = [
                (c[0] as f64 + 0.5) * pitch,
                c[1] as f64 * pitch,
                (c[2] as f64 + 0.5) * pitch,
            ];
            let (mut lo, mut hi) = (centre, centre);
            let far = centre[axis] + sign * (pitch / 2.0 + 0.3);
            lo[axis] = centre[axis].min(far);
            hi[axis] = centre[axis].max(far);
            let across = 2 - axis;
            lo[across] = centre[across] - 2.0;
            hi[across] = centre[across] + 2.0;
            lo[1] = centre[1] - 0.5;
            hi[1] = centre[1];
            out.push((lo, hi));
        }
    }
    out
}

/// The doorways of an L2 cell's masses as `(outside, inside)`: points 2 m either side of the
/// wall, on the floor of the doorway, in cell-local metres (the census and the edge pass read
/// them as ways on).
pub fn doors_of(t: &crate::tissue::Tissue) -> Vec<(DVec3, DVec3)> {
    let em = crate::tissue_grow::Emitter::new(&t.lattice);
    let mut doors = Vec::new();
    for m in mass_specs(t) {
        let part = &t.structures[m.structure].parts[m.part];
        for e in &m.entrances {
            if e.kind != EntranceKind::Door {
                continue;
            }
            let (lo, hi) = entrance_cut(e, part);
            let c = [
                f64::midpoint(lo[0], hi[0]),
                lo[1],
                f64::midpoint(lo[2], hi[2]),
            ];
            let d = match e.side {
                0 => [0.0, 0.0, -1.0],
                1 => [1.0, 0.0, 0.0],
                2 => [0.0, 0.0, 1.0],
                _ => [-1.0, 0.0, 0.0],
            };
            let at = |k: f64| [c[0] + d[0] * k, c[1] + 0.1, c[2] + d[2] * k];
            doors.push((em.at(at(2.0)), em.at(at(-2.0))));
        }
    }
    doors
}

/// Every mass of an L2 cell's tissue, in structure and part order. A part that shares volume
/// with an earlier mass (a fused neighbour, MUT-14) is not a mass: it stays solid, so that every
/// slot of the lattice belongs to at most one plan.
pub fn mass_specs(t: &crate::tissue::Tissue) -> Vec<MassSpec> {
    let coords = |slot: u32| t.lattice.coords(slot as usize).map(|v| v as i32);
    let mut kept: Vec<MassSpec> = Vec::new();
    for (si, s) in t.structures.iter().enumerate() {
        for m in structure_masses(si, s, &coords, &|slot| way_in(t, slot)) {
            if m.entrances.is_empty() {
                continue;
            }
            // A stair's column through a part stays solid (cut for the stair), not a house
            // with a stair through its rooms (PW.4.13).
            let q = |v: Q| f64::from(v) / 16.0;
            let (lo, hi) = (
                [q(m.aabb.min.x), q(m.aabb.min.y), q(m.aabb.min.z)],
                [q(m.aabb.max.x), q(m.aabb.max.y), q(m.aabb.max.z)],
            );
            if t.paths
                .rise_clear
                .iter()
                .any(|(a, b)| (0..3).all(|k| a[k] < hi[k] && lo[k] < b[k]))
            {
                continue;
            }
            if !kept.iter().any(|k| overlap(&k.aabb, &m.aabb)) {
                kept.push(m);
            }
        }
    }
    kept
}

/// The parent context of L2 cell `cell`: the plan of every mass, in lattice coordinates.
pub fn web_parent(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> Parent {
    debug_assert_eq!(cell.level, Level::L2);
    let t = tissue_cell(seed, cache.tissue(), cell);
    parent_of(seed, cell, &t)
}

/// [`web_parent`] from a tissue already grown.
pub fn parent_of(seed: WorldSeed, cell: CellKey, t: &crate::tissue::Tissue) -> Parent {
    let plans = mass_specs(t)
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            let arch = crate::l3_arch::pick(seed, cell, i, m.form);
            MassPlan::new(seed, cell, i, m.aabb, m.entrances, arch)
        })
        .collect();
    Parent::new(plans, Vec::new())
}

/// The L4 modules of one block and where the block stands (`WebL3::cell_modules`).
#[derive(Debug, Clone)]
pub struct BlockModules {
    pub modules: Vec<crate::l4::Module>,
    /// Cell-local metres of the block's origin.
    pub corner: [f64; 3],
    /// The horizontal part of the frame's x axis: where the block's +x points.
    pub ex: [f64; 2],
}

/// An L2 cell's interiors: the plans of its masses, its lattice frame and the blocks solved so
/// far. Build it once per L2 cell and reuse it for every world L3 cell in it.
pub struct WebL3 {
    parent: Parent,
    frame: Frame,
    blocks: HashMap<[i32; 3], L3Block>,
    cell: CellKey,
}

/// The interiors of L2 cell `cell`.
pub fn web_context(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> WebL3 {
    debug_assert_eq!(cell.level, Level::L2);
    let t = tissue_cell(seed, cache.tissue(), cell);
    WebL3 {
        parent: parent_of(seed, cell, &t),
        frame: t.lattice.frame,
        blocks: HashMap::new(),
        cell,
    }
}

impl WebL3 {
    pub fn parent(&self) -> &Parent {
        &self.parent
    }

    /// The cell-local metres of lattice-frame metres `f` (the inverse of `to_lattice`).
    fn to_world(&self, f: [f64; 3]) -> DVec3 {
        let [ex, ey, ez] = self.frame.axes();
        let c = DVec3::new(f[0] - 256.0, f[1] - 256.0, f[2] - 256.0);
        DVec3::splat(256.0) + ex * c.x + ey * c.y + ez * c.z
    }

    fn to_lattice(&self, p: DVec3) -> [f64; 3] {
        let [ex, ey, ez] = self.frame.axes();
        let c = p - DVec3::splat(256.0);
        [ex.dot(c) + 256.0, ey.dot(c) + 256.0, ez.dot(c) + 256.0]
    }

    /// The lattice blocks (index in the cell, 0 to 7 on each axis) that world L3 cell `w` of
    /// this L2 cell meets and that hold a mass.
    pub fn blocks_of(&self, w: CellKey) -> Vec<[i32; 3]> {
        let o = w.coords.map(|v| floor_mod_pow2(v, 3));
        let o = DVec3::new(o.x as f64 * 64.0, o.y as f64 * 64.0, o.z as f64 * 64.0);
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for i in 0..8 {
            let p = DVec3::new(
                o.x + if i & 1 == 0 { 0.0 } else { 64.0 },
                o.y + if i & 2 == 0 { 0.0 } else { 64.0 },
                o.z + if i & 4 == 0 { 0.0 } else { 64.0 },
            );
            let f = self.to_lattice(p);
            for k in 0..3 {
                lo[k] = lo[k].min(f[k]);
                hi[k] = hi[k].max(f[k]);
            }
        }
        let range = |k: usize| {
            ((lo[k] / 64.0).floor() as i32).max(0)..=((hi[k] / 64.0).floor() as i32).min(7)
        };
        let mut out = Vec::new();
        for z in range(2) {
            for y in range(1) {
                for x in range(0) {
                    let key = CellKey::new(
                        Level::L3,
                        self.cell.coords * 8
                            + glam::I64Vec3::new(i64::from(x), i64::from(y), i64::from(z)),
                    );
                    if block_is_active(&self.parent, key) {
                        out.push([x, y, z]);
                    }
                }
            }
        }
        out
    }

    /// Whether world L3 cell `w` has any interior to build.
    pub fn is_active(&self, w: CellKey) -> bool {
        !self.blocks_of(w).is_empty()
    }

    fn block(&mut self, seed: WorldSeed, idx: [i32; 3]) -> &L3Block {
        let key = CellKey::new(
            Level::L3,
            self.cell.coords * 8
                + glam::I64Vec3::new(i64::from(idx[0]), i64::from(idx[1]), i64::from(idx[2])),
        );
        let parent = &self.parent;
        self.blocks
            .entry(idx)
            .or_insert_with(|| l3_block_with(seed, parent, key))
    }

    /// The L4 modules of the blocks that world L3 cell `w` meets (panels, door frames, lamps,
    /// benches, plants, pipes and cables), block by block, for a lattice with level floors (a
    /// yaw only, any heading); none for a tilted lattice, whose interiors stay bare until they
    /// stand upright (PW.4.20). Positions are block-local quanta: the engine places each block
    /// with its `corner` (cell-local metres of the block's origin) and the frame's horizontal
    /// x axis `ex` (the block's z axis is `(-ex.z, ex.x)`), and keeps what lies in `w`.
    pub fn cell_modules(&mut self, seed: WorldSeed, w: CellKey) -> Vec<BlockModules> {
        if !crate::plumb::is_upright(self.frame) {
            return Vec::new();
        }
        let o = w.coords.map(|v| floor_mod_pow2(v, 3));
        let o = DVec3::new(o.x as f64 * 64.0, o.y as f64 * 64.0, o.z as f64 * 64.0);
        let ex = self.frame.axes()[0];
        let mut out = Vec::new();
        for idx in self.blocks_of(w) {
            let corner = self.to_world([
                f64::from(idx[0]) * 64.0,
                f64::from(idx[1]) * 64.0,
                f64::from(idx[2]) * 64.0,
            ]) - o;
            let key = CellKey::new(
                Level::L3,
                self.cell.coords * 8
                    + glam::I64Vec3::new(i64::from(idx[0]), i64::from(idx[1]), i64::from(idx[2])),
            );
            let block = self.block(seed, idx);
            let modules = crate::l4::block_modules(seed, key, block);
            if !modules.is_empty() {
                out.push(BlockModules {
                    modules,
                    corner: corner.to_array(),
                    ex: [ex.x, ex.z],
                });
            }
        }
        out
    }

    /// The interior pieces in world L3 cell `w`, in its local quanta: every block it meets,
    /// turned into place by the lattice frame and clipped to the cell.
    pub fn cell_primitives(&mut self, seed: WorldSeed, w: CellKey) -> Vec<Primitive> {
        let o = w.coords.map(|v| floor_mod_pow2(v, 3));
        let o = DVec3::new(o.x as f64 * 64.0, o.y as f64 * 64.0, o.z as f64 * 64.0);
        let frame = self.frame;
        let clip = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(1024, 1024, 1024));
        let mut out = Vec::new();
        for idx in self.blocks_of(w) {
            let corner = self.to_world([
                f64::from(idx[0]) * 64.0,
                f64::from(idx[1]) * 64.0,
                f64::from(idx[2]) * 64.0,
            ]) - DVec3::new(o.x, o.y, o.z);
            let origin = QVec3::new(quanta(corner.x), quanta(corner.y), quanta(corner.z));
            let block = self.block(seed, idx);
            for p in &block.out.primitives {
                let local = match p.shape {
                    Shape::Box { aabb } => LocalShape::Box { aabb },
                    Shape::StairRun { aabb, rise, steps } => {
                        LocalShape::StairRun { aabb, rise, steps }
                    }
                    Shape::Cylinder {
                        axis,
                        base,
                        length,
                        radius,
                    } => LocalShape::Cylinder {
                        axis,
                        base,
                        length,
                        radius,
                    },
                    Shape::Ramp { aabb, rise } => LocalShape::Ramp { aabb, rise },
                    _ => continue,
                };
                let shape = Shape::oriented(frame, origin, local, clip);
                let b = shape.aabb();
                if b.max.x <= 0
                    || b.min.x >= 1024
                    || b.max.y <= 0
                    || b.min.y >= 1024
                    || b.max.z <= 0
                    || b.min.z >= 1024
                {
                    continue;
                }
                out.push(Primitive { shape, ..*p });
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    clippy::default_trait_access,
    clippy::items_after_statements
)]
mod tests {
    use super::*;
    use crate::l3::{SlotClass, VESTIBULE, block_is_active, l3_block_with};
    use crate::l3_solver::Dims;
    use crate::l3_tiles::{self as tiles, D, N_TILES, O};
    use glam::I64Vec3;
    use std::collections::HashMap;

    fn cells(n: i64) -> impl Iterator<Item = CellKey> {
        (0..n).map(|k| {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            )
        })
    }

    /// The tile of every Core slot of a cell, by global lattice slot, and the mass of each.
    fn solve_cell(seed: WorldSeed, parent: &Parent, cell: CellKey) -> HashMap<[i32; 3], (u16, u8)> {
        let dims = Dims {
            nx: 8,
            nt: 16,
            nz: 8,
        };
        let mut out = HashMap::new();
        for k in 0..512i64 {
            let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
            let block = CellKey::new(Level::L3, cell.coords * 8 + idx);
            if !block_is_active(parent, block) {
                continue;
            }
            let b = l3_block_with(seed, parent, block);
            for (i, s) in b.layout.slots.iter().enumerate() {
                if s.class != SlotClass::Core {
                    continue;
                }
                let (x, t, z) = dims.coords(i);
                out.insert(
                    [
                        idx.x as i32 * 8 + x as i32,
                        idx.y as i32 * 16 + t as i32,
                        idx.z as i32 * 8 + z as i32,
                    ],
                    (s.tile, s.mass),
                );
            }
        }
        out
    }

    fn walkable(tile: u16) -> bool {
        tile == VESTIBULE
            || (usize::from(tile) < N_TILES && tiles::table()[usize::from(tile)].walkable())
    }

    fn socket(tile: u16, side: usize) -> u8 {
        if tile == VESTIBULE {
            D
        } else {
            tiles::table()[usize::from(tile)].sockets()[side]
        }
    }

    /// Walk components over global slots, as the solver's acceptance sees them.
    fn components(slots: &HashMap<[i32; 3], (u16, u8)>) -> HashMap<[i32; 3], usize> {
        let keys: Vec<[i32; 3]> = slots.keys().copied().collect();
        let index: HashMap<[i32; 3], usize> =
            keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let mut parent: Vec<usize> = (0..keys.len()).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for (k, &(tile, mass)) in slots {
            if !walkable(tile) {
                continue;
            }
            for (side, d) in [(1usize, [1, 0, 0]), (2, [0, 0, 1]), (5, [0, 1, 0])] {
                let n = [k[0] + d[0], k[1] + d[1], k[2] + d[2]];
                let Some(&(nt, nm)) = slots.get(&n) else {
                    continue;
                };
                if nm != mass || !walkable(nt) {
                    continue;
                }
                let (a, c) = (
                    socket(tile, side),
                    socket(nt, crate::l3_solver::opposite(side)),
                );
                let linked = if side == 5 {
                    a == c && a >= 4
                } else {
                    (matches!(a, O | D) && matches!(c, O | D) && a == c)
                        || (tile == VESTIBULE && matches!(c, O | D))
                        || (nt == VESTIBULE && matches!(a, O | D))
                };
                if linked {
                    let (ri, rj) = (find(&mut parent, index[k]), find(&mut parent, index[&n]));
                    parent[ri] = rj;
                }
            }
        }
        keys.iter()
            .map(|k| (*k, find(&mut parent, index[k])))
            .collect()
    }

    /// In every Full mass of 120 cells the vestibules of all its entrances are in one walk
    /// component (Lemma L3W-1), and how many masses and entrances there are.
    #[test]
    fn every_entrance_reaches_every_other() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut masses, mut full, mut multi, mut entrances, mut broken) = (0, 0, 0, 0, 0);
        let mut kinds: HashMap<EntranceKind, usize> = HashMap::new();
        let mut tally: HashMap<crate::l3_arch::Archetype, [usize; 6]> = HashMap::new();
        let mut counts: HashMap<crate::l3_arch::Archetype, usize> = HashMap::new();
        for cell in cells(120) {
            let parent = web_parent(seed, &cache, cell);
            if parent.plans().is_empty() {
                continue;
            }
            let slots = solve_cell(seed, &parent, cell);
            let comp = components(&slots);
            for (&k, &(tile, mass)) in &slots {
                let a = parent.plans()[usize::from(mass)].archetype;
                let fam = if tile == VESTIBULE {
                    5
                } else {
                    match tiles::table()[usize::from(tile)].family {
                        tiles::Family::Solid => 0,
                        tiles::Family::Room => 1,
                        tiles::Family::Atrium => 2,
                        tiles::Family::Void(_) => 3,
                        tiles::Family::Corridor => 4,
                        _ => 5,
                    }
                };
                let _ = k;
                tally.entry(a).or_insert([0usize; 6])[fam] += 1;
            }
            for (mi, p) in parent.plans().iter().enumerate() {
                masses += 1;
                *counts.entry(p.archetype).or_default() += 1;
                for e in &p.entrances {
                    *kinds.entry(e.kind).or_default() += 1;
                }
                entrances += p.entrances.len();
                if p.class != crate::l3_plan::PlanClass::Full {
                    continue;
                }
                full += 1;
                if p.entrances.len() < 2 {
                    continue;
                }
                multi += 1;
                let ids: Vec<usize> = p
                    .entrances
                    .iter()
                    .filter_map(|e| comp.get(&[e.cols[0].0, e.t, e.cols[0].1]).copied())
                    .collect();
                if ids.len() != p.entrances.len() || ids.iter().any(|&i| i != ids[0]) {
                    broken += 1;
                    eprintln!("{cell:?} mass {mi}: entrances in {ids:?}");
                }
            }
        }
        eprintln!(
            "{masses} masses, {full} full, {multi} with several entrances, {entrances} entrances {kinds:?}, {broken} broken"
        );
        eprintln!("archetypes {counts:?}");
        let share = |a: crate::l3_arch::Archetype, fam: usize| {
            tally.get(&a).map_or(0.0, |t| {
                t[fam] as f64 / t.iter().sum::<usize>().max(1) as f64
            })
        };
        use crate::l3_arch::Archetype::{MachineFloors, OpenCage, Plain, RackAisles};
        for (a, fam, name) in [
            (OpenCage, 3, "voids"),
            (MachineFloors, 2, "atria"),
            (RackAisles, 0, "racks"),
        ] {
            eprintln!(
                "{a:?}: {name} {:.2} against {:.2} in plain interiors",
                share(a, fam),
                share(Plain, fam)
            );
            if counts.get(&a).copied().unwrap_or(0) >= 5 {
                assert!(
                    share(a, fam) > share(Plain, fam),
                    "{a:?} has no more {name}"
                );
            }
        }
        assert!(
            counts.len() >= 8,
            "only {} archetypes in {masses} masses: {counts:?}",
            counts.len()
        );
        assert!(multi > 20, "only {multi} masses with several entrances");
        assert_eq!(broken, 0);
    }

    /// The interiors in world cells stay in their cells, are the same however the blocks were
    /// reached, and cover the L2 cell's masses.
    #[test]
    fn world_cells_hold_their_interiors() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut active, mut pieces) = (0, 0usize);
        for cell in cells(12) {
            let mut ctx = web_context(seed, &cache, cell);
            if ctx.parent().plans().is_empty() {
                continue;
            }
            for k in 0..512i64 {
                let w = CellKey::new(
                    Level::L3,
                    cell.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                );
                if !ctx.is_active(w) {
                    continue;
                }
                active += 1;
                let a = ctx.cell_primitives(seed, w);
                let b = ctx.cell_primitives(seed, w);
                assert_eq!(a, b, "{w:?} differs between two builds");
                pieces += a.len();
                for p in &a {
                    let bb = p.aabb();
                    assert!(
                        bb.min.x >= 0
                            && bb.min.y >= 0
                            && bb.min.z >= 0
                            && bb.max.x <= 1024
                            && bb.max.y <= 1024
                            && bb.max.z <= 1024,
                        "{w:?}: {bb:?}"
                    );
                }
            }
        }
        eprintln!("{active} active world cells, {pieces} pieces");
        assert!(active > 20 && pieces > 1000);
    }

    /// Prints places to stand outside the doors of masses, for the game's teleport variables.
    #[test]
    #[ignore = "prints viewpoints"]
    fn print_door_viewpoints() {
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        let mut shown = 0;
        for cell in cells(60) {
            let t = tissue_cell(seed, cache.tissue(), cell);
            let ctx = web_context(seed, &cache, cell);
            for m in mass_specs(&t) {
                let Some(e) = m.entrances.iter().find(|e| e.kind == EntranceKind::Door) else {
                    continue;
                };
                // The route slot outside the door: the slot cell beside the face at the door.
                let (cx, cz) = (e.cols[0].0, e.cols[0].1);
                let mut f = [
                    f64::from(cx) * 8.0 + 4.0,
                    f64::from(e.t) * 4.0 + 1.65,
                    f64::from(cz) * 8.0 + 4.0,
                ];
                if std::env::var("INSIDE").is_err() {
                    match e.side {
                        0 => f[2] -= 12.0,
                        1 => f[0] += 12.0,
                        2 => f[2] += 12.0,
                        _ => f[0] -= 12.0,
                    }
                } else {
                    f[1] += 0.3;
                }
                let p = ctx.to_world(f);
                let [ex, _, ez] = ctx.frame.axes();
                let into = match e.side {
                    0 => ez,
                    1 => -ex,
                    2 => -ez,
                    _ => ex,
                };
                let yaw = (-into.x).atan2(-into.z).to_degrees().rem_euclid(360.0);
                let w = cell.coords * 512 + I64Vec3::new(p.x as i64, p.y as i64, p.z as i64);
                let (sec, loc) = (
                    w.div_euclid(I64Vec3::splat(1024)),
                    w.rem_euclid(I64Vec3::splat(1024)),
                );
                eprintln!(
                    "SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={}.0,{}.0,{}.0 SUPERSTRUCT_YAW={yaw:.0} (door side {})",
                    sec.x, sec.y, sec.z, loc.x, loc.y, loc.z, e.side
                );
                shown += 1;
                break;
            }
            if shown >= 6 {
                break;
            }
        }
    }

    /// Every corridor kind (L3-14..20) is drawn somewhere, and the blocks that hold them still
    /// build inside their cell.
    #[test]
    fn every_corridor_kind_occurs() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let mut kinds: HashMap<u8, usize> = HashMap::new();
        for cell in cells(40) {
            let mut ctx = web_context(seed, &cache, cell);
            for k in 0..512i64 {
                let idx = [(k % 8) as i32, (k / 64) as i32, (k / 8 % 8) as i32];
                let key = CellKey::new(
                    Level::L3,
                    cell.coords * 8
                        + I64Vec3::new(i64::from(idx[0]), i64::from(idx[1]), i64::from(idx[2])),
                );
                if !block_is_active(ctx.parent(), key) {
                    continue;
                }
                for &(kind, n) in &ctx.block(seed, idx).layout.corridors {
                    *kinds.entry(kind).or_default() += usize::from(n);
                }
            }
        }
        eprintln!("corridor kinds {kinds:?}");
        for kind in 14..=20u8 {
            assert!(
                kinds.get(&kind).copied().unwrap_or(0) > 0,
                "L3-{kind} never occurs: {kinds:?}"
            );
        }
    }

    /// The floor of every vestibule is where the door puts it: in the world cell that holds the
    /// point, the pieces contain a point 0.1 m above the storey line under the vestibule's
    /// centre, in world space (the check of the transform from lattice to cell).
    #[test]
    fn vestibule_floors_are_in_their_world_cells() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut checked, mut missing) = (0, 0);
        for cell in cells(30) {
            let t = tissue_cell(seed, cache.tissue(), cell);
            let mut ctx = web_context(seed, &cache, cell);
            for m in mass_specs(&t) {
                for e in m
                    .entrances
                    .iter()
                    .filter(|e| e.kind == EntranceKind::Door)
                    .take(1)
                {
                    let (cx, cz) = e.cols[0];
                    let f = [
                        f64::from(cx) * 8.0 + 4.0,
                        f64::from(e.t) * 4.0 + 0.1,
                        f64::from(cz) * 8.0 + 4.0,
                    ];
                    let p = ctx.to_world(f);
                    if p.min_element() < 1.0 || p.max_element() > 511.0 {
                        continue;
                    }
                    let w = CellKey::new(
                        Level::L3,
                        cell.coords * 8
                            + I64Vec3::new(
                                (p.x / 64.0) as i64,
                                (p.y / 64.0) as i64,
                                (p.z / 64.0) as i64,
                            ),
                    );
                    let o = DVec3::new(
                        (p.x / 64.0).floor() * 64.0,
                        (p.y / 64.0).floor() * 64.0,
                        (p.z / 64.0).floor() * 64.0,
                    );
                    let q =
                        glam::Vec3::new((p.x - o.x) as f32, (p.y - o.y) as f32, (p.z - o.z) as f32);
                    let prims = ctx.cell_primitives(seed, w);
                    checked += 1;
                    if !prims.iter().any(|pr| pr.shape.contains(q)) {
                        missing += 1;
                    }
                }
            }
        }
        eprintln!("{checked} vestibules, {missing} without a floor under them");
        assert!(checked > 100);
        assert!(missing * 20 <= checked, "{missing} of {checked}");
    }

    /// Every set-piece room (INT-01..19) is drawn somewhere.
    #[test]
    fn every_room_occurs() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let mut kinds: HashMap<u8, usize> = HashMap::new();
        for cell in cells(30) {
            let mut ctx = web_context(seed, &cache, cell);
            for k in 0..512i64 {
                let idx = [(k % 8) as i32, (k / 64) as i32, (k / 8 % 8) as i32];
                let key = CellKey::new(
                    Level::L3,
                    cell.coords * 8
                        + I64Vec3::new(i64::from(idx[0]), i64::from(idx[1]), i64::from(idx[2])),
                );
                if !block_is_active(ctx.parent(), key) {
                    continue;
                }
                for &(kind, n) in &ctx.block(seed, idx).layout.rooms {
                    *kinds.entry(kind).or_default() += usize::from(n);
                }
            }
        }
        eprintln!("rooms {kinds:?}");
        for kind in 1..=19u8 {
            assert!(
                kinds.get(&kind).copied().unwrap_or(0) > 0,
                "INT-{kind:02} never occurs: {kinds:?}"
            );
        }
    }

    /// PW.4.21: the interiors of upright lattices carry L4 modules, with lamps (a light a block),
    /// and a tilted lattice has none.
    #[test]
    fn upright_interiors_are_furnished_and_lit() {
        use crate::l4::ModuleKind;
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut blocks, mut lamps, mut panels, mut tilted) = (0usize, 0usize, 0usize, 0usize);
        for key in cells(40) {
            let mut ctx = web_context(seed, &cache, key);
            for k in 0..512i64 {
                let w = CellKey::new(
                    Level::L3,
                    key.coords * 8 + I64Vec3::new(k % 8, k / 8 % 8, k / 64),
                );
                if !ctx.is_active(w) {
                    continue;
                }
                let m = ctx.cell_modules(seed, w);
                if !crate::plumb::is_upright(ctx.frame) {
                    tilted += usize::from(m.is_empty());
                    continue;
                }
                for b in &m {
                    blocks += 1;
                    lamps += b
                        .modules
                        .iter()
                        .filter(|x| x.kind == ModuleKind::Lamp)
                        .count();
                    panels += b
                        .modules
                        .iter()
                        .filter(|x| x.kind == ModuleKind::WallPanel)
                        .count();
                }
            }
        }
        eprintln!(
            "{blocks} block placements, {lamps} lamps, {panels} panels, {tilted} tilted cells bare"
        );
        assert!(blocks > 20 && lamps >= blocks && panels > blocks);
    }
}
