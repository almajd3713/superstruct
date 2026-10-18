//! Oriented primitives: integer frames from constant tables, clipping to the owning cell.
//! Spec: `docs/spec/primitives.md` §9.
//!
//! No angle is ever computed. A frame is a key into the tables below, and its axes are built
//! with `+ − × ÷` and `sqrt` in `f64` only (`hashing.md` §6.1), so equal keys give equal bits.

// Narrowing casts here are bounded by construction (table entries, rotated cell-local
// coordinates below 2²⁰ quanta, step counts); the exact float compares test for exact axes.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use glam::{DVec3, Vec3};

use crate::cell::Axis;
use crate::prim::{
    Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape,
};

/// First-quadrant headings `(a, b)`: `round(64 · tan(5.625° · r))`, pinned. Spec §9.1.1.
pub const HEADING_QUADRANT: [(i32, i32); 16] = [
    (64, 0),
    (64, 6),
    (64, 13),
    (64, 19),
    (64, 27),
    (64, 34),
    (64, 43),
    (64, 53),
    (64, 64),
    (53, 64),
    (43, 64),
    (34, 64),
    (27, 64),
    (19, 64),
    (13, 64),
    (6, 64),
];

/// `round(64 · tan(5° · tilt))`, pinned. Spec §9.1.2.
pub const TILT_SLOPE: [i32; 7] = [0, 6, 11, 17, 23, 30, 37];

/// Number of tilt indices.
pub const TILTS: u8 = TILT_SLOPE.len() as u8;

/// Number of entries of the direction table.
pub const DIR_COUNT: usize = 49;

/// Every vector with components in `-2..=2`, `gcd = 1`, first nonzero component positive, in
/// lexicographic order. Spec §9.1.3.
pub const DIRS: [[i8; 3]; DIR_COUNT] = build_dirs();

const fn gcd(a: i32, b: i32) -> i32 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

const fn build_dirs() -> [[i8; 3]; DIR_COUNT] {
    let mut out = [[0i8; 3]; DIR_COUNT];
    let mut n = 0;
    let mut x = -2;
    while x <= 2 {
        let mut y = -2;
        while y <= 2 {
            let mut z = -2;
            while z <= 2 {
                let g = gcd(gcd(i32::abs(x), i32::abs(y)), i32::abs(z));
                let first = if x != 0 {
                    x
                } else if y != 0 {
                    y
                } else {
                    z
                };
                if g == 1 && first > 0 {
                    out[n] = [x as i8, y as i8, z as i8];
                    n += 1;
                }
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
    assert!(n == DIR_COUNT);
    out
}

/// The weave class of a direction (`regimes.md` §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirClass {
    Axis,
    FaceDiagonal,
    SpaceDiagonal,
    Skew,
}

/// Class of table entry `i`.
pub fn dir_class(i: usize) -> DirClass {
    let d = DIRS[i];
    let nz = d.iter().filter(|&&c| c != 0).count();
    let unit = d.iter().all(|&c| c.abs() <= 1);
    match (nz, unit) {
        (1, true) => DirClass::Axis,
        (2, true) => DirClass::FaceDiagonal,
        (3, true) => DirClass::SpaceDiagonal,
        _ => DirClass::Skew,
    }
}

/// Heading `k` (mod 64) as an integer vector `(a, b)`: the unit vector is `(a, 0, b) / sqrt(a² + b²)`.
pub fn heading(k: u8) -> (i32, i32) {
    let k = k % 64;
    let (mut a, mut b) = HEADING_QUADRANT[usize::from(k % 16)];
    for _ in 0..k / 16 {
        (a, b) = (-b, a);
    }
    (a, b)
}

/// A rotation, as a key into the tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Frame {
    /// Axis-aligned: the identity.
    Id,
    /// A heading (0..64) then a tilt (0..7). Never `(0, 0)`: build it with [`Frame::table`].
    Table { yaw: u8, tilt: u8 },
    /// Local X along an integer direction, any components in `-127..=127`. The table of §9.1.3
    /// lists the directions the generator picks for beams; ties and tails may use others.
    Dir { d: [i8; 3] },
}

impl Frame {
    /// Normalises `(0, 0)` to [`Frame::Id`].
    pub fn table(yaw: u8, tilt: u8) -> Self {
        debug_assert!(tilt < TILTS, "tilt {tilt} out of range");
        let yaw = yaw % 64;
        if yaw == 0 && tilt == 0 {
            Self::Id
        } else {
            Self::Table { yaw, tilt }
        }
    }

    /// A frame with local X along `d`. Axis directions normalise to a frame whose axes are exact.
    pub fn dir(d: [i8; 3]) -> Self {
        debug_assert!(d != [0, 0, 0], "zero direction");
        if d == [1, 0, 0] {
            Self::Id
        } else {
            Self::Dir { d }
        }
    }

    /// The columns `[ex, ey, ez]` of the rotation. Orthonormal to within rounding; exact for
    /// `Id` and for quarter turns with no tilt.
    pub fn axes(&self) -> [DVec3; 3] {
        match *self {
            Self::Id => [DVec3::X, DVec3::Y, DVec3::Z],
            Self::Table { yaw, tilt } => {
                let (a, b) = heading(yaw);
                let (a, b) = (f64::from(a), f64::from(b));
                let len = (a * a + b * b).sqrt();
                let x = DVec3::new(a / len, 0.0, b / len);
                let z = DVec3::new(-b / len, 0.0, a / len);
                let s = f64::from(TILT_SLOPE[usize::from(tilt)]);
                let m = (64.0 * 64.0 + s * s).sqrt();
                let (c, s) = (64.0 / m, s / m);
                [x * c + DVec3::Y * s, x * -s + DVec3::Y * c, z]
            }
            Self::Dir { d } => {
                let v = DVec3::new(f64::from(d[0]), f64::from(d[1]), f64::from(d[2]));
                let ex = v / v.length_squared().sqrt();
                let w = if d[0] != 0 || d[2] != 0 {
                    DVec3::Y
                } else {
                    DVec3::X
                };
                let t = w - ex * w.dot(ex);
                let ey = t / t.length_squared().sqrt();
                [ex, ey, ex.cross(ey)]
            }
        }
    }

    /// Whether every axis is exactly a signed world axis, so quarter-turn pieces stay exact.
    pub fn is_axis_aligned(&self) -> bool {
        self.axes().iter().all(|a| {
            [a.x, a.y, a.z]
                .iter()
                .all(|&c| c == 0.0 || c == 1.0 || c == -1.0)
        })
    }
}

/// A shape in frame-local quanta. The solid shapes of `primitives.md` §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LocalShape {
    Box {
        aabb: QAabb,
    },
    Ramp {
        aabb: QAabb,
        rise: Dir4,
    },
    Cylinder {
        axis: Axis,
        base: QVec3,
        length: i32,
        radius: i32,
    },
    StairRun {
        aabb: QAabb,
        rise: Dir4,
        steps: u16,
    },
}

impl LocalShape {
    /// The same shape as an axis-aligned [`Shape`] in frame-local space.
    pub fn to_shape(self) -> Shape {
        match self {
            Self::Box { aabb } => Shape::Box { aabb },
            Self::Ramp { aabb, rise } => Shape::Ramp { aabb, rise },
            Self::Cylinder {
                axis,
                base,
                length,
                radius,
            } => Shape::cylinder(axis, base, length, radius),
            Self::StairRun { aabb, rise, steps } => Shape::stair_run(aabb, rise, steps),
        }
    }

    pub fn aabb(self) -> QAabb {
        self.to_shape().aabb()
    }
}

/// A shape in a frame, clipped to a box (`primitives.md` §9.2, §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Oriented {
    pub frame: Frame,
    /// Frame origin in cell-local quanta.
    pub origin: QVec3,
    pub local: LocalShape,
    /// The owning cell's box (or a tighter one), cell-local quanta.
    pub clip: QAabb,
}

impl Oriented {
    /// The same solid with its origin and clip moved by `f` (a translation of cell-local space).
    #[must_use]
    pub fn translated(&self, f: impl Fn(QVec3) -> QVec3) -> Self {
        Self {
            origin: f(self.origin),
            clip: QAabb {
                min: f(self.clip.min),
                max: f(self.clip.max),
            },
            ..*self
        }
    }

    /// Cell-local point (metres) to frame-local metres.
    pub fn to_local(&self, p: Vec3) -> Vec3 {
        let [ex, ey, ez] = self.frame.axes();
        let o = self.origin.to_metres();
        let v = DVec3::new(
            f64::from(p.x) - f64::from(o.x),
            f64::from(p.y) - f64::from(o.y),
            f64::from(p.z) - f64::from(o.z),
        );
        Vec3::new(ex.dot(v) as f32, ey.dot(v) as f32, ez.dot(v) as f32)
    }

    /// Conservative integer bounds, outward-rounded and cut to the clip (`primitives.md` §9.5).
    /// If the solid misses the clip entirely, the unclipped bounds are returned.
    pub fn aabb(&self) -> QAabb {
        let [ex, ey, ez] = self.frame.axes();
        let l = self.local.aabb();
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for i in 0..8 {
            let c = DVec3::new(
                f64::from(if i & 1 == 0 { l.min.x } else { l.max.x }),
                f64::from(if i & 2 == 0 { l.min.y } else { l.max.y }),
                f64::from(if i & 4 == 0 { l.min.z } else { l.max.z }),
            );
            let w = ex * c.x + ey * c.y + ez * c.z;
            let w = [
                w.x + f64::from(self.origin.x),
                w.y + f64::from(self.origin.y),
                w.z + f64::from(self.origin.z),
            ];
            for k in 0..3 {
                lo[k] = lo[k].min(w[k]);
                hi[k] = hi[k].max(w[k]);
            }
        }
        let min = QVec3::new(
            lo[0].floor() as i32,
            lo[1].floor() as i32,
            lo[2].floor() as i32,
        );
        let max = QVec3::new(
            hi[0].ceil() as i32,
            hi[1].ceil() as i32,
            hi[2].ceil() as i32,
        );
        let cut = QAabb {
            min: QVec3::new(
                min.x.max(self.clip.min.x),
                min.y.max(self.clip.min.y),
                min.z.max(self.clip.min.z),
            ),
            max: QVec3::new(
                max.x.min(self.clip.max.x),
                max.y.min(self.clip.max.y),
                max.z.min(self.clip.max.z),
            ),
        };
        if cut.min.x < cut.max.x && cut.min.y < cut.max.y && cut.min.z < cut.max.z {
            cut
        } else {
            QAabb { min, max }
        }
    }

    /// Point-in-solid: inside the shape in its frame and inside the clip.
    pub fn contains(&self, p: Vec3) -> bool {
        let clip_min = self.clip.min.to_metres();
        let clip_max = self.clip.max.to_metres();
        p.cmpge(clip_min).all()
            && p.cmplt(clip_max).all()
            && self.local.to_shape().contains(self.to_local(p))
    }

    /// Lower bound on the signed distance: `max(shape, clip)` (`primitives.md` §9.4). A rotation
    /// is an isometry, so the shape's own distance carries over from its frame.
    pub fn sdf(&self, p: Vec3) -> f32 {
        let clip = Shape::Box { aabb: self.clip }.sdf(p);
        self.local.to_shape().sdf(self.to_local(p)).max(clip)
    }
}

impl Shape {
    /// An oriented shape. An identity frame whose solid already lies inside the clip stays a
    /// plain axis-aligned variant, so axis-aligned output is bit-identical (`primitives.md` §9.6).
    pub fn oriented(frame: Frame, origin: QVec3, local: LocalShape, clip: QAabb) -> Self {
        if frame == Frame::Id {
            let moved = shift(local, origin);
            let b = moved.aabb();
            if b.min.x >= clip.min.x
                && b.min.y >= clip.min.y
                && b.min.z >= clip.min.z
                && b.max.x <= clip.max.x
                && b.max.y <= clip.max.y
                && b.max.z <= clip.max.z
            {
                return moved.to_shape();
            }
        }
        Self::Oriented(Oriented {
            frame,
            origin,
            local,
            clip,
        })
    }
}

fn shift_aabb(a: QAabb, o: QVec3) -> QAabb {
    QAabb {
        min: QVec3::new(a.min.x + o.x, a.min.y + o.y, a.min.z + o.z),
        max: QVec3::new(a.max.x + o.x, a.max.y + o.y, a.max.z + o.z),
    }
}

/// `local` translated by `o`.
fn shift(local: LocalShape, o: QVec3) -> LocalShape {
    match local {
        LocalShape::Box { aabb } => LocalShape::Box {
            aabb: shift_aabb(aabb, o),
        },
        LocalShape::Ramp { aabb, rise } => LocalShape::Ramp {
            aabb: shift_aabb(aabb, o),
            rise,
        },
        LocalShape::Cylinder {
            axis,
            base,
            length,
            radius,
        } => LocalShape::Cylinder {
            axis,
            base: QVec3::new(base.x + o.x, base.y + o.y, base.z + o.z),
            length,
            radius,
        },
        LocalShape::StairRun { aabb, rise, steps } => LocalShape::StairRun {
            aabb: shift_aabb(aabb, o),
            rise,
            steps,
        },
    }
}

/// `tan(π/n) · 128`, rounded up, for `n` facets per turn (8, 16, 32, 64). Pinned. Spec §9.3.
const fn chord_k(n: u8) -> i32 {
    match n {
        8 => 54,
        16 => 26,
        32 => 13,
        _ => 7,
    }
}

/// Parameters of a faceted helical stair (`primitives.md` §9.3).
#[derive(Debug, Clone, Copy)]
pub struct HelixStair {
    /// Axis point at the base, cell-local quanta.
    pub axis: QVec3,
    /// Inner radius and width of the tread, quanta.
    pub inner: i32,
    pub width: i32,
    /// Steps per turn: 8, 16, 32 or 64.
    pub per_turn: u8,
    /// Total steps.
    pub steps: u32,
    /// Rise per step, quanta (at most 7 to stay walkable).
    pub rise: i32,
    /// Tread slab thickness, quanta.
    pub thick: i32,
    /// First heading, and the turning sense (+1 or -1).
    pub start: u8,
    pub sense: i8,
    pub material: MaterialId,
    pub kind: FeatureKind,
    pub flags: PrimFlags,
}

/// A faceted helical stair: one tread slab per step, each a box in its own heading (spec §9.3).
/// Adjacent treads overlap by one quantum, so there are no cracks.
pub fn helix_stair(h: &HelixStair, clip: QAabb) -> Vec<Primitive> {
    debug_assert!(matches!(h.per_turn, 8 | 16 | 32 | 64), "bad facet count");
    debug_assert!(h.rise > 0 && h.rise <= 7, "rise {} not walkable", h.rise);
    let step_h = i32::from(64 / h.per_turn);
    let outer = h.inner + h.width;
    let half = (outer * chord_k(h.per_turn) + 127) / 128 + 1;
    (0..h.steps)
        .map(|j| {
            let k = i32::from(h.start) + i32::from(h.sense) * (j as i32) * step_h;
            let yaw = k.rem_euclid(64) as u8;
            let top = (j as i32 + 1) * h.rise;
            let aabb = QAabb {
                min: QVec3::new(h.inner, top - h.thick, -half),
                max: QVec3::new(outer, top, half),
            };
            Primitive {
                shape: Shape::oriented(
                    Frame::table(yaw, 0),
                    h.axis,
                    LocalShape::Box { aabb },
                    clip,
                ),
                material: h.material,
                kind: h.kind,
                flags: h.flags,
            }
        })
        .collect()
}

/// Parameters of a ring of `n` wall facets (polygonal rings, rotundas, domes; spec §9.3).
#[derive(Debug, Clone, Copy)]
pub struct FacetRing {
    pub axis: QVec3,
    /// Inner radius of the wall, wall thickness and height, quanta.
    pub radius: i32,
    pub thick: i32,
    pub height: i32,
    /// Facets: 8, 16, 32 or 64.
    pub n: u8,
    pub start: u8,
    pub material: MaterialId,
    pub kind: FeatureKind,
    pub flags: PrimFlags,
}

/// `n` wall boxes round the axis, each facing inwards.
pub fn facet_ring(r: &FacetRing, clip: QAabb) -> Vec<Primitive> {
    debug_assert!(matches!(r.n, 8 | 16 | 32 | 64), "bad facet count");
    let step = 64 / r.n;
    let outer = r.radius + r.thick;
    let half = (outer * chord_k(r.n) + 127) / 128 + 1;
    (0..r.n)
        .map(|j| {
            let yaw = (r.start + j * step) % 64;
            let aabb = QAabb {
                min: QVec3::new(r.radius, 0, -half),
                max: QVec3::new(outer, r.height, half),
            };
            Primitive {
                shape: Shape::oriented(
                    Frame::table(yaw, 0),
                    r.axis,
                    LocalShape::Box { aabb },
                    clip,
                ),
                material: r.material,
                kind: r.kind,
                flags: r.flags,
            }
        })
        .collect()
}

/// A cylinder whose axis is the table direction `d`, starting at `start` and `length` quanta
/// long. Along a world axis it is a plain axis-aligned cylinder (when it fits the clip).
pub fn beam_cylinder(d: [i8; 3], start: QVec3, length: i32, radius: i32, clip: QAabb) -> Shape {
    let plain = match d {
        [1, 0, 0] => Some(Axis::X),
        [0, 1, 0] => Some(Axis::Y),
        [0, 0, 1] => Some(Axis::Z),
        _ => None,
    };
    if let Some(axis) = plain {
        let s = Shape::cylinder(axis, start, length, radius);
        let b = s.aabb();
        if b.min.x >= clip.min.x
            && b.min.y >= clip.min.y
            && b.min.z >= clip.min.z
            && b.max.x <= clip.max.x
            && b.max.y <= clip.max.y
            && b.max.z <= clip.max.z
        {
            return s;
        }
        return Shape::Oriented(Oriented {
            frame: Frame::Id,
            origin: QVec3::new(0, 0, 0),
            local: LocalShape::Cylinder {
                axis,
                base: start,
                length,
                radius,
            },
            clip,
        });
    }
    Shape::Oriented(Oriented {
        frame: Frame::dir(d),
        origin: start,
        local: LocalShape::Cylinder {
            axis: Axis::X,
            base: QVec3::new(0, 0, 0),
            length,
            radius,
        },
        clip,
    })
}

/// Quanta of one metre, re-exported for callers building frames.
pub const Q: i32 = Q_PER_M;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn qb(min: [i32; 3], max: [i32; 3]) -> QAabb {
        QAabb::new(
            QVec3::new(min[0], min[1], min[2]),
            QVec3::new(max[0], max[1], max[2]),
        )
    }

    fn cell() -> QAabb {
        qb([0; 3], [64 * Q; 3])
    }

    #[test]
    fn heading_table_matches_tan() {
        for (r, &(a, b)) in HEADING_QUADRANT.iter().enumerate() {
            let t = (5.625f64 * r as f64).to_radians().tan();
            if r <= 8 {
                assert_eq!((a, b), (64, (64.0 * t).round() as i32), "r={r}");
            } else {
                let t45 = (5.625f64 * (16 - r) as f64).to_radians().tan();
                assert_eq!((a, b), ((64.0 * t45).round() as i32, 64), "r={r}");
            }
        }
        for (t, &s) in TILT_SLOPE.iter().enumerate() {
            let want = (64.0 * (5.0f64 * t as f64).to_radians().tan()).round() as i32;
            assert_eq!(s, want, "tilt {t}");
        }
        for n in [8u8, 16, 32, 64] {
            let exact = 128.0 * (std::f64::consts::PI / f64::from(n)).tan();
            assert_eq!(f64::from(chord_k(n)), exact.ceil(), "n={n}");
        }
    }

    #[test]
    fn direction_table() {
        assert_eq!(DIRS.len(), 49);
        assert_eq!(DIRS[0], [0, 0, 1]);
        assert_eq!(DIRS[48], [2, 2, 1]);
        let mut count = [0; 4];
        for i in 0..DIR_COUNT {
            count[dir_class(i) as usize] += 1;
        }
        assert_eq!(count, [3, 6, 4, 36]);
        let mut sorted = DIRS;
        sorted.sort_unstable();
        assert_eq!(sorted, DIRS, "lexicographic order");
    }

    #[test]
    fn headings_quarter_rule() {
        for k in 0..64u8 {
            let (a, b) = heading(k);
            assert_eq!(heading(k + 32), (-a, -b));
            assert_eq!(heading(k + 16), (-b, a));
        }
        assert_eq!(heading(0), (64, 0));
        assert_eq!(heading(16), (0, 64));
    }

    #[test]
    fn axes_are_orthonormal() {
        let mut frames = vec![];
        for yaw in 0..64 {
            for tilt in 0..TILTS {
                frames.push(Frame::table(yaw, tilt));
            }
        }
        frames.extend(DIRS.iter().map(|&d| Frame::dir(d)));
        for f in frames {
            let [x, y, z] = f.axes();
            for (v, name) in [(x, "x"), (y, "y"), (z, "z")] {
                assert!((v.length() - 1.0).abs() < 1e-12, "{f:?} {name} not unit");
            }
            assert!(x.dot(y).abs() < 1e-12 && x.dot(z).abs() < 1e-12 && y.dot(z).abs() < 1e-12);
            assert!(
                (x.cross(y).dot(z) - 1.0).abs() < 1e-12,
                "{f:?} is a reflection"
            );
        }
    }

    #[test]
    fn identity_and_quarter_turns_are_exact() {
        assert_eq!(Frame::table(0, 0), Frame::Id);
        assert_eq!(Frame::dir([1, 0, 0]), Frame::Id);
        assert_eq!(Frame::Id.axes(), [DVec3::X, DVec3::Y, DVec3::Z]);
        for q in 0..4u8 {
            let f = Frame::table(16 * q, 0);
            assert!(f.is_axis_aligned(), "{f:?}");
        }
        // Quarter turn: local x -> world z, local z -> world -x.
        let [x, y, z] = Frame::table(16, 0).axes();
        assert_eq!((x, y, z), (DVec3::Z, DVec3::Y, -DVec3::X));
        assert!(!Frame::table(1, 0).is_axis_aligned());
        assert!(!Frame::table(0, 1).is_axis_aligned());
        // Beams along an axis have exact axes too.
        for d in [[0, 1, 0], [0, 0, 1]] {
            assert!(Frame::dir(d).is_axis_aligned());
        }
    }

    #[test]
    fn identity_oriented_box_is_the_plain_box() {
        let aabb = qb([16, 16, 16], [160, 96, 80]);
        let s = Shape::oriented(
            Frame::Id,
            QVec3::new(0, 0, 0),
            LocalShape::Box { aabb },
            cell(),
        );
        assert_eq!(s, Shape::Box { aabb });
        // Shifted by the origin.
        let s = Shape::oriented(
            Frame::Id,
            QVec3::new(32, 0, 0),
            LocalShape::Box { aabb },
            cell(),
        );
        assert_eq!(
            s,
            Shape::Box {
                aabb: qb([48, 16, 16], [192, 96, 80])
            }
        );
        // Crossing the clip keeps the clip, so it stays oriented.
        let wide = qb([0, 0, 0], [80 * Q, 16, 16]);
        let s = Shape::oriented(
            Frame::Id,
            QVec3::new(0, 0, 0),
            LocalShape::Box { aabb: wide },
            cell(),
        );
        assert!(matches!(s, Shape::Oriented(_)));
    }

    #[test]
    fn quarter_turn_box_matches_plain_box() {
        // A box in the quarter-turn frame covers the world box (-z, y, x) from the origin.
        let local = qb([2 * Q, 0, Q], [10 * Q, 4 * Q, 5 * Q]);
        let o = Oriented {
            frame: Frame::table(16, 0),
            origin: QVec3::new(20 * Q, 8 * Q, 6 * Q),
            local: LocalShape::Box { aabb: local },
            clip: cell(),
        };
        // x_world = ox - lz ∈ (20-5, 20-1], y = 8 + ly, z_world = oz + lx ∈ [6+2, 6+10).
        let world = Shape::Box {
            aabb: qb([15 * Q, 8 * Q, 8 * Q], [19 * Q, 12 * Q, 16 * Q]),
        };
        let mut rng = Rng::new(7);
        for _ in 0..5000 {
            let p = Vec3::new(
                rng.next_f32() * 64.0,
                rng.next_f32() * 64.0,
                rng.next_f32() * 64.0,
            );
            // Skip points within rounding of a face.
            let near = |v: f32, w: f32| (v - w).abs() < 1e-3;
            if [15.0, 19.0].iter().any(|&w| near(p.x, w))
                || [8.0, 12.0].iter().any(|&w| near(p.y, w))
                || [8.0, 16.0].iter().any(|&w| near(p.z, w))
            {
                continue;
            }
            assert_eq!(o.contains(p), world.contains(p), "at {p}");
            assert!((o.sdf(p) - world.sdf(p).max(Shape::Box { aabb: cell() }.sdf(p))).abs() < 1e-4);
        }
        assert_eq!(
            o.aabb(),
            qb([15 * Q, 8 * Q, 8 * Q], [19 * Q, 12 * Q, 16 * Q])
        );
    }

    #[test]
    fn tilted_box_bounds_and_distance() {
        let local = qb([-4 * Q, -Q, -3 * Q], [4 * Q, Q, 3 * Q]);
        let mut rng = Rng::new(99);
        for yaw in [1u8, 5, 9, 13, 21, 37, 50] {
            for tilt in 0..TILTS {
                let o = Oriented {
                    frame: Frame::table(yaw, tilt),
                    origin: QVec3::new(32 * Q, 32 * Q, 32 * Q),
                    local: LocalShape::Box { aabb: local },
                    clip: cell(),
                };
                let b = o.aabb();
                for _ in 0..400 {
                    let p = Vec3::new(
                        26.0 + rng.next_f32() * 12.0,
                        26.0 + rng.next_f32() * 12.0,
                        26.0 + rng.next_f32() * 12.0,
                    );
                    if o.contains(p) {
                        assert!(
                            p.cmpge(b.min.to_metres()).all() && p.cmplt(b.max.to_metres()).all(),
                            "{yaw}/{tilt}: {p} inside but outside {b:?}"
                        );
                        assert!(o.sdf(p) <= 1e-4);
                    } else {
                        assert!(o.sdf(p) >= -1e-4);
                    }
                }
            }
        }
    }

    #[test]
    fn sdf_is_a_lower_bound_for_a_rotated_cylinder() {
        let o = Oriented {
            frame: Frame::dir([1, 2, -1]),
            origin: QVec3::new(10 * Q, 10 * Q, 40 * Q),
            local: LocalShape::Cylinder {
                axis: Axis::X,
                base: QVec3::new(0, 0, 0),
                length: 30 * Q,
                radius: 2 * Q,
            },
            clip: cell(),
        };
        let mut rng = Rng::new(5);
        for _ in 0..1500 {
            let p = Vec3::new(
                rng.next_f32() * 64.0,
                rng.next_f32() * 64.0,
                rng.next_f32() * 64.0,
            );
            let d = o.sdf(p);
            if d <= 0.01 {
                continue;
            }
            for _ in 0..24 {
                let dir = (Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32())
                    - Vec3::splat(0.5))
                .normalize_or_zero();
                let q = p + dir * (d - 0.01) * rng.next_f32();
                assert!(!o.contains(q), "{q} inside the ball of radius {d} at {p}");
            }
        }
    }

    #[test]
    fn clip_cuts_pieces_of_one_solid_exactly() {
        // One long oblique beam, clipped by two adjacent cells: every point is in exactly one.
        let make = |clip: QAabb| Oriented {
            frame: Frame::dir([2, 1, 0]),
            origin: QVec3::new(-10 * Q, 20 * Q, 30 * Q),
            local: LocalShape::Cylinder {
                axis: Axis::X,
                base: QVec3::new(0, 0, 0),
                length: 100 * Q,
                radius: 3 * Q,
            },
            clip,
        };
        let whole = make(qb([-64 * Q, -64 * Q, -64 * Q], [128 * Q, 128 * Q, 128 * Q]));
        let left = make(qb([-64 * Q, -64 * Q, -64 * Q], [32 * Q, 128 * Q, 128 * Q]));
        let right = make(qb([32 * Q, -64 * Q, -64 * Q], [128 * Q, 128 * Q, 128 * Q]));
        let mut rng = Rng::new(3);
        for _ in 0..6000 {
            let p = Vec3::new(
                rng.next_f32() * 100.0 - 20.0,
                rng.next_f32() * 100.0 - 10.0,
                rng.next_f32() * 100.0 - 10.0,
            );
            assert_eq!(
                whole.contains(p),
                left.contains(p) ^ right.contains(p),
                "at {p}"
            );
        }
    }

    #[test]
    fn beam_along_an_axis_is_a_plain_cylinder() {
        let s = beam_cylinder(
            [0, 1, 0],
            QVec3::new(32 * Q, 0, 32 * Q),
            64 * Q,
            4 * Q,
            cell(),
        );
        assert_eq!(
            s,
            Shape::cylinder(Axis::Y, QVec3::new(32 * Q, 0, 32 * Q), 64 * Q, 4 * Q)
        );
        let s = beam_cylinder([1, 1, 0], QVec3::new(0, 0, 32 * Q), 40 * Q, 4 * Q, cell());
        assert!(matches!(s, Shape::Oriented(_)));
    }

    #[test]
    fn helix_is_a_monotone_chain_of_treads() {
        let h = HelixStair {
            axis: QVec3::new(32 * Q, 0, 32 * Q),
            inner: 2 * Q,
            width: 4 * Q,
            per_turn: 16,
            steps: 48,
            rise: 3,
            thick: 8,
            start: 0,
            sense: 1,
            material: MaterialId::METAL,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        };
        let prims = helix_stair(&h, cell());
        assert_eq!(prims.len(), 48);
        // Step j top is (j+1)*3 quanta; a point on the middle of a tread is solid just below it.
        for (j, p) in prims.iter().enumerate() {
            let Shape::Oriented(o) = p.shape else {
                // Heading 0, 16, 32, 48 are quarter turns: plain boxes.
                assert!(j % 4 == 0, "step {j} should be oriented");
                continue;
            };
            let [ex, _, _] = o.frame.axes();
            let mid = DVec3::new(32.0, 0.0, 32.0) + ex * 4.0;
            let top = (j as f32 + 1.0) * 3.0 / Q as f32;
            let q = Vec3::new(mid.x as f32, top - 0.1, mid.z as f32);
            assert!(p.shape.contains(q), "step {j}");
            assert!(!p.shape.contains(q + Vec3::Y * 0.2), "step {j} top");
        }
        // 16 per turn: the heading comes round after 16 steps.
        let yaw = |p: &Primitive| match p.shape {
            Shape::Oriented(o) => o.frame,
            _ => Frame::Id,
        };
        assert_eq!(yaw(&prims[0]), Frame::Id);
        assert_eq!(yaw(&prims[16]), Frame::Id);
        assert_eq!(yaw(&prims[2]), Frame::table(8, 0));
    }

    #[test]
    fn facet_ring_closes() {
        let r = FacetRing {
            axis: QVec3::new(32 * Q, 0, 32 * Q),
            radius: 10 * Q,
            thick: Q,
            height: 4 * Q,
            n: 16,
            start: 0,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags: PrimFlags::NONE,
        };
        let prims = facet_ring(&r, cell());
        assert_eq!(prims.len(), 16);
        // Sweep the circle at mid-wall radius: there must be solid within 0.3 m everywhere.
        for k in 0..360 {
            let a = (k as f32).to_radians();
            let (s, c) = a.sin_cos();
            let p = Vec3::new(32.0 + 10.5 * c, 2.0, 32.0 + 10.5 * s);
            let hit = prims.iter().any(|q| q.shape.sdf(p) < 0.4);
            assert!(hit, "gap at {k}°");
        }
    }

    /// `hashing.md` §6.1: no transcendental function outside tests. The scan covers the
    /// non-test part of every source file of this crate.
    #[test]
    fn no_transcendentals_in_generator_code() {
        const BANNED: [&str; 11] = [
            ".sin(", ".cos(", ".tan(", ".atan(", ".atan2(", ".exp(", ".ln(", ".powf(", ".powi(",
            ".hypot(", ".cbrt(",
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = vec![];
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let live = text.split("#[cfg(test)]").next().unwrap();
            for (n, line) in live.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                for b in BANNED.iter().chain(&[".mul_add("]) {
                    if code.contains(b) {
                        bad.push(format!("{}:{}: {b}", path.display(), n + 1));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "banned calls in generator code: {bad:#?}");
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        fn frame() -> impl Strategy<Value = Frame> {
            prop_oneof![
                (0u8..64, 0u8..TILTS).prop_map(|(y, t)| Frame::table(y, t)),
                (0usize..DIR_COUNT).prop_map(|i| Frame::dir(DIRS[i])),
            ]
        }

        proptest! {
            /// Inside the solid implies inside the integer bounds, whatever the frame.
            #[test]
            fn solid_is_inside_its_bounds(
                f in frame(),
                ox in 16i32..48, oy in 16i32..48, oz in 16i32..48,
                hx in 1i32..12, hy in 1i32..12, hz in 1i32..12,
                px in 0.0f32..64.0, py in 0.0f32..64.0, pz in 0.0f32..64.0,
            ) {
                let o = Oriented {
                    frame: f,
                    origin: QVec3::new(ox * Q, oy * Q, oz * Q),
                    local: LocalShape::Box { aabb: qb([-hx * Q, -hy * Q, -hz * Q], [hx * Q, hy * Q, hz * Q]) },
                    clip: cell(),
                };
                let p = Vec3::new(px, py, pz);
                if o.contains(p) {
                    let b = o.aabb();
                    prop_assert!(p.cmpge(b.min.to_metres()).all() && p.cmplt(b.max.to_metres()).all());
                    prop_assert!(o.sdf(p) <= 1e-4);
                } else {
                    prop_assert!(o.sdf(p) >= -1e-4);
                }
            }

            /// Pieces of one solid under two complementary clips never overlap and never leave gaps.
            #[test]
            fn clips_partition_the_solid(
                f in frame(),
                split in 8i32..56,
                px in 0.0f32..64.0, py in 0.0f32..64.0, pz in 0.0f32..64.0,
            ) {
                let make = |clip| Oriented {
                    frame: f,
                    origin: QVec3::new(32 * Q, 32 * Q, 32 * Q),
                    local: LocalShape::Box { aabb: qb([-20 * Q, -6 * Q, -8 * Q], [20 * Q, 6 * Q, 8 * Q]) },
                    clip,
                };
                let all = make(cell());
                let a = make(qb([0, 0, 0], [split * Q, 64 * Q, 64 * Q]));
                let b = make(qb([split * Q, 0, 0], [64 * Q, 64 * Q, 64 * Q]));
                let p = Vec3::new(px, py, pz);
                prop_assert_eq!(all.contains(p), a.contains(p) ^ b.contains(p));
            }
        }
    }
}
