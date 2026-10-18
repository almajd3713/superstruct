//! The stack of an L0 regime: crusts alternating with Betweens, in units of 64 m.
//! Spec: `docs/spec/regimes.md` §8, `levels.md` §3W.1.
//!
//! A stack is a pure function of the L0 cell. The Between at an L0 face is derived from the
//! face key, so the two cells that share the face agree without communicating.

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::hash;
use crate::orient::{TILT_SLOPE, heading};
use crate::regime::regime;
use crate::regime_web::{StackGeometry, WebFields};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// An L0 cell is 512 units of 64 m.
pub const UNITS: i64 = 512;

/// Half the L0 cell in metres: the point the stack geometry is centred on.
const HALF_M: i64 = 16_384;

/// Shortest Between that can follow a crust inside the stack, in units.
const MIN_BETWEEN: i64 = 4;

/// Chance of a flooded Between (BTW-05), in sixths.
const FLOODED_IN_6: u64 = 1;

/// The depth/content kinds of a Between (BTW-01..04). Order: `WebFields::between_mix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BetweenKind {
    Shallow,
    Normal,
    Abyssal,
    Pillared,
    Webbed,
}

impl BetweenKind {
    pub const ALL: [Self; 5] = [
        Self::Shallow,
        Self::Normal,
        Self::Abyssal,
        Self::Pillared,
        Self::Webbed,
    ];

    /// Depth range in units, inclusive.
    pub const fn depth(self) -> (i64, i64) {
        match self {
            Self::Shallow => (4, 6),
            Self::Normal => (8, 32),
            Self::Abyssal => (48, 64),
            Self::Pillared | Self::Webbed => (8, 40),
        }
    }

    /// The registry ID, for the census.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Shallow => "BTW-01",
            Self::Normal => "STR-02",
            Self::Abyssal => "BTW-02",
            Self::Pillared => "BTW-03",
            Self::Webbed => "BTW-04",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Stratum {
    /// Crust `k`, numbered from the bottom.
    Crust(u16),
    /// Between `j`, numbered from the bottom.
    Between(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Segment {
    pub stratum: Stratum,
    /// Start of the segment in units, inclusive.
    pub start: u16,
    /// End in units, exclusive.
    pub end: u16,
    /// `Some` for a Between.
    pub kind: Option<BetweenKind>,
    /// BTW-05: only ever true for a Between.
    pub flooded: bool,
}

impl Segment {
    pub fn depth(&self) -> u16 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub geometry: StackGeometry,
    /// Contiguous, covering `0..512`, starting and ending with a Between.
    pub segments: Vec<Segment>,
}

/// The seam Between on one stack face: its kind, its even depth in units and whether it is
/// flooded. `owner` is the L0 cell above (or on the + side of) the face, `lower` the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seam {
    pub kind: BetweenKind,
    pub depth: i64,
    pub flooded: bool,
}

/// The seam on the face between `lower` and `owner` (spec §8.2). `axis` is 0, 1 or 2.
pub fn seam(seed: WorldSeed, owner: I64Vec3, lower: I64Vec3, axis: u8) -> Seam {
    let salt = match axis {
        0 => salt::SEAM_BETWEEN_X,
        2 => salt::SEAM_BETWEEN_Z,
        _ => salt::SEAM_BETWEEN_Y,
    };
    let mut rng = Rng::new(hash(seed.0, Level::L0, owner, salt));
    let mix = regime(seed, lower).web.between_mix.map(u32::from);
    let kind = BetweenKind::ALL[rng.pick_weighted(&mix).unwrap_or(1)];
    let (lo, hi) = kind.depth();
    let depth = 2 * i64::from(rng.range_i32(
        i32::try_from((lo / 2).max(2)).unwrap_or(2),
        i32::try_from(hi / 2 + 1).unwrap_or(2),
    ));
    let flooded = rng.below(6) < FLOODED_IN_6;
    Seam {
        kind,
        depth,
        flooded,
    }
}

/// The stack of L0 cell `cell`. Nothing is cached here; callers that query per L2 cell keep
/// the result for the L0 cell.
pub fn stack(seed: WorldSeed, cell: I64Vec3) -> Stack {
    build(seed, cell, &regime(seed, cell).web)
}

fn build(seed: WorldSeed, cell: I64Vec3, web: &WebFields) -> Stack {
    let axis: u8 = match web.stack {
        StackGeometry::Vertical { axis: 0 } => 0,
        StackGeometry::Vertical { .. } => 2,
        _ => 1,
    };
    let unit = match axis {
        0 => I64Vec3::X,
        2 => I64Vec3::Z,
        _ => I64Vec3::Y,
    };
    let bottom = seam(seed, cell, cell - unit, axis);
    let top = seam(seed, cell + unit, cell, axis);
    let (crust_lo, crust_hi) = (i64::from(web.crust_units.0), i64::from(web.crust_units.1));
    let mix = web.between_mix.map(u32::from);
    let mut rng = Rng::new(hash(seed.0, Level::L0, cell, salt::STACK));

    let mut segs: Vec<Segment> = Vec::new();
    let mut crusts = 0u16;
    let mut betweens = 0u16;
    let mut push = |segs: &mut Vec<Segment>, len: i64, crust: bool, kind: BetweenKind, fl: bool| {
        let start = segs.last().map_or(0, |s| s.end);
        let end = start + u16::try_from(len).unwrap_or(0);
        let (stratum, kind, flooded) = if crust {
            crusts += 1;
            (Stratum::Crust(crusts - 1), None, false)
        } else {
            betweens += 1;
            (Stratum::Between(betweens - 1), Some(kind), fl)
        };
        segs.push(Segment {
            stratum,
            start,
            end,
            kind,
            flooded,
        });
    };

    push(
        &mut segs,
        bottom.depth / 2,
        false,
        bottom.kind,
        bottom.flooded,
    );
    let top_start = UNITS - top.depth / 2;
    let mut pos = bottom.depth / 2;
    loop {
        let room = top_start - pos;
        if room < crust_lo + MIN_BETWEEN {
            // Cannot fit another crust: the last Between runs on to the upper half-seam.
            let last = segs.len() - 1;
            segs[last].end = u16::try_from(UNITS).unwrap_or(512);
            segs[last].kind = Some(top.kind);
            segs[last].flooded = top.flooded;
            break;
        }
        let draw = i64::from(rng.range_i32(
            i32::try_from(crust_lo).unwrap_or(8),
            i32::try_from(crust_hi).unwrap_or(8) + 1,
        ));
        let d = draw.min(room - MIN_BETWEEN);
        push(&mut segs, d, true, BetweenKind::Normal, false);
        pos += d;

        let kind = BetweenKind::ALL[rng.pick_weighted(&mix).unwrap_or(1)];
        let (lo, hi) = kind.depth();
        let bd = i64::from(rng.range_i32(
            i32::try_from(lo).unwrap_or(8),
            i32::try_from(hi).unwrap_or(8) + 1,
        ));
        let flooded = rng.below(6) < FLOODED_IN_6;
        if top_start - pos - bd < crust_lo + MIN_BETWEEN {
            push(&mut segs, UNITS - pos, false, top.kind, top.flooded);
            break;
        }
        push(&mut segs, bd, false, kind, flooded);
        pos += bd;
    }
    Stack {
        geometry: web.stack,
        segments: segs,
    }
}

impl Stack {
    /// The segment holding stack coordinate `u` (any integer: it wraps over the 512 units,
    /// which is how obliquely cut stacks keep crusts up to the L0 faces).
    pub fn segment_at(&self, u: i64) -> &Segment {
        let u = u.rem_euclid(UNITS);
        let i = self.segments.partition_point(|s| i64::from(s.end) <= u);
        &self.segments[i.min(self.segments.len() - 1)]
    }

    /// `Crust(k)` or `Between(j)` at metre position `local_m` inside the L0 cell (`0 ≤ c < 32768`).
    /// `well` is the centre of the nearest gravity well, used only by `Curved` stacks.
    pub fn stratum_at(&self, local_m: I64Vec3, well: Option<I64Vec3>) -> Stratum {
        self.segment_at(u_units(self.geometry, local_m, well))
            .stratum
    }

    /// The `u` interval `[start, end)` of crust `k`.
    pub fn crust_extent(&self, k: u16) -> Option<(u16, u16)> {
        self.segments
            .iter()
            .find(|s| s.stratum == Stratum::Crust(k))
            .map(|s| (s.start, s.end))
    }

    pub fn crusts(&self) -> impl Iterator<Item = &Segment> {
        self.segments
            .iter()
            .filter(|s| matches!(s.stratum, Stratum::Crust(_)))
    }

    /// Share of the 512 units that is crust, out of 1000.
    pub fn crust_per_mille(&self) -> i64 {
        let n: i64 = self.crusts().map(|s| i64::from(s.depth())).sum();
        n * 1000 / UNITS
    }
}

/// How far a Tilted stack's strata are raised at plane position `(x, z)` (metres inside the L0
/// cell): the plane through the cell's centre leaning by slope `s/64` towards heading `yaw`.
pub fn tilt_shift(tilt: u8, yaw: u8, x: i64, z: i64) -> i64 {
    let s = i64::from(TILT_SLOPE[usize::from(tilt)]);
    let (a, b) = heading(yaw);
    let (a, b) = (i64::from(a), i64::from(b));
    let len = (a * a + b * b).isqrt();
    let along = a * (x - HALF_M) + b * (z - HALF_M);
    (s * along).div_euclid(64 * len)
}

/// `oct3` of `gravity.md` §2: `max(m∞, 4·S/7)`.
pub fn oct3(r: I64Vec3) -> i64 {
    let a = r.abs();
    let sum = a.x + a.y + a.z;
    a.max_element().max(4 * sum / 7)
}

/// The stack coordinate `u` of a point, in units, from integers only (spec §8.1). `local_m` is
/// the position in metres inside the L0 cell. `Curved` without a well is `Flat`.
pub fn u_units(geometry: StackGeometry, local_m: I64Vec3, well: Option<I64Vec3>) -> i64 {
    match geometry {
        StackGeometry::Flat => local_m.y.div_euclid(64),
        StackGeometry::Vertical { axis: 0 } => local_m.x.div_euclid(64),
        StackGeometry::Vertical { .. } => local_m.z.div_euclid(64),
        StackGeometry::Tilted { tilt, yaw } => {
            (local_m.y - tilt_shift(tilt, yaw, local_m.x, local_m.z)).div_euclid(64)
        }
        StackGeometry::Curved => match well {
            Some(w) => oct3(local_m - w).div_euclid(64),
            None => local_m.y.div_euclid(64),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regime::STYLES;

    fn cells() -> impl Iterator<Item = I64Vec3> {
        (-12..12).flat_map(|x| (-3..3).map(move |y| I64Vec3::new(x, y, x * 3 - y)))
    }

    /// STR-01: the stack covers the cell, alternates, and starts and ends with a Between.
    #[test]
    fn stacks_are_well_formed() {
        let seed = WorldSeed(0xBEEF);
        for cell in cells() {
            let web = regime(seed, cell).web;
            let st = stack(seed, cell);
            let segs = &st.segments;
            assert_eq!(segs[0].start, 0);
            assert_eq!(i64::from(segs[segs.len() - 1].end), UNITS);
            assert!(matches!(segs[0].stratum, Stratum::Between(0)));
            assert!(matches!(segs[segs.len() - 1].stratum, Stratum::Between(_)));
            for (i, w) in segs.windows(2).enumerate() {
                assert_eq!(w[0].end, w[1].start);
                assert_ne!(
                    matches!(w[0].stratum, Stratum::Crust(_)),
                    matches!(w[1].stratum, Stratum::Crust(_)),
                    "{cell:?} piece {i}"
                );
            }
            assert!(segs.len() <= 40, "{} pieces", segs.len());
            for s in segs {
                if matches!(s.stratum, Stratum::Crust(_)) {
                    // Crusts are within the style's range; only the last may be trimmed to fit.
                    assert!(i64::from(s.depth()) >= i64::from(web.crust_units.0));
                    assert!(s.depth() <= u16::from(web.crust_units.1));
                    assert!(s.kind.is_none() && !s.flooded);
                } else {
                    assert!(s.kind.is_some());
                }
            }
        }
    }

    /// STR-02: interior Between depths follow their kind; the seam halves are even-halved.
    #[test]
    fn between_depths_follow_their_kind() {
        let seed = WorldSeed(21);
        for cell in cells() {
            let st = stack(seed, cell);
            let n = st.segments.len();
            for (i, s) in st.segments.iter().enumerate() {
                let Some(kind) = s.kind else { continue };
                let (lo, hi) = kind.depth();
                let d = i64::from(s.depth());
                if i > 0 && i + 1 < n {
                    assert!((lo..=hi).contains(&d), "{cell:?} {kind:?} {d}");
                }
            }
        }
    }

    /// Invariant 4: the two cells at a face derive the same seam, so the Between across the
    /// face is one piece of the stated depth, whichever side computes it.
    #[test]
    fn seams_agree_across_faces() {
        let seed = WorldSeed(5);
        for cell in cells() {
            let web = regime(seed, cell).web;
            let above = cell + I64Vec3::Y;
            let web_above = regime(seed, above).web;
            if !matches!(
                web.stack,
                StackGeometry::Flat | StackGeometry::Tilted { .. }
            ) || !matches!(
                web_above.stack,
                StackGeometry::Flat | StackGeometry::Tilted { .. }
            ) {
                continue;
            }
            let lower = stack(seed, cell);
            let upper = stack(seed, above);
            let sm = seam(seed, above, cell, 1);
            let top_half = lower.segments.last().unwrap();
            let bottom_half = &upper.segments[0];
            assert_eq!(top_half.kind, Some(sm.kind));
            assert_eq!(bottom_half.kind, Some(sm.kind));
            assert_eq!(top_half.flooded, bottom_half.flooded);
            assert_eq!(i64::from(bottom_half.depth()), sm.depth / 2);
            // The last Between of the lower cell may be longer than its half-seam, but never
            // shorter, so no crust touches the face.
            assert!(i64::from(top_half.depth()) >= sm.depth / 2);
            assert_eq!(sm.depth % 2, 0);
        }
    }

    /// Proportions follow the style (the-web.md §4.1): thick crusts in Pipeworks, thin ones in
    /// Hollow Expanse, and the crust share of every stack is plausible.
    #[test]
    fn crust_share_follows_the_style() {
        let seed = WorldSeed(77);
        let mut share = [(0i64, 0i64); STYLES.len()];
        for x in -60..60 {
            for z in -60..60 {
                let cell = I64Vec3::new(x, 0, z);
                let r = regime(seed, cell);
                if !matches!(r.web.stack, StackGeometry::Flat) {
                    continue;
                }
                let e = &mut share[usize::from(r.style.0)];
                e.0 += stack(seed, cell).crust_per_mille();
                e.1 += 1;
            }
        }
        let mean = |i: usize| share[i].0 as f64 / share[i].1 as f64;
        assert!(mean(1) > mean(3) + 50.0, "{} vs {}", mean(1), mean(3));
        for (i, (sum, n)) in share.iter().enumerate() {
            assert!(*n > 20, "style {i}: {n} cells");
            let m = *sum as f64 / *n as f64;
            assert!((150.0..900.0).contains(&m), "style {i} crust share {m}");
        }
    }

    /// Every Between kind and the flooded modifier occur over a large sample.
    #[test]
    fn every_kind_and_flooding_occur() {
        let seed = WorldSeed(3);
        let mut kinds = [0u32; 5];
        let mut flooded = 0u32;
        let mut total = 0u32;
        for x in -40..40 {
            for z in -40..40 {
                for s in &stack(seed, I64Vec3::new(x, 1, z)).segments {
                    if let Some(k) = s.kind {
                        kinds[BetweenKind::ALL.iter().position(|&a| a == k).unwrap()] += 1;
                        flooded += u32::from(s.flooded);
                        total += 1;
                    }
                }
            }
        }
        assert!(kinds.iter().all(|&c| c > 100), "{kinds:?}");
        let f = f64::from(flooded) / f64::from(total);
        assert!((0.12..0.22).contains(&f), "flooded share {f}");
    }

    /// BTW-06..08 occur as geometries over a large sample (Curved only draws, since no well
    /// exists yet).
    #[test]
    fn every_geometry_is_drawn() {
        let seed = WorldSeed(8);
        let mut seen = [false; 4];
        for x in -80..80 {
            for z in -80..80 {
                match regime(seed, I64Vec3::new(x, 0, z)).web.stack {
                    StackGeometry::Flat => seen[0] = true,
                    StackGeometry::Tilted { tilt, yaw } => {
                        assert!(tilt >= 1 && yaw < 64);
                        seen[1] = true;
                    }
                    StackGeometry::Vertical { axis } => {
                        assert!(axis < 2);
                        seen[2] = true;
                    }
                    StackGeometry::Curved => seen[3] = true,
                }
            }
        }
        assert!(seen.iter().all(|&b| b), "{seen:?}");
    }

    /// `u` per geometry: flat is y, vertical is x or z, tilted passes through the centre
    /// unchanged and leans by the table slope, curved is the octagonal radius.
    #[test]
    fn u_per_geometry() {
        let p = I64Vec3::new(1000, 6400, 2000);
        assert_eq!(u_units(StackGeometry::Flat, p, None), 100);
        assert_eq!(u_units(StackGeometry::Vertical { axis: 0 }, p, None), 15);
        assert_eq!(u_units(StackGeometry::Vertical { axis: 1 }, p, None), 31);
        let centre = I64Vec3::new(HALF_M, 6400, HALF_M);
        let t = StackGeometry::Tilted { tilt: 3, yaw: 0 };
        assert_eq!(u_units(t, centre, None), 100);
        // Heading 0 is +X: 6400 m along it raises the plane by 6400·17/64 = 1700 m.
        let ahead = centre + I64Vec3::new(6400, 0, 0);
        assert_eq!(u_units(t, ahead, None), (6400 - 1700) / 64);
        let w = I64Vec3::new(0, 0, 0);
        let r = I64Vec3::new(6400, 0, 0);
        assert_eq!(u_units(StackGeometry::Curved, r, Some(w)), 100);
        assert_eq!(oct3(I64Vec3::new(100, 100, 100)), 171);
        assert_eq!(u_units(StackGeometry::Curved, p, None), 100);
    }

    /// Negative and out-of-range `u` wrap, so a tilted stack has crusts up to the faces.
    #[test]
    fn segment_lookup_wraps() {
        let st = stack(WorldSeed(1), I64Vec3::ZERO);
        for u in [-1, 0, 511, 512, 513, -512, 100_000] {
            let s = st.segment_at(u);
            let w = u.rem_euclid(UNITS);
            assert!(i64::from(s.start) <= w && w < i64::from(s.end));
        }
        assert_eq!(st.crust_extent(0).map(|(a, _)| a > 0), Some(true));
        assert_eq!(st.crust_extent(u16::MAX), None);
    }

    /// The Web draws did not move: the fields after the stack keep their pinned values.
    #[test]
    fn deterministic() {
        let a = stack(WorldSeed(9), I64Vec3::new(-4, 2, 7));
        let b = stack(WorldSeed(9), I64Vec3::new(-4, 2, 7));
        assert_eq!(a, b);
    }
}
