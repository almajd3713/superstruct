//! Mass entrances (`docs/spec/levels.md` §6.2). A doorway in a mass envelope, reached from the
//! deck by an apron (deck masses) or from the skywalk landing (masses attached to a trunk).
//!
//! L3 gives entrances their meaning, so this is written before L3 and called directly by it.
//! P4.1.6 makes L2 emit them as ports and keep-outs.

use crate::cell::{CellKey, Level, floor_mod_pow2};
use crate::hash::hash;
use crate::l2::{CELL, MassInfo};
use crate::l2_mass::{Parent, overlaps};
use crate::l3_plan::{COL, Cols, Doorway, entrance_column};
use crate::prim::{CellOutput, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Doorway width, 4 m, and the apron depth, 8 m, and clear height, 4 m.
const WIDTH: Q = 64;
const APRON_DEPTH: Q = 128;
const APRON_HEIGHT: Q = 64;

/// An entrance and the clear strip in front of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entrance {
    pub doorway: Doorway,
    /// The reserved apron of a deck mass. A trunk-attached mass uses its skywalk landing.
    pub apron: Option<QAabb>,
}

/// The apron of `doorway` on a mass: the 4 m × 8 m strip in front of the doorway.
fn apron_of(aabb: &QAabb, doorway: &Doorway) -> QAabb {
    let (u0, u1) = (doorway.u0, doorway.u1);
    let (y0, y1) = (doorway.floor, doorway.floor + APRON_HEIGHT);
    match doorway.side {
        0 => QAabb::new(
            QVec3::new(u0, y0, aabb.min.z - APRON_DEPTH),
            QVec3::new(u1, y1, aabb.min.z),
        ),
        2 => QAabb::new(
            QVec3::new(u0, y0, aabb.max.z),
            QVec3::new(u1, y1, aabb.max.z + APRON_DEPTH),
        ),
        1 => QAabb::new(
            QVec3::new(aabb.max.x, y0, u0),
            QVec3::new(aabb.max.x + APRON_DEPTH, y1, u1),
        ),
        _ => QAabb::new(
            QVec3::new(aabb.min.x - APRON_DEPTH, y0, u0),
            QVec3::new(aabb.min.x, y1, u1),
        ),
    }
}

/// The side of a mass face that a skywalk starts from: 0 = -z, 1 = +x, 2 = +z, 3 = -x.
fn walk_side(axis: crate::cell::Axis, inward: Q) -> u8 {
    match (axis, inward > 0) {
        (crate::cell::Axis::X, true) => 1,
        (crate::cell::Axis::X, false) => 3,
        (_, true) => 2,
        (_, false) => 0,
    }
}

/// One entrance per mass, in mass order, or `None` where none fits. `parent` is the L1 output of
/// the cell's parent, `origin` the cell's offset inside it, and `l2_prims` everything L2 emitted
/// (envelopes included).
#[allow(clippy::cast_possible_truncation)] // a small index
pub fn entrances(
    seed: WorldSeed,
    cell: CellKey,
    parent: &CellOutput,
    masses: &[MassInfo],
    l2_prims: &[Primitive],
) -> Vec<Option<Entrance>> {
    debug_assert_eq!(cell.level, Level::L2);
    let o = cell.coords.map(|v| floor_mod_pow2(v, 3) * i64::from(CELL));
    let origin = [o.x as Q, o.y as Q, o.z as Q];
    let ctx = Parent::new(parent, origin);
    let solid: Vec<QAabb> = l2_prims
        .iter()
        .filter(|p| {
            !matches!(p.shape, Shape::LiftShaft { .. }) && !p.flags.contains(PrimFlags::NO_COLLIDE)
        })
        .map(Primitive::aabb)
        .collect();
    let mut rng = Rng::new(hash(seed.0, Level::L2, cell.coords, salt::MASS_ENTRANCE));
    masses
        .iter()
        .map(|m| {
            let cols = Cols::of(&m.aabb);
            match m.walk {
                None => {
                    // Two draws per deck mass, whatever the outcome.
                    let side0 = rng.below(4) as u8;
                    let j0 = rng.next_u32() as usize;
                    deck_entrance(&ctx, &solid, &m.aabb, &cols, side0, j0)
                }
                Some(walk) => attached_entrance(
                    &m.aabb,
                    &cols,
                    walk_side(walk.axis, walk.inward),
                    walk.u0,
                    walk.u1,
                ),
            }
        })
        .collect()
}

fn deck_entrance(
    ctx: &Parent,
    solid: &[QAabb],
    aabb: &QAabb,
    cols: &Cols,
    side0: u8,
    j0: usize,
) -> Option<Entrance> {
    if cols.count() == 0 {
        return None;
    }
    for n in 0..4u8 {
        let side = (side0 + n) % 4;
        let row = cols.row(side);
        for t in 0..row.len() {
            let (cx, cz) = row[(j0 % row.len() + t) % row.len()];
            let u = if side.is_multiple_of(2) { cx } else { cz };
            let doorway = Doorway {
                side,
                u0: u * COL + 32,
                u1: u * COL + 32 + WIDTH,
                floor: aabb.min.y,
            };
            let apron = apron_of(aabb, &doorway);
            if ctx.fits(&apron) && !solid.iter().any(|s| overlaps(s, &apron)) {
                return Some(Entrance {
                    doorway,
                    apron: Some(apron),
                });
            }
        }
    }
    None
}

/// The lowest 4 m sub-range of the skywalk's width that lies inside one IN column.
fn attached_entrance(aabb: &QAabb, cols: &Cols, side: u8, u0: Q, u1: Q) -> Option<Entrance> {
    let mut a = u0;
    while a + WIDTH <= u1 {
        let doorway = Doorway {
            side,
            u0: a,
            u1: a + WIDTH,
            floor: aabb.min.y,
        };
        if entrance_column(cols, &doorway).is_some() {
            return Some(Entrance {
                doorway,
                apron: None,
            });
        }
        a += WIDTH;
    }
    None
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::l2::l2_build;
    use crate::l2_port::L1Cache;

    #[test]
    fn entrances_are_pure_and_sit_on_deck_or_skywalk() {
        let seed = WorldSeed(0x0E17);
        let (mut deck, mut deck_ok, mut attached, mut attached_ok) = (0, 0, 0, 0);
        for x in -10..10 {
            let parent_key = CellKey::new(Level::L1, I64Vec3::new(x, x % 3, 2 * x % 5));
            let cache = L1Cache::default();
            let parent = cache.get(seed, parent_key);
            for k in 0..512i64 {
                let idx = [k % 8, k / 64, k / 8 % 8];
                let cell = CellKey::new(
                    Level::L2,
                    parent_key.coords * 8 + I64Vec3::new(idx[0], idx[1], idx[2]),
                );
                let (out, masses) = l2_build(seed, &cache, cell);
                if masses.is_empty() {
                    continue;
                }
                let es = entrances(seed, cell, &parent, &masses, &out.primitives);
                assert_eq!(es, entrances(seed, cell, &parent, &masses, &out.primitives));
                for (m, e) in masses.iter().zip(&es) {
                    let is_attached = m.walk.is_some();
                    if is_attached {
                        attached += 1;
                    } else {
                        deck += 1;
                    }
                    let Some(e) = e else {
                        continue;
                    };
                    let d = e.doorway;
                    assert_eq!(d.floor, m.aabb.min.y);
                    assert_eq!(d.u1 - d.u0, 64);
                    let cols = Cols::of(&m.aabb);
                    assert!(entrance_column(&cols, &d).is_some());
                    if is_attached {
                        attached_ok += 1;
                    } else {
                        deck_ok += 1;
                        assert!(e.apron.is_some());
                    }
                }
            }
        }
        println!(
            "deck masses {deck}, with entrance {deck_ok}; attached {attached}, with entrance {attached_ok}"
        );
        assert!(deck > 50 && deck_ok * 2 > deck, "{deck_ok}/{deck}");
        assert_eq!(attached, attached_ok, "every attached mass has an entrance");
    }
}
