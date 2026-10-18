//! The winit event loop: window and GPU lifetime, fixed-step simulation, variable-rate rendering.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use glam::{I64Vec3, Vec3};
use worldgen::cell::{CellKey, Level};
use worldgen::coord::WorldPos;
use worldgen::seed::WorldSeed;

use crate::camera::Camera;
use crate::cellscene::busiest_l2_cell;
use crate::controller::{Controller, EYE_HEIGHT, MoveInput};
use crate::debugcmd::Command;
use crate::gpu::{Gpu, GpuError};
use crate::gpu_timer::GpuTimer;
use crate::input::{Action, Bindings, InputState};
use crate::liftdraw::LiftDraw;
use crate::liftplan::plan_cell;
use crate::liftsys::{LiftInput, LiftSystem, PlanResult, PlayerView};
use crate::meshpool::MeshPool;
use crate::overlay::{Overlay, OverlayInfo};
use crate::readback;
use crate::renderer::{DebugBox, FrameInput, FrameParams, Renderer};
use crate::respawn::{Event, SafeSpot, Survival};
use crate::settings::{Settings, VsyncMode};
use crate::shaders::{ShaderSources, ShaderWatcher};
use crate::streamer::{RendererSink, Streamer, StreamerConfig};
use crate::tramauto::TramAuto;
use crate::tramdraw::TramDraw;
use crate::tramsys::{Riding, TramSystem};
use crate::window::toggle_fullscreen;

/// Simulation rate (PLAN.md §11).
pub const SIM_HZ: f64 = 120.0;
const SIM_DT: f64 = 1.0 / SIM_HZ;
/// After a stall (window drag, breakpoint) the simulation drops time instead of spiralling.
const MAX_FRAME_TIME: f64 = 0.25;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("event loop: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("gpu: {0}")]
    Gpu(#[from] GpuError),
}

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    overlay: Overlay,
    timer: GpuTimer,
    shader_watcher: ShaderWatcher,
}

#[allow(clippy::struct_excessive_bools)] // independent debug and input flags
struct App {
    title: String,
    settings: Settings,
    running: Option<Running>,
    failure: Option<AppError>,
    streamer: Streamer,
    walker: Controller,
    survival: Survival,
    lifts: LiftSystem,
    lift_draw: LiftDraw,
    trams: TramSystem,
    tram_draw: TramDraw,
    /// `SUPERSTRUCT_TRAM=auto`: a scripted ride for hitch captures.
    tram_auto: Option<TramAuto>,
    lift_results_tx: crossbeam_channel::Sender<PlanResult>,
    lift_results_rx: crossbeam_channel::Receiver<PlanResult>,
    /// Presses seen since the last fixed step, consumed by the first step of a frame.
    lift_input: LiftInput,
    debug_tx: crossbeam_channel::Sender<crate::debugcmd::Outcome>,
    debug_rx: crossbeam_channel::Receiver<crate::debugcmd::Outcome>,
    sanctuary_pick: u64,
    fly_mode: bool,
    /// Noclip speed in m/s, 1 to 1000.
    fly_speed: f32,
    debug_boxes: bool,
    box_scratch: Vec<DebugBox>,
    input: InputState,
    camera: Camera,
    /// Camera position one sim step ago, for interpolation.
    prev_cam_pos: WorldPos,
    cursor_grabbed: bool,
    last_frame: Instant,
    accumulator: f64,
    fps_window_start: Instant,
    fps_frames: u32,
    /// Longest frame in the current one-second window, seconds.
    worst_frame: f64,
    /// `SUPERSTRUCT_HITCH=<ms>`: log a breakdown of every frame that takes longer.
    hitch_ms: Option<f64>,
    /// Milliseconds spent this frame in the simulation, streamer update and the rest of draw.
    parts_ms: [f64; 3],
    /// `SUPERSTRUCT_AUTOFLY=<m/s>`: fly level along the view at a constant speed, for captures.
    autofly: Option<f32>,
    frame_count: u64,
    quit: bool,
    /// `SUPERSTRUCT_AUTOSHOT=<n>`: screenshot after n frames, then exit. For remote runs.
    autoshot_after: Option<u64>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, AppError> {
        let s = &self.settings;
        let mut attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(PhysicalSize::new(s.width, s.height));
        if s.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(winit::error::EventLoopError::from)?,
        );
        let gpu = Gpu::new(&window, s.vsync)?;
        let mut renderer = Renderer::new(&gpu.device, gpu.config.format, gpu.size());
        renderer.set_volumetric_fog(true);
        renderer.set_ao(crate::ao::AoMode::Analytic);
        if let Ok(list) = std::env::var("SUPERSTRUCT_POST") {
            renderer.set_post(crate::post::PostSettings::parse(&list));
        }
        renderer.set_world_seed(self.settings.seed);
        renderer.set_horizon_enabled(self.settings.horizon);
        renderer.set_horizon_half_res(self.settings.horizon_half_res);
        self.place_camera();
        if !self.fly_mode {
            self.enter_walk_mode();
        }
        if cfg!(debug_assertions) {
            // Errors are logged and kept for the overlay; the embedded pipelines stay in use.
            let _ = renderer.reload_shaders(&gpu.device, &ShaderSources::load());
        }
        let overlay = Overlay::new(&gpu.device, gpu.config.format.remove_srgb_suffix(), &window);
        let mut overlay = overlay;
        overlay.visible = std::env::var_os("SUPERSTRUCT_OVERLAY").is_some();
        let timer = GpuTimer::new(
            &gpu.device,
            &gpu.queue,
            gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY),
        );
        Ok(Running {
            window,
            gpu,
            renderer,
            overlay,
            timer,
            shader_watcher: ShaderWatcher::new(),
        })
    }

    /// Starts walking from the camera: respawn falls back to here until safe ground is found.
    fn enter_walk_mode(&mut self) {
        self.walker.set_eye(self.camera.pos);
        self.survival = Survival::new(self.walker.pos);
        self.survival.set_fallback(SafeSpot {
            pos: self.walker.pos,
            yaw: self.camera.yaw,
        });
    }

    /// Moves the player by one fixed step: noclip fly, or the walking controller.
    fn move_player(&mut self, dt: f32) {
        // Without the cursor grabbed there is no input, but gravity still applies when walking.
        let grabbed = self.cursor_grabbed;
        let axis = |pos: Action, neg: Action| {
            f32::from(grabbed && self.input.held(pos)) - f32::from(grabbed && self.input.held(neg))
        };
        let flat = self.camera.forward_flat() * axis(Action::MoveForward, Action::MoveBack)
            + self.camera.right_flat() * axis(Action::MoveRight, Action::MoveLeft);
        if self.fly_mode
            && let Some(speed) = self.autofly
        {
            let step = self.camera.forward_flat() * speed * dt;
            if let Ok(p) = self.camera.pos.offset(step) {
                self.camera.pos = p;
            }
        } else if self.fly_mode {
            let dir = flat + Vec3::Y * axis(Action::MoveUp, Action::MoveDown);
            let boost = if grabbed && self.input.held(Action::Sprint) {
                4.0
            } else {
                1.0
            };
            let speed = self.fly_speed * boost;
            if let Ok(p) = self.camera.pos.offset(dir.normalize_or_zero() * speed * dt) {
                self.camera.pos = p;
            }
        } else {
            let steer = self.survival.controllable();
            let mut flat = flat;
            if let Some(auto) = &mut self.tram_auto {
                let cmd = auto.step(&self.trams);
                if let Some(feet) = cmd.teleport
                    && let Ok(eye) = feet.offset(Vec3::Y * (EYE_HEIGHT + 0.05))
                {
                    self.walker.set_eye(eye);
                    self.camera.pos = eye;
                    self.prev_cam_pos = eye;
                    self.survival = Survival::new(self.walker.pos);
                    self.lifts.drop_all();
                    self.trams.drop_all();
                }
                flat = cmd.wish;
                self.quit |= cmd.finished;
            }
            let input = MoveInput {
                wish: if steer || self.tram_auto.is_some() {
                    flat.normalize_or_zero()
                } else {
                    Vec3::ZERO
                },
                run: steer && grabbed && self.input.held(Action::Sprint),
            };
            self.step_lifts(dt);
            let aboard = self.trams.aboard();
            let world = Riding {
                world: self.streamer.collision(),
                force_ready: aboard,
            };
            if aboard || !self.lifts.holds_controller(&self.walker.pos) {
                self.walker.step(dt, input, &world);
            }
            if let Event::Respawn(spot) =
                self.survival
                    .update(dt, &self.walker, self.camera.yaw, self.streamer.collision())
                && let Ok(eye) = spot.pos.offset(Vec3::Y * (EYE_HEIGHT + 0.05))
            {
                self.walker.set_eye(eye);
                self.lifts.drop_all();
                self.trams.drop_all();
                self.camera.yaw = spot.yaw;
                self.camera.pitch = 0.0;
                self.prev_cam_pos = eye;
            }
            self.camera.pos = self.walker.eye();
        }
    }

    /// Runs the lift system for one fixed step: takes finished plans, sends new requests to
    /// worker threads and gives the world its gates and cars.
    fn step_lifts(&mut self, dt: f32) {
        while let Ok(result) = self.lift_results_rx.try_recv() {
            self.lifts.apply(result);
        }
        let view = PlayerView {
            feet: self.walker.pos,
            on_platform: self.walker.on_platform(),
            controllable: self.survival.controllable(),
        };
        let input = std::mem::take(&mut self.lift_input);
        self.trams.update(&mut self.walker, input.call);
        self.streamer.set_probes(self.trams.probes());
        let requests = self
            .lifts
            .update(dt, &view, input, self.streamer.collision());
        let seed = WorldSeed(self.settings.seed);
        for request in requests {
            let tx = self.lift_results_tx.clone();
            std::thread::spawn(move || {
                crate::zone!("lift plan");
                for cell_y in request.cells {
                    let key = CellKey::new(
                        Level::L1,
                        I64Vec3::new(request.column.0, cell_y, request.column.1),
                    );
                    let _ = tx.send(PlanResult {
                        column: request.column,
                        generation: request.generation,
                        cell_y,
                        plan: plan_cell(seed, key),
                    });
                }
            });
        }
        let mut solids = self.lifts.solids().to_vec();
        solids.extend_from_slice(self.trams.solids());
        self.streamer.collision_mut().set_dynamic(solids);
    }

    /// Starts requested debug commands (overlay buttons and F5/F6) and applies finished ones.
    fn run_debug_commands(&mut self) {
        let mut cmds = Vec::new();
        if self.input.pressed(Action::TeleportPort) {
            cmds.push(Command::NearestPort);
        }
        if self.input.pressed(Action::TeleportSanctuary) {
            cmds.push(Command::Sanctuary { pick: u64::MAX });
        }
        if let Some(running) = &mut self.running {
            cmds.append(&mut running.overlay.requests);
            if let Some(bands) = running.overlay.band_request.take() {
                self.streamer.set_stream_config(bands);
                self.settings.set_stream_config(&bands);
            }
            if std::mem::take(&mut running.overlay.band_commit)
                && let Err(err) = self.settings.save(&settings_path())
            {
                tracing::warn!("could not save band radii: {err}");
            }
        }
        for cmd in cmds {
            let cmd = match cmd {
                Command::Sanctuary { .. } => {
                    self.sanctuary_pick += 1;
                    Command::Sanctuary {
                        pick: self.sanctuary_pick,
                    }
                }
                other => other,
            };
            let from = self.camera.pos;
            crate::debugcmd::spawn(
                WorldSeed(self.settings.seed),
                cmd,
                from,
                self.debug_tx.clone(),
            );
        }
        while let Ok(outcome) = self.debug_rx.try_recv() {
            let status = match outcome {
                Ok(feet) => match feet.offset(Vec3::Y * (crate::controller::EYE_HEIGHT + 0.05)) {
                    Ok(eye) => {
                        self.camera.pos = eye;
                        self.prev_cam_pos = eye;
                        self.walker.set_eye(eye);
                        self.lifts.drop_all();
                        self.trams.drop_all();
                        self.trams.drop_all();
                        format!("teleported to sector {:?}", feet.sector().to_array())
                    }
                    Err(e) => e.to_string(),
                },
                Err(e) => e,
            };
            if let Some(running) = &mut self.running {
                running.overlay.status = status;
            }
        }
    }

    fn set_cursor_grab(&mut self, grab: bool) {
        let Some(running) = &self.running else { return };
        let window = &running.window;
        let result = if grab {
            window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            window.set_cursor_grab(CursorGrabMode::None)
        };
        match result {
            Ok(()) => {
                window.set_cursor_visible(!grab);
                self.cursor_grabbed = grab;
            }
            Err(err) => error!("cursor grab: {err}"),
        }
    }

    fn handle_actions(&mut self) {
        if self.input.pressed(Action::ReleaseCursor) && self.cursor_grabbed {
            self.set_cursor_grab(false);
        }
        if self.input.pressed(Action::ToggleFly) {
            self.fly_mode = !self.fly_mode;
            if !self.fly_mode {
                self.enter_walk_mode();
            }
        }
        if self.input.pressed(Action::FlyFaster) {
            self.fly_speed = (self.fly_speed * 2.0).min(1000.0);
        }
        if self.input.pressed(Action::FlySlower) {
            self.fly_speed = (self.fly_speed / 2.0).max(1.0);
        }
        let Some(running) = &mut self.running else {
            return;
        };
        if self.input.pressed(Action::ToggleFullscreen) {
            toggle_fullscreen(&running.window);
        }
        if self.input.pressed(Action::ToggleOverlay) {
            running.overlay.visible = !running.overlay.visible;
        }
        if self.input.pressed(Action::FreezeCulling) {
            let frozen = running.renderer.culling_frozen();
            running.renderer.set_freeze_culling(!frozen);
        }
        if self.input.pressed(Action::ToggleDebugLines) {
            self.debug_boxes = !self.debug_boxes;
        }
        let mut post = running.renderer.post();
        post.posterize ^= self.input.pressed(Action::TogglePosterize);
        post.dither ^= self.input.pressed(Action::ToggleDither);
        post.outlines ^= self.input.pressed(Action::ToggleOutlines);
        post.grain ^= self.input.pressed(Action::ToggleGrain);
        running.renderer.set_post(post);
        if self.input.pressed(Action::CycleVsync) {
            let next = running.gpu.vsync().next();
            running.gpu.set_vsync(next);
        }
    }

    /// Start position: `SUPERSTRUCT_SECTOR=x,y,z` teleports to that sector (at `SUPERSTRUCT_LOCAL=x,y,z`, default its middle); otherwise
    /// outside the low corner of `SUPERSTRUCT_CELL` (default: the busiest L2 cell near the origin),
    /// high up and looking at its centre.
    fn place_camera(&mut self) {
        if let Some([x, y, z]) = std::env::var("SUPERSTRUCT_SECTOR")
            .ok()
            .and_then(|v| parse_triple(&v))
        {
            let local = std::env::var("SUPERSTRUCT_LOCAL")
                .ok()
                .and_then(|v| parse_floats(&v))
                .unwrap_or(Vec3::splat(512.0));
            match WorldPos::new(I64Vec3::new(x, y, z), local) {
                Ok(p) => {
                    self.camera.pos = p;
                    self.prev_cam_pos = p;
                    // `SUPERSTRUCT_YAW` / `SUPERSTRUCT_PITCH` in degrees: yaw 0 looks along -z,
                    // 90 along -x, 180 along +z.
                    let angle = |name: &str| {
                        std::env::var(name)
                            .ok()
                            .and_then(|v| v.trim().parse::<f32>().ok())
                    };
                    if let Some(deg) = angle("SUPERSTRUCT_YAW") {
                        self.camera.yaw = deg.to_radians();
                    }
                    if let Some(deg) = angle("SUPERSTRUCT_PITCH") {
                        self.camera.pitch = deg.to_radians();
                    }
                }
                Err(err) => error!("cannot teleport to sector {x} {y} {z}: {err}"),
            }
            return;
        }
        let key = std::env::var("SUPERSTRUCT_CELL")
            .ok()
            .and_then(|v| parse_triple(&v))
            .map_or_else(
                || busiest_l2_cell(WorldSeed(self.settings.seed), 4),
                |[x, y, z]| CellKey::new(Level::L2, I64Vec3::new(x, y, z)),
            );
        match key
            .aabb_world()
            .and_then(|b| b.min.offset(Vec3::new(-120.0, 420.0, -120.0)))
        {
            Ok(p) => {
                self.camera.pos = p;
                self.prev_cam_pos = p;
                self.camera.yaw = 5.0 * std::f32::consts::FRAC_PI_4;
                self.camera.pitch = -0.4;
            }
            Err(err) => error!("cannot place the camera at cell {key:?}: {err}"),
        }
    }

    #[allow(clippy::too_many_lines)] // the frame's orchestration reads best in one place
    fn draw(&mut self, alpha: f32, frame_time: f64, want_screenshot: bool) -> bool {
        let Some(running) = &mut self.running else {
            return false;
        };
        let mut presented = false;
        if running.shader_watcher.changed() {
            let _ = running
                .renderer
                .reload_shaders(&running.gpu.device, &ShaderSources::load());
        }
        let mut render_camera = self.camera;
        let step = self.camera.pos.delta(&self.prev_cam_pos).as_vec3();
        if let Ok(p) = self.prev_cam_pos.offset(step * alpha) {
            render_camera.pos = p;
        }
        let t_stream = Instant::now();
        self.streamer.update(
            &render_camera.pos,
            render_camera.forward(),
            &mut RendererSink {
                renderer: &mut running.renderer,
                device: &running.gpu.device,
                queue: &running.gpu.queue,
            },
        );
        self.parts_ms[1] = t_stream.elapsed().as_secs_f64() * 1000.0;
        let t_draws = Instant::now();
        self.lift_draw.update(
            &mut running.renderer,
            &running.gpu.device,
            &running.gpu.queue,
            &self.lifts,
        );
        self.tram_draw.update(
            &mut running.renderer,
            &running.gpu.device,
            &running.gpu.queue,
            &self.trams,
            &render_camera.pos,
        );
        self.parts_ms[2] = t_draws.elapsed().as_secs_f64() * 1000.0;
        self.box_scratch.clear();
        if self.debug_boxes {
            cell_boxes(&render_camera.pos, &mut self.box_scratch);
        }
        running.overlay.push_frame_time(frame_time as f32);
        running.timer.begin_frame(&running.gpu.device);
        if let Some(frame) = running.gpu.acquire() {
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder =
                running
                    .gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("frame"),
                    });
            crate::zone!("record scene");
            running
                .renderer
                .set_horizon_start(self.streamer.config().stream.l1_far_radius_m as f32);
            {
                // The regime around the camera lights the interiors it is in; a Blackout has
                // no emissive colour, so every lamp and window goes dark.
                let atm = worldgen::atmosphere::atmosphere_at(
                    WorldSeed(self.settings.seed),
                    &render_camera.pos,
                );
                let e = atm.emissive;
                running
                    .renderer
                    .set_module_glow(Some([e.x, e.y, e.z, f32::from(atm.light > 0.0)]));
            }
            running.renderer.render(
                &running.gpu.queue,
                &mut encoder,
                &view,
                &FrameInput {
                    camera: &render_camera,
                    size: running.gpu.size(),
                    params: frame_params(
                        WorldSeed(self.settings.seed),
                        &render_camera.pos,
                        self.survival.fade(),
                    ),
                    boxes: &self.box_scratch,
                    timer: Some(&running.timer),
                    dt: frame_time as f32,
                },
            );
            let mode = mode_line(
                self.fly_mode,
                self.fly_speed,
                &self.walker,
                self.survival.fade(),
            );
            let info = OverlayInfo {
                sector: render_camera.pos.sector(),
                local: render_camera.pos.local(),
                seed: self.settings.seed,
                adapter: &running.gpu.adapter_info.name,
                vsync: match running.gpu.vsync() {
                    VsyncMode::On => "on",
                    VsyncMode::Mailbox => "mailbox",
                    VsyncMode::Off => "off",
                },
                mode: &mode,
                stream: self.streamer.stats(),
                gpu_bytes: running.renderer.gpu_bytes(),
                shader_error: running.renderer.shader_error(),
                timer: &running.timer,
                bands: self.streamer.config().stream,
            };
            crate::zone!("overlay prepare");
            running.overlay.prepare(
                &running.gpu.device,
                &running.gpu.queue,
                &mut encoder,
                &running.window,
                running.gpu.size(),
                &info,
            );
            let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(running.gpu.config.format.remove_srgb_suffix()),
                ..Default::default()
            });
            running
                .overlay
                .render(&mut encoder, &ui_view, running.gpu.size(), &running.timer);
            running.timer.resolve(&mut encoder);
            crate::zone!("submit");
            running.gpu.queue.submit([encoder.finish()]);
            running.timer.after_submit();
            if want_screenshot {
                save_screenshot(&running.gpu, &frame.texture);
            }
            running.window.pre_present_notify();
            crate::zone!("present");
            running.gpu.queue.present(frame);
            presented = true;
        }
        presented
    }

    fn frame(&mut self) {
        if self.running.is_none() {
            return;
        }
        crate::zone!("frame");
        let now = Instant::now();
        let frame_time = now
            .duration_since(self.last_frame)
            .as_secs_f64()
            .min(MAX_FRAME_TIME);
        self.last_frame = now;

        if self.cursor_grabbed {
            let (dx, dy) = self.input.mouse_delta();
            let k = self.settings.mouse_sensitivity;
            self.camera.look(-(dx as f32) * k, -(dy as f32) * k);
        }

        if self.cursor_grabbed {
            self.lift_input.call |= self.input.pressed(Action::Interact);
            self.lift_input.up |= self.input.pressed(Action::MoveUp);
            self.lift_input.down |= self.input.pressed(Action::MoveDown);
        }
        let t_sim = Instant::now();
        self.accumulator += frame_time;
        crate::zone!("simulate");
        while self.accumulator >= SIM_DT {
            self.prev_cam_pos = self.camera.pos;
            self.move_player(SIM_DT as f32);
            self.accumulator -= SIM_DT;
        }
        self.parts_ms[0] = t_sim.elapsed().as_secs_f64() * 1000.0;
        let alpha = (self.accumulator / SIM_DT) as f32;
        self.frame_count += 1;
        let autoshot = self.autoshot_after.is_some_and(|n| self.frame_count == n);
        let want_screenshot = autoshot || self.input.pressed(Action::Screenshot);
        self.run_debug_commands();
        self.handle_actions();
        self.input.end_frame();

        let presented = {
            crate::zone!("draw");
            self.draw(alpha, frame_time, want_screenshot)
        };
        self.quit |= presented && autoshot;
        let total_ms = now.elapsed().as_secs_f64() * 1000.0;
        if self.hitch_ms.is_some_and(|limit| total_ms > limit) {
            info!(
                total = format!("{total_ms:.1}"),
                sim = format!("{:.1}", self.parts_ms[0]),
                streamer = format!("{:.1}", self.parts_ms[1]),
                draws = format!("{:.1}", self.parts_ms[2]),
                retarget = ?self.streamer.retarget_ms().map(|v| (v * 10.0).round() / 10.0),
                parts = ?self.streamer.last_parts_ms().map(|v| (v * 10.0).round() / 10.0),
                "slow frame"
            );
        }

        crate::profile::frame_mark();
        self.fps_frames += 1;
        self.worst_frame = self.worst_frame.max(frame_time);
        let elapsed = now.duration_since(self.fps_window_start);
        if elapsed >= Duration::from_secs(1) {
            info!(
                fps = format!("{:.1}", f64::from(self.fps_frames) / elapsed.as_secs_f64()),
                worst_ms = format!("{:.1}", self.worst_frame * 1000.0),
                "frame rate"
            );
            self.worst_frame = 0.0;
            self.fps_window_start = now;
            self.fps_frames = 0;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(running) => {
                self.running = Some(running);
                self.last_frame = Instant::now();
            }
            Err(err) => {
                error!("startup failed: {err}");
                self.failure = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };
        if !self.cursor_grabbed && running.overlay.on_window_event(&running.window, &event) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                running.gpu.resize(size.width, size.height);
                running
                    .renderer
                    .resize(&running.gpu.device, running.gpu.size());
            }
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        ..
                    },
                ..
            } => self.input.on_key(code, state, repeat),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.cursor_grabbed => {
                self.set_cursor_grab(true);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                self.input.on_mouse_button(button, state);
            }
            WindowEvent::Focused(false) => {
                self.input.release_all();
                if self.cursor_grabbed {
                    self.set_cursor_grab(false);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            self.input.on_mouse_motion(dx, dy);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// Opens the window and blocks until it is closed.
pub fn run(title: &str, settings: Settings) -> Result<(), AppError> {
    let settings_fov = settings.fov_deg;
    let settings_seed = settings.seed;
    let settings_bands = settings.stream_config();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let start =
        WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 513.65, 458.0)).unwrap_or(WorldPos::ORIGIN);
    let (debug_tx, debug_rx) = crossbeam_channel::unbounded();
    let (lift_results_tx, lift_results_rx) = crossbeam_channel::unbounded();
    let mut app = App {
        title: title.to_owned(),
        settings,
        running: None,
        failure: None,
        walker: Controller::new(start),
        survival: Survival::new(start),
        lifts: LiftSystem::new(WorldSeed(settings_seed)),
        lift_draw: LiftDraw::default(),
        trams: TramSystem::new(WorldSeed(settings_seed)),
        tram_draw: TramDraw::default(),
        tram_auto: (std::env::var("SUPERSTRUCT_TRAM").as_deref() == Ok("auto"))
            .then(|| TramAuto::new(WorldSeed(settings_seed))),
        lift_results_tx,
        lift_results_rx,
        lift_input: LiftInput::default(),
        debug_tx,
        debug_rx,
        sanctuary_pick: 0,
        fly_mode: std::env::var_os("SUPERSTRUCT_WALK").is_none()
            && std::env::var_os("SUPERSTRUCT_TRAM").is_none(),
        fly_speed: 20.0,
        streamer: Streamer::new(
            StreamerConfig {
                stream: settings_bands,
                ..StreamerConfig::default()
            },
            MeshPool::with_default_threads(WorldSeed(settings_seed)),
        ),
        debug_boxes: std::env::var_os("SUPERSTRUCT_DEBUG_BOXES").is_some(),
        box_scratch: Vec::new(),
        input: InputState::new(Bindings::default()),
        camera: Camera {
            yaw: std::f32::consts::PI,
            ..Camera::new(start, settings_fov)
        },
        prev_cam_pos: start,
        cursor_grabbed: false,
        last_frame: now,
        accumulator: 0.0,
        fps_window_start: now,
        fps_frames: 0,
        worst_frame: 0.0,
        hitch_ms: std::env::var("SUPERSTRUCT_HITCH")
            .ok()
            .and_then(|v| v.parse().ok()),
        parts_ms: [0.0; 3],
        autofly: std::env::var("SUPERSTRUCT_AUTOFLY")
            .ok()
            .and_then(|v| v.parse().ok()),
        frame_count: 0,
        quit: false,
        autoshot_after: std::env::var("SUPERSTRUCT_AUTOSHOT")
            .ok()
            .and_then(|v| v.parse().ok()),
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), Err)
}

/// `settings.toml` next to the working directory unless `SUPERSTRUCT_SETTINGS` points elsewhere.
pub fn settings_path() -> PathBuf {
    std::env::var_os("SUPERSTRUCT_SETTINGS")
        .map_or_else(|| PathBuf::from("settings.toml"), PathBuf::from)
}

/// L2 and L1 cell outlines around `pos`, for the debug-line toggle.
fn cell_boxes(pos: &WorldPos, out: &mut Vec<DebugBox>) {
    for (level, colour) in [
        (Level::L2, [0.2, 0.6, 1.0, 1.0]),
        (Level::L1, [1.0, 0.55, 0.1, 1.0]),
    ] {
        let centre = CellKey::containing(level, pos);
        for d in neighbourhood() {
            let key = CellKey::new(level, centre.coords + d);
            if let Ok(b) = key.aabb_world() {
                out.push(DebugBox {
                    min: b.min,
                    size_m: b.size_m as f32,
                    colour,
                });
            }
        }
    }
}

fn neighbourhood() -> impl Iterator<Item = I64Vec3> {
    (-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| I64Vec3::new(x, y, z))))
}

fn save_screenshot(gpu: &Gpu, texture: &wgpu::Texture) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = PathBuf::from(format!("out/screenshot-{stamp}.png"));
    let result = readback::read_texture_rgba8(&gpu.device, &gpu.queue, texture)
        .and_then(|px| readback::save_png(&path, texture.width(), texture.height(), &px));
    match result {
        Ok(()) => info!(path = %path.display(), "screenshot saved"),
        Err(err) => error!("screenshot failed: {err}"),
    }
}

/// `x,y,z` as integers.
fn parse_triple(text: &str) -> Option<[i64; 3]> {
    let mut it = text.split(',').map(|v| v.trim().parse::<i64>());
    let v = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    it.next().is_none().then_some(v)
}

fn mode_line(fly: bool, fly_speed: f32, walker: &Controller, fade: f32) -> String {
    if fly {
        format!("fly {fly_speed:.0} m/s (F4 walk, +/- speed)")
    } else {
        let state = if walker.grounded {
            "grounded"
        } else {
            "airborne"
        };
        format!(
            "walk, {state} (F4 fly) vel {:.2} m/s fade {fade:.2}",
            walker.vel.length()
        )
    }
}

/// `x,y,z` as floats.
fn parse_floats(text: &str) -> Option<Vec3> {
    let mut it = text.split(',').map(|v| v.trim().parse::<f32>());
    let v = Vec3::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    it.next().is_none().then_some(v)
}

/// Fog, sky and light for the regime around the camera.
fn frame_params(seed: WorldSeed, cam: &WorldPos, fade: f32) -> FrameParams {
    let atm = worldgen::atmosphere::atmosphere_at(seed, cam);
    FrameParams {
        // Koschmieder's extinction scaled by the style's haze: Pipeworks close, Hollow airy.
        fog_density: atm.sigma * atm.haze,
        // Fog thins by 1/e every 1/k metres above the camera.
        fog_height_k: atm.height_k,
        fog_color: atm.fog_color.to_array(),
        ambient: atm.ambient.to_array(),
        light: atm.light,
        fade,
        ..FrameParams::default()
    }
}
