//! Lifts (`docs/spec/traversal.md` §5): a car that runs up and down a lift shaft between its
//! stops, as a moving platform for the controller. Pure logic; the shaft and its stops come from
//! the generator (`liftplan`).

use glam::{Vec2, Vec3};
use worldgen::coord::WorldPos;
use worldgen::prim::{Q_PER_M, QAabb, QVec3, Shape};

/// Cruise speed of the L1 core lift.
pub const LONG_CRUISE: f32 = 30.0;
/// Cruise speed of a short L2 shaft lift (under 200 m).
pub const SHORT_CRUISE: f32 = 6.0;
pub const ACCEL: f32 = 1.5;
/// Thickness of the car's floor slab, metres; its top is the stop height.
pub const CAR_THICKNESS: f32 = 0.5;
/// Share of `ACCEL` used for the braking curve.
const BRAKE: f32 = 0.9;
/// How far a stop must be beyond the current target to count as the next one.
const STOP_TOLERANCE: f32 = 0.5;

pub struct Lift {
    /// Minimum corner of the shaft's cell at the height all stops are measured from.
    base: WorldPos,
    /// Footprint inside that cell: min corner x, z, and size x, z, metres.
    footprint: (Vec2, Vec2),
    /// Stop heights above `base`, ascending.
    stops: Vec<f32>,
    cruise: f32,
    /// Floor height above `base`, and speed along y.
    y: f32,
    vel: f32,
    target: Option<f32>,
}

impl Lift {
    /// A car waiting at `stops[at]`.
    pub fn new(
        base: WorldPos,
        footprint: (Vec2, Vec2),
        stops: Vec<f32>,
        at: usize,
        cruise: f32,
    ) -> Self {
        let y = stops.get(at).copied().unwrap_or(0.0);
        Self {
            base,
            footprint,
            stops,
            cruise,
            y,
            vel: 0.0,
            target: None,
        }
    }

    pub fn height(&self) -> f32 {
        self.y
    }

    pub fn stops(&self) -> &[f32] {
        &self.stops
    }

    pub fn velocity(&self) -> f32 {
        self.vel
    }

    /// Whether the car is waiting at a stop.
    pub fn idle(&self) -> bool {
        self.target.is_none()
    }

    pub fn target(&self) -> Option<f32> {
        self.target
    }

    /// Sends the car to the next stop above or below. Pressing again while it moves picks the
    /// stop after the one it is heading for; pressing the other way turns it back.
    pub fn go(&mut self, up: bool) {
        let heading_up = self.target.is_some_and(|t| t > self.y);
        let from = match self.target {
            Some(t) if heading_up == up => t,
            _ => self.y,
        };
        let next = if up {
            self.stops
                .iter()
                .copied()
                .find(|&s| s > from + STOP_TOLERANCE)
        } else {
            self.stops
                .iter()
                .rev()
                .copied()
                .find(|&s| s < from - STOP_TOLERANCE)
        };
        if next.is_some() {
            self.target = next;
        }
    }

    /// Moves the car by `dt` seconds: accelerate towards cruise, and once the braking distance at
    /// the current speed reaches the remaining distance, brake at exactly the rate that stops it
    /// on the target.
    pub fn update(&mut self, dt: f32) {
        let Some(target) = self.target else {
            self.vel = 0.0;
            return;
        };
        let remaining = target - self.y;
        let dir = remaining.signum();
        let dist = remaining.abs();
        // Speed towards the target; negative while still moving away from it.
        let mut speed = self.vel * dir;
        let needed = speed * speed / (2.0 * dist.max(1.0e-3));
        if speed > 0.0 && needed >= BRAKE * ACCEL {
            speed -= needed.min(ACCEL) * dt;
        } else {
            speed = (speed + ACCEL * dt).min(self.cruise);
        }
        if speed > 0.0 && speed * dt >= dist || dist < 1.0e-3 {
            self.y = target;
            self.vel = 0.0;
            self.target = None;
            return;
        }
        self.vel = speed * dir;
        self.y += self.vel * dt;
    }

    /// The floor slab's minimum corner.
    pub fn car_origin(&self) -> WorldPos {
        let (min, _) = self.footprint;
        self.base
            .offset(Vec3::new(min.x, self.y - CAR_THICKNESS, min.y))
            .unwrap_or(self.base)
    }

    /// The floor slab as a solid shape in its own local space (min corner at the origin).
    pub fn car_shape(&self) -> Shape {
        let (_, size) = self.footprint;
        let q = |m: f32| (m * Q_PER_M as f32).round() as i32;
        Shape::Box {
            aabb: QAabb::new(
                QVec3::new(0, 0, 0),
                QVec3::new(q(size.x), q(CAR_THICKNESS), q(size.y)),
            ),
        }
    }

    /// Velocity of the car in the world, for the controller.
    pub fn world_velocity(&self) -> Vec3 {
        Vec3::Y * self.vel
    }

    /// Horizontal centre of the shaft.
    pub fn centre(&self) -> WorldPos {
        let (min, size) = self.footprint;
        self.base
            .offset(Vec3::new(
                min.x + size.x * 0.5,
                self.y,
                min.y + size.y * 0.5,
            ))
            .unwrap_or(self.base)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // arrival snaps to exact stop heights
mod tests {
    use super::*;

    fn lift(stops: &[f32], cruise: f32) -> Lift {
        Lift::new(
            WorldPos::ORIGIN,
            (Vec2::new(100.0, 100.0), Vec2::new(16.0, 16.0)),
            stops.to_vec(),
            0,
            cruise,
        )
    }

    /// Runs until idle; returns (seconds, peak speed, peak acceleration). The step that lands on
    /// the stop is left out of the acceleration, since it only removes the last 2 cm/s.
    fn ride(l: &mut Lift) -> (f32, f32, f32) {
        let dt = 1.0 / 120.0;
        let (mut t, mut peak_v, mut peak_a) = (0.0, 0.0_f32, 0.0_f32);
        let mut last = l.velocity();
        let target = l.target().expect("a ride needs a target");
        let dir = (target - l.height()).signum();
        while !l.idle() && t < 600.0 {
            let before = l.height();
            l.update(dt);
            assert!((l.height() - before) * dir >= -1e-4, "moved the wrong way");
            peak_v = peak_v.max(l.velocity().abs());
            if !l.idle() {
                peak_a = peak_a.max(((l.velocity() - last) / dt).abs());
            }
            last = l.velocity();
            t += dt;
        }
        (t, peak_v, peak_a)
    }

    #[test]
    fn a_four_kilometre_ride_takes_about_two_and_a_half_minutes() {
        let mut l = lift(&[0.0, 4096.0], LONG_CRUISE);
        l.go(true);
        let (secs, peak_v, peak_a) = ride(&mut l);
        // 20 s up to speed, 300 m each way, the rest at 30 m/s: about 156 s.
        assert!((secs - 156.5).abs() < 2.0, "{secs} s");
        assert!(
            peak_v <= LONG_CRUISE + 1e-3 && peak_v > LONG_CRUISE - 0.1,
            "{peak_v}"
        );
        assert!(peak_a <= ACCEL + 1e-2, "{peak_a} m/s^2");
        assert_eq!(l.height(), 4096.0);
        assert!(l.idle() && l.velocity() == 0.0);
    }

    #[test]
    fn short_hops_never_reach_cruise_and_stop_exactly() {
        let mut l = lift(&[0.0, 40.0, 100.0], LONG_CRUISE);
        l.go(true);
        let (_, peak_v, _) = ride(&mut l);
        assert!(peak_v < 9.0, "{peak_v}");
        assert_eq!(l.height(), 40.0);
        l.go(true);
        ride(&mut l);
        assert_eq!(l.height(), 100.0);
        // Nothing above: pressing up again does nothing.
        l.go(true);
        assert!(l.idle());
        l.go(false);
        ride(&mut l);
        assert_eq!(l.height(), 40.0);
    }

    #[test]
    fn pressing_again_while_moving_queues_the_next_stop_and_the_other_way_turns_back() {
        let mut l = lift(&[0.0, 300.0, 600.0, 900.0], LONG_CRUISE);
        l.go(true);
        for _ in 0..600 {
            l.update(1.0 / 120.0);
        }
        assert!(l.height() > 5.0 && l.height() < 300.0 && !l.idle());
        l.go(true);
        assert_eq!(l.target(), Some(600.0));
        // Reverse while heading up: the stop below the car, not below the target.
        l.go(false);
        assert_eq!(l.target(), Some(0.0));
        // It has to shed its upward speed first, so it overshoots a little before turning back.
        let top = l.height();
        for _ in 0..72_000 {
            if l.idle() {
                break;
            }
            l.update(1.0 / 120.0);
        }
        assert!(l.idle() && l.height() == 0.0);
        assert!(top < 300.0);
    }

    #[test]
    fn the_short_lift_cruises_at_six_metres_per_second() {
        let mut l = lift(&[0.0, 180.0], SHORT_CRUISE);
        l.go(true);
        let (_, peak_v, _) = ride(&mut l);
        assert!((peak_v - SHORT_CRUISE).abs() < 0.05, "{peak_v}");
    }
}
