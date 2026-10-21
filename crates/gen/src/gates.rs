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

/// LAD-14: the ladder of a ladder gate whose membrane is horizontal (the ladder runs along y):
/// a shaft of 18 m through the zone, with a wall to put the rungs on, a rung every 0.31 m, and a
/// landing at each end. In the field of the low side the climb is up; past the membrane it is
/// down, and the far landing is a floor from that side (the deck's underside). Cell-local
/// quanta; other axes and other kinds have none.
pub fn gate_ladder(zone: &GateZone, l2: I64Vec3) -> Vec<Primitive> {
    if zone.kind != GateKind::Ladder || zone.axis != 1 {
        return Vec::new();
    }
    let origin = [l2.x * L2_SIZE, l2.y * L2_SIZE, l2.z * L2_SIZE];
    let (cx, cz) = (
        i64::midpoint(zone.min[0], zone.max[0]),
        i64::midpoint(zone.min[2], zone.max[2]),
    );
    let (y0, y1) = (zone.min[1] - 9, zone.max[1] + 9);
    // Quanta from the cell's corner for absolute metres plus a number of sixteenths.
    let q = |x: i64, sx: i64, y: i64, sy: i64, z: i64, sz: i64| {
        QVec3::new(
            ((x - origin[0]) * 16 + sx) as i32,
            ((y - origin[1]) * 16 + sy) as i32,
            ((z - origin[2]) * 16 + sz) as i32,
        )
    };
    let boxed =
        |lo: QVec3, hi: QVec3, material: MaterialId, kind: FeatureKind, flags: PrimFlags| {
            Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(lo, hi),
                },
                material,
                kind,
                flags,
            }
        };
    let (solid, free) = (PrimFlags::NONE, PrimFlags::NO_COLLIDE);
    let mut out = Vec::new();
    // The climbing corridor (rungs on its +x side, code 1), not drawn and not solid.
    out.push(boxed(
        q(cx, 5, y0, 0, cz, -6),
        q(cx, 19, y1, 0, cz, 6),
        MaterialId(1),
        FeatureKind::LADDER,
        free,
    ));
    // The wall behind the rungs.
    out.push(boxed(
        q(cx, 20, y0, 0, cz, -16),
        q(cx, 30, y1, 0, cz, 16),
        MaterialId::CONCRETE,
        FeatureKind::SHAFT_WALL,
        solid,
    ));
    // The rungs, 5 sixteenths apart.
    let mut rung = 0i64;
    while y0 * 16 + rung < y1 * 16 {
        out.push(boxed(
            q(cx, 16, y0, rung, cz, -7),
            q(cx, 20, y0, rung + 1, cz, 7),
            MaterialId::METAL,
            FeatureKind::MISC,
            free,
        ));
        rung += 5;
    }
    // The landings: the low one is a floor of the low side, the high one a floor of the high side.
    for (ya, sy) in [(y0, -3i64), (y1, 0)] {
        out.push(boxed(
            q(cx, -32, ya, sy, cz, -19),
            q(cx, 5, ya, sy + 3, cz, 19),
            MaterialId::CONCRETE,
            FeatureKind::DECK,
            solid,
        ));
    }
    out
}

/// The markers and the ladders of every gate in L2 cell `l2`.
pub fn cell_markers(seed: WorldSeed, l2: I64Vec3) -> Vec<Primitive> {
    gates_in_l2(seed, l2)
        .iter()
        .flat_map(|z| {
            let mut v = gate_markers(z, l2);
            v.extend(gate_ladder(z, l2));
            v
        })
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

    /// LAD-14: a ladder gate with a horizontal membrane carries a climbable shaft through the zone,
    /// with a landing at each end, all inside the cell.
    #[test]
    fn a_ladder_gate_has_a_shaft_with_rungs_and_two_landings() {
        let mut found = 0;
        'search: for seed in (0..24).map(WorldSeed) {
            let Some((_, w)) = find_well(seed, WellKind::Seam, 40) else {
                continue;
            };
            if dir(w.axis)[1] == 0 {
                continue;
            }
            let c = w.centre;
            for dz in -10..10 {
                for dy in -10..10 {
                    for dx in -10..10 {
                        let l2 = I64Vec3::new(
                            c[0].div_euclid(L2_SIZE) + dx,
                            c[1].div_euclid(L2_SIZE) + dy,
                            c[2].div_euclid(L2_SIZE) + dz,
                        );
                        for g in gates_in_l2(seed, l2).iter() {
                            let pieces = gate_ladder(g, l2);
                            if g.kind != GateKind::Ladder || g.axis != 1 {
                                assert!(pieces.is_empty());
                                continue;
                            }
                            assert!(
                                pieces.iter().any(|p| p.kind == FeatureKind::LADDER),
                                "no climbing corridor"
                            );
                            let decks = pieces
                                .iter()
                                .filter(|p| p.kind == FeatureKind::DECK)
                                .count();
                            assert_eq!(decks, 2);
                            let rungs = pieces
                                .iter()
                                .filter(|p| p.material == MaterialId::METAL)
                                .count();
                            assert!(rungs >= 50, "{rungs} rungs");
                            let cell = 512 * 16;
                            for p in &pieces {
                                let b = p.aabb();
                                assert!(
                                    b.min.x >= 0
                                        && b.min.y >= 0
                                        && b.min.z >= 0
                                        && b.max.x <= cell
                                        && b.max.y <= cell
                                        && b.max.z <= cell,
                                    "{b:?} leaves its cell"
                                );
                            }
                            found += 1;
                            if found >= 2 {
                                break 'search;
                            }
                        }
                    }
                }
            }
        }
        assert!(found > 0, "no ladder gate with a horizontal membrane found");
    }

    #[test]
    fn markers_form_four_bars_that_do_not_collide() {
        let (l2, z) = first_gate();
        let m = gate_markers(&z, l2);
        assert_eq!(m.len(), 4);
        assert!(m.iter().all(|p| p.flags.contains(PrimFlags::NO_COLLIDE)));
    }
}
