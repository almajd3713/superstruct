//! Mutations MUT-01..15: changes to a form before it is placed (`levels.md` §5W.4.2, §5W.15).
//! A mutation turns a `FormOut` into another one; up to two are applied in the drawn order.
//!
//! Groups (`regimes.md` §9): POSE 01 inverted, 02 tilted, 03 twisted · DAMAGE 04 broken,
//! 05 overgrown, 15 swallowed · COPY 06 doubled or mirrored, 07 stretched · HOLLOW 08 hollowed,
//! 09 caged, 10 pierced · LIGHT 11 dark, 12 lit · SCALE 13 scaled, 14 fused.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::forms::{FORMS, FormOut, Turned};
use crate::rng::Rng;
use crate::tissue_grow::{Shaped, SlotBox};

/// The mutations of each of the six groups of `Character::mutations`.
pub const GROUPS: [&[u8]; 6] = [
    &[1, 2, 3],
    &[4, 5, 15],
    &[6, 7],
    &[8, 9, 10],
    &[11, 12],
    &[13, 14],
];

fn bx(lo: [i32; 3], hi: [i32; 3]) -> SlotBox {
    SlotBox { lo, hi }
}

fn map_boxes(out: &mut FormOut, f: impl Fn(SlotBox) -> SlotBox) {
    out.boxes = out.boxes.iter().map(|&b| f(b)).collect();
    out.shaped = out.shaped.iter().map(|&(s, b)| (s, f(b))).collect();
    for t in &mut out.turned {
        t.bx = f(t.bx);
    }
    out.inner = out.inner.iter().map(|&b| f(b)).collect();
}

fn extent(out: &FormOut) -> ([i32; 3], [i32; 3]) {
    let all = out.envelope();
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for b in &all {
        for k in 0..3 {
            lo[k] = lo[k].min(b.lo[k]);
            hi[k] = hi[k].max(b.hi[k]);
        }
    }
    if all.is_empty() {
        ([0; 3], [0; 3])
    } else {
        (lo, hi)
    }
}

/// MUT-07 builds the form at half its length first; the rest act on a built form.
pub fn size_before(id: u8, size: [i32; 3], rng: &mut Rng) -> [i32; 3] {
    if id == 7 {
        let k = rng.below(3) as usize;
        let mut s = size;
        s[k] = (s[k] / 2).max(1);
        s
    } else {
        size
    }
}

/// Applies mutation `id` to `out`, built for `size` (the size `size_before` gave, scaled back
/// up by `stretch` for MUT-07). `size` is the bounding box the structure will occupy.
#[allow(clippy::too_many_lines)] // one arm per mutation
pub fn apply(mut out: FormOut, size: [i32; 3], id: u8, rng: &mut Rng) -> FormOut {
    let [w, h, d] = size;
    match id {
        // Inverted: the form upside down on its support.
        1 => {
            map_boxes(&mut out, |b| {
                bx(
                    [b.lo[0], h - b.hi[1], b.lo[2]],
                    [b.hi[0], h - b.lo[1], b.hi[2]],
                )
            });
            for (s, _) in &mut out.shaped {
                if let Shaped::Cone { inverted } = s {
                    *inverted = !*inverted;
                }
            }
            out
        }
        // Tilted: the whole form turned about the centre of its base.
        2 => {
            let tilt = 1 + rng.below(3) as u8;
            let yaw = [0u8, 16, 32, 48][rng.below(4) as usize];
            let pivot = Some([f64::from(w) / 2.0, f64::from(d) / 2.0]);
            let mut turned = out.turned.clone();
            for b in out
                .boxes
                .drain(..)
                .chain(out.shaped.drain(..).map(|(_, b)| b))
            {
                turned.push(Turned {
                    bx: b,
                    yaw,
                    tilt,
                    pivot,
                });
            }
            for t in &mut turned {
                if t.pivot.is_none() {
                    t.pivot = pivot;
                }
                t.tilt = tilt;
                t.yaw = yaw;
            }
            out.turned = turned;
            out
        }
        // Twisted: each box turned about the vertical axis by more the higher it stands.
        3 => {
            let step = 2 + rng.below(3) as u8;
            let pivot = Some([f64::from(w) / 2.0, f64::from(d) / 2.0]);
            let mut turned = Vec::new();
            for b in out
                .boxes
                .drain(..)
                .chain(out.shaped.drain(..).map(|(_, b)| b))
            {
                // A tall box is cut into floors so that it can twist.
                for y in b.lo[1]..b.hi[1] {
                    turned.push(Turned {
                        bx: bx([b.lo[0], y, b.lo[2]], [b.hi[0], y + 1, b.hi[2]]),
                        yaw: (y as u8).wrapping_mul(step) % 64,
                        tilt: 0,
                        pivot,
                    });
                }
            }
            turned.append(&mut out.turned);
            out.turned = turned;
            out
        }
        // Broken: ragged tops, as if the upper floors had come away.
        4 => {
            let cut = (h * 3 / 5).max(1);
            let mut boxes = Vec::new();
            for b in out.boxes.drain(..) {
                if b.hi[1] <= cut {
                    boxes.push(b);
                    continue;
                }
                for (i, x) in (b.lo[0]..b.hi[0]).enumerate() {
                    let top = (cut + (i as i32 * 7 + rng.below(3) as i32) % (b.hi[1] - cut + 1))
                        .clamp(b.lo[1] + 1, b.hi[1]);
                    boxes.push(bx([x, b.lo[1], b.lo[2]], [x + 1, top, b.hi[2]]));
                }
            }
            out.boxes = boxes;
            out
        }
        // Overgrown: vines hang down the sides.
        5 => {
            let (lo, hi) = extent(&out);
            for _ in 0..(3 + rng.below(4)) {
                let on_x = rng.below(2) == 0;
                let (x, z) = if on_x {
                    (
                        if rng.below(2) == 0 { lo[0] } else { hi[0] - 1 },
                        lo[2] + rng.range_i32(0, (hi[2] - lo[2]).max(1)),
                    )
                } else {
                    (
                        lo[0] + rng.range_i32(0, (hi[0] - lo[0]).max(1)),
                        if rng.below(2) == 0 { lo[2] } else { hi[2] - 1 },
                    )
                };
                out.vines.push([x, z, lo[1], hi[1]]);
            }
            out
        }
        // Doubled or mirrored: the left half and its reflection.
        6 => {
            let half = (w / 2).max(1);
            let clip = |b: SlotBox| {
                (b.lo[0] < half).then(|| bx(b.lo, [b.hi[0].min(half), b.hi[1], b.hi[2]]))
            };
            let mirror = |b: SlotBox| {
                bx(
                    [w - b.hi[0], b.lo[1], b.lo[2]],
                    [w - b.lo[0], b.hi[1], b.hi[2]],
                )
            };
            let mut boxes: Vec<SlotBox> = out.boxes.iter().filter_map(|&b| clip(b)).collect();
            let copies: Vec<SlotBox> = boxes.iter().map(|&b| mirror(b)).collect();
            boxes.extend(copies);
            let mut shaped: Vec<(Shaped, SlotBox)> = out
                .shaped
                .iter()
                .filter_map(|&(s, b)| clip(b).map(|b| (s, b)))
                .collect();
            let copies: Vec<(Shaped, SlotBox)> =
                shaped.iter().map(|&(s, b)| (s, mirror(b))).collect();
            shaped.extend(copies);
            out.boxes = boxes;
            out.shaped = shaped;
            out
        }
        // Stretched: every piece twice as long along one axis (the form was built at half).
        7 => {
            let k = (0..3).max_by_key(|&k| size[k]).unwrap_or(1);
            let _ = k;
            let k = rng.below(3) as usize;
            map_boxes(&mut out, |b| {
                let mut b = b;
                b.lo[k] = (b.lo[k] * 2).min(size[k] - 1);
                b.hi[k] = (b.hi[k] * 2).min(size[k]).max(b.lo[k] + 1);
                b
            });
            out
        }
        // Hollowed: walls round an empty inside.
        8 => {
            let mut boxes = Vec::new();
            for b in out.boxes.drain(..) {
                let (bw, bd) = (b.hi[0] - b.lo[0], b.hi[2] - b.lo[2]);
                if bw >= 3 && bd >= 3 {
                    boxes.push(bx(b.lo, [b.hi[0], b.hi[1], b.lo[2] + 1]));
                    boxes.push(bx([b.lo[0], b.lo[1], b.hi[2] - 1], b.hi));
                    boxes.push(bx(
                        [b.lo[0], b.lo[1], b.lo[2] + 1],
                        [b.lo[0] + 1, b.hi[1], b.hi[2] - 1],
                    ));
                    boxes.push(bx(
                        [b.hi[0] - 1, b.lo[1], b.lo[2] + 1],
                        [b.hi[0], b.hi[1], b.hi[2] - 1],
                    ));
                } else {
                    boxes.push(b);
                }
            }
            out.boxes = boxes;
            out
        }
        // Caged: the boxes become frames.
        9 => {
            let boxes = std::mem::take(&mut out.boxes);
            out.shaped.extend(
                boxes
                    .into_iter()
                    .filter(|b| b.volume() >= 2)
                    .map(|b| (Shaped::Cage, b)),
            );
            out
        }
        // Pierced: a one-slot passage bored through the middle for a route.
        10 => {
            let (lo, hi) = extent(&out);
            let along_x = (hi[0] - lo[0]) >= (hi[2] - lo[2]);
            let y = lo[1] + rng.range_i32(0, (hi[1] - lo[1]).max(1));
            let z = lo[2] + (hi[2] - lo[2]) / 2;
            let x = lo[0] + (hi[0] - lo[0]) / 2;
            let (axis, start, len) = if along_x {
                (0, [lo[0], y, z], hi[0] - lo[0])
            } else {
                (2, [x, y, lo[2]], hi[2] - lo[2])
            };
            // The boxes that hold the passage lose it.
            let mut boxes = Vec::new();
            for b in out.boxes.drain(..) {
                let hit = b.lo[1] <= y
                    && y < b.hi[1]
                    && if along_x {
                        b.lo[2] <= z && z < b.hi[2]
                    } else {
                        b.lo[0] <= x && x < b.hi[0]
                    };
                if !hit {
                    boxes.push(b);
                    continue;
                }
                let (p, q) = if along_x { (z, 2) } else { (x, 0) };
                for (l, hh) in [(b.lo[q], p), (p + 1, b.hi[q])] {
                    if l < hh {
                        let mut c = b;
                        c.lo[q] = l;
                        c.hi[q] = hh;
                        boxes.push(c);
                    }
                }
                let mut a = b;
                a.lo[1] = b.lo[1];
                a.hi[1] = y;
                if a.hi[1] > a.lo[1] {
                    let mut c = a;
                    c.lo[q] = p;
                    c.hi[q] = p + 1;
                    boxes.push(c);
                }
                let mut a = b;
                a.lo[1] = y + 1;
                if a.hi[1] > a.lo[1] {
                    let mut c = a;
                    c.lo[q] = p;
                    c.hi[q] = p + 1;
                    boxes.push(c);
                }
            }
            out.boxes = boxes;
            out.pierce = Some((axis, start, len));
            out
        }
        11 => {
            out.dark = true;
            out.lit = false;
            out
        }
        12 => {
            out.lit = true;
            out.dark = false;
            out
        }
        // Swallowed: an older form inside the newer mass.
        15 => {
            let older = &FORMS[1 + rng.below((FORMS.len() - 1) as u64) as usize];
            let inner = [(w / 2).max(1), (h / 2).max(1), (d / 2).max(1)];
            let mut sub = Rng::new(rng.next_u64());
            if let Some(f) = (older.build)(inner, &mut sub) {
                let off = [(w - inner[0]) / 2, (h - inner[1]) / 2, (d - inner[2]) / 2];
                out.inner = f.shifted(off).envelope();
            }
            out
        }
        // Scaled (13) and fused (14) change the size and the overlap before placement.
        _ => out,
    }
}
