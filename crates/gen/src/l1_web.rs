//! The Web L1 cell (version 7): Frame, Between content, crust surfaces, voids and the skeleton,
//! assembled into one `CellOutput`. Spec: `docs/spec/levels.md` §4W.
//!
//! Draw order is fixed by the modules themselves (each hangs its features on keys, so no
//! order of calls matters). The output order is: beams and knot, Between content, surfaces,
//! standing matter inside voids. Voids' keep-out shapes, then one `ForcedPort` per artery
//! crossing, are the child constraints.

use crate::cavity::CavityView;
use crate::cell::{Axis, CellKey, FaceKey, Level};
use crate::l1_artery::{ArteryPort, ArteryType, L2c, Skeleton, coords, skeleton};
use crate::l1_between::{WebPiece, between_content};
use crate::l1_frame::frame_content;
use crate::l1_surface::surface_content;
use crate::l1_void::voids_of_cell;
use crate::port::{Port, PortKind};
use crate::prim::{CellMeta, CellOutput, ChildConstraint, Primitive, Q, QRect, Shape};
use crate::seed::WorldSeed;
use glam::I64Vec3;

/// L2 cell edge in quanta.
const L2_Q: Q = 512 * 16;

/// Everything the Web L1 emits for `cell`. Cavity cells emit nothing.
#[allow(clippy::cast_possible_truncation)] // port counts are small
pub fn l1_cell(seed: WorldSeed, cell: CellKey) -> CellOutput {
    debug_assert_eq!(cell.level, Level::L1);
    let mut out = CellOutput {
        key: cell,
        primitives: Vec::new(),
        ports: Vec::new(),
        child_constraints: Vec::new(),
        meta: CellMeta::default(),
    };
    let l1 = cell.coords;
    if CavityView::new(seed, cell).is_cavity(cell) {
        return out;
    }
    let pieces = |v: Vec<WebPiece>| v.into_iter().map(|w| w.prim);
    out.primitives.extend(pieces(frame_content(seed, l1)));
    out.primitives.extend(pieces(between_content(seed, l1)));
    out.primitives.extend(pieces(surface_content(seed, l1)));
    for v in voids_of_cell(seed, l1) {
        if v.solid {
            out.primitives.push(v.piece.prim);
        } else {
            out.child_constraints.push(ChildConstraint::KeepOut {
                aabb: v.piece.prim.aabb(),
            });
        }
    }
    add_staircase(seed, l1, &mut out);
    if let Some(s) = skeleton(seed, l1) {
        out.ports = skeleton_ports(l1, &s);
        out.child_constraints
            .extend((0..out.ports.len()).map(|i| ChildConstraint::ForcedPort { port: i as u32 }));
    }
    out
}

/// The infinite staircase of this L1 column, if it has one (STAIR-01..04): the other pieces of
/// the cell are cut out of its footprint, which stays clear of everything the cell's children
/// make, and the open flights and the core go in.
fn add_staircase(seed: WorldSeed, l1: I64Vec3, out: &mut CellOutput) {
    let column = crate::key::ColumnKey {
        level: Level::L1,
        x: l1.x,
        z: l1.z,
    };
    let Some(foot) = crate::l1::stair_footprint(seed, column) else {
        return;
    };
    let keep = crate::l1_stair::keep_out(foot);
    let bore = Primitive {
        shape: Shape::Box { aabb: keep },
        material: crate::prim::MaterialId::CONCRETE,
        kind: crate::prim::FeatureKind::MISC,
        flags: crate::prim::PrimFlags::NO_COLLIDE,
    };
    let prims = std::mem::take(&mut out.primitives);
    out.primitives = crate::carve::carve(prims, &[bore], [0; 3]).0;
    // What the carve cannot cut (cables, turned beams) goes if it really enters the footprint.
    out.primitives.retain(|p| !enters(p, &keep));
    out.child_constraints
        .push(ChildConstraint::KeepOut { aabb: keep });
    out.primitives.extend(crate::l1_stair::stair_pieces(
        foot,
        crate::l1_stair::style(seed, column),
    ));
}

/// Whether the shape of `p` has a point inside the column `keep`, found by sampling the part of
/// the shape's box that overlaps it every 1 m across and 8 m along.
pub(crate) fn enters(p: &Primitive, keep: &crate::prim::QAabb) -> bool {
    use glam::Vec3;
    let b = p.aabb();
    let lo = [
        b.min.x.max(keep.min.x),
        b.min.y.max(keep.min.y),
        b.min.z.max(keep.min.z),
    ];
    let hi = [
        b.max.x.min(keep.max.x),
        b.max.y.min(keep.max.y),
        b.max.z.min(keep.max.z),
    ];
    if (0..3).any(|k| lo[k] >= hi[k]) {
        return false;
    }
    let step = |a: Q, z: Q, s: usize| (a..z).step_by(s);
    for x in step(lo[0], hi[0], 16) {
        for z in step(lo[2], hi[2], 16) {
            for y in step(lo[1], hi[1], 128) {
                let v = Vec3::new(x as f32 / 16.0, y as f32 / 16.0, z as f32 / 16.0);
                if p.shape.contains(v) {
                    return true;
                }
            }
        }
    }
    false
}

#[allow(clippy::cast_possible_truncation)] // port counts are small
fn skeleton_ports(l1: I64Vec3, s: &Skeleton) -> Vec<Port> {
    let whole = QRect {
        u0: 0,
        v0: 0,
        u1: L2_Q,
        v1: L2_Q,
    };
    let mut ports: Vec<Port> = Vec::with_capacity(s.ports.len() + s.arteries.len() * 8);
    // The ports on the six L1 faces: each is the L2 cell's side of the face.
    for &(_, p) in &s.ports {
        ports.push(l1_face_port(&p, whole));
    }
    // Every L2 face crossed by an artery, once per type.
    let mut seen: Vec<(CellKey, Axis, ArteryType)> = Vec::new();
    for (a, b, kind) in s.links() {
        let (ca, cb) = (coords(a as usize), coords(b as usize));
        let (axis, lower) = crossing(ca, cb);
        let owner = CellKey::new(Level::L2, l1 * 8 + I64Vec3::from_array(lower));
        if seen.contains(&(owner, axis, kind)) {
            continue;
        }
        seen.push((owner, axis, kind));
        let id = ports
            .iter()
            .filter(|p| p.face == FaceKey { owner, axis })
            .count() as u16;
        ports.push(Port {
            face: FaceKey { owner, axis },
            id,
            kind: PortKind::Artery { kind },
            rect: whole,
        });
    }
    ports
}

fn l1_face_port(p: &ArteryPort, rect: QRect) -> Port {
    Port {
        face: p.face,
        id: u16::from(p.id),
        kind: PortKind::Artery { kind: p.kind },
        rect,
    }
}

/// The axis along which neighbouring L2 cells `a` and `b` touch, and the lower of the two.
fn crossing(a: L2c, b: L2c) -> (Axis, L2c) {
    let k = (0..3).find(|&k| a[k] != b[k]).unwrap_or(0);
    let axis = Axis::ALL[k];
    (axis, if a[k] < b[k] { a } else { b })
}
