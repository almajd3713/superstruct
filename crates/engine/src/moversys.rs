//! World-clock movers at runtime (`docs/spec/levels.md` §3W.10): the moving bridge, the lift
//! bridge, the lift that turns and the tram through the gate. Each is a pure function of the world
//! clock (`worldgen::movers`); this system keeps the ones near the player at their pose, as
//! collision solids that carry the walker and as chunks that move.

use std::collections::HashMap;

use glam::{I64Vec3, Mat3, Vec3};
use worldgen::coord::WorldPos;
use worldgen::movers::{Mover, Pose, movers_in_cell};
use worldgen::orient::{Frame, LocalShape};
use worldgen::prim::{QAabb, QVec3, Shape};
use worldgen::seed::WorldSeed;

use crate::collision::{LadderVol, Solid, SolidSource};
use crate::mesh::Mesh;
use crate::renderer::{ChunkHandle, Renderer};

/// Movers farther than this from the player are dropped, metres.
const ACTIVE_M: f64 = 3500.0;
/// The sample step for a mover's velocity, milliseconds.
const VEL_MS: u64 = 50;

struct Active {
    mover: Mover,
    pose: Pose,
    /// Metres per second of the origin.
    vel: Vec3,
    drawn: Option<(ChunkHandle, u8)>,
    /// How many timed pieces stand (BLD-05).
    placed: usize,
    /// The chunk of the timed pieces that stand, and how many it holds.
    frame: Option<(ChunkHandle, usize)>,
}

#[derive(Default)]
pub struct MoverSystem {
    active: HashMap<u64, Active>,
    /// Chunks of movers that went out of range, removed at the next draw.
    gone: Vec<ChunkHandle>,
}

fn absolute(p: &WorldPos) -> [f64; 3] {
    let (s, l) = (p.sector(), p.local());
    [0, 1, 2].map(|i| s[i] as f64 * 1024.0 + f64::from(l[i]))
}

impl MoverSystem {
    /// Keeps the movers within 3.5 km of `player` at their pose at world time `t_ms`.
    pub fn update(&mut self, seed: WorldSeed, player: &WorldPos, t_ms: u64) {
        let p = absolute(player);
        let l1 = I64Vec3::new(
            (p[0] / 4096.0).floor() as i64,
            (p[1] / 4096.0).floor() as i64,
            (p[2] / 4096.0).floor() as i64,
        );
        let mut seen = Vec::new();
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    for m in movers_in_cell(seed, l1 + I64Vec3::new(dx, dy, dz)) {
                        let d = [0, 1, 2].map(|i| m.origin[i] as f64 - p[i]);
                        if d.iter().map(|v| v * v).sum::<f64>().sqrt() > ACTIVE_M {
                            continue;
                        }
                        seen.push(m.key);
                        // The Slow district runs the world clock at a tenth (STG-09).
                        let (num, den) = worldgen::strange::time_scale_at(seed, m.origin);
                        let t_ms = t_ms * num / den;
                        let pose = m.pose(t_ms);
                        let ahead = m.pose(t_ms + VEL_MS * num / den);
                        let dt = (VEL_MS * num / den) as f32 / 1000.0;
                        let vel = (Vec3::from(ahead.offset) - Vec3::from(pose.offset)) / dt;
                        match self.active.get_mut(&m.key) {
                            Some(a) => {
                                a.pose = pose;
                                a.vel = vel;
                                a.placed = a.mover.placed(t_ms);
                            }
                            None => {
                                self.active.insert(
                                    m.key,
                                    Active {
                                        placed: m.placed(t_ms),
                                        mover: m,
                                        pose,
                                        vel,
                                        drawn: None,
                                        frame: None,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        let far: Vec<u64> = self
            .active
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for k in far {
            if let Some(a) = self.active.remove(&k) {
                self.gone.extend(
                    a.drawn
                        .map(|(h, _)| h)
                        .into_iter()
                        .chain(a.frame.map(|(h, _)| h)),
                );
            }
        }
    }

    /// The active movers' keys and poses, for tests and tools.
    pub fn poses(&self) -> Vec<(u64, Pose)> {
        self.active.iter().map(|(&k, a)| (k, a.pose)).collect()
    }

    /// The movers' solids in the frame of `centre`, within `half` metres of it.
    pub fn gather(&self, centre: &WorldPos, half: f32, out: &mut Vec<Solid>) {
        let c = absolute(centre);
        let clip = QAabb::new(
            QVec3::new(-(1 << 28), -(1 << 28), -(1 << 28)),
            QVec3::new(1 << 28, 1 << 28, 1 << 28),
        );
        for a in self.active.values() {
            let at = [0, 1, 2].map(|i| a.mover.origin[i] as f64 + f64::from(a.pose.offset[i]));
            let d = Vec3::new(
                (at[0] - c[0]) as f32,
                (at[1] - c[1]) as f32,
                (at[2] - c[2]) as f32,
            );
            if d.length() > half + 200.0 {
                continue;
            }
            // The pieces the machine has placed stand where it put them.
            let base = Vec3::new(
                (a.mover.origin[0] as f64 - c[0]) as f32,
                (a.mover.origin[1] as f64 - c[1]) as f32,
                (a.mover.origin[2] as f64 - c[2]) as f32,
            );
            for (part, _) in a.mover.timed.iter().take(a.placed).filter(|(p, _)| p.solid) {
                out.push(Solid {
                    shape: Shape::oriented(
                        Frame::Id,
                        QVec3::new(0, 0, 0),
                        LocalShape::Box { aabb: part.aabb },
                        clip,
                    ),
                    offset: base,
                    platform: None,
                });
            }
            for part in a.mover.parts.iter().filter(|p| p.solid) {
                out.push(Solid {
                    shape: Shape::oriented(
                        Frame::table(a.pose.yaw, 0),
                        QVec3::new(0, 0, 0),
                        LocalShape::Box { aabb: part.aabb },
                        clip,
                    ),
                    offset: d,
                    platform: Some(a.vel),
                });
            }
        }
    }

    /// Draws every active mover as a chunk at its pose (the mesh is rebuilt when the turn
    /// changes), and removes the chunks of movers that went away.
    pub fn draw(&mut self, renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
        for h in self.gone.drain(..) {
            renderer.remove_chunk(h);
        }
        for a in self.active.values_mut() {
            let origin = WorldPos::new(
                I64Vec3::new(
                    a.mover.origin[0].div_euclid(1024),
                    a.mover.origin[1].div_euclid(1024),
                    a.mover.origin[2].div_euclid(1024),
                ),
                Vec3::new(
                    a.mover.origin[0].rem_euclid(1024) as f32,
                    a.mover.origin[1].rem_euclid(1024) as f32,
                    a.mover.origin[2].rem_euclid(1024) as f32,
                ),
            )
            .ok()
            .and_then(|w| w.offset(Vec3::from(a.pose.offset)).ok());
            let Some(origin) = origin else {
                continue;
            };
            // The timed pieces that stand, as one chunk at the mover's resting origin.
            let at_rest = WorldPos::new(
                I64Vec3::new(
                    a.mover.origin[0].div_euclid(1024),
                    a.mover.origin[1].div_euclid(1024),
                    a.mover.origin[2].div_euclid(1024),
                ),
                Vec3::new(
                    a.mover.origin[0].rem_euclid(1024) as f32,
                    a.mover.origin[1].rem_euclid(1024) as f32,
                    a.mover.origin[2].rem_euclid(1024) as f32,
                ),
            )
            .ok();
            if let Some(rest) = at_rest
                && a.frame.is_none_or(|(_, n)| n != a.placed)
            {
                if let Some((h, _)) = a.frame {
                    renderer.remove_chunk(h);
                }
                let mut mesh = Mesh::default();
                let q = |v: QVec3| Vec3::new(v.x as f32, v.y as f32, v.z as f32) / 16.0;
                for (p, _) in a.mover.timed.iter().take(a.placed) {
                    mesh.add_box(q(p.aabb.min), q(p.aabb.max), p.material.0);
                }
                let (h, _) = renderer.upload_chunk(device, queue, rest, &mesh);
                a.frame = Some((h, a.placed));
            }
            match a.drawn {
                Some((h, yaw)) if yaw == a.pose.yaw => renderer.set_chunk_origin(h, origin),
                old => {
                    if let Some((h, _)) = old {
                        renderer.remove_chunk(h);
                    }
                    let mesh = mesh_of(&a.mover, a.pose.yaw);
                    let (h, _) = renderer.upload_chunk(device, queue, origin, &mesh);
                    a.drawn = Some((h, a.pose.yaw));
                }
            }
        }
    }

    /// Removes every chunk (when walking starts anew, for instance).
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for a in self.active.drain().map(|(_, a)| a) {
            if let Some((h, _)) = a.drawn {
                renderer.remove_chunk(h);
            }
            if let Some((h, _)) = a.frame {
                renderer.remove_chunk(h);
            }
        }
    }
}

/// The mesh of a mover turned `yaw` 64ths of a turn about the vertical axis through its origin.
fn mesh_of(m: &Mover, yaw: u8) -> Mesh {
    let mut base = Mesh::default();
    let q = |v: QVec3| Vec3::new(v.x as f32, v.y as f32, v.z as f32) / 16.0;
    for p in &m.parts {
        base.add_box(q(p.aabb.min), q(p.aabb.max), p.material.0);
    }
    if yaw == 0 {
        return base;
    }
    let [x, y, z] = Frame::table(yaw, 0).axes();
    let rot = Mat3::from_cols(
        Vec3::new(x.x as f32, x.y as f32, x.z as f32),
        Vec3::new(y.x as f32, y.y as f32, y.z as f32),
        Vec3::new(z.x as f32, z.y as f32, z.z as f32),
    );
    let mut out = Mesh::default();
    out.add_transformed(&base, glam::Mat4::from_mat3(rot));
    out
}

/// The collision world plus the movers: the walker is carried by whatever moves.
pub struct WithMovers<'a> {
    pub inner: &'a dyn SolidSource,
    pub movers: &'a MoverSystem,
}

impl SolidSource for WithMovers<'_> {
    fn gather(&self, centre: &WorldPos, half: f32, out: &mut Vec<Solid>) {
        self.inner.gather(centre, half, out);
        self.movers.gather(centre, half, out);
    }

    fn ready(&self, pos: &WorldPos) -> bool {
        self.inner.ready(pos)
    }

    fn ladders(&self, centre: &WorldPos, half: f32, out: &mut Vec<LadderVol>) {
        self.inner.ladders(centre, half, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::tests::Flat;
    use worldgen::movers::{MoverKind, site_of};

    const SEED: WorldSeed = WorldSeed(0);

    fn bridge() -> Mover {
        for x in -40..40 {
            for z in -40..40 {
                if let Some((k, l1)) = site_of(SEED, I64Vec3::new(x, 0, z))
                    && k == MoverKind::MovingBridge
                {
                    return movers_in_cell(SEED, l1).remove(0);
                }
            }
        }
        panic!("no moving bridge");
    }

    fn at(m: &Mover, dx: f32, dy: f32, dz: f32) -> WorldPos {
        let o = m.origin;
        WorldPos::new(
            I64Vec3::new(
                o[0].div_euclid(1024),
                o[1].div_euclid(1024),
                o[2].div_euclid(1024),
            ),
            Vec3::new(
                o[0].rem_euclid(1024) as f32,
                o[1].rem_euclid(1024) as f32,
                o[2].rem_euclid(1024) as f32,
            ) + Vec3::new(dx, dy, dz),
        )
        .unwrap()
    }

    /// A walker stands on the leaf of a moving bridge and is carried when it slides: SYS-01 on
    /// the controller side.
    #[test]
    fn the_leaf_carries_a_walker() {
        let m = bridge();
        let mut sys = MoverSystem::default();
        // A moment when the leaf is about to move: the cycle position 340/1024.
        let t0 = 10 * m.period_ms
            + (m.period_ms - m.phase_ms % m.period_ms + m.period_ms * 335 / 1024) % m.period_ms;
        let mut c = crate::controller::Controller::new(at(&m, 0.0, 0.5, 0.0));
        let empty = Flat(vec![]);
        let start = absolute(&c.pos);
        let mut t = t0;
        for _ in 0..(40 * 120) {
            sys.update(SEED, &c.pos, t);
            let world = WithMovers {
                inner: &empty,
                movers: &sys,
            };
            c.step(1.0 / 120.0, crate::controller::MoveInput::default(), &world);
            t += 1000 / 120;
        }
        let end = absolute(&c.pos);
        assert!(c.grounded, "the walker fell off the leaf: {:?}", c.pos);
        assert!(end[0] - start[0] > 5.0, "carried {} m", end[0] - start[0]);
    }

    /// BLD-01: a beam the Builder has placed is solid where it was put, and not before.
    #[test]
    fn a_placed_beam_is_solid() {
        let m = (0..40)
            .map(|i| {
                let mut seen = 0;
                for x in -60..60 {
                    for z in -60..60 {
                        if let Some((k, l1)) = site_of(SEED, I64Vec3::new(x, 0, z))
                            && k == MoverKind::Builder
                        {
                            if seen == i {
                                return movers_in_cell(SEED, l1).remove(0);
                            }
                            seen += 1;
                        }
                    }
                }
                panic!("no builder");
            })
            .find(|m| !m.dock)
            .unwrap();
        let start = (m.period_ms - m.phase_ms) % m.period_ms + 10 * m.period_ms;
        let player = at(&m, 0.0, 12.0, 0.0);
        let mut sys = MoverSystem::default();
        sys.update(SEED, &player, start);
        let mut before = Vec::new();
        sys.gather(&player, 600.0, &mut before);
        sys.update(SEED, &player, start + worldgen::movers::BUILDER_WORK_MS / 2);
        let mut after = Vec::new();
        sys.gather(&player, 600.0, &mut after);
        assert_eq!(
            after.len() - before.len(),
            worldgen::movers::BEAMS / 2,
            "five beams stand after half the hour"
        );
        // The first beam stands at x = 30 to 50 m and 12 to 16 m up; the player is 12 m up.
        let probe = Vec3::new(40.0, 2.0, 0.0);
        let hit = crate::collision::sdf(&after, probe) < 0.0;
        assert!(hit, "the first beam is solid at {probe:?}");
    }
}
