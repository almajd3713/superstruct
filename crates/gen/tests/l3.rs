//! L3 blocks (`docs/spec/levels.md` §6.14, P4.1.3).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::manual_assert,
    clippy::cast_sign_loss
)] // test code

use std::time::Instant;

use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::l1::CELL as L1_CELL;
use worldgen::l2::{MassInfo, l2_build};
use worldgen::l2_port::L1Cache;
use worldgen::l3::{ATTEMPTS, L3Block, l3_block};
use worldgen::prim::{PrimFlags, Q, QAabb, Shape};
use worldgen::seed::WorldSeed;

const SEED: WorldSeed = WorldSeed(0x7A3);

fn active_blocks(l2: CellKey, masses: &[MassInfo]) -> Vec<CellKey> {
    let mut out = Vec::new();
    for b in 0..512i64 {
        let idx = I64Vec3::new(b % 8, b / 64, b / 8 % 8);
        let o = idx * 1024;
        let hit = masses.iter().any(|m| {
            let a = &m.aabb;
            let (lo, hi) = (o, o + I64Vec3::splat(1024));
            i64::from(a.min.x) < hi.x
                && i64::from(a.max.x) > lo.x
                && i64::from(a.min.y) < hi.y
                && i64::from(a.max.y) > lo.y
                && i64::from(a.min.z) < hi.z
                && i64::from(a.max.z) > lo.z
        });
        if hit {
            out.push(CellKey::new(Level::L3, l2.coords * 8 + idx));
        }
    }
    out
}

/// Active blocks from a scattering of L2 cells, in a fixed order.
fn sample(count: usize) -> Vec<(CellKey, Vec<MassInfo>)> {
    let mut out = Vec::new();
    let cache = L1Cache::default();
    'outer: for x in -40..40i64 {
        let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 3, 2 * x % 5));
        let _ = L1_CELL;
        for k in 0..512i64 {
            let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
            let cell = CellKey::new(Level::L2, l1.coords * 8 + idx);
            let (_, masses) = l2_build(SEED, &cache, cell);
            if !masses.is_empty() {
                out.push((cell, masses));
                if out.len() >= count {
                    break 'outer;
                }
            }
        }
    }
    out
}

fn inside(bb: &QAabb, o: [Q; 3], masses: &[MassInfo]) -> bool {
    let w = |a: Q, b: Q, k: usize| (a + o[k], b + o[k]);
    let (x0, x1) = w(bb.min.x, bb.max.x, 0);
    let (y0, y1) = w(bb.min.y, bb.max.y, 1);
    let (z0, z1) = w(bb.min.z, bb.max.z, 2);
    masses.iter().any(|m| {
        let a = &m.aabb;
        a.min.x <= x0
            && x1 <= a.max.x
            && a.min.y <= y0
            && y1 <= a.max.y
            && a.min.z <= z0
            && z1 <= a.max.z
    })
}

#[test]
#[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
fn blocks_are_pure_bounded_and_inside_their_envelopes() {
    let cache = L1Cache::default();
    let mut blocks = 0;
    let (mut fallbacks, mut sealed, mut attempts) = (0, 0u32, [0u32; 8]);
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    for (cell, masses) in sample(60) {
        for block in active_blocks(cell, &masses) {
            let t = Instant::now();
            let b: L3Block = l3_block(SEED, &cache, block);
            let dt = t.elapsed().as_secs_f64() * 1000.0;
            total += dt;
            worst = worst.max(dt);
            blocks += 1;
            let again = l3_block(SEED, &cache, block);
            assert_eq!(b.out, again.out, "{block:?} is not pure");
            assert_eq!(b.layout, again.layout);
            fallbacks += usize::from(b.meta.fallback);
            sealed += u32::from(b.meta.sealed_slots);
            attempts[usize::from(b.meta.attempt).min(7)] += 1;
            let b3 = block.coords - cell.coords * 8;
            let o = [b3.x as Q * 1024, b3.y as Q * 1024, b3.z as Q * 1024];
            assert!(
                b.out.primitives.len() <= 4096,
                "{} primitives",
                b.out.primitives.len()
            );
            for p in &b.out.primitives {
                let bb = p.shape.aabb();
                assert!(
                    bb.min.x >= 0
                        && bb.max.x <= 1024
                        && bb.min.y >= 0
                        && bb.max.y <= 1024
                        && bb.min.z >= 0
                        && bb.max.z <= 1024,
                    "{block:?}: {:?} leaves the block: {bb:?}",
                    p.kind
                );
                if !p.flags.contains(PrimFlags::NO_COLLIDE)
                    && !matches!(p.shape, Shape::LiftShaft { .. })
                {
                    assert!(
                        inside(&bb, o, &masses),
                        "{block:?}: {:?} outside every envelope: {bb:?}",
                        p.kind
                    );
                }
            }
        }
    }
    println!(
        "{blocks} blocks, {fallbacks} fallbacks, sealed slots {sealed}, attempts {attempts:?}, mean {:.2} ms, worst {worst:.2} ms",
        total / f64::from(blocks)
    );
    assert!(blocks > 100);
    assert_eq!(attempts.iter().sum::<u32>(), blocks);
    let _ = ATTEMPTS;
}

#[test]
#[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
fn the_fallback_is_always_legal_and_the_solver_rarely_needs_it() {
    use worldgen::l3::{SlotClass, VESTIBULE, l3_block_fallback};
    use worldgen::l3_tiles::{self, D, N_TILES};
    let cache = L1Cache::default();
    let mut checked = 0;
    for (cell, masses) in sample(25) {
        for block in active_blocks(cell, &masses) {
            let b = l3_block_fallback(SEED, &cache, block);
            assert!(b.meta.fallback);
            // Every pair of neighbouring tiles inside the block has equal sockets on the shared
            // side (Lemma L3-1), counting the vestibule as D towards Core neighbours.
            let at = |x: usize, t: usize, z: usize| b.layout.slots[(t * 8 + z) * 8 + x];
            let sock = |s: &worldgen::l3::SlotInfo, side: usize| -> Option<u8> {
                (s.class == SlotClass::Core && usize::from(s.tile) < N_TILES)
                    .then(|| l3_tiles::table()[usize::from(s.tile)].sockets()[side])
            };
            for t in 0..16 {
                for z in 0..8 {
                    for x in 0..8 {
                        let a = at(x, t, z);
                        for (side, (dx, dt, dz)) in
                            [(1usize, (1, 0, 0)), (2, (0, 0, 1)), (5, (0, 1, 0))]
                        {
                            let (nx, nt, nz) = (x + dx, t + dt, z + dz);
                            if nx >= 8 || nt >= 16 || nz >= 8 {
                                continue;
                            }
                            let c = at(nx, nt, nz);
                            if a.class != SlotClass::Core
                                || c.class != SlotClass::Core
                                || a.mass != c.mass
                            {
                                continue;
                            }
                            let opp = worldgen::l3_solver::opposite(side);
                            let (sa, sc) = (sock(&a, side), sock(&c, opp));
                            let (va, vc) = (a.tile == VESTIBULE, c.tile == VESTIBULE);
                            match (sa, sc) {
                                (Some(p), Some(q)) => {
                                    assert_eq!(p, q, "{block:?} ({x},{t},{z}) side {side}");
                                }
                                (None, Some(q)) if va => assert!(q == D || side >= 4 || q == 0),
                                (Some(p), None) if vc => assert!(p == D || side >= 4 || p == 0),
                                _ => {}
                            }
                        }
                    }
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 50, "{checked}");
}

/// P4.1.3: at least 99.9 % of blocks solve before needing the fallback.
#[test]
#[ignore = "10^5 blocks; run in release"]
fn a_hundred_thousand_blocks_rarely_fall_back() {
    let cache = L1Cache::default();
    let (mut blocks, mut fallbacks) = (0u32, 0u32);
    let mut attempts = [0u32; 8];
    let mut max_prims = 0;
    'scan: for x in -400..400i64 {
        let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 4, 2 * x % 7));
        for k in 0..512i64 {
            let idx = I64Vec3::new(k % 8, k / 64, k / 8 % 8);
            let cell = CellKey::new(Level::L2, l1.coords * 8 + idx);
            for (_, b) in worldgen::l3::l3_blocks_of(SEED, &cache, cell) {
                blocks += 1;
                fallbacks += u32::from(b.meta.fallback);
                attempts[usize::from(b.meta.attempt).min(7)] += 1;
                max_prims = max_prims.max(b.out.primitives.len());
                if blocks >= 100_000 {
                    break 'scan;
                }
            }
        }
    }
    println!(
        "{blocks} blocks, {fallbacks} fallbacks, attempts {attempts:?}, most primitives {max_prims}"
    );
    assert!(f64::from(fallbacks) <= f64::from(blocks) * 0.001);
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// 100 golden L3 blocks: digests of the output, the layout and the solve statistics. They are
/// regenerated with `UPDATE_GOLDEN=1 cargo test -p gen --release --test l3`.
#[test]
#[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
fn golden_l3_blocks_are_unchanged() {
    use std::fmt::Write as _;
    let cache = L1Cache::default();
    let mut text = format!("gen_version {}\n", worldgen::GEN_VERSION);
    let mut n = 0;
    'pick: for (cell, masses) in sample(400) {
        // One block per sampled cell, the middle active one, so the list spreads out.
        let blocks = active_blocks(cell, &masses);
        let Some(block) = blocks.get(blocks.len() / 2) else {
            continue;
        };
        let b = l3_block(SEED, &cache, *block);
        let c = block.coords;
        writeln!(
            text,
            "{} {} {} {:016x}",
            c.x,
            c.y,
            c.z,
            fnv(&format!("{:?}{:?}{:?}", b.out, b.layout, b.meta))
        )
        .unwrap();
        n += 1;
        if n == 100 {
            break 'pick;
        }
    }
    assert_eq!(n, 100);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/l3.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let expect = std::fs::read_to_string(&path).expect("run with UPDATE_GOLDEN=1 once");
    assert_eq!(
        text, expect,
        "L3 output changed: bump GEN_VERSION and regenerate"
    );
}

/// P4.1.5: a block solves in under 5 ms on one worker thread, with its parent already built.
#[test]
#[ignore = "timing; run in release"]
fn a_block_solves_in_under_five_milliseconds() {
    use worldgen::l3::{l3_block_with, l3_parent};
    let cache = L1Cache::default();
    let mut times: Vec<f64> = Vec::new();
    for (cell, masses) in sample(80) {
        let parent = l3_parent(SEED, &cache, cell);
        for block in active_blocks(cell, &masses) {
            // The work is deterministic, so the fastest of three runs is the cost without
            // scheduler noise.
            let best = (0..3)
                .map(|_| {
                    let t = Instant::now();
                    std::hint::black_box(l3_block_with(SEED, &parent, block));
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .fold(f64::MAX, f64::min);
            times.push(best);
        }
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{} blocks: mean {mean:.2} ms, median {:.2}, p99 {:.2}, max {:.2}",
        times.len(),
        p(0.5),
        p(0.99),
        times[times.len() - 1]
    );
    assert!(mean < 5.0 && p(0.99) < 5.0, "mean {mean} p99 {}", p(0.99));
}
