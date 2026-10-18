//! Hi-Z depth pyramid: level 0 copies the depth buffer, every further level keeps the minimum
//! (farthest, with reversed Z) of each 2x2 block. Built by compute after the depth prepass.

pub const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const TILE: u32 = 8;

pub fn level_count(size: (u32, u32)) -> u32 {
    size.0.max(size.1).max(1).ilog2() + 1
}

/// Size of pyramid level `level` for a base of `size`.
pub fn level_size(size: (u32, u32), level: u32) -> (u32, u32) {
    ((size.0 >> level).max(1), (size.1 >> level).max(1))
}

struct Layouts {
    init: wgpu::PipelineLayout,
    reduce: wgpu::PipelineLayout,
    debug: wgpu::PipelineLayout,
}

struct Pipelines {
    init: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
    debug: wgpu::RenderPipeline,
}

struct Targets {
    texture: wgpu::Texture,
    size: (u32, u32),
    levels: u32,
    init_bg: wgpu::BindGroup,
    reduce_bgs: Vec<wgpu::BindGroup>,
    debug_bg: wgpu::BindGroup,
}

pub struct HizPass {
    layouts: Layouts,
    bgls: [wgpu::BindGroupLayout; 3],
    pipelines: Pipelines,
    debug_buf: wgpu::Buffer,
    targets: Targets,
}

fn bgl(device: &wgpu::Device, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hiz bgl"),
        entries,
    })
}

fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: HIZ_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn build_pipelines(
    device: &wgpu::Device,
    source: &str,
    layouts: &Layouts,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> Pipelines {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("hiz"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let compute = |label, layout, entry| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    let debug = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("hiz debug"),
        layout: Some(&layouts.debug),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_full"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_debug"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    Pipelines {
        init: compute("hiz init", &layouts.init, "cs_init"),
        reduce: compute("hiz reduce", &layouts.reduce, "cs_reduce"),
        debug,
    }
}

impl HizPass {
    pub fn new(
        device: &wgpu::Device,
        source: &str,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        depth_view: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Self {
        use wgpu::ShaderStages as S;
        use wgpu::TextureSampleType as T;
        let bgls = [
            bgl(
                device,
                &[texture_entry(0, S::COMPUTE, T::Depth), storage_entry(1)],
            ),
            bgl(
                device,
                &[
                    texture_entry(0, S::COMPUTE, T::Float { filterable: false }),
                    storage_entry(1),
                ],
            ),
            bgl(
                device,
                &[
                    texture_entry(0, S::FRAGMENT, T::Float { filterable: false }),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: S::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            ),
        ];
        let layout = |b: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("hiz layout"),
                bind_group_layouts: &[Some(b)],
                immediate_size: 0,
            })
        };
        let layouts = Layouts {
            init: layout(&bgls[0]),
            reduce: layout(&bgls[1]),
            debug: layout(&bgls[2]),
        };
        let pipelines = build_pipelines(device, source, &layouts, format, depth_format);
        let debug_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hiz debug level"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let targets = Self::make_targets(device, &bgls, &debug_buf, depth_view, size);
        Self {
            layouts,
            bgls,
            pipelines,
            debug_buf,
            targets,
        }
    }

    fn make_targets(
        device: &wgpu::Device,
        bgls: &[wgpu::BindGroupLayout; 3],
        debug_buf: &wgpu::Buffer,
        depth_view: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Targets {
        let levels = level_count(size);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hiz"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HIZ_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mip = |level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let views: Vec<_> = (0..levels).map(mip).collect();
        let group = |layout, a: wgpu::BindingResource<'_>, b: wgpu::BindingResource<'_>| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hiz bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: a,
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: b,
                    },
                ],
            })
        };
        let init_bg = group(
            &bgls[0],
            wgpu::BindingResource::TextureView(depth_view),
            wgpu::BindingResource::TextureView(&views[0]),
        );
        let reduce_bgs = views
            .windows(2)
            .map(|w| {
                group(
                    &bgls[1],
                    wgpu::BindingResource::TextureView(&w[0]),
                    wgpu::BindingResource::TextureView(&w[1]),
                )
            })
            .collect();
        let all = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let debug_bg = group(
            &bgls[2],
            wgpu::BindingResource::TextureView(&all),
            debug_buf.as_entire_binding(),
        );
        Targets {
            texture,
            size,
            levels,
            init_bg,
            reduce_bgs,
            debug_bg,
        }
    }

    /// Rebuilds the pyramid for a new depth buffer.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        self.targets = Self::make_targets(device, &self.bgls, &self.debug_buf, depth_view, size);
    }

    /// Keeps the old pipelines and returns the message on a compile error.
    pub fn reload(
        &mut self,
        device: &wgpu::Device,
        source: &str,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = build_pipelines(device, source, &self.layouts, format, depth_format);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        self.pipelines = pipelines;
        Ok(())
    }

    pub fn levels(&self) -> u32 {
        self.targets.levels
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.targets.texture
    }

    /// Builds every level from the depth buffer given to [`HizPass::new`] or `resize`.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let t = &self.targets;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("hiz"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipelines.init);
        pass.set_bind_group(0, &t.init_bg, &[]);
        pass.dispatch_workgroups(t.size.0.div_ceil(TILE), t.size.1.div_ceil(TILE), 1);
        pass.set_pipeline(&self.pipelines.reduce);
        for (i, bg) in t.reduce_bgs.iter().enumerate() {
            let (w, h) = level_size(t.size, i as u32 + 1);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(TILE), h.div_ceil(TILE), 1);
        }
    }

    /// Draws `level` over the whole target. Call inside a pass whose depth format matches.
    pub fn draw_debug(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, level: u32) {
        let level = level.min(self.targets.levels - 1);
        queue.write_buffer(&self.debug_buf, 0, &level.to_le_bytes());
        pass.set_pipeline(&self.pipelines.debug);
        pass.set_bind_group(0, &self.targets.debug_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Reads one level back, row-major, blocking. For tests and tools.
    pub fn read_level(&self, device: &wgpu::Device, queue: &wgpu::Queue, level: u32) -> Vec<f32> {
        let (w, h) = level_size(self.targets.size, level);
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hiz readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.targets.texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mut out = Vec::with_capacity((w * h) as usize);
        if let Ok(view) = buf.slice(..).get_mapped_range() {
            for y in 0..h as usize {
                let start = y * row as usize;
                out.extend_from_slice(bytemuck::cast_slice(&view[start..start + w as usize * 4]));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_sizes_halve_down_to_one() {
        assert_eq!(level_count((1, 1)), 1);
        assert_eq!(level_count((2560, 1440)), 12);
        assert_eq!(level_size((2560, 1440), 11), (1, 1));
        assert_eq!(level_size((2560, 1440), 1), (1280, 720));
        assert_eq!(level_size((37, 19), 2), (9, 4));
    }
}
