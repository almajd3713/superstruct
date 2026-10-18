//! Crust surfaces: the skin plate with its openings, roofscape, ceiling-scape and the section
//! cliff (SURF-01..04, HOR-02). Spec: `docs/spec/levels.md` §4W.6.2.
//!
//! Like the Between content, every feature hangs on a run and a lattice site, so each cell
//! derives the same feature and clips its own piece.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use glam::I64Vec3;

use crate::cell::{Axis, Level};
use crate::hash::hash;
use crate::l1::{Rect, decompose};
use crate::l1_between::{
    Gen, L0_M, L1_M, Q, Run, WebPiece, begin, cell_clip, comp, site_rng, unit,
};
use crate::orient::{Frame, LocalShape};
use crate::prim::{FeatureKind, MaterialId, PrimFlags, QAabb, QVec3, Shape};
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::stack::{Stack, Stratum, stack};

pub const SURF_ROOF: &str = "SURF-01";
pub const SURF_CEILING: &str = "SURF-02";
pub const SURF_CLIFF: &str = "SURF-03";
pub const SURF_SKIN: &str = "SURF-04";

/// Skin thickness in metres.
const PLATE: i64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    Roofscape,
    Ceilingscape,
    Skin,
}

/// An opening in a skin, in stack coordinates and world metres (HOR-02). Wider than 32 m.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Opening {
    pub run_key: u64,
    pub ceiling: bool,
    pub s0: i64,
    pub t0: i64,
    pub s1: i64,
    pub t1: i64,
}

/// The kind of the floor (`ceiling = false`) or the ceiling of a run.
pub fn surface_kind(seed: WorldSeed, run_key: u64, ceiling: bool) -> SurfaceKind {
    let mut rng = site_rng(seed, i64::from(ceiling), 0, run_key, salt::SURFACE_KIND);
    let skin = rng.below(100) < if ceiling { 35 } else { 30 };
    match (skin, ceiling) {
        (true, _) => SurfaceKind::Skin,
        (false, false) => SurfaceKind::Roofscape,
        (false, true) => SurfaceKind::Ceilingscape,
    }
}

fn which_key(run: &Run, ceiling: bool) -> u64 {
    run.key.wrapping_add(u64::from(ceiling))
}

/// Everything the crust surfaces and cuts in L1 cell `l1` hold.
pub fn surface_content(seed: WorldSeed, l1: I64Vec3) -> Vec<WebPiece> {
    let (mut g, runs) = begin(seed, l1);
    let a0 = comp(g.origin, g.map.a);
    let (smin, smax) = g.shift_range();
    for run in &runs {
        for ceiling in [false, true] {
            let surf = if ceiling { run.hi } else { run.lo };
            if surf + smax + PLATE + 8 < a0 || surf + smin - PLATE - 8 > a0 + L1_M {
                continue;
            }
            g.surface(seed, run, ceiling);
        }
    }
    g.cliffs(seed);
    g.out
}

/// The openings of the skins of L1 cell `l1` (HOR-02). L1 data for the children.
pub fn surface_openings(seed: WorldSeed, l1: I64Vec3) -> Vec<Opening> {
    let (g, runs) = begin(seed, l1);
    let a0 = comp(g.origin, g.map.a);
    let mut out = Vec::new();
    if g.tilt.is_some() {
        return out;
    }
    for run in &runs {
        for ceiling in [false, true] {
            let surf = if ceiling { run.hi } else { run.lo };
            if surf + PLATE + 8 < a0 || surf - PLATE - 8 > a0 + L1_M {
                continue;
            }
            let kind = surface_kind(seed, run.key, ceiling);
            out.extend(g.openings(seed, run, ceiling, kind));
        }
    }
    out
}

impl Gen {
    /// The openings of one surface that overlap the cell.
    fn openings(
        &self,
        seed: WorldSeed,
        run: &Run,
        ceiling: bool,
        kind: SurfaceKind,
    ) -> Vec<Opening> {
        const P: i64 = 1024;
        let (chance, lo, hi) = if kind == SurfaceKind::Skin {
            (20, 8, 51)
        } else {
            (35, 4, 26)
        };
        let key = which_key(run, ceiling);
        let mut out = Vec::new();
        for (p, q) in self.lattice(P) {
            let mut rng = site_rng(seed, p, q, key, salt::SURFACE_OPENING);
            if rng.below(100) >= chance {
                continue;
            }
            let w = 8 * i64::from(rng.range_i32(lo, hi));
            let d = 8 * i64::from(rng.range_i32(lo, hi));
            let off_s = 8 * rng.below(((P - w) / 8 + 1) as u64) as i64;
            let off_t = 8 * rng.below(((P - d) / 8 + 1) as u64) as i64;
            let (s0, t0) = (p * P + off_s, q * P + off_t);
            out.push(Opening {
                run_key: run.key,
                ceiling,
                s0,
                t0,
                s1: s0 + w,
                t1: t0 + d,
            });
        }
        out
    }

    fn surface(&mut self, seed: WorldSeed, run: &Run, ceiling: bool) {
        let kind = surface_kind(seed, run.key, ceiling);
        let surf = if ceiling { run.hi } else { run.lo };
        if self.tilt.is_some() {
            self.tilted_plate(surf, ceiling);
            return;
        }
        let holes = self.openings(seed, run, ceiling, kind);
        self.plate(surf, ceiling, &holes);
        match kind {
            SurfaceKind::Roofscape => self.roofscape(seed, run, surf, &holes),
            SurfaceKind::Ceilingscape => self.ceilingscape(seed, run, surf),
            SurfaceKind::Skin => self.seams(surf, ceiling),
        }
    }

    /// The `a` range of a plate, below a floor or above a ceiling.
    fn plate_range(surf: i64, ceiling: bool) -> (i64, i64) {
        if ceiling {
            (surf, surf + PLATE)
        } else {
            (surf - PLATE, surf)
        }
    }

    /// The skin plate, cut into boxes around the openings (SURF-04).
    fn plate(&mut self, surf: i64, ceiling: bool, holes: &[Opening]) {
        let (s0, t0) = (self.s0(), self.t0());
        let to_q = |v: i64, o: i64| ((v - o) * Q) as i32;
        let rects: Vec<Rect> = holes
            .iter()
            .map(|h| Rect {
                x0: to_q(h.s0, s0).clamp(0, 65_536),
                z0: to_q(h.t0, t0).clamp(0, 65_536),
                x1: to_q(h.s1, s0).clamp(0, 65_536),
                z1: to_q(h.t1, t0).clamp(0, 65_536),
            })
            .collect();
        let (a_lo, a_hi) = Self::plate_range(surf, ceiling);
        self.kind = Some(FeatureKind::SKIN);
        for r in decompose(&rects) {
            let (ls, hs) = (s0 + i64::from(r.x0) / Q, s0 + i64::from(r.x1) / Q);
            let (lt, ht) = (t0 + i64::from(r.z0) / Q, t0 + i64::from(r.z1) / Q);
            self.bx(
                SURF_SKIN,
                [ls, lt, a_lo],
                [hs, ht, a_hi],
                MaterialId::CONCRETE,
                PrimFlags::FAR_VISIBLE,
            );
        }
        self.kind = None;
    }

    /// The plate of a Tilted stack: one slab in the stack's frame, no openings (PW.2.4b).
    fn tilted_plate(&mut self, surf: i64, ceiling: bool) {
        let Some((tilt, yaw)) = self.tilt else { return };
        let (cs, ct) = (self.s0() + L1_M / 2, self.t0() + L1_M / 2);
        let y = surf + self.shift(cs, ct);
        let q = |v: i64, axis: Axis| ((v - comp(self.origin, axis)) * Q) as i32;
        let origin = QVec3::new(q(cs, Axis::X), q(y, Axis::Y), q(ct, Axis::Z));
        let r = (4096 * Q) as i32;
        let thick = (PLATE * Q) as i32;
        let (y0, y1) = if ceiling { (0, thick) } else { (-thick, 0) };
        let aabb = QAabb::new(QVec3::new(-r, y0, -r), QVec3::new(r, y1, r));
        let shape = Shape::oriented(
            Frame::table(yaw, tilt),
            origin,
            LocalShape::Box { aabb },
            cell_clip(),
        );
        self.kind = Some(FeatureKind::SKIN);
        self.push(
            SURF_SKIN,
            shape,
            MaterialId::CONCRETE,
            FeatureKind::SKIN,
            PrimFlags::FAR_VISIBLE,
        );
        self.kind = None;
    }

    /// Panel seams of a near-blank skin: 1 m strips every 128 m.
    fn seams(&mut self, surf: i64, ceiling: bool) {
        let (s0, t0) = (self.s0(), self.t0());
        let (a_lo, a_hi) = if ceiling {
            (surf - 1, surf)
        } else {
            (surf, surf + 1)
        };
        self.kind = Some(FeatureKind::SKIN);
        let mut x = s0.div_euclid(128) * 128;
        while x < s0 + L1_M {
            self.bx(
                SURF_SKIN,
                [x, t0, a_lo],
                [x + 1, t0 + L1_M, a_hi],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
            x += 128;
        }
        let mut y = t0.div_euclid(128) * 128;
        while y < t0 + L1_M {
            self.bx(
                SURF_SKIN,
                [s0, y, a_lo],
                [s0 + L1_M, y + 1, a_hi],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
            y += 128;
        }
        self.kind = None;
    }

    /// `a` range of something standing on (floor) or hanging from (ceiling) the surface: `len`
    /// outward and `embed` into the plate.
    fn out_range(surf: i64, ceiling: bool, len: i64, embed: i64) -> (i64, i64) {
        if ceiling {
            (surf - len, surf + embed)
        } else {
            (surf - embed, surf + len)
        }
    }

    /// SURF-01: rooftops, terraces and antenna forests.
    fn roofscape(&mut self, seed: WorldSeed, run: &Run, surf: i64, holes: &[Opening]) {
        const LOT: i64 = 64;
        let key = which_key(run, false);
        let touches = |s: i64, t: i64, w: i64| {
            holes
                .iter()
                .any(|h| s < h.s1 && s + w > h.s0 && t < h.t1 && t + w > h.t0)
        };
        for (p, q) in self.lattice(LOT) {
            let mut rng = site_rng(seed, p, q, key, salt::SURFACE_LOT);
            if rng.below(100) >= 55 {
                continue;
            }
            let w = 8 * i64::from(rng.range_i32(2, 8));
            let d = 8 * i64::from(rng.range_i32(2, 8));
            let tower = rng.below(20) == 0;
            let h = if tower {
                60 + 4 * rng.below(16) as i64
            } else {
                4 * i64::from(rng.range_i32(1, 8))
            };
            let terrace = rng.below(10) < 6;
            let th = 4 * i64::from(rng.range_i32(1, 4));
            let s = p * LOT + rng.below(((LOT - w) / 4 + 1) as u64) as i64 * 4;
            let t = q * LOT + rng.below(((LOT - d) / 4 + 1) as u64) as i64 * 4;
            if touches(s, t, w.max(d)) {
                continue;
            }
            let far = if w.max(d) > 24 || tower {
                PrimFlags::FAR_VISIBLE
            } else {
                PrimFlags::NONE
            };
            self.kind = Some(FeatureKind::MASS);
            let (a, b) = Self::out_range(surf, false, h, 4);
            self.bx(
                SURF_ROOF,
                [s, t, a],
                [s + w, t + d, b],
                MaterialId::CONCRETE,
                far,
            );
            if terrace && w > 12 && d > 12 {
                self.bx(
                    SURF_ROOF,
                    [s + 6, t + 6, b],
                    [s + w - 6, t + d - 6, b + th],
                    MaterialId::CONCRETE,
                    PrimFlags::NONE,
                );
            }
        }
        self.masts(seed, key, surf, holes);
        self.kind = None;
    }

    /// Antenna forests: blocks of 256 m with masts and beacons.
    fn masts(&mut self, seed: WorldSeed, key: u64, surf: i64, _holes: &[Opening]) {
        const BLOCK: i64 = 256;
        for (p, q) in self.lattice(BLOCK) {
            let mut rng = site_rng(seed, p, q, key, salt::SURFACE_OPENING ^ 0x100);
            if rng.below(100) >= 25 {
                continue;
            }
            let n = 6 + rng.below(15) as i64;
            for _ in 0..n {
                let s = p * BLOCK + rng.below(BLOCK as u64) as i64;
                let t = q * BLOCK + rng.below(BLOCK as u64) as i64;
                let h = 20 + rng.below(101) as i64;
                let beacon = rng.below(10) < 3;
                self.kind = Some(FeatureKind::CABLE);
                self.cyl(
                    SURF_ROOF,
                    2,
                    [s, t, 0],
                    surf - 2,
                    surf + h,
                    1,
                    MaterialId::METAL,
                    PrimFlags::NO_COLLIDE,
                );
                if beacon {
                    self.kind = Some(FeatureKind::WINDOW);
                    self.bx(
                        SURF_ROOF,
                        [s - 1, t - 1, surf + h],
                        [s + 1, t + 1, surf + h + 2],
                        MaterialId::BEACON,
                        PrimFlags::NO_COLLIDE,
                    );
                }
            }
        }
    }

    /// SURF-02: coffers, hanging machinery, pipe runs and lamps.
    fn ceilingscape(&mut self, seed: WorldSeed, run: &Run, surf: i64) {
        let key = which_key(run, true);
        let (s0, t0) = (self.s0(), self.t0());
        self.kind = Some(FeatureKind::SKIN);
        let (a, b) = Self::out_range(surf, true, 10, 1);
        let mut x = s0.div_euclid(64) * 64;
        while x < s0 + L1_M {
            self.bx(
                SURF_CEILING,
                [x, t0, a],
                [x + 6, t0 + L1_M, b],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
            x += 64;
        }
        let mut y = t0.div_euclid(64) * 64;
        while y < t0 + L1_M {
            self.bx(
                SURF_CEILING,
                [s0, y, a],
                [s0 + L1_M, y + 6, b],
                MaterialId::CONCRETE,
                PrimFlags::NONE,
            );
            y += 64;
        }
        // Hanging machinery per 256 m block.
        for (p, q) in self.lattice(256) {
            let mut rng = site_rng(seed, p, q, key, salt::SURFACE_LOT);
            if rng.below(100) >= 30 {
                continue;
            }
            let w = 8 * i64::from(rng.range_i32(1, 5));
            let d = 8 * i64::from(rng.range_i32(1, 5));
            let depth = 12 + rng.below(29) as i64;
            let s = p * 256 + 8 * rng.below(((256 - w) / 8) as u64) as i64;
            let t = q * 256 + 8 * rng.below(((256 - d) / 8) as u64) as i64;
            let (a, b) = Self::out_range(surf, true, depth, 1);
            self.kind = Some(FeatureKind::MASS);
            self.bx(
                SURF_CEILING,
                [s, t, a],
                [s + w, t + d, b],
                MaterialId::METAL,
                PrimFlags::NONE,
            );
        }
        // Pipe runs along s every 128 m.
        for q in t0.div_euclid(128)..=(t0 + L1_M - 1).div_euclid(128) {
            let mut rng = site_rng(seed, q, -1, key, salt::SURFACE_LOT);
            if rng.below(100) >= 50 {
                continue;
            }
            self.kind = Some(FeatureKind::PIPE);
            let at = surf - 14;
            self.cyl(
                SURF_CEILING,
                0,
                [0, q * 128 + 64, at],
                s0,
                s0 + L1_M,
                2,
                MaterialId::PIPE,
                PrimFlags::NO_COLLIDE,
            );
        }
        // Hanging lamps on a 32 m lattice.
        for (p, q) in self.lattice(32) {
            let mut rng = site_rng(seed, p, q, key, salt::SURFACE_LOT ^ 0x200);
            if rng.below(100) >= 10 {
                continue;
            }
            let chain = 6 + rng.below(25) as i64;
            let (s, t) = (p * 32 + 16, q * 32 + 16);
            self.kind = Some(FeatureKind::CABLE);
            self.cyl(
                SURF_CEILING,
                2,
                [s, t, 0],
                surf - chain,
                surf,
                1,
                MaterialId::METAL,
                PrimFlags::NO_COLLIDE,
            );
            self.kind = Some(FeatureKind::WINDOW);
            self.bx(
                SURF_CEILING,
                [s - 1, t - 1, surf - chain - 2],
                [s + 1, t + 1, surf - chain],
                MaterialId::EMISSIVE,
                PrimFlags::NO_COLLIDE,
            );
        }
        self.kind = None;
    }

    /// SURF-03: walls of floors where a crust is cut by an L0 side face.
    fn cliffs(&mut self, seed: WorldSeed) {
        let l1 = self.origin.div_euclid(I64Vec3::splat(L1_M));
        let axes = [self.map.s, self.map.t];
        for (idx, axis) in axes.into_iter().enumerate() {
            for positive in [false, true] {
                let c = comp(l1, axis);
                if (positive && c.rem_euclid(8) != 7) || (!positive && c.rem_euclid(8) != 0) {
                    continue;
                }
                self.cliff_face(seed, idx, axis, positive);
            }
        }
    }

    fn cliff_face(&mut self, seed: WorldSeed, idx: usize, axis: Axis, positive: bool) {
        const G: i64 = 64;
        const N: usize = (L1_M / G) as usize;
        let home = self.home;
        let neighbour = home + if positive { unit(axis) } else { -unit(axis) };
        let (st_home, st_there) = (stack(seed, home), stack(seed, neighbour));
        let (geo_home, geo_there) = (
            regime(seed, home).web.stack,
            regime(seed, neighbour).web.stack,
        );
        let face = comp(self.origin, axis) + if positive { L1_M } else { 0 };
        let (inside, outside) = if positive {
            (face - 1, face)
        } else {
            (face, face - 1)
        };
        let a_axis = self.map.a;
        let other = if idx == 0 { self.map.t } else { self.map.s };
        let (a0, o0) = (comp(self.origin, a_axis), comp(self.origin, other));
        let crust = |st: &Stack, l0: I64Vec3, geo, p: I64Vec3| {
            let local = p - l0 * L0_M;
            let u = crate::stack::u_units(geo, local, None);
            matches!(st.segment_at(u).stratum, Stratum::Crust(_))
        };
        let point = |f: i64, a: i64, o: i64| {
            let mut p = I64Vec3::ZERO;
            for (ax, v) in [(axis, f), (a_axis, a), (other, o)] {
                match ax {
                    Axis::X => p.x = v,
                    Axis::Y => p.y = v,
                    Axis::Z => p.z = v,
                }
            }
            p
        };
        // exposed[o][a]
        let mut exposed = vec![[false; N]; N];
        let mut any = false;
        for (io, col) in exposed.iter_mut().enumerate() {
            for (ia, cell) in col.iter_mut().enumerate() {
                let (a, o) = (a0 + ia as i64 * G + G / 2, o0 + io as i64 * G + G / 2);
                if crust(&st_home, home, geo_home, point(inside, a, o))
                    && !crust(&st_there, neighbour, geo_there, point(outside, a, o))
                {
                    *cell = true;
                    any = true;
                }
            }
        }
        if !any {
            return;
        }
        // Runs down each column, then equal runs across columns.
        let mut rects: Vec<(usize, usize, usize, usize)> = Vec::new(); // (o0, o1, a0, a1)
        for (io, col) in exposed.iter().enumerate() {
            let mut ia = 0;
            while ia < N {
                if !col[ia] {
                    ia += 1;
                    continue;
                }
                let start = ia;
                while ia < N && col[ia] {
                    ia += 1;
                }
                if let Some(r) = rects
                    .iter_mut()
                    .find(|r| r.1 == io && r.2 == start && r.3 == ia)
                {
                    r.1 = io + 1;
                } else {
                    rects.push((io, io + 1, start, ia));
                }
            }
        }
        let face_key = hash(
            seed.0,
            Level::L1,
            I64Vec3::new(face, i64::from(axis as u8), i64::from(positive)),
            salt::CLIFF_WINDOW,
        );
        for (ro0, ro1, ra0, ra1) in rects {
            self.cliff_rect(
                idx,
                positive,
                face,
                (o0 + ro0 as i64 * G, o0 + ro1 as i64 * G),
                (a0 + ra0 as i64 * G, a0 + ra1 as i64 * G),
                face_key,
            );
        }
    }

    /// One exposed rectangle: wall plate, floor slabs and window blocks.
    fn cliff_rect(
        &mut self,
        idx: usize,
        positive: bool,
        face: i64,
        (o_lo, o_hi): (i64, i64),
        (a_lo, a_hi): (i64, i64),
        face_key: u64,
    ) {
        // Along the face axis the wall is set back 3 m from the face, inwards.
        let (wall, slab, glass) = if positive {
            ((face - 8, face - 3), (face - 3, face), (face - 3, face - 2))
        } else {
            ((face + 3, face + 8), (face, face + 3), (face + 2, face + 3))
        };
        // Triples are (s, t, a); the face axis is `idx`, the tangent is the other.
        let tri = |f: (i64, i64), o: (i64, i64), a: (i64, i64)| {
            let mut lo = [0i64; 3];
            let mut hi = [0i64; 3];
            lo[idx] = f.0;
            hi[idx] = f.1;
            lo[1 - idx] = o.0;
            hi[1 - idx] = o.1;
            lo[2] = a.0;
            hi[2] = a.1;
            (lo, hi)
        };
        self.kind = Some(FeatureKind::SKIN);
        let (lo, hi) = tri(wall, (o_lo, o_hi), (a_lo, a_hi));
        self.bx(
            SURF_CLIFF,
            lo,
            hi,
            MaterialId::CONCRETE,
            PrimFlags::FAR_VISIBLE,
        );
        let mut a = a_lo - a_lo.rem_euclid(4) + 4;
        while a < a_hi {
            let (lo, hi) = tri(slab, (o_lo, o_hi), (a, a + 1));
            self.bx(
                SURF_CLIFF,
                lo,
                hi,
                MaterialId::CONCRETE,
                PrimFlags::FAR_VISIBLE,
            );
            a += 4;
        }
        // Lit windows on every other floor, in blocks of 256 m along the face.
        self.kind = Some(FeatureKind::WINDOW);
        let mut floor = a_lo.div_euclid(8) * 8 + 8;
        while floor + 3 < a_hi {
            for block in o_lo.div_euclid(256)..=(o_hi - 1).div_euclid(256) {
                let mut rng = Rng::new(hash(
                    face_key,
                    Level::L1,
                    I64Vec3::new(block, floor, 0),
                    salt::CLIFF_WINDOW,
                ));
                if rng.below(100) >= 30 {
                    continue;
                }
                let len = 8 * i64::from(rng.range_i32(2, 6));
                let at = block * 256 + 8 * rng.below(((256 - len) / 8) as u64) as i64;
                let (s, e) = (at.max(o_lo), (at + len).min(o_hi));
                if s >= e {
                    continue;
                }
                let (lo, hi) = tri(glass, (s, e), (floor + 1, floor + 3));
                self.bx(
                    SURF_CLIFF,
                    lo,
                    hi,
                    MaterialId::EMISSIVE,
                    PrimFlags::NO_COLLIDE,
                );
            }
            floor += 8;
        }
        self.kind = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regime_web::StackGeometry;

    fn count(
        seed: u64,
        cells: impl Iterator<Item = I64Vec3>,
    ) -> std::collections::HashMap<&'static str, usize> {
        let mut m = std::collections::HashMap::new();
        for c in cells {
            for p in surface_content(WorldSeed(seed), c) {
                *m.entry(p.tag).or_insert(0) += 1;
            }
        }
        m
    }

    fn cells() -> impl Iterator<Item = I64Vec3> {
        (-4..4).flat_map(|x| (0..16).flat_map(move |y| (-4..4).map(move |z| I64Vec3::new(x, y, z))))
    }

    /// SURF-01..04 each occur.
    #[test]
    fn every_surface_tag_occurs() {
        let mut seen = std::collections::HashMap::new();
        for seed in [1u64, 2] {
            for (k, v) in count(seed, cells()) {
                *seen.entry(k).or_insert(0usize) += v;
            }
        }
        for tag in [SURF_ROOF, SURF_CEILING, SURF_CLIFF, SURF_SKIN] {
            assert!(seen.get(tag).copied().unwrap_or(0) > 0, "{tag}: {seen:?}");
        }
    }

    #[test]
    fn pieces_stay_in_the_cell_and_are_deterministic() {
        let e = (L1_M * Q) as i32;
        for c in cells().step_by(37).take(150) {
            let a = surface_content(WorldSeed(5), c);
            assert_eq!(a, surface_content(WorldSeed(5), c));
            assert!(a.len() < 40_000, "{} pieces", a.len());
            for p in a {
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

    /// HOR-02: every opening is wider than 32 m (at least 32 m on each side), keeps inside its
    /// lattice cell, and the plate is open exactly there and solid elsewhere.
    #[test]
    fn plate_is_open_exactly_at_the_openings() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        let mut rng = Rng::new(9);
        let (mut inside, mut outside) = (0, 0);
        for c in cells().step_by(11).take(400) {
            let (g, runs) = begin(seed, c);
            if g.tilt.is_some() {
                continue;
            }
            let ops = surface_openings(seed, c);
            for o in &ops {
                assert!(o.s1 - o.s0 >= 32 && o.t1 - o.t0 >= 32, "{o:?}");
                assert!((o.s1 - o.s0).max(o.t1 - o.t0) <= 400);
            }
            let pieces = surface_content(seed, c);
            for run in &runs {
                for ceiling in [false, true] {
                    let surf = if ceiling { run.hi } else { run.lo };
                    let a0 = comp(g.origin, g.map.a);
                    if surf - 8 < a0 || surf + 8 >= a0 + L1_M {
                        continue;
                    }
                    // Plate middle, 8 m into the crust.
                    let mid = if ceiling { surf + 8 } else { surf - 8 };
                    for _ in 0..400 {
                        let (s, t) = (
                            g.s0() as f32 + rng.next_f32() * 4096.0,
                            g.t0() as f32 + rng.next_f32() * 4096.0,
                        );
                        let in_hole = ops.iter().any(|o| {
                            o.run_key == run.key
                                && o.ceiling == ceiling
                                && s >= o.s0 as f32
                                && s < o.s1 as f32
                                && t >= o.t0 as f32
                                && t < o.t1 as f32
                        });
                        let mut v = [0f32; 3];
                        v[0] = s;
                        v[1] = t;
                        v[2] = mid as f32 + 0.5;
                        let mut w = Vec3::ZERO;
                        for (ax, x) in g.map.axes().into_iter().zip(v) {
                            match ax {
                                Axis::X => w.x = x - g.origin.x as f32,
                                Axis::Y => w.y = x - g.origin.y as f32,
                                Axis::Z => w.z = x - g.origin.z as f32,
                            }
                        }
                        let solid = pieces
                            .iter()
                            .any(|p| p.tag == SURF_SKIN && p.prim.shape.contains(w));
                        if in_hole {
                            assert!(!solid, "plate over an opening at {w:?}");
                            inside += 1;
                        } else {
                            assert!(solid, "hole in the plate at {w:?} cell {c:?}");
                            outside += 1;
                        }
                    }
                }
            }
        }
        assert!(inside > 20 && outside > 1000, "{inside} in, {outside} out");
    }

    /// The skin of a Tilted stack lies on the stack's surface: a point 4 m inside the plate is
    /// solid and a point 4 m outside it is not.
    #[test]
    fn tilted_plate_follows_the_surface() {
        use crate::stack::tilt_shift;
        use glam::Vec3;
        let seed = WorldSeed(7);
        let mut rng = Rng::new(2);
        let (mut checked, mut tilted_cells) = (0, 0);
        for x in -10..10 {
            for z in -10..10 {
                let home = I64Vec3::new(x, 0, z);
                let StackGeometry::Tilted { tilt, yaw } = regime(seed, home).web.stack else {
                    continue;
                };
                for l in [(3, 2, 3), (4, 5, 1), (1, 6, 6)] {
                    let c = home * 8 + I64Vec3::new(l.0, l.1, l.2);
                    tilted_cells += 1;
                    let (g, runs) = begin(seed, c);
                    let pieces = surface_content(seed, c);
                    for run in &runs {
                        for ceiling in [false, true] {
                            let surf = if ceiling { run.hi } else { run.lo };
                            for _ in 0..200 {
                                let (lx, lz) = (rng.next_f32() * 4096.0, rng.next_f32() * 4096.0);
                                let (wx, wz) = (g.origin.x + lx as i64, g.origin.z + lz as i64);
                                let sh =
                                    tilt_shift(tilt, yaw, wx - home.x * L0_M, wz - home.z * L0_M);
                                let y = surf + sh;
                                let inside = if ceiling { y + 4 } else { y - 4 } - g.origin.y;
                                let outside = if ceiling { y - 4 } else { y + 4 } - g.origin.y;
                                let solid = |ly: i64| {
                                    pieces.iter().any(|p| {
                                        p.tag == SURF_SKIN
                                            && p.prim.shape.contains(Vec3::new(lx, ly as f32, lz))
                                    })
                                };
                                if (0..L1_M).contains(&inside) {
                                    assert!(
                                        solid(inside),
                                        "{c:?} plate missing at {lx} {inside} {lz}"
                                    );
                                    checked += 1;
                                }
                                if (0..L1_M).contains(&outside) {
                                    assert!(
                                        !solid(outside),
                                        "{c:?} ceiling={ceiling} lo {} hi {} surf {surf} sh {sh} lx {lx} lz {lz} outside {outside} tilt {tilt} yaw {yaw}",
                                        run.lo,
                                        run.hi
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            tilted_cells > 0 && checked > 50,
            "{tilted_cells} cells, {checked} checks"
        );
    }

    /// SURF-03: a wall stands exactly where the crust here is cut by a different stack beyond
    /// the L0 face, sampled at the centres of the 64 m grid.
    #[test]
    fn cliff_walls_stand_where_the_crust_is_cut() {
        use crate::stack::u_units;
        use glam::Vec3;
        let seed = WorldSeed(1);
        let (mut walls, mut gaps) = (0, 0);
        for hx in -3..3 {
            for hz in -3..3 {
                let home = I64Vec3::new(hx, 0, hz);
                let geo = regime(seed, home).web.stack;
                if !matches!(geo, StackGeometry::Flat) {
                    continue;
                }
                let neighbour = home + I64Vec3::X;
                let geo_n = regime(seed, neighbour).web.stack;
                let (st, st_n) = (stack(seed, home), stack(seed, neighbour));
                // The +X face of the cell at the far end of the L0 cell.
                let c = home * 8 + I64Vec3::new(7, 3, 2);
                let pieces = surface_content(seed, c);
                let face = (c.x + 1) * L1_M;
                for iy in 0..64 {
                    for iz in 0..64 {
                        let (y, z) = (c.y * L1_M + iy * 64 + 32, c.z * L1_M + iz * 64 + 32);
                        let inside = I64Vec3::new(face - 1, y, z);
                        let outside = I64Vec3::new(face, y, z);
                        let is_crust = |s: &Stack, l0: I64Vec3, g, p: I64Vec3| {
                            matches!(
                                s.segment_at(u_units(g, p - l0 * L0_M, None)).stratum,
                                Stratum::Crust(_)
                            )
                        };
                        let exposed = is_crust(&st, home, geo, inside)
                            && !is_crust(&st_n, neighbour, geo_n, outside);
                        let p = Vec3::new(
                            (L1_M - 5) as f32,
                            (y - c.y * L1_M) as f32,
                            (z - c.z * L1_M) as f32,
                        );
                        let wall = pieces
                            .iter()
                            .any(|q| q.tag == SURF_CLIFF && q.prim.shape.contains(p));
                        assert_eq!(wall, exposed, "cell {c:?} y {y} z {z}");
                        walls += usize::from(wall);
                        gaps += usize::from(!wall);
                    }
                }
            }
        }
        assert!(
            walls > 500 && gaps > 500,
            "{walls} wall samples, {gaps} open"
        );
    }

    /// Invariant 4 for the skin plates: the plate on both sides of every shared face agrees,
    /// openings included (across an L0 side face the stacks differ and the face is a cliff).
    /// Gridded dressing (lots, ribs, seams) starts on the lattice and so exactly at the faces;
    /// nothing of it crosses one, which the lattice pitches guarantee.
    #[test]
    fn solids_agree_across_faces() {
        use glam::Vec3;
        let seed = WorldSeed(1);
        let mut rng = Rng::new(31);
        let (mut total, mut solid, mut bad) = (0, 0, 0);
        for c in [
            I64Vec3::new(0, 0, 0),
            I64Vec3::new(1, 1, 0),
            I64Vec3::new(3, 2, 1),
            I64Vec3::new(5, 3, 2),
            I64Vec3::new(2, 3, 5),
            I64Vec3::new(9, 7, 1),
            I64Vec3::new(17, 7, 9),
            I64Vec3::new(25, 15, 1),
            I64Vec3::new(33, 7, 41),
        ] {
            for axis in Axis::ALL {
                let next = c + unit(axis);
                let home = |cell: I64Vec3| cell.div_euclid(I64Vec3::splat(8));
                if home(c) != home(next) {
                    let w = |cell: I64Vec3| regime(seed, home(cell)).web.stack;
                    if !crate::l1_between::joins(w(c), w(next))
                        || axis != crate::l1_between::stack_axis(w(c))
                    {
                        continue;
                    }
                }
                let (a, b) = (surface_content(seed, c), surface_content(seed, next));
                let eps = 1.0 / 32.0;
                for _ in 0..6000 {
                    let (u, v) = (rng.next_f32() * 4096.0, rng.next_f32() * 4096.0);
                    let at = |along: f32| match axis {
                        Axis::X => Vec3::new(along, u, v),
                        Axis::Y => Vec3::new(u, along, v),
                        Axis::Z => Vec3::new(u, v, along),
                    };
                    let plate = |p: &&WebPiece| {
                        p.tag == SURF_SKIN && p.prim.flags.contains(PrimFlags::FAR_VISIBLE)
                    };
                    let inside = a
                        .iter()
                        .filter(plate)
                        .any(|p| p.prim.shape.contains(at(4096.0 - eps)));
                    let beyond = b
                        .iter()
                        .filter(plate)
                        .any(|p| p.prim.shape.contains(at(eps)));
                    total += 1;
                    solid += usize::from(inside || beyond);
                    bad += usize::from(inside != beyond);
                }
            }
        }
        assert!(solid > 100, "only {solid} solid samples of {total}");
        assert!(
            bad * 100 <= solid,
            "{bad} disagreements among {solid} solid samples"
        );
    }
}
