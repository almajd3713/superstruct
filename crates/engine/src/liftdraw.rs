//! Draws lift cars and gates as renderer chunks (`docs/spec/lifts.md` §7.4).
//!
//! Cars are chunks whose origin is moved every frame. All closed gates share one chunk that is
//! rebuilt only when the set of gates changes.

use glam::{Vec2, Vec3};
use worldgen::coord::WorldPos;
use worldgen::prim::{MaterialId, Shape};

use crate::liftsys::LiftSystem;
use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};

const CAR_THICKNESS: f32 = crate::lift::CAR_THICKNESS;
const BORDER: f32 = 0.25;
const BORDER_RISE: f32 = 0.02;

struct CarMesh {
    size: Vec2,
    handle: ChunkHandle,
}

#[derive(Default)]
pub struct LiftDraw {
    cars: Vec<CarMesh>,
    gates: Option<ChunkHandle>,
    /// Origin and size of every gate drawn now, to notice changes.
    gate_key: Vec<(WorldPos, Vec3)>,
}

fn car_mesh(size: Vec2) -> Mesh {
    let mut mesh = Mesh::default();
    let top = CAR_THICKNESS;
    mesh.add_box(
        Vec3::ZERO,
        Vec3::new(size.x, top, size.y),
        MaterialId::METAL.0,
    );
    let rim = |mesh: &mut Mesh, a: Vec3, b: Vec3| {
        mesh.add_box(a, b, MaterialId::EMISSIVE.0);
    };
    let (lo, hi) = (top, top + BORDER_RISE);
    rim(
        &mut mesh,
        Vec3::new(0.0, lo, 0.0),
        Vec3::new(size.x, hi, BORDER),
    );
    rim(
        &mut mesh,
        Vec3::new(0.0, lo, size.y - BORDER),
        Vec3::new(size.x, hi, size.y),
    );
    rim(
        &mut mesh,
        Vec3::new(0.0, lo, BORDER),
        Vec3::new(BORDER, hi, size.y - BORDER),
    );
    rim(
        &mut mesh,
        Vec3::new(size.x - BORDER, lo, BORDER),
        Vec3::new(size.x, hi, size.y - BORDER),
    );
    mesh
}

impl LiftDraw {
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sys: &LiftSystem,
    ) {
        crate::zone!("lift draw");
        let mut used = vec![false; self.cars.len()];
        for (origin, size) in sys.cars() {
            let slot = self
                .cars
                .iter()
                .enumerate()
                .position(|(i, c)| !used[i] && c.size == size);
            let i = slot.unwrap_or_else(|| {
                let (handle, _) = renderer.upload_chunk(device, queue, origin, &car_mesh(size));
                self.cars.push(CarMesh { size, handle });
                used.push(false);
                self.cars.len() - 1
            });
            used[i] = true;
            renderer.set_chunk_origin(self.cars[i].handle, origin);
            renderer.set_chunk_visible(self.cars[i].handle, true);
        }
        for (car, used) in self.cars.iter().zip(&used) {
            if !used {
                renderer.set_chunk_visible(car.handle, false);
            }
        }
        self.update_gates(renderer, device, queue, sys);
    }

    fn update_gates(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sys: &LiftSystem,
    ) {
        let boxes: Vec<(WorldPos, Vec3)> = sys
            .solids()
            .iter()
            .filter(|s| s.platform.is_none())
            .filter_map(|s| match s.shape {
                Shape::Box { aabb } => Some((s.origin, aabb.max_m())),
                _ => None,
            })
            .collect();
        if boxes == self.gate_key {
            return;
        }
        if let Some(h) = self.gates.take() {
            renderer.remove_chunk(h);
        }
        if let Some(&(anchor, _)) = boxes.first() {
            let mut mesh = Mesh::default();
            for &(origin, size) in &boxes {
                let min = origin.delta(&anchor).as_vec3();
                mesh.add_box(min, min + size, MaterialId::GRATING.0);
            }
            self.gates = Some(renderer.upload_chunk(device, queue, anchor, &mesh).0);
        }
        self.gate_key = boxes;
    }
}
