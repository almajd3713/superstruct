//! The L1 context query that L2 reads (`docs/spec/levels.md` §5, P0.8.1): what a point in an
//! L1 cell is, judged from the cell's own primitives.

use glam::Vec3;

use crate::cell::CellKey;
use crate::l1::l1_cell;
use crate::prim::{FeatureKind, PrimFlags, Primitive, QAabb, Shape};
use crate::seed::WorldSeed;

/// How far above a deck or bridge top, in metres, a point still counts as standing on it.
pub const SURFACE_DEPTH: f32 = 0.5;

/// What a point of an L1 cell is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Volume {
    /// Open air (including shafts, chasms, cavities and the lift core's interior).
    Void,
    /// Inside a collidable L1 solid other than a trunk: deck, bridge, core wall.
    Structure,
    /// Open air within `SURFACE_DEPTH` above the top of a deck or bridge piece.
    DeckSurface,
    /// Inside a trunk.
    TrunkInterior,
}

/// The solids of one L1 cell, ready for point queries.
#[derive(Debug, Clone)]
pub struct L1Context {
    trunks: Vec<Primitive>,
    solids: Vec<Primitive>,
    /// Deck and bridge boxes, whose tops are walkable.
    floors: Vec<QAabb>,
}

impl L1Context {
    pub fn new(seed: WorldSeed, cell: CellKey) -> Self {
        Self::from_primitives(l1_cell(seed, cell).primitives)
    }

    pub fn from_primitives(primitives: Vec<Primitive>) -> Self {
        let mut ctx = Self {
            trunks: Vec::new(),
            solids: Vec::new(),
            floors: Vec::new(),
        };
        for p in primitives {
            if matches!(p.shape, Shape::LiftShaft { .. }) || p.flags.contains(PrimFlags::NO_COLLIDE)
            {
                continue;
            }
            if p.kind == FeatureKind::TRUNK {
                ctx.trunks.push(p);
                continue;
            }
            if matches!(p.kind, FeatureKind::DECK | FeatureKind::BRIDGE) {
                ctx.floors.push(p.aabb());
            }
            ctx.solids.push(p);
        }
        ctx
    }

    /// Classifies `p`, in metres in the cell's local space. Trunk interior wins over other
    /// solids, and any solid wins over a surface.
    pub fn classify(&self, p: Vec3) -> Volume {
        if self.trunks.iter().any(|t| t.shape.contains(p)) {
            return Volume::TrunkInterior;
        }
        if self.solids.iter().any(|s| s.shape.contains(p)) {
            return Volume::Structure;
        }
        let on_floor = self.floors.iter().any(|bb| {
            let (lo, hi) = (bb.min_m(), bb.max_m());
            p.x >= lo.x
                && p.x < hi.x
                && p.z >= lo.z
                && p.z < hi.z
                && (hi.y..hi.y + SURFACE_DEPTH).contains(&p.y)
        });
        if on_floor {
            Volume::DeckSurface
        } else {
            Volume::Void
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::cell::Level;
    use crate::rng::Rng;

    /// P0.8.1: the query agrees with a brute-force reading of the primitives through the SDF,
    /// at points on, inside and just above each primitive.
    #[test]
    fn agrees_with_primitives() {
        let seed = WorldSeed(0x0801);
        let mut rng = Rng::new(7);
        let (mut points, mut seen) = (0, [0u32; 4]);
        for x in -60..60 {
            for y in [-4, 0, 3] {
                for z in [-6, 1] {
                    let cell = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                    let prims = l1_cell(seed, cell).primitives;
                    if prims.is_empty() {
                        continue;
                    }
                    let ctx = L1Context::from_primitives(prims.clone());
                    for _ in 0..60 {
                        let bb =
                            prims[usize::try_from(rng.below(prims.len() as u64)).unwrap()].aabb();
                        let (lo, hi) = (bb.min_m(), bb.max_m());
                        let k = rng.range_i32(-6, 10) as f32;
                        let edge = if rng.chance(0.5) { hi.y } else { lo.y };
                        let on_edge = rng.chance(0.5);
                        let axis = |r: &mut Rng, lo: f32, hi: f32| lo + (hi - lo) * r.next_f32();
                        // Offsets of half a quantum-step keep the point off every face.
                        let (px, py, pz) = (
                            axis(&mut rng, lo.x, hi.x),
                            axis(&mut rng, lo.y, hi.y),
                            axis(&mut rng, lo.z, hi.z),
                        );
                        let p =
                            Vec3::new(px, if on_edge { edge + (k + 0.5) / 32.0 } else { py }, pz);
                        // Skip points on a face, where half-open `contains` and `sdf` differ.
                        if prims.iter().any(|q| q.shape.sdf(p).abs() < 1e-3) {
                            continue;
                        }
                        let want = oracle(&prims, p);
                        assert_eq!(ctx.classify(p), want, "{cell:?} at {p:?}");
                        points += 1;
                        seen[want as usize] += 1;
                    }
                }
            }
        }
        assert!(points >= 10_000, "{points}");
        assert!(seen.iter().all(|&n| n > 100), "{seen:?}");
    }

    fn oracle(prims: &[Primitive], p: Vec3) -> Volume {
        let inside = |q: &&Primitive| {
            !matches!(q.shape, Shape::LiftShaft { .. })
                && !q.flags.contains(PrimFlags::NO_COLLIDE)
                && q.shape.sdf(p) < 0.0
        };
        if prims
            .iter()
            .filter(inside)
            .any(|q| q.kind == FeatureKind::TRUNK)
        {
            return Volume::TrunkInterior;
        }
        if prims.iter().any(|q| inside(&q)) {
            return Volume::Structure;
        }
        let above = prims.iter().any(|q| {
            let bb = q.aabb();
            let (lo, hi) = (bb.min_m(), bb.max_m());
            matches!(q.kind, FeatureKind::DECK | FeatureKind::BRIDGE)
                && (lo.x..hi.x).contains(&p.x)
                && (lo.z..hi.z).contains(&p.z)
                && p.y > hi.y
                && p.y - hi.y < SURFACE_DEPTH
        });
        if above {
            Volume::DeckSurface
        } else {
            Volume::Void
        }
    }
}
