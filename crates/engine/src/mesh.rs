//! CPU-side meshes. Greybox only: axis-aligned boxes.

use glam::Vec3;

use crate::vertex::Vertex;

#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Adds a box with flat-shaded faces, counter-clockwise when seen from outside.
    pub fn add_box(&mut self, min: Vec3, max: Vec3, material: u16) {
        // (normal, u axis, v axis) with u × v = normal, so the winding below is CCW from outside.
        const FACES: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ];
        let centre = (min + max) * 0.5;
        let half = (max - min) * 0.5;
        for (n, u, v) in FACES {
            let c = centre + n * half.dot(n.abs());
            let (hu, hv) = (half.dot(u.abs()), half.dot(v.abs()));
            let base = self.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                self.vertices
                    .push(Vertex::new(c + u * hu * su + v * hv * sv, n, material));
            }
            self.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::oct_decode;

    #[test]
    fn box_faces_wind_outward() {
        let mut m = Mesh::default();
        m.add_box(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0), 1);
        assert_eq!((m.vertices.len(), m.indices.len()), (24, 36));
        for tri in m.indices.chunks(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from_array(m.vertices[i as usize].position))
                .collect();
            let geometric = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            let stored = oct_decode(m.vertices[tri[0] as usize].normal);
            assert!(geometric.dot(stored) > 0.99, "{geometric} vs {stored}");
            // Outward: the face normal points away from the box centre.
            assert!(geometric.dot(p[0]) > 0.0);
        }
    }
}
