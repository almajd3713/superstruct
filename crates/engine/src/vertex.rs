//! Mesh vertex format.
//!
//! 20 bytes, 4-byte aligned:
//!
//! | offset | size | field | wgpu format |
//! |---|---|---|---|
//! | 0 | 12 | position, chunk-local metres | `Float32x3` |
//! | 12 | 4 | normal, octahedral | `Snorm16x2` |
//! | 16 | 2 | material id | `Uint16x2` (x) |
//! | 18 | 2 | debug tag (`debugview`), 0 unless enabled | |

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [i16; 2],
    pub material: u16,
    pub tag: u16,
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Snorm16x2,
            2 => Uint16x2,
        ],
    };

    pub fn new(position: Vec3, normal: Vec3, material: u16) -> Self {
        Self {
            position: position.to_array(),
            normal: oct_encode(normal),
            material,
            tag: 0,
        }
    }
}

/// Unit vector to two snorm16 values (octahedral mapping).
pub fn oct_encode(n: Vec3) -> [i16; 2] {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs());
    let mut p = Vec2::new(n.x, n.y);
    if n.z < 0.0 {
        p = (Vec2::ONE - Vec2::new(p.y, p.x).abs()) * Vec2::new(sign(p.x), sign(p.y));
    }
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    [q(p.x), q(p.y)]
}

pub fn oct_decode(e: [i16; 2]) -> Vec3 {
    let p = Vec2::new(f32::from(e[0]), f32::from(e[1])) / 32767.0;
    let mut n = Vec3::new(p.x, p.y, 1.0 - p.x.abs() - p.y.abs());
    if n.z < 0.0 {
        let xy = (Vec2::ONE - Vec2::new(n.y, n.x).abs()) * Vec2::new(sign(n.x), sign(n.y));
        n.x = xy.x;
        n.y = xy.y;
    }
    n.normalize()
}

fn sign(v: f32) -> f32 {
    if v >= 0.0 { 1.0 } else { -1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_alignment() {
        assert_eq!(std::mem::size_of::<Vertex>(), 20);
        assert_eq!(std::mem::align_of::<Vertex>(), 4);
        assert_eq!(std::mem::offset_of!(Vertex, normal), 12);
        assert_eq!(std::mem::offset_of!(Vertex, material), 16);
        assert_eq!(Vertex::LAYOUT.array_stride, 20);
    }

    #[test]
    fn octahedral_round_trip() {
        let mut worst = 0.0_f32;
        for i in 0..2000 {
            let t = i as f32 * 0.37;
            let n = Vec3::new(t.sin() * 1.3, (t * 1.7).cos(), (t * 0.9).sin() - 0.2).normalize();
            worst = worst.max((oct_decode(oct_encode(n)) - n).length());
        }
        for n in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            worst = worst.max((oct_decode(oct_encode(n)) - n).length());
        }
        assert!(worst < 1e-3, "{worst}");
    }
}
