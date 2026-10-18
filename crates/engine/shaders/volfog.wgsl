// Fragment side of the froxel fog: group 3 holds the integrated grid. `common.wgsl` and
// `shade.wgsl` come before this file.

struct VolFogParams {
    view_rot: mat4x4<f32>,
    prev_view_rot: mat4x4<f32>,
    grid: vec4<u32>,
    screen: vec4<f32>,
    lens: vec4<f32>,
    colour: vec4<f32>,
    medium: vec4<f32>,
    delta: vec4<f32>,
};

@group(3) @binding(0) var<uniform> vf: VolFogParams;
@group(3) @binding(1) var fog_grid: texture_3d<f32>;
@group(3) @binding(2) var fog_sampler: sampler;

// Optical depth of the height fog along the ray to `rel` (the same formula as `apply_fog`).
fn height_tau(rel: vec3<f32>) -> f32 {
    let dist = length(rel);
    let kh = clamp(vf.colour.w * rel.y, -12.0, 12.0);
    var path = dist;
    if (abs(kh) > 1e-4) {
        path = dist * (1.0 - exp(-kh)) / kh;
    }
    return vf.medium.x * path;
}

// Fog for a surface at camera-relative `rel`, view depth `z`, pixel `frag`: the froxel grid out to
// its far plane, the analytic height fog beyond it.
fn apply_fog_vol(colour_in: vec3<f32>, rel: vec3<f32>, z: f32, frag: vec2<f32>) -> vec3<f32> {
    if (vf.medium.w < 0.5) {
        return apply_fog(colour_in, rel);
    }
    let near = vf.screen.z;
    let far = vf.screen.w;
    let zc = clamp(z, near, far);
    let uv = frag / vf.screen.xy;
    let zf = log(zc / near) / log(far / near);
    let s = textureSampleLevel(fog_grid, fog_sampler, vec3<f32>(uv, zf), 0.0);
    var colour = colour_in * s.a + s.rgb;
    if (z > far) {
        let tail = max(height_tau(rel) - height_tau(rel * (far / z)), 0.0);
        colour = mix(colour, vf.colour.rgb, 1.0 - exp(-tail));
    }
    return colour * (1.0 - globals.params.z);
}
