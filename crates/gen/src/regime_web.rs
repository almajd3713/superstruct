//! The Web fields of a regime: fill, weave, grain, orientation policy and the style palettes
//! of forms, supports, paths, quarters and mutations. Spec: `docs/spec/regimes.md` §7, §9.
//!
//! Stack geometry, crust depths and Between kinds (§8) join in PW.2.2; their draws are
//! reserved here so the sequence of the later fields does not move.

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::{absorb, hash};
use crate::orient::Frame;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

/// Smallest and largest `grain_log2` (§7.3): 64 m to 4096 m.
pub const GRAIN_LOG2: (u8, u8) = (6, 12);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WebFields {
    /// Mean built fraction inside crusts, 255 = 100 % (REG-01).
    pub fill_crust: u8,
    /// Mean built fraction inside Betweens (REG-01).
    pub fill_between: u8,
    /// 0 = axes only … 255 = mostly oblique (REG-01).
    pub weave: u8,
    /// Grain size is `2^grain_log2` metres, 6..=12 (REG-02).
    pub grain_log2: u8,
    pub orientation: OrientationPolicy,
    /// Weights 0..=8 over form families (REG-03). Order: [`FORM_FAMILIES`].
    pub forms: [u8; 7],
    /// Weights over SUP-01..11.
    pub supports: [u8; 11],
    /// Weights over path families. Order: [`PATH_FAMILIES`].
    pub paths: [u8; 6],
    /// Weights over QTR-01..16.
    pub quarters: [u8; 16],
    /// Weights over mutation groups. Order: [`MUTATION_GROUPS`].
    pub mutations: [u8; 6],
}

impl WebFields {
    /// Placeholder for tests that compare other fields.
    pub const NONE: Self = Self {
        fill_crust: 0,
        fill_between: 0,
        weave: 0,
        grain_log2: GRAIN_LOG2.0,
        orientation: OrientationPolicy::Axis,
        forms: [0; 7],
        supports: [0; 11],
        paths: [0; 6],
        quarters: [0; 16],
        mutations: [0; 6],
    };
}

pub const FORM_FAMILIES: [&str; 7] = [
    "TOWER",
    "STEP",
    "FRAMEWORK",
    "PLANT",
    "ROUND",
    "RUIN",
    "ANGLED",
];
pub const PATH_FAMILIES: [&str; 6] = ["WALK", "ENCLOSED", "STREET", "STAIR", "LIFTSLOPE", "LADDER"];
pub const MUTATION_GROUPS: [&str; 6] = ["POSE", "DAMAGE", "COPY", "HOLLOW", "LIGHT", "SCALE"];

/// Index of the families that can never be banned (§9: PH-01 and PV-01 are universal).
pub const PATH_WALK: usize = 0;
pub const PATH_STAIR: usize = 3;

/// How structures are turned (REG-04, ORI-12..16). Spec §7.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OrientationPolicy {
    /// ORI-12: the calm default.
    Axis,
    /// ORI-13: one yaw per L1 cell.
    YawPerL1,
    /// ORI-14: one yaw per grain cell.
    GrainYaw,
    /// ORI-15: yaw and pitch. `max_tilt` is an index into the tilt table, 1..=6.
    TiltedGrain { max_tilt: u8 },
    /// ORI-16: around a gravity well. Until wells exist (PW.6) it falls back to `Axis`.
    Radial,
}

impl OrientationPolicy {
    /// The registry ID of the policy, for the census.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Axis => "ORI-12",
            Self::YawPerL1 => "ORI-13",
            Self::GrainYaw => "ORI-14",
            Self::TiltedGrain { .. } => "ORI-15",
            Self::Radial => "ORI-16",
        }
    }
}

/// One row of the per-style Web table (§7.6, §9.1). Ranges are inclusive.
#[derive(Debug)]
pub struct WebDef {
    pub fill_crust: (u8, u8),
    pub fill_between: (u8, u8),
    pub weave: (u8, u8),
    pub grain_log2: (u8, u8),
    /// `Axis`, `YawPerL1`, `GrainYaw`, `TiltedGrain`, `Radial`.
    pub orientation: [u32; 5],
    pub tilt_cap: u8,
    pub forms: [u8; 7],
    pub supports: [u8; 11],
    pub paths: [u8; 6],
    pub quarters: [u8; 16],
    pub mutations: [u8; 6],
}

/// Indexed by `StyleId`.
pub const WEB_STYLES: [WebDef; 5] = [
    WebDef {
        fill_crust: (190, 225),
        fill_between: (5, 15),
        weave: (0, 40),
        grain_log2: (11, 12),
        orientation: [70, 20, 5, 5, 0],
        tilt_cap: 2,
        forms: [8, 6, 3, 2, 3, 2, 2],
        supports: [8, 4, 3, 3, 3, 5, 2, 2, 4, 2, 1],
        paths: [4, 5, 8, 8, 4, 2],
        quarters: [4, 3, 5, 6, 5, 1, 5, 8, 7, 3, 3, 3, 4, 5, 4, 3],
        mutations: [1, 2, 4, 3, 3, 2],
    },
    WebDef {
        fill_crust: (205, 230),
        fill_between: (10, 26),
        weave: (20, 110),
        grain_log2: (8, 11),
        orientation: [25, 35, 25, 15, 0],
        tilt_cap: 3,
        forms: [3, 2, 7, 8, 4, 3, 2],
        supports: [5, 3, 4, 4, 6, 3, 4, 6, 3, 3, 1],
        paths: [6, 6, 4, 4, 5, 8],
        quarters: [5, 8, 3, 4, 2, 2, 2, 2, 4, 6, 8, 1, 3, 1, 2, 8],
        mutations: [2, 4, 3, 4, 4, 2],
    },
    WebDef {
        fill_crust: (178, 210),
        fill_between: (5, 12),
        weave: (30, 120),
        grain_log2: (10, 12),
        orientation: [30, 30, 15, 25, 0],
        tilt_cap: 4,
        forms: [5, 3, 4, 1, 6, 3, 4],
        supports: [4, 5, 2, 3, 8, 2, 3, 2, 2, 2, 1],
        paths: [5, 3, 3, 6, 6, 3],
        quarters: [2, 2, 3, 3, 4, 4, 6, 2, 5, 3, 4, 7, 2, 8, 2, 2],
        mutations: [3, 3, 2, 3, 3, 4],
    },
    WebDef {
        fill_crust: (178, 200),
        fill_between: (5, 10),
        weave: (40, 180),
        grain_log2: (9, 12),
        orientation: [20, 20, 15, 40, 5],
        tilt_cap: 6,
        forms: [4, 4, 2, 1, 5, 4, 6],
        supports: [3, 6, 6, 5, 2, 2, 6, 1, 3, 3, 1],
        paths: [8, 3, 2, 5, 6, 2],
        quarters: [1, 1, 4, 5, 6, 3, 8, 3, 4, 4, 4, 8, 3, 6, 3, 1],
        mutations: [5, 3, 2, 3, 2, 5],
    },
    WebDef {
        fill_crust: (190, 228),
        fill_between: (8, 26),
        weave: (60, 220),
        grain_log2: (6, 10),
        orientation: [10, 20, 40, 30, 0],
        tilt_cap: 5,
        forms: [3, 2, 3, 3, 3, 8, 5],
        supports: [4, 4, 3, 3, 4, 6, 5, 8, 5, 4, 1],
        paths: [4, 6, 5, 4, 4, 6],
        quarters: [8, 2, 1, 2, 3, 8, 4, 2, 2, 2, 6, 3, 6, 4, 6, 2],
        mutations: [3, 8, 3, 3, 2, 3],
    },
];

/// Draws made for the stack fields of §8 (stack geometry, its tilt, two crust bounds, five
/// Between weights). PW.2.2 replaces the burn with the real draws; the count must stay.
const RESERVED_STACK_DRAWS: usize = 9;

/// The Web fields of L0 cell `cell` (§7.5). `style` indexes `WEB_STYLES`.
pub fn draw_web(seed: WorldSeed, cell: I64Vec3, style: usize) -> WebFields {
    let def = &WEB_STYLES[style];
    let mut rng = Rng::new(hash(seed.0, Level::L0, cell, salt::REGIME_WEB));
    let fill_crust = range_u8(&mut rng, def.fill_crust);
    let fill_between = range_u8(&mut rng, def.fill_between);
    let weave = range_u8(&mut rng, def.weave);
    let grain_log2 = range_u8(&mut rng, def.grain_log2);
    // The weights are not all zero, so a policy is always picked.
    let policy = rng.pick_weighted(&def.orientation).unwrap_or(0);
    let orientation = match policy {
        1 => OrientationPolicy::YawPerL1,
        2 => OrientationPolicy::GrainYaw,
        3 => OrientationPolicy::TiltedGrain {
            max_tilt: range_u8(&mut rng, (1, def.tilt_cap)),
        },
        4 => OrientationPolicy::Radial,
        _ => OrientationPolicy::Axis,
    };
    for _ in 0..RESERVED_STACK_DRAWS {
        rng.next_u64();
    }
    let mut forms = weights(&mut rng, def.forms);
    let mut supports = weights(&mut rng, def.supports);
    let mut paths = weights(&mut rng, def.paths);
    let quarters = weights(&mut rng, def.quarters);
    let mutations = weights(&mut rng, def.mutations);
    // Routing needs a fallback (§9): the universal path families and at least one form and
    // support survive any draw.
    paths[PATH_WALK] = paths[PATH_WALK].max(1);
    paths[PATH_STAIR] = paths[PATH_STAIR].max(1);
    if forms.iter().all(|&w| w == 0) {
        forms[0] = 1;
    }
    if supports.iter().all(|&w| w == 0) {
        supports[0] = 1;
    }
    WebFields {
        fill_crust,
        fill_between,
        weave,
        grain_log2,
        orientation,
        forms,
        supports,
        paths,
        quarters,
        mutations,
    }
}

/// One draw per entry: `max(0, base + range(−2, 3))` capped at 8. A zero base is a ban; its
/// draw is still made so the count never depends on the table.
fn weights<const N: usize>(rng: &mut Rng, base: [u8; N]) -> [u8; N] {
    base.map(|b| {
        let d = rng.range_i32(-2, 3);
        if b == 0 {
            0
        } else {
            u8::try_from((i32::from(b) + d).clamp(0, 8)).unwrap_or(0)
        }
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // result in [lo, hi]
fn range_u8(rng: &mut Rng, (lo, hi): (u8, u8)) -> u8 {
    rng.range_i32(i32::from(lo), i32::from(hi) + 1) as u8
}

/// The 64-heading yaw of a hash, quantised to multiples of 4 for calm regimes (§7.4).
#[allow(clippy::cast_possible_truncation)] // below(64) < 64
fn yaw_of(rng: &mut Rng, weave: u8) -> u8 {
    let yaw = rng.below(64) as u8;
    if weave < 64 { yaw & !3 } else { yaw }
}

impl WebFields {
    /// The frame of an L1 cell, for the L1 frame machinery (`levels.md` §4W). Grain policies
    /// read the grain cell holding the L1 cell's centre.
    pub fn frame_l1(&self, seed: WorldSeed, l1: I64Vec3) -> Frame {
        match self.orientation {
            OrientationPolicy::YawPerL1 => self.yaw_l1(seed, l1),
            _ => self.frame_at(seed, l1 * 4096 + I64Vec3::splat(2048), l1),
        }
    }

    /// The frame of an L2 cell, which L3 blocks inherit. Grain policies use the cell's centre.
    pub fn frame_l2(&self, seed: WorldSeed, l2: I64Vec3) -> Frame {
        let l1 = l2.div_euclid(I64Vec3::splat(8));
        self.frame_at(seed, l2 * 512 + I64Vec3::splat(256), l1)
    }

    /// The frame of a structure anchored at integer world position `anchor_m` (metres).
    /// `l1` is the L1 cell for `YawPerL1`.
    pub fn frame_at(&self, seed: WorldSeed, anchor_m: I64Vec3, l1: I64Vec3) -> Frame {
        match self.orientation {
            OrientationPolicy::Axis | OrientationPolicy::Radial => Frame::Id,
            OrientationPolicy::YawPerL1 => self.yaw_l1(seed, l1),
            OrientationPolicy::GrainYaw => {
                let mut rng = self.grain_rng(seed, anchor_m);
                Frame::table(yaw_of(&mut rng, self.weave), 0)
            }
            OrientationPolicy::TiltedGrain { max_tilt } => {
                let mut rng = self.grain_rng(seed, anchor_m);
                let yaw = yaw_of(&mut rng, self.weave);
                #[allow(clippy::cast_possible_truncation)] // below(max_tilt) < 7
                let tilt = 1 + rng.below(u64::from(max_tilt)) as u8;
                Frame::table(yaw, tilt)
            }
        }
    }

    fn yaw_l1(&self, seed: WorldSeed, l1: I64Vec3) -> Frame {
        let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::ORI_YAW));
        Frame::table(yaw_of(&mut rng, self.weave), 0)
    }

    /// Seeded from the integer grain cell; the grain size is folded in so that two grain sizes
    /// never share a stream.
    fn grain_rng(&self, seed: WorldSeed, anchor_m: I64Vec3) -> Rng {
        let size = 1i64 << self.grain_log2;
        let cell = anchor_m.div_euclid(I64Vec3::splat(size));
        let h = hash(seed.0, Level::L0, cell, salt::ORI_YAW);
        Rng::new(absorb(h, u64::from(self.grain_log2)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regime::{STYLES, regime};

    fn fields(seed: u64, cell: [i64; 3]) -> WebFields {
        regime(WorldSeed(seed), I64Vec3::from_array(cell)).web
    }

    fn within<T: PartialOrd + Copy>(v: T, (lo, hi): (T, T)) -> bool {
        (lo..=hi).contains(&v)
    }

    #[test]
    fn tables_are_well_formed() {
        assert_eq!(WEB_STYLES.len(), STYLES.len());
        for d in &WEB_STYLES {
            assert!(d.fill_crust.0 <= d.fill_crust.1 && d.fill_between.0 <= d.fill_between.1);
            assert!(d.weave.0 <= d.weave.1);
            assert!(GRAIN_LOG2.0 <= d.grain_log2.0 && d.grain_log2.0 <= d.grain_log2.1);
            assert!(d.grain_log2.1 <= GRAIN_LOG2.1);
            assert!(d.orientation.iter().sum::<u32>() > 0);
            assert!((1..crate::orient::TILTS).contains(&d.tilt_cap));
            assert!(d.paths[PATH_WALK] > 0 && d.paths[PATH_STAIR] > 0);
            assert!(d.forms.iter().chain(&d.supports).all(|&w| w > 0));
        }
    }

    /// Every field lies in its style's range, for many cells.
    #[test]
    fn fields_stay_in_range() {
        let seed = WorldSeed(0x5EED);
        for x in -40..40 {
            for z in -40..40 {
                let r = regime(seed, I64Vec3::new(x, 1, z));
                let w = r.web;
                let d = &WEB_STYLES[usize::from(r.style.0)];
                assert!(within(w.fill_crust, d.fill_crust), "{w:?}");
                assert!(within(w.fill_between, d.fill_between), "{w:?}");
                assert!(within(w.weave, d.weave), "{w:?}");
                assert!(within(w.grain_log2, d.grain_log2), "{w:?}");
                if let OrientationPolicy::TiltedGrain { max_tilt } = w.orientation {
                    assert!(within(max_tilt, (1, d.tilt_cap)), "{w:?}");
                }
                let idx = match w.orientation {
                    OrientationPolicy::Axis => 0,
                    OrientationPolicy::YawPerL1 => 1,
                    OrientationPolicy::GrainYaw => 2,
                    OrientationPolicy::TiltedGrain { .. } => 3,
                    OrientationPolicy::Radial => 4,
                };
                assert!(d.orientation[idx] > 0, "{w:?}");
                assert!(w.paths[PATH_WALK] > 0 && w.paths[PATH_STAIR] > 0);
                assert!(w.forms.iter().any(|&f| f > 0) && w.supports.iter().any(|&f| f > 0));
                for (got, base) in w.quarters.iter().zip(d.quarters) {
                    assert!(*got <= 8 && i32::from(*got) >= i32::from(base) - 2);
                }
            }
        }
    }

    /// Every policy occurs over a large sample, and a policy of weight 0 never does.
    #[test]
    fn every_policy_occurs() {
        let seed = WorldSeed(11);
        let mut seen = [0u32; 5];
        for i in 0..20_000i64 {
            let r = regime(seed, I64Vec3::new(i % 200, i / 200, 5));
            let idx = match r.web.orientation {
                OrientationPolicy::Axis => 0,
                OrientationPolicy::YawPerL1 => 1,
                OrientationPolicy::GrainYaw => 2,
                OrientationPolicy::TiltedGrain { .. } => 3,
                OrientationPolicy::Radial => 4,
            };
            seen[idx] += 1;
            let d = &WEB_STYLES[usize::from(r.style.0)];
            assert!(d.orientation[idx] > 0);
        }
        assert!(seen.iter().all(|&c| c > 0), "{seen:?}");
    }

    /// The Web draws do not move the old fields (`regimes.md` §1).
    #[test]
    fn old_fields_keep_their_values() {
        let r = regime(WorldSeed(1), I64Vec3::ZERO);
        assert_eq!(r.style.0, 1);
        assert_eq!(r.density, 189);
        assert_eq!(r.verticality, 155);
    }

    /// Pinned outputs. A change here is a generator output change: bump `GEN_VERSION`.
    #[test]
    fn golden() {
        let a = fields(1, [0, 0, 0]);
        assert_eq!(
            (a.fill_crust, a.fill_between, a.weave, a.grain_log2),
            (229, 11, 70, 8)
        );
        assert_eq!(a.orientation, OrientationPolicy::YawPerL1);
        assert_eq!(a.forms, [2, 1, 8, 7, 4, 5, 4]);
        assert_eq!(a.supports, [5, 2, 6, 3, 6, 3, 2, 7, 4, 2, 3]);
        assert_eq!(a.paths, [4, 5, 2, 3, 7, 8]);
        assert_eq!(a.mutations, [4, 6, 3, 6, 2, 3]);
        let b = fields(42, [-1, 3, 7]);
        assert_eq!(
            (b.fill_crust, b.fill_between, b.weave, b.grain_log2),
            (191, 11, 100, 11)
        );
        assert_eq!(b.paths, [3, 4, 2, 8, 4, 4]);
        let c = fields(7, [-100, 2, 33]);
        assert_eq!(c.orientation, OrientationPolicy::Axis);
        assert_eq!(c.forms, [7, 4, 5, 0, 5, 5, 6]);
    }

    fn with(policy: OrientationPolicy, weave: u8, grain_log2: u8) -> WebFields {
        WebFields {
            orientation: policy,
            weave,
            grain_log2,
            ..WebFields::NONE
        }
    }

    /// ORI-12: the calm default is the identity everywhere.
    #[test]
    fn axis_policy_is_identity() {
        let w = with(OrientationPolicy::Axis, 200, 8);
        for i in -20..20 {
            let c = I64Vec3::new(i, 2 * i, -i);
            assert_eq!(w.frame_l1(WorldSeed(3), c), Frame::Id);
            assert_eq!(w.frame_l2(WorldSeed(3), c), Frame::Id);
        }
        let w = with(OrientationPolicy::Radial, 0, 8);
        assert_eq!(w.frame_l2(WorldSeed(3), I64Vec3::ONE), Frame::Id);
    }

    /// ORI-13: one yaw per L1 cell, shared by its 8×8×8 L2 cells; quantised when weave < 64.
    #[test]
    fn yaw_per_l1_is_shared_by_children() {
        let seed = WorldSeed(9);
        let calm = with(OrientationPolicy::YawPerL1, 10, 12);
        let wild = with(OrientationPolicy::YawPerL1, 200, 12);
        let mut wild_odd = false;
        for i in -6..6i64 {
            let l1 = I64Vec3::new(i, -i, 3);
            let f = calm.frame_l1(seed, l1);
            for k in 0..8 {
                let l2 = l1 * 8 + I64Vec3::new(k, 7 - k, k % 3);
                assert_eq!(calm.frame_l2(seed, l2), f);
            }
            if let Frame::Table { yaw, tilt } = f {
                assert_eq!(tilt, 0);
                assert_eq!(yaw % 4, 0);
            }
            if let Frame::Table { yaw, .. } = wild.frame_l1(seed, l1) {
                wild_odd |= yaw % 4 != 0;
            }
        }
        assert!(wild_odd, "an unquantised yaw never appeared");
    }

    /// ORI-14: frames are constant inside a grain cell and differ between cells.
    #[test]
    fn grain_yaw_is_constant_inside_a_grain_cell() {
        let seed = WorldSeed(4);
        let w = with(OrientationPolicy::GrainYaw, 200, 8); // 256 m grains
        let l1 = I64Vec3::ZERO;
        let at = |p: [i64; 3]| w.frame_at(seed, I64Vec3::from_array(p), l1);
        assert_eq!(at([0, 0, 0]), at([255, 255, 255]));
        assert_eq!(at([-256, 0, 0]), at([-1, 255, 0]));
        let mut distinct = std::collections::HashSet::new();
        for i in -20..20 {
            distinct.insert(at([i * 256, 0, 0]));
        }
        assert!(distinct.len() > 8, "{} frames", distinct.len());
        // Another grain size never shares the stream.
        let w2 = with(OrientationPolicy::GrainYaw, 200, 9);
        let same = (0..40)
            .filter(|&i| {
                let p = I64Vec3::new(i * 512, 0, 0);
                w.frame_at(seed, p, l1) == w2.frame_at(seed, p, l1)
            })
            .count();
        assert!(same < 20, "{same}");
    }

    /// ORI-15: tilted frames pitch by `1..=max_tilt` and stay inside the table.
    #[test]
    fn tilted_grain_respects_the_cap() {
        let seed = WorldSeed(5);
        let w = with(OrientationPolicy::TiltedGrain { max_tilt: 3 }, 200, 7);
        let mut seen = [false; 8];
        for i in -300..300 {
            match w.frame_at(seed, I64Vec3::new(i * 128, i, 0), I64Vec3::ZERO) {
                Frame::Table { tilt, yaw } => {
                    assert!((1..=3).contains(&tilt) && yaw < 64);
                    seen[usize::from(tilt)] = true;
                }
                f => panic!("{f:?}"),
            }
        }
        assert!(seen[1] && seen[2] && seen[3]);
    }

    /// Negative coordinates use floor semantics: the cell holding -1 is not the one holding 0.
    #[test]
    fn grain_cells_use_floor_division() {
        let seed = WorldSeed(6);
        let w = with(OrientationPolicy::GrainYaw, 200, 6);
        let a = w.frame_at(seed, I64Vec3::new(-1, 0, 0), I64Vec3::ZERO);
        let b = w.frame_at(seed, I64Vec3::new(-64, 0, 0), I64Vec3::ZERO);
        assert_eq!(a, b);
    }
}
