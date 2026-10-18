// Fragment side of the clustered point lights: finds the fragment's cluster and sums the lights
// in it. Declares group 2, which the scene pipelines share. `common.wgsl` and `shade.wgsl` come
// before this file.

struct LightParams {
    view_rot: mat4x4<f32>,
    grid: vec4<u32>,
    screen: vec4<f32>,
    lens: vec4<f32>,
    counts: vec4<u32>,
};

struct PointLight {
    pos_radius: vec4<f32>,
    colour: vec4<f32>,
};

@group(2) @binding(0) var<uniform> light_params: LightParams;
@group(2) @binding(1) var<storage, read> point_lights: array<PointLight>;
@group(2) @binding(3) var<storage, read> cluster_counts: array<u32>;
@group(2) @binding(4) var<storage, read> cluster_list: array<u32>;

// A windowed inverse-square falloff that reaches exactly zero at the light's radius.
fn light_term(l: PointLight, rel: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let to = l.pos_radius.xyz - rel;
    let d2 = dot(to, to);
    let r = l.pos_radius.w;
    if (d2 >= r * r) {
        return vec3<f32>(0.0);
    }
    let d = sqrt(d2);
    let w = clamp(1.0 - pow(d / r, 4.0), 0.0, 1.0);
    let atten = w * w / (d2 + 1.0);
    let ndotl = max(dot(n, to / max(d, 1.0e-4)), 0.0);
    return l.colour.rgb * (atten * ndotl);
}

// Light arriving at a surface point `rel` (camera-relative) with normal `n`, from the lights of
// its cluster. `frag` is the pixel position and `z` the view depth in metres.
fn point_light_sum(rel: vec3<f32>, n: vec3<f32>, frag: vec2<f32>, z: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    if (light_params.counts.x == 0u) {
        return sum;
    }
    if (light_params.counts.y != 0u) {
        // Debug path: every light, no clusters.
        for (var i = 0u; i < light_params.counts.x; i = i + 1u) {
            sum = sum + light_term(point_lights[i], rel, n);
        }
        return sum;
    }
    let near = light_params.lens.z;
    let far = light_params.lens.w;
    if (z >= far) {
        return sum;
    }
    let slices = f32(light_params.grid.z);
    let s = u32(clamp(log(max(z, near) / near) / log(far / near) * slices, 0.0, slices - 1.0));
    let tx = min(u32(frag.x / light_params.screen.z), light_params.grid.x - 1u);
    let ty = min(u32(frag.y / light_params.screen.z), light_params.grid.y - 1u);
    let c = (s * light_params.grid.y + ty) * light_params.grid.x + tx;
    let n_here = cluster_counts[c];
    let base = c * light_params.grid.w;
    for (var k = 0u; k < n_here; k = k + 1u) {
        sum = sum + light_term(point_lights[cluster_list[base + k]], rel, n);
    }
    return sum;
}
