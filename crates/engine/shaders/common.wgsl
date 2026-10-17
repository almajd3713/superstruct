struct Globals {
    view_proj: mat4x4<f32>,
    // Camera position inside its sector, metres in [0, 1024). 1024 is a multiple of every grid
    // spacing, so the sector index never changes the grid phase.
    cam_local: vec4<f32>,
    // x: fog density, y: grid on/off.
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
