//! GPU texture readback and PNG output. Synchronous: for screenshots and tests, never the frame loop.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error("unsupported texture format {0:?}")]
    Format(wgpu::TextureFormat),
    #[error("buffer map failed: {0}")]
    Map(#[from] wgpu::BufferAsyncError),
    #[error("mapped range: {0}")]
    MapRange(#[from] wgpu::MapRangeError),
    #[error("device poll: {0}")]
    Poll(#[from] wgpu::PollError),
    #[error("png: {0}")]
    Image(#[from] image::ImageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Tightly packed RGBA8 pixels, row 0 at the top. Handles BGRA and RGBA 8-bit formats.
/// Colour values are the stored (sRGB-encoded for `*Srgb` formats) bytes.
pub fn read_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, ReadbackError> {
    let format = texture.format();
    let swap_rb = match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        other => return Err(ReadbackError::Format(other)),
    };
    let (w, h) = (texture.width(), texture.height());
    let row_bytes = w * 4;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    // The callback has run once the poll returns; a dropped sender would mean map_async never fired.
    rx.recv().map_err(|_| wgpu::BufferAsyncError {})??;

    let mapped = buffer.slice(..).get_mapped_range()?;
    let mut out = Vec::with_capacity((row_bytes * h) as usize);
    for row in mapped.chunks(padded as usize).take(h as usize) {
        out.extend_from_slice(&row[..row_bytes as usize]);
    }
    drop(mapped);
    buffer.unmap();
    if swap_rb {
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    Ok(out)
}

pub fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), ReadbackError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)?;
    Ok(())
}
