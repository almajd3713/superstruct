//! `--tissue N`: the metrics that the L2 tissue brings (`docs/spec/web-report.md` §3), over N
//! sampled L2 cells, estimated per cell:
//! MET-01 built fraction in crusts, MET-08 path types per km of path, MET-10 nav nodes with
//! three or more links. The metrics that follow a walker across cells (MET-04, 05, 07) need the
//! bots (P6.1.1) and are not measured here, and MET-03's rays are limited to the cell.

use std::collections::BTreeSet;

use glam::{I64Vec3, Vec3};
use rayon::prelude::*;
use worldgen::cell::{CellKey, Level};
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::prim::{PrimFlags, Q_PER_M};
use worldgen::seed::WorldSeed;
use worldgen::stack::{Stratum, stack};
use worldgen::tissue::tissue_cell;
use worldgen::walk::Nav;

const GRID: usize = 64;
const CUBE: usize = 8;

/// What one cell contributes.
#[derive(Default, Clone, Copy)]
pub struct Cell {
    /// Crust only: the mean over 64 m cubes of the share of 8 m cells that are built.
    pub built: Option<f64>,
    /// The tissue's own fill target in these cells, 0 to 1.
    pub target: f64,
    /// Crust only: the share of slots that are built, engulfed or solid, which counts every
    /// structure by its envelope (a mass that L3 fills is emitted as a hollow shell).
    pub envelope: Option<f64>,
    /// Nav nodes (plates), and how many have three or more links.
    pub nodes: u64,
    pub linked3: u64,
    /// Path length (m) and distinct path types in the cell.
    pub path_m: f64,
    pub types: u32,
}

fn built_fraction(prims: &[worldgen::prim::Primitive]) -> f64 {
    let mut solid = vec![false; GRID * GRID * GRID];
    let q = Q_PER_M as f32;
    for p in prims {
        let counts = p.flags.contains(PrimFlags::ENVELOPE)
            && !p.flags.contains(PrimFlags::INTERIOR)
            || !p.flags.contains(PrimFlags::NO_COLLIDE);
        if !counts || p.flags.contains(PrimFlags::FAR_ONLY) {
            continue;
        }
        let b = p.aabb();
        let lo =
            [b.min.x, b.min.y, b.min.z].map(|v| ((v as f32 / q / 8.0).floor().max(0.0)) as usize);
        let hi = [b.max.x, b.max.y, b.max.z]
            .map(|v| ((v as f32 / q / 8.0).ceil().min(GRID as f32)) as usize);
        for z in lo[2]..hi[2] {
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    let i = (z * GRID + y) * GRID + x;
                    if !solid[i] {
                        let c = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) * 8.0;
                        // A hollow shell that L3 will fill is an envelope: all of it counts.
                        solid[i] = p.shape.contains(c);
                    }
                }
            }
        }
    }
    let mut sum = 0.0;
    let mut cubes = 0.0;
    for cz in 0..GRID / CUBE {
        for cy in 0..GRID / CUBE {
            for cx in 0..GRID / CUBE {
                let mut n = 0;
                for z in 0..CUBE {
                    for y in 0..CUBE {
                        for x in 0..CUBE {
                            let i = ((cz * CUBE + z) * GRID + cy * CUBE + y) * GRID + cx * CUBE + x;
                            n += usize::from(solid[i]);
                        }
                    }
                }
                sum += n as f64 / (CUBE * CUBE * CUBE) as f64;
                cubes += 1.0;
            }
        }
    }
    sum / cubes
}

fn measure(seed: WorldSeed, cache: &L1Cache, c: I64Vec3) -> Cell {
    let key = CellKey::new(Level::L2, c);
    let l0 = c.div_euclid(I64Vec3::splat(64));
    let local = c * 512 + I64Vec3::splat(256) - l0 * 32_768;
    let crust = matches!(stack(seed, l0).stratum_at(local, None), Stratum::Crust(_));
    let out = l2_cell_in(seed, cache, key);
    let mut cell = Cell::default();
    if crust {
        // The cell's own pieces plus the parent L1 pieces that meet it (the frame, the skin).
        let l1 = c.div_euclid(I64Vec3::splat(8));
        let parent = cache.get(seed, CellKey::new(Level::L1, l1));
        let o = (c - l1 * 8) * 512 * i64::from(Q_PER_M);
        let origin = [o.x as i32, o.y as i32, o.z as i32];
        let mut all = worldgen::walk::cell_window(&parent.primitives, origin, 512 * Q_PER_M);
        all.extend(out.primitives.iter().copied());
        cell.built = Some(built_fraction(&all));
    }
    let nav = Nav::build(&out.primitives);
    let mut degree = vec![0u32; nav.plates.len()];
    for &(a, b) in &nav.edges {
        degree[a] += 1;
        degree[b] += 1;
    }
    cell.nodes = degree.len() as u64;
    cell.linked3 = degree.iter().filter(|&&d| d >= 3).count() as u64;
    let t = tissue_cell(seed, cache.tissue(), key);
    cell.target = f64::from(t.fill) / 255.0;
    if crust {
        let present = t.lattice.present.iter().filter(|&&p| p).count().max(1);
        let built = t
            .lattice
            .state
            .iter()
            .zip(&t.lattice.present)
            .filter(|(s, p)| {
                **p && matches!(
                    s,
                    worldgen::tissue::SlotState::Built
                        | worldgen::tissue::SlotState::Engulfed
                        | worldgen::tissue::SlotState::Solid
                )
            })
            .count();
        cell.envelope = Some(built as f64 / present as f64);
    }
    let mut types = BTreeSet::new();
    for w in &t.paths.walks {
        types.insert(w.id);
        cell.path_m += w
            .points
            .windows(2)
            .map(|p| (p[1] - p[0]).length())
            .sum::<f64>();
    }
    cell.types = types.len() as u32;
    cell
}

/// Measures `cells` on `threads` threads and prints the metrics.
pub fn run(seed: WorldSeed, cells: &[I64Vec3], threads: usize) -> anyhow::Result<()> {
    let cache = L1Cache::default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;
    let all: Vec<Cell> = pool.install(|| {
        cells
            .par_iter()
            .map(|&c| measure(seed, &cache, c))
            .collect()
    });
    let built: Vec<f64> = all.iter().filter_map(|c| c.built).collect();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let (nodes, linked): (u64, u64) = all
        .iter()
        .fold((0, 0), |a, c| (a.0 + c.nodes, a.1 + c.linked3));
    let (km, types): (f64, f64) = all.iter().fold((0.0, 0.0), |a, c| {
        (a.0 + c.path_m / 1000.0, a.1 + f64::from(c.types))
    });
    println!(
        "tissue metrics: {} cells ({} in crusts)",
        all.len(),
        built.len()
    );
    let target: Vec<f64> = all
        .iter()
        .filter(|c| c.built.is_some())
        .map(|c| c.target)
        .collect();
    let env: Vec<f64> = all.iter().filter_map(|c| c.envelope).collect();
    println!(
        "  MET-01 built fraction in crusts        {:.3}   target 0.70-0.90   (envelopes as solid; the tissue aims at {:.3}; solid pieces alone cover {:.3})",
        mean(&env),
        mean(&target),
        mean(&built)
    );
    println!(
        "  MET-10 nav nodes with 3+ links         {:.3}   target >= 0.40   ({linked} of {nodes} plates)",
        linked as f64 / nodes.max(1) as f64
    );
    println!(
        "  MET-08 path types per km (per cell)    {:.1}    target >= 8      ({types:.0} types over {km:.1} km of path)",
        types / km.max(1e-9)
    );
    Ok(())
}
