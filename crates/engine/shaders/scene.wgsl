// Greybox scene shader: Lambert plus a world-aligned 1 m / 8 m grid.
// `common.wgsl` is prepended at load time.

struct Instance {
    origin: vec4<f32>,
    // Chunk-local bounding sphere, used by the culling pass.
    bounds: vec4<f32>,
    // First index, index count, base vertex, LOD band.
    mesh: vec4<u32>,
};

// 64x64 blue-noise tile, ranks scaled to 0..1.
@group(0) @binding(1) var blue_noise: texture_2d<f32>;

@group(1) @binding(0) var<storage, read> instances: array<Instance>;

// Far band: a primitive's bounding box in cell-local metres plus the owning cell's slot.
struct FarInstance {
    min: vec3<f32>,
    cell: u32,
    size: vec3<f32>,
    // Material in the low 16 bits, bit 16 for a cylinder, bits 17-18 its axis (0 x, 1 y, 2 z).
    attrs: u32,
};

@group(1) @binding(1) var<storage, read> far_instances: array<FarInstance>;
// Camera-relative origin of each far cell, rewritten every frame.
@group(1) @binding(2) var<storage, read> far_cells: array<vec4<f32>>;

// L4 modules: a position in the owning chunk's frame and the quarter turn about +Y in bits 0-1;
// the chunk's cell slot is in bits 2-17.
struct ModInstance {
    pos: vec3<f32>,
    packed: u32,
};

@group(1) @binding(3) var<storage, read> mod_instances: array<ModInstance>;
// Two vec4 per cell: camera-relative origin and fade, then emissive colour and strength.
@group(1) @binding(4) var<storage, read> mod_cells: array<vec4<f32>>;

struct VsIn {
    @builtin(instance_index) draw: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal_oct: vec2<f32>,
    @location(2) material: vec2<u32>,
};

struct VsOut {
    // Invariant so the prepass and colour pass produce bit-identical depth.
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) material: u32,
    // Band cross-fade: magnitude is the fade level, the sign says whether the chunk is
    // fading in (positive) or out (negative). The two rules are exact complements.
    @location(3) @interpolate(flat) fade: f32,
    // Colour of emissive parts; white for everything that is not an L4 module.
    @location(4) @interpolate(flat) glow: vec3<f32>,
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
    let rel = instances[in.draw].origin.xyz + in.position;
    out.rel = rel;
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = oct_decode(in.normal_oct);
    out.material = in.material.x;
    out.fade = bitcast<f32>(instances[in.draw].mesh.w);
    out.glow = vec3<f32>(1.0);
    return out;
}

struct FarIn {
    @builtin(instance_index) draw: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal_oct: vec2<f32>,
};

// Scales the shared unit box or unit cylinder (axis along y inside the unit box) into place.
@vertex
fn vs_far(in: FarIn) -> VsOut {
    var out: VsOut;
    let inst = far_instances[in.draw];
    var p = in.position;
    var n = oct_decode(in.normal_oct);
    let axis = (inst.attrs >> 17u) & 3u;
    if ((inst.attrs & 0x10000u) != 0u) {
        // Cyclic permutations are rotations, so the winding stays outward; a plain swap would
        // mirror the mesh and the back-face cull would remove the wrong side.
        if (axis == 0u) {
            p = vec3<f32>(p.y, p.z, p.x);
            n = vec3<f32>(n.y, n.z, n.x);
        } else if (axis == 2u) {
            p = vec3<f32>(p.z, p.x, p.y);
            n = vec3<f32>(n.z, n.x, n.y);
        }
    }
    let rel = far_cells[inst.cell].xyz + inst.min + p * inst.size;
    out.rel = rel;
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = n;
    out.material = inst.attrs & 0xffffu;
    out.fade = far_cells[inst.cell].w;
    out.glow = vec3<f32>(1.0);
    return out;
}

// Kit pieces: the shared mesh turned by the instance's quarter turn and moved into place.
@vertex
fn vs_module(in: VsIn) -> VsOut {
    var out: VsOut;
    let inst = mod_instances[in.draw];
    let turns = inst.packed & 3u;
    var p = in.position;
    var n = oct_decode(in.normal_oct);
    // A rotation about +Y by turns * 90 degrees.
    if (turns == 1u) {
        p = vec3<f32>(p.z, p.y, -p.x);
        n = vec3<f32>(n.z, n.y, -n.x);
    } else if (turns == 2u) {
        p = vec3<f32>(-p.x, p.y, -p.z);
        n = vec3<f32>(-n.x, n.y, -n.z);
    } else if (turns == 3u) {
        p = vec3<f32>(-p.z, p.y, p.x);
        n = vec3<f32>(-n.z, n.y, n.x);
    }
    let slot = (inst.packed >> 2u) & 0xffffu;
    let cell = mod_cells[slot * 2u];
    let glow = mod_cells[slot * 2u + 1u];
    let level = f32((inst.packed >> 18u) & 15u) / 15.0;
    let rel = cell.xyz + inst.pos + p;
    out.rel = rel;
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = n;
    out.material = in.material.x;
    out.fade = cell.w;
    out.glow = glow.rgb * (glow.a * level);
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

// Blue-noise dither: true when this pixel belongs to the other band at this fade level.
fn fade_discard(fade: f32, frag: vec4<f32>) -> bool {
    let f = abs(fade);
    if (f >= 1.0) {
        return false;
    }
    let n = textureLoad(blue_noise, vec2<u32>(frag.xy) & vec2<u32>(63u), 0).r;
    let keep_in = n < f;
    let keep_out = n >= 1.0 - f;
    return !select(keep_out, keep_in, fade >= 0.0);
}

// Depth prepass while chunks are fading: only the pixels the colour pass will keep.
@fragment
fn fs_prepass(in: VsOut) {
    if (fade_discard(in.fade, in.clip)) {
        discard;
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (fade_discard(in.fade, in.clip)) {
        discard;
    }
    let n = normalize(in.normal);
    var colour = lit_glow(n, in.material, in.glow);
    colour = colour + material_albedo(in.material) * point_light_sum(in.rel, n, in.clip.xy, 1.0 / in.clip.w);

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

    colour = apply_fog_vol(colour, in.rel, 1.0 / in.clip.w, in.clip.xy);
    return vec4<f32>(colour, 1.0);
}
