//! Temporary moving quad that makes loop smoothness visible (P1.1.4). Replaced by the real scene in P1.3.

use crate::gpu::Gpu;

#[derive(Clone, Copy)]
pub struct TestObject {
    prev_x: f32,
    x: f32,
    dir: f32,
}

impl Default for TestObject {
    fn default() -> Self {
        Self::new()
    }
}

impl TestObject {
    const SPEED: f32 = 1.2; // clip units per second
    const LIMIT: f32 = 0.85;

    pub fn new() -> Self {
        Self {
            prev_x: 0.0,
            x: 0.0,
            dir: 1.0,
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.prev_x = self.x;
        self.x += self.dir * Self::SPEED * dt;
        if self.x.abs() > Self::LIMIT {
            self.x = self.x.clamp(-Self::LIMIT, Self::LIMIT);
            self.dir = -self.dir;
        }
    }

    pub fn interpolated(&self, alpha: f32) -> f32 {
        self.prev_x + (self.x - self.prev_x) * alpha
    }
}

pub struct TestPass {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl TestPass {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test_object"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/test_object.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_object params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test_object bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test_object bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test_object pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test_object"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(gpu.config.format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform,
            bind_group,
        }
    }

    pub fn draw(&self, gpu: &Gpu, pass: &mut wgpu::RenderPass<'_>, x: f32) {
        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let params = [x, 0.0, 0.06 / aspect, 0.06 * 2.0];
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}
