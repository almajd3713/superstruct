//! Collision data and queries. The solid world is the union of all collidable primitives
//! (`docs/spec/primitives.md` §3), so a query is "signed distance to the nearest solid".
//!
//! Everything here works in a small query frame: `f32` metres relative to a centre `WorldPos`.
//! Nothing large ever reaches `f32` (`CLAUDE.md` §3.6).

use std::collections::HashMap;

use glam::Vec3;
use worldgen::coord::WorldPos;
use worldgen::prim::{PrimFlags, Primitive, Shape};

use crate::stream::ChunkId;

/// A solid shape placed in the query frame: `offset` is its cell origin relative to the centre.
#[derive(Debug, Clone, Copy)]
pub struct Solid {
    pub shape: Shape,
    pub offset: Vec3,
    /// `Some(velocity)` for a piece of a moving platform (lift cabin, tram car), even while it
    /// is at rest; `None` for the static world. Metres per second in the query frame.
    pub platform: Option<Vec3>,
}

/// Signed distance to the nearest solid at `p` (query frame). `+inf` when there are none.
pub fn sdf(solids: &[Solid], p: Vec3) -> f32 {
    solids
        .iter()
        .map(|s| s.shape.sdf(p - s.offset))
        .fold(f32::INFINITY, f32::min)
}

/// Outward direction of steepest ascent of the field at `p`.
pub fn gradient(solids: &[Solid], p: Vec3) -> Vec3 {
    const E: f32 = 0.004;
    let d = |v: Vec3| sdf(solids, p + v);
    Vec3::new(
        d(Vec3::X * E) - d(Vec3::NEG_X * E),
        d(Vec3::Y * E) - d(Vec3::NEG_Y * E),
        d(Vec3::Z * E) - d(Vec3::NEG_Z * E),
    )
    .try_normalize()
    .unwrap_or(Vec3::Y)
}

/// Distance from `start` straight down to the first solid, if within `max`. Point trace, no radius.
pub fn ray_down(solids: &[Solid], start: Vec3, max: f32) -> Option<f32> {
    let mut t = 0.0;
    for _ in 0..64 {
        let d = sdf(solids, start - Vec3::Y * t);
        if d <= 0.002 {
            return Some(t);
        }
        t += d;
        if t > max {
            return None;
        }
    }
    None
}

/// Where the controller reads solids from.
pub trait SolidSource {
    /// Appends every solid within `half` metres (per axis) of `centre`, in the frame of `centre`.
    fn gather(&self, centre: &WorldPos, half: f32, out: &mut Vec<Solid>);

    /// Whether collision data exists around `pos`. The controller holds still until it does.
    fn ready(&self, pos: &WorldPos) -> bool;
}

/// Extra distance, metres, within which a dynamic solid still counts as near.
const NEAR_SLACK: f32 = 1.0;

struct Item {
    min: Vec3,
    max: Vec3,
    shape: Shape,
}

/// One resident chunk's collidable primitives, with their bounds precomputed.
pub struct CollisionChunk {
    origin: WorldPos,
    size_m: f32,
    items: Vec<Item>,
}

impl CollisionChunk {
    pub fn new(origin: WorldPos, size_m: f32, prims: &[Primitive]) -> Self {
        let items = prims
            .iter()
            .filter(|p| {
                !p.flags.contains(PrimFlags::NO_COLLIDE)
                    && !matches!(p.shape, Shape::LiftShaft { .. })
            })
            .map(|p| {
                let b = p.shape.aabb();
                Item {
                    min: b.min_m(),
                    max: b.max_m(),
                    shape: p.shape,
                }
            })
            .collect();
        Self {
            origin,
            size_m,
            items,
        }
    }
}

/// A moving solid owned by the game rather than by a chunk (a lift car, a tram): a shape with
/// its minimum corner at `origin`, moving at `velocity` m/s.
#[derive(Debug, Clone, Copy)]
pub struct DynamicSolid {
    pub shape: Shape,
    pub origin: WorldPos,
    pub velocity: Vec3,
}

#[derive(Default)]
pub struct CollisionWorld {
    chunks: HashMap<ChunkId, CollisionChunk>,
    dynamic: Vec<DynamicSolid>,
}

impl CollisionWorld {
    pub fn insert(&mut self, id: ChunkId, chunk: CollisionChunk) {
        self.chunks.insert(id, chunk);
    }

    pub fn remove(&mut self, id: &ChunkId) {
        self.chunks.remove(id);
    }

    pub fn contains(&self, id: &ChunkId) -> bool {
        self.chunks.contains_key(id)
    }

    /// Replaces the moving solids. Call once per tick, after moving them and before the
    /// controller steps, so they are where they will be at the end of that step.
    pub fn set_dynamic(&mut self, solids: Vec<DynamicSolid>) {
        self.dynamic = solids;
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

impl SolidSource for CollisionWorld {
    fn gather(&self, centre: &WorldPos, half: f32, out: &mut Vec<Solid>) {
        crate::zone!("collision gather");
        for d in &self.dynamic {
            let reach = d.shape.aabb().max_m().max_element() + half;
            let offset = d.origin.delta(centre).as_vec3();
            if offset.abs().max_element() <= reach + NEAR_SLACK {
                out.push(Solid {
                    shape: d.shape,
                    offset,
                    platform: Some(d.velocity),
                });
            }
        }
        for chunk in self.chunks.values() {
            // The centre in this chunk's cell-local space. Chunks far away are rejected before
            // any per-primitive work.
            let local = centre.delta(&chunk.origin).as_vec3();
            if local.cmplt(Vec3::splat(-half)).any()
                || local.cmpgt(Vec3::splat(chunk.size_m + half)).any()
            {
                continue;
            }
            let (lo, hi) = (local - Vec3::splat(half), local + Vec3::splat(half));
            let offset = -local;
            out.extend(
                chunk
                    .items
                    .iter()
                    .filter(|i| i.min.cmple(hi).all() && i.max.cmpge(lo).all())
                    .map(|i| Solid {
                        shape: i.shape,
                        offset,
                        platform: None,
                    }),
            );
        }
    }

    fn ready(&self, pos: &WorldPos) -> bool {
        use worldgen::cell::{CellKey, Level};
        let l2 = ChunkId::l2(CellKey::containing(Level::L2, pos).coords);
        let l1 = ChunkId::l1(
            CellKey::containing(Level::L1, pos).coords,
            crate::stream::Detail::Full,
        );
        self.chunks.contains_key(&l2) && self.chunks.contains_key(&l1)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use glam::I64Vec3;
    use worldgen::prim::{FeatureKind, MaterialId, QAabb, QVec3};

    pub(crate) fn floor_box(min: [i32; 3], max: [i32; 3]) -> Primitive {
        Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(
                    QVec3::new(min[0], min[1], min[2]),
                    QVec3::new(max[0], max[1], max[2]),
                ),
            },
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        }
    }

    #[test]
    fn gather_returns_nearby_solids_in_the_centre_frame() {
        let origin = WorldPos::new(I64Vec3::new(2, 0, 0), Vec3::ZERO).unwrap();
        let chunk = CollisionChunk::new(
            origin,
            512.0,
            &[
                floor_box([0, 0, 0], [1600, 16, 1600]),
                floor_box([4000, 0, 4000], [4100, 100, 4100]),
            ],
        );
        let mut world = CollisionWorld::default();
        world.insert(ChunkId::l2(I64Vec3::new(4, 0, 0)), chunk);
        let centre = origin.offset(Vec3::new(10.0, 1.0, 10.0)).unwrap();
        let mut out = Vec::new();
        world.gather(&centre, 3.0, &mut out);
        assert_eq!(out.len(), 1);
        // The floor's top (y = 1 m) is 0 m above the centre's feet at y = 1: sdf at centre is 0.
        assert!(sdf(&out, Vec3::ZERO).abs() < 1e-5);
        assert!((sdf(&out, Vec3::new(0.0, 2.0, 0.0)) - 2.0).abs() < 1e-5);
        // A centre well away from the chunk gets nothing.
        let mut none = Vec::new();
        world.gather(
            &origin.offset(Vec3::new(900.0, 0.0, 0.0)).unwrap(),
            3.0,
            &mut none,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn no_collide_and_lift_shafts_are_skipped() {
        let mut p = floor_box([0, 0, 0], [16, 16, 16]);
        p.flags = PrimFlags::NO_COLLIDE;
        let shaft = Primitive {
            shape: Shape::LiftShaft {
                aabb: QAabb::new(QVec3::new(0, 0, 0), QVec3::new(16, 64, 16)),
            },
            ..p
        };
        let c = CollisionChunk::new(WorldPos::ORIGIN, 512.0, &[p, shaft]);
        assert!(c.items.is_empty());
    }

    #[test]
    fn dynamic_solids_are_gathered_with_their_platform_velocity() {
        let mut world = CollisionWorld::default();
        let origin = WorldPos::new(I64Vec3::new(1, 0, 0), Vec3::new(10.0, 20.0, 10.0)).unwrap();
        let slab = floor_box([0, 0, 0], [160, 8, 160]).shape;
        world.set_dynamic(vec![DynamicSolid {
            shape: slab,
            origin,
            velocity: Vec3::new(0.0, 3.0, 0.0),
        }]);
        let centre = origin.offset(Vec3::new(5.0, 0.5, 5.0)).unwrap();
        let mut out = Vec::new();
        world.gather(&centre, 3.0, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].platform, Some(Vec3::new(0.0, 3.0, 0.0)));
        // The slab's top (0.5 m thick) is 0.5 m below... the slab spans y 0..0.5 above its
        // origin, and the centre is at y = 0.5, so the top surface passes through the centre.
        assert!(sdf(&out, Vec3::ZERO).abs() < 1e-5);
        // Far from the slab, nothing.
        let mut none = Vec::new();
        world.gather(
            &origin.offset(Vec3::new(200.0, 0.0, 0.0)).unwrap(),
            3.0,
            &mut none,
        );
        assert!(none.is_empty());
        world.set_dynamic(Vec::new());
        out.clear();
        world.gather(&centre, 3.0, &mut out);
        assert!(out.is_empty());
    }
}
