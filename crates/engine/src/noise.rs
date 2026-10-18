//! Blue-noise tile for the dithered band cross-fade: void-and-cluster, deterministic.

pub const TILE: usize = 64;

/// Ranks every texel of a `TILE`x`TILE` torus so that equal-rank thresholds are spread as evenly
/// as possible; returned as bytes (rank scaled to 0..=255).
pub fn blue_noise_tile() -> &'static [u8] {
    static TILE_DATA: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    TILE_DATA.get_or_init(compute_tile)
}

fn compute_tile() -> Vec<u8> {
    const N: usize = TILE * TILE;
    const SIGMA: f32 = 1.9;
    // Gaussian energy of a point at toroidal offset (dx, dy).
    let energy = |dx: usize, dy: usize| {
        let wrap = |d: usize| d.min(TILE - d) as f32;
        let (x, y) = (wrap(dx), wrap(dy));
        (-(x * x + y * y) / (2.0 * SIGMA * SIGMA)).exp()
    };
    let kernel: Vec<f32> = (0..N).map(|i| energy(i % TILE, i / TILE)).collect();
    let splat = |field: &mut [f32], at: usize, sign: f32| {
        let (ax, ay) = (at % TILE, at / TILE);
        for y in 0..TILE {
            let ky = (y + TILE - ay) % TILE;
            for x in 0..TILE {
                let kx = (x + TILE - ax) % TILE;
                field[y * TILE + x] += sign * kernel[ky * TILE + kx];
            }
        }
    };

    // Seed pattern: a deterministic scatter of about a tenth of the texels.
    let mut state = 0x9e37_79b9_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let mut set = vec![false; N];
    for _ in 0..N / 10 {
        set[next() as usize % N] = true;
    }
    let mut field = vec![0.0_f32; N];
    for i in (0..N).filter(|&i| set[i]) {
        splat(&mut field, i, 1.0);
    }
    // Relax: move the tightest cluster point into the largest void until they coincide.
    for _ in 0..N {
        let tight = (0..N)
            .filter(|&i| set[i])
            .max_by(|&a, &b| field[a].total_cmp(&field[b]).then(b.cmp(&a)));
        let Some(tight) = tight else { break };
        set[tight] = false;
        splat(&mut field, tight, -1.0);
        let void = (0..N)
            .filter(|&i| !set[i])
            .min_by(|&a, &b| field[a].total_cmp(&field[b]).then(a.cmp(&b)))
            .unwrap_or(tight);
        set[void] = true;
        splat(&mut field, void, 1.0);
        if void == tight {
            break;
        }
    }

    let mut rank = vec![0_usize; N];
    // Phase 1: remove the tightest points one by one, ranking down from the seed count.
    let seeds = set.iter().filter(|&&s| s).count();
    let (mut s1, mut f1) = (set.clone(), field.clone());
    for r in (0..seeds).rev() {
        let tight = (0..N)
            .filter(|&i| s1[i])
            .max_by(|&a, &b| f1[a].total_cmp(&f1[b]).then(b.cmp(&a)))
            .unwrap_or(0);
        s1[tight] = false;
        splat(&mut f1, tight, -1.0);
        rank[tight] = r;
    }
    // Phase 2: fill the largest voids, ranking up.
    let (mut s2, mut f2) = (set, field);
    for r in seeds..N {
        let void = (0..N)
            .filter(|&i| !s2[i])
            .min_by(|&a, &b| f2[a].total_cmp(&f2[b]).then(a.cmp(&b)))
            .unwrap_or(0);
        s2[void] = true;
        splat(&mut f2, void, 1.0);
        rank[void] = r;
    }
    rank.iter()
        .map(|&r| ((r * 256 + 128) / N).min(255) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_threshold_appears_equally_often() {
        let tile = blue_noise_tile();
        assert_eq!(tile.len(), TILE * TILE);
        let mut hist = [0usize; 256];
        for &v in tile {
            hist[v as usize] += 1;
        }
        // 4096 ranks over 256 levels: 16 each.
        assert!(hist.iter().all(|&n| n == 16), "{hist:?}");
    }

    #[test]
    fn neighbours_differ_more_than_white_noise() {
        let tile = blue_noise_tile();
        let at = |x: usize, y: usize| f64::from(tile[(y % TILE) * TILE + x % TILE]) / 255.0;
        let mut sum = 0.0;
        for y in 0..TILE {
            for x in 0..TILE {
                sum += (at(x, y) - at(x + 1, y)).abs() + (at(x, y) - at(x, y + 1)).abs();
            }
        }
        let mean = sum / (2 * TILE * TILE) as f64;
        // Independent uniform values differ by 1/3 on average; blue noise pushes neighbours apart
        // (measured 0.39).
        assert!(mean > 0.37, "mean neighbour difference {mean}");
    }
}
