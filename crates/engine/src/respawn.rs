//! Safe ground, fall detection and the death sequence (`docs/spec/traversal.md` §3 and §4).
//! Pure logic driven by the controller each fixed step.

use glam::{Quat, Vec3};
use worldgen::coord::WorldPos;

use crate::collision::{Solid, SolidSource, ray_along};
use crate::controller::Controller;

/// A fall of more than this many metres from the last ground contact kills.
pub const FALL_DEATH_HEIGHT: f32 = 15.0;
/// Continuous airtime after which a missing floor below counts as a void.
pub const VOID_TIMEOUT: f32 = 3.0;
/// The void timeout never grows beyond this, however weak gravity gets (`gravity.md` §7).
pub const VOID_TIMEOUT_MAX: f32 = 12.0;
/// Under this strength fraction the field is a lull.
const LULL_STRENGTH: f32 = 4.0 / 256.0;
/// Drifting this long with no surface within `DRIFT_SURFACE` metres counts as lost.
pub const DRIFT_TIMEOUT: f32 = 90.0;
const DRIFT_SURFACE: f32 = 100.0;
pub const VOID_PROBE: f32 = 200.0;
/// Seconds grounded before a spot counts as safe.
pub const SAFE_GROUNDED_TIME: f32 = 1.0;
const EDGE_RADIUS: f32 = 1.0;
const EDGE_RAYS: usize = 8;
/// How far below the feet each edge ray must find ground (the step-down distance).
const EDGE_REACH: f32 = 0.45;
/// Rays start slightly above the feet so uneven ground does not start inside a solid.
const EDGE_LIFT: f32 = 0.1;

pub const FALL_ON: f32 = 1.0;
pub const FADE_OUT: f32 = 1.5;
pub const FADE_IN: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeSpot {
    pub pos: WorldPos,
    pub yaw: f32,
    /// The local frame at the spot (`gravity.md` §6): `frame * Y` is up.
    pub frame: Quat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Alive,
    /// The fall plays on, then the screen fades to black.
    Dying {
        t: f32,
    },
    /// Black; respawn happens on entry.
    FadingIn {
        t: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    None,
    Died,
    /// The player must be put back at this spot now, while the screen is black.
    Respawn(SafeSpot),
}

#[derive(Debug)]
pub struct Survival {
    pub safe: Option<SafeSpot>,
    pub phase: Phase,
    /// Where the controller was at the last update, for the fall energy.
    prev_pos: WorldPos,
    /// Metres of standard fall since the last ground contact (`gravity.md` §7).
    energy: f32,
    airtime: f32,
    scratch: Vec<Solid>,
}

impl Survival {
    pub fn new(start: WorldPos) -> Self {
        Self {
            safe: None,
            phase: Phase::Alive,
            prev_pos: start,
            energy: 0.0,
            airtime: 0.0,
            scratch: Vec::new(),
        }
    }

    /// Where to respawn when no safe ground was ever recorded: where walking began.
    pub fn set_fallback(&mut self, spot: SafeSpot) {
        self.prev_pos = spot.pos;
        self.energy = 0.0;
        self.airtime = 0.0;
        if self.safe.is_none() {
            self.safe = Some(spot);
        }
    }

    /// Black-out amount for the renderer, 0 (clear) to 1 (black).
    pub fn fade(&self) -> f32 {
        match self.phase {
            Phase::Alive => 0.0,
            Phase::Dying { t } => ((t - FALL_ON) / FADE_OUT).clamp(0.0, 1.0),
            Phase::FadingIn { t } => (1.0 - t / FADE_IN).clamp(0.0, 1.0),
        }
    }

    /// Whether the player may steer; input is ignored while dying.
    pub fn controllable(&self) -> bool {
        self.phase == Phase::Alive
    }

    /// Call once per fixed step, after the controller moved. `yaw` is the player's facing.
    pub fn update(&mut self, dt: f32, ctl: &Controller, yaw: f32, src: &dyn SolidSource) -> Event {
        match self.phase {
            Phase::Alive => {
                if self.track(dt, ctl, yaw, src) {
                    self.phase = Phase::Dying { t: 0.0 };
                    return Event::Died;
                }
                Event::None
            }
            Phase::Dying { t } => {
                let t = t + dt;
                if t >= FALL_ON + FADE_OUT {
                    self.phase = Phase::FadingIn { t: 0.0 };
                    self.airtime = 0.0;
                    self.energy = 0.0;
                    if let Some(spot) = self.safe {
                        self.prev_pos = spot.pos;
                        return Event::Respawn(spot);
                    }
                    return Event::None;
                }
                self.phase = Phase::Dying { t };
                Event::None
            }
            Phase::FadingIn { t } => {
                let t = t + dt;
                self.phase = if t >= FADE_IN {
                    Phase::Alive
                } else {
                    Phase::FadingIn { t }
                };
                Event::None
            }
        }
    }

    /// Updates safe ground and the fall counters; true when the player should die.
    fn track(&mut self, dt: f32, ctl: &Controller, yaw: f32, src: &dyn SolidSource) -> bool {
        let moved = ctl.pos.delta(&self.prev_pos).as_vec3();
        self.prev_pos = ctl.pos;
        if ctl.grounded {
            self.energy = 0.0;
            self.airtime = 0.0;
            if ctl.grounded_time >= SAFE_GROUNDED_TIME
                && !ctl.on_platform()
                && self.edge_clear(ctl, src)
            {
                self.safe = Some(SafeSpot {
                    pos: ctl.pos,
                    yaw,
                    frame: ctl.frame,
                });
            }
            return false;
        }
        self.airtime += dt;
        // Metres of standard fall: the drop along the local down, scaled by the strength.
        self.energy += ctl.strength * moved.dot(-ctl.up()).max(0.0);
        if self.energy > FALL_DEATH_HEIGHT {
            return true;
        }
        // In a lull there is no fall: only 90 s of drifting with no surface within 100 m ends it
        // (`gravity.md` §5.1).
        if ctl.strength < LULL_STRENGTH {
            return self.airtime >= DRIFT_TIMEOUT && !self.surface_near(ctl, src);
        }
        let timeout = (VOID_TIMEOUT / ctl.strength.sqrt()).min(VOID_TIMEOUT_MAX);
        self.airtime >= timeout && ctl.vel.dot(ctl.up()) < 0.0 && !self.ground_below(ctl, src)
    }

    /// Eight rays around the feet at 1 m must all find ground within the step-down distance.
    fn edge_clear(&mut self, ctl: &Controller, src: &dyn SolidSource) -> bool {
        self.scratch.clear();
        src.gather(&ctl.pos, EDGE_RADIUS + 1.0, &mut self.scratch);
        // A parked lift car must not make the spot beside its opening look like firm ground.
        self.scratch.retain(|s| s.platform.is_none());
        let up = ctl.up();
        let (tx, tz) = up.any_orthonormal_pair();
        (0..EDGE_RAYS).all(|k| {
            let a = std::f32::consts::TAU * k as f32 / EDGE_RAYS as f32;
            let start = (tx * a.cos() + tz * a.sin()) * EDGE_RADIUS + up * EDGE_LIFT;
            ray_along(&self.scratch, start, -up, EDGE_LIFT + EDGE_REACH).is_some()
        })
    }

    fn surface_near(&mut self, ctl: &Controller, src: &dyn SolidSource) -> bool {
        self.scratch.clear();
        src.gather(&ctl.pos, DRIFT_SURFACE, &mut self.scratch);
        crate::collision::sdf(&self.scratch, Vec3::ZERO) < DRIFT_SURFACE
    }

    fn ground_below(&mut self, ctl: &Controller, src: &dyn SolidSource) -> bool {
        self.scratch.clear();
        src.gather(&ctl.pos, VOID_PROBE, &mut self.scratch);
        ray_along(&self.scratch, Vec3::ZERO, -ctl.up(), VOID_PROBE).is_some()
    }
}

#[cfg(test)]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
mod tests {
    use super::*;
    use crate::collision::tests::floor_box;
    use crate::controller::MoveInput;
    use crate::controller::tests::{Flat, at, floor, local};

    const DT: f32 = 1.0 / 120.0;

    fn local_of(p: &WorldPos) -> Vec3 {
        p.local()
    }

    fn drive(c: &mut Controller, s: &mut Survival, w: &Flat, secs: f32, wish: Vec3) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(secs / DT) as u32 {
            c.step(
                DT,
                MoveInput {
                    wish,
                    run: false,
                    ..Default::default()
                },
                w,
            );
            match s.update(DT, c, 0.0, w) {
                Event::None => {}
                e => events.push(e),
            }
        }
        events
    }

    #[test]
    fn safe_ground_needs_a_second_of_standing_on_firm_ground() {
        let w = Flat(vec![floor()]);
        let mut c = at(100.0, 10.0, 100.0);
        let mut s = Survival::new(c.pos);
        drive(&mut c, &mut s, &w, 0.9, Vec3::ZERO);
        assert!(s.safe.is_none(), "recorded before 1 s grounded");
        drive(&mut c, &mut s, &w, 0.5, Vec3::ZERO);
        let safe = s.safe.expect("no safe spot after 1.4 s");
        assert!(safe.pos.delta(&c.pos).length() < 0.01);
    }

    #[test]
    fn a_moving_platform_is_never_safe_ground() {
        use crate::controller::tests::Moving;
        let w = Moving::new(|t| (Vec3::X * 2.0 * t, Vec3::X * 2.0));
        let mut c = at(100.0, 12.0, 100.0);
        let mut s = Survival::new(c.pos);
        for _ in 0..(4.0 / DT) as u32 {
            w.step(&mut c, DT, MoveInput::default());
            s.update(DT, &c, 0.0, &w);
        }
        assert!(c.grounded && c.grounded_time > 3.0);
        assert!(s.safe.is_none(), "recorded a spot on a moving platform");
    }

    #[test]
    fn standing_near_an_edge_is_not_safe() {
        // A slab with its edge 0.5 m from the player: some of the 1 m rays hit nothing.
        let slab = floor_box([1440, 160, 320], [1760, 192, 2880]);
        let w = Flat(vec![floor(), slab]);
        let mut c = at(109.5, 12.0, 100.0);
        let mut s = Survival::new(c.pos);
        drive(&mut c, &mut s, &w, 3.0, Vec3::ZERO);
        assert!(c.grounded);
        assert!(s.safe.is_none(), "edge position was recorded as safe");
        // Three metres further in, it is.
        let mut c = at(107.0, 12.0, 100.0);
        let mut s = Survival::new(c.pos);
        drive(&mut c, &mut s, &w, 2.0, Vec3::ZERO);
        assert!(s.safe.is_some());
    }

    #[test]
    fn a_short_drop_is_harmless_and_a_long_one_kills() {
        // A 10 m column over a floor: stepping off a 10 m ledge does not kill.
        let low = floor_box([1440, 160, 320], [1760, 320, 2880]);
        let w = Flat(vec![floor(), low]);
        let mut c = at(108.0, 20.0, 100.0);
        let mut s = Survival::new(c.pos);
        let ev = drive(&mut c, &mut s, &w, 6.0, Vec3::X);
        assert!(ev.is_empty(), "10 m drop killed: {ev:?}");
        assert!((local(&c).y - 10.0).abs() < 0.05);
        // A 25 m ledge does.
        let high = floor_box([1440, 160, 320], [1760, 560, 2880]);
        let w = Flat(vec![floor(), high]);
        let mut c = at(108.0, 35.0, 100.0);
        let mut s = Survival::new(c.pos);
        let ev = drive(&mut c, &mut s, &w, 6.0, Vec3::X);
        assert_eq!(ev.first(), Some(&Event::Died));
    }

    #[test]
    fn falling_into_the_void_times_out_after_three_seconds() {
        // A small platform in empty space: walk off, nothing below for 200 m.
        let slab = floor_box([1440, 1600, 1440], [1600, 1616, 1600]);
        let w = Flat(vec![slab]);
        let mut c = at(98.5, 101.0, 95.0);
        let mut s = Survival::new(c.pos);
        // The fall-height rule (15 m, ~1.75 s) fires before the 3 s void timeout would.
        let ev = drive(&mut c, &mut s, &w, 6.0, Vec3::X);
        assert_eq!(ev.first(), Some(&Event::Died));
    }

    #[test]
    fn void_timeout_covers_falls_that_never_register_height() {
        // Same-height ground contact is refreshed by a hovering "last ground": simulate a
        // controller that is airborne with no ground below and a recent ground at its own height.
        let w = Flat(vec![]);
        let mut c = at(100.0, 50.0, 100.0);
        c.grounded = false;
        c.vel = Vec3::new(0.0, -0.1, 0.0);
        let mut s = Survival::new(c.pos);
        let mut died = None;
        for i in 0..(5.0 / DT) as u32 {
            // Hold the controller still so only the timeout can end the fall.
            if let Event::Died = s.update(DT, &c, 0.0, &w) {
                died = Some(i as f32 * DT);
                break;
            }
        }
        let t = died.expect("no void death");
        assert!((2.9..3.2).contains(&t), "died at {t}");
    }

    #[test]
    fn death_sequence_respawns_at_the_safe_spot_while_black() {
        let high = floor_box([1440, 160, 320], [1760, 560, 2880]);
        let w = Flat(vec![floor(), high]);
        let mut c = at(105.0, 35.0, 100.0);
        let mut s = Survival::new(c.pos);
        drive(&mut c, &mut s, &w, 2.0, Vec3::ZERO);
        assert!(s.safe.is_some(), "safe spot");
        let mut respawned = None;
        let mut max_fade = 0.0_f32;
        for _ in 0..(12.0 / DT) as u32 {
            // Walk off the ledge once, then stand still.
            let wish = if s.phase == Phase::Alive && respawned.is_none() {
                Vec3::X
            } else {
                Vec3::ZERO
            };
            c.step(
                DT,
                MoveInput {
                    wish,
                    run: false,
                    ..Default::default()
                },
                &w,
            );
            if let Event::Respawn(spot) = s.update(DT, &c, 1.25, &w) {
                {
                    assert!(
                        (s.fade() - 1.0).abs() < 1e-3,
                        "respawn not on black: {}",
                        s.fade()
                    );
                    c.set_eye(spot.pos.offset(Vec3::Y * 1.7).unwrap());
                    respawned = Some(spot);
                }
            }
            max_fade = max_fade.max(s.fade());
        }
        let spot = respawned.expect("never respawned");
        // The last safe spot before the edge: on the slab, at least 1 m from its edge at x = 110.
        let at = local_of(&spot.pos);
        assert!(
            (at.y - 35.0).abs() < 0.01 && (105.0..=109.01).contains(&at.x),
            "{at:?}"
        );
        assert!((spot.yaw - 1.25).abs() < 1e-6);
        assert!((max_fade - 1.0).abs() < 1e-3);
        assert_eq!(s.phase, Phase::Alive);
        assert!(s.controllable() && s.fade() == 0.0);
    }

    /// GRV-11: the fatal drop is 15 m in standard gravity and 15 / strength in a constant
    /// weaker field, because the energy is the drop scaled by the strength.
    fn fatal_drop(strength: f32) -> f32 {
        use crate::controller::Field;
        // Ground far below, so the void timeout never decides.
        let w = Flat(vec![floor_box([320, 12000, 320], [2880, 12160, 2880])]);
        let mut c = at(100.0, 900.0, 100.0);
        let mut s = Survival::new(c.pos);
        let field = Field {
            up: Vec3::Y,
            strength,
            current: Vec3::ZERO,
        };
        let top = local(&c).y;
        for _ in 0..(20.0 / DT) as u32 {
            c.step_in(DT, MoveInput::default(), &w, field);
            if s.update(DT, &c, 0.0, &w) == Event::Died {
                return top - local(&c).y;
            }
        }
        panic!("never died at strength {strength}");
    }

    #[test]
    fn the_fatal_drop_scales_with_the_inverse_of_the_strength() {
        let standard = fatal_drop(1.0);
        assert!((standard - 15.0).abs() < 0.3, "{standard}");
        let half = fatal_drop(0.5);
        assert!((half - 30.0).abs() < 0.6, "{half}");
        let low = fatal_drop(0.25);
        assert!((low - 60.0).abs() < 1.2, "{low}");
    }
}
