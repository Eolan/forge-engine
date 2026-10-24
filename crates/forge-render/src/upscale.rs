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

/// The pre-exposure handed to DLSS while it follows an adapting exposure, stops relative to
/// [`REFERENCE_EV100`]'s, at least and at most: the colour DLSS divides by it stays where fp16
/// keeps it (#164 posterized the night's darks at 17 stops; at 8 their steps are under 2 %, and
/// the brightest lamps stay under fp16's top at −6).
const FOLLOW_RANGE: (f32, f32) = (-6.0, 8.0);

/// Stops a frame the pre-exposure eases back to the steady one once it has followed the
/// exposure (`FORGE_DLSS_EXPOSURE_EASE` overrides it): DLSS's history is off by this much a
/// frame, against the whole adaptation's step before (0.06 stops a frame through a window).
const EASE_STOPS: f32 = 0.004;

/// A change of exposure over this many stops in one frame is a new start (the first frames'
/// jump to the metered exposure), not an adaptation to follow.
const CUT_STOPS: f32 = 2.0;

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
    /// Stops the pre-exposure stands from the steady one ([`pre_exposure`]), 0 when settled.
    follow: f32,
    /// The last frame's exposure relative to [`REFERENCE_EV100`], stops; `None` before the
    /// first.
    last_stops: Option<f32>,
    /// Stops a frame the pre-exposure eases back ([`EASE_STOPS`]).
    ease: f32,
}

/// The pre-exposure for an exposure `ratio` to [`REFERENCE_EV100`]'s, given how far it still
/// `follow`s the adaptation from the steady value (the ratio within 16, as #164 set it).
/// Handed the steady value alone, DLSS never saw the exposure change while it was clamped (a
/// night's street and the lit room behind a window both lie past 4 stops): its history no
/// longer matched the frame as the eye adapted, and it showed its jitter pattern as a fine
/// cross-hatch over the walls for a second or two (the owner, 2026-10-24: "a pattern stays
/// visible after going through the window").
fn pre_exposure(ratio: f32, follow: f32) -> f32 {
    if follow == 0.0 {
        ratio.clamp(1.0 / 16.0, 16.0)
    } else {
        2f32.powf(ratio.log2().clamp(-4.0, 4.0) + follow)
    }
}

/// The next `follow` after the exposure moved from `last` to `stops` (relative stops): the
/// change taken by the pre-exposure as far as the steady value's clamp hid it, within
/// [`FOLLOW_RANGE`], then eased back by `ease` stops towards the steady value. A jump past
/// [`CUT_STOPS`] starts again from the steady value.
fn next_follow(follow: f32, last: f32, stops: f32, ease: f32) -> f32 {
    if (stops - last).abs() > CUT_STOPS {
        return 0.0;
    }
    let (before, after) = (last.clamp(-4.0, 4.0), stops.clamp(-4.0, 4.0));
    // What the clamp hid of the change: the exposure's move less the steady value's.
    let hidden = (stops - last) - (after - before);
    let f = (follow + hidden).clamp(FOLLOW_RANGE.0 - after, FOLLOW_RANGE.1 - after);
    if f.abs() <= ease {
        0.0
    } else {
        f - ease.copysign(f)
    }
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
            follow: 0.0,
            last_stops: None,
            ease: std::env::var("FORGE_DLSS_EXPOSURE_EASE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(EASE_STOPS),
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
    /// pre-exposure; `reactive`, when the frame has one, is the mask FSR reads too (1 where the
    /// history is not to be trusted: the spray, the particles, the tank's water), which DLSS
    /// takes as its bias towards the current colour.
    #[allow(clippy::too_many_arguments)]
    pub fn upscale<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        frame_number: u64,
        frame: &TaaFrame,
        camera: UpscaleCamera,
        depth: ImageHandle,
        motion: ImageHandle,
        reactive: Option<ImageHandle>,
        exposure: f32,
    ) -> Result<ImageHandle> {
        let reset = std::mem::take(&mut self.reset) || frame.reset;
        let ratio = exposure / exposure_from_ev100(REFERENCE_EV100);
        let stops = ratio.log2();
        self.follow = match self.last_stops {
            Some(last) if !reset && stops.is_finite() => {
                next_follow(self.follow, last, stops, self.ease)
            }
            _ => 0.0,
        };
        self.last_stops = stops.is_finite().then_some(stops);
        let pre = pre_exposure(ratio, self.follow);
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
            // Within 4 stops of it only (#164): a night under the Moon is 17 stops from EV 15,
            // and the colour divided by 2^17 fell under fp16's normal range inside DLSS, which
            // posterized it and smeared the stars. Past that, the adaptation's changes are
            // followed for a while (`pre_exposure`), then eased back.
            pre,
        )?;
        let output = graph.import(&this.output);
        let exposure_image = graph.import(&this.exposure);
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let color = frame.color;
        let pass = graph
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
            );
        let pass = match reactive {
            Some(mask) => pass.image(mask, ImageAccess::Sampled(compute)),
            None => pass,
        };
        pass.run(move |resources, commands| {
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
                    bias: reactive.map(sampled),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Through a window at night the exposure falls 6 stops while both ends lie past the
    /// steady value's clamp: the pre-exposure follows the change frame by frame, then eases
    /// back to the steady value.
    #[test]
    fn the_pre_exposure_follows_an_adaptation_past_the_clamp_then_eases_back() {
        let (mut follow, mut last) = (0.0, 16.9_f32);
        let mut pre_last = pre_exposure(2f32.powf(last), follow);
        assert_eq!(pre_last, 16.0);
        for _ in 0..100 {
            let stops = last - 0.061;
            follow = next_follow(follow, last, stops, 0.0);
            let pre = pre_exposure(2f32.powf(stops), follow);
            assert!(((pre / pre_last).log2() + 0.061).abs() < 1e-3);
            (last, pre_last) = (stops, pre);
        }
        for _ in 0..2000 {
            follow = next_follow(follow, last, last, EASE_STOPS);
        }
        assert_eq!(follow, 0.0);
        assert_eq!(pre_exposure(2f32.powf(last), follow), 16.0);
    }

    /// The first frames' jump to the metered exposure is no adaptation; and within the clamp
    /// nothing is followed.
    #[test]
    fn a_jump_or_a_change_within_the_clamp_leaves_the_steady_value() {
        assert_eq!(next_follow(0.0, 1.0, 16.9, EASE_STOPS), 0.0);
        assert_eq!(next_follow(0.0, 1.0, 1.5, EASE_STOPS), 0.0);
        // And the pre-exposure never leaves where fp16 keeps the colour.
        let f = next_follow(0.0, 4.0, 5.9, 0.0);
        assert!(4.0 + f <= FOLLOW_RANGE.1);
    }
}
