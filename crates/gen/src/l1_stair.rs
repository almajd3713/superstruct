//! The infinite staircase's geometry (`docs/spec/regimes.md` §5.2). A square spiral inside the
//! 64 m footprint, the same in every L1 cell of the column: each side carries one flight that
//! climbs 32 m over 48 m, and an 8 m landing in each corner joins one flight to the next. A turn
//! takes 128 m, so a cell holds 32 turns and its top landing is the bottom landing of the cell
//! above. The flights hug the footprint's edge, where the decks around the hole meet them.

use crate::l1::Rect;
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};

const M: Q = 16;
/// Steps per flight; 4 q rise and 6 q tread each.
const STEPS: u16 = 128;
const FLIGHT_RISE: Q = 512;
const FLIGHT_RUN: Q = 768;
const LANDING: Q = 8 * M;
const WIDTH: Q = 4 * M;
const SLAB: Q = 8;
/// Flights per L1 cell; a multiple of four so every cell starts at the same corner.
pub const FLIGHTS: Q = 128;

fn prim(shape: Shape) -> Primitive {
    Primitive {
        shape,
        material: MaterialId::METAL,
        kind: FeatureKind::STAIR_TOWER,
        flags: PrimFlags::NONE,
    }
}

/// The flights and landings of one L1 cell, in cell-local quanta.
pub fn stair_pieces(foot: Rect) -> Vec<Primitive> {
    let mut out = Vec::new();
    let aabb = |x0: Q, y0: Q, z0: Q, x1: Q, y1: Q, z1: Q| {
        QAabb::new(QVec3::new(x0, y0, z0), QVec3::new(x1, y1, z1))
    };
    let (x_lo, x_hi) = (foot.x0 + LANDING, foot.x1 - LANDING);
    let (z_lo, z_hi) = (foot.z0 + LANDING, foot.z1 - LANDING);
    debug_assert_eq!(x_hi - x_lo, FLIGHT_RUN);
    for k in 0..FLIGHTS {
        let y = k * FLIGHT_RISE;
        let (strip, dir) = match k % 4 {
            0 => (
                aabb(x_lo, y, foot.z0, x_hi, y + FLIGHT_RISE, foot.z0 + WIDTH),
                Dir4::PosX,
            ),
            1 => (
                aabb(foot.x1 - WIDTH, y, z_lo, foot.x1, y + FLIGHT_RISE, z_hi),
                Dir4::PosZ,
            ),
            2 => (
                aabb(x_lo, y, foot.z1 - WIDTH, x_hi, y + FLIGHT_RISE, foot.z1),
                Dir4::NegX,
            ),
            _ => (
                aabb(foot.x0, y, z_lo, foot.x0 + WIDTH, y + FLIGHT_RISE, z_hi),
                Dir4::NegZ,
            ),
        };
        out.push(prim(Shape::stair_run(strip, dir, STEPS)));
    }
    // Landing `k` sits where flight `k` starts, with its top at the flight's bottom.
    for k in 1..=FLIGHTS {
        let y = k * FLIGHT_RISE;
        let (x0, z0) = match k % 4 {
            0 => (foot.x0, foot.z0),
            1 => (foot.x1 - LANDING, foot.z0),
            2 => (foot.x1 - LANDING, foot.z1 - LANDING),
            _ => (foot.x0, foot.z1 - LANDING),
        };
        out.push(prim(Shape::Box {
            aabb: aabb(x0, y - SLAB, z0, x0 + LANDING, y, z0 + LANDING),
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l1::CELL;
    use crate::walk::Nav;

    fn foot() -> Rect {
        Rect::square(100 * M, 200 * M, 64 * M)
    }

    #[test]
    fn the_spiral_stays_in_the_cell_and_in_its_footprint() {
        let f = foot();
        for p in stair_pieces(f) {
            let b = p.aabb();
            assert!(b.min.y >= 0 && b.max.y <= CELL, "{b:?}");
            assert!(b.min.x >= f.x0 && b.max.x <= f.x1 && b.min.z >= f.z0 && b.max.z <= f.z1);
        }
    }

    /// One piece after another climbs from the cell's bottom landing to its top landing.
    #[test]
    fn every_flight_is_connected_to_the_next() {
        let prims = stair_pieces(foot());
        let nav = Nav::build(&prims);
        let plates_of = |source: usize| -> Vec<usize> {
            (0..nav.plates.len())
                .filter(|&i| nav.plates[i].source == source)
                .collect()
        };
        let first = plates_of(0);
        let last = plates_of(FLIGHTS as usize - 1);
        assert!(!first.is_empty() && !last.is_empty());
        assert!(
            nav.connected(first[0], last[0]),
            "{} plates, first flight not joined to the last",
            nav.plates.len()
        );
    }
}
