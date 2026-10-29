//! AMD's FSR through its signed DLLs in the frame (`forge_gpu::ffx`, the FSR SDK 2.4.0,
//! `docs/research/fsr4-vulkan.md`): FSR 4's ML upscaler on the AMD cards that run it, the DLLs'
//! FSR 3.1 wherever else they load. It takes the frame Forge's own FSR 3.1 takes
//! ([`crate::FsrUpscaler`]: the jittered HDR colour, its depth, the motion vectors, the reactive
//! mask where there is one) and draws its output at the same size, so `r.aa=fsr` can hand the
//! frame to either; one graph pass, "temporal/FSR (AMD)", in which the DLLs record theirs.

use std::path::Path;
use std::sync::Arc;

use forge_gpu::{
    Device, Ffx, FfxFrame, FfxImages, FfxPick, FrameGraph, GraphImage, ImageAccess, ImageDesc,
    ImageHandle, Result, vk,
};

use crate::fsr::{FsrMode, render_extent};
use crate::taa::{HDR_FORMAT, TaaFrame};
use crate::upscale::UpscaleCamera;

/// The DLLs' upscaler, its output image and its exposure input.
pub struct FfxUpscaler {
    device: Arc<Device>,
    ffx: Ffx,
    /// Where the DLLs were loaded from and the version asked for, to make the context again for
    /// a larger output.
    dir: std::path::PathBuf,
    pick: FfxPick,
    mode: FsrMode,
    output_extent: vk::Extent2D,
    /// The largest output the context was made for.
    max_output: vk::Extent2D,
    output: GraphImage,
    /// 1 × 1 `R32_SFLOAT` holding 1: the scene is pre-exposed, as for DLSS and FSR 3.1.
    exposure: GraphImage,
    /// The next frame starts a new history (first frame, new mode or size).
    reset: bool,
    /// RCAS after the upscale, 0 to 1, or none (`FsrUpscaler::sharpness`'s).
    pub sharpness: Option<f32>,
}

impl FfxUpscaler {
    /// Loads the DLLs from `dir` (`forge_gpu::FFX_LOADER` and the upscaler beside it) for an
    /// output of `output_extent` in `mode`, with the version `pick` asks for (FSR 4: major 4).
    pub fn new(
        device: &Arc<Device>,
        dir: &Path,
        mode: FsrMode,
        output_extent: vk::Extent2D,
        pick: FfxPick,
    ) -> Result<Self> {
        let render = render_extent(mode, output_extent);
        let ffx = Ffx::load(device, dir, render, output_extent, pick)?;
        let exposure = GraphImage::uploaded(
            device,
            ImageDesc {
                width: 1,
                height: 1,
                format: vk::Format::R32_SFLOAT,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "ffx exposure",
            },
            bytemuck::bytes_of(&1.0_f32),
        )?;
        Ok(Self {
            device: Arc::clone(device),
            ffx,
            dir: dir.to_path_buf(),
            pick,
            mode,
            output_extent,
            max_output: output_extent,
            output: create_output(device, output_extent)?,
            exposure,
            reset: true,
            sharpness: None,
        })
    }

    /// The versions the DLLs offer on this device, and the one running.
    pub fn versions(&self) -> (&[String], &str) {
        self.ffx.versions()
    }

    /// The quality mode.
    pub fn mode(&self) -> FsrMode {
        self.mode
    }

    /// Starts a new history at the next frame.
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// A new mode or output size: the output image again, the context again when the output
    /// outgrew the one it was made for (a new size waits for the device to be idle, as the
    /// swapchain's does).
    pub fn configure(&mut self, mode: FsrMode, output_extent: vk::Extent2D) -> Result<()> {
        if mode == self.mode && output_extent == self.output_extent {
            return Ok(());
        }
        if output_extent.width > self.max_output.width
            || output_extent.height > self.max_output.height
        {
            let render = render_extent(FsrMode::NativeAa, output_extent);
            self.ffx = Ffx::load(&self.device, &self.dir, render, output_extent, self.pick)?;
            self.max_output = output_extent;
        }
        if output_extent != self.output_extent {
            self.output = create_output(&self.device, output_extent)?;
        }
        self.mode = mode;
        self.output_extent = output_extent;
        self.reset = true;
        Ok(())
    }

    /// The size the frame is to be drawn at.
    pub fn render_extent(&self) -> vk::Extent2D {
        render_extent(self.mode, self.output_extent)
    }

    /// Declares the pass "temporal/FSR (AMD)", which upscales `frame.color` (drawn with `depth`,
    /// moved by `motion`, with an optional `reactive` mask) into the HDR output, and returns it.
    /// `exposure` is the frame's pre-exposure, `delta_time` the seconds since the last frame.
    #[allow(clippy::too_many_arguments)]
    pub fn upscale<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        frame: &TaaFrame,
        camera: UpscaleCamera,
        depth: ImageHandle,
        motion: ImageHandle,
        reactive: Option<ImageHandle>,
        exposure: f32,
        delta_time: f32,
    ) -> ImageHandle {
        let reset = std::mem::take(&mut self.reset) || frame.reset;
        let this: &'f Self = self;
        let output = graph.import(&this.output);
        let exposure_image = graph.import(&this.exposure);
        let ffx_frame = FfxFrame {
            jitter: frame.jitter.to_array(),
            render: this.render_extent(),
            output: this.output_extent,
            reset,
            near: camera.near,
            vertical_fov: camera.vertical_fov,
            frame_ms: delta_time * 1000.0,
            pre_exposure: if exposure > 0.0 { exposure } else { 1.0 },
            sharpness: this.sharpness,
        };
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let color = frame.color;
        let pass = graph
            .pass("temporal/FSR (AMD)")
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(motion, ImageAccess::Sampled(compute))
            .image(exposure_image, ImageAccess::Sampled(compute))
            .image(output, ImageAccess::StorageWrite(compute));
        let pass = match reactive {
            Some(mask) => pass.image(mask, ImageAccess::Sampled(compute)),
            None => pass,
        };
        pass.run(move |resources, commands| {
            this.ffx.dispatch(
                commands,
                &ffx_frame,
                &FfxImages {
                    color: resources.image(color),
                    depth: resources.image(depth),
                    motion: resources.image(motion),
                    exposure: resources.image(exposure_image),
                    reactive: reactive.map(|m| resources.image(m)),
                    output: resources.image(output),
                },
            )
        });
        output
    }
}

fn create_output(device: &Arc<Device>, extent: vk::Extent2D) -> Result<GraphImage> {
    GraphImage::new(
        device,
        ImageDesc {
            width: extent.width,
            height: extent.height,
            format: HDR_FORMAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
            name: "ffx output",
        },
    )
}
