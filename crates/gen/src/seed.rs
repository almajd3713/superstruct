//! World seeds. Spec: `docs/spec/hashing.md` §5.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldSeed(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeedError {
    #[error("empty seed")]
    Empty,
}

impl WorldSeed {
    /// All-digit text that fits in `u64` is that number; any other text is hashed (FNV-1a 64).
    pub fn parse(text: &str) -> Result<Self, SeedError> {
        let t = text.trim_matches(|c: char| c.is_ascii_whitespace());
        if t.is_empty() {
            return Err(SeedError::Empty);
        }
        let numeric = t.len() <= 20 && t.bytes().all(|b| b.is_ascii_digit());
        if numeric && let Ok(n) = t.parse::<u64>() {
            return Ok(Self(n));
        }
        Ok(Self(fnv1a64(t.as_bytes())))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_vectors() {
        let cases: [(&str, u64); 12] = [
            ("42", 42),
            (" 42\n", 42),
            ("0042", 42),
            ("18446744073709551615", u64::MAX),
            ("18446744073709551616", 0xEDF2_AA6B_38FC_416D),
            ("superstruct", 0xE7E3_863E_EE77_F165),
            ("BLAME!", 0x3E16_C973_059C_069F),
            ("Ω", 0x0AD4_1307_B728_3D68),
            ("-5", 0x07D0_0F07_B497_D7F7),
            ("1e3", 0x44BB_4218_1789_16D8),
            ("+42", fnv1a64(b"+42")),
            ("\t7 ", 7),
        ];
        for (text, expect) in cases {
            assert_eq!(WorldSeed::parse(text), Ok(WorldSeed(expect)), "{text:?}");
        }
        assert_eq!(WorldSeed::parse(""), Err(SeedError::Empty));
        assert_eq!(WorldSeed::parse("  "), Err(SeedError::Empty));
    }

    #[test]
    fn same_text_same_seed() {
        for text in ["a", "superstruct", "0", "999", "  x y  "] {
            assert_eq!(WorldSeed::parse(text), WorldSeed::parse(text));
        }
        assert_ne!(WorldSeed::parse("a"), WorldSeed::parse("b"));
    }
}
