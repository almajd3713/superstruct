//! Vertical path types, batch 2 (P2-13..25, `docs/proposals/the-web-ideas.md` §6): ids 73 to
//! 85. Each is built on a climb the first batch already makes and dressed to be its own thing:
//! the walking part is the base type's, the rest is not solid. Spec: `docs/spec/levels.md`
//! §5W.41.

#![allow(clippy::too_many_lines)]

use super::{M, S};
use crate::prim::{FeatureKind, MaterialId};

/// Ids 60 plus 13 to 25 for P2-13..25.
pub const P2V_IDS: [u8; 13] = [73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85];

/// The base type a P2 id climbs as: a ladder family (41 to 53) or a stair type (21 to 39).
pub fn base(id: u8) -> u8 {
    match id {
        73 | 74 | 85 => 41,
        75 => 45,
        79 => 50,
        80 => 32,
        81 => 25,
        82 => 26,
        _ => 21,
    }
}

/// The point `k` of `n` (a divisor of 64) on a circle of `radius` metres, from the heading table.
fn on_circle(k: u32, n: u32, radius: f64) -> (f64, f64) {
    let (a, b) = crate::orient::heading((k * (64 / n)) as u8);
    let len = f64::from(a * a + b * b).sqrt();
    (radius * f64::from(a) / len, radius * f64::from(b) / len)
}

const METAL: MaterialId = MaterialId::METAL;
const CONC: MaterialId = MaterialId::CONCRETE;

fn free(m: &mut M<'_, '_>, u: S, v: S, y: S, material: MaterialId, kind: FeatureKind) {
    m.bx_free(u, v, y, material, kind);
}

/// The dressing of P2 id `id` over a climb of `height` metres from the column's foot.
pub(super) fn dress(m: &mut M<'_, '_>, id: u8, height: f64) {
    let (y0, y1) = (m.col.y0, m.col.y0 + height);
    let mut y = y0;
    match id {
        // P2-13: a rolling ladder on an archive wall: shelving behind it, a rail along the top.
        73 => {
            while y < y1 - 1.0 {
                free(
                    m,
                    (1.3, 2.2),
                    (-6.0, 6.0),
                    (y, y + 0.12),
                    CONC,
                    FeatureKind::MISC,
                );
                free(
                    m,
                    (1.3, 1.4),
                    (-6.0, 6.0),
                    (y, y + 2.4),
                    METAL,
                    FeatureKind::MISC,
                );
                y += 2.5;
            }
            free(
                m,
                (1.0, 1.6),
                (-8.0, 8.0),
                (y1 - 0.1, y1 + 0.1),
                METAL,
                FeatureKind::RAILING,
            );
        }
        // P2-14: a dumbwaiter shaft: walls round the ladder and a parked box on its ropes.
        74 => {
            free(
                m,
                (1.3, 1.5),
                (-1.4, 1.4),
                (y0, y1),
                CONC,
                FeatureKind::MISC,
            );
            free(
                m,
                (0.3, 1.5),
                (-1.6, -1.4),
                (y0, y1),
                CONC,
                FeatureKind::MISC,
            );
            free(m, (0.3, 1.5), (1.4, 1.6), (y0, y1), CONC, FeatureKind::MISC);
            free(
                m,
                (0.5, 1.1),
                (-0.5, 0.5),
                (y0 + height / 3.0, y0 + height / 3.0 + 0.8),
                METAL,
                FeatureKind::MISC,
            );
            free(
                m,
                (0.7, 0.75),
                (-0.05, 0.05),
                (y0, y1),
                METAL,
                FeatureKind::CABLE,
            );
        }
        // P2-15: a chimney: a ring of brick slabs round the climb and a glow at the top.
        75 => {
            for k in 0..16 {
                let (u, v) = on_circle(k, 16, 6.0);
                free(
                    m,
                    (u - 0.8, u + 0.8),
                    (v - 0.8, v + 0.8),
                    (y0, y1),
                    CONC,
                    FeatureKind::MISC,
                );
            }
            free(
                m,
                (-5.0, 5.0),
                (-5.0, 5.0),
                (y1, y1 + 0.2),
                MaterialId::WARM,
                FeatureKind::MISC,
            );
        }
        // P2-16: a stair-escalator: the machinery housings at both ends and a lit side strip.
        76 => {
            for yy in [y0, y1 - 1.5] {
                free(
                    m,
                    (-3.0, -1.0),
                    (-3.0, 3.0),
                    (yy, yy + 1.5),
                    METAL,
                    FeatureKind::MISC,
                );
            }
            free(
                m,
                (-8.0, -7.8),
                (-8.0, 8.0),
                (y0, y1),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
            );
        }
        // P2-17: a counterweight pair: two cars on cables beside the stair, one up, one down.
        77 => {
            free(
                m,
                (9.0, 11.0),
                (-1.5, 1.5),
                (y0 + height * 0.7, y0 + height * 0.7 + 2.5),
                METAL,
                FeatureKind::MISC,
            );
            free(
                m,
                (-11.0, -9.0),
                (-1.5, 1.5),
                (y0 + height * 0.3, y0 + height * 0.3 + 2.5),
                METAL,
                FeatureKind::MISC,
            );
            for u in [-10.0, 10.0] {
                free(
                    m,
                    (u - 0.05, u + 0.05),
                    (-0.05, 0.05),
                    (y0, y1),
                    METAL,
                    FeatureKind::CABLE,
                );
            }
        }
        // P2-18: Ferris cabins: a wheel of eight cabins round the stair on spokes.
        78 => {
            let (cy, r) = (y0 + height / 2.0, 11.0);
            for k in 0..8 {
                let (u, v) = on_circle(k, 8, r);
                free(
                    m,
                    (u - 0.1, u + 0.1),
                    (-0.1, 0.1),
                    (cy + v - 0.1, cy + v + 0.1),
                    METAL,
                    FeatureKind::CABLE,
                );
                free(
                    m,
                    (u - 1.0, u + 1.0),
                    (-1.2, 1.2),
                    (cy + v - 3.2, cy + v - 0.8),
                    CONC,
                    FeatureKind::MISC,
                );
            }
            free(
                m,
                (-r, r),
                (-0.2, 0.2),
                (cy - 0.2, cy + 0.2),
                METAL,
                FeatureKind::MISC,
            );
        }
        // P2-19: a bucket chain: a slow loop of buckets down one side of the ladders.
        79 => {
            while y < y1 - 1.0 {
                free(
                    m,
                    (2.6, 3.4),
                    (-0.4, 0.4),
                    (y, y + 0.7),
                    METAL,
                    FeatureKind::MISC,
                );
                y += 3.0;
            }
            free(
                m,
                (2.95, 3.05),
                (-0.03, 0.03),
                (y0, y1),
                METAL,
                FeatureKind::CABLE,
            );
        }
        // P2-20: a funicular on a stair beam: rails along the slope and a car at the foot.
        80 => {
            for v in [-1.0, 1.0] {
                free(
                    m,
                    (-9.0, 9.0),
                    (v - 0.1, v + 0.1),
                    (y0, y0 + 0.3),
                    METAL,
                    FeatureKind::TRAM,
                );
            }
            free(
                m,
                (-3.0, 3.0),
                (-1.4, 1.4),
                (y0 + 0.3, y0 + 2.6),
                METAL,
                FeatureKind::TRAM,
            );
        }
        // P2-21: a spiral of spirals: an outer ring of light posts that turn with the stair.
        81 => {
            for k in 0..16 {
                let (u, v) = on_circle(k, 16, 13.0);
                free(
                    m,
                    (u - 0.2, u + 0.2),
                    (v - 0.2, v + 0.2),
                    (y0 + height * f64::from(k) / 16.0, y1),
                    MaterialId::EMISSIVE,
                    FeatureKind::MISC,
                );
            }
        }
        // P2-22: a grand ramp: a wide lit parapet round the ramp.
        82 => {
            free(
                m,
                (-14.0, -13.8),
                (-14.0, 14.0),
                (y0, y1),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
            );
            free(
                m,
                (13.8, 14.0),
                (-14.0, 14.0),
                (y0, y1),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
            );
        }
        // P2-23: a stepped pyramid stair: terraces round the stair, each narrower than the last.
        83 => {
            let n = (height / 8.0).ceil().max(1.0) as i32;
            for k in 0..n {
                let half = 14.0 - f64::from(k) * (10.0 / f64::from(n));
                let yy = y0 + f64::from(k) * 8.0;
                free(
                    m,
                    (-half, half),
                    (-half, half),
                    (yy, (yy + 0.6).min(y1)),
                    CONC,
                    FeatureKind::MISC,
                );
            }
        }
        // P2-24: an inverted stair: it hangs from rods that reach to the top of the climb.
        84 => {
            for (u, v) in [(-6.0, -6.0), (6.0, -6.0), (-6.0, 6.0), (6.0, 6.0)] {
                free(
                    m,
                    (u - 0.05, u + 0.05),
                    (v - 0.05, v + 0.05),
                    (y0, y1 + 3.0),
                    METAL,
                    FeatureKind::CABLE,
                );
            }
            free(
                m,
                (-7.0, 7.0),
                (-7.0, 7.0),
                (y1 + 3.0, y1 + 3.5),
                CONC,
                FeatureKind::MISC,
            );
        }
        // P2-25: a tether climb: a thick cable up the middle of the ladders, a lamp at its top.
        85 => {
            free(
                m,
                (3.0, 4.0),
                (-0.5, 0.5),
                (y0, y1),
                METAL,
                FeatureKind::TETHER,
            );
            free(
                m,
                (2.8, 4.2),
                (-0.7, 0.7),
                (y1, y1 + 0.5),
                MaterialId::EMISSIVE,
                FeatureKind::MISC,
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orient::Frame;
    use crate::paths_v::{Column, VParams, build_rise};
    use crate::tissue::Lattice;
    use crate::tissue_grow::Emitter;

    /// The P2 climbs that ride on a ladder reach the top from the foot, inside the column, with
    /// their dressing in place.
    #[test]
    fn ladder_based_climbs_reach_the_top_and_are_dressed() {
        let lat = Lattice::new(Frame::Id);
        for id in P2V_IDS
            .into_iter()
            .filter(|&i| (41..=53).contains(&base(i)))
        {
            for slots in [1.0, 2.0] {
                let p = VParams {
                    w: 3.0,
                    swap: false,
                    fx: 1.0,
                    fz: 1.0,
                    k: 0.5,
                    side: 1.0,
                };
                let col = Column {
                    cx: 256.0,
                    cz: 256.0,
                    y0: 100.0,
                    height: 32.0 * slots,
                };
                let mut e = Emitter::new(&lat);
                let r = build_rise(&mut e, id, col, &p);
                let (first, last) = (r.walk.first().unwrap(), r.walk.last().unwrap());
                assert!(
                    (first[1] - 100.0).abs() < 0.3,
                    "id {id}: starts at {first:?}"
                );
                assert!(
                    (last[1] - (100.0 + col.height)).abs() < 0.3,
                    "id {id}: ends at {last:?}"
                );
                assert!(e.out.len() > 20, "id {id}: {} pieces", e.out.len());
                let (a, b) = r.bounds.unwrap();
                assert!(
                    a[0] >= 256.0 - 16.0 && b[0] <= 256.0 + 16.0,
                    "id {id}: {a:?} {b:?}"
                );
            }
        }
    }
}
