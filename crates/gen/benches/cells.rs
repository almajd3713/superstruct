//! P0.11.4: generation cost of L1 and L2 cells, single thread. Budgets (PLAN.md §11.4):
//! L1 cell under 5 ms, L2 cell under 2 ms.
#![allow(clippy::expect_used)] // benches may panic on a missing fixture

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::l1::l1_cell;
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::seed::WorldSeed;

const SEED: WorldSeed = WorldSeed(0x601D);

/// L1 cells that emit something, scattered, so the average covers real structure.
fn l1_cells(n: usize) -> Vec<CellKey> {
    (0..400i64)
        .map(|i| {
            CellKey::new(
                Level::L1,
                I64Vec3::new(i * 7 % 61 - 30, i % 5 - 2, i * 11 % 53 - 26),
            )
        })
        .filter(|&c| !l1_cell(SEED, c).primitives.is_empty())
        .take(n)
        .collect()
}

fn bench(c: &mut Criterion) {
    let l1 = l1_cells(32);
    assert_eq!(l1.len(), 32);
    c.bench_function("l1_cell (mean of 32 cells with content)", |b| {
        b.iter(|| {
            for &cell in &l1 {
                black_box(l1_cell(SEED, black_box(cell)));
            }
        });
    });

    // The L2 children of 4 L1 cells that have decks: every child, empty or not.
    let parents: Vec<CellKey> = l1.iter().copied().take(4).collect();
    let children: Vec<(CellKey, CellKey)> = parents
        .iter()
        .flat_map(|&p| {
            (0..512i64).map(move |k| {
                (
                    p,
                    CellKey::new(
                        Level::L2,
                        p.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                    ),
                )
            })
        })
        .collect();
    c.bench_function("l2_cell, L1 parent cached (mean of 2048 children)", |b| {
        b.iter(|| {
            let cache = L1Cache::default();
            for &(_, cell) in &children {
                black_box(l2_cell_in(SEED, &cache, black_box(cell)));
            }
        });
    });
    // Only children that emit something, for the worst realistic case.
    let cache = L1Cache::default();
    let busy: Vec<CellKey> = children
        .iter()
        .map(|&(_, c)| c)
        .filter(|&c| l2_cell_in(SEED, &cache, c).primitives.len() > 30)
        .collect();
    assert!(!busy.is_empty());
    c.bench_function("l2_cell, busy cells only (mean)", |b| {
        b.iter(|| {
            for &cell in &busy {
                black_box(l2_cell_in(SEED, &cache, black_box(cell)));
            }
        });
    });
    c.bench_function("l2_cell, L1 parent computed per cell", |b| {
        b.iter(|| {
            for &cell in busy.iter().take(8) {
                black_box(l2_cell_in(SEED, &L1Cache::default(), black_box(cell)));
            }
        });
    });
    eprintln!(
        "cells: {} L1, {} L2 children, {} busy L2 cells",
        l1.len(),
        children.len(),
        busy.len()
    );
}

criterion_group!(benches, bench);
criterion_main!(benches);
