//! L1 face ports. Spec: `docs/spec/ports.md` §3, §4.1.
//!
//! A face's ports depend only on its key and the keyed features of the two cells beside it,
//! so both cells derive the same list.

use crate::cavity::is_cavity;
use crate::cell::{Axis, CellKey, Dir6, FaceKey, Level};
use crate::hash::hash;
use crate::key::{ColumnKey, LayerKey};
use crate::l1::{CELL, DECK_REGION_LOG2, Deck, SEPARATION, core_footprint, decks};
use crate::port::{Port, PortKind};
use crate::prim::{Q, QRect};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// Height of every L1 X/Z opening above its floor. Bridges keep 8 m from slabs, so 4 m
/// openings never meet a slab or another bridge.
pub const PORT_CLEARANCE: Q = 4 * M;
/// Bridge floors keep this far from every deck slab on both sides.
pub const BRIDGE_SLAB_CLEARANCE: Q = 8 * M;
/// Bridge deck thickness, below the floor (`levels.md` §4.5).
pub const BRIDGE_THICKNESS: Q = 4 * M;
const BRIDGE_HEIGHT_TRIES: u32 = 8;

/// The ports on `face`, in deterministic order with `id = 0, 1, …`.
pub fn face_ports(seed: WorldSeed, face: FaceKey) -> Vec<Port> {
    debug_assert_eq!(face.owner.level, Level::L1);
    let dir = match face.axis {
        Axis::X => Dir6::PosX,
        Axis::Y => Dir6::PosY,
        Axis::Z => Dir6::PosZ,
    };
    let Ok(other) = face.owner.neighbor(dir) else {
        return Vec::new();
    };
    if is_cavity(seed, face.owner) || is_cavity(seed, other) {
        return Vec::new();
    }
    let salt = match face.axis {
        Axis::X => salt::PORT_FACE_X,
        Axis::Y => salt::PORT_FACE_Y,
        Axis::Z => salt::PORT_FACE_Z,
    };
    let mut rng = Rng::new(hash(seed.0, Level::L1, face.owner.coords, salt));
    let mut rects: Vec<(PortKind, QRect)> = Vec::new();
    if face.axis == Axis::Y {
        // Same column on both sides, so the same core.
        let core = core_footprint(seed, ColumnKey::of(face.owner));
        rects.push((
            PortKind::LiftStop,
            QRect {
                u0: core.x0,
                v0: core.z0,
                u1: core.x1,
                v1: core.z1,
            },
        ));
    } else {
        side_ports(&mut rng, face.owner, other, seed, &mut rects);
    }
    rects
        .into_iter()
        .enumerate()
        .map(|(i, (kind, rect))| Port {
            face,
            #[allow(clippy::cast_possible_truncation)] // at most a handful per face
            id: i as u16,
            kind,
            rect,
        })
        .collect()
}

/// Deck continuations, then 1–3 bridge ends, on an X or Z face.
fn side_ports(
    rng: &mut Rng,
    a: CellKey,
    b: CellKey,
    seed: WorldSeed,
    out: &mut Vec<(PortKind, QRect)>,
) {
    let da = decks(seed, LayerKey::of(a, DECK_REGION_LOG2));
    let db = decks(seed, LayerKey::of(b, DECK_REGION_LOG2));
    let span = (SEPARATION, CELL - SEPARATION);
    for d in da.iter().filter(|d| db.iter().any(|e| e.top == d.top)) {
        out.push((
            PortKind::DeckContinuation,
            QRect {
                u0: span.0,
                v0: d.top,
                u1: span.1,
                v1: d.top + PORT_CLEARANCE,
            },
        ));
    }

    let slabs: Vec<&Deck> = da.iter().chain(&db).collect();
    let clear = |v0: Q| {
        slabs.iter().all(|d| {
            v0 >= d.top + BRIDGE_SLAB_CLEARANCE || v0 <= d.top - d.thickness - BRIDGE_SLAB_CLEARANCE
        })
    };
    let count = 1 + rng.below(3);
    for _ in 0..count {
        let width = rng.range_i32(2, 9) * 4 * M;
        let u0 = span.0 + 4 * M * rng.range_i32(0, (span.1 - width - span.0) / (4 * M) + 1);
        // Floor heights keep the slab below and the opening above inside the cell.
        let (lo, hi) = (
            BRIDGE_THICKNESS / (4 * M),
            (CELL - PORT_CLEARANCE) / (4 * M),
        );
        let mut v0 = None;
        for _ in 0..BRIDGE_HEIGHT_TRIES {
            let v = 4 * M * rng.range_i32(lo, hi + 1);
            if v0.is_none() && clear(v) {
                v0 = Some(v);
            }
        }
        let v0 = v0.unwrap_or_else(|| largest_gap_midpoint(&slabs));
        let rect = QRect {
            u0,
            v0,
            u1: u0 + width,
            v1: v0 + PORT_CLEARANCE,
        };
        // A bridge that would collide with an earlier port is dropped; its draws still count.
        let collides = out.iter().any(|(_, r)| {
            r.u0 < rect.u1
                && rect.u0 < r.u1
                && r.v0 - BRIDGE_THICKNESS < rect.v1
                && rect.v0 - BRIDGE_THICKNESS < r.v1
        });
        if !collides {
            out.push((PortKind::BridgeEnd, rect));
        }
    }
}

/// Midpoint, on the 4 m grid, of the largest vertical gap between the slabs of both sides and
/// the cell's floor and ceiling.
fn largest_gap_midpoint(slabs: &[&Deck]) -> Q {
    let mut edges: Vec<(Q, Q)> = slabs.iter().map(|d| (d.top - d.thickness, d.top)).collect();
    edges.sort_unstable();
    let mut best = (0, 0);
    let mut floor = 0;
    for &(bottom, top) in edges.iter().chain(&[(CELL, CELL)]) {
        if bottom - floor > best.1 - best.0 {
            best = (floor, bottom);
        }
        floor = floor.max(top);
    }
    i32::midpoint(best.0, best.1) / (4 * M) * (4 * M)
}

/// The ports on all six faces of `cell`, by face in `Dir6::ALL` order.
pub fn cell_ports(seed: WorldSeed, cell: CellKey) -> Vec<Port> {
    Dir6::ALL
        .iter()
        .filter_map(|&dir| FaceKey::of(cell, dir).ok())
        .flat_map(|face| face_ports(seed, face))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::I64Vec3;
    use proptest::prelude::*;

    fn l1(c: [i64; 3]) -> CellKey {
        CellKey::new(Level::L1, I64Vec3::from_array(c))
    }

    fn check_port_geometry(seed: WorldSeed, face: FaceKey, p: &Port) {
        let r = p.rect;
        assert!(r.u0 < r.u1 && r.v0 < r.v1, "{p:?}");
        match p.kind {
            PortKind::LiftStop => assert_eq!(face.axis, Axis::Y),
            PortKind::DeckContinuation | PortKind::BridgeEnd => {
                assert_ne!(face.axis, Axis::Y);
                assert!(
                    r.u0 >= SEPARATION && r.u1 <= CELL - SEPARATION,
                    "margin {p:?}"
                );
                assert!(r.v1 - r.v0 >= 39, "clearance ≥ 2.4 m {p:?}");
                assert!(r.v0 - BRIDGE_THICKNESS >= 0 && r.v1 <= CELL, "{p:?}");
            }
            PortKind::Walkway | PortKind::StairLanding => panic!("not an L1 port: {p:?}"),
        }
        let dir = match face.axis {
            Axis::X => Dir6::PosX,
            Axis::Y => Dir6::PosY,
            Axis::Z => Dir6::PosZ,
        };
        let sides = [face.owner, face.owner.neighbor(dir).unwrap()];
        let all: Vec<Deck> = sides
            .iter()
            .flat_map(|&c| decks(seed, LayerKey::of(c, DECK_REGION_LOG2)))
            .collect();
        match p.kind {
            PortKind::DeckContinuation => {
                for c in sides {
                    let ds = decks(seed, LayerKey::of(c, DECK_REGION_LOG2));
                    assert!(
                        ds.iter().any(|d| d.top == r.v0),
                        "floor flush on both sides"
                    );
                }
            }
            PortKind::BridgeEnd => {
                let w = r.u1 - r.u0;
                assert!((8 * M..=32 * M).contains(&w) && w % (4 * M) == 0, "{p:?}");
                for d in &all {
                    let below = r.v0 >= d.top + BRIDGE_SLAB_CLEARANCE;
                    let above = r.v1 <= d.top - d.thickness;
                    assert!(below || above, "{p:?} meets slab {d:?}");
                }
            }
            _ => {}
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        /// P0.7.4: every face between two non-cavity L1 cells carries at least one walkable
        /// port, and every port obeys the geometry rules.
        #[test]
        fn backbone_faces_carry_a_port(
            seed in any::<u64>(),
            c in prop::array::uniform3(-1_000_000i64..1_000_000),
            axis in 0usize..3,
        ) {
            let seed = WorldSeed(seed);
            let owner = l1(c);
            let face = FaceKey { owner, axis: Axis::ALL[axis] };
            let dir = [Dir6::PosX, Dir6::PosY, Dir6::PosZ][axis];
            let other = owner.neighbor(dir).unwrap();
            let ports = face_ports(seed, face);
            if is_cavity(seed, owner) || is_cavity(seed, other) {
                prop_assert!(ports.is_empty());
            } else {
                prop_assert!(!ports.is_empty(), "no port on {face:?}");
            }
            for (i, p) in ports.iter().enumerate() {
                prop_assert_eq!(usize::from(p.id), i);
                prop_assert_eq!(p.face, face);
                check_port_geometry(seed, face, p);
            }
            // Bridge ports never overlap each other, slab included.
            let bridges: Vec<_> = ports.iter().filter(|p| p.kind == PortKind::BridgeEnd).collect();
            for (i, a) in bridges.iter().enumerate() {
                for b in &bridges[i + 1..] {
                    let (a, b) = (a.rect, b.rect);
                    let apart = a.u1 <= b.u0 || b.u1 <= a.u0
                        || a.v1 <= b.v0 - BRIDGE_THICKNESS || b.v1 <= a.v0 - BRIDGE_THICKNESS;
                    prop_assert!(apart);
                }
            }
        }
    }

    /// Neighbours list the same ports for their shared face.
    #[test]
    fn neighbours_agree_on_shared_faces() {
        let seed = WorldSeed(0x9087);
        for x in -6..6 {
            for z in [-2, 5] {
                let cell = l1([x, 3, z]);
                let ports = cell_ports(seed, cell);
                for dir in [Dir6::PosX, Dir6::NegY, Dir6::PosZ] {
                    let n = cell.neighbor(dir).unwrap();
                    let face = FaceKey::of(cell, dir).unwrap();
                    let mine: Vec<_> = ports.iter().filter(|p| p.face == face).collect();
                    let theirs = cell_ports(seed, n);
                    let theirs: Vec<_> = theirs.iter().filter(|p| p.face == face).collect();
                    assert_eq!(mine, theirs, "{cell:?} {dir:?}");
                }
            }
        }
    }

    #[test]
    fn largest_gap_midpoint_known_values() {
        assert_eq!(largest_gap_midpoint(&[]), CELL / 2);
        let low = Deck {
            top: 1000 * M,
            thickness: 40 * M,
        };
        let high = Deck {
            top: 1600 * M,
            thickness: 40 * M,
        };
        // Gaps: [0, 960], [1000, 1560], [1600, 4096] m → the top one, midpoint 2848 m.
        assert_eq!(largest_gap_midpoint(&[&high, &low]), 2848 * M);
    }

    /// Deck continuations appear inside regions; bridge ends appear on every side face.
    #[test]
    fn port_mix() {
        let seed = WorldSeed(0x9087);
        let (mut dc, mut be, mut faces) = (0, 0, 0);
        for x in -40..40 {
            for y in [-3, 0, 2] {
                let face = FaceKey {
                    owner: l1([x, y, 7]),
                    axis: Axis::X,
                };
                let ports = face_ports(seed, face);
                faces += 1;
                dc += ports
                    .iter()
                    .filter(|p| p.kind == PortKind::DeckContinuation)
                    .count();
                be += ports
                    .iter()
                    .filter(|p| p.kind == PortKind::BridgeEnd)
                    .count();
            }
        }
        assert!(dc > faces / 2, "{dc} deck continuations on {faces} faces");
        assert!(be >= faces * 3 / 2, "{be} bridge ends on {faces} faces");
    }
}
