//! Forward renderer: reversed-Z depth, per-draw camera-relative origins, greybox shading, debug lines.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use worldgen::coord::WorldPos;

use crate::camera::{Camera, camera_relative};
use crate::gpu_timer::{GpuTimer, Zone};
use crate::mesh::Mesh;
use crate::shaders::ShaderSources;
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

struct GpuChunk {
    handle: ChunkHandle,
    origin: WorldPos,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
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
    pub fog_density: f32,
    pub grid: bool,
    /// 0 clear to 1 black, for the death sequence.
    pub fade: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            fog_density: 0.0008,
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
    draws_buf: wgpu::Buffer,
    draws_bg: wgpu::BindGroup,
    scene_layout: wgpu::PipelineLayout,
    lines_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    shader_error: Option<String>,
    scene_pipeline: wgpu::RenderPipeline,
    lines_pipeline: wgpu::RenderPipeline,
    chunks: Vec<GpuChunk>,
    next_handle: u32,
    gpu_bytes: u64,
    origins_scratch: Vec<[f32; 4]>,
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

        let draws_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw origins"),
            size: (MAX_DRAWS * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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
                resource: draws_buf.as_entire_binding(),
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
        let (scene_pipeline, lines_pipeline) = build_pipelines(
            device,
            &ShaderSources::embedded(),
            &scene_layout,
            &lines_layout,
            format,
        );
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
            draws_buf,
            draws_bg,
            scene_layout,
            lines_layout,
            format,
            shader_error: None,
            scene_pipeline,
            lines_pipeline,
            chunks: Vec::new(),
            next_handle: 0,
            gpu_bytes: 0,
            origins_scratch: Vec::with_capacity(MAX_DRAWS),
            lines_buf,
            lines_scratch: Vec::with_capacity(4096),
        }
    }

    /// Replaces every chunk with `meshes`, each placed at a world-space origin. Vertices are
    /// origin-local.
    pub fn set_scene(&mut self, device: &wgpu::Device, meshes: &[(WorldPos, Mesh)]) {
        let old: Vec<ChunkHandle> = self.chunks.iter().map(|c| c.handle).collect();
        for h in old {
            self.remove_chunk(h);
        }
        for (origin, mesh) in meshes.iter().take(MAX_DRAWS) {
            self.upload_chunk(device, *origin, mesh);
        }
    }

    /// Uploads `mesh` as its own chunk and returns its handle and size in bytes. Empty meshes
    /// get a handle and no draw.
    pub fn upload_chunk(
        &mut self,
        device: &wgpu::Device,
        origin: WorldPos,
        mesh: &Mesh,
    ) -> (ChunkHandle, u64) {
        use wgpu::util::DeviceExt;
        crate::zone!("upload chunk");
        let handle = ChunkHandle(self.next_handle);
        self.next_handle += 1;
        if mesh.indices.is_empty() {
            return (handle, 0);
        }
        let make = |label, usage, data: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage,
            })
        };
        let vertex_buf = make(
            "chunk vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&mesh.vertices),
        );
        let index_buf = make(
            "chunk indices",
            wgpu::BufferUsages::INDEX,
            bytemuck::cast_slice(&mesh.indices),
        );
        let bytes = vertex_buf.size() + index_buf.size();
        self.gpu_bytes += bytes;
        if self.chunks.len() >= MAX_DRAWS {
            tracing::warn!("more than {MAX_DRAWS} chunks resident, the extra ones are not drawn");
        }
        self.chunks.push(GpuChunk {
            handle,
            origin,
            vertex_buf,
            index_buf,
            index_count: mesh.indices.len() as u32,
            bytes,
            visible: true,
        });
        (handle, bytes)
    }

    pub fn remove_chunk(&mut self, handle: ChunkHandle) {
        if let Some(i) = self.chunks.iter().position(|c| c.handle == handle) {
            self.gpu_bytes -= self.chunks.swap_remove(i).bytes;
        }
    }

    /// Hidden chunks stay resident but are not drawn.
    pub fn set_chunk_visible(&mut self, handle: ChunkHandle, visible: bool) {
        if let Some(c) = self.chunks.iter_mut().find(|c| c.handle == handle) {
            c.visible = visible;
        }
    }

    /// Bytes of vertex and index buffers currently resident.
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
    fn write_draw_origins(&mut self, queue: &wgpu::Queue, cam: &WorldPos) {
        self.origins_scratch.clear();
        self.origins_scratch.extend(
            self.chunks
                .iter()
                .filter(|c| c.visible)
                .take(MAX_DRAWS)
                .map(|d| {
                    let r = camera_relative(&d.origin, cam);
                    [r.x, r.y, r.z, 0.0]
                }),
        );
        if !self.origins_scratch.is_empty() {
            queue.write_buffer(
                &self.draws_buf,
                0,
                bytemuck::cast_slice(&self.origins_scratch),
            );
        }
    }

    /// Records the frame's passes into `encoder`.
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
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        self.write_draw_origins(queue, &camera.pos);

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
                        r: 0.02 * f64::from(1.0 - params.fade),
                        g: 0.025 * f64::from(1.0 - params.fade),
                        b: 0.03 * f64::from(1.0 - params.fade),
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
        if self.chunks.iter().any(|c| c.visible) {
            pass.set_pipeline(&self.scene_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &self.draws_bg, &[]);
            for (i, c) in self
                .chunks
                .iter()
                .filter(|c| c.visible)
                .take(MAX_DRAWS)
                .enumerate()
            {
                let i = i as u32;
                pass.set_vertex_buffer(0, c.vertex_buf.slice(..));
                pass.set_index_buffer(c.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..c.index_count, 0, i..i + 1);
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
