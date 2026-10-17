//! Sanctuary selection `[SANCTUARY]`. Spec: `docs/spec/levels.md` §4.7.
//!
//! Each L1 cell may hold one sanctuary zone, placed on a deck top inside one L2 child.

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l2_mass::Parent;
use crate::prim::{CellOutput, FeatureKind, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// L2 child edge in quanta.
const CHILD: Q = 512 * M;
/// Zone footprint and height (§5.5).
const SIDE: Q = 32 * M;
const HEIGHT: Q = 16 * M;
/// Keeps the zone clear of the 64 m corridors along the child's faces, plus 8 m.
const RIM: Q = 72 * M;
const TRIES: u32 = 16;
const PRESENCE: f32 = 0.5;

/// The sanctuary zone of L1 cell `cell`, in the cell's local quanta, given its output without
/// the zone. `None` if the draw says no, no child has a deck top, or no spot fits in the child.
pub fn sanctuary(seed: WorldSeed, cell: CellKey, out: &CellOutput) -> Option<QAabb> {
    debug_assert_eq!(cell.level, Level::L1);
    let mut rng = Rng::new(hash(seed.0, Level::L1, cell.coords, salt::SANCTUARY));
    let present = rng.chance(PRESENCE);
    let decks: Vec<QAabb> = out
        .primitives
        .iter()
        .filter(|p| p.kind == FeatureKind::DECK)
        .filter_map(|p| match p.shape {
            Shape::Box { aabb } => Some(aabb),
            _ => None,
        })
        .collect();
    // Children that hold a deck top surface, in x, then y, then z order.
    let mut children: Vec<([Q; 3], Q)> = Vec::new();
    for cx in 0..8 {
        for cy in 0..8 {
            for cz in 0..8 {
                let (x0, y0, z0) = (cx * CHILD, cy * CHILD, cz * CHILD);
                let top = decks.iter().find(|d| {
                    (y0..y0 + CHILD).contains(&d.max.y)
                        && d.min.x < x0 + CHILD
                        && x0 < d.max.x
                        && d.min.z < z0 + CHILD
                        && z0 < d.max.z
                });
                if let Some(d) = top {
                    children.push(([x0, y0, z0], d.max.y));
                }
            }
        }
    }
    let pick = rng.below(children.len().max(1) as u64);
    let steps = (CHILD - 2 * RIM - SIDE) / (8 * M) + 1;
    let tries: Vec<(Q, Q)> = (0..TRIES)
        .map(|_| (rng.range_i32(0, steps), rng.range_i32(0, steps)))
        .collect();
    if !present {
        return None;
    }
    let &([x0, y0, z0], top) = children.get(usize::try_from(pick).ok()?)?;
    let ctx = Parent::new(out, [0, 0, 0]);
    tries.into_iter().find_map(|(ix, iy)| {
        let (x, z) = (x0 + RIM + 8 * M * ix, z0 + RIM + 8 * M * iy);
        let zone = QAabb::new(
            QVec3::new(x, top, z),
            QVec3::new(x + SIDE, top + HEIGHT, z + SIDE),
        );
        (zone.max.y <= y0 + CHILD - 8 * M && ctx.fits(&zone)).then_some(zone)
    })
}

#[cfg(test)]
mod tests {
    use glam::{I64Vec3, Vec3};

    use super::*;
    use crate::cavity::is_cavity;
    use crate::l1::l1_cell;
    use crate::l1_query::{L1Context, Volume};
    use crate::prim::{ChildConstraint, MetaFlags, ZoneKind};

    fn zone_of(out: &CellOutput) -> Option<QAabb> {
        out.child_constraints.iter().find_map(|c| match c {
            ChildConstraint::Zone {
                aabb,
                kind: ZoneKind::Sanctuary,
            } => Some(*aabb),
            _ => None,
        })
    }

    /// P0.8.6: sanctuary frequency and spacing over 10⁴ L1 cells, and each zone's placement.
    #[test]
    fn frequency_and_spacing() {
        let seed = WorldSeed(0x0806);
        let (mut cells, mut with_deck, mut zones) = (0u32, 0u32, 0u32);
        let mut gaps: Vec<u32> = Vec::new();
        for row in 0..100i64 {
            let mut since = 0;
            for x in 0..100i64 {
                let cell = CellKey::new(Level::L1, I64Vec3::new(x - 50, row % 7 - 3, row));
                if is_cavity(seed, cell) {
                    continue;
                }
                cells += 1;
                let out = l1_cell(seed, cell);
                with_deck += u32::from(out.primitives.iter().any(|p| p.kind == FeatureKind::DECK));
                since += 1;
                if let Some(zone) = zone_of(&out) {
                    zones += 1;
                    gaps.push(since);
                    since = 0;
                    assert!(out.meta.flags.contains(MetaFlags::SANCTUARY));
                    assert_eq!(zone.max.x - zone.min.x, SIDE);
                    assert_eq!(zone.max.y - zone.min.y, HEIGHT);
                    // The zone is open air standing on deck, inside one child.
                    let ctx = L1Context::from_primitives(out.primitives.clone());
                    let (lo, hi) = (zone.min.to_metres(), zone.max.to_metres());
                    for (u, v, w) in [(0.02, 0.5, 0.02), (0.5, 0.5, 0.5), (0.98, 0.98, 0.98)] {
                        let p = lo + (hi - lo) * Vec3::new(u, v, w);
                        assert!(matches!(
                            ctx.classify(p),
                            Volume::Void | Volume::DeckSurface
                        ));
                        let under = Vec3::new(p.x, lo.y - 0.25, p.z);
                        assert_eq!(ctx.classify(under), Volume::Structure);
                    }
                    assert_eq!(zone.min.x / CHILD, (zone.max.x - 1) / CHILD);
                    assert_eq!(zone.min.z / CHILD, (zone.max.z - 1) / CHILD);
                    assert_eq!(zone.min.y / CHILD, (zone.max.y - 1) / CHILD);
                } else {
                    assert!(!out.meta.flags.contains(MetaFlags::SANCTUARY));
                }
            }
        }
        // Half of the cells draw a sanctuary; those without a deck or a fitting spot lose it.
        let rate = f64::from(zones) / f64::from(with_deck);
        assert!(
            (0.3..=0.5).contains(&rate),
            "{zones} zones in {with_deck} deck cells ({cells} cells)"
        );
        let mean =
            f64::from(gaps.iter().sum::<u32>()) / f64::from(u32::try_from(gaps.len()).unwrap());
        assert!((1.8..=3.5).contains(&mean), "mean gap {mean} cells");
    }
}
