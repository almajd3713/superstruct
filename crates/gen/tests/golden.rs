//! Golden tests: pinned digests of generator output for fixed cells (P0.11.1).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//!
//! `tests/golden/cells.txt` lists `GEN_VERSION`, then one line per cell: level, coordinates
//! and the FNV-1a digest of the cell's debug text. The test fails on any output change. After
//! an intended change, bump `GEN_VERSION` and regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p gen --release --test golden`.

use std::fmt::Write as _;
use std::path::PathBuf;

use glam::I64Vec3;
use worldgen::GEN_VERSION;
use worldgen::cell::{CellKey, Level};
use worldgen::hash::hash;
use worldgen::l1::l1_cell;
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::regime::regime;
use worldgen::seed::WorldSeed;

const SEED: WorldSeed = WorldSeed(0x601D);
const PER_LEVEL: usize = 100;

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cells.txt")
}

/// The digest of one cell: L0 regime, L1 output, or L2 output.
fn digest(cell: CellKey) -> u64 {
    match cell.level {
        Level::L0 => fnv(&format!("{:?}", regime(SEED, cell.coords))),
        Level::L1 => fnv(&format!("{:?}", l1_cell(SEED, cell))),
        Level::L2 => fnv(&format!(
            "{:?}",
            l2_cell_in(SEED, &L1Cache::default(), cell)
        )),
        _ => unreachable!("no generator below L2 yet"),
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::L0 => "L0",
        Level::L1 => "L1",
        _ => "L2",
    }
}

/// Picks cells for a new golden file: scattered coordinates, keeping L1 and L2 cells that
/// emit something so the digests cover real structure.
fn pick(level: Level) -> Vec<CellKey> {
    let mut cells = Vec::new();
    let mut i = 0u64;
    while cells.len() < PER_LEVEL {
        let h = |salt: u64| {
            i64::try_from(
                hash(
                    SEED.0,
                    level,
                    I64Vec3::new(i64::try_from(i).unwrap(), 0, 0),
                    salt,
                ) % 400,
            )
            .unwrap()
                - 200
        };
        let coords = match level {
            Level::L2 => I64Vec3::new(h(1), h(2).rem_euclid(24), h(3)),
            _ => I64Vec3::new(h(1), h(2) / 8, h(3)),
        };
        let cell = CellKey::new(level, coords);
        i += 1;
        let keep = match level {
            Level::L0 => true,
            Level::L1 => !l1_cell(SEED, cell).primitives.is_empty(),
            _ => !l2_cell_in(SEED, &L1Cache::default(), cell)
                .primitives
                .is_empty(),
        };
        if keep {
            cells.push(cell);
        }
    }
    cells
}

#[test]
fn golden_cells_are_unchanged() {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let mut text = format!("gen_version {GEN_VERSION}\n");
        for level in [Level::L0, Level::L1, Level::L2] {
            for cell in pick(level) {
                let c = cell.coords;
                writeln!(
                    text,
                    "{} {} {} {} {:016x}",
                    level_name(level),
                    c.x,
                    c.y,
                    c.z,
                    digest(cell)
                )
                .unwrap();
            }
        }
        std::fs::write(path(), text).unwrap();
        return;
    }
    let text = std::fs::read_to_string(path()).expect("tests/golden/cells.txt");
    let mut lines = text.lines();
    let version: u32 = lines
        .next()
        .and_then(|l| l.strip_prefix("gen_version "))
        .and_then(|v| v.parse().ok())
        .expect("gen_version line");
    assert_eq!(
        version, GEN_VERSION,
        "GEN_VERSION changed: regenerate the golden file with UPDATE_GOLDEN=1"
    );
    let mut changed = Vec::new();
    let mut counts = [0usize; 3];
    for line in lines {
        let f: Vec<&str> = line.split_whitespace().collect();
        let level = match f[0] {
            "L0" => Level::L0,
            "L1" => Level::L1,
            _ => Level::L2,
        };
        counts[level as usize] += 1;
        let coords = I64Vec3::new(
            f[1].parse().unwrap(),
            f[2].parse().unwrap(),
            f[3].parse().unwrap(),
        );
        let want = u64::from_str_radix(f[4], 16).unwrap();
        let cell = CellKey::new(level, coords);
        if digest(cell) != want {
            changed.push(format!("{} {} {} {}", f[0], f[1], f[2], f[3]));
        }
    }
    assert_eq!(counts, [PER_LEVEL; 3]);
    assert!(
        changed.is_empty(),
        "generator output changed in {} cells (bump GEN_VERSION and regenerate if intended): {:?}",
        changed.len(),
        &changed[..changed.len().min(5)]
    );
}

/// Digests are sensitive: the same cell under another seed differs.
#[test]
fn digests_depend_on_the_seed() {
    let cell = CellKey::new(Level::L1, I64Vec3::new(3, 0, 1));
    let a = fnv(&format!("{:?}", l1_cell(SEED, cell)));
    let b = fnv(&format!("{:?}", l1_cell(WorldSeed(SEED.0 + 1), cell)));
    assert_ne!(a, b);
}
