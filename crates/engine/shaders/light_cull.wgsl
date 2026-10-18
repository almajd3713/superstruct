// Clustered forward+: assigns point lights to a 3D grid of view-space clusters (tiles of 64 px,
// slices exponential in depth). Standalone: no `common.wgsl`. The fragment side is `lights.wgsl`.

struct Params {
    // Rotation from camera-relative world space into view space.
    view_rot: mat4x4<f32>,
    // Tiles in x, tiles in y, depth slices, lights allowed per cluster.
    grid: vec4<u32>,
    // Screen width and height in pixels, tile size in pixels, 1 to skip the cluster lists.
    screen: vec4<f32>,
    // tan(fov_y / 2), aspect, near, far of the cluster grid (metres).
    lens: vec4<f32>,
    // Light count, brute-force flag.
    counts: vec4<u32>,
};

struct Light {
    // Camera-relative world position and radius.
    pos_radius: vec4<f32>,
    // Linear colour times intensity in rgb, the spot's inner cosine in w.
    colour: vec4<f32>,
    // Spot direction and outer cosine; below -1.5 means a plain point light.
    dir_cone: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> lights: array<Light>;
@group(0) @binding(2) var<storage, read_write> view_lights: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> counts: array<u32>;
@group(0) @binding(4) var<storage, read_write> list: array<u32>;

@compute @workgroup_size(64)
fn cs_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= p.counts.x) {
        return;
    }
    let l = lights[i];
    view_lights[i] = vec4<f32>((p.view_rot * vec4<f32>(l.pos_radius.xyz, 0.0)).xyz, l.pos_radius.w);
}

fn slice_depth(s: f32) -> f32 {
    return p.lens.z * pow(p.lens.w / p.lens.z, s / f32(p.grid.z));
}

@compute @workgroup_size(64)
fn cs_cull(@builtin(global_invocation_id) id: vec3<u32>) {
    let total = p.grid.x * p.grid.y * p.grid.z;
    let c = id.x;
    if (c >= total) {
        return;
    }
    let tx = c % p.grid.x;
    let ty = (c / p.grid.x) % p.grid.y;
    let s = c / (p.grid.x * p.grid.y);
    // The cluster's box in view space: four screen corners at the near and far slice depths.
    let px0 = f32(tx) * p.screen.z;
    let px1 = f32(tx + 1u) * p.screen.z;
    let py0 = f32(ty) * p.screen.z;
    let py1 = f32(ty + 1u) * p.screen.z;
    let xn0 = px0 / p.screen.x * 2.0 - 1.0;
    let xn1 = px1 / p.screen.x * 2.0 - 1.0;
    let yn0 = 1.0 - py0 / p.screen.y * 2.0;
    let yn1 = 1.0 - py1 / p.screen.y * 2.0;
    let d0 = slice_depth(f32(s));
    let d1 = slice_depth(f32(s + 1u));
    let kx = p.lens.x * p.lens.y;
    let ky = p.lens.x;
    var lo = vec3<f32>(1.0e30);
    var hi = vec3<f32>(-1.0e30);
    for (var k = 0u; k < 8u; k = k + 1u) {
        let d = select(d0, d1, (k & 1u) != 0u);
        let xn = select(xn0, xn1, (k & 2u) != 0u);
        let yn = select(yn0, yn1, (k & 4u) != 0u);
        let v = vec3<f32>(xn * kx * d, yn * ky * d, -d);
        lo = min(lo, v);
        hi = max(hi, v);
    }
    var n = 0u;
    let cap = p.grid.w;
    let base = c * cap;
    for (var i = 0u; i < p.counts.x; i = i + 1u) {
        let l = view_lights[i];
        let nearest = clamp(l.xyz, lo, hi);
        let d = l.xyz - nearest;
        if (dot(d, d) <= l.w * l.w && n < cap) {
            list[base + n] = i;
            n = n + 1u;
        }
    }
    counts[c] = n;
}
