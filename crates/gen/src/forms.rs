//! The structure forms of the Web tissue (FORM-01..23). A form turns a size in slots and a
//! stream of randomness into pieces in corner-origin slot coordinates; the support places
//! them. Spec: `docs/spec/levels.md` §5W.4.1, §5W.14.
//!
//! Families (`regimes.md` §9): 0 TOWER, 1 STEP, 2 FRAMEWORK, 3 PLANT, 4 ROUND, 5 RUIN,
//! 6 ANGLED. Sizes are `[width, height, depth]` in slots (`x`, `y`, `z`); a form that cannot
//! be made at the size it is given returns `None` and the slab tower is used instead.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::unnecessary_wraps
)]

use crate::cell::Axis;
use crate::forms2::{
    arcology, battleship_island, blast_furnace, boullee_hall, bridge_house, capsule_tower,
    cathedral, cooling_tower, dish, gasometer, habitat_stack, hanging_garden_tower,
    hollow_mountain, lattice_ghost, lift_tower_cluster, megaframe, metabolist_trunk,
    monster_building, silo_bank, space_frame, sphere_house, stair_tower_field, stepped_slab,
    viaduct,
};
use crate::prim::Dir4;
use crate::rng::Rng;
use crate::tissue_grow::{Shaped, SlotBox};

pub const FAMILIES: usize = 7;

/// A box in a turned frame: turned about the centre of its base by `Frame::table(yaw, tilt)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turned {
    pub bx: SlotBox,
    pub yaw: u8,
    pub tilt: u8,
    /// The pivot `(x, z)` in slots when it is not the centre of the base (a group turned
    /// together by a mutation).
    pub pivot: Option<[f64; 2]>,
}

/// What a form makes, in slots from the corner of its bounding size.
#[derive(Debug, Clone, Default)]
pub struct FormOut {
    pub boxes: Vec<SlotBox>,
    pub shaped: Vec<(Shaped, SlotBox)>,
    pub turned: Vec<Turned>,
    /// Vines hung on the sides: `(x, z, y0, y1)` in slots (MUT-05).
    pub vines: Vec<[i32; 4]>,
    /// An older form swallowed inside, drawn in another material (MUT-15).
    pub inner: Vec<SlotBox>,
    /// Without lights (MUT-11), or lit by windows (MUT-12).
    pub dark: bool,
    pub lit: bool,
    /// A passage bored through along the axis from the slot, for the length (MUT-10).
    pub pierce: Option<(usize, [i32; 3], i32)>,
}

impl FormOut {
    /// One cube of `side` slots (the Grid, STG-04).
    pub fn of_cube(side: i32) -> Option<Self> {
        Self::of_boxes(vec![bx([0, 0, 0], [side, side, side])])
    }

    fn of_boxes(boxes: Vec<SlotBox>) -> Option<Self> {
        Some(Self {
            boxes,
            ..Self::default()
        })
    }

    fn of_shaped(shaped: Vec<(Shaped, SlotBox)>) -> Option<Self> {
        Some(Self {
            shaped,
            ..Self::default()
        })
    }

    #[must_use]
    pub fn shifted(&self, d: [i32; 3]) -> Self {
        Self {
            boxes: self.boxes.iter().map(|b| b.shifted(d)).collect(),
            shaped: self
                .shaped
                .iter()
                .map(|(s, b)| (*s, b.shifted(d)))
                .collect(),
            turned: self
                .turned
                .iter()
                .map(|t| Turned {
                    bx: t.bx.shifted(d),
                    pivot: t
                        .pivot
                        .map(|p| [p[0] + f64::from(d[0]), p[1] + f64::from(d[2])]),
                    ..*t
                })
                .collect(),
            vines: self
                .vines
                .iter()
                .map(|v| [v[0] + d[0], v[1] + d[2], v[2] + d[1], v[3] + d[1]])
                .collect(),
            inner: self.inner.iter().map(|b| b.shifted(d)).collect(),
            dark: self.dark,
            lit: self.lit,
            pierce: self
                .pierce
                .map(|(k, p, l)| (k, [p[0] + d[0], p[1] + d[1], p[2] + d[2]], l)),
        }
    }

    /// The boxes only, for placements that arrange boxes (the support variants): the
    /// boxes, the boxes of shaped pieces, and the boxes of turned ones.
    pub fn envelope(&self) -> Vec<SlotBox> {
        let mut v = self.boxes.clone();
        v.extend(self.shaped.iter().map(|(_, b)| *b));
        v.extend(self.turned.iter().map(|t| t.bx));
        v
    }
}

/// A form: its registry tag, family, size range in metres and builder.
pub struct FormDef {
    pub tag: &'static str,
    pub family: u8,
    /// Inclusive size range in metres `(min, max)` as `[width, depth, height]`.
    pub size_m: ([i32; 3], [i32; 3]),
    /// Sealed by definition: no entrance is needed (a blank monument face).
    pub sealed: bool,
    /// The pieces for a bounding size in slots `[width, height, depth]`.
    pub build: fn(size: [i32; 3], rng: &mut Rng) -> Option<FormOut>,
}

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

fn whole(s: [i32; 3]) -> SlotBox {
    bx([0, 0, 0], s)
}

fn slab_tower(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    FormOut::of_boxes(vec![whole(s)])
}

/// FORM-02: tiers, widest at the bottom or, inverted, at the top (every step a terrace).
fn ziggurat(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = (w / 2).min(d / 2).min(h).min(4);
    if n < 2 {
        return None;
    }
    let inverted = r.below(2) == 0;
    let tier = h / n;
    let boxes = (0..n)
        .map(|i| {
            let inset = if inverted { n - 1 - i } else { i };
            let top = if i + 1 == n { h } else { (i + 1) * tier };
            bx([inset, i * tier, inset], [w - inset, top, d - inset])
        })
        .collect();
    FormOut::of_boxes(boxes)
}

/// FORM-03: a needle: one or two slots across.
fn needle(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if h < 4 {
        return None;
    }
    let (a, b) = (1 + r.below(2) as i32, 1 + r.below(2) as i32);
    let (a, b) = (a.min(w), b.min(d));
    let (x, z) = ((w - a) / 2, (d - b) / 2);
    FormOut::of_boxes(vec![bx([x, 0, z], [x + a, h, z + b])])
}

/// FORM-04: a tube tower: four walls round an open atrium core.
fn tube_tower(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || d < 3 || h < 3 {
        return None;
    }
    FormOut::of_boxes(vec![
        bx([0, 0, 0], [w, h, 1]),
        bx([0, 0, d - 1], [w, h, d]),
        bx([0, 0, 1], [1, h, d - 1]),
        bx([w - 1, 0, 1], [w, h, d - 1]),
    ])
}

/// FORM-05: two towers joined by skybridges at several heights.
fn h_pair(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 5 || h < 4 {
        return None;
    }
    let a = ((w - 2) / 3).max(1);
    let mut boxes = vec![bx([0, 0, 0], [a, h, d]), bx([w - a, 0, 0], [w, h, d])];
    let bridges = 2 + r.below(2) as i32;
    for i in 0..bridges {
        let y = (1 + i * (h - 2) / bridges).min(h - 1);
        let z = d / 2;
        boxes.push(bx([a, y, z], [w - a, y + 1, z + 1]));
    }
    FormOut::of_boxes(boxes)
}

/// FORM-06: a comb: a base slab with teeth.
fn comb(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || h < 3 {
        return None;
    }
    let mut boxes = vec![bx([0, 0, 0], [w, 1, d])];
    let mut x = 0;
    while x < w {
        boxes.push(bx([x, 1, 0], [x + 1, h, d]));
        x += 2;
    }
    FormOut::of_boxes(boxes)
}

/// FORM-07: a cage: a frame with floors and no walls.
fn cage(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    if s[0] < 2 || s[1] < 2 || s[2] < 2 {
        return None;
    }
    FormOut::of_shaped(vec![(Shaped::Cage, whole(s))])
}

/// FORM-08: a cluster of 5 to 30 fused boxes.
fn cluster(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w * h * d < 12 {
        return None;
    }
    let count = (5 + r.below(26) as i32).min(w * h * d / 2).max(5);
    let first = bx(
        [w / 2, 0, d / 2],
        [w / 2 + 1.max(w / 3), 1.max(h / 3), d / 2 + 1.max(d / 3)],
    );
    let mut boxes = vec![first];
    for _ in 0..count * 3 {
        if boxes.len() as i32 >= count {
            break;
        }
        let base = boxes[r.below(boxes.len() as u64) as usize];
        let sz = [
            1 + r.below(((w / 3).max(1)) as u64) as i32,
            1 + r.below(((h / 3).max(1)) as u64) as i32,
            1 + r.below(((d / 3).max(1)) as u64) as i32,
        ];
        let k = r.below(3) as usize;
        let mut lo = base.lo;
        lo[k] = if r.below(2) == 0 {
            base.hi[k]
        } else {
            base.lo[k] - sz[k]
        };
        for j in 0..3 {
            if j != k {
                lo[j] = base.lo[j] + r.range_i32(-(sz[j] - 1), base.hi[j] - base.lo[j]);
            }
        }
        let b = bx(lo, [lo[0] + sz[0], lo[1] + sz[1], lo[2] + sz[2]]);
        if (0..3).all(|j| b.lo[j] >= 0 && b.hi[j] <= s[j]) {
            boxes.push(b);
        }
    }
    (boxes.len() >= 5).then_some(FormOut {
        boxes,
        ..FormOut::default()
    })
}

/// FORM-09: a drum or silo: upright, or lying down along x or z.
fn drum(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 2 {
        return None;
    }
    if r.below(4) == 0 && w.max(d) >= 3 {
        let along_x = w >= d;
        let (len, other) = if along_x { (w, d) } else { (d, w) };
        let dia = other.min(h).max(2);
        let b = if along_x {
            bx([0, 0, 0], [len, dia, dia])
        } else {
            bx([0, 0, 0], [dia, dia, len])
        };
        let axis = if along_x { Axis::X } else { Axis::Z };
        return FormOut::of_shaped(vec![(Shaped::CylH(axis), b)]);
    }
    FormOut::of_shaped(vec![(Shaped::Cyl, bx([0, 0, 0], [dia, h, dia]))])
}

/// FORM-10: a terrace cliff: terraces climbing a slope.
fn terrace_cliff(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let n = w.min(h).min(5);
    if n < 3 {
        return None;
    }
    let tier = h / n;
    let boxes = (0..n)
        .map(|i| bx([i * (w / n).max(1), 0, 0], [w, (i + 1) * tier, d]))
        .collect();
    FormOut::of_boxes(boxes)
}

/// FORM-11: a gantry: two legs and a beam, with the way through open.
fn gantry(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 3 || h < 3 {
        return None;
    }
    FormOut::of_boxes(vec![
        bx([0, 0, 0], [1, h, d]),
        bx([w - 1, 0, 0], [w, h, d]),
        bx([0, h - 1, 0], [w, h, d]),
    ])
}

/// FORM-12: a machine: turbine, gear stack, piston bank or radiator fins.
fn machine(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    match r.below(4) {
        0 if w >= 3 => {
            let dia = h.min(d).max(2);
            FormOut::of_shaped(vec![(Shaped::CylH(Axis::X), bx([0, 0, 0], [w, dia, dia]))])
        }
        1 if h >= 3 && w.min(d) >= 2 => {
            let n = (h / 2).clamp(2, 4);
            let tier = h / n;
            let shaped = (0..n)
                .map(|i| {
                    let dia = (w.min(d) - i).max(2);
                    (
                        Shaped::Cyl,
                        bx([0, i * tier, 0], [dia, (i + 1) * tier, dia]),
                    )
                })
                .collect();
            FormOut::of_shaped(shaped)
        }
        2 if w >= 3 && h >= 2 => {
            let mut shaped = vec![];
            for x in 0..w {
                shaped.push((Shaped::Cyl, bx([x, 1, 0], [x + 1, h, 1.max(d.min(2))])));
            }
            let mut out = FormOut::of_shaped(shaped)?;
            out.boxes.push(bx([0, 0, 0], [w, 1, d.min(2)]));
            Some(out)
        }
        _ if w >= 3 && h >= 2 => {
            let mut boxes = vec![bx([0, 0, 0], [w, 1, d])];
            let mut x = 0;
            while x < w {
                boxes.push(bx([x, 1, 0], [x + 1, h, d]));
                x += 2;
            }
            FormOut::of_boxes(boxes)
        }
        _ => None,
    }
}

/// FORM-13: a rack canyon: two long walls with a canyon between.
fn rack_canyon(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if d < 3 || w < 3 {
        return None;
    }
    let wall = (d / 3).max(1);
    FormOut::of_boxes(vec![
        bx([0, 0, 0], [w, h, wall]),
        bx([0, 0, d - wall], [w, h, d]),
    ])
}

/// FORM-14: a tank farm: a grid of tanks on a base slab.
fn tank_farm(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 2 || d < 2 || h < 2 {
        return None;
    }
    let mut out = FormOut::of_boxes(vec![bx([0, 0, 0], [w, 1, d])])?;
    for x in (0..w - 1).step_by(2) {
        for z in (0..d - 1).step_by(2) {
            out.shaped
                .push((Shaped::Cyl, bx([x, 1, z], [x + 2, h, z + 2])));
        }
    }
    Some(out)
}

/// FORM-15: a ruin: ragged columns of different height.
fn ruin(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w * d < 4 || h < 2 {
        return None;
    }
    let step = if w * d > 64 { 2 } else { 1 };
    let mut boxes = Vec::new();
    for x in (0..w).step_by(step as usize) {
        for z in (0..d).step_by(step as usize) {
            if r.below(100) < 60 {
                let top = 1 + r.below(h as u64) as i32;
                boxes.push(bx([x, 0, z], [(x + step).min(w), top, (z + step).min(d)]));
            }
        }
    }
    (boxes.len() >= 3).then_some(FormOut {
        boxes,
        ..FormOut::default()
    })
}

/// FORM-16: a rotunda with a radial plan: a ring wall round a central drum.
fn rotunda(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 {
        return None;
    }
    let c = (dia - dia / 3).max(1);
    let off = (dia - c) / 2;
    FormOut::of_shaped(vec![
        (Shaped::Ring(16), bx([0, 0, 0], [dia, h, dia])),
        (
            Shaped::Cyl,
            bx([off, 0, off], [off + c / 2 + 1, h, off + c / 2 + 1]),
        ),
    ])
}

/// FORM-17: a twisted tower: each floor turned a little from the one below.
fn twisted(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let side = w.min(d);
    if side < 2 || h < 4 {
        return None;
    }
    let step = 2 + r.below(3) as u8;
    let turned = (0..h)
        .map(|i| Turned {
            bx: bx([0, i, 0], [side, i + 1, side]),
            yaw: (i as u8).wrapping_mul(step) % 64,
            tilt: 0,
            pivot: None,
        })
        .collect();
    Some(FormOut {
        turned,
        ..FormOut::default()
    })
}

/// FORM-18: a leaning tower.
fn leaning(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if h < 4 {
        return None;
    }
    let side = w.min(d).clamp(1, 3);
    let yaw = [0u8, 16, 32, 48][r.below(4) as usize];
    let tilt = 1 + r.below(3) as u8;
    Some(FormOut {
        turned: vec![Turned {
            bx: bx([0, 0, 0], [side, h, side]),
            yaw,
            tilt,
            pivot: None,
        }],
        ..FormOut::default()
    })
}

/// FORM-19: a faceted dome.
fn dome(s: [i32; 3], _: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 || h < 2 {
        return None;
    }
    FormOut::of_shaped(vec![(
        Shaped::Dome,
        bx([0, 0, 0], [dia, h.min(dia / 2 + 1), dia]),
    )])
}

/// FORM-20: a polygonal ring.
fn polygonal_ring(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 4 {
        return None;
    }
    let n = if r.below(2) == 0 { 8 } else { 16 };
    FormOut::of_shaped(vec![(Shaped::Ring(n), bx([0, 0, 0], [dia, h, dia]))])
}

/// FORM-21: a starburst hub: a core with eight arms.
fn starburst(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 5 {
        return None;
    }
    let c = dia / 2;
    let arm = (dia / 2 - 1).max(1);
    let hh = 1 + r.below(h.clamp(1, 2) as u64) as i32;
    let core = bx([c - 1, 0, c - 1], [c + 1, h, c + 1]);
    let turned = (0..8)
        .map(|k| Turned {
            bx: bx([c, 0, c - 1], [c + arm, hh, c]),
            yaw: k * 8,
            tilt: 0,
            pivot: None,
        })
        .collect();
    Some(FormOut {
        boxes: vec![core],
        turned,
        ..FormOut::default()
    })
}

/// FORM-22: a wedge building: a prism whose roof slopes.
fn wedge(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    if w < 2 || h < 2 || d < 2 {
        return None;
    }
    let rise = Dir4::ALL[r.below(4) as usize];
    FormOut::of_shaped(vec![(Shaped::Wedge(rise), whole([w, h, d]))])
}

/// FORM-23: a cone or funnel tower.
fn cone(s: [i32; 3], r: &mut Rng) -> Option<FormOut> {
    let [w, h, d] = s;
    let dia = w.min(d);
    if dia < 2 || h < 3 {
        return None;
    }
    let inverted = r.below(2) == 0;
    FormOut::of_shaped(vec![(
        Shaped::Cone { inverted },
        bx([0, 0, 0], [dia, h, dia]),
    )])
}

macro_rules! form {
    ($tag:literal, $fam:literal, $lo:expr, $hi:expr, $f:ident) => {
        FormDef {
            tag: $tag,
            family: $fam,
            size_m: ($lo, $hi),
            sealed: false,
            build: $f,
        }
    };
}

/// All forms, in registry order.
pub const FORMS: &[FormDef] = &[
    form!("FORM-01", 0, [192, 128, 160], [512, 448, 512], slab_tower),
    form!("FORM-02", 1, [160, 160, 128], [448, 448, 384], ziggurat),
    form!("FORM-03", 0, [32, 32, 192], [96, 96, 512], needle),
    form!("FORM-04", 0, [128, 128, 160], [320, 320, 512], tube_tower),
    form!("FORM-05", 0, [224, 96, 160], [480, 288, 448], h_pair),
    form!("FORM-06", 0, [160, 96, 96], [448, 320, 320], comb),
    form!("FORM-07", 2, [128, 128, 128], [320, 320, 448], cage),
    form!("FORM-08", 0, [160, 160, 128], [416, 416, 384], cluster),
    form!("FORM-09", 4, [96, 96, 160], [288, 288, 512], drum),
    form!(
        "FORM-10",
        1,
        [192, 128, 128],
        [480, 320, 384],
        terrace_cliff
    ),
    form!("FORM-11", 2, [192, 64, 160], [448, 192, 384], gantry),
    form!("FORM-12", 3, [96, 96, 96], [320, 320, 320], machine),
    form!("FORM-13", 2, [224, 96, 160], [512, 320, 384], rack_canyon),
    form!("FORM-14", 3, [160, 160, 64], [448, 448, 224], tank_farm),
    form!("FORM-15", 5, [160, 160, 96], [416, 416, 320], ruin),
    form!("FORM-16", 4, [192, 192, 96], [416, 416, 256], rotunda),
    form!("FORM-17", 6, [128, 128, 192], [288, 288, 512], twisted),
    form!("FORM-18", 6, [96, 96, 192], [256, 256, 512], leaning),
    form!("FORM-19", 4, [192, 192, 96], [448, 448, 224], dome),
    form!(
        "FORM-20",
        4,
        [192, 192, 64],
        [448, 448, 224],
        polygonal_ring
    ),
    form!("FORM-21", 6, [192, 192, 96], [480, 480, 224], starburst),
    form!("FORM-22", 6, [160, 128, 96], [416, 320, 320], wedge),
    form!("FORM-23", 6, [128, 128, 192], [320, 320, 512], cone),
    form!("F2-01", 0, [96, 96, 160], [224, 224, 480], capsule_tower),
    form!("F2-02", 0, [128, 128, 128], [352, 352, 352], habitat_stack),
    form!("F2-03", 1, [224, 224, 128], [480, 480, 384], arcology),
    form!("F2-04", 4, [160, 160, 128], [320, 320, 448], cooling_tower),
    form!("F2-05", 3, [128, 128, 96], [320, 320, 224], dish),
    form!("F2-06", 3, [128, 128, 96], [288, 288, 224], gasometer),
    form!("F2-07", 3, [128, 128, 128], [320, 320, 384], blast_furnace),
    form!("F2-08", 2, [224, 64, 96], [512, 192, 224], viaduct),
    form!("F2-09", 2, [160, 160, 128], [416, 416, 384], megaframe),
    form!("F2-10", 2, [192, 192, 64], [480, 480, 128], space_frame),
    form!("F2-11", 0, [96, 96, 160], [256, 256, 480], metabolist_trunk),
    form!("F2-12", 1, [192, 96, 128], [480, 320, 384], stepped_slab),
    form!("F2-13", 4, [192, 192, 96], [448, 448, 256], boullee_hall),
    form!("F2-14", 4, [128, 128, 160], [288, 288, 288], sphere_house),
    form!("F2-15", 0, [160, 224, 96], [352, 512, 256], cathedral),
    form!(
        "F2-16",
        6,
        [160, 160, 128],
        [416, 416, 384],
        monster_building
    ),
    form!(
        "F2-17",
        3,
        [224, 96, 64],
        [512, 224, 160],
        battleship_island
    ),
    form!("F2-18", 2, [192, 160, 96], [448, 384, 224], bridge_house),
    form!("F2-19", 3, [128, 128, 64], [320, 320, 192], silo_bank),
    form!(
        "F2-20",
        0,
        [160, 160, 128],
        [352, 352, 416],
        lift_tower_cluster
    ),
    form!(
        "F2-21",
        6,
        [160, 160, 96],
        [352, 352, 224],
        stair_tower_field
    ),
    form!("F2-22", 1, [192, 192, 96], [448, 448, 224], hollow_mountain),
    form!(
        "F2-23",
        0,
        [96, 96, 160],
        [224, 224, 416],
        hanging_garden_tower
    ),
    form!("F2-24", 5, [128, 128, 128], [320, 320, 352], lattice_ghost),
];

/// The indices into `FORMS` of family `family`.
pub fn members(family: usize) -> Vec<usize> {
    FORMS
        .iter()
        .enumerate()
        .filter(|(_, f)| usize::from(f.family) == family)
        .map(|(i, _)| i)
        .collect()
}
