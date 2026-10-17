//! Per-cell sequential randomness: `SplitMix64`. Spec: `docs/spec/hashing.md` §3.
//!
//! Every operation is defined on top of `next_u64`, so the number of draws is part of the spec.

use crate::hash::{GAMMA, mix};

#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Usually seeded with a `hash(...)` output.
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix(self.state)
    }

    #[allow(clippy::cast_possible_truncation)] // keeps the high 32 bits on purpose
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)`, exact: 24 random bits scaled by 2⁻²⁴.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in `[0, n)` (Lemire's multiply-shift with rejection). Returns 0 when `n == 0`.
    #[allow(clippy::cast_possible_truncation)] // low and high halves of a u128 product
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            debug_assert!(false, "below(0)");
            return 0;
        }
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        if (m as u64) < n {
            let t = n.wrapping_neg() % n;
            while (m as u64) < t {
                m = u128::from(self.next_u64()) * u128::from(n);
            }
        }
        (m >> 64) as u64
    }

    /// Uniform in `[lo, hi)`. A caller bug if `lo >= hi`: returns `lo` without drawing.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // result < hi
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        if lo >= hi {
            debug_assert!(false, "range_i32({lo}, {hi})");
            return lo;
        }
        let span = (i64::from(hi) - i64::from(lo)).unsigned_abs();
        (i64::from(lo) + self.below(span) as i64) as i32
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }

    /// Index chosen with probability proportional to its weight. `None` if all weights are 0.
    pub fn pick_weighted(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
        if total == 0 {
            return None;
        }
        let r = self.below(total);
        let mut acc = 0u64;
        weights.iter().position(|&w| {
            acc += u64::from(w);
            r < acc
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0x61F0_BC62_5448_5B97;

    #[test]
    fn pinned_raw_sequences() {
        let cases: [(u64, [u64; 4]); 3] = [
            (
                0,
                [
                    0xE220_A839_7B1D_CDAF,
                    0x6E78_9E6A_A1B9_65F4,
                    0x06C4_5D18_8009_454F,
                    0xF88B_B8A8_724C_81EC,
                ],
            ),
            (
                1,
                [
                    0x910A_2DEC_8902_5CC1,
                    0xBEEB_8DA1_658E_EC67,
                    0xF893_A2EE_FB32_555E,
                    0x71C1_8690_EE42_C90B,
                ],
            ),
            (
                SEED,
                [
                    0xA99F_B48F_16F1_CB2D,
                    0x3348_90D4_01E4_4B6F,
                    0x66C0_5064_33B0_D742,
                    0x9851_6F89_099B_D6CB,
                ],
            ),
        ];
        for (seed, expect) in cases {
            let mut rng = Rng::new(seed);
            assert_eq!(expect.map(|_| rng.next_u64()), expect, "seed {seed:#x}");
        }
    }

    #[test]
    fn pinned_derived_operations() {
        let mut r = Rng::new(SEED);
        assert_eq!(
            [(); 4].map(|()| r.next_u32()),
            [0xA99F_B48F, 0x3348_90D4, 0x66C0_5064, 0x9851_6F89]
        );

        let mut r = Rng::new(SEED);
        let f = [(); 4].map(|()| r.next_f32());
        let expect =
            [11_116_468u32, 3_360_912, 6_733_904, 9_982_319].map(|n| n as f32 / 16_777_216.0);
        assert_eq!(f.map(f32::to_bits), expect.map(f32::to_bits));

        let mut r = Rng::new(SEED);
        assert_eq!(
            [(); 8].map(|()| r.range_i32(-10, 10)),
            [3, -6, -2, 1, 6, 1, 5, 8]
        );

        let mut r = Rng::new(SEED);
        assert_eq!(
            [(); 3].map(|()| r.range_i32(i32::MIN, i32::MAX)),
            [698_332_302, -1_287_089_965, -423_604_125]
        );

        let mut r = Rng::new(SEED);
        assert_eq!(
            [(); 8].map(|()| r.chance(0.25)),
            [false, true, false, false, false, false, false, false]
        );

        let mut r = Rng::new(SEED);
        assert_eq!(
            [(); 8].map(|()| r.pick_weighted(&[1, 0, 3, 6])),
            [3, 2, 3, 3, 3, 3, 3, 3].map(Some)
        );
    }

    #[test]
    fn edge_cases() {
        let mut r = Rng::new(9);
        assert_eq!(r.pick_weighted(&[]), None);
        assert_eq!(r.pick_weighted(&[0, 0]), None);
        assert_eq!(r.pick_weighted(&[0, 5, 0]), Some(1));
        assert_eq!(r.below(1), 0);
        assert_eq!(r.range_i32(4, 5), 4);
        assert!(!r.chance(0.0));
        assert!(r.chance(1.0));
    }

    /// χ² over `buckets` equally likely outcomes; returns the statistic.
    fn chi_square(counts: &[u64], samples: u64) -> f64 {
        let expected = samples as f64 / counts.len() as f64;
        counts
            .iter()
            .map(|&c| (c as f64 - expected).powi(2) / expected)
            .sum()
    }

    #[test]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // f32 in [0, 64)
    fn uniformity_chi_square() {
        const N: u64 = 1_000_000;
        // Bounds are about ±5σ around the mean (k − 1) of a χ² with k − 1 degrees of freedom.
        let check = |name: &str, counts: &[u64], lo: f64, hi: f64| {
            let x2 = chi_square(counts, N);
            assert!((lo..hi).contains(&x2), "{name}: χ² = {x2:.1}");
        };

        let mut r = Rng::new(SEED);
        let mut top_byte = [0u64; 256];
        for _ in 0..N {
            top_byte[(r.next_u32() >> 24) as usize] += 1;
        }
        check("next_u32 top byte", &top_byte, 255.0 - 113.0, 255.0 + 113.0);

        let mut r = Rng::new(SEED ^ 1);
        let mut range = [0u64; 20];
        for _ in 0..N {
            range[usize::try_from(r.range_i32(-10, 10) + 10).unwrap()] += 1;
        }
        check("range_i32(-10, 10)", &range, 19.0 - 30.8, 19.0 + 30.8);

        let mut r = Rng::new(SEED ^ 2);
        let mut f = [0u64; 64];
        for _ in 0..N {
            f[(r.next_f32() * 64.0) as usize] += 1;
        }
        check("next_f32", &f, 63.0 - 56.1, 63.0 + 56.1);
    }
}
