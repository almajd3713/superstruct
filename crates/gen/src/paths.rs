//! Horizontal path types PH-01..10 (`levels.md` §5W.5, §5W.16): what a level route segment is
//! built as when it runs in free air, over a roof or beam, or beside a façade.
//!
//! A segment is cut into straight legs. Each leg is built in its own frame: `s` along the leg,
//! `t` across it and `y` up from the walking surface, which is the floor of the route's slots.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::cell::{Axis, Dir6, Level};
use crate::hash::hash;
use crate::modifiers::{self, Mods};
use crate::orient::LocalShape;
use crate::paths_v::{self, Column, VParams, build_rise, flat_yaw};
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::regime_web::PATH_WALK;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue::{SlotState, Surround, Tissue};
use crate::tissue_grow::{Emitter, Shaped, quanta};

/// Index of the ENCLOSED family in `Character::paths`.
const PATH_ENCLOSED: usize = 1;
/// Index of the STREET family.
const PATH_STREET: usize = 2;
/// Index of the STAIR family.
const PATH_STAIR_FAMILY: usize = 3;
/// Index of the LIFTSLOPE family (PV-11..19).
const PATH_LIFTSLOPE: usize = 4;
/// The ladder families (PW.5.3, `paths_v::ladders`).
const PATH_LADDER: usize = 5;

/// The ids of this module, PH-01..PH-20.
pub const IDS: [u8; 32] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 61, 62, 63, 64, 65, 66,
    67, 68, 69, 70, 71, 72,
];

/// The second batch (P2-01..12) takes ids 60 plus its number.
pub const P2_IDS: [u8; 12] = [61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72];

/// The path types chosen for a tissue and the pieces they emit.
#[derive(Debug, Clone, Default)]
pub struct PathPieces {
    /// `(segment, PH id)` for every segment that got a type.
    pub chosen: Vec<(u16, u8)>,
    /// The modifiers drawn for each chosen segment (MOD-01..09), in the same order.
    pub mods: Vec<(u16, Mods)>,
    /// Cell-local primitives.
    pub prims: Vec<Primitive>,
    /// The air each leg and each stair needs (the leg's walkway 1 m each side and 4 m high, the
    /// stair's footprint): structures that reach into it are cut back.
    pub corridors: Vec<Primitive>,
    /// Clearances of bored passages (PH-11, PH-12, PH-20 and any path in solid): air to cut
    /// out of the parent's solid where a route passes. Not drawn and not collidable.
    pub bores: Vec<Primitive>,
    /// Where each leg and rise is walked, for the defect census (PW.4.8). Not an output.
    pub walks: Vec<Walk>,
    /// The clear volume of every rise and the range of `prims` it built, for the census
    /// (PW.4.13). Not an output.
    pub rises: Vec<(Primitive, std::ops::Range<usize>)>,
    /// The clear box of every rise in lattice-frame metres, for the masses (PW.4.13).
    pub rise_clear: Vec<Clear>,
    /// The range of `prims` built for each walk, by index into `walks` (PW.4.15 census).
    pub owners: Vec<(std::ops::Range<usize>, usize)>,
}

/// The walking line of a leg (its two ends) or a rise (its samples), in cell-local metres on
/// the walking surface.
#[derive(Debug, Clone)]
pub struct Walk {
    /// The path type (PH-nn or PV-nn as numbered in the registry ids of `chosen`).
    pub id: u8,
    pub rise: bool,
    pub width: f64,
    pub points: Vec<glam::DVec3>,
    /// The frame's up, in cell-local space.
    pub up: glam::DVec3,
}

/// A clear box in lattice-frame metres: `(lo, hi)`.
pub type Clear = ([f64; 3], [f64; 3]);

/// The drawn modifiers of one segment (MOD-01 width, MOD-02 enclosure, MOD-03 condition).
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub width: f64,
    /// 0 bare, 1 one rail, 2 both rails.
    pub rails: u8,
    pub windowed: bool,
    /// −1 or +1: the side a wall or façade is on.
    pub side: f64,
    /// Fraction in `[0, 1)` for sizes (sag, amplitude, pipe radius, ceiling).
    pub k: f64,
    pub j: f64,
    /// NEAR-04: the grating deck of a catwalk has gaps that show the drop.
    pub gaps: bool,
}

fn frac(word: u64) -> f64 {
    (word >> 40) as f64 / f64::from(1u32 << 24)
}

/// The clear width range of each type, in metres (PH-01: 1.2–3 m).
pub fn width_range(id: u8) -> (f64, f64) {
    match id {
        2 => (3.0, 8.0),
        3 => (2.0, 5.0),
        4 | 5 => (1.5, 3.0),
        6 => (1.2, 2.5),
        7 => (0.9, 0.9),
        8 | 18 => (1.2, 2.0),
        9 | 11 | 16 => (3.0, 6.0),
        10 => (4.0, 10.0),
        12 => (1.2, 1.6),
        13 => (10.0, 28.0),
        14 => (2.0, 6.0),
        15 | 19 | 20 | 61 | 69 => (2.0, 4.0),
        17 => (1.2, 2.4),
        62 | 63 | 66 | 72 => (1.6, 3.0),
        64 => (2.4, 3.2),
        65 => (1.0, 1.6),
        67 => (0.9, 1.4),
        68 => (1.6, 2.4),
        70 => (1.6, 3.5),
        71 => (6.0, 10.0),
        _ => (1.2, 3.0),
    }
}

/// A straight run in lattice-frame metres: `start` is on the walking surface at the route's
/// centre line, `axis` is 0 (x) or 2 (z), `sign` the direction along it.
#[derive(Debug, Clone, Copy)]
pub struct Leg {
    pub start: [f64; 3],
    pub axis: usize,
    pub sign: f64,
    pub len: f64,
    /// How far from each end the side pieces (rails, walls, posts) stop: at a corner they would
    /// stand across the walkway of the leg that continues there.
    pub trim: [f64; 2],
}

impl Leg {
    pub fn map(&self, s: f64, t: f64, y: f64) -> [f64; 3] {
        let mut p = self.start;
        p[self.axis] += self.sign * s;
        p[2 - self.axis] += t;
        p[1] += y;
        p
    }

    fn dir4(&self, forward: bool) -> Dir4 {
        let plus = (self.sign > 0.0) == forward;
        match (self.axis, plus) {
            (0, true) => Dir4::PosX,
            (0, false) => Dir4::NegX,
            (_, true) => Dir4::PosZ,
            (_, false) => Dir4::NegZ,
        }
    }
}

pub(crate) struct Build<'a, 'b> {
    pub(crate) e: &'b mut Emitter<'a>,
    pub(crate) leg: Leg,
    pub(crate) clear: Vec<Clear>,
}

pub(crate) type S = (f64, f64);

impl Build<'_, '_> {
    /// A box over `s`, `t` and `y` ranges.
    pub(crate) fn bx(
        &mut self,
        s: S,
        t: S,
        y: S,
        m: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        let a = self.leg.map(s.0, t.0, y.0);
        let b = self.leg.map(s.1, t.1, y.1);
        let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        self.e.frame_box(lo, hi, m, kind, flags);
    }

    /// Marks a box of air to cut from solid parent matter.
    pub(crate) fn cut(&mut self, s: S, t: S, y: S) {
        let a = self.leg.map(s.0, t.0, y.0);
        let b = self.leg.map(s.1, t.1, y.1);
        self.clear.push((
            [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        ));
    }

    pub(crate) fn deck(&mut self, s: S, t: S, y: S, m: MaterialId) {
        self.bx(s, t, y, m, FeatureKind::WALKWAY, PrimFlags::NONE);
    }

    pub(crate) fn solid(&mut self, s: S, t: S, y: S, m: MaterialId) {
        self.bx(s, t, y, m, FeatureKind::BRIDGE, PrimFlags::NONE);
    }

    /// `s` cut to the part of the leg that side pieces may stand on, or `None` (see `Leg::trim`).
    pub(crate) fn clamp_side(&self, s: S) -> Option<S> {
        let (lo, hi) = (self.leg.trim[0], self.leg.len - self.leg.trim[1]);
        let (a, b) = (s.0.max(lo), s.1.min(hi));
        (a < b).then_some((a, b))
    }

    /// A rail, which stops short of the corners of the leg.
    pub(crate) fn rail(&mut self, s: S, t: S, y: S) {
        if let Some(s) = self.clamp_side(s) {
            self.bx(
                s,
                t,
                y,
                MaterialId::METAL,
                FeatureKind::RAILING,
                PrimFlags::NONE,
            );
        }
    }

    /// A wall or post at the side of the walkway, which stops short of the corners.
    pub(crate) fn side_solid(&mut self, s: S, t: S, y: S, m: MaterialId) {
        if let Some(s) = self.clamp_side(s) {
            self.solid(s, t, y, m);
        }
    }

    pub(crate) fn glass(&mut self, s: S, t: S, y: S) {
        self.bx(
            s,
            t,
            y,
            MaterialId::LIT_WINDOW,
            FeatureKind::WINDOW,
            PrimFlags::NO_COLLIDE,
        );
    }

    /// A thin line from `(s0, y0)` to `(s1, y1)` at `t`, as short boxes (cables are stepped).
    fn diag(&mut self, a: (f64, f64), b: (f64, f64), t: f64, thick: f64) {
        let n = (((b.0 - a.0).abs().max((b.1 - a.1).abs()) / 2.0).ceil() as usize).clamp(1, 12);
        for i in 0..n {
            let (u0, u1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
            let p = |u: f64| (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
            let (p0, p1) = (p(u0), p(u1));
            self.bx(
                (p0.0.min(p1.0) - thick / 2.0, p0.0.max(p1.0) + thick / 2.0),
                (t - thick / 2.0, t + thick / 2.0),
                (p0.1.min(p1.1) - thick / 2.0, p0.1.max(p1.1) + thick / 2.0),
                MaterialId::PIPE,
                FeatureKind::CABLE,
                PrimFlags::NO_COLLIDE,
            );
        }
    }

    /// A wedge over `s` rising from `y0` to `y1` (the roof is the walking surface).
    fn wedge(&mut self, s: S, t: S, y: (f64, f64)) {
        let (low, high) = (y.0.min(y.1), y.0.max(y.1));
        let a = self.leg.map(s.0, t.0, low);
        let b = self.leg.map(s.1, t.1, high);
        let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        if hi[1] - lo[1] < 1.0 / 16.0 {
            self.deck(s, t, (high - 0.4, high), MaterialId::GRATING);
            return;
        }
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(quanta(size[0]), quanta(size[1]), quanta(size[2])),
        );
        // Rising towards the end with the higher `y`.
        let rise = self.leg.dir4(y.1 >= y.0);
        let origin = Emitter::qv(self.e.at(lo));
        self.e.out.push(Primitive {
            shape: Shape::oriented(
                self.e.lat.frame,
                origin,
                LocalShape::Ramp { aabb: local, rise },
                self.e.cell,
            ),
            material: MaterialId::GRATING,
            kind: FeatureKind::WALKWAY,
            flags: PrimFlags::NONE,
        });
        // The underside overlaps the wedge by 4 cm so rounding to quanta leaves no gap.
        self.deck(s, t, (low - 0.4, low + 0.04), MaterialId::GRATING);
    }

    /// A cylinder along the leg with its top at `y = 0`.
    fn pipe(&mut self, s: S, radius: f64) {
        let a = self.leg.map(s.0, -radius, -2.0 * radius);
        let b = self.leg.map(s.1, radius, 0.0);
        let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let (axis, base, length) = if self.leg.axis == 0 {
            (
                Axis::X,
                QVec3::new(0, quanta(size[1] / 2.0), quanta(size[2] / 2.0)),
                size[0],
            )
        } else {
            (
                Axis::Z,
                QVec3::new(quanta(size[0] / 2.0), quanta(size[1] / 2.0), 0),
                size[2],
            )
        };
        let local = LocalShape::Cylinder {
            axis,
            base,
            length: quanta(length),
            radius: quanta(radius),
        };
        let origin = Emitter::qv(self.e.at(lo));
        self.e.out.push(Primitive {
            shape: Shape::oriented(self.e.lat.frame, origin, local, self.e.cell),
            material: MaterialId::PIPE,
            kind: FeatureKind::PIPE,
            flags: PrimFlags::NONE,
        });
    }

    /// A wall of height `h` at `t`, solid or with a band of glass.
    fn wall(&mut self, t: S, h: f64, len: f64, windowed: bool) {
        if !windowed {
            self.side_solid((0.0, len), t, (0.0, h), MaterialId::CONCRETE);
            return;
        }
        let lo = h.min(1.0);
        let hi = (h - 0.8).max(lo + 0.5);
        self.side_solid((0.0, len), t, (0.0, lo), MaterialId::CONCRETE);
        self.side_solid((0.0, len), t, (hi, h), MaterialId::CONCRETE);
        let mut s = 0.0;
        while s < len {
            let e = (s + 6.0).min(len);
            self.side_solid((s, s + 0.5), t, (lo, hi), MaterialId::CONCRETE);
            if let Some(g) = self.clamp_side((s + 0.5, e)) {
                self.glass(g, (t.0 + 0.1, t.1 - 0.1), (lo, hi));
            }
            s = e;
        }
    }

    fn posts(&mut self, len: f64, step: f64, t: f64, h: f64, w: f64) {
        let mut s = 0.0;
        while s <= len {
            self.side_solid(
                (s - w / 2.0, s + w / 2.0),
                (t - w / 2.0, t + w / 2.0),
                (-0.5, h),
                MaterialId::CONCRETE,
            );
            s += step;
        }
    }
}

/// The sag of a catenary of length `len` (PH-04), capped so the slope stays under 27°.
pub fn sag(len: f64, k: f64) -> f64 {
    (len * (0.04 + 0.04 * k)).clamp(3.0, 12.0)
}

/// Lateral offset of the run of a zigzag (PH-05): the ends run on the centre line.
fn zig_t(k: usize, runs: usize, amp: f64) -> f64 {
    if k == 0 || k + 1 == runs {
        0.0
    } else if k % 2 == 1 {
        amp
    } else {
        -amp
    }
}

/// The zigzag's runs: count, run length and jog width for a leg `len` long.
fn zig_layout(len: f64, w: f64, j: f64) -> (usize, f64) {
    let want = 8.0 + 6.0 * j;
    let n = (((len + w) / (want + w)).floor() as usize).max(1);
    let n = if n >= 3 { n } else { 1 };
    (n, (len - (n as f64 - 1.0) * w) / n as f64)
}

/// The tile offset of PH-15 (at most 0.3 m apart, so each step is within the step-up).
pub fn roof_dy(i: usize, j: f64) -> f64 {
    ((i * 5 + (j * 3.0) as usize) % 3) as f64 * 0.15
}

/// Sideways reach of the curved gallery (PH-20), at most a fifth of the leg.
fn curve_amp(len: f64, k: f64) -> f64 {
    (6.0 + 4.0 * k).min(0.2 * len)
}

/// Offset of the curved gallery at `s`: a parabola, 0 at both ends.
pub fn curve_t(s: f64, len: f64, k: f64) -> f64 {
    let u = s / len;
    4.0 * curve_amp(len, k) * u * (1.0 - u)
}

/// The layout of PH-19: `n` half-widths of diagonal each way, `a` sideways in all, and the
/// stations along the leg. `None` when the leg is too short for a skew.
#[derive(Debug, Clone, Copy)]
struct Skew {
    n: usize,
    a: f64,
    s_a: f64,
    s_b: f64,
    s_c: f64,
    s_d: f64,
}

fn skew_layout(len: f64, w: f64, k: f64) -> Option<Skew> {
    let h = w / 2.0;
    let mut n = (((4.0 + 8.0 * k) / h).round() as usize).max(1);
    while n > 0 && len < 2.0 * n as f64 * h + 6.0 * w {
        n -= 1;
    }
    if n == 0 {
        return None;
    }
    let a = n as f64 * h;
    let free = len - 2.0 * a;
    let s_a = free * 0.35;
    let s_b = s_a + a;
    let s_c = s_b + free * 0.3;
    Some(Skew {
        n,
        a,
        s_a,
        s_b,
        s_c,
        s_d: s_c + a,
    })
}

/// The centre line a walker follows on a leg of type `id`, as `(s, t, y)` samples at most a
/// metre apart. Every point is on the walking surface, so a test can probe above and below it.
pub fn walk_line(id: u8, len: f64, p: &Params) -> Vec<[f64; 3]> {
    let n = len.ceil() as usize;
    let at = |s: f64| match id {
        15 => [s, 0.0, roof_dy((s / 8.0) as usize, p.j)],
        20 => [s, curve_t(s, len, p.k), 0.0],
        4 => {
            let u = s / len;
            [s, 0.0, -sag(len, p.k) * 4.0 * u * (1.0 - u)]
        }
        _ => [s, 0.0, 0.0],
    };
    if id == 19
        && let Some(k) = skew_layout(len, p.width, p.k)
    {
        {
            let mut out = Vec::new();
            let line = |out: &mut Vec<[f64; 3]>, a: f64, z: f64, t: f64| {
                let mut x = a + 0.5;
                while x < z - 0.25 {
                    out.push([x, t, 0.0]);
                    x += 1.0;
                }
            };
            line(&mut out, 0.0, k.s_a, 0.0);
            let mut u = 0.0;
            while u < k.a - 0.25 {
                out.push([k.s_a + u, u, 0.0]);
                u += 0.5;
            }
            line(&mut out, k.s_b, k.s_c, k.a);
            let mut u = 0.0;
            while u < k.a - 0.25 {
                out.push([k.s_c + u, k.a - u, 0.0]);
                u += 0.5;
            }
            line(&mut out, k.s_d, len, 0.0);
            return out;
        }
    }
    if id == 5 {
        let amp = 2.0 + 2.0 * p.k;
        let (runs, run) = zig_layout(len, p.width, p.j);
        let mut out = Vec::new();
        let mut s = 0.0;
        for k in 0..runs {
            let t = zig_t(k, runs, amp);
            let mut x = s + 0.5;
            while x < s + run {
                out.push([x, t, 0.0]);
                x += 1.0;
            }
            out.push([s + run - 0.25, t, 0.0]);
            out.push([s + run + 0.25, t, 0.0]);
            if k + 1 < runs {
                let t1 = zig_t(k + 1, runs, amp);
                let steps = ((t1 - t).abs() / 0.5).ceil() as usize;
                for i in 1..=steps.max(1) {
                    let u = i as f64 / steps.max(1) as f64;
                    out.push([s + run + p.width / 2.0, t + (t1 - t) * u, 0.0]);
                }
                s += run + p.width;
            }
        }
        return out
            .into_iter()
            .map(|[s, t, y]| [s.clamp(0.01, len - 0.01), t, y])
            .collect();
    }
    (0..n)
        .map(|i| at((i as f64 + 0.5).min(len - 0.01)))
        .collect()
}

/// Builds one leg of type `id` into `e`.
#[allow(clippy::too_many_lines)] // one arm per path type
pub(crate) fn build_leg(e: &mut Emitter<'_>, leg: Leg, id: u8, p: &Params) -> Vec<Clear> {
    let len = leg.len;
    let w = p.width;
    let h = w / 2.0;
    let mut b = Build {
        e,
        leg,
        clear: Vec::new(),
    };
    match id {
        // PH-03: a deck hung from a pylon at mid span by stay cables.
        3 => {
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            let mast = 14.0 + 8.0 * p.k;
            let mid = len / 2.0;
            for side in [-1.0, 1.0] {
                let t = side * (h + 0.8);
                b.solid(
                    (mid - 0.6, mid + 0.6),
                    (t - 0.6, t + 0.6),
                    (-0.5, mast),
                    MaterialId::CONCRETE,
                );
                for k in 1..=3 {
                    let d = len / 2.0 * f64::from(k) / 3.5;
                    b.diag((mid, mast - 0.5), (mid - d, 0.0), side * (h + 0.2), 0.3);
                    b.diag((mid, mast - 0.5), (mid + d, 0.0), side * (h + 0.2), 0.3);
                }
            }
            b.solid(
                (mid - 0.6, mid + 0.6),
                (-h - 1.4, h + 1.4),
                (mast - 1.0, mast),
                MaterialId::CONCRETE,
            );
        }
        // PH-04: a deck that sags between its ends, with a handrail cable.
        4 => {
            let sg = sag(len, p.k);
            let y = |s: f64| {
                let u = s / len;
                -sg * 4.0 * u * (1.0 - u)
            };
            let mut s = 0.0;
            while s < len {
                let e = (s + 4.0).min(len);
                b.wedge((s, e), (-h, h), (y(s), y(e)));
                if p.rails > 0 {
                    let m = y(f64::midpoint(s, e));
                    for side in [-1.0, 1.0] {
                        b.rail(
                            (s, e),
                            (side * h - 0.05, side * h + 0.05),
                            (m + 1.0, m + 1.1),
                        );
                    }
                }
                s = e;
            }
        }
        // PH-05: a nine-turn bridge, runs alternating sideways joined by jogs.
        5 => {
            let amp = 2.0 + 2.0 * p.k;
            let (runs, run) = zig_layout(len, w, p.j);
            let mut s = 0.0;
            for k in 0..runs {
                let t = zig_t(k, runs, amp);
                b.deck(
                    (s, s + run),
                    (t - h, t + h),
                    (-0.3, 0.0),
                    MaterialId::GRATING,
                );
                if k + 1 < runs {
                    let t1 = zig_t(k + 1, runs, amp);
                    b.deck(
                        (s + run, s + run + w),
                        (t.min(t1) - h, t.max(t1) + h),
                        (-0.3, 0.0),
                        MaterialId::GRATING,
                    );
                    s += run + w;
                }
            }
        }
        // PH-06: a tread laid along the top of a beam, kerbed, with a lamp post now and then.
        6 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                b.rail(
                    (0.0, len),
                    (side * h - 0.075, side * h + 0.075),
                    (0.0, 0.25),
                );
            }
            let mut s = 6.0;
            while s < len {
                let t = p.side * (h + 0.1);
                b.rail((s - 0.1, s + 0.1), (t - 0.1, t + 0.1), (0.0, 1.4));
                b.bx(
                    (s - 0.25, s + 0.25),
                    (t - 0.25, t + 0.25),
                    (1.4, 1.9),
                    MaterialId::EMISSIVE,
                    FeatureKind::WINDOW,
                    PrimFlags::NO_COLLIDE,
                );
                s += 16.0;
            }
        }
        // PH-07: a pipe set into the roof with a flat tread along its top.
        7 => {
            let r = 0.8 + 0.6 * p.k;
            b.pipe((0.0, len), r);
            b.deck((0.0, len), (-0.45, 0.45), (-0.1, 0.0), MaterialId::GRATING);
            let mut s = 4.0;
            while s < len {
                b.solid(
                    (s - 0.2, s + 0.2),
                    (-r - 0.15, r + 0.15),
                    (-2.0 * r - 0.15, -0.1),
                    MaterialId::METAL,
                );
                s += 8.0;
            }
        }
        // PH-08: a ledge held out from the façade on ties.
        8 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            let wall = p.side * 15.5;
            let mut s = 2.0;
            while s < len {
                b.solid(
                    (s - 0.25, s + 0.25),
                    (wall.min(p.side * h), wall.max(p.side * h)),
                    (-0.9, -0.3),
                    MaterialId::METAL,
                );
                s += 8.0;
            }
            b.rail(
                (0.0, len),
                (-p.side * h - 0.05, -p.side * h + 0.05),
                (0.0, 1.1),
            );
        }
        // PH-09: a covered gallery along a façade: wall on one side, posts on the other.
        9 => {
            let ceil = 3.4;
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            b.solid(
                (0.0, len),
                (-h, h),
                (ceil, ceil + 0.5),
                MaterialId::CONCRETE,
            );
            let back = p.side * h;
            b.wall(
                (back.min(back - p.side * 0.4), back.max(back - p.side * 0.4)),
                ceil,
                len,
                p.windowed,
            );
            let open = -p.side * (h - 0.3);
            b.posts(len, 8.0, open, ceil, 0.6);
            if p.rails > 0 {
                b.rail((0.0, len), (open - 0.05, open + 0.05), (0.0, 1.1));
            }
        }
        // PH-10: an arcade of pillars and lintels under a roof, through a row of buildings.
        10 => {
            let ceil = 4.0 + 2.0 * p.k;
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            b.solid(
                (0.0, len),
                (-h, h),
                (ceil, ceil + 1.0),
                MaterialId::CONCRETE,
            );
            for side in [-1.0, 1.0] {
                let t = side * (h - 0.4);
                b.posts(len, 8.0, t, ceil, 0.8);
                b.solid(
                    (0.0, len),
                    (t - 0.4, t + 0.4),
                    (ceil - 0.8, ceil),
                    MaterialId::CONCRETE,
                );
            }
            let mut s = 0.0;
            while s <= len {
                b.solid(
                    (s - 0.25, s + 0.25),
                    (-h, h),
                    (ceil - 0.5, ceil),
                    MaterialId::CONCRETE,
                );
                s += 16.0;
            }
        }
        // PH-02: an enclosed bridge, blind or with a band of windows.
        2 => {
            let ceil = 3.2;
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                let t = side * h;
                b.wall(
                    (t.min(t - side * 0.4), t.max(t - side * 0.4)),
                    ceil,
                    len,
                    p.windowed,
                );
            }
            b.solid(
                (0.0, len),
                (-h, h),
                (ceil, ceil + 0.4),
                MaterialId::CONCRETE,
            );
        }
        // PH-11 and PH-12: a lined passage bored through solid, tall or a crawl.
        11 | 12 => {
            let (ch, wl) = if id == 11 { (3.2, 0.3) } else { (1.3, 0.2) };
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                b.side_solid(
                    (0.0, len),
                    (side * h, side * (h + wl)),
                    (0.0, ch),
                    MaterialId::CONCRETE,
                );
            }
            b.solid(
                (0.0, len),
                (-h - wl, h + wl),
                (ch, ch + wl),
                MaterialId::CONCRETE,
            );
            if id == 11 {
                let mut s = 3.0;
                while s < len {
                    b.solid(
                        (s - 0.2, s + 0.2),
                        (-h - wl - 0.3, h + wl + 0.3),
                        (ch + wl, ch + wl + 0.3),
                        MaterialId::METAL,
                    );
                    b.bx(
                        (s + 2.8, s + 3.2),
                        (-0.15, 0.15),
                        (ch - 0.15, ch),
                        MaterialId::EMISSIVE,
                        FeatureKind::WINDOW,
                        PrimFlags::NO_COLLIDE,
                    );
                    s += 6.0;
                }
            }
            b.cut(
                (0.0, len),
                (-h - wl - 0.3, h + wl + 0.3),
                (-0.5, ch + wl + 0.3),
            );
        }
        // PH-13: a broad paved avenue with lamp posts on both edges.
        13 => {
            b.deck((0.0, len), (-h, h), (-0.6, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                let t = side * (h - 0.5);
                b.rail((0.0, len), (side * h - 0.15 * side, side * h), (0.0, 0.15));
                let mut s = 6.0;
                while s < len {
                    b.rail((s - 0.15, s + 0.15), (t - 0.15, t + 0.15), (0.0, 8.0));
                    b.bx(
                        (s - 0.4, s + 0.4),
                        (t - 0.4, t + 0.4),
                        (8.0, 8.6),
                        MaterialId::EMISSIVE,
                        FeatureKind::WINDOW,
                        PrimFlags::NO_COLLIDE,
                    );
                    s += 24.0;
                }
            }
        }
        // PH-14: a canyon floor with walls both sides, doors on one, beams overhead.
        14 => {
            let wall = 14.0;
            b.deck((0.0, len), (-h, h), (-0.5, 0.0), MaterialId::CONCRETE);
            let mut s = 0.0;
            while s < len {
                let e = (s + 18.0).min(len);
                let (g0, g1) = ((s + 7.0).min(e), (s + 9.5).min(e));
                for (a, z) in [(s, g0), (g1, e)] {
                    b.side_solid(
                        (a, z),
                        (p.side * h, p.side * (h + 0.5)),
                        (0.0, wall),
                        MaterialId::CONCRETE,
                    );
                }
                s = e;
            }
            b.side_solid(
                (0.0, len),
                (-p.side * h, -p.side * (h + 0.5)),
                (0.0, wall),
                MaterialId::CONCRETE,
            );
            b.bx(
                (0.0, len),
                (-p.side * (h - 0.3), -p.side * h),
                (2.0, 2.3),
                MaterialId::PIPE,
                FeatureKind::PIPE,
                PrimFlags::NONE,
            );
            let mut s = 7.0;
            while s < len {
                b.solid(
                    (s - 0.2, s + 0.2),
                    (-h - 0.5, h + 0.5),
                    (10.0, 10.4),
                    MaterialId::METAL,
                );
                s += 14.0;
            }
        }
        // PH-15: tiles of mismatched rooftops, each a little higher or lower than the last.
        15 => {
            let mut i = 0;
            let mut s = 0.0;
            while s < len {
                let e = (s + 8.0).min(len);
                let dy = roof_dy(i, p.j);
                b.deck((s, e), (-h, h), (dy - 0.4, dy), MaterialId::CONCRETE);
                for side in [-1.0, 1.0] {
                    b.rail((s, e), (side * h, side * (h + 0.15)), (dy, dy + 0.4));
                }
                if i % 2 == 0 {
                    b.solid(
                        (s + 2.0, s + 3.5),
                        (p.side * (h + 0.8), p.side * (h + 2.3)),
                        (dy - 0.4, dy + 1.0),
                        MaterialId::METAL,
                    );
                }
                i += 1;
                s = e;
            }
        }
        // PH-16: a heavy pier on piles with bollards and a lamp at the head.
        16 => {
            b.deck((0.0, len), (-h, h), (-0.8, 0.0), MaterialId::CONCRETE);
            let mut s = 6.0;
            while s < len {
                for side in [-1.0, 1.0] {
                    let t = side * (h - 0.6);
                    b.solid(
                        (s - 0.6, s + 0.6),
                        (t - 0.6, t + 0.6),
                        (-24.0, -0.8),
                        MaterialId::CONCRETE,
                    );
                    b.rail((s - 0.25, s + 0.25), (t - 0.25, t + 0.25), (0.0, 0.5));
                }
                s += 24.0;
            }
            b.bx(
                (len - 0.6, len - 0.2),
                (-0.2, 0.2),
                (0.0, 3.0),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
        }
        // PH-17: a light deck hung from an overhead girder on chains.
        17 => {
            b.deck((0.0, len), (-h, h), (-0.2, 0.0), MaterialId::GRATING);
            b.solid((0.0, len), (-0.4, 0.4), (14.0, 15.0), MaterialId::METAL);
            let mut s = 4.0;
            while s < len {
                for side in [-1.0, 1.0] {
                    let t = side * (h + 0.1);
                    b.bx(
                        (s - 0.06, s + 0.06),
                        (t - 0.06, t + 0.06),
                        (0.0, 14.0),
                        MaterialId::PIPE,
                        FeatureKind::CABLE,
                        PrimFlags::NO_COLLIDE,
                    );
                }
                s += 24.0;
            }
            if p.rails > 0 {
                for side in [-1.0, 1.0] {
                    b.rail((0.0, len), (side * h - 0.05, side * h + 0.05), (0.0, 1.0));
                }
            }
        }
        // PH-18: a dead conveyor, its belt set into the roof with cleats and side frames.
        18 => {
            b.deck((0.0, len), (-h, h), (-0.9, 0.0), MaterialId::METAL);
            for side in [-1.0, 1.0] {
                b.rail((0.0, len), (side * h, side * (h + 0.15)), (-0.9, 0.3));
            }
            let mut s = 1.0;
            while s < len {
                b.bx(
                    (s, s + 0.1),
                    (-h, h),
                    (0.0, 0.04),
                    MaterialId::CONCRETE,
                    FeatureKind::WALKWAY,
                    PrimFlags::NONE,
                );
                s += 6.0;
            }
            let mut s = 3.0;
            while s < len {
                for side in [-1.0, 1.0] {
                    let t = side * (h + 0.1);
                    b.solid(
                        (s - 0.15, s + 0.15),
                        (t - 0.15, t + 0.15),
                        (-6.0, -0.9),
                        MaterialId::METAL,
                    );
                }
                s += 12.0;
            }
        }
        // PH-19: a corridor that crosses the grid at 45°, built of overlapping squares.
        19 => match skew_layout(len, w, p.k) {
            None => b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::GRATING),
            Some(k) => {
                let sq = |b: &mut Build<'_, '_>, s: f64, t: f64| {
                    b.deck(
                        ((s - h).max(0.0), (s + h).min(len)),
                        (t - h, t + h),
                        (-0.3, 0.0),
                        MaterialId::GRATING,
                    );
                };
                b.deck((0.0, k.s_a), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
                b.deck(
                    (k.s_b, k.s_c),
                    (k.a - h, k.a + h),
                    (-0.3, 0.0),
                    MaterialId::GRATING,
                );
                b.deck((k.s_d, len), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
                for i in 0..=k.n {
                    let d = i as f64 * h;
                    sq(&mut b, k.s_a + d, d);
                    sq(&mut b, k.s_c + d, k.a - d);
                }
            }
        },
        // PH-20: a gallery that bows sideways, inner colonnade, as inside a rotunda.
        20 => {
            let steps = (len / h).ceil() as usize;
            for i in 0..=steps {
                let s = (i as f64 * h).min(len);
                let t = curve_t(s, len, p.k);
                b.deck(
                    ((s - h).max(0.0), (s + h).min(len)),
                    (t - h, t + h),
                    (-0.3, 0.0),
                    MaterialId::GRATING,
                );
            }
            let mut s = 4.0;
            while s < len {
                let t = curve_t(s, len, p.k) - h - 0.4;
                b.solid(
                    (s - 0.3, s + 0.3),
                    (t - 0.3, t + 0.3),
                    (-0.3, 4.0),
                    MaterialId::CONCRETE,
                );
                s += 8.0;
            }
            b.cut(
                (0.0, len),
                (-h - 1.0, h + 1.0 + 4.0 * curve_amp(len, p.k)),
                (-0.5, 4.5),
            );
        }
        // P2-01: inside a great pipe: a floor, walls and a roof of pipe, flanges every 6 m.
        61 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
            for side in [-1.0, 1.0] {
                b.side_solid(
                    (0.0, len),
                    (side * h, side * (h + 0.4)),
                    (0.0, 2.6),
                    MaterialId::PIPE,
                );
            }
            b.side_solid(
                (0.0, len),
                (-h - 0.4, h + 0.4),
                (2.6, 3.0),
                MaterialId::PIPE,
            );
            let mut s = 3.0;
            while s < len {
                for side in [-1.0, 1.0] {
                    b.side_solid(
                        (s - 0.15, s + 0.15),
                        (side * (h + 0.4), side * (h + 0.7)),
                        (0.0, 3.3),
                        MaterialId::METAL,
                    );
                }
                b.side_solid(
                    (s - 0.15, s + 0.15),
                    (-h - 0.7, h + 0.7),
                    (3.0, 3.3),
                    MaterialId::METAL,
                );
                s += 6.0;
            }
        }
        // P2-02: a gutter: a channel of walls 1.1 m high with a lip, a floor with a drain strip.
        62 => {
            b.deck((0.0, len), (-h, h), (-0.4, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                b.side_solid(
                    (0.0, len),
                    (side * h, side * (h + 0.3)),
                    (-0.4, 1.1),
                    MaterialId::CONCRETE,
                );
                b.rail((0.0, len), (side * (h - 0.1), side * (h + 0.4)), (1.1, 1.2));
            }
            b.bx(
                (0.0, len),
                (-0.1, 0.1),
                (-0.02, 0.0),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
        }
        // P2-03: a track walk: planks between two rails on sleepers.
        63 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            for t in [-0.7, 0.7] {
                b.bx(
                    (0.0, len),
                    (t - 0.05, t + 0.05),
                    (0.0, 0.15),
                    MaterialId::METAL,
                    FeatureKind::TRAM,
                    PrimFlags::NO_COLLIDE,
                );
            }
            let mut s = 1.0;
            while s < len {
                b.bx(
                    (s, s + 0.2),
                    (-h - 0.3, h + 0.3),
                    (-0.45, -0.3),
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    PrimFlags::NO_COLLIDE,
                );
                s += 2.0;
            }
        }
        // P2-04: a tram car for a bridge: floor, glazed sides and a roof, end to end.
        64 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::METAL);
            for side in [-1.0, 1.0] {
                b.wall((side * h, side * (h + 0.15)), 2.8, len, true);
            }
            b.side_solid(
                (0.0, len),
                (-h - 0.15, h + 0.15),
                (2.8, 3.1),
                MaterialId::METAL,
            );
            let mut s = 12.0;
            while s < len {
                b.side_solid((s - 0.1, s + 0.1), (-h, h), (0.0, 2.8), MaterialId::METAL);
                s += 24.0;
            }
        }
        // P2-05: a chain walk: planks on two chains that hang as rails.
        65 => {
            let mut s = 0.0;
            while s < len {
                b.deck(
                    (s, (s + 0.9).min(len)),
                    (-h, h),
                    (-0.1, 0.0),
                    MaterialId::GRATING,
                );
                s += 1.0;
            }
            for side in [-1.0, 1.0] {
                b.rail((0.0, len), (side * h - 0.03, side * h + 0.03), (1.0, 1.06));
                let mut s = 2.0;
                while s < len {
                    b.rail(
                        (s - 0.03, s + 0.03),
                        (side * h - 0.03, side * h + 0.03),
                        (0.0, 1.0),
                    );
                    s += 4.0;
                }
            }
        }
        // P2-06: a colonnade walk: columns every 4 m on one side under an entablature.
        66 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            let t = p.side * (h + 0.5);
            let mut s = 2.0;
            while s < len {
                b.side_solid(
                    (s - 0.4, s + 0.4),
                    (t - 0.4, t + 0.4),
                    (-0.3, 4.0),
                    MaterialId::CONCRETE,
                );
                s += 4.0;
            }
            b.side_solid(
                (0.0, len),
                (t - 0.6, t + 0.6),
                (4.0, 4.7),
                MaterialId::CONCRETE,
            );
            b.rail(
                (0.0, len),
                (-p.side * h - 0.05, -p.side * h + 0.05),
                (0.0, 1.0),
            );
        }
        // P2-07: a cornice walk: a ledge from a wall, with no rail on the open side.
        67 => {
            b.deck((0.0, len), (-h, h), (-0.4, 0.0), MaterialId::CONCRETE);
            b.side_solid(
                (0.0, len),
                (p.side * h, p.side * (h + 1.0)),
                (-0.4, 3.0),
                MaterialId::CONCRETE,
            );
            b.side_solid(
                (0.0, len),
                (-p.side * h - p.side * 0.15, -p.side * h),
                (-0.4, 0.1),
                MaterialId::CONCRETE,
            );
        }
        // P2-08: a passage in a wall: lined with concrete, a lit strip along the roof.
        68 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            for side in [-1.0, 1.0] {
                b.side_solid(
                    (0.0, len),
                    (side * h, side * (h + 0.3)),
                    (0.0, 2.4),
                    MaterialId::CONCRETE,
                );
            }
            b.side_solid(
                (0.0, len),
                (-h - 0.3, h + 0.3),
                (2.4, 2.7),
                MaterialId::CONCRETE,
            );
            b.bx(
                (0.0, len),
                (-0.15, 0.15),
                (2.38, 2.4),
                MaterialId::EMISSIVE,
                FeatureKind::WINDOW,
                PrimFlags::NO_COLLIDE,
            );
            b.cut((0.0, len), (-h - 1.0, h + 1.0), (-0.5, 3.4));
        }
        // P2-09: a tunnel through a bundle of cables: sixteen cables round the walkway.
        69 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
            for k in 0..16 {
                let a = f64::from(k) / 16.0;
                let t = (a * 2.0 - 1.0) * (h + 0.6);
                let y = if k % 2 == 0 {
                    2.4 + 0.5 * a
                } else {
                    0.9 + 1.4 * a
                };
                let t = if k % 2 == 0 {
                    t
                } else {
                    (if k % 4 == 1 { 1.0 } else { -1.0 }) * (h + 0.5 + 0.3 * a)
                };
                b.bx(
                    (0.0, len),
                    (t - 0.12, t + 0.12),
                    (y - 0.12, y + 0.12),
                    MaterialId::PIPE,
                    FeatureKind::CABLE,
                    PrimFlags::NO_COLLIDE,
                );
            }
            let mut s = 4.0;
            while s < len {
                for side in [-1.0, 1.0] {
                    b.side_solid(
                        (s - 0.1, s + 0.1),
                        (side * (h + 0.3), side * (h + 0.5)),
                        (0.0, 2.6),
                        MaterialId::METAL,
                    );
                }
                b.side_solid(
                    (s - 0.1, s + 0.1),
                    (-h - 0.5, h + 0.5),
                    (2.6, 2.8),
                    MaterialId::METAL,
                );
                s += 8.0;
            }
            b.cut((0.0, len), (-h - 1.5, h + 1.5), (-0.5, 3.4));
        }
        // P2-10: a bridge floored with glass: panes in a frame, you see the drop.
        70 => {
            b.bx(
                (0.0, len),
                (-h, h),
                (-0.15, 0.0),
                MaterialId::LIT_WINDOW,
                FeatureKind::WALKWAY,
                PrimFlags::NONE,
            );
            for side in [-1.0, 1.0] {
                b.deck(
                    (0.0, len),
                    (side * h - 0.1, side * h + 0.1),
                    (-0.6, 0.0),
                    MaterialId::METAL,
                );
                b.rail(
                    (0.0, len),
                    (side * (h + 0.05) - 0.04, side * (h + 0.05) + 0.04),
                    (0.0, 1.1),
                );
            }
            let mut s = 2.0;
            while s < len {
                b.deck((s, s + 0.1), (-h, h), (-0.5, -0.15), MaterialId::METAL);
                s += 2.0;
            }
        }
        // P2-11: a covered market street: stalls and awnings on both sides, roof beams, lamps.
        71 => {
            b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::CONCRETE);
            let mut s = 1.0;
            while s + 2.0 < len {
                for side in [-1.0, 1.0] {
                    let t0 = side * (h - 0.8);
                    let t1 = side * h;
                    b.side_solid(
                        (s, s + 2.5),
                        (t0.min(t1), t0.max(t1)),
                        (0.0, 1.0),
                        MaterialId::CONCRETE,
                    );
                    let a0 = side * (h - 1.8);
                    b.side_solid(
                        (s, s + 2.5),
                        (a0.min(t1), a0.max(t1)),
                        (2.5, 2.7),
                        MaterialId::WARM,
                    );
                    b.side_solid(
                        (s + 1.1, s + 1.3),
                        (side * (h - 0.3) - 0.1, side * (h - 0.3) + 0.1),
                        (1.9, 2.5),
                        MaterialId::EMISSIVE,
                    );
                }
                s += 4.0;
            }
            let mut s = 4.0;
            while s < len {
                b.side_solid(
                    (s - 0.2, s + 0.2),
                    (-h - 0.3, h + 0.3),
                    (3.6, 4.0),
                    MaterialId::METAL,
                );
                s += 8.0;
            }
        }
        // P2-12: a ceiling walk: panes between ribs, a hall seen below.
        72 => {
            let mut s = 0.0;
            while s < len {
                b.bx(
                    (s, (s + 3.0).min(len)),
                    (-h, h),
                    (-0.15, 0.0),
                    MaterialId::LIT_WINDOW,
                    FeatureKind::WALKWAY,
                    PrimFlags::NONE,
                );
                b.deck(
                    ((s + 3.0).min(len), (s + 4.0).min(len)),
                    (-h, h),
                    (-0.4, 0.0),
                    MaterialId::CONCRETE,
                );
                s += 4.0;
            }
            for side in [-1.0, 1.0] {
                b.rail((0.0, len), (side * h - 0.05, side * h + 0.05), (0.0, 1.0));
            }
        }
        // PH-01: a grating deck with ribs and optional rails.
        _ => {
            if p.gaps {
                // NEAR-04: 0.2 m gaps across the deck every 6 m (narrower than a foot).
                let mut s = 0.0;
                while s < len {
                    let e = (s + 5.8).min(len);
                    b.deck((s, e), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
                    s = e + 0.2;
                }
            } else {
                b.deck((0.0, len), (-h, h), (-0.3, 0.0), MaterialId::GRATING);
            }
            let mut s = 2.0;
            while s < len {
                b.solid((s - 0.1, s + 0.1), (-h, h), (-0.7, -0.3), MaterialId::METAL);
                s += 8.0;
            }
            let sides: &[f64] = match p.rails {
                0 => &[],
                1 => &[1.0],
                _ => &[-1.0, 1.0],
            };
            for &side in sides {
                let side = side * p.side;
                b.rail((0.0, len), (side * h - 0.05, side * h + 0.05), (0.0, 1.1));
            }
        }
    }
    b.clear
}

/// Where a segment runs: in the open, bored through parent solid, or inside a structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Bored,
    /// Swallowed by a structure; `ring` when that structure has a ring wall (a rotunda).
    Engulfed {
        ring: bool,
    },
}

/// What lies around a segment, from the lattice (`levels.md` §5W.16).
#[derive(Debug, Clone, Copy)]
struct Ctx {
    kind: Kind,
    /// A rotunda (a structure with a ring wall) is next to the segment.
    ring_beside: bool,
    roof: bool,
    open_above: bool,
    /// A façade on the −t side and on the +t side.
    wall: [bool; 2],
    steps: usize,
}

fn solid_state(s: Option<SlotState>) -> bool {
    matches!(
        s,
        Some(SlotState::Solid | SlotState::Built | SlotState::Engulfed)
    )
}

fn state_at(t: &Tissue, c: [i64; 3]) -> Option<SlotState> {
    let n = t.lattice.n as i64;
    if c.iter().any(|&v| v < 0 || v >= n) {
        return None;
    }
    let i = t
        .lattice
        .index([c[0] as usize, c[1] as usize, c[2] as usize]);
    t.lattice.present[i].then(|| t.lattice.state[i])
}

fn context(t: &Tissue, slots: &[u32], surround: Surround, axis: usize, ring: &[bool]) -> Ctx {
    let cs: Vec<[i64; 3]> = slots
        .iter()
        .map(|&i| t.lattice.coords(i as usize).map(|v| v as i64))
        .collect();
    let count = |d: [i64; 3]| {
        cs.iter()
            .filter(|c| solid_state(state_at(t, [c[0] + d[0], c[1] + d[1], c[2] + d[2]])))
            .count()
    };
    let n = cs.len();
    let other = 2 - axis;
    let lateral = |sign: i64| {
        let mut d = [0; 3];
        d[other] = sign;
        count(d) * 2 >= n
    };
    let above = cs.iter().all(|c| {
        matches!(
            state_at(t, [c[0], c[1] + 1, c[2]]),
            None | Some(SlotState::Free | SlotState::Void)
        )
    });
    Ctx {
        kind: Kind::Open,
        ring_beside: cs.iter().any(|c| {
            Dir6::ALL.iter().any(|d| {
                let o = d.offset();
                let at = [c[0] + o.x, c[1] + o.y, c[2] + o.z];
                state_at(t, at).is_some()
                    && ring[t
                        .lattice
                        .index([at[0] as usize, at[1] as usize, at[2] as usize])]
            })
        }),
        roof: surround != Surround::Span && count([0, -1, 0]) * 2 >= n,
        open_above: above,
        wall: [lateral(-1), lateral(1)],
        steps: n.saturating_sub(1),
    }
}

/// The legal types for a context with their families (`levels.md` §5W.5 table). PH-01 is
/// always there.
fn legal(c: &Ctx) -> Vec<(u8, usize)> {
    let mut v = vec![(1, PATH_WALK)];
    match c.kind {
        Kind::Bored => {
            v.extend([(11, PATH_ENCLOSED), (12, PATH_ENCLOSED)]);
            v.extend([(68, PATH_ENCLOSED), (69, PATH_ENCLOSED)]);
            return v;
        }
        Kind::Engulfed { ring } => {
            v.extend([
                (11, PATH_ENCLOSED),
                (12, PATH_ENCLOSED),
                (68, PATH_ENCLOSED),
            ]);
            if ring {
                v.push((20, PATH_ENCLOSED));
            }
            return v;
        }
        Kind::Open => {}
    }
    // P2-03 a track walk, P2-02 a gutter and P2-01 a pipe interior run anywhere in the open.
    v.extend([(63, PATH_WALK), (62, PATH_WALK), (61, PATH_ENCLOSED)]);
    if c.roof {
        v.extend([
            (6, PATH_WALK),
            (7, PATH_WALK),
            (15, PATH_STREET),
            (18, PATH_WALK),
            (72, PATH_WALK),
        ]);
    } else {
        v.push((2, PATH_ENCLOSED));
        if c.steps >= 2 {
            v.push((16, PATH_WALK));
            if c.open_above {
                v.push((17, PATH_WALK));
            }
        }
        if c.steps >= 2 {
            v.push((5, PATH_WALK));
        }
        if c.steps >= 2 {
            v.push((4, PATH_WALK));
            v.push((65, PATH_WALK));
            if c.open_above {
                v.push((3, PATH_WALK));
                v.extend([(64, PATH_WALK), (70, PATH_WALK)]);
            }
        }
    }
    if c.ring_beside {
        v.push((20, PATH_ENCLOSED));
    }
    if c.wall[0] || c.wall[1] {
        v.extend([
            (8, PATH_WALK),
            (9, PATH_ENCLOSED),
            (13, PATH_STREET),
            (66, PATH_WALK),
            (67, PATH_WALK),
        ]);
        if c.steps >= 2 {
            v.push((19, PATH_STREET));
        }
        if c.wall[0] && c.wall[1] && c.steps >= 2 {
            v.extend([(10, PATH_ENCLOSED), (14, PATH_STREET), (71, PATH_STREET)]);
        }
    }
    v
}

/// Straight runs of equal height in a segment: `(from, to, axis, sign)` by slot index.
fn straight_legs(cs: &[[i64; 3]]) -> Vec<(usize, usize, usize, f64)> {
    let mut legs = Vec::new();
    let mut k = 0;
    while k + 1 < cs.len() {
        let d = [
            cs[k + 1][0] - cs[k][0],
            cs[k + 1][1] - cs[k][1],
            cs[k + 1][2] - cs[k][2],
        ];
        if d[1] != 0 {
            k += 1;
            continue;
        }
        let axis = if d[0] != 0 { 0 } else { 2 };
        let mut m = k + 1;
        while m + 1 < cs.len()
            && cs[m + 1][1] == cs[m][1]
            && cs[m + 1][axis] - cs[m][axis] == d[axis]
            && cs[m + 1][2 - axis] == cs[m][2 - axis]
        {
            m += 1;
        }
        legs.push((k, m, axis, d[axis] as f64));
        k = m;
    }
    legs
}

/// The modifiers drawn for a segment from its seven words.
fn params_for(id: u8, ctx: &Ctx, words: &[u64; 7]) -> Params {
    let (lo, hi) = width_range(id);
    let flip = words[4] & 1 == 1;
    let side = match (ctx.wall[0], ctx.wall[1]) {
        (true, true) if flip => -1.0,
        (_, true) => 1.0,
        (true, false) => -1.0,
        _ if flip => 1.0,
        _ => -1.0,
    };
    Params {
        width: lo + (hi - lo) * frac(words[1]),
        rails: (words[2] % 3) as u8,
        windowed: words[3] & 1 == 1,
        side,
        k: frac(words[5]),
        j: frac(words[6]),
        gaps: false,
    }
}

/// The leg between slots `from` and `to` of a segment, in lattice-frame metres: overlapped by
/// half a width at corners and reaching the cell's side at a port.
fn place_leg(
    cs: &[[i64; 3]],
    leg: (usize, usize, usize, f64),
    width: f64,
    t: &Tissue,
) -> Option<Leg> {
    let (from, to, axis, sign) = leg;
    let pitch = t.lattice.pitch as f64;
    let n = t.lattice.n as i64;
    let centre = |c: [i64; 3]| {
        [
            (c[0] as f64 + 0.5) * pitch,
            c[1] as f64 * pitch,
            (c[2] as f64 + 0.5) * pitch,
        ]
    };
    let (a, z) = (cs[from], cs[to]);
    let (mut start, mut stop) = (centre(a), centre(z));
    let reach = |c: i64, edge: i64| if c == edge { pitch / 2.0 } else { width / 2.0 };
    let (low, high) = (0, n - 1);
    let (first, last) = if sign > 0.0 { (low, high) } else { (high, low) };
    start[axis] -= sign * reach(a[axis], first);
    stop[axis] += sign * reach(z[axis], last);
    let len = (stop[axis] - start[axis]) * sign;
    // Side pieces stop short of a corner (1.7 m past the node covers a walkway 3 m wide), and
    // run on across a cell's side, where the leg goes on straight.
    let trim = |c: i64, edge: i64| if c == edge { 0.0 } else { width / 2.0 + 1.7 };
    (len > 0.0).then_some(Leg {
        start,
        axis,
        sign,
        len,
        trim: [trim(a[axis], first), trim(z[axis], last)],
    })
}

/// Slots of structures that have a ring wall (rotundas and silos).
fn ring_slots(t: &Tissue) -> Vec<bool> {
    let mut mask = vec![false; t.lattice.len()];
    for st in &t.structures {
        if st
            .shaped
            .iter()
            .any(|(sh, _)| matches!(sh, Shaped::Ring(_)))
        {
            for &i in &st.slots {
                mask[i as usize] = true;
            }
        }
    }
    mask
}

/// Where segment `si` runs A segment is swallowed when any of
/// its slots is inside a structure.
fn kind_of(t: &Tissue, si: usize, ring: &[bool]) -> Kind {
    let seg = &t.segments[si];
    if t.engulfed.contains(&(si as u16)) {
        let ring = seg.slots.iter().any(|&i| ring[i as usize]);
        return Kind::Engulfed { ring };
    }
    match seg.surround {
        Surround::Bored => Kind::Bored,
        Surround::Level | Surround::Span | Surround::Rise => Kind::Open,
    }
}

/// The column of a vertical segment: centre, lowest floor and height, in lattice-frame metres.
fn vertical_column(t: &Tissue, cs: &[[i64; 3]]) -> Option<Column> {
    let first = cs.first()?;
    let same = cs.iter().all(|c| c[0] == first[0] && c[2] == first[2]);
    let ys: Vec<i64> = cs.iter().map(|c| c[1]).collect();
    let (lo, hi) = (*ys.iter().min()?, *ys.iter().max()?);
    let monotone = ys.windows(2).all(|w| (w[1] - w[0]).abs() == 1)
        && (ys.windows(2).all(|w| w[1] > w[0]) || ys.windows(2).all(|w| w[1] < w[0]));
    if !same || hi == lo || !monotone {
        return None;
    }
    let pitch = t.lattice.pitch as f64;
    Some(Column {
        cx: (first[0] as f64 + 0.5) * pitch,
        cz: (first[2] as f64 + 0.5) * pitch,
        y0: lo as f64 * pitch,
        height: (hi - lo) as f64 * pitch,
    })
}

/// The legal vertical types (PV ids as `20 + n`) for a column.
fn vertical_legal(t: &Tissue, cs: &[[i64; 3]], kind: Kind, height: f64) -> Vec<u8> {
    let n = cs.len();
    let wall = [[1, 0], [-1, 0], [0, 1], [0, -1]].iter().any(|d| {
        cs.iter()
            .filter(|c| solid_state(state_at(t, [c[0] + d[0], c[1], c[2] + d[1]])))
            .count()
            * 2
            >= n
    });
    let helix = flat_yaw(t.lattice.frame).is_some() && height <= 128.0;
    let mut v = vec![21];
    if kind == Kind::Open {
        v.extend([22, 23, 24, 26, 30, 31, 32, 33, 34, 35, 38, 39]);
        if height >= 64.0 {
            v.extend([36, 37]);
        }
        if wall {
            v.extend([27, 29]);
        }
        if height >= 96.0 {
            v.push(28);
        }
    } else {
        v.extend([29, 24, 32]);
    }
    if helix {
        v.push(25);
    }
    if kind == Kind::Open {
        v.extend(paths_v::LADDER_IDS);
        v.extend(paths_v::P2V_IDS);
    }
    v
}

/// Builds the vertical segment `cs` as one of PV-01..10. Returns the chosen id.
fn place_rise(
    t: &Tissue,
    cs: &[[i64; 3]],
    col: Column,
    kind: Kind,
    forced: bool,
    words: &[u64; 7],
    e: &mut Emitter<'_>,
) -> (u8, Option<Clear>, Vec<[f64; 3]>, f64) {
    let weights = t.character.map_or([1, 0, 0, 1, 0, 0], |c| c.paths);
    let cands = if forced {
        vec![36]
    } else {
        vertical_legal(t, cs, kind, col.height)
    };
    let ws: Vec<u32> = cands
        .iter()
        .map(|&id| {
            let fam = if id >= 41 {
                PATH_LADDER
            } else if id >= 31 {
                PATH_LIFTSLOPE
            } else {
                PATH_STAIR_FAMILY
            };
            // Thirteen ladder families against the stairs: each ladder id counts a quarter.
            let w = u32::from(weights[fam]).max(u32::from(id == 21));
            if fam == PATH_LADDER { w } else { 4 * w }
        })
        .collect();
    let mut id = Rng::new(words[0])
        .pick_weighted(&ws)
        .map_or(21, |i| cands[i]);
    // At its foot a stair stands solid from the floor up on one side of its middle corridor
    // (the first lane, on plan -v), so a leg must come in along the corridor (plan u) or from
    // the other side. Segments are fixed by routing; `t.route` also grows later, which the
    // bored cut must not see.
    let foot = cs
        .iter()
        .min_by_key(|c| c[1])
        .map_or([0; 3], |c| [c[0], c[1], c[2]]);
    let leg_at = |d: [i64; 2]| {
        let c = [foot[0] + d[0], foot[1], foot[2] + d[1]];
        state_at(t, c).is_some() && {
            let i = t
                .lattice
                .index([c[0] as usize, c[1] as usize, c[2] as usize]) as u32;
            t.segments.iter().any(|s| s.slots.contains(&i))
        }
    };
    let (fx, fz) = (
        if words[3] & 1 == 1 { 1.0 } else { -1.0 },
        if words[4] & 1 == 1 { 1.0 } else { -1.0 },
    );
    // The drawn plan stands unless a leg comes in on its solid side and another plan is free.
    let drawn = (words[2] & 1 == 1, fx, fz);
    // The scissor stair (PV-03) is solid on both sides of the corridor.
    let solid_side = |id: u8, (swap, fx, fz): (bool, f64, f64)| {
        let across = |sign: i64| {
            if swap {
                [sign * fx as i64, 0]
            } else {
                [0, sign * fz as i64]
            }
        };
        leg_at(across(-1)) || (id == 23 && leg_at(across(1)))
    };
    let find_plan = |id: u8| {
        [
            drawn,
            (drawn.0, -fx, fz),
            (drawn.0, fx, -fz),
            (drawn.0, -fx, -fz),
            (!drawn.0, fx, fz),
            (!drawn.0, -fx, fz),
            (!drawn.0, fx, -fz),
            (!drawn.0, -fx, -fz),
        ]
        .into_iter()
        .find(|&plan| !solid_side(id, plan))
    };
    let mut plan = find_plan(id);
    if plan.is_none() && id == 23 {
        id = 21;
        plan = find_plan(id);
    }
    let best = plan.unwrap_or(drawn);
    let (lo, hi) = paths_v::width_range(id);
    let p = VParams {
        w: lo + (hi - lo) * frac(words[1]),
        swap: best.0,
        fx: best.1,
        fz: best.2,
        k: frac(words[5]),
        side: if words[6] & 1 == 1 { 1.0 } else { -1.0 },
    };
    let rise = build_rise(e, id, col, &p);
    (id, rise.bounds, rise.walk, p.w)
}

/// Chooses a type for every level, span, bored and rising segment and builds it.
pub fn paths_pass(seed: WorldSeed, t: &Tissue) -> PathPieces {
    paths_impl(seed, t, false)
}

/// The clear boxes of the bored segments of `t`, which depend on the routes alone (a bored
/// segment's type, width and legs do not read the structures), so the parent's solid can be cut
/// without growing the cell. Same boxes as `paths_pass(..).bores` for those segments.
pub fn bored_bores(seed: WorldSeed, t: &Tissue) -> Vec<Primitive> {
    paths_impl(seed, t, true).bores
}

/// `paths_pass`, or only the bored segments when `only_bored`.
#[allow(clippy::too_many_lines)] // one pass, the stages in order
fn paths_impl(seed: WorldSeed, t: &Tissue, only_bored: bool) -> PathPieces {
    let mut out = PathPieces::default();
    let mut rng = Rng::new(hash(seed.0, Level::L2, t.cell.coords, salt::PATHS));
    let mut mod_rng = Rng::new(hash(seed.0, Level::L2, t.cell.coords, salt::MODS));
    let mat_weights = t.character.map_or([0; 4], |c| c.materials);
    let weights = t.character.map_or([1, 0, 0, 1, 0, 0], |c| c.paths);
    let mut e = Emitter::new(&t.lattice);
    let mut bores = Emitter::new(&t.lattice);
    let mut corridors = Emitter::new(&t.lattice);
    let ring = ring_slots(t);
    for (si, seg) in t.segments.iter().enumerate() {
        // The draws are made for every segment so skipping one never moves the rest.
        let words: [u64; 7] = std::array::from_fn(|_| rng.next_u64());
        let mod_words: [u64; 9] = std::array::from_fn(|_| mod_rng.next_u64());
        let kind = kind_of(t, si, &ring);
        if seg.slots.len() < 2 || (only_bored && kind != Kind::Bored) {
            continue;
        }
        let cs: Vec<[i64; 3]> = seg
            .slots
            .iter()
            .map(|&i| t.lattice.coords(i as usize).map(|v| v as i64))
            .collect();
        if let Some(col) = vertical_column(t, &cs) {
            let first = e.out.len();
            let (id, cut, walk, width) = place_rise(
                t,
                &cs,
                col,
                kind,
                t.lift_segments.contains(&(si as u16)),
                &words,
                &mut e,
            );
            let mods = modifiers::draw(&mod_words, id, mat_weights);
            modifiers::retint(&mut e, &mods, first);
            out.mods.push((si as u16, mods));
            if let Some(c) = cut {
                out.rise_clear.push(c);
            }
            if let Some((a, b)) = cut {
                let before = corridors.out.len();
                corridors.frame_box(
                    a,
                    b,
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    PrimFlags::NO_COLLIDE,
                );
                if let Some(&volume) = corridors.out.get(before) {
                    out.rises.push((volume, first..e.out.len()));
                }
            }
            if let Some((a, b)) = cut.filter(|_| kind != Kind::Open) {
                bores.frame_box(
                    [a[0] - 0.5, a[1] - 0.5, a[2] - 0.5],
                    [b[0] + 0.5, b[1] + 0.5, b[2] + 0.5],
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    PrimFlags::NO_COLLIDE,
                );
            }
            out.owners.push((first..e.out.len(), out.walks.len()));
            out.walks.push(Walk {
                id,
                rise: true,
                width,
                points: walk.iter().map(|&f| e.at(f)).collect(),
                up: t.lattice.frame.axes()[1],
            });
            out.chosen.push((si as u16, id));
            continue;
        }
        let legs = straight_legs(&cs);
        let Some(&(first, _, axis0, _)) = legs.first() else {
            continue;
        };
        let mut ctx = context(t, &seg.slots[first..], seg.surround, axis0, &ring);
        ctx.kind = kind;
        if kind == Kind::Bored {
            // The cut must not read states that growth sets (`bored_bores` runs without it).
            ctx.wall = [false; 2];
        }
        let cands = legal(&ctx);
        let ws: Vec<u32> = cands
            .iter()
            .map(|&(id, fam)| u32::from(weights[fam]).max(u32::from(id == 1)))
            .collect();
        let mut pick = Rng::new(words[0]);
        let id = pick.pick_weighted(&ws).map_or(1, |i| cands[i].0);
        let mut p = params_for(id, &ctx, &words);
        let mods = modifiers::draw(&mod_words, id, mat_weights);
        p.gaps = mods.near == 4;
        out.mods.push((si as u16, mods));
        out.chosen.push((si as u16, id));
        for l in legs {
            let Some(leg) = place_leg(&cs, l, p.width, t) else {
                continue;
            };
            let first = e.out.len();
            out.walks.push(Walk {
                id,
                rise: false,
                width: p.width,
                points: vec![
                    e.at(leg.map(0.0, 0.0, 0.0)),
                    e.at(leg.map(leg.len, 0.0, 0.0)),
                ],
                up: t.lattice.frame.axes()[1],
            });
            let mut clear = build_leg(&mut e, leg, id, &p);
            modifiers::apply(&mut e, leg, &p, &mods, first);
            out.owners.push((first..e.out.len(), out.walks.len() - 1));
            let h = p.width / 2.0 + 1.0;
            let (ca, cb) = (leg.map(0.0, -h, -0.5), leg.map(leg.len, h, 4.0));
            corridors.frame_box(
                [ca[0].min(cb[0]), ca[1].min(cb[1]), ca[2].min(cb[2])],
                [ca[0].max(cb[0]), ca[1].max(cb[1]), ca[2].max(cb[2])],
                MaterialId::CONCRETE,
                FeatureKind::MISC,
                PrimFlags::NO_COLLIDE,
            );
            if clear.is_empty() && kind != Kind::Open {
                // Any path through solid or a structure still needs its passage cut.
                let h = p.width / 2.0 + 1.0;
                let (a, b) = (leg.map(0.0, -h, -0.5), leg.map(leg.len, h, 3.5));
                clear.push((
                    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
                    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
                ));
            }
            for (lo, hi) in clear {
                bores.frame_box(
                    lo,
                    hi,
                    MaterialId::CONCRETE,
                    FeatureKind::MISC,
                    PrimFlags::NO_COLLIDE,
                );
            }
        }
    }
    out.prims = e.out;
    out.bores = bores.out;
    out.corridors = corridors.out;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orient::Frame;
    use crate::tissue::Lattice;

    fn params(width: f64, k: f64, j: f64, side: f64, rails: u8, windowed: bool) -> Params {
        Params {
            width,
            rails,
            windowed,
            side,
            k,
            j,
            gaps: false,
        }
    }

    /// Cell-local metres of a point in a leg's frame, for an identity lattice.
    fn probe(leg: &Leg, e: &Emitter<'_>, s: f64, t: f64, y: f64) -> glam::Vec3 {
        let p = e.at(leg.map(s, t, y));
        glam::Vec3::new(p.x as f32, p.y as f32, p.z as f32)
    }

    fn solid_at(prims: &[Primitive], p: glam::Vec3) -> bool {
        prims
            .iter()
            .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
    }

    /// Every type has a floor under the whole walking line (even the sagging and zigzagging
    /// ones) and nothing solid in the first 1.9 m above it.
    #[test]
    fn every_type_is_walkable() {
        let lat = Lattice::new(Frame::Id);
        for id in IDS {
            for (axis, sign) in [(0, 1.0), (2, -1.0)] {
                for variant in 0..6u32 {
                    let (lo, hi) = width_range(id);
                    let f = f64::from(variant) / 5.0;
                    let p = params(
                        lo + (hi - lo) * f,
                        f,
                        1.0 - f,
                        if variant % 2 == 0 { 1.0 } else { -1.0 },
                        (variant % 3) as u8,
                        variant % 2 == 1,
                    );
                    let len = 100.0 + 20.0 * f;
                    let mut start = [200.0, 100.0, 200.0];
                    start[axis] = if sign > 0.0 { 100.0 } else { 400.0 };
                    let leg = Leg {
                        start,
                        axis,
                        sign,
                        len,
                        trim: [0.0; 2],
                    };
                    let mut e = Emitter::new(&lat);
                    build_leg(&mut e, leg, id, &p);
                    assert!(!e.out.is_empty(), "PH-{id:02} emitted nothing");
                    for [s, t, y] in walk_line(id, len, &p) {
                        let below = probe(&leg, &e, s, t, y - 0.1);
                        assert!(
                            solid_at(&e.out, below),
                            "PH-{id:02} v{variant} axis {axis}: no floor at s {s:.1} t {t:.1}"
                        );
                        // The crawl duct (PH-12) is 1.3 m clear; everything else 1.9 m.
                        let heads: &[f64] = if id == 12 {
                            &[0.25, 0.9, 1.15]
                        } else {
                            &[0.25, 0.9, 1.5, 1.85]
                        };
                        for &dy in heads {
                            let above = probe(&leg, &e, s, t, y + dy);
                            assert!(
                                !solid_at(&e.out, above),
                                "PH-{id:02} v{variant} axis {axis}: blocked {dy} m above s {s:.1}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Every piece stays within the lattice's cell when the leg does.
    #[test]
    fn pieces_stay_in_the_cell() {
        let lat = Lattice::new(Frame::Id);
        let q = 512 * crate::prim::Q_PER_M;
        for id in IDS {
            let p = params(width_range(id).1, 1.0, 1.0, 1.0, 2, true);
            let leg = Leg {
                start: [20.0, 100.0, 256.0],
                axis: 0,
                sign: 1.0,
                len: 470.0,
                trim: [0.0; 2],
            };
            let mut e = Emitter::new(&lat);
            build_leg(&mut e, leg, id, &p);
            for prim in &e.out {
                let b = prim.aabb();
                assert!(
                    b.min.x >= -1 && b.min.y >= 0 && b.min.z >= 0,
                    "PH-{id:02} {b:?}"
                );
                assert!(
                    b.max.x <= q + 1 && b.max.y <= q && b.max.z <= q,
                    "PH-{id:02} {b:?}"
                );
            }
        }
    }

    /// The width range of PH-01 is 1.2–3 m and the universal type is always legal.
    #[test]
    fn catwalk_is_always_legal() {
        assert_eq!(width_range(1), (1.2, 3.0));
        let none = Ctx {
            kind: Kind::Open,
            ring_beside: false,
            roof: true,
            open_above: false,
            wall: [false; 2],
            steps: 1,
        };
        assert!(legal(&none).iter().any(|&(id, _)| id == 1));
    }

    /// PH-01..20 all occur in real tissue, and no segment that has a type is left bare of
    /// pieces.
    #[test]
    fn every_path_type_occurs() {
        use crate::cell::CellKey;
        use crate::tissue::{TissueCache, tissue_cell};
        use glam::I64Vec3;
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let mut seen = [0usize; 100];
        let mut segments = 0usize;
        let mut bored = 0usize;
        let mut mods = [[0usize; 6]; 9];
        for k in 0..2000i64 {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(seed, &cache, key);
            segments += t.segments.len();
            for &(_, id) in &t.paths.chosen {
                seen[id as usize] += 1;
            }
            assert_eq!(t.paths.mods.len(), t.paths.chosen.len());
            for (_, m) in &t.paths.mods {
                for (row, v) in [
                    m.enclosure,
                    m.condition,
                    m.light,
                    m.ceiling,
                    m.rhythm,
                    m.material,
                    m.dressing,
                    m.angle,
                    m.near,
                ]
                .into_iter()
                .enumerate()
                {
                    mods[row][usize::from(v)] += 1;
                }
            }
            // A route through solid or a structure always has its passage cut.
            if t.paths.chosen.iter().any(|&(si, _)| {
                t.segments[si as usize].surround == Surround::Bored || t.engulfed.contains(&si)
            }) {
                assert!(!t.paths.bores.is_empty(), "{key:?}: bored, no passage");
                bored += 1;
            }
            if !t.paths.chosen.is_empty() {
                assert!(!t.paths.prims.is_empty(), "{key:?}");
            }
        }
        eprintln!(
            "segments {segments}, bored cells {bored}, per type {:?}",
            &seen[1..]
        );
        assert!(bored > 0);
        eprintln!("modifiers {mods:?}");
        for (row, n) in [6, 2, 2, 4, 4, 3, 4, 3, 6].into_iter().enumerate() {
            assert!(
                mods[row][..n].iter().all(|&c| c > 0),
                "modifier {row}: {:?}",
                mods[row]
            );
        }
        for id in IDS
            .into_iter()
            .chain(crate::paths_v::VIDS)
            .chain(crate::paths_v::P2V_IDS)
        {
            assert!(seen[id as usize] > 0, "PH-{id:02} never occurs: {seen:?}");
        }
    }
}
