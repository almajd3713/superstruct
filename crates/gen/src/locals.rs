//! Local places LOC-01..06 (`levels.md` §5W.24; web §4.3, the 100 m scale): a light well ringed
//! by galleries, an atrium, a thin spot in a crust, a cut-away building, a sunk courtyard and a
//! gap between two leaning towers. Forms like the set pieces, placed after them with chance
//! 1/4 per cell.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_wraps
)]

use crate::forms::{FormDef, FormOut, Turned};
use crate::rng::Rng;
use crate::tissue::Tissue;
use crate::tissue_grow::{Shaped, SlotBox};

/// `Structure::form` of local place `n` (0-based) is `LOC_BASE + n`.
pub const LOC_BASE: usize = 300;

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

fn out(boxes: Vec<SlotBox>, shaped: Vec<(Shaped, SlotBox)>, lit: bool) -> Option<FormOut> {
    Some(FormOut {
        boxes,
        shaped,
        lit,
        ..FormOut::default()
    })
}

/// The four walls of a court `w` by `d`, from height `y0` to `h`.
fn walls(w: i32, d: i32, y0: i32, h: i32) -> Vec<SlotBox> {
    vec![
        bx([0, y0, 0], [w, h, 1]),
        bx([0, y0, d - 1], [w, h, d]),
        bx([0, y0, 1], [1, h, d - 1]),
        bx([w - 1, y0, 1], [w, h, d - 1]),
    ]
}

/// LOC-01: a light well ringed by galleries: a tall ring wall with a gallery every 2 slots.
fn ringed_well(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 5 || h < 4 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [dia, 1, dia])];
    let mut y = 2;
    while y < h {
        boxes.push(bx([1, y, 1], [dia - 1, y + 1, 2]));
        boxes.push(bx([1, y, dia - 2], [dia - 1, y + 1, dia - 1]));
        boxes.push(bx([1, y, 2], [2, y + 1, dia - 2]));
        boxes.push(bx([dia - 2, y, 2], [dia - 1, y + 1, dia - 1]));
        y += 2;
    }
    out(
        boxes,
        vec![(Shaped::Ring(16), bx([0, 1, 0], [dia, h, dia]))],
        true,
    )
}

/// LOC-02: an atrium: four walls and a floor under a glazed roof.
fn atrium_roofed(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 3 {
        return None;
    }
    let mut boxes = walls(w, d, 0, h);
    boxes.push(bx([0, 0, 0], [w, 1, d]));
    boxes.push(bx([0, h - 1, 0], [w, h, d]));
    out(boxes, Vec::new(), true)
}

/// LOC-03: a thin spot: a slab two slots thick with a great lit window opening onto the Between.
fn thin_spot(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, _, d] = s;
    if w < 4 || d < 4 {
        return None;
    }
    let (x0, x1, z0, z1) = (w / 3, w - w / 3, d / 3, d - d / 3);
    out(
        vec![
            bx([0, 0, 0], [w, 2, z0]),
            bx([0, 0, z1], [w, 2, d]),
            bx([0, 0, z0], [x0, 2, z1]),
            bx([x1, 0, z0], [w, 2, z1]),
        ],
        Vec::new(),
        true,
    )
}

/// LOC-04: a cut-away: a building with its front wall missing, every floor open to view.
fn cut_away(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 4 {
        return None;
    }
    let mut boxes = vec![
        bx([0, 0, d - 1], [w, h, d]),
        bx([0, 0, 0], [1, h, d]),
        bx([w - 1, 0, 0], [w, h, d]),
    ];
    let mut y = 0;
    while y < h {
        boxes.push(bx([0, y, 0], [w, y + 1, d]));
        y += 2;
    }
    out(boxes, Vec::new(), true)
}

/// LOC-05: a courtyard sunk into a block: a low block with an open court in the middle.
fn sunk_courtyard(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || d < 5 || h < 2 {
        return None;
    }
    let (x0, x1, z0, z1) = (w / 3, w - w / 3, d / 3, d - d / 3);
    out(
        vec![
            bx([0, 0, 0], [w, 1, d]),
            bx([0, 1, 0], [w, h, z0]),
            bx([0, 1, z1], [w, h, d]),
            bx([0, 1, z0], [x0, h, z1]),
            bx([x1, 1, z0], [w, h, z1]),
        ],
        Vec::new(),
        true,
    )
}

/// LOC-06: a gap between two towers leaning towards each other.
fn leaning_gap(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 6 || d < 2 || h < 4 {
        return None;
    }
    let turned = |x0: i32, yaw: u8| Turned {
        bx: bx([x0, 0, 0], [x0 + 2, h, d]),
        yaw,
        tilt: 1,
        pivot: None,
    };
    Some(FormOut {
        turned: vec![turned(0, 0), turned(w - 2, 32)],
        ..FormOut::default()
    })
}

macro_rules! local {
    ($tag:expr, $min:expr, $max:expr, $f:expr) => {
        FormDef {
            tag: $tag,
            family: 9,
            size_m: ($min, $max),
            sealed: false,
            build: $f,
        }
    };
}

/// The local places, in registry order. Sizes are `[width, depth, height]` in metres.
pub const LOCALS: &[FormDef] = &[
    local!("LOC-01", [160, 160, 160], [288, 288, 320], ringed_well),
    local!("LOC-02", [128, 128, 96], [256, 256, 192], atrium_roofed),
    local!("LOC-03", [128, 128, 64], [288, 288, 96], thin_spot),
    local!("LOC-04", [96, 96, 128], [224, 192, 320], cut_away),
    local!("LOC-05", [160, 160, 64], [320, 320, 96], sunk_courtyard),
    local!("LOC-06", [192, 64, 128], [352, 160, 320], leaning_gap),
];

/// The local places a cell can take, as `(0-based index, weight)`.
pub fn legal(t: &Tissue) -> Vec<(usize, u32)> {
    let mut v: Vec<(usize, u32)> = Vec::new();
    if t.fill >= 128 {
        v.extend([(0, 2), (1, 2)]);
    }
    if t.fill >= 64 {
        v.extend([(3, 2), (4, 2)]);
    }
    if t.fill >= 96 {
        v.push((5, 2));
    }
    // A thin spot: a column of the lattice with one or two crust slots only.
    let lat = &t.lattice;
    let n = lat.n;
    let thin = (0..n).any(|x| {
        (0..n).any(|z| {
            let crust = (0..n)
                .filter(|&y| {
                    let i = lat.index([x, y, z]);
                    lat.present[i] && t.in_crust[i]
                })
                .count();
            (1..=2).contains(&crust)
        })
    });
    if thin {
        v.push((2, 4));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellKey, Level};
    use crate::seed::WorldSeed;
    use crate::tissue::{TissueCache, tissue_cell};
    use glam::I64Vec3;

    /// Every local place builds at its sizes and keeps every piece inside them.
    #[test]
    fn every_local_place_builds_inside_its_size() {
        for (i, l) in LOCALS.iter().enumerate() {
            let mut built = 0;
            for size in [[5, 4, 5], [8, 8, 8], [12, 6, 5], [6, 12, 4], [10, 10, 10]] {
                let mut r = Rng::new(5 + i as u64);
                let Some(o) = (l.build)(size, &mut r) else {
                    continue;
                };
                built += 1;
                for b in o.envelope() {
                    for k in 0..3 {
                        assert!(
                            b.lo[k] >= 0 && b.hi[k] <= size[k] && b.lo[k] < b.hi[k],
                            "{} {size:?}: {b:?}",
                            l.tag
                        );
                    }
                }
            }
            assert!(built >= 3, "{} builds at {built} of 5 sizes", l.tag);
        }
    }

    /// LOC-01..06 all occur in real tissue, placed as themselves.
    #[test]
    fn every_local_place_occurs() {
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let mut seen = [0usize; 7];
        for k in 0..2000i64 {
            let l1 = I64Vec3::new(k % 9 - 4, k / 9 % 3, k / 27 % 40 - 20);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(seed, &cache, key);
            for &(id, si) in &t.locals {
                seen[usize::from(id)] += 1;
                let s = &t.structures[usize::from(si)];
                assert_eq!(usize::from(s.form), LOC_BASE + usize::from(id) - 1);
            }
        }
        eprintln!("local places per type {:?}", &seen[1..]);
        for id in 1..=6 {
            assert!(seen[id] > 0, "LOC-{id:02} never occurs: {seen:?}");
        }
    }
}
