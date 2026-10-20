//! Support variants SV-01..26: how each support is varied (`levels.md` §5W.3, §5W.13).
//! A variant places a different arrangement of boxes and shaped pieces on the same
//! candidates; when it cannot be placed in a cell, the plain support is used.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::similar_names,
    clippy::manual_midpoint
)]

use glam::DVec3;

use crate::cell::Dir6;
use crate::forms::FormDef;
use crate::orient::Frame;
use crate::prim::Dir4;
use crate::rng::Rng;
use crate::tissue::{SlotState, Tissue};
use crate::tissue_grow::{
    Candidates, OPart, Overlap, Placed, Shaped, SlotBox, World, add, below, expand, place,
};

#[allow(clippy::wildcard_imports)]
use crate::tissue_grow::{
    COLLAR, EMBEDDED, HANGING, NESTED, PILED, SIDEWAYS, SPANNING, STANDING, SUSPENDED,
};

/// The variants of each support, as SV numbers; 0 is the plain support. Order of the supports:
/// `tissue_grow::SUPPORT_TAGS`.
pub fn variants_of(support: usize) -> &'static [u8] {
    match support {
        STANDING => &[0, 1, 2],
        HANGING => &[0, 3, 4, 5, 6, 7, 8, 26],
        SPANNING => &[0, 9, 10, 11],
        SIDEWAYS => &[0, 12, 13, 14, 25],
        COLLAR => &[0, 15, 16],
        EMBEDDED => &[0, 17, 18],
        SUSPENDED => &[0, 19, 20],
        PILED => &[0, 21, 22],
        NESTED => &[0, 23, 24],
        _ => &[0],
    }
}

struct Ctx<'a> {
    t: &'a Tissue,
    w: &'a World,
    c: &'a Candidates,
    size: [i32; 3],
    form: &'a FormDef,
    words: &'a [u64; 16],
}

impl Ctx<'_> {
    fn n(&self) -> i32 {
        self.t.lattice.n as i32
    }

    fn free(&self, p: [i32; 3]) -> bool {
        self.w.state(p) == Some(SlotState::Free)
    }

    fn box_free(&self, b: &SlotBox) -> bool {
        b.inside(self.t.lattice.n)
            && (b.lo[2]..b.hi[2]).all(|z| {
                (b.lo[1]..b.hi[1]).all(|y| (b.lo[0]..b.hi[0]).all(|x| self.free([x, y, z])))
            })
    }

    /// Whether the slot `p` hangs from a support above it.
    fn hangs(&self, p: [i32; 3]) -> bool {
        self.free(p) && self.w.support(add(p, 1, 1), p, false)
    }

    fn rng(&self) -> Rng {
        Rng::new(self.words[12] ^ 0x5eed_5eed)
    }

    /// A candidate slot from `list`, by the placement word.
    fn pick(&self, list: &[u32]) -> Option<[i32; 3]> {
        Some(self.w.xyz(*list.get(below(self.words[9], list.len()))?))
    }

    /// The form's boxes in a box (its envelope when it is made of shaped pieces).
    fn fill(&self, b: SlotBox) -> Vec<SlotBox> {
        let sz = [0, 1, 2].map(|k| b.hi[k] - b.lo[k]);
        let mut rng = Rng::new(self.words[7]);
        let out = (self.form.build)(sz, &mut rng)
            .map(|o| o.envelope())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![bx([0, 0, 0], sz)]);
        out.iter().map(|f| f.shifted(b.lo)).collect()
    }

    fn yaw_for(&self, tilt: u8, dir: [f64; 3]) -> Option<u8> {
        [0u8, 16, 32, 48].into_iter().find(|&yaw| {
            let ex = Frame::table(yaw, tilt).axes()[0];
            let h = DVec3::new(ex.x, 0.0, ex.z).normalize_or_zero();
            h.dot(DVec3::from_array(dir)) > 0.95
        })
    }
}

fn unit(axis: usize, sign: i32) -> [f64; 3] {
    let mut d = [0.0; 3];
    d[axis] = f64::from(sign);
    d
}

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn place_variant(
    t: &Tissue,
    w: &World,
    support: usize,
    variant: u8,
    c: &Candidates,
    size: [i32; 3],
    form: &FormDef,
    words: &[u64; 16],
) -> Option<Placed> {
    let ctx = Ctx {
        t,
        w,
        c,
        size,
        form,
        words,
    };
    let mut placed = match variant {
        1 => stilts(&ctx)?,
        2 => stepped_footings(&ctx)?,
        3 => kissing(&ctx)?,
        4 => viewing_pod(&ctx)?,
        5 => chandelier(&ctx)?,
        6 => broken_off(&ctx)?,
        7 => on_cables(&ctx)?,
        8 => off_plumb(&ctx)?,
        9 => pillar_building(&ctx)?,
        10 => arch_building(&ctx)?,
        11 => fallen_tower(&ctx)?,
        12 => both_sides(&ctx)?,
        13 => sagging(&ctx)?,
        14 => slanted(&ctx)?,
        15 => half_ring(&ctx)?,
        16 => helix(&ctx)?,
        17 => facade(&ctx)?,
        18 => windows_only(&ctx)?,
        19 => platform_town(&ctx)?,
        20 => cages(&ctx)?,
        21 => omnidirectional(&ctx)?,
        22 => leaning_pile(&ctx)?,
        23 => town_in_a_hall(&ctx)?,
        24 => district_in_a_tether(&ctx)?,
        25 => stepped_long(&ctx)?,
        26 => place(t, w, support, c, size, form, words)?,
        _ => return None,
    };
    placed.variant = variant;
    Some(placed)
}

const UP_DIRS: [(usize, i32); 5] = [(0, -1), (0, 1), (2, -1), (2, 1), (1, 1)];
const DOWN_DIRS: [(usize, i32); 5] = [(0, -1), (0, 1), (2, -1), (2, 1), (1, -1)];

/// SV-01: on stilts over a route.
fn stilts(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.route_top)?;
    let gap = 1 + below(x.words[10] >> 8, 2) as i32;
    let q = add(p, 1, gap);
    if !x.free(q) {
        return None;
    }
    let b = expand(x.w, q, x.size, &UP_DIRS, x.words[11]);
    let mut parts = x.fill(b);
    let corners = [
        [b.lo[0], b.lo[2]],
        [b.hi[0] - 1, b.lo[2]],
        [b.lo[0], b.hi[2] - 1],
        [b.hi[0] - 1, b.hi[2] - 1],
    ];
    let mut seen: Vec<[i32; 2]> = Vec::new();
    for c in corners {
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        let pillar = bx([c[0], p[1], c[1]], [c[0] + 1, q[1], c[1] + 1]);
        if !x.box_free(&pillar) {
            return None;
        }
        parts.push(pillar);
    }
    Some(Placed::boxes(parts, Overlap::None))
}

/// SV-02: on a slope with stepped footings: the underside steps down to the ground in
/// columns of different depth.
fn stepped_footings(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.stand)?;
    let step = 1 + below(x.words[10] >> 8, 2) as i32;
    let q = add(p, 1, step);
    if !x.free(q) {
        return None;
    }
    let b = expand(x.w, q, x.size, &UP_DIRS, x.words[11]);
    let mut depths: Vec<(i32, i32, i32)> = Vec::new();
    for z in b.lo[2]..b.hi[2] {
        for xx in b.lo[0]..b.hi[0] {
            let mut d = 0;
            while d < 3 && x.free([xx, b.lo[1] - 1 - d, z]) {
                d += 1;
            }
            let below_ok =
                x.w.support([xx, b.lo[1] - 1 - d, z], [xx, b.lo[1] - d, z], true);
            depths.push((xx, z, if below_ok { d } else { -1 }));
        }
    }
    if depths.iter().any(|d| d.2 < 0) {
        return None;
    }
    let distinct = {
        let mut v: Vec<i32> = depths.iter().map(|d| d.2).collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    if distinct < 2 {
        return None;
    }
    let mut parts = x.fill(b);
    for z in b.lo[2]..b.hi[2] {
        let mut xx = b.lo[0];
        while xx < b.hi[0] {
            let d = depths.iter().find(|e| e.0 == xx && e.1 == z)?.2;
            let mut end = xx;
            while end < b.hi[0] && depths.iter().find(|e| e.0 == end && e.1 == z)?.2 == d {
                end += 1;
            }
            if d > 0 {
                parts.push(bx([xx, b.lo[1] - d, z], [end, b.lo[1], z + 1]));
            }
            xx = end;
        }
    }
    Some(Placed::boxes(parts, Overlap::None))
}

/// The box of a hanging structure from the candidate `p`.
fn hung(x: &Ctx, p: [i32; 3]) -> SlotBox {
    expand(x.w, p, x.size, &DOWN_DIRS, x.words[11])
}

/// SV-03: kissing a standing tower below, joined by a stair.
fn kissing(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let b = hung(x, p);
    for g in 2..=4 {
        for z in b.lo[2]..b.hi[2] {
            for xx in b.lo[0]..b.hi[0] {
                let top = [xx, b.lo[1] - g, z];
                let gap = bx([xx, b.lo[1] - g + 1, z], [xx + 1, b.lo[1], z + 1]);
                if x.w.state(top) == Some(SlotState::Built) && x.box_free(&gap) {
                    let mut placed = Placed::boxes(x.fill(b), Overlap::None);
                    placed.shaped.push((Shaped::Stair(Dir4::PosX), gap));
                    return Some(placed);
                }
            }
        }
    }
    None
}

/// SV-04: ending in a viewing pod.
fn viewing_pod(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let b = hung(x, p);
    let (cx, cz) = ((b.lo[0] + b.hi[0]) / 2, (b.lo[2] + b.hi[2]) / 2);
    let pod = bx([cx, b.lo[1] - 1, cz], [cx + 1, b.lo[1], cz + 1]);
    if !x.box_free(&pod) {
        return None;
    }
    let mut placed = Placed::boxes(x.fill(b), Overlap::None);
    placed.shaped.push((Shaped::Cyl, pod));
    Some(placed)
}

/// SV-05: a chandelier cluster: three to six pendants of different lengths.
fn chandelier(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let mut rng = x.rng();
    let count = 3 + rng.below(4) as usize;
    let mut parts: Vec<SlotBox> = Vec::new();
    for _ in 0..count * 3 {
        if parts.len() >= count {
            break;
        }
        let q = [
            p[0] + rng.range_i32(-2, 3),
            p[1],
            p[2] + rng.range_i32(-2, 3),
        ];
        let long = 1 + rng.below(5) as i32;
        let wide = 1 + rng.below(2) as i32;
        if !x.hangs(q) {
            continue;
        }
        let b = expand(x.w, q, [wide, long, wide], &DOWN_DIRS, rng.next_u64());
        let overlaps = parts
            .iter()
            .any(|o| (0..3).all(|k| b.lo[k] < o.hi[k] && o.lo[k] < b.hi[k]));
        if !overlaps {
            parts.push(b);
        }
    }
    (parts.len() >= 3).then(|| Placed::boxes(parts, Overlap::None))
}

/// SV-06: broken off, floors exposed: the underside is cut away diagonally.
fn broken_off(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let b = hung(x, p);
    let (w, h) = (b.hi[0] - b.lo[0], b.hi[1] - b.lo[1]);
    if w < 3 || h < 3 {
        return None;
    }
    let parts = (0..w)
        .map(|i| {
            let keep = h - (h - 1) * i / (w - 1);
            bx(
                [b.lo[0] + i, b.hi[1] - keep, b.lo[2]],
                [b.lo[0] + i + 1, b.hi[1], b.hi[2]],
            )
        })
        .collect();
    Some(Placed::boxes(parts, Overlap::None))
}

/// SV-07: hung from cables, one to three slots below the support.
fn on_cables(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let gap = 1 + below(x.words[10] >> 8, 3) as i32;
    let q = add(p, 1, -gap);
    if !x.free(q) {
        return None;
    }
    let b = expand(x.w, q, x.size, &DOWN_DIRS, x.words[11]);
    let (cx, cz) = ((b.lo[0] + b.hi[0] - 1) / 2, (b.lo[2] + b.hi[2] - 1) / 2);
    let mut placed = Placed::boxes(x.fill(b), Overlap::None);
    placed.cables.push([cx, cz, b.hi[1], x.n()]);
    Some(placed)
}

/// SV-08: swung off-plumb: a box in a tilted frame, hinged at the support.
fn off_plumb(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let pitch = x.t.lattice.pitch as f64;
    let tilt = 1 + below(x.words[10] >> 8, 3) as u8;
    let yaw = [0u8, 16, 32, 48][below(x.words[11], 4)];
    let frame = Frame::table(yaw, tilt);
    let [ex, ey, ez] = frame.axes();
    let [w, h, d] = x.size;
    let size = [
        f64::from(w.min(4)) * pitch,
        f64::from(h.min(8)) * pitch,
        f64::from(d.min(4)) * pitch,
    ];
    let pivot = x.t.lattice.centre(x.t.lattice.index(p.map(|v| v as usize)))
        + DVec3::new(0.0, pitch / 2.0, 0.0);
    let origin = pivot - ey * size[1] - ex * (size[0] / 2.0) - ez * (size[2] / 2.0);
    Some(Placed {
        oriented: vec![OPart {
            frame,
            origin: origin.to_array(),
            size,
        }],
        ..Placed::boxes(Vec::new(), Overlap::None)
    })
}

fn gap_of(x: &Ctx) -> Option<([i32; 3], usize, i32)> {
    let &(k, axis, len) = x.c.gaps.get(below(x.words[9], x.c.gaps.len()))?;
    Some((x.w.xyz(k), usize::from(axis), i32::from(len)))
}

/// SV-09: a pillar building: two pillars at the ends of the gap and a span on top.
fn pillar_building(x: &Ctx) -> Option<Placed> {
    let (p, axis, len) = gap_of(x)?;
    let hp = 2 + below(x.words[10] >> 8, 3) as i32;
    let end = add(p, axis, len - 1);
    let a = bx(p, [p[0] + 1, p[1] + hp, p[2] + 1]);
    let b = bx(end, [end[0] + 1, end[1] + hp, end[2] + 1]);
    let mut hi = [p[0] + 1, p[1] + hp + 1, p[2] + 1];
    hi[axis] = p[axis] + len;
    let span = bx(add(p, 1, hp), hi);
    Some(Placed::boxes(vec![a, b, span], Overlap::Void))
}

/// SV-10: an arch building with a route under it: legs two slots high and a body on top.
fn arch_building(x: &Ctx) -> Option<Placed> {
    let (p, axis, len) = gap_of(x)?;
    let end = add(p, axis, len - 1);
    let a = bx(p, [p[0] + 1, p[1] + 2, p[2] + 1]);
    let b = bx(end, [end[0] + 1, end[1] + 2, end[2] + 1]);
    let mut hi = [p[0] + 1, p[1] + 4, p[2] + 1];
    hi[axis] = p[axis] + len;
    let body = bx(add(p, 1, 2), hi);
    Some(Placed::boxes(vec![a, b, body], Overlap::Void))
}

/// SV-11: a fallen tower lying diagonally across the void.
fn fallen_tower(x: &Ctx) -> Option<Placed> {
    let (p, axis, len) = gap_of(x)?;
    let pitch = x.t.lattice.pitch as f64;
    let tilt = 3;
    let yaw = x.yaw_for(tilt, unit(axis, 1))?;
    let frame = Frame::table(yaw, tilt);
    let [ex, ey, ez] = frame.axes();
    let size = [f64::from(len) * pitch, 2.0 * pitch, 2.0 * pitch];
    let start = x.t.lattice.centre(x.t.lattice.index(p.map(|v| v as usize)));
    let origin = start - ex * (pitch / 2.0) - ey * (pitch / 2.0) - ez * pitch;
    Some(Placed {
        oriented: vec![OPart {
            frame,
            origin: origin.to_array(),
            size,
        }],
        ..Placed::boxes(Vec::new(), Overlap::Void)
    })
}

/// SV-12: from both sides of a trunk.
fn both_sides(x: &Ctx) -> Option<Placed> {
    let t = x.pick(&x.c.trunk)?;
    let axis = if x.words[10] >> 8 & 1 == 0 { 0 } else { 2 };
    let other = 2 - axis;
    let mut parts = Vec::new();
    for s in [-1, 1] {
        let start = add(t, axis, s);
        if !x.free(start) {
            return None;
        }
        let mut caps = [0; 3];
        caps[axis] = x.size[1].max(2);
        caps[other] = x.size[0].min(3);
        caps[1] = x.size[2].min(3);
        let dirs = [(axis, s), (other, -1), (other, 1), (1, 1)];
        parts.push(expand(x.w, start, caps, &dirs, x.words[11]));
    }
    Some(Placed::boxes(parts, Overlap::None))
}

fn side_of(x: &Ctx) -> Option<([i32; 3], usize, i32)> {
    let &(k, axis, s) = x.c.side.get(below(x.words[9], x.c.side.len()))?;
    Some((x.w.xyz(k), usize::from(axis), i32::from(s)))
}

/// SV-13: sagging floor by floor: each floor along the wall sits lower than the last.
fn sagging(x: &Ctx) -> Option<Placed> {
    let (p, axis, s) = side_of(x)?;
    let other = 2 - axis;
    let floors = 3 + below(x.words[10] >> 8, 3) as i32;
    let wide = x.size[0].clamp(1, 4);
    let mut parts = Vec::new();
    for i in 0..floors {
        let mut lo = p;
        lo[axis] = if s > 0 {
            p[axis] + 2 * i
        } else {
            p[axis] - 2 * i - 1
        };
        lo[1] = p[1] - i / 2;
        lo[other] = p[other] - wide / 2;
        let mut hi = [lo[0] + 1, lo[1] + 1, lo[2] + 1];
        hi[axis] = lo[axis] + 2;
        hi[other] = lo[other] + wide;
        parts.push(bx(lo, hi));
    }
    Some(Placed::boxes(parts, Overlap::None))
}

/// SV-14: jutting out at a slant: a box in a frame tilted upwards from the wall.
fn slanted(x: &Ctx) -> Option<Placed> {
    let (p, axis, s) = side_of(x)?;
    let pitch = x.t.lattice.pitch as f64;
    let tilt = 2 + below(x.words[10] >> 8, 3) as u8;
    let yaw = x.yaw_for(tilt, unit(axis, s))?;
    let frame = Frame::table(yaw, tilt);
    let [ex, ey, ez] = frame.axes();
    let len = f64::from(x.size[1].clamp(3, 8));
    let size = [
        len * pitch,
        2.0 * pitch,
        f64::from(x.size[0].min(3)) * pitch,
    ];
    let c = x.t.lattice.centre(x.t.lattice.index(p.map(|v| v as usize)));
    let origin = c - ex * (pitch / 2.0) - ey * (pitch / 2.0) - ez * (size[2] / 2.0);
    Some(Placed {
        oriented: vec![OPart {
            frame,
            origin: origin.to_array(),
            size,
        }],
        ..Placed::boxes(Vec::new(), Overlap::None)
    })
}

/// SV-25: 300 m long and stepped: nine to twelve slots along the wall, in steps of three
/// that rise by one slot each.
fn stepped_long(x: &Ctx) -> Option<Placed> {
    let (p, axis, s) = side_of(x)?;
    let other = 2 - axis;
    let steps = 3 + below(x.words[10] >> 8, 2) as i32;
    let wide = x.size[0].clamp(1, 3);
    let mut parts = Vec::new();
    for k in 0..steps {
        let mut lo = p;
        lo[axis] = if s > 0 {
            p[axis] + 3 * k
        } else {
            p[axis] - 3 * k - 2
        };
        lo[other] = p[other] - wide / 2;
        let mut hi = [lo[0] + 1, lo[1] + 1 + k, lo[2] + 1];
        hi[axis] = lo[axis] + 3;
        hi[other] = lo[other] + wide;
        parts.push(bx(lo, hi));
    }
    Some(Placed::boxes(parts, Overlap::None))
}

fn ring_parts(x: i32, y0: i32, z: i32, hh: i32) -> [SlotBox; 4] {
    [
        bx([x - 1, y0, z - 1], [x + 2, y0 + hh, z]),
        bx([x - 1, y0, z + 1], [x + 2, y0 + hh, z + 2]),
        bx([x - 1, y0, z], [x, y0 + hh, z + 1]),
        bx([x + 1, y0, z], [x + 2, y0 + hh, z + 1]),
    ]
}

/// SV-15: a half ring: three of the four sides.
fn half_ring(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.trunk)?;
    let hh = x.size[1].clamp(2, 4);
    let ring = ring_parts(p[0], p[1], p[2], hh);
    let skip = below(x.words[10] >> 8, 4);
    let parts: Vec<SlotBox> = ring
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != skip)
        .map(|(_, b)| *b)
        .collect();
    Some(Placed::boxes(parts, Overlap::None))
}

/// SV-16: a true helix climbing the trunk.
fn helix(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.trunk)?;
    let hh = 4 + below(x.words[10] >> 8, 4) as i32;
    let b = bx([p[0] - 1, p[1], p[2] - 1], [p[0] + 2, p[1] + hh, p[2] + 2]);
    let sense = if x.words[11] & 1 == 0 { 1 } else { -1 };
    let mut placed = Placed::boxes(Vec::new(), Overlap::Solid);
    placed.shaped.push((Shaped::Helix { sense }, b));
    Some(placed)
}

/// SV-17: a façade with nothing behind it, on the face of a wall.
fn facade(x: &Ctx) -> Option<Placed> {
    let (p, axis, s) = side_of(x)?;
    let host = match (axis, s > 0) {
        (0, true) => Dir6::NegX,
        (0, false) => Dir6::PosX,
        (_, true) => Dir6::NegZ,
        (_, false) => Dir6::PosZ,
    };
    let other = 2 - axis;
    let mut caps = [0; 3];
    caps[axis] = 1;
    caps[other] = x.size[0].clamp(2, 5);
    caps[1] = x.size[1].clamp(2, 6);
    let dirs = [(other, -1), (other, 1), (1, -1), (1, 1)];
    let b = expand(x.w, p, caps, &dirs, x.words[11]);
    let mut placed = Placed::boxes(Vec::new(), Overlap::None);
    placed.shaped.push((Shaped::Facade(host), b));
    Some(placed)
}

/// SV-18: windows only: openings in a wall with no mass of their own.
fn windows_only(x: &Ctx) -> Option<Placed> {
    let (p, axis, s) = side_of(x)?;
    let facing = match (axis, s > 0) {
        (0, true) => Dir6::PosX,
        (0, false) => Dir6::NegX,
        (_, true) => Dir6::PosZ,
        (_, false) => Dir6::NegZ,
    };
    let mut placed = Placed::boxes(Vec::new(), Overlap::None);
    placed
        .windows
        .push((bx(p, [p[0] + 1, p[1] + 1, p[2] + 1]), facing));
    Some(placed)
}

/// SV-19: a cable-hung platform town: flat platforms at different heights.
fn platform_town(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let mut rng = x.rng();
    let count = 3 + rng.below(4) as usize;
    let mut placed = Placed::boxes(Vec::new(), Overlap::None);
    for _ in 0..count * 3 {
        if placed.parts.len() >= count {
            break;
        }
        let q = [
            p[0] + rng.range_i32(-4, 5),
            p[1] - rng.range_i32(1, 5),
            p[2] + rng.range_i32(-4, 5),
        ];
        let b = bx(
            q,
            [
                q[0] + 2 + rng.range_i32(0, 3),
                q[1] + 1,
                q[2] + 2 + rng.range_i32(0, 3),
            ],
        );
        let clear = x.box_free(&b)
            && !placed
                .parts
                .iter()
                .any(|o| (0..3).all(|k| b.lo[k] < o.hi[k] && o.lo[k] < b.hi[k]));
        if clear {
            placed.cables.push([
                (b.lo[0] + b.hi[0] - 1) / 2,
                (b.lo[2] + b.hi[2] - 1) / 2,
                b.hi[1],
                x.n(),
            ]);
            placed.parts.push(b);
        }
    }
    (placed.parts.len() >= 3).then_some(placed)
}

/// SV-20: cages hung from cables.
fn cages(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hang)?;
    let mut rng = x.rng();
    let count = 1 + rng.below(3) as usize;
    let mut placed = Placed::boxes(Vec::new(), Overlap::None);
    for _ in 0..count * 3 {
        if placed.shaped.len() >= count {
            break;
        }
        let gap = rng.range_i32(1, 4);
        let q = [
            p[0] + rng.range_i32(-2, 3),
            p[1] - gap - 2,
            p[2] + rng.range_i32(-2, 3),
        ];
        let b = bx(q, [q[0] + 2, q[1] + 3, q[2] + 2]);
        let clear = x.box_free(&b)
            && !placed
                .shaped
                .iter()
                .any(|(_, o)| (0..3).all(|k| b.lo[k] < o.hi[k] && o.lo[k] < b.hi[k]));
        if clear {
            placed.cables.push([q[0], q[2], b.hi[1], x.n()]);
            placed.shaped.push((Shaped::Cage, b));
        }
    }
    (!placed.shaped.is_empty()).then_some(placed)
}

/// SV-21: an omnidirectional cluster: a core with boxes on five of its faces.
fn omnidirectional(x: &Ctx) -> Option<Placed> {
    let pool: &[u32] = if x.c.stand.is_empty() {
        &x.c.hang
    } else {
        &x.c.stand
    };
    let p = x.pick(pool)?;
    let core = bx(p, [p[0] + 2, p[1] + 2, p[2] + 2]);
    if !x.box_free(&core) {
        return None;
    }
    let mut parts = vec![core];
    for (k, s) in [(0, -1), (0, 1), (2, -1), (2, 1), (1, 1)] {
        let mut lo = core.lo;
        let mut hi = core.hi;
        if s > 0 {
            lo[k] = core.hi[k];
            hi[k] = core.hi[k] + 1;
        } else {
            lo[k] = core.lo[k] - 1;
            hi[k] = core.lo[k];
        }
        let arm = bx(lo, hi);
        if x.box_free(&arm) {
            parts.push(arm);
        }
    }
    (parts.len() >= 4).then(|| Placed::boxes(parts, Overlap::None))
}

/// SV-22: a leaning pile across an alley: one box per level, each a step further across.
fn leaning_pile(x: &Ctx) -> Option<Placed> {
    let (p, axis, len) = gap_of(x)?;
    let parts: Vec<SlotBox> = (0..len)
        .map(|i| {
            let mut lo = add(p, 1, i);
            lo[axis] = p[axis] + i;
            bx(lo, [lo[0] + 1, lo[1] + 1, lo[2] + 1])
        })
        .collect();
    Some(Placed::boxes(parts, Overlap::Void))
}

fn hall_extent(x: &Ctx, p: [i32; 3]) -> [i32; 3] {
    let ext = |k: usize| {
        (1..=12)
            .take_while(|&i| x.w.state(add(p, k, i - 1)) == Some(SlotState::Void))
            .count() as i32
    };
    [ext(0), ext(1), ext(2)]
}

/// SV-23: a town in a hall: four to eight small buildings on its floor.
fn town_in_a_hall(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hall)?;
    let e = hall_extent(x, p);
    if e[0] < 4 || e[2] < 4 || e[1] < 2 {
        return None;
    }
    let mut rng = x.rng();
    let count = 4 + rng.below(5) as usize;
    let mut parts: Vec<SlotBox> = Vec::new();
    for _ in 0..count * 4 {
        if parts.len() >= count {
            break;
        }
        let (w, d) = (1 + rng.below(2) as i32, 1 + rng.below(2) as i32);
        let h = 1 + rng.below(e[1].min(3) as u64) as i32;
        let lo = [
            p[0] + rng.range_i32(0, e[0] - w + 1),
            p[1],
            p[2] + rng.range_i32(0, e[2] - d + 1),
        ];
        let b = bx(lo, [lo[0] + w, lo[1] + h, lo[2] + d]);
        if !parts
            .iter()
            .any(|o| (0..3).all(|k| b.lo[k] < o.hi[k] && o.lo[k] < b.hi[k]))
        {
            parts.push(b);
        }
    }
    (parts.len() >= 3).then(|| Placed::boxes(parts, Overlap::Void))
}

/// SV-24: a district inside a hollow tether: tiers stacked up a tall narrow void.
fn district_in_a_tether(x: &Ctx) -> Option<Placed> {
    let p = x.pick(&x.c.hall)?;
    let e = hall_extent(x, p);
    if e[1] < 3 || e[0] < 2 || e[2] < 2 {
        return None;
    }
    let mut parts = Vec::new();
    let mut y = 0;
    while y + 1 < e[1] {
        let (w, d) = (e[0].min(1 + (y / 2) % 2 + 1), e[2].min(2));
        parts.push(bx(
            [p[0], p[1] + y, p[2]],
            [p[0] + w, p[1] + y + 1, p[2] + d],
        ));
        y += 2;
    }
    (parts.len() >= 2).then(|| Placed::boxes(parts, Overlap::Void))
}
