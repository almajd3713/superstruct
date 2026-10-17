//! The winit event loop: window and GPU lifetime, fixed-step simulation, variable-rate rendering.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::gpu::{Gpu, GpuError};
use crate::settings::Settings;
use crate::testobj::{TestObject, TestPass};
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
    test_pass: TestPass,
}

struct App {
    title: String,
    settings: Settings,
    running: Option<Running>,
    failure: Option<AppError>,
    test_object: TestObject,
    last_frame: Instant,
    accumulator: f64,
    fps_window_start: Instant,
    fps_frames: u32,
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
        let test_pass = TestPass::new(&gpu);
        Ok(Running {
            window,
            gpu,
            test_pass,
        })
    }

    fn frame(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let now = Instant::now();
        let frame_time = now
            .duration_since(self.last_frame)
            .as_secs_f64()
            .min(MAX_FRAME_TIME);
        self.last_frame = now;

        self.accumulator += frame_time;
        while self.accumulator >= SIM_DT {
            self.test_object.step(SIM_DT as f32);
            self.accumulator -= SIM_DT;
        }
        let alpha = (self.accumulator / SIM_DT) as f32;

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
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.02,
                                g: 0.025,
                                b: 0.03,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                running.test_pass.draw(
                    &running.gpu,
                    &mut pass,
                    self.test_object.interpolated(alpha),
                );
            }
            running.gpu.queue.submit([encoder.finish()]);
            running.window.pre_present_notify();
            running.gpu.queue.present(frame);
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
            WindowEvent::Resized(size) => running.gpu.resize(size.width, size.height),
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => match code {
                KeyCode::F11 => toggle_fullscreen(&running.window),
                KeyCode::F2 => {
                    let next = running.gpu.vsync().next();
                    running.gpu.set_vsync(next);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// Opens the window and blocks until it is closed.
pub fn run(title: &str, settings: Settings) -> Result<(), AppError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let mut app = App {
        title: title.to_owned(),
        settings,
        running: None,
        failure: None,
        test_object: TestObject::default(),
        last_frame: now,
        accumulator: 0.0,
        fps_window_start: now,
        fps_frames: 0,
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), Err)
}

/// `settings.toml` next to the working directory unless `SUPERSTRUCT_SETTINGS` points elsewhere.
pub fn settings_path() -> PathBuf {
    std::env::var_os("SUPERSTRUCT_SETTINGS")
        .map_or_else(|| PathBuf::from("settings.toml"), PathBuf::from)
}
