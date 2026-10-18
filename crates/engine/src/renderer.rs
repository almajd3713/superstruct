//! Forward renderer: reversed-Z depth, per-draw camera-relative origins, greybox shading, debug lines.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use worldgen::coord::WorldPos;

use crate::camera::{Camera, camera_relative};
use crate::cull::{CullPass, Frustum};
use crate::gpu_timer::{GpuTimer, Zone};
use crate::mesh::Mesh;
use crate::shaders::ShaderSources;
use crate::suballoc::{Range, SubAllocator};
use crate::vertex::Vertex;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Reversed Z: 1 at the near plane, 0 at infinity, so "closer" is greater and clear is 0.
pub const DEPTH_COMPARE: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;
pub const DEPTH_CLEAR: f32 = 0.0;
pub const MAX_DRAWS: usize = 4096;
const MAX_LINE_VERTICES: usize = 1 << 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    cam_local: [f32; 4],
    params: [f32; 4],
    fog: [f32; 4],
    ambient: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub rel: [f32; 3],
    pub colour: [f32; 4],
}

/// An axis-aligned cube outline in world space.
#[derive(Clone, Copy, Debug)]
pub struct DebugBox {
    pub min: WorldPos,
    pub size_m: f32,
    pub colour: [f32; 4],
}

/// Identifies an uploaded chunk until it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkHandle(pub u32);

/// Per-chunk record in the instance storage buffer; mirrored by `Instance` in `scene.wgsl`.
/// The culling pass (P3.1.3) reads `bounds` and `mesh` and writes the indirect arguments.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Camera-relative chunk origin; w unused.
    origin: [f32; 4],
    /// Chunk-local bounding sphere: centre xyz, radius w.
    bounds: [f32; 4],
    /// first index, index count, base vertex (bit cast from i32), LOD band (0 until P3.2).
    mesh: [u32; 4],
}

/// Layout of `wgpu`'s indirect indexed draw arguments.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IndirectArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

/// Consecutive instances that live in the same buffer page and share one multi-draw.
#[derive(Clone, Copy)]
struct DrawRun {
    page: usize,
    start: u32,
    count: u32,
}

fn bounding_sphere(vertices: &[Vertex]) -> [f32; 4] {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in vertices {
        let p = Vec3::from(v.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let c = (lo + hi) * 0.5;
    let r = vertices
        .iter()
        .map(|v| Vec3::from(v.position).distance(c))
        .fold(0.0, f32::max);
    [c.x, c.y, c.z, r]
}

/// Vertices and indices per shared buffer page. A chunk bigger than a page gets a page of its
/// own size.
const PAGE_VERTICES: u32 = 1 << 21;
const PAGE_INDICES: u32 = 1 << 22;

/// One pair of shared vertex and index buffers with a sub-allocator for each.
struct MeshPage {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    vertices: SubAllocator,
    indices: SubAllocator,
}

impl MeshPage {
    fn new(device: &wgpu::Device, vertices: u32, indices: u32) -> Self {
        let make = |label, usage, len: u32, stride: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(len) * stride as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            vertex_buf: make(
                "page vertices",
                wgpu::BufferUsages::VERTEX,
                vertices,
                std::mem::size_of::<Vertex>(),
            ),
            index_buf: make("page indices", wgpu::BufferUsages::INDEX, indices, 4),
            vertices: SubAllocator::new(vertices),
            indices: SubAllocator::new(indices),
        }
    }

    /// Both ranges or neither.
    fn alloc(&mut self, nv: u32, ni: u32) -> Option<(Range, Range)> {
        let v = self.vertices.alloc(nv)?;
        let Some(x) = self.indices.alloc(ni) else {
            self.vertices.free(v);
            return None;
        };
        Some((v, x))
    }

    fn bytes(&self) -> u64 {
        self.vertex_buf.size() + self.index_buf.size()
    }
}

struct GpuChunk {
    handle: ChunkHandle,
    origin: WorldPos,
    page: usize,
    vertices: Range,
    indices: Range,
    bounds: [f32; 4],
    bytes: u64,
    visible: bool,
}

/// Everything that varies per frame.
#[derive(Clone, Copy)]
pub struct FrameInput<'a> {
    pub camera: &'a Camera,
    pub size: (u32, u32),
    pub params: FrameParams,
    pub boxes: &'a [DebugBox],
    pub timer: Option<&'a GpuTimer>,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Fog extinction per metre at the camera.
    pub fog_density: f32,
    /// Fog density falls off as `exp(-k * height above the camera)`; 0 is uniform fog.
    pub fog_height_k: f32,
    /// Linear RGB. Also the sky and void colour: distant geometry fades into it.
    pub fog_color: [f32; 3],
    /// Linear RGB tint and strength of the light.
    pub ambient: [f32; 3],
    pub light: f32,
    pub grid: bool,
    /// 0 clear to 1 black, for the death sequence.
    pub fade: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            fog_density: 0.0008,
            fog_height_k: 0.0,
            fog_color: [0.02, 0.025, 0.03],
            ambient: [0.27, 0.27, 0.27],
            light: 0.25,
            grid: true,
            fade: 0.0,
        }
    }
}

pub struct Renderer {
    depth_view: wgpu::TextureView,
    depth_size: (u32, u32),
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    instances_buf: wgpu::Buffer,
    instances_bg: wgpu::BindGroup,
    indirect_buf: wgpu::Buffer,
    cull: CullPass,
    /// Whether indirect draws may use a non-zero first instance.
    indirect_first_instance: bool,
    /// Debug override: draw with plain `draw_indexed` even when indirect draws are available.
    force_direct: bool,
    scene_layout: wgpu::PipelineLayout,
    lines_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    shader_error: Option<String>,
    scene_pipeline: wgpu::RenderPipeline,
    lines_pipeline: wgpu::RenderPipeline,
    chunks: Vec<GpuChunk>,
    pages: Vec<MeshPage>,
    next_handle: u32,
    gpu_bytes: u64,
    page_bytes: u64,
    instance_scratch: Vec<Instance>,
    indirect_scratch: Vec<IndirectArgs>,
    runs: Vec<DrawRun>,
    lines_buf: wgpu::Buffer,
    lines_scratch: Vec<LineVertex>,
}

fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl Renderer {
    #[allow(clippy::too_many_lines)]
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals bg"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let instances_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (MAX_DRAWS * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect args"),
            size: (MAX_DRAWS * std::mem::size_of::<IndirectArgs>()) as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let draws_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draws bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let draws_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draws bg"),
            layout: &draws_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instances_buf.as_entire_binding(),
            }],
        });

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene layout"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&draws_bgl)],
            immediate_size: 0,
        });
        let lines_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines layout"),
            bind_group_layouts: &[Some(&globals_bgl)],
            immediate_size: 0,
        });
        let sources = ShaderSources::embedded();
        let (scene_pipeline, lines_pipeline) =
            build_pipelines(device, &sources, &scene_layout, &lines_layout, format);
        let lines_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug lines"),
            size: (MAX_LINE_VERTICES * std::mem::size_of::<LineVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            depth_view: create_depth(device, size.0, size.1),
            depth_size: size,
            globals_buf,
            globals_bg,
            instances_bg: draws_bg,
            cull: CullPass::new(
                device,
                &sources.cull,
                &instances_buf,
                &indirect_buf,
                MAX_DRAWS,
            ),
            instances_buf,
            indirect_buf,
            force_direct: false,
            indirect_first_instance: device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            scene_layout,
            lines_layout,
            format,
            shader_error: None,
            scene_pipeline,
            lines_pipeline,
            chunks: Vec::new(),
            pages: Vec::new(),
            next_handle: 0,
            gpu_bytes: 0,
            page_bytes: 0,
            instance_scratch: Vec::with_capacity(MAX_DRAWS),
            indirect_scratch: Vec::with_capacity(MAX_DRAWS),
            runs: Vec::new(),
            lines_buf,
            lines_scratch: Vec::with_capacity(4096),
        }
    }

    /// Replaces every chunk with `meshes`, each placed at a world-space origin. Vertices are
    /// origin-local.
    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        meshes: &[(WorldPos, Mesh)],
    ) {
        let old: Vec<ChunkHandle> = self.chunks.iter().map(|c| c.handle).collect();
        for h in old {
            self.remove_chunk(h);
        }
        for (origin, mesh) in meshes.iter().take(MAX_DRAWS) {
            self.upload_chunk(device, queue, *origin, mesh);
        }
    }

    /// Uploads `mesh` into the shared buffers and returns its handle and size in bytes. Empty
    /// meshes get a handle and no draw.
    pub fn upload_chunk(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origin: WorldPos,
        mesh: &Mesh,
    ) -> (ChunkHandle, u64) {
        crate::zone!("upload chunk");
        let handle = ChunkHandle(self.next_handle);
        self.next_handle += 1;
        if mesh.indices.is_empty() {
            return (handle, 0);
        }
        let (nv, ni) = (mesh.vertices.len() as u32, mesh.indices.len() as u32);
        let Some((page, vertices, indices)) = self.allocate(device, nv, ni) else {
            tracing::error!("no room for a chunk of {nv} vertices and {ni} indices; not drawn");
            return (handle, 0);
        };
        let p = &self.pages[page];
        queue.write_buffer(
            &p.vertex_buf,
            u64::from(vertices.offset) * std::mem::size_of::<Vertex>() as u64,
            bytemuck::cast_slice(&mesh.vertices),
        );
        queue.write_buffer(
            &p.index_buf,
            u64::from(indices.offset) * 4,
            bytemuck::cast_slice(&mesh.indices),
        );
        let bytes = u64::from(nv) * std::mem::size_of::<Vertex>() as u64 + u64::from(ni) * 4;
        self.gpu_bytes += bytes;
        if self.chunks.len() >= MAX_DRAWS {
            tracing::warn!("more than {MAX_DRAWS} chunks resident, the extra ones are not drawn");
        }
        self.chunks.push(GpuChunk {
            handle,
            origin,
            page,
            vertices,
            indices,
            bounds: bounding_sphere(&mesh.vertices),
            bytes,
            visible: true,
        });
        (handle, bytes)
    }

    /// Finds room in an existing page, or adds one sized for at least this request.
    fn allocate(
        &mut self,
        device: &wgpu::Device,
        nv: u32,
        ni: u32,
    ) -> Option<(usize, Range, Range)> {
        if let Some(found) = self
            .pages
            .iter_mut()
            .enumerate()
            .find_map(|(i, p)| p.alloc(nv, ni).map(|(v, x)| (i, v, x)))
        {
            return Some(found);
        }
        let mut page = MeshPage::new(device, PAGE_VERTICES.max(nv), PAGE_INDICES.max(ni));
        let (v, x) = page.alloc(nv, ni)?;
        self.page_bytes += page.bytes();
        self.pages.push(page);
        Some((self.pages.len() - 1, v, x))
    }

    pub fn remove_chunk(&mut self, handle: ChunkHandle) {
        if let Some(i) = self.chunks.iter().position(|c| c.handle == handle) {
            let c = self.chunks.swap_remove(i);
            self.gpu_bytes -= c.bytes;
            let p = &mut self.pages[c.page];
            p.vertices.free(c.vertices);
            p.indices.free(c.indices);
        }
    }

    /// Switches between the indirect path and the direct fallback, which must look the same.
    pub fn set_force_direct(&mut self, on: bool) {
        self.force_direct = on;
    }

    /// Instances that survived frustum culling in the last submitted frame. Blocks on the GPU;
    /// for tests and tools.
    pub fn read_visible_count(&self, device: &wgpu::Device) -> u32 {
        self.cull.read_visible_count(device)
    }

    /// Hidden chunks stay resident but are not drawn.
    pub fn set_chunk_visible(&mut self, handle: ChunkHandle, visible: bool) {
        if let Some(c) = self.chunks.iter_mut().find(|c| c.handle == handle) {
            c.visible = visible;
        }
    }

    /// Bytes of shared buffer pages allocated, used or not.
    pub fn page_bytes(&self) -> u64 {
        self.page_bytes
    }

    /// Bytes of vertex and index data currently resident.
    pub fn gpu_bytes(&self) -> u64 {
        self.gpu_bytes
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// The last shader compile error, cleared by the next successful reload.
    pub fn shader_error(&self) -> Option<&str> {
        self.shader_error.as_deref()
    }

    /// Rebuilds the pipelines from `sources`. On any error the previous pipelines keep running
    /// and the error is returned (and kept for [`Renderer::shader_error`]).
    pub fn reload_shaders(
        &mut self,
        device: &wgpu::Device,
        sources: &ShaderSources,
    ) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = build_pipelines(
            device,
            sources,
            &self.scene_layout,
            &self.lines_layout,
            self.format,
        );
        if let Some(err) = pollster::block_on(scope.pop()) {
            let msg = err.to_string();
            tracing::error!("shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.cull.reload(device, &sources.cull) {
            tracing::error!("cull shader reload failed, keeping previous pipeline:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        (self.scene_pipeline, self.lines_pipeline) = pipelines;
        self.shader_error = None;
        tracing::info!("shaders reloaded");
        Ok(())
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size != self.depth_size {
            self.depth_view = create_depth(device, size.0, size.1);
            self.depth_size = size;
        }
    }

    /// Camera-relative origin of every drawn chunk, in draw order.
    /// Rebuilds the instance and indirect-argument arrays for the visible chunks, grouped by
    /// buffer page so each page needs one multi-draw.
    fn write_instances(&mut self, queue: &wgpu::Queue, cam: &WorldPos) {
        self.instance_scratch.clear();
        self.indirect_scratch.clear();
        self.runs.clear();
        let mut order: Vec<&GpuChunk> = self.chunks.iter().filter(|c| c.visible).collect();
        order.truncate(MAX_DRAWS);
        order.sort_by_key(|c| c.page);
        for (i, c) in order.iter().enumerate() {
            let r = camera_relative(&c.origin, cam);
            let base_vertex = c.vertices.offset.cast_signed();
            self.instance_scratch.push(Instance {
                origin: [r.x, r.y, r.z, 0.0],
                bounds: c.bounds,
                mesh: [
                    c.indices.offset,
                    c.indices.len,
                    base_vertex.cast_unsigned(),
                    0,
                ],
            });
            self.indirect_scratch.push(IndirectArgs {
                index_count: c.indices.len,
                instance_count: 1,
                first_index: c.indices.offset,
                base_vertex,
                first_instance: i as u32,
            });
            match self.runs.last_mut() {
                Some(run) if run.page == c.page => run.count += 1,
                _ => self.runs.push(DrawRun {
                    page: c.page,
                    start: i as u32,
                    count: 1,
                }),
            }
        }
        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instances_buf,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
            queue.write_buffer(
                &self.cull.src_buf,
                0,
                bytemuck::cast_slice(&self.indirect_scratch),
            );
        }
    }

    /// Records the frame's passes into `encoder`.
    #[allow(clippy::too_many_lines)]
    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &FrameInput<'_>,
    ) {
        let FrameInput {
            camera,
            size,
            params,
            boxes,
            timer,
        } = *frame;
        let aspect = size.0 as f32 / size.1 as f32;
        let view_proj: Mat4 = camera.view_proj(aspect);
        let local = camera.pos.local();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_local: [local.x, local.y, local.z, 0.0],
            params: [params.fog_density, f32::from(params.grid), params.fade, 0.0],
            fog: [
                params.fog_color[0],
                params.fog_color[1],
                params.fog_color[2],
                params.fog_height_k,
            ],
            ambient: [
                params.ambient[0],
                params.ambient[1],
                params.ambient[2],
                params.light,
            ],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        self.write_instances(queue, &camera.pos);
        self.cull.record(
            queue,
            encoder,
            &Frustum::from_view_proj(view_proj),
            self.instance_scratch.len() as u32,
            timer.and_then(|t| t.compute_pass_writes(Zone::Cull)),
        );

        self.lines_scratch.clear();
        for b in boxes {
            push_box_lines(
                &mut self.lines_scratch,
                camera_relative(&b.min, &camera.pos),
                b.size_m,
                b.colour,
            );
        }
        self.lines_scratch.truncate(MAX_LINE_VERTICES);
        if !self.lines_scratch.is_empty() {
            queue.write_buffer(
                &self.lines_buf,
                0,
                bytemuck::cast_slice(&self.lines_scratch),
            );
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(params.fog_color[0] * (1.0 - params.fade)),
                        g: f64::from(params.fog_color[1] * (1.0 - params.fade)),
                        b: f64::from(params.fog_color[2] * (1.0 - params.fade)),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timer.and_then(|t| t.pass_writes(Zone::Scene)),
            ..Default::default()
        });
        if !self.runs.is_empty() {
            pass.set_pipeline(&self.scene_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &self.instances_bg, &[]);
            let stride = std::mem::size_of::<IndirectArgs>() as u64;
            for run in &self.runs {
                let page = &self.pages[run.page];
                pass.set_vertex_buffer(0, page.vertex_buf.slice(..));
                pass.set_index_buffer(page.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                if self.indirect_first_instance && !self.force_direct {
                    pass.multi_draw_indexed_indirect(
                        &self.indirect_buf,
                        u64::from(run.start) * stride,
                        run.count,
                    );
                } else {
                    let range = run.start as usize..(run.start + run.count) as usize;
                    for (a, i) in self.indirect_scratch[range].iter().zip(run.start..) {
                        let first = a.first_index;
                        pass.draw_indexed(first..first + a.index_count, a.base_vertex, i..i + 1);
                    }
                }
            }
        }
        if !self.lines_scratch.is_empty() {
            pass.set_pipeline(&self.lines_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_vertex_buffer(0, self.lines_buf.slice(..));
            pass.draw(0..self.lines_scratch.len() as u32, 0..1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    topology: wgpu::PrimitiveTopology,
    depth_write: bool,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: (topology == wgpu::PrimitiveTopology::TriangleList)
                .then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(DEPTH_COMPARE),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn push_box_lines(out: &mut Vec<LineVertex>, min: Vec3, size: f32, colour: [f32; 4]) {
    let c = |i: u32| {
        let p =
            min + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) * size;
        LineVertex {
            rel: p.to_array(),
            colour,
        }
    };
    for a in 0..8u32 {
        for bit in [1u32, 2, 4] {
            let b = a | bit;
            if b != a {
                out.push(c(a));
                out.push(c(b));
            }
        }
    }
}

/// Clip-space depth of a view-space distance `z` in front of the camera, for tests.
pub fn depth_at(proj: Mat4, z: f32) -> f32 {
    let c = proj * Vec4::new(0.0, 0.0, -z, 1.0);
    c.z / c.w
}

fn build_pipelines(
    device: &wgpu::Device,
    sources: &ShaderSources,
    scene_layout: &wgpu::PipelineLayout,
    lines_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let scene = build_pipeline(
        device,
        "scene",
        &sources.scene,
        scene_layout,
        format,
        &[Some(Vertex::LAYOUT)],
        wgpu::PrimitiveTopology::TriangleList,
        true,
    );
    let line_vertex = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<LineVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
    };
    let lines = build_pipeline(
        device,
        "lines",
        &sources.lines,
        lines_layout,
        format,
        &[Some(line_vertex)],
        wgpu::PrimitiveTopology::LineList,
        false,
    );
    (scene, lines)
}
