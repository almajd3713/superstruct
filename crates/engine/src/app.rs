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

use crate::camera::Camera;
use crate::demo::demo_scene;
use crate::gpu::{Gpu, GpuError};
use crate::input::{Action, Bindings, InputState};
use crate::readback;
use crate::renderer::{DebugBox, FrameInput, FrameParams, Renderer};
use crate::settings::Settings;
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
}

struct App {
    title: String,
    settings: Settings,
    running: Option<Running>,
    failure: Option<AppError>,
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
        renderer.set_scene(&gpu.device, &demo_scene());
        Ok(Running {
            window,
            gpu,
            renderer,
        })
    }

    /// Debug free-fly until the walking controller exists in `game`.
    fn fly(&mut self, dt: f32) {
        if !self.cursor_grabbed {
            return;
        }
        let axis = |pos: Action, neg: Action| {
            f32::from(self.input.held(pos)) - f32::from(self.input.held(neg))
        };
        let dir = self.camera.forward_flat() * axis(Action::MoveForward, Action::MoveBack)
            + self.camera.right_flat() * axis(Action::MoveRight, Action::MoveLeft)
            + Vec3::Y * axis(Action::MoveUp, Action::MoveDown);
        let speed = if self.input.held(Action::Sprint) {
            20.0
        } else {
            5.0
        };
        if let Ok(p) = self.camera.pos.offset(dir.normalize_or_zero() * speed * dt) {
            self.camera.pos = p;
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
        let Some(running) = &mut self.running else {
            return;
        };
        if self.input.pressed(Action::ToggleFullscreen) {
            toggle_fullscreen(&running.window);
        }
        if self.input.pressed(Action::ToggleDebugLines) {
            self.debug_boxes = !self.debug_boxes;
        }
        if self.input.pressed(Action::CycleVsync) {
            let next = running.gpu.vsync().next();
            running.gpu.set_vsync(next);
        }
    }

    fn frame(&mut self) {
        if self.running.is_none() {
            return;
        }
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

        self.accumulator += frame_time;
        while self.accumulator >= SIM_DT {
            self.prev_cam_pos = self.camera.pos;
            self.fly(SIM_DT as f32);
            self.accumulator -= SIM_DT;
        }
        let alpha = (self.accumulator / SIM_DT) as f32;
        self.frame_count += 1;
        let autoshot = self.autoshot_after.is_some_and(|n| self.frame_count == n);
        let want_screenshot = autoshot || self.input.pressed(Action::Screenshot);
        self.handle_actions();
        self.input.end_frame();

        let Some(running) = &mut self.running else {
            return;
        };
        let mut render_camera = self.camera;
        let step = self.camera.pos.delta(&self.prev_cam_pos).as_vec3();
        if let Ok(p) = self.prev_cam_pos.offset(step * alpha) {
            render_camera.pos = p;
        }
        self.box_scratch.clear();
        if self.debug_boxes {
            cell_boxes(&render_camera.pos, &mut self.box_scratch);
        }
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
            running.renderer.render(
                &running.gpu.queue,
                &mut encoder,
                &view,
                &FrameInput {
                    camera: &render_camera,
                    size: running.gpu.size(),
                    params: FrameParams::default(),
                    boxes: &self.box_scratch,
                },
            );
            running.gpu.queue.submit([encoder.finish()]);
            if want_screenshot {
                save_screenshot(&running.gpu, &frame.texture);
            }
            running.window.pre_present_notify();
            running.gpu.queue.present(frame);
            self.quit |= autoshot;
        }

        self.fps_frames += 1;
        let elapsed = now.duration_since(self.fps_window_start);
        if elapsed >= Duration::from_secs(1) {
            info!(
                fps = format!("{:.1}", f64::from(self.fps_frames) / elapsed.as_secs_f64()),
                "frame rate"
            );
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
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let start =
        WorldPos::new(I64Vec3::ZERO, Vec3::new(512.0, 513.65, 458.0)).unwrap_or(WorldPos::ORIGIN);
    let mut app = App {
        title: title.to_owned(),
        settings,
        running: None,
        failure: None,
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
