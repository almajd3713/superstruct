#![allow(clippy::unwrap_used, clippy::cloned_ref_to_slice_refs)]
//! Headless render tests. They skip, loudly, when the machine has no adapter: WSL without
//! `mesa-vulkan-drivers` (lavapipe) is such a machine, native Windows is not.

use engine::camera::Camera;
use engine::demo::demo_scene_at;
use engine::headless::{Headless, image_diff};
use engine::mesh::Mesh;
use engine::readback::save_png;
use engine::renderer::FrameParams;
use glam::{I64Vec3, Vec3};
use worldgen::coord::WorldPos;

const SIZE: (u32, u32) = (64, 64);
const PLAIN: FrameParams = FrameParams {
    fog_density: 0.0,
    fog_height_k: 0.0,
    fog_color: [0.02, 0.025, 0.03],
    ambient: [0.27, 0.27, 0.27],
    light: 0.25,
    grid: false,
    fade: 0.0,
};

fn headless() -> Option<Headless> {
    let h = Headless::new();
    if h.is_none() {
        eprintln!("SKIPPED: no GPU adapter available for headless rendering");
    }
    if let Some(h) = &h {
        eprintln!("adapter: {}", h.adapter_name);
    }
    h
}

fn camera_at(sector: I64Vec3) -> Camera {
    let pos = WorldPos::new(sector, Vec3::new(512.0, 512.0, 512.0)).unwrap();
    Camera::new(pos, 70.0)
}

/// A slab facing the camera: front face `dist` metres ahead, `half` metres to each side.
fn wall(cam: &Camera, dist: f32, half: f32, material: u16) -> (WorldPos, Mesh) {
    let origin = cam.pos.offset(Vec3::new(0.0, 0.0, -dist)).unwrap();
    let mut mesh = Mesh::default();
    mesh.add_box(
        Vec3::new(-half, -half, -2.0),
        Vec3::new(half, half, 0.0),
        material,
    );
    (origin, mesh)
}

fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

#[test]
fn depth_sorts_at_1m_and_10km() {
    let Some(h) = headless() else { return };
    let cam = camera_at(I64Vec3::ZERO);
    let render = |scene: &[(WorldPos, Mesh)]| h.render(scene, &cam, SIZE, PLAIN, &[]).unwrap();

    let near = wall(&cam, 1.0, 0.25, 1);
    let far = wall(&cam, 10_000.0, 8_000.0, 2);
    let near_only = render(&[near.clone()]);
    let far_only = render(&[far.clone()]);
    for order in [[far.clone(), near.clone()], [near.clone(), far.clone()]] {
        let both = render(&order);
        assert_eq!(
            pixel(&both, 32, 32),
            pixel(&near_only, 32, 32),
            "near box must cover the centre"
        );
        assert_eq!(
            pixel(&both, 2, 32),
            pixel(&far_only, 2, 32),
            "far wall visible beside it"
        );
    }
    assert_ne!(pixel(&near_only, 32, 32), pixel(&far_only, 32, 32));

    // Two walls 5 m apart at 10 km: the nearer must win whatever the draw order.
    let front = wall(&cam, 10_000.0, 8_000.0, 1);
    let back = wall(&cam, 10_005.0, 8_000.0, 2);
    let front_only = render(&[front.clone()]);
    let back_only = render(&[back.clone()]);
    assert_ne!(pixel(&front_only, 32, 32), pixel(&back_only, 32, 32));
    for order in [[front.clone(), back.clone()], [back, front]] {
        assert_eq!(
            render(&order),
            front_only,
            "5 m separation at 10 km must sort"
        );
    }
}

#[test]
fn grid_does_not_swim_at_large_coordinates() {
    let Some(h) = headless() else { return };
    let params = FrameParams::default();
    let size = (320, 180);
    let at = |sector: I64Vec3| {
        let mut cam = camera_at(sector);
        cam.pos = cam.pos.offset(Vec3::new(-54.0, 1.65, -54.0)).unwrap();
        cam.yaw = std::f32::consts::PI;
        h.render(&demo_scene_at(sector), &cam, size, params, &[])
            .unwrap()
    };
    let origin = at(I64Vec3::ZERO);
    let far = at(I64Vec3::new(1 << 40, -(1 << 40), (1 << 40) + 12_345));
    let (max, mean) = image_diff(&origin, &far);
    assert!(max <= 2 && mean < 0.05, "max {max}, mean {mean}");
}

#[test]
fn demo_scene_matches_reference() {
    let Some(h) = headless() else { return };
    let size = (320, 180);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let img = h
        .render(
            &demo_scene_at(I64Vec3::ZERO),
            &cam,
            size,
            FrameParams::default(),
            &[],
        )
        .unwrap();

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/demo.png");
    if std::env::var_os("UPDATE_REFERENCE").is_some() || !path.exists() {
        save_png(&path, size.0, size.1, &img).unwrap();
        eprintln!("wrote reference {}", path.display());
        return;
    }
    let reference = image::open(&path).unwrap().to_rgba8();
    assert_eq!(reference.dimensions(), size);
    // Different GPUs and drivers round differently; the tolerance catches real regressions only.
    let (max, mean) = image_diff(&img, reference.as_raw());
    assert!(
        mean < 1.5,
        "mean diff {mean} (max {max}) on {}",
        h.adapter_name
    );
}

#[test]
fn broken_shader_keeps_the_old_pipeline() {
    let Some(h) = headless() else { return };
    let cam = camera_at(I64Vec3::ZERO);
    let scene = [wall(&cam, 2.0, 1.0, 1)];
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, SIZE);
    renderer.set_scene(&h.device, &h.queue, &scene);
    let before = h
        .render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();

    let mut broken = engine::shaders::ShaderSources::embedded();
    broken.scene.push_str("\nfn oops( { this is not wgsl");
    assert!(renderer.reload_shaders(&h.device, &broken).is_err());
    assert!(renderer.shader_error().is_some());
    let after = h
        .render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    assert_eq!(before, after, "old pipeline must keep rendering");

    renderer
        .reload_shaders(&h.device, &engine::shaders::ShaderSources::embedded())
        .unwrap();
    assert!(renderer.shader_error().is_none());
}
