//! Tram line consistency (`docs/spec/trams.md` §9.1, P3.4.3).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::fmt::Write as _;

use glam::I64Vec3;
use worldgen::cell::{Axis, CellKey, Level};
use worldgen::hash::hash;
use worldgen::key::LayerKey;
use worldgen::l1::{CELL, decks, l1_cell};
use worldgen::prim::{PrimFlags, Shape};
use worldgen::seed::WorldSeed;
use worldgen::tram::{TramLine, lines_in_cell, tram_lines, tram_pieces};

const SEED: WorldSeed = WorldSeed(0x7A11);

fn layers(n: usize) -> Vec<LayerKey> {
    let mut out = Vec::new();
    let mut i = 0u64;
    while out.len() < n {
        let h = |salt: u64, m: u64| {
            i64::try_from(
                hash(
                    SEED.0,
                    Level::L1,
                    I64Vec3::new(i64::try_from(i).unwrap(), 0, 0),
                    salt,
                ) % m,
            )
            .unwrap()
        };
        let layer = LayerKey {
            level: Level::L1,
            y: h(1, 32) - 16,
            rx: h(2, 128) - 64,
            rz: h(3, 128) - 64,
        };
        i += 1;
        if !decks(SEED, layer).is_empty() {
            out.push(layer);
        }
    }
    out
}

fn run_cells(l: &TramLine) -> Vec<CellKey> {
    (l.cells.0..=l.cells.1)
        .map(|a| {
            let (x, z) = if l.axis == Axis::X {
                (a, l.column)
            } else {
                (l.column, a)
            };
            CellKey::new(Level::L1, I64Vec3::new(x, l.id.layer.y, z))
        })
        .collect()
}

#[test]
fn lines_agree_in_every_cell_on_any_thread() {
    let mut lines = Vec::new();
    for layer in layers(300) {
        lines.extend(tram_lines(SEED, layer));
    }
    assert!(!lines.is_empty());
    // Probe cells in a fixed shuffled order, each from scratch, on 8 threads.
    let mut probes: Vec<(CellKey, TramLine, bool)> = Vec::new();
    for l in &lines {
        for c in run_cells(l) {
            probes.push((c, l.clone(), true));
        }
        let c0 = run_cells(l)[0].coords;
        let off = if l.axis == Axis::X {
            I64Vec3::new(-1, 0, 0)
        } else {
            I64Vec3::new(0, 0, -1)
        };
        probes.push((CellKey::new(Level::L1, c0 + off), l.clone(), false));
        let side = if l.axis == Axis::X {
            I64Vec3::new(0, 0, 1)
        } else {
            I64Vec3::new(1, 0, 0)
        };
        probes.push((CellKey::new(Level::L1, c0 + side), l.clone(), false));
    }
    probes.sort_by_key(|(c, _, _)| hash(SEED.0, Level::L1, c.coords, 0xFEED));
    let chunks: Vec<_> = probes.chunks(probes.len().div_ceil(8)).collect();
    let results: Vec<Vec<String>> = std::thread::scope(|s| {
        chunks
            .iter()
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|(cell, l, inside)| {
                            let got = lines_in_cell(SEED, *cell);
                            assert_eq!(
                                got.contains(l),
                                *inside,
                                "cell {cell:?} and line {:?}",
                                l.id
                            );
                            format!("{cell:?} {got:?}")
                        })
                        .collect()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    assert_eq!(results.iter().map(Vec::len).sum::<usize>(), probes.len());
}

type Key = (i32, i32, i32, i32, u16, u8, u8);

fn pieces_at(cell: CellKey, axis: Axis, face_high: bool) -> Vec<Key> {
    let mut v: Vec<Key> = tram_pieces(SEED, cell)
        .iter()
        .filter_map(|p| {
            let Shape::Box { aabb } = p.shape else {
                panic!("tram pieces are boxes")
            };
            let (lo, hi, c0, c1) = if axis == Axis::X {
                (aabb.min.x, aabb.max.x, aabb.min.z, aabb.max.z)
            } else {
                (aabb.min.z, aabb.max.z, aabb.min.x, aabb.max.x)
            };
            let touches = if face_high { hi == CELL } else { lo == 0 };
            touches.then_some((
                c0,
                c1,
                aabb.min.y,
                aabb.max.y,
                p.material.0,
                p.kind.0,
                p.flags.0,
            ))
        })
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn pieces_meet_exactly_at_shared_faces() {
    let mut checked = 0;
    for layer in layers(300) {
        for l in tram_lines(SEED, layer) {
            let cells = run_cells(&l);
            for w in cells.windows(2) {
                let (a, b) = (
                    pieces_at(w[0], l.axis, true),
                    pieces_at(w[1], l.axis, false),
                );
                // Rails, strips and (when the end is not here) nothing else cross a face.
                assert!(!a.is_empty());
                assert_eq!(a, b, "seam between {:?} and {:?}", w[0], w[1]);
                checked += 1;
            }
            for c in &cells {
                for p in tram_pieces(SEED, *c) {
                    assert!(p.flags.0 & PrimFlags::NO_COLLIDE.0 != 0);
                    let bb = p.aabb();
                    assert!(bb.min.x >= 0 && bb.max.x <= CELL && bb.min.z >= 0 && bb.max.z <= CELL);
                }
            }
        }
    }
    assert!(checked > 5, "only {checked} seams");
}

#[test]
fn nothing_collidable_enters_the_envelope() {
    let mut cells = 0;
    for layer in layers(120) {
        for l in tram_lines(SEED, layer) {
            for c in run_cells(&l) {
                let out = l1_cell(SEED, c);
                for e in worldgen::tram::envelopes(SEED, c) {
                    for p in &out.primitives {
                        if p.flags.0 & PrimFlags::NO_COLLIDE.0 != 0
                            || matches!(p.shape, Shape::LiftShaft { .. })
                        {
                            continue;
                        }
                        let b = p.aabb();
                        let overlap = b.min.x < e.max.x
                            && b.max.x > e.min.x
                            && b.min.y < e.max.y
                            && b.max.y > e.min.y
                            && b.min.z < e.max.z
                            && b.max.z > e.min.z;
                        assert!(
                            !overlap,
                            "{:?} in {c:?} meets the envelope of {:?}",
                            p.kind, l.id
                        );
                    }
                }
                cells += 1;
            }
        }
    }
    assert!(cells > 10, "only {cells} cells checked");
}

#[test]
fn line_statistics_and_station_spacing() {
    let (mut decks_n, mut lines_n, mut full) = (0usize, 0usize, 0usize);
    for layer in layers(2000) {
        decks_n += decks(SEED, layer).len();
        for l in tram_lines(SEED, layer) {
            lines_n += 1;
            full += usize::from(l.cells.1 - l.cells.0 == 3);
            for w in l.stations.windows(2) {
                let d = w[1].s - w[0].s;
                assert!(d > 0);
                if l.stations.len() > 2 {
                    assert!(d >= 14 * 512 * 16, "stations {d} Q apart");
                }
            }
            for st in &l.stations {
                let b = st.boarding;
                assert!(b.x0 >= 0 && b.z0 >= 0 && b.x1 <= CELL && b.z1 <= CELL);
            }
        }
    }
    let share = lines_n as f64 / decks_n as f64;
    println!(
        "{lines_n} lines on {decks_n} decks: {share:.2}, full {:.2}",
        full as f64 / lines_n as f64
    );
    assert!((0.4..=0.55).contains(&share), "line share {share}");
}

#[test]
fn golden_digests_are_pinned() {
    let mut text = format!("gen_version {}\n", worldgen::GEN_VERSION);
    let mut layers_seen = 0;
    for layer in layers(400) {
        let lines = tram_lines(SEED, layer);
        if lines.is_empty() {
            continue;
        }
        let digest = fnv(&format!("{lines:?}"));
        let piece_digest = fnv(&format!("{:?}", tram_pieces(SEED, run_cells(&lines[0])[0])));
        writeln!(
            text,
            "{} {} {} {digest:016x} {piece_digest:016x}",
            layer.rx, layer.y, layer.rz
        )
        .unwrap();
        layers_seen += 1;
        if layers_seen == 100 {
            break;
        }
    }
    assert_eq!(layers_seen, 100);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/trams.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let expect = std::fs::read_to_string(&path).expect("run with UPDATE_GOLDEN=1 once");
    assert_eq!(
        text, expect,
        "tram output changed: bump GEN_VERSION and regenerate"
    );
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn no_l2_primitive_enters_the_envelope() {
    use worldgen::l2::l2_cell_in;
    use worldgen::l2_port::L1Cache;
    const L2: i32 = 512 * 16;
    let mut checked = 0;
    for layer in layers(60) {
        for l in tram_lines(SEED, layer) {
            for c in run_cells(&l) {
                for e in worldgen::tram::envelopes(SEED, c) {
                    let ratio = i64::from(CELL / L2);
                    let lo = |q: i32| i64::from(q.div_euclid(L2));
                    let hi = |q: i32| i64::from((q - 1).div_euclid(L2));
                    for ix in lo(e.min.x)..=hi(e.max.x) {
                        for iy in lo(e.min.y)..=hi(e.max.y) {
                            for iz in lo(e.min.z)..=hi(e.max.z) {
                                let k = CellKey::new(
                                    Level::L2,
                                    c.coords * ratio + I64Vec3::new(ix, iy, iz),
                                );
                                let out = l2_cell_in(SEED, &L1Cache::default(), k);
                                let o = [ix, iy, iz].map(|v| i32::try_from(v).unwrap() * L2);
                                for p in &out.primitives {
                                    if p.flags.0 & PrimFlags::NO_COLLIDE.0 != 0
                                        || matches!(p.shape, Shape::LiftShaft { .. })
                                    {
                                        continue;
                                    }
                                    let b = p.aabb();
                                    let hit = b.min.x + o[0] < e.max.x
                                        && b.max.x + o[0] > e.min.x
                                        && b.min.y + o[1] < e.max.y
                                        && b.max.y + o[1] > e.min.y
                                        && b.min.z + o[2] < e.max.z
                                        && b.max.z + o[2] > e.min.z;
                                    assert!(
                                        !hit,
                                        "{:?} in L2 {k:?} meets the envelope of {:?}",
                                        p.kind, l.id
                                    );
                                }
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 50, "only {checked} L2 cells checked");
}
