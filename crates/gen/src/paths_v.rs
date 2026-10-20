//! Vertical path types PV-01..10 (`levels.md` §5W.17): what a straight vertical run of route
//! slots is built as. A run of `k` slots climbs `32 k` m inside one 32 m column, so every type
//! folds, spirals or winds. The plan is drawn in `(u, v)` metres about the column centre and
//! mapped to the lattice frame by a swap and two flips.
//!
//! Stairs are 0.1875 m by 0.3125 m (3 Q by 5 Q, `traversal.md`), built as `StairRun` pieces
//! that are whole flights between landings. Ids in `PathPieces::chosen` are `20 + PV number`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::orient::{Frame, HelixStair, LocalShape, helix_stair};
use crate::paths::Clear;
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::tissue_grow::{Emitter, quanta};

/// The ids of this module: 20 plus the PV number, PV-01..PV-10.
pub const VIDS: [u8; 19] = [
    21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
];

/// Rise and tread of a stair step, in metres (3 Q and 5 Q).
const RISE: f64 = 0.1875;
const TREAD: f64 = 0.3125;
/// A comfortable ramp: rise per metre of run (about 15°).
const RAMP_SLOPE: f64 = 0.268;

/// How a flight climbs: rise and run of one unit (a step, or half a metre of ramp).
#[derive(Debug, Clone, Copy)]
struct Style {
    rise: f64,
    run: f64,
    ramp: bool,
}

/// The walking stair of `traversal.md`: 3 Q by 5 Q.
const STAIR: Style = Style {
    rise: RISE,
    run: TREAD,
    ramp: false,
};
/// The stair of a stair beam (`levels.md` §4W.2.1a): 4 Q by 8 Q, a slope of 1 in 2.
const BEAM: Style = Style {
    rise: 0.25,
    run: 0.5,
    ramp: false,
};
/// The inclined plane (PV-15): a ramp of 25°, in half-metre units.
const SLAB: Style = Style {
    rise: 0.2332,
    run: 0.5,
    ramp: true,
};

/// The modifiers of one vertical run.
#[derive(Debug, Clone, Copy)]
pub struct VParams {
    /// Flight width in metres.
    pub w: f64,
    pub swap: bool,
    pub fx: f64,
    pub fz: f64,
    /// Fraction in `[0, 1)` for sizes.
    pub k: f64,
    /// −1 or +1: the side of a façade (PV-07) and the sense of the helix (PV-05).
    pub side: f64,
}

/// The width range of each type in metres.
pub fn width_range(id: u8) -> (f64, f64) {
    match id {
        21 => (2.0, 4.0),
        22 => (2.0, 3.5),
        23 | 32 | 39 => (2.0, 3.0),
        24 | 25 => (2.5, 4.0),
        26 => (4.0, 6.0),
        27 | 36 | 37 => (1.0, 1.2),
        28 => (7.0, 9.0),
        29 => (2.0, 2.5),
        30 => (9.0, 10.0),
        31 => (3.0, 4.5),
        33 => (2.5, 3.5),
        34 => (8.0, 10.0),
        35 => (5.0, 9.0),
        _ => (1.4, 1.6),
    }
}

/// What a built run reports: where a walker goes (cell-local metres) and the box it fills in
/// the lattice frame.
#[derive(Debug, Clone, Default)]
pub struct Rise {
    pub walk: Vec<[f64; 3]>,
    pub bounds: Option<Clear>,
}

/// The column: centre in lattice-frame metres, floor of the lowest slot, height.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub cx: f64,
    pub cz: f64,
    pub y0: f64,
    pub height: f64,
}

type S = (f64, f64);
type P = (f64, f64);

struct M<'a, 'b> {
    e: &'b mut Emitter<'a>,
    col: Column,
    p: VParams,
    /// Lattice-frame samples of the walker's route.
    walk: Vec<[f64; 3]>,
    /// Cell-local samples, for a type that builds in cell-local frames (PV-05).
    local_walk: Option<Vec<[f64; 3]>>,
    style: Style,
    lo: [f64; 3],
    hi: [f64; 3],
}

impl M<'_, '_> {
    fn xz(&self, u: f64, v: f64) -> (f64, f64) {
        let (a, b) = if self.p.swap { (v, u) } else { (u, v) };
        (self.col.cx + self.p.fx * a, self.col.cz + self.p.fz * b)
    }

    /// The lattice direction of a plan direction.
    fn dir4(&self, d: P) -> Dir4 {
        let (a, b) = if self.p.swap { (d.1, d.0) } else { d };
        let (a, b) = (a * self.p.fx, b * self.p.fz);
        if a > 0.0 {
            Dir4::PosX
        } else if a < 0.0 {
            Dir4::NegX
        } else if b > 0.0 {
            Dir4::PosZ
        } else {
            Dir4::NegZ
        }
    }

    /// Lattice-frame corners of a plan box.
    fn corners(&self, u: S, v: S, y: S) -> ([f64; 3], [f64; 3]) {
        let (x0, z0) = self.xz(u.0, v.0);
        let (x1, z1) = self.xz(u.1, v.1);
        (
            [x0.min(x1), y.0.min(y.1), z0.min(z1)],
            [x0.max(x1), y.0.max(y.1), z0.max(z1)],
        )
    }

    fn grow(&mut self, lo: [f64; 3], hi: [f64; 3]) {
        for k in 0..3 {
            self.lo[k] = self.lo[k].min(lo[k]);
            self.hi[k] = self.hi[k].max(hi[k]);
        }
    }

    fn slab(&mut self, u: S, v: S, y: S, m: MaterialId, kind: FeatureKind) {
        let (lo, hi) = self.corners(u, v, y);
        self.grow(lo, hi);
        self.e.frame_box(lo, hi, m, kind, PrimFlags::NONE);
    }

    /// A box that nothing collides with (cabins, markers).
    fn bx_free(&mut self, u: S, v: S, y: S, m: MaterialId, kind: FeatureKind) {
        let (lo, hi) = self.corners(u, v, y);
        self.grow(lo, hi);
        self.e.frame_box(lo, hi, m, kind, PrimFlags::NO_COLLIDE);
    }

    /// A landing or deck, 0.5 m thick, its top at `top`, over plan ranges.
    fn deck(&mut self, u: S, v: S, top: f64) {
        // Decks overlap what they join by 7 cm, more than rounding to quanta can open.
        const JOIN: f64 = 0.07;
        self.slab(
            (u.0 - JOIN, u.1 + JOIN),
            (v.0 - JOIN, v.1 + JOIN),
            (top - 0.5, top),
            MaterialId::CONCRETE,
            FeatureKind::WALKWAY,
        );
    }

    /// A square landing of side `w` centred on `c`.
    fn landing(&mut self, c: P, top: f64) {
        let h = self.p.w / 2.0;
        self.deck((c.0 - h, c.0 + h), (c.1 - h, c.1 + h), top);
    }

    /// A strip of width `w` between two points that share a coordinate.
    fn strip(&mut self, a: P, b: P, top: f64) {
        let h = self.p.w / 2.0;
        self.deck(
            (a.0.min(b.0) - h, a.0.max(b.0) + h),
            (a.1.min(b.1) - h, a.1.max(b.1) + h),
            top,
        );
    }

    fn sample(&mut self, p: P, y: f64) {
        let (x, z) = self.xz(p.0, p.1);
        self.walk.push([x, y, z]);
    }

    /// Samples every half metre along a plan line at height `y`.
    fn line(&mut self, a: P, b: P, y: f64) {
        let len = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        let n = (len / 0.5).ceil().max(1.0) as u32;
        for i in 0..=n {
            let f = f64::from(i) / f64::from(n);
            self.sample((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f), y);
        }
    }

    /// A flight of `n` units from `from`, along plan `u` if `axis_u` else `v`, direction `dir`
    /// (±1), centred on the other coordinate, foot at height `y`. Returns the end and its height.
    fn flight(&mut self, from: P, axis_u: bool, dir: f64, n: u32, y: f64) -> (P, f64) {
        let st = self.style;
        if st.ramp {
            return self.ramp(
                from,
                axis_u,
                dir,
                f64::from(n) * st.run,
                y,
                st.rise / st.run,
            );
        }
        let w = self.p.w;
        let run = f64::from(n) * st.run;
        let rise = f64::from(n) * st.rise;
        let (end, d) = if axis_u {
            ((from.0 + dir * run, from.1), (dir, 0.0))
        } else {
            ((from.0, from.1 + dir * run), (0.0, dir))
        };
        let (u, v) = if axis_u {
            ((from.0, end.0), (from.1 - w / 2.0, from.1 + w / 2.0))
        } else {
            ((from.0 - w / 2.0, from.0 + w / 2.0), (from.1, end.1))
        };
        let (lo, hi) = self.corners(u, v, (y, y + rise));
        self.grow(lo, hi);
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(quanta(size[0]), quanta(size[1]), quanta(size[2])),
        );
        let origin = Emitter::qv(self.e.at(lo));
        self.e.out.push(Primitive {
            shape: Shape::oriented(
                self.e.lat.frame,
                origin,
                LocalShape::StairRun {
                    aabb: local,
                    rise: self.dir4(d),
                    steps: n as u16,
                },
                self.e.cell,
            ),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        });
        for i in 0..n {
            let s = (f64::from(i) + 0.5) * st.run;
            let at = (from.0 + d.0 * s, from.1 + d.1 * s);
            self.sample(at, y + f64::from(i + 1) * st.rise);
        }
        (end, y + rise)
    }

    /// A ramp of `run` metres rising `slope` per metre.
    fn ramp(&mut self, from: P, axis_u: bool, dir: f64, run: f64, y: f64, slope: f64) -> (P, f64) {
        let w = self.p.w;
        let rise = run * slope;
        let (end, d) = if axis_u {
            ((from.0 + dir * run, from.1), (dir, 0.0))
        } else {
            ((from.0, from.1 + dir * run), (0.0, dir))
        };
        let (u, v) = if axis_u {
            ((from.0, end.0), (from.1 - w / 2.0, from.1 + w / 2.0))
        } else {
            ((from.0 - w / 2.0, from.0 + w / 2.0), (from.1, end.1))
        };
        let (lo, hi) = self.corners(u, v, (y, y + rise));
        self.grow(lo, hi);
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(quanta(size[0]), quanta(size[1]), quanta(size[2])),
        );
        let origin = Emitter::qv(self.e.at(lo));
        self.e.out.push(Primitive {
            shape: Shape::oriented(
                self.e.lat.frame,
                origin,
                LocalShape::Ramp {
                    aabb: local,
                    rise: self.dir4(d),
                },
                self.e.cell,
            ),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::RAMP,
            flags: PrimFlags::NONE,
        });
        // A footing under the thin foot of the wedge only: the first metre of the run, not a
        // plate under all of it.
        let foot = run.min(1.0);
        let (fu, fv) = if axis_u {
            let e = from.0 + dir * foot;
            ((from.0.min(e), from.0.max(e)), v)
        } else {
            let e = from.1 + dir * foot;
            (u, (from.1.min(e), from.1.max(e)))
        };
        self.slab(
            fu,
            fv,
            (y - 0.5, y + 0.02),
            MaterialId::CONCRETE,
            FeatureKind::RAMP,
        );
        let n = (run / 0.5).floor() as u32;
        for i in 0..n {
            let s = (f64::from(i) + 0.5) * 0.5;
            self.sample((from.0 + d.0 * s, from.1 + d.1 * s), y + s * slope);
        }
        (end, y + rise)
    }

    /// A thin rail of a flight along the lane edge `v`: a box 15 cm high and 30 cm wide on each
    /// tread, so that it follows the nosings and never crosses a step (a wedge laid on the
    /// slope cut every step in a sawtooth).
    fn track(&mut self, u0: f64, dir: f64, n: u32, v: f64, y: f64) {
        let st = self.style;
        for i in 0..n {
            let a = u0 + dir * f64::from(i) * st.run;
            let b = u0 + dir * f64::from(i + 1) * st.run;
            let top = y + f64::from(i + 1) * st.rise;
            self.slab(
                (a.min(b), a.max(b)),
                (v - 0.15, v + 0.15),
                (top - 0.02, top + 0.15),
                MaterialId::CONCRETE,
                FeatureKind::STAIR_TOWER,
            );
        }
    }

    /// A stepped handrail 1 m above a flight, on lane edge `v`, in pieces of two units, each
    /// on a post that stands on its tread: the rail climbs with the flight in steps of one or
    /// two risers, and no piece floats.
    fn handrail(&mut self, u0: f64, dir: f64, n: u32, v: f64, y: f64) {
        let st = self.style;
        let mut j = 0;
        while j < n {
            let k = (n - j).min(2);
            let a = u0 + dir * f64::from(j) * st.run;
            let b = u0 + dir * f64::from(j + k) * st.run;
            let top = y + f64::from(j + k) * st.rise + 1.0;
            self.slab(
                (a.min(b), a.max(b)),
                (v - 0.05, v + 0.05),
                (top, top + 0.12),
                MaterialId::METAL,
                FeatureKind::RAILING,
            );
            let mid = f64::midpoint(a, b);
            let tread = y + f64::from(j + 1) * st.rise;
            self.slab(
                (mid - 0.04, mid + 0.04),
                (v - 0.04, v + 0.04),
                (tread - 0.02, top),
                MaterialId::METAL,
                FeatureKind::RAILING,
            );
            j += k;
        }
    }
}

/// Units of `style` to climb `h` metres.
fn units_for(h: f64, style: Style) -> u32 {
    (h / style.rise).round() as u32
}

/// Splits `total` steps into the fewest flights of at most `n_max`, as equal as possible. A
/// short last flight would put its landing over the flight before it.
fn flights(total: u32, n_max: u32) -> Vec<u32> {
    let count = total.div_ceil(n_max.max(1)).max(1);
    let (base, extra) = (total / count, total % count);
    (0..count).map(|i| base + u32::from(i < extra)).collect()
}

/// Extras a folded chain adds to each flight and landing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extra {
    None,
    /// Piers under the landings (PV-11).
    Piers,
    /// A stepped handrail on each lane edge (PV-18).
    Handrail,
    /// A rail on each lane edge (PV-19).
    Track,
}

/// Flights folded back and forth in two lanes at `v = ∓lane` (first lane `-sign · lane`),
/// each ending on a landing `depth` deep that spans both lanes. The route is sampled only
/// when `walk` is set. Returns the plan `u` where the last flight ended and the top height.
#[allow(clippy::too_many_arguments)]
fn fold_chain(
    m: &mut M<'_, '_>,
    total: u32,
    lh: f64,
    lane: f64,
    depth: f64,
    first: (f64, f64, f64),
    walk: bool,
    extra: Extra,
) -> (f64, f64) {
    let (start, dir0, sign) = first;
    let w = m.p.w;
    let st = m.style;
    let n_max = ((2.0 * lh) / st.run).floor() as u32;
    let reach = lane + w / 2.0;
    let mut y = m.col.y0;
    m.deck((start - dir0 * depth, start), (-reach, reach), y);
    let (mut u, mut dir, mut prev_v) = (start, dir0, 0.0);
    for (i, n) in flights(total, n_max).into_iter().enumerate() {
        let v = if i % 2 == 0 {
            -sign * lane
        } else {
            sign * lane
        };
        let keep = m.walk.len();
        m.line(
            (u - dir * depth / 2.0, prev_v),
            (u - dir * depth / 2.0, v),
            y,
        );
        let foot = y;
        let ((u1, _), y1) = m.flight((u, v), true, dir, n, y);
        y = y1;
        m.deck((u1, u1 + dir * depth), (-reach, reach), y);
        match extra {
            Extra::Handrail => {
                for side in [-1.0, 1.0] {
                    m.handrail(u, dir, n, v + side * (w / 2.0 - 0.08), foot);
                }
            }
            Extra::Track => {
                for side in [-1.0, 1.0] {
                    m.track(u, dir, n, v + side * (w / 2.0 - 0.2), foot);
                }
            }
            Extra::Piers => {
                let low = (y - 2.0 * (y - foot)).max(m.col.y0);
                for pu in [u1 + dir * 0.3, u1 + dir * (depth - 0.3)] {
                    for pv in [-reach + 0.3, reach - 0.3] {
                        m.slab(
                            (pu - 0.25, pu + 0.25),
                            (pv - 0.25, pv + 0.25),
                            (low, y - 0.5),
                            MaterialId::CONCRETE,
                            FeatureKind::MASS,
                        );
                    }
                }
            }
            Extra::None => {}
        }
        if !walk {
            m.walk.truncate(keep);
        }
        (u, dir, prev_v) = (u1, -dir, v);
    }
    // Off the last landing and across to the centre line.
    let keep = m.walk.len();
    let landing_u = u - dir * depth / 2.0;
    m.line((landing_u, prev_v), (landing_u, 0.0), y);
    if !walk {
        m.walk.truncate(keep);
    }
    (u, y)
}

/// The decks that join a stair's foot and top to a shaft: from the corridor out to `u = 6.8`,
/// along to `v = 11`, and in at the shaft's door.
fn connect(m: &mut M<'_, '_>, y: f64) {
    m.deck((3.0, 6.8), (-0.8, 0.8), y);
    m.deck((5.2, 6.8), (0.0, 12.2), y);
    m.deck((5.2, 8.2), (10.2, 11.8), y);
}

/// PV-16: a lift shaft beside a narrow stair, freight, cage or open platform.
fn lift(m: &mut M<'_, '_>, y0: f64, y_top: f64) {
    let kind = (m.p.k * 3.0) as u32;
    connect(m, y0);
    connect(m, y_top);
    let (u0, u1, v0, v1) = (8.0, 14.0, 8.0, 14.0);
    for y in [y0, y_top] {
        m.deck((u0, u1), (v0, v1), y);
    }
    let top = y_top + 5.0;
    let wall_mat = if kind == 1 {
        MaterialId::GRATING
    } else {
        MaterialId::CONCRETE
    };
    if kind == 2 {
        // An open platform: four posts and a header.
        for (pu, pv) in [
            (u0, v0),
            (u0, v1 - 0.5),
            (u1 - 0.5, v0),
            (u1 - 0.5, v1 - 0.5),
        ] {
            m.slab(
                (pu, pu + 0.5),
                (pv, pv + 0.5),
                (y0, top),
                MaterialId::METAL,
                FeatureKind::SHAFT_WALL,
            );
        }
    } else {
        // The wall towards the stair has a door at each stop, 4 m high and 3 m wide.
        let t = 0.5;
        let door = (9.5, 12.5);
        let kind_w = FeatureKind::SHAFT_WALL;
        m.slab((u0, u0 + t), (v0, door.0), (y0, top), wall_mat, kind_w);
        m.slab((u0, u0 + t), (door.1, v1), (y0, top), wall_mat, kind_w);
        m.slab((u0, u0 + t), door, (y0 + 4.0, y_top), wall_mat, kind_w);
        m.slab((u0, u0 + t), door, (y_top + 4.0, top), wall_mat, kind_w);
        m.slab((u1 - t, u1), (v0, v1), (y0, top), wall_mat, kind_w);
        m.slab((u0, u1), (v0, v0 + t), (y0, top), wall_mat, kind_w);
        m.slab((u0, u1), (v1 - t, v1), (y0, top), wall_mat, kind_w);
        m.slab((u0, u1), (v0, v1), (top, top + 0.5), wall_mat, kind_w);
    }
    // The car's way, for the engine (identity frames only: the shape is axis-aligned).
    if m.e.lat.frame == Frame::Id {
        let (lo, hi) = m.corners((8.5, 13.5), (8.5, 13.5), (y0, y_top + 4.0));
        let q = |c: [f64; 3]| QVec3::new(quanta(c[0]), quanta(c[1]), quanta(c[2]));
        let (a, b) = (m.e.at(lo), m.e.at(hi));
        m.e.out.push(Primitive {
            shape: Shape::LiftShaft {
                aabb: QAabb::new(q([a.x, a.y, a.z]), q([b.x, b.y, b.z])),
            },
            material: MaterialId::METAL,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NO_COLLIDE,
        });
    }
}

/// PV-17: a paternoster, two open shafts side by side with a loop of cabins.
fn paternoster(m: &mut M<'_, '_>, y0: f64, y_top: f64) {
    connect(m, y0);
    connect(m, y_top);
    for y in [y0, y_top] {
        m.deck((8.0, 14.5), (9.0, 12.0), y);
    }
    for (a, b, phase) in [(8.0, 11.0, 0.0), (11.5, 14.5, 3.0)] {
        for (pu, pv) in [(a, 9.0), (a, 11.6), (b - 0.4, 9.0), (b - 0.4, 11.6)] {
            m.slab(
                (pu, pu + 0.4),
                (pv, pv + 0.4),
                (y0, y_top + 4.0),
                MaterialId::METAL,
                FeatureKind::SHAFT_WALL,
            );
        }
        let mut y = y0 + phase;
        while y < y_top + 2.0 {
            let c = f64::midpoint(a, b);
            m.bx_free(
                (c - 0.8, c + 0.8),
                (10.0, 11.6),
                (y, y + 2.4),
                MaterialId::WARM,
                FeatureKind::MISC,
            );
            y += 6.0;
        }
    }
}

/// PV-01..03, 07..11, 14, 15, 18, 19 (and 12, 16, 17, 20): folded flights with a free
/// corridor down the middle.
fn fold(m: &mut M<'_, '_>, id: u8, total: u32) {
    let w = m.p.w;
    let (lh, depth) = match id {
        21 => (11.5, w),
        22 => (5.0 + 3.0 * m.p.k, w),
        23 | 29 => (10.0, w),
        27 | 36 | 37 => (3.5, w),
        28 => (6.0, w),
        31 => (10.5, w),
        32 => (12.5, w),
        34 => (5.0, 6.0),
        35 => (11.0, 4.0),
        38 => (8.0, w),
        39 => (12.0, w),
        _ => (5.0, w),
    };
    let extra = match id {
        31 => Extra::Piers,
        38 => Extra::Handrail,
        39 => Extra::Track,
        _ => Extra::None,
    };
    let y0 = m.col.y0;
    // The bottom corridor runs the length of the stair, so every base landing meets it.
    m.deck((-lh, lh), (-w / 2.0, w / 2.0), y0);
    m.line((0.0, 0.0), (-lh, 0.0), y0);
    let (end, y_top) = fold_chain(m, total, lh, w, depth, (-lh, 1.0, 1.0), true, extra);
    if id == 23 {
        // The partner stair of a scissor: turned half a circle, in the outer lanes.
        fold_chain(
            m,
            total,
            lh,
            3.0 * w,
            depth,
            (lh, -1.0, -1.0),
            false,
            Extra::None,
        );
    }
    m.deck((-lh, lh), (-w / 2.0, w / 2.0), y_top);
    m.line((end, 0.0), (0.0, 0.0), y_top);
    let n_max = ((2.0 * lh) / m.style.run).floor() as u32;
    if id == 27 {
        // Ties to the façade at every landing.
        let reach = w + w / 2.0;
        let (mut y, mut u, mut dir) = (y0, -lh, 1.0);
        for n in flights(total, n_max) {
            u += dir * f64::from(n) * m.style.run;
            y += f64::from(n) * m.style.rise;
            let wall = m.p.side * 15.5;
            m.slab(
                (u, u + dir * 0.4),
                (wall.min(m.p.side * reach), wall.max(m.p.side * reach)),
                (y - 0.9, y - 0.5),
                MaterialId::METAL,
                FeatureKind::MISC,
            );
            dir = -dir;
        }
    }
    if id == 29 {
        // The walls of the thickness the stair lives in.
        let reach = w + w / 2.0;
        for side in [-1.0, 1.0] {
            m.slab(
                (-lh - w, lh + w),
                (side * (reach + 0.1), side * (reach + 1.1)),
                (y0, y_top),
                MaterialId::CONCRETE,
                FeatureKind::MASS,
            );
        }
    }
    match id {
        36 => lift(m, y0, y_top),
        37 => paternoster(m, y0, y_top),
        _ => {}
    }
}

/// PV-04, PV-06 and PV-13: a square spiral of side `2s` round a well, stair or ramp,
/// counter-clockwise; with a `pillar` the well holds a trunk, clear of the foot and the top.
fn ring(m: &mut M<'_, '_>, ramp: bool, total: u32, s: f64, pillar: bool) {
    let w = m.p.w;
    let y0 = m.col.y0;
    let corners = [(-s, -s), (s, -s), (s, s), (-s, s)];
    let dirs = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let inward = [(0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)];
    let c0 = corners[0];
    // Base: along the middle to the first corner's side, then down the side to its landing.
    m.strip((-s, 0.0), (w / 2.0, 0.0), y0);
    m.strip((-s, 0.0), c0, y0);
    m.landing(c0, y0);
    m.line((0.0, 0.0), (-s, 0.0), y0);
    m.line((-s, 0.0), c0, y0);
    let full = 2.0 * s - w;
    let n_full = (full / TREAD).floor() as u32;
    let (mut y, mut landing, mut i) = (y0, c0, 0usize);
    let mut steps_left = total;
    let mut ramp_left = f64::from(total) * RISE;
    loop {
        let d = dirs[i % 4];
        let (axis_u, dir) = (d.1 == 0.0, d.0 + d.1);
        let start = (landing.0 + d.0 * w / 2.0, landing.1 + d.1 * w / 2.0);
        m.line(landing, (start.0 - d.0 * 0.05, start.1 - d.1 * 0.05), y);
        let ((end, y1), done) = if ramp {
            let run = (ramp_left / RAMP_SLOPE).min(full);
            ramp_left -= run * RAMP_SLOPE;
            (
                m.ramp(start, axis_u, dir, run, y, RAMP_SLOPE),
                ramp_left < 0.05,
            )
        } else {
            let n = steps_left.min(n_full);
            steps_left -= n;
            (m.flight(start, axis_u, dir, n, y), steps_left == 0)
        };
        y = y1;
        landing = (end.0 + d.0 * w / 2.0, end.1 + d.1 * w / 2.0);
        m.landing(landing, y);
        m.line((end.0 + d.0 * 0.05, end.1 + d.1 * 0.05), landing, y);
        if done {
            break;
        }
        i += 1;
    }
    // Top: inward from the last landing, then along the middle to the centre.
    let inw = inward[i % 4];
    let to = (
        if inw.0 == 0.0 { landing.0 } else { 0.0 },
        if inw.1 == 0.0 { landing.1 } else { 0.0 },
    );
    m.strip(landing, to, y);
    m.strip(to, (0.0, 0.0), y);
    m.line(landing, to, y);
    m.line(to, (0.0, 0.0), y);
    if pillar {
        let half = s - w / 2.0 - 1.0;
        m.slab(
            (-half, half),
            (-half, half),
            (y0 + 3.0, y - 3.0),
            MaterialId::CONCRETE,
            FeatureKind::TRUNK,
        );
    }
}

/// The yaw of a lattice frame that has no tilt, or `None`.
pub fn flat_yaw(frame: Frame) -> Option<u8> {
    match frame {
        Frame::Id => Some(0),
        Frame::Table { yaw, tilt: 0 } => Some(yaw),
        _ => None,
    }
}

/// PV-05: a true helical stair on the column's axis, built in the lattice frame turned by each
/// heading (so it needs a lattice frame without tilt). Steps are 4 Q high, so the climb is
/// exact.
fn helix(m: &mut M<'_, '_>, total_q: i32, yaw0: u8) {
    let w = m.p.w;
    let rise_q = 4;
    let steps = (total_q / rise_q) as u32;
    let inner = 4.0;
    let sense: i8 = if m.p.side > 0.0 { 1 } else { -1 };
    let start = (i32::from(yaw0) + (m.p.k * 16.0) as i32 * 4).rem_euclid(64) as u8;
    let ay = m.col.y0;
    let top = ay + f64::from(total_q) / 16.0;
    let centre = m.e.at([m.col.cx, ay, m.col.cz]);
    let h = HelixStair {
        axis: Emitter::qv(centre),
        inner: quanta(inner),
        width: quanta(w),
        per_turn: 32,
        steps,
        rise: rise_q,
        thick: 8,
        start,
        sense,
        material: MaterialId::CONCRETE,
        kind: FeatureKind::STAIR_TOWER,
        flags: PrimFlags::NONE,
    };
    let clip = m.e.cell;
    m.e.out.extend(helix_stair(&h, clip));
    let yaw_of = |j: u32| (i32::from(start) + i32::from(sense) * j as i32 * 2).rem_euclid(64) as u8;
    let last = yaw_of(steps - 1);
    // A deck from the axis to the first tread, and from the last tread back to the axis.
    for (yaw, y) in [(start, ay), (last, top)] {
        let local = QAabb::new(
            QVec3::new(0, -quanta(0.5), -quanta(w / 2.0)),
            QVec3::new(quanta(inner) + 2, 0, quanta(w / 2.0)),
        );
        let o = Emitter::qv(m.e.at([m.col.cx, y, m.col.cz]));
        m.e.out.push(Primitive {
            shape: Shape::oriented(
                Frame::table(yaw, 0),
                o,
                LocalShape::Box { aabb: local },
                m.e.cell,
            ),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::WALKWAY,
            flags: PrimFlags::NONE,
        });
    }
    let dir_of = |yaw: u8| {
        let a = Frame::table(yaw, 0).axes()[0];
        (a.x, a.z)
    };
    let rmid = inner + w / 2.0;
    let mut pts: Vec<[f64; 3]> = Vec::new();
    let (dx, dz) = dir_of(start);
    let mut r = 0.0;
    while r < inner {
        pts.push([centre.x + dx * r, centre.y, centre.z + dz * r]);
        r += 0.5;
    }
    for j in 0..steps {
        let (dx, dz) = dir_of(yaw_of(j));
        pts.push([
            centre.x + dx * rmid,
            centre.y + f64::from((j as i32 + 1) * rise_q) / 16.0,
            centre.z + dz * rmid,
        ]);
    }
    let (dx, dz) = dir_of(last);
    let mut r = rmid;
    while r > 0.0 {
        pts.push([centre.x + dx * r, centre.y + top - ay, centre.z + dz * r]);
        r -= 0.5;
    }
    m.local_walk = Some(pts);
    let reach = inner + w + 1.0;
    m.grow(
        [m.col.cx - reach, ay, m.col.cz - reach],
        [m.col.cx + reach, top, m.col.cz + reach],
    );
}

/// Builds vertical type `id` (21..=39) in column `col`.
pub(crate) fn build_rise(e: &mut Emitter<'_>, id: u8, col: Column, p: &VParams) -> Rise {
    let total_q = (col.height * 16.0).round() as i32;
    let style = match id {
        32 | 39 => BEAM,
        35 => SLAB,
        _ => STAIR,
    };
    let units = units_for(col.height, style);
    let mut m = M {
        e,
        col,
        p: *p,
        walk: Vec::new(),
        local_walk: None,
        style,
        lo: [f64::MAX; 3],
        hi: [f64::MIN; 3],
    };
    match id {
        24 => ring(&mut m, false, units, 12.0, false),
        26 => ring(&mut m, true, units, 13.0 - p.w / 2.0, false),
        33 => ring(&mut m, false, units, 8.0, true),
        25 => match flat_yaw(m.e.lat.frame) {
            Some(yaw) => helix(&mut m, total_q, yaw),
            None => fold(&mut m, 21, units),
        },
        _ => fold(&mut m, id, units),
    }
    let walk = match m.local_walk.take() {
        Some(w) => w,
        None => m
            .walk
            .iter()
            .map(|&q| {
                let c = m.e.at(q);
                [c.x, c.y, c.z]
            })
            .collect(),
    };
    let bounds = (m.lo[0] <= m.hi[0]).then_some((m.lo, m.hi));
    Rise { walk, bounds }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tissue::Lattice;

    fn solid_at(prims: &[Primitive], p: glam::Vec3) -> bool {
        prims
            .iter()
            .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
    }

    fn vec(p: [f64; 3]) -> glam::Vec3 {
        glam::Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
    }

    /// Every type, in every orientation of its plan, gives a route from the column centre at
    /// the foot to the column centre at the top, with a floor under every sample and 1.85 m of
    /// headroom above it, inside the 32 m column.
    #[test]
    fn every_type_climbs_the_column() {
        let lat = Lattice::new(Frame::Id);
        for id in VIDS {
            for slots in [1.0, 2.0, 4.0] {
                if id == 28 && slots < 3.0 || id == 25 && slots > 4.0 {
                    continue;
                }
                for variant in 0..4u32 {
                    let (lo, hi) = width_range(id);
                    let f = f64::from(variant) / 3.0;
                    let p = VParams {
                        w: lo + (hi - lo) * f,
                        swap: variant & 1 == 1,
                        fx: if variant & 2 == 2 { 1.0 } else { -1.0 },
                        fz: if variant == 1 { -1.0 } else { 1.0 },
                        k: f,
                        side: if variant % 2 == 0 { 1.0 } else { -1.0 },
                    };
                    let col = Column {
                        cx: 256.0,
                        cz: 256.0,
                        y0: 100.0,
                        height: 32.0 * slots,
                    };
                    let mut e = Emitter::new(&lat);
                    let r = build_rise(&mut e, id, col, &p);
                    let tag = format!("PV-{:02} k {slots} v{variant}", id - 20);
                    let first = r.walk.first().expect("no route");
                    let last = r.walk.last().expect("no route");
                    assert!((first[1] - 100.0).abs() < 0.3, "{tag}: starts at {first:?}");
                    assert!(
                        (last[1] - (100.0 + col.height)).abs() < 0.3,
                        "{tag}: ends at {last:?}"
                    );
                    for end in [first, last] {
                        assert!(
                            (end[0] - 256.0).abs() < 0.6 && (end[2] - 256.0).abs() < 0.6,
                            "{tag}: not at the centre: {end:?}"
                        );
                    }
                    let (a, b) = r.bounds.expect("bounds");
                    assert!(
                        a[0] >= 256.0 - 16.0 && b[0] <= 256.0 + 16.0,
                        "{tag}: {a:?} {b:?}"
                    );
                    assert!(
                        a[2] >= 256.0 - 16.0 && b[2] <= 256.0 + 16.0,
                        "{tag}: {a:?} {b:?}"
                    );
                    for q in &r.walk {
                        assert!(
                            solid_at(&e.out, vec([q[0], q[1] - 0.1, q[2]])),
                            "{tag}: no floor at {q:?}"
                        );
                        for dy in [0.25, 0.9, 1.5, 1.85] {
                            assert!(
                                !solid_at(&e.out, vec([q[0], q[1] + dy, q[2]])),
                                "{tag}: blocked {dy} m above {q:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
