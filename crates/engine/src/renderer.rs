//! Forward renderer: reversed-Z depth, per-draw camera-relative origins, greybox shading, debug lines.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use worldgen::coord::WorldPos;

use crate::ao::{AoBox, AoMode, AoPass};
use crate::camera::{Camera, camera_relative};
use crate::cull::{CullParams, CullPass, Frustum};
use crate::farinst::{FarData, FarInstance, UnitMeshes, unit_meshes};
use crate::gpu_timer::{GpuTimer, Zone};
use crate::hiz::HizPass;
use crate::horizon_pass::HorizonPass;
use crate::kit::ModuleBatch;
use crate::lights::{Light, LightSystem};
use crate::mesh::Mesh;
use crate::modsys::ModuleState;
use crate::noise;
use crate::post::{PostPass, PostSettings};
use crate::shaders::ShaderSources;
use crate::suballoc::{Range, SubAllocator};
use crate::vertex::Vertex;
use crate::volfog::{FogAtmosphere, FogVolume, VolFog};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Reversed Z: 1 at the near plane, 0 at infinity, so "closer" is greater and clear is 0.
pub const DEPTH_COMPARE: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;
pub const DEPTH_CLEAR: f32 = 0.0;
pub const MAX_DRAWS: usize = 4096;
const MAX_LINE_VERTICES: usize = 1 << 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    cam_local: [f32; 4],
    params: [f32; 4],
    fog: [f32; 4],
    ambient: [f32; 4],
    /// `materials::gpu_table`: albedo and emissive, then roughness.
    materials: [[f32; 4]; 2 * crate::materials::SLOTS],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub rel: [f32; 3],
    pub colour: [f32; 4],
}

/// An axis-aligned cube outline in world space.
#[derive(Clone, Copy, Debug)]
pub struct DebugBox {
    pub min: WorldPos,
    pub size_m: f32,
    pub colour: [f32; 4],
}

/// Identifies an uploaded chunk until it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkHandle(pub u32);

/// Per-chunk record in the instance storage buffer; mirrored by `Instance` in `scene.wgsl`.
/// The culling pass (P3.1.3) reads `bounds` and `mesh` and writes the indirect arguments.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Camera-relative chunk origin; w unused.
    origin: [f32; 4],
    /// Chunk-local bounding sphere: centre xyz, radius w.
    bounds: [f32; 4],
    /// first index, index count, base vertex (bit cast from i32), LOD band (0 until P3.2).
    mesh: [u32; 4],
}

/// Layout of `wgpu`'s indirect indexed draw arguments.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IndirectArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

/// Consecutive instances that live in the same buffer page and share one multi-draw.
#[derive(Clone, Copy)]
struct DrawRun {
    page: usize,
    start: u32,
    count: u32,
}

fn bounding_sphere(vertices: &[Vertex]) -> [f32; 4] {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in vertices {
        let p = Vec3::from(v.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let c = (lo + hi) * 0.5;
    let r = vertices
        .iter()
        .map(|v| Vec3::from(v.position).distance(c))
        .fold(0.0, f32::max);
    [c.x, c.y, c.z, r]
}

/// Vertices and indices per shared buffer page. A chunk bigger than a page gets a page of its
/// own size.
const PAGE_VERTICES: u32 = 1 << 21;
const PAGE_INDICES: u32 = 1 << 22;

/// One pair of shared vertex and index buffers with a sub-allocator for each.
struct MeshPage {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    vertices: SubAllocator,
    indices: SubAllocator,
}

impl MeshPage {
    fn new(device: &wgpu::Device, vertices: u32, indices: u32) -> Self {
        let make = |label, usage, len: u32, stride: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(len) * stride as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            vertex_buf: make(
                "page vertices",
                wgpu::BufferUsages::VERTEX,
                vertices,
                std::mem::size_of::<Vertex>(),
            ),
            index_buf: make("page indices", wgpu::BufferUsages::INDEX, indices, 4),
            vertices: SubAllocator::new(vertices),
            indices: SubAllocator::new(indices),
        }
    }

    /// Both ranges or neither.
    fn alloc(&mut self, nv: u32, ni: u32) -> Option<(Range, Range)> {
        let v = self.vertices.alloc(nv)?;
        let Some(x) = self.indices.alloc(ni) else {
            self.vertices.free(v);
            return None;
        };
        Some((v, x))
    }

    fn bytes(&self) -> u64 {
        self.vertex_buf.size() + self.index_buf.size()
    }
}

struct GpuChunk {
    handle: ChunkHandle,
    origin: WorldPos,
    page: usize,
    vertices: Range,
    indices: Range,
    bounds: [f32; 4],
    /// Stable index into the culling pass's per-chunk visibility history.
    slot: u32,
    bytes: u64,
    /// Target: fading in (true) or out (false).
    visible: bool,
    fade: f32,
}

/// Everything that varies per frame.
#[derive(Clone, Copy)]
pub struct FrameInput<'a> {
    pub camera: &'a Camera,
    pub size: (u32, u32),
    pub params: FrameParams,
    pub boxes: &'a [DebugBox],
    pub timer: Option<&'a GpuTimer>,
    /// Seconds since the previous frame, which advances chunk fades.
    pub dt: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Fog extinction per metre at the camera.
    pub fog_density: f32,
    /// Fog density falls off as `exp(-k * height above the camera)`; 0 is uniform fog.
    pub fog_height_k: f32,
    /// Linear RGB. Also the sky and void colour: distant geometry fades into it.
    pub fog_color: [f32; 3],
    /// Linear RGB tint and strength of the light.
    pub ambient: [f32; 3],
    pub light: f32,
    pub grid: bool,
    /// 0 clear to 1 black, for the death sequence.
    pub fade: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            fog_density: 0.0008,
            fog_height_k: 0.0,
            fog_color: [0.02, 0.025, 0.03],
            ambient: [0.27, 0.27, 0.27],
            light: 0.25,
            grid: true,
            fade: 0.0,
        }
    }
}

/// Seconds for a chunk to fade in or out when bands swap or the load radius moves.
const FADE_SECONDS: f32 = 0.5;

/// Signed fade level the shaders read: negative while the chunk fades out.
fn signed_fade(fade: f32, fading_in: bool) -> f32 {
    if fading_in { fade } else { -fade }
}

/// Moves `fade` towards its target by `dt` seconds.
fn step_fade(fade: f32, fading_in: bool, dt: f32) -> f32 {
    let d = dt / FADE_SECONDS;
    if fading_in {
        (fade + d).min(1.0)
    } else {
        (fade - d).max(0.0)
    }
}

/// Most far cells and instances resident at once; beyond that new ones are not drawn.
const FAR_MAX_CELLS: usize = 32768;
const FAR_MAX_INSTANCES: u32 = 3 << 20;

struct FarChunk {
    handle: ChunkHandle,
    origin: WorldPos,
    slot: u32,
    boxes: Range,
    cylinders: Range,
    /// Cell-local bounding sphere.
    bounds: [f32; 4],
    bytes: u64,
    /// Target: fading in (true) or out (false).
    visible: bool,
    fade: f32,
}

/// Far-band chunks as instance ranges, drawn from one shared unit box and cylinder.
struct FarState {
    instances: wgpu::Buffer,
    cells: wgpu::Buffer,
    args: wgpu::Buffer,
    unit_vertices: wgpu::Buffer,
    unit_indices: wgpu::Buffer,
    unit: UnitMeshes,
    alloc: SubAllocator,
    chunks: Vec<FarChunk>,
    free_slots: Vec<u32>,
    next_slot: u32,
    cell_scratch: Vec<[f32; 4]>,
    args_scratch: Vec<IndirectArgs>,
}

impl FarState {
    fn new(device: &wgpu::Device) -> Self {
        use wgpu::util::DeviceExt;
        let unit = unit_meshes();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let buf = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            instances: buf(
                "far instances",
                u64::from(FAR_MAX_INSTANCES) * std::mem::size_of::<FarInstance>() as u64,
                storage,
            ),
            cells: buf("far cells", (FAR_MAX_CELLS * 16) as u64, storage),
            // A box and a cylinder record per cell at most.
            args: buf(
                "far args",
                (FAR_MAX_CELLS * 2 * std::mem::size_of::<IndirectArgs>()) as u64,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            ),
            unit_vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("far unit vertices"),
                contents: bytemuck::cast_slice(&unit.mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            unit_indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("far unit indices"),
                contents: bytemuck::cast_slice(&unit.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            unit,
            alloc: SubAllocator::new(FAR_MAX_INSTANCES),
            chunks: Vec::new(),
            free_slots: Vec::new(),
            next_slot: 0,
            cell_scratch: Vec::new(),
            args_scratch: Vec::new(),
        }
    }

    /// Writes the cell table and the indirect records of the chunks inside `frustum`.
    fn prepare(&mut self, queue: &wgpu::Queue, cam: &WorldPos, frustum: &Frustum) {
        self.args_scratch.clear();
        self.cell_scratch.clear();
        self.cell_scratch.resize(self.next_slot as usize, [0.0; 4]);
        for c in &self.chunks {
            let o = camera_relative(&c.origin, cam);
            self.cell_scratch[c.slot as usize] = [o.x, o.y, o.z, signed_fade(c.fade, c.visible)];
        }
        for c in self.chunks.iter().filter(|c| c.fade > 0.0) {
            let o = camera_relative(&c.origin, cam);
            let centre = o + Vec3::new(c.bounds[0], c.bounds[1], c.bounds[2]);
            if !frustum.sphere_visible(centre, c.bounds[3]) {
                continue;
            }
            let (nb, nc) = (c.boxes.len, c.cylinders.len);
            if nb > 0 {
                self.args_scratch.push(IndirectArgs {
                    index_count: self.unit.box_indices,
                    instance_count: nb,
                    first_index: 0,
                    base_vertex: 0,
                    first_instance: c.boxes.offset,
                });
            }
            if nc > 0 {
                self.args_scratch.push(IndirectArgs {
                    index_count: self.unit.cylinder_indices,
                    instance_count: nc,
                    first_index: self.unit.box_indices,
                    // The unit mesh's indices are absolute, so no base vertex.
                    base_vertex: 0,
                    first_instance: c.cylinders.offset,
                });
            }
        }
        if !self.cell_scratch.is_empty() {
            queue.write_buffer(&self.cells, 0, bytemuck::cast_slice(&self.cell_scratch));
        }
        if !self.args_scratch.is_empty() {
            queue.write_buffer(&self.args, 0, bytemuck::cast_slice(&self.args_scratch));
        }
    }
}

// The flags are independent debug toggles, not states of one machine.
#[allow(clippy::struct_excessive_bools)]
pub struct Renderer {
    depth_view: wgpu::TextureView,
    depth_size: (u32, u32),
    hiz: HizPass,
    /// Pyramid level to draw over the frame, for debugging.
    debug_hiz: Option<u32>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    instances_buf: wgpu::Buffer,
    instances_bg: wgpu::BindGroup,
    cull: CullPass,
    far: FarState,
    modules: ModuleState,
    post: PostPass,
    lights: LightSystem,
    volfog: VolFog,
    ao: AoPass,
    light_scratch: Vec<Light>,
    /// Module chunks that live and die with a mesh chunk: `(mesh, modules)`.
    module_links: Vec<(ChunkHandle, ChunkHandle)>,
    horizon: HorizonPass,
    world_seed: u64,
    /// Two-phase occlusion culling (needs the prepass and indirect draws).
    occlusion: bool,
    /// Some chunk is mid-fade, so the prepass needs its fragment stage.
    fading: bool,
    fades_enabled: bool,
    noise: wgpu::Texture,
    noise_uploaded: bool,
    /// Culling frustum held fixed in the world, with the camera position it was taken at.
    frozen: Option<(Frustum, WorldPos)>,
    freeze_pending: bool,
    free_slots: Vec<u32>,
    next_slot: u32,
    /// Whether indirect draws may use a non-zero first instance.
    indirect_first_instance: bool,
    /// Debug override: draw with plain `draw_indexed` even when indirect draws are available.
    force_direct: bool,
    scene_layout: wgpu::PipelineLayout,
    lines_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    shader_error: Option<String>,
    pipelines: Pipelines,
    /// Draw depth first, then shade only the front-most fragment of each pixel.
    prepass: bool,
    chunks: Vec<GpuChunk>,
    /// Shared buffer pages; a page that holds nothing is dropped (its slot is reused), or the
    /// video memory of every area ever visited would stay allocated.
    pages: Vec<Option<MeshPage>>,
    next_handle: u32,
    gpu_bytes: u64,
    page_bytes: u64,
    instance_scratch: Vec<Instance>,
    indirect_scratch: Vec<IndirectArgs>,
    runs: Vec<DrawRun>,
    lines_buf: wgpu::Buffer,
    lines_scratch: Vec<LineVertex>,
}

fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl Renderer {
    #[allow(clippy::too_many_lines)]
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blue noise"),
            size: wgpu::Extent3d {
                width: noise::TILE as u32,
                height: noise::TILE as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let noise_view = noise.create_view(&wgpu::TextureViewDescriptor::default());
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals bg"),
            layout: &globals_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&noise_view),
                },
            ],
        });

        let instances_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (MAX_DRAWS * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draws bgl"),
            entries: &[0, 1, 2, 3, 4].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let far = FarState::new(device);
        let modules = ModuleState::new(device);
        let draws_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draws bg"),
            layout: &draws_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instances_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: far.instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: far.cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: modules.instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: modules.cells.as_entire_binding(),
                },
            ],
        });

        let sources = ShaderSources::embedded();
        let mut lights = LightSystem::new(device, size, &sources.light_cull);
        let volfog = VolFog::new(device, size, &sources.fog);
        let ao = AoPass::new(device, size, &sources.ao);
        lights.set_ao_view(device, ao.view());
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene layout"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&draws_bgl),
                Some(&lights.render_bgl),
                Some(&volfog.frag_bgl),
            ],
            immediate_size: 0,
        });
        let lines_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines layout"),
            bind_group_layouts: &[Some(&globals_bgl)],
            immediate_size: 0,
        });
        let post = PostPass::new(device, format, size, &sources.post);
        let depth_view = create_depth(device, size.0, size.1);
        let hiz = HizPass::new(
            device,
            &sources.hiz,
            format,
            DEPTH_FORMAT,
            &depth_view,
            size,
        );
        let pipelines = build_pipelines(device, &sources, &scene_layout, &lines_layout, format);
        let lines_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug lines"),
            size: (MAX_LINE_VERTICES * std::mem::size_of::<LineVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let horizon = HorizonPass::new(device, &sources, format, DEPTH_FORMAT, &globals_bgl, size);
        let cull = CullPass::new(
            device,
            &sources.cull,
            &instances_buf,
            hiz.all_levels(),
            MAX_DRAWS,
        );
        Self {
            depth_view,
            depth_size: size,
            hiz,
            debug_hiz: None,
            globals_buf,
            globals_bg,
            instances_bg: draws_bg,
            cull,
            far,
            modules,
            post,
            lights,
            volfog,
            ao,
            light_scratch: Vec::new(),
            module_links: Vec::new(),
            horizon,
            world_seed: 0,
            occlusion: true,
            fading: false,
            fades_enabled: true,
            noise,
            noise_uploaded: false,
            frozen: None,
            freeze_pending: false,
            free_slots: Vec::new(),
            next_slot: 0,
            instances_buf,
            force_direct: false,
            indirect_first_instance: device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            scene_layout,
            lines_layout,
            format,
            shader_error: None,
            pipelines,
            prepass: true,
            chunks: Vec::new(),
            pages: Vec::new(),
            next_handle: 0,
            gpu_bytes: 0,
            page_bytes: 0,
            instance_scratch: Vec::with_capacity(MAX_DRAWS),
            indirect_scratch: Vec::with_capacity(MAX_DRAWS),
            runs: Vec::new(),
            lines_buf,
            lines_scratch: Vec::with_capacity(4096),
        }
    }

    /// Replaces every chunk with `meshes`, each placed at a world-space origin. Vertices are
    /// origin-local.
    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        meshes: &[(WorldPos, Mesh)],
    ) {
        let old: Vec<ChunkHandle> = self.chunks.iter().map(|c| c.handle).collect();
        for h in old {
            self.remove_chunk(h);
        }
        for (origin, mesh) in meshes.iter().take(MAX_DRAWS) {
            self.upload_chunk(device, queue, *origin, mesh);
        }
    }

    /// Uploads `mesh` into the shared buffers and returns its handle and size in bytes. Empty
    /// meshes get a handle and no draw.
    pub fn upload_chunk(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origin: WorldPos,
        mesh: &Mesh,
    ) -> (ChunkHandle, u64) {
        crate::zone!("upload chunk");
        let handle = ChunkHandle(self.next_handle);
        self.next_handle += 1;
        if mesh.indices.is_empty() {
            return (handle, 0);
        }
        let (nv, ni) = (mesh.vertices.len() as u32, mesh.indices.len() as u32);
        let Some((page, vertices, indices)) = self.allocate(device, nv, ni) else {
            tracing::error!("no room for a chunk of {nv} vertices and {ni} indices; not drawn");
            return (handle, 0);
        };
        let slot = self.free_slots.pop().unwrap_or_else(|| {
            self.next_slot += 1;
            self.next_slot - 1
        });
        self.cull.clear_slot(queue, slot.min(MAX_DRAWS as u32 - 1));
        let Some(p) = self.pages[page].as_ref() else {
            return (handle, 0);
        };
        queue.write_buffer(
            &p.vertex_buf,
            u64::from(vertices.offset) * std::mem::size_of::<Vertex>() as u64,
            bytemuck::cast_slice(&mesh.vertices),
        );
        queue.write_buffer(
            &p.index_buf,
            u64::from(indices.offset) * 4,
            bytemuck::cast_slice(&mesh.indices),
        );
        let bytes = u64::from(nv) * std::mem::size_of::<Vertex>() as u64 + u64::from(ni) * 4;
        self.gpu_bytes += bytes;
        if self.chunks.len() >= MAX_DRAWS {
            tracing::warn!("more than {MAX_DRAWS} chunks resident, the extra ones are not drawn");
        }
        self.chunks.push(GpuChunk {
            handle,
            origin,
            page,
            vertices,
            indices,
            bounds: bounding_sphere(&mesh.vertices),
            slot,
            bytes,
            visible: true,
            fade: 1.0,
        });
        (handle, bytes)
    }

    /// Finds room in an existing page, or adds one sized for at least this request.
    fn allocate(
        &mut self,
        device: &wgpu::Device,
        nv: u32,
        ni: u32,
    ) -> Option<(usize, Range, Range)> {
        if let Some(found) = self.pages.iter_mut().enumerate().find_map(|(i, p)| {
            p.as_mut()
                .and_then(|p| p.alloc(nv, ni))
                .map(|(v, x)| (i, v, x))
        }) {
            return Some(found);
        }
        let mut page = MeshPage::new(device, PAGE_VERTICES.max(nv), PAGE_INDICES.max(ni));
        let (v, x) = page.alloc(nv, ni)?;
        self.page_bytes += page.bytes();
        let at = if let Some(i) = self.pages.iter().position(Option::is_none) {
            self.pages[i] = Some(page);
            i
        } else {
            self.pages.push(Some(page));
            self.pages.len() - 1
        };
        Some((at, v, x))
    }

    pub fn remove_chunk(&mut self, handle: ChunkHandle) {
        self.ao.remove_group(handle);
        if let Some(i) = self.module_links.iter().position(|l| l.0 == handle) {
            let (_, modules) = self.module_links.swap_remove(i);
            self.remove_chunk(modules);
        }
        if let Some(i) = self.chunks.iter().position(|c| c.handle == handle) {
            let c = self.chunks.swap_remove(i);
            self.gpu_bytes -= c.bytes;
            if let Some(p) = self.pages[c.page].as_mut() {
                p.vertices.free(c.vertices);
                p.indices.free(c.indices);
                if p.vertices.used() == 0 {
                    self.page_bytes -= p.bytes();
                    self.pages[c.page] = None;
                }
            }
            self.free_slots.push(c.slot);
        } else if let Some(bytes) = self.modules.remove(handle) {
            self.gpu_bytes -= bytes;
        } else if let Some(i) = self.far.chunks.iter().position(|c| c.handle == handle) {
            let c = self.far.chunks.swap_remove(i);
            self.gpu_bytes -= c.bytes;
            self.far.alloc.free(Range {
                offset: c.boxes.offset,
                len: c.boxes.len + c.cylinders.len,
            });
            self.far.free_slots.push(c.slot);
        }
    }

    /// Uploads far-band instances. Empty data gets a handle and no draw.
    pub fn upload_far_chunk(
        &mut self,
        queue: &wgpu::Queue,
        origin: WorldPos,
        data: &FarData,
    ) -> (ChunkHandle, u64) {
        crate::zone!("upload far chunk");
        let handle = ChunkHandle(self.next_handle);
        self.next_handle += 1;
        let total = data.len() as u32;
        let far = &mut self.far;
        let slot = far.free_slots.pop().unwrap_or(far.next_slot);
        let range = (total > 0 && (slot as usize) < FAR_MAX_CELLS)
            .then(|| far.alloc.alloc(total))
            .flatten();
        let Some(range) = range else {
            if total > 0 {
                tracing::error!("far band is full; a chunk of {total} instances is not drawn");
            }
            far.free_slots.push(slot);
            return (handle, 0);
        };
        far.next_slot = far.next_slot.max(slot + 1);
        let mut all: Vec<FarInstance> = data.boxes.iter().chain(&data.cylinders).copied().collect();
        for i in &mut all {
            i.cell = slot;
        }
        queue.write_buffer(
            &far.instances,
            u64::from(range.offset) * std::mem::size_of::<FarInstance>() as u64,
            bytemuck::cast_slice(&all),
        );
        let nb = data.boxes.len() as u32;
        let bytes = data.bytes();
        self.gpu_bytes += bytes;
        far.chunks.push(FarChunk {
            handle,
            origin,
            slot,
            boxes: Range {
                offset: range.offset,
                len: nb,
            },
            cylinders: Range {
                offset: range.offset + nb,
                len: total - nb,
            },
            bounds: data.bounding_sphere(),
            bytes,
            visible: true,
            fade: 1.0,
        });
        (handle, bytes)
    }

    /// Uploads the L4 module instances of one chunk. Empty batches get a handle and no draw.
    pub fn upload_module_chunk(
        &mut self,
        queue: &wgpu::Queue,
        origin: WorldPos,
        batch: &ModuleBatch,
    ) -> (ChunkHandle, u64) {
        let handle = ChunkHandle(self.next_handle);
        self.next_handle += 1;
        let bytes = self.modules.upload(queue, handle, origin, batch);
        self.gpu_bytes += bytes;
        (handle, bytes)
    }

    /// Adds what an interior chunk carries besides its mesh: instanced modules (with their lamps)
    /// and the boxes the AO measures. Both go away with `handle`. Returns the module bytes.
    pub fn attach_chunk_extras(
        &mut self,
        queue: &wgpu::Queue,
        handle: ChunkHandle,
        origin: WorldPos,
        modules: Option<&ModuleBatch>,
        ao_boxes: Vec<(Vec3, Vec3)>,
    ) -> u64 {
        self.ao.add_group(handle, origin, ao_boxes);
        let Some(batch) = modules else {
            return 0;
        };
        let (mh, bytes) = self.upload_module_chunk(queue, origin, batch);
        self.module_links.push((handle, mh));
        bytes
    }

    /// Module instances drawn in the last prepared frame.
    pub fn module_instances_drawn(&self) -> u32 {
        self.modules.drawn_instances
    }

    /// Switches between the indirect path and the direct fallback, which must look the same.
    pub fn set_force_direct(&mut self, on: bool) {
        self.force_direct = on;
    }

    /// Instances drawn from set A and from set B in the last submitted frame. Blocks on the
    /// GPU; for tests and tools.
    pub fn read_drawn_counts(&self, device: &wgpu::Device) -> (u32, u32) {
        self.cull.read_counts(device)
    }

    /// Turns the SDF horizon beyond the far band on or off.
    pub fn set_horizon_enabled(&mut self, on: bool) {
        self.horizon.enabled = on;
    }

    /// Marches the horizon at half resolution and upsamples it.
    pub fn set_horizon_half_res(&mut self, on: bool) {
        self.horizon.half_res = on;
    }

    /// Where the horizon starts, metres from the camera (the far band's reach).
    pub fn set_horizon_start(&mut self, metres: f32) {
        self.horizon.t_start = metres;
    }

    /// Where the horizon stops marching, metres from the camera.
    pub fn set_horizon_end(&mut self, metres: f32) {
        self.horizon.t_end = metres;
    }

    /// The world seed the horizon rules run with.
    pub fn set_world_seed(&mut self, seed: u64) {
        self.world_seed = seed;
    }

    /// Keeps culling against the current frustum while the camera moves on, so the culled set
    /// can be inspected from outside. Occlusion culling pauses meanwhile, since its pyramid
    /// belongs to the live view.
    pub fn set_freeze_culling(&mut self, on: bool) {
        self.freeze_pending = on;
        if !on {
            self.frozen = None;
        }
    }

    pub fn culling_frozen(&self) -> bool {
        self.freeze_pending || self.frozen.is_some()
    }

    /// Turns two-phase occlusion culling on or off; frustum culling always runs.
    pub fn set_occlusion(&mut self, on: bool) {
        self.occlusion = on;
    }

    /// Hidden chunks stay resident but are not drawn.
    /// Moves a chunk to a new world origin, for geometry that travels (lift cars, trams).
    pub fn set_chunk_origin(&mut self, handle: ChunkHandle, origin: WorldPos) {
        if let Some(c) = self.chunks.iter_mut().find(|c| c.handle == handle) {
            c.origin = origin;
        } else {
            self.modules.set_origin(handle, origin);
        }
    }

    /// Starts a freshly uploaded chunk invisible so it fades in.
    pub fn begin_fade_in(&mut self, handle: ChunkHandle) {
        self.set_chunk_fade_state(handle, 0.0, true);
    }

    /// Forces a chunk's fade level and direction; for tests and tools.
    pub fn set_chunk_fade_state(&mut self, handle: ChunkHandle, fade: f32, fading_in: bool) {
        if let Some(c) = self.chunks.iter_mut().find(|c| c.handle == handle) {
            (c.fade, c.visible) = (fade, fading_in);
        } else if let Some(c) = self.far.chunks.iter_mut().find(|c| c.handle == handle) {
            (c.fade, c.visible) = (fade, fading_in);
        }
    }

    /// With fades off, chunks pop in and out at once; for comparing against the fading look.
    pub fn set_fades_enabled(&mut self, on: bool) {
        self.fades_enabled = on;
    }

    fn advance_fades(&mut self, dt: f32) {
        let dt = if self.fades_enabled { dt } else { 1.0e6 };
        let mut any = false;
        let mut step = |fade: &mut f32, target: bool| {
            *fade = step_fade(*fade, target, dt);
            any |= *fade > 0.0 && *fade < 1.0;
        };
        for c in &mut self.chunks {
            step(&mut c.fade, c.visible);
        }
        for c in &mut self.far.chunks {
            step(&mut c.fade, c.visible);
        }
        self.fading = any;
    }

    /// Hidden chunks fade out; they stay resident.
    pub fn set_chunk_visible(&mut self, handle: ChunkHandle, visible: bool) {
        if let Some(&(_, m)) = self.module_links.iter().find(|l| l.0 == handle) {
            self.modules.set_visible(m, visible);
        }
        if let Some(c) = self.chunks.iter_mut().find(|c| c.handle == handle) {
            c.visible = visible;
        } else if let Some(c) = self.far.chunks.iter_mut().find(|c| c.handle == handle) {
            c.visible = visible;
        } else {
            self.modules.set_visible(handle, visible);
        }
    }

    /// Replaces the point lights; the clusters are rebuilt every frame from the nearest ones.
    pub fn set_lights(&mut self, lights: &[Light]) {
        self.lights.set_lights(lights);
    }

    /// Sets the emissive colour (linear) and strength of every module chunk, or `None` to use
    /// each chunk's own.
    pub fn set_module_glow(&mut self, glow: Option<[f32; 4]>) {
        self.modules.glow_override = glow;
    }

    /// Selects the ambient-occlusion candidate (needs the depth prepass).
    pub fn set_ao(&mut self, mode: AoMode) {
        self.ao.mode = mode;
    }

    /// AO reach in metres and strength 0 to 1.
    pub fn set_ao_tuning(&mut self, radius: f32, strength: f32) {
        self.ao.radius = radius;
        self.ao.strength = strength;
    }

    /// Boxes the analytic AO measures distance to (the nearest 256 within 64 m are used).
    pub fn set_ao_boxes(&mut self, boxes: &[AoBox]) {
        self.ao.set_boxes(boxes);
    }

    /// Turns the froxel volumetric fog on; off falls back to the analytic height fog.
    pub fn set_volumetric_fog(&mut self, on: bool) {
        self.volfog.enabled = on;
    }

    /// Extra fog boxes (at most 64) on top of the regime's height fog.
    pub fn set_fog_volumes(&mut self, volumes: &[FogVolume]) {
        self.volfog.set_volumes(volumes);
    }

    /// Fog tuning: how strongly lights scatter into it and how forward-peaked the scattering is.
    pub fn set_fog_scattering(&mut self, light_gain: f32, anisotropy: f32) {
        self.volfog.light_gain = light_gain;
        self.volfog.anisotropy = anisotropy;
    }

    /// The most lights any view uses; the nearest are kept (P5.1.2).
    pub fn set_light_budget(&mut self, budget: usize) {
        self.lights.budget = budget;
    }

    /// Lights in the cluster grid in the last frame.
    pub fn lights_active(&self) -> u32 {
        self.lights.active
    }

    /// Debug: test every light per fragment instead of using the clusters.
    pub fn set_lights_brute_force(&mut self, on: bool) {
        self.lights.brute_force = on;
    }

    /// Turns the stylization prototypes on or off (`post.rs`).
    pub fn set_post(&mut self, settings: PostSettings) {
        self.post.settings = settings;
    }

    pub fn post(&self) -> PostSettings {
        self.post.settings
    }

    /// Bytes of shared buffer pages allocated, used or not.
    pub fn page_bytes(&self) -> u64 {
        self.page_bytes
    }

    /// Bytes of vertex and index data currently resident.
    pub fn gpu_bytes(&self) -> u64 {
        self.gpu_bytes
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len() + self.far.chunks.len() + self.modules.chunks.len()
    }

    /// The last shader compile error, cleared by the next successful reload.
    pub fn shader_error(&self) -> Option<&str> {
        self.shader_error.as_deref()
    }

    /// Rebuilds the pipelines from `sources`. On any error the previous pipelines keep running
    /// and the error is returned (and kept for [`Renderer::shader_error`]).
    pub fn reload_shaders(
        &mut self,
        device: &wgpu::Device,
        sources: &ShaderSources,
    ) -> Result<(), String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = build_pipelines(
            device,
            sources,
            &self.scene_layout,
            &self.lines_layout,
            self.format,
        );
        if let Some(err) = pollster::block_on(scope.pop()) {
            let msg = err.to_string();
            tracing::error!("shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.horizon.reload(device, sources) {
            tracing::error!("horizon shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.cull.reload(device, &sources.cull) {
            tracing::error!("cull shader reload failed, keeping previous pipeline:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self
            .hiz
            .reload(device, &sources.hiz, self.format, DEPTH_FORMAT)
        {
            tracing::error!("hiz shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.ao.reload(device, &sources.ao) {
            tracing::error!("ao shader reload failed, keeping previous pipeline:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.volfog.reload(device, &sources.fog) {
            tracing::error!("fog shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.lights.reload(device, &sources.light_cull) {
            tracing::error!("light shader reload failed, keeping previous pipelines:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        if let Err(msg) = self.post.reload(device, &sources.post) {
            tracing::error!("post shader reload failed, keeping previous pipeline:\n{msg}");
            self.shader_error = Some(msg.clone());
            return Err(msg);
        }
        self.pipelines = pipelines;
        self.shader_error = None;
        tracing::info!("shaders reloaded");
        Ok(())
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size != self.depth_size {
            self.depth_view = create_depth(device, size.0, size.1);
            self.depth_size = size;
            self.hiz.resize(device, &self.depth_view, size);
            self.cull.rebind_hiz(device, self.hiz.all_levels());
            self.horizon.resize(device, size);
            self.post.resize(device, size);
            self.lights.resize(device, size);
            self.volfog.resize(device, size);
            if self.ao.resize(device, size) {
                self.lights.set_ao_view(device, self.ao.view());
            }
        }
    }

    /// Camera-relative origin of every drawn chunk, in draw order.
    /// Rebuilds the instance and indirect-argument arrays for the visible chunks, grouped by
    /// buffer page so each page needs one multi-draw.
    fn write_instances(&mut self, queue: &wgpu::Queue, cam: &WorldPos) {
        self.instance_scratch.clear();
        self.indirect_scratch.clear();
        self.runs.clear();
        let mut order: Vec<&GpuChunk> = self.chunks.iter().filter(|c| c.fade > 0.0).collect();
        order.truncate(MAX_DRAWS);
        order.sort_by_key(|c| c.page);
        for (i, c) in order.iter().enumerate() {
            let r = camera_relative(&c.origin, cam);
            let base_vertex = c.vertices.offset.cast_signed();
            self.instance_scratch.push(Instance {
                origin: [r.x, r.y, r.z, f32::from_bits(c.slot)],
                bounds: c.bounds,
                mesh: [
                    c.indices.offset,
                    c.indices.len,
                    base_vertex.cast_unsigned(),
                    signed_fade(c.fade, c.visible).to_bits(),
                ],
            });
            self.indirect_scratch.push(IndirectArgs {
                index_count: c.indices.len,
                instance_count: 1,
                first_index: c.indices.offset,
                base_vertex,
                first_instance: i as u32,
            });
            match self.runs.last_mut() {
                Some(run) if run.page == c.page => run.count += 1,
                _ => self.runs.push(DrawRun {
                    page: c.page,
                    start: i as u32,
                    count: 1,
                }),
            }
        }
        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instances_buf,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
            queue.write_buffer(
                &self.cull.src_buf,
                0,
                bytemuck::cast_slice(&self.indirect_scratch),
            );
        }
    }

    /// Issues the culled indirect draws (or the direct fallback) with `pipeline`.
    fn draw_chunks(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        set: DrawSet,
    ) {
        let indirect = self.indirect_first_instance && !self.force_direct;
        // The direct fallback draws everything unculled in set A, so B has nothing left.
        if self.runs.is_empty() || (!indirect && set == DrawSet::B) {
            return;
        }
        let args = match set {
            DrawSet::A => &self.cull.args_a,
            DrawSet::B => &self.cull.args_b,
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_bind_group(1, &self.instances_bg, &[]);
        pass.set_bind_group(2, self.lights.render_bg(), &[]);
        pass.set_bind_group(3, self.volfog.frag_bg(), &[]);
        let stride = std::mem::size_of::<IndirectArgs>() as u64;
        for run in &self.runs {
            let Some(page) = self.pages[run.page].as_ref() else {
                continue;
            };
            pass.set_vertex_buffer(0, page.vertex_buf.slice(..));
            pass.set_index_buffer(page.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            if indirect {
                pass.multi_draw_indexed_indirect(args, u64::from(run.start) * stride, run.count);
            } else {
                let range = run.start as usize..(run.start + run.count) as usize;
                for (a, i) in self.indirect_scratch[range].iter().zip(run.start..) {
                    let first = a.first_index;
                    pass.draw_indexed(first..first + a.index_count, a.base_vertex, i..i + 1);
                }
            }
        }
    }

    /// Draws the visible module instances with the shared kit mesh.
    fn draw_modules(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.modules.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipelines.modules);
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_bind_group(1, &self.instances_bg, &[]);
        pass.set_bind_group(2, self.lights.render_bg(), &[]);
        pass.set_bind_group(3, self.volfog.frag_bg(), &[]);
        self.modules
            .draw(pass, self.indirect_first_instance && !self.force_direct);
    }

    /// Draws the visible far cells, after the near geometry so depth testing trims the overdraw.
    fn draw_far(&self, pass: &mut wgpu::RenderPass<'_>) {
        let far = &self.far;
        if far.args_scratch.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipelines.far);
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_bind_group(1, &self.instances_bg, &[]);
        pass.set_bind_group(2, self.lights.render_bg(), &[]);
        pass.set_bind_group(3, self.volfog.frag_bg(), &[]);
        pass.set_vertex_buffer(0, far.unit_vertices.slice(..));
        pass.set_index_buffer(far.unit_indices.slice(..), wgpu::IndexFormat::Uint32);
        if self.indirect_first_instance && !self.force_direct {
            pass.multi_draw_indexed_indirect(&far.args, 0, far.args_scratch.len() as u32);
        } else {
            for a in &far.args_scratch {
                let first = a.first_index;
                pass.draw_indexed(
                    first..first + a.index_count,
                    a.base_vertex,
                    a.first_instance..a.first_instance + a.instance_count,
                );
            }
        }
    }

    /// Depth-only pass over one draw set. The first pass clears depth, a second one adds to it.
    fn prepass_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        set: DrawSet,
        clear: bool,
        timer: Option<&GpuTimer>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth prepass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(DEPTH_CLEAR)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timer.and_then(|t| t.pass_writes(Zone::Prepass)),
            ..Default::default()
        });
        let pipeline = if self.fading {
            &self.pipelines.prepass_fade
        } else {
            &self.pipelines.prepass
        };
        self.draw_chunks(&mut pass, pipeline, set);
    }

    /// Draws one Hi-Z level over the whole frame; `None` turns it off.
    pub fn set_debug_hiz(&mut self, level: Option<u32>) {
        self.debug_hiz = level;
    }

    pub fn hiz(&self) -> &HizPass {
        &self.hiz
    }

    pub fn set_prepass(&mut self, on: bool) {
        self.prepass = on;
    }

    /// Records the frame's passes into `encoder`.
    #[allow(clippy::too_many_lines)]
    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &FrameInput<'_>,
    ) {
        let FrameInput {
            camera,
            size,
            params,
            boxes,
            timer,
            dt,
        } = *frame;
        if !self.noise_uploaded {
            self.noise_uploaded = true;
            queue.write_texture(
                self.noise.as_image_copy(),
                noise::blue_noise_tile(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(noise::TILE as u32),
                    rows_per_image: None,
                },
                self.noise.size(),
            );
        }
        self.advance_fades(dt);
        let aspect = size.0 as f32 / size.1 as f32;
        let view_proj: Mat4 = camera.view_proj(aspect);
        let local = camera.pos.local();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_local: [local.x, local.y, local.z, 0.0],
            params: [params.fog_density, f32::from(params.grid), params.fade, 0.0],
            fog: [
                params.fog_color[0],
                params.fog_color[1],
                params.fog_color[2],
                params.fog_height_k,
            ],
            materials: crate::materials::gpu_table(),
            ambient: [
                params.ambient[0],
                params.ambient[1],
                params.ambient[2],
                params.light,
            ],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        self.write_instances(queue, &camera.pos);
        let count = self.instance_scratch.len() as u32;
        // Occlusion culling needs a depth pass to build the pyramid from and GPU-written
        // draw arguments to act on.
        if self.freeze_pending {
            self.frozen = Some((Frustum::from_view_proj(view_proj), camera.pos));
            self.freeze_pending = false;
        }
        let frustum = match &self.frozen {
            Some((f, at)) => f.translated(camera_relative(&camera.pos, at)),
            None => Frustum::from_view_proj(view_proj),
        };
        self.lights.ao_on = self.prepass && self.ao.mode != AoMode::Off;
        self.light_scratch.clear();
        self.modules.collect_lights(&mut self.light_scratch);
        self.lights.record(
            queue,
            encoder,
            &camera.pos,
            camera.view_rotation(),
            camera.fov_y,
            &self.light_scratch,
            timer.and_then(|t| t.compute_pass_writes(Zone::LightCull)),
        );
        self.volfog.record(
            queue,
            encoder,
            &camera.pos,
            camera.view_rotation(),
            camera.fov_y,
            FogAtmosphere {
                sigma: params.fog_density,
                height_k: params.fog_height_k,
                colour: params.fog_color,
            },
            &self.lights,
            timer.and_then(|t| t.compute_pass_writes(Zone::Fog)),
        );
        self.far.prepare(queue, &camera.pos, &frustum);
        self.modules.prepare(queue, &camera.pos, &frustum);
        self.horizon.prepare(
            queue,
            encoder,
            &camera.pos,
            view_proj.inverse(),
            size,
            self.world_seed,
            timer.and_then(|t| t.compute_pass_writes(Zone::HorizonCache)),
        );
        self.horizon.encode_half(
            encoder,
            &self.globals_bg,
            timer.and_then(|t| t.pass_writes(Zone::HorizonHalf)),
        );
        let two_phase = self.occlusion
            && self.frozen.is_none()
            && self.prepass
            && self.indirect_first_instance
            && !self.force_direct;
        self.cull.record_phase1(
            queue,
            encoder,
            &CullParams {
                frustum,
                view_proj,
                screen: size,
                count,
                hiz_levels: self.hiz.levels(),
                occlusion: two_phase,
            },
            timer.and_then(|t| t.compute_pass_writes(Zone::Cull)),
        );

        self.lines_scratch.clear();
        for b in boxes {
            push_box_lines(
                &mut self.lines_scratch,
                camera_relative(&b.min, &camera.pos),
                b.size_m,
                b.colour,
            );
        }
        self.lines_scratch.truncate(MAX_LINE_VERTICES);
        if !self.lines_scratch.is_empty() {
            queue.write_buffer(
                &self.lines_buf,
                0,
                bytemuck::cast_slice(&self.lines_scratch),
            );
        }

        if self.prepass {
            self.prepass_pass(encoder, DrawSet::A, true, timer);
            self.hiz.record(
                encoder,
                timer.and_then(|t| t.compute_pass_writes(Zone::HiZ)),
            );
            if two_phase {
                self.cull.record_phase2(
                    encoder,
                    count,
                    timer.and_then(|t| t.compute_pass_writes(Zone::Occlusion)),
                );
                self.prepass_pass(encoder, DrawSet::B, false, None);
            }
        }
        self.cull.resolve(encoder);
        let ao_on = self.prepass && self.ao.mode != AoMode::Off;
        if ao_on {
            self.ao.record(
                queue,
                encoder,
                &self.depth_view,
                &camera.pos,
                camera.view_rotation(),
                camera.fov_y,
                timer.and_then(|t| t.compute_pass_writes(Zone::Ao)),
            );
        }
        // Without the prepass the colour pass clears and writes depth itself; with it the depth
        // is already final and the colour pass only tests against it.
        let prepassed = self.prepass;
        // The stylization pass reads colour and depth, so the scene goes to its own texture and
        // depth is kept.
        let styled = self.post.settings.any();
        let scene_target = if styled {
            self.post.colour_view()
        } else {
            target
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene_target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(params.fog_color[0] * (1.0 - params.fade)),
                        g: f64::from(params.fog_color[1] * (1.0 - params.fade)),
                        b: f64::from(params.fog_color[2] * (1.0 - params.fade)),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: if prepassed {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(DEPTH_CLEAR)
                    },
                    store: if styled {
                        wgpu::StoreOp::Store
                    } else {
                        wgpu::StoreOp::Discard
                    },
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timer.and_then(|t| t.pass_writes(Zone::Scene)),
            ..Default::default()
        });
        self.horizon.draw(&mut pass, &self.globals_bg);
        let scene = if prepassed {
            &self.pipelines.scene_tested
        } else {
            &self.pipelines.scene
        };
        self.draw_chunks(&mut pass, scene, DrawSet::A);
        if two_phase {
            self.draw_chunks(&mut pass, scene, DrawSet::B);
        }
        self.draw_modules(&mut pass);
        self.draw_far(&mut pass);
        if !self.lines_scratch.is_empty() {
            pass.set_pipeline(&self.pipelines.lines);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_vertex_buffer(0, self.lines_buf.slice(..));
            pass.draw(0..self.lines_scratch.len() as u32, 0..1);
        }
        if let Some(level) = self.debug_hiz {
            self.hiz.draw_debug(queue, &mut pass, level);
        }
        drop(pass);
        if styled {
            self.post.record(
                queue,
                encoder,
                &self.depth_view,
                target,
                camera.fov_y,
                aspect,
                timer.and_then(|t| t.pass_writes(Zone::Post)),
            );
        }
    }
}

/// Which culling result a draw consumes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawSet {
    /// Last frame's visible set, drawn before the Hi-Z exists.
    A,
    /// Newly visible after the Hi-Z test.
    B,
}

/// How a pipeline treats the depth buffer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthUse {
    Write,
    Test,
    /// Depth only: no fragment stage and no colour target.
    PrepassOnly,
    /// Depth only, with a fragment stage that discards the faded-out pixels.
    PrepassFade,
}

#[allow(clippy::too_many_arguments)]
fn build_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    topology: wgpu::PrimitiveTopology,
    depth: DepthUse,
    vertex_entry: &str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let colour_targets = [Some(format.into())];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        fragment: (depth != DepthUse::PrepassOnly).then_some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(if depth == DepthUse::PrepassFade {
                "fs_prepass"
            } else {
                "fs_main"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: if depth == DepthUse::PrepassFade {
                &[]
            } else {
                &colour_targets
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: (topology == wgpu::PrimitiveTopology::TriangleList)
                .then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth != DepthUse::Test),
            depth_compare: Some(DEPTH_COMPARE),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn push_box_lines(out: &mut Vec<LineVertex>, min: Vec3, size: f32, colour: [f32; 4]) {
    let c = |i: u32| {
        let p =
            min + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) * size;
        LineVertex {
            rel: p.to_array(),
            colour,
        }
    };
    for a in 0..8u32 {
        for bit in [1u32, 2, 4] {
            let b = a | bit;
            if b != a {
                out.push(c(a));
                out.push(c(b));
            }
        }
    }
}

/// Clip-space depth of a view-space distance `z` in front of the camera, for tests.
pub fn depth_at(proj: Mat4, z: f32) -> f32 {
    let c = proj * Vec4::new(0.0, 0.0, -z, 1.0);
    c.z / c.w
}

struct Pipelines {
    /// Colour pass that also writes depth, for frames without a prepass.
    scene: wgpu::RenderPipeline,
    /// Colour pass that only tests against a finished prepass depth.
    scene_tested: wgpu::RenderPipeline,
    prepass: wgpu::RenderPipeline,
    /// Prepass with the dither test, used while any chunk is fading.
    prepass_fade: wgpu::RenderPipeline,
    /// Far-band instances scaled out of the shared unit meshes; writes depth itself.
    far: wgpu::RenderPipeline,
    /// L4 kit pieces instanced from the shared kit mesh.
    modules: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
}

fn build_pipelines(
    device: &wgpu::Device,
    sources: &ShaderSources,
    scene_layout: &wgpu::PipelineLayout,
    lines_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> Pipelines {
    let scene = |label, depth, entry| {
        build_pipeline(
            device,
            label,
            &sources.scene,
            scene_layout,
            format,
            &[Some(Vertex::LAYOUT)],
            wgpu::PrimitiveTopology::TriangleList,
            depth,
            entry,
        )
    };
    let line_vertex = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<LineVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
    };
    let lines = build_pipeline(
        device,
        "lines",
        &sources.lines,
        lines_layout,
        format,
        &[Some(line_vertex)],
        wgpu::PrimitiveTopology::LineList,
        DepthUse::Test,
        "vs_main",
    );
    Pipelines {
        scene: scene("scene", DepthUse::Write, "vs_main"),
        scene_tested: scene("scene tested", DepthUse::Test, "vs_main"),
        prepass: scene("depth prepass", DepthUse::PrepassOnly, "vs_main"),
        prepass_fade: scene("depth prepass fade", DepthUse::PrepassFade, "vs_main"),
        far: scene("far band", DepthUse::Write, "vs_far"),
        modules: scene("modules", DepthUse::Write, "vs_module"),
        lines,
    }
}
