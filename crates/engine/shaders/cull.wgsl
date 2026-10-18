// Two-phase culling. Standalone (no `common.wgsl`); `Instance` mirrors `scene.wgsl`.
//
// Phase 1 (`cs_phase1`): frustum test; in two-phase mode only instances that were visible last
// frame go into set A, which is drawn first so a depth buffer exists.
// Phase 2 (`cs_phase2`): after the Hi-Z pyramid is built from set A, every frustum-visible
// instance is tested against it. Instances that pass and were not in A form set B, drawn on top.
// `visible` holds each slot's result for the next frame's phase 1.

struct Instance {
    origin: vec4<f32>,
    bounds: vec4<f32>,
    mesh: vec4<u32>,
};

struct Params {
    // Normalised inward-facing planes in camera-relative space: dot(n, p) + d >= 0 is inside.
    planes: array<vec4<f32>, 6>,
    view_proj: mat4x4<f32>,
    screen: vec2<f32>,
    count: u32,
    hiz_levels: u32,
    // 0: frustum only, everything visible goes into set A. 1: two-phase occlusion culling.
    mode: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
// Five u32 per draw: index_count, instance_count, first_index, base_vertex, first_instance.
@group(0) @binding(2) var<storage, read> src_args: array<u32>;
@group(0) @binding(3) var<storage, read_write> args_a: array<u32>;
@group(0) @binding(4) var<storage, read_write> args_b: array<u32>;
@group(0) @binding(5) var<storage, read_write> visible: array<u32>;
// Instances drawn in set A and set B.
@group(0) @binding(6) var<storage, read_write> counters: array<atomic<u32>, 2>;
@group(0) @binding(7) var hiz: texture_2d<f32>;

fn slot_of(inst: Instance) -> u32 {
    return min(bitcast<u32>(inst.origin.w), arrayLength(&visible) - 1u);
}

fn in_frustum(inst: Instance) -> bool {
    let centre = inst.origin.xyz + inst.bounds.xyz;
    for (var p = 0u; p < 6u; p = p + 1u) {
        let plane = params.planes[p];
        if (dot(plane.xyz, centre) + plane.w < -inst.bounds.w) {
            return false;
        }
    }
    return true;
}

// True when the instance's bounding cube is certainly behind what the pyramid already holds.
fn occluded(inst: Instance) -> bool {
    let c = inst.origin.xyz + inst.bounds.xyz;
    let r = inst.bounds.w;
    var lo = vec2<f32>(1.0e9);
    var hi = vec2<f32>(-1.0e9);
    var nearest = 0.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        let sign = vec3<f32>(
            select(-1.0, 1.0, (k & 1u) != 0u),
            select(-1.0, 1.0, (k & 2u) != 0u),
            select(-1.0, 1.0, (k & 4u) != 0u),
        );
        let clip = params.view_proj * vec4<f32>(c + sign * r, 1.0);
        if (clip.w <= 0.0) {
            // Reaches the eye plane: no usable screen rectangle.
            return false;
        }
        let ndc = clip.xyz / clip.w;
        lo = min(lo, ndc.xy);
        hi = max(hi, ndc.xy);
        // Reversed Z: the larger value is the nearer one.
        nearest = max(nearest, ndc.z);
    }
    // NDC y points up, texture rows go down.
    let px_min = clamp(vec2<f32>(lo.x * 0.5 + 0.5, 0.5 - hi.y * 0.5) * params.screen, vec2<f32>(0.0), params.screen - vec2<f32>(1.0));
    let px_max = clamp(vec2<f32>(hi.x * 0.5 + 0.5, 0.5 - lo.y * 0.5) * params.screen, vec2<f32>(0.0), params.screen - vec2<f32>(1.0));
    let extent = max(max(px_max.x - px_min.x, px_max.y - px_min.y), 1.0);
    let level = min(u32(ceil(log2(extent))), params.hiz_levels - 1u);
    let dims = vec2<i32>(textureDimensions(hiz, i32(level)));
    let a = min(vec2<i32>(px_min) >> vec2<u32>(level), dims - vec2<i32>(1));
    let b = min(vec2<i32>(px_max) >> vec2<u32>(level), dims - vec2<i32>(1));
    let lvl = i32(level);
    // The farthest surface anywhere under the rectangle; sky is 0, so it never occludes.
    let farthest = min(
        min(textureLoad(hiz, vec2<i32>(a.x, a.y), lvl).x, textureLoad(hiz, vec2<i32>(b.x, a.y), lvl).x),
        min(textureLoad(hiz, vec2<i32>(a.x, b.y), lvl).x, textureLoad(hiz, vec2<i32>(b.x, b.y), lvl).x),
    );
    return farthest > nearest;
}

fn copy_args(i: u32) {
    let base = i * 5u;
    for (var k = 0u; k < 5u; k = k + 1u) {
        args_a[base + k] = src_args[base + k];
        args_b[base + k] = src_args[base + k];
    }
    args_b[base + 1u] = 0u;
}

@compute @workgroup_size(64)
fn cs_phase1(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.count) {
        return;
    }
    let inst = instances[i];
    var in_a = in_frustum(inst);
    if (params.mode == 1u) {
        in_a = in_a && visible[slot_of(inst)] == 1u;
    }
    copy_args(i);
    if (!in_a) {
        args_a[i * 5u + 1u] = 0u;
    } else {
        atomicAdd(&counters[0], 1u);
    }
}

@compute @workgroup_size(64)
fn cs_phase2(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.count) {
        return;
    }
    let inst = instances[i];
    let slot = slot_of(inst);
    let frustum = in_frustum(inst);
    let was_drawn = frustum && visible[slot] == 1u;
    var now = frustum;
    if (frustum) {
        now = !occluded(inst);
    }
    visible[slot] = select(0u, 1u, now);
    if (now && !was_drawn) {
        args_b[i * 5u + 1u] = 1u;
        atomicAdd(&counters[1], 1u);
    }
}
