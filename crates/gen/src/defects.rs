//! Defect census (PW.4.8): counters for the faults the 2026-10-20 walk found, over L2 cells
//! with their L3 interiors and parent L1 pieces, in one cell-local frame. Spec: `levels.md`
//! §5W.29. Nothing here changes generator output.
//!
//! The counters (letters as in `CHECKLIST.md` PW.4.8):
//! - `a` rails with no floor beside them on either side;
//! - `b` solid pieces of paths (own and neighbours') inside the walk space of a path;
//! - `c` dressing (`NO_COLLIDE`) with no solid host touching it;
//! - `d` vertical members with nothing at either end;
//! - `e` doors with no floor outside, or none inside;
//! - `f` path ends with nothing to continue on (over air or against solid);
//! - `g` faces of different pieces in one plane, both visible, facing the same way;
//! - `h` interior floors (L3 slabs) tilted by more than 1 degree;
//! - `i` rails the engine cannot draw in their own frame (not a box or a wedge).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::default_trait_access,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::manual_midpoint,
    clippy::cast_sign_loss,
    clippy::collapsible_if,
    clippy::needless_pass_by_value,
    clippy::items_after_statements
)]

use std::collections::{BTreeMap, HashMap};

use glam::{DVec3, I64Vec3, Vec3};

use crate::carve::{as_box, translated};
use crate::cell::{CellKey, Level};
use crate::l2_port::L1Cache;
use crate::l2_web::l2_cell_web;
use crate::l3_web::{doors_of, web_context};
use crate::paths::Walk;
use crate::prim::{FeatureKind, PrimFlags, Primitive, Shape};
use crate::seed::WorldSeed;
use crate::tissue::tissue_cell;

/// The levels a piece can come from, for pair counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Src {
    L1,
    L2,
    L3,
    /// A neighbour's path piece: only a witness, never a subject.
    Nbr,
}

/// Counter letters in report order.
pub const LETTERS: [char; 9] = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i'];

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub cells: usize,
    /// Counter letter, and for `e` and `f` a sub-name, to count.
    pub counts: BTreeMap<String, u64>,
    /// What was measured, so a count can be read against it.
    pub bases: BTreeMap<String, u64>,
    pub examples: Vec<String>,
}

impl Report {
    fn hit(&mut self, name: &str, example: impl FnOnce() -> String) {
        let n = self.counts.entry(name.to_string()).or_default();
        *n += 1;
        if *n <= 3 {
            self.examples.push(format!("{name}: {}", example()));
        }
    }

    fn base(&mut self, name: &str, n: u64) {
        *self.bases.entry(name.to_string()).or_default() += n;
    }

    pub fn merge(&mut self, o: &Report) {
        self.cells += o.cells;
        for (k, v) in &o.counts {
            *self.counts.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &o.bases {
            *self.bases.entry(k.clone()).or_default() += v;
        }
        for e in &o.examples {
            if self
                .examples
                .iter()
                .filter(|x| x.split(':').next() == e.split(':').next())
                .count()
                < 6
            {
                self.examples.push(e.clone());
            }
        }
    }
}

struct Item {
    prim: Primitive,
    src: Src,
    lo: Vec3,
    hi: Vec3,
    solid: bool,
    drawn: bool,
    /// The chunk the engine meshes the piece in: 0 L2, 1 L1, 2 and up one per L3 cell.
    chunk: u32,
}

/// Uniform grid over the items near the cell (the cell box grown by 96 m).
struct Index {
    items: Vec<Item>,
    grid: HashMap<[i32; 3], Vec<u32>>,
}

const CS: f32 = 8.0;
const LO: f32 = -96.0;
const HI: f32 = 608.0;

fn cell_of(p: Vec3) -> [i32; 3] {
    [
        (p.x / CS).floor() as i32,
        (p.y / CS).floor() as i32,
        (p.z / CS).floor() as i32,
    ]
}

impl Index {
    fn new(items: Vec<Item>) -> Self {
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            if it.hi.cmplt(Vec3::splat(LO)).any() || it.lo.cmpgt(Vec3::splat(HI)).any() {
                continue;
            }
            let a = cell_of(it.lo.max(Vec3::splat(LO)));
            let b = cell_of(it.hi.min(Vec3::splat(HI)));
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        grid.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        Self { items, grid }
    }

    fn near(&self, p: Vec3) -> &[u32] {
        self.grid.get(&cell_of(p)).map_or(&[], Vec::as_slice)
    }

    /// The first solid (collidable) item containing `p`, other than `skip`.
    fn solid_at(&self, p: Vec3, skip: Option<u32>) -> Option<u32> {
        self.near(p).iter().copied().find(|&i| {
            let it = &self.items[i as usize];
            Some(i) != skip && it.solid && it.shape_contains(p)
        })
    }

    /// Whether any solid other than `skip` and of a kind not in `not` contains `p`.
    fn host_at(&self, p: Vec3, skip: u32, not: impl Fn(&Item) -> bool) -> bool {
        self.near(p).iter().any(|&i| {
            let it = &self.items[i as usize];
            i != skip && it.solid && !not(it) && it.shape_contains(p)
        })
    }

    fn drawn_at(&self, p: Vec3, skip: u32) -> Option<u32> {
        self.near(p).iter().copied().find(|&i| {
            let it = &self.items[i as usize];
            i != skip && it.drawn && it.shape_contains(p)
        })
    }
}

impl Item {
    fn shape_contains(&self, p: Vec3) -> bool {
        p.cmpge(self.lo - Vec3::splat(0.001)).all()
            && p.cmple(self.hi + Vec3::splat(0.001)).all()
            && self.prim.shape.contains(p)
    }
}

fn item(prim: Primitive, src: Src, chunk: u32) -> Item {
    let b = prim.shape.aabb();
    let is_shaft = matches!(prim.shape, Shape::LiftShaft { .. });
    let envelope =
        prim.flags.contains(PrimFlags::ENVELOPE) && !prim.flags.contains(PrimFlags::INTERIOR);
    Item {
        lo: b.min_m(),
        hi: b.max_m(),
        solid: !prim.flags.contains(PrimFlags::NO_COLLIDE) && !is_shaft,
        drawn: src != Src::Nbr
            && !is_shaft
            && !envelope
            && prim.kind != FeatureKind::VOID
            && prim.kind != FeatureKind::RAILING,
        prim,
        src,
        chunk,
    }
}

/// What one L2 cell contributes to its neighbours and itself, read once.
struct Loaded {
    own: Vec<Primitive>,
    /// Path, threshold and payoff pieces.
    paths: Vec<Primitive>,
    walks: Vec<Walk>,
    /// The clear volume of each rise and the pieces the rise built.
    rises: Vec<(Primitive, std::collections::HashSet<Primitive>)>,
    /// The walk each path piece was built for.
    owner: HashMap<Primitive, usize>,
    /// `(outside point, inside point, up)` of every door, cell-local metres.
    doors: Vec<(DVec3, DVec3, DVec3)>,
}

fn load(seed: WorldSeed, cache: &L1Cache, key: CellKey) -> Loaded {
    let t = tissue_cell(seed, cache.tissue(), key);
    let own = l2_cell_web(seed, cache, key).primitives;
    let mut paths: Vec<Primitive> = t.paths.prims.clone();
    paths.extend(t.thresholds.prims.iter().copied());
    paths.extend(t.payoffs.prims.iter().copied());
    let up = t.lattice.frame.axes()[1];
    let doors = doors_of(&t).into_iter().map(|(o, i)| (o, i, up)).collect();
    Loaded {
        own,
        paths,
        walks: t.paths.walks.clone(),
        owner: t
            .paths
            .owners
            .iter()
            .flat_map(|(r, w)| t.paths.prims[r.clone()].iter().map(move |p| (*p, *w)))
            .collect(),
        rises: t
            .paths
            .rises
            .iter()
            .map(|(v, r)| (*v, t.paths.prims[r.clone()].iter().copied().collect()))
            .collect(),
        doors,
    }
}

fn v(d: DVec3) -> Vec3 {
    Vec3::new(d.x as f32, d.y as f32, d.z as f32)
}

/// A box piece's centre, world axes and half extents, in metres.
fn box_of(p: &Primitive) -> Option<(Vec3, [Vec3; 3], Vec3)> {
    let (frame, origin, l) = as_box(&p.shape)?;
    let [ex, ey, ez] = frame.axes();
    let o = origin.to_metres();
    let (lo, hi) = (l.min.to_metres(), l.max.to_metres());
    let c = (lo + hi) * 0.5;
    let centre = o + v(ex) * c.x + v(ey) * c.y + v(ez) * c.z;
    Some((centre, [v(ex), v(ey), v(ez)], (hi - lo) * 0.5))
}

fn describe(p: &Primitive, at: Vec3) -> String {
    format!(
        "kind {} flags {} at ({:.1}, {:.1}, {:.1}) aabb {:?}",
        p.kind.0,
        p.flags.0,
        at.x,
        at.y,
        at.z,
        p.shape.aabb()
    )
}

/// Counts every defect in the cells `cells`, on `threads` threads. Each thread keeps its own
/// caches (they are not shared between threads) and loads a cell's 26 neighbours as witnesses
/// for what crosses its faces.
pub fn scan_cells(seed: WorldSeed, cells: &[I64Vec3], threads: usize) -> Report {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let total = Mutex::new(Report::default());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let cache = L1Cache::default();
                let mut loaded: HashMap<I64Vec3, Loaded> = HashMap::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&c) = cells.get(i) else { break };
                    if loaded.len() > 300 {
                        loaded.clear();
                    }
                    let r = scan_cell(seed, &cache, c, &mut loaded);
                    if let Ok(mut t) = total.lock() {
                        t.merge(&r);
                    }
                }
            });
        }
    });
    total.into_inner().unwrap_or_default()
}

/// The cells of the block `[min, min + size)`.
pub fn block(min: I64Vec3, size: I64Vec3) -> Vec<I64Vec3> {
    let mut v = Vec::new();
    for z in min.z..min.z + size.z {
        for y in min.y..min.y + size.y {
            for x in min.x..min.x + size.x {
                v.push(I64Vec3::new(x, y, z));
            }
        }
    }
    v
}

/// The defects of one cell, whose neighbours are in `loaded`.
fn scan_cell(
    seed: WorldSeed,
    cache: &L1Cache,
    c: I64Vec3,
    loaded: &mut HashMap<I64Vec3, Loaded>,
) -> Report {
    let key = CellKey::new(Level::L2, c);
    for dz in -1..=1i64 {
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                let n = c + I64Vec3::new(dx, dy, dz);
                loaded
                    .entry(n)
                    .or_insert_with(|| load(seed, cache, CellKey::new(Level::L2, n)));
            }
        }
    }
    let loaded = &*loaded;
    let me = &loaded[&c];
    let mut items: Vec<Item> = me.own.iter().map(|p| item(*p, Src::L2, 0)).collect();
    // L3: every world block of the cell, moved into the cell's frame.
    let mut ctx = web_context(seed, cache, key);
    for k in 0..512i64 {
        let (i, j, l) = (k % 8, k / 8 % 8, k / 64);
        let w = CellKey::new(Level::L3, c * 8 + I64Vec3::new(i, j, l));
        if !ctx.is_active(w) {
            continue;
        }
        let d = [i as i32 * 1024, j as i32 * 1024, l as i32 * 1024];
        for p in ctx.cell_primitives(seed, w) {
            items.push(item(translated(&p, d), Src::L3, 2 + k as u32));
        }
    }
    // L1 parent.
    let l1 = CellKey::new(Level::L1, c.div_euclid(I64Vec3::splat(8)));
    let local = c.rem_euclid(I64Vec3::splat(8));
    let d = [local.x, local.y, local.z].map(|n| -(n as i32) * 512 * 16);
    for p in &cache.get(seed, l1).primitives {
        let q = translated(p, d);
        let b = q.aabb();
        if b.max.x > -96 * 16
            && b.min.x < 608 * 16
            && b.max.y > -96 * 16
            && b.min.y < 608 * 16
            && b.max.z > -96 * 16
            && b.min.z < 608 * 16
        {
            items.push(item(q, Src::L1, 1));
        }
    }
    // Paths of the neighbours, as witnesses for what crosses a face.
    let mut nbr_paths = Vec::new();
    for dz in -1..=1i64 {
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                if (dx, dy, dz) == (0, 0, 0) {
                    continue;
                }
                let Some(n) = loaded.get(&(c + I64Vec3::new(dx, dy, dz))) else {
                    continue;
                };
                let d = [dx as i32 * 8192, dy as i32 * 8192, dz as i32 * 8192];
                for p in &n.paths {
                    let q = translated(p, d);
                    nbr_paths.push(q);
                    items.push(item(q, Src::Nbr, 9_999_999));
                }
            }
        }
    }
    let ix = Index::new(items);
    let mut r = Report {
        cells: 1,
        ..Default::default()
    };
    let inside = |p: Vec3| p.cmpge(Vec3::ZERO).all() && p.cmplt(Vec3::splat(512.0)).all();
    let paths_own: std::collections::HashSet<Primitive> = me.paths.iter().copied().collect();
    let nbr_set: std::collections::HashSet<Primitive> = nbr_paths.iter().copied().collect();

    for (n, it) in ix.items.iter().enumerate() {
        let n = n as u32;
        let p = &it.prim;
        let centre = (it.lo + it.hi) * 0.5;
        if it.src == Src::Nbr || it.src == Src::L1 || !inside(centre) {
            continue;
        }
        let bx = box_of(p);
        // a, i: rails.
        if p.kind == FeatureKind::RAILING {
            if let Shape::Oriented(o) = p.shape {
                if !crate::plumb::is_upright(o.frame) {
                    r.hit("a lean", || describe(p, centre));
                }
            }
            if let Some((c0, ax, half)) = bx {
                let up_idx = (0..3)
                    .max_by(|&a, &b| ax[a].y.abs().total_cmp(&ax[b].y.abs()))
                    .unwrap_or(1);
                let long = (0..3)
                    .filter(|&k| k != up_idx)
                    .max_by(|&a, &b| half[a].total_cmp(&half[b]))
                    .unwrap_or(0);
                r.base("a rails", 1);
                let along = Vec3::new(ax[long].x, 0.0, ax[long].z);
                // Only a rail that stands (a handrail slab floats above its stair).
                // A floor beside a rail at the cell's side is the neighbour's.
                let interior = (c0 - Vec3::splat(256.0)).abs().max_element() < 255.0;
                if half[long] >= 0.5 && along.length() > 0.5 && half[up_idx] >= 0.3 && interior {
                    let along = along.normalize();
                    let side = Vec3::new(-along.z, 0.0, along.x);
                    let y = c0.y - half[up_idx] - 0.1;
                    let mut blind = 0;
                    for f in [-0.75f32, 0.0, 0.75] {
                        let at = Vec3::new(c0.x, y, c0.z) + along * half[long] * f;
                        // A stair's rise along a run puts the floor up to 0.4 m under the foot.
                        let floor = |s: f32| {
                            [0.0f32, 0.4].iter().any(|&dy| {
                                let q = at + side * s - Vec3::Y * dy;
                                ix.near(q).iter().any(|&j| {
                                    let o = &ix.items[j as usize];
                                    o.solid
                                        && o.prim.kind != FeatureKind::RAILING
                                        && o.shape_contains(q)
                                })
                            })
                        };
                        if !floor(0.6) && !floor(-0.6) {
                            blind += 1;
                        }
                    }
                    if blind >= 2 {
                        // Where it comes from: a path's own rail (the walk's type) or the edge pass.
                        let origin = match me.owner.get(p) {
                            Some(&w) => format!("a walk {}", me.walks[w].id),
                            None if nbr_set.contains(p) => "a neighbour".to_string(),
                            None => "a edge".to_string(),
                        };
                        r.hit(&origin, || describe(p, c0));
                    }
                }
            } else if !matches!(
                p.shape,
                Shape::Ramp { .. }
                    | Shape::Oriented(crate::orient::Oriented {
                        local: crate::orient::LocalShape::Ramp { .. },
                        ..
                    })
            ) {
                r.hit("i", || describe(p, centre));
            }
            continue;
        }
        // c: dressing.
        if p.flags.contains(PrimFlags::NO_COLLIDE)
            && it.src == Src::L2
            && !matches!(p.kind, FeatureKind::VOID | FeatureKind::MISC)
            && !p.flags.contains(PrimFlags::ENVELOPE)
            && !p.flags.contains(PrimFlags::MID_ONLY)
            && p.kind != FeatureKind::LADDER
            && !matches!(p.shape, Shape::LiftShaft { .. })
        {
            r.base("c dressing", 1);
            let g = Vec3::splat(0.1);
            let hosts =
                |q: Vec3| ix.host_at(q, n, |o| o.prim.kind == p.kind && o.prim.flags == p.flags);
            let hosted = if p.kind == FeatureKind::CABLE {
                // A cable ends on solid at both ends.
                let size = it.hi - it.lo;
                let k = (0..3)
                    .max_by(|&a, &b| size[a].total_cmp(&size[b]))
                    .unwrap_or(1);
                let end = |hi: bool| {
                    let mut q = centre;
                    q[k] = if hi { it.hi[k] + 0.1 } else { it.lo[k] - 0.1 };
                    q
                };
                // An end on the cell's side goes on into the neighbour.
                let edge =
                    |q: Vec3| q.cmplt(Vec3::splat(0.2)).any() || q.cmpgt(Vec3::splat(511.8)).any();
                [end(false), end(true)].iter().all(|&q| edge(q) || hosts(q))
            } else {
                let (lo, hi) = (it.lo - g, it.hi + g);
                [
                    (lo + hi) * 0.5,
                    lo,
                    hi,
                    Vec3::new(lo.x, lo.y, hi.z),
                    Vec3::new(hi.x, lo.y, lo.z),
                    Vec3::new(lo.x, hi.y, lo.z),
                    Vec3::new(hi.x, hi.y, lo.z),
                    Vec3::new(lo.x, hi.y, hi.z),
                ]
                .iter()
                .any(|&q| hosts(q))
            };
            if !hosted {
                r.hit(&format!("c kind {}", p.kind.0), || describe(p, centre));
            }
            continue;
        }
        // d: vertical members.
        if it.solid && it.src != Src::L1 && !p.flags.contains(PrimFlags::INTERIOR) {
            if let Some((c0, ax, half)) = bx {
                if ax[1].y.abs() > 0.99 || (0..3).any(|k| ax[k].y.abs() > 0.99) {
                    let k = (0..3)
                        .max_by(|&a, &b| ax[a].y.abs().total_cmp(&ax[b].y.abs()))
                        .unwrap_or(1);
                    let h = half[k] * 2.0;
                    let others: Vec<f32> =
                        (0..3).filter(|&q| q != k).map(|q| half[q] * 2.0).collect();
                    if h >= 1.0
                        && others.iter().all(|&w| w <= 0.8)
                        && h >= 3.0 * others.iter().copied().fold(0.0, f32::max)
                        // What it stands on or hangs from at the cell's side is the neighbour's.
                        && c0.x > 1.0
                        && c0.z > 1.0
                        && c0.x < 511.0
                        && c0.z < 511.0
                        && it.lo.y > 1.0
                        && it.hi.y < 511.0
                    {
                        r.base("d members", 1);
                        let (bot, top) = (
                            Vec3::new(c0.x, it.lo.y - 0.05, c0.z),
                            Vec3::new(c0.x, it.hi.y + 0.05, c0.z),
                        );
                        if ix.solid_at(bot, Some(n)).is_none()
                            && ix.solid_at(top, Some(n)).is_none()
                        {
                            r.hit("d", || describe(p, c0));
                        }
                    }
                }
            }
        }
        // h: interior floors that are not level.
        if it.src == Src::L3 && it.solid {
            if let Some((c0, ax, half)) = bx {
                if let Some(k) = (0..3).min_by(|&a, &b| half[a].total_cmp(&half[b])) {
                    let slab =
                        half[k] * 2.0 <= 1.0 && (0..3).filter(|&q| q != k).all(|q| half[q] >= 1.0);
                    if slab && ax[k].y.abs() > 0.76 {
                        r.base("h floors", 1);
                        if ax[k].y.abs() < 0.9998 {
                            r.hit("h", || describe(p, c0));
                        }
                    }
                }
            }
        }
    }

    // g: coplanar faces.
    let eps = 0.003f64;
    for (n, it) in ix.items.iter().enumerate() {
        if !it.drawn || !inside((it.lo + it.hi) * 0.5) {
            continue;
        }
        for (corner, u, w, nrm) in crate::zfight::faces(&it.prim) {
            if u.length() < 0.2 || w.length() < 0.2 {
                continue;
            }
            for (fu, fv) in [(0.23, 0.31), (0.67, 0.19), (0.41, 0.77), (0.83, 0.58)] {
                let p = corner + u * fu + w * fv;
                let (out, inn) = (v(p + nrm * eps), v(p - nrm * eps));
                if !inside(v(p)) || !it.shape_contains(inn) || it.shape_contains(out) {
                    continue;
                }
                r.base("g faces", 1);
                if ix.drawn_at(out, n as u32).is_some() {
                    continue;
                }
                if let Some(b) = ix.drawn_at(inn, n as u32) {
                    let other = &ix.items[b as usize];
                    // Plain boxes of one chunk are culled and merged by the mesher.
                    if it.chunk == other.chunk
                        && it.prim.shape_axis_aligned()
                        && other.prim.shape_axis_aligned()
                    {
                        continue;
                    }
                    let (la, lb) = (it.src.min(other.src), it.src.max(other.src));
                    if it.src == Src::L2
                        && other.src == Src::L2
                        && [it.prim.kind, other.prim.kind]
                            .iter()
                            .any(|k| *k == FeatureKind::STAIR_TOWER || *k == FeatureKind::RAMP)
                    {
                        r.hit("g stair", || {
                            format!(
                                "{} against {}",
                                describe(&it.prim, v(p)),
                                describe(&other.prim, v(p))
                            )
                        });
                    }
                    r.hit(&format!("g {la:?}/{lb:?}"), || {
                        format!(
                            "{} against {}",
                            describe(&it.prim, v(p)),
                            describe(&other.prim, v(p))
                        )
                    });
                    break;
                }
            }
        }
    }

    // b, f: walks.
    let path_item = |i: u32| {
        let it = &ix.items[i as usize];
        it.solid && (paths_own.contains(&it.prim) || nbr_set.contains(&it.prim))
    };
    let own_rises: std::collections::HashSet<Primitive> = me
        .rises
        .iter()
        .flat_map(|(_, o)| o.iter().copied())
        .collect();
    for (walk_index, w) in me.walks.iter().enumerate() {
        let up = v(w.up);
        let pts = &w.points;
        // The ends of a walk are where it meets the stair, deck or door it leads on to: its steps
        // and landings stand in the headroom there, by design.
        let junction = w.width as f32 / 2.0 + 1.0;
        let samples: Vec<(Vec3, Vec3)> = if w.rise {
            let skip = 4.min(pts.len() / 2);
            pts[skip..pts.len() - skip]
                .iter()
                .map(|&p| (v(p), Vec3::ZERO))
                .collect()
        } else {
            let (a, z) = (v(pts[0]), v(pts[1]));
            let len = (z - a).length();
            let dir = (z - a) / len.max(1e-3);
            let lat = dir.cross(up).normalize_or_zero();
            let n = (len as usize).max(1);
            (0..=n)
                .filter(|&k| {
                    let s = len * k as f32 / n as f32;
                    s >= junction && s <= len - junction
                })
                .flat_map(|k| {
                    let at = a + dir * (len * k as f32 / n as f32);
                    [-0.3f32, 0.0, 0.3].map(|f| (at + lat * (f * w.width as f32), lat))
                })
                .collect()
        };
        let mut hit_by: std::collections::BTreeSet<u32> = Default::default();
        for (s, _) in &samples {
            // A sample on the cell's side is where a neighbour's walkway of another width
            // begins: nothing stands across it, but its rails start on that plane.
            if s.cmplt(Vec3::splat(0.5)).any() || s.cmpgt(Vec3::splat(511.5)).any() {
                continue;
            }
            r.base("b samples", 1);
            for dy in [0.3f32, 0.9, 1.5, 1.8] {
                let q = *s + up * dy;
                if let Some(i) = ix
                    .near(q)
                    .iter()
                    .copied()
                    .find(|&i| path_item(i) && ix.items[i as usize].shape_contains(q))
                {
                    hit_by.insert(i);
                }
            }
        }
        for i in hit_by {
            let it = &ix.items[i as usize];
            // A walk's own pieces (its steps, walls and roof) are its own business.
            if me.owner.get(&it.prim) == Some(&walk_index) {
                continue;
            }
            let name = if it.prim.kind == FeatureKind::RAILING {
                "b rail".to_string()
            } else {
                format!("b kind {}", it.prim.kind.0)
            };
            r.hit(&name, || {
                format!(
                    "PH/PV {} walk: {}",
                    w.id,
                    describe(&it.prim, (it.lo + it.hi) * 0.5)
                )
            });
        }
        // f: the ends.
        if w.rise {
            // A rise's foot and top: a floor that is not the rise's own within 3 m, any way.
            for (which, e) in [("foot", pts[0]), ("top", pts[pts.len() - 1])] {
                r.base("f rise ends", 1);
                let e = v(e);
                const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
                const HEADINGS: [(f32, f32); 8] = [
                    (1.0, 0.0),
                    (H, H),
                    (0.0, 1.0),
                    (-H, H),
                    (-1.0, 0.0),
                    (-H, -H),
                    (0.0, -1.0),
                    (H, -H),
                ];
                let found = HEADINGS.iter().any(|&(hx, hz)| {
                    [1.5f32, 2.25, 3.0].iter().any(|&d| {
                        let q = e + Vec3::new(hx * d, -0.1, hz * d);
                        ix.near(q).iter().any(|&i| {
                            let it = &ix.items[i as usize];
                            it.solid
                                && it.src != Src::L1
                                && !it.prim.flags.contains(PrimFlags::INTERIOR)
                                && !own_rises.contains(&it.prim)
                                && it.shape_contains(q)
                        })
                    })
                });
                if !found {
                    r.hit(&format!("f rise {which} id {}", w.id), || {
                        format!("PV {} end at ({:.1}, {:.1}, {:.1})", w.id, e.x, e.y, e.z)
                    });
                    r.hit("f rise", || String::new());
                }
            }
        }
        let ends: Vec<(Vec3, Vec3, Vec3)> = if w.rise {
            Vec::new()
        } else {
            let (a, z) = (v(pts[0]), v(pts[1]));
            let dir = (z - a).normalize_or_zero();
            let lat = dir.cross(up).normalize_or_zero();
            vec![(z, dir, lat), (a, -dir, lat)]
        };
        for (e, dir, lat) in ends {
            r.base("f ends", 1);
            let floor = |p: Vec3| ix.solid_at(p - up * 0.1, None).is_some();
            let reach = w.width as f32 / 2.0 + 0.8;
            let continues =
                floor(e + dir * 0.8) || floor(e + lat * reach) || floor(e - lat * reach);
            if continues {
                continue;
            }
            let blocked = ix.solid_at(e + dir * 0.6 + up * 1.0, None).is_some();
            r.hit(if blocked { "f solid" } else { "f air" }, || {
                format!("PH/PV {} end at ({:.1}, {:.1}, {:.1})", w.id, e.x, e.y, e.z)
            });
        }
    }
    // r: pieces that stand in the clear volume of a rise.
    for (volume, own) in &me.rises {
        let (lo, hi) = (volume.shape.aabb().min_m(), volume.shape.aabb().max_m());
        let mut seen: std::collections::BTreeSet<u32> = Default::default();
        let mut z = lo.z;
        while z < hi.z {
            let mut y = lo.y;
            while y < hi.y {
                let mut x = lo.x;
                while x < hi.x {
                    let p = Vec3::new(x, y, z);
                    if volume.shape.contains(p) {
                        for &i in ix.near(p) {
                            let it = &ix.items[i as usize];
                            if it.solid
                                && it.src != Src::Nbr
                                && it.src != Src::L1
                                && it.prim.kind != FeatureKind::RAILING
                                && !own.contains(&it.prim)
                                && !paths_own.contains(&it.prim)
                                && it.shape_contains(p)
                            {
                                seen.insert(i);
                            }
                        }
                    }
                    x += 1.5;
                }
                y += 1.5;
            }
            z += 1.5;
        }
        r.base("r rises", 1);
        for i in seen {
            let it = &ix.items[i as usize];
            let name = format!("r {:?}", it.src);
            r.hit(&name, || describe(&it.prim, (it.lo + it.hi) * 0.5));
        }
    }
    // e: doors.
    for (out, inn, _) in &me.doors {
        let (out, inn) = (v(*out), v(*inn));
        // The floor of a door at the cell's side is the neighbour's.
        if out.cmplt(Vec3::splat(1.0)).any() || out.cmpgt(Vec3::splat(511.0)).any() {
            continue;
        }
        r.base("e doors", 1);
        let floor = |p: Vec3| {
            ix.solid_at(p, None).is_some() || ix.solid_at(p - Vec3::Y * 0.2, None).is_some()
        };
        if !floor(out) {
            if std::env::var("DEFECT_DEBUG").is_ok() {
                for it in &ix.items {
                    let g = Vec3::splat(3.0);
                    if it.lo.cmple(out + g).all() && it.hi.cmpge(out - g).all() {
                        eprintln!(
                            "  near door out {out:?}: {:?} {:?} {:?}",
                            it.src,
                            it.prim.kind,
                            it.prim.shape.aabb()
                        );
                    }
                }
            }
            r.hit("e outside", || {
                format!(
                    "no floor outside at ({:.1}, {:.1}, {:.1})",
                    out.x, out.y, out.z
                )
            });
        }
        if !floor(inn) {
            r.hit("e inside", || {
                format!(
                    "no floor inside at ({:.1}, {:.1}, {:.1})",
                    inn.x, inn.y, inn.z
                )
            });
        }
    }
    for e in &mut r.examples {
        *e = format!("[cell {} {} {}] {e}", c.x, c.y, c.z);
    }
    r
}

impl Primitive {
    /// Whether the shape is exactly axis-aligned (a plain shape or an identity-like frame).
    fn shape_axis_aligned(&self) -> bool {
        match self.shape {
            Shape::Oriented(o) => o.frame.is_axis_aligned(),
            Shape::Ramp { .. } => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// A sampled census over a small block; the baseline for the later PW.4 items.
    #[test]
    fn census_runs() {
        let n: i64 = std::env::var("DEFECT_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let cells = block(I64Vec3::new(0, 6, 2), I64Vec3::new(n, 1, n));
        let r = scan_cells(WorldSeed(0), &cells, 4);
        eprintln!("{} cells", r.cells);
        for (k, v) in &r.counts {
            eprintln!("  {k}: {v}");
        }
        for (k, v) in &r.bases {
            eprintln!("  [{k}] {v}");
        }
        for e in &r.examples {
            eprintln!("  eg {e}");
        }
    }

    /// Prints places beside a level rail and a flight's rail, to look at: `SUPERSTRUCT_SECTOR`
    /// and `SUPERSTRUCT_LOCAL` for the game.
    #[test]
    #[ignore = "prints viewpoints"]
    fn print_rail_viewpoints() {
        let cache = L1Cache::default();
        let (mut level, mut flight) = (0, 0);
        for c in block(I64Vec3::new(0, 6, 2), I64Vec3::new(6, 1, 6)) {
            let out = l2_cell_web(WorldSeed(0), &cache, CellKey::new(Level::L2, c)).primitives;
            for p in out.iter().filter(|p| p.kind == FeatureKind::RAILING) {
                let ramp = matches!(p.shape, Shape::Ramp { .. })
                    || matches!(p.shape, Shape::Oriented(o) if matches!(o.local, crate::orient::LocalShape::Ramp { .. }));
                let b = p.aabb();
                let m = (b.min_m() + b.max_m()) * 0.5;
                let (n, name) = if ramp {
                    (&mut flight, "flight")
                } else {
                    (&mut level, "level")
                };
                if *n >= 3 {
                    continue;
                }
                *n += 1;
                let w = m + Vec3::new(c.x as f32 * 512.0, c.y as f32 * 512.0, c.z as f32 * 512.0);
                let sec = (w / 1024.0).floor();
                let loc = w - sec * 1024.0 + Vec3::new(5.0, 3.0, 5.0);
                eprintln!(
                    "{name}: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1}",
                    sec.x, sec.y, sec.z, loc.x, loc.y, loc.z
                );
            }
        }
    }

    /// Prints a place to look at each kind of stair: `SUPERSTRUCT_SECTOR`, `_LOCAL`, `_YAW`
    /// and `_PITCH` for the game.
    #[test]
    #[ignore = "prints viewpoints"]
    fn print_stair_viewpoints() {
        let cache = L1Cache::default();
        let mut want: Vec<u8> = std::env::var("PV_IDS").map_or_else(
            |_| vec![21, 31, 38, 39, 25],
            |v| v.split(',').filter_map(|x| x.parse().ok()).collect(),
        );
        for c in block(I64Vec3::new(-6, 4, -6), I64Vec3::new(14, 4, 14)) {
            if want.is_empty() {
                break;
            }
            let t = tissue_cell(WorldSeed(0), cache.tissue(), CellKey::new(Level::L2, c));
            for w in t.paths.walks.iter().filter(|w| w.rise) {
                let Some(k) = want.iter().position(|&id| id == w.id) else {
                    continue;
                };
                want.remove(k);
                let mid = w.points[w.points.len() / 2];
                let target =
                    mid + DVec3::new(c.x as f64 * 512.0, c.y as f64 * 512.0, c.z as f64 * 512.0);
                let eye = target + DVec3::new(7.0, 2.0, 7.0);
                let d = target - eye;
                let yaw = (-d.x).atan2(-d.z).to_degrees();
                let pitch = (d.y / d.length()).asin().to_degrees();
                let sec = (eye / 1024.0).floor();
                let loc = eye - sec * 1024.0;
                eprintln!(
                    "PV id {}: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW={yaw:.0} SUPERSTRUCT_PITCH={pitch:.0}",
                    w.id, sec.x, sec.y, sec.z, loc.x, loc.y, loc.z
                );
            }
        }
    }

    /// How far from each door the floor outside starts (a probe along the way out).
    #[test]
    #[ignore = "prints"]
    fn door_floor_distance() {
        let cache = L1Cache::default();
        let key = CellKey::new(Level::L2, I64Vec3::new(0, 6, 2));
        let t = tissue_cell(WorldSeed(0), cache.tissue(), key);
        let prims = &t.paths.prims;
        let solid = |p: Vec3| {
            prims
                .iter()
                .chain(t.thresholds.prims.iter())
                .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
        };
        for (o, i) in doors_of(&t).into_iter().take(12) {
            let d = (o - i).normalize();
            let mut found = None;
            for k in 0..40 {
                let p = v(i + d * (2.0 + f64::from(k) * 0.5) - DVec3::Y * 0.2);
                if solid(p) {
                    found = Some(2.0 + f64::from(k) * 0.5 - 2.0);
                    break;
                }
            }
            eprintln!(
                "door at ({:.0},{:.0},{:.0}): floor starts {:?} m outside the wall",
                o.x, o.y, o.z, found
            );
        }
    }

    /// Prints a place beside an open ring, a flat-roofed one and one with an oculus.
    #[test]
    #[ignore = "prints viewpoints"]
    fn print_ring_viewpoints() {
        use crate::tissue_grow::Shaped;
        let cache = L1Cache::default();
        let mut want = vec![(16usize, "open"), (17, "flat roof"), (32, "oculus")];
        for c in block(I64Vec3::new(-6, 4, -6), I64Vec3::new(14, 4, 14)) {
            if want.is_empty() {
                break;
            }
            let t = tissue_cell(WorldSeed(0), cache.tissue(), CellKey::new(Level::L2, c));
            for s in &t.structures {
                for (sh, b) in &s.shaped {
                    let Shaped::Ring(n) = sh else { continue };
                    if *n != 16 {
                        continue;
                    }
                    let mut e = crate::tissue_grow::Emitter::new(&t.lattice);
                    e.ring(b, *n, PrimFlags::NONE);
                    let Some(k) = want.iter().position(|w| w.0 == e.out.len()) else {
                        continue;
                    };
                    let (_, name) = want.remove(k);
                    let pitch = t.lattice.pitch as f64;
                    let top = e.at([
                        (f64::from(b.lo[0] + b.hi[0]) / 2.0) * pitch,
                        f64::from(b.hi[1]) * pitch + 12.0,
                        (f64::from(b.lo[2] + b.hi[2]) / 2.0) * pitch + 30.0,
                    ]);
                    let w = top
                        + DVec3::new(c.x as f64 * 512.0, c.y as f64 * 512.0, c.z as f64 * 512.0);
                    let sec = (w / 1024.0).floor();
                    let loc = w - sec * 1024.0;
                    eprintln!(
                        "RING {name}: SUPERSTRUCT_SECTOR={},{},{} SUPERSTRUCT_LOCAL={:.1},{:.1},{:.1} SUPERSTRUCT_YAW=180 SUPERSTRUCT_PITCH=-35",
                        sec.x, sec.y, sec.z, loc.x, loc.y, loc.z
                    );
                }
            }
        }
    }
}
