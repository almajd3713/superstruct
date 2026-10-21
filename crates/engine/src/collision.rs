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

/// A ladder's climbing corridor in a query frame (`docs/spec/traversal.md`): an oriented box
/// whose frame is upright, and the side its rungs are on (the way a climber faces).
#[derive(Debug, Clone, Copy)]
pub struct LadderVol {
    pub centre: Vec3,
    /// The box's unit axes, with `axes[1]` plumb.
    pub axes: [Vec3; 3],
    pub half: Vec3,
    /// Horizontal unit vector from the open side to the rungs.
    pub toward: Vec3,
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

    /// Appends every ladder corridor within `half` metres of `centre`, in the frame of `centre`.
    fn ladders(&self, _centre: &WorldPos, _half: f32, _out: &mut Vec<LadderVol>) {}
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
    /// Every drawn primitive, kept for the pick key; empty unless debug views are on.
    picks: Vec<Primitive>,
    /// The ladder corridors of the chunk, in chunk-local metres.
    ladders: Vec<LadderVol>,
}

/// What the pick key found.
#[derive(Debug, Clone, Copy)]
pub struct PickHit {
    pub chunk: ChunkId,
    pub index: usize,
    pub prim: Primitive,
    pub distance: f32,
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
        let picks = if crate::debugview::enabled() {
            prims
                .iter()
                .filter(|p| !matches!(p.shape, Shape::LiftShaft { .. }))
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        let ladders = prims.iter().filter_map(ladder_of).collect();
        Self {
            origin,
            size_m,
            items,
            picks,
            ladders,
        }
    }
}

/// The corridor of a `FeatureKind::LADDER` box, in the cell's own metres.
fn ladder_of(p: &Primitive) -> Option<LadderVol> {
    if p.kind != worldgen::prim::FeatureKind::LADDER {
        return None;
    }
    let (frame, origin, local) = match p.shape {
        Shape::Box { aabb } => (
            worldgen::orient::Frame::Id,
            worldgen::prim::QVec3::default(),
            aabb,
        ),
        Shape::Oriented(o) => match o.local {
            worldgen::orient::LocalShape::Box { aabb } => (o.frame, o.origin, aabb),
            _ => return None,
        },
        _ => return None,
    };
    let v = |d: glam::DVec3| Vec3::new(d.x as f32, d.y as f32, d.z as f32);
    let [ex, ey, ez] = frame.axes();
    let axes = [v(ex), v(ey), v(ez)];
    if axes[1].y < 0.999 {
        return None;
    }
    let (lo, hi) = (local.min_m(), local.max_m());
    let mid = (lo + hi) * 0.5;
    let centre = origin.to_metres() + axes[0] * mid.x + axes[1] * mid.y + axes[2] * mid.z;
    let toward = match p.material.0 & 3 {
        0 => -axes[2],
        1 => axes[0],
        2 => axes[2],
        _ => -axes[0],
    };
    Some(LadderVol {
        centre,
        axes,
        half: (hi - lo) * 0.5,
        toward,
    })
}

/// A solid owned by the game rather than by a chunk (a lift car, a gate, a tram): a shape with
/// its minimum corner at `origin`. `platform` is `Some(velocity)` for a moving platform.
#[derive(Debug, Clone, Copy)]
pub struct DynamicSolid {
    pub shape: Shape,
    pub origin: WorldPos,
    pub platform: Option<Vec3>,
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

impl CollisionWorld {
    /// The first primitive (collidable or not) along a ray, by sphere tracing each chunk the
    /// ray crosses. Needs `debugview::enabled()` when the chunks were inserted.
    pub fn pick(&self, from: &WorldPos, dir: Vec3, max: f32) -> Option<PickHit> {
        let mut best: Option<PickHit> = None;
        for (id, chunk) in &self.chunks {
            let start = from.delta(&chunk.origin).as_vec3();
            let inv = dir.recip();
            let t0 = (Vec3::ZERO - start) * inv;
            let t1 = (Vec3::splat(chunk.size_m) - start) * inv;
            let enter = t0.min(t1).max_element().max(0.0);
            let exit = t0.max(t1).min_element().min(max);
            if enter > exit {
                continue;
            }
            let mut t = enter;
            for _ in 0..160 {
                if t > exit || best.is_some_and(|b| b.distance < t) {
                    break;
                }
                let p = start + dir * t;
                let mut d = f32::INFINITY;
                let mut at = 0;
                for (i, prim) in chunk.picks.iter().enumerate() {
                    let di = prim.shape.sdf(p);
                    if di < d {
                        d = di;
                        at = i;
                    }
                }
                if d <= 0.01 {
                    if best.is_none_or(|b| b.distance > t) {
                        best = Some(PickHit {
                            chunk: *id,
                            index: at,
                            prim: chunk.picks[at],
                            distance: t,
                        });
                    }
                    break;
                }
                t += d.max(0.005);
            }
        }
        best
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
                    platform: d.platform,
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

    fn ladders(&self, centre: &WorldPos, half: f32, out: &mut Vec<LadderVol>) {
        for chunk in self.chunks.values() {
            if chunk.ladders.is_empty() {
                continue;
            }
            let local = centre.delta(&chunk.origin).as_vec3();
            if local.cmplt(Vec3::splat(-half)).any()
                || local.cmpgt(Vec3::splat(chunk.size_m + half)).any()
            {
                continue;
            }
            out.extend(chunk.ladders.iter().map(|l| LadderVol {
                centre: l.centre - local,
                ..*l
            }));
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
            platform: Some(Vec3::new(0.0, 3.0, 0.0)),
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

    /// PW.5.1: the ladder wells of generated interiors carry a climbing corridor that the
    /// collision chunk finds, upright and facing the rungs.
    #[test]
    fn generated_ladder_wells_give_climbing_corridors() {
        use worldgen::cell::{CellKey, Level};
        use worldgen::l2_port::L1Cache;
        let seed = worldgen::seed::WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let mut found = 0;
        'cells: for k in 0..60i64 {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            let l2 = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let mut ctx = worldgen::l3_web::web_context(seed, &cache, l2);
            for j in 0..512i64 {
                let w = CellKey::new(
                    Level::L3,
                    l2.coords * 8 + I64Vec3::new(j % 8, j / 8 % 8, j / 64),
                );
                if !ctx.is_active(w) {
                    continue;
                }
                let prims = ctx.cell_primitives(seed, w);
                if prims
                    .iter()
                    .all(|p| p.kind != worldgen::prim::FeatureKind::LADDER)
                {
                    continue;
                }
                let origin = WorldPos::new(I64Vec3::ZERO, Vec3::ZERO).unwrap();
                let chunk = CollisionChunk::new(origin, 64.0, &prims);
                let upright = prims
                    .iter()
                    .filter(|p| p.kind == worldgen::prim::FeatureKind::LADDER)
                    .filter(|p| match p.shape {
                        Shape::Oriented(o) => worldgen::plumb::is_upright(o.frame),
                        _ => true,
                    })
                    .count();
                assert_eq!(
                    chunk.ladders.len(),
                    upright,
                    "every upright corridor is found"
                );
                if upright == 0 {
                    continue;
                }
                for l in &chunk.ladders {
                    assert!(l.axes[1].y > 0.999 && l.toward.y.abs() < 1e-6);
                    assert!((l.toward.length() - 1.0).abs() < 1e-4);
                    assert!(l.half.y > 1.5, "tall: {:?}", l.half);
                }
                found += 1;
                if found >= 3 {
                    break 'cells;
                }
            }
        }
        assert!(found >= 1, "no generated ladder well in 60 cells");
    }
}
