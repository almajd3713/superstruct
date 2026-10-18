// Ambient occlusion, two candidates (P5.1.4), computed from the depth prepass before the colour
// pass. Mode 1 is screen-space (hemisphere samples against the depth buffer); mode 2 is analytic:
// the signed distance to the nearest of up to 256 boxes, sampled along the surface normal.
// Standalone: no `common.wgsl`.

struct AoParams {
    // Rotation from camera-relative world space into view space.
    view_rot: mat4x4<f32>,
    // Output width and height (the AO texture), depth buffer width and height.
    sizes: vec4<f32>,
    // tan(fov_y / 2), aspect, near, mode (1 SSAO, 2 analytic).
    lens: vec4<f32>,
    // radius (metres), strength, box count, frame.
    tune: vec4<f32>,
};

struct AoBox {
    // Camera-relative centre and half size.
    centre: vec4<f32>,
    half: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ap: AoParams;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var<storage, read> boxes: array<AoBox>;
@group(0) @binding(3) var ao_out: texture_storage_2d<rgba8unorm, write>;

fn view_pos(px: vec2<f32>, d: f32) -> vec3<f32> {
    // px in depth-buffer pixels; reversed infinite depth d = near / z.
    let ndc = vec2<f32>(px.x / ap.sizes.z * 2.0 - 1.0, 1.0 - px.y / ap.sizes.w * 2.0);
    let z = ap.lens.z / max(d, 1.0e-7);
    return vec3<f32>(ndc.x * ap.lens.x * ap.lens.y * z, ndc.y * ap.lens.x * z, -z);
}

fn load_depth(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(i32(ap.sizes.z), i32(ap.sizes.w));
    return textureLoad(depth_tex, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// Normal from the nearer of each pair of depth neighbours, so edges do not bend it.
fn view_normal(p: vec2<i32>, pc: vec3<f32>, dc: f32) -> vec3<f32> {
    let dl = load_depth(p + vec2<i32>(-1, 0));
    let dr = load_depth(p + vec2<i32>(1, 0));
    let du = load_depth(p + vec2<i32>(0, -1));
    let dd = load_depth(p + vec2<i32>(0, 1));
    let fp = vec2<f32>(p) + 0.5;
    var dx: vec3<f32>;
    if (abs(dr - dc) < abs(dl - dc)) {
        dx = view_pos(fp + vec2<f32>(1.0, 0.0), dr) - pc;
    } else {
        dx = pc - view_pos(fp + vec2<f32>(-1.0, 0.0), dl);
    }
    var dy: vec3<f32>;
    if (abs(dd - dc) < abs(du - dc)) {
        dy = view_pos(fp + vec2<f32>(0.0, 1.0), dd) - pc;
    } else {
        dy = pc - view_pos(fp + vec2<f32>(0.0, -1.0), du);
    }
    // Screen y grows downwards, so the winding is dy x dx.
    return normalize(cross(dy, dx));
}

fn hash12(p: vec2<u32>, frame: u32) -> f32 {
    var h = p.x * 374761393u + p.y * 668265263u + frame * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

fn project(v: vec3<f32>) -> vec2<f32> {
    let ndc = vec2<f32>(v.x / (-v.z * ap.lens.x * ap.lens.y), v.y / (-v.z * ap.lens.x));
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * ap.sizes.zw;
}

// Cosine-ish hemisphere sample `i` of `count` around `n`, turned by a per-pixel angle.
fn hemisphere(i: u32, count: u32, n: vec3<f32>, angle: f32) -> vec3<f32> {
    let t = (f32(i) + 0.5) / f32(count);
    // Golden-angle spiral over the hemisphere, rotated per pixel.
    let phi = f32(i) * 2.39996323 + angle * 6.2831853;
    let cos_t = sqrt(1.0 - t);
    let sin_t = sqrt(t);
    let local = vec3<f32>(cos(phi) * sin_t, sin(phi) * sin_t, cos_t);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let tx = normalize(cross(up, n));
    let ty = cross(n, tx);
    return tx * local.x + ty * local.y + n * local.z;
}

fn ssao(id: vec2<u32>, pc: vec3<f32>, n: vec3<f32>) -> f32 {
    let radius = ap.tune.x;
    let angle = hash12(id, u32(ap.tune.w));
    let count = 12u;
    var occ = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let s = (f32(i) + 1.0) / f32(count);
        let dir = hemisphere(i, count, n, angle);
        let sp = pc + dir * (radius * mix(0.1, 1.0, s * s));
        if (sp.z > -ap.lens.z) {
            continue;
        }
        let px = project(sp);
        let ds = load_depth(vec2<i32>(px));
        // Reversed depth: a larger value is nearer. The scene surface at that pixel hides the
        // sample when it is nearer to the camera than the sample is.
        let scene_z = -ap.lens.z / max(ds, 1.0e-7);
        let bias = 0.03 + 0.01 * (-pc.z);
        if (scene_z > sp.z + bias) {
            // Ignore occluders far in front of the shaded point: they are not neighbours.
            let range = smoothstep(0.0, 1.0, radius / max(abs(pc.z - scene_z), 1.0e-3));
            occ = occ + range;
        }
    }
    return clamp(1.0 - ap.tune.y * occ / f32(count), 0.0, 1.0);
}

fn sd_box(p: vec3<f32>, b: AoBox) -> f32 {
    let q = abs(p - b.centre.xyz) - b.half.xyz;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// Boxes that can matter to the 8x8 tile this workgroup shades, found cooperatively.
const TILE_LIST: u32 = 64u;
var<workgroup> tile_pos: array<vec3<f32>, 64>;
var<workgroup> tile_valid: array<u32, 64>;
var<workgroup> tile_sphere: vec4<f32>;
var<workgroup> tile_count: atomic<u32>;
var<workgroup> tile_list: array<u32, 64>;

fn analytic(pw: vec3<f32>, nw: vec3<f32>) -> f32 {
    let count = min(atomicLoad(&tile_count), TILE_LIST);
    let radius = ap.tune.x;
    var occ = 0.0;
    var weight = 1.0;
    var total = 0.0;
    for (var k = 0u; k < 5u; k = k + 1u) {
        let h = radius * (0.1 + 0.9 * f32(k * k) / 16.0);
        let q = pw + nw * h;
        var d = 1.0e9;
        for (var i = 0u; i < count; i = i + 1u) {
            d = min(d, sd_box(q, boxes[tile_list[i]]));
        }
        occ = occ + clamp(1.0 - d / h, 0.0, 1.0) * weight;
        total = total + weight;
        weight = weight * 0.75;
    }
    // Smooth, and never fully black: at most 80 % of the light is taken.
    let o = occ / total;
    return 1.0 - ap.tune.y * 0.8 * o * o * (3.0 - 2.0 * o);
}

@compute @workgroup_size(8, 8)
fn cs_ao(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let in_range = f32(id.x) < ap.sizes.x && f32(id.y) < ap.sizes.y;
    // The AO texture may be smaller than the depth buffer: sample the depth at its centre.
    let scale = ap.sizes.zw / ap.sizes.xy;
    let p = vec2<i32>((vec2<f32>(id.xy) + 0.5) * scale);
    var dc = 0.0;
    if (in_range) {
        dc = load_depth(p);
    }
    let sky = dc <= 0.0;
    var pc = vec3<f32>(0.0);
    var n = vec3<f32>(0.0, 0.0, 1.0);
    if (!sky) {
        pc = view_pos(vec2<f32>(p) + 0.5, dc);
        n = view_normal(p, pc, dc);
    }
    let m = transpose(mat3x3<f32>(ap.view_rot[0].xyz, ap.view_rot[1].xyz, ap.view_rot[2].xyz));
    let pw = m * pc;

    if (ap.lens.w > 1.5) {
        // Tile culling: bound the tile's surface points, then keep the boxes near the bound.
        tile_pos[li] = pw;
        tile_valid[li] = select(1u, 0u, sky);
        if (li == 0u) {
            atomicStore(&tile_count, 0u);
        }
        workgroupBarrier();
        if (li == 0u) {
            var lo = vec3<f32>(1.0e30);
            var hi = vec3<f32>(-1.0e30);
            for (var i = 0u; i < 64u; i = i + 1u) {
                if (tile_valid[i] != 0u) {
                    lo = min(lo, tile_pos[i]);
                    hi = max(hi, tile_pos[i]);
                }
            }
            let c = (lo + hi) * 0.5;
            tile_sphere = vec4<f32>(c, select(-1.0, length(hi - c), lo.x < 1.0e29));
        }
        workgroupBarrier();
        let sph = tile_sphere;
        if (sph.w >= 0.0) {
            for (var i = li; i < u32(ap.tune.z); i = i + 64u) {
                let b = boxes[i];
                // Samples reach the radius along the normal, so a box within that of the tile
                // can change the result.
                if (length(b.centre.xyz - sph.xyz) < sph.w + length(b.half.xyz) + ap.tune.x) {
                    let slot = atomicAdd(&tile_count, 1u);
                    if (slot < TILE_LIST) {
                        tile_list[slot] = i;
                    }
                }
            }
        }
        workgroupBarrier();
    }
    if (!in_range) {
        return;
    }
    if (sky) {
        textureStore(ao_out, id.xy, vec4<f32>(1.0));
        return;
    }
    var ao = 1.0;
    if (ap.lens.w < 1.5) {
        ao = ssao(id.xy, pc, n);
    } else {
        ao = analytic(pw, m * n);
    }
    textureStore(ao_out, id.xy, vec4<f32>(ao, 0.0, 0.0, 1.0));
}
