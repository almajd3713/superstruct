//! `SUPERSTRUCT_TRAM=auto`: a scripted ride along the first full line near the origin, for
//! native hitch captures (`docs/spec/trams.md` §9.3). The player waits in the median of the first
//! station, boards the first lane 0 car that opens its doors, rides to the last station and
//! steps out.

use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::coord::WorldPos;
use worldgen::seed::WorldSeed;
use worldgen::tram::TramLineId;

use crate::tramsys::{Session, TramSystem, find_full_line};

const BOARD_SECONDS: f64 = 3.0;
const LEAVE_SECONDS: f64 = 4.0;

#[derive(Debug, Clone, Copy)]
enum Phase {
    Start,
    Waiting,
    Boarding { until: f64, car: u32 },
    Riding { car: u32 },
    Leaving { until: f64 },
    Done,
}

pub struct TramAuto {
    seed: WorldSeed,
    phase: Phase,
    line: Option<TramLineId>,
    axis: Axis,
    started: f64,
}

/// What the script wants this step.
pub struct AutoCommand {
    pub wish: Vec3,
    /// Place the player here (feet) before stepping.
    pub teleport: Option<WorldPos>,
    pub finished: bool,
}

impl TramAuto {
    pub fn new(seed: WorldSeed) -> Self {
        Self {
            seed,
            phase: Phase::Start,
            line: None,
            axis: Axis::X,
            started: 0.0,
        }
    }

    fn cross(&self, sign: f32) -> Vec3 {
        match self.axis {
            Axis::X => Vec3::new(0.0, 0.0, sign),
            _ => Vec3::new(sign, 0.0, 0.0),
        }
    }

    pub fn step(&mut self, trams: &TramSystem) -> AutoCommand {
        let mut cmd = AutoCommand {
            wish: Vec3::ZERO,
            teleport: None,
            finished: false,
        };
        let t = trams.time();
        if let Phase::Start = self.phase {
            let Some(line) = find_full_line(self.seed) else {
                tracing::error!("no full tram line near the origin");
                cmd.finished = true;
                return cmd;
            };
            let Some(session) = Session::new(line) else {
                cmd.finished = true;
                return cmd;
            };
            self.line = Some(session.line.id);
            self.axis = session.line.axis;
            let first = session.schedule.stations[0];
            cmd.teleport = Some(session.world(first + 8.0, 0.0, 0.05));
            self.started = t;
            tracing::info!(line = ?session.line.id, "scripted tram ride: waiting at the first station");
            self.phase = Phase::Waiting;
            return cmd;
        }
        let Some(session) = trams.sessions.iter().find(|s| Some(s.line.id) == self.line) else {
            return cmd;
        };
        let sch = &session.schedule;
        let first = sch.stations[0];
        let last = sch.stations[sch.stations.len() - 1];
        match self.phase {
            Phase::Waiting => {
                if let Some(car) = (0..sch.cars).find(|&k| {
                    let c = sch.state(t, k);
                    c.lane == 0 && (c.s - first).abs() < 1e-6 && c.doors >= 1.0
                }) {
                    self.phase = Phase::Boarding {
                        until: t + BOARD_SECONDS,
                        car,
                    };
                }
            }
            Phase::Boarding { until, car } => {
                cmd.wish = self.cross(-1.0);
                if t >= until {
                    if trams.aboard() {
                        tracing::info!(car, "scripted tram ride: aboard");
                        self.started = t;
                        self.phase = Phase::Riding { car };
                    } else {
                        tracing::warn!("scripted tram ride: missed the car, waiting for the next");
                        self.phase = Phase::Waiting;
                    }
                }
            }
            Phase::Riding { car } => {
                let c = sch.state(t, car);
                if (c.s - last).abs() < 1e-6 && c.doors >= 1.0 {
                    tracing::info!(seconds = t - self.started, "scripted tram ride: arrived");
                    self.phase = Phase::Leaving {
                        until: t + LEAVE_SECONDS,
                    };
                }
            }
            Phase::Leaving { until } => {
                cmd.wish = self.cross(1.0);
                if t >= until {
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => cmd.finished = true,
            Phase::Start => {}
        }
        cmd
    }
}
