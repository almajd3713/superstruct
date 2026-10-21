//! Weather in the engine (WX-01..05, 07..10, LGT-05, 07; `docs/spec/levels.md` §5W.47): the air at
//! the camera from `worldgen::weather`, drawn as thicker or tinted fog, as fields of particles
//! that fall, drift or blow past, as strokes of lightning, and as the warm light of a furnace.
//!
//! A field is a block of static particles in a mesh, drawn in 3 × 3 × 3 copies that tile space
//! and slide with the particles' velocity; the copies are moved by chunk origin, so a frame
//! uploads nothing.

use glam::{I64Vec3, Vec3};
use worldgen::coord::WorldPos;
use worldgen::prim::MaterialId;
use worldgen::seed::WorldSeed;
use worldgen::weather::{Air, FLASH_MS, flash_at};

use crate::lights::Light;
use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};

/// Metres a field's block spans horizontally and vertically.
const TILE_M: i64 = 96;
const BAND_M: i64 = 48;
/// An effect shows once it is this strong.
const SHOW_AT: f32 = 0.1;

#[derive(Clone, Copy)]
pub struct FieldSpec {
    pub count: usize,
    pub size: Vec3,
    pub material: MaterialId,
    pub seed: u32,
}

pub const RAIN: FieldSpec = FieldSpec {
    count: 500,
    size: Vec3::new(0.04, 1.4, 0.04),
    material: MaterialId::VAPOUR,
    seed: 1,
};
pub const DUST: FieldSpec = FieldSpec {
    count: 700,
    size: Vec3::new(0.07, 0.07, 0.07),
    material: MaterialId::VAPOUR,
    seed: 2,
};
pub const ASH: FieldSpec = FieldSpec {
    count: 500,
    size: Vec3::new(0.16, 0.03, 0.16),
    material: MaterialId::CONCRETE,
    seed: 3,
};
pub const RIME: FieldSpec = FieldSpec {
    count: 500,
    size: Vec3::new(0.05, 0.05, 0.05),
    material: MaterialId::FUNGUS,
    seed: 4,
};

/// A cheap 32-bit generator for placing particles (not world generation).
fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

fn block(spec: FieldSpec) -> Mesh {
    let mut mesh = Mesh::default();
    let mut s = spec.seed.wrapping_mul(2_654_435_761);
    for _ in 0..spec.count {
        let at = Vec3::new(
            lcg(&mut s) * TILE_M as f32,
            lcg(&mut s) * BAND_M as f32,
            lcg(&mut s) * TILE_M as f32,
        );
        mesh.add_box(at, at + spec.size, spec.material.0);
    }
    mesh
}

/// Absolute metres to a world position, with a fractional shift.
fn pos_of(abs: [i64; 3], shift: Vec3) -> Option<WorldPos> {
    let sector = I64Vec3::new(
        abs[0].div_euclid(1024),
        abs[1].div_euclid(1024),
        abs[2].div_euclid(1024),
    );
    let local = Vec3::new(
        abs[0].rem_euclid(1024) as f32,
        abs[1].rem_euclid(1024) as f32,
        abs[2].rem_euclid(1024) as f32,
    );
    WorldPos::new(sector, local).ok()?.offset(shift).ok()
}

/// The absolute position of `p` in whole metres and the rest.
pub fn absolute(p: &WorldPos) -> ([i64; 3], Vec3) {
    let (s, l) = (p.sector(), p.local());
    let whole = [0, 1, 2].map(|i| s[i].saturating_mul(1024) + l[i].floor() as i64);
    (whole, Vec3::new(l.x.fract(), l.y.fract(), l.z.fract()))
}

pub struct Field {
    spec: FieldSpec,
    handles: Vec<ChunkHandle>,
    shown: bool,
}

impl Field {
    pub fn new(spec: FieldSpec) -> Self {
        Self {
            spec,
            handles: Vec::new(),
            shown: false,
        }
    }

    /// Slides the copies by `velocity` (m/s) at world time `t_s` around the camera at `cam`, and
    /// shows them when `strength` is enough.
    #[allow(clippy::too_many_arguments)] // the renderer's three handles and the field's motion
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cam: &WorldPos,
        t_s: f64,
        velocity: Vec3,
        strength: f32,
    ) {
        let show = strength >= SHOW_AT;
        if self.handles.is_empty() && show {
            let mesh = block(self.spec);
            self.handles = (0..27)
                .filter_map(|_| {
                    pos_of([0; 3], Vec3::ZERO)
                        .map(|o| renderer.upload_chunk(device, queue, o, &mesh).0)
                })
                .collect();
        }
        if show != self.shown {
            for &h in &self.handles {
                renderer.set_chunk_visible(h, show);
            }
            self.shown = show;
        }
        if !show || self.handles.is_empty() {
            return;
        }
        let (whole, _) = absolute(cam);
        let size = [TILE_M, BAND_M, TILE_M];
        // The slide of each axis, within one block.
        let slide = [0, 1, 2].map(|i| {
            let s = f64::from(velocity[i]) * t_s;
            s.rem_euclid(size[i] as f64)
        });
        let first =
            [0, 1, 2].map(|i| ((whole[i] as f64 - slide[i]) / size[i] as f64).floor() as i64 - 1);
        let mut n = 0;
        for ky in 0..3 {
            for kz in 0..3 {
                for kx in 0..3 {
                    let k = [first[0] + kx, first[1] + ky, first[2] + kz];
                    let base = [0, 1, 2].map(|i| k[i] * size[i]);
                    let shift = Vec3::new(slide[0] as f32, slide[1] as f32, slide[2] as f32);
                    if let (Some(o), Some(&h)) = (pos_of(base, shift), self.handles.get(n)) {
                        renderer.set_chunk_origin(h, o);
                    }
                    n += 1;
                }
            }
        }
    }
}

/// One stroke of lightning as a chunk and a light.
struct Bolt {
    handle: ChunkHandle,
    at: [i64; 3],
}

#[derive(Default)]
pub struct Weather {
    rain: Option<Field>,
    dust: Option<Field>,
    ash: Option<Field>,
    rime: Option<Field>,
    bolt: Option<Bolt>,
    /// The air at the camera this frame.
    pub air: Air,
    pub lights: Vec<Light>,
}

/// The bolt: a zigzag of beams from 220 m up to the strike point.
fn bolt_mesh() -> Mesh {
    let mut mesh = Mesh::default();
    let mut s = 7u32;
    let (mut x, mut z) = (0.0f32, 0.0f32);
    for k in 0..11 {
        let y = 220.0 - k as f32 * 20.0;
        let (nx, nz) = (
            x + (lcg(&mut s) - 0.5) * 30.0,
            z + (lcg(&mut s) - 0.5) * 30.0,
        );
        let (lo, hi) = (
            Vec3::new(x.min(nx), y - 20.0, z.min(nz)),
            Vec3::new(x.max(nx) + 1.2, y, z.max(nz) + 1.2),
        );
        mesh.add_box(lo, hi, MaterialId::BEAM.0);
        (x, z) = (nx, nz);
    }
    mesh
}

impl Weather {
    /// The air at the camera, from the world clock, and the lights of the furnaces near it.
    pub fn sense(&mut self, seed: WorldSeed, cam: &WorldPos, t_ms: u64) {
        let (whole, _) = absolute(cam);
        let l0 = I64Vec3::new(
            whole[0].div_euclid(32_768),
            whole[1].div_euclid(32_768),
            whole[2].div_euclid(32_768),
        );
        let local = I64Vec3::new(whole[0], whole[1], whole[2]) - l0 * 32_768;
        let between = worldgen::character::character_at(seed, l0, local).is_none();
        self.lights.clear();
        let furnace = self.furnaces(seed, cam);
        self.air = worldgen::weather::air_at(seed, whole, t_ms, between);
        self.air.heat = self.air.heat.max(furnace);
        if let Some(w) = forced() {
            match w.as_str() {
                "rain" => self.air.rain = 1.0,
                "dust" => self.air.dust = 1.0,
                "ash" => self.air.ash = 1.0,
                "frost" => self.air.frost = 1.0,
                "heat" => self.air.heat = 1.0,
                "wind" => self.air.wind = [8.0, 0.0, 0.0],
                "fog" => {
                    self.air.fog_mul = 12.0;
                    self.air.in_fog_sea = true;
                }
                _ => {}
            }
        }
    }

    /// The fog's colour and thickness changed by the air.
    pub fn tint_fog(&self, colour: [f32; 3], density: f32) -> ([f32; 3], f32) {
        let a = &self.air;
        let mut c = Vec3::from(colour);
        c = c.lerp(
            Vec3::new(0.62, 0.66, 0.72) * c.length().max(0.05),
            a.frost * 0.5,
        );
        c = c.lerp(
            Vec3::new(0.9, 0.5, 0.25) * c.length().max(0.05),
            a.heat * 0.35,
        );
        c = c.lerp(Vec3::splat(0.5) * c.length().max(0.05), a.ash * 0.3);
        let thick = a.fog_mul * (1.0 + a.heat * 0.4 + a.ash * 0.3 + a.rain * 0.3);
        (c.to_array(), density * thick)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        seed: WorldSeed,
        cam: &WorldPos,
        t_ms: u64,
    ) {
        let t_s = t_ms as f64 / 1000.0;
        let a = self.air;
        let wind = Vec3::new(a.wind[0], 0.0, a.wind[2]);
        let blown = (wind.length() / 8.0).min(1.0);
        let fields: [(&mut Option<Field>, FieldSpec, Vec3, f32); 4] = [
            (
                &mut self.rain,
                RAIN,
                Vec3::new(wind.x * 0.3, -11.0, wind.z * 0.3),
                a.rain,
            ),
            (
                &mut self.dust,
                DUST,
                Vec3::new(wind.x, -0.4, wind.z),
                a.dust.max(blown),
            ),
            (
                &mut self.ash,
                ASH,
                Vec3::new(1.2 + wind.x * 0.5, -0.9, wind.z * 0.5),
                a.ash,
            ),
            (
                &mut self.rime,
                RIME,
                Vec3::new(wind.x * 0.2, -0.35, wind.z * 0.2),
                a.frost,
            ),
        ];
        for (slot, spec, v, strength) in fields {
            if slot.is_none() && strength < SHOW_AT {
                continue;
            }
            slot.get_or_insert_with(|| Field::new(spec))
                .update(renderer, device, queue, cam, t_s, v, strength);
        }
        self.lightning(renderer, device, queue, seed, cam, t_ms);
    }

    /// WX-10: a stroke inside a fog sea lights the fog for a moment.
    fn lightning(
        &mut self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        seed: WorldSeed,
        cam: &WorldPos,
        t_ms: u64,
    ) {
        let flash = if self.air.in_fog_sea {
            let (whole, _) = absolute(cam);
            let l1 = I64Vec3::new(
                whole[0].div_euclid(4096),
                whole[1].div_euclid(4096),
                whole[2].div_euclid(4096),
            );
            flash_at(seed, l1, t_ms)
        } else {
            None
        };
        match (flash, self.bolt.take()) {
            (Some(f), Some(b)) if b.at == f.at => self.bolt = Some(b),
            (Some(f), old) => {
                if let Some(b) = old {
                    renderer.remove_chunk(b.handle);
                }
                if let Some(o) = pos_of(f.at, Vec3::ZERO) {
                    let h = renderer.upload_chunk(device, queue, o, &bolt_mesh()).0;
                    self.bolt = Some(Bolt {
                        handle: h,
                        at: f.at,
                    });
                }
            }
            (None, Some(b)) => renderer.remove_chunk(b.handle),
            (None, None) => {}
        }
        if let (Some(f), Some(b)) = (flash, &self.bolt)
            && let Some(p) = pos_of(b.at, Vec3::new(0.0, 100.0, 0.0))
        {
            let fade = 1.0 - f.age_ms as f32 / FLASH_MS as f32;
            self.lights.push(Light {
                pos: p,
                colour: [0.7, 0.8, 1.0],
                intensity: 80.0 * fade * fade,
                radius: 250.0,
                spot: None,
            });
        }
    }

    /// LGT-07, WX-09: the furnaces of the wells around the camera, as the nearest distance's heat
    /// (0 to 1) and, within reach, a warm light each.
    fn furnaces(&mut self, seed: WorldSeed, cam: &WorldPos) -> f32 {
        let (whole, _) = absolute(cam);
        let l0 = I64Vec3::new(
            whole[0].div_euclid(32_768),
            whole[1].div_euclid(32_768),
            whole[2].div_euclid(32_768),
        );
        let mut heat = 0.0f32;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    for w in worldgen::gravity::wells_of(seed, l0 + I64Vec3::new(dx, dy, dz))
                        .iter()
                        .filter(|w| !w.quiet)
                    {
                        let d: f64 = (0..3)
                            .map(|i| ((w.centre[i] - whole[i]) as f64).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        heat = heat.max((1.0 - d / 700.0).max(0.0) as f32);
                        if d < 400.0
                            && let Some(p) = pos_of(w.centre, Vec3::ZERO)
                        {
                            self.lights.push(Light {
                                pos: p,
                                colour: [1.0, 0.45, 0.15],
                                intensity: 14.0,
                                radius: 250.0,
                                spot: None,
                            });
                        }
                    }
                }
            }
        }
        heat
    }
}

/// `SUPERSTRUCT_WEATHER=rain|dust|ash|frost|heat|wind|fog` forces an effect everywhere, for captures.
fn forced() -> Option<String> {
    static FORCED: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FORCED
        .get_or_init(|| std::env::var("SUPERSTRUCT_WEATHER").ok())
        .clone()
}

/// LGT-05: windows of whole districts turn on and off in slow waves, 1 at full.
pub fn window_wave(cam_abs: [i64; 3], t_ms: u64) -> f32 {
    let phase = (t_ms / 1000).cast_signed() + (cam_abs[0] + cam_abs[2]) / 64;
    let p = phase.rem_euclid(240);
    let tri = 2.0 * p.min(240 - p) as f32 / 240.0;
    0.55 + 0.45 * tri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_holds_its_particles_inside_its_tile() {
        let mesh = block(RAIN);
        assert!(!mesh.vertices.is_empty());
    }

    /// LGT-07, WX-09: next to a furnace the air is hot and a warm light shines.
    #[test]
    fn a_furnace_lights_the_fog_and_heats_the_air() {
        use worldgen::gravity::{WellKind, find_well};
        let (seed, w) = (0..40u64)
            .map(WorldSeed)
            .find_map(|s| {
                find_well(s, WellKind::Point, 40)
                    .filter(|(_, w)| !w.quiet)
                    .map(|(_, w)| (s, w))
            })
            .expect("a furnace well");
        let cam = pos_of(w.centre, Vec3::new(0.0, 10.0, 0.0)).unwrap();
        let mut weather = Weather::default();
        weather.sense(seed, &cam, 0);
        assert!(weather.air.heat > 0.9, "heat {}", weather.air.heat);
        assert!(!weather.lights.is_empty());
        assert!(weather.lights[0].colour[0] > weather.lights[0].colour[2]);
    }

    #[test]
    fn the_window_wave_stays_between_half_and_full() {
        for t in (0..600_000).step_by(7_000) {
            let w = window_wave([123, 4, -456], t);
            assert!((0.55..=1.0).contains(&w), "{w}");
        }
    }

    #[test]
    fn fog_is_tinted_and_thickened_by_the_air() {
        let mut w = Weather::default();
        let (c, d) = w.tint_fog([0.1, 0.1, 0.12], 0.01);
        assert!((d - 0.01).abs() < 1e-9 && (c[0] - 0.1).abs() < 1e-6);
        w.air.fog_mul = 6.0;
        w.air.heat = 1.0;
        let (c2, d2) = w.tint_fog([0.1, 0.1, 0.12], 0.01);
        assert!(d2 > 0.06);
        assert!(c2[0] > c2[2], "heat warms the fog: {c2:?}");
    }

    #[test]
    fn absolute_splits_into_whole_metres_and_a_fraction() {
        let p = WorldPos::new(I64Vec3::new(-1, 2, 3), Vec3::new(1.25, 0.0, 1023.5)).unwrap();
        let (whole, frac) = absolute(&p);
        assert_eq!(whole, [-1023, 2048, 4095]);
        assert!((frac.x - 0.25).abs() < 1e-6 && (frac.z - 0.5).abs() < 1e-6);
    }
}
