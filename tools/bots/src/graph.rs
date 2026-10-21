//! The nav graph of a block of L2 cells and what the walkers measure on it. The nodes are the
//! lattice slots of the tissue's route (its segments, whose ends are shared with the neighbours),
//! the edges join consecutive slots of a segment, and the route slots on the two sides of an L2
//! face are joined where they face each other: the designed network, which the generator
//! guarantees to be connected, with the path type of each segment.

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

const GRID_M: f64 = 16.0;
/// A long view is a ray this long (MET-04), the cap on the search (20 km) and the rays' reach.
const LONG_M: f64 = 2000.0;
const CAP_M: f64 = 20_000.0;
const REACH_M: f64 = 4000.0;

/// A segment: its path type, whether a near detail looks out from it (NEAR-02..05) and its slots
/// (index and centre).
type Seg = (u8, bool, Vec<(u16, [f32; 3])>);

struct CellData {
    crust: bool,
    /// The slots of every segment (index and cell-local centre in metres, which a turned
    /// lattice moves), with its path type.
    segs: Vec<Seg>,
    /// The slots of the payoffs that show open space or a place.
    breath: Vec<[f32; 3]>,
    /// The ports on the cell's faces: the axis, whether it is the high face, the point in
    /// cell-local metres and the slot the port routes from.
    ports: Vec<(u8, bool, [f32; 3], u16)>,
    /// Cell-local centres of the Between overlooks (HUB-09) and of the shortcuts: lift
    /// segments and tram ports.
    overlooks: Vec<[f32; 3]>,
    shortcuts: Vec<[f32; 3]>,
}

pub struct Graph {
    pub cells: usize,
    dim: i64,
    data: Vec<CellData>,
    pos: Vec<[f32; 3]>,
    /// The cell of each node.
    cell: Vec<u32>,
    /// The first node of each cell and one past its last, and the node of each slot.
    range: Vec<(u32, u32)>,
    slot_node: Vec<HashMap<u16, u32>>,
    /// Neighbours with the distance and the path type (`CROSS` over a face).
    adj: Vec<Vec<(u32, f32, u8)>>,
    grid: Vec<AtomicU64>,
    gdim: usize,
}

/// The type of an edge that crosses an L2 face.
const CROSS: u8 = 255;

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

pub fn load(o: &Opts) -> anyhow::Result<Graph> {
    let seed = WorldSeed(o.seed);
    let dim = o.blocks * 8;
    let low = o.low * 8;
    let gdim = (dim * 512 / GRID_M as i64) as usize;
    let words = gdim * gdim * gdim / 64 + 1;
    let grid: Vec<AtomicU64> = (0..words).map(|_| AtomicU64::new(0)).collect();
    let cache = L1Cache::default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(o.threads.max(1))
        .build()?;
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
                    .map(|p| p.slot as u16)
                    // Local places (light wells, atria, thin spots, ...) show open space too.
                    .chain(
                        t.locals
                            .iter()
                            .filter_map(|&(_, si)| t.structures[usize::from(si)].slots.first())
                            .map(|&i| i as u16),
                    )
                    .map(|i| {
                        let c = t.lattice.centre(usize::from(i));
                        [c.x as f32, c.y as f32, c.z as f32]
                    })
                    .collect();
                // The path type of each segment (PH and PV ids); a segment without one is 0.
                let kind_of = |si: usize| {
                    t.paths
                        .chosen
                        .iter()
                        .find(|c| usize::from(c.0) == si)
                        .map_or(0, |c| c.1)
                };
                let looks_out = |si: usize| {
                    t.paths
                        .mods
                        .iter()
                        .find(|m| usize::from(m.0) == si)
                        .is_some_and(|m| (2..=5).contains(&m.1.near))
                };
                let segs = t
                    .segments
                    .iter()
                    .enumerate()
                    .map(|(si, seg)| {
                        (
                            kind_of(si),
                            looks_out(si),
                            seg.slots
                                .iter()
                                .map(|&i| {
                                    let c = t.lattice.centre(i as usize);
                                    (i as u16, [c.x as f32, c.y as f32, c.z as f32])
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let ports = t
                    .anchors
                    .iter()
                    .filter_map(|a| {
                        let worldgen::tissue::AnchorKind::Port { face, .. } = a.kind else {
                            return None;
                        };
                        let o = face.offset();
                        let axis = [o.x, o.y, o.z].iter().position(|&v| v != 0)?;
                        Some((
                            axis as u8,
                            [o.x, o.y, o.z][axis] > 0,
                            a.point.map(|v| v as f32),
                            a.slot as u16,
                        ))
                    })
                    .collect();
                let centre = |slot: u32| {
                    let c = t.lattice.centre(slot as usize);
                    [c.x as f32, c.y as f32, c.z as f32]
                };
                let overlooks = t
                    .hubs
                    .iter()
                    .filter(|&&(id, _)| id == 9)
                    .filter_map(|&(_, si)| t.structures[usize::from(si)].slots.first())
                    .map(|&i| centre(i))
                    .collect();
                let mut shortcuts: Vec<[f32; 3]> = t
                    .lift_segments
                    .iter()
                    .filter_map(|&si| t.segments.get(usize::from(si)))
                    .filter_map(|seg| seg.slots.first())
                    .map(|&i| centre(i))
                    .collect();
                shortcuts.extend(
                    t.anchors
                        .iter()
                        .filter(|a| {
                            matches!(
                                a.kind,
                                worldgen::tissue::AnchorKind::Port {
                                    kind: worldgen::l1_artery::ArteryType::TramTube,
                                    ..
                                }
                            )
                        })
                        .map(|a| centre(a.slot)),
                );
                CellData {
                    crust,
                    segs,
                    breath,
                    ports,
                    overlooks,
                    shortcuts,
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
        range: Vec::new(),
        slot_node: Vec::new(),
        adj: Vec::new(),
        grid,
        gdim,
    };
    g.build_nodes();
    Ok(g)
}

impl Graph {
    fn add(&mut self, a: u32, b: u32, ty: u8) {
        let w = dist(self.pos[a as usize], self.pos[b as usize]);
        self.adj[a as usize].push((b, w, ty));
        self.adj[b as usize].push((a, w, ty));
    }

    fn org(&self, k: usize) -> [f32; 3] {
        let d = self.dim;
        [
            (k as i64 % d * 512) as f32,
            (k as i64 / d % d * 512) as f32,
            (k as i64 / (d * d) * 512) as f32,
        ]
    }

    /// A node for every slot of every segment, edges along the segments and over the faces.
    fn build_nodes(&mut self) {
        let d = self.dim;
        for k in 0..self.data.len() {
            let org = self.org(k);
            let start = self.pos.len() as u32;
            let mut map: HashMap<u16, u32> = HashMap::new();
            for (_, _, slots) in &self.data[k].segs {
                for &(i, c) in slots {
                    map.entry(i).or_insert_with(|| {
                        self.pos.push([0, 1, 2].map(|a| org[a] + c[a]));
                        self.cell.push(k as u32);
                        (self.pos.len() - 1) as u32
                    });
                }
            }
            self.range.push((start, self.pos.len() as u32));
            self.slot_node.push(map);
        }
        self.adj = vec![Vec::new(); self.pos.len()];
        for k in 0..self.data.len() {
            for si in 0..self.data[k].segs.len() {
                let ty = self.data[k].segs[si].0;
                let ids: Vec<u16> = self.data[k].segs[si].2.iter().map(|s| s.0).collect();
                for w in ids.windows(2) {
                    let (a, b) = (self.slot_node[k][&w[0]], self.slot_node[k][&w[1]]);
                    if a != b && !self.adj[a as usize].iter().any(|e| e.0 == b) {
                        self.add(a, b, ty);
                    }
                }
            }
        }
        // The ports on the two sides of a face that meet: the same point within 4 m.
        let mut found = Vec::new();
        for z in 0..d {
            for y in 0..d {
                for x in 0..d {
                    let a = ((z * d + y) * d + x) as usize;
                    for (axis, step) in [(0usize, (1, 0, 0)), (1, (0, 1, 0)), (2, (0, 0, 1))] {
                        let (nx, ny, nz) = (x + step.0, y + step.1, z + step.2);
                        if nx >= d || ny >= d || nz >= d {
                            continue;
                        }
                        let b = ((nz * d + ny) * d + nx) as usize;
                        for pa in self.data[a]
                            .ports
                            .iter()
                            .filter(|p| p.0 as usize == axis && p.1)
                        {
                            for pb in self.data[b]
                                .ports
                                .iter()
                                .filter(|p| p.0 as usize == axis && !p.1)
                            {
                                let across = (0..3)
                                    .filter(|&i| i != axis)
                                    .all(|i| (pa.2[i] - pb.2[i]).abs() <= 4.0);
                                if !across {
                                    continue;
                                }
                                if let (Some(&na), Some(&nb)) =
                                    (self.slot_node[a].get(&pa.3), self.slot_node[b].get(&pb.3))
                                {
                                    found.push((na, nb));
                                }
                            }
                        }
                    }
                }
            }
        }
        for (na, nb) in found {
            self.add(na, nb, CROSS);
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
            for &(o, w, _) in &self.adj[n as usize] {
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
    three_of_junctions: f64,
    /// Overlooks passed per km walked, overlooks in the block, and the share with a lift or
    /// tram within 300 m of walking.
    overlooks_per_km: f64,
    overlooks: usize,
    shortcut_share: f64,
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
        let org = g.org(k);
        let (lo, hi) = g.range[k];
        // Every slot of a segment with a view detail (NEAR-02..05) is near breathing space.
        for (_, looks_out, slots) in &g.data[k].segs {
            if *looks_out {
                for (i, _) in slots {
                    if let Some(&n) = g.slot_node[k].get(i) {
                        breath.push(n);
                    }
                }
            }
        }
        for c in &g.data[k].breath {
            let at = [org[0] + c[0], org[1] + c[1], org[2] + c[2]];
            // The nearest route node of the cell within 48 m (a balcony hangs beside a leaf).
            let best = (lo..hi)
                .map(|n| (n, dist(g.pos[n as usize], at)))
                .filter(|&(_, d)| d <= 48.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((n, _)) = best {
                breath.push(n);
            }
        }
    }
    eprintln!("{} breathing-space nodes", breath.len());
    let cross = g.adj.iter().flatten().filter(|e| e.2 == CROSS).count() / 2;
    eprintln!("{cross} links over faces");
    {
        // The components of the graph: how much of it a walker can reach from where it stands.
        let mut comp = vec![u32::MAX; g.pos.len()];
        let mut sizes = Vec::new();
        for n in 0..g.pos.len() {
            if comp[n] != u32::MAX {
                continue;
            }
            let id = sizes.len() as u32;
            let mut stack = vec![n as u32];
            comp[n] = id;
            let mut size = 0usize;
            while let Some(m) = stack.pop() {
                size += 1;
                for &(o, _, _) in &g.adj[m as usize] {
                    if comp[o as usize] == u32::MAX {
                        comp[o as usize] = id;
                        stack.push(o);
                    }
                }
            }
            sizes.push(size);
        }
        let mut sorted = sizes.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        eprintln!(
            "{} components; the largest holds {} of {} nodes; the next five {:?}",
            sizes.len(),
            sorted[0],
            g.pos.len(),
            &sorted[1..sorted.len().min(6)]
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
    // The Between overlooks and the shortcuts (lifts, trams) as nodes.
    let near_node = |k: usize, c: &[f32; 3]| {
        let org = g.org(k);
        let at = [org[0] + c[0], org[1] + c[1], org[2] + c[2]];
        let (lo, hi) = g.range[k];
        (lo..hi)
            .map(|n| (n, dist(g.pos[n as usize], at)))
            .filter(|&(_, d)| d <= 48.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, _)| n)
    };
    let (mut overlooks, mut shortcuts) = (Vec::new(), Vec::new());
    for k in 0..g.data.len() {
        overlooks.extend(g.data[k].overlooks.iter().filter_map(|c| near_node(k, c)));
        shortcuts.extend(g.data[k].shortcuts.iter().filter_map(|c| near_node(k, c)));
    }
    let to_shortcut = g.distances(&shortcuts);
    {
        let mut d: Vec<f32> = overlooks
            .iter()
            .map(|&n| to_shortcut[n as usize].min(CAP_M as f32))
            .collect();
        eprintln!(
            "{} shortcut nodes; the walking distance from an overlook to the nearest, median {:.0} m",
            shortcuts.len(),
            median(&mut d)
        );
    }
    let with_shortcut = overlooks
        .iter()
        .filter(|&&n| to_shortcut[n as usize] <= 300.0)
        .count();
    let overlook_set: std::collections::HashSet<u32> = overlooks.iter().copied().collect();
    let mut overlooks_passed = 0usize;
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
        let mut passed = std::collections::HashSet::new();
        for _ in 0..200_000 {
            if walked_m >= o.bot_km * 1000.0 {
                break;
            }
            let nbrs = &g.adj[at as usize];
            if nbrs.is_empty() {
                break;
            }
            let choices: Vec<_> = nbrs.iter().filter(|&&(n, _, _)| n != prev).collect();
            let &(to, w, ty) = if choices.is_empty() {
                &nbrs[0]
            } else {
                choices[next(choices.len())]
            };
            let (a, b) = (g.pos[at as usize], g.pos[to as usize]);
            up += f64::from((b[1] - a[1]).abs());
            total += f64::from(w);
            walked_m += f64::from(w);
            if ty != CROSS {
                seen.insert(ty);
            }
            if overlook_set.contains(&to) {
                passed.insert(g.cell_of(to));
            }
            prev = at;
            at = to;
        }
        overlooks_passed += passed.len();
        types_km += seen.len() as f64;
        walked += walked_m / 1000.0;
    }
    let linked = (0..g.pos.len()).filter(|&n| g.adj[n].len() >= 3).count();
    let ends = (0..g.pos.len()).filter(|&n| g.adj[n].len() != 2).count();
    Report {
        starts: starts.len(),
        long_m,
        breath_m,
        vertical: up / total.max(1.0),
        types_per_km: types_km / walked.max(1e-9),
        three_links: linked as f64 / g.pos.len().max(1) as f64,
        three_of_junctions: linked as f64 / ends.max(1) as f64,
        bots: o.bots,
        km: walked,
        overlooks_per_km: overlooks_passed as f64 / walked.max(1e-9),
        overlooks: overlooks.len(),
        shortcut_share: with_shortcut as f64 / overlooks.len().max(1) as f64,
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
        writeln!(
            f,
            "  MET-10 nodes with three or more links                {:>8.3}     target >= 0.40     (of all route slots)",
            self.three_links
        )?;
        writeln!(
            f,
            "  MET-10 ... of the junctions and ends                 {:>8.3}     target >= 0.40     (nodes whose degree is not 2)",
            self.three_of_junctions
        )?;
        writeln!(
            f,
            "  HOR-13 Between overlooks passed per km walked        {:>8.3}     (rare: {} in the block)",
            self.overlooks_per_km, self.overlooks
        )?;
        write!(
            f,
            "  HOR-14 overlooks with a lift or tram within 300 m    {:>8.2}     (share of the overlooks)",
            self.shortcut_share
        )
    }
}
