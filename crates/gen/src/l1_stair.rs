//! The infinite staircase (STAIR-01..04). Spec: `docs/spec/levels.md` §5W.35, `regimes.md` §5.2.
//!
//! A square spiral inside the 64 m footprint, the same in every L1 cell of the column: each side
//! carries one flight that climbs 32 m over 48 m, and an 8 m landing in each corner joins one
//! flight to the next. A turn takes 128 m, so a cell holds 32 turns and its top landing is the
//! bottom landing of the cell above. The flights are **open**: a thin tread for every step on two
//! stringers, with air beneath (STAIR-01). The spiral winds round a **core** (STAIR-02): glazed,
//! a lattice, or a hollow trunk; or it is a **real round spiral** of 64 treads a turn
//! (STAIR-03). The far band draws a core and four corner pylons that the near band replaces
//! (STAIR-04).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::cell::Level;
use crate::hash::hash;
use crate::key::ColumnKey;
use crate::l1::{CELL, Rect};
use crate::orient::{HelixStair, helix_stair};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = 16;
/// Steps per flight; 4 q rise and 6 q tread each.
const STEPS: Q = 128;
const RISE: Q = 4;
const TREAD: Q = 6;
const FLIGHT_RISE: Q = RISE * STEPS;
const LANDING: Q = 8 * M;
const WIDTH: Q = 4 * M;
/// Thickness of a tread plate and of a stringer, in quanta.
const PLATE: Q = 2;
const STRINGER: Q = 5;
const SLAB: Q = 8;
/// Flights per L1 cell; a multiple of four so every cell starts at the same corner.
pub const FLIGHTS: Q = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Core {
    /// A solid column faced with rows of lit windows.
    Glazed,
    /// Four posts joined by rings of beams every 32 m.
    Lattice,
    /// A square trunk with thick walls and nothing inside.
    Trunk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StairStyle {
    pub core: Core,
    /// A round spiral instead of the square one.
    pub round: bool,
}

/// The style of the staircase in L1 column `column`: a function of the column only.
pub fn style(seed: WorldSeed, column: ColumnKey) -> StairStyle {
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L1,
        glam::I64Vec3::new(column.x, 0, column.z),
        salt::STAIR_STYLE,
    ));
    let core = match rng.below(3) {
        0 => Core::Glazed,
        1 => Core::Lattice,
        _ => Core::Trunk,
    };
    StairStyle {
        core,
        round: rng.below(3) == 0,
    }
}

fn prim(shape: Shape, material: MaterialId, kind: FeatureKind, flags: PrimFlags) -> Primitive {
    Primitive {
        shape,
        material,
        kind,
        flags,
    }
}

fn boxed(lo: [Q; 3], hi: [Q; 3]) -> Shape {
    Shape::Box {
        aabb: QAabb::new(
            QVec3::new(lo[0], lo[1], lo[2]),
            QVec3::new(hi[0], hi[1], hi[2]),
        ),
    }
}

fn metal(shape: Shape) -> Primitive {
    prim(
        shape,
        MaterialId::METAL,
        FeatureKind::STAIR_TOWER,
        PrimFlags::NONE,
    )
}

/// One open flight: `STEPS` treads on stringers. `dir` is the way up; `lo` the strip's low
/// corner in the plane, `y` the flight's base.
fn flight(out: &mut Vec<Primitive>, k: Q, foot: Rect) {
    let y = k * FLIGHT_RISE;
    debug_assert_eq!(foot.x1 - foot.x0 - 2 * LANDING, TREAD * STEPS);
    let (x_lo, x_hi) = (foot.x0 + LANDING, foot.x1 - LANDING);
    let (z_lo, z_hi) = (foot.z0 + LANDING, foot.z1 - LANDING);
    // Along the flight: position `a` metres-in-quanta from its start; across: `c`.
    // (origin of the run, sign of the run, runs along x?, origin across)
    let (start, sign, along_x, across) = match k % 4 {
        0 => (x_lo, 1, true, foot.z0),
        1 => (z_lo, 1, false, foot.x1 - WIDTH),
        2 => (x_hi, -1, true, foot.z1 - WIDTH),
        _ => (z_hi, -1, false, foot.x0),
    };
    let at = |a0: Q, a1: Q, c0: Q, c1: Q, y0: Q, y1: Q| {
        let (p0, p1) = if sign > 0 {
            (start + a0, start + a1)
        } else {
            (start - a1, start - a0)
        };
        if along_x {
            boxed([p0, y0, c0], [p1, y1, c1])
        } else {
            boxed([c0, y0, p0], [c1, y1, p1])
        }
    };
    for j in 0..STEPS {
        let top = y + (j + 1) * RISE;
        out.push(metal(at(
            j * TREAD,
            (j + 1) * TREAD,
            across,
            across + WIDTH,
            top - PLATE,
            top,
        )));
    }
    // A stringer under each edge, stepped in groups of four treads.
    for g in 0..STEPS / 4 {
        let first = y + (4 * g + 1) * RISE;
        let last = y + (4 * g + 4) * RISE;
        for c in [across, across + WIDTH - STRINGER] {
            out.push(metal(at(
                g * 4 * TREAD,
                (g + 1) * 4 * TREAD,
                c,
                c + STRINGER,
                (first - 3 * RISE).max(y),
                last - PLATE,
            )));
        }
    }
}

/// The corner landings of the square spiral: landing `k` sits where flight `k` starts, with its
/// top at the flight's base.
fn landings(out: &mut Vec<Primitive>, foot: Rect) {
    for k in 1..=FLIGHTS {
        let y = k * FLIGHT_RISE;
        let (x0, z0) = match k % 4 {
            0 => (foot.x0, foot.z0),
            1 => (foot.x1 - LANDING, foot.z0),
            2 => (foot.x1 - LANDING, foot.z1 - LANDING),
            _ => (foot.x0, foot.z1 - LANDING),
        };
        out.push(metal(boxed(
            [x0, y - SLAB, z0],
            [x0 + LANDING, y, z0 + LANDING],
        )));
    }
}

/// A round spiral: 64 treads a turn of 0.4375 m rise, 4 m wide on a radius of 24 m, round the
/// footprint's middle, with a landing every 128 treads that the sides of the footprint meet.
fn round_spiral(out: &mut Vec<Primitive>, foot: Rect) {
    let (cx, cz) = (
        i32::midpoint(foot.x0, foot.x1),
        i32::midpoint(foot.z0, foot.z1),
    );
    let rise = 7;
    // 64 steps a turn and 7 q each is 28 m a turn; a whole number of turns fills the cell.
    let per_turn = 64;
    let turns = CELL / (per_turn * rise);
    let clip = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(CELL, CELL, CELL));
    let h = HelixStair {
        axis: QVec3::new(cx, 0, cz),
        inner: 24 * M,
        width: WIDTH,
        per_turn: per_turn as u8,
        steps: (turns * per_turn) as u32,
        rise,
        thick: PLATE,
        start: 0,
        sense: 1,
        material: MaterialId::METAL,
        kind: FeatureKind::STAIR_TOWER,
        flags: PrimFlags::NONE,
    };
    out.extend(helix_stair(&h, clip));
}

/// The core of the spiral, and the far silhouette that stands for it from afar.
fn core(out: &mut Vec<Primitive>, foot: Rect, core: Core) {
    let (cx, cz) = (
        i32::midpoint(foot.x0, foot.x1),
        i32::midpoint(foot.z0, foot.z1),
    );
    let half = 16 * M;
    let (x0, x1, z0, z1) = (cx - half, cx + half, cz - half, cz + half);
    // The silhouette: the whole column, and a pylon at each corner of the footprint, in the
    // far band only (STAIR-04).
    let far = PrimFlags::FAR_VISIBLE | PrimFlags::FAR_ONLY | PrimFlags::NO_COLLIDE;
    out.push(prim(
        boxed([x0, 0, z0], [x1, CELL, z1]),
        MaterialId::METAL,
        FeatureKind::STAIR_TOWER,
        far,
    ));
    for (px, pz) in [
        (foot.x0, foot.z0),
        (foot.x1 - 32 * M, foot.z0),
        (foot.x0, foot.z1 - 32 * M),
        (foot.x1 - 32 * M, foot.z1 - 32 * M),
    ] {
        out.push(prim(
            boxed([px, 0, pz], [px + 32 * M, CELL, pz + 32 * M]),
            MaterialId::METAL,
            FeatureKind::STAIR_TOWER,
            far,
        ));
    }
    let wall = |out: &mut Vec<Primitive>, lo: [Q; 3], hi: [Q; 3]| {
        out.push(prim(
            boxed(lo, hi),
            MaterialId::CONCRETE,
            FeatureKind::STAIR_TOWER,
            PrimFlags::NONE,
        ));
    };
    match core {
        Core::Glazed => {
            wall(out, [x0, 0, z0], [x1, CELL, z1]);
            // Rows of lit panes on the four faces, one every 32 m, 2 m tall.
            for k in 0..CELL / (32 * M) {
                let y = k * 32 * M + 8 * M;
                for (lo, hi) in [
                    ([x0 - 2, y, z0 + 4 * M], [x0, y + 2 * M, z1 - 4 * M]),
                    ([x1, y, z0 + 4 * M], [x1 + 2, y + 2 * M, z1 - 4 * M]),
                    ([x0 + 4 * M, y, z0 - 2], [x1 - 4 * M, y + 2 * M, z0]),
                    ([x0 + 4 * M, y, z1], [x1 - 4 * M, y + 2 * M, z1 + 2]),
                ] {
                    out.push(prim(
                        boxed(lo, hi),
                        MaterialId::LIT_WINDOW,
                        FeatureKind::WINDOW,
                        PrimFlags::NO_COLLIDE,
                    ));
                }
            }
        }
        Core::Lattice => {
            for (px, pz) in [(x0, z0), (x1 - 24, z0), (x0, z1 - 24), (x1 - 24, z1 - 24)] {
                wall(out, [px, 0, pz], [px + 24, CELL, pz + 24]);
            }
            for k in 0..CELL / (32 * M) {
                let y = k * 32 * M;
                wall(out, [x0, y, z0], [x1, y + 16, z0 + 16]);
                wall(out, [x0, y, z1 - 16], [x1, y + 16, z1]);
                wall(out, [x0, y, z0], [x0 + 16, y + 16, z1]);
                wall(out, [x1 - 16, y, z0], [x1, y + 16, z1]);
            }
        }
        Core::Trunk => {
            let t = 4 * M;
            wall(out, [x0, 0, z0], [x1, CELL, z0 + t]);
            wall(out, [x0, 0, z1 - t], [x1, CELL, z1]);
            wall(out, [x0, 0, z0 + t], [x0 + t, CELL, z1 - t]);
            wall(out, [x1 - t, 0, z0 + t], [x1, CELL, z1 - t]);
        }
    }
}

/// The staircase of one L1 cell in the column, in cell-local quanta.
pub fn stair_pieces(foot: Rect, style: StairStyle) -> Vec<Primitive> {
    let mut out = Vec::new();
    debug_assert_eq!(foot.x1 - foot.x0, 64 * M);
    if style.round {
        round_spiral(&mut out, foot);
    } else {
        for k in 0..FLIGHTS {
            flight(&mut out, k, foot);
        }
        landings(&mut out, foot);
    }
    core(&mut out, foot, style.core);
    out
}

/// The volume the staircase keeps clear of everything else in its cell: the footprint, floor to
/// ceiling.
pub fn keep_out(foot: Rect) -> QAabb {
    QAabb::new(
        QVec3::new(foot.x0, 0, foot.z0),
        QVec3::new(foot.x1, CELL, foot.z1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foot() -> Rect {
        Rect::square(100 * M, 200 * M, 64 * M)
    }

    #[test]
    fn the_spiral_stays_in_the_cell_and_in_its_footprint() {
        let f = foot();
        for style in [
            StairStyle {
                core: Core::Glazed,
                round: false,
            },
            StairStyle {
                core: Core::Lattice,
                round: true,
            },
            StairStyle {
                core: Core::Trunk,
                round: false,
            },
        ] {
            for p in stair_pieces(f, style) {
                let b = p.aabb();
                assert!(b.min.y >= 0 && b.max.y <= CELL, "{style:?} {b:?}");
                assert!(
                    b.min.x >= f.x0 - 4
                        && b.max.x <= f.x1 + 4
                        && b.min.z >= f.z0 - 4
                        && b.max.z <= f.z1 + 4,
                    "{style:?} {b:?}"
                );
            }
        }
    }

    /// STAIR-01: treads have air beneath: no solid box under a tread is taller than a
    /// stringer, and the flights are open (treads, not a solid wedge).
    #[test]
    fn flights_are_open_treads_on_stringers() {
        let prims = stair_pieces(
            foot(),
            StairStyle {
                core: Core::Lattice,
                round: false,
            },
        );
        let wedges = prims
            .iter()
            .filter(|p| matches!(p.shape, Shape::StairRun { .. }))
            .count();
        assert_eq!(wedges, 0);
        let treads = prims
            .iter()
            .filter(|p| {
                let b = p.aabb();
                b.max.y - b.min.y == PLATE
                    && (b.max.x - b.min.x == TREAD || b.max.z - b.min.z == TREAD)
            })
            .count();
        assert_eq!(treads as Q, FLIGHTS * STEPS);
    }

    /// The top tread of flight `k` meets landing `k + 1` at its own height, and flight `k + 1`
    /// starts from that landing: the chain climbs from the cell's bottom to its top.
    #[test]
    fn every_flight_is_connected_to_the_next() {
        let f = foot();
        let mut land = Vec::new();
        landings(&mut land, f);
        for k in 0..FLIGHTS - 1 {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            flight(&mut a, k, f);
            flight(&mut b, k + 1, f);
            let last = a[STEPS as usize - 1].aabb();
            let first = b[0].aabb();
            let landing = land[k as usize].aabb();
            assert_eq!(last.max.y, landing.max.y, "flight {k} ends at its landing");
            assert_eq!(
                first.max.y,
                landing.max.y + RISE,
                "flight {} starts from it",
                k + 1
            );
            // The last tread touches the landing and the next flight's first tread does too.
            let touches = |x: &QAabb, y: &QAabb| {
                x.min.x <= y.max.x && y.min.x <= x.max.x && x.min.z <= y.max.z && y.min.z <= x.max.z
            };
            assert!(touches(&last, &landing), "flight {k}");
            assert!(touches(&first, &landing), "flight {}", k + 1);
        }
    }

    #[test]
    fn the_far_silhouette_is_far_sized_and_only_far() {
        let prims = stair_pieces(
            foot(),
            StairStyle {
                core: Core::Glazed,
                round: false,
            },
        );
        let far: Vec<_> = prims
            .iter()
            .filter(|p| p.flags.contains(PrimFlags::FAR_ONLY))
            .collect();
        assert_eq!(far.len(), 5);
        for p in far {
            assert!(
                p.flags
                    .contains(PrimFlags::NO_COLLIDE | PrimFlags::FAR_VISIBLE)
            );
            let b = p.aabb();
            assert!(b.max.x - b.min.x >= 32 * M && b.max.z - b.min.z >= 32 * M);
        }
    }

    #[test]
    fn a_round_spiral_has_whole_turns() {
        let prims = stair_pieces(
            foot(),
            StairStyle {
                core: Core::Trunk,
                round: true,
            },
        );
        // 64 treads a turn, as many turns as fit the cell; the core adds its few pieces.
        let treads = prims.len() - 9;
        assert_eq!(treads as i32, CELL / (64 * 7) * 64);
    }
}

#[cfg(test)]
mod web_tests {
    use super::*;
    use crate::cell::CellKey;
    use crate::key::ColumnKey;
    use crate::l1_web::l1_cell;
    use crate::prim::ChildConstraint;
    use crate::regime::rare_stair;
    use glam::I64Vec3;

    /// A staircase column: the cell has the open flights and the far silhouette, nothing else
    /// stands in the footprint, the children keep out of it, and the skeleton ties its cells.
    #[test]
    fn a_staircase_column_is_reserved_and_tied() {
        let seed = WorldSeed(0x57A1);
        let mut checked = 0;
        'search: for x in -60..60 {
            for z in -60..60 {
                let l0 = ColumnKey {
                    level: Level::L0,
                    x,
                    z,
                };
                let Some(st) = rare_stair(seed, l0) else {
                    continue;
                };
                let column = ColumnKey {
                    level: Level::L1,
                    x: x * 8 + i64::from(st.l1_x),
                    z: z * 8 + i64::from(st.l1_z),
                };
                let Some(foot) = crate::l1::stair_footprint(seed, column) else {
                    continue;
                };
                let cell = CellKey::new(Level::L1, I64Vec3::new(column.x, 1, column.z));
                if crate::cavity::CavityView::new(seed, cell).is_cavity(cell) {
                    continue;
                }
                let out = l1_cell(seed, cell);
                let keep = keep_out(foot);
                assert!(
                    out.child_constraints
                        .contains(&ChildConstraint::KeepOut { aabb: keep })
                );
                let mut far_only = 0;
                for p in &out.primitives {
                    let own = matches!(p.kind, FeatureKind::STAIR_TOWER | FeatureKind::WINDOW);
                    // The far mass of the fill (HOR-03) is far-only too, but is not the staircase's.
                    if p.flags.contains(PrimFlags::FAR_ONLY) && own {
                        far_only += 1;
                    }
                    if !own {
                        let inside = crate::l1_web::enters(p, &keep);
                        assert!(
                            !inside,
                            "{:?} stands in the staircase footprint at {column:?}",
                            p.kind
                        );
                    }
                }
                assert_eq!(far_only, 5);
                let sk = crate::l1_artery::skeleton(seed, cell.coords).unwrap();
                let in_tree = |c: [i64; 3]| {
                    sk.tree
                        .contains(&u16::try_from(c[0] + 8 * c[1] + 64 * c[2]).unwrap())
                };
                let l2 = |q: i32| i64::from(q / 16).div_euclid(512);
                let mut any = false;
                for z in l2(foot.z0)..=l2(foot.z1 - 1) {
                    for y in 0..8 {
                        for x in l2(foot.x0)..=l2(foot.x1 - 1) {
                            any |= in_tree([x, y, z]);
                        }
                    }
                }
                assert!(any, "{column:?}: no cell of the column is on the tree");
                checked += 1;
                if checked == 3 {
                    break 'search;
                }
            }
        }
        assert!(checked > 0, "no staircase column found");
    }
}
