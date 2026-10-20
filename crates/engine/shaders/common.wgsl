struct Globals {
    view_proj: mat4x4<f32>,
    // Camera position inside its sector, metres in [0, 1024). 1024 is a multiple of every grid
    // spacing, so the sector index never changes the grid phase.
    cam_local: vec4<f32>,
    // x: fog density, y: grid on/off, z: fade to black, w: debug colour mode.
    params: vec4<f32>,
    // rgb: fog and sky colour (linear), a: fog height falloff per metre.
    fog: vec4<f32>,
    // rgb: ambient tint (linear), a: light level.
    ambient: vec4<f32>,
    // Material table: rgb albedo and emissive in 0..16, roughness in x of 16..32.
    materials: array<vec4<f32>, 32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
