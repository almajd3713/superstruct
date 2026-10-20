//! The Web L2 cell (version 11): the tissue of a cell (`tissue::tissue_cell`: structures, skin,
//! paths, thresholds, payoffs) assembled into one `CellOutput`. Spec: `docs/spec/levels.md`
//! §5W.
//!
//! Output order: structures (hubs, set pieces, local places among them) in the order they were
//! placed, the skin's cladding, path pieces, thresholds, payoffs. Ports are the L2 face ports of
//! the L1 skeleton (`l2_port::cell_ports_in`). The clear boxes of bored passages and of
//! thresholds are `KeepOut` constraints: air that L3 must leave open (PW.4) and that the
//! parent's solid is cut for (PW.3.21b).

use crate::carve::{CarveStats, carve, translated};
use crate::cell::{CellKey, Level, floor_div_pow2, floor_mod_pow2};
use crate::l2_port::{L1Cache, cell_ports_in};
use crate::prim::{CellMeta, CellOutput, ChildConstraint, PrimFlags, Primitive, Q, QAabb};
use crate::seed::WorldSeed;
use crate::tissue::tissue_cell;
use crate::tissue::tissue_routes;

/// Everything the Web tissue emits for L2 cell `cell`, reading parents through `cache`.
pub fn l2_cell_web(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> CellOutput {
    l2_cell_web_edges(seed, cache, cell).0
}

/// `l2_cell_web`, and what the railing pass found at the cell's edges.
pub fn l2_cell_web_edges(
    seed: WorldSeed,
    cache: &L1Cache,
    cell: CellKey,
) -> (CellOutput, crate::edges::EdgeStats) {
    debug_assert_eq!(cell.level, Level::L2);
    let t = tissue_cell(seed, cache.tissue(), cell);
    let mut structures: Vec<Primitive> = Vec::new();
    // The parts that L3 fills are hollow shells with their doors cut.
    let masses = crate::l3_web::mass_specs(&t);
    for (si, s) in t.structures.iter().enumerate() {
        structures.extend(s.primitives_with(&t.lattice, &|part| {
            masses
                .iter()
                .find(|m| m.structure == si && m.part == part)
                .map(|m| m.entrances.clone())
        }));
    }
    // The doorways and passages are cut out of the structures and the skin's cladding; the
    // pieces that line them stay.
    let bores: Vec<Primitive> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .chain(&t.paths.corridors)
        .copied()
        .collect();
    structures.extend(t.skin.cladding.iter().copied());
    let (mut primitives, _) = carve(structures, &bores, [0; 3]);
    primitives.extend(t.paths.prims.iter().copied());
    primitives.extend(t.thresholds.prims.iter().copied());
    primitives.extend(t.payoffs.prims.iter().copied());
    drop_in_tram_volumes(seed, cell, &mut primitives);
    // Two identical pieces (the shared edge of two frames) would z-fight, and no offset can
    // separate them.
    let mut seen = std::collections::HashSet::new();
    primitives.retain(|p| seen.insert((p.shape, p.material, p.kind, p.flags)));
    let l1 = CellKey::new(Level::L1, cell.coords.map(|v| floor_div_pow2(v, 3)));
    let at = cell
        .coords
        .to_array()
        .map(|v| -Q::try_from(floor_mod_pow2(v, 3)).unwrap_or(0) * L2_Q);
    // The parent's pieces that reach the cell and 64 m around it, moved into its coordinates
    // (tested before the move: most of an L1 cell is far away).
    let reach = 64 * 16;
    let near = |p: &&Primitive| {
        let b = p.aabb();
        let (lo, hi) = (
            [b.min.x + at[0], b.min.y + at[1], b.min.z + at[2]],
            [b.max.x + at[0], b.max.y + at[1], b.max.z + at[2]],
        );
        (0..3).all(|k| hi[k] > -reach && lo[k] < L2_Q + reach)
    };
    let parent: Vec<Primitive> = cache
        .get(seed, l1)
        .primitives
        .iter()
        .filter(near)
        .map(|p| translated(p, at))
        .collect();
    let clear: Vec<Primitive> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .copied()
        .collect();
    let (rails, stats) = {
        let world = crate::edges::World::new(&primitives, &parent, &clear);
        crate::edges::guard_edges(seed, cell, &primitives, &world)
    };
    primitives.extend(rails);
    let mut primitives = crate::edges::cut_rails(primitives, &ways_of(&t));
    let mut seen = std::collections::HashSet::new();
    primitives.retain(|p| seen.insert((p.shape, p.material, p.kind, p.flags)));
    let child_constraints = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .map(|p| ChildConstraint::KeepOut { aabb: p.aabb() })
        .collect();
    let out = CellOutput {
        key: cell,
        primitives,
        ports: cell_ports_in(seed, cache, cell),
        child_constraints,
        meta: CellMeta::default(),
    };
    (out, stats)
}

/// A tram line's volume is clear air (`tissue::drop_tram_slots` keeps slots out of it), but a
/// turned structure reaches past its slots: whatever still meets a volume is dropped.
fn drop_in_tram_volumes(seed: WorldSeed, cell: CellKey, prims: &mut Vec<Primitive>) {
    const L2: Q = 512 * 16;
    let l1 = CellKey::new(Level::L1, cell.coords.map(|v| floor_div_pow2(v, 3)));
    let o = cell
        .coords
        .to_array()
        .map(|v| Q::try_from(floor_mod_pow2(v, 3)).unwrap_or(0) * L2);
    let volumes: Vec<QAabb> = crate::tram::envelopes(seed, l1);
    if volumes.is_empty() {
        return;
    }
    prims.retain(|p| {
        if p.flags.contains(PrimFlags::NO_COLLIDE) {
            return true;
        }
        let b = p.aabb();
        !volumes.iter().any(|e| {
            b.min.x + o[0] < e.max.x
                && b.max.x + o[0] > e.min.x
                && b.min.y + o[1] < e.max.y
                && b.max.y + o[1] > e.min.y
                && b.min.z + o[2] < e.max.z
                && b.max.z + o[2] > e.min.z
        })
    });
}

/// The ways on of a cell for the edge pass (PW.4.11): every leg and rise, and every doorway.
fn ways_of(t: &crate::tissue::Tissue) -> Vec<crate::edges::Way> {
    let mut ways = Vec::new();
    for w in &t.paths.walks {
        // A rise is sampled every half metre; a way every 1.5 m is enough (a helix bends little
        // over that) and keeps the edge pass fast.
        let mut points: Vec<glam::DVec3> = w.points.iter().step_by(3).copied().collect();
        if let (Some(last), Some(&end)) = (points.last(), w.points.last())
            && *last != end
        {
            points.push(end);
        }
        for pair in points.windows(2) {
            ways.push(crate::edges::Way {
                a: pair[0],
                b: pair[1],
                half: w.width / 2.0,
            });
        }
    }
    ways.extend(
        crate::l3_web::doors_of(t)
            .into_iter()
            .map(|(a, b)| crate::edges::Way { a, b, half: 2.0 }),
    );
    ways
}

const L2_Q: Q = 512 * 16;

/// The bored passages of L2 cell `cell`, as clear boxes in its coordinates: the bores of its
/// bored segments. They follow from the routes alone (the routes do not read the structures),
/// so this skips the growth. (A threshold at a bored end sits in the same corridor.)
pub fn bored_cuts(seed: WorldSeed, cache: &L1Cache, cell: CellKey) -> Vec<Primitive> {
    let t = tissue_routes(seed, cache.tissue(), cell);
    if !t
        .segments
        .iter()
        .any(|s| s.surround == crate::tissue::Surround::Bored)
    {
        return Vec::new();
    }
    crate::paths::bored_bores(seed, &t)
}

/// The primitives of L1 cell `cell` with the bored passages of its 512 L2 cells cut out of
/// them (`levels.md` §5W.27). Costs the routes of every L2 cell of the cell.
pub fn l1_primitives_carved(
    seed: WorldSeed,
    cache: &L1Cache,
    cell: CellKey,
) -> (Vec<Primitive>, CarveStats) {
    debug_assert_eq!(cell.level, Level::L1);
    let prims = cache.get(seed, cell).primitives.clone();
    let mut cuts: Vec<Primitive> = Vec::new();
    for i in 0..512i64 {
        let local = [i % 8, i / 8 % 8, i / 64];
        let l2 = CellKey::new(
            Level::L2,
            cell.coords * 8 + glam::I64Vec3::new(local[0], local[1], local[2]),
        );
        let d = local.map(|v| Q::try_from(v).unwrap_or(0) * L2_Q);
        cuts.extend(bored_cuts(seed, cache, l2).iter().map(|c| translated(c, d)));
    }
    carve(prims, &cuts, [0; 3])
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::default_trait_access,
    clippy::too_many_lines
)]
mod tests {
    use super::*;
    use crate::seed::WorldSeed;
    use glam::I64Vec3;

    fn cells(n: i64) -> impl Iterator<Item = CellKey> {
        (0..n).map(|k| {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            )
        })
    }

    /// The cuts known from the routes alone are among the bores of the full tissue, so cutting
    /// the L1 solid before the cell has grown cuts what the cell's own pieces assume is cut.
    #[test]
    fn the_cuts_follow_from_the_routes_alone() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let mut with_cuts = 0;
        for key in cells(600) {
            let cuts = bored_cuts(seed, &cache, key);
            if cuts.is_empty() {
                continue;
            }
            with_cuts += 1;
            let t = tissue_cell(seed, cache.tissue(), key);
            for c in &cuts {
                assert!(
                    t.paths.bores.contains(c),
                    "{key:?}: a cut that the grown cell does not have: {c:?}"
                );
            }
        }
        assert!(
            with_cuts > 20,
            "only {with_cuts} cells with a bored passage"
        );
    }

    /// Cutting an L1 cell: the passages leave no solid in them where every piece could be cut,
    /// and the numbers of pieces cut and left whole are reported.
    #[test]
    fn an_l1_cell_is_cut_open() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let (mut cut, mut stuck, mut cells_done) = (0, 0, 0);
        let t0 = std::time::Instant::now();
        for k in 0..6i64 {
            let cell = CellKey::new(Level::L1, I64Vec3::new(k % 3 - 1, k / 3, k % 2));
            let (prims, stats) = l1_primitives_carved(seed, &cache, cell);
            let plain = cache.get(seed, cell).primitives.len();
            eprintln!("{cell:?}: {plain} pieces -> {}, {stats:?}", prims.len());
            cut += stats.cut;
            stuck += stats.unsupported;
            cells_done += 1;
        }
        eprintln!(
            "{cells_done} L1 cells in {:?}: {cut} pieces cut, {stuck} could not be",
            t0.elapsed()
        );
        assert!(cut > 0, "nothing was cut in six L1 cells");
    }

    /// Pieces by 64 m bucket, so a probe tests only the pieces near it.
    struct Probe<'a> {
        prims: &'a [Primitive],
        grid: std::collections::HashMap<[i32; 3], Vec<u32>>,
    }

    impl<'a> Probe<'a> {
        fn new(prims: &'a [Primitive]) -> Self {
            const B: i32 = 64 * 16;
            let mut grid: std::collections::HashMap<[i32; 3], Vec<u32>> = Default::default();
            for (i, p) in prims.iter().enumerate() {
                if p.flags.contains(crate::prim::PrimFlags::NO_COLLIDE) {
                    continue;
                }
                let b = p.aabb();
                for x in b.min.x.div_euclid(B)..=(b.max.x - 1).div_euclid(B) {
                    for y in b.min.y.div_euclid(B)..=(b.max.y - 1).div_euclid(B) {
                        for z in b.min.z.div_euclid(B)..=(b.max.z - 1).div_euclid(B) {
                            grid.entry([x, y, z]).or_default().push(i as u32);
                        }
                    }
                }
            }
            Self { prims, grid }
        }

        fn hit(&self, p: glam::Vec3) -> Option<crate::prim::FeatureKind> {
            let c =
                [p.x, p.y, p.z].map(|v| (v * 16.0).floor() as i32 / (64 * 16) - i32::from(v < 0.0));
            self.grid
                .get(&c)?
                .iter()
                .map(|&i| &self.prims[i as usize])
                .find(|q| q.shape.contains(p))
                .map(|q| q.kind)
        }
    }

    /// What walking the horizontal level segments of an L1 cell finds along their centre lines.
    #[derive(Debug, Default)]
    struct Walk {
        samples: usize,
        no_floor: usize,
        /// Blocked (0.3, 0.9 or 1.1 m above the floor) by a piece of the L1 cell.
        by_l1: usize,
        /// By a structure of the L2 cell (a mass).
        by_structure: usize,
        /// By a rail or wall (the side pieces of a path).
        by_rail: usize,
        /// By any other piece of the L2 cell (decks, stairs, ribs).
        by_other: usize,
        /// Per path type: samples, without floor, blocked.
        by_id: std::collections::BTreeMap<u8, [usize; 3]>,
        kinds: std::collections::BTreeMap<(u8, u8), usize>,
    }

    impl Walk {
        fn blocked(&self) -> usize {
            self.by_l1 + self.by_structure + self.by_rail + self.by_other
        }
    }

    /// Walks the horizontal segments of the 512 L2 cells of `cell` that are bored (`bored`) or
    /// open, in the L1 solid with the passages cut and the L2 pieces together.
    #[allow(clippy::cast_possible_truncation)]
    fn walk_l1(seed: WorldSeed, cache: &L1Cache, cell: CellKey, bored: bool) -> Walk {
        use crate::prim::FeatureKind;
        let (l1_prims, _) = l1_primitives_carved(seed, cache, cell);
        let l1_probe = Probe::new(&l1_prims);
        let mut w = Walk::default();
        for i in (0..512i64).step_by(if bored { 1 } else { 5 }) {
            let local = [i % 8, i / 8 % 8, i / 64];
            let key = CellKey::new(
                Level::L2,
                cell.coords * 8 + I64Vec3::new(local[0], local[1], local[2]),
            );
            let routes = tissue_routes(seed, cache.tissue(), key);
            let segs: Vec<_> = routes
                .segments
                .iter()
                .filter(|s| {
                    (s.surround == crate::tissue::Surround::Bored) == bored
                        && s.surround != crate::tissue::Surround::Rise
                        && s.slots.len() >= 2
                })
                .collect();
            if segs.is_empty() {
                continue;
            }
            let t = tissue_cell(seed, cache.tissue(), key);
            let l2_prims = l2_cell_web(seed, cache, key).primitives;
            let l2 = Probe::new(&l2_prims);
            let up = routes.lattice.frame.axes()[1];
            // Headroom is measured plumb (PW.4.10): a walker stands upright on a tilted floor.
            let at_l2 = |c: glam::DVec3, dy: f64| {
                let p = c + glam::DVec3::Y * dy;
                glam::Vec3::new(p.x as f32, p.y as f32, p.z as f32)
            };
            let o = local.map(|v| v as f32 * 512.0);
            for (si, seg) in t.segments.iter().enumerate() {
                if !segs.iter().any(|s| s.slots == seg.slots) || t.engulfed.contains(&(si as u16)) {
                    continue;
                }
                let pid = t
                    .paths
                    .chosen
                    .iter()
                    .find(|c| c.0 as usize == si)
                    .map_or(0, |c| c.1);
                for win in seg.slots.windows(2) {
                    let level = t.lattice.coords(win[0] as usize)[1]
                        == t.lattice.coords(win[1] as usize)[1];
                    if !level {
                        continue;
                    }
                    let (a, b) = (
                        t.lattice.centre(win[0] as usize),
                        t.lattice.centre(win[1] as usize),
                    );
                    let half = t.lattice.pitch as f64 / 2.0;
                    for step in 1..16 {
                        let c = a + (b - a) * (f64::from(step) / 16.0) - up * half;
                        w.samples += 1;
                        w.by_id.entry(pid).or_default()[0] += 1;
                        if l2.hit(at_l2(c, -0.1)).is_none() {
                            w.no_floor += 1;
                            w.by_id.entry(pid).or_default()[1] += 1;
                        }
                        for dy in [0.3, 0.9, 1.1] {
                            let p = at_l2(c, dy);
                            let pl1 = glam::Vec3::new(p.x + o[0], p.y + o[1], p.z + o[2]);
                            if l1_probe.hit(pl1).is_some() {
                                w.by_l1 += 1;
                                break;
                            }
                            if let Some(kind) = l2.hit(p) {
                                w.by_id.entry(pid).or_default()[2] += 1;
                                *w.kinds.entry((pid, kind.0)).or_default() += 1;
                                match kind {
                                    FeatureKind::MASS => w.by_structure += 1,
                                    FeatureKind::RAILING => w.by_rail += 1,
                                    _ => w.by_other += 1,
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        w
    }

    /// A walker can follow a bored passage: along its centre line there is a floor (the lining
    /// of the L2 cell) and nothing solid from 0.3 m to 1.1 m above it (a crawl duct is 1.3 m
    /// clear), in the L1 solid with the passages cut and in the L2 pieces together. A catwalk
    /// deck has gaps 0.2 m wide (NEAR-04), so a few samples have no floor.
    #[test]
    fn a_bored_passage_can_be_walked() {
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let w = walk_l1(
            seed,
            &cache,
            CellKey::new(Level::L1, I64Vec3::new(-1, 0, 0)),
            true,
        );
        eprintln!("bored: {w:?}");
        assert!(w.samples > 1000);
        assert!(w.by_l1 == 0, "{} samples blocked by the L1 solid", w.by_l1);
        assert!(w.no_floor * 12 <= w.samples, "{w:?}");
        assert!(w.blocked() * 100 <= w.samples, "{w:?}");
    }

    /// The same for the open level segments (catwalks, bridges, avenues): what blocks them.
    #[test]
    fn an_open_segment_can_be_walked() {
        use crate::prim::FeatureKind;
        let seed = WorldSeed(0x5eed);
        let cache = L1Cache::default();
        let w = walk_l1(
            seed,
            &cache,
            CellKey::new(Level::L1, I64Vec3::new(-1, 0, 0)),
            false,
        );
        eprintln!("open: {w:?}");
        assert!(w.samples > 10_000);
        // The centre line of PH-04, 05, 19 and 20 leaves their deck (the sampler's limit), and
        // their edge rails stand on it: those are not counted.
        let rails: usize = w
            .kinds
            .iter()
            .filter(|((id, k), _)| *k == FeatureKind::RAILING.0 && ![4, 5, 19, 20].contains(id))
            .map(|(_, n)| n)
            .sum();
        let artefact = w.by_rail - rails;
        assert!(
            (w.blocked() - artefact) * 500 <= w.samples,
            "{} samples blocked: {w:?}",
            w.blocked() - artefact
        );
        assert!(
            rails * 2500 <= w.samples,
            "{rails} samples blocked by rails: {w:?}"
        );
        // Where a leg meets a stair the stair is turned so that the leg comes in along its
        // corridor or from its free side (PW.3.12b).
        let stairs: usize = w
            .kinds
            .iter()
            .filter(|((_, k), _)| *k == FeatureKind::STAIR_TOWER.0 || *k == FeatureKind::RAMP.0)
            .map(|(_, n)| n)
            .sum();
        assert!(
            stairs * 2000 <= w.samples,
            "{stairs} samples blocked by stairs: {w:?}"
        );
    }
}
