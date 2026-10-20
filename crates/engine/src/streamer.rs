//! Keeps the wanted chunks resident: asks the pool for them, uploads finished meshes under a
//! per-frame byte budget, and evicts the least recently wanted chunks when GPU memory is tight.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::fasthash::FastMap;
use std::time::Instant;

use glam::Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::coord::WorldPos;

use crate::collision::{CollisionChunk, CollisionWorld};
use crate::meshpool::{ChunkMesh, MeshPool, PoolStats};
use crate::renderer::{ChunkHandle, Renderer};
use crate::stream::{ChunkId, Detail, Probe, StreamConfig, Wanted, desired_set_with_probes};
use crate::vertex::Vertex;
use crate::zone;

/// Where finished meshes go. The renderer in the game, a counter in tests.
pub trait ChunkSink {
    /// Uploads `mesh` and returns its handle and GPU size in bytes.
    fn upload(&mut self, chunk: &ChunkMesh) -> (ChunkHandle, u64);
    fn remove(&mut self, handle: ChunkHandle);
    fn set_visible(&mut self, handle: ChunkHandle, visible: bool);
    /// Video memory the sink holds for chunks, used or not (pages left part empty by
    /// evictions count): 0 when it does not know.
    fn allocated_bytes(&self) -> u64 {
        0
    }
}

/// [`ChunkSink`] backed by the renderer.
pub struct RendererSink<'a> {
    pub renderer: &'a mut Renderer,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

impl ChunkSink for RendererSink<'_> {
    fn upload(&mut self, chunk: &ChunkMesh) -> (ChunkHandle, u64) {
        let (handle, bytes) = match &chunk.far {
            Some(far) => self
                .renderer
                .upload_far_chunk(self.queue, chunk.origin, far),
            None => self
                .renderer
                .upload_chunk(self.device, self.queue, chunk.origin, &chunk.mesh),
        };
        // Streamed chunks fade in; a bare upload (tests, tools) is drawn at once.
        self.renderer.begin_fade_in(handle);
        let extra = if chunk.modules.is_some() || !chunk.ao_boxes.is_empty() {
            self.renderer.attach_chunk_extras(
                self.queue,
                handle,
                chunk.origin,
                chunk.modules.as_ref(),
                chunk.ao_boxes.clone(),
            )
        } else {
            0
        };
        (handle, bytes + extra)
    }

    fn remove(&mut self, handle: ChunkHandle) {
        self.renderer.remove_chunk(handle);
    }

    fn set_visible(&mut self, handle: ChunkHandle, visible: bool) {
        self.renderer.set_chunk_visible(handle, visible);
    }

    fn allocated_bytes(&self) -> u64 {
        self.renderer.page_bytes()
    }
}

/// How many times the upload budget a single chunk may be and still be uploaded (alone).
const OVERSIZE_FACTOR: u64 = 4;

#[derive(Debug, Clone, Copy)]
pub struct StreamerConfig {
    pub stream: StreamConfig,
    /// Most bytes uploaded in one frame. A chunk larger than this is rejected, never split.
    pub upload_budget_bytes: u64,
    /// Resident GPU bytes above which unwanted chunks are evicted, least recently wanted first.
    pub memory_budget_bytes: u64,
}

impl Default for StreamerConfig {
    fn default() -> Self {
        Self {
            stream: StreamConfig::default(),
            upload_budget_bytes: 16 << 20,
            memory_budget_bytes: 768 << 20,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StreamStats {
    pub wanted: usize,
    pub resident: usize,
    pub visible: usize,
    pub ready: usize,
    pub pool: PoolStats,
    pub resident_bytes: u64,
    pub uploads_this_frame: usize,
    pub upload_bytes_this_frame: u64,
    pub evictions: u64,
    /// Chunks too big for the per-frame budget.
    pub rejected: u64,
    /// Frames where wanted chunks alone exceeded the memory budget.
    pub over_budget_frames: u64,
    /// Debug builds: far and full L1 chunks of one cell compared (P2.7.1).
    pub consistency_checks: u64,
}

struct Resident {
    handle: ChunkHandle,
    bytes: u64,
    last_wanted: u64,
    visible: bool,
}

/// While chunks are still loading, frames between re-ranking inside the same L2 cell, so
/// turning the view changes the load order.
const RERANK_FRAMES: u64 = 30;
/// With everything loaded, re-ranking only pays off after the camera moved this far (metres),
/// since the wanted set cannot change otherwise.
const RERANK_MOVE_M: f64 = 32.0;
/// While riding on a schedule the wanted set is stable far longer, and a re-rank costs several
/// milliseconds, so it happens this rarely (`docs/spec/trams.md` §7).
const RERANK_MOVE_PROBED_M: f64 = 256.0;

pub struct Streamer {
    cfg: StreamerConfig,
    pool: MeshPool,
    resident: FastMap<ChunkId, Resident>,
    /// Meshes the workers finished that still wait for upload budget.
    ready: VecDeque<ChunkMesh>,
    wanted: Vec<Wanted>,
    wanted_map: FastMap<ChunkId, f32>,
    last_cell: Option<CellKey>,
    last_rank: u64,
    last_rank_pos: Option<WorldPos>,
    probes: Vec<Probe>,
    /// Milliseconds of the last update: retarget, collect, upload, visibility, evict.
    last_parts_ms: [f64; 5],
    /// Milliseconds of the last retarget: desired set, wanted map, pool retarget, submit loop.
    retarget_ms: [f64; 4],
    /// Visibility only depends on the wanted and resident sets, which change rarely.
    visibility_dirty: bool,
    frame: u64,
    resident_bytes: u64,
    stats: StreamStats,
    scratch: Vec<Wanted>,
    collision: CollisionWorld,
    /// Debug builds: far-visible digest of each resident L1 chunk, for the consistency check.
    digests: HashMap<ChunkId, u64>,
}

impl Streamer {
    pub fn new(cfg: StreamerConfig, pool: MeshPool) -> Self {
        Self {
            cfg,
            pool,
            resident: FastMap::default(),
            ready: VecDeque::new(),
            wanted: Vec::new(),
            wanted_map: FastMap::default(),
            last_cell: None,
            last_rank: 0,
            last_rank_pos: None,
            probes: Vec::new(),
            last_parts_ms: [0.0; 5],
            retarget_ms: [0.0; 4],
            visibility_dirty: true,
            frame: 0,
            resident_bytes: 0,
            stats: StreamStats::default(),
            scratch: Vec::new(),
            collision: CollisionWorld::default(),
            digests: HashMap::new(),
        }
    }

    pub fn config(&self) -> &StreamerConfig {
        &self.cfg
    }

    /// Changes the band radii live; the next update re-targets with them.
    pub fn set_stream_config(&mut self, stream: StreamConfig) {
        let stream = stream.ordered();
        if stream != self.cfg.stream {
            self.cfg.stream = stream;
            self.last_cell = None;
        }
    }

    /// `(resident, wanted)` among the full-detail chunks within `radius` metres of `cam`: how
    /// much of the place the camera stands in has arrived (PW.4.24).
    pub fn near_arrived(&self, cam: &WorldPos, radius: f64) -> (usize, usize) {
        let (mut have, mut want) = (0, 0);
        for w in &self.wanted {
            if w.id.detail != Detail::Full {
                continue;
            }
            if crate::stream::distance_to_cell(cam, w.id.key).is_some_and(|d| d <= radius) {
                want += 1;
                have += usize::from(self.resident.contains_key(&w.id));
            }
        }
        (have, want)
    }

    /// The full-detail chunks within `radius` of `cam` that are not resident yet.
    pub fn near_missing(&self, cam: &WorldPos, radius: f64) -> Vec<ChunkId> {
        self.wanted
            .iter()
            .filter(|w| w.id.detail == Detail::Full && !self.resident.contains_key(&w.id))
            .filter(|w| crate::stream::distance_to_cell(cam, w.id.key).is_some_and(|d| d <= radius))
            .map(|w| w.id)
            .collect()
    }

    /// Resident chunks and bytes by level and detail, for the logs (PW.4.23).
    pub fn breakdown(&self) -> std::collections::BTreeMap<String, (usize, u64)> {
        let mut by = std::collections::BTreeMap::new();
        for (id, r) in &self.resident {
            let e = by
                .entry(format!("{:?} {:?}", id.key.level, id.detail))
                .or_insert((0, 0));
            e.0 += 1;
            e.1 += r.bytes;
        }
        by
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            wanted: self.wanted.len(),
            resident: self.resident.len(),
            visible: self.resident.values().filter(|r| r.visible).count(),
            ready: self.ready.len(),
            pool: self.pool.stats(),
            resident_bytes: self.resident_bytes,
            ..self.stats
        }
    }

    /// Collision data for every resident full-detail chunk.
    pub fn collision(&self) -> &CollisionWorld {
        &self.collision
    }

    /// For the game to place its moving solids (lift cars, trams).
    pub fn collision_mut(&mut self) -> &mut CollisionWorld {
        &mut self.collision
    }

    pub fn is_resident(&self, id: &ChunkId) -> bool {
        self.resident.contains_key(id)
    }

    /// One frame of streaming around `cam`.
    pub fn update(&mut self, cam: &WorldPos, view_dir: Vec3, sink: &mut dyn ChunkSink) {
        zone!("streamer update");
        self.frame += 1;
        let cell = CellKey::containing(Level::L2, cam);
        let moved = self
            .last_rank_pos
            .is_none_or(|p| cam.delta(&p).length() >= self.rerank_distance());
        let loading = self.frame - self.last_rank >= RERANK_FRAMES && self.has_pending_work();
        let mut mark = Instant::now();
        let mut lap = |slot: usize, parts: &mut [f64; 5]| {
            parts[slot] = mark.elapsed().as_secs_f64() * 1000.0;
            mark = Instant::now();
        };
        let mut parts = [0.0; 5];
        if self.last_cell != Some(cell) || moved || loading {
            self.last_cell = Some(cell);
            self.last_rank = self.frame;
            self.last_rank_pos = Some(*cam);
            self.visibility_dirty = true;
            self.retarget(cam, view_dir);
        }
        lap(0, &mut parts);
        self.collect_finished();
        lap(1, &mut parts);
        self.upload(sink);
        lap(2, &mut parts);
        if self.visibility_dirty {
            self.visibility_dirty = false;
            self.refresh_visibility(sink);
        }
        lap(3, &mut parts);
        self.evict(sink);
        lap(4, &mut parts);
        self.last_parts_ms = parts;
    }

    /// Wanted-set look-ahead from a known schedule; empty when not on one.
    pub fn set_probes(&mut self, probes: &[Probe]) {
        if probes.len() != self.probes.len() {
            self.last_rank_pos = None;
        }
        self.probes.clear();
        self.probes.extend_from_slice(probes);
    }

    pub fn retarget_ms(&self) -> [f64; 4] {
        self.retarget_ms
    }

    pub fn last_parts_ms(&self) -> [f64; 5] {
        self.last_parts_ms
    }

    fn rerank_distance(&self) -> f64 {
        if self.probes.is_empty() {
            RERANK_MOVE_M
        } else {
            RERANK_MOVE_PROBED_M
        }
    }

    fn has_pending_work(&self) -> bool {
        let pool = self.pool.stats();
        !self.ready.is_empty() || pool.queued + pool.running > 0
    }

    /// The far band must draw exactly the structure the near band does for the same cell: when
    /// both details of an L1 cell are resident their far-visible primitives must match.
    fn check_consistency(&mut self, chunk: &ChunkMesh) {
        let Some(digest) = chunk.far_digest else {
            return;
        };
        let other = ChunkId {
            detail: match chunk.id.detail {
                Detail::Full => Detail::Far,
                Detail::Far => Detail::Full,
                Detail::Mid => return,
            },
            ..chunk.id
        };
        if let Some(theirs) = self.digests.get(&other) {
            self.stats.consistency_checks += 1;
            assert_eq!(
                digest, *theirs,
                "far and near L1 primitives differ for {:?}",
                chunk.id.key
            );
        }
        self.digests.insert(chunk.id, digest);
    }

    fn retarget(&mut self, cam: &WorldPos, view_dir: Vec3) {
        let t = Instant::now();
        desired_set_with_probes(
            &self.cfg.stream,
            cam,
            view_dir,
            &self.probes,
            &mut self.scratch,
        );
        let t_set = t.elapsed().as_secs_f64() * 1000.0;
        std::mem::swap(&mut self.wanted, &mut self.scratch);
        self.wanted_map.clear();
        self.wanted_map
            .extend(self.wanted.iter().map(|w| (w.id, w.priority)));
        let t_map = t.elapsed().as_secs_f64() * 1000.0;
        self.pool.retarget(&self.wanted_map);
        let t_pool = t.elapsed().as_secs_f64() * 1000.0;
        let in_ready: HashSet<ChunkId, std::hash::BuildHasherDefault<crate::fasthash::FxHasher>> =
            self.ready.iter().map(|c| c.id).collect();
        for w in &self.wanted {
            if let Some(r) = self.resident.get_mut(&w.id) {
                r.last_wanted = self.frame;
            } else if !in_ready.contains(&w.id) {
                self.pool.submit(w.id, w.priority);
            }
        }
        let map = &self.wanted_map;
        self.ready.retain(|c| map.contains_key(&c.id));
        self.retarget_ms = [
            t_set,
            t_map - t_set,
            t_pool - t_map,
            t.elapsed().as_secs_f64() * 1000.0 - t_pool,
        ];
    }

    fn collect_finished(&mut self) {
        while let Some(done) = self.pool.try_recv() {
            match done {
                Ok(chunk) if self.wanted_map.contains_key(&chunk.id) => self.ready.push_back(chunk),
                Ok(_) => {}
                Err((id, err)) => tracing::error!("cannot build chunk {id:?}: {err}"),
            }
        }
    }

    /// Uploads ready meshes, nearest first, without exceeding the frame budget.
    fn upload(&mut self, sink: &mut dyn ChunkSink) {
        self.stats.uploads_this_frame = 0;
        self.stats.upload_bytes_this_frame = 0;
        let map = &self.wanted_map;
        self.ready.make_contiguous().sort_by(|a, b| {
            let p = |c: &ChunkMesh| map.get(&c.id).copied().unwrap_or(f32::MAX);
            p(a).total_cmp(&p(b)).then(a.id.cmp(&b.id))
        });
        let mut used = 0u64;
        let mut i = 0;
        while i < self.ready.len() {
            let chunk = &self.ready[i];
            let bytes = chunk.upload_bytes();
            let budget = self.cfg.upload_budget_bytes;
            // A chunk over the frame's budget uploads alone, in a frame of its own, up to four
            // times the budget (the Web L1 and L2 cells are heavy: PW.3.21c); beyond that it is
            // dropped.
            if bytes > budget * OVERSIZE_FACTOR {
                tracing::error!(
                    "chunk {:?} is {bytes} bytes, over {OVERSIZE_FACTOR} times the {budget} byte upload budget; dropped",
                    chunk.id
                );
                self.stats.rejected += 1;
                self.ready.remove(i);
                continue;
            }
            if bytes > budget && used > 0 {
                i += 1;
                continue;
            }
            if bytes <= budget && used + bytes > budget {
                i += 1;
                continue;
            }
            let Some(chunk) = self.ready.remove(i) else {
                break;
            };
            let (handle, actual) = sink.upload(&chunk);
            self.check_consistency(&chunk);
            if chunk.id.detail == Detail::Full {
                let size = chunk.id.key.level.size_m() as f32;
                self.collision.insert(
                    chunk.id,
                    CollisionChunk::new(chunk.origin, size, &chunk.prims),
                );
            }
            used += bytes;
            self.visibility_dirty = true;
            self.stats.uploads_this_frame += 1;
            self.stats.upload_bytes_this_frame = used;
            self.resident_bytes += actual;
            self.resident.insert(
                chunk.id,
                Resident {
                    handle,
                    bytes: actual,
                    last_wanted: self.frame,
                    visible: true,
                },
            );
        }
    }

    /// Wanted chunks draw. An unwanted one keeps drawing only while its cell's replacement detail
    /// is not resident yet, so a far-to-full swap leaves no hole.
    fn refresh_visibility(&mut self, sink: &mut dyn ChunkSink) {
        let other_details = |d: Detail| match d {
            Detail::Full => [Detail::Mid, Detail::Far],
            Detail::Mid => [Detail::Full, Detail::Far],
            Detail::Far => [Detail::Full, Detail::Mid],
        };
        let mut changes: Vec<(ChunkId, bool)> = Vec::new();
        for (id, r) in &self.resident {
            // Wanted, or its cell is wanted at another detail that is not resident yet.
            let visible = self.wanted_map.contains_key(id) || {
                let mut wanted_elsewhere = false;
                let mut replacement_ready = false;
                for d in other_details(id.detail) {
                    let other = ChunkId { detail: d, ..*id };
                    if self.wanted_map.contains_key(&other) {
                        wanted_elsewhere = true;
                        replacement_ready |= self.resident.contains_key(&other);
                    }
                }
                wanted_elsewhere && !replacement_ready
            };
            if visible != r.visible {
                changes.push((*id, visible));
            }
        }
        for (id, visible) in changes {
            if let Some(r) = self.resident.get_mut(&id) {
                r.visible = visible;
                sink.set_visible(r.handle, visible);
            }
        }
    }

    fn evict(&mut self, sink: &mut dyn ChunkSink) {
        // Pages are bigger than what is in them (a chunk is a few in a page and evictions leave
        // holes), so the memory held is also held to half as much again as the budget.
        let (budget, held_limit) = (
            self.cfg.memory_budget_bytes,
            self.cfg.memory_budget_bytes + self.cfg.memory_budget_bytes / 2,
        );
        if self.resident_bytes <= budget && sink.allocated_bytes() <= held_limit {
            return;
        }
        let mut victims: Vec<(u64, ChunkId)> = self
            .resident
            .iter()
            .filter(|(id, r)| !self.wanted_map.contains_key(id) && !r.visible)
            .map(|(id, r)| (r.last_wanted, *id))
            .collect();
        victims.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, id) in victims {
            if self.resident_bytes <= budget && sink.allocated_bytes() <= held_limit {
                break;
            }
            if let Some(r) = self.resident.remove(&id) {
                sink.remove(r.handle);
                self.collision.remove(&id);
                self.digests.remove(&id);
                self.resident_bytes -= r.bytes;
                self.stats.evictions += 1;
                self.visibility_dirty = true;
            }
        }
        if self.resident_bytes > budget || sink.allocated_bytes() > held_limit {
            self.stats.over_budget_frames += 1;
        }
    }
}

/// Size of a chunk's vertex and index buffers, matching what the renderer allocates.
pub fn chunk_bytes(vertices: &[Vertex], indices: &[u32]) -> u64 {
    if indices.is_empty() {
        return 0;
    }
    (std::mem::size_of_val(vertices) + std::mem::size_of_val(indices)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;
    use std::time::{Duration, Instant};
    use worldgen::seed::WorldSeed;

    #[derive(Default)]
    struct FakeSink {
        next: u32,
        live: HashMap<u32, (u64, bool)>,
    }

    impl ChunkSink for FakeSink {
        fn upload(&mut self, chunk: &ChunkMesh) -> (ChunkHandle, u64) {
            let bytes = chunk.upload_bytes();
            self.next += 1;
            self.live.insert(self.next, (bytes, true));
            (ChunkHandle(self.next), bytes)
        }
        fn remove(&mut self, h: ChunkHandle) {
            assert!(self.live.remove(&h.0).is_some(), "double free");
        }
        fn set_visible(&mut self, h: ChunkHandle, v: bool) {
            self.live.get_mut(&h.0).unwrap().1 = v;
        }
    }

    fn small() -> StreamConfig {
        StreamConfig {
            l3_radius_m: 0.0,
            l2_radius_m: 600.0,
            l2_mid_radius_m: 1536.0,
            l1_full_radius_m: 2048.0,
            l1_far_radius_m: 4096.0,
        }
    }

    fn pos(x: f32, sector_x: i64) -> WorldPos {
        WorldPos::new(I64Vec3::new(sector_x, 0, 0), Vec3::new(x, 400.0, 400.0)).unwrap()
    }

    fn streamer(cfg: StreamerConfig) -> Streamer {
        Streamer::new(cfg, MeshPool::new(WorldSeed(0), 2))
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn uploads_never_exceed_the_frame_budget_and_everything_arrives() {
        let budget = 200_000;
        let mut s = streamer(StreamerConfig {
            stream: small(),
            upload_budget_bytes: budget,
            memory_budget_bytes: u64::MAX,
        });
        let mut sink = FakeSink::default();
        let cam = pos(500.0, 0);
        let start = Instant::now();
        loop {
            s.update(&cam, Vec3::X, &mut sink);
            let st = s.stats();
            assert!(st.upload_bytes_this_frame <= budget, "{st:?}");
            let idle = st.pool.queued + st.pool.running + st.ready == 0;
            if idle && st.resident == st.wanted {
                break;
            }
            assert!(start.elapsed() < Duration::from_mins(2), "stalled: {st:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(sink.live.values().all(|(_, visible)| *visible));
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn fast_flythrough_stays_inside_the_memory_budget_without_stale_work() {
        let memory = 900_000;
        let mut s = streamer(StreamerConfig {
            stream: small(),
            upload_budget_bytes: 1 << 20,
            memory_budget_bytes: memory,
        });
        let mut sink = FakeSink::default();
        // 400 m/s at 60 fps along x, across several sectors.
        let (mut x, mut sector) = (0.0_f32, 0_i64);
        let mut peak = 0;
        let mut evicted = 0;
        for _ in 0..2400 {
            x += 400.0 / 60.0;
            if x >= 1024.0 {
                x -= 1024.0;
                sector += 1;
            }
            s.update(&pos(x, sector), Vec3::X, &mut sink);
            let st = s.stats();
            assert_eq!(st.over_budget_frames, 0, "{st:?}");
            peak = peak.max(st.resident_bytes);
            evicted = st.evictions;
            std::thread::sleep(Duration::from_micros(500));
        }
        let st = s.stats();
        assert!(peak <= memory, "peak {peak}");
        assert!(evicted > 0, "the budget never bit: {st:?}");
        assert!(st.pool.cancelled > 0, "{st:?}");
        // The pool never holds more than the wanted set.
        assert!(st.pool.queued + st.pool.running <= st.wanted);
        // The sink agrees with the streamer's books.
        let live: u64 = sink.live.values().map(|(b, _)| *b).sum();
        assert_eq!(live, st.resident_bytes);
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn far_to_full_swap_keeps_the_old_chunk_drawn_until_the_new_one_is_resident() {
        let mut s = streamer(StreamerConfig {
            stream: small(),
            upload_budget_bytes: 8 << 20,
            memory_budget_bytes: u64::MAX,
        });
        let mut sink = FakeSink::default();
        let settle = |s: &mut Streamer, sink: &mut FakeSink, cam: &WorldPos| {
            let t = Instant::now();
            loop {
                s.update(cam, Vec3::X, sink);
                let st = s.stats();
                if st.pool.queued + st.pool.running + st.ready == 0 {
                    return;
                }
                assert!(t.elapsed() < Duration::from_mins(2));
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        // Cell x = 1 is far from sector 0 start and full-detail once we are at sector 3.
        settle(&mut s, &mut sink, &pos(10.0, 0));
        let far = ChunkId::l1(I64Vec3::new(1, 0, 0), crate::stream::Detail::Far);
        let full = ChunkId::l1(I64Vec3::new(1, 0, 0), crate::stream::Detail::Full);
        assert!(s.is_resident(&far) && !s.is_resident(&full));
        // One frame at the new position: the Full chunk is wanted but not resident yet.
        s.update(&pos(10.0, 3), Vec3::X, &mut sink);
        if !s.is_resident(&full) {
            assert!(sink.live.values().filter(|(_, v)| *v).count() > 0);
            let handle = s.resident[&far].handle;
            assert!(
                sink.live[&handle.0].1,
                "far chunk hidden before its replacement arrived"
            );
        }
        settle(&mut s, &mut sink, &pos(10.0, 3));
        assert!(s.is_resident(&full));
        // Both details were resident at once, so the debug consistency check ran and passed.
        assert!(s.stats().consistency_checks >= 1);
        let handle = s.resident[&far].handle;
        assert!(
            !sink.live[&handle.0].1,
            "far chunk still drawn under the full one"
        );
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn mid_to_full_swap_hides_the_mid_chunk_and_keeps_it_out_of_collision() {
        let mut s = streamer(StreamerConfig {
            stream: small(),
            upload_budget_bytes: 8 << 20,
            memory_budget_bytes: u64::MAX,
        });
        let mut sink = FakeSink::default();
        let settle = |s: &mut Streamer, sink: &mut FakeSink, cam: &WorldPos| {
            let t = Instant::now();
            loop {
                s.update(cam, Vec3::X, sink);
                let st = s.stats();
                if st.pool.queued + st.pool.running + st.ready == 0 {
                    return;
                }
                assert!(t.elapsed() < Duration::from_mins(2));
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        // L2 cell x = 2 spans 1024..1536 m: 1014 m away is mid, 424 m away is near.
        let cell = I64Vec3::new(2, 0, 0);
        let (mid, full) = (ChunkId::l2_mid(cell), ChunkId::l2(cell));
        settle(&mut s, &mut sink, &pos(10.0, 0));
        assert!(s.is_resident(&mid) && !s.is_resident(&full));
        assert!(!s.collision().contains(&mid), "mid chunks never collide");
        settle(&mut s, &mut sink, &pos(600.0, 0));
        assert!(s.is_resident(&full));
        let handle = s.resident[&mid].handle;
        assert!(
            !sink.live[&handle.0].1,
            "mid chunk still drawn under the full one"
        );
    }

    /// The far band at its default 50 km radius fills within its time budget on 8 workers.
    /// Prints the time; run `cargo test --release -p engine --lib far_band -- --nocapture`.
    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn far_band_reaches_fifty_km_within_its_time_budget() {
        let mut s = Streamer::new(
            StreamerConfig {
                stream: StreamConfig::default(),
                upload_budget_bytes: 16 << 20,
                memory_budget_bytes: u64::MAX,
            },
            MeshPool::new(WorldSeed(0), 8),
        );
        let mut sink = FakeSink::default();
        let cam = pos(512.0, 0);
        let start = Instant::now();
        loop {
            s.update(&cam, Vec3::X, &mut sink);
            let st = s.stats();
            if st.pool.queued + st.pool.running + st.ready == 0 {
                break;
            }
            assert!(start.elapsed() < Duration::from_mins(2), "{st:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
        let took = start.elapsed();
        let st = s.stats();
        let far = st.wanted;
        eprintln!(
            "{far} chunks resident in {took:?}, {} bytes",
            st.resident_bytes
        );
        // Steady state: nothing arrives, the camera creeps along, so only bookkeeping remains.
        let mut times: Vec<f64> = (0..120)
            .map(|i| {
                let t = Instant::now();
                s.update(&pos(512.0 + i as f32 * 0.1, 0), Vec3::X, &mut sink);
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        eprintln!(
            "steady update: median {:.3} ms, max {:.3} ms",
            times[60], times[119]
        );
        // 50 km reached: a far cell 12 L1 cells (49 km) out along x is resident.
        let edge = ChunkId::l1(I64Vec3::new(12, 0, 0), crate::stream::Detail::Far);
        assert!(s.is_resident(&edge));
        let budget = if cfg!(debug_assertions) {
            Duration::from_mins(1)
        } else {
            Duration::from_secs(10)
        };
        assert!(took < budget, "took {took:?}");
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn band_radii_change_live() {
        let mut s = streamer(StreamerConfig {
            stream: small(),
            upload_budget_bytes: 8 << 20,
            memory_budget_bytes: u64::MAX,
        });
        let mut sink = FakeSink::default();
        let settle = |s: &mut Streamer, sink: &mut FakeSink| {
            let t = Instant::now();
            loop {
                s.update(&pos(10.0, 0), Vec3::X, sink);
                let st = s.stats();
                if st.pool.queued + st.pool.running + st.ready == 0 {
                    return;
                }
                assert!(t.elapsed() < Duration::from_mins(2));
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        settle(&mut s, &mut sink);
        let before = s.stats();
        let mut cfg = small();
        cfg.l1_far_radius_m = 2048.0;
        cfg.l2_mid_radius_m = 600.0;
        s.set_stream_config(cfg);
        settle(&mut s, &mut sink);
        let after = s.stats();
        assert!(after.wanted < before.wanted, "{before:?} {after:?}");
        assert!(after.visible < before.visible);
        // Widening again brings the chunks back, loaded from the resident set or rebuilt.
        s.set_stream_config(small());
        settle(&mut s, &mut sink);
        assert_eq!(s.stats().wanted, before.wanted);
        assert_eq!(s.stats().visible, before.visible);
    }
}
