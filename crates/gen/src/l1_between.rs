//! What fills a Between: tethers, drips, spires, kisses, islands, tubes, nets, dead machines and
//! the window speckle on the underside of the crust above (STR-03..11).
//! Spec: `docs/spec/levels.md` §4W.6.1.
//!
//! Every feature is keyed by its Between run and a lattice site, never by the cell, so each L1
//! cell that sees a feature derives it identically and clips its own piece (invariant 4).

// Coordinates are bounded by the cell and its sites (a few kilometres, 16 quanta to the metre),
// and the draws below are bounded by their tables.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::hash::{absorb, hash};
use crate::orient::TILT_SLOPE;
use crate::orient::{Frame, LocalShape};
use crate::prim::{
    Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q_PER_M, QAabb, QVec3, Shape,
};
use crate::regime::regime;
use crate::regime_web::{StackGeometry, WebFields};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{BetweenKind, Stack, Stratum, stack, tilt_shift};

/// A piece of Between content with the registry ID it counts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WebPiece {
    pub tag: &'static str,
    pub prim: Primitive,
}

pub const STR_TETHER: &str = "STR-03";
pub const STR_DRIP: &str = "STR-04";
pub const STR_SPIRE: &str = "STR-05";
pub const STR_KISS: &str = "STR-06";
pub const STR_ISLAND: &str = "STR-07";
pub const STR_TUBE: &str = "STR-08";
pub const STR_NET: &str = "STR-09";
pub const STR_MACHINE: &str = "STR-10";
pub const STR_LIGHTS: &str = "STR-11";

pub(crate) const L1_M: i64 = 4096;
pub(crate) const L0_M: i64 = 32_768;
pub(crate) const Q: i64 = Q_PER_M as i64;

/// One stretch of Between along the stack axis, in world metres. `lo` is the surface of the
/// crust below, `hi` the underside of the crust above.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run {
    pub(crate) key: u64,
    pub(crate) kind: BetweenKind,
    pub(crate) flooded: bool,
    /// `fill_between` of the cell that owns the key: both cells of a seam must draw alike.
    pub(crate) fb: i64,
    pub(crate) lo: i64,
    pub(crate) hi: i64,
}

impl Run {
    pub(crate) fn depth(&self) -> i64 {
        self.hi - self.lo
    }
}

/// World axes of the stack coordinates `(s, t, a)`: two across the stack and the axis along it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AxisMap {
    pub(crate) s: Axis,
    pub(crate) t: Axis,
    pub(crate) a: Axis,
}

impl AxisMap {
    pub(crate) fn of(axis: Axis) -> Self {
        match axis {
            Axis::Y => Self {
                s: Axis::X,
                t: Axis::Z,
                a: Axis::Y,
            },
            Axis::X => Self {
                s: Axis::Y,
                t: Axis::Z,
                a: Axis::X,
            },
            Axis::Z => Self {
                s: Axis::X,
                t: Axis::Y,
                a: Axis::Z,
            },
        }
    }

    pub(crate) fn axes(self) -> [Axis; 3] {
        [self.s, self.t, self.a]
    }
}

pub(crate) fn comp(v: I64Vec3, axis: Axis) -> i64 {
    match axis {
        Axis::X => v.x,
        Axis::Y => v.y,
        Axis::Z => v.z,
    }
}

pub(crate) fn unit(axis: Axis) -> I64Vec3 {
    match axis {
        Axis::X => I64Vec3::X,
        Axis::Y => I64Vec3::Y,
        Axis::Z => I64Vec3::Z,
    }
}

pub(crate) fn stack_axis(g: StackGeometry) -> Axis {
    match g {
        StackGeometry::Vertical { axis: 0 } => Axis::X,
        StackGeometry::Vertical { .. } => Axis::Z,
        _ => Axis::Y,
    }
}

/// Whether the stacks of two L0 cells that touch along `axis` share a seam Between.
pub(crate) fn joins(a: StackGeometry, b: StackGeometry) -> bool {
    !matches!(a, StackGeometry::Tilted { .. })
        && !matches!(b, StackGeometry::Tilted { .. })
        && stack_axis(a) == stack_axis(b)
}

fn run_key(seed: WorldSeed, home: I64Vec3, index: u16) -> u64 {
    absorb(
        hash(seed.0, Level::L0, home, salt::BETWEEN_KEY),
        u64::from(index),
    )
}

/// The Betweens of L0 cell `home`, as runs along its stack axis (spec §4W.6.1). Seam Betweens
/// are merged with the half in the neighbouring cell when both stacks are plain, so both cells
/// derive one run with one key.
pub(crate) fn runs(seed: WorldSeed, home: I64Vec3, web: &WebFields, st: &Stack) -> Vec<Run> {
    let axis = stack_axis(web.stack);
    let base = comp(home, axis) * L0_M;
    let mut out = Vec::new();
    let n = st.segments.len();
    let copies: &[i64] = if matches!(web.stack, StackGeometry::Tilted { .. }) {
        &[-1, 0, 1]
    } else {
        &[0]
    };
    for (i, seg) in st.segments.iter().enumerate() {
        let (Stratum::Between(j), Some(kind)) = (seg.stratum, seg.kind) else {
            continue;
        };
        // A Tilted stack wraps: its first Between is the continuation of its last one.
        let tilted = matches!(web.stack, StackGeometry::Tilted { .. });
        if tilted && i == 0 {
            continue;
        }
        let mut lo = base + i64::from(seg.start) * 64;
        let mut hi = base + i64::from(seg.end) * 64;
        let mut key = run_key(seed, home, j);
        let mut fb = i64::from(web.fill_between.max(1));
        if i == 0 {
            let below = home - unit(axis);
            let bw = regime(seed, below).web;
            if joins(web.stack, bw.stack) {
                let bst = stack(seed, below);
                if let Some(last) = bst.segments.last()
                    && let Stratum::Between(bj) = last.stratum
                {
                    lo = base - i64::from(last.depth()) * 64;
                    key = run_key(seed, below, bj);
                    fb = i64::from(bw.fill_between.max(1));
                }
            }
        }
        if tilted && i + 1 == n {
            hi = base + L0_M + i64::from(st.segments[0].depth()) * 64;
        }
        if !tilted && i + 1 == n {
            let above = home + unit(axis);
            let aw = regime(seed, above).web;
            if joins(web.stack, aw.stack) {
                let ast = stack(seed, above);
                hi = base + L0_M + i64::from(ast.segments[0].depth()) * 64;
            }
        }
        for &k in copies {
            out.push(Run {
                key: if k == 0 {
                    key
                } else {
                    absorb(key, (k + 2) as u64)
                },
                kind,
                flooded: seg.flooded,
                fb,
                lo: lo + k * L0_M,
                hi: hi + k * L0_M,
            });
        }
    }
    out
}

pub(crate) fn site_rng(seed: WorldSeed, p: i64, q: i64, key: u64, salt: u64) -> Rng {
    Rng::new(hash(
        seed.0,
        Level::L1,
        I64Vec3::new(p, q, key as i64),
        salt,
    ))
}

/// Everything the Betweens of L1 cell `l1` hold that the cell can see (spec §4W.6.1).
pub fn between_content(seed: WorldSeed, l1: I64Vec3) -> Vec<WebPiece> {
    content(seed, l1, None)
}

/// `between_content` with `fill_between` forced, for calibration tests.
fn content(seed: WorldSeed, l1: I64Vec3, fb_override: Option<i64>) -> Vec<WebPiece> {
    let (mut g, runs) = begin(seed, l1);
    let a0 = comp(g.origin, g.map.a);
    let (smin, smax) = g.shift_range();
    for mut run in runs {
        if let Some(fb) = fb_override {
            run.fb = fb;
        }
        if run.hi + smax < a0 - 16 || run.lo + smin > a0 + L1_M + 16 {
            continue;
        }
        g.run(&run);
    }
    g.out
}

/// The generator state of L1 cell `l1` and the Between runs of its L0 cell.
pub(crate) fn begin(seed: WorldSeed, l1: I64Vec3) -> (Gen, Vec<Run>) {
    let home = l1.div_euclid(I64Vec3::splat(8));
    let web = regime(seed, home).web;
    let st = stack(seed, home);
    let axis = stack_axis(web.stack);
    let g = Gen {
        seed,
        origin: l1 * L1_M,
        home,
        map: AxisMap::of(axis),
        tilt: match web.stack {
            StackGeometry::Tilted { tilt, yaw } => Some((tilt, yaw)),
            _ => None,
        },
        out: Vec::new(),
        kind: None,
    };
    (g, runs(seed, home, &web, &st))
}

pub(crate) struct Gen {
    pub(crate) seed: WorldSeed,
    pub(crate) origin: I64Vec3,
    pub(crate) home: I64Vec3,
    pub(crate) map: AxisMap,
    pub(crate) tilt: Option<(u8, u8)>,
    pub(crate) out: Vec<WebPiece>,
    /// Replaces the `FeatureKind` of the pieces pushed while set.
    pub(crate) kind: Option<FeatureKind>,
}

type V3 = [i64; 3];

impl Gen {
    /// Cell range across the stack: `[s0, s0 + 4096)` and `[t0, t0 + 4096)`.
    pub(crate) fn s0(&self) -> i64 {
        comp(self.origin, self.map.s)
    }

    pub(crate) fn t0(&self) -> i64 {
        comp(self.origin, self.map.t)
    }

    pub(crate) fn a0(&self) -> i64 {
        comp(self.origin, self.map.a)
    }

    /// How far the strata are raised at plane position `(s, t)` in world metres.
    pub(crate) fn shift(&self, s: i64, t: i64) -> i64 {
        self.tilt.map_or(0, |(tilt, yaw)| {
            tilt_shift(
                tilt,
                yaw,
                s - comp(self.home, self.map.s) * L0_M,
                t - comp(self.home, self.map.t) * L0_M,
            )
        })
    }

    /// The least and greatest shift over the cell's footprint (the shift is linear).
    pub(crate) fn shift_range(&self) -> (i64, i64) {
        let (s0, t0) = (self.s0(), self.t0());
        let v = [
            self.shift(s0, t0),
            self.shift(s0 + L1_M, t0),
            self.shift(s0, t0 + L1_M),
            self.shift(s0 + L1_M, t0 + L1_M),
        ];
        (
            v.into_iter().min().unwrap_or(0),
            v.into_iter().max().unwrap_or(0),
        )
    }

    /// Slack at the ends of a column so an oblique crust surface covers them.
    pub(crate) fn margin(&self, width: i64) -> i64 {
        8 + self.tilt.map_or(0, |(t, _)| {
            i64::from(TILT_SLOPE[usize::from(t)]) * width / 128
        })
    }

    pub(crate) fn world(&self, v: V3) -> I64Vec3 {
        let mut w = I64Vec3::ZERO;
        for (axis, x) in self.map.axes().into_iter().zip(v) {
            match axis {
                Axis::X => w.x = x,
                Axis::Y => w.y = x,
                Axis::Z => w.z = x,
            }
        }
        w
    }

    pub(crate) fn local(&self, w: i64, axis: Axis) -> i64 {
        (w - comp(self.origin, axis)) * Q
    }

    /// A box from `lo` to `hi` in stack coordinates, trimmed to the cell.
    pub(crate) fn bx(
        &mut self,
        tag: &'static str,
        lo: V3,
        hi: V3,
        material: MaterialId,
        flags: PrimFlags,
    ) {
        let (a, b) = (self.world(lo), self.world(hi));
        let (lo, hi) = (a.min(b), a.max(b));
        let mut min = [0i32; 3];
        let mut max = [0i32; 3];
        for (i, axis) in Axis::ALL.into_iter().enumerate() {
            let l = comp(lo, axis).max(comp(self.origin, axis));
            let h = comp(hi, axis).min(comp(self.origin, axis) + L1_M);
            if l >= h {
                return;
            }
            min[i] = self.local(l, axis) as i32;
            max[i] = self.local(h, axis) as i32;
        }
        let aabb = QAabb::new(
            QVec3::new(min[0], min[1], min[2]),
            QVec3::new(max[0], max[1], max[2]),
        );
        self.push(
            tag,
            Shape::Box { aabb },
            material,
            FeatureKind::TETHER,
            flags,
        );
    }

    /// A cylinder along stack axis `k` (0 = s, 1 = t, 2 = a) through `centre` (the other two
    /// coordinates), from `start` to `end` along it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cyl(
        &mut self,
        tag: &'static str,
        k: usize,
        centre: V3,
        start: i64,
        end: i64,
        radius: i64,
        material: MaterialId,
        flags: PrimFlags,
    ) {
        if end <= start || radius <= 0 {
            return;
        }
        let axes = self.map.axes();
        let axis = axes[k];
        // Whole-cell rejection and trimming along the axis.
        let o = comp(self.origin, axis);
        let (start, end) = (start.max(o), end.min(o + L1_M));
        if end <= start {
            return;
        }
        for (i, ax) in axes.into_iter().enumerate() {
            if i == k {
                continue;
            }
            let c = centre[i];
            let o = comp(self.origin, ax);
            if c + radius <= o || c - radius >= o + L1_M {
                return;
            }
        }
        let mut c = centre;
        c[k] = start;
        let base = self.world(c);
        let base = QVec3::new(
            self.local(base.x, Axis::X) as i32,
            self.local(base.y, Axis::Y) as i32,
            self.local(base.z, Axis::Z) as i32,
        );
        let shape = Shape::cylinder(axis, base, ((end - start) * Q) as i32, (radius * Q) as i32);
        let kind = if matches!(material, MaterialId::PIPE) {
            FeatureKind::CABLE
        } else {
            FeatureKind::TETHER
        };
        let clip = cell_clip();
        let shape = Shape::oriented(Frame::Id, QVec3::new(0, 0, 0), local_of(shape), clip);
        self.push(tag, shape, material, kind, flags);
    }

    /// A ramp or stair in stack coordinates (axis Y stacks only: `rise` is a world direction).
    pub(crate) fn sloped(
        &mut self,
        tag: &'static str,
        lo: V3,
        hi: V3,
        rise: Dir4,
        steps: Option<u16>,
        material: MaterialId,
    ) {
        let (a, b) = (self.world(lo), self.world(hi));
        let (lo, hi) = (a.min(b), a.max(b));
        let q = |v: I64Vec3| {
            QVec3::new(
                self.local(v.x, Axis::X) as i32,
                self.local(v.y, Axis::Y) as i32,
                self.local(v.z, Axis::Z) as i32,
            )
        };
        let aabb = QAabb::new(q(lo), q(hi));
        let local = match steps {
            Some(steps) => LocalShape::StairRun { aabb, rise, steps },
            None => LocalShape::Ramp { aabb, rise },
        };
        let shape = Shape::oriented(Frame::Id, QVec3::new(0, 0, 0), local, cell_clip());
        self.push(tag, shape, material, FeatureKind::RAMP, PrimFlags::NONE);
    }

    pub(crate) fn push(
        &mut self,
        tag: &'static str,
        shape: Shape,
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        let b = shape.aabb();
        let clip = cell_clip();
        if b.max.x <= clip.min.x
            || b.min.x >= clip.max.x
            || b.max.y <= clip.min.y
            || b.min.y >= clip.max.y
            || b.max.z <= clip.min.z
            || b.min.z >= clip.max.z
        {
            return;
        }
        self.out.push(WebPiece {
            tag,
            prim: Primitive {
                shape,
                material,
                kind: self.kind.unwrap_or(kind),
                flags,
            },
        });
    }

    /// Lattice cells `(p, q)` of pitch `pitch` that overlap the cell's footprint.
    pub(crate) fn lattice(&self, pitch: i64) -> Vec<(i64, i64)> {
        let (s0, t0) = (self.s0(), self.t0());
        let (p0, p1) = (s0.div_euclid(pitch), (s0 + L1_M - 1).div_euclid(pitch));
        let (q0, q1) = (t0.div_euclid(pitch), (t0 + L1_M - 1).div_euclid(pitch));
        let mut v = Vec::new();
        for p in p0..=p1 {
            for q in q0..=q1 {
                v.push((p, q));
            }
        }
        v
    }

    fn run(&mut self, run: &Run) {
        match run.kind {
            BetweenKind::Shallow => self.forest(run),
            BetweenKind::Normal => self.mixed(run, 1024),
            BetweenKind::Abyssal => self.mixed(run, 2048),
            BetweenKind::Pillared => self.hypostyle(run),
            BetweenKind::Webbed => {}
        }
        self.nets(run);
        self.tubes(run);
        self.lights(run);
    }

    /// The run's extent along `a` at plane position `(s, t)`.
    pub(crate) fn extent(&self, run: &Run, s: i64, t: i64) -> (i64, i64) {
        let sh = self.shift(s, t);
        (run.lo + sh, run.hi + sh)
    }

    /// BTW-01: a forest of thin tethers.
    fn forest(&mut self, run: &Run) {
        const P: i64 = 128;
        for (p, q) in self.lattice(P) {
            let mut rng = site_rng(self.seed, p, q, run.key, salt::BETWEEN_SITE);
            if (rng.below(1000) as i64) * 196 >= 10_000 * run.fb {
                continue;
            }
            let w = 2 * i64::from(rng.range_i32(8, 33));
            let m = (P - w) / 32;
            let cs = p * P + P / 2 + 16 * i64::from(rng.range_i32(-(m as i32), m as i32 + 1));
            let ct = q * P + P / 2 + 16 * i64::from(rng.range_i32(-(m as i32), m as i32 + 1));
            let (lo, hi) = self.extent(run, cs, ct);
            let mg = self.margin(w);
            self.cyl(
                STR_TETHER,
                2,
                [cs, ct, 0],
                lo - mg,
                hi + mg,
                w / 2,
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
        }
    }

    /// BTW-03: a hypostyle, every site a tether of one width.
    fn hypostyle(&mut self, run: &Run) {
        let mut rng = site_rng(self.seed, 0, 0, run.key, salt::BETWEEN_SITE);
        let pitch = 256i64 << rng.below(3);
        let w = [32i64, 48, 64, 96][rng.below(4) as usize];
        for (p, q) in self.lattice(pitch) {
            let (cs, ct) = (p * pitch + pitch / 2, q * pitch + pitch / 2);
            let (lo, hi) = self.extent(run, cs, ct);
            let mg = self.margin(w);
            self.cyl(
                STR_TETHER,
                2,
                [cs, ct, 0],
                lo - mg,
                hi + mg,
                w / 2,
                MaterialId::CONCRETE,
                if w >= 48 {
                    PrimFlags::FAR_VISIBLE
                } else {
                    PrimFlags::NONE
                },
            );
        }
    }

    /// Normal and Abyssal runs: the standard mix of sites.
    fn mixed(&mut self, run: &Run, pitch: i64) {
        let abyssal = run.kind == BetweenKind::Abyssal;
        for (p, q) in self.lattice(pitch) {
            let mut rng = site_rng(self.seed, p, q, run.key, salt::BETWEEN_SITE);
            if (rng.below(1000) as i64) * 199 >= 10_000 * run.fb {
                continue;
            }
            let weights: [u32; 6] = if abyssal {
                [200, 0, 0, 0, 50, 0]
            } else {
                [450, 120, 120, 60, 80, 20]
            };
            let mut ty = rng.pick_weighted(&weights).unwrap_or(0);
            if ty == 5 && (pitch < 1024 || run.depth() < 512) {
                ty = 0;
            }
            let class = rng.pick_weighted(&[15, 25, 30, 30]).unwrap_or(0);
            let (wlo, whi) = [(50, 100), (100, 200), (200, 400), (400, 800)][class];
            let w = (2 * i64::from(rng.range_i32(wlo / 2, whi / 2 + 1))).min((pitch * 4 / 5) & !1);
            let round = rng.below(10) < 6;
            let m = ((pitch - w) / 32) as i32;
            let cs = p * pitch + pitch / 2 + 16 * i64::from(rng.range_i32(-m, m + 1));
            let ct = q * pitch + pitch / 2 + 16 * i64::from(rng.range_i32(-m, m + 1));
            let site = Site {
                cs,
                ct,
                w,
                pitch,
                p,
                q,
            };
            match ty {
                0 => self.tether(run, &site, round, &mut rng),
                1 => self.tiers(run, &site, false, &mut rng),
                2 => self.tiers(run, &site, true, &mut rng),
                3 => self.kiss(run, &site, &mut rng),
                4 => self.island(run, &site, &mut rng),
                _ => self.machine(run, &site, &mut rng),
            }
        }
    }

    /// STR-03.
    fn tether(&mut self, run: &Run, site: &Site, round: bool, rng: &mut Rng) {
        let w = site.w;
        let hollow = !round && w >= 120 && rng.below(10) < 3;
        let (lo, hi) = self.extent(run, site.cs, site.ct);
        let mg = self.margin(w);
        let (alo, ahi) = (lo - mg, hi + mg);
        let far = if w >= 24 {
            PrimFlags::FAR_VISIBLE
        } else {
            PrimFlags::NONE
        };
        if round {
            self.cyl(
                STR_TETHER,
                2,
                [site.cs, site.ct, 0],
                alo,
                ahi,
                w / 2,
                MaterialId::CONCRETE,
                far,
            );
        } else if !hollow {
            self.bx(
                STR_TETHER,
                [site.cs - w / 2, site.ct - w / 2, alo],
                [site.cs + w / 2, site.ct + w / 2, ahi],
                MaterialId::CONCRETE,
                far,
            );
        } else {
            let th = (w / 12).max(8);
            let (s0, s1, t0, t1) = (
                site.cs - w / 2,
                site.cs + w / 2,
                site.ct - w / 2,
                site.ct + w / 2,
            );
            for (a, b) in [
                ([s0, t0, alo], [s1, t0 + th, ahi]),
                ([s0, t1 - th, alo], [s1, t1, ahi]),
                ([s0, t0 + th, alo], [s0 + th, t1 - th, ahi]),
                ([s1 - th, t0 + th, alo], [s1, t1 - th, ahi]),
            ] {
                self.bx(STR_TETHER, a, b, MaterialId::CONCRETE, far);
            }
            self.switchback(site, th, lo, hi);
        }
        if run.flooded {
            let r = w / 2 + 1;
            let mut a = lo + 64;
            while a < hi {
                self.cyl(
                    STR_TETHER,
                    2,
                    [site.cs, site.ct, 0],
                    a,
                    a + 2,
                    r,
                    MaterialId::EMISSIVE,
                    PrimFlags::NO_COLLIDE,
                );
                a += 128;
            }
        }
    }

    /// The ramp switchback inside a hollow tether. Ramps rise along a world horizontal axis,
    /// so only stacks along Y have it.
    fn switchback(&mut self, site: &Site, th: i64, lo: i64, hi: i64) {
        if self.map.a != Axis::Y {
            return;
        }
        let iw = site.w - 2 * th;
        let rise = (iw / 4).max(8);
        let (sl, sr) = (site.cs - iw / 2, site.cs + iw / 2);
        let lane_a = site.ct - iw / 2;
        let lane_b = site.ct + iw / 2 - 6;
        let a0 = self.a0();
        let first = ((a0 - lo) / rise).max(0);
        let last = ((a0 + L1_M - lo) / rise)
            .min((hi - lo) / rise - 1)
            .min(first + 40);
        for i in first..=last {
            let base = lo + i * rise;
            let (lane, dir) = if i % 2 == 0 {
                (lane_a, Dir4::PosX)
            } else {
                (lane_b, Dir4::NegX)
            };
            self.sloped(
                STR_TETHER,
                [sl, lane, base],
                [sr, lane + 6, base + rise],
                dir,
                None,
                MaterialId::CONCRETE,
            );
            // The landing at the end of the flight, where the next one begins.
            let (ls, rs) = if i % 2 == 0 {
                (sr - 6, sr)
            } else {
                (sl, sl + 6)
            };
            self.bx(
                STR_TETHER,
                [ls, lane_a, base + rise - 1],
                [rs, lane_b + 6, base + rise],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
        }
    }

    /// Tiers of a drip (hanging from the crust above) or a spire (rising from the crust below).
    /// STR-04 and STR-05.
    fn tiers(&mut self, run: &Run, site: &Site, spire: bool, rng: &mut Rng) {
        let n = 4 + rng.below(5) as i64;
        let d = run.depth();
        let h_total = d * (30 + rng.below(56) as i64) / 100;
        let w0 = site.w.max(100).min((site.pitch * 4 / 5) & !1);
        let tag = if spire { STR_SPIRE } else { STR_DRIP };
        self.tier_stack(run, site, spire, n, h_total, w0, w0 / 5, tag, rng);
    }

    #[allow(clippy::too_many_arguments)]
    fn tier_stack(
        &mut self,
        run: &Run,
        site: &Site,
        from_lower: bool,
        n: i64,
        h_total: i64,
        w0: i64,
        w_end: i64,
        tag: &'static str,
        rng: &mut Rng,
    ) {
        let (lo, hi) = self.extent(run, site.cs, site.ct);
        let mg = self.margin(w0);
        let hi_t = (h_total / n).max(4);
        let (mut cs, mut ct) = (site.cs, site.ct);
        let jit = (w0 / 12).max(2);
        for i in 0..n {
            // Interpolates from `w0` at the root to `w_end` at the tip, in whole metres.
            let w = (w_end + (w0 - w_end) * (n - 1 - i) / (n - 1).max(1)).max(4) & !1;
            if i > 0 {
                cs += (rng.below(3) as i64 - 1) * jit;
                ct += (rng.below(3) as i64 - 1) * jit;
            }
            let (a, b) = if from_lower {
                (
                    lo + i * hi_t - if i == 0 { mg } else { 0 },
                    lo + (i + 1) * hi_t,
                )
            } else {
                (
                    hi - (i + 1) * hi_t,
                    hi - i * hi_t + if i == 0 { mg } else { 0 },
                )
            };
            self.bx(
                tag,
                [cs - w / 2, ct - w / 2, a],
                [cs + w / 2, ct + w / 2, b],
                MaterialId::CONCRETE,
                PrimFlags::FAR_VISIBLE,
            );
        }
        // The tip needle.
        let tip = (w0 / 40).max(1);
        let len = h_total / 4;
        if from_lower {
            let top = lo + n * hi_t;
            self.cyl(
                tag,
                2,
                [cs, ct, 0],
                top,
                top + len,
                tip,
                MaterialId::METAL,
                PrimFlags::NONE,
            );
        } else {
            let bot = hi - n * hi_t;
            self.cyl(
                tag,
                2,
                [cs, ct, 0],
                bot - len,
                bot,
                tip,
                MaterialId::METAL,
                PrimFlags::NONE,
            );
        }
    }

    /// STR-06: a spire and a drip that meet halfway, with a waist and its stair.
    fn kiss(&mut self, run: &Run, site: &Site, rng: &mut Rng) {
        let d = run.depth();
        let m = d * (40 + rng.below(21) as i64) / 100;
        let n = 4 + rng.below(5) as i64;
        let w0 = site.w.max(100).min((site.pitch * 4 / 5) & !1);
        let t = (w0 / 5).max(8) & !1;
        self.tier_stack(run, site, true, n, m, w0, t, STR_KISS, &mut rng.clone());
        self.tier_stack(
            run,
            site,
            false,
            n,
            d - m,
            w0,
            t,
            STR_KISS,
            &mut rng.clone(),
        );
        let (lo, _) = self.extent(run, site.cs, site.ct);
        let waist = lo + m;
        self.bx(
            STR_KISS,
            [site.cs - t / 2, site.ct - t / 2, waist - 4],
            [site.cs + t / 2, site.ct + t / 2, waist + 4],
            MaterialId::CONCRETE,
            PrimFlags::NONE,
        );
        if self.map.a == Axis::Y {
            self.sloped(
                STR_KISS,
                [site.cs + t / 2, site.ct - 4, waist - 8],
                [site.cs + t / 2 + 32, site.ct + 4, waist + 8],
                Dir4::PosX,
                Some(64),
                MaterialId::CONCRETE,
            );
        }
    }

    /// STR-07: a platform stranded on a single stem.
    fn island(&mut self, run: &Run, site: &Site, rng: &mut Rng) {
        let d = run.depth();
        let (lo, hi) = self.extent(run, site.cs, site.ct);
        let hung = rng.below(10) >= 7;
        let r = 4 + rng.below(21) as i64;
        let stem = d * (40 + rng.below(41) as i64) / 100;
        let pw = (120 + 2 * rng.below(191) as i64).min((site.pitch * 4 / 5) & !1);
        let thick = 10 + rng.below(21) as i64;
        let (cs, ct) = (site.cs, site.ct);
        let (stem_lo, stem_hi, plat_lo, plat_hi) = if hung {
            (hi - stem, hi + 8, hi - stem - thick, hi - stem)
        } else {
            (lo - 8, lo + stem, lo + stem, lo + stem + thick)
        };
        self.cyl(
            STR_ISLAND,
            2,
            [cs, ct, 0],
            stem_lo,
            stem_hi,
            r,
            MaterialId::CONCRETE,
            PrimFlags::FAR_VISIBLE,
        );
        self.bx(
            STR_ISLAND,
            [cs - pw / 2, ct - pw / 2, plat_lo],
            [cs + pw / 2, ct + pw / 2, plat_hi],
            MaterialId::CONCRETE,
            PrimFlags::FAR_VISIBLE,
        );
        let sheds = 1 + rng.below(3) as i64;
        for _ in 0..sheds {
            let sz = 8 + rng.below(17) as i64;
            let off_s = (rng.below((pw - sz).max(1) as u64) as i64) - (pw - sz) / 2;
            let off_t = (rng.below((pw - sz).max(1) as u64) as i64) - (pw - sz) / 2;
            let (a, b) = if hung {
                (plat_lo - sz, plat_lo)
            } else {
                (plat_hi, plat_hi + sz)
            };
            self.bx(
                STR_ISLAND,
                [cs + off_s - sz / 2, ct + off_t - sz / 2, a],
                [cs + off_s + sz / 2, ct + off_t + sz / 2, b],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
        }
    }

    /// STR-10: a machine the size of a district, on legs.
    fn machine(&mut self, run: &Run, site: &Site, rng: &mut Rng) {
        let d = run.depth();
        let (lo, hi) = self.extent(run, site.cs, site.ct);
        let along_s = rng.below(2) == 0;
        let r = (100 + 2 * rng.below(101) as i64)
            .min(d * 35 / 100)
            .min(site.pitch * 2 / 5);
        let len =
            (r * 12 / 5 + rng.below((r * 18 / 5).max(1) as u64) as i64).min(site.pitch * 9 / 10);
        let hung = rng.below(10) >= 7;
        let ac = lo + d * (33 + rng.below(34) as i64) / 100;
        let (cs, ct) = (site.cs, site.ct);
        let k = usize::from(!along_s);
        let c0 = if along_s { cs } else { ct };
        let mut centre = [cs, ct, ac];
        centre[k] = 0;
        // The body axis lies along `k`; the cylinder's centre is on the other two coordinates.
        self.cyl(
            STR_MACHINE,
            k,
            centre,
            c0 - len / 2,
            c0 + len / 2,
            r,
            MaterialId::METAL,
            PrimFlags::FAR_VISIBLE,
        );
        let flanges = 2 + rng.below(3) as i64;
        for i in 1..=flanges {
            let at = c0 - len / 2 + len * i / (flanges + 1);
            self.cyl(
                STR_MACHINE,
                k,
                centre,
                at - 12,
                at + 12,
                r + 30,
                MaterialId::METAL,
                PrimFlags::FAR_VISIBLE,
            );
        }
        for end in [c0 - len / 2 - 16, c0 + len / 2] {
            self.cyl(
                STR_MACHINE,
                k,
                centre,
                end,
                end + 16,
                r / 3,
                MaterialId::METAL,
                PrimFlags::NONE,
            );
        }
        let leg = (r / 6).max(4) & !1;
        for ds in [-1i64, 1] {
            for dt in [-1i64, 1] {
                let (ls, lt) = if along_s {
                    (cs + ds * len * 2 / 5, ct + dt * r * 3 / 5)
                } else {
                    (cs + dt * r * 3 / 5, ct + ds * len * 2 / 5)
                };
                let (a, b) = if hung {
                    (ac + r, hi + 8)
                } else {
                    (lo - 8, ac - r)
                };
                self.bx(
                    STR_MACHINE,
                    [ls - leg / 2, lt - leg / 2, a],
                    [ls + leg / 2, lt + leg / 2, b],
                    MaterialId::METAL,
                    PrimFlags::FAR_VISIBLE,
                );
            }
        }
    }

    /// STR-08: transit tubes and open gantries with trams, keyed by the run.
    fn tubes(&mut self, run: &Run) {
        let n = {
            let mut r = site_rng(self.seed, -1, -1, run.key, salt::BETWEEN_LINE);
            match run.kind {
                BetweenKind::Normal => r.below(3) as i64,
                BetweenKind::Abyssal | BetweenKind::Webbed => r.below(2) as i64,
                _ => 0,
            }
        };
        let base = (
            comp(self.home, self.map.s) * L0_M,
            comp(self.home, self.map.t) * L0_M,
        );
        for t in 0..n {
            let mut rng = site_rng(self.seed, t, 0, run.key, salt::BETWEEN_LINE);
            // `k` is the axis of the tube across the stack: 0 along s, 1 along t.
            let k = rng.below(2) as usize;
            let pos = 64 * i64::from(rng.range_i32(64, 448));
            let a0 = 64 * i64::from(rng.range_i32(32, 256));
            let a1 = (a0 + 4096 + 64 * rng.below(128) as i64).min(28_672);
            let h = run.lo + run.depth() * (20 + rng.below(61) as i64) / 100;
            let radius = 8 + rng.below(23) as i64;
            let gantry = rng.below(2) == 0;
            let cars = 3 + rng.below(6) as i64;
            let car_at = a0 + 64 * rng.below(((a1 - a0) / 64 - 400 / 64).max(1) as u64) as i64;
            let (cs, ct) = if k == 0 {
                (base.0, base.1 + pos)
            } else {
                (base.0 + pos, base.1)
            };
            let (start, end) = (
                if k == 0 { base.0 + a0 } else { base.1 + a0 },
                if k == 0 { base.0 + a1 } else { base.1 + a1 },
            );
            let mid = i64::midpoint(start, end);
            let (ms, mt) = if k == 0 { (mid, ct) } else { (cs, mid) };
            let tube = Tube {
                k,
                cs,
                ct,
                h: h + self.shift(ms, mt),
                start,
                end,
                radius,
            };
            if gantry {
                self.gantry(&tube, cars, start + (car_at - a0));
            } else {
                self.closed_tube(&tube);
            }
            self.tube_ends(run, &tube);
        }
    }

    fn closed_tube(&mut self, t: &Tube) {
        let centre = [t.cs, t.ct, t.h];
        let far = PrimFlags::FAR_VISIBLE;
        self.cyl(
            STR_TUBE,
            t.k,
            centre,
            t.start,
            t.end,
            t.radius,
            MaterialId::METAL,
            far,
        );
        let mut x = t.start + 128;
        while x < t.end {
            let r = t.radius + 3;
            self.cyl(
                STR_TUBE,
                t.k,
                centre,
                x,
                x + 4,
                r,
                MaterialId::METAL,
                PrimFlags::NONE,
            );
            x += 256;
        }
    }

    /// An open rail with a tram on it.
    fn gantry(&mut self, t: &Tube, cars: i64, first_car: i64) {
        // A box along the tube from `a` to `b`, `w` wide, from height `lo` to `hi`.
        let along = |a: i64, b: i64, w: i64, lo: i64, hi: i64| {
            if t.k == 0 {
                ([a, t.ct - w / 2, lo], [b, t.ct + w / 2, hi])
            } else {
                ([t.cs - w / 2, a, lo], [t.cs + w / 2, b, hi])
            }
        };
        let (l, h) = along(t.start, t.end, 6, t.h - 3, t.h);
        self.bx(STR_TUBE, l, h, MaterialId::METAL, PrimFlags::NONE);
        for c in 0..cars {
            let at = first_car + c * 40;
            let (l, h) = along(at, at + 30, 8, t.h, t.h + 8);
            self.bx(STR_TUBE, l, h, MaterialId::METAL, PrimFlags::NONE);
            let (l, h) = along(at + 1, at + 29, 10, t.h + 3, t.h + 6);
            self.bx(
                STR_TUBE,
                l,
                h,
                MaterialId::TRAIN_LIGHT,
                PrimFlags::NO_COLLIDE,
            );
        }
    }

    /// The stations at both ends and the pylons that hang the line from the crust above.
    fn tube_ends(&mut self, run: &Run, t: &Tube) {
        for at in [t.start, t.end] {
            let (l, h) = if t.k == 0 {
                (
                    [at - 40, t.ct - 20, t.h - 20],
                    [at + 40, t.ct + 20, t.h + 20],
                )
            } else {
                (
                    [t.cs - 20, at - 40, t.h - 20],
                    [t.cs + 20, at + 40, t.h + 20],
                )
            };
            self.bx(STR_TUBE, l, h, MaterialId::METAL, PrimFlags::FAR_VISIBLE);
        }
        let mut x = t.start + 512;
        while x < t.end {
            let (ps, pt) = if t.k == 0 { (x, t.ct) } else { (t.cs, x) };
            let (_, hi) = self.extent(run, ps, pt);
            let from = t.h + t.radius.max(3);
            self.cyl(
                STR_TUBE,
                2,
                [ps, pt, 0],
                from,
                hi + 8,
                3,
                MaterialId::METAL,
                PrimFlags::NONE,
            );
            x += 1024;
        }
    }

    /// STR-09: patches of cable net.
    fn nets(&mut self, run: &Run) {
        const P: i64 = 2048;
        let chance = match run.kind {
            BetweenKind::Normal => 250,
            BetweenKind::Webbed => 700,
            BetweenKind::Abyssal => 100,
            _ => return,
        };
        for (p, q) in self.lattice(P) {
            let mut rng = site_rng(self.seed, p, q, run.key, salt::BETWEEN_NET);
            if (rng.below(1000) as i64) * 15 >= chance * run.fb.max(5) {
                continue;
            }
            let layers = if run.kind == BetweenKind::Webbed {
                2 + rng.below(3) as i64
            } else {
                1
            };
            let side = 512 + 32 * rng.below(41) as i64;
            let off_s = 16 * rng.below(((P - side) / 16 + 1) as u64) as i64;
            let off_t = 16 * rng.below(((P - side) / 16 + 1) as u64) as i64;
            let first = run.depth() * (10 + rng.below(81) as i64) / 100;
            let (s0, t0) = (p * P + off_s, q * P + off_t);
            let (lo, _) = self.extent(run, s0 + side / 2, t0 + side / 2);
            for layer in 0..layers {
                let span = (run.depth() - 16).max(1);
                let h = lo + 8 + (first + layer * run.depth() / 6) % span;
                let mut off = 0;
                while off <= side {
                    // Cables along s at t = t0 + off, along t at s = s0 + off.
                    self.cyl(
                        STR_NET,
                        0,
                        [0, t0 + off, h],
                        s0,
                        s0 + side,
                        1,
                        MaterialId::PIPE,
                        PrimFlags::NO_COLLIDE,
                    );
                    self.cyl(
                        STR_NET,
                        1,
                        [s0 + off, 0, h],
                        t0,
                        t0 + side,
                        1,
                        MaterialId::PIPE,
                        PrimFlags::NO_COLLIDE,
                    );
                    off += 32;
                }
            }
        }
    }

    /// STR-11: window speckle on the underside of the crust above.
    fn lights(&mut self, run: &Run) {
        const P: i64 = 64;
        let (s0, t0) = (self.s0(), self.t0());
        let (a_lo, a_hi) = (self.a0(), self.a0() + L1_M);
        if self.tilt.is_none() && (run.hi < a_lo || run.hi > a_hi) {
            return;
        }
        let mult = match (run.kind == BetweenKind::Abyssal, run.flooded) {
            (true, true) => 6,
            (true, false) => 2,
            (false, true) => 3,
            (false, false) => 1,
        };
        for q in t0.div_euclid(P)..=(t0 + L1_M - 1).div_euclid(P) {
            for p in s0.div_euclid(P)..=(s0 + L1_M - 1).div_euclid(P) {
                // Density per 256 m block, decided once per block.
                let mut block = site_rng(
                    self.seed,
                    p.div_euclid(4),
                    q.div_euclid(4),
                    run.key,
                    salt::BETWEEN_LIGHT,
                );
                let dens = [0, 0, 2, 4, 8, 14][block.below(6) as usize] * mult;
                if dens == 0 {
                    continue;
                }
                let mut rng = site_rng(self.seed, p, q, run.key, salt::BETWEEN_WINDOW);
                if rng.below(100) >= dens {
                    continue;
                }
                let (os, ot) = (4 * rng.below(14) as i64, 4 * rng.below(14) as i64);
                let (ws, wt) = (p * P + os, q * P + ot);
                let (_, hi) = self.extent(run, ws, wt);
                self.bx(
                    STR_LIGHTS,
                    [ws, wt, hi - 1],
                    [ws + 8, wt + 5, hi],
                    MaterialId::EMISSIVE,
                    PrimFlags::NO_COLLIDE,
                );
            }
        }
    }
}

/// A transit line across the stack: along axis `k` (0 = s, 1 = t) through `(cs, ct)` at height
/// `h`, from `start` to `end` along that axis.
#[derive(Debug, Clone, Copy)]
struct Tube {
    k: usize,
    cs: i64,
    ct: i64,
    h: i64,
    start: i64,
    end: i64,
    radius: i64,
}

#[derive(Debug, Clone, Copy)]
struct Site {
    cs: i64,
    ct: i64,
    w: i64,
    pitch: i64,
    #[allow(dead_code)]
    p: i64,
    #[allow(dead_code)]
    q: i64,
}

pub(crate) fn cell_clip() -> QAabb {
    let e = (L1_M * Q) as i32;
    QAabb::new(QVec3::new(0, 0, 0), QVec3::new(e, e, e))
}

fn local_of(shape: Shape) -> LocalShape {
    match shape {
        Shape::Cylinder {
            axis,
            base,
            length,
            radius,
        } => LocalShape::Cylinder {
            axis,
            base,
            length,
            radius,
        },
        // Only cylinders are routed here.
        _ => LocalShape::Box { aabb: shape.aabb() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_tags(
        seed: u64,
        cells: impl Iterator<Item = I64Vec3>,
    ) -> std::collections::HashMap<&'static str, usize> {
        let mut m = std::collections::HashMap::new();
        for c in cells {
            for p in between_content(WorldSeed(seed), c) {
                *m.entry(p.tag).or_insert(0) += 1;
            }
        }
        m
    }

    fn sample_cells() -> impl Iterator<Item = I64Vec3> {
        (-3..3).flat_map(|x| (0..8).flat_map(move |y| (-3..3).map(move |z| I64Vec3::new(x, y, z))))
    }

    /// STR-03..11 each occur in a sample of cells.
    #[test]
    fn every_tag_occurs() {
        let mut seen = std::collections::HashMap::new();
        for seed in [1u64, 2, 3] {
            for (k, v) in all_tags(seed, sample_cells()) {
                *seen.entry(k).or_insert(0) += v;
            }
        }
        for tag in [
            STR_TETHER,
            STR_DRIP,
            STR_SPIRE,
            STR_KISS,
            STR_ISLAND,
            STR_TUBE,
            STR_NET,
            STR_MACHINE,
            STR_LIGHTS,
        ] {
            assert!(seen.get(tag).copied().unwrap_or(0) > 0, "{tag}: {seen:?}");
        }
    }

    /// Every piece stays inside its own cell (invariant 4's ownership rule).
    #[test]
    fn pieces_stay_in_the_cell() {
        let e = (L1_M * Q) as i32;
        for c in sample_cells().take(120) {
            let pieces = between_content(WorldSeed(5), c);
            assert!(pieces.len() < 30_000, "{} pieces", pieces.len());
            for p in pieces {
                let b = p.prim.aabb();
                assert!(
                    b.min.x >= 0
                        && b.min.y >= 0
                        && b.min.z >= 0
                        && b.max.x <= e
                        && b.max.y <= e
                        && b.max.z <= e,
                    "{c:?} {} {b:?}",
                    p.tag
                );
            }
        }
    }

    #[test]
    fn deterministic() {
        let c = I64Vec3::new(2, 3, -1);
        assert_eq!(
            between_content(WorldSeed(9), c),
            between_content(WorldSeed(9), c)
        );
    }

    /// Invariant 4: a feature that crosses a cell face is the same solid on both sides. Points
    /// just inside and just beyond each shared face agree on whether they are solid.
    #[test]
    fn solids_agree_across_faces() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        let mut rng = Rng::new(44);
        let (mut total, mut solid, mut bad) = (0, 0, 0);
        let cells = [
            I64Vec3::new(0, 0, 0),
            I64Vec3::new(1, 1, 0),
            I64Vec3::new(3, 2, 1),
            I64Vec3::new(5, 3, 2),
            I64Vec3::new(-2, 6, -3),
            I64Vec3::new(1, 7, 1),
            I64Vec3::new(9, 7, 1),
            I64Vec3::new(17, 7, 9),
            I64Vec3::new(25, 15, 1),
            I64Vec3::new(33, 7, 41),
            I64Vec3::new(41, -1, 7),
            I64Vec3::new(49, 15, 25),
            I64Vec3::new(57, 23, 33),
        ];
        for c in cells {
            for axis in Axis::ALL {
                let next = c + unit(axis);
                // Across an L0 face the stacks agree only where they join (spec §4W.6.1);
                // elsewhere the face is a section cliff.
                if (comp(c, axis) + 1).rem_euclid(8) == 0 {
                    let w =
                        |cell: I64Vec3| regime(seed, cell.div_euclid(I64Vec3::splat(8))).web.stack;
                    if !joins(w(c), w(next)) || axis != stack_axis(w(c)) {
                        continue;
                    }
                }
                let (a, b) = (between_content(seed, c), between_content(seed, next));
                let eps = 1.0 / 32.0;
                for _ in 0..6000 {
                    let (u, v) = (rng.next_f32() * 4096.0, rng.next_f32() * 4096.0);
                    let at = |along: f32| match axis {
                        Axis::X => Vec3::new(along, u, v),
                        Axis::Y => Vec3::new(u, along, v),
                        Axis::Z => Vec3::new(u, v, along),
                    };
                    let inside = a.iter().any(|p| p.prim.shape.contains(at(4096.0 - eps)));
                    let beyond = b.iter().any(|p| p.prim.shape.contains(at(eps)));
                    total += 1;
                    solid += usize::from(inside || beyond);
                    bad += usize::from(inside != beyond);
                }
            }
        }
        assert!(solid > 100, "only {solid} solid samples of {total}");
        // The two points are `2 · eps` apart, so a round piece that touches the face exactly
        // (a thin tether whose footprint reaches its lattice edge) can differ on a handful of them.
        assert!(
            bad * 100 <= solid,
            "{bad} disagreements among {solid} solid samples"
        );
    }

    /// REG-01: the built share of Normal and Shallow Betweens follows `fill_between / 255`
    /// across its range (`regimes.md` §7.1). Above 20 the lattice is full and the share
    /// saturates at about 8 %, which the spec states.
    #[test]
    fn built_fraction_tracks_fill_between() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        for (kind, fb) in [BetweenKind::Normal, BetweenKind::Shallow]
            .into_iter()
            .flat_map(|k| [2i64, 5, 10, 20, 26].map(|f| (k, f)))
        {
            let mut rng = Rng::new(3);
            let (mut between, mut built) = (0u32, 0u32);
            for x in 0..10 {
                for z in 0..10 {
                    let c = I64Vec3::new(x * 8 + 3, 2, z * 8 + 3);
                    let home = c.div_euclid(I64Vec3::splat(8));
                    if !matches!(regime(seed, home).web.stack, StackGeometry::Flat) {
                        continue;
                    }
                    let st = stack(seed, home);
                    let pieces = content(seed, c, Some(fb));
                    for _ in 0..3000 {
                        let p = Vec3::new(
                            rng.next_f32() * 4096.0,
                            rng.next_f32() * 4096.0,
                            rng.next_f32() * 4096.0,
                        );
                        let w = c * L1_M + I64Vec3::new(p.x as i64, p.y as i64, p.z as i64);
                        let seg = st.segment_at(w.y.rem_euclid(L0_M) / 64);
                        if seg.kind == Some(kind) {
                            between += 1;
                            built += u32::from(pieces.iter().any(|q| q.prim.shape.contains(p)));
                        }
                    }
                }
            }
            let target = fb as f64 / 255.0;
            let got = f64::from(built) / f64::from(between.max(1));
            assert!(between > 3000, "{kind:?}: {between} samples");
            if fb <= 20 {
                assert!(
                    (0.7 * target..1.45 * target).contains(&got),
                    "{kind:?} fb {fb}: target {target:.3}, built {got:.3}"
                );
            } else {
                assert!(
                    (0.065..0.095).contains(&got),
                    "{kind:?} saturated at {got:.3}"
                );
            }
        }
    }

    /// BTW-01..04: each Between kind holds what the spec says and nothing else.
    #[test]
    fn kinds_hold_their_content() {
        let seed = WorldSeed(11);
        let mut tags: std::collections::HashMap<BetweenKind, std::collections::HashSet<&str>> =
            std::collections::HashMap::new();
        for x in -6..6 {
            for z in -6..6 {
                let home = I64Vec3::new(x, 0, z);
                let web = regime(seed, home).web;
                if web.stack != StackGeometry::Flat {
                    continue;
                }
                for run in runs(seed, home, &web, &stack(seed, home)) {
                    // The L1 cell holding the middle of the run, in the middle of the L0 cell.
                    let mid = i64::midpoint(run.lo, run.hi);
                    let l1 = I64Vec3::new(x * 8 + 3, mid.div_euclid(L1_M), z * 8 + 3);
                    let set = tags.entry(run.kind).or_default();
                    for p in between_content(seed, l1) {
                        // Only pieces in this run's own range belong to it.
                        let y = i64::from(p.prim.aabb().min.y) / Q + l1.y * L1_M;
                        if y >= run.lo && y < run.hi {
                            set.insert(p.tag);
                        }
                    }
                }
            }
        }
        let has = |k: BetweenKind, tag: &str| tags.get(&k).is_some_and(|s| s.contains(tag));
        for tag in [
            STR_DRIP,
            STR_SPIRE,
            STR_KISS,
            STR_ISLAND,
            STR_MACHINE,
            STR_NET,
        ] {
            assert!(!has(BetweenKind::Pillared, tag), "pillared holds {tag}");
            assert!(!has(BetweenKind::Shallow, tag), "shallow holds {tag}");
        }
        for tag in [
            STR_TETHER,
            STR_DRIP,
            STR_SPIRE,
            STR_KISS,
            STR_ISLAND,
            STR_MACHINE,
        ] {
            assert!(!has(BetweenKind::Webbed, tag), "webbed holds {tag}");
        }
        for tag in [STR_DRIP, STR_SPIRE, STR_KISS, STR_MACHINE] {
            assert!(!has(BetweenKind::Abyssal, tag), "abyssal holds {tag}");
        }
        assert!(has(BetweenKind::Pillared, STR_TETHER));
        assert!(has(BetweenKind::Shallow, STR_TETHER));
        assert!(has(BetweenKind::Webbed, STR_NET));
        assert!(has(BetweenKind::Normal, STR_TETHER));
    }
}
