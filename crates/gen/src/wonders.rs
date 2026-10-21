//! The wonders (WON-01..24, `docs/proposals/the-web-ideas.md` §2): unique landmarks, one in
//! about a dozen L0 cells, each seen from tens of kilometres. Spec: `docs/spec/levels.md`
//! §3W.9.

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::too_many_arguments
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::features::{Content, M};
use crate::hash::hash;
use crate::l1::CELL;
use crate::orient::{FacetRing, Frame, LocalShape, facet_ring, heading};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wonder {
    Cenotaph,
    TatlinsSpiral,
    Dam,
    WalkingCity,
    BabelInverted,
    Organ,
    LampSun,
    Loom,
    Monument,
    Hyperboloid,
    Wheel,
    Cistern,
    Drop,
    Library,
    BridgeCity,
    Nest,
    Stair,
    FurnaceHeart,
    Antenna,
    ArchiveWall,
    Mirror,
    SleepingBuilder,
    Funnel,
    Gate,
}

/// In the order of WON-01..24.
pub const ALL: [Wonder; 24] = [
    Wonder::Cenotaph,
    Wonder::TatlinsSpiral,
    Wonder::Dam,
    Wonder::WalkingCity,
    Wonder::BabelInverted,
    Wonder::Organ,
    Wonder::LampSun,
    Wonder::Loom,
    Wonder::Monument,
    Wonder::Hyperboloid,
    Wonder::Wheel,
    Wonder::Cistern,
    Wonder::Drop,
    Wonder::Library,
    Wonder::BridgeCity,
    Wonder::Nest,
    Wonder::Stair,
    Wonder::FurnaceHeart,
    Wonder::Antenna,
    Wonder::ArchiveWall,
    Wonder::Mirror,
    Wonder::SleepingBuilder,
    Wonder::Funnel,
    Wonder::Gate,
];

impl Wonder {
    pub fn id(self) -> String {
        let n = ALL.iter().position(|&w| w == self).unwrap_or(0) + 1;
        format!("WON-{n:02}")
    }

    /// Whether this wonder is built here (the Drop needs the featherfall shafts of PW.10.14 and
    /// the Gate a Rampart, PW.10.34).
    pub const fn built(self) -> bool {
        !matches!(self, Wonder::Drop | Wonder::Gate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub kind: Wonder,
    pub l1: I64Vec3,
}

/// An L0 cell holds a wonder with chance 1 in this.
const ONE_IN: u64 = 12;

/// The wonder of L0 cell `l0`, if any, at an inner L1 cell.
pub fn site_of(seed: WorldSeed, l0: I64Vec3) -> Option<Site> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::WONDER_SITE));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    let built: Vec<Wonder> = ALL.into_iter().filter(|w| w.built()).collect();
    let kind = built[rng.below(built.len() as u64) as usize];
    let inner = |r: &mut Rng| 1 + r.below(6) as i64;
    let l1 = l0 * 8 + I64Vec3::new(inner(&mut rng), inner(&mut rng), inner(&mut rng));
    Some(Site { kind, l1 })
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

    /// A vertical cylinder from `y0` to `y1` of radius `r`, axis at `(x, z)`.
    fn column(
        &mut self,
        x: i32,
        z: i32,
        y0: i32,
        y1: i32,
        r: i32,
        material: MaterialId,
        flags: PrimFlags,
    ) {
        if y1 <= y0 {
            return;
        }
        self.out.push(Primitive {
            shape: Shape::cylinder(
                Axis::Y,
                QVec3::new(x * M, y0 * M, z * M),
                (y1 - y0) * M,
                r * M,
            ),
            material,
            kind: FeatureKind::MASS,
            flags,
        });
    }

    /// A box `[0, size]` in the frame `Frame::table(yaw, tilt)` with its corner at `at`.
    fn turned(
        &mut self,
        at: [i32; 3],
        size: [i32; 3],
        yaw: u8,
        tilt: u8,
        material: MaterialId,
        flags: PrimFlags,
    ) {
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(size[0] * M, size[1] * M, size[2] * M),
        );
        self.out.push(Primitive {
            shape: Shape::oriented(
                Frame::table(yaw, tilt),
                QVec3::new(at[0] * M, at[1] * M, at[2] * M),
                LocalShape::Box { aabb: local },
                QAabb::new(QVec3::new(0, 0, 0), QVec3::new(CELL, CELL, CELL)),
            ),
            material,
            kind: FeatureKind::MASS,
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

/// `(x, z)` offsets of `radius` metres at heading `k` (a table heading, 64 round the circle).
fn ring_point(k: u8, radius: i32) -> (i32, i32) {
    let (a, b) = heading(k);
    let len = (f64::from(a) * f64::from(a) + f64::from(b) * f64::from(b)).sqrt();
    (
        (f64::from(radius) * f64::from(a) / len).round() as i32,
        (f64::from(radius) * f64::from(b) / len).round() as i32,
    )
}

/// The geometry of the wonder of L1 cell `l1`, if it is a site.
pub fn wonder_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let Some(site) = site_of(seed, l0).filter(|s| s.l1 == l1) else {
        return Content::default();
    };
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::WONDER_BUILD));
    let mut b = B { out: Vec::new() };
    let c = 2048;
    let far = PrimFlags::FAR_VISIBLE;
    let solid = PrimFlags::NONE;
    let none = PrimFlags::NO_COLLIDE;
    let conc = MaterialId::CONCRETE;
    let metal = MaterialId::METAL;
    let clip = volume([0; 3], [4096; 3]);
    let mut keep = vec![volume(
        [c - 100, c - 100, c - 100],
        [c + 100, c + 100, c + 100],
    )];
    match site.kind {
        // WON-01: an empty tomb: a black cube of 600 m with one door and a plinth.
        Wonder::Cenotaph => {
            let (h, w) = (300, 24);
            let y0 = c - h;
            b.boxed(
                [c - h, y0, c - h],
                [c + h, y0 + w, c + h],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            b.boxed(
                [c - h, y0 + 2 * h - w, c - h],
                [c + h, y0 + 2 * h, c + h],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            for (lo, hi) in [
                ([c - h, y0 + w, c - h], [c - h + w, y0 + 2 * h - w, c + h]),
                ([c + h - w, y0 + w, c - h], [c + h, y0 + 2 * h - w, c + h]),
                (
                    [c - h + w, y0 + w, c + h - w],
                    [c + h - w, y0 + 2 * h - w, c + h],
                ),
                // The front wall, with a door of 24 by 60 m in its middle.
                (
                    [c - h + w, y0 + w, c - h],
                    [c - 12, y0 + 2 * h - w, c - h + w],
                ),
                (
                    [c + 12, y0 + w, c - h],
                    [c + h - w, y0 + 2 * h - w, c - h + w],
                ),
                (
                    [c - 12, y0 + 60, c - h],
                    [c + 12, y0 + 2 * h - w, c - h + w],
                ),
            ] {
                b.boxed(lo, hi, conc, FeatureKind::MASS, far | solid);
            }
            b.boxed(
                [c - 20, y0 + w, c - 10],
                [c + 20, y0 + w + 14, c + 10],
                metal,
                FeatureKind::MISC,
                solid,
            );
            b.boxed(
                [c - 2, y0 + w + 14, c - 2],
                [c + 2, y0 + w + 15, c + 2],
                MaterialId::WARM,
                FeatureKind::MISC,
                none,
            );
            keep = vec![volume(
                [c - h - 8, y0 - 8, c - h - 40],
                [c + h + 8, y0 + 2 * h + 8, c + h + 8],
            )];
        }
        // WON-02: two leaning helices of girders, rising 30°.
        Wonder::TatlinsSpiral => {
            for strand in 0..2u8 {
                for k in 0..48u8 {
                    let step = (k + 32 * strand) % 64;
                    let (dx, dz) = ring_point(step, 220);
                    let tangent = (step + 16) % 64;
                    b.turned(
                        [c + dx, c - 500 + i32::from(k) * 18, c + dz],
                        [70, 36, 36],
                        tangent,
                        6,
                        metal,
                        far | solid,
                    );
                }
            }
            keep = vec![volume(
                [c - 300, c - 520, c - 300],
                [c + 300, c + 480, c + 300],
            )];
        }
        // WON-03: a curved wall of 60 m, 400 m tall, 800 m in radius.
        Wonder::Dam => {
            let ring = facet_ring(
                &FacetRing {
                    axis: QVec3::new(c * M, (c - 200) * M, (c + 700) * M),
                    radius: 800 * M,
                    thick: 60 * M,
                    height: 400 * M,
                    n: 32,
                    start: 24,
                    material: conc,
                    kind: FeatureKind::MASS,
                    flags: far | solid,
                },
                clip,
            );
            b.out.extend(ring.into_iter().take(13));
            keep = vec![volume(
                [c - 700, c - 200, c - 200],
                [c + 700, c + 220, c + 120],
            )];
        }
        // WON-04: a city of towers on a platform standing on six legs.
        Wonder::WalkingCity => {
            b.boxed(
                [c - 350, c, c - 350],
                [c + 350, c + 30, c + 350],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for (x, z) in [
                (-300, -300),
                (0, -300),
                (300, -300),
                (-300, 300),
                (0, 300),
                (300, 300),
            ] {
                b.column(c + x, c + z, 0, c, 24, metal, far | solid);
            }
            for _ in 0..24 {
                let (x, z) = (
                    c - 320 + rng.below(560) as i32,
                    c - 320 + rng.below(560) as i32,
                );
                let (w, d, h) = (
                    30 + rng.below(30) as i32,
                    30 + rng.below(30) as i32,
                    40 + rng.below(120) as i32,
                );
                b.boxed(
                    [x, c + 30, z],
                    [x + w, c + 30 + h, z + d],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume(
                [c - 360, c - 8, c - 360],
                [c + 360, c + 220, c + 360],
            )];
        }
        // WON-05: a stepped tower hung from the top of the cell, widest at the top.
        Wonder::BabelInverted => {
            for k in 0..12 {
                let half = 40 + 24 * (11 - k);
                let (y1, y0) = (4096 - k * 70, 4096 - (k + 1) * 70);
                b.boxed(
                    [c - half, y0, c - half],
                    [c + half, y1, c + half],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume([c - 330, 3200, c - 330], [c + 330, 4096, c + 330])];
        }
        // WON-06: forty pipes in five ranks on a platform, and a console.
        Wonder::Organ => {
            b.boxed(
                [c - 220, c - 60, c - 120],
                [c + 220, c - 50, c + 120],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            for rank in 0..5 {
                for pipe in 0..8 {
                    let x = c - 190 + pipe * 54;
                    let z = c - 90 + rank * 40;
                    let r = 6 + rng.below(9) as i32;
                    let h = 150 + rng.below(350) as i32;
                    b.column(x, z, c - 50, c - 50 + h, r, MaterialId::PIPE, far | solid);
                }
            }
            b.boxed(
                [c - 30, c - 50, c + 130],
                [c + 30, c - 40, c + 150],
                metal,
                FeatureKind::MISC,
                solid,
            );
            keep = vec![volume(
                [c - 230, c - 70, c - 130],
                [c + 230, c + 460, c + 160],
            )];
        }
        // WON-07: a light of 300 m with spikes and a ring.
        Wonder::LampSun => {
            b.boxed(
                [c - 150, c - 150, c - 150],
                [c + 150, c + 150, c + 150],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                far | none,
            );
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let (x, y, z) = (c + dx * 260, c + dy * 260, c + dz * 260);
                b.boxed(
                    [x - 40, y - 40, z - 40],
                    [x + 40, y + 40, z + 40],
                    MaterialId::BEACON,
                    FeatureKind::MISC,
                    far | none,
                );
            }
            b.out.extend(facet_ring(
                &FacetRing {
                    axis: QVec3::new(c * M, (c - 16) * M, c * M),
                    radius: 500 * M,
                    thick: 36 * M,
                    height: 32 * M,
                    n: 16,
                    start: 0,
                    material: MaterialId::EMISSIVE,
                    kind: FeatureKind::MISC,
                    flags: far | none,
                },
                clip,
            ));
            keep = vec![volume(
                [c - 560, c - 320, c - 560],
                [c + 560, c + 320, c + 560],
            )];
        }
        // WON-08: a frame hung with 120 vertical cables and two shuttle beams.
        Wonder::Loom => {
            for x in [c - 400, c + 376] {
                b.boxed(
                    [x, c - 300, c - 12],
                    [x + 24, c + 300, c + 12],
                    metal,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            b.boxed(
                [c - 400, c + 276, c - 12],
                [c + 400, c + 300, c + 12],
                metal,
                FeatureKind::MASS,
                far | solid,
            );
            b.boxed(
                [c - 400, c - 300, c - 12],
                [c + 400, c - 276, c + 12],
                metal,
                FeatureKind::MASS,
                far | solid,
            );
            for k in 0..120 {
                let x = c - 380 + k * 6;
                b.boxed(
                    [x, c - 276, c - 1],
                    [x + 1, c + 276, c + 1],
                    metal,
                    FeatureKind::CABLE,
                    none,
                );
            }
            for y in [c - 80, c + 120] {
                b.boxed(
                    [c - 380, y, c - 8],
                    [c + 380, y + 12, c + 8],
                    metal,
                    FeatureKind::MISC,
                    solid,
                );
            }
            keep = vec![volume(
                [c - 410, c - 310, c - 40],
                [c + 410, c + 310, c + 40],
            )];
        }
        // WON-09: a blank slab: 200 by 800 m, 40 m thick, nothing on it.
        Wonder::Monument => {
            b.boxed(
                [c - 100, c - 400, c - 20],
                [c + 100, c + 400, c + 20],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            keep = vec![volume(
                [c - 140, c - 410, c - 120],
                [c + 140, c + 410, c + 120],
            )];
        }
        // WON-10: a tower whose waist is narrow: 24 rings of 16 facets.
        Wonder::Hyperboloid => {
            for k in 0..24 {
                let d = k - 12;
                let radius = 80 + d * d * 2;
                b.out.extend(facet_ring(
                    &FacetRing {
                        axis: QVec3::new(c * M, (c - 360 + k * 30) * M, c * M),
                        radius: radius * M,
                        thick: 24 * M,
                        height: 24 * M,
                        n: 16,
                        start: 0,
                        material: conc,
                        kind: FeatureKind::MASS,
                        flags: far | solid,
                    },
                    clip,
                ));
            }
            keep = vec![volume(
                [c - 420, c - 370, c - 420],
                [c + 420, c + 380, c + 420],
            )];
        }
        // WON-11: a wheel of 2 km: a rim of 64 facets, a hub and eight spokes.
        Wonder::Wheel => {
            b.out.extend(facet_ring(
                &FacetRing {
                    axis: QVec3::new(c * M, c * M, c * M),
                    radius: 1000 * M,
                    thick: 36 * M,
                    height: 48 * M,
                    n: 64,
                    start: 0,
                    material: conc,
                    kind: FeatureKind::MASS,
                    flags: far | solid,
                },
                clip,
            ));
            b.boxed(
                [c - 60, c, c - 60],
                [c + 60, c + 48, c + 60],
                metal,
                FeatureKind::MASS,
                far | solid,
            );
            for k in 0..8u8 {
                b.turned([c, c + 8, c], [960, 32, 32], k * 8, 0, metal, far | solid);
            }
            keep = vec![volume(
                [c - 1060, c - 8, c - 1060],
                [c + 1060, c + 60, c + 1060],
            )];
        }
        // WON-12: a hall of ten thousand identical columns, 16 m apart.
        Wonder::Cistern => {
            let (x0, z0) = (c - 800, c - 800);
            b.boxed(
                [x0, 0, z0],
                [x0 + 1600, 2, z0 + 1600],
                conc,
                FeatureKind::DECK,
                far | solid,
            );
            b.boxed(
                [x0, 44, z0],
                [x0 + 1600, 48, z0 + 1600],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            for i in 0..100 {
                for j in 0..100 {
                    b.boxed(
                        [x0 + 6 + 16 * i, 2, z0 + 6 + 16 * j],
                        [x0 + 10 + 16 * i, 44, z0 + 10 + 16 * j],
                        conc,
                        FeatureKind::MASS,
                        solid,
                    );
                }
            }
            keep = vec![volume([x0 - 4, 0, z0 - 4], [x0 + 1604, 52, z0 + 1604])];
        }
        // WON-14: six hexagonal galleries, a gallery floor every 8 m.
        Wonder::Library => {
            for ring in 0..6 {
                let radius = 120 + 60 * ring;
                for level in 0..30 {
                    b.out.extend(facet_ring(
                        &FacetRing {
                            axis: QVec3::new(c * M, (c - 120 + level * 8) * M, c * M),
                            radius: radius * M,
                            thick: 24 * M,
                            height: 2 * M,
                            n: 8,
                            start: 0,
                            material: conc,
                            kind: FeatureKind::DECK,
                            flags: far | solid,
                        },
                        clip,
                    ));
                }
            }
            for k in 0..8u8 {
                let (dx, dz) = ring_point(k * 8, 470);
                b.boxed(
                    [c + dx - 6, c - 120, c + dz - 6],
                    [c + dx + 6, c + 130, c + dz + 6],
                    metal,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume(
                [c - 560, c - 130, c - 560],
                [c + 560, c + 140, c + 560],
            )];
        }
        // WON-15: a deck of 3 km with a city of towers on it, on three arches.
        Wonder::BridgeCity => {
            b.boxed(
                [c - 1500, c, c - 25],
                [c + 1500, c + 6, c + 25],
                conc,
                FeatureKind::BRIDGE,
                far | solid,
            );
            for x in [-1000, 0, 1000] {
                b.boxed(
                    [c + x - 30, c - 220, c - 22],
                    [c + x + 30, c, c + 22],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            for _ in 0..18 {
                let x = c - 1400 + rng.below(2760) as i32;
                let (w, h) = (20 + rng.below(30) as i32, 30 + rng.below(110) as i32);
                b.boxed(
                    [x, c + 6, c - 20],
                    [x + w, c + 6 + h, c + 20],
                    conc,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume(
                [c - 1510, c - 230, c - 30],
                [c + 1510, c + 160, c + 30],
            )];
        }
        // WON-16: a tangle of 220 beams round a core.
        Wonder::Nest => {
            b.boxed(
                [c - 40, c - 40, c - 40],
                [c + 40, c + 40, c + 40],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            for _ in 0..220 {
                let (dx, dz) = ring_point(rng.below(64) as u8, 40 + rng.below(220) as i32);
                let y = c - 220 + rng.below(440) as i32;
                b.turned(
                    [c + dx, y, c + dz],
                    [40 + rng.below(50) as i32, 4, 4],
                    rng.below(64) as u8,
                    rng.below(7) as u8,
                    metal,
                    solid,
                );
            }
            keep = vec![volume(
                [c - 330, c - 330, c - 330],
                [c + 330, c + 330, c + 330],
            )];
        }
        // WON-17: one stair 200 m wide, 400 m of rise over 600 m.
        Wonder::Stair => {
            let aabb = QAabb::new(
                QVec3::new((c - 100) * M, (c - 200) * M, (c - 300) * M),
                QVec3::new((c + 100) * M, (c + 200) * M, (c + 300) * M),
            );
            b.out.push(Primitive {
                shape: Shape::stair_run(aabb, crate::prim::Dir4::PosZ, 1600),
                material: conc,
                kind: FeatureKind::STAIR_TOWER,
                flags: far | solid,
            });
            keep = vec![volume(
                [c - 110, c - 210, c - 310],
                [c + 110, c + 420, c + 310],
            )];
        }
        // WON-18: a hot core in a cage of eight chimneys.
        Wonder::FurnaceHeart => {
            b.boxed(
                [c - 150, c - 150, c - 150],
                [c + 150, c + 150, c + 150],
                MaterialId::WARM,
                FeatureKind::MASS,
                far | solid,
            );
            b.boxed(
                [c - 100, c - 100, c - 160],
                [c + 100, c + 100, c - 150],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                far | none,
            );
            for (x, z) in [
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
                (0, -1),
                (0, 1),
                (-1, 0),
                (1, 0),
            ] {
                let (px, pz) = (c + x * 200 - 18, c + z * 200 - 18);
                b.boxed(
                    [px, c - 150, pz],
                    [px + 36, c + 350, pz + 36],
                    metal,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume(
                [c - 260, c - 160, c - 260],
                [c + 260, c + 360, c + 260],
            )];
        }
        // WON-19: a mast of 1.4 km with cross arms and a light.
        Wonder::Antenna => {
            b.boxed(
                [c - 20, 100, c - 20],
                [c + 20, 1500, c + 20],
                metal,
                FeatureKind::TETHER,
                far | solid,
            );
            for y in [400, 800, 1200] {
                b.boxed(
                    [c - 120, y, c - 6],
                    [c + 120, y + 12, c + 6],
                    metal,
                    FeatureKind::MISC,
                    far | solid,
                );
                b.boxed(
                    [c - 6, y + 12, c - 120],
                    [c + 6, y + 24, c + 120],
                    metal,
                    FeatureKind::MISC,
                    far | solid,
                );
            }
            b.boxed(
                [c - 18, 1500, c - 18],
                [c + 18, 1536, c + 18],
                MaterialId::SIGNAL,
                FeatureKind::MISC,
                far | none,
            );
            keep = vec![volume([c - 130, 90, c - 130], [c + 130, 1540, c + 130])];
        }
        // WON-20: a wall of drawers: 600 by 300 m, 12 by 12 drawers.
        Wonder::ArchiveWall => {
            b.boxed(
                [c - 300, c - 150, c - 20],
                [c + 300, c + 150, c + 20],
                conc,
                FeatureKind::MASS,
                far | solid,
            );
            for i in 0..12 {
                for j in 0..12 {
                    let (x, y) = (c - 290 + i * 48, c - 140 + j * 24);
                    b.boxed(
                        [x, y, c + 20],
                        [x + 40, y + 18, c + 28],
                        metal,
                        FeatureKind::MISC,
                        solid,
                    );
                    b.boxed(
                        [x + 16, y + 8, c + 28],
                        [x + 24, y + 10, c + 30],
                        MaterialId::WARM,
                        FeatureKind::MISC,
                        none,
                    );
                }
            }
            keep = vec![volume(
                [c - 310, c - 160, c - 30],
                [c + 310, c + 160, c + 80],
            )];
        }
        // WON-21: a district and its mirror image across a plane.
        Wonder::Mirror => {
            let mut towers = Vec::new();
            for _ in 0..8 {
                let (x, z) = (20 + rng.below(280) as i32, c - 300 + rng.below(560) as i32);
                let (w, d, h) = (
                    30 + rng.below(40) as i32,
                    30 + rng.below(40) as i32,
                    60 + rng.below(200) as i32,
                );
                towers.push((x, z, w, d, h));
            }
            for (x, z, w, d, h) in towers {
                for sign in [1, -1] {
                    let (lo, hi) = if sign > 0 {
                        (c + x, c + x + w)
                    } else {
                        (c - x - w, c - x)
                    };
                    b.boxed(
                        [lo, c - 40, z],
                        [hi, c - 40 + h, z + d],
                        conc,
                        FeatureKind::MASS,
                        far | solid,
                    );
                }
            }
            b.boxed(
                [c - 2, c - 80, c - 320],
                [c + 2, c + 260, c + 320],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                far | none,
            );
            keep = vec![volume(
                [c - 360, c - 80, c - 340],
                [c + 360, c + 270, c + 340],
            )];
        }
        // WON-22: a colossal machine lying down: a body, a head, six arms, four legs.
        Wonder::SleepingBuilder => {
            b.boxed(
                [c - 150, 60, c - 30],
                [c + 150, 100, c + 30],
                metal,
                FeatureKind::MASS,
                far | solid,
            );
            b.boxed(
                [c + 150, 50, c - 25],
                [c + 210, 110, c + 25],
                metal,
                FeatureKind::MASS,
                far | solid,
            );
            b.boxed(
                [c + 190, 80, c - 20],
                [c + 200, 84, c - 10],
                MaterialId::WARM,
                FeatureKind::MISC,
                none,
            );
            for k in 0..6i32 {
                let x = c - 130 + k * 50;
                let sign = if k % 2 == 0 { 1 } else { -1 };
                b.turned(
                    [x, 70, c + sign * 30],
                    [90, 8, 8],
                    if sign > 0 { 16 } else { 48 },
                    0,
                    metal,
                    solid,
                );
            }
            for (x, z) in [(-120, -30), (-120, 20), (100, -30), (100, 20)] {
                b.boxed(
                    [c + x, 0, c + z],
                    [c + x + 14, 60, c + z + 14],
                    metal,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep = vec![volume([c - 160, 0, c - 120], [c + 220, 120, c + 120])];
        }
        // WON-23: a funnel: twenty rings from 80 m to 1 900 m, widening up.
        Wonder::Funnel => {
            for k in 0..20 {
                let radius = 80 + 95 * k;
                b.out.extend(facet_ring(
                    &FacetRing {
                        axis: QVec3::new(c * M, (400 + k * 170) * M, c * M),
                        radius: radius * M,
                        thick: 32 * M,
                        height: 24 * M,
                        n: 16,
                        start: 0,
                        material: conc,
                        kind: FeatureKind::MASS,
                        flags: far | solid,
                    },
                    clip,
                ));
            }
            keep = vec![volume(
                [c - 1950, 380, c - 1950],
                [c + 1950, 3900, c + 1950],
            )];
        }
        Wonder::Drop | Wonder::Gate => {}
    }
    let _: Q = 0;
    Content {
        prims: b.out,
        keep_out: keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: Wonder) -> Site {
        for x in -90..90 {
            for z in -90..90 {
                if let Some(s) = site_of(SEED, I64Vec3::new(x, 0, z)).filter(|s| s.kind == kind) {
                    return s;
                }
            }
        }
        panic!("{kind:?} not found in 32 400 L0 cells");
    }

    #[test]
    fn wonders_occur_about_one_cell_in_twelve() {
        let (mut n, mut cells) = (0, 0);
        for x in -30..30 {
            for z in -30..30 {
                cells += 1;
                if let Some(s) = site_of(SEED, I64Vec3::new(x, 1, z)) {
                    n += 1;
                    assert!(s.kind.built());
                    assert!((s.l1 - I64Vec3::new(x, 1, z) * 8).min_element() >= 1);
                }
            }
        }
        assert!((cells / 20..cells / 6).contains(&n), "{n} of {cells}");
    }

    /// Every built wonder exists, is inside its cell and shows from afar.
    #[test]
    fn every_wonder_builds_inside_its_cell_and_shows_from_afar() {
        for kind in ALL.into_iter().filter(|w| w.built()) {
            let s = find(kind);
            let c = wonder_content(SEED, s.l1);
            assert!(!c.prims.is_empty(), "{kind:?} {}", kind.id());
            assert!(!c.keep_out.is_empty());
            let far = c
                .prims
                .iter()
                .filter(|p| p.flags.contains(PrimFlags::FAR_VISIBLE))
                .count();
            assert!(far > 0, "{kind:?} has nothing for the far band");
            for p in &c.prims {
                let b = p.aabb();
                assert!(
                    b.min.x >= -64 && b.min.y >= -64 && b.min.z >= -64,
                    "{kind:?} {b:?}"
                );
                assert!(
                    b.max.x <= CELL + 64 && b.max.y <= CELL + 64 && b.max.z <= CELL + 64,
                    "{kind:?} {b:?}"
                );
            }
            assert_eq!(wonder_content(SEED, s.l1).prims.len(), c.prims.len());
        }
        assert_eq!(wonder_content(SEED, I64Vec3::new(0, 0, 0)).prims.len(), 0);
    }

    #[test]
    fn the_cistern_has_ten_thousand_columns() {
        let s = find(Wonder::Cistern);
        let c = wonder_content(SEED, s.l1);
        assert!(c.prims.len() >= 10_002);
    }
}
