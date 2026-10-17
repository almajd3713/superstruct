//! Pure, deterministic world generator. No GPU, windowing, audio or engine dependencies.

#![forbid(unsafe_code)]

pub mod atmosphere;
pub mod cavity;
pub mod cell;
pub mod coord;
pub mod hash;
pub mod key;
pub mod l1;
pub mod l1_bridge;
pub mod l1_port;
pub mod l1_rim;
pub mod port;
pub mod prim;
pub mod regime;
pub mod rng;
pub mod salt;
pub mod sdf;
pub mod seed;

/// Version of the generator's output. Bump it, and update the golden tests, on any change
/// that alters what the generator produces (CLAUDE.md §3.8).
pub const GEN_VERSION: u32 = 1;
