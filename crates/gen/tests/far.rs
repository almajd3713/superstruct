//! Far-coordinate test (P0.11.3): cells near ±2⁵⁰ sectors (2⁶⁰ m) generate without overflow or
//! panics, and their connectivity checks pass.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use glam::I64Vec3;
use worldgen::cell::{CellKey, Dir6, FaceKey, Level};
use worldgen::l1::l1_cell;
use worldgen::l2_port::{L1Cache, face_ports};
use worldgen::regime::regime;
use worldgen::seed::WorldSeed;
use worldgen::validate::{check_cell, check_l1};

/// 2⁵⁰ sectors of 1024 m, in L1 cells of 4096 m.
const FAR_L1: i64 = 1 << 48;

#[test]
fn cells_near_two_to_the_fifty_sectors() {
    let seed = WorldSeed(0xFA2);
    let (mut cells, mut ports, mut with_content) = (0, 0, 0);
    for sign in [-1i64, 1] {
        // L0 regimes at the same distance.
        for d in 0..20 {
            let l0 = I64Vec3::new(sign * (FAR_L1 / 8) + d, d % 3, sign * (FAR_L1 / 8) - d);
            let _ = regime(seed, l0);
        }
        for d in 0..40i64 {
            let parent = CellKey::new(
                Level::L1,
                I64Vec3::new(sign * FAR_L1 - d * 7, d % 3 - 1, sign * FAR_L1 + d * 5),
            );
            assert!(parent.in_domain());
            let cache = L1Cache::default();
            assert!(check_l1(seed, &cache, parent).is_empty(), "{parent:?}");
            with_content += usize::from(!l1_cell(seed, parent).primitives.is_empty());
            for k in 0..512i64 {
                let cell = CellKey::new(
                    Level::L2,
                    parent.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
                );
                let r = check_cell(seed, &cache, cell);
                assert!(r.failures.is_empty(), "{cell:?}: {:?}", r.failures);
                cells += 1;
                ports += r.ports;
                // Both cells beside a face agree on its ports.
                if let Ok(other) = cell.neighbor(Dir6::PosX) {
                    let face = FaceKey::of(cell, Dir6::PosX).unwrap();
                    assert_eq!(FaceKey::of(other, Dir6::NegX).unwrap(), face);
                    let _ = face_ports(seed, face);
                }
            }
        }
    }
    assert!(
        cells >= 40_000 && with_content > 10 && ports > 1000,
        "{cells} {with_content} {ports}"
    );
}
