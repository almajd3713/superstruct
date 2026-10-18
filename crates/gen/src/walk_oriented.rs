//! Walkability on oriented plates (`primitives.md` §9.7, TEC-06).
//!
//! Axis-aligned content keeps the fast plate path of [`crate::walk`]. Oriented content (any
//! `Shape::Oriented` piece) is walked on a **sample graph**: each walkable surface of the piece,
//! taken in its own frame, is laid with nodes every 0.5 m. A node exists where the surface is
//! within the slope limit, inside the piece's clip and has headroom for the capsule, tested
//! against the real solids with [`Shape::contains`]. Nodes join their neighbours on the same
//! surface, and nodes of different surfaces or axis-aligned plates that meet within a step.
//!
//! Headroom is measured along world +Y, the walker's up until gravity wells make it local
//! (`gravity.md` §6). Like the plate path, the test only ever removes walkable area, never adds.

// Casts are of cell-local quanta (below 2²⁰) and bucket indices.
#![allow(clippy::cast_possible_truncation, clippy::too_many_lines)]

use glam::{DVec3, Vec3};

use crate::prim::{Primitive, Q, Q_PER_M, Shape};
use crate::walk::{HEADROOM, Nav, Plate, STEP_UP, surfaces};

/// Node spacing on a surface: 0.5 m.
pub const SPACING: Q = 8;
/// Steepest walkable incline, `tan 40°`, as in [`crate::walk`].
const MAX_TAN: f64 = 0.839;
/// Plan reach within which two surfaces count as touching: a node cell plus a half.
const TOUCH: f64 = 12.0;
/// Bucket size of the spatial hash, quanta.
const BUCKET: f64 = 16.0;

/// A point of a walkable oriented surface, in cell-local quanta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ONode {
    pub pos: [f64; 3],
    /// Index of the primitive the surface belongs to.
    pub source: usize,
}

/// The nav graph of a set of primitives that may include oriented ones. Indices of the
/// `component` and `edges` run over the axis-aligned plates first (`0..plates`), then the nodes.
#[derive(Debug, Clone)]
pub struct OrientedNav {
    pub nav: Nav,
    pub nodes: Vec<ONode>,
    pub edges: Vec<(usize, usize)>,
    /// Component id per plate and node: the smallest index in its component.
    pub component: Vec<usize>,
}

fn collidable(p: &Primitive) -> bool {
    !matches!(p.shape, Shape::LiftShaft { .. })
        && !p.flags.contains(crate::prim::PrimFlags::NO_COLLIDE)
}

fn metres(q: [f64; 3]) -> Vec3 {
    let s = f64::from(Q_PER_M);
    Vec3::new((q[0] / s) as f32, (q[1] / s) as f32, (q[2] / s) as f32)
}

/// Whether the capsule has room above `at` (quanta): no collidable solid but `skip` in the column.
fn has_headroom(prims: &[Primitive], skip: usize, at: [f64; 3]) -> bool {
    let probes = [6.0, 12.0, 18.0, 24.0, f64::from(HEADROOM)];
    prims
        .iter()
        .enumerate()
        .filter(|&(j, p)| j != skip && collidable(p))
        .all(|(_, p)| {
            let b = p.aabb();
            let (lo, hi) = (b.min, b.max);
            // The column is one quantum wide: skip solids whose box misses it in plan or height.
            if at[0] < f64::from(lo.x) - 1.0
                || at[0] > f64::from(hi.x) + 1.0
                || at[2] < f64::from(lo.z) - 1.0
                || at[2] > f64::from(hi.z) + 1.0
                || f64::from(hi.y) < at[1]
                || f64::from(lo.y) > at[1] + f64::from(HEADROOM)
            {
                return true;
            }
            probes
                .iter()
                .all(|&dy| !p.shape.contains(metres([at[0], at[1] + dy, at[2]])))
        })
}

/// The nodes of every walkable surface of the oriented primitive `index`.
fn surface_nodes(prims: &[Primitive], index: usize) -> Vec<ONode> {
    let Shape::Oriented(o) = prims[index].shape else {
        return Vec::new();
    };
    let local = Primitive {
        shape: o.local.to_shape(),
        ..prims[index]
    };
    let [ex, ey, ez] = o.frame.axes();
    let origin = DVec3::new(
        f64::from(o.origin.x),
        f64::from(o.origin.y),
        f64::from(o.origin.z),
    );
    let (cmin, cmax) = (o.clip.min, o.clip.max);
    let mut nodes = Vec::new();
    for plate in surfaces(&[local]) {
        let r = plate.rect;
        let mut u = r.x0 + SPACING / 2;
        while u < r.x1 {
            let mut v = r.z0 + SPACING / 2;
            while v < r.z1 {
                let h = plate.height_at(u, v);
                let w = origin + ex * f64::from(u) + ey * h + ez * f64::from(v);
                let inside = w.x >= f64::from(cmin.x)
                    && w.x < f64::from(cmax.x)
                    && w.y >= f64::from(cmin.y)
                    && w.y < f64::from(cmax.y)
                    && w.z >= f64::from(cmin.z)
                    && w.z < f64::from(cmax.z);
                if inside && walkable_slope(&plate, &ex, &ey, &ez) {
                    let at = [w.x, w.y, w.z];
                    if has_headroom(prims, index, at) {
                        nodes.push(ONode {
                            pos: at,
                            source: index,
                        });
                    }
                }
                v += SPACING;
            }
            u += SPACING;
        }
    }
    nodes
}

/// Whether the plate's surface, in the world, is within the slope limit of +Y.
fn walkable_slope(plate: &Plate, ex: &DVec3, ey: &DVec3, ez: &DVec3) -> bool {
    use crate::prim::Dir4;
    // Local normal of the plane: flat is +y; a rise of dy over run dx along the ascent has the
    // normal (-dy, run, 0) rotated into the rise axis.
    let n = match plate.rise {
        None => DVec3::new(0.0, 1.0, 0.0),
        Some(rise) => {
            let r = plate.rect;
            let dy = f64::from(plate.y1 - plate.y0);
            let (run, axis_x) = match rise {
                Dir4::PosX | Dir4::NegX => (f64::from(r.x1 - r.x0), true),
                Dir4::PosZ | Dir4::NegZ => (f64::from(r.z1 - r.z0), false),
            };
            let sign = match rise {
                Dir4::PosX | Dir4::PosZ => 1.0,
                Dir4::NegX | Dir4::NegZ => -1.0,
            };
            if axis_x {
                DVec3::new(-dy * sign, run, 0.0)
            } else {
                DVec3::new(0.0, run, -dy * sign)
            }
        }
    };
    let w = *ex * n.x + *ey * n.y + *ez * n.z;
    w.y > 0.0 && (w.x * w.x + w.z * w.z).sqrt() <= MAX_TAN * w.y
}

impl OrientedNav {
    pub fn build(prims: &[Primitive]) -> Self {
        let axis: Vec<Primitive> = prims
            .iter()
            .map(|p| match p.shape {
                // Oriented pieces stay as blockers through their bounding box for the plate
                // path, which only removes walkable area.
                Shape::Oriented(_) => Primitive {
                    shape: Shape::Box { aabb: p.aabb() },
                    ..*p
                },
                _ => *p,
            })
            .collect();
        let nav = Nav::build(&axis);
        let plates = nav.plates.len();
        let mut nodes: Vec<ONode> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for i in 0..prims.len() {
            if matches!(prims[i].shape, Shape::Oriented(_)) && collidable(&prims[i]) {
                nodes.extend(surface_nodes(prims, i));
            }
        }
        let n = plates + nodes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let find = |p: &mut Vec<usize>, mut i: usize| {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        };
        for &(a, b) in &nav.edges {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        }
        edges.extend_from_slice(&nav.edges);
        let bucket = |x: f64, z: f64| ((x / BUCKET).floor() as i64, (z / BUCKET).floor() as i64);
        let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
            std::collections::HashMap::new();
        for (k, nd) in nodes.iter().enumerate() {
            grid.entry(bucket(nd.pos[0], nd.pos[2]))
                .or_default()
                .push(k);
        }
        let link = |parent: &mut Vec<usize>, edges: &mut Vec<(usize, usize)>, a, b| {
            edges.push((a, b));
            let (ra, rb) = (find(parent, a), find(parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        };
        for (k, nd) in nodes.iter().enumerate() {
            let (bx, bz) = bucket(nd.pos[0], nd.pos[2]);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = grid.get(&(bx + dx, bz + dz)) else {
                        continue;
                    };
                    for &m in list.iter().filter(|&&m| m > k) {
                        let other = &nodes[m];
                        let (ex, ez) = (nd.pos[0] - other.pos[0], nd.pos[2] - other.pos[2]);
                        let plan = (ex * ex + ez * ez).sqrt();
                        let same = nd.source == other.source;
                        // Neighbours on one surface are a cell apart; different surfaces meet
                        // wherever their nodes come close in plan and within a step in height.
                        let reach = if same {
                            f64::from(SPACING) * 1.01
                        } else {
                            TOUCH
                        };
                        if plan <= reach && (nd.pos[1] - other.pos[1]).abs() <= f64::from(STEP_UP) {
                            link(&mut parent, &mut edges, plates + k, plates + m);
                        }
                    }
                }
            }
            for (i, plate) in nav.plates.iter().enumerate() {
                let r = plate.rect;
                let reach = 6.0;
                let (x, z) = (nd.pos[0], nd.pos[2]);
                if x < f64::from(r.x0) - reach
                    || x > f64::from(r.x1) + reach
                    || z < f64::from(r.z0) - reach
                    || z > f64::from(r.z1) + reach
                {
                    continue;
                }
                let cx = x.clamp(f64::from(r.x0), f64::from(r.x1));
                let cz = z.clamp(f64::from(r.z0), f64::from(r.z1));
                // `height_at` takes whole quanta; the clamp keeps the point on the plate.
                let h = plate.height_at(cx.round() as Q, cz.round() as Q);
                if (h - nd.pos[1]).abs() <= f64::from(STEP_UP) {
                    link(&mut parent, &mut edges, i, plates + k);
                }
            }
        }
        let component = (0..n).map(|i| find(&mut parent, i)).collect();
        Self {
            nav,
            nodes,
            edges,
            component,
        }
    }

    /// Whether two entries (plate index, or `plates + node index`) are in one component.
    pub fn connected(&self, a: usize, b: usize) -> bool {
        self.component[a] == self.component[b]
    }

    /// Index of node `k` in the unified numbering.
    pub fn node(&self, k: usize) -> usize {
        self.nav.plates.len() + k
    }

    /// The nearest node to `pos` (quanta) within `max_q`, as a unified index.
    pub fn nearest_node(&self, pos: [f64; 3], max_q: f64) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(k, n)| {
                let d = DVec3::from(n.pos) - DVec3::from(pos);
                (d.length(), k)
            })
            .filter(|&(d, _)| d <= max_q)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, k)| self.node(k))
    }

    /// The axis-aligned plate whose component holds `(x, z)` at about height `y` (quanta).
    pub fn plate_at(&self, x: Q, z: Q, y: Q) -> Option<usize> {
        self.nav.plates.iter().position(|p| {
            let r = p.rect;
            r.x0 <= x
                && x < r.x1
                && r.z0 <= z
                && z < r.z1
                && (p.height_at(x, z) - f64::from(y)).abs() <= f64::from(STEP_UP)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orient::{
        FacetRing, Frame, HelixStair, LocalShape, Oriented, facet_ring, helix_stair,
    };
    use crate::prim::{FeatureKind, MaterialId, PrimFlags, QAabb, QVec3};

    const M: i32 = 16;

    fn q3(x: i32, y: i32, z: i32) -> QVec3 {
        QVec3::new(x * M, y * M, z * M)
    }

    fn cell() -> QAabb {
        QAabb::new(q3(0, 0, 0), q3(64, 64, 64))
    }

    fn prim(shape: Shape) -> Primitive {
        Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        }
    }

    /// A floor with its top at y = 0 over the whole cell.
    fn floor() -> Primitive {
        prim(Shape::Box {
            aabb: QAabb::new(q3(0, -2, 0), q3(64, 0, 64)),
        })
    }

    fn oriented(frame: Frame, origin: QVec3, local: QAabb) -> Primitive {
        prim(Shape::Oriented(Oriented {
            frame,
            origin,
            local: LocalShape::Box { aabb: local },
            clip: cell(),
        }))
    }

    /// A slab 4 m wide whose top surface is the local y = 0, `len` metres long.
    fn slab(frame: Frame, origin: QVec3, len: i32) -> Primitive {
        oriented(
            frame,
            origin,
            QAabb::new(QVec3::new(0, -8, -2 * M), QVec3::new(len * M, 0, 2 * M)),
        )
    }

    fn floor_node(nav: &OrientedNav) -> usize {
        nav.plate_at(32 * M, 56 * M, 0).expect("floor plate")
    }

    #[test]
    fn a_turned_slab_on_the_floor_is_walkable_and_joined() {
        // Heading 5 (about 28°), flat on the floor: its top is at y = 0.
        let nav = OrientedNav::build(&[floor(), slab(Frame::table(5, 0), q3(20, 0, 20), 12)]);
        assert!(!nav.nodes.is_empty());
        let f = floor_node(&nav);
        assert!(
            nav.nodes
                .iter()
                .enumerate()
                .all(|(k, _)| nav.connected(f, nav.node(k)))
        );
    }

    #[test]
    fn a_gentle_tilt_is_climbed_and_a_steep_one_is_not() {
        // Tilt 4 is 20°: walkable. Tilt 6 (30°) too. Direction (1,1,0) is 45°: refused.
        for tilt in [4u8, 6] {
            let nav =
                OrientedNav::build(&[floor(), slab(Frame::table(0, tilt), q3(10, 0, 30), 20)]);
            assert!(nav.nodes.len() > 100, "tilt {tilt}");
            let f = floor_node(&nav);
            assert!(
                nav.nodes
                    .iter()
                    .enumerate()
                    .all(|(k, _)| nav.connected(f, nav.node(k)))
            );
            let top = nav.nodes.iter().map(|n| n.pos[1]).fold(f64::MIN, f64::max);
            assert!(top > 5.0 * f64::from(M), "tilt {tilt} only reaches {top}");
        }
        let nav = OrientedNav::build(&[floor(), slab(Frame::dir([1, 1, 0]), q3(10, 0, 30), 20)]);
        assert!(
            nav.nodes.is_empty(),
            "a 45 degree slab must not be walkable"
        );
    }

    #[test]
    fn a_platform_high_up_is_disconnected_until_a_slab_reaches_it() {
        let platform = oriented(
            Frame::table(0, 0),
            q3(36, 6, 20),
            QAabb::new(QVec3::new(0, -8, 0), QVec3::new(8 * M, 0, 8 * M)),
        );
        let alone = OrientedNav::build(&[floor(), platform]);
        let f = floor_node(&alone);
        assert!(
            alone
                .nodes
                .iter()
                .enumerate()
                .all(|(k, _)| !alone.connected(f, alone.node(k)))
        );
        // A 20° slab (tilt 4) from the floor reaches 6 m at x = 36.5, on the platform's edge.
        let ramp = slab(Frame::table(0, 4), q3(20, 0, 24), 20);
        let nav = OrientedNav::build(&[floor(), platform, ramp]);
        let f = floor_node(&nav);
        let on_platform = nav
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.source == 1)
            .collect::<Vec<_>>();
        assert!(!on_platform.is_empty());
        let joined = on_platform
            .iter()
            .filter(|(k, _)| nav.connected(f, nav.node(*k)))
            .count();
        assert!(joined > 0, "the ramp should reach the platform");
    }

    #[test]
    fn solid_overhead_removes_nodes() {
        let low_roof = prim(Shape::Box {
            aabb: QAabb::new(q3(0, 2, 0), q3(64, 3, 64)),
        });
        let open = OrientedNav::build(&[floor(), slab(Frame::table(5, 0), q3(20, 0, 20), 12)]);
        // The roof is 2 m up: 1.8 m of headroom plus the 0.2 m margin to spare, nothing removed.
        let high = OrientedNav::build(&[
            floor(),
            slab(Frame::table(5, 0), q3(20, 0, 20), 12),
            low_roof,
        ]);
        assert_eq!(open.nodes.len(), high.nodes.len());
        let tight = prim(Shape::Box {
            aabb: QAabb::new(q3(0, 1, 0), q3(64, 2, 64)),
        });
        let none =
            OrientedNav::build(&[floor(), slab(Frame::table(5, 0), q3(20, 0, 20), 12), tight]);
        assert!(
            none.nodes.is_empty(),
            "a 1 m ceiling leaves no walkable surface"
        );
    }

    #[test]
    fn a_helical_stair_joins_the_floor_to_a_landing() {
        let clip = cell();
        let mut prims = vec![floor()];
        prims.extend(helix_stair(
            &HelixStair {
                axis: q3(32, 0, 32),
                inner: 2 * M,
                width: 4 * M,
                per_turn: 16,
                steps: 48,
                rise: 3,
                thick: 8,
                start: 0,
                sense: 1,
                material: MaterialId::METAL,
                kind: FeatureKind::STAIR_TOWER,
                flags: PrimFlags::NONE,
            },
            clip,
        ));
        // A landing at the top of the stair, 48 * 3 / 16 = 9 m up, beside the last tread.
        let nav = OrientedNav::build(&prims);
        let f = nav.plate_at(8 * M, 8 * M, 0).expect("floor");
        let top = nav
            .nodes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.pos[1].total_cmp(&b.1.pos[1]))
            .expect("nodes");
        assert!(
            top.1.pos[1] > 8.5 * f64::from(M),
            "stair top at {}",
            top.1.pos[1]
        );
        assert!(
            nav.connected(f, nav.node(top.0)),
            "the top tread is not reachable from the floor"
        );
    }

    #[test]
    fn a_closed_facet_ring_separates_inside_from_outside() {
        let mut prims = vec![floor()];
        prims.extend(facet_ring(
            &FacetRing {
                axis: q3(32, 0, 32),
                radius: 10 * M,
                thick: M,
                height: 4 * M,
                n: 16,
                start: 0,
                material: MaterialId::CONCRETE,
                kind: FeatureKind::MASS,
                flags: PrimFlags::NONE,
            },
            cell(),
        ));
        let nav = OrientedNav::build(&prims);
        // The floor plate loses the footprint of each wall facet (their bounding boxes), so the
        // inner and outer floor are separated only if the ring is closed: they are.
        let inside = nav.plate_at(32 * M, 32 * M, 0);
        let outside = nav.plate_at(2 * M, 2 * M, 0);
        if let (Some(i), Some(o)) = (inside, outside) {
            assert!(
                !nav.connected(i, o),
                "a closed ring should separate inside from outside"
            );
        }
    }
}
