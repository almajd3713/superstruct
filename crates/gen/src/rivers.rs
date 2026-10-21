//! Rivers (RIV-01..04, `docs/proposals/the-web-ideas.md` §3): pipe bundles 300–800 m wide. Spec:
//! `docs/spec/levels.md` §3W.8.
//!
//! A river is a function of an L0 line, like a road (`roads.rs`): it runs the whole row at one
//! inner L1 row, one height and one width; each pipe of the bundle is keyed by the line and its
//! place in the bundle, so every cell on the line draws the same pipes.

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::features::{Content, M};
use crate::hash::hash;
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const ONE_IN: u64 = 14;
const KIND_WEIGHTS: [u32; 4] = [4, 2, 2, 2];
/// Layers of pipes, and their spacing in metres.
const LAYERS: i32 = 3;
const LAYER_M: i32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RiverKind {
    /// RIV-01: a bundle of plain pipes.
    Plain,
    /// RIV-02: bands of light and warm pipes.
    Hot,
    /// RIV-03: rusted, with breaks in its pipes.
    Dead,
    /// RIV-04: it ends in the middle of every L0 run and the pipes hang open.
    Broken,
}

pub const KINDS: [RiverKind; 4] = [
    RiverKind::Plain,
    RiverKind::Hot,
    RiverKind::Dead,
    RiverKind::Broken,
];

impl RiverKind {
    pub const fn id(self) -> &'static str {
        match self {
            RiverKind::Plain => "RIV-01",
            RiverKind::Hot => "RIV-02",
            RiverKind::Dead => "RIV-03",
            RiverKind::Broken => "RIV-04",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct River {
    pub axis: usize,
    pub kind: RiverKind,
    /// Bundle width in metres, and the height of its lowest layer.
    pub width: i32,
    pub height: i32,
    /// The line's own hash, from which each pipe's draws come.
    pub key: u64,
}

fn line(seed: WorldSeed, axis: usize, row: (i64, i64)) -> Option<(River, (i64, i64))> {
    let key = I64Vec3::new(row.0, row.1, axis as i64);
    let h = hash(seed.0, Level::L0, key, salt::RIVER_LINE);
    let mut rng = Rng::new(h);
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    let kind = KINDS[rng.pick_weighted(&KIND_WEIGHTS).unwrap_or(0)];
    let (a, b) = (1 + rng.below(6) as i64, 1 + rng.below(6) as i64);
    let width = 300 + 100 * rng.below(6) as i32;
    let height = 800 + 128 * rng.below(12) as i32;
    Some((
        River {
            axis,
            kind,
            width,
            height,
            key: h,
        },
        (a, b),
    ))
}

/// The rivers through L1 cell `l1`: at most one along each axis.
pub fn rivers_in_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<River> {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let inner = l1 - l0 * 8;
    let mut out = Vec::new();
    if let Some((r, (a, b))) = line(seed, 0, (l0.y, l0.z))
        && (inner.y, inner.z) == (a, b)
    {
        out.push(r);
    }
    if let Some((r, (a, b))) = line(seed, 2, (l0.y, l0.x))
        && (inner.y, inner.x) == (a, b)
    {
        out.push(r);
    }
    out
}

fn build(river: River, l1: I64Vec3) -> Content {
    let along = if river.axis == 0 { l1.x } else { l1.z };
    // RIV-04: the bundle is absent from the middle two cells of every run of eight.
    let gone = river.kind == RiverKind::Broken && matches!(along.rem_euclid(8), 3 | 4);
    let c = 2048;
    let (c0, c1) = (c - river.width / 2, c + river.width / 2);
    let mut out: Vec<Primitive> = Vec::new();
    let axis = if river.axis == 0 { Axis::X } else { Axis::Z };
    let mut pipe = |a: i32,
                    len: i32,
                    across: i32,
                    y: i32,
                    radius: i32,
                    material: MaterialId,
                    flags: PrimFlags| {
        let base = if river.axis == 0 {
            QVec3::new(a * M, y * M, across * M)
        } else {
            QVec3::new(across * M, y * M, a * M)
        };
        out.push(Primitive {
            shape: Shape::cylinder(axis, base, len * M, radius * M),
            material,
            kind: FeatureKind::PIPE,
            flags,
        });
    };
    if !gone {
        for layer in 0..LAYERS {
            let mut j = 0;
            let mut x = c0 + 4;
            while x < c1 - 4 {
                // The pipe's own draws come from the line and its place in the bundle.
                let mut rng = Rng::new(hash(
                    river.key,
                    Level::L1,
                    I64Vec3::new(i64::from(j), i64::from(layer), 7),
                    salt::RIVER_LINE,
                ));
                let radius = [3, 4, 6][rng.below(3) as usize];
                let present = rng.below(4) != 0;
                let rusty = river.kind == RiverKind::Dead;
                let material = match river.kind {
                    RiverKind::Hot if rng.below(3) == 0 => MaterialId::WARM,
                    _ if rusty => MaterialId::CONCRETE,
                    _ => MaterialId::PIPE,
                };
                // RIV-03: each pipe breaks once in two cells, at a place its own hash draws.
                let (gap_at, gap) = (rng.below(3000) as i32, 24 + rng.below(40) as i32);
                let breaks = rusty && (along + i64::from(j)).rem_euclid(2) == 0;
                let y = river.height + layer * LAYER_M + radius;
                if present {
                    let solid = PrimFlags::FAR_VISIBLE;
                    if breaks {
                        pipe(0, gap_at, x, y, radius, material, solid);
                        pipe(
                            gap_at + gap,
                            4096 - gap_at - gap,
                            x,
                            y,
                            radius,
                            material,
                            solid,
                        );
                    } else {
                        pipe(0, 4096, x, y, radius, material, solid);
                    }
                    if river.kind == RiverKind::Hot {
                        // Bands of light every 64 m.
                        let mut a = 32;
                        while a < 4096 {
                            pipe(
                                a,
                                2,
                                x,
                                y,
                                radius + 1,
                                MaterialId::EMISSIVE,
                                PrimFlags::NO_COLLIDE,
                            );
                            a += 64;
                        }
                    }
                }
                x += 2 * radius + 4;
                j += 1;
            }
        }
        // A service catwalk along the top of the bundle, on posts every 64 m.
        let top = river.height + LAYERS * LAYER_M + 12;
        let deck = |lo: [i32; 3], hi: [i32; 3], material: MaterialId, kind: FeatureKind| {
            let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
            Primitive {
                shape: Shape::Box {
                    aabb: QAabb::new(q(lo), q(hi)),
                },
                material,
                kind,
                flags: PrimFlags::NONE,
            }
        };
        let at = |a0: i32, a1: i32, c0: i32, c1: i32, y0: i32, y1: i32| {
            if river.axis == 0 {
                ([a0, y0, c0], [a1, y1, c1])
            } else {
                ([c0, y0, a0], [c1, y1, a1])
            }
        };
        let (lo, hi) = at(0, 4096, c - 2, c + 2, top, top + 1);
        out.push(deck(lo, hi, MaterialId::GRATING, FeatureKind::WALKWAY));
        for edge in [c - 2, c + 1] {
            let (lo, hi) = at(0, 4096, edge, edge + 1, top + 1, top + 2);
            out.push(deck(lo, hi, MaterialId::METAL, FeatureKind::RAILING));
        }
        let mut a = 32;
        while a < 4096 {
            let (lo, hi) = at(a - 1, a + 1, c - 1, c + 1, river.height, top);
            out.push(deck(lo, hi, MaterialId::METAL, FeatureKind::MISC));
            a += 64;
        }
    }
    let height = river.height + LAYERS * LAYER_M + 20;
    let keep = if river.axis == 0 {
        QAabb::new(
            QVec3::new(0, (river.height - 2) * M, (c0 - 2) * M),
            QVec3::new(4096 * M, height * M, (c1 + 2) * M),
        )
    } else {
        QAabb::new(
            QVec3::new((c0 - 2) * M, (river.height - 2) * M, 0),
            QVec3::new((c1 + 2) * M, height * M, 4096 * M),
        )
    };
    Content {
        prims: out,
        keep_out: vec![keep],
    }
}

/// The geometry of the rivers through L1 cell `l1`.
pub fn river_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let mut all = Content::default();
    for river in rivers_in_cell(seed, l1) {
        let c = build(river, l1);
        all.prims.extend(c.prims);
        all.keep_out.extend(c.keep_out);
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: RiverKind) -> (I64Vec3, River) {
        for y in -4..4 {
            for z in -60..60 {
                if let Some((r, (a, b))) = line(SEED, 0, (y, z)).filter(|(r, _)| r.kind == kind) {
                    return (I64Vec3::new(0, y * 8 + a, z * 8 + b), r);
                }
            }
        }
        panic!("{kind:?} not found");
    }

    #[test]
    fn every_kind_occurs_and_builds_a_wide_bundle() {
        for kind in KINDS {
            let (l1, r) = find(kind);
            assert!((300..=800).contains(&r.width), "{kind:?}");
            let c = river_content(SEED, l1 + I64Vec3::new(5, 0, 0));
            if kind == RiverKind::Broken {
                // A broken river has none in two of every eight cells.
                let along = |k: i64| {
                    river_content(SEED, l1 + I64Vec3::new(k, 0, 0))
                        .prims
                        .is_empty()
                };
                assert!((0..8).any(along) && (0..8).any(|k| !along(k)), "{kind:?}");
            } else {
                assert!(c.prims.len() > 60, "{kind:?}: {} pieces", c.prims.len());
            }
            assert!(!c.keep_out.is_empty());
        }
    }

    /// Every cell on the line draws the same pipes: the bundle meets itself at every face.
    #[test]
    fn a_river_is_the_same_in_every_cell_of_its_row() {
        let (l1, _) = find(RiverKind::Plain);
        let a = river_content(SEED, l1 + I64Vec3::new(2, 0, 0));
        let b = river_content(SEED, l1 + I64Vec3::new(11, 0, 0));
        assert_eq!(a.prims, b.prims);
    }

    #[test]
    fn hot_rivers_glow_and_dead_ones_break() {
        let (l1, _) = find(RiverKind::Hot);
        let c = river_content(SEED, l1);
        assert!(c.prims.iter().any(|p| p.material == MaterialId::EMISSIVE));
        let (l1, _) = find(RiverKind::Dead);
        let plain = |k: i64| river_content(SEED, l1 + I64Vec3::new(k, 0, 0)).prims.len();
        assert_ne!(
            plain(0),
            plain(1),
            "pipes break in one cell and not in the next"
        );
    }
}
