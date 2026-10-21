//! The glyph sheet of a world (TOOL-05): the 32 glyphs of its alphabet, and the names of a few
//! L1 cells set in them.

use glam::I64Vec3;
use image::{Rgb, RgbImage};
use worldgen::glyphs::{ALPHABET, SIDE, alphabet, name_of};
use worldgen::seed::WorldSeed;

const CELL: u32 = 8;
const GAP: u32 = 10;

fn draw(img: &mut RgbImage, g: [u8; SIDE], x0: u32, y0: u32) {
    for (row, bits) in g.iter().enumerate() {
        for col in 0..SIDE {
            if bits >> col & 1 == 1 {
                for dy in 0..CELL - 1 {
                    for dx in 0..CELL - 1 {
                        img.put_pixel(
                            x0 + col as u32 * CELL + dx,
                            y0 + row as u32 * CELL + dy,
                            Rgb([232, 226, 206]),
                        );
                    }
                }
            }
        }
    }
}

/// 8 columns of glyphs with their numbers implied by order, then ten names.
pub fn render(seed: WorldSeed) -> RgbImage {
    let glyph = SIDE as u32 * CELL;
    let cols = 8u32;
    let width = cols * (glyph + GAP) + GAP;
    let rows = (ALPHABET as u32).div_ceil(cols);
    let height = rows * (glyph + GAP) + GAP + 12 * (glyph + GAP) / 3 * 2 + GAP;
    let mut img = RgbImage::from_pixel(width, height, Rgb([16, 18, 22]));
    let a = alphabet(seed);
    for (i, g) in a.iter().enumerate() {
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        draw(
            &mut img,
            *g,
            GAP + c * (glyph + GAP),
            GAP + r * (glyph + GAP),
        );
    }
    let top = rows * (glyph + GAP) + 2 * GAP;
    for n in 0..8i64 {
        let name = name_of(seed, I64Vec3::new(n * 3, 1, -n));
        for (k, &gi) in name.iter().enumerate() {
            draw(
                &mut img,
                a[usize::from(gi)],
                GAP + k as u32 * (glyph + GAP / 2),
                top + n as u32 * (glyph + GAP / 2),
            );
        }
    }
    img
}
