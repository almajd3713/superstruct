#![allow(clippy::unwrap_used, clippy::cast_possible_truncation)]
//! The WGSL port of the L1 rules against the real generator: classify points of L1 cells on the
//! GPU and compare with `Shape::contains` over the trunk and deck primitives `l1_cell` emits.
//! Skips without a GPU adapter.

use bytemuck::{Pod, Zeroable};
use engine::headless::Headless;
use glam::{I64Vec3, Vec3};
use wgpu::util::DeviceExt;
use worldgen::cavity::CavityView;
use worldgen::cell::{CellKey, Level};
use worldgen::l1::l1_cell;
use worldgen::prim::{FeatureKind, Primitive};
use worldgen::seed::WorldSeed;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    seed: [u32; 2],
    count: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Probe {
    cell: [[u32; 2]; 3],
    pad0: [u32; 2],
    p: [f32; 3],
    pad1: f32,
}

fn split(v: i64) -> [u32; 2] {
    let u = v.cast_unsigned();
    [u as u32, (u >> 32) as u32]
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16_777_216.0
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo).cast_unsigned()).cast_signed()
    }
}

struct Sample {
    key: CellKey,
    p: Vec3,
    solid: bool,
}

fn near_cavity(seed: WorldSeed, key: CellKey) -> bool {
    let view = CavityView::new(seed, key);
    [
        (0, 0, 0),
        (1, 0, 0),
        (-1, 0, 0),
        (0, 1, 0),
        (0, -1, 0),
        (0, 0, 1),
        (0, 0, -1),
    ]
    .iter()
    .any(|&(x, y, z)| view.is_cavity(CellKey::new(Level::L1, key.coords + I64Vec3::new(x, y, z))))
}

fn is_far_structure(p: &Primitive) -> bool {
    p.kind == FeatureKind::TRUNK || p.kind == FeatureKind::DECK
}

/// Cells near the origin and far out, a few points each: uniform ones and ones inside or just
/// beside the generator's own primitives, where boundary mistakes would show.
fn samples(seed: WorldSeed, cells: usize) -> (Vec<Sample>, usize) {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::new();
    let mut skipped = 0;
    while out.len() < cells * 4 {
        let scale: i64 = match rng.next() % 4 {
            0 => 40,
            1 => 5_000,
            2 => 1 << 40,
            _ => 1 << 49,
        };
        let key = CellKey::new(
            Level::L1,
            I64Vec3::new(
                rng.range(-scale, scale),
                rng.range(-scale.min(1 << 20), scale.min(1 << 20)),
                rng.range(-scale, scale),
            ),
        );
        if !key.in_domain() || near_cavity(seed, key) {
            skipped += 1;
            continue;
        }
        let prims: Vec<Primitive> = l1_cell(seed, key)
            .primitives
            .into_iter()
            .filter(is_far_structure)
            .collect();
        let truth = |p: Vec3| prims.iter().any(|q| q.shape.contains(p));
        for i in 0..4 {
            let p = if i == 0 || prims.is_empty() {
                Vec3::new(rng.unit(), rng.unit(), rng.unit()) * 4096.0
            } else {
                let q = &prims[rng.next() as usize % prims.len()];
                let bb = q.shape.aabb();
                let (lo, hi) = (bb.min_m(), bb.max_m());
                let pad = Vec3::splat(2.0);
                let t = Vec3::new(rng.unit(), rng.unit(), rng.unit());
                (lo - pad + (hi - lo + pad * 2.0) * t).clamp(Vec3::ZERO, Vec3::splat(4095.99))
            };
            out.push(Sample {
                key,
                p,
                solid: truth(p),
            });
        }
    }
    (out, skipped)
}

fn classify_on_gpu(h: &Headless, seed: u64, samples: &[Sample]) -> Vec<u32> {
    let probes: Vec<Probe> = samples
        .iter()
        .map(|s| Probe {
            cell: [
                split(s.key.coords.x),
                split(s.key.coords.y),
                split(s.key.coords.z),
            ],
            pad0: [0; 2],
            p: s.p.to_array(),
            pad1: 0.0,
        })
        .collect();
    let params = Params {
        seed: [seed as u32, (seed >> 32) as u32],
        count: probes.len() as u32,
        pad: 0,
    };
    let module = h.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("horizon probe"),
        source: wgpu::ShaderSource::Wgsl(engine::horizon::probe_source().into()),
    });
    let pipeline = h
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("horizon probe"),
            layout: None,
            module: &module,
            entry_point: Some("cs_probe"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
    let init = |label, data: &[u8], usage| {
        h.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage,
            })
    };
    let params_buf = init(
        "params",
        bytemuck::bytes_of(&params),
        wgpu::BufferUsages::UNIFORM,
    );
    let probes_buf = init(
        "probes",
        bytemuck::cast_slice(&probes),
        wgpu::BufferUsages::STORAGE,
    );
    let size = (probes.len() * 4) as u64;
    let out_buf = h.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("classes"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read_buf = h.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("classes readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bg = h.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe bg"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: probes_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out_buf.as_entire_binding(),
            },
        ],
    });
    let mut encoder = h
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups((probes.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, size);
    h.queue.submit([encoder.finish()]);
    read_buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = h.device.poll(wgpu::PollType::wait_indefinitely());
    let view = read_buf.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice(&view).to_vec()
}

#[test]
fn gpu_port_matches_the_generator() {
    let Some(h) = Headless::new() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let seed = WorldSeed(0x5eed);
    let cells = if cfg!(debug_assertions) { 500 } else { 2500 };
    let (samples, skipped) = samples(seed, cells);
    let classes = classify_on_gpu(&h, seed.0, &samples);
    let mut mismatches = Vec::new();
    let (mut solid, mut void) = (0, 0);
    for (s, &c) in samples.iter().zip(&classes) {
        if s.solid {
            solid += 1;
        } else {
            void += 1;
        }
        if s.solid != (c != 0) {
            mismatches.push((s.key.coords, s.p, s.solid, c));
        }
    }
    eprintln!(
        "{} points ({solid} solid, {void} void), {skipped} cells skipped near cavities, {} mismatches",
        samples.len(),
        mismatches.len()
    );
    for m in mismatches.iter().take(10) {
        eprintln!(
            "mismatch: cell {:?} p {:?} cpu solid {} gpu class {}",
            m.0, m.1, m.2, m.3
        );
    }
    assert!(
        solid > samples.len() / 10,
        "test points should hit structure"
    );
    assert!(mismatches.is_empty());
}
