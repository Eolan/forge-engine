//! A device with no window, for the GPU tests: a pass over synthetic inputs, run through the
//! render graph, read back and judged in `cargo test`, in milliseconds, with no demo and no
//! capture. A test creates one with [`Headless::for_test`], which returns `None` unless
//! `FORGE_GPU_TESTS=1`, so a machine without a GPU (CI's) skips the test and passes, and the
//! owner's machine runs it:
//!
//! ```text
//! FORGE_GPU_TESTS=1 cargo test --release -p forge-render
//! ```
//!
//! The validation layer is on when it is installed, and [`Headless::finish`] fails when the
//! layer reported an error during the test's frames: every pass test is a validation test too.
//! `FORGE_GPU_TESTS_NO_VALIDATION=1` leaves the layer off.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::device::{Device, DeviceOptions};
use crate::error::{GpuError, Result};
use crate::frame::{FrameSlot, Frames};
use crate::graph::{
    BufferAccess, FrameGraph, GraphBuffer, GraphImage, GraphStats, ImageAccess, ImageHandle,
    RenderGraph,
};
use crate::instance::{Instance, validation_errors};
use crate::memory::{BufferDesc, ImageDesc};
use crate::memory_report::MemoryCategory;
use crate::shader::ShaderCompiler;

/// The variable that turns the GPU tests on: `FORGE_GPU_TESTS=1`.
pub const TESTS_VARIABLE: &str = "FORGE_GPU_TESTS";

/// The workspace's `shaders/` from a crate's manifest directory (`env!("CARGO_MANIFEST_DIR")`
/// of a crate under `crates/` or `demos/`).
pub fn shaders_of(manifest_dir: &str) -> PathBuf {
    Path::new(manifest_dir).join("../../shaders")
}

/// One headless device at a time in a process: the tests run in threads, and the validation
/// layer's errors are counted for the process, so a test's errors must be its own.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A device, its frames in flight and a render graph, with no window.
pub struct Headless {
    /// Held for the device's life ([`ONE_AT_A_TIME`]); first, so it is dropped last.
    _serial: std::sync::MutexGuard<'static, ()>,
    _instance: Arc<Instance>,
    /// The device.
    pub device: Arc<Device>,
    /// The frames in flight, submitted without a swapchain.
    pub frames: Frames,
    /// The render graph's persistent side (the transient heap, the statistics).
    pub graph: RenderGraph,
    /// The shader compiler over the workspace's `shaders/`.
    pub shaders: ShaderCompiler,
    /// The validation layer's error count when the device was made.
    errors_at_start: usize,
}

impl Headless {
    /// Whether `FORGE_GPU_TESTS` asks for the GPU tests to run.
    pub fn wanted() -> bool {
        std::env::var_os(TESTS_VARIABLE).is_some_and(|v| !v.is_empty() && v != "0")
    }

    /// Creates the instance (the validation layer on when installed, unless
    /// `FORGE_GPU_TESTS_NO_VALIDATION=1`), the device, the frames and the compiler over
    /// `shader_dir` ([`shaders_of`]).
    pub fn new(shader_dir: impl Into<PathBuf>) -> Result<Self> {
        let serial = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let validation = std::env::var_os("FORGE_GPU_TESTS_NO_VALIDATION").is_none_or(|v| v == "0");
        let errors_at_start = validation_errors();
        let instance = Arc::new(Instance::new(c"forge headless", validation, None)?);
        let device = Device::with_options(Arc::clone(&instance), None, DeviceOptions::default())?;
        let frames = Frames::headless(Arc::clone(&device))?;
        let mut graph = RenderGraph::new(&device);
        graph.allow_readback();
        let shaders = ShaderCompiler::new(
            shader_dir,
            forge_core::derived::cache_dir(forge_core::derived::CacheKind::Shaders),
            true,
        )?;
        Ok(Self {
            _serial: serial,
            _instance: instance,
            device,
            frames,
            graph,
            shaders,
            errors_at_start,
        })
    }

    /// For a test: `None` unless [`Headless::wanted`], and the test returns at once, passing.
    /// With the variable set, a machine that cannot make the device fails the test rather than
    /// skip it, so a broken driver or a missing `slangc` is seen.
    pub fn for_test(shader_dir: impl Into<PathBuf>) -> Option<Self> {
        if !Self::wanted() {
            return None;
        }
        match Self::new(shader_dir) {
            Ok(headless) => Some(headless),
            Err(error) => {
                panic!("{TESTS_VARIABLE} is set but no headless device could be made: {error}")
            }
        }
    }

    /// Runs one frame: `declare` puts the passes into a graph over a target of `extent`, with
    /// the frame's slot for the passes that keep a buffer per slot; the graph derives the
    /// barriers, records and submits. Returns what `declare` returned and the graph's
    /// statistics. The GPU may still be running the frame: [`Headless::finish`] waits for it.
    pub fn frame<'f, R>(
        &mut self,
        extent: vk::Extent2D,
        declare: impl FnOnce(&mut FrameGraph<'f>, FrameSlot) -> R,
    ) -> Result<(R, GraphStats)> {
        let slot = self.frames.wait_for_slot()?;
        self.frames.begin(slot)?;
        let mut graph = FrameGraph::new(extent);
        let result = declare(&mut graph, slot);
        let stats = self.graph.execute(graph, &mut self.frames, slot)?;
        self.frames.submit_headless(slot)?;
        Ok((result, stats))
    }

    /// Waits for every frame submitted so far, so the readback buffers can be read and the
    /// frames' resources dropped; fails when the validation layer reported an error since the
    /// device was made, naming the last few (the log has them all).
    pub fn finish(&self) -> Result<()> {
        self.device.wait_idle();
        let errors = validation_errors().saturating_sub(self.errors_at_start);
        if errors > 0 {
            let messages = crate::instance::validation_messages();
            let last: Vec<&str> = messages
                .iter()
                .rev()
                .take(errors)
                .rev()
                .map(String::as_str)
                .collect();
            return Err(GpuError::Unsupported(format!(
                "the validation layer reported {errors} error(s) during the frames:\n  {}",
                last.join("\n  ")
            )));
        }
        Ok(())
    }

    /// A single-level image of `format` holding `data` (tightly packed rows), readable by every
    /// stage: a pass's synthetic input.
    pub fn upload_image(
        &self,
        name: &str,
        width: u32,
        height: u32,
        format: vk::Format,
        data: &[u8],
    ) -> Result<GraphImage> {
        GraphImage::uploaded(
            &self.device,
            ImageDesc {
                width,
                height,
                format,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name,
            },
            data,
        )
    }

    /// A host-readable buffer of `size` bytes, for [`readback_image`] or a pass's own copy; read
    /// it with [`crate::Buffer::read`] after [`Headless::finish`].
    pub fn readback_buffer(&self, name: &str, size: u64) -> Result<GraphBuffer> {
        Ok(GraphBuffer::new(self.device.create_buffer(BufferDesc {
            size: size.max(4),
            usage: vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name,
        })?))
    }
}

/// Declares the passes that copy `image`, a single-level colour image of `extent`, into
/// `buffer` (its texels tightly packed), which the host may read once the frame has completed
/// ([`Headless::finish`]): as a demo's `--capture` does.
pub fn readback_image<'f>(
    graph: &mut FrameGraph<'f>,
    image: ImageHandle,
    extent: vk::Extent2D,
    buffer: &'f GraphBuffer,
) {
    let handle = graph.import_buffer(buffer);
    graph
        .pass("headless/readback")
        .image(image, ImageAccess::TransferSrc)
        .buffer(handle, BufferAccess::TransferDst)
        .run(move |resources, commands| {
            commands.copy_image_to_buffer(resources.image(image).raw, extent, buffer);
            Ok(())
        });
    graph
        .pass("headless/readback")
        .buffer(handle, BufferAccess::HostRead)
        .run(|_, _| Ok(()));
}

/// An IEEE half (a texel of an `R16_SFLOAT` image) as an `f32`, for reading a half-float image
/// back.
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    let magnitude = match exponent {
        0 => mantissa * 2.0_f32.powi(-24),
        31 if mantissa == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + mantissa / 1024.0) * 2.0_f32.powi(e - 15),
    };
    sign * magnitude
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_decode() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x3400), 0.25);
        assert_eq!(f16_to_f32(0x0001), 2.0_f32.powi(-24));
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
        assert_eq!(f16_to_f32(0), 0.0);
    }

    #[test]
    fn the_tests_variable_is_read_once_per_call() {
        // Not set in the gate: the GPU tests skip themselves.
        if std::env::var_os(TESTS_VARIABLE).is_none() {
            assert!(!Headless::wanted());
        }
    }
}
