//! The SDF horizon pass: fills a window of L1 cell records on the GPU, then marches every pixel's
//! ray through them beyond the far band (`shaders/horizon_march.wgsl`).

use bytemuck::{Pod, Zeroable};
use glam::{I64Vec3, Mat4};
use worldgen::cell::{CellKey, Level};
use worldgen::coord::WorldPos;

use crate::shaders::ShaderSources;

/// The window reaches this many L1 cells (4096 m) each way from the camera's cell.
pub const HORIZON_HALF: u32 = 32;
pub const HORIZON_N: u32 = 2 * HORIZON_HALF + 1;
const RECORD_BYTES: u64 = 128;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CacheParams {
    seed: [u32; 2],
    origin: [[u32; 2]; 3],
    n: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MarchParams {
    inv_view_proj: [[f32; 4]; 4],
    cam_local: [f32; 3],
    t_start: f32,
    t_end: f32,
    n: u32,
    half: u32,
    pad: u32,
    screen: [f32; 2],
    pad2: [f32; 2],
}

fn split(v: i64) -> [u32; 2] {
    let u = v.cast_unsigned();
    [u as u32, (u >> 32) as u32]
}

pub struct HorizonPass {
    cache_pipeline: wgpu::ComputePipeline,
    cache_bgl: wgpu::BindGroupLayout,
    cache_bg: wgpu::BindGroup,
    cache_params: wgpu::Buffer,
    march_pipeline: wgpu::RenderPipeline,
    march_layout: wgpu::PipelineLayout,
    march_bg: wgpu::BindGroup,
    half: HalfRes,
    march_params: wgpu::Buffer,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    /// Cell whose neighbourhood the records currently describe, with the seed they used.
    window: Option<(I64Vec3, u64)>,
    pub enabled: bool,
    /// March at half resolution and upsample, trading sharpness for speed.
    pub half_res: bool,
    /// Distance from the camera at which marching starts (the far band's reach), metres.
    pub t_start: f32,
    /// Where marching stops; the window's edge by default.
    pub t_end: f32,
    drawn: bool,
}

fn build_cache(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::PipelineLayout,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("horizon cache"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("horizon cache"),
        layout: Some(layout),
        module: &module,
        entry_point: Some("cs_cache"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn build_march(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("horizon march"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("horizon march"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_full"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_march"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(crate::renderer::DEPTH_COMPARE),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn storage_entry(
    binding: u32,
    read_only: bool,
    stage: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, stage: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const HALF_COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const HALF_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Everything the half-resolution path needs: pipelines, targets and their bind group.
struct HalfRes {
    march_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    upsample_layout: wgpu::PipelineLayout,
    bgl: wgpu::BindGroupLayout,
    colour_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    bg: wgpu::BindGroup,
}

fn half_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1))
}

fn half_target(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) -> wgpu::TextureView {
    let (w, h) = half_size(size);
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn half_bind_group(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    colour: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("horizon half bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(colour),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(depth),
            },
        ],
    })
}

fn build_half_pipelines(
    device: &wgpu::Device,
    source: &str,
    march_layout: &wgpu::PipelineLayout,
    upsample_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("horizon half"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let vertex = wgpu::VertexState {
        module: &module,
        entry_point: Some("vs_full"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        buffers: &[],
    };
    let march = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("horizon march half"),
        layout: Some(march_layout),
        vertex: vertex.clone(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_march_half"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(HALF_COLOUR.into()), Some(HALF_DEPTH.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let upsample = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("horizon upsample"),
        layout: Some(upsample_layout),
        vertex,
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_upsample"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(crate::renderer::DEPTH_COMPARE),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (march, upsample)
}

impl HorizonPass {
    #[allow(clippy::too_many_lines)]
    pub fn new(
        device: &wgpu::Device,
        sources: &ShaderSources,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        globals_bgl: &wgpu::BindGroupLayout,
        size: (u32, u32),
    ) -> Self {
        let records = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("horizon records"),
            size: u64::from(HORIZON_N).pow(3) * RECORD_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let buf = |label, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let cache_params = buf("horizon cache params", 48);
        let march_params = buf(
            "horizon march params",
            std::mem::size_of::<MarchParams>() as u64,
        );
        let cache_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("horizon cache bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::COMPUTE),
                storage_entry(1, false, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let cache_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("horizon cache layout"),
            bind_group_layouts: &[Some(&cache_bgl)],
            immediate_size: 0,
        });
        let cache_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("horizon cache bg"),
            layout: &cache_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cache_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: records.as_entire_binding(),
                },
            ],
        });
        let march_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("horizon march bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                storage_entry(1, true, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let march_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("horizon march layout"),
            bind_group_layouts: &[Some(globals_bgl), Some(&march_bgl)],
            immediate_size: 0,
        });
        let march_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("horizon march bg"),
            layout: &march_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: march_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: records.as_entire_binding(),
                },
            ],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let half_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("horizon half bgl"),
            entries: &[texture_entry(0), texture_entry(1)],
        });
        let upsample_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("horizon upsample layout"),
            bind_group_layouts: &[Some(globals_bgl), Some(&march_bgl), Some(&half_bgl)],
            immediate_size: 0,
        });
        let (half_march, half_upsample) = build_half_pipelines(
            device,
            &sources.horizon_march,
            &march_layout,
            &upsample_layout,
            format,
            depth_format,
        );
        let colour_view = half_target(device, "horizon half colour", HALF_COLOUR, size);
        let depth_view = half_target(device, "horizon half depth", HALF_DEPTH, size);
        let half_bg = half_bind_group(device, &half_bgl, &colour_view, &depth_view);
        Self {
            half: HalfRes {
                march_pipeline: half_march,
                upsample_pipeline: half_upsample,
                upsample_layout,
                bgl: half_bgl,
                colour_view,
                depth_view,
                bg: half_bg,
            },
            half_res: false,
            cache_pipeline: build_cache(device, &sources.horizon_cache, &cache_layout),
            cache_bgl,
            cache_bg,
            cache_params,
            march_pipeline: build_march(
                device,
                &sources.horizon_march,
                &march_layout,
                format,
                depth_format,
            ),
            march_layout,
            march_bg,
            march_params,
            format,
            depth_format,
            window: None,
            enabled: false,
            t_start: 51_200.0,
            t_end: 1.0e9,
            drawn: false,
        }
    }

    /// Keeps the old pipelines and returns the message on a compile error.
    pub fn reload(&mut self, device: &wgpu::Device, sources: &ShaderSources) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let cache_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("horizon cache layout"),
            bind_group_layouts: &[Some(&self.cache_bgl)],
            immediate_size: 0,
        });
        let cache = build_cache(device, &sources.horizon_cache, &cache_layout);
        let march = build_march(
            device,
            &sources.horizon_march,
            &self.march_layout,
            self.format,
            self.depth_format,
        );
        let half = build_half_pipelines(
            device,
            &sources.horizon_march,
            &self.march_layout,
            &self.half.upsample_layout,
            self.format,
            self.depth_format,
        );
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        self.cache_pipeline = cache;
        self.march_pipeline = march;
        (self.half.march_pipeline, self.half.upsample_pipeline) = half;
        Ok(())
    }

    /// Re-creates the half-resolution targets for a new frame size.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let h = &mut self.half;
        h.colour_view = half_target(device, "horizon half colour", HALF_COLOUR, size);
        h.depth_view = half_target(device, "horizon half depth", HALF_DEPTH, size);
        h.bg = half_bind_group(device, &h.bgl, &h.colour_view, &h.depth_view);
    }

    /// Marches into the half-resolution targets. Call between `prepare` and the main pass, and
    /// only when `half_res` is on.
    pub fn encode_half(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        globals_bg: &wgpu::BindGroup,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        if !self.drawn || !self.half_res {
            return;
        }
        let target = |view| wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("horizon half"),
            color_attachments: &[
                Some(target(&self.half.colour_view)),
                Some(target(&self.half.depth_view)),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            ..Default::default()
        });
        pass.set_pipeline(&self.half.march_pipeline);
        pass.set_bind_group(0, globals_bg, &[]);
        pass.set_bind_group(1, &self.march_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Refreshes the cell records if the camera moved to another L1 cell, and uploads the march
    /// parameters. Records the cache compute pass into `encoder`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cam: &WorldPos,
        inv_view_proj: Mat4,
        size: (u32, u32),
        seed: u64,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        self.drawn = false;
        if !self.enabled {
            return;
        }
        let key = CellKey::containing(Level::L1, cam);
        let Ok(bounds) = key.aabb_world() else {
            return;
        };
        let refill = self.window != Some((key.coords, seed));
        if refill {
            self.window = Some((key.coords, seed));
            let half = i64::from(HORIZON_HALF);
            let origin = key.coords - I64Vec3::splat(half);
            let params = CacheParams {
                seed: [seed as u32, (seed >> 32) as u32],
                origin: [split(origin.x), split(origin.y), split(origin.z)],
                n: HORIZON_N,
                pad: 0,
            };
            queue.write_buffer(&self.cache_params, 0, bytemuck::bytes_of(&params));
        }
        // Opened every frame so its timestamps are always written; empty unless refilling.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("horizon cache"),
            timestamp_writes: timestamps,
        });
        if refill {
            pass.set_pipeline(&self.cache_pipeline);
            pass.set_bind_group(0, &self.cache_bg, &[]);
            pass.dispatch_workgroups(HORIZON_N.pow(3).div_ceil(64), 1, 1);
        }
        drop(pass);
        let local = -bounds.min.delta(cam);
        let march = MarchParams {
            inv_view_proj: inv_view_proj.to_cols_array_2d(),
            cam_local: [local.x as f32, local.y as f32, local.z as f32],
            t_start: self.t_start,
            t_end: self.t_end,
            n: HORIZON_N,
            half: HORIZON_HALF,
            pad: 0,
            screen: [size.0 as f32, size.1 as f32],
            pad2: [0.0; 2],
        };
        queue.write_buffer(&self.march_params, 0, bytemuck::bytes_of(&march));
        self.drawn = true;
    }

    /// Draws the horizon into a pass with the main colour and depth attachments.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals_bg: &wgpu::BindGroup) {
        if !self.drawn {
            return;
        }
        if self.half_res {
            pass.set_pipeline(&self.half.upsample_pipeline);
            pass.set_bind_group(0, globals_bg, &[]);
            pass.set_bind_group(1, &self.march_bg, &[]);
            pass.set_bind_group(2, &self.half.bg, &[]);
        } else {
            pass.set_pipeline(&self.march_pipeline);
            pass.set_bind_group(0, globals_bg, &[]);
            pass.set_bind_group(1, &self.march_bg, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}
