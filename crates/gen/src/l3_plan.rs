//! Mass plans for L3 (`docs/spec/levels.md` §6.3 and §6.4).
//!
//! Every block that meets a mass computes the same plan from the parent L2 output, so blocks
//! agree without communicating. The plan lives on the L2 cell's own slot lattice: 64 columns by
//! 64 columns of 8 m, and 128 storeys of 4 m.

// Columns, storeys and draws are small bounded numbers.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::cell::CellKey;
use crate::hash::hash;
use crate::l3_arch::Archetype;
use crate::prim::{Q, QAabb};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Column width in quanta (8 m) and storey height (4 m).
pub const COL: Q = 128;
pub const STOREY: Q = 64;
/// Directions in side order: -z, +x, +z, -x.
pub const DELTA: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// What kind of way in an entrance is (`levels.md` §6W.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntranceKind {
    /// A doorway in a side wall, at any storey.
    Door,
    /// An opening in the roof of the top storey, to a ladder well.
    RoofHatch,
    /// An opening in the floor of the bottom storey.
    FloorHatch,
    /// A ladder port: a hatch whose way up or down is a ladder well.
    Ladder,
}

/// An entrance of a mass, on the L2 cell's lattice: a vestibule on one or two columns of one
/// storey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrance {
    pub kind: EntranceKind,
    /// The face a door is in: 0 = -z, 1 = +x, 2 = +z, 3 = -x (0 for hatches).
    pub side: u8,
    /// A door's range along its face (z for x faces, x for z faces), 4 m wide; for hatches the
    /// 4 m by 4 m opening's x range.
    pub u0: Q,
    pub u1: Q,
    /// The vestibule columns: the columns the opening overlaps, one or two.
    pub cols: Vec<(i32, i32)>,
    /// The storey of the vestibule (the lattice's, not the mass's).
    pub t: i32,
}

/// The IN columns of a mass: a rectangle `[cx0, cx1) × [cz0, cz1)` of 8 m columns, world-aligned
/// and fully inside the footprint. Empty when the footprint holds no whole column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cols {
    pub cx0: i32,
    pub cx1: i32,
    pub cz0: i32,
    pub cz1: i32,
}

impl Cols {
    pub fn of(aabb: &QAabb) -> Self {
        Self {
            cx0: (aabb.min.x + COL - 1).div_euclid(COL),
            cx1: aabb.max.x.div_euclid(COL),
            cz0: (aabb.min.z + COL - 1).div_euclid(COL),
            cz1: aabb.max.z.div_euclid(COL),
        }
    }

    pub fn nx(&self) -> i32 {
        (self.cx1 - self.cx0).max(0)
    }

    pub fn nz(&self) -> i32 {
        (self.cz1 - self.cz0).max(0)
    }

    pub fn count(&self) -> i32 {
        self.nx() * self.nz()
    }

    pub fn contains(&self, c: (i32, i32)) -> bool {
        (self.cx0..self.cx1).contains(&c.0) && (self.cz0..self.cz1).contains(&c.1)
    }

    /// Index of a column inside the rectangle.
    fn index(&self, c: (i32, i32)) -> usize {
        ((c.1 - self.cz0) * self.nx() + (c.0 - self.cx0)) as usize
    }

    fn at(&self, i: usize) -> (i32, i32) {
        let i = i as i32;
        (self.cx0 + i % self.nx(), self.cz0 + i / self.nx())
    }

    /// The outermost row of columns facing `side`, in increasing `u`.
    pub fn row(&self, side: u8) -> Vec<(i32, i32)> {
        match side {
            0 => (self.cx0..self.cx1).map(|x| (x, self.cz0)).collect(),
            2 => (self.cx0..self.cx1).map(|x| (x, self.cz1 - 1)).collect(),
            1 => (self.cz0..self.cz1).map(|z| (self.cx1 - 1, z)).collect(),
            _ => (self.cz0..self.cz1).map(|z| (self.cx0, z)).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanClass {
    /// No entrance, or no whole column: every `Core` slot is solid.
    Sealed,
    /// One IN column: a dead-end entrance hall on storey 0, solid above.
    VestibuleOnly,
    Full,
}

/// A spine: a spiral stair through every storey of one column, with its door towards `d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spine {
    pub col: (i32, i32),
    /// The side of the spine column that carries the door, as a direction index.
    pub rot: u8,
    /// The neighbour column the door opens onto.
    pub d: (i32, i32),
}

#[derive(Debug, Clone)]
pub struct MassPlan {
    pub aabb: QAabb,
    pub cols: Cols,
    /// Storey range `[t0, t1)` on the L2 cell's storey lattice.
    pub t0: i32,
    pub t1: i32,
    pub class: PlanClass,
    pub entrances: Vec<Entrance>,
    pub archetype: Archetype,
    /// One to four spines, in draw order.
    pub spines: Vec<Spine>,
    /// Per storey (from `t0`) and column: bit `k` set when the skeleton has an edge to the
    /// neighbour in direction `k`.
    skeleton: Vec<Vec<u8>>,
}

fn permutation(mut n: u32) -> [usize; 4] {
    let mut pool = vec![0usize, 1, 2, 3];
    let mut out = [0usize; 4];
    for (k, fact) in [6u32, 2, 1, 1].into_iter().enumerate() {
        let i = (n / fact) as usize;
        n %= fact;
        out[k] = pool.remove(i);
    }
    out
}

impl MassPlan {
    /// The plan of mass `index` of the L2 cell `parent`, with all its entrances (§6W.3).
    pub fn new(
        seed: WorldSeed,
        parent: CellKey,
        index: usize,
        aabb: QAabb,
        entrances: Vec<Entrance>,
        archetype: Archetype,
    ) -> Self {
        let cols = Cols::of(&aabb);
        let (t0, t1) = (aabb.min.y.div_euclid(STOREY), aabb.max.y.div_euclid(STOREY));
        let mut plan = Self {
            aabb,
            cols,
            t0,
            t1,
            class: PlanClass::Sealed,
            entrances: Vec::new(),
            archetype,
            spines: Vec::new(),
            skeleton: Vec::new(),
        };
        // An entrance whose vestibule is not inside the columns and storeys does not count.
        let entrances: Vec<Entrance> = entrances
            .into_iter()
            .filter(|e| (t0..t1).contains(&e.t) && e.cols.iter().all(|&c| cols.contains(c)))
            .collect();
        if entrances.is_empty() {
            return plan;
        }
        plan.entrances = entrances;
        if cols.count() == 1 {
            plan.class = PlanClass::VestibuleOnly;
            return plan;
        }
        plan.class = PlanClass::Full;
        let mut seeds = Rng::new(hash(
            seed.0,
            crate::cell::Level::L2,
            parent.coords,
            salt::L3_MASS,
        ));
        let mut s_i = 0;
        for _ in 0..=index {
            s_i = seeds.next_u64();
        }
        let mut rng = Rng::new(s_i);
        plan.build_full(&mut rng);
        plan
    }

    fn neighbour(&self, c: (i32, i32), k: usize) -> Option<(i32, i32)> {
        let n = (c.0 + DELTA[k].0, c.1 + DELTA[k].1);
        self.cols.contains(n).then_some(n)
    }

    /// Whether the columns of IN that are not `removed` form one 4-connected set.
    fn connected_without(&self, removed: &[(i32, i32)]) -> bool {
        let n = self.cols.count() as usize;
        let gone: Vec<bool> = (0..n).map(|i| removed.contains(&self.cols.at(i))).collect();
        let Some(start) = (0..n).find(|&i| !gone[i]) else {
            return true;
        };
        let mut seen = vec![false; n];
        seen[start] = true;
        let mut stack = vec![self.cols.at(start)];
        let mut count = 1;
        while let Some(c) = stack.pop() {
            for k in 0..4 {
                let Some(nb) = self.neighbour(c, k) else {
                    continue;
                };
                let i = self.cols.index(nb);
                if !gone[i] && !seen[i] {
                    seen[i] = true;
                    count += 1;
                    stack.push(nb);
                }
            }
        }
        count == gone.iter().filter(|&&g| !g).count()
    }

    fn is_vestibule_col(&self, c: (i32, i32)) -> bool {
        self.entrances.iter().any(|e| e.cols.contains(&c))
    }

    #[allow(clippy::too_many_lines)] // spines, then a tree per storey
    fn build_full(&mut self, rng: &mut Rng) {
        // Spines: one to four, each a column that is not a vestibule column and whose removal,
        // with the earlier spines, keeps the rest connected.
        let want = ((1 + self.cols.count() / 16) as usize + self.archetype.extra_spines()).min(4);
        let mut taken: Vec<(i32, i32)> = Vec::new();
        for _ in 0..want {
            let cands: Vec<(i32, i32)> = (0..self.cols.count() as usize)
                .map(|i| self.cols.at(i))
                .filter(|&c| !self.is_vestibule_col(c) && !taken.contains(&c))
                .filter(|&c| {
                    let mut gone = taken.clone();
                    gone.push(c);
                    self.connected_without(&gone)
                })
                .collect();
            if cands.is_empty() {
                break;
            }
            let s = cands[rng.below(cands.len() as u64) as usize];
            taken.push(s);
            let dirs: Vec<usize> = (0..4)
                .filter(|&k| self.neighbour(s, k).is_some_and(|n| !taken.contains(&n)))
                .collect();
            if dirs.is_empty() {
                taken.pop();
                continue;
            }
            let rot = dirs[rng.below(dirs.len() as u64) as usize];
            let d = self.neighbour(s, rot).unwrap_or(s);
            self.spines.push(Spine {
                col: s,
                rot: rot as u8,
                d,
            });
        }
        // A door neighbour that a later spine took is moved to the nearest column that is not
        // a spine, so every spine has a door onto the tree.
        let cols_of_spines: Vec<(i32, i32)> = self.spines.iter().map(|s| s.col).collect();
        for sp in &mut self.spines {
            if cols_of_spines.contains(&sp.d) {
                for (k, delta) in DELTA.iter().enumerate() {
                    let n = (sp.col.0 + delta.0, sp.col.1 + delta.1);
                    if self.cols.contains(n) && !cols_of_spines.contains(&n) {
                        sp.rot = k as u8;
                        sp.d = n;
                        break;
                    }
                }
            }
        }
        self.spines.retain(|s| !cols_of_spines.contains(&s.d));
        let Some(first) = self.spines.first().copied() else {
            // No spine can stand (a one-column-wide mass of vestibules): the vestibules are
            // the skeleton, as in a vestibule-only plan.
            self.class = PlanClass::VestibuleOnly;
            return;
        };
        let targets = self.static_targets();
        let n = self.cols.count() as usize;
        let storeys = (self.t1 - self.t0) as usize;
        let spine_cols: Vec<(i32, i32)> = self.spines.iter().map(|s| s.col).collect();
        let mut sk = Vec::with_capacity(storeys);
        for t in 0..storeys {
            let perm = permutation(rng.below(24) as u32);
            let mut mask = vec![0u8; n];
            // BFS from the first spine's door over IN minus the spines, expanding neighbours
            // in the drawn order.
            let mut parent: Vec<Option<(i32, i32)>> = vec![None; n];
            let mut seen = vec![false; n];
            for c in &spine_cols {
                seen[self.cols.index(*c)] = true;
            }
            let mut queue = std::collections::VecDeque::new();
            seen[self.cols.index(first.d)] = true;
            queue.push_back(first.d);
            // Radial and twisted plans grow along spokes first: the row and column through the
            // spine, or the one ray that turns a quarter every storey, then the rest.
            let spoke = |c: (i32, i32)| match self.archetype {
                Archetype::Radial => c.0 == first.col.0 || c.1 == first.col.1,
                Archetype::Twisted => {
                    let (dx, dz) = (c.0 - first.col.0, c.1 - first.col.1);
                    match t % 4 {
                        0 => dz == 0 && dx >= 0,
                        1 => dx == 0 && dz >= 0,
                        2 => dz == 0 && dx <= 0,
                        _ => dx == 0 && dz <= 0,
                    }
                }
                _ => false,
            };
            let spokes = matches!(self.archetype, Archetype::Radial | Archetype::Twisted);
            let mut later = std::collections::VecDeque::new();
            while let Some(c) = queue.pop_front().or_else(|| later.pop_front()) {
                for &k in &perm {
                    let Some(nb) = self.neighbour(c, k) else {
                        continue;
                    };
                    let i = self.cols.index(nb);
                    if !seen[i] {
                        seen[i] = true;
                        parent[i] = Some(c);
                        if !spokes || spoke(nb) {
                            queue.push_back(nb);
                        } else {
                            later.push_back(nb);
                        }
                    }
                }
            }
            let mut want: Vec<(i32, i32)> = targets.clone();
            want.extend(self.spines.iter().map(|s| s.d));
            let here = self.t0 + t as i32;
            for e in self.entrances.iter().filter(|e| e.t == here) {
                want.extend(e.cols.iter().copied());
            }
            for target in want {
                let mut at = target;
                while let Some(p) = parent[self.cols.index(at)] {
                    self.link(&mut mask, at, p);
                    at = p;
                }
            }
            // Every spine's door edge at every storey.
            for sp in &self.spines {
                self.link(&mut mask, sp.col, sp.d);
            }
            sk.push(mask);
        }
        self.skeleton = sk;
    }

    fn link(&self, mask: &mut [u8], a: (i32, i32), b: (i32, i32)) {
        for (k, delta) in DELTA.iter().enumerate() {
            if (a.0 + delta.0, a.1 + delta.1) == b {
                mask[self.cols.index(a)] |= 1 << k;
                mask[self.cols.index(b)] |= 1 << ((k + 2) % 4);
            }
        }
    }

    /// Per block column range and component of IN minus the spines, the column nearest the
    /// first spine's door.
    fn static_targets(&self) -> Vec<(i32, i32)> {
        let c = &self.cols;
        let d = self.spines[0].d;
        let spine_cols: Vec<(i32, i32)> = self.spines.iter().map(|s| s.col).collect();
        let mut targets = vec![d];
        for bx in c.cx0.div_euclid(8)..=(c.cx1 - 1).div_euclid(8) {
            for bz in c.cz0.div_euclid(8)..=(c.cz1 - 1).div_euclid(8) {
                let inside = |p: (i32, i32)| {
                    c.contains(p)
                        && !spine_cols.contains(&p)
                        && p.0.div_euclid(8) == bx
                        && p.1.div_euclid(8) == bz
                };
                let mut done: Vec<(i32, i32)> = Vec::new();
                for z in c.cz0.max(bz * 8)..c.cz1.min(bz * 8 + 8) {
                    for x in c.cx0.max(bx * 8)..c.cx1.min(bx * 8 + 8) {
                        if !inside((x, z)) || done.contains(&(x, z)) {
                            continue;
                        }
                        // Flood fill the component, keeping the column nearest d.
                        let mut comp = vec![(x, z)];
                        let mut best = (x, z);
                        let mut i = 0;
                        while i < comp.len() {
                            let p = comp[i];
                            i += 1;
                            let dist =
                                |q: (i32, i32)| ((q.0 - d.0).abs() + (q.1 - d.1).abs(), q.1, q.0);
                            if dist(p) < dist(best) {
                                best = p;
                            }
                            for delta in DELTA {
                                let q = (p.0 + delta.0, p.1 + delta.1);
                                if inside(q) && !comp.contains(&q) {
                                    comp.push(q);
                                }
                            }
                        }
                        done.extend(comp.iter().copied());
                        if !targets.contains(&best) {
                            targets.push(best);
                        }
                    }
                }
            }
        }
        targets
    }

    /// The entrances whose vestibule is the slot `(col, storey)`.
    pub fn entrances_at(&self, col: (i32, i32), storey: i32) -> impl Iterator<Item = &Entrance> {
        self.entrances
            .iter()
            .filter(move |e| e.t == storey && e.cols.contains(&col))
    }

    pub fn is_vestibule(&self, col: (i32, i32), storey: i32) -> bool {
        self.entrances_at(col, storey).next().is_some()
    }

    pub fn spine_at(&self, col: (i32, i32)) -> Option<&Spine> {
        self.spines.iter().find(|s| s.col == col)
    }

    /// Whether the slot is part of the skeleton: spine, vestibule or a tree slot.
    pub fn in_skeleton(&self, col: (i32, i32), storey: i32) -> bool {
        match self.class {
            PlanClass::Sealed => false,
            PlanClass::VestibuleOnly => self.is_vestibule(col, storey),
            PlanClass::Full => {
                self.spine_at(col).is_some()
                    || self.is_vestibule(col, storey)
                    || self.edges(col, storey) != 0
            }
        }
    }

    /// The skeleton edge directions of a slot, as a bit mask over `DELTA` (0 outside a Full
    /// plan or its storeys).
    pub fn edges(&self, col: (i32, i32), storey: i32) -> u8 {
        if self.class != PlanClass::Full || !self.cols.contains(col) {
            return 0;
        }
        let Some(row) = self.skeleton.get((storey - self.t0) as usize) else {
            return 0;
        };
        row[self.cols.index(col)]
    }
}
