// Hi-Z depth pyramid. Depth is reversed (1 = near), so the farthest surface of a 2x2 block is
// the minimum, which is what an occlusion test needs to stay conservative.
// Standalone: no `common.wgsl`.

@group(0) @binding(0) var depth_in: texture_depth_2d;
@group(0) @binding(1) var level0_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn cs_init(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(level0_out);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    textureStore(level0_out, id.xy, vec4<f32>(textureLoad(depth_in, id.xy, 0), 0.0, 0.0, 0.0));
}

@group(0) @binding(0) var prev_level: texture_2d<f32>;
@group(0) @binding(1) var next_level: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn cs_reduce(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(next_level);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    // Mip sizes round down, so with an odd source the last destination texel also covers the
    // extra source row or column; it must be included or the pyramid stops being conservative.
    let src = vec2<i32>(textureDimensions(prev_level));
    let extra_x = select(0, 1, src.x > i32(dims.x) * 2 && id.x == dims.x - 1u);
    let extra_y = select(0, 1, src.y > i32(dims.y) * 2 && id.y == dims.y - 1u);
    let span = vec2<i32>(2 + extra_x, 2 + extra_y);
    let base = vec2<i32>(id.xy) * 2;
    var m = 1.0;
    for (var dy = 0; dy < span.y; dy = dy + 1) {
        for (var dx = 0; dx < span.x; dx = dx + 1) {
            m = min(m, textureLoad(prev_level, min(base + vec2<i32>(dx, dy), src - vec2<i32>(1)), 0).x);
        }
    }
    textureStore(next_level, id.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}

// Debug view: draws one pyramid level stretched over the screen.
struct DebugParams {
    level: u32,
};
@group(0) @binding(0) var pyramid: texture_2d<f32>;
@group(0) @binding(1) var<uniform> debug_params: DebugParams;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_debug(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let level = i32(debug_params.level);
    let max_xy = vec2<i32>(textureDimensions(pyramid, level)) - vec2<i32>(1);
    let texel = min(vec2<i32>(frag.xy) >> vec2<u32>(debug_params.level), max_xy);
    let v = textureLoad(pyramid, texel, level).x;
    // Reversed depth is tiny far away; a square root keeps the distance readable.
    let g = clamp(sqrt(v * 8.0), 0.0, 1.0);
    return vec4<f32>(g, g, g, 1.0);
}
