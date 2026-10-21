//! Strange regions (STG-01..13, `docs/proposals/the-web-ideas.md` §16): whole L0 cells that break
//! a rule of the ordinary city. Spec: `docs/spec/levels.md` §3W.11.
//!
//! About one L0 cell in fourteen is strange, with one of eleven characters; a twelfth, the
//! Rampart, is a wall along an L0 line, and the Far Lands are the cells farther from the origin
//! than 2^20 L0 cells. Most change how the tissue grows (`Tissue::strange`, `Tissue::rare`), a few
//! add geometry (`strange_content`, `cell_pieces`), the rest are engine rules (`silent_at`,
//! `time_scale_at`, the fog of `atmosphere_at`).

// Draws are small and positions are below 2^40 m, so the casts below never lose bits.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::needless_range_loop,
    clippy::too_many_lines
)]

use glam::I64Vec3;

use crate::cell::Level;
use crate::features::{Content, M};
use crate::hash::hash;
use crate::l1::CELL;
use crate::orient::{FacetRing, facet_ring};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strange {
    /// STG-01: the Library as a whole region: only Archive quarters and gallery stacks.
    Library,
    /// STG-02: the Recursion: every structure swallows a smaller copy of an older form.
    Recursion,
    /// STG-03: the Inversion: the whole region hangs (RARE-01 for every cell).
    Inversion,
    /// STG-04: the Grid: every structure is one of the same cube of 64 m.
    Grid,
    /// STG-05: the Field of Doors: free-standing doorways everywhere.
    FieldOfDoors,
    /// STG-06: the Labyrinth: a maze of 2 km at the middle of the region.
    Labyrinth,
    /// STG-07: a fog district: the visibility is a sixth of what the regime says.
    Fog,
    /// STG-08: a silent district: the City's sound is off.
    Silent,
    /// STG-09: the Slow district: the world clock runs at a tenth there.
    Slow,
    /// STG-10: the Miniature: everything at half scale (RARE-04 for every cell).
    Miniature,
    /// STG-11: the Far Lands: beyond 2^20 L0 cells from the origin, everything at ×4 and in fog.
    FarLands,
    /// STG-13: the Builder frontier: half the movers are Builders.
    BuilderFrontier,
}

/// The characters a region is drawn from (the Far Lands are by distance).
pub const DRAWN: [Strange; 11] = [
    Strange::Library,
    Strange::Recursion,
    Strange::Inversion,
    Strange::Grid,
    Strange::FieldOfDoors,
    Strange::Labyrinth,
    Strange::Fog,
    Strange::Silent,
    Strange::Slow,
    Strange::Miniature,
    Strange::BuilderFrontier,
];

impl Strange {
    pub const fn id(self) -> &'static str {
        match self {
            Strange::Library => "STG-01",
            Strange::Recursion => "STG-02",
            Strange::Inversion => "STG-03",
            Strange::Grid => "STG-04",
            Strange::FieldOfDoors => "STG-05",
            Strange::Labyrinth => "STG-06",
            Strange::Fog => "STG-07",
            Strange::Silent => "STG-08",
            Strange::Slow => "STG-09",
            Strange::Miniature => "STG-10",
            Strange::FarLands => "STG-11",
            Strange::BuilderFrontier => "STG-13",
        }
    }
}

/// An L0 cell is strange with chance 1 in this.
const ONE_IN: u64 = 14;
/// L0 cells from the origin beyond which the Far Lands begin.
pub const FAR_LANDS: i64 = 1 << 20;

/// The character of L0 cell `l0`, if it is strange.
pub fn strange_of(seed: WorldSeed, l0: I64Vec3) -> Option<Strange> {
    if l0.abs().max_element() >= FAR_LANDS {
        return Some(Strange::FarLands);
    }
    let mut rng = Rng::new(hash(seed.0, Level::L0, l0, salt::STRANGE_SITE));
    if rng.below(ONE_IN) != 0 {
        return None;
    }
    Some(DRAWN[rng.below(DRAWN.len() as u64) as usize])
}

/// The rare event a strange region wears for the tissue: inversion hangs everything, the
/// miniature and the Far Lands change the scale.
pub fn rare_of(s: Strange) -> Option<crate::rare::Rare> {
    match s {
        Strange::Inversion => Some(crate::rare::Rare::Inverted),
        Strange::Miniature => Some(crate::rare::Rare::Dollhouse),
        Strange::FarLands => Some(crate::rare::Rare::ScaleBreak),
        _ => None,
    }
}

fn l0_of_metres(p: [i64; 3]) -> I64Vec3 {
    I64Vec3::new(
        p[0].div_euclid(32768),
        p[1].div_euclid(32768),
        p[2].div_euclid(32768),
    )
}

/// STG-08: whether the City is silent at `p` (absolute metres).
pub fn silent_at(seed: WorldSeed, p: [i64; 3]) -> bool {
    strange_of(seed, l0_of_metres(p)) == Some(Strange::Silent)
}

/// STG-09: the rate of the world clock at `p` as `(numerator, denominator)`: 1 in 10 in the Slow
/// district, 1 elsewhere.
pub fn time_scale_at(seed: WorldSeed, p: [i64; 3]) -> (u64, u64) {
    if strange_of(seed, l0_of_metres(p)) == Some(Strange::Slow) {
        (1, 10)
    } else {
        (1, 1)
    }
}

/// STG-07, STG-11: how much thicker the air is in L0 cell `l0` than its regime says.
pub fn fog_factor(seed: WorldSeed, l0: I64Vec3) -> f32 {
    match strange_of(seed, l0) {
        Some(Strange::Fog) => 6.0,
        Some(Strange::FarLands) => 3.0,
        _ => 1.0,
    }
}

/// STG-05: the free-standing doorways of L2 cell `l2` in a Field of Doors: 24 frames of two
/// posts and a lintel at hashed places, 3 m wide and 2.6 m tall (cell-local quanta).
pub fn cell_pieces(seed: WorldSeed, l2: I64Vec3) -> Vec<Primitive> {
    let l0 = l2.div_euclid(I64Vec3::splat(64));
    if strange_of(seed, l0) != Some(Strange::FieldOfDoors) {
        return Vec::new();
    }
    let mut rng = Rng::new(hash(seed.0, Level::L2, l2, salt::STRANGE_BUILD));
    let mut out = Vec::new();
    let q = |v: [f64; 3]| {
        QVec3::new(
            (v[0] * 16.0) as i32,
            (v[1] * 16.0) as i32,
            (v[2] * 16.0) as i32,
        )
    };
    for _ in 0..24 {
        let (x, y, z) = (
            12.0 + f64::from(rng.below(480) as u32),
            8.0 + f64::from(rng.below(480) as u32),
            12.0 + f64::from(rng.below(480) as u32),
        );
        let along_x = rng.below(2) == 0;
        let frame = |lo: [f64; 3], hi: [f64; 3]| Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        };
        if along_x {
            out.push(frame([x, y, z], [x + 0.4, y + 2.6, z + 0.4]));
            out.push(frame([x + 3.0, y, z], [x + 3.4, y + 2.6, z + 0.4]));
            out.push(frame([x, y + 2.6, z], [x + 3.4, y + 3.0, z + 0.4]));
        } else {
            out.push(frame([x, y, z], [x + 0.4, y + 2.6, z + 0.4]));
            out.push(frame([x, y, z + 3.0], [x + 0.4, y + 2.6, z + 3.4]));
            out.push(frame([x, y + 2.6, z], [x + 0.4, y + 3.0, z + 3.4]));
        }
    }
    out
}

/// A Rampart along L0 line `(row, row)` on `axis` (0 for x, 2 for z): its inner L1 row, and the
/// L1 cell index (mod 8, along the line) that holds its gate.
pub fn rampart_line(seed: WorldSeed, axis: usize, row: (i64, i64)) -> Option<((i64, i64), i64)> {
    let mut rng = Rng::new(hash(
        seed.0,
        Level::L0,
        I64Vec3::new(row.0, row.1, axis as i64),
        salt::RAMPART_LINE,
    ));
    if rng.below(40) != 0 {
        return None;
    }
    Some((
        (1 + rng.below(6) as i64, 1 + rng.below(6) as i64),
        rng.below(8) as i64,
    ))
}

struct B {
    out: Vec<Primitive>,
}

impl B {
    fn boxed(
        &mut self,
        lo: [i32; 3],
        hi: [i32; 3],
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        if (0..3).any(|i| lo[i] >= hi[i]) {
            return;
        }
        let q = |v: [i32; 3]| QVec3::new(v[0] * M, v[1] * M, v[2] * M);
        self.out.push(Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(q(lo), q(hi)),
            },
            material,
            kind,
            flags,
        });
    }
}

fn volume(lo: [i32; 3], hi: [i32; 3]) -> QAabb {
    QAabb::new(
        QVec3::new(lo[0] * M, lo[1] * M, lo[2] * M),
        QVec3::new(hi[0] * M, hi[1] * M, hi[2] * M),
    )
}

/// STG-12, WON-24: the Rampart in L1 cell `l1`: a wall of 4 km by 2 km and 64 m thick along its
/// line, with a gate of 60 by 120 m in one cell of every eight (the Gate, WON-24).
fn rampart(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let inner = l1 - l0 * 8;
    let mut hit = None;
    if let Some(((a, b), gate)) = rampart_line(seed, 0, (l0.y, l0.z))
        && (inner.y, inner.z) == (a, b)
    {
        hit = Some((0, gate, inner.x));
    } else if let Some(((a, b), gate)) = rampart_line(seed, 2, (l0.y, l0.x))
        && (inner.y, inner.x) == (a, b)
    {
        hit = Some((2, gate, inner.z));
    }
    let Some((axis, gate, along)) = hit else {
        return Content::default();
    };
    let c = 2048;
    let mut b = B { out: Vec::new() };
    let far = PrimFlags::FAR_VISIBLE;
    let conc = MaterialId::CONCRETE;
    let at = |a0: i32, a1: i32, y0: i32, y1: i32| {
        if axis == 0 {
            ([a0, y0, c - 32], [a1, y1, c + 32])
        } else {
            ([c - 32, y0, a0], [c + 32, y1, a1])
        }
    };
    let (g0, g1) = (c - 30, c + 30);
    let mut parts = Vec::new();
    if along == gate {
        // The Gate: a door of 60 by 120 m; a lintel above it and a sill of light.
        parts.push(at(0, g0, 0, 2048));
        parts.push(at(g1, 4096, 0, 2048));
        parts.push(at(g0, g1, 120, 2048));
    } else {
        parts.push(at(0, 4096, 0, 2048));
    }
    for (lo, hi) in parts {
        b.boxed(lo, hi, conc, FeatureKind::MASS, far | PrimFlags::NONE);
    }
    if along == gate {
        let (lo, hi) = at(g0 - 2, g1 + 2, 118, 122);
        b.boxed(
            lo,
            hi,
            MaterialId::BEACON,
            FeatureKind::MISC,
            far | PrimFlags::NO_COLLIDE,
        );
    }
    let keep = if axis == 0 {
        volume([0, 0, c - 40], [4096, 2048, c + 40])
    } else {
        volume([c - 40, 0, 0], [c + 40, 2048, 4096])
    };
    Content {
        prims: b.out,
        keep_out: vec![keep],
    }
}

/// STG-06: the maze: 32 by 32 rooms of 64 m, walls of 4 m and 12 m tall on a deck, a spanning
/// tree of passages drawn by a depth-first walk from the keyed hash, at the middle of the region's
/// middle L1 cell.
fn labyrinth(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    if strange_of(seed, l0) != Some(Strange::Labyrinth) || l1 - l0 * 8 != I64Vec3::splat(4) {
        return Content::default();
    }
    const N: usize = 32;
    let mut rng = Rng::new(hash(seed.0, Level::L1, l1, salt::STRANGE_BUILD));
    // Passages: `open[i][j]` bits 1 (east) and 2 (south) between rooms.
    let mut open = [[0u8; N]; N];
    let mut seen = [[false; N]; N];
    let mut stack = vec![(0usize, 0usize)];
    seen[0][0] = true;
    while let Some(&(x, z)) = stack.last() {
        let mut moves = Vec::new();
        if x + 1 < N && !seen[x + 1][z] {
            moves.push((x + 1, z, 0));
        }
        if x > 0 && !seen[x - 1][z] {
            moves.push((x - 1, z, 1));
        }
        if z + 1 < N && !seen[x][z + 1] {
            moves.push((x, z + 1, 2));
        }
        if z > 0 && !seen[x][z - 1] {
            moves.push((x, z - 1, 3));
        }
        if moves.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, nz, d) = moves[rng.below(moves.len() as u64) as usize];
        match d {
            0 => open[x][z] |= 1,
            1 => open[nx][nz] |= 1,
            2 => open[x][z] |= 2,
            _ => open[nx][nz] |= 2,
        }
        seen[nx][nz] = true;
        stack.push((nx, nz));
    }
    let mut b = B { out: Vec::new() };
    let (x0, z0, y) = (2048 - 1024, 2048 - 1024, 2048);
    b.boxed(
        [x0, y - 2, z0],
        [x0 + 2048, y, z0 + 2048],
        MaterialId::CONCRETE,
        FeatureKind::DECK,
        PrimFlags::FAR_VISIBLE,
    );
    for i in 0..N {
        for j in 0..N {
            let (px, pz) = (x0 + i as i32 * 64, z0 + j as i32 * 64);
            // The wall on the east side and the south side unless a passage goes through it.
            if open[i][j] & 1 == 0 {
                b.boxed(
                    [px + 62, y, pz],
                    [px + 66, y + 12, pz + 66],
                    MaterialId::CONCRETE,
                    FeatureKind::MASS,
                    PrimFlags::NONE,
                );
            }
            if open[i][j] & 2 == 0 {
                b.boxed(
                    [px, y, pz + 62],
                    [px + 66, y + 12, pz + 66],
                    MaterialId::CONCRETE,
                    FeatureKind::MASS,
                    PrimFlags::NONE,
                );
            }
        }
    }
    // The outer walls on the north and west sides.
    b.boxed(
        [x0 - 2, y, z0 - 2],
        [x0 + 2050, y + 12, z0 + 2],
        MaterialId::CONCRETE,
        FeatureKind::MASS,
        PrimFlags::NONE,
    );
    b.boxed(
        [x0 - 2, y, z0 - 2],
        [x0 + 2, y + 12, z0 + 2050],
        MaterialId::CONCRETE,
        FeatureKind::MASS,
        PrimFlags::NONE,
    );
    Content {
        prims: b.out,
        keep_out: vec![volume(
            [x0 - 4, y - 4, z0 - 4],
            [x0 + 2056, y + 40, z0 + 2056],
        )],
    }
}

/// STG-01: a gallery stack in every L1 cell of a Library region: two rings of eight facets,
/// a floor every 16 m for 16 floors.
fn library(seed: WorldSeed, l1: I64Vec3) -> Content {
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    if strange_of(seed, l0) != Some(Strange::Library) {
        return Content::default();
    }
    let c = 2048;
    let clip = volume([0; 3], [4096; 3]);
    let mut out = Vec::new();
    for ring in 0..2 {
        for level in 0..16 {
            out.extend(facet_ring(
                &FacetRing {
                    axis: QVec3::new(c * M, (c - 128 + level * 16) * M, c * M),
                    radius: (120 + 60 * ring) * M,
                    thick: 24 * M,
                    height: 2 * M,
                    n: 8,
                    start: 0,
                    material: MaterialId::CONCRETE,
                    kind: FeatureKind::DECK,
                    flags: PrimFlags::FAR_VISIBLE,
                },
                clip,
            ));
        }
    }
    Content {
        prims: out,
        keep_out: vec![volume(
            [c - 240, c - 130, c - 240],
            [c + 240, c + 140, c + 240],
        )],
    }
}

/// The geometry of the strange regions in L1 cell `l1`.
pub fn strange_content(seed: WorldSeed, l1: I64Vec3) -> Content {
    let mut all = rampart(seed, l1);
    for c in [labyrinth(seed, l1), library(seed, l1)] {
        all.prims.extend(c.prims);
        all.keep_out.extend(c.keep_out);
    }
    let _: i32 = CELL;
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WorldSeed = WorldSeed(0);

    fn find(kind: Strange) -> I64Vec3 {
        for x in -40..40 {
            for z in -40..40 {
                let l0 = I64Vec3::new(x, 0, z);
                if strange_of(SEED, l0) == Some(kind) {
                    return l0;
                }
            }
        }
        panic!("{kind:?} not found");
    }

    #[test]
    fn strange_cells_occur_in_every_character() {
        let mut counts = std::collections::HashMap::new();
        let mut cells = 0;
        for x in -50..50 {
            for z in -50..50 {
                cells += 1;
                if let Some(s) = strange_of(SEED, I64Vec3::new(x, 1, z)) {
                    *counts.entry(s).or_insert(0usize) += 1;
                }
            }
        }
        for s in DRAWN {
            assert!(counts.get(&s).copied().unwrap_or(0) > 0, "{s:?}");
        }
        let total: usize = counts.values().sum();
        assert!(
            (cells / 25..cells / 8).contains(&total),
            "{total} of {cells}"
        );
        assert_eq!(
            strange_of(SEED, I64Vec3::new(FAR_LANDS, 0, 3)),
            Some(Strange::FarLands)
        );
        assert_ne!(
            strange_of(SEED, I64Vec3::new(FAR_LANDS - 1, 0, 3)),
            Some(Strange::FarLands)
        );
    }

    #[test]
    fn the_engine_rules_follow_the_region() {
        let slow = find(Strange::Slow);
        let p = [
            slow.x * 32768 + 100,
            slow.y * 32768 + 100,
            slow.z * 32768 + 100,
        ];
        assert_eq!(time_scale_at(SEED, p), (1, 10));
        assert!(time_scale_at(SEED, [p[0] + 32768 * 3, p[1], p[2]]).1 > 0);
        let silent = find(Strange::Silent);
        assert!(silent_at(
            SEED,
            [silent.x * 32768 + 5, silent.y * 32768, silent.z * 32768]
        ));
        assert!((fog_factor(SEED, find(Strange::Fog)) - 6.0).abs() < 1e-6);
        assert_eq!(
            rare_of(Strange::Inversion),
            Some(crate::rare::Rare::Inverted)
        );
        assert_eq!(
            rare_of(Strange::Miniature),
            Some(crate::rare::Rare::Dollhouse)
        );
    }

    /// STG-05: a field of doors fills every L2 cell of its region with frames.
    #[test]
    fn a_field_of_doors_has_doorways_everywhere() {
        let l0 = find(Strange::FieldOfDoors);
        let l2 = l0 * 64 + I64Vec3::new(5, 7, 9);
        assert_eq!(cell_pieces(SEED, l2).len(), 72);
        assert!(
            cell_pieces(SEED, (l0 + I64Vec3::new(1, 0, 0)) * 64).is_empty()
                || strange_of(SEED, l0 + I64Vec3::X) == Some(Strange::FieldOfDoors)
        );
    }

    /// STG-06: a maze with a way from the corner to every room: the passages are a spanning tree.
    #[test]
    fn the_labyrinth_is_a_maze_with_walls_and_a_floor() {
        let l0 = find(Strange::Labyrinth);
        let l1 = l0 * 8 + I64Vec3::splat(4);
        let c = labyrinth(SEED, l1);
        // 1 deck, two outer walls, and 2 · 32 · 32 inner walls minus the 1023 passages.
        let walls = c.prims.len() - 3;
        assert_eq!(
            walls,
            2 * 32 * 32 - 1023,
            "a spanning tree of 1024 rooms has 1023 passages"
        );
        assert_eq!(labyrinth(SEED, l1 + I64Vec3::X).prims.len(), 0);
    }

    /// STG-12, WON-24: a Rampart is a wall of a whole row, with one gate in eight cells.
    #[test]
    fn a_rampart_has_a_gate() {
        for y in -3..3 {
            for z in -60..60 {
                let Some(((a, b), gate)) = rampart_line(SEED, 0, (y, z)) else {
                    continue;
                };
                let base = I64Vec3::new(0, y * 8 + a, z * 8 + b);
                let gate_cell = rampart(SEED, base + I64Vec3::new(gate, 0, 0));
                let wall_cell = rampart(SEED, base + I64Vec3::new((gate + 1) % 8, 0, 0));
                assert_eq!(wall_cell.prims.len(), 1, "a plain wall");
                assert_eq!(
                    gate_cell.prims.len(),
                    4,
                    "two jambs, a lintel and a lit sill"
                );
                return;
            }
        }
        panic!("no rampart");
    }

    #[test]
    fn a_library_region_has_galleries_in_every_cell() {
        let l0 = find(Strange::Library);
        for d in [(1, 1, 1), (6, 3, 2)] {
            let c = library(SEED, l0 * 8 + I64Vec3::new(d.0, d.1, d.2));
            assert_eq!(c.prims.len(), 2 * 16 * 8);
        }
    }

    fn tissue_of(l0: I64Vec3, k: i64) -> crate::tissue::Tissue {
        let cache = crate::tissue::TissueCache::default();
        let l2 = l0 * 64 + I64Vec3::new(8 + k % 4, 8 + k / 4 % 4, 8 + k / 16 % 4);
        crate::tissue::tissue_cell(SEED, &cache, crate::cell::CellKey::new(Level::L2, l2))
    }

    /// STG-02, 03, 04, 10: the tissue of a strange region wears its character.
    #[test]
    fn the_tissue_wears_the_region() {
        let inv = find(Strange::Inversion);
        assert_eq!(tissue_of(inv, 0).rare, Some(crate::rare::Rare::Inverted));
        let mini = find(Strange::Miniature);
        assert_eq!(tissue_of(mini, 0).rare, Some(crate::rare::Rare::Dollhouse));
        // The Recursion: structures swallow older forms (mutation 15).
        let rec = find(Strange::Recursion);
        let swallowing = (0..8)
            .flat_map(|k| tissue_of(rec, k).structures)
            .filter(|s| s.mutations.contains(&15))
            .count();
        assert!(swallowing > 20, "{swallowing} swallowing structures");
        // The Grid: forms are all one cube of two slots (set pieces and hubs keep their own).
        let grid = find(Strange::Grid);
        let mut cubes = 0;
        for k in 0..8 {
            let t = tissue_of(grid, k);
            assert_eq!(t.strange, Some(Strange::Grid));
            cubes += t
                .structures
                .iter()
                .filter(|s| s.mutations.is_empty() && s.parts.len() == 1)
                .filter(|s| (0..3).all(|i| s.parts[0].hi[i] - s.parts[0].lo[i] == 2))
                .count();
        }
        assert!(cubes > 10, "{cubes} cubes");
    }
}
