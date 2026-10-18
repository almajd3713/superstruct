//! Walkability validator: walkable surfaces from primitives, and the nav graph over them.
//! Spec: `docs/spec/primitives.md` §6 and `docs/spec/traversal.md`.
//!
//! The extractor trusts no flag except `NO_COLLIDE`, which makes a primitive invisible to
//! collision and walking. Surfaces are axis-aligned rectangles in plan, flat or sloping along
//! one axis. Round obstacles are treated by their bounding square, which only removes
//! walkable area, never adds it.

use crate::cell::Axis;
use crate::l1::Rect;
use crate::prim::{Dir4, FeatureKind, PrimFlags, Primitive, Q, Shape};

/// Step-up, 0.45 m, rounded down to whole quanta.
pub const STEP_UP: Q = 7;
/// Capsule height, 1.8 m, rounded up.
pub const HEADROOM: Q = 29;
/// A surface must hold a disk of the capsule radius: 2 × 0.3 m, rounded up.
pub const MIN_WIDTH: Q = 10;
/// Steepest walkable incline, `tan 40°`.
const MAX_TAN: f64 = 0.839;
/// Smallest stair tread, 0.25 m.
const MIN_TREAD: Q = 4;

/// A walkable region: a plan rectangle whose height runs from `y0` to `y1` along `rise`
/// (flat when `rise` is `None`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    pub rect: Rect,
    pub y0: Q,
    pub y1: Q,
    pub rise: Option<Dir4>,
    /// The kind of primitive it belongs to, or `MISC` for a lift platform.
    pub kind: FeatureKind,
    /// Index of the source primitive, or the lift shaft's for a platform.
    pub source: usize,
    pub lift: bool,
}

impl Plate {
    /// Height at plan point `(x, z)`, clamped into the rectangle.
    pub fn height_at(&self, x: Q, z: Q) -> f64 {
        let Some(rise) = self.rise else {
            return f64::from(self.y0);
        };
        let r = &self.rect;
        let (t, len) = match rise {
            Dir4::PosX => (x - r.x0, r.x1 - r.x0),
            Dir4::NegX => (r.x1 - x, r.x1 - r.x0),
            Dir4::PosZ => (z - r.z0, r.z1 - r.z0),
            Dir4::NegZ => (r.z1 - z, r.z1 - r.z0),
        };
        let f = (f64::from(t) / f64::from(len)).clamp(0.0, 1.0);
        f64::from(self.y0) + f64::from(self.y1 - self.y0) * f
    }
}

fn rect_of(aabb: &crate::prim::QAabb) -> Rect {
    Rect {
        x0: aabb.min.x,
        z0: aabb.min.z,
        x1: aabb.max.x,
        z1: aabb.max.z,
    }
}

fn overlap_area(a: &Rect, b: &Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.z0 < b.z1 && b.z0 < a.z1
}

/// Strips that approximate a vertical cylinder's disk (chord at each strip's midpoint).
const DISK_STRIPS: i32 = 48;

/// The plan footprint of a solid as rectangles: its box, or strips for a vertical cylinder.
fn footprint(p: &Primitive) -> Vec<Rect> {
    let bb = p.aabb();
    let Shape::Cylinder {
        axis: Axis::Y,
        base,
        radius,
        ..
    } = p.shape
    else {
        return vec![rect_of(&bb)];
    };
    let r = f64::from(radius);
    let h = 2.0 * r / f64::from(DISK_STRIPS);
    (0..DISK_STRIPS)
        .filter_map(|k| {
            let dz = -r + (f64::from(k) + 0.5) * h;
            let half = (r * r - dz * dz).max(0.0).sqrt();
            #[allow(clippy::cast_possible_truncation)] // within the cylinder's bounding box
            let rect = Rect {
                x0: base.x - half.round() as Q,
                x1: base.x + half.round() as Q,
                z0: bb.min.z + (f64::from(k) * h).round() as Q,
                z1: bb.min.z + (f64::from(k + 1) * h).round() as Q,
            };
            (rect.x0 < rect.x1 && rect.z0 < rect.z1).then_some(rect)
        })
        .collect()
}

/// `r` minus `o`, as up to four disjoint rectangles. Whole-width bands above and below `o`
/// come first, so successive cuts along z leave long bands and no thin slivers.
fn subtract(r: Rect, o: &Rect) -> Vec<Rect> {
    if !overlap_area(&r, o) {
        return vec![r];
    }
    let mut out = Vec::with_capacity(4);
    if o.z0 > r.z0 {
        out.push(Rect { z1: o.z0, ..r });
    }
    if o.z1 < r.z1 {
        out.push(Rect { z0: o.z1, ..r });
    }
    let (mz0, mz1) = (r.z0.max(o.z0), r.z1.min(o.z1));
    if o.x0 > r.x0 {
        out.push(Rect {
            x1: o.x0,
            z0: mz0,
            z1: mz1,
            ..r
        });
    }
    if o.x1 < r.x1 {
        out.push(Rect {
            x0: o.x1,
            z0: mz0,
            z1: mz1,
            ..r
        });
    }
    out
}

/// Walkable surfaces of `prims`, plus the lift platforms of their `LiftShaft`s.
pub fn surfaces(prims: &[Primitive]) -> Vec<Plate> {
    let solid = |p: &Primitive| {
        !matches!(p.shape, Shape::LiftShaft { .. }) && !p.flags.contains(PrimFlags::NO_COLLIDE)
    };
    let mut plates = Vec::new();
    for (i, p) in prims.iter().enumerate().filter(|(_, p)| solid(p)) {
        let bb = p.aabb();
        let (rect, y0, y1, rise) = match p.shape {
            Shape::Box { .. } => (rect_of(&bb), bb.max.y, bb.max.y, None),
            Shape::Ramp { rise, .. } => {
                let run = bb.size(rise.axis());
                if f64::from(bb.size(Axis::Y)) > MAX_TAN * f64::from(run) {
                    continue;
                }
                (rect_of(&bb), bb.min.y, bb.max.y, Some(rise))
            }
            Shape::StairRun { rise, steps, .. } => {
                let n = Q::from(steps);
                let (h, run) = (bb.size(Axis::Y), bb.size(rise.axis()));
                if h / n > STEP_UP || run / n < MIN_TREAD {
                    continue;
                }
                (rect_of(&bb), bb.min.y, bb.max.y, Some(rise))
            }
            _ => continue,
        };
        let top = y1;
        let reach = top + HEADROOM;
        let blockers: Vec<Rect> = prims
            .iter()
            .enumerate()
            .filter(|&(j, q)| j != i && solid(q))
            .filter(|(_, q)| {
                let o = q.aabb();
                o.min.y < reach && o.max.y > y0
            })
            .flat_map(|(_, q)| footprint(q))
            .filter(|o| overlap_area(&rect, o))
            .collect();
        let mut pieces = vec![rect];
        for o in &blockers {
            pieces = pieces.into_iter().flat_map(|r| subtract(r, o)).collect();
        }
        for r in pieces
            .into_iter()
            .filter(|r| r.x1 - r.x0 >= MIN_WIDTH && r.z1 - r.z0 >= MIN_WIDTH)
        {
            // A sloped plate keeps its slope over the cut piece.
            let piece = Plate {
                rect: r,
                y0,
                y1,
                rise,
                kind: p.kind,
                source: i,
                lift: false,
            };
            plates.push(match rise {
                None => piece,
                Some(_) => trim_slope(piece, &rect),
            });
        }
    }
    let lifts = lift_platforms(prims, &plates);
    plates.extend(lifts);
    plates
}

/// Re-derives `y0` and `y1` of a cut piece of a sloped plate from the whole plate's slope.
fn trim_slope(piece: Plate, whole: &Rect) -> Plate {
    let full = Plate {
        rect: *whole,
        ..piece
    };
    let Some(rise) = piece.rise else {
        return piece;
    };
    let (lo, hi) = match rise {
        Dir4::PosX | Dir4::PosZ => (
            (piece.rect.x0, piece.rect.z0),
            (piece.rect.x1, piece.rect.z1),
        ),
        Dir4::NegX => (
            (piece.rect.x1, piece.rect.z0),
            (piece.rect.x0, piece.rect.z1),
        ),
        Dir4::NegZ => (
            (piece.rect.x0, piece.rect.z1),
            (piece.rect.x1, piece.rect.z0),
        ),
    };
    #[allow(clippy::cast_possible_truncation)] // heights are whole quanta within i32
    Plate {
        y0: full.height_at(lo.0, lo.1).round() as Q,
        y1: full.height_at(hi.0, hi.1).round() as Q,
        ..piece
    }
}

/// One flat platform per stop of each `LiftShaft`. A stop is the top of a collidable box lying
/// inside the shaft footprint (the sill of a wall opening), or the height of a flat plate that
/// meets the footprint where every wall is open for the capsule's height. Stops lie in the shaft's
/// height, from its floor up.
fn lift_platforms(prims: &[Primitive], plates: &[Plate]) -> Vec<Plate> {
    let mut out = Vec::new();
    for (i, p) in prims.iter().enumerate() {
        let Shape::LiftShaft { aabb } = p.shape else {
            continue;
        };
        let foot = rect_of(&aabb);
        let walls: Vec<crate::prim::QAabb> = prims
            .iter()
            .filter(|q| {
                matches!(q.shape, Shape::Box { .. })
                    && !q.flags.contains(PrimFlags::NO_COLLIDE)
                    && q.kind == p.kind
            })
            .map(Primitive::aabb)
            .filter(|b| {
                b.min.x >= foot.x0 && b.max.x <= foot.x1 && b.min.z >= foot.z0 && b.max.z <= foot.z1
            })
            .collect();
        let open = |y: Q| {
            walls
                .iter()
                .all(|b| b.min.y >= y + HEADROOM || b.max.y <= y)
        };
        let touching = plates
            .iter()
            .filter(|q| q.rise.is_none() && !q.lift)
            .filter(|q| {
                let (x0, x1) = (foot.x0.max(q.rect.x0), foot.x1.min(q.rect.x1));
                let (z0, z1) = (foot.z0.max(q.rect.z0), foot.z1.min(q.rect.z1));
                x0 <= x1 && z0 <= z1 && ((x1 - x0).max(z1 - z0) >= MIN_WIDTH)
            });
        let mut stops: Vec<Q> = walls
            .iter()
            .map(|b| b.max.y)
            .chain(touching.map(|q| q.y0).filter(|&y| open(y)))
            .filter(|&y| y >= aabb.min.y && y < aabb.max.y)
            .collect();
        stops.sort_unstable();
        stops.dedup();
        out.extend(stops.into_iter().map(|y| Plate {
            rect: foot,
            y0: y,
            y1: y,
            rise: None,
            kind: FeatureKind::MISC,
            source: i,
            lift: true,
        }));
    }
    out
}

/// Whether a person can walk from plate `a` onto plate `b`: the plan rectangles meet (an edge of
/// at least the minimum width, or an overlap) and the surfaces are within a step of each
/// other somewhere on that contact.
pub fn linked(a: &Plate, b: &Plate) -> bool {
    let (x0, x1) = (a.rect.x0.max(b.rect.x0), a.rect.x1.min(b.rect.x1));
    let (z0, z1) = (a.rect.z0.max(b.rect.z0), a.rect.z1.min(b.rect.z1));
    if x0 > x1 || z0 > z1 {
        return false;
    }
    let contact = if x0 < x1 && z0 < z1 {
        true
    } else {
        (x1 - x0).max(z1 - z0) >= MIN_WIDTH
    };
    if !contact {
        return false;
    }
    let d: Vec<f64> = [(x0, z0), (x1, z0), (x0, z1), (x1, z1)]
        .iter()
        .map(|&(x, z)| a.height_at(x, z) - b.height_at(x, z))
        .collect();
    let (lo, hi) = d
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    lo <= 0.0 && hi >= 0.0 || lo.abs().min(hi.abs()) <= f64::from(STEP_UP)
}

fn find(p: &mut [usize], mut i: usize) -> usize {
    while p[i] != i {
        p[i] = p[p[i]];
        i = p[i];
    }
    i
}

fn collidable(p: &Primitive) -> bool {
    !matches!(p.shape, Shape::LiftShaft { .. }) && !p.flags.contains(PrimFlags::NO_COLLIDE)
}

/// Whether the plan point is under a solid's footprint: its box, or the disk of a vertical
/// cylinder.
fn covers(p: &Primitive, x: Q, z: Q) -> bool {
    let bb = p.aabb();
    if let Shape::Cylinder {
        axis: Axis::Y,
        base,
        radius,
        ..
    } = p.shape
    {
        let (dx, dz) = (i64::from(x - base.x), i64::from(z - base.z));
        return dx * dx + dz * dz < i64::from(radius) * i64::from(radius);
    }
    bb.min.x <= x && x < bb.max.x && bb.min.z <= z && z < bb.max.z
}

/// The highest top, at or below `y`, of a collidable solid under the plan point.
pub fn solid_top_below(prims: &[Primitive], x: Q, z: Q, y: Q) -> Option<Q> {
    prims
        .iter()
        .filter(|p| collidable(p) && covers(p, x, z))
        .map(Primitive::aabb)
        .filter(|bb| bb.min.y < y)
        .map(|bb| bb.max.y.min(y))
        .max()
}

/// Collidable primitives bucketed by plan position, for many point queries against one set.
/// Answers exactly what [`solid_top_below`] and [`solid_in_column`] answer, without scanning
/// every primitive per query.
pub struct ColumnIndex<'a> {
    prims: &'a [Primitive],
    shift: u32,
    side: usize,
    buckets: Vec<Vec<u32>>,
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // clamped to the grid
impl<'a> ColumnIndex<'a> {
    /// Buckets of `2^shift` quanta over a square of `extent` quanta from the origin. Primitives
    /// outside the square are kept in the nearest edge bucket, so queries stay correct.
    pub fn new(prims: &'a [Primitive], extent: Q, shift: u32) -> Self {
        let side = ((extent >> shift) + 1).max(1) as usize;
        let mut buckets = vec![Vec::new(); side * side];
        let clamp = |v: Q| ((v >> shift).max(0) as usize).min(side - 1);
        for (i, p) in prims.iter().enumerate().filter(|(_, p)| collidable(p)) {
            let bb = p.aabb();
            // A cylinder's disk lies inside its box, so the box bounds both.
            let (x0, x1) = (clamp(bb.min.x), clamp(bb.max.x));
            let (z0, z1) = (clamp(bb.min.z), clamp(bb.max.z));
            for z in z0..=z1 {
                for x in x0..=x1 {
                    buckets[z * side + x].push(i as u32);
                }
            }
        }
        Self {
            prims,
            shift,
            side,
            buckets,
        }
    }

    fn bucket(&self, x: Q, z: Q) -> &[u32] {
        let clamp = |v: Q| ((v >> self.shift).max(0) as usize).min(self.side - 1);
        &self.buckets[clamp(z) * self.side + clamp(x)]
    }

    pub fn solid_top_below(&self, x: Q, z: Q, y: Q) -> Option<Q> {
        self.bucket(x, z)
            .iter()
            .map(|&i| &self.prims[i as usize])
            .filter(|p| covers(p, x, z))
            .map(Primitive::aabb)
            .filter(|bb| bb.min.y < y)
            .map(|bb| bb.max.y.min(y))
            .max()
    }

    pub fn solid_in_column(&self, x: Q, z: Q, y0: Q, y1: Q) -> bool {
        self.bucket(x, z).iter().any(|&i| {
            let p = &self.prims[i as usize];
            let bb = p.aabb();
            covers(p, x, z) && bb.min.y < y1 && bb.max.y > y0
        })
    }
}

/// Whether a collidable solid occupies the plan point somewhere strictly inside `(y0, y1)`.
pub fn solid_in_column(prims: &[Primitive], x: Q, z: Q, y0: Q, y1: Q) -> bool {
    prims.iter().any(|p| {
        let bb = p.aabb();
        collidable(p) && covers(p, x, z) && bb.min.y < y1 && bb.max.y > y0
    })
}

/// The primitives of `parent` that meet the cube of `size` quanta at `origin` (in the parent's
/// frame), moved into that cube's frame. Boxes are cut to the cube in plan, never in height,
/// so a deck's top stays where it is. Lift shafts and cylinders move whole.
pub fn cell_window(parent: &[Primitive], origin: [Q; 3], size: Q) -> Vec<Primitive> {
    let [ox, oy, oz] = origin;
    let shift = |v: crate::prim::QVec3| crate::prim::QVec3::new(v.x - ox, v.y - oy, v.z - oz);
    let margin = 2 * crate::prim::Q_PER_M;
    parent
        .iter()
        .filter_map(|p| {
            let bb = p.aabb();
            let (lo, hi) = (shift(bb.min), shift(bb.max));
            let meets = lo.x < size
                && hi.x > 0
                && lo.z < size
                && hi.z > 0
                && lo.y < size + margin
                && hi.y > -margin;
            if !meets {
                return None;
            }
            let shape = match p.shape {
                Shape::Box { .. } => Shape::Box {
                    aabb: crate::prim::QAabb::new(
                        crate::prim::QVec3::new(lo.x.max(0), lo.y, lo.z.max(0)),
                        crate::prim::QVec3::new(hi.x.min(size), hi.y, hi.z.min(size)),
                    ),
                },
                Shape::LiftShaft { .. } => Shape::LiftShaft {
                    aabb: crate::prim::QAabb::new(lo, hi),
                },
                Shape::Cylinder {
                    axis,
                    base,
                    length,
                    radius,
                } => Shape::Cylinder {
                    axis,
                    base: shift(base),
                    length,
                    radius,
                },
                Shape::Ramp { rise, .. } => Shape::Ramp {
                    aabb: crate::prim::QAabb::new(lo, hi),
                    rise,
                },
                Shape::StairRun { rise, steps, .. } => Shape::StairRun {
                    aabb: crate::prim::QAabb::new(lo, hi),
                    rise,
                    steps,
                },
            };
            Some(Primitive { shape, ..*p })
        })
        .collect()
}

/// The plates a person standing in the opening of a port can step onto: those whose plan
/// rectangle reaches the face plane at `plane` along `axis`, overlaps the opening `u` along the
/// face, and is within a step of the port's `floor` there.
pub fn plates_at_port(nav: &Nav, axis: Axis, plane: Q, u: (Q, Q), floor: Q) -> Vec<usize> {
    (0..nav.plates.len())
        .filter(|&i| {
            let r = &nav.plates[i].rect;
            let (n0, n1, t0, t1) = match axis {
                Axis::X => (r.x0, r.x1, r.z0, r.z1),
                _ => (r.z0, r.z1, r.x0, r.x1),
            };
            if !(n0 <= plane && plane <= n1) || t0.max(u.0) >= t1.min(u.1) {
                return false;
            }
            let (x, z) = match axis {
                Axis::X => (plane, t0.max(u.0)),
                _ => (t0.max(u.0), plane),
            };
            (nav.plates[i].height_at(x, z) - f64::from(floor)).abs() <= f64::from(STEP_UP)
        })
        .collect()
}

/// The nav graph of one set of primitives.
#[derive(Debug, Clone)]
pub struct Nav {
    pub plates: Vec<Plate>,
    pub edges: Vec<(usize, usize)>,
    /// Component id per plate: the smallest plate index in its component.
    pub component: Vec<usize>,
}

impl Nav {
    pub fn build(prims: &[Primitive]) -> Self {
        Self::from_plates(surfaces(prims))
    }

    pub fn from_plates(plates: Vec<Plate>) -> Self {
        let n = plates.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let mut edges = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                // Lift platforms of one shaft ride to each other; a lift reaches nothing else
                // except through the sills and decks it overlaps.
                let ride = plates[i].lift && plates[j].lift && plates[i].source == plates[j].source;
                if ride || linked(&plates[i], &plates[j]) {
                    edges.push((i, j));
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let component = (0..n).map(|i| find(&mut parent, i)).collect();
        Self {
            plates,
            edges,
            component,
        }
    }

    /// Whether two plates are in one component.
    pub fn connected(&self, a: usize, b: usize) -> bool {
        self.component[a] == self.component[b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{MaterialId, QAabb, QVec3};

    fn prim(shape: Shape, kind: FeatureKind) -> Primitive {
        Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind,
            flags: PrimFlags::NONE,
        }
    }

    fn slab(x0: Q, z0: Q, x1: Q, z1: Q, bottom: Q, top: Q) -> Primitive {
        prim(
            Shape::Box {
                aabb: QAabb::new(QVec3::new(x0, bottom, z0), QVec3::new(x1, top, z1)),
            },
            FeatureKind::DECK,
        )
    }

    #[test]
    fn open_slab_is_one_plate() {
        let plates = surfaces(&[slab(0, 0, 1600, 1600, -64, 0)]);
        assert_eq!(plates.len(), 1);
        assert_eq!((plates[0].y0, plates[0].rect.x1), (0, 1600));
    }

    #[test]
    fn solid_above_removes_headroom() {
        // A slab 1.7 m above leaves no walkable floor under it; 1.8 m clears.
        let low = surfaces(&[
            slab(0, 0, 1600, 1600, -64, 0),
            slab(400, 400, 800, 800, 27, 91),
        ]);
        let under = |ps: &[Plate]| {
            ps.iter().any(|p| {
                p.y0 == 0
                    && p.rect.x0 <= 500
                    && 700 <= p.rect.x1
                    && p.rect.z0 <= 500
                    && 700 <= p.rect.z1
            })
        };
        assert!(!under(&low));
        let high = surfaces(&[
            slab(0, 0, 1600, 1600, -64, 0),
            slab(400, 400, 800, 800, 29, 93),
        ]);
        assert!(under(&high));
    }

    #[test]
    fn mass_on_deck_splits_deck_but_not_its_connectivity() {
        let nav = Nav::build(&[
            slab(0, 0, 1600, 1600, -64, 0),
            slab(600, 600, 1000, 1000, 0, 800),
        ]);
        let deck: Vec<usize> = (0..nav.plates.len())
            .filter(|&i| nav.plates[i].y0 == 0)
            .collect();
        assert!(deck.len() >= 4);
        assert!(deck.iter().all(|&i| nav.connected(deck[0], i)));
        // The mass top is isolated.
        let top = (0..nav.plates.len())
            .find(|&i| nav.plates[i].y0 == 800)
            .unwrap();
        assert!(!nav.connected(top, deck[0]));
    }

    #[test]
    fn step_limit_decides_links() {
        let a = slab(0, 0, 800, 800, -64, 0);
        let near = slab(800, 0, 1600, 800, -64, 7);
        let far = slab(800, 0, 1600, 800, -64, 8);
        assert!(Nav::build(&[a, near]).connected(0, 1));
        assert!(!Nav::build(&[a, far]).connected(0, 1));
    }

    #[test]
    fn stairs_link_deck_to_landing() {
        // 96 steps of 3 Q rise and 5 Q tread: 18 m up over 30 m, ascending towards +x.
        let run = |steps, rise_q| {
            Shape::stair_run(
                QAabb::new(
                    QVec3::new(0, 0, 0),
                    QVec3::new(5 * steps, rise_q * steps, 64),
                ),
                Dir4::PosX,
                u16::try_from(steps).unwrap(),
            )
        };
        let deck = slab(-800, -400, 0, 400, -64, 0);
        let landing = slab(480, -400, 1280, 400, 100, 288);
        let ok = Nav::build(&[deck, prim(run(96, 3), FeatureKind::RAMP), landing]);
        assert!(ok.connected(0, 2), "{:?}", ok.plates);
        // 0.5 m risers are too tall to walk.
        let steep = Nav::build(&[
            deck,
            prim(run(96, 8), FeatureKind::RAMP),
            slab(480, -400, 1280, 400, 100, 768),
        ]);
        assert!(!steep.plates.iter().any(|p| p.rise.is_some()));
    }

    #[test]
    fn no_collide_is_ignored() {
        let mut ghost = slab(0, 0, 800, 800, 0, 400);
        ghost.flags = PrimFlags::NO_COLLIDE;
        let plates = surfaces(&[slab(0, 0, 1600, 1600, -64, 0), ghost]);
        assert_eq!(plates.len(), 1);
    }

    /// Lemma A, checked by the validator: in an L1 cell, all deck and bridge surfaces and the
    /// lift platforms form one component.
    #[test]
    fn l1_cells_are_internally_connected() {
        use crate::cell::{CellKey, Level};
        use crate::l1::l1_cell;
        use crate::seed::WorldSeed;
        use glam::I64Vec3;
        let (mut cells, mut multi) = (0, 0);
        for (seed, x) in [(0x0901, -10..10), (0x0902, 40..70), (0x0903, -90..-60)] {
            let seed = WorldSeed(seed);
            for x in x {
                for y in [-3, 0, 2] {
                    for z in [-4, 5] {
                        let cell = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                        let out = l1_cell(seed, cell);
                        let nav = Nav::build(&out.primitives);
                        let walk: Vec<usize> = (0..nav.plates.len())
                            .filter(|&i| {
                                let p = &nav.plates[i];
                                p.lift || matches!(p.kind, FeatureKind::DECK | FeatureKind::BRIDGE)
                            })
                            .collect();
                        let Some(&first) = walk.first() else {
                            continue;
                        };
                        cells += 1;
                        multi += usize::from(walk.len() > 3);
                        for &i in &walk {
                            assert!(
                                nav.connected(first, i),
                                "{cell:?}: plate {:?} is cut off",
                                nav.plates[i]
                            );
                        }
                    }
                }
            }
        }
        assert!(cells > 300 && multi > 150, "{cells} {multi}");
    }
}
