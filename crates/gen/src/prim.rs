//! Geometric primitives a generator level emits. Spec: `docs/spec/primitives.md`.
//!
//! Geometry is integer quanta of 1/16 m in the owning cell's local space.

/// One quantum is 1/16 m.
pub type Q = i32;

/// Quanta per metre.
pub const Q_PER_M: Q = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QVec3 {
    pub x: Q,
    pub y: Q,
    pub z: Q,
}

impl QVec3 {
    pub const fn new(x: Q, y: Q, z: Q) -> Self {
        Self { x, y, z }
    }

    pub const fn get(self, axis: Axis3) -> Q {
        match axis {
            Axis3::X => self.x,
            Axis3::Y => self.y,
            Axis3::Z => self.z,
        }
    }
}

/// Half-open on every axis, with `min < max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    pub const fn size(&self, axis: Axis3) -> Q {
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

/// Direction of ascent for ramps and stairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir4 {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Dir4 {
    pub const ALL: [Self; 4] = [Self::PosX, Self::NegX, Self::PosZ, Self::NegZ];

    /// The horizontal axis the run lies along.
    pub const fn axis(self) -> Axis3 {
        match self {
            Self::PosX | Self::NegX => Axis3::X,
            Self::PosZ | Self::NegZ => Axis3::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis3 {
    X,
    Y,
    Z,
}

impl Axis3 {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        axis: Axis3,
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
    pub fn cylinder(axis: Axis3, base: QVec3, length: Q, radius: Q) -> Self {
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
            aabb.size(Axis3::Y) % Q::from(steps) == 0
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
                let lo = |a: Axis3| {
                    if a == axis {
                        base.get(a)
                    } else {
                        base.get(a) - radius
                    }
                };
                let hi = |a: Axis3| {
                    if a == axis {
                        base.get(a) + length
                    } else {
                        base.get(a) + radius
                    }
                };
                QAabb::new(
                    QVec3::new(lo(Axis3::X), lo(Axis3::Y), lo(Axis3::Z)),
                    QVec3::new(hi(Axis3::X), hi(Axis3::Y), hi(Axis3::Z)),
                )
            }
        }
    }
}

/// Index into the material table. Append-only: renumbering changes output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialId(pub u16);

impl MaterialId {
    pub const CONCRETE: Self = Self(0);
    pub const METAL: Self = Self(1);
    pub const GRATING: Self = Self(2);
    pub const EMISSIVE: Self = Self(3);
    pub const PIPE: Self = Self(4);
    pub const WARM: Self = Self(5);
}

/// What a primitive belongs to. Append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
            (Axis3::X, bx((100, -54, 3), (140, -46, 11))),
            (Axis3::Y, bx((96, -50, 3), (104, -10, 11))),
            (Axis3::Z, bx((96, -54, 7), (104, -46, 47))),
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
        let shape = Shape::cylinder(Axis3::Y, QVec3::new(0, 0, 0), 64, 16);
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
        assert_eq!(axes, [Axis3::X, Axis3::X, Axis3::Z, Axis3::Z]);
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
}
