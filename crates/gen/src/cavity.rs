//! The giant-cavity rare event: a blob of whole L1 cells inside one L0 cell.
//! Spec: `docs/spec/levels.md` §3.

use glam::I64Vec3;

use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::regime::{RareEvent, rare_event};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Which of an L0 cell's 8 × 8 × 8 L1 cells are cavity cells. Bit `x + 8y + 64z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CavityMask(pub [u64; 8]);

impl CavityMask {
    const fn bit(i: [usize; 3]) -> (usize, u32) {
        let n = i[0] + 8 * i[1] + 64 * i[2];
        #[allow(clippy::cast_possible_truncation)] // n % 64 < 64
        (n / 64, (n % 64) as u32)
    }

    pub const fn get(&self, i: [usize; 3]) -> bool {
        let (w, b) = Self::bit(i);
        self.0[w] >> b & 1 == 1
    }

    fn set(&mut self, i: [usize; 3]) {
        let (w, b) = Self::bit(i);
        self.0[w] |= 1 << b;
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    pub fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }
}

/// The ellipsoid a cavity is voxelized from, in L1-cell units of its L0 cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    pub centre: [f32; 3],
    pub radii: [f32; 3],
}

/// The cavity ellipsoid of L0 cell `l0`, if its rare event is a giant cavity.
pub fn cavity_ellipsoid(seed: WorldSeed, l0: I64Vec3) -> Option<Ellipsoid> {
    if rare_event(seed, l0) != RareEvent::GiantCavity {
        return None;
    }
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::RARE_EVENT_PLACE));
    let quarter = |rng: &mut Rng, lo: f32, steps: i32| {
        #[allow(clippy::cast_precision_loss)]
        let k = rng.range_i32(0, steps + 1) as f32;
        lo + 0.25 * k
    };
    let centre = [(); 3].map(|()| quarter(&mut rng, 2.5, 12));
    let radii = [(); 3].map(|()| quarter(&mut rng, 1.0, 8));
    Some(Ellipsoid { centre, radii })
}

/// The cavity cells of L0 cell `l0` (empty without a cavity): noisy voxelization of the
/// ellipsoid inside the inner 6 × 6 × 6, then pocket filling.
pub fn cavity_mask(seed: WorldSeed, l0: I64Vec3) -> CavityMask {
    let mut mask = CavityMask::default();
    let Some(e) = cavity_ellipsoid(seed, l0) else {
        return mask;
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // centre in [2.5, 5.5]
    let centre_cell = e.centre.map(|c| c.floor() as usize);
    for z in 1..7 {
        for y in 1..7 {
            for x in 1..7 {
                let i = [x, y, z];
                #[allow(clippy::cast_possible_wrap)] // indices < 8
                let l1 = l0 * 8 + I64Vec3::new(x as i64, y as i64, z as i64);
                let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::CAVITY_NOISE));
                #[allow(clippy::cast_precision_loss)]
                let noise = rng.range_i32(-16, 17) as f32 / 64.0;
                // Summed x, y, z in order, so the float result is fixed.
                let sum: f32 = i
                    .iter()
                    .zip(e.centre.iter().zip(&e.radii))
                    .map(|(&ia, (&c, &r))| {
                        #[allow(clippy::cast_precision_loss)]
                        let t = (ia as f32 + 0.5 - c) / r;
                        t * t
                    })
                    .sum();
                if sum <= 1.0 + noise || i == centre_cell {
                    mask.set(i);
                }
            }
        }
    }
    fill_pockets(&mut mask);
    mask
}

/// Every non-cavity cell that a 6-neighbour flood fill from the margin layer cannot reach
/// becomes a cavity cell.
fn fill_pockets(mask: &mut CavityMask) {
    let mut reached = CavityMask::default();
    let mut stack: Vec<[usize; 3]> = Vec::new();
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                let i = [x, y, z];
                if i.iter().any(|&c| c == 0 || c == 7) && !mask.get(i) {
                    reached.set(i);
                    stack.push(i);
                }
            }
        }
    }
    while let Some(i) = stack.pop() {
        for a in 0..3 {
            for up in [false, true] {
                let mut n = i;
                if up && n[a] < 7 {
                    n[a] += 1;
                } else if !up && n[a] > 0 {
                    n[a] -= 1;
                } else {
                    continue;
                }
                if !mask.get(n) && !reached.get(n) {
                    reached.set(n);
                    stack.push(n);
                }
            }
        }
    }
    for w in 0..8 {
        mask.0[w] = !reached.0[w];
    }
}

/// The cavity mask of one L0 cell, computed once for all the `is_cavity` queries of an L1 cell
/// and its neighbours. Cavities keep to the inner 6 × 6 × 6 of their L0 cell, so a cell in
/// another L0 cell next to the view's is never a cavity cell.
#[derive(Debug, Clone, Copy)]
pub struct CavityView {
    l0: I64Vec3,
    mask: CavityMask,
}

impl CavityView {
    /// The view of the L0 cell containing L1 cell `cell`.
    pub fn new(seed: WorldSeed, cell: CellKey) -> Self {
        debug_assert_eq!(cell.level, Level::L1);
        let l0 = cell.coords.map(|v| floor_div_pow2(v, 3));
        Self {
            l0,
            mask: cavity_mask(seed, l0),
        }
    }

    /// Whether `cell`, in the view's L0 cell or adjacent to it, is a cavity cell.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // floor_mod < 8
    pub fn is_cavity(&self, cell: CellKey) -> bool {
        let c = cell.coords;
        if c.map(|v| floor_div_pow2(v, 3)) != self.l0 {
            return false;
        }
        self.mask
            .get([c.x, c.y, c.z].map(|v| floor_mod_pow2(v, 3) as usize))
    }
}

/// Whether L1 cell `cell` is a cavity cell.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // floor_mod < 8
pub fn is_cavity(seed: WorldSeed, cell: CellKey) -> bool {
    debug_assert_eq!(cell.level, Level::L1);
    let c = cell.coords;
    let l0 = I64Vec3::new(
        floor_div_pow2(c.x, 3),
        floor_div_pow2(c.y, 3),
        floor_div_pow2(c.z, 3),
    );
    if rare_event(seed, l0) != RareEvent::GiantCavity {
        return false;
    }
    let i = [c.x, c.y, c.z].map(|v| floor_mod_pow2(v, 3) as usize);
    cavity_mask(seed, l0).get(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L0 cells with a cavity, scanning a slab.
    fn cavity_cells(seed: WorldSeed, want: usize) -> Vec<I64Vec3> {
        (0..)
            .map(|i: i64| I64Vec3::new(i % 101 - 50, i / 101 % 7 - 3, i / 707 - 20))
            .filter(|&l0| rare_event(seed, l0) == RareEvent::GiantCavity)
            .take(want)
            .collect()
    }

    #[test]
    fn cavity_stays_in_the_inner_cells_and_contains_its_centre() {
        let seed = WorldSeed(0xCA7);
        for l0 in cavity_cells(seed, 300) {
            let e = cavity_ellipsoid(seed, l0).unwrap();
            let mask = cavity_mask(seed, l0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let centre = e.centre.map(|c| c.floor() as usize);
            assert!(mask.get(centre));
            for z in 0..8 {
                for y in 0..8 {
                    for x in 0..8 {
                        let margin = [x, y, z].iter().any(|&c| c == 0 || c == 7);
                        assert!(!(margin && mask.get([x, y, z])), "{l0} margin cavity");
                    }
                }
            }
        }
    }

    /// No cavity encloses a pocket: every non-cavity cell reaches the margin.
    #[test]
    fn no_enclosed_pockets() {
        let seed = WorldSeed(0xCA7);
        for l0 in cavity_cells(seed, 300) {
            let mask = cavity_mask(seed, l0);
            let mut refilled = mask;
            fill_pockets(&mut refilled);
            assert_eq!(refilled, mask, "{l0}");
        }
    }

    #[test]
    fn pocket_filling_closes_a_hollow_shell() {
        let mut mask = CavityMask::default();
        for z in 2..5 {
            for y in 2..5 {
                for x in 2..5 {
                    if [x, y, z] != [3, 3, 3] {
                        mask.set([x, y, z]);
                    }
                }
            }
        }
        assert!(!mask.get([3, 3, 3]));
        fill_pockets(&mut mask);
        assert!(mask.get([3, 3, 3]));
        assert_eq!(mask.count(), 27);
    }

    #[test]
    fn cavities_have_plausible_sizes() {
        let seed = WorldSeed(0xCA7);
        let sizes: Vec<u32> = cavity_cells(seed, 300)
            .iter()
            .map(|&l0| cavity_mask(seed, l0).count())
            .collect();
        assert!(sizes.iter().all(|&n| (1..=216).contains(&n)));
        let mean = f64::from(sizes.iter().sum::<u32>()) / sizes.len() as f64;
        // Mean ellipsoid volume (4/3)π·2³ ≈ 33.5 cells, clipped to the inner 6³.
        assert!((15.0..60.0).contains(&mean), "mean cavity {mean:.1} cells");
    }

    #[test]
    fn no_cavity_without_the_event() {
        let seed = WorldSeed(0xCA7);
        for x in -30..30 {
            let l0 = I64Vec3::new(x, 2, -x);
            if rare_event(seed, l0) != RareEvent::GiantCavity {
                assert!(cavity_mask(seed, l0).is_empty());
            }
        }
    }
}
