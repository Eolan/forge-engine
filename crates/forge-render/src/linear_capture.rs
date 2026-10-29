//! A frame's scene light as numbers (D-083): the HDR image the display pass reads, divided by the
//! frame's exposure, written as a Radiance HDR file in cd/m² per channel. A path tracer's linear
//! output (Falcor's EXRs) compares with it without a tone curve or eight bits in between.
//!
//! [`LinearCapture::declare`] copies the image into a host-visible buffer in the frame's graph;
//! [`LinearCapture::save`], once the GPU has finished that frame, writes the file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use forge_gpu::{
    BufferAccess, BufferDesc, Device, FrameGraph, GpuError, GraphBuffer, ImageAccess, ImageHandle,
    MemoryCategory, MemoryLocation, Result, vk,
};

/// The most bytes a texel of the captured image takes (four 32-bit floats).
const MOST_TEXEL_BYTES: u64 = 16;

/// One frame's capture of its scene light.
pub struct LinearCapture {
    path: PathBuf,
    buffer: GraphBuffer,
    /// The exposure the frame's image was multiplied by.
    exposure: f32,
    /// The image's format and size, as the copy found them.
    format: AtomicI32,
    width: AtomicU32,
    height: AtomicU32,
}

impl LinearCapture {
    /// A capture to `path` of an image of at most `extent`, drawn at `exposure`.
    pub fn new(
        device: &Arc<Device>,
        path: PathBuf,
        extent: vk::Extent2D,
        exposure: f32,
    ) -> Result<Self> {
        let buffer = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: u64::from(extent.width) * u64::from(extent.height) * MOST_TEXEL_BYTES,
            usage: vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "linear capture",
        })?);
        Ok(Self {
            path,
            buffer,
            exposure,
            format: AtomicI32::new(vk::Format::UNDEFINED.as_raw()),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
        })
    }

    /// Declares the copy of `image` (pre-exposed HDR colour) into the capture's buffer, read by
    /// the host after the frame.
    pub fn declare<'f>(&'f self, graph: &mut FrameGraph<'f>, image: ImageHandle) {
        let handle = graph.import_buffer(&self.buffer);
        graph
            .pass("post/linear capture")
            .image(image, ImageAccess::TransferSrc)
            .buffer(handle, BufferAccess::TransferDst)
            .run(move |resources, commands| {
                let resolved = resources.image(image);
                self.format
                    .store(resolved.format.as_raw(), Ordering::Relaxed);
                self.width.store(resolved.extent.width, Ordering::Relaxed);
                self.height.store(resolved.extent.height, Ordering::Relaxed);
                commands.copy_image_to_buffer(resolved.raw, resolved.extent, &self.buffer);
                Ok(())
            });
        graph
            .pass("post/linear capture")
            .buffer(handle, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// Writes the file: each texel over the exposure, cd/m². Call once the GPU has finished the
    /// frame that declared the copy.
    pub fn save(&self) -> Result<()> {
        let format = vk::Format::from_raw(self.format.load(Ordering::Relaxed));
        let (w, h) = (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        );
        let texel = match format {
            vk::Format::R16G16B16A16_SFLOAT => 8,
            vk::Format::R32G32B32A32_SFLOAT => 16,
            _ => {
                tracing::warn!(?format, "a linear capture of this format is not written");
                return Ok(());
            }
        };
        let mut bytes = vec![0_u8; (w * h * texel) as usize];
        self.buffer.read(0, &mut bytes);
        let scale = 1.0 / self.exposure.max(1e-30);
        let pixels: Vec<image::Rgb<f32>> = bytes
            .chunks_exact(texel as usize)
            .map(|t| {
                let channel = |k: usize| match texel {
                    8 => f16_to_f32(u16::from_le_bytes([t[2 * k], t[2 * k + 1]])),
                    _ => f32::from_le_bytes([t[4 * k], t[4 * k + 1], t[4 * k + 2], t[4 * k + 3]]),
                };
                image::Rgb([0, 1, 2].map(|k| (channel(k) * scale).max(0.0)))
            })
            .collect();
        write_hdr(&self.path, &pixels, w, h)?;
        tracing::info!(path = %self.path.display(), width = w, height = h, "the frame's scene light, cd/m²");
        Ok(())
    }
}

/// A Radiance HDR file of `pixels`, `w` × `h`, rows top to bottom.
fn write_hdr(path: &Path, pixels: &[image::Rgb<f32>], w: u32, h: u32) -> Result<()> {
    let io = |e: String| GpuError::Io(std::io::Error::other(format!("{}: {e}", path.display())));
    let file = std::fs::File::create(path).map_err(|e| io(e.to_string()))?;
    image::codecs::hdr::HdrEncoder::new(std::io::BufWriter::new(file))
        .encode(pixels, w as usize, h as usize)
        .map_err(|e| io(e.to_string()))
}

/// An IEEE half-precision float's value.
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1F);
    let mantissa = f32::from(h & 0x3FF);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_read_as_their_values() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(f16_to_f32(0x7BFF), 65504.0);
        assert!(f16_to_f32(0x7C00).is_infinite());
    }
}
