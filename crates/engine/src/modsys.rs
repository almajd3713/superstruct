//! Instanced L4 modules: one shared kit mesh, and per chunk a list of 16-byte instances grouped
//! by piece. A frame writes the camera-relative origin of every chunk and one indirect record per
//! visible (chunk, piece) group, so the CPU never touches an individual module. Mirrors the
//! far-band path in `renderer.rs`; the vertex shader is `vs_module` in `scene.wgsl`.

use glam::Vec3;
use wgpu::util::DeviceExt;
use worldgen::coord::WorldPos;

use crate::camera::camera_relative;
use crate::cull::Frustum;
use crate::kit::{GLOW_MASK, KitSet, ModuleBatch, ModuleInstance, Piece};
use crate::lights::Light;
use crate::renderer::ChunkHandle;
use crate::suballoc::{Range, SubAllocator};

/// Most module chunks and instances resident at once; beyond that new ones are not drawn.
pub const MOD_MAX_CELLS: usize = 4096;
pub const MOD_MAX_INSTANCES: u32 = 1 << 20;
/// A full-brightness lamp: strength and reach of its point light.
const LAMP_INTENSITY: f32 = 5.0;
const LAMP_RADIUS: f32 = 9.0;
/// The cell slot lives in bits 2..18 of an instance.
const SLOT_SHIFT: u32 = 2;

/// Layout of `wgpu`'s indirect indexed draw arguments.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct IndirectArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

struct Group {
    piece: usize,
    range: Range,
}

pub struct ModChunk {
    pub handle: ChunkHandle,
    origin: WorldPos,
    slot: u32,
    groups: Vec<Group>,
    /// Chunk-local bounding sphere.
    bounds: [f32; 4],
    glow: [f32; 4],
    lamps: Vec<([f32; 3], u8)>,
    bytes: u64,
    visible: bool,
}

pub struct ModuleState {
    pub instances: wgpu::Buffer,
    pub cells: wgpu::Buffer,
    args: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    kit: KitSet,
    alloc: SubAllocator,
    pub chunks: Vec<ModChunk>,
    free_slots: Vec<u32>,
    next_slot: u32,
    cell_scratch: Vec<[[f32; 4]; 2]>,
    pub args_scratch: Vec<IndirectArgs>,
    /// Instances in the groups drawn last frame, for tests and the overlay.
    pub drawn_instances: u32,
}

impl ModuleState {
    pub fn new(device: &wgpu::Device) -> Self {
        let kit = KitSet::new();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let buf = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            instances: buf(
                "module instances",
                u64::from(MOD_MAX_INSTANCES) * std::mem::size_of::<ModuleInstance>() as u64,
                storage,
            ),
            // Two vec4 per cell: origin and fade, then the emissive colour and strength.
            cells: buf("module cells", (MOD_MAX_CELLS * 32) as u64, storage),
            args: buf(
                "module args",
                (MOD_MAX_CELLS * Piece::ALL.len() * std::mem::size_of::<IndirectArgs>()) as u64,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            ),
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kit vertices"),
                contents: bytemuck::cast_slice(&kit.mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kit indices"),
                contents: bytemuck::cast_slice(&kit.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            kit,
            alloc: SubAllocator::new(MOD_MAX_INSTANCES),
            chunks: Vec::new(),
            free_slots: Vec::new(),
            next_slot: 0,
            cell_scratch: Vec::new(),
            args_scratch: Vec::new(),
            drawn_instances: 0,
        }
    }

    /// Uploads a chunk's instances. Empty batches get no draw. Returns the bytes resident.
    pub fn upload(
        &mut self,
        queue: &wgpu::Queue,
        handle: ChunkHandle,
        origin: WorldPos,
        batch: &ModuleBatch,
    ) -> u64 {
        crate::zone!("upload module chunk");
        let total = batch.len() as u32;
        let slot = self.free_slots.pop().unwrap_or(self.next_slot);
        let range = (total > 0 && (slot as usize) < MOD_MAX_CELLS)
            .then(|| self.alloc.alloc(total))
            .flatten();
        let Some(range) = range else {
            if total > 0 {
                tracing::error!("module buffer is full; a chunk of {total} instances is not drawn");
            }
            self.free_slots.push(slot);
            return 0;
        };
        self.next_slot = self.next_slot.max(slot + 1);
        let mut all: Vec<ModuleInstance> = Vec::with_capacity(total as usize);
        let mut groups = Vec::new();
        for (piece, list) in batch.by_piece.iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            groups.push(Group {
                piece,
                range: Range {
                    offset: range.offset + all.len() as u32,
                    len: list.len() as u32,
                },
            });
            all.extend(list.iter().map(|i| ModuleInstance {
                pos: i.pos,
                packed: (i.packed & (3 | GLOW_MASK)) | (slot << SLOT_SHIFT),
            }));
        }
        queue.write_buffer(
            &self.instances,
            u64::from(range.offset) * std::mem::size_of::<ModuleInstance>() as u64,
            bytemuck::cast_slice(&all),
        );
        let bytes = batch.bytes();
        self.chunks.push(ModChunk {
            handle,
            origin,
            slot,
            groups,
            bounds: batch.bounding_sphere(),
            glow: batch.glow,
            lamps: batch.lamps.clone(),
            bytes,
            visible: true,
        });
        bytes
    }

    /// Removes a chunk and returns the bytes it held, or `None` for another kind of handle.
    pub fn remove(&mut self, handle: ChunkHandle) -> Option<u64> {
        let i = self.chunks.iter().position(|c| c.handle == handle)?;
        let c = self.chunks.swap_remove(i);
        let len: u32 = c.groups.iter().map(|g| g.range.len).sum();
        if let Some(first) = c.groups.iter().map(|g| g.range.offset).min() {
            self.alloc.free(Range { offset: first, len });
        }
        self.free_slots.push(c.slot);
        Some(c.bytes)
    }

    pub fn set_visible(&mut self, handle: ChunkHandle, visible: bool) -> bool {
        self.chunks
            .iter_mut()
            .find(|c| c.handle == handle)
            .map(|c| c.visible = visible)
            .is_some()
    }

    pub fn set_origin(&mut self, handle: ChunkHandle, origin: WorldPos) -> bool {
        self.chunks
            .iter_mut()
            .find(|c| c.handle == handle)
            .map(|c| c.origin = origin)
            .is_some()
    }

    /// Writes the cell table and the indirect records of the visible chunks inside `frustum`.
    pub fn prepare(&mut self, queue: &wgpu::Queue, cam: &WorldPos, frustum: &Frustum) {
        self.args_scratch.clear();
        self.cell_scratch.clear();
        self.cell_scratch
            .resize(self.next_slot as usize, [[0.0; 4]; 2]);
        self.drawn_instances = 0;
        for c in &self.chunks {
            let o = camera_relative(&c.origin, cam);
            self.cell_scratch[c.slot as usize] = [[o.x, o.y, o.z, 1.0], c.glow];
        }
        for c in self.chunks.iter().filter(|c| c.visible) {
            let o = camera_relative(&c.origin, cam);
            let centre = o + Vec3::new(c.bounds[0], c.bounds[1], c.bounds[2]);
            if !frustum.sphere_visible(centre, c.bounds[3]) {
                continue;
            }
            for g in &c.groups {
                let r = self.kit.ranges[g.piece];
                self.drawn_instances += g.range.len;
                self.args_scratch.push(IndirectArgs {
                    index_count: r.index_count,
                    instance_count: g.range.len,
                    first_index: r.first_index,
                    base_vertex: r.base_vertex,
                    first_instance: g.range.offset,
                });
            }
        }
        if !self.cell_scratch.is_empty() {
            queue.write_buffer(&self.cells, 0, bytemuck::cast_slice(&self.cell_scratch));
        }
        if !self.args_scratch.is_empty() {
            queue.write_buffer(&self.args, 0, bytemuck::cast_slice(&self.args_scratch));
        }
    }

    /// Draws what `prepare` selected. The caller has bound the globals and the draw tables and
    /// set the pipeline.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, indirect: bool) {
        if self.args_scratch.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        if indirect {
            pass.multi_draw_indexed_indirect(&self.args, 0, self.args_scratch.len() as u32);
        } else {
            for a in &self.args_scratch {
                pass.draw_indexed(
                    a.first_index..a.first_index + a.index_count,
                    a.base_vertex,
                    a.first_instance..a.first_instance + a.instance_count,
                );
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.args_scratch.is_empty()
    }

    /// Point lights for the lit lamps of the visible chunks, tinted by each chunk's emissive
    /// colour and strength. The caller applies the per-view budget.
    pub fn collect_lights(&self, out: &mut Vec<Light>) {
        for c in self.chunks.iter().filter(|c| c.visible) {
            for &(at, level) in &c.lamps {
                let Ok(pos) = c.origin.offset(Vec3::from(at)) else {
                    continue;
                };
                out.push(Light {
                    pos,
                    colour: [c.glow[0], c.glow[1], c.glow[2]],
                    intensity: LAMP_INTENSITY * c.glow[3] * f32::from(level) / 15.0,
                    radius: LAMP_RADIUS,
                    spot: None,
                });
            }
        }
    }
}
