// Stylization prototypes (P4.3.3): ink outlines from depth and normals reconstructed from it,
// ordered dither, posterize and film grain, each behind a toggle. Standalone: no `common.wgsl`.

struct Params {
    // Bit 0 posterize, 1 dither, 2 outlines, 3 grain.
    flags: u32,
    frame: u32,
    levels: f32,
    near: f32,
    // x: tan(fov_y / 2), y: aspect, z: ink strength, w: grain amount.
    lens: vec4<f32>,
};

@group(0) @binding(0) var scene_colour: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: Params;

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the screen.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn has(bit: u32) -> bool {
    return (params.flags & (1u << bit)) != 0u;
}

fn load_depth(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(scene_depth));
    return textureLoad(scene_depth, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// View-space position of a pixel from its reversed infinite depth (d = near / z).
fn view_pos(p: vec2<i32>, d: f32) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(scene_depth));
    let ndc = (vec2<f32>(p) + 0.5) / size * 2.0 - 1.0;
    let z = params.near / max(d, 1.0e-7);
    return vec3<f32>(ndc.x * params.lens.x * params.lens.y * z, -ndc.y * params.lens.x * z, -z);
}

// 0 to 1: how much ink a pixel gets. Silhouettes show as a break in the linear inverse depth,
// creases as a change of the normal between the two halves of the neighbourhood.
fn ink(p: vec2<i32>) -> f32 {
    let dc = load_depth(p);
    let dl = load_depth(p + vec2<i32>(-1, 0));
    let dr = load_depth(p + vec2<i32>(1, 0));
    let du = load_depth(p + vec2<i32>(0, -1));
    let dd = load_depth(p + vec2<i32>(0, 1));
    // Inverse depth is linear over a plane in screen space, so its second difference is zero
    // inside a surface and large across an edge.
    let curve = max(abs(dl + dr - 2.0 * dc), abs(du + dd - 2.0 * dc));
    let silhouette = smoothstep(0.08, 0.25, curve / max(dc, 1.0e-6));
    let pc = view_pos(p, dc);
    let pl = view_pos(p + vec2<i32>(-1, 0), dl);
    let pr = view_pos(p + vec2<i32>(1, 0), dr);
    let pu = view_pos(p + vec2<i32>(0, -1), du);
    let pd = view_pos(p + vec2<i32>(0, 1), dd);
    let na = normalize(cross(pr - pc, pd - pc));
    let nb = normalize(cross(pl - pc, pu - pc));
    // Only trust the normals where the neighbours are on one surface.
    let flat_enough = 1.0 - silhouette;
    let crease = smoothstep(0.12, 0.35, 1.0 - dot(na, nb)) * flat_enough;
    return max(silhouette, crease) * select(1.0, 0.0, dc <= 0.0);
}

// 4x4 Bayer thresholds in [0, 1).
fn bayer(p: vec2<u32>) -> f32 {
    let m = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    return (f32(m[(p.y & 3u) * 4u + (p.x & 3u)]) + 0.5) / 16.0;
}

fn hash21(p: vec2<u32>, frame: u32) -> f32 {
    var h = p.x * 374761393u + p.y * 668265263u + frame * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

fn to_display(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

@fragment
fn fs_post(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(frag.xy);
    var c = to_display(textureLoad(scene_colour, p, 0).rgb);
    if (has(2u)) {
        let a = ink(p) * params.lens.z;
        c = mix(c, vec3<f32>(0.02, 0.025, 0.035), a);
    }
    if (has(0u) || has(1u)) {
        // Dithering alone still needs a coarse ladder to show.
        let n = select(8.0, params.levels, has(0u));
        let t = select(0.5, bayer(vec2<u32>(frag.xy)), has(1u));
        c = floor(c * (n - 1.0) + t) / (n - 1.0);
    }
    if (has(3u)) {
        let lum = dot(c, vec3<f32>(0.299, 0.587, 0.114));
        let amount = params.lens.w * (0.35 + 0.65 * (1.0 - abs(2.0 * lum - 1.0)));
        c = c + (hash21(vec2<u32>(frag.xy), params.frame) - 0.5) * amount;
    }
    return vec4<f32>(to_linear(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
