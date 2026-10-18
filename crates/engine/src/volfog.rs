//! Froxel volumetric fog (`shaders/fog.wgsl`, `shaders/volfog.wgsl`). A grid of 16 px screen
//! tiles by 64 exponential depth slices holds the light scattered towards the camera and the
//! extinction. Each frame it is injected from the regime's height fog, local fog volumes and the
//! clustered lights, blended with last frame's grid (reprojected, with a jittered depth), and
//! integrated front to back. Scene fragments read the integrated grid; past its far plane the
//! analytic height fog takes over.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use worldgen::coord::WorldPos;

use crate::camera::camera_relative;
use crate::lights::LightSystem;

pub const TILE_PX: u32 = 16;
pub const SLICES: u32 = 64;
pub const NEAR: f32 = 0.1;
/// The grid's far plane in view depth; the analytic fog covers the rest.
pub const FAR: f32 = 250.0;
pub const MAX_VOLUMES: usize = 64;
/// Share of last frame's grid kept once there is one.
pub const HISTORY_BLEND: f32 = 0.88;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// A box of extra fog: `density` is extinction per metre added at its centre, fading out
/// towards its faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogVolume {
    pub centre: WorldPos,
    pub half: Vec3,
    pub density: f32,
}

/// The regime's fog as the shaders take it.
#[derive(Debug, Clone, Copy)]
pub struct FogAtmosphere {
    /// Extinction per metre at the camera's height.
    pub sigma: f32,
    /// Height falloff per metre.
    pub height_k: f32,
    /// Linear fog and ambient scatter colour.
    pub colour: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_rot: [[f32; 4]; 4],
    prev_view_rot: [[f32; 4]; 4],
    grid: [u32; 4],
    screen: [f32; 4],
    lens: [f32; 4],
    colour: [f32; 4],
    medium: [f32; 4],
    delta: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVolume {
    centre: [f32; 4],
    half_density: [f32; 4],
}

fn grid_for(size: (u32, u32)) -> [u32; 3] {
    [size.0.div_ceil(TILE_PX), size.1.div_ceil(TILE_PX), SLICES]
}

fn create_grid(device: &wgpu::Device, label: &str, dims: [u32; 3]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: dims[0],
            height: dims[1],
            depth_or_array_layers: dims[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

struct Grids {
    scatter: [wgpu::TextureView; 2],
    integrated: wgpu::TextureView,
}

impl Grids {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let dims = grid_for(size);
        let view = |t: wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            scatter: [
                view(create_grid(device, "fog scatter A", dims)),
                view(create_grid(device, "fog scatter B", dims)),
            ],
            integrated: view(create_grid(device, "fog integrated", dims)),
        }
    }
}

pub struct VolFog {
    pub enabled: bool,
    /// Scales the light the point lights put into the fog.
    pub light_gain: f32,
    /// Henyey-Greenstein anisotropy: 0 scatters evenly, towards 1 favours looking at the light.
    pub anisotropy: f32,
    device: wgpu::Device,
    inject: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    inject_bgl: wgpu::BindGroupLayout,
    integrate_bgl: wgpu::BindGroupLayout,
    /// The scene pipelines' group 3.
    pub frag_bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    volumes_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    grids: Grids,
    frag_bg: wgpu::BindGroup,
    size: (u32, u32),
    ping: usize,
    frame: u32,
    prev: Option<(WorldPos, Mat4)>,
    volumes: Vec<FogVolume>,
}

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn buf(ty: wgpu::BufferBindingType) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn tex3d() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D3,
        multisampled: false,
    }
}

fn storage3d() -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: FORMAT,
        view_dimension: wgpu::TextureViewDimension::D3,
    }
}

impl VolFog {
    pub fn new(device: &wgpu::Device, size: (u32, u32), source: &str) -> Self {
        use wgpu::BufferBindingType::{Storage, Uniform};
        use wgpu::ShaderStages as S;
        let ro = Storage { read_only: true };
        let inject_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog inject bgl"),
            entries: &[
                entry(0, S::COMPUTE, buf(Uniform)),
                entry(1, S::COMPUTE, buf(Uniform)),
                entry(2, S::COMPUTE, buf(ro)),
                entry(3, S::COMPUTE, buf(ro)),
                entry(4, S::COMPUTE, buf(ro)),
                entry(5, S::COMPUTE, buf(ro)),
                entry(6, S::COMPUTE, tex3d()),
                entry(
                    7,
                    S::COMPUTE,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(8, S::COMPUTE, storage3d()),
            ],
        });
        let integrate_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog integrate bgl"),
            entries: &[
                entry(0, S::COMPUTE, buf(Uniform)),
                entry(1, S::COMPUTE, tex3d()),
                entry(2, S::COMPUTE, storage3d()),
            ],
        });
        let frag_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog frag bgl"),
            entries: &[
                entry(0, S::FRAGMENT, buf(Uniform)),
                entry(1, S::FRAGMENT, tex3d()),
                entry(
                    2,
                    S::FRAGMENT,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let (inject, integrate) = build(device, &inject_bgl, &integrate_bgl, source);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let volumes_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog volumes"),
            size: (MAX_VOLUMES * std::mem::size_of::<GpuVolume>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let grids = Grids::new(device, size);
        let frag_bg = make_frag_bg(device, &frag_bgl, &params, &grids, &sampler);
        Self {
            enabled: false,
            light_gain: 1.0,
            anisotropy: 0.4,
            device: device.clone(),
            inject,
            integrate,
            inject_bgl,
            integrate_bgl,
            frag_bgl,
            params,
            volumes_buf,
            sampler,
            grids,
            frag_bg,
            size,
            ping: 0,
            frame: 0,
            prev: None,
            volumes: Vec::new(),
        }
    }

    pub fn frag_bg(&self) -> &wgpu::BindGroup {
        &self.frag_bg
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.grids = Grids::new(device, size);
        self.frag_bg = make_frag_bg(
            device,
            &self.frag_bgl,
            &self.params,
            &self.grids,
            &self.sampler,
        );
        self.prev = None;
    }

    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = build(device, &self.inject_bgl, &self.integrate_bgl, source);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        (self.inject, self.integrate) = pipelines;
        Ok(())
    }

    pub fn set_volumes(&mut self, volumes: &[FogVolume]) {
        self.volumes.clear();
        self.volumes
            .extend(volumes.iter().take(MAX_VOLUMES).copied());
    }

    /// Writes the parameters and, when enabled, injects and integrates the grid.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cam: &WorldPos,
        view_rot: Mat4,
        fov_y: f32,
        atmos: FogAtmosphere,
        lights: &LightSystem,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let dims = grid_for(self.size);
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let (prev_rot, delta, blend) = match self.prev {
            Some((p, rot)) if self.enabled => {
                let d = camera_relative(&p, cam);
                (rot, d, HISTORY_BLEND)
            }
            _ => (view_rot, Vec3::ZERO, 0.0),
        };
        self.frame = self.frame.wrapping_add(1);
        let params = Params {
            view_rot: view_rot.to_cols_array_2d(),
            prev_view_rot: prev_rot.to_cols_array_2d(),
            grid: [dims[0], dims[1], dims[2], self.frame],
            screen: [self.size.0 as f32, self.size.1 as f32, NEAR, FAR],
            lens: [
                (fov_y * 0.5).tan(),
                aspect,
                blend,
                self.volumes.len() as f32,
            ],
            colour: [
                atmos.colour[0],
                atmos.colour[1],
                atmos.colour[2],
                atmos.height_k,
            ],
            medium: [
                atmos.sigma,
                self.anisotropy,
                self.light_gain,
                f32::from(self.enabled),
            ],
            delta: [delta.x, delta.y, delta.z, 0.0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if !self.enabled {
            self.prev = None;
            return;
        }
        let gpu: Vec<GpuVolume> = self
            .volumes
            .iter()
            .map(|v| {
                let c = camera_relative(&v.centre, cam);
                GpuVolume {
                    centre: [c.x, c.y, c.z, 0.0],
                    half_density: [v.half.x, v.half.y, v.half.z, v.density],
                }
            })
            .collect();
        if !gpu.is_empty() {
            queue.write_buffer(&self.volumes_buf, 0, bytemuck::cast_slice(&gpu));
        }
        let out = self.ping;
        let hist = 1 - self.ping;
        self.ping = hist;
        let inject_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog inject bg"),
            layout: &self.inject_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: lights.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lights.lights_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: lights.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: lights.list.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.volumes_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.grids.scatter[hist]),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&self.grids.scatter[out]),
                },
            ],
        });
        let integrate_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog integrate bg"),
            layout: &self.integrate_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.grids.scatter[out]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.grids.integrated),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fog"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.inject);
        pass.set_bind_group(0, &inject_bg, &[]);
        pass.dispatch_workgroups(
            dims[0].div_ceil(8),
            dims[1].div_ceil(8),
            dims[2].div_ceil(4),
        );
        drop(pass);
        // Separate passes: the integration reads what the injection wrote.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fog integrate"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.integrate);
        pass.set_bind_group(1, &integrate_bg, &[]);
        pass.dispatch_workgroups(dims[0].div_ceil(8), dims[1].div_ceil(8), 1);
        self.prev = Some((*cam, view_rot));
    }
}

fn make_frag_bg(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    params: &wgpu::Buffer,
    grids: &Grids,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fog frag bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&grids.integrated),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn build(
    device: &wgpu::Device,
    inject_bgl: &wgpu::BindGroupLayout,
    integrate_bgl: &wgpu::BindGroupLayout,
    source: &str,
) -> (wgpu::ComputePipeline, wgpu::ComputePipeline) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fog"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let make = |groups: &[Option<&wgpu::BindGroupLayout>], entry: &str| {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(entry),
            bind_group_layouts: groups,
            immediate_size: 0,
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    (
        make(&[Some(inject_bgl)], "cs_inject"),
        // The integration declares its tables as group 1.
        make(&[None, Some(integrate_bgl)], "cs_integrate"),
    )
}
