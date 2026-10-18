// Frustum culling: copies each instance's indirect draw arguments and sets instance_count to
// 0 or 1. Standalone (no `common.wgsl`); `Instance` mirrors `scene.wgsl`.

struct Instance {
    origin: vec4<f32>,
    bounds: vec4<f32>,
    mesh: vec4<u32>,
};

struct Params {
    // Normalised inward-facing planes in camera-relative space: dot(n, p) + d >= 0 is inside.
    planes: array<vec4<f32>, 6>,
    count: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
// Five u32 per draw: index_count, instance_count, first_index, base_vertex, first_instance.
@group(0) @binding(2) var<storage, read> src_args: array<u32>;
@group(0) @binding(3) var<storage, read_write> out_args: array<u32>;
@group(0) @binding(4) var<storage, read_write> visible_count: atomic<u32>;

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.count) {
        return;
    }
    let inst = instances[i];
    let centre = inst.origin.xyz + inst.bounds.xyz;
    var visible = true;
    for (var p = 0u; p < 6u; p = p + 1u) {
        let plane = params.planes[p];
        if (dot(plane.xyz, centre) + plane.w < -inst.bounds.w) {
            visible = false;
        }
    }
    let base = i * 5u;
    for (var k = 0u; k < 5u; k = k + 1u) {
        out_args[base + k] = src_args[base + k];
    }
    if (visible) {
        atomicAdd(&visible_count, 1u);
    } else {
        out_args[base + 1u] = 0u;
    }
}
