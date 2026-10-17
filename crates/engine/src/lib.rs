//! Renderer, streaming, camera, input and audio wrappers.

// Time and pixel conversions between f64/u32 and f32 are deliberate and small.
#![allow(clippy::cast_possible_truncation)]

pub mod app;
pub mod gpu;
pub mod settings;
pub mod testobj;
pub mod window;
