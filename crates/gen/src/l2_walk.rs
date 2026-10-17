//! Skywalks at L2 walkway ports. Spec: `docs/spec/levels.md` §5.3 and `docs/spec/ports.md` §4.2.
//!
//! A walkway port sits above a deck. Each of the two L2 cells beside the face builds its half:
//! a grating slab at the port's floor, then a stair down to the deck. Both halves lie inside a
//! corridor of fixed depth, which port derivation checks against the L1 data, so a realization
//! never fails and never depends on anything else in the cell.

use crate::cell::Axis;
use crate::l1::Rect;
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};

const M: Q = 16;
/// L2 cell edge in quanta.
pub const CELL: Q = 512 * M;
/// Depth of the corridor reserved from the face, in quanta (64 m).
pub const DEPTH: Q = 64 * M;
/// Length of the grating slab from the face.
const LANDING: Q = 8 * M;
/// Slab thickness, 0.5 m.
const SLAB: Q = 8;
/// Stair rise and tread (`traversal.md` §6).
const RISE: Q = 3;
const TREAD: Q = 5;
/// Clear height above the floor kept open in the corridor.
const CLEARANCE: Q = 4 * M;

/// One cell's half of a skywalk, in that cell's local space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skywalk {
    /// Axis of the face: X or Z.
    pub axis: Axis,
    /// Face plane position along `axis`: 0 or `CELL`.
    pub plane: Q,
    /// Tangential extent along the face (`u`: z for X faces, x for Z faces).
    pub u0: Q,
    pub u1: Q,
    /// Floor height at the face.
    pub floor: Q,
    /// Floor minus the deck top it descends to.
    pub height: Q,
}

impl Skywalk {
    /// Direction of `+n`, from the face into the cell.
    fn inward(&self) -> Q {
        if self.plane == 0 { 1 } else { -1 }
    }

    /// Plan range along the axis covering `[a, b)` measured inward from the face.
    fn span(&self, a: Q, b: Q) -> (Q, Q) {
        if self.inward() > 0 {
            (self.plane + a, self.plane + b)
        } else {
            (self.plane - b, self.plane - a)
        }
    }

    fn aabb(&self, n: (Q, Q), y: (Q, Q)) -> QAabb {
        let (a, b) = self.span(n.0, n.1);
        let (min, max) = match self.axis {
            Axis::X => (QVec3::new(a, y.0, self.u0), QVec3::new(b, y.1, self.u1)),
            _ => (QVec3::new(self.u0, y.0, a), QVec3::new(self.u1, y.1, b)),
        };
        QAabb::new(min, max)
    }

    fn steps(&self) -> Q {
        self.height / RISE
    }

    /// The reserved volume: the corridor from the deck top to the clear height above the floor.
    pub fn corridor(&self) -> QAabb {
        self.aabb(
            (0, DEPTH),
            (self.floor - self.height, self.floor + CLEARANCE),
        )
    }

    /// Plan footprint of the stair, which must stand on deck.
    pub fn foot(&self) -> Rect {
        let bb = self.aabb((LANDING, LANDING + self.steps() * TREAD), (0, 1));
        Rect {
            x0: bb.min.x,
            z0: bb.min.z,
            x1: bb.max.x,
            z1: bb.max.z,
        }
    }

    /// The slab and the stair. The stair ends up to 2 quanta below the slab, which is a step.
    pub fn primitives(&self) -> Vec<Primitive> {
        if self.steps() == 0 {
            return Vec::new();
        }
        let run = self.steps() * TREAD;
        let top = self.floor - self.height + self.steps() * RISE;
        let ascend = match (self.axis, self.inward() > 0) {
            (Axis::X, true) => Dir4::NegX,
            (Axis::X, false) => Dir4::PosX,
            (_, true) => Dir4::NegZ,
            (_, false) => Dir4::PosZ,
        };
        let slab = Primitive {
            shape: Shape::Box {
                aabb: self.aabb((0, LANDING), (self.floor - SLAB, self.floor)),
            },
            material: MaterialId::GRATING,
            kind: FeatureKind::WALKWAY,
            flags: PrimFlags::NONE,
        };
        let stair = Primitive {
            shape: Shape::stair_run(
                self.aabb((LANDING, LANDING + run), (self.floor - self.height, top)),
                ascend,
                u16::try_from(self.steps()).unwrap_or(u16::MAX),
            ),
            material: MaterialId::METAL,
            kind: FeatureKind::RAMP,
            flags: PrimFlags::NONE,
        };
        vec![slab, stair]
    }
}
