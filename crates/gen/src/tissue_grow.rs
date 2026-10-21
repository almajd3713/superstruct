//! Step 3 of the tissue: structures grow on supports until the cell reaches its fill density
//! (TIS-03, TIS-04, TIS-06, SUP-01..11). Spec: `docs/spec/levels.md` §5W.2 step 3, §5W.3.
//!
//! Every attempt makes the same draws whether it succeeds or not: sixteen words, from which the
//! support, the form, the mutations, the size, the place and the entrances are all read, so
//! the cell's output depends on the order of attempts only.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::manual_midpoint,
    clippy::match_same_arms,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::type_complexity
)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::DVec3;

use crate::cell::{Axis, Dir6};
use crate::forms::{FAMILIES, FORMS, members};
use crate::hubs::HUBS;
use crate::orient::{Frame, LocalShape};
use crate::prim::{
    Dir4, FeatureKind, MaterialId, PrimFlags, Primitive, Q, Q_PER_M, QAabb, QVec3, Shape,
};
use crate::rng::Rng;
use crate::tissue::{Lattice, SlotState, Tissue};

pub const SUPPORT_TAGS: [&str; 11] = [
    "SUP-01", "SUP-02", "SUP-03", "SUP-04", "SUP-05", "SUP-06", "SUP-07", "SUP-08", "SUP-09",
    "SUP-10", "SUP-11",
];
pub const STANDING: usize = 0;
pub const HANGING: usize = 1;
pub const SPANNING: usize = 2;
pub const SIDEWAYS: usize = 3;
pub const COLLAR: usize = 4;
pub const EMBEDDED: usize = 5;
pub const SUSPENDED: usize = 6;
pub const PILED: usize = 7;
pub const NESTED: usize = 8;
pub const COLLIDING: usize = 9;
pub const RADIAL: usize = 10;

/// Attempts, and consecutive rejections, after which growth stops (`levels.md` §5W.2).
pub const MAX_ATTEMPTS: usize = 64;
pub const MAX_REJECTS: usize = 16;

/// A box of slots in the lattice's frame, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotBox {
    pub lo: [i32; 3],
    pub hi: [i32; 3],
}

impl SlotBox {
    pub fn volume(&self) -> i64 {
        (0..3).map(|k| i64::from(self.hi[k] - self.lo[k])).product()
    }

    #[must_use]
    pub fn shifted(&self, d: [i32; 3]) -> Self {
        Self {
            lo: [0, 1, 2].map(|k| self.lo[k] + d[k]),
            hi: [0, 1, 2].map(|k| self.hi[k] + d[k]),
        }
    }

    pub(crate) fn inside(&self, n: usize) -> bool {
        let n = n as i32;
        (0..3).all(|k| self.lo[k] >= 0 && self.hi[k] <= n && self.lo[k] < self.hi[k])
    }

    pub(crate) fn slots(&self, lat: &Lattice) -> Vec<usize> {
        let mut out = Vec::new();
        for z in self.lo[2]..self.hi[2] {
            for y in self.lo[1]..self.hi[1] {
                for x in self.lo[0]..self.hi[0] {
                    out.push(lat.index([x as usize, y as usize, z as usize]));
                }
            }
        }
        out
    }
}

/// An oriented part of a structure that is not on the lattice's grid (Colliding).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OPart {
    pub frame: Frame,
    /// The corner, in cell-local metres.
    pub origin: [f64; 3],
    pub size: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entrance {
    /// The route slot outside the structure.
    pub slot: u32,
    /// The direction from that slot into the structure, in slot space.
    pub into: Dir6,
    /// The entrance of an engulfed route segment (levels.md §5W.2, TIS-04).
    pub engulf: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Structure {
    pub support: u8,
    pub form: u16,
    pub variant: u8,
    /// Up to two mutations, as indices into the mutation groups (PW.3.8 applies them).
    pub mutations: Vec<u8>,
    pub parts: Vec<SlotBox>,
    pub oriented: Vec<OPart>,
    /// Cables of a Suspended structure: `(x, z, y0, y1)` in slots.
    pub cables: Vec<[i32; 4]>,
    pub shaped: Vec<(Shaped, SlotBox)>,
    pub windows: Vec<(SlotBox, Dir6)>,
    /// An older form swallowed inside (MUT-15).
    pub inner: Vec<SlotBox>,
    pub dark: bool,
    pub lit: bool,
    pub slots: Vec<u32>,
    pub entrances: Vec<Entrance>,
    pub sealed: bool,
}

pub(crate) fn dir_of(k: usize, positive: bool) -> Dir6 {
    match (k, positive) {
        (0, true) => Dir6::PosX,
        (0, false) => Dir6::NegX,
        (1, true) => Dir6::PosY,
        (1, false) => Dir6::NegY,
        (2, true) => Dir6::PosZ,
        _ => Dir6::NegZ,
    }
}

/// Picks an index from `weights` with a word of randomness (no draw is made here).
fn pick(word: u64, weights: &[u32]) -> Option<usize> {
    let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    if total == 0 {
        return None;
    }
    let mut r = word % total;
    weights.iter().position(|&w| {
        let w = u64::from(w);
        if r < w {
            true
        } else {
            r -= w;
            false
        }
    })
}

/// Tries of the knot's hub after its first.
const HUB_RETRIES: usize = 3;

/// A rejected attempt. The code names the reason (1..=8 in `fit` and the entrances, 19 for
/// no support, 20 + the support when a place cannot be found); it is kept for debugging.
fn rej(_code: usize) -> Option<Structure> {
    None
}

/// The mutations of an attempt (MUT-xx numbers): none to two, by the group weights of the
/// stratum, read from words 3 to 5.
pub(crate) fn mutations_of(t: &Tissue, words: &[u64; 16]) -> Vec<u8> {
    let weights = t.character.map_or([4u8; 6], |c| c.mutations).map(u32::from);
    let mut out: Vec<u8> = Vec::new();
    // An inverted district turns every form upside down (RARE-01).
    if t.rare == Some(crate::rare::Rare::Inverted) {
        out.push(1);
    }
    // The Recursion: every structure swallows a smaller copy of an older form (STG-02).
    if t.strange == Some(crate::strange::Strange::Recursion) {
        out.push(15);
    }
    // None in half of the cases, one in three tenths, two in the rest.
    let count = match below(words[3], 10) {
        0..=4 => 0,
        5..=7 => 1,
        _ => 2,
    };
    for i in 0..count {
        let w = words[4 + i];
        let g = pick(w, &weights).unwrap_or(below(w, 6));
        let list = crate::mutations::GROUPS[g];
        let m = list[below(w >> 16, list.len())];
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// A word split in two ranges: `word % n`.
pub(crate) fn below(word: u64, n: usize) -> usize {
    (word % n.max(1) as u64) as usize
}

/// The lattice with a one-slot border, as bytes, for the candidate scan of one attempt.
pub(crate) struct World {
    pub(crate) n: usize,
    /// `0` outside or absent, then `1 + SlotState`.
    pub(crate) state: Vec<u8>,
    pub(crate) crust: Vec<bool>,
    /// Free slots a person can walk to from the route (everything, while there is no route).
    pub(crate) reach: Vec<bool>,
}

fn code(s: SlotState) -> u8 {
    1 + match s {
        SlotState::Free => 0,
        SlotState::Void => 1,
        SlotState::Solid => 2,
        SlotState::Route => 3,
        SlotState::Built => 4,
        SlotState::Engulfed => 5,
    }
}

fn decode(c: u8) -> Option<SlotState> {
    Some(match c {
        1 => SlotState::Free,
        2 => SlotState::Void,
        3 => SlotState::Solid,
        4 => SlotState::Route,
        5 => SlotState::Built,
        6 => SlotState::Engulfed,
        _ => return None,
    })
}

impl World {
    pub(crate) fn new(t: &Tissue) -> Self {
        let n = t.lattice.n;
        let m = n + 2;
        let mut state = vec![0u8; m * m * m];
        let mut crust = vec![false; m * m * m];
        let mut i = 0;
        for z in 0..n {
            for y in 0..n {
                let row = 1 + m * ((y + 1) + m * (z + 1));
                for x in 0..n {
                    if t.lattice.present[i] {
                        state[row + x] = code(t.lattice.state[i]);
                        crust[row + x] = t.in_crust[i];
                    }
                    i += 1;
                }
            }
        }
        let mut reach = vec![t.route.is_empty(); m * m * m];
        if !t.route.is_empty() {
            let pad = |i: usize| {
                let c = t.lattice.coords(i);
                (c[0] + 1) + m * ((c[1] + 1) + m * (c[2] + 1))
            };
            let mut stack: Vec<usize> = Vec::new();
            for &r in &t.route {
                let k = pad(r as usize);
                if !reach[k] {
                    reach[k] = true;
                    stack.push(r as usize);
                }
            }
            while let Some(i) = stack.pop() {
                for j in t.lattice.neighbours(i) {
                    let k = pad(j);
                    if !reach[k] && matches!(t.lattice.state[j], SlotState::Free | SlotState::Route)
                    {
                        reach[k] = true;
                        stack.push(j);
                    }
                }
            }
        }
        Self {
            n,
            state,
            crust,
            reach,
        }
    }

    pub(crate) fn at(&self, c: [i32; 3]) -> Option<usize> {
        let m = self.n as i32 + 2;
        if (0..3).any(|k| c[k] < -1 || c[k] > self.n as i32) {
            return None;
        }
        Some(((c[0] + 1) + m * ((c[1] + 1) + m * (c[2] + 1))) as usize)
    }

    pub(crate) fn state(&self, c: [i32; 3]) -> Option<SlotState> {
        self.at(c).and_then(|k| decode(self.state[k]))
    }

    pub(crate) fn crust(&self, c: [i32; 3]) -> bool {
        self.at(c).is_some_and(|k| self.crust[k])
    }

    /// Something a structure can lean on from the neighbouring slot `c`: parent solid, built
    /// matter, or the shell of the cell when `c` is outside the lattice but the slot beside it
    /// is inside a crust.
    pub(crate) fn support(&self, c: [i32; 3], from: [i32; 3], with_route: bool) -> bool {
        match self.state(c) {
            Some(SlotState::Solid | SlotState::Built | SlotState::Engulfed) => true,
            Some(SlotState::Route) => with_route,
            Some(_) => false,
            None => self.crust(from),
        }
    }
}

pub(crate) fn add(a: [i32; 3], k: usize, d: i32) -> [i32; 3] {
    let mut o = a;
    o[k] += d;
    o
}

/// The candidate lists of one attempt: where each support can start, as padded indices into
/// `World` (see `World::xyz`).
#[derive(Default)]
pub(crate) struct Candidates {
    pub(crate) stand: Vec<u32>,
    pub(crate) hang: Vec<u32>,
    /// The slot, and the axis (0 = x, 2 = z) and sign the structure extends along.
    pub(crate) side: Vec<(u32, u8, i8)>,
    /// The first slot, the axis and the length of a gap between two supports.
    pub(crate) gaps: Vec<(u32, u8, u8)>,
    pub(crate) trunk: Vec<u32>,
    pub(crate) host: Vec<u32>,
    pub(crate) hall: Vec<u32>,
    /// Free slots with route directly below, for stilts.
    pub(crate) route_top: Vec<u32>,
    /// Number of free slots.
    pub(crate) free: usize,
}

const FREE: u8 = 1;
const VOID: u8 = 2;
const SOLID: u8 = 3;
const ROUTE: u8 = 4;
const BUILT: u8 = 5;
const ENGULFED: u8 = 6;

impl World {
    pub(crate) fn xyz(&self, k: u32) -> [i32; 3] {
        let m = self.n as u32 + 2;
        [
            (k % m) as i32 - 1,
            (k / m % m) as i32 - 1,
            (k / (m * m)) as i32 - 1,
        ]
    }

    /// The `i`-th free slot in scan order.
    pub(crate) fn nth_free(&self, i: usize) -> Option<[i32; 3]> {
        self.state
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == FREE)
            .nth(i)
            .map(|(k, _)| self.xyz(k as u32))
    }
}

fn candidates(w: &World) -> Candidates {
    let m = w.n + 2;
    let stride = [1usize, m, m * m];
    let mut c = Candidates::default();
    // Whether slot code `nc` next to a slot whose crust flag is `crust` can carry a structure.
    let sup = |nc: u8, crust: bool, route: bool| {
        matches!(nc, SOLID | BUILT | ENGULFED) || (route && nc == ROUTE) || (nc == 0 && crust)
    };
    for z in 0..w.n {
        for y in 0..w.n {
            for x in 0..w.n {
                let k = (x + 1) + m * ((y + 1) + m * (z + 1));
                let code = w.state[k];
                match code {
                    FREE if !w.reach[k] => {}
                    FREE => {
                        c.free += 1;
                        let crust = w.crust[k];
                        if sup(w.state[k - stride[1]], crust, true) {
                            c.stand.push(k as u32);
                        }
                        if w.state[k - stride[1]] == ROUTE {
                            c.route_top.push(k as u32);
                        }
                        if sup(w.state[k + stride[1]], crust, false) {
                            c.hang.push(k as u32);
                        }
                        for (axis, s) in [(0u8, 1i8), (0, -1), (2, 1), (2, -1)] {
                            let behind = if s > 0 {
                                k - stride[usize::from(axis)]
                            } else {
                                k + stride[usize::from(axis)]
                            };
                            if sup(w.state[behind], crust, false) {
                                c.side.push((k as u32, axis, s));
                            }
                        }
                    }
                    SOLID => {
                        if w.state[k - stride[1]] == SOLID && w.state[k + stride[1]] == SOLID {
                            c.trunk.push(k as u32);
                        }
                        if w.state[k + stride[0]] == SOLID
                            && w.state[k + stride[1]] == SOLID
                            && w.state[k + stride[2]] == SOLID
                        {
                            c.host.push(k as u32);
                        }
                    }
                    VOID => {
                        let below = w.state[k - stride[1]];
                        if below != VOID
                            && below != 0
                            && w.state[k + stride[0]] == VOID
                            && w.state[k + stride[2]] == VOID
                        {
                            c.hall.push(k as u32);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Gaps along x and z: runs of free or void slots between two supports.
    for axis in [0usize, 2] {
        let other = 2 - axis;
        for y in 0..w.n {
            for o in 0..w.n {
                let mut p = [0usize; 3];
                p[1] = y;
                p[other] = o;
                let row = (p[0] + 1) + m * ((p[1] + 1) + m * (p[2] + 1));
                let mut i = 0;
                while i < w.n {
                    let k = row + i * stride[axis];
                    if !matches!(w.state[k], FREE | VOID) {
                        i += 1;
                        continue;
                    }
                    let mut len = 0;
                    while i + len < w.n && matches!(w.state[k + len * stride[axis]], FREE | VOID) {
                        len += 1;
                    }
                    if (2..=14).contains(&len) {
                        let (a, b) = (w.state[k - stride[axis]], w.state[k + len * stride[axis]]);
                        let (ca, cb) = (w.crust[k], w.crust[k + (len - 1) * stride[axis]]);
                        if sup(a, ca, false) && sup(b, cb, false) {
                            c.gaps.push((k as u32, axis as u8, len as u8));
                        }
                    }
                    i += len.max(1);
                }
            }
        }
    }
    c
}

/// The largest free box that grows from the slot `p` within `caps` (`[x, y, z]` extents) by
/// taking whole free layers in the directions `dirs`, in an order set by `word`.
pub(crate) fn expand(
    w: &World,
    p: [i32; 3],
    caps: [i32; 3],
    dirs: &[(usize, i32)],
    word: u64,
) -> SlotBox {
    let mut b = SlotBox {
        lo: p,
        hi: [p[0] + 1, p[1] + 1, p[2] + 1],
    };
    let start = below(word, dirs.len());
    loop {
        let mut grew = false;
        for i in 0..dirs.len() {
            let (k, s) = dirs[(start + i) % dirs.len()];
            if b.hi[k] - b.lo[k] >= caps[k] {
                continue;
            }
            // The layer that would be added.
            let at = if s > 0 { b.hi[k] } else { b.lo[k] - 1 };
            let (u, v) = match k {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let free = (b.lo[u]..b.hi[u]).all(|a| {
                (b.lo[v]..b.hi[v]).all(|c| {
                    let mut q = [0; 3];
                    q[k] = at;
                    q[u] = a;
                    q[v] = c;
                    w.state(q) == Some(SlotState::Free)
                })
            });
            if free {
                if s > 0 {
                    b.hi[k] += 1;
                } else {
                    b.lo[k] -= 1;
                }
                grew = true;
            }
        }
        if !grew {
            return b;
        }
    }
}

/// The largest of the boxes that `grow` makes from six candidates drawn with `word`.
pub(crate) fn best_of(list: &[u32], word: u64, grow: impl Fn(u32) -> SlotBox) -> Option<SlotBox> {
    if list.is_empty() {
        return None;
    }
    (0..6u64)
        .map(|j| {
            let h =
                (word ^ j.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            grow(list[below(h ^ (h >> 31), list.len())])
        })
        .max_by_key(SlotBox::volume)
}

pub(crate) fn clamp_box(lo: [i32; 3], size: [i32; 3], n: i32) -> SlotBox {
    let mut b = SlotBox { lo, hi: lo };
    for k in 0..3 {
        let s = size[k].min(n).max(1);
        let l = lo[k].clamp(0, n - s);
        b.lo[k] = l;
        b.hi[k] = l + s;
    }
    b
}

/// A structure being placed, before acceptance.
pub(crate) struct Placed {
    pub(crate) parts: Vec<SlotBox>,
    pub(crate) oriented: Vec<OPart>,
    pub(crate) cables: Vec<[i32; 4]>,
    /// Pieces that are not plain boxes; their boxes count as taken slots.
    pub(crate) shaped: Vec<(Shaped, SlotBox)>,
    /// Boxes whose slots the structure takes whole (its envelope), whatever it is made of:
    /// L3 refines what is inside (`levels.md` §5W.4.1).
    pub(crate) envelope: Vec<SlotBox>,
    /// An older form swallowed inside, in another material.
    pub(crate) inner: Vec<SlotBox>,
    pub(crate) dark: bool,
    pub(crate) lit: bool,
    /// A passage bored through: axis, first slot, length.
    pub(crate) pierce: Option<(usize, [i32; 3], i32)>,
    /// Windows on a face of a slot box, with no mass behind them.
    pub(crate) windows: Vec<(SlotBox, Dir6)>,
    /// The variant (SV-xx) this is; 0 for the plain support.
    pub(crate) variant: u8,
    /// The form actually used when it is not the one drawn (the slab tower stands in).
    pub(crate) form: Option<u16>,
    /// Slots that are meant to overlap what is there, by kind.
    pub(crate) may_overlap: Overlap,
}

impl Placed {
    pub(crate) fn boxes(parts: Vec<SlotBox>, may_overlap: Overlap) -> Self {
        Self {
            parts,
            oriented: Vec::new(),
            cables: Vec::new(),
            shaped: Vec::new(),
            envelope: Vec::new(),
            inner: Vec::new(),
            dark: false,
            lit: false,
            pierce: None,
            windows: Vec::new(),
            variant: 0,
            form: None,
            may_overlap,
        }
    }
}

/// A piece with a shape of its own, in a box of slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shaped {
    /// A vertical cylinder in the box (a pod, a silo).
    Cyl,
    /// A flight of stairs rising along the direction.
    Stair(Dir4),
    /// A cage: the edges of the box only.
    Cage,
    /// A helical stair round the vertical axis of the box.
    Helix { sense: i8 },
    /// A thin façade on the box's side that touches the host in the direction.
    Facade(Dir6),
    /// A cylinder lying along x or z.
    CylH(Axis),
    /// A ring wall of `n` facets (8, 16, 32 or 64) round the box's vertical axis.
    Ring(u8),
    /// A faceted dome on the box's base.
    Dome,
    /// A cone: a stack of cylinders that narrow upwards, or downwards when inverted.
    Cone { inverted: bool },
    /// A prism whose roof rises towards the direction.
    Wedge(Dir4),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Overlap {
    #[default]
    None,
    /// Parent solid, for the trunk of a collar or the host of an embedded structure.
    Solid,
    /// Parent void, for a structure nested in a hall.
    Void,
    /// Built slots, for two structures in two frames.
    Built,
}

/// The table pose that puts a structure on `b` normal to the field at its centre, if a well
/// bends the field there (GRV-01).
fn field_pose(t: &Tissue, b: SlotBox, size: [i32; 3]) -> Option<(u8, u8)> {
    let pitch = t.lattice.pitch as f64;
    let cell = t.cell.coords * 512;
    let at = |k: usize| {
        let mid = f64::from(b.lo[k]) * pitch + f64::from(size[k]) * pitch / 2.0;
        cell[k] + mid.floor() as i64
    };
    let g = crate::gravity::field_at_metres(t.seed, [at(0), at(1), at(2)]);
    crate::gravity::pose_for_up(g.up)
}

/// Places the bounding size `size` (slots, `[w, d, h]` with `h` the form's long axis) on
/// `support`, using `words` for every choice.
#[allow(clippy::too_many_lines)] // one arm per support
pub(crate) fn place(
    t: &Tissue,
    wd: &World,
    support: usize,
    c: &Candidates,
    size: [i32; 3],
    form: &crate::forms::FormDef,
    words: &[u64; 16],
) -> Option<Placed> {
    let n = t.lattice.n as i32;
    let [w, h, d] = size;
    let muts = mutations_of(t, words);
    let form_at = |b: SlotBox, overlap: Overlap| -> Option<Placed> {
        let mut overlap = overlap;
        let mut b = b;
        // MUT-14: fused with a neighbour: one slice wider where a built structure stands.
        if muts.contains(&14) {
            for (k, s) in [(0, 1), (0, -1), (2, 1), (2, -1)] {
                let at = if s > 0 { b.hi[k] } else { b.lo[k] - 1 };
                let (u, v) = if k == 0 { (2, 1) } else { (0, 1) };
                let mut touches = false;
                let mut clear = true;
                for a in b.lo[u]..b.hi[u] {
                    for c in b.lo[v]..b.hi[v] {
                        let mut q = [0; 3];
                        q[k] = at;
                        q[u] = a;
                        q[v] = c;
                        match wd.state(q) {
                            Some(SlotState::Built) => touches = true,
                            Some(SlotState::Free) => {}
                            _ => clear = false,
                        }
                    }
                }
                if touches && clear {
                    if s > 0 {
                        b.hi[k] += 1;
                    } else {
                        b.lo[k] -= 1;
                    }
                    overlap = Overlap::Built;
                    break;
                }
            }
        }
        let sz = [0, 1, 2].map(|k| b.hi[k] - b.lo[k]);
        let mut rng = Rng::new(words[7]);
        let build_size = if muts.contains(&7) {
            crate::mutations::size_before(7, sz, &mut rng)
        } else {
            sz
        };
        // A form that cannot be made at this size is a slab tower instead, unless it is a hub,
        // a set piece or a local place: those are landmarks and are not placed as a plain slab.
        // The Grid: every structure is the same cube of two slots (STG-04).
        let grid = t.strange == Some(crate::strange::Strange::Grid)
            && form.tag.starts_with('F')
            && build_size.iter().all(|&v| v >= 2);
        let built = if grid {
            crate::forms::FormOut::of_cube(2)
        } else {
            (form.build)(build_size, &mut rng)
        };
        let (out, used) = match built {
            Some(o) => (o, None),
            None if !form.tag.starts_with("FORM") && !form.tag.starts_with("F2") => return None,
            None => (
                (crate::forms::FORMS[0].build)(build_size, &mut rng)?,
                Some(0u16),
            ),
        };
        let mut out = out;
        for &m in &muts {
            out = crate::mutations::apply(out, sz, m, &mut rng);
        }
        // GRV-01: where a well bends the field, floors stand normal to it.
        if t.lattice.frame.is_axis_aligned()
            && let Some((yaw, tilt)) = field_pose(t, b, sz)
        {
            out = crate::mutations::pose(out, sz, yaw, tilt);
        }
        // RARE-12: in a shattered district every box has a frame of its own.
        if t.rare == Some(crate::rare::Rare::Shattered) && t.lattice.frame.is_axis_aligned() {
            out = crate::mutations::shatter(out, &mut rng);
        }
        let out = out.shifted(b.lo);
        let mut placed = Placed::boxes(out.boxes, overlap);
        placed.shaped = out.shaped;
        placed.form = used;
        placed.envelope = vec![b];
        placed.oriented = out.turned.iter().map(|tb| turned_part(t, tb)).collect();
        placed
            .cables
            .extend(out.vines.iter().map(|v| [v[0], v[1], v[2], v[3]]));
        placed.inner = out.inner;
        placed.dark = out.dark;
        placed.lit = out.lit;
        placed.pierce = out.pierce;
        Some(placed)
    };
    let simple = |parts: Vec<SlotBox>, overlap| Some(Placed::boxes(parts, overlap));
    match support {
        STANDING | RADIAL => {
            let dirs = [(0, -1), (0, 1), (2, -1), (2, 1), (1, 1)];
            let b = best_of(&c.stand, words[9], |k| {
                expand(wd, wd.xyz(k), size, &dirs, words[11])
            })?;
            form_at(b, Overlap::None)
        }
        HANGING => {
            let dirs = [(0, -1), (0, 1), (2, -1), (2, 1), (1, -1)];
            let b = best_of(&c.hang, words[9], |k| {
                expand(wd, wd.xyz(k), size, &dirs, words[11])
            })?;
            form_at(b, Overlap::None)
        }
        SUSPENDED => {
            let p = wd.xyz(*c.hang.get(below(words[9], c.hang.len()))?);
            let gap = 1 + below(words[10], 3) as i32;
            let start = add(p, 1, -gap);
            if wd.state(start) != Some(SlotState::Free) {
                return None;
            }
            let b = expand(
                wd,
                start,
                size,
                &[(0, -1), (0, 1), (2, -1), (2, 1), (1, -1)],
                words[11],
            );
            let (cx, cz) = ((b.lo[0] + b.hi[0] - 1) / 2, (b.lo[2] + b.hi[2] - 1) / 2);
            let top = b.hi[1];
            Some(Placed {
                cables: vec![[cx, cz, top, n]],
                ..form_at(b, Overlap::None)?
            })
        }
        SPANNING => {
            let &(pk, axis, len) = c.gaps.get(below(words[9], c.gaps.len()))?;
            let (p, axis, len) = (wd.xyz(pk), usize::from(axis), i32::from(len));
            let cross = [w.min(2), h.min(3)];
            let mut lo = p;
            let mut hi = p;
            hi[axis] = p[axis] + len;
            let other = 2 - axis;
            hi[other] = p[other] + cross[0];
            hi[1] = p[1] + cross[1];
            let b = SlotBox { lo, hi };
            lo = b.lo;
            let _ = lo;
            b.inside(t.lattice.n).then_some(())?;
            simple(vec![b], Overlap::Void)
        }
        SIDEWAYS => {
            let &(pk, axis, s) = c.side.get(below(words[9], c.side.len()))?;
            let (p, axis, s) = (wd.xyz(pk), usize::from(axis), i32::from(s));
            // The form laid down: its height runs along the axis, its depth stands up.
            let other = 2 - axis;
            let mut sz = [0; 3];
            sz[axis] = h;
            sz[other] = w;
            sz[1] = d.min(6);
            let mut lo = [0; 3];
            lo[axis] = if s > 0 { p[axis] } else { p[axis] - h + 1 };
            lo[other] = p[other] - w / 2;
            lo[1] = p[1];
            let _ = (lo, sz);
            let dirs = [(axis, s), (other, -1), (other, 1), (1, 1)];
            let mut caps = [0; 3];
            caps[axis] = h;
            caps[other] = w;
            caps[1] = d.min(6);
            let b = expand(wd, p, caps, &dirs, words[11]);
            simple(vec![b], Overlap::None)
        }
        COLLAR => {
            let p = wd.xyz(*c.trunk.get(below(words[9], c.trunk.len()))?);
            let hh = h.clamp(2, 4);
            let (x, z) = (p[0], p[2]);
            let y0 = p[1];
            let ring = |lo: [i32; 3], hi: [i32; 3]| SlotBox { lo, hi };
            let parts = vec![
                ring([x - 1, y0, z - 1], [x + 2, y0 + hh, z]),
                ring([x - 1, y0, z + 1], [x + 2, y0 + hh, z + 2]),
                ring([x - 1, y0, z], [x, y0 + hh, z + 1]),
                ring([x + 1, y0, z], [x + 2, y0 + hh, z + 1]),
            ];
            parts.iter().all(|b| b.inside(t.lattice.n)).then_some(())?;
            simple(parts, Overlap::None)
        }
        EMBEDDED => {
            let p = wd.xyz(*c.host.get(below(words[9], c.host.len()))?);
            let b = SlotBox {
                lo: p,
                hi: [p[0] + 2, p[1] + 2, p[2] + 2],
            };
            b.inside(t.lattice.n).then_some(())?;
            simple(vec![b], Overlap::Solid)
        }
        PILED => {
            let pool: &[u32] = if c.stand.is_empty() {
                &c.hang
            } else {
                &c.stand
            };
            let p = wd.xyz(*pool.get(below(words[9], pool.len()))?);
            let mut rng = Rng::new(words[12]);
            let count = 3 + rng.below(4) as usize;
            let mut parts: Vec<SlotBox> = Vec::new();
            let mut lo = clamp_box(
                p,
                [1 + (words[10] % 2) as i32, 1 + (words[11] % 2) as i32, 1],
                n,
            );
            parts.push(lo);
            for _ in 1..count {
                let base = parts[rng.below(parts.len() as u64) as usize];
                let k = rng.below(3) as usize;
                let pos = rng.below(2) == 0;
                let sz = [
                    1 + rng.below(2) as i32,
                    1 + rng.below(2) as i32,
                    1 + rng.below(2) as i32,
                ];
                let mut l = base.lo;
                l[k] = if pos { base.hi[k] } else { base.lo[k] - sz[k] };
                lo = SlotBox {
                    lo: l,
                    hi: [l[0] + sz[0], l[1] + sz[1], l[2] + sz[2]],
                };
                if lo.inside(t.lattice.n) {
                    parts.push(lo);
                }
            }
            simple(parts, Overlap::None)
        }
        NESTED => {
            let p = wd.xyz(*c.hall.get(below(words[9], c.hall.len()))?);
            // As large as the hall allows from `p`, at most the form's size.
            let extent = |k: usize, max: i32| {
                (1..=max)
                    .take_while(|&i| wd.state(add(p, k, i - 1)) == Some(SlotState::Void))
                    .count() as i32
            };
            let sz = [extent(0, w), extent(1, h), extent(2, d)];
            if sz.iter().any(|&s| s < 2) {
                return None;
            }
            let b = SlotBox {
                lo: p,
                hi: [p[0] + sz[0], p[1] + sz[1], p[2] + sz[2]],
            };
            simple(vec![b], Overlap::Void)
        }
        COLLIDING => {
            let p = wd.nth_free(below(words[9], c.free))?;
            let yaw = match t.lattice.frame {
                Frame::Table { yaw, .. } => yaw,
                _ => 0,
            };
            let frame = Frame::table((yaw + 8) % 64, 0);
            let pitch = t.lattice.pitch as f64;
            let centre =
                t.lattice.centre(
                    t.lattice
                        .index([p[0] as usize, p[1] as usize, p[2] as usize]),
                );
            let size_m = [
                f64::from(w) * pitch,
                f64::from(h) * pitch,
                f64::from(d) * pitch,
            ];
            let [ex, _, ez] = frame.axes();
            let origin = centre - ex * (size_m[0] / 2.0) - ez * (size_m[2] / 2.0);
            Some(Placed {
                oriented: vec![OPart {
                    frame,
                    origin: origin.to_array(),
                    size: size_m,
                }],
                ..Placed::boxes(Vec::new(), Overlap::Built)
            })
        }
        _ => None,
    }
}

/// A turned box of a form as an oriented part in cell-local metres: turned about the centre of
/// its base.
pub(crate) fn turned_part(t: &Tissue, tb: &crate::forms::Turned) -> OPart {
    let pitch = t.lattice.pitch as f64;
    let axes = t.lattice.frame.axes();
    let b = tb.bx;
    let centre = |k: usize| f64::from(b.lo[k] + b.hi[k]) / 2.0;
    // The pivot in slots: the form's, or the centre of the base of the box.
    let (px, pz) = tb.pivot.map_or((centre(0), centre(2)), |p| (p[0], p[1]));
    let at = |x: f64, y: f64, z: f64| {
        let c = DVec3::new(x * pitch - 256.0, y * pitch - 256.0, z * pitch - 256.0);
        DVec3::splat(256.0) + axes[0] * c.x + axes[1] * c.y + axes[2] * c.z
    };
    let pivot = at(px, f64::from(b.lo[1]), pz);
    let frame = Frame::table(tb.yaw, tb.tilt);
    let [ex, _, ez] = frame.axes();
    let size = [0, 1, 2].map(|k| f64::from(b.hi[k] - b.lo[k]) * pitch);
    let origin =
        pivot + ex * ((f64::from(b.lo[0]) - px) * pitch) + ez * ((f64::from(b.lo[2]) - pz) * pitch);
    OPart {
        frame,
        origin: origin.to_array(),
        size,
    }
}

/// The slots the oriented parts cover: those whose centre lies inside.
pub(crate) fn rasterise(lat: &Lattice, parts: &[OPart]) -> Vec<usize> {
    let mut out = Vec::new();
    for o in parts {
        let axes = o.frame.axes();
        let origin = DVec3::from_array(o.origin);
        for i in 0..lat.len() {
            if !lat.present[i] {
                continue;
            }
            let q = lat.centre(i) - origin;
            let l = [q.dot(axes[0]), q.dot(axes[1]), q.dot(axes[2])];
            if (0..3).all(|k| l[k] >= 0.0 && l[k] < o.size[k]) && !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out
}

/// The cheapest path from `from` to a route slot over slots that structures have not taken.
fn access_path(t: &Tissue, in_route: &[bool], from: usize) -> Option<Vec<usize>> {
    let lat = &t.lattice;
    let cost = |s: SlotState| match s {
        SlotState::Built | SlotState::Engulfed => None,
        SlotState::Solid => Some(8),
        SlotState::Void => Some(2),
        _ => Some(1),
    };
    let mut best = vec![u32::MAX; lat.len()];
    let mut prev = vec![usize::MAX; lat.len()];
    let mut heap = BinaryHeap::new();
    best[from] = 0;
    heap.push(Reverse((0u32, from)));
    let mut end = None;
    while let Some(Reverse((d, i))) = heap.pop() {
        if d > best[i] {
            continue;
        }
        if in_route[i] {
            end = Some(i);
            break;
        }
        for j in lat.neighbours(i) {
            let Some(c) = cost(lat.state[j]) else {
                continue;
            };
            if d + c < best[j] {
                best[j] = d + c;
                prev[j] = i;
                heap.push(Reverse((d + c, j)));
            }
        }
    }
    let mut path = vec![end?];
    while *path.last()? != from {
        path.push(prev[*path.last()?]);
    }
    path.reverse();
    Some(path)
}

/// The weights of the supports for this cell: the character's, or the regime's palette.
fn support_weights(t: &Tissue) -> [u32; 11] {
    let mut w = t.character.map_or([4u8; 11], |c| c.supports).map(u32::from);
    // Radial stands on the surface of a gravity well (SUP-11): drawn only where a well bends
    // the field at the cell's centre.
    let c = t.cell.coords * 512 + 256;
    let g = crate::gravity::field_at_metres(t.seed, [c.x, c.y, c.z]);
    if g == crate::gravity::Gravity::DEFAULT || g.is_lull() {
        w[RADIAL] = 0;
    }
    // RARE-01: in an inverted district everything hangs.
    if t.rare == Some(crate::rare::Rare::Inverted) {
        let hang = w[HANGING].max(4) * 8;
        w = [0; 11];
        w[HANGING] = hang;
        w[SUSPENDED] = 2;
    }
    w
}

fn form_weights(t: &Tissue) -> [u32; FAMILIES] {
    t.character
        .map_or([4u8; FAMILIES], |c| c.forms)
        .map(u32::from)
}

/// Step 3. Returns when the cell reaches its fill, after `MAX_ATTEMPTS` attempts or after
/// `MAX_REJECTS` rejections in a row.
#[allow(clippy::too_many_lines)] // one attempt, read top to bottom
pub fn grow(t: &mut Tissue, rng: &mut Rng, _weave: u8) {
    let present = t.lattice.present.iter().filter(|&&p| p).count();
    let target = (present as u64 * u64::from(t.fill) / 255) as usize;
    // Parent solid (plates, beams, tethers) is built matter and counts towards the fill.
    let mut built = t
        .lattice
        .state
        .iter()
        .zip(&t.lattice.present)
        .filter(|(s, p)| **p && **s == SlotState::Solid)
        .count();
    let mut rejects = 0;
    let mut in_route = vec![false; t.lattice.len()];
    for &i in &t.route {
        in_route[i as usize] = true;
    }
    let mut seg_map: Vec<Vec<u16>> = vec![Vec::new(); t.lattice.len()];
    for (k, s) in t.segments.iter().enumerate() {
        for &i in &s.slots {
            seg_map[i as usize].push(k as u16);
        }
    }
    let mut world = World::new(t);
    let mut cands = candidates(&world);
    let mut dirty = false;
    // Hubs first (the draws are always made): the knot's, and a contextual one with chance 1/4.
    let hub_words: [[u64; 16]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| rng.next_u64()));
    // Fresh words for the knot's hub when it did not fit (PW.3.15c).
    let retry_words: [[u64; 16]; HUB_RETRIES] =
        std::array::from_fn(|_| std::array::from_fn(|_| rng.next_u64()));
    let (pick_word, chance_word) = (rng.next_u64(), rng.next_u64());
    // The kinds each wanted hub may take, in the order they are tried: the drawn one first.
    let mut wanted: Vec<Vec<usize>> = Vec::new();
    if let Some(f) = t.knot_hub {
        let kinds = crate::hubs::knot_kinds(f);
        let ws: Vec<u32> = kinds.iter().map(|k| k.1).collect();
        let first = pick(pick_word, &ws).unwrap_or(0);
        wanted.push(
            (0..kinds.len())
                .map(|k| kinds[(first + k) % kinds.len()].0)
                .collect(),
        );
    }
    if let Some(kind) = crate::hubs::contextual(t)
        && chance_word % 4 == 0
    {
        wanted.push(vec![kind]);
    }
    for (i, kinds) in wanted.into_iter().enumerate() {
        // The knot's hub is tried again with fresh words, and with the next kind of its family
        // where it has one; a contextual hub is tried once.
        let tries = if i == 0 && t.knot_hub.is_some() {
            1 + HUB_RETRIES
        } else {
            1
        };
        for r in 0..tries {
            if dirty {
                world = World::new(t);
                cands = candidates(&world);
                dirty = false;
            }
            let words = if r == 0 {
                &hub_words[i]
            } else {
                &retry_words[r - 1]
            };
            let h = kinds[r % kinds.len()];
            if let Some(s) = attempt(
                t,
                words,
                0,
                &mut in_route,
                &seg_map,
                &world,
                &cands,
                Some(crate::hubs::HUB_BASE + h),
            ) {
                built += s.slots.len();
                if (crate::hubs::HUB_BASE..crate::sets::SET_BASE).contains(&usize::from(s.form)) {
                    t.hubs.push((h as u8 + 1, t.structures.len() as u16));
                }
                t.structures.push(s);
                dirty = true;
                break;
            }
        }
    }
    // Set pieces: none (1/2), one (1/4) or two (1/4), of the kinds the cell's context allows.
    let set_words: [[u64; 16]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| rng.next_u64()));
    let (count_word, kind_words) = (rng.next_u64(), [rng.next_u64(), rng.next_u64()]);
    let legal = crate::sets::legal(t);
    let ws: Vec<u32> = legal.iter().map(|k| k.1).collect();
    let n_sets = match count_word % 4 {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    };
    for i in 0..n_sets {
        let Some(k) = pick(kind_words[i], &ws) else {
            continue;
        };
        let kind = legal[k].0;
        if dirty {
            world = World::new(t);
            cands = candidates(&world);
            dirty = false;
        }
        if let Some(s) = attempt(
            t,
            &set_words[i],
            target.saturating_sub(built).max(1),
            &mut in_route,
            &seg_map,
            &world,
            &cands,
            Some(crate::sets::SET_BASE + kind),
        ) {
            built += s.slots.len();
            if (crate::sets::SET_BASE..crate::locals::LOC_BASE).contains(&usize::from(s.form)) {
                t.sets.push((kind as u8 + 1, t.structures.len() as u16));
            }
            t.structures.push(s);
            dirty = true;
        }
    }
    // A local place with chance 1/4, of the kinds the context allows.
    let loc_words: [u64; 16] = std::array::from_fn(|_| rng.next_u64());
    let (loc_chance, loc_kind) = (rng.next_u64(), rng.next_u64());
    let loc_legal = crate::locals::legal(t);
    let loc_ws: Vec<u32> = loc_legal.iter().map(|k| k.1).collect();
    if loc_chance % 4 == 0
        && let Some(k) = pick(loc_kind, &loc_ws)
    {
        let kind = loc_legal[k].0;
        if dirty {
            world = World::new(t);
            cands = candidates(&world);
            dirty = false;
        }
        if let Some(s) = attempt(
            t,
            &loc_words,
            target.saturating_sub(built).max(1),
            &mut in_route,
            &seg_map,
            &world,
            &cands,
            Some(crate::locals::LOC_BASE + kind),
        ) {
            built += s.slots.len();
            if usize::from(s.form) >= crate::locals::LOC_BASE {
                t.locals.push((kind as u8 + 1, t.structures.len() as u16));
            }
            t.structures.push(s);
            dirty = true;
        }
    }
    for _ in 0..MAX_ATTEMPTS {
        let words: [u64; 16] = std::array::from_fn(|_| rng.next_u64());
        if built >= target || rejects >= MAX_REJECTS {
            // The draws are made even when growth is over, so that nothing after this depends
            // on where it stopped.
            continue;
        }
        let remaining = target - built;
        if dirty {
            world = World::new(t);
            cands = candidates(&world);
            dirty = false;
        }
        match attempt(
            t,
            &words,
            remaining,
            &mut in_route,
            &seg_map,
            &world,
            &cands,
            None,
        ) {
            Some(s) => {
                built += s.slots.len();
                t.structures.push(s);
                rejects = 0;
                dirty = true;
            }
            None => rejects += 1,
        }
    }
}

/// Places a structure of at most `size` and checks that it can stand: the slots it takes
/// must be free (or deliberately overlapped), and it must lean on something. On a conflict
/// the largest dimension shrinks by one slot and the structure is placed again, so a
/// structure fits itself to the room between the routes.
#[allow(clippy::type_complexity)]
fn fit(
    t: &Tissue,
    w: &World,
    support: usize,
    variant: u8,
    cands: &Candidates,
    mut size: [i32; 3],
    min: [i32; 3],
    form: &crate::forms::FormDef,
    words: &[u64; 16],
    seg_map: &[Vec<u16>],
) -> Result<(Placed, Vec<usize>, Vec<usize>), usize> {
    let mut last = 20 + support;
    for _ in 0..24 {
        match fit_once(t, w, support, variant, cands, size, form, words, seg_map) {
            Ok(r) => return Ok(r),
            Err(code) => last = code,
        }
        // Shrinks the axis with the most to give; a hub stops at the least its builder needs.
        let Some(k) = (0..3)
            .filter(|&k| size[k] > min[k].max(1))
            .max_by_key(|&k| size[k] - min[k])
        else {
            break;
        };
        size[k] -= 1;
    }
    Err(last)
}

#[allow(clippy::type_complexity)]
fn fit_once(
    t: &Tissue,
    w: &World,
    support: usize,
    variant: u8,
    cands: &Candidates,
    size: [i32; 3],
    form: &crate::forms::FormDef,
    words: &[u64; 16],
    seg_map: &[Vec<u16>],
) -> Result<(Placed, Vec<usize>, Vec<usize>), usize> {
    let n = t.lattice.n;
    // A variant that cannot be placed here falls back to the plain support.
    let placed = (variant != 0)
        .then(|| {
            crate::tissue_variants::place_variant(t, w, support, variant, cands, size, form, words)
        })
        .flatten()
        .or_else(|| place(t, w, support, cands, size, form, words));
    let Some(placed) = placed else {
        return Err(20 + support);
    };

    // Rasterise and accept.
    let mut slots: Vec<usize> = Vec::new();
    for b in &placed.parts {
        if !b.inside(n) {
            return Err(1);
        }
        slots.extend(b.slots(&t.lattice));
    }
    for (_, b) in &placed.shaped {
        if !b.inside(n) {
            return Err(1);
        }
        slots.extend(b.slots(&t.lattice));
    }
    for b in &placed.envelope {
        if !b.inside(n) {
            return Err(1);
        }
        slots.extend(b.slots(&t.lattice));
    }
    slots.extend(rasterise(&t.lattice, &placed.oriented));
    slots.sort_unstable();
    slots.dedup();
    if slots.is_empty() && placed.windows.is_empty() {
        return Err(2);
    }
    // Route slots a structure may swallow: those whose every segment is engulfable.
    let segs_of =
        |slot: usize| -> Vec<usize> { seg_map[slot].iter().map(|&k| usize::from(k)).collect() };
    let mut swallowed: Vec<usize> = Vec::new();
    for &i in &slots {
        if !t.lattice.present[i] {
            return Err(3);
        }
        match t.lattice.state[i] {
            SlotState::Free => {}
            SlotState::Solid if placed.may_overlap == Overlap::Solid => {}
            SlotState::Void if placed.may_overlap == Overlap::Void => {}
            SlotState::Built if placed.may_overlap == Overlap::Built => {}
            SlotState::Route => {
                if t.anchors.iter().any(|a| a.slot as usize == i) {
                    return Err(4);
                }
                let segs = segs_of(i);
                if segs.is_empty() || !segs.iter().all(|&s| t.segments[s].engulfable) {
                    return Err(5);
                }
                swallowed.push(i);
            }
            _ => return Err(6),
        }
    }
    // The structure must lean on something (a shell counts inside a crust).
    {
        let leans = slots.iter().any(|&i| {
            let c = t.lattice.coords(i).map(|v| v as i32);
            (0..3).any(|k| {
                [-1, 1].iter().any(|&s| {
                    w.support(add(c, k, s), c, true)
                        || matches!(w.state(add(c, k, s)), Some(SlotState::Route))
                })
            })
        });
        if !leans && placed.may_overlap == Overlap::None && !slots.is_empty() {
            return Err(7);
        }
    }

    Ok((placed, slots, swallowed))
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)] // one attempt, read top to bottom
fn attempt(
    t: &mut Tissue,
    words: &[u64; 16],
    remaining: usize,
    in_route: &mut [bool],
    seg_map: &[Vec<u16>],
    w: &World,
    cands: &Candidates,
    hub: Option<usize>,
) -> Option<Structure> {
    // Support: only those with somewhere to start.
    let mut sw = support_weights(t);
    let has = [
        !cands.stand.is_empty(),
        !cands.hang.is_empty(),
        !cands.gaps.is_empty(),
        !cands.side.is_empty(),
        !cands.trunk.is_empty(),
        !cands.host.is_empty(),
        !cands.hang.is_empty(),
        !cands.stand.is_empty() || !cands.hang.is_empty(),
        !cands.hall.is_empty(),
        cands.free > 0,
        false,
    ];
    for (k, w) in sw.iter_mut().enumerate() {
        if !has[k] {
            *w = 0;
        }
    }
    if sw.iter().all(|&x| x == 0) {
        // Every offered support is banned by the stratum: the universal ones still work.
        for (k, w) in sw.iter_mut().enumerate() {
            *w = u32::from(has[k] && k != RADIAL);
        }
    }
    let Some(support) = pick(words[0], &sw) else {
        return rej(19);
    };
    let family = pick(words[1], &form_weights(t)).unwrap_or(0);
    let mut pool = members(family);
    if pool.is_empty() {
        pool = (0..FORMS.len()).collect();
    }
    // A hub is a form of its own, plain, whatever the fill.
    let (form, form_idx, mutations) = if let Some(id) = hub {
        // A hub (100 + n) or a set piece (200 + n) is a form of its own, plain.
        let def = if id >= crate::locals::LOC_BASE {
            &crate::locals::LOCALS[id - crate::locals::LOC_BASE]
        } else if id >= crate::sets::SET_BASE {
            &crate::sets::SETS[id - crate::sets::SET_BASE]
        } else {
            &HUBS[id - crate::hubs::HUB_BASE]
        };
        (def, id, Vec::new())
    } else {
        let i = pool[below(words[2], pool.len())];
        (&FORMS[i], i, mutations_of(t, words))
    };
    // Size in slots from the form's range in metres, shrunk to what is left to build.
    let pitch = t.lattice.pitch as i32;
    let (num, den) = t.character.map_or((1, 1), |c| c.scale_ratio());
    let dim = |k: usize, word: u64| {
        let (lo, hi) = (form.size_m.0[k], form.size_m.1[k]);
        // Dense strata draw their structures large: the unit draw is raised to
        // `1 / (1 + 4 · field)`, which pushes it towards the top of the range. The power is
        // built from square roots (one per binary digit of the exponent, in 64ths) because
        // `sqrt` is exact on every machine and `powf` is not.
        let u = ((word >> 11) as f64) / ((1u64 << 53) as f64);
        let e64 = (64.0 / (1.0 + 4.0 * f64::from(t.fill) / 255.0)).round() as u32;
        let u = if e64 >= 64 {
            u
        } else {
            let (mut root, mut acc) = (u, 1.0);
            for bit in (0..6).rev() {
                root = root.sqrt();
                if e64 >> bit & 1 == 1 {
                    acc *= root;
                }
            }
            acc
        };
        let m = lo + (f64::from(hi - lo) * u).round() as i32;
        let m = m * i32::from(num) / i32::from(den);
        ((m + pitch - 1) / pitch).max(1)
    };
    let mut size = [dim(0, words[6]), dim(2, words[8]), dim(1, words[7])];
    // MUT-13: scaled, a quarter of the size or twice it.
    if mutations.contains(&13) {
        let n = t.lattice.n as i32;
        for v in &mut size {
            *v = if words[15] & 1 == 0 {
                (*v / 4).max(1)
            } else {
                (*v * 2).min(n)
            };
        }
    }
    // Hubs are placed whatever the fill; set pieces and local places stay within it.
    let cap = if hub.is_some_and(|id| id < crate::sets::SET_BASE) {
        usize::MAX
    } else {
        (remaining + remaining / 8).max(1)
    };
    while size.iter().map(|&s| s as usize).product::<usize>() > cap {
        let k = (0..3).max_by_key(|&k| size[k]).unwrap_or(0);
        if size[k] <= 1 {
            break;
        }
        size[k] -= 1;
    }
    // A hub keeps at least the size its builder needs (the low end of its range).
    let min = if hub.is_some_and(|id| id < crate::sets::SET_BASE) {
        let lo = |k: usize| {
            let m = form.size_m.0[k] * i32::from(num) / i32::from(den);
            ((m + pitch - 1) / pitch).max(1)
        };
        [lo(0), lo(2), lo(1)]
    } else {
        [1; 3]
    };
    let variants = crate::tissue_variants::variants_of(support);
    // Three in four structures are plain; the rest share the variants equally.
    let want = if hub.is_some() || words[10] & 3 != 0 || variants.len() < 2 {
        0
    } else {
        variants[1 + below(words[10] >> 8, variants.len() - 1)]
    };
    let (placed, slots, swallowed) =
        match fit(t, w, support, want, cands, size, min, form, words, seg_map) {
            Ok(r) => r,
            Err(code) => return rej(code),
        };
    let variant = placed.variant;
    let form_idx = placed.form.map_or(form_idx, usize::from);
    if slots.is_empty() {
        // Windows with no mass behind them: nothing to take and nothing to enter.
        return Some(Structure {
            support: support as u8,
            form: form_idx as u16,
            variant,
            mutations,
            parts: Vec::new(),
            oriented: Vec::new(),
            cables: Vec::new(),
            shaped: Vec::new(),
            windows: placed.windows,
            inner: Vec::new(),
            dark: false,
            lit: false,
            slots: Vec::new(),
            entrances: Vec::new(),
            sealed: true,
        });
    }
    let segs_of =
        |slot: usize| -> Vec<usize> { seg_map[slot].iter().map(|&k| usize::from(k)).collect() };

    // Commit tentatively, find entrances, and undo on failure.
    let before: Vec<(usize, SlotState)> = slots.iter().map(|&i| (i, t.lattice.state[i])).collect();
    for &i in &slots {
        t.lattice.state[i] = if t.lattice.state[i] == SlotState::Route {
            SlotState::Engulfed
        } else {
            SlotState::Built
        };
    }
    let mut route_added: Vec<usize> = Vec::new();
    let mut entrances: Vec<Entrance> = Vec::new();
    let mut ok = true;
    // Entrances of swallowed segments: both route slots just outside.
    let mut engulfed_segments: Vec<u16> = Vec::new();
    for &i in &swallowed {
        for s in segs_of(i) {
            if engulfed_segments.contains(&(s as u16)) {
                continue;
            }
            engulfed_segments.push(s as u16);
            let seg = &t.segments[s];
            let inside = |slot: u32| t.lattice.state[slot as usize] == SlotState::Engulfed;
            for (a, b) in seg.slots.windows(2).map(|w| (w[0], w[1])) {
                for (out, inn) in [(a, b), (b, a)] {
                    if !inside(out) && inside(inn) {
                        entrances.push(Entrance {
                            slot: out,
                            into: dir_between(&t.lattice, out as usize, inn as usize),
                            engulf: true,
                        });
                    }
                }
            }
        }
    }
    if !form.sealed {
        // One entrance per 30 m of footprint, up to 8.
        let foot_m: i64 = {
            let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
            for &i in &slots {
                let c = t.lattice.coords(i);
                for k in 0..3 {
                    lo[k] = lo[k].min(c[k] as i32);
                    hi[k] = hi[k].max(c[k] as i32 + 1);
                }
            }
            i64::from((hi[0] - lo[0]) + (hi[2] - lo[2])) * t.lattice.pitch
        };
        let want = (((foot_m + 29) / 30) as usize).clamp(1, 8);
        // Outside neighbours: free or route slots next to the structure.
        let mut outs: Vec<(usize, Dir6)> = Vec::new();
        for &i in &slots {
            let c = t.lattice.coords(i).map(|v| v as i32);
            for k in 0..3 {
                for pos in [false, true] {
                    let d = if pos { 1 } else { -1 };
                    let o = add(c, k, d);
                    // `w` is the snapshot from before this structure: read the slot itself.
                    let n = t.lattice.n as i32;
                    if (0..3).all(|q| (0..n).contains(&o[q])) {
                        let j = t.lattice.index(o.map(|v| v as usize));
                        if t.lattice.present[j]
                            && matches!(t.lattice.state[j], SlotState::Free | SlotState::Route)
                        {
                            outs.push((j, dir_of(k, !pos)));
                        }
                    }
                }
            }
        }
        outs.sort_unstable_by_key(|&(j, d)| (j, d as u8));
        outs.dedup();
        // Entrances on slots that are already route first; a new walk only for the first
        // entrance, or when it is short, so that coarse lattices keep their free slots.
        let mut order: Vec<(usize, Dir6)> =
            outs.iter().copied().filter(|&(j, _)| in_route[j]).collect();
        let mut word = words[13];
        for _ in 0..24.min(outs.len()) {
            order.push(outs[below(word, outs.len())]);
            word = word.rotate_left(17) ^ words[14].wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        let mut got = 0;
        for (j, d) in order {
            if got >= want {
                break;
            }
            if entrances.iter().any(|e| e.slot as usize == j && !e.engulf) {
                continue;
            }
            // With no route yet (a cell off the skeleton), the first entrance starts one.
            let path = if in_route[j] || t.route.is_empty() {
                Some(vec![j])
            } else {
                access_path(t, in_route, j).filter(|p| got == 0 || p.len() <= 4)
            };
            let Some(path) = path else { continue };
            for &p in &path {
                if !in_route[p] {
                    in_route[p] = true;
                    route_added.push(p);
                    if t.lattice.state[p] == SlotState::Free {
                        t.lattice.state[p] = SlotState::Route;
                    }
                    t.route.push(p as u32);
                }
            }
            entrances.push(Entrance {
                slot: j as u32,
                into: d,
                engulf: false,
            });
            got += 1;
        }
        ok = got > 0;
    }
    // MUT-10: the ends of a bored passage are entrances too, when a route can reach them.
    if let (true, Some((axis, start, len))) = (ok, placed.pierce) {
        {
            let n = t.lattice.n as i32;
            for s in [-1, len] {
                let mut q = start;
                q[axis] += s;
                if !(0..3).all(|k| (0..n).contains(&q[k])) {
                    continue;
                }
                let j = t.lattice.index(q.map(|v| v as usize));
                let open = t.lattice.present[j]
                    && matches!(t.lattice.state[j], SlotState::Free | SlotState::Route);
                if !open || entrances.iter().any(|e| e.slot as usize == j) {
                    continue;
                }
                let path = if in_route[j] {
                    Some(vec![j])
                } else {
                    access_path(t, in_route, j)
                };
                let Some(path) = path else { continue };
                for &p in &path {
                    if !in_route[p] {
                        in_route[p] = true;
                        route_added.push(p);
                        if t.lattice.state[p] == SlotState::Free {
                            t.lattice.state[p] = SlotState::Route;
                        }
                        t.route.push(p as u32);
                    }
                }
                entrances.push(Entrance {
                    slot: j as u32,
                    into: dir_of(axis, s < 0),
                    engulf: false,
                });
            }
        }
    }
    if !ok || (entrances.is_empty() && !form.sealed) {
        // Undo: the structure, the access routes and the engulfed marks.
        for (i, s) in before {
            t.lattice.state[i] = s;
        }
        for &p in &route_added {
            in_route[p] = false;
            if t.lattice.state[p] == SlotState::Route {
                t.lattice.state[p] = SlotState::Free;
            }
            t.route.retain(|&r| r as usize != p);
        }
        return rej(8);
    }
    t.engulfed.extend(engulfed_segments);
    Some(Structure {
        support: support as u8,
        form: form_idx as u16,
        variant,
        mutations,
        parts: placed.parts,
        oriented: placed.oriented,
        cables: placed.cables,
        shaped: placed.shaped,
        windows: placed.windows,
        inner: placed.inner,
        dark: placed.dark,
        lit: placed.lit,
        slots: slots.iter().map(|&i| i as u32).collect(),
        entrances,
        sealed: form.sealed,
    })
}

fn dir_between(lat: &Lattice, from: usize, to: usize) -> Dir6 {
    let (a, b) = (lat.coords(from), lat.coords(to));
    for k in 0..3 {
        if a[k] != b[k] {
            return dir_of(k, b[k] > a[k]);
        }
    }
    Dir6::PosX
}

impl Structure {
    /// The structure's pieces in cell-local quanta.
    pub fn primitives(&self, lat: &Lattice) -> Vec<Primitive> {
        self.primitives_with(lat, &|_| None)
    }

    /// [`Structure::primitives`] where `interior(part)` is the entrances of a part that L3
    /// fills (§6W): such a part is a hollow shell with its doors and hatches cut, flagged
    /// `INTERIOR`.
    pub fn primitives_with(
        &self,
        lat: &Lattice,
        interior: &dyn Fn(usize) -> Option<Vec<crate::l3_plan::Entrance>>,
    ) -> Vec<Primitive> {
        let mut e = Emitter::new(lat);
        let flags = PrimFlags::ENVELOPE | PrimFlags::FAR_VISIBLE;
        let material = if self.dark {
            MaterialId::METAL
        } else {
            MaterialId::CONCRETE
        };
        for (i, b) in self.parts.iter().enumerate() {
            match interior(i) {
                Some(ents) => {
                    let cuts: Vec<_> = ents
                        .iter()
                        .map(|en| crate::l3_web::entrance_cut(en, b))
                        .collect();
                    e.interior_shell(b, material, flags | PrimFlags::INTERIOR, &cuts);
                }
                None => e.slot_box(b, material, FeatureKind::MASS, flags),
            }
        }
        for b in &self.inner {
            e.slot_box(b, MaterialId::METAL, FeatureKind::MASS, PrimFlags::NONE);
        }
        if self.lit {
            // Only boxes have walls to put windows on (a cylinder fills its box's corners with air).
            for b in &self.parts {
                e.lit_faces(b);
            }
        }
        for p in &self.oriented {
            e.oriented_box(p, flags);
        }
        for (shape, b) in &self.shaped {
            match *shape {
                Shaped::Cyl => e.cylinder(b, flags),
                Shaped::Stair(rise) => e.stair(b, rise),
                Shaped::Cage => e.cage(b),
                Shaped::Helix { sense } => e.helix(b, sense),
                Shaped::Facade(host) => e.facade(b, host, flags),
                Shaped::CylH(axis) => e.cylinder_lying(b, axis, flags),
                Shaped::Ring(n) => e.ring(b, n, flags),
                Shaped::Dome => e.dome(b, flags),
                Shaped::Cone { inverted } => e.cone(b, inverted, flags),
                Shaped::Wedge(rise) => e.wedge(b, rise, flags),
            }
        }
        for &[x, z, y0, y1] in &self.cables {
            e.cable(x, z, y0, y1);
        }
        for (b, facing) in &self.windows {
            e.windows(b, *facing);
        }
        // Pieces of a turned lattice that fall wholly outside the cell are dropped.
        let e_q = 512 * Q_PER_M;
        e.out.retain(|p| {
            let b = p.aabb();
            b.max.x > 0
                && b.min.x < e_q
                && b.max.y > 0
                && b.min.y < e_q
                && b.max.z > 0
                && b.min.z < e_q
        });
        e.out
    }
}

/// `a` minus `c`: up to six boxes (lattice-frame metres).
fn subtract_box(a: ([f64; 3], [f64; 3]), c: ([f64; 3], [f64; 3])) -> Vec<([f64; 3], [f64; 3])> {
    let (mut lo, mut hi) = a;
    if (0..3).any(|k| c.0[k] >= hi[k] || c.1[k] <= lo[k]) {
        return vec![a];
    }
    let mut out = Vec::new();
    for k in 0..3 {
        if c.0[k] > lo[k] {
            let mut h = hi;
            h[k] = c.0[k];
            out.push((lo, h));
            lo[k] = c.0[k];
        }
        if c.1[k] < hi[k] {
            let mut l = lo;
            l[k] = c.1[k];
            out.push((l, hi));
            hi[k] = c.1[k];
        }
    }
    out
}

/// Turns slot geometry into cell-local primitives.
pub(crate) struct Emitter<'a> {
    pub(crate) lat: &'a Lattice,
    axes: [DVec3; 3],
    pub(crate) cell: QAabb,
    pub(crate) out: Vec<Primitive>,
}

pub(crate) fn quanta(v: f64) -> Q {
    (v * f64::from(Q_PER_M)).round() as Q
}

/// How far the shell of an interior part stands inside the part's box: one quantum.
pub(crate) const SHELL_INSET: f64 = 0.0625;

impl<'a> Emitter<'a> {
    pub(crate) fn new(lat: &'a Lattice) -> Self {
        Self {
            lat,
            axes: lat.frame.axes(),
            cell: QAabb::new(
                QVec3::new(0, 0, 0),
                QVec3::new(512 * Q_PER_M, 512 * Q_PER_M, 512 * Q_PER_M),
            ),
            out: Vec::new(),
        }
    }

    /// The cell-local point (metres) of lattice-frame metres `f` from the cell's lattice corner.
    pub(crate) fn at(&self, f: [f64; 3]) -> DVec3 {
        let c = DVec3::new(f[0] - 256.0, f[1] - 256.0, f[2] - 256.0);
        DVec3::splat(256.0) + self.axes[0] * c.x + self.axes[1] * c.y + self.axes[2] * c.z
    }

    fn corner(&self, c: [i32; 3]) -> [f64; 3] {
        let p = self.lat.pitch as f64;
        [
            f64::from(c[0]) * p,
            f64::from(c[1]) * p,
            f64::from(c[2]) * p,
        ]
    }

    pub(crate) fn qv(v: DVec3) -> QVec3 {
        QVec3::new(quanta(v.x), quanta(v.y), quanta(v.z))
    }

    /// A box between lattice-frame metres `lo` and `hi`.
    pub(crate) fn frame_box(
        &mut self,
        lo: [f64; 3],
        hi: [f64; 3],
        material: MaterialId,
        kind: FeatureKind,
        flags: PrimFlags,
    ) {
        if kind == FeatureKind::RAILING && !crate::plumb::is_upright(self.lat.frame) {
            self.plumb_rail(lo, hi, material, flags);
            return;
        }
        let origin = self.at(lo);
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        if size.iter().any(|&s| quanta(s) <= 0) {
            return;
        }
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(quanta(size[0]), quanta(size[1]), quanta(size[2])),
        );
        self.out.push(Primitive {
            shape: Shape::oriented(
                self.lat.frame,
                Self::qv(origin),
                LocalShape::Box { aabb: local },
                self.cell,
            ),
            material,
            kind,
            flags,
        });
    }

    /// A rail of a tilted lattice, rebuilt to stand plumb (`plumb.rs`); none in a lattice whose
    /// floors are not walkable.
    fn plumb_rail(&mut self, lo: [f64; 3], hi: [f64; 3], material: MaterialId, flags: PrimFlags) {
        let Some((upright, _, _)) = crate::plumb::upright(self.lat.frame) else {
            return;
        };
        let origin = self.at([0.0; 3]);
        let v = |a: [f64; 3]| DVec3::new(a[0], a[1], a[2]);
        for b in crate::plumb::plumb_box(self.lat.frame, origin, v(lo), v(hi), 0.5) {
            let size = b.hi - b.lo;
            if [size.x, size.y, size.z].iter().any(|&s| quanta(s) <= 0) {
                continue;
            }
            let local = LocalShape::Box {
                aabb: QAabb::new(
                    QVec3::new(0, 0, 0),
                    QVec3::new(quanta(size.x), quanta(size.y), quanta(size.z)),
                ),
            };
            self.out.push(Primitive {
                shape: Shape::oriented(upright, Self::qv(b.corner), local, self.cell),
                material,
                kind: FeatureKind::RAILING,
                flags,
            });
        }
    }

    /// A part as a hollow shell 0.5 m thick (six slabs) with `cuts` (lattice-frame boxes in
    /// metres) taken out of it.
    fn interior_shell(
        &mut self,
        b: &SlotBox,
        material: MaterialId,
        flags: PrimFlags,
        cuts: &[([f64; 3], [f64; 3])],
    ) {
        const T: f64 = 0.5;
        // The shell stands one quantum (6 cm) inside the part's box, so that none of its faces
        // lies in the plane of a face of the L3 block it hides (PW.4.18): the block's outer
        // walls, floor and roof are flush with the box.
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let (lo, hi) = (lo.map(|v| v + SHELL_INSET), hi.map(|v| v - SHELL_INSET));
        let slab = |l: [f64; 3], h: [f64; 3]| (l, h);
        let slabs = [
            slab(lo, [lo[0] + T, hi[1], hi[2]]),
            slab([hi[0] - T, lo[1], lo[2]], hi),
            slab([lo[0] + T, lo[1], lo[2]], [hi[0] - T, hi[1], lo[2] + T]),
            slab([lo[0] + T, lo[1], hi[2] - T], [hi[0] - T, hi[1], hi[2]]),
            slab(
                [lo[0] + T, lo[1], lo[2] + T],
                [hi[0] - T, lo[1] + T, hi[2] - T],
            ),
            slab(
                [lo[0] + T, hi[1] - T, lo[2] + T],
                [hi[0] - T, hi[1], hi[2] - T],
            ),
        ];
        // The part's whole box, for the mid band, which does not draw shells.
        self.frame_box(
            self.corner(b.lo),
            self.corner(b.hi),
            material,
            FeatureKind::MASS,
            PrimFlags::MID_ONLY | PrimFlags::NO_COLLIDE,
        );
        for s in slabs {
            let mut pieces = vec![s];
            for c in cuts {
                pieces = pieces
                    .into_iter()
                    .flat_map(|p| subtract_box(p, *c))
                    .collect();
            }
            for (l, h) in pieces {
                self.frame_box(l, h, material, FeatureKind::MASS, flags);
            }
        }
    }

    fn slot_box(&mut self, b: &SlotBox, material: MaterialId, kind: FeatureKind, flags: PrimFlags) {
        self.frame_box(self.corner(b.lo), self.corner(b.hi), material, kind, flags);
    }

    fn oriented_box(&mut self, p: &OPart, flags: PrimFlags) {
        if p.size.iter().any(|&v| v <= 0.0) {
            return;
        }
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(quanta(p.size[0]), quanta(p.size[1]), quanta(p.size[2])),
        );
        let o = QVec3::new(
            quanta(p.origin[0]),
            quanta(p.origin[1]),
            quanta(p.origin[2]),
        );
        self.out.push(Primitive {
            shape: Shape::oriented(p.frame, o, LocalShape::Box { aabb: local }, self.cell),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags,
        });
    }

    fn cylinder(&mut self, b: &SlotBox, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let r = ((hi[0] - lo[0]).min(hi[2] - lo[2])) / 2.0;
        let base = QVec3::new(
            quanta((hi[0] - lo[0]) / 2.0),
            0,
            quanta((hi[2] - lo[2]) / 2.0),
        );
        let local = LocalShape::Cylinder {
            axis: crate::cell::Axis::Y,
            base,
            length: quanta(hi[1] - lo[1]),
            radius: quanta(r),
        };
        self.out.push(Primitive {
            shape: Shape::oriented(self.lat.frame, Self::qv(self.at(lo)), local, self.cell),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags,
        });
    }

    fn stair(&mut self, b: &SlotBox, rise: Dir4) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(
                quanta(hi[0] - lo[0]),
                quanta(hi[1] - lo[1]),
                quanta(hi[2] - lo[2]),
            ),
        );
        self.out.push(Primitive {
            shape: Shape::oriented(
                self.lat.frame,
                Self::qv(self.at(lo)),
                LocalShape::StairRun {
                    aabb: local,
                    rise,
                    steps: 32,
                },
                self.cell,
            ),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        });
    }

    /// The twelve edges of the box, 1 m thick.
    fn cage(&mut self, b: &SlotBox) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let t = 1.0;
        for k in 0..3 {
            let (u, v) = ((k + 1) % 3, (k + 2) % 3);
            for (a, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let mut l = lo;
                let mut h = hi;
                for (axis, side) in [(u, a), (v, c)] {
                    if side == 0 {
                        h[axis] = lo[axis] + t;
                    } else {
                        l[axis] = hi[axis] - t;
                    }
                }
                self.frame_box(l, h, MaterialId::METAL, FeatureKind::MASS, PrimFlags::NONE);
            }
        }
        // A floor every 8 m.
        let mut y = lo[1] + 8.0;
        while y < hi[1] {
            self.frame_box(
                [lo[0], y, lo[2]],
                [hi[0], y + 0.5, hi[2]],
                MaterialId::GRATING,
                FeatureKind::MASS,
                PrimFlags::NONE,
            );
            y += 8.0;
        }
    }

    fn helix(&mut self, b: &SlotBox, sense: i8) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let centre = self.at([(lo[0] + hi[0]) / 2.0, lo[1], (lo[2] + hi[2]) / 2.0]);
        let steps = ((hi[1] - lo[1]) * f64::from(Q_PER_M) / 7.0).min(256.0) as u32;
        let h = crate::orient::HelixStair {
            axis: Self::qv(centre),
            inner: quanta(16.0),
            width: quanta(8.0),
            per_turn: 16,
            steps,
            rise: 7,
            thick: 4,
            start: 0,
            sense,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        };
        self.out.extend(crate::orient::helix_stair(&h, self.cell));
    }

    fn cylinder_lying(&mut self, b: &SlotBox, axis: Axis, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let r = if axis == Axis::X {
            size[1].min(size[2])
        } else {
            size[0].min(size[1])
        } / 2.0;
        let (base, length) = if axis == Axis::X {
            (
                QVec3::new(0, quanta(size[1] / 2.0), quanta(size[2] / 2.0)),
                size[0],
            )
        } else {
            (
                QVec3::new(quanta(size[0] / 2.0), quanta(size[1] / 2.0), 0),
                size[2],
            )
        };
        let local = LocalShape::Cylinder {
            axis,
            base,
            length: quanta(length),
            radius: quanta(r),
        };
        self.out.push(Primitive {
            shape: Shape::oriented(self.lat.frame, Self::qv(self.at(lo)), local, self.cell),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags,
        });
    }

    /// A wall of `n` facets round the box's vertical axis, open at the top, or (two in five)
    /// roofed (FORM-24): flat, or a roof with an oculus over the middle.
    pub(crate) fn ring(&mut self, b: &SlotBox, n: u8, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let r = (hi[0] - lo[0]).min(hi[2] - lo[2]) / 2.0;
        let centre = [(lo[0] + hi[0]) / 2.0, lo[1], (lo[2] + hi[2]) / 2.0];
        self.ring_at(centre, r, hi[1] - lo[1], n, flags);
        // The draw is keyed by where the ring is, so the same ring is roofed the same way in
        // every cell that builds it.
        let mut h = 0x9e37_79b9_7f4a_7c15_u64;
        for v in [centre[0], centre[1], centre[2], r] {
            h = (h ^ u64::from(quanta(v).cast_unsigned())).wrapping_mul(0x0100_0000_01b3);
        }
        let roof = (h >> 33) % 5;
        if roof >= 2 {
            return;
        }
        let thick = 8.0_f64.min(r / 2.0);
        let inner = (r - thick).max(1.0);
        // The roof is 1 m thick and sits on the wall, inside its inner face.
        let top = [centre[0], hi[1] - 1.0, centre[2]];
        let (x0, z0) = (centre[0] - inner, centre[2] - inner);
        let at = self.at([x0, top[1], z0]);
        if roof == 0 {
            // Flat: a cylinder across the ring.
            let local = LocalShape::Cylinder {
                axis: Axis::Y,
                base: QVec3::new(quanta(inner), 0, quanta(inner)),
                length: quanta(1.0),
                radius: quanta(inner),
            };
            self.out.push(Primitive {
                shape: Shape::oriented(self.lat.frame, Self::qv(at), local, self.cell),
                material: MaterialId::CONCRETE,
                kind: FeatureKind::MASS,
                flags,
            });
        } else {
            // With an oculus: a ring of facets, 1 m high, from the oculus out to the wall.
            let oculus = inner * 0.3;
            let rr = crate::orient::FacetRing {
                axis: Self::qv(self.at([centre[0], top[1], centre[2]])),
                radius: quanta(oculus),
                thick: quanta(inner - oculus + 0.5),
                height: quanta(1.0),
                n,
                start: 0,
                material: MaterialId::CONCRETE,
                kind: FeatureKind::MASS,
                flags,
            };
            self.out.extend(crate::orient::facet_ring(&rr, self.cell));
        }
    }

    fn ring_at(&mut self, centre: [f64; 3], radius: f64, height: f64, n: u8, flags: PrimFlags) {
        let thick = 8.0_f64.min(radius / 2.0);
        let r = crate::orient::FacetRing {
            axis: Self::qv(self.at(centre)),
            radius: quanta((radius - thick).max(1.0)),
            thick: quanta(thick),
            height: quanta(height),
            n,
            start: 0,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags,
        };
        self.out.extend(crate::orient::facet_ring(&r, self.cell));
    }

    /// Four rings that narrow towards the top.
    fn dome(&mut self, b: &SlotBox, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let big = (hi[0] - lo[0]).min(hi[2] - lo[2]) / 2.0;
        let tier = (hi[1] - lo[1]) / 4.0;
        for k in 0..4 {
            let t = f64::from(k) / 4.0;
            let r = big * (1.0 - t * t).sqrt();
            self.ring_at(
                [
                    (lo[0] + hi[0]) / 2.0,
                    lo[1] + f64::from(k) * tier,
                    (lo[2] + hi[2]) / 2.0,
                ],
                r,
                tier,
                16,
                flags,
            );
        }
    }

    /// Eight stacked cylinders, narrowing upwards (or downwards when inverted).
    fn cone(&mut self, b: &SlotBox, inverted: bool, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let big = (hi[0] - lo[0]).min(hi[2] - lo[2]) / 2.0;
        let len = (hi[1] - lo[1]) / 8.0;
        for i in 0..8 {
            let t = f64::from(if inverted { 7 - i } else { i }) / 8.0;
            let r = (big * (1.0 - 0.85 * t)).max(1.0);
            let local = LocalShape::Cylinder {
                axis: Axis::Y,
                base: QVec3::new(quanta(big), 0, quanta(big)),
                length: quanta(len),
                radius: quanta(r),
            };
            let at = self.at([lo[0], lo[1] + f64::from(i) * len, lo[2]]);
            self.out.push(Primitive {
                shape: Shape::oriented(self.lat.frame, Self::qv(at), local, self.cell),
                material: MaterialId::CONCRETE,
                kind: FeatureKind::MASS,
                flags,
            });
        }
    }

    fn wedge(&mut self, b: &SlotBox, rise: Dir4, flags: PrimFlags) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let local = QAabb::new(
            QVec3::new(0, 0, 0),
            QVec3::new(
                quanta(hi[0] - lo[0]),
                quanta(hi[1] - lo[1]),
                quanta(hi[2] - lo[2]),
            ),
        );
        self.out.push(Primitive {
            shape: Shape::oriented(
                self.lat.frame,
                Self::qv(self.at(lo)),
                LocalShape::Ramp { aabb: local, rise },
                self.cell,
            ),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MASS,
            flags,
        });
    }

    /// Windows on the four sides of the box, one grid per slot of wall (at most 16 slots).
    fn lit_faces(&mut self, b: &SlotBox) {
        let mut budget = 8;
        for (axis, positive) in [(0usize, true), (0, false), (2, true), (2, false)] {
            let other = 2 - axis;
            for y in b.lo[1]..b.hi[1] {
                for o in b.lo[other]..b.hi[other] {
                    if budget == 0 {
                        return;
                    }
                    let mut lo = [0; 3];
                    lo[1] = y;
                    lo[other] = o;
                    lo[axis] = if positive { b.hi[axis] } else { b.lo[axis] - 1 };
                    // A face against another solid slot has no outside to look out of.
                    let n = self.lat.n as i32;
                    if lo.iter().all(|&v| (0..n).contains(&v))
                        && matches!(
                            self.lat.state[self.lat.index(lo.map(|v| v as usize))],
                            SlotState::Built | SlotState::Solid | SlotState::Engulfed
                        )
                    {
                        continue;
                    }
                    budget -= 1;
                    let slab = SlotBox {
                        lo,
                        hi: [lo[0] + 1, lo[1] + 1, lo[2] + 1],
                    };
                    let facing = match (axis, positive) {
                        (0, true) => Dir6::PosX,
                        (0, false) => Dir6::NegX,
                        (_, true) => Dir6::PosZ,
                        (_, false) => Dir6::NegZ,
                    };
                    self.windows(&slab, facing);
                }
            }
        }
    }

    /// A 4 m slab on the side of the box that touches its host.
    fn facade(&mut self, b: &SlotBox, host: Dir6, flags: PrimFlags) {
        let (mut lo, mut hi) = (self.corner(b.lo), self.corner(b.hi));
        let (k, positive) = match host {
            Dir6::PosX => (0, true),
            Dir6::NegX => (0, false),
            Dir6::PosY => (1, true),
            Dir6::NegY => (1, false),
            Dir6::PosZ => (2, true),
            Dir6::NegZ => (2, false),
        };
        if positive {
            lo[k] = hi[k] - 4.0;
        } else {
            hi[k] = lo[k] + 4.0;
        }
        self.frame_box(lo, hi, MaterialId::CONCRETE, FeatureKind::MASS, flags);
    }

    /// A thin vertical cable at slot column `(x, z)` from slot level `y0` to `y1`.
    fn cable(&mut self, x: i32, z: i32, y0: i32, y1: i32) {
        let p = self.lat.pitch as f64;
        let (cx, cz) = ((f64::from(x) + 0.5) * p, (f64::from(z) + 0.5) * p);
        let (lo, hi) = (
            [cx - 0.25, f64::from(y0) * p, cz - 0.25],
            [cx + 0.25, f64::from(y1) * p, cz + 0.25],
        );
        self.frame_box(
            lo,
            hi,
            MaterialId::PIPE,
            FeatureKind::CABLE,
            PrimFlags::NONE,
        );
    }

    /// Lit windows on the face of the host that `facing` looks away from: 1.4 m by 1.6 m panels
    /// on a 4 m grid (one storey high), about a third of them lit, standing 12 cm off the wall.
    fn windows(&mut self, b: &SlotBox, facing: Dir6) {
        let (lo, hi) = (self.corner(b.lo), self.corner(b.hi));
        let (axis, positive) = match facing {
            Dir6::PosX => (0, true),
            Dir6::NegX => (0, false),
            Dir6::PosZ => (2, true),
            _ => (2, false),
        };
        let other = 2 - axis;
        let plane = if positive { lo[axis] } else { hi[axis] };
        let (across, up) = (hi[other] - lo[other], hi[1] - lo[1]);
        let key =
            b.lo.iter()
                .fold(u64::from(axis as u8) * 2 + u64::from(positive), |h, &v| {
                    h.wrapping_mul(0x0100_0000_01b3) ^ u64::from(v.cast_unsigned())
                });
        for i in 0..((across - 2.0) / 4.0).floor() as i32 {
            for j in 0..((up - 1.0) / 4.0).floor() as i32 {
                let h = (key
                    ^ (u64::from(i.cast_unsigned()) << 8)
                    ^ (u64::from(j.cast_unsigned()) << 16))
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15);
                if (h >> 40) % 100 >= 30 {
                    continue;
                }
                let (mut l, mut hgh) = (lo, hi);
                // The back of the panel is on the face of the shell, which stands `SHELL_INSET`
                // inside the part's box.
                if positive {
                    l[axis] = plane - SHELL_INSET;
                    hgh[axis] = plane - SHELL_INSET + 0.12;
                } else {
                    l[axis] = plane + SHELL_INSET - 0.12;
                    hgh[axis] = plane + SHELL_INSET;
                }
                let u = lo[other] + 2.0 + 4.0 * f64::from(i);
                l[other] = u;
                hgh[other] = u + 1.4;
                let v = lo[1] + 1.2 + 4.0 * f64::from(j);
                l[1] = v;
                hgh[1] = v + 1.6;
                self.frame_box(
                    l,
                    hgh,
                    MaterialId::LIT_WINDOW,
                    FeatureKind::WINDOW,
                    PrimFlags::FAR_VISIBLE,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellKey, Level};
    use crate::seed::WorldSeed;
    use crate::tissue::{TissueCache, tissue_cell};
    use glam::I64Vec3;

    /// FORM-24: a ring is open at the top, or roofed (flat, or with an oculus), by a draw keyed
    /// by where it is: about three in five stay open, a fifth are flat and a fifth have an oculus.
    #[test]
    fn rings_are_open_or_roofed() {
        let lat = crate::tissue::Lattice::new(crate::orient::Frame::Id);
        let (mut open, mut flat, mut oculus) = (0, 0, 0);
        for k in 0..300i32 {
            let b = SlotBox {
                lo: [k % 5, k / 5 % 4, k / 20 % 5],
                hi: [k % 5 + 2, k / 5 % 4 + 2, k / 20 % 5 + 2],
            };
            let mut e = Emitter::new(&lat);
            e.ring(&b, 16, PrimFlags::NONE);
            match e.out.len() {
                16 => open += 1,
                17 => flat += 1,
                32 => oculus += 1,
                n => panic!("{n} pieces in a ring"),
            }
        }
        assert!(
            open > 120 && flat > 30 && oculus > 30,
            "{open} open, {flat} flat, {oculus} with an oculus"
        );
    }

    fn seed() -> WorldSeed {
        WorldSeed(0x5eed)
    }

    fn cells(parents: i64, per: i64) -> impl Iterator<Item = CellKey> {
        (0..parents).flat_map(move |k| {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 * 5 - 4 + k % 3);
            (0..per).map(move |j| {
                CellKey::new(
                    Level::L2,
                    l1 * 8 + I64Vec3::new(j % 8, j / 8 % 8, (j * 7 + k) % 8),
                )
            })
        })
    }

    /// Built matter as the fill counts it: structures, swallowed routes and parent solid.
    fn built_fraction(t: &Tissue) -> f64 {
        let present = t.lattice.present.iter().filter(|&&p| p).count().max(1);
        let built = t
            .lattice
            .state
            .iter()
            .zip(&t.lattice.present)
            .filter(|(s, p)| {
                **p && matches!(s, SlotState::Built | SlotState::Engulfed | SlotState::Solid)
            })
            .count();
        built as f64 / present as f64
    }

    /// TIS-03, TIS-06: dense cells are filled to within ±0.10 of their field, every structure
    /// has an entrance on the route (or is sealed), and the anchors stay connected through the
    /// swallowed segments.
    #[test]
    fn growth_fills_cells_to_the_field() {
        let cache = TissueCache::default();
        let (mut dense, mut within, mut err) = (0, 0, 0.0);
        for key in cells(8, 100) {
            let t = tissue_cell(seed(), &cache, key);
            assert!(t.connects_anchors(), "{key:?}");
            let mut on = vec![false; t.lattice.len()];
            for &i in &t.route {
                on[i as usize] = true;
            }
            for s in &t.structures {
                assert!(s.sealed || !s.entrances.is_empty(), "{key:?}: no entrance");
                for e in &s.entrances {
                    assert!(on[e.slot as usize], "{key:?}: entrance off the route");
                }
                assert!(s.slots.iter().all(|&i| t.lattice.present[i as usize]));
            }
            let f = f64::from(t.fill) / 255.0;
            if f > 0.5 {
                let b = built_fraction(&t);
                dense += 1;
                within += usize::from((b - f).abs() <= 0.10);
                err += (b - f).abs();
            }
            if key.coords.x % 5 == 0 {
                let again = tissue_cell(seed(), &cache, key);
                assert_eq!(t.structures, again.structures, "{key:?}: deterministic");
            }
        }
        let share = within as f64 / f64::from(dense);
        eprintln!(
            "dense {dense}: {within} within ±0.10 ({share:.2}), mean error {:.3}",
            err / f64::from(dense)
        );
        assert!(dense > 200);
        assert!(share >= 0.75 && err / f64::from(dense) <= 0.07);
    }

    /// SUP-01..10 all occur (SUP-11 needs a gravity well, PW.6).
    #[test]
    fn every_support_occurs() {
        let cache = TissueCache::default();
        let mut seen = [0usize; 11];
        for key in cells(16, 60) {
            let t = tissue_cell(seed(), &cache, key);
            for s in &t.structures {
                seen[usize::from(s.support)] += 1;
            }
        }
        // Nested structures need a hall: look at the L2 cell in the middle of some.
        let mut halls = 0;
        'search: for x in -8..8 {
            for z in -8..8 {
                let home = I64Vec3::new(x, 0, z);
                for v in crate::l1_void::voids_of_l0(seed(), home) {
                    if v.kind != crate::l1_void::VoidKind::Hall || halls >= 40 {
                        continue;
                    }
                    halls += 1;
                    let (lo, hi) = v.bounds();
                    let c = ((lo + hi) / 2).div_euclid(I64Vec3::splat(512));
                    let t = tissue_cell(seed(), &cache, CellKey::new(Level::L2, c));
                    for s in &t.structures {
                        seen[usize::from(s.support)] += 1;
                    }
                    if halls >= 40 {
                        break 'search;
                    }
                }
            }
        }
        eprintln!("supports {seen:?}");
        for (k, n) in seen.iter().enumerate().take(10) {
            assert!(*n > 0, "{} never placed", SUPPORT_TAGS[k]);
        }
        assert_eq!(seen[RADIAL], 0);
    }

    /// SV-01..26 all occur, and every piece they emit stays inside the cell.
    #[test]
    fn every_variant_occurs() {
        let cache = TissueCache::default();
        let mut seen = [0usize; 27];
        let mut forms = [0usize; FORMS.len()];
        let mut muts = [0usize; 16];
        let cell = (0..3).flat_map(|_| [0i64]).count();
        let _ = cell;
        for key in cells(24, 60) {
            let t = tissue_cell(seed(), &cache, key);
            for s in &t.structures {
                seen[usize::from(s.variant)] += 1;
                if usize::from(s.form) < crate::hubs::HUB_BASE {
                    forms[usize::from(s.form)] += 1;
                }
                for &m in &s.mutations {
                    muts[usize::from(m)] += 1;
                }
                for p in s.primitives(&t.lattice) {
                    let b = p.aabb();
                    let e = 512 * Q_PER_M;
                    assert!(
                        b.min.x >= 0
                            && b.min.y >= 0
                            && b.min.z >= 0
                            && b.max.x <= e
                            && b.max.y <= e
                            && b.max.z <= e,
                        "{key:?} variant {}: {b:?}",
                        s.variant
                    );
                }
            }
        }
        // Halls for the nested variants.
        let mut halls = 0;
        'search: for x in -8..8 {
            for z in -8..8 {
                for v in crate::l1_void::voids_of_l0(seed(), I64Vec3::new(x, 0, z)) {
                    if !matches!(
                        v.kind,
                        crate::l1_void::VoidKind::Hall | crate::l1_void::VoidKind::Shaft
                    ) || halls >= 80
                    {
                        continue;
                    }
                    halls += 1;
                    let (lo, hi) = v.bounds();
                    let c = ((lo + hi) / 2).div_euclid(I64Vec3::splat(512));
                    let t = tissue_cell(seed(), &cache, CellKey::new(Level::L2, c));
                    for s in &t.structures {
                        seen[usize::from(s.variant)] += 1;
                    }
                    if halls >= 80 {
                        break 'search;
                    }
                }
            }
        }
        eprintln!("variants {seen:?}\nforms {forms:?}\nmutations {muts:?}");
        for (k, n) in muts.iter().enumerate().skip(1) {
            assert!(*n > 0, "MUT-{k:02} never applied");
        }
        for (k, n) in forms.iter().enumerate() {
            assert!(*n > 0, "{} never placed", FORMS[k].tag);
        }
        for (sv, n) in seen.iter().enumerate().skip(1) {
            assert!(*n > 0, "SV-{sv:02} never placed");
        }
    }

    /// The skin: structures and cladding together cover at least 90 % of the tiles, measured
    /// again from the final pieces (a tile counts when 12 of its 16 probes lie in a piece).
    #[test]
    fn the_skin_is_covered() {
        use glam::Vec3;
        let cache = TissueCache::default();
        let (mut tiles, mut covered, mut by_structures, mut cladding) =
            (0u32, 0u32, 0usize, 0usize);
        for key in cells(12, 100) {
            let t = tissue_cell(seed(), &cache, key);
            by_structures += t.skin.covered;
            cladding += t.skin.cladding.len();
            let mut prims: Vec<_> = t
                .structures
                .iter()
                .flat_map(|s| s.primitives(&t.lattice))
                .collect();
            prims.extend(t.skin.cladding.iter().copied());
            for probes in &t.skin.samples {
                tiles += 1;
                let hit = probes
                    .iter()
                    .filter(|&&p| prims.iter().any(|q| q.shape.contains(p + Vec3::ZERO)))
                    .count();
                covered += u32::from(hit >= 12);
            }
        }
        let share = f64::from(covered) / f64::from(tiles);
        eprintln!(
            "skin: {tiles} tiles, {covered} covered ({share:.3}); {by_structures} by structures, {cladding} cladding pieces"
        );
        assert!(tiles > 1000 && cladding > 0);
        assert!(share >= 0.90, "{share}");
    }

    /// GRV-01: near a Point well structures stand on turned floors, nowhere else.
    #[test]
    fn structures_near_a_well_stand_normal_to_the_field() {
        use crate::gravity::{WellKind, find_well, gravity_at_metres, pose_for_up};
        let (_, w) = find_well(seed(), WellKind::Point, 8).expect("a point well");
        let cache = crate::tissue::TissueCache::default();
        let mut turned = 0;
        let mut checked = 0;
        for k in 0..200i64 {
            // Cells on a ring at half the reach, where the field leans by tens of degrees.
            let r = w.reach / 2;
            let p = [
                w.centre[0] + r * (k % 5 - 2) / 2,
                w.centre[1] + r / 2 + (k / 5 % 4) * 128,
                w.centre[2] + r * (k / 20 % 5 - 2) / 2,
            ];
            let g = gravity_at_metres(seed(), p);
            if pose_for_up(g.up).is_none() {
                continue;
            }
            let l2 = glam::I64Vec3::new(p[0] / 512, p[1] / 512, p[2] / 512);
            let t = crate::tissue::tissue_cell(seed(), &cache, CellKey::new(Level::L2, l2));
            checked += 1;
            turned += t
                .structures
                .iter()
                .filter(|s| !s.oriented.is_empty())
                .count();
        }
        assert!(checked > 20, "{checked}");
        assert!(
            turned > 0,
            "no turned structure in {checked} cells near a well"
        );
    }
}
