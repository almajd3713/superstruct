//! The world-scale features of an L1 cell, gathered in one place: rare events (`rare`), roads
//! (`roads`) and the systems that follow. Each is a pure function of `(seed, L1 cell)` that
//! returns the pieces it adds and the space it keeps clear.

use glam::I64Vec3;

use crate::prim::{Primitive, Q, QAabb};
use crate::seed::WorldSeed;

/// What a feature adds to its L1 cell, in cell-local quanta, and the space it keeps clear of
/// everything else.
#[derive(Debug, Default)]
pub struct Content {
    pub prims: Vec<Primitive>,
    pub keep_out: Vec<QAabb>,
}

/// Metres to quanta.
pub const M: Q = 16;

/// Every feature that touches L1 cell `l1`.
pub fn contents(seed: WorldSeed, l1: I64Vec3) -> Vec<Content> {
    let mut out = Vec::new();
    // Movers come first: the engine draws them whatever the cell holds, so their supports must
    // stand.
    for c in [
        crate::movers::mover_content(seed, l1),
        crate::rare::rare_content(seed, l1),
        crate::roads::road_content(seed, l1),
        crate::spines::spine_content(seed, l1),
        crate::rivers::river_content(seed, l1),
        crate::wonders::wonder_content(seed, l1),
        crate::weightless::weightless_content(seed, l1),
        crate::growth::tree_content(seed, l1),
        crate::strange::strange_content(seed, l1),
        crate::ambient::lamp_sun_content(seed, l1),
        crate::ambient::pool_content(seed, l1),
    ] {
        if c.prims.is_empty() && c.keep_out.is_empty() {
            continue;
        }
        // Two features that would clear the same space cannot both stand: the first in the
        // list keeps it, so a cell never holds one feature inside another's cleared space.
        let clash = |a: &QAabb, b: &QAabb| {
            a.min.x < b.max.x
                && a.max.x > b.min.x
                && a.min.y < b.max.y
                && a.max.y > b.min.y
                && a.min.z < b.max.z
                && a.max.z > b.min.z
        };
        let taken = out.iter().any(|o: &Content| {
            o.keep_out
                .iter()
                .any(|a| c.keep_out.iter().any(|b| clash(a, b)))
        });
        if !taken {
            out.push(c);
        }
    }
    out
}

/// The L2 cells of `l1` that hold or touch the space the features keep clear, for the artery
/// tree to reach.
pub fn tie_cells(seed: WorldSeed, l1: I64Vec3) -> Vec<[i64; 3]> {
    let l2 = |q: i32| i64::from(q / M).div_euclid(512);
    let mut cells = Vec::new();
    for content in contents(seed, l1) {
        for k in &content.keep_out {
            let lo = [k.min.x, k.min.y, k.min.z].map(|v| (l2(v) - 1).max(0));
            let hi = [k.max.x, k.max.y, k.max.z].map(|v| (l2(v - 1) + 1).min(7));
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        if !cells.contains(&[x, y, z]) {
                            cells.push([x, y, z]);
                        }
                    }
                }
            }
        }
    }
    cells
}
