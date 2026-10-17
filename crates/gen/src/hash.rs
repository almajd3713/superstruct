//! The single hash entry point. Spec: `docs/spec/hashing.md` §1–2.

use glam::I64Vec3;

use crate::cell::Level;

pub(crate) const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// `SplitMix64` output function (Stafford Mix13). A bijection on `u64`.
pub(crate) const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const fn absorb(h: u64, v: u64) -> u64 {
    mix((h ^ v).wrapping_add(GAMMA))
}

/// Hashes one decision: `salt` names it, `(seed, level, cell)` locates it.
#[allow(clippy::cast_sign_loss)] // i64 → u64 is a deliberate two's-complement bit cast.
pub const fn hash(seed: u64, level: Level, cell: I64Vec3, salt: u64) -> u64 {
    let mut h = mix(seed.wrapping_add(GAMMA));
    h = absorb(h, level as u64);
    h = absorb(h, cell.x as u64);
    h = absorb(h, cell.y as u64);
    h = absorb(h, cell.z as u64);
    absorb(h, salt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn pinned_vectors() {
        let p52 = 1i64 << 52;
        let p50 = 1i64 << 50;
        let cases: [(u64, Level, [i64; 3], u64, u64); 11] = [
            (0, Level::L0, [0, 0, 0], 0x0000, 0xCBD3_7AD2_9B93_B094),
            (0, Level::L0, [0, 0, 0], 0x0001, 0x1C72_8780_035A_0397),
            (1, Level::L0, [0, 0, 0], 0x0000, 0xFC1F_9D47_C1A5_266C),
            (0, Level::L1, [0, 0, 0], 0x0000, 0x75BD_1CB4_6E5E_F9B8),
            (42, Level::L2, [-1, 0, 6], 0x0301, 0x61F0_BC62_5448_5B97),
            (42, Level::L2, [0, -1, 6], 0x0301, 0x2C12_A530_C9FC_9D86),
            (42, Level::L2, [6, 0, -1], 0x0301, 0xA93A_3B02_AA52_D299),
            (42, Level::L4, [-3, 0, 386], 0x0501, 0x5EF3_A634_62E0_ABA8),
            (
                u64::MAX,
                Level::L0,
                [-p52, p52, -1],
                0x0101,
                0x682F_0B3A_FE1B_AE22,
            ),
            (
                0xDEAD_BEEF,
                Level::L1,
                [p50, -p50, 123_456_789],
                0x0201,
                0x3ED8_42A8_8A14_8678,
            ),
            (
                7,
                Level::L3,
                [i64::MIN, i64::MAX, 0],
                u64::MAX,
                0x2B75_5AF0_E8D0_291B,
            ),
        ];
        for (seed, level, cell, salt, expect) in cases {
            assert_eq!(
                hash(seed, level, I64Vec3::from_array(cell), salt),
                expect,
                "seed {seed:#x} level {level:?} cell {cell:?} salt {salt:#x}"
            );
        }
    }

    const SAMPLES: u32 = 2000;
    /// About 5σ for a flip-rate estimate from `SAMPLES` trials (σ = 0.5 / √2000 ≈ 0.011).
    const TOLERANCE: f64 = 0.06;

    /// Per-output-bit flip rates when `flip` changes one input bit of a random key.
    fn flip_rates(rng: &mut Rng, setup: impl Fn(&mut Key), flip: impl Fn(&mut Key)) -> [f64; 64] {
        let mut flips = [0u32; 64];
        for _ in 0..SAMPLES {
            let mut key = Key::random(rng);
            setup(&mut key);
            let before = key.hash();
            flip(&mut key);
            let diff = before ^ key.hash();
            for (bit, count) in flips.iter_mut().enumerate() {
                *count += u32::from((diff >> bit) & 1 == 1);
            }
        }
        flips.map(|c| f64::from(c) / f64::from(SAMPLES))
    }

    #[derive(Clone, Copy)]
    struct Key {
        seed: u64,
        level: Level,
        cell: [i64; 3],
        salt: u64,
    }

    impl Key {
        #[allow(clippy::cast_possible_wrap)]
        fn random(rng: &mut Rng) -> Self {
            Self {
                seed: rng.next_u64(),
                level: Level::L0,
                cell: [(); 3].map(|()| rng.next_u64() as i64),
                salt: rng.next_u64(),
            }
        }

        fn hash(&self) -> u64 {
            hash(
                self.seed,
                self.level,
                I64Vec3::from_array(self.cell),
                self.salt,
            )
        }
    }

    /// P0.4.5: flipping any single input bit flips every output bit with p ≈ 0.5.
    #[test]
    fn avalanche() {
        let mut rng = Rng::new(0xA5A5_A5A5);
        let mut rates = Vec::new();
        for bit in 0..64 {
            rates.push(flip_rates(&mut rng, |_| (), |k| k.seed ^= 1 << bit));
            rates.push(flip_rates(&mut rng, |_| (), |k| k.salt ^= 1 << bit));
            for axis in 0..3 {
                rates.push(flip_rates(&mut rng, |_| (), |k| k.cell[axis] ^= 1 << bit));
            }
        }
        // Level is an enum: compare valid levels that differ in exactly one bit.
        for (a, b) in [(0, 1), (2, 3), (0, 2), (1, 3), (0, 4)] {
            let (a, b) = (Level::ALL[a], Level::ALL[b]);
            rates.push(flip_rates(&mut rng, |k| k.level = a, |k| k.level = b));
        }
        let all: Vec<f64> = rates.iter().flatten().copied().collect();
        let worst = all.iter().map(|p| (p - 0.5).abs()).fold(0.0, f64::max);
        let mean = all.iter().sum::<f64>() / all.len() as f64;
        assert!(worst < TOLERANCE, "worst flip-rate deviation {worst:.4}");
        assert!((mean - 0.5).abs() < 0.002, "mean flip rate {mean:.4}");
    }
}
