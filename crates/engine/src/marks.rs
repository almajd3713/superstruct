//! Player-made marks drawn in the world (WAKE-01..02, TOOL-08): the lamps of the trail and the
//! chalk on walls. Points are grouped into chunks anchored at the first of each group, so f32
//! offsets stay small however far apart the marks are.

use glam::Vec3;
use worldgen::coord::WorldPos;
use worldgen::prim::MaterialId;

use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};

/// Points per chunk, and the farthest a point may be from its group's anchor.
const GROUP: usize = 16;
const REACH_M: f64 = 1500.0;

#[derive(Clone, Copy)]
pub enum Shape {
    /// A lamp post, 6 m tall and slim, readable from an overlook.
    Lamp,
    /// A chalk cross on a wall: a small bright cube.
    Chalk,
}

impl Shape {
    fn add(self, mesh: &mut Mesh, at: Vec3) {
        match self {
            Shape::Lamp => {
                mesh.add_box(
                    at - Vec3::new(0.2, 0.0, 0.2),
                    at + Vec3::new(0.2, 6.0, 0.2),
                    MaterialId::WARM.0,
                );
                mesh.add_box(
                    at + Vec3::new(-0.5, 6.0, -0.5),
                    at + Vec3::new(0.5, 6.8, 0.5),
                    MaterialId::SIGNAL.0,
                );
            }
            Shape::Chalk => mesh.add_box(
                at - Vec3::splat(0.25),
                at + Vec3::splat(0.25),
                MaterialId::SIGNAL.0,
            ),
        }
    }
}

struct Group {
    anchor: WorldPos,
    points: Vec<Vec3>,
    handle: Option<ChunkHandle>,
}

pub struct MarkDraw {
    shape: Shape,
    groups: Vec<Group>,
    covered: usize,
}

impl MarkDraw {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            groups: Vec::new(),
            covered: 0,
        }
    }

    /// Draws the points not drawn yet, rebuilding only the chunks they touch.
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        points: &[WorldPos],
    ) {
        if self.covered >= points.len() {
            return;
        }
        for p in &points[self.covered..] {
            let fits = self
                .groups
                .last()
                .is_some_and(|g| g.points.len() < GROUP && p.delta(&g.anchor).length() < REACH_M);
            if !fits {
                self.groups.push(Group {
                    anchor: *p,
                    points: Vec::new(),
                    handle: None,
                });
            }
            if let Some(g) = self.groups.last_mut() {
                g.points.push(p.delta(&g.anchor).as_vec3());
            }
        }
        self.covered = points.len();
        let first_dirty = self.groups.len().saturating_sub(self.touched(points.len()));
        for g in &mut self.groups[first_dirty..] {
            if let Some(h) = g.handle.take() {
                renderer.remove_chunk(h);
            }
            let mut mesh = Mesh::default();
            for &p in &g.points {
                self.shape.add(&mut mesh, p);
            }
            g.handle = Some(renderer.upload_chunk(device, queue, g.anchor, &mesh).0);
        }
    }

    /// How many trailing groups hold points added by the latest update.
    fn touched(&self, _total: usize) -> usize {
        // Groups fill in order, so only the last group, or the last few when a long jump
        // opened new ones, can be stale; rebuilding the tail that has no handle plus the
        // last drawn one is enough.
        let undrawn = self
            .groups
            .iter()
            .rev()
            .take_while(|g| g.handle.is_none())
            .count();
        (undrawn + 1).min(self.groups.len())
    }
}
