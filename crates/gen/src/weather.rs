//! Weather and air (WX-01..05, WX-07..10; `docs/spec/levels.md` §5W.47): fog seas and tides,
//! rain, dust fall, ash snow, wind, frost, heat haze and the lightning in a fog sea. Everything
//! is a function of `(seed, position, world clock)` and nothing else: the engine reads it and
//! draws; it changes no geometry.
//!
//! Regions are blobs of the L1 cells (a centre inside the cell and a radius below the cell's
//! size), so a point looks at the 27 cells around it. Floats are used only for the intensity of
//! an effect, never for a decision that the generator depends on.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::hash;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L1_M: i64 = 4096;
const L0_M: i64 = 32_768;

/// A round region: its centre in absolute metres and its radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blob {
    pub centre: [i64; 3],
    pub radius: i64,
}

impl Blob {
    /// 0 outside, rising smoothly to 1 at the centre.
    pub fn coverage(&self, p: [i64; 3]) -> f32 {
        let d2: i128 = (0..3)
            .map(|i| i128::from(p[i] - self.centre[i]).pow(2))
            .sum();
        let r2 = i128::from(self.radius).pow(2);
        if d2 >= r2 {
            return 0.0;
        }
        let t = 1.0 - (d2 as f32 / r2 as f32).sqrt();
        t * t * (3.0 - 2.0 * t)
    }
}

/// What an L1 cell holds of the weather.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Climate {
    pub rain: Option<Blob>,
    pub dust: Option<Blob>,
    pub ash: Option<Blob>,
    pub frost: Option<Blob>,
    pub heat: Option<Blob>,
    /// A wind of `speed` m/s along `dir`, over the whole cell.
    pub wind: Option<([i8; 3], f32)>,
    /// Fog banks drift across the cell.
    pub tide: bool,
}

/// The climate of L1 cell `l1`.
pub fn climate(seed: WorldSeed, l1: I64Vec3) -> Climate {
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::WEATHER));
    let blob = |one_in: u64, lo: u64, hi: u64, rng: &mut Rng| {
        let has = rng.below(one_in) == 0;
        let radius = (lo + rng.below(hi - lo + 1)) as i64;
        let c = [0, 1, 2].map(|i| l1[i] * L1_M + 600 + rng.below(2900) as i64);
        has.then_some(Blob { centre: c, radius })
    };
    let rain = blob(14, 600, 1400, &mut rng);
    let dust = blob(10, 500, 1200, &mut rng);
    let ash = blob(30, 900, 1500, &mut rng);
    let frost = blob(16, 400, 900, &mut rng);
    let heat = blob(16, 300, 800, &mut rng);
    let windy = rng.below(6) == 0;
    let axis = rng.below(3) as usize;
    let sign = if rng.below(2) == 0 { 1 } else { -1 };
    let speed = 3.0 + rng.below(8) as f32;
    let mut dir = [0i8; 3];
    dir[axis] = sign;
    let tide = rng.below(3) == 0;
    Climate {
        rain,
        dust,
        ash,
        frost,
        heat,
        wind: windy.then_some((dir, speed)),
        tide,
    }
}

/// The top of the fog sea of L0 cell `l0`, in absolute metres, if it has one (1 in 4).
pub fn fog_sea_top(seed: WorldSeed, l0: I64Vec3) -> Option<i64> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::FOG_SEA));
    if rng.below(4) != 0 {
        return None;
    }
    Some(l0.y * L0_M + 2048 * (1 + rng.below(14) as i64))
}

/// The air at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Air {
    /// How many times thicker the fog is (WX-01, WX-02).
    pub fog_mul: f32,
    /// 0 to 1 each.
    pub rain: f32,
    pub dust: f32,
    pub ash: f32,
    pub frost: f32,
    pub heat: f32,
    /// Wind in m/s (WX-07).
    pub wind: [f32; 3],
    /// Whether the point is inside a fog sea (lightning may strike).
    pub in_fog_sea: bool,
}

impl Default for Air {
    fn default() -> Self {
        Self {
            fog_mul: 1.0,
            rain: 0.0,
            dust: 0.0,
            ash: 0.0,
            frost: 0.0,
            heat: 0.0,
            wind: [0.0; 3],
            in_fog_sea: false,
        }
    }
}

/// Metres a fog sea takes to thicken below its top.
const SEA_SOFT_M: f32 = 80.0;
/// How many times thicker the sea is than the clear air.
const SEA_THICK: f32 = 12.0;
/// The tide's period in seconds, and how much thicker the bank is than the trough.
const TIDE_S: i64 = 480;
const TIDE_BANK: f32 = 0.7;

fn tri(phase: i64, period: i64) -> f32 {
    let p = phase.rem_euclid(period);
    2.0 * p.min(period - p) as f32 / period as f32
}

/// The air at `p` (absolute metres) at world time `t_ms`; `between` is whether the point is in a
/// Between, where the fog seas, the tides and the rain are.
pub fn air_at(seed: WorldSeed, p: [i64; 3], t_ms: u64, between: bool) -> Air {
    let l1 = I64Vec3::new(
        p[0].div_euclid(L1_M),
        p[1].div_euclid(L1_M),
        p[2].div_euclid(L1_M),
    );
    let mut air = Air::default();
    let max = |a: &mut f32, b: Option<Blob>| {
        if let Some(b) = b {
            *a = a.max(b.coverage(p));
        }
    };
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = climate(seed, l1 + I64Vec3::new(dx, dy, dz));
                if between {
                    max(&mut air.rain, c.rain);
                }
                max(&mut air.dust, c.dust);
                max(&mut air.ash, c.ash);
                max(&mut air.frost, c.frost);
                max(&mut air.heat, c.heat);
                if dx == 0 && dy == 0 && dz == 0 {
                    if let Some((dir, speed)) = c.wind {
                        air.wind = dir.map(|d| f32::from(d) * speed);
                    }
                    if c.tide && between {
                        let phase = (t_ms / 1000) as i64 + (p[0] + p[2]) / 16;
                        air.fog_mul *= 1.0 + TIDE_BANK * tri(phase, TIDE_S);
                    }
                }
            }
        }
    }
    if between {
        let l0 = l1.div_euclid(I64Vec3::splat(8));
        if let Some(top) = fog_sea_top(seed, l0)
            && p[1] <= top
        {
            let depth = ((top - p[1]) as f32 / SEA_SOFT_M).min(1.0);
            let k = depth * depth * (3.0 - 2.0 * depth);
            air.fog_mul *= 1.0 + (SEA_THICK - 1.0) * k;
            air.in_fog_sea = true;
        }
    }
    air
}

/// A stroke of lightning: where it is (absolute metres) and how long ago it began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flash {
    pub at: [i64; 3],
    pub age_ms: u64,
}

/// How long a stroke lasts, and the slot in which one may fall.
pub const FLASH_MS: u64 = 400;
const SLOT_MS: u64 = 8_000;

/// WX-10: the stroke over L1 cell `l1` at world time `t_ms`, if any: one slot in three holds
/// one, somewhere within 300 m of the cell's middle plane (a conduit in the cloud).
pub fn flash_at(seed: WorldSeed, l1: I64Vec3, t_ms: u64) -> Option<Flash> {
    let slot = t_ms / SLOT_MS;
    let key = l1 + I64Vec3::new(slot as i64 * 1_000_003, 0, 0);
    let mut rng = Rng::new(hash(seed.0, Level::L1, key, salt::FLASH));
    if rng.below(3) != 0 {
        return None;
    }
    let start = slot * SLOT_MS + rng.below(SLOT_MS - FLASH_MS);
    if t_ms < start || t_ms >= start + FLASH_MS {
        return None;
    }
    let at = [0, 1, 2].map(|i| l1[i] * L1_M + 200 + rng.below(3600) as i64);
    Some(Flash {
        at,
        age_ms: t_ms - start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(2);

    /// The air is a pure function of the point and the clock.
    #[test]
    fn the_air_is_a_pure_function() {
        for k in 0..50i64 {
            let p = [k * 977, 4000 + k * 31, -k * 1301];
            assert_eq!(air_at(SEED, p, 12_345, true), air_at(SEED, p, 12_345, true));
        }
    }

    /// Every effect occurs somewhere, and none everywhere.
    #[test]
    fn every_effect_occurs_and_none_is_everywhere() {
        let mut seen = [0usize; 7];
        let n = 600;
        for k in 0..n {
            let p = [
                k * 5_113 - 1_500_000,
                6_000 + (k % 40) * 3_000,
                k * 7_919 % 900_000,
            ];
            let a = air_at(SEED, p, 0, true);
            for (i, v) in [
                a.rain,
                a.dust,
                a.ash,
                a.frost,
                a.heat,
                a.wind[0].abs() + a.wind[2].abs() + a.wind[1].abs(),
                a.fog_mul - 1.0,
            ]
            .into_iter()
            .enumerate()
            {
                seen[i] += usize::from(v > 0.01);
            }
        }
        for (i, &s) in seen.iter().enumerate() {
            assert!(s > 0 && s < n as usize, "effect {i} at {s} of {n}");
        }
    }

    /// WX-01: below the top of a fog sea the fog is many times thicker, above it clear.
    #[test]
    fn a_fog_sea_is_thick_below_its_top() {
        let l0 = (-30..30)
            .flat_map(|x| (-30..30).map(move |z| I64Vec3::new(x, 0, z)))
            .find(|&c| fog_sea_top(SEED, c).is_some())
            .unwrap();
        let top = fog_sea_top(SEED, l0).unwrap();
        let (x, z) = (l0.x * L0_M + 16_000, l0.z * L0_M + 16_000);
        let below = air_at(SEED, [x, top - 200, z], 0, true);
        let above = air_at(SEED, [x, top + 200, z], 0, true);
        assert!(below.in_fog_sea && below.fog_mul > 8.0, "{}", below.fog_mul);
        assert!(!above.in_fog_sea);
        // Only in a Between.
        assert!(!air_at(SEED, [x, top - 200, z], 0, false).in_fog_sea);
    }

    /// WX-02: the tide thickens and thins the fog with the clock.
    #[test]
    fn fog_tides_come_and_go() {
        let l1 = (-30..30)
            .flat_map(|x| (-30..30).map(move |z| I64Vec3::new(x, 100, z)))
            .find(|&c| climate(SEED, c).tide)
            .unwrap();
        let p = [l1.x * L1_M + 100, l1.y * L1_M, l1.z * L1_M + 100];
        let ms = |s: u64| air_at(SEED, p, s * 1000, true).fog_mul;
        let samples: Vec<f32> = (0..16).map(|k| ms(k * 30)).collect();
        let (lo, hi) = samples
            .iter()
            .fold((f32::MAX, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi > lo * 1.3, "{lo}..{hi}");
    }

    /// WX-10: strokes come about one slot in three and last under half a second.
    #[test]
    fn lightning_strikes_now_and_then() {
        let l1 = I64Vec3::new(3, 1, -2);
        let mut slots = 0;
        let mut ms = 0;
        for t in 0..(8_000 * 60) {
            if let Some(f) = flash_at(SEED, l1, t) {
                ms += 1;
                assert!(f.age_ms < FLASH_MS);
            }
            if t % 8_000 == 0 && (0..8_000).any(|k| flash_at(SEED, l1, t + k).is_some()) {
                slots += 1;
            }
        }
        assert!((10..40).contains(&slots), "{slots} of 60");
        assert!(ms > 0);
    }
}
