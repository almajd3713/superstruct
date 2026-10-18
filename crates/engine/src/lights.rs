//! Clustered forward+ point lights. Every frame the lights are made camera-relative, a compute
//! pass moves them into view space and assigns them to a grid of clusters (64 px tiles, 24
//! exponential depth slices), and scene fragments read the list of their own cluster
//! (`shaders/light_cull.wgsl`, `shaders/lights.wgsl`).

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use worldgen::coord::WorldPos;

use crate::camera::camera_relative;

/// Most lights the buffers hold; with more, the nearest to the camera win.
pub const MAX_LIGHTS: usize = 4096;
pub const TILE_PX: u32 = 64;
pub const SLICES: u32 = 24;
/// Default per-view light budget: how many of the nearest lights are put in the grid.
pub const DEFAULT_BUDGET: usize = 512;
/// Lights kept per cluster; any beyond are dropped.
pub const MAX_PER_CLUSTER: u32 = 64;
/// The grid spans these view depths in metres. Fragments beyond `FAR` get no point lights.
pub const NEAR: f32 = 0.1;
pub const FAR: f32 = 250.0;
const WORKGROUP: u32 = 64;

/// A point light in world space. `intensity` scales `colour`; the light reaches `radius` metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub pos: WorldPos,
    pub colour: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight {
    pos_radius: [f32; 4],
    colour: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_rot: [[f32; 4]; 4],
    grid: [u32; 4],
    screen: [f32; 4],
    lens: [f32; 4],
    counts: [u32; 4],
}

fn bind(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

fn grid_for(size: (u32, u32)) -> (u32, u32) {
    (size.0.div_ceil(TILE_PX), size.1.div_ceil(TILE_PX))
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage,
        mapped_at_creation: false,
    })
}

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub struct LightSystem {
    /// The scene pipelines' group 2: parameters, lights, cluster counts and lists, read-only.
    pub render_bgl: wgpu::BindGroupLayout,
    compute_bgl: wgpu::BindGroupLayout,
    view_pipeline: wgpu::ComputePipeline,
    cull_pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    lights_buf: wgpu::Buffer,
    view_lights: wgpu::Buffer,
    counts: wgpu::Buffer,
    list: wgpu::Buffer,
    compute_bg: wgpu::BindGroup,
    render_bg: wgpu::BindGroup,
    lights: Vec<Light>,
    scratch: Vec<(f32, GpuLight)>,
    size: (u32, u32),
    /// Lights in the grid last frame, for tests and the overlay.
    pub active: u32,
    /// Skip the clusters and test every light per fragment, for comparing against them.
    pub brute_force: bool,
    /// Most lights used in a view; the nearest win (at most `MAX_LIGHTS`).
    pub budget: usize,
}

impl LightSystem {
    pub fn new(device: &wgpu::Device, size: (u32, u32), source: &str) -> Self {
        use wgpu::BufferBindingType::{Storage, Uniform};
        use wgpu::ShaderStages as S;
        let render_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lights render bgl"),
            entries: &[
                entry(0, S::FRAGMENT, Uniform),
                entry(1, S::FRAGMENT, Storage { read_only: true }),
                entry(3, S::FRAGMENT, Storage { read_only: true }),
                entry(4, S::FRAGMENT, Storage { read_only: true }),
            ],
        });
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lights compute bgl"),
            entries: &[
                entry(0, S::COMPUTE, Uniform),
                entry(1, S::COMPUTE, Storage { read_only: true }),
                entry(2, S::COMPUTE, Storage { read_only: false }),
                entry(3, S::COMPUTE, Storage { read_only: false }),
                entry(4, S::COMPUTE, Storage { read_only: false }),
            ],
        });
        let (view_pipeline, cull_pipeline) = build(device, &compute_bgl, source);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let params = buffer(
            device,
            "light params",
            std::mem::size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let lights_buf = buffer(
            device,
            "lights",
            (MAX_LIGHTS * std::mem::size_of::<GpuLight>()) as u64,
            storage,
        );
        let view_lights = buffer(device, "view lights", (MAX_LIGHTS * 16) as u64, storage);
        let (counts, list, compute_bg, render_bg) = Self::sized(
            device,
            size,
            &render_bgl,
            &compute_bgl,
            &params,
            &lights_buf,
            &view_lights,
        );
        Self {
            render_bgl,
            compute_bgl,
            view_pipeline,
            cull_pipeline,
            params,
            lights_buf,
            view_lights,
            counts,
            list,
            compute_bg,
            render_bg,
            lights: Vec::new(),
            scratch: Vec::new(),
            size,
            active: 0,
            brute_force: false,
            budget: DEFAULT_BUDGET,
        }
    }

    #[allow(clippy::type_complexity)]
    fn sized(
        device: &wgpu::Device,
        size: (u32, u32),
        render_bgl: &wgpu::BindGroupLayout,
        compute_bgl: &wgpu::BindGroupLayout,
        params: &wgpu::Buffer,
        lights: &wgpu::Buffer,
        view_lights: &wgpu::Buffer,
    ) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroup) {
        let (nx, ny) = grid_for(size);
        let clusters = u64::from(nx) * u64::from(ny) * u64::from(SLICES);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let counts = buffer(device, "cluster counts", clusters * 4, storage);
        let list = buffer(
            device,
            "cluster lists",
            clusters * u64::from(MAX_PER_CLUSTER) * 4,
            storage,
        );
        let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lights compute bg"),
            layout: compute_bgl,
            entries: &[
                bind(0, params),
                bind(1, lights),
                bind(2, view_lights),
                bind(3, &counts),
                bind(4, &list),
            ],
        });
        let render_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lights render bg"),
            layout: render_bgl,
            entries: &[
                bind(0, params),
                bind(1, lights),
                bind(3, &counts),
                bind(4, &list),
            ],
        });
        (counts, list, compute_bg, render_bg)
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size == self.size {
            return;
        }
        self.size = size;
        let (counts, list, compute_bg, render_bg) = Self::sized(
            device,
            size,
            &self.render_bgl,
            &self.compute_bgl,
            &self.params,
            &self.lights_buf,
            &self.view_lights,
        );
        self.counts = counts;
        self.list = list;
        self.compute_bg = compute_bg;
        self.render_bg = render_bg;
    }

    pub fn render_bg(&self) -> &wgpu::BindGroup {
        &self.render_bg
    }

    /// Rebuilds the compute pipelines; on a compile error the old ones keep running.
    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = build(device, &self.compute_bgl, source);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        (self.view_pipeline, self.cull_pipeline) = pipelines;
        Ok(())
    }

    pub fn set_lights(&mut self, lights: &[Light]) {
        self.lights.clear();
        self.lights.extend_from_slice(lights);
    }

    pub fn len(&self) -> usize {
        self.lights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    /// Uploads the lights relative to `cam` and records the view-space and cluster passes.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cam: &WorldPos,
        view_rot: Mat4,
        fov_y: f32,
        extra: &[Light],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        self.scratch.clear();
        for l in self.lights.iter().chain(extra) {
            let rel = camera_relative(&l.pos, cam);
            // Lights that cannot reach the grid are not worth a slot.
            if rel.length() - l.radius > FAR {
                continue;
            }
            self.scratch.push((
                rel.length_squared(),
                GpuLight {
                    pos_radius: [rel.x, rel.y, rel.z, l.radius],
                    colour: [
                        l.colour[0] * l.intensity,
                        l.colour[1] * l.intensity,
                        l.colour[2] * l.intensity,
                        0.0,
                    ],
                },
            ));
        }
        let budget = self.budget.min(MAX_LIGHTS);
        if self.scratch.len() > budget {
            self.scratch.sort_by(|a, b| a.0.total_cmp(&b.0));
            self.scratch.truncate(budget);
        }
        let gpu: Vec<GpuLight> = self.scratch.iter().map(|(_, g)| *g).collect();
        self.active = gpu.len() as u32;
        let (nx, ny) = grid_for(self.size);
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let params = Params {
            view_rot: view_rot.to_cols_array_2d(),
            grid: [nx, ny, SLICES, MAX_PER_CLUSTER],
            screen: [self.size.0 as f32, self.size.1 as f32, TILE_PX as f32, 0.0],
            lens: [(fov_y * 0.5).tan(), aspect, NEAR, FAR],
            counts: [self.active, u32::from(self.brute_force), 0, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if !gpu.is_empty() {
            queue.write_buffer(&self.lights_buf, 0, bytemuck::cast_slice(&gpu));
        }
        if gpu.is_empty() || self.brute_force {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("light cull"),
            timestamp_writes: timestamps,
        });
        pass.set_bind_group(0, &self.compute_bg, &[]);
        pass.set_pipeline(&self.view_pipeline);
        pass.dispatch_workgroups(self.active.div_ceil(WORKGROUP), 1, 1);
        pass.set_pipeline(&self.cull_pipeline);
        pass.dispatch_workgroups((nx * ny * SLICES).div_ceil(WORKGROUP), 1, 1);
    }
}

fn build(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    source: &str,
) -> (wgpu::ComputePipeline, wgpu::ComputePipeline) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("light cull"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("light cull layout"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    let make = |entry| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    (make("cs_view"), make("cs_cull"))
}
