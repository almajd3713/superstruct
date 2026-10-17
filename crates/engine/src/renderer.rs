//! Forward renderer: reversed-Z depth, per-draw camera-relative origins, greybox shading, debug lines.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use worldgen::coord::WorldPos;

use crate::camera::{Camera, camera_relative};
use crate::mesh::Mesh;
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

#[derive(Clone, Copy)]
struct Draw {
    origin: WorldPos,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
}

/// Everything that varies per frame.
#[derive(Clone, Copy)]
pub struct FrameInput<'a> {
    pub camera: &'a Camera,
    pub size: (u32, u32),
    pub params: FrameParams,
    pub boxes: &'a [DebugBox],
}

#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    pub fog_density: f32,
    pub grid: bool,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            fog_density: 0.0008,
            grid: true,
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
    scene_pipeline: wgpu::RenderPipeline,
    lines_pipeline: wgpu::RenderPipeline,
    vertex_buf: Option<wgpu::Buffer>,
    index_buf: Option<wgpu::Buffer>,
    draws: Vec<Draw>,
    origins_scratch: Vec<[f32; 4]>,
    lines_buf: wgpu::Buffer,
    lines_scratch: Vec<LineVertex>,
}

fn scene_source() -> String {
    format!(
        "{}\n{}",
        include_str!("../shaders/common.wgsl"),
        include_str!("../shaders/scene.wgsl")
    )
}

fn lines_source() -> String {
    format!(
        "{}\n{}",
        include_str!("../shaders/common.wgsl"),
        include_str!("../shaders/lines.wgsl")
    )
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
        let scene_pipeline = build_pipeline(
            device,
            "scene",
            &scene_source(),
            &scene_layout,
            format,
            &[Some(Vertex::LAYOUT)],
            wgpu::PrimitiveTopology::TriangleList,
            true,
        );
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
        };
        let lines_pipeline = build_pipeline(
            device,
            "lines",
            &lines_source(),
            &lines_layout,
            format,
            &[Some(line_layout)],
            wgpu::PrimitiveTopology::LineList,
            false,
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
            scene_pipeline,
            lines_pipeline,
            vertex_buf: None,
            index_buf: None,
            draws: Vec::new(),
            origins_scratch: Vec::with_capacity(MAX_DRAWS),
            lines_buf,
            lines_scratch: Vec::with_capacity(4096),
        }
    }

    /// Replaces the scene with `meshes`, each placed at a world-space origin. Vertices are origin-local.
    pub fn set_scene(&mut self, device: &wgpu::Device, meshes: &[(WorldPos, Mesh)]) {
        use wgpu::util::DeviceExt;
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        self.draws.clear();
        for (origin, mesh) in meshes.iter().take(MAX_DRAWS) {
            self.draws.push(Draw {
                origin: *origin,
                first_index: indices.len() as u32,
                index_count: mesh.indices.len() as u32,
                base_vertex: i32::try_from(vertices.len()).unwrap_or(i32::MAX),
            });
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend_from_slice(&mesh.indices);
        }
        let make = |label, usage, data: &[u8]| {
            (!data.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: data,
                    usage,
                })
            })
        };
        self.vertex_buf = make(
            "scene vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&vertices),
        );
        self.index_buf = make(
            "scene indices",
            wgpu::BufferUsages::INDEX,
            bytemuck::cast_slice(&indices),
        );
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size != self.depth_size {
            self.depth_view = create_depth(device, size.0, size.1);
            self.depth_size = size;
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
        } = *frame;
        let aspect = size.0 as f32 / size.1 as f32;
        let view_proj: Mat4 = camera.view_proj(aspect);
        let local = camera.pos.local();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_local: [local.x, local.y, local.z, 0.0],
            params: [params.fog_density, f32::from(params.grid), 0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        self.origins_scratch.clear();
        self.origins_scratch.extend(self.draws.iter().map(|d| {
            let r = camera_relative(&d.origin, &camera.pos);
            [r.x, r.y, r.z, 0.0]
        }));
        if !self.origins_scratch.is_empty() {
            queue.write_buffer(
                &self.draws_buf,
                0,
                bytemuck::cast_slice(&self.origins_scratch),
            );
        }

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
                        r: 0.02,
                        g: 0.025,
                        b: 0.03,
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
            ..Default::default()
        });
        if let (Some(vb), Some(ib)) = (&self.vertex_buf, &self.index_buf) {
            pass.set_pipeline(&self.scene_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &self.draws_bg, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            for (i, d) in self.draws.iter().enumerate() {
                let i = i as u32;
                pass.draw_indexed(
                    d.first_index..d.first_index + d.index_count,
                    d.base_vertex,
                    i..i + 1,
                );
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
