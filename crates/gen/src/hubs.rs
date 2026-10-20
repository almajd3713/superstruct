//! Hubs HUB-01..11 (`levels.md` §5W.6, §5W.21): the big structures at the places routes meet.
//! A hub is a form like those of `forms.rs`, placed first in step 3 and whether or not the
//! cell's fill is already met. Spec: the knot's structure in the cell that holds the knot,
//! and one more in a cell that has a tram terminus, a valley rim or an edge on a Between,
//! with chance 1/4 (the Spine station and Road crossing hubs wait for PW.10).

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

/// `Structure::form` of hub `n` (0-based) is `HUB_BASE + n`.
pub const HUB_BASE: usize = 100;

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

/// HUB-01: a junction hall: a wide low hall with a tower and a beacon spire in the middle.
fn junction_hall(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let low = (h / 3).max(1);
    let (x0, x1, z0, z1) = (w / 3, w - w / 3, d / 3, d - d / 3);
    out(
        vec![
            bx([0, 0, 0], [w, low, d]),
            bx([x0, low, z0], [x1.max(x0 + 1), h - 1, z1.max(z0 + 1)]),
        ],
        vec![(
            Shaped::Cone { inverted: false },
            bx([x0, h - 1, z0], [x1.max(x0 + 1), h, z1.max(z0 + 1)]),
        )],
        true,
    )
}

/// HUB-02: a hypostyle forest: a floor, a roof and a grid of columns.
fn hypostyle(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [w, 1, d]), bx([0, h - 1, 0], [w, h, d])];
    let mut x = 0;
    while x < w {
        let mut z = 0;
        while z < d {
            boxes.push(bx([x, 1, z], [x + 1, h - 1, z + 1]));
            z += 2;
        }
        x += 2;
    }
    out(boxes, Vec::new(), false)
}

/// HUB-03: a machine cathedral: a long horizontal drum between two lower aisles, with fins.
fn machine_cathedral(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 3 || h < 3 {
        return None;
    }
    let (z0, z1) = (d / 3, d - d / 3);
    let mut boxes = vec![
        bx([0, 0, 0], [w, (h / 2).max(1), z0.max(1)]),
        bx([0, 0, z1.min(d - 1)], [w, (h / 2).max(1), d]),
    ];
    let mut x = 1;
    while x < w {
        boxes.push(bx([x, (h / 2).max(1), 0], [x + 1, (h / 2 + 1).min(h), d]));
        x += 2;
    }
    out(
        boxes,
        vec![(
            Shaped::CylH(Axis::X),
            bx([0, 0, z0.max(1) - 1], [w, h, (z1 + 1).min(d)]),
        )],
        false,
    )
}

/// HUB-04: a stair cathedral: a ring wall round a helical stair, with four corner towers.
fn stair_cathedral(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 3 {
        return None;
    }
    let t = (dia / 4).max(1);
    let mut boxes = Vec::new();
    for (x, z) in [(0, 0), (dia - t, 0), (0, dia - t), (dia - t, dia - t)] {
        boxes.push(bx([x, 0, z], [x + t, h, z + t]));
    }
    out(
        boxes,
        vec![
            (Shaped::Ring(16), bx([0, 0, 0], [dia, h, dia])),
            (
                Shaped::Helix { sense: 1 },
                bx([t, 0, t], [dia - t, h, dia - t]),
            ),
        ],
        false,
    )
}

/// HUB-05: a reservoir hall: a roofed floor with a row of great tanks.
fn reservoir_hall(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = d.min(h).max(2);
    if w < 4 || d < 2 || h < 3 {
        return None;
    }
    let mut shaped = Vec::new();
    let mut x = 0;
    while x + dia <= w {
        shaped.push((Shaped::Cyl, bx([x, 1, 0], [x + dia, h - 1, dia])));
        x += dia;
    }
    out(
        vec![bx([0, 0, 0], [w, 1, d]), bx([0, h - 1, 0], [w, h, d])],
        shaped,
        false,
    )
}

/// HUB-06: a server canyon: two racks of towers with a floor between and bridges across.
fn server_canyon(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if d < 4 || w < 3 || h < 3 {
        return None;
    }
    let t = (d / 4).max(1);
    let mut boxes = vec![
        bx([0, 0, 0], [w, h, t]),
        bx([0, 0, d - t], [w, h, d]),
        bx([0, 0, t], [w, 1, d - t]),
    ];
    let mut x = 1;
    while x < w {
        boxes.push(bx([x, h - 2, t], [x + 1, h - 1, d - t]));
        x += 3;
    }
    out(boxes, Vec::new(), true)
}

/// HUB-07: a valley rim: terraces stepping down to an edge, with a balcony.
fn valley_rim(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let n = 3.min(d);
    let step = (d / n).max(1);
    let mut boxes = Vec::new();
    for i in 0..n {
        boxes.push(bx(
            [0, 0, i * step],
            [w, h - (n - 1 - i) * (h / (n + 1)).max(1), (i + 1) * step],
        ));
    }
    boxes.push(bx([0, 0, 0], [w, 1, d]));
    out(boxes, Vec::new(), true)
}

/// HUB-08: a tram terminus: a long platform under a roof on columns, with an end block.
fn tram_terminus(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 4 || d < 2 || h < 3 {
        return None;
    }
    let mut boxes = vec![
        bx([0, 0, 0], [w, 1, d]),
        bx([0, h - 1, 0], [w, h, d]),
        bx([w - 2, 1, 0], [w, h - 1, d]),
    ];
    let mut x = 0;
    while x < w - 2 {
        for z in [0, d - 1] {
            boxes.push(bx([x, 1, z], [x + 1, h - 1, z + 1]));
        }
        x += 2;
    }
    out(boxes, Vec::new(), true)
}

/// HUB-09: a Between overlook: a column with a platform thrust out at the top and a pod.
fn overlook(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    let (cx, cz) = (w / 3, d / 3);
    out(
        vec![
            bx([0, 0, 0], [cx.max(1) + 1, h - 1, cz.max(1) + 1]),
            bx([0, h - 2, 0], [w, h - 1, d]),
        ],
        vec![(
            Shaped::Cyl,
            bx([w - 2, h - 1, d - 2], [w, h.max(h - 1 + 2).min(h), d]),
        )],
        true,
    )
}

/// HUB-10: a gravity furnace: a dome on a drum, with a chimney.
fn gravity_furnace(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 4 {
        return None;
    }
    let base = (h / 3).max(1);
    let chim = (dia / 4).max(1);
    out(
        Vec::new(),
        vec![
            (Shaped::Cyl, bx([0, 0, 0], [dia, base, dia])),
            (Shaped::Dome, bx([0, base, 0], [dia, h, dia])),
            (
                Shaped::Cyl,
                bx([dia - chim, base, dia - chim], [dia, h, dia]),
            ),
        ],
        false,
    )
}

/// HUB-11: a sanctuary: a quiet platform with a ring wall and a pillar, lit.
fn sanctuary(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 3 || h < 2 {
        return None;
    }
    out(
        vec![
            bx([0, 0, 0], [dia, 1, dia]),
            bx([dia / 2, 1, dia / 2], [dia / 2 + 1, h, dia / 2 + 1]),
        ],
        vec![(Shaped::Ring(8), bx([0, 1, 0], [dia, (h / 2).max(2), dia]))],
        true,
    )
}

macro_rules! hub {
    ($tag:expr, $min:expr, $max:expr, $f:expr) => {
        FormDef {
            tag: $tag,
            family: 7,
            size_m: ($min, $max),
            sealed: false,
            build: $f,
        }
    };
}

/// The hubs, in registry order. Sizes are `[width, depth, height]` in metres.
pub const HUBS: &[FormDef] = &[
    hub!("HUB-01", [224, 160, 192], [480, 352, 480], junction_hall),
    hub!("HUB-02", [192, 192, 96], [448, 448, 192], hypostyle),
    hub!(
        "HUB-03",
        [224, 128, 128],
        [480, 288, 320],
        machine_cathedral
    ),
    hub!("HUB-04", [160, 160, 192], [352, 352, 512], stair_cathedral),
    hub!("HUB-05", [256, 96, 96], [480, 224, 224], reservoir_hall),
    hub!("HUB-06", [256, 128, 160], [512, 256, 416], server_canyon),
    hub!("HUB-07", [192, 192, 128], [448, 384, 320], valley_rim),
    hub!("HUB-08", [288, 96, 96], [512, 192, 192], tram_terminus),
    hub!("HUB-09", [160, 160, 192], [352, 352, 416], overlook),
    hub!("HUB-10", [160, 160, 160], [320, 320, 352], gravity_furnace),
    hub!("HUB-11", [128, 128, 64], [256, 256, 160], sanctuary),
];

/// The hub kinds (0-based indices into `HUBS`) a knot of hub family `family` may take, with
/// weights.
pub fn knot_kinds(family: u8) -> &'static [(usize, u32)] {
    match family {
        0 => &[(0, 2), (1, 1)],
        1 => &[(3, 1)],
        2 => &[(2, 2), (4, 1), (5, 1)],
        3 => &[(9, 1)],
        _ => &[(10, 1)],
    }
}

/// The contextual hub a cell can take: a tram terminus at a tram port, a valley rim beside a
/// void, or an overlook at the edge of a crust over a Between. The first that applies.
pub fn contextual(t: &Tissue) -> Option<usize> {
    let tram = t.anchors.iter().any(|a| {
        matches!(
            a.kind,
            AnchorKind::Port {
                kind: ArteryType::TramTube,
                ..
            }
        )
    });
    if tram {
        return Some(7);
    }
    let lat = &t.lattice;
    let valley = (0..lat.len()).any(|i| {
        lat.present[i]
            && lat.state[i] == SlotState::Free
            && lat.neighbours(i).any(|j| lat.state[j] == SlotState::Void)
    });
    if valley {
        return Some(6);
    }
    let edge = (0..lat.len()).any(|i| {
        lat.present[i]
            && t.in_crust[i]
            && lat.neighbours(i).any(|j| lat.present[j] && !t.in_crust[j])
    });
    edge.then_some(8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellKey, Level};
    use crate::seed::WorldSeed;
    use crate::tissue::{TissueCache, tissue_cell};
    use glam::I64Vec3;

    /// Every hub builds at hub-like sizes and keeps every piece inside its bounding size.
    #[test]
    fn every_hub_builds_inside_its_size() {
        for (i, h) in HUBS.iter().enumerate() {
            let mut built = 0;
            for size in [[8, 8, 8], [14, 6, 10], [6, 12, 6], [16, 10, 12]] {
                let mut r = Rng::new(7 + i as u64);
                let Some(o) = (h.build)(size, &mut r) else {
                    continue;
                };
                built += 1;
                for b in o.envelope() {
                    for k in 0..3 {
                        assert!(
                            b.lo[k] >= 0 && b.hi[k] <= size[k] && b.lo[k] < b.hi[k],
                            "{} {size:?}: {b:?}",
                            h.tag
                        );
                    }
                }
            }
            assert!(built >= 3, "{} builds at {built} of 4 sizes", h.tag);
        }
    }

    /// HUB-01..11 all occur: the knot's hub in the knot's L2 cell of 600 L1 cells, and the
    /// contextual ones in 800 cells.
    #[test]
    fn every_hub_occurs() {
        let seed = WorldSeed(0x5eed);
        let cache = TissueCache::default();
        let mut seen = [0usize; 12];
        let (mut knots, mut with_hub, mut with_knot_hub) = (0, 0, 0);
        for k in 0..600i64 {
            let l1 = I64Vec3::new(k % 9 - 4, k / 9 % 3, k / 27 % 20 - 10);
            let Some(sk) = crate::l1_artery::skeleton(seed, l1) else {
                continue;
            };
            let c = crate::l1_artery::coords(usize::from(sk.knot));
            let l2 = l1 * 8 + I64Vec3::new(c[0], c[1], c[2]);
            let t = tissue_cell(seed, &cache, CellKey::new(Level::L2, l2));
            assert_eq!(t.knot_hub, Some(sk.knot_hub));
            knots += 1;
            with_hub += usize::from(!t.hubs.is_empty());
            let kinds = knot_kinds(sk.knot_hub);
            with_knot_hub += usize::from(
                t.hubs
                    .iter()
                    .any(|&(id, _)| kinds.iter().any(|k| k.0 + 1 == usize::from(id))),
            );
            for &(id, _) in &t.hubs {
                seen[usize::from(id)] += 1;
            }
        }
        for k in 0..800i64 {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 % 9 - 4);
            let key = CellKey::new(
                Level::L2,
                l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
            );
            let t = tissue_cell(seed, &cache, key);
            for &(id, si) in &t.hubs {
                seen[usize::from(id)] += 1;
                let s = &t.structures[usize::from(si)];
                assert_eq!(usize::from(s.form), HUB_BASE + usize::from(id) - 1);
            }
        }
        eprintln!(
            "knots {knots}, with a hub {with_hub}, with the knot's own hub {with_knot_hub}, per hub {:?}",
            &seen[1..]
        );
        assert!(
            with_knot_hub * 10 >= knots * 9,
            "{with_knot_hub} of {knots} knot cells have their hub"
        );
        for id in 1..=11 {
            assert!(seen[id] > 0, "HUB-{id:02} never occurs: {seen:?}");
        }
    }
}
