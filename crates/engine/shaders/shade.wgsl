// Shared surface shading for the scene and the horizon pass. `common.wgsl` is prepended.

// The material table lives in the globals (`materials.rs`): ids follow
// `worldgen::prim::MaterialId`, and anything past the table reads the fallback slot.
fn material_albedo(id: u32) -> vec3<f32> {
    return globals.materials[min(id, 15u)].rgb;
}

// Lighting before fog: a Lambert sun plus ambient, tinted by the regime's light, with a little
// sheen on glossy materials and the emissive share of the base tone added back unlit.
fn lit_colour(n: vec3<f32>, material: u32) -> vec3<f32> {
    let slot = min(material, 15u);
    let albedo = globals.materials[slot].rgb;
    let emissive = globals.materials[slot].a;
    let gloss = 1.0 - globals.materials[16u + slot].x;
    let sun = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let lambert = max(dot(n, sun), 0.0);
    // Tuned so a grey palette at light 0.25 matches the earlier flat greybox look.
    let tint = globals.ambient.rgb / 0.27;
    let sheen = gloss * gloss * pow(lambert, 6.0);
    let shade = 0.22 + 0.1 * n.y + 3.0 * globals.ambient.a * (lambert + sheen);
    return mix(albedo * tint * shade, albedo, emissive);
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
