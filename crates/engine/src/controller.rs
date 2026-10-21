//! First-person walking controller: a kinematic capsule with step-up, step-down snap, a slope
//! limit and no jump (`docs/spec/traversal.md`). Pure logic; solids come from a [`SolidSource`].
//!
//! The capsule is a chain of spheres, pushed out of the union of solids along the signed
//! distance gradient.

// Substep and frame counts are small non-negative floats.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use glam::{Quat, Vec3};
use worldgen::coord::WorldPos;

use crate::collision::{LadderVol, Solid, SolidSource, gradient, sdf};

pub const RADIUS: f32 = 0.3;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.65;
pub const STEP_UP: f32 = 0.45;
pub const WALK_SPEED: f32 = 1.4;
pub const RUN_SPEED: f32 = 4.0;
pub const GRAVITY: f32 = 9.81;
/// Climbing speed on a ladder (CLB-05).
pub const LADDER_SPEED: f32 = 0.8;
const MAX_SLOPE_DEG: f32 = 40.0;
const ACCEL_TIME: f32 = 0.35;
const DECEL_TIME: f32 = 0.25;
const AIR_CONTROL: f32 = 0.1;
/// Largest move between collision resolutions, well under the capsule radius.
const MAX_SUBSTEP: f32 = 0.1;
const SPHERES: usize = 5;
/// Spheres of the crouched capsule: 1.2 m high, for the low ducts of the Web (CLB-07).
const CROUCH_SPHERES: usize = 3;
const CROUCH_EYE: f32 = 1.05;
const SKIN: f32 = 1e-3;

#[derive(Debug, Clone, Copy, Default)]
pub struct MoveInput {
    /// Desired horizontal direction in the world, length at most 1.
    pub wish: Vec3,
    pub run: bool,
    /// On a ladder: -1 down to 1 up (forward moves where you look, `docs/spec/climbing.md`).
    pub climb: f32,
    /// The view direction (unit, or zero), for the drift push where there is no ground.
    pub look: Vec3,
    /// Forward (1) or back (-1) on the keys, pushing along `look` when drifting.
    pub thrust: f32,
}

/// A ladder the player is on: its corridor, in the world.
#[derive(Debug, Clone, Copy)]
struct Climb {
    centre: WorldPos,
    half_y: f32,
    /// Half the corridor's depth along `toward`.
    half_toward: f32,
    toward: Vec3,
}

#[derive(Debug)]
pub struct Controller {
    /// Position of the capsule's lowest point.
    pub pos: WorldPos,
    pub vel: Vec3,
    pub grounded: bool,
    pub ground_normal: Vec3,
    /// Seconds spent continuously grounded.
    pub grounded_time: f32,
    /// Velocity of the moving platform the player stands on, if any.
    platform: Option<Vec3>,
    solids: Vec<Solid>,
    climb: Option<Climb>,
    ladders: Vec<LadderVol>,
    /// Crouched in a low duct: the capsule is 1.2 m (CLB-07).
    pub crouched: bool,
    /// Rotation of the local frame: `frame * Y` is the capsule's up (`docs/spec/gravity.md` §6).
    pub frame: Quat,
    /// Gravity strength of the last step as a fraction of standard (0 in a lull).
    pub strength: f32,
}

/// The gravity a step runs in: a unit `up` (zero in a lull, where the frame keeps its turn) and
/// a strength as a fraction of standard gravity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub up: Vec3,
    pub strength: f32,
    /// The current of a drift lane here, in m/s: a drifting player's speed relaxes towards it.
    pub current: Vec3,
}

impl Field {
    pub const STANDARD: Field = Field {
        up: Vec3::Y,
        strength: 1.0,
        current: Vec3::ZERO,
    };

    pub fn from_gravity(g: worldgen::gravity::Gravity) -> Self {
        let v = Vec3::new(f32::from(g.up[0]), f32::from(g.up[1]), f32::from(g.up[2]));
        Field {
            up: v.normalize_or_zero(),
            strength: f32::from(g.strength) / 256.0,
            current: Vec3::ZERO,
        }
    }
}

/// Fastest the frame turns when `up` changes, in radians per second (`gravity.md` §6).
const TURN_RATE: f32 = std::f32::consts::FRAC_PI_2;
/// Time constant of the ease at the end of a turn.
const TURN_EASE: f32 = 0.15;
/// Under this fraction of standard gravity a field has no direction (a lull).
const LULL: f32 = 4.0 / 256.0;
/// Free drift (`gravity.md` §5.1): velocity decays with this time constant, the push is
/// `DRIFT_PUSH` m/s² along the view up to `DRIFT_TOP` m/s. In low gravity the air push is
/// `AIR_PUSH`.
const DRIFT_DECAY: f32 = 4.0;
const DRIFT_PUSH: f32 = 0.5;
const DRIFT_TOP: f32 = 1.0;
const AIR_PUSH: f32 = 0.25;
/// Below this strength fraction gravity is low enough for the air push (64/256).
const LOW_GRAVITY: f32 = 0.25;

impl Controller {
    pub fn new(pos: WorldPos) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            grounded: false,
            ground_normal: Vec3::Y,
            grounded_time: 0.0,
            platform: None,
            solids: Vec::new(),
            climb: None,
            ladders: Vec::new(),
            crouched: false,
            frame: Quat::IDENTITY,
            strength: 1.0,
        }
    }

    /// The capsule's up in the world.
    pub fn up(&self) -> Vec3 {
        self.frame * Vec3::Y
    }

    /// Removes the up component.
    fn flat(&self, v: Vec3) -> Vec3 {
        v - self.up() * v.dot(self.up())
    }

    /// Turns the frame towards the field's up by the shortest rotation at a capped rate.
    fn orient(&mut self, dt: f32, field: Field) {
        self.strength = field.strength;
        if field.strength < LULL || field.up == Vec3::ZERO {
            return;
        }
        let up = self.up();
        let angle = up.dot(field.up).clamp(-1.0, 1.0).acos();
        if angle < 1e-4 {
            return;
        }
        let step = (angle * (dt / TURN_EASE).min(1.0))
            .min(TURN_RATE * dt)
            .max(1e-4_f32.min(angle));
        let axis = up.cross(field.up);
        let axis = if axis.length_squared() < 1e-10 {
            self.frame * Vec3::X
        } else {
            axis.normalize()
        };
        self.frame = (Quat::from_axis_angle(axis, step.min(angle)) * self.frame).normalize();
    }

    /// Whether the ground underfoot belongs to a moving platform. Such spots never count as safe
    /// ground: the platform will have moved on when you respawn there.
    pub fn on_platform(&self) -> bool {
        self.grounded && self.platform.is_some()
    }

    /// Whether the player is on a ladder.
    pub fn on_ladder(&self) -> bool {
        self.climb.is_some()
    }

    fn spheres(&self) -> usize {
        if self.crouched {
            CROUCH_SPHERES
        } else {
            SPHERES
        }
    }

    /// Eye height above the feet.
    pub fn eye_height(&self) -> f32 {
        if self.crouched {
            CROUCH_EYE
        } else {
            EYE_HEIGHT
        }
    }

    /// Crouches where the ceiling is lower than the standing capsule and stands up again once
    /// there is room, here and half a metre ahead (CLB-07).
    fn update_crouch(&mut self, wish: Vec3) {
        let up = self.up();
        let ahead = self.flat(wish).normalize_or_zero() * 0.5;
        let tight = |at: Vec3| penetration(&self.solids, at, SPHERES, up) > 0.02;
        let (here, next) = (tight(Vec3::ZERO), tight(ahead));
        if !self.crouched {
            let at = if here { Vec3::ZERO } else { ahead };
            if (here || next) && penetration(&self.solids, at, CROUCH_SPHERES, up) <= 0.02 {
                self.crouched = true;
            }
        } else if !here && !next {
            self.crouched = false;
        }
    }

    pub fn eye(&self) -> WorldPos {
        self.pos
            .offset(self.up() * self.eye_height())
            .unwrap_or(self.pos)
    }

    /// Moves the controller so its eye is at `eye`.
    pub fn set_eye(&mut self, eye: WorldPos) {
        self.pos = eye.offset(-self.up() * self.eye_height()).unwrap_or(eye);
        self.vel = Vec3::ZERO;
        self.grounded = false;
        self.grounded_time = 0.0;
        self.platform = None;
    }

    /// Advances by `dt` seconds. Does nothing until collision data exists around the player.
    pub fn step(&mut self, dt: f32, input: MoveInput, src: &dyn SolidSource) {
        self.step_in(dt, input, src, Field::STANDARD);
    }

    /// `step` in an arbitrary gravity field: the frame turns to the field's up, gravity is
    /// scaled by its strength, and everything vertical runs along the local up.
    pub fn step_in(&mut self, dt: f32, input: MoveInput, src: &dyn SolidSource, field: Field) {
        crate::zone!("controller step");
        if !src.ready(&self.pos) {
            return;
        }
        self.orient(dt, field);
        if field.strength < LULL {
            self.drift_step(dt, input, src, field.current);
            return;
        }
        if let Some(c) = self.climb {
            self.climb_step(dt, input, c, src);
            return;
        }
        if self.up().y > 0.98
            && let Some(c) = self.try_attach(input, src)
        {
            self.climb = Some(c);
            self.vel = Vec3::ZERO;
            self.grounded = false;
            self.grounded_time = 0.0;
            self.platform = None;
            return;
        }
        let top_speed = if input.run { RUN_SPEED } else { WALK_SPEED };
        let up = self.up();
        let wish = self.flat(input.wish).clamp_length_max(1.0);
        let target = wish * top_speed;
        let mut rate = if wish == Vec3::ZERO {
            RUN_SPEED / DECEL_TIME
        } else {
            top_speed / ACCEL_TIME
        };
        if !self.grounded {
            rate *= AIR_CONTROL;
        }
        let vh = approach(self.flat(self.vel), target, rate * dt);
        let vy = if self.grounded {
            0.0
        } else {
            self.vel.dot(up) - GRAVITY * field.strength * dt
        };
        self.vel = vh + up * vy;
        if !self.grounded && field.strength < LOW_GRAVITY {
            self.vel += input.look * (input.thrust.clamp(-1.0, 1.0) * AIR_PUSH * dt);
        }

        let travel = self.vel.length() * dt;
        self.solids.clear();
        src.gather(&self.pos, 3.0 + travel, &mut self.solids);
        self.update_crouch(input.wish);
        // The platform underfoot has already moved to its new place; ride along with it.
        let mut p = self.platform.map_or(Vec3::ZERO, |v| v * dt);
        let mut disp = self.vel * dt;
        if self.grounded {
            // Follow the ground plane so slopes are climbed at walking speed.
            let n = self.ground_normal;
            disp = self.flat(disp);
            disp -= n * disp.dot(n) * 1.0;
            disp = disp.clamp_length_max(vh.length() * dt * 1.5 + 1e-6);
        } else {
            disp = vh * dt + up * (vy * dt);
        }
        let substeps = (disp.length() / MAX_SUBSTEP).ceil().max(1.0) as u32;
        let sub = disp / substeps as f32;
        for _ in 0..substeps {
            self.substep(&mut p, sub);
        }
        self.settle(&mut p, dt);
        self.pos = self.pos.offset(p).unwrap_or(self.pos);
    }

    /// Free drift in a lull (GRV-14): no gravity, velocity decays over 4 s, the forward key pushes
    /// along the view at 0.5 m/s² up to 1 m/s, and a surface takes the speed into it away
    /// without damage.
    fn drift_step(&mut self, dt: f32, input: MoveInput, src: &dyn SolidSource, current: Vec3) {
        self.grounded = false;
        self.grounded_time = 0.0;
        self.platform = None;
        self.climb = None;
        // The speed decays towards the current of the lane, if there is one.
        self.vel = current + (self.vel - current) * (1.0 - dt / DRIFT_DECAY).max(0.0);
        let thrust = input.thrust.clamp(-1.0, 1.0);
        let along = self.vel.dot(input.look);
        if thrust != 0.0
            && ((thrust > 0.0 && along < DRIFT_TOP) || (thrust < 0.0 && along > -DRIFT_TOP))
        {
            self.vel += input.look * (thrust * DRIFT_PUSH * dt);
        }
        self.solids.clear();
        src.gather(&self.pos, 3.0 + self.vel.length() * dt, &mut self.solids);
        let mut p = Vec3::ZERO;
        let disp = self.vel * dt;
        let substeps = (disp.length() / MAX_SUBSTEP).ceil().max(1.0) as u32;
        let sub = disp / substeps as f32;
        for _ in 0..substeps {
            self.substep(&mut p, sub);
        }
        self.pos = self.pos.offset(p).unwrap_or(self.pos);
    }

    /// Walking into a ladder's corridor while facing the rungs puts you on it (CLB-01).
    fn try_attach(&mut self, input: MoveInput, src: &dyn SolidSource) -> Option<Climb> {
        let wish = Vec3::new(input.wish.x, 0.0, input.wish.z).normalize_or_zero();
        if wish == Vec3::ZERO {
            return None;
        }
        self.ladders.clear();
        src.ladders(&self.pos, 2.0, &mut self.ladders);
        let mid = Vec3::Y * (HEIGHT * 0.5);
        for l in &self.ladders {
            if wish.dot(l.toward) < 0.5 {
                continue;
            }
            let d = mid - l.centre;
            let local = [d.dot(l.axes[0]), d.dot(l.axes[1]), d.dot(l.axes[2])];
            let half_toward =
                l.toward.dot(l.axes[0]).abs() * l.half.x + l.toward.dot(l.axes[2]).abs() * l.half.z;
            let in_front = d.dot(l.toward) <= half_toward + 0.2;
            let feet = local[1] - HEIGHT * 0.5;
            if local[0].abs() <= l.half.x + 0.2
                && local[2].abs() <= l.half.z + 0.2
                && in_front
                && feet >= -l.half.y - 0.1
                && feet <= l.half.y - 0.1
            {
                let centre = self.pos.offset(l.centre).ok()?;
                return Some(Climb {
                    centre,
                    half_y: l.half.y,
                    half_toward,
                    toward: l.toward,
                });
            }
        }
        None
    }

    /// On a ladder: no gravity, no walking off; the climb input moves you up or down at
    /// `LADDER_SPEED`, and only the ends let go (CLB-02..05).
    fn climb_step(&mut self, dt: f32, input: MoveInput, c: Climb, src: &dyn SolidSource) {
        let rel = c.centre.delta(&self.pos).as_vec3();
        // Hold against the rungs: the corridor's middle line, shifted to the open side.
        let hold = Vec3::new(rel.x, 0.0, rel.z) - c.toward * (c.half_toward - 0.35).max(0.0);
        let side = hold * (dt * 10.0).min(1.0);
        let dy = input.climb.clamp(-1.0, 1.0) * LADDER_SPEED * dt;
        let top = rel.y + c.half_y;
        let bottom = rel.y - c.half_y;
        if dy >= 0.0 && top <= 0.05 {
            // The top: step forward over the edge onto what is there, or stay on the last rung.
            self.solids.clear();
            src.gather(&self.pos, 3.0, &mut self.solids);
            for dir in [c.toward, -c.toward] {
                let q = dir * 0.7 + Vec3::Y * 0.05;
                if penetration(&self.solids, q, self.spheres(), Vec3::Y) <= SKIN
                    && let Some((drop, n)) = probe_down(&self.solids, q, 1.0, Vec3::Y)
                    && walkable(&self.solids, q - Vec3::Y * drop, n, Vec3::Y)
                {
                    self.pos = self.pos.offset(q - Vec3::Y * drop).unwrap_or(self.pos);
                    self.climb = None;
                    self.vel = Vec3::ZERO;
                    return;
                }
            }
        } else if dy <= 0.0 && bottom >= -0.02 {
            // The bottom: step back off, onto the floor you came from.
            self.pos = self.pos.offset(-c.toward * 0.5).unwrap_or(self.pos);
            self.climb = None;
            self.vel = Vec3::ZERO;
            return;
        }
        let step = side + Vec3::Y * dy;
        self.pos = self.pos.offset(step).unwrap_or(self.pos);
        self.vel = Vec3::ZERO;
    }

    fn substep(&mut self, p: &mut Vec3, sub: Vec3) {
        let before = *p;
        *p += sub;
        let mut contacts = Contacts::default();
        depenetrate(&self.solids, p, &mut contacts, self.spheres(), self.up());
        let sub_h = self.flat(sub);
        let blocked = sub_h.length() > 1e-6
            && self.flat(*p - before).dot(sub_h.normalize()) < sub_h.length() * 0.5;
        if let Some(q) = (blocked && self.grounded)
            .then(|| self.try_step_up(before, sub_h))
            .flatten()
        {
            *p = q;
            return;
        }
        for n in contacts.iter() {
            self.vel -= n * self.vel.dot(n).min(0.0);
        }
    }

    /// Raise by the step height, move over the obstacle, then come back down onto it. The move
    /// goes at least a capsule radius past the edge, so the lowest sphere lands on the top of the
    /// step instead of resting on its corner, which matters for steps near `STEP_UP`.
    fn try_step_up(&self, from: Vec3, sub_h: Vec3) -> Option<Vec3> {
        let up = self.up();
        let mut q = from + up * STEP_UP;
        if penetration(&self.solids, q, self.spheres(), up) > SKIN {
            return None;
        }
        let dir = sub_h.normalize();
        q += dir * (RADIUS + 0.05).max(sub_h.length());
        let mut contacts = Contacts::default();
        depenetrate(&self.solids, &mut q, &mut contacts, self.spheres(), up);
        let moved = self.flat(q - from).dot(dir);
        if moved < sub_h.length() * 0.5 {
            return None;
        }
        let (drop, n) = probe_down(&self.solids, q, STEP_UP + 0.05, up)?;
        walkable(&self.solids, q - up * drop, n, up).then(|| q - up * drop)
    }

    /// Ground contact after the move: snap down onto walkable ground, else stay airborne.
    fn settle(&mut self, p: &mut Vec3, dt: f32) {
        let up = self.up();
        // A little slack: a frame off the axes leaves rounding noise in the normal velocity.
        let vy = self.vel.dot(up);
        let rising = vy > 0.01;
        let reach = if self.grounded && !rising {
            STEP_UP
        } else {
            0.03
        };
        match probe_down(&self.solids, *p, reach, up) {
            Some((drop, n))
                if !rising && walkable_normal(&self.solids, *p - up * drop, n, up).is_some() =>
            {
                let n = walkable_normal(&self.solids, *p - up * drop, n, up).unwrap_or(n);
                *p -= up * drop;
                self.vel -= up * vy;
                self.ground_normal = n;
                self.grounded_time = if self.grounded {
                    self.grounded_time + dt
                } else {
                    0.0
                };
                self.grounded = true;
                self.platform = platform_under(&self.solids, *p, up);
            }
            _ => {
                // Leaving a platform keeps its momentum.
                if self.grounded
                    && let Some(v) = self.platform
                {
                    self.vel += v;
                }
                self.grounded = false;
                self.grounded_time = 0.0;
                self.platform = None;
            }
        }
    }
}

/// Velocity of the moving platform just below the capsule's feet at `p`, if the ground there is
/// one: the platform flag of the nearest solid.
fn platform_under(solids: &[Solid], p: Vec3, up: Vec3) -> Option<Vec3> {
    let at = p - up * 0.01;
    solids
        .iter()
        .map(|s| (s.shape.sdf(at - s.offset), s.platform))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .and_then(|(_, platform)| platform)
}

fn max_slope_cos() -> f32 {
    MAX_SLOPE_DEG.to_radians().cos()
}

fn approach(v: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let d = target - v;
    let len = d.length();
    if len <= max_delta {
        target
    } else {
        v + d / len * max_delta
    }
}

fn centres(p: Vec3, n: usize, up: Vec3) -> impl Iterator<Item = Vec3> {
    (0..n).map(move |k| p + up * (RADIUS + k as f32 * RADIUS))
}

/// Deepest overlap of any capsule sphere with a solid, in metres (0 when clear).
fn penetration(solids: &[Solid], p: Vec3, n: usize, up: Vec3) -> f32 {
    centres(p, n, up)
        .map(|c| RADIUS - sdf(solids, c))
        .fold(0.0, f32::max)
}

#[derive(Default)]
struct Contacts {
    normals: [Vec3; 8],
    len: usize,
}

impl Contacts {
    fn push(&mut self, n: Vec3) {
        if self.len < self.normals.len() {
            self.normals[self.len] = n;
            self.len += 1;
        }
    }

    fn iter(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.normals[..self.len].iter().copied()
    }
}

fn depenetrate(solids: &[Solid], p: &mut Vec3, contacts: &mut Contacts, spheres: usize, up: Vec3) {
    for _ in 0..6 {
        let mut moved = false;
        for c in centres(*p, spheres, up) {
            let d = sdf(solids, c);
            if d < RADIUS - SKIN {
                let n = gradient(solids, c);
                *p += n * (RADIUS - d);
                contacts.push(n);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// Whether the capsule resting at `p` on a contact with normal `n` stands on walkable ground.
/// A sphere rolling over a stair edge has a tilted contact normal, so a flat surface just under
/// the capsule axis counts too.
fn walkable(solids: &[Solid], p: Vec3, n: Vec3, up: Vec3) -> bool {
    walkable_normal(solids, p, n, up).is_some()
}

/// The walkable surface normal under `p`: `n` when it is walkable, else the normal found by
/// tracing further down. A sphere touching the side of a step first reports the side's normal,
/// which must not become the ground normal or the walk direction is projected away.
fn walkable_normal(solids: &[Solid], p: Vec3, n: Vec3, up: Vec3) -> Option<Vec3> {
    if n.dot(up) >= max_slope_cos() {
        return Some(n);
    }
    let start = p + up * RADIUS;
    let mut t = 0.0;
    for _ in 0..16 {
        let at = start - up * t;
        let d = sdf(solids, at);
        if d <= 0.002 {
            let g = gradient(solids, at);
            return (g.dot(up) >= max_slope_cos()).then_some(g);
        }
        t += d;
        if t > RADIUS + 0.25 {
            break;
        }
    }
    None
}

/// Sphere-traces the lowest sphere straight down. Returns how far the capsule can drop before
/// touching and the surface normal there.
fn probe_down(solids: &[Solid], p: Vec3, max: f32, up: Vec3) -> Option<(f32, Vec3)> {
    let start = p + up * RADIUS;
    let mut t = 0.0;
    for _ in 0..24 {
        let at = start - up * t;
        let d = sdf(solids, at) - RADIUS;
        if d <= 0.002 {
            return Some((t.max(0.0), gradient(solids, at)));
        }
        t += d;
        if t > max {
            return None;
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::collision::tests::floor_box;
    use worldgen::prim::{
        Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape,
    };

    /// Primitives in sector-0 coordinates, served relative to whatever centre is asked for.
    pub(crate) struct Flat(pub(crate) Vec<Primitive>);

    impl SolidSource for Flat {
        fn gather(&self, centre: &WorldPos, _half: f32, out: &mut Vec<Solid>) {
            let offset = -centre.local();
            out.extend(self.0.iter().map(|p| Solid {
                shape: p.shape,
                offset,
                platform: None,
            }));
        }

        fn ready(&self, _pos: &WorldPos) -> bool {
            true
        }
    }

    /// A static world plus one platform whose displacement and velocity come from a function of
    /// time. The driver advances `time` before each controller step, so the platform is already
    /// where it will be at the end of that step.
    pub(crate) struct Moving {
        pub(crate) world: Vec<Primitive>,
        pub(crate) platform: Primitive,
        pub(crate) time: std::cell::Cell<f32>,
        /// Time to (displacement, velocity) of the platform.
        pub(crate) motion: Box<dyn Fn(f32) -> (Vec3, Vec3)>,
    }

    impl SolidSource for Moving {
        fn gather(&self, centre: &WorldPos, _half: f32, out: &mut Vec<Solid>) {
            let offset = -centre.local();
            out.extend(self.world.iter().map(|p| Solid {
                shape: p.shape,
                offset,
                platform: None,
            }));
            let (shift, velocity) = (self.motion)(self.time.get());
            out.push(Solid {
                shape: self.platform.shape,
                offset: offset + shift,
                platform: Some(velocity),
            });
        }

        fn ready(&self, _pos: &WorldPos) -> bool {
            true
        }
    }

    impl Moving {
        /// A 6 m by 6 m slab, 0.5 m thick with its top at y = 12, over the floor (top at y = 10),
        /// spanning x and z from 98 to 104 m at time 0.
        pub(crate) fn new(motion: impl Fn(f32) -> (Vec3, Vec3) + 'static) -> Self {
            Self {
                world: vec![floor()],
                platform: floor_box([1568, 184, 1568], [1664, 192, 1664]),
                time: std::cell::Cell::new(0.0),
                motion: Box::new(motion),
            }
        }

        pub(crate) fn shift(&self) -> Vec3 {
            (self.motion)(self.time.get()).0
        }

        /// Advances the world by `dt`, then the controller.
        pub(crate) fn step(&self, c: &mut Controller, dt: f32, input: MoveInput) {
            self.time.set(self.time.get() + dt);
            c.step(dt, input, self);
        }
    }

    fn shape(shape: Shape) -> Primitive {
        Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        }
    }

    fn q(v: [i32; 3]) -> QVec3 {
        QVec3::new(v[0], v[1], v[2])
    }

    /// A 160 m square floor whose top is y = 0, from -80 to +80 m on x and z... in sector 0
    /// coordinates, so offset by 100 m to stay non-negative: x, z in 20..180, top at y = 10.
    pub(crate) fn floor() -> Primitive {
        floor_box([320, 0, 320], [2880, 160, 2880])
    }

    pub(crate) fn at(x: f32, y: f32, z: f32) -> Controller {
        Controller::new(WorldPos::new(glam::I64Vec3::ZERO, Vec3::new(x, y, z)).unwrap())
    }

    fn run(c: &mut Controller, w: &Flat, secs: f32, wish: Vec3, run: bool) {
        let dt = 1.0 / 120.0;
        for _ in 0..(secs / dt) as u32 {
            c.step(
                dt,
                MoveInput {
                    wish,
                    run,
                    ..Default::default()
                },
                w,
            );
        }
    }

    pub(crate) fn local(c: &Controller) -> Vec3 {
        c.pos.local()
    }

    fn run_in(c: &mut Controller, w: &Flat, secs: f32, wish: Vec3, field: Field) {
        let dt = 1.0 / 120.0;
        for _ in 0..(secs / dt) as u32 {
            let input = MoveInput {
                wish,
                ..Default::default()
            };
            c.step_in(dt, input, w, field);
        }
    }

    /// GRV-13: a 20 m block with the field pointing away from its centre is walkable on all six
    /// faces, with the same speed and ground contact as on a floor.
    #[test]
    fn walking_on_all_six_faces_of_a_block() {
        let block = floor_box([1600, 1600, 1600], [1920, 1920, 1920]);
        let w = Flat(vec![block]);
        let centre = Vec3::splat(110.0);
        for n in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            let field = Field {
                up: n,
                strength: 1.0,
                current: Vec3::ZERO,
            };
            let start = centre + n * 10.05;
            let mut c = at(start.x, start.y, start.z);
            c.frame = Quat::from_rotation_arc(Vec3::Y, n);
            run_in(&mut c, &w, 1.0, Vec3::ZERO, field);
            assert!(c.grounded, "face {n}: not grounded at {:?}", local(&c));
            let off = |c: &Controller| (local(c) - centre).dot(n) - 10.0;
            assert!(off(&c).abs() < 0.02, "face {n}: {}", off(&c));
            let t = n.any_orthonormal_vector();
            let from = local(&c);
            run_in(&mut c, &w, 3.0, t, field);
            assert!(
                c.grounded,
                "face {n}: fell while walking t={t} at {:?} vel {} up {}",
                local(&c),
                c.vel,
                c.up()
            );
            assert!(off(&c).abs() < 0.05, "face {n}: {}", off(&c));
            let moved = (local(&c) - from).dot(t);
            assert!(moved > 3.0, "face {n}: moved {moved}");
        }
    }

    #[test]
    fn the_frame_turns_to_a_flipped_field_at_the_capped_rate() {
        let w = Flat(vec![]);
        let mut c = at(100.0, 100.0, 100.0);
        let flip = Field {
            up: Vec3::NEG_Y,
            strength: 1.0,
            current: Vec3::ZERO,
        };
        run_in(&mut c, &w, 1.0, Vec3::ZERO, flip);
        let turned = c.up().dot(Vec3::Y).clamp(-1.0, 1.0).acos();
        assert!(turned <= 90f32.to_radians() + 0.02, "{turned}");
        assert!(turned > 0.5, "{turned}");
        run_in(&mut c, &w, 4.0, Vec3::ZERO, flip);
        assert!(c.up().dot(Vec3::NEG_Y) > 0.999, "{:?}", c.up());
    }

    fn drift_input(thrust: f32, look: Vec3) -> MoveInput {
        MoveInput {
            look,
            thrust,
            ..Default::default()
        }
    }

    /// GRV-14: in a lull nothing pulls, the forward key pushes along the view up to 1 m/s, and
    /// letting go lets the speed decay with a 4 s time constant.
    #[test]
    fn free_drift_in_a_lull() {
        let w = Flat(vec![]);
        let lull = Field {
            up: Vec3::ZERO,
            strength: 0.0,
            current: Vec3::ZERO,
        };
        let mut c = at(100.0, 100.0, 100.0);
        let look = Vec3::new(1.0, 0.0, 0.0);
        for _ in 0..240 {
            c.step_in(1.0 / 120.0, drift_input(1.0, look), &w, lull);
        }
        // 2 s at 0.5 m/s² with the decay: 2 (1 - e^-0.5) = 0.79 m/s.
        assert!(c.vel.x > 0.75 && c.vel.x < 0.83, "{}", c.vel);
        assert!(c.vel.y.abs() < 1e-4, "gravity acted: {}", c.vel);
        assert!(!c.grounded);
        for _ in 0..240 {
            c.step_in(1.0 / 120.0, drift_input(1.0, look), &w, lull);
        }
        assert!(c.vel.x > 0.95 && c.vel.x <= 1.02, "cap: {}", c.vel);
        let v0 = c.vel.x;
        for _ in 0..480 {
            c.step_in(1.0 / 120.0, drift_input(0.0, look), &w, lull);
        }
        // 4 s of decay leaves about 1/e of the speed.
        let ratio = c.vel.x / v0;
        assert!((ratio - (-1.0f32).exp()).abs() < 0.03, "{ratio}");
        // The frame did not turn.
        assert!(c.up().dot(Vec3::Y) > 0.9999);
    }

    /// WGT-03: in a drift lane the speed relaxes to the current, with nothing pushing.
    #[test]
    fn a_drift_lane_carries_a_drifter() {
        let w = Flat(vec![]);
        let lane = Field {
            up: Vec3::ZERO,
            strength: 0.0,
            current: Vec3::new(1.5, 0.0, 0.0),
        };
        let mut c = at(100.0, 100.0, 100.0);
        for _ in 0..(20 * 120) {
            c.step_in(1.0 / 120.0, drift_input(0.0, Vec3::X), &w, lane);
        }
        assert!((c.vel.x - 1.5).abs() < 0.05, "{}", c.vel);
        assert!(local(&c).x > 120.0, "carried {} m", local(&c).x - 100.0);
    }

    #[test]
    fn a_surface_stops_a_drifter_without_a_bounce() {
        let wall = floor_box([1700, 1000, 1000], [1760, 3000, 3000]);
        let w = Flat(vec![wall]);
        let lull = Field {
            up: Vec3::ZERO,
            strength: 0.0,
            current: Vec3::ZERO,
        };
        let mut c = at(100.0, 100.0, 100.0);
        for _ in 0..1200 {
            c.step_in(1.0 / 120.0, drift_input(1.0, Vec3::X), &w, lull);
        }
        // The wall's face is at x = 106.25; the capsule stays outside it and is not moving in.
        assert!(local(&c).x < 106.25 - RADIUS + 0.01, "{:?}", local(&c));
        assert!(c.vel.x <= 0.05, "{}", c.vel);
    }

    #[test]
    fn weaker_gravity_falls_proportionally_slower() {
        let w = Flat(vec![]);
        let half = Field {
            up: Vec3::Y,
            strength: 0.5,
            current: Vec3::ZERO,
        };
        let mut c = at(100.0, 100.0, 100.0);
        run_in(&mut c, &w, 1.0, Vec3::ZERO, half);
        assert!((c.vel.y + GRAVITY * 0.5).abs() < 0.1, "{}", c.vel.y);
    }

    #[test]
    fn standing_on_a_floor_is_stable() {
        let w = Flat(vec![floor()]);
        let mut c = at(100.0, 10.5, 100.0);
        run(&mut c, &w, 2.0, Vec3::ZERO, false);
        assert!(c.grounded);
        assert!((local(&c).y - 10.0).abs() < 0.01, "{:?}", local(&c));
        assert!(c.grounded_time > 1.5);
    }

    #[test]
    fn walk_speed_and_acceleration_follow_the_spec() {
        let w = Flat(vec![floor()]);
        let mut c = at(100.0, 10.0, 100.0);
        run(&mut c, &w, 0.2, Vec3::X, false);
        // Ease-in: after 0.2 s well below walk speed.
        assert!(c.vel.x > 0.2 && c.vel.x < WALK_SPEED * 0.9, "{}", c.vel.x);
        run(&mut c, &w, 1.0, Vec3::X, false);
        assert!((c.vel.x - WALK_SPEED).abs() < 1e-3);
        let x0 = local(&c).x;
        run(&mut c, &w, 1.0, Vec3::X, true);
        assert!((c.vel.x - RUN_SPEED).abs() < 1e-3);
        assert!(local(&c).x - x0 > 1.4);
        // Stops within about 0.25 s.
        run(&mut c, &w, 0.3, Vec3::ZERO, false);
        assert!(c.vel.length() < 1e-3);
    }

    #[test]
    fn walls_stop_and_slide() {
        // A wall at x = 105 (2 m thick, 4 m tall), spanning z.
        let wall = floor_box([1680, 160, 320], [1712, 224, 2880]);
        let w = Flat(vec![floor(), wall]);
        let mut c = at(100.0, 10.0, 100.0);
        run(&mut c, &w, 6.0, Vec3::X, false);
        assert!(local(&c).x <= 105.0 - RADIUS + 0.01, "{:?}", local(&c));
        assert!(local(&c).x > 104.0);
        // Pushing diagonally into the wall still slides along it.
        let z0 = local(&c).z;
        run(&mut c, &w, 2.0, Vec3::new(1.0, 0.0, 1.0).normalize(), false);
        assert!(local(&c).z - z0 > 1.0, "{:?}", local(&c));
        assert!(local(&c).x <= 105.0 - RADIUS + 0.01);
    }

    #[test]
    fn climbs_and_descends_stairs_without_leaving_the_ground() {
        // 8 steps of 3 Q rise and 5 Q tread: 0.1875 m by 0.3125 m, rising along +x from x = 105.
        let stairs = shape(Shape::stair_run(
            QAabb::new(q([1680, 160, 1500]), q([1720, 184, 1540])),
            Dir4::PosX,
            8,
        ));
        let landing = floor_box([1720, 160, 1500], [2200, 184, 1540]);
        let w = Flat(vec![floor(), stairs, landing]);
        let mut c = at(103.0, 10.0, 94.5);
        run(&mut c, &w, 2.0, Vec3::X, false);
        run(&mut c, &w, 3.0, Vec3::X, true);
        let top = 10.0 + 24.0 / 16.0;
        assert!((local(&c).y - top).abs() < 0.02, "{:?}", local(&c));
        // Back down: never airborne for long, ends on the floor.
        let dt = 1.0 / 120.0;
        let mut airborne = 0;
        for _ in 0..(14.0 / dt) as u32 {
            c.step(
                dt,
                MoveInput {
                    wish: Vec3::NEG_X,
                    run: false,
                    ..Default::default()
                },
                &w,
            );
            airborne += u32::from(!c.grounded);
        }
        assert!(airborne < 12, "airborne {airborne} frames on the way down");
        assert!(
            (local(&c).y - 10.0).abs() < 0.02 && local(&c).x < 105.0,
            "{:?}",
            local(&c)
        );
    }

    /// A 10 m wide slab along the table direction `d`, its low end at (110, 10, 100) on the
    /// floor, its top surface rising at the slope of `d`.
    fn oriented_slab(d: [i8; 3]) -> Primitive {
        use worldgen::orient::{Frame, LocalShape};
        let q = |v: [i32; 3]| QVec3::new(v[0], v[1], v[2]);
        shape(Shape::Oriented(worldgen::orient::Oriented {
            frame: Frame::dir(d),
            origin: q([110 * 16, 10 * 16, 100 * 16]),
            local: LocalShape::Box {
                aabb: QAabb::new(q([0, -8, -80]), q([40 * 16, 0, 80])),
            },
            clip: QAabb::new(q([0, 0, 0]), q([4096 * 16; 3])),
        }))
    }

    /// TEC-05: the capsule climbs a tilted oriented slab within the slope limit and is stopped
    /// by one steeper than 40 degrees, exactly as with the axis-aligned ramps.
    #[test]
    fn oriented_slabs_follow_the_slope_limit() {
        // (2,1,0) rises 26.6 degrees; (1,1,0) rises 45.
        let gentle = Flat(vec![floor(), oriented_slab([2, 1, 0])]);
        let mut up = at(103.0, 10.0, 100.0);
        run(&mut up, &gentle, 7.0, Vec3::X, true);
        assert!(up.grounded, "{:?}", local(&up));
        assert!(
            local(&up).y > 16.0,
            "gentle oriented slab not climbed: {:?}",
            local(&up)
        );
        let steep = Flat(vec![floor(), oriented_slab([1, 1, 0])]);
        let mut bad = at(103.0, 10.0, 100.0);
        run(&mut bad, &steep, 7.0, Vec3::X, true);
        assert!(
            local(&bad).y < 11.0,
            "steep oriented slab climbed: {:?}",
            local(&bad)
        );
    }

    /// A rotated box is a solid wall to the capsule: it cannot be walked through at an angle.
    #[test]
    fn rotated_walls_stop_the_capsule() {
        use worldgen::orient::{Frame, LocalShape};
        let q = |v: [i32; 3]| QVec3::new(v[0], v[1], v[2]);
        // A wall 2 m thick, 4 m tall, 60 m long, turned 30 degrees about Y (heading 5.33 ~ 5).
        let wall = shape(Shape::Oriented(worldgen::orient::Oriented {
            frame: Frame::table(5, 0),
            origin: q([110 * 16, 10 * 16, 100 * 16]),
            local: LocalShape::Box {
                aabb: QAabb::new(q([-30 * 16, 0, -16]), q([30 * 16, 4 * 16, 16])),
            },
            clip: QAabb::new(q([0, 0, 0]), q([4096 * 16; 3])),
        }));
        let w = Flat(vec![floor(), wall]);
        let mut c = at(100.0, 10.0, 90.0);
        run(&mut c, &w, 12.0, Vec3::new(1.0, 0.0, 1.0).normalize(), true);
        // Still on the near side of the wall's centre line: the capsule never got through.
        let [ex, _, ez] = worldgen::orient::Frame::table(5, 0).axes();
        let _ = ex;
        let n = Vec3::new(ez.x as f32, 0.0, ez.z as f32);
        let side = (local(&c) - Vec3::new(110.0, 10.0, 100.0)).dot(n);
        let start = (Vec3::new(100.0, 10.0, 90.0) - Vec3::new(110.0, 10.0, 100.0)).dot(n);
        assert!(side * start > 0.0, "crossed the wall: {:?}", local(&c));
    }

    #[test]
    fn gentle_ramps_are_climbed_and_steep_ones_are_not() {
        // 5 m run over 2.5 m: about 27 degrees. And 3 m run over 3.5 m: about 49 degrees.
        let gentle = shape(Shape::Ramp {
            aabb: QAabb::new(q([1680, 160, 800]), q([1760, 200, 840])),
            rise: Dir4::PosX,
        });
        let steep = shape(Shape::Ramp {
            aabb: QAabb::new(q([1680, 160, 2000]), q([1728, 216, 2040])),
            rise: Dir4::PosX,
        });
        let w = Flat(vec![floor(), gentle, steep]);
        let mut up = at(103.0, 10.0, 51.25);
        run(&mut up, &w, 6.0, Vec3::X, false);
        assert!(
            local(&up).y > 11.5,
            "gentle ramp not climbed: {:?}",
            local(&up)
        );
        let mut bad = at(103.0, 10.0, 126.25);
        run(&mut bad, &w, 6.0, Vec3::X, false);
        assert!(
            local(&bad).y < 11.0,
            "steep ramp climbed: {:?}",
            local(&bad)
        );
    }

    #[test]
    fn stepping_off_a_ledge_falls() {
        // A 2 m slab top at y = 12 over the floor, edge at x = 110.
        let slab = floor_box([1440, 160, 320], [1760, 192, 2880]);
        let w = Flat(vec![floor(), slab]);
        let mut c = at(108.0, 12.0, 100.0);
        run(&mut c, &w, 4.0, Vec3::X, false);
        // Fell off the 2 m ledge (more than the step-down) and landed on the floor.
        assert!((local(&c).y - 10.0).abs() < 0.02, "{:?}", local(&c));
        assert!(c.grounded);
    }

    #[test]
    fn edge_standing_does_not_jitter() {
        let slab = floor_box([1440, 160, 320], [1760, 192, 2880]);
        let w = Flat(vec![floor(), slab]);
        // Capsule centre 0.1 m past the edge: still supported by the rim of its lowest sphere.
        let mut c = at(110.1, 12.0, 100.0);
        run(&mut c, &w, 1.0, Vec3::ZERO, false);
        let start = local(&c);
        let (mut lo, mut hi) = (start, start);
        for _ in 0..240 {
            c.step(1.0 / 120.0, MoveInput::default(), &w);
            lo = lo.min(local(&c));
            hi = hi.max(local(&c));
        }
        assert!((hi - lo).max_element() < 1e-3, "{lo:?} {hi:?}");
    }

    #[test]
    fn does_not_move_until_collision_exists() {
        struct NotReady;
        impl SolidSource for NotReady {
            fn gather(&self, _: &WorldPos, _: f32, _: &mut Vec<Solid>) {}
            fn ready(&self, _: &WorldPos) -> bool {
                false
            }
        }
        let mut c = at(100.0, 10.0, 100.0);
        for _ in 0..120 {
            c.step(1.0 / 120.0, MoveInput::default(), &NotReady);
        }
        assert_eq!(local(&c), Vec3::new(100.0, 10.0, 100.0));
    }

    mod real_world {
        use super::*;
        use crate::collision::{CollisionChunk, CollisionWorld};
        use crate::stream::{ChunkId, Detail};
        use glam::I64Vec3;
        use worldgen::cell::{CellKey, Level};
        use worldgen::l1::l1_cell;
        use worldgen::l2::l2_cell;
        use worldgen::seed::WorldSeed;

        /// Loads the L1 cell and every L2 cell of its 3x3x3 neighbourhood that touches `around`.
        pub(super) fn world_around(seed: WorldSeed, around: &WorldPos) -> CollisionWorld {
            let mut world = CollisionWorld::default();
            let l1 = CellKey::containing(Level::L1, around);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let k = CellKey::new(Level::L1, l1.coords + I64Vec3::new(dx, 0, dz));
                    let out = l1_cell(seed, k);
                    let origin = k.aabb_world().unwrap().min;
                    world.insert(
                        ChunkId::l1(k.coords, Detail::Full),
                        CollisionChunk::new(origin, 4096.0, &out.primitives),
                    );
                }
            }
            let l2 = CellKey::containing(Level::L2, around);
            for d in [-1, 0, 1] {
                for e in [-1, 0, 1] {
                    for f in [-1, 0, 1] {
                        let k = CellKey::new(Level::L2, l2.coords + I64Vec3::new(d, e, f));
                        let out = l2_cell(seed, k);
                        let origin = k.aabb_world().unwrap().min;
                        world.insert(
                            ChunkId::l2(k.coords),
                            CollisionChunk::new(origin, 512.0, &out.primitives),
                        );
                    }
                }
            }
            world
        }

        /// A point on top of the widest deck box in the first L1 cell near the origin that has one.
        pub(super) fn deck_spot(seed: WorldSeed) -> (WorldPos, f32) {
            use worldgen::cell::Axis;
            for y in -1..=1 {
                for z in -1..=1 {
                    for x in -1..=1 {
                        let k = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                        let out = l1_cell(seed, k);
                        let origin = k.aabb_world().unwrap().min;
                        let deck = out
                            .primitives
                            .iter()
                            .filter(|p| p.kind == FeatureKind::DECK)
                            .filter_map(|p| match p.shape {
                                Shape::Box { aabb } => Some(aabb),
                                _ => None,
                            })
                            .max_by_key(|a| {
                                i64::from(a.size(Axis::X)) * i64::from(a.size(Axis::Z))
                            });
                        if let Some(deck) = deck {
                            let (lo, hi) = (deck.min_m(), deck.max_m());
                            let mid = (lo + hi) * 0.5;
                            let spot = origin.offset(Vec3::new(mid.x, hi.y, mid.z)).unwrap();
                            println!("deck in L1 {x} {y} {z}: {lo:?}..{hi:?}");
                            return (spot, hi.x - lo.x);
                        }
                    }
                }
            }
            panic!("no L1 deck near the origin");
        }

        #[test]
        #[ignore = "timing; run with --release -- --ignored --nocapture"]
        fn collision_query_and_step_cost() {
            let seed = WorldSeed(0);
            let (spot, _) = deck_spot(seed);
            let world = world_around(seed, &spot);
            let mut solids = Vec::new();
            let n = 20_000;
            let t = std::time::Instant::now();
            for _ in 0..n {
                solids.clear();
                world.gather(&spot, 3.0, &mut solids);
            }
            let gather = t.elapsed().as_secs_f64() * 1e3 / f64::from(n);
            let mut c = Controller::new(spot);
            let t = std::time::Instant::now();
            for _ in 0..n {
                c.step(
                    1.0 / 120.0,
                    MoveInput {
                        wish: Vec3::X,
                        run: false,
                        ..Default::default()
                    },
                    &world,
                );
            }
            let step = t.elapsed().as_secs_f64() * 1e3 / f64::from(n);
            println!(
                "{} chunks, {} solids in range: gather {gather:.4} ms, full step {step:.4} ms",
                world.len(),
                solids.len()
            );
        }

        #[test]
        #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
        fn lands_on_a_real_deck_and_walks_it() {
            let seed = WorldSeed(0);
            let (spot, width) = deck_spot(seed);
            let world = world_around(seed, &spot);
            let mut c = Controller::new(spot.offset(Vec3::Y * 1.0).unwrap());
            let dt = 1.0 / 120.0;
            for _ in 0..240 {
                c.step(dt, MoveInput::default(), &world);
            }
            assert!(c.grounded, "did not land on the deck");
            let rest = c.pos.delta(&spot).y;
            assert!(rest.abs() < 0.02, "rests {rest} m off the deck top");
            // Walk along x for 10 s (at most 14 m): the deck is wider than that near its middle.
            assert!(width > 30.0, "deck too narrow for the test: {width}");
            let start = c.pos;
            for _ in 0..1200 {
                c.step(
                    dt,
                    MoveInput {
                        wish: Vec3::X,
                        run: false,
                        ..Default::default()
                    },
                    &world,
                );
            }
            assert!(c.grounded);
            assert!(c.pos.delta(&start).x > 10.0);
            assert!(c.pos.delta(&spot).y.abs() < 0.3);
        }
    }

    const DT: f32 = 1.0 / 120.0;

    #[test]
    fn rides_a_platform_moving_sideways() {
        let w = Moving::new(|t| (Vec3::X * 5.0 * t, Vec3::X * 5.0));
        let mut c = at(100.0, 12.0, 100.0);
        for _ in 0..(4.0 / DT) as u32 {
            w.step(&mut c, DT, MoveInput::default());
            assert!(c.grounded && c.on_platform());
        }
        // 20 m along, still in the middle of the slab.
        let rel = local(&c) - w.shift();
        assert!(
            (rel.x - 100.0).abs() < 0.05 && (rel.z - 100.0).abs() < 0.05,
            "{rel:?}"
        );
        assert!((local(&c).y - 12.0).abs() < 0.02);
    }

    #[test]
    fn rides_a_lift_up_and_down_without_losing_the_ground() {
        // Up at 4 m/s for 3 s, then down at 4 m/s for 3 s.
        let w = Moving::new(|t| {
            if t < 3.0 {
                (Vec3::Y * 4.0 * t, Vec3::Y * 4.0)
            } else {
                (
                    Vec3::Y * (12.0 - 4.0 * (t - 3.0)).max(0.0),
                    Vec3::NEG_Y * 4.0,
                )
            }
        });
        let mut c = at(100.0, 12.0, 100.0);
        for _ in 0..(6.0 / DT) as u32 {
            w.step(&mut c, DT, MoveInput::default());
            assert!(c.grounded, "left the lift at t = {}", w.time.get());
            let height = local(&c).y - 12.0 - w.shift().y;
            assert!(
                height.abs() < 0.03,
                "{height} off the deck at t = {}",
                w.time.get()
            );
        }
        assert!((local(&c).y - 12.0).abs() < 0.05);
    }

    #[test]
    fn walking_on_a_platform_adds_to_its_motion() {
        let w = Moving::new(|t| (Vec3::X * 5.0 * t, Vec3::X * 5.0));
        let mut c = at(99.0, 12.0, 100.0);
        for _ in 0..(2.0 / DT) as u32 {
            w.step(
                &mut c,
                DT,
                MoveInput {
                    wish: Vec3::X,
                    run: false,
                    ..Default::default()
                },
            );
        }
        // Relative to the slab the player walked at the usual speed; the controller's own
        // velocity does not include the platform's.
        let rel = local(&c) - w.shift();
        assert!(rel.x - 99.0 > 1.4 && rel.x - 99.0 < 2.8, "{rel:?}");
        assert!((c.vel.x - WALK_SPEED).abs() < 1e-3, "{:?}", c.vel);
        assert!(c.on_platform());
    }

    #[test]
    fn stepping_off_keeps_the_platforms_momentum() {
        let w = Moving::new(|t| (Vec3::X * 5.0 * t, Vec3::X * 5.0));
        let mut c = at(102.0, 12.0, 100.0);
        let wish = MoveInput {
            wish: Vec3::X,
            run: false,
            ..Default::default()
        };
        let mut left_at = None;
        for _ in 0..(3.0 / DT) as u32 {
            w.step(&mut c, DT, wish);
            if left_at.is_none() && !c.grounded {
                left_at = Some(c.vel);
            }
        }
        let vel = left_at.expect("walked off the front edge");
        assert!(vel.x > 5.0, "momentum lost: {vel:?}");
        // The floor is 2 m below: it lands there and is no longer on a platform.
        assert!(c.grounded && !c.on_platform(), "{:?}", local(&c));
        assert!((local(&c).y - 10.0).abs() < 0.05);
    }

    #[test]
    fn rides_a_tram_at_one_hundred_metres_per_second() {
        let w = Moving::new(|t| (Vec3::X * 100.0 * t, Vec3::X * 100.0));
        let mut c = at(100.0, 12.0, 100.0);
        // One step before the tram moves, so the controller already knows it stands on it.
        c.step(DT, MoveInput::default(), &w);
        for _ in 0..(5.0 / DT) as u32 {
            w.step(&mut c, DT, MoveInput::default());
            assert!(c.grounded);
        }
        let rel = local(&c) - w.shift();
        assert!((rel.x - 100.0).abs() < 0.05, "{rel:?}");
    }

    /// A ladder in sector-0 coordinates over a floor and a roof, for the climbing tests.
    struct LadderWorld {
        solids: Vec<Primitive>,
        ladder: LadderVol,
    }

    impl SolidSource for LadderWorld {
        fn gather(&self, centre: &WorldPos, _half: f32, out: &mut Vec<Solid>) {
            let offset = -centre.local();
            out.extend(self.solids.iter().map(|p| Solid {
                shape: p.shape,
                offset,
                platform: None,
            }));
        }

        fn ready(&self, _pos: &WorldPos) -> bool {
            true
        }

        fn ladders(&self, centre: &WorldPos, _half: f32, out: &mut Vec<LadderVol>) {
            out.push(LadderVol {
                centre: self.ladder.centre - centre.local(),
                ..self.ladder
            });
        }
    }

    /// A 10 m wall at x 12..13 with a ladder on its -x face, a floor below and a roof on top.
    fn ladder_world() -> LadderWorld {
        LadderWorld {
            solids: vec![
                floor_box([0, 0, 0], [16 * 24, 16, 16 * 24]),
                floor_box([16 * 12, 0, 16 * 8], [16 * 13, 16 * 10, 16 * 16]),
                floor_box([16 * 12, 16 * 10 - 16, 16 * 8], [16 * 20, 16 * 10, 16 * 16]),
            ],
            // Corridor 0.9 wide, 0.9 deep, y 1 to 10 m, in front of the wall (rungs at +x).
            ladder: LadderVol {
                centre: Vec3::new(11.55, 5.5, 12.0),
                axes: [Vec3::X, Vec3::Y, Vec3::Z],
                half: Vec3::new(0.45, 4.5, 0.45),
                toward: Vec3::X,
            },
        }
    }

    fn at_sector0(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(glam::I64Vec3::ZERO, Vec3::new(x, y, z)).unwrap()
    }

    /// CLB-01 to CLB-05: walking into the rungs grabs the ladder, forward climbs at 0.8 m/s, you
    /// cannot step off in the middle, and the top puts you on the roof.
    #[test]
    fn a_ladder_is_grabbed_climbed_and_left_at_the_top() {
        let world = ladder_world();
        let mut c = Controller::new(at_sector0(10.0, 1.0, 12.0));
        // Walk to the ladder.
        for _ in 0..300 {
            c.step(
                1.0 / 60.0,
                MoveInput {
                    wish: Vec3::X,
                    ..Default::default()
                },
                &world,
            );
            if c.on_ladder() {
                break;
            }
        }
        assert!(c.on_ladder(), "not grabbed at {:?}", c.pos.local());
        let y0 = c.pos.local().y;
        for _ in 0..60 {
            c.step(
                1.0 / 60.0,
                MoveInput {
                    climb: 1.0,
                    wish: Vec3::NEG_X,
                    ..Default::default()
                },
                &world,
            );
        }
        let rise = c.pos.local().y - y0;
        assert!(
            (rise - LADDER_SPEED).abs() < 0.05,
            "rose {rise} m in a second"
        );
        assert!(c.on_ladder(), "stepping back must not let go");
        assert!(
            c.pos.local().x > 11.0,
            "held against the rungs: {:?}",
            c.pos.local()
        );
        for _ in 0..60 * 15 {
            c.step(
                1.0 / 60.0,
                MoveInput {
                    climb: 1.0,
                    ..Default::default()
                },
                &world,
            );
            if !c.on_ladder() {
                break;
            }
        }
        assert!(!c.on_ladder());
        let p = c.pos.local();
        assert!(p.x > 12.0 && (p.y - 10.0).abs() < 0.2, "on the roof: {p:?}");
    }

    /// Down the ladder and off at the bottom.
    #[test]
    fn climbing_down_ends_on_the_floor() {
        let world = ladder_world();
        let mut c = Controller::new(at_sector0(11.4, 6.0, 12.0));
        c.climb = Some(Climb {
            centre: at_sector0(11.55, 5.5, 12.0),
            half_y: 4.5,
            half_toward: 0.45,
            toward: Vec3::X,
        });
        for _ in 0..60 * 12 {
            c.step(
                1.0 / 60.0,
                MoveInput {
                    climb: -1.0,
                    ..Default::default()
                },
                &world,
            );
            if !c.on_ladder() {
                break;
            }
        }
        assert!(!c.on_ladder());
        assert!(c.pos.local().y < 1.6, "{:?}", c.pos.local());
    }

    /// CLB-07: a duct with a 1.4 m ceiling is entered crouched and left standing.
    #[test]
    fn a_low_duct_makes_you_crouch_and_standing_resumes_after() {
        let world = Flat(vec![
            floor_box([0, 0, 0], [16 * 30, 16, 16 * 10]),
            // The duct: ceiling at 1.4 m from x 8 to 14.
            floor_box([16 * 8, 38, 0], [16 * 14, 16 * 4, 16 * 10]),
        ]);
        let mut c = Controller::new(at_sector0(4.0, 1.0, 5.0));
        let (mut saw_crouch, mut inside) = (false, false);
        for _ in 0..60 * 14 {
            c.step(
                1.0 / 60.0,
                MoveInput {
                    wish: Vec3::X,
                    ..Default::default()
                },
                &world,
            );
            let x = c.pos.local().x;
            if (9.0..13.0).contains(&x) {
                saw_crouch |= c.crouched;
                inside = true;
                assert!(c.eye_height() < 1.2);
            }
            if x > 18.0 {
                break;
            }
        }
        assert!(
            inside && saw_crouch,
            "inside {inside} crouch {saw_crouch} at {:?}",
            c.pos.local()
        );
        assert!(c.pos.local().x > 14.5, "went through: {:?}", c.pos.local());
        assert!(!c.crouched, "standing again in the open");
    }
}
