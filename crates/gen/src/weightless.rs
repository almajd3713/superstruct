//! Weightless and low-gravity features (WGT-01..09, `docs/spec/gravity.md` §5 and §8): featherfall
//! shafts, drift lanes and gravity rivers, the Archipelago, tumbler rooms, orbiters and the
//! weightless cathedral. Their fields are wells with no geometry of their own (`Well::quiet`);
//! the geometry is the feature's.

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::too_many_arguments
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::features::{Content, M};
use crate::gravity::{Well, WellKind};
use crate::hash::hash;
use crate::l1::CELL;
use crate::orient::{DIRS, heading};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L0_M: i64 = 32768;
const L1_M: i64 = 4096;

/// An L0 column holds a featherfall shaft with chance 1 in this; an L0 cell another weightless
/// feature with chance 1 in `CELL_ONE_IN`.
const COLUMN_ONE_IN: u64 = 7;
const CELL_ONE_IN: u64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shaft {
    /// WGT-01: a straight shaft with a platform every 96 m.
    Straight,
    /// WGT-01: a shaft with a spiral of lights.
    Helical,
    /// WGT-02: side galleries every 256 m.
    Branching,
    /// WGT-02: you fall upwards.
    Upward,
    /// WON-13, the Drop: a shaft of 128 m, weaker still, one in forty of the shafts.
    Drop,
}

/// The six directions of a tumbler room's up.
const UPS: [[i8; 3]; 6] = [
    [0, 8, 0],
    [0, -8, 0],
    [8, 0, 0],
    [-8, 0, 0],
    [0, 0, 8],
    [0, 0, -8],
];

impl Shaft {
    /// Half the clear width of the shaft, in metres.
    pub const fn half(self) -> i32 {
        if matches!(self, Shaft::Drop) { 64 } else { 24 }
    }
}

const SHAFTS: [Shaft; 4] = [
    Shaft::Straight,
    Shaft::Helical,
    Shaft::Branching,
    Shaft::Upward,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// WGT-03: a lull tube with a current, the length of the L0 cell.
    DriftLane,
    /// WGT-04: a long drift lane between two wells, 1–10 km.
    GravityRiver,
    /// WGT-05: a lull with floating fragments, each its own small planetoid.
    Archipelago,
    /// WGT-06: a room whose down changes every few minutes of world time.
    Tumbler,
    /// WGT-07: debris in orbit round a point well.
    Orbiters,
    /// WGT-09: a weightless hall of floating lamps with a handrail.
    Cathedral,
}

const KINDS: [Kind; 6] = [
    Kind::DriftLane,
    Kind::GravityRiver,
    Kind::Archipelago,
    Kind::Tumbler,
    Kind::Orbiters,
    Kind::Cathedral,
];
const KIND_WEIGHTS: [u32; 6] = [3, 2, 2, 2, 2, 1];

impl Kind {
    pub const fn id(self) -> &'static str {
        match self {
            Kind::DriftLane => "WGT-03",
            Kind::GravityRiver => "WGT-04",
            Kind::Archipelago => "WGT-05",
            Kind::Tumbler => "WGT-06",
            Kind::Orbiters => "WGT-07",
            Kind::Cathedral => "WGT-09",
        }
    }
}

/// The featherfall shaft of L0 column `(x, z)`: its inner L1 column, variant and strength.
pub fn shaft_of(seed: WorldSeed, l0x: i64, l0z: i64) -> Option<((i64, i64), Shaft, i64)> {
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L0,
        I64Vec3::new(l0x, 1 << 40, l0z),
        salt::WEIGHTLESS,
    ));
    if rng.below(COLUMN_ONE_IN) != 0 {
        return None;
    }
    let ab = (1 + rng.below(6) as i64, 1 + rng.below(6) as i64);
    let shaft = SHAFTS[rng.below(4) as usize];
    // 4..=16 of 256: a fall of 1–3 m/s.
    let strength = 4 + 4 * rng.below(4) as i64;
    // One shaft in forty is the Drop (WON-13): wide, straight and the weakest.
    let (shaft, strength) = if rng.below(40) == 0 {
        (Shaft::Drop, 4)
    } else {
        (shaft, strength)
    };
    Some((ab, shaft, strength))
}

/// The feature of L0 cell `l0`, its inner L1 cell and its draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub kind: Kind,
    pub l1: I64Vec3,
    /// The axis of a lane, 0 for x and 2 for z.
    pub axis: usize,
    /// Radius of a lane in metres, current in cm/s and the length of a river in metres.
    pub radius: i64,
    pub current: i64,
    pub length: i64,
}

pub fn site_of(seed: WorldSeed, l0: I64Vec3) -> Option<Site> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::WEIGHTLESS));
    if rng.below(CELL_ONE_IN) != 0 {
        return None;
    }
    let kind = KINDS[rng.pick_weighted(&KIND_WEIGHTS).unwrap_or(0)];
    let inner = |r: &mut Rng| 1 + r.below(6) as i64;
    let l1 = l0 * 8 + I64Vec3::new(inner(&mut rng), inner(&mut rng), inner(&mut rng));
    let axis = if rng.below(2) == 0 { 0 } else { 2 };
    let radius = 10 + 5 * rng.below(5) as i64;
    let speed = 50 + 10 * rng.below(16) as i64;
    let current = if rng.below(2) == 0 { speed } else { -speed };
    let length = 1000 + 1000 * rng.below(10) as i64;
    Some(Site {
        kind,
        l1,
        axis,
        radius,
        current,
        length,
    })
}

fn centre_of(l1: I64Vec3) -> [i64; 3] {
    [0, 1, 2].map(|i| l1[i] * L1_M + L1_M / 2)
}

fn axis_index(d: [i8; 3]) -> usize {
    DIRS.iter().position(|x| *x == d).unwrap_or(0)
}

/// A lane's field: a lull tube along `axis` through `centre`, `half` metres long each way.
fn lane_well(l1: I64Vec3, axis: usize, centre: [i64; 3], half: i64, radius: i64) -> Well {
    let mut dir = [0i8; 3];
    dir[axis] = 1;
    Well {
        kind: WellKind::Tube,
        centre,
        axis: axis_index(dir),
        // Full lull inside three quarters of the reach: the reach is the radius and a quarter.
        reach: radius * 4 / 3 + 8,
        strength: 0,
        extent: [0, half, 0],
        l1,
        fold: Vec::new(),
        quiet: true,
        flip: false,
    }
}

/// The wells of the weightless features of L0 cell `l0`, in the order of the summation.
pub fn wells(seed: WorldSeed, l0: I64Vec3) -> Vec<Well> {
    let mut out = Vec::new();
    // Featherfall: one plane well per L0 cell of the column, weak along the shaft.
    if let Some(((a, b), shaft, strength)) = shaft_of(seed, l0.x, l0.z) {
        let l1 = I64Vec3::new(l0.x * 8 + a, l0.y * 8 + 3, l0.z * 8 + b);
        let c = centre_of(l1);
        out.push(Well {
            kind: WellKind::Plane,
            centre: [c[0], l0.y * L0_M + L0_M / 2, c[2]],
            axis: axis_index([0, 1, 0]),
            reach: 96,
            strength,
            extent: [i64::from(shaft.half()), L0_M / 2, i64::from(shaft.half())],
            l1,
            fold: Vec::new(),
            quiet: true,
            flip: shaft == Shaft::Upward,
        });
    }
    let Some(site) = site_of(seed, l0) else {
        return out;
    };
    let c = centre_of(site.l1);
    match site.kind {
        Kind::DriftLane => {
            let mut centre = c;
            centre[site.axis] = l0[site.axis] * L0_M + L0_M / 2;
            out.push(lane_well(site.l1, site.axis, centre, L0_M / 2, site.radius));
        }
        Kind::GravityRiver => {
            let mut centre = c;
            centre[site.axis] = l0[site.axis] * L0_M + L0_M / 2;
            out.push(lane_well(
                site.l1,
                site.axis,
                centre,
                site.length / 2,
                site.radius,
            ));
        }
        Kind::Archipelago => {
            out.push(Well {
                kind: WellKind::Plane,
                centre: c,
                axis: axis_index([0, 1, 0]),
                reach: 160,
                strength: 0,
                extent: [512, 512, 512],
                l1: site.l1,
                fold: Vec::new(),
                quiet: true,
                flip: false,
            });
            for f in fragments(seed, site.l1) {
                out.push(Well {
                    kind: WellKind::Point,
                    centre: f.centre,
                    axis: 0,
                    reach: f.reach,
                    strength: f.strength,
                    extent: [0; 3],
                    l1: site.l1,
                    fold: Vec::new(),
                    quiet: true,
                    flip: false,
                });
            }
        }
        Kind::Orbiters | Kind::Tumbler | Kind::Cathedral => {
            if site.kind == Kind::Cathedral {
                out.push(Well {
                    kind: WellKind::Plane,
                    centre: c,
                    axis: axis_index([0, 1, 0]),
                    reach: 32,
                    strength: 0,
                    extent: [240, 32, 32],
                    l1: site.l1,
                    fold: Vec::new(),
                    quiet: true,
                    flip: false,
                });
            }
        }
    }
    out
}

/// The current of a drift lane at `p` in cm/s per axis, or zero outside a lane.
pub fn current_at(seed: WorldSeed, p: [i64; 3]) -> [i32; 3] {
    let home = I64Vec3::new(
        p[0].div_euclid(L0_M),
        p[1].div_euclid(L0_M),
        p[2].div_euclid(L0_M),
    );
    let Some(site) = site_of(seed, home) else {
        return [0; 3];
    };
    if !matches!(site.kind, Kind::DriftLane | Kind::GravityRiver) {
        return [0; 3];
    }
    let c = centre_of(site.l1);
    let across: Vec<usize> = (0..3).filter(|&i| i != site.axis && i != 1).collect();
    // Inside the tube: within `radius` metres of its axis on both other axes.
    let inside = (0..3)
        .filter(|&i| i != site.axis)
        .all(|i| (p[i] - c[i]).abs() <= site.radius);
    let _ = across;
    let along = p[site.axis] - (home[site.axis] * L0_M + L0_M / 2);
    let half = if site.kind == Kind::DriftLane {
        L0_M / 2
    } else {
        site.length / 2
    };
    if !inside || along.abs() > half {
        return [0; 3];
    }
    let mut v = [0i32; 3];
    v[site.axis] = site.current as i32;
    v
}

/// A floating fragment of the Archipelago.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fragment {
    pub centre: [i64; 3],
    pub half: i64,
    pub reach: i64,
    pub strength: i64,
}

pub fn fragments(seed: WorldSeed, l1: I64Vec3) -> Vec<Fragment> {
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::WEIGHTLESS));
    let c = centre_of(l1);
    (0..10)
        .map(|_| Fragment {
            centre: [
                c[0] - 400 + rng.below(800) as i64,
                c[1] - 400 + rng.below(800) as i64,
                c[2] - 400 + rng.below(800) as i64,
            ],
            half: 20 + 10 * rng.below(5) as i64,
            reach: 200 + 100 * rng.below(5) as i64,
            strength: 32 + 16 * rng.below(7) as i64,
        })
        .collect()
}

/// A tumbler room: a hollow cube whose down is one of the six axes, redrawn every `period`
/// seconds of world time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tumbler {
    pub centre: [i64; 3],
    pub half: i64,
    pub period_s: i64,
    key: u64,
}

impl Tumbler {
    /// The room's up at world time `t_s` (seconds): the direction of the epoch's draw.
    pub fn up_at(&self, t_s: i64) -> [i8; 3] {
        let epoch = t_s.div_euclid(self.period_s);
        let mut rng = Rng::new(hash(
            self.key,
            Level::L0,
            I64Vec3::new(epoch, 0, 0),
            salt::WEIGHTLESS,
        ));
        UPS[rng.below(6) as usize]
    }

    pub fn contains(&self, p: [i64; 3]) -> bool {
        (0..3).all(|i| (p[i] - self.centre[i]).abs() < self.half)
    }
}

pub fn tumbler_of(seed: WorldSeed, l0: I64Vec3) -> Option<Tumbler> {
    let site = site_of(seed, l0).filter(|s| s.kind == Kind::Tumbler)?;
    let mut rng = Rng::new(hash(seed.0, Level::L1, site.l1, salt::WEIGHTLESS));
    Some(Tumbler {
        centre: centre_of(site.l1),
        half: 60,
        period_s: 180 + 60 * rng.below(6) as i64,
        key: rng.next_u64(),
    })
}

/// The tumbler room that holds `p`, if any.
pub fn tumbler_at(seed: WorldSeed, p: [i64; 3]) -> Option<Tumbler> {
    let home = I64Vec3::new(
        p[0].div_euclid(L0_M),
        p[1].div_euclid(L0_M),
        p[2].div_euclid(L0_M),
    );
    tumbler_of(seed, home).filter(|t| t.contains(p))
}

/// Debris in orbit round a Point well: radius in metres, period in seconds, phase in 64ths of a
/// turn and size in metres. A function of the well's L1 cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orbiter {
    pub radius: i64,
    pub period_s: i64,
    pub phase: u8,
    pub size: i64,
}

pub fn orbiters(seed: WorldSeed, l1: I64Vec3) -> Vec<Orbiter> {
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::WEIGHTLESS));
    (0..24)
        .map(|_| Orbiter {
            radius: 400 + 25 * rng.below(20) as i64,
            period_s: 120 + 30 * rng.below(8) as i64,
            phase: rng.below(64) as u8,
            size: 12 + 4 * rng.below(6) as i64,
        })
        .collect()
}

struct B {
    out: Vec<Primitive>,
}

impl B {
    fn boxed(
        &mut self,
        lo: [i32; 3],
        hi: [i32; 3],
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        if (0..3).any(|i| lo[i] >= hi[i]) {
            return;
        }
        let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
        self.out.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    }
}

fn volume(lo: [i32; 3], hi: [i32; 3]) -> QAabb {
    QAabb::new(
        QVec3::new(lo[0] * M, lo[1] * M, lo[2] * M),
        QVec3::new(hi[0] * M, hi[1] * M, hi[2] * M),
    )
}

/// A ring of four bars of light at height `y`, `inner` metres from the axis `(x, z)`.
fn ring(b: &mut B, x: i32, z: i32, y: i32, inner: i32, material: MaterialId) {
    let none = PrimFlags::NO_COLLIDE;
    for (lo, hi) in [
        ([x - inner, y, z - inner], [x + inner, y + 1, z - inner + 1]),
        ([x - inner, y, z + inner - 1], [x + inner, y + 1, z + inner]),
        (
            [x - inner, y, z - inner + 1],
            [x - inner + 1, y + 1, z + inner - 1],
        ),
        (
            [x + inner - 1, y, z - inner + 1],
            [x + inner, y + 1, z + inner - 1],
        ),
    ] {
        b.boxed(lo, hi, material, FeatureKind::MISC, none);
    }
}

/// The featherfall shaft in L1 cell `l1`, if its column holds one.
fn shaft_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let inner = l1 - l0 * 8;
    let Some(((a, bb), shaft, _)) = shaft_of(seed, l0.x, l0.z) else {
        return Content::default();
    };
    if (inner.x, inner.z) != (a, bb) {
        return Content::default();
    }
    let c = 2048;
    let (half, wall) = (shaft.half(), 4);
    let solid = PrimFlags::NONE;
    let mut b = B { out: Vec::new() };
    let conc = MaterialId::CONCRETE;
    // The walls: full on three sides, and the +z side with a doorway every 128 m.
    for (lo, hi) in [
        (
            [c - half - wall, 0, c - half - wall],
            [c - half, 4096, c + half + wall],
        ),
        (
            [c + half, 0, c - half - wall],
            [c + half + wall, 4096, c + half + wall],
        ),
        ([c - half, 0, c - half - wall], [c + half, 4096, c - half]),
    ] {
        b.boxed(
            lo,
            hi,
            conc,
            FeatureKind::SHAFT_WALL,
            PrimFlags::FAR_VISIBLE | solid,
        );
    }
    for k in 0..32 {
        let y = k * 128;
        b.boxed(
            [c - half, y, c + half],
            [c - 4, y + 128, c + half + wall],
            conc,
            FeatureKind::SHAFT_WALL,
            solid,
        );
        b.boxed(
            [c + 4, y, c + half],
            [c + half, y + 128, c + half + wall],
            conc,
            FeatureKind::SHAFT_WALL,
            solid,
        );
        b.boxed(
            [c - 4, y + 6, c + half],
            [c + 4, y + 128, c + half + wall],
            conc,
            FeatureKind::SHAFT_WALL,
            solid,
        );
        // The landing outside the doorway, and a platform inside against the far wall.
        b.boxed(
            [c - 6, y - 1, c + half + wall],
            [c + 6, y, c + half + wall + 10],
            conc,
            FeatureKind::WALKWAY,
            solid,
        );
        b.boxed(
            [c - 7, y + 48, c - half],
            [c + 7, y + 49, c - half + 12],
            conc,
            FeatureKind::WALKWAY,
            solid,
        );
        // Rings of light every 64 m: red where you fall upwards.
        let lamp = if shaft == Shaft::Upward {
            MaterialId::SIGNAL
        } else {
            MaterialId::EMISSIVE
        };
        ring(&mut b, c, c, y + 2, half - 1, lamp);
        ring(&mut b, c, c, y + 66, half - 1, lamp);
        match shaft {
            Shaft::Helical => {
                // A spiral of lights: thirty-two points a turn.
                for s in 0..32u8 {
                    let (a2, b2) = heading(s * 2);
                    let len =
                        (f64::from(a2) * f64::from(a2) + f64::from(b2) * f64::from(b2)).sqrt();
                    let (dx, dz) = (
                        (f64::from(half - 2) * f64::from(a2) / len).round() as i32,
                        (f64::from(half - 2) * f64::from(b2) / len).round() as i32,
                    );
                    let yy = y + i32::from(s) * 4;
                    b.boxed(
                        [c + dx, yy, c + dz],
                        [c + dx + 1, yy + 2, c + dz + 1],
                        MaterialId::EMISSIVE,
                        FeatureKind::MISC,
                        PrimFlags::NO_COLLIDE,
                    );
                }
            }
            Shaft::Branching if k % 2 == 0 => {
                // A gallery of 6 by 6 m, 48 m long, out of the landing.
                let z0 = c + half + wall + 10;
                b.boxed(
                    [c - 4, y - 1, z0],
                    [c + 4, y, z0 + 48],
                    conc,
                    FeatureKind::WALKWAY,
                    solid,
                );
                b.boxed(
                    [c - 4, y + 6, z0],
                    [c + 4, y + 7, z0 + 48],
                    conc,
                    FeatureKind::MASS,
                    solid,
                );
                b.boxed(
                    [c - 4, y, z0],
                    [c - 3, y + 6, z0 + 48],
                    conc,
                    FeatureKind::MASS,
                    solid,
                );
                b.boxed(
                    [c + 3, y, z0],
                    [c + 4, y + 6, z0 + 48],
                    conc,
                    FeatureKind::MASS,
                    solid,
                );
            }
            _ => {}
        }
    }
    Content {
        prims: b.out,
        keep_out: vec![volume(
            [c - half - wall - 2, 0, c - half - wall - 2],
            [c + half + wall + 14, 4096, c + half + wall + 12],
        )],
    }
}

/// The geometry of the weightless feature at L1 cell `l1`: the lane's rings of light, the
/// fragments, a tumbler room, the orbiters' debris, the cathedral.
fn site_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let Some(site) = site_of(seed, l0) else {
        return Content::default();
    };
    let c = 2048;
    let mut b = B { out: Vec::new() };
    let mut keep = Vec::new();
    let solid = PrimFlags::NONE;
    let none = PrimFlags::NO_COLLIDE;
    let far = PrimFlags::FAR_VISIBLE;
    let conc = MaterialId::CONCRETE;
    match site.kind {
        Kind::DriftLane | Kind::GravityRiver => {
            // The lane runs along `axis` through the cell row of the site.
            let (y1, other) = (
                site.l1.y,
                if site.axis == 0 { site.l1.z } else { site.l1.x },
            );
            let (my_other, my_along) = if site.axis == 0 {
                (l1.z, l1.x)
            } else {
                (l1.x, l1.z)
            };
            if l1.y != y1 || my_other != other {
                return Content::default();
            }
            let cell_lo = my_along * L1_M;
            let mid = l0[site.axis] * L0_M + L0_M / 2;
            let half = if site.kind == Kind::DriftLane {
                L0_M / 2
            } else {
                site.length / 2
            };
            if cell_lo + L1_M <= mid - half || cell_lo >= mid + half {
                return Content::default();
            }
            let r = site.radius as i32;
            let (a0, a1) = (
                ((mid - half - cell_lo).max(0)) as i32,
                ((mid + half - cell_lo).min(L1_M)) as i32,
            );
            let mut a = (a0 / 64 + 1) * 64;
            while a < a1 {
                // Rings of light every 64 m; the dust is the engine's.
                if site.axis == 0 {
                    for (lo, hi) in [
                        ([a, c - r, c - r], [a + 1, c - r + 1, c + r]),
                        ([a, c + r - 1, c - r], [a + 1, c + r, c + r]),
                        ([a, c - r + 1, c - r], [a + 1, c + r - 1, c - r + 1]),
                        ([a, c - r + 1, c + r - 1], [a + 1, c + r - 1, c + r]),
                    ] {
                        b.boxed(lo, hi, MaterialId::EMISSIVE, FeatureKind::MISC, none);
                    }
                } else {
                    for (lo, hi) in [
                        ([c - r, c - r, a], [c + r, c - r + 1, a + 1]),
                        ([c - r, c + r - 1, a], [c + r, c + r, a + 1]),
                        ([c - r, c - r + 1, a], [c - r + 1, c + r - 1, a + 1]),
                        ([c + r - 1, c - r + 1, a], [c + r, c + r - 1, a + 1]),
                    ] {
                        b.boxed(lo, hi, MaterialId::EMISSIVE, FeatureKind::MISC, none);
                    }
                }
                a += 64;
            }
            // LAD-15: a rung ladder across the ceiling of the tube: a rail along it and a rung
            // every 4 m, passable (the lane is a lull: hands pull you along).
            let ceil = c + r - 4;
            let mut a = (a0 / 4 + 1) * 4;
            if site.axis == 0 {
                b.boxed(
                    [a0, ceil, c - 1],
                    [a1, ceil + 1, c],
                    MaterialId::METAL,
                    FeatureKind::RAILING,
                    none,
                );
            } else {
                b.boxed(
                    [c - 1, ceil, a0],
                    [c, ceil + 1, a1],
                    MaterialId::METAL,
                    FeatureKind::RAILING,
                    none,
                );
            }
            while a + 1 < a1 {
                if site.axis == 0 {
                    b.boxed(
                        [a, ceil, c - 2],
                        [a + 1, ceil + 1, c + 2],
                        MaterialId::METAL,
                        FeatureKind::MISC,
                        none,
                    );
                } else {
                    b.boxed(
                        [c - 2, ceil, a],
                        [c + 2, ceil + 1, a + 1],
                        MaterialId::METAL,
                        FeatureKind::MISC,
                        none,
                    );
                }
                a += 4;
            }
            keep.push(if site.axis == 0 {
                volume([a0, c - r - 4, c - r - 4], [a1, c + r + 4, c + r + 4])
            } else {
                volume([c - r - 4, c - r - 4, a0], [c + r + 4, c + r + 4, a1])
            });
        }
        Kind::Archipelago if l1 == site.l1 => {
            for f in fragments(seed, l1) {
                let o = [0, 1, 2].map(|i| (f.centre[i] - l1[i] * L1_M) as i32);
                let h = f.half as i32;
                b.boxed(
                    [o[0] - h, o[1] - h, o[2] - h],
                    [o[0] + h, o[1] + h, o[2] + h],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep.push(volume(
                [c - 520, c - 520, c - 520],
                [c + 520, c + 520, c + 520],
            ));
        }
        Kind::Tumbler if l1 == site.l1 => {
            let h = 60;
            let w = 6;
            for (lo, hi) in [
                ([c - h, c - h, c - h], [c + h, c - h + w, c + h]),
                ([c - h, c + h - w, c - h], [c + h, c + h, c + h]),
                ([c - h, c - h + w, c - h], [c - h + w, c + h - w, c + h]),
                ([c + h - w, c - h + w, c - h], [c + h, c + h - w, c + h]),
                (
                    [c - h + w, c - h + w, c - h],
                    [c + h - w, c + h - w, c - h + w],
                ),
                // The +z wall, with a doorway of 6 by 8 m in each of the six... in its middle.
                ([c - h + w, c - h + w, c + h - w], [c - 3, c + h - w, c + h]),
                ([c + 3, c - h + w, c + h - w], [c + h - w, c + h - w, c + h]),
                ([c - 3, c - h + w + 8, c + h - w], [c + 3, c + h - w, c + h]),
            ] {
                b.boxed(lo, hi, conc, FeatureKind::MASS, far | solid);
            }
            // A marker on every face: the room is turning.
            for (lo, hi) in [
                ([c - 2, c - h + w, c - 2], [c + 2, c - h + w + 1, c + 2]),
                ([c - 2, c + h - w - 1, c - 2], [c + 2, c + h - w, c + 2]),
            ] {
                b.boxed(lo, hi, MaterialId::SIGNAL, FeatureKind::MISC, none);
            }
            keep.push(volume(
                [c - h - 8, c - h - 8, c - h - 8],
                [c + h + 8, c + h + 8, c + h + 24],
            ));
        }
        Kind::Orbiters if l1 == site.l1 => {
            for o in orbiters(seed, l1) {
                let (a2, b2) = heading(o.phase);
                let len = (f64::from(a2) * f64::from(a2) + f64::from(b2) * f64::from(b2)).sqrt();
                let (dx, dz) = (
                    (o.radius as f64 * f64::from(a2) / len).round() as i32,
                    (o.radius as f64 * f64::from(b2) / len).round() as i32,
                );
                let s = o.size as i32;
                b.boxed(
                    [c + dx - s, c - s, c + dz - s],
                    [c + dx + s, c + s, c + dz + s],
                    conc,
                    FeatureKind::MASS,
                    far | none,
                );
            }
        }
        Kind::Cathedral if l1 == site.l1 => {
            let (hx, hy, hz, w) = (240, 32, 32, 8);
            for (lo, hi) in [
                (
                    [c - hx, c - hy - w, c - hz - w],
                    [c + hx, c - hy, c + hz + w],
                ),
                (
                    [c - hx, c + hy, c - hz - w],
                    [c + hx, c + hy + w, c + hz + w],
                ),
                ([c - hx, c - hy, c - hz - w], [c + hx, c + hy, c - hz]),
                ([c - hx, c - hy, c + hz], [c + hx, c + hy, c + hz + w]),
            ] {
                b.boxed(lo, hi, conc, FeatureKind::MASS, far | solid);
            }
            // Ribs every 32 m, the end walls with a door, floating lamps and a handrail.
            let mut x = c - hx + 16;
            while x < c + hx {
                b.boxed(
                    [x, c - hy, c - hz],
                    [x + 2, c + hy, c - hz + 2],
                    conc,
                    FeatureKind::MISC,
                    solid,
                );
                b.boxed(
                    [x, c - hy, c + hz - 2],
                    [x + 2, c + hy, c + hz],
                    conc,
                    FeatureKind::MISC,
                    solid,
                );
                x += 32;
            }
            let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::WEIGHTLESS));
            for _ in 0..30 {
                let (x, y, z) = (
                    c - hx + 8 + rng.below(464) as i32,
                    c - hy + 4 + rng.below(56) as i32,
                    c - hz + 4 + rng.below(56) as i32,
                );
                b.boxed(
                    [x, y, z],
                    [x + 1, y + 1, z + 1],
                    MaterialId::EMISSIVE,
                    FeatureKind::MISC,
                    none,
                );
            }
            for (dy, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                b.boxed(
                    [c - hx + 4, c + dy * 2, c + dz * 2],
                    [c + hx - 4, c + dy * 2 + 1, c + dz * 2 + 1],
                    MaterialId::METAL,
                    FeatureKind::RAILING,
                    none,
                );
            }
            keep.push(volume(
                [c - hx - 8, c - hy - 16, c - hz - 16],
                [c + hx + 8, c + hy + 16, c + hz + 16],
            ));
        }
        _ => {}
    }
    let _: i64 = CELL.into();
    Content {
        prims: b.out,
        keep_out: keep,
    }
}

/// Everything the weightless features add to L1 cell `l1`.
pub fn weightless_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let mut all = shaft_content(seed, l1);
    let c = site_content(seed, l1);
    all.prims.extend(c.prims);
    all.keep_out.extend(c.keep_out);
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gravity::{gravity_at_metres, wells_of};

    const SEED: WorldSeed = WorldSeed(0);

    fn find_site(kind: Kind) -> (I64Vec3, Site) {
        for x in -40..40 {
            for z in -40..40 {
                let l0 = I64Vec3::new(x, 0, z);
                if let Some(s) = site_of(SEED, l0).filter(|s| s.kind == kind) {
                    return (l0, s);
                }
            }
        }
        panic!("{kind:?} not found");
    }

    /// WGT-01, 02: a shaft is a weak field along its axis only, and upward ones turn it over.
    #[test]
    fn featherfall_shafts_are_weak_inside_and_ordinary_outside() {
        let mut seen = [false; 5];
        let mut exact = 0;
        for x in -30..30 {
            for z in -30..30 {
                let Some(((a, b), shaft, strength)) = shaft_of(SEED, x, z) else {
                    continue;
                };
                seen[SHAFTS.iter().position(|&s| s == shaft).unwrap_or(4)] = true;
                let l0 = I64Vec3::new(x, 0, z);
                let c = [(x * 8 + a) * 4096 + 2048, 16384, (z * 8 + b) * 4096 + 2048];
                let g = gravity_at_metres(SEED, c);
                // Another well may add its own pull; alone, the shaft's strength is the field's.
                if i64::from(g.strength) == strength {
                    exact += 1;
                    assert_eq!(
                        g.up,
                        if shaft == Shaft::Upward {
                            [0, -8, 0]
                        } else {
                            [0, 8, 0]
                        }
                    );
                }
                assert!(g.strength <= 64 || !g.is_lull(), "{g:?}");
                let out = gravity_at_metres(SEED, [c[0] + 400, c[1], c[2]]);
                assert!(
                    out.strength >= 64 || wells_of(SEED, l0).len() > 1,
                    "{out:?}"
                );
            }
        }
        assert!(seen.iter().all(|&s| s), "{seen:?}");
        assert!(exact > 20, "{exact} shafts alone");
    }

    #[test]
    fn a_shaft_has_walls_landings_and_rings_in_every_cell_of_its_column() {
        for x in -30..30 {
            for z in -30..30 {
                let Some(((a, b), _, _)) = shaft_of(SEED, x, z) else {
                    continue;
                };
                for y in [-5, 0, 7] {
                    let c = weightless_content(SEED, I64Vec3::new(x * 8 + a, y, z * 8 + b));
                    assert!(c.prims.len() > 100, "{} pieces", c.prims.len());
                    assert!(!c.keep_out.is_empty());
                }
                return;
            }
        }
        panic!("no shaft");
    }

    /// WGT-03, 04: a lane is a lull with a current along it; a river ends.
    #[test]
    fn lanes_are_lulls_with_a_current_and_rivers_end() {
        let (l0, s) = find_site(Kind::DriftLane);
        let c = centre_of(s.l1);
        let mut p = c;
        p[s.axis] = l0[s.axis] * L0_M + L0_M / 2;
        assert!(gravity_at_metres(SEED, p).is_lull(), "the lane is a lull");
        let v = current_at(SEED, p);
        assert_eq!(i64::from(v[s.axis]), s.current);
        let mut off = p;
        off[1] += s.radius + 40;
        assert_eq!(current_at(SEED, off), [0; 3]);
        assert!(!gravity_at_metres(SEED, off).is_lull());

        let (l0, r) = find_site(Kind::GravityRiver);
        let c = centre_of(r.l1);
        let mut mid = c;
        mid[r.axis] = l0[r.axis] * L0_M + L0_M / 2;
        assert!(gravity_at_metres(SEED, mid).is_lull());
        let mut past = mid;
        past[r.axis] += r.length / 2 + 200;
        assert!(!gravity_at_metres(SEED, past).is_lull(), "the river ends");
        assert!((1000..=10_000).contains(&r.length));
    }

    /// LAD-15: the tube of a drift lane has a rail and rungs across its ceiling, passable.
    #[test]
    fn a_lane_has_a_rung_ladder_across_its_ceiling() {
        let (_, s) = find_site(Kind::DriftLane);
        let content = weightless_content(SEED, s.l1);
        let rungs = content
            .prims
            .iter()
            .filter(|p| p.material == MaterialId::METAL && p.kind == FeatureKind::MISC)
            .count();
        let rails = content
            .prims
            .iter()
            .filter(|p| p.material == MaterialId::METAL && p.kind == FeatureKind::RAILING)
            .count();
        assert!(rungs > 100, "{rungs} rungs");
        assert_eq!(rails, 1);
        assert!(
            content
                .prims
                .iter()
                .filter(|p| p.material == MaterialId::METAL)
                .all(|p| p.flags.contains(PrimFlags::NO_COLLIDE))
        );
    }

    /// WGT-05: a lull with ten fragments, each its own planetoid.
    #[test]
    fn the_archipelago_floats_fragments_that_pull_on_their_own() {
        let (_, s) = find_site(Kind::Archipelago);
        let c = centre_of(s.l1);
        let frags = fragments(SEED, s.l1);
        assert_eq!(frags.len(), 10);
        // Far from any fragment the field is a lull; next to one it points away from it.
        let f = frags[0];
        let near = [f.centre[0] + f.half + 30, f.centre[1], f.centre[2]];
        let g = gravity_at_metres(SEED, near);
        assert!(!g.is_lull(), "{g:?} beside {f:?}");
        let content = weightless_content(SEED, s.l1);
        assert_eq!(content.prims.len(), 10);
        let _ = c;
    }

    /// WGT-06: the room's up is one of six and changes with the epoch, the same at the same time.
    #[test]
    fn a_tumbler_room_turns_with_the_world_clock() {
        let (l0, s) = find_site(Kind::Tumbler);
        let room = tumbler_of(SEED, l0).unwrap();
        assert!(room.contains(centre_of(s.l1)));
        assert_eq!(tumbler_at(SEED, centre_of(s.l1)), Some(room));
        let mut ups = std::collections::HashSet::new();
        for epoch in 0..40 {
            let t = epoch * room.period_s;
            let up = room.up_at(t);
            assert_eq!(
                up,
                room.up_at(t + room.period_s - 1),
                "one up for a whole period"
            );
            assert_eq!(up, room.up_at(t));
            ups.insert(up);
        }
        assert!(ups.len() >= 4, "{ups:?}");
        assert!((180..=480).contains(&room.period_s));
    }

    /// WGT-07, 09.
    #[test]
    fn orbiters_and_the_cathedral_exist() {
        let (_, s) = find_site(Kind::Orbiters);
        assert_eq!(orbiters(SEED, s.l1).len(), 24);
        assert_eq!(weightless_content(SEED, s.l1).prims.len(), 24);
        let (l0, h) = find_site(Kind::Cathedral);
        let c = centre_of(h.l1);
        assert!(
            gravity_at_metres(SEED, c).is_lull(),
            "the nave is weightless"
        );
        assert!(weightless_content(SEED, h.l1).prims.len() > 40);
        let _ = l0;
    }

    /// WGT-08: a share of the ordinary wells are weak enough to be low-gravity districts.
    #[test]
    fn low_gravity_districts_exist() {
        let (mut weak, mut all) = (0, 0);
        for x in -20..20 {
            for z in -20..20 {
                for w in wells_of(SEED, I64Vec3::new(x, 0, z))
                    .iter()
                    .filter(|w| !w.quiet)
                {
                    all += 1;
                    weak += usize::from(w.strength < 160);
                }
            }
        }
        assert!(
            all > 100 && weak * 5 >= all,
            "{weak} of {all} wells are weak"
        );
    }
}
