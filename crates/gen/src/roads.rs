//! Great Roads, the Rail Abyss and the Cable Highway (RD-01..08, `docs/proposals/the-web-ideas.md`
//! §3). Spec: `docs/spec/levels.md` §3W.6.
//!
//! A road is a function of an **L0 line**: the row `(y, z)` of L0 cells for a road along x, the
//! row `(y, x)` for one along z. A line holds a road with chance 1 in `ONE_IN`, and the road
//! then runs the whole length of the row (every L0 cell of it), at one L1 row inside the L0
//! cell, one height and one width, so every L1 cell on the line derives the same road and the
//! deck meets itself at every cell face.

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
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// A line of L0 cells holds a road with chance 1 in this.
const ONE_IN: u64 = 10;
const KIND_WEIGHTS: [u32; 6] = [4, 2, 2, 2, 1, 1];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoadKind {
    /// RD-02: lit by a lamp every 50 m.
    Lantern,
    /// RD-03: the same posts, unlit.
    Dark,
    /// RD-04: shrines with a candle every 256 m.
    Pilgrim,
    /// RD-05: wide, with rails inlaid and gantries.
    Freight,
    /// RD-07: a tram line on a trestle across a Between.
    RailAbyss,
    /// RD-08: a gondola line on pylons.
    CableHighway,
}

pub const KINDS: [RoadKind; 6] = [
    RoadKind::Lantern,
    RoadKind::Dark,
    RoadKind::Pilgrim,
    RoadKind::Freight,
    RoadKind::RailAbyss,
    RoadKind::CableHighway,
];

impl RoadKind {
    pub const fn id(self) -> &'static str {
        match self {
            RoadKind::Lantern => "RD-02",
            RoadKind::Dark => "RD-03",
            RoadKind::Pilgrim => "RD-04",
            RoadKind::Freight => "RD-05",
            RoadKind::RailAbyss => "RD-07",
            RoadKind::CableHighway => "RD-08",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Road {
    /// The axis it runs along: 0 for x, 2 for z.
    pub axis: usize,
    pub kind: RoadKind,
    /// Deck top in metres in the L1 cell, and the road's centre across, in metres.
    pub height: i32,
    pub across: i32,
    /// Deck width in metres.
    pub width: i32,
}

/// The road of line `key` along `axis`, if any, with the inner L1 row `(a, b)` it takes in each
/// L0 cell (`a` across the first row coordinate, `b` across the second).
fn line(seed: WorldSeed, axis: usize, row: (i64, i64)) -> Option<(Road, (i64, i64))> {
    let key = I64Vec3::new(row.0, row.1, axis as i64);
    let mut rng = Rng::new(hash(seed.0, Level::L0, key, salt::ROAD_LINE));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    let kind = KINDS[rng.pick_weighted(&KIND_WEIGHTS).unwrap_or(0)];
    let (a, b) = (1 + rng.below(6) as i64, 1 + rng.below(6) as i64);
    let height = 1024 + 128 * rng.below(16) as i32;
    let width = match kind {
        RoadKind::Freight => 24,
        RoadKind::Pilgrim | RoadKind::RailAbyss => 12,
        RoadKind::CableHighway => 8,
        _ => 16,
    };
    Some((
        Road {
            axis,
            kind,
            height,
            across: 2048,
            width,
        },
        (a, b),
    ))
}

/// The roads through L1 cell `l1`: at most one along x and one along z.
pub fn roads_in_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<Road> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let inner = l1 - l0 * 8;
    let mut out = Vec::new();
    // Along x: the row is (y, z); the cell is on it if its (y, z) is the line's inner row.
    if let Some((road, (a, b))) = line(seed, 0, (l0.y, l0.z))
        && (inner.y, inner.z) == (a, b)
    {
        out.push(road);
    }
    // Along z: the row is (y, x).
    if let Some((road, (a, b))) = line(seed, 2, (l0.y, l0.x))
        && (inner.y, inner.x) == (a, b)
    {
        out.push(road);
    }
    out
}

struct B {
    out: Vec<Primitive>,
    axis: usize,
}

impl B {
    /// A box from `a0..a1` along the road, `c0..c1` across it and `y0..y1` high, in metres.
    fn at(
        &mut self,
        a: (i32, i32),
        c: (i32, i32),
        y: (i32, i32),
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        let (lo, hi) = if self.axis == 0 {
            ([a.0, y.0, c.0], [a.1, y.1, c.1])
        } else {
            ([c.0, y.0, a.0], [c.1, y.1, a.1])
        };
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

/// The geometry of `road` in the L1 cell `l1`.
fn build(road: Road, l1: I64Vec3) -> Content {
    let mut b = B {
        out: Vec::new(),
        axis: road.axis,
    };
    let (c, h, w) = (road.across, road.height, road.width);
    let (c0, c1) = (c - w / 2, c + w / 2);
    let solid = PrimFlags::NONE;
    let none = PrimFlags::NO_COLLIDE;
    let far = PrimFlags::FAR_VISIBLE;
    // The deck, from face to face, and its low rails.
    b.at(
        (0, 4096),
        (c0, c1),
        (h - 1, h),
        MaterialId::CONCRETE,
        FeatureKind::WALKWAY,
        far | solid,
    );
    for (e0, e1) in [(c0, c0 + 1), (c1 - 1, c1)] {
        b.at(
            (0, 4096),
            (e0, e1),
            (h, h + 1),
            MaterialId::METAL,
            FeatureKind::RAILING,
            solid,
        );
    }
    // Piers under the deck: every 256 m down to 64 m (a trestle's are 160 m).
    let pier = if road.kind == RoadKind::RailAbyss {
        160
    } else {
        64
    };
    let spacing = if road.kind == RoadKind::RailAbyss {
        128
    } else {
        256
    };
    let mut a = 128;
    while a < 4096 {
        b.at(
            (a - 4, a + 4),
            (c - 4, c + 4),
            (h - pier, h - 1),
            MaterialId::CONCRETE,
            FeatureKind::MASS,
            solid,
        );
        a += spacing;
    }
    // RD-06: lamp posts every 50 m on both edges; unlit on the Dark Road.
    let lamp = if road.kind == RoadKind::Dark {
        MaterialId::METAL
    } else {
        MaterialId::EMISSIVE
    };
    if !matches!(road.kind, RoadKind::CableHighway) {
        let mut a = 25;
        while a < 4096 {
            for edge in [c0 + 1, c1 - 2] {
                b.at(
                    (a, a + 1),
                    (edge, edge + 1),
                    (h, h + 5),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
                b.at(
                    (a - 1, a + 2),
                    (edge - 1, edge + 2),
                    (h + 5, h + 6),
                    lamp,
                    FeatureKind::MISC,
                    none,
                );
            }
            a += 50;
        }
    }
    // RD-06: a rest niche every 2 km (at 512 m and 2560 m in a cell).
    for at in [512, 2560] {
        b.at(
            (at - 8, at + 8),
            (c1, c1 + 12),
            (h - 1, h),
            MaterialId::CONCRETE,
            FeatureKind::WALKWAY,
            solid,
        );
        b.at(
            (at - 8, at + 8),
            (c1 + 11, c1 + 12),
            (h, h + 4),
            MaterialId::CONCRETE,
            FeatureKind::MASS,
            solid,
        );
        b.at(
            (at - 6, at + 6),
            (c1 + 9, c1 + 10),
            (h, h + 1),
            MaterialId::METAL,
            FeatureKind::MISC,
            solid,
        );
    }
    // RD-06: a way-house every 12 km (every third cell along the road): a hall of 32 m with
    // a door on the road side and a lamp over it.
    let along = if road.axis == 0 { l1.x } else { l1.z };
    if along.rem_euclid(3) == 1 {
        let (a0, a1, hc0, hc1) = (2032, 2064, c1 + 12, c1 + 36);
        b.at(
            (a0, a1),
            (hc0, hc1),
            (h - 1, h),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0, a1),
            (hc0, hc1),
            (h + 8, h + 9),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0, a0 + 2),
            (hc0, hc1),
            (h, h + 8),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a1 - 2, a1),
            (hc0, hc1),
            (h, h + 8),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0 + 2, a1 - 2),
            (hc1 - 2, hc1),
            (h, h + 8),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0 + 2, a1 - 2),
            (hc0, hc0 + 2),
            (h, h + 3),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0 + 2, a1 - 2),
            (hc0, hc0 + 2),
            (h + 6, h + 8),
            MaterialId::CONCRETE,
            FeatureKind::SANCTUARY,
            solid,
        );
        b.at(
            (a0 + 14, a0 + 18),
            (hc0 + 4, hc0 + 5),
            (h + 4, h + 5),
            MaterialId::WARM,
            FeatureKind::SANCTUARY,
            none,
        );
    }
    match road.kind {
        RoadKind::Pilgrim => {
            // A shrine every 256 m: an arch over the road with a candle.
            let mut a = 64;
            while a < 4096 {
                b.at(
                    (a - 1, a + 1),
                    (c0, c0 + 1),
                    (h, h + 6),
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    solid,
                );
                b.at(
                    (a - 1, a + 1),
                    (c1 - 1, c1),
                    (h, h + 6),
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    solid,
                );
                b.at(
                    (a - 1, a + 1),
                    (c0, c1),
                    (h + 6, h + 7),
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    solid,
                );
                b.at(
                    (a - 1, a),
                    (c - 1, c + 1),
                    (h + 4, h + 5),
                    MaterialId::WARM,
                    FeatureKind::MISC,
                    none,
                );
                a += 256;
            }
        }
        RoadKind::Freight => {
            // Rails inlaid at 5 m from the middle, and a gantry every 512 m.
            for r in [c - 5, c + 5] {
                b.at(
                    (0, 4096),
                    (r, r + 1),
                    (h, h + 1),
                    MaterialId::METAL,
                    FeatureKind::TRAM,
                    none,
                );
            }
            let mut a = 256;
            while a < 4096 {
                b.at(
                    (a - 2, a + 2),
                    (c0 - 2, c0),
                    (h, h + 14),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    solid,
                );
                b.at(
                    (a - 2, a + 2),
                    (c1, c1 + 2),
                    (h, h + 14),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    solid,
                );
                b.at(
                    (a - 2, a + 2),
                    (c0 - 2, c1 + 2),
                    (h + 14, h + 16),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    solid,
                );
                a += 512;
            }
        }
        RoadKind::RailAbyss => {
            // Two tracks of two rails, and a catenary mast every 64 m.
            for r in [c - 3, c - 1, c + 1, c + 3] {
                b.at(
                    (0, 4096),
                    (r, r + 1),
                    (h, h + 1),
                    MaterialId::METAL,
                    FeatureKind::TRAM,
                    none,
                );
            }
            let mut a = 32;
            while a < 4096 {
                b.at(
                    (a, a + 1),
                    (c1 - 1, c1),
                    (h, h + 9),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
                a += 64;
            }
            b.at(
                (0, 4096),
                (c1 - 1, c1),
                (h + 8, h + 9),
                MaterialId::METAL,
                FeatureKind::CABLE,
                none,
            );
        }
        RoadKind::CableHighway => {
            // Pylons every 512 m with a cable on each side at their tips, and a parked cabin
            // every 400 m.
            let mut a = 256;
            while a < 4096 {
                for side in [c - 12, c + 12] {
                    b.at(
                        (a - 4, a + 4),
                        (side - 4, side + 4),
                        (h, h + 60),
                        MaterialId::METAL,
                        FeatureKind::TETHER,
                        far | solid,
                    );
                }
                a += 512;
            }
            for side in [c - 12, c + 12] {
                b.at(
                    (0, 4096),
                    (side, side + 1),
                    (h + 58, h + 59),
                    MaterialId::METAL,
                    FeatureKind::CABLE,
                    none,
                );
            }
            let mut a = 200;
            while a < 4096 {
                b.at(
                    (a, a + 4),
                    (c - 14, c - 10),
                    (h + 52, h + 57),
                    MaterialId::METAL,
                    FeatureKind::MISC,
                    none,
                );
                a += 400;
            }
        }
        RoadKind::Lantern | RoadKind::Dark => {}
    }
    // The space it keeps clear: the deck's width and 40 m above, and the niches' side.
    let keep = if road.axis == 0 {
        QAabb::new(
            QVec3::new(0, (h - 2) * M, (c0 - 2) * M),
            QVec3::new(4096 * M, (h + 40) * M, (c1 + 40) * M),
        )
    } else {
        QAabb::new(
            QVec3::new((c0 - 2) * M, (h - 2) * M, 0),
            QVec3::new((c1 + 40) * M, (h + 40) * M, 4096 * M),
        )
    };
    Content {
        prims: b.out,
        keep_out: vec![keep],
    }
}

/// The geometry of the roads through L1 cell `l1`.
pub fn road_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let mut all = Content::default();
    for road in roads_in_cell(seed, l1) {
        let c = build(road, l1);
        all.prims.extend(c.prims);
        all.keep_out.extend(c.keep_out);
    }
    all
}

/// The quanta of a metre, for callers that place things on the deck.
pub const fn metres(m: i32) -> Q {
    m * M
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::Nav;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: RoadKind) -> (I64Vec3, Road) {
        for y in -3..3 {
            for z in -40..40 {
                if let Some((road, (a, b))) = line(SEED, 0, (y, z)).filter(|(r, _)| r.kind == kind)
                {
                    return (I64Vec3::new(0, y * 8 + a, z * 8 + b), road);
                }
            }
        }
        panic!("{kind:?} not found");
    }

    #[test]
    fn roads_occur_in_every_kind_at_about_one_line_in_ten() {
        let mut counts = [0usize; 6];
        let mut lines = 0;
        for y in -10..10 {
            for z in -30..30 {
                lines += 1;
                if let Some((r, _)) = line(SEED, 0, (y, z)) {
                    counts[KINDS.iter().position(|&k| k == r.kind).unwrap()] += 1;
                }
            }
        }
        assert!(counts.iter().all(|&n| n > 0), "{counts:?}");
        let total: usize = counts.iter().sum();
        assert!(
            (lines / 20..lines / 5).contains(&total),
            "{total} of {lines}"
        );
    }

    /// Every cell on a line derives the same road, and no other cell does.
    #[test]
    fn a_road_runs_the_whole_row() {
        let (l1, road) = find(RoadKind::Lantern);
        for dx in [-17, -8, -1, 0, 1, 7, 8, 25, 40] {
            let cell = l1 + I64Vec3::new(dx, 0, 0);
            let roads = roads_in_cell(SEED, cell);
            assert!(roads.contains(&road), "cell {dx} cells along has no road");
        }
        assert!(!roads_in_cell(SEED, l1 + I64Vec3::new(0, 1, 0)).contains(&road));
        assert!(!roads_in_cell(SEED, l1 + I64Vec3::new(0, 0, 1)).contains(&road));
    }

    /// RD-01: a lit route you can follow: the deck of 24 cells (three L0 cells) is one run of
    /// walkable plates, the same at every face, joined from one end to the other.
    #[test]
    fn a_bot_follows_a_road_across_three_l0_cells() {
        let (l1, road) = find(RoadKind::Lantern);
        let mut prims = Vec::new();
        for k in 0..24i64 {
            let content = road_content(SEED, l1 + I64Vec3::new(k, 0, 0));
            let shift = (k as i32) * 4096 * M;
            for p in content.prims {
                let b = p.aabb();
                if !matches!(p.kind, FeatureKind::WALKWAY) || b.max.x - b.min.x != 4096 * M {
                    continue;
                }
                let moved = QAabb::new(
                    QVec3::new(b.min.x + shift, b.min.y, b.min.z),
                    QVec3::new(b.max.x + shift, b.max.y, b.max.z),
                );
                prims.push(Primitive {
                    shape: Shape::Box { aabb: moved },
                    ..p
                });
            }
        }
        assert_eq!(prims.len(), 24, "one deck per cell");
        // The deck pieces meet end to end at the same height and width.
        for w in prims.windows(2) {
            let (a, b) = (w[0].aabb(), w[1].aabb());
            assert_eq!(a.max.x, b.min.x);
            assert_eq!(
                (a.min.y, a.max.y, a.min.z, a.max.z),
                (b.min.y, b.max.y, b.min.z, b.max.z)
            );
        }
        let nav = Nav::build(&prims);
        assert!(!nav.plates.is_empty());
        let last = nav.plates.len() - 1;
        assert!(nav.connected(0, last), "the road is not one walkable run");
        let _ = road;
    }

    #[test]
    fn every_kind_builds_inside_its_cell_with_its_space_clear() {
        for kind in KINDS {
            let (l1, road) = find(kind);
            let c = build(road, l1);
            assert!(!c.prims.is_empty(), "{kind:?}");
            for p in &c.prims {
                let b = p.aabb();
                assert!(
                    b.min.x >= 0 && b.min.y >= 0 && b.min.z >= 0,
                    "{kind:?} {b:?}"
                );
                assert!(
                    b.max.x <= 4096 * M && b.max.y <= 4096 * M && b.max.z <= 4096 * M,
                    "{kind:?} {b:?}"
                );
            }
        }
    }
}
