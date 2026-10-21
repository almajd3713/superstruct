//! Railings on the open edges of the Web's walkable pieces (PW.3.22, spec `levels.md` §5W.31).
//!
//! The edges of decks and landings and the sides of stair flights that look onto a drop of
//! more than the fall threshold are guarded with `chance(0.85)` by runs, or left unguarded on
//! purpose (§5.4). A rail is a box in the frame of the piece it stands on, 1.125 m high and
//! 0.125 m thick, kind `Railing`. Where a wall, a rail or another deck already stands at the
//! edge nothing is added.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::manual_midpoint,
    clippy::collapsible_if
)]

use std::collections::BTreeMap;

use glam::{DVec3, I64Vec3, Vec3};

use crate::carve::as_box;
use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::orient::{Frame, LocalShape};
use crate::plumb;
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
const CELL: Q = 512 * M;
/// Railing height (1.1 m rounded up to quanta) and thickness.
const HEIGHT: Q = 18;
const THICKNESS: Q = 2;
/// How far a flight's rail stands inside the side of the flight.
const INSET: Q = 1;
/// The fall threshold, 15 m (`traversal.md` §3), and the metres between two probes down.
const FALL: i32 = 15;
const GUARDED: u64 = 85;

/// The grid of buckets, 16 m on a side, from 64 m before the cell to 64 m past it.
const BUCKET: Q = 16 * M;
const GRID_LO: Q = -4 * BUCKET;
const GRID_N: i32 = (CELL / BUCKET) + 8;
/// A piece in more buckets than this is kept in a short list that every query reads.
const BIG: i64 = 400;

/// A box piece as dot products: its origin and axes in metres, and its local range.
struct Boxed {
    o: [f32; 3],
    ax: [[f32; 3]; 3],
    mn: [f32; 3],
    mx: [f32; 3],
}

impl Boxed {
    fn new(frame: Frame, origin: QVec3, l: QAabb) -> Self {
        let o = origin.to_metres();
        let f = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32];
        let [ex, ey, ez] = frame.axes();
        let m = |q: Q| q as f32 / M as f32;
        Self {
            o: [o.x, o.y, o.z],
            ax: [f(ex), f(ey), f(ez)],
            mn: [m(l.min.x), m(l.min.y), m(l.min.z)],
            mx: [m(l.max.x), m(l.max.y), m(l.max.z)],
        }
    }

    fn contains(&self, p: Vec3) -> bool {
        let d = [p.x - self.o[0], p.y - self.o[1], p.z - self.o[2]];
        (0..3).all(|k| {
            let a = self.ax[k];
            let v = a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
            self.mn[k] <= v && v < self.mx[k]
        })
    }
}

struct Entry<'a> {
    lo: [f32; 3],
    hi: [f32; 3],
    prim: &'a Primitive,
    boxed: Option<Boxed>,
}

impl Entry<'_> {
    fn contains(&self, p: Vec3) -> bool {
        (0..3).all(|k| self.lo[k] <= p[k] && p[k] < self.hi[k])
            && self
                .boxed
                .as_ref()
                .map_or_else(|| self.prim.shape.contains(p), |b| b.contains(p))
    }
}

/// Solid pieces by bucket, for point and column queries.
struct Index<'a> {
    entries: Vec<Entry<'a>>,
    buckets: Vec<Vec<u32>>,
    big: Vec<u32>,
}

fn bucket_of(v: f32) -> Option<i32> {
    let b = ((v * M as f32).floor() as i32 - GRID_LO).div_euclid(BUCKET);
    (0..GRID_N).contains(&b).then_some(b)
}

fn slot(x: i32, y: i32, z: i32) -> usize {
    ((x * GRID_N + y) * GRID_N + z) as usize
}

impl<'a> Index<'a> {
    fn new(set: &'a [Primitive], all: bool) -> Self {
        let mut entries = Vec::new();
        let mut buckets = vec![Vec::new(); (GRID_N * GRID_N * GRID_N) as usize];
        let mut big = Vec::new();
        let (lo, hi) = (GRID_LO, CELL - GRID_LO);
        for p in set {
            if !all
                && (p.flags.contains(PrimFlags::NO_COLLIDE)
                    || matches!(p.shape, Shape::LiftShaft { .. })
                    || p.kind == FeatureKind::VOID
                    || p.kind == FeatureKind::WINDOW)
            {
                continue;
            }
            let b = p.aabb();
            let (mn, mx) = (
                [b.min.x.max(lo), b.min.y.max(lo), b.min.z.max(lo)],
                [b.max.x.min(hi), b.max.y.min(hi), b.max.z.min(hi)],
            );
            if (0..3).any(|k| mn[k] >= mx[k]) {
                continue;
            }
            let i = entries.len() as u32;
            let m = |q: Q| q as f32 / M as f32;
            entries.push(Entry {
                lo: [m(b.min.x), m(b.min.y), m(b.min.z)],
                hi: [m(b.max.x), m(b.max.y), m(b.max.z)],
                prim: p,
                boxed: as_box(&p.shape).map(|(f, o, l)| Boxed::new(f, o, l)),
            });
            let r = |k: usize| {
                (
                    (mn[k] - GRID_LO).div_euclid(BUCKET),
                    (mx[k] - 1 - GRID_LO).div_euclid(BUCKET),
                )
            };
            let (rx, ry, rz) = (r(0), r(1), r(2));
            let count = i64::from(rx.1 - rx.0 + 1)
                * i64::from(ry.1 - ry.0 + 1)
                * i64::from(rz.1 - rz.0 + 1);
            if count > BIG {
                big.push(i);
                continue;
            }
            for x in rx.0..=rx.1 {
                for y in ry.0..=ry.1 {
                    for z in rz.0..=rz.1 {
                        buckets[slot(x, y, z)].push(i);
                    }
                }
            }
        }
        Self {
            entries,
            buckets,
            big,
        }
    }

    fn contains(&self, p: Vec3) -> bool {
        let near = match (bucket_of(p.x), bucket_of(p.y), bucket_of(p.z)) {
            (Some(x), Some(y), Some(z)) => self.buckets[slot(x, y, z)].as_slice(),
            _ => &[],
        };
        near.iter()
            .chain(&self.big)
            .any(|&i| self.entries[i as usize].contains(p))
    }

    /// Whether the vertical line at `(x, z)` meets a piece between heights `y_hi` and `y_lo`
    /// at a point that `accept` allows (sampled every 0.25 m, thinner than any floor).
    fn column_hit(
        &self,
        x: f32,
        z: f32,
        y_hi: f32,
        y_lo: f32,
        accept: &dyn Fn(Vec3) -> bool,
    ) -> bool {
        let (bx, bz) = (bucket_of(x), bucket_of(z));
        let (top, bottom) = (
            bucket_of(y_hi).unwrap_or(GRID_N - 1),
            bucket_of(y_lo).unwrap_or(0),
        );
        let mut lists: Vec<&[u32]> = vec![&self.big];
        if let (Some(bx), Some(bz)) = (bx, bz) {
            lists.extend((bottom..=top).map(|by| self.buckets[slot(bx, by, bz)].as_slice()));
        }
        for list in lists {
            for &i in list {
                let e = &self.entries[i as usize];
                if x < e.lo[0] || x >= e.hi[0] || z < e.lo[2] || z >= e.hi[2] {
                    continue;
                }
                let (mut y, floor) = (y_hi.min(e.hi[1] - 1e-3), y_lo.max(e.lo[1]));
                while y >= floor {
                    let p = Vec3::new(x, y, z);
                    if e.contains(p) && accept(p) {
                        return true;
                    }
                    y -= 0.25;
                }
            }
        }
        false
    }
}

/// What the world is made of at a point: the cell's own pieces, which stand wherever they
/// are, and the parent's solid and structures, which the clear boxes of passages cut open.
pub struct World<'a> {
    pieces: Index<'a>,
    solid: Index<'a>,
    bores: Index<'a>,
}

impl<'a> World<'a> {
    pub fn new(pieces: &'a [Primitive], solid: &'a [Primitive], bores: &'a [Primitive]) -> Self {
        Self {
            pieces: Index::new(pieces, false),
            solid: Index::new(solid, false),
            bores: Index::new(bores, true),
        }
    }

    fn contains(&self, p: Vec3) -> bool {
        self.pieces.contains(p) || (self.solid.contains(p) && !self.cut(p))
    }

    fn cut(&self, p: Vec3) -> bool {
        self.bores.contains(p)
    }

    /// Whether anything solid stands in the column below `(x, z)` down to `depth` metres.
    fn column_hit(&self, x: f32, y: f32, z: f32, depth: f32) -> bool {
        self.pieces.column_hit(x, z, y, y - depth, &|_| true)
            || self.solid.column_hit(x, z, y, y - depth, &|p| !self.cut(p))
    }
}

/// What the edges of a cell came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeStats {
    /// Runs of open edge above a drop, before the draw.
    pub runs: usize,
    pub guarded: usize,
    /// Edge samples above a drop that already had a rail or a wall at them.
    pub already: usize,
    /// Of `runs`, those on the sides of flights.
    pub flight_runs: usize,
    /// Rail boxes added (after joining the rails of neighbouring deck pieces).
    pub added: usize,
}

/// A way on: a line a walker follows (cell-local metres, on the walking surface) and half the
/// width of the walkway, as the edge pass sees it (PW.4.11): no rail crosses it.
#[derive(Debug, Clone, Copy)]
pub struct Way {
    pub a: DVec3,
    pub b: DVec3,
    pub half: f64,
    /// The walk of `PathPieces::walks` it belongs to; `usize::MAX` for a doorway.
    pub walk: usize,
}

/// Rails stop this far short of a way on.
const WAY_MARGIN: f64 = 0.3;
/// How far above or below a way's walking surface a rail base still counts as on it.
const WAY_REACH: f64 = 2.0;

/// A piece's frame, origin and the unit axes in cell-local metres.
struct Placed {
    frame: Frame,
    origin: QVec3,
    axes: [DVec3; 3],
}

impl Placed {
    fn new(frame: Frame, origin: QVec3) -> Self {
        Self {
            frame,
            origin,
            axes: frame.axes(),
        }
    }

    /// A point of the piece's own frame (quanta, fractional) in cell-local metres.
    fn at(&self, l: [f64; 3]) -> DVec3 {
        let o = self.origin.to_metres();
        DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z))
            + (self.axes[0] * l[0] + self.axes[1] * l[1] + self.axes[2] * l[2]) / f64::from(M)
    }
}

/// A rail along a deck edge in the canonical coordinates of its frame (metres from the
/// cell's origin, along the frame's axes), so that rails of neighbouring pieces can join.
#[derive(Clone)]
struct Strip {
    frame: Frame,
    along: usize,
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Placed {
    /// The rail box `lo..hi` (local quanta) as plumb strips in the upright frame (`plumb.rs`):
    /// none when the piece is not a walking surface.
    fn strips(&self, lo: [Q; 3], hi: [Q; 3], along: usize) -> Vec<Strip> {
        let Some((upright, _, _)) = plumb::upright(self.frame) else {
            return Vec::new();
        };
        plumb::plumb_box(
            self.frame,
            self.world_origin(),
            Self::local(lo),
            Self::local(hi),
            0.5,
        )
        .into_iter()
        .map(|b| Strip {
            frame: upright,
            along,
            lo: b.lo.to_array(),
            hi: b.hi.to_array(),
        })
        .collect()
    }

    fn world_origin(&self) -> DVec3 {
        let o = self.origin.to_metres();
        DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z))
    }

    fn local(q: [Q; 3]) -> DVec3 {
        DVec3::new(f64::from(q[0]), f64::from(q[1]), f64::from(q[2])) / f64::from(M)
    }
}

/// A way in the upright frame of some heading, ready for `cut_ways`.
struct Prepared {
    a: DVec3,
    b: DVec3,
    half: f64,
    flat: DVec3,
    /// Bounds grown by the half width and the margin.
    lo: DVec3,
    hi: DVec3,
}

fn prepare(ways: &[Way], frame: Frame) -> Vec<Prepared> {
    let Some((_, hx, hz)) = plumb::upright(frame) else {
        return Vec::new();
    };
    let to_f0 = |p: DVec3| DVec3::new(p.dot(hx), p.y, p.dot(hz));
    ways.iter()
        .map(|w| {
            let (a, b) = (to_f0(w.a), to_f0(w.b));
            let g = DVec3::splat(w.half + WAY_MARGIN);
            Prepared {
                a,
                b,
                half: w.half,
                flat: DVec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or_zero(),
                lo: a.min(b) - g,
                hi: a.max(b) + g,
            }
        })
        .collect()
}

/// `s` without the stretches that a way on crosses or comes within a margin of (PW.4.11);
/// stretches left shorter than 0.5 m are dropped.
fn cut_ways(s: &Strip, ways: &[Prepared]) -> Vec<Strip> {
    let (a0, a1) = (s.lo[s.along], s.hi[s.along]);
    let across = 2 - s.along;
    let centre = (s.lo[across] + s.hi[across]) / 2.0;
    let base = s.lo[1];
    let (mn, mx) = (
        DVec3::new(s.lo[0], s.lo[1] - WAY_REACH, s.lo[2]),
        DVec3::new(s.hi[0], s.hi[1] + WAY_REACH, s.hi[2]),
    );
    let mut rail = DVec3::ZERO;
    rail[s.along] = 1.0;
    let near: Vec<(DVec3, DVec3, f64)> = ways
        .iter()
        .filter_map(|w| {
            // A rail alongside its own walkway stands at its edge, inside the half width plus
            // the margin; only a rail that crosses a way is kept off it by the margin.
            let parallel = w.flat.dot(rail).abs() > 0.97;
            let r = if parallel {
                w.half - 0.15
            } else {
                w.half + WAY_MARGIN
            };
            let (lo, hi) = (w.lo, w.hi);
            (mx.cmpge(lo).all() && mn.cmple(hi).all()).then_some((w.a, w.b, r))
        })
        .collect();
    if near.is_empty() {
        return vec![s.clone()];
    }
    let blocked = |t: f64| {
        let mut p = DVec3::new(0.0, base, 0.0);
        p[s.along] = t;
        p[across] = centre;
        near.iter().any(|(a, b, r)| {
            let ab = *b - *a;
            let flat = DVec3::new(ab.x, 0.0, ab.z);
            let len2 = flat.length_squared().max(1e-9);
            let u = (DVec3::new(p.x - a.x, 0.0, p.z - a.z).dot(flat) / len2).clamp(0.0, 1.0);
            let q = *a + ab * u;
            DVec3::new(p.x - q.x, 0.0, p.z - q.z).length() <= *r && (p.y - q.y).abs() <= WAY_REACH
        })
    };
    if a1 - a0 < 0.5 {
        // A post or a stub: kept whole if its middle is clear.
        return if blocked((a0 + a1) / 2.0) {
            Vec::new()
        } else {
            vec![s.clone()]
        };
    }
    let n = (((a1 - a0) / 0.25).ceil() as usize).max(1);
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    for i in 0..=n {
        let t = a0 + (a1 - a0) * i as f64 / n as f64;
        let open = !blocked(t);
        match (open, start) {
            (true, None) => start = Some(t),
            (false, Some(b)) => {
                start = None;
                if t - b >= 0.5 {
                    out.push(piece_of(s, b, t - 0.25));
                }
            }
            _ => {}
        }
    }
    if let Some(b) = start {
        if a1 - b >= 0.5 {
            out.push(piece_of(s, b, a1));
        }
    }
    out
}

fn piece_of(s: &Strip, from: f64, to: f64) -> Strip {
    let mut p = s.clone();
    p.lo[s.along] = from;
    p.hi[s.along] = to;
    p
}

/// The rails of `prims` kept off the ways on (PW.4.11): every RAILING box in an upright frame is
/// cut where a way crosses it, stops 0.3 m short of the way, and the stretches under 0.5 m
/// go. Wedge rails and rails in no upright frame are left as they are.
pub fn cut_rails(prims: Vec<Primitive>, ways: &[Way]) -> Vec<Primitive> {
    let mut out = Vec::with_capacity(prims.len());
    let mut prepared: BTreeMap<[i32; 4], Vec<Prepared>> = BTreeMap::new();
    for p in prims {
        let box_of = (p.kind == FeatureKind::RAILING)
            .then(|| as_box(&p.shape))
            .flatten()
            .filter(|(f, _, _)| plumb::is_upright(*f));
        let Some((frame, origin, l)) = box_of else {
            out.push(p);
            continue;
        };
        // Canonical coordinates: metres from the cell's origin along the frame's axes.
        let o = origin.to_metres();
        let o = DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z));
        let t: [f64; 3] = std::array::from_fn(|k| frame.axes()[k].dot(o));
        let m = |q: Q| f64::from(q) / f64::from(M);
        let lo = [m(l.min.x) + t[0], m(l.min.y) + t[1], m(l.min.z) + t[2]];
        let hi = [m(l.max.x) + t[0], m(l.max.y) + t[1], m(l.max.z) + t[2]];
        let along = if hi[0] - lo[0] >= hi[2] - lo[2] { 0 } else { 2 };
        let strip = Strip {
            frame,
            along,
            lo,
            hi,
        };
        let key = match frame {
            Frame::Id => [0, 0, 0, 0],
            Frame::Table { yaw, tilt } => [1, i32::from(yaw), i32::from(tilt), 0],
            Frame::Dir { d } => [2, i32::from(d[0]), i32::from(d[1]), i32::from(d[2])],
        };
        let set = prepared.entry(key).or_insert_with(|| prepare(ways, frame));
        let pieces = cut_ways(&strip, set);
        if pieces.len() == 1
            && pieces[0]
                .lo
                .iter()
                .zip(&lo)
                .all(|(a, b)| (a - b).abs() < 1e-9)
            && pieces[0]
                .hi
                .iter()
                .zip(&hi)
                .all(|(a, b)| (a - b).abs() < 1e-9)
        {
            out.push(p);
            continue;
        }
        for q in pieces {
            out.push(rail(
                q.frame,
                QVec3::new(0, 0, 0),
                quanta_out(q.lo, false),
                quanta_out(q.hi, true),
            ));
        }
    }
    out
}

/// Joins strips of one frame that lie on one line and touch (a gap of up to 0.1 m), and makes
/// the rails. Rails of a long walkway built from many pieces become a few long boxes.
fn merge(strips: &[Strip]) -> Vec<Primitive> {
    let frame_key = |f: Frame| match f {
        Frame::Id => [0, 0, 0, 0],
        Frame::Table { yaw, tilt } => [1, i32::from(yaw), i32::from(tilt), 0],
        Frame::Dir { d } => [2, i32::from(d[0]), i32::from(d[1]), i32::from(d[2])],
    };
    let key = |s: &Strip| {
        let a = s.along;
        let c = 2 - a;
        let r = |v: f64| (v * 32.0).round() as i32;
        (
            frame_key(s.frame),
            a,
            [r(s.lo[1]), r(s.hi[1]), r(s.lo[c]), r(s.hi[c])],
        )
    };
    let mut groups: BTreeMap<_, Vec<(f64, f64, usize)>> = BTreeMap::new();
    for (i, s) in strips.iter().enumerate() {
        groups
            .entry(key(s))
            .or_default()
            .push((s.lo[s.along], s.hi[s.along], i));
    }
    let mut out = Vec::new();
    for (_, mut v) in groups {
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let mut i = 0;
        while i < v.len() {
            let (start, mut end, first) = v[i];
            i += 1;
            while i < v.len() && v[i].0 <= end + 0.1 {
                end = end.max(v[i].1);
                i += 1;
            }
            let s = &strips[first];
            let (mut lo, mut hi) = (s.lo, s.hi);
            lo[s.along] = start;
            hi[s.along] = end;
            let q = |v: f64, up: bool| {
                let x = v * f64::from(M);
                (if up { x.ceil() } else { x.floor() }) as Q
            };
            out.push(rail(
                s.frame,
                QVec3::new(0, 0, 0),
                [q(lo[0], false), q(lo[1], false), q(lo[2], false)],
                [q(hi[0], true), q(hi[1], true), q(hi[2], true)],
            ));
        }
    }
    out
}

fn v3(p: DVec3) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, p.z as f32)
}

/// Whether the open edge at `top` (a point on the edge of a walkable surface) looks onto a drop
/// with nothing guarding it: `Some(false)` if a rail or wall stands there already, `Some(true)`
/// if it is open above a drop, `None` if it is closed or the drop is under 15 m.
fn open_drop(w: &World<'_>, top: DVec3, out: DVec3, up: DVec3) -> Option<bool> {
    // An edge on the cell's side goes on into the neighbour, which decides (PW.4.11): a deck
    // that ends on the face is not an open edge.
    let beyond = top + out * 0.45;
    if [beyond.x, beyond.y, beyond.z]
        .iter()
        .any(|&c| !(0.0..=512.0).contains(&c))
    {
        return None;
    }
    // Closed by a wall above, or continued by a floor within a step down (0.75 m) or the foot
    // of a stair (a step up).
    if w.contains(v3(top + out * 0.45 + up * 0.9))
        || [-0.3, -0.1, 0.25, 0.5, 0.75]
            .iter()
            .any(|&d| w.contains(v3(top + out * 0.45 - up * d)))
    {
        return None;
    }
    let beyond = top + out * 0.45 - up * 0.75;
    if w.column_hit(
        beyond.x as f32,
        beyond.y as f32,
        beyond.z as f32,
        FALL as f32,
    ) {
        return None;
    }
    let guarded = [0.6, 1.05]
        .iter()
        .any(|&h| w.contains(v3(top - out * 0.06 + up * h)));
    Some(!guarded)
}

fn rail(frame: Frame, origin: QVec3, lo: [Q; 3], hi: [Q; 3]) -> Primitive {
    Primitive {
        shape: Shape::oriented(
            frame,
            origin,
            LocalShape::Box {
                aabb: QAabb::new(
                    QVec3::new(lo[0], lo[1], lo[2]),
                    QVec3::new(hi[0], hi[1], hi[2]),
                ),
            },
            QAabb::new(QVec3::new(0, 0, 0), QVec3::new(CELL, CELL, CELL)),
        ),
        material: MaterialId::METAL,
        kind: FeatureKind::RAILING,
        flags: PrimFlags::NONE,
    }
}

/// Quanta of canonical metres, rounded outwards.
fn quanta_out(v: [f64; 3], up: bool) -> [Q; 3] {
    v.map(|m| {
        let x = m * f64::from(M);
        (if up { x.ceil() } else { x.floor() }) as Q
    })
}

/// Plumb steps of a rail along the line from `foot` to `head` (upright-frame coordinates of the
/// wedge's top line: the rail's top is `HEIGHT` above the line of the nosings).
fn stepped_rail(foot: DVec3, head: DVec3, upright: Frame, out: &mut Vec<Primitive>) {
    let h = f64::from(HEIGHT) / f64::from(M);
    let run = DVec3::new(head.x - foot.x, 0.0, head.z - foot.z).length();
    let n = ((run / 1.0).ceil() as usize).max(1);
    let t = f64::from(THICKNESS) / f64::from(M) / 2.0;
    for i in 0..n {
        let (p0, p1) = (
            foot + (head - foot) * (i as f64 / n as f64),
            foot + (head - foot) * ((i + 1) as f64 / n as f64),
        );
        let mn = DVec3::new(p0.x.min(p1.x) - t, p0.y.min(p1.y) - h, p0.z.min(p1.z) - t);
        let mx = DVec3::new(p0.x.max(p1.x) + t, p0.y.max(p1.y), p0.z.max(p1.z) + t);
        out.push(rail(
            upright,
            QVec3::new(0, 0, 0),
            quanta_out(mn.to_array(), false),
            quanta_out(mx.to_array(), true),
        ));
    }
}

/// Which of `prims` to keep (PW.4.15): a decorative piece of a path (anything that is not a
/// floor, a wall of the walk it belongs to or a rail: lamps, frames, pillars, pipes) that stands
/// in the corridor of another walk or a doorway (the walkway's width, 0.3 m to 2.2 m above its
/// surface) is dropped. `owner` says which walk each path piece was built for.
#[allow(clippy::implicit_hasher)]
pub fn decor_mask(
    prims: &[Primitive],
    owner: &std::collections::HashMap<Primitive, usize>,
    ways: &[Way],
) -> Vec<bool> {
    let q = |v: Q| v as f32 / M as f32;
    prims
        .iter()
        .map(|p| {
            let decor = matches!(
                p.kind,
                FeatureKind::MASS | FeatureKind::PIPE | FeatureKind::MISC | FeatureKind::TETHER
            ) && !p.flags.contains(PrimFlags::NO_COLLIDE)
                && owner.contains_key(p);
            if !decor {
                return true;
            }
            let b = p.aabb();
            let (lo, hi) = (
                Vec3::new(q(b.min.x), q(b.min.y), q(b.min.z)),
                Vec3::new(q(b.max.x), q(b.max.y), q(b.max.z)),
            );
            let mine = owner.get(p).copied();
            let in_way = |w: &Way| {
                if Some(w.walk) == mine {
                    return false;
                }
                let (wa, wb) = (v3(w.a), v3(w.b));
                let g = Vec3::splat(w.half as f32 + 0.5);
                if (wa.min(wb) - g).cmpgt(hi).any()
                    || (wa.max(wb) + g + Vec3::Y * 2.5).cmplt(lo).any()
                {
                    return false;
                }
                let len = (wb - wa).length().max(1e-3);
                let dir = (wb - wa) / len;
                let lat = dir.cross(Vec3::Y).normalize_or_zero();
                let n = (len as usize).max(1);
                (0..=n).any(|k| {
                    let at = wa + dir * (len * k as f32 / n as f32);
                    [-0.6f32, 0.0, 0.6].iter().any(|&f| {
                        [0.3f32, 1.2, 2.1].iter().any(|&h| {
                            let pt = at + lat * (f * w.half as f32) + Vec3::Y * h;
                            pt.cmpge(lo).all() && pt.cmple(hi).all() && p.shape.contains(pt)
                        })
                    })
                })
            };
            !ways.iter().any(in_way)
        })
        .collect()
}

/// Which of `prims` to keep: false for the dressing that has no host (PW.4.14): a `NO_COLLIDE` window, lamp or glyph
/// needs solid (a collidable piece of the cell or the parent's solid that no bore cuts) within
/// 10 cm of its bounds, and a cable needs it at both ends unless the end is on the cell's side
/// (the neighbour holds the other end). Called last, on the final geometry, so a host that was
/// cut, dropped or carved away takes its dressing with it.
pub fn orphan_mask(prims: &[Primitive], world: &World<'_>) -> Vec<bool> {
    let q = |v: Q| v as f32 / M as f32;
    prims
        .iter()
        .map(|p| {
            let dressing = p.flags.contains(PrimFlags::NO_COLLIDE)
                && !p.flags.contains(PrimFlags::ENVELOPE)
                && matches!(p.kind, FeatureKind::WINDOW | FeatureKind::CABLE);
            if !dressing {
                return true;
            }
            let b = p.aabb();
            let (lo, hi) = (
                Vec3::new(q(b.min.x), q(b.min.y), q(b.min.z)) - Vec3::splat(0.1),
                Vec3::new(q(b.max.x), q(b.max.y), q(b.max.z)) + Vec3::splat(0.1),
            );
            if p.kind == FeatureKind::CABLE {
                let size = hi - lo;
                let k = (0..3)
                    .max_by(|&a, &b| size[a].total_cmp(&size[b]))
                    .unwrap_or(1);
                let centre = (lo + hi) * 0.5;
                return [lo[k], hi[k]].into_iter().all(|e| {
                    let mut at = centre;
                    at[k] = e;
                    at.cmplt(Vec3::splat(0.2)).any()
                        || at.cmpgt(Vec3::splat(511.8)).any()
                        || world.contains(at)
                });
            }
            let corners = [
                (lo + hi) * 0.5,
                lo,
                hi,
                Vec3::new(lo.x, lo.y, hi.z),
                Vec3::new(hi.x, lo.y, lo.z),
                Vec3::new(lo.x, hi.y, lo.z),
                Vec3::new(hi.x, hi.y, lo.z),
                Vec3::new(lo.x, hi.y, hi.z),
            ];
            corners.iter().any(|&c| world.contains(c))
        })
        .collect()
}

/// The vertical members of `prims` (PW.4.16): a post, frame, pier or hanger (a plumb box at least
/// 1 m high, at most 0.8 m across and three times as tall as wide) that touches nothing at
/// either end is extended down or up, whichever meets solid first within 8 m, by that distance
/// plus 5 cm; one that meets nothing within reach is dropped. Members on the cell's side are
/// left alone (what they stand on is in the neighbour). Returns each piece, changed or not, or
/// `None` for a dropped one.
pub fn fix_members(prims: &[Primitive], world: &World<'_>) -> Vec<Option<Primitive>> {
    let q = |v: Q| v as f32 / M as f32;
    prims
        .iter()
        .map(|p| {
            let member = p.kind != FeatureKind::RAILING
                && !p.flags.contains(PrimFlags::NO_COLLIDE)
                && !p.flags.contains(PrimFlags::INTERIOR)
                && !p.flags.contains(PrimFlags::ENVELOPE);
            let Some((frame, origin, l)) = member.then(|| as_box(&p.shape)).flatten() else {
                return Some(*p);
            };
            if !plumb::is_upright(frame) {
                return Some(*p);
            }
            let (h, wx, wz) = (
                q(l.max.y - l.min.y),
                q(l.max.x - l.min.x),
                q(l.max.z - l.min.z),
            );
            if h < 1.0 || wx.max(wz) > 0.8 || h < 3.0 * wx.max(wz) {
                return Some(*p);
            }
            let b = p.aabb();
            let (lo, hi) = (
                Vec3::new(q(b.min.x), q(b.min.y), q(b.min.z)),
                Vec3::new(q(b.max.x), q(b.max.y), q(b.max.z)),
            );
            let c = (lo + hi) * 0.5;
            if c.x < 1.0 || c.z < 1.0 || c.x > 511.0 || c.z > 511.0 || lo.y < 1.0 || hi.y > 511.0 {
                return Some(*p);
            }
            let meets = |y: f32| world.contains(Vec3::new(c.x, y, c.z));
            if meets(lo.y - 0.05) || meets(hi.y + 0.05) {
                return Some(*p);
            }
            let reach = |from: f32, dir: f32| {
                (1..=32)
                    .map(|k| k as f32 * 0.25)
                    .find(|d| meets(from + dir * d))
            };
            let (down, up) = (reach(lo.y, -1.0), reach(hi.y, 1.0));
            let (extend_down, d) = match (down, up) {
                (Some(d), Some(u)) if d <= u => (true, d),
                (Some(d), None) => (true, d),
                (_, Some(u)) => (false, u),
                (None, None) => return None,
            };
            let dq = ((d + 0.05) * M as f32).ceil() as Q;
            let mut l = l;
            if extend_down {
                l.min.y -= dq;
            } else {
                l.max.y += dq;
            }
            let shape = match p.shape {
                Shape::Oriented(o) => {
                    Shape::oriented(frame, origin, LocalShape::Box { aabb: l }, o.clip)
                }
                _ => Shape::Box { aabb: l },
            };
            Some(Primitive { shape, ..*p })
        })
        .collect()
}

/// The rails for the open edges of the walkable pieces among `pieces`, in cell-local quanta.
pub fn guard_edges(
    seed: WorldSeed,
    cell: CellKey,
    pieces: &[Primitive],
    world: &World<'_>,
) -> (Vec<Primitive>, EdgeStats) {
    debug_assert_eq!(cell.level, Level::L2);
    let mut out = Vec::new();
    let mut strips = Vec::new();
    let mut stats = EdgeStats::default();
    let mut draw = |idx: usize, edge: i64, run: i64| {
        let key = cell.coords + I64Vec3::new(idx as i64, edge, run);
        Rng::new(hash(seed.0, Level::L2, key, salt::EDGE_GUARD)).below(100) < GUARDED
    };
    for (idx, p) in pieces.iter().enumerate() {
        if p.flags.contains(PrimFlags::NO_COLLIDE) {
            continue;
        }
        match p.kind {
            FeatureKind::WALKWAY | FeatureKind::DECK | FeatureKind::BRIDGE => {
                let Some((frame, origin, l)) = as_box(&p.shape) else {
                    continue;
                };
                let (sx, sy, sz) = (l.max.x - l.min.x, l.max.y - l.min.y, l.max.z - l.min.z);
                if !(2..=M).contains(&sy) || sx < M / 2 || sz < M / 2 {
                    continue;
                }
                deck_edges(
                    world,
                    &Placed::new(frame, origin),
                    l,
                    idx,
                    &mut draw,
                    &mut strips,
                    &mut stats,
                );
            }
            FeatureKind::STAIR_TOWER => {
                if let Shape::Oriented(o) = p.shape {
                    if let LocalShape::StairRun { aabb, rise, steps } = o.local {
                        flight_edges(
                            world,
                            &Placed::new(o.frame, o.origin),
                            (aabb, rise, steps),
                            idx,
                            &mut draw,
                            &mut out,
                            &mut stats,
                        );
                    }
                } else if let Shape::StairRun { aabb, rise, steps } = p.shape {
                    flight_edges(
                        world,
                        &Placed::new(Frame::Id, QVec3::new(0, 0, 0)),
                        (aabb, rise, steps),
                        idx,
                        &mut draw,
                        &mut out,
                        &mut stats,
                    );
                }
            }
            _ => {}
        }
    }
    out.extend(merge(&strips));
    stats.added = out.len();
    (out, stats)
}

/// The four edges of a thin box's top face, sampled every metre.
fn deck_edges(
    w: &World<'_>,
    at: &Placed,
    l: QAabb,
    idx: usize,
    draw: &mut dyn FnMut(usize, i64, i64) -> bool,
    strips: &mut Vec<Strip>,
    stats: &mut EdgeStats,
) {
    // Only a floor that can be walked on (within the slope limit of level) gets rails, and
    // gravity, not the piece, says which way is up.
    if !plumb::walkable(at.frame) {
        return;
    }
    let up = DVec3::Y;
    // Something to walk on: standing room over the middle of the top face. (A rib under a deck
    // or a plate under a wall has none.)
    let mid = at.at([
        f64::from(l.min.x + l.max.x) / 2.0,
        f64::from(l.max.y),
        f64::from(l.min.z + l.max.z) / 2.0,
    ]);
    if [0.1, 0.9, 1.7]
        .iter()
        .any(|&h| w.contains(v3(mid + up * h)))
    {
        return;
    }
    // (along axis, outward sign, edge coordinate on the other axis)
    for (e, (along, sign)) in [(0usize, -1), (0, 1), (2, -1), (2, 1)]
        .into_iter()
        .enumerate()
    {
        let across = 2 - along;
        let (a0, a1) = (
            [l.min.x, l.min.y, l.min.z][along],
            [l.max.x, l.max.y, l.max.z][along],
        );
        let edge = if sign < 0 {
            [l.min.x, l.min.y, l.min.z][across]
        } else {
            [l.max.x, l.max.y, l.max.z][across]
        };
        let outward = at.axes[across] * f64::from(sign);
        let n = ((a1 - a0) / M).max(1);
        let sample = |i: Q| {
            let (s0, s1) = (a0 + (a1 - a0) * i / n, a0 + (a1 - a0) * (i + 1) / n);
            let mut c = [0.0; 3];
            c[along] = f64::from(s0 + s1) / 2.0;
            c[across] = f64::from(edge);
            c[1] = f64::from(l.max.y);
            open_drop(w, at.at(c), outward, up)
        };
        // A long edge whose ends and middle are all closed or guarded is taken as closed.
        if n >= 4 && [0, n / 2, n - 1].iter().all(|&i| sample(i) != Some(true)) {
            stats.already += 1;
            continue;
        }
        let mut run_start: Option<Q> = None;
        for i in 0..=n {
            let s0 = a0 + (a1 - a0) * i / n;
            let state = if i == n {
                None
            } else {
                match sample(i) {
                    Some(false) => {
                        stats.already += 1;
                        None
                    }
                    other => other,
                }
            };
            match (state, run_start) {
                (Some(true), None) => run_start = Some(s0),
                (Some(true), Some(_)) | (_, None) => {}
                (_, Some(start)) => {
                    run_start = None;
                    stats.runs += 1;
                    if draw(idx, e as i64, i64::from(start)) {
                        stats.guarded += 1;
                        let (mut lo, mut hi) = ([0; 3], [0; 3]);
                        lo[along] = start;
                        hi[along] = s0;
                        lo[1] = l.max.y;
                        hi[1] = l.max.y + HEIGHT;
                        if sign < 0 {
                            lo[across] = edge;
                            hi[across] = edge + THICKNESS;
                        } else {
                            lo[across] = edge - THICKNESS;
                            hi[across] = edge;
                        }
                        strips.extend(at.strips(lo, hi, along));
                    }
                }
            }
        }
    }
}

/// The two sides of a flight, in pieces of a few steps each, stepped up the slope.
#[allow(clippy::too_many_lines)] // the sides of a flight, foot box and wedge
fn flight_edges(
    w: &World<'_>,
    at: &Placed,
    (l, rise, steps): (QAabb, Dir4, u16),
    idx: usize,
    draw: &mut dyn FnMut(usize, i64, i64) -> bool,
    out: &mut Vec<Primitive>,
    stats: &mut EdgeStats,
) {
    let along_x = matches!(rise, Dir4::PosX | Dir4::NegX);
    let (along, across) = if along_x { (0usize, 2usize) } else { (2, 0) };
    let lo = [l.min.x, l.min.y, l.min.z];
    let hi = [l.max.x, l.max.y, l.max.z];
    let Some((upright, hx, hz)) = plumb::upright(at.frame) else {
        return;
    };
    let up = DVec3::Y;
    let forward = matches!(rise, Dir4::PosX | Dir4::PosZ);
    let steps = f64::from(steps);
    for (e, sign) in [-1i64, 1].into_iter().enumerate() {
        let edge = if sign < 0 { lo[across] } else { hi[across] };
        let outward = at.axes[across] * sign as f64;
        // Three places along the flight, a tread's height above the foot at each.
        let mut open = false;
        let mut guarded = false;
        for f in [0.1, 0.5, 0.9] {
            let u = f64::from(lo[along]) + f64::from(hi[along] - lo[along]) * f;
            let n = if forward { f } else { 1.0 - f };
            let mut c = [0.0; 3];
            c[along] = u;
            c[across] = f64::from(edge);
            c[1] = f64::from(lo[1])
                + f64::from(hi[1] - lo[1]) * n
                + f64::from(hi[1] - lo[1]) / steps / 2.0;
            match open_drop(w, at.at(c), outward, up) {
                Some(true) => open = true,
                Some(false) => guarded = true,
                None => {}
            }
        }
        if guarded && !open {
            stats.already += 1;
        }
        if !open {
            continue;
        }
        stats.runs += 1;
        stats.flight_runs += 1;
        if !draw(idx, e as i64 + 4, 0) {
            continue;
        }
        stats.guarded += 1;
        // A wall 0.125 m thick that follows the flight: a box up to the height of the rail at
        // the foot, and a ramp on it that rises with the flight, so its top is the rail's
        // height above the line of the nosings all the way up. Both stand plumb: they are built
        // in the upright frame of the flight's heading.
        let band = |a: &mut [Q; 3], b: &mut [Q; 3]| {
            a[along] = lo[along];
            b[along] = hi[along];
            // A quantum inside the side of the flight, so that no face of the rail lies in the
            // plane of the flight's own side.
            if sign < 0 {
                a[across] = edge + INSET;
                b[across] = edge + INSET + THICKNESS;
            } else {
                a[across] = edge - INSET - THICKNESS;
                b[across] = edge - INSET;
            }
        };
        let (mut a, mut b) = ([0; 3], [0; 3]);
        band(&mut a, &mut b);
        a[1] = lo[1] + HEIGHT;
        b[1] = hi[1] + HEIGHT;
        // The ends of the wedge's top line, in the upright frame: the foot and the head.
        let corner = |along_hi: bool, across_hi: bool, y: Q| {
            let mut c = [0; 3];
            c[along] = if along_hi { b[along] } else { a[along] };
            c[across] = if across_hi { b[across] } else { a[across] };
            c[1] = y;
            let w = at.at(c.map(f64::from));
            DVec3::new(w.dot(hx), w.y, w.dot(hz))
        };
        let (foot_hi, head_hi) = (!forward, forward);
        let pts = [
            (corner(foot_hi, false, a[1]), corner(foot_hi, true, a[1])),
            (corner(head_hi, false, b[1]), corner(head_hi, true, b[1])),
        ];
        let run = if along == 0 { 0 } else { 2 };
        let (foot, head) = ((pts[0].0 + pts[0].1) / 2.0, (pts[1].0 + pts[1].1) / 2.0);
        let all = [pts[0].0, pts[0].1, pts[1].0, pts[1].1];
        let (mut mn, mut mx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
        for p in all {
            mn = mn.min(p);
            mx = mx.max(p);
        }
        let other = 2 - run;
        if (head[other] - foot[other]).abs() > 0.2 {
            // A flight in a tilted frame climbs along the frame's up, which is slanted: its
            // line is not in a plane of the upright frame, so the rail is a row of plumb steps
            // along the line of the nosings.
            stepped_rail(foot, head, upright, out);
            continue;
        }
        // The slanted case above and this one are the same: one plumb box to every metre along
        // the nosings, so no level wall hangs beside the flight at the foot's height.
        stepped_rail(foot, head, upright, out);
    }
}

#[cfg(test)]
#[allow(clippy::default_trait_access, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::l2_port::L1Cache;
    use crate::l2_web::l2_cell_web_edges;

    fn cells(n: i64) -> impl Iterator<Item = CellKey> {
        (0..n).map(|k| {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            )
        })
    }

    /// Open edges above a drop are guarded in runs with chance 0.85, and what is left is the
    /// deliberately unguarded share; the rails cost pieces, counted here.
    #[test]
    fn open_edges_above_a_drop_are_guarded() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut total, mut rails, mut pieces) = (EdgeStats::default(), 0usize, 0usize);
        let t0 = std::time::Instant::now();
        let n = 120;
        for key in cells(n) {
            let (out, st) = l2_cell_web_edges(seed, &cache, key);
            total.runs += st.runs;
            total.guarded += st.guarded;
            total.already += st.already;
            total.flight_runs += st.flight_runs;
            total.added += st.added;
            rails += out
                .primitives
                .iter()
                .filter(|p| p.kind == FeatureKind::RAILING)
                .count();
            pieces += out.primitives.len();
        }
        eprintln!(
            "{n} cells in {:?}: {total:?}, {rails} rail pieces of {pieces}",
            t0.elapsed()
        );
        assert!(total.runs > 100, "only {} runs above a drop", total.runs);
        let share = total.guarded as f64 / total.runs as f64;
        assert!((0.75..=0.95).contains(&share), "guarded share {share}");
        assert!(
            total.added <= 3500 * n as usize,
            "{} rails in {n} cells",
            total.added
        );
    }

    /// PW.4.10: gravity, not the piece, says which way up is. Every rail of the cells stands
    /// plumb (its frame is upright), whatever the frame of the deck or flight it guards.
    #[test]
    fn rails_stand_plumb() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut rails, mut tilted_cells) = (0usize, 0usize);
        for key in cells(120) {
            let (out, _) = l2_cell_web_edges(seed, &cache, key);
            let tissue = crate::tissue::tissue_cell(seed, cache.tissue(), key);
            tilted_cells += usize::from(!crate::plumb::is_upright(tissue.lattice.frame));
            for p in out
                .primitives
                .iter()
                .filter(|p| p.kind == FeatureKind::RAILING)
            {
                rails += 1;
                let Shape::Oriented(o) = p.shape else {
                    continue;
                };
                assert!(
                    crate::plumb::is_upright(o.frame),
                    "cell {key:?}: a rail leans, frame {:?}",
                    o.frame
                );
            }
        }
        eprintln!("{rails} rails in 120 cells, {tilted_cells} with a tilted lattice");
        assert!(rails > 1000 && tilted_cells > 0);
    }

    #[test]
    #[ignore = "timing"]
    fn timing() {
        let cache = L1Cache::default();
        let t0 = std::time::Instant::now();
        for key in cells(20) {
            let _ = l2_cell_web_edges(WorldSeed(0x5eed), &cache, key);
        }
        eprintln!("20 cells {:?}", t0.elapsed());
    }
}
