//! Connectivity validation of generated cells. Spec: `docs/spec/ports.md` §6 (Lemma D) and
//! `CHECKLIST.md` P0.9.
//!
//! `check_cell` builds a cell's nav graph from its parent's window plus everything L2 added and
//! tests each port against it. `BlockNav` links the graphs of neighbouring cells through their
//! shared ports. `check_random` runs `check_cell` over many cells on several threads.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use glam::I64Vec3;

use crate::cell::{CellKey, Dir6, FaceKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::l2::{CELL, l2_cell_in};
use crate::l2_port::{L1Cache, face_ports_in};
use crate::port::PortKind;
use crate::prim::{CellOutput, FeatureKind, Q};
use crate::seed::WorldSeed;
use crate::walk::{Nav, cell_window, plates_at_port};

/// A cell's nav graph and the data it was built from.
pub struct CellNav {
    pub cell: CellKey,
    pub output: CellOutput,
    pub nav: Nav,
    /// Plates whose source primitive came from L1 (the network N) have `source < from_l1`.
    pub from_l1: usize,
}

impl CellNav {
    pub fn build(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> Self {
        let output = l2_cell_in(seed, cache, cell);
        let c = cell.coords;
        let parent = cache.get(
            seed,
            CellKey::new(Level::L1, c.map(|v| floor_div_pow2(v, 3))),
        );
        let origin = [c.x, c.y, c.z].map(|v| Q::try_from(floor_mod_pow2(v, 3)).unwrap_or(0) * CELL);
        let mut prims = cell_window(&parent.primitives, origin, CELL);
        let from_l1 = prims.len();
        prims.extend(output.primitives.iter().copied());
        Self {
            cell,
            output,
            nav: Nav::build(&prims),
            from_l1,
        }
    }

    pub fn in_network(&self, plate: usize) -> bool {
        self.nav.plates[plate].source < self.from_l1
    }

    /// Plates a person can step onto from the opening of `port`, in this cell's frame.
    fn port_plates(&self, face: FaceKey, rect: &crate::prim::QRect) -> Vec<usize> {
        let plane = if face.owner == self.cell { CELL } else { 0 };
        plates_at_port(&self.nav, face.axis, plane, (rect.u0, rect.u1), rect.v0)
    }
}

/// What `check_cell` found.
#[derive(Debug, Clone, Default)]
pub struct CellReport {
    pub ports: u32,
    pub walkway_ports: u32,
    pub failures: Vec<String>,
}

/// Tests every non-lift port of `cell`: it has floor, and that floor reaches the network N
/// inside the cell. Also that nothing the cell emits leaves its bounds.
pub fn check_cell(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> CellReport {
    let cn = CellNav::build(seed, cache, cell);
    let mut report = CellReport::default();
    for p in &cn.output.ports {
        if matches!(p.kind, PortKind::LiftStop | PortKind::Entrance { .. }) {
            continue;
        }
        report.ports += 1;
        report.walkway_ports += u32::from(p.kind == PortKind::Walkway);
        let found = cn.port_plates(p.face, &p.rect);
        if found.is_empty() {
            report.failures.push(format!("no floor at {p:?}"));
            continue;
        }
        let reaches = found
            .iter()
            .any(|&i| (0..cn.nav.plates.len()).any(|j| cn.in_network(j) && cn.nav.connected(i, j)));
        if !reaches {
            report
                .failures
                .push(format!("{p:?} is cut off from the network"));
        }
    }
    for p in &cn.output.primitives {
        let bb = p.aabb();
        let inside = |v: Q| (0..=CELL).contains(&v);
        if ![bb.min.x, bb.min.y, bb.min.z, bb.max.x, bb.max.y, bb.max.z]
            .into_iter()
            .all(inside)
        {
            report.failures.push(format!("{p:?} leaves the cell"));
        }
    }
    report
}

/// Lemma A for one L1 cell: its deck and bridge surfaces and lift platforms form one component.
/// Returns a message per disconnected plate, empty when the cell passes.
pub fn check_l1(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> Vec<String> {
    let out = cache.get(seed, cell);
    let nav = Nav::build(&out.primitives);
    let network: Vec<usize> = (0..nav.plates.len())
        .filter(|&i| {
            let p = &nav.plates[i];
            p.lift || matches!(p.kind, FeatureKind::DECK | FeatureKind::BRIDGE)
        })
        .collect();
    let Some(&first) = network.first() else {
        return Vec::new();
    };
    network
        .iter()
        .filter(|&&i| !nav.connected(first, i))
        .map(|&i| format!("L1 plate {:?} is cut off", nav.plates[i].rect))
        .collect()
}

/// The nav graphs of a set of cells, joined through the ports on faces they share.
pub struct BlockNav {
    pub cells: Vec<CellNav>,
    offsets: Vec<usize>,
    parent: Vec<usize>,
    /// Ports that joined two cells, for tests.
    pub joins: u32,
}

fn find(p: &mut [usize], mut i: usize) -> usize {
    while p[i] != i {
        p[i] = p[p[i]];
        i = p[i];
    }
    i
}

impl BlockNav {
    pub fn build(seed: WorldSeed, cells: &[CellKey]) -> Self {
        let cache = L1Cache::default();
        let cells: Vec<CellNav> = cells
            .iter()
            .map(|&c| CellNav::build(seed, &cache, c))
            .collect();
        let mut offsets = Vec::with_capacity(cells.len());
        let mut total = 0;
        for c in &cells {
            offsets.push(total);
            total += c.nav.plates.len();
        }
        let mut parent: Vec<usize> = (0..total).collect();
        for (ci, c) in cells.iter().enumerate() {
            for (i, &rep) in c.nav.component.iter().enumerate() {
                let (a, b) = (find(&mut parent, offsets[ci] + i), offsets[ci] + rep);
                parent[a.max(b)] = a.min(b);
            }
        }
        let index: HashMap<CellKey, usize> =
            cells.iter().enumerate().map(|(i, c)| (c.cell, i)).collect();
        let mut block = Self {
            cells,
            offsets,
            parent,
            joins: 0,
        };
        for ai in 0..block.cells.len() {
            for dir in [Dir6::PosX, Dir6::PosY, Dir6::PosZ] {
                let a = block.cells[ai].cell;
                let Ok(b) = a.neighbor(dir) else { continue };
                if let Some(&bi) = index.get(&b) {
                    block.join_faces(seed, &cache, ai, bi, FaceKey::of(a, dir).ok());
                }
            }
        }
        block
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (find(&mut self.parent, a), find(&mut self.parent, b));
        self.parent[ra.max(rb)] = ra.min(rb);
    }

    fn join_faces(
        &mut self,
        seed: WorldSeed,
        cache: &L1Cache,
        ai: usize,
        bi: usize,
        face: Option<FaceKey>,
    ) {
        let Some(face) = face else { return };
        for p in face_ports_in(seed, cache, face) {
            let (pa, pb): (Vec<usize>, Vec<usize>) = if p.kind == PortKind::LiftStop {
                let foot = |c: &CellNav| -> Vec<usize> {
                    (0..c.nav.plates.len())
                        .filter(|&i| {
                            let pl = &c.nav.plates[i];
                            pl.lift
                                && pl.rect.x0 < p.rect.u1
                                && p.rect.u0 < pl.rect.x1
                                && pl.rect.z0 < p.rect.v1
                                && p.rect.v0 < pl.rect.z1
                        })
                        .collect()
                };
                (foot(&self.cells[ai]), foot(&self.cells[bi]))
            } else {
                (
                    self.cells[ai].port_plates(face, &p.rect),
                    self.cells[bi].port_plates(face, &p.rect),
                )
            };
            if pa.is_empty() || pb.is_empty() {
                continue;
            }
            self.joins += 1;
            for &i in &pa {
                for &j in &pb {
                    let (x, y) = (self.offsets[ai] + i, self.offsets[bi] + j);
                    self.union(x, y);
                }
            }
        }
    }

    /// Whether plate `i` of cell `ci` and plate `j` of cell `cj` are connected in the block.
    pub fn connected(&mut self, (ci, i): (usize, usize), (cj, j): (usize, usize)) -> bool {
        let (a, b) = (self.offsets[ci] + i, self.offsets[cj] + j);
        find(&mut self.parent, a) == find(&mut self.parent, b)
    }

    /// Whether plate `i` of cell `ci` is connected to any plate of the network N in the block.
    pub fn reaches_backbone(&mut self, ci: usize, i: usize) -> bool {
        let me = self.offsets[ci] + i;
        let root = find(&mut self.parent, me);
        for cj in 0..self.cells.len() {
            for j in 0..self.cells[cj].nav.plates.len() {
                if self.cells[cj].in_network(j)
                    && find(&mut self.parent, self.offsets[cj] + j) == root
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Result of `check_random`.
#[derive(Debug, Default)]
pub struct Summary {
    pub cells: u32,
    pub ports: u32,
    pub walkway_ports: u32,
    /// `(cell, message)` for every failure.
    pub failures: Vec<(CellKey, String)>,
    pub elapsed: Duration,
}

/// Checks `count` L2 cells drawn from `seed` and `sample_seed` across a region of `extent`
/// L2 cells per axis around the origin, on `threads` threads. Cells are drawn in runs of the
/// 512 children of one L1 cell, so that the L1 cache pays off. Failures are listed in cell
/// order, so a run is reproducible whatever the thread count.
pub fn check_random(
    seed: WorldSeed,
    sample_seed: u64,
    count: u32,
    extent: i64,
    threads: usize,
) -> Summary {
    let start = Instant::now();
    let runs = count.div_ceil(512);
    let run_cells = |run: u32| -> Vec<CellKey> {
        let h = |salt: u64| {
            i64::try_from(
                hash(
                    sample_seed,
                    Level::L1,
                    I64Vec3::new(i64::from(run), 0, 0),
                    salt,
                ) % u64::try_from(extent / 8).unwrap_or(1).max(1),
            )
            .unwrap_or(0)
        };
        let half = extent / 16;
        let parent = I64Vec3::new(h(1) - half, h(2) % 3 - 1, h(3) - half);
        (0..512i64)
            .map(|k| {
                CellKey::new(
                    Level::L2,
                    parent * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                )
            })
            .collect()
    };
    let threads = threads.max(1);
    let mut results: Vec<(u32, Vec<(CellKey, CellReport)>)> = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut out = Vec::new();
                    for run in (0..runs).filter(|r| usize::try_from(*r).unwrap_or(0) % threads == t)
                    {
                        let cache = L1Cache::default();
                        let cells = run_cells(run);
                        let parent =
                            CellKey::new(Level::L1, cells[0].coords.map(|v| floor_div_pow2(v, 3)));
                        let mut reports: Vec<(CellKey, CellReport)> = Vec::new();
                        let l1 = check_l1(seed, &cache, parent);
                        if !l1.is_empty() {
                            reports.push((
                                parent,
                                CellReport {
                                    failures: l1,
                                    ..CellReport::default()
                                },
                            ));
                        }
                        reports.extend(cells.into_iter().map(|c| (c, check_cell(seed, &cache, c))));
                        out.push((run, reports));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    results.sort_by_key(|r| r.0);
    let mut summary = Summary::default();
    for (_, reports) in results {
        for (cell, r) in reports {
            summary.cells += 1;
            summary.ports += r.ports;
            summary.walkway_ports += r.walkway_ports;
            summary
                .failures
                .extend(r.failures.into_iter().map(|m| (cell, m)));
        }
    }
    summary.elapsed = start.elapsed();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(origin: I64Vec3) -> Vec<CellKey> {
        let mut cells = Vec::new();
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    cells.push(CellKey::new(Level::L2, origin + I64Vec3::new(x, y, z)));
                }
            }
        }
        cells
    }

    /// P0.9.3: in 3 × 3 × 3 blocks of L2 cells, every port of every cell reaches the network,
    /// and ports shared by two cells really join their graphs.
    #[test]
    #[ignore = "reads the v6 L1 (lift core, decks, bridges), replaced in version 7; rebuilt or deleted with PW.3"]
    fn blocks_are_reachable_from_the_backbone() {
        let seed = WorldSeed(0x0903);
        let (mut ports_checked, mut joins) = (0, 0);
        let cache = L1Cache::default();
        let mut starts = Vec::new();
        for parent in [
            (3, 0, 1),
            (-7, 2, 4),
            (11, 1, -5),
            (-2, 0, -9),
            (20, 0, 7),
            (-30, 1, 2),
        ] {
            let key = CellKey::new(Level::L1, I64Vec3::new(parent.0, parent.1, parent.2));
            let out = cache.get(seed, key);
            let Some(top) = out
                .primitives
                .iter()
                .filter(|p| p.kind == crate::prim::FeatureKind::DECK)
                .map(|p| p.aabb().max.y)
                .next()
            else {
                continue;
            };
            let cy = i64::from(top / CELL).clamp(1, 6) - 1;
            // Inside the L1 cell, and straddling its +x +z corner.
            for (dx, dz) in [(2, 2), (6, 6)] {
                starts.push(key.coords * 8 + I64Vec3::new(dx, cy, dz));
            }
        }
        for origin in starts {
            let mut b = BlockNav::build(seed, &block(origin));
            joins += b.joins;
            for ci in 0..b.cells.len() {
                let ports: Vec<_> = b.cells[ci].output.ports.clone();
                for p in ports
                    .iter()
                    .filter(|p| !matches!(p.kind, PortKind::LiftStop | PortKind::Entrance { .. }))
                {
                    let found = b.cells[ci].port_plates(p.face, &p.rect);
                    assert!(
                        !found.is_empty(),
                        "{:?}: no floor at {p:?}",
                        b.cells[ci].cell
                    );
                    assert!(
                        found.iter().any(|&pl| b.reaches_backbone(ci, pl)),
                        "{:?}: {p:?} does not reach the backbone",
                        b.cells[ci].cell
                    );
                    ports_checked += 1;
                }
            }
        }
        assert!(
            ports_checked > 100 && joins > 50,
            "{ports_checked} ports, {joins} joins"
        );
    }

    /// P0.9.4: a random sample passes, in CI time.
    #[test]
    #[ignore = "reads the v6 L1 (lift core, decks, bridges), replaced in version 7; rebuilt or deleted with PW.3"]
    fn random_cells_pass() {
        let count = if cfg!(debug_assertions) {
            2_048
        } else {
            10_240
        };
        let threads = thread::available_parallelism().map_or(2, usize::from);
        let s = check_random(WorldSeed(0x0904), 7, count, 4096, threads);
        assert!(
            s.failures.is_empty(),
            "{} failures, first: {:?}",
            s.failures.len(),
            s.failures.first()
        );
        assert!(
            s.cells >= count && s.ports > 100,
            "{} cells, {} ports",
            s.cells,
            s.ports
        );
        assert!(s.elapsed < Duration::from_mins(1), "{:?}", s.elapsed);
        // The same sample on one thread gives the same totals.
        let one = check_random(WorldSeed(0x0904), 7, 512, 4096, 1);
        let many = check_random(WorldSeed(0x0904), 7, 512, 4096, 3);
        assert_eq!(
            (one.cells, one.ports, one.walkway_ports),
            (many.cells, many.ports, many.walkway_ports)
        );
    }
}
