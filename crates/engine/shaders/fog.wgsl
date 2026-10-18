// Froxel volumetric fog, compute side. A 3D grid of view-space froxels (screen tiles by
// exponential depth slices) holds, per froxel, the light scattered towards the camera per metre
// and the extinction. `cs_inject` fills it from the regime's height fog, local fog volumes and
// the clustered lights, blended with last frame's grid; `cs_integrate` marches each column
// front to back into accumulated in-scatter and transmittance, which `volfog.wgsl` samples.
// Standalone: no `common.wgsl`.

struct FogParams {
    view_rot: mat4x4<f32>,
    prev_view_rot: mat4x4<f32>,
    // Grid size x, y, z and the frame counter.
    grid: vec4<u32>,
    // Screen width and height, near and far of the grid (metres).
    screen: vec4<f32>,
    // tan(fov_y / 2), aspect, history blend (0 = no history), volume count.
    lens: vec4<f32>,
    // Fog colour (linear) and the height falloff per metre.
    colour: vec4<f32>,
    // Extinction at the camera's height, phase anisotropy, light gain, enabled.
    medium: vec4<f32>,
    // Previous camera position minus this one, in this frame's camera-relative axes.
    delta: vec4<f32>,
};

struct Volume {
    // Camera-relative centre and half size.
    centre: vec4<f32>,
    half_density: vec4<f32>,
};

struct LightParams {
    view_rot: mat4x4<f32>,
    grid: vec4<u32>,
    screen: vec4<f32>,
    lens: vec4<f32>,
    counts: vec4<u32>,
};

struct PointLight {
    pos_radius: vec4<f32>,
    // Linear colour times intensity, and the spot's inner cosine.
    colour: vec4<f32>,
    // Spot direction and outer cosine; an outer cosine below -1.5 means a plain point light.
    dir_cone: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fp: FogParams;
@group(0) @binding(1) var<uniform> light_params: LightParams;
@group(0) @binding(2) var<storage, read> point_lights: array<PointLight>;
@group(0) @binding(3) var<storage, read> cluster_counts: array<u32>;
@group(0) @binding(4) var<storage, read> cluster_list: array<u32>;
@group(0) @binding(5) var<storage, read> volumes: array<Volume>;
@group(0) @binding(6) var history: texture_3d<f32>;
@group(0) @binding(7) var hist_sampler: sampler;
@group(0) @binding(8) var scatter_out: texture_storage_3d<rgba16float, write>;

fn slice_depth(z: f32) -> f32 {
    return fp.screen.z * pow(fp.screen.w / fp.screen.z, z / f32(fp.grid.z));
}

fn hash3(p: vec3<u32>, frame: u32) -> f32 {
    var h = p.x * 374761393u + p.y * 668265263u + p.z * 2147483647u + frame * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

// Henyey-Greenstein, scaled so the isotropic case is 1.
fn phase(cos_t: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_t;
    return (1.0 - g * g) / (d * sqrt(d));
}

fn light_in(l: PointLight, p: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    let to = l.pos_radius.xyz - p;
    let d2 = dot(to, to);
    let r = l.pos_radius.w;
    if (d2 >= r * r) {
        return vec3<f32>(0.0);
    }
    let d = sqrt(d2);
    let w = clamp(1.0 - pow(d / r, 4.0), 0.0, 1.0);
    var atten = w * w / (d2 + 1.0);
    let dir = to / max(d, 1.0e-4);
    if (l.dir_cone.w > -1.5) {
        atten = atten * smoothstep(l.dir_cone.w, l.colour.w, dot(-dir, l.dir_cone.xyz));
    }
    // The light travels from the lamp to the point (-dir) and scatters towards the eye
    // (-view_dir, view_dir pointing from the eye into the scene): the angle between the two
    // is the one `phase` takes, and it is 1 when the lamp shines straight at the viewer.
    let ph = phase(dot(dir, view_dir), fp.medium.y);
    return l.colour.rgb * (atten * ph);
}

@compute @workgroup_size(8, 8, 4)
fn cs_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= fp.grid.x || id.y >= fp.grid.y || id.z >= fp.grid.z) {
        return;
    }
    let size = vec3<f32>(fp.grid.xyz);
    // Jitter along depth, different every frame, so the history averages the slicing away.
    let jitter = hash3(id, fp.grid.w) - 0.5;
    let zs = f32(id.z) + 0.5 + jitter * 0.9;
    let d = slice_depth(zs);
    let ndc = vec2<f32>((f32(id.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(id.y) + 0.5) / size.y * 2.0);
    let kx = fp.lens.x * fp.lens.y;
    let ky = fp.lens.x;
    let v = vec3<f32>(ndc.x * kx * d, ndc.y * ky * d, -d);
    // View rotations are orthonormal: the inverse is the transpose.
    let w = transpose(mat3x3<f32>(fp.view_rot[0].xyz, fp.view_rot[1].xyz, fp.view_rot[2].xyz)) * v;
    let view_dir = normalize(w);

    var ext = fp.medium.x * exp(-clamp(fp.colour.w * w.y, -12.0, 12.0));
    let n_vol = u32(fp.lens.w);
    for (var i = 0u; i < n_vol; i = i + 1u) {
        let vol = volumes[i];
        let q = abs(w - vol.centre.xyz) / max(vol.half_density.xyz, vec3<f32>(1.0e-3));
        let edge = max(q.x, max(q.y, q.z));
        ext = ext + vol.half_density.w * (1.0 - smoothstep(0.6, 1.0, edge));
    }
    // The fog colour is the ambient light the medium scatters.
    var rad = fp.colour.rgb;
    if (light_params.counts.x > 0u && d < light_params.lens.w) {
        let pix = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * light_params.screen.xy;
        let lslices = f32(light_params.grid.z);
        let s = u32(clamp(log(max(d, light_params.lens.z) / light_params.lens.z)
            / log(light_params.lens.w / light_params.lens.z) * lslices, 0.0, lslices - 1.0));
        let tx = min(u32(pix.x / light_params.screen.z), light_params.grid.x - 1u);
        let ty = min(u32(pix.y / light_params.screen.z), light_params.grid.y - 1u);
        let c = (s * light_params.grid.y + ty) * light_params.grid.x + tx;
        let base = c * light_params.grid.w;
        for (var k = 0u; k < cluster_counts[c]; k = k + 1u) {
            rad = rad + light_in(point_lights[cluster_list[base + k]], w, view_dir) * fp.medium.z;
        }
    }
    var out = vec4<f32>(rad * ext, ext);

    // Last frame's grid, reprojected.
    if (fp.lens.z > 0.0) {
        let wp = w - fp.delta.xyz;
        let vp = (fp.prev_view_rot * vec4<f32>(wp, 0.0)).xyz;
        let dp = -vp.z;
        if (dp > fp.screen.z && dp < fp.screen.w) {
            let uv = vec2<f32>(vp.x / (kx * dp) * 0.5 + 0.5, 0.5 - vp.y / (ky * dp) * 0.5);
            let zf = log(dp / fp.screen.z) / log(fp.screen.w / fp.screen.z);
            if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
                let h = textureSampleLevel(history, hist_sampler, vec3<f32>(uv, zf), 0.0);
                out = mix(out, h, fp.lens.z);
            }
        }
    }
    textureStore(scatter_out, id, out);
}

@group(1) @binding(0) var<uniform> ip: FogParams;
@group(1) @binding(1) var scatter_in: texture_3d<f32>;
@group(1) @binding(2) var integrated: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= ip.grid.x || id.y >= ip.grid.y) {
        return;
    }
    var acc = vec3<f32>(0.0);
    var trans = 1.0;
    var prev_d = 0.0;
    for (var z = 0u; z < ip.grid.z; z = z + 1u) {
        let d = ip.screen.z * pow(ip.screen.w / ip.screen.z, f32(z + 1u) / f32(ip.grid.z));
        let dz = d - prev_d;
        prev_d = d;
        let s = textureLoad(scatter_in, vec3<u32>(id.xy, z), 0);
        let ext = max(s.a, 1.0e-6);
        let t = exp(-ext * dz);
        // In-scatter over the slice, integrated exactly for a constant medium.
        acc = acc + trans * s.rgb * (1.0 - t) / ext;
        trans = trans * t;
        textureStore(integrated, vec3<u32>(id.xy, z), vec4<f32>(acc, trans));
    }
}
