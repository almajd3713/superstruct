//! World-clock movers (SYS-01, P2-26..29, `docs/proposals/the-web-ideas.md` §6 and §19): a moving
//! bridge, a lift bridge, a lift that turns and a tram that goes through a gate. Spec:
//! `docs/spec/levels.md` §3W.10.
//!
//! A mover is a **pure function of the world clock**: its `pose(t)` depends on integers only, so
//! it agrees across restarts at the same world time and on every thread. The generator emits the
//! parts of the moving piece in its own frame and the fixed supports as an ordinary feature; the
//! engine draws and collides the moving piece at `pose(t)`.

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
use crate::prim::{FeatureKind, MaterialId, QAabb, QVec3};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L1_M: i64 = 4096;
/// An L0 cell holds a mover with chance 1 in this.
const ONE_IN: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoverKind {
    /// P2-26: a leaf that slides away from a gap and back.
    MovingBridge,
    /// P2-27: a leaf that lifts out of the way (a lift bridge: the frames of a pitching leaf do
    /// not tilt further than 30°).
    Drawbridge,
    /// P2-28: a lift cabin that turns a quarter as it rises.
    TurningLift,
    /// P2-29: a tram car on a line through a ring of light that turns it over the gate.
    GateTram,
    /// BLD-01..05, P2-30: a Builder: a slow giant machine that places a beam every six minutes
    /// for an hour while it creeps along its rails, or one half built in its dock.
    Builder,
    /// NOT-01: an engine hall whose rotor starts turning when someone walks in and spins down
    /// after they leave. Not on the world clock: the engine drives it (`pose_sensed`).
    EngineHall,
    /// NOT-03: four doors in a gallery, each of which rises as someone comes within reach.
    Doors,
}

/// The movers that notice the walker (PW.10.15); their pose is `pose_sensed`.
pub const SENSED: [MoverKind; 2] = [MoverKind::EngineHall, MoverKind::Doors];

pub const KINDS: [MoverKind; 5] = [
    MoverKind::MovingBridge,
    MoverKind::Drawbridge,
    MoverKind::TurningLift,
    MoverKind::GateTram,
    MoverKind::Builder,
];

/// The Builder's cycle: an hour of work, then 23 hours with the frame standing, then it begins
/// again (the frame is gone and the machine is back at its start).
pub const BUILDER_CYCLE_MS: u64 = 86_400_000;
/// The hour of work.
pub const BUILDER_WORK_MS: u64 = 3_600_000;
/// Beams a Builder places, one every `BUILDER_WORK_MS / BEAMS`.
pub const BEAMS: usize = 10;

impl MoverKind {
    pub const fn id(self) -> &'static str {
        match self {
            MoverKind::MovingBridge => "P2-26",
            MoverKind::Drawbridge => "P2-27",
            MoverKind::TurningLift => "P2-28",
            MoverKind::GateTram => "P2-29",
            MoverKind::Builder => "P2-30",
            MoverKind::EngineHall => "NOT-01",
            MoverKind::Doors => "NOT-03",
        }
    }
}

/// One box of a moving piece, in quanta from the mover's origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub aabb: QAabb,
    pub material: MaterialId,
    pub kind: FeatureKind,
    pub solid: bool,
}

/// Where a mover is at a time: a shift in metres of its origin and a turn about the vertical
/// axis through the origin, in 64ths of a turn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub offset: [f32; 3],
    pub yaw: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mover {
    pub kind: MoverKind,
    /// Absolute metres of the mover's origin at rest.
    pub origin: [i64; 3],
    pub parts: Vec<Part>,
    /// Pieces that appear at a time in the cycle, in milliseconds, and stay until it ends
    /// (BLD-05: a schedule of predetermined geometry).
    pub timed: Vec<(Part, u64)>,
    /// A Builder half built in its dock: it does not move and places nothing.
    pub dock: bool,
    /// The cycle and where this mover is in it at world time 0, in milliseconds.
    pub period_ms: u64,
    pub phase_ms: u64,
    /// How far it travels (metres) or how high it lifts, per kind.
    pub reach: i64,
    /// A key that is the same for every thread and restart.
    pub key: u64,
}

/// `u / 1024` of a cycle as a smoothed ramp: 0 until `a`, up to 1024 at `b`, held, back down
/// between `c` and `d` (all out of 1024). Integers only, with the smoothstep of the gates.
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

/// Milliseconds a door takes to rise, and the rotor's quarter-turn steps per second.
pub const DOOR_RISE_M: i64 = 9;
const ROTOR_STEP_MS: u64 = 250;

impl Mover {
    /// Whether this mover follows the walker instead of the world clock.
    pub fn sensed(&self) -> bool {
        SENSED.contains(&self.kind)
    }

    /// How near, in metres, the walker wakes it.
    pub fn wake_radius(&self) -> f64 {
        match self.kind {
            MoverKind::EngineHall => 90.0,
            _ => 24.0,
        }
    }

    /// The pose of a sensed mover that is `awake` (0 to 1024) awake and has turned for
    /// `spun_ms`: a door is raised by `awake`; the rotor is turned by the time it has run.
    pub fn pose_sensed(&self, awake: i64, spun_ms: u64) -> Pose {
        let smooth = |t: i64| t * t * (3 * 1024 - 2 * t) / (1024 * 1024);
        match self.kind {
            MoverKind::Doors => Pose {
                offset: [
                    0.0,
                    (smooth(awake.clamp(0, 1024)) * self.reach) as f32 / 1024.0,
                    0.0,
                ],
                yaw: 0,
            },
            MoverKind::EngineHall => Pose {
                offset: [0.0; 3],
                yaw: ((spun_ms / ROTOR_STEP_MS) % 64) as u8,
            },
            _ => self.pose(spun_ms),
        }
    }

    /// The position in the cycle in 1024ths at world time `t_ms`.
    fn cycle(&self, t_ms: u64) -> i64 {
        (((t_ms + self.phase_ms) % self.period_ms) * 1024 / self.period_ms) as i64
    }

    /// Milliseconds into the cycle at world time `t_ms`.
    pub fn cycle_ms(&self, t_ms: u64) -> u64 {
        (t_ms + self.phase_ms) % self.period_ms
    }

    /// How many of the timed pieces stand at world time `t_ms`.
    pub fn placed(&self, t_ms: u64) -> usize {
        let c = self.cycle_ms(t_ms);
        self.timed.iter().filter(|(_, at)| *at <= c).count()
    }

    pub fn pose(&self, t_ms: u64) -> Pose {
        if self.kind == MoverKind::Builder {
            // Creeps along its rails during the hour of work, then waits at the end.
            let c = self.cycle_ms(t_ms).min(BUILDER_WORK_MS) as i64;
            let along = if self.dock {
                0.0
            } else {
                (c * self.reach) as f32 / BUILDER_WORK_MS as f32
            };
            return Pose {
                offset: [along, 0.0, 0.0],
                yaw: 0,
            };
        }
        let u = self.cycle(t_ms);
        let f = |k: i64| (k * self.reach) as f32 / 1024.0;
        match self.kind {
            // Spans the gap for a third of the cycle, slides away, waits, slides back.
            MoverKind::MovingBridge => Pose {
                offset: [f(ramp(u, 340, 500, 840, 1000)), 0.0, 0.0],
                yaw: 0,
            },
            MoverKind::Drawbridge => Pose {
                offset: [0.0, f(ramp(u, 340, 500, 840, 1000)), 0.0],
                yaw: 0,
            },
            // Rises 64 m over the first half and sinks back over the second, a quarter turn
            // (16 of 64) from the foot to the top.
            MoverKind::TurningLift => {
                let k = ramp(u, 0, 480, 520, 1000);
                Pose {
                    offset: [0.0, f(k), 0.0],
                    yaw: (k * 16 / 1024) as u8,
                }
            }
            // Runs the length of its line and back; over the ring in the middle it is turned
            // half a turn.
            MoverKind::Builder | MoverKind::EngineHall | MoverKind::Doors => Pose {
                offset: [0.0; 3],
                yaw: 0,
            },
            MoverKind::GateTram => {
                let tri = if u < 512 { u * 2 } else { 2048 - u * 2 };
                let near_gate = (tri - 512).abs() < 128;
                Pose {
                    offset: [f(tri), 0.0, 0.0],
                    yaw: if near_gate { 32 } else { 0 },
                }
            }
        }
    }
}

/// The site of the mover of L0 cell `l0`: its kind and its inner L1 cell.
pub fn site_of(seed: WorldSeed, l0: I64Vec3) -> Option<(MoverKind, I64Vec3)> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::MOVER_SITE));
    // In the Builder frontier (STG-13) half the cells hold a Builder.
    let frontier =
        crate::strange::strange_of(seed, l0) == Some(crate::strange::Strange::BuilderFrontier);
    if rng.below(if frontier { 2 } else { ONE_IN }) != 0 {
        return None;
    }
    let all: Vec<MoverKind> = KINDS.into_iter().chain(SENSED).collect();
    let drawn = all[rng.below(all.len() as u64) as usize];
    let kind = if frontier { MoverKind::Builder } else { drawn };
    let inner = |r: &mut Rng| 1 + r.below(6) as i64;
    Some((
        kind,
        l0 * 8 + I64Vec3::new(inner(&mut rng), inner(&mut rng), inner(&mut rng)),
    ))
}

fn part(lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind, solid: bool) -> Part {
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    Part {
        aabb: QAabb::new(q(lo), q(hi)),
        material,
        kind,
        solid,
    }
}

/// The movers of L1 cell `l1` (at most one).
pub fn movers_in_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<Mover> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let Some((kind, site)) = site_of(seed, l0).filter(|s| s.1 == l1) else {
        return Vec::new();
    };
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::MOVER_SITE));
    let c = 2048i32;
    let period_ms = if kind == MoverKind::Builder {
        BUILDER_CYCLE_MS
    } else {
        240_000 + 30_000 * rng.below(9)
    };
    let phase_ms = rng.below(period_ms);
    // One Builder in three is in its dock, half built.
    let dock = kind == MoverKind::Builder && rng.below(3) == 0;
    let mut timed: Vec<(Part, u64)> = Vec::new();
    let key = rng.next_u64();
    let origin = [0, 1, 2].map(|i| site[i] * L1_M + i64::from(c));
    let conc = MaterialId::CONCRETE;
    let metal = MaterialId::METAL;
    let (parts, reach) = match kind {
        // The leaf of 120 m over a gap of 120 m between two banks; it slides 140 m along +x.
        MoverKind::MovingBridge => (
            vec![
                part([-60, -1, -6], [60, 0, 6], conc, FeatureKind::BRIDGE, true),
                part([-60, 0, -6], [60, 1, -5], metal, FeatureKind::RAILING, true),
                part([-60, 0, 5], [60, 1, 6], metal, FeatureKind::RAILING, true),
            ],
            140,
        ),
        MoverKind::Drawbridge => (
            vec![
                part([-40, -1, -6], [40, 0, 6], conc, FeatureKind::BRIDGE, true),
                part([-40, 0, -6], [40, 1, -5], metal, FeatureKind::RAILING, true),
                part([-40, 0, 5], [40, 1, 6], metal, FeatureKind::RAILING, true),
            ],
            60,
        ),
        MoverKind::TurningLift => (
            vec![
                part([-5, -1, -5], [5, 0, 5], metal, FeatureKind::DECK, true),
                part(
                    [-5, 0, -5],
                    [-4, 3, 5],
                    metal,
                    FeatureKind::SHAFT_WALL,
                    true,
                ),
                part([4, 0, -5], [5, 3, 5], metal, FeatureKind::SHAFT_WALL, true),
                part(
                    [-4, 0, -5],
                    [4, 3, -4],
                    metal,
                    FeatureKind::SHAFT_WALL,
                    true,
                ),
                part([-5, 3, -5], [5, 4, 5], metal, FeatureKind::MASS, true),
            ],
            64,
        ),
        // The machine: a deck on four legs with a gantry and a head; the beams it places are
        // the timed pieces, one every six minutes along 240 m of frame.
        MoverKind::Builder => {
            for k in 0..BEAMS {
                let x = 30 + 24 * k as i32;
                timed.push((
                    part(
                        [x, 12, -2],
                        [x + 20, 16, 2],
                        metal,
                        FeatureKind::TETHER,
                        true,
                    ),
                    BUILDER_WORK_MS * (k as u64 + 1) / BEAMS as u64,
                ));
            }
            let mut parts = vec![
                part([-20, 8, -10], [20, 10, 10], metal, FeatureKind::DECK, true),
                part([-20, 0, -10], [-17, 8, -7], metal, FeatureKind::MASS, true),
                part([17, 0, -10], [20, 8, -7], metal, FeatureKind::MASS, true),
            ];
            if !dock {
                parts.extend([
                    part([-20, 0, 7], [-17, 8, 10], metal, FeatureKind::MASS, true),
                    part([17, 0, 7], [20, 8, 10], metal, FeatureKind::MASS, true),
                    part([-20, 10, -10], [20, 24, -7], metal, FeatureKind::MASS, true),
                    part([20, 10, -3], [34, 14, 3], metal, FeatureKind::MASS, true),
                    part(
                        [30, 6, -1],
                        [34, 10, 1],
                        MaterialId::WARM,
                        FeatureKind::WINDOW,
                        false,
                    ),
                ]);
            }
            (parts, 240)
        }
        // A rotor of three long blades and a short one on a hub, 14 m above the floor of the
        // hall, passable (the blades turn high above the walkway).
        MoverKind::EngineHall => (
            vec![
                part([-4, 12, -4], [4, 20, 4], metal, FeatureKind::MASS, false),
                part([4, 14, -1], [44, 18, 1], metal, FeatureKind::TETHER, false),
                part([-1, 14, 4], [1, 18, 40], metal, FeatureKind::TETHER, false),
                part(
                    [-34, 14, -1],
                    [-4, 18, 1],
                    metal,
                    FeatureKind::TETHER,
                    false,
                ),
                part(
                    [-1, 14, -24],
                    [1, 18, -4],
                    MaterialId::WARM,
                    FeatureKind::TETHER,
                    false,
                ),
            ],
            0,
        ),
        // One leaf of a door, 12 m wide and 8 m high, that rises 9 m.
        MoverKind::Doors => (
            vec![part([-6, 0, -1], [6, 8, 0], metal, FeatureKind::MASS, true)],
            DOOR_RISE_M,
        ),
        MoverKind::GateTram => (
            vec![
                part([-12, 0, -2], [12, 1, 2], metal, FeatureKind::TRAM, true),
                part([-12, 1, -2], [12, 4, -1], metal, FeatureKind::TRAM, true),
                part([-12, 1, 1], [12, 4, 2], metal, FeatureKind::TRAM, true),
                part([-12, 4, -2], [12, 5, 2], metal, FeatureKind::TRAM, true),
                part(
                    [-12, 1, -1],
                    [-11, 4, 1],
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    false,
                ),
                part(
                    [11, 1, -1],
                    [12, 4, 1],
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    false,
                ),
            ],
            400,
        ),
    };
    // The gallery has four doors, 40 m apart along z.
    let doors: Vec<i64> = if kind == MoverKind::Doors {
        (0..4).map(|k| (k - 2) * 40 + 20).collect()
    } else {
        vec![0]
    };
    doors
        .into_iter()
        .enumerate()
        .map(|(i, dz)| Mover {
            kind,
            origin: [origin[0], origin[1], origin[2] + dz],
            parts: parts.clone(),
            timed: timed.clone(),
            dock,
            period_ms,
            phase_ms,
            reach,
            key: key.wrapping_add(i as u64),
        })
        .collect()
}

/// The fixed supports of the mover in L1 cell `l1`, and the space it moves through.
pub fn mover_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let Some(m) = movers_in_cell(seed, l1).into_iter().next() else {
        return Content::default();
    };
    let kind = m.kind;
    let c = 2048i32;
    let mut b: Vec<crate::prim::Primitive> = Vec::new();
    let mut add = |lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind, flags| {
        let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
        b.push(crate::prim::Primitive {
            shape: crate::prim::Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    };
    let solid = crate::prim::PrimFlags::NONE;
    let far = crate::prim::PrimFlags::FAR_VISIBLE;
    let conc = MaterialId::CONCRETE;
    let keep = match kind {
        // Two banks of 60 m with a gap of 120 m between them; the leaf rests over the gap.
        MoverKind::MovingBridge => {
            add(
                [c - 180, c - 8, c - 20],
                [c - 60, c, c + 20],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c + 60, c - 8, c - 20],
                [c + 180, c, c + 20],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c - 180, c - 8, c - 20],
                [c + 180, c - 6, c - 14],
                conc,
                FeatureKind::MASS,
                solid,
            );
            ([c - 190, c - 40, c - 24], [c + 330, c + 20, c + 24])
        }
        MoverKind::Drawbridge => {
            add(
                [c - 120, c - 8, c - 20],
                [c - 40, c, c + 20],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c + 40, c - 8, c - 20],
                [c + 120, c, c + 20],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for x in [-130, 120] {
                add(
                    [c + x, c, c - 8],
                    [c + x + 10, c + 80, c + 8],
                    MaterialId::METAL,
                    FeatureKind::TETHER,
                    far | solid,
                );
            }
            ([c - 130, c - 40, c - 24], [c + 130, c + 80, c + 24])
        }
        MoverKind::TurningLift => {
            // Landings at the foot and the top of the shaft, and four posts.
            add(
                [c - 12, c - 2, c - 12],
                [c + 12, c, c + 12],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c - 12, c + 62, c - 12],
                [c + 12, c + 64, c + 12],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for (x, z) in [(-9, -9), (9, -9), (-9, 9), (9, 9)] {
                add(
                    [c + x - 1, c, c + z - 1],
                    [c + x + 1, c + 68, c + z + 1],
                    MaterialId::METAL,
                    FeatureKind::TETHER,
                    solid,
                );
            }
            ([c - 14, c - 4, c - 14], [c + 14, c + 70, c + 14])
        }
        MoverKind::Builder => {
            // The rails, and the posts of the frame it is building; a work light on a mast,
            // far-visible (BLD-04).
            for z in [-9, 9] {
                add(
                    [c - 24, c - 1, c + z - 1],
                    [c + 280, c, c + z + 1],
                    MaterialId::METAL,
                    FeatureKind::TRAM,
                    solid,
                );
            }
            for k in 0..=BEAMS as i32 {
                let x = c + 30 + 24 * k;
                add(
                    [x - 2, c, c - 4],
                    [x + 2, c + 12, c + 4],
                    MaterialId::METAL,
                    FeatureKind::TETHER,
                    far | solid,
                );
            }
            add(
                [c - 22, c, c - 12],
                [c - 18, c + 80, c - 8],
                MaterialId::METAL,
                FeatureKind::TETHER,
                far | solid,
            );
            add(
                [c - 24, c + 80, c - 14],
                [c - 16, c + 86, c - 6],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                far | crate::prim::PrimFlags::NO_COLLIDE,
            );
            ([c - 28, c - 8, c - 16], [c + 290, c + 90, c + 16])
        }
        MoverKind::EngineHall => {
            // A floor of 100 m with a pillar at each corner and a ring of beams above.
            add(
                [c - 50, c - 2, c - 50],
                [c + 50, c, c + 50],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for (x, z) in [(-48, -48), (48, -48), (-48, 48), (48, 48)] {
                add(
                    [c + x - 2, c, c + z - 2],
                    [c + x + 2, c + 30, c + z + 2],
                    MaterialId::METAL,
                    FeatureKind::TETHER,
                    far | solid,
                );
            }
            for (lo, hi) in [
                ([c - 50, c + 28, c - 50], [c + 50, c + 30, c - 46]),
                ([c - 50, c + 28, c + 46], [c + 50, c + 30, c + 50]),
                ([c - 50, c + 28, c - 50], [c - 46, c + 30, c + 50]),
                ([c + 46, c + 28, c - 50], [c + 50, c + 30, c + 50]),
            ] {
                add(lo, hi, MaterialId::METAL, FeatureKind::MASS, far | solid);
            }
            ([c - 54, c - 6, c - 54], [c + 54, c + 34, c + 54])
        }
        MoverKind::Doors => {
            // A gallery 16 m wide and 200 m long along z, with a floor, two walls and a frame
            // at each of the four doors.
            add(
                [c - 8, c - 1, c - 100],
                [c + 8, c, c + 100],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for x in [-8, 6] {
                add(
                    [c + x, c, c - 100],
                    [c + x + 2, c + 12, c + 100],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            for k in 0..4 {
                let z = c + (k - 2) * 40 + 20;
                add(
                    [c - 8, c + 8, z - 2],
                    [c + 8, c + 12, z + 1],
                    MaterialId::METAL,
                    FeatureKind::MASS,
                    solid,
                );
            }
            ([c - 10, c - 4, c - 104], [c + 10, c + 22, c + 104])
        }
        MoverKind::GateTram => {
            // The line: two rails over 400 m, a station at each end and a ring of light.
            for z in [-1, 1] {
                add(
                    [c, c - 1, c + z - 1],
                    [c + 412, c, c + z],
                    MaterialId::METAL,
                    FeatureKind::TRAM,
                    solid,
                );
            }
            add(
                [c - 16, c - 2, c - 8],
                [c, c, c + 8],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            add(
                [c + 412, c - 2, c - 8],
                [c + 428, c, c + 8],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            let mid = c + 200;
            for (lo, hi) in [
                ([mid, c - 6, c - 7], [mid + 1, c - 5, c + 7]),
                ([mid, c + 8, c - 7], [mid + 1, c + 9, c + 7]),
                ([mid, c - 5, c - 7], [mid + 1, c + 8, c - 6]),
                ([mid, c - 5, c + 6], [mid + 1, c + 8, c + 7]),
            ] {
                add(
                    lo,
                    hi,
                    MaterialId::EMISSIVE,
                    FeatureKind::MISC,
                    crate::prim::PrimFlags::NO_COLLIDE,
                );
            }
            ([c - 18, c - 8, c - 12], [c + 430, c + 12, c + 12])
        }
    };
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    Content {
        prims: b,
        keep_out: vec![QAabb::new(q(keep.0), q(keep.1))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: MoverKind) -> (I64Vec3, Mover) {
        for x in -40..40 {
            for z in -40..40 {
                if let Some((k, l1)) = site_of(SEED, I64Vec3::new(x, 0, z))
                    && k == kind
                {
                    return (l1, movers_in_cell(SEED, l1).remove(0));
                }
            }
        }
        panic!("{kind:?} not found");
    }

    /// NOT-01, NOT-03: the sensed movers occur, a door rises with its wakefulness, and the rotor
    /// turns with the time it has run.
    #[test]
    fn sensed_movers_follow_the_walker_not_the_clock() {
        let (l1, door) = find(MoverKind::Doors);
        assert!(door.sensed());
        assert_eq!(movers_in_cell(SEED, l1).len(), 4);
        let keys: std::collections::HashSet<u64> =
            movers_in_cell(SEED, l1).iter().map(|m| m.key).collect();
        assert_eq!(keys.len(), 4);
        assert!(door.pose_sensed(0, 0).offset.iter().all(|c| c.abs() < 1e-6));
        assert!((door.pose_sensed(1024, 0).offset[1] - DOOR_RISE_M as f32).abs() < 1e-4);
        let mid = door.pose_sensed(512, 0).offset[1];
        assert!(mid > 0.0 && mid < DOOR_RISE_M as f32);
        // The world clock does not move a sensed mover.
        assert_eq!(door.pose(0), door.pose(123_456));
        let (l1, hall) = find(MoverKind::EngineHall);
        assert!(!mover_content(SEED, l1).prims.is_empty());
        assert_eq!(hall.pose_sensed(0, 0).yaw, 0);
        assert_ne!(hall.pose_sensed(1024, 1000).yaw, 0);
        assert_eq!(
            hall.pose_sensed(1024, 0).yaw,
            hall.pose_sensed(1024, 16_000).yaw
        );
    }

    #[test]
    fn movers_occur_in_every_kind() {
        for kind in KINDS {
            let (l1, m) = find(kind);
            assert_eq!(m.kind, kind);
            assert!(!m.parts.is_empty());
            assert!(!mover_content(SEED, l1).prims.is_empty(), "{kind:?}");
            assert!(
                (240_000..=480_000).contains(&m.period_ms)
                    || (kind == MoverKind::Builder && m.period_ms == BUILDER_CYCLE_MS)
            );
        }
    }

    /// SYS-01: the pose is a pure function of the world time, the same at the same time, and
    /// periodic.
    #[test]
    fn the_pose_is_a_pure_function_of_the_world_clock() {
        for kind in KINDS {
            let (_, m) = find(kind);
            let again = find(kind).1;
            for t in [0u64, 1, 59_999, 123_456, 7_654_321, 1 << 40] {
                assert_eq!(m.pose(t), again.pose(t), "{kind:?} at {t}");
                assert_eq!(m.pose(t), m.pose(t + m.period_ms), "{kind:?} is periodic");
            }
        }
    }

    #[test]
    fn each_mover_moves_and_rests() {
        for kind in KINDS.into_iter().filter(|&k| k != MoverKind::Builder) {
            let (_, m) = find(kind);
            let poses: Vec<Pose> = (0..64).map(|k| m.pose(k * m.period_ms / 64)).collect();
            let moved = poses.iter().filter(|p| **p != poses[0]).count();
            assert!(
                moved > 8 && moved < 64,
                "{kind:?}: moves in {moved} of 64 samples"
            );
            // The offset never exceeds the reach.
            for p in &poses {
                let r = m.reach as f32 + 0.01;
                assert!(p.offset.iter().all(|c| c.abs() <= r), "{kind:?} {p:?}");
            }
        }
        // The turning lift has turned a quarter at the top; the tram is turned over the ring.
        let (_, lift) = find(MoverKind::TurningLift);
        let top = (0..1024)
            .map(|u| lift.pose(u * lift.period_ms / 1024))
            .map(|p| p.yaw)
            .max();
        assert_eq!(top, Some(16));
        let (_, tram) = find(MoverKind::GateTram);
        assert!((0..256).any(|u| tram.pose(u * tram.period_ms / 256).yaw == 32));
    }

    /// BLD-01, 05: a Builder places a beam every six minutes over an hour and then the frame
    /// stands until the cycle ends; the machine creeps along its rails meanwhile.
    #[test]
    fn a_builder_places_beams_on_a_schedule() {
        let (_, m) = (0..40)
            .map(|i| find_nth(MoverKind::Builder, i))
            .find(|(_, m)| !m.dock)
            .unwrap();
        assert_eq!(m.timed.len(), BEAMS);
        // Start of the cycle: nothing placed; after the first slice: one; after the hour: all.
        let start = (m.period_ms - m.phase_ms) % m.period_ms;
        assert_eq!(m.placed(start), 0);
        assert_eq!(m.placed(start + BUILDER_WORK_MS / BEAMS as u64), 1);
        assert_eq!(m.placed(start + BUILDER_WORK_MS / 2), BEAMS / 2);
        assert_eq!(m.placed(start + BUILDER_WORK_MS + 1), BEAMS);
        assert_eq!(m.placed(start + m.period_ms - 1), BEAMS);
        assert_eq!(
            m.placed(start + m.period_ms),
            0,
            "the frame is gone and it begins again"
        );
        // The machine's creep: 240 m in the hour.
        let half = m.pose(start + BUILDER_WORK_MS / 2).offset[0];
        assert!((half - 120.0).abs() < 0.5, "{half}");
        assert!((m.pose(start + BUILDER_WORK_MS * 3).offset[0] - 240.0).abs() < 0.01);
    }

    /// BLD-03: some Builders are half built in their docks.
    #[test]
    fn some_builders_wait_half_built_in_a_dock() {
        let docks = (0..40)
            .map(|i| find_nth(MoverKind::Builder, i).1)
            .filter(|m| m.dock)
            .count();
        assert!(docks > 3 && docks < 30, "{docks} of 40");
        let m = (0..40)
            .map(|i| find_nth(MoverKind::Builder, i).1)
            .find(|m| m.dock)
            .unwrap();
        assert!(m.pose(123_456).offset.iter().all(|c| c.abs() < 1e-6));
    }

    fn find_nth(kind: MoverKind, n: usize) -> (I64Vec3, Mover) {
        let mut seen = 0;
        for x in -60..60 {
            for z in -60..60 {
                if let Some((k, l1)) = site_of(SEED, I64Vec3::new(x, 0, z))
                    && k == kind
                {
                    if seen == n {
                        return (l1, movers_in_cell(SEED, l1).remove(0));
                    }
                    seen += 1;
                }
            }
        }
        panic!("{kind:?} number {n} not found");
    }
}
