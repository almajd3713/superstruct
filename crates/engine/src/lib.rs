//! Renderer, streaming, camera, input and audio wrappers.

// Time and pixel conversions between f64/u32 and f32 are deliberate and small.
#![allow(clippy::cast_possible_truncation)]

pub mod app;
pub mod camera;
pub mod demo;
pub mod gpu;
pub mod input;
pub mod mesh;
pub mod readback;
pub mod renderer;
pub mod settings;
pub mod vertex;
pub mod window;
