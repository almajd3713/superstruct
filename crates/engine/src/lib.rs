//! Renderer, streaming, camera, input and audio wrappers.

// Time and pixel conversions between f64/u32 and f32 are deliberate and small.
#![allow(clippy::cast_possible_truncation)]

pub mod app;
pub mod camera;
pub mod cellscene;
pub mod chunkmesh;
pub mod demo;
pub mod gpu;
pub mod gpu_timer;
pub mod headless;
pub mod input;
pub mod mesh;
pub mod meshpool;
pub mod overlay;
pub mod primmesh;
pub mod profile;
pub mod readback;
pub mod renderer;
pub mod settings;
pub mod shaders;
pub mod vertex;
pub mod window;
