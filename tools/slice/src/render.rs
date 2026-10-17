//! Rasterizes a plane through the generated world.

use glam::{I64Vec3, Vec3};
use image::{Rgb, RgbImage};
use worldgen::cell::{Axis, CellKey, Level, floor_div_pow2};
use worldgen::l1::l1_cell;
use worldgen::l2::l2_cell_in;
use worldgen::l2_port::L1Cache;
use worldgen::prim::{CellOutput, FeatureKind, Primitive, QAabb, Shape};
use worldgen::seed::WorldSeed;
use worldgen::validate::CellNav;
use worldgen::walk::Nav;

/// Quanta per metre.
const Q: i64 = 16;

pub struct View {
    pub axis: Axis,
    pub at_m: f64,
    pub center: (i64, i64),
    pub extent_m: f64,
    pub res: u32,
}

pub struct Options {
    pub level: u8,
    pub grid: bool,
    pub ports: bool,
    pub walkable: bool,
}

/// Indices of the plane's coordinates `(u, v)` into `[x, y, z]`, and whether `v` runs up the image.
fn plane(axis: Axis) -> (usize, usize, bool) {
    match axis {
        Axis::Y => (0, 2, false),
        Axis::X => (2, 1, true),
        Axis::Z => (0, 1, true),
    }
}

struct Canvas<'a> {
    view: &'a View,
    img: RgbImage,
    /// Plane position in quanta.
    at: i64,
    u_axis: usize,
    v_axis: usize,
    v_up: bool,
    /// Image origin and pixel size in quanta.
    u0: i64,
    v0: i64,
    step: f64,
    /// Opacity of the next `fill`, 0 to 1.
    alpha: f32,
}

impl<'a> Canvas<'a> {
    fn new(view: &'a View) -> Self {
        let (u_axis, v_axis, v_up) = plane(view.axis);
        let half = view.extent_m / 2.0;
        let q = Q as f64;
        let u0 = ((view.center.0 as f64 - half) * q).round() as i64;
        let v0 = if v_up {
            ((view.center.1 as f64 + half) * q).round() as i64
        } else {
            ((view.center.1 as f64 - half) * q).round() as i64
        };
        Self {
            view,
            img: RgbImage::from_pixel(view.res, view.res, Rgb([8, 8, 10])),
            at: (view.at_m * q).round() as i64,
            u_axis,
            v_axis,
            v_up,
            u0,
            v0,
            step: view.extent_m * q / f64::from(view.res),
            alpha: 1.0,
        }
    }

    /// Pixel column for a `u` in quanta (may fall outside the image).
    fn col(&self, u: i64) -> i64 {
        ((u - self.u0) as f64 / self.step).floor() as i64
    }

    fn row(&self, v: i64) -> i64 {
        let d = if self.v_up { self.v0 - v } else { v - self.v0 };
        (d as f64 / self.step).floor() as i64
    }

    /// Plan-view quanta of a pixel centre.
    fn world(&self, col: i64, row: i64) -> (i64, i64) {
        let u = self.u0 + ((col as f64 + 0.5) * self.step) as i64;
        let dv = ((row as f64 + 0.5) * self.step) as i64;
        (
            u,
            if self.v_up {
                self.v0 - dv
            } else {
                self.v0 + dv
            },
        )
    }

    /// Fills the pixels where the world box `[min, max)` meets the plane. `test` refines the
    /// coverage per pixel with the world point in quanta.
    fn fill(
        &mut self,
        min: [i64; 3],
        max: [i64; 3],
        color: [u8; 3],
        test: &dyn Fn([i64; 3]) -> bool,
    ) {
        let a = match self.view.axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        if !(min[a] <= self.at && self.at < max[a]) {
            return;
        }
        let (c0, c1) = (self.col(min[self.u_axis]), self.col(max[self.u_axis] - 1));
        let (r_lo, r_hi) = if self.v_up {
            (self.row(max[self.v_axis] - 1), self.row(min[self.v_axis]))
        } else {
            (self.row(min[self.v_axis]), self.row(max[self.v_axis] - 1))
        };
        let res = i64::from(self.view.res);
        for row in r_lo.max(0)..=r_hi.min(res - 1) {
            for col in c0.max(0)..=c1.min(res - 1) {
                let (u, v) = self.world(col, row);
                let mut w = [0; 3];
                w[a] = self.at;
                w[self.u_axis] = u;
                w[self.v_axis] = v;
                if (min[0]..max[0]).contains(&w[0])
                    && (min[1]..max[1]).contains(&w[1])
                    && (min[2]..max[2]).contains(&w[2])
                    && test(w)
                {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    // clamped above
                    {
                        let p = self.img.get_pixel_mut(col as u32, row as u32);
                        for (dst, &src) in p.0.iter_mut().zip(&color) {
                            let mixed =
                                f32::from(*dst) * (1.0 - self.alpha) + f32::from(src) * self.alpha;
                            *dst = mixed as u8;
                        }
                    }
                }
            }
        }
    }

    /// A one-pixel line along `u` or `v` at `at_q`, spanning the whole image.
    fn line(&mut self, along_u: bool, at_q: i64, color: [u8; 3]) {
        let res = i64::from(self.view.res);
        let idx = if along_u {
            self.row(at_q)
        } else {
            self.col(at_q)
        };
        if !(0..res).contains(&idx) {
            return;
        }
        for k in 0..res {
            let (x, y) = if along_u { (k, idx) } else { (idx, k) };
            #[allow(clippy::cast_sign_loss)]
            {
                let p = self.img.get_pixel_mut(x as u32, y as u32);
                for (dst, &src) in p.0.iter_mut().zip(&color) {
                    *dst = u8::midpoint(*dst, src);
                }
            }
        }
    }
}

fn color_of(kind: FeatureKind) -> [u8; 3] {
    match kind {
        FeatureKind::TRUNK => [88, 88, 100],
        FeatureKind::DECK => [150, 150, 150],
        FeatureKind::SHAFT_WALL => [210, 120, 40],
        FeatureKind::BRIDGE => [110, 170, 215],
        FeatureKind::MASS => [70, 90, 170],
        FeatureKind::STAIR_TOWER | FeatureKind::RAMP => [90, 200, 120],
        FeatureKind::WALKWAY => [130, 225, 130],
        FeatureKind::RAILING => [235, 225, 80],
        FeatureKind::SANCTUARY => [245, 175, 95],
        _ => [190, 190, 190],
    }
}

/// Draws the primitives of one cell. `origin` is the cell's min corner in quanta.
fn draw_prims(canvas: &mut Canvas, prims: &[Primitive], origin: [i64; 3]) {
    for p in prims {
        if matches!(p.shape, Shape::LiftShaft { .. }) {
            continue;
        }
        let bb: QAabb = p.aabb();
        let min = [bb.min.x, bb.min.y, bb.min.z].map(i64::from);
        let max = [bb.max.x, bb.max.y, bb.max.z].map(i64::from);
        let wmin = [0, 1, 2].map(|i| origin[i] + min[i]);
        let wmax = [0, 1, 2].map(|i| origin[i] + max[i]);
        let exact = !matches!(p.shape, Shape::Box { .. });
        let shape = p.shape;
        let test = move |w: [i64; 3]| {
            if !exact {
                return true;
            }
            let local = Vec3::new(
                (w[0] - origin[0]) as f32,
                (w[1] - origin[1]) as f32,
                (w[2] - origin[2]) as f32,
            ) / Q as f32;
            shape.contains(local)
        };
        canvas.fill(wmin, wmax, color_of(p.kind), &test);
    }
}

fn origin_q(cell: CellKey) -> [i64; 3] {
    let size = Level::size_m(cell.level) * Q;
    [
        cell.coords.x * size,
        cell.coords.y * size,
        cell.coords.z * size,
    ]
}

/// Cells of `level` that meet the plane inside the image.
fn cells_in_view(canvas: &Canvas, level: Level) -> Vec<CellKey> {
    let size = level.size_m() * Q;
    let shift = level.size_log2() + 4;
    let a = match canvas.view.axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    let span = (canvas.view.extent_m * Q as f64) as i64;
    let (u_lo, u_hi) = (canvas.u0, canvas.u0 + span);
    let (v_a, v_b) = if canvas.v_up {
        (canvas.v0 - span, canvas.v0)
    } else {
        (canvas.v0, canvas.v0 + span)
    };
    let fl = |q: i64| floor_div_pow2(q, shift);
    let mut cells = Vec::new();
    for u in fl(u_lo)..=fl(u_hi) {
        for v in fl(v_a)..=fl(v_b) {
            let mut c = [0; 3];
            c[a] = fl(canvas.at);
            c[canvas.u_axis] = u;
            c[canvas.v_axis] = v;
            cells.push(CellKey::new(level, I64Vec3::from_array(c)));
        }
    }
    let _ = size;
    cells
}

pub fn render(seed: WorldSeed, view: &View, opts: &Options) -> RgbImage {
    let mut canvas = Canvas::new(view);
    let l1_cells = cells_in_view(&canvas, Level::L1);
    let l1: Vec<(CellKey, CellOutput)> = l1_cells.iter().map(|&c| (c, l1_cell(seed, c))).collect();
    for (cell, out) in &l1 {
        draw_prims(&mut canvas, &out.primitives, origin_q(*cell));
    }
    let l2_cells = if opts.level >= 2 || opts.walkable {
        cells_in_view(&canvas, Level::L2)
    } else {
        Vec::new()
    };
    let cache = L1Cache::default();
    if opts.level >= 2 {
        for &cell in &l2_cells {
            let out = l2_cell_in(seed, &cache, cell);
            draw_prims(&mut canvas, &out.primitives, origin_q(cell));
        }
    }
    if opts.walkable {
        walkable(&mut canvas, seed, opts.level, &l1, &l2_cells, &cache);
    }
    if opts.grid {
        draw_grid(&mut canvas, Level::L2, opts.level >= 2, [70, 70, 70]);
        draw_grid(&mut canvas, Level::L1, true, [255, 255, 255]);
    }
    if opts.ports {
        let mut ports = Vec::new();
        for (_, out) in &l1 {
            ports.extend(out.ports.iter().copied());
        }
        if opts.level >= 2 {
            for &cell in &l2_cells {
                ports.extend(l2_cell_in(seed, &cache, cell).ports);
            }
        }
        draw_ports(&mut canvas, &ports);
    }
    canvas.img
}

fn draw_grid(canvas: &mut Canvas, level: Level, on: bool, color: [u8; 3]) {
    if !on {
        return;
    }
    let size = level.size_m() * Q;
    let span = (canvas.view.extent_m * Q as f64) as i64;
    let first = |lo: i64| lo.div_euclid(size) * size;
    let mut u = first(canvas.u0);
    while u <= canvas.u0 + span {
        canvas.line(false, u, color);
        u += size;
    }
    let (lo, hi) = if canvas.v_up {
        (canvas.v0 - span, canvas.v0)
    } else {
        (canvas.v0, canvas.v0 + span)
    };
    let mut v = first(lo);
    while v <= hi {
        canvas.line(true, v, color);
        v += size;
    }
}

fn draw_ports(canvas: &mut Canvas, ports: &[worldgen::port::Port]) {
    canvas.alpha = 1.0;
    for p in ports {
        let size = p.face.owner.level.size_m() * Q;
        let o = origin_q(p.face.owner);
        let r = p.rect;
        let (u0, v0, u1, v1) = (
            i64::from(r.u0),
            i64::from(r.v0),
            i64::from(r.u1),
            i64::from(r.v1),
        );
        // Face coordinates (u, v) map onto world axes as in `ports.md` §1; the face plane is
        // at the owner's far side.
        let (min, max) = match p.face.axis {
            Axis::X => (
                [o[0] + size, o[1] + v0, o[2] + u0],
                [o[0] + size + 1, o[1] + v1, o[2] + u1],
            ),
            Axis::Y => (
                [o[0] + u0, o[1] + size, o[2] + v0],
                [o[0] + u1, o[1] + size + 1, o[2] + v1],
            ),
            Axis::Z => (
                [o[0] + u0, o[1] + v0, o[2] + size],
                [o[0] + u1, o[1] + v1, o[2] + size + 1],
            ),
        };
        // Thin boxes would vanish at coarse resolution, so widen the face direction to a pixel.
        let a = p.face.axis as usize;
        let pad = canvas.step.ceil() as i64;
        let (mut min, mut max) = (min, max);
        for i in 0..3 {
            if i != a && i != canvas.u_axis && i != canvas.v_axis {
                continue;
            }
            if max[i] - min[i] <= 1 {
                min[i] -= pad / 2;
                max[i] += pad / 2 + 1;
            }
        }
        canvas.fill(min, max, [255, 60, 220], &|_| true);
    }
}

fn walkable(
    canvas: &mut Canvas,
    seed: WorldSeed,
    level: u8,
    l1: &[(CellKey, CellOutput)],
    l2_cells: &[CellKey],
    cache: &L1Cache,
) {
    canvas.alpha = 0.45;
    let mut draw = |plates: &Nav, origin: [i64; 3]| {
        for pl in &plates.plates {
            let (lo, hi) = (pl.y0.min(pl.y1), pl.y0.max(pl.y1) + 1);
            let min = [
                origin[0] + i64::from(pl.rect.x0),
                origin[1] + i64::from(lo),
                origin[2] + i64::from(pl.rect.z0),
            ];
            let max = [
                origin[0] + i64::from(pl.rect.x1),
                origin[1] + i64::from(hi),
                origin[2] + i64::from(pl.rect.z1),
            ];
            let mut min = min;
            let mut max = max;
            // Flat plates are one quantum thick: show them within a metre of the plane in plan
            // views, and a pixel thick in side views.
            let pad = if canvas.view.axis == Axis::Y {
                Q
            } else {
                canvas.step.ceil() as i64
            };
            min[1] -= pad;
            max[1] += pad;
            canvas.fill(min, max, [60, 255, 90], &|_| true);
        }
    };
    if level >= 2 {
        for &cell in l2_cells {
            let cn = CellNav::build(seed, cache, cell);
            draw(&cn.nav, origin_q(cell));
        }
    } else {
        for (cell, out) in l1 {
            draw(&Nav::build(&out.primitives), origin_q(*cell));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(axis: Axis) -> View {
        // 100 m across in 100 pixels, centred on (0, 0): one metre per pixel.
        View {
            axis,
            at_m: 0.0,
            center: (0, 0),
            extent_m: 100.0,
            res: 100,
        }
    }

    fn lit(c: &Canvas, x: u32, y: u32) -> bool {
        c.img.get_pixel(x, y).0 == [255, 0, 0]
    }

    /// Plan view: x grows right and z grows down the image.
    #[test]
    fn plan_view_orientation() {
        let v = view(Axis::Y);
        let mut c = Canvas::new(&v);
        // x in [10, 20) m, z in [30, 40) m, around y = 0.
        c.fill([160, -16, 480], [320, 16, 640], [255, 0, 0], &|_| true);
        assert!(lit(&c, 50 + 10, 50 + 30));
        assert!(lit(&c, 50 + 19, 50 + 39));
        assert!(!lit(&c, 50 + 20, 50 + 30));
        assert!(!lit(&c, 50 + 10, 50 + 29));
        // A box that misses the plane draws nothing.
        let mut d = Canvas::new(&v);
        d.fill([160, 16, 480], [320, 48, 640], [255, 0, 0], &|_| true);
        assert!(!lit(&d, 50 + 10, 50 + 30));
    }

    /// Side view: the vertical axis runs up the image.
    #[test]
    fn side_view_is_upright() {
        let v = view(Axis::Z);
        let mut c = Canvas::new(&v);
        // x in [0, 10) m, y in [20, 30) m, around z = 0.
        c.fill([0, 320, -16], [160, 480, 16], [255, 0, 0], &|_| true);
        assert!(lit(&c, 50, 50 - 21));
        assert!(!lit(&c, 50, 50 + 21));
    }

    #[test]
    fn render_is_deterministic() {
        let v = view(Axis::Y);
        let o = Options {
            level: 2,
            grid: true,
            ports: true,
            walkable: true,
        };
        let a = render(WorldSeed(5), &v, &o);
        let b = render(WorldSeed(5), &v, &o);
        assert_eq!(a.as_raw(), b.as_raw());
    }
}
