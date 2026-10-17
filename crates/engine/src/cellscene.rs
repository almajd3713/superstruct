//! Picks the single L2 cell the app shows until streaming (P2.2) replaces it.

use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::l2::l2_cell;
use worldgen::seed::WorldSeed;

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
