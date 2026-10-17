//! Hierarchy levels, cell keys and world → cell math. Spec: `docs/spec/coordinates.md` §3–5.

use glam::I64Vec3;

use crate::coord::{SECTOR_SIZE_LOG2, WorldPos};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Level {
    L0 = 0,
    L1 = 1,
    L2 = 2,
    L3 = 3,
    L4 = 4,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::L0, Level::L1, Level::L2, Level::L3, Level::L4];

    /// log₂ of the cell size in metres: 15, 12, 9, 6, 3.
    pub const fn size_log2(self) -> u32 {
        15 - 3 * self as u32
    }

    pub const fn size_m(self) -> i64 {
        1 << self.size_log2()
    }
}

/// Floor division by `2^shift`. An arithmetic right shift rounds towards −∞.
pub const fn floor_div_pow2(a: i64, shift: u32) -> i64 {
    a >> shift
}

/// Floor modulo `2^shift`, always in `[0, 2^shift)`.
pub const fn floor_mod_pow2(a: i64, shift: u32) -> i64 {
    a & ((1 << shift) - 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellKey {
    pub level: Level,
    pub coords: I64Vec3,
}

impl CellKey {
    pub const fn new(level: Level, coords: I64Vec3) -> Self {
        Self { level, coords }
    }

    /// The cell containing `pos`. Infallible for any valid `WorldPos` (`coordinates.md` §7).
    pub fn containing(level: Level, pos: &WorldPos) -> Self {
        let k = level.size_log2();
        let sector = pos.sector();
        let coords = if k >= SECTOR_SIZE_LOG2 {
            let shift = k - SECTOR_SIZE_LOG2;
            I64Vec3::new(
                floor_div_pow2(sector.x, shift),
                floor_div_pow2(sector.y, shift),
                floor_div_pow2(sector.z, shift),
            )
        } else {
            let per_sector = 1i64 << (SECTOR_SIZE_LOG2 - k);
            // `local / size` is exact for a power-of-two size, and `local < 1024` keeps the
            // floor in `[0, per_sector)`.
            #[allow(clippy::cast_possible_truncation)]
            let sub = (pos.local() / level.size_m() as f32).floor().as_i64vec3();
            sector * per_sector + sub
        };
        Self { level, coords }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// Reference: floor(absolute / size) in i128, from positions on a 1/16 m grid.
    fn reference(level: Level, sector: I64Vec3, local_q: [i64; 3]) -> I64Vec3 {
        let size_q = i128::from(level.size_m()) * 16;
        let c = |s: i64, q: i64| {
            let abs_q = i128::from(s) * 1024 * 16 + i128::from(q);
            i64::try_from(abs_q.div_euclid(size_q)).unwrap()
        };
        I64Vec3::new(
            c(sector.x, local_q[0]),
            c(sector.y, local_q[1]),
            c(sector.z, local_q[2]),
        )
    }

    fn check(sector: I64Vec3, local_q: [i64; 3]) {
        #[allow(clippy::cast_precision_loss)]
        let local = Vec3::from_array(local_q.map(|q| q as f32 / 16.0));
        let pos = WorldPos::new(sector, local).unwrap();
        assert_eq!(
            pos.sector(),
            sector,
            "test input must already be normalized"
        );
        for level in Level::ALL {
            assert_eq!(
                CellKey::containing(level, &pos).coords,
                reference(level, sector, local_q),
                "level {level:?} sector {sector} local_q {local_q:?}"
            );
        }
    }

    #[test]
    fn floor_helpers() {
        assert_eq!(floor_div_pow2(-1, 2), -1);
        assert_eq!(floor_div_pow2(-24, 9), -1);
        assert_eq!(floor_div_pow2(-3, 3), -1);
        assert_eq!(floor_div_pow2(7, 3), 0);
        assert_eq!(floor_mod_pow2(-3, 7), 125);
        assert_eq!(floor_mod_pow2(-1, 1), 1);
    }

    #[test]
    fn spec_worked_example() {
        let pos = WorldPos::new(I64Vec3::new(-1, 0, 3), Vec3::new(1000.0, 5.0, 17.5)).unwrap();
        let expect = [
            (-1, 0, 0),
            (-1, 0, 0),
            (-1, 0, 6),
            (-1, 0, 48),
            (-3, 0, 386),
        ];
        for (level, (x, y, z)) in Level::ALL.into_iter().zip(expect) {
            assert_eq!(
                CellKey::containing(level, &pos).coords,
                I64Vec3::new(x, y, z)
            );
        }
    }

    #[test]
    fn zero_minus_one_and_sector_edges() {
        let last_q = 1024 * 16 - 1; // 1023.9375 m
        for s in [0, -1, 1, -2, 31, 32, -32, -33] {
            for q in [0, 1, 8 * 16 - 1, 8 * 16, 512 * 16, last_q] {
                check(I64Vec3::new(s, -s, s), [q, last_q - q, q]);
            }
        }
    }

    #[test]
    fn far_coordinates() {
        let p50 = 1i64 << 50;
        for s in [p50, -p50, p50 - 1, -p50 + 1, -p50 - 1] {
            for q in [0, 1, 1024 * 16 - 1] {
                check(I64Vec3::splat(s), [q; 3]);
            }
        }
    }
}
