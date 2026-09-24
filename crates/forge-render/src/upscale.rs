//! DLSS Super Resolution in the frame (issue #8): the jittered HDR frame, its depth and the
//! TAA's motion vectors handed to NVIDIA's model through Streamline ([`forge_gpu::dlss`]) in
//! place of the TAA resolve, upscaled into an HDR image at the output size that the display
//! pass then takes through the tone curve.
//!
//! The renderer draws at [`DlssUpscaler::render_extent`] (the output in DLAA, about two
//! thirds of it in Quality, half in Performance) with a longer jitter sequence
//! ([`crate::taa::jitter_phases`]); everything before the resolve is unchanged. Without DLSS
//! on the device (no `dlss` feature, no Streamline, another GPU) [`DlssUpscaler::new`] finds
//! nothing and the TAA stays the only path.

use std::sync::Arc;

use forge_gpu::{
    Device, DlssFrame, DlssImage, DlssImages, DlssMode, FrameGraph, GraphImage, ImageAccess,
    ImageDesc, ImageHandle, Result, vk,
};
use glam::Mat4;

use crate::exposure::exposure_from_ev100;
use crate::taa::{HDR_FORMAT, TaaFrame};

/// The exposure DLSS sees the scene at: pre-exposure is handed over relative to it.
const REFERENCE_EV100: f32 = 15.0;

/// The camera a frame was drawn with, besides what the [`TaaFrame`] carries.
#[derive(Clone, Copy, Debug)]
pub struct UpscaleCamera {
    /// The unjittered projection.
    pub projection: Mat4,
    /// Near plane in metres.
    pub near: f32,
    /// Vertical field of view, radians.
    pub vertical_fov: f32,
}

/// DLSS's output image and its exposure input.
pub struct DlssUpscaler {
    device: Arc<Device>,
    mode: DlssMode,
    output_extent: vk::Extent2D,
    output: GraphImage,
    /// 1 × 1 `R32_SFLOAT` holding 1: the scene is pre-exposed, so the tone curve takes the
    /// colour as it is (DLSS would guess an exposure of its own without it).
    exposure: GraphImage,
    /// The next frame starts a new history (first frame, new mode or size).
    reset: bool,
}

impl DlssUpscaler {
    /// DLSS in `mode` for an output of `output_extent`, or `None` when the device has no DLSS.
    pub fn new(
        device: &Arc<Device>,
        mode: DlssMode,
        output_extent: vk::Extent2D,
    ) -> Result<Option<Self>> {
        if device.dlss().is_none() {
            return Ok(None);
        }
        let exposure = GraphImage::uploaded(
            device,
            ImageDesc {
                width: 1,
                height: 1,
                format: vk::Format::R32_SFLOAT,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "dlss exposure",
            },
            bytemuck::bytes_of(&1.0_f32),
        )?;
        Ok(Some(Self {
            device: Arc::clone(device),
            mode,
            output_extent,
            output: create_output(device, output_extent)?,
            exposure,
            reset: true,
        }))
    }

    /// The mode.
    pub fn mode(&self) -> DlssMode {
        self.mode
    }

    /// Switches to `mode` for an output of `output_extent` (the device must be idle when the
    /// size changes); the next frame starts a new history.
    pub fn configure(&mut self, mode: DlssMode, output_extent: vk::Extent2D) -> Result<()> {
        if output_extent != self.output_extent {
            self.output = create_output(&self.device, output_extent)?;
            self.output_extent = output_extent;
        }
        self.mode = mode;
        self.reset = true;
        Ok(())
    }

    /// The size to draw at for the current mode and output.
    pub fn render_extent(&self) -> Result<vk::Extent2D> {
        match self.device.dlss() {
            Some(dlss) => dlss.render_size(self.mode, self.output_extent),
            None => Ok(self.output_extent),
        }
    }

    /// Declares the pass "temporal/DLSS" that upscales `frame.color` (drawn with `depth`, moved
    /// by `motion` from [`crate::Taa::motion_vectors`]) into the HDR output, and returns it.
    /// `frame_number` identifies the frame to Streamline; `exposure` is the frame's
    /// pre-exposure.
    #[allow(clippy::too_many_arguments)]
    pub fn upscale<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        frame_number: u64,
        frame: &TaaFrame,
        camera: UpscaleCamera,
        depth: ImageHandle,
        motion: ImageHandle,
        exposure: f32,
    ) -> Result<ImageHandle> {
        let reset = std::mem::take(&mut self.reset) || frame.reset;
        let this: &'f Self = self;
        let dlss = this
            .device
            .dlss()
            .expect("a DlssUpscaler exists only on a device with DLSS");
        let aspect = frame.extent.width.max(1) as f32 / frame.extent.height.max(1) as f32;
        let clip_from_current = frame.previous_from_current;
        let token = dlss.prepare(
            frame_number,
            this.mode,
            this.output_extent,
            &DlssFrame {
                clip_from_view: camera.projection.to_cols_array(),
                view_from_clip: camera.projection.inverse().to_cols_array(),
                previous_clip_from_clip: clip_from_current.to_cols_array(),
                clip_from_previous_clip: clip_from_current.inverse().to_cols_array(),
                jitter: frame.jitter.to_array(),
                reset,
                near: camera.near,
                vertical_fov: camera.vertical_fov,
                aspect,
            },
            // Relative to a fixed EV100: DLSS divides the colour by it on the way in and
            // multiplies it back on the way out (checked: the output keeps the input's level), so
            // its history stays in one space while the exposure adapts, as the TAA's rescale does.
            exposure / exposure_from_ev100(REFERENCE_EV100),
        )?;
        let output = graph.import(&this.output);
        let exposure_image = graph.import(&this.exposure);
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let color = frame.color;
        graph
            .pass("temporal/DLSS")
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(motion, ImageAccess::Sampled(compute))
            .image(exposure_image, ImageAccess::Sampled(compute))
            // DLSS clears its output (a transfer) before writing it from compute shaders.
            .image(
                output,
                ImageAccess::Custom {
                    layout: vk::ImageLayout::GENERAL,
                    stages: compute | vk::PipelineStageFlags2::CLEAR,
                    access: vk::AccessFlags2::TRANSFER_WRITE
                        | vk::AccessFlags2::SHADER_STORAGE_READ
                        | vk::AccessFlags2::SHADER_STORAGE_WRITE,
                },
            )
            .run(move |resources, commands| {
                let sampled = |handle| {
                    let image = resources.image(handle);
                    DlssImage {
                        image,
                        layout: image.sampled_layout,
                    }
                };
                dlss.evaluate(
                    commands,
                    token,
                    &DlssImages {
                        color: sampled(color),
                        depth: sampled(depth),
                        motion: sampled(motion),
                        exposure: sampled(exposure_image),
                        output: DlssImage {
                            image: resources.image(output),
                            layout: vk::ImageLayout::GENERAL,
                        },
                    },
                )
            });
        Ok(output)
    }
}

fn create_output(device: &Arc<Device>, extent: vk::Extent2D) -> Result<GraphImage> {
    GraphImage::new(
        device,
        ImageDesc {
            width: extent.width,
            height: extent.height,
            format: HDR_FORMAT,
            // DLSS clears it (a transfer) before writing it (storage).
            usage: vk::ImageUsageFlags::STORAGE
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_DST,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
            name: "dlss output",
        },
    )
}
