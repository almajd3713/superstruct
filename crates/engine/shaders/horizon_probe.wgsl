// Test and tool entry point for `horizon.wgsl`: classifies points of L1 cells.

struct Params {
    seed: vec2<u32>,
    count: u32,
    pad: u32,
};

struct Probe {
    cx: vec2<u32>,
    cy: vec2<u32>,
    cz: vec2<u32>,
    p: vec3<f32>,
    pad: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> probes: array<Probe>;
@group(0) @binding(2) var<storage, read_write> classes: array<u32>;

@compute @workgroup_size(64)
fn cs_probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.count) {
        return;
    }
    let pr = probes[i];
    let d = l1_cell_data(params.seed, pr.cx, pr.cy, pr.cz);
    classes[i] = l1_classify(d, pr.p);
}

// Oriented boxes: one box and one point per probe, all vec4 (w unused), std430.
struct OBoxProbe {
    origin: vec4<f32>,
    rot: vec4<f32>,
    centre: vec4<f32>,
    half_size: vec4<f32>,
    clip_min: vec4<f32>,
    clip_max: vec4<f32>,
    p: vec4<f32>,
};

@group(0) @binding(3) var<storage, read> oboxes: array<OBoxProbe>;
@group(0) @binding(4) var<storage, read_write> odist: array<f32>;

@compute @workgroup_size(64)
fn cs_oriented(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.count) {
        return;
    }
    let b = oboxes[i];
    odist[i] = sdf_oriented_box(
        b.p.xyz, b.origin.xyz, b.rot, b.centre.xyz, b.half_size.xyz, b.clip_min.xyz, b.clip_max.xyz,
    );
}
