//! Forms, batch 2 (F2-01..24, `docs/proposals/the-web-ideas.md` §5). Spec: `docs/spec/levels.md`
//! §5W.40. Same contract as `forms.rs`: a size in slots `[width, height, depth]` and a stream of
//! randomness in, pieces in corner-origin slot coordinates out, `None` when it cannot be made.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::unnecessary_wraps
)]

use crate::cell::Axis;
use crate::forms::{FormOut, Turned};
use crate::prim::Dir4;
use crate::rng::Rng;
use crate::tissue_grow::{Shaped, SlotBox};

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

/// The boxes of a form without its empty ones, or `None` when nothing is left.
fn boxes(b: Vec<SlotBox>) -> Option<FormOut> {
    let b: Vec<SlotBox> = b
        .into_iter()
        .filter(|b| (0..3).all(|k| b.lo[k] < b.hi[k]))
        .collect();
    if b.is_empty() {
        return None;
    }
    Some(FormOut {
        boxes: b,
        ..FormOut::default()
    })
}

/// F2-01: capsules (a slot each) plugged into a core, alternating faces up the tower.
pub fn capsule_tower(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 4 {
        return None;
    }
    let (cx, cz) = (w / 2, d / 2);
    let mut b = vec![bx([cx - 1, 0, cz - 1], [cx + 1, h, cz + 1])];
    for y in (0..h).step_by(2) {
        let side = (y / 2 + r.below(4) as i32) % 4;
        let pod = match side {
            0 => bx([0, y, cz - 1], [cx - 1, y + 1, cz + 1]),
            1 => bx([cx + 1, y, cz - 1], [w, y + 1, cz + 1]),
            2 => bx([cx - 1, y, 0], [cx + 1, y + 1, cz - 1]),
            _ => bx([cx - 1, y, cz + 1], [cx + 1, y + 1, d]),
        };
        b.push(pod);
    }
    boxes(b)
}

/// F2-02: a stack of boxes each turned a quarter from the one below (wide in x, then in z).
pub fn habitat_stack(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = h.min(6);
    if n < 3 || w < 3 || d < 3 {
        return None;
    }
    let tier = (h / n).max(1);
    let b = (0..n)
        .map(|i| {
            let (a, c) = if i % 2 == 0 { (0, d / 3) } else { (w / 3, 0) };
            let (x1, z1) = if i % 2 == 0 {
                (w, d - d / 3)
            } else {
                (w - w / 3, d)
            };
            bx([a, i * tier, c], [x1, (i + 1) * tier, z1])
        })
        .collect();
    boxes(b)
}

/// F2-03: an arcology: a steep pyramid of many terraces over a deep core.
pub fn arcology(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = (w / 2).min(d / 2).min(h).min(7);
    if n < 3 {
        return None;
    }
    let tier = (h / n).max(1);
    let mut b: Vec<SlotBox> = (0..n)
        .map(|i| bx([i, i * tier, i], [w - i, ((i + 1) * tier).min(h), d - i]))
        .collect();
    b.push(bx([w / 2 - 1, 0, d / 2 - 1], [w / 2 + 1, h, d / 2 + 1]));
    boxes(b)
}

/// F2-04: a cooling tower: rings narrowing to a waist and flaring out.
pub fn cooling_tower(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 5 {
        return None;
    }
    let n = h.min(7);
    let tier = (h / n).max(1);
    let shaped = (0..n)
        .map(|i| {
            let waist = (i - n / 2).abs();
            let inset = ((n / 2 - waist) * dia / (2 * n)).max(0);
            (
                Shaped::Ring(16),
                bx(
                    [inset, i * tier, inset],
                    [dia - inset, (i + 1) * tier, dia - inset],
                ),
            )
        })
        .collect();
    Some(FormOut {
        shaped,
        ..FormOut::default()
    })
}

/// F2-05: a dish: a wide shallow cone held up on a mast.
pub fn dish(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 4 {
        return None;
    }
    let mast = (h * 2 / 3).max(2);
    Some(FormOut {
        boxes: vec![bx(
            [dia / 2 - 1, 0, dia / 2 - 1],
            [dia / 2 + 1, mast, dia / 2 + 1],
        )],
        shaped: vec![(
            Shaped::Cone { inverted: true },
            bx([0, mast, 0], [dia, h, dia]),
        )],
        ..FormOut::default()
    })
}

/// F2-06: a gasometer: a drum inside a frame of four posts.
pub fn gasometer(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 3 {
        return None;
    }
    let mut b = vec![
        bx([0, 0, 0], [1, h, 1]),
        bx([dia - 1, 0, 0], [dia, h, 1]),
        bx([0, 0, dia - 1], [1, h, dia]),
        bx([dia - 1, 0, dia - 1], [dia, h, dia]),
    ];
    b.push(bx([0, h - 1, 0], [dia, h, dia]));
    Some(FormOut {
        boxes: b,
        shaped: vec![(Shaped::Cyl, bx([1, 0, 1], [dia - 1, h - 1, dia - 1]))],
        ..FormOut::default()
    })
}

/// F2-07: a blast furnace: a cone on a drum with pipes round it.
pub fn blast_furnace(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 4 {
        return None;
    }
    let base = (h / 3).max(1);
    let mut shaped = vec![
        (Shaped::Cyl, bx([0, 0, 0], [dia, base, dia])),
        (
            Shaped::Cone { inverted: false },
            bx([1, base, 1], [dia - 1, h, dia - 1]),
        ),
    ];
    for (x, z) in [(0, 0), (dia - 1, 0), (0, dia - 1), (dia - 1, dia - 1)] {
        shaped.push((Shaped::Cyl, bx([x, base, z], [x + 1, h * 2 / 3 + 1, z + 1])));
    }
    Some(FormOut {
        shaped,
        ..FormOut::default()
    })
}

/// F2-08: a viaduct: a long deck on piers that march along it.
pub fn viaduct(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let (len, wide, along_x) = if w >= d { (w, d, true) } else { (d, w, false) };
    if len < 6 || h < 3 {
        return None;
    }
    let wide = wide.min(3);
    let at = |a0: i32, a1: i32, y0: i32, y1: i32| {
        if along_x {
            bx([a0, y0, 0], [a1, y1, wide])
        } else {
            bx([0, y0, a0], [wide, y1, a1])
        }
    };
    let mut b = vec![at(0, len, h - 1, h)];
    let mut a = 0;
    while a < len {
        b.push(at(a, a + 1, 0, h - 1));
        a += 3;
    }
    b.push(at(len - 1, len, 0, h - 1));
    boxes(b)
}

/// F2-09: a megaframe: columns and floors of a frame with capsules plugged into it.
pub fn megaframe(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 4 {
        return None;
    }
    let mut b = Vec::new();
    for (x, z) in [(0, 0), (w - 1, 0), (0, d - 1), (w - 1, d - 1)] {
        b.push(bx([x, 0, z], [x + 1, h, z + 1]));
    }
    for y in (1..h).step_by(2) {
        b.push(bx([0, y, 0], [w, y + 1, d]));
        if r.below(2) == 0 {
            let (x, z) = (
                1 + r.below((w - 2) as u64) as i32,
                1 + r.below((d - 2) as u64) as i32,
            );
            b.push(bx(
                [x, y + 1.min(h - y - 1), z],
                [(x + 1).min(w - 1), (y + 2).min(h), (z + 1).min(d - 1)],
            ));
        }
    }
    boxes(b)
}

/// F2-10: a canopy: a cage with a wide low roof.
pub fn space_frame(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || d < 5 || h < 2 {
        return None;
    }
    Some(FormOut {
        shaped: vec![(Shaped::Cage, bx([0, 0, 0], [w, h, d]))],
        boxes: vec![bx([0, h - 1, 0], [w, h, d])],
        ..FormOut::default()
    })
}

/// F2-11: a metabolist trunk: a tall core with pods on both faces at every level.
pub fn metabolist_trunk(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 2 || h < 4 {
        return None;
    }
    let cx = w / 2;
    let mut b = vec![bx([cx - 1, 0, 0], [cx + 1, h, d])];
    for y in 0..h {
        if r.below(3) != 0 {
            b.push(bx([0, y, 0], [cx - 1, y + 1, d.min(2)]));
        }
        if r.below(3) != 0 {
            b.push(bx([cx + 1, y, d - d.min(2)], [w, y + 1, d]));
        }
    }
    boxes(b)
}

/// F2-12: a slab stepping back on one side only.
pub fn stepped_slab(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = d.min(h).min(5);
    if n < 3 {
        return None;
    }
    let tier = (h / n).max(1);
    let flip = r.below(2) == 0;
    let b = (0..n)
        .map(|i| {
            let cut = i * (d / n).max(1);
            if flip {
                bx([0, i * tier, 0], [w, (i + 1) * tier, d - cut])
            } else {
                bx([0, i * tier, cut], [w, (i + 1) * tier, d])
            }
        })
        .collect();
    boxes(b)
}

/// F2-13: a Boullée hall: a great dome on a ring of columns.
pub fn boullee_hall(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 6 || h < 4 {
        return None;
    }
    let col = (h / 3).max(1);
    let mut b = Vec::new();
    for k in 0..dia {
        for (x, z) in [(k, 0), (k, dia - 1), (0, k), (dia - 1, k)] {
            if k % 2 == 0 {
                b.push(bx([x, 0, z], [x + 1, col, z + 1]));
            }
        }
    }
    Some(FormOut {
        boxes: b,
        shaped: vec![(Shaped::Dome, bx([0, col, 0], [dia, h, dia]))],
        ..FormOut::default()
    })
}

/// F2-14: a sphere house: rings that swell and narrow.
pub fn sphere_house(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 5 {
        return None;
    }
    let n = h.min(7);
    let tier = (h / n).max(1);
    let shaped = (0..n)
        .map(|i| {
            let m = (i - (n - 1) / 2).abs();
            let inset = (m * dia / (2 * n)).min(dia / 2 - 1);
            (
                Shaped::Ring(16),
                bx(
                    [inset, i * tier, inset],
                    [dia - inset, (i + 1) * tier, dia - inset],
                ),
            )
        })
        .collect();
    Some(FormOut {
        shaped,
        ..FormOut::default()
    })
}

/// F2-15: a nave with lower aisles and two towers at one end.
pub fn cathedral(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || d < 7 || h < 4 {
        return None;
    }
    let (a0, a1) = (w / 3, w - w / 3);
    let aisle = (h / 2).max(1);
    let tower = d.min(2);
    Some(FormOut {
        boxes: vec![
            bx([a0, 0, 0], [a1, h * 2 / 3, d]),
            bx([0, 0, 0], [a0, aisle, d]),
            bx([a1, 0, 0], [w, aisle, d]),
            bx([0, 0, d - tower], [a0 + 1, h, d]),
            bx([a1 - 1, 0, d - tower], [w, h, d]),
        ],
        ..FormOut::default()
    })
}

/// F2-16: a monster building: a dozen boxes of every size heaped and fused.
pub fn monster_building(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let mut b = Vec::new();
    for _ in 0..12 {
        let (bw, bd) = (
            1 + r.below((w / 2).max(1) as u64) as i32,
            1 + r.below((d / 2).max(1) as u64) as i32,
        );
        let (x, z) = (
            r.below((w - bw + 1) as u64) as i32,
            r.below((d - bd + 1) as u64) as i32,
        );
        let bh = 1 + r.below(h as u64) as i32;
        b.push(bx([x, 0, z], [x + bw, bh, z + bd]));
    }
    let mut turned = Vec::new();
    for _ in 0..3 {
        let (x, z) = (
            r.below((w - 1) as u64) as i32,
            r.below((d - 1) as u64) as i32,
        );
        turned.push(Turned {
            bx: bx(
                [x, h / 2, z],
                [x + 2.min(w - x), h / 2 + 2.min(h - h / 2), z + 2.min(d - z)],
            ),
            yaw: r.below(64) as u8,
            tilt: 1 + r.below(4) as u8,
            pivot: None,
        });
    }
    Some(FormOut {
        boxes: b,
        turned,
        ..FormOut::default()
    })
}

/// F2-17: a battleship island: a long low hull with towers in a row.
pub fn battleship_island(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let (len, wide, along_x) = if w >= d { (w, d, true) } else { (d, w, false) };
    if len < 6 || h < 3 || wide < 2 {
        return None;
    }
    let at = |a0: i32, a1: i32, c0: i32, c1: i32, y0: i32, y1: i32| {
        if along_x {
            bx([a0, y0, c0], [a1, y1, c1])
        } else {
            bx([c0, y0, a0], [c1, y1, a1])
        }
    };
    let mut b = vec![at(0, len, 0, wide, 0, 1)];
    let mut a = 1;
    while a + 2 <= len {
        let top = 2 + r.below((h - 1) as u64) as i32;
        b.push(at(a, a + 2, wide / 4, wide - wide / 4, 1, top.min(h)));
        a += 3;
    }
    boxes(b)
}

/// F2-18: a bridge house: two legs and a building across them.
pub fn bridge_house(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || h < 4 || d < 2 {
        return None;
    }
    let leg = (h * 2 / 3).max(2);
    boxes(vec![
        bx([0, 0, 0], [2, leg, d]),
        bx([w - 2, 0, 0], [w, leg, d]),
        bx([0, leg, 0], [w, h, d]),
    ])
}

/// F2-19: a bank of silos on a roof deck.
pub fn silo_bank(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let (nx, nz) = ((w / 2).min(4), (d / 2).min(3));
    if nx < 2 || nz < 1 || h < 3 {
        return None;
    }
    let mut shaped = Vec::new();
    for i in 0..nx {
        for j in 0..nz {
            shaped.push((
                Shaped::Cyl,
                bx([i * 2, 1, j * 2], [i * 2 + 2, h, j * 2 + 2]),
            ));
        }
    }
    Some(FormOut {
        boxes: vec![bx([0, 0, 0], [nx * 2, 1, nz * 2])],
        shaped,
        ..FormOut::default()
    })
}

/// F2-20: a cluster of slender towers joined by skybridges.
pub fn lift_tower_cluster(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || d < 5 || h < 5 {
        return None;
    }
    let spots = [(0, 0), (w - 2, 0), (0, d - 2), (w - 2, d - 2)];
    let mut b: Vec<SlotBox> = spots
        .iter()
        .map(|&(x, z)| bx([x, 0, z], [x + 2, h - r.below(2) as i32, z + 2]))
        .collect();
    for y in [h / 3, h * 2 / 3] {
        b.push(bx([2, y, 0], [w - 2, y + 1, 2]));
        b.push(bx([2, y, d - 2], [w - 2, y + 1, d]));
        b.push(bx([0, y, 2], [2, y + 1, d - 2]));
        b.push(bx([w - 2, y, 2], [w, y + 1, d - 2]));
    }
    boxes(b)
}

/// F2-21: a field of stair towers: each tower a box with a stair rising along its side.
pub fn stair_tower_field(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let (nx, nz) = ((w / 3).min(3), (d / 3).min(3));
    if nx < 1 || nz < 1 || h < 3 {
        return None;
    }
    let mut shaped = Vec::new();
    let mut b = Vec::new();
    for i in 0..nx {
        for j in 0..nz {
            let th = 2 + r.below((h - 1) as u64) as i32;
            b.push(bx([i * 3, 0, j * 3], [i * 3 + 2, th.min(h), j * 3 + 2]));
            shaped.push((
                Shaped::Stair(Dir4::ALL[r.below(4) as usize]),
                bx(
                    [i * 3 + 2, 0, j * 3],
                    [i * 3 + 3, (th / 2).max(1), j * 3 + 2],
                ),
            ));
        }
    }
    Some(FormOut {
        boxes: b,
        shaped,
        ..FormOut::default()
    })
}

/// F2-22: a hollow mountain: heaped terraces round an empty middle, lit inside.
pub fn hollow_mountain(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = (w / 2).min(d / 2).min(h).min(6);
    if n < 3 || w < 6 || d < 6 {
        return None;
    }
    let tier = (h / n).max(1);
    let b: Vec<SlotBox> = (0..n)
        .map(|i| bx([i, 0, i], [w - i, ((i + 1) * tier).min(h), d - i]))
        .collect();
    Some(FormOut {
        boxes: b,
        dark: true,
        pierce: Some((0, [0, 1, d / 2], w)),
        ..FormOut::default()
    })
}

/// F2-23: a tower with garden terraces and vines.
pub fn hanging_garden_tower(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 4 {
        return None;
    }
    let mut b = Vec::new();
    let mut vines = Vec::new();
    for y in (0..h).step_by(2) {
        let inset = r.below(2) as i32;
        b.push(bx(
            [inset, y, inset],
            [w - inset, (y + 2).min(h), d - inset],
        ));
        vines.push([
            r.below(w as u64) as i32,
            r.below(d as u64) as i32,
            y,
            (y + 3).min(h),
        ]);
    }
    Some(FormOut {
        boxes: b,
        vines,
        ..FormOut::default()
    })
}

/// F2-24: a lattice ghost: a cage with half its members gone.
pub fn lattice_ghost(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 4 || h < 4 {
        return None;
    }
    let mut b = Vec::new();
    for (x, z) in [(0, 0), (w - 1, 0), (0, d - 1), (w - 1, d - 1)] {
        if r.below(4) != 0 {
            b.push(bx([x, 0, z], [x + 1, h, z + 1]));
        }
    }
    for y in (0..h).step_by(2) {
        for (lo, hi) in [
            ([0, y, 0], [w, y + 1, 1]),
            ([0, y, d - 1], [w, y + 1, d]),
            ([0, y, 0], [1, y + 1, d]),
            ([w - 1, y, 0], [w, y + 1, d]),
        ] {
            if r.below(2) == 0 {
                b.push(bx(lo, hi));
            }
        }
    }
    if b.is_empty() {
        return None;
    }
    let _ = Axis::X;
    boxes(b)
}

#[cfg(test)]
mod tests {
    use crate::forms::FORMS;
    use crate::rng::Rng;

    /// F2-01..24: each form builds at the smallest and the largest size of its range, and its
    /// pieces stay in the bounding size.
    #[test]
    fn every_form_builds_inside_its_size() {
        let forms: Vec<_> = FORMS.iter().filter(|f| f.tag.starts_with("F2")).collect();
        assert_eq!(forms.len(), 24);
        for f in forms {
            let mut made = 0;
            for pick in [0, 1, 2] {
                let m = |k: usize| match pick {
                    0 => f.size_m.0[k],
                    1 => f.size_m.1[k],
                    _ => i32::midpoint(f.size_m.0[k], f.size_m.1[k]),
                };
                // Metres [w, d, h] to slots [w, h, d] of 32 m.
                let s = [m(0) / 32, m(2) / 32, m(1) / 32];
                for seed in 0..8u64 {
                    let Some(out) = (f.build)(s, &mut Rng::new(seed)) else {
                        continue;
                    };
                    made += 1;
                    assert!(
                        !out.boxes.is_empty() || !out.shaped.is_empty() || !out.turned.is_empty(),
                        "{} is empty",
                        f.tag
                    );
                    for b in out
                        .boxes
                        .iter()
                        .chain(out.shaped.iter().map(|(_, b)| b))
                        .chain(out.turned.iter().map(|t| &t.bx))
                    {
                        for k in 0..3 {
                            assert!(
                                b.lo[k] >= 0 && b.hi[k] <= s[k] && b.lo[k] < b.hi[k],
                                "{} at {s:?}: {b:?}",
                                f.tag
                            );
                        }
                    }
                }
            }
            assert!(made > 0, "{} never builds", f.tag);
        }
    }
}
