//! Distant slow movers (`docs/spec/trams.md` §8, P5.2.5): the trams of lines beyond the reach of
//! the real cars, drawn as faint bright blocks that grow with distance so each stays a few
//! pixels across. They follow the same pure schedule as the cars, so a light seen crawling along
//! the horizon is a tram you can ride when you get there.

use std::collections::HashMap;

use glam::{I64Vec3, Vec3};
use worldgen::cell::{Axis, Level};
use worldgen::coord::WorldPos;
use worldgen::key::LayerKey;
use worldgen::l1::DECK_REGION_LOG2;
use worldgen::prim::MaterialId;
use worldgen::seed::WorldSeed;
use worldgen::tram::tram_lines;

use crate::farinst::{FarData, FarInstance};
use crate::renderer::{ChunkHandle, Renderer};
use crate::tram::CAR_LENGTH;
use crate::tramsys::Session;

/// Cars nearer than this are drawn as real cars (the attach radius is 4 km).
const NEAR_M: f64 = 3500.0;
const FAR_M: f64 = 60_000.0;
/// Layer regions searched around the player, in layers of four L1 cells.
const REGION_REACH: i64 = 4;
/// Layers whose lines are worked out per update.
const LAYERS_PER_UPDATE: usize = 2;
/// Seconds of game time between rebuilds of the instance list.
const REBUILD_S: f64 = 0.25;
/// Pixels a light should span at 90 degrees of FOV and 900 px height, as a fraction of its
/// distance: one pixel is about distance / 450.
const SIZE_PER_M: f64 = 3.0 / 450.0;

#[derive(Default)]
pub struct FarTrams {
    layers: HashMap<LayerKey, Vec<Session>>,
    chunk: Option<ChunkHandle>,
    built_at: Option<f64>,
    built_layers: usize,
}

impl FarTrams {
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        queue: &wgpu::Queue,
        seed: WorldSeed,
        time: f64,
        player: &WorldPos,
    ) {
        crate::zone!("far trams");
        self.find_layers(seed, player);
        let fresh = self.built_at.is_some_and(|t| (time - t).abs() < REBUILD_S)
            && self.built_layers == self.layers.len();
        if fresh {
            return;
        }
        self.built_at = Some(time);
        self.built_layers = self.layers.len();
        let data = self.instances(time, player);
        if let Some(h) = self.chunk.take() {
            renderer.remove_chunk(h);
        }
        self.chunk = Some(renderer.upload_far_chunk(queue, *player, &data).0);
    }

    fn find_layers(&mut self, seed: WorldSeed, player: &WorldPos) {
        let cell = player.sector().div_euclid(I64Vec3::splat(4));
        let region = I64Vec3::new(cell.x.div_euclid(4), cell.y, cell.z.div_euclid(4));
        let mut made = 0;
        'scan: for dy in -1..=1 {
            for dx in -REGION_REACH..=REGION_REACH {
                for dz in -REGION_REACH..=REGION_REACH {
                    let key = LayerKey {
                        level: Level::L1,
                        y: region.y + dy,
                        rx: region.x + dx,
                        rz: region.z + dz,
                    };
                    if self.layers.contains_key(&key) {
                        continue;
                    }
                    let sessions = tram_lines(seed, key)
                        .into_iter()
                        .filter_map(Session::new)
                        .collect();
                    self.layers.insert(key, sessions);
                    made += 1;
                    if made == LAYERS_PER_UPDATE {
                        break 'scan;
                    }
                }
            }
        }
        debug_assert_eq!(DECK_REGION_LOG2, 2, "layer regions are four cells wide");
    }

    fn instances(&self, time: f64, player: &WorldPos) -> FarData {
        let mut data = FarData::default();
        for session in self.layers.values().flatten() {
            for k in 0..session.schedule.cars {
                let state = session.schedule.state(time, k);
                let at = session.world(state.s, state.cross, 2.5);
                let off = at.delta(player);
                let d = off.length();
                if !(NEAR_M..FAR_M).contains(&d) {
                    continue;
                }
                let grow = (d * SIZE_PER_M) as f32;
                let (along, across) = ((CAR_LENGTH as f32).max(grow * 2.0), grow.max(6.0));
                let size = match session.line.axis {
                    Axis::X => Vec3::new(along, across, across),
                    _ => Vec3::new(across, across, along),
                };
                data.boxes.push(FarInstance::axis_aligned(
                    (off.as_vec3() - size * 0.5).to_array(),
                    size.to_array(),
                    u32::from(MaterialId::TRAIN_LIGHT.0),
                ));
            }
        }
        data
    }
}
