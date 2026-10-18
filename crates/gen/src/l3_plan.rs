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
use crate::prim::{Q, QAabb};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Column width in quanta (8 m) and storey height (4 m).
pub const COL: Q = 128;
pub const STOREY: Q = 64;
/// Directions in side order: -z, +x, +z, -x.
pub const DELTA: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// A doorway in a mass face (`docs/spec/levels.md` §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Doorway {
    /// The face it is in: 0 = -z, 1 = +x, 2 = +z, 3 = -x.
    pub side: u8,
    /// Along the face (z for x faces, x for z faces): 4 m wide inside one IN column.
    pub u0: Q,
    pub u1: Q,
    /// Floor height outside, the mass bottom.
    pub floor: Q,
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

/// The column of `cols` behind a doorway: the IN column of the facing row whose range holds it.
pub fn entrance_column(cols: &Cols, door: &Doorway) -> Option<(i32, i32)> {
    cols.row(door.side).into_iter().find(|&(cx, cz)| {
        let u = if door.side.is_multiple_of(2) { cx } else { cz };
        u * COL <= door.u0 && door.u1 <= (u + 1) * COL
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanClass {
    /// No entrance, or no whole column: every `Core` slot is solid.
    Sealed,
    /// One IN column: a dead-end entrance hall on storey 0, solid above.
    VestibuleOnly,
    Full,
}

/// The spine: a spiral stair through every storey of one column, with its door towards `d`.
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
    pub doorway: Option<Doorway>,
    pub entrance_col: Option<(i32, i32)>,
    pub spine: Option<Spine>,
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
    /// The plan of mass `index` (in `Envelope` order) of the L2 cell `parent`.
    pub fn new(
        seed: WorldSeed,
        parent: CellKey,
        index: usize,
        aabb: QAabb,
        doorway: Option<Doorway>,
    ) -> Self {
        let cols = Cols::of(&aabb);
        let (t0, t1) = (aabb.min.y.div_euclid(STOREY), aabb.max.y.div_euclid(STOREY));
        let mut plan = Self {
            aabb,
            cols,
            t0,
            t1,
            class: PlanClass::Sealed,
            doorway,
            entrance_col: None,
            spine: None,
            skeleton: Vec::new(),
        };
        let Some(door) = doorway else {
            return plan;
        };
        let Some(ce) = entrance_column(&cols, &door) else {
            return plan;
        };
        plan.entrance_col = Some(ce);
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
        plan.build_full(ce, &mut rng);
        plan
    }

    fn neighbour(&self, c: (i32, i32), k: usize) -> Option<(i32, i32)> {
        let n = (c.0 + DELTA[k].0, c.1 + DELTA[k].1);
        self.cols.contains(n).then_some(n)
    }

    /// IN columns whose removal leaves the rest 4-connected, other than `ce`.
    fn spine_candidates(&self, ce: (i32, i32)) -> Vec<(i32, i32)> {
        let c = &self.cols;
        let mut out = Vec::new();
        for i in 0..c.count() as usize {
            let col = c.at(i);
            if col == ce {
                continue;
            }
            let strip = c.nx() == 1 || c.nz() == 1;
            let end = if c.nx() == 1 {
                col.1 == c.cz0 || col.1 == c.cz1 - 1
            } else {
                col.0 == c.cx0 || col.0 == c.cx1 - 1
            };
            if !strip || end {
                out.push(col);
            }
        }
        out
    }

    fn build_full(&mut self, ce: (i32, i32), rng: &mut Rng) {
        let cands = self.spine_candidates(ce);
        let s = cands[rng.below(cands.len() as u64) as usize];
        let dirs: Vec<usize> = (0..4).filter(|&k| self.neighbour(s, k).is_some()).collect();
        let rot = dirs[rng.below(dirs.len() as u64) as usize];
        let Some(d) = self.neighbour(s, rot) else {
            return;
        };
        self.spine = Some(Spine {
            col: s,
            rot: rot as u8,
            d,
        });
        let targets = self.static_targets(s, d);
        let n = self.cols.count() as usize;
        let storeys = (self.t1 - self.t0) as usize;
        let mut sk = Vec::with_capacity(storeys);
        for t in 0..storeys {
            let perm = permutation(rng.below(24) as u32);
            let mut mask = vec![0u8; n];
            // BFS from d over IN minus the spine, expanding neighbours in the drawn order.
            let mut parent: Vec<Option<(i32, i32)>> = vec![None; n];
            let mut seen = vec![false; n];
            let mut queue = std::collections::VecDeque::new();
            seen[self.cols.index(d)] = true;
            seen[self.cols.index(s)] = true;
            queue.push_back(d);
            while let Some(c) = queue.pop_front() {
                for &k in &perm {
                    let Some(nb) = self.neighbour(c, k) else {
                        continue;
                    };
                    let i = self.cols.index(nb);
                    if !seen[i] {
                        seen[i] = true;
                        parent[i] = Some(c);
                        queue.push_back(nb);
                    }
                }
            }
            let mut want = targets.clone();
            if t == 0 {
                want.push(ce);
            }
            for target in want {
                let mut at = target;
                while let Some(p) = parent[self.cols.index(at)] {
                    self.link(&mut mask, at, p);
                    at = p;
                }
            }
            // The spine's door edge at every storey.
            self.link(&mut mask, s, d);
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

    /// Per block column range and component of IN∖{s}, the column nearest `d`.
    fn static_targets(&self, s: (i32, i32), d: (i32, i32)) -> Vec<(i32, i32)> {
        let c = &self.cols;
        let mut targets = vec![d];
        for bx in c.cx0.div_euclid(8)..=(c.cx1 - 1).div_euclid(8) {
            for bz in c.cz0.div_euclid(8)..=(c.cz1 - 1).div_euclid(8) {
                let inside = |p: (i32, i32)| {
                    c.contains(p) && p != s && p.0.div_euclid(8) == bx && p.1.div_euclid(8) == bz
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

    /// Whether the slot is part of the skeleton: spine, vestibule or a tree slot.
    pub fn in_skeleton(&self, col: (i32, i32), storey: i32) -> bool {
        match self.class {
            PlanClass::Sealed => false,
            PlanClass::VestibuleOnly => storey == self.t0 && Some(col) == self.entrance_col,
            PlanClass::Full => {
                if Some(col) == self.spine.map(|s| s.col) {
                    return true;
                }
                if storey == self.t0 && Some(col) == self.entrance_col {
                    return true;
                }
                self.edges(col, storey) != 0
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

#[cfg(test)]
mod tests {
    use glam::I64Vec3;

    use super::*;
    use crate::cell::Level;
    use crate::prim::QVec3;

    fn mass(x: (Q, Q), y: (Q, Q), z: (Q, Q)) -> QAabb {
        QAabb::new(QVec3::new(x.0, y.0, z.0), QVec3::new(x.1, y.1, z.1))
    }

    fn door_at(aabb: &QAabb, side: u8) -> Doorway {
        let cols = Cols::of(aabb);
        let col = cols.row(side)[0];
        let u = if side.is_multiple_of(2) { col.0 } else { col.1 };
        Doorway {
            side,
            u0: u * COL + 32,
            u1: u * COL + 96,
            floor: aabb.min.y,
        }
    }

    #[test]
    fn columns_are_the_whole_aligned_columns_inside_the_footprint() {
        let a = mass(
            (1024 + 64, 1024 + 64 + 64 * 20),
            (0, 640),
            (2048, 2048 + 64 * 9),
        );
        let c = Cols::of(&a);
        // x: 1088..2368 holds columns 9..18 (1152..2304); z: 2048..2624 holds 16..20.
        assert_eq!((c.cx0, c.cx1, c.cz0, c.cz1), (9, 18, 16, 20));
        let thin = mass((100, 200), (0, 256), (0, 1024));
        assert_eq!(Cols::of(&thin).count(), 0);
    }

    #[test]
    fn permutations_cover_all_24_orders() {
        let mut seen = std::collections::BTreeSet::new();
        for n in 0..24 {
            let p = permutation(n);
            let mut s = p;
            s.sort_unstable();
            assert_eq!(s, [0, 1, 2, 3]);
            seen.insert(p);
        }
        assert_eq!(seen.len(), 24);
        assert_eq!(permutation(0), [0, 1, 2, 3]);
        assert_eq!(permutation(23), [3, 2, 1, 0]);
    }

    #[test]
    fn plan_classes() {
        let seed = WorldSeed(5);
        let parent = CellKey::new(Level::L2, I64Vec3::new(1, 2, 3));
        let big = mass(
            (1024, 1024 + 128 * 6),
            (640, 640 + 64 * 8),
            (1024, 1024 + 128 * 5),
        );
        assert_eq!(
            MassPlan::new(seed, parent, 0, big, None).class,
            PlanClass::Sealed
        );
        let p = MassPlan::new(seed, parent, 0, big, Some(door_at(&big, 0)));
        assert_eq!(p.class, PlanClass::Full);
        let single = mass((1024, 1024 + 128), (640, 640 + 64 * 4), (1024, 1024 + 128));
        let p = MassPlan::new(seed, parent, 0, single, Some(door_at(&single, 2)));
        assert_eq!(p.class, PlanClass::VestibuleOnly);
        assert!(p.in_skeleton((8, 8), 10) && !p.in_skeleton((8, 8), 11));
    }

    #[test]
    fn skeleton_is_connected_on_every_storey_and_reaches_every_block_part() {
        let seed = WorldSeed(11);
        let parent = CellKey::new(Level::L2, I64Vec3::new(4, 1, -2));
        for (k, (nx, nz, side)) in [(6, 5, 0u8), (16, 9, 1), (1, 7, 3), (2, 2, 2), (9, 1, 0)]
            .into_iter()
            .enumerate()
        {
            let a = mass(
                (1024, 1024 + 128 * nx),
                (640, 640 + 64 * 6),
                (3 * 128, 3 * 128 + 128 * nz),
            );
            let door = door_at(&a, side);
            let p = MassPlan::new(seed, parent, k, a, Some(door));
            if p.class != PlanClass::Full {
                continue;
            }
            let s = p.spine.unwrap();
            for t in p.t0..p.t1 {
                // Flood fill over skeleton edges from d.
                let mut seen = vec![s.d];
                let mut i = 0;
                while i < seen.len() {
                    let c = seen[i];
                    i += 1;
                    for (dir, delta) in DELTA.iter().enumerate() {
                        if p.edges(c, t) >> dir & 1 == 1 {
                            let n = (c.0 + delta.0, c.1 + delta.1);
                            if !seen.contains(&n) {
                                seen.push(n);
                            }
                        }
                    }
                }
                assert!(seen.contains(&s.col), "door edge missing at storey {t}");
                // Every slot with an edge is in the connected skeleton.
                for z in p.cols.cz0..p.cols.cz1 {
                    for x in p.cols.cx0..p.cols.cx1 {
                        if p.edges((x, z), t) != 0 {
                            assert!(
                                seen.contains(&(x, z)),
                                "stray skeleton slot at {x},{z} storey {t}"
                            );
                        }
                    }
                }
                if t == p.t0 {
                    assert!(seen.contains(&p.entrance_col.unwrap()));
                }
                // Every block column range keeps a skeleton slot.
                for bx in p.cols.cx0.div_euclid(8)..=(p.cols.cx1 - 1).div_euclid(8) {
                    for bz in p.cols.cz0.div_euclid(8)..=(p.cols.cz1 - 1).div_euclid(8) {
                        let any = seen
                            .iter()
                            .chain(std::iter::once(&s.col))
                            .any(|c| c.0.div_euclid(8) == bx && c.1.div_euclid(8) == bz);
                        assert!(
                            any,
                            "block range ({bx}, {bz}) has no skeleton slot at storey {t}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn plans_are_pure() {
        let seed = WorldSeed(3);
        let parent = CellKey::new(Level::L2, I64Vec3::new(0, 0, 0));
        let a = mass((1024, 1024 + 128 * 7), (0, 64 * 9), (1024, 1024 + 128 * 7));
        let d = door_at(&a, 1);
        let p1 = MassPlan::new(seed, parent, 2, a, Some(d));
        let p2 = MassPlan::new(seed, parent, 2, a, Some(d));
        assert_eq!(p1.spine, p2.spine);
        assert_eq!(p1.skeleton, p2.skeleton);
    }
}
