//! Temporary greybox scene (P1.3.3) until the generated L2 cell replaces it in P1.5.2.

use glam::{I64Vec3, Vec3};
use worldgen::coord::WorldPos;

use crate::mesh::Mesh;

/// A floor slab and a field of pillars of varied height in sector (0,0,0).
pub fn demo_scene() -> Vec<(WorldPos, Mesh)> {
    let origin =
        WorldPos::new(I64Vec3::ZERO, Vec3::new(448.0, 480.0, 448.0)).unwrap_or(WorldPos::ORIGIN);
    let mut mesh = Mesh::default();
    mesh.add_box(Vec3::new(0.0, 0.0, 0.0), Vec3::new(128.0, 32.0, 128.0), 2);
    for ix in 0..12u32 {
        for iz in 0..12u32 {
            let h = 2.0 + ((ix * 7 + iz * 13) % 9) as f32 * 1.5;
            let min = Vec3::new(8.0 + ix as f32 * 9.0, 32.0, 8.0 + iz as f32 * 9.0);
            mesh.add_box(min, min + Vec3::new(3.0, h, 3.0), 1 + (ix + iz) as u16 % 3);
        }
    }
    vec![(origin, mesh)]
}
