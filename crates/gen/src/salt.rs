//! The salt registry. Rules: `docs/spec/hashing.md` §4; assignments: `levels.md` §8,
//! `regimes.md` §6, `ports.md` §7.
//!
//! One salt, one purpose. Values are append-only: a salt that is no longer used moves to
//! `RETIRED` and its value is never reassigned.

macro_rules! salts {
    ($($(#[$doc:meta])* $name:ident = $value:literal;)*) => {
        $($(#[$doc])* pub const $name: u64 = $value;)*

        /// Every active salt, for the uniqueness test and tooling.
        pub const ALL: &[(&str, u64)] = &[$((stringify!($name), $value)),*];
    };
}

salts! {
    // L0 regime: 0x01xx
    REGIME = 0x0101;
    RARE_EVENT = 0x0102;
    RARE_EVENT_PLACE = 0x0103;
    /// Keyed by `ColumnKey(L0)`.
    RARE_STAIR = 0x0104;
    /// Per L1 cell, used by the L0 cavity voxelization.
    CAVITY_NOISE = 0x0105;

    // L1 megaforms: 0x02xx
    /// Keyed by `ColumnSpanKey(L1)`.
    TRUNK = 0x0201;
    /// Keyed by `LayerKey(L1, W = 4)`.
    DECK = 0x0202;
    /// Keyed by `ColumnSpanKey(L1)`.
    SHAFT = 0x0203;
    CHASM = 0x0204;
    BRIDGE = 0x0205;
    SANCTUARY = 0x0206;
    /// Keyed by `ColumnKey(L1)`.
    CORE = 0x0207;
    CAVITY_BITE = 0x0208;

    // L2 districts: 0x03xx
    MASS = 0x0301;
    /// Keyed by `ColumnSpanKey(L2)`.
    STAIR_TOWER = 0x0302;
    /// Keyed by `ColumnSpanKey(L2)`.
    LIFT_SHAFT = 0x0303;
    WALKWAY = 0x0304;
    RAMP = 0x0305;
    EDGE_GUARD = 0x0306;
    VISTA = 0x0307;

    // L3 blocks: 0x04xx
    L3_RETRY = 0x0401;

    // L4 modules: 0x05xx
    MODULE = 0x0501;

    // L5 dressing: 0x06xx
    DRESSING = 0x0601;

    // Cross-level: 0x0Fxx. Face salts are keyed by `FaceKey` at every level.
    PORT_FACE_X = 0x0F01;
    PORT_FACE_Y = 0x0F02;
    PORT_FACE_Z = 0x0F03;
}

/// Salts that were used by an earlier `GEN_VERSION`. Their values must never be reused.
pub const RETIRED: &[(&str, u64)] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn all_salts_are_unique() {
        let mut seen: HashMap<u64, &str> = HashMap::new();
        for &(name, value) in ALL.iter().chain(RETIRED) {
            if let Some(other) = seen.insert(value, name) {
                panic!("salt {value:#06x} used by both {other} and {name}");
            }
        }
    }

    #[test]
    fn salts_sit_in_known_blocks() {
        for &(name, value) in ALL {
            let block = value >> 8;
            assert!(
                (0x01..=0x06).contains(&block) || block == 0x0F,
                "{name} = {value:#06x} is outside the registry blocks"
            );
        }
    }
}
