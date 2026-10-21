//! The Web's rare events (RARE-01..14, `docs/proposals/the-web.md` §12). Spec:
//! `docs/spec/levels.md` §3W.5.
//!
//! One L0 cell in six holds one rare event, at an inner L1 cell. Four are **districts** that
//! change how the tissue of the site's L1 cell and its 26 neighbours grows (inverted, scale
//! break, dollhouse, shattered grain); three **steer the wells** (planetoid, Möbius corridor,
//! quiet lull); the rest are **geometry** of the site's L1 cell (`rare_content`).

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::hash;
use crate::l1::CELL;
use crate::orient::{FacetRing, Frame, HelixStair, LocalShape, facet_ring, helix_stair};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// An L0 cell holds an event with chance 1 in `ONE_IN`.
const ONE_IN: u64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rare {
    /// RARE-01: everything hangs.
    Inverted,
    /// RARE-02: one section of corridor repeated for the whole L1 cell.
    Repeating,
    /// RARE-03: tissue at the largest scale (×4).
    ScaleBreak,
    /// RARE-04: tissue at half scale.
    Dollhouse,
    /// RARE-05: a hollow trunk with a helix of stairs inside.
    HollowTrunk,
    /// RARE-06: a half-built frame with scaffolds.
    BuilderSite,
    /// RARE-07: a void crossed only by catwalks and cables.
    TheWeb,
    /// RARE-08: one enormous light.
    LampSun,
    /// RARE-09: parked trams in heaps.
    TramGraveyard,
    /// RARE-10: a strong point well with a whole small world.
    Planetoid,
    /// RARE-11: a fold well that returns you upside down.
    Moebius,
    /// RARE-12: rooms at different angles.
    Shattered,
    /// RARE-13: a fracture showing a thousand floors.
    SectionCliff,
    /// RARE-14: a weightless chamber with one handrail tube.
    QuietLull,
}

pub const ALL: [Rare; 14] = [
    Rare::Inverted,
    Rare::Repeating,
    Rare::ScaleBreak,
    Rare::Dollhouse,
    Rare::HollowTrunk,
    Rare::BuilderSite,
    Rare::TheWeb,
    Rare::LampSun,
    Rare::TramGraveyard,
    Rare::Planetoid,
    Rare::Moebius,
    Rare::Shattered,
    Rare::SectionCliff,
    Rare::QuietLull,
];

impl Rare {
    /// The registry ID.
    pub const fn id(self) -> &'static str {
        match self {
            Rare::Inverted => "RARE-01",
            Rare::Repeating => "RARE-02",
            Rare::ScaleBreak => "RARE-03",
            Rare::Dollhouse => "RARE-04",
            Rare::HollowTrunk => "RARE-05",
            Rare::BuilderSite => "RARE-06",
            Rare::TheWeb => "RARE-07",
            Rare::LampSun => "RARE-08",
            Rare::TramGraveyard => "RARE-09",
            Rare::Planetoid => "RARE-10",
            Rare::Moebius => "RARE-11",
            Rare::Shattered => "RARE-12",
            Rare::SectionCliff => "RARE-13",
            Rare::QuietLull => "RARE-14",
        }
    }

    /// Districts spread over the site's L1 cell and its neighbours.
    pub const fn is_district(self) -> bool {
        matches!(
            self,
            Rare::Inverted | Rare::ScaleBreak | Rare::Dollhouse | Rare::Shattered
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub kind: Rare,
    pub l1: I64Vec3,
}

/// The rare event of L0 cell `l0`, if any (a function of the cell only).
pub fn site_of(seed: WorldSeed, l0: I64Vec3) -> Option<Site> {
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::RARE_SITE));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    let kind = ALL[rng.below(ALL.len() as u64) as usize];
    let inner = |r: &mut Rng| 1 + r.below(6) as i64;
    let l1 = l0 * 8 + I64Vec3::new(inner(&mut rng), inner(&mut rng), inner(&mut rng));
    Some(Site { kind, l1 })
}

/// The rare event that touches L1 cell `l1`: its own site, or a district that reaches it.
pub fn rare_at(seed: WorldSeed, l1: I64Vec3) -> Option<Rare> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let site = site_of(seed, l0)?;
    let d = (l1 - site.l1).abs().max_element();
    if d == 0 || (d <= 1 && site.kind.is_district()) {
        Some(site.kind)
    } else {
        None
    }
}

/// What the geometry events add to their L1 cell, in cell-local quanta, and the space they keep
/// clear of everything else.
#[derive(Debug, Default)]
pub struct Content {
    pub prims: Vec<Primitive>,
    pub keep_out: Vec<QAabb>,
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
        let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
        if (0..3).any(|i| lo[i] >= hi[i]) {
            return;
        }
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

fn square(centre: i32, half: i32) -> (i32, i32) {
    (centre - half, centre + half)
}

fn volume(lo: [i32; 3], hi: [i32; 3]) -> QAabb {
    QAabb::new(
        QVec3::new(lo[0] * M, lo[1] * M, lo[2] * M),
        QVec3::new(hi[0] * M, hi[1] * M, hi[2] * M),
    )
}

/// The geometry of a rare event at L1 cell `l1` (empty for the other kinds).
pub fn rare_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let Some(site) = site_of(seed, l0).filter(|s| s.l1 == l1) else {
        return Content::default();
    };
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::RARE_BUILD));
    let mut b = B { out: Vec::new() };
    let mut keep_out = Vec::new();
    let c = 2048;
    let far = PrimFlags::FAR_VISIBLE;
    let solid = PrimFlags::NONE;
    let none = PrimFlags::NO_COLLIDE;
    match site.kind {
        Rare::HollowTrunk => {
            let axis = QVec3::new(c * M, 0, c * M);
            let clip = volume([0; 3], [4096; 3]);
            b.out.extend(facet_ring(
                &FacetRing {
                    axis,
                    radius: 80 * M,
                    thick: 32 * M,
                    height: CELL,
                    n: 16,
                    start: 0,
                    material: MaterialId::CONCRETE,
                    kind: FeatureKind::MASS,
                    flags: far,
                },
                clip,
            ));
            // 64 steps a turn, 0.4375 m each: 28 m a turn, whole turns up the cell.
            let turns = CELL / (64 * 7);
            b.out.extend(helix_stair(
                &HelixStair {
                    axis,
                    inner: 48 * M,
                    width: 24 * M,
                    per_turn: 64,
                    steps: (turns * 64) as u32,
                    rise: 7,
                    thick: 2,
                    start: 0,
                    sense: 1,
                    material: MaterialId::METAL,
                    kind: FeatureKind::STAIR_TOWER,
                    flags: solid,
                },
                clip,
            ));
            keep_out.push(volume([c - 112, 0, c - 112], [c + 112, 4096, c + 112]));
        }
        Rare::BuilderSite => {
            let (x0, x1) = square(c, 240);
            let posts = [(x0, x0), (x1 - 16, x0), (x0, x1 - 16), (x1 - 16, x1 - 16)];
            for (px, pz) in posts {
                let h = 288 + 32 * rng.below(7) as i32;
                b.boxed(
                    [px, c - 240, pz],
                    [px + 16, c - 240 + h, pz + 16],
                    MaterialId::METAL,
                    FeatureKind::TETHER,
                    far | solid,
                );
            }
            // Beams between the posts at every 96 m, each present half the time.
            for level in 0..4 {
                let y = c - 240 + 96 * (level + 1);
                for (a, bb) in [(0, 1), (1, 3), (3, 2), (2, 0)] {
                    if rng.below(2) == 0 {
                        continue;
                    }
                    let (pa, pb) = (posts[a], posts[bb]);
                    b.boxed(
                        [pa.0.min(pb.0), y, pa.1.min(pb.1)],
                        [pa.0.max(pb.0) + 16, y + 8, pa.1.max(pb.1) + 16],
                        MaterialId::METAL,
                        FeatureKind::TETHER,
                        solid,
                    );
                }
            }
            // A scaffold up the front: poles every 16 m and a deck every 32 m.
            for k in 0..30 {
                b.boxed(
                    [x0 + 16 * k, c - 240, x0 - 12],
                    [x0 + 16 * k + 1, c + 48, x0 - 11],
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
            }
            for k in 0..9 {
                let y = c - 240 + 32 * (k + 1);
                b.boxed(
                    [x0, y, x0 - 12],
                    [x0 + 480, y + 1, x0],
                    MaterialId::GRATING,
                    FeatureKind::WALKWAY,
                    solid,
                );
            }
            // The builder's crane: a boom on a mast.
            b.boxed(
                [c - 8, c + 48, c - 8],
                [c + 8, c + 144, c + 8],
                MaterialId::METAL,
                FeatureKind::MISC,
                solid,
            );
            b.boxed(
                [c - 120, c + 144, c - 4],
                [c + 120, c + 152, c + 4],
                MaterialId::METAL,
                FeatureKind::MISC,
                solid,
            );
            keep_out.push(volume([x0, c - 240, x0 - 16], [x1, c + 160, x1]));
        }
        Rare::TheWeb => {
            let (v0, v1) = (c - 512, c + 512);
            keep_out.push(volume([v0, v0, v0], [v1, v1, v1]));
            for i in 0..10 {
                let y = v0 + 64 + 88 * i;
                let across = v0 + 64 + rng.below(896) as i32;
                if i % 2 == 0 {
                    b.boxed(
                        [v0 - 16, y, across],
                        [v1 + 16, y + 1, across + 3],
                        MaterialId::GRATING,
                        FeatureKind::WALKWAY,
                        solid,
                    );
                    b.boxed(
                        [v0 - 16, y + 1, across],
                        [v1 + 16, y + 2, across + 1],
                        MaterialId::METAL,
                        FeatureKind::RAILING,
                        solid,
                    );
                    b.boxed(
                        [v0 - 16, y + 1, across + 2],
                        [v1 + 16, y + 2, across + 3],
                        MaterialId::METAL,
                        FeatureKind::RAILING,
                        solid,
                    );
                } else {
                    b.boxed(
                        [across, y, v0 - 16],
                        [across + 3, y + 1, v1 + 16],
                        MaterialId::GRATING,
                        FeatureKind::WALKWAY,
                        solid,
                    );
                    b.boxed(
                        [across, y + 1, v0 - 16],
                        [across + 1, y + 2, v1 + 16],
                        MaterialId::METAL,
                        FeatureKind::RAILING,
                        solid,
                    );
                    b.boxed(
                        [across + 2, y + 1, v0 - 16],
                        [across + 3, y + 2, v1 + 16],
                        MaterialId::METAL,
                        FeatureKind::RAILING,
                        solid,
                    );
                }
            }
            // Cables, vertical, from the ceiling to the floor of the void.
            for _ in 0..14 {
                let (x, z) = (
                    v0 + 32 + rng.below(960) as i32,
                    v0 + 32 + rng.below(960) as i32,
                );
                b.boxed(
                    [x, v0, z],
                    [x + 1, v1, z + 1],
                    MaterialId::METAL,
                    FeatureKind::CABLE,
                    none,
                );
            }
        }
        Rare::LampSun => {
            // A cube of light 160 m across with four spikes, visible for tens of kilometres.
            b.boxed(
                [c - 80, c - 80, c - 80],
                [c + 80, c + 80, c + 80],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                far | none,
            );
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (x, z) = (c + dx * 140, c + dz * 140);
                b.boxed(
                    [x - 24, c - 24, z - 24],
                    [x + 24, c + 24, z + 24],
                    MaterialId::BEACON,
                    FeatureKind::MISC,
                    far | none,
                );
            }
            keep_out.push(volume(
                [c - 100, c - 100, c - 100],
                [c + 100, c + 100, c + 100],
            ));
        }
        Rare::TramGraveyard => {
            let clip = volume([0; 3], [4096; 3]);
            keep_out.push(volume(
                [c - 400, c - 40, c - 400],
                [c + 400, c + 40, c + 400],
            ));
            for _ in 0..48 {
                let (x, z) = (
                    c - 380 + rng.below(760) as i32,
                    c - 380 + rng.below(760) as i32,
                );
                let y = c - 30 + rng.below(40) as i32;
                let (yaw, tilt) = (rng.below(64) as u8, rng.below(3) as u8);
                let body = QAabb::new(
                    QVec3::new(0, 0, 0),
                    QVec3::new(24 * M, 3 * M + 4, 3 * M + 4),
                );
                b.out.push(Primitive {
                    shape: Shape::oriented(
                        Frame::table(yaw, tilt),
                        QVec3::new(x * M, y * M, z * M),
                        LocalShape::Box { aabb: body },
                        clip,
                    ),
                    material: MaterialId::METAL,
                    kind: FeatureKind::TRAM,
                    flags: solid,
                });
            }
        }
        Rare::SectionCliff => {
            // The back wall, a thousand floors cantilevered out of it, and the columns.
            b.boxed(
                [c - 132, 0, 1024],
                [c - 128, 4096, 3072],
                MaterialId::CONCRETE,
                FeatureKind::MASS,
                far | solid,
            );
            for k in 0..1024 {
                let y = 4 * k;
                b.boxed(
                    [c - 128, y, 1024],
                    [c, y + 1, 3072],
                    MaterialId::CONCRETE,
                    FeatureKind::DECK,
                    solid,
                );
            }
            for k in 0..9 {
                let z = 1040 + 240 * k;
                b.boxed(
                    [c - 8, 0, z],
                    [c - 4, 4096, z + 4],
                    MaterialId::METAL,
                    FeatureKind::MASS,
                    far | solid,
                );
            }
            keep_out.push(volume([c - 140, 0, 1024], [c + 700, 4096, 3072]));
        }
        Rare::QuietLull => {
            // A hollow cube of 128 m half size with 16 m walls, and a square tube across it.
            let h = 128;
            let w = 16;
            for a in 0..3 {
                for sign in [-1, 1] {
                    let mut lo = [0i32; 3];
                    let mut hi = [0i32; 3];
                    for i in 0..3 {
                        let inset = if i < a { w } else { 0 };
                        lo[i] = c - h + inset;
                        hi[i] = c + h - inset;
                    }
                    if sign < 0 {
                        hi[a] = c - h + w;
                    } else {
                        lo[a] = c + h - w;
                    }
                    b.boxed(lo, hi, MaterialId::CONCRETE, FeatureKind::MASS, far | solid);
                }
            }
            // The handrail tube along x: four rails of 4 cm.
            for (dy, dz) in [(-2, -2), (2, -2), (-2, 2), (2, 2)] {
                b.out.push(Primitive {
                    shape: Shape::cylinder(
                        crate::cell::Axis::X,
                        QVec3::new((c - h + w) * M, (c + dy) * M, (c + dz) * M),
                        (2 * (h - w)) * M,
                        1,
                    ),
                    material: MaterialId::METAL,
                    kind: FeatureKind::RAILING,
                    flags: none,
                });
            }
            b.boxed(
                [c - 1, c + h - w - 2, c - 1],
                [c + 1, c + h - w, c + 1],
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
                none,
            );
            keep_out.push(volume([c - h, c - h, c - h], [c + h, c + h, c + h]));
        }
        Rare::Repeating => {
            // One section of corridor, 6 m wide and 4 m tall, repeated every 8 m for the
            // whole cell, with the same frame in every section.
            let (y, z) = (c, c);
            b.boxed(
                [0, y - 1, z - 3],
                [4096, y, z + 3],
                MaterialId::CONCRETE,
                FeatureKind::WALKWAY,
                solid,
            );
            b.boxed(
                [0, y + 4, z - 3],
                [4096, y + 5, z + 3],
                MaterialId::CONCRETE,
                FeatureKind::MASS,
                solid,
            );
            b.boxed(
                [0, y, z - 4],
                [4096, y + 4, z - 3],
                MaterialId::CONCRETE,
                FeatureKind::MASS,
                solid,
            );
            b.boxed(
                [0, y, z + 3],
                [4096, y + 4, z + 4],
                MaterialId::CONCRETE,
                FeatureKind::MASS,
                solid,
            );
            for k in 0..512 {
                let x = 8 * k + 4;
                b.boxed(
                    [x, y, z - 3],
                    [x + 1, y + 4, z - 2],
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
                b.boxed(
                    [x, y, z + 2],
                    [x + 1, y + 4, z + 3],
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
                b.boxed(
                    [x, y + 3, z - 3],
                    [x + 1, y + 4, z + 3],
                    MaterialId::EMISSIVE,
                    FeatureKind::MISC,
                    none,
                );
            }
            keep_out.push(volume([0, y - 8, z - 8], [4096, y + 12, z + 8]));
        }
        _ => {}
    }
    Content {
        prims: b.out,
        keep_out,
    }
}

/// The L2 cells of L1 cell `l1` that hold or touch the space a rare event keeps clear, for the
/// artery tree to reach (an event with no geometry has none).
pub fn tie_cells(seed: WorldSeed, l1: I64Vec3) -> Vec<[i64; 3]> {
    let content = rare_content(seed, l1);
    let l2 = |q: i32| i64::from(q / M).div_euclid(512);
    let mut cells = Vec::new();
    for k in &content.keep_out {
        let lo = [k.min.x, k.min.y, k.min.z].map(|v| (l2(v) - 1).max(0));
        let hi = [k.max.x, k.max.y, k.max.z].map(|v| (l2(v - 1) + 1).min(7));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    if !cells.contains(&[x, y, z]) {
                        cells.push([x, y, z]);
                    }
                }
            }
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: Rare) -> Site {
        for x in -40..40 {
            for z in -40..40 {
                if let Some(s) = site_of(SEED, I64Vec3::new(x, 0, z)).filter(|s| s.kind == kind) {
                    return s;
                }
            }
        }
        panic!("{kind:?} not found in 6400 L0 cells");
    }

    #[test]
    fn every_event_occurs_about_one_cell_in_eighty_four() {
        let mut counts = [0usize; 14];
        let mut cells = 0;
        for x in -30..30 {
            for z in -30..30 {
                cells += 1;
                if let Some(s) = site_of(SEED, I64Vec3::new(x, 1, z)) {
                    let k = ALL.iter().position(|&a| a == s.kind).unwrap();
                    counts[k] += 1;
                    // Inner cell: a district stays in its L0 cell.
                    let l0 = I64Vec3::new(x, 1, z);
                    assert!(
                        (s.l1 - l0 * 8).min_element() >= 1 && (s.l1 - l0 * 8).max_element() <= 6
                    );
                }
            }
        }
        assert!(counts.iter().all(|&n| n > 0), "{counts:?} in {cells} cells");
        let total: usize = counts.iter().sum();
        assert!(
            (cells / 8..cells / 4).contains(&total),
            "{total} of {cells}"
        );
    }

    #[test]
    fn districts_reach_their_neighbours() {
        let s = find(Rare::Inverted);
        assert_eq!(rare_at(SEED, s.l1), Some(Rare::Inverted));
        assert_eq!(
            rare_at(SEED, s.l1 + I64Vec3::new(1, 0, -1)),
            Some(Rare::Inverted)
        );
        assert_eq!(rare_at(SEED, s.l1 + I64Vec3::new(2, 0, 0)), None);
        let t = find(Rare::LampSun);
        assert_eq!(rare_at(SEED, t.l1 + I64Vec3::X), None);
    }

    #[test]
    fn geometry_events_build_inside_their_cell() {
        for kind in [
            Rare::HollowTrunk,
            Rare::BuilderSite,
            Rare::TheWeb,
            Rare::LampSun,
            Rare::TramGraveyard,
            Rare::SectionCliff,
            Rare::QuietLull,
            Rare::Repeating,
        ] {
            let s = find(kind);
            let c = rare_content(SEED, s.l1);
            assert!(!c.prims.is_empty(), "{kind:?}");
            assert!(!c.keep_out.is_empty(), "{kind:?}");
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
            assert_eq!(rare_content(SEED, s.l1).prims.len(), c.prims.len());
        }
        // The other kinds add no geometry of their own.
        assert!(
            rare_content(SEED, find(Rare::Dollhouse).l1)
                .prims
                .is_empty()
        );
    }

    fn tissue_at(l1: I64Vec3, k: i64) -> crate::tissue::Tissue {
        let cache = crate::tissue::TissueCache::default();
        let l2 = l1 * 8 + I64Vec3::new(2 + k % 4, 2 + k / 4 % 4, 2 + k / 16 % 4);
        crate::tissue::tissue_cell(SEED, &cache, crate::cell::CellKey::new(Level::L2, l2))
    }

    /// RARE-01: in an inverted district everything hangs.
    #[test]
    fn an_inverted_district_hangs() {
        use crate::tissue_grow::{HANGING, SUSPENDED};
        let s = find(Rare::Inverted);
        let (mut hang, mut other) = (0, 0);
        for k in 0..32 {
            let t = tissue_at(s.l1, k);
            assert_eq!(t.rare, Some(Rare::Inverted));
            for st in &t.structures {
                if usize::from(st.support) == HANGING || usize::from(st.support) == SUSPENDED {
                    hang += 1;
                } else {
                    other += 1;
                }
            }
        }
        assert!(hang > 20, "{hang} hanging structures");
        assert!(
            other * 5 <= hang + other,
            "{other} of {} do not hang",
            hang + other
        );
    }

    /// RARE-03 and RARE-04: the scale of the whole district.
    #[test]
    fn scale_break_and_dollhouse_set_the_scale() {
        for (kind, scale) in [(Rare::ScaleBreak, 6), (Rare::Dollhouse, 0)] {
            let s = find(kind);
            let mut seen = 0;
            for k in 0..32 {
                let t = tissue_at(s.l1, k);
                if let Some(c) = t.character {
                    assert_eq!(c.scale, scale, "{kind:?}");
                    seen += 1;
                }
            }
            assert!(seen > 0, "{kind:?} has no crust in the sample");
        }
    }

    /// RARE-12: in a shattered district boxes are turned, each by its own frame.
    #[test]
    fn shattered_grain_turns_the_rooms() {
        let s = find(Rare::Shattered);
        let mut turned = 0;
        let mut frames = std::collections::HashSet::new();
        for k in 0..32 {
            let t = tissue_at(s.l1, k);
            if !t.lattice.frame.is_axis_aligned() {
                continue;
            }
            for st in &t.structures {
                for p in &st.oriented {
                    turned += 1;
                    frames.insert(format!("{:?}", p.frame));
                }
            }
        }
        assert!(
            turned > 20 && frames.len() > 10,
            "{turned} turned boxes, {} frames",
            frames.len()
        );
    }

    /// RARE-10, 11, 14: the wells.
    #[test]
    fn the_rare_wells_shape_the_field() {
        use crate::gravity::gravity_at_metres;
        let at = |s: &Site| [0, 1, 2].map(|i| s.l1[i] * 4096 + 2048);
        let p = find(Rare::Planetoid);
        let c = at(&p);
        assert_eq!(
            gravity_at_metres(SEED, [c[0] + 3000, c[1], c[2]]).up,
            [8, 0, 0]
        );
        assert_eq!(
            gravity_at_metres(SEED, [c[0], c[1] - 3000, c[2]]).up,
            [0, -8, 0]
        );

        let q = find(Rare::QuietLull);
        let c = at(&q);
        assert!(
            gravity_at_metres(SEED, c).is_lull(),
            "the chamber is a lull"
        );
        assert!(!gravity_at_metres(SEED, [c[0] + 1500, c[1], c[2]]).is_lull());

        let m = find(Rare::Moebius);
        let c = at(&m);
        // The loop is 1.2 km on a side from (c - 600, c - 600): its first leg runs along +x
        // upright, its third (at z = c + 600) along -x upside down.
        let first = gravity_at_metres(SEED, [c[0], c[1], c[2] - 600]);
        let third = gravity_at_metres(SEED, [c[0], c[1], c[2] + 600]);
        assert_eq!(first.up, [0, 8, 0], "{first:?}");
        assert_eq!(third.up, [0, -8, 0], "{third:?}");
    }

    /// The geometry events cut their space out of the cell and keep their children out of it.
    #[test]
    fn a_rare_cell_has_its_space_clear() {
        use crate::l1_web::l1_cell;
        use crate::prim::ChildConstraint;
        for kind in [
            Rare::TheWeb,
            Rare::HollowTrunk,
            Rare::Repeating,
            Rare::SectionCliff,
        ] {
            let s = find(kind);
            let out = l1_cell(SEED, crate::cell::CellKey::new(Level::L1, s.l1));
            let content = rare_content(SEED, s.l1);
            for k in &content.keep_out {
                assert!(
                    out.child_constraints
                        .contains(&ChildConstraint::KeepOut { aabb: *k }),
                    "{kind:?}"
                );
            }
            let own: std::collections::HashSet<_> = content.prims.iter().copied().collect();
            for p in out.primitives.iter().filter(|p| !own.contains(*p)) {
                for k in &content.keep_out {
                    assert!(
                        !crate::l1_web::enters(p, k),
                        "{kind:?}: a piece of {:?} stands in the cleared space",
                        p.kind
                    );
                }
            }
        }
    }
}
