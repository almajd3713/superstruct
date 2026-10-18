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
pub mod l1_query;
pub mod l1_rim;
pub mod l1_stair;
pub mod l2;
pub mod l2_attach;
pub mod l2_column;
pub mod l2_entrance;
pub mod l2_links;
pub mod l2_mass;
pub mod l2_port;
pub mod l2_rail;
pub mod l2_vista;
pub mod l2_walk;
pub mod l3;
pub mod l3_plan;
pub mod l3_solver;
pub mod l3_tiles;
pub mod l3_validate;
pub mod l4;
pub mod orient;
pub mod port;
pub mod prim;
pub mod regime;
pub mod regime_web;
pub mod rng;
pub mod salt;
pub mod sanctuary;
pub mod sanctuary_room;
pub mod sdf;
pub mod seed;
pub mod tram;
pub mod validate;
pub mod walk;
pub mod walk_oriented;

/// Version of the generator's output. Bump it, and update the golden tests, on any change
/// that alters what the generator produces (CLAUDE.md §3.8).
pub const GEN_VERSION: u32 = 6;
