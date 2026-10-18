//! Drop-in beacons: a slim emissive pillar that stays readable far into the fog. Each is one
//! renderer chunk anchored at the spot where it was dropped.

use glam::Vec3;
use worldgen::coord::WorldPos;
use worldgen::prim::MaterialId;

use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};

/// Pillar height; tall enough to clear the megastructure's nearby silhouettes.
pub const HEIGHT: f32 = 600.0;
/// Wide enough to cover a few pixels at a kilometre.
const WIDTH: f32 = 4.0;

fn pillar() -> Mesh {
    let mut mesh = Mesh::default();
    let h = WIDTH / 2.0;
    mesh.add_box(
        Vec3::new(-2.5, 0.0, -2.5),
        Vec3::new(2.5, 0.3, 2.5),
        MaterialId::METAL.0,
    );
    mesh.add_box(
        Vec3::new(-h, 0.3, -h),
        Vec3::new(h, HEIGHT, h),
        MaterialId::BEACON.0,
    );
    mesh
}

#[derive(Default)]
pub struct BeaconDraw {
    handles: Vec<ChunkHandle>,
}

impl BeaconDraw {
    /// Uploads a pillar for every beacon not drawn yet.
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        beacons: &[WorldPos],
    ) {
        for at in &beacons[self.handles.len().min(beacons.len())..] {
            self.handles
                .push(renderer.upload_chunk(device, queue, *at, &pillar()).0);
        }
    }
}
