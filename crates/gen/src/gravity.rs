//! The gravity field `g(p)`: keyed wells, integer metrics, fixed-point coverage.
//! Spec: `docs/spec/gravity.md` §1–3.

// Draws are bounded (below 2^20) and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use glam::{I64Vec3, Vec3};

use crate::cavity::is_cavity;
use crate::cell::{CellKey, Level};
use crate::coord::SECTOR_SIZE;
use crate::hash::hash;
use crate::orient::{DIR_COUNT, DIRS};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L0_SIZE: i64 = 32768;
const L1_SIZE: i64 = 4096;
const STD: i64 = 256;
const DEFAULT_UP: [i64; 3] = [0, 8, 0];
const PATCH: i64 = 256;

/// `up` is the direction of anti-gravity, max-norm 8, or all zero in a lull.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gravity {
    pub up: [i8; 3],
    pub strength: u16,
}

impl Gravity {
    pub const DEFAULT: Gravity = Gravity {
        up: [0, 8, 0],
        strength: 256,
    };

    pub const fn is_lull(&self) -> bool {
        self.up[0] == 0 && self.up[1] == 0 && self.up[2] == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellKind {
    Point,
    Shell,
    Line,
    Tube,
    Plane,
    Twin,
    Seam,
    Fold,
    Damaged,
    Tidal,
}

const KIND_WEIGHTS: [u32; 10] = [20, 8, 22, 12, 12, 8, 5, 5, 5, 3];
const KINDS: [WellKind; 10] = [
    WellKind::Point,
    WellKind::Shell,
    WellKind::Line,
    WellKind::Tube,
    WellKind::Plane,
    WellKind::Twin,
    WellKind::Seam,
    WellKind::Fold,
    WellKind::Damaged,
    WellKind::Tidal,
];

/// One fold segment: start, table-free axis (0..3), sign, length, up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldSegment {
    pub start: [i64; 3],
    pub axis: usize,
    pub sign: i64,
    pub len: i64,
    pub up: [i64; 3],
}

/// A single field source in absolute world metres. A Twin is stored as two Point wells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Well {
    pub kind: WellKind,
    pub centre: [i64; 3],
    pub axis: usize,
    pub reach: i64,
    pub strength: i64,
    pub extent: [i64; 3],
    pub l1: I64Vec3,
    pub fold: Vec<FoldSegment>,
    /// A quiet lull's twins: they leave no planetoid and no furnace (`rare.rs`).
    pub quiet: bool,
}

fn oct3(r: [i64; 3]) -> i64 {
    let a = [r[0].abs(), r[1].abs(), r[2].abs()];
    let m = a[0].max(a[1]).max(a[2]);
    m.max(4 * (a[0] + a[1] + a[2]) / 7)
}

fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = i64::midpoint(x, n / x);
    }
    x
}

/// Spec §1.1: nearest-integer quantisation to max-norm 8 with floor division.
pub(crate) fn quantise(v: [i64; 3]) -> [i64; 3] {
    let m = v[0].abs().max(v[1].abs()).max(v[2].abs());
    if m == 0 {
        return [0; 3];
    }
    [0, 1, 2].map(|i| (16 * v[i] + m).div_euclid(2 * m))
}

pub(crate) fn smooth_coverage(reach: i64, d: i64) -> i64 {
    let t = ((reach - d) * 4 * 4096 / reach).clamp(0, 4096);
    let s = t * t * (3 * 4096 - 2 * t) / (4096 * 4096);
    s >> 4
}

pub(crate) fn dir(axis: usize) -> [i64; 3] {
    DIRS[axis % DIR_COUNT].map(i64::from)
}

fn dot(a: [i64; 3], b: [i64; 3]) -> i64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn neg(a: [i64; 3]) -> [i64; 3] {
    a.map(|x| -x)
}

/// `r` minus its component along the table axis `a`, scaled by `|a|²` (§2.1).
fn perp(r: [i64; 3], a: [i64; 3]) -> ([i64; 3], i64) {
    let aa = dot(a, a);
    let ra = dot(r, a);
    ([0, 1, 2].map(|i| aa * r[i] - ra * a[i]), aa)
}

fn box_distance(r: [i64; 3], extent: [i64; 3]) -> i64 {
    (0..3).map(|i| r[i].abs() - extent[i]).max().unwrap_or(0)
}

/// Coverage 0..=256 and the contribution vector direction (unnormalised) with strength.
struct Contribution {
    coverage: i64,
    strength: i64,
    up: [i64; 3],
}

impl Well {
    fn contribution(&self, seed: WorldSeed, p: [i64; 3]) -> Contribution {
        let r = sub(p, self.centre);
        let zero = Contribution {
            coverage: 0,
            strength: 0,
            up: [0; 3],
        };
        match self.kind {
            WellKind::Point | WellKind::Twin | WellKind::Shell | WellKind::Tidal => {
                let d = oct3(r);
                if d >= self.reach {
                    return zero;
                }
                let up = if self.kind == WellKind::Point || self.kind == WellKind::Twin {
                    quantise(r)
                } else {
                    quantise(neg(r))
                };
                let mut strength = self.strength;
                if self.kind == WellKind::Tidal {
                    strength = strength * (self.reach - d) / self.reach;
                }
                Contribution {
                    coverage: smooth_coverage(self.reach, d),
                    strength,
                    up,
                }
            }
            WellKind::Line | WellKind::Tube => {
                let a = dir(self.axis);
                let (pr, aa) = perp(r, a);
                let radial = oct3(pr) / aa.max(1);
                let along = dot(r, a).abs() / isqrt(aa).max(1);
                let d = radial.max(along - self.extent[1]);
                if d >= self.reach {
                    return zero;
                }
                let up = if self.kind == WellKind::Line {
                    quantise(pr)
                } else {
                    quantise(neg(pr))
                };
                Contribution {
                    coverage: smooth_coverage(self.reach, d),
                    strength: self.strength,
                    up,
                }
            }
            WellKind::Plane | WellKind::Seam => {
                let d = box_distance(r, self.extent);
                if d >= self.reach {
                    return zero;
                }
                let n = dir(self.axis);
                let mut up = [n[0] * 4, n[1] * 4, n[2] * 4];
                let mut coverage = smooth_coverage(self.reach, d);
                if self.kind == WellKind::Seam {
                    if dot(r, n) < 0 {
                        up = neg(up);
                    }
                    // no taper on the shared plane: only the outer edges taper
                    let outer = (0..3)
                        .map(|i| r[i].abs() - self.extent[i])
                        .max()
                        .unwrap_or(0);
                    coverage = smooth_coverage(self.reach, outer);
                }
                Contribution {
                    coverage,
                    strength: self.strength,
                    up,
                }
            }
            WellKind::Fold => {
                let mut best: Option<(i64, [i64; 3])> = None;
                for s in &self.fold {
                    let rel = sub(p, s.start);
                    let along = (rel[s.axis] * s.sign).clamp(0, s.len);
                    let mut off = rel;
                    off[s.axis] -= along * s.sign;
                    let d = oct3(off);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, s.up));
                    }
                }
                match best {
                    Some((d, up)) if d < self.reach => Contribution {
                        coverage: smooth_coverage(self.reach, d),
                        strength: self.strength,
                        up: up.map(|x| x * 8),
                    },
                    _ => zero,
                }
            }
            WellKind::Damaged => {
                let d = oct3(r);
                if d >= self.reach {
                    return zero;
                }
                let patch = I64Vec3::new(
                    p[0].div_euclid(PATCH),
                    p[1].div_euclid(PATCH),
                    p[2].div_euclid(PATCH),
                );
                let h = hash(seed.0, Level::L0, patch, salt::WELL_PATCH);
                let mut rng = Rng::new(h);
                let up = dir(rng.below(DIR_COUNT as u64) as usize);
                let up = if rng.below(2) == 0 { up } else { neg(up) };
                let strength = 96 + 16 * rng.below(11) as i64;
                Contribution {
                    coverage: smooth_coverage(self.reach, d),
                    strength,
                    up: up.map(|x| x * 4),
                }
            }
        }
    }
}

fn l0_origin(l0: I64Vec3) -> [i64; 3] {
    [l0.x * L0_SIZE, l0.y * L0_SIZE, l0.z * L0_SIZE]
}

fn live_centre(
    seed: WorldSeed,
    l0: I64Vec3,
    rng: &mut Rng,
    around: Option<[i64; 3]>,
) -> Option<([i64; 3], I64Vec3)> {
    let o = l0_origin(l0);
    for _ in 0..8 {
        let c = match around {
            None => [0, 1, 2].map(|i| o[i] + L1_SIZE + 16 * rng.below(6 * 256) as i64),
            Some(c0) => {
                let d = dir(rng.below(DIR_COUNT as u64) as usize);
                let len = 3000 + 16 * rng.below(9000 / 16) as i64;
                let n = isqrt(dot(d, d)).max(1);
                [0, 1, 2].map(|i| c0[i] + d[i] * len / n)
            }
        };
        let l1 = I64Vec3::new(
            c[0].div_euclid(L1_SIZE),
            c[1].div_euclid(L1_SIZE),
            c[2].div_euclid(L1_SIZE),
        );
        if !is_cavity(seed, CellKey::new(Level::L1, l1)) {
            return Some((c, l1));
        }
    }
    None
}

/// The wells a rare event sets in place of the ordinary draw: a planetoid, a Möbius corridor or
/// the twins of a quiet lull (`rare.rs`).
fn rare_wells(seed: WorldSeed, l0: I64Vec3) -> Option<Vec<Well>> {
    use crate::rare::{Rare, site_of};
    let site = site_of(seed, l0)?;
    let c = [0, 1, 2].map(|i| site.l1[i] * L1_SIZE + L1_SIZE / 2);
    if is_cavity(seed, CellKey::new(Level::L1, site.l1)) {
        return None;
    }
    let base = Well {
        kind: WellKind::Point,
        centre: c,
        axis: 0,
        reach: 9000,
        strength: 256,
        extent: [0; 3],
        l1: site.l1,
        fold: Vec::new(),
        quiet: false,
    };
    match site.kind {
        Rare::Planetoid => Some(vec![base]),
        Rare::QuietLull => {
            // Equal and opposed: the middle of the chamber is a lull.
            let pair = |dx: i64| Well {
                kind: WellKind::Twin,
                centre: [c[0] + dx, c[1], c[2]],
                reach: 6000,
                strength: 160,
                quiet: true,
                ..base.clone()
            };
            Some(vec![pair(-200), pair(200)])
        }
        Rare::Moebius => {
            // A square loop of 1.2 km, upright for two sides and upside down for two.
            let side = 1200;
            let start = [c[0] - side / 2, c[1], c[2] - side / 2];
            let legs = [(0, 1, 1), (2, 1, 1), (0, -1, -1), (2, -1, -1)];
            let mut at = start;
            let mut fold = Vec::new();
            for (axis, sign, up) in legs {
                fold.push(FoldSegment {
                    start: at,
                    axis,
                    sign,
                    len: side,
                    up: [0, up, 0],
                });
                at[axis] += sign * side;
            }
            Some(vec![Well {
                kind: WellKind::Fold,
                reach: 300,
                strength: 224,
                fold,
                ..base
            }])
        }
        _ => None,
    }
}

fn draw_wells(seed: WorldSeed, l0: I64Vec3) -> Vec<Well> {
    if let Some(wells) = rare_wells(seed, l0) {
        return wells;
    }
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::WELL));
    if !rng.chance(0.25) {
        return Vec::new();
    }
    let kind = KINDS[rng.pick_weighted(&KIND_WEIGHTS).unwrap_or(0)];
    let Some((centre, l1)) = live_centre(seed, l0, &mut rng, None) else {
        return Vec::new();
    };
    let mut reach = 2000 + 1000 * i64::from(rng.range_i32(0, 15));
    if matches!(kind, WellKind::Line | WellKind::Tube) {
        reach = reach.min(8000);
    }
    let strength = 96 + 16 * i64::from(rng.range_i32(0, 11));
    let strength = if kind == WellKind::Tidal {
        strength.max(128)
    } else {
        strength
    };
    let axis = rng.below(DIR_COUNT as u64) as usize;
    // A seam's membrane and a plane's deck are boxes, so their normal is a coordinate axis.
    let axis = if matches!(kind, WellKind::Seam | WellKind::Plane) {
        const AXES: [[i8; 3]; 3] = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
        DIRS.iter()
            .position(|d| *d == AXES[axis % 3])
            .unwrap_or(axis)
    } else {
        axis
    };
    let mut extent = [0i64; 3];
    if matches!(kind, WellKind::Plane | WellKind::Seam) {
        extent = [0, 1, 2].map(|_| 256 + 64 * rng.below(61) as i64);
    }
    if matches!(kind, WellKind::Line | WellKind::Tube) {
        extent[1] = 256 + 64 * rng.below(61) as i64;
    }
    let mut well = Well {
        kind,
        centre,
        axis,
        reach,
        strength,
        extent,
        l1,
        fold: Vec::new(),
        quiet: false,
    };
    let mut out = Vec::new();
    match kind {
        WellKind::Fold => {
            well.reach = well.reach.min(4000);
            let segs = 2 + rng.below(5) as usize;
            let mut at = centre;
            let mut prev_axis = 3usize;
            let mut up_axis = 1usize;
            for _ in 0..segs {
                let mut ax = rng.below(3) as usize;
                if ax == prev_axis {
                    ax = (ax + 1) % 3;
                }
                let sign = if rng.below(2) == 0 { 1 } else { -1 };
                let len = 1000 + 16 * rng.below(3000 / 16) as i64;
                if ax == up_axis {
                    up_axis = (ax + 1 + rng.below(2) as usize) % 3;
                }
                let mut up = [0i64; 3];
                up[up_axis] = if rng.below(2) == 0 { 1 } else { -1 };
                well.fold.push(FoldSegment {
                    start: at,
                    axis: ax,
                    sign,
                    len,
                    up,
                });
                at[ax] += sign * len;
                prev_axis = ax;
            }
            out.push(well);
        }
        WellKind::Twin => {
            let partner = live_centre(seed, l0, &mut rng, Some(centre));
            out.push(well.clone());
            if let Some((c2, l1b)) = partner {
                out.push(Well {
                    centre: c2,
                    l1: l1b,
                    ..well
                });
            }
        }
        _ => out.push(well),
    }
    out
}

type Cache = Mutex<HashMap<(u64, I64Vec3), Arc<Vec<Well>>>>;

/// The wells keyed by one L0 cell (cached; the cache never changes a result).
pub fn wells_of(seed: WorldSeed, l0: I64Vec3) -> Arc<Vec<Well>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(w) = cache
        .lock()
        .ok()
        .and_then(|m| m.get(&(seed.0, l0)).cloned())
    {
        return w;
    }
    let wells = Arc::new(draw_wells(seed, l0));
    if let Ok(mut map) = cache.lock() {
        map.insert((seed.0, l0), Arc::clone(&wells));
    }
    wells
}

/// Gravity at an absolute integer metre position.
pub fn field_at_metres(seed: WorldSeed, p: [i64; 3]) -> Gravity {
    let home = I64Vec3::new(
        p[0].div_euclid(L0_SIZE),
        p[1].div_euclid(L0_SIZE),
        p[2].div_euclid(L0_SIZE),
    );
    let mut contribs: Vec<Contribution> = Vec::new();
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let l0 = home + I64Vec3::new(dx, dy, dz);
                for w in wells_of(seed, l0).iter() {
                    let c = w.contribution(seed, p);
                    if c.coverage > 0 {
                        contribs.push(c);
                    }
                }
            }
        }
    }
    let total: i64 = contribs.iter().map(|c| c.coverage).sum();
    if total == 0 {
        return Gravity::DEFAULT;
    }
    let scale = |c: i64| if total > STD { c * STD / total } else { c };
    let covered: i64 = contribs.iter().map(|c| scale(c.coverage)).sum();
    let def = (STD - covered).max(0);
    let mut v = [0i64; 3];
    for i in 0..3 {
        v[i] = def * STD * DEFAULT_UP[i];
    }
    for c in &contribs {
        let k = scale(c.coverage) * c.strength;
        let m = c.up[0].abs().max(c.up[1].abs()).max(c.up[2].abs()).max(1);
        // normalise to max-norm 8 before weighting
        for (vi, ci) in v.iter_mut().zip(c.up) {
            *vi += k * ci * 8 / m;
        }
    }
    let m = v[0].abs().max(v[1].abs()).max(v[2].abs());
    let strength = (m / 2048).min(256);
    if strength < 4 {
        return Gravity {
            up: [0; 3],
            strength: strength as u16,
        };
    }
    let q = quantise(v);
    Gravity {
        up: q.map(|x| x as i8),
        strength: strength as u16,
    }
}

/// The centre of the Point or Shell well nearest to the middle of L0 cell `l0`, in metres from
/// the cell's origin, for the radial crusts of a `Curved` stack (spec §3.3, GRV-02).
pub fn radial_well(seed: WorldSeed, l0: I64Vec3) -> Option<I64Vec3> {
    let o = l0_origin(l0);
    let mid = [o[0] + L0_SIZE / 2, o[1] + L0_SIZE / 2, o[2] + L0_SIZE / 2];
    let mut best: Option<(i64, [i64; 3])> = None;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                for w in wells_of(seed, l0 + I64Vec3::new(dx, dy, dz)).iter() {
                    if !matches!(w.kind, WellKind::Point | WellKind::Shell) {
                        continue;
                    }
                    let d = oct3(sub(mid, w.centre));
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, w.centre));
                    }
                }
            }
        }
    }
    best.map(|(_, c)| I64Vec3::new(c[0] - o[0], c[1] - o[1], c[2] - o[2]))
}

/// Gravity at an absolute integer metre position, with the gate zones of its L2 cell applied.
pub fn gravity_at_metres(seed: WorldSeed, p: [i64; 3]) -> Gravity {
    let base = field_at_metres(seed, p);
    crate::gates::apply_gates(seed, p, base)
}

/// Gravity at a world position given as sector and local offset.
#[allow(clippy::cast_possible_truncation)] // local < 1024 after normalisation
pub fn gravity_at(seed: WorldSeed, sector: I64Vec3, local: Vec3) -> Gravity {
    let s = SECTOR_SIZE as i64;
    let p = [0, 1, 2].map(|i| sector[i] * s + local[i].floor() as i64);
    gravity_at_metres(seed, p)
}

/// The centre (metres) of a well whose furnace is in L1 cell `l1`, if any (spec §3.3).
pub fn well_centre_in_l1(seed: WorldSeed, l1: I64Vec3) -> Option<[i64; 3]> {
    let l0 = I64Vec3::new(l1.x.div_euclid(8), l1.y.div_euclid(8), l1.z.div_euclid(8));
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let found = wells_of(seed, l0 + I64Vec3::new(dx, dy, dz))
                    .iter()
                    .find(|w| w.l1 == l1 && !w.quiet)
                    .map(|w| w.centre);
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    None
}

/// The `(yaw, tilt)` of the table frame whose up is nearest to the field direction `up`, for a
/// tilt of at most 30° (the table's range). `None` when the field is a lull, the default, or
/// turned further than 90° from the world up, where no table frame has its floor under you.
pub fn pose_for_up(up: [i8; 3]) -> Option<(u8, u8)> {
    use crate::orient::{Frame, TILTS};
    if up == [0, 0, 0] || up == [0, 8, 0] || up[1] <= 0 {
        return None;
    }
    let len = up
        .iter()
        .map(|&v| f64::from(v) * f64::from(v))
        .sum::<f64>()
        .sqrt();
    let u = glam::DVec3::new(f64::from(up[0]), f64::from(up[1]), f64::from(up[2])) / len;
    let mut best: Option<(f64, u8, u8)> = None;
    for tilt in 0..TILTS {
        for yaw in 0..64u8 {
            let d = Frame::table(yaw, tilt).axes()[1].dot(u);
            if best.is_none_or(|(bd, _, _)| d > bd + 1e-12) {
                best = Some((d, yaw, tilt));
            }
        }
    }
    best.map(|(_, yaw, tilt)| (yaw, tilt))
}

/// The first well of any L0 cell around `l0` within `radius` cells, for tests and tools.
pub fn find_well(seed: WorldSeed, kind: WellKind, search: i64) -> Option<(I64Vec3, Well)> {
    for r in 0..=search {
        for z in -r..=r {
            for y in -r..=r {
                for x in -r..=r {
                    if x.abs().max(y.abs()).max(z.abs()) != r {
                        continue;
                    }
                    let l0 = I64Vec3::new(x, y, z);
                    if let Some(w) = wells_of(seed, l0).iter().find(|w| w.kind == kind) {
                        return Some((l0, w.clone()));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    #[test]
    fn quantise_is_nearest_max_norm_8() {
        assert_eq!(quantise([0, 5, 0]), [0, 8, 0]);
        assert_eq!(quantise([-3, -3, 0]), [-8, -8, 0]);
        assert_eq!(quantise([1, 100, 0]), [0, 8, 0]);
    }

    #[test]
    fn default_field_is_up_everywhere_without_wells() {
        let g = gravity_at_metres(SEED, [1_000_000_000, 5, -1_000_000_000]);
        // far away may still hold a well; just require the invariant on a quiet cell
        assert!(g.strength > 0);
        let quiet = (0..2000).map(|i| i * 40_000).find_map(|x| {
            let p = [x, 0, 0];
            (gravity_at_metres(SEED, p) == Gravity::DEFAULT).then_some(p)
        });
        assert!(quiet.is_some());
    }

    #[test]
    fn pure_across_threads() {
        let probes: Vec<[i64; 3]> = (0..64)
            .map(|i| [i * 9_137, i * 4_001 - 70_000, i * 6_151])
            .collect();
        let a: Vec<Gravity> = probes.iter().map(|&p| gravity_at_metres(SEED, p)).collect();
        let b = std::thread::scope(|s| {
            let h = s.spawn(|| {
                probes
                    .iter()
                    .rev()
                    .map(|&p| gravity_at_metres(SEED, p))
                    .collect::<Vec<_>>()
            });
            h.join().unwrap()
        });
        let b: Vec<Gravity> = b.into_iter().rev().collect();
        assert_eq!(a, b);
    }

    #[test]
    fn point_well_points_away_from_centre() {
        let (_, w) = find_well(SEED, WellKind::Point, 6).expect("a point well exists");
        let p = [w.centre[0] + 200, w.centre[1], w.centre[2]];
        let g = gravity_at_metres(SEED, p);
        assert_eq!(g.up, [8, 0, 0]);
        assert!(g.strength >= 96);
    }

    #[test]
    fn shell_centre_is_a_lull() {
        let (_, w) = find_well(SEED, WellKind::Shell, 8).expect("a shell well exists");
        let g = gravity_at_metres(SEED, w.centre);
        assert!(g.is_lull(), "{g:?}");
        let near = gravity_at_metres(SEED, [w.centre[0] + 400, w.centre[1], w.centre[2]]);
        assert_eq!(near.up, [-8, 0, 0]);
    }

    #[test]
    fn twin_midpoint_is_a_lull() {
        let mut found = false;
        for z in -6..=6 {
            for y in -6..=6 {
                for x in -6..=6 {
                    let ws = wells_of(SEED, I64Vec3::new(x, y, z));
                    if ws.len() == 2
                        && ws[0].kind == WellKind::Twin
                        && ws[0].strength == ws[1].strength
                    {
                        let (a, b) = (ws[0].centre, ws[1].centre);
                        if oct3(sub(a, b)) / 2 >= ws[0].reach * 3 / 4 {
                            continue;
                        }
                        let mid = [0, 1, 2].map(|i| i64::midpoint(a[i], b[i]));
                        let g = gravity_at_metres(SEED, mid);
                        assert!(g.is_lull() || g.strength < 256, "{g:?}");
                        found = true;
                    }
                }
            }
        }
        assert!(found, "no equal-strength twin pair found in the search box");
    }

    #[test]
    fn taper_turns_at_most_one_step_per_metre() {
        let (_, w) = find_well(SEED, WellKind::Point, 6).expect("a point well exists");
        let mut prev =
            gravity_at_metres(SEED, [w.centre[0] + w.reach / 2, w.centre[1], w.centre[2]]);
        let mut worst = 0;
        for x in (w.reach / 2)..(w.reach + 64) {
            let g = gravity_at_metres(SEED, [w.centre[0] + x, w.centre[1], w.centre[2]]);
            let step = (0..3)
                .map(|i| (i32::from(g.up[i]) - i32::from(prev.up[i])).abs())
                .max()
                .unwrap_or(0);
            worst = worst.max(step);
            prev = g;
        }
        assert!(worst <= 1, "up jumped {worst} steps in one metre");
    }

    #[test]
    fn wells_occur_about_a_quarter_of_cells() {
        let mut n = 0;
        for x in 0..20 {
            for z in 0..20 {
                if !wells_of(SEED, I64Vec3::new(x, 0, z)).is_empty() {
                    n += 1;
                }
            }
        }
        assert!((60..=140).contains(&n), "{n} of 400");
    }

    #[test]
    fn pose_follows_the_field_up() {
        use crate::orient::Frame;
        assert_eq!(pose_for_up([0, 8, 0]), None);
        assert_eq!(pose_for_up([0, 0, 0]), None);
        assert_eq!(pose_for_up([8, 0, 0]), None);
        let (yaw, tilt) = pose_for_up([2, 8, 0]).unwrap();
        assert!(tilt > 0);
        let y = Frame::table(yaw, tilt).axes()[1];
        let want = glam::DVec3::new(2.0, 8.0, 0.0).normalize();
        assert!(y.dot(want) > 0.999, "{y:?}");
    }

    #[test]
    fn a_curved_stack_wraps_the_nearest_radial_well() {
        use crate::regime::regime;
        use crate::regime_web::StackGeometry;
        let mut found = 0;
        for x in -30..30 {
            for z in -30..30 {
                let l0 = I64Vec3::new(x, 0, z);
                if !matches!(regime(SEED, l0).web.stack, StackGeometry::Curved) {
                    continue;
                }
                if let Some(w) = radial_well(SEED, l0) {
                    assert_eq!(crate::stack::stack(SEED, l0).well, Some(w));
                    found += 1;
                }
            }
        }
        assert!(found > 0, "no curved stack has a radial well in 3600 cells");
    }

    /// Spec §3.3: the cell that holds a well's centre has the gravity furnace as its knot hub.
    #[test]
    fn a_well_cell_has_a_furnace_knot() {
        let mut checked = 0;
        for kind in [
            WellKind::Point,
            WellKind::Line,
            WellKind::Shell,
            WellKind::Plane,
        ] {
            let (_, w) = find_well(SEED, kind, 8).expect("a well");
            assert_eq!(well_centre_in_l1(SEED, w.l1), Some(w.centre));
            if let Some(sk) = crate::l1_artery::skeleton(SEED, w.l1) {
                assert_eq!(sk.knot_hub, crate::l1_frame::FURNACE_HUB);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}

#[cfg(test)]
mod viewpoints {
    use super::*;

    /// `cargo test -p gen --lib print_well_viewpoints -- --ignored --nocapture`: the first well
    /// of each kind and a spot to view it from, as `SUPERSTRUCT_SECTOR` / `SUPERSTRUCT_LOCAL`.
    #[test]
    #[ignore = "prints viewpoints for screenshots"]
    fn print_well_viewpoints() {
        let seed = WorldSeed(0);
        for kind in KINDS {
            let Some((_, w)) = find_well(seed, kind, 12) else {
                continue;
            };
            let off = match kind {
                WellKind::Point | WellKind::Twin => crate::wellgeom::core_half(&w) + 160,
                WellKind::Shell => crate::wellgeom::shell_half(&w) / 2,
                _ => 160,
            };
            let p = [w.centre[0] + off, w.centre[1] + 40, w.centre[2] + off];
            let s = [0, 1, 2].map(|i| p[i].div_euclid(1024));
            let l = [0, 1, 2].map(|i| p[i].rem_euclid(1024));
            println!(
                "{kind:?}: centre {:?} reach {} strength {} axis {:?} SECTOR={},{},{} LOCAL={},{},{}",
                w.centre, w.reach, w.strength, DIRS[w.axis], s[0], s[1], s[2], l[0], l[1], l[2]
            );
        }
    }
}
