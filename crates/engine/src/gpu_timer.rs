//! GPU timestamp zones, one begin/end pair per render pass, read back a few frames late.
//! Without `TIMESTAMP_QUERY` the timer is inert and reports nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Passes that are timed. Each owns two queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Scene = 0,
    Ui = 1,
    Cull = 2,
    Prepass = 3,
    HiZ = 4,
}

impl Zone {
    pub const ALL: [Zone; 5] = [Zone::Scene, Zone::Ui, Zone::Cull, Zone::Prepass, Zone::HiZ];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Scene => "scene",
            Zone::Ui => "ui",
            Zone::Cull => "cull",
            Zone::Prepass => "prepass",
            Zone::HiZ => "hiz",
        }
    }
}

const ZONES: usize = Zone::ALL.len();
const QUERIES: u32 = (ZONES * 2) as u32;
const SLOTS: usize = 3;

struct Slot {
    readback: wgpu::Buffer,
    /// Set by the map callback once the slot's data can be read.
    ready: Arc<AtomicBool>,
    in_flight: bool,
}

struct Queries {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Slot being recorded this frame, if one was free.
    current: Option<usize>,
    period_ns: f32,
}

pub struct GpuTimer {
    inner: Option<Queries>,
    ms: [f32; ZONES],
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, supported: bool) -> Self {
        if !supported {
            return Self {
                inner: None,
                ms: [0.0; ZONES],
            };
        }
        let bytes = u64::from(QUERIES) * 8;
        let slots = (0..SLOTS)
            .map(|_| Slot {
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                ready: Arc::new(AtomicBool::new(false)),
                in_flight: false,
            })
            .collect();
        Self {
            inner: Some(Queries {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: QUERIES,
                }),
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp resolve"),
                    size: bytes,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                slots,
                current: None,
                period_ns: queue.get_timestamp_period(),
            }),
            ms: [0.0; ZONES],
        }
    }

    pub fn supported(&self) -> bool {
        self.inner.is_some()
    }

    /// Collects finished readbacks and picks a free slot for this frame.
    pub fn begin_frame(&mut self, device: &wgpu::Device) {
        let Some(q) = &mut self.inner else { return };
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut q.slots {
            if slot.in_flight && slot.ready.load(Ordering::Acquire) {
                if let Ok(view) = slot.readback.slice(..).get_mapped_range() {
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    for (i, ms) in self.ms.iter_mut().enumerate() {
                        let (begin, end) = (ticks[i * 2], ticks[i * 2 + 1]);
                        if end >= begin {
                            *ms = (end - begin) as f32 * q.period_ns * 1e-6;
                        }
                    }
                }
                slot.readback.unmap();
                slot.ready.store(false, Ordering::Release);
                slot.in_flight = false;
            }
        }
        q.current = q.slots.iter().position(|s| !s.in_flight);
    }

    /// Timestamp writes for `zone`'s pass, if this frame is being measured.
    pub fn pass_writes(&self, zone: Zone) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let q = self.inner.as_ref()?;
        q.current?;
        let base = zone as u32 * 2;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &q.set,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        })
    }

    /// Like [`GpuTimer::pass_writes`] for a compute pass.
    pub fn compute_pass_writes(&self, zone: Zone) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let q = self.inner.as_ref()?;
        q.current?;
        let base = zone as u32 * 2;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &q.set,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        })
    }

    /// Call after every timed pass has been recorded, before `encoder.finish()`.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(q) = &self.inner else { return };
        let Some(slot) = q.current else { return };
        encoder.resolve_query_set(&q.set, 0..QUERIES, &q.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &q.resolve,
            0,
            &q.slots[slot].readback,
            0,
            u64::from(QUERIES) * 8,
        );
    }

    /// Call after the frame's command buffer was submitted.
    pub fn after_submit(&mut self) {
        let Some(q) = &mut self.inner else { return };
        let Some(slot) = q.current.take() else { return };
        let slot = &mut q.slots[slot];
        let ready = slot.ready.clone();
        slot.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                if r.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
        slot.in_flight = true;
    }

    /// Most recent GPU milliseconds per zone.
    pub fn timings(&self) -> impl Iterator<Item = (Zone, f32)> + '_ {
        Zone::ALL.into_iter().map(|z| (z, self.ms[z as usize]))
    }
}
