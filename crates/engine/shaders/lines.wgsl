// Debug lines: camera-relative positions with a per-vertex colour. `common.wgsl` is prepended.

struct VsIn {
    @location(0) rel: vec3<f32>,
    @location(1) colour: vec4<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = globals.view_proj * vec4<f32>(in.rel, 1.0);
    out.colour = in.colour;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.colour;
}
