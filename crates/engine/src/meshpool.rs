//! Worker threads that generate and mesh chunks, so the main thread never does either.
//!
//! Jobs wait in a priority queue. The streamer re-ranks and prunes it as the camera moves
//! ([`MeshPool::retarget`]); a job already running is flagged stale and its result is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use worldgen::cell::Level;
use worldgen::coord::{CoordError, WorldPos};
use worldgen::l1::{l1_cell, l1_far_primitives};
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::prim::Primitive;
use worldgen::seed::WorldSeed;

use crate::chunkmesh::mesh_primitives;
use crate::mesh::Mesh;
use crate::stream::{ChunkId, Detail};
use crate::zone;

/// A chunk's CPU-side mesh, ready for upload.
pub struct ChunkMesh {
    pub id: ChunkId,
    /// World position of the cell's minimum corner; vertices are relative to it.
    pub origin: WorldPos,
    pub mesh: Mesh,
    pub primitives: usize,
    pub gen_ms: f32,
    pub mesh_ms: f32,
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
}

impl Shared {
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
const CACHE_JOBS: u32 = 64;

impl MeshPool {
    /// Spawns `threads` workers (at least one).
    pub fn new(seed: WorldSeed, threads: usize) -> Self {
        let shared = Arc::new(Shared {
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
    pub fn retarget(&self, wanted: &HashMap<ChunkId, f32>) {
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
    let mut cache = L1Cache::default();
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
            cache = L1Cache::default();
            ran = 0;
        }
        ran += 1;
        let result = build(shared.seed, &cache, id, &stale);
        shared.lock().running.remove(&id);
        if let Some(r) = result {
            shared.completed.fetch_add(1, Ordering::Relaxed);
            if tx.send(r).is_err() {
                return;
            }
        }
    }
}

/// `None` when the job went stale before meshing; the result is not wanted any more.
fn build(seed: WorldSeed, cache: &L1Cache, id: ChunkId, stale: &AtomicBool) -> Option<JobResult> {
    zone!("mesh job");
    let origin = match id.key.aabb_world() {
        Ok(b) => b.min,
        Err(e) => return Some(Err((id, e))),
    };
    let t0 = Instant::now();
    let prims: Vec<Primitive> = match (id.key.level, id.detail) {
        (Level::L2, _) => l2_cell_in(seed, cache, id.key).primitives,
        (_, Detail::Far) => l1_far_primitives(seed, id.key),
        _ => l1_cell(seed, id.key).primitives,
    };
    let t1 = Instant::now();
    if stale.load(Ordering::Relaxed) {
        return None;
    }
    let (mesh, _) = mesh_primitives(&prims);
    let t2 = Instant::now();
    if stale.load(Ordering::Relaxed) {
        return None;
    }
    Some(Ok(ChunkMesh {
        id,
        origin,
        mesh,
        primitives: prims.len(),
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;

    #[test]
    fn jobs_run_off_thread_and_match_the_direct_result() {
        let id = ChunkId::l2(I64Vec3::new(1, 0, -2));
        let pool = MeshPool::new(WorldSeed(5), 2);
        pool.submit(id, 1.0);
        let got = pool.recv().unwrap().unwrap();
        let direct =
            mesh_primitives(&l2_cell_in(WorldSeed(5), &L1Cache::default(), id.key).primitives).0;
        assert_eq!(got.id, id);
        assert_eq!(got.mesh.vertices, direct.vertices);
        assert_eq!(got.mesh.indices, direct.indices);
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
        // One worker, so most jobs are still queued when we retarget.
        let pool = MeshPool::new(WorldSeed(1), 1);
        let ids: Vec<ChunkId> = (0..20)
            .map(|x| ChunkId::l2(I64Vec3::new(x, 0, 0)))
            .collect();
        for (i, id) in ids.iter().enumerate() {
            pool.submit(*id, i as f32);
        }
        let keep: HashMap<ChunkId, f32> = ids[18..].iter().map(|id| (*id, 0.0)).collect();
        pool.retarget(&keep);
        let stats = pool.stats();
        assert!(stats.cancelled >= 10, "{stats:?}");
        assert!(stats.queued <= 2 + 1);
        // Everything that still completes is a wanted chunk, once the stale one is discarded.
        while pool.stats().queued + pool.stats().running > 0 {
            thread::yield_now();
        }
        let mut got = Vec::new();
        while let Some(r) = pool.try_recv() {
            got.push(r.unwrap().id);
        }
        assert!(
            got.iter()
                .all(|id| keep.contains_key(id) || got.len() <= 1 + 2)
        );
        assert!(ids[18..].iter().all(|id| got.contains(id)));
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
}
