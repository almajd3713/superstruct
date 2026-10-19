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
use crate::prim::{CellMeta, CellOutput, ChildConstraint, Q, QRect};
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
    if let Some(s) = skeleton(seed, l1) {
        out.ports = skeleton_ports(l1, &s);
        out.child_constraints
            .extend((0..out.ports.len()).map(|i| ChildConstraint::ForcedPort { port: i as u32 }));
    }
    out
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
