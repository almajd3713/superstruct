//! The walker's tools (TOOL-01..04, 06..08, WAKE-01..02): a held zoom, a gaze-mark that pins a
//! far landmark, the atlas of places arrived in, a cue for the direction of a sound when
//! standing still, the trail and chalk marks. All of it is saved in `save.toml`. Spec:
//! `docs/spec/levels.md` §5W.44.

#![allow(clippy::cast_sign_loss)] // distances and counts here are never negative

use glam::{I64Vec3, Vec3};
use worldgen::coord::WorldPos;
use worldgen::glyphs::{name_of, transliterate};
use worldgen::landmarks::{self, Landmark};
use worldgen::quarters::{NAMES as QUARTERS, quarter_at_l1};
use worldgen::seed::WorldSeed;

use crate::save::{BeaconPos, Save};

/// Magnification of the held zoom, and in the Observatory quarter (TOOL-01, TOOL-03).
const ZOOM: f32 = 4.0;
const ZOOM_OBSERVATORY: f32 = 16.0;
const OBSERVATORY: u8 = 11;
/// A point of the trail every this many metres (WAKE-02), and the most that are kept.
const TRAIL_STEP_M: f64 = 25.0;
const TRAIL_MAX: usize = 6000;
const MARKS_MAX: usize = 8;
const PLACES_MAX: usize = 4096;
/// Seconds in one L1 cell before it is a place arrived in (TOOL-04).
const ARRIVAL_S: f32 = 4.0;
/// Seconds standing still before a sound is heard (TOOL-07).
const LISTEN_S: f32 = 2.5;
/// How far a gaze-mark reaches, in L1 cells, and how closely the view must point at it.
const MARK_REACH: f64 = 400.0;
const MARK_COS: f64 = 0.995;
const CHALK_REACH_M: f32 = 6.0;

#[allow(clippy::struct_excessive_bools)] // one flag per key the tools read
pub struct Frame {
    pub eye: WorldPos,
    pub forward: Vec3,
    pub dt: f32,
    pub zoom: bool,
    pub mark: bool,
    pub chalk: bool,
    pub moving: bool,
    /// Distance to the first surface along `forward`, if any in 300 m.
    pub hit_m: Option<f32>,
}

/// NOT-04: lamps every `ROUTE_STEP_M` for `ROUTE_LAMPS` of them (a kilometre), laid again once
/// the walker has moved `ROUTE_RELAY_M`.
const ROUTE_STEP_M: f32 = 40.0;
const ROUTE_LAMPS: usize = 25;
const ROUTE_RELAY_M: f64 = 40.0;

/// The lamps of a route from the walker toward a pinned mark.
pub struct Route {
    pub anchor: WorldPos,
    /// Metres from `anchor`; empty when there is no route.
    pub lamps: Vec<Vec3>,
}

pub struct Tools {
    seed: WorldSeed,
    /// Set when the route was laid anew this frame; the app takes it to redraw.
    pub route: Option<Route>,
    last_route: Option<WorldPos>,
    pub atlas: bool,
    still_s: f32,
    dwell: Option<(I64Vec3, f32)>,
    last_trail: Option<WorldPos>,
    pub lines: Vec<String>,
}

pub struct Out {
    /// The factor to narrow the view by, 1 when not zooming.
    pub zoom: f32,
    /// Something in the save changed.
    pub dirty: bool,
}

/// The L1 cell holding `p`, and `p` in L1 cells.
pub fn l1_of(p: &WorldPos) -> (I64Vec3, [f64; 3]) {
    let s = p.sector();
    let l = p.local();
    let cell = s.div_euclid(I64Vec3::splat(4));
    let rem = s - cell * 4;
    let f = |r: i64, l: f32| (r as f64 * 1024.0 + f64::from(l)) / 4096.0;
    (
        cell,
        [
            cell.x as f64 + f(rem.x, l.x),
            cell.y as f64 + f(rem.y, l.y),
            cell.z as f64 + f(rem.z, l.z),
        ],
    )
}

/// The view angle after magnifying by `factor`.
pub fn zoomed_fov(fov_y: f32, factor: f32) -> f32 {
    2.0 * ((fov_y / 2.0).tan() / factor).atan()
}

/// Where `to` lies relative to the view `forward`, in words: "ahead", "right 40°", "behind".
pub fn bearing(forward: Vec3, to: Vec3) -> String {
    let (f, t) = (
        Vec3::new(forward.x, 0.0, forward.z),
        Vec3::new(to.x, 0.0, to.z),
    );
    if t.length() < 1e-6 {
        return if to.y >= 0.0 { "above" } else { "below" }.into();
    }
    if f.length() < 1e-6 {
        return "ahead".into();
    }
    let ang = f.angle_between(t).to_degrees();
    if ang < 10.0 {
        return "ahead".into();
    }
    if ang > 170.0 {
        return "behind".into();
    }
    // Right-handed, +Y up: positive y of f × t is a turn to the left.
    let side = if f.cross(t).y > 0.0 { "left" } else { "right" };
    format!("{side} {ang:.0}\u{b0}")
}

fn direction_to(from: [f64; 3], l1: I64Vec3) -> (Vec3, f64) {
    let d = [
        l1.x as f64 + 0.5 - from[0],
        l1.y as f64 + 0.5 - from[1],
        l1.z as f64 + 0.5 - from[2],
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (
        Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32).normalize_or_zero(),
        len * 4.096,
    )
}

fn place_line(seed: WorldSeed, l1: [i64; 3]) -> String {
    let c = I64Vec3::from(l1);
    let q = quarter_at_l1(seed, c).map_or("between", |q| QUARTERS[usize::from(q)]);
    format!(
        "{}  {q}  ({} {} {})",
        transliterate(name_of(seed, c)),
        l1[0],
        l1[1],
        l1[2]
    )
}

impl Tools {
    pub fn new(seed: WorldSeed) -> Self {
        Self {
            seed,
            route: None,
            last_route: None,
            atlas: false,
            still_s: 0.0,
            dwell: None,
            last_trail: None,
            lines: Vec::new(),
        }
    }

    pub fn update(&mut self, f: &Frame, save: &mut Save, toggle_atlas: bool) -> Out {
        let mut dirty = false;
        if toggle_atlas {
            self.atlas = !self.atlas;
        }
        let (cell, at) = l1_of(&f.eye);
        let observatory = quarter_at_l1(self.seed, cell) == Some(OBSERVATORY);
        let zoom = if f.zoom {
            if observatory { ZOOM_OBSERVATORY } else { ZOOM }
        } else {
            1.0
        };

        // TOOL-02: the landmark the view points at is pinned, or released if it already is.
        if f.mark {
            let dir = [
                f64::from(f.forward.x),
                f64::from(f.forward.y),
                f64::from(f.forward.z),
            ];
            if let Some(Landmark { l1, .. }) =
                landmarks::best_along(self.seed, at, dir, MARK_REACH, MARK_COS)
            {
                let key = [l1.x, l1.y, l1.z];
                if let Some(i) = save.marks.iter().position(|m| *m == key) {
                    save.marks.remove(i);
                } else {
                    if save.marks.len() == MARKS_MAX {
                        save.marks.remove(0);
                    }
                    save.marks.push(key);
                }
                dirty = true;
            }
        }

        // TOOL-04: a place is arrived in by staying in its cell a few seconds.
        match &mut self.dwell {
            Some((c, t)) if *c == cell => *t += f.dt,
            _ => self.dwell = Some((cell, 0.0)),
        }
        if let Some((c, t)) = self.dwell {
            let key = [c.x, c.y, c.z];
            if t >= ARRIVAL_S && save.places.len() < PLACES_MAX && !save.places.contains(&key) {
                save.places.push(key);
                dirty = true;
            }
        }

        // WAKE-01..02: a lamp wakes every 25 m along the way, and stays.
        let far = self
            .last_trail
            .is_none_or(|p| f.eye.delta(&p).length() >= TRAIL_STEP_M);
        if far && save.trail.len() < TRAIL_MAX {
            let feet = f
                .eye
                .offset(Vec3::NEG_Y * crate::controller::EYE_HEIGHT)
                .unwrap_or(f.eye);
            save.trail.push(BeaconPos::of(&feet));
            self.last_trail = Some(f.eye);
            dirty = true;
        }

        // TOOL-08: chalk on the wall in view.
        if f.chalk
            && let Some(d) = f.hit_m.filter(|d| *d <= CHALK_REACH_M)
            && let Ok(p) = f.eye.offset(f.forward * (d - 0.2).max(0.0))
        {
            save.chalk.push(BeaconPos::of(&p));
            dirty = true;
        }

        // NOT-04: with a mark pinned, a line of lamps lights up ahead toward the nearest one.
        let relay = self
            .last_route
            .is_none_or(|p| f.eye.delta(&p).length() >= ROUTE_RELAY_M);
        if relay {
            self.last_route = Some(f.eye);
            let nearest = save
                .marks
                .iter()
                .map(|m| direction_to(at, I64Vec3::from(*m)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let lamps = nearest.map_or_else(Vec::new, |(dir, km)| {
                let n =
                    ((km * 1000.0 / f64::from(ROUTE_STEP_M)).max(0.0) as usize).min(ROUTE_LAMPS);
                (1..=n)
                    .map(|k| {
                        dir * (k as f32 * ROUTE_STEP_M) - Vec3::Y * crate::controller::EYE_HEIGHT
                    })
                    .collect()
            });
            self.route = Some(Route {
                anchor: f.eye,
                lamps,
            });
        }

        self.still_s = if f.moving { 0.0 } else { self.still_s + f.dt };
        self.lines = self.text(f, at, cell, save);
        Out { zoom, dirty }
    }

    fn text(&self, f: &Frame, at: [f64; 3], cell: I64Vec3, save: &Save) -> Vec<String> {
        let mut lines = Vec::new();
        for m in &save.marks {
            let (dir, km) = direction_to(at, I64Vec3::from(*m));
            lines.push(format!(
                "\u{25c7} {}  {km:.1} km  {}",
                transliterate(name_of(self.seed, I64Vec3::from(*m))),
                bearing(f.forward, dir)
            ));
        }
        // TOOL-07: standing still, the nearest sound within its hearing is given a direction.
        if self.still_s >= LISTEN_S && !self.silent(f) {
            let heard = landmarks::near(self.seed, cell.div_euclid(I64Vec3::splat(8)), 2)
                .into_iter()
                .filter_map(|lm| {
                    let (dir, km) = direction_to(at, lm.l1);
                    let cells = km / 4.096;
                    (cells <= lm.kind.hearing() as f64).then_some((cells, lm, dir))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, lm, dir)) = heard {
                lines.push(format!(
                    "you hear {}, {}",
                    lm.kind.sound(),
                    bearing(f.forward, dir)
                ));
            }
        }
        if self.atlas {
            lines.push(format!("atlas: {} places", save.places.len()));
            for p in save.places.iter().rev().take(12) {
                lines.push(place_line(self.seed, *p));
            }
        }
        lines
    }

    fn silent(&self, f: &Frame) -> bool {
        let s = f.eye.sector();
        let l = f.eye.local();
        worldgen::strange::silent_at(
            self.seed,
            [
                s.x * 1024 + l.x as i64,
                s.y * 1024 + l.y as i64,
                s.z * 1024 + l.z as i64,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: f32) -> WorldPos {
        WorldPos::new(I64Vec3::new(10, 2, 10), Vec3::new(x, 100.0, 0.0)).unwrap()
    }

    fn frame(eye: WorldPos) -> Frame {
        Frame {
            eye,
            forward: Vec3::NEG_Z,
            dt: 1.0,
            zoom: false,
            mark: false,
            chalk: false,
            moving: false,
            hit_m: None,
        }
    }

    #[test]
    fn zoom_narrows_the_view_by_the_factor() {
        let fov = 90f32.to_radians();
        let z = zoomed_fov(fov, 4.0);
        assert!(((z / 2.0).tan() - 0.25).abs() < 1e-5);
        assert!((zoomed_fov(fov, 1.0) - fov).abs() < 1e-5);
    }

    #[test]
    fn bearings_read_left_right_and_behind() {
        let f = Vec3::NEG_Z;
        assert_eq!(bearing(f, Vec3::NEG_Z), "ahead");
        assert_eq!(bearing(f, Vec3::Z), "behind");
        assert!(bearing(f, Vec3::X).starts_with("right"));
        assert!(bearing(f, Vec3::NEG_X).starts_with("left"));
    }

    #[test]
    fn the_trail_gains_a_lamp_every_25_metres_and_places_are_arrived_in() {
        let mut tools = Tools::new(WorldSeed(1));
        let mut save = Save::default();
        tools.update(&frame(pos(0.0)), &mut save, false);
        assert_eq!(save.trail.len(), 1);
        tools.update(&frame(pos(10.0)), &mut save, false);
        assert_eq!(save.trail.len(), 1);
        tools.update(&frame(pos(30.0)), &mut save, false);
        assert_eq!(save.trail.len(), 2);
        // Four seconds in one cell: arrived.
        for _ in 0..4 {
            tools.update(&frame(pos(30.0)), &mut save, false);
        }
        assert_eq!(save.places.len(), 1);
    }

    #[test]
    fn chalk_goes_on_a_wall_in_reach_only() {
        let mut tools = Tools::new(WorldSeed(1));
        let mut save = Save::default();
        let mut f = frame(pos(0.0));
        f.chalk = true;
        f.hit_m = Some(20.0);
        tools.update(&f, &mut save, false);
        assert!(save.chalk.is_empty());
        f.hit_m = Some(3.0);
        tools.update(&f, &mut save, false);
        assert_eq!(save.chalk.len(), 1);
    }

    #[test]
    fn a_gaze_mark_pins_the_landmark_in_view_and_a_second_press_releases_it() {
        let seed = WorldSeed(3);
        let lm = landmarks::near(seed, I64Vec3::new(2, 0, 2), 2)[0];
        let eye_l1 = I64Vec3::new(lm.l1.x - 6, lm.l1.y, lm.l1.z);
        // L1 cell -> sector: 4 sectors per L1 cell; stand mid-cell.
        let eye = WorldPos::new(
            eye_l1 * 4 + I64Vec3::new(2, 2, 2),
            Vec3::new(512.0, 512.0, 512.0),
        )
        .unwrap();
        let mut tools = Tools::new(seed);
        let mut save = Save::default();
        let mut f = frame(eye);
        f.mark = true;
        let (_, at) = l1_of(&eye);
        let (dir, _) = direction_to(at, lm.l1);
        f.forward = dir;
        tools.update(&f, &mut save, false);
        assert_eq!(save.marks.len(), 1, "{lm:?}");
        tools.update(&f, &mut save, false);
        assert!(save.marks.is_empty());
    }
}
