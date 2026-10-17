//! First-person walking controller: a kinematic capsule with step-up, step-down snap, a slope
//! limit and no jump (`docs/spec/traversal.md`). Pure logic; solids come from a [`SolidSource`].
//!
//! The capsule is a chain of spheres, pushed out of the union of solids along the signed
//! distance gradient.

// Substep and frame counts are small non-negative floats.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use glam::Vec3;
use worldgen::coord::WorldPos;

use crate::collision::{Solid, SolidSource, gradient, sdf};

pub const RADIUS: f32 = 0.3;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.65;
pub const STEP_UP: f32 = 0.45;
pub const WALK_SPEED: f32 = 1.4;
pub const RUN_SPEED: f32 = 4.0;
pub const GRAVITY: f32 = 9.81;
const MAX_SLOPE_DEG: f32 = 40.0;
const ACCEL_TIME: f32 = 0.35;
const DECEL_TIME: f32 = 0.25;
const AIR_CONTROL: f32 = 0.1;
/// Largest move between collision resolutions, well under the capsule radius.
const MAX_SUBSTEP: f32 = 0.1;
const SPHERES: usize = 5;
const SKIN: f32 = 1e-3;

#[derive(Debug, Clone, Copy, Default)]
pub struct MoveInput {
    /// Desired horizontal direction in the world, length at most 1.
    pub wish: Vec3,
    pub run: bool,
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
    solids: Vec<Solid>,
}

impl Controller {
    pub fn new(pos: WorldPos) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            grounded: false,
            ground_normal: Vec3::Y,
            grounded_time: 0.0,
            solids: Vec::new(),
        }
    }

    pub fn eye(&self) -> WorldPos {
        self.pos.offset(Vec3::Y * EYE_HEIGHT).unwrap_or(self.pos)
    }

    /// Moves the controller so its eye is at `eye`.
    pub fn set_eye(&mut self, eye: WorldPos) {
        self.pos = eye.offset(Vec3::NEG_Y * EYE_HEIGHT).unwrap_or(eye);
        self.vel = Vec3::ZERO;
        self.grounded = false;
        self.grounded_time = 0.0;
    }

    /// Advances by `dt` seconds. Does nothing until collision data exists around the player.
    pub fn step(&mut self, dt: f32, input: MoveInput, src: &dyn SolidSource) {
        crate::zone!("controller step");
        if !src.ready(&self.pos) {
            return;
        }
        let top_speed = if input.run { RUN_SPEED } else { WALK_SPEED };
        let wish = Vec3::new(input.wish.x, 0.0, input.wish.z).clamp_length_max(1.0);
        let target = wish * top_speed;
        let mut rate = if wish == Vec3::ZERO {
            RUN_SPEED / DECEL_TIME
        } else {
            top_speed / ACCEL_TIME
        };
        if !self.grounded {
            rate *= AIR_CONTROL;
        }
        let vh = approach(Vec3::new(self.vel.x, 0.0, self.vel.z), target, rate * dt);
        let vy = if self.grounded {
            0.0
        } else {
            self.vel.y - GRAVITY * dt
        };
        self.vel = vh + Vec3::Y * vy;

        let travel = self.vel.length() * dt;
        self.solids.clear();
        src.gather(&self.pos, 3.0 + travel, &mut self.solids);
        let mut p = Vec3::ZERO;
        let mut disp = self.vel * dt;
        if self.grounded {
            // Follow the ground plane so slopes are climbed at walking speed.
            let n = self.ground_normal;
            disp = Vec3::new(disp.x, 0.0, disp.z);
            disp -= n * disp.dot(n) * 1.0;
            disp = disp.clamp_length_max(vh.length() * dt * 1.5 + 1e-6);
        } else {
            disp.y = vy * dt;
        }
        let substeps = (disp.length() / MAX_SUBSTEP).ceil().max(1.0) as u32;
        let sub = disp / substeps as f32;
        for _ in 0..substeps {
            self.substep(&mut p, sub);
        }
        self.settle(&mut p, dt);
        self.pos = self.pos.offset(p).unwrap_or(self.pos);
    }

    fn substep(&mut self, p: &mut Vec3, sub: Vec3) {
        let before = *p;
        *p += sub;
        let mut contacts = Contacts::default();
        depenetrate(&self.solids, p, &mut contacts);
        let sub_h = Vec3::new(sub.x, 0.0, sub.z);
        let blocked = sub_h.length() > 1e-6
            && Vec3::new(p.x - before.x, 0.0, p.z - before.z).dot(sub_h.normalize())
                < sub_h.length() * 0.5;
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

    /// Raise by the step height, move over the obstacle, then come back down onto it.
    fn try_step_up(&self, from: Vec3, sub_h: Vec3) -> Option<Vec3> {
        let mut q = from + Vec3::Y * STEP_UP;
        if penetration(&self.solids, q) > SKIN {
            return None;
        }
        q += sub_h;
        let mut contacts = Contacts::default();
        depenetrate(&self.solids, &mut q, &mut contacts);
        let moved = Vec3::new(q.x - from.x, 0.0, q.z - from.z).dot(sub_h.normalize());
        if moved < sub_h.length() * 0.5 {
            return None;
        }
        let (drop, n) = probe_down(&self.solids, q, STEP_UP + 0.05)?;
        walkable(&self.solids, q - Vec3::Y * drop, n).then(|| q - Vec3::Y * drop)
    }

    /// Ground contact after the move: snap down onto walkable ground, else stay airborne.
    fn settle(&mut self, p: &mut Vec3, dt: f32) {
        let reach = if self.grounded && self.vel.y <= 0.0 {
            STEP_UP
        } else {
            0.03
        };
        match probe_down(&self.solids, *p, reach) {
            Some((drop, n))
                if self.vel.y <= 0.0 && walkable(&self.solids, *p - Vec3::Y * drop, n) =>
            {
                p.y -= drop;
                self.vel.y = 0.0;
                self.ground_normal = n;
                self.grounded_time = if self.grounded {
                    self.grounded_time + dt
                } else {
                    0.0
                };
                self.grounded = true;
            }
            _ => {
                self.grounded = false;
                self.grounded_time = 0.0;
            }
        }
    }
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

fn centres(p: Vec3) -> impl Iterator<Item = Vec3> {
    (0..SPHERES).map(move |k| p + Vec3::Y * (RADIUS + k as f32 * RADIUS))
}

/// Deepest overlap of any capsule sphere with a solid, in metres (0 when clear).
fn penetration(solids: &[Solid], p: Vec3) -> f32 {
    centres(p)
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

fn depenetrate(solids: &[Solid], p: &mut Vec3, contacts: &mut Contacts) {
    for _ in 0..6 {
        let mut moved = false;
        for c in centres(*p) {
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
fn walkable(solids: &[Solid], p: Vec3, n: Vec3) -> bool {
    if n.y >= max_slope_cos() {
        return true;
    }
    let start = p + Vec3::Y * RADIUS;
    let mut t = 0.0;
    for _ in 0..16 {
        let at = start - Vec3::Y * t;
        let d = sdf(solids, at);
        if d <= 0.002 {
            return gradient(solids, at).y >= max_slope_cos();
        }
        t += d;
        if t > RADIUS + 0.25 {
            break;
        }
    }
    false
}

/// Sphere-traces the lowest sphere straight down. Returns how far the capsule can drop before
/// touching and the surface normal there.
fn probe_down(solids: &[Solid], p: Vec3, max: f32) -> Option<(f32, Vec3)> {
    let start = p + Vec3::Y * RADIUS;
    let mut t = 0.0;
    for _ in 0..24 {
        let at = start - Vec3::Y * t;
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
            }));
        }

        fn ready(&self, _pos: &WorldPos) -> bool {
            true
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
            c.step(dt, MoveInput { wish, run }, w);
        }
    }

    pub(crate) fn local(c: &Controller) -> Vec3 {
        c.pos.local()
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
                    },
                    &world,
                );
            }
            assert!(c.grounded);
            assert!(c.pos.delta(&start).x > 10.0);
            assert!(c.pos.delta(&spot).y.abs() < 0.3);
        }
    }
}
