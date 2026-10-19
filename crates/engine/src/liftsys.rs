//! Lift sessions, gates and cars at runtime (`docs/spec/lifts.md` §3-§8).
//!
//! Everything here is game state: nothing is persisted and nothing feeds back into the
//! generator. Plans come from `liftplan::plan_cell` on worker threads; this module only asks for
//! them (`PlanRequest`) and takes the results (`apply`).

use std::collections::{BTreeMap, BTreeSet};

use glam::{I64Vec3, Vec2, Vec3};
use worldgen::cell::{CellKey, Level};
use worldgen::coord::WorldPos;
use worldgen::key::ColumnKey;
use worldgen::l1::{Rect, core_footprint};
use worldgen::l1_port::PORT_CLEARANCE;
use worldgen::prim::{Q, Q_PER_M, QAabb, QVec3, Shape};
use worldgen::seed::WorldSeed;

use crate::collision::{DynamicSolid, SolidSource};
use crate::lift::{ACCEL, CELL_HEIGHT, EMPTY_ACCEL, LONG_CRUISE, Lift};
use crate::liftplan::{CellPlan, SIDES};

pub const ATTACH_RADIUS: f64 = 256.0;
pub const DETACH_RADIUS: f64 = 384.0;
pub const PLAN_READY_RADIUS: f64 = 64.0;
pub const PLAN_HALF_EXTENT: i64 = 1;
pub const PLAN_KEEP: i64 = 2;
pub const PLAN_SEARCH: i64 = 8;
/// Vertical distance beyond which an unused car is discarded, metres.
pub const CAR_KEEP: f64 = 1024.0;
pub const ARRIVAL_DISTANCE: f32 = 32.0;
pub const GATE_TIME: f32 = 1.0;
pub const CALL_REACH: f64 = 2.0;
pub const CALL_REACH_Y: f64 = 0.5;
pub const ON_CAR_TOL: f64 = 0.1;
pub const CAPTION_TIME: f32 = 2.5;
const CAPTION_FADE: f32 = 0.5;
/// Gates are 2 Q (0.125 m) thick, 13 Q (0.8125 m) inside the footprint: the grid has no 0.1 m
/// or 0.8 m, so the spec's values are rounded to whole quanta.
const GATE_OFFSET_Q: Q = 13;
const GATE_THICK_Q: Q = 2;
/// The core wall zone and the car's inset from the inner shaft face, quanta.
const WALL_Q: Q = 16;
const CAR_INSET_Q: Q = 1;
const BOARDING_HINTS: u32 = 3;

/// Which column and cells a worker should plan.
#[derive(Debug, Clone)]
pub struct PlanRequest {
    pub column: (i64, i64),
    pub generation: u64,
    pub cells: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct PlanResult {
    pub column: (i64, i64),
    pub generation: u64,
    pub cell_y: i64,
    pub plan: CellPlan,
}

/// Edge-triggered presses for this tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiftInput {
    pub call: bool,
    pub up: bool,
    pub down: bool,
}

/// What the lift system needs to know about the player.
#[derive(Debug, Clone, Copy)]
pub struct PlayerView {
    pub feet: WorldPos,
    pub on_platform: bool,
    pub controllable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Empty, travelling to the stop it was called to.
    Arriving,
    Parked,
    /// Gates are rising before a ride starts.
    Closing,
    Riding,
}

struct Car {
    lift: Lift,
    phase: Phase,
    /// L1 cell y of the base the lift measures from.
    base_y: i64,
    /// The stop whose served gates are open, and how far (0 closed, 1 open).
    open_stop: Option<(i64, Q)>,
    open_frac: f32,
}

struct GateBox {
    solid: DynamicSolid,
    stop: (i64, Q),
    /// A served segment opens with the car; a fixed one never does.
    served: bool,
}

struct Session {
    col: (i64, i64),
    generation: u64,
    plans: BTreeMap<i64, CellPlan>,
    requested: BTreeSet<i64>,
    car: Option<Car>,
    /// A `go` held until the plan reaches a stop in that direction (true is up).
    pending: Option<bool>,
    gates: Vec<GateBox>,
    gates_dirty: bool,
}

struct Caption {
    text: String,
    age: f32,
}

pub struct LiftSystem {
    seed: WorldSeed,
    sessions: Vec<Session>,
    next_generation: u64,
    caption: Option<Caption>,
    hints_shown: u32,
    solids: Vec<DynamicSolid>,
}

fn cell_key(col: (i64, i64), y: i64) -> CellKey {
    CellKey::new(Level::L1, I64Vec3::new(col.0, y, col.1))
}

/// The point in `key`'s cell-local frame, metres.
fn local_in(key: CellKey, p: &WorldPos) -> Option<Vec3> {
    let min = key.aabb_world().ok()?.min;
    Some((-min.delta(p)).as_vec3())
}

fn q_m(q: Q) -> f32 {
    q as f32 / Q_PER_M as f32
}

/// Distance in the plan from `(x, z)` metres to the rectangle `r` (0 inside).
fn plan_distance(r: &Rect, x: f32, z: f32) -> f64 {
    let dx = (q_m(r.x0) - x).max(x - q_m(r.x1)).max(0.0);
    let dz = (q_m(r.z0) - z).max(z - q_m(r.z1)).max(0.0);
    f64::from(dx).hypot(f64::from(dz))
}

fn footprint(seed: WorldSeed, col: (i64, i64)) -> Rect {
    core_footprint(
        seed,
        ColumnKey {
            level: Level::L1,
            x: col.0,
            z: col.1,
        },
    )
}

/// The car's rectangle: the shaft interior inset by one quantum, as (min corner, size) metres.
fn car_rect(f: &Rect) -> (Vec2, Vec2) {
    let inset = WALL_Q + CAR_INSET_Q;
    (
        Vec2::new(q_m(f.x0 + inset), q_m(f.z0 + inset)),
        Vec2::new(q_m(f.x1 - f.x0 - 2 * inset), q_m(f.z1 - f.z0 - 2 * inset)),
    )
}

impl Session {
    fn terminal(&self, focus: i64, up: bool) -> bool {
        let step = if up { 1 } else { -1 };
        let mut c = focus;
        for _ in 0..PLAN_SEARCH {
            c += step;
            match self.plans.get(&c) {
                Some(CellPlan::Cavity | CellPlan::OutOfDomain) => return true,
                Some(CellPlan::Core { .. }) => {}
                None => return false,
            }
        }
        true
    }

    fn stop_heights(&self, base_y: i64) -> Vec<f32> {
        let mut out: Vec<f32> = self
            .plans
            .iter()
            .filter_map(|(&y, plan)| match plan {
                CellPlan::Core { stops, .. } => Some((y, stops)),
                _ => None,
            })
            .flat_map(|(y, stops)| {
                stops
                    .iter()
                    .map(move |s| (y - base_y) as f32 * CELL_HEIGHT + q_m(s.y))
            })
            .collect();
        out.sort_by(f32::total_cmp);
        out.dedup();
        out
    }
}

impl LiftSystem {
    pub fn new(seed: WorldSeed) -> Self {
        Self {
            seed,
            sessions: Vec::new(),
            next_generation: 1,
            caption: None,
            hints_shown: 0,
            solids: Vec::new(),
        }
    }

    /// Forgets every session, plan, gate and car: debug teleports.
    pub fn drop_all(&mut self) {
        self.sessions.clear();
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The solids this system adds to the world: closed gates and cars.
    pub fn solids(&self) -> &[DynamicSolid] {
        &self.solids
    }

    /// The caption to show and its opacity, if any.
    pub fn caption(&self) -> Option<(&str, f32)> {
        self.caption.as_ref().map(|c| {
            let fade = ((CAPTION_TIME + CAPTION_FADE - c.age) / CAPTION_FADE).clamp(0.0, 1.0);
            (c.text.as_str(), fade)
        })
    }

    fn say(&mut self, text: &str) {
        self.caption = Some(Caption {
            text: text.to_owned(),
            age: 0.0,
        });
    }

    /// Takes a finished plan. Results for a dropped or re-made session are discarded.
    pub fn apply(&mut self, r: PlanResult) {
        let Some(s) = self
            .sessions
            .iter_mut()
            .find(|s| s.col == r.column && s.generation == r.generation)
        else {
            return;
        };
        s.requested.remove(&r.cell_y);
        s.plans.insert(r.cell_y, r.plan);
        s.gates_dirty = true;
        // New stops mean a held press may now succeed, and a car needs the new list.
        if let Some(base_y) = s.car.as_ref().map(|c| c.base_y) {
            let stops = s.stop_heights(base_y);
            if let Some(car) = &mut s.car {
                car.lift.set_stops(stops);
            }
        }
    }

    /// The readiness guard (spec §3.3): true while a core within `PLAN_READY_RADIUS` of the
    /// player has no plan for the player's cell yet, so gates may be missing.
    pub fn holds_controller(&self, feet: &WorldPos) -> bool {
        let here = CellKey::containing(Level::L1, feet).coords;
        self.sessions.iter().any(|s| {
            if s.plans.contains_key(&here.y) {
                return false;
            }
            local_in(cell_key(s.col, here.y), feet).is_some_and(|l| {
                plan_distance(&footprint(self.seed, s.col), l.x, l.z) <= PLAN_READY_RADIUS
            })
        })
    }

    /// One fixed step. Plan requests are returned for the caller to run on workers.
    pub fn update(
        &mut self,
        dt: f32,
        player: &PlayerView,
        input: LiftInput,
        world: &dyn SolidSource,
    ) -> Vec<PlanRequest> {
        let mut requests = Vec::new();
        if let Some(c) = &mut self.caption {
            c.age += dt;
            if c.age > CAPTION_TIME + CAPTION_FADE {
                self.caption = None;
            }
        }
        self.attach_and_detach(player);
        for i in 0..self.sessions.len() {
            self.update_session(i, dt, player, input, world, &mut requests);
        }
        self.rebuild_solids(dt);
        requests
    }

    fn attach_and_detach(&mut self, player: &PlayerView) {
        let here = CellKey::containing(Level::L1, &player.feet).coords;
        let seed = self.seed;
        // Detach: far away and not riding.
        let on_car = |s: &Session| {
            s.car
                .as_ref()
                .is_some_and(|c| Self::on_car(seed, s.col, c, player))
        };
        self.sessions.retain(|s| {
            let near = local_in(cell_key(s.col, here.y), &player.feet)
                .is_some_and(|l| plan_distance(&footprint(seed, s.col), l.x, l.z) <= DETACH_RADIUS);
            near || on_car(s)
        });
        // Attach: the columns around the player.
        for dz in -1..=1 {
            for dx in -1..=1 {
                let col = (here.x + dx, here.z + dz);
                if self.sessions.iter().any(|s| s.col == col) {
                    continue;
                }
                let near = local_in(cell_key(col, here.y), &player.feet).is_some_and(|l| {
                    plan_distance(&footprint(seed, col), l.x, l.z) <= ATTACH_RADIUS
                });
                if near && self.sessions.len() < 4 {
                    let generation = self.next_generation;
                    self.next_generation += 1;
                    self.sessions.push(Session {
                        col,
                        generation,
                        plans: BTreeMap::new(),
                        requested: BTreeSet::new(),
                        car: None,
                        pending: None,
                        gates: Vec::new(),
                        gates_dirty: true,
                    });
                }
            }
        }
    }

    /// Whether the player stands on `car` (spec §6.3).
    fn on_car(seed: WorldSeed, col: (i64, i64), car: &Car, player: &PlayerView) -> bool {
        if !player.on_platform {
            return false;
        }
        let key = cell_key(col, car.base_y);
        let Some(l) = local_in(key, &player.feet) else {
            return false;
        };
        let (min, size) = car_rect(&footprint(seed, col));
        let inside = l.x >= min.x && l.x <= min.x + size.x && l.z >= min.y && l.z <= min.y + size.y;
        inside && f64::from((l.y - car.lift.height()).abs()) <= ON_CAR_TOL
    }

    fn update_session(
        &mut self,
        i: usize,
        dt: f32,
        player: &PlayerView,
        input: LiftInput,
        world: &dyn SolidSource,
        requests: &mut Vec<PlanRequest>,
    ) {
        let seed = self.seed;
        let here = CellKey::containing(Level::L1, &player.feet).coords;
        let riding = {
            let s = &self.sessions[i];
            s.car
                .as_ref()
                .is_some_and(|c| Self::on_car(seed, s.col, c, player))
        };
        // Focus: the car's cell while riding, the player's otherwise.
        let focus = match (&self.sessions[i].car, riding) {
            (Some(c), true) => c.base_y + (c.lift.height() / CELL_HEIGHT).floor() as i64,
            _ => here.y,
        };
        self.request_plans(i, focus, requests);

        if input.call && player.controllable && !riding {
            self.call(i, player, requests);
        }
        if player.controllable && riding && (input.up || input.down) {
            self.press(i, input.up, focus, requests);
        }
        self.try_pending(i, focus, requests);
        self.drive_car(i, dt, player, riding, world);
        self.discard_far_car(i, player, riding);
    }

    /// Asks for the cells around `focus` that are not planned, cutting at cavities.
    fn request_plans(&mut self, i: usize, focus: i64, requests: &mut Vec<PlanRequest>) {
        let s = &mut self.sessions[i];
        let mut want = Vec::new();
        let mut consider = |y: i64, s: &Session| {
            if !s.plans.contains_key(&y) && !s.requested.contains(&y) {
                want.push(y);
            }
        };
        consider(focus, s);
        for up in [false, true] {
            let step = if up { 1 } else { -1 };
            for k in 1..=PLAN_HALF_EXTENT {
                let y = focus + step * k;
                consider(y, s);
                if matches!(
                    s.plans.get(&y),
                    Some(CellPlan::Cavity | CellPlan::OutOfDomain)
                ) {
                    break;
                }
            }
        }
        // Trim far cells, but never the car's cell or its target's.
        let keep_car = s.car.as_ref().map(|c| {
            let h = |h: f32| c.base_y + (h / CELL_HEIGHT).floor() as i64;
            (h(c.lift.height()), c.lift.target().map(h))
        });
        let before = s.plans.len();
        s.plans.retain(|&y, _| {
            (y - focus).abs() <= PLAN_KEEP || keep_car.is_some_and(|(a, t)| y == a || t == Some(y))
        });
        if s.plans.len() != before {
            s.gates_dirty = true;
        }
        if !want.is_empty() {
            for &y in &want {
                s.requested.insert(y);
            }
            requests.push(PlanRequest {
                column: s.col,
                generation: s.generation,
                cells: want,
            });
        }
    }

    /// A call at an opening in reach (spec §4.3 and §6.2).
    fn call(&mut self, i: usize, player: &PlayerView, _requests: &mut [PlanRequest]) {
        let seed = self.seed;
        let s = &self.sessions[i];
        let f = footprint(seed, s.col);
        // The nearest served segment within reach, over every planned cell.
        let mut best: Option<(f64, i64, Q)> = None;
        for (&y, plan) in &s.plans {
            let CellPlan::Core { stops, .. } = plan else {
                continue;
            };
            let Some(l) = local_in(cell_key(s.col, y), &player.feet) else {
                continue;
            };
            for stop in stops {
                if f64::from((l.y - q_m(stop.y)).abs()) > CALL_REACH_Y {
                    continue;
                }
                for (side, spans) in stop.served.iter().enumerate() {
                    for &(a, b) in spans {
                        let d = side_distance(&f, side, (a, b), l.x, l.z);
                        if d <= CALL_REACH && best.is_none_or(|(bd, _, _)| d < bd) {
                            best = Some((d, y, stop.y));
                        }
                    }
                }
            }
        }
        let Some((_, cell_y, stop_q)) = best else {
            return;
        };
        let Some(s) = self.sessions.get_mut(i) else {
            return;
        };
        let stop_here =
            |car: &Car| -> f32 { (cell_y - car.base_y) as f32 * CELL_HEIGHT + q_m(stop_q) };
        let mut say = "the lift is coming";
        let mut create = true;
        if let Some(car) = &mut s.car {
            let h = stop_here(car);
            let at = (car.lift.height() - h).abs() < 0.01;
            if car.phase == Phase::Parked && at {
                say = "the lift is here";
                create = false;
            } else if car.phase == Phase::Arriving
                && car.lift.target().is_some_and(|t| (t - h).abs() < 0.01)
            {
                create = false;
            } else if (car.lift.height() - h).abs() <= ARRIVAL_DISTANCE
                && car.phase != Phase::Riding
            {
                car.lift.go_to(h);
                car.phase = Phase::Arriving;
                car.open_stop = None;
                car.open_frac = 0.0;
                create = false;
            } else if car.phase == Phase::Riding {
                return;
            }
        }
        if create {
            s.car = None;
            let base_y = cell_y;
            let (min, size) = car_rect(&f);
            let stops = s.stop_heights(base_y);
            let base = cell_key(s.col, base_y)
                .aabb_world()
                .map_or(WorldPos::ORIGIN, |b| b.min);
            let s_h = q_m(stop_q);
            // Above the stop unless the planned shaft ends within 32 m above it.
            let top = s
                .plans
                .iter()
                .filter(|(_, p)| matches!(p, CellPlan::Core { .. }))
                .map(|(&y, _)| (y - base_y + 1) as f32 * CELL_HEIGHT)
                .fold(0.0, f32::max);
            let start = if s_h + ARRIVAL_DISTANCE <= top {
                s_h + ARRIVAL_DISTANCE
            } else {
                s_h - ARRIVAL_DISTANCE
            };
            let mut lift = Lift::new_at(base, (min, size), stops, start, LONG_CRUISE);
            lift.set_accel(EMPTY_ACCEL);
            lift.go_to(s_h);
            s.car = Some(Car {
                lift,
                phase: Phase::Arriving,
                base_y,
                open_stop: None,
                open_frac: 0.0,
            });
        }
        self.say(say);
    }

    /// Space or Ctrl while on the car (spec §3.4, §5.2).
    fn press(&mut self, i: usize, up: bool, focus: i64, _requests: &mut [PlanRequest]) {
        let mut caption: Option<&str> = None;
        let s = &mut self.sessions[i];
        let Some(car) = &mut s.car else {
            return;
        };
        let before = car.lift.target();
        car.lift.go(up);
        let after = car.lift.target();
        if after != before {
            s.pending = None;
            if car.phase == Phase::Parked {
                // Gates rise first; the car waits.
                car.phase = Phase::Closing;
                car.lift.set_hold(true);
            }
            if self.hints_shown < BOARDING_HINTS {
                self.hints_shown += 1;
                caption = Some("Space up · Ctrl down");
            }
        } else if s.terminal(focus, up) && Self::no_stop_beyond(s, up) {
            caption = Some("the shaft ends here");
        } else {
            s.pending = Some(up);
        }
        if let Some(text) = caption {
            self.say(text);
        }
    }

    fn no_stop_beyond(s: &Session, up: bool) -> bool {
        let Some(car) = &s.car else {
            return true;
        };
        let h = car.lift.target().unwrap_or(car.lift.height());
        let stops = car.lift.stops();
        if up {
            !stops.iter().any(|&x| x > h + 0.5)
        } else {
            !stops.iter().any(|&x| x < h - 0.5)
        }
    }

    /// Applies a held press once the plan has a stop that way, or asks for the next cell.
    fn try_pending(&mut self, i: usize, focus: i64, requests: &mut Vec<PlanRequest>) {
        let Some(up) = self.sessions[i].pending else {
            return;
        };
        let s = &mut self.sessions[i];
        let Some(car) = &mut s.car else {
            s.pending = None;
            return;
        };
        let before = car.lift.target();
        car.lift.go(up);
        if car.lift.target() != before {
            s.pending = None;
            if car.phase == Phase::Parked {
                car.phase = Phase::Closing;
                car.lift.set_hold(true);
            }
            return;
        }
        if s.terminal(focus, up) {
            s.pending = None;
            self.say("the shaft ends here");
            return;
        }
        // Extend one cell at a time, up to PLAN_SEARCH cells beyond the car.
        let s = &mut self.sessions[i];
        let Some(car) = &s.car else {
            return;
        };
        let car_cell = car.base_y + (car.lift.height() / CELL_HEIGHT).floor() as i64;
        let step = if up { 1 } else { -1 };
        let mut y = car_cell;
        for _ in 0..=PLAN_SEARCH {
            y += step;
            match s.plans.get(&y) {
                Some(CellPlan::Core { .. }) => {}
                Some(_) => return,
                None => {
                    if s.requested.insert(y) {
                        requests.push(PlanRequest {
                            column: s.col,
                            generation: s.generation,
                            cells: vec![y],
                        });
                    }
                    return;
                }
            }
        }
        s.pending = None;
        self.say("the shaft ends here");
    }

    /// Moves the car and runs its gate and phase transitions.
    fn drive_car(
        &mut self,
        i: usize,
        dt: f32,
        player: &PlayerView,
        riding: bool,
        world: &dyn SolidSource,
    ) {
        let s = &mut self.sessions[i];
        let Some(car) = &mut s.car else {
            return;
        };
        // Gate ramps.
        match car.phase {
            Phase::Parked if car.open_stop.is_some() => {
                car.open_frac = (car.open_frac + dt / GATE_TIME).min(1.0);
            }
            Phase::Closing => {
                car.open_frac = (car.open_frac - dt / GATE_TIME).max(0.0);
                if car.open_frac <= 0.0 {
                    car.open_stop = None;
                    car.phase = Phase::Riding;
                    car.lift.set_accel(ACCEL);
                    car.lift.set_hold(false);
                }
            }
            _ => {}
        }
        if !matches!(car.phase, Phase::Arriving | Phase::Riding) {
            car.lift.update(0.0); // keeps `prev_y` in step while parked
            return;
        }
        // Freeze together with a passenger whose ground has not streamed in.
        if riding && !world.ready(&player.feet) {
            car.lift.update(0.0);
            return;
        }
        // Stream hold: brake before the car outruns collision data.
        if let Some(target) = car.lift.target() {
            let dir = (target - car.lift.height()).signum();
            let reach =
                (car.lift.stopping_distance() + 32.0).min((target - car.lift.height()).abs());
            let probe = car.lift.point_at(car.lift.height() + dir * reach);
            car.lift.set_hold(!world.ready(&probe));
        }
        car.lift.update(dt);
        // Rebase when the car leaves its cell.
        if car.lift.height() >= CELL_HEIGHT {
            car.lift.rebase(1);
            car.base_y += 1;
        } else if car.lift.height() < 0.0 {
            car.lift.rebase(-1);
            car.base_y -= 1;
        }
        if car.lift.idle() {
            car.phase = Phase::Parked;
            car.lift.set_hold(false);
            let h = car.lift.height();
            car.open_stop = s
                .plans
                .iter()
                .filter_map(|(&y, p)| match p {
                    CellPlan::Core { stops, .. } => Some((y, stops)),
                    _ => None,
                })
                .flat_map(|(y, stops)| stops.iter().map(move |st| (y, st.y)))
                .find(|&(y, q)| ((y - car.base_y) as f32 * CELL_HEIGHT + q_m(q) - h).abs() < 0.01);
            car.open_frac = 0.0;
        }
    }

    fn discard_far_car(&mut self, i: usize, player: &PlayerView, riding: bool) {
        let s = &mut self.sessions[i];
        let Some(car) = &s.car else {
            return;
        };
        if riding {
            return;
        }
        let Some(l) = local_in(cell_key(s.col, car.base_y), &player.feet) else {
            return;
        };
        if f64::from((l.y - car.lift.height()).abs()) > CAR_KEEP {
            s.car = None;
        }
    }

    /// Rebuilds the world solids: gates (closed ones) and cars.
    fn rebuild_solids(&mut self, dt: f32) {
        self.solids.clear();
        let seed = self.seed;
        for s in &mut self.sessions {
            if s.gates_dirty {
                s.gates = build_gates(seed, s);
                s.gates_dirty = false;
            }
            let open = s
                .car
                .as_ref()
                .and_then(|c| (c.open_frac >= 1.0).then_some(c.open_stop).flatten());
            for g in &s.gates {
                if g.served && Some(g.stop) == open {
                    continue;
                }
                self.solids.push(g.solid);
            }
            if let Some(car) = &s.car {
                let v = if dt > 0.0 {
                    Vec3::Y * (car.lift.step_displacement() / dt)
                } else {
                    Vec3::ZERO
                };
                self.solids.push(DynamicSolid {
                    shape: car.lift.car_shape(),
                    origin: car.lift.car_origin(),
                    platform: Some(v),
                });
            }
        }
    }

    /// For drawing: every car's slab origin and size.
    pub fn cars(&self) -> impl Iterator<Item = (WorldPos, Vec2)> + '_ {
        self.sessions.iter().filter_map(|s| {
            s.car
                .as_ref()
                .map(|c| (c.lift.car_origin(), c.lift.footprint().1))
        })
    }
}

/// Distance in the plan from `(x, z)` to the strip (1 m wall zone) behind side `side`'s served
/// interval `(a, b)`.
fn side_distance(f: &Rect, side: usize, (a, b): (Q, Q), x: f32, z: f32) -> f64 {
    let strip = match side {
        0 => Rect {
            x0: f.x0,
            z0: a,
            x1: f.x0 + WALL_Q,
            z1: b,
        },
        1 => Rect {
            x0: f.x1 - WALL_Q,
            z0: a,
            x1: f.x1,
            z1: b,
        },
        2 => Rect {
            x0: a,
            z0: f.z0,
            x1: b,
            z1: f.z0 + WALL_Q,
        },
        _ => Rect {
            x0: a,
            z0: f.z1 - WALL_Q,
            x1: b,
            z1: f.z1,
        },
    };
    plan_distance(&strip, x, z)
}

/// The gate boxes of every planned stop of a session (spec §5.1).
fn build_gates(seed: WorldSeed, s: &Session) -> Vec<GateBox> {
    let f = footprint(seed, s.col);
    let mut out = Vec::new();
    let height = PORT_CLEARANCE;
    for (&y, plan) in &s.plans {
        let CellPlan::Core { stops, .. } = plan else {
            continue;
        };
        let Ok(cell) = cell_key(s.col, y).aabb_world() else {
            continue;
        };
        for stop in stops {
            for (side, served) in stop.served.iter().enumerate().take(SIDES) {
                // The side's extent along itself, inset so the ring closes at the corners.
                let (e0, e1) = if side < 2 {
                    (f.z0 + GATE_OFFSET_Q, f.z1 - GATE_OFFSET_Q)
                } else {
                    (f.x0 + GATE_OFFSET_Q, f.x1 - GATE_OFFSET_Q)
                };
                let mut cuts: Vec<(Q, Q, bool)> = Vec::new();
                let mut at = e0;
                for &(a, b) in served {
                    let (a, b) = (a.max(e0), b.min(e1));
                    if a >= b {
                        continue;
                    }
                    if a > at {
                        cuts.push((at, a, false));
                    }
                    cuts.push((a, b, true));
                    at = b;
                }
                if at < e1 {
                    cuts.push((at, e1, false));
                }
                for (a, b, is_served) in cuts {
                    let (lo, hi) = match side {
                        0 => (f.x0 + GATE_OFFSET_Q, f.x0 + GATE_OFFSET_Q + GATE_THICK_Q),
                        1 => (f.x1 - GATE_OFFSET_Q - GATE_THICK_Q, f.x1 - GATE_OFFSET_Q),
                        2 => (f.z0 + GATE_OFFSET_Q, f.z0 + GATE_OFFSET_Q + GATE_THICK_Q),
                        _ => (f.z1 - GATE_OFFSET_Q - GATE_THICK_Q, f.z1 - GATE_OFFSET_Q),
                    };
                    let (min, max) = if side < 2 {
                        (
                            QVec3::new(lo, stop.y, a),
                            QVec3::new(hi, stop.y + height, b),
                        )
                    } else {
                        (
                            QVec3::new(a, stop.y, lo),
                            QVec3::new(b, stop.y + height, hi),
                        )
                    };
                    let shape = Shape::Box {
                        aabb: QAabb::new(
                            QVec3::new(0, 0, 0),
                            QVec3::new(max.x - min.x, max.y - min.y, max.z - min.z),
                        ),
                    };
                    let origin = cell
                        .min
                        .offset(Vec3::new(q_m(min.x), q_m(min.y), q_m(min.z)))
                        .unwrap_or(cell.min);
                    out.push(GateBox {
                        solid: DynamicSolid {
                            shape,
                            origin,
                            platform: None,
                        },
                        stop: (y, stop.y),
                        served: is_served,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::cast_sign_loss)]
mod tests {
    use super::*;
    use crate::collision::{CollisionChunk, CollisionWorld};
    use crate::controller::{Controller, MoveInput};
    use crate::liftplan::{StopPlan, plan_cell};
    use crate::stream::{ChunkId, Detail};
    use worldgen::l1::l1_cell;

    const SEED: WorldSeed = WorldSeed(0);
    const DT: f32 = 1.0 / 120.0;

    /// The first L1 cell (y -4..=4, z 0..8, x 0..8) whose core has two stops at least 600 m
    /// apart, with side 0 served at its middle at the lowest stop.
    fn find_cell() -> (CellKey, Rect, Vec<StopPlan>) {
        for y in -4..=4 {
            for z in 0..8 {
                for x in 0..8 {
                    let k = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                    if let CellPlan::Core { footprint, stops } = plan_cell(SEED, k) {
                        let mid = i32::midpoint(footprint.z0, footprint.z1);
                        if stops.len() >= 2
                            && q_m(stops[1].y - stops[0].y) >= 600.0
                            && stops[0].served[0].iter().any(|&(a, b)| a < mid && mid < b)
                        {
                            println!("lift test cell {:?}", k.coords);
                            return (k, footprint, stops);
                        }
                    }
                }
            }
        }
        panic!("no cell in the search range matched");
    }

    fn world_for(k: CellKey) -> CollisionWorld {
        let mut world = CollisionWorld::default();
        let out = l1_cell(SEED, k);
        world.insert(
            ChunkId::l1(k.coords, Detail::Full),
            CollisionChunk::new(k.aabb_world().unwrap().min, 4096.0, &out.primitives),
        );
        for e in 0..8 {
            for f in 0..8 {
                for g in 0..8 {
                    let c = k.coords * 8 + I64Vec3::new(e, f, g);
                    let l2 = CellKey::new(Level::L2, c);
                    world.insert(
                        ChunkId::l2(c),
                        CollisionChunk::new(l2.aabb_world().unwrap().min, 512.0, &[]),
                    );
                }
            }
        }
        world
    }

    struct Rig {
        sys: LiftSystem,
        world: CollisionWorld,
        ctl: Controller,
    }

    impl Rig {
        fn tick(&mut self, wish: Vec3, input: LiftInput) {
            let view = PlayerView {
                feet: self.ctl.pos,
                on_platform: self.ctl.on_platform(),
                controllable: true,
            };
            for r in self.sys.update(DT, &view, input, &self.world) {
                for y in r.cells {
                    self.sys.apply(PlanResult {
                        column: r.column,
                        generation: r.generation,
                        cell_y: y,
                        plan: plan_cell(SEED, cell_key(r.column, y)),
                    });
                }
            }
            self.world.set_dynamic(self.sys.solids().to_vec());
            self.ctl
                .step(DT, MoveInput { wish, run: false }, &self.world);
        }

        fn walk(&mut self, secs: f32, wish: Vec3) {
            for _ in 0..(secs * 120.0).round() as u32 {
                self.tick(wish, LiftInput::default());
            }
        }

        fn car(&self) -> Option<&Car> {
            self.sys.sessions.first().and_then(|s| s.car.as_ref())
        }

        fn parked_open(&self) -> bool {
            self.car()
                .is_some_and(|c| c.phase == Phase::Parked && c.open_frac >= 1.0)
        }

        fn wait(&mut self, max_s: f32, wish: Vec3, done: impl Fn(&Rig) -> bool) -> f32 {
            for i in 0..(max_s * 120.0).round() as u32 {
                if done(self) {
                    return i as f32 * DT;
                }
                self.tick(wish, LiftInput::default());
            }
            panic!("condition not reached in {max_s} s");
        }
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn a_scripted_ride_between_two_decks() {
        let (k, f, stops) = find_cell();
        let (s0, s1) = (q_m(stops[0].y), q_m(stops[1].y));
        let min = k.aabb_world().unwrap().min;
        let at = |x: f32, y: f32, z: f32| min.offset(Vec3::new(x, y, z)).unwrap();
        let mid_z = q_m(f.z0 + f.z1) / 2.0;
        let mut rig = Rig {
            sys: LiftSystem::new(SEED),
            world: world_for(k),
            ctl: Controller::new(at(q_m(f.x0) - 3.0, s0 + 0.05, mid_z)),
        };

        // 1. The gate keeps the feet outside the shaft.
        rig.walk(1.0, Vec3::ZERO);
        rig.walk(3.0, Vec3::X);
        let x = (-min.delta(&rig.ctl.pos)).x as f32;
        assert!(
            x <= q_m(f.x0) + 0.8125 - 0.3 + 0.02,
            "feet passed the gate: {x}"
        );

        // 2. Call: the car arrives and the gates open.
        rig.tick(
            Vec3::ZERO,
            LiftInput {
                call: true,
                ..Default::default()
            },
        );
        let t = rig.wait(12.0, Vec3::ZERO, Rig::parked_open);
        println!("car answered in {t:.1} s");

        // 3. Walk aboard.
        rig.walk(4.0, Vec3::X);
        assert!(rig.ctl.on_platform(), "not on the car");

        // 4. Ride up, checking the passenger stays on the car at every step.
        rig.tick(
            Vec3::ZERO,
            LiftInput {
                up: true,
                ..Default::default()
            },
        );
        let expect = (s1 - s0) / 30.0 + 20.0;
        let mut secs = 0.0;
        loop {
            rig.tick(Vec3::ZERO, LiftInput::default());
            secs += DT;
            assert!(secs < expect + 10.0, "ride too long");
            assert!(
                rig.ctl.on_platform() && rig.ctl.grounded,
                "left the car at {secs:.2} s"
            );
            if rig.parked_open() {
                break;
            }
        }
        assert!(
            (secs - expect).abs() <= 4.0,
            "ride took {secs:.1} s, expected {expect:.1}"
        );
        let top = (-min.delta(&rig.ctl.pos)).y as f32;
        assert!((top - s1).abs() < 0.05, "feet at {top}, stop at {s1}");

        // 5. Walk off onto the deck.
        rig.walk(8.0, -Vec3::X);
        assert!(!rig.ctl.on_platform() && rig.ctl.grounded);
        let l = (-min.delta(&rig.ctl.pos)).as_vec3();
        assert!((l.y - s1).abs() < 0.05, "feet at {}, deck at {s1}", l.y);
        assert!(l.x < q_m(f.x0) - 1.0, "still inside the shaft: {}", l.x);
    }
}
