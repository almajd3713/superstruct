//! Debug teleport commands: to a coordinate, to the nearest port, to a sanctuary. They generate
//! cells, so callers run them off the main thread (`spawn`).

use std::thread;

use crossbeam_channel::Sender;
use glam::{I64Vec3, Vec3};
use worldgen::cell::{Axis, CellKey, Level};
use worldgen::coord::WorldPos;
use worldgen::l1::l1_cell;
use worldgen::l2_port::cell_ports;
use worldgen::prim::{ChildConstraint, MetaFlags, Q_PER_M, ZoneKind};
use worldgen::seed::WorldSeed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `sx sy sz [lx ly lz]`: a sector, and optionally a local position in it.
    Teleport(String),
    NearestPort,
    /// Picks among the sanctuaries around the player; `pick` varies the choice.
    Sanctuary {
        pick: u64,
    },
}

/// Result of a command: the new position of the player's feet.
pub type Outcome = Result<WorldPos, String>;

/// Runs `cmd` on a new thread and sends the outcome to `tx`.
pub fn spawn(seed: WorldSeed, cmd: Command, from: WorldPos, tx: Sender<Outcome>) {
    let spawned = thread::Builder::new()
        .name("debug-command".into())
        .spawn(move || {
            let _ = tx.send(run(seed, &cmd, &from));
        });
    if let Err(e) = spawned {
        tracing::error!("cannot start debug command: {e}");
    }
}

pub fn run(seed: WorldSeed, cmd: &Command, from: &WorldPos) -> Outcome {
    match cmd {
        Command::Teleport(text) => parse_teleport(text),
        Command::NearestPort => nearest_port(seed, from, 2).ok_or_else(|| "no port nearby".into()),
        Command::Sanctuary { pick } => {
            sanctuary(seed, from, *pick, 2).ok_or_else(|| "no sanctuary nearby".into())
        }
    }
}

pub fn parse_teleport(text: &str) -> Outcome {
    let nums: Vec<&str> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();
    let bad = |what: &str| format!("expected `sx sy sz` or `sx sy sz lx ly lz`: {what}");
    if nums.len() != 3 && nums.len() != 6 {
        return Err(bad("wrong number of values"));
    }
    let sector: Result<Vec<i64>, _> = nums[..3].iter().map(|t| t.parse::<i64>()).collect();
    let sector = sector.map_err(|e| bad(&e.to_string()))?;
    let local = if nums.len() == 6 {
        let l: Result<Vec<f32>, _> = nums[3..].iter().map(|t| t.parse::<f32>()).collect();
        let l = l.map_err(|e| bad(&e.to_string()))?;
        Vec3::new(l[0], l[1], l[2])
    } else {
        Vec3::splat(512.0)
    };
    WorldPos::new(I64Vec3::new(sector[0], sector[1], sector[2]), local).map_err(|e| e.to_string())
}

/// The port on any face of the L2 cells within `radius` cells of `from` closest to it, as a
/// position at the port's floor (or at the middle of a lift stop).
pub fn nearest_port(seed: WorldSeed, from: &WorldPos, radius: i64) -> Option<WorldPos> {
    let centre = CellKey::containing(Level::L2, from).coords;
    let mut best: Option<(f64, WorldPos)> = None;
    for dz in -radius..=radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let cell = CellKey::new(Level::L2, centre + I64Vec3::new(dx, dy, dz));
                if !cell.in_domain() {
                    continue;
                }
                for port in cell_ports(seed, cell) {
                    let Some(p) = port_position(&port) else {
                        continue;
                    };
                    let d = p.delta(from).length();
                    if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                        best = Some((d, p));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn port_position(port: &worldgen::port::Port) -> Option<WorldPos> {
    let q = f64::from(Q_PER_M);
    let m = |v: i32| (f64::from(v) / q) as f32;
    let (u, v) = ((m(port.rect.u0) + m(port.rect.u1)) * 0.5, m(port.rect.v0));
    let owner = port.face.owner.aabb_world().ok()?;
    let size = owner.size_m as f32;
    let local = match port.face.axis {
        Axis::X => Vec3::new(size, v, u),
        Axis::Z => Vec3::new(u, v, size),
        Axis::Y => Vec3::new(u, size, (m(port.rect.v0) + m(port.rect.v1)) * 0.5),
    };
    owner.min.offset(local).ok()
}

/// The sanctuaries in the L1 cells within `radius` cells of `from` (x and z; y one cell either
/// way), in a fixed order, indexed by `pick` modulo their count.
pub fn sanctuary(seed: WorldSeed, from: &WorldPos, pick: u64, radius: i64) -> Option<WorldPos> {
    let centre = CellKey::containing(Level::L1, from).coords;
    let mut found: Vec<WorldPos> = Vec::new();
    for dz in -radius..=radius {
        for dy in -1..=1 {
            for dx in -radius..=radius {
                let cell = CellKey::new(Level::L1, centre + I64Vec3::new(dx, dy, dz));
                if !cell.in_domain() {
                    continue;
                }
                let out = l1_cell(seed, cell);
                if !out.meta.flags.contains(MetaFlags::SANCTUARY) {
                    continue;
                }
                let zone = out.child_constraints.iter().find_map(|c| match c {
                    ChildConstraint::Zone {
                        aabb,
                        kind: ZoneKind::Sanctuary,
                    } => Some(*aabb),
                    _ => None,
                });
                let (Some(zone), Ok(b)) = (zone, cell.aabb_world()) else {
                    continue;
                };
                let (lo, hi) = (zone.min_m(), zone.max_m());
                let spot = Vec3::new((lo.x + hi.x) * 0.5, lo.y + 0.05, (lo.z + hi.z) * 0.5);
                if let Ok(p) = b.min.offset(spot) {
                    found.push(p);
                }
            }
        }
    }
    let n = found.len() as u64;
    (n > 0).then(|| found[(pick % n) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teleport_parsing() {
        let p = parse_teleport("3, -4 5").unwrap();
        assert_eq!(p.sector(), I64Vec3::new(3, -4, 5));
        assert_eq!(p.local(), Vec3::splat(512.0));
        let p = parse_teleport("1099511627776 0 0 1 2 3").unwrap();
        assert_eq!(p.sector().x, 1 << 40);
        assert_eq!(p.local(), Vec3::new(1.0, 2.0, 3.0));
        assert!(parse_teleport("1 2").is_err());
        assert!(parse_teleport("a b c").is_err());
        assert!(parse_teleport("1 2 3 4 5 x").is_err());
    }

    #[test]
    fn nearest_port_is_deterministic_and_close() {
        let seed = WorldSeed(0);
        let from = WorldPos::new(I64Vec3::new(1, -2, 1), Vec3::new(904.0, 420.0, 904.0)).unwrap();
        let a = nearest_port(seed, &from, 2).expect("a port within two cells");
        assert_eq!(Some(a), nearest_port(seed, &from, 2));
        // Within the searched neighbourhood: two L2 cells plus the cell's own extent.
        assert!(a.delta(&from).length() < 3.0 * 512.0 * 1.8);
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn sanctuary_lies_inside_a_flagged_l1_cell() {
        let seed = WorldSeed(0);
        let from = WorldPos::new(I64Vec3::ZERO, Vec3::splat(512.0)).unwrap();
        let Some(p) = sanctuary(seed, &from, 0, 2) else {
            panic!("no sanctuary within two L1 cells at seed 0");
        };
        let cell = CellKey::containing(Level::L1, &p);
        assert!(
            l1_cell(seed, cell)
                .meta
                .flags
                .contains(MetaFlags::SANCTUARY)
        );
        // `pick` wraps around the list.
        let many: Vec<_> = (0..40)
            .filter_map(|k| sanctuary(seed, &from, k, 2))
            .collect();
        assert!(many.contains(&p) && many.len() == 40);
    }
}
