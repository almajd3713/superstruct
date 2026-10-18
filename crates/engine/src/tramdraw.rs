//! Draws tram cars as renderer chunks (`docs/spec/trams.md` §5.1, §8).
//!
//! Each car near the player has a body and a door chunk for the lane it is on. The chunks are
//! uploaded when a car first comes near and moved every frame; the doors are hidden while open.

use std::collections::HashMap;

use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::coord::WorldPos;
use worldgen::prim::MaterialId;
use worldgen::tram::TramLineId;

use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};
use crate::tram::{CAR_LENGTH, LANE_WIDTH};
use crate::tramsys::TramSystem;

/// Cars farther than this from the player are not drawn, metres.
const DRAW_RADIUS: f64 = 4500.0;
const Q: f32 = 16.0;

struct CarChunks {
    /// Body and doors for lane 0 and lane 1.
    handles: [[ChunkHandle; 2]; 2],
    shown: Option<(u8, bool)>,
}

#[derive(Default)]
pub struct TramDraw {
    cars: HashMap<(TramLineId, u32), CarChunks>,
}

/// A box of the car in metres: along, cross, height ranges relative to the car's min corner.
fn add(mesh: &mut Mesh, axis: Axis, r: [(f32, f32); 3], material: MaterialId) {
    let (min, max) = match axis {
        Axis::X => (
            Vec3::new(r[0].0, r[2].0, r[1].0),
            Vec3::new(r[0].1, r[2].1, r[1].1),
        ),
        _ => (
            Vec3::new(r[1].0, r[2].0, r[0].0),
            Vec3::new(r[1].1, r[2].1, r[0].1),
        ),
    };
    mesh.add_box(min, max, material.0);
}

/// Body and door meshes for a car on `lane`, with its min corner at the origin.
fn car_meshes(axis: Axis, lane: u8) -> [Mesh; 2] {
    let (len, wid) = (CAR_LENGTH as f32, LANE_WIDTH as f32);
    let median_high = lane == 0;
    // Cross ranges of the median-side and far walls, from the min corner.
    let (near, far) = if median_high {
        ((wid - 0.25, wid), (0.0, 0.25))
    } else {
        ((0.0, 0.25), (wid - 0.25, wid))
    };
    let (floor_lo, floor_hi, wall_hi, roof_hi) = (3.0 / Q, 7.0 / Q, 48.0 / Q, 52.0 / Q);
    let mut body = Mesh::default();
    let mut doors = Mesh::default();
    let mut b = |r: [(f32, f32); 3], m: MaterialId| add(&mut body, axis, r, m);
    b(
        [(0.0, len), (0.0, wid), (floor_lo, floor_hi)],
        MaterialId::METAL,
    );
    b(
        [(0.0, len), (0.0, wid), (wall_hi, roof_hi)],
        MaterialId::METAL,
    );
    b(
        [(0.0, 0.25), (0.0, wid), (floor_hi, wall_hi)],
        MaterialId::EMISSIVE,
    );
    b(
        [(len - 0.25, len), (0.0, wid), (floor_hi, wall_hi)],
        MaterialId::EMISSIVE,
    );
    b(
        [(0.25, len - 0.25), far, (floor_hi, wall_hi)],
        MaterialId::METAL,
    );
    // The median wall, with two 2 m openings at 8 m either side of the centre.
    let mid = len / 2.0;
    for (a, z) in [
        (0.25, mid - 9.0),
        (mid - 7.0, mid + 7.0),
        (mid + 9.0, len - 0.25),
    ] {
        b([(a, z), near, (floor_hi, wall_hi)], MaterialId::METAL);
    }
    // Light strip along the ceiling inside.
    b(
        [
            (2.0, len - 2.0),
            (wid / 2.0 - 0.1, wid / 2.0 + 0.1),
            (wall_hi - 0.05, wall_hi),
        ],
        MaterialId::EMISSIVE,
    );
    for c in [mid - 9.0, mid + 7.0] {
        add(
            &mut doors,
            axis,
            [(c, c + 2.0), near, (floor_hi, wall_hi)],
            MaterialId::GRATING,
        );
    }
    [body, doors]
}

impl TramDraw {
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        trams: &TramSystem,
        player: &WorldPos,
    ) {
        crate::zone!("tram draw");
        let mut seen = Vec::new();
        for (i, k, state) in trams.cars() {
            let session = &trams.sessions[i];
            let key = (session.line.id, k);
            seen.push(key);
            let f = session.frame_of(player);
            let near = (f.along - state.s).abs() <= DRAW_RADIUS
                && f.cross.abs() <= DRAW_RADIUS
                && f.height.abs() <= DRAW_RADIUS;
            let origin = session.world(
                state.s - CAR_LENGTH / 2.0,
                state.cross - LANE_WIDTH / 2.0,
                0.0,
            );
            let lane = state.lane.min(1);
            let doors_shut = state.doors < 1.0;
            if let std::collections::hash_map::Entry::Vacant(slot) = self.cars.entry(key) {
                if !near {
                    continue;
                }
                let axis = session.line.axis;
                let mut handles = [[ChunkHandle(0); 2]; 2];
                for l in 0..2u8 {
                    let [body, doors] = car_meshes(axis, l);
                    handles[usize::from(l)][0] =
                        renderer.upload_chunk(device, queue, origin, &body).0;
                    handles[usize::from(l)][1] =
                        renderer.upload_chunk(device, queue, origin, &doors).0;
                }
                slot.insert(CarChunks {
                    handles,
                    shown: None,
                });
            }
            let Some(car) = self.cars.get_mut(&key) else {
                continue;
            };
            for row in car.handles {
                for h in row {
                    renderer.set_chunk_origin(h, origin);
                }
            }
            let want = near.then_some((lane, doors_shut));
            if car.shown != want {
                for (l, row) in car.handles.iter().enumerate() {
                    let on_lane = want.is_some_and(|(wl, _)| usize::from(wl) == l);
                    renderer.set_chunk_visible(row[0], on_lane);
                    renderer.set_chunk_visible(row[1], on_lane && doors_shut);
                }
                car.shown = want;
            }
        }
        // Forget cars of sessions that were dropped.
        let gone: Vec<_> = self
            .cars
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for key in gone {
            if let Some(car) = self.cars.remove(&key) {
                for h in car.handles.into_iter().flatten() {
                    renderer.remove_chunk(h);
                }
            }
        }
    }
}
