//! Ambient occlusion candidates (P5.1.4): screen-space and analytic box-distance, selectable and
//! compared in `tests/render.rs`. Runs between the depth prepass and the colour pass, so it needs
//! the prepass; the scene shader multiplies its colour by the result.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use worldgen::coord::WorldPos;

use crate::camera::{NEAR, camera_relative};

/// Most boxes the analytic mode considers; the nearest to the camera win.
pub const MAX_BOXES: usize = 256;
/// Boxes farther than this from the camera are ignored.
pub const BOX_REACH: f32 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AoMode {
    #[default]
    Off,
    /// Hemisphere samples against the depth buffer.
    Ssao,
    /// Signed distance to the nearest of a buffer of boxes, sampled along the normal.
    Analytic,
}

/// An axis-aligned box that occludes, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AoBox {
    pub centre: WorldPos,
    pub half: Vec3,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_rot: [[f32; 4]; 4],
    sizes: [f32; 4],
    lens: [f32; 4],
    tune: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuBox {
    centre: [f32; 4],
    half: [f32; 4],
}

pub struct AoPass {
    pub mode: AoMode,
    /// Reach of the occlusion in metres.
    pub radius: f32,
    pub strength: f32,
    device: wgpu::Device,
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    boxes_buf: wgpu::Buffer,
    view: wgpu::TextureView,
    boxes: Vec<AoBox>,
    size: (u32, u32),
    frame: u32,
}

fn ao_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1))
}

fn create_view(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
    let (w, h) = ao_size(size);
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("ao"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl AoPass {
    pub fn new(device: &wgpu::Device, size: (u32, u32), source: &str) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let buffer = |ty| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ao bgl"),
            entries: &[
                entry(0, buffer(wgpu::BufferBindingType::Uniform)),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    buffer(wgpu::BufferBindingType::Storage { read_only: true }),
                ),
                entry(
                    3,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
            ],
        });
        Self {
            mode: AoMode::Off,
            radius: 1.2,
            strength: 1.0,
            device: device.clone(),
            pipeline: build(device, &bgl, source),
            bgl,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ao params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            boxes_buf: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ao boxes"),
                size: (MAX_BOXES * std::mem::size_of::<GpuBox>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            view: create_view(device, size),
            boxes: Vec::new(),
            size,
            frame: 0,
        }
    }

    /// The AO texture, half the screen size; the scene shader samples it.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Reallocates the AO texture; the caller must rebind it.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) -> bool {
        if size == self.size {
            return false;
        }
        self.size = size;
        self.view = create_view(device, size);
        true
    }

    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = build(device, &self.bgl, source);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        self.pipeline = pipeline;
        Ok(())
    }

    pub fn set_boxes(&mut self, boxes: &[AoBox]) {
        self.boxes.clear();
        self.boxes.extend_from_slice(boxes);
    }

    /// Records the AO pass for this frame from the finished prepass depth.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        cam: &WorldPos,
        view_rot: Mat4,
        fov_y: f32,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut near: Vec<(f32, GpuBox)> = Vec::new();
        if self.mode == AoMode::Analytic {
            for b in &self.boxes {
                let c = camera_relative(&b.centre, cam);
                let reach = b.half.length() + self.radius;
                let d = c.length();
                if d - reach < BOX_REACH {
                    near.push((
                        d,
                        GpuBox {
                            centre: [c.x, c.y, c.z, 0.0],
                            half: [b.half.x, b.half.y, b.half.z, 0.0],
                        },
                    ));
                }
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            near.truncate(MAX_BOXES);
        }
        let gpu: Vec<GpuBox> = near.iter().map(|(_, b)| *b).collect();
        if !gpu.is_empty() {
            queue.write_buffer(&self.boxes_buf, 0, bytemuck::cast_slice(&gpu));
        }
        let (aw, ah) = ao_size(self.size);
        self.frame = self.frame.wrapping_add(1);
        let params = Params {
            view_rot: view_rot.to_cols_array_2d(),
            sizes: [aw as f32, ah as f32, self.size.0 as f32, self.size.1 as f32],
            lens: [
                (fov_y * 0.5).tan(),
                self.size.0 as f32 / self.size.1 as f32,
                NEAR,
                if self.mode == AoMode::Analytic {
                    2.0
                } else {
                    1.0
                },
            ],
            tune: [
                self.radius,
                self.strength,
                gpu.len() as f32,
                self.frame as f32,
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ao bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.boxes_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ao"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(aw.div_ceil(8), ah.div_ceil(8), 1);
    }
}

fn build(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    source: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ao"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ao layout"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ao"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("cs_ao"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}
