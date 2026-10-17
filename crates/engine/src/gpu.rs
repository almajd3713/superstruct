//! wgpu instance, adapter, device and surface.

use std::sync::Arc;

use tracing::{info, warn};
use winit::window::Window;

use crate::settings::VsyncMode;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("create surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("adapter lacks required features {0:?}; update the GPU driver")]
    MissingFeatures(wgpu::Features),
    #[error("request device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("surface is not supported by the adapter")]
    UnsupportedSurface,
}

/// Needed for the GPU-driven renderer (PLAN.md §11). Plain multi-draw-indirect is core in wgpu.
const REQUIRED: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE;
/// Used when present; each has a fallback.
const OPTIONAL: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    .union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub features: wgpu::Features,
    pub adapter_info: wgpu::AdapterInfo,
    present_modes: Vec<wgpu::PresentMode>,
    vsync: VsyncMode,
}

impl Gpu {
    pub fn new(window: &Arc<Window>, vsync: VsyncMode) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let adapter_info = adapter.get_info();
        info!(
            name = %adapter_info.name,
            backend = ?adapter_info.backend,
            kind = ?adapter_info.device_type,
            driver = %adapter_info.driver,
            "adapter"
        );

        let available = adapter.features();
        let missing = REQUIRED.difference(available);
        if !missing.is_empty() {
            return Err(GpuError::MissingFeatures(missing));
        }
        let features = REQUIRED.union(OPTIONAL.intersection(available));
        let unavailable = OPTIONAL.difference(available);
        if !unavailable.is_empty() {
            warn!(?unavailable, "optional GPU features not available");
        }
        info!(?features, "granted GPU features");

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                required_features: features,
                required_limits: adapter.limits(),
                ..Default::default()
            }))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(GpuError::UnsupportedSurface)?;
        let caps = surface.get_capabilities(&adapter);
        let present_modes = caps.present_modes;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        config.present_mode = pick_present_mode(vsync, &present_modes);
        surface.configure(&device, &config);
        info!(format = ?config.format, mode = ?config.present_mode, "surface");

        Ok(Self {
            device,
            queue,
            surface,
            config,
            features,
            adapter_info,
            present_modes,
            vsync,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn vsync(&self) -> VsyncMode {
        self.vsync
    }

    pub fn set_vsync(&mut self, vsync: VsyncMode) {
        self.vsync = vsync;
        self.config.present_mode = pick_present_mode(vsync, &self.present_modes);
        self.surface.configure(&self.device, &self.config);
        info!(?vsync, mode = ?self.config.present_mode, "present mode");
    }

    /// Acquires the next frame, reconfiguring the surface when it is stale.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture as C;
        match self.surface.get_current_texture() {
            C::Success(t) => Some(t),
            C::Suboptimal(t) => {
                self.surface.configure(&self.device, &self.config);
                Some(t)
            }
            C::Outdated | C::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            C::Timeout | C::Occluded => None,
            C::Validation => {
                warn!("surface texture validation error");
                None
            }
        }
    }
}

/// Falls back to `Fifo`, which every surface supports, when the wanted mode is missing.
pub fn pick_present_mode(vsync: VsyncMode, supported: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    let wanted = match vsync {
        VsyncMode::On => wgpu::PresentMode::Fifo,
        VsyncMode::Mailbox => wgpu::PresentMode::Mailbox,
        VsyncMode::Off => wgpu::PresentMode::Immediate,
    };
    if supported.contains(&wanted) {
        wanted
    } else {
        wgpu::PresentMode::Fifo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::PresentMode as P;

    #[test]
    fn falls_back_to_fifo() {
        assert_eq!(pick_present_mode(VsyncMode::Mailbox, &[P::Fifo]), P::Fifo);
        assert_eq!(
            pick_present_mode(VsyncMode::Off, &[P::Fifo, P::Immediate]),
            P::Immediate
        );
    }
}
