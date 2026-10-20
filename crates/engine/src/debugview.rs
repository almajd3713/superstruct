//! Debug views (PW.4.7): what is a pixel? Every vertex carries a 16-bit tag (feature kind, frame
//! class, collides or not, level); `scene.wgsl` colours by one of them, and the pick key prints
//! the primitive under the crosshair. All of it is off unless `SUPERSTRUCT_DEBUG_VIEWS=1`,
//! because tagging stops axis-aligned boxes of different kinds from merging into one quad.
//!
//! Tag bits: 0-4 kind, 5-6 frame class, 7 no-collide, 8-9 level (1 to 3, 0 unknown), 10 envelope,
//! 11 interior.

use std::sync::atomic::{AtomicBool, Ordering};

use worldgen::cell::Level;
use worldgen::orient::Frame;
use worldgen::prim::{PrimFlags, Primitive, Shape};

use crate::vertex::Vertex;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Colour modes the view key cycles through; 0 is the normal look.
pub const MODE_NAMES: [&str; 6] = ["off", "level", "kind", "frame", "chunk", "collides"];

pub const FRAME_NAMES: [&str; 4] = ["axis", "turned", "tilted", "sideways"];

/// 0 axis-aligned, 1 turned about +Y only, 2 tilted, 3 sideways (local X has a vertical part).
pub fn frame_class(shape: &Shape) -> u16 {
    let Shape::Oriented(o) = shape else {
        return 0;
    };
    if o.frame.is_axis_aligned() {
        return 0;
    }
    match o.frame {
        Frame::Id => 0,
        Frame::Table { tilt: 0, .. } => 1,
        Frame::Table { .. } => 2,
        Frame::Dir { d } if d[1] == 0 => 1,
        Frame::Dir { .. } => 3,
    }
}

pub fn prim_tag(p: &Primitive) -> u16 {
    u16::from(p.kind.0.min(31))
        | frame_class(&p.shape) << 5
        | u16::from(p.flags.contains(PrimFlags::NO_COLLIDE)) << 7
        | u16::from(p.flags.contains(PrimFlags::ENVELOPE)) << 10
        | u16::from(p.flags.contains(PrimFlags::INTERIOR)) << 11
}

pub fn level_bits(level: Level) -> u16 {
    match level {
        Level::L1 => 1,
        Level::L2 => 2,
        Level::L3 => 3,
        _ => 0,
    }
}

/// Marks every vertex of a finished chunk mesh with its level.
pub fn tag_level(vertices: &mut [Vertex], level: Level) {
    let bits = level_bits(level) << 8;
    for v in vertices {
        v.tag = (v.tag & !0x300) | bits;
    }
}

/// One line describing a primitive, for the pick key.
pub fn describe(p: &Primitive) -> String {
    let shape = match p.shape {
        Shape::Box { .. } => "box".to_string(),
        Shape::Ramp { .. } => "ramp".to_string(),
        Shape::Cylinder { .. } => "cylinder".to_string(),
        Shape::StairRun { steps, .. } => format!("stairs x{steps}"),
        Shape::LiftShaft { .. } => "lift shaft".to_string(),
        Shape::Oriented(o) => format!("oriented {:?} {:?}", o.frame, o.local),
    };
    let f = p.flags;
    format!(
        "kind {} material {} frame {} collides {} envelope {} interior {} shape {shape} aabb {:?}",
        p.kind.0,
        p.material.0,
        FRAME_NAMES[usize::from(frame_class(&p.shape))],
        !f.contains(PrimFlags::NO_COLLIDE),
        f.contains(PrimFlags::ENVELOPE),
        f.contains(PrimFlags::INTERIOR),
        p.shape.aabb(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionChunk, CollisionWorld};
    use crate::stream::ChunkId;
    use glam::{I64Vec3, Vec3};
    use worldgen::coord::WorldPos;
    use worldgen::orient::LocalShape;
    use worldgen::prim::{FeatureKind, MaterialId, QAabb, QVec3};

    fn prim(shape: Shape, kind: FeatureKind, flags: PrimFlags) -> Primitive {
        Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind,
            flags,
        }
    }

    #[test]
    fn tag_carries_kind_frame_and_flags() {
        let aabb = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(16, 16, 16));
        let tilted = Shape::oriented(
            Frame::table(3, 2),
            QVec3::new(0, 0, 0),
            LocalShape::Box { aabb },
            QAabb::new(QVec3::new(-99, -99, -99), QVec3::new(99, 99, 99)),
        );
        let t = prim_tag(&prim(tilted, FeatureKind::RAILING, PrimFlags::NO_COLLIDE));
        assert_eq!(t & 31, u16::from(FeatureKind::RAILING.0));
        assert_eq!((t >> 5) & 3, 2);
        assert_eq!((t >> 7) & 1, 1);
        let plain = prim_tag(&prim(
            Shape::Box { aabb },
            FeatureKind::DECK,
            PrimFlags::NONE,
        ));
        assert_eq!(plain >> 5, 0);
    }

    #[test]
    fn pick_finds_the_nearest_primitive_along_the_ray() {
        set_enabled(true);
        let b = |x0, x1| {
            prim(
                Shape::Box {
                    aabb: QAabb::new(QVec3::new(x0, 0, 0), QVec3::new(x1, 160, 160)),
                },
                FeatureKind::DECK,
                PrimFlags::NONE,
            )
        };
        let origin = WorldPos::new(I64Vec3::new(2, 0, 0), Vec3::ZERO).unwrap();
        let mut world = CollisionWorld::default();
        world.insert(
            ChunkId::l2(I64Vec3::new(4, 0, 0)),
            CollisionChunk::new(origin, 512.0, &[b(1600, 1760), b(800, 960)]),
        );
        let from = origin.offset(Vec3::new(10.0, 5.0, 5.0)).unwrap();
        let hit = world.pick(&from, Vec3::X, 300.0).unwrap();
        assert_eq!(hit.index, 1);
        assert!((hit.distance - 40.0).abs() < 0.05, "{}", hit.distance);
        assert!(world.pick(&from, Vec3::NEG_X, 300.0).is_none());
    }
}
