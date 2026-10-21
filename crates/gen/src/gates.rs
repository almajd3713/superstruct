//! Gravity gates: thresholds where "down" turns over a short distance. Spec: `docs/spec/gravity.md` §4.

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_range_loop
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use glam::I64Vec3;

use crate::cell::Level;
use crate::gravity::{Gravity, WellKind, dir, smooth_coverage, wells_of};
use crate::hash::hash;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const L2_SIZE: i64 = 512;
const L0_SIZE: i64 = 32768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    Door,
    StairTurnover,
    Ladder,
    Bridge,
}

/// A box in absolute metres in which `up` turns from `from_up` at its low end to `to_up` at the
/// high end along `axis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateZone {
    pub min: [i64; 3],
    pub max: [i64; 3],
    pub from_up: [i8; 3],
    pub to_up: [i8; 3],
    pub axis: usize,
    pub kind: GateKind,
}

impl GateZone {
    pub fn contains(&self, p: [i64; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] < self.max[i])
    }

    /// Half sizes of the lateral axes' ring (the opening) in metres.
    pub fn lateral_half(&self) -> i64 {
        let a = self.axis;
        let b = (a + 1) % 3;
        (self.max[b] - self.min[b]) / 2
    }

    /// The field inside the zone at `p` (spec §4): the integer smoothstep between the two ups.
    pub fn gravity_at(&self, p: [i64; 3]) -> Gravity {
        let a = self.axis;
        let t = ((p[a] - self.min[a]) * 4096 / (self.max[a] - self.min[a])).clamp(0, 4096);
        // With reach 16384 the taper parameter of `smooth_coverage` equals `16384 - d`, so
        // this is `256 · S(t)` in fixed point.
        let k = smooth_coverage(16384, 16384 - t);
        let mut v = [0i64; 3];
        for i in 0..3 {
            v[i] = (256 - k) * i64::from(self.from_up[i]) + k * i64::from(self.to_up[i]);
        }
        let m = v[0].abs().max(v[1].abs()).max(v[2].abs());
        let strength = (m / 8).min(256);
        if strength < 4 {
            return Gravity {
                up: [0; 3],
                strength: strength as u16,
            };
        }
        let q = crate::gravity::quantise(v);
        Gravity {
            up: q.map(|x| x as i8),
            strength: strength as u16,
        }
    }
}

const KIND_WEIGHTS: [u32; 4] = [55, 20, 15, 10];
const KINDS: [GateKind; 4] = [
    GateKind::Door,
    GateKind::StairTurnover,
    GateKind::Ladder,
    GateKind::Bridge,
];

/// `(thickness, lateral half)` in metres: a door is a 2 m wall with a 3 m opening each side, a
/// stair's last 2 m with a 2 m half, a ladder's top with a 1.5 m half, a bridge's 16 m ring.
const fn dims(kind: GateKind) -> (i64, i64) {
    match kind {
        GateKind::Door => (2, 3),
        GateKind::StairTurnover | GateKind::Ladder => (2, 2),
        GateKind::Bridge => (16, 8),
    }
}

type Cache = Mutex<HashMap<(u64, I64Vec3), Arc<Vec<GateZone>>>>;

/// The gate zones whose box is in L2 cell `l2` (cached; the cache never changes a result).
pub fn gates_in_l2(seed: WorldSeed, l2: I64Vec3) -> Arc<Vec<GateZone>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(g) = cache
        .lock()
        .ok()
        .and_then(|m| m.get(&(seed.0, l2)).cloned())
    {
        return g;
    }
    let gates = Arc::new(draw_gates(seed, l2));
    if let Ok(mut m) = cache.lock() {
        m.insert((seed.0, l2), Arc::clone(&gates));
    }
    gates
}

fn draw_gates(seed: WorldSeed, l2: I64Vec3) -> Vec<GateZone> {
    let lo = [l2.x * L2_SIZE, l2.y * L2_SIZE, l2.z * L2_SIZE];
    let home = I64Vec3::new(
        lo[0].div_euclid(L0_SIZE),
        lo[1].div_euclid(L0_SIZE),
        lo[2].div_euclid(L0_SIZE),
    );
    let mut out = Vec::new();
    let mut rng = Rng::new(hash(seed.0, Level::L2, l2, salt::GATE));
    // Every draw is made whether or not a seam crosses the cell, so the stream is fixed.
    let place = rng.chance(0.5);
    let kind = KINDS[rng.pick_weighted(&KIND_WEIGHTS).unwrap_or(0)];
    let lateral = [rng.below(24) as i64, rng.below(24) as i64];
    if !place {
        return out;
    }
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                for w in wells_of(seed, home + I64Vec3::new(dx, dy, dz)).iter() {
                    if w.kind != WellKind::Seam {
                        continue;
                    }
                    let n = dir(w.axis);
                    let a = (0..3).position(|i| n[i] != 0).unwrap_or(1);
                    let pc = w.centre[a];
                    if pc < lo[a] + 32 || pc >= lo[a] + L2_SIZE - 32 {
                        continue;
                    }
                    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
                    let pb = lo[b] + 64 + 16 * lateral[0];
                    let pcc = lo[c] + 64 + 16 * lateral[1];
                    let inside = |i: usize, v: i64| (v - w.centre[i]).abs() <= w.extent[i] - 16;
                    if !inside(b, pb) || !inside(c, pcc) {
                        continue;
                    }
                    let (thick, half) = dims(kind);
                    let mut min = [0i64; 3];
                    let mut max = [0i64; 3];
                    min[a] = pc - thick / 2;
                    max[a] = pc + thick / 2;
                    min[b] = pb - half;
                    max[b] = pb + half;
                    min[c] = pcc - half;
                    max[c] = pcc + half;
                    let mut up = [0i8; 3];
                    up[a] = 8;
                    // Below the membrane (low end) the field is `-n`, above it `+n` (spec §2.1).
                    out.push(GateZone {
                        min,
                        max,
                        from_up: up.map(|x| -x),
                        to_up: up,
                        axis: a,
                        kind,
                    });
                }
            }
        }
    }
    out
}

/// `base`, replaced by the gate zone's field if `p` is inside one.
pub fn apply_gates(seed: WorldSeed, p: [i64; 3], base: Gravity) -> Gravity {
    let l2 = I64Vec3::new(
        p[0].div_euclid(L2_SIZE),
        p[1].div_euclid(L2_SIZE),
        p[2].div_euclid(L2_SIZE),
    );
    match gates_in_l2(seed, l2).iter().find(|z| z.contains(p)) {
        Some(z) => z.gravity_at(p),
        None => base,
    }
}

/// The ring of light that marks a gate, as cell-local primitives (1/16 m): four emissive bars
/// round the opening in the plane of the zone, which nothing collides with.
pub fn gate_markers(zone: &GateZone, l2: I64Vec3) -> Vec<Primitive> {
    let origin = [l2.x * L2_SIZE, l2.y * L2_SIZE, l2.z * L2_SIZE];
    let a = zone.axis;
    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
    let q = |v: [i64; 3]| {
        let l = |i: usize| ((v[i] - origin[i]) * 16) as i32;
        QVec3::new(l(0), l(1), l(2))
    };
    let (mid_a, bar) = (i64::midpoint(zone.min[a], zone.max[a]), 1);
    let mut out = Vec::new();
    for (along, across, sign) in [(b, c, -1), (b, c, 1), (c, b, -1), (c, b, 1)] {
        let mut lo = [0i64; 3];
        let mut hi = [0i64; 3];
        lo[a] = mid_a - bar;
        hi[a] = mid_a + bar;
        let edge = if sign < 0 {
            zone.min[across] - bar
        } else {
            zone.max[across]
        };
        lo[across] = edge;
        hi[across] = edge + bar;
        lo[along] = zone.min[along] - bar;
        hi[along] = zone.max[along] + bar;
        out.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material: MaterialId::EMISSIVE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NO_COLLIDE,
        });
    }
    out
}

/// The markers of every gate in L2 cell `l2`.
pub fn cell_markers(seed: WorldSeed, l2: I64Vec3) -> Vec<Primitive> {
    gates_in_l2(seed, l2)
        .iter()
        .flat_map(|z| gate_markers(z, l2))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gravity::{find_well, gravity_at_metres};

    const SEED: WorldSeed = WorldSeed(0);

    fn first_gate() -> (I64Vec3, GateZone) {
        let (l0, w) = find_well(SEED, WellKind::Seam, 40).expect("a seam exists");
        let c = w.centre;
        for dz in -8..8 {
            for dy in -8..8 {
                for dx in -8..8 {
                    let l2 = I64Vec3::new(
                        c[0].div_euclid(L2_SIZE) + dx,
                        c[1].div_euclid(L2_SIZE) + dy,
                        c[2].div_euclid(L2_SIZE) + dz,
                    );
                    if let Some(z) = gates_in_l2(SEED, l2).first() {
                        let _ = l0;
                        return (l2, *z);
                    }
                }
            }
        }
        panic!("no gate near the first seam");
    }

    #[test]
    fn a_seam_has_gates_and_the_field_turns_inside() {
        let (_, z) = first_gate();
        let a = z.axis;
        let mut lo = [0i64; 3];
        for i in 0..3 {
            lo[i] = i64::midpoint(z.min[i], z.max[i]);
        }
        let mut below = lo;
        below[a] = z.min[a] - 1;
        let mut above = lo;
        above[a] = z.max[a];
        let g0 = gravity_at_metres(SEED, below);
        let g1 = gravity_at_metres(SEED, above);
        assert_eq!(g0.up[a], -8, "{g0:?}");
        assert_eq!(g1.up[a], 8, "{g1:?}");
        let mid = gravity_at_metres(SEED, lo);
        assert!(mid.strength < 256, "the zone does not override: {mid:?}");
    }

    #[test]
    fn gates_are_pure_and_inside_their_cell() {
        let (l2, z) = first_gate();
        let again = gates_in_l2(SEED, l2);
        assert!(again.contains(&z));
        for i in 0..3 {
            assert!(z.min[i] >= l2[i] * L2_SIZE && z.max[i] <= (l2[i] + 1) * L2_SIZE);
        }
    }

    #[test]
    fn the_turn_is_gentle_and_passes_through_weightlessness_when_opposed() {
        let (_, z) = first_gate();
        let a = z.axis;
        let mut p = [0i64; 3];
        for i in 0..3 {
            p[i] = i64::midpoint(z.min[i], z.max[i]);
        }
        let mut weak = 256;
        let mut prev = None::<Gravity>;
        for k in 0..(z.max[a] - z.min[a]) * 8 {
            p[a] = z.min[a] + k / 8;
            let g = z.gravity_at(p);
            weak = weak.min(g.strength);
            if let Some(q) = prev {
                let step = (0..3)
                    .map(|i| (i32::from(g.up[i]) - i32::from(q.up[i])).abs())
                    .max()
                    .unwrap_or(0);
                assert!(g.is_lull() || q.is_lull() || step <= 8, "{q:?} -> {g:?}");
            }
            prev = Some(g);
        }
        assert!(weak < 64, "no weak point inside the gate: {weak}");
    }

    #[test]
    fn markers_form_four_bars_that_do_not_collide() {
        let (l2, z) = first_gate();
        let m = gate_markers(&z, l2);
        assert_eq!(m.len(), 4);
        assert!(m.iter().all(|p| p.flags.contains(PrimFlags::NO_COLLIDE)));
    }
}
