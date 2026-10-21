//! The Frame of an L1 cell: beams keyed by lines on every axis and slant, and the knot.
//! Spec: `docs/spec/levels.md` §4W.1–§4W.2.
//!
//! A beam is a span of an infinite line `(i, p, q)`. Every cell the line crosses derives the
//! same beam from the line's key and emits its own clipped piece (FRM-01).

// Coordinates are bounded by the world's spans and sections; draws are bounded by their tables.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::{DVec3, I64Vec3};

use crate::cell::{Axis, Level};
use crate::hash::{absorb, hash};
use crate::l1_between::WebPiece;
use crate::orient::{DIRS, DirClass, Frame, LocalShape, beam_cylinder, dir_class};
use crate::prim::{
    Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape,
};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

pub const FRM_LINE: &str = "FRM-01";
pub const FRM_OBLIQUE: &str = "FRM-02";
pub const FRM_SECTION: &str = "FRM-03";
pub const FRM_STUB: &str = "FRM-04";
pub const FRM_GIRDER: &str = "FRM-05";
pub const FRM_INHABITED: &str = "FRM-06";
pub const FRM_TRUSS: &str = "FRM-07";
pub const FRM_CONDUIT: &str = "FRM-08";
pub const FRM_RAIL: &str = "FRM-09";
pub const FRM_STEPPED: &str = "FRM-10";
pub const FRM_STAIR: &str = "FRM-11";
pub const FRM_FALLEN: &str = "FRM-12";
pub const FRM_KNOT: &str = "FRM-13";
pub const FRM_BRIDGE: &str = "FRM-14";

const L1_M: i64 = 4096;
const L0_M: i64 = 32_768;
const Q: i64 = Q_PER_M as i64;
/// Beams: L1 lattice pitch (spec §4W.1.4).
const PITCH: i64 = 2048;
/// A span is one L0 cell long along its dominant axis.
const SPAN: i64 = L0_M;

/// The section shapes (FRM-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Square,
    /// Height half of the width.
    Flattened,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BeamKind {
    Girder,
    Inhabited,
    Truss,
    Conduit,
    Rail,
    Stepped,
    Stair,
    Fallen,
}

impl BeamKind {
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Girder => FRM_GIRDER,
            Self::Inhabited => FRM_INHABITED,
            Self::Truss => FRM_TRUSS,
            Self::Conduit => FRM_CONDUIT,
            Self::Rail => FRM_RAIL,
            Self::Stepped => FRM_STEPPED,
            Self::Stair => FRM_STAIR,
            Self::Fallen => FRM_FALLEN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum End {
    /// The beam goes on into the next span.
    Continue,
    /// A junction structure (FRM-13).
    Knot,
    /// A broken end (FRM-04).
    Stub,
}

/// One beam: a span of a line, with everything drawn for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Beam {
    /// Index into the 49-entry direction table.
    pub dir: usize,
    pub p: i64,
    pub q: i64,
    /// Intercepts with the plane `a = 0`, in metres.
    pub u0: i64,
    pub v0: i64,
    /// The span index along the dominant axis.
    pub span: i64,
    /// World extent along the dominant axis, in metres.
    pub a_from: i64,
    pub a_to: i64,
    /// Section width in metres, 32–400 in steps of 16.
    pub width: i64,
    pub section: Section,
    pub kind: BeamKind,
    pub ends: [End; 2],
    /// The L0 cell whose regime owns the span.
    pub home: I64Vec3,
    /// Seed of the kind's own draws (car positions, drift, …).
    pub detail: u64,
}

/// Direction data with the dominant axis made positive (`spec §4W.1.1`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DirInfo {
    /// Dominant, then the two others in cyclic order.
    pub(crate) a: Axis,
    pub(crate) u: Axis,
    pub(crate) v: Axis,
    pub(crate) da: i64,
    pub(crate) du: i64,
    pub(crate) dv: i64,
}

pub(crate) fn dir_info(i: usize) -> DirInfo {
    let d = DIRS[i].map(i64::from);
    // Ties go to Y, then X, then Z.
    let a = if d[1].abs() >= d[0].abs() && d[1].abs() >= d[2].abs() {
        Axis::Y
    } else if d[0].abs() >= d[2].abs() {
        Axis::X
    } else {
        Axis::Z
    };
    let (u, v) = match a {
        Axis::Y => (Axis::X, Axis::Z),
        Axis::X => (Axis::Z, Axis::Y),
        Axis::Z => (Axis::Y, Axis::X),
    };
    let get = |ax: Axis| match ax {
        Axis::X => d[0],
        Axis::Y => d[1],
        Axis::Z => d[2],
    };
    let sign = get(a).signum();
    DirInfo {
        a,
        u,
        v,
        da: get(a) * sign,
        du: get(u) * sign,
        dv: get(v) * sign,
    }
}

fn comp(v: I64Vec3, axis: Axis) -> i64 {
    match axis {
        Axis::X => v.x,
        Axis::Y => v.y,
        Axis::Z => v.z,
    }
}

fn set(v: &mut I64Vec3, axis: Axis, x: i64) {
    match axis {
        Axis::X => v.x = x,
        Axis::Y => v.y = x,
        Axis::Z => v.z = x,
    }
}

impl Beam {
    fn info(&self) -> DirInfo {
        dir_info(self.dir)
    }

    /// The direction with its dominant component positive.
    pub fn direction(&self) -> [i8; 3] {
        let i = self.info();
        let mut d = [0i8; 3];
        for (ax, c) in [(i.a, i.da), (i.u, i.du), (i.v, i.dv)] {
            d[match ax {
                Axis::X => 0,
                Axis::Y => 1,
                Axis::Z => 2,
            }] = c as i8;
        }
        d
    }

    /// The axis point at world coordinate `a` along the dominant axis, in 1/16 m (exact: the
    /// denominator `da` is 1 or 2).
    pub fn point_q(&self, a: i64) -> [i64; 3] {
        let i = self.info();
        let mut w = [0i64; 3];
        let q = |origin: i64, d: i64| (origin * i.da + d * a) * Q / i.da;
        for (ax, val) in [
            (i.a, a * Q),
            (i.u, q(self.u0, i.du)),
            (i.v, q(self.v0, i.dv)),
        ] {
            w[match ax {
                Axis::X => 0,
                Axis::Y => 1,
                Axis::Z => 2,
            }] = val;
        }
        w
    }

    pub fn class(&self) -> DirClass {
        dir_class(self.dir)
    }

    /// The L2 cells (`0..8` on each axis) of L1 cell `l1` that the beam's axis passes through.
    /// The sampling step is half an L2 cell along the dominant axis, so no cell is skipped.
    pub fn l2_cells(&self, l1: I64Vec3) -> Vec<[u8; 3]> {
        let origin = l1 * L1_M;
        let c0 = comp(origin, self.info().a);
        let (a0, a1) = (c0.max(self.a_from), (c0 + L1_M).min(self.a_to));
        let mut out: Vec<[u8; 3]> = Vec::new();
        let mut a = a0;
        while a < a1 {
            let p = self.point_q(a);
            let cell = [0, 1, 2].map(|k| (p[k] / Q - origin[k]).div_euclid(512));
            if cell.iter().all(|c| (0..8).contains(c)) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // in 0..8
                let cell = cell.map(|c| c as u8);
                if !out.contains(&cell) {
                    out.push(cell);
                }
            }
            a = if a + 256 < a1 {
                a + 256
            } else if a + 1 < a1 {
                a1 - 1
            } else {
                a1
            };
        }
        out
    }

    /// The registry IDs this beam stands for.
    pub fn tags(&self) -> Vec<&'static str> {
        let mut t = vec![FRM_LINE, FRM_SECTION, self.kind.tag()];
        if self.class() != DirClass::Axis {
            t.push(FRM_OBLIQUE);
        }
        t.extend(
            self.ends
                .iter()
                .filter(|&&e| e == End::Stub)
                .map(|_| FRM_STUB),
        );
        t
    }
}

/// Beam existence per lattice point: `E_i / 4096` (spec §4W.1.4).
pub fn beam_chance(weave: u8, class: DirClass) -> i64 {
    let w = i64::from(weave);
    let weights = if weave < 16 {
        [255 - w, 0, 0, 0]
    } else {
        [255 - w, w / 2, w / 4, w / 8]
    };
    let counts = [3i64, 6, 4, 36];
    let total: i64 = weights.iter().sum();
    let k = match class {
        DirClass::Axis => 0,
        DirClass::FaceDiagonal => 1,
        DirClass::SpaceDiagonal => 2,
        DirClass::Skew => 3,
    };
    if total == 0 {
        return 0;
    }
    4096 * weights[k] / (counts[k] * total)
}

/// The greatest chance any weave can give a direction class: a cheap early-out before a
/// span's regime is read.
fn max_chance(class: DirClass) -> i64 {
    static TABLE: std::sync::OnceLock<[i64; 4]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut m = [0i64; 4];
        for w in 0..=255u8 {
            for (k, c) in [
                DirClass::Axis,
                DirClass::FaceDiagonal,
                DirClass::SpaceDiagonal,
                DirClass::Skew,
            ]
            .into_iter()
            .enumerate()
            {
                m[k] = m[k].max(beam_chance(w, c));
            }
        }
        m
    });
    t[match class {
        DirClass::Axis => 0,
        DirClass::FaceDiagonal => 1,
        DirClass::SpaceDiagonal => 2,
        DirClass::Skew => 3,
    }]
}

fn line_hash(seed: WorldSeed, dir: usize, p: i64, q: i64) -> u64 {
    hash(
        seed.0,
        Level::L1,
        I64Vec3::new(p, q, dir as i64),
        salt::LINE_BEAM,
    )
}

/// The draws of one span of a line: `(a_from, a_to, ends)` in world metres, or `None` when
/// the span is absent. `weave` is the weave of the span's home regime.
struct SpanDraw {
    a_from: i64,
    a_to: i64,
    ends: [End; 2],
    width: i64,
    section: Section,
    kind: BeamKind,
    detail: u64,
}

fn span_of(
    seed: WorldSeed,
    info: &DirInfo,
    dir: usize,
    line: u64,
    e: i64,
    span: i64,
    weave: u8,
) -> Option<SpanDraw> {
    if e >= beam_chance(weave, dir_class(dir)) {
        return None;
    }
    let _ = seed;
    let mut rng = Rng::new(absorb(line, span as u64));
    if rng.below(4) >= 3 {
        return None;
    }
    let base = span * SPAN;
    let full = rng.below(2) == 0;
    let (a_from, a_to, ends) = if full {
        (base, base + SPAN, [End::Continue, End::Continue])
    } else {
        let a0 = i64::from(rng.range_i32(0, 8));
        let a1 = i64::from(rng.range_i32(a0 as i32 + 1, 9));
        let end = |r: &mut Rng| {
            if r.below(2) == 0 {
                End::Knot
            } else {
                End::Stub
            }
        };
        let e0 = end(&mut rng);
        let e1 = end(&mut rng);
        (base + a0 * L1_M, base + a1 * L1_M, [e0, e1])
    };
    let width = 32 + 16 * i64::from(rng.range_i32(0, 24));
    let section = [Section::Square, Section::Flattened, Section::Round]
        [rng.pick_weighted(&[50, 20, 30]).unwrap_or(0)];
    // Stair beams climb in a vertical plane of X or Z at slope 1/2 (their stairs have 64
    // treads over 32 m); a fallen beam needs a vertical component.
    let d = DIRS[dir];
    let vertical_plane = d[1] != 0 && info.a != Axis::Y && (d[0] == 0 || d[2] == 0);
    let half_slope = vertical_plane
        && 2 * i64::from(d[1]).abs() == i64::from(d[0]).abs().max(i64::from(d[2]).abs());
    let weights: [u32; 8] = [
        30,
        15,
        20,
        10,
        8,
        7,
        if half_slope { 60 } else { 0 },
        if d[1] != 0 && !half_slope { 5 } else { 0 },
    ];
    let kinds = [
        BeamKind::Girder,
        BeamKind::Inhabited,
        BeamKind::Truss,
        BeamKind::Conduit,
        BeamKind::Rail,
        BeamKind::Stepped,
        BeamKind::Stair,
        BeamKind::Fallen,
    ];
    let kind = kinds[rng.pick_weighted(&weights).unwrap_or(0)];
    let detail = rng.next_u64();
    Some(SpanDraw {
        a_from,
        a_to,
        ends,
        width,
        section,
        kind,
        detail,
    })
}

/// The L0 cell holding the middle of a span of a line, whose regime owns the span.
pub(crate) fn span_home(info: &DirInfo, u0: i64, v0: i64, span: i64) -> I64Vec3 {
    let a_mid = span * SPAN + SPAN / 2;
    let u = u0 + info.du * a_mid / info.da;
    let v = v0 + info.dv * a_mid / info.da;
    let mut c = I64Vec3::ZERO;
    set(&mut c, info.a, a_mid.div_euclid(L0_M));
    set(&mut c, info.u, u.div_euclid(L0_M));
    set(&mut c, info.v, v.div_euclid(L0_M));
    c
}

/// Every beam that crosses L1 cell `l1`, in the order direction, `p`, `q` (spec §4W.1.2).
pub fn beams_of_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<Beam> {
    let origin = l1 * L1_M;
    let mut out = Vec::new();
    for dir in 0..DIRS.len() {
        let info = dir_info(dir);
        let class = dir_class(dir);
        let emax = max_chance(class);
        if emax == 0 {
            continue;
        }
        let (a0, a1) = (comp(origin, info.a), comp(origin, info.a) + L1_M);
        let span = a0.div_euclid(SPAN);
        let (ulo, uhi) = range_for(origin, info.u, info.du, info.da, a0, a1);
        let (vlo, vhi) = range_for(origin, info.v, info.dv, info.da, a0, a1);
        for p in (ulo - PITCH + 1).div_euclid(PITCH)..=uhi.div_euclid(PITCH) {
            for q in (vlo - PITCH + 1).div_euclid(PITCH)..=vhi.div_euclid(PITCH) {
                let line = line_hash(seed, dir, p, q);
                let mut rng = Rng::new(line);
                let e = rng.below(4096) as i64;
                if e >= emax {
                    continue;
                }
                let u0 = p * PITCH + 16 * rng.below((PITCH / 16) as u64) as i64;
                let v0 = q * PITCH + 16 * rng.below((PITCH / 16) as u64) as i64;
                let home = span_home(&info, u0, v0, span);
                let weave = regime(seed, home).web.weave;
                let Some(s) = span_of(seed, &info, dir, line, e, span, weave) else {
                    continue;
                };
                let mut beam = Beam {
                    dir,
                    p,
                    q,
                    u0,
                    v0,
                    span,
                    a_from: s.a_from,
                    a_to: s.a_to,
                    width: s.width,
                    section: s.section,
                    kind: s.kind,
                    ends: s.ends,
                    home,
                    detail: s.detail,
                };
                if s.a_to <= a0 || s.a_from >= a1 {
                    continue;
                }
                if !crosses(&beam, origin, &info) {
                    continue;
                }
                // A full span continues into the next only if the next exists.
                for (k, boundary, next) in [(0, s.a_from, span - 1), (1, s.a_to, span + 1)] {
                    if beam.ends[k] == End::Continue
                        && !continues(seed, &info, dir, line, e, u0, v0, next, k == 1)
                    {
                        beam.ends[k] = End::Stub;
                    }
                    let _ = boundary;
                }
                out.push(beam);
            }
        }
    }
    out
}

/// The range of intercepts `o` that can put a line with slope `d / da` inside the cell's
/// footprint on `axis`, with a pad for the section.
fn range_for(origin: I64Vec3, axis: Axis, d: i64, da: i64, a0: i64, a1: i64) -> (i64, i64) {
    let (x, y) = (d * a0 / da, d * a1 / da);
    let (lo, hi) = (x.min(y) - 1, x.max(y) + 1);
    let c0 = comp(origin, axis);
    let pad = 300;
    (c0 - pad - hi, c0 + L1_M + pad - lo)
}

/// Whether the beam's axis comes within its section of the cell, in a conservative test.
fn crosses(b: &Beam, origin: I64Vec3, info: &DirInfo) -> bool {
    let (c0, c1) = (comp(origin, info.a), comp(origin, info.a) + L1_M);
    let (a0, a1) = (c0.max(b.a_from), c1.min(b.a_to));
    if a0 >= a1 {
        return false;
    }
    let pad = b.width * 2 + 16;
    let inside = |a: i64| {
        let p = b.point_q(a);
        (0..3).all(|k| {
            let o = match k {
                0 => origin.x,
                1 => origin.y,
                _ => origin.z,
            };
            let m = p[k] / Q;
            m >= o - pad && m < o + L1_M + pad
        })
    };
    // The axis is a straight segment, so it enters the padded box if it does at one of a few
    // samples or crosses it between samples; the pad covers the gap.
    let steps = 64;
    (0..=steps).any(|k| inside(a0 + (a1 - a0) * k / steps))
}

#[allow(clippy::too_many_arguments)]
fn continues(
    seed: WorldSeed,
    info: &DirInfo,
    dir: usize,
    line: u64,
    e: i64,
    u0: i64,
    v0: i64,
    next: i64,
    forward: bool,
) -> bool {
    let home = span_home(info, u0, v0, next);
    let weave = regime(seed, home).web.weave;
    match span_of(seed, info, dir, line, e, next, weave) {
        None => false,
        Some(s) => {
            let base = next * SPAN;
            if forward {
                s.a_from == base
            } else {
                s.a_to == base + SPAN
            }
        }
    }
}

/// FRM-14: a beam whose axis passes through a Between is a mega-bridge. Nothing extra is
/// generated; the census counts them.
pub fn is_mega_bridge(seed: WorldSeed, beam: &Beam) -> bool {
    use crate::stack::{Stratum, stack};
    let mut a = beam.a_from;
    while a < beam.a_to {
        let p = beam.point_q(a);
        let w = I64Vec3::new(p[0] / Q, p[1] / Q, p[2] / Q);
        let l0 = w.div_euclid(I64Vec3::splat(L0_M));
        let st = stack(seed, l0);
        let geo = regime(seed, l0).web.stack;
        let u = crate::stack::u_units(geo, w - l0 * L0_M, st.well);
        if matches!(st.segment_at(u).stratum, Stratum::Between(_)) {
            return true;
        }
        a += 512;
    }
    false
}

/// The knot of an L1 cell (FRM-13): a hub in one of the inner L2 cells, snapped to the beams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Knot {
    /// The L2 cell inside the L1 cell, each coordinate in `1..7`.
    pub l2: [i64; 3],
    /// Hub family: junction hall, lift nexus, machine, gravity furnace, sanctuary site.
    pub hub: u8,
    /// Far signature: silhouette, light colour and rhythm come from these bits.
    pub signature: u64,
    /// Whether the knot snapped to a beam.
    pub on_beam: bool,
}

pub const HUB_WEIGHTS: [u32; 5] = [40, 20, 25, 5, 10];

/// The hub family of a gravity furnace (HUB-10).
pub const FURNACE_HUB: u8 = 3;

/// The knot of L1 cell `l1`, which may depend on its own beams (spec §4W.2.2).
pub fn knot(seed: WorldSeed, l1: I64Vec3, beams: &[Beam]) -> Knot {
    let h = hash(seed.0, Level::L1, l1, salt::KNOT);
    let mut rng = Rng::new(h);
    let drawn = [
        i64::from(rng.range_i32(1, 7)),
        i64::from(rng.range_i32(1, 7)),
        i64::from(rng.range_i32(1, 7)),
    ];
    let hub = rng.pick_weighted(&HUB_WEIGHTS).unwrap_or(0) as u8;
    let signature = rng.next_u64();
    let origin = l1 * L1_M;
    let l2_of = |p: [i64; 3]| -> [i64; 3] {
        [
            (p[0] / Q - origin.x).div_euclid(512),
            (p[1] / Q - origin.y).div_euclid(512),
            (p[2] / Q - origin.z).div_euclid(512),
        ]
    };
    let inner = |c: [i64; 3]| c.iter().all(|&x| (1..7).contains(&x));
    let cheb = |a: [i64; 3], b: [i64; 3]| (0..3).map(|k| (a[k] - b[k]).abs()).max().unwrap_or(0);
    // Candidates: (is not a knot end, distance, beam index, cell index, cell).
    let mut best: Option<(bool, i64, usize, i64, [i64; 3])> = None;
    for (bi, b) in beams.iter().enumerate() {
        let info = b.info();
        let (c0, c1) = (comp(origin, info.a), comp(origin, info.a) + L1_M);
        let (a0, a1) = (c0.max(b.a_from), c1.min(b.a_to));
        // A beam whose end is a knot in this cell wins outright.
        for (k, end_a) in [(0, b.a_from), (1, b.a_to)] {
            if b.ends[k] == End::Knot && (c0..c1).contains(&end_a) {
                let cell = l2_of(b.point_q(end_a.clamp(c0, c1 - 1)));
                let cell = [
                    cell[0].clamp(1, 6),
                    cell[1].clamp(1, 6),
                    cell[2].clamp(1, 6),
                ];
                let cand = (false, 0, bi, 0, cell);
                if best.is_none_or(|x| (x.0, x.1, x.2, x.3) > (cand.0, cand.1, cand.2, cand.3)) {
                    best = Some(cand);
                }
            }
        }
        let mut a = a0;
        while a < a1 {
            let cell = l2_of(b.point_q(a));
            if inner(cell) {
                let d = cheb(cell, drawn);
                if d <= 1 {
                    let idx = (cell[2] * 8 + cell[1]) * 8 + cell[0];
                    let cand = (true, d, bi, idx, cell);
                    if best.is_none_or(|x| (x.0, x.1, x.2, x.3) > (cand.0, cand.1, cand.2, cand.3))
                    {
                        best = Some(cand);
                    }
                }
            }
            a += 256;
        }
    }
    // A well's furnace is the knot of its cell (spec `gravity.md` §3.3): the hub is HUB-10.
    if let Some(c) = crate::gravity::well_centre_in_l1(seed, l1) {
        let cell = [0, 1, 2].map(|k| {
            (c[k] - [origin.x, origin.y, origin.z][k])
                .div_euclid(512)
                .clamp(1, 6)
        });
        return Knot {
            l2: cell,
            hub: FURNACE_HUB,
            signature,
            on_beam: false,
        };
    }
    match best {
        Some((_, _, _, _, cell)) => Knot {
            l2: cell,
            hub,
            signature,
            on_beam: true,
        },
        None => Knot {
            l2: drawn,
            hub,
            signature,
            on_beam: false,
        },
    }
}

/// Everything the Frame of L1 cell `l1` emits: beam solids and the knot's structure.
pub fn frame_content(seed: WorldSeed, l1: I64Vec3) -> Vec<WebPiece> {
    let beams = beams_of_cell(seed, l1);
    let mut e = Emitter {
        origin: l1 * L1_M,
        out: Vec::new(),
    };
    for b in &beams {
        e.beam(b);
    }
    let k = knot(seed, l1, &beams);
    e.knot(&k);
    e.out
}

struct Emitter {
    origin: I64Vec3,
    out: Vec<WebPiece>,
}

fn clip() -> QAabb {
    let e = (L1_M * Q) as i32;
    QAabb::new(QVec3::new(0, 0, 0), QVec3::new(e, e, e))
}

type Q3 = [i64; 3];

impl Emitter {
    fn push(
        &mut self,
        tag: &'static str,
        shape: Shape,
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        let b = shape.aabb();
        let c = clip();
        if b.max.x <= c.min.x
            || b.min.x >= c.max.x
            || b.max.y <= c.min.y
            || b.min.y >= c.max.y
            || b.max.z <= c.min.z
            || b.min.z >= c.max.z
        {
            return;
        }
        self.out.push(WebPiece {
            tag,
            prim: Primitive {
                shape,
                material,
                kind,
                flags,
            },
        });
    }

    /// A cell-local point in quanta from a world point in quanta.
    fn local(&self, p: Q3) -> QVec3 {
        QVec3::new(
            (p[0] - self.origin.x * Q) as i32,
            (p[1] - self.origin.y * Q) as i32,
            (p[2] - self.origin.z * Q) as i32,
        )
    }

    /// A box in a frame, with its local extents `lo..hi` in quanta.
    #[allow(clippy::too_many_arguments)]
    fn fbox(
        &mut self,
        tag: &'static str,
        frame: Frame,
        origin: QVec3,
        lo: [i32; 3],
        hi: [i32; 3],
        material: MaterialId,
        flags: PrimFlags,
    ) {
        if lo[0] >= hi[0] || lo[1] >= hi[1] || lo[2] >= hi[2] {
            return;
        }
        let aabb = QAabb::new(
            QVec3::new(lo[0], lo[1], lo[2]),
            QVec3::new(hi[0], hi[1], hi[2]),
        );
        let shape = Shape::oriented(frame, origin, LocalShape::Box { aabb }, clip());
        self.push(tag, shape, material, FeatureKind::TETHER, flags);
    }

    fn beam(&mut self, b: &Beam) {
        let info = b.info();
        let d = b.direction();
        let frame = Frame::dir(d);
        let axes = frame.axes();
        // The portion of the beam near the cell along its dominant axis, padded so the clip cuts it.
        let (c0, c1) = (comp(self.origin, info.a), comp(self.origin, info.a) + L1_M);
        let pad = b.width * 3 + 64;
        let a_lo = b.a_from.max(c0 - pad);
        let a_hi = b.a_to.min(c1 + pad);
        if a_hi <= a_lo {
            return;
        }
        let norm = {
            let v = d.map(f64::from);
            (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
        };
        // Local `x` of world coordinate `a`, from the beam's own start so that every cell that
        // sees the beam places its repeated parts (floors, bays, steps) at the same positions.
        let to_x = |a: i64| (((a - b.a_from) * Q) as f64 * norm / info.da as f64).round() as i32;
        let w = (b.width * Q) as i32;
        let h = match b.section {
            Section::Flattened => w / 2,
            _ => w,
        };
        let ctx = BeamCtx {
            frame,
            origin: self.local(b.point_q(b.a_from)),
            length: to_x(b.a_to),
            x0: to_x(a_lo),
            x1: to_x(a_hi),
            cyl_origin: self.local(b.point_q(a_lo)),
            w,
            h,
            axes,
            d,
            info,
        };
        match b.kind {
            BeamKind::Girder | BeamKind::Fallen => {
                self.body(b, &ctx, b.kind.tag(), PrimFlags::FAR_VISIBLE);
            }
            BeamKind::Inhabited => self.inhabited(b, &ctx),
            BeamKind::Truss => self.truss(b, &ctx),
            BeamKind::Conduit => self.conduit(b, &ctx),
            BeamKind::Rail => self.rail(b, &ctx),
            BeamKind::Stepped => self.stepped(b, &ctx),
            BeamKind::Stair => self.stair(b, &ctx),
        }
        self.ends(b, &ctx);
    }

    /// The solid body: a box, or a cylinder for a round section.
    fn body(&mut self, b: &Beam, c: &BeamCtx, tag: &'static str, flags: PrimFlags) {
        if b.section == Section::Round {
            let shape = beam_cylinder(c.d, c.cyl_origin, c.x1 - c.x0, c.w / 2, clip());
            self.push(tag, shape, MaterialId::CONCRETE, FeatureKind::TETHER, flags);
        } else {
            self.fbox(
                tag,
                c.frame,
                c.origin,
                [c.x0, -c.h / 2, -c.w / 2],
                [c.x1, c.h / 2, c.w / 2],
                MaterialId::CONCRETE,
                flags,
            );
        }
    }

    /// FRM-06: a tube with floors.
    fn inhabited(&mut self, b: &Beam, c: &BeamCtx) {
        let t = ((b.width / 16).max(8) * Q) as i32;
        let (w, h) = (c.w, c.h);
        let m = FRM_INHABITED;
        let f = PrimFlags::FAR_VISIBLE;
        let n = MaterialId::CONCRETE;
        let (x0, x1) = (c.x0, c.x1);
        let o = c.origin;
        self.fbox(
            m,
            c.frame,
            o,
            [x0, -h / 2, -w / 2],
            [x1, -h / 2 + t, w / 2],
            n,
            f,
        );
        self.fbox(
            m,
            c.frame,
            o,
            [x0, h / 2 - t, -w / 2],
            [x1, h / 2, w / 2],
            n,
            f,
        );
        self.fbox(
            m,
            c.frame,
            o,
            [x0, -h / 2 + t, -w / 2],
            [x1, h / 2 - t, -w / 2 + t],
            n,
            f,
        );
        self.fbox(
            m,
            c.frame,
            o,
            [x0, -h / 2 + t, w / 2 - t],
            [x1, h / 2 - t, w / 2],
            n,
            f,
        );
        // Floors every 6 m inside, 1 m thick.
        let storey = (6 * Q) as i32;
        let mut y = -h / 2 + t + storey;
        while y < h / 2 - t - storey / 2 {
            self.fbox(
                m,
                c.frame,
                o,
                [x0, y, -w / 2 + t],
                [x1, y + Q as i32, w / 2 - t],
                n,
                PrimFlags::NONE,
            );
            y += storey;
        }
    }

    /// FRM-07: four chords, frames and diagonals.
    fn truss(&mut self, b: &Beam, c: &BeamCtx) {
        let chord = ((b.width / 20).max(4) * Q) as i32;
        let (w, h) = (c.w, c.h);
        let n = MaterialId::METAL;
        let m = FRM_TRUSS;
        let f = PrimFlags::FAR_VISIBLE;
        for (sy, sz) in [(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
            let y0 = if sy < 0 { -h / 2 } else { h / 2 - chord };
            let z0 = if sz < 0 { -w / 2 } else { w / 2 - chord };
            self.fbox(
                m,
                c.frame,
                c.origin,
                [c.x0, y0, z0],
                [c.x1, y0 + chord, z0 + chord],
                n,
                f,
            );
        }
        let pitch = (((b.width / 2).max(32)) * Q) as i32;
        let thin = (Q as i32).max(chord / 3);
        let mut x = c.x0 - c.x0.rem_euclid(pitch);
        while x < c.x1 {
            // A frame across the section.
            self.fbox(
                m,
                c.frame,
                c.origin,
                [x, -h / 2, -w / 2],
                [x + thin, -h / 2 + thin, w / 2],
                n,
                PrimFlags::NONE,
            );
            self.fbox(
                m,
                c.frame,
                c.origin,
                [x, h / 2 - thin, -w / 2],
                [x + thin, h / 2, w / 2],
                n,
                PrimFlags::NONE,
            );
            // A diagonal on each side face, from the chord of one bay to the next; the
            // direction alternates with the bay's position along the beam.
            let flip = (x / pitch) % 2 != 0;
            for side in [-1i32, 1] {
                self.diagonal(c, x, pitch, h, w, side, flip, thin);
            }
            x += pitch;
        }
    }

    /// A brace from corner to corner of a bay on one side face. Its frame is the table direction
    /// closest to the brace, so it is a slightly rotated box and not an exact one.
    #[allow(clippy::too_many_arguments)]
    fn diagonal(
        &mut self,
        c: &BeamCtx,
        x: i32,
        pitch: i32,
        h: i32,
        w: i32,
        side: i32,
        flip: bool,
        thin: i32,
    ) {
        let [ex, ey, ez] = c.axes;
        let (dy0, dy1) = if flip {
            (h / 2, -h / 2)
        } else {
            (-h / 2, h / 2)
        };
        let a = ex * f64::from(x) + ey * f64::from(dy0) + ez * f64::from(side * (w / 2 - thin / 2));
        let b = ex * f64::from(x + pitch)
            + ey * f64::from(dy1)
            + ez * f64::from(side * (w / 2 - thin / 2));
        let v = b - a;
        let len = v.length();
        if len < 1.0 {
            return;
        }
        let m = v.abs().max_element();
        let dir = [
            (v.x / m * 100.0).round() as i8,
            (v.y / m * 100.0).round() as i8,
            (v.z / m * 100.0).round() as i8,
        ];
        let p = [
            f64::from(c.origin.x) + a.x,
            f64::from(c.origin.y) + a.y,
            f64::from(c.origin.z) + a.z,
        ];
        let origin = QVec3::new(
            p[0].round() as i32,
            p[1].round() as i32,
            p[2].round() as i32,
        );
        let half = thin / 2;
        self.fbox(
            FRM_TRUSS,
            Frame::dir(dir),
            origin,
            [0, -half, -half],
            [len.round() as i32, half, half],
            MaterialId::METAL,
            PrimFlags::NONE,
        );
    }

    /// FRM-08: a bundle of giant pipes.
    fn conduit(&mut self, b: &Beam, c: &BeamCtx) {
        let r = ((b.width / 6).max(4) * Q) as i32;
        let [_, ey, ez] = c.axes;
        let rows: [(f64, f64); 7] = [
            (0.0, 0.0),
            (2.0, 0.0),
            (-2.0, 0.0),
            (1.0, 1.75),
            (-1.0, 1.75),
            (1.0, -1.75),
            (-1.0, -1.75),
        ];
        for (oy, oz) in rows {
            let off = ey * (oy * f64::from(r)) + ez * (oz * f64::from(r));
            let start = QVec3::new(
                c.cyl_origin.x + off.x.round() as i32,
                c.cyl_origin.y + off.y.round() as i32,
                c.cyl_origin.z + off.z.round() as i32,
            );
            let shape = beam_cylinder(c.d, start, c.x1 - c.x0, r, clip());
            self.push(
                FRM_CONDUIT,
                shape,
                MaterialId::PIPE,
                FeatureKind::PIPE,
                PrimFlags::FAR_VISIBLE,
            );
        }
    }

    /// FRM-09: a girder with a rail and a tram on top.
    fn rail(&mut self, b: &Beam, c: &BeamCtx) {
        let (w, h) = (c.w, c.h);
        let q = Q as i32;
        self.fbox(
            FRM_RAIL,
            c.frame,
            c.origin,
            [c.x0, -h / 2, -w / 2],
            [c.x1, h / 2, w / 2],
            MaterialId::CONCRETE,
            PrimFlags::FAR_VISIBLE,
        );
        let top = h / 2;
        self.fbox(
            FRM_RAIL,
            c.frame,
            c.origin,
            [c.x0, top, -3 * q],
            [c.x1, top + 2 * q, 3 * q],
            MaterialId::METAL,
            PrimFlags::NONE,
        );
        // A tram every 600 m along the beam, cars 40 m apart, positions drawn from the beam.
        let mut rng = Rng::new(b.detail);
        let cars = 3 + rng.below(6) as i32;
        let lead = (rng.below(15) as i32) * 40 * q;
        let train = 600 * q;
        let mut t0 = c.x0 - c.x0.rem_euclid(train);
        while t0 < c.x1 {
            for k in 0..cars {
                let x = t0 + lead + k * 40 * q;
                self.fbox(
                    FRM_RAIL,
                    c.frame,
                    c.origin,
                    [x, top + 2 * q, -4 * q],
                    [x + 30 * q, top + 10 * q, 4 * q],
                    MaterialId::METAL,
                    PrimFlags::NONE,
                );
                self.fbox(
                    FRM_RAIL,
                    c.frame,
                    c.origin,
                    [x + q, top + 5 * q, -5 * q],
                    [x + 29 * q, top + 8 * q, 5 * q],
                    MaterialId::TRAIN_LIGHT,
                    PrimFlags::NO_COLLIDE,
                );
            }
            t0 += train;
        }
    }

    /// FRM-10: boxes drifting up and down along the beam.
    fn stepped(&mut self, b: &Beam, c: &BeamCtx) {
        let seg = ((b.width * 2).max(64) * Q) as i32;
        let step = (b.width / 4 * Q) as i32;
        let wave = [0, 1, 2, 3, 4, 3, 2, 1];
        for k in (c.x0 / seg)..=(c.x1 / seg) {
            let oy = (wave[k.rem_euclid(8) as usize] - 2) * step;
            let (a, e) = (k * seg, ((k + 1) * seg + 16).min(c.length));
            if a >= c.length {
                break;
            }
            self.fbox(
                FRM_STEPPED,
                c.frame,
                c.origin,
                [a.max(0), -c.h / 2 + oy, -c.w / 2],
                [e, c.h / 2 + oy, c.w / 2],
                MaterialId::CONCRETE,
                PrimFlags::FAR_VISIBLE,
            );
        }
    }

    /// FRM-11: a staircase of blocks climbing at slope 1/2 in a vertical plane of X or Z. Each
    /// 32 m block is a slab with a 64-tread stair run on top (16 m of rise), so the top of the
    /// beam is a real flight of stairs.
    fn stair(&mut self, b: &Beam, c: &BeamCtx) {
        let info = c.info;
        let run_axis = info.a;
        let dy = i64::from(c.d[1]);
        // The stair rises towards +a when the beam climbs along it, towards −a otherwise.
        let rise_dir = match (run_axis, dy > 0) {
            (Axis::X, true) => Dir4::PosX,
            (Axis::X, false) => Dir4::NegX,
            (_, true) => Dir4::PosZ,
            (_, false) => Dir4::NegZ,
        };
        let block = 32i64;
        let t = (b.width / 2).max(8);
        let (c0, c1) = (comp(self.origin, info.a), comp(self.origin, info.a) + L1_M);
        let a_from = b.a_from.max(c0 - block).div_euclid(block) * block;
        let a_to = b.a_to.min(c1 + block);
        let mut a = a_from;
        while a < a_to {
            let (p0, p1) = (b.point_q(a), b.point_q(a + block));
            let y_min = p0[1].min(p1[1]) / Q;
            let (cross_axis, cross_c) = if run_axis == Axis::X {
                (Axis::Z, p0[2] / Q)
            } else {
                (Axis::X, p0[0] / Q)
            };
            let mut lo = I64Vec3::ZERO;
            let mut hi = I64Vec3::ZERO;
            set(&mut lo, run_axis, a);
            set(&mut hi, run_axis, a + block);
            set(&mut lo, cross_axis, cross_c - b.width / 2);
            set(&mut hi, cross_axis, cross_c + b.width / 2);
            set(&mut lo, Axis::Y, y_min - t);
            set(&mut hi, Axis::Y, y_min);
            self.world_box(
                FRM_STAIR,
                lo,
                hi,
                MaterialId::CONCRETE,
                PrimFlags::FAR_VISIBLE,
            );
            set(&mut lo, Axis::Y, y_min);
            set(&mut hi, Axis::Y, y_min + 16);
            self.world_stair(lo, hi, rise_dir);
            a += block;
        }
    }

    fn world_box(
        &mut self,
        tag: &'static str,
        lo: I64Vec3,
        hi: I64Vec3,
        material: MaterialId,
        flags: PrimFlags,
    ) {
        let o = self.origin;
        let c = |v: i64, axis: Axis| ((v - comp(o, axis)) * Q).clamp(0, L1_M * Q) as i32;
        let (min, max) = (
            QVec3::new(c(lo.x, Axis::X), c(lo.y, Axis::Y), c(lo.z, Axis::Z)),
            QVec3::new(c(hi.x, Axis::X), c(hi.y, Axis::Y), c(hi.z, Axis::Z)),
        );
        if min.x >= max.x || min.y >= max.y || min.z >= max.z {
            return;
        }
        self.push(
            tag,
            Shape::Box {
                aabb: QAabb::new(min, max),
            },
            material,
            FeatureKind::TETHER,
            flags,
        );
    }

    fn world_stair(&mut self, lo: I64Vec3, hi: I64Vec3, rise: Dir4) {
        let o = self.origin;
        let q = |v: I64Vec3| {
            QVec3::new(
                ((v.x - o.x) * Q) as i32,
                ((v.y - o.y) * Q) as i32,
                ((v.z - o.z) * Q) as i32,
            )
        };
        let aabb = QAabb::new(q(lo), q(hi));
        let shape = Shape::oriented(
            Frame::Id,
            QVec3::new(0, 0, 0),
            LocalShape::StairRun {
                aabb,
                rise,
                steps: 64,
            },
            clip(),
        );
        self.push(
            FRM_STAIR,
            shape,
            MaterialId::CONCRETE,
            FeatureKind::RAMP,
            PrimFlags::NONE,
        );
    }

    /// Knot ends get a junction block; stub ends get broken shards (FRM-04, FRM-13).
    fn ends(&mut self, b: &Beam, c: &BeamCtx) {
        for (k, at_start) in [(0usize, true), (1, false)] {
            let a = if at_start { b.a_from } else { b.a_to };
            let (c0, c1) = (
                comp(self.origin, c.info.a),
                comp(self.origin, c.info.a) + L1_M,
            );
            if a < c0 - b.width * 2 || a > c1 + b.width * 2 {
                continue;
            }
            let p = b.point_q(a);
            let origin = self.local(p);
            let frame = c.frame;
            let (w, h) = (c.w, c.h);
            match b.ends[k] {
                End::Continue => {}
                End::Knot => {
                    // A junction block a little wider than the beam, across the end.
                    let s = w * 3 / 2;
                    self.fbox(
                        FRM_KNOT,
                        frame,
                        origin,
                        [-s / 2, -s / 2, -s / 2],
                        [s / 2, s / 2, s / 2],
                        MaterialId::METAL,
                        PrimFlags::FAR_VISIBLE,
                    );
                }
                End::Stub => {
                    // Shards: the end is cut off, and two fragments hang from it.
                    let dir = if at_start { -1 } else { 1 };
                    let mut rng = Rng::new(absorb(b.detail, k as u64));
                    for _ in 0..3 {
                        let len = (8 + rng.below(24) as i32) * Q as i32;
                        let oy = (rng.below(h.max(2) as u64) as i32) - h / 2;
                        let oz = (rng.below(w.max(2) as u64) as i32) - w / 2;
                        let sz = (4 + rng.below(12) as i32) * Q as i32;
                        let (x0, x1) = if dir > 0 { (0, len) } else { (-len, 0) };
                        self.fbox(
                            FRM_STUB,
                            frame,
                            origin,
                            [x0, oy - sz / 2, oz - sz / 2],
                            [x1, oy + sz / 2, oz + sz / 2],
                            MaterialId::CONCRETE,
                            PrimFlags::NONE,
                        );
                    }
                }
            }
        }
    }

    /// The knot structure: a junction block and a beacon in its L2 cell.
    fn knot(&mut self, k: &Knot) {
        let centre = I64Vec3::new(k.l2[0], k.l2[1], k.l2[2]) * 512 + I64Vec3::splat(256);
        let s = 48 + 16 * i64::from((k.signature & 3) as u8);
        let lo = self.origin + centre - I64Vec3::splat(s);
        let hi = self.origin + centre + I64Vec3::splat(s);
        self.world_box(FRM_KNOT, lo, hi, MaterialId::METAL, PrimFlags::FAR_VISIBLE);
        let top = hi.y;
        self.world_box(
            FRM_KNOT,
            I64Vec3::new(lo.x + s - 4, top, lo.z + s - 4),
            I64Vec3::new(lo.x + s + 4, top + 24, lo.z + s + 4),
            MaterialId::BEACON,
            PrimFlags::NO_COLLIDE,
        );
    }
}

struct BeamCtx {
    frame: Frame,
    /// The beam's own start (local `x = 0`), the same for every cell that sees it.
    origin: QVec3,
    /// Total length from the start to the end of the beam.
    length: i32,
    /// The part of the beam near the cell: local `x` from `x0` to `x1`.
    x0: i32,
    x1: i32,
    /// The axis point at `x0`, for cylinders.
    cyl_origin: QVec3,
    w: i32,
    h: i32,
    axes: [DVec3; 3],
    d: [i8; 3],
    info: DirInfo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn cells() -> impl Iterator<Item = I64Vec3> {
        (-3..3).flat_map(|x| (0..8).flat_map(move |y| (-3..3).map(move |z| I64Vec3::new(x, y, z))))
    }

    #[test]
    fn chance_follows_the_weave() {
        // At weave 0 only axes carry beams, 1365 each (spec §4W.1.4).
        assert_eq!(beam_chance(0, DirClass::Axis), 1365);
        assert_eq!(beam_chance(0, DirClass::FaceDiagonal), 0);
        assert_eq!(beam_chance(15, DirClass::Skew), 0);
        // At weave 255: weights 0, 127, 63, 31.
        assert_eq!(beam_chance(255, DirClass::Axis), 0);
        assert_eq!(
            beam_chance(255, DirClass::FaceDiagonal),
            4096 * 127 / (6 * 221)
        );
        for w in 0..=255u8 {
            let total: i64 = [
                DirClass::Axis,
                DirClass::FaceDiagonal,
                DirClass::SpaceDiagonal,
                DirClass::Skew,
            ]
            .into_iter()
            .zip([3i64, 6, 4, 36])
            .map(|(c, n)| beam_chance(w, c) * n)
            .sum();
            assert!(total <= 4096, "weave {w}: {total}");
        }
    }

    /// Beams are not rare, not a flood, and every kind, shape, class and end occurs.
    #[test]
    fn beams_occur_in_every_form() {
        let seed = WorldSeed(1);
        let mut kinds = std::collections::HashSet::new();
        let mut shapes = std::collections::HashSet::new();
        let mut classes = std::collections::HashSet::new();
        let (mut stubs, mut knots, mut total, mut cells_n) = (0, 0, 0, 0);
        for c in cells() {
            let beams = beams_of_cell(seed, c);
            cells_n += 1;
            total += beams.len();
            assert!(beams.len() < 60, "{} beams in {c:?}", beams.len());
            for b in &beams {
                kinds.insert(b.kind);
                shapes.insert(b.section);
                classes.insert(b.class() as u8);
                stubs += b.ends.iter().filter(|&&e| e == End::Stub).count();
                knots += b.ends.iter().filter(|&&e| e == End::Knot).count();
                assert!((32..=400).contains(&b.width) && b.width % 16 == 0);
            }
        }
        // Stair beams need a skew direction of slope 1/2, the rarest class: look further.
        let mut far = (100..).map(|k| I64Vec3::new(k, 3, -k)).take(4000);
        while !kinds.contains(&BeamKind::Stair) {
            let Some(c) = far.next() else { break };
            kinds.extend(beams_of_cell(seed, c).iter().map(|b| b.kind));
        }
        assert_eq!(kinds.len(), 8, "{kinds:?}");
        assert_eq!(shapes.len(), 3);
        assert!(classes.len() >= 3, "{classes:?}");
        assert!(stubs > 0 && knots > 0);
        let mean = total as f64 / f64::from(cells_n);
        assert!((1.0..14.0).contains(&mean), "mean {mean} beams per cell");
    }

    /// FRM-01: a beam is derived identically by every cell its axis passes through.
    #[test]
    fn beams_agree_in_every_cell_they_cross() {
        let seed = WorldSeed(3);
        let (mut checked, mut oblique) = (0, 0);
        for c in cells().step_by(5) {
            for b in beams_of_cell(seed, c) {
                oblique += usize::from(b.class() != DirClass::Axis);
                let mut a = b.a_from;
                while a < b.a_to {
                    let p = b.point_q(a);
                    let w = I64Vec3::new(p[0] / Q, p[1] / Q, p[2] / Q);
                    let n = w.div_euclid(I64Vec3::splat(L1_M));
                    // Only cells near the region keep the test fast.
                    if (n - c).abs().max_element() <= 2 {
                        assert!(
                            beams_of_cell(seed, n).contains(&b),
                            "beam of {c:?} is missing from {n:?} at a = {a}"
                        );
                        checked += 1;
                    }
                    a += 512;
                }
            }
        }
        assert!(
            checked > 200 && oblique > 0,
            "{checked} checks, {oblique} oblique"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        /// FRM-01 across random cells and seeds: the cell on the other side of a face sees every
        /// beam that crosses that face.
        #[test]
        fn line_keys_agree_across_faces(seed in any::<u64>(), x in -40i64..40, y in -3i64..12, z in -40i64..40, axis in 0usize..3) {
            let seed = WorldSeed(seed);
            let c = I64Vec3::new(x, y, z);
            let mut step = I64Vec3::ZERO;
            step[axis] = 1;
            let n = c + step;
            let (ours, theirs) = (beams_of_cell(seed, c), beams_of_cell(seed, n));
            let face = comp(n * L1_M, [Axis::X, Axis::Y, Axis::Z][axis]);
            for b in &ours {
                // Sample the axis just inside the neighbour; if it is there, the neighbour has the beam.
                let mut a = b.a_from;
                while a < b.a_to {
                    let p = b.point_q(a);
                    let w = I64Vec3::new(p[0] / Q, p[1] / Q, p[2] / Q);
                    if w.div_euclid(I64Vec3::splat(L1_M)) == n && comp(w, [Axis::X, Axis::Y, Axis::Z][axis]) >= face {
                        prop_assert!(theirs.contains(b), "beam {b:?} lost across a face");
                        break;
                    }
                    a += 256;
                }
            }
            for b in &theirs {
                let mut a = b.a_from;
                while a < b.a_to {
                    let p = b.point_q(a);
                    let w = I64Vec3::new(p[0] / Q, p[1] / Q, p[2] / Q);
                    if w.div_euclid(I64Vec3::splat(L1_M)) == c {
                        prop_assert!(ours.contains(b), "beam {b:?} lost across a face");
                        break;
                    }
                    a += 256;
                }
            }
        }
    }

    /// Pieces stay in their cell, are deterministic, and the knot is placed in the inner box.
    #[test]
    fn pieces_and_knots_are_well_formed() {
        let seed = WorldSeed(2);
        let e = (L1_M * Q) as i32;
        let mut on_beam = 0;
        for c in cells().step_by(3) {
            let a = frame_content(seed, c);
            assert_eq!(a, frame_content(seed, c));
            for p in &a {
                let b = p.prim.aabb();
                assert!(
                    b.min.x >= 0
                        && b.min.y >= 0
                        && b.min.z >= 0
                        && b.max.x <= e
                        && b.max.y <= e
                        && b.max.z <= e,
                    "{c:?} {} {b:?}",
                    p.tag
                );
            }
            let beams = beams_of_cell(seed, c);
            let k = knot(seed, c, &beams);
            assert!(k.l2.iter().all(|&v| (1..7).contains(&v)), "{k:?}");
            assert!(
                a.iter().any(|p| p.tag == FRM_KNOT),
                "no knot structure in {c:?}"
            );
            on_beam += usize::from(k.on_beam);
            assert_eq!(k, knot(seed, c, &beams));
        }
        assert!(on_beam > 0, "no knot ever snapped to a beam");
    }

    /// Every beam kind emits its own tag, and stair beams carry real stair runs.
    #[test]
    fn every_kind_emits_its_pieces() {
        let seed = WorldSeed(1);
        let mut seen = std::collections::HashSet::new();
        let mut stairs = 0;
        let cells = cells().chain((100..2600).map(|k| I64Vec3::new(k, 3, -k)));
        for c in cells {
            for p in frame_content(seed, c) {
                seen.insert(p.tag);
                if let Shape::Oriented(o) = p.prim.shape {
                    stairs += usize::from(matches!(o.local, LocalShape::StairRun { .. }));
                } else if matches!(p.prim.shape, Shape::StairRun { .. }) {
                    stairs += 1;
                }
            }
        }
        for tag in [
            FRM_GIRDER,
            FRM_INHABITED,
            FRM_TRUSS,
            FRM_CONDUIT,
            FRM_RAIL,
            FRM_STEPPED,
            FRM_STAIR,
            FRM_FALLEN,
            FRM_STUB,
            FRM_KNOT,
        ] {
            assert!(seen.contains(tag), "{tag} missing: {seen:?}");
        }
        assert!(stairs > 0);
    }

    /// FRM-14: some beams pass through a Between and are mega-bridges, some do not.
    #[test]
    fn mega_bridges_exist() {
        let seed = WorldSeed(1);
        let (mut bridges, mut others) = (0, 0);
        for c in cells().step_by(2) {
            for b in beams_of_cell(seed, c) {
                if is_mega_bridge(seed, &b) {
                    bridges += 1;
                } else {
                    others += 1;
                }
            }
        }
        assert!(
            bridges > 10 && others > 10,
            "{bridges} bridges, {others} others"
        );
    }

    /// The solids on both sides of each shared face agree: a beam is one piece of matter
    /// across the faces it crosses (the clip cuts it; it does not change it).
    #[test]
    fn solids_agree_across_faces() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        let mut rng = crate::rng::Rng::new(17);
        let (mut total, mut solid, mut bad) = (0, 0, 0);
        for c in cells().step_by(7) {
            for axis in Axis::ALL {
                let mut step = I64Vec3::ZERO;
                set(&mut step, axis, 1);
                let (a, b) = (frame_content(seed, c), frame_content(seed, c + step));
                // Beams that end on this face (spans end on cell faces) legitimately stop here.
                let face = comp((c + step) * L1_M, axis);
                let ending: Vec<([i64; 3], i64)> = beams_of_cell(seed, c)
                    .into_iter()
                    .chain(beams_of_cell(seed, c + step))
                    .filter(|b| b.info().a == axis && (b.a_to == face || b.a_from == face))
                    .map(|b| (b.point_q(face), b.width * 2))
                    .collect();
                // Only the beam pieces, not the knot blocks, which are cell-local.
                let beam = |p: &&WebPiece| p.tag != FRM_KNOT;
                let eps = 1.0 / 32.0;
                for _ in 0..1500 {
                    let (u, v) = (rng.next_f32() * 4096.0, rng.next_f32() * 4096.0);
                    let at = |along: f32| match axis {
                        Axis::X => Vec3::new(along, u, v),
                        Axis::Y => Vec3::new(u, along, v),
                        Axis::Z => Vec3::new(u, v, along),
                    };
                    let near_end = ending.iter().any(|(p, r)| {
                        let w = at(0.0);
                        let o = (c * L1_M).as_vec3();
                        let (pu, pv) = match axis {
                            Axis::X => (p[1], p[2]),
                            Axis::Y => (p[0], p[2]),
                            Axis::Z => (p[0], p[1]),
                        };
                        let (cu, cv) = match axis {
                            Axis::X => (o.y + w.y, o.z + w.z),
                            Axis::Y => (o.x + w.x, o.z + w.z),
                            Axis::Z => (o.x + w.x, o.y + w.y),
                        };
                        (pu as f32 / Q as f32 - cu).abs() < *r as f32
                            && (pv as f32 / Q as f32 - cv).abs() < *r as f32
                    });
                    if near_end {
                        continue;
                    }
                    let inside = a
                        .iter()
                        .filter(beam)
                        .any(|p| p.prim.shape.contains(at(4096.0 - eps)));
                    let beyond = b
                        .iter()
                        .filter(beam)
                        .any(|p| p.prim.shape.contains(at(eps)));
                    total += 1;
                    solid += usize::from(inside || beyond);
                    bad += usize::from(inside != beyond);
                }
            }
        }
        assert!(solid > 100, "only {solid} solid samples of {total}");
        assert!(
            bad * 50 <= solid,
            "{bad} disagreements among {solid} solid samples"
        );
    }
}
