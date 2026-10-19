//! Tram sessions and cars at runtime (`docs/spec/trams.md` §5-§7).
//!
//! Game state only. Lines come from `worldgen::tram`, cars from the pure schedule in `tram`.

use glam::{DVec3, I64Vec3, Vec3};
use worldgen::cell::{Axis, CellKey, Level};
use worldgen::coord::WorldPos;
use worldgen::key::LayerKey;
use worldgen::l1::DECK_REGION_LOG2;
use worldgen::prim::{QAabb, QVec3, Shape};
use worldgen::seed::WorldSeed;
use worldgen::tram::{TramLine, tram_lines};

use crate::collision::{DynamicSolid, SolidSource};
use crate::controller::Controller;
use crate::stream::Probe;
use crate::tram::{CAR_LENGTH, CarState, LANE_WIDTH, Schedule};

pub const ATTACH: f64 = 4096.0;
pub const DETACH: f64 = 5120.0;
/// Cars within this distance of the player get collision solids, metres.
pub const GATHER_RADIUS: f64 = 64.0;
pub const LOOKAHEAD_S: f64 = 30.0;
pub const PROBE_SPACING_M: f64 = 256.0;
pub const SIM_DT: f64 = 1.0 / 120.0;
const CAPSULE_RADIUS: f64 = 0.3;
const PUSH_CLEARANCE: f64 = 0.05;
const Q: f64 = 16.0;
/// Floor top above the deck, metres (7 Q).
pub const FLOOR_TOP: f64 = 7.0 / Q;
const ABOARD_TOL: f64 = 0.1;
const CAPTION_TIME: f32 = 2.5;

/// One line near the player: its geometry and schedule.
pub struct Session {
    pub line: TramLine,
    pub schedule: Schedule,
    origin: WorldPos,
    plane_m: f64,
    top_m: f64,
}

/// Where a point is relative to a line: along, across from the plane, above the deck top.
#[derive(Debug, Clone, Copy)]
pub struct LineFrame {
    pub along: f64,
    pub cross: f64,
    pub height: f64,
}

impl Session {
    pub fn new(line: TramLine) -> Option<Self> {
        let first = CellKey::new(
            Level::L1,
            match line.axis {
                Axis::X => I64Vec3::new(line.cells.0, line.id.layer.y, line.column),
                _ => I64Vec3::new(line.column, line.id.layer.y, line.cells.0),
            },
        );
        let origin = first.aabb_world().ok()?.min;
        Some(Self {
            schedule: Schedule::new(&line),
            plane_m: f64::from(line.plane) / Q,
            top_m: f64::from(line.top) / Q,
            line,
            origin,
        })
    }

    /// A point of the line frame in the world.
    pub fn world(&self, along: f64, cross: f64, height: f64) -> WorldPos {
        let c = self.plane_m + cross;
        let h = self.top_m + height;
        let v = match self.line.axis {
            Axis::X => Vec3::new(along as f32, h as f32, c as f32),
            _ => Vec3::new(c as f32, h as f32, along as f32),
        };
        self.origin.offset(v).unwrap_or(self.origin)
    }

    pub fn frame_of(&self, p: &WorldPos) -> LineFrame {
        let d: DVec3 = p.delta(&self.origin);
        let (along, c) = match self.line.axis {
            Axis::X => (d.x, d.z),
            _ => (d.z, d.x),
        };
        LineFrame {
            along,
            cross: c - self.plane_m,
            height: d.y - self.top_m,
        }
    }

    /// Plan distance to the reserved strip and vertical distance to the deck, metres.
    fn distance(&self, p: &WorldPos) -> (f64, f64) {
        let f = self.frame_of(p);
        let (e0, e1) = (self.schedule.ends.0 - 16.0, self.schedule.ends.1 + 16.0);
        let da = (e0 - f.along).max(f.along - e1).max(0.0);
        let dc = (f.cross.abs() - 8.0).max(0.0);
        (da.hypot(dc), f.height.abs())
    }
}

/// The boxes of one car in its own frame, in quanta: along, cross, height as `(lo, hi)`.
/// `median` is the sign of the cross direction of the median side.
fn car_boxes(median: i32, doors_closed: bool) -> Vec<[(i32, i32); 3]> {
    let len = (CAR_LENGTH * Q) as i32 / 2;
    let half = (LANE_WIDTH * Q) as i32 / 2;
    let (far, near) = if median > 0 {
        ((-half, -half + 4), (half - 4, half))
    } else {
        ((half - 4, half), (-half, -half + 4))
    };
    let mut boxes = vec![
        [(-len, len), (-half, half), (3, 7)],
        [(-len, len), (-half, half), (48, 52)],
        [(-len, -len + 4), (-half, half), (7, 48)],
        [(len - 4, len), (-half, half), (7, 48)],
        [(-len + 4, len - 4), far, (7, 48)],
        [(-len + 4, -144), near, (7, 48)],
        [(-112, 112), near, (7, 48)],
        [(144, len - 4), near, (7, 48)],
    ];
    if doors_closed {
        boxes.push([(-144, -112), near, (7, 48)]);
        boxes.push([(112, 144), near, (7, 48)]);
    }
    boxes
}

fn q_aabb(b: &[(i32, i32); 3], axis: Axis) -> (QAabb, [i32; 3]) {
    let size = |r: (i32, i32)| r.1 - r.0;
    let (along, cross, h) = (b[0], b[1], b[2]);
    let aabb = match axis {
        Axis::X => QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(size(along), size(h), size(cross)),
        ),
        _ => QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(size(cross), size(h), size(along)),
        ),
    };
    (aabb, [along.0, cross.0, h.0])
}

pub struct TramSystem {
    seed: WorldSeed,
    tick: u64,
    candidates: Vec<TramLine>,
    refreshed: Option<I64Vec3>,
    pub sessions: Vec<Session>,
    solids: Vec<DynamicSolid>,
    probes: Vec<Probe>,
    aboard: Option<(usize, u32)>,
    caption: Option<(String, f32)>,
    boarded_before: u32,
}

impl TramSystem {
    pub fn new(seed: WorldSeed) -> Self {
        Self {
            seed,
            tick: 0,
            candidates: Vec::new(),
            refreshed: None,
            sessions: Vec::new(),
            solids: Vec::new(),
            probes: Vec::new(),
            aboard: None,
            caption: None,
            boarded_before: 0,
        }
    }

    /// Game time in seconds.
    pub fn time(&self) -> f64 {
        self.tick as f64 * SIM_DT
    }

    pub fn drop_all(&mut self) {
        self.sessions.clear();
        self.candidates.clear();
        self.refreshed = None;
        self.aboard = None;
    }

    pub fn solids(&self) -> &[DynamicSolid] {
        &self.solids
    }

    pub fn probes(&self) -> &[Probe] {
        &self.probes
    }

    /// While aboard the collision data is the car itself, so readiness is not required.
    pub fn aboard(&self) -> bool {
        self.aboard.is_some()
    }

    pub fn caption(&self) -> Option<(&str, f32)> {
        self.caption.as_ref().map(|(t, age)| {
            (
                t.as_str(),
                ((CAPTION_TIME + 0.5 - age) / 0.5).clamp(0.0, 1.0),
            )
        })
    }

    /// Cars to draw: session index, car index, state.
    pub fn cars(&self) -> impl Iterator<Item = (usize, u32, CarState)> + '_ {
        let t = self.time();
        self.sessions.iter().enumerate().flat_map(move |(i, s)| {
            (0..s.schedule.cars).map(move |k| (i, k, s.schedule.state(t, k)))
        })
    }

    pub fn car_pose(&self, session: usize, state: &CarState) -> WorldPos {
        self.sessions[session].world(state.s, state.cross, 0.0)
    }

    fn refresh_candidates(&mut self, feet: &WorldPos) {
        let here = CellKey::containing(Level::L1, feet).coords;
        if self.refreshed == Some(here) {
            return;
        }
        self.refreshed = Some(here);
        let mut layers: Vec<LayerKey> = Vec::new();
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let c = CellKey::new(Level::L1, here + I64Vec3::new(dx, dy, dz));
                    let l = LayerKey::of(c, DECK_REGION_LOG2);
                    if !layers.contains(&l) {
                        layers.push(l);
                    }
                }
            }
        }
        self.candidates = layers
            .into_iter()
            .flat_map(|l| tram_lines(self.seed, l))
            .collect();
    }

    fn attach_and_detach(&mut self, feet: &WorldPos) {
        self.sessions.retain(|s| {
            let (plan, vert) = s.distance(feet);
            plan <= DETACH && vert <= DETACH
        });
        for line in &self.candidates {
            if self.sessions.iter().any(|s| s.line.id == line.id) {
                continue;
            }
            if let Some(s) = Session::new(line.clone()) {
                let (plan, vert) = s.distance(feet);
                if plan <= ATTACH && vert <= ATTACH {
                    self.sessions.push(s);
                }
            }
        }
    }

    /// One fixed step: moves cars to `t + dt`, clears the lane, and builds the car solids.
    pub fn update(&mut self, ctl: &mut Controller, interact: bool) {
        crate::zone!("tram update");
        let dt = SIM_DT;
        let t0 = self.time();
        self.tick += 1;
        let t1 = self.time();
        if let Some((_, age)) = &mut self.caption {
            *age += dt as f32;
            if *age > CAPTION_TIME + 0.5 {
                self.caption = None;
            }
        }
        let feet = ctl.pos;
        self.refresh_candidates(&feet);
        self.attach_and_detach(&feet);
        self.aboard = self.find_aboard(ctl, t0);
        self.push_off_lanes(ctl, t0, t1);
        self.build_solids(ctl, t0, t1, dt);
        self.build_probes(t1);
        if interact {
            self.ask(ctl, t1);
        }
        if let Some((i, _)) = self.aboard
            && self.boarded_before < 3
            && self.caption.is_none()
            && self.sessions.get(i).is_some()
        {
            self.boarded_before += 1;
            self.caption = Some(("the tram stops at every station".to_owned(), 0.0));
        }
    }

    fn find_aboard(&self, ctl: &Controller, t: f64) -> Option<(usize, u32)> {
        if !ctl.on_platform() {
            return None;
        }
        for (i, s) in self.sessions.iter().enumerate() {
            let f = s.frame_of(&ctl.pos);
            for k in 0..s.schedule.cars {
                let c = s.schedule.state(t, k);
                if (f.along - c.s).abs() <= CAR_LENGTH / 2.0 - 0.25
                    && (f.cross - c.cross).abs() <= LANE_WIDTH / 2.0 - 0.25
                    && (f.height - FLOOR_TOP).abs() <= ABOARD_TOL
                {
                    return Some((i, k));
                }
            }
        }
        None
    }

    /// A walker on the rails is moved to the median before a car sweeps over them (spec §5.5).
    fn push_off_lanes(&self, ctl: &mut Controller, t0: f64, t1: f64) {
        for (i, s) in self.sessions.iter().enumerate() {
            let f = s.frame_of(&ctl.pos);
            if f.height < -0.5 || f.height > 3.75 || f.cross.abs() > 8.0 {
                continue;
            }
            for k in 0..s.schedule.cars {
                if self.aboard == Some((i, k)) {
                    continue;
                }
                let (a, b) = (s.schedule.state(t0, k), s.schedule.state(t1, k));
                if a.v == 0.0 && b.v == 0.0 && !a.transferring && !b.transferring {
                    continue;
                }
                let r = CAPSULE_RADIUS + LANE_WIDTH / 2.0;
                let (lo, hi) = (
                    a.s.min(b.s) - CAR_LENGTH / 2.0 - CAPSULE_RADIUS,
                    a.s.max(b.s) + CAR_LENGTH / 2.0 + CAPSULE_RADIUS,
                );
                let cross_c = a.cross.min(b.cross);
                let cross_d = a.cross.max(b.cross);
                if f.along < lo || f.along > hi || f.cross < cross_c - r || f.cross > cross_d + r {
                    continue;
                }
                let (da, dc) = if a.transferring || b.transferring {
                    // Towards the line's interior along the line.
                    let mid = f64::midpoint(s.schedule.ends.0, s.schedule.ends.1);
                    let edge = if a.s < mid { hi } else { lo };
                    (edge - f.along + (mid - a.s).signum() * PUSH_CLEARANCE, 0.0)
                } else {
                    // To the median side of the car's lane.
                    let target = if a.lane == 0 {
                        cross_d + r + PUSH_CLEARANCE
                    } else {
                        cross_c - r - PUSH_CLEARANCE
                    };
                    (0.0, target - f.cross)
                };
                let d = match s.line.axis {
                    Axis::X => Vec3::new(da as f32, 0.0, dc as f32),
                    _ => Vec3::new(dc as f32, 0.0, da as f32),
                };
                if let Ok(p) = ctl.pos.offset(d) {
                    ctl.pos = p;
                }
                match s.line.axis {
                    Axis::X => ctl.vel.z = 0.0,
                    _ => ctl.vel.x = 0.0,
                }
            }
        }
    }

    fn build_solids(&mut self, ctl: &Controller, t0: f64, t1: f64, dt: f64) {
        self.solids.clear();
        for s in &self.sessions {
            let f = s.frame_of(&ctl.pos);
            for k in 0..s.schedule.cars {
                let (a, b) = (s.schedule.state(t0, k), s.schedule.state(t1, k));
                if (f.along - b.s).abs() > GATHER_RADIUS + CAR_LENGTH
                    || (f.cross - b.cross).abs() > GATHER_RADIUS
                    || f.height.abs() > GATHER_RADIUS
                {
                    continue;
                }
                let (va, vc) = ((b.s - a.s) / dt, (b.cross - a.cross) / dt);
                let velocity = match s.line.axis {
                    Axis::X => Vec3::new(va as f32, 0.0, vc as f32),
                    _ => Vec3::new(vc as f32, 0.0, va as f32),
                };
                let median = if b.lane == 0 { 1 } else { -1 };
                for bx in car_boxes(median, b.doors < 1.0) {
                    let (aabb, lo) = q_aabb(&bx, s.line.axis);
                    self.solids.push(DynamicSolid {
                        shape: Shape::Box { aabb },
                        origin: s.world(
                            b.s + f64::from(lo[0]) / Q,
                            b.cross + f64::from(lo[1]) / Q,
                            f64::from(lo[2]) / Q,
                        ),
                        platform: Some(velocity),
                    });
                }
            }
        }
    }

    /// Positions the aboard car will pass, for the streamer (spec §7).
    fn build_probes(&mut self, t1: f64) {
        self.probes.clear();
        let Some((i, k)) = self.aboard else {
            return;
        };
        let s = &self.sessions[i];
        let mut last = s.schedule.state(t1, k).s;
        let mut lead = 0.0;
        while lead <= LOOKAHEAD_S && self.probes.len() < 16 {
            let c = s.schedule.state(t1 + lead, k);
            if lead == 0.0 || (c.s - last).abs() >= PROBE_SPACING_M {
                self.probes.push(Probe {
                    pos: s.world(c.s, c.cross, 1.0),
                    lead_s: lead as f32,
                });
                last = c.s;
            }
            lead += 0.25;
        }
    }

    /// E on a boarding strip says when the next car comes. Nothing can be called (spec §6.1).
    fn ask(&mut self, ctl: &Controller, t: f64) {
        for s in &self.sessions {
            let f = s.frame_of(&ctl.pos);
            if f.cross.abs() > 2.25 || f.height.abs() > 1.0 {
                continue;
            }
            let Some(i) = s
                .schedule
                .stations
                .iter()
                .position(|&st| (st - f.along).abs() <= CAR_LENGTH / 2.0)
            else {
                continue;
            };
            let mut soonest = f64::INFINITY;
            for k in 0..s.schedule.cars {
                for lane in 0..2 {
                    if let Some(at) = s.schedule.next_dwell(t, k, lane, i) {
                        soonest = soonest.min(at - t);
                    }
                    let c = s.schedule.state(t, k);
                    if c.lane == lane
                        && c.v == 0.0
                        && (c.s - s.schedule.stations[i]).abs() < 1e-6
                        && c.doors > 0.0
                    {
                        soonest = 0.0;
                    }
                }
            }
            let text = if soonest == 0.0 {
                "the tram is here"
            } else if soonest <= 30.0 {
                "a tram is coming"
            } else {
                "the tram is far"
            };
            self.caption = Some((text.to_owned(), 0.0));
            return;
        }
    }
}

/// The first line of four cells found near the origin, for scripted rides.
pub fn find_full_line(seed: WorldSeed) -> Option<TramLine> {
    for ring in 0..4i64 {
        for y in -ring..=ring {
            for rz in -ring..=ring {
                for rx in -ring..=ring {
                    if rx.abs().max(rz.abs()).max(y.abs()) != ring {
                        continue;
                    }
                    let layer = LayerKey {
                        level: Level::L1,
                        y,
                        rx,
                        rz,
                    };
                    if let Some(l) = tram_lines(seed, layer)
                        .into_iter()
                        .find(|l| l.cells.1 - l.cells.0 == 3)
                    {
                        return Some(l);
                    }
                }
            }
        }
    }
    None
}

/// The world plus forced readiness while riding.
pub struct Riding<'a> {
    pub world: &'a dyn SolidSource,
    pub force_ready: bool,
}

impl SolidSource for Riding<'_> {
    fn gather(&self, centre: &WorldPos, half: f32, out: &mut Vec<crate::collision::Solid>) {
        self.world.gather(centre, half, out);
    }

    fn ready(&self, pos: &WorldPos) -> bool {
        self.force_ready || self.world.ready(pos)
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
mod tests {
    use super::*;
    use crate::collision::{CollisionChunk, CollisionWorld};
    use crate::controller::MoveInput;
    use crate::stream::{ChunkId, Detail};
    use worldgen::l1::l1_cell;

    const SEED: WorldSeed = WorldSeed(0x7A11);
    const DT: f32 = SIM_DT as f32;

    struct Rig {
        trams: TramSystem,
        world: CollisionWorld,
        ctl: Controller,
        id: worldgen::tram::TramLineId,
    }

    fn l2_around(world: &mut CollisionWorld, p: &WorldPos) {
        let c = CellKey::containing(Level::L2, p).coords;
        for d in -1..=1 {
            for e in -1..=1 {
                for f in -1..=1 {
                    let k = CellKey::new(Level::L2, c + I64Vec3::new(d, e, f));
                    if let Ok(b) = k.aabb_world() {
                        world.insert(
                            ChunkId::l2(k.coords),
                            CollisionChunk::new(b.min, 512.0, &[]),
                        );
                    }
                }
            }
        }
    }

    fn rig(line: &TramLine, start: &WorldPos) -> Rig {
        let sess = Session::new(line.clone()).unwrap();
        let mut world = CollisionWorld::default();
        for a in line.cells.0..=line.cells.1 {
            let c = CellKey::new(
                Level::L1,
                match line.axis {
                    Axis::X => I64Vec3::new(a, line.id.layer.y, line.column),
                    _ => I64Vec3::new(line.column, line.id.layer.y, a),
                },
            );
            let out = l1_cell(SEED, c);
            world.insert(
                ChunkId::l1(c.coords, Detail::Full),
                CollisionChunk::new(c.aabb_world().unwrap().min, 4096.0, &out.primitives),
            );
        }
        l2_around(&mut world, start);
        let last = sess.world(*sess.schedule.stations.last().unwrap(), 0.0, 1.0);
        l2_around(&mut world, &last);
        let first = sess.world(sess.schedule.stations[0], 0.0, 1.0);
        l2_around(&mut world, &first);
        Rig {
            trams: TramSystem::new(SEED),
            world,
            ctl: Controller::new(*start),
            id: line.id,
        }
    }

    impl Rig {
        fn session(&self) -> &Session {
            self.trams
                .sessions
                .iter()
                .find(|s| s.line.id == self.id)
                .unwrap()
        }

        fn tick(&mut self, wish: Vec3) {
            self.trams.update(&mut self.ctl, false);
            self.world.set_dynamic(self.trams.solids().to_vec());
            let src = Riding {
                world: &self.world,
                force_ready: self.trams.aboard(),
            };
            self.ctl.step(DT, MoveInput { wish, run: false }, &src);
        }

        fn frame(&self) -> LineFrame {
            self.session().frame_of(&self.ctl.pos)
        }
    }

    /// Unit vector of a cross-line direction in the world.
    fn cross_dir(axis: Axis, sign: f32) -> Vec3 {
        match axis {
            Axis::X => Vec3::new(0.0, 0.0, sign),
            _ => Vec3::new(sign, 0.0, 0.0),
        }
    }

    fn full_line() -> TramLine {
        crate::tram::tests::sample_lines(1, true).remove(0)
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn a_walker_on_the_rails_is_pushed_into_the_median() {
        let line = full_line();
        let sess = Session::new(line.clone()).unwrap();
        let mid = f64::midpoint(sess.schedule.stations[0], sess.schedule.stations[1]);
        let start = sess.world(mid, -4.5, 0.05);
        let mut rig = rig(&line, &start);
        let mut pushed = false;
        for _ in 0..(150.0 / SIM_DT) as usize {
            rig.tick(Vec3::ZERO);
            let f = rig.frame();
            let t = rig.trams.time();
            for k in 0..rig.session().schedule.cars {
                let c = rig.session().schedule.state(t, k);
                let inside = (f.along - c.s).abs() < CAR_LENGTH / 2.0
                    && (f.cross - c.cross).abs() < LANE_WIDTH / 2.0
                    && f.height < 3.0;
                assert!(!inside, "player inside car {k} at {t:.1} s");
            }
            pushed |= f.cross > -2.25;
        }
        assert!(pushed, "the player was never moved off the lane");
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn a_scripted_ride_from_the_first_station_to_the_last() {
        let line = full_line();
        let sess = Session::new(line.clone()).unwrap();
        let (first, last) = (
            sess.schedule.stations[0],
            *sess.schedule.stations.last().unwrap(),
        );
        let mut rig = rig(&line, &sess.world(first + 8.0, 0.0, 0.05));
        let axis = line.axis;
        let ready = |rig: &Rig, station: f64| {
            let t = rig.trams.time();
            let sch = &rig.session().schedule;
            (0..sch.cars).find(|&k| {
                let c = sch.state(t, k);
                c.lane == 0 && (c.s - station).abs() < 1e-6 && c.doors >= 1.0
            })
        };
        // Wait in the median for a lane 0 car with open doors, then walk in.
        let mut car = None;
        for _ in 0..(300.0 / SIM_DT) as usize {
            rig.tick(Vec3::ZERO);
            if let Some(k) = ready(&rig, first) {
                car = Some(k);
                break;
            }
        }
        let car = car.expect("no car opened its doors");
        for _ in 0..(3.0 / SIM_DT) as usize {
            rig.tick(cross_dir(axis, -1.0));
        }
        let c = rig.session().schedule.state(rig.trams.time(), car);
        assert!(
            rig.trams.aboard(),
            "not aboard: frame {:?}, car {c:?}, grounded {}, on_platform {}",
            rig.frame(),
            rig.ctl.grounded,
            rig.ctl.on_platform()
        );
        // Ride.
        let top = f64::from(line.top) / 16.0;
        let mut steps = 0u32;
        loop {
            rig.tick(Vec3::ZERO);
            steps += 1;
            assert!(steps < (400.0 / SIM_DT) as u32, "the ride took too long");
            assert!(
                rig.trams.aboard() && rig.ctl.grounded,
                "left the car at {:.1} s",
                rig.trams.time()
            );
            let f = rig.frame();
            assert!(
                (f.height - FLOOR_TOP).abs() < 0.03,
                "floor offset {}",
                f.height
            );
            let c = rig.session().schedule.state(rig.trams.time(), car);
            assert!((f.along - c.s).abs() < 16.0 && (f.cross - c.cross).abs() < 2.25);
            if rig.session().schedule.state(rig.trams.time(), car).s == last && c.doors >= 1.0 {
                break;
            }
        }
        // Walk out.
        for _ in 0..(4.0 / SIM_DT) as usize {
            rig.tick(cross_dir(axis, 1.0));
        }
        let f = rig.frame();
        assert!(!rig.ctl.on_platform() && rig.ctl.grounded);
        assert!(f.height.abs() < 0.03 && f.cross > -2.25, "{f:?} top {top}");
    }
}
