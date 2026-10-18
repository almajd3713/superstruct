//! Tram lines (`docs/spec/trams.md` §2-§4).
//!
//! A line is a pure function of the deck `LayerKey`, so every cell it crosses derives the same
//! line. Its static pieces are `NO_COLLIDE` and lie in a volume no existing level enters, so
//! none of this changes the output of `l1_cell` or `l2_cell`.

use crate::cavity::is_cavity;
use crate::cell::{Axis, CellKey, Level};
use crate::hash::hash;
use crate::key::{ColumnSpanKey, LayerKey};
use crate::l1::{CELL, DECK_REGION_LOG2, Rect, deck_holes, decks, trunk, trunk_piece};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q, Q_PER_M, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const M: Q = Q_PER_M;
const TRAM_CHANCE: f32 = 0.5;
const CANDIDATES: u64 = 28;
/// Required distance from the strip to every hole and trunk.
const HOLE_CLEAR: Q = 16 * M;
const L2_CELL: Q = 512 * M;
const STRIP_HALF: Q = 8 * M;
const ENVELOPE_HEIGHT: Q = 56;
/// Lane centres lie 4.5 m from the plane; the median is 4.5 m wide.
const LANE_OFFSET: Q = 72;
const MEDIAN_HALF: Q = 36;
const RAIL_OFFSET: Q = 24;
const RAIL_HALF: Q = 2;
const TRAVERSER_HALF: Q = 108;
const CAR_HALF: Q = 16 * M;
const END_OVERRUN: Q = 64 * M;
const STATION_SPACING_CELLS: i32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TramLineId {
    pub layer: LayerKey,
    pub deck: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TramStation {
    pub line: TramLineId,
    /// 0..=h along +s.
    pub index: u8,
    /// The L1 cell holding the station centre.
    pub cell: CellKey,
    /// Along position in quanta from the min corner of the line's first cell.
    pub s: i32,
    /// Cell-local plan rectangle: the median over the car length.
    pub boarding: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TramLine {
    pub id: TramLineId,
    pub axis: Axis,
    /// Absolute coordinate of the cell column across the axis.
    pub column: i64,
    /// The internal L2 face the line runs along, in cell-local quanta across the axis.
    pub plane: Q,
    /// Absolute first and last cell along the axis.
    pub cells: (i64, i64),
    /// Deck top in cell-local quanta.
    pub top: Q,
    pub stations: Vec<TramStation>,
    /// Turnaround points `e_A`, `e_B` in quanta along the line.
    pub ends: (i32, i32),
    pub phase_ms: u32,
}

impl TramLine {
    /// The L1 cell of the run at absolute `along` coordinate.
    fn cell_at(&self, along: i64) -> CellKey {
        let y = self.id.layer.y;
        let (x, z) = match self.axis {
            Axis::X => (along, self.column),
            _ => (self.column, along),
        };
        CellKey::new(Level::L1, glam::I64Vec3::new(x, y, z))
    }

    /// Offset of run cell `along` from the start of the line, in quanta.
    #[allow(clippy::cast_possible_truncation)] // at most four cells
    fn offset(&self, along: i64) -> i32 {
        (along - self.cells.0) as i32 * CELL
    }

    fn holds(&self, cell: CellKey) -> bool {
        let c = cell.coords;
        let (along, cross) = match self.axis {
            Axis::X => (c.x, c.z),
            _ => (c.z, c.x),
        };
        c.y == self.id.layer.y
            && cross == self.column
            && (self.cells.0..=self.cells.1).contains(&along)
    }

    /// `[lo, hi)` along quanta of the line clipped to run cell `along`, in cell-local space.
    fn clip(&self, along: i64, lo: i32, hi: i32) -> Option<(Q, Q)> {
        let off = self.offset(along);
        let (lo, hi) = ((lo - off).max(0), (hi - off).min(CELL));
        (lo < hi).then_some((lo, hi))
    }

    fn extent(&self) -> (i32, i32) {
        (self.ends.0 - CAR_HALF, self.ends.1 + CAR_HALF)
    }
}

/// The lines of deck-region layer `layer`, at most one per deck, bottom up.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // small draws
pub fn tram_lines(seed: WorldSeed, layer: LayerKey) -> Vec<TramLine> {
    let ds = decks(seed, layer);
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L1,
        layer.hash_coords(),
        salt::TRAM_LINE,
    ));
    let mut out = Vec::new();
    for (i, deck) in ds.iter().enumerate() {
        // Always the same seven draws, so a deck's line never depends on another deck's.
        let has = rng.chance(TRAM_CHANCE);
        let axis = if rng.below(2) == 0 { Axis::X } else { Axis::Z };
        let full = rng.chance(0.5);
        let a0 = rng.range_i32(0, 4);
        let b0 = rng.range_i32(a0, 4);
        let start = rng.below(CANDIDATES) as i32;
        let phase_ms = rng.below(600_000) as u32;
        if !has {
            continue;
        }
        let (a, b) = if full { (0, 3) } else { (a0, b0) };
        let (along_base, cross_base) = match axis {
            Axis::X => (layer.rx, layer.rz),
            _ => (layer.rz, layer.rx),
        };
        let id = TramLineId {
            layer,
            deck: i as u8,
        };
        let mut line = TramLine {
            id,
            axis,
            column: 0,
            plane: 0,
            cells: (4 * along_base + i64::from(a), 4 * along_base + i64::from(b)),
            top: deck.top,
            stations: Vec::new(),
            ends: (0, 0),
            phase_ms,
        };
        stations(&mut line, a, b);
        for n in 0..CANDIDATES as i32 {
            let c = (start + n).rem_euclid(CANDIDATES as i32);
            line.column = 4 * cross_base + i64::from(c / 7);
            line.plane = (c % 7 + 1) * L2_CELL;
            if clear(seed, &line) {
                let mut sts = std::mem::take(&mut line.stations);
                for st in &mut sts {
                    st.cell = line.cell_at(line.cells.0 + i64::from(st.s / CELL));
                    st.boarding = boarding(&line, st.s);
                }
                line.stations = sts;
                out.push(line);
                break;
            }
        }
    }
    out
}

/// Station positions and line ends (§2.4). Cells and rectangles are filled in once the
/// column is known.
fn stations(line: &mut TramLine, a: i32, b: i32) {
    let j0 = 1;
    let j_last = 8 * (b - a) + 6;
    let span = j_last - j0;
    let hops = (span / STATION_SPACING_CELLS).max(1);
    let at = |i: i32| L2_CELL * (j0 + i * span / hops) + L2_CELL / 2;
    line.stations = (0..=hops)
        .map(|i| TramStation {
            line: line.id,
            index: u8::try_from(i).unwrap_or(u8::MAX),
            cell: line.cell_at(line.cells.0),
            s: at(i),
            boarding: Rect {
                x0: 0,
                z0: 0,
                x1: 1,
                z1: 1,
            },
        })
        .collect();
    let (first, last) = (at(0), at(hops));
    line.ends = (first - END_OVERRUN, last + END_OVERRUN);
}

/// The median over the car length around `s`, in the local frame of the cell holding `s`.
fn boarding(line: &TramLine, s: i32) -> Rect {
    let local = s % CELL;
    let (c0, c1) = (line.plane - MEDIAN_HALF, line.plane + MEDIAN_HALF);
    let (a0, a1) = (local - CAR_HALF, local + CAR_HALF);
    match line.axis {
        Axis::X => Rect {
            x0: a0,
            z0: c0,
            x1: a1,
            z1: c1,
        },
        _ => Rect {
            x0: c0,
            z0: a0,
            x1: c1,
            z1: a1,
        },
    }
}

/// Whether the line's corridor is clear of cavities, rims, holes and trunks in every run cell.
fn clear(seed: WorldSeed, line: &TramLine) -> bool {
    let (lo, hi) = line.extent();
    (line.cells.0..=line.cells.1).all(|along| {
        let cell = line.cell_at(along);
        if !cell.in_domain() || is_cavity(seed, cell) || is_rim(seed, cell) {
            return false;
        }
        let Some((a0, a1)) = line.clip(along, lo, hi) else {
            return true;
        };
        let (c0, c1) = (line.plane - STRIP_HALF, line.plane + STRIP_HALF);
        let r = match line.axis {
            Axis::X => Rect {
                x0: a0,
                z0: c0,
                x1: a1,
                z1: c1,
            },
            _ => Rect {
                x0: c0,
                z0: a0,
                x1: c1,
                z1: a1,
            },
        };
        let need = i64::from(HOLE_CLEAR);
        if deck_holes(seed, cell)
            .iter()
            .any(|h| r.dist2_rect(h) < need * need)
        {
            return false;
        }
        if let Some(t) = trunk(seed, ColumnSpanKey::of(cell))
            && trunk_piece(&t, cell).is_some()
        {
            let need = i64::from(t.radius + HOLE_CLEAR);
            if r.dist2_point(t.centre_x, t.centre_z) < need * need {
                return false;
            }
        }
        true
    })
}

fn is_rim(seed: WorldSeed, cell: CellKey) -> bool {
    use glam::I64Vec3;
    [
        I64Vec3::X,
        I64Vec3::NEG_X,
        I64Vec3::Y,
        I64Vec3::NEG_Y,
        I64Vec3::Z,
        I64Vec3::NEG_Z,
    ]
    .into_iter()
    .any(|d| is_cavity(seed, CellKey::new(Level::L1, cell.coords + d)))
}

/// Lines whose run holds `cell`.
pub fn lines_in_cell(seed: WorldSeed, cell: CellKey) -> Vec<TramLine> {
    tram_lines(seed, LayerKey::of(cell, DECK_REGION_LOG2))
        .into_iter()
        .filter(|l| l.holds(cell))
        .collect()
}

/// The reserved volume of each line in `cell`, clipped, in cell-local quanta.
pub fn envelopes(seed: WorldSeed, cell: CellKey) -> Vec<QAabb> {
    lines_in_cell(seed, cell)
        .iter()
        .filter_map(|l| {
            let along = along_of(l, cell);
            let (lo, hi) = l.clip(along, l.extent().0, l.extent().1)?;
            Some(boxed(
                l,
                (l.plane - STRIP_HALF, l.plane + STRIP_HALF),
                (lo, hi),
                (0, ENVELOPE_HEIGHT),
            ))
        })
        .collect()
}

fn along_of(line: &TramLine, cell: CellKey) -> i64 {
    match line.axis {
        Axis::X => cell.coords.x,
        _ => cell.coords.z,
    }
}

/// A box from `cross` and `along` ranges and heights above the deck top, in cell space.
fn boxed(line: &TramLine, cross: (Q, Q), along: (Q, Q), h: (Q, Q)) -> QAabb {
    let (y0, y1) = ((line.top + h.0).min(CELL - 1), (line.top + h.1).min(CELL));
    match line.axis {
        Axis::X => QAabb::new(
            QVec3::new(along.0, y0, cross.0),
            QVec3::new(along.1, y1, cross.1),
        ),
        _ => QAabb::new(
            QVec3::new(cross.0, y0, along.0),
            QVec3::new(cross.1, y1, along.1),
        ),
    }
}

/// The static pieces of every line in `cell`, clipped and in the order of the spec's table.
pub fn tram_pieces(seed: WorldSeed, cell: CellKey) -> Vec<Primitive> {
    let mut out = Vec::new();
    for line in lines_in_cell(seed, cell) {
        let along = along_of(&line, cell);
        let (e0, e1) = line.extent();
        let p = line.plane;
        let mut add = |cross: (Q, Q), range: (i32, i32), h: (Q, Q), material: MaterialId| {
            if let Some(a) = line.clip(along, range.0, range.1) {
                out.push(Primitive {
                    shape: Shape::Box {
                        aabb: boxed(&line, cross, a, h),
                    },
                    material,
                    kind: FeatureKind::TRAM,
                    flags: PrimFlags::NO_COLLIDE,
                });
            }
        };
        for lane in [-LANE_OFFSET, LANE_OFFSET] {
            for rail in [-RAIL_OFFSET, RAIL_OFFSET] {
                let c = p + lane + rail;
                add(
                    (c - RAIL_HALF, c + RAIL_HALF),
                    (e0, e1),
                    (0, 2),
                    MaterialId::METAL,
                );
            }
        }
        for (c0, c1) in [(-36, -32), (32, 36)] {
            add((p + c0, p + c1), (e0, e1), (0, 1), MaterialId::EMISSIVE);
        }
        let median = (p - MEDIAN_HALF, p + MEDIAN_HALF);
        for st in &line.stations {
            add(
                median,
                (st.s - CAR_HALF, st.s + CAR_HALF),
                (0, 1),
                MaterialId::METAL,
            );
        }
        for st in &line.stations {
            for (a, b) in [
                (st.s - CAR_HALF, st.s - CAR_HALF + 4),
                (st.s + CAR_HALF - 4, st.s + CAR_HALF),
            ] {
                add(median, (a, b), (1, 2), MaterialId::EMISSIVE);
            }
        }
        for e in [line.ends.0, line.ends.1] {
            add(
                (p - TRAVERSER_HALF, p + TRAVERSER_HALF),
                (e - CAR_HALF, e + CAR_HALF),
                (0, 1),
                MaterialId::METAL,
            );
        }
    }
    out
}
