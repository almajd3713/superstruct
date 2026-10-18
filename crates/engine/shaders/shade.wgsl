// Shared surface shading for the scene and the horizon pass. `common.wgsl` is prepended.

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

// Lighting before fog: a Lambert sun plus ambient, tinted by the regime's light.
fn lit_colour(n: vec3<f32>, material: u32) -> vec3<f32> {
    let sun = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let lambert = max(dot(n, sun), 0.0);
    // Tuned so a grey palette at light 0.25 matches the earlier flat greybox look.
    let tint = globals.ambient.rgb / 0.27;
    let shade = 0.22 + 0.1 * n.y + 3.0 * globals.ambient.a * lambert;
    return material_albedo(material) * tint * shade;
}

// Fog and the death fade, for a surface point `rel` metres from the camera.
fn apply_fog(colour_in: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    // Exponential height fog: density sigma * exp(-k h) with h measured from the camera, integrated
    // along the ray analytically. Thicker below the camera, thinner above.
    let dist = length(rel);
    let k = globals.fog.a;
    let kh = clamp(k * rel.y, -12.0, 12.0);
    var path = dist;
    if (abs(kh) > 1e-4) {
        path = dist * (1.0 - exp(-kh)) / kh;
    }
    let fog = 1.0 - exp(-globals.params.x * path);
    var colour = mix(colour_in, globals.fog.rgb, fog);
    colour = colour * (1.0 - globals.params.z);
    return colour;
}
