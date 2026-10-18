//! Tram schedule and car geometry (`docs/spec/trams.md` §5).
//!
//! The schedule is a pure function of a line and game time, so the car's future positions are
//! known exactly (the streamer's look-ahead needs that). Nothing here is persisted.

use worldgen::tram::TramLine;

pub const CRUISE: f64 = 100.0;
pub const ACCEL: f64 = 6.0;
pub const DWELL: f64 = 20.0;
pub const TRANSFER_TIME: f64 = 12.0;
pub const HEADWAY: f64 = 60.0;
pub const DOOR_TIME: f64 = 1.0;
/// Car length and lane width, metres.
pub const CAR_LENGTH: f64 = 32.0;
pub const LANE_WIDTH: f64 = 4.5;
/// Lane centres lie this far from the plane, metres.
pub const LANE_OFFSET: f64 = 4.5;

const Q: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// `from` to `to` along the line, metres.
    Hop {
        from: f64,
        to: f64,
    },
    Dwell {
        at: f64,
    },
    /// Crossing from one lane to the other at the line end `at`.
    Transfer {
        at: f64,
        to_lane: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Seg {
    t0: f64,
    dur: f64,
    lane: u8,
    kind: Kind,
}

/// One car's state: where it is and what its doors do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarState {
    /// Along the line from the first cell's min corner, metres.
    pub s: f64,
    /// Speed along +s, metres per second (negative on lane 1).
    pub v: f64,
    /// Across from the plane, metres: `-LANE_OFFSET` on lane 0, `+LANE_OFFSET` on lane 1.
    pub cross: f64,
    /// 0 closed to 1 open.
    pub doors: f64,
    /// 0 or 1: the lane it is on or leaving.
    pub lane: u8,
    pub transferring: bool,
}

#[derive(Debug, Clone)]
pub struct Schedule {
    segs: Vec<Seg>,
    pub t_cycle: f64,
    pub cars: u32,
    pub headway: f64,
    phase: f64,
    pub stations: Vec<f64>,
    pub ends: (f64, f64),
}

/// Time to cover `d` metres from rest to rest.
pub fn hop_time(d: f64) -> f64 {
    if d >= CRUISE * CRUISE / ACCEL {
        d / CRUISE + CRUISE / ACCEL
    } else {
        2.0 * (d / ACCEL).sqrt()
    }
}

/// Distance covered and speed `u` seconds into a hop of length `d`.
fn hop_at(d: f64, u: f64) -> (f64, f64) {
    let dur = hop_time(d);
    let u = u.clamp(0.0, dur);
    let rem = dur - u;
    if d >= CRUISE * CRUISE / ACCEL {
        let ta = CRUISE / ACCEL;
        if u < ta {
            (0.5 * ACCEL * u * u, ACCEL * u)
        } else if rem > ta {
            (CRUISE * CRUISE / (2.0 * ACCEL) + CRUISE * (u - ta), CRUISE)
        } else {
            (d - 0.5 * ACCEL * rem * rem, ACCEL * rem)
        }
    } else if u < dur / 2.0 {
        (0.5 * ACCEL * u * u, ACCEL * u)
    } else {
        (d - 0.5 * ACCEL * rem * rem, ACCEL * rem)
    }
}

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Door opening for `u` seconds into a dwell: shut for 1 s, opening over `DOOR_TIME`, open, then
/// closing from 2 s before departure.
fn doors_at(u: f64) -> f64 {
    let opening = ((u - 1.0) / DOOR_TIME).clamp(0.0, 1.0);
    let closing = ((DWELL - 2.0 - u) / DOOR_TIME + 1.0).clamp(0.0, 1.0);
    opening.min(closing)
}

impl Schedule {
    pub fn new(line: &TramLine) -> Self {
        let stations: Vec<f64> = line.stations.iter().map(|s| f64::from(s.s) / Q).collect();
        let ends = (f64::from(line.ends.0) / Q, f64::from(line.ends.1) / Q);
        let mut segs = Vec::new();
        let mut t = 0.0;
        let mut push = |lane: u8, dur: f64, kind: Kind| {
            segs.push(Seg {
                t0: t,
                dur,
                lane,
                kind,
            });
            t += dur;
        };
        // Lane 0 runs towards +s, ending in a transfer at the far end; lane 1 mirrors it.
        let forward: Vec<f64> = std::iter::once(ends.0)
            .chain(stations.iter().copied())
            .chain(std::iter::once(ends.1))
            .collect();
        for (lane, pts) in [
            (0u8, forward.clone()),
            (1u8, forward.iter().rev().copied().collect()),
        ] {
            for (i, w) in pts.windows(2).enumerate() {
                let d = (w[1] - w[0]).abs();
                push(
                    lane,
                    hop_time(d),
                    Kind::Hop {
                        from: w[0],
                        to: w[1],
                    },
                );
                if i + 2 < pts.len() {
                    push(lane, DWELL, Kind::Dwell { at: w[1] });
                }
            }
            let at = pts[pts.len() - 1];
            push(
                lane,
                TRANSFER_TIME,
                Kind::Transfer {
                    at,
                    to_lane: 1 - lane,
                },
            );
        }
        let t_cycle = t;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a few cars
        let cars = ((t_cycle / HEADWAY).round() as u32).max(1);
        Self {
            segs,
            t_cycle,
            cars,
            headway: t_cycle / f64::from(cars),
            phase: f64::from(line.phase_ms) / 1000.0,
            stations,
            ends,
        }
    }

    /// Car `k` at game time `t` seconds.
    pub fn state(&self, t: f64, k: u32) -> CarState {
        let tau = (t + self.phase + f64::from(k) * self.headway).rem_euclid(self.t_cycle);
        let i = self
            .segs
            .partition_point(|s| s.t0 + s.dur <= tau)
            .min(self.segs.len() - 1);
        let seg = self.segs[i];
        let u = tau - seg.t0;
        let lane_cross = |lane: u8| if lane == 0 { -LANE_OFFSET } else { LANE_OFFSET };
        match seg.kind {
            Kind::Hop { from, to } => {
                let d = (to - from).abs();
                let dir = (to - from).signum();
                let (x, v) = hop_at(d, u);
                CarState {
                    s: from + dir * x,
                    v: dir * v,
                    cross: lane_cross(seg.lane),
                    doors: 0.0,
                    lane: seg.lane,
                    transferring: false,
                }
            }
            Kind::Dwell { at } => CarState {
                s: at,
                v: 0.0,
                cross: lane_cross(seg.lane),
                doors: doors_at(u),
                lane: seg.lane,
                transferring: false,
            },
            Kind::Transfer { at, to_lane } => {
                let f = smoothstep(u / seg.dur);
                CarState {
                    s: at,
                    v: 0.0,
                    cross: lane_cross(seg.lane) + (lane_cross(to_lane) - lane_cross(seg.lane)) * f,
                    doors: 0.0,
                    lane: seg.lane,
                    transferring: true,
                }
            }
        }
    }

    /// The time within one cycle at which car 0 dwells at station `i` on `lane`, or `None`.
    pub fn dwell_start(&self, lane: u8, station: usize) -> Option<f64> {
        let at = *self.stations.get(station)?;
        self.segs
            .iter()
            .find(|s| {
                s.lane == lane && matches!(s.kind, Kind::Dwell { at: a } if (a - at).abs() < 1e-9)
            })
            .map(|s| s.t0)
    }

    /// The game time at or after `t` when car `k` starts dwelling at `station` on `lane`.
    pub fn next_dwell(&self, t: f64, k: u32, lane: u8, station: usize) -> Option<f64> {
        let t0 = self.dwell_start(lane, station)?;
        let shift = self.phase + f64::from(k) * self.headway;
        // tau = t + shift (mod cycle) equals t0 at the wanted time.
        let wait = (t0 - (t + shift)).rem_euclid(self.t_cycle);
        Some(t + wait)
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
pub(crate) mod tests {
    use super::*;
    use worldgen::cell::Level;
    use worldgen::hash::hash;
    use worldgen::key::LayerKey;
    use worldgen::seed::WorldSeed;
    use worldgen::tram::tram_lines;

    const DT: f64 = 1.0 / 120.0;

    /// Lines from scattered layers: a full one first when `full` is set.
    pub(crate) fn sample_lines(n: usize, full: bool) -> Vec<TramLine> {
        let seed = WorldSeed(0x7A11);
        let mut out = Vec::new();
        let mut i = 0i64;
        while out.len() < n {
            let h = |salt: u64, m: u64| {
                i64::try_from(hash(seed.0, Level::L1, glam::I64Vec3::new(i, 0, 0), salt) % m)
                    .unwrap_or(0)
            };
            let layer = LayerKey {
                level: Level::L1,
                y: h(1, 32) - 16,
                rx: h(2, 128) - 64,
                rz: h(3, 128) - 64,
            };
            i += 1;
            for l in tram_lines(seed, layer) {
                if !full || l.cells.1 - l.cells.0 == 3 {
                    out.push(l);
                }
            }
        }
        out.truncate(n);
        out
    }

    #[test]
    fn hop_times_match_the_spec_table() {
        let near = |a: f64, b: f64| assert!((a - b).abs() < 0.06, "{a} vs {b}");
        near(hop_time(2560.0), 42.27);
        near(hop_time(6656.0), 83.23);
        near(hop_time(7168.0), 88.35);
        near(hop_time(7680.0), 93.47);
        // A short hop is a triangle: 64 m takes 6.53 s.
        near(hop_time(64.0), 6.53);
    }

    #[test]
    fn a_full_line_cycle_matches_the_worked_example() {
        let line = &sample_lines(1, true)[0];
        let s = Schedule::new(line);
        assert_eq!(line.stations.len(), 3);
        assert!((s.t_cycle - 533.8).abs() < 1.0, "{}", s.t_cycle);
        assert_eq!(s.cars, 9);
        assert!((s.headway - 59.3).abs() < 0.2, "{}", s.headway);
    }

    #[test]
    fn motion_is_continuous_bounded_and_periodic() {
        for line in sample_lines(12, false) {
            let sch = Schedule::new(&line);
            let mut prev = sch.state(0.0, 0);
            let mut peak_a = 0.0f64;
            let steps = (2.0 * sch.t_cycle / DT) as usize;
            for n in 1..=steps {
                let t = n as f64 * DT;
                let now = sch.state(t, 0);
                assert!((now.s - prev.s).abs() <= CRUISE * DT + 1e-6, "jump at {t}");
                assert!(now.v.abs() <= CRUISE + 1e-3);
                peak_a = peak_a.max(((now.v - prev.v) / DT).abs());
                if now.doors > 0.0 {
                    assert!(now.v == 0.0, "doors open while moving at {t}");
                }
                prev = now;
            }
            assert!(peak_a <= ACCEL + 0.01, "peak acceleration {peak_a}");
            let (a, b) = (sch.state(123.4, 0), sch.state(123.4 + sch.t_cycle, 0));
            assert!((a.s - b.s).abs() < 1e-3 && a.lane == b.lane);
        }
    }

    #[test]
    fn dwells_hold_the_car_at_the_station_with_doors_that_open_and_close() {
        let line = &sample_lines(1, true)[0];
        let sch = Schedule::new(line);
        let start = sch.dwell_start(0, 1).unwrap();
        let at = |u: f64| sch.state(start + u - sch.phase_for_test(), 0);
        for u in [0.0, 5.0, 19.99] {
            let c = at(u);
            assert!((c.s - sch.stations[1]).abs() < 1e-3 && c.v == 0.0);
        }
        assert_eq!(at(0.5).doors, 0.0);
        assert_eq!(at(10.0).doors, 1.0);
        assert_eq!(at(19.5).doors, 0.0);
    }

    #[test]
    fn same_lane_cars_keep_apart() {
        for line in sample_lines(8, false) {
            let sch = Schedule::new(&line);
            let mut t = 0.0;
            while t < sch.t_cycle {
                for k in 0..sch.cars {
                    for j in (k + 1)..sch.cars {
                        let (a, b) = (sch.state(t, k), sch.state(t, j));
                        if a.lane == b.lane && !a.transferring && !b.transferring {
                            assert!((a.s - b.s).abs() >= 2.0 * CAR_LENGTH, "cars {k},{j} at {t}");
                        }
                    }
                }
                t += 0.5;
            }
        }
    }

    impl Schedule {
        fn phase_for_test(&self) -> f64 {
            self.phase
        }
    }
}
