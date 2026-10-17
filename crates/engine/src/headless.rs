//! Offscreen rendering without a window, for tests and tools.

use crate::camera::Camera;
use crate::mesh::Mesh;
use crate::readback::{self, ReadbackError};
use crate::renderer::{DebugBox, FrameInput, FrameParams, Renderer};
use worldgen::coord::WorldPos;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
}

impl Headless {
    /// `None` when the machine has no usable adapter (no GPU and no software Vulkan such as lavapipe).
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            // GL is deliberately excluded: Mesa's GL-on-D3D12 under WSL crashes, and the game targets Vulkan/DX12.
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("headless"),
                required_features: adapter
                    .features()
                    .intersection(wgpu::Features::INDIRECT_FIRST_INSTANCE),
                required_limits: adapter.limits(),
                ..Default::default()
            }),
        )
        .ok()?;
        Some(Self {
            device,
            queue,
            adapter_name: format!(
                "{} ({:?})",
                adapter.get_info().name,
                adapter.get_info().backend
            ),
        })
    }

    /// Renders one frame with a fresh renderer and returns tightly packed sRGB RGBA8 pixels.
    pub fn render(
        &self,
        meshes: &[(WorldPos, Mesh)],
        camera: &Camera,
        size: (u32, u32),
        params: FrameParams,
        boxes: &[DebugBox],
    ) -> Result<Vec<u8>, ReadbackError> {
        let mut renderer = Renderer::new(&self.device, FORMAT, size);
        renderer.set_scene(&self.device, &self.queue, meshes);
        self.render_with(&mut renderer, camera, size, params, boxes)
    }

    /// Like [`Headless::render`] but with a caller-owned renderer, whose scene is already set.
    pub fn render_with(
        &self,
        renderer: &mut Renderer,
        camera: &Camera,
        size: (u32, u32),
        params: FrameParams,
        boxes: &[DebugBox],
    ) -> Result<Vec<u8>, ReadbackError> {
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless"),
            });
        renderer.render(
            &self.queue,
            &mut encoder,
            &view,
            &FrameInput {
                camera,
                size,
                params,
                boxes,
                timer: None,
            },
        );
        self.queue.submit([encoder.finish()]);
        readback::read_texture_rgba8(&self.device, &self.queue, &target)
    }
}

/// Largest per-channel difference and the mean absolute difference between two RGBA8 buffers.
pub fn image_diff(a: &[u8], b: &[u8]) -> (u8, f64) {
    assert_eq!(a.len(), b.len(), "image sizes differ");
    let mut max = 0u8;
    let mut sum = 0u64;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += u64::from(d);
    }
    (max, sum as f64 / a.len().max(1) as f64)
}
