// Greybox scene shader: Lambert plus a world-aligned 1 m / 8 m grid.
// `common.wgsl` is prepended at load time.

@group(1) @binding(0) var<storage, read> draw_origins: array<vec4<f32>>;

struct VsIn {
    @builtin(instance_index) draw: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal_oct: vec2<f32>,
    @location(2) material: vec2<u32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) material: u32,
};

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let xy = (vec2<f32>(1.0) - abs(vec2<f32>(n.y, n.x))) * select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>(xy, n.z);
    }
    return normalize(n);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    // Camera-relative: the chunk origin was subtracted on the CPU in f64.
    let rel = draw_origins[in.draw].xyz + in.position;
    out.rel = rel;
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = oct_decode(in.normal_oct);
    out.material = in.material.x;
    return out;
}

// Distance in metres to the nearest line of a grid with `spacing`, over pixel footprint.
fn grid_line(x: f32, spacing: f32, half_px: f32) -> f32 {
    let w = max(fwidth(x), 1e-6);
    let d = abs(fract(x / spacing + 0.5) - 0.5) * spacing;
    let line = 1.0 - clamp((d - half_px * w) / w + 0.5, 0.0, 1.0);
    // Fade lines out when they get denser than ~3 px so distant surfaces do not shimmer.
    let fade = 1.0 - smoothstep(0.15, 0.4, w / spacing);
    return line * fade;
}

// Ids follow `worldgen::prim::MaterialId`: concrete, metal, grating, emissive, pipe, warm.
fn material_albedo(id: u32) -> vec3<f32> {
    switch id {
        case 0u: { return vec3<f32>(0.36, 0.37, 0.39); }
        case 1u: { return vec3<f32>(0.2, 0.21, 0.24); }
        case 2u: { return vec3<f32>(0.14, 0.15, 0.17); }
        case 3u: { return vec3<f32>(0.9, 0.88, 0.8); }
        case 4u: { return vec3<f32>(0.27, 0.29, 0.29); }
        case 5u: { return vec3<f32>(0.62, 0.46, 0.3); }
        default: { return vec3<f32>(0.4, 0.4, 0.4); }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let sun = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let lambert = max(dot(n, sun), 0.0);
    let ambient = 0.22 + 0.1 * n.y;
    var colour = material_albedo(in.material) * (ambient + 0.75 * lambert);

    if (globals.params.y > 0.5) {
        let wp = globals.cam_local.xyz + in.rel;
        // Lines on the two axes spanning the surface.
        let a = abs(n);
        var uv: vec2<f32>;
        if (a.y >= a.x && a.y >= a.z) { uv = wp.xz; } else if (a.x >= a.z) { uv = wp.zy; } else { uv = wp.xy; }
        let g1 = max(grid_line(uv.x, 1.0, 0.5), grid_line(uv.y, 1.0, 0.5));
        let g8 = max(grid_line(uv.x, 8.0, 1.0), grid_line(uv.y, 8.0, 1.0));
        colour = mix(colour, colour * 0.55, g1 * 0.5);
        colour = mix(colour, vec3<f32>(0.05, 0.06, 0.07), g8 * 0.8);
    }

    let dist = length(in.rel);
    let fog = 1.0 - exp(-globals.params.x * dist);
    colour = mix(colour, vec3<f32>(0.02, 0.025, 0.03), fog);
    colour = colour * (1.0 - globals.params.z);
    return vec4<f32>(colour, 1.0);
}
