#![allow(clippy::unwrap_used)]
//! Streams for real while a camera flies, and measures how much each frame differs from the
//! last, with and without the band cross-fade. Skips without a GPU adapter.

use std::time::{Duration, Instant};

use engine::camera::Camera;
use engine::headless::Headless;
use engine::meshpool::MeshPool;
use engine::renderer::{FrameParams, Renderer};
use engine::stream::StreamConfig;
use engine::streamer::{RendererSink, Streamer, StreamerConfig};
use glam::{I64Vec3, Vec3};
use worldgen::coord::WorldPos;
use worldgen::seed::WorldSeed;

const SIZE: (u32, u32) = (320, 180);
// Slow enough that camera motion is almost invisible frame to frame, so what remains is
// streaming swaps.
const SPEED: f32 = 6.0;
const FRAME: f32 = 1.0 / 60.0;

/// Pixels that changed noticeably between the picture just before a frame's streaming update
/// and the picture just after it, at the same camera position. That isolates what the update
/// itself popped in or out, whatever the camera motion does.
fn flythrough(h: &Headless, fades: bool, frames: usize) -> Vec<usize> {
    let mut renderer = Renderer::new(&h.device, engine::headless::FORMAT, SIZE);
    renderer.set_fades_enabled(fades);
    let mut streamer = Streamer::new(
        StreamerConfig {
            stream: StreamConfig::default(),
            upload_budget_bytes: 16 << 20,
            memory_budget_bytes: 768 << 20,
            memory_hard_cap_bytes: u64::MAX,
        },
        MeshPool::new(WorldSeed(0), 8),
    );
    let start = WorldPos::new(I64Vec3::new(-2, -2, -3), Vec3::new(500.0, 745.0, 584.0)).unwrap();
    let mut cam = Camera::new(start, 70.0);
    cam.yaw = -std::f32::consts::FRAC_PI_2;
    let dir = cam.forward();
    let mut pops = Vec::new();
    let params = FrameParams::default();
    for _ in 0..frames {
        cam.pos = cam.pos.offset(dir * SPEED * FRAME).unwrap();
        let before = h
            .render_dt(&mut renderer, &cam, SIZE, params, &[], 0.0)
            .unwrap();
        // Let every load finish inside the frame so the run does not depend on thread timing.
        let t = Instant::now();
        loop {
            streamer.update(
                &cam.pos,
                dir,
                &mut RendererSink {
                    renderer: &mut renderer,
                    device: &h.device,
                    queue: &h.queue,
                },
            );
            let st = streamer.stats();
            if st.pool.queued + st.pool.running + st.ready == 0 {
                break;
            }
            assert!(t.elapsed() < Duration::from_mins(2));
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = h
            .render_dt(&mut renderer, &cam, SIZE, params, &[], FRAME)
            .unwrap();
        let changed = before
            .chunks(4)
            .zip(after.chunks(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 3))
            .count();
        pops.push(changed);
    }
    pops
}

#[test]
#[ignore = "slow, prints how many pixels pop per frame"]
fn flythrough_pops_are_smoothed_by_the_fade() {
    let Some(h) = Headless::new() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let frames = 700;
    // The initial load pops or fades everything in; judge the flight after it.
    let summary = |p: &[usize]| {
        let p = &p[60..];
        (
            p.iter().copied().max().unwrap_or(0),
            p.iter().sum::<usize>(),
        )
    };
    let (max1, sum1) = summary(&flythrough(&h, true, frames));
    let (max0, sum0) = summary(&flythrough(&h, false, frames));
    eprintln!("with fades:    worst frame {max1} px changed, total {sum1}");
    eprintln!("without fades: worst frame {max0} px changed, total {sum0}");
}
