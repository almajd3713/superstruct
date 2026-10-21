//! The far-vs-near test (HOR-03, HOR-06): what the far view of an L1 cell shows against what the
//! nearer bands add. Orthographic first-hit depth maps of the primitives' boxes along the three
//! axes at 16 m pixels: the far band (L1 only, 5 km), the mid band (L1 plus the L2 exterior
//! masses, 3 km) and the near band (everything, 300 m). Run in release:
//! `cargo test -p gen --release --test far_near -- --ignored --nocapture`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)] // test code

use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::prim::{PrimFlags, Primitive};
use worldgen::seed::WorldSeed;

const PIXEL_M: i32 = 16;
const SIDE: usize = 4096 / PIXEL_M as usize;

/// The nearest front face of the primitives' boxes along each axis, per 16 m pixel, seen from
/// the low side: what a viewer outside the cell sees first.
struct Depths([Vec<i32>; 3]);

const NONE: i32 = i32::MAX;
/// Surfaces within this many metres of each other are the same surface to the eye.
const AGREE_M: i32 = 48;

impl Depths {
    fn new() -> Self {
        Self([
            vec![NONE; SIDE * SIDE],
            vec![NONE; SIDE * SIDE],
            vec![NONE; SIDE * SIDE],
        ])
    }

    /// Adds `p`, whose boxes are in quanta of 1/16 m from `origin` quanta.
    fn add(&mut self, p: &Primitive, origin: [i32; 3]) {
        let b = p.aabb();
        let lo = [
            b.min.x + origin[0],
            b.min.y + origin[1],
            b.min.z + origin[2],
        ];
        let hi = [
            b.max.x + origin[0],
            b.max.y + origin[1],
            b.max.z + origin[2],
        ];
        for (axis, map) in self.0.iter_mut().enumerate() {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let px = |q: i32| (q / 16 / PIXEL_M).clamp(0, SIDE as i32 - 1) as usize;
            let depth = lo[axis] / 16;
            for j in px(lo[v])..=px((hi[v] - 1).max(lo[v])) {
                for i in px(lo[u])..=px((hi[u] - 1).max(lo[u])) {
                    let d = &mut map[j * SIDE + i];
                    *d = (*d).min(depth);
                }
            }
        }
    }
}

/// The silhouette agreement of two views after downsampling by `DOWN` (64 m cells): occupied in
/// both, over occupied in either (HOR-03 compares silhouettes, not depths).
fn silhouette(far: &Depths, near: &Depths) -> f64 {
    const DOWN: usize = 4;
    let (mut both, mut either) = (0usize, 0usize);
    for (f, g) in far.0.iter().zip(&near.0) {
        for cy in 0..SIDE / DOWN {
            for cx in 0..SIDE / DOWN {
                let occ = |m: &Vec<i32>| {
                    (0..DOWN).any(|dy| {
                        (0..DOWN).any(|dx| m[(cy * DOWN + dy) * SIDE + cx * DOWN + dx] != NONE)
                    })
                };
                let (a, b) = (occ(f), occ(g));
                both += usize::from(a && b);
                either += usize::from(a || b);
            }
        }
    }
    both as f64 / either.max(1) as f64
}

/// How the near view's pixels relate to the far view's: `[same surface, near only (the far view
/// has nothing there), different surface, far only]`, as fractions of the pixels either sees.
fn compare(far: &Depths, near: &Depths) -> [f64; 4] {
    let mut n = [0usize; 4];
    for (f, g) in far.0.iter().zip(&near.0) {
        for (&a, &b) in f.iter().zip(g) {
            match (a == NONE, b == NONE) {
                (true, true) => {}
                (true, false) => n[1] += 1,
                (false, true) => n[3] += 1,
                (false, false) if (a - b).abs() <= AGREE_M => n[0] += 1,
                (false, false) => n[2] += 1,
            }
        }
    }
    let total = n.iter().sum::<usize>().max(1) as f64;
    n.map(|x| x as f64 / total)
}

#[test]
#[ignore = "generates 512 L2 cells per L1 cell; run in release"]
fn the_far_view_matches_the_near_view() {
    let seed = WorldSeed(0);
    let mut worst = 1.0f64;
    for l1 in [
        I64Vec3::new(0, 1, -1),
        I64Vec3::new(3, 0, 2),
        I64Vec3::new(-2, 1, 1),
        I64Vec3::new(5, 1, -3),
    ] {
        let cache = L1Cache::default();
        let parent = cache.get(seed, CellKey::new(Level::L1, l1));
        let (mut far, mut mid, mut near) = (Depths::new(), Depths::new(), Depths::new());
        for p in &parent.primitives {
            far.add(p, [0; 3]);
            if !p.flags.contains(PrimFlags::FAR_ONLY) {
                mid.add(p, [0; 3]);
            }
            if !p.flags.contains(PrimFlags::FAR_ONLY) {
                near.add(p, [0; 3]);
            }
        }
        for k in 0..512i64 {
            let c = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
            let cell = CellKey::new(Level::L2, l1 * 8 + c);
            let out = l2_cell_in(seed, &cache, cell);
            let origin = [c.x as i32 * 8192, c.y as i32 * 8192, c.z as i32 * 8192];
            for p in &out.primitives {
                near.add(p, origin);
                if p.flags.contains(PrimFlags::ENVELOPE) && !p.flags.contains(PrimFlags::INTERIOR) {
                    mid.add(p, origin);
                }
            }
        }
        let m = compare(&far, &mid);
        let n = compare(&far, &near);
        println!(
            "L1 {l1}: 3 km view vs 5 km: same {:.1} % near-only {:.1} % other {:.1} % far-only {:.1} %",
            m[0] * 100.0,
            m[1] * 100.0,
            m[2] * 100.0,
            m[3] * 100.0
        );
        println!(
            "L1 {l1}: 300 m view vs 5 km: same {:.1} % near-only {:.1} % other {:.1} % far-only {:.1} %",
            n[0] * 100.0,
            n[1] * 100.0,
            n[2] * 100.0,
            n[3] * 100.0
        );
        let sil = silhouette(&far, &near);
        println!(
            "L1 {l1}: silhouette agreement of the near view with the far view {:.1} %",
            sil * 100.0
        );
        worst = worst.min(sil);
    }
    println!(
        "worst silhouette agreement of the near view with the far view {:.1} %",
        worst * 100.0
    );
    assert!(
        worst >= 0.85,
        "the near view's silhouette leaves the far view's (HOR-03)"
    );
}
