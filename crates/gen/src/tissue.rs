//! The L2 tissue of the Web: the slot lattice, the reservation of what the parent holds, and
//! the routes that connect the cell's anchors (TIS-01, TIS-02, TIS-04 marking).
//! Spec: `docs/spec/levels.md` §5W.1, §5W.2 steps 1 and 2.
//!
//! The order is what makes connectivity hold: routes are laid on a lattice that is still
//! almost empty, so a walk always succeeds (its Dijkstra finish runs over every present slot,
//! reserved ones costing more), and growth (PW.3.2) may only take what routes leave.

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
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use glam::{DVec3, I64Vec3, Vec3};

use crate::cavity::CavityView;
use crate::cell::{CellKey, Dir6, Level};
use crate::character::{Character, character_near};
use crate::hash::hash;
use crate::l1_artery::{ArteryType, Skeleton, skeleton};
use crate::l1_between::{WebPiece, between_content};
use crate::l1_frame::{FRM_INHABITED, frame_content};
use crate::l1_surface::surface_content;
use crate::l1_void::voids_of_cell;
use crate::orient::Frame;
use crate::prim::Primitive;
use crate::regime::regime;
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;
use crate::tissue_grow::{Structure, grow};

/// Edge of an L2 cell in metres.
const CELL_M: f64 = 512.0;
/// Slot pitch before the stratum's scale, in metres.
pub const PITCH_M: i64 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotState {
    Free,
    /// Parent air that tissue may not fill (a void): a route over it is a span.
    Void,
    /// Parent solid (a beam, a tether, a plate): a route through it is bored.
    Solid,
    Route,
    Built,
    Engulfed,
}

/// The slot lattice of an L2 cell, in the cell's frame.
#[derive(Debug, Clone)]
pub struct Lattice {
    /// Slots per side.
    pub n: usize,
    /// Slot edge in metres.
    pub pitch: i64,
    pub frame: Frame,
    centres: Vec<DVec3>,
    pub present: Vec<bool>,
    pub state: Vec<SlotState>,
}

impl Lattice {
    /// The lattice of a cell with `frame`. The pitch is always 32 m: the stratum's tissue
    /// scale (SC-05) multiplies the sizes of forms and paths, not the lattice, because a
    /// route slot of 96 m would take a third of a coarse cell (`levels.md` §5W.11).
    pub fn new(frame: Frame) -> Self {
        let pitch = PITCH_M;
        let n = (512 / pitch) as usize;
        let axes = frame.axes();
        let centres: Vec<DVec3> = (0..n * n * n)
            .map(|i| {
                let c = [i % n, i / n % n, i / (n * n)];
                let f = |k: usize| (c[k] as f64 + 0.5) * pitch as f64 - CELL_M / 2.0;
                DVec3::splat(CELL_M / 2.0) + axes[0] * f(0) + axes[1] * f(1) + axes[2] * f(2)
            })
            .collect();
        let present = centres
            .iter()
            .map(|c| c.cmpge(DVec3::ZERO).all() && c.cmplt(DVec3::splat(CELL_M)).all())
            .collect();
        Self {
            n,
            pitch,
            frame,
            centres,
            present,
            state: vec![SlotState::Free; n * n * n],
        }
    }

    pub fn len(&self) -> usize {
        self.n * self.n * self.n
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn coords(&self, i: usize) -> [usize; 3] {
        [i % self.n, i / self.n % self.n, i / (self.n * self.n)]
    }

    pub fn index(&self, c: [usize; 3]) -> usize {
        c[0] + self.n * (c[1] + self.n * c[2])
    }

    /// The slot centre in cell-local metres.
    pub fn centre(&self, i: usize) -> DVec3 {
        self.centres[i]
    }

    /// The six axis neighbours in slot space, present ones only, in the order −x, +x, −y,
    /// +y, −z, +z.
    pub fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let c = self.coords(i);
        let n = self.n as i64;
        (0..6).filter_map(move |d| {
            let (k, s) = (d / 2, if d % 2 == 0 { -1 } else { 1 });
            let v = c[k] as i64 + s;
            if !(0..n).contains(&v) {
                return None;
            }
            let mut o = c;
            o[k] = v as usize;
            let j = self.index(o);
            self.present[j].then_some(j)
        })
    }

    /// The present slot nearest to the cell-local point `p` (ties: lowest index).
    pub fn nearest(&self, p: DVec3) -> Option<usize> {
        (0..self.len())
            .filter(|&i| self.present[i])
            .min_by(|&a, &b| {
                let (da, db) = (
                    self.centre(a).distance_squared(p),
                    self.centre(b).distance_squared(p),
                );
                da.total_cmp(&db).then(a.cmp(&b))
            })
    }

    /// Chebyshev distance of two slots in slot space.
    pub fn cheb(&self, a: usize, b: usize) -> usize {
        let (a, b) = (self.coords(a), self.coords(b));
        (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0)
    }
}

/// What an L2 cell reads from its parent: the skeleton, the pieces it reserves, the stratum.
#[derive(Debug)]
pub struct WebParent {
    pub l1: I64Vec3,
    pub skeleton: Option<Skeleton>,
    /// Every L1 piece with its tag, in L1-local quanta. Void pieces carry `solid`.
    pub pieces: Vec<Reserved>,
    pub cavity: bool,
    /// The fill field of the L1 cell (`l1_fill`), 512 values.
    pub fill: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct Reserved {
    pub piece: WebPiece,
    /// `false` for the keep-out of a void: air that tissue may not fill.
    pub solid: bool,
}

impl WebParent {
    pub fn new(seed: WorldSeed, l1: I64Vec3) -> Self {
        let key = CellKey::new(Level::L1, l1);
        if CavityView::new(seed, key).is_cavity(key) {
            return Self {
                l1,
                skeleton: None,
                pieces: Vec::new(),
                cavity: true,
                fill: vec![0; 512],
            };
        }
        let mut pieces = Vec::new();
        let solid = |v: Vec<WebPiece>, out: &mut Vec<Reserved>| {
            out.extend(
                v.into_iter()
                    // Inhabited beams are walkable: routes run along them, not through them.
                    .filter(|p| p.tag != FRM_INHABITED)
                    .map(|piece| Reserved { piece, solid: true }),
            );
        };
        solid(frame_content(seed, l1), &mut pieces);
        solid(between_content(seed, l1), &mut pieces);
        solid(surface_content(seed, l1), &mut pieces);
        for v in voids_of_cell(seed, l1) {
            pieces.push(Reserved {
                piece: v.piece,
                solid: v.solid,
            });
        }
        Self {
            l1,
            skeleton: skeleton(seed, l1),
            pieces,
            cavity: false,
            fill: crate::l1_fill::fill_field(seed, l1),
        }
    }
}

/// Parents by L1 cell, for one batch of queries. Lookups only; never iterated. Shared between
/// threads like `L1Cache`.
#[derive(Default)]
pub struct TissueCache {
    parents: Mutex<HashMap<I64Vec3, Arc<OnceLock<Arc<WebParent>>>>>,
}

impl TissueCache {
    pub fn get(&self, seed: WorldSeed, l1: I64Vec3) -> Arc<WebParent> {
        let slot = {
            let mut parents = self
                .parents
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(parents.entry(l1).or_default())
        };
        Arc::clone(slot.get_or_init(|| Arc::new(WebParent::new(seed, l1))))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnchorKind {
    /// An artery crossing the face `face`, or an L1 face port.
    Port {
        face: Dir6,
        kind: ArteryType,
    },
    Knot,
    /// A set-piece or lift-lobby place drawn in step 2 (PW.3.16 gives it a piece).
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub kind: AnchorKind,
    /// The lattice slot the anchor routes from.
    pub slot: u32,
    /// Where the port sits, in cell-local metres.
    pub point: [f64; 3],
}

/// What surrounds a route segment (`levels.md` §5W.2 step 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surround {
    /// Level, in free space or over a void.
    Level,
    /// A height change.
    Rise,
    /// Through parent solid: a tunnel or crawl duct.
    Bored,
    /// Over a void.
    Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Slots in order, ends included (shared with the neighbouring segments).
    pub slots: Vec<u32>,
    pub surround: Surround,
    /// May disappear into a structure in step 3 (TIS-04).
    pub engulfable: bool,
    /// Index of the walk this segment belongs to.
    pub walk: u16,
}

#[derive(Debug, Clone)]
pub struct Tissue {
    pub seed: WorldSeed,
    pub cell: CellKey,
    pub lattice: Lattice,
    pub anchors: Vec<Anchor>,
    /// The route tree and loops: slots in insertion order.
    pub route: Vec<u32>,
    pub segments: Vec<Segment>,
    /// The cell's fill density from the parent field, 0..=255.
    pub fill: u8,
    /// Per slot: lies inside a crust (where the cell's shell can carry structures).
    pub in_crust: Vec<bool>,
    /// The stratum's character, when the cell has a crust.
    pub character: Option<Character>,
    /// Structures in the order they were placed (step 3).
    pub structures: Vec<Structure>,
    /// Route segments that structures swallowed (TIS-04).
    pub engulfed: Vec<u16>,
    /// The skin pass: covered tiles and the cladding that covers the rest.
    pub skin: crate::skin::Skin,
    /// The path types of the level segments and their pieces (PW.3.9).
    pub paths: crate::paths::PathPieces,
    /// The thresholds on structure entrances and bored passages (PW.3.13).
    pub thresholds: crate::thresholds::Thresholds,
    /// The hub family of the knot, when this cell holds the knot.
    pub knot_hub: Option<u8>,
    /// `(HUB id, index into structures)` of the hubs placed (PW.3.15).
    pub hubs: Vec<(u8, u16)>,
    /// A neighbouring L2 cell has another frame: a grain seam runs through or beside this one.
    pub seam: bool,
    /// Where a tether (STR-03) lands in the cell, in cell-local metres.
    pub tether_foot: Option<[f64; 3]>,
    /// `(SET id, index into structures)` of the set pieces placed (PW.3.16, PW.3.17).
    pub sets: Vec<(u8, u16)>,
    /// `(LOC id, index into structures)` of the local places placed (PW.3.19).
    pub locals: Vec<(u8, u16)>,
    /// What ends each dead end, and the knot's beacon (PW.3.18).
    pub payoffs: crate::payoffs::Payoffs,
    /// Segments added by the dead-end pass that must be built as a lift (PV-16).
    pub lift_segments: Vec<u16>,
    /// The entrance slots with a floor under the route (PW.4.17), from `l3_web::floored_slots`.
    pub floored_slots: Vec<u32>,
}

/// Local Dir6 order of the skeleton's faces: −x, +x, −y, +y, −z, +z.
const FACE_ORDER: [Dir6; 6] = [
    Dir6::NegX,
    Dir6::PosX,
    Dir6::NegY,
    Dir6::PosY,
    Dir6::NegZ,
    Dir6::PosZ,
];

fn face_rank(d: Dir6) -> usize {
    FACE_ORDER.iter().position(|&f| f == d).unwrap_or(0)
}

/// The centre of face `d` of an L2 cell, in cell-local metres.
fn face_point(d: Dir6) -> [f64; 3] {
    let o = d.offset();
    [0, 1, 2].map(|k| {
        let v = [o.x, o.y, o.z][k];
        256.0 + v as f64 * 255.99
    })
}

/// Tissue of L2 cell `cell` up to its routes: reservation, anchors and routes (steps 1 and 2).
pub fn tissue_routes(seed: WorldSeed, cache: &TissueCache, cell: CellKey) -> Tissue {
    build(seed, cache, cell, false)
}

/// The whole tissue of L2 cell `cell`: routes, then growth (step 3).
pub fn tissue_cell(seed: WorldSeed, cache: &TissueCache, cell: CellKey) -> Tissue {
    build(seed, cache, cell, true)
}

#[allow(clippy::too_many_lines)] // one pipeline, read top to bottom
fn build(seed: WorldSeed, cache: &TissueCache, cell: CellKey, grown: bool) -> Tissue {
    debug_assert_eq!(cell.level, Level::L2);
    let l2 = cell.coords;
    let l1 = l2.div_euclid(I64Vec3::splat(8));
    let local = l2.rem_euclid(I64Vec3::splat(8));
    let l0 = l1.div_euclid(I64Vec3::splat(8));
    let parent = cache.get(seed, l1);
    let web = regime(seed, l0).web;
    let frame = web.frame_l2(seed, l2);
    let centre = l2 * 512 + I64Vec3::splat(256);
    let chr = character_near(seed, l0, centre.rem_euclid(I64Vec3::splat(32_768)));
    let mut lat = Lattice::new(frame);
    let fill = parent.fill[((local.z * 8 + local.y) * 8 + local.x) as usize];

    reserve(&mut lat, &parent, local);
    drop_tram_slots(&mut lat, seed, l1, local);
    let st = crate::stack::stack(seed, l0);
    let base = l2 * 512 - l0 * 32_768;
    let in_crust: Vec<bool> = (0..lat.len())
        .map(|i| {
            let c = lat.centre(i);
            let p = base + I64Vec3::new(c.x as i64, c.y as i64, c.z as i64);
            lat.present[i] && matches!(st.stratum_at(p, None), crate::stack::Stratum::Crust(_))
        })
        .collect();

    // Anchors: ports sorted by face and type, the knot, then drawn extras.
    let mut found: Vec<(usize, usize, AnchorKind, [f64; 3])> = Vec::new();
    if let Some(s) = &parent.skeleton {
        let here = crate::l1_artery::index([local.x, local.y, local.z]) as u16;
        let type_rank = |k: ArteryType| ArteryType::ALL.iter().position(|&t| t == k).unwrap_or(0);
        for &(dir, p) in &s.ports {
            if crate::l1_artery::index(p.l2(!dir.is_positive())) as u16 == here {
                found.push((
                    face_rank(dir),
                    type_rank(p.kind),
                    AnchorKind::Port {
                        face: dir,
                        kind: p.kind,
                    },
                    face_point(dir),
                ));
            }
        }
        for (a, b, kind) in s.links() {
            let (other, _) = if a == here {
                (b, a)
            } else if b == here {
                (a, b)
            } else {
                continue;
            };
            let (c, o) = (
                crate::l1_artery::coords(here as usize),
                crate::l1_artery::coords(other as usize),
            );
            let dir = Dir6::ALL
                .into_iter()
                .find(|d| {
                    let v = d.offset();
                    c[0] + v.x == o[0] && c[1] + v.y == o[1] && c[2] + v.z == o[2]
                })
                .unwrap_or(Dir6::PosX);
            let e = (
                face_rank(dir),
                type_rank(kind),
                AnchorKind::Port { face: dir, kind },
                face_point(dir),
            );
            if !found.iter().any(|f| f.2 == e.2) {
                found.push(e);
            }
        }
        found.sort_by_key(|f| (f.0, f.1));
        if crate::l1_artery::index([local.x, local.y, local.z]) as u16 == s.knot {
            found.push((6, 0, AnchorKind::Knot, [256.0; 3]));
        }
    }
    let mut anchors: Vec<Anchor> = found
        .into_iter()
        .filter_map(|(_, _, kind, point)| {
            lat.nearest(DVec3::from_array(point)).map(|slot| Anchor {
                kind,
                slot: slot as u32,
                point,
            })
        })
        .collect();

    let mut rng = Rng::new(hash(seed.0, Level::L2, l2, salt::TISSUE));
    // Extra anchors (0..=2): free slots at the edge of voids first.
    for _ in 0..rng.below(3) {
        let draw = rng.next_u64();
        let free: Vec<usize> = (0..lat.len())
            .filter(|&i| lat.present[i] && lat.state[i] == SlotState::Free)
            .collect();
        let edge: Vec<usize> = free
            .iter()
            .copied()
            .filter(|&i| lat.neighbours(i).any(|j| lat.state[j] == SlotState::Void))
            .collect();
        let pool = if edge.is_empty() { &free } else { &edge };
        if pool.is_empty() {
            continue;
        }
        let i = pool[(draw % pool.len() as u64) as usize];
        anchors.push(Anchor {
            kind: AnchorKind::Extra,
            slot: i as u32,
            point: lat.centre(i).to_array(),
        });
    }

    let seam = Dir6::ALL
        .iter()
        .any(|d| web.frame_l2(seed, l2 + d.offset()) != frame);
    let origin_m = local * 512;
    let tether_foot = parent
        .pieces
        .iter()
        .filter(|r| r.piece.tag == crate::l1_between::STR_TETHER)
        .find_map(|r| {
            let b = r.piece.prim.aabb();
            let at = |lo: i32, hi: i32, o: i64| (f64::from(lo) + f64::from(hi)) / 32.0 - o as f64;
            let (x, z) = (
                at(b.min.x, b.max.x, origin_m.x),
                at(b.min.z, b.max.z, origin_m.z),
            );
            let y = f64::from(b.min.y) / 16.0 - origin_m.y as f64;
            ((0.0..512.0).contains(&x) && (0.0..512.0).contains(&y) && (0.0..512.0).contains(&z))
                .then_some([x, y, z])
        });
    let mut t = Tissue {
        seed,
        cell,
        lattice: lat,
        anchors,
        route: Vec::new(),
        segments: Vec::new(),
        fill,
        in_crust,
        character: chr,
        structures: Vec::new(),
        engulfed: Vec::new(),
        skin: crate::skin::Skin::default(),
        paths: crate::paths::PathPieces::default(),
        thresholds: crate::thresholds::Thresholds::default(),
        knot_hub: parent
            .skeleton
            .as_ref()
            .filter(|s| s.knot == crate::l1_artery::index([local.x, local.y, local.z]) as u16)
            .map(|s| s.knot_hub),
        hubs: Vec::new(),
        seam,
        tether_foot,
        sets: Vec::new(),
        locals: Vec::new(),
        payoffs: crate::payoffs::Payoffs::default(),
        lift_segments: Vec::new(),
        floored_slots: Vec::new(),
    };
    let probe = SolidProbe::new(&parent, local);
    route(&mut t, &mut rng, web.weave, &probe);
    if grown {
        grow(&mut t, &mut rng, web.weave);
        crate::payoffs::payoff_pass(&mut t, &mut rng);
        let prims: Vec<_> = t
            .structures
            .iter()
            .flat_map(|s| s.primitives(&t.lattice))
            .collect();
        t.skin = crate::skin::skin_pass(seed, &t, &parent.pieces, local, &prims);
        t.paths = crate::paths::paths_pass(seed, &t);
        t.floored_slots = crate::l3_web::floored_slots(&t);
        // The approach to each door: a deck from the route slot to the face (PW.4.17), part of
        // the paths so that the rest of the pipeline treats it as one.
        let mut approach = crate::tissue_grow::Emitter::new(&t.lattice);
        for (lo, hi) in crate::l3_web::approach_decks(&t) {
            approach.frame_box(
                lo,
                hi,
                crate::prim::MaterialId::CONCRETE,
                crate::prim::FeatureKind::WALKWAY,
                crate::prim::PrimFlags::NONE,
            );
        }
        t.paths.prims.extend(approach.out);
        t.thresholds = crate::thresholds::thresholds_pass(seed, &t);
    }
    t
}

/// A tram line's reserved volume is clear air: a slot it touches is absent, so no route,
/// structure or path piece enters it (the lattice stays connected: a tube is a line).
fn drop_tram_slots(lat: &mut Lattice, seed: WorldSeed, l1: I64Vec3, local: I64Vec3) {
    let origin = (local * 512).as_dvec3();
    let reach = lat.pitch as f64 * 0.87;
    let q = 16.0;
    let boxes: Vec<(DVec3, DVec3)> = crate::tram::envelopes(seed, CellKey::new(Level::L1, l1))
        .iter()
        .map(|e| {
            let lo =
                DVec3::new(f64::from(e.min.x), f64::from(e.min.y), f64::from(e.min.z)) / q - origin;
            let hi =
                DVec3::new(f64::from(e.max.x), f64::from(e.max.y), f64::from(e.max.z)) / q - origin;
            ((lo + hi) / 2.0, (hi - lo) / 2.0)
        })
        .collect();
    if boxes.is_empty() {
        return;
    }
    for i in 0..lat.len() {
        if !lat.present[i] {
            continue;
        }
        let c = lat.centre(i);
        if boxes.iter().any(|(mid, half)| {
            let d = (c - *mid).abs();
            d.x <= half.x + reach && d.y <= half.y + reach && d.z <= half.z + reach
        }) {
            lat.present[i] = false;
        }
    }
}

/// The parent's solid pieces near an L2 cell, for asking whether a point of the cell is in
/// solid.
pub(crate) struct SolidProbe {
    /// `(lo, hi, piece)` in L1-local metres.
    items: Vec<(Vec3, Vec3, Primitive)>,
    /// The cell's origin in L1-local metres.
    origin: Vec3,
}

impl SolidProbe {
    fn new(parent: &WebParent, local: I64Vec3) -> Self {
        let o = local * 512;
        let origin = Vec3::new(o.x as f32, o.y as f32, o.z as f32);
        let (lo, hi) = (origin, origin + Vec3::splat(512.0));
        let v = |x: i32| x as f32 / 16.0;
        let items = parent
            .pieces
            .iter()
            .filter(|r| r.solid)
            .filter_map(|r| {
                let b = r.piece.prim.aabb();
                let (a, z) = (
                    Vec3::new(v(b.min.x), v(b.min.y), v(b.min.z)),
                    Vec3::new(v(b.max.x), v(b.max.y), v(b.max.z)),
                );
                (z.cmpgt(lo - Vec3::splat(8.0)).all() && a.cmplt(hi + Vec3::splat(8.0)).all())
                    .then_some((a, z, r.piece.prim))
            })
            .collect();
        Self { items, origin }
    }

    /// Whether the cell-local point `p` (metres) is inside a solid piece.
    fn contains(&self, p: DVec3) -> bool {
        let q = Vec3::new(p.x as f32, p.y as f32, p.z as f32) + self.origin;
        self.items
            .iter()
            .any(|(a, z, prim)| q.cmpge(*a).all() && q.cmplt(*z).all() && prim.shape.contains(q))
    }

    /// Whether the way from slot `a` to slot `b`, a walkway 4 m wide and 3.6 m high on the
    /// floor of the slots, runs into solid. Vertical steps are not tested (a stair's footprint
    /// is its own).
    fn step_hits(&self, lat: &Lattice, a: usize, b: usize) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let (ca, cb) = (lat.centre(a), lat.centre(b));
        let axes = lat.frame.axes();
        let (ia, ib) = (lat.coords(a), lat.coords(b));
        if ia[1] != ib[1] {
            return false;
        }
        let side = if ia[0] == ib[0] { axes[0] } else { axes[2] };
        let floor = lat.pitch as f64 / 2.0;
        let steps = 16;
        for i in 0..=steps {
            let c = ca + (cb - ca) * (f64::from(i) / f64::from(steps));
            for dy in [0.3, 1.2, 2.4, 3.6] {
                for dt in [-2.0, 0.0, 2.0] {
                    if self.contains(c - axes[1] * (floor - dy) + side * dt) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Step 1: marks the slots whose centre lies in a parent piece.
fn reserve(lat: &mut Lattice, parent: &WebParent, local: I64Vec3) {
    let origin = local * 512;
    let lo = Vec3::new(origin.x as f32, origin.y as f32, origin.z as f32);
    let hi = lo + Vec3::splat(512.0);
    let q = 16.0;
    let bounds = |r: &Reserved| {
        let b = r.piece.prim.aabb();
        let v = |x: i32| x as f32 / q;
        (
            Vec3::new(v(b.min.x), v(b.min.y), v(b.min.z)),
            Vec3::new(v(b.max.x), v(b.max.y), v(b.max.z)),
        )
    };
    let near: Vec<(&Reserved, Vec3, Vec3)> = parent
        .pieces
        .iter()
        .filter_map(|r| {
            let (a, b) = bounds(r);
            (b.cmpgt(lo).all() && a.cmplt(hi).all()).then_some((r, a, b))
        })
        .collect();
    if near.is_empty() {
        return;
    }
    for i in 0..lat.len() {
        if !lat.present[i] {
            continue;
        }
        let c = lat.centre(i);
        let p = lo + Vec3::new(c.x as f32, c.y as f32, c.z as f32);
        // Solid beats void: a column standing inside a hall is solid.
        let mut state = None;
        for &(r, a, b) in &near {
            if p.cmpge(a).all() && p.cmplt(b).all() && r.piece.prim.shape.contains(p) {
                if r.solid {
                    state = Some(SlotState::Solid);
                    break;
                }
                state = Some(SlotState::Void);
            }
        }
        if let Some(s) = state {
            lat.state[i] = s;
        }
    }
}

/// The cost of entering a slot in the Dijkstra finish.
fn cost(s: SlotState) -> u32 {
    match s {
        SlotState::Solid => 8,
        SlotState::Void => 2,
        _ => 1,
    }
}

/// Where a walk is heading.
#[derive(Clone, Copy)]
pub(crate) enum Goal {
    /// Any slot of the route.
    Route,
    Slot(usize),
}

pub(crate) struct Router<'a> {
    pub(crate) lat: &'a Lattice,
    pub(crate) in_route: Vec<bool>,
    pub(crate) list: Vec<u32>,
    pub(crate) weave: u8,
}

impl Router<'_> {
    fn reached(&self, i: usize, goal: Goal) -> bool {
        match goal {
            Goal::Route => self.in_route[i],
            Goal::Slot(g) => i == g,
        }
    }

    /// The goal slot nearest to `from` (Chebyshev; ties: lowest index).
    fn target(&self, from: usize, goal: Goal) -> usize {
        match goal {
            Goal::Slot(g) => g,
            Goal::Route => self
                .list
                .iter()
                .map(|&j| j as usize)
                .min_by_key(|&j| (self.lat.cheb(from, j), j))
                .unwrap_or(from),
        }
    }

    /// A weighted random walk with a step budget, finished by Dijkstra. Returns the slots from
    /// `start` to the first goal slot, 6-connected, `start` included.
    fn walk(&self, rng: &mut Rng, start: usize, goal: Goal) -> Vec<usize> {
        let target = self.target(start, goal);
        let dist = self.lat.cheb(start, target);
        let budget = 4 * dist + 8;
        let n = self.lat.n as i64;
        let mut path = vec![start];
        let mut seen = vec![false; self.lat.len()];
        seen[start] = true;
        let mut cur = start;
        let w_axis = u32::from(255 - self.weave).max(1);
        let w_diag = u32::from(self.weave / 2);
        for _ in 0..budget {
            if self.reached(cur, goal) && cur != start {
                return path;
            }
            let c = self.lat.coords(cur);
            let here = self.lat.cheb(cur, target);
            // (slot, via, weight): `via` is the supporting slot of a diagonal move.
            let mut moves: Vec<(usize, Option<usize>, u32)> = Vec::new();
            for k in 0..3 {
                for s in [-1i64, 1] {
                    let v = c[k] as i64 + s;
                    if !(0..n).contains(&v) {
                        continue;
                    }
                    let mut o = c;
                    o[k] = v as usize;
                    let j = self.lat.index(o);
                    if self.lat.present[j] && !seen[j] {
                        moves.push((j, None, w_axis));
                    }
                }
            }
            if w_diag > 0 {
                for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                    for (sa, sb) in [(-1i64, -1i64), (-1, 1), (1, -1), (1, 1)] {
                        let (va, vb) = (c[a] as i64 + sa, c[b] as i64 + sb);
                        if !(0..n).contains(&va) || !(0..n).contains(&vb) {
                            continue;
                        }
                        let mut o = c;
                        o[a] = va as usize;
                        let via = self.lat.index(o);
                        o[b] = vb as usize;
                        let j = self.lat.index(o);
                        if self.lat.present[j] && self.lat.present[via] && !seen[j] && !seen[via] {
                            moves.push((j, Some(via), w_diag));
                        }
                    }
                }
            }
            if moves.is_empty() {
                break;
            }
            let weights: Vec<u32> = moves
                .iter()
                .map(|&(j, _, w)| {
                    let toward = if self.lat.cheb(j, target) < here {
                        4
                    } else {
                        1
                    };
                    let avoid = if self.lat.state[j] == SlotState::Solid {
                        8
                    } else {
                        1
                    };
                    (w * toward / avoid).max(1)
                })
                .collect();
            let Some(pick) = rng.pick_weighted(&weights) else {
                break;
            };
            let (j, via, _) = moves[pick];
            if let Some(v) = via {
                path.push(v);
                seen[v] = true;
            }
            path.push(j);
            seen[j] = true;
            cur = j;
        }
        if self.reached(cur, goal) && cur != start {
            return path;
        }
        // Dijkstra from `cur` to the nearest goal slot, over every present slot.
        let tail = self.dijkstra(cur, goal);
        path.extend(tail.into_iter().skip(1));
        path
    }

    /// Cheapest path from `from` to the nearest goal slot (ties: lowest index). `from` is
    /// the first element. A walk that is already at its goal returns just `from`.
    pub(crate) fn dijkstra(&self, from: usize, goal: Goal) -> Vec<usize> {
        let mut best = vec![u32::MAX; self.lat.len()];
        let mut prev = vec![usize::MAX; self.lat.len()];
        let mut heap = BinaryHeap::new();
        best[from] = 0;
        heap.push(Reverse((0u32, from)));
        let mut end = from;
        while let Some(Reverse((d, i))) = heap.pop() {
            if d > best[i] {
                continue;
            }
            if self.reached(i, goal) {
                end = i;
                break;
            }
            for j in self.lat.neighbours(i) {
                let nd = d + cost(self.lat.state[j]);
                if nd < best[j] {
                    best[j] = nd;
                    prev[j] = i;
                    heap.push(Reverse((nd, j)));
                }
            }
        }
        let mut out = vec![end];
        while *out.last().unwrap_or(&from) != from {
            let p = prev[*out.last().unwrap_or(&from)];
            if p == usize::MAX {
                break;
            }
            out.push(p);
        }
        out.reverse();
        out
    }

    fn add(&mut self, path: &[usize]) {
        for &i in path {
            if !self.in_route[i] {
                self.in_route[i] = true;
                self.list.push(i as u32);
            }
        }
    }
}

/// Step 2: the route tree over the anchors, loops, segments and engulfable marks.
pub(crate) fn route(t: &mut Tissue, rng: &mut Rng, weave: u8, probe: &SolidProbe) {
    let lat = t.lattice.clone();
    let mut r = Router {
        lat: &lat,
        in_route: vec![false; lat.len()],
        list: Vec::new(),
        weave,
    };
    let mut walks: Vec<Vec<usize>> = Vec::new();
    for (k, a) in t.anchors.iter().enumerate() {
        let start = a.slot as usize;
        if k == 0 {
            r.add(&[start]);
            continue;
        }
        let path = r.walk(rng, start, Goal::Route);
        r.add(&path);
        walks.push(path);
    }
    // Loops: 0..=2 extra walks between route slots at least 4 slots apart.
    for _ in 0..rng.below(3) {
        let n = (r.list.len() as u64).max(1);
        let (i, j) = (rng.below(n) as usize, rng.below(n) as usize);
        if r.list.is_empty() {
            continue;
        }
        let (a, b) = (r.list[i] as usize, r.list[j] as usize);
        if t.lattice.cheb(a, b) >= 4 {
            let path = r.walk(rng, a, Goal::Slot(b));
            r.add(&path);
            walks.push(path);
        }
    }
    // A step that runs into parent solid between the slot centres (a plate or a wall that no
    // centre lies in) is bored like a step through a solid slot: its slots count as crossed.
    // A slot where three or more routes meet is a junction: segments end there, so that a stair
    // or a catwalk has its landing or deck at the node and not a leg across its flights.
    let mut nb: Vec<Vec<usize>> = vec![Vec::new(); t.lattice.len()];
    for path in &walks {
        for w in path.windows(2) {
            for (a, b) in [(w[0], w[1]), (w[1], w[0])] {
                if !nb[a].contains(&b) {
                    nb[a].push(b);
                }
            }
        }
    }
    let junction: Vec<bool> = nb.iter().map(|v| v.len() >= 3).collect();
    let mut crossed = vec![false; t.lattice.len()];
    for path in &walks {
        for w in path.windows(2) {
            if probe.step_hits(&t.lattice, w[0], w[1]) {
                crossed[w[0]] = true;
                crossed[w[1]] = true;
            }
        }
    }
    for (w, path) in walks.iter().enumerate() {
        for seg in cut(&t.lattice, path, &crossed, &junction) {
            // 1/8 in the open, up to 3/4 in a dense stratum: routes run through buildings there.
            let engulfable = rng.below(1024) < 128 + 640 * u64::from(t.fill) / 255;
            let surround = surround_of(&t.lattice, &seg, &crossed);
            t.segments.push(Segment {
                slots: seg.iter().map(|&i| i as u32).collect(),
                surround,
                engulfable: engulfable && matches!(surround, Surround::Level | Surround::Rise),
                walk: w as u16,
            });
        }
    }
    for &i in &r.list {
        let i = i as usize;
        if matches!(t.lattice.state[i], SlotState::Free) {
            t.lattice.state[i] = SlotState::Route;
        }
    }
    t.route = r.list;
}

/// Cuts a walk at corners, at junctions, at height changes and where the surround changes.
fn cut(lat: &Lattice, path: &[usize], crossed: &[bool], junction: &[bool]) -> Vec<Vec<usize>> {
    let class = |i: usize| match lat.state[i] {
        SlotState::Solid => 2u8,
        _ if crossed[i] => 2,
        SlotState::Void => 1,
        _ => 0,
    };
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = vec![path[0]];
    let mut last_step: Option<[i64; 3]> = None;
    for w in path.windows(2) {
        let (a, b) = (lat.coords(w[0]), lat.coords(w[1]));
        let step = [0, 1, 2].map(|k| b[k] as i64 - a[k] as i64);
        let corner = last_step.is_some_and(|s| s != step);
        if corner || junction[w[0]] || class(w[0]) != class(w[1]) {
            out.push(std::mem::take(&mut cur));
            cur.push(w[0]);
        }
        cur.push(w[1]);
        last_step = Some(step);
    }
    out.push(cur);
    out.retain(|s| s.len() > 1);
    out
}

fn surround_of(lat: &Lattice, seg: &[usize], crossed: &[bool]) -> Surround {
    if seg
        .iter()
        .any(|&i| lat.state[i] == SlotState::Solid || crossed[i])
    {
        return Surround::Bored;
    }
    if seg.iter().any(|&i| lat.state[i] == SlotState::Void) {
        return Surround::Span;
    }
    let (a, b) = (lat.coords(seg[0]), lat.coords(seg[seg.len() - 1]));
    if a[1] == b[1] {
        Surround::Level
    } else {
        Surround::Rise
    }
}

impl Tissue {
    /// Whether every anchor's slot lies in one 6-connected component of the route.
    pub fn connects_anchors(&self) -> bool {
        let Some(first) = self.anchors.first() else {
            return true;
        };
        let mut on = vec![false; self.lattice.len()];
        for &i in &self.route {
            on[i as usize] = true;
        }
        let mut seen = vec![false; self.lattice.len()];
        let mut stack = vec![first.slot as usize];
        seen[first.slot as usize] = true;
        while let Some(i) = stack.pop() {
            for j in self.lattice.neighbours(i) {
                if on[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        self.anchors.iter().all(|a| seen[a.slot as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> WorldSeed {
        WorldSeed(0x5eed)
    }

    /// Every frame of the tables leaves the present slots 6-connected, so a route always
    /// exists.
    #[test]
    fn the_lattice_stays_connected_in_every_frame() {
        let mut frames = vec![Frame::Id];
        for yaw in 0..64 {
            for tilt in 0..7 {
                if yaw != 0 || tilt != 0 {
                    frames.push(Frame::table(yaw, tilt));
                }
            }
        }
        for frame in frames {
            for scale in [0u8] {
                let lat = Lattice::new(frame);
                let first = (0..lat.len()).find(|&i| lat.present[i]).unwrap();
                let mut seen = vec![false; lat.len()];
                let mut stack = vec![first];
                seen[first] = true;
                while let Some(i) = stack.pop() {
                    for j in lat.neighbours(i) {
                        if !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
                let present = lat.present.iter().filter(|&&p| p).count();
                let reached = seen.iter().filter(|&&s| s).count();
                assert_eq!(present, reached, "{frame:?} scale {scale}");
                assert!(
                    present * 2 > lat.len(),
                    "{frame:?}: only {present} of {}",
                    lat.len()
                );
            }
        }
    }

    #[test]
    fn id_frame_has_every_slot() {
        let lat = Lattice::new(Frame::Id);
        assert_eq!((lat.len(), lat.pitch), (4096, 32));
        assert!(lat.present.iter().all(|&p| p));
    }

    /// TIS-01/02: over many cells every anchor is connected by the route, the segments are
    /// 6-connected paths in the route, and a cell derives the same tissue twice.
    #[test]
    fn routes_connect_every_anchor_in_ten_thousand_cells() {
        let cache = TissueCache::default();
        let (mut cells, mut with_ports, mut reserved, mut segs, mut bored) = (0, 0, 0, 0, 0);
        for k in 0..40i64 {
            let l1 = I64Vec3::new(k % 7 - 3, k / 7 % 3, k / 21 * 5 - 4 + k % 3);
            for j in 0..250i64 {
                let c = l1 * 8 + I64Vec3::new(j % 8, j / 8 % 8, (j * 7 + k) % 8);
                let key = CellKey::new(Level::L2, c);
                let t = tissue_routes(seed(), &cache, key);
                assert!(t.connects_anchors(), "{c:?}");
                let mut on = vec![false; t.lattice.len()];
                for &i in &t.route {
                    on[i as usize] = true;
                }
                for s in &t.segments {
                    for w in s.slots.windows(2) {
                        let (a, b) = (
                            t.lattice.coords(w[0] as usize),
                            t.lattice.coords(w[1] as usize),
                        );
                        let m: usize = (0..3).map(|k| a[k].abs_diff(b[k])).sum();
                        assert_eq!(m, 1, "{c:?}: segment step");
                    }
                    assert!(s.slots.iter().all(|&i| on[i as usize]));
                    segs += 1;
                    bored += usize::from(s.surround == Surround::Bored);
                }
                if j % 100 == 0 {
                    let u = tissue_routes(seed(), &cache, key);
                    assert_eq!(t.route, u.route, "{c:?}: deterministic");
                    assert_eq!(t.segments, u.segments);
                }
                cells += 1;
                with_ports += usize::from(
                    t.anchors
                        .iter()
                        .any(|a| matches!(a.kind, AnchorKind::Port { .. })),
                );
                reserved += usize::from(
                    t.lattice
                        .state
                        .iter()
                        .any(|s| matches!(s, SlotState::Solid | SlotState::Void)),
                );
            }
        }
        assert_eq!(cells, 10_000);
        assert!(with_ports > 500, "{with_ports} cells with ports");
        assert!(reserved > 500, "{reserved} cells with reserved slots");
        assert!(segs > 1000, "{segs} segments");
        println!(
            "tissue: {cells} cells, {with_ports} with ports, {reserved} with reserved slots, {segs} segments, {bored} bored"
        );
    }
}
