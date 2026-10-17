//! L2 face ports. Spec: `docs/spec/ports.md` §3, §4.2 and §5.
//!
//! A face's ports are a function of its key and of the L1 data beside it. **Cascade** ports are
//! the L1 walkable structure that crosses the face. **Own** ports are skywalks over a deck that
//! both sides share. Geometry is in quanta in the face plane `(u, v)`, as `ports.md` §1 defines.

use crate::cavity::CavityView;
use crate::cell::{Axis, CellKey, Dir6, FaceKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::key::{ColumnKey, ColumnSpanKey, LayerKey};
use crate::l1::{
    DECK_REGION_LOG2, Deck, Rect, core_footprint, decks, l1_cell, stair_footprint, trunk,
    trunk_piece,
};
use crate::l1_port::{PORT_CLEARANCE, face_ports as l1_face_ports};
use crate::port::{Port, PortKind};
use crate::prim::{FeatureKind, Q, QRect, Shape};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// L2 cell edge in quanta.
pub const CELL: Q = 512 * M;
/// Own ports keep this far from the face edges.
const MARGIN: Q = 8 * M;
/// Skywalk widths are `2 m × range(1, 5)`.
const WIDTH_STEP: Q = 2 * M;
/// A skywalk floor is its deck's top plus `4 m × range(0, 8)`.
const FLOORS: i32 = 8;
const SLOTS: u32 = 2;

/// All ports on the six faces of L2 cell `cell`, in `Dir6::ALL` order.
pub fn cell_ports(seed: WorldSeed, cell: CellKey) -> Vec<Port> {
    Dir6::ALL
        .iter()
        .filter_map(|&dir| FaceKey::of(cell, dir).ok())
        .flat_map(|face| face_ports(seed, face))
        .collect()
}

/// The ports on L2 face `face`, cascade first (by floor, then `u`), then own ports, with
/// `id = 0, 1, …`.
#[allow(clippy::cast_possible_truncation)] // a handful per face
pub fn face_ports(seed: WorldSeed, face: FaceKey) -> Vec<Port> {
    debug_assert_eq!(face.owner.level, Level::L2);
    let dir = match face.axis {
        Axis::X => Dir6::PosX,
        Axis::Y => Dir6::PosY,
        Axis::Z => Dir6::PosZ,
    };
    let Ok(other) = face.owner.neighbor(dir) else {
        return Vec::new();
    };
    let frame = Frame::new(face, other);
    let mut rects = cascade(seed, &frame);
    if face.axis != Axis::Y {
        own(seed, &frame, &mut rects);
    }
    rects
        .into_iter()
        .enumerate()
        .map(|(i, (kind, rect))| Port {
            face,
            id: i as u16,
            kind,
            rect,
        })
        .collect()
}

/// Where an L2 face sits inside the L1 cells beside it.
struct Frame {
    face: FaceKey,
    /// L1 parents of the owner and of the cell across the face (equal inside an L1 cell).
    po: CellKey,
    pn: CellKey,
    /// The owner's L2 index in its parent, per axis (0..8).
    idx: [Q; 3],
}

impl Frame {
    fn new(face: FaceKey, other: CellKey) -> Self {
        let l1 = |c: CellKey| CellKey::new(Level::L1, c.coords.map(|v| floor_div_pow2(v, 3)));
        let c = face.owner.coords;
        #[allow(clippy::cast_possible_truncation)] // floor_mod < 8
        let idx = [c.x, c.y, c.z].map(|v| floor_mod_pow2(v, 3) as Q);
        Self {
            face,
            po: l1(face.owner),
            pn: l1(other),
            idx,
        }
    }

    /// Whether the face lies on an L1 cell boundary.
    fn on_l1_face(&self) -> bool {
        self.po != self.pn
    }

    /// Tangential axes `(u, v)` as indices into `[x, y, z]`.
    fn uv(&self) -> (usize, usize) {
        match self.face.axis {
            Axis::X => (2, 1),
            Axis::Y => (0, 2),
            Axis::Z => (0, 1),
        }
    }

    fn axis_index(&self) -> usize {
        self.face.axis as usize
    }

    /// The tile's `(u, v)` origin in the owner's L1 frame.
    fn origin(&self) -> (Q, Q) {
        let (u, v) = self.uv();
        (self.idx[u] * CELL, self.idx[v] * CELL)
    }

    /// The face plane's position along its axis in the owner's L1 frame.
    fn plane(&self) -> Q {
        (self.idx[self.axis_index()] + 1) * CELL
    }
}

type Found = Vec<(PortKind, QRect)>;

fn cascade(seed: WorldSeed, f: &Frame) -> Found {
    let (tu, tv) = f.origin();
    let mut out: Found = Vec::new();
    if f.on_l1_face() {
        let l1face = FaceKey {
            owner: f.po,
            axis: f.face.axis,
        };
        for p in l1_face_ports(seed, l1face) {
            let r = p.rect;
            let clipped = QRect {
                u0: r.u0.max(tu) - tu,
                v0: r.v0.max(tv) - tv,
                u1: r.u1.min(tu + CELL) - tu,
                v1: r.v1.min(tv + CELL) - tv,
            };
            if clipped.u0 < clipped.u1 && clipped.v0 < clipped.v1 {
                out.push((p.kind, clipped));
            }
        }
    } else if CavityView::new(seed, f.po).is_cavity(f.po) {
        return out;
    } else if f.face.axis == Axis::Y {
        let core = core_footprint(seed, ColumnKey::of(f.po));
        let r = QRect {
            u0: core.x0.max(tu) - tu,
            v0: core.z0.max(tv) - tv,
            u1: core.x1.min(tu + CELL) - tu,
            v1: core.z1.min(tv + CELL) - tv,
        };
        if r.u0 < r.u1 && r.v0 < r.v1 {
            out.push((PortKind::LiftStop, r));
        }
    } else {
        out = interior_floors(seed, f);
    }
    out.sort_unstable_by_key(|&(k, r)| (r.v0, r.u0, k as u8));
    out
}

/// Deck and bridge floors that cross an X/Z plane inside an L1 cell, merged along the face.
fn interior_floors(seed: WorldSeed, f: &Frame) -> Found {
    let (tu, tv) = f.origin();
    let a = f.plane();
    let (n, u) = (f.axis_index(), f.uv().0);
    // Left and right of the plane: pieces that reach it from each side.
    let mut sides: [Vec<(u8, Q, Q, Q)>; 2] = [Vec::new(), Vec::new()];
    for p in l1_cell(seed, f.po).primitives {
        let kind = match p.kind {
            FeatureKind::DECK => 0,
            FeatureKind::BRIDGE => 1,
            _ => continue,
        };
        let Shape::Box { aabb } = p.shape else {
            continue;
        };
        let (n0, n1) = (aabb.min.get(axis(n)), aabb.max.get(axis(n)));
        let (u0, u1) = (
            aabb.min.get(axis(u)).max(tu),
            aabb.max.get(axis(u)).min(tu + CELL),
        );
        let floor = aabb.max.y;
        if u0 >= u1 || floor < tv || floor >= tv + CELL {
            continue;
        }
        if n0 < a && a <= n1 {
            sides[0].push((kind, floor, u0, u1));
        }
        if n0 <= a && a < n1 {
            sides[1].push((kind, floor, u0, u1));
        }
    }
    let [left, right] = sides.map(merge);
    let mut out = Found::new();
    for l in &left {
        for r in right.iter().filter(|r| r.0 == l.0 && r.1 == l.1) {
            let (u0, u1) = (l.2.max(r.2), l.3.min(r.3));
            if u0 < u1 {
                let kind = if l.0 == 0 {
                    PortKind::DeckContinuation
                } else {
                    PortKind::BridgeEnd
                };
                out.push((
                    kind,
                    QRect {
                        u0: u0 - tu,
                        v0: l.1 - tv,
                        u1: u1 - tu,
                        v1: l.1 - tv + PORT_CLEARANCE,
                    },
                ));
            }
        }
    }
    out
}

fn axis(i: usize) -> Axis {
    Axis::ALL[i]
}

/// Sorts `(kind, floor, u0, u1)` and joins intervals of the same kind and floor that touch.
fn merge(mut v: Vec<(u8, Q, Q, Q)>) -> Vec<(u8, Q, Q, Q)> {
    v.sort_unstable();
    let mut out: Vec<(u8, Q, Q, Q)> = Vec::with_capacity(v.len());
    for e in v {
        match out.last_mut() {
            Some(l) if l.0 == e.0 && l.1 == e.1 && e.2 <= l.3 => l.3 = l.3.max(e.3),
            _ => out.push(e),
        }
    }
    out
}

/// Skywalk ports `[PORT_FACE_*]`: up to two per face, over a deck that both sides share.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // few decks, weights ≤ 1
fn own(seed: WorldSeed, f: &Frame, out: &mut Found) {
    let salt = match f.face.axis {
        Axis::X => salt::PORT_FACE_X,
        Axis::Y => salt::PORT_FACE_Y,
        Axis::Z => salt::PORT_FACE_Z,
    };
    let mut rng = Rng::new(hash(seed.0, Level::L2, f.face.owner.coords, salt));
    let l0 = f.face.owner.coords.map(|v| floor_div_pow2(v, 6));
    let d = f32::from(regime(seed, l0).density) / 255.0;
    let chance = 0.2 + (0.6 - 0.2) * d;

    let cavity = |c: CellKey| CavityView::new(seed, c).is_cavity(c);
    let usable = !cavity(f.po) && (f.po == f.pn || !cavity(f.pn));
    let (tu, tv) = f.origin();
    let layer = |c: CellKey| decks(seed, LayerKey::of(c, DECK_REGION_LOG2));
    let (da, db) = (layer(f.po), layer(f.pn));
    // Decks that are the same slab on both sides, with at least one floor inside this tile.
    let shared: Vec<Deck> = da
        .iter()
        .filter(|d| db.contains(d))
        .copied()
        .filter(|d| {
            (0..FLOORS).any(|k| {
                let v0 = d.top + 4 * M * k - tv;
                (0..=CELL - PORT_CLEARANCE).contains(&v0)
            })
        })
        .collect();

    for _ in 0..SLOTS {
        let present = rng.chance(chance);
        let k = rng.range_i32(0, FLOORS);
        let width = WIDTH_STEP * rng.range_i32(1, 5);
        let u0 =
            MARGIN + WIDTH_STEP * rng.range_i32(0, (CELL - 2 * MARGIN - width) / WIDTH_STEP + 1);
        let pick = rng.below(shared.len().max(1) as u64) as usize;
        let Some(deck) = shared.get(pick).filter(|_| present && usable) else {
            continue;
        };
        let v0 = deck.top + 4 * M * k - tv;
        let rect = QRect {
            u0,
            v0,
            u1: u0 + width,
            v1: v0 + PORT_CLEARANCE,
        };
        let in_tile = (0..=CELL - PORT_CLEARANCE).contains(&v0);
        let overlaps = out
            .iter()
            .any(|(_, r)| r.u0 < rect.u1 && rect.u0 < r.u1 && r.v0 < rect.v1 && rect.v0 < r.v1);
        if in_tile && !overlaps && !blocked(seed, f, (rect.u0 + tu, rect.u1 + tu)) {
            out.push((PortKind::Walkway, rect));
        }
    }
}

/// Whether the face's segment `t` meets the lift core, the staircase footprint or a trunk in
/// either L1 cell beside the face.
fn blocked(seed: WorldSeed, f: &Frame, t: (Q, Q)) -> bool {
    let along_x = f.face.axis == Axis::X;
    [
        (f.po, f.plane()),
        (f.pn, if f.on_l1_face() { 0 } else { f.plane() }),
    ]
    .into_iter()
    .any(|(cell, a)| {
        let seg = if along_x {
            Rect {
                x0: a,
                x1: a,
                z0: t.0,
                z1: t.1,
            }
        } else {
            Rect {
                x0: t.0,
                x1: t.1,
                z0: a,
                z1: a,
            }
        };
        let column = ColumnKey::of(cell);
        let hits = |r: Rect| seg.dist2_rect(&r) == 0;
        let trunk_hit = trunk(seed, ColumnSpanKey::of(cell))
            .filter(|tr| trunk_piece(tr, cell).is_some())
            .is_some_and(|tr| {
                let r = i64::from(tr.radius);
                seg.dist2_point(tr.centre_x, tr.centre_z) < r * r
            });
        hits(core_footprint(seed, column))
            || stair_footprint(seed, column).is_some_and(hits)
            || trunk_hit
    })
}

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::l1::l1_cell;

    fn l1(x: i64, y: i64, z: i64) -> CellKey {
        CellKey::new(Level::L1, I64Vec3::new(x, y, z))
    }

    /// The L2 face tile `(i, j)` of L1 face `f`, on the owner's +axis side.
    fn tile(f: FaceKey, i: i64, j: i64) -> FaceKey {
        let (a, b) = match f.axis {
            Axis::X => (2, 1),
            Axis::Y => (0, 2),
            Axis::Z => (0, 1),
        };
        let mut c = (f.owner.coords * 8).to_array();
        c[f.axis as usize] += 7;
        c[a] += i;
        c[b] += j;
        FaceKey {
            owner: CellKey::new(Level::L2, I64Vec3::from_array(c)),
            axis: f.axis,
        }
    }

    fn overlap(a: &QRect, b: &QRect) -> bool {
        a.u0 < b.u1 && b.u0 < a.u1 && a.v0 < b.v1 && b.v0 < a.v1
    }

    /// P0.8.3: every L1 port is covered by the L2 ports of the tiles it crosses.
    #[test]
    fn l1_ports_cascade_into_l2() {
        let seed = WorldSeed(0x0803);
        let (mut l1_ports, mut l2_ports) = (0, 0);
        for x in -3..3 {
            for z in [-2, 4] {
                for axis in Axis::ALL {
                    let f = FaceKey {
                        owner: l1(x, x + 1, z),
                        axis,
                    };
                    let want = l1_face_ports(seed, f);
                    l1_ports += want.len();
                    for i in 0..8 {
                        for j in 0..8 {
                            let (tu, tv) = (
                                Q::try_from(i).unwrap() * CELL,
                                Q::try_from(j).unwrap() * CELL,
                            );
                            let mut expect = Vec::new();
                            for p in &want {
                                let r = p.rect;
                                let c = QRect {
                                    u0: r.u0.max(tu) - tu,
                                    v0: r.v0.max(tv) - tv,
                                    u1: r.u1.min(tu + CELL) - tu,
                                    v1: r.v1.min(tv + CELL) - tv,
                                };
                                if c.u0 < c.u1 && c.v0 < c.v1 {
                                    expect.push((p.kind, c));
                                }
                            }
                            let mut got: Vec<_> = face_ports(seed, tile(f, i, j))
                                .iter()
                                .filter(|p| p.kind != PortKind::Walkway)
                                .map(|p| (p.kind, p.rect))
                                .collect();
                            got.sort_by_key(|&(k, r)| (r.v0, r.u0, k as u8));
                            expect.sort_by_key(|&(k, r)| (r.v0, r.u0, k as u8));
                            assert_eq!(got, expect, "{f:?} tile {i},{j}");
                            l2_ports += got.len();
                        }
                    }
                }
            }
        }
        assert!(
            l1_ports > 30 && l2_ports > l1_ports,
            "{l1_ports} {l2_ports}"
        );
    }

    /// Inside an L1 cell, every deck or bridge port has flush floor on both sides, and the
    /// port ids are dense; own ports obey their rules and overlap nothing.
    #[test]
    fn interior_and_own_ports_are_sound() {
        let seed = WorldSeed(0x0804);
        let (mut cascade, mut own) = (0, 0);
        for x in -4..4 {
            for z in [-3, 2] {
                let parent = l1(x, 1, z);
                let prims = l1_cell(seed, parent).primitives;
                for k in 0..512i64 {
                    let c = parent.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8);
                    for axis in [Axis::X, Axis::Z] {
                        let face = FaceKey {
                            owner: CellKey::new(Level::L2, c),
                            axis,
                        };
                        let ports = face_ports(seed, face);
                        let (u_axis, n_axis) = if axis == Axis::X { (2, 0) } else { (0, 2) };
                        let o = (c - parent.coords * 8).to_array();
                        let at = |i: usize| Q::try_from(o[i]).unwrap() * CELL;
                        for (i, p) in ports.iter().enumerate() {
                            assert_eq!(usize::from(p.id), i);
                            assert!(p.rect.u0 < p.rect.u1 && p.rect.v1 - p.rect.v0 >= 39);
                            assert!(
                                ports[..i].iter().all(|q| !overlap(&q.rect, &p.rect)),
                                "{p:?}"
                            );
                            if p.kind == PortKind::Walkway {
                                own += 1;
                                assert!(p.rect.u0 >= MARGIN && p.rect.u1 <= CELL - MARGIN);
                                assert!(p.rect.u1 - p.rect.u0 >= 2 * M);
                                continue;
                            }
                            cascade += 1;
                            if o[n_axis] == 7 {
                                continue;
                            }
                            let kind = if p.kind == PortKind::BridgeEnd {
                                FeatureKind::BRIDGE
                            } else {
                                FeatureKind::DECK
                            };
                            let floor = p.rect.v0 + at(1);
                            let plane = at(n_axis) + CELL;
                            for side in [plane - 1, plane] {
                                for u in [
                                    p.rect.u0,
                                    i32::midpoint(p.rect.u0, p.rect.u1),
                                    p.rect.u1 - 1,
                                ] {
                                    let u = u + at(u_axis);
                                    let covered = prims.iter().any(|q| {
                                        let bb = q.aabb();
                                        let (lo, hi) = (bb.min, bb.max);
                                        let (n, t) = (axis_get(&lo, n_axis), axis_get(&lo, u_axis));
                                        q.kind == kind
                                            && hi.y == floor
                                            && n <= side
                                            && side < axis_get(&hi, n_axis)
                                            && t <= u
                                            && u < axis_get(&hi, u_axis)
                                    });
                                    assert!(covered, "{face:?} {p:?}: no floor at {side},{u}");
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(cascade > 500 && own > 50, "{cascade} cascade, {own} own");
    }

    fn axis_get(v: &crate::prim::QVec3, i: usize) -> Q {
        [v.x, v.y, v.z][i]
    }

    #[test]
    fn cells_share_face_ports() {
        let seed = WorldSeed(0x0805);
        let a = CellKey::new(Level::L2, I64Vec3::new(15, 9, -8));
        let b = a.neighbor(Dir6::PosX).unwrap();
        let face = FaceKey::of(a, Dir6::PosX).unwrap();
        assert_eq!(FaceKey::of(b, Dir6::NegX).unwrap(), face);
        let shared = face_ports(seed, face);
        for cell in [a, b] {
            let mine = cell_ports(seed, cell);
            assert!(shared.iter().all(|p| mine.contains(p)));
        }
    }
}
