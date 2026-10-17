//! Shared keys that let several cells agree on one feature. Spec: `docs/spec/levels.md` §1.2.
//!
//! Each key exposes `hash_coords()`: the cell coordinates it is hashed with, together with its
//! own salt, through the single hash entry point.

use glam::I64Vec3;

use crate::cell::{CellKey, Level, floor_div_pow2};

/// A whole vertical column of cells at one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnKey {
    pub level: Level,
    pub x: i64,
    pub z: i64,
}

impl ColumnKey {
    pub fn of(cell: CellKey) -> Self {
        Self {
            level: cell.level,
            x: cell.coords.x,
            z: cell.coords.z,
        }
    }

    pub fn hash_coords(&self) -> I64Vec3 {
        I64Vec3::new(self.x, 0, self.z)
    }
}

/// One horizontal layer of cells over a `W × W` region, with `W = 2^region_log2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerKey {
    pub level: Level,
    pub y: i64,
    pub rx: i64,
    pub rz: i64,
}

impl LayerKey {
    pub fn of(cell: CellKey, region_log2: u32) -> Self {
        Self {
            level: cell.level,
            y: cell.coords.y,
            rx: floor_div_pow2(cell.coords.x, region_log2),
            rz: floor_div_pow2(cell.coords.z, region_log2),
        }
    }

    pub fn hash_coords(&self) -> I64Vec3 {
        I64Vec3::new(self.rx, self.y, self.rz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(level: Level, x: i64, y: i64, z: i64) -> CellKey {
        CellKey::new(level, I64Vec3::new(x, y, z))
    }

    #[test]
    fn column_ignores_y() {
        let a = ColumnKey::of(cell(Level::L1, -3, -100, 7));
        let b = ColumnKey::of(cell(Level::L1, -3, 1 << 40, 7));
        assert_eq!(a, b);
        assert_eq!(a.hash_coords(), I64Vec3::new(-3, 0, 7));
        assert_ne!(a, ColumnKey::of(cell(Level::L1, -2, 0, 7)));
        assert_ne!(a, ColumnKey::of(cell(Level::L2, -3, 0, 7)));
    }

    #[test]
    fn layer_groups_regions_with_floor_semantics() {
        // W = 4: x in [-4, -1] is region -1, x in [0, 3] is region 0.
        let k = |x, z| LayerKey::of(cell(Level::L1, x, 9, z), 2);
        assert_eq!(k(-4, 0), k(-1, 3));
        assert_eq!(k(0, 0), k(3, 3));
        assert_ne!(k(-1, 0), k(0, 0));
        assert_eq!(k(-1, -1).hash_coords(), I64Vec3::new(-1, 9, -1));
        assert_ne!(k(0, 0), LayerKey::of(cell(Level::L1, 0, 10, 0), 2));
    }
}
