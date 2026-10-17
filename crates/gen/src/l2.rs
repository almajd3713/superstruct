//! L2 districts (512 m). Spec: `docs/spec/levels.md` §5.
//!
//! So far: face ports, the skywalk halves at walkway ports, lift shafts and building masses.

use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::key::LayerKey;
use crate::l1::{DECK_REGION_LOG2, decks};
use crate::l2_attach::attached_masses;
use crate::l2_column::{lift_column, span_of};
use crate::l2_mass::masses_in;
use crate::l2_port::{L1Cache, cell_ports_in};
use crate::l2_rail::railings;
use crate::l2_vista::vista;
use crate::l2_walk::Skywalk;
use crate::port::PortKind;
use crate::prim::{
    CellMeta, CellOutput, ChildConstraint, MetaFlags, Primitive, Q, QAabb, ZoneKind,
};
use crate::seed::WorldSeed;
use crate::walk::cell_window;

const M: Q = 16;
/// L2 cell edge in quanta.
pub const CELL: Q = 512 * M;

/// Everything L2 emits for `cell`.
pub fn l2_cell(seed: WorldSeed, cell: CellKey) -> CellOutput {
    l2_cell_in(seed, &L1Cache::default(), cell)
}

/// `l2_cell` reading L1 cells through `cache`.
pub fn l2_cell_in(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> CellOutput {
    debug_assert_eq!(cell.level, Level::L2);
    let c = cell.coords;
    let parent_key = CellKey::new(Level::L1, c.map(|v| floor_div_pow2(v, 3)));
    let parent = cache.get(seed, parent_key);
    let ports = cell_ports_in(seed, cache, cell);
    let mut out = CellOutput {
        key: cell,
        primitives: Vec::new(),
        ports,
        child_constraints: Vec::new(),
        meta: CellMeta::default(),
    };

    let tops: Vec<Q> = decks(seed, LayerKey::of(parent_key, DECK_REGION_LOG2))
        .iter()
        .map(|d| d.top)
        .collect();
    let y_offset = Q::try_from(floor_mod_pow2(c.y, 3)).unwrap_or(0) * CELL;
    let mut corridors: Vec<QAabb> = Vec::new();
    for port in out.ports.iter().filter(|p| p.kind == PortKind::Walkway) {
        let floor = port.rect.v0 + y_offset;
        let Some(height) = (0..8)
            .map(|k| 4 * M * k)
            .find(|h| tops.contains(&(floor - h)))
        else {
            continue;
        };
        let walk = Skywalk {
            axis: port.face.axis,
            plane: if port.face.owner == cell { CELL } else { 0 },
            inward: if port.face.owner == cell { -1 } else { 1 },
            u0: port.rect.u0,
            u1: port.rect.u1,
            floor: port.rect.v0,
            height,
        };
        out.primitives.extend(walk.primitives());
        corridors.push(walk.corridor());
    }
    out.child_constraints.extend(
        corridors
            .iter()
            .map(|&aabb| ChildConstraint::KeepOut { aabb }),
    );

    let origin = [c.x, c.y, c.z].map(|v| Q::try_from(floor_mod_pow2(v, 3)).unwrap_or(0) * CELL);
    if let Some(col) = lift_column(seed, span_of(cell), &parent) {
        let (prims, keepouts) = col.slice(origin);
        out.primitives.extend(prims);
        out.child_constraints.extend(
            keepouts
                .iter()
                .map(|&aabb| ChildConstraint::KeepOut { aabb }),
        );
        corridors.extend(keepouts);
    }

    copy_sanctuary(&parent, origin, &mut out);

    if let Some(v) = vista(seed, cell, parent_key, &parent, origin, &corridors) {
        out.primitives.push(v.balcony);
        out.child_constraints
            .push(ChildConstraint::KeepOut { aabb: v.corridor });
        out.child_constraints.push(ChildConstraint::Zone {
            aabb: v.zone,
            kind: ZoneKind::Vista,
        });
        out.meta.flags = MetaFlags(out.meta.flags.0 | MetaFlags::VISTA.0);
        corridors.push(v.zone);
    }

    let normal = masses_in(seed, cell, &parent, &corridors);
    let normal_boxes: Vec<QAabb> = normal.iter().map(Primitive::aabb).collect();
    let mut masses: Vec<Primitive> = normal;
    for a in attached_masses(seed, cell, &parent, origin, &corridors, &normal_boxes) {
        masses.push(a.mass);
        out.primitives.extend(a.walk.primitives());
        out.child_constraints.push(ChildConstraint::KeepOut {
            aabb: a.walk.corridor(),
        });
    }
    for m in masses {
        out.child_constraints.push(ChildConstraint::Envelope {
            prim: u32::try_from(out.primitives.len()).unwrap_or(u32::MAX),
        });
        out.primitives.push(m);
    }
    let mut seen = cell_window(&parent.primitives, origin, CELL);
    seen.extend(out.primitives.iter().copied());
    out.primitives.extend(railings(seed, cell, &seen).0);
    for i in 0..out.ports.len() {
        out.child_constraints.push(ChildConstraint::ForcedPort {
            port: u32::try_from(i).unwrap_or(u32::MAX),
        });
    }
    out
}

/// Copies the parent's sanctuary zone and flag into the child that holds it.
fn copy_sanctuary(parent: &CellOutput, origin: [Q; 3], out: &mut CellOutput) {
    for c in &parent.child_constraints {
        if let ChildConstraint::Zone {
            aabb,
            kind: ZoneKind::Sanctuary,
        } = c
        {
            let local = crate::l2_mass::shift(aabb, origin);
            let inside = [local.min.x, local.min.y, local.min.z]
                .into_iter()
                .all(|v| (0..CELL).contains(&v));
            if inside {
                out.child_constraints.push(ChildConstraint::Zone {
                    aabb: local,
                    kind: ZoneKind::Sanctuary,
                });
                out.meta.flags = MetaFlags::SANCTUARY;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::cell::Axis;
    use crate::prim::{FeatureKind, Primitive, Shape};
    use crate::walk::{Nav, cell_window, plates_at_port};

    /// Checks the cell's ports against the validator. Returns `(ports, walkway ports)`.
    fn check(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> (usize, usize) {
        let out = l2_cell_in(seed, cache, cell);
        let c = cell.coords;
        let parent = cache.get(
            seed,
            CellKey::new(Level::L1, c.map(|v| floor_div_pow2(v, 3))),
        );
        let origin = [c.x, c.y, c.z].map(|v| Q::try_from(floor_mod_pow2(v, 3)).unwrap() * CELL);
        let mut prims = cell_window(&parent.primitives, origin, CELL);
        let from_l1 = prims.len();
        prims.extend(out.primitives.iter().copied());
        let nav = Nav::build(&prims);
        let in_n = |i: usize| nav.plates[i].source < from_l1;
        let (mut ports, mut walkways) = (0, 0);
        for p in &out.ports {
            if p.kind == PortKind::LiftStop {
                continue;
            }
            ports += 1;
            walkways += usize::from(p.kind == PortKind::Walkway);
            let axis = p.face.axis;
            assert_ne!(axis, Axis::Y);
            let plane = if p.face.owner == cell { CELL } else { 0 };
            let found = plates_at_port(&nav, axis, plane, (p.rect.u0, p.rect.u1), p.rect.v0);
            assert!(!found.is_empty(), "{cell:?}: no floor at {p:?}");
            let reaches = found
                .iter()
                .any(|&i| (0..nav.plates.len()).any(|j| in_n(j) && nav.connected(i, j)));
            assert!(reaches, "{cell:?}: {p:?} is cut off from the network");
        }
        for p in &out.primitives {
            let bb = p.aabb();
            let inside = |v: Q| (0..=CELL).contains(&v);
            assert!(
                [bb.min.x, bb.min.y, bb.min.z, bb.max.x, bb.max.y, bb.max.z]
                    .into_iter()
                    .all(inside),
                "{cell:?}: {p:?} leaves the cell"
            );
        }
        // The solid parts of a skywalk never meet a mass.
        let walk_prims: Vec<&Primitive> = out
            .primitives
            .iter()
            .filter(|p| matches!(p.kind, FeatureKind::WALKWAY | FeatureKind::RAMP))
            .collect();
        for m in out
            .primitives
            .iter()
            .filter(|p| p.kind == FeatureKind::MASS)
        {
            let Shape::Box { aabb } = m.shape else {
                continue;
            };
            for w in &walk_prims {
                assert!(!crate::l2_mass::overlaps(&aabb, &w.aabb()), "{cell:?}");
            }
        }
        (ports, walkways)
    }

    /// P0.8.4: every non-lift port of 10⁴ cells reaches the L1 network inside its cell.
    #[test]
    fn every_port_reaches_the_network() {
        let (mut cells, mut ports, mut walkways) = (0, 0, 0);
        'outer: for (seed, y) in [(0x0804u64, 0), (0x0805, 1), (0x0806, -2)] {
            let seed = WorldSeed(seed);
            for x in -20..20 {
                let parent = CellKey::new(Level::L1, I64Vec3::new(x, y, 3 * x % 7));
                let cache = L1Cache::default();
                for k in 0..512i64 {
                    let cell = CellKey::new(
                        Level::L2,
                        parent.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                    );
                    let (p, w) = check(seed, &cache, cell);
                    cells += 1;
                    ports += p;
                    walkways += w;
                    if cells >= 10_000 {
                        break 'outer;
                    }
                }
            }
        }
        assert!(
            cells >= 10_000 && ports > 1000 && walkways > 100,
            "{cells} {ports} {walkways}"
        );
    }

    /// P0.8.6: exactly one L2 child carries each sanctuary, with the zone free of masses.
    #[test]
    fn sanctuary_is_marked_in_one_child() {
        let seed = WorldSeed(0x0807);
        let (mut found, mut masses) = (0, 0);
        for x in 0..30 {
            let parent = CellKey::new(Level::L1, I64Vec3::new(x, 0, 2));
            let cache = L1Cache::default();
            let zones = cache
                .get(seed, parent)
                .child_constraints
                .iter()
                .filter(|c| matches!(c, ChildConstraint::Zone { .. }))
                .count();
            let mut flagged = 0;
            for k in 0..512i64 {
                let cell = CellKey::new(
                    Level::L2,
                    parent.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                );
                let out = l2_cell_in(seed, &cache, cell);
                if !out.meta.flags.contains(MetaFlags::SANCTUARY) {
                    continue;
                }
                flagged += 1;
                let zone = out
                    .child_constraints
                    .iter()
                    .find_map(|c| match c {
                        ChildConstraint::Zone { aabb, .. } => Some(*aabb),
                        _ => None,
                    })
                    .unwrap();
                for p in out
                    .primitives
                    .iter()
                    .filter(|p| p.kind == crate::prim::FeatureKind::MASS)
                {
                    masses += 1;
                    assert!(!crate::l2_mass::overlaps(&zone, &p.aabb()));
                }
            }
            assert_eq!(flagged, zones);
            found += flagged;
        }
        assert!(found >= 5, "{found}");
        let _ = masses;
    }
}
