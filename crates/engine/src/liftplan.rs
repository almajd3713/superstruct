//! Lift shaft plans (`docs/spec/lifts.md` §2.2): what a core looks like from outside, as pure
//! functions of the generator. They generate a cell, so callers run them off the main thread.

use worldgen::cavity::is_cavity;
use worldgen::cell::CellKey;
use worldgen::key::LayerKey;
use worldgen::l1::{DECK_REGION_LOG2, Rect, core_footprint, core_stops, decks, l1_cell};
use worldgen::prim::{FeatureKind, Q, Shape};
use worldgen::seed::WorldSeed;

/// Width of the band just outside a core side that is searched for floor, in quanta (0.5 m).
const BAND: Q = 8;

/// Sides in the order -x, +x, -z, +z.
pub const SIDES: usize = 4;

/// One stop of a core in one L1 cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPlan {
    /// Cell-local stop height, quanta.
    pub y: Q,
    /// Per side, the open intervals along that side (z for the x sides, x for the z sides),
    /// quanta, where a deck or bridge floor at this height lies just outside.
    pub served: [Vec<(Q, Q)>; SIDES],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellPlan {
    OutOfDomain,
    /// The core is absent in giant-cavity cells.
    Cavity,
    Core {
        footprint: Rect,
        stops: Vec<StopPlan>,
    },
}

/// The core of `cell` and its stops. The stops come from the same function and inputs
/// `l1_cell` uses to cut the wall openings, so they match the openings exactly.
pub fn plan_cell(seed: WorldSeed, cell: CellKey) -> CellPlan {
    if !cell.in_domain() {
        return CellPlan::OutOfDomain;
    }
    if is_cavity(seed, cell) {
        return CellPlan::Cavity;
    }
    let out = l1_cell(seed, cell);
    let footprint = core_footprint(seed, worldgen::key::ColumnKey::of(cell));
    let layer = decks(seed, LayerKey::of(cell, DECK_REGION_LOG2));
    let bridges: Vec<_> = out
        .primitives
        .iter()
        .filter(|p| p.kind == FeatureKind::BRIDGE)
        .copied()
        .collect();
    let floors: Vec<_> = out
        .primitives
        .iter()
        .filter(|p| matches!(p.kind, FeatureKind::DECK | FeatureKind::BRIDGE))
        .filter_map(|p| match p.shape {
            Shape::Box { aabb } => Some(aabb),
            _ => None,
        })
        .collect();
    let stops = core_stops(&layer, &bridges)
        .into_iter()
        .map(|y| StopPlan {
            y,
            served: served_sides(&footprint, y, &floors),
        })
        .collect();
    CellPlan::Core { footprint, stops }
}

fn served_sides(f: &Rect, y: Q, floors: &[worldgen::prim::QAabb]) -> [Vec<(Q, Q)>; SIDES] {
    // (band along the cross axis, the side's extent along its own axis, which axis is "across")
    let bands = [
        (f.x0 - BAND, f.x0, f.z0, f.z1, true),
        (f.x1, f.x1 + BAND, f.z0, f.z1, true),
        (f.z0 - BAND, f.z0, f.x0, f.x1, false),
        (f.z1, f.z1 + BAND, f.x0, f.x1, false),
    ];
    bands.map(|(lo, hi, a0, a1, x_across)| {
        let mut spans: Vec<(Q, Q)> = floors
            .iter()
            .filter(|b| b.max.y == y)
            .filter_map(|b| {
                let (across, along) = if x_across {
                    ((b.min.x, b.max.x), (b.min.z, b.max.z))
                } else {
                    ((b.min.z, b.max.z), (b.min.x, b.max.x))
                };
                let touches = across.0 < hi && across.1 > lo;
                let (s, e) = (along.0.max(a0), along.1.min(a1));
                (touches && s < e).then_some((s, e))
            })
            .collect();
        spans.sort_unstable();
        let mut merged: Vec<(Q, Q)> = Vec::new();
        for (s, e) in spans {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        merged
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;
    use worldgen::cell::Level;
    use worldgen::l1_port::PORT_CLEARANCE;

    const SEED: WorldSeed = WorldSeed(0);

    fn cell(x: i64, y: i64, z: i64) -> CellKey {
        CellKey::new(Level::L1, I64Vec3::new(x, y, z))
    }

    /// First cell in the fixed search order that satisfies `want`. A `GEN_VERSION` change moves
    /// the cell instead of breaking the test.
    pub(crate) fn find(want: impl Fn(CellKey) -> bool) -> CellKey {
        for y in -4..=4 {
            for z in 0..8 {
                for x in 0..8 {
                    let c = cell(x, y, z);
                    if want(c) {
                        eprintln!("search found {:?}", c.coords);
                        return c;
                    }
                }
            }
        }
        panic!("no cell in the search range matched");
    }

    fn with_two_decks(c: CellKey) -> bool {
        !is_cavity(SEED, c) && decks(SEED, LayerKey::of(c, DECK_REGION_LOG2)).len() >= 2
    }

    fn core(c: CellKey) -> (Rect, Vec<StopPlan>) {
        match plan_cell(SEED, c) {
            CellPlan::Core { footprint, stops } => (footprint, stops),
            other => panic!("expected a core, got {other:?}"),
        }
    }

    #[test]
    fn footprint_and_stops_match_the_generator() {
        let c = find(with_two_decks);
        let (footprint, stops) = core(c);
        assert_eq!(
            footprint,
            core_footprint(SEED, worldgen::key::ColumnKey::of(c))
        );
        assert!(stops.windows(2).all(|w| w[0].y < w[1].y), "ascending");
        for d in decks(SEED, LayerKey::of(c, DECK_REGION_LOG2)) {
            assert!(stops.iter().any(|s| s.y == d.top), "deck top {}", d.top);
        }
    }

    #[test]
    fn stops_match_the_openings_in_the_core_walls() {
        let c = find(with_two_decks);
        let (f, stops) = core(c);
        let walls: Vec<_> = l1_cell(SEED, c)
            .primitives
            .into_iter()
            .filter(|p| p.kind == FeatureKind::SHAFT_WALL)
            .filter_map(|p| match p.shape {
                Shape::Box { aabb } => Some(aabb),
                _ => None,
            })
            .collect();
        assert!(!walls.is_empty());
        let in_opening = |y: Q| stops.iter().any(|s| s.y <= y && y < s.y + PORT_CLEARANCE);
        for s in &stops {
            // Nothing blocks the opening.
            let blocked = walls
                .iter()
                .any(|w| w.min.y < s.y + PORT_CLEARANCE && w.max.y > s.y);
            let covered_by_earlier = stops
                .iter()
                .any(|o| o.y < s.y && s.y < o.y + PORT_CLEARANCE);
            if !covered_by_earlier {
                assert!(!blocked, "wall in the opening at {}", s.y);
                // Four walls end at the stop, one per side, unless the cell starts here.
                let ending = walls.iter().filter(|w| w.max.y == s.y).count();
                assert!(ending == 4 || s.y == 0, "{ending} walls end at {}", s.y);
            }
        }
        // Everywhere else the four sides are walled (sampled every 64 m).
        for y in (0..65_536).step_by(1024) {
            if in_opening(y) {
                continue;
            }
            let at = walls.iter().filter(|w| w.min.y <= y && y < w.max.y).count();
            assert_eq!(at, 4, "wall count at height {y}");
        }
        assert!(f.x1 > f.x0 && f.z1 > f.z0);
    }

    #[test]
    fn deck_stops_are_served_on_every_side_over_the_full_length() {
        let c = find(with_two_decks);
        let (f, stops) = core(c);
        let layer = decks(SEED, LayerKey::of(c, DECK_REGION_LOG2));
        for s in stops.iter().filter(|s| layer.iter().any(|d| d.top == s.y)) {
            assert_eq!(s.served[0], vec![(f.z0, f.z1)]);
            assert_eq!(s.served[1], vec![(f.z0, f.z1)]);
            assert_eq!(s.served[2], vec![(f.x0, f.x1)]);
            assert_eq!(s.served[3], vec![(f.x0, f.x1)]);
        }
    }

    #[test]
    fn bridge_stops_serve_a_single_side_interval_of_bridge_width() {
        // A cell with a bridge stop that no deck shares.
        let c = find(|c| {
            !is_cavity(SEED, c)
                && match plan_cell(SEED, c) {
                    CellPlan::Core { stops, .. } => {
                        let layer = decks(SEED, LayerKey::of(c, DECK_REGION_LOG2));
                        stops.iter().any(|s| !layer.iter().any(|d| d.top == s.y))
                    }
                    _ => false,
                }
        });
        let (_, stops) = core(c);
        let layer = decks(SEED, LayerKey::of(c, DECK_REGION_LOG2));
        let bridge = stops
            .iter()
            .find(|s| !layer.iter().any(|d| d.top == s.y))
            .unwrap();
        let sides: Vec<_> = bridge.served.iter().filter(|v| !v.is_empty()).collect();
        assert!(!sides.is_empty());
        for intervals in sides {
            for &(a, b) in intervals {
                assert!(b - a >= 8 * 16, "interval of {} Q", b - a);
            }
        }
    }

    #[test]
    fn plans_are_the_same_every_time_and_on_every_thread() {
        let c = find(with_two_decks);
        let a = plan_cell(SEED, c);
        let b = std::thread::spawn(move || plan_cell(SEED, c))
            .join()
            .unwrap();
        assert_eq!(a, plan_cell(SEED, c));
        assert_eq!(a, b);
    }

    #[test]
    fn cavity_cells_have_no_core() {
        let mut found = None;
        'search: for z in 0..40 {
            for y in -4..4 {
                for x in 0..40 {
                    let c = cell(x, y, z);
                    if is_cavity(SEED, c) {
                        found = Some(c);
                        break 'search;
                    }
                }
            }
        }
        let c = found.expect("a cavity cell within the search range");
        assert_eq!(plan_cell(SEED, c), CellPlan::Cavity);
        let outside = CellKey::new(Level::L1, I64Vec3::new(i64::MAX / 2, 0, 0));
        assert_eq!(plan_cell(SEED, outside), CellPlan::OutOfDomain);
    }
}
