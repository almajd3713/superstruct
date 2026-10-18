//! Streaming: which chunks should exist around the camera. Pure logic, no GPU and no threads.
//!
//! Structural L1 geometry (trunks, decks) is drawn at every distance, and L2 chunks add detail on
//! top of it nearby (`docs/spec/primitives.md` §5), so the bands overlap by design:
//!
//! | Band | Chunk | Within |
//! |---|---|---|
//! | Near | L2 cell, full | `l2_radius_m` |
//! | Mid | L2 cell, envelope masses only | `l2_mid_radius_m` |
//! | Near | L1 cell, full | `l1_full_radius_m` |
//! | Far | L1 cell, far-visible subset | `l1_far_radius_m` |

use std::collections::HashSet;

use glam::{DVec3, I64Vec3, Vec3};
use worldgen::cell::{CellKey, Level};
use worldgen::coord::WorldPos;

/// How much of a cell's output a chunk draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Detail {
    Full,
    /// The `FAR_VISIBLE` subset (`l1_far_primitives`); L1 only.
    Far,
    /// The `ENVELOPE` primitives (exterior masses) of an L2 cell; L2 only.
    Mid,
}

/// A chunk's identity: a generator cell and a detail choice. The same id always yields the same
/// mesh, so it is also the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkId {
    pub key: CellKey,
    pub detail: Detail,
}

impl ChunkId {
    pub const fn l2(coords: I64Vec3) -> Self {
        Self {
            key: CellKey::new(Level::L2, coords),
            detail: Detail::Full,
        }
    }

    pub const fn l2_mid(coords: I64Vec3) -> Self {
        Self {
            key: CellKey::new(Level::L2, coords),
            detail: Detail::Mid,
        }
    }

    pub const fn l1(coords: I64Vec3, detail: Detail) -> Self {
        Self {
            key: CellKey::new(Level::L1, coords),
            detail,
        }
    }

    pub fn band(&self) -> Band {
        match (self.key.level, self.detail) {
            (Level::L2, Detail::Mid) => Band::Mid,
            (Level::L2, _) => Band::Near,
            (_, Detail::Full) => Band::Mid,
            _ => Band::Far,
        }
    }

    fn sort_key(&self) -> (u8, i64, i64, i64, Detail) {
        let c = self.key.coords;
        (self.key.level as u8, c.x, c.y, c.z, self.detail)
    }
}

impl PartialOrd for ChunkId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ChunkId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Near,
    Mid,
    Far,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamConfig {
    pub l2_radius_m: f64,
    /// Beyond `l2_radius_m` and up to this, L2 cells draw their envelope masses only.
    pub l2_mid_radius_m: f64,
    pub l1_full_radius_m: f64,
    pub l1_far_radius_m: f64,
}

impl StreamConfig {
    /// Makes each band reach at least as far as the one inside it.
    #[must_use]
    pub fn ordered(mut self) -> Self {
        self.l2_mid_radius_m = self.l2_mid_radius_m.max(self.l2_radius_m);
        self.l1_far_radius_m = self.l1_far_radius_m.max(self.l1_full_radius_m);
        self
    }
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            l2_radius_m: 1024.0,
            l2_mid_radius_m: 3072.0,
            l1_full_radius_m: 4096.0,
            l1_far_radius_m: 51200.0,
        }
    }
}

/// A wanted chunk and its load priority (lower loads first).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wanted {
    pub id: ChunkId,
    pub priority: f32,
}

/// Distance in metres from `cam` to the nearest point of `key`'s cell; zero inside it.
fn distance_to_cell(cam: &WorldPos, key: CellKey) -> Option<f64> {
    let bounds = key.aabb_world().ok()?;
    let min = bounds.min.delta(cam);
    let size = bounds.size_m as f64;
    let outside = (min - DVec3::ZERO)
        .max(-min - DVec3::splat(size))
        .max(DVec3::ZERO);
    Some(outside.length())
}

/// Chunks wanted around `cam`, priority-ordered by distance biased towards `view_dir`
/// (cells ahead load before cells behind at the same distance). Output order is deterministic.
pub fn desired_set(cfg: &StreamConfig, cam: &WorldPos, view_dir: Vec3, out: &mut Vec<Wanted>) {
    desired_set_with_probes(cfg, cam, view_dir, &[], out);
}

/// A place the camera will be, from a known schedule (`docs/spec/trams.md` §7).
#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub pos: WorldPos,
    /// Seconds until the camera gets there.
    pub lead_s: f32,
}

/// L2 full chunks wanted within this distance of each probe, metres.
pub const PROBE_L2_RADIUS: f64 = 512.0;

/// [`desired_set`] plus the L2 full chunks around each probe, ranked by lead time first.
pub fn desired_set_with_probes(
    cfg: &StreamConfig,
    cam: &WorldPos,
    view_dir: Vec3,
    probes: &[Probe],
    out: &mut Vec<Wanted>,
) {
    out.clear();
    let view = view_dir.as_dvec3().normalize_or_zero();
    let mut add = |id: ChunkId, dist: f64, centre: DVec3| {
        let ahead = centre.try_normalize().map_or(0.0, |dir| dir.dot(view));
        // Behind costs up to 50% extra distance.
        let priority = dist * (1.0 + 0.25 * (1.0 - ahead));
        out.push(Wanted {
            id,
            priority: priority as f32,
        });
    };
    // Cells are visited by integer offset from the camera's cell, with distances computed from
    // the camera's position inside its own cell: no `WorldPos` arithmetic per cell, which keeps a
    // re-rank over tens of thousands of cells cheap.
    let around = |level: Level, radius: f64, f: &mut dyn FnMut(CellKey, f64, DVec3)| {
        let centre_key = CellKey::containing(level, cam);
        let Ok(bounds) = centre_key.aabb_world() else {
            return;
        };
        let size = bounds.size_m as f64;
        let local = -bounds.min.delta(cam);
        let n = (radius / level.size_m() as f64).ceil() as i64 + 1;
        let gap = |d: i64, l: f64| match d {
            0 => 0.0,
            d if d > 0 => d as f64 * size - l,
            d => l - (d + 1) as f64 * size,
        };
        for dz in -n..=n {
            for dy in -n..=n {
                for dx in -n..=n {
                    let key = CellKey::new(level, centre_key.coords + I64Vec3::new(dx, dy, dz));
                    if !key.in_domain() {
                        continue;
                    }
                    let d =
                        DVec3::new(gap(dx, local.x), gap(dy, local.y), gap(dz, local.z)).length();
                    if d <= radius {
                        let centre = DVec3::new(dx as f64, dy as f64, dz as f64) * size
                            + DVec3::splat(size * 0.5)
                            - local;
                        f(key, d, centre);
                    }
                }
            }
        }
    };
    around(
        Level::L2,
        cfg.l2_mid_radius_m.max(cfg.l2_radius_m),
        &mut |k, d, c| {
            let id = if d <= cfg.l2_radius_m {
                ChunkId::l2(k.coords)
            } else {
                ChunkId::l2_mid(k.coords)
            };
            add(id, d, c);
        },
    );
    around(Level::L1, cfg.l1_far_radius_m, &mut |k, d, c| {
        let detail = if d <= cfg.l1_full_radius_m {
            Detail::Full
        } else {
            Detail::Far
        };
        add(ChunkId::l1(k.coords, detail), d, c);
    });
    if !probes.is_empty() {
        add_probes(probes, out);
    }
    out.sort_by(|a, b| {
        a.priority
            .total_cmp(&b.priority)
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Adds the probes' L2 full chunks. A cell wanted full by a probe is not also wanted as mid.
fn add_probes(probes: &[Probe], out: &mut Vec<Wanted>) {
    let mut fulls: Vec<Wanted> = Vec::new();
    for probe in probes {
        let centre = CellKey::containing(Level::L2, &probe.pos).coords;
        let n = (PROBE_L2_RADIUS / Level::L2.size_m() as f64).ceil() as i64 + 1;
        for z in -n..=n {
            for y in -n..=n {
                for x in -n..=n {
                    let key = CellKey::new(Level::L2, centre + I64Vec3::new(x, y, z));
                    if let Some(d) =
                        distance_to_cell(&probe.pos, key).filter(|d| *d <= PROBE_L2_RADIUS)
                    {
                        fulls.push(Wanted {
                            id: ChunkId::l2(key.coords),
                            priority: 100.0 * probe.lead_s + d as f32,
                        });
                    }
                }
            }
        }
    }
    fulls.append(out);
    fulls.sort_by(|a, b| a.id.cmp(&b.id).then(a.priority.total_cmp(&b.priority)));
    fulls.dedup_by(|b, a| a.id == b.id);
    let full_cells: HashSet<I64Vec3> = fulls
        .iter()
        .filter(|w| w.id.key.level == Level::L2 && w.id.detail == Detail::Full)
        .map(|w| w.id.key.coords)
        .collect();
    fulls.retain(|w| {
        !(w.id.key.level == Level::L2
            && w.id.detail == Detail::Mid
            && full_cells.contains(&w.id.key.coords))
    });
    *out = fulls;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(cfg: &StreamConfig, cam: &WorldPos) -> Vec<Wanted> {
        let mut v = Vec::new();
        desired_set(cfg, cam, Vec3::NEG_Z, &mut v);
        v
    }

    fn ids(v: &[Wanted]) -> std::collections::BTreeSet<ChunkId> {
        v.iter().map(|w| w.id).collect()
    }

    fn at(sector: [i64; 3], local: [f32; 3]) -> WorldPos {
        WorldPos::new(I64Vec3::from_array(sector), Vec3::from_array(local)).unwrap()
    }

    #[test]
    fn camera_cell_is_wanted_first_at_every_level() {
        let cam = at([3, -2, 7], [100.0, 200.0, 300.0]);
        let v = set(&StreamConfig::default(), &cam);
        let l2 = CellKey::containing(Level::L2, &cam).coords;
        let l1 = CellKey::containing(Level::L1, &cam).coords;
        assert!(ids(&v).contains(&ChunkId::l2(l2)));
        assert!(ids(&v).contains(&ChunkId::l1(l1, Detail::Full)));
        assert!(v[0].priority.abs() < f32::EPSILON);
    }

    #[test]
    fn shells_have_the_configured_detail_and_radius() {
        let cfg = StreamConfig::default();
        let cam = at([0, 0, 0], [10.0, 10.0, 10.0]);
        for w in set(&cfg, &cam) {
            let d = distance_to_cell(&cam, w.id.key).unwrap();
            match w.id.band() {
                Band::Near => assert!(d <= cfg.l2_radius_m),
                Band::Mid if w.id.key.level == Level::L2 => {
                    assert!(d > cfg.l2_radius_m && d <= cfg.l2_mid_radius_m);
                }
                Band::Mid => assert!(d <= cfg.l1_full_radius_m),
                Band::Far => assert!(d > cfg.l1_full_radius_m && d <= cfg.l1_far_radius_m),
            }
        }
    }

    #[test]
    fn crossing_a_cell_boundary_swaps_exactly_the_edge_cells() {
        let cfg = StreamConfig::default();
        // 511.5 m and 512.5 m along x: one L2 cell apart.
        let a = ids(&set(&cfg, &at([0, 0, 0], [511.5, 256.0, 256.0])));
        let b = ids(&set(&cfg, &at([0, 0, 0], [512.5, 256.0, 256.0])));
        assert!(a.intersection(&b).count() > a.len() / 2);
        // A cell whose detail changes (mid to full) swaps chunk ids without leaving, so compare
        // cells: everything that left is on the -x side, everything that arrived on the +x side.
        let cells = |s: &std::collections::BTreeSet<ChunkId>| {
            s.iter()
                .map(|id| id.key)
                .collect::<std::collections::HashSet<_>>()
        };
        let (ca, cb) = (cells(&a), cells(&b));
        let removed: Vec<_> = ca.difference(&cb).collect();
        let added: Vec<_> = cb.difference(&ca).collect();
        assert!(!added.is_empty() || !removed.is_empty());
        assert!(removed.iter().all(|k| k.coords.x <= 0));
        assert!(added.iter().all(|k| k.coords.x >= 1));
        // Walking back restores the original set.
        assert_eq!(a, ids(&set(&cfg, &at([0, 0, 0], [511.5, 256.0, 256.0]))));
    }

    #[test]
    fn l1_detail_changes_with_distance() {
        let cfg = StreamConfig::default();
        let a = ids(&set(&cfg, &at([0, 0, 0], [10.0, 10.0, 10.0])));
        let b = ids(&set(&cfg, &at([8, 0, 0], [10.0, 10.0, 10.0])));
        let l1 = |s: &std::collections::BTreeSet<ChunkId>, c: [i64; 3], d| {
            s.contains(&ChunkId::l1(I64Vec3::from_array(c), d))
        };
        assert!(l1(&a, [0, 0, 0], Detail::Full));
        assert!(l1(&a, [2, 0, 0], Detail::Far));
        assert!(!l1(&a, [2, 0, 0], Detail::Full));
        // After moving 8 km along x, the cell at x = 2 is within the full radius.
        assert!(l1(&b, [2, 0, 0], Detail::Full));
        assert!(!l1(&b, [0, 0, 0], Detail::Full));
    }

    #[test]
    fn cells_ahead_load_before_cells_behind() {
        let cfg = StreamConfig::default();
        let cam = at([0, 0, 0], [256.0, 256.0, 256.0]);
        let mut v = Vec::new();
        desired_set(&cfg, &cam, Vec3::X, &mut v);
        let rank = |x: i64| {
            v.iter()
                .position(|w| w.id == ChunkId::l2(I64Vec3::new(x, 0, 0)))
                .unwrap()
        };
        assert!(rank(1) < rank(-1));
    }

    #[test]
    fn works_at_sector_two_pow_forty() {
        let cam = at([1 << 40, -(1 << 40), 5], [1.0, 2.0, 3.0]);
        let v = set(&StreamConfig::default(), &cam);
        assert!(v.len() > 50);
        assert!(v.iter().all(|w| w.id.key.in_domain()));
    }

    #[test]
    fn probes_add_l2_full_chunks_far_ahead_and_drop_their_mid_variant() {
        let cfg = StreamConfig::default();
        let cam = at([0, 0, 0], [512.0, 512.0, 512.0]);
        let ahead = at([3, 0, 0], [512.0, 512.0, 512.0]);
        let key = CellKey::containing(Level::L2, &ahead).coords;
        let plain = set(&cfg, &cam);
        assert!(!plain.iter().any(|w| w.id == ChunkId::l2(key)));
        let mut with = Vec::new();
        let probe = Probe {
            pos: ahead,
            lead_s: 30.0,
        };
        desired_set_with_probes(&cfg, &cam, Vec3::X, &[probe], &mut with);
        assert!(with.iter().any(|w| w.id == ChunkId::l2(key)));
        assert!(!with.iter().any(|w| w.id == ChunkId::l2_mid(key)));
        let mut ids: Vec<_> = with.iter().map(|w| w.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate ids");
        // Same result twice.
        let mut again = Vec::new();
        desired_set_with_probes(&cfg, &cam, Vec3::X, &[probe], &mut again);
        assert_eq!(
            with.iter().map(|w| w.id).collect::<Vec<_>>(),
            again.iter().map(|w| w.id).collect::<Vec<_>>()
        );
    }
}
