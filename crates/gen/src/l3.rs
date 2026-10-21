//! L3 blocks (64 m): the interiors of L2 masses. Spec: `docs/spec/levels.md` §6.
//!
//! Each block is solved on its own by model synthesis inside boundary conditions that the parent
//! L2 cell fixes completely (`l3_plan`), with a fallback that always works. Geometry is in the
//! block's local quanta.

// Slot and storey indices, draws and small counts: all tiny and bounded.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    // Slot loops index several parallel arrays by the same slot number.
    clippy::needless_range_loop,
    clippy::too_many_lines,
    clippy::items_after_statements,
    clippy::unused_self,
    clippy::trivially_copy_pass_by_ref
)]

use std::collections::HashMap;

use glam::I64Vec3;

use crate::cell::{Axis, CellKey, FaceKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::l2::CELL as L2_CELL;
use crate::l2_port::L1Cache;
use crate::l3_plan::{COL, DELTA, EntranceKind, MassPlan, PlanClass};
use crate::l3_solver::{Dims, Domain, Scratch, domain_of, solve_weighted, tile_of};
use crate::l3_tiles::{
    self as tiles, D, F, Family, N_TILES, O, ROOM0, SOLID, SPIRAL_BASE0, SPIRAL_TOP0, SPIRAL0,
    STOREY, W,
};
pub use crate::l3_web::web_parent as l3_parent;
use crate::port::{Port, PortKind};
use crate::prim::{
    CellMeta, CellOutput, ChildConstraint, FeatureKind, MaterialId, PrimFlags, Primitive, Q, QAabb,
    QRect, QVec3, Shape, ZoneKind,
};
use crate::regime::{Regime, StyleId, regime};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Solve attempts before the fallback.
pub const ATTEMPTS: u32 = 6;
/// Slots per block edge, and storeys per block.
const NX: usize = 8;
const NT: usize = 16;
const BLOCK: Q = 1024;
const DIMS: Dims = Dims {
    nx: NX,
    nt: NT,
    nz: NX,
};
const NONE_TILE: u16 = u16::MAX;
/// Tile id used in the layout for the fixed vestibule.
pub const VESTIBULE: u16 = N_TILES as u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotClass {
    Out,
    Rind,
    Core,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotInfo {
    pub class: SlotClass,
    /// Index of the mass in the parent L2 cell, or 255.
    pub mass: u8,
    /// Tile id (`VESTIBULE` for the entrance hall), or `u16::MAX` outside `Core`.
    pub tile: u16,
    /// Turned solid because nothing could reach it.
    pub sealed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipeRun {
    pub storey: u8,
    pub axis: u8,
    pub line: u8,
    pub from: u8,
    pub to: u8,
    pub count: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct L3Layout {
    pub slots: Vec<SlotInfo>,
    pub pipes: Vec<PipeRun>,
    /// Corridor kinds drawn in the block (`l3_dress::CorridorKind` numbers) with counts.
    pub corridors: Vec<(u8, u16)>,
    /// Set-piece rooms drawn in the block (`l3_rooms::RoomKind` numbers) with counts.
    pub rooms: Vec<(u8, u16)>,
}

/// `attempt` is 0 to `ATTEMPTS - 1`, or `ATTEMPTS` when the fallback was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct L3Meta {
    pub attempt: u8,
    pub fallback: bool,
    pub sealed_slots: u16,
}

#[derive(Debug, Clone)]
pub struct L3Block {
    pub out: CellOutput,
    pub layout: L3Layout,
    pub meta: L3Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cls {
    Out,
    Rind(usize),
    Core(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fixed {
    Free,
    Solid,
    Tile(usize),
    Vestibule,
}

/// Everything the block needs from its parent L2 cell.
/// What an L3 block needs from its parent L2 cell: the mass plans and zones. Build it once per
/// L2 cell with [`l3_parent`] and reuse it for every block of that cell.
pub struct Parent {
    plans: Vec<MassPlan>,
    zones: Vec<(QAabb, ZoneKind)>,
}

impl Parent {
    /// The parent of an L2 cell from its mass plans and the zones that reach into it.
    pub fn new(plans: Vec<MassPlan>, zones: Vec<(QAabb, ZoneKind)>) -> Self {
        Self { plans, zones }
    }

    pub fn plans(&self) -> &[MassPlan] {
        &self.plans
    }

    fn classify(&self, cx: i32, t: i32, cz: i32) -> Cls {
        if !(0..64).contains(&cx) || !(0..64).contains(&cz) || !(0..128).contains(&t) {
            return Cls::Out;
        }
        for (i, p) in self.plans.iter().enumerate() {
            if t < p.t0 || t >= p.t1 {
                continue;
            }
            let (x0, z0) = (cx * COL, cz * COL);
            if x0 < p.aabb.max.x
                && x0 + COL > p.aabb.min.x
                && z0 < p.aabb.max.z
                && z0 + COL > p.aabb.min.z
            {
                return if p.cols.contains((cx, cz)) {
                    Cls::Core(i)
                } else {
                    Cls::Rind(i)
                };
            }
        }
        Cls::Out
    }

    fn fixed(&self, mass: usize, cx: i32, t: i32, cz: i32) -> Fixed {
        let p = &self.plans[mass];
        let col = (cx, cz);
        match p.class {
            PlanClass::Sealed => Fixed::Solid,
            PlanClass::VestibuleOnly => {
                if p.is_vestibule(col, t) {
                    Fixed::Vestibule
                } else {
                    Fixed::Solid
                }
            }
            PlanClass::Full => {
                if let Some(s) = p.spine_at(col) {
                    let base = if t == p.t0 {
                        SPIRAL_BASE0
                    } else if t == p.t1 - 1 {
                        SPIRAL_TOP0
                    } else {
                        SPIRAL0
                    };
                    Fixed::Tile(base + usize::from(s.rot))
                } else if p.is_vestibule(col, t) {
                    Fixed::Vestibule
                } else {
                    Fixed::Free
                }
            }
        }
    }
}

/// Domain restrictions over the tile set.
struct Masks {
    walkable: Domain,
    side: [[Domain; 8]; 6],
}

fn and_with(d: &mut Domain, m: &Domain) {
    for i in 0..d.len() {
        d[i] &= m[i];
    }
}

impl Masks {
    fn new(ts: &crate::l3_solver::TileSet) -> Self {
        let walkable = domain_of(
            tiles::table()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.walkable())
                .map(|(i, _)| i),
        );
        let mut side = [[[0; 4]; 8]; 6];
        for (s, row) in side.iter_mut().enumerate() {
            for (k, m) in row.iter_mut().enumerate() {
                *m = ts.with_socket(s, k as u8);
            }
        }
        Self { walkable, side }
    }

    fn require(&self, d: &mut Domain, side: usize, sockets: &[u8]) {
        let mut m = [0u64; 4];
        for &s in sockets {
            for i in 0..4 {
                m[i] |= self.side[side][usize::from(s)][i];
            }
        }
        and_with(d, &m);
    }
}

/// The face pattern of an X or Z block face (§6.6): socket W or D for each of 16 storeys by 8
/// columns, indexed `v * 8 + u`.
fn face_pattern(
    seed: WorldSeed,
    parent: &Parent,
    reg: &Regime,
    owner: I64Vec3,
    axis: Axis,
) -> [u8; NT * NX] {
    let salt = if axis == Axis::X {
        salt::PORT_FACE_X
    } else {
        salt::PORT_FACE_Z
    };
    let mut rng = Rng::new(hash(seed.0, Level::L3, owner, salt));
    let p = 0.04 + (0.10 - 0.04) * (1.0 - f32::from(reg.density) / 255.0);
    let base = owner.map(|v| floor_mod_pow2(v, 3));
    let base = [base.x as i32, base.y as i32, base.z as i32];
    let mut out = [W; NT * NX];
    for v in 0..NT {
        for u in 0..NX {
            let fired = rng.chance(p);
            // The two slots, in global lattice coordinates: lower side first.
            let (a, b) = if axis == Axis::X {
                (
                    (
                        base[0] * 8 + 7,
                        base[1] * 16 + v as i32,
                        base[2] * 8 + u as i32,
                    ),
                    (
                        base[0] * 8 + 8,
                        base[1] * 16 + v as i32,
                        base[2] * 8 + u as i32,
                    ),
                )
            } else {
                (
                    (
                        base[0] * 8 + u as i32,
                        base[1] * 16 + v as i32,
                        base[2] * 8 + 7,
                    ),
                    (
                        base[0] * 8 + u as i32,
                        base[1] * 16 + v as i32,
                        base[2] * 8 + 8,
                    ),
                )
            };
            let dir_a = if axis == Axis::X { 1 } else { 2 };
            let (Cls::Core(ia), Cls::Core(ib)) = (
                parent.classify(a.0, a.1, a.2),
                parent.classify(b.0, b.1, b.2),
            ) else {
                continue;
            };
            if ia != ib {
                continue;
            }
            let plan = &parent.plans[ia];
            let skeleton = plan.edges((a.0, a.2), a.1) >> dir_a & 1 == 1;
            let fa = parent.fixed(ia, a.0, a.1, a.2);
            let fb = parent.fixed(ia, b.0, b.1, b.2);
            let vestibule = fa == Fixed::Vestibule || fb == Fixed::Vestibule;
            let both_free = fa == Fixed::Free && fb == Fixed::Free;
            if skeleton || vestibule || (both_free && fired) {
                out[v * NX + u] = D;
            }
        }
    }
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Unions of axis-aligned boxes in block-local quanta, merged along x, then z, then y.
fn merge_boxes(mut boxes: Vec<[Q; 6]>) -> Vec<[Q; 6]> {
    // Box layout: x0, x1, y0, y1, z0, z1.
    let pass = |boxes: &mut Vec<[Q; 6]>, key: fn(&[Q; 6]) -> [Q; 5], lo: usize, hi: usize| {
        boxes.sort_unstable_by_key(key);
        let mut out: Vec<[Q; 6]> = Vec::with_capacity(boxes.len());
        for b in boxes.drain(..) {
            if let Some(last) = out.last_mut() {
                let same = (0..6)
                    .filter(|&k| k != lo && k != hi)
                    .all(|k| last[k] == b[k]);
                if same && last[hi] == b[lo] {
                    last[hi] = b[hi];
                    continue;
                }
            }
            out.push(b);
        }
        *boxes = out;
    };
    pass(&mut boxes, |b| [b[2], b[3], b[4], b[5], b[0]], 0, 1);
    pass(&mut boxes, |b| [b[0], b[1], b[2], b[3], b[4]], 4, 5);
    pass(&mut boxes, |b| [b[0], b[1], b[4], b[5], b[2]], 2, 3);
    boxes
}

fn bx_prim(b: [Q; 6], material: MaterialId, kind: FeatureKind) -> Primitive {
    Primitive {
        shape: Shape::Box {
            aabb: QAabb::new(QVec3::new(b[0], b[2], b[4]), QVec3::new(b[1], b[3], b[5])),
        },
        material,
        kind,
        // Ladder rails and rungs, and the corridor a climber stands in, are not solid.
        flags: if kind == FeatureKind::MISC || kind == FeatureKind::LADDER {
            PrimFlags::NO_COLLIDE
        } else {
            PrimFlags::NONE
        },
    }
}

/// One block with everything the stages share.
struct Blk<'a> {
    parent: &'a Parent,
    b: [i32; 3],
    cls: Vec<Cls>,
    fixed: Vec<Fixed>,
    /// Face sockets for -x, +x, -z, +z: `[v * 8 + u]`.
    faces: [[u8; NT * NX]; 4],
}

/// Tile choices per slot after acceptance or the fallback.
struct Solved {
    tile: Vec<u16>,
    sealed: Vec<bool>,
}

impl Blk<'_> {
    /// Global lattice coordinates of block slot `i`.
    fn gl(&self, i: usize) -> (i32, i32, i32) {
        let (x, t, z) = DIMS.coords(i);
        (
            self.b[0] * 8 + x as i32,
            self.b[1] * 16 + t as i32,
            self.b[2] * 8 + z as i32,
        )
    }

    /// The socket the face pattern gives slot `(x, t, z)` on horizontal side `k`.
    fn across(&self, x: usize, t: usize, z: usize, k: usize) -> u8 {
        match k {
            3 => self.faces[0][t * NX + z],
            1 => self.faces[1][t * NX + z],
            0 => self.faces[2][t * NX + x],
            _ => self.faces[3][t * NX + x],
        }
    }

    fn mass_of(&self, i: usize) -> Option<usize> {
        match self.cls[i] {
            Cls::Core(m) | Cls::Rind(m) => Some(m),
            Cls::Out => None,
        }
    }

    /// Neighbour slot index inside the block, if any.
    fn nb(&self, i: usize, side: usize) -> Option<usize> {
        DIMS.neighbour(i, side)
    }

    fn is_skeleton(&self, i: usize) -> bool {
        let Cls::Core(m) = self.cls[i] else {
            return false;
        };
        let (cx, t, cz) = self.gl(i);
        self.parent.plans[m].in_skeleton((cx, cz), t)
    }

    /// Whether the slot's socket on horizontal side `k` is a door to another block.
    fn face_door(&self, i: usize, k: usize, tile: u16) -> bool {
        let (x, t, z) = DIMS.coords(i);
        let outside = match k {
            0 => z == 0,
            1 => x == NX - 1,
            2 => z == NX - 1,
            _ => x == 0,
        };
        outside
            && usize::from(tile) < N_TILES
            && tiles::table()[usize::from(tile)].sides[k] == D
            && self.across(x, t, z, k) == D
    }
}

/// The block `block` (an L3 cell): its structure, slot layout and solve statistics.
pub fn l3_block(seed: WorldSeed, cache: &L1Cache, block: CellKey) -> L3Block {
    debug_assert_eq!(block.level, Level::L3);
    let parent_cell = CellKey::new(Level::L2, block.coords.map(|v| floor_div_pow2(v, 3)));
    let parent = crate::l3_web::web_parent(seed, cache, parent_cell);
    l3_block_in(seed, &parent, block, false)
}

/// [`l3_block`] with the fallback forced, to test it on every kind of boundary condition.
pub fn l3_block_fallback(seed: WorldSeed, cache: &L1Cache, block: CellKey) -> L3Block {
    let parent_cell = CellKey::new(Level::L2, block.coords.map(|v| floor_div_pow2(v, 3)));
    let parent = crate::l3_web::web_parent(seed, cache, parent_cell);
    l3_block_in(seed, &parent, block, true)
}

/// Every active block of the L2 cell `parent_cell` (those meeting a mass), in block order. The
/// parent is built once, so this is much cheaper than calling [`l3_block`] per block.
pub fn l3_blocks_of(
    seed: WorldSeed,
    cache: &L1Cache,
    parent_cell: CellKey,
) -> Vec<(CellKey, L3Block)> {
    debug_assert_eq!(parent_cell.level, Level::L2);
    let parent = crate::l3_web::web_parent(seed, cache, parent_cell);
    let mut out = Vec::new();
    for k in 0..512i64 {
        let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
        let block = CellKey::new(Level::L3, parent_cell.coords * 8 + idx);
        if block_is_active(&parent, block) {
            out.push((block, l3_block_in(seed, &parent, block, false)));
        }
    }
    out
}

/// Whether `block` meets a mass of its parent, which is when it has anything to build.
pub fn block_is_active(parent: &Parent, block: CellKey) -> bool {
    let lo = block.coords.map(|v| floor_mod_pow2(v, 3)) * i64::from(BLOCK);
    let hi = lo + I64Vec3::splat(i64::from(BLOCK));
    parent.plans.iter().any(|p| {
        let a = &p.aabb;
        i64::from(a.min.x) < hi.x
            && i64::from(a.max.x) > lo.x
            && i64::from(a.min.y) < hi.y
            && i64::from(a.max.y) > lo.y
            && i64::from(a.min.z) < hi.z
            && i64::from(a.max.z) > lo.z
    })
}

/// The block `block` from a prebuilt parent context.
pub fn l3_block_with(seed: WorldSeed, parent: &Parent, block: CellKey) -> L3Block {
    l3_block_in(seed, parent, block, false)
}

fn l3_block_in(seed: WorldSeed, parent: &Parent, block: CellKey, force_fallback: bool) -> L3Block {
    let reg = regime(seed, block.coords.map(|v| floor_div_pow2(v, 9)));
    let ts = tiles::tileset(reg.density, reg.verticality, reg.style);
    let masks = Masks::new(&ts);
    let b = block.coords.map(|v| floor_mod_pow2(v, 3));
    let n = DIMS.len();
    let mut blk = Blk {
        parent,
        b: [b.x as i32, b.y as i32, b.z as i32],
        cls: Vec::new(),
        fixed: Vec::new(),
        faces: [[W; NT * NX]; 4],
    };
    blk.cls = (0..n)
        .map(|i| {
            let (cx, t, cz) = blk.gl(i);
            parent.classify(cx, t, cz)
        })
        .collect();
    blk.fixed = (0..n)
        .map(|i| match blk.cls[i] {
            Cls::Core(m) => {
                let (cx, t, cz) = blk.gl(i);
                parent.fixed(m, cx, t, cz)
            }
            _ => Fixed::Free,
        })
        .collect();
    // Only blocks with a Core slot that is not already decided need a face pattern.
    blk.faces = [
        face_pattern(seed, parent, &reg, block.coords - I64Vec3::X, Axis::X),
        face_pattern(seed, parent, &reg, block.coords, Axis::X),
        face_pattern(seed, parent, &reg, block.coords - I64Vec3::Z, Axis::Z),
        face_pattern(seed, parent, &reg, block.coords, Axis::Z),
    ];

    let forced = forced_paths(&blk);
    let (active, base) = initial_domains(&blk, &masks, &forced);
    // The archetype of the mass a slot is in weighs the tiles slot by slot.
    let wm = |i: usize, tile: usize| -> u32 {
        match blk.mass_of(i) {
            Some(m) => {
                let plan = &parent.plans[m];
                let (_, t, _) = blk.gl(i);
                crate::l3_arch::weight_mult(plan.archetype, &tiles::table()[tile], t - plan.t0)
            }
            None => 8,
        }
    };
    let mut scratch = Scratch::default();
    let block_hash = |s: u64| hash(seed.0, Level::L3, block.coords, s);
    let mut retry = Rng::new(block_hash(salt::L3_RETRY));
    let mut chosen: Option<(Solved, u8)> = None;
    for attempt in 0..if force_fallback { 0 } else { ATTEMPTS } {
        let mut rng = if attempt == 0 {
            Rng::new(block_hash(salt::L3_SOLVE))
        } else {
            Rng::new(retry.next_u64())
        };
        let mut d = base.clone();
        if solve_weighted(
            &ts,
            DIMS,
            &mut d,
            Some(&active),
            &mut rng,
            &mut scratch,
            Some(&wm),
        )
        .is_ok()
            && let Some(s) = accept(&blk, &d, &active)
        {
            chosen = Some((s, attempt as u8));
            break;
        }
    }
    let (solved, attempt, fallback) = match chosen {
        Some((s, a)) => (s, a, false),
        None => (fallback_tiles(&blk, &active), ATTEMPTS as u8, true),
    };
    assemble(seed, &blk, block, &reg, &solved, attempt, fallback)
}

/// Edges, as direction masks per slot, that connect every slot holding a face door to the
/// skeleton: a shortest path (neighbours in side order) over free slots of the same mass on the
/// same storey. They are imposed like skeleton edges, so a face door can never lead to a room
/// that the solver left cut off.
fn forced_paths(blk: &Blk) -> Vec<u8> {
    let n = DIMS.len();
    let mut edges = vec![0u8; n];
    let free = |i: usize| matches!(blk.cls[i], Cls::Core(_)) && blk.fixed[i] == Fixed::Free;
    for i in 0..n {
        if !free(i) {
            continue;
        }
        let (x, t, z) = DIMS.coords(i);
        let door = (0..4).any(|k| {
            let outside = match k {
                0 => z == 0,
                1 => x == NX - 1,
                2 => z == NX - 1,
                _ => x == 0,
            };
            outside && blk.across(x, t, z, k) == D && face_neighbour_is_core(blk, i, k)
        });
        if !door {
            continue;
        }
        let mut prev: HashMap<usize, usize> = HashMap::new();
        let mut queue = std::collections::VecDeque::from([i]);
        let mut seen = vec![false; n];
        seen[i] = true;
        let mut target = None;
        while let Some(c) = queue.pop_front() {
            if blk.is_skeleton(c) {
                target = Some(c);
                break;
            }
            for k in 0..4 {
                let Some(j) = blk.nb(c, k) else {
                    continue;
                };
                if seen[j] || !free(j) || blk.mass_of(j) != blk.mass_of(i) {
                    continue;
                }
                seen[j] = true;
                prev.insert(j, c);
                queue.push_back(j);
            }
        }
        let mut at = target;
        while let Some(c) = at {
            if let Some(&p) = prev.get(&c) {
                for k in 0..4 {
                    if blk.nb(c, k) == Some(p) {
                        edges[c] |= 1 << k;
                        edges[p] |= 1 << ((k + 2) % 4);
                    }
                }
            }
            at = prev.get(&c).copied();
        }
    }
    edges
}

/// Active slots and their initial domains (§6.7 step 1 and the boundary conditions of §6.6).
fn initial_domains(blk: &Blk, masks: &Masks, forced: &[u8]) -> (Vec<bool>, Vec<Domain>) {
    let n = DIMS.len();
    let mut active = vec![false; n];
    let mut base: Vec<Domain> = vec![[0; 4]; n];
    for i in 0..n {
        let Cls::Core(m) = blk.cls[i] else {
            continue;
        };
        let (x, t, z) = DIMS.coords(i);
        let (cx, ts_, cz) = blk.gl(i);
        let plan = &blk.parent.plans[m];
        match blk.fixed[i] {
            Fixed::Tile(id) => {
                active[i] = true;
                base[i] = domain_of([id]);
                continue;
            }
            Fixed::Solid | Fixed::Vestibule => continue,
            Fixed::Free => {}
        }
        active[i] = true;
        let mut d = domain_of(0..N_TILES);
        for (k, delta) in DELTA.iter().enumerate() {
            let (nx_, nz_) = (cx + delta.0, cz + delta.1);
            let skeleton = (plan.edges((cx, cz), ts_) | forced[i]) >> k & 1 == 1;
            match blk.parent.classify(nx_, ts_, nz_) {
                Cls::Core(j) if j == m => {
                    if blk.nb(i, k).is_some() {
                        if skeleton {
                            masks.require(&mut d, k, &[O, D]);
                        } else if blk.parent.fixed(m, nx_, ts_, nz_) == Fixed::Vestibule {
                            masks.require(&mut d, k, &[D]);
                        }
                    } else {
                        masks.require(&mut d, k, &[blk.across(x, t, z, k)]);
                    }
                }
                _ => masks.require(&mut d, k, &[W]),
            }
        }
        if plan.edges((cx, cz), ts_) != 0 || forced[i] != 0 {
            and_with(&mut d, &masks.walkable);
        }
        for (side, dt) in [(4usize, -1i32), (5, 1)] {
            let free_same = matches!(blk.parent.classify(cx, ts_ + dt, cz), Cls::Core(j) if j == m)
                && blk.nb(i, side).is_some()
                && blk.parent.fixed(m, cx, ts_ + dt, cz) == Fixed::Free;
            if !free_same {
                masks.require(&mut d, side, &[F]);
            }
        }
        base[i] = d;
    }
    (active, base)
}

/// The walk components of a tile assignment: union-find over walkable slots joined by open or
/// door sides and by stacked spirals.
fn walk_components(blk: &Blk, tile: &[u16]) -> Vec<usize> {
    let n = DIMS.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let walkable = |i: usize| -> bool {
        let t = tile[i];
        t == VESTIBULE || (usize::from(t) < N_TILES && tiles::table()[usize::from(t)].walkable())
    };
    let socket = |i: usize, side: usize| -> Option<u8> {
        let t = tile[i];
        if t == VESTIBULE {
            // D towards every Core neighbour of its mass, W elsewhere; resolved by the caller
            // through the neighbour's own socket.
            return Some(D);
        }
        (usize::from(t) < N_TILES).then(|| tiles::table()[usize::from(t)].sockets()[side])
    };
    for i in 0..n {
        if !walkable(i) {
            continue;
        }
        for side in [1usize, 2, 5] {
            let Some(j) = blk.nb(i, side) else {
                continue;
            };
            if !walkable(j) {
                continue;
            }
            let (a, c) = (socket(i, side), socket(j, crate::l3_solver::opposite(side)));
            let linked = if side == 5 {
                // Only a stair through both slots joins storeys: matching spiral sockets, not a
                // top below a base, which meet at a floor.
                matches!((a, c), (Some(p), Some(q)) if p == q && p >= 4)
            } else {
                matches!(a, Some(O | D)) && matches!(c, Some(O | D)) && a == c
                    || (tile[i] == VESTIBULE && matches!(c, Some(O | D)))
                    || (tile[j] == VESTIBULE && matches!(a, Some(O | D)))
            };
            if linked {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            }
        }
    }
    (0..n).map(|i| find(&mut parent, i)).collect()
}

fn tile_is_spiral(t: u16) -> bool {
    usize::from(t) < N_TILES && tiles::table()[usize::from(t)].is_spiral()
}

/// Initial tile per slot: the collapsed domain, or the fixed one.
fn tiles_of(blk: &Blk, d: &[Domain], active: &[bool]) -> Vec<u16> {
    (0..DIMS.len())
        .map(|i| match blk.cls[i] {
            Cls::Core(_) => match blk.fixed[i] {
                Fixed::Solid => SOLID as u16,
                Fixed::Vestibule => VESTIBULE,
                _ if active[i] => tile_of(&d[i]).map_or(NONE_TILE, |t| t as u16),
                _ => NONE_TILE,
            },
            _ => NONE_TILE,
        })
        .collect()
}

/// §6.8: seal what nothing reaches, collapse dead voids, and accept when every remaining walkable
/// slot reaches the skeleton. `None` is a failed attempt.
fn accept(blk: &Blk, d: &[Domain], active: &[bool]) -> Option<Solved> {
    let n = DIMS.len();
    let mut tile = tiles_of(blk, d, active);
    if tile
        .iter()
        .enumerate()
        .any(|(i, &t)| matches!(blk.cls[i], Cls::Core(_)) && t == NONE_TILE)
    {
        return None;
    }
    let comp = walk_components(blk, &tile);
    let mut has_skeleton: HashMap<usize, bool> = HashMap::new();
    let mut has_face_door: HashMap<usize, bool> = HashMap::new();
    for i in 0..n {
        let t = tile[i];
        let walkable = t == VESTIBULE
            || (usize::from(t) < N_TILES && tiles::table()[usize::from(t)].walkable());
        if !walkable {
            continue;
        }
        let e = has_skeleton.entry(comp[i]).or_insert(false);
        *e |= blk.is_skeleton(i);
        let f = has_face_door.entry(comp[i]).or_insert(false);
        *f |= (0..4).any(|k| blk.face_door(i, k, t));
    }
    let mut sealed = vec![false; n];
    for i in 0..n {
        let t = tile[i];
        let walkable = t == VESTIBULE
            || (usize::from(t) < N_TILES && tiles::table()[usize::from(t)].walkable());
        if !walkable || has_skeleton[&comp[i]] {
            continue;
        }
        if has_face_door[&comp[i]] {
            return None;
        }
        tile[i] = SOLID as u16;
        sealed[i] = true;
    }
    // A void standing on solid keeps a floor only if a walkable slot of the skeleton's
    // component opens onto it; otherwise it fills in.
    for t in 0..NT {
        for z in 0..NX {
            for x in 0..NX {
                let i = DIMS.index(x, t, z);
                let Some(Family::Void(_)) = tile_family(tile[i]) else {
                    continue;
                };
                let Some(below) = blk.nb(i, 4) else {
                    continue;
                };
                if tile[below] != SOLID as u16 {
                    continue;
                }
                let info = tiles::table()[usize::from(tile[i])];
                let open_to_walkable = (0..4).any(|k| {
                    info.sides[k] == O
                        && blk.nb(i, k).is_some_and(|j| {
                            let tj = tile[j];
                            tj == VESTIBULE
                                || (usize::from(tj) < N_TILES
                                    && tiles::table()[usize::from(tj)].walkable())
                        })
                });
                if !open_to_walkable {
                    tile[i] = SOLID as u16;
                    sealed[i] = true;
                }
            }
        }
    }
    Some(Solved { tile, sealed })
}

fn tile_family(t: u16) -> Option<Family> {
    (usize::from(t) < N_TILES).then(|| tiles::table()[usize::from(t)].family)
}

/// §6.8 fallback: rooms on the skeleton and on paths from every face door to it, solid elsewhere.
fn fallback_tiles(blk: &Blk, active: &[bool]) -> Solved {
    let n = DIMS.len();
    let mut tile = vec![NONE_TILE; n];
    let mut sealed = vec![false; n];
    let mut room = vec![false; n];
    // Fixed slots first.
    for i in 0..n {
        if let Cls::Core(_) = blk.cls[i] {
            match blk.fixed[i] {
                Fixed::Solid => tile[i] = SOLID as u16,
                Fixed::Vestibule => tile[i] = VESTIBULE,
                Fixed::Tile(id) => tile[i] = id as u16,
                Fixed::Free => {}
            }
        }
    }
    let free = |i: usize| active[i] && blk.fixed[i] == Fixed::Free;
    for i in 0..n {
        if !free(i) {
            continue;
        }
        if blk.is_skeleton(i) {
            room[i] = true;
        }
        // Adjacent to a fixed slot's D side: the vestibule opens on every Core neighbour.
        for k in 0..4 {
            if let Some(j) = blk.nb(i, k)
                && blk.fixed[j] == Fixed::Vestibule
            {
                room[i] = true;
            }
        }
    }
    // Paths from every face-door slot to the nearest skeleton slot, on its storey.
    for i in 0..n {
        if !free(i) {
            continue;
        }
        let (x, t, z) = DIMS.coords(i);
        let door = (0..4).any(|k| {
            let outside = match k {
                0 => z == 0,
                1 => x == NX - 1,
                2 => z == NX - 1,
                _ => x == 0,
            };
            outside && blk.across(x, t, z, k) == D && face_neighbour_is_core(blk, i, k)
        });
        if !door {
            continue;
        }
        room[i] = true;
        let mut prev: HashMap<usize, usize> = HashMap::new();
        let mut queue = std::collections::VecDeque::from([i]);
        let mut target = None;
        let mut seen = vec![false; n];
        seen[i] = true;
        while let Some(c) = queue.pop_front() {
            if blk.is_skeleton(c) {
                target = Some(c);
                break;
            }
            for k in 0..4 {
                let Some(j) = blk.nb(c, k) else {
                    continue;
                };
                if seen[j] || !free(j) || blk.mass_of(j) != blk.mass_of(i) {
                    continue;
                }
                seen[j] = true;
                prev.insert(j, c);
                queue.push_back(j);
            }
        }
        let mut at = target;
        while let Some(c) = at {
            room[c] = true;
            at = prev.get(&c).copied();
        }
    }
    for i in 0..n {
        if !free(i) {
            continue;
        }
        if !room[i] {
            tile[i] = SOLID as u16;
            continue;
        }
        let (x, t, z) = DIMS.coords(i);
        let mut code = 0usize;
        for k in 0..4 {
            let socket = match blk.nb(i, k) {
                Some(j) => {
                    if room[j] || blk.fixed[j] == Fixed::Vestibule {
                        D
                    } else if let Fixed::Tile(id) = blk.fixed[j] {
                        // The spine is open only on its door side.
                        tiles::table()[id].sockets()[crate::l3_solver::opposite(k)]
                    } else {
                        W
                    }
                }
                None => {
                    if face_neighbour_is_core(blk, i, k) {
                        blk.across(x, t, z, k)
                    } else {
                        W
                    }
                }
            };
            code += usize::from(socket) * 3usize.pow(k as u32);
        }
        tile[i] = (ROOM0 + code) as u16;
    }
    // Free active slots left without a tile cannot happen; keep the layout total.
    for i in 0..n {
        if tile[i] == NONE_TILE && matches!(blk.cls[i], Cls::Core(_)) {
            tile[i] = SOLID as u16;
            sealed[i] = true;
        }
    }
    Solved { tile, sealed }
}

/// Whether the slot across the block face on side `k` is a Core slot of the same mass.
fn face_neighbour_is_core(blk: &Blk, i: usize, k: usize) -> bool {
    let (cx, t, cz) = blk.gl(i);
    let m = blk.mass_of(i);
    matches!(
        blk.parent.classify(cx + DELTA[k].0, t, cz + DELTA[k].1),
        Cls::Core(j) if Some(j) == m
    )
}

/// A whole-slot floor slab. These stay one box per slot: the walkability extractor cuts a plate
/// from each box's top, and one big merged slab gives it badly shaped pieces.
fn is_floor_slab(b: &QAabb) -> bool {
    b.max.x - b.min.x == COL && b.max.z - b.min.z == COL && b.max.y - b.min.y == 4
}

fn translate(shape: Shape, o: [Q; 3]) -> Shape {
    let sh = |bb: QAabb| {
        QAabb::new(
            QVec3::new(bb.min.x + o[0], bb.min.y + o[1], bb.min.z + o[2]),
            QVec3::new(bb.max.x + o[0], bb.max.y + o[1], bb.max.z + o[2]),
        )
    };
    match shape {
        Shape::Box { aabb } => Shape::Box { aabb: sh(aabb) },
        Shape::StairRun { aabb, rise, steps } => Shape::StairRun {
            aabb: sh(aabb),
            rise,
            steps,
        },
        other => other,
    }
}

fn clip(a: &QAabb, b: &QAabb) -> Option<QAabb> {
    let min = QVec3::new(
        a.min.x.max(b.min.x),
        a.min.y.max(b.min.y),
        a.min.z.max(b.min.z),
    );
    let max = QVec3::new(
        a.max.x.min(b.max.x),
        a.max.y.min(b.max.y),
        a.max.z.min(b.max.z),
    );
    (min.x < max.x && min.y < max.y && min.z < max.z).then(|| QAabb::new(min, max))
}

/// The slot box of block slot `i`, in block-local quanta.
fn slot_box(i: usize) -> QAabb {
    let (x, t, z) = DIMS.coords(i);
    let (x0, y0, z0) = (x as Q * COL, t as Q * STOREY, z as Q * COL);
    QAabb::new(
        QVec3::new(x0, y0, z0),
        QVec3::new(x0 + COL, y0 + STOREY, z0 + COL),
    )
}

fn box6(b: &QAabb) -> [Q; 6] {
    [b.min.x, b.max.x, b.min.y, b.max.y, b.min.z, b.max.z]
}

#[allow(clippy::too_many_lines)] // geometry, pipes, ports and constraints in one pass
fn assemble(
    seed: WorldSeed,
    blk: &Blk,
    block: CellKey,
    reg: &Regime,
    solved: &Solved,
    attempt: u8,
    fallback: bool,
) -> L3Block {
    let n = DIMS.len();
    let origin = [blk.b[0] * BLOCK, blk.b[1] * BLOCK, blk.b[2] * BLOCK];
    let block_box = QAabb::new(
        QVec3::new(origin[0], origin[1], origin[2]),
        QVec3::new(origin[0] + BLOCK, origin[1] + BLOCK, origin[2] + BLOCK),
    );
    let local = |bb: &QAabb| {
        QAabb::new(
            QVec3::new(
                bb.min.x - origin[0],
                bb.min.y - origin[1],
                bb.min.z - origin[2],
            ),
            QVec3::new(
                bb.max.x - origin[0],
                bb.max.y - origin[1],
                bb.max.z - origin[2],
            ),
        )
    };
    let parent = blk.parent;
    let mut prims: Vec<Primitive> = Vec::new();

    // Corridor kinds (§6W.5), from the tiles chosen.
    let view: Vec<crate::l3_dress::SlotView> = (0..n)
        .map(|i| {
            let Cls::Core(m) = blk.cls[i] else {
                return crate::l3_dress::SlotView {
                    tile: NONE_TILE,
                    ..Default::default()
                };
            };
            let (cx, t, cz) = blk.gl(i);
            let plan = &parent.plans[m];
            crate::l3_dress::SlotView {
                tile: solved.tile[i],
                core: plan.class != PlanClass::Sealed,
                skeleton: plan.in_skeleton((cx, cz), t),
                edges: plan.edges((cx, cz), t),
                outer: std::array::from_fn(|k| {
                    !matches!(
                        parent.classify(cx + DELTA[k].0, t, cz + DELTA[k].1),
                        Cls::Core(j) if j == m
                    )
                }),
            }
        })
        .collect();
    let dress = crate::l3_dress::corridor_kinds(seed, block, &view);
    let (room_pieces, room_kinds) = crate::l3_rooms::rooms(seed, block, &view, &dress.holes);

    // Layout.
    let slots: Vec<SlotInfo> = (0..n)
        .map(|i| SlotInfo {
            class: match blk.cls[i] {
                Cls::Out => SlotClass::Out,
                Cls::Rind(_) => SlotClass::Rind,
                Cls::Core(_) => SlotClass::Core,
            },
            mass: blk.mass_of(i).map_or(255, |m| m as u8),
            tile: if matches!(blk.cls[i], Cls::Core(_)) {
                solved.tile[i]
            } else {
                NONE_TILE
            },
            sealed: solved.sealed[i],
        })
        .collect();

    // Solid and rind boxes, merged. A sealed mass is one box per block.
    let mut solids: Vec<[Q; 6]> = Vec::new();
    let extra: Vec<Primitive> = Vec::new();
    for (m, plan) in parent.plans.iter().enumerate() {
        if plan.class == PlanClass::Sealed
            && let Some(c) = clip(&plan.aabb, &block_box)
        {
            solids.push(box6(&local(&c)));
        }
        let _ = m;
    }
    for i in 0..n {
        let Some(m) = blk.mass_of(i) else {
            continue;
        };
        let plan = &parent.plans[m];
        if plan.class == PlanClass::Sealed {
            continue;
        }
        let (cx, t, cz) = blk.gl(i);
        match blk.cls[i] {
            Cls::Rind(_) => {
                let (x0, z0) = (cx * COL, cz * COL);
                let full = QAabb::new(
                    QVec3::new(x0, t * STOREY, z0),
                    QVec3::new(x0 + COL, t * STOREY + STOREY, z0 + COL),
                );
                let Some(c) = clip(&full, &plan.aabb) else {
                    continue;
                };
                solids.push(box6(&local(&c)));
            }
            Cls::Core(_) if solved.tile[i] == SOLID as u16 => solids.push(box6(&slot_box(i))),
            _ => {}
        }
    }
    // Boxes of one material and kind are merged across slots; other shapes stay as they are.
    let mut groups: std::collections::BTreeMap<(u16, u8), Vec<[Q; 6]>> =
        std::collections::BTreeMap::new();
    groups
        .entry((MaterialId::CONCRETE.0, FeatureKind::MASS.0))
        .or_default()
        .extend(solids);
    let mut others: Vec<Primitive> = Vec::new();
    let mut floors: Vec<Primitive> = Vec::new();

    // Tile pieces, in slot order.
    let mut cache: HashMap<u16, Vec<tiles::Piece>> = HashMap::new();
    let mut door_boxes: Vec<QAabb> = Vec::new();
    for i in 0..n {
        let Cls::Core(m) = blk.cls[i] else {
            continue;
        };
        let plan = &parent.plans[m];
        let tile = solved.tile[i];
        if plan.class == PlanClass::Sealed || tile == SOLID as u16 || tile == NONE_TILE {
            continue;
        }
        let (x, t, z) = DIMS.coords(i);
        let o = [x as Q * COL, t as Q * STOREY, z as Q * COL];
        let (cx, ts_, cz) = blk.gl(i);
        let pieces = if tile == VESTIBULE {
            let inner: [bool; 4] = std::array::from_fn(|k| {
                matches!(
                    parent.classify(cx + DELTA[k].0, ts_, cz + DELTA[k].1),
                    Cls::Core(j) if j == m
                )
            });
            let mut doors: Vec<tiles::VDoor> = Vec::new();
            let mut hatch = tiles::Hatch::None;
            for e in plan.entrances_at((cx, cz), ts_) {
                match e.kind {
                    EntranceKind::Door => {
                        let c0 = if e.side % 2 == 0 { cx } else { cz } * COL;
                        let open = ((e.u0 - c0).max(0), (e.u1 - c0).min(COL));
                        if open.0 < open.1 {
                            doors.push(tiles::VDoor {
                                side: usize::from(e.side),
                                open,
                            });
                        }
                    }
                    EntranceKind::FloorHatch => hatch = tiles::Hatch::Floor,
                    EntranceKind::RoofHatch | EntranceKind::Ladder => hatch = tiles::Hatch::Roof,
                }
            }
            tiles::vestibule_pieces(&doors, hatch, inner, reg.style)
        } else {
            cache
                .entry(tile)
                .or_insert_with(|| tiles::pieces(usize::from(tile), reg.style))
                .clone()
        };
        for p in pieces {
            match translate(p.shape, o) {
                Shape::Box { aabb } if is_floor_slab(&aabb) && dress.holes.contains(&i) => {
                    // A ladder well: the slab round a 1 m hole.
                    floors.extend(
                        crate::l3_dress::holed_floor(o)
                            .into_iter()
                            .map(|a| Primitive {
                                shape: a.shape,
                                material: a.material,
                                kind: a.kind,
                                flags: a.flags,
                            }),
                    );
                }
                Shape::Box { aabb } if is_floor_slab(&aabb) => floors.push(Primitive {
                    shape: Shape::Box { aabb },
                    material: p.material,
                    kind: p.kind,
                    flags: PrimFlags::NONE,
                }),
                Shape::Box { aabb } => groups
                    .entry((p.material.0, p.kind.0))
                    .or_default()
                    .push(box6(&aabb)),
                shape => others.push(Primitive {
                    shape,
                    material: p.material,
                    kind: p.kind,
                    flags: PrimFlags::NONE,
                }),
            }
        }
        // Roof where nothing of the same mass stands above.
        let above_core = matches!(parent.classify(cx, ts_ + 1, cz), Cls::Core(j) if j == m);
        let roof_hatch = tile == VESTIBULE
            && plan
                .entrances_at((cx, cz), ts_)
                .any(|e| matches!(e.kind, EntranceKind::RoofHatch | EntranceKind::Ladder));
        if !above_core && !roof_hatch {
            groups
                .entry((MaterialId::CONCRETE.0, FeatureKind::MASS.0))
                .or_default()
                .push([
                    o[0],
                    o[0] + COL,
                    o[1] + STOREY - 4,
                    o[1] + STOREY,
                    o[2],
                    o[2] + COL,
                ]);
        }
        if usize::from(tile) < N_TILES {
            let info = tiles::table()[usize::from(tile)];
            for k in 0..4 {
                if info.sides[k] == D && !info.is_spiral() {
                    door_boxes.push(door_keepout(i, k));
                }
            }
        }
    }

    for ((material, kind), boxes) in groups {
        for bxx in merge_boxes(boxes) {
            prims.push(bx_prim(bxx, MaterialId(material), FeatureKind(kind)));
        }
    }
    prims.extend(floors);
    prims.extend(others);
    prims.extend(dress.pieces.iter().chain(&room_pieces).map(|a| Primitive {
        shape: a.shape,
        material: a.material,
        kind: a.kind,
        flags: a.flags,
    }));
    prims.extend(extra);

    // Pipes.
    let (pipe_prims, pipes) = pipe_runs(seed, blk, block, reg, solved);
    prims.extend(pipe_prims);

    // Ports: every door on an X or Z block face, every spine crossing of a Y face.
    let mut ports: Vec<Port> = Vec::new();
    let faces = [
        (block.coords - I64Vec3::X, Axis::X, 0usize, 7usize),
        (block.coords, Axis::X, 1, 0),
        (block.coords - I64Vec3::Z, Axis::Z, 2, 0),
        (block.coords, Axis::Z, 3, 7),
    ];
    let _ = faces;
    for (owner, axis, which, near_plus) in [
        (block.coords - I64Vec3::X, Axis::X, 0usize, false),
        (block.coords, Axis::X, 1, true),
        (block.coords - I64Vec3::Z, Axis::Z, 2, false),
        (block.coords, Axis::Z, 3, true),
    ] {
        let _ = near_plus;
        let mut id = 0u16;
        for v in 0..NT {
            for u in 0..NX {
                if blk.faces[which][v * NX + u] != D {
                    continue;
                }
                // The slot of this block on that face must be a Core slot with a door there.
                let (x, z) = match which {
                    0 => (0, u),
                    1 => (NX - 1, u),
                    2 => (u, 0),
                    _ => (u, NX - 1),
                };
                let i = DIMS.index(x, v, z);
                let k = [3, 1, 0, 2][which];
                if !blk.face_door(i, k, solved.tile[i]) {
                    continue;
                }
                ports.push(Port {
                    face: FaceKey {
                        owner: CellKey::new(Level::L3, owner),
                        axis,
                    },
                    id,
                    kind: PortKind::Walkway,
                    rect: QRect {
                        u0: u as Q * COL + 48,
                        u1: u as Q * COL + 80,
                        v0: v as Q * STOREY + 4,
                        v1: v as Q * STOREY + 48,
                    },
                });
                id += 1;
            }
        }
    }

    let mut constraints: Vec<ChildConstraint> = (0..ports.len())
        .map(|i| ChildConstraint::ForcedPort { port: i as u32 })
        .collect();
    constraints.extend(
        door_boxes
            .into_iter()
            .map(|aabb| ChildConstraint::KeepOut { aabb }),
    );
    for i in 0..n {
        if let Cls::Core(_) = blk.cls[i]
            && tile_is_spiral(solved.tile[i])
        {
            let o = slot_box(i).min;
            for (x, z) in [
                ((4, 124), (4, 32)),
                ((4, 124), (96, 124)),
                ((4, 32), (32, 96)),
                ((96, 124), (32, 96)),
            ] {
                constraints.push(ChildConstraint::KeepOut {
                    aabb: QAabb::new(
                        QVec3::new(o.x + x.0, o.y, o.z + z.0),
                        QVec3::new(o.x + x.1, o.y + STOREY, o.z + z.1),
                    ),
                });
            }
        }
    }
    let mut flags = 0;
    for (zone, kind) in &parent.zones {
        if let Some(c) = clip(zone, &block_box) {
            constraints.push(ChildConstraint::Zone {
                aabb: local(&c),
                kind: *kind,
            });
            flags |= match kind {
                ZoneKind::Sanctuary => crate::prim::MetaFlags::SANCTUARY.0,
                ZoneKind::Vista => crate::prim::MetaFlags::VISTA.0,
                ZoneKind::GiantCavity => 0,
            };
        }
    }

    let sealed_slots = solved.sealed.iter().filter(|&&s| s).count() as u16;
    L3Block {
        out: CellOutput {
            key: block,
            primitives: prims,
            ports,
            child_constraints: constraints,
            meta: CellMeta {
                flags: crate::prim::MetaFlags(flags),
            },
        },
        layout: L3Layout {
            slots,
            pipes,
            corridors: dress.kinds.iter().map(|&(k, n)| (k as u8, n)).collect(),
            rooms: room_kinds.iter().map(|&(k, n)| (k as u8, n)).collect(),
        },
        meta: L3Meta {
            attempt,
            fallback,
            sealed_slots,
        },
    }
}

/// The keep-out of a door on side `k` of slot `i`: the opening and a metre either side, from the
/// floor to the lintel, through the wall.
fn door_keepout(i: usize, k: usize) -> QAabb {
    let o = slot_box(i).min;
    let (u0, u1) = (32, 96);
    let depth = |at: Q| (at - 16, at + 16);
    let (x, z) = match k {
        0 => ((u0, u1), depth(0)),
        2 => ((u0, u1), depth(COL)),
        1 => (depth(COL), (u0, u1)),
        _ => (depth(0), (u0, u1)),
    };
    QAabb::new(
        QVec3::new(o.x + x.0, o.y + 4, o.z + z.0),
        QVec3::new(o.x + x.1, o.y + 48, o.z + z.1),
    )
}

/// The pipe overlay (§6.9): runs along rooms, atria and voids, drawn after acceptance so they
/// never affect solvability.
fn pipe_runs(
    seed: WorldSeed,
    blk: &Blk,
    block: CellKey,
    reg: &Regime,
    solved: &Solved,
) -> (Vec<Primitive>, Vec<PipeRun>) {
    let mut rng = Rng::new(hash(seed.0, Level::L3, block.coords, salt::L3_PIPE));
    let max = lerp(1.0, 6.0, f32::from(reg.density) / 255.0) as u64
        * if reg.style == StyleId::PIPEWORKS {
            2
        } else {
            1
        };
    let runs = rng.below(max + 1);
    let pipeable = |i: usize| -> bool {
        matches!(blk.cls[i], Cls::Core(_))
            && matches!(
                tile_family(solved.tile[i]),
                Some(Family::Room | Family::Atrium | Family::Void(_))
            )
    };
    let mut prims = Vec::new();
    let mut out = Vec::new();
    for _ in 0..runs {
        let storey = rng.below(16) as usize;
        let axis = rng.below(2) as usize;
        let line = rng.below(8) as usize;
        let count = rng.range_i32(1, 4) as usize;
        let offsets = [rng.below(5) as Q, rng.below(5) as Q, rng.below(5) as Q];
        let at = |k: usize| {
            if axis == 0 {
                DIMS.index(k, storey, line)
            } else {
                DIMS.index(line, storey, k)
            }
        };
        let Some(first) = (0..NX).find(|&k| pipeable(at(k))) else {
            continue;
        };
        let mut from = first;
        while from > 0 && pipeable(at(from - 1)) {
            from -= 1;
        }
        let mut to = first;
        while to + 1 < NX && pipeable(at(to + 1)) {
            to += 1;
        }
        out.push(PipeRun {
            storey: storey as u8,
            axis: axis as u8,
            line: line as u8,
            from: from as u8,
            to: to as u8,
            count: count as u8,
        });
        for &o in offsets.iter().take(count) {
            let across = 16 + 24 * o;
            let y = storey as Q * STOREY + 52;
            let (base, cyl_axis) = if axis == 0 {
                (
                    QVec3::new(from as Q * COL, y, line as Q * COL + across),
                    Axis::X,
                )
            } else {
                (
                    QVec3::new(line as Q * COL + across, y, from as Q * COL),
                    Axis::Z,
                )
            };
            prims.push(Primitive {
                shape: Shape::cylinder(cyl_axis, base, (to - from + 1) as Q * COL, 4),
                material: MaterialId::PIPE,
                kind: FeatureKind::PIPE,
                flags: PrimFlags::NONE,
            });
        }
    }
    (prims, out)
}

/// Mass index range helper for tests and tools: the L2 cell's lattice size.
pub const L2_SLOTS: (i32, i32) = (64, 128);
const _: () = assert!(L2_CELL == 64 * COL);
