//! The landmarks of the world (TOOL-02, TOOL-07): the sites that a viewer can pin from afar or
//! hear from a direction. A function of the world seed and the L0 cells only. Spec:
//! `docs/spec/levels.md` §5W.44.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use glam::I64Vec3;

use crate::seed::WorldSeed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Wonder,
    Rare,
    /// A machine that moves (and so can be heard).
    Mover,
    /// A spine tower.
    Spine,
}

impl Kind {
    pub const fn name(self) -> &'static str {
        match self {
            Kind::Wonder => "wonder",
            Kind::Rare => "oddity",
            Kind::Mover => "machine",
            Kind::Spine => "spine",
        }
    }

    /// What it sounds like, for the listening cue.
    pub const fn sound(self) -> &'static str {
        match self {
            Kind::Wonder => "a held chord",
            Kind::Rare => "something wrong with the air",
            Kind::Mover => "a slow grinding",
            Kind::Spine => "wind in a long shaft",
        }
    }

    /// How far, in L1 cells, it can be heard.
    pub const fn hearing(self) -> i64 {
        match self {
            Kind::Wonder => 12,
            Kind::Rare => 8,
            Kind::Mover => 10,
            Kind::Spine => 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landmark {
    pub kind: Kind,
    /// The L1 cell it stands in.
    pub l1: I64Vec3,
}

/// The landmarks in the L0 cells within `reach` cells of `l0` (a cube), in a fixed order.
pub fn near(seed: WorldSeed, l0: I64Vec3, reach: i64) -> Vec<Landmark> {
    let mut out = Vec::new();
    for z in -reach..=reach {
        for y in -reach..=reach {
            for x in -reach..=reach {
                let c = l0 + I64Vec3::new(x, y, z);
                if let Some(s) = crate::wonders::site_of(seed, c) {
                    out.push(Landmark {
                        kind: Kind::Wonder,
                        l1: s.l1,
                    });
                }
                if let Some(s) = crate::rare::site_of(seed, c) {
                    out.push(Landmark {
                        kind: Kind::Rare,
                        l1: s.l1,
                    });
                }
                if let Some((_, l1)) = crate::movers::site_of(seed, c) {
                    out.push(Landmark {
                        kind: Kind::Mover,
                        l1,
                    });
                }
            }
        }
    }
    // Spines run through a whole column: one landmark at the middle of the cell's range.
    for z in -reach..=reach {
        for x in -reach..=reach {
            let (cx, cz) = (l0.x + x, l0.z + z);
            if let Some((ix, iz)) = crate::spines::spine_in_column(seed, cx, cz) {
                out.push(Landmark {
                    kind: Kind::Spine,
                    l1: I64Vec3::new(cx * 8 + ix, l0.y * 8 + 4, cz * 8 + iz),
                });
            }
        }
    }
    out
}

/// The landmark that `dir` (a unit vector, in metres' space) points at best from `from`, as an
/// L1 cell coordinate pair `(position in L1 cells, landmark)`, within `max_cells` L1 cells and
/// `cos_min` of the view direction.
pub fn best_along(
    seed: WorldSeed,
    from_l1: [f64; 3],
    dir: [f64; 3],
    max_cells: f64,
    cos_min: f64,
) -> Option<Landmark> {
    let l0 = I64Vec3::new(
        (from_l1[0] / 8.0).floor() as i64,
        (from_l1[1] / 8.0).floor() as i64,
        (from_l1[2] / 8.0).floor() as i64,
    );
    let reach = (max_cells / 8.0).ceil() as i64;
    let mut best: Option<(f64, Landmark)> = None;
    for lm in near(seed, l0, reach) {
        let d = [
            lm.l1.x as f64 + 0.5 - from_l1[0],
            lm.l1.y as f64 + 0.5 - from_l1[1],
            lm.l1.z as f64 + 0.5 - from_l1[2],
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 0.5 || len > max_cells {
            continue;
        }
        let cos = (d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]) / len;
        if cos >= cos_min && best.as_ref().is_none_or(|b| cos > b.0) {
            best = Some((cos, lm));
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landmarks_are_a_function_of_the_cells() {
        let seed = WorldSeed(3);
        let a = near(seed, I64Vec3::new(2, 0, -3), 3);
        assert_eq!(a, near(seed, I64Vec3::new(2, 0, -3), 3));
        assert!(a.iter().any(|l| l.kind == Kind::Wonder));
        assert!(a.iter().any(|l| l.kind == Kind::Spine));
    }

    #[test]
    fn the_landmark_in_view_is_found_and_one_behind_is_not() {
        let seed = WorldSeed(3);
        let from = [20.5, 4.0, 20.5];
        let lms = near(seed, I64Vec3::new(2, 0, 2), 3);
        let lm = lms[0];
        let d = [
            lm.l1.x as f64 + 0.5 - from[0],
            lm.l1.y as f64 + 0.5 - from[1],
            lm.l1.z as f64 + 0.5 - from[2],
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = [d[0] / len, d[1] / len, d[2] / len];
        assert!(best_along(seed, from, dir, 200.0, 0.99).is_some());
        let back = dir.map(|v| -v);
        let found = best_along(seed, from, back, 200.0, 0.999);
        assert!(found.is_none_or(|f| f != lm));
    }
}
