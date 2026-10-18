//! Geometric primitives a generator level emits. Spec: `docs/spec/primitives.md`.
//!
//! Geometry is integer quanta of 1/16 m in the owning cell's local space.

use crate::cell::{Axis, CellKey};
use crate::port::Port;

/// One quantum is 1/16 m.
pub type Q = i32;

/// Quanta per metre.
pub const Q_PER_M: Q = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QVec3 {
    pub x: Q,
    pub y: Q,
    pub z: Q,
}

impl QVec3 {
    pub const fn new(x: Q, y: Q, z: Q) -> Self {
        Self { x, y, z }
    }

    pub const fn get(self, axis: Axis) -> Q {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }
}

/// Half-open on every axis, with `min < max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QAabb {
    pub min: QVec3,
    pub max: QVec3,
}

impl QAabb {
    pub fn new(min: QVec3, max: QVec3) -> Self {
        debug_assert!(
            min.x < max.x && min.y < max.y && min.z < max.z,
            "empty QAabb {min:?}..{max:?}"
        );
        Self { min, max }
    }

    pub const fn size(&self, axis: Axis) -> Q {
        self.max.get(axis) - self.min.get(axis)
    }

    /// The smallest box containing both.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: QVec3::new(
                self.min.x.min(other.min.x),
                self.min.y.min(other.min.y),
                self.min.z.min(other.min.z),
            ),
            max: QVec3::new(
                self.max.x.max(other.max.x),
                self.max.y.max(other.max.y),
                self.max.z.max(other.max.z),
            ),
        }
    }
}

/// A rectangle in a face plane `(u, v)`, half-open, with `u0 < u1` and `v0 < v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QRect {
    pub u0: Q,
    pub v0: Q,
    pub u1: Q,
    pub v1: Q,
}

/// Direction of ascent for ramps and stairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Dir4 {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Dir4 {
    pub const ALL: [Self; 4] = [Self::PosX, Self::NegX, Self::PosZ, Self::NegZ];

    /// The horizontal axis the run lies along.
    pub const fn axis(self) -> Axis {
        match self {
            Self::PosX | Self::NegX => Axis::X,
            Self::PosZ | Self::NegZ => Axis::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shape {
    Box {
        aabb: QAabb,
    },
    Ramp {
        aabb: QAabb,
        rise: Dir4,
    },
    /// `base` is the centre of the cap at the axis minimum.
    Cylinder {
        axis: Axis,
        base: QVec3,
        length: Q,
        radius: Q,
    },
    StairRun {
        aabb: QAabb,
        rise: Dir4,
        steps: u16,
    },
    /// Not solid: a reserved vertical void for a lift platform.
    LiftShaft {
        aabb: QAabb,
    },
}

impl Shape {
    pub fn cylinder(axis: Axis, base: QVec3, length: Q, radius: Q) -> Self {
        debug_assert!(length > 0 && radius > 0, "degenerate cylinder");
        Self::Cylinder {
            axis,
            base,
            length,
            radius,
        }
    }

    /// Height and run must both divide evenly into `steps`, so every step is whole quanta.
    pub fn stair_run(aabb: QAabb, rise: Dir4, steps: u16) -> Self {
        debug_assert!(steps > 0, "stair run with no steps");
        debug_assert!(
            aabb.size(Axis::Y) % Q::from(steps) == 0
                && aabb.size(rise.axis()) % Q::from(steps) == 0,
            "stair run {aabb:?} not divisible into {steps} steps"
        );
        Self::StairRun { aabb, rise, steps }
    }

    pub fn aabb(&self) -> QAabb {
        match *self {
            Self::Box { aabb }
            | Self::Ramp { aabb, .. }
            | Self::StairRun { aabb, .. }
            | Self::LiftShaft { aabb } => aabb,
            Self::Cylinder {
                axis,
                base,
                length,
                radius,
            } => {
                let lo = |a: Axis| {
                    if a == axis {
                        base.get(a)
                    } else {
                        base.get(a) - radius
                    }
                };
                let hi = |a: Axis| {
                    if a == axis {
                        base.get(a) + length
                    } else {
                        base.get(a) + radius
                    }
                };
                QAabb::new(
                    QVec3::new(lo(Axis::X), lo(Axis::Y), lo(Axis::Z)),
                    QVec3::new(hi(Axis::X), hi(Axis::Y), hi(Axis::Z)),
                )
            }
        }
    }
}

/// Index into the material table. Append-only: renumbering changes output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaterialId(pub u16);

impl MaterialId {
    pub const CONCRETE: Self = Self(0);
    pub const METAL: Self = Self(1);
    pub const GRATING: Self = Self(2);
    pub const EMISSIVE: Self = Self(3);
    pub const PIPE: Self = Self(4);
    pub const WARM: Self = Self(5);
    /// Player beacons: drawn by the engine, mostly through fog. Never emitted by the generator.
    pub const BEACON: Self = Self(6);
}

/// What a primitive belongs to. Append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeatureKind(pub u8);

impl FeatureKind {
    pub const TRUNK: Self = Self(0);
    pub const DECK: Self = Self(1);
    pub const SHAFT_WALL: Self = Self(2);
    pub const BRIDGE: Self = Self(3);
    pub const MASS: Self = Self(4);
    pub const STAIR_TOWER: Self = Self(5);
    pub const RAMP: Self = Self(6);
    pub const WALKWAY: Self = Self(7);
    pub const RAILING: Self = Self(8);
    pub const PIPE: Self = Self(9);
    pub const SANCTUARY: Self = Self(10);
    pub const MISC: Self = Self(11);
    pub const TRAM: Self = Self(12);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PrimFlags(pub u8);

impl PrimFlags {
    pub const NONE: Self = Self(0);
    /// A placeholder the child level refines (spec §5).
    pub const ENVELOPE: Self = Self(1 << 0);
    /// Visual only: collision and walkability ignore it.
    pub const NO_COLLIDE: Self = Self(1 << 1);
    /// Part of the far-band reduction and the SDF horizon.
    pub const FAR_VISIBLE: Self = Self(1 << 2);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for PrimFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Primitive {
    pub shape: Shape,
    pub material: MaterialId,
    pub kind: FeatureKind,
    pub flags: PrimFlags,
}

impl Primitive {
    pub fn aabb(&self) -> QAabb {
        self.shape.aabb()
    }
}

/// Everything a level emits for one cell (spec §7). All vectors are in a fixed,
/// code-defined order, and all coordinates are in the cell's local space.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellOutput {
    pub key: CellKey,
    pub primitives: Vec<Primitive>,
    pub ports: Vec<Port>,
    pub child_constraints: Vec<ChildConstraint>,
    pub meta: CellMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChildConstraint {
    /// Children may not place collidable solid here.
    KeepOut { aabb: QAabb },
    /// Children refine `primitives[prim]`, an `ENVELOPE`.
    Envelope { prim: u32 },
    /// Children must realize `ports[port]` on their boundary.
    ForcedPort { port: u32 },
    /// A region with a purpose children must honour.
    Zone { aabb: QAabb, kind: ZoneKind },
}

/// Append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ZoneKind {
    Sanctuary,
    Vista,
    GiantCavity,
}

/// Level-specific summary (`levels.md`). Fields are added by the items that produce them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellMeta {
    pub flags: MetaFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MetaFlags(pub u8);

impl MetaFlags {
    pub const NONE: Self = Self(0);
    pub const SANCTUARY: Self = Self(1 << 0);
    pub const VISTA: Self = Self(1 << 1);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(min: (Q, Q, Q), max: (Q, Q, Q)) -> QAabb {
        QAabb::new(
            QVec3::new(min.0, min.1, min.2),
            QVec3::new(max.0, max.1, max.2),
        )
    }

    #[test]
    fn box_like_shapes_return_their_aabb() {
        let aabb = bx((-16, 0, 3), (32, 8, 48));
        for shape in [
            Shape::Box { aabb },
            Shape::Ramp {
                aabb,
                rise: Dir4::NegZ,
            },
            Shape::stair_run(aabb, Dir4::PosX, 4),
            Shape::LiftShaft { aabb },
        ] {
            assert_eq!(shape.aabb(), aabb, "{shape:?}");
        }
    }

    #[test]
    fn cylinder_aabb_per_axis() {
        let base = QVec3::new(100, -50, 7);
        let cases = [
            (Axis::X, bx((100, -54, 3), (140, -46, 11))),
            (Axis::Y, bx((96, -50, 3), (104, -10, 11))),
            (Axis::Z, bx((96, -54, 7), (104, -46, 47))),
        ];
        for (axis, expected) in cases {
            assert_eq!(
                Shape::cylinder(axis, base, 40, 4).aabb(),
                expected,
                "{axis:?}"
            );
        }
    }

    #[test]
    fn primitive_aabb_is_shape_aabb() {
        let shape = Shape::cylinder(Axis::Y, QVec3::new(0, 0, 0), 64, 16);
        let prim = Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::TRUNK,
            flags: PrimFlags::FAR_VISIBLE,
        };
        assert_eq!(prim.aabb(), shape.aabb());
    }

    #[test]
    fn union_covers_both() {
        let a = bx((0, 0, 0), (4, 4, 4));
        let b = bx((-2, 3, 1), (2, 9, 5));
        assert_eq!(a.union(&b), bx((-2, 0, 0), (4, 9, 5)));
    }

    #[test]
    fn dir4_axis() {
        let axes = Dir4::ALL.map(Dir4::axis);
        assert_eq!(axes, [Axis::X, Axis::X, Axis::Z, Axis::Z]);
    }

    #[test]
    #[should_panic(expected = "not divisible")]
    #[cfg(debug_assertions)]
    fn stair_run_rejects_fractional_steps() {
        let _ = Shape::stair_run(bx((0, 0, 0), (10, 9, 4)), Dir4::PosX, 4);
    }

    #[test]
    fn flags_combine() {
        let f = PrimFlags::ENVELOPE | PrimFlags::FAR_VISIBLE;
        assert!(f.contains(PrimFlags::ENVELOPE));
        assert!(!f.contains(PrimFlags::NO_COLLIDE));
    }

    /// A `CellOutput` that uses every shape, constraint and port field.
    #[cfg(feature = "serde")]
    fn sample_output() -> CellOutput {
        use crate::cell::{FaceKey, Level};
        use crate::port::PortKind;
        use glam::I64Vec3;

        let key = CellKey::new(Level::L2, I64Vec3::new(-3, 7, i64::from(i32::MIN)));
        let aabb = bx((-16, 0, 3), (32, 8, 48));
        let prim = |shape, kind| Primitive {
            shape,
            material: MaterialId::METAL,
            kind,
            flags: PrimFlags::ENVELOPE | PrimFlags::FAR_VISIBLE,
        };
        CellOutput {
            key,
            primitives: vec![
                prim(Shape::Box { aabb }, FeatureKind::MASS),
                prim(
                    Shape::Ramp {
                        aabb,
                        rise: Dir4::NegX,
                    },
                    FeatureKind::RAMP,
                ),
                prim(
                    Shape::cylinder(Axis::Z, QVec3::new(1, -2, 3), 40, 4),
                    FeatureKind::PIPE,
                ),
                prim(
                    Shape::stair_run(bx((0, 0, 0), (16, 8, 32)), Dir4::PosZ, 4),
                    FeatureKind::STAIR_TOWER,
                ),
                prim(Shape::LiftShaft { aabb }, FeatureKind::MISC),
            ],
            ports: vec![Port {
                face: FaceKey {
                    owner: key,
                    axis: Axis::X,
                },
                id: 2,
                kind: PortKind::BridgeEnd,
                rect: QRect {
                    u0: 128,
                    v0: 64,
                    u1: 256,
                    v1: 128,
                },
            }],
            child_constraints: vec![
                ChildConstraint::KeepOut { aabb },
                ChildConstraint::Envelope { prim: 0 },
                ChildConstraint::ForcedPort { port: 0 },
                ChildConstraint::Zone {
                    aabb,
                    kind: ZoneKind::Sanctuary,
                },
            ],
            meta: CellMeta {
                flags: MetaFlags::SANCTUARY,
            },
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn cell_output_serde_round_trip() {
        let out = sample_output();
        let json = serde_json::to_string(&out).unwrap();
        let back: CellOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, out);
    }
}
