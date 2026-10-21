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
    /// Keyed by the L0 cell: the Web fields (`regimes.md` §7.5).
    /// Keyed by the L0 cell: well presence and draws (`gravity.md` §3.1).
    WELL = 0x0122;
    /// Keyed by a 256 m patch cell: Damaged well patches.
    WELL_PATCH = 0x0123;
    REGIME_WEB = 0x0110;
    /// Keyed by the L0 cell: the interior stack sequence (`regimes.md` §8.2).
    STACK = 0x0111;
    /// Keyed by the L0 cell above the face: the seam Between along Y.
    SEAM_BETWEEN_Y = 0x0112;
    /// Keyed by the L0 cell on the + side: the seam along X (Vertical stacks).
    SEAM_BETWEEN_X = 0x0113;
    /// Keyed by the L0 cell on the + side: the seam along Z (Vertical stacks).
    SEAM_BETWEEN_Z = 0x0114;
    /// Keyed by the L0 cell with the stratum index absorbed: the stratum character (`regimes.md` §10.2).
    STRATUM_CHAR = 0x0115;
    /// Keyed by an L1 cell or a grain cell, with the grain size absorbed: frame yaw (§7.4).
    ORI_YAW = 0x0116;
    /// Keyed by the L0 cell: lean heading and wall axis of the stack.
    STACK_YAW = 0x0118;

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
    /// Keyed by `LayerKey(L1, W = 4)`: tram lines per deck.
    TRAM_LINE = 0x0209;
    /// `(p, q, direction)`: a beam line (`levels.md` §4W.1).
    LINE_BEAM = 0x0210;
    /// Lattice corner (L1): fill noise.
    FILL_CORNER = 0x0214;
    /// Cell (L1): the knot draw.
    KNOT = 0x0212;
    /// Cell (L1): walks, loops and ties of the skeleton (`levels.md` §4W.5.2).
    ARTERY = 0x0215;
    /// Owner cell (L1): ports of its +X, +Y and +Z face (`levels.md` §4W.5.1).
    ARTERY_FACE_X = 0x0224;
    ARTERY_FACE_Y = 0x0225;
    ARTERY_FACE_Z = 0x0226;
    /// Cell (L0) with the Between index absorbed: identity of a run (`levels.md` §4W.6.1).
    BETWEEN_KEY = 0x0216;
    /// `(p, q, run key)`: the site draw of a Between lattice.
    BETWEEN_SITE = 0x0217;
    /// `(run key, t, 0)`: transit tubes.
    BETWEEN_LINE = 0x0218;
    /// `(block p, block q, run key)`: density of a block of underside windows.
    BETWEEN_LIGHT = 0x0219;
    /// `(p, q, run key)`: a net patch.
    BETWEEN_NET = 0x021A;
    /// `(p, q, run key)`: one underside window.
    BETWEEN_WINDOW = 0x021B;
    /// Cell (L0) with the crust index absorbed: identity of a crust (`levels.md` §4W.3.3).
    VOID_KEY = 0x0220;
    /// `(x, z, crust key)`: area void sites.
    VOID_SITE = 0x0221;
    /// `(axis, lane, crust key)`: valleys and rifts.
    VOID_LINE = 0x0222;
    /// `(p, q, direction)`: bore lines.
    VOID_BORE = 0x0223;
    /// `(which, 0, run key)`: the kind of a crust surface.
    SURFACE_KIND = 0x021C;
    /// `(p, q, run key + which)`: openings of a skin.
    SURFACE_OPENING = 0x021D;
    /// `(p, q, run key + which)`: roof lots, ceiling blocks and lamps.
    SURFACE_LOT = 0x021E;
    /// `(block, row, face key)`: window blocks of a section cliff.
    CLIFF_WINDOW = 0x021F;

    // L2 tissue of the Web: 0x031x (`levels.md` §5W.10)
    /// Keyed by the L2 cell: the four steps of the tissue.
    TISSUE = 0x0310;
    /// Keyed by the L2 cell: the frame of a structure in a grain.
    TISSUE_FRAME = 0x0311;
    /// Keyed by the L2 cell: set-piece draws.
    SET_PIECE = 0x0312;
    /// Keyed by a skin tile: window lights, shared with the far band.
    SKIN_WINDOWS = 0x0313;
    /// Keyed by the L2 cell: path types and their modifiers.
    PATHS = 0x0314;
    /// Keyed by the L2 cell: thresholds on entrances and bored passages.
    THRESHOLDS = 0x0315;
    /// Keyed by the L2 cell: the modifiers of each path segment (MOD-01..09).
    MODS = 0x0316;

    // L2 districts: 0x03xx
    MASS = 0x0301;
    /// Keyed by `ColumnSpanKey(L2)`.
    STAIR_TOWER = 0x0302;
    /// Keyed by `ColumnSpanKey(L2)`.
    LIFT_SHAFT = 0x0303;
    WALKWAY = 0x0304;
    /// Keyed by the L2 cell: gate placement and kind (`gravity.md` §4).
    GATE = 0x0317;
    RAMP = 0x0305;
    EDGE_GUARD = 0x0306;
    VISTA = 0x0307;
    /// Keyed by `CellKey(L2)`: masses attached to trunks.
    TRUNK_MASS = 0x0308;
    /// Keyed by `CellKey(L2)`: mass entrances.
    MASS_ENTRANCE = 0x0309;
    /// Keyed by `CellKey(L2)`: which side of a sanctuary room holds the door.
    SANCTUARY_ROOM = 0x030A;

    // L3 blocks: 0x04xx
    L3_RETRY = 0x0401;
    L3_SOLVE = 0x0402;
    /// Keyed by the parent L2 cell: mass plans.
    L3_MASS = 0x0403;
    L3_PIPE = 0x0404;
    /// Keyed by the L2 cell with the mass index added: the archetype of a mass (`levels.md` §6W.4).
    L3_ARCH = 0x0410;
    /// Keyed by the L3 block: corridor kinds along its skeleton edges (§6W.5).
    L3_CORR = 0x0411;
    /// Keyed by the L3 block: payoff rooms and interiors (§6W.5, §6W.6).
    L3_PAYOFF = 0x0412;

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
