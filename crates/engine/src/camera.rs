//! First-person camera, reversed-Z projection and camera-relative transforms.

use glam::{DVec3, Mat4, Quat, Vec3};
use worldgen::coord::WorldPos;

pub const NEAR: f32 = 0.05;
const PITCH_LIMIT: f32 = 89.0_f32.to_radians();

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub pos: WorldPos,
    /// Radians about +Y; 0 looks down −Z, positive turns left (right-handed, +Y up).
    pub yaw: f32,
    /// Radians, positive looks up. Clamped to ±89°.
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
}

impl Camera {
    pub fn new(pos: WorldPos, fov_y_deg: f32) -> Self {
        Self {
            pos,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: fov_y_deg.to_radians(),
        }
    }

    pub fn look(&mut self, d_yaw: f32, d_pitch: f32) {
        self.yaw = (self.yaw + d_yaw).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + d_pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    /// Forward on the horizontal plane, for walking and flying level.
    pub fn forward_flat(&self) -> Vec3 {
        Quat::from_rotation_y(self.yaw) * Vec3::NEG_Z
    }

    pub fn right_flat(&self) -> Vec3 {
        Quat::from_rotation_y(self.yaw) * Vec3::X
    }

    /// World → eye rotation. Translation is absent: everything the GPU sees is camera-relative.
    pub fn view_rotation(&self) -> Mat4 {
        Mat4::from_quat(self.rotation().inverse())
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        projection_reversed_infinite(self.fov_y, aspect, NEAR)
    }

    /// Rotation-only view times projection, for use with camera-relative positions.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view_rotation()
    }
}

/// Reversed-Z infinite perspective: the near plane maps to depth 1 and infinity to 0.
pub fn projection_reversed_infinite(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective_infinite_reverse(fov_y, aspect, near)
}

/// `point − camera` computed in f64 from the integer sectors, then narrowed to f32.
pub fn camera_relative(point: &WorldPos, camera: &WorldPos) -> Vec3 {
    let d: DVec3 = point.delta(camera);
    d.as_vec3()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{I64Vec3, Vec4};

    fn depth(m: Mat4, z: f32) -> f32 {
        let c = m * Vec4::new(0.0, 0.0, -z, 1.0);
        c.z / c.w
    }

    #[test]
    fn reversed_z_maps_near_to_one_and_far_to_zero() {
        let m = projection_reversed_infinite(1.5, 16.0 / 9.0, NEAR);
        assert!((depth(m, NEAR) - 1.0).abs() < 1e-5);
        assert!(depth(m, 1.0) < depth(m, 0.5));
        let far = depth(m, 1.0e7);
        assert!(far > 0.0 && far < 1e-6, "{far}");
    }

    #[test]
    fn camera_relative_is_precise_at_sector_2_pow_40() {
        let s = I64Vec3::new(1 << 40, -(1 << 40), (1 << 40) + 12_345);
        let cam = WorldPos::new(s, Vec3::new(100.25, 7.5, 900.0)).unwrap();
        let p = cam.offset(Vec3::new(3.001, -2.002, 1.003)).unwrap();
        let rel = camera_relative(&p, &cam);
        // Both positions are exact f32 locals in the same or adjacent sector, so the true
        // difference is the f32 sum rounded once; sub-millimetre holds with huge margin.
        assert!((rel - Vec3::new(3.001, -2.002, 1.003)).abs().max_element() < 1e-3);
        let far = WorldPos::new(s + I64Vec3::new(5, 0, -3), Vec3::new(100.25, 7.5, 900.0)).unwrap();
        let rel = camera_relative(&far, &cam);
        assert_eq!(rel, Vec3::new(5120.0, 0.0, -3072.0));
    }

    #[test]
    fn walking_across_a_sector_boundary_moves_the_view_smoothly() {
        // A fixed world point seen from a camera that steps 1 cm at a time over the +x sector face.
        let s = I64Vec3::new(1 << 40, 3, -(1 << 40));
        let point = WorldPos::new(s + I64Vec3::X, Vec3::new(40.0, 10.0, 500.0)).unwrap();
        let mut cam = WorldPos::new(s, Vec3::new(1023.9, 10.0, 500.0)).unwrap();
        let mut prev = camera_relative(&point, &cam);
        for _ in 0..40 {
            cam = cam.offset(Vec3::new(0.01, 0.0, 0.0)).unwrap();
            let rel = camera_relative(&point, &cam);
            let step = prev - rel;
            assert!(
                (step - Vec3::new(0.01, 0.0, 0.0)).abs().max_element() < 2e-4,
                "{step}"
            );
            prev = rel;
        }
        assert_ne!(cam.sector(), s);
    }

    #[test]
    fn pitch_is_clamped_and_yaw_wraps() {
        let mut c = Camera::new(WorldPos::ORIGIN, 90.0);
        c.look(0.0, 10.0);
        assert!((c.pitch - 89.0_f32.to_radians()).abs() < 1e-6);
        c.look(-0.1, 0.0);
        assert!(c.yaw > 0.0 && c.yaw < std::f32::consts::TAU);
    }
}
