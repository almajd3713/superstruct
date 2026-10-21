//! Atmosphere and its blending across L0 faces. Spec: `docs/spec/regimes.md` §3–4.

use glam::{I64Vec3, Vec3};

use crate::cell::{Level, floor_div_pow2, floor_mod_pow2};
use crate::coord::{SECTOR_SIZE, SECTOR_SIZE_LOG2, WorldPos};
use crate::regime::{RareEvent, Regime, StyleId, regime};
use crate::seed::WorldSeed;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Atmosphere {
    /// Linear RGB.
    pub fog_color: Vec3,
    /// Extinction per metre: `3 / visibility_m` (Koschmieder).
    pub sigma: f32,
    pub ambient: Vec3,
    pub emissive: Vec3,
    pub accent: Vec3,
    /// `light / 255`.
    pub light: f32,
    /// How much thicker than Koschmieder's the haze is felt: the engine uses `sigma * haze`.
    /// It is what makes Pipeworks close and oppressive and Hollow Expanse airy.
    pub haze: f32,
    /// Height falloff of the fog per metre.
    pub height_k: f32,
}

/// One palette: fog, ambient, emissive and accent.
#[derive(Debug)]
pub struct PaletteDef {
    /// `0xRRGGBB` as in the spec table.
    pub srgb: [u32; 4],
    /// The same colours in linear RGB, pinned here because the sRGB curve needs `powf`,
    /// which `hashing.md` §6 bans. A test re-derives them from `srgb`.
    pub linear: [Vec3; 4],
}

/// Indexed by `PaletteId`.
#[allow(clippy::unreadable_literal)] // colours read as #RRGGBB, as in the spec
pub const PALETTES: [PaletteDef; 6] = [
    // GREY
    PaletteDef {
        srgb: [0x3A3C40, 0x8A8D93, 0xC9D4E0, 0xE8E8E8],
        linear: [
            Vec3::new(0.04231141, 0.045186203, 0.051269457),
            Vec3::new(0.2541521, 0.2663556, 0.29177064),
            Vec3::new(0.58407843, 0.65837485, 0.7454042),
            Vec3::new(0.80695224, 0.80695224, 0.80695224),
        ],
    },
    // COLD_BLUE
    PaletteDef {
        srgb: [0x1E2A36, 0x6E8496, 0x9FD3FF, 0xCFE8FF],
        linear: [
            Vec3::new(0.0129830325, 0.023153367, 0.03688945),
            Vec3::new(0.15592647, 0.23074006, 0.3049873),
            Vec3::new(0.34670407, 0.65140563, 1.0),
            Vec3::new(0.6239604, 0.80695224, 1.0),
        ],
    },
    // SEPIA
    PaletteDef {
        srgb: [0x3B3127, 0x9A8670, 0xFFC27A, 0xF2D7B0],
        linear: [
            Vec3::new(0.04373503, 0.030713445, 0.020288562),
            Vec3::new(0.3231432, 0.23839757, 0.16202937),
            Vec3::new(1.0, 0.5394795, 0.19461784),
            Vec3::new(0.8879231, 0.6795425, 0.43415365),
        ],
    },
    // RUST
    PaletteDef {
        srgb: [0x3A2219, 0x8E5B45, 0xFF7A3D, 0xE9B39A],
        linear: [
            Vec3::new(0.04231141, 0.015996294, 0.009721218),
            Vec3::new(0.2704978, 0.104616486, 0.059511237),
            Vec3::new(1.0, 0.19461784, 0.046665087),
            Vec3::new(0.8148466, 0.4507858, 0.3231432),
        ],
    },
    // BONE
    PaletteDef {
        srgb: [0x5A5850, 0xB8B3A3, 0xFFF4D6, 0xFFFFFF],
        linear: [
            Vec3::new(0.10224173, 0.09758735, 0.08021982),
            Vec3::new(0.47932017, 0.4507858, 0.3662526),
            Vec3::new(1.0, 0.9046612, 0.67244315),
            Vec3::new(1.0, 1.0, 1.0),
        ],
    },
    // SICK_GREEN
    PaletteDef {
        srgb: [0x1F2B22, 0x6F8A72, 0xB6FF9E, 0xD8F2C8],
        linear: [
            Vec3::new(0.013702083, 0.024157632, 0.015996294),
            Vec3::new(0.15896083, 0.2541521, 0.1682694),
            Vec3::new(0.4677838, 1.0, 0.34191442),
            Vec3::new(0.6866853, 0.8879231, 0.57758045),
        ],
    },
];

/// Half-width of the blend band on each side of an L0 face, in metres.
pub const BLEND_BAND_M: f32 = 4096.0;

const L0_SIZE_M: f32 = 32768.0;
const SECTORS_PER_L0_LOG2: u32 = Level::L0.size_log2() - SECTOR_SIZE_LOG2;

/// The unblended atmosphere of one regime.
pub fn regime_atmosphere(r: &Regime) -> Atmosphere {
    let [fog, ambient, emissive, accent] = PALETTES[usize::from(r.palette.0)].linear;
    let emissive = if r.event == RareEvent::Blackout {
        Vec3::ZERO
    } else {
        emissive
    };
    Atmosphere {
        fog_color: fog,
        sigma: 3.0 / f32::from(r.visibility_m),
        ambient,
        emissive,
        accent,
        light: f32::from(r.light) / 255.0,
        haze: style_haze(r.style),
        height_k: style_height_k(r.style),
    }
}

/// Aerial perspective per style (P5.1.5, first guesses for the art review): the haze factor on
/// the regime's extinction.
fn style_haze(style: StyleId) -> f32 {
    match style {
        StyleId::PIPEWORKS => 3.0,
        StyleId::PILLAR_FOREST => 1.2,
        StyleId::HOLLOW_EXPANSE => 0.5,
        StyleId::OVERGROWTH => 2.0,
        // Concrete Strata, and any style added later, read as plain Koschmieder haze.
        _ => 1.0,
    }
}

/// How fast the fog thins with height, per metre: close fog hugs the floor, airy regimes keep it
/// high.
fn style_height_k(style: StyleId) -> f32 {
    match style {
        StyleId::PIPEWORKS => 1.0 / 150.0,
        StyleId::PILLAR_FOREST => 1.0 / 800.0,
        StyleId::HOLLOW_EXPANSE => 1.0 / 2000.0,
        StyleId::OVERGROWTH => 1.0 / 250.0,
        _ => 1.0 / 500.0,
    }
}

/// The atmosphere at `pos`, blended over the 2×2×2 L0 cells around it (spec §4).
pub fn atmosphere_at(seed: WorldSeed, pos: &WorldPos) -> Atmosphere {
    let sector = pos.sector().to_array();
    let local = pos.local().to_array();
    let axes: [[(i64, f32); 2]; 3] = std::array::from_fn(|i| {
        let cell = floor_div_pow2(sector[i], SECTORS_PER_L0_LOG2);
        #[allow(clippy::cast_possible_truncation)] // floor_mod < 32
        let a = floor_mod_pow2(sector[i], SECTORS_PER_L0_LOG2) as f32 * SECTOR_SIZE + local[i];
        let (offset, w) = neighbour_weight(a);
        [(cell, 1.0 - w), (cell + offset, w)]
    });

    let mut out = Atmosphere::default();
    for &(x, wx) in &axes[0] {
        for &(y, wy) in &axes[1] {
            for &(z, wz) in &axes[2] {
                let w = wx * wy * wz;
                if w > 0.0 {
                    let l0 = I64Vec3::new(x, y, z);
                    let mut a = regime_atmosphere(&regime(seed, l0));
                    // Fog districts and the Far Lands (STG-07, STG-11).
                    a.sigma *= crate::strange::fog_factor(seed, l0);
                    out.fog_color += a.fog_color * w;
                    out.sigma += a.sigma * w;
                    out.ambient += a.ambient * w;
                    out.emissive += a.emissive * w;
                    out.accent += a.accent * w;
                    out.light += a.light * w;
                    out.haze += a.haze * w;
                    out.height_k += a.height_k * w;
                }
            }
        }
    }
    out
}

/// For a position `a` in `[0, 32768)` along one axis of an L0 cell: which neighbour to blend
/// with (−1 or +1) and its weight in `[0, 0.5]`.
pub fn neighbour_weight(a: f32) -> (i64, f32) {
    if a < BLEND_BAND_M {
        (-1, 0.5 * smoothstep(1.0 - a / BLEND_BAND_M))
    } else if a >= L0_SIZE_M - BLEND_BAND_M {
        (1, 0.5 * smoothstep(1.0 - (L0_SIZE_M - a) / BLEND_BAND_M))
    } else {
        (1, 0.0)
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P5.1.5: every style has its own aerial perspective, Pipeworks the closest and Hollow
    /// Expanse the airiest, and the blend carries haze and falloff like the other fields.
    #[test]
    fn styles_have_distinct_aerial_perspective() {
        let seed = WorldSeed(0);
        let mut seen: std::collections::BTreeMap<u8, (f32, f32)> =
            std::collections::BTreeMap::new();
        for x in -20..20i64 {
            for z in -20..20i64 {
                let r = regime(seed, I64Vec3::new(x, 0, z));
                let a = regime_atmosphere(&r);
                assert!(a.haze > 0.0 && a.height_k > 0.0);
                seen.insert(r.style.0, (a.haze, a.height_k));
            }
        }
        assert_eq!(seen.len(), 5, "all five styles occur");
        let haze = |s: StyleId| seen[&s.0].0;
        assert!(haze(StyleId::PIPEWORKS) > haze(StyleId::OVERGROWTH));
        assert!(haze(StyleId::OVERGROWTH) > haze(StyleId::PILLAR_FOREST));
        assert!(haze(StyleId::PILLAR_FOREST) > haze(StyleId::CONCRETE_STRATA));
        assert!(haze(StyleId::CONCRETE_STRATA) > haze(StyleId::HOLLOW_EXPANSE));
        // At an L0 face the blend is an even mix of the two sides.
        let a = atmosphere_at(
            seed,
            &WorldPos::new(I64Vec3::ZERO, glam::Vec3::splat(512.0)).unwrap(),
        );
        assert!(a.haze > 0.0 && a.height_k > 0.0);
    }

    /// Diagnostic: the first L0 cell of each style, for regime tours (`-- --ignored --nocapture`).
    #[test]
    #[ignore = "diagnostic"]
    fn print_a_cell_of_each_style() {
        let seed = WorldSeed(0);
        let mut shown = std::collections::BTreeSet::new();
        for x in -20..20i64 {
            for z in -20..20i64 {
                let r = regime(seed, I64Vec3::new(x, 0, z));
                if shown.insert(r.style.0) {
                    println!(
                        "style {} palette {} at L0 ({x}, 0, {z}): sector {} 0 {}",
                        r.style.0,
                        r.palette.0,
                        x * 32 + 16,
                        z * 32 + 16
                    );
                }
            }
        }
    }

    #[test]
    fn linear_palette_matches_srgb() {
        let to_linear = |byte: u32| {
            let c = f64::from(byte) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        for p in &PALETTES {
            for (&srgb, &linear) in p.srgb.iter().zip(&p.linear) {
                let expected = [16, 8, 0].map(|shift| to_linear((srgb >> shift) & 0xFF));
                for (got, want) in linear.to_array().into_iter().zip(expected) {
                    assert!((f64::from(got) - want).abs() < 1e-7, "{srgb:06X}");
                }
            }
        }
    }

    #[test]
    #[allow(clippy::float_cmp)] // the weights are exact at these points
    fn neighbour_weight_known_values() {
        assert_eq!(neighbour_weight(0.0), (-1, 0.5));
        assert_eq!(neighbour_weight(1024.0), (-1, 0.5 * 0.843_75));
        assert_eq!(neighbour_weight(BLEND_BAND_M).1, 0.0);
        assert_eq!(neighbour_weight(16384.0).1, 0.0);
        assert_eq!(neighbour_weight(L0_SIZE_M - BLEND_BAND_M).1, 0.0);
        assert_eq!(neighbour_weight(L0_SIZE_M - 1024.0), (1, 0.5 * 0.843_75));
    }

    fn pos_x(x_m: f64, y: i64, z: i64) -> WorldPos {
        #[allow(clippy::cast_possible_truncation)]
        let sector = (x_m / 1024.0).floor() as i64;
        #[allow(clippy::cast_possible_truncation)]
        let local = (x_m - sector as f64 * 1024.0) as f32;
        WorldPos::new(I64Vec3::new(sector, y, z), Vec3::new(local, 512.0, 300.0)).unwrap()
    }

    fn max_diff(a: &Atmosphere, b: &Atmosphere) -> f32 {
        let v = |x: Vec3, y: Vec3| (x - y).abs().max_element();
        v(a.fog_color, b.fog_color)
            .max(v(a.ambient, b.ambient))
            .max(v(a.emissive, b.emissive))
            .max(v(a.accent, b.accent))
            .max((a.light - b.light).abs())
            .max((a.sigma - b.sigma).abs() * 100.0)
    }

    /// P0.6.3: walking across L0 faces, the atmosphere never jumps.
    #[test]
    fn continuous_across_l0_faces() {
        let seed = WorldSeed(0xA7);
        // Max slope of the weight is 0.5 · 1.5 / 4096 per metre; field values differ by ≤ 1
        // (sigma is scaled ×100 in `max_diff`, so ≤ 1.2 there).
        let bound = 1.2 * 0.75 / BLEND_BAND_M * 1.01;
        for face in [-2i64, 0, 1, 7] {
            let face_m = face as f64 * 32768.0;
            let mut prev = atmosphere_at(seed, &pos_x(face_m - 5000.0, 16, -16));
            for step in 1..=10_000 {
                let x = face_m - 5000.0 + f64::from(step);
                let a = atmosphere_at(seed, &pos_x(x, 16, -16));
                let d = max_diff(&prev, &a);
                assert!(d <= bound, "jump {d} at x = {x}");
                prev = a;
            }
        }
    }

    #[test]
    fn face_is_an_even_mix() {
        let seed = WorldSeed(0xA7);
        let at_face = atmosphere_at(seed, &pos_x(32768.0, 16, -16));
        let below = regime_atmosphere(&regime(seed, I64Vec3::new(0, 0, -1)));
        let above = regime_atmosphere(&regime(seed, I64Vec3::new(1, 0, -1)));
        // y = 16 and z = −16 sectors are the centres of L0 cells 0 and −1.
        let mix = f32::midpoint(below.light, above.light);
        assert!((at_face.light - mix).abs() < 1e-6);
    }

    #[test]
    fn cell_interior_is_unblended() {
        let seed = WorldSeed(0xA7);
        let a = atmosphere_at(seed, &pos_x(16384.0 + 32768.0 * 4.0, 16, -16));
        let r = regime_atmosphere(&regime(seed, I64Vec3::new(4, 0, -1)));
        assert_eq!(a, r);
    }

    #[test]
    fn blackout_has_no_light_or_emissive() {
        let seed = WorldSeed(42);
        let r = regime(seed, I64Vec3::new(21, 0, 0));
        assert_eq!(r.event, RareEvent::Blackout);
        let a = regime_atmosphere(&r);
        assert_eq!((a.light, a.emissive), (0.0, Vec3::ZERO));
    }
}
