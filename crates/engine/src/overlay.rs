//! Debug overlay (F1): frame rate, frame-time graph, camera position, seed and GPU pass timings.

use std::collections::VecDeque;

use glam::{I64Vec3, Vec3};
use winit::event::WindowEvent;
use winit::window::Window;

use crate::debugcmd::Command;
use crate::gpu_timer::{GpuTimer, Zone};
use crate::streamer::StreamStats;

const HISTORY: usize = 240;

/// Per-frame values shown by the overlay.
pub struct OverlayInfo<'a> {
    pub sector: I64Vec3,
    pub local: Vec3,
    pub seed: u64,
    pub adapter: &'a str,
    pub vsync: &'a str,
    pub mode: &'a str,
    pub stream: StreamStats,
    pub gpu_bytes: u64,
    pub shader_error: Option<&'a str>,
    pub timer: &'a GpuTimer,
}

pub struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    pub visible: bool,
    frame_ms: VecDeque<f32>,
    prepared: Option<Prepared>,
    teleport_text: String,
    /// Debug commands the user asked for this frame; the app drains them.
    pub requests: Vec<Command>,
    pub status: String,
}

struct Prepared {
    jobs: Vec<egui::ClippedPrimitive>,
    textures_free: Vec<egui::TextureId>,
    pixels_per_point: f32,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, window: &Window) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Self {
            ctx,
            state,
            renderer,
            visible: false,
            frame_ms: VecDeque::with_capacity(HISTORY),
            prepared: None,
            teleport_text: String::new(),
            requests: Vec::new(),
            status: String::new(),
        }
    }

    /// Returns true if egui wants the event (pointer over a widget, text focus).
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub fn push_frame_time(&mut self, seconds: f32) {
        if self.frame_ms.len() == HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(seconds * 1000.0);
    }

    /// Builds the UI for this frame and uploads its textures and buffers.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &Window,
        size: (u32, u32),
        info: &OverlayInfo<'_>,
    ) {
        self.prepared = None;
        if !self.visible {
            return;
        }
        let input = self.state.take_egui_input(window);
        let frame_ms = &self.frame_ms;
        let (text, requests, status) = (&mut self.teleport_text, &mut self.requests, &self.status);
        let mut output = self.ctx.run_ui(input, |ui| {
            draw(ui, frame_ms, info, (text, requests, status));
        });
        self.state
            .handle_platform_output(window, output.platform_output);
        let pixels_per_point = output.pixels_per_point;
        let jobs = self.ctx.tessellate(output.shapes, pixels_per_point);
        // egui asserts that every delta is consumed before the struct is dropped.
        let mut textures = std::mem::take(&mut output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.renderer.update_texture(device, queue, id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point,
        };
        // User callbacks are not used, so the extra command buffers are always empty.
        self.renderer
            .update_buffers(device, queue, encoder, &jobs, &screen);
        self.prepared = Some(Prepared {
            jobs,
            textures_free: textures.free.drain().collect(),
            pixels_per_point,
        });
    }

    /// Draws over `target`. A no-op when hidden.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        timer: &GpuTimer,
    ) {
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    timestamp_writes: timer.pass_writes(Zone::Ui),
                    ..Default::default()
                })
                .forget_lifetime();
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [size.0, size.1],
                pixels_per_point: prepared.pixels_per_point,
            };
            self.renderer.render(&mut pass, &prepared.jobs, &screen);
        }
        for id in &prepared.textures_free {
            self.renderer.free_texture(id);
        }
    }
}

fn draw(
    ui: &mut egui::Ui,
    frame_ms: &VecDeque<f32>,
    info: &OverlayInfo<'_>,
    (text, requests, status): (&mut String, &mut Vec<Command>, &str),
) {
    egui::Window::new("debug")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .show(ui.ctx(), |ui| {
            let avg = frame_ms.iter().sum::<f32>() / frame_ms.len().max(1) as f32;
            let worst = frame_ms.iter().copied().fold(0.0_f32, f32::max);
            ui.monospace(format!(
                "{:.0} fps   {avg:.2} ms avg   {worst:.2} ms worst",
                1000.0 / avg.max(0.001)
            ));
            frame_graph(ui, frame_ms);
            ui.monospace(format!("{} | vsync {}", info.adapter, info.vsync));
            ui.monospace(format!("seed {}", info.seed));
            ui.monospace(info.mode);
            debug_commands(ui, text, requests, status);
            stream_lines(ui, &info.stream, info.gpu_bytes);
            ui.monospace(format!(
                "sector {} {} {}",
                info.sector.x, info.sector.y, info.sector.z
            ));
            ui.monospace(format!(
                "local  {:.2} {:.2} {:.2}",
                info.local.x, info.local.y, info.local.z
            ));
            if info.timer.supported() {
                for (zone, ms) in info.timer.timings() {
                    ui.monospace(format!("gpu {:<6} {ms:.3} ms", zone.name()));
                }
            } else {
                ui.monospace("gpu timings unsupported");
            }
            if let Some(err) = info.shader_error {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "shader error (previous pipeline kept):",
                );
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    egui::RichText::new(err).monospace(),
                );
            }
        });
}

/// Bars scaled so 16.7 ms is two thirds of the height; red above 33 ms.
fn frame_graph(ui: &mut egui::Ui, frame_ms: &VecDeque<f32>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(HISTORY as f32, 48.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(80));
    let scale = rect.height() / 25.0;
    let y60 = rect.bottom() - 16.7 * scale;
    painter.hline(
        rect.x_range(),
        y60,
        egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
    );
    for (i, ms) in frame_ms.iter().enumerate() {
        let x = rect.left() + i as f32;
        let h = (ms * scale).min(rect.height());
        let colour = if *ms > 33.0 {
            egui::Color32::LIGHT_RED
        } else if *ms > 16.8 {
            egui::Color32::YELLOW
        } else {
            egui::Color32::LIGHT_GREEN
        };
        painter.vline(
            x,
            (rect.bottom() - h)..=rect.bottom(),
            egui::Stroke::new(1.0, colour),
        );
    }
}

fn stream_lines(ui: &mut egui::Ui, s: &StreamStats, gpu_bytes: u64) {
    let mib = |b: u64| b as f64 / f64::from(1u32 << 20);
    ui.monospace(format!(
        "chunks {} resident ({} drawn) / {} wanted",
        s.resident, s.visible, s.wanted
    ));
    ui.monospace(format!(
        "queue {} + {} running, {} ready, {} cancelled",
        s.pool.queued, s.pool.running, s.ready, s.pool.cancelled
    ));
    ui.monospace(format!(
        "gpu mem {:.1} MiB | uploads {} ({:.2} MiB) this frame",
        mib(gpu_bytes),
        s.uploads_this_frame,
        mib(s.upload_bytes_this_frame)
    ));
    ui.monospace(format!(
        "evicted {} | rejected {} | over budget {} | l1 checks {}",
        s.evictions, s.rejected, s.over_budget_frames, s.consistency_checks
    ));
}

/// Teleport box and buttons. Needs the cursor released (Esc) to click or type; F5 and F6 work
/// without it.
fn debug_commands(ui: &mut egui::Ui, text: &mut String, requests: &mut Vec<Command>, status: &str) {
    ui.separator();
    let field = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text("sx sy sz [lx ly lz]")
            .desired_width(220.0),
    );
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.horizontal(|ui| {
        if ui.button("go").clicked() || enter {
            requests.push(Command::Teleport(text.clone()));
        }
        if ui.button("port (F5)").clicked() {
            requests.push(Command::NearestPort);
        }
        if ui.button("sanctuary (F6)").clicked() {
            requests.push(Command::Sanctuary { pick: u64::MAX });
        }
    });
    if !status.is_empty() {
        ui.monospace(status);
    }
}
