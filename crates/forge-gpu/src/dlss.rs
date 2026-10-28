//! DLSS Super Resolution through NVIDIA Streamline (issue #8): the jittered frame, its depth
//! and its motion vectors upscaled by NVIDIA's model in place of a temporal resolve.
//!
//! Always compiled, so renderers and demos need no feature gates: in builds without the `dlss`
//! feature (or off Windows) [`Dlss`] cannot be constructed and [`crate::Device::dlss`] is always
//! `None`. With the feature, [`crate::Instance::with_streamline`] loads Streamline's interposer
//! in place of the Vulkan loader, and the device offers DLSS when its GPU runs it.

use ash::vk;

use crate::commands::Commands;
use crate::error::Result;
use crate::graph::ResolvedImage;

/// How much DLSS upscales: the render size is a fraction of the output in each direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DlssMode {
    /// DLAA: anti-aliasing at the output resolution (no upscaling).
    Dlaa,
    /// About 0.67 of the output in each direction.
    Quality,
    /// About 0.58.
    Balanced,
    /// 0.5.
    Performance,
    /// About 0.33.
    UltraPerformance,
}

impl DlssMode {
    /// Every mode, from the highest render resolution down.
    pub const ALL: [Self; 5] = [
        Self::Dlaa,
        Self::Quality,
        Self::Balanced,
        Self::Performance,
        Self::UltraPerformance,
    ];

    /// Short name (command-line value).
    pub fn name(self) -> &'static str {
        match self {
            Self::Dlaa => "dlaa",
            Self::Quality => "quality",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::UltraPerformance => "ultra-performance",
        }
    }

    /// Name for the screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dlaa => "DLAA",
            Self::Quality => "DLSS Quality",
            Self::Balanced => "DLSS Balanced",
            Self::Performance => "DLSS Performance",
            Self::UltraPerformance => "DLSS Ultra Performance",
        }
    }

    /// The mode with `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }

    /// `sl::DLSSMode`.
    #[cfg_attr(not(all(feature = "dlss", windows)), allow(dead_code))]
    pub(crate) fn streamline(self) -> u32 {
        match self {
            Self::Performance => 1,
            Self::Balanced => 2,
            Self::Quality => 3,
            Self::UltraPerformance => 4,
            Self::Dlaa => 6,
        }
    }
}

/// The camera of one frame as DLSS needs it. Matrices are column-major (`glam`'s
/// `to_cols_array`) and carry no jitter.
#[derive(Clone, Copy, Debug)]
pub struct DlssFrame {
    /// Clip coordinates from camera-view coordinates.
    pub clip_from_view: [f32; 16],
    /// Its inverse.
    pub view_from_clip: [f32; 16],
    /// The previous frame's clip coordinates from this frame's.
    pub previous_clip_from_clip: [f32; 16],
    /// Its inverse.
    pub clip_from_previous_clip: [f32; 16],
    /// The jitter in render pixels, x right and y down, as the projection moved the scene.
    pub jitter: [f32; 2],
    /// Nothing on screen matches the frames before (a cut, a new size, a new mode).
    pub reset: bool,
    /// Near plane in metres (the projection is reversed-Z with an infinite far plane).
    pub near: f32,
    /// Vertical field of view, radians.
    pub vertical_fov: f32,
    /// Width over height.
    pub aspect: f32,
}

/// An image a pass hands to DLSS, in the layout the pass declared it in.
#[derive(Clone, Copy, Debug)]
pub struct DlssImage<'a> {
    /// The image, from [`crate::Resources::image`].
    pub image: &'a ResolvedImage,
    /// Its layout during the pass (the graph put it there).
    pub layout: vk::ImageLayout,
}

/// What DLSS reads and writes in one evaluation.
#[derive(Clone, Copy, Debug)]
pub struct DlssImages<'a> {
    /// The jittered, pre-exposed HDR colour at the render size.
    pub color: DlssImage<'a>,
    /// The hardware depth it was drawn with (reversed-Z).
    pub depth: DlssImage<'a>,
    /// Motion vectors at the render size: the offset in UV from each pixel to where it was in
    /// the previous frame, without jitter.
    pub motion: DlssImage<'a>,
    /// A 1 × 1 `R32_SFLOAT` image holding the exposure that brings the colour to the tone
    /// curve's input (1 for a pre-exposed scene).
    pub exposure: DlssImage<'a>,
    /// The upscaled HDR colour at the output size (storage image).
    pub output: DlssImage<'a>,
}

/// The frame DLSS was prepared for, handed from [`Dlss::prepare`] to [`Dlss::evaluate`].
#[derive(Clone, Copy, Debug)]
pub struct DlssToken {
    #[cfg_attr(not(all(feature = "dlss", windows)), allow(dead_code))]
    pub(crate) token: usize,
}

/// DLSS on this device (see [`crate::Device::dlss`]).
pub struct Dlss(imp::Dlss);

impl Dlss {
    #[cfg(all(feature = "dlss", windows))]
    pub(crate) fn new(streamline: std::sync::Arc<crate::streamline::Streamline>) -> Self {
        Self(imp::Dlss { streamline })
    }

    /// The render size DLSS asks for in `mode` at an output of `output` pixels.
    pub fn render_size(&self, mode: DlssMode, output: vk::Extent2D) -> Result<vk::Extent2D> {
        self.0.render_size(mode, output)
    }

    /// Sets up frame `frame_number` for `mode` at an output of `output` pixels: the camera
    /// constants and the options. `pre_exposure` is what the input colour was multiplied by.
    pub fn prepare(
        &self,
        frame_number: u64,
        mode: DlssMode,
        output: vk::Extent2D,
        frame: &DlssFrame,
        pre_exposure: f32,
    ) -> Result<DlssToken> {
        self.0
            .prepare(frame_number, mode, output, frame, pre_exposure)
    }

    /// Records DLSS for the prepared frame into `commands`, from inside a graph pass that
    /// declared every image of `images` (the colour, depth, motion and exposure sampled by
    /// compute, the output written as storage). Streamline restores each image's layout.
    pub fn evaluate(
        &self,
        commands: &Commands<'_>,
        token: DlssToken,
        images: &DlssImages<'_>,
    ) -> Result<()> {
        self.0.evaluate(commands, token, images)
    }
}

#[cfg(all(feature = "dlss", windows))]
mod imp {
    use std::sync::Arc;

    use ash::vk;

    use super::{DlssFrame, DlssImage, DlssImages, DlssMode, DlssToken};
    use crate::commands::Commands;
    use crate::error::Result;
    use crate::streamline::{
        BUFFER_DEPTH, BUFFER_EXPOSURE, BUFFER_MOTION_VECTORS, BUFFER_SCALING_INPUT_COLOR,
        BUFFER_SCALING_OUTPUT_COLOR, Constants, DlssOptions, Extent, FALSE, Resource, ResourceTag,
        Streamline, TRUE, ViewportHandle, VulkanImage,
    };

    /// The one viewport this engine upscales.
    const VIEWPORT: u32 = 0;
    /// Far plane handed to DLSS for an infinite projection.
    const FAR: f32 = 1.0e9;

    pub(super) struct Dlss {
        pub(super) streamline: Arc<Streamline>,
    }

    fn vulkan_image(image: &DlssImage<'_>) -> VulkanImage {
        use ash::vk::Handle;
        VulkanImage {
            image: image.image.raw.as_raw(),
            view: image.image.view.as_raw(),
            memory: 0,
            layout: image.layout.as_raw(),
            format: image.image.format.as_raw(),
            usage: image.image.usage.as_raw(),
            width: image.image.extent.width,
            height: image.image.extent.height,
        }
    }

    impl Dlss {
        pub(super) fn render_size(
            &self,
            mode: DlssMode,
            output: vk::Extent2D,
        ) -> Result<vk::Extent2D> {
            let settings = self.streamline.dlss_optimal_settings(&DlssOptions::new(
                mode.streamline(),
                [output.width, output.height],
                1.0,
            ))?;
            Ok(vk::Extent2D {
                width: settings.optimal_render_width.clamp(1, output.width.max(1)),
                height: settings
                    .optimal_render_height
                    .clamp(1, output.height.max(1)),
            })
        }

        pub(super) fn prepare(
            &self,
            frame_number: u64,
            mode: DlssMode,
            output: vk::Extent2D,
            frame: &DlssFrame,
            pre_exposure: f32,
        ) -> Result<DlssToken> {
            let token = self.streamline.frame_token(frame_number as u32)?;
            let viewport = ViewportHandle::new(VIEWPORT);
            let constants = Constants {
                camera_view_to_clip: frame.clip_from_view,
                clip_to_camera_view: frame.view_from_clip,
                clip_to_prev_clip: frame.previous_clip_from_clip,
                prev_clip_to_clip: frame.clip_from_previous_clip,
                jitter_offset: frame.jitter,
                // The motion vectors are UV offsets: already a fraction of the image.
                mvec_scale: [1.0, 1.0],
                camera_near: frame.near,
                camera_far: FAR,
                camera_fov: frame.vertical_fov,
                camera_aspect_ratio: frame.aspect,
                depth_inverted: TRUE,
                camera_motion_included: TRUE,
                motion_vectors_3d: FALSE,
                reset: if frame.reset { TRUE } else { FALSE },
                motion_vectors_jittered: FALSE,
                ..Constants::default()
            };
            self.streamline
                .set_constants(&constants, token, &viewport)?;
            self.streamline.dlss_set_options(
                &viewport,
                &DlssOptions::new(
                    mode.streamline(),
                    [output.width, output.height],
                    pre_exposure,
                ),
            )?;
            Ok(DlssToken {
                token: token as usize,
            })
        }

        pub(super) fn evaluate(
            &self,
            commands: &Commands<'_>,
            token: DlssToken,
            images: &DlssImages<'_>,
        ) -> Result<()> {
            let viewport = ViewportHandle::new(VIEWPORT);
            let tagged = [
                (images.color, BUFFER_SCALING_INPUT_COLOR),
                (images.depth, BUFFER_DEPTH),
                (images.motion, BUFFER_MOTION_VECTORS),
                (images.exposure, BUFFER_EXPOSURE),
                (images.output, BUFFER_SCALING_OUTPUT_COLOR),
            ]
            .map(|(image, kind)| (vulkan_image(&image), kind));
            let mut resources = tagged.map(|(image, _)| Resource::image(image));
            let tags: Vec<ResourceTag> = resources
                .iter_mut()
                .zip(tagged)
                .map(|(resource, (image, kind))| {
                    let extent = Extent {
                        top: 0,
                        left: 0,
                        width: image.width,
                        height: image.height,
                    };
                    ResourceTag::until_present(resource, kind, extent)
                })
                .collect();
            let token = token.token as *mut std::ffi::c_void;
            let raw = vk::Handle::as_raw(commands.raw());
            // SAFETY: the pass declared every tagged image, so the graph put each in the layout
            // its tag gives and keeps it alive for the frame; `commands` is being recorded; the
            // tags and the resources they point to outlive both calls.
            unsafe {
                self.streamline
                    .set_tags(token, &viewport, &tags, raw)
                    .and_then(|()| self.streamline.evaluate_dlss(token, &viewport, raw))
            }
        }
    }
}

#[cfg(not(all(feature = "dlss", windows)))]
mod imp {
    use ash::vk;

    use super::{DlssFrame, DlssImages, DlssMode, DlssToken};
    use crate::commands::Commands;
    use crate::error::Result;

    /// Never constructed without the `dlss` feature on Windows.
    pub(super) struct Dlss(std::convert::Infallible);

    impl Dlss {
        pub(super) fn render_size(&self, _: DlssMode, _: vk::Extent2D) -> Result<vk::Extent2D> {
            match self.0 {}
        }

        pub(super) fn prepare(
            &self,
            _: u64,
            _: DlssMode,
            _: vk::Extent2D,
            _: &DlssFrame,
            _: f32,
        ) -> Result<DlssToken> {
            match self.0 {}
        }

        pub(super) fn evaluate(
            &self,
            _: &Commands<'_>,
            _: DlssToken,
            _: &DlssImages<'_>,
        ) -> Result<()> {
            match self.0 {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_their_names() {
        for mode in DlssMode::ALL {
            assert_eq!(DlssMode::from_name(mode.name()), Some(mode));
        }
        assert_eq!(DlssMode::from_name("taa"), None);
    }
}
