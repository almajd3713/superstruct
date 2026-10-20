//! Picks the single L2 cell the app shows until streaming (P2.2) replaces it.

use glam::{I64Vec3, Vec3};
use worldgen::cell::{CellKey, Level};
use worldgen::l2::l2_cell;
use worldgen::seed::WorldSeed;
use worldgen::tissue::{Surround, TissueCache, tissue_cell};

/// The L2 cell with the most primitives within `radius` cells of the origin, ties to the lowest
/// coordinates so the choice is stable.
pub fn busiest_l2_cell(seed: WorldSeed, radius: i64) -> CellKey {
    let mut best = (0usize, CellKey::new(Level::L2, I64Vec3::ZERO));
    for x in -radius..=radius {
        for y in -radius..=radius {
            for z in -radius..=radius {
                let key = CellKey::new(Level::L2, I64Vec3::new(x, y, z));
                let n = l2_cell(seed, key).primitives.len();
                if n > best.0 {
                    best = (n, key);
                }
            }
        }
    }
    best.1
}

/// A place to stand in the Web world: in the knot's L2 cell of the first L1 cell near the origin
/// that has one, at the start of its longest level route segment that is in the open, 1 m above
/// the floor of the slot. Returned as the L2 cell and the point in it (cell-local metres). A
/// path piece lies under every segment end, and the knot itself is an L1 block, which is solid.
pub fn knot_spawn(seed: WorldSeed) -> Option<(CellKey, Vec3)> {
    let cache = TissueCache::default();
    for r in 0..3i64 {
        for x in -r..=r {
            for y in -r..=r {
                for z in -r..=r {
                    if x.abs().max(y.abs()).max(z.abs()) != r {
                        continue;
                    }
                    let l1 = I64Vec3::new(x, y, z);
                    let Some(sk) = worldgen::l1_artery::skeleton(seed, l1) else {
                        continue;
                    };
                    let c = worldgen::l1_artery::coords(usize::from(sk.knot));
                    let key = CellKey::new(Level::L2, l1 * 8 + I64Vec3::new(c[0], c[1], c[2]));
                    let t = tissue_cell(seed, &cache, key);
                    let best = t
                        .segments
                        .iter()
                        .enumerate()
                        .filter(|(i, s)| {
                            s.surround == Surround::Level
                                && !t.engulfed.contains(&(*i as u16))
                                && s.slots.len() >= 3
                        })
                        .max_by_key(|(_, s)| s.slots.len());
                    let Some((_, seg)) = best else {
                        continue;
                    };
                    let centre = t.lattice.centre(seg.slots[0] as usize);
                    let up = t.lattice.frame.axes()[1];
                    let p = centre - up * (t.lattice.pitch as f64 / 2.0 - 1.0);
                    return Some((key, Vec3::new(p.x as f32, p.y as f32, p.z as f32)));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "exploration helper: prints the busiest cell"]
    fn print_busy_cells() {
        for radius in [2, 4] {
            println!(
                "radius {radius}: {:?}",
                busiest_l2_cell(WorldSeed(0), radius)
            );
        }
    }
}
