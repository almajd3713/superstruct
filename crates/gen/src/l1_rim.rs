//! Giant-cavity rim bites (`docs/spec/levels.md` §4.6).
//!
//! A rim cell is a non-cavity L1 cell sharing a face with a cavity cell. Each of its decks
//! loses the tiles of the cavity ellipsoid's horizontal slice, as long as the rest of the
//! deck stays one connected piece reaching every port-carrying face and the core.

use glam::I64Vec3;

use crate::cavity::{Ellipsoid, cavity_ellipsoid, is_cavity};
use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::hash::hash;
use crate::l1::{CELL, Deck, Rect, SEPARATION, Trunk};
use crate::prim::Q;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Tiles per cell edge. A tile is 64 m.
const N: usize = 64;
const TILE: Q = 64 * 16;
/// Scale `k` is `k32 / 32`; the draw covers 0.9..=1.3 and the loop stops below 0.5.
const K_MIN: i32 = 16;

/// Horizontal neighbours as `(dx, dz)`, in the order -x, +x, -z, +z.
const SIDES: [(i64, i64); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// Whether `cell` shares a face with a cavity cell.
pub fn is_rim(seed: WorldSeed, cell: CellKey) -> bool {
    !is_cavity(seed, cell) && neighbours(cell).into_iter().any(|n| is_cavity(seed, n))
}

fn neighbours(cell: CellKey) -> [CellKey; 6] {
    let c = cell.coords;
    [
        I64Vec3::new(-1, 0, 0),
        I64Vec3::new(1, 0, 0),
        I64Vec3::new(0, -1, 0),
        I64Vec3::new(0, 1, 0),
        I64Vec3::new(0, 0, -1),
        I64Vec3::new(0, 0, 1),
    ]
    .map(|d| CellKey::new(Level::L1, c + d))
}

/// The bitten rectangles of each deck of `cell` (same order as `decks`). Empty for a deck with
/// no bite, and for every deck of a cell that is not a rim cell.
#[allow(clippy::cast_precision_loss)] // tile and quanta values are far below 2^24
pub fn rim_bites(
    seed: WorldSeed,
    cell: CellKey,
    decks: &[Deck],
    holes: &[Rect],
    trunk: Option<&Trunk>,
) -> Vec<Vec<Rect>> {
    let none = || vec![Vec::new(); decks.len()];
    if decks.is_empty() || !is_rim(seed, cell) {
        return none();
    }
    let c = cell.coords;
    let l0 = I64Vec3::new(
        floor_div_pow2(c.x, 3),
        floor_div_pow2(c.y, 3),
        floor_div_pow2(c.z, 3),
    );
    let Some(e) = cavity_ellipsoid(seed, l0) else {
        return none();
    };
    let local = [c.x, c.y, c.z].map(|v| floor_mod_pow2(v, 3) as f32);
    let k0 = Rng::new(hash(seed.0, Level::L1, c, salt::CAVITY_BITE)).range_i32(29, 42);

    // Tiles that may never be bitten, and the faces whose edge tiles must stay connected.
    let mut keep = vec![false; N * N];
    let mut required: Vec<usize> = Vec::new();
    for (side, (dx, dz)) in SIDES.into_iter().enumerate() {
        let n = CellKey::new(Level::L1, c + I64Vec3::new(dx, 0, dz));
        if is_cavity(seed, n) {
            continue;
        }
        for t in 0..N {
            let i = match side {
                0 => t * N,
                1 => t * N + N - 1,
                2 => t,
                _ => (N - 1) * N + t,
            };
            keep[i] = true;
            required.push(i);
        }
    }
    let sep2 = i64::from(SEPARATION) * i64::from(SEPARATION);
    for tz in 0..N {
        for tx in 0..N {
            let r = tile_rect(tx, tz);
            let near_hole = holes.iter().any(|h| rect_dist2(&r, h) <= sep2);
            let need = i64::from(SEPARATION) + trunk.map_or(0, |t| i64::from(t.radius));
            let near_trunk =
                trunk.is_some_and(|t| r.dist2_point(t.centre_x, t.centre_z) <= need * need);
            if near_hole || near_trunk {
                keep[tz * N + tx] = true;
            }
        }
    }
    // The first hole is the core, whose stops every deck must reach.
    if let Some(core) = holes.first() {
        for tz in 0..N {
            for tx in 0..N {
                if rect_dist2(&tile_rect(tx, tz), core) <= sep2 {
                    required.push(tz * N + tx);
                }
            }
        }
    }

    decks
        .iter()
        .map(|deck| {
            let y = local[1] + deck.top as f32 / CELL as f32;
            let mut k32 = k0;
            while k32 >= K_MIN {
                let bitten = slice(&e, &local, y, k32, &keep);
                if bitten.iter().any(|&b| b) && connected(&bitten, &required) {
                    return runs_to_rects(&bitten);
                }
                if !bitten.iter().any(|&b| b) {
                    return Vec::new();
                }
                k32 -= 1;
            }
            Vec::new()
        })
        .collect()
}

fn tile_rect(tx: usize, tz: usize) -> Rect {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // < 64
    let (x0, z0) = (tx as Q * TILE, tz as Q * TILE);
    Rect {
        x0,
        z0,
        x1: x0 + TILE,
        z1: z0 + TILE,
    }
}

fn rect_dist2(a: &Rect, b: &Rect) -> i64 {
    let dx = i64::from((b.x0 - a.x1).max(a.x0 - b.x1).max(0));
    let dz = i64::from((b.z0 - a.z1).max(a.z0 - b.z1).max(0));
    dx * dx + dz * dz
}

/// Tiles whose centre lies inside the ellipsoid scaled by `k32 / 32`, except `keep` tiles.
/// The terms are summed x, y, z in order so the float result is fixed.
#[allow(clippy::cast_precision_loss)]
fn slice(e: &Ellipsoid, local: &[f32; 3], y: f32, k32: i32, keep: &[bool]) -> Vec<bool> {
    let k = k32 as f32 / 32.0;
    let ty = {
        let t = (y - e.centre[1]) / (k * e.radii[1]);
        t * t
    };
    (0..N * N)
        .map(|i| {
            let (tx, tz) = (i % N, i / N);
            let px = local[0] + (tx as f32 + 0.5) / N as f32;
            let pz = local[2] + (tz as f32 + 0.5) / N as f32;
            let tx2 = {
                let t = (px - e.centre[0]) / (k * e.radii[0]);
                t * t
            };
            let tz2 = {
                let t = (pz - e.centre[2]) / (k * e.radii[2]);
                t * t
            };
            !keep[i] && tx2 + ty + tz2 <= 1.0
        })
        .collect()
}

/// Whether the unbitten tiles form one 4-connected component containing every required tile.
fn connected(bitten: &[bool], required: &[usize]) -> bool {
    let Some(start) = (0..N * N).find(|&i| !bitten[i]) else {
        return false;
    };
    let mut seen = vec![false; N * N];
    let mut stack = vec![start];
    seen[start] = true;
    let mut count = 1;
    while let Some(i) = stack.pop() {
        let (x, z) = (i % N, i / N);
        let around = [
            (x > 0).then(|| i - 1),
            (x + 1 < N).then(|| i + 1),
            (z > 0).then(|| i - N),
            (z + 1 < N).then(|| i + N),
        ];
        for j in around.into_iter().flatten() {
            if !bitten[j] && !seen[j] {
                seen[j] = true;
                count += 1;
                stack.push(j);
            }
        }
    }
    count == bitten.iter().filter(|&&b| !b).count() && required.iter().all(|&i| seen[i])
}

/// Bitten tiles as disjoint rectangles: runs along x per row, extended down while identical.
fn runs_to_rects(bitten: &[bool]) -> Vec<Rect> {
    let mut done: Vec<Rect> = Vec::new();
    let mut open: Vec<Rect> = Vec::new();
    for tz in 0..N {
        let mut row: Vec<(usize, usize)> = Vec::new();
        for tx in 0..N {
            if bitten[tz * N + tx] {
                match row.last_mut() {
                    Some(run) if run.1 == tx => run.1 = tx + 1,
                    _ => row.push((tx, tx + 1)),
                }
            }
        }
        let z = tile_rect(0, tz);
        let mut next = Vec::with_capacity(row.len());
        for (a, b) in row {
            let (x0, x1) = (tile_rect(a, 0).x0, tile_rect(b - 1, 0).x1);
            if let Some(i) = open.iter().position(|r| r.x0 == x0 && r.x1 == x1) {
                let mut r = open.remove(i);
                r.z1 = z.z1;
                next.push(r);
            } else {
                next.push(Rect {
                    x0,
                    z0: z.z0,
                    x1,
                    z1: z.z1,
                });
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    done.sort_unstable_by_key(|r| (r.z0, r.x0));
    done
}
