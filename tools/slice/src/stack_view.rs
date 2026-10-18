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

fn tag_colour(tag: &str) -> Rgb<u8> {
    match tag {
        "STR-03" => Rgb([230, 230, 220]),
        "STR-04" => Rgb([240, 150, 60]),
        "STR-05" => Rgb([90, 200, 120]),
        "STR-06" => Rgb([220, 90, 200]),
        "STR-07" => Rgb([80, 160, 255]),
        "STR-08" => Rgb([250, 230, 70]),
        "STR-09" => Rgb([255, 120, 120]),
        "STR-10" => Rgb([150, 110, 255]),
        "STR-11" => Rgb([255, 255, 255]),
        "SURF-01" => Rgb([200, 150, 90]),
        "SURF-02" => Rgb([60, 170, 170]),
        "SURF-03" => Rgb([230, 80, 40]),
        "SURF-04" => Rgb([120, 130, 170]),
        "FRM-13" => Rgb([255, 240, 80]),
        "VOID-09" => Rgb([255, 120, 0]),
        "VOID-11" | "VOID-12" => Rgb([0, 255, 120]),
        t if t.starts_with("VOID-") => Rgb([230, 40, 200]),
        t if t.starts_with("FRM-") => Rgb([90, 220, 255]),
        _ => Rgb([255, 0, 255]),
    }
}

/// `--between`: an elevation of the Between content of the L1 layer holding the plane, drawn
/// over the stacks, one colour per registry ID. Pieces are drawn as their projected bounds, so
/// thin ones stay visible.
pub fn render_content(seed: WorldSeed, view: &View) -> RgbImage {
    use worldgen::l1_between::between_content;
    use worldgen::stack::u_units;

    let res = view.res;
    let mut img = RgbImage::new(res, res);
    let scale = view.extent_m / f64::from(res);
    let at = view.at_m.floor() as i64;
    let mut stacks: HashMap<I64Vec3, Stack> = HashMap::new();
    for py in 0..res {
        for px in 0..res {
            let u = view.center.0 as f64 + (f64::from(px) + 0.5) * scale - view.extent_m / 2.0;
            let v = view.center.1 as f64 + (f64::from(py) + 0.5) * scale - view.extent_m / 2.0;
            let p = plane_point(view.axis, u, v, at);
            let l0 = p.div_euclid(I64Vec3::splat(L0));
            let local = p.rem_euclid(I64Vec3::splat(L0));
            let st = stacks.entry(l0).or_insert_with(|| stack(seed, l0));
            let seg = *st.segment_at(u_units(st.geometry, local, None));
            let c = match seg.stratum {
                Stratum::Crust(_) => Rgb([70, 72, 78]),
                Stratum::Between(_) => Rgb([8, 8, 12]),
            };
            img.put_pixel(px, py, c);
        }
    }
    // The L1 cells of the plane's layer that the view covers.
    let half = view.extent_m / 2.0;
    let (u0, u1) = (view.center.0 as f64 - half, view.center.0 as f64 + half);
    let (v0, v1) = (view.center.1 as f64 - half, view.center.1 as f64 + half);
    let corner_a = plane_point(view.axis, u0, v0, at);
    let corner_b = plane_point(view.axis, u1, v1, at);
    let (lo, hi) = (corner_a.min(corner_b), corner_a.max(corner_b));
    let (lo, hi) = (
        lo.div_euclid(I64Vec3::splat(4096)),
        hi.div_euclid(I64Vec3::splat(4096)),
    );
    let layer = at.div_euclid(4096);
    let normal = view.axis;
    let mut order: Vec<(u8, usize, [i64; 4], &'static str)> = Vec::new();
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                let l1 = I64Vec3::new(x, y, z);
                let l = match normal {
                    Axis::X => l1.x,
                    Axis::Y => l1.y,
                    Axis::Z => l1.z,
                };
                if l != layer {
                    continue;
                }
                let origin = l1 * 4096;
                let mut pieces = between_content(seed, l1);
                pieces.extend(worldgen::l1_surface::surface_content(seed, l1));
                pieces.extend(worldgen::l1_frame::frame_content(seed, l1));
                pieces.extend(
                    worldgen::l1_void::voids_of_cell(seed, l1)
                        .into_iter()
                        .map(|v| v.piece),
                );
                for (i, w) in pieces.iter().enumerate() {
                    let b = w.prim.aabb();
                    let m = |a: i32| f64::from(a) / 16.0;
                    let (wx0, wx1) = (origin.x as f64 + m(b.min.x), origin.x as f64 + m(b.max.x));
                    let (wy0, wy1) = (origin.y as f64 + m(b.min.y), origin.y as f64 + m(b.max.y));
                    let (wz0, wz1) = (origin.z as f64 + m(b.min.z), origin.z as f64 + m(b.max.z));
                    // Plane coordinates, (u right, v down).
                    let (a0, a1, b0, b1) = match normal {
                        Axis::Y => (wx0, wx1, wz0, wz1),
                        Axis::X => (wz0, wz1, -wy1, -wy0),
                        Axis::Z => (wx0, wx1, -wy1, -wy0),
                    };
                    let to_px = |u: f64, c: i64| ((u - (c as f64 - half)) / scale).floor() as i64;
                    let r = [
                        to_px(a0, view.center.0),
                        to_px(b0, view.center.1),
                        to_px(a1, view.center.0),
                        to_px(b1, view.center.1),
                    ];
                    order.push((rank(w.tag), i, r, w.tag));
                }
            }
        }
    }
    // Windows first, then the rest in a fixed order, so big pieces do not hide small ones.
    order.sort_by_key(|o| o.0);
    for (_, _, r, tag) in order {
        let c = tag_colour(tag);
        for y in r[1].max(0)..=r[3].min(i64::from(res) - 1) {
            for x in r[0].max(0)..=r[2].min(i64::from(res) - 1) {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
    img
}

fn rank(tag: &str) -> u8 {
    match tag {
        "SURF-04" | "VOID-01" | "VOID-02" | "VOID-03" | "VOID-09" | "VOID-10" | "VOID-13" => 0,
        "STR-11" | "STR-03" => 1,
        "STR-10" => 2,
        "STR-09" => 9,
        _ => 5,
    }
}

fn plane_point(axis: Axis, u: f64, v: f64, at: i64) -> I64Vec3 {
    match axis {
        Axis::Y => I64Vec3::new(u.floor() as i64, at, v.floor() as i64),
        Axis::X => I64Vec3::new(at, (-v).floor() as i64, u.floor() as i64),
        Axis::Z => I64Vec3::new(u.floor() as i64, (-v).floor() as i64, at),
    }
}
