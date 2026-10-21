//! The nav graph of a block of L2 cells and what the walkers measure on it. The nodes are points
//! 8 m apart along the tissue's walks (the designed walk network, with its path types); walks
//! that pass within 2.5 m of each other, in a cell or across a face, are joined.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::I64Vec3;
use rayon::prelude::*;
use worldgen::cell::{CellKey, Level};
use worldgen::l2_port::L1Cache;
use worldgen::prim::{PrimFlags, Q_PER_M};
use worldgen::seed::WorldSeed;
use worldgen::stack::{Stratum, stack};
use worldgen::tissue::tissue_cell;

use crate::Opts;

/// The occupancy grid: 16 m cells; a cell is solid when a piece covers a quarter of it.
const GRID_M: f64 = 16.0;
/// A long view is a ray this long (MET-04), the cap on the search (20 km) and the rays' reach.
const LONG_M: f64 = 2000.0;
const CAP_M: f64 = 20_000.0;
const REACH_M: f64 = 4000.0;

struct CellData {
    crust: bool,
    /// Cell-local metres of the centre of each payoff that shows open space or a place.
    breath: Vec<[f32; 3]>,
    /// The tissue's walks: the type and the polyline in cell-local metres.
    walks: Vec<(u8, Vec<[f32; 3]>)>,
}

/// Metres between nodes along a walk, and how near two walks must pass to be joined.
const STEP_M: f32 = 8.0;
const JOIN_M: f32 = 20.0;

pub struct Graph {
    pub cells: usize,
    dim: i64,
    data: Vec<CellData>,
    pos: Vec<[f32; 3]>,
    /// The cell of each node, and the type of its walk.
    cell: Vec<u32>,
    kind: Vec<u8>,
    /// Whether the node is an end of a walk or joins another walk (what MET-10 counts).
    key: Vec<bool>,
    /// The first node of each cell and one past its last.
    range: Vec<(u32, u32)>,
    adj: Vec<Vec<(u32, f32)>>,
    grid: Vec<AtomicU64>,
    gdim: usize,
}

impl Graph {
    pub fn nodes(&self) -> usize {
        self.pos.len()
    }

    pub fn links(&self) -> usize {
        self.adj.iter().map(Vec::len).sum::<usize>() / 2
    }

    fn solid(&self, p: [f64; 3]) -> bool {
        let g = |v: f64| (v / GRID_M).floor();
        let (x, y, z) = (g(p[0]), g(p[1]), g(p[2]));
        let n = self.gdim as f64;
        if x < 0.0 || y < 0.0 || z < 0.0 || x >= n || y >= n || z >= n {
            return false;
        }
        let i = ((z as usize * self.gdim + y as usize) * self.gdim) + x as usize;
        self.grid[i / 64].load(Ordering::Relaxed) >> (i % 64) & 1 == 1
    }
}

/// Marks the grid cells that the box `lo..hi` (block metres) covers by a quarter or more.
fn mark(grid: &[AtomicU64], gdim: usize, lo: [f64; 3], hi: [f64; 3]) {
    let n = gdim as i64;
    let range = |k: usize| {
        (
            ((lo[k] / GRID_M).floor() as i64).max(0),
            ((hi[k] / GRID_M).ceil() as i64).min(n),
        )
    };
    let (rx, ry, rz) = (range(0), range(1), range(2));
    let cover = |k: usize, c: i64| {
        let (a, b) = (c as f64 * GRID_M, (c + 1) as f64 * GRID_M);
        (hi[k].min(b) - lo[k].max(a)).max(0.0) / GRID_M
    };
    for z in rz.0..rz.1 {
        let fz = cover(2, z);
        for y in ry.0..ry.1 {
            let fy = cover(1, y);
            for x in rx.0..rx.1 {
                if cover(0, x) * fy * fz >= 0.25 {
                    let i = ((z * n + y) * n + x) as usize;
                    grid[i / 64].fetch_or(1 << (i % 64), Ordering::Relaxed);
                }
            }
        }
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2))
        .sqrt()
        .max(0.5)
}

pub fn load(o: &Opts) -> Graph {
    let seed = WorldSeed(o.seed);
    let dim = o.blocks * 8;
    let low = o.low * 8;
    let gdim = (dim * 512 / GRID_M as i64) as usize;
    let words = gdim * gdim * gdim / 64 + 1;
    let grid: Vec<AtomicU64> = (0..words).map(|_| AtomicU64::new(0)).collect();
    let cache = L1Cache::default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(o.threads.max(1))
        .build()
        .expect("thread pool");
    // The L1 pieces: the frame, the skin and the features.
    pool.install(|| {
        (0..o.blocks * o.blocks * o.blocks)
            .into_par_iter()
            .for_each(|k| {
                let (x, y, z) = (
                    k % o.blocks,
                    k / o.blocks % o.blocks,
                    k / (o.blocks * o.blocks),
                );
                let l1 = o.low + I64Vec3::new(x, y, z);
                let out = cache.get(seed, CellKey::new(Level::L1, l1));
                let org = [x, y, z].map(|v| (v * 4096) as f64);
                let q = f64::from(Q_PER_M);
                for p in &out.primitives {
                    if p.flags.contains(PrimFlags::NO_COLLIDE)
                        || p.flags.contains(PrimFlags::FAR_ONLY)
                    {
                        continue;
                    }
                    let b = p.aabb();
                    let lo = [b.min.x, b.min.y, b.min.z].map(f64::from);
                    let hi = [b.max.x, b.max.y, b.max.z].map(f64::from);
                    mark(
                        &grid,
                        gdim,
                        [0, 1, 2].map(|i| org[i] + lo[i] / q),
                        [0, 1, 2].map(|i| org[i] + hi[i] / q),
                    );
                }
            });
    });
    let total = (dim * dim * dim) as usize;
    let data: Vec<CellData> = pool.install(|| {
        (0..total)
            .into_par_iter()
            .map(|k| {
                let k = k as i64;
                let (x, y, z) = (k % dim, k / dim % dim, k / (dim * dim));
                let c = low + I64Vec3::new(x, y, z);
                let key = CellKey::new(Level::L2, c);
                let l0 = c.div_euclid(I64Vec3::splat(64));
                let local = c * 512 + I64Vec3::splat(256) - l0 * 32_768;
                let crust = matches!(stack(seed, l0).stratum_at(local, None), Stratum::Crust(_));
                let t = tissue_cell(seed, cache.tissue(), key);
                let pitch = t.lattice.pitch as f32;
                let org = [x, y, z].map(|v| (v * 512) as f64);
                for s in &t.structures {
                    for b in &s.parts {
                        let lo = [0, 1, 2].map(|i| org[i] + f64::from(b.lo[i]) * f64::from(pitch));
                        let hi = [0, 1, 2].map(|i| org[i] + f64::from(b.hi[i]) * f64::from(pitch));
                        mark(&grid, gdim, lo, hi);
                    }
                }
                let breath = t
                    .payoffs
                    .list
                    .iter()
                    .filter(|p| {
                        matches!(
                            p.kind,
                            worldgen::payoffs::PayoffKind::View(_)
                                | worldgen::payoffs::PayoffKind::Set(_)
                                | worldgen::payoffs::PayoffKind::Install(_)
                        )
                    })
                    .map(|p| {
                        let c = t.lattice.coords(p.slot as usize);
                        [0, 1, 2].map(|i| (c[i] as f32 + 0.5) * pitch)
                    })
                    .collect();
                let walks = t
                    .paths
                    .walks
                    .iter()
                    .map(|w| {
                        (
                            w.id,
                            w.points
                                .iter()
                                .map(|p| [p.x as f32, p.y as f32, p.z as f32])
                                .collect(),
                        )
                    })
                    .collect();
                CellData {
                    crust,
                    breath,
                    walks,
                }
            })
            .collect()
    });
    let mut g = Graph {
        cells: total,
        dim,
        data,
        pos: Vec::new(),
        cell: Vec::new(),
        kind: Vec::new(),
        key: Vec::new(),
        range: Vec::new(),
        adj: Vec::new(),
        grid,
        gdim,
    };
    g.build_nodes();
    g
}

impl Graph {
    fn add(&mut self, a: u32, b: u32) {
        let w = dist(self.pos[a as usize], self.pos[b as usize]);
        self.adj[a as usize].push((b, w));
        self.adj[b as usize].push((a, w));
    }

    /// Nodes along every walk, joined along it, then joined to the walks that pass near.
    fn build_nodes(&mut self) {
        let d = self.dim;
        let mut walk_of: Vec<u32> = Vec::new();
        let mut uid = 0u32;
        for k in 0..self.data.len() {
            let org = [
                (k as i64 % d * 512) as f32,
                (k as i64 / d % d * 512) as f32,
                (k as i64 / (d * d) * 512) as f32,
            ];
            let start = self.pos.len() as u32;
            for (id, pts) in &self.data[k].walks {
                for seg in pts.windows(2) {
                    let len = dist(seg[0], seg[1]);
                    let n = (len / STEP_M).ceil().max(1.0) as usize;
                    for i in 0..n {
                        let f = i as f32 / n as f32;
                        let p = [0, 1, 2].map(|a| org[a] + seg[0][a] + (seg[1][a] - seg[0][a]) * f);
                        self.pos.push(p);
                        self.cell.push(k as u32);
                        self.kind.push(*id);
                        walk_of.push(uid);
                        self.key.push(i == 0 && seg.as_ptr() == pts.as_ptr());
                    }
                }
                if let Some(&last) = pts.last() {
                    self.pos.push([0, 1, 2].map(|a| org[a] + last[a]));
                    self.cell.push(k as u32);
                    self.kind.push(*id);
                    walk_of.push(uid);
                    self.key.push(true);
                }
                uid += 1;
            }
            self.range.push((start, self.pos.len() as u32));
        }
        self.adj = vec![Vec::new(); self.pos.len()];
        for n in 1..self.pos.len() {
            if walk_of[n] == walk_of[n - 1] {
                self.add((n - 1) as u32, n as u32);
            }
        }
        // Joins: walks that pass within `JOIN_M`, found through a hash of 3 m cubes.
        let mut cubes: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (n, p) in self.pos.iter().enumerate() {
            let c = p.map(|v| (v / 3.0).floor() as i32);
            cubes.entry(c).or_default().push(n as u32);
        }
        let mut joins = Vec::new();
        for (n, p) in self.pos.iter().enumerate() {
            let c = p.map(|v| (v / 3.0).floor() as i32);
            let mut count = 0;
            'scan: for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(list) = cubes.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else {
                            continue;
                        };
                        for &o in list {
                            if (o as usize) > n
                                && walk_of[o as usize] != walk_of[n]
                                && dist(*p, self.pos[o as usize]) <= JOIN_M
                            {
                                joins.push((n as u32, o));
                                count += 1;
                                if count >= 4 {
                                    break 'scan;
                                }
                            }
                        }
                    }
                }
            }
        }
        for (a, b) in joins {
            self.add(a, b);
            self.key[a as usize] = true;
            self.key[b as usize] = true;
        }
    }

    fn cell_of(&self, node: u32) -> usize {
        self.cell[node as usize] as usize
    }

    fn in_core(&self, k: usize) -> bool {
        let d = self.dim;
        let (x, y, z) = (k as i64 % d, k as i64 / d % d, k as i64 / (d * d));
        let core = d / 4..d - d / 4;
        core.contains(&x) && core.contains(&y) && core.contains(&z)
    }

    /// Whether any of the 26 rays from `node`'s eye runs `LONG_M` without meeting a solid.
    fn has_long_view(&self, node: u32) -> bool {
        let p = self.pos[node as usize];
        let eye = [f64::from(p[0]), f64::from(p[1]) + 1.65, f64::from(p[2])];
        for dz in -1..=1i32 {
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let l = f64::from(dx * dx + dy * dy + dz * dz).sqrt();
                    let d = [f64::from(dx) / l, f64::from(dy) / l, f64::from(dz) / l];
                    let mut t = 4.0;
                    let mut hit = None;
                    while t < REACH_M {
                        let q = [eye[0] + d[0] * t, eye[1] + d[1] * t, eye[2] + d[2] * t];
                        if self.solid(q) {
                            hit = Some(t);
                            break;
                        }
                        t += GRID_M / 2.0;
                    }
                    // A ray that starts into solid is skipped.
                    if hit.is_some_and(|h| h < 6.0) {
                        continue;
                    }
                    if hit.is_none_or(|h| h >= LONG_M) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The walking distance from every node to the nearest of `targets`, in metres.
    fn distances(&self, targets: &[u32]) -> Vec<f32> {
        let mut d = vec![f32::INFINITY; self.pos.len()];
        let mut heap = BinaryHeap::new();
        for &t in targets {
            d[t as usize] = 0.0;
            heap.push((Reverse(0u64), t));
        }
        while let Some((Reverse(m), n)) = heap.pop() {
            if m as f32 / 1000.0 > d[n as usize] + 0.01 {
                continue;
            }
            for &(o, w) in &self.adj[n as usize] {
                let nd = d[n as usize] + w;
                if nd < d[o as usize] {
                    d[o as usize] = nd;
                    heap.push((Reverse((nd * 1000.0) as u64), o));
                }
            }
        }
        d
    }

    /// The nodes of the central crust cells (where the rays stay inside the block), thinned to
    /// about `want`.
    fn core_nodes(&self, want: usize) -> Vec<u32> {
        let all: Vec<u32> = (0..self.pos.len() as u32)
            .filter(|&n| {
                let k = self.cell_of(n);
                self.in_core(k) && self.data[k].crust && !self.adj[n as usize].is_empty()
            })
            .collect();
        let stride = (all.len() / want.max(1)).max(1);
        all.into_iter().step_by(stride).collect()
    }
}

pub struct Report {
    starts: usize,
    long_m: Vec<f32>,
    breath_m: Vec<f32>,
    vertical: f64,
    types_per_km: f64,
    three_links: f64,
    bots: usize,
    km: f64,
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
}

pub fn measure(g: &Graph, o: &Opts) -> Report {
    let starts = g.core_nodes(o.starts);
    // Long views: found for a thinned set of the central nodes, then walked to.
    let candidates = g.core_nodes(40_000);
    let long: Vec<u32> = candidates
        .par_iter()
        .copied()
        .filter(|&n| g.has_long_view(n))
        .collect();
    eprintln!(
        "{} of {} candidate nodes have a long view",
        long.len(),
        candidates.len()
    );
    let to_long = g.distances(&long);
    // Breathing space: the nodes near a payoff.
    let mut breath = Vec::new();
    for k in 0..g.data.len() {
        let org = {
            let d = g.dim;
            [
                (k as i64 % d * 512) as f32,
                (k as i64 / d % d * 512) as f32,
                (k as i64 / (d * d) * 512) as f32,
            ]
        };
        for b in &g.data[k].breath {
            let at = [org[0] + b[0], org[1] + b[1], org[2] + b[2]];
            // The nearest node of the cell within 24 m.
            let (lo, hi) = g.range[k];
            let best = (lo..hi)
                .map(|n| (n, dist(g.pos[n as usize], at)))
                .filter(|&(_, d)| d < 24.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((n, _)) = best {
                breath.push(n);
            }
        }
    }
    eprintln!("{} breathing-space nodes", breath.len());
    if let Some(&s0) = starts.first() {
        let reach = g.distances(&[s0]);
        let n = reach.iter().filter(|d| d.is_finite()).count();
        eprintln!(
            "from the first start {} of {} nodes are reachable",
            n,
            g.pos.len()
        );
    }
    let to_breath = g.distances(&breath);
    let cap = CAP_M as f32;
    let long_m = starts
        .iter()
        .map(|&n| to_long[n as usize].min(cap))
        .collect();
    let breath_m = starts
        .iter()
        .map(|&n| to_breath[n as usize].min(cap))
        .collect();
    // The walkers.
    let mut state = o.seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = move |n: usize| {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) % n.max(1) as u64) as usize
    };
    let (mut up, mut total, mut types_km, mut walked) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for _ in 0..o.bots {
        let mut at = starts[next(starts.len())];
        let (mut prev, mut walked_m, mut seen) = (u32::MAX, 0.0f64, BTreeSet::new());
        for _ in 0..200_000 {
            if walked_m >= o.bot_km * 1000.0 {
                break;
            }
            let nbrs = &g.adj[at as usize];
            if nbrs.is_empty() {
                break;
            }
            let choices: Vec<_> = nbrs.iter().filter(|&&(n, _)| n != prev).collect();
            let &(to, w) = if choices.is_empty() {
                &nbrs[0]
            } else {
                choices[next(choices.len())]
            };
            let (a, b) = (g.pos[at as usize], g.pos[to as usize]);
            up += f64::from((b[1] - a[1]).abs());
            total += f64::from(w);
            walked_m += f64::from(w);
            seen.insert(g.kind[to as usize]);
            prev = at;
            at = to;
        }
        types_km += seen.len() as f64;
        walked += walked_m / 1000.0;
    }
    let keys: Vec<usize> = (0..g.pos.len()).filter(|&n| g.key[n]).collect();
    let linked = keys.iter().filter(|&&n| g.adj[n].len() >= 3).count();
    Report {
        starts: starts.len(),
        long_m,
        breath_m,
        vertical: up / total.max(1.0),
        types_per_km: types_km / walked.max(1e-9),
        three_links: linked as f64 / keys.len().max(1) as f64,
        bots: o.bots,
        km: walked,
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mut l, mut b) = (self.long_m.clone(), self.breath_m.clone());
        let share = |v: &[f32]| {
            v.iter().filter(|&&x| x < CAP_M as f32).count() as f64 / v.len().max(1) as f64
        };
        writeln!(
            f,
            "  MET-04 walking distance to a 2 km sightline, median  {:>8.0} m   target 1500-2500   ({:.0} % of {} starts reach one within 20 km)",
            median(&mut l),
            share(&self.long_m) * 100.0,
            self.starts
        )?;
        writeln!(
            f,
            "  MET-05 walking distance to breathing space, median   {:>8.0} m   target <= 150      ({:.0} % reach one within 20 km)",
            median(&mut b),
            share(&self.breath_m) * 100.0
        )?;
        writeln!(
            f,
            "  MET-07 vertical share of walked distance             {:>8.3}     target >= 0.33     ({} walkers, {:.1} km)",
            self.vertical, self.bots, self.km
        )?;
        writeln!(
            f,
            "  MET-08 path types per km walked                      {:>8.1}     target >= 8",
            self.types_per_km
        )?;
        write!(
            f,
            "  MET-10 nodes with three or more links                {:>8.3}     target >= 0.40",
            self.three_links
        )
    }
}
