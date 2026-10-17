//! L0 regimes: one architectural character and atmosphere per 32 km cell.
//! Spec: `docs/spec/regimes.md`.

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::hash;
use crate::key::ColumnKey;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Regime {
    pub style: StyleId,
    /// 0 = hollow … 255 = packed.
    pub density: u8,
    /// 0 = slabs and decks … 255 = pillars and shafts.
    pub verticality: u8,
    pub palette: PaletteId,
    /// Fog visibility, a multiple of 250 m.
    pub visibility_m: u16,
    /// Baseline light level, 0 = black.
    pub light: u8,
    pub event: RareEvent,
}

/// Append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StyleId(pub u8);

impl StyleId {
    pub const CONCRETE_STRATA: Self = Self(0);
    pub const PIPEWORKS: Self = Self(1);
    pub const PILLAR_FOREST: Self = Self(2);
    pub const HOLLOW_EXPANSE: Self = Self(3);
    pub const OVERGROWTH: Self = Self(4);

    pub fn def(self) -> &'static StyleDef {
        &STYLES[usize::from(self.0)]
    }
}

/// Append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PaletteId(pub u8);

impl PaletteId {
    pub const GREY: Self = Self(0);
    pub const COLD_BLUE: Self = Self(1);
    pub const SEPIA: Self = Self(2);
    pub const RUST: Self = Self(3);
    pub const BONE: Self = Self(4);
    pub const SICK_GREEN: Self = Self(5);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RareEvent {
    None,
    GiantCavity,
    Blackout,
    /// Flag only until P5.2.4.
    GravityRegion,
}

impl RareEvent {
    /// Draw order for `pick_weighted`, with weights out of 1000 (spec §5.1).
    pub const TABLE: [(Self, u32); 4] = [
        (Self::None, 920),
        (Self::GiantCavity, 30),
        (Self::Blackout, 30),
        (Self::GravityRegion, 20),
    ];
}

/// One row of the style table (spec §2). Every range is inclusive.
#[derive(Debug)]
pub struct StyleDef {
    pub name: &'static str,
    pub weight: u32,
    pub density: (u8, u8),
    pub verticality: (u8, u8),
    pub palettes: &'static [PaletteId],
    pub visibility_m: (u16, u16),
    pub light: (u8, u8),
}

/// Indexed by `StyleId`.
pub const STYLES: [StyleDef; 5] = [
    StyleDef {
        name: "Concrete Strata",
        weight: 30,
        density: (96, 200),
        verticality: (40, 140),
        palettes: &[PaletteId::GREY, PaletteId::COLD_BLUE],
        visibility_m: (4_000, 12_000),
        light: (40, 90),
    },
    StyleDef {
        name: "Pipeworks",
        weight: 25,
        density: (140, 255),
        verticality: (60, 180),
        palettes: &[PaletteId::SEPIA, PaletteId::RUST],
        visibility_m: (2_000, 6_000),
        light: (30, 70),
    },
    StyleDef {
        name: "Pillar Forest",
        weight: 20,
        density: (60, 160),
        verticality: (170, 255),
        palettes: &[PaletteId::GREY, PaletteId::BONE],
        visibility_m: (6_000, 20_000),
        light: (50, 100),
    },
    StyleDef {
        name: "Hollow Expanse",
        weight: 15,
        density: (0, 80),
        verticality: (0, 120),
        palettes: &[PaletteId::COLD_BLUE, PaletteId::BONE],
        visibility_m: (15_000, 30_000),
        light: (20, 60),
    },
    StyleDef {
        name: "Overgrowth",
        weight: 10,
        density: (100, 220),
        verticality: (80, 200),
        palettes: &[PaletteId::SICK_GREEN, PaletteId::SEPIA],
        visibility_m: (3_000, 8_000),
        light: (25, 60),
    },
];

pub const VISIBILITY_STEP_M: u16 = 250;

/// Blackout forces visibility into this range (spec §5.1).
pub const BLACKOUT_VISIBILITY_M: (u16, u16) = (1_000, 3_000);

/// The regime of L0 cell `cell` (spec §1).
pub fn regime(seed: WorldSeed, cell: I64Vec3) -> Regime {
    let mut rng = Rng::new(hash(seed.0, Level::L0, cell, salt::REGIME));
    let weights = STYLES.map(|s| s.weight);
    // The table has nonzero weights, so a style is always picked.
    let style_index = rng.pick_weighted(&weights).unwrap_or(0);
    let def = &STYLES[style_index];
    let density = range_u8(&mut rng, def.density);
    let verticality = range_u8(&mut rng, def.verticality);
    let palette = pick(&mut rng, def.palettes);
    let mut visibility_m = range_visibility(&mut rng, def.visibility_m);
    let mut light = range_u8(&mut rng, def.light);

    let (event, mut event_rng) = rare_event_draw(seed, cell);
    if event == RareEvent::Blackout {
        light = 0;
        visibility_m = range_visibility(&mut event_rng, BLACKOUT_VISIBILITY_M);
    }

    Regime {
        #[allow(clippy::cast_possible_truncation)] // fewer than 256 styles
        style: StyleId(style_index as u8),
        density,
        verticality,
        palette,
        visibility_m,
        light,
        event,
    }
}

/// The rare event of L0 cell `cell` (spec §5.1).
pub fn rare_event(seed: WorldSeed, cell: I64Vec3) -> RareEvent {
    rare_event_draw(seed, cell).0
}

/// The event and the RNG after its draw, which supplies the event's own parameters.
fn rare_event_draw(seed: WorldSeed, cell: I64Vec3) -> (RareEvent, Rng) {
    let mut rng = Rng::new(hash(seed.0, Level::L0, cell, salt::RARE_EVENT));
    let weights = RareEvent::TABLE.map(|(_, w)| w);
    let event = rng
        .pick_weighted(&weights)
        .map_or(RareEvent::None, |i| RareEvent::TABLE[i].0);
    (event, rng)
}

/// The infinite staircase through one L0 column (spec §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RareStair {
    /// The L1 column it occupies inside the L0 column, each in `0..8`.
    pub l1_x: u8,
    pub l1_z: u8,
}

/// Staircase presence per L0 column, out of 1000.
pub const RARE_STAIR_PER_MILLE: u64 = 4;

/// The infinite staircase of an L0 column, if it has one. The footprint inside the L1 column
/// is placed by L1 under its separation rule.
#[allow(clippy::cast_possible_truncation)] // below(8) < 8
pub fn rare_stair(seed: WorldSeed, column: ColumnKey) -> Option<RareStair> {
    debug_assert_eq!(column.level, Level::L0);
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L0,
        column.hash_coords(),
        salt::RARE_STAIR,
    ));
    if rng.below(1000) >= RARE_STAIR_PER_MILLE {
        return None;
    }
    Some(RareStair {
        l1_x: rng.below(8) as u8,
        l1_z: rng.below(8) as u8,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // result in [lo, hi]
fn range_u8(rng: &mut Rng, (lo, hi): (u8, u8)) -> u8 {
    rng.range_i32(i32::from(lo), i32::from(hi) + 1) as u8
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // result in [lo, hi]
fn range_visibility(rng: &mut Rng, (lo, hi): (u16, u16)) -> u16 {
    let step = i32::from(VISIBILITY_STEP_M);
    let steps = rng.range_i32(i32::from(lo) / step, i32::from(hi) / step + 1);
    (steps * step) as u16
}

#[allow(clippy::cast_possible_truncation)] // index < len
fn pick<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[rng.below(items.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_table_is_well_formed() {
        for (i, s) in STYLES.iter().enumerate() {
            assert!(s.weight > 0, "{}", s.name);
            assert!(s.density.0 <= s.density.1 && s.verticality.0 <= s.verticality.1);
            assert!(s.light.0 <= s.light.1 && s.visibility_m.0 <= s.visibility_m.1);
            assert_eq!(s.visibility_m.0 % VISIBILITY_STEP_M, 0);
            assert_eq!(s.visibility_m.1 % VISIBILITY_STEP_M, 0);
            assert!(!s.palettes.is_empty());
            assert_eq!(StyleId(u8::try_from(i).unwrap()).def().name, s.name);
        }
    }

    /// Pinned outputs. A change here is a generator output change: bump `GEN_VERSION`.
    #[test]
    fn golden() {
        let cases: &[(u64, [i64; 3], Regime)] = &[
            (
                0,
                [0, 0, 0],
                Regime {
                    style: StyleId(0),
                    density: 154,
                    verticality: 116,
                    palette: PaletteId(1),
                    visibility_m: 2250,
                    light: 0,
                    event: RareEvent::Blackout,
                },
            ),
            (
                1,
                [0, 0, 0],
                Regime {
                    style: StyleId(1),
                    density: 189,
                    verticality: 155,
                    palette: PaletteId(2),
                    visibility_m: 2000,
                    light: 59,
                    event: RareEvent::None,
                },
            ),
            (
                7,
                [-100, 2, 33],
                Regime {
                    style: StyleId(2),
                    density: 75,
                    verticality: 243,
                    palette: PaletteId(4),
                    visibility_m: 6500,
                    light: 76,
                    event: RareEvent::None,
                },
            ),
            (
                42,
                [-1, 3, 7],
                Regime {
                    style: StyleId(2),
                    density: 155,
                    verticality: 185,
                    palette: PaletteId(0),
                    visibility_m: 2250,
                    light: 0,
                    event: RareEvent::Blackout,
                },
            ),
            (
                42,
                [8, 0, 0],
                Regime {
                    style: StyleId(4),
                    density: 120,
                    verticality: 198,
                    palette: PaletteId(2),
                    visibility_m: 5750,
                    light: 41,
                    event: RareEvent::GiantCavity,
                },
            ),
            (
                0xDEAD_BEEF,
                [1 << 40, -(1 << 40), -5],
                Regime {
                    style: StyleId(0),
                    density: 144,
                    verticality: 119,
                    palette: PaletteId(1),
                    visibility_m: 11750,
                    light: 71,
                    event: RareEvent::None,
                },
            ),
        ];
        for &(seed, cell, expected) in cases {
            let got = regime(WorldSeed(seed), I64Vec3::from_array(cell));
            assert_eq!(got, expected, "seed {seed} cell {cell:?}");
        }
    }

    fn within<T: PartialOrd + Copy>(v: T, (lo, hi): (T, T)) -> bool {
        (lo..=hi).contains(&v)
    }

    /// A 64×64 slab of L0 cells: every field lies in its style's range, and styles follow
    /// their weights.
    #[test]
    fn histogram_64x64() {
        let seed = WorldSeed(0x5EED);
        let mut style_counts = [0u32; STYLES.len()];
        for x in -32..32 {
            for z in -32..32 {
                let r = regime(seed, I64Vec3::new(x, 0, z));
                let def = r.style.def();
                style_counts[usize::from(r.style.0)] += 1;
                assert!(within(r.density, def.density), "{r:?}");
                assert!(within(r.verticality, def.verticality), "{r:?}");
                assert!(def.palettes.contains(&r.palette), "{r:?}");
                assert_eq!(r.visibility_m % VISIBILITY_STEP_M, 0, "{r:?}");
                if r.event == RareEvent::Blackout {
                    assert_eq!(r.light, 0);
                    assert!(within(r.visibility_m, BLACKOUT_VISIBILITY_M), "{r:?}");
                } else {
                    assert!(within(r.light, def.light), "{r:?}");
                    assert!(within(r.visibility_m, def.visibility_m), "{r:?}");
                }
            }
        }
        let n = 64.0 * 64.0;
        let total: u32 = STYLES.iter().map(|s| s.weight).sum();
        let chi2: f64 = STYLES
            .iter()
            .zip(style_counts)
            .map(|(s, c)| {
                let e = n * f64::from(s.weight) / f64::from(total);
                (f64::from(c) - e).powi(2) / e
            })
            .sum();
        // 4 degrees of freedom: p = 0.001 at 18.47.
        assert!(
            chi2 < 18.47,
            "style chi-square {chi2:.2}, counts {style_counts:?}"
        );
    }

    /// Within each style, every inclusive range is reached at both ends and is roughly uniform.
    #[test]
    fn ranges_are_inclusive_and_uniform() {
        let seed = WorldSeed(7);
        let mut seen: Vec<Vec<Regime>> = vec![Vec::new(); STYLES.len()];
        for i in 0..100_000 {
            let r = regime(seed, I64Vec3::new(i, -i / 7, 3));
            if r.event == RareEvent::None {
                seen[usize::from(r.style.0)].push(r);
            }
        }
        for (def, regimes) in STYLES.iter().zip(&seen) {
            let field = |name, (lo, hi): (u32, u32), get: fn(&Regime) -> u32| {
                let values: Vec<u32> = regimes.iter().map(get).collect();
                assert_eq!(values.iter().min(), Some(&lo), "{} {name}", def.name);
                assert_eq!(values.iter().max(), Some(&hi), "{} {name}", def.name);
                let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / values.len() as f64;
                let mid = f64::from(lo + hi) / 2.0;
                let tol = 0.03 * f64::from(hi - lo);
                assert!(
                    (mean - mid).abs() < tol,
                    "{} {name} mean {mean:.1}",
                    def.name
                );
            };
            let wide = |(lo, hi): (u8, u8)| (u32::from(lo), u32::from(hi));
            field("density", wide(def.density), |r| r.density.into());
            field("verticality", wide(def.verticality), |r| {
                r.verticality.into()
            });
            field("light", wide(def.light), |r| r.light.into());
            let vis = (def.visibility_m.0.into(), def.visibility_m.1.into());
            field("visibility", vis, |r| r.visibility_m.into());
            for &p in def.palettes {
                let share =
                    regimes.iter().filter(|r| r.palette == p).count() as f64 / regimes.len() as f64;
                let expected = 1.0 / def.palettes.len() as f64;
                assert!(
                    (share - expected).abs() < 0.03,
                    "{} palette {p:?}",
                    def.name
                );
            }
        }
    }

    /// P0.6.2: each event's frequency over 10⁵ cells is within ±10% of its weight.
    #[test]
    fn event_frequencies() {
        let seed = WorldSeed(0xE7E7);
        let mut counts = [0u32; RareEvent::TABLE.len()];
        for i in 0..100_000i64 {
            let event = rare_event(seed, I64Vec3::new(i % 400 - 200, i / 400 - 125, -i % 3));
            let k = RareEvent::TABLE
                .iter()
                .position(|&(e, _)| e == event)
                .unwrap();
            counts[k] += 1;
        }
        for (&(event, weight), count) in RareEvent::TABLE.iter().zip(counts) {
            let expected = 100.0 * f64::from(weight);
            let ratio = f64::from(count) / expected;
            assert!(
                (0.9..1.1).contains(&ratio),
                "{event:?}: {count} vs {expected}"
            );
        }
    }

    /// The regime's event is the per-cell event.
    #[test]
    fn regime_event_matches_rare_event() {
        let seed = WorldSeed(3);
        for x in -50..50 {
            let cell = I64Vec3::new(x, 1, -x);
            assert_eq!(regime(seed, cell).event, rare_event(seed, cell));
        }
    }

    /// Staircase columns: frequency 4/1000 (±10%), and the L1 column is uniform over 8×8.
    #[test]
    fn rare_stair_frequency_and_placement() {
        let seed = WorldSeed(0x57A1);
        let mut present = 0u32;
        let mut per_l1 = [0u32; 64];
        for x in -500..500 {
            for z in -500..500 {
                let column = ColumnKey {
                    level: Level::L0,
                    x,
                    z,
                };
                if let Some(s) = rare_stair(seed, column) {
                    assert!(s.l1_x < 8 && s.l1_z < 8);
                    present += 1;
                    per_l1[usize::from(s.l1_z * 8 + s.l1_x)] += 1;
                }
            }
        }
        let ratio = f64::from(present) / 4_000.0;
        assert!(
            (0.9..1.1).contains(&ratio),
            "{present} staircases in 10⁶ columns"
        );
        let e = f64::from(present) / 64.0;
        let chi2: f64 = per_l1.iter().map(|&c| (f64::from(c) - e).powi(2) / e).sum();
        // 63 degrees of freedom: p = 0.001 at 103.4.
        assert!(chi2 < 103.4, "L1 column chi-square {chi2:.1}");
    }

    /// Every L0 cell in a column agrees on the staircase.
    #[test]
    fn rare_stair_is_per_column() {
        use crate::cell::CellKey;
        let seed = WorldSeed(0x57A1);
        for x in -40..40 {
            for z in -40..40 {
                let at = |y| {
                    rare_stair(
                        seed,
                        ColumnKey::of(CellKey::new(Level::L0, I64Vec3::new(x, y, z))),
                    )
                };
                assert_eq!(at(-3), at(0));
                assert_eq!(at(0), at(1_000_000));
            }
        }
    }
}
