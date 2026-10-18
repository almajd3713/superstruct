//! Far-band instances. Far L1 structure is only boxes and cylinders, so a chunk is a list of
//! 32-byte instances that a vertex shader scales out of one shared unit box and unit cylinder,
//! about twenty times smaller than meshing the same primitives.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Quat, Vec3};
use worldgen::cell::Axis;
use worldgen::orient::{LocalShape, Oriented};
use worldgen::prim::{Primitive, Q_PER_M, Shape};

use crate::mesh::Mesh;
use crate::vertex::Vertex;

pub const UNIT_CYLINDER_SIDES: usize = 32;
const CYLINDER_BIT: u32 = 1 << 16;
const AXIS_SHIFT: u32 = 17;
/// Set when `origin` and `rot` apply (an oriented primitive, `primitives.md` §9).
const ORIENTED_BIT: u32 = 1 << 19;

/// Mirrored by `FarInstance` in `scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FarInstance {
    /// Minimum corner of the primitive's bounding box, cell-local metres.
    pub min: [f32; 3],
    /// Slot of the owning chunk in the cell table; the renderer fills it in on upload.
    pub cell: u32,
    pub size: [f32; 3],
    /// Material in the low 16 bits, bit 16 set for a cylinder, bits 17-18 its axis, bit 19 for
    /// an oriented primitive.
    pub meta: u32,
    /// Frame origin, cell-local metres. Only read when bit 19 of `meta` is set.
    pub origin: [f32; 3],
    _pad: f32,
    /// Frame rotation as a unit quaternion `(x, y, z, w)`. Identity for axis-aligned instances.
    pub rot: [f32; 4],
}

impl FarInstance {
    /// An axis-aligned instance: `min` and `size` in cell-local metres.
    pub fn axis_aligned(min: [f32; 3], size: [f32; 3], meta: u32) -> Self {
        Self {
            min,
            cell: 0,
            size,
            meta,
            origin: [0.0; 3],
            _pad: 0.0,
            rot: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FarData {
    pub boxes: Vec<FarInstance>,
    pub cylinders: Vec<FarInstance>,
    /// World-space bounds of the oriented instances, which `min` and `size` do not describe.
    pub oriented_bounds: Option<(Vec3, Vec3)>,
}

impl FarData {
    pub fn len(&self) -> usize {
        self.boxes.len() + self.cylinders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> u64 {
        (self.len() * std::mem::size_of::<FarInstance>()) as u64
    }

    /// Bounding sphere over every instance's box: centre xyz, radius w, cell-local.
    pub fn bounding_sphere(&self) -> [f32; 4] {
        let all = || self.boxes.iter().chain(&self.cylinders);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for i in all().filter(|i| i.meta & ORIENTED_BIT == 0) {
            lo = lo.min(Vec3::from(i.min));
            hi = hi.max(Vec3::from(i.min) + Vec3::from(i.size));
        }
        if let Some((a, b)) = self.oriented_bounds {
            lo = lo.min(a);
            hi = hi.max(b);
        }
        if self.is_empty() {
            return [0.0; 4];
        }
        let c = (lo + hi) * 0.5;
        [c.x, c.y, c.z, (hi - lo).length() * 0.5]
    }
}

fn m(q: i32) -> f32 {
    q as f32 / Q_PER_M as f32
}

/// Turns far-visible primitives (cell-local) into instances. Ramps and stairs, which the far
/// set does not contain today, fall back to their bounding boxes; lift shafts are not solid.
pub fn far_instances(prims: &[Primitive]) -> FarData {
    let mut data = FarData::default();
    for p in prims {
        if let Shape::Oriented(o) = p.shape {
            push_oriented(&mut data, &o, p.material.0);
            continue;
        }
        let (cylinder, axis) = match p.shape {
            Shape::Cylinder { axis, .. } => (true, axis),
            Shape::LiftShaft { .. } => continue,
            _ => (false, Axis::Y),
        };
        let aabb = p.shape.aabb();
        let inst = FarInstance::axis_aligned(
            [m(aabb.min.x), m(aabb.min.y), m(aabb.min.z)],
            [
                m(aabb.max.x - aabb.min.x),
                m(aabb.max.y - aabb.min.y),
                m(aabb.max.z - aabb.min.z),
            ],
            u32::from(p.material.0) | shape_bits(cylinder, axis),
        );
        if cylinder {
            data.cylinders.push(inst);
        } else {
            data.boxes.push(inst);
        }
    }
    data
}

fn shape_bits(cylinder: bool, axis: Axis) -> u32 {
    let axis_bits = match (cylinder, axis) {
        (false, _) | (_, Axis::X) => 0,
        (_, Axis::Y) => 1,
        (_, Axis::Z) => 2,
    };
    let kind = if cylinder { CYLINDER_BIT } else { 0 };
    kind | (axis_bits << AXIS_SHIFT)
}

/// An oriented primitive as one instance in its own frame. Boxes and cylinders are exact; ramps
/// and stairs fall back to their local bounding box. The instance is the whole solid, not its
/// clipped piece: neighbouring cells draw the same solid, so the overlap is identical triangles.
fn push_oriented(data: &mut FarData, o: &Oriented, material: u16) {
    let (cylinder, axis) = match o.local {
        LocalShape::Cylinder { axis, .. } => (true, axis),
        _ => (false, Axis::Y),
    };
    let local = o.local.aabb();
    let [ex, ey, ez] = o.frame.axes();
    let rot =
        Quat::from_mat3(&Mat3::from_cols(ex.as_vec3(), ey.as_vec3(), ez.as_vec3())).normalize();
    let origin = o.origin.to_metres();
    let inst = FarInstance {
        min: [m(local.min.x), m(local.min.y), m(local.min.z)],
        cell: 0,
        size: [
            m(local.max.x - local.min.x),
            m(local.max.y - local.min.y),
            m(local.max.z - local.min.z),
        ],
        meta: u32::from(material) | shape_bits(cylinder, axis) | ORIENTED_BIT,
        origin: origin.to_array(),
        _pad: 0.0,
        rot: rot.to_array(),
    };
    let b = o.aabb();
    let (lo, hi) = (b.min.to_metres(), b.max.to_metres());
    data.oriented_bounds = Some(match data.oriented_bounds {
        Some((a, c)) => (a.min(lo), c.max(hi)),
        None => (lo, hi),
    });
    if cylinder {
        data.cylinders.push(inst);
    } else {
        data.boxes.push(inst);
    }
}

/// A cylinder along y inside the unit box (radius 0.5), with smooth radial side normals so a
/// far trunk shades like the horizon's exactly round one; the caps are flat.
fn add_unit_cylinder_smooth(mesh: &mut Mesh, sides: usize) {
    let ring = |k: usize| {
        let a = std::f32::consts::TAU * k as f32 / sides as f32;
        Vec3::new(a.cos(), 0.0, a.sin())
    };
    let base = mesh.vertices.len() as u32;
    for k in 0..sides {
        let r = ring(k);
        let at = |y: f32| Vec3::new(0.5 + 0.5 * r.x, y, 0.5 + 0.5 * r.z);
        mesh.vertices.push(Vertex::new(at(0.0), r, 0));
        mesh.vertices.push(Vertex::new(at(1.0), r, 0));
    }
    for k in 0..sides as u32 {
        let k1 = (k + 1) % sides as u32;
        let (b0, t0, b1, t1) = (
            base + 2 * k,
            base + 2 * k + 1,
            base + 2 * k1,
            base + 2 * k1 + 1,
        );
        // Counter-clockwise seen from outside, matching the box faces.
        mesh.indices.extend([b0, t0, t1, b0, t1, b1]);
    }
    for (y, normal) in [(0.0, Vec3::NEG_Y), (1.0, Vec3::Y)] {
        let first = mesh.vertices.len() as u32;
        for k in 0..sides {
            let r = ring(k);
            mesh.vertices.push(Vertex::new(
                Vec3::new(0.5 + 0.5 * r.x, y, 0.5 + 0.5 * r.z),
                normal,
                0,
            ));
        }
        for k in 1..sides as u32 - 1 {
            if y == 0.0 {
                mesh.indices.extend([first, first + k + 1, first + k]);
            } else {
                mesh.indices.extend([first, first + k, first + k + 1]);
            }
        }
    }
}

/// The shared unit meshes: a box then a cylinder, in one vertex and one index buffer.
pub struct UnitMeshes {
    pub mesh: Mesh,
    pub box_indices: u32,
    pub box_vertices: u32,
    pub cylinder_indices: u32,
}

pub fn unit_meshes() -> UnitMeshes {
    let mut mesh = Mesh::default();
    mesh.add_box(Vec3::ZERO, Vec3::ONE, 0);
    let (box_vertices, box_indices) = (mesh.vertices.len() as u32, mesh.indices.len() as u32);
    add_unit_cylinder_smooth(&mut mesh, UNIT_CYLINDER_SIDES);
    let cylinder_indices = mesh.indices.len() as u32 - box_indices;
    UnitMeshes {
        mesh,
        box_indices,
        box_vertices,
        cylinder_indices,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // the quanta convert to exactly representable metres
mod tests {
    use super::*;
    use worldgen::prim::{FeatureKind, MaterialId, PrimFlags, QAabb, QVec3};

    fn prim(shape: Shape, material: u16) -> Primitive {
        Primitive {
            shape,
            material: MaterialId(material),
            kind: FeatureKind::TRUNK,
            flags: PrimFlags::FAR_VISIBLE,
        }
    }

    #[test]
    fn boxes_and_cylinders_become_scaled_instances() {
        let b = Shape::Box {
            aabb: QAabb::new(QVec3::new(16, 32, 48), QVec3::new(48, 96, 112)),
        };
        let c = Shape::cylinder(Axis::Z, QVec3::new(160, 160, 0), 320, 80);
        let data = far_instances(&[prim(b, 1), prim(c, 4)]);
        assert_eq!(data.boxes.len(), 1);
        assert_eq!(data.boxes[0].min, [1.0, 2.0, 3.0]);
        assert_eq!(data.boxes[0].size, [2.0, 4.0, 4.0]);
        assert_eq!(data.boxes[0].meta, 1);
        let cyl = data.cylinders[0];
        assert_eq!(cyl.min, [5.0, 5.0, 0.0]);
        assert_eq!(cyl.size, [10.0, 10.0, 20.0]);
        assert_eq!(cyl.meta, 4 | CYLINDER_BIT | (2 << AXIS_SHIFT));
        assert_eq!(data.bytes(), 128, "two 64-byte instances");
    }

    #[test]
    fn unit_meshes_fill_the_unit_box() {
        let u = unit_meshes();
        assert_eq!(u.box_indices, 36);
        assert_eq!(u.box_vertices, 24);
        assert!(u.cylinder_indices > 0);
        for v in &u.mesh.vertices {
            assert!(v.position.iter().all(|c| (-1e-5..=1.0 + 1e-5).contains(c)));
        }
    }

    #[test]
    fn instance_is_64_bytes_like_the_shader() {
        assert_eq!(std::mem::size_of::<FarInstance>(), 64);
    }

    #[test]
    fn oriented_pieces_become_rotated_instances() {
        use worldgen::orient::{Frame, LocalShape};
        let q = |x: i32, y: i32, z: i32| QVec3::new(x * 16, y * 16, z * 16);
        let o = Shape::oriented(
            Frame::table(16, 0),
            q(20, 8, 6),
            LocalShape::Box {
                aabb: QAabb::new(q(2, 0, 1), q(10, 4, 5)),
            },
            QAabb::new(q(0, 0, 0), q(64, 64, 64)),
        );
        let data = far_instances(&[prim(o, 3)]);
        assert_eq!(data.boxes.len(), 1);
        let i = data.boxes[0];
        assert_eq!(i.meta, 3 | ORIENTED_BIT);
        assert_eq!(i.origin, [20.0, 8.0, 6.0]);
        assert_eq!(i.min, [2.0, 0.0, 1.0]);
        assert_eq!(i.size, [8.0, 4.0, 4.0]);
        // A quarter turn about +Y: local x maps to world z.
        let r = Quat::from_array(i.rot);
        assert!((r * Vec3::X - Vec3::Z).length() < 1e-6);
        assert!((r * Vec3::Y - Vec3::Y).length() < 1e-6);
        // The bounds cover the rotated box, not the unrotated min and size.
        let s = data.bounding_sphere();
        let c = Vec3::new(s[0], s[1], s[2]);
        assert!((c - Vec3::new(17.0, 10.0, 12.0)).length() < s[3]);
        assert!(s[3] > 3.0);
    }
}
