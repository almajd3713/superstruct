//! The character of a stratum: what a crust is made of, drawn once per `(L0 cell, k)` and read
//! by every cell inside it (SC-01..SC-11). Spec: `docs/spec/regimes.md` §10.

use glam::I64Vec3;

use crate::cell::Level;
use crate::hash::{absorb, hash};
use crate::regime::{PaletteId, STYLES, regime};
use crate::regime_web::{GRAIN_LOG2, OrientationPolicy, PATH_STAIR, PATH_WALK, WEB_STYLES};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Segment, Stratum, UNITS, stack, u_units};

/// `(num, den)` of the tissue scale for each `Character::scale` index (SC-05).
pub const SCALES: [(u8, u8); 7] = [(1, 2), (3, 4), (1, 1), (3, 2), (2, 1), (3, 1), (4, 1)];
const SCALE_WEIGHTS: [u32; 7] = [2, 4, 40, 4, 3, 1, 1];
/// Unused quarter slot.
pub const NO_QUARTER: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Character {
    /// SC-02: 2–4 distinct dominant QTR indices (0-based), unused slots `NO_QUARTER`.
    pub quarters: [u8; 4],
    pub forms: [u8; 7],
    pub supports: [u8; 11],
    pub paths: [u8; 6],
    pub mutations: [u8; 6],
    pub grain_log2: u8,
    pub orientation: OrientationPolicy,
    pub scale: u8,
    pub fill: u8,
    pub materials: [u8; 4],
    pub palette: PaletteId,
    pub light: u8,
    pub lamps: u8,
}

impl Character {
    pub const fn scale_ratio(&self) -> (u8, u8) {
        SCALES[self.scale as usize]
    }

    /// The dominant quarters in use.
    pub fn dominant(&self) -> impl Iterator<Item = u8> + '_ {
        self.quarters.iter().copied().filter(|&q| q != NO_QUARTER)
    }
}

/// Perturbs each regime weight by −3..=3 (SC-03). A regime ban (0) stays a ban; the draw is
/// still made.
fn perturb<const N: usize>(rng: &mut Rng, base: [u8; N]) -> [u8; N] {
    base.map(|b| {
        let d = rng.range_i32(-3, 4);
        if b == 0 {
            0
        } else {
            u8::try_from((i32::from(b) + d).clamp(0, 8)).unwrap_or(0)
        }
    })
}

/// Perturb, boost 1–3 entries to 8, then ban 0–2 entries whose regime weight is at most 4.
/// A ban that would leave a `keep` entry at zero, or every entry at zero, is skipped; its
/// draw is consumed.
fn category<const N: usize>(rng: &mut Rng, base: [u8; N], keep: &[usize]) -> [u8; N] {
    let mut w = perturb(rng, base);
    for _ in 0..rng.range_i32(1, 4) {
        let weights = w.map(u32::from);
        if let Some(i) = rng.pick_weighted(&weights) {
            w[i] = 8;
        }
    }
    for _ in 0..rng.range_i32(0, 3) {
        let weights = base.map(|b| u32::from(b > 0 && b <= 4));
        let Some(i) = rng.pick_weighted(&weights) else {
            continue;
        };
        let mut next = w;
        next[i] = 0;
        if next.iter().any(|&v| v > 0) && keep.iter().all(|&k| next[k] > 0 || w[k] == 0) {
            w = next;
        }
    }
    w
}

/// The character of crust `k` of L0 cell `l0`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // draws are range-limited
pub fn character(seed: WorldSeed, l0: I64Vec3, k: u16) -> Character {
    let reg = regime(seed, l0);
    let web = &reg.web;
    let style = usize::from(reg.style.0);
    let def = &WEB_STYLES[style];
    let mut rng = Rng::new(absorb(
        hash(seed.0, Level::L0, l0, salt::STRATUM_CHAR),
        u64::from(k),
    ));

    let mut quarters = [NO_QUARTER; 4];
    let mut pool = web.quarters.map(u32::from);
    for slot in quarters.iter_mut().take(rng.range_i32(2, 5) as usize) {
        if let Some(i) = rng.pick_weighted(&pool) {
            *slot = i as u8;
            pool[i] = 0;
        }
    }
    let forms = category(&mut rng, web.forms, &[]);
    let supports = category(&mut rng, web.supports, &[]);
    let mut paths = category(&mut rng, web.paths, &[PATH_WALK, PATH_STAIR]);
    paths[PATH_WALK] = paths[PATH_WALK].max(1);
    paths[PATH_STAIR] = paths[PATH_STAIR].max(1);
    let mutations = category(&mut rng, web.mutations, &[]);

    let grain_log2 = (i32::from(web.grain_log2) + rng.range_i32(-2, 3))
        .clamp(i32::from(GRAIN_LOG2.0), i32::from(GRAIN_LOG2.1)) as u8;
    let shuffled = rng.chance(0.25);
    let policy = rng.pick_weighted(&def.orientation).unwrap_or(0);
    let tilt = rng.range_i32(1, i32::from(def.tilt_cap) + 1) as u8;
    let orientation = if shuffled {
        match policy {
            1 => OrientationPolicy::YawPerL1,
            2 => OrientationPolicy::GrainYaw,
            3 => OrientationPolicy::TiltedGrain { max_tilt: tilt },
            4 => OrientationPolicy::Radial,
            _ => OrientationPolicy::Axis,
        }
    } else {
        web.orientation
    };
    let scale = rng.pick_weighted(&SCALE_WEIGHTS).unwrap_or(2) as u8;
    let fill = (i32::from(web.fill_crust) + rng.range_i32(-12, 13)).clamp(178, 230) as u8;
    let materials = [0; 4].map(|_| rng.range_i32(0, 9) as u8);
    let other = rng.chance(1.0 / 3.0);
    let list = STYLES[style].palettes;
    let pick = list[rng.below(list.len() as u64) as usize];
    let palette = if other { pick } else { reg.palette };
    let jitter = rng.range_i32(-15, 16);
    let light = if reg.light == 0 {
        0
    } else {
        (i32::from(reg.light) + jitter).clamp(0, 255) as u8
    };
    let lamps = rng.range_i32(0, 256) as u8;
    Character {
        quarters,
        forms,
        supports,
        paths,
        mutations,
        grain_log2,
        orientation,
        scale,
        fill,
        materials,
        palette,
        light,
        lamps,
    }
}

/// The character at `local_m` (metres inside L0 cell `l0`): the crust's, or `None` in a
/// Between (`regimes.md` §10.4).
pub fn character_at(seed: WorldSeed, l0: I64Vec3, local_m: I64Vec3) -> Option<Character> {
    match stack(seed, l0).stratum_at(local_m, None) {
        Stratum::Crust(k) => Some(character(seed, l0, k)),
        Stratum::Between(_) => None,
    }
}

/// The character that dresses a position: the crust's own, or in a Between that of the nearer
/// crust (ties to the lower one). `None` only when the stack has no crust at all.
pub fn character_near(seed: WorldSeed, l0: I64Vec3, local_m: I64Vec3) -> Option<Character> {
    let st = stack(seed, l0);
    let u = u_units(st.geometry, local_m, st.well).rem_euclid(UNITS);
    let i = st.segments.partition_point(|s| i64::from(s.end) <= u);
    let i = i.min(st.segments.len() - 1);
    let crust = |s: &Segment| match s.stratum {
        Stratum::Crust(k) => Some(k),
        Stratum::Between(_) => None,
    };
    if let Some(k) = crust(&st.segments[i]) {
        return Some(character(seed, l0, k));
    }
    let below = st.segments[..i]
        .iter()
        .rev()
        .find_map(|s| crust(s).map(|k| (k, u - i64::from(s.end))));
    let above = st.segments[i + 1..]
        .iter()
        .find_map(|s| crust(s).map(|k| (k, i64::from(s.start) - u)));
    let k = match (below, above) {
        (Some((a, da)), Some((b, db))) => {
            if da <= db {
                a
            } else {
                b
            }
        }
        (Some((k, _)), None) | (None, Some((k, _))) => k,
        (None, None) => return None,
    };
    Some(character(seed, l0, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> WorldSeed {
        WorldSeed(0x5eed)
    }

    fn sample() -> Vec<(I64Vec3, u16, Character)> {
        let mut v = Vec::new();
        for x in -4..4 {
            for z in -4..4 {
                let l0 = I64Vec3::new(x, 0, z);
                for seg in stack(seed(), l0).crusts() {
                    if let Stratum::Crust(k) = seg.stratum {
                        v.push((l0, k, character(seed(), l0, k)));
                    }
                }
            }
        }
        v
    }

    /// SC-01..09: pure, well-formed, and the routing fallbacks survive.
    #[test]
    fn characters_are_pure_and_well_formed() {
        for (l0, k, c) in sample() {
            assert_eq!(c, character(seed(), l0, k));
            let n = c.dominant().count();
            assert!((2..=4).contains(&n), "{n} quarters");
            let mut q: Vec<u8> = c.dominant().collect();
            q.sort_unstable();
            q.dedup();
            assert_eq!(q.len(), n, "distinct quarters");
            assert!(c.forms.iter().any(|&w| w > 0));
            assert!(c.supports.iter().any(|&w| w > 0));
            assert!(c.paths[PATH_WALK] > 0 && c.paths[PATH_STAIR] > 0);
            for w in c
                .forms
                .iter()
                .chain(&c.supports)
                .chain(&c.paths)
                .chain(&c.mutations)
            {
                assert!(*w <= 8);
            }
            assert!((GRAIN_LOG2.0..=GRAIN_LOG2.1).contains(&c.grain_log2));
            assert!((178..=230).contains(&c.fill));
            assert!(c.materials.iter().all(|&m| m <= 8));
            assert!(usize::from(c.scale) < SCALES.len());
        }
    }

    /// SC-01: strata differ from one another; SC-04/05: policies shift, scale ≠ 1 is rare.
    #[test]
    fn strata_differ_and_scale_is_rare() {
        let all = sample();
        assert!(all.len() > 100);
        let distinct = all.iter().filter(|(_, _, c)| *c != all[0].2).count();
        assert!(distinct * 10 > all.len() * 9);
        let odd = all.iter().filter(|(_, _, c)| c.scale != 2).count();
        assert!(
            odd > 0 && odd * 100 < all.len() * 40,
            "{odd} of {}",
            all.len()
        );
        let shifted = all
            .iter()
            .filter(|(l0, _, c)| c.orientation != regime(seed(), *l0).web.orientation)
            .count();
        assert!(shifted > 0, "some strata break with the regime's policy");
    }

    #[test]
    fn lookups_follow_the_stack() {
        let l0 = I64Vec3::new(2, 0, 3);
        let st = stack(seed(), l0);
        let mut crust_hits = 0;
        let mut between_hits = 0;
        for y in (0..32768).step_by(256) {
            let p = I64Vec3::new(y, y, y);
            let at = character_at(seed(), l0, p);
            let near = character_near(seed(), l0, p).unwrap();
            match st.stratum_at(p, None) {
                Stratum::Crust(k) => {
                    assert_eq!(at, Some(character(seed(), l0, k)));
                    assert_eq!(at, Some(near));
                    crust_hits += 1;
                }
                Stratum::Between(_) => {
                    assert!(at.is_none());
                    between_hits += 1;
                }
            }
        }
        assert!(crust_hits > 0 && between_hits > 0);
    }
}
