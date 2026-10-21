//! The skeleton of a live L1 cell: face ports, port arteries, loops and ties
//! (ART-01..03). Spec: `docs/spec/levels.md` §4W.5, `docs/spec/ports.md` §6W.
//!
//! The skeleton is a tree `T` of L2 cells of the 8×8×8 grid. Every walk ends in a cell it
//! already shares with `T` and a BFS finishes walks that run out of steps, so `T` is connected
//! by construction (Lemma A). Nothing here reads a generated piece's shape beyond its bounds.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::collections::VecDeque;

use glam::I64Vec3;

use crate::cavity::CavityView;
use crate::cell::{Axis, CellKey, Dir6, FaceKey, Level};
use crate::hash::hash;
use crate::l1_between::{STR_DRIP, STR_ISLAND, STR_SPIRE, STR_TETHER, between_content};
use crate::l1_frame::{BeamKind, beams_of_cell, knot};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Stratum, stack};

/// L2 cells per side of an L1 cell.
const N: i64 = 8;
const CELLS: usize = 512;
/// Local L2 cell coordinates.
pub type L2c = [i64; 3];

/// Every number that sets how dense the skeleton is. Changing one changes generator output:
/// bump `GEN_VERSION` and regenerate the goldens. `skeleton_census` prints their effect.
pub mod tuning {
    /// Ports on a face: `PORTS_MIN + below(PORTS_SPREAD)`.
    pub const PORTS_MIN: u64 = 2;
    pub const PORTS_SPREAD: u64 = 3;
    /// Loops per cell: `below(LOOPS_SPREAD)`.
    pub const LOOPS_SPREAD: u64 = 3;
    /// A loop joins two cells at least this far apart (Chebyshev); closer pairs are dropped.
    pub const LOOP_MIN_SPAN: i64 = 3;
    /// A walk gets `BUDGET_PER_DIST * dist + BUDGET_BASE` random steps before the BFS.
    pub const BUDGET_PER_DIST: i64 = 4;
    pub const BUDGET_BASE: i64 = 8;
    /// Weight multiplier of a move that gets closer to the target.
    pub const TOWARD_BIAS: i64 = 4;
    /// Ties: which kinds of feature get one.
    pub const TIE_BEAMS: bool = true;
    pub const TIE_CRUSTS: bool = true;
    pub const TIE_BETWEEN: bool = true;
}

/// L1 face order of the skeleton: `−x, +x, −y, +y, −z, +z` (`levels.md` §4W.5.2).
const FACE_ORDER: [Dir6; 6] = [
    Dir6::NegX,
    Dir6::PosX,
    Dir6::NegY,
    Dir6::PosY,
    Dir6::NegZ,
    Dir6::PosZ,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ArteryType {
    GrandStair,
    RampRoad,
    LiftTrunk,
    Avenue,
    TramTube,
    CatwalkWeb,
    BeamCorridor,
    LadderChain,
    GravityGate,
}

impl ArteryType {
    /// Every type, in the order that the port draw weighs them.
    pub const ALL: [ArteryType; 9] = [
        Self::GrandStair,
        Self::RampRoad,
        Self::LiftTrunk,
        Self::Avenue,
        Self::TramTube,
        Self::CatwalkWeb,
        Self::BeamCorridor,
        Self::LadderChain,
        Self::GravityGate,
    ];

    /// Whether a port on a face of `axis` may take the type (`levels.md` §4W.5.3). Beam
    /// corridors and gravity gates also need context, see `face_artery_ports`.
    pub const fn allowed(self, axis: Axis) -> bool {
        match self {
            Self::GrandStair | Self::CatwalkWeb | Self::BeamCorridor | Self::GravityGate => true,
            Self::LiftTrunk | Self::LadderChain => matches!(axis, Axis::Y),
            Self::RampRoad | Self::Avenue | Self::TramTube => !matches!(axis, Axis::Y),
        }
    }

    /// The types a loop between two cells may take: those that need no context.
    pub const fn offered(axis: Axis) -> &'static [ArteryType] {
        match axis {
            Axis::Y => &[
                Self::GrandStair,
                Self::LiftTrunk,
                Self::CatwalkWeb,
                Self::LadderChain,
            ],
            Axis::X | Axis::Z => &[
                Self::GrandStair,
                Self::RampRoad,
                Self::Avenue,
                Self::TramTube,
                Self::CatwalkWeb,
            ],
        }
    }
}

/// A port on an L1 face: an L2 cell of the face and the type of the artery that crosses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArteryPort {
    pub face: FaceKey,
    pub id: u8,
    /// Position on the face grid: X faces `(y, z)`, Y faces `(x, z)`, Z faces `(x, y)`.
    pub ij: [u8; 2],
    pub kind: ArteryType,
}

impl ArteryPort {
    /// The L2 cell of the port next to the face, in the L1 cell on its low side (`high`: the
    /// cell at the high end of the axis, index 7) or on its high side (the cell at index 0).
    pub fn l2(&self, high: bool) -> L2c {
        let (i, j) = (i64::from(self.ij[0]), i64::from(self.ij[1]));
        let k = if high { N - 1 } else { 0 };
        match self.face.axis {
            Axis::X => [k, i, j],
            Axis::Y => [i, k, j],
            Axis::Z => [i, j, k],
        }
    }
}

/// The ports on `face`: 2–4 distinct L2 cells of the face grid, none if either side is a
/// cavity cell. Depends on the face key only, so both cells derive the same list.
pub fn face_artery_ports(seed: WorldSeed, cavity: &CavityView, face: FaceKey) -> Vec<ArteryPort> {
    ports_with(seed, cavity, face, near_well(seed, face))
}

/// Whether the gravity field at the face centre differs from the default: a well reaches it.
fn near_well(seed: WorldSeed, face: FaceKey) -> bool {
    let o = face.owner.coords;
    let side = 4096;
    let mut c = [
        o.x * side + side / 2,
        o.y * side + side / 2,
        o.z * side + side / 2,
    ];
    let k = match face.axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    c[k] += side / 2;
    crate::gravity::field_at_metres(seed, c) != crate::gravity::Gravity::DEFAULT
}

/// The cells `(i, j)` of `face` that a beam crosses: the beam has an L2 cell on the face in
/// both L1 cells. Conduits are too thin to live in. Depends on the face key only.
fn beam_cells(seed: WorldSeed, face: FaceKey, other: CellKey) -> Vec<[u8; 2]> {
    let k = Axis::ALL.iter().position(|&a| a == face.axis).unwrap_or(0);
    let on_face = |l1: I64Vec3, layer: u8| {
        let mut out: Vec<[u8; 2]> = Vec::new();
        for beam in beams_of_cell(seed, l1) {
            if beam.kind == BeamKind::Conduit {
                continue;
            }
            for c in beam.l2_cells(l1).into_iter().filter(|c| c[k] == layer) {
                let ij = match face.axis {
                    Axis::X => [c[1], c[2]],
                    Axis::Y => [c[0], c[2]],
                    Axis::Z => [c[0], c[1]],
                };
                if !out.contains(&ij) {
                    out.push(ij);
                }
            }
        }
        out
    };
    let low = on_face(face.owner.coords, 7);
    let mut high = on_face(other.coords, 0);
    high.retain(|ij| low.contains(ij));
    high
}

/// `face_artery_ports` with the well context given. The draws are the same whatever the
/// context: types are weighted 0 when not offered, never skipped.
fn ports_with(seed: WorldSeed, cavity: &CavityView, face: FaceKey, gate: bool) -> Vec<ArteryPort> {
    debug_assert_eq!(face.owner.level, Level::L1);
    let dir = match face.axis {
        Axis::X => Dir6::PosX,
        Axis::Y => Dir6::PosY,
        Axis::Z => Dir6::PosZ,
    };
    let Ok(other) = face.owner.neighbor(dir) else {
        return Vec::new();
    };
    if cavity.is_cavity(face.owner) || cavity.is_cavity(other) {
        return Vec::new();
    }
    let salt = match face.axis {
        Axis::X => salt::ARTERY_FACE_X,
        Axis::Y => salt::ARTERY_FACE_Y,
        Axis::Z => salt::ARTERY_FACE_Z,
    };
    let mut rng = Rng::new(hash(seed.0, Level::L1, face.owner.coords, salt));
    let count = (tuning::PORTS_MIN + rng.below(tuning::PORTS_SPREAD)) as usize;
    let beams = beam_cells(seed, face, other);
    let mut out: Vec<ArteryPort> = Vec::with_capacity(count);
    while out.len() < count {
        let ij = [rng.below(N as u64) as u8, rng.below(N as u64) as u8];
        if out.iter().any(|p| p.ij == ij) {
            continue;
        }
        let weights = ArteryType::ALL.map(|t| {
            let ok = t.allowed(face.axis)
                && match t {
                    ArteryType::BeamCorridor => beams.contains(&ij),
                    ArteryType::GravityGate => gate,
                    _ => true,
                };
            u32::from(ok)
        });
        let kind = ArteryType::ALL[rng.pick_weighted(&weights).unwrap_or(5)];
        out.push(ArteryPort {
            face,
            id: out.len() as u8,
            ij,
            kind,
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TieKind {
    Beam,
    Crust,
    Between,
    /// The L2 cells of an infinite staircase's footprint column.
    Stair,
    /// The L2 cells round a rare event's geometry.
    Rare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// An artery from the port `id` of the face on side `dir`.
    Port {
        dir: Dir6,
        id: u8,
    },
    Loop,
    Tie(TieKind),
}

/// A walk, as the 6-connected sequence of L2 cells it occupies (indices `x + 8y + 64z`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artery {
    pub kind: ArteryType,
    pub role: Role,
    pub cells: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    /// The knot's L2 cell.
    pub knot: u16,
    /// The knot's hub family (`l1_frame::HUB_WEIGHTS`): junction hall, lift nexus, machine,
    /// gravity furnace, sanctuary site.
    pub knot_hub: u8,
    /// Cells of `T` in insertion order.
    pub tree: Vec<u16>,
    pub arteries: Vec<Artery>,
    /// Ports of the six faces in the order of the walks, with the face side they sit on.
    pub ports: Vec<(Dir6, ArteryPort)>,
}

impl Skeleton {
    /// Every L2 face crossed by an artery, as `(from, to, type)`. Both L2 cells read the same
    /// record, which is the `ForcedPort` of `levels.md` §4W.5.2.
    pub fn links(&self) -> impl Iterator<Item = (u16, u16, ArteryType)> + '_ {
        self.arteries
            .iter()
            .flat_map(|a| a.cells.windows(2).map(|w| (w[0], w[1], a.kind)))
    }
}

pub const fn index(c: L2c) -> usize {
    (c[0] + N * c[1] + N * N * c[2]) as usize
}

pub const fn coords(i: usize) -> L2c {
    let i = i as i64;
    [i % N, (i / N) % N, i / (N * N)]
}

fn cheb(a: L2c, b: L2c) -> i64 {
    (0..3).map(|k| (a[k] - b[k]).abs()).max().unwrap_or(0)
}

#[derive(Clone, Copy)]
enum Goal {
    /// Any cell of the tree.
    Tree,
    Cell(L2c),
}

/// The walk from `start` to `goal` (spec §4W.5.2 step 1): a biased random walk, then a BFS.
/// Returns the axis-step path, start and end included. Draws nothing when `start` is the goal.
fn walk(
    rng: &mut Rng,
    weave: u8,
    radial: Option<[i64; 3]>,
    start: L2c,
    goal: Goal,
    tree: &[bool; CELLS],
    list: &[u16],
) -> Vec<L2c> {
    let reached = |c: L2c| match goal {
        Goal::Tree => tree[index(c)],
        Goal::Cell(g) => c == g,
    };
    if reached(start) {
        return vec![start];
    }
    let target = match goal {
        Goal::Cell(g) => g,
        Goal::Tree => list
            .iter()
            .map(|&i| coords(i as usize))
            .min_by_key(|&c| (cheb(start, c), index(c)))
            .unwrap_or(start),
    };
    let weave = i64::from(weave);
    let w_by_axes = [0, (255 - weave).max(1), weave / 2, weave / 4];
    let budget = tuning::BUDGET_PER_DIST * cheb(start, target) + tuning::BUDGET_BASE;
    let mut visited = [false; CELLS];
    visited[index(start)] = true;
    let mut cur = start;
    let mut landings = vec![start];
    let mut done = false;
    for _ in 0..budget {
        let mut moves: Vec<L2c> = Vec::with_capacity(26);
        let mut weights: Vec<u32> = Vec::with_capacity(26);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let axes = [dx, dy, dz].iter().filter(|&&d| d != 0).count();
                    if axes == 0 {
                        continue;
                    }
                    let n = [cur[0] + dx, cur[1] + dy, cur[2] + dz];
                    if n.iter().any(|c| !(0..N).contains(c)) || visited[index(n)] {
                        continue;
                    }
                    let mut w = w_by_axes[axes];
                    if cheb(n, target) < cheb(cur, target) {
                        w *= tuning::TOWARD_BIAS;
                    }
                    if let Some(up) = radial {
                        w *= radial_factor([dx, dy, dz], up);
                    }
                    moves.push(n);
                    weights.push(w as u32);
                }
            }
        }
        let Some(pick) = rng.pick_weighted(&weights) else {
            break;
        };
        cur = moves[pick];
        visited[index(cur)] = true;
        landings.push(cur);
        if reached(cur) {
            done = true;
            break;
        }
    }
    if !done {
        landings.extend(bfs(cur, &reached).into_iter().skip(1));
    }
    expand(&landings)
}

/// Shortest 6-neighbour path from `from` to the goal cell with the lowest index among the
/// nearest. The grid is fully passable, so a path always exists.
fn bfs(from: L2c, reached: &dyn Fn(L2c) -> bool) -> Vec<L2c> {
    let mut dist = [u16::MAX; CELLS];
    let mut parent = [u16::MAX; CELLS];
    let mut queue = VecDeque::new();
    dist[index(from)] = 0;
    queue.push_back(from);
    let mut best: Option<(u16, usize)> = None;
    while let Some(c) = queue.pop_front() {
        let d = dist[index(c)];
        if best.is_some_and(|(bd, _)| d > bd) {
            break;
        }
        if reached(c) {
            let cand = (d, index(c));
            if best.is_none_or(|b| cand < b) {
                best = Some(cand);
            }
        }
        for dir in Dir6::ALL {
            let o = dir.offset();
            let n = [c[0] + o.x, c[1] + o.y, c[2] + o.z];
            if n.iter().any(|v| !(0..N).contains(v)) || dist[index(n)] != u16::MAX {
                continue;
            }
            dist[index(n)] = d + 1;
            parent[index(n)] = index(c) as u16;
            queue.push_back(n);
        }
    }
    let mut path = Vec::new();
    if let Some((_, goal)) = best {
        let mut i = goal;
        loop {
            path.push(coords(i));
            if i == index(from) {
                break;
            }
            i = parent[i] as usize;
        }
        path.reverse();
    }
    path
}

/// Replaces each diagonal move by its supporting axis path: x, then y, then z.
fn expand(landings: &[L2c]) -> Vec<L2c> {
    let mut out = vec![landings[0]];
    for w in landings.windows(2) {
        let mut c = w[0];
        for k in 0..3 {
            while c[k] != w[1][k] {
                c[k] += (w[1][k] - c[k]).signum();
                out.push(c);
            }
        }
    }
    out
}

/// GRV-03: three times the weight for a move within 30° of the local up (radial) or within
/// 30° of perpendicular to it (along a shell), by integer dot products.
fn radial_factor(d: [i64; 3], up: [i64; 3]) -> i64 {
    let dd: i64 = d.iter().map(|v| v * v).sum();
    let uu: i64 = up.iter().map(|v| v * v).sum();
    let du: i64 = d.iter().zip(up).map(|(a, b)| a * b).sum();
    let c2 = 4 * du * du;
    if c2 >= 3 * dd * uu || c2 <= dd * uu {
        3
    } else {
        1
    }
}

/// The local up at the centre of L1 cell `l1` when a well bends the field there.
fn well_up(seed: WorldSeed, l1: I64Vec3) -> Option<[i64; 3]> {
    let c = [0, 1, 2].map(|i| l1[i] * 4096 + 2048);
    let g = crate::gravity::field_at_metres(seed, c);
    (g != crate::gravity::Gravity::DEFAULT && !g.is_lull()).then(|| g.up.map(i64::from))
}

struct Builder {
    rng: Rng,
    weave: u8,
    radial: Option<[i64; 3]>,
    tree: [bool; CELLS],
    list: Vec<u16>,
    arteries: Vec<Artery>,
}

impl Builder {
    fn add(&mut self, c: L2c) {
        if !self.tree[index(c)] {
            self.tree[index(c)] = true;
            self.list.push(index(c) as u16);
        }
    }

    fn run(&mut self, start: L2c, goal: Goal, kind: ArteryType, role: Role) {
        let path = walk(
            &mut self.rng,
            self.weave,
            self.radial,
            start,
            goal,
            &self.tree,
            &self.list,
        );
        for &c in &path {
            self.add(c);
        }
        if path.len() > 1 {
            self.arteries.push(Artery {
                kind,
                role,
                cells: path.iter().map(|&c| index(c) as u16).collect(),
            });
        }
    }

    /// The tie of a feature that occupies `cells`, started from the one closest to `knot`.
    fn tie(&mut self, knot: L2c, cells: &[L2c], kind: TieKind) {
        let Some(&start) = cells.iter().min_by_key(|&&c| (cheb(c, knot), index(c))) else {
            return;
        };
        self.run(start, Goal::Tree, ArteryType::CatwalkWeb, Role::Tie(kind));
    }
}

/// The skeleton of L1 cell `l1`, or `None` for a cavity cell.
#[allow(clippy::too_many_lines)] // one pipeline, read top to bottom
pub fn skeleton(seed: WorldSeed, l1: I64Vec3) -> Option<Skeleton> {
    let key = CellKey::new(Level::L1, l1);
    let cavity = CavityView::new(seed, key);
    if cavity.is_cavity(key) {
        return None;
    }
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let weave = regime(seed, l0).web.weave;
    let beams = beams_of_cell(seed, l1);
    let knot_of = knot(seed, l1, &beams);
    let k = knot_of.l2;
    let mut b = Builder {
        rng: Rng::new(hash(seed.0, Level::L1, l1, salt::ARTERY)),
        weave,
        radial: well_up(seed, l1),
        tree: [false; CELLS],
        list: Vec::new(),
        arteries: Vec::new(),
    };
    b.add(k);

    // Port arteries.
    let mut ports = Vec::new();
    for dir in FACE_ORDER {
        let Ok(face) = FaceKey::of(key, dir) else {
            continue;
        };
        for p in face_artery_ports(seed, &cavity, face) {
            ports.push((dir, p));
        }
    }
    for &(dir, p) in &ports {
        // The L2 cell of this L1 cell that touches the face: the high end (7) of an axis for
        // a positive face, the low end (0) for a negative one.
        let start = p.l2(dir.is_positive());
        b.run(start, Goal::Tree, p.kind, Role::Port { dir, id: p.id });
    }

    // Loops.
    for _ in 0..b.rng.below(tuning::LOOPS_SPREAD) {
        let n = b.list.len() as u64;
        let (mut i, mut j) = (b.rng.below(n) as usize, b.rng.below(n) as usize);
        if i > j {
            std::mem::swap(&mut i, &mut j);
        }
        let (a, c) = (coords(b.list[i] as usize), coords(b.list[j] as usize));
        let d = [0, 1, 2].map(|k| (a[k] - c[k]).abs());
        let axis = if d[1] >= d[0] && d[1] >= d[2] {
            Axis::Y
        } else {
            Axis::X
        };
        let offered = ArteryType::offered(axis);
        let kind = offered[b.rng.below(offered.len() as u64) as usize];
        if cheb(a, c) >= tuning::LOOP_MIN_SPAN {
            b.run(a, Goal::Cell(c), kind, Role::Loop);
        }
    }

    // Ties: beams (by line index), crusts (by k), Between content (in generation order).
    for beam in &beams {
        if tuning::TIE_BEAMS && beam.kind != BeamKind::Conduit {
            let cells: Vec<L2c> = beam.l2_cells(l1).iter().map(|c| c.map(i64::from)).collect();
            b.tie(k, &cells, TieKind::Beam);
        }
    }
    let st = stack(seed, l0);
    let origin = l1 * 4096 - l0 * 32768;
    let mut by_crust: Vec<(u16, Vec<L2c>)> = Vec::new();
    for i in 0..CELLS {
        let c = coords(i);
        let centre = origin + I64Vec3::from_array(c.map(|v| v * 512 + 256));
        if let Stratum::Crust(kk) = st.stratum_at(centre, None) {
            match by_crust.iter_mut().find(|(n, _)| *n == kk) {
                Some((_, v)) => v.push(c),
                None => by_crust.push((kk, vec![c])),
            }
        }
    }
    by_crust.sort_by_key(|(kk, _)| *kk);
    for (_, cells) in by_crust.iter().filter(|_| tuning::TIE_CRUSTS) {
        b.tie(k, cells, TieKind::Crust);
    }
    let between = if tuning::TIE_BETWEEN {
        between_content(seed, l1)
    } else {
        Vec::new()
    };
    for piece in between {
        if ![STR_TETHER, STR_DRIP, STR_SPIRE, STR_ISLAND].contains(&piece.tag) {
            continue;
        }
        let bb = piece.prim.aabb();
        let lo = [bb.min.x, bb.min.y, bb.min.z].map(|v| (i64::from(v) / 16).div_euclid(512));
        let hi = [bb.max.x, bb.max.y, bb.max.z].map(|v| ((i64::from(v) - 1) / 16).div_euclid(512));
        let mut cells = Vec::new();
        for z in lo[2].max(0)..=hi[2].min(N - 1) {
            for y in lo[1].max(0)..=hi[1].min(N - 1) {
                for x in lo[0].max(0)..=hi[0].min(N - 1) {
                    cells.push([x, y, z]);
                }
            }
        }
        b.tie(k, &cells, TieKind::Between);
    }

    // The infinite staircase's cells are on the tree, so the routes of the tissue reach it.
    let column = crate::key::ColumnKey {
        level: Level::L1,
        x: l1.x,
        z: l1.z,
    };
    if let Some(foot) = crate::l1::stair_footprint(seed, column) {
        let l2 = |q: i32| i64::from(q / 16).div_euclid(512);
        let (xs, zs) = (
            (l2(foot.x0), l2(foot.x1 - 1)),
            (l2(foot.z0), l2(foot.z1 - 1)),
        );
        let mut cells = Vec::new();
        for z in zs.0..=zs.1 {
            for y in 0..N {
                for x in xs.0..=xs.1 {
                    cells.push([x, y, z]);
                }
            }
        }
        b.tie(k, &cells, TieKind::Stair);
    }

    let cells: Vec<L2c> = crate::features::tie_cells(seed, l1);
    if !cells.is_empty() {
        b.tie(k, &cells, TieKind::Rare);
    }

    Some(Skeleton {
        knot: index(k) as u16,
        knot_hub: knot_of.hub,
        tree: b.list,
        arteries: b.arteries,
        ports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> WorldSeed {
        WorldSeed(0x5eed)
    }

    /// Lemma A on one cell: `T` is 6-connected, holds the knot and every port cell, and every
    /// artery is a 6-connected path inside `T`.
    fn check(seed: WorldSeed, l1: I64Vec3, s: &Skeleton) {
        let in_t = |i: u16| s.tree.contains(&i);
        assert!(in_t(s.knot), "{l1:?}: knot");
        let mut seen = [false; CELLS];
        let mut stack = vec![s.knot as usize];
        seen[s.knot as usize] = true;
        while let Some(i) = stack.pop() {
            let c = coords(i);
            for dir in Dir6::ALL {
                let o = dir.offset();
                let n = [c[0] + o.x, c[1] + o.y, c[2] + o.z];
                if n.iter().all(|v| (0..N).contains(v)) {
                    let ni = index(n);
                    if !seen[ni] && in_t(ni as u16) {
                        seen[ni] = true;
                        stack.push(ni);
                    }
                }
            }
        }
        assert!(
            s.tree.iter().all(|&i| seen[i as usize]),
            "{l1:?}: tree is not connected"
        );
        for &(dir, p) in &s.ports {
            assert!(in_t(index(p.l2(dir.is_positive())) as u16), "{l1:?}: port");
        }
        for a in &s.arteries {
            for w in a.cells.windows(2) {
                let (c, d) = (coords(w[0] as usize), coords(w[1] as usize));
                let manhattan: i64 = (0..3).map(|k| (c[k] - d[k]).abs()).sum();
                assert_eq!(manhattan, 1, "{l1:?}: artery step");
            }
            assert!(a.cells.iter().all(|&i| in_t(i)));
        }
        let _ = seed;
    }

    #[test]
    fn skeletons_are_connected_and_hold_their_ports() {
        let mut live = 0;
        for x in -3..3 {
            for y in -2..2 {
                for z in -3..3 {
                    let l1 = I64Vec3::new(x, y, z);
                    if let Some(s) = skeleton(seed(), l1) {
                        check(seed(), l1, &s);
                        assert_eq!(s, skeleton(seed(), l1).unwrap(), "deterministic");
                        live += 1;
                    }
                }
            }
        }
        assert!(live > 60);
    }

    /// ART-04..11 occur on real faces; a beam corridor sits only where a beam crosses the face,
    /// and the lift and ladder types only on Y faces.
    #[test]
    fn port_types_follow_their_context() {
        let mut count = [0usize; 9];
        for x in -8..8 {
            for y in -4..4 {
                for z in -8..8 {
                    let key = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                    let cavity = CavityView::new(seed(), key);
                    for axis in Axis::ALL {
                        let face = FaceKey { owner: key, axis };
                        let other = key
                            .neighbor(match axis {
                                Axis::X => Dir6::PosX,
                                Axis::Y => Dir6::PosY,
                                Axis::Z => Dir6::PosZ,
                            })
                            .unwrap();
                        let beams = beam_cells(seed(), face, other);
                        for p in face_artery_ports(seed(), &cavity, face) {
                            assert!(p.kind.allowed(axis), "{key:?} {axis:?} {:?}", p.kind);
                            assert_ne!(p.kind, ArteryType::GravityGate, "no wells yet");
                            if p.kind == ArteryType::BeamCorridor {
                                assert!(beams.contains(&p.ij));
                            }
                            count[ArteryType::ALL.iter().position(|&t| t == p.kind).unwrap()] += 1;
                        }
                    }
                }
            }
        }
        for (t, n) in ArteryType::ALL.iter().zip(count) {
            if *t != ArteryType::GravityGate {
                assert!(n > 0, "{t:?} never drawn");
            }
        }
        println!("port types {:?}: {count:?}", ArteryType::ALL);
    }

    /// With a well near, gravity gates join the draw, on any axis.
    #[test]
    fn gravity_gates_are_drawn_near_a_well() {
        let mut gates = 0;
        for x in -6..6 {
            for z in -6..6 {
                let key = CellKey::new(Level::L1, I64Vec3::new(x, 0, z));
                let cavity = CavityView::new(seed(), key);
                let face = FaceKey {
                    owner: key,
                    axis: Axis::X,
                };
                gates += ports_with(seed(), &cavity, face, true)
                    .iter()
                    .filter(|p| p.kind == ArteryType::GravityGate)
                    .count();
            }
        }
        assert!(gates > 0);
    }

    #[test]
    fn cavity_cells_have_no_skeleton() {
        let l0 = (0..100_000)
            .map(|i: i64| I64Vec3::new(i % 101 - 50, i / 101 % 7 - 3, i / 707 - 20))
            .find(|&l0| {
                crate::regime::rare_event(seed(), l0) == crate::regime::RareEvent::GiantCavity
            })
            .unwrap();
        let mut found = 0;
        for i in 0..512 {
            let l1 = l0 * 8 + I64Vec3::new(i % 8, (i / 8) % 8, i / 64);
            if crate::cavity::is_cavity(seed(), CellKey::new(Level::L1, l1)) {
                assert!(skeleton(seed(), l1).is_none());
                found += 1;
            } else {
                assert!(skeleton(seed(), l1).is_some());
            }
        }
        assert!(found > 0);
    }

    #[test]
    fn faces_carry_two_to_four_distinct_ports_that_both_sides_hold() {
        let cavity_seed = seed();
        for x in -2..2 {
            for y in -2..2 {
                for z in -2..2 {
                    let a = I64Vec3::new(x, y, z);
                    for dir in [Dir6::PosX, Dir6::PosY, Dir6::PosZ] {
                        let b = a + dir.offset();
                        let (Some(sa), Some(sb)) =
                            (skeleton(cavity_seed, a), skeleton(cavity_seed, b))
                        else {
                            continue;
                        };
                        let face = FaceKey::of(CellKey::new(Level::L1, a), dir).unwrap();
                        let ports: Vec<_> = sa
                            .ports
                            .iter()
                            .filter(|(d, _)| *d == dir)
                            .map(|(_, p)| *p)
                            .collect();
                        assert!((2..=4).contains(&ports.len()), "{a:?} {dir:?}");
                        for (i, p) in ports.iter().enumerate() {
                            assert!(ports[..i].iter().all(|q| q.ij != p.ij));
                            assert!(sa.tree.contains(&(index(p.l2(true)) as u16)));
                            assert!(sb.tree.contains(&(index(p.l2(false)) as u16)));
                        }
                        let _ = face;
                    }
                }
            }
        }
    }

    #[test]
    fn walks_always_end_in_their_goal() {
        let mut rng = Rng::new(7);
        for weave in [0u8, 60, 255] {
            let tree = {
                let mut t = [false; CELLS];
                t[index([4, 4, 4])] = true;
                t
            };
            for _ in 0..200 {
                let start = [0, 1, 2].map(|_| rng.below(8) as i64);
                let path = walk(
                    &mut rng,
                    weave,
                    None,
                    start,
                    Goal::Tree,
                    &tree,
                    &[index([4, 4, 4]) as u16],
                );
                assert_eq!(path[0], start);
                assert_eq!(*path.last().unwrap(), [4, 4, 4]);
                assert!(
                    path.windows(2)
                        .all(|w| { (0..3).map(|k| (w[0][k] - w[1][k]).abs()).sum::<i64>() == 1 })
                );
            }
        }
    }

    #[test]
    fn bfs_finishes_a_walk_with_no_budget_left() {
        let path = bfs([0, 0, 0], &|c| c == [7, 7, 7]);
        assert_eq!(path.len(), 22);
    }

    /// PW.2.8 exit: 10⁴ L1 cells spread over many L0 cells (stride 7, so every regime style
    /// and the cell shells are hit) all pass the backbone check, and ties are exercised.
    #[test]
    fn ten_thousand_cells_pass_the_backbone_check() {
        let (mut cells, mut ties, mut loops) = (0, 0, 0);
        for i in 0..10_000i64 {
            let l1 = I64Vec3::new(i % 22 * 7 - 77, i / 22 % 22 * 7 - 77, i / 484 * 7 - 70);
            let Some(s) = skeleton(seed(), l1) else {
                continue;
            };
            check(seed(), l1, &s);
            cells += 1;
            ties += s
                .arteries
                .iter()
                .filter(|a| matches!(a.role, Role::Tie(_)))
                .count();
            loops += s.arteries.iter().filter(|a| a.role == Role::Loop).count();
        }
        assert!(cells > 9_900);
        assert!(ties > 0 && loops > 0, "ties {ties}, loops {loops}");
    }

    /// Prints the skeleton size by role over 600 cells (`--nocapture`) and bounds it loosely.
    #[test]
    fn skeleton_census() {
        let (mut n, mut t, mut by) = (0usize, 0usize, [0usize; 4]);
        for i in 0..600i64 {
            let l1 = I64Vec3::new(i % 10 * 7 - 35, i / 10 % 10 * 7 - 35, i / 100 * 7 - 21);
            let Some(s) = skeleton(seed(), l1) else {
                continue;
            };
            n += 1;
            t += s.tree.len();
            for a in &s.arteries {
                by[match a.role {
                    Role::Port { .. } => 0,
                    Role::Loop => 1,
                    Role::Tie(TieKind::Beam) => 2,
                    Role::Tie(_) => 3,
                }] += a.cells.len();
            }
        }
        eprintln!(
            "skeleton census over {n} cells: tree {} of 512; path cells per cell: ports {}, loops {}, beam ties {}, crust/Between ties {}",
            t / n,
            by[0] / n,
            by[1] / n,
            by[2] / n,
            by[3] / n
        );
        assert!((16..=400).contains(&(t / n)));
    }

    /// GRV-03: radial and along-shell moves weigh three times as much; diagonals do not.
    #[test]
    fn radial_moves_weigh_more_near_a_well() {
        let up = [0, 8, 0];
        assert_eq!(radial_factor([0, 1, 0], up), 3);
        assert_eq!(radial_factor([1, 0, 0], up), 3);
        assert_eq!(radial_factor([1, 1, 0], up), 1);
        let tilted = [8, 8, 0];
        assert_eq!(radial_factor([1, 1, 0], tilted), 3);
        assert_eq!(radial_factor([1, -1, 0], tilted), 3);
        assert_eq!(radial_factor([1, 0, 0], tilted), 1);
    }
}
