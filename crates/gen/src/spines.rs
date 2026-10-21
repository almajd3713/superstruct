//! Spines (SPN-01..04, `docs/proposals/the-web-ideas.md` §3): vertical super-arteries. Spec:
//! `docs/spec/levels.md` §3W.7.
//!
//! An L0 column holds a spine with chance 1 in `ONE_IN`; it stands in an inner L1 column of that
//! L0 column and runs through every L1 cell of the column, the same in each: a tower of 48 m
//! square with a lift shaft in its middle (SPN-02), a round spiral stair that never ends round
//! the shaft (SPN-03), and doorways with landings at every crust surface (SPN-04).

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
use crate::l1::CELL;
use crate::orient::{HelixStair, helix_stair};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Stratum, stack};

/// An L0 column holds a spine with chance 1 in this.
const ONE_IN: u64 = 8;
/// Half the side of the tower, the shaft and the wall, in metres.
const HALF: i32 = 24;
const SHAFT_HALF: i32 = 6;
const WALL: i32 = 2;

/// The inner L1 column `(x, z)` of L0 column `(x, z)` that holds a spine, if any.
pub fn spine_in_column(seed: WorldSeed, l0x: i64, l0z: i64) -> Option<(i64, i64)> {
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L0,
        I64Vec3::new(l0x, 0, l0z),
        salt::SPINE_COLUMN,
    ));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    Some((1 + rng.below(6) as i64, 1 + rng.below(6) as i64))
}

fn bx(lo: [i32; 3], hi: [i32; 3]) -> Shape {
    let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
    Shape::Box {
        aabb: QAabb::new(q(lo), q(hi)),
    }
}

/// The heights in metres, inside the L1 cell, where a crust begins above a Between on the
/// column's axis: the doorways of the tower.
pub(crate) fn stations(seed: WorldSeed, l1: I64Vec3) -> Vec<i32> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let st = stack(seed, l0);
    let origin = (l1 - l0 * 8) * 4096;
    let mut out = Vec::new();
    let mut below = None;
    for row in 0..8i64 {
        let p = I64Vec3::new(origin.x + 2048, origin.y + row * 512 + 256, origin.z + 2048);
        let s = st.stratum_at(p, None);
        if matches!(s, Stratum::Crust(_)) && matches!(below, Some(Stratum::Between(_))) {
            out.push((row * 512) as i32);
        }
        below = Some(s);
    }
    out
}

/// The geometry of the spine in L1 cell `l1`, if its column holds one.
pub fn spine_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let inner = l1 - l0 * 8;
    match spine_in_column(seed, l0.x, l0.z) {
        Some((a, b)) if (inner.x, inner.z) == (a, b) => {}
        _ => return Content::default(),
    }
    let c = 2048;
    let mut out: Vec<Primitive> = Vec::new();
    let solid = PrimFlags::NONE;
    let prim =
        |shape: Shape, material: MaterialId, kind: FeatureKind, flags: PrimFlags| Primitive {
            shape,
            material,
            kind,
            flags,
        };
    let stops = stations(seed, l1);
    // The outer wall in spans between the doorways (4 m wide, 3.5 m tall, from the station's floor).
    let mut spans: Vec<(i32, i32)> = Vec::new();
    let mut y = 0;
    for &s in &stops {
        if s > y {
            spans.push((y, s));
        }
        y = y.max(s + 4);
    }
    if y < 4096 {
        spans.push((y, 4096));
    }
    for &(y0, y1) in &spans {
        for (lo, hi) in [
            ([c - HALF, y0, c - HALF], [c - HALF + WALL, y1, c + HALF]),
            ([c + HALF - WALL, y0, c - HALF], [c + HALF, y1, c + HALF]),
            (
                [c - HALF + WALL, y0, c - HALF],
                [c + HALF - WALL, y1, c - HALF + WALL],
            ),
            (
                [c - HALF + WALL, y0, c + HALF - WALL],
                [c + HALF - WALL, y1, c + HALF],
            ),
        ] {
            out.push(prim(
                bx(lo, hi),
                MaterialId::CONCRETE,
                FeatureKind::SHAFT_WALL,
                solid,
            ));
        }
    }
    // The doorways' landings: a deck 8 m deep out of the wall facing +z.
    for &s in &stops {
        out.push(prim(
            bx([c - 6, s - 1, c + HALF], [c + 6, s, c + HALF + 8]),
            MaterialId::CONCRETE,
            FeatureKind::WALKWAY,
            solid,
        ));
        out.push(prim(
            bx([c - 3, s + 4, c + HALF - WALL], [c + 3, s + 6, c + HALF]),
            MaterialId::EMISSIVE,
            FeatureKind::MISC,
            PrimFlags::NO_COLLIDE,
        ));
    }
    // SPN-02: the lift shaft in the middle (the volume a car runs in, and its walls).
    out.push(prim(
        Shape::LiftShaft {
            aabb: QAabb::new(
                QVec3::new((c - SHAFT_HALF) * M, 0, (c - SHAFT_HALF) * M),
                QVec3::new((c + SHAFT_HALF) * M, CELL, (c + SHAFT_HALF) * M),
            ),
        },
        MaterialId::METAL,
        FeatureKind::SHAFT_WALL,
        solid,
    ));
    for (lo, hi) in [
        (
            [c - SHAFT_HALF, 0, c - SHAFT_HALF],
            [c - SHAFT_HALF + 1, 4096, c + SHAFT_HALF],
        ),
        (
            [c + SHAFT_HALF - 1, 0, c - SHAFT_HALF],
            [c + SHAFT_HALF, 4096, c + SHAFT_HALF],
        ),
        (
            [c - SHAFT_HALF + 1, 0, c - SHAFT_HALF],
            [c + SHAFT_HALF - 1, 4096, c - SHAFT_HALF + 1],
        ),
        (
            [c - SHAFT_HALF + 1, 0, c + SHAFT_HALF - 1],
            [c + SHAFT_HALF - 1, 4096, c + SHAFT_HALF],
        ),
    ] {
        out.push(prim(
            bx(lo, hi),
            MaterialId::METAL,
            FeatureKind::SHAFT_WALL,
            solid,
        ));
    }
    // SPN-03: a round spiral stair between the shaft and the wall, 64 treads a turn.
    let per_turn = 64;
    let rise = 7;
    let turns = CELL / (per_turn * rise);
    out.extend(helix_stair(
        &HelixStair {
            axis: QVec3::new(c * M, 0, c * M),
            inner: 11 * M,
            width: 4 * M,
            per_turn: per_turn as u8,
            steps: (turns * per_turn) as u32,
            rise,
            thick: 2,
            start: 0,
            sense: 1,
            material: MaterialId::METAL,
            kind: FeatureKind::STAIR_TOWER,
            flags: solid,
        },
        QAabb::new(QVec3::new(0, 0, 0), QVec3::new(CELL, CELL, CELL)),
    ));
    // SPN-01: seen from afar, a column of 32 m (far band only).
    out.push(prim(
        bx([c - 16, 0, c - 16], [c + 16, 4096, c + 16]),
        MaterialId::METAL,
        FeatureKind::STAIR_TOWER,
        PrimFlags::FAR_VISIBLE | PrimFlags::FAR_ONLY | PrimFlags::NO_COLLIDE,
    ));
    let keep = QAabb::new(
        QVec3::new((c - HALF - 2) * M, 0, (c - HALF - 2) * M),
        QVec3::new((c + HALF + 12) * M, CELL, (c + HALF + 12) * M),
    );
    let _: Q = 0;
    Content {
        prims: out,
        keep_out: vec![keep],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find() -> (I64Vec3, i64, i64) {
        for x in -30..30 {
            for z in -30..30 {
                if let Some((a, b)) = spine_in_column(SEED, x, z) {
                    return (I64Vec3::new(x * 8 + a, 0, z * 8 + b), x, z);
                }
            }
        }
        panic!("no spine");
    }

    #[test]
    fn spines_stand_in_one_column_in_eight_and_run_through_every_cell() {
        let mut n = 0;
        for x in -20..20 {
            for z in -20..20 {
                n += usize::from(spine_in_column(SEED, x, z).is_some());
            }
        }
        assert!((100..300).contains(&n), "{n} of 1600");
        let (l1, _, _) = find();
        for y in [-9, -1, 0, 3, 12] {
            let c = spine_content(SEED, I64Vec3::new(l1.x, y, l1.z));
            assert!(!c.prims.is_empty(), "no spine {y} cells up");
        }
        assert!(spine_content(SEED, l1 + I64Vec3::X).prims.is_empty());
    }

    /// SPN-02..04: a lift shaft, a spiral and the tower round them; doorways at crusts.
    #[test]
    fn a_spine_has_a_shaft_a_spiral_and_stations() {
        let (l1, x, z) = find();
        let mut stations_seen = 0;
        for y in -6..6 {
            let cell = I64Vec3::new(l1.x, y, l1.z);
            let c = spine_content(SEED, cell);
            assert!(
                c.prims
                    .iter()
                    .any(|p| matches!(p.shape, Shape::LiftShaft { .. }))
            );
            assert!(
                c.prims.len() > 9000,
                "{} pieces: the spiral is missing",
                c.prims.len()
            );
            stations_seen += stations(SEED, cell).len();
            assert!(
                c.prims
                    .iter()
                    .any(|p| p.flags.contains(PrimFlags::FAR_ONLY))
            );
        }
        let _ = (x, z);
        assert!(
            stations_seen > 0,
            "no crust surface in 12 cells of the column"
        );
    }
}
