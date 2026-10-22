//! Census v2 (PW.13.1): the fault classes of the polish audit (`docs/playtests/2026-10-22-polish-audit.md`)
//! counted over L2 cells with their L3 interiors and parent L1 pieces. Ported from the audit's
//! probe; nothing here changes generator output. Spec: `levels.md` §5W.48.
//!
//! The counters (letter, then a name; every count is keyed `"<letter> <what>"`):
//! - `A` walk clearance against every solid, by leg or rise, blocker and path id;
//! - `B` clusters of solid that touch nothing, split by size;
//! - `C` interiors inside other solid, and interiors with a shell face missing;
//! - `D` two legs on one line;
//! - `E` slivers under 0.1 m and crumbs;
//! - `F` walk lines through non-solid decoration;
//! - `G` non-solid plates of at least 1 x 1 m that look walkable;
//! - `H` large pieces the mid band does not draw;
//! - `L` L3 solid outside every mass;
//! - `R` rails with no walk line beside them, and rails inside solid of a path;
//! - `S` structure parts sliced by a cell face.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::needless_pass_by_value,
    clippy::items_after_statements,
    clippy::type_complexity,
    clippy::collapsible_if,
    clippy::manual_midpoint,
    clippy::uninlined_format_args,
    clippy::useless_format,
    clippy::explicit_iter_loop,
    clippy::needless_range_loop,
    clippy::many_single_char_names,
    clippy::cast_lossless
)]

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{DVec3, I64Vec3, Vec3};

use crate::carve::translated;
use crate::cell::{CellKey, Level};
use crate::l2_port::L1Cache;
use crate::orient::LocalShape;
use crate::prim::{FeatureKind, PrimFlags, Primitive, Shape};
use crate::seed::WorldSeed;
use crate::tissue::tissue_cell;

#[derive(Default)]
pub struct AuditReport {
    pub cells: u64,
    pub counts: BTreeMap<String, u64>,
    pub examples: BTreeMap<String, Vec<String>>,
}

impl AuditReport {
    fn add(&mut self, k: impl Into<String>, n: u64) {
        *self.counts.entry(k.into()).or_default() += n;
    }
    fn ex(&mut self, k: &str, s: impl FnOnce() -> String) {
        let v = self.examples.entry(k.to_string()).or_default();
        if v.len() < 8 {
            v.push(s());
        }
    }
    pub fn merge(&mut self, o: AuditReport) {
        self.cells += o.cells;
        for (k, v) in o.counts {
            *self.counts.entry(k).or_default() += v;
        }
        for (k, v) in o.examples {
            let e = self.examples.entry(k).or_default();
            for s in v {
                if e.len() < 8 {
                    e.push(s);
                }
            }
        }
    }
}

fn kind_name(k: FeatureKind) -> &'static str {
    [
        "trunk",
        "deck",
        "shaft_wall",
        "bridge",
        "mass",
        "stair_tower",
        "ramp",
        "walkway",
        "railing",
        "pipe",
        "sanctuary",
        "misc",
        "tram",
        "tether",
        "cable",
        "window",
        "skin",
        "void",
        "ladder",
    ]
    .get(k.0 as usize)
    .copied()
    .unwrap_or("?")
}

struct Item {
    p: Primitive,
    lo: Vec3,
    hi: Vec3,
    cat: &'static str,
    solid: bool,
}

struct Grid {
    items: Vec<Item>,
    grid: HashMap<[i32; 3], Vec<u32>>,
}
const CS: f32 = 8.0;
fn bucket(p: Vec3) -> [i32; 3] {
    [
        (p.x / CS).floor() as i32,
        (p.y / CS).floor() as i32,
        (p.z / CS).floor() as i32,
    ]
}
impl Grid {
    fn new(items: Vec<Item>) -> Self {
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        let (lo_b, hi_b) = (Vec3::splat(-80.0), Vec3::splat(592.0));
        for (i, it) in items.iter().enumerate() {
            if it.hi.cmplt(lo_b).any() || it.lo.cmpgt(hi_b).any() {
                continue;
            }
            let a = bucket(it.lo.max(lo_b));
            let b = bucket(it.hi.min(hi_b));
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        grid.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        Self { items, grid }
    }
    fn solid_at(&self, p: Vec3, filt: impl Fn(&Item) -> bool) -> Option<&Item> {
        self.grid
            .get(&bucket(p))?
            .iter()
            .map(|&i| &self.items[i as usize])
            .find(|it| {
                it.solid
                    && filt(it)
                    && p.cmpge(it.lo - 0.001).all()
                    && p.cmple(it.hi + 0.001).all()
                    && it.p.shape.contains(p)
            })
    }
}

fn metres(p: &Primitive) -> (Vec3, Vec3) {
    let b = p.aabb();
    (b.min_m(), b.max_m())
}

fn v(d: DVec3) -> Vec3 {
    Vec3::new(d.x as f32, d.y as f32, d.z as f32)
}

/// Smallest local extent of a box-like piece, metres.
fn thinness(p: &Primitive) -> Option<f32> {
    let b = match p.shape {
        Shape::Box { aabb } => aabb,
        Shape::Oriented(o) => match o.local {
            LocalShape::Box { aabb } => aabb,
            _ => return None,
        },
        _ => return None,
    };
    let s = b.max_m() - b.min_m();
    Some(s.min_element())
}

pub fn audit_cell(seed: WorldSeed, cache: &L1Cache, c: I64Vec3, l3: bool) -> AuditReport {
    let mut r = AuditReport {
        cells: 1,
        ..Default::default()
    };
    let key = CellKey::new(Level::L2, c);
    let t = tissue_cell(seed, cache.tissue(), key);
    let own = crate::l2_web::l2_cell_web(seed, cache, key).primitives;
    let tag = format!(
        "L2 {},{},{}{}{}",
        c.x,
        c.y,
        c.z,
        if t.strange.is_some() { " strange" } else { "" },
        if t.rare.is_some() { " rare" } else { "" }
    );
    let world = |p: Vec3| {
        // cell (c) local -> sector/local of the game.
        let w = DVec3::new(
            c.x as f64 * 512.0 + p.x as f64,
            c.y as f64 * 512.0 + p.y as f64,
            c.z as f64 * 512.0 + p.z as f64,
        );
        let s = (w / 1024.0).floor();
        let l = w - s * 1024.0;
        format!(
            "SECTOR={},{},{} LOCAL={:.1},{:.1},{:.1}",
            s.x as i64, s.y as i64, s.z as i64, l.x, l.y, l.z
        )
    };

    let set = |v: &[Primitive]| v.iter().copied().collect::<HashSet<_>>();
    let paths = set(&t.paths.prims);
    let thr = set(&t.thresholds.prims);
    let pay = set(&t.payoffs.prims);
    let kit: HashSet<Primitive> = crate::kit_ext::dress(&t).into_iter().map(|x| x.1).collect();
    let quar: HashSet<Primitive> = crate::quarters::dress(&t)
        .into_iter()
        .map(|x| x.1)
        .collect();
    let grow: HashSet<Primitive> = crate::growth::dress(&t).into_iter().map(|x| x.1).collect();
    let strange = set(&crate::strange::cell_pieces(seed, c));
    let well = set(&crate::wellgeom::cell_pieces(seed, c));
    let gates = set(&crate::gates::cell_markers(seed, c));
    let cat_of = |p: &Primitive| -> &'static str {
        if p.kind == FeatureKind::RAILING {
            "rail"
        } else if paths.contains(p) {
            "path"
        } else if thr.contains(p) {
            "threshold"
        } else if pay.contains(p) {
            "payoff"
        } else if kit.contains(p) {
            "kit_ext"
        } else if quar.contains(p) {
            "quarters"
        } else if grow.contains(p) {
            "growth"
        } else if strange.contains(p) {
            "strange"
        } else if well.contains(p) {
            "wellgeom"
        } else if gates.contains(p) {
            "gates"
        } else if p.flags.contains(PrimFlags::INTERIOR) {
            "shell"
        } else {
            "structure"
        }
    };
    let solid_flag = |p: &Primitive| {
        !p.flags.contains(PrimFlags::NO_COLLIDE)
            && !matches!(p.shape, Shape::LiftShaft { .. })
            && p.kind != FeatureKind::VOID
    };
    let mut items: Vec<Item> = Vec::new();
    for p in &own {
        let (lo, hi) = metres(p);
        items.push(Item {
            p: *p,
            lo,
            hi,
            cat: cat_of(p),
            solid: solid_flag(p),
        });
    }
    let n_own = items.len();
    // L1 parent.
    let l1 = CellKey::new(Level::L1, c.div_euclid(I64Vec3::splat(8)));
    let local = c.rem_euclid(I64Vec3::splat(8));
    let d = [local.x, local.y, local.z].map(|n| -(n as i32) * 512 * 16);
    for p in &cache.get(seed, l1).primitives {
        if p.flags.contains(PrimFlags::FAR_ONLY) {
            continue;
        }
        let q = translated(p, d);
        let (lo, hi) = metres(&q);
        if hi.cmpgt(Vec3::splat(-80.0)).all() && lo.cmplt(Vec3::splat(592.0)).all() {
            items.push(Item {
                p: q,
                lo,
                hi,
                cat: "L1",
                solid: solid_flag(&q),
            });
        }
    }
    let n_l1_end = items.len();
    // L3 interiors.
    if l3 {
        let mut ctx = crate::l3_web::web_context(seed, cache, key);
        for k in 0..512i64 {
            let (i, j, l) = (k % 8, k / 8 % 8, k / 64);
            let w = CellKey::new(Level::L3, c * 8 + I64Vec3::new(i, j, l));
            if !ctx.is_active(w) {
                continue;
            }
            let d = [i as i32 * 1024, j as i32 * 1024, l as i32 * 1024];
            for p in ctx.cell_primitives(seed, w) {
                let q = translated(&p, d);
                let (lo, hi) = metres(&q);
                items.push(Item {
                    p: q,
                    lo,
                    hi,
                    cat: "L3",
                    solid: solid_flag(&q),
                });
            }
        }
    }
    let g = Grid::new(items);
    let inside =
        |p: Vec3, m: f32| p.cmpge(Vec3::splat(m)).all() && p.cmple(Vec3::splat(512.0 - m)).all();

    // ---------- A: walk clearance over every leg and rise ----------
    let tilted = t.lattice.frame.axes()[1].y < 0.999;
    let mut own_of: Vec<Vec<Primitive>> = vec![Vec::new(); t.paths.walks.len()];
    for (rg, wi) in &t.paths.owners {
        if let Some(v) = own_of.get_mut(*wi) {
            v.extend(t.paths.prims[rg.clone()].iter().copied());
        }
    }
    let final_set: HashSet<Primitive> = own.iter().copied().collect();
    let tram_l1 = crate::tram::envelopes(seed, l1);
    let l1o_q = (I64Vec3::new(local.x, local.y, local.z) * 512 * 16).as_ivec3();
    let in_tram = |p: &Primitive| {
        let b = p.aabb();
        tram_l1.iter().any(|e| {
            b.min.x + l1o_q.x < e.max.x
                && b.max.x + l1o_q.x > e.min.x
                && b.min.y + l1o_q.y < e.max.y
                && b.max.y + l1o_q.y > e.min.y
                && b.min.z + l1o_q.z < e.max.z
                && b.max.z + l1o_q.z > e.min.z
        })
    };
    for (wi, own_pieces) in own_of.iter().enumerate() {
        for p in own_pieces {
            if p.flags.contains(PrimFlags::NO_COLLIDE) || p.kind == FeatureKind::RAILING {
                continue;
            }
            r.add("A0 walk pieces (solid, not rail)", 1);
            if !final_set.contains(p) {
                r.add(
                    format!("A0 walk piece missing from output {}", kind_name(p.kind)),
                    1,
                );
                if in_tram(p) {
                    r.add("A0 missing and in a tram volume", 1);
                }
                let _ = wi;
            }
        }
    }
    let bores_all: Vec<Primitive> = t
        .paths
        .bores
        .iter()
        .chain(&t.thresholds.bores)
        .copied()
        .collect();
    let in_bore = |q: Vec3| bores_all.iter().any(|b| b.shape.contains(q));
    if tilted {
        r.add("A cells tilted (skipped)", 1);
    }
    for (wi, w) in t.paths.walks.iter().enumerate() {
        if tilted {
            break;
        }
        let mine = &own_of[wi];
        let turned = t.lattice.frame.axes()[0].x < 0.9999;
        if w.rise && turned {
            // Undo one `at` (the suspected double transform) and test the floor again.
            let [ex, ey, ez] = t.lattice.frame.axes();
            let inv = |p: DVec3| {
                let c = p - DVec3::splat(256.0);
                DVec3::splat(256.0) + DVec3::new(ex.dot(c), ey.dot(c), ez.dot(c))
            };
            for q in w.points.iter().step_by(2) {
                let has = |p: DVec3| {
                    [0.05f32, 0.2, 0.4].iter().any(|&h| {
                        let x = v(p) - Vec3::Y * h;
                        mine.iter().any(|m| {
                            !m.flags.contains(PrimFlags::NO_COLLIDE) && m.shape.contains(x)
                        })
                    })
                };
                r.add("R turned rise samples", 1);
                if has(*q) {
                    r.add("R floor under the stored point", 1);
                }
                if has(inv(*q)) {
                    r.add("R floor under the point with one at() undone", 1);
                }
            }
        }
        if w.rise && !turned {
            for q in w.points.iter().step_by(2) {
                let has = [0.05f32, 0.2, 0.4].iter().any(|&h| {
                    let x = v(*q) - Vec3::Y * h;
                    mine.iter()
                        .any(|m| !m.flags.contains(PrimFlags::NO_COLLIDE) && m.shape.contains(x))
                });
                r.add("R upright-grid rise samples", 1);
                if has {
                    r.add("R upright-grid floor under the stored point", 1);
                }
            }
        }
        let id = if w.rise {
            format!("PV-{:02}", w.id)
        } else {
            format!("PH-{:02}", w.id)
        };
        let mut samples: Vec<DVec3> = Vec::new();
        let [fx, fy, fz] = t.lattice.frame.axes();
        let fix = |p: DVec3| {
            if !w.rise || fx.x >= 0.9999 {
                return p;
            }
            let c = p - DVec3::splat(256.0);
            DVec3::splat(256.0) + DVec3::new(fx.dot(c), fy.dot(c), fz.dot(c))
        };
        if w.rise {
            samples.extend(w.points.iter().step_by(2).map(|&p| fix(p)));
        } else {
            for pair in w.points.windows(2) {
                let n = (pair[1] - pair[0]).length().ceil().max(1.0) as usize;
                for k in 0..=n {
                    samples.push(pair[0].lerp(pair[1], k as f64 / n as f64));
                }
            }
        }
        // A ladder climbs: no floor test.
        let climb = w.points.windows(2).any(|p| {
            let d = p[1] - p[0];
            d.y.abs() > 2.0 * (d.x * d.x + d.z * d.z).sqrt()
        });
        let mut blocked_walk = false;
        for s in samples {
            let s = v(s);
            if !inside(s, 1.5) {
                continue;
            }
            r.add("A samples", 1);
            for h in [0.6f32, 1.4] {
                let q = s + Vec3::Y * h;
                if let Some(list) = g.grid.get(&bucket(q)) {
                    if let Some(it) = list.iter().map(|&i| &g.items[i as usize]).find(|it| {
                        !it.solid
                            && it.p.flags.contains(PrimFlags::NO_COLLIDE)
                            && !it.p.flags.contains(PrimFlags::MID_ONLY)
                            && it.p.kind != FeatureKind::VOID
                            && q.cmpge(it.lo).all()
                            && q.cmple(it.hi).all()
                            && it.p.shape.contains(q)
                    }) {
                        r.add(
                            format!(
                                "F walk passes through non-solid {} {}",
                                it.cat,
                                kind_name(it.p.kind)
                            ),
                            1,
                        );
                        r.ex("F ghost", || {
                            format!("{tag} {id} at {} {:?}", world(s), it.p.shape.aabb())
                        });
                        break;
                    }
                }
            }
            r.add(
                format!("A samples {}", if w.rise { "rise" } else { "leg" }),
                1,
            );
            let skip_l1 = |q: Vec3| in_bore(q);
            let low = [0.5f32, 1.2].iter().find_map(|&h| {
                let q = s + Vec3::Y * h;
                g.solid_at(q, |it| !(it.cat == "L1" && skip_l1(q)))
            });
            if let Some(it) = low {
                let k = format!("A blocked by {} {}", it.cat, kind_name(it.p.kind));
                r.add(&k, 1);
                r.add(format!("A blocked on {}", id), 1);
                r.add(
                    format!(
                        "A blocked {} by {}",
                        if w.rise { "rise" } else { "leg" },
                        it.cat
                    ),
                    1,
                );
                blocked_walk = true;
                r.ex(&format!("A blocked by {}", it.cat), || {
                    format!("{tag} {id} at {} piece {:?}", world(s), it.p.shape.aabb())
                });
            } else if let Some(it) = g.solid_at(s + Vec3::Y * 1.8, |it| {
                !(it.cat == "L1" && skip_l1(s + Vec3::Y * 1.8))
            }) {
                r.add(
                    format!("A headroom<1.8 by {} {}", it.cat, kind_name(it.p.kind)),
                    1,
                );
                r.add(format!("A headroom<1.8 on {}", id), 1);
                r.add(
                    format!(
                        "A headroom {} by {}",
                        if w.rise { "rise" } else { "leg" },
                        it.cat
                    ),
                    1,
                );
                r.ex("A headroom", || format!("{tag} {id} at {}", world(s)));
            }
            if !climb {
                let offs = [
                    Vec3::ZERO,
                    Vec3::X * 0.3,
                    -Vec3::X * 0.3,
                    Vec3::Z * 0.3,
                    -Vec3::Z * 0.3,
                ];
                let floor = offs.iter().any(|o| {
                    [0.05f32, 0.2, 0.4, 0.6]
                        .iter()
                        .any(|&h| g.solid_at(s + *o - Vec3::Y * h, |_| true).is_some())
                });
                let tissue_floor = offs.iter().any(|o| {
                    [0.05f32, 0.2, 0.4, 0.6].iter().any(|&h| {
                        let q = s + *o - Vec3::Y * h;
                        mine.iter().any(|p| {
                            !p.flags.contains(PrimFlags::NO_COLLIDE) && p.shape.contains(q)
                        })
                    })
                });
                if !floor && tissue_floor {
                    r.add("A no floor but the tissue had one", 1);
                }
                if !floor && !tissue_floor {
                    r.add(
                        format!(
                            "A no floor, none in the tissue either {}",
                            if w.rise { "rise" } else { "leg" }
                        ),
                        1,
                    );
                }
                if !floor {
                    r.add(format!("A no floor on {}", id), 1);
                    r.add("A no floor", 1);
                    r.ex("A no floor", || format!("{tag} {id} at {}", world(s)));
                }
            }
        }
        r.add("A walks", 1);
        if blocked_walk {
            r.add("A walks with a block", 1);
        }
    }

    // ---------- B: floating clusters of L2 solids ----------
    {
        let own_items = &g.items[..n_own];
        let structural = |it: &Item| {
            (it.solid || it.p.kind == FeatureKind::CABLE || it.p.kind == FeatureKind::TETHER)
                && !matches!(it.p.shape, Shape::LiftShaft { .. })
                && it.p.kind != FeatureKind::VOID
        };
        let idx: Vec<usize> = (0..n_own).filter(|&i| structural(&own_items[i])).collect();
        let mut parent: Vec<usize> = (0..idx.len()).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let tol = 0.15f32;
        let mut bk: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        for (j, &i) in idx.iter().enumerate() {
            let it = &own_items[i];
            let a = bucket(it.lo - tol);
            let b = bucket(it.hi + tol);
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        bk.entry([x, y, z]).or_default().push(j);
                    }
                }
            }
        }
        for list in bk.values() {
            for a in 0..list.len() {
                for b in a + 1..list.len() {
                    let (ia, ib) = (&own_items[idx[list[a]]], &own_items[idx[list[b]]]);
                    if (ia.lo - tol).cmple(ib.hi).all() && (ib.lo - tol).cmple(ia.hi).all() {
                        let (x, y) = (find(&mut parent, list[a]), find(&mut parent, list[b]));
                        if x != y {
                            parent[x] = y;
                        }
                    }
                }
            }
        }
        // Grounded: AABB touches an L1 piece (lenient), or near the cell's side.
        let l1_items = &g.items[n_own..n_l1_end];
        let mut l1g: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        for (i, it) in l1_items.iter().enumerate() {
            if !it.solid {
                continue;
            }
            let a = bucket(it.lo.max(Vec3::splat(-80.0)));
            let b = bucket(it.hi.min(Vec3::splat(592.0)));
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        l1g.entry([x, y, z]).or_default().push(i);
                    }
                }
            }
        }
        let mut comp: HashMap<usize, (bool, bool, Vec<usize>)> = HashMap::new();
        for j in 0..idx.len() {
            let root = find(&mut parent, j);
            let it = &own_items[idx[j]];
            let e = comp.entry(root).or_insert((false, false, Vec::new()));
            e.2.push(idx[j]);
            if !inside(it.lo, 1.0) || !inside(it.hi, 1.0) {
                e.1 = true;
            }
            if !e.0 {
                let a = bucket(it.lo - tol);
                let b = bucket(it.hi + tol);
                'o: for x in a[0]..=b[0] {
                    for y in a[1]..=b[1] {
                        for z in a[2]..=b[2] {
                            if let Some(l) = l1g.get(&[x, y, z]) {
                                for &k in l {
                                    let q = &l1_items[k];
                                    if (it.lo - tol).cmple(q.hi).all()
                                        && (q.lo - tol).cmple(it.hi).all()
                                    {
                                        e.0 = true;
                                        break 'o;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        r.add("B components", comp.len() as u64);
        for (_, (grounded, edge, members)) in comp {
            if grounded || edge {
                continue;
            }
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut kinds: BTreeMap<String, u32> = BTreeMap::new();
            for &m in &members {
                lo = lo.min(own_items[m].lo);
                hi = hi.max(own_items[m].hi);
                *kinds
                    .entry(format!(
                        "{}/{}",
                        own_items[m].cat,
                        kind_name(own_items[m].p.kind)
                    ))
                    .or_default() += 1;
            }
            let ext = (hi - lo).max_element();
            let size = if ext < 2.0 {
                "tiny<2m"
            } else if ext < 10.0 {
                "small<10m"
            } else {
                "large"
            };
            r.add(format!("B floating {size}"), 1);
            r.add(format!("B floating pieces"), members.len() as u64);
            let main = kinds
                .iter()
                .max_by_key(|x| x.1)
                .map(|x| x.0.clone())
                .unwrap_or_default();
            r.add(format!("B floating led by {main}"), 1);
            r.ex(&format!("B floating {size}"), || {
                format!(
                    "{tag} {} extent {:.1} m, {} pieces {:?}",
                    world((lo + hi) * 0.5),
                    ext,
                    members.len(),
                    kinds
                )
            });
        }
    }

    // ---------- C: interiors against what crosses them ----------
    {
        let masses = crate::l3_web::mass_specs(&t);
        let [ex, ey, ez] = t.lattice.frame.axes();
        let to_world = |f: DVec3| {
            DVec3::splat(256.0) + ex * (f.x - 256.0) + ey * (f.y - 256.0) + ez * (f.z - 256.0)
        };
        let l1o = I64Vec3::new(local.x, local.y, local.z) * 512;
        let trams = crate::tram::envelopes(seed, l1);
        let corridors = &t.paths.corridors;
        let bores = &t.paths.bores;
        let mut intr: BTreeMap<String, u64> = BTreeMap::new();
        for m in &masses {
            r.add("C masses", 1);
            let lo = DVec3::new(
                m.aabb.min.x as f64,
                m.aabb.min.y as f64,
                m.aabb.min.z as f64,
            ) / 16.0;
            let hi = DVec3::new(
                m.aabb.max.x as f64,
                m.aabb.max.y as f64,
                m.aabb.max.z as f64,
            ) / 16.0;
            let mut hits: BTreeMap<&str, u32> = BTreeMap::new();
            let mut n = 0;
            let step = 3.0;
            let mut x = lo.x + 1.0;
            while x < hi.x - 1.0 {
                let mut y = lo.y + 1.0;
                while y < hi.y - 1.0 {
                    let mut z = lo.z + 1.0;
                    while z < hi.z - 1.0 {
                        let p = v(to_world(DVec3::new(x, y, z)));
                        n += 1;
                        if corridors.iter().any(|q| q.shape.contains(p)) {
                            *hits.entry("corridor").or_default() += 1;
                        }
                        if bores.iter().any(|q| q.shape.contains(p)) {
                            *hits.entry("bore").or_default() += 1;
                        }
                        let pq = (p * 16.0).as_ivec3() + (l1o * 16).as_ivec3();
                        if trams.iter().any(|e| {
                            pq.x >= e.min.x
                                && pq.x < e.max.x
                                && pq.y >= e.min.y
                                && pq.y < e.max.y
                                && pq.z >= e.min.z
                                && pq.z < e.max.z
                        }) {
                            *hits.entry("tram").or_default() += 1;
                        }
                        if let Some(it) = g.solid_at(p, |it| it.cat != "L3" && it.cat != "shell") {
                            let k = if it.cat == "L1" {
                                "L1 solid"
                            } else {
                                "L2 solid"
                            };
                            *hits.entry(k).or_default() += 1;
                            let env = match it.p.shape {
                                Shape::Oriented(o) if o.frame != t.lattice.frame => "other-frame",
                                _ => {
                                    if it.p.flags.contains(PrimFlags::ENVELOPE) {
                                        "envelope"
                                    } else {
                                        "plain"
                                    }
                                }
                            };
                            *intr
                                .entry(format!(
                                    "C intruder {} {} {} mat {}",
                                    it.cat,
                                    kind_name(it.p.kind),
                                    env,
                                    it.p.material.0
                                ))
                                .or_default() += 1;
                        }
                        z += step;
                    }
                    y += step;
                }
                x += step;
            }
            r.add("C mass samples", n);
            for (k, c) in &hits {
                r.add(format!("C masses crossed by {k}"), 1);
                r.add(format!("C samples in {k}"), *c as u64);
            }
            if !hits.is_empty() {
                let mid = v(to_world((lo + hi) * 0.5));
                r.ex("C mass crossed", || {
                    format!("{tag} {} {:?} of {n}", world(mid), hits)
                });
            }
            // Shell faces: is there an INTERIOR piece just inside each face centre?
            let mut missing = 0;
            for k in 0..6 {
                let axis = k / 2;
                let mut f = (lo + hi) * 0.5;
                f[axis] = if k % 2 == 0 {
                    lo[axis] + 0.25
                } else {
                    hi[axis] - 0.25
                };
                // Look around the centre (a door may sit in the middle).
                let offs = [(0.0, 0.0), (3.0, 0.0), (-3.0, 0.0), (0.0, 3.0), (0.0, -3.0)];
                let a1 = (axis + 1) % 3;
                let a2 = (axis + 2) % 3;
                let any = offs.iter().any(|&(u, w)| {
                    let mut q = f;
                    q[a1] = (q[a1] + u).clamp(lo[a1] + 0.6, hi[a1] - 0.6);
                    q[a2] = (q[a2] + w).clamp(lo[a2] + 0.6, hi[a2] - 0.6);
                    let p = v(to_world(q));
                    g.solid_at(p, |it| it.cat != "L3").is_some()
                });
                if !any {
                    missing += 1;
                }
            }
            if missing > 0 {
                r.add("C masses with a face missing", 1);
                r.add("C faces missing", missing);
                let mid = v(to_world((lo + hi) * 0.5));
                r.ex("C face missing", || {
                    format!(
                        "{tag} {} faces missing {missing} size {:?}",
                        world(mid),
                        hi - lo
                    )
                });
            }
        }
        for (k, n) in intr {
            r.add(k, n);
        }
    }

    // ---------- L: L3 pieces outside every mass ----------
    if l3 {
        let masses = crate::l3_web::mass_specs(&t);
        let [ex, ey, ez] = t.lattice.frame.axes();
        let to_lat = |p: DVec3| {
            let c = p - DVec3::splat(256.0);
            DVec3::new(ex.dot(c), ey.dot(c), ez.dot(c)) + DVec3::splat(256.0)
        };
        let boxes: Vec<(DVec3, DVec3)> = masses
            .iter()
            .map(|m| {
                (
                    DVec3::new(
                        m.aabb.min.x as f64,
                        m.aabb.min.y as f64,
                        m.aabb.min.z as f64,
                    ) / 16.0,
                    DVec3::new(
                        m.aabb.max.x as f64,
                        m.aabb.max.y as f64,
                        m.aabb.max.z as f64,
                    ) / 16.0,
                )
            })
            .collect();
        for it in &g.items[n_l1_end..] {
            if !it.solid {
                continue;
            }
            let c = to_lat(DVec3::new(
                ((it.lo + it.hi) * 0.5).x as f64,
                ((it.lo + it.hi) * 0.5).y as f64,
                ((it.lo + it.hi) * 0.5).z as f64,
            ));
            r.add("L L3 solid pieces", 1);
            let inside = boxes.iter().any(|(lo, hi)| {
                c.cmpge(*lo - DVec3::splat(0.6)).all() && c.cmple(*hi + DVec3::splat(0.6)).all()
            });
            if !inside {
                r.add(
                    format!("L L3 piece outside every mass {}", kind_name(it.p.kind)),
                    1,
                );
                r.ex("L L3 outside", || {
                    format!("{tag} {}", world((it.lo + it.hi) * 0.5))
                });
            }
        }
    }

    // ---------- D: two legs on one line ----------
    {
        let legs: Vec<&crate::paths::Walk> = t.paths.walks.iter().filter(|w| !w.rise).collect();
        for i in 0..legs.len() {
            for j in i + 1..legs.len() {
                let (a0, a1) = (legs[i].points[0], legs[i].points[1]);
                let (b0, b1) = (legs[j].points[0], legs[j].points[1]);
                let da = (a1 - a0).normalize_or_zero();
                let db = (b1 - b0).normalize_or_zero();
                if da.dot(db).abs() < 0.999 {
                    continue;
                }
                let off = |p: DVec3| {
                    let q = p - a0;
                    (q - da * q.dot(da)).length()
                };
                if off(b0) > 0.6 || off(b1) > 0.6 {
                    continue;
                }
                let (s0, s1) = ((b0 - a0).dot(da), (b1 - a0).dot(da));
                let (lo, hi) = (s0.min(s1).max(0.0), s0.max(s1).min((a1 - a0).length()));
                if hi - lo >= 4.0 {
                    r.add(
                        format!(
                            "D legs sharing a line PH-{:02}/PH-{:02}",
                            legs[i].id.min(legs[j].id),
                            legs[i].id.max(legs[j].id)
                        ),
                        1,
                    );
                    r.add("D legs sharing a line (pairs)", 1);
                    r.add("D shared length (m)", (hi - lo) as u64);
                    let mid = a0 + da * ((lo + hi) / 2.0);
                    r.ex("D shared line", || {
                        format!(
                            "{tag} PH-{:02}+PH-{:02} {:.0} m at {}",
                            legs[i].id,
                            legs[j].id,
                            hi - lo,
                            world(v(mid))
                        )
                    });
                }
            }
        }
        r.add("D legs", legs.len() as u64);
    }

    // ---------- S: parts sliced by the cell's faces ----------
    {
        let [ex, ey, ez] = t.lattice.frame.axes();
        let pitch = t.lattice.pitch as f64;
        let to_world = |f: DVec3| {
            DVec3::splat(256.0) + ex * (f.x - 256.0) + ey * (f.y - 256.0) + ez * (f.z - 256.0)
        };
        let turned = ex.x < 0.9999;
        for st in &t.structures {
            for b in &st.parts {
                let lo = DVec3::new(b.lo[0] as f64, b.lo[1] as f64, b.lo[2] as f64) * pitch;
                let hi = DVec3::new(b.hi[0] as f64, b.hi[1] as f64, b.hi[2] as f64) * pitch;
                let mut out = 0;
                let n = 5;
                for i in 0..n {
                    for j in 0..n {
                        for k in 0..n {
                            let f = lo
                                + (hi - lo)
                                    * DVec3::new(
                                        (i as f64 + 0.5) / n as f64,
                                        (j as f64 + 0.5) / n as f64,
                                        (k as f64 + 0.5) / n as f64,
                                    );
                            let w = to_world(f);
                            if w.min_element() < 0.0 || w.max_element() > 512.0 {
                                out += 1;
                            }
                        }
                    }
                }
                let fr = if turned { "turned" } else { "upright" };
                r.add(format!("S parts {fr}"), 1);
                if out > 0 {
                    r.add(format!("S parts sliced by a cell face {fr}"), 1);
                    r.add(format!("S part volume outside the cell (1/125) {fr}"), out);
                }
            }
        }
    }

    // ---------- E: slivers and fragments ----------
    for it in &g.items[..n_own] {
        if !it.solid {
            continue;
        }
        if matches!(
            it.p.kind,
            FeatureKind::RAILING | FeatureKind::CABLE | FeatureKind::LADDER | FeatureKind::WINDOW
        ) {
            continue;
        }
        r.add("E solid pieces", 1);
        if let Some(th) = thinness(&it.p) {
            if th < 0.1 {
                r.add(
                    format!("E sliver<0.1 {} {}", it.cat, kind_name(it.p.kind)),
                    1,
                );
                r.ex("E sliver", || {
                    format!("{tag} {} {:?}", world((it.lo + it.hi) * 0.5), it.p.shape)
                });
            }
        }
        let vol = (it.hi - it.lo).x * (it.hi - it.lo).y * (it.hi - it.lo).z;
        if vol < 0.02 {
            r.add(format!("E crumb<0.02m3 {}", it.cat), 1);
        }
    }
    // ---------- R: rails with no walk line beside them, and rails inside a path's solid ----------
    {
        let segs: Vec<(Vec3, Vec3, f32)> = t
            .paths
            .walks
            .iter()
            .flat_map(|w| {
                let width = w.width as f32;
                w.points
                    .windows(2)
                    .map(move |q| (v(q[0]), v(q[1]), width))
                    .collect::<Vec<_>>()
            })
            .collect();
        let near_walk = |c: Vec3| {
            segs.iter().any(|&(a, b, width)| {
                let d = b - a;
                let k = ((c - a).dot(d) / d.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (a + d * k - c).length() <= 2.5 + width / 2.0
            })
        };
        for (n, it) in g.items[..n_own].iter().enumerate() {
            if it.cat != "rail" {
                continue;
            }
            let c = (it.lo + it.hi) * 0.5;
            r.add("R rails", 1);
            if !near_walk(c) {
                r.add("R rail with no walk line beside it", 1);
                r.ex("R no walk", || format!("{tag} {}", world(c)));
            }
            let enclosed = g.grid.get(&bucket(c)).is_some_and(|l| {
                l.iter().any(|&i| {
                    let o = &g.items[i as usize];
                    i as usize != n
                        && o.solid
                        && (o.cat == "path" || o.cat == "threshold")
                        && o.p.kind != FeatureKind::RAILING
                        && c.cmpge(o.lo).all()
                        && c.cmple(o.hi).all()
                        && o.p.shape.contains(c)
                })
            });
            if enclosed {
                r.add("R rail inside an enclosed path", 1);
                r.ex("R enclosed", || format!("{tag} {}", world(c)));
            }
        }
    }
    for it in &g.items[..n_own] {
        // G: plates that look walkable but are not solid.
        if it.p.flags.contains(PrimFlags::NO_COLLIDE)
            && !it.p.flags.contains(PrimFlags::MID_ONLY)
            && it.p.kind != FeatureKind::WINDOW
            && it.p.kind != FeatureKind::VOID
            && it.p.material.0 != 8
            && it.p.material.0 != 3
        {
            if let Shape::Box { aabb } = it.p.shape {
                let sz = aabb.max_m() - aabb.min_m();
                if sz.y <= 0.6 && sz.x >= 1.0 && sz.z >= 1.0 {
                    r.add(
                        format!(
                            "G non-solid plate >=1x1 m {} {} mat {}",
                            it.cat,
                            kind_name(it.p.kind),
                            it.p.material.0
                        ),
                        1,
                    );
                    r.ex("G fake floor", || {
                        format!("{tag} {} size {:?}", world((it.lo + it.hi) * 0.5), sz)
                    });
                }
            }
        }
        // H: large drawn pieces the mid band (640 m to 4 km) does not draw.
        if !it.p.flags.contains(PrimFlags::NO_COLLIDE)
            && it.p.kind != FeatureKind::RAILING
            && !matches!(it.p.shape, Shape::LiftShaft { .. })
        {
            let ext = (it.hi - it.lo).max_element();
            let mid = it.p.flags.contains(PrimFlags::MID_ONLY)
                || (it.p.flags.contains(PrimFlags::ENVELOPE)
                    && !it.p.flags.contains(PrimFlags::INTERIOR));
            if ext >= 16.0 {
                r.add("H large drawn pieces (>=16 m)", 1);
                if !mid && !it.p.flags.contains(PrimFlags::INTERIOR) {
                    r.add(
                        format!(
                            "H large piece absent from the mid band {} {}",
                            it.cat,
                            kind_name(it.p.kind)
                        ),
                        1,
                    );
                }
            }
        }
    }
    r
}

/// Counts the faults of `cells` on `threads` threads; `l3` adds the interiors.
pub fn audit_cells(seed: WorldSeed, cells: &[I64Vec3], threads: usize, l3: bool) -> AuditReport {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cache = L1Cache::default();
    let next = AtomicUsize::new(0);
    let total = Mutex::new(AuditReport::default());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&c) = cells.get(i) else { break };
                    let r = audit_cell(seed, &cache, c, l3);
                    if let Ok(mut t) = total.lock() {
                        t.merge(r);
                    }
                }
            });
        }
    });
    total.into_inner().unwrap_or_default()
}

/// Parses a cell list: one `x,y,z` per line; blank lines and `#` comments are skipped.
pub fn parse_cells(text: &str) -> Result<Vec<I64Vec3>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let v: Vec<i64> = line
            .split(',')
            .map(|x| x.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("line {}: {e}", n + 1))?;
        let [x, y, z] = v[..] else {
            return Err(format!("line {}: need x,y,z", n + 1));
        };
        out.push(I64Vec3::new(x, y, z));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELLS: &str = include_str!("../../../docs/playtests/2026-10-22-audit/cells.txt");

    #[test]
    fn the_cell_list_parses_and_comments_are_skipped() {
        let all = parse_cells(CELLS).unwrap();
        assert_eq!(all.len(), 119);
        assert_eq!(
            parse_cells("# a\n1,2,3  # b\n\n-4, 5,6\n").unwrap(),
            vec![I64Vec3::new(1, 2, 3), I64Vec3::new(-4, 5, 6)]
        );
        assert!(parse_cells("1,2").is_err());
    }

    /// A sample of the fixed list runs every counter's measuring loop; the numbers themselves are
    /// recorded in `docs/playtests/web.md`, not asserted.
    #[test]
    fn a_sample_of_the_fixed_cells_is_measured() {
        let cells: Vec<I64Vec3> = parse_cells(CELLS).unwrap().into_iter().take(3).collect();
        let r = audit_cells(WorldSeed(0), &cells, 3, true);
        assert_eq!(r.cells, 3);
        for key in [
            "A samples",
            "A walks",
            "B components",
            "C masses",
            "D legs",
            "R rails",
        ] {
            assert!(
                r.counts.contains_key(key),
                "no counter {key}: {:?}",
                r.counts.keys()
            );
        }
    }

    /// The walk line of a rise in a turned lattice has a floor under it (PW.13.3).
    #[test]
    #[ignore = "PW.13.3"]
    fn rise_walk_lines_in_a_turned_lattice_have_a_floor_under_them() {
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        let (mut samples, mut floored, mut turned_cells) = (0u32, 0u32, 0);
        for c in parse_cells(CELLS).unwrap() {
            let t = tissue_cell(seed, cache.tissue(), CellKey::new(Level::L2, c));
            let [ex, ey, _] = t.lattice.frame.axes();
            if ex.x >= 0.9999 || ey.y < 0.999 || !t.paths.walks.iter().any(|w| w.rise) {
                continue;
            }
            turned_cells += 1;
            let mut own: Vec<Primitive> = Vec::new();
            for (rg, _) in &t.paths.owners {
                own.extend(t.paths.prims[rg.clone()].iter().copied());
            }
            for w in t.paths.walks.iter().filter(|w| w.rise) {
                for q in w.points.iter().step_by(2) {
                    samples += 1;
                    let has = [0.05f32, 0.2, 0.4].iter().any(|&h| {
                        let x = v(*q) - Vec3::Y * h;
                        own.iter().any(|m| {
                            !m.flags.contains(PrimFlags::NO_COLLIDE) && m.shape.contains(x)
                        })
                    });
                    floored += u32::from(has);
                }
            }
            if turned_cells == 6 {
                break;
            }
        }
        assert!(turned_cells > 0, "no turned cell with a rise in the list");
        assert!(
            f64::from(floored) >= 0.95 * f64::from(samples),
            "{floored} of {samples} rise samples have a floor"
        );
    }
}
