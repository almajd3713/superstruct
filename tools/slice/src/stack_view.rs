//! `--stack`: a cross-section of the regime stacks (crusts and Betweens), PW.2.2.
//! Crusts are light grey. Betweens are dark, tinted by kind; flooded ones are blue.

use std::collections::HashMap;

use glam::I64Vec3;
use image::{Rgb, RgbImage};
use worldgen::cell::Axis;
use worldgen::seed::WorldSeed;
use worldgen::stack::{BetweenKind, Stack, Stratum, stack};

use crate::render::View;

const L0: i64 = 32_768;

fn between_colour(kind: BetweenKind, flooded: bool) -> Rgb<u8> {
    let base = match kind {
        BetweenKind::Shallow => [70, 56, 44],
        BetweenKind::Normal => [34, 34, 38],
        BetweenKind::Abyssal => [10, 10, 14],
        BetweenKind::Pillared => [52, 44, 70],
        BetweenKind::Webbed => [40, 62, 52],
    };
    if flooded {
        Rgb([base[0] / 2, base[1] / 2 + 20, base[2] / 2 + 70])
    } else {
        Rgb(base)
    }
}

/// Draws the plane of `view` (axis x or z; y shows the stack from above, which is only
/// meaningful for vertical stacks).
pub fn render(seed: WorldSeed, view: &View) -> RgbImage {
    let res = view.res;
    let mut img = RgbImage::new(res, res);
    let mut cache: HashMap<I64Vec3, Stack> = HashMap::new();
    let scale = view.extent_m / f64::from(res);
    for py in 0..res {
        for px in 0..res {
            let u = view.center.0 as f64 + (f64::from(px) + 0.5) * scale - view.extent_m / 2.0;
            let v = view.center.1 as f64 + (f64::from(py) + 0.5) * scale - view.extent_m / 2.0;
            let at = view.at_m.floor() as i64;
            // Plane coordinates → world metres.
            let p = match view.axis {
                Axis::Y => I64Vec3::new(u.floor() as i64, at, v.floor() as i64),
                Axis::X => I64Vec3::new(at, (-v).floor() as i64, u.floor() as i64),
                Axis::Z => I64Vec3::new(u.floor() as i64, (-v).floor() as i64, at),
            };
            let cell = p.div_euclid(I64Vec3::splat(L0));
            let local = p.rem_euclid(I64Vec3::splat(L0));
            let st = cache.entry(cell).or_insert_with(|| stack(seed, cell));
            let seg = *st.segment_at(worldgen::stack::u_units(st.geometry, local, None));
            let mut c = match (seg.stratum, seg.kind) {
                (Stratum::Crust(_), _) => Rgb([172, 172, 168]),
                (_, Some(kind)) => between_colour(kind, seg.flooded),
                _ => Rgb([0, 0, 0]),
            };
            if local.x < i64::from(scale as i32 + 1) || local.y < i64::from(scale as i32 + 1) {
                c = Rgb([200, 60, 60]);
            }
            img.put_pixel(px, py, c);
        }
    }
    img
}
