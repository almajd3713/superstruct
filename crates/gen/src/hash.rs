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
}
