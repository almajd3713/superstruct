//! Temporary primitive mesher: turns generated primitives into flat-shaded triangles.
//! Replaced by the real mesher (greedy faces, LOD bands) in Phase 2.

use glam::Vec3;
use worldgen::cell::Axis;
use worldgen::prim::{Dir4, Primitive, Q_PER_M, QAabb, QVec3, Shape};

use crate::mesh::Mesh;

const CYLINDER_SIDES: usize = 12;

fn m(q: i32) -> f32 {
    q as f32 / Q_PER_M as f32
}

fn v3(q: QVec3) -> Vec3 {
    Vec3::new(m(q.x), m(q.y), m(q.z))
}

/// Appends `prim` to `mesh`, in cell-local metres. `LiftShaft` is not solid and adds nothing.
pub fn add_primitive(mesh: &mut Mesh, prim: &Primitive) {
    let material = prim.material.0;
    match prim.shape {
        Shape::Box { aabb } => add_aabb(mesh, aabb, material),
        Shape::Ramp { aabb, rise } => add_ramp(mesh, aabb, rise, material),
        Shape::Cylinder {
            axis,
            base,
            length,
            radius,
        } => add_cylinder(mesh, axis, v3(base), m(length), m(radius), material),
        Shape::StairRun { aabb, rise, steps } => add_stairs(mesh, aabb, rise, steps, material),
        Shape::LiftShaft { .. } => {}
    }
}

fn add_aabb(mesh: &mut Mesh, aabb: QAabb, material: u16) {
    mesh.add_box(v3(aabb.min), v3(aabb.max), material);
}

/// Maps a point in the rise frame (`run` 0..1 along the ascent, `across` 0..1, `up` 0..1) into `aabb`.
fn rise_frame(aabb: QAabb, rise: Dir4) -> impl Fn(f32, f32, f32) -> Vec3 {
    let (lo, hi) = (v3(aabb.min), v3(aabb.max));
    move |run, across, up| {
        let (rx, rz) = match rise {
            Dir4::PosX => (run, across),
            Dir4::NegX => (1.0 - run, across),
            Dir4::PosZ => (across, run),
            Dir4::NegZ => (across, 1.0 - run),
        };
        lo + (hi - lo) * Vec3::new(rx, up, rz)
    }
}

fn add_ramp(mesh: &mut Mesh, aabb: QAabb, rise: Dir4, material: u16) {
    let p = rise_frame(aabb, rise);
    // Triangular cross-section (0,0), (1,0), (1,1) in (run, up): its centroid is inside the solid
    // and off the sloped plane.
    let inside = p(2.0 / 3.0, 0.5, 1.0 / 3.0);
    mesh.add_polygon_outward(
        &[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ],
        inside,
        material,
    );
    mesh.add_polygon_outward(
        &[
            p(1.0, 0.0, 0.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 1.0, 0.0),
        ],
        inside,
        material,
    );
    mesh.add_polygon_outward(
        &[
            p(0.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 0.0, 1.0),
        ],
        inside,
        material,
    );
    mesh.add_polygon_outward(
        &[p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0)],
        inside,
        material,
    );
    mesh.add_polygon_outward(
        &[p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0)],
        inside,
        material,
    );
}

fn add_stairs(mesh: &mut Mesh, aabb: QAabb, rise: Dir4, steps: u16, material: u16) {
    let p = rise_frame(aabb, rise);
    let n = f32::from(steps);
    for i in 0..steps {
        let (r0, r1) = (f32::from(i) / n, f32::from(i + 1) / n);
        let h = f32::from(i + 1) / n;
        let (a, b) = (p(r0, 0.0, 0.0), p(r1, 1.0, h));
        mesh.add_box(a.min(b), a.max(b), material);
    }
}

fn add_cylinder(mesh: &mut Mesh, axis: Axis, base: Vec3, length: f32, radius: f32, material: u16) {
    // (along, u, v) unit vectors, a right-handed frame so rings wind consistently.
    let (along, u, v) = match axis {
        Axis::X => (Vec3::X, Vec3::Y, Vec3::Z),
        Axis::Y => (Vec3::Y, Vec3::Z, Vec3::X),
        Axis::Z => (Vec3::Z, Vec3::X, Vec3::Y),
    };
    let ring = |k: usize, at: f32| {
        let a = std::f32::consts::TAU * k as f32 / CYLINDER_SIDES as f32;
        base + along * at + (u * a.cos() + v * a.sin()) * radius
    };
    let inside = base + along * (length * 0.5);
    for k in 0..CYLINDER_SIDES {
        let k1 = (k + 1) % CYLINDER_SIDES;
        mesh.add_polygon_outward(
            &[
                ring(k, 0.0),
                ring(k1, 0.0),
                ring(k1, length),
                ring(k, length),
            ],
            inside,
            material,
        );
    }
    for at in [0.0, length] {
        let cap: Vec<Vec3> = (0..CYLINDER_SIDES).map(|k| ring(k, at)).collect();
        mesh.add_polygon_outward(&cap, inside, material);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::oct_decode;
    use worldgen::prim::{FeatureKind, MaterialId, PrimFlags, Q};

    fn prim(shape: Shape) -> Primitive {
        Primitive {
            shape,
            material: MaterialId::CONCRETE,
            kind: FeatureKind::MISC,
            flags: PrimFlags::NONE,
        }
    }

    fn aabb(min: [Q; 3], max: [Q; 3]) -> QAabb {
        QAabb::new(
            QVec3::new(min[0], min[1], min[2]),
            QVec3::new(max[0], max[1], max[2]),
        )
    }

    /// Every triangle's winding agrees with its stored normal, and the normal faces away from `inside`.
    fn assert_outward(mesh: &Mesh, inside: Vec3) {
        assert!(!mesh.indices.is_empty());
        for tri in mesh.indices.chunks(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from_array(mesh.vertices[i as usize].position))
                .collect();
            let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
            if geometric.length() < 1e-6 {
                continue;
            }
            let stored = oct_decode(mesh.vertices[tri[0] as usize].normal);
            assert!(
                geometric.normalize().dot(stored) > 0.99,
                "winding disagrees with normal"
            );
            assert!(stored.dot(p[0] - inside) > -1e-3, "normal faces inward");
        }
    }

    #[test]
    fn every_shape_winds_outward() {
        for rise in Dir4::ALL {
            let mut ramp = Mesh::default();
            add_primitive(
                &mut ramp,
                &prim(Shape::Ramp {
                    aabb: aabb([0, 0, 0], [64, 32, 48]),
                    rise,
                }),
            );
            assert_outward(&ramp, Vec3::new(2.0, 0.4, 1.5));
        }
        let mut cyl = Mesh::default();
        add_primitive(
            &mut cyl,
            &prim(Shape::cylinder(Axis::Y, QVec3::new(32, 0, 32), 160, 16)),
        );
        assert_outward(&cyl, Vec3::new(2.0, 5.0, 2.0));
        let mut stairs = Mesh::default();
        let run = aabb([0, 0, 0], [64, 32, 32]);
        add_primitive(&mut stairs, &prim(Shape::stair_run(run, Dir4::PosX, 4)));
        assert_eq!(stairs.vertices.len(), 4 * 24);
    }

    #[test]
    fn lift_shaft_adds_nothing() {
        let mut mesh = Mesh::default();
        add_primitive(
            &mut mesh,
            &prim(Shape::LiftShaft {
                aabb: aabb([0, 0, 0], [16, 64, 16]),
            }),
        );
        assert!(mesh.vertices.is_empty());
    }
}
