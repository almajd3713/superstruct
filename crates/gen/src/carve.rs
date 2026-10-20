//! Cutting holes: the clear boxes of bored passages and thresholds (`PathPieces::bores`,
//! `Thresholds::bores`) subtracted from solid boxes. Spec: `docs/spec/levels.md` §5W.27.
//!
//! A hole is exact where the piece is a box and the cut is a box in the same frame: the piece
//! is replaced by up to six boxes that leave the hole. Where the frames differ the hole is
//! covered by slices that may grow it a little (never shrink it). Anything that is not a box (a
//! ramp, a cylinder) stays whole and is counted as unsupported.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;

use glam::DVec3;

use crate::orient::{Frame, LocalShape, Oriented};
use crate::prim::{PrimFlags, Primitive, Q, QAabb, QVec3, Shape};

/// `p` moved by `d` quanta (a primitive of one cell in the coordinates of another).
pub fn translated(p: &Primitive, d: [Q; 3]) -> Primitive {
    let mv = |v: QVec3| QVec3::new(v.x + d[0], v.y + d[1], v.z + d[2]);
    let mva = |a: QAabb| QAabb {
        min: mv(a.min),
        max: mv(a.max),
    };
    let shape = match p.shape {
        Shape::Box { aabb } => Shape::Box { aabb: mva(aabb) },
        Shape::Oriented(o) => Shape::Oriented(Oriented {
            origin: mv(o.origin),
            clip: mva(o.clip),
            ..o
        }),
        other => other,
    };
    Primitive { shape, ..*p }
}

/// The most slices a hole is cut into when its frame is not the piece's.
const MAX_SLICES: Q = 6;

/// Covers of a hole by the frame and origin of the piece they are for.
type Cache = HashMap<(usize, Frame, QVec3), Vec<QAabb>>;

/// What a carving did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CarveStats {
    /// Pieces replaced by the boxes that remain.
    pub cut: usize,
    /// Pieces a hole meets that cannot be cut exactly.
    pub unsupported: usize,
}

/// A box shape as its frame, origin and local box.
fn as_box(shape: &Shape) -> Option<(Frame, QVec3, QAabb)> {
    match *shape {
        Shape::Box { aabb } => Some((Frame::Id, QVec3::default(), aabb)),
        Shape::Oriented(o) => match o.local {
            LocalShape::Box { aabb } => Some((o.frame, o.origin, aabb)),
            _ => None,
        },
        _ => None,
    }
}

fn overlaps(a: &QAabb, b: &QAabb) -> bool {
    a.min.x < b.max.x
        && b.min.x < a.max.x
        && a.min.y < b.max.y
        && b.min.y < a.max.y
        && a.min.z < b.max.z
        && b.min.z < a.max.z
}

/// `a` minus `cut`, as at most six disjoint boxes.
fn split(a: QAabb, cut: QAabb) -> Vec<QAabb> {
    let mut out = Vec::with_capacity(6);
    let mut rest = a;
    let mk = |lo: [Q; 3], hi: [Q; 3]| QAabb {
        min: QVec3::new(lo[0], lo[1], lo[2]),
        max: QVec3::new(hi[0], hi[1], hi[2]),
    };
    let lo = |b: &QAabb| [b.min.x, b.min.y, b.min.z];
    let hi = |b: &QAabb| [b.max.x, b.max.y, b.max.z];
    for k in 0..3 {
        let (c0, c1) = (lo(&cut)[k], hi(&cut)[k]);
        if rest.min.get_k(k) < c0 {
            let mut h = hi(&rest);
            h[k] = c0;
            out.push(mk(lo(&rest), h));
            let mut l = lo(&rest);
            l[k] = c0;
            rest = mk(l, hi(&rest));
        }
        if rest.max.get_k(k) > c1 {
            let mut l = lo(&rest);
            l[k] = c1;
            out.push(mk(l, hi(&rest)));
            let mut h = hi(&rest);
            h[k] = c1;
            rest = mk(lo(&rest), h);
        }
    }
    out
}

/// `a` minus the union of `cuts`, as disjoint boxes: cut by the first, then each piece by the
/// cuts that still meet it. The pieces are about five to a cut, however the cuts overlap.
fn subtract_many(a: QAabb, cuts: &[QAabb]) -> Vec<QAabb> {
    let meeting: Vec<QAabb> = cuts.iter().copied().filter(|c| overlaps(&a, c)).collect();
    let Some((first, rest)) = meeting.split_first() else {
        return vec![a];
    };
    split(a, *first)
        .into_iter()
        .flat_map(|piece| subtract_many(piece, rest))
        .collect()
}

trait GetK {
    fn get_k(&self, k: usize) -> Q;
}

impl GetK for QVec3 {
    fn get_k(&self, k: usize) -> Q {
        [self.x, self.y, self.z][k]
    }
}

/// The hole, as boxes in the piece's local coordinates that cover it. In one frame it is the
/// hole itself. In another it is cut into slices of at most 2 m along its longest side, and each
/// slice is covered by its bounds in the piece's axes (rounded outwards): the hole can grow by
/// a little, never shrink, so nothing solid is left in the passage.
fn cover(piece: (Frame, QVec3), hole: (Frame, QVec3, QAabb), offset: [Q; 3]) -> Vec<QAabb> {
    let shifted = QVec3::new(
        hole.1.x + offset[0],
        hole.1.y + offset[1],
        hole.1.z + offset[2],
    );
    if piece.0 == hole.0 {
        let d = [
            shifted.x - piece.1.x,
            shifted.y - piece.1.y,
            shifted.z - piece.1.z,
        ];
        let d = if piece.0 == Frame::Id {
            d
        } else {
            let [ex, ey, ez] = piece.0.axes();
            let v = DVec3::new(f64::from(d[0]), f64::from(d[1]), f64::from(d[2]));
            [v.dot(ex), v.dot(ey), v.dot(ez)].map(|c| c.round() as Q)
        };
        let s = |p: QVec3| QVec3::new(p.x + d[0], p.y + d[1], p.z + d[2]);
        return vec![QAabb {
            min: s(hole.2.min),
            max: s(hole.2.max),
        }];
    }
    let (hex, hey, hez) = {
        let [a, b, c] = hole.0.axes();
        (a, b, c)
    };
    let [pex, pey, pez] = piece.0.axes();
    let size = [
        hole.2.max.x - hole.2.min.x,
        hole.2.max.y - hole.2.min.y,
        hole.2.max.z - hole.2.min.z,
    ];
    let k = (0..3).max_by_key(|&k| size[k]).unwrap_or(0);
    let lo = [hole.2.min.x, hole.2.min.y, hole.2.min.z];
    let origin = DVec3::new(
        f64::from(shifted.x),
        f64::from(shifted.y),
        f64::from(shifted.z),
    ) - DVec3::new(
        f64::from(piece.1.x),
        f64::from(piece.1.y),
        f64::from(piece.1.z),
    );
    // The bounds, in the piece's axes, of the part of the hole from `t0` to `t1` along side `k`.
    let bounds = |t0: Q, t1: Q| -> QAabb {
        let mut a = lo;
        let mut b = [lo[0] + size[0], lo[1] + size[1], lo[2] + size[2]];
        a[k] = lo[k] + t0;
        b[k] = lo[k] + t1;
        let (mut mn, mut mx) = ([f64::MAX; 3], [f64::MIN; 3]);
        for c in 0..8 {
            let p = [
                if c & 1 == 0 { a[0] } else { b[0] },
                if c & 2 == 0 { a[1] } else { b[1] },
                if c & 4 == 0 { a[2] } else { b[2] },
            ];
            let w = origin + hex * f64::from(p[0]) + hey * f64::from(p[1]) + hez * f64::from(p[2]);
            let l = [w.dot(pex), w.dot(pey), w.dot(pez)];
            for j in 0..3 {
                mn[j] = mn[j].min(l[j]);
                mx[j] = mx[j].max(l[j]);
            }
        }
        QAabb {
            min: QVec3::new(mn[0].floor() as Q, mn[1].floor() as Q, mn[2].floor() as Q),
            max: QVec3::new(mx[0].ceil() as Q, mx[1].ceil() as Q, mx[2].ceil() as Q),
        }
    };
    // As few slices as keep each slice's bounds within GROWTH of the hole's own cross-section.
    let volume = |b: &QAabb| {
        i64::from(b.max.x - b.min.x) * i64::from(b.max.y - b.min.y) * i64::from(b.max.z - b.min.z)
    };
    let exact = i64::from(size[0]) * i64::from(size[1]) * i64::from(size[2]);
    let mut n = 1;
    while n < MAX_SLICES {
        let step = (size[k] + n - 1) / n;
        let worst = (0..n)
            .map(|i| volume(&bounds(i * step, ((i + 1) * step).min(size[k]))))
            .max()
            .unwrap_or(0);
        // The slice's own volume is `exact / n`; allow the slices together to grow to twice the
        // hole and a little (16 384 Q³ is a 2.5 m cube per slice).
        if worst * i64::from(n) <= exact * 2 + i64::from(n) * 16384 {
            break;
        }
        n += 1;
    }
    let step = (size[k] + n - 1) / n;
    (0..n)
        .map(|i| bounds(i * step, ((i + 1) * step).min(size[k])))
        .collect()
}

/// The boxes of `p` that remain outside `holes` (bore shapes in the coordinates of `p` plus
/// `offset`), or `None` when `p` is not a box.
fn cut_one(
    p: &Primitive,
    holes: &[&Primitive],
    offset: [Q; 3],
    cache: &mut Cache,
) -> Option<Vec<Primitive>> {
    let (frame, origin, local) = as_box(&p.shape)?;
    let mut covers: Vec<QAabb> = Vec::new();
    for h in holes {
        // The same hole covers every piece of one frame and origin the same way.
        let key = (std::ptr::from_ref::<Primitive>(h) as usize, frame, origin);
        let c = if let Some(c) = cache.get(&key) {
            c.clone()
        } else {
            let c = cover((frame, origin), as_box(&h.shape)?, offset);
            cache.insert(key, c.clone());
            c
        };
        covers.extend(c);
    }
    Some(
        subtract_many(local, &covers)
            .into_iter()
            .map(|aabb| Primitive {
                shape: match p.shape {
                    Shape::Oriented(o) => Shape::Oriented(Oriented {
                        local: LocalShape::Box { aabb },
                        ..o
                    }),
                    _ => Shape::Box { aabb },
                },
                ..*p
            })
            .collect(),
    )
}

/// Cuts every bore out of the solid boxes of `prims`. The bores are in coordinates `offset`
/// quanta from those of `prims` (an L2 cell's bores in an L1 cell's coordinates).
pub fn carve(
    prims: Vec<Primitive>,
    bores: &[Primitive],
    offset: [Q; 3],
) -> (Vec<Primitive>, CarveStats) {
    let mut stats = CarveStats::default();
    if bores.is_empty() {
        return (prims, stats);
    }
    let holes: Vec<(QAabb, &Primitive)> = bores
        .iter()
        .map(|b| {
            let a = b.aabb();
            let s = |p: QVec3| QVec3::new(p.x + offset[0], p.y + offset[1], p.z + offset[2]);
            (
                QAabb {
                    min: s(a.min),
                    max: s(a.max),
                },
                b,
            )
        })
        .collect();
    let mut out = Vec::with_capacity(prims.len());
    let mut cache: Cache = HashMap::new();
    for p in prims {
        if p.flags.contains(PrimFlags::NO_COLLIDE) {
            out.push(p);
            continue;
        }
        let bb = p.aabb();
        let meeting: Vec<&Primitive> = holes
            .iter()
            .filter(|(hb, _)| overlaps(&bb, hb))
            .map(|(_, h)| *h)
            .collect();
        if meeting.is_empty() {
            out.push(p);
            continue;
        }
        if let Some(pieces) = cut_one(&p, &meeting, offset, &mut cache) {
            stats.cut += 1;
            out.extend(pieces);
        } else {
            stats.unsupported += 1;
            out.push(p);
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{FeatureKind, MaterialId};

    fn bx(lo: [Q; 3], hi: [Q; 3]) -> QAabb {
        QAabb {
            min: QVec3::new(lo[0], lo[1], lo[2]),
            max: QVec3::new(hi[0], hi[1], hi[2]),
        }
    }

    fn prim(aabb: QAabb) -> Primitive {
        Primitive {
            shape: Shape::Box { aabb },
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags: PrimFlags::NONE,
        }
    }

    fn volume(a: &QAabb) -> i64 {
        [a.max.x - a.min.x, a.max.y - a.min.y, a.max.z - a.min.z]
            .iter()
            .map(|&v| i64::from(v))
            .product()
    }

    /// The boxes that remain tile the piece minus the hole exactly: no overlap, no hole left,
    /// and the volume adds up.
    #[test]
    fn a_box_with_a_hole_through_it_keeps_the_rest() {
        let a = bx([0, 0, 0], [160, 80, 160]);
        for cut in [
            bx([60, 20, -50], [100, 60, 300]),
            bx([-10, -10, -10], [50, 50, 50]),
            bx([0, 0, 0], [160, 80, 160]),
            bx([200, 0, 0], [300, 10, 10]),
        ] {
            let parts = subtract_many(a, &[cut]);
            let inter = {
                let lo = [
                    a.min.x.max(cut.min.x),
                    a.min.y.max(cut.min.y),
                    a.min.z.max(cut.min.z),
                ];
                let hi = [
                    a.max.x.min(cut.max.x),
                    a.max.y.min(cut.max.y),
                    a.max.z.min(cut.max.z),
                ];
                if (0..3).all(|k| lo[k] < hi[k]) {
                    volume(&bx(lo, hi))
                } else {
                    0
                }
            };
            let total: i64 = parts.iter().map(volume).sum();
            assert_eq!(total, volume(&a) - inter, "{cut:?}");
            for (i, p) in parts.iter().enumerate() {
                assert!(!overlaps(p, &cut), "{p:?} meets {cut:?}");
                for q in &parts[i + 1..] {
                    assert!(!overlaps(p, q), "{p:?} meets {q:?}");
                }
            }
        }
    }

    #[test]
    fn carving_cuts_solids_and_spares_the_rest() {
        let solid = prim(bx([0, 0, 0], [320, 160, 320]));
        let mut free = prim(bx([0, 0, 0], [320, 160, 320]));
        free.flags = PrimFlags::NO_COLLIDE;
        let far = prim(bx([1000, 0, 0], [1100, 50, 50]));
        let hole = prim(bx([100, 0, 100], [180, 60, 180]));
        let (out, stats) = carve(vec![solid, free, far], &[hole], [0, 0, 0]);
        assert_eq!(
            stats,
            CarveStats {
                cut: 1,
                unsupported: 0
            }
        );
        assert!(out.contains(&free) && out.contains(&far));
        let solids: i64 = out
            .iter()
            .filter(|p| *p != &free && *p != &far)
            .map(|p| volume(&p.aabb()))
            .sum();
        assert_eq!(solids, 320 * 160 * 320 - 80 * 60 * 80);
        // The offset moves the hole: an L2 hole in L1 coordinates.
        let (out, _) = carve(
            vec![prim(bx([0, 0, 0], [100, 100, 100]))],
            &[hole],
            [-100, 0, -100],
        );
        assert!(out.iter().map(|p| volume(&p.aabb())).sum::<i64>() < 100 * 100 * 100);
    }

    #[test]
    fn a_piece_that_cannot_be_cut_is_counted() {
        let cyl = Primitive {
            shape: Shape::cylinder(crate::cell::Axis::Y, QVec3::new(50, 0, 50), 100, 40),
            ..prim(bx([0, 0, 0], [1, 1, 1]))
        };
        let hole = prim(bx([40, 10, 40], [60, 50, 60]));
        let (out, stats) = carve(vec![cyl], &[hole], [0, 0, 0]);
        assert_eq!(out.len(), 1);
        assert_eq!(stats.unsupported, 1);
    }
}
