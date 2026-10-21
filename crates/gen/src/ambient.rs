//! Ambient movers (LGT-01..04, WX-06, LIFE-01..05; `docs/spec/levels.md` §5W.46): lighthouses
//! and searchlights, light rivers, aurora sheets, steam vents, drone flocks, crawlers, cable
//! spiders, moths and the far lantern. Each is a `Mover` on the world clock like those of
//! `movers`, with its own sites (one L0 cell in six).

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::features::{Content, M};
use crate::hash::hash;
use crate::movers::{Mover, MoverKind, Part, Pose, part};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L1_M: i64 = 4096;
/// An L0 cell holds an ambient mover with chance 1 in this.
const ONE_IN: u64 = 6;

pub const KINDS: [MoverKind; 10] = [
    MoverKind::Lighthouse,
    MoverKind::Searchlight,
    MoverKind::LightRiver,
    MoverKind::Aurora,
    MoverKind::Steam,
    MoverKind::Flock,
    MoverKind::Crawler,
    MoverKind::Spider,
    MoverKind::Moths,
    MoverKind::Lantern,
];

/// How often each kind is drawn: the lantern is very rare (LIFE-05).
const WEIGHTS: [u32; 10] = [3, 2, 2, 2, 3, 2, 3, 2, 3, 1];

/// The beam of a lighthouse, and the longest thing an ambient mover has, in metres.
pub const BEAM_M: i32 = 2500;

/// The site of the ambient mover of L0 cell `l0`: its kind and its inner L1 cell.
pub fn site_of(seed: WorldSeed, l0: I64Vec3) -> Option<(MoverKind, I64Vec3)> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::AMBIENT_SITE));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    let kind = KINDS[rng.pick_weighted(&WEIGHTS).unwrap_or(0)];
    let inner = |r: &mut Rng| 1 + r.below(6) as i64;
    Some((
        kind,
        l0 * 8 + I64Vec3::new(inner(&mut rng), inner(&mut rng), inner(&mut rng)),
    ))
}

/// The cycle of a kind in milliseconds, drawn from `rng`.
fn period_of(kind: MoverKind, rng: &mut Rng) -> u64 {
    match kind {
        // Each lighthouse has its own rhythm: 32 to 96 s a turn.
        MoverKind::Lighthouse => 32_000 + 8_000 * rng.below(9),
        MoverKind::Searchlight | MoverKind::Crawler => 120_000 + 15_000 * rng.below(9),
        // Pulses 50 m apart pass a point every 1 to 2 s.
        MoverKind::LightRiver => 1_000 + 125 * rng.below(9),
        MoverKind::Aurora => 180_000 + 20_000 * rng.below(9),
        MoverKind::Steam => 20_000 + 5_000 * rng.below(9),
        MoverKind::Flock | MoverKind::Spider => 90_000 + 10_000 * rng.below(7),
        MoverKind::Moths => 16_000,
        // 700 m at a walking pace of 1.4 m/s.
        _ => 500_000,
    }
}

fn pulses() -> Vec<Part> {
    (0..20)
        .map(|k| {
            let x = k * 50;
            part(
                [x, -1, -1],
                [x + 8, 1, 1],
                MaterialId::BEAM,
                FeatureKind::CABLE,
                false,
            )
        })
        .collect()
}

/// The moving parts of `kind` in the mover's frame, and its reach.
fn parts_of(kind: MoverKind, rng: &mut Rng) -> (Vec<Part>, i64) {
    let metal = MaterialId::METAL;
    let none = FeatureKind::MISC;
    match kind {
        MoverKind::Lighthouse => (
            vec![part(
                [0, 118, -3],
                [BEAM_M, 122, 3],
                MaterialId::BEAM,
                none,
                false,
            )],
            0,
        ),
        MoverKind::Searchlight => (
            vec![part(
                [0, 9, -1],
                [600, 11, 1],
                MaterialId::BEAM,
                none,
                false,
            )],
            0,
        ),
        MoverKind::LightRiver => (pulses(), 50),
        MoverKind::Aurora => {
            // Twenty-four curtains of 25 m with ragged hems, hung in a row 600 m wide.
            let strips = (0..24)
                .map(|i| {
                    let h = 80 + rng.below(80) as i32;
                    let x = i * 25 - 300;
                    part([x, 0, -1], [x + 24, h, 0], MaterialId::AURORA, none, false)
                })
                .collect();
            (strips, 0)
        }
        MoverKind::Steam => (
            (0..5)
                .map(|k| {
                    let (w, y) = (2 + k * 2, k * 8);
                    part([-w, y, -w], [w, y + 9, w], MaterialId::VAPOUR, none, false)
                })
                .collect(),
            STEAM_HIDDEN_M,
        ),
        MoverKind::Flock => (
            (0..24)
                .map(|_| {
                    // A ring of 60 to 100 m, in integers.
                    let (mut x, mut z) = (rng.below(201) as i32 - 100, rng.below(201) as i32 - 100);
                    if x.abs() + z.abs() < 60 {
                        x += if x >= 0 { 60 } else { -60 };
                        z += if z >= 0 { 30 } else { -30 };
                    }
                    let y = rng.below(50) as i32 - 25;
                    part(
                        [x, y, z],
                        [x + 2, y + 1, z + 1],
                        MaterialId::WARM,
                        none,
                        false,
                    )
                })
                .collect(),
            0,
        ),
        MoverKind::Crawler => (
            vec![
                part([-1, 10, -1], [1, 11, 1], metal, none, false),
                part([-1, 11, -1], [1, 12, 1], MaterialId::EMISSIVE, none, false),
            ],
            70,
        ),
        MoverKind::Spider => (
            vec![
                part([-1, 0, -1], [1, 1, 1], MaterialId::WARM, none, false),
                part([-2, 0, 0], [2, 1, 1], metal, none, false),
            ],
            14,
        ),
        MoverKind::Moths => (
            (0..8)
                .map(|k| {
                    let (x, z) = [(2, 0), (0, 2), (-2, 0), (0, -2)][k % 4];
                    let y = 5 + (k as i32 % 3);
                    part(
                        [x, y, z],
                        [x + 1, y + 1, z + 1],
                        MaterialId::WARM,
                        none,
                        false,
                    )
                })
                .collect(),
            0,
        ),
        _ => (
            vec![
                part([0, 0, 0], [1, 2, 1], metal, none, false),
                part([0, 2, 0], [1, 3, 1], MaterialId::EMISSIVE, none, false),
            ],
            700,
        ),
    }
}

/// Where a hidden steam plume waits, metres below the vent.
pub const STEAM_HIDDEN_M: i64 = 10_000;

/// The movers of L1 cell `l1` (at most one).
pub fn movers_in_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<Mover> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let Some((kind, _)) = site_of(seed, l0).filter(|s| s.1 == l1) else {
        return Vec::new();
    };
    // A cell holds one mover: the main one has it.
    if crate::movers::site_of(seed, l0).is_some_and(|s| s.1 == l1) {
        return Vec::new();
    }
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::AMBIENT_SITE));
    let c = 2048i64;
    let period_ms = period_of(kind, &mut rng);
    let phase_ms = rng.below(period_ms);
    let key = rng.next_u64();
    let (parts, reach) = parts_of(kind, &mut rng);
    vec![Mover {
        kind,
        origin: [0, 1, 2].map(|i| l1[i] * L1_M + c),
        parts,
        timed: Vec::new(),
        dock: false,
        period_ms,
        phase_ms,
        reach,
        key,
    }]
}

/// The pose of ambient mover `m` at `u` 1024ths of its cycle.
pub fn pose(m: &Mover, u: i64) -> Pose {
    let tri = if u < 512 { u * 2 } else { 2048 - u * 2 };
    let f = |k: i64, scale: i64| (k * scale) as f32 / 1024.0;
    match m.kind {
        // Sweeps through 24 of 64 turns and back.
        MoverKind::Searchlight => Pose {
            offset: [0.0; 3],
            yaw: (tri * 24 / 1024) as u8,
        },
        // Pulses slide one spacing and start again.
        MoverKind::LightRiver => Pose {
            offset: [f(u, 50), 0.0, 0.0],
            yaw: 0,
        },
        // The curtains sway a fifth of a quarter turn.
        MoverKind::Aurora => Pose {
            offset: [0.0; 3],
            yaw: (tri * 3 / 1024) as u8,
        },
        MoverKind::Steam => {
            let k = ramp(u, 0, 100, 250, 400);
            let y = if k == 0 {
                -(STEAM_HIDDEN_M as f32)
            } else {
                -((1024 - k) * 30) as f32 / 1024.0
            };
            Pose {
                offset: [0.0, y, 0.0],
                yaw: 0,
            }
        }
        MoverKind::Lighthouse | MoverKind::Flock | MoverKind::Moths => Pose {
            offset: [0.0; 3],
            yaw: (u * 64 / 1024) as u8,
        },
        MoverKind::Crawler => Pose {
            offset: [f(tri, 70), 0.0, 0.0],
            yaw: 0,
        },
        MoverKind::Spider => Pose {
            offset: [f(tri, 14), f(tri, 6), 0.0],
            yaw: 0,
        },
        // Always away: down the catwalk and back at the start.
        _ => Pose {
            offset: [f(u, 700), 0.0, 0.0],
            yaw: 0,
        },
    }
}

fn ramp(u: i64, a: i64, b: i64, c: i64, d: i64) -> i64 {
    let smooth = |t: i64| t * t * (3 * 1024 - 2 * t) / (1024 * 1024);
    if u < a || u >= d {
        0
    } else if u < b {
        smooth((u - a) * 1024 / (b - a).max(1))
    } else if u < c {
        1024
    } else {
        1024 - smooth((u - c) * 1024 / (d - c).max(1))
    }
}

/// The fixed parts of the ambient mover of `l1`: a tower, a cable, posts, a lamp.
pub fn content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let Some(m) = movers_in_cell(seed, l1).into_iter().next() else {
        return Content::default();
    };
    let c = 2048i32;
    let mut b: Vec<Primitive> = Vec::new();
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    let mut add = |lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind, flags| {
        b.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    };
    let solid = PrimFlags::NONE;
    let far = PrimFlags::FAR_VISIBLE;
    let glow = PrimFlags::NO_COLLIDE;
    let metal = MaterialId::METAL;
    let conc = MaterialId::CONCRETE;
    let keep = match m.kind {
        // A tower of 10 m and 118 m, a landing, and the lamp room under the beam.
        MoverKind::Lighthouse => {
            add(
                [c - 7, c - 2, c - 7],
                [c + 7, c, c + 7],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c - 5, c, c - 5],
                [c + 5, c + 116, c + 5],
                metal,
                FeatureKind::TETHER,
                far | solid,
            );
            add(
                [c - 7, c + 116, c - 7],
                [c + 7, c + 119, c + 7],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c - 3, c + 119, c - 3],
                [c + 3, c + 123, c + 3],
                MaterialId::BEAM,
                FeatureKind::MISC,
                far | glow,
            );
            ([c - 10, c - 4, c - 10], [c + 10, c + 126, c + 10])
        }
        MoverKind::Searchlight => {
            add(
                [c - 6, c - 2, c - 6],
                [c + 6, c, c + 6],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c - 2, c, c - 2],
                [c + 2, c + 8, c + 2],
                metal,
                FeatureKind::TETHER,
                solid,
            );
            add(
                [c - 3, c + 8, c - 3],
                [c + 3, c + 12, c + 3],
                metal,
                FeatureKind::MASS,
                solid,
            );
            ([c - 8, c - 4, c - 8], [c + 8, c + 14, c + 8])
        }
        // The cable of 1 000 m on posts every 100 m.
        MoverKind::LightRiver => {
            add(
                [c, c - 1, c - 1],
                [c + 1000, c + 1, c + 1],
                metal,
                FeatureKind::CABLE,
                far | solid,
            );
            for k in 0..=10 {
                add(
                    [c + k * 100 - 1, c - 12, c - 1],
                    [c + k * 100 + 1, c - 1, c + 1],
                    metal,
                    FeatureKind::TETHER,
                    solid,
                );
            }
            ([c - 4, c - 14, c - 4], [c + 1004, c + 4, c + 4])
        }
        // Two tethers hold the curtains' rail.
        MoverKind::Aurora => {
            add(
                [c - 300, c + 160, c - 1],
                [c + 300, c + 161, c + 1],
                metal,
                FeatureKind::CABLE,
                far | glow,
            );
            ([c - 304, c - 4, c - 6], [c + 304, c + 170, c + 6])
        }
        // A grating over a vent pipe.
        MoverKind::Steam => {
            add(
                [c - 4, c - 1, c - 4],
                [c + 4, c, c + 4],
                MaterialId::GRATING,
                FeatureKind::DECK,
                solid,
            );
            add(
                [c - 5, c - 12, c - 5],
                [c + 5, c - 1, c + 5],
                MaterialId::PIPE,
                FeatureKind::PIPE,
                solid,
            );
            ([c - 8, c - 14, c - 8], [c + 8, c + 48, c + 8])
        }
        MoverKind::Flock => {
            add(
                [c - 1, c - 1, c - 1],
                [c + 1, c + 1, c + 1],
                metal,
                FeatureKind::MISC,
                far | glow,
            );
            ([c - 2, c - 2, c - 2], [c + 2, c + 2, c + 2])
        }
        // A pipe of 80 m on posts, at 10 m.
        MoverKind::Crawler => {
            add(
                [c - 5, c + 9, c - 1],
                [c + 75, c + 10, c + 1],
                MaterialId::PIPE,
                FeatureKind::PIPE,
                solid,
            );
            for x in [0, 40, 70] {
                add(
                    [c + x - 1, c, c - 1],
                    [c + x + 1, c + 9, c + 1],
                    metal,
                    FeatureKind::TETHER,
                    solid,
                );
            }
            ([c - 6, c - 2, c - 4], [c + 78, c + 16, c + 4])
        }
        // Two posts and a net of cables between them.
        MoverKind::Spider => {
            for x in [-3, 17] {
                add(
                    [c + x - 1, c - 4, c - 1],
                    [c + x + 1, c + 20, c + 1],
                    metal,
                    FeatureKind::TETHER,
                    solid,
                );
            }
            for y in [0, 4, 8, 12, 16] {
                add(
                    [c - 3, c + y, c],
                    [c + 17, c + y + 1, c + 1],
                    metal,
                    FeatureKind::CABLE,
                    glow,
                );
            }
            for x in [0, 4, 8, 12] {
                add(
                    [c + x, c, c],
                    [c + x + 1, c + 17, c + 1],
                    metal,
                    FeatureKind::CABLE,
                    glow,
                );
            }
            ([c - 5, c - 6, c - 4], [c + 19, c + 22, c + 4])
        }
        MoverKind::Moths => {
            add(
                [c - 1, c, c - 1],
                [c + 1, c + 5, c + 1],
                metal,
                FeatureKind::TETHER,
                solid,
            );
            add(
                [c - 1, c + 5, c - 1],
                [c + 1, c + 7, c + 1],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                glow,
            );
            ([c - 5, c - 2, c - 5], [c + 5, c + 10, c + 5])
        }
        // A catwalk of 720 m.
        _ => {
            add(
                [c - 10, c - 1, c - 1],
                [c + 710, c, c + 1],
                MaterialId::GRATING,
                FeatureKind::WALKWAY,
                far | solid,
            );
            for x in (0..=700).step_by(100) {
                add(
                    [c + x - 1, c - 20, c - 1],
                    [c + x + 1, c - 1, c + 1],
                    metal,
                    FeatureKind::TETHER,
                    solid,
                );
            }
            ([c - 12, c - 22, c - 4], [c + 712, c + 6, c + 4])
        }
    };
    Content {
        prims: b,
        keep_out: vec![QAabb::new(q(keep.0), q(keep.1))],
    }
}

/// An L1 cell holds a lamp sun over a perforated crust with chance 1 in this (LGT-06, LGT-08).
const SUN_ONE_IN: u64 = 24;

/// The lamp sun of L1 cell `l1`: a small sun hung 700 m up over a crust plate of 400 m with
/// holes in it, and the shafts of light that fall through the holes to the floor (god rays).
/// Fixed in the world; its light is geometry (`VAPOUR` beams), not a light source.
pub fn lamp_sun_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::LAMP_SUN));
    if rng.below(SUN_ONE_IN) != 0 {
        return Content::default();
    }
    let c = 2048i32;
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    let mut b: Vec<Primitive> = Vec::new();
    let mut add = |lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind, flags| {
        b.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    };
    let far = PrimFlags::FAR_VISIBLE;
    let glow = PrimFlags::NO_COLLIDE;
    // The sun: a disc of 60 m and a ring of lamps.
    add(
        [c - 30, c + 700, c - 30],
        [c + 30, c + 708, c + 30],
        MaterialId::BEAM,
        FeatureKind::MISC,
        far | glow,
    );
    // The crust: sixteen squares of 100 m, a hole where the draw says so (at least two).
    let mut holes = 0;
    let mut mask = rng.next_u64() & 0xFFFF;
    while mask.count_ones() < 2 {
        mask |= 1 << rng.below(16);
    }
    for k in 0..16 {
        let (ix, iz) = (k % 4, k / 4);
        let (x, z) = (c - 200 + ix * 100, c - 200 + iz * 100);
        if mask >> k & 1 == 1 {
            holes += 1;
            // The shaft of light: 40 m wide, from the hole to the floor.
            add(
                [x + 30, c - 2, z + 30],
                [x + 70, c + 640, z + 70],
                MaterialId::VAPOUR,
                FeatureKind::MISC,
                far | glow,
            );
        } else {
            add(
                [x, c + 636, z],
                [x + 100, c + 644, z + 100],
                MaterialId::CONCRETE,
                FeatureKind::DECK,
                far | PrimFlags::NONE,
            );
        }
    }
    debug_assert!(holes >= 2);
    Content {
        prims: b,
        keep_out: vec![QAabb::new(
            q([c - 204, c - 4, c - 204]),
            q([c + 204, c + 712, c + 204]),
        )],
    }
}

/// An L1 cell holds a cistern with a still pool with chance 1 in this (WX-11).
const POOL_ONE_IN: u64 = 30;

/// WX-11: a cistern of 60 m with a pool of coolant 2 m deep, flat and dark. *Deviation:* the
/// planar reflection that makes it a mirror is not drawn (PW.10.20b); the surface is the
/// glossy metal of the table.
pub fn pool_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    if Rng::new(hash(seed.0, Level::L1, l1, salt::POOL)).below(POOL_ONE_IN) != 0 {
        return Content::default();
    }
    let c = 2048i32;
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    let mut b: Vec<Primitive> = Vec::new();
    let mut add = |lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind, flags| {
        b.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    };
    let conc = MaterialId::CONCRETE;
    let solid = PrimFlags::NONE;
    add(
        [c - 32, c - 2, c - 32],
        [c + 32, c, c + 32],
        conc,
        FeatureKind::DECK,
        solid,
    );
    for (lo, hi) in [
        ([c - 32, c, c - 32], [c + 32, c + 3, c - 30]),
        ([c - 32, c, c + 30], [c + 32, c + 3, c + 32]),
        ([c - 32, c, c - 30], [c - 30, c + 3, c + 30]),
        ([c + 30, c, c - 30], [c + 32, c + 3, c + 30]),
    ] {
        add(lo, hi, conc, FeatureKind::MASS, solid);
    }
    // The coolant: a flat plate of metal at 2 m that nobody stands on.
    add(
        [c - 30, c + 1, c - 30],
        [c + 30, c + 2, c + 30],
        MaterialId::METAL,
        FeatureKind::MISC,
        PrimFlags::NO_COLLIDE,
    );
    Content {
        prims: b,
        keep_out: vec![QAabb::new(
            q([c - 34, c - 4, c - 34]),
            q([c + 34, c + 6, c + 34]),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: MoverKind) -> (I64Vec3, Mover) {
        for x in -40..40 {
            for z in -40..40 {
                for y in -1..2 {
                    if let Some((k, l1)) = site_of(SEED, I64Vec3::new(x, y, z))
                        && k == kind
                    {
                        return (l1, movers_in_cell(SEED, l1).remove(0));
                    }
                }
            }
        }
        panic!("{kind:?} not found");
    }

    /// LGT-06, LGT-08: lamp suns hang over perforated crusts in about one cell in 24, and shafts
    /// of light stand under the holes.
    #[test]
    fn lamp_suns_hang_over_perforated_crusts() {
        let mut n = 0;
        for x in -20..20 {
            for z in -20..20 {
                let c = lamp_sun_content(SEED, I64Vec3::new(x, 1, z));
                if c.prims.is_empty() {
                    continue;
                }
                n += 1;
                assert!(c.prims.iter().any(|p| p.material == MaterialId::BEAM));
                let shafts = c
                    .prims
                    .iter()
                    .filter(|p| p.material == MaterialId::VAPOUR)
                    .count();
                let crust = c
                    .prims
                    .iter()
                    .filter(|p| p.material == MaterialId::CONCRETE)
                    .count();
                assert!(shafts >= 2 && shafts + crust == 16, "{shafts} + {crust}");
            }
        }
        assert!((20..110).contains(&n), "{n} of 1600");
    }

    #[test]
    fn cisterns_hold_a_flat_pool_in_about_one_cell_in_thirty() {
        let mut n = 0;
        for x in -30..30 {
            for z in -30..30 {
                let c = pool_content(SEED, I64Vec3::new(x, 1, z));
                if !c.prims.is_empty() {
                    n += 1;
                    assert_eq!(c.prims.len(), 6);
                }
            }
        }
        assert!((50..200).contains(&n), "{n} of 3600");
    }

    #[test]
    fn every_ambient_kind_occurs_with_parts_and_supports() {
        for kind in KINDS {
            let (l1, m) = find(kind);
            assert_eq!(m.kind, kind);
            assert!(!m.parts.is_empty());
            assert!(
                m.parts.iter().all(|p| !p.solid),
                "{kind:?}: ambient parts are passable"
            );
            assert!(!content(SEED, l1).prims.is_empty(), "{kind:?}");
        }
    }

    /// Each pose is periodic and a pure function of the world clock.
    #[test]
    fn ambient_poses_are_periodic_and_pure() {
        for kind in KINDS {
            let (_, m) = find(kind);
            for t in [0u64, 1, 59_999, 123_456, 7_654_321, 1 << 40] {
                assert_eq!(m.pose(t), m.pose(t + m.period_ms), "{kind:?} at {t}");
            }
        }
    }

    #[test]
    fn lighthouses_have_their_own_rhythm() {
        let mut periods = std::collections::HashSet::new();
        for x in -40..40 {
            for z in -40..40 {
                if let Some((MoverKind::Lighthouse, l1)) = site_of(SEED, I64Vec3::new(x, 0, z)) {
                    periods.insert(movers_in_cell(SEED, l1)[0].period_ms);
                }
            }
        }
        assert!(periods.len() >= 4, "{periods:?}");
        assert!(periods.iter().all(|p| (32_000..=96_000).contains(p)));
    }

    /// LIFE-05: the lantern is far rarer than the lighthouse, and goes one way.
    #[test]
    fn the_lantern_is_rare_and_always_goes_away() {
        let (mut lanterns, mut lighthouses) = (0, 0);
        for x in -40..40 {
            for z in -40..40 {
                match site_of(SEED, I64Vec3::new(x, 0, z)) {
                    Some((MoverKind::Lantern, _)) => lanterns += 1,
                    Some((MoverKind::Lighthouse, _)) => lighthouses += 1,
                    _ => {}
                }
            }
        }
        assert!(
            lanterns > 0 && lanterns * 2 < lighthouses,
            "{lanterns} / {lighthouses}"
        );
        let (_, m) = find(MoverKind::Lantern);
        let xs: Vec<f32> = (0..64)
            .map(|k| m.pose(k * m.period_ms / 64).offset[0])
            .collect();
        // It only goes one way; the single drop is the catwalk's far end, where the cycle starts again.
        assert!(
            xs.windows(2).filter(|w| w[1] < w[0]).count() <= 1,
            "the lantern turns back"
        );
    }

    /// WX-06: a vent is hidden most of the cycle and shows a plume for a part of it.
    #[test]
    fn a_vent_puffs_on_a_cycle() {
        let (_, m) = find(MoverKind::Steam);
        let up = (0..64)
            .filter(|k| m.pose(k * m.period_ms / 64).offset[1] > -100.0)
            .count();
        assert!((10..40).contains(&up), "{up} of 64");
    }
}
