//! The census of the world beyond L1 (`docs/spec/web-report.md` §4): the features of the L0 and
//! L1 cells (roads, rare events, wonders, wells, movers, ...) and what the L2 tissue of sampled
//! cells holds (forms, supports, variants, mutations, paths, modifiers, quarters, the kit,
//! growth, payoffs, hubs, sets, places). Every instance is a registry ID and a key, so the same
//! feature seen from two cells counts once.

use std::hash::{Hash, Hasher};

use glam::I64Vec3;
use rayon::prelude::*;
use worldgen::l2_port::L1Cache;
use worldgen::seed::WorldSeed;

use crate::Found;

fn key_of<T: Hash>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn push(out: &mut Vec<Found>, tag: &str, key: u64, cell: I64Vec3) {
    out.push(Found {
        tag: tag.to_string(),
        key,
        cell,
    });
}

/// Features that belong to an L0 cell: wonders, strange regions, weightless sites, wells, the
/// movers' sites.
pub fn l0_extra(seed: WorldSeed, l0: I64Vec3) -> Vec<Found> {
    let mut out = Vec::new();
    let cell = l0 * 8;
    let k = key_of(&(l0.x, l0.y, l0.z));
    if let Some(w) = worldgen::wonders::site_of(seed, l0) {
        push(&mut out, &w.kind.id(), k, w.l1);
    }
    if let Some(s) = worldgen::strange::strange_of(seed, l0) {
        push(&mut out, s.id(), k, cell);
    }
    if let Some(s) = worldgen::weightless::site_of(seed, l0) {
        push(&mut out, s.kind.id(), k, s.l1);
    }
    for (i, w) in worldgen::gravity::wells_of(seed, l0).iter().enumerate() {
        use worldgen::gravity::WellKind as W;
        let n = match w.kind {
            W::Point => 1,
            W::Shell => 2,
            W::Line => 3,
            W::Tube => 4,
            W::Plane => 5,
            W::Twin => 6,
            W::Seam => 7,
            W::Fold => 8,
            W::Damaged => 9,
            W::Tidal => 10,
        };
        push(
            &mut out,
            &format!("WELL-{n:02}"),
            key_of(&(l0.x, l0.y, l0.z, i)),
            cell,
        );
    }
    out
}

/// Features that belong to an L1 cell.
pub fn l1_extra(seed: WorldSeed, l1: I64Vec3) -> Vec<Found> {
    let mut out = Vec::new();
    let k = key_of(&(l1.x, l1.y, l1.z));
    for r in worldgen::roads::roads_in_cell(seed, l1) {
        push(&mut out, r.kind.id(), k, l1);
        // Every deck has rails, lamp posts every 50 m and rest niches.
        push(&mut out, "RD-01", k, l1);
        push(&mut out, "RD-06", k, l1);
    }
    if let Some(r) = worldgen::rare::rare_at(seed, l1) {
        push(&mut out, r.id(), k, l1);
    }
    if !worldgen::spines::spine_content(seed, l1).prims.is_empty() {
        for id in ["SPN-01", "SPN-02", "SPN-03"] {
            push(&mut out, id, k, l1);
        }
        if !worldgen::spines::stations(seed, l1).is_empty() {
            push(&mut out, "SPN-04", k, l1);
        }
    }
    for r in worldgen::rivers::rivers_in_cell(seed, l1) {
        push(&mut out, r.kind.id(), k, l1);
    }
    for m in worldgen::movers::movers_in_cell(seed, l1) {
        push(&mut out, m.kind.id(), m.key, l1);
        if m.kind == worldgen::movers::MoverKind::Builder {
            for id in ["BLD-01", "BLD-02", "BLD-03", "BLD-04", "BLD-05"] {
                push(&mut out, id, m.key, l1);
            }
        }
    }
    if !worldgen::ambient::lamp_sun_content(seed, l1)
        .prims
        .is_empty()
    {
        push(&mut out, "LGT-06", k, l1);
        push(&mut out, "LGT-08", k, l1);
    }
    if !worldgen::ambient::pool_content(seed, l1).prims.is_empty() {
        push(&mut out, "WX-11", k, l1);
    }
    out
}

/// The sites that a cheap L0 scan finds in a cube of `2 * radius + 1` L0 cells round the origin:
/// wonders, strange regions, weightless sites, rare events, movers, wells, the lines of roads
/// and rivers and the spine columns. Nothing is generated; each is a function of its L0 cell.
pub fn wide_scan(seed: WorldSeed, radius: i64) -> Vec<Found> {
    let cells: Vec<I64Vec3> = (-radius..=radius)
        .flat_map(|x| {
            (-radius..=radius)
                .flat_map(move |y| (-radius..=radius).map(move |z| I64Vec3::new(x, y, z)))
        })
        .collect();
    cells
        .par_iter()
        .flat_map(|&l0| {
            let mut out = l0_extra(seed, l0);
            let k = key_of(&(l0.x, l0.y, l0.z));
            if let Some(s) = worldgen::rare::site_of(seed, l0) {
                push(&mut out, s.kind.id(), k, s.l1);
            }
            if let Some((kind, l1)) = worldgen::movers::site_of(seed, l0) {
                push(&mut out, kind.id(), k, l1);
                if kind == worldgen::movers::MoverKind::Builder {
                    for id in ["BLD-01", "BLD-02", "BLD-03", "BLD-04", "BLD-05"] {
                        push(&mut out, id, k, l1);
                    }
                }
            }
            if let Some((kind, l1)) = worldgen::ambient::site_of(seed, l0) {
                push(&mut out, kind.id(), k, l1);
            }
            if let Some((_, shaft, _)) = worldgen::weightless::shaft_of(seed, l0.x, l0.z) {
                let _ = shaft;
                push(&mut out, "WGT-01", key_of(&(l0.x, l0.z)), l0 * 8);
                push(&mut out, "WGT-02", key_of(&(l0.x, l0.z)), l0 * 8);
            }
            if worldgen::spines::spine_in_column(seed, l0.x, l0.z).is_some() && l0.y == 0 {
                push(&mut out, "SPN-01", key_of(&(l0.x, l0.z, 1)), l0 * 8);
                push(&mut out, "SPN-02", key_of(&(l0.x, l0.z, 1)), l0 * 8);
                push(&mut out, "SPN-03", key_of(&(l0.x, l0.z, 1)), l0 * 8);
                push(&mut out, "SPN-04", key_of(&(l0.x, l0.z, 1)), l0 * 8);
            }
            // The infinite staircase of this L0 column (STAIR-01..04).
            if l0.y == 0 {
                let col = worldgen::key::ColumnKey {
                    level: worldgen::cell::Level::L0,
                    x: l0.x,
                    z: l0.z,
                };
                if worldgen::regime::rare_stair(seed, col).is_some() {
                    for id in ["STAIR-01", "STAIR-02", "STAIR-03", "STAIR-04"] {
                        push(&mut out, id, key_of(&(l0.x, l0.z, 7)), l0 * 8);
                    }
                }
            }
            if let Some(s) = worldgen::weightless::site_of(seed, l0) {
                // LAD-15: a rung ladder across the ceiling of every lane's tube.
                if matches!(
                    s.kind,
                    worldgen::weightless::Kind::DriftLane
                        | worldgen::weightless::Kind::GravityRiver
                ) {
                    push(&mut out, "LAD-15", k, s.l1);
                }
            }
            for (axis, row) in [(0usize, (l0.y, l0.z)), (2, (l0.y, l0.x))] {
                let key = key_of(&(axis, row.0, row.1));
                if worldgen::strange::rampart_line(seed, axis, row).is_some() {
                    push(&mut out, "STG-12", key, l0 * 8);
                    push(&mut out, "WON-24", key, l0 * 8);
                }
                if let Some(id) = worldgen::roads::line_tag(seed, axis, row) {
                    push(&mut out, id, key, l0 * 8);
                    push(&mut out, "RD-01", key, l0 * 8);
                    push(&mut out, "RD-06", key, l0 * 8);
                }
                if let Some(id) = worldgen::rivers::line_tag(seed, axis, row) {
                    push(&mut out, id, key, l0 * 8);
                }
            }
            // Gravity gates and the ladders in them: the L2 cells round a seam's middle.
            for (i, w) in worldgen::gravity::wells_of(seed, l0).iter().enumerate() {
                if w.kind != worldgen::gravity::WellKind::Seam {
                    continue;
                }
                let c = [0, 1, 2].map(|a| w.centre[a].div_euclid(512));
                for dz in -3..=3 {
                    for dy in -3..=3 {
                        for dx in -3..=3 {
                            let l2 = I64Vec3::new(c[0] + dx, c[1] + dy, c[2] + dz);
                            for g in worldgen::gates::gates_in_l2(seed, l2).iter() {
                                let key = key_of(&(g.min, g.max));
                                push(&mut out, "THR-09", key, l2);
                                push(&mut out, "GRV-04", key, l2);
                                if !worldgen::gates::gate_ladder(g, l2).is_empty() {
                                    push(&mut out, "LAD-14", key, l2);
                                }
                            }
                        }
                    }
                }
                let _ = i;
            }
            out
        })
        .collect()
}

/// The path type as its registry tag.
fn path_tag(id: u8) -> String {
    match id {
        1..=20 => format!("PH-{id:02}"),
        21..=40 => format!("PV-{:02}", id - 20),
        41..=53 => format!("LAD-{:02}", id - 40),
        61..=85 => format!("P2-{:02}", id - 60),
        _ => format!("PATH-{id}"),
    }
}

/// What the L2 tissue of one cell holds.
pub fn l2_extra(seed: WorldSeed, cache: &L1Cache, c: I64Vec3) -> Vec<Found> {
    use worldgen::cell::{CellKey, Level};
    use worldgen::prim::MaterialId;
    let mut out = Vec::new();
    let t = worldgen::tissue::tissue_cell(seed, cache.tissue(), CellKey::new(Level::L2, c));
    let ck = |n: u64| key_of(&(c.x, c.y, c.z, n));
    // The steps of the tissue (TIS-01..06) are in every cell that has a route.
    if !t.segments.is_empty() {
        for n in 1..=6 {
            push(&mut out, &format!("TIS-{n:02}"), ck(100 + n), c);
        }
    }
    for (si, s) in t.structures.iter().enumerate() {
        let f = usize::from(s.form);
        let key = ck(si as u64);
        if f < worldgen::hubs::HUB_BASE
            && let Some(d) = worldgen::forms::FORMS.get(f)
        {
            push(&mut out, d.tag, key, c);
        }
        push(
            &mut out,
            &format!("SUP-{:02}", u16::from(s.support) + 1),
            key,
            c,
        );
        push(&mut out, &format!("SV-{:02}", s.variant), key, c);
        for m in &s.mutations {
            push(&mut out, &format!("MUT-{m:02}"), key, c);
        }
    }
    for &(id, si) in &t.hubs {
        if let Some(d) = worldgen::hubs::HUBS.get(usize::from(id) - 1) {
            push(&mut out, d.tag, ck(u64::from(si)), c);
        }
    }
    for &(id, si) in &t.sets {
        if let Some(d) = worldgen::sets::SETS.get(usize::from(id) - 1) {
            push(&mut out, d.tag, ck(u64::from(si)), c);
        }
    }
    for &(id, si) in &t.locals {
        if let Some(d) = worldgen::locals::LOCALS.get(usize::from(id) - 1) {
            push(&mut out, d.tag, ck(u64::from(si)), c);
        }
    }
    for &(si, id) in &t.paths.chosen {
        push(&mut out, &path_tag(id), ck(u64::from(si)), c);
    }
    for (si, m) in &t.paths.mods {
        let key = ck(u64::from(*si));
        for n in 1..=9 {
            push(&mut out, &format!("MOD-{n:02}"), key, c);
        }
        if (1..=5).contains(&m.near) {
            push(&mut out, &format!("NEAR-{:02}", m.near), key, c);
        }
    }
    for (i, &(_, id)) in t.thresholds.list.iter().enumerate() {
        push(&mut out, &format!("THR-{id:02}"), ck(i as u64), c);
    }
    for (i, p) in t.payoffs.list.iter().enumerate() {
        if let worldgen::payoffs::PayoffKind::Install(n) = p.kind {
            push(
                &mut out,
                &format!("PAY-{:02}", u16::from(n) + 1),
                ck(i as u64),
                c,
            );
        }
    }
    for (q, p) in worldgen::quarters::dress(&t) {
        push(
            &mut out,
            &format!("QTR-{:02}", q + 1),
            key_of(&(c.x, c.y, c.z, q)),
            c,
        );
        if p.material == MaterialId::SIGNAL {
            push(&mut out, "LGT-10", ck(q as u64), c);
        }
        if p.material == MaterialId::NEON {
            push(&mut out, "LGT-11", ck(q as u64), c);
        }
    }
    for (i, _) in worldgen::kit_ext::dress(&t) {
        push(
            &mut out,
            &format!("KIT-{:02}", i + 1),
            key_of(&(c.x, c.y, c.z, i)),
            c,
        );
        if i == 9 {
            push(&mut out, "MUL-01", ck(9), c);
            push(&mut out, "MUL-02", ck(9), c);
        }
    }
    for (i, _) in worldgen::growth::dress(&t) {
        push(
            &mut out,
            &format!("GROW-{:02}", i + 1),
            key_of(&(c.x, c.y, c.z, i)),
            c,
        );
    }
    out
}

/// GROW-05 and the gates: found by the features of an L1 cell and the seams of an L0 cell.
pub fn gates_and_trees(seed: WorldSeed, l1: I64Vec3) -> Vec<Found> {
    let mut out = Vec::new();
    if worldgen::growth::has_tree(seed, l1) {
        push(&mut out, "GROW-05", key_of(&(l1.x, l1.y, l1.z)), l1);
    }
    out
}

/// The cells of the sample, in batches grouped by their L1 cell so each batch has its own
/// cache (a cache that keeps every parent of a wide sample would fill the memory).
pub fn l2_census(seed: WorldSeed, cells: &[I64Vec3]) -> Vec<Found> {
    let mut sorted = cells.to_vec();
    sorted.sort_by_key(|c| {
        let l1 = c.div_euclid(I64Vec3::splat(8));
        (l1.x, l1.y, l1.z)
    });
    sorted
        .chunks(400)
        .flat_map(|batch| {
            let cache = L1Cache::default();
            batch
                .par_iter()
                .flat_map(|&c| l2_extra(seed, &cache, c))
                .collect::<Vec<_>>()
        })
        .collect()
}
