//! GPU frustum culling: a compute pass that turns per-instance bounds into indirect draw
//! arguments. [`Frustum`] is the CPU reference the GPU result is tested against.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

/// Inward-facing, normalised planes of a reversed-Z infinite frustum, so there is no far plane.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    pub planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb-Hartmann extraction. `view_proj` maps camera-relative points to clip space with
    /// depth in `0..=w` (wgpu), reversed so that the near plane is `z = w`.
    pub fn from_view_proj(m: Mat4) -> Self {
        let row = |i: usize| Vec4::new(m.x_axis[i], m.y_axis[i], m.z_axis[i], m.w_axis[i]);
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        // The sixth slot would be the far plane, which sits at infinity; a plane that always
        // passes keeps the array fixed-size.
        let always = Vec4::new(0.0, 0.0, 0.0, 1.0);
        let raw = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 - r2, always];
        Self {
            planes: raw.map(|p| {
                let len = p.truncate().length();
                if len > 0.0 { p / len } else { always }
            }),
        }
    }

    /// Whether a sphere at camera-relative `centre` touches the frustum.
    pub fn sphere_visible(&self, centre: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|p| p.truncate().dot(centre) + p.w >= -radius)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    planes: [[f32; 4]; 6],
    count: u32,
    _pad: [u32; 3],
}

const WORKGROUP: u32 = 64;

pub struct CullPass {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    params_buf: wgpu::Buffer,
    counter_buf: wgpu::Buffer,
    counter_read: wgpu::Buffer,
    /// Indirect arguments as the CPU wrote them, before culling.
    pub src_buf: wgpu::Buffer,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn build_pipeline(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::PipelineLayout,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cull"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cull"),
        layout: Some(layout),
        module: &module,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

impl CullPass {
    /// `instances` and `out_args` are the renderer's buffers; `max_draws` sizes the argument
    /// buffers (20 bytes each).
    pub fn new(
        device: &wgpu::Device,
        source: &str,
        instances: &wgpu::Buffer,
        out_args: &wgpu::Buffer,
        max_draws: usize,
    ) -> Self {
        let buf = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let params_buf = buf(
            "cull params",
            std::mem::size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let counter_buf = buf(
            "cull visible count",
            4,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        );
        let counter_read = buf(
            "cull count readback",
            4,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let src_buf = buf(
            "cull source args",
            (max_draws * 20) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
                storage_entry(4, false),
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull bg"),
            layout: &bgl,
            entries: &[
                params_buf.as_entire_binding(),
                instances.as_entire_binding(),
                src_buf.as_entire_binding(),
                out_args.as_entire_binding(),
                counter_buf.as_entire_binding(),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, resource)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource,
            })
            .collect::<Vec<_>>(),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        Self {
            pipeline: build_pipeline(device, source, &layout),
            bind_group,
            params_buf,
            counter_buf,
            counter_read,
            src_buf,
        }
    }

    /// Rebuilds the pipeline; on a compile error keeps the old one and returns the message.
    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = self.pipeline.get_bind_group_layout(0);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = build_pipeline(device, source, &layout);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        self.pipeline = pipeline;
        Ok(())
    }

    /// Culls `count` instances against `frustum`. The visible total lands in the counter, which
    /// is copied for [`CullPass::read_visible_count`].
    pub fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frustum: &Frustum,
        count: u32,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let params = Params {
            planes: frustum.planes.map(|p| p.to_array()),
            count,
            _pad: [0; 3],
        };
        queue.write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&params));
        queue.write_buffer(&self.counter_buf, 0, &0u32.to_le_bytes());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull"),
                timestamp_writes: timestamps,
            });
            if count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.dispatch_workgroups(count.div_ceil(WORKGROUP), 1, 1);
            }
        }
        encoder.copy_buffer_to_buffer(&self.counter_buf, 0, &self.counter_read, 0, 4);
    }

    /// Blocks until the last submitted frame's visible count can be read. For tests and tools.
    pub fn read_visible_count(&self, device: &wgpu::Device) -> u32 {
        let slice = self.counter_read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let n = slice.get_mapped_range().map_or(0, |v| {
            u32::from_le_bytes(v[..4].try_into().unwrap_or([0; 4]))
        });
        self.counter_read.unmap();
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::projection_reversed_infinite;

    #[test]
    fn sphere_test_matches_the_view_cone() {
        let f = Frustum::from_view_proj(projection_reversed_infinite(1.2, 1.0, 0.1));
        // Looking down -Z.
        assert!(f.sphere_visible(Vec3::new(0.0, 0.0, -50.0), 1.0));
        assert!(f.sphere_visible(Vec3::new(0.0, 0.0, -1.0e6), 1.0));
        assert!(!f.sphere_visible(Vec3::new(0.0, 0.0, 50.0), 1.0));
        assert!(!f.sphere_visible(Vec3::new(500.0, 0.0, -10.0), 1.0));
        assert!(f.sphere_visible(Vec3::new(500.0, 0.0, -10.0), 600.0));
        assert!(!f.sphere_visible(Vec3::new(0.0, 0.0, -0.05), 0.01));
    }
}
