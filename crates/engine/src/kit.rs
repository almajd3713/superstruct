//! The L4 module kit (`docs/spec/levels.md` §7): wall panels, railings, grating, pipes, cables,
//! door frames, lamps and stairs, all generated in code. Each piece is built in its own frame,
//! in metres: +Y up, the visible face towards +Z, the origin on the floor at the middle of the
//! piece's base. Placement rules move pieces with [`Mesh::add_transformed`].

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)] // small counts from metre lengths

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::{Mat3, Mat4, Vec3};
use worldgen::prim::MaterialId;

use crate::mesh::Mesh;
use crate::vertex::{Vertex, oct_decode};

const CONCRETE: u16 = MaterialId::CONCRETE.0;
const METAL: u16 = MaterialId::METAL.0;
const GRATING: u16 = MaterialId::GRATING.0;
const EMISSIVE: u16 = MaterialId::EMISSIVE.0;
const PIPE: u16 = MaterialId::PIPE.0;

/// Module slot: 8 m wide, 4 m storey.
pub const SLOT_W: f32 = 8.0;
pub const SLOT_H: f32 = 4.0;
/// Wall panels are this deep behind their face.
pub const PANEL_DEPTH: f32 = 0.25;
/// Stair geometry from `traversal.md`.
pub const RISE: f32 = 0.1875;
pub const TREAD: f32 = 0.3125;

impl Mesh {
    /// Appends `other` moved by `m`, which must be a rotation, a translation and a uniform scale.
    pub fn add_transformed(&mut self, other: &Mesh, m: Mat4) {
        let base = self.vertices.len() as u32;
        let rot = Mat3::from_mat4(m).inverse().transpose();
        self.vertices.extend(other.vertices.iter().map(|v| {
            let n = (rot * oct_decode(v.normal)).normalize();
            Vertex::new(m.transform_point3(Vec3::from(v.position)), n, v.material)
        }));
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }
}

fn bx(m: &mut Mesh, min: [f32; 3], max: [f32; 3], material: u16) {
    m.add_box(Vec3::from(min), Vec3::from(max), material);
}

/// A tube of `radius` along `path`, smooth shaded, with flat caps.
fn add_tube(m: &mut Mesh, path: &[Vec3], radius: f32, sides: usize, material: u16) {
    debug_assert!(path.len() >= 2);
    let tangent = |i: usize| {
        let a = path[i.saturating_sub(1)];
        let b = path[(i + 1).min(path.len() - 1)];
        (b - a).normalize()
    };
    // A frame carried along the path without twisting.
    let t0 = tangent(0);
    let mut u = if t0.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    u = (u - t0 * u.dot(t0)).normalize();
    let first = m.vertices.len() as u32;
    for (i, p) in path.iter().enumerate() {
        let t = tangent(i);
        u = (u - t * u.dot(t)).normalize();
        let v = t.cross(u);
        for k in 0..sides {
            let a = TAU * k as f32 / sides as f32;
            let n = u * a.cos() + v * a.sin();
            m.vertices.push(Vertex::new(*p + n * radius, n, material));
        }
    }
    let s = sides as u32;
    for i in 0..path.len() as u32 - 1 {
        for k in 0..s {
            let (a, b) = (first + i * s + k, first + i * s + (k + 1) % s);
            let (c, d) = (a + s, b + s);
            m.indices.extend([a, b, c, b, d, c]);
        }
    }
    // Caps share no vertices with the wall, so they stay flat.
    for (end, sign) in [(0, -1.0), (path.len() - 1, 1.0)] {
        let t = tangent(end) * sign;
        let c = m.vertices.len() as u32;
        m.vertices.push(Vertex::new(path[end], t, material));
        let ring = first + end as u32 * s;
        for k in 0..sides {
            m.vertices.push(Vertex::new(
                Vec3::from(m.vertices[(ring + k as u32) as usize].position),
                t,
                material,
            ));
        }
        for k in 0..s {
            let (a, b) = (c + 1 + k, c + 1 + (k + 1) % s);
            if sign > 0.0 {
                m.indices.extend([c, a, b]);
            } else {
                m.indices.extend([c, b, a]);
            }
        }
    }
}

fn arc(centre: Vec3, from: Vec3, to: Vec3, radius: f32, steps: usize) -> Vec<Vec3> {
    (0..=steps)
        .map(|i| {
            let t = FRAC_PI_2 * i as f32 / steps as f32;
            centre + from * (radius * t.cos()) + to * (radius * t.sin())
        })
        .collect()
}

/// An 8 m × 4 m wall panel, 0.25 m deep. Variants: 0 plain with seams, 1 ribbed, 2 louvred vent,
/// 3 with an emissive window strip.
pub fn wall_panel(variant: u8) -> Mesh {
    let mut m = Mesh::default();
    let (hw, d) = (SLOT_W / 2.0, PANEL_DEPTH);
    bx(&mut m, [-hw, 0.0, -d], [hw, SLOT_H, 0.0], CONCRETE);
    match variant % 4 {
        0 => {
            // Two shallow horizontal seams and a plinth.
            for y in [1.3, 2.7] {
                bx(&mut m, [-hw, y, 0.0], [hw, y + 0.06, 0.04], METAL);
            }
            bx(&mut m, [-hw, 0.0, 0.0], [hw, 0.3, 0.08], METAL);
        }
        1 => {
            for i in 0..8 {
                let x = -hw + 0.5 + i as f32;
                bx(
                    &mut m,
                    [x - 0.08, 0.3, 0.0],
                    [x + 0.08, SLOT_H - 0.2, 0.12],
                    METAL,
                );
            }
            bx(&mut m, [-hw, SLOT_H - 0.2, 0.0], [hw, SLOT_H, 0.1], METAL);
        }
        2 => {
            // A recessed vent with louvres.
            bx(&mut m, [-1.5, 1.0, 0.0], [1.5, 3.0, 0.06], METAL);
            for i in 0..9 {
                let y = 1.15 + 0.2 * i as f32;
                bx(&mut m, [-1.35, y, 0.06], [1.35, y + 0.05, 0.16], PIPE);
            }
        }
        _ => {
            bx(&mut m, [-hw + 0.5, 1.4, 0.0], [hw - 0.5, 2.6, 0.05], METAL);
            bx(
                &mut m,
                [-hw + 0.6, 1.5, 0.05],
                [hw - 0.6, 2.5, 0.08],
                EMISSIVE,
            );
            for i in 1..4 {
                let x = -hw + 0.5 + (SLOT_W - 1.0) * i as f32 / 4.0;
                bx(&mut m, [x - 0.04, 1.5, 0.08], [x + 0.04, 2.5, 0.12], METAL);
            }
        }
    }
    m
}

/// A railing `length` metres along X: posts every 1.5 m or less, a top rail and a mid rail.
/// 1.1 m high, with a toe plate.
pub fn railing(length: f32) -> Mesh {
    let mut m = Mesh::default();
    let hl = length / 2.0;
    let n = (length / 1.5).ceil().max(1.0) as usize;
    for i in 0..=n {
        let x = -hl + length * i as f32 / n as f32;
        bx(
            &mut m,
            [x - 0.03, 0.0, -0.03],
            [x + 0.03, 1.05, 0.03],
            METAL,
        );
    }
    bx(&mut m, [-hl, 1.05, -0.04], [hl, 1.12, 0.04], METAL);
    bx(&mut m, [-hl, 0.55, -0.015], [hl, 0.6, 0.015], METAL);
    bx(&mut m, [-hl, 0.0, -0.02], [hl, 0.12, 0.02], METAL);
    m
}

/// A grating floor `w` × `d` metres (x by z), 5 cm thick, centred on the origin: a frame and
/// slats, so it reads as open metal from above.
pub fn grating_floor(w: f32, d: f32) -> Mesh {
    let mut m = Mesh::default();
    let (hw, hd) = (w / 2.0, d / 2.0);
    let frame = 0.06;
    bx(&mut m, [-hw, -0.05, -hd], [hw, 0.0, -hd + frame], GRATING);
    bx(&mut m, [-hw, -0.05, hd - frame], [hw, 0.0, hd], GRATING);
    bx(&mut m, [-hw, -0.05, -hd], [-hw + frame, 0.0, hd], GRATING);
    bx(&mut m, [hw - frame, -0.05, -hd], [hw, 0.0, hd], GRATING);
    let slats = (w / 0.1).floor() as usize;
    for i in 0..slats {
        let x = -hw + frame + (w - 2.0 * frame) * (i as f32 + 0.5) / slats as f32;
        bx(
            &mut m,
            [x - 0.012, -0.05, -hd],
            [x + 0.012, 0.0, hd],
            GRATING,
        );
    }
    let bars = (d / 0.4).floor() as usize;
    for i in 0..bars {
        let z = -hd + frame + (d - 2.0 * frame) * (i as f32 + 0.5) / bars as f32;
        bx(
            &mut m,
            [-hw, -0.035, z - 0.01],
            [hw, -0.015, z + 0.01],
            GRATING,
        );
    }
    m
}

/// A straight pipe along X, `length` metres, with a flange every 2 m.
pub fn pipe_straight(length: f32, radius: f32) -> Mesh {
    let mut m = Mesh::default();
    let hl = length / 2.0;
    add_tube(
        &mut m,
        &[Vec3::new(-hl, 0.0, 0.0), Vec3::new(hl, 0.0, 0.0)],
        radius,
        16,
        PIPE,
    );
    let flanges = (length / 2.0).floor() as usize + 1;
    for i in 0..flanges {
        let x = if flanges == 1 {
            0.0
        } else {
            -hl + length * i as f32 / (flanges - 1) as f32
        };
        let ends = [Vec3::new(x - 0.04, 0.0, 0.0), Vec3::new(x + 0.04, 0.0, 0.0)];
        add_tube(&mut m, &ends, radius * 1.3, 16, METAL);
    }
    m
}

/// A 90° pipe bend from running along +X (entering at the origin side, x < 0) to running up +Y,
/// with centre-line bend radius `bend`.
pub fn pipe_bend(radius: f32, bend: f32, straight: f32) -> Mesh {
    let mut m = Mesh::default();
    let centre = Vec3::new(0.0, bend, 0.0);
    let mut path = vec![Vec3::new(-straight, 0.0, 0.0)];
    path.extend(arc(centre, Vec3::NEG_Y, Vec3::X, bend, 12));
    path.push(path[path.len() - 1] + Vec3::Y * straight);
    add_tube(&mut m, &path, radius, 16, PIPE);
    m
}

/// A bundle of `n` cables along X with clips every metre.
pub fn cable_bundle(length: f32, n: usize) -> Mesh {
    let mut m = Mesh::default();
    let hl = length / 2.0;
    let r = 0.018;
    for i in 0..n {
        let a = TAU * i as f32 / n as f32;
        let off = Vec3::new(0.0, a.cos(), a.sin()) * r * 1.7;
        let segs = 8;
        let path: Vec<Vec3> = (0..=segs)
            .map(|s| {
                let t = s as f32 / segs as f32;
                // A slight sag between the clips.
                let sag = -0.03 * (1.0 - (2.0 * t - 1.0).powi(2));
                Vec3::new(-hl + length * t, sag, 0.0) + off
            })
            .collect();
        add_tube(&mut m, &path, r, 6, PIPE);
    }
    let clips = length.floor() as usize + 1;
    for i in 0..clips {
        let x = -hl + length * i as f32 / (clips - 1).max(1) as f32;
        bx(
            &mut m,
            [x - 0.02, -0.07, -0.07],
            [x + 0.02, 0.07, 0.07],
            METAL,
        );
    }
    m
}

/// A door frame for the 2 m × 2.75 m opening of an L3 door side (3 m above the storey line,
/// less the 0.25 m floor), in a wall of `PANEL_DEPTH`. The origin is on the floor.
pub fn door_frame() -> Mesh {
    let mut m = Mesh::default();
    let (hw, h, t, d) = (1.0, 2.75, 0.12, PANEL_DEPTH);
    bx(&mut m, [-hw - t, 0.0, -d], [-hw, h + t, 0.06], METAL);
    bx(&mut m, [hw, 0.0, -d], [hw + t, h + t, 0.06], METAL);
    bx(&mut m, [-hw, h, -d], [hw, h + t, 0.06], METAL);
    // A threshold plate and a small status light over the door.
    bx(&mut m, [-hw, 0.0, -d], [hw, 0.03, 0.04], METAL);
    bx(
        &mut m,
        [-0.15, h + t, 0.0],
        [0.15, h + t + 0.06, 0.04],
        EMISSIVE,
    );
    m
}

/// A ceiling lamp hanging from y = 0 downwards (the origin is on the ceiling): a housing and an
/// emissive panel.
pub fn lamp() -> Mesh {
    let mut m = Mesh::default();
    bx(&mut m, [-0.45, -0.1, -0.2], [0.45, 0.0, 0.2], METAL);
    bx(&mut m, [-0.4, -0.13, -0.15], [0.4, -0.1, 0.15], EMISSIVE);
    bx(&mut m, [-0.47, -0.14, -0.22], [-0.4, -0.04, 0.22], METAL);
    bx(&mut m, [0.4, -0.14, -0.22], [0.47, -0.04, 0.22], METAL);
    m
}

/// A straight stair run 1.2 m wide that climbs `rise_total` metres along +Z from the origin, steps
/// as boxes between two stringers.
pub fn stair(rise_total: f32) -> Mesh {
    let mut m = Mesh::default();
    let steps = (rise_total / RISE).round() as usize;
    for i in 0..steps {
        let (z0, y) = (TREAD * i as f32, RISE * (i + 1) as f32);
        bx(
            &mut m,
            [-0.55, y - 0.04, z0],
            [0.55, y, z0 + TREAD],
            GRATING,
        );
        // The riser underneath keeps the run from being see-through from the side.
        bx(
            &mut m,
            [-0.55, y - RISE, z0],
            [0.55, y - 0.04, z0 + 0.02],
            METAL,
        );
    }
    for sx in [-1.0f32, 1.0] {
        let x = sx * 0.6;
        // Stringer: a stepped slab following the stairs.
        for i in 0..steps {
            let (z0, y) = (TREAD * i as f32, RISE * (i + 1) as f32);
            bx(
                &mut m,
                [x - 0.03, y - 0.25, z0],
                [x + 0.03, y, z0 + TREAD],
                METAL,
            );
        }
    }
    m
}

/// The pieces that get instanced, each with fixed dimensions. Variable lengths are tiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Piece {
    WallPlain,
    WallRibbed,
    WallVent,
    WallWindow,
    /// 2 m of railing.
    Railing,
    /// A slot's grating overlay: 118 quanta square.
    Grating,
    DoorFrame,
    Lamp,
    /// 8 m of pipe, radius 0.25 m.
    Pipe,
    /// 8 m of cable bundle.
    Cable,
}

impl Piece {
    pub const ALL: [Piece; 10] = [
        Piece::WallPlain,
        Piece::WallRibbed,
        Piece::WallVent,
        Piece::WallWindow,
        Piece::Railing,
        Piece::Grating,
        Piece::DoorFrame,
        Piece::Lamp,
        Piece::Pipe,
        Piece::Cable,
    ];

    pub fn mesh(self) -> Mesh {
        match self {
            Piece::WallPlain => wall_panel(0),
            Piece::WallRibbed => wall_panel(1),
            Piece::WallVent => wall_panel(2),
            Piece::WallWindow => wall_panel(3),
            Piece::Railing => railing(RAILING_TILE),
            Piece::Grating => grating_floor(GRATING_SIDE, GRATING_SIDE),
            Piece::DoorFrame => door_frame(),
            Piece::Lamp => lamp(),
            Piece::Pipe => pipe_straight(8.0, 0.25),
            Piece::Cable => cable_bundle(8.0, 5),
        }
    }
}

pub const RAILING_TILE: f32 = 2.0;
/// 118 quanta, a slot between its walls.
pub const GRATING_SIDE: f32 = 118.0 / 16.0;

/// Where a piece's triangles sit in the shared kit mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceRange {
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
}

/// Every instanced piece in one mesh, so a frame binds one vertex and one index buffer.
#[derive(Debug, Clone)]
pub struct KitSet {
    pub mesh: Mesh,
    pub ranges: Vec<PieceRange>,
}

impl KitSet {
    pub fn new() -> Self {
        let mut mesh = Mesh::default();
        let mut ranges = Vec::new();
        for piece in Piece::ALL {
            let m = piece.mesh();
            ranges.push(PieceRange {
                first_index: mesh.indices.len() as u32,
                index_count: m.indices.len() as u32,
                base_vertex: mesh.vertices.len() as i32,
            });
            mesh.vertices.extend_from_slice(&m.vertices);
            mesh.indices.extend_from_slice(&m.indices);
        }
        Self { mesh, ranges }
    }

    pub fn range(&self, piece: Piece) -> PieceRange {
        self.ranges[Piece::ALL.iter().position(|&p| p == piece).unwrap_or(0)]
    }
}

impl Default for KitSet {
    fn default() -> Self {
        Self::new()
    }
}

/// One placed piece, 16 bytes, mirrored by `ModInstance` in `scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModuleInstance {
    /// Base-middle of the piece in the chunk's frame, metres.
    pub pos: [f32; 3],
    /// Bits 0-1 the quarter turns about +Y; bits 2-17 the chunk's cell slot, filled in on upload.
    pub packed: u32,
}

/// The instances of one chunk, grouped by piece.
#[derive(Debug, Clone, Default)]
pub struct ModuleBatch {
    pub by_piece: Vec<Vec<ModuleInstance>>,
}

impl ModuleBatch {
    pub fn new() -> Self {
        Self {
            by_piece: vec![Vec::new(); Piece::ALL.len()],
        }
    }

    pub fn len(&self) -> usize {
        self.by_piece.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> u64 {
        (self.len() * std::mem::size_of::<ModuleInstance>()) as u64
    }

    fn push(&mut self, piece: Piece, pos: [f32; 3], facing: u8) {
        let k = Piece::ALL.iter().position(|&p| p == piece).unwrap_or(0);
        self.by_piece[k].push(ModuleInstance {
            pos,
            packed: u32::from(facing & 3),
        });
    }

    /// Bounding sphere of every instance's base point plus a piece's reach, as centre and radius.
    pub fn bounding_sphere(&self) -> [f32; 4] {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for i in self.by_piece.iter().flatten() {
            lo = lo.min(Vec3::from(i.pos));
            hi = hi.max(Vec3::from(i.pos));
        }
        if self.is_empty() {
            return [0.0; 4];
        }
        // The largest piece is 8 m long.
        let reach = 8.0;
        let c = (lo + hi) * 0.5;
        let r = (hi - lo).length() * 0.5 + reach;
        [c.x, c.y, c.z, r]
    }
}

/// Turns placements into instances. Long railings, pipes and cables are tiled with their fixed
/// pieces; positions go from quanta to metres.
pub fn expand(modules: &[worldgen::l4::Module]) -> ModuleBatch {
    use worldgen::l4::ModuleKind;
    let mut batch = ModuleBatch::new();
    let q = |v: i32| v as f32 / 16.0;
    for m in modules {
        let pos = [q(m.pos.x), q(m.pos.y), q(m.pos.z)];
        match m.kind {
            ModuleKind::WallPanel => {
                let piece = [
                    Piece::WallPlain,
                    Piece::WallRibbed,
                    Piece::WallVent,
                    Piece::WallWindow,
                ][usize::from(m.variant) % 4];
                batch.push(piece, pos, m.facing);
            }
            ModuleKind::DoorFrame => batch.push(Piece::DoorFrame, pos, m.facing),
            ModuleKind::Lamp => batch.push(Piece::Lamp, pos, m.facing),
            ModuleKind::Grating => batch.push(Piece::Grating, pos, m.facing),
            ModuleKind::Railing | ModuleKind::Pipe | ModuleKind::Cable => {
                let (piece, tile) = match m.kind {
                    ModuleKind::Railing => (Piece::Railing, RAILING_TILE),
                    ModuleKind::Pipe => (Piece::Pipe, 8.0),
                    _ => (Piece::Cable, 8.0),
                };
                let len = q(m.len);
                let n = (len / tile).ceil().max(1.0) as usize;
                // The pieces are centred on their origin along local x, which the facing turns
                // onto +x (0, 2) or onto z (1, 3).
                let dir = match m.facing & 3 {
                    0 => Vec3::X,
                    1 => Vec3::NEG_Z,
                    2 => Vec3::NEG_X,
                    _ => Vec3::Z,
                };
                let start = Vec3::from(pos) - dir * (len / 2.0);
                for i in 0..n {
                    // The last tile is pulled back so the run ends where it should.
                    let centre = (tile * (i as f32 + 0.5))
                        .min(len - tile / 2.0)
                        .max(tile / 2.0);
                    batch.push(piece, (start + dir * centre.min(len)).to_array(), m.facing);
                }
            }
        }
    }
    batch
}

/// Every piece with its name, in the order of the gallery.
pub fn pieces() -> Vec<(&'static str, Mesh)> {
    vec![
        ("wall_plain", wall_panel(0)),
        ("wall_ribbed", wall_panel(1)),
        ("wall_vent", wall_panel(2)),
        ("wall_window", wall_panel(3)),
        ("railing", railing(6.0)),
        ("grating", grating_floor(4.0, 2.0)),
        ("pipe_straight", pipe_straight(6.0, 0.15)),
        ("pipe_bend", pipe_bend(0.15, 0.6, 1.5)),
        ("cable_bundle", cable_bundle(6.0, 5)),
        ("door_frame", door_frame()),
        ("lamp", lamp()),
        ("stair", stair(SLOT_H)),
    ]
}

/// The pieces laid out in two rows on a floor, for inspection: wall panels along the back,
/// everything else in front. Returns one mesh in gallery coordinates (metres) and its extent.
pub fn gallery() -> (Mesh, Vec3) {
    let mut out = Mesh::default();
    let floor_w = 52.0;
    bx(
        &mut out,
        [-floor_w / 2.0, -0.2, -6.0],
        [floor_w / 2.0, 0.0, 20.0],
        CONCRETE,
    );
    let place = |out: &mut Mesh, mesh: &Mesh, at: Vec3| {
        out.add_transformed(mesh, Mat4::from_translation(at));
    };
    let all = pieces();
    let get = |n: &str| -> &Mesh {
        all.iter()
            .find(|(k, _)| *k == n)
            .map_or(&all[0].1, |(_, m)| m)
    };
    // Back row: walls, 10 m apart.
    for (i, n) in ["wall_plain", "wall_ribbed", "wall_vent", "wall_window"]
        .iter()
        .enumerate()
    {
        place(
            &mut out,
            get(n),
            Vec3::new(-15.0 + 10.0 * i as f32, 0.0, -4.0),
        );
    }
    // A door frame set into its own wall to the right.
    place(&mut out, get("wall_plain"), Vec3::new(25.0, 0.0, -4.0));
    place(&mut out, get("door_frame"), Vec3::new(25.0, 0.0, -4.0));
    // Front row.
    place(&mut out, get("railing"), Vec3::new(-20.0, 0.0, 6.0));
    place(&mut out, get("grating"), Vec3::new(-12.0, 0.0, 6.0));
    place(&mut out, get("pipe_straight"), Vec3::new(-4.0, 1.0, 6.0));
    place(&mut out, get("pipe_bend"), Vec3::new(2.5, 1.0, 6.0));
    place(&mut out, get("cable_bundle"), Vec3::new(10.0, 1.5, 6.0));
    place(&mut out, get("lamp"), Vec3::new(17.0, 3.0, 6.0));
    // A lamp's reach on the floor beneath, for scale: a 1.8 m figure post.
    bx(&mut out, [16.8, 0.0, 9.8], [17.2, 1.8, 10.2], METAL);
    // A ceiling slab for the lamp to hang from.
    bx(&mut out, [14.0, 3.0, 3.0], [20.0, 3.2, 9.0], CONCRETE);
    out.add_transformed(
        get("stair"),
        Mat4::from_translation(Vec3::new(23.0, 0.0, 6.0)) * Mat4::from_rotation_y(-FRAC_PI_2),
    );
    (out, Vec3::new(floor_w, SLOT_H, 26.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(m: &Mesh) -> (Vec3, Vec3) {
        m.vertices
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
                let p = Vec3::from(v.position);
                (lo.min(p), hi.max(p))
            })
    }

    #[test]
    fn every_piece_is_a_valid_mesh() {
        for (name, m) in pieces() {
            assert!(!m.vertices.is_empty() && m.indices.len() % 3 == 0, "{name}");
            assert!(
                m.indices.iter().all(|&i| (i as usize) < m.vertices.len()),
                "{name}"
            );
            for v in &m.vertices {
                assert!((oct_decode(v.normal).length() - 1.0).abs() < 1e-3, "{name}");
                assert!(Vec3::from(v.position).is_finite(), "{name}");
            }
        }
    }

    #[test]
    fn triangles_agree_with_their_normals() {
        for (name, m) in pieces() {
            let mut bad = 0;
            for tri in m.indices.chunks(3) {
                let p: Vec<Vec3> = tri
                    .iter()
                    .map(|&i| Vec3::from(m.vertices[i as usize].position))
                    .collect();
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                if n.length() < 1e-9 {
                    continue;
                }
                let stored = oct_decode(m.vertices[tri[0] as usize].normal);
                if n.normalize().dot(stored) < 0.0 {
                    bad += 1;
                }
            }
            assert_eq!(bad, 0, "{name}: {bad} triangles wound against their normal");
        }
    }

    #[test]
    fn wall_panels_fill_their_slot_face() {
        for v in 0..4 {
            let (lo, hi) = bounds(&wall_panel(v));
            assert!(
                (lo.x + 4.0).abs() < 1e-4 && (hi.x - 4.0).abs() < 1e-4,
                "{v}"
            );
            assert!(lo.y.abs() < 1e-4 && (hi.y - SLOT_H).abs() < 1e-4, "{v}");
            assert!((lo.z + PANEL_DEPTH).abs() < 1e-4 && hi.z <= 0.2, "{v}");
        }
    }

    #[test]
    fn the_door_opening_is_clear() {
        let m = door_frame();
        // Nothing stands in the opening (|x| < 1, 0.03 < y < 2.75).
        for tri in m.indices.chunks(3) {
            let c = tri
                .iter()
                .map(|&i| Vec3::from(m.vertices[i as usize].position))
                .sum::<Vec3>()
                / 3.0;
            assert!(
                !(c.x.abs() < 0.99 && c.y > 0.04 && c.y < 2.74),
                "frame intrudes at {c:?}"
            );
        }
    }

    #[test]
    fn stairs_climb_a_storey_within_the_step_limit() {
        let (lo, hi) = bounds(&stair(SLOT_H));
        assert!((hi.y - SLOT_H).abs() < RISE, "{hi:?}");
        assert!(lo.y >= -0.3);
    }

    #[test]
    fn collidable_additions_keep_the_wall_rule() {
        // Panel details protrude at most 0.25 m from the wall face (levels.md §7).
        for v in 0..4 {
            assert!(bounds(&wall_panel(v)).1.z <= 0.25);
        }
    }

    #[test]
    fn transforms_move_normals_with_the_piece() {
        let mut m = Mesh::default();
        m.add_transformed(
            &wall_panel(0),
            Mat4::from_rotation_y(FRAC_PI_2) * Mat4::from_scale(Vec3::ONE),
        );
        // The +Z face now looks along +X.
        let n = m
            .vertices
            .iter()
            .map(|v| oct_decode(v.normal))
            .find(|n| n.x > 0.99);
        assert!(n.is_some());
    }

    #[test]
    fn the_gallery_is_deterministic() {
        let (a, ea) = gallery();
        let (b, eb) = gallery();
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
        assert_eq!(ea, eb);
    }
}
