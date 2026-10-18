//! Stylization prototypes behind toggles: posterize, ordered dither, ink outlines from depth and
//! normals, and film grain (`shaders/post.wgsl`). With every toggle off the scene renders
//! straight to the target and this pass costs nothing.

use bytemuck::{Pod, Zeroable};

use crate::camera::NEAR;

// Four independent toggles, not states of one machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PostSettings {
    pub posterize: bool,
    pub dither: bool,
    pub outlines: bool,
    pub grain: bool,
    /// Photo-mode exposure compensation in stops; the pass runs whenever it is not zero.
    pub exposure_stops: f32,
}

impl PostSettings {
    pub fn any(self) -> bool {
        self.posterize || self.dither || self.outlines || self.grain || self.exposure_stops != 0.0
    }

    pub const ALL: Self = Self {
        posterize: true,
        dither: true,
        outlines: true,
        grain: true,
        exposure_stops: 0.0,
    };

    /// Parses a comma list of `posterize`, `dither`, `outlines`, `grain` or `all`; unknown names
    /// are ignored.
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for word in text.split(',').map(str::trim) {
            match word {
                "posterize" => s.posterize = true,
                "dither" => s.dither = true,
                "outlines" | "ink" => s.outlines = true,
                "grain" => s.grain = true,
                "all" => s = Self::ALL,
                _ => {}
            }
        }
        s
    }

    fn flags(self) -> u32 {
        u32::from(self.posterize)
            | u32::from(self.dither) << 1
            | u32::from(self.outlines) << 2
            | u32::from(self.grain) << 3
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    flags: u32,
    frame: u32,
    levels: f32,
    near: f32,
    lens: [f32; 4],
    /// x: linear exposure multiplier.
    exposure: [f32; 4],
}

pub struct PostPass {
    pub settings: PostSettings,
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    colour: wgpu::TextureView,
    format: wgpu::TextureFormat,
    frame: u32,
}

fn create_colour(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("post colour"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
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

impl PostPass {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        source: &str,
    ) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline = build(device, &bgl, format, source);
        Self {
            settings: PostSettings::default(),
            device: device.clone(),
            pipeline,
            bgl,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            colour: create_colour(device, format, size),
            format,
            frame: 0,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.colour = create_colour(device, self.format, size);
    }

    /// The texture the scene renders into while a toggle is on.
    pub fn colour_view(&self) -> &wgpu::TextureView {
        &self.colour
    }

    /// Rebuilds the pipeline; on a compile error the old one keeps running.
    pub fn reload(&mut self, device: &wgpu::Device, source: &str) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = build(device, &self.bgl, self.format, source);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        self.pipeline = pipeline;
        Ok(())
    }

    /// Records the full-screen pass from the scene colour and depth into `target`.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
        fov_y: f32,
        aspect: f32,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        self.frame = self.frame.wrapping_add(1);
        let params = Params {
            flags: self.settings.flags(),
            frame: self.frame,
            levels: 5.0,
            near: NEAR,
            lens: [(fov_y * 0.5).tan(), aspect, 0.85, 0.07],
            exposure: [self.settings.exposure_stops.exp2(), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.colour),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn build(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    source: &str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("post"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post layout"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("post"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_post"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_post"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_parse_names() {
        let s = PostSettings::parse("posterize, ink,bogus");
        assert!(s.posterize && s.outlines && !s.dither && !s.grain);
        assert_eq!(PostSettings::parse("all"), PostSettings::ALL);
        assert!(!PostSettings::parse("").any());
        assert_eq!(PostSettings::ALL.flags(), 0b1111);
    }
}
