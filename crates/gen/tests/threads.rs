//! Thread-order test (P0.11.2): the same cells generated serially and in a shuffled order on
//! several threads come out identical.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use glam::I64Vec3;
use rayon::prelude::*;
use worldgen::cell::{CellKey, Level};
use worldgen::hash::hash;
use worldgen::l1::l1_cell;
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::regime::regime;
use worldgen::seed::WorldSeed;

const SEED: WorldSeed = WorldSeed(0x7EAD);

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn digest(cell: CellKey) -> u64 {
    match cell.level {
        Level::L0 => fnv(&format!("{:?}", regime(SEED, cell.coords))),
        Level::L1 => fnv(&format!("{:?}", l1_cell(SEED, cell))),
        _ => fnv(&format!(
            "{:?}",
            l2_cell_in(SEED, &L1Cache::default(), cell)
        )),
    }
}

fn cells() -> Vec<CellKey> {
    let mut cells = Vec::new();
    for i in 0..300i64 {
        let h = |level: Level, salt: u64| {
            i64::try_from(hash(SEED.0, level, I64Vec3::new(i, 0, 0), salt) % 200).unwrap() - 100
        };
        cells.push(CellKey::new(
            Level::L0,
            I64Vec3::new(h(Level::L0, 1), h(Level::L0, 2) / 20, h(Level::L0, 3)),
        ));
        cells.push(CellKey::new(
            Level::L1,
            I64Vec3::new(h(Level::L1, 1), h(Level::L1, 2) / 20, h(Level::L1, 3)),
        ));
        cells.push(CellKey::new(
            Level::L2,
            I64Vec3::new(
                h(Level::L2, 1),
                h(Level::L2, 2).rem_euclid(16),
                h(Level::L2, 3),
            ),
        ));
    }
    cells
}

#[test]
fn shuffled_parallel_generation_matches_serial() {
    let cells = cells();
    let serial: Vec<u64> = cells.iter().map(|&c| digest(c)).collect();

    // A deterministic shuffle of the order the cells are handed out in.
    let mut order: Vec<usize> = (0..cells.len()).collect();
    for i in (1..order.len()).rev() {
        let j = usize::try_from(
            hash(
                SEED.0,
                Level::L0,
                I64Vec3::new(i64::try_from(i).unwrap(), 9, 9),
                5,
            ) % (i as u64 + 1),
        )
        .unwrap();
        order.swap(i, j);
    }
    for threads in [2, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let mut parallel: Vec<(usize, u64)> =
            pool.install(|| order.par_iter().map(|&i| (i, digest(cells[i]))).collect());
        parallel.sort_unstable();
        let parallel: Vec<u64> = parallel.into_iter().map(|(_, d)| d).collect();
        assert_eq!(serial, parallel, "{threads} threads");
    }
}
