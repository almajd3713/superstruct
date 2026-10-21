//! Ladder types LAD-01..13 as vertical path types (ids 41 to 53; PW.5.3, PW.5.4, spec
//! `docs/spec/ladders.md`). A ladder is built in the plan of its column like the stairs: the
//! climbing corridor stands at `u` 0.3 to 1.2 with its rungs on its +u side, a deck for the foot
//! and the top beside it at `u` -2 to 0.3, so the route passes from the column centre through the
//! corridor and back to the column centre. A climb of more than 24 m is a chain of ladders with a
//! rest platform between them (CLB-06), the next ladder 1.5 m to the side.

use super::{M, S};
use crate::prim::{FeatureKind, MaterialId};

/// Ids of the ladder families: `40 + n` for LAD-`n`. LAD-14 (the gravity-gate ladder) and LAD-15
/// (the ladder across a lull) wait for the gravity wells (PW.6.3, PW.6.8).
pub const LADDER_IDS: [u8; 13] = [41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53];

/// The longest run of one ladder: the rest platform comes before this (CLB-06 says 30 m).
pub const MAX_RUN: f64 = 24.0;

const METAL: MaterialId = MaterialId::METAL;

/// Which family an id belongs to, as the LAD number, or `None`.
pub fn lad_number(id: u8) -> Option<u8> {
    (LADDER_IDS.contains(&id)).then(|| id - 40)
}

/// The side code of the rungs (+u) in the lattice frame.
fn side(m: &M<'_, '_>) -> u16 {
    use crate::prim::Dir4;
    match m.dir4((1.0, 0.0)) {
        Dir4::NegZ => 0,
        Dir4::PosX => 1,
        Dir4::PosZ => 2,
        Dir4::NegX => 3,
    }
}

/// One ladder from `y0` to `y1` at lateral offset `vo`, in the style of LAD-`n`.
#[allow(clippy::too_many_lines)] // the families side by side
fn run(m: &mut M<'_, '_>, n: u8, vo: f64, y0: f64, y1: f64) {
    let v = |a: f64, b: f64| -> S { (vo + a, vo + b) };
    let climb: S = (y0, y1);
    // The climbing corridor, not drawn and not solid.
    let code = side(m);
    m.bx_free(
        (0.3, 1.2),
        v(-0.4, 0.4),
        climb,
        MaterialId(code),
        FeatureKind::LADDER,
    );
    // What the rungs are on.
    match n {
        // A chain ladder hangs free; a pipe ladder has the pipe; the rest have a wall.
        7 => {}
        6 => m.slab(
            (1.25, 1.85),
            v(-0.3, 0.3),
            climb,
            MaterialId::PIPE,
            FeatureKind::PIPE,
        ),
        8 => m.slab(
            (1.3, 2.5),
            v(-1.2, 1.2),
            climb,
            MaterialId::CONCRETE,
            FeatureKind::MASS,
        ),
        13 => m.slab(
            (1.25, 2.45),
            v(-0.6, 0.6),
            climb,
            MaterialId::CONCRETE,
            FeatureKind::MASS,
        ),
        _ => m.slab(
            (1.3, 1.5),
            v(-0.7, 0.7),
            climb,
            MaterialId::CONCRETE,
            FeatureKind::MASS,
        ),
    }
    let rung_x: S = (1.2, 1.26);
    // Rails or chains, and rungs or staples.
    let rails = !matches!(n, 5);
    if rails {
        let w = if n == 7 { 0.02 } else { 0.03 };
        for lat in [-0.25, 0.25] {
            m.bx_free(rung_x, v(lat - w, lat + w), climb, METAL, FeatureKind::MISC);
        }
    }
    let pitch = if n == 5 { 0.35 } else { 0.3 };
    let mut y = y0 + 0.25;
    let mut k = 0;
    while y < y1 - 0.1 {
        if n == 5 {
            // Staples: one side then the other, set in the wall.
            let lat = if k % 2 == 0 { -0.18 } else { 0.18 };
            m.bx_free(
                (1.24, 1.32),
                v(lat - 0.05, lat + 0.05),
                (y, y + 0.03),
                METAL,
                FeatureKind::MISC,
            );
        } else if n == 13 {
            // Helical rungs round the drum: each one 20 degrees on, drawn as a short bar on a
            // slanted line across the drum's face.
            let lat = (f64::from(k % 18) - 9.0) / 9.0 * 0.45;
            m.bx_free(
                rung_x,
                v(lat - 0.06, lat + 0.06),
                (y, y + 0.03),
                METAL,
                FeatureKind::MISC,
            );
            m.bx_free(
                rung_x,
                v(-0.25, 0.25),
                (y, y + 0.02),
                METAL,
                FeatureKind::MISC,
            );
        } else {
            m.bx_free(
                rung_x,
                v(-0.25, 0.25),
                (y, y + 0.03),
                METAL,
                FeatureKind::MISC,
            );
        }
        y += pitch;
        k += 1;
    }
    if n == 2 {
        // A cage: hoops every metre from 2.2 m up, on the three closed sides, and straps.
        let mut y = y0 + 2.2;
        while y < y1 - 0.2 {
            m.bx_free(
                (0.3, 1.2),
                v(-0.5, -0.46),
                (y, y + 0.03),
                METAL,
                FeatureKind::MISC,
            );
            m.bx_free(
                (0.3, 1.2),
                v(0.46, 0.5),
                (y, y + 0.03),
                METAL,
                FeatureKind::MISC,
            );
            y += 1.0;
        }
        for lat in [-0.5, 0.5] {
            m.bx_free(
                (0.3, 0.34),
                v(lat - 0.02, lat + 0.02),
                (y0 + 2.2, y1),
                METAL,
                FeatureKind::MISC,
            );
        }
    }
    let mut y = y0;
    while y <= y1 + 1e-9 {
        m.sample((0.75, vo), y);
        y += 0.5;
    }
    m.sample((0.75, vo), y1);
}

/// A platform beside the corridor at `top`, with a rail on the three sides that are not the
/// way on, over `v0..v1`.
fn platform(m: &mut M<'_, '_>, top: f64, v0: f64, v1: f64) {
    m.deck((-2.0, 0.3), (v0, v1), top);
    for edge in [v0, v1] {
        m.slab(
            (-2.0, 0.3),
            (edge - 0.03, edge + 0.03),
            (top, top + 1.1),
            METAL,
            FeatureKind::RAILING,
        );
    }
    m.slab(
        (-2.03, -1.97),
        (v0, v1),
        (top, top + 1.1),
        METAL,
        FeatureKind::RAILING,
    );
}

/// Builds LAD-`n` (`id` 40 + `n`) up the column from its floor to its top.
pub(super) fn build(m: &mut M<'_, '_>, id: u8, total: f64) {
    let Some(n) = lad_number(id) else {
        return;
    };
    let y_foot = m.col.y0;
    let y_top = y_foot + total;
    // The runs: a ladder well has one every storey, a ladder chain short ones, the rest are as
    // long as they may be, and every break is a rest platform.
    let step = match n {
        3 | 12 => 4.0,
        10 => 3.5,
        _ => MAX_RUN,
    };
    m.sample((0.0, 0.0), y_foot);
    platform(m, y_foot, -1.5, 1.5);
    let mut y = y_foot;
    let mut vo = 0.0;
    let mut v_far = 1.5f64;
    let mut v_near = -1.5f64;
    let mut count = 0;
    while y < y_top - 1e-6 {
        let h = (y_top - y).min(step);
        run(m, n, vo, y, y + h);
        let top = y + h;
        count += 1;
        if n == 3 && top < y_top - 1e-6 {
            // The deck of the storey, with the corridor left open through it.
            m.deck((-0.6, 0.3), (vo - 1.5, vo + 1.5), top);
            m.deck((1.2, 1.5), (vo - 1.5, vo + 1.5), top);
            m.deck((0.3, 1.2), (vo - 1.5, vo - 0.4), top);
            m.deck((0.3, 1.2), (vo + 0.4, vo + 1.5), top);
        } else {
            // A rest platform, and the next ladder 1.5 m to the other side.
            let next = if count % 2 == 1 { 1.5 } else { 0.0 };
            let (a, b) = (vo.min(next) - 1.5, vo.max(next) + 1.5);
            platform(m, top, a, b);
            v_far = v_far.max(b);
            v_near = v_near.min(a);
            vo = next;
        }
        y = top;
    }
    let _ = (v_far, v_near);
    // From the last platform back to the column centre.
    m.sample((0.0, 0.0), y_top);
    match n {
        // A hanging tower's roof hatch: a slab over the top with the corridor left open.
        11 => {
            m.slab(
                (1.3, 3.5),
                (-1.5, 1.5),
                (y_top - 0.3, y_top),
                MaterialId::CONCRETE,
                FeatureKind::MASS,
            );
        }
        // A façade with balconies every 4 m beside the ladder.
        12 => {
            let mut b = y_foot + 4.0;
            while b < y_top {
                m.deck((1.5, 3.5), (-1.5, 1.5), b);
                b += 4.0;
            }
        }
        // A dead lift: its frame and a cab stuck half way.
        9 => {
            for (u, v) in [(1.6, -1.5), (1.6, 1.5), (3.6, -1.5), (3.6, 1.5)] {
                m.slab(
                    (u - 0.1, u + 0.1),
                    (v - 0.1, v + 0.1),
                    (y_foot, y_top),
                    METAL,
                    FeatureKind::MASS,
                );
            }
            let mid = y_foot + total / 2.0;
            m.slab(
                (1.8, 3.4),
                (-1.3, 1.3),
                (mid, mid + 2.4),
                MaterialId::CONCRETE,
                FeatureKind::MASS,
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::tissue::Lattice;

    fn solid_at(prims: &[Primitive], p: glam::Vec3) -> bool {
        prims
            .iter()
            .any(|q| !q.flags.contains(PrimFlags::NO_COLLIDE) && q.shape.contains(p))
    }

    /// Every ladder family, at several heights: corridors of at most 24 m with a rest platform
    /// between, clear of solids, a route that starts and ends on a deck at the column centre.
    #[test]
    fn every_ladder_family_is_a_climbable_chain_with_rests() {
        let lat = Lattice::new(Frame::Id);
        for id in LADDER_IDS {
            for height in [8.0, 24.0, 32.0, 64.0, 128.0] {
                let p = VParams {
                    w: 3.0,
                    swap: false,
                    fx: 1.0,
                    fz: 1.0,
                    k: 0.5,
                    side: 1.0,
                };
                let col = Column {
                    cx: 256.0,
                    cz: 256.0,
                    y0: 100.0,
                    height,
                };
                let mut e = Emitter::new(&lat);
                let r = build_rise(&mut e, id, col, &p);
                let tag = format!("LAD-{:02} h {height}", id - 40);
                let corridors: Vec<&Primitive> = e
                    .out
                    .iter()
                    .filter(|q| q.kind == FeatureKind::LADDER)
                    .collect();
                let step = match id - 40 {
                    3 | 12 => 4.0,
                    10 => 3.5,
                    _ => MAX_RUN,
                };
                let want = (height / step).ceil() as usize;
                assert_eq!(corridors.len(), want, "{tag}: corridors");
                for c in &corridors {
                    let b = c.shape.aabb();
                    let h = f64::from(b.max.y - b.min.y) / 16.0;
                    assert!(h <= MAX_RUN + 0.01, "{tag}: a {h} m ladder without a rest");
                    assert!(c.flags.contains(PrimFlags::NO_COLLIDE) && c.material.0 < 4);
                    // Nothing solid in the corridor: sample its middle every metre.
                    let (lo, hi) = (b.min_m(), b.max_m());
                    let mid = (lo + hi) * 0.5;
                    let mut y = lo.y + 0.5;
                    while y < hi.y - 0.2 {
                        assert!(
                            !solid_at(&e.out, glam::Vec3::new(mid.x, y, mid.z)),
                            "{tag}: solid in the corridor at {y}"
                        );
                        y += 1.0;
                    }
                }
                let (first, last) = (
                    r.walk.first().expect("route"),
                    r.walk.last().expect("route"),
                );
                for (end, y) in [(first, 100.0), (last, 100.0 + height)] {
                    assert!((end[1] - y).abs() < 0.3, "{tag}: route end {end:?}");
                    assert!(
                        (end[0] - 256.0).abs() < 0.6 && (end[2] - 256.0).abs() < 0.6,
                        "{tag}: not at the centre: {end:?}"
                    );
                    assert!(
                        solid_at(
                            &e.out,
                            glam::Vec3::new(end[0] as f32, end[1] as f32 - 0.1, end[2] as f32)
                        ),
                        "{tag}: no deck under {end:?}"
                    );
                }
            }
        }
    }

    /// PW.5.3, PW.5.4: the ladder families occur in the world (a census over cells).
    #[test]
    fn ladder_families_occur_in_cells() {
        use crate::cell::{CellKey, Level};
        use crate::seed::WorldSeed;
        use crate::tissue::{TissueCache, tissue_cell};
        let cache = TissueCache::default();
        let mut seen = [0usize; 13];
        let n: i64 = std::env::var("LAD_CELLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        for k in 0..n {
            let l1 = glam::I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + glam::I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(WorldSeed(0x5eed), &cache, key);
            for &(_, id) in &t.paths.chosen {
                if let Some(n) = lad_number(id) {
                    seen[usize::from(n) - 1] += 1;
                }
            }
        }
        eprintln!("ladder families in {n} cells: {seen:?}");
        assert!(seen.iter().filter(|&&c| c > 0).count() >= 6, "{seen:?}");
    }
}
