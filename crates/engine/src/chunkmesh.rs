//! Chunk mesher: turns a cell's primitives into one flat-shaded mesh, dropping box faces that are
//! buried against other boxes. Coordinates stay in integer quanta until a quad is emitted, so
//! touching faces cancel exactly (`docs/spec/primitives.md` §1).

use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::prim::{Dir4, Primitive, Q, Q_PER_M, QAabb, Shape};

use crate::mesh::Mesh;
use crate::primmesh;
use crate::vertex::Vertex;

/// Solid boxes the mesher sees, including each stair step.
#[derive(Clone, Copy)]
struct SolidBox {
    aabb: QAabb,
    material: u16,
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

/// Broadphase cell edge in quanta (64 m).
const GRID: Q = 1024;

/// Grid cell of a non-negative offset in quanta; negative offsets clamp to the first cell.
fn grid_index(q: Q) -> usize {
    usize::try_from(q.max(0) / GRID).unwrap_or(0)
}

/// Uniform grid over the boxes so a face only tests nearby occluders. Cells hold ascending box
/// indices, and queries return sorted, deduplicated indices, so output order does not depend on
/// the grid.
struct BoxGrid {
    origin: [Q; 3],
    dims: [usize; 3],
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
        let dims = std::array::from_fn(|k| grid_index(hi[k] - lo[k]) + 1);
        let mut grid = Self {
            origin: lo,
            dims,
            cells: vec![Vec::new(); dims[0] * dims[1] * dims[2]],
        };
        for (i, b) in boxes.iter().enumerate() {
            grid.for_each_cell(&b.aabb, |c| c.push(i as u32));
        }
        grid
    }

    fn for_each_cell(&mut self, aabb: &QAabb, mut f: impl FnMut(&mut Vec<u32>)) {
        let range = |k: usize, a: Axis| {
            let lo = grid_index(aabb.min.get(a) - 1 - self.origin[k]);
            let hi = grid_index(aabb.max.get(a) + 1 - self.origin[k]).min(self.dims[k] - 1);
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

    /// Boxes whose cells touch `aabb` grown by one quantum, into `out`.
    fn query(&mut self, aabb: &QAabb, out: &mut Vec<u32>) {
        out.clear();
        self.for_each_cell(aabb, |c| out.extend_from_slice(c));
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
pub fn mesh_primitives(prims: &[Primitive]) -> (Mesh, ChunkMeshStats) {
    let mut mesh = Mesh::default();
    let mut boxes: Vec<SolidBox> = Vec::new();
    for p in prims {
        let material = p.material.0;
        match p.shape {
            Shape::Box { aabb } => boxes.push(SolidBox { aabb, material }),
            Shape::StairRun { aabb, rise, steps } => {
                stair_boxes(aabb, rise, steps, |aabb| {
                    boxes.push(SolidBox { aabb, material });
                });
            }
            Shape::Ramp { .. } | Shape::Cylinder { .. } => primmesh::add_primitive(&mut mesh, p),
            Shape::LiftShaft { .. } => {}
        }
    }
    let stats = ChunkMeshStats {
        boxes: boxes.len(),
        tris_unculled: boxes.len() * 12,
    };
    let mut pieces = Vec::new();
    let mut next = Vec::new();
    let mut near = Vec::new();
    let mut grid = BoxGrid::new(&boxes);
    for (i, b) in boxes.iter().enumerate() {
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
                for r in &pieces {
                    emit_quad(&mut mesh, axis, positive, c, *r, b.material);
                }
            }
        }
    }
    (mesh, stats)
}

fn emit_quad(mesh: &mut Mesh, axis: Axis, positive: bool, plane: Q, r: Rect, material: u16) {
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
        mesh.vertices.push(Vertex::new(p, normal, material));
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
        // Two 1 m cubes side by side: a 2x1x1 block, 10 unit faces, one quad each.
        assert_eq!(mesh.indices.len() / 3, 20);
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
