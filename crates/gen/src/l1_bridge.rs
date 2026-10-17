//! L1 half-bridges from `BridgeEnd` ports to the lift core. Spec: `docs/spec/levels.md` §4.5.

use crate::cell::{Axis, CellKey, FaceKey};
use crate::l1::{CELL, Rect, Trunk};
use crate::l1_port::BRIDGE_THICKNESS;
use crate::port::{Port, PortKind};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};

const M: Q = 16;
/// Inset of route segments that run along a side face, inside its 64 m band.
const BAND_INSET: Q = 16 * M;
/// Landing width, from the core to a trunk that crosses no deck.
const LANDING_WIDTH: Q = 8 * M;
/// Bridges at least this wide are part of the far band (§4.8).
pub const FAR_BRIDGE_WIDTH: Q = 16 * M;

/// Which face of the cell a port is on, as a frame: `n` runs inward from the face, `t` along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Side {
    fn of(cell: CellKey, face: FaceKey) -> Option<Self> {
        let positive = face.owner == cell;
        match (face.axis, positive) {
            (Axis::X, true) => Some(Self::PosX),
            (Axis::X, false) => Some(Self::NegX),
            (Axis::Z, true) => Some(Self::PosZ),
            (Axis::Z, false) => Some(Self::NegZ),
            (Axis::Y, _) => None,
        }
    }

    /// Frame `(n0, n1, t0, t1)` → cell-local rectangle. The map is its own inverse.
    fn to_cell(self, f: [Q; 4]) -> Rect {
        let [n0, n1, t0, t1] = f;
        match self {
            Self::PosX => Rect {
                x0: CELL - n1,
                x1: CELL - n0,
                z0: t0,
                z1: t1,
            },
            Self::NegX => Rect {
                x0: n0,
                x1: n1,
                z0: t0,
                z1: t1,
            },
            Self::PosZ => Rect {
                x0: t0,
                x1: t1,
                z0: CELL - n1,
                z1: CELL - n0,
            },
            Self::NegZ => Rect {
                x0: t0,
                x1: t1,
                z0: n0,
                z1: n1,
            },
        }
    }

    fn to_frame(self, r: Rect) -> [Q; 4] {
        match self {
            Self::PosX => [CELL - r.x1, CELL - r.x0, r.z0, r.z1],
            Self::NegX => [r.x0, r.x1, r.z0, r.z1],
            Self::PosZ => [CELL - r.z1, CELL - r.z0, r.x0, r.x1],
            Self::NegZ => [r.z0, r.z1, r.x0, r.x1],
        }
    }
}

/// A frame rectangle from two corners in any order.
fn span(n_a: Q, n_b: Q, t_a: Q, t_b: Q) -> [Q; 4] {
    [n_a.min(n_b), n_a.max(n_b), t_a.min(t_b), t_a.max(t_b)]
}

/// A strip of width `w` centred on `[lo, hi)`.
fn centred(lo: Q, hi: Q, w: Q) -> Q {
    lo + (hi - lo - w) / 2
}

/// Candidate routes for one port, in preference order (§4.5). Each is a list of frame rects
/// that chain from the face to a face of the core.
fn routes(port_t: (Q, Q), core: [Q; 4]) -> Vec<Vec<[Q; 4]>> {
    let (u0, u1) = port_t;
    let w = u1 - u0;
    let [cn0, cn1, ct0, ct1] = core;
    // Moves along t from strip [a, a + w) until touching the core's near t-face.
    let towards_core_t = |a: Q| if a + w <= ct0 { (a + w, ct0) } else { (ct1, a) };
    let mut out = Vec::new();

    // 1. In along the normal, then turn.
    if u0 < ct1 && ct0 < u1 {
        out.push(vec![span(0, cn0, u0, u1)]);
    } else {
        let k = centred(cn0, cn1, w);
        let (t_a, t_b) = towards_core_t(u0);
        out.push(vec![span(0, k + w, u0, u1), span(k, k + w, t_a, t_b)]);
    }

    // 2. Along the face, then in.
    let j = centred(ct0, ct1, w);
    out.push(vec![
        span(0, w, u0.min(j), u1.max(j + w)),
        span(w, cn0, j, j + w),
    ]);

    // 3. Along this face's band to a side band, along it, then in towards the core.
    let k = centred(cn0, cn1, w);
    for b in [BAND_INSET, CELL - BAND_INSET - w] {
        let (t_a, t_b) = towards_core_t(b);
        out.push(vec![
            span(0, w, u0.min(b), u1.max(b + w)),
            span(w, k + w, b, b + w),
            span(k, k + w, t_a, t_b),
        ]);
    }
    out
}

fn clear_of_stair(route: &[Rect], stair: Option<Rect>) -> bool {
    stair.is_none_or(|s| route.iter().all(|r| r.dist2_rect(&s) > 0))
}

fn clear_of_trunk(route: &[Rect], trunk: Option<&Trunk>) -> bool {
    trunk.is_none_or(|t| {
        let r2 = i64::from(t.radius) * i64::from(t.radius);
        route
            .iter()
            .all(|r| r.dist2_point(t.centre_x, t.centre_z) >= r2)
    })
}

/// The bridges of one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bridges {
    pub primitives: Vec<Primitive>,
    /// False when some port had no route clear of the trunk, so the trunk piece is omitted.
    pub trunk_kept: bool,
}

/// Routes every `BridgeEnd` port of `cell` to the core at its floor height. `trunk` is the
/// trunk piece in this cell, if any; `has_deck` says whether the trunk crosses a deck.
/// Routes also avoid the infinite-staircase footprint `stair` where they can.
pub fn bridges(
    cell: CellKey,
    ports: &[Port],
    core: Rect,
    stair: Option<Rect>,
    trunk: Option<&Trunk>,
    has_deck: bool,
) -> Bridges {
    let mut chosen: Vec<(Vec<Rect>, &Port)> = Vec::new();
    let mut trunk_kept = true;
    for p in ports.iter().filter(|p| p.kind == PortKind::BridgeEnd) {
        let Some(side) = Side::of(cell, p.face) else {
            continue;
        };
        let candidates: Vec<Vec<Rect>> = routes((p.rect.u0, p.rect.u1), side.to_frame(core))
            .into_iter()
            .map(|r| r.into_iter().map(|f| side.to_cell(f)).collect())
            .collect();
        let route = candidates
            .iter()
            .find(|r| clear_of_trunk(r, trunk) && clear_of_stair(r, stair))
            .unwrap_or_else(|| {
                trunk_kept = false;
                candidates
                    .iter()
                    .find(|r| clear_of_stair(r, stair))
                    .unwrap_or(&candidates[0])
            })
            .clone();
        chosen.push((route, p));
    }

    let mut primitives = Vec::new();
    for (i, (route, port)) in chosen.iter().enumerate() {
        let floor = port.rect.v0;
        let width = port.rect.u1 - port.rect.u0;
        let flags = if width >= FAR_BRIDGE_WIDTH {
            PrimFlags::FAR_VISIBLE
        } else {
            PrimFlags::NONE
        };
        let mut rects = route.clone();
        if i == 0 && trunk_kept && !has_deck {
            rects.extend(trunk.map(|t| landing(core, t)).into_iter().flatten());
        }
        primitives.extend(
            rects
                .iter()
                .filter(|r| r.x0 < r.x1 && r.z0 < r.z1)
                .map(|r| Primitive {
                    shape: Shape::Box {
                        aabb: QAabb::new(
                            QVec3::new(r.x0, floor - BRIDGE_THICKNESS, r.z0),
                            QVec3::new(r.x1, floor, r.z1),
                        ),
                    },
                    material: MaterialId::METAL,
                    kind: FeatureKind::BRIDGE,
                    flags,
                }),
        );
    }
    Bridges {
        primitives,
        trunk_kept,
    }
}

/// An L from the core to the trunk's axis. It leaves the core along an axis on which the
/// trunk lies wholly beside the core, so the second leg never crosses the core footprint.
fn landing(core: Rect, t: &Trunk) -> [Rect; 2] {
    let beside_x = t.centre_x + LANDING_WIDTH <= core.x0 || t.centre_x - LANDING_WIDTH >= core.x1;
    if beside_x {
        l_route(core, t.centre_x, t.centre_z)
    } else {
        // Separation guarantees the trunk is beside the core in z instead.
        let swap = |r: Rect| Rect {
            x0: r.z0,
            x1: r.z1,
            z0: r.x0,
            z1: r.x1,
        };
        l_route(swap(core), t.centre_z, t.centre_x).map(swap)
    }
}

/// Along x from the core at its centre z to `x`, then along z to `z`.
fn l_route(core: Rect, x: Q, z: Q) -> [Rect; 2] {
    let half = LANDING_WIDTH / 2;
    let cz = centred(core.z0, core.z1, LANDING_WIDTH);
    let (xa, xb) = if x < core.x0 {
        (x - half, core.x0)
    } else {
        (core.x1, x + half)
    };
    let (za, zb) = if z < cz {
        (z, cz)
    } else {
        (cz + LANDING_WIDTH, z)
    };
    [
        Rect {
            x0: xa,
            x1: xb,
            z0: cz,
            z1: cz + LANDING_WIDTH,
        },
        Rect {
            x0: x - half,
            x1: x + half,
            z0: za,
            z1: zb,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips() {
        let r = Rect {
            x0: 100,
            z0: 2000,
            x1: 900,
            z1: 2600,
        };
        for side in [Side::PosX, Side::NegX, Side::PosZ, Side::NegZ] {
            assert_eq!(side.to_cell(side.to_frame(r)), r, "{side:?}");
        }
    }

    #[test]
    fn straight_route_when_aligned() {
        let core = [1000, 1400, 500, 900];
        let r = routes((600, 800), core);
        assert_eq!(r[0], vec![[0, 1000, 600, 800]]);
    }
}
