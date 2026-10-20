//! Worker threads that generate and mesh chunks, so the main thread never does either.
//!
//! Jobs wait in a priority queue. The streamer re-ranks and prunes it as the camera moves
//! ([`MeshPool::retarget`]); a job already running is flagged stale and its result is dropped.

use std::collections::HashMap;

use crate::fasthash::FastMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::cell::{CellKey, Level, floor_div_pow2};
use worldgen::coord::{CoordError, WorldPos};
use worldgen::l1::l1_far_primitives;
use worldgen::l2::l2_build;
use worldgen::l2_port::L1Cache;
use worldgen::l3_web::{WebL3, web_context};
use worldgen::port::{Port, PortKind};
use worldgen::prim::{PrimFlags, Primitive, QAabb, QVec3, Shape};
use worldgen::seed::WorldSeed;

use crate::chunkmesh::mesh_primitives;
use crate::farinst::{FarData, far_instances};
use crate::kit::{ModuleBatch, expand};
use crate::mesh::Mesh;
use crate::stream::{ChunkId, Detail};
use crate::zone;

/// A chunk's CPU-side mesh, ready for upload.
pub struct ChunkMesh {
    pub id: ChunkId,
    /// World position of the cell's minimum corner; vertices are relative to it.
    pub origin: WorldPos,
    pub mesh: Mesh,
    /// Kept for collision; the streamer takes it on upload.
    pub prims: Vec<Primitive>,
    /// Debug builds, L1 only: digest of the far-visible primitives this chunk was built from.
    pub far_digest: Option<u64>,
    pub gen_ms: f32,
    pub mesh_ms: f32,
    /// Far L1 chunks carry instances instead of a mesh.
    pub far: Option<FarData>,
    /// L4 modules to draw instanced with this chunk: railings of an L2 cell, everything an L3
    /// block's interior holds.
    pub modules: Option<ModuleBatch>,
    /// Boxes the analytic AO measures: `(centre, half size)` in metres relative to `origin`.
    pub ao_boxes: Vec<(Vec3, Vec3)>,
}

impl ChunkMesh {
    /// Bytes this chunk occupies on the GPU.
    pub fn upload_bytes(&self) -> u64 {
        match &self.far {
            Some(far) => far.bytes(),
            None => {
                crate::streamer::chunk_bytes(&self.mesh.vertices, &self.mesh.indices)
                    + self.modules.as_ref().map_or(0, ModuleBatch::bytes)
            }
        }
    }
}

pub type JobResult = Result<ChunkMesh, (ChunkId, CoordError)>;

/// Counters for the overlay and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    pub queued: usize,
    pub running: usize,
    /// Jobs dropped before they ran, or whose results were discarded, because they went stale.
    pub cancelled: u64,
    pub completed: u64,
}

struct Queued {
    priority: f32,
}

#[derive(Default)]
struct State {
    queue: HashMap<ChunkId, Queued>,
    running: HashMap<ChunkId, Arc<AtomicBool>>,
    shutdown: bool,
}

struct Shared {
    seed: WorldSeed,
    state: Mutex<State>,
    wake: Condvar,
    cancelled: AtomicU64,
    completed: AtomicU64,
    /// The L1 outputs and tissue parents, shared by every worker (one copy, built once), and
    /// the jobs run since it was made; it starts afresh every `CACHE_JOBS` jobs to bound memory.
    cache: Mutex<(Arc<L1Cache>, u32)>,
}

impl Shared {
    fn cache(&self) -> Arc<L1Cache> {
        let mut c = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.1 >= CACHE_JOBS {
            *c = (Arc::new(L1Cache::default()), 0);
        }
        c.1 += 1;
        Arc::clone(&c.0)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means a worker panicked; the maps stay consistent.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub struct MeshPool {
    shared: Arc<Shared>,
    results: Receiver<JobResult>,
    workers: Vec<JoinHandle<()>>,
}

/// Jobs a worker runs before dropping its L1 cache, which only grows.
const CACHE_JOBS: u32 = 256;

impl MeshPool {
    /// Spawns `threads` workers (at least one).
    pub fn new(seed: WorldSeed, threads: usize) -> Self {
        let shared = Arc::new(Shared {
            cache: Mutex::new((Arc::new(L1Cache::default()), 0)),
            seed,
            state: Mutex::default(),
            wake: Condvar::new(),
            cancelled: AtomicU64::new(0),
            completed: AtomicU64::new(0),
        });
        let (tx, results) = unbounded();
        let workers = (0..threads.max(1))
            .filter_map(|i| {
                let (shared, tx) = (Arc::clone(&shared), tx.clone());
                thread::Builder::new()
                    .name(format!("mesh-{i}"))
                    .spawn(move || worker(&shared, &tx))
                    .inspect_err(|e| tracing::error!("cannot spawn mesh worker {i}: {e}"))
                    .ok()
            })
            .collect();
        Self {
            shared,
            results,
            workers,
        }
    }

    /// A pool sized to leave two cores for the main and render threads.
    pub fn with_default_threads(seed: WorldSeed) -> Self {
        let cores = thread::available_parallelism().map_or(2, usize::from);
        Self::new(seed, cores.saturating_sub(2).max(1))
    }

    /// Queues `id`. Does nothing if it is already queued or running.
    pub fn submit(&self, id: ChunkId, priority: f32) {
        let mut st = self.shared.lock();
        if st.running.contains_key(&id) {
            return;
        }
        st.queue.entry(id).or_insert(Queued { priority });
        drop(st);
        self.shared.wake.notify_one();
    }

    /// Drops every queued or running job whose id is not in `wanted` and re-ranks the rest.
    pub fn retarget(&self, wanted: &FastMap<ChunkId, f32>) {
        let mut st = self.shared.lock();
        let before = st.queue.len();
        st.queue.retain(|id, _| wanted.contains_key(id));
        let mut dropped = (before - st.queue.len()) as u64;
        for (id, q) in &mut st.queue {
            if let Some(p) = wanted.get(id) {
                q.priority = *p;
            }
        }
        for (id, stale) in &st.running {
            if !wanted.contains_key(id) && !stale.swap(true, Ordering::Relaxed) {
                dropped += 1;
            }
        }
        drop(st);
        self.shared.cancelled.fetch_add(dropped, Ordering::Relaxed);
    }

    pub fn stats(&self) -> PoolStats {
        let st = self.shared.lock();
        PoolStats {
            queued: st.queue.len(),
            running: st
                .running
                .values()
                .filter(|s| !s.load(Ordering::Relaxed))
                .count(),
            cancelled: self.shared.cancelled.load(Ordering::Relaxed),
            completed: self.shared.completed.load(Ordering::Relaxed),
        }
    }

    /// Whether `id` is queued or running and not stale.
    pub fn is_pending(&self, id: &ChunkId) -> bool {
        let st = self.shared.lock();
        st.queue.contains_key(id)
            || st
                .running
                .get(id)
                .is_some_and(|s| !s.load(Ordering::Relaxed))
    }

    /// A finished mesh, if any. Never blocks.
    pub fn try_recv(&self) -> Option<JobResult> {
        self.results.try_recv().ok()
    }

    /// Waits for the next finished mesh. For tests and tools.
    pub fn recv(&self) -> Option<JobResult> {
        self.results.recv().ok()
    }
}

impl Drop for MeshPool {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker(shared: &Shared, tx: &Sender<JobResult>) {
    let mut parents: HashMap<CellKey, WebL3> = HashMap::new();
    let mut ran = 0u32;
    loop {
        let (id, stale) = {
            let mut st = shared.lock();
            loop {
                if st.shutdown {
                    return;
                }
                let best = st
                    .queue
                    .iter()
                    .min_by(|a, b| a.1.priority.total_cmp(&b.1.priority).then(a.0.cmp(b.0)))
                    .map(|(id, _)| *id);
                if let Some(id) = best {
                    st.queue.remove(&id);
                    let stale = Arc::new(AtomicBool::new(false));
                    st.running.insert(id, Arc::clone(&stale));
                    break (id, stale);
                }
                st = shared
                    .wake
                    .wait(st)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        if ran >= CACHE_JOBS {
            parents.clear();
            ran = 0;
        }
        ran += 1;
        let cache = shared.cache();
        let result = build(shared.seed, &cache, &mut parents, id, &stale);
        shared.lock().running.remove(&id);
        if let Some(r) = result {
            shared.completed.fetch_add(1, Ordering::Relaxed);
            if tx.send(r).is_err() {
                return;
            }
        }
    }
}

/// Digest of the far-visible primitives of an L1 chunk: all of a far chunk's, the flagged subset
/// of a full chunk's. Equal digests for the same cell mean both bands draw the same structure.
fn far_digest(id: ChunkId, prims: &[Primitive]) -> Option<u64> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    if id.key.level != Level::L1 {
        return None;
    }
    let mut h = DefaultHasher::new();
    for p in prims
        .iter()
        .filter(|p| id.detail == Detail::Far || p.flags.contains(PrimFlags::FAR_VISIBLE))
    {
        p.hash(&mut h);
    }
    Some(h.finish())
}

/// Whether L3 fills the interior parts of L2 structures (the Web L3, PW.4). Those parts are
/// hollow shells at L2, drawn and collided as they are.
pub const L3_ENABLED: bool = true;

/// The mid band draws an L2 cell's exterior masses only: the `ENVELOPE` primitives, which are
/// exactly what the finer levels refine, so the silhouette matches the full chunk's.
pub fn mid_primitives(mut prims: Vec<Primitive>) -> Vec<Primitive> {
    // The solid masses, and the whole box of each hollow part in place of its six slabs.
    prims.retain(|p| {
        p.flags.contains(PrimFlags::MID_ONLY)
            || (p.flags.contains(PrimFlags::ENVELOPE) && !p.flags.contains(PrimFlags::INTERIOR))
    });
    prims
}

/// Mid-band budgets (P3.2.1): resident triangles over all mid chunks, and per chunk. Measured
/// at 0.8k to 5.4k and 72 on the Phase 2 generator; the headroom is for denser L2 content.
#[cfg(test)]
const MID_BAND_TRIANGLE_BUDGET: usize = 200_000;
#[cfg(test)]
const MID_CELL_TRIANGLE_BUDGET: usize = 1_000;

/// How far an envelope shell sits inside the envelope faces, and how thick it is, in quanta. The
/// inset keeps it behind the flush exterior walls of an L3 block.
const SHELL_INSET: i32 = 2;
const SHELL_THICKNESS: i32 = 6;

fn solid_box(min: [i32; 3], max: [i32; 3], like: &Primitive) -> Option<Primitive> {
    if min[0] >= max[0] || min[1] >= max[1] || min[2] >= max[2] {
        return None;
    }
    Some(Primitive {
        shape: Shape::Box {
            aabb: QAabb::new(
                QVec3::new(min[0], min[1], min[2]),
                QVec3::new(max[0], max[1], max[2]),
            ),
        },
        ..*like
    })
}

/// The entrance port, if any, that pierces `bb`: its side (0 = -z, 1 = +x, 2 = +z, 3 = -x) and
/// the door's span along the face.
fn doorway_of(bb: &QAabb, ports: &[Port]) -> Option<(u8, i32, i32)> {
    ports.iter().find_map(|p| {
        let PortKind::Entrance { plane } = p.kind else {
            return None;
        };
        if p.rect.v0 != bb.min.y {
            return None;
        }
        let (lo, hi, side_lo, side_hi) = match p.face.axis {
            Axis::X => (bb.min.z, bb.max.z, 3, 1),
            _ => (bb.min.x, bb.max.x, 0, 2),
        };
        if p.rect.u0 < lo || p.rect.u1 > hi {
            return None;
        }
        let (face_lo, face_hi) = match p.face.axis {
            Axis::X => (bb.min.x, bb.max.x),
            _ => (bb.min.z, bb.max.z),
        };
        let side = if plane == face_lo {
            side_lo
        } else if plane == face_hi {
            side_hi
        } else {
            return None;
        };
        Some((side, p.rect.u0, p.rect.u1))
    })
}

/// An envelope drawn as a hollow shell, with the doorway cut through the wall it is in, so the
/// interior an L3 block builds shows through and the shell is dark inside until then.
fn hollow_envelope(env: &Primitive, door: (u8, i32, i32)) -> Vec<Primitive> {
    let Shape::Box { aabb: bb } = env.shape else {
        return vec![*env];
    };
    let (side, u0, u1) = door;
    let (i, t) = (SHELL_INSET, SHELL_THICKNESS);
    let o = [
        [bb.min.x + i, bb.min.y + i, bb.min.z + i],
        [bb.max.x - i, bb.max.y - i, bb.max.z - i],
    ];
    let mut out = Vec::new();
    let mut push = |min: [i32; 3], max: [i32; 3]| out.extend(solid_box(min, max, env));
    // Bottom and top, then the four walls between them.
    push(o[0], [o[1][0], o[0][1] + t, o[1][2]]);
    push([o[0][0], o[1][1] - t, o[0][2]], o[1]);
    let (y0, y1) = (o[0][1] + t, o[1][1] - t);
    let door_lo = bb.min.y + 4;
    let door_hi = bb.min.y + 48;
    // Each wall as (min, max, the axis the door runs along, which wall it is).
    let walls: [([i32; 3], [i32; 3], u8); 4] = [
        ([o[0][0], y0, o[0][2]], [o[1][0], y1, o[0][2] + t], 0),
        (
            [o[1][0] - t, y0, o[0][2] + t],
            [o[1][0], y1, o[1][2] - t],
            1,
        ),
        ([o[0][0], y0, o[1][2] - t], [o[1][0], y1, o[1][2]], 2),
        (
            [o[0][0], y0, o[0][2] + t],
            [o[0][0] + t, y1, o[1][2] - t],
            3,
        ),
    ];
    for (min, max, which) in walls {
        if which != side {
            push(min, max);
            continue;
        }
        // Along x for the z walls, along z for the x walls.
        let a = usize::from(which % 2 == 1) * 2;
        let (lo, hi) = (min[a], max[a]);
        let (d0, d1) = (u0.clamp(lo, hi), u1.clamp(lo, hi));
        let cut = |from: i32, to: i32, ylo: i32, yhi: i32| {
            let (mut mn, mut mx) = (min, max);
            mn[a] = from;
            mx[a] = to;
            mn[1] = ylo;
            mx[1] = yhi;
            (mn, mx)
        };
        for (mn, mx) in [
            cut(lo, hi, min[1], door_lo.clamp(min[1], max[1])),
            cut(lo, hi, door_hi.clamp(min[1], max[1]), max[1]),
            cut(
                lo,
                d0,
                door_lo.clamp(min[1], max[1]),
                door_hi.clamp(min[1], max[1]),
            ),
            cut(
                d1,
                hi,
                door_lo.clamp(min[1], max[1]),
                door_hi.clamp(min[1], max[1]),
            ),
        ] {
            push(mn, mx);
        }
    }
    out
}

/// The primitives to mesh for an L2 chunk: envelopes with an entrance become hollow shells and
/// railing boxes are left to their instanced modules.
fn mesh_prims_l2(prims: &[Primitive], ports: &[Port], mid: bool) -> Vec<Primitive> {
    let mut out = Vec::with_capacity(prims.len());
    for p in prims {
        // The whole box of a hollow part is for the mid band only.
        if p.flags.contains(PrimFlags::MID_ONLY) && !mid {
            continue;
        }
        match (p.flags.contains(PrimFlags::ENVELOPE), p.shape) {
            (true, Shape::Box { aabb }) => match doorway_of(&aabb, ports) {
                Some(door) => out.extend(hollow_envelope(p, door)),
                None => out.push(*p),
            },
            _ => out.push(*p),
        }
    }
    out
}

fn local_boxes(prims: &[Primitive]) -> Vec<(Vec3, Vec3)> {
    let m = |v: i32| v as f32 / 16.0;
    prims
        .iter()
        .filter(|p| !p.flags.contains(PrimFlags::NO_COLLIDE))
        .filter_map(|p| match p.shape {
            Shape::Box { aabb } => {
                let (a, b) = (
                    Vec3::new(m(aabb.min.x), m(aabb.min.y), m(aabb.min.z)),
                    Vec3::new(m(aabb.max.x), m(aabb.max.y), m(aabb.max.z)),
                );
                Some(((a + b) * 0.5, (b - a) * 0.5))
            }
            _ => None,
        })
        .collect()
}

/// `None` when the job went stale before meshing; the result is not wanted any more.
fn build(
    seed: WorldSeed,
    cache: &L1Cache,
    parents: &mut HashMap<CellKey, WebL3>,
    id: ChunkId,
    stale: &AtomicBool,
) -> Option<JobResult> {
    zone!("mesh job");
    let origin = match id.key.aabb_world() {
        Ok(b) => b.min,
        Err(e) => return Some(Err((id, e))),
    };
    let t0 = Instant::now();
    // `prims` is what collides, `draw` what is meshed.
    let mut modules = None;
    let mut ao_boxes = Vec::new();
    let (prims, draw): (Vec<Primitive>, Option<Vec<Primitive>>) = match (id.key.level, id.detail) {
        (Level::L2, Detail::Mid) => {
            let (out, _) = l2_build(seed, cache, id.key);
            let env = mid_primitives(out.primitives);
            let draw = mesh_prims_l2(&env, &out.ports, true);
            (env, Some(draw))
        }
        (Level::L2, _) => {
            let (out, _) = l2_build(seed, cache, id.key);
            let mut rails = Vec::new();
            if let Some(room) = worldgen::sanctuary_room::room_of(seed, &out) {
                rails.extend(worldgen::l4::sanctuary_modules(&room));
            }
            if !rails.is_empty() {
                modules = Some(expand(&rails));
            }
            let draw = mesh_prims_l2(&out.primitives, &out.ports, false);
            // The shells of the interior parts collide, and L3 collides inside them.
            (out.primitives, Some(draw))
        }
        (Level::L3, _) if !L3_ENABLED => (Vec::new(), None),
        (Level::L3, _) => {
            let parent_key = CellKey::new(Level::L2, id.key.coords.map(|v| floor_div_pow2(v, 3)));
            // Each context holds the blocks solved so far, so few are kept.
            if parents.len() > 12 {
                parents.clear();
            }
            let ctx = parents
                .entry(parent_key)
                .or_insert_with(|| web_context(seed, cache, parent_key));
            if ctx.is_active(id.key) {
                let prims = ctx.cell_primitives(seed, id.key);
                ao_boxes = local_boxes(&prims);
                (prims, None)
            } else {
                (Vec::new(), None)
            }
        }
        (_, Detail::Far) => (l1_far_primitives(seed, id.key), None),
        _ => {
            // The bored passages of its L2 cells are cut out of the solid (PW.3.21b).
            let (mut prims, _) = worldgen::l2_web::l1_primitives_carved(seed, cache, id.key);
            // Static tram track: drawn with the L1 full chunk, never collidable.
            prims.extend(worldgen::tram::tram_pieces(seed, id.key));
            (prims, None)
        }
    };
    let t1 = Instant::now();
    if stale.load(Ordering::Relaxed) {
        return None;
    }
    let far_digest = if cfg!(debug_assertions) {
        far_digest(id, &prims)
    } else {
        None
    };
    let (mesh, far) = if id.detail == Detail::Far {
        (Mesh::default(), Some(far_instances(&prims)))
    } else {
        let coarse = id.detail == Detail::Mid;
        let mut mesh = crate::primmesh::with_coarse(coarse, || {
            mesh_primitives(draw.as_deref().unwrap_or(&prims)).0
        });
        // The level goes into every vertex: the shader draws L2 and L3 a hair nearer than L1
        // so that surfaces of different levels in one plane do not fight (PW.4.18).
        crate::debugview::tag_level(&mut mesh.vertices, id.key.level);
        (mesh, None)
    };
    let t2 = Instant::now();
    if stale.load(Ordering::Relaxed) {
        return None;
    }
    Some(Ok(ChunkMesh {
        id,
        origin,
        mesh,
        prims,
        far_digest,
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
        far,
        modules,
        ao_boxes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;

    /// PW.4.17: `doorway_of` and `hollow_envelope` are v6 code. They fire on `Entrance` ports,
    /// which only the v6 L2 (`l2::l2_cell_in`) emits; the Web L2 has none, so they never act on
    /// a Web chunk. Kept for the ignored v6 tests until those are deleted.
    #[test]
    fn web_l2_cells_have_no_entrance_ports() {
        let cache = L1Cache::default();
        for k in 0..30i64 {
            let key = CellKey::new(Level::L2, I64Vec3::new(k % 8, 6 + k / 8 % 2, 2 + k / 3 % 6));
            let (out, _) = l2_build(WorldSeed(0), &cache, key);
            assert!(
                out.ports
                    .iter()
                    .all(|p| !matches!(p.kind, worldgen::port::PortKind::Entrance { .. })),
                "cell {key:?} has an entrance port"
            );
        }
    }

    /// PW.4.27: no chunk is over what the streamer accepts (4 times the 16 MiB upload budget of a
    /// frame; a bigger one is dropped, which leaves a hole in the world). L1 chunk (0,1,-1) of seed 0
    /// was 69.7 MB, because a 400 m diagonal beam cut by the bores of 1 700 passages was 73 000
    /// turned boxes whose shared faces were all drawn; the mesher now culls them in the beam's
    /// frame.
    #[test]
    fn chunks_fit_the_upload_budget() {
        const BUDGET: usize = 4 * 16 * 1024 * 1024;
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        let bytes = |m: &Mesh| m.vertices.len() * 20 + m.indices.len() * 4;
        for c in [
            I64Vec3::new(0, 1, -1),
            I64Vec3::new(-1, 0, 0),
            I64Vec3::new(0, 0, 0),
        ] {
            let key = CellKey::new(Level::L1, c);
            let (mut prims, _) = worldgen::l2_web::l1_primitives_carved(seed, &cache, key);
            prims.extend(worldgen::tram::tram_pieces(seed, key));
            let b = bytes(&mesh_primitives(&prims).0);
            assert!(b <= BUDGET, "L1 chunk {c:?} is {b} bytes");
        }
        for c in [
            I64Vec3::new(0, 6, 2),
            I64Vec3::new(0, 6, 3),
            I64Vec3::new(1, 5, 2),
        ] {
            let key = CellKey::new(Level::L2, c);
            let (out, _) = l2_build(seed, &cache, key);
            let m = mesh_primitives(&mesh_prims_l2(&out.primitives, &out.ports, false)).0;
            assert!(bytes(&m) <= BUDGET, "L2 chunk {c:?} is {} bytes", bytes(&m));
        }
    }

    #[test]
    #[ignore = "prints"]
    fn l2_mid_anatomy() {
        let cache = L1Cache::default();
        for c in [I64Vec3::new(0, 6, 2), I64Vec3::new(3, 5, 4)] {
            let key = CellKey::new(Level::L2, c);
            let (out, _) = l2_build(WorldSeed(0), &cache, key);
            let env = mid_primitives(out.primitives);
            let mut by: std::collections::BTreeMap<String, (usize, usize)> =
                std::collections::BTreeMap::new();
            for p in &env {
                let mut m = Mesh::default();
                crate::primmesh::add_primitive(&mut m, p);
                let shape = match p.shape {
                    worldgen::prim::Shape::Box { .. } => "box",
                    worldgen::prim::Shape::Oriented(o) => match o.local {
                        worldgen::orient::LocalShape::Box { .. } => "turned box",
                        worldgen::orient::LocalShape::Cylinder { .. } => "turned cylinder",
                        _ => "turned other",
                    },
                    worldgen::prim::Shape::Cylinder { .. } => "cylinder",
                    _ => "other",
                };
                let e = by
                    .entry(format!(
                        "{shape} interior={}",
                        p.flags.contains(PrimFlags::INTERIOR)
                    ))
                    .or_default();
                e.0 += 1;
                e.1 += m.vertices.len();
            }
            let mesh = mesh_primitives(&mesh_prims_l2(&env, &out.ports, true)).0;
            eprintln!(
                "cell {c:?}: {} envelopes, {} verts after culling = {:.2} MiB",
                env.len(),
                mesh.vertices.len(),
                (mesh.vertices.len() * 20 + mesh.indices.len() * 4) as f64 / 1_048_576.0
            );
            for (k, (n, v)) in by {
                eprintln!("  {k}: {n} pieces {v} verts");
            }
        }
    }

    #[test]
    #[ignore = "prints"]
    fn l2_full_anatomy() {
        let cache = L1Cache::default();
        let mut by: std::collections::BTreeMap<u8, (usize, usize)> =
            std::collections::BTreeMap::new();
        let (mut total, mut cells) = (0usize, 0);
        for dx in 0..3i64 {
            for dy in 0..2i64 {
                for dz in 0..3i64 {
                    let key = CellKey::new(Level::L2, I64Vec3::new(-24 + dx, dy, -32 + dz));
                    let (out, _) = l2_build(WorldSeed(0), &cache, key);
                    let env = mid_primitives(out.primitives.clone());
                    let draw = mesh_prims_l2(&env, &out.ports, true);
                    for p in &draw {
                        let mut m = Mesh::default();
                        crate::primmesh::add_primitive(&mut m, p);
                        let e = by.entry(p.kind.0).or_default();
                        e.0 += 1;
                        e.1 += m.vertices.len();
                    }
                    let mesh = mesh_primitives(&draw).0;
                    total += mesh.vertices.len() * 20 + mesh.indices.len() * 4;
                    cells += 1;
                }
            }
        }
        eprintln!(
            "{cells} cells, {:.1} MiB mean",
            total as f64 / cells as f64 / 1_048_576.0
        );
        for (k, (n, v)) in by {
            eprintln!(
                "  kind {k}: {} pieces, {} verts a cell",
                n / cells,
                v / cells
            );
        }
    }

    #[test]
    fn jobs_run_off_thread_and_match_the_direct_result() {
        let id = ChunkId::l2(I64Vec3::new(1, 0, -2));
        let pool = MeshPool::new(WorldSeed(5), 2);
        pool.submit(id, 1.0);
        let got = pool.recv().unwrap().unwrap();
        let (out, _) = l2_build(WorldSeed(5), &L1Cache::default(), id.key);
        let mut direct = mesh_primitives(&mesh_prims_l2(&out.primitives, &out.ports, false)).0;
        crate::debugview::tag_level(&mut direct.vertices, id.key.level);
        assert_eq!(got.id, id);
        assert_eq!(got.mesh.vertices, direct.vertices);
        assert_eq!(got.mesh.indices, direct.indices);
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn mid_chunk_is_the_envelope_subset_of_the_full_chunk() {
        let key = worldgen::cell::CellKey::new(Level::L2, I64Vec3::new(4, -4, 4));
        let full = worldgen::l2::l2_cell_in(WorldSeed(0), &L1Cache::default(), key).primitives;
        let mid = mid_primitives(full.clone());
        assert!(!mid.is_empty() && mid.len() < full.len());
        assert!(mid.iter().all(|p| full.contains(p)));
        assert!(mid.iter().all(|p| p.flags.contains(PrimFlags::ENVELOPE)));
        let tris = |p: &[Primitive]| mesh_primitives(p).0.indices.len() / 3;
        assert!(tris(&mid) < tris(&full));
    }

    /// A cell with masses and an entrance, found by search.
    fn cell_with_entrance(seed: WorldSeed) -> (CellKey, worldgen::prim::CellOutput) {
        let cache = L1Cache::default();
        for x in -12..12i64 {
            for k in (0..512i64).step_by(3) {
                let key = CellKey::new(
                    Level::L2,
                    I64Vec3::new(x * 8 + k % 8, (k / 64) % 8 - 4, (x % 5) * 8 + k / 8 % 8),
                );
                let (out, masses) = l2_build(seed, &cache, key);
                if !masses.is_empty()
                    && out
                        .ports
                        .iter()
                        .any(|p| matches!(p.kind, PortKind::Entrance { .. }))
                {
                    return (key, out);
                }
            }
        }
        panic!("no cell with an entrance in the search range");
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn an_envelope_with_an_entrance_is_a_hollow_shell_with_a_door() {
        let (_, out) = cell_with_entrance(WorldSeed(0));
        let mut doors = 0;
        for env in out
            .primitives
            .iter()
            .filter(|p| p.flags.contains(PrimFlags::ENVELOPE))
        {
            let Shape::Box { aabb } = env.shape else {
                continue;
            };
            let Some((side, u0, u1)) = doorway_of(&aabb, &out.ports) else {
                continue;
            };
            doors += 1;
            let shell = hollow_envelope(env, (side, u0, u1));
            assert!(shell.len() >= 8, "{} boxes", shell.len());
            // Nothing of the shell stands in the doorway: the opening is 4 to 48 quanta above
            // the mass bottom and spans the door along the face.
            let probe_y = aabb.min.y + 20;
            let mid = i32::midpoint(u0, u1);
            let (px, pz) = match side {
                0 => (mid, aabb.min.z + SHELL_INSET + 2),
                2 => (mid, aabb.max.z - SHELL_INSET - 2),
                1 => (aabb.max.x - SHELL_INSET - 2, mid),
                _ => (aabb.min.x + SHELL_INSET + 2, mid),
            };
            for b in &shell {
                let bb = b.aabb();
                let inside = bb.min.x < px
                    && px < bb.max.x
                    && bb.min.y < probe_y
                    && probe_y < bb.max.y
                    && bb.min.z < pz
                    && pz < bb.max.z;
                assert!(!inside, "the door is blocked by {bb:?}");
            }
            // Everything stays inside the original envelope.
            for b in &shell {
                let bb = b.aabb();
                assert!(bb.min.x >= aabb.min.x && bb.max.x <= aabb.max.x);
                assert!(bb.min.y >= aabb.min.y && bb.max.y <= aabb.max.y);
                assert!(bb.min.z >= aabb.min.z && bb.max.z <= aabb.max.z);
            }
        }
        assert!(doors > 0, "an entrance port matches an envelope");
    }

    /// Diagnostic: where to stand to look at an entrance, as `SUPERSTRUCT_SECTOR` and
    /// `SUPERSTRUCT_LOCAL` (`-- --ignored --nocapture`).
    #[test]
    #[ignore = "diagnostic"]
    fn print_entrance_viewpoints() {
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        let mut shown = 0;
        'scan: for x in -12..12i64 {
            for k in (0..512i64).step_by(3) {
                let key = CellKey::new(
                    Level::L2,
                    I64Vec3::new(x * 8 + k % 8, (k / 64) % 8 - 4, (x % 5) * 8 + k / 8 % 8),
                );
                let (out, masses) = l2_build(seed, &cache, key);
                for m in &masses {
                    let Some((side, u0, u1)) = doorway_of(&m.aabb, &out.ports) else {
                        continue;
                    };
                    let mid = f64::from(i32::midpoint(u0, u1)) / 16.0;
                    let b = &m.aabb;
                    let m_ = |v: i32| f64::from(v) / 16.0;
                    // Six metres out from the door, at eye height above the mass bottom.
                    let (x_, z_) = match side {
                        0 => (mid, m_(b.min.z) - 6.0),
                        2 => (mid, m_(b.max.z) + 6.0),
                        1 => (m_(b.max.x) + 6.0, mid),
                        _ => (m_(b.min.x) - 6.0, mid),
                    };
                    let y_ = m_(b.min.y) + 1.65;
                    let origin = key.coords.as_dvec3() * 512.0;
                    let p = origin + glam::DVec3::new(x_, y_, z_);
                    let sector = (p / 1024.0).floor();
                    let local = p - sector * 1024.0;
                    let yaw = [180, 90, 0, 270][usize::from(side)];
                    println!(
                        "side {side}: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW={yaw}",
                        sector.x, sector.y, sector.z, local.x, local.y, local.z
                    );
                    shown += 1;
                    if shown == 4 {
                        break 'scan;
                    }
                }
            }
        }
    }

    /// Prints standpoints 8 m outside the doors of sanctuary rooms, looking at the door.
    #[test]
    #[ignore = "diagnostic"]
    fn print_sanctuary_viewpoints() {
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        let mut shown = 0;
        for x in -6..6i64 {
            for k in 0..512i64 {
                let key = CellKey::new(
                    Level::L2,
                    I64Vec3::new(x * 8 + k % 8, k / 64, (x % 3) * 8 + k / 8 % 8),
                );
                let out = worldgen::l2::l2_cell_in(seed, &cache, key);
                let Some(room) = worldgen::sanctuary_room::room_of(seed, &out) else {
                    continue;
                };
                let f = room.floor;
                let [cx, cz] = room.centre();
                let m = |v: i32| f64::from(v) / 16.0;
                let (px, pz) = match room.door_side {
                    0 => (m(cx), m(f.min.z) - 8.0),
                    2 => (m(cx), m(f.max.z) + 8.0),
                    1 => (m(f.max.x) + 8.0, m(cz)),
                    _ => (m(f.min.x) - 8.0, m(cz)),
                };
                let origin = key.coords.as_dvec3() * 512.0;
                let p = origin + glam::DVec3::new(px, m(f.min.y) + 1.65, pz);
                let sector = (p / 1024.0).floor();
                let local = p - sector * 1024.0;
                let yaw = [180, 90, 0, 270][usize::from(room.door_side)];
                println!(
                    "door {}: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW={yaw}",
                    room.door_side, sector.x, sector.y, sector.z, local.x, local.y, local.z
                );
                shown += 1;
                if shown == 4 {
                    return;
                }
            }
        }
    }

    /// Prints standpoints on the deck edge of rim cells, looking into the giant cavity.
    #[test]
    #[ignore = "diagnostic"]
    fn print_cavity_rim_viewpoints() {
        use worldgen::cavity::is_cavity;
        use worldgen::prim::FeatureKind;
        let seed = WorldSeed(0);
        let mut shown = 0;
        for l0 in -4..4i64 {
            for l0z in -4..4i64 {
                for l0y in -1..2i64 {
                    let base = I64Vec3::new(l0, l0y, l0z) * 8;
                    for k in 0..512i64 {
                        let c = base + I64Vec3::new(k % 8, k / 64, k / 8 % 8);
                        let cell = CellKey::new(Level::L1, c);
                        if is_cavity(seed, cell) {
                            continue;
                        }
                        // Direction (dx, dz, yaw) towards a cavity neighbour.
                        let Some((dx, dz, yaw)) =
                            [(1, 0, 270), (-1, 0, 90), (0, 1, 0), (0, -1, 180)]
                                .into_iter()
                                .find(|&(dx, dz, _)| {
                                    is_cavity(
                                        seed,
                                        CellKey::new(Level::L1, c + I64Vec3::new(dx, 0, dz)),
                                    )
                                })
                        else {
                            continue;
                        };
                        let prims = worldgen::l1::l1_cell(seed, cell).primitives;
                        let reach = |p: &Primitive| {
                            let b = p.aabb();
                            match (dx, dz) {
                                (1, _) => b.max.x,
                                (-1, _) => -b.min.x,
                                (_, 1) => b.max.z,
                                _ => -b.min.z,
                            }
                        };
                        let Some(deck) = prims
                            .iter()
                            .filter(|p| p.kind == FeatureKind::DECK)
                            .max_by_key(|p| reach(p))
                        else {
                            continue;
                        };
                        let b = deck.aabb();
                        let m = |v: i32| f64::from(v) / 16.0;
                        let x = match dx {
                            1 => m(b.max.x) - 6.0,
                            -1 => m(b.min.x) + 6.0,
                            _ => m(i32::midpoint(b.min.x, b.max.x)),
                        };
                        let z = match dz {
                            1 => m(b.max.z) - 6.0,
                            -1 => m(b.min.z) + 6.0,
                            _ => m(i32::midpoint(b.min.z, b.max.z)),
                        };
                        let p = c.as_dvec3() * 4096.0 + glam::DVec3::new(x, m(b.max.y) + 1.65, z);
                        let sector = (p / 1024.0).floor();
                        let local = p - sector * 1024.0;
                        println!(
                            "SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW={yaw}",
                            sector.x, sector.y, sector.z, local.x, local.y, local.z
                        );
                        shown += 1;
                        if shown == 4 {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Prints standpoints for the infinite staircase: on a deck beside the hole, and floating
    /// 150 m off its corner at the middle of an L1 cell.
    #[test]
    #[ignore = "diagnostic"]
    fn print_staircase_viewpoints() {
        use worldgen::key::ColumnKey;
        use worldgen::l1::{DECK_REGION_LOG2, decks, stair_footprint};
        let seed = WorldSeed(0);
        let mut shown = 0;
        for lx in -40..40i64 {
            for lz in -40..40i64 {
                let l0 = ColumnKey {
                    level: Level::L0,
                    x: lx,
                    z: lz,
                };
                let Some(st) = worldgen::regime::rare_stair(seed, l0) else {
                    continue;
                };
                let col = ColumnKey {
                    level: Level::L1,
                    x: lx * 8 + i64::from(st.l1_x),
                    z: lz * 8 + i64::from(st.l1_z),
                };
                let Some(foot) = stair_footprint(seed, col) else {
                    continue;
                };
                let m = |v: i32| f64::from(v) / 16.0;
                for y in 0..4i64 {
                    let cell = CellKey::new(Level::L1, I64Vec3::new(col.x, y, col.z));
                    let Some(deck) =
                        decks(seed, worldgen::key::LayerKey::of(cell, DECK_REGION_LOG2))
                            .first()
                            .copied()
                    else {
                        continue;
                    };
                    let origin = cell.coords.as_dvec3() * 4096.0;
                    // Beside the hole's west edge on the deck, looking east at it (yaw 270).
                    let p = origin
                        + glam::DVec3::new(m(foot.x0) - 6.0, m(deck.top) + 1.65, m(foot.z0) + 32.0);
                    let sector = (p / 1024.0).floor();
                    let local = p - sector * 1024.0;
                    println!(
                        "deck: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW=270",
                        sector.x, sector.y, sector.z, local.x, local.y, local.z
                    );
                    shown += 1;
                    break;
                }
                if shown >= 3 {
                    return;
                }
            }
        }
    }

    #[test]
    fn an_l3_chunk_carries_the_interior_of_its_masses() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (key, ctx) = (0..40i64)
            .map(|k| {
                CellKey::new(
                    Level::L2,
                    I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4) * 8
                        + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
                )
            })
            .map(|key| (key, web_context(seed, &cache, key)))
            .find(|(_, ctx)| !ctx.parent().plans().is_empty())
            .expect("a cell with a mass");
        let mut ctx = ctx;
        let world = |k: i64| {
            CellKey::new(
                Level::L3,
                key.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
            )
        };
        let active: Vec<CellKey> = (0..512i64)
            .map(world)
            .filter(|w| !ctx.cell_primitives(seed, *w).is_empty())
            .collect();
        assert!(!active.is_empty());
        let pool = MeshPool::new(seed, 2);
        pool.submit(ChunkId::l3(active[0].coords), 1.0);
        let got = pool.recv().unwrap().unwrap();
        assert_eq!(got.id, ChunkId::l3(active[0].coords));
        assert!(!got.mesh.indices.is_empty() && !got.prims.is_empty());
        assert!(got.upload_bytes() > 0);
        // A cell no mass meets builds nothing.
        if let Some(b) = (0..512i64).map(world).find(|w| !ctx.is_active(*w)) {
            pool.submit(ChunkId::l3(b.coords), 1.0);
            let got = pool.recv().unwrap().unwrap();
            assert!(got.mesh.indices.is_empty() && got.prims.is_empty());
        }
    }

    #[test]
    fn many_jobs_all_complete_and_drop_joins() {
        let pool = MeshPool::new(WorldSeed(1), 3);
        for x in 0..12 {
            pool.submit(ChunkId::l2(I64Vec3::new(x, 0, 0)), x as f32);
        }
        let mut seen: Vec<i64> = (0..12)
            .map(|_| pool.recv().unwrap().unwrap().id.key.coords.x)
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..12).collect::<Vec<_>>());
        assert_eq!(pool.stats().completed, 12);
    }

    #[test]
    fn retarget_cancels_stale_queued_jobs_and_keeps_wanted_ones() {
        // One worker, so most jobs are still queued when we retarget; how many is a race, so
        // only timing-independent facts are asserted.
        let pool = MeshPool::new(WorldSeed(1), 1);
        let ids: Vec<ChunkId> = (0..20)
            .map(|x| ChunkId::l2(I64Vec3::new(x, 0, 0)))
            .collect();
        for (i, id) in ids.iter().enumerate() {
            pool.submit(*id, i as f32);
        }
        let keep: FastMap<ChunkId, f32> = ids[18..].iter().map(|id| (*id, 0.0)).collect();
        pool.retarget(&keep);
        while pool.stats().queued + pool.stats().running > 0 {
            thread::yield_now();
        }
        let mut got = Vec::new();
        while let Some(r) = pool.try_recv() {
            got.push(r.unwrap().id);
        }
        // The wanted jobs always finish, the worker cannot have done all twenty before the
        // retarget, and every job ends either as a result or as a cancellation.
        assert!(ids[18..].iter().all(|id| got.contains(id)));
        assert!(pool.stats().cancelled > 0, "{:?}", pool.stats());
        assert!(got.len() < 20, "{} results", got.len());
    }

    #[test]
    fn lowest_priority_value_runs_first() {
        let pool = MeshPool::new(WorldSeed(1), 1);
        // Occupy the worker, then queue out of order.
        pool.submit(ChunkId::l2(I64Vec3::new(100, 0, 0)), 0.0);
        for x in [3, 1, 2] {
            pool.submit(ChunkId::l2(I64Vec3::new(x, 0, 0)), x as f32);
        }
        let order: Vec<i64> = (0..4)
            .map(|_| pool.recv().unwrap().unwrap().id.key.coords.x)
            .collect();
        assert_eq!(&order[1..], &[1, 2, 3]);
    }

    /// Triangle totals of the mid band, checked against the budgets at a few spots. Run with
    /// `cargo test --release -p engine --lib mid_band -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement"]
    fn mid_band_triangle_totals() {
        use crate::stream::{Detail, StreamConfig, desired_set};
        use glam::Vec3;
        use worldgen::coord::WorldPos;
        let cfg = StreamConfig::default();
        let pool = MeshPool::new(WorldSeed(0), 8);
        for (sector, local) in [
            ([0, 0, 0], [512.0, 512.0, 512.0]),
            ([3, -2, 5], [100.0, 800.0, 300.0]),
            ([-2, -2, -3], [10.0, 745.0, 584.0]),
            ([40, 7, -9], [900.0, 20.0, 450.0]),
        ] {
            let cam = WorldPos::new(I64Vec3::from_array(sector), Vec3::from_array(local)).unwrap();
            let mut wanted = Vec::new();
            desired_set(&cfg, &cam, Vec3::NEG_Z, &mut wanted);
            let mid: Vec<_> = wanted
                .iter()
                .filter(|w| w.id.detail == Detail::Mid)
                .collect();
            for w in &mid {
                pool.submit(w.id, w.priority);
            }
            let (mut tris, mut max_cell) = (0usize, 0usize);
            for _ in 0..mid.len() {
                let t = pool.recv().unwrap().unwrap().mesh.indices.len() / 3;
                tris += t;
                max_cell = max_cell.max(t);
            }
            eprintln!(
                "{sector:?}: {} mid cells, {tris} triangles, busiest cell {max_cell}",
                mid.len()
            );
            assert!(
                tris <= MID_BAND_TRIANGLE_BUDGET,
                "{tris} triangles over budget"
            );
            assert!(
                max_cell <= MID_CELL_TRIANGLE_BUDGET,
                "busiest cell {max_cell}"
            );
        }
    }
}
