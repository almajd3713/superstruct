//! Worker threads that generate and mesh cells, so the main thread never does either.

use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use worldgen::cell::CellKey;
use worldgen::coord::{CoordError, WorldPos};
use worldgen::l2::l2_cell;
use worldgen::seed::WorldSeed;

use crate::chunkmesh::mesh_primitives;
use crate::mesh::Mesh;
use crate::zone;

/// A cell's CPU-side mesh, ready for upload.
pub struct ChunkMesh {
    pub key: CellKey,
    /// World position of the cell's minimum corner; vertices are relative to it.
    pub origin: WorldPos,
    pub mesh: Mesh,
    pub primitives: usize,
    pub gen_ms: f32,
    pub mesh_ms: f32,
}

#[derive(Clone, Copy)]
struct Job {
    seed: WorldSeed,
    key: CellKey,
}

pub struct MeshPool {
    jobs: Option<Sender<Job>>,
    results: Receiver<Result<ChunkMesh, (CellKey, CoordError)>>,
    workers: Vec<JoinHandle<()>>,
}

impl MeshPool {
    /// Spawns `threads` workers (at least one).
    pub fn new(threads: usize) -> Self {
        let (jobs, job_rx) = unbounded::<Job>();
        let (res_tx, results) = unbounded();
        let workers = (0..threads.max(1))
            .filter_map(|i| {
                let (rx, tx) = (job_rx.clone(), res_tx.clone());
                thread::Builder::new()
                    .name(format!("mesh-{i}"))
                    .spawn(move || {
                        for job in rx {
                            if tx.send(build(job)).is_err() {
                                break;
                            }
                        }
                    })
                    .inspect_err(|e| tracing::error!("cannot spawn mesh worker {i}: {e}"))
                    .ok()
            })
            .collect();
        Self {
            jobs: Some(jobs),
            results,
            workers,
        }
    }

    /// A pool sized to leave two cores for the main and render threads.
    pub fn with_default_threads() -> Self {
        let cores = thread::available_parallelism().map_or(2, usize::from);
        Self::new(cores.saturating_sub(2).max(1))
    }

    pub fn submit(&self, seed: WorldSeed, key: CellKey) {
        if let Some(tx) = &self.jobs {
            // The workers outlive the sender, so this only fails during shutdown.
            let _ = tx.send(Job { seed, key });
        }
    }

    /// A finished mesh, if any. Never blocks.
    pub fn try_recv(&self) -> Option<Result<ChunkMesh, (CellKey, CoordError)>> {
        self.results.try_recv().ok()
    }

    /// Waits for the next finished mesh. For tests and tools.
    pub fn recv(&self) -> Option<Result<ChunkMesh, (CellKey, CoordError)>> {
        self.results.recv().ok()
    }
}

impl Drop for MeshPool {
    fn drop(&mut self) {
        self.jobs = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn build(job: Job) -> Result<ChunkMesh, (CellKey, CoordError)> {
    zone!("mesh job");
    let origin = job.key.aabb_world().map_err(|e| (job.key, e))?.min;
    let t0 = Instant::now();
    let out = l2_cell(job.seed, job.key);
    let t1 = Instant::now();
    let (mesh, _) = mesh_primitives(&out.primitives);
    let t2 = Instant::now();
    Ok(ChunkMesh {
        key: job.key,
        origin,
        mesh,
        primitives: out.primitives.len(),
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;
    use worldgen::cell::Level;

    #[test]
    fn jobs_run_off_thread_and_match_the_direct_result() {
        let key = CellKey::new(Level::L2, I64Vec3::new(1, 0, -2));
        let pool = MeshPool::new(2);
        pool.submit(WorldSeed(5), key);
        let got = pool.recv().unwrap().unwrap();
        let direct = mesh_primitives(&l2_cell(WorldSeed(5), key).primitives).0;
        assert_eq!(got.key, key);
        assert_eq!(got.mesh.vertices, direct.vertices);
        assert_eq!(got.mesh.indices, direct.indices);
    }

    #[test]
    fn many_jobs_all_complete_and_drop_joins() {
        let pool = MeshPool::new(3);
        for x in 0..12 {
            pool.submit(WorldSeed(1), CellKey::new(Level::L2, I64Vec3::new(x, 0, 0)));
        }
        let mut seen: Vec<i64> = (0..12)
            .map(|_| pool.recv().unwrap().unwrap().key.coords.x)
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..12).collect::<Vec<_>>());
    }
}
