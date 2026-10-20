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
    clippy::cast_sign_loss
)]

use std::collections::BTreeMap;

use glam::{DVec3, I64Vec3, Vec3};

use crate::carve::as_box;
use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::orient::{Frame, LocalShape};
use crate::prim::{Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
const CELL: Q = 512 * M;
/// Railing height (1.1 m rounded up to quanta) and thickness.
const HEIGHT: Q = 18;
const THICKNESS: Q = 2;
/// The fall threshold, 15 m (`traversal.md` §3), and the metres between two probes down.
const FALL: i32 = 15;
const GUARDED: u64 = 85;
/// Steps of a flight that one rail piece covers.
const FLIGHT_PIECE: u32 = 12;

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
struct Strip {
    frame: Frame,
    along: usize,
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Placed {
    fn strip(&self, lo: [Q; 3], hi: [Q; 3], along: usize) -> Strip {
        let o = self.origin.to_metres();
        let o = DVec3::new(f64::from(o.x), f64::from(o.y), f64::from(o.z));
        let t: [f64; 3] = std::array::from_fn(|k| self.axes[k].dot(o));
        Strip {
            frame: self.frame,
            along,
            lo: std::array::from_fn(|k| t[k] + f64::from(lo[k]) / f64::from(M)),
            hi: std::array::from_fn(|k| t[k] + f64::from(hi[k]) / f64::from(M)),
        }
    }
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
    let up = at.axes[1];
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
                        strips.push(at.strip(lo, hi, along));
                    }
                }
            }
        }
    }
}

/// The two sides of a flight, in pieces of a few steps each, stepped up the slope.
fn flight_edges(
    w: &World<'_>,
    at: &Placed,
    (l, rise, steps): (QAabb, Dir4, u16),
    idx: usize,
    draw: &mut dyn FnMut(usize, i64, i64) -> bool,
    out: &mut Vec<Primitive>,
    stats: &mut EdgeStats,
) {
    let steps = u32::from(steps);
    let along_x = matches!(rise, Dir4::PosX | Dir4::NegX);
    let forward = matches!(rise, Dir4::PosX | Dir4::PosZ);
    let (along, across) = if along_x { (0usize, 2usize) } else { (2, 0) };
    let lo = [l.min.x, l.min.y, l.min.z];
    let hi = [l.max.x, l.max.y, l.max.z];
    let (run, height) = (
        f64::from(hi[along] - lo[along]) / f64::from(steps),
        f64::from(hi[1] - lo[1]) / f64::from(steps),
    );
    let up = at.axes[1];
    let mut j = 0;
    while j < steps {
        let k = (steps - j).min(FLIGHT_PIECE);
        // Steps j..j+k, counted from the foot; the tread of step i has its top at (i + 1) rises.
        let (u0, u1) = if forward {
            (
                f64::from(lo[along]) + f64::from(j) * run,
                f64::from(lo[along]) + f64::from(j + k) * run,
            )
        } else {
            (
                f64::from(hi[along]) - f64::from(j + k) * run,
                f64::from(hi[along]) - f64::from(j) * run,
            )
        };
        let mid = f64::from(j) + f64::from(k) / 2.0;
        let y_mid = f64::from(lo[1]) + (mid + 0.5) * height;
        let y_low = f64::from(lo[1]) + f64::from(j) * height;
        let y_top = f64::from(lo[1]) + f64::from(j + k) * height;
        for (e, sign) in [-1i64, 1].into_iter().enumerate() {
            let edge = if sign < 0 { lo[across] } else { hi[across] };
            let mut c = [0.0; 3];
            c[along] = f64::midpoint(u0, u1);
            c[across] = f64::from(edge);
            c[1] = y_mid;
            let outward = at.axes[across] * sign as f64;
            let state = match open_drop(w, at.at(c), outward, up) {
                Some(false) => {
                    stats.already += 1;
                    None
                }
                other => other,
            };
            if state == Some(true) {
                stats.runs += 1;
                stats.flight_runs += 1;
                if draw(idx, e as i64 + 4, i64::from(j)) {
                    stats.guarded += 1;
                    let (mut a, mut b) = ([0; 3], [0; 3]);
                    a[along] = u0.floor() as Q;
                    b[along] = u1.ceil() as Q;
                    a[1] = y_low.floor() as Q;
                    b[1] = y_top.ceil() as Q + HEIGHT;
                    if sign < 0 {
                        a[across] = edge;
                        b[across] = edge + THICKNESS;
                    } else {
                        a[across] = edge - THICKNESS;
                        b[across] = edge;
                    }
                    out.push(rail(at.frame, at.origin, a, b));
                }
            }
        }
        j += k;
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
            total.added <= 2000 * n as usize,
            "{} rails in {n} cells",
            total.added
        );
    }
}
