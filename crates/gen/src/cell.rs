//! Hierarchy levels, cell keys and world → cell math. Spec: `docs/spec/coordinates.md` §3–5.

use glam::I64Vec3;

use crate::coord::{CoordError, SECTOR_SIZE_LOG2, WorldPos};

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

    /// Smallest valid cell coordinate on any axis: the domain `[−2⁶², 2⁶²)` m divided by the
    /// cell size (`coordinates.md` §7).
    pub const fn cell_min(self) -> i64 {
        -(1 << (62 - self.size_log2()))
    }

    pub const fn cell_max(self) -> i64 {
        (1 << (62 - self.size_log2())) - 1
    }

    pub const fn parent(self) -> Option<Level> {
        match self {
            Level::L0 => None,
            Level::L1 => Some(Level::L0),
            Level::L2 => Some(Level::L1),
            Level::L3 => Some(Level::L2),
            Level::L4 => Some(Level::L3),
        }
    }

    pub const fn child(self) -> Option<Level> {
        match self {
            Level::L0 => Some(Level::L1),
            Level::L1 => Some(Level::L2),
            Level::L2 => Some(Level::L3),
            Level::L3 => Some(Level::L4),
            Level::L4 => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir6 {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Dir6 {
    pub const ALL: [Dir6; 6] = [
        Dir6::PosX,
        Dir6::NegX,
        Dir6::PosY,
        Dir6::NegY,
        Dir6::PosZ,
        Dir6::NegZ,
    ];

    pub const fn offset(self) -> I64Vec3 {
        match self {
            Dir6::PosX => I64Vec3::X,
            Dir6::NegX => I64Vec3::NEG_X,
            Dir6::PosY => I64Vec3::Y,
            Dir6::NegY => I64Vec3::NEG_Y,
            Dir6::PosZ => I64Vec3::Z,
            Dir6::NegZ => I64Vec3::NEG_Z,
        }
    }

    #[must_use]
    pub const fn opposite(self) -> Dir6 {
        match self {
            Dir6::PosX => Dir6::NegX,
            Dir6::NegX => Dir6::PosX,
            Dir6::PosY => Dir6::NegY,
            Dir6::NegY => Dir6::PosY,
            Dir6::PosZ => Dir6::NegZ,
            Dir6::NegZ => Dir6::PosZ,
        }
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

/// A cell's world-space bounds: its minimum corner and its edge length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellBounds {
    pub min: WorldPos,
    pub size_m: i64,
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

    pub fn in_domain(&self) -> bool {
        let (lo, hi) = (self.level.cell_min(), self.level.cell_max());
        self.coords.cmpge(I64Vec3::splat(lo)).all() && self.coords.cmple(I64Vec3::splat(hi)).all()
    }

    pub fn parent(&self) -> Option<CellKey> {
        let level = self.level.parent()?;
        let c = self.coords;
        Some(CellKey::new(
            level,
            I64Vec3::new(
                floor_div_pow2(c.x, 3),
                floor_div_pow2(c.y, 3),
                floor_div_pow2(c.z, 3),
            ),
        ))
    }

    /// The 512 children in a fixed order: x fastest, then y, then z. `Ok(None)` at L4.
    pub fn children(&self) -> Result<Option<impl Iterator<Item = CellKey>>, CoordError> {
        if !self.in_domain() {
            return Err(CoordError::OutOfDomain);
        }
        let Some(level) = self.level.child() else {
            return Ok(None);
        };
        let base = self.coords * 8;
        Ok(Some((0..512i64).map(move |i| {
            CellKey::new(level, base + I64Vec3::new(i & 7, (i >> 3) & 7, i >> 6))
        })))
    }

    pub fn neighbor(&self, dir: Dir6) -> Result<CellKey, CoordError> {
        let d = dir.offset();
        let add = |a: i64, b: i64| a.checked_add(b).ok_or(CoordError::OutOfDomain);
        let n = CellKey::new(
            self.level,
            I64Vec3::new(
                add(self.coords.x, d.x)?,
                add(self.coords.y, d.y)?,
                add(self.coords.z, d.z)?,
            ),
        );
        if n.in_domain() {
            Ok(n)
        } else {
            Err(CoordError::OutOfDomain)
        }
    }

    /// Minimum corner and size (`coordinates.md` §5.1).
    pub fn aabb_world(&self) -> Result<CellBounds, CoordError> {
        if !self.in_domain() {
            return Err(CoordError::OutOfDomain);
        }
        let k = self.level.size_log2();
        let c = self.coords;
        let min = if k >= SECTOR_SIZE_LOG2 {
            WorldPos::new(c * (1 << (k - SECTOR_SIZE_LOG2)), glam::Vec3::ZERO)?
        } else {
            let shift = SECTOR_SIZE_LOG2 - k;
            let sector = I64Vec3::new(
                floor_div_pow2(c.x, shift),
                floor_div_pow2(c.y, shift),
                floor_div_pow2(c.z, shift),
            );
            let sub = I64Vec3::new(
                floor_mod_pow2(c.x, shift),
                floor_mod_pow2(c.y, shift),
                floor_mod_pow2(c.z, shift),
            );
            WorldPos::new(sector, (sub * self.level.size_m()).as_vec3())?
        };
        Ok(CellBounds {
            min,
            size_m: self.level.size_m(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use proptest::prelude::*;
    use std::collections::HashSet;

    /// In-domain cells: anywhere, hugging either domain edge, or near the origin.
    fn any_cell() -> impl Strategy<Value = CellKey> {
        (0usize..5, any::<[i64; 3]>(), 0u8..4).prop_map(|(l, raw, mode)| {
            let level = Level::ALL[l];
            let (lo, hi) = (level.cell_min(), level.cell_max());
            let span = hi - lo + 1;
            let c = raw.map(|r| match mode {
                0 => lo + r.rem_euclid(span),
                1 => lo + r.rem_euclid(3),
                2 => hi - r.rem_euclid(3),
                _ => r.rem_euclid(9) - 4,
            });
            CellKey::new(level, I64Vec3::from_array(c))
        })
    }

    proptest! {
        #[test]
        fn children_have_this_parent(cell in any_cell()) {
            if let Some(children) = cell.children().unwrap() {
                let all: Vec<CellKey> = children.collect();
                prop_assert_eq!(all.len(), 512);
                prop_assert_eq!(all.iter().collect::<HashSet<_>>().len(), 512);
                for child in &all {
                    prop_assert!(child.in_domain());
                    prop_assert_eq!(child.parent(), Some(cell));
                }
                prop_assert_eq!(all[0].aabb_world().unwrap().min, cell.aabb_world().unwrap().min);
            }
        }

        #[test]
        fn neighbor_round_trip(cell in any_cell()) {
            for dir in Dir6::ALL {
                match cell.neighbor(dir) {
                    Ok(n) => prop_assert_eq!(n.neighbor(dir.opposite()), Ok(cell)),
                    Err(e) => {
                        prop_assert_eq!(e, CoordError::OutOfDomain);
                        let edge = cell.coords + dir.offset();
                        prop_assert!(
                            edge.min_element() < cell.level.cell_min()
                                || edge.max_element() > cell.level.cell_max()
                        );
                    }
                }
            }
        }

        #[test]
        fn aabb_min_lies_in_the_cell(cell in any_cell()) {
            let bounds = cell.aabb_world().unwrap();
            prop_assert_eq!(CellKey::containing(cell.level, &bounds.min), cell);
        }
    }

    #[test]
    fn spec_aabb_examples() {
        let x_of = |level, x| {
            let b = CellKey::new(level, I64Vec3::new(x, 0, 0))
                .aabb_world()
                .unwrap();
            (b.min.sector().x, b.min.local().x)
        };
        assert_eq!(x_of(Level::L2, -1), (-1, 512.0));
        assert_eq!(x_of(Level::L2, 6), (3, 0.0));
        assert_eq!(x_of(Level::L1, -1), (-4, 0.0));
        assert_eq!(x_of(Level::L3, -1), (-1, 960.0));
        assert_eq!(x_of(Level::L4, -3), (-1, 1000.0));
    }

    #[test]
    fn spec_parent_examples() {
        let p = |level, x| {
            CellKey::new(level, I64Vec3::new(x, 0, 0))
                .parent()
                .unwrap()
                .coords
                .x
        };
        assert_eq!(p(Level::L4, -3), -1);
        assert_eq!(p(Level::L3, -1), -1);
        assert_eq!(p(Level::L2, 6), 0);
        assert_eq!(CellKey::new(Level::L0, I64Vec3::ZERO).parent(), None);
    }

    #[test]
    fn domain_edge_neighbor() {
        let top = CellKey::new(Level::L0, I64Vec3::new((1 << 47) - 1, 0, 0));
        assert!(top.in_domain());
        assert_eq!(top.neighbor(Dir6::PosX), Err(CoordError::OutOfDomain));
        let worst = CellKey::new(Level::L4, I64Vec3::splat(i64::MAX));
        assert_eq!(worst.neighbor(Dir6::PosX), Err(CoordError::OutOfDomain));
        assert!(worst.children().is_err());
        assert!(worst.aabb_world().is_err());
    }

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
