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

/// Everything a frame's culling passes need besides the buffers.
pub struct CullParams {
    pub frustum: Frustum,
    pub view_proj: Mat4,
    pub screen: (u32, u32),
    pub count: u32,
    pub hiz_levels: u32,
    /// Two-phase occlusion culling instead of frustum-only.
    pub occlusion: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    planes: [[f32; 4]; 6],
    view_proj: [[f32; 4]; 4],
    screen: [f32; 2],
    count: u32,
    hiz_levels: u32,
    mode: u32,
    _pad: [u32; 3],
}

const WORKGROUP: u32 = 64;

pub struct CullPass {
    phase1: wgpu::ComputePipeline,
    phase2: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    counters: wgpu::Buffer,
    counters_read: wgpu::Buffer,
    /// Last frame's per-slot visibility, rewritten by phase 2.
    visible: wgpu::Buffer,
    /// Indirect arguments as the CPU wrote them, before culling.
    pub src_buf: wgpu::Buffer,
    /// Set A: drawn first, from last frame's visible set.
    pub args_a: wgpu::Buffer,
    /// Set B: newly visible after the Hi-Z test.
    pub args_b: wgpu::Buffer,
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

fn build_pipelines(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::PipelineLayout,
) -> (wgpu::ComputePipeline, wgpu::ComputePipeline) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cull"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let make = |entry| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    (make("cs_phase1"), make("cs_phase2"))
}

#[allow(clippy::too_many_arguments)]
fn make_bind_group(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    params: &wgpu::Buffer,
    instances: &wgpu::Buffer,
    src: &wgpu::Buffer,
    a: &wgpu::Buffer,
    b: &wgpu::Buffer,
    visible: &wgpu::Buffer,
    counters: &wgpu::Buffer,
    hiz: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let buffers = [params, instances, src, a, b, visible, counters];
    let mut entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, buf)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: buf.as_entire_binding(),
        })
        .collect();
    entries.push(wgpu::BindGroupEntry {
        binding: 7,
        resource: wgpu::BindingResource::TextureView(hiz),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cull bg"),
        layout: bgl,
        entries: &entries,
    })
}

impl CullPass {
    /// `instances` is the renderer's instance buffer, `hiz` a view of every pyramid level, and
    /// `max_draws` sizes the argument and visibility buffers.
    pub fn new(
        device: &wgpu::Device,
        source: &str,
        instances: &wgpu::Buffer,
        hiz: &wgpu::TextureView,
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
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let args_size = (max_draws * 20) as u64;
        let params_buf = buf(
            "cull params",
            std::mem::size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let counters = buf("cull counters", 8, storage | wgpu::BufferUsages::COPY_SRC);
        let counters_read = buf(
            "cull counters readback",
            8,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let visible = buf("cull visible", (max_draws * 4) as u64, storage);
        let src_buf = buf("cull source args", args_size, storage);
        let indirect = storage | wgpu::BufferUsages::INDIRECT;
        let args_a = buf("cull args A", args_size, indirect);
        let args_b = buf("cull args B", args_size, indirect);
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
                storage_entry(5, false),
                storage_entry(6, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let (phase1, phase2) = build_pipelines(device, source, &layout);
        let bind_group = make_bind_group(
            device,
            &bgl,
            &params_buf,
            instances,
            &src_buf,
            &args_a,
            &args_b,
            &visible,
            &counters,
            hiz,
        );
        Self {
            phase1,
            phase2,
            bgl,
            bind_group,
            instances: instances.clone(),
            params_buf,
            counters,
            counters_read,
            visible,
            src_buf,
            args_a,
            args_b,
        }
    }

    /// Points phase 2 at a rebuilt pyramid.
    pub fn rebind_hiz(&mut self, device: &wgpu::Device, hiz: &wgpu::TextureView) {
        self.bind_group = make_bind_group(
            device,
            &self.bgl,
            &self.params_buf,
            &self.instances,
            &self.src_buf,
            &self.args_a,
            &self.args_b,
            &self.visible,
            &self.counters,
            hiz,
        );
    }

    /// Rebuilds the pipelines; on a compile error keeps the old ones and returns the message.
    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull layout"),
            bind_group_layouts: &[Some(&self.bgl)],
            immediate_size: 0,
        });
        let pipelines = build_pipelines(device, source, &layout);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        (self.phase1, self.phase2) = pipelines;
        Ok(())
    }

    /// Marks a freshly assigned slot as not drawn last frame.
    pub fn clear_slot(&self, queue: &wgpu::Queue, slot: u32) {
        queue.write_buffer(&self.visible, u64::from(slot) * 4, &0u32.to_le_bytes());
    }

    /// Frustum test (and, in two-phase mode, the last-frame filter) into set A.
    pub fn record_phase1(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        p: &CullParams,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let params = Params {
            planes: p.frustum.planes.map(|v| v.to_array()),
            view_proj: p.view_proj.to_cols_array_2d(),
            screen: [p.screen.0 as f32, p.screen.1 as f32],
            count: p.count,
            hiz_levels: p.hiz_levels,
            mode: u32::from(p.occlusion),
            _pad: [0; 3],
        };
        queue.write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&params));
        queue.write_buffer(&self.counters, 0, &[0; 8]);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cull phase 1"),
            timestamp_writes: timestamps,
        });
        if p.count > 0 {
            pass.set_pipeline(&self.phase1);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(p.count.div_ceil(WORKGROUP), 1, 1);
        }
    }

    /// Hi-Z test of everything in the frustum into set B. Uses the parameters of phase 1.
    pub fn record_phase2(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        count: u32,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cull phase 2"),
            timestamp_writes: timestamps,
        });
        if count > 0 {
            pass.set_pipeline(&self.phase2);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(count.div_ceil(WORKGROUP), 1, 1);
        }
    }

    /// Copies the set sizes for [`CullPass::read_counts`]; record after the last culling pass.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(&self.counters, 0, &self.counters_read, 0, 8);
    }

    /// Blocks until the last submitted frame's set sizes (A, B) can be read. For tests and
    /// tools.
    pub fn read_counts(&self, device: &wgpu::Device) -> (u32, u32) {
        let slice = self.counters_read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let counts = slice.get_mapped_range().map_or((0, 0), |v| {
            let word =
                |i: usize| u32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
            (word(0), word(1))
        });
        self.counters_read.unmap();
        counts
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
