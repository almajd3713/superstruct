//! Set pieces SET-01..16 (`levels.md` §5W.8, §5W.22): the places that pay off a route. A set
//! piece is a form like those of `forms.rs` and the hubs, placed early in step 3 (so the cell's
//! fill counts it) from the kinds a cell's context allows.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_wraps
)]

use crate::cell::Axis;
use crate::forms::{FormDef, FormOut};
use crate::l1_artery::ArteryType;
use crate::rng::Rng;
use crate::tissue::{AnchorKind, SlotState, Tissue};
use crate::tissue_grow::{Shaped, SlotBox};

/// `Structure::form` of set piece `n` (0-based) is `SET_BASE + n`.
pub const SET_BASE: usize = 200;

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

/// SET-01: an atrium with galleries: four walls round an open court, a gallery every 3 slots.
fn atrium(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || d < 5 || h < 4 {
        return None;
    }
    let mut boxes = vec![
        bx([0, 0, 0], [w, h, 1]),
        bx([0, 0, d - 1], [w, h, d]),
        bx([0, 0, 1], [1, h, d - 1]),
        bx([w - 1, 0, 1], [w, h, d - 1]),
        bx([0, 0, 0], [w, 1, d]),
    ];
    let mut y = 3;
    while y < h {
        boxes.push(bx([1, y, 1], [w - 1, y + 1, 2]));
        boxes.push(bx([1, y, d - 2], [w - 1, y + 1, d - 1]));
        boxes.push(bx([1, y, 2], [2, y + 1, d - 2]));
        boxes.push(bx([w - 2, y, 2], [w - 1, y + 1, d - 2]));
        y += 3;
    }
    out(boxes, Vec::new(), true)
}

/// SET-02: a light well: a tall ring wall round a shaft of light, on a base.
fn light_well(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 3 || h < 4 {
        return None;
    }
    out(
        vec![bx([0, 0, 0], [dia, 1, dia])],
        vec![(Shaped::Ring(16), bx([0, 1, 0], [dia, h, dia]))],
        true,
    )
}

/// SET-03: a cistern: a broad low tank on a slab.
fn cistern(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 3 || h < 2 {
        return None;
    }
    out(
        vec![bx([0, 0, 0], [dia, 1, dia])],
        vec![(
            Shaped::Cyl,
            bx([0, 1, 0], [dia, (h / 2).max(2).min(h), dia]),
        )],
        false,
    )
}

/// SET-04: an organ room: a hall with a back wall and a row of pipes of rising heights.
fn organ_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 3 || h < 4 {
        return None;
    }
    let pipes = (0..w)
        .map(|x| {
            let top = 2 + (x * 5) % (h - 2);
            (
                Shaped::Cyl,
                bx([x, 1, d - 3], [x + 1, top.min(h - 1), d - 2]),
            )
        })
        .collect();
    out(
        vec![
            bx([0, 0, 0], [w, 1, d]),
            bx([0, h - 1, 0], [w, h, d]),
            bx([0, 1, d - 1], [w, h - 1, d]),
        ],
        pipes,
        false,
    )
}

/// SET-05: a window wall: one thin wall with buttresses, a grid of lit windows.
fn window_wall(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || h < 3 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [w, h, 1])];
    let mut x = 1;
    while x < w && d > 1 {
        boxes.push(bx([x, 0, 1], [x + 1, (h / 2).max(1), 2.min(d)]));
        x += 4;
    }
    out(boxes, Vec::new(), true)
}

/// SET-06: an observation pod: a stalk, an arm and a pod at its end.
fn observation_pod(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || h < 4 || d < 2 {
        return None;
    }
    out(
        vec![bx([0, 0, 0], [1, h, 1]), bx([0, h - 1, 0], [w, h, 1])],
        vec![(Shaped::Cyl, bx([w - 2, h - 3, 0], [w, h - 1, 2.min(d)]))],
        true,
    )
}

/// SET-07: a hanging garden: platforms hung under a beam, with vines.
fn hanging_garden(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 2 || h < 4 {
        return None;
    }
    let mut boxes = vec![bx([0, h - 1, 0], [w, h, d])];
    let mut y = h - 3;
    let mut i = 0;
    while y >= 1 {
        let inset = i.min((w - 1) / 2);
        boxes.push(bx([inset, y, 0], [w - inset, y + 1, d]));
        y -= 2;
        i += 1;
    }
    let vines = (0..w).map(|x| [x, d / 2, 1, h - 1]).collect();
    Some(FormOut {
        boxes,
        vines,
        ..FormOut::default()
    })
}

/// SET-08: a niche or shrine: a small alcove in a wall with a statue, lit.
fn shrine(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 2 || d < 2 || h < 2 {
        return None;
    }
    out(
        vec![bx([0, 0, 0], [w, 1, d]), bx([0, 1, d - 1], [w, h, d])],
        vec![(
            Shaped::Cyl,
            bx([w / 2, 1, d / 2], [w / 2 + 1, 2.min(h), d / 2 + 1]),
        )],
        true,
    )
}

/// SET-09: a generator room: a roofed hall with a row of turbines lying along it.
fn generator_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 3 || h < 3 {
        return None;
    }
    let mut shaped = Vec::new();
    let mut z = 0;
    while z + 2 <= d {
        shaped.push((
            Shaped::CylH(Axis::X),
            bx([1, 1, z], [w - 1, 3.min(h - 1), z + 2]),
        ));
        z += 2;
    }
    out(
        vec![bx([0, 0, 0], [w, 1, d]), bx([0, h - 1, 0], [w, h, d])],
        shaped,
        false,
    )
}

/// SET-10: abandoned dwellings: little boxes piled in a grid, some lit (human traces).
fn dwellings(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 2 || d < 2 || h < 2 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [w, 1, d])];
    for x in 0..w {
        for z in 0..d {
            let n = r.below(h as u64) as i32;
            for y in 1..=n.min(h - 1) {
                if r.below(3) != 0 {
                    boxes.push(bx([x, y, z], [x + 1, y + 1, z + 1]));
                }
            }
        }
    }
    out(boxes, Vec::new(), true)
}

/// SET-11: an interchange: four decks at four heights crossing in one space, on four posts.
fn interchange(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 5 {
        return None;
    }
    let mut boxes = Vec::new();
    for (x, z) in [(0, 0), (w - 1, 0), (0, d - 1), (w - 1, d - 1)] {
        boxes.push(bx([x, 0, z], [x + 1, h, z + 1]));
    }
    for i in 0..4 {
        let y = 1 + i * (h - 2) / 4;
        boxes.push(if i % 2 == 0 {
            bx([0, y, d / 3], [w, y + 1, d - d / 3])
        } else {
            bx([w / 3, y, 0], [w - w / 3, y + 1, d])
        });
    }
    out(boxes, Vec::new(), true)
}

/// SET-12: a lift lobby: a hall with a square shaft in the middle.
fn lift_lobby(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 4 {
        return None;
    }
    let low = (h / 3).max(1);
    out(
        vec![
            bx([0, 0, 0], [w, low, d]),
            bx([w / 2 - 1, low, d / 2 - 1], [w / 2 + 1, h, d / 2 + 1]),
        ],
        Vec::new(),
        true,
    )
}

/// SET-13: a tram stop: a platform under a canopy on posts.
fn tram_stop(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 2 || h < 3 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [w, 1, d]), bx([0, h - 1, 0], [w, h, d])];
    let mut x = 0;
    while x < w {
        boxes.push(bx([x, 1, 0], [x + 1, h - 1, 1]));
        x += 2;
    }
    out(boxes, Vec::new(), true)
}

/// SET-14: a valley balcony: a block with a slab thrust out over the edge and posts.
fn valley_balcony(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let mut boxes = vec![
        bx([0, 0, 0], [w, h, d / 3 + 1]),
        bx([0, h / 2, 0], [w, h / 2 + 1, d]),
    ];
    let mut x = 0;
    while x < w {
        boxes.push(bx([x, h / 2 + 1, d - 1], [x + 1, h / 2 + 2, d]));
        x += 2;
    }
    out(boxes, Vec::new(), true)
}

/// SET-15: a grain seam plaza: a triangular slab, narrowing along its depth.
fn seam_plaza(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, _, d] = s;
    if w < 3 || d < 3 {
        return None;
    }
    let boxes = (0..d)
        .map(|z| bx([0, 0, z], [(w * (d - z) / d).max(1), 1, z + 1]))
        .collect();
    out(boxes, Vec::new(), true)
}

/// SET-16: a tether foot: a slab with a collar where the tether lands.
fn tether_foot(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 2 {
        return None;
    }
    out(
        vec![bx([0, 0, 0], [w, 1, d])],
        vec![(
            Shaped::Cyl,
            bx([w / 2 - 1, 1, d / 2 - 1], [w / 2 + 1, 3.min(h), d / 2 + 1]),
        )],
        true,
    )
}

macro_rules! set {
    ($tag:expr, $min:expr, $max:expr, $f:expr) => {
        FormDef {
            tag: $tag,
            family: 8,
            size_m: ($min, $max),
            sealed: false,
            build: $f,
        }
    };
}

/// The set pieces, in registry order. Sizes are `[width, depth, height]` in metres.
pub const SETS: &[FormDef] = &[
    set!("SET-01", [128, 128, 128], [256, 256, 320], atrium),
    set!("SET-02", [96, 96, 160], [192, 192, 384], light_well),
    set!("SET-03", [96, 96, 64], [256, 256, 128], cistern),
    set!("SET-04", [128, 96, 128], [288, 192, 256], organ_room),
    set!("SET-05", [128, 32, 96], [320, 64, 256], window_wall),
    set!("SET-06", [96, 64, 128], [192, 96, 256], observation_pod),
    set!("SET-07", [96, 64, 128], [224, 160, 256], hanging_garden),
    set!("SET-08", [64, 64, 64], [128, 128, 128], shrine),
    set!("SET-09", [128, 96, 96], [288, 224, 192], generator_room),
    set!("SET-10", [96, 96, 96], [224, 224, 192], dwellings),
    set!("SET-11", [128, 128, 192], [256, 256, 320], interchange),
    set!("SET-12", [96, 96, 128], [192, 192, 256], lift_lobby),
    set!("SET-13", [96, 64, 96], [224, 128, 128], tram_stop),
    set!("SET-14", [96, 96, 96], [224, 224, 192], valley_balcony),
    set!("SET-15", [128, 128, 64], [288, 288, 96], seam_plaza),
    set!("SET-16", [96, 96, 64], [192, 192, 128], tether_foot),
];

/// The set pieces a cell can take, as `(0-based index, weight)`, from its context.
pub fn legal(t: &Tissue) -> Vec<(usize, u32)> {
    let mut v: Vec<(usize, u32)> = vec![(2, 2), (4, 2), (6, 1), (7, 3), (9, 2)];
    if t.fill >= 128 {
        v.extend([(0, 3), (1, 2), (3, 2), (8, 2)]);
    }
    let lat = &t.lattice;
    let void_edge = (0..lat.len()).any(|i| {
        lat.present[i]
            && lat.state[i] == SlotState::Free
            && lat.neighbours(i).any(|j| lat.state[j] == SlotState::Void)
    });
    if void_edge {
        v.extend([(5, 3), (13, 3)]);
    }
    if t.anchors.len() >= 4 {
        v.push((10, 3));
    }
    let port = |kind: ArteryType| {
        t.anchors
            .iter()
            .any(|a| matches!(a.kind, AnchorKind::Port { kind: k, .. } if k == kind))
    };
    if port(ArteryType::LiftTrunk) {
        v.push((11, 4));
    }
    if port(ArteryType::TramTube) {
        v.push((12, 4));
    }
    if t.seam {
        v.push((14, 4));
    }
    if t.tether_foot.is_some() {
        v.push((15, 4));
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

    /// Every set piece builds at piece-like sizes and keeps every piece inside its size.
    #[test]
    fn every_set_piece_builds_inside_its_size() {
        for (i, s) in SETS.iter().enumerate() {
            let mut built = 0;
            for size in [[4, 4, 4], [8, 8, 8], [12, 6, 5], [6, 12, 4], [10, 10, 10]] {
                let mut r = Rng::new(3 + i as u64);
                let Some(o) = (s.build)(size, &mut r) else {
                    continue;
                };
                built += 1;
                for b in o.envelope() {
                    for k in 0..3 {
                        assert!(
                            b.lo[k] >= 0 && b.hi[k] <= size[k] && b.lo[k] < b.hi[k],
                            "{} {size:?}: {b:?}",
                            s.tag
                        );
                    }
                }
            }
            assert!(built >= 3, "{} builds at {built} of 5 sizes", s.tag);
        }
    }

    /// SET-01..16 all occur in real tissue, placed as themselves.
    #[test]
    fn every_set_piece_occurs() {
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let mut seen = [0usize; 17];
        for k in 0..3000i64 {
            let l1 = I64Vec3::new(k % 9 - 4, k / 9 % 3, k / 27 % 40 - 20);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(seed, &cache, key);
            for &(id, si) in &t.sets {
                seen[usize::from(id)] += 1;
                let s = &t.structures[usize::from(si)];
                assert_eq!(usize::from(s.form), SET_BASE + usize::from(id) - 1);
            }
        }
        eprintln!("set pieces per type {:?}", &seen[1..]);
        for id in 1..=16 {
            assert!(seen[id] > 0, "SET-{id:02} never occurs: {seen:?}");
        }
    }
}
