//! P0.7.10: `l1_cell` cost, and the cavity queries it used to repeat.
#![allow(clippy::expect_used)] // benches may panic on a missing fixture

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use glam::I64Vec3;
use worldgen::cavity::{CavityView, is_cavity};
use worldgen::cell::{CellKey, Level};
use worldgen::l1_web::l1_cell;
use worldgen::regime::{RareEvent, rare_event};
use worldgen::seed::WorldSeed;

fn bench(c: &mut Criterion) {
    let seed = WorldSeed(0x601D);
    let cell = |x, y, z| CellKey::new(Level::L1, I64Vec3::new(x, y, z));
    // Cells of the first giant-cavity L0 cell, where the mask is not empty.
    let l0 = (0..2000i64)
        .map(|i| I64Vec3::new(i % 50 - 25, i / 50 - 20, 3))
        .find(|&l0| rare_event(seed, l0) == RareEvent::GiantCavity)
        .expect("a cavity in range");
    let cells: Vec<CellKey> = (0..16)
        .map(|i| cell(l0.x * 8 + i % 8, l0.y * 8 + 1 + i / 8, l0.z * 8 + 2))
        .collect();

    c.bench_function("l1_cell", |b| {
        b.iter(|| {
            for &cell in &cells {
                black_box(l1_cell(seed, black_box(cell)));
            }
        });
    });
    c.bench_function("cavity_13_queries_per_call", |b| {
        b.iter(|| {
            for &cell in &cells {
                for _ in 0..13 {
                    black_box(is_cavity(seed, black_box(cell)));
                }
            }
        });
    });
    c.bench_function("cavity_view_once", |b| {
        b.iter(|| {
            for &cell in &cells {
                let view = CavityView::new(seed, black_box(cell));
                for _ in 0..13 {
                    black_box(view.is_cavity(cell));
                }
            }
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
