//! Chunk mesher: turns a cell's primitives into one flat-shaded mesh, dropping box faces that are
//! buried against other boxes. Coordinates stay in integer quanta until a quad is emitted, so
//! touching faces cancel exactly (`docs/spec/primitives.md` §1).

use std::collections::BTreeMap;

use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::orient::{LocalShape, Oriented};
use worldgen::prim::{Dir4, Primitive, Q, Q_PER_M, QAabb, Shape};

use crate::mesh::Mesh;
use crate::primmesh;
use crate::vertex::Vertex;

/// Solid boxes the mesher sees, including each stair step.
#[derive(Clone, Copy)]
struct SolidBox {
    aabb: QAabb,
    material: u16,
    tag: u16,
}

/// A face-plane rectangle in quanta, half-open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    u0: Q,
    v0: Q,
    u1: Q,
    v1: Q,
}

impl Rect {
    fn is_empty(self) -> bool {
        self.u0 >= self.u1 || self.v0 >= self.v1
    }

    fn overlaps(self, o: Self) -> bool {
        self.u0 < o.u1 && o.u0 < self.u1 && self.v0 < o.v1 && o.v0 < self.v1
    }
}

/// `r` minus `hole`, as at most four disjoint rectangles pushed to `out`.
fn subtract(r: Rect, hole: Rect, out: &mut Vec<Rect>) {
    if !r.overlaps(hole) {
        out.push(r);
        return;
    }
    let (hu0, hu1) = (hole.u0.max(r.u0), hole.u1.min(r.u1));
    for piece in [
        Rect {
            u0: r.u0,
            v0: r.v0,
            u1: r.u1,
            v1: hole.v0,
        },
        Rect {
            u0: r.u0,
            v0: hole.v1,
            u1: r.u1,
            v1: r.v1,
        },
        Rect {
            u0: r.u0,
            v0: hole.v0.max(r.v0),
            u1: hu0,
            v1: hole.v1.min(r.v1),
        },
        Rect {
            u0: hu1,
            v0: hole.v0.max(r.v0),
            u1: r.u1,
            v1: hole.v1.min(r.v1),
        },
    ] {
        if !piece.is_empty() {
            out.push(piece);
        }
    }
}

const fn cycle(a: Axis) -> (Axis, Axis) {
    match a {
        Axis::X => (Axis::Y, Axis::Z),
        Axis::Y => (Axis::Z, Axis::X),
        Axis::Z => (Axis::X, Axis::Y),
    }
}

fn axis_vec(a: Axis) -> Vec3 {
    match a {
        Axis::X => Vec3::X,
        Axis::Y => Vec3::Y,
        Axis::Z => Vec3::Z,
    }
}

fn metres(q: Q) -> f32 {
    q as f32 / Q_PER_M as f32
}

fn rect_of(b: &QAabb, u: Axis, v: Axis) -> Rect {
    Rect {
        u0: b.min.get(u),
        v0: b.min.get(v),
        u1: b.max.get(u),
        v1: b.max.get(v),
    }
}

/// The steps of a stair run as boxes, in whole quanta.
fn stair_boxes(aabb: QAabb, rise: Dir4, steps: u16, mut push: impl FnMut(QAabb)) {
    let n = Q::from(steps.max(1));
    let run_axis = rise.axis();
    let positive = matches!(rise, Dir4::PosX | Dir4::PosZ);
    let (run, h) = (aabb.size(run_axis), aabb.size(Axis::Y));
    for i in 0..n {
        let (a, b) = (run * i / n, run * (i + 1) / n);
        let (lo, hi) = if positive { (a, b) } else { (run - b, run - a) };
        let mut min = aabb.min;
        let mut max = aabb.max;
        max.y = min.y + h * (i + 1) / n;
        if run_axis == Axis::X {
            max.x = min.x + hi;
            min.x += lo;
        } else {
            max.z = min.z + hi;
            min.z += lo;
        }
        push(QAabb::new(min, max));
    }
}

/// A face left in at least this many fragments by the boxes beside it is drawn whole.
const WHOLE_FACE_FRAGMENTS: usize = 3;

/// A face plane and its material: axis, side, plane and material with tag.
type QuadKey = (u8, bool, Q, u32);

/// Sets of at least this many boxes are culled on the thread pool.
const PARALLEL_BOXES: usize = 4096;

/// The smallest broadphase cell edge in quanta (8 m); a grid of many small boxes uses more.
const GRID_MIN: Q = 128;

/// Grid cell of a non-negative offset in quanta; negative offsets clamp to the first cell.
fn grid_index(q: Q, edge: Q) -> usize {
    usize::try_from(q.max(0) / edge).unwrap_or(0)
}

/// Uniform grid over the boxes so a face only tests nearby occluders. Cells hold ascending box
/// indices, and queries return sorted, deduplicated indices, so output order does not depend on
/// the grid.
struct BoxGrid {
    origin: [Q; 3],
    dims: [usize; 3],
    /// Cell edge in quanta: as small as keeps the cells to a few a box.
    edge: Q,
    cells: Vec<Vec<u32>>,
}

impl BoxGrid {
    fn new(boxes: &[SolidBox]) -> Self {
        let mut lo = [0; 3];
        let mut hi = [0; 3];
        if let Some(first) = boxes.first() {
            for (k, a) in [Axis::X, Axis::Y, Axis::Z].into_iter().enumerate() {
                lo[k] = first.aabb.min.get(a);
                hi[k] = first.aabb.max.get(a);
            }
        }
        for b in boxes {
            for (k, a) in [Axis::X, Axis::Y, Axis::Z].into_iter().enumerate() {
                lo[k] = lo[k].min(b.aabb.min.get(a));
                hi[k] = hi[k].max(b.aabb.max.get(a));
            }
        }
        let mut edge = GRID_MIN;
        let dims_at = |edge: Q| -> [usize; 3] {
            std::array::from_fn(|k| grid_index(hi[k] - lo[k], edge) + 1)
        };
        while dims_at(edge).iter().product::<usize>() > 4 * boxes.len() + 64 && edge < 1 << 20 {
            edge *= 2;
        }
        let dims = dims_at(edge);
        let mut grid = Self {
            origin: lo,
            dims,
            edge,
            cells: vec![Vec::new(); dims[0] * dims[1] * dims[2]],
        };
        for (i, b) in boxes.iter().enumerate() {
            grid.for_each_cell(&b.aabb, |c| c.push(i as u32));
        }
        grid
    }

    fn for_each_cell(&mut self, aabb: &QAabb, mut f: impl FnMut(&mut Vec<u32>)) {
        let range = |k: usize, a: Axis| {
            let lo = grid_index(aabb.min.get(a) - 1 - self.origin[k], self.edge);
            let hi =
                grid_index(aabb.max.get(a) + 1 - self.origin[k], self.edge).min(self.dims[k] - 1);
            (lo.min(self.dims[k] - 1), hi)
        };
        let (x, y, z) = (range(0, Axis::X), range(1, Axis::Y), range(2, Axis::Z));
        for iz in z.0..=z.1 {
            for iy in y.0..=y.1 {
                for ix in x.0..=x.1 {
                    f(&mut self.cells[(iz * self.dims[1] + iy) * self.dims[0] + ix]);
                }
            }
        }
    }

    fn for_each_cell_ref(&self, aabb: &QAabb, mut f: impl FnMut(&Vec<u32>)) {
        let range = |k: usize, a: Axis| {
            let lo = grid_index(aabb.min.get(a) - 1 - self.origin[k], self.edge);
            let hi =
                grid_index(aabb.max.get(a) + 1 - self.origin[k], self.edge).min(self.dims[k] - 1);
            (lo.min(self.dims[k] - 1), hi)
        };
        let (x, y, z) = (range(0, Axis::X), range(1, Axis::Y), range(2, Axis::Z));
        for iz in z.0..=z.1 {
            for iy in y.0..=y.1 {
                for ix in x.0..=x.1 {
                    f(&self.cells[(iz * self.dims[1] + iy) * self.dims[0] + ix]);
                }
            }
        }
    }

    /// Boxes whose cells touch `aabb` grown by one quantum, into `out`.
    fn query(&self, aabb: &QAabb, out: &mut Vec<u32>) {
        out.clear();
        self.for_each_cell_ref(aabb, |c| out.extend_from_slice(c));
        out.sort_unstable();
        out.dedup();
    }
}

pub struct ChunkMeshStats {
    pub boxes: usize,
    /// Triangles the same boxes would have without face removal.
    pub tris_unculled: usize,
}

/// Meshes `prims` (cell-local metres). Boxes and stair steps are culled against each other;
/// ramps and cylinders are emitted whole and occlude nothing.
/// A plane's quads by material.
type Layers = Vec<(u32, Vec<Rect>)>;

#[allow(clippy::too_many_lines)] // cull, de-overlap and merge in one pass
pub fn mesh_primitives(prims: &[Primitive]) -> (Mesh, ChunkMeshStats) {
    let mut mesh = Mesh::default();
    let mut boxes: Vec<SolidBox> = Vec::new();
    // Turned boxes of one frame and origin (the fragments a bored beam is cut into) are culled
    // against each other in their own frame, like plain boxes (PW.4.27).
    let mut turned: BTreeMap<TurnedKey, (Oriented, Vec<SolidBox>)> = BTreeMap::new();
    for p in prims {
        let material = p.material.0;
        let tag = if crate::debugview::enabled() {
            crate::debugview::prim_tag(p)
        } else {
            0
        };
        if p.kind == worldgen::prim::FeatureKind::RAILING {
            primmesh::add_primitive(&mut mesh, p);
            continue;
        }
        match p.shape {
            Shape::Box { aabb } => boxes.push(SolidBox {
                aabb,
                material,
                tag,
            }),
            Shape::StairRun { aabb, rise, steps } => {
                stair_boxes(aabb, rise, steps, |aabb| {
                    boxes.push(SolidBox {
                        aabb,
                        material,
                        tag,
                    });
                });
            }
            Shape::Oriented(o) if matches!(o.local, LocalShape::Box { .. }) => {
                let LocalShape::Box { aabb } = o.local else {
                    continue;
                };
                turned
                    .entry(turned_key(&o))
                    .or_insert_with(|| (o, Vec::new()))
                    .1
                    .push(SolidBox {
                        aabb,
                        material,
                        tag,
                    });
            }
            Shape::Ramp { .. } | Shape::Cylinder { .. } | Shape::Oriented(_) => {
                primmesh::add_primitive(&mut mesh, p);
            }
            Shape::LiftShaft { .. } => {}
        }
    }
    for (_, (o, group)) in turned {
        if group.len() < 4 {
            for b in group {
                let prim = Primitive {
                    shape: Shape::Oriented(Oriented {
                        local: LocalShape::Box { aabb: b.aabb },
                        ..o
                    }),
                    material: worldgen::prim::MaterialId(b.material),
                    kind: worldgen::prim::FeatureKind::MASS,
                    flags: worldgen::prim::PrimFlags::NONE,
                };
                primmesh::add_primitive(&mut mesh, &prim);
            }
            continue;
        }
        let mut local = Mesh::default();
        mesh_boxes(&group, &mut local);
        primmesh::turn_group(&mut mesh, &local, &o);
    }
    let stats = mesh_boxes(&boxes, &mut mesh);
    (mesh, stats)
}

/// A frame, an origin and a clip box, as a map key.
type TurnedKey = (i32, [i32; 4], [i32; 3], [i32; 6]);

fn turned_key(o: &Oriented) -> TurnedKey {
    let (kind, f) = match o.frame {
        worldgen::orient::Frame::Id => (0, [0; 4]),
        worldgen::orient::Frame::Table { yaw, tilt } => {
            (1, [i32::from(yaw), i32::from(tilt), 0, 0])
        }
        worldgen::orient::Frame::Dir { d } => {
            (2, [i32::from(d[0]), i32::from(d[1]), i32::from(d[2]), 0])
        }
    };
    let c = o.clip;
    (
        kind,
        f,
        [o.origin.x, o.origin.y, o.origin.z],
        [c.min.x, c.min.y, c.min.z, c.max.x, c.max.y, c.max.z],
    )
}

/// Culls, de-overlaps and merges the faces of `boxes` into `mesh`.
#[allow(clippy::too_many_lines)] // cull, de-overlap and merge in one pass
fn mesh_boxes(boxes: &[SolidBox], mesh: &mut Mesh) -> ChunkMeshStats {
    let stats = ChunkMeshStats {
        boxes: boxes.len(),
        tris_unculled: boxes.len() * 12,
    };
    let mut quads: BTreeMap<(u8, bool, Q, u32), Vec<Rect>> = BTreeMap::new();
    let grid = BoxGrid::new(boxes);
    // The faces of each box, culled against the boxes beside it. Boxes are independent of each
    // other here, so a big set is done on the thread pool and gathered in box order (the same
    // result whatever the threads do).
    let faces_of = |i: usize| -> Vec<(QuadKey, Vec<Rect>)> {
        let b = &boxes[i];
        let (mut pieces, mut next, mut near) = (Vec::new(), Vec::new(), Vec::new());
        let mut out = Vec::new();
        grid.query(&b.aabb, &mut near);
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let (u, v) = cycle(axis);
            for positive in [true, false] {
                let c = if positive {
                    b.aabb.max.get(axis)
                } else {
                    b.aabb.min.get(axis)
                };
                pieces.clear();
                pieces.push(rect_of(&b.aabb, u, v));
                for &j in &near {
                    let o = &boxes[j as usize];
                    if i == j as usize {
                        continue;
                    }
                    // The slab just beyond the face must lie inside `o`.
                    let (lo, hi) = (o.aabb.min.get(axis), o.aabb.max.get(axis));
                    let beyond = if positive {
                        lo <= c && c < hi
                    } else {
                        lo < c && c <= hi
                    };
                    if !beyond {
                        continue;
                    }
                    let hole = rect_of(&o.aabb, u, v);
                    next.clear();
                    for r in pieces.drain(..) {
                        subtract(r, hole, &mut next);
                    }
                    std::mem::swap(&mut pieces, &mut next);
                    if pieces.is_empty() {
                        break;
                    }
                }
                if !pieces.is_empty() {
                    // A face that other boxes leave in three fragments or more is drawn whole:
                    // what hides behind them is invisible, and the fragments cost more quads
                    // than the face (a plate with a hundred little boxes on it).
                    if pieces.len() >= WHOLE_FACE_FRAGMENTS {
                        pieces.clear();
                        pieces.push(rect_of(&b.aabb, u, v));
                    }
                    out.push((
                        (
                            axis as u8,
                            positive,
                            c,
                            u32::from(b.material) | u32::from(b.tag) << 16,
                        ),
                        pieces.clone(),
                    ));
                }
            }
        }
        out
    };
    let per_box: Vec<Vec<_>> = if boxes.len() >= PARALLEL_BOXES {
        use rayon::prelude::*;
        (0..boxes.len()).into_par_iter().map(faces_of).collect()
    } else {
        (0..boxes.len()).map(faces_of).collect()
    };
    for faces in per_box {
        for (key, rects) in faces {
            quads.entry(key).or_default().extend(rects);
        }
    }
    let mut next = Vec::new();
    // Quads of different materials in one plane that overlap would z-fight: the first material
    // (lowest id) keeps the overlap, the others lose it.
    let mut planes: BTreeMap<(u8, bool, Q), Layers> = BTreeMap::new();
    for ((axis, positive, plane, material), rects) in quads {
        planes
            .entry((axis, positive, plane))
            .or_default()
            .push((material, rects));
    }
    let mut quads: BTreeMap<(u8, bool, Q, u32), Vec<Rect>> = BTreeMap::new();
    for ((axis, positive, plane), layers) in planes {
        let mut covered: Vec<Rect> = Vec::new();
        for (material, rects) in layers {
            let mut kept = rects.clone();
            for hole in &covered {
                next.clear();
                for r in kept.drain(..) {
                    subtract(r, *hole, &mut next);
                }
                std::mem::swap(&mut kept, &mut next);
            }
            covered.extend(rects);
            if !kept.is_empty() {
                quads.insert((axis, positive, plane, material), kept);
            }
        }
    }
    // Coplanar quads that touch along a whole edge become one: boxes laid side by side (plates,
    // steps, tiles) share a plane, and the culling above leaves each face in fragments.
    for ((axis, positive, plane, material), mut rects) in quads {
        merge(&mut rects);
        let axis = [Axis::X, Axis::Y, Axis::Z][usize::from(axis)];
        for r in rects {
            emit_quad(
                mesh,
                axis,
                positive,
                plane,
                r,
                material as u16,
                (material >> 16) as u16,
            );
        }
    }
    stats
}

/// Merges rectangles that share a full edge, along `u` and then `v`, until no pair does.
fn merge(rects: &mut Vec<Rect>) {
    for _ in 0..6 {
        let before = rects.len();
        for along_u in [true, false] {
            rects.sort_unstable_by_key(|r| {
                if along_u {
                    (r.v0, r.v1, r.u0, r.u1)
                } else {
                    (r.u0, r.u1, r.v0, r.v1)
                }
            });
            let mut out: Vec<Rect> = Vec::with_capacity(rects.len());
            for r in rects.drain(..) {
                match out.last_mut() {
                    Some(last)
                        if along_u && last.v0 == r.v0 && last.v1 == r.v1 && last.u1 == r.u0 =>
                    {
                        last.u1 = r.u1;
                    }
                    Some(last)
                        if !along_u && last.u0 == r.u0 && last.u1 == r.u1 && last.v1 == r.v0 =>
                    {
                        last.v1 = r.v1;
                    }
                    _ => out.push(r),
                }
            }
            *rects = out;
        }
        if rects.len() == before {
            break;
        }
    }
}

fn emit_quad(
    mesh: &mut Mesh,
    axis: Axis,
    positive: bool,
    plane: Q,
    r: Rect,
    material: u16,
    tag: u16,
) {
    let (ua, va) = cycle(axis);
    let (ev, eu, en) = (axis_vec(va), axis_vec(ua), axis_vec(axis));
    let at = |u: Q, v: Q| en * metres(plane) + eu * metres(u) + ev * metres(v);
    let normal = if positive { en } else { -en };
    // u × v = axis for the cyclic frame, so this order is counter-clockwise seen from +axis.
    let mut corners = [
        at(r.u0, r.v0),
        at(r.u1, r.v0),
        at(r.u1, r.v1),
        at(r.u0, r.v1),
    ];
    if !positive {
        corners.swap(1, 3);
    }
    let base = mesh.vertices.len() as u32;
    for p in corners {
        let mut v = Vertex::new(p, normal, material);
        v.tag = tag;
        mesh.vertices.push(v);
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Total exposed area of `mesh` in square metres, for tests.
pub fn surface_area(mesh: &Mesh) -> f32 {
    mesh.indices
        .chunks(3)
        .map(|t| {
            let p = |k: usize| Vec3::from_array(mesh.vertices[t[k] as usize].position);
            (p(1) - p(0)).cross(p(2) - p(0)).length() * 0.5
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::oct_decode;
    use worldgen::prim::{FeatureKind, MaterialId, PrimFlags, QVec3};

    fn bx(min: [Q; 3], max: [Q; 3]) -> Primitive {
        Primitive {
            shape: Shape::Box {
                aabb: QAabb::new(
                    QVec3::new(min[0], min[1], min[2]),
                    QVec3::new(max[0], max[1], max[2]),
                ),
            },
            material: MaterialId(3),
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        }
    }

    fn assert_winding(mesh: &Mesh) {
        for tri in mesh.indices.chunks(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from_array(mesh.vertices[i as usize].position))
                .collect();
            let geometric = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            let stored = oct_decode(mesh.vertices[tri[0] as usize].normal);
            assert!(geometric.dot(stored) > 0.99, "{geometric} vs {stored}");
        }
    }

    /// Two boxes of different materials with tops in one plane: the overlap is drawn once.
    #[test]
    fn coplanar_tops_of_two_materials_do_not_overlap() {
        let mut a = bx([0, 0, 0], [64, 16, 64]);
        let mut b = bx([32, 0, 0], [96, 16, 64]);
        a.material = MaterialId(0);
        b.material = MaterialId(1);
        let (mesh, _) = mesh_primitives(&[a, b]);
        let top_area: f32 = mesh
            .indices
            .chunks(3)
            .filter(|t| {
                t.iter()
                    .all(|&i| (mesh.vertices[i as usize].position[1] - 1.0).abs() < 1e-4)
            })
            .map(|t| {
                let p = |k: usize| Vec3::from_array(mesh.vertices[t[k] as usize].position);
                (p(1) - p(0)).cross(p(2) - p(0)).length() * 0.5
            })
            .sum();
        // The union of the two tops: 6 m by 4 m.
        assert!((top_area - 24.0).abs() < 1e-3, "top area {top_area}");
    }

    #[test]
    fn lone_box_has_twelve_triangles_with_normals_and_material() {
        let (mesh, _) = mesh_primitives(&[bx([0, 0, 0], [32, 16, 48])]);
        assert_eq!(mesh.indices.len() / 3, 12);
        assert_eq!(mesh.vertices.len(), 24);
        assert!(mesh.vertices.iter().all(|v| v.material == 3));
        assert_winding(&mesh);
        assert!((surface_area(&mesh) - 2.0 * (2.0 * 1.0 + 2.0 * 3.0 + 1.0 * 3.0)).abs() < 1e-3);
    }

    #[test]
    fn touching_boxes_lose_their_shared_face() {
        let (mesh, _) =
            mesh_primitives(&[bx([0, 0, 0], [16, 16, 16]), bx([16, 0, 0], [32, 16, 16])]);
        // Two 1 m cubes side by side: a 2x1x1 block, whose coplanar unit faces merge into six
        // quads (twelve triangles) of the same total area, 10.
        assert_eq!(mesh.indices.len() / 3, 12);
        assert!((surface_area(&mesh) - 10.0).abs() < 1e-3);
        assert_winding(&mesh);
    }

    #[test]
    fn partial_overlap_leaves_an_l_shaped_face_without_holes() {
        // A 1x1 face half covered by a neighbour: exposed area is exactly the uncovered half.
        let (mesh, _) =
            mesh_primitives(&[bx([0, 0, 0], [16, 16, 16]), bx([16, 0, 0], [32, 8, 16])]);
        let a = 6.0 - 0.5;
        let b = 2.0 * (1.0 * 0.5 + 1.0 * 1.0 + 0.5 * 1.0) - 0.5;
        assert!(
            (surface_area(&mesh) - (a + b)).abs() < 1e-3,
            "{}",
            surface_area(&mesh)
        );
        assert_winding(&mesh);
    }

    #[test]
    #[ignore = "reads the v6 world (lift core, decks, v6 L2/L3), replaced in version 7; rebuilt or deleted with PW.3"]
    fn real_cell_loses_triangles() {
        use worldgen::cell::{CellKey, Level};
        use worldgen::l2::l2_cell;
        use worldgen::seed::WorldSeed;
        let key = CellKey::new(Level::L2, glam::I64Vec3::new(4, -4, 4));
        let out = l2_cell(WorldSeed(0), key);
        let (mesh, stats) = mesh_primitives(&out.primitives);
        assert!(stats.boxes > 10);
        let mut old = Mesh::default();
        for p in &out.primitives {
            primmesh::add_primitive(&mut old, p);
        }
        println!(
            "tris {} -> {}",
            old.indices.len() / 3,
            mesh.indices.len() / 3
        );
        assert!(mesh.indices.len() < old.indices.len());
        assert_winding(&mesh);
    }

    #[test]
    fn stair_steps_drop_buried_faces() {
        let aabb = QAabb::new(QVec3::new(0, 0, 0), QVec3::new(64, 32, 32));
        let prim = Primitive {
            shape: Shape::stair_run(aabb, Dir4::PosX, 4),
            material: MaterialId::CONCRETE,
            kind: FeatureKind::STAIR_TOWER,
            flags: PrimFlags::NONE,
        };
        let (mesh, stats) = mesh_primitives(&[prim]);
        assert_eq!(stats.boxes, 4);
        assert!(mesh.indices.len() / 3 < stats.tris_unculled);
        assert_winding(&mesh);
        // Silhouette area of the staircase: sides 2 * (4 steps' profile), treads, risers, back, bottom.
        let (run, rise, wide) = (4.0_f32, 2.0_f32, 2.0_f32);
        let steps = [0.5, 1.0, 1.5, 2.0];
        let side: f32 = steps.iter().map(|h| 1.0 * h).sum();
        let expected = 2.0 * side + (run * wide) /* bottom */ + rise * wide /* back */
            + run * wide /* treads */ + rise * wide /* risers */;
        assert!(
            (surface_area(&mesh) - expected).abs() < 1e-2,
            "{} vs {expected}",
            surface_area(&mesh)
        );
    }
}

#[cfg(test)]
mod budget {
    use super::*;
    use glam::I64Vec3;
    use worldgen::cell::{CellKey, Level};
    use worldgen::l2_port::L1Cache;
    use worldgen::seed::WorldSeed;

    fn bytes(m: &Mesh) -> usize {
        m.vertices.len() * 20 + m.indices.len() * 4
    }

    /// Exploration helper (PW.3.21c): the mesh bytes of sample L1 and L2 cells, and what the
    /// bytes are made of.
    #[test]
    #[ignore = "measurement: prints the mesh bytes of sample cells"]
    fn print_mesh_bytes() {
        let seed = WorldSeed(0);
        let cache = L1Cache::default();
        for k in 0..4i64 {
            let cell = CellKey::new(Level::L1, I64Vec3::new(k % 2, 0, k / 2));
            let (prims, _) = worldgen::l2_web::l1_primitives_carved(seed, &cache, cell);
            let (m, stats) = mesh_primitives(&prims);
            let oriented = prims
                .iter()
                .filter(|p| matches!(p.shape, Shape::Oriented(_)))
                .count();
            let mut by_kind: BTreeMap<String, (usize, usize)> = BTreeMap::default();
            for p in &prims {
                let mut one = Mesh::default();
                if !matches!(p.shape, Shape::Box { .. } | Shape::StairRun { .. }) {
                    primmesh::add_primitive(&mut one, p);
                    let name = match p.shape {
                        Shape::Cylinder { .. } => "cylinder".to_string(),
                        Shape::Ramp { .. } => "ramp".to_string(),
                        Shape::Oriented(o) => format!(
                            "oriented {}",
                            match o.local {
                                worldgen::orient::LocalShape::Box { .. } => "box",
                                worldgen::orient::LocalShape::Cylinder { .. } => "cylinder",
                                _ => "other",
                            }
                        ),
                        _ => "other".to_string(),
                    };
                    let e = by_kind.entry(name).or_default();
                    e.0 += 1;
                    e.1 += one.vertices.len();
                }
            }
            println!("L1 non-box verts: {by_kind:?}");
            println!(
                "L1 {:?}: {} prims ({} oriented), {} boxes, {} verts, {:.1} MiB",
                cell.coords,
                prims.len(),
                oriented,
                stats.boxes,
                m.vertices.len(),
                bytes(&m) as f64 / 1_048_576.0
            );
        }
        let mut sizes: Vec<f64> = Vec::new();
        for k in 0..120i64 {
            let key = CellKey::new(
                Level::L2,
                I64Vec3::new(k % 6 + 8, k / 6 % 4 + 1, k / 24 + 3),
            );
            let out = worldgen::l2_web::l2_cell_web(seed, &cache, key);
            let (m, _) = mesh_primitives(&out.primitives);
            sizes.push(bytes(&m) as f64 / 1_048_576.0);
        }
        sizes.sort_by(f64::total_cmp);
        println!(
            "L2 MiB over 120 cells: median {:.1}, p90 {:.1}, max {:.1}",
            sizes[60], sizes[108], sizes[119]
        );
    }
}
