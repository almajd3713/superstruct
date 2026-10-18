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

#[test]
fn gpu_culling_matches_the_cpu_frustum() {
    use engine::camera::camera_relative;
    use engine::cull::Frustum;
    let Some(h) = headless() else { return };
    let mut cam = camera_at(I64Vec3::new(3, -2, 5));
    cam.look(0.7, 0.2);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32 * 200.0 - 100.0
    };
    let mut cube = Mesh::default();
    cube.add_box(Vec3::splat(-1.0), Vec3::splat(1.0), 0);
    let scene: Vec<_> = (0..500)
        .map(|_| {
            let d = Vec3::new(rnd(), rnd(), rnd());
            (cam.pos.offset(d).unwrap(), cube.clone())
        })
        .collect();
    let frustum = Frustum::from_view_proj(cam.view_proj(1.0));
    let expected = u32::try_from(
        scene
            .iter()
            .filter(|(o, _)| frustum.sphere_visible(camera_relative(o, &cam.pos), 3.0_f32.sqrt()))
            .count(),
    )
    .unwrap();
    assert!(
        expected > 20 && expected < 400,
        "degenerate scene: {expected}"
    );

    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, SIZE);
    renderer.set_scene(&h.device, &h.queue, &scene);
    h.render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    renderer.set_occlusion(false);
    h.render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    assert_eq!(renderer.read_drawn_counts(&h.device), (expected, 0));
}

#[test]
fn direct_and_indirect_draw_paths_render_identically() {
    let Some(h) = headless() else { return };
    let size = (160, 90);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let scene = demo_scene_at(I64Vec3::ZERO);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &scene);
    let params = FrameParams::default();
    let indirect = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    renderer.set_force_direct(true);
    let direct = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    assert_eq!(image_diff(&indirect, &direct), (0, 0.0));
}

#[test]
fn prepass_does_not_change_the_image() {
    let Some(h) = headless() else { return };
    let size = (160, 90);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &demo_scene_at(I64Vec3::ZERO));
    let params = FrameParams::default();
    let with = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    renderer.set_prepass(false);
    let without = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    assert_eq!(image_diff(&with, &without), (0, 0.0));
}

/// A dense scene: 40 large slabs stacked along the view axis, back to front in memory order,
/// so every pixel is shaded many times without a prepass. Run with `--ignored --nocapture`.
#[test]
#[ignore = "timing measurement, prints GPU milliseconds"]
fn prepass_cuts_shading_cost_in_a_dense_scene() {
    let Some(h) = headless() else { return };
    let size = (2560, 1440);
    let cam = camera_at(I64Vec3::ZERO);
    let scene: Vec<_> = (0..40u16)
        .map(|i| wall(&cam, 4.0 + f32::from(40 - i) * 0.5, 30.0, i % 6))
        .collect();
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &scene);
    let params = FrameParams {
        grid: true,
        ..PLAIN
    };
    for prepass in [false, true] {
        renderer.set_prepass(prepass);
        let Some(times) = h.gpu_times(&mut renderer, &cam, size, params, 30) else {
            eprintln!("SKIPPED: no timestamp queries");
            return;
        };
        let line: Vec<_> = times
            .iter()
            .map(|(z, ms)| format!("{} {ms:.3} ms", z.name()))
            .collect();
        eprintln!("prepass {prepass}: {}", line.join(", "));
    }
}

/// CPU reduction with the rule from `hiz.wgsl`: floor-sized levels, the last texel of an odd
/// source row or column folded into the final destination texel.
fn reduce_cpu(prev: &[f32], (pw, ph): (u32, u32)) -> Vec<f32> {
    let (w, h) = ((pw / 2).max(1), (ph / 2).max(1));
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let sx = (x * 2)..(x * 2 + if x == w - 1 { pw - x * 2 } else { 2 }).min(pw);
            let sy = (y * 2)..(y * 2 + if y == h - 1 { ph - y * 2 } else { 2 }).min(ph);
            let mut m = 1.0_f32;
            for yy in sy.clone() {
                for xx in sx.clone() {
                    m = m.min(prev[(yy * pw + xx) as usize]);
                }
            }
            out.push(m);
        }
    }
    out
}

#[test]
fn hiz_levels_are_conservative_minimums_and_debug_view_draws() {
    use engine::hiz::{level_count, level_size};
    let Some(h) = headless() else { return };
    let size = (157, 91);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &demo_scene_at(I64Vec3::ZERO));
    // With history the first frame's set A is empty, so its pyramid would be too.
    renderer.set_occlusion(false);
    let params = FrameParams::default();
    let plain = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();

    let hiz = renderer.hiz();
    let mut prev = hiz.read_level(&h.device, &h.queue, 0);
    assert!(prev.iter().any(|&d| d > 0.0), "level 0 saw no geometry");
    assert!(prev.contains(&0.0), "level 0 should have sky");
    for level in 1..level_count(size) {
        let expected = reduce_cpu(&prev, level_size(size, level - 1));
        let got = hiz.read_level(&h.device, &h.queue, level);
        assert_eq!(got, expected, "level {level}");
        prev = got;
    }

    renderer.set_debug_hiz(Some(2));
    let debug = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    assert!(image_diff(&plain, &debug).0 > 20, "debug view did not draw");
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/hiz-level2.png");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    save_png(&out, size.0, size.1, &debug).unwrap();
}

/// A wide wall with 300 cubes hidden behind it.
fn hidden_behind_wall() -> (Camera, Vec<(WorldPos, Mesh)>) {
    let cam = camera_at(I64Vec3::new(1, 0, -4));
    let mut scene = vec![wall(&cam, 5.0, 40.0, 1)];
    let mut cube = Mesh::default();
    cube.add_box(Vec3::splat(-0.5), Vec3::splat(0.5), 0);
    for i in 0..300u32 {
        let x = (i % 15) as f32 - 7.0;
        let y = ((i / 15) % 5) as f32 - 2.0;
        let z = -12.0 - (i / 75) as f32 * 9.0;
        scene.push((cam.pos.offset(Vec3::new(x, y, z)).unwrap(), cube.clone()));
    }
    (cam, scene)
}

#[test]
fn occlusion_culling_drops_hidden_instances_without_changing_the_image() {
    let Some(h) = headless() else { return };
    let (cam, scene) = hidden_behind_wall();
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, SIZE);
    renderer.set_scene(&h.device, &h.queue, &scene);

    renderer.set_occlusion(false);
    let reference = h
        .render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    let (frustum_only, _) = renderer.read_drawn_counts(&h.device);
    assert!(
        frustum_only >= 250,
        "scene should have many instances in view: {frustum_only}"
    );

    renderer.set_occlusion(true);
    let mut drawn = Vec::new();
    for frame in 0..4 {
        let img = h
            .render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
            .unwrap();
        let (a, b) = renderer.read_drawn_counts(&h.device);
        drawn.push(a + b);
        // No popping: whatever the history says, the picture is the same.
        assert_eq!(image_diff(&img, &reference), (0, 0.0), "frame {frame}");
    }
    eprintln!("drawn per frame with occlusion: {drawn:?}, frustum only: {frustum_only}");
    let settled = *drawn.last().unwrap();
    assert!(
        settled * 5 <= frustum_only,
        "wanted at least 5x fewer instances: {settled} vs {frustum_only}"
    );
}

#[test]
fn frozen_culling_keeps_the_old_frustum_while_the_camera_turns_away() {
    use engine::camera::camera_relative;
    use engine::cull::Frustum;
    let Some(h) = headless() else { return };
    let mut cam = camera_at(I64Vec3::new(3, -2, 5));
    cam.look(0.7, 0.2);
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32 * 200.0 - 100.0
    };
    let mut cube = Mesh::default();
    cube.add_box(Vec3::splat(-1.0), Vec3::splat(1.0), 0);
    let scene: Vec<_> = (0..500)
        .map(|_| {
            let d = Vec3::new(rnd(), rnd(), rnd());
            (cam.pos.offset(d).unwrap(), cube.clone())
        })
        .collect();
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, SIZE);
    renderer.set_scene(&h.device, &h.queue, &scene);
    renderer.set_occlusion(false);

    h.render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    let live = renderer.read_drawn_counts(&h.device).0;
    renderer.set_freeze_culling(true);
    h.render_with(&mut renderer, &cam, SIZE, PLAIN, &[])
        .unwrap();
    assert_eq!(renderer.read_drawn_counts(&h.device).0, live);

    // Turn around and step aside: the live frustum would see a different set.
    let mut turned = cam;
    turned.look(std::f32::consts::PI, 0.0);
    turned.pos = cam.pos.offset(Vec3::new(10.0, 0.0, 0.0)).unwrap();
    let frozen = Frustum::from_view_proj(cam.view_proj(1.0));
    let to_new = camera_relative(&turned.pos, &cam.pos);
    let expected = u32::try_from(
        scene
            .iter()
            .filter(|(o, _)| {
                let c = camera_relative(o, &turned.pos);
                frozen.translated(to_new).sphere_visible(c, 3.0_f32.sqrt())
            })
            .count(),
    )
    .unwrap();
    h.render_with(&mut renderer, &turned, SIZE, PLAIN, &[])
        .unwrap();
    assert_eq!(renderer.read_drawn_counts(&h.device).0, expected);
    assert_eq!(expected, live, "same world set, whatever the camera does");

    renderer.set_freeze_culling(false);
    h.render_with(&mut renderer, &turned, SIZE, PLAIN, &[])
        .unwrap();
    assert_ne!(renderer.read_drawn_counts(&h.device).0, live);
}

#[test]
fn far_instances_draw_like_the_meshed_primitives() {
    use engine::chunkmesh::mesh_primitives;
    use engine::farinst::far_instances;
    use worldgen::cell::Axis;
    use worldgen::prim::{FeatureKind, MaterialId, PrimFlags, Primitive, QAabb, QVec3, Shape};
    let Some(h) = headless() else { return };
    let size = (160, 120);
    let cam = camera_at(I64Vec3::ZERO);
    let prim = |shape, material| Primitive {
        shape,
        material: MaterialId(material),
        kind: FeatureKind::TRUNK,
        flags: PrimFlags::FAR_VISIBLE,
    };
    // Cell-local quanta (16 per metre), placed 40 m ahead of the camera.
    let b = |lo: [i32; 3], hi: [i32; 3]| Shape::Box {
        aabb: QAabb::new(
            QVec3::new(lo[0], lo[1], lo[2]),
            QVec3::new(hi[0], hi[1], hi[2]),
        ),
    };
    let prims = [
        prim(b([0, 0, 0], [320, 160, 320]), 0),
        prim(b([480, 0, 0], [800, 480, 160]), 1),
        prim(
            Shape::cylinder(Axis::Y, QVec3::new(160, 320, 640), 480, 128),
            4,
        ),
        prim(
            Shape::cylinder(Axis::X, QVec3::new(480, 160, 640), 480, 96),
            4,
        ),
        prim(
            Shape::cylinder(Axis::Z, QVec3::new(1000, 160, 400), 480, 96),
            5,
        ),
    ];
    let origin = cam.pos.offset(Vec3::new(-25.0, -8.0, -60.0)).unwrap();
    let mesh = mesh_primitives(&prims).0;
    let params = FrameParams::default();

    let mut meshed = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    meshed.set_scene(&h.device, &h.queue, &[(origin, mesh)]);
    let a = h.render_with(&mut meshed, &cam, size, params, &[]).unwrap();

    let mut instanced = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    instanced.upload_far_chunk(&h.queue, origin, &far_instances(&prims));
    let f = h
        .render_with(&mut instanced, &cam, size, params, &[])
        .unwrap();
    save_png(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/far-instanced.png"),
        size.0,
        size.1,
        &f,
    )
    .unwrap();
    save_png(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/far-meshed.png"),
        size.0,
        size.1,
        &a,
    )
    .unwrap();
    // Far cylinders are smooth-shaded with their own side count, the meshed ones flat, so
    // only the cylinders may differ.
    let (_, mean) = image_diff(&a, &f);
    assert!(mean < 1.5, "mean difference {mean}");
    assert!(
        image_diff(&a, &vec![a[0]; a.len()]).1 > 1.0,
        "scene should not be empty"
    );
}

/// CPU time to record a frame with the whole 50 km far band resident, and GPU zone times.
/// Run: `scripts/win.sh test --release -p engine --test render far_band_frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "timing measurement"]
fn far_band_frame_cost() {
    use engine::farinst::far_instances;
    use engine::renderer::FrameInput;
    use worldgen::cell::{CellKey, Level};
    use worldgen::l1::l1_far_primitives;
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (2560, 1440);
    let cam = camera_at(I64Vec3::ZERO);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    let mut cells = 0;
    for z in -13..=13 {
        for y in -13..=13 {
            for x in -13..=13 {
                let key = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                let Ok(b) = key.aabb_world() else { continue };
                let data = far_instances(&l1_far_primitives(WorldSeed(0), key));
                renderer.upload_far_chunk(&h.queue, b.min, &data);
                cells += 1;
            }
        }
    }
    let target = h.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: engine::headless::FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut cpu = Vec::new();
    for _ in 0..60 {
        let mut encoder = h
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let t = std::time::Instant::now();
        renderer.render(
            &h.queue,
            &mut encoder,
            &view,
            &FrameInput {
                camera: &cam,
                size,
                params: PLAIN,
                boxes: &[],
                timer: None,
                dt: 0.0,
            },
        );
        cpu.push(t.elapsed().as_secs_f64() * 1000.0);
        h.queue.submit([encoder.finish()]);
        let _ = h.device.poll(wgpu::PollType::wait_indefinitely());
    }
    cpu.sort_by(f64::total_cmp);
    eprintln!(
        "{cells} cells, {} chunks; render() CPU median {:.3} ms, max {:.3} ms",
        renderer.chunk_count(),
        cpu[30],
        cpu[59]
    );
    if let Some(t) = h.gpu_times(&mut renderer, &cam, size, PLAIN, 30) {
        for (z, ms) in t {
            eprintln!("gpu {} {ms:.3} ms", z.name());
        }
    }
}

/// Renders a screen-filling wall with the given fade state and returns which pixels were drawn
/// (not the clear colour).
fn fade_mask(h: &Headless, material: u16, fade: f32, fading_in: bool) -> Vec<bool> {
    let size = (96, 96);
    let cam = camera_at(I64Vec3::ZERO);
    let (origin, mesh) = wall(&cam, 2.0, 50.0, material);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    let (handle, _) = renderer.upload_chunk(&h.device, &h.queue, origin, &mesh);
    renderer.set_chunk_fade_state(handle, fade, fading_in);
    let img = h
        .render_with(&mut renderer, &cam, size, PLAIN, &[])
        .unwrap();
    let clear = h.render(&[], &cam, size, PLAIN, &[]).unwrap();
    img.chunks(4)
        .zip(clear.chunks(4))
        .map(|(a, b)| a != b)
        .collect()
}

#[test]
fn fade_level_sets_the_share_of_pixels_drawn() {
    let Some(h) = headless() else { return };
    for fade in [0.25_f32, 0.5, 0.75] {
        let mask = fade_mask(&h, 0, fade, true);
        let share = mask.iter().filter(|&&d| d).count() as f32 / mask.len() as f32;
        assert!((share - fade).abs() < 0.03, "fade {fade} drew {share}");
    }
}

#[test]
fn fading_in_and_fading_out_chunks_tile_the_screen_exactly() {
    let Some(h) = headless() else { return };
    for t in [0.2_f32, 0.5, 0.8] {
        let incoming = fade_mask(&h, 0, t, true);
        let outgoing = fade_mask(&h, 1, 1.0 - t, false);
        for (i, (a, b)) in incoming.iter().zip(&outgoing).enumerate() {
            assert!(
                a != b,
                "t {t}: pixel {i} drawn by {} chunks",
                u8::from(*a) + u8::from(*b)
            );
        }
    }
}

/// The horizon pass against the far band: the same L1 cells drawn as instanced geometry and by
/// ray marching, from one camera. Both leave out cavities and bridges differently, so they
/// should agree on nearly every pixel, not all.
#[test]
fn horizon_march_covers_the_same_pixels_as_the_far_band() {
    use engine::farinst::far_instances;
    use worldgen::cell::{CellKey, Level};
    use worldgen::l1::l1_far_primitives;
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (320, 180);
    let seed = WorldSeed(0);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    let params = FrameParams {
        fog_density: 0.0,
        ..PLAIN
    };
    let reach_cells = 6;
    let base = CellKey::containing(Level::L1, &start).coords;
    let mut agree_total = 0.0;
    let mut worst = 1.0_f64;
    for (n, yaw) in [0.0_f32, 1.0, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
        let mut cam = Camera::new(start, 70.0);
        cam.yaw = yaw;
        cam.pitch = 0.05;

        let mut far = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
        for z in -reach_cells..=reach_cells {
            for y in -reach_cells..=reach_cells {
                for x in -reach_cells..=reach_cells {
                    let key = CellKey::new(Level::L1, base + I64Vec3::new(x, y, z));
                    let Ok(b) = key.aabb_world() else { continue };
                    let data = far_instances(&l1_far_primitives(seed, key));
                    far.upload_far_chunk(&h.queue, b.min, &data);
                }
            }
        }
        let a = h.render_with(&mut far, &cam, size, params, &[]).unwrap();

        let mut march = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
        march.set_horizon_enabled(true);
        march.set_world_seed(seed.0);
        march.set_horizon_start(0.0);
        // The far band only holds cells within `reach_cells` of the camera's.
        march.set_horizon_end(reach_cells as f32 * 4096.0);
        let b = h.render_with(&mut march, &cam, size, params, &[]).unwrap();

        let clear = h.render(&[], &cam, size, params, &[]).unwrap();
        let drawn = |img: &[u8], i: usize| img[i * 4..i * 4 + 4] != clear[i * 4..i * 4 + 4];
        let pixels = (size.0 * size.1) as usize;
        let (mut both, mut either) = (0usize, 0usize);
        for i in 0..pixels {
            let (x, y) = (drawn(&a, i), drawn(&b, i));
            both += usize::from(x && y);
            either += usize::from(x || y);
        }
        let agree = if either == 0 {
            1.0
        } else {
            both as f64 / either as f64
        };
        eprintln!(
            "yaw {yaw}: far band {} px, march {} px, overlap {agree:.3}",
            (0..pixels).filter(|&i| drawn(&a, i)).count(),
            (0..pixels).filter(|&i| drawn(&b, i)).count()
        );
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/horizon-{n}-far.png")),
            size.0,
            size.1,
            &a,
        )
        .unwrap();
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/horizon-{n}-march.png")),
            size.0,
            size.1,
            &b,
        )
        .unwrap();
        agree_total += agree;
        worst = worst.min(agree);
    }
    eprintln!("mean overlap {:.3}, worst {worst:.3}", agree_total / 6.0);
}

#[test]
fn horizon_shows_megaforms_beyond_fifty_km() {
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (640, 360);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    let params = FrameParams {
        fog_density: 0.0,
        ..PLAIN
    };
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_horizon_enabled(true);
    renderer.set_world_seed(WorldSeed(0).0);
    renderer.set_horizon_start(51_200.0);
    let mut shares = Vec::new();
    for (n, yaw) in [0.0_f32, 1.6, 3.2, 4.8].into_iter().enumerate() {
        let mut cam = Camera::new(start, 70.0);
        cam.yaw = yaw;
        cam.pitch = 0.02;
        let img = h
            .render_with(&mut renderer, &cam, size, params, &[])
            .unwrap();
        let clear = h.render(&[], &cam, size, params, &[]).unwrap();
        let drawn = img
            .chunks(4)
            .zip(clear.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        shares.push(drawn as f64 / f64::from(size.0 * size.1));
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/horizon-50km-{n}.png")),
            size.0,
            size.1,
            &img,
        )
        .unwrap();
    }
    eprintln!("share of pixels with horizon structure: {shares:.3?}");
    assert!(shares.iter().all(|&s| s > 0.05), "{shares:?}");
}

/// GPU cost of the horizon at 1440p in clear air, where every pixel hits something.
/// Run: `scripts/win.sh test --release -p engine --test render horizon_cost -- --ignored --nocapture`.
#[test]
#[ignore = "timing measurement"]
fn horizon_cost() {
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (2560, 1440);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    let params = FrameParams {
        fog_density: 0.0,
        ..PLAIN
    };
    let mut cam = Camera::new(start, 70.0);
    cam.pitch = 0.02;
    let report = |label: &str, times: &[(engine::gpu_timer::Zone, f32)]| {
        let ms = |name: &str| {
            times
                .iter()
                .find(|(z, _)| z.name() == name)
                .map_or(0.0, |t| t.1)
        };
        eprintln!(
            "{label}: scene pass {:.3} ms, cache pass {:.3} ms",
            ms("scene"),
            ms("horizon cache")
        );
    };
    for on in [false, true] {
        let mut renderer =
            engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
        renderer.set_horizon_enabled(on);
        renderer.set_world_seed(WorldSeed(0).0);
        let Some(times) = h.gpu_times(&mut renderer, &cam, size, params, 30) else {
            eprintln!("SKIPPED: no timestamp queries");
            return;
        };
        report(&format!("horizon {on}, steady"), &times);
    }
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_horizon_enabled(true);
    renderer.set_horizon_half_res(true);
    renderer.set_world_seed(WorldSeed(0).0);
    let times = h.gpu_times(&mut renderer, &cam, size, params, 30).unwrap();
    let half = times
        .iter()
        .find(|(z, _)| z.name() == "horizon half")
        .map_or(0.0, |t| t.1);
    report("horizon true, half resolution, steady", &times);
    eprintln!("half-resolution march pass {half:.3} ms");
    // A new seed every frame forces a refill of the cell records each time.
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_horizon_enabled(true);
    let times = h
        .gpu_times_with(&mut renderer, &cam, size, params, 30, |r, frame| {
            r.set_world_seed(u64::from(frame));
        })
        .unwrap();
    report("horizon true, refilling every frame", &times);
}

#[test]
fn half_resolution_horizon_matches_full_resolution() {
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (640, 360);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    let params = FrameParams {
        fog_density: 0.0,
        ..PLAIN
    };
    let mut full = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    let mut half = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    for r in [&mut full, &mut half] {
        r.set_horizon_enabled(true);
        r.set_world_seed(WorldSeed(0).0);
    }
    half.set_horizon_half_res(true);
    for (n, yaw) in [0.0_f32, 1.6, 3.2].into_iter().enumerate() {
        let mut cam = Camera::new(start, 70.0);
        cam.yaw = yaw;
        cam.pitch = 0.02;
        let a = h.render_with(&mut full, &cam, size, params, &[]).unwrap();
        let b = h.render_with(&mut half, &cam, size, params, &[]).unwrap();
        let clear = h.render(&[], &cam, size, params, &[]).unwrap();
        let drawn = |img: &[u8], i: usize| img[i * 4..i * 4 + 4] != clear[i * 4..i * 4 + 4];
        let pixels = (size.0 * size.1) as usize;
        let (mut both, mut either) = (0usize, 0usize);
        for i in 0..pixels {
            let (x, y) = (drawn(&a, i), drawn(&b, i));
            both += usize::from(x && y);
            either += usize::from(x || y);
        }
        let overlap = both as f64 / either as f64;
        let (max, mean) = image_diff(&a, &b);
        eprintln!("yaw {yaw}: coverage overlap {overlap:.4}, mean diff {mean:.3}, max {max}");
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/horizon-half-{n}.png")),
            size.0,
            size.1,
            &b,
        )
        .unwrap();
        assert!(overlap > 0.95, "coverage overlap {overlap}");
        assert!(mean < 6.0, "mean difference {mean}");
    }
}

/// Far band to `reach_m` plus the horizon from there, in clear air, as one image.
#[allow(clippy::cast_possible_truncation)] // the reach is a few tens of kilometres
fn handoff_image(h: &Headless, reach_m: f64, cam: &Camera, size: (u32, u32)) -> Vec<u8> {
    use engine::farinst::far_instances;
    use engine::stream::{ChunkId, Detail, StreamConfig, desired_set};
    use worldgen::cell::Level;
    use worldgen::l1::l1_far_primitives;
    use worldgen::seed::WorldSeed;
    let seed = WorldSeed(0);
    let cfg = StreamConfig {
        l2_radius_m: 0.0,
        l2_mid_radius_m: 0.0,
        l1_full_radius_m: 0.0,
        l1_far_radius_m: reach_m,
    };
    let mut wanted = Vec::new();
    desired_set(&cfg, &cam.pos, Vec3::NEG_Z, &mut wanted);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    for w in wanted
        .iter()
        .filter(|w| w.id.key.level == Level::L1 && w.id.detail == Detail::Far)
    {
        let ChunkId { key, .. } = w.id;
        let Ok(b) = key.aabb_world() else { continue };
        renderer.upload_far_chunk(
            &h.queue,
            b.min,
            &far_instances(&l1_far_primitives(seed, key)),
        );
    }
    renderer.set_horizon_enabled(true);
    renderer.set_world_seed(seed.0);
    renderer.set_horizon_start(reach_m as f32);
    let params = FrameParams {
        fog_density: 0.0,
        ..PLAIN
    };
    h.render_with(&mut renderer, cam, size, params, &[])
        .unwrap()
}

/// The far band and the horizon draw the same structure, so moving the handoff radius must
/// not change the picture: any seam would show up as a difference between two radii.
#[test]
fn moving_the_handoff_radius_does_not_change_the_picture() {
    let Some(h) = headless() else { return };
    let size = (640, 360);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    for (n, yaw) in [0.0_f32, 2.1, 4.2].into_iter().enumerate() {
        let mut cam = Camera::new(start, 70.0);
        cam.yaw = yaw;
        cam.pitch = 0.02;
        let near = handoff_image(&h, 20_480.0, &cam, size);
        let far = handoff_image(&h, 51_200.0, &cam, size);
        let (max, mean) = image_diff(&near, &far);
        eprintln!("yaw {yaw}: handoff 20 km vs 51 km, mean diff {mean:.3}, max {max}");
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/handoff-{n}-20km.png")),
            size.0,
            size.1,
            &near,
        )
        .unwrap();
        save_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../out/handoff-{n}-51km.png")),
            size.0,
            size.1,
            &far,
        )
        .unwrap();
        assert!(mean < 2.0, "mean difference {mean}");
    }
}

#[test]
fn fully_fogged_horizon_pixels_are_skipped() {
    use worldgen::seed::WorldSeed;
    let Some(h) = headless() else { return };
    let size = (160, 90);
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(10.0, 745.0, 584.0)).unwrap();
    let cam = Camera::new(start, 70.0);
    // 0.0008 per metre over 51 km: nothing beyond the horizon's start can show.
    let params = FrameParams {
        fog_density: 0.0008,
        ..PLAIN
    };
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_horizon_enabled(true);
    renderer.set_world_seed(WorldSeed(0).0);
    let with = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    let without = h.render(&[], &cam, size, params, &[]).unwrap();
    assert_eq!(image_diff(&with, &without), (0, 0.0));
}

/// P4.2.1: the module kit laid out for inspection. Writes `out/kit-gallery.png`.
#[test]
fn kit_gallery_renders() {
    let Some(h) = headless() else { return };
    let (mesh, _) = engine::kit::gallery();
    let params = FrameParams {
        fog_density: 0.0,
        fog_height_k: 0.0,
        fog_color: [0.02, 0.025, 0.03],
        ambient: [0.27, 0.27, 0.27],
        light: 0.35,
        grid: false,
        fade: 0.0,
    };
    let size = (1600, 800);
    let shots: [(&str, Vec3, f32, f32); 5] = [
        ("kit-gallery.png", Vec3::new(0.0, 7.0, 24.0), 0.0, -0.22),
        (
            "kit-gallery-lamp.png",
            Vec3::new(13.5, 1.8, 11.0),
            0.0,
            0.15,
        ),
        (
            "kit-gallery-walls.png",
            Vec3::new(-6.0, 2.2, 13.0),
            0.0,
            -0.1,
        ),
        (
            "kit-gallery-left.png",
            Vec3::new(-16.0, 3.0, 12.0),
            0.0,
            -0.3,
        ),
        (
            "kit-gallery-right.png",
            Vec3::new(19.0, 3.2, 15.0),
            -0.5,
            -0.3,
        ),
    ];
    for (name, at, yaw, pitch) in shots {
        let mut cam = camera_at(I64Vec3::ZERO);
        cam.pos = cam.pos.offset(at).unwrap();
        cam.yaw = yaw;
        cam.pitch = pitch;
        let origin = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0)).unwrap();
        let img = h
            .render(&[(origin, mesh.clone())], &cam, size, params, &[])
            .unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../out")
            .join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        save_png(&path, size.0, size.1, &img).unwrap();
        assert!(img.chunks(4).any(|p| p[0] > 40), "{name} is black");
    }
}

/// P4.2.3: the densest of a sample of real L3 blocks, meshed from its primitives and decorated by
/// instanced L4 modules, at 1440p from inside it. Writes `out/l4-block*.png`, prints the cost
/// and checks the geometry budget of PLAN.md §11.4.
#[test]
fn l4_modules_in_a_dense_block_meet_the_geometry_budget() {
    use engine::kit::expand;
    use worldgen::cell::{CellKey, Level};
    use worldgen::l2_port::L1Cache;
    use worldgen::l3::{l3_block_with, l3_blocks_of, l3_parent};
    use worldgen::l4::{ModuleKind, block_modules};
    use worldgen::seed::WorldSeed;

    let Some(h) = headless() else { return };
    let seed = WorldSeed(0x0423);
    let cache = L1Cache::default();
    // The sampled block with the most modules.
    let mut best: Option<(usize, CellKey, worldgen::l3::L3Block)> = None;
    'scan: for x in -30..30i64 {
        let l1 = CellKey::new(Level::L1, I64Vec3::new(x, x % 4, 2 * x % 7));
        for k in (0..512i64).step_by(5) {
            let cell = CellKey::new(
                Level::L2,
                l1.coords * 8 + I64Vec3::new(k % 8, k / 64, k / 8 % 8),
            );
            if l3_blocks_of(seed, &cache, cell).is_empty() {
                continue;
            }
            let parent = l3_parent(seed, &cache, cell);
            for (key, _) in l3_blocks_of(seed, &cache, cell) {
                let b = l3_block_with(seed, &parent, key);
                let n = block_modules(seed, key, &b).len();
                if best.as_ref().is_none_or(|(m, _, _)| n > *m) {
                    best = Some((n, key, b));
                }
            }
            if best.as_ref().is_some_and(|(n, _, _)| *n > 3000) {
                break 'scan;
            }
        }
    }
    let (n_modules, key, block) = best.unwrap();
    let modules = block_modules(seed, key, &block);
    let batch = expand(&modules);
    let mut prims = Mesh::default();
    for p in &block.out.primitives {
        engine::primmesh::add_primitive(&mut prims, p);
    }
    eprintln!(
        "block {key:?}: {n_modules} modules -> {} instances ({} KB), {} primitives -> {} vertices",
        batch.len(),
        batch.bytes() / 1024,
        block.out.primitives.len(),
        prims.vertices.len()
    );

    let origin = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0)).unwrap();
    let size = (2560, 1440);
    let params = FrameParams {
        light: 0.35,
        ..FrameParams::default()
    };
    // Stand in the first lit room slot, at eye height above its floor.
    let lamp = modules
        .iter()
        .find(|m| m.kind == ModuleKind::Lamp)
        .expect("a lit slot");
    let eye = Vec3::new(
        lamp.pos.x as f32 / 16.0,
        lamp.pos.y as f32 / 16.0 - 4.0 + 0.25 + 1.65,
        lamp.pos.z as f32 / 16.0,
    );
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0) + eye).unwrap();

    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &[(origin, prims)]);
    let base = h.gpu_times(&mut renderer, &cam, size, params, 30);
    let (handle, bytes) = renderer.upload_module_chunk(&h.queue, origin, &batch);
    assert!(bytes > 0 && renderer.chunk_count() >= 2);
    let with = h.gpu_times(&mut renderer, &cam, size, params, 30);
    let img = h
        .render_with(&mut renderer, &cam, size, params, &[])
        .unwrap();
    assert!(renderer.module_instances_drawn() > 0, "nothing was drawn");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/l4-block.png");
    save_png(&path, size.0, size.1, &img).unwrap();
    renderer.remove_chunk(handle);
    assert!(renderer.gpu_bytes() > 0);

    if let (Some(base), Some(with)) = (base, with) {
        let scene = |t: &[(engine::gpu_timer::Zone, f32)]| {
            t.iter()
                .find(|(z, _)| *z == engine::gpu_timer::Zone::Scene)
                .map_or(0.0, |(_, ms)| *ms)
        };
        eprintln!(
            "scene GPU ms: block meshes only {:.3}, with {} module instances {:.3}",
            scene(&base),
            batch.len(),
            scene(&with)
        );
        assert!(scene(&with) < 6.0, "geometry budget is 6 ms");
    }
}

/// P4.3.2: emissive module parts (lamps, window strips, door lights) show in a dark scene, scale
/// with their brightness level, and go dark when the chunk's emissive strength is zero.
#[test]
fn emissive_modules_are_visible_in_the_dark() {
    use engine::kit::expand;
    use worldgen::l4::{Module, ModuleKind};
    use worldgen::prim::QVec3;

    let Some(h) = headless() else { return };
    let m = |kind, pos: [i32; 3], facing, variant, glow| Module {
        kind,
        pos: QVec3::new(pos[0], pos[1], pos[2]),
        facing,
        len: 0,
        width: 0,
        variant,
        glow,
    };
    // Eight metres ahead: a window panel at full, half and zero brightness side by side, a lamp
    // above the camera and a door light.
    let mods = [
        m(ModuleKind::WallPanel, [-128, 0, -128], 0, 3, 15),
        m(ModuleKind::WallPanel, [0, 0, -128], 0, 3, 8),
        m(ModuleKind::WallPanel, [128, 0, -128], 0, 3, 0),
        m(ModuleKind::Lamp, [0, 64, 0], 0, 0, 15),
    ];
    let origin = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0)).unwrap();
    let dark = FrameParams {
        fog_density: 0.0,
        fog_height_k: 0.0,
        fog_color: [0.0, 0.0, 0.0],
        ambient: [0.02, 0.02, 0.02],
        light: 0.0,
        grid: false,
        fade: 0.0,
    };
    let size = (960, 360);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 514.0, 512.0)).unwrap();
    let render = |strength: f32| {
        let mut batch = expand(&mods);
        batch.glow = [1.0, 0.95, 0.85, strength];
        let mut renderer =
            engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
        renderer.set_scene(&h.device, &h.queue, &[]);
        renderer.upload_module_chunk(&h.queue, origin, &batch);
        h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap()
    };
    let lit = render(1.0);
    let off = render(0.0);
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/l4-emissive-dark.png");
    save_png(&path, size.0, size.1, &lit).unwrap();
    // The three window panels sit at x = -8, 0 and 8 m; probe one pane of each.
    let strip = |img: &[u8], fx: f32| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (x, y) = ((fx * size.0 as f32) as u32, 180);
        let i = ((y * size.0 + x) * 4) as usize;
        u32::from(img[i]) + u32::from(img[i + 1]) + u32::from(img[i + 2])
    };
    let level = |fx| strip(&lit, fx);
    let (a, b, c) = (level(0.198), level(0.469), level(0.742));
    eprintln!("window strips full {a}, half {b}, off {c}");
    assert!(a > 450, "a full strip glows in the dark: {a}");
    assert!(a > b && b > c, "brightness orders the strips: {a} {b} {c}");
    assert!(c < 260, "a dead strip stays dim: {c}");
    let max_off = off
        .chunks(4)
        .map(|p| u32::from(p[0]).max(u32::from(p[1])))
        .max()
        .unwrap();
    assert!(max_off < 80, "no glow at zero strength: {max_off}");
}

/// P4.3.3: each stylization prototype toggles on its own. With all off the image is the plain
/// scene; each one changes it in its own way. Writes `out/post-*.png`.
#[test]
fn stylization_passes_toggle_independently() {
    use engine::post::PostSettings;
    let Some(h) = headless() else { return };
    let size = (960, 540);
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let scene = demo_scene_at(I64Vec3::ZERO);
    let params = FrameParams::default();
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &scene);
    let mut shot = |name: &str, s: PostSettings| {
        renderer.set_post(s);
        let img = h
            .render_with(&mut renderer, &cam, size, params, &[])
            .unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../out")
            .join(format!("post-{name}.png"));
        save_png(&path, size.0, size.1, &img).unwrap();
        img
    };
    let plain = shot("off", PostSettings::default());
    let again = shot("off-again", PostSettings::default());
    assert_eq!(
        plain, again,
        "with everything off the render is deterministic"
    );
    let on = |f: fn(&mut PostSettings)| {
        let mut s = PostSettings::default();
        f(&mut s);
        s
    };
    let poster = shot("posterize", on(|s| s.posterize = true));
    let dither = shot("dither", on(|s| s.dither = true));
    let ink = shot("outlines", on(|s| s.outlines = true));
    let grain_a = shot("grain", on(|s| s.grain = true));
    let grain_b = shot("grain-2", on(|s| s.grain = true));
    let all = shot("all", PostSettings::ALL);

    let colours = |img: &[u8]| {
        img.chunks(4)
            .map(|p| [p[0], p[1], p[2]])
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    // Posterize leaves few levels per channel; the plain scene has many more colours.
    assert!(
        colours(&poster) < colours(&plain) / 2,
        "{} {}",
        colours(&poster),
        colours(&plain)
    );
    assert!(colours(&poster) <= 5 * 5 * 5, "{}", colours(&poster));
    // Dither alone differs from the plain render and from the plain ladder (no pattern).
    assert_ne!(dither, plain);
    assert_ne!(dither, poster);
    // Ink darkens pixels at edges and leaves flat areas alone.
    let darker = ink
        .chunks(4)
        .zip(plain.chunks(4))
        .filter(|(a, b)| u32::from(a[0]) + 60 < u32::from(b[0]))
        .count();
    let same = ink
        .chunks(4)
        .zip(plain.chunks(4))
        .filter(|(a, b)| a == b)
        .count();
    assert!(darker > 500, "ink drew {darker} dark pixels");
    assert!(
        same > ink.len() / 4 / 2,
        "ink left only {same} pixels alone"
    );
    // Grain moves from frame to frame.
    assert_ne!(grain_a, grain_b);
    assert_ne!(all, plain);
    // Everything back off restores the plain scene exactly.
    assert_eq!(shot("off-last", PostSettings::default()), plain);

    // Cost of all four at 1440p, reported (the post zone has no budget of its own yet: the
    // stylization and post share 1.5 ms in PLAN.md 11.4).
    let big = (2560, 1440);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, big);
    renderer.set_scene(&h.device, &h.queue, &scene);
    renderer.set_post(PostSettings::ALL);
    if let Some(times) = h.gpu_times(&mut renderer, &cam, big, params, 30) {
        let post = times
            .iter()
            .find(|(z, _)| *z == engine::gpu_timer::Zone::Post)
            .map_or(0.0, |(_, ms)| *ms);
        eprintln!("post zone, all four at 1440p: {post:.3} ms");
        assert!(post < 1.5, "stylization budget is 1.5 ms: {post}");
    }
}

/// P5.1.1: clustered point lights give the same picture as testing every light, and 1000 of
/// them stay inside the culling (0.5 ms) and geometry (6 ms) budgets at 1440p. Writes
/// `out/lights-*.png`.
#[test]
fn clustered_lights_match_brute_force_and_fit_the_budget() {
    use engine::gpu_timer::Zone;
    use engine::lights::Light;
    let Some(h) = headless() else { return };
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = cam.pos.offset(Vec3::new(0.0, 1.65, -54.0)).unwrap();
    cam.yaw = std::f32::consts::PI;
    let scene = demo_scene_at(I64Vec3::ZERO);
    let dark = FrameParams {
        ambient: [0.05, 0.05, 0.05],
        light: 0.02,
        ..FrameParams::default()
    };
    // A deterministic scatter of coloured lights over the demo scene.
    let lights = |n: u32| -> Vec<Light> {
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        (0..n)
            .map(|_| {
                let at = Vec3::new(next() * 120.0 - 60.0, next() * 12.0, next() * 120.0 - 60.0);
                Light {
                    pos: WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0) + at).unwrap(),
                    colour: [0.6 + 0.4 * next(), 0.6 + 0.4 * next(), 0.6 + 0.4 * next()],
                    intensity: 6.0,
                    radius: 9.0,
                }
            })
            .collect()
    };

    let size = (960, 540);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    renderer.set_scene(&h.device, &h.queue, &scene);
    let unlit = h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap();
    renderer.set_lights(&lights(300));
    let clustered = h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap();
    assert!(
        renderer.lights_active() > 50,
        "{}",
        renderer.lights_active()
    );
    renderer.set_lights_brute_force(true);
    let brute = h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    save_png(
        &dir.join("lights-clustered.png"),
        size.0,
        size.1,
        &clustered,
    )
    .unwrap();
    let (max, mean) = image_diff(&clustered, &brute);
    assert!(
        max <= 3 && mean < 0.05,
        "clusters differ from brute force: max {max}, mean {mean}"
    );
    let (max, _) = image_diff(&clustered, &unlit);
    assert!(max > 40, "the lights light something: {max}");

    // Cost at 1440p with 1000 lights.
    let big = (2560, 1440);
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, big);
    renderer.set_scene(&h.device, &h.queue, &scene);
    renderer.set_lights(&lights(1000));
    if let Some(times) = h.gpu_times(&mut renderer, &cam, big, dark, 30) {
        let ms = |zone| {
            times
                .iter()
                .find(|(z, _)| *z == zone)
                .map_or(0.0, |(_, ms)| *ms)
        };
        eprintln!(
            "1000 lights at 1440p: light cull {:.3} ms, scene {:.3} ms",
            ms(Zone::LightCull),
            ms(Zone::Scene)
        );
        assert!(ms(Zone::LightCull) < 0.5, "culling budget is 0.5 ms");
        assert!(ms(Zone::Scene) < 6.0, "geometry budget is 6 ms");
    }
    let img = h.render_with(&mut renderer, &cam, big, dark, &[]).unwrap();
    save_png(&dir.join("lights-1000.png"), big.0, big.1, &img).unwrap();
}

/// P5.1.2: lit module lamps become point lights, only the nearest within the per-view budget,
/// and none when the chunk's emissive strength is zero.
#[test]
fn module_lamps_light_the_view_within_the_budget() {
    use engine::kit::expand;
    use worldgen::l4::{Module, ModuleKind};
    use worldgen::prim::QVec3;
    let Some(h) = headless() else { return };
    // A 20 × 20 grid of lamps 8 m apart on a ceiling, some dead.
    let mods: Vec<Module> = (0..400)
        .map(|i| Module {
            kind: ModuleKind::Lamp,
            pos: QVec3::new((i % 20) * 128 + 64, 64, (i / 20) * 128 + 64),
            facing: 0,
            len: 0,
            width: 0,
            variant: 0,
            glow: if i % 5 == 0 { 0 } else { 15 },
        })
        .collect();
    let batch = expand(&mods);
    assert_eq!(batch.lamps.len(), 320, "dead lamps give no light");
    let origin = WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 512.0, 512.0)).unwrap();
    let mut cam = camera_at(I64Vec3::ZERO);
    cam.pos = WorldPos::new(I64Vec3::ZERO, Vec3::new(520.0, 514.0, 520.0)).unwrap();
    cam.pitch = -0.7;
    let size = (640, 360);
    let dark = FrameParams {
        ambient: [0.03, 0.03, 0.03],
        light: 0.0,
        fog_density: 0.0,
        ..FrameParams::default()
    };
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    let floor = {
        let mut m = Mesh::default();
        m.add_box(
            Vec3::new(-10.0, -0.3, -10.0),
            Vec3::new(170.0, 0.0, 170.0),
            0,
        );
        m
    };
    renderer.set_scene(&h.device, &h.queue, &[(origin, floor)]);
    let mut up = batch.clone();
    up.glow[3] = 1.0;
    renderer.upload_module_chunk(&h.queue, origin, &up);
    let mut run = |budget: usize| {
        renderer.set_light_budget(budget);
        let img = h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap();
        (renderer.lights_active(), img)
    };
    let (n_small, img_small) = run(4);
    let (n_big, img_big) = run(512);
    let (n_none, img_none) = run(0);
    assert_eq!(n_none, 0);
    assert!(img_none != img_big, "the lamps light the floor");
    assert_eq!(n_small, 4, "the budget caps the lights");
    assert_eq!(n_big, 320, "all lit lamps fit the default budget");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    save_png(&dir.join("lamps-lit.png"), size.0, size.1, &img_big).unwrap();
    save_png(&dir.join("lamps-few.png"), size.0, size.1, &img_small).unwrap();

    // A blacked-out chunk lights nothing.
    let mut renderer = engine::renderer::Renderer::new(&h.device, engine::headless::FORMAT, size);
    let mut off = batch;
    off.glow[3] = 0.0;
    renderer.upload_module_chunk(&h.queue, origin, &off);
    renderer.set_scene(&h.device, &h.queue, &[]);
    let _ = h.render_with(&mut renderer, &cam, size, dark, &[]).unwrap();
    // Strength zero keeps the lights but makes them black: intensity is zero.
    assert!(renderer.lights_active() > 0);
}
