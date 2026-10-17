//! A generated L2 cell turned into a renderable mesh, with the cost of producing it.
//! Temporary until streaming (Phase 2) replaces the single static cell.

use std::time::Instant;

use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::coord::{CoordError, WorldPos};
use worldgen::l2::l2_cell;
use worldgen::seed::WorldSeed;

use crate::mesh::Mesh;
use crate::primmesh::add_primitive;

pub struct CellScene {
    pub key: CellKey,
    pub origin: WorldPos,
    pub mesh: Mesh,
    pub primitives: usize,
    pub gen_ms: f32,
    pub mesh_ms: f32,
}

impl CellScene {
    pub fn build(seed: WorldSeed, key: CellKey) -> Result<Self, CoordError> {
        let origin = key.aabb_world()?.min;
        let t0 = Instant::now();
        let out = l2_cell(seed, key);
        let t1 = Instant::now();
        let mut mesh = Mesh::default();
        for prim in &out.primitives {
            add_primitive(&mut mesh, prim);
        }
        let t2 = Instant::now();
        Ok(Self {
            key,
            origin,
            mesh,
            primitives: out.primitives.len(),
            gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
            mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
        })
    }

    pub fn triangles(&self) -> usize {
        self.mesh.indices.len() / 3
    }

    /// One-line summary for the overlay.
    pub fn summary(&self) -> String {
        let c = self.key.coords;
        format!(
            "L2 cell {} {} {}: {} prims, {} tris | gen {:.2} ms, mesh {:.2} ms",
            c.x,
            c.y,
            c.z,
            self.primitives,
            self.triangles(),
            self.gen_ms,
            self.mesh_ms
        )
    }
}

/// The L2 cell with the most primitives within `radius` cells of the origin, ties to the lowest
/// coordinates so the choice is stable.
pub fn busiest_l2_cell(seed: WorldSeed, radius: i64) -> CellKey {
    let mut best = (0usize, CellKey::new(Level::L2, I64Vec3::ZERO));
    for x in -radius..=radius {
        for y in -radius..=radius {
            for z in -radius..=radius {
                let key = CellKey::new(Level::L2, I64Vec3::new(x, y, z));
                let n = l2_cell(seed, key).primitives.len();
                if n > best.0 {
                    best = (n, key);
                }
            }
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "exploration helper: prints the busiest cells"]
    fn print_busy_cells() {
        let seed = WorldSeed(0);
        for radius in [2, 4] {
            let key = busiest_l2_cell(seed, radius);
            let scene = CellScene::build(seed, key).unwrap();
            println!("radius {radius}: {}", scene.summary());
        }
    }

    #[test]
    fn building_is_deterministic() {
        let key = CellKey::new(Level::L2, I64Vec3::new(1, 0, -2));
        let a = CellScene::build(WorldSeed(5), key).unwrap();
        let b = CellScene::build(WorldSeed(5), key).unwrap();
        assert_eq!(a.mesh.vertices, b.mesh.vertices);
        assert_eq!(a.mesh.indices, b.mesh.indices);
    }
}
