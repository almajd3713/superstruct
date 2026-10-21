//! Sanctuaries, batch 2 (SAN-01..13, `docs/proposals/the-web-ideas.md` §17): small places to rest,
//! as set pieces (`sets.rs` appends them to `SETS` as indices 16 to 28). Spec: `docs/spec/levels.md`
//! §5W.43. Sizes are `[width, height, depth]` in slots of 32 m; every one is lit.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::unnecessary_wraps
)]

use crate::forms::FormOut;
use crate::rng::Rng;
use crate::tissue_grow::{Shaped, SlotBox};

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

fn lit(boxes: Vec<SlotBox>, shaped: Vec<(Shaped, SlotBox)>) -> Option<FormOut> {
    let boxes: Vec<SlotBox> = boxes
        .into_iter()
        .filter(|b| (0..3).all(|k| b.lo[k] < b.hi[k]))
        .collect();
    if boxes.is_empty() && shaped.is_empty() {
        return None;
    }
    Some(FormOut {
        boxes,
        shaped,
        lit: true,
        ..FormOut::default()
    })
}

/// SAN-01: a tea room over a Between: a slab and a small room set back on it, a terrace in front.
pub fn tea_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 2 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [w, 1, d]), bx([1, 1, 1], [w - 1, h, d - 1])],
        Vec::new(),
    )
}

/// SAN-02: a greenhouse under a small lamp sun: a low hall with a dome on it.
pub fn greenhouse(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 3 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [w, 2, d])],
        vec![(Shaped::Dome, bx([1, 2, 1], [w - 1, h, d - 1]))],
    )
}

/// SAN-03: a library nook: a small room with shelving in its walls.
pub fn library_nook(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 2 || d < 2 || h < 2 {
        return None;
    }
    lit(
        vec![
            bx([0, 0, 0], [w, h.min(2), d]),
            bx([0, 0, 0], [1, h, d]),
            bx([w - 1, 0, 0], [w, h, d]),
        ],
        Vec::new(),
    )
}

/// SAN-04: a lamp chapel: a nave with a lit spire.
pub fn lamp_chapel(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 4 || h < 4 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [w, h - 2, d])],
        vec![(
            Shaped::Cone { inverted: false },
            bx([w / 2 - 1, h - 2, d / 2 - 1], [w / 2 + 1, h, d / 2 + 1]),
        )],
    )
}

/// SAN-05: a bunk room: a low room with three tiers of bunks.
pub fn bunk_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 2 {
        return None;
    }
    let mut b = vec![bx([0, 0, 0], [w, h, d])];
    for y in 1..h.min(4) {
        b.push(bx([1, y, 1], [w - 1, y + 1, d - 1]));
    }
    lit(b, Vec::new())
}

/// SAN-06: an observatory: a room with a dome and a telescope slit.
pub fn observatory(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [w, h - 1, d])],
        vec![(Shaped::Dome, bx([0, h - 1, 0], [w, h, d]))],
    )
}

/// SAN-07: the top of a needle: a thin shaft with a room at its top.
pub fn needle_top(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 4 {
        return None;
    }
    lit(
        vec![
            bx([w / 2 - 1, 0, d / 2 - 1], [w / 2 + 1, h - 2, d / 2 + 1]),
            bx([0, h - 2, 0], [w, h, d]),
        ],
        Vec::new(),
    )
}

/// SAN-08: a gondola cabin parked at a tower: a tower with a small cabin hung beside it.
pub fn gondola_stop(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 2 || h < 3 {
        return None;
    }
    lit(
        vec![
            bx([0, 0, 0], [1, h, d.min(2)]),
            bx([1, h / 2, 0], [3.min(w), h / 2 + 1, d.min(2)]),
            bx([2.min(w - 1), h / 2 - 1, 0], [w, h / 2, d.min(2)]),
        ],
        Vec::new(),
    )
}

/// SAN-09: a pool room: a wide low hall around a sunk pool.
pub fn pool_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 2 {
        return None;
    }
    lit(
        vec![
            bx([0, 0, 0], [w, 1, d]),
            bx([0, 1, 0], [w, h, 1]),
            bx([0, 1, d - 1], [w, h, d]),
            bx([0, 1, 1], [1, h, d - 1]),
            bx([w - 1, 1, 1], [w, h, d - 1]),
        ],
        Vec::new(),
    )
}

/// SAN-10: a bell room: a room with the bell's drum standing in the middle.
pub fn bell_room(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [w, 1, d]), bx([0, h - 1, 0], [w, h, d])],
        vec![(Shaped::Cyl, bx([1, 1, 1], [w - 1, h - 1, d - 1]))],
    )
}

/// SAN-11: a garden in a light well: a ring wall with vines down its inside.
pub fn light_well_garden(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 3 {
        return None;
    }
    let vines = (0..4)
        .map(|_| [r.below(dia as u64) as i32, r.below(dia as u64) as i32, 1, h])
        .collect();
    Some(FormOut {
        boxes: vec![bx([0, 0, 0], [dia, 1, dia])],
        shaped: vec![(Shaped::Ring(8), bx([0, 1, 0], [dia, h, dia]))],
        vines,
        lit: true,
        ..FormOut::default()
    })
}

/// SAN-12: a pod hung under a crust: a slab to hang from and a capsule below it.
pub fn hanging_pod(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    lit(
        vec![bx([0, h - 1, 0], [w, h, d])],
        vec![(Shaped::Cyl, bx([1, 0, 1], [w - 1, h - 1, d - 1]))],
    )
}

/// SAN-13: a place you reach only by a long ladder or a featherfall: a room on top of a pillar
/// with a cage up its side for the ladder.
pub fn aloft(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 6 {
        return None;
    }
    lit(
        vec![bx([0, 0, 0], [1, h - 2, 1]), bx([0, h - 2, 0], [w, h, d])],
        vec![(Shaped::Cage, bx([1, 0, 0], [2, h - 2, 1]))],
    )
}
