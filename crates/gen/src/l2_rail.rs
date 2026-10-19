//! Railings on walkable edges `[EDGE_GUARD]`. Spec: `docs/spec/levels.md` §5.4.
//!
//! An edge segment above a drop of more than the fall threshold is guarded with a railing
//! with `chance(0.85)`, or left unguarded on purpose. Geometry is in the L2 cell's local
//! quanta, read from the cell's own primitives plus its parent's window.

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::walk::{ColumnIndex, surfaces};

const M: Q = 16;
const CELL: Q = 512 * M;
/// Fall-death height, 15 m (`traversal.md` §3).
pub const FALL_THRESHOLD: Q = 15 * M;
/// Railing height 1.1 m and thickness 0.1 m, rounded to quanta.
const HEIGHT: Q = 18;
const THICKNESS: Q = 2;
/// Runs of drop cells are guarded or not together, up to this length.
const SEGMENT: Q = 64 * M;
/// Edges are probed in cells of this length, 2 m.
const PROBE_CELL: Q = 2 * M;
/// How far past the edge the drop is probed, and the headroom that must be free there.
const PROBE: Q = 8;
const GUARDED: f32 = 0.85;
/// Plan buckets of 64 m for the point queries.
const BUCKET_SHIFT: u32 = 10;

/// What the railing pass saw, for tuning and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeStats {
    /// Edge segments above a drop greater than the threshold.
    pub drops: u32,
    /// How many of those got a railing.
    pub guarded: u32,
}

/// One run of drop cells along a walkable edge and the guard decision it drew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeRun {
    /// Whether the run goes along x (its edge is at a fixed z).
    pub along_x: bool,
    /// Plan coordinate of the edge across the run.
    pub fixed: Q,
    /// Direction of the drop across the edge, -1 or 1.
    pub outward: Q,
    /// Height of the walkable top.
    pub y: Q,
    pub a: Q,
    pub b: Q,
    pub guarded: bool,
}

/// The railings for `prims` (the cell's window of L1 plus everything L2 added so far).
pub fn railings(
    seed: WorldSeed,
    cell: CellKey,
    prims: &[Primitive],
) -> (Vec<Primitive>, EdgeStats) {
    let (out, stats, _) = railings_with_runs(seed, cell, prims);
    (out, stats)
}

/// [`railings`] with every edge run and its decision, guarded or not.
pub fn railings_with_runs(
    seed: WorldSeed,
    cell: CellKey,
    prims: &[Primitive],
) -> (Vec<Primitive>, EdgeStats, Vec<EdgeRun>) {
    debug_assert_eq!(cell.level, Level::L2);
    let mut runs: Vec<EdgeRun> = Vec::new();
    let mut rng = Rng::new(hash(seed.0, Level::L2, cell.coords, salt::EDGE_GUARD));
    let mut out = Vec::new();
    let mut stats = EdgeStats::default();
    let index = ColumnIndex::new(prims, CELL, BUCKET_SHIFT);
    for plate in surfaces(prims)
        .into_iter()
        .filter(|p| p.rise.is_none() && !p.lift && p.kind != FeatureKind::MASS)
        // Tops of tall L1 pieces (core walls) belong to the cell they end in.
        .filter(|p| (0..CELL - HEIGHT).contains(&p.y0))
    {
        let (r, y) = (plate.rect, plate.y0);
        // -z, +z, -x, +x edges; each is `(fixed, lo, hi, along_x, outward)`.
        let edges = [
            (r.z0, r.x0, r.x1, true, -1),
            (r.z1, r.x0, r.x1, true, 1),
            (r.x0, r.z0, r.z1, false, -1),
            (r.x1, r.z0, r.z1, false, 1),
        ];
        for (fixed, lo, hi, along_x, outward) in edges {
            // An edge on the cell's face continues in the neighbour.
            if fixed <= 0 || fixed >= CELL {
                continue;
            }
            // Probe in short cells, then guard each run of drop cells, so a railing never covers
            // a stretch of edge that is flush with a walkway or doorway.
            let mut run: Option<(Q, Q)> = None;
            let mut a = lo;
            while a < hi {
                let b = (a + PROBE_CELL).min(hi);
                let mid = i32::midpoint(a, b);
                let beyond = fixed + outward * PROBE;
                let (px, pz) = if along_x {
                    (mid, beyond)
                } else {
                    (beyond, mid)
                };
                let wall = index.solid_in_column(px, pz, y, y + 29);
                let drop = y - index.solid_top_below(px, pz, y).unwrap_or(y - 100 * CELL);
                let is_drop = !wall && drop > FALL_THRESHOLD;
                run = match (run, is_drop) {
                    (Some((s, _)), true) if b - s <= SEGMENT => Some((s, b)),
                    (Some((s, e)), _) => {
                        emit(
                            &mut rng,
                            &mut stats,
                            (&mut out, &mut runs),
                            (along_x, fixed, outward, y),
                            (s, e),
                        );
                        is_drop.then_some((a, b))
                    }
                    (None, true) => Some((a, b)),
                    (None, false) => None,
                };
                a = b;
            }
            if let Some((s, e)) = run {
                emit(
                    &mut rng,
                    &mut stats,
                    (&mut out, &mut runs),
                    (along_x, fixed, outward, y),
                    (s, e),
                );
            }
        }
    }
    (out, stats, runs)
}

/// Draws the guard decision for one run of drop cells and adds its railing.
fn emit(
    rng: &mut Rng,
    stats: &mut EdgeStats,
    (out, runs): (&mut Vec<Primitive>, &mut Vec<EdgeRun>),
    (along_x, fixed, outward, y): (bool, Q, Q, Q),
    (a, b): (Q, Q),
) {
    stats.drops += 1;
    let guarded = rng.chance(GUARDED);
    runs.push(EdgeRun {
        along_x,
        fixed,
        outward,
        y,
        a,
        b,
        guarded,
    });
    if guarded {
        stats.guarded += 1;
        out.push(rail(along_x, fixed, outward, a, b, y));
    }
}

/// A railing along the inside of an edge at `fixed`, between `a` and `b`.
fn rail(along_x: bool, fixed: Q, outward: Q, a: Q, b: Q, y: Q) -> Primitive {
    let inner = if outward > 0 {
        (fixed - THICKNESS, fixed)
    } else {
        (fixed, fixed + THICKNESS)
    };
    let (min, max) = if along_x {
        (
            QVec3::new(a, y, inner.0),
            QVec3::new(b, y + HEIGHT, inner.1),
        )
    } else {
        (
            QVec3::new(inner.0, y, a),
            QVec3::new(inner.1, y + HEIGHT, b),
        )
    };
    Primitive {
        shape: Shape::Box {
            aabb: QAabb::new(min, max),
        },
        material: MaterialId::METAL,
        kind: FeatureKind::RAILING,
        flags: PrimFlags::NONE,
    }
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::l2::l2_cell_in;
    use crate::l2_port::L1Cache;
    use crate::walk::{cell_window, solid_top_below};

    /// P0.8.8: about 15 % of drop edges are left unguarded, and every railing stands on a
    /// flat edge above a real drop.
    #[test]
    #[ignore = "reads the v6 L1 (lift core, decks, bridges), replaced in version 7; rebuilt or deleted with PW.3"]
    fn unguarded_fraction_matches_spec() {
        let seed = WorldSeed(0x0808);
        let (mut drops, mut guarded, mut rails) = (0u32, 0u32, 0usize);
        for x in -12..12 {
            let parent_key = CellKey::new(Level::L1, I64Vec3::new(x, x % 3, 2 * x % 5));
            let cache = L1Cache::default();
            let parent = cache.get(seed, parent_key);
            for k in 0..512i64 {
                let idx = [k % 8, k / 64, k / 8 % 8];
                let cell = CellKey::new(
                    Level::L2,
                    parent_key.coords * 8 + I64Vec3::new(idx[0], idx[1], idx[2]),
                );
                let out = l2_cell_in(seed, &cache, cell);
                let origin = idx.map(|i| Q::try_from(i).unwrap() * CELL);
                let mut seen = cell_window(&parent.primitives, origin, CELL);
                seen.extend(
                    out.primitives
                        .iter()
                        .filter(|p| p.kind != FeatureKind::RAILING)
                        .copied(),
                );
                let (rs, stats) = railings(seed, cell, &seen);
                let emitted: Vec<&Primitive> = out
                    .primitives
                    .iter()
                    .filter(|p| p.kind == FeatureKind::RAILING)
                    .collect();
                assert_eq!(rs.len(), emitted.len());
                drops += stats.drops;
                guarded += stats.guarded;
                rails += rs.len();
                for p in &rs {
                    let bb = p.aabb();
                    assert_eq!(bb.max.y - bb.min.y, HEIGHT);
                    let (cx, cz) = (
                        i32::midpoint(bb.min.x, bb.max.x),
                        i32::midpoint(bb.min.z, bb.max.z),
                    );
                    // The railing stands on a solid top.
                    assert_eq!(solid_top_below(&seen, cx, cz, bb.min.y + 1), Some(bb.min.y));
                }
            }
        }
        assert_eq!(rails, usize::try_from(guarded).unwrap());
        assert!(drops > 300, "only {drops} drop edges");
        let unguarded = 1.0 - f64::from(guarded) / f64::from(drops);
        assert!((0.10..=0.20).contains(&unguarded), "{unguarded} of {drops}");
    }
}
