//! Voids at km scale: valleys, halls, shafts, rifts, bores, lungs and funnels (VOID-01..13).
//! Spec: `docs/spec/levels.md` §4W.3 and §4W.3.3.
//!
//! A void is data first: whole voids per L0 cell (`voids_of_l0`) and per bore line, clipped to
//! an L1 cell on demand (`voids_of_cell`). Keep-out pieces carry `FeatureKind::VOID`.

// Coordinates are bounded by the L0 cell and a few kilometres of line; draws by their tables.
// The kinds are tables of draws and pieces; splitting them would only scatter one spec section.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::hash::{absorb, hash};
use crate::l1_between::WebPiece;
use crate::l1_frame::{Beam, BeamKind, End, Section, dir_info, span_home};
use crate::orient::{DIRS, Frame, LocalShape, beam_cylinder};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape};
use crate::regime::regime;
use crate::regime_web::StackGeometry;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Stack, Stratum, stack};

pub const VOID_VALLEY: &str = "VOID-01";
pub const VOID_HALL: &str = "VOID-02";
pub const VOID_VARIANT: &str = "VOID-03";
pub const VOID_HYPOSTYLE: &str = "VOID-04";
pub const VOID_VAULT: &str = "VOID-05";
pub const VOID_CRUSH: &str = "VOID-06";
pub const VOID_STEPPED: &str = "VOID-07";
pub const VOID_DROWNED: &str = "VOID-08";
pub const VOID_SHAFT: &str = "VOID-09";
pub const VOID_RIFT: &str = "VOID-10";
pub const VOID_BORE: &str = "VOID-11";
pub const VOID_LUNGS: &str = "VOID-12";
pub const VOID_FUNNEL: &str = "VOID-13";

const L1_M: i64 = 4096;
const L0_M: i64 = 32_768;
const Q: i64 = Q_PER_M as i64;
/// Bore lattice pitch and segment length (spec §4W.3.3).
const BORE_PITCH: i64 = 16_384;
const BORE_SEG: i64 = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoidKind {
    Valley,
    Hall,
    Shaft,
    Rift,
    Bore,
    Funnel,
}

/// A convex piece in world metres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shp {
    Box {
        lo: I64Vec3,
        hi: I64Vec3,
    },
    /// `base` is the centre of the cap at the axis minimum.
    Cyl {
        axis: Axis,
        base: I64Vec3,
        len: i64,
        r: i64,
    },
    /// A box in `Frame::table(yaw, tilt)` at `origin`: local `x` across, `y` along the lean,
    /// `z` along the line.
    Rift {
        yaw: u8,
        tilt: u8,
        origin: I64Vec3,
        x: (i64, i64),
        y: i64,
        z: i64,
    },
    /// A cylinder along table direction `d` from `start` (world quanta), `len` quanta long.
    Line {
        d: [i8; 3],
        start: [i64; 3],
        len: i64,
        r: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Void {
    pub tag: &'static str,
    /// Variant IDs that go with `tag`, for the census.
    pub also: Vec<&'static str>,
    pub kind: VoidKind,
    pub key: u64,
    /// The region kept free.
    pub keep_out: Vec<Shp>,
    /// What the void leaves standing (hypostyle columns).
    pub solids: Vec<Shp>,
    /// VOID-08: the atmosphere is fogged inside.
    pub fog: bool,
}

impl Void {
    pub fn tags(&self) -> Vec<&'static str> {
        let mut t = vec![self.tag];
        t.extend(self.also.iter().copied());
        t
    }

    /// Conservative world bounds of the keep-out pieces.
    pub fn bounds(&self) -> (I64Vec3, I64Vec3) {
        let mut lo = I64Vec3::splat(i64::MAX);
        let mut hi = I64Vec3::splat(i64::MIN);
        for s in self.keep_out.iter().chain(&self.solids) {
            let (a, b) = shp_bounds(s);
            lo = lo.min(a);
            hi = hi.max(b);
        }
        (lo, hi)
    }
}

fn shp_bounds(s: &Shp) -> (I64Vec3, I64Vec3) {
    match *s {
        Shp::Box { lo, hi } => (lo, hi),
        Shp::Cyl { axis, base, len, r } => {
            let mut lo = base - I64Vec3::splat(r);
            let mut hi = base + I64Vec3::splat(r);
            match axis {
                Axis::X => {
                    lo.x = base.x;
                    hi.x = base.x + len;
                }
                Axis::Y => {
                    lo.y = base.y;
                    hi.y = base.y + len;
                }
                Axis::Z => {
                    lo.z = base.z;
                    hi.z = base.z + len;
                }
            }
            (lo, hi)
        }
        Shp::Rift {
            origin, x, y, z, ..
        } => {
            let reach = x.1.abs().max(x.0.abs()) + y;
            (
                origin - I64Vec3::splat(reach + z),
                origin + I64Vec3::splat(reach + z),
            )
        }
        Shp::Line { start, len, r, .. } => {
            let s = I64Vec3::new(start[0] / Q, start[1] / Q, start[2] / Q);
            let l = len / Q + r + 1;
            (s - I64Vec3::splat(l), s + I64Vec3::splat(l))
        }
    }
}

fn comp(v: I64Vec3, axis: Axis) -> i64 {
    match axis {
        Axis::X => v.x,
        Axis::Y => v.y,
        Axis::Z => v.z,
    }
}

fn crust_key(seed: WorldSeed, home: I64Vec3, k: u16) -> u64 {
    absorb(hash(seed.0, Level::L0, home, salt::VOID_KEY), u64::from(k))
}

/// One crust of the stack in world metres, clamped to the inner zone of the L0 cell.
#[derive(Debug, Clone, Copy)]
struct Crust {
    key: u64,
    lo: i64,
    hi: i64,
}

fn crusts(seed: WorldSeed, home: I64Vec3, st: &Stack) -> Vec<Crust> {
    let base = home.y * L0_M;
    let (zone_lo, zone_hi) = (base + L1_M, base + L0_M - L1_M);
    st.segments
        .iter()
        .filter_map(|s| {
            let Stratum::Crust(k) = s.stratum else {
                return None;
            };
            let lo = (base + i64::from(s.start) * 64).max(zone_lo);
            let hi = (base + i64::from(s.end) * 64).min(zone_hi);
            (hi - lo >= 64).then_some(Crust {
                key: crust_key(seed, home, k),
                lo,
                hi,
            })
        })
        .collect()
}

fn pick_crust(rng: &mut Rng, cs: &[Crust]) -> Option<Crust> {
    let w: Vec<u32> = cs.iter().map(|c| (c.hi - c.lo) as u32).collect();
    rng.pick_weighted(&w).map(|i| cs[i])
}

fn site_rng(seed: WorldSeed, a: i64, b: i64, key: u64, salt: u64) -> Rng {
    Rng::new(hash(
        seed.0,
        Level::L1,
        I64Vec3::new(a, b, key as i64),
        salt,
    ))
}

/// Void chance per site in thousandths, from the regime's crust fill (spec §4W.3.3).
fn void_chance(fill_crust: u8) -> u64 {
    (1000 * (255 - u64::from(fill_crust)) * 3 / 255).min(600)
}

/// Every area void and line void (valleys, rifts) of L0 cell `home`, in a fixed order. Empty
/// for stacks other than Flat and Curved (PW.2.6b).
pub fn voids_of_l0(seed: WorldSeed, home: I64Vec3) -> Vec<Void> {
    let web = regime(seed, home).web;
    if !matches!(web.stack, StackGeometry::Flat | StackGeometry::Curved) {
        return Vec::new();
    }
    let st = stack(seed, home);
    let cs = crusts(seed, home, &st);
    if cs.is_empty() {
        return Vec::new();
    }
    let origin = home * L0_M;
    let mut out = Vec::new();
    for axis in [Axis::X, Axis::Z] {
        for lane in 1..=6i64 {
            line_void(seed, origin, &cs, axis, lane, &mut out);
        }
    }
    let lines = out.clone();
    let chance = void_chance(web.fill_crust);
    for cx in 1..=6i64 {
        for cz in 1..=6i64 {
            for c in &cs {
                if let Some(v) = area_void(seed, origin, c, cx, cz, chance) {
                    // Rule 3: area voids keep 512 m from line voids of their crust.
                    let (lo, hi) = v.bounds();
                    let clash = lines.iter().any(|l| {
                        let (a, b) = l.bounds();
                        let (y0, y1) = (c.lo, c.hi);
                        a.x < hi.x + 512
                            && b.x > lo.x - 512
                            && a.z < hi.z + 512
                            && b.z > lo.z - 512
                            && a.y < y1
                            && b.y > y0
                    });
                    if !clash {
                        out.push(v);
                    }
                }
            }
        }
    }
    out
}

fn bx(lo: [i64; 3], hi: [i64; 3]) -> Shp {
    Shp::Box {
        lo: I64Vec3::from_array(lo),
        hi: I64Vec3::from_array(hi),
    }
}

/// A valley or a rift on one lane (spec §4W.3.3).
fn line_void(
    seed: WorldSeed,
    origin: I64Vec3,
    cs: &[Crust],
    axis: Axis,
    lane: i64,
    out: &mut Vec<Void>,
) {
    let ax = i64::from(axis == Axis::Z);
    let mut rng = site_rng(seed, ax, lane, cs[0].key, salt::VOID_LINE);
    let crust = pick_crust(&mut rng, cs).unwrap_or(cs[0]);
    let mut rng = site_rng(seed, ax, lane, crust.key, salt::VOID_LINE);
    if rng.below(1000) >= 300 {
        return;
    }
    let valley = rng.below(2) != 0;
    // Across the line (`s`) and along it (`a`), in world metres.
    let across0 = comp(origin, if axis == Axis::X { Axis::Z } else { Axis::X }) + lane * L1_M;
    let along0 = comp(origin, axis);
    let key = absorb(crust.key, 0x100 + (ax as u64) * 16 + lane as u64);
    let mk = |lo: [i64; 3], hi: [i64; 3]| {
        // `lo`/`hi` are given as (along, across, y).
        let (l, h) = if axis == Axis::X {
            (
                I64Vec3::new(lo[0], lo[2], lo[1]),
                I64Vec3::new(hi[0], hi[2], hi[1]),
            )
        } else {
            (
                I64Vec3::new(lo[1], lo[2], lo[0]),
                I64Vec3::new(hi[1], hi[2], hi[0]),
            )
        };
        Shp::Box { lo: l, hi: h }
    };
    if valley {
        let w = 100 * i64::from(rng.range_i32(3, 21));
        let len = (1000 * i64::from(rng.range_i32(2, 25))).min(24_576);
        let a0 = along0 + L1_M + 64 * rng.below(((L0_M - 2 * L1_M - len) / 64 + 1) as u64) as i64;
        let centre = across0
            + L1_M / 2
            + 16 * i64::from(rng.range_i32(
                -((L1_M - w) / 32 - 8) as i32,
                ((L1_M - w) / 32 - 8) as i32 + 1,
            ));
        let (y0, y1) = (crust.lo, crust.hi);
        let mut pieces = vec![mk(
            [a0 + w / 2, centre - w / 2, y0],
            [a0 + len - w / 2, centre + w / 2, y1],
        )];
        // Rounded ends: four tapering boxes at each end.
        for i in 0..4i64 {
            let half = w * (7 - 2 * i) / 16;
            let (s0, s1) = (a0 + w / 2 - (i + 1) * w / 8, a0 + w / 2 - i * w / 8);
            let (e0, e1) = (
                a0 + len - w / 2 + i * w / 8,
                a0 + len - w / 2 + (i + 1) * w / 8,
            );
            pieces.push(mk([s0, centre - half, y0], [s1, centre + half, y1]));
            pieces.push(mk([e0, centre - half, y0], [e1, centre + half, y1]));
        }
        out.push(Void {
            tag: VOID_VALLEY,
            also: Vec::new(),
            kind: VoidKind::Valley,
            key,
            keep_out: pieces,
            solids: Vec::new(),
            fog: false,
        });
    } else {
        let w = 20 + 4 * i64::from(rng.below(16) as i32);
        let len = 1000 * i64::from(rng.range_i32(1, 7));
        let tilt = rng.below(7) as u8;
        let lean = rng.below(2) == 0;
        let step = rng.below(3) == 0;
        let a0 = along0 + L1_M + 64 * rng.below(((L0_M - 2 * L1_M - len) / 64 + 1) as u64) as i64;
        let centre = across0 + L1_M / 2 + 16 * i64::from(rng.range_i32(-30, 31));
        let thick = crust.hi - crust.lo;
        let slope = f64::from(crate::orient::TILT_SLOPE[usize::from(tilt)]);
        let m = (64.0 * 64.0 + slope * slope).sqrt();
        let y_len = (thick as f64 * m / 64.0).ceil() as i64;
        let yaw = match (axis, lean) {
            (Axis::Z, true) => 0,
            (Axis::Z, false) => 32,
            (_, true) => 16,
            (_, false) => 48,
        };
        let halves: &[(i64, i64, i64)] = if step {
            &[(0, len / 2, 0), (len / 2, len, w * 3 / 2)]
        } else {
            &[(0, len, 0)]
        };
        let mut pieces = Vec::new();
        for &(z0, z1, off) in halves {
            // `origin` sits at the line's end that the frame's `z` runs away from.
            let (ox, oz, ox_off) = match (axis, yaw) {
                (Axis::Z, 0) => (centre + off, a0 + z0, 0),
                (Axis::Z, _) => (centre + off, a0 + z1, 0),
                (_, 16) => (a0 + z1, centre + off, 0),
                _ => (a0 + z0, centre + off, 0),
            };
            let _ = ox_off;
            let origin = I64Vec3::new(ox, crust.lo, oz);
            pieces.push(Shp::Rift {
                yaw,
                tilt,
                origin,
                x: (-w / 2, w / 2),
                y: y_len,
                z: z1 - z0,
            });
        }
        out.push(Void {
            tag: VOID_RIFT,
            also: Vec::new(),
            kind: VoidKind::Rift,
            key,
            keep_out: pieces,
            solids: Vec::new(),
            fog: false,
        });
    }
}

/// A hall, shaft or funnel at one site (spec §4W.3.3).
fn area_void(
    seed: WorldSeed,
    origin: I64Vec3,
    c: &Crust,
    cx: i64,
    cz: i64,
    chance: u64,
) -> Option<Void> {
    let mut rng = site_rng(seed, cx, cz, c.key, salt::VOID_SITE);
    let e = rng.below(1000);
    let kind = rng.pick_weighted(&[55, 25, 20]).unwrap_or(0);
    let variant = rng
        .pick_weighted(&[20, 15, 15, 10, 15, 15, 10])
        .unwrap_or(0);
    let (col_x, col_z) = (origin.x + cx * L1_M, origin.z + cz * L1_M);
    // Size draws in a fixed order, used by whichever kind was picked.
    let d_a = i64::from(rng.range_i32(3, 16));
    let d_b = i64::from(rng.range_i32(3, 16));
    let d_h = i64::from(rng.range_i32(5, 31));
    let d_s = i64::from(rng.range_i32(4, 33));
    let d_r = i64::from(rng.range_i32(2, 16));
    let round = rng.below(2) == 0;
    let n_rings = 8 + rng.below(9) as i64;
    let pos_a = rng.next_u64();
    let pos_b = rng.next_u64();
    let pos_y = rng.next_u64();
    if e >= chance {
        return None;
    }
    let place = |size: i64, r: u64| -> i64 {
        // A multiple of 16 m that keeps the footprint 256 m inside the L1 column.
        256 + 16 * (r % (((L1_M - 512 - size) / 16 + 1).max(1) as u64)) as i64
    };
    let key = absorb(c.key, (cx * 8 + cz) as u64);
    let thick = c.hi - c.lo;
    match kind {
        0 => {
            let (mut fx, mut fz) = (100 * d_a, 100 * d_b);
            let mut h = 4 * d_h;
            let (tag, mut also, mut fog) = (VOID_HALL, Vec::new(), false);
            let mut tag = tag;
            match variant {
                0 => {}
                1 => {
                    tag = VOID_VARIANT;
                    also.push(VOID_HYPOSTYLE);
                }
                2 | 3 => {
                    tag = VOID_VARIANT;
                    also.push(VOID_VAULT);
                }
                4 => {
                    tag = VOID_VARIANT;
                    also.push(VOID_CRUSH);
                    fx = fx.max(800);
                    fz = fz.max(800);
                    h = 20;
                }
                5 => {
                    tag = VOID_VARIANT;
                    also.push(VOID_STEPPED);
                }
                _ => {
                    tag = VOID_VARIANT;
                    also.push(VOID_DROWNED);
                    fog = true;
                }
            }
            let room = thick - 32;
            if room < h + 8 {
                return None;
            }
            let (x0, z0) = (col_x + place(fx, pos_a), col_z + place(fz, pos_b));
            let y0 = c.lo + 16 + 16 * (pos_y % (((room - h) / 16 + 1).max(1) as u64)) as i64;
            let mut keep = vec![bx([x0, y0, z0], [x0 + fx, y0 + h, z0 + fz])];
            let mut solids = Vec::new();
            match variant {
                1 => {
                    let pitch = 48 + 16 * (pos_a % 4) as i64;
                    let r = 4 + (pos_b % 7) as i64;
                    let mut x = x0 + pitch / 2;
                    while x < x0 + fx {
                        let mut z = z0 + pitch / 2;
                        while z < z0 + fz {
                            solids.push(Shp::Cyl {
                                axis: Axis::Y,
                                base: I64Vec3::new(x, y0, z),
                                len: h,
                                r,
                            });
                            z += pitch;
                        }
                        x += pitch;
                    }
                }
                2 => {
                    for i in 1..=3i64 {
                        let (ix, iz) = (fx * i / 8, fz * i / 8);
                        // Tiers rise no higher than 16 m under the upper surface.
                        let top = (y0 + h + h / 3 * i).min(c.hi - 16);
                        if top > y0 + h {
                            keep.push(bx(
                                [x0 + ix, y0 + h, z0 + iz],
                                [x0 + fx - ix, top, z0 + fz - iz],
                            ));
                        }
                    }
                }
                3 => {
                    let top = (y0 + 3 * h).min(c.hi - 16).max(y0 + h);
                    keep.push(bx([x0 + fx / 3, y0, z0], [x0 + 2 * fx / 3, top, z0 + fz]));
                }
                5 => {
                    keep.clear();
                    let n = 3 + (pos_a % 3) as i64;
                    for i in 0..n {
                        let floor = (y0 + 4 * i).min(c.hi - 16 - h);
                        keep.push(bx(
                            [x0 + fx * i / n, floor, z0],
                            [x0 + fx * (i + 1) / n, floor + h, z0 + fz],
                        ));
                    }
                }
                _ => {}
            }
            Some(Void {
                tag,
                also,
                kind: VoidKind::Hall,
                key,
                keep_out: keep,
                solids,
                fog,
            })
        }
        1 => {
            let s = 16 * d_s;
            let (x0, z0) = (col_x + place(s, pos_a), col_z + place(s, pos_b));
            let piece = if round {
                Shp::Cyl {
                    axis: Axis::Y,
                    base: I64Vec3::new(x0 + s / 2, c.lo, z0 + s / 2),
                    len: thick,
                    r: s / 2,
                }
            } else {
                bx([x0, c.lo, z0], [x0 + s, c.hi, z0 + s])
            };
            Some(Void {
                tag: VOID_SHAFT,
                also: Vec::new(),
                kind: VoidKind::Shaft,
                key,
                keep_out: vec![piece],
                solids: Vec::new(),
                fog: false,
            })
        }
        _ => {
            let top = 100 * d_r;
            let (x0, z0) = (col_x + place(2 * top, pos_a), col_z + place(2 * top, pos_b));
            let (cxm, czm) = (x0 + top, z0 + top);
            let ring = thick / n_rings;
            let mut keep = Vec::new();
            for i in 0..n_rings {
                // Widest at the top, narrowing downwards to `top / n`.
                let r = (top * (n_rings - i) / n_rings).max(top / n_rings).max(8);
                keep.push(Shp::Cyl {
                    axis: Axis::Y,
                    base: I64Vec3::new(cxm, c.hi - (i + 1) * ring, czm),
                    len: ring,
                    r,
                });
            }
            Some(Void {
                tag: VOID_FUNNEL,
                also: Vec::new(),
                kind: VoidKind::Funnel,
                key,
                keep_out: keep,
                solids: Vec::new(),
                fog: false,
            })
        }
    }
}

/// Bores and their Lungs that cross L1 cell `l1` (VOID-11, VOID-12).
pub fn bores_of_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<Void> {
    let origin = l1 * L1_M;
    let mut out = Vec::new();
    for (dir, d) in DIRS.iter().enumerate() {
        if d[1] != 0 {
            continue;
        }
        let info = dir_info(dir);
        let (a0, a1) = (comp(origin, info.a), comp(origin, info.a) + L1_M);
        let seg = a0.div_euclid(BORE_SEG);
        let range = |axis: Axis, d: i64| {
            let (x, y) = (d * a0 / info.da, d * a1 / info.da);
            let (lo, hi) = (x.min(y) - 1, x.max(y) + 1);
            let c0 = comp(origin, axis);
            (c0 - 100 - hi, c0 + L1_M + 100 - lo)
        };
        let (ulo, uhi) = range(info.u, info.du);
        let (vlo, vhi) = range(info.v, info.dv);
        for p in (ulo - BORE_PITCH + 1).div_euclid(BORE_PITCH)..=uhi.div_euclid(BORE_PITCH) {
            for q in (vlo - BORE_PITCH + 1).div_euclid(BORE_PITCH)..=vhi.div_euclid(BORE_PITCH) {
                let line = hash(
                    seed.0,
                    Level::L1,
                    I64Vec3::new(p, q, dir as i64),
                    salt::VOID_BORE,
                );
                let mut rng = Rng::new(line);
                if rng.below(1000) >= 125 {
                    continue;
                }
                let u0 = p * BORE_PITCH + 16 * rng.below((BORE_PITCH / 16) as u64) as i64;
                let v0 = q * BORE_PITCH + 16 * rng.below((BORE_PITCH / 16) as u64) as i64;
                let mut srng = Rng::new(absorb(line, seg as u64));
                if srng.below(2) == 0 {
                    continue;
                }
                let len = 1000 * i64::from(srng.range_i32(12, 49));
                let off = 4096 * srng.below(((BORE_SEG - len) / 4096 + 1) as u64) as i64;
                let r = 6 + srng.below(15) as i64;
                let lungs = srng.below(3) == 0;
                let pitch = 1000 * (2 + srng.below(3) as i64);
                let (a_from, a_to) = (seg * BORE_SEG + off, seg * BORE_SEG + off + len);
                if a_to <= a0 || a_from >= a1 {
                    continue;
                }
                let beam = Beam {
                    dir,
                    p,
                    q,
                    u0,
                    v0,
                    span: seg,
                    a_from,
                    a_to,
                    width: 2 * r,
                    section: Section::Round,
                    kind: BeamKind::Girder,
                    ends: [End::Stub, End::Stub],
                    home: span_home(&info, u0, v0, seg),
                    detail: 0,
                };
                let d = beam.direction();
                let norm = {
                    let v = d.map(f64::from);
                    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
                };
                let length = (((a_to - a_from) * Q) as f64 * norm / info.da as f64).round() as i64;
                let key = absorb(line, seg as u64);
                let mut v = Void {
                    tag: VOID_BORE,
                    also: Vec::new(),
                    kind: VoidKind::Bore,
                    key,
                    keep_out: vec![Shp::Line {
                        d,
                        start: beam.point_q(a_from),
                        len: length,
                        r,
                    }],
                    solids: Vec::new(),
                    fog: false,
                };
                if lungs {
                    v.also.push(VOID_LUNGS);
                    let mut lr = Rng::new(absorb(key, 0x1005));
                    let fp = 200 + 8 * lr.below(51) as i64;
                    let h = 4 * i64::from(lr.range_i32(5, 31));
                    let mut at = a_from + pitch;
                    while at < a_to {
                        let c = beam.point_q(at);
                        let c = I64Vec3::new(c[0] / Q, c[1] / Q, c[2] / Q);
                        v.keep_out.push(Shp::Box {
                            lo: I64Vec3::new(c.x - fp / 2, c.y - h / 2, c.z - fp / 2),
                            hi: I64Vec3::new(c.x + fp / 2, c.y + h / 2, c.z + fp / 2),
                        });
                        at += pitch;
                    }
                }
                out.push(v);
            }
        }
    }
    out
}

/// A piece of a void that reaches L1 cell `l1`, in cell-local space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoidPiece {
    pub piece: WebPiece,
    /// Standing matter inside the void (hypostyle columns), not keep-out.
    pub solid: bool,
    pub fog: bool,
}

/// Everything of the voids of L1 cell `l1`: area and line voids of its L0 cell, and bores.
pub fn voids_of_cell(seed: WorldSeed, l1: I64Vec3) -> Vec<VoidPiece> {
    let home = l1.div_euclid(I64Vec3::splat(8));
    let cell_lo = l1 * L1_M;
    let cell_hi = cell_lo + I64Vec3::splat(L1_M);
    let mut out = Vec::new();
    let mut emit = |v: &Void| {
        let (lo, hi) = v.bounds();
        if lo.cmpge(cell_hi).any() || hi.cmple(cell_lo).any() {
            return;
        }
        for (list, solid) in [(&v.keep_out, false), (&v.solids, true)] {
            for s in list {
                if let Some(shape) = to_cell(s, cell_lo) {
                    out.push(VoidPiece {
                        piece: WebPiece {
                            tag: v.tag,
                            prim: Primitive {
                                shape,
                                material: MaterialId::CONCRETE,
                                kind: if solid {
                                    FeatureKind::TETHER
                                } else {
                                    FeatureKind::VOID
                                },
                                flags: if solid {
                                    PrimFlags::NONE
                                } else {
                                    PrimFlags::NO_COLLIDE
                                },
                            },
                        },
                        solid,
                        fog: v.fog,
                    });
                }
            }
        }
    };
    for v in voids_of_l0(seed, home) {
        emit(&v);
    }
    for v in bores_of_cell(seed, l1) {
        emit(&v);
    }
    out
}

fn clip() -> QAabb {
    let e = (L1_M * Q) as i32;
    QAabb::new(QVec3::new(0, 0, 0), QVec3::new(e, e, e))
}

/// A world-metre shape as a cell-local piece, clipped to the cell; `None` if it misses.
fn to_cell(s: &Shp, cell_lo: I64Vec3) -> Option<Shape> {
    let q = |v: i64, axis: Axis| ((v - comp(cell_lo, axis)) * Q) as i32;
    let qv = |v: I64Vec3| QVec3::new(q(v.x, Axis::X), q(v.y, Axis::Y), q(v.z, Axis::Z));
    let shape = match *s {
        Shp::Box { lo, hi } => {
            let (mut min, mut max) = ([0i32; 3], [0i32; 3]);
            for (i, axis) in Axis::ALL.into_iter().enumerate() {
                let l = comp(lo, axis).max(comp(cell_lo, axis));
                let h = comp(hi, axis).min(comp(cell_lo, axis) + L1_M);
                if l >= h {
                    return None;
                }
                min[i] = q(l, axis);
                max[i] = q(h, axis);
            }
            return Some(Shape::Box {
                aabb: QAabb::new(
                    QVec3::new(min[0], min[1], min[2]),
                    QVec3::new(max[0], max[1], max[2]),
                ),
            });
        }
        Shp::Cyl { axis, base, len, r } => Shape::oriented(
            Frame::Id,
            QVec3::new(0, 0, 0),
            LocalShape::Cylinder {
                axis,
                base: qv(base),
                length: (len * Q) as i32,
                radius: (r * Q) as i32,
            },
            clip(),
        ),
        Shp::Rift {
            yaw,
            tilt,
            origin,
            x,
            y,
            z,
        } => {
            let aabb = QAabb::new(
                QVec3::new((x.0 * Q) as i32, 0, 0),
                QVec3::new((x.1 * Q) as i32, (y * Q) as i32, (z * Q) as i32),
            );
            Shape::oriented(
                Frame::table(yaw, tilt),
                qv(origin),
                LocalShape::Box { aabb },
                clip(),
            )
        }
        Shp::Line { d, start, len, r } => {
            let o = |a: usize, axis: Axis| (start[a] - comp(cell_lo, axis) * Q) as i32;
            let st = QVec3::new(o(0, Axis::X), o(1, Axis::Y), o(2, Axis::Z));
            beam_cylinder(d, st, len as i32, (r * Q) as i32, clip())
        }
    };
    let b = shape.aabb();
    let c = clip();
    let outside = b.max.x <= c.min.x
        || b.min.x >= c.max.x
        || b.max.y <= c.min.y
        || b.min.y >= c.max.y
        || b.max.z <= c.min.z
        || b.min.z >= c.max.z;
    (!outside).then_some(shape)
}

/// The kinds of void present in the voids of an L0 cell, for the census.
pub fn census_tags(v: &Void) -> Vec<&'static str> {
    v.tags()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn homes() -> impl Iterator<Item = I64Vec3> {
        (-8..8).flat_map(|x| (-8..8).map(move |z| I64Vec3::new(x, 0, z)))
    }

    /// Every kind and every variant tag occurs somewhere.
    #[test]
    fn every_void_tag_occurs() {
        let mut seen = std::collections::HashSet::new();
        let mut fog = false;
        for seed in [1u64, 2, 3] {
            for h in homes() {
                for v in voids_of_l0(WorldSeed(seed), h) {
                    seen.extend(v.tags());
                    fog |= v.fog;
                }
            }
            for k in 0..300 {
                for v in bores_of_cell(WorldSeed(seed), I64Vec3::new(k, 3, -k)) {
                    seen.extend(v.tags());
                }
            }
        }
        for tag in [
            VOID_VALLEY,
            VOID_HALL,
            VOID_VARIANT,
            VOID_HYPOSTYLE,
            VOID_VAULT,
            VOID_CRUSH,
            VOID_STEPPED,
            VOID_DROWNED,
            VOID_SHAFT,
            VOID_RIFT,
            VOID_BORE,
            VOID_LUNGS,
            VOID_FUNNEL,
        ] {
            assert!(seen.contains(tag), "{tag} missing: {seen:?}");
        }
        assert!(fog);
    }

    /// Rule 1: area voids and line voids stay in the inner zone of their L0 cell and never
    /// touch an L0 face (bores are the exception).
    #[test]
    fn voids_keep_off_the_l0_faces() {
        for seed in [1u64, 5] {
            for h in homes() {
                let origin = h * L0_M;
                for v in voids_of_l0(WorldSeed(seed), h) {
                    let (lo, hi) = v.bounds();
                    if v.kind == VoidKind::Rift {
                        continue;
                    }
                    for axis in Axis::ALL {
                        let (a, b) = (
                            comp(lo, axis) - comp(origin, axis),
                            comp(hi, axis) - comp(origin, axis),
                        );
                        assert!(
                            a >= L1_M - 1 && b <= L0_M - L1_M + 1,
                            "{:?} {a} {b} on {axis:?}",
                            v.tag
                        );
                    }
                }
            }
        }
    }

    /// Rule 3: area voids of different keys in one crust are at least 512 m apart.
    #[test]
    fn area_voids_are_separated() {
        let seed = WorldSeed(1);
        let mut pairs = 0;
        for h in homes().take(40) {
            let vs: Vec<_> = voids_of_l0(seed, h)
                .into_iter()
                .filter(|v| matches!(v.kind, VoidKind::Hall | VoidKind::Shaft | VoidKind::Funnel))
                .collect();
            for (i, a) in vs.iter().enumerate() {
                for b in &vs[i + 1..] {
                    let ((al, ah), (bl, bh)) = (a.bounds(), b.bounds());
                    if al.y < bh.y && bl.y < ah.y {
                        let gap_x = (al.x - bh.x).max(bl.x - ah.x);
                        let gap_z = (al.z - bh.z).max(bl.z - ah.z);
                        assert!(
                            gap_x.max(gap_z) >= 512,
                            "{:?} and {:?} are {gap_x}/{gap_z} apart",
                            a.tag,
                            b.tag
                        );
                        pairs += 1;
                    }
                }
            }
        }
        assert!(pairs > 20, "{pairs} pairs");
    }

    /// Pieces stay inside their cell and are deterministic.
    #[test]
    fn cell_pieces_are_clipped() {
        let e = (L1_M * Q) as i32;
        let seed = WorldSeed(2);
        let mut n = 0;
        for h in homes().take(30) {
            for c in [(1, 3, 1), (3, 2, 4), (6, 5, 6), (0, 4, 0)] {
                let l1 = h * 8 + I64Vec3::new(c.0, c.1, c.2);
                let a = voids_of_cell(seed, l1);
                assert_eq!(a, voids_of_cell(seed, l1));
                for p in &a {
                    let b = p.piece.prim.aabb();
                    assert!(
                        b.min.x >= 0
                            && b.min.y >= 0
                            && b.min.z >= 0
                            && b.max.x <= e
                            && b.max.y <= e
                            && b.max.z <= e,
                        "{b:?}"
                    );
                    n += 1;
                }
            }
        }
        assert!(n > 50, "{n} pieces");
    }

    /// The void share of crusts stays at or under 8 % (spec §4W.3.1 rule 5).
    #[test]
    fn void_share_is_small() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        let mut rng = Rng::new(5);
        let (mut crust, mut void, mut worst) = (0u32, 0u32, 0.0f64);
        for h in homes().take(48) {
            let web = regime(seed, h).web;
            if !matches!(web.stack, StackGeometry::Flat | StackGeometry::Curved) {
                continue;
            }
            let st = stack(seed, h);
            let (mut c_h, mut v_h) = (0u32, 0u32);
            let cells: Vec<Vec<VoidPiece>> = (0..216)
                .map(|i| {
                    voids_of_cell(
                        seed,
                        h * 8 + I64Vec3::new(1 + i % 6, 1 + (i / 6) % 6, 1 + i / 36),
                    )
                })
                .filter(|v| !v.is_empty())
                .collect();
            let _ = cells;
            for _ in 0..3000 {
                let l = I64Vec3::new(
                    1 + rng.below(6) as i64,
                    1 + rng.below(6) as i64,
                    1 + rng.below(6) as i64,
                );
                let p = Vec3::new(
                    rng.next_f32() * 4096.0,
                    rng.next_f32() * 4096.0,
                    rng.next_f32() * 4096.0,
                );
                let w = l * L1_M + I64Vec3::new(p.x as i64, p.y as i64, p.z as i64);
                if !matches!(st.segment_at(w.y / 64).stratum, Stratum::Crust(_)) {
                    continue;
                }
                c_h += 1;
                let pieces = voids_of_cell(seed, h * 8 + l);
                if pieces
                    .iter()
                    .any(|v| !v.solid && v.piece.prim.shape.contains(p))
                {
                    v_h += 1;
                }
            }
            crust += c_h;
            void += v_h;
            if c_h > 200 {
                worst = worst.max(f64::from(v_h) / f64::from(c_h));
            }
        }
        let share = f64::from(void) / f64::from(crust.max(1));
        assert!(crust > 2000, "{crust} crust samples");
        assert!(
            share <= 0.08 && worst <= 0.20,
            "void share {share:.3}, worst regime {worst:.3}"
        );
    }

    /// A bore is the same on both sides of an L0 face: the cells beside the face derive it alike.
    #[test]
    fn bores_agree_across_l0_faces() {
        let seed = WorldSeed(3);
        let mut crossed = 0;
        for k in 0..400 {
            let c = I64Vec3::new(8 * (k % 7 - 3) - 1, 3 + k % 5, 8 * (k / 7 % 7 - 3) + 2);
            for axis in [Axis::X, Axis::Z] {
                let mut step = I64Vec3::ZERO;
                match axis {
                    Axis::X => step.x = 1,
                    _ => step.z = 1,
                }
                let c = if axis == Axis::X {
                    c
                } else {
                    I64Vec3::new(c.z, c.y, c.x)
                };
                let (a, b) = (bores_of_cell(seed, c), bores_of_cell(seed, c + step));
                for v in &a {
                    if b.iter().any(|w| w.key == v.key) {
                        crossed += 1;
                    }
                }
            }
        }
        assert!(crossed > 0, "no bore crossed a checked face");
    }
}
