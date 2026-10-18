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
    // Cylinders are polygons with a different side count, so only their rims may differ.
    let (_, mean) = image_diff(&a, &f);
    assert!(mean < 0.6, "mean difference {mean}");
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
