//! World positions and coordinate math. Spec: `docs/spec/coordinates.md`.

use glam::{DVec3, I64Vec3, Vec3};

pub const SECTOR_SIZE: f32 = 1024.0;
pub const SECTOR_SIZE_LOG2: u32 = 10;

/// Valid sectors on every axis: `[SECTOR_MIN, SECTOR_MAX]`, i.e. `[−2⁶², 2⁶²)` metres
/// (`coordinates.md` §7). Half-open so cells of every level tile it exactly.
pub const SECTOR_MIN: i64 = -(1 << 52);
pub const SECTOR_MAX: i64 = (1 << 52) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CoordError {
    #[error("coordinate outside the world domain")]
    OutOfDomain,
    #[error("non-finite local offset")]
    NonFinite,
}

pub fn sector_in_domain(sector: I64Vec3) -> bool {
    sector.cmpge(I64Vec3::splat(SECTOR_MIN)).all() && sector.cmple(I64Vec3::splat(SECTOR_MAX)).all()
}

/// A position as an integer sector plus an `f32` offset inside it.
///
/// Invariant: every component of `local` is finite and in `[0, SECTOR_SIZE)`, and `sector` is
/// within `[SECTOR_MIN, SECTOR_MAX]`. Only [`WorldPos::new`] and the methods below construct one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldPos {
    sector: I64Vec3,
    local: Vec3,
}

impl WorldPos {
    pub const ORIGIN: Self = Self {
        sector: I64Vec3::ZERO,
        local: Vec3::ZERO,
    };

    /// Validates and normalizes `(sector, local)`.
    pub fn new(sector: I64Vec3, local: Vec3) -> Result<Self, CoordError> {
        if !sector_in_domain(sector) {
            return Err(CoordError::OutOfDomain);
        }
        normalize(sector, local)
    }

    pub fn sector(&self) -> I64Vec3 {
        self.sector
    }

    pub fn local(&self) -> Vec3 {
        self.local
    }

    /// Moves by `d` metres. Meant for per-frame motion; large jumps build a new `WorldPos`.
    pub fn offset(&self, d: Vec3) -> Result<Self, CoordError> {
        normalize(self.sector, self.local + d)
    }

    /// `self − other` in metres. Infallible: two in-domain sectors differ by at most 2⁵³.
    pub fn delta(&self, other: &Self) -> DVec3 {
        let ds = self.sector - other.sector;
        ds.as_dvec3() * f64::from(SECTOR_SIZE) + (self.local.as_dvec3() - other.local.as_dvec3())
    }
}

fn normalize(sector: I64Vec3, local: Vec3) -> Result<WorldPos, CoordError> {
    if !local.is_finite() {
        return Err(CoordError::NonFinite);
    }
    let mut out_sector = sector;
    let mut out_local = local;
    for axis in 0..3 {
        let (carry, l) = normalize_axis(local[axis]);
        out_sector[axis] = sector[axis]
            .checked_add(carry)
            .ok_or(CoordError::OutOfDomain)?;
        out_local[axis] = l;
    }
    if !sector_in_domain(out_sector) {
        return Err(CoordError::OutOfDomain);
    }
    Ok(WorldPos {
        sector: out_sector,
        local: out_local,
    })
}

/// Splits a finite offset into a sector carry and a local value in `[0, SECTOR_SIZE)`.
fn normalize_axis(local: f32) -> (i64, f32) {
    let k = (local / SECTOR_SIZE).floor();
    let mut l = local - k * SECTOR_SIZE;
    // Saturating cast is fine: an absurd carry fails the domain check afterwards.
    #[allow(clippy::cast_possible_truncation)]
    let mut carry = k as i64;
    // A tiny negative offset rounds up to exactly SECTOR_SIZE (coordinates.md §2.1).
    if l >= SECTOR_SIZE {
        l -= SECTOR_SIZE;
        carry = carry.saturating_add(1);
    }
    (carry, l)
}

#[cfg(test)]
// The spec pins exact values, so exact float equality is intended.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn wp(s: [i64; 3], l: [f32; 3]) -> WorldPos {
        WorldPos::new(I64Vec3::from_array(s), Vec3::from_array(l)).unwrap()
    }

    #[test]
    fn spec_normalization_table() {
        let cases: [(i64, f32, i64, f32); 5] = [
            (0, -0.5, -1, 1023.5),
            (0, 1030.25, 1, 6.25),
            (0, 2048.0, 2, 0.0),
            (0, -1e-7, 0, 0.0),
            (7, 1023.99999, 8, 0.0),
        ];
        for (s, l, es, el) in cases {
            let p = wp([s, 0, 0], [l, 0.0, 0.0]);
            assert_eq!((p.sector().x, p.local().x), (es, el), "input {s} {l}");
        }
    }

    #[test]
    fn crossing_both_directions_every_axis() {
        for axis in 0..3 {
            let mut start = [0.0f32; 3];
            start[axis] = 1023.0;
            let p = wp([5, -5, 0], start);

            let mut fwd = Vec3::ZERO;
            fwd[axis] = 2.0;
            let up = p.offset(fwd).unwrap();
            assert_eq!(up.sector()[axis], p.sector()[axis] + 1);
            assert_eq!(up.local()[axis], 1.0);

            let down = up.offset(-fwd * 1.5).unwrap();
            assert_eq!(down.sector()[axis], p.sector()[axis]);
            assert_eq!(down.local()[axis], 1022.0);

            let mut back = Vec3::ZERO;
            back[axis] = -1023.5;
            let below = p.offset(back).unwrap();
            assert_eq!(below.sector()[axis], p.sector()[axis] - 1);
            assert_eq!(below.local()[axis], 1023.5);
        }
    }

    #[test]
    fn spec_offset_example() {
        let p = wp([0, 0, 0], [3.0, 1.65, 0.25])
            .offset(Vec3::new(-5.0, 0.0, -0.5))
            .unwrap();
        assert_eq!(p.sector(), I64Vec3::new(-1, 0, -1));
        assert_eq!(p.local(), Vec3::new(1022.0, 1.65, 1023.75));
    }

    #[test]
    fn spec_delta_example() {
        let a = wp([-1, 0, 0], [1000.0, 0.0, 0.0]);
        let b = wp([2, 0, 0], [10.0, 0.0, 0.0]);
        assert_eq!(a.delta(&b), DVec3::new(-2082.0, 0.0, 0.0));
        assert_eq!(b.delta(&a), DVec3::new(2082.0, 0.0, 0.0));
    }

    #[test]
    fn delta_at_domain_extremes_is_exact() {
        let a = wp([SECTOR_MAX, 0, 0], [0.0; 3]);
        let b = wp([SECTOR_MIN, 0, 0], [0.0; 3]);
        assert_eq!(a.delta(&b).x, (2.0f64.powi(53) - 1.0) * 1024.0);
    }

    #[test]
    fn invariant_holds_after_new() {
        for l in [-1e-7f32, -1e-30, 1024.0, -1024.0, 5e6, -5e6] {
            let p = wp([0; 3], [l; 3]);
            let local = p.local();
            assert!(local.cmpge(Vec3::ZERO).all() && local.cmplt(Vec3::splat(SECTOR_SIZE)).all());
        }
    }

    #[test]
    fn rejects_non_finite() {
        for l in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                WorldPos::new(I64Vec3::ZERO, Vec3::new(0.0, l, 0.0)),
                Err(CoordError::NonFinite)
            );
        }
    }

    #[test]
    fn domain_edges() {
        let edge = wp([SECTOR_MAX, 0, 0], [1023.5, 0.0, 0.0]);
        assert_eq!(edge.offset(Vec3::X), Err(CoordError::OutOfDomain));
        let low = WorldPos::new(I64Vec3::new(SECTOR_MIN, 0, 0), Vec3::new(-0.5, 0.0, 0.0));
        assert_eq!(low, Err(CoordError::OutOfDomain));
        assert!(WorldPos::new(I64Vec3::new(SECTOR_MIN, 0, 0), Vec3::ZERO).is_ok());
        assert_eq!(
            WorldPos::new(I64Vec3::new(SECTOR_MAX + 1, 0, 0), Vec3::ZERO),
            Err(CoordError::OutOfDomain)
        );
    }
}
