//! Validation of L3 output (`docs/spec/levels.md` §6.11, P4.1.4).
//!
//! `check_l3` works per L2 cell, because the connectivity theorem for L3 is stated inside the
//! parent L2 cell: it builds the walkability graph from the L1 window, the L2 output without
//! its envelopes, and every active block's primitives, and checks that L3 attaches to the
//! network N.

// Block and slot indices and counts are small.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]

use crate::cell::{Axis, CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::l2::{CELL as L2_CELL, l2_build};
use crate::l2_entrance::entrances;
use crate::l2_port::L1Cache;
use crate::l3::{L3Block, l3_blocks_of};
use crate::l3_plan::COL;
use crate::port::PortKind;
use crate::prim::{ChildConstraint, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::seed::WorldSeed;
use crate::walk::{Nav, STEP_UP, cell_window, plates_at_port};

/// What `check_l3` found in one L2 cell.
#[derive(Debug, Clone, Default)]
pub struct L3Report {
    pub blocks: u32,
    pub fallbacks: u32,
    pub entrances: u32,
    pub ports: u32,
    pub l3_plates: u32,
    pub failures: Vec<String>,
}

fn shift_prim(p: &Primitive, o: [Q; 3]) -> Primitive {
    let sh = |bb: QAabb| {
        QAabb::new(
            QVec3::new(bb.min.x + o[0], bb.min.y + o[1], bb.min.z + o[2]),
            QVec3::new(bb.max.x + o[0], bb.max.y + o[1], bb.max.z + o[2]),
        )
    };
    let shape = match p.shape {
        Shape::Box { aabb } => Shape::Box { aabb: sh(aabb) },
        Shape::Ramp { aabb, rise } => Shape::Ramp {
            aabb: sh(aabb),
            rise,
        },
        Shape::StairRun { aabb, rise, steps } => Shape::StairRun {
            aabb: sh(aabb),
            rise,
            steps,
        },
        Shape::LiftShaft { aabb } => Shape::LiftShaft { aabb: sh(aabb) },
        Shape::Oriented(or) => {
            Shape::Oriented(or.translated(|q| QVec3::new(q.x + o[0], q.y + o[1], q.z + o[2])))
        }
        Shape::Cylinder {
            axis,
            base,
            length,
            radius,
        } => Shape::Cylinder {
            axis,
            base: QVec3::new(base.x + o[0], base.y + o[1], base.z + o[2]),
            length,
            radius,
        },
    };
    Primitive { shape, ..*p }
}

fn overlaps(a: &QAabb, b: &QAabb) -> bool {
    a.min.x < b.max.x
        && b.min.x < a.max.x
        && a.min.y < b.max.y
        && b.min.y < a.max.y
        && a.min.z < b.max.z
        && b.min.z < a.max.z
}

/// Plates whose plan rectangle holds `(x, z)` and whose surface is within a step of `y` there.
fn plates_at(nav: &Nav, x: Q, z: Q, y: Q) -> Vec<usize> {
    (0..nav.plates.len())
        .filter(|&i| {
            let p = &nav.plates[i];
            p.rect.x0 <= x
                && x <= p.rect.x1
                && p.rect.z0 <= z
                && z <= p.rect.z1
                && (p.height_at(x, z) - f64::from(y)).abs() <= f64::from(STEP_UP)
        })
        .collect()
}

#[allow(clippy::too_many_lines)] // one checklist, item by item
pub fn check_l3(seed: WorldSeed, cache: &L1Cache, l2: CellKey) -> L3Report {
    debug_assert_eq!(l2.level, Level::L2);
    let mut report = L3Report::default();
    let (out, masses) = l2_build(seed, cache, l2);
    if masses.is_empty() {
        return report;
    }
    let l1 = cache.get(
        seed,
        CellKey::new(Level::L1, l2.coords.map(|v| floor_div_pow2(v, 3))),
    );
    let masses = crate::l3::clear_of_trunks(&masses, &l1, l2);
    let o = l2.coords.map(|v| floor_mod_pow2(v, 3) * i64::from(L2_CELL));
    let origin = [o.x as Q, o.y as Q, o.z as Q];
    let blocks: Vec<(CellKey, L3Block)> = l3_blocks_of(seed, cache, l2);
    report.blocks = blocks.len() as u32;
    report.fallbacks = blocks.iter().filter(|(_, b)| b.meta.fallback).count() as u32;

    // The nav: L1 window, L2 without envelopes, then L3.
    let mut prims = cell_window(&l1.primitives, origin, L2_CELL);
    let from_l1 = prims.len();
    prims.extend(
        out.primitives
            .iter()
            .filter(|p| !p.flags.contains(PrimFlags::ENVELOPE))
            .copied(),
    );
    let l3_start = prims.len();
    for (key, b) in &blocks {
        let k = key.coords - l2.coords * 8;
        let off = [k.x as Q * 1024, k.y as Q * 1024, k.z as Q * 1024];
        prims.extend(b.out.primitives.iter().map(|p| shift_prim(p, off)));
        // 5. Nothing leaves its block, and nothing collidable is outside an envelope or inside a
        // parent keep-out.
        for p in &b.out.primitives {
            let bb = p.aabb();
            if bb.min.x < 0
                || bb.min.y < 0
                || bb.min.z < 0
                || bb.max.x > 1024
                || bb.max.y > 1024
                || bb.max.z > 1024
            {
                report
                    .failures
                    .push(format!("{key:?}: {:?} leaves its block", p.kind));
            }
            if p.flags.contains(PrimFlags::NO_COLLIDE) || matches!(p.shape, Shape::LiftShaft { .. })
            {
                continue;
            }
            let w = shift_prim(p, off).aabb();
            let inside = masses.iter().any(|m| {
                let a = &m.aabb;
                a.min.x <= w.min.x
                    && w.max.x <= a.max.x
                    && a.min.y <= w.min.y
                    && w.max.y <= a.max.y
                    && a.min.z <= w.min.z
                    && w.max.z <= a.max.z
            });
            if !inside {
                report
                    .failures
                    .push(format!("{key:?}: {:?} outside every envelope", p.kind));
            }
            if out
                .child_constraints
                .iter()
                .any(|c| matches!(c, ChildConstraint::KeepOut { aabb } if overlaps(aabb, &w)))
            {
                report
                    .failures
                    .push(format!("{key:?}: {:?} meets a keep-out", p.kind));
            }
        }
    }
    let nav = Nav::build_fine(&prims);
    let in_network: std::collections::HashSet<usize> = (0..nav.plates.len())
        .filter(|&i| nav.plates[i].source < from_l1)
        .map(|i| nav.component[i])
        .collect();
    let reaches = |plates: &[usize]| {
        plates
            .iter()
            .any(|&i| in_network.contains(&nav.component[i]))
    };

    // 1. L2 ports keep their floor and their connection.
    for p in &out.ports {
        if matches!(p.kind, PortKind::LiftStop | PortKind::Entrance { .. }) {
            continue;
        }
        let plane = if p.face.owner == l2 { L2_CELL } else { 0 };
        let found = plates_at_port(&nav, p.face.axis, plane, (p.rect.u0, p.rect.u1), p.rect.v0);
        if found.is_empty() || !reaches(&found) {
            report
                .failures
                .push(format!("L2 port {p:?} lost its connection"));
        }
    }

    // 2. Entrances: floor outside and inside, both reaching N.
    let parent_l1 = cache.get(
        seed,
        CellKey::new(Level::L1, l2.coords.map(|v| floor_div_pow2(v, 3))),
    );
    for (m, e) in masses
        .iter()
        .zip(entrances(seed, l2, &parent_l1, &masses, &out.primitives))
    {
        let Some(e) = e else {
            continue;
        };
        report.entrances += 1;
        let d = e.doorway;
        let u = i32::midpoint(d.u0, d.u1);
        let (face, out_dir): (Q, Q) = match d.side {
            0 => (m.aabb.min.z, -1),
            1 => (m.aabb.max.x, 1),
            2 => (m.aabb.max.z, 1),
            _ => (m.aabb.min.x, -1),
        };
        let at = |n: Q, y: Q| {
            if d.side.is_multiple_of(2) {
                plates_at(&nav, u, n, y)
            } else {
                plates_at(&nav, n, u, y)
            }
        };
        let outside = at(face + out_dir * 8, d.floor);
        // Deck masses stand on a deck apron; attached ones on the skywalk landing.
        if outside.is_empty() || !reaches(&outside) {
            report
                .failures
                .push(format!("entrance {d:?} has no connected floor outside"));
        }
        let inside = at(face - out_dir * 8, d.floor + 4);
        if inside.is_empty() || !reaches(&inside) {
            report
                .failures
                .push(format!("entrance {d:?} has no connected floor inside"));
        }
    }

    // 3. L3 own ports have floor on both sides.
    for (key, b) in &blocks {
        let k = key.coords - l2.coords * 8;
        for p in &b.out.ports {
            if p.kind != PortKind::Walkway {
                continue;
            }
            report.ports += 1;
            let owner = p.face.owner.coords - l2.coords * 8;
            let (plane, uo, yo) = if p.face.axis == Axis::X {
                (
                    (owner.x as Q + 1) * 1024,
                    k.z as Q * 1024,
                    owner.y as Q * 1024,
                )
            } else {
                (
                    (owner.z as Q + 1) * 1024,
                    k.x as Q * 1024,
                    owner.y as Q * 1024,
                )
            };
            let um = i32::midpoint(p.rect.u0, p.rect.u1) + uo;
            let y = p.rect.v0 + yo;
            let side = |n: Q| {
                if p.face.axis == Axis::X {
                    plates_at(&nav, n, um, y)
                } else {
                    plates_at(&nav, um, n, y)
                }
            };
            let (a, c) = (side(plane - 8), side(plane + 8));
            let neighbour_info = || {
                // The slot on the far side of the face, from the block on that side.
                let step = if p.face.axis == Axis::X {
                    glam::I64Vec3::X
                } else {
                    glam::I64Vec3::Z
                };
                let (lo, hi) = (p.face.owner.coords, p.face.owner.coords + step);
                let ui = (p.rect.u0 / 128) as usize;
                let vi = (p.rect.v0 / 64) as usize;
                [lo, hi].map(|c| {
                    let slot = if p.face.axis == Axis::X {
                        (vi * 8 + ui) * 8 + if c == hi { 0 } else { 7 }
                    } else {
                        (vi * 8 + if c == hi { 0 } else { 7 }) * 8 + ui
                    };
                    blocks.iter().find(|(k, _)| k.coords == c).map(|(_, b)| {
                        let sl = b.layout.slots[slot];
                        let fam = (usize::from(sl.tile) < crate::l3_tiles::N_TILES)
                            .then(|| crate::l3_tiles::table()[usize::from(sl.tile)]);
                        format!("{:?}/{:?}/{fam:?}", sl.class, sl.tile)
                    })
                })
            };
            if a.is_empty() || c.is_empty() || !reaches(&a) || !reaches(&c) {
                report.failures.push(format!(
                    "{key:?}: port {p:?} has no connected floor on both sides (plane {plane} at {um},{y}: {} plates before, {} after, reach {} {}; slots {:?})",
                    a.len(),
                    c.len(),
                    reaches(&a),
                    reaches(&c),
                    neighbour_info()
                ));
            }
        }
    }

    // 4. No stranded floor: the floor of every walkable slot reaches N. (Plates that the
    // extractor leaves as isolated slivers beside a wall are not rooms, so the check is made per
    // slot, at the point where a person would stand.)
    for (key, b) in &blocks {
        let k = key.coords - l2.coords * 8;
        let off = [k.x as Q * 1024, k.y as Q * 1024, k.z as Q * 1024];
        for (i, s) in b.layout.slots.iter().enumerate() {
            if s.class != crate::l3::SlotClass::Core {
                continue;
            }
            let t = s.tile;
            let (px, pz) = if t == crate::l3::VESTIBULE {
                (64, 64)
            } else if usize::from(t) < crate::l3_tiles::N_TILES {
                let info = crate::l3_tiles::table()[usize::from(t)];
                match info.family {
                    crate::l3_tiles::Family::Room
                    | crate::l3_tiles::Family::Atrium
                    | crate::l3_tiles::Family::Corridor => (64, 64),
                    crate::l3_tiles::Family::SpiralBase
                    | crate::l3_tiles::Family::Spiral
                    | crate::l3_tiles::Family::SpiralTop => {
                        // The lane-0 flat, turned with the tile.
                        let (mut x, mut z) = (50, 18);
                        for _ in 0..info.rot {
                            (x, z) = (128 - z, x);
                        }
                        (x, z)
                    }
                    _ => continue,
                }
            } else {
                continue;
            };
            let (x, tt, z) = (i % 8, i / 64, i / 8 % 8);
            let at = [
                off[0] + x as Q * 128 + px,
                off[1] + tt as Q * 64 + 4,
                off[2] + z as Q * 128 + pz,
            ];
            report.l3_plates += 1;
            let found = plates_at(&nav, at[0], at[2], at[1]);
            if found.is_empty() || !reaches(&found) {
                report.failures.push(format!(
                    "{key:?}: stranded floor in slot ({x},{tt},{z}) tile {t} at {at:?}"
                ));
            }
        }
    }

    // 6. Envelope fill: at least 75 % of each side face is solid within 0.5 m of its plane.
    let l3_solids: Vec<QAabb> = prims[l3_start..]
        .iter()
        .filter(|p| {
            !p.flags.contains(PrimFlags::NO_COLLIDE)
                && !matches!(p.shape, Shape::LiftShaft { .. } | Shape::Cylinder { .. })
        })
        .map(Primitive::aabb)
        .collect();
    for m in &masses {
        let a = &m.aabb;
        let step: Q = 32;
        for side in 0..4u8 {
            let (mut covered, mut total) = (0u32, 0u32);
            let along = if side % 2 == 0 {
                (a.min.x, a.max.x)
            } else {
                (a.min.z, a.max.z)
            };
            let mut u = along.0;
            while u < along.1 {
                let mut y = a.min.y;
                while y < a.max.y {
                    let probe = |lo: Q, hi: Q| {
                        let (p0, p1) = (lo, hi);
                        let bx = if side.is_multiple_of(2) {
                            QAabb::new(QVec3::new(u, y, p0), QVec3::new(u + step, y + step, p1))
                        } else {
                            QAabb::new(QVec3::new(p0, y, u), QVec3::new(p1, y + step, u + step))
                        };
                        l3_solids.iter().any(|s| overlaps(s, &bx))
                    };
                    let hit = match side {
                        0 => probe(a.min.z, a.min.z + 8),
                        2 => probe(a.max.z - 8, a.max.z),
                        1 => probe(a.max.x - 8, a.max.x),
                        _ => probe(a.min.x, a.min.x + 8),
                    };
                    total += 1;
                    covered += u32::from(hit);
                    y += step;
                }
                u += step;
            }
            if total > 0 && f64::from(covered) < 0.75 * f64::from(total) {
                report.failures.push(format!(
                    "envelope face {side} of {:?} is only {covered}/{total} covered",
                    a.min
                ));
            }
        }
    }
    let _ = COL;
    report
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;

    #[test]
    #[ignore = "reads the v6 L1 (lift core, decks, bridges), replaced in version 7; rebuilt or deleted with PW.3"]
    fn l3_attaches_to_the_network() {
        let seed = WorldSeed(0x0413);
        let cache = L1Cache::default();
        let (mut cells, mut blocks, mut entrances, mut ports, mut plates) = (0, 0, 0, 0, 0);
        let mut failures: Vec<String> = Vec::new();
        'scan: for x in -30..30i64 {
            let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 3, 2 * x % 5));
            for k in 0..512i64 {
                let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
                let cell = CellKey::new(Level::L2, l1.coords * 8 + idx);
                let r = check_l3(seed, &cache, cell);
                if r.blocks == 0 {
                    continue;
                }
                cells += 1;
                blocks += r.blocks;
                entrances += r.entrances;
                ports += r.ports;
                plates += r.l3_plates;
                failures.extend(r.failures.into_iter().map(|f| format!("{cell:?}: {f}")));
                if cells >= 40 {
                    break 'scan;
                }
            }
        }
        println!(
            "{cells} cells, {blocks} blocks, {entrances} entrances, {ports} ports, {plates} L3 plates, {} failures",
            failures.len()
        );
        let mut kinds: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for f in &failures {
            let k = [
                "stranded",
                "both sides",
                "entrance",
                "envelope face",
                "keep-out",
                "leaves its block",
                "outside every",
                "lost its connection",
            ]
            .iter()
            .find(|k| f.contains(**k))
            .map_or("other", |k| *k);
            *kinds.entry(k.to_owned()).or_default() += 1;
        }
        println!("{kinds:?}");
        let mut by_family: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for f in &failures {
            if let Some(rest) = f.split("tile ").nth(1)
                && let Some(id) = rest
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
            {
                let fam = match id {
                    0 => "solid",
                    1..=81 => "room",
                    82..=97 => "atrium",
                    98..=177 => "void",
                    178..=192 => "corridor",
                    193..=204 => "spiral",
                    _ => "vestibule",
                };
                *by_family.entry(fam).or_default() += 1;
            } else if f.contains("stranded floor in slot") {
                *by_family.entry("vestibule").or_default() += 1;
            }
        }
        println!("{by_family:?}");
        for f in failures.iter().take(8) {
            println!("  F {f}");
        }
        for k in [
            "entrance",
            "stranded",
            "envelope face",
            "keep-out",
            "outside every",
        ] {
            if let Some(f) = failures.iter().find(|f| f.contains(k)) {
                println!("  {f}");
            }
        }
        assert!(failures.is_empty(), "{} failures", failures.len());
    }

    /// Diagnostic: the failures of one L2 cell (`L3_CELL=x,y,z`).
    #[test]
    #[ignore = "diagnostic"]
    fn one_cell() {
        let v: Vec<i64> = std::env::var("L3_CELL")
            .unwrap()
            .split(',')
            .map(|s| s.parse().unwrap())
            .collect();
        let cell = CellKey::new(Level::L2, I64Vec3::new(v[0], v[1], v[2]));
        let r = check_l3(WorldSeed(0x1004), &L1Cache::default(), cell);
        println!(
            "{} blocks, {} fallbacks, {} failures",
            r.blocks,
            r.fallbacks,
            r.failures.len()
        );
        let mut kinds: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for f in &r.failures {
            let k = f
                .split(": ")
                .nth(1)
                .unwrap_or(f)
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            *kinds.entry(k).or_default() += 1;
        }
        println!("{kinds:?}");
        for f in r.failures.iter().take(3) {
            println!("  F {f}");
        }
    }

    /// P4.1.4: 10^4 active blocks keep every port, entrance and walkable slot connected.
    #[test]
    #[ignore = "10^4 blocks; run in release"]
    fn ten_thousand_blocks_stay_connected() {
        let seed = WorldSeed(0x1004);
        let cache = L1Cache::default();
        let (mut blocks, mut cells) = (0u32, 0u32);
        let mut failures: Vec<String> = Vec::new();
        'scan: for x in -200..200i64 {
            let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 4, 2 * x % 7));
            for k in 0..512i64 {
                let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
                let cell = CellKey::new(Level::L2, l1.coords * 8 + idx);
                let r = check_l3(seed, &cache, cell);
                if r.blocks == 0 {
                    continue;
                }
                cells += 1;
                blocks += r.blocks;
                failures.extend(r.failures.into_iter().map(|f| format!("{cell:?}: {f}")));
                if blocks >= 10_000 {
                    break 'scan;
                }
            }
        }
        println!(
            "{cells} cells, {blocks} blocks, {} failures",
            failures.len()
        );
        for f in failures
            .iter()
            .take(if std::env::var_os("L3_ALL").is_some() {
                usize::MAX
            } else {
                10
            })
        {
            println!("  {f}");
        }
        assert!(failures.is_empty());
    }
}
