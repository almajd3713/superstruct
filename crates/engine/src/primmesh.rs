//! Ramp and cylinder meshers, used by `chunkmesh` for the shapes it does not cull.

use glam::{Mat3, Vec3};
use worldgen::cell::Axis;
use worldgen::orient::{LocalShape, Oriented};
use worldgen::prim::{Dir4, FeatureKind, Primitive, Q_PER_M, QAabb, QVec3, Shape};

use crate::mesh::Mesh;
use crate::vertex::{Vertex, oct_decode};

/// Segments grow with radius so large trunks stay round and thin pipes stay cheap: 6 to 24
/// (the Web has thousands of cylinders in a cell, and every side is a quad of its own).
#[allow(clippy::cast_sign_loss)] // clamped to 6..=24 above the cast
fn cylinder_sides(radius_m: f32) -> usize {
    (6.0 + radius_m * 2.0).round().clamp(6.0, 24.0) as usize
}

fn m(q: i32) -> f32 {
    q as f32 / Q_PER_M as f32
}

fn v3(q: QVec3) -> Vec3 {
    Vec3::new(m(q.x), m(q.y), m(q.z))
}

/// Appends `prim` to `mesh`, in cell-local metres. `LiftShaft` is not solid and adds nothing.
pub fn add_primitive(mesh: &mut Mesh, prim: &Primitive) {
    let first = mesh.vertices.len();
    if prim.kind == FeatureKind::RAILING && add_rail(mesh, prim) {
    } else if let Shape::Oriented(o) = prim.shape {
        add_oriented(mesh, &o, prim.material.0, nudge(&o, prim.kind));
    } else {
        add_shape(mesh, prim.shape, prim.material.0);
    }
    let tag = crate::debugview::prim_tag(prim);
    let orient = orient_bits(&prim.shape);
    for v in &mut mesh.vertices[first..] {
        v.tag = tag;
        v.material |= orient;
    }
}

/// The heading and tilt of a piece's frame, in the bits above the material of its vertices
/// (4 to 9 the heading in 64ths of a turn, 10 to 12 the tilt in steps of 5 degrees), so the
/// shader can lay its grid along the piece's edges (PW.4.19). Zero for an axis-aligned piece.
pub fn orient_bits(shape: &Shape) -> u16 {
    let Shape::Oriented(o) = shape else {
        return 0;
    };
    let mut ex = o.frame.axes()[0];
    if ex.y < 0.0 {
        ex = -ex;
    }
    let flat = ex.x.hypot(ex.z);
    if flat < 1e-6 {
        return 0;
    }
    let turn = ex.z.atan2(ex.x) / std::f64::consts::TAU * 64.0;
    let yaw = (turn.round() as i32).rem_euclid(64) as u16;
    let tilt = ((ex.y.atan2(flat).to_degrees() / 5.0).round() as i32)
        .clamp(0, 7)
        .cast_unsigned() as u16;
    (yaw << 4) | (tilt << 10)
}

/// How far a turned box is grown on every side, in metres. Turned pieces are not culled
/// against each other, so two of them with a face in one plane would z-fight; each grows by
/// 0.15 to 2.4 mm from a hash of its own geometry, which settles who is in front the same way
/// every time and in every chunk. Skin and windows sit 4 mm further out than what they clad.
fn nudge(o: &Oriented, kind: FeatureKind) -> f32 {
    let LocalShape::Box { aabb } = o.local else {
        return 0.0;
    };
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for v in [
        o.origin.x, o.origin.y, o.origin.z, aabb.min.x, aabb.min.y, aabb.min.z, aabb.max.x,
        aabb.max.y, aabb.max.z,
    ] {
        h = (h ^ u64::from(v.cast_unsigned())).wrapping_mul(0x0100_0000_01b3);
    }
    let level = (h >> 32) % 16 + 1;
    let base = if kind == FeatureKind::SKIN || kind == FeatureKind::WINDOW {
        0.004
    } else {
        0.0
    };
    base + level as f32 * 0.000_15
}

fn add_shape(mesh: &mut Mesh, shape: Shape, material: u16) {
    match shape {
        Shape::Box { aabb } => add_aabb(mesh, aabb, material),
        Shape::Ramp { aabb, rise } => add_ramp(mesh, aabb, rise, material),
        Shape::Cylinder {
            axis,
            base,
            length,
            radius,
        } => add_cylinder(mesh, axis, v3(base), m(length), m(radius), material),
        Shape::StairRun { aabb, rise, steps } => add_stairs(mesh, aabb, rise, steps, material),
        Shape::Oriented(o) => add_oriented(mesh, &o, material, 0.0),
        Shape::LiftShaft { .. } => {}
    }
}

/// An oriented shape: its mesh in the frame, turned into place and clipped to the cell box
/// (`primitives.md` §9.4). Clipped faces get no cap: the neighbouring cell's piece of the same
/// solid meets this one on the plane, so a cap would be an interior face.
fn add_oriented(mesh: &mut Mesh, o: &Oriented, material: u16, grow: f32) {
    let mut local = Mesh::default();
    match o.local {
        LocalShape::Box { aabb } if grow > 0.0 => {
            local.add_box(
                v3(aabb.min) - Vec3::splat(grow),
                v3(aabb.max) + Vec3::splat(grow),
                material,
            );
        }
        _ => add_shape(&mut local, o.local.to_shape(), material),
    }
    turn_into(mesh, &local, o, material);
}

/// A mesh of several boxes in the frame of `o` (their own materials and tags), turned into
/// place and clipped to the cell box.
pub fn turn_group(mesh: &mut Mesh, local: &Mesh, o: &Oriented) {
    let first = mesh.vertices.len();
    turn_into(mesh, local, o, 0);
    let orient = orient_bits(&Shape::Oriented(*o));
    for v in &mut mesh.vertices[first..] {
        v.material |= orient;
    }
}

/// A mesh in the frame of `o`, turned into place and clipped to the cell box. Each vertex
/// keeps the material and tag of the local vertex it comes from (`material` is the fallback
/// for meshes that carry none).
fn turn_into(mesh: &mut Mesh, local: &Mesh, o: &Oriented, material: u16) {
    let [ex, ey, ez] = o.frame.axes();
    let rot = Mat3::from_cols(ex.as_vec3(), ey.as_vec3(), ez.as_vec3());
    let origin = v3(o.origin);
    let (lo, hi) = (v3(o.clip.min), v3(o.clip.max));
    for tri in local.indices.chunks_exact(3) {
        let v = |i: u32| local.vertices[i as usize];
        let n = rot * oct_decode(v(tri[0]).normal);
        let poly: Vec<Vec3> = tri
            .iter()
            .map(|&i| origin + rot * Vec3::from(v(i).position))
            .collect();
        let poly = clip_polygon(poly, lo, hi);
        if poly.len() < 3 {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        let (m, tag) = (v(tri[0]).material, v(tri[0]).tag);
        mesh.vertices.extend(poly.iter().map(|&p| {
            let mut vert = Vertex::new(p, n, if m == 0 { material } else { m });
            vert.tag = tag;
            vert
        }));
        for i in 1..poly.len() as u32 - 1 {
            mesh.indices.extend([base, base + i, base + i + 1]);
        }
    }
}

/// Post spacing, metres, and the sizes of a railing's parts.
/// The height of a rail above its walking surface: `edges::HEIGHT`, 18 quanta.
pub const RAIL_HEIGHT: f32 = 1.125;
const POST_SPACING: f32 = 3.0;
const POST: f32 = 0.1;
const BAR: f32 = 0.07;

/// A post centred on `at`, kept inside `[l0, l1]` (a sliver of a rail gets a thin post).
fn post_span(at: f32, l0: f32, l1: f32) -> (f32, f32) {
    let w = POST.min(l1 - l0);
    let a = (at - w * 0.5).clamp(l0, l1 - w);
    (a, a + w)
}

/// Spans of a rail of `len` metres between posts.
#[allow(clippy::cast_sign_loss)] // len is positive
fn posts_in(len: f32) -> usize {
    ((len / POST_SPACING).ceil() as usize).max(1)
}

/// A railing in the frame of its piece (`primitives.md` §9): posts stand plumb, the top and
/// middle bars follow the line of the piece. A box rail is level; the wedge a
/// flight's rail is is raked with the flight. Returns false for a shape that is neither.
fn add_rail(mesh: &mut Mesh, prim: &Primitive) -> bool {
    let material = prim.material.0;
    let (o, local) = match prim.shape {
        Shape::Oriented(o) => (o, o.local),
        Shape::Box { aabb } => (plain_frame(aabb), LocalShape::Box { aabb }),
        Shape::Ramp { aabb, rise } => (plain_frame(aabb), LocalShape::Ramp { aabb, rise }),
        _ => return false,
    };
    // A path builds its rails from posts and bars of its own: a box lower than 0.4 m (a bar, a
    // handrail slab) or shorter than 0.5 m (a post) is drawn as the box it is.
    if let LocalShape::Box { aabb } = local {
        let (lo, hi) = (v3(aabb.min), v3(aabb.max));
        if hi.y - lo.y < 0.4 || (hi.x - lo.x).max(hi.z - lo.z) < 0.5 {
            return false;
        }
    }
    let mut l = Mesh::default();
    match local {
        LocalShape::Box { aabb } => rail_level(&mut l, v3(aabb.min), v3(aabb.max), material),
        LocalShape::Ramp { aabb, rise } => {
            rail_raked(&mut l, v3(aabb.min), v3(aabb.max), rise, material);
        }
        _ => return false,
    }
    turn_into(mesh, &l, &o, material);
    true
}

/// A plain shape as an identity-framed `Oriented` that clips nothing.
fn plain_frame(aabb: QAabb) -> Oriented {
    let far = 1 << 24;
    Oriented {
        frame: worldgen::orient::Frame::Id,
        origin: QVec3::new(0, 0, 0),
        local: LocalShape::Box { aabb },
        clip: QAabb::new(QVec3::new(-far, -far, -far), QVec3::new(far, far, far)),
    }
}

/// `(along, across)` of a box: along its longer horizontal side.
fn rail_axes(lo: Vec3, hi: Vec3) -> (usize, usize) {
    if hi.x - lo.x >= hi.z - lo.z {
        (0, 2)
    } else {
        (2, 0)
    }
}

fn rail_level(m: &mut Mesh, lo: Vec3, hi: Vec3, material: u16) {
    let (al, ac) = rail_axes(lo, hi);
    let (l0, l1) = (lo[al], hi[al]);
    let c = (lo[ac] + hi[ac]) * 0.5;
    let put = |m: &mut Mesh, a0: f32, a1: f32, y0: f32, y1: f32, w: f32| {
        let mut p = Vec3::ZERO;
        let mut q = Vec3::ZERO;
        p[al] = a0;
        q[al] = a1;
        p[ac] = c - w * 0.5;
        q[ac] = c + w * 0.5;
        p.y = y0;
        q.y = y1;
        m.add_box(p, q, material);
    };
    let n = posts_in(l1 - l0);
    for i in 0..=n {
        let at = l0 + (l1 - l0) * i as f32 / n as f32;
        let (a, b) = post_span(at, l0, l1);
        put(m, a, b, lo.y, hi.y, POST);
    }
    put(m, l0, l1, hi.y - BAR, hi.y, POST);
    let mid = lo.y + (hi.y - lo.y) * 0.5;
    put(m, l0, l1, mid - BAR * 0.5, mid + BAR * 0.5, BAR);
}

/// The rail of a flight: the wedge `[lo, hi]` rising along `rise`, its sloped top the rail's top.
fn rail_raked(m: &mut Mesh, lo: Vec3, hi: Vec3, rise: Dir4, material: u16) {
    let (al, ac) = match rise {
        Dir4::PosX | Dir4::NegX => (0, 2),
        Dir4::PosZ | Dir4::NegZ => (2, 0),
    };
    let up = matches!(rise, Dir4::PosX | Dir4::PosZ);
    let (l0, l1) = (lo[al], hi[al]);
    let c = (lo[ac] + hi[ac]) * 0.5;
    // Top line height at run position `t` in 0..1 along the length.
    let top = |t: f32| {
        let k = if up { t } else { 1.0 - t };
        lo.y + (hi.y - lo.y) * k
    };
    let pt = |t: f32, y: f32, w: f32| {
        let mut p = Vec3::ZERO;
        p[al] = l0 + (l1 - l0) * t;
        p[ac] = c + w;
        p.y = y;
        p
    };
    let n = posts_in(l1 - l0);
    let rail_h = RAIL_HEIGHT;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, x1) = post_span(l0 + (l1 - l0) * t, l0, l1);
        let tt = ((x + x1) * 0.5 - l0) / (l1 - l0).max(1e-4);
        let y1 = top(tt);
        let mut a = Vec3::ZERO;
        let mut b = Vec3::ZERO;
        a[al] = x;
        b[al] = x1;
        a[ac] = c - POST * 0.5;
        b[ac] = c + POST * 0.5;
        a.y = y1 - rail_h;
        b.y = y1;
        m.add_box(a, b, material);
    }
    for (drop, th) in [(BAR, POST), (rail_h * 0.5 + BAR * 0.5, BAR)] {
        let w = th * 0.5;
        let corners = [
            pt(0.0, top(0.0) - drop, -w),
            pt(1.0, top(1.0) - drop, -w),
            pt(1.0, top(1.0) - drop + BAR, -w),
            pt(0.0, top(0.0) - drop + BAR, -w),
            pt(0.0, top(0.0) - drop, w),
            pt(1.0, top(1.0) - drop, w),
            pt(1.0, top(1.0) - drop + BAR, w),
            pt(0.0, top(0.0) - drop + BAR, w),
        ];
        let inside = corners.iter().sum::<Vec3>() / 8.0;
        for f in [
            [0, 1, 2, 3],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [3, 2, 6, 7],
            [0, 3, 7, 4],
            [1, 2, 6, 5],
        ] {
            m.add_polygon_outward(&f.map(|i| corners[i]), inside, material);
        }
    }
}

/// Sutherland-Hodgman against the six planes of the box `[lo, hi]`. Points created on a plane
/// get that plane's coordinate exactly, so pieces of one solid meet without cracks.
pub fn clip_polygon(mut poly: Vec<Vec3>, lo: Vec3, hi: Vec3) -> Vec<Vec3> {
    for axis in 0..3 {
        for (plane, keep_above) in [(lo[axis], true), (hi[axis], false)] {
            if poly.is_empty() {
                return poly;
            }
            let inside = |p: Vec3| {
                if keep_above {
                    p[axis] >= plane
                } else {
                    p[axis] <= plane
                }
            };
            let mut out = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                match (inside(a), inside(b)) {
                    (true, true) => out.push(b),
                    (true, false) | (false, true) => {
                        let t = (plane - a[axis]) / (b[axis] - a[axis]);
                        let mut x = a + (b - a) * t;
                        x[axis] = plane;
                        out.push(x);
                        if inside(b) {
                            out.push(b);
                        }
                    }
                    (false, false) => {}
                }
            }
            poly = out;
        }
    }
    poly
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
    add_cylinder_sided(
        mesh,
        axis,
        base,
        length,
        radius,
        material,
        cylinder_sides(radius),
    );
}

/// Adds a cylinder along Y inside the unit box (base cap centre `(0.5, 0, 0.5)`, radius 0.5),
/// the mesh far-band instances scale into place.
pub fn add_unit_cylinder(mesh: &mut Mesh, sides: usize) {
    add_cylinder_sided(mesh, Axis::Y, Vec3::new(0.5, 0.0, 0.5), 1.0, 0.5, 0, sides);
}

fn add_cylinder_sided(
    mesh: &mut Mesh,
    axis: Axis,
    base: Vec3,
    length: f32,
    radius: f32,
    material: u16,
    sides: usize,
) {
    // (along, u, v) unit vectors, a right-handed frame so rings wind consistently.
    let (along, u, v) = match axis {
        Axis::X => (Vec3::X, Vec3::Y, Vec3::Z),
        Axis::Y => (Vec3::Y, Vec3::Z, Vec3::X),
        Axis::Z => (Vec3::Z, Vec3::X, Vec3::Y),
    };
    let ring = |k: usize, at: f32| {
        let a = std::f32::consts::TAU * k as f32 / sides as f32;
        base + along * at + (u * a.cos() + v * a.sin()) * radius
    };
    let inside = base + along * (length * 0.5);
    for k in 0..sides {
        let k1 = (k + 1) % sides;
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
        let cap: Vec<Vec3> = (0..sides).map(|k| ring(k, at)).collect();
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
    fn cylinder_segments_scale_with_radius() {
        assert_eq!(cylinder_sides(0.25), 7);
        assert_eq!(cylinder_sides(2.0), 10);
        assert_eq!(cylinder_sides(100.0), 24);
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

    /// Two turned boxes with a face in one plane (a skin plate and the wall it clads) do not
    /// meet in that plane, whichever way they are drawn.
    #[test]
    fn coplanar_turned_boxes_are_nudged_apart() {
        use worldgen::orient::Frame;
        let clip = aabb([0; 3], [64 * 16; 3]);
        let mk = |size: i32, kind: FeatureKind| {
            let o = Oriented {
                frame: Frame::Table { yaw: 34, tilt: 3 },
                origin: QVec3::new(32 * 16, 30 * 16, 32 * 16),
                local: LocalShape::Box {
                    aabb: aabb([0, 0, 0], [size, 16, 16]),
                },
                clip,
            };
            let mut m = Mesh::default();
            let mut p = prim(Shape::Oriented(o));
            p.kind = kind;
            add_primitive(&mut m, &p);
            m
        };
        let top = |m: &Mesh| {
            m.vertices
                .iter()
                .map(|v| Vec3::from(v.position))
                .map(|p| p.y)
                .fold(f32::MIN, f32::max)
        };
        // The same plane in the frame (local y = 1 m) in both: the lengths differ so the
        // hashes do, and the skin one stands further out in any case.
        let wall = mk(160, FeatureKind::MASS);
        let skin = mk(96, FeatureKind::SKIN);
        assert!((top(&skin) - top(&wall)).abs() > 0.0002);
    }

    mod oriented {
        use super::*;
        use worldgen::orient::{Frame, LocalShape};

        fn cell() -> QAabb {
            aabb([0; 3], [64 * 16; 3])
        }

        fn beam(frame: Frame, clip: QAabb) -> Oriented {
            Oriented {
                frame,
                origin: QVec3::new(32 * 16, 30 * 16, 32 * 16),
                local: LocalShape::Box {
                    aabb: aabb([-20 * 16, -2 * 16, -3 * 16], [20 * 16, 2 * 16, 3 * 16]),
                },
                clip,
            }
        }

        fn mesh_of(o: &Oriented) -> Mesh {
            let mut mesh = Mesh::default();
            add_shape(&mut mesh, Shape::Oriented(*o), 0);
            mesh
        }

        /// Every vertex lies in the clip box, and every face points out of the solid.
        #[test]
        fn rotated_box_is_closed_and_outward() {
            for frame in [
                Frame::table(5, 0),
                Frame::table(21, 3),
                Frame::dir([2, 1, -1]),
                Frame::table(16, 0),
            ] {
                let o = beam(frame, cell());
                let mesh = mesh_of(&o);
                assert_eq!(mesh.indices.len(), 36, "{frame:?}: a box is 12 triangles");
                for tri in mesh.indices.chunks(3) {
                    let v: Vec<Vec3> = tri
                        .iter()
                        .map(|&i| Vec3::from(mesh.vertices[i as usize].position))
                        .collect();
                    let n = oct_decode(mesh.vertices[tri[0] as usize].normal);
                    let wound = (v[1] - v[0]).cross(v[2] - v[0]).normalize();
                    assert!(
                        wound.dot(n) > 0.999,
                        "{frame:?}: winding disagrees with the normal"
                    );
                    let c = (v[0] + v[1] + v[2]) / 3.0;
                    assert!(o.contains(c - n * 0.02), "{frame:?}: inside is not solid");
                    assert!(!o.contains(c + n * 0.02), "{frame:?}: outside is solid");
                }
            }
        }

        /// Clipped meshes stay in the box, and the two halves of one solid cover its surface
        /// outside the cut with no overlap: the area on the cut plane is the only difference.
        #[test]
        fn clipping_keeps_vertices_in_the_cell() {
            let left = aabb([0; 3], [32 * 16, 64 * 16, 64 * 16]);
            let right = aabb([32 * 16, 0, 0], [64 * 16, 64 * 16, 64 * 16]);
            let f = Frame::dir([2, 1, 0]);
            let whole = mesh_of(&beam(f, cell()));
            let (l, r) = (mesh_of(&beam(f, left)), mesh_of(&beam(f, right)));
            for (mesh, clip) in [(&l, left), (&r, right)] {
                for v in &mesh.vertices {
                    let p = Vec3::from(v.position);
                    assert!(
                        p.cmpge(clip.min_m() - Vec3::splat(1e-4)).all()
                            && p.cmple(clip.max_m() + Vec3::splat(1e-4)).all(),
                        "{p} outside {clip:?}"
                    );
                }
            }
            let area = |m: &Mesh| {
                m.indices
                    .chunks(3)
                    .map(|t| {
                        let p = |i: u32| Vec3::from(m.vertices[i as usize].position);
                        (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length() * 0.5
                    })
                    .sum::<f32>()
            };
            // The open cut faces are missing from both halves, so the two surfaces add up to the
            // whole surface minus nothing: the whole box does not cross the plane x = 32 in a cap.
            let (aw, al, ar) = (area(&whole), area(&l), area(&r));
            assert!((al + ar - aw).abs() < 1e-2 * aw, "{al} + {ar} != {aw}");
        }

        #[test]
        fn a_clip_that_misses_draws_nothing() {
            let far = aabb([60 * 16, 0, 0], [64 * 16, 4 * 16, 4 * 16]);
            assert!(mesh_of(&beam(Frame::table(3, 2), far)).indices.is_empty());
        }

        #[test]
        fn cylinders_and_ramps_turn_with_the_frame() {
            let o = Oriented {
                frame: Frame::dir([1, 1, 0]),
                origin: QVec3::new(10 * 16, 10 * 16, 10 * 16),
                local: LocalShape::Cylinder {
                    axis: worldgen::cell::Axis::X,
                    base: QVec3::new(0, 0, 0),
                    length: 20 * 16,
                    radius: 2 * 16,
                },
                clip: cell(),
            };
            let mesh = mesh_of(&o);
            assert!(!mesh.indices.is_empty());
            // The far cap centre lies 20 m along (1,1,0)/sqrt2 from the origin.
            let far = Vec3::splat(10.0) + Vec3::new(1.0, 1.0, 0.0).normalize() * 20.0;
            let near = mesh
                .vertices
                .iter()
                .map(|v| (Vec3::from(v.position) - far).length())
                .fold(f32::MAX, f32::min);
            assert!(
                near < 2.1,
                "no vertex within a radius of the far cap centre: {near}"
            );
        }
    }

    fn rail(shape: Shape) -> Primitive {
        Primitive {
            kind: FeatureKind::RAILING,
            ..prim(shape)
        }
    }

    /// PW.4.9: a rail is drawn inside the shape it collides as, in its own frame: posts plumb,
    /// bars along it, and a flight's rail rakes with the flight.
    #[test]
    fn rails_are_drawn_where_they_collide() {
        let level = aabb([0, 0, 0], [16 * 10, 18, 2]);
        let mut m = Mesh::default();
        add_primitive(&mut m, &rail(Shape::Box { aabb: level }));
        assert!(m.indices.len() / 3 > 12 * 4, "posts and bars, not one box");
        for v in &m.vertices {
            let p = Vec3::from(v.position);
            assert!(
                p.cmpge(Vec3::splat(-1e-4)).all()
                    && p.cmple(Vec3::new(10.0, 1.125, 0.125) + Vec3::splat(1e-4))
                        .all(),
                "{p}"
            );
        }
        // The wedge of a flight rising 2 m over 10 m: every vertex under the top line, and the
        // top of the last post as high as the wedge's top.
        let wedge = aabb([0, 18, 0], [16 * 10, 18 + 32, 2]);
        let mut m = Mesh::default();
        add_primitive(
            &mut m,
            &rail(Shape::Ramp {
                aabb: wedge,
                rise: Dir4::PosX,
            }),
        );
        let top = m.vertices.iter().map(|v| v.position[1]).fold(0.0, f32::max);
        assert!((top - (1.125 + 2.0)).abs() < 0.2, "{top}");
        for v in &m.vertices {
            let (x, y) = (v.position[0], v.position[1]);
            assert!(
                y <= 1.125 + 2.0 * x / 10.0 + 0.02,
                "above the top line at {x}: {y}"
            );
        }
        // A turned rail stays inside its piece's turned bounds: a 45-degree turn about +Y.
        let clip = aabb([-4000, -4000, -4000], [4000, 4000, 4000]);
        let turned = Shape::oriented(
            worldgen::orient::Frame::table(16, 0),
            QVec3::new(0, 0, 0),
            LocalShape::Box { aabb: level },
            clip,
        );
        let mut m = Mesh::default();
        add_primitive(&mut m, &rail(turned));
        let heights = m.vertices.iter().map(|v| v.position[1]);
        assert!(heights.clone().fold(f32::MAX, f32::min) >= -1e-4);
        assert!(heights.fold(0.0, f32::max) <= 1.125 + 1e-4);
        let reach = |v: &Vertex| v.position[0].hypot(v.position[2]);
        assert!(
            m.vertices.iter().map(reach).fold(0.0, f32::max) > 9.9,
            "still ten metres long"
        );
    }
}
