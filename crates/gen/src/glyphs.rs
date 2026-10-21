//! The City's script and place names (MUL-01, MUL-02, TOOL-05). Spec: `docs/spec/levels.md`
//! §5W.38.
//!
//! The world has an alphabet of 32 glyphs, each a 7 × 7 bitmap drawn from strokes by the seed,
//! so no two worlds share a script. A **name** is three glyphs (15 bits). Every L1 cell has one
//! (its knot's), and **no two cells within 8 cells (32 km) of each other share a name**: the name
//! is a bijection of `x + 17 y + 289 z` modulo 2^15, and for differences of at most 8 on every
//! axis that sum is not 0 modulo 2^15, so the names differ.

// Glyph sizes are 7 and indices below 32: the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::hash;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

pub const ALPHABET: usize = 32;
pub const SIDE: usize = 7;
const NAMES: i64 = 1 << 15;

/// A glyph: seven rows of seven bits, bit 0 the left column.
pub type Glyph = [u8; SIDE];

/// How far apart, in L1 cells, two cells must be to be allowed the same name.
pub const UNIQUE_WITHIN_CELLS: i64 = 8;

fn plot(g: &mut Glyph, x: i32, y: i32) {
    if (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y) {
        g[y as usize] |= 1 << x;
    }
}

/// A line from `(x0, y0)` to `(x1, y1)` in integer steps (Bresenham).
fn stroke(g: &mut Glyph, mut x0: i32, mut y0: i32, x1: i32, y1: i32) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        plot(g, x0, y0);
        if (x0, y0) == (x1, y1) {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// The alphabet of the world: 32 distinct glyphs, each of three to six strokes. A function of
/// the seed.
pub fn alphabet(seed: WorldSeed) -> [Glyph; ALPHABET] {
    let mut out = [[0u8; SIDE]; ALPHABET];
    for i in 0..ALPHABET {
        let mut attempt = 0i64;
        loop {
            let mut rng = Rng::new(hash(
                seed.0,
                Level::L0,
                I64Vec3::new(i as i64, attempt, 0),
                salt::GLYPH,
            ));
            let mut g = [0u8; SIDE];
            for _ in 0..3 + rng.below(4) {
                let p = |r: &mut Rng| r.below(SIDE as u64) as i32;
                let (x0, y0, x1, y1) = (p(&mut rng), p(&mut rng), p(&mut rng), p(&mut rng));
                stroke(&mut g, x0, y0, x1, y1);
            }
            let lit: u32 = g.iter().map(|r| r.count_ones()).sum();
            if (8..=30).contains(&lit) && !out[..i].contains(&g) {
                out[i] = g;
                break;
            }
            attempt += 1;
        }
    }
    out
}

/// The name of L1 cell `l1`: three glyph indices.
pub fn name_of(seed: WorldSeed, l1: I64Vec3) -> [u8; 3] {
    let h = hash(seed.0, Level::L0, I64Vec3::ZERO, salt::NAME_PERM);
    // An odd multiplier and any offset make `i -> k i + s` a bijection of Z/2^15.
    let (k, s) = ((h & 0x7FFF) | 1, (h >> 20) & 0x7FFF);
    let lattice = (l1.x + 17 * l1.y + 289 * l1.z).rem_euclid(NAMES);
    let n = ((lattice * k as i64 + s as i64).rem_euclid(NAMES)) as u32;
    [(n >> 10) as u8 & 31, (n >> 5) as u8 & 31, n as u8 & 31]
}

/// A name as one number, for comparing.
pub fn name_index(name: [u8; 3]) -> u32 {
    (u32::from(name[0]) << 10) | (u32::from(name[1]) << 5) | u32::from(name[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(11);

    #[test]
    fn the_alphabet_is_thirty_two_distinct_glyphs_that_depend_on_the_seed() {
        let a = alphabet(SEED);
        for i in 0..ALPHABET {
            for j in 0..i {
                assert_ne!(a[i], a[j], "glyphs {i} and {j}");
            }
            let lit: u32 = a[i].iter().map(|r| r.count_ones()).sum();
            assert!((8..=30).contains(&lit));
        }
        assert_ne!(a, alphabet(WorldSeed(12)));
        assert_eq!(a, alphabet(SEED));
    }

    /// MUL-02: over a 100 km region no two cells within 32 km share a name.
    #[test]
    fn no_two_cells_within_thirty_two_kilometres_share_a_name() {
        let n = 25i64;
        let mut by_name: std::collections::HashMap<u32, Vec<I64Vec3>> =
            std::collections::HashMap::new();
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    let c = I64Vec3::new(x, y, z);
                    by_name
                        .entry(name_index(name_of(SEED, c)))
                        .or_default()
                        .push(c);
                }
            }
        }
        let mut shared = 0;
        for cells in by_name.values() {
            for (i, a) in cells.iter().enumerate() {
                for b in &cells[..i] {
                    shared += 1;
                    assert!(
                        (*a - *b).abs().max_element() > UNIQUE_WITHIN_CELLS,
                        "{a} and {b} share a name"
                    );
                }
            }
        }
        // Names do repeat, but only far apart (15 625 cells, 32 768 names).
        assert!(shared < 15_625);
    }

    #[test]
    fn names_are_stable_and_use_the_whole_alphabet() {
        let mut seen = [false; ALPHABET];
        for x in -20..20 {
            for z in -20..20 {
                let n = name_of(SEED, I64Vec3::new(x, 1, z));
                assert_eq!(n, name_of(SEED, I64Vec3::new(x, 1, z)));
                for g in n {
                    seen[usize::from(g)] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s));
    }
}
