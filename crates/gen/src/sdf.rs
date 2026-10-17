//! Point-in-solid and signed distance for each primitive shape (`primitives.md` §3).
//!
//! Points are `f32` metres in the owning cell's local space. Distances are negative inside.

use glam::{Vec2, Vec3};

use crate::cell::Axis;
use crate::prim::{Dir4, Q_PER_M, QAabb, QVec3, Shape};

impl QVec3 {
    /// Exact for any coordinate below 2²⁰ m.
    pub fn to_metres(self) -> Vec3 {
        Vec3::new(self.x as f32, self.y as f32, self.z as f32) / Q_PER_M as f32
    }
}

impl QAabb {
    pub fn min_m(&self) -> Vec3 {
        self.min.to_metres()
    }

    pub fn max_m(&self) -> Vec3 {
        self.max.to_metres()
    }
}

impl Shape {
    /// Point-in-solid. Boxes are half-open, so touching shapes never both contain a point.
    pub fn contains(&self, p: Vec3) -> bool {
        match *self {
            Self::Box { aabb } => in_box(p, aabb.min_m(), aabb.max_m()),
            Self::Ramp { aabb, rise } => {
                let (q, size) = rise_frame(p, aabb, rise);
                in_box(q, Vec3::ZERO, size) && q.y * size.x <= size.y * q.x
            }
            Self::Cylinder {
                axis,
                base,
                length,
                radius,
            } => {
                let (a, r) = cylinder_coords(p, axis, base);
                (0.0..q_to_m(length)).contains(&a) && r.length_squared() < q_to_m(radius).powi(2)
            }
            Self::StairRun { aabb, rise, steps } => {
                let (q, size) = rise_frame(p, aabb, rise);
                if !in_box(q, Vec3::ZERO, size) {
                    return false;
                }
                let n = f32::from(steps);
                let step = (q.x * n / size.x).floor().min(n - 1.0);
                q.y < (step + 1.0) * size.y / n
            }
            Self::LiftShaft { .. } => false,
        }
    }

    /// Signed distance. Exact for `Box` and `Cylinder`. `Ramp` is exact inside and a lower
    /// bound outside; `StairRun` is exact outside and underestimates depth inside. A bound never
    /// overestimates distance, so sphere tracing stays safe. `LiftShaft` has no surface.
    pub fn sdf(&self, p: Vec3) -> f32 {
        match *self {
            Self::Box { aabb } => box_sdf(p, aabb.min_m(), aabb.max_m()),
            Self::Ramp { aabb, rise } => {
                let (q, size) = rise_frame(p, aabb, rise);
                // Distance above the incline through the low edge and the high top edge.
                let plane = (q.y * size.x - size.y * q.x) / size.x.hypot(size.y);
                box_sdf(q, Vec3::ZERO, size).max(plane)
            }
            Self::Cylinder {
                axis,
                base,
                length,
                radius,
            } => {
                let (a, r) = cylinder_coords(p, axis, base);
                let half = q_to_m(length) / 2.0;
                let d = Vec2::new(r.length() - q_to_m(radius), (a - half).abs() - half);
                d.max(Vec2::ZERO).length() + d.max_element().min(0.0)
            }
            Self::StairRun { aabb, rise, steps } => {
                let (q, size) = rise_frame(p, aabb, rise);
                let n = f32::from(steps);
                let (tread, step_h) = (size.x / n, size.y / n);
                (0..steps)
                    .map(|i| {
                        let i = f32::from(i);
                        let min = Vec3::new(i * tread, 0.0, 0.0);
                        let max = Vec3::new((i + 1.0) * tread, (i + 1.0) * step_h, size.z);
                        box_sdf(q, min, max)
                    })
                    .fold(f32::INFINITY, f32::min)
            }
            Self::LiftShaft { .. } => f32::INFINITY,
        }
    }
}

fn q_to_m(q: i32) -> f32 {
    q as f32 / Q_PER_M as f32
}

fn in_box(p: Vec3, min: Vec3, max: Vec3) -> bool {
    p.cmpge(min).all() && p.cmplt(max).all()
}

fn box_sdf(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    let half = (max - min) / 2.0;
    let q = (p - (min + max) / 2.0).abs() - half;
    q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
}

/// Maps `p` into a frame where the box is `[0, size)`, `x` runs along `rise` from the low edge,
/// `y` is height above the box floor and `z` is across the run. The map is an isometry, so
/// distances carry over unchanged.
fn rise_frame(p: Vec3, aabb: QAabb, rise: Dir4) -> (Vec3, Vec3) {
    let (min, max) = (aabb.min_m(), aabb.max_m());
    let d = p - min;
    let size = max - min;
    match rise {
        Dir4::PosX => (Vec3::new(d.x, d.y, d.z), size),
        Dir4::NegX => (Vec3::new(max.x - p.x, d.y, d.z), size),
        Dir4::PosZ => (Vec3::new(d.z, d.y, d.x), Vec3::new(size.z, size.y, size.x)),
        Dir4::NegZ => (
            Vec3::new(max.z - p.z, d.y, d.x),
            Vec3::new(size.z, size.y, size.x),
        ),
    }
}

/// Position along the axis from `base`, and the radial offset from the axis.
fn cylinder_coords(p: Vec3, axis: Axis, base: QVec3) -> (f32, Vec2) {
    let d = p - base.to_metres();
    match axis {
        Axis::X => (d.x, Vec2::new(d.y, d.z)),
        Axis::Y => (d.y, Vec2::new(d.x, d.z)),
        Axis::Z => (d.z, Vec2::new(d.x, d.y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    const EPS: f32 = 1e-5;

    /// A box in whole metres.
    fn bxm(min: [i32; 3], max: [i32; 3]) -> QAabb {
        let q = |v: [i32; 3]| QVec3::new(v[0] * Q_PER_M, v[1] * Q_PER_M, v[2] * Q_PER_M);
        QAabb::new(q(min), q(max))
    }

    fn assert_sdf(shape: &Shape, p: Vec3, expected: f32) {
        let d = shape.sdf(p);
        assert!(
            (d - expected).abs() < EPS,
            "{shape:?} at {p}: {d} != {expected}"
        );
    }

    #[test]
    fn box_known_points() {
        let s = Shape::Box {
            aabb: bxm([0, 0, 0], [4, 2, 6]),
        };
        assert!(s.contains(Vec3::new(2.0, 1.0, 3.0)));
        assert!(s.contains(Vec3::ZERO));
        assert!(!s.contains(Vec3::new(4.0, 1.0, 3.0)), "half-open max face");
        assert_sdf(&s, Vec3::new(2.0, 1.0, 3.0), -1.0);
        assert_sdf(&s, Vec3::new(2.0, 5.0, 3.0), 3.0);
        assert_sdf(&s, Vec3::new(7.0, 6.0, 3.0), 5.0);
        assert_sdf(&s, Vec3::new(-1.0, -2.0, -2.0), 3.0);
    }

    #[test]
    fn cylinder_known_points() {
        // Vertical trunk, radius 2 m, from y = 1 to y = 11, centred on (5, _, 5).
        let s = Shape::cylinder(Axis::Y, QVec3::new(80, 16, 80), 160, 32);
        assert!(s.contains(Vec3::new(5.0, 6.0, 5.0)));
        assert!(s.contains(Vec3::new(6.9, 1.0, 5.0)));
        assert!(!s.contains(Vec3::new(5.0, 11.0, 5.0)), "half-open far cap");
        assert!(!s.contains(Vec3::new(6.5, 6.0, 6.5)), "outside radius");
        assert_sdf(&s, Vec3::new(5.0, 6.0, 5.0), -2.0);
        assert_sdf(&s, Vec3::new(5.0, 3.0, 5.0), -2.0);
        assert_sdf(&s, Vec3::new(10.0, 6.0, 5.0), 3.0);
        assert_sdf(&s, Vec3::new(5.0, 14.0, 5.0), 3.0);
        assert_sdf(&s, Vec3::new(10.0, 15.0, 5.0), 5.0);

        // Horizontal pipe along Z, radius 1 m, axis at (x, y) = (0, 0), z from 0 to 4.
        let pipe = Shape::cylinder(Axis::Z, QVec3::new(0, 0, 0), 64, 16);
        assert_sdf(&pipe, Vec3::new(0.0, 3.0, 2.0), 2.0);
        assert_sdf(&pipe, Vec3::new(0.0, 0.0, -1.5), 1.5);
        assert!(pipe.contains(Vec3::new(0.0, -0.5, 3.9)));
    }

    #[test]
    fn ramp_known_points() {
        // 8 m run along +X rising 4 m, 2 m wide.
        let aabb = bxm([0, 0, 0], [8, 4, 2]);
        let s = Shape::Ramp {
            aabb,
            rise: Dir4::PosX,
        };
        assert!(s.contains(Vec3::new(6.0, 2.9, 1.0)));
        assert!(!s.contains(Vec3::new(2.0, 1.1, 1.0)), "above incline");
        assert!(s.contains(Vec3::new(2.0, 0.9, 1.0)));
        // Straight above the incline, the distance is the perpendicular to the plane.
        let slope = 0.5f32;
        let above = Vec3::new(4.0, 2.0 + 1.0, 1.0);
        assert_sdf(&s, above, 1.0 / (1.0 + slope * slope).sqrt());
        // Beyond the high end, the nearest point is the top edge.
        assert_sdf(&s, Vec3::new(10.0, 4.0, 1.0), 2.0);

        // Mirroring the rise mirrors the shape.
        let neg = Shape::Ramp {
            aabb,
            rise: Dir4::NegX,
        };
        let pz = Shape::Ramp {
            aabb: bxm([0, 0, 0], [2, 4, 8]),
            rise: Dir4::PosZ,
        };
        for p in [above, Vec3::new(6.0, 2.9, 1.0), Vec3::new(1.0, 0.3, 0.5)] {
            let mirrored = Vec3::new(8.0 - p.x, p.y, p.z);
            let swapped = Vec3::new(p.z, p.y, p.x);
            assert_eq!(s.contains(p), neg.contains(mirrored));
            assert_eq!(s.contains(p), pz.contains(swapped));
            assert!((s.sdf(p) - neg.sdf(mirrored)).abs() < EPS);
            assert!((s.sdf(p) - pz.sdf(swapped)).abs() < EPS);
        }
    }

    #[test]
    fn stair_run_known_points() {
        // 4 steps of 0.25 m rise and 0.5 m tread along −Z, 1 m wide.
        let s = Shape::stair_run(bxm([0, 0, 0], [1, 1, 2]), Dir4::NegZ, 4);
        // The low edge is at z = 2; step 0 spans z in (1.5, 2].
        assert!(s.contains(Vec3::new(0.5, 0.2, 1.75)));
        assert!(!s.contains(Vec3::new(0.5, 0.3, 1.75)));
        assert!(s.contains(Vec3::new(0.5, 0.95, 0.25)), "top step");
        assert_sdf(&s, Vec3::new(0.5, 0.5, 1.9), 0.25);
        // Nearer to the next step's corner than to the tread below.
        assert_sdf(&s, Vec3::new(0.5, 0.5, 1.75), 0.25);
        assert_sdf(&s, Vec3::new(0.5, 0.75, 1.75), 0.125f32.sqrt());
        assert_sdf(&s, Vec3::new(0.5, 1.5, 0.25), 0.5);
        // Above the riser corner between steps 1 and 2 (the tread edge at z = 1).
        assert_sdf(&s, Vec3::new(0.5, 0.5, 1.0 + 0.1), 0.0);
        assert_sdf(&s, Vec3::new(0.5, 0.2, 1.75), -0.05);
    }

    #[test]
    fn lift_shaft_is_never_solid() {
        let s = Shape::LiftShaft {
            aabb: bxm([0, 0, 0], [8, 64, 8]),
        };
        assert!(!s.contains(Vec3::new(4.0, 32.0, 4.0)));
        assert!(s.sdf(Vec3::new(4.0, 32.0, 4.0)).is_infinite());
    }

    fn sample_shapes() -> Vec<Shape> {
        let aabb = bxm([1, 0, 2], [9, 4, 5]);
        let mut shapes = vec![
            Shape::Box { aabb },
            Shape::cylinder(Axis::X, QVec3::new(16, 40, 48), 120, 24),
            Shape::cylinder(Axis::Y, QVec3::new(80, 0, 80), 64, 40),
            Shape::cylinder(Axis::Z, QVec3::new(40, 40, 8), 100, 8),
        ];
        for rise in Dir4::ALL {
            shapes.push(Shape::Ramp { aabb, rise });
            shapes.push(Shape::stair_run(bxm([1, 0, 2], [9, 4, 10]), rise, 8));
        }
        shapes
    }

    fn random_point(rng: &mut Rng) -> Vec3 {
        Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 14.0 - 2.0
    }

    /// Away from the surface, `contains` and the sign of `sdf` agree.
    #[test]
    fn contains_matches_sdf_sign() {
        let mut rng = Rng::new(0x5DF);
        for shape in sample_shapes() {
            for _ in 0..20_000 {
                let p = random_point(&mut rng);
                let d = shape.sdf(p);
                if d.abs() > 1e-3 {
                    assert_eq!(shape.contains(p), d < 0.0, "{shape:?} at {p}: sdf {d}");
                }
            }
        }
    }

    /// Outside, the ball of radius `sdf(p)` holds no solid: the bound never overestimates.
    #[test]
    fn sdf_never_overestimates() {
        let mut rng = Rng::new(0xBA11);
        for shape in sample_shapes() {
            for _ in 0..2_000 {
                let p = random_point(&mut rng);
                let d = shape.sdf(p);
                if d <= 1e-3 {
                    continue;
                }
                for _ in 0..64 {
                    let dir = (random_point(&mut rng) - Vec3::splat(5.0)).normalize_or_zero();
                    let q = p + dir * (d - 1e-3) * rng.next_f32().cbrt();
                    assert!(
                        !shape.contains(q),
                        "{shape:?}: {q} inside ball at {p} r {d}"
                    );
                }
            }
        }
    }

    /// Signed distance is 1-Lipschitz.
    #[test]
    fn sdf_is_lipschitz() {
        let mut rng = Rng::new(0x11F);
        for shape in sample_shapes() {
            for _ in 0..20_000 {
                let (p, q) = (random_point(&mut rng), random_point(&mut rng));
                let q = p + (q - p) * 0.1;
                assert!((shape.sdf(p) - shape.sdf(q)).abs() <= p.distance(q) + 1e-4);
            }
        }
    }
}
