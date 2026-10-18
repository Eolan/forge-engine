//! The temporal frame (`shaders/taa.slang`): what DLSS and FSR resolve.
//!
//! The scene is drawn into an HDR colour target with a Halton-jittered projection, and a motion
//! pass reprojects every pixel into the previous frame from depth and the two cameras. DLSS
//! ([`crate::DlssUpscaler`]) or FSR 3.1 ([`crate::FsrUpscaler`]) resolve the frame with its
//! history; with the anti-aliasing off, nothing is jittered and the display pass shows the frame
//! as drawn. Forge's own TAA resolve and its RCAS sharpening were removed when FSR replaced them
//! (D-070); the history's clip, the Lanczos-3 resampling and the sharpening are in the git history
//! before that commit.
//!
//! The motion is a compute pass (#161): as a fullscreen draw, on the serial frame
//! (`FORGE_ASYNC=0`), the same draw from the same inputs came out one of two ways from one run to
//! the next.
//!
//! The colour and motion targets are transients of the render graph.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::{DMat4, Mat4, Vec2};

use crate::cells::CellPos;

/// Colour format of the offscreen scene target.
pub const HDR_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const MOTION_FORMAT: vk::Format = vk::Format::R16G16_SFLOAT;
/// Jitter phases before the sequence repeats, at native resolution. An upscaler wants
/// `JITTER_PHASES × (output / render)²` so every output pixel is covered ([`jitter_phases`]).
pub const JITTER_PHASES: u32 = 8;

/// Jitter phases for drawing at `render` and presenting at `output` (NVIDIA's rule for DLSS:
/// eight per output pixel covered by one render pixel).
pub fn jitter_phases(render: vk::Extent2D, output: vk::Extent2D) -> u32 {
    let scale = output.height.max(1) as f32 / render.height.max(1) as f32;
    ((JITTER_PHASES as f32 * scale * scale).round() as u32).max(JITTER_PHASES)
}

/// The `index`-th element of the Halton sequence in `base`, in [0, 1).
pub fn halton(mut index: u32, base: u32) -> f32 {
    let mut fraction = 1.0;
    let mut result = 0.0;
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

/// Sub-pixel jitter of frame `index` in a sequence of `phases`, in pixels (x right, y down),
/// in (−0.5, 0.5).
pub fn jitter(index: u64, phases: u32) -> Vec2 {
    let i = (index % u64::from(phases.max(1))) as u32 + 1;
    Vec2::new(halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// Shifts a projection so the scene moves by `jitter` pixels (x right, y down) on an image of
/// `extent` pixels.
pub fn jittered_projection(projection: Mat4, jitter: Vec2, extent: vk::Extent2D) -> Mat4 {
    let ndc = glam::Vec3::new(
        2.0 * jitter.x / extent.width.max(1) as f32,
        -2.0 * jitter.y / extent.height.max(1) as f32,
        0.0,
    );
    Mat4::from_translation(ndc) * projection
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MotionPush {
    previous_from_current: [f32; 16],
    jitter: [f32; 2],
    depth: u32,
    width: u32,
    height: u32,
    motion: u32,
}

/// What the current frame draws with, from [`Taa::begin`].
#[derive(Clone, Copy, Debug)]
pub struct TaaFrame {
    /// Projection shifted by this frame's jitter: draw with it.
    pub jittered_projection: Mat4,
    /// This frame's jitter in pixels.
    pub jitter: Vec2,
    /// The HDR colour target to draw into (a transient of the frame).
    pub color: ImageHandle,
    /// Size of the targets.
    pub extent: vk::Extent2D,
    /// The previous frame's unjittered clip coordinates from this frame's.
    pub previous_from_current: Mat4,
    /// Nothing on screen matches the frames before (first frame, a cut, a new size).
    pub reset: bool,
}

/// The temporal frame: the jitter, the reprojection and the motion vectors pass.
pub struct Taa {
    pipeline_motion: Pipeline,
    extent: vk::Extent2D,
    frame_index: u64,
    previous_view_proj: Option<DMat4>,
    /// Where the camera stood for `previous_view_proj` (issue #93).
    previous_camera: Option<CellPos>,
    reset: bool,
    /// When false (the anti-aliasing off), frames are drawn without jitter.
    pub enabled: bool,
    /// How much of the shown image is bloom, for the display pass (0.04 by default).
    pub bloom_strength: f32,
    /// Length of the jitter sequence ([`JITTER_PHASES`] at native resolution, more when an
    /// upscaler draws below it: [`jitter_phases`]).
    pub jitter_phases: u32,
}

impl Taa {
    /// Compiles the motion pass for frames of `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("taa.slang", "motion_main", ShaderStage::Compute)?,
            "taa motion",
        )?;
        let pipeline_motion = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "motion_main"),
            push_constant_bytes: std::mem::size_of::<MotionPush>() as u32,
            name: "taa motion",
        });
        device.destroy_shader_module(module);
        Ok(Self {
            pipeline_motion: pipeline_motion?,
            extent,
            frame_index: 0,
            previous_view_proj: None,
            previous_camera: None,
            reset: true,
            enabled: true,
            bloom_strength: 0.04,
            jitter_phases: JITTER_PHASES,
        })
    }

    /// Follows a new size: the frames after it start a new history.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.extent = extent;
        self.reset = true;
        Ok(())
    }

    /// The HDR scene target's format.
    pub fn color_format(&self) -> vk::Format {
        HDR_FORMAT
    }

    /// Discards the history at the next frame (a cut).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// Frames begun so far (the jitter phase is derived from it).
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// The unjittered world-to-clip of the previous frame, if any.
    pub fn previous_view_proj(&self) -> Option<DMat4> {
        self.previous_view_proj
    }

    /// Starts a frame: declares the HDR colour target and returns the jittered projection to
    /// draw with. `view_proj` is the *unjittered* camera-relative-to-clip of this frame and
    /// `camera` where the camera stands (the reprojection into the previous frame is derived
    /// from them and the previous frame's: the previous clip of a point is its clip in the
    /// previous camera's frame, which sits `camera − previous camera` away, issue #93).
    pub fn begin<'f>(
        &mut self,
        graph: &mut FrameGraph<'f>,
        projection: Mat4,
        view_proj: Mat4,
        camera: CellPos,
    ) -> TaaFrame {
        let jitter = if self.enabled {
            jitter(self.frame_index, self.jitter_phases)
        } else {
            Vec2::ZERO
        };
        let extent = self.extent;
        let current = view_proj.as_dmat4();
        // A point `x` relative to this camera was `x + (camera − previous camera)` relative to
        // the previous one; the step is small and exact in the cells' arithmetic.
        let previous_from_current = match (self.previous_view_proj, self.previous_camera) {
            (Some(previous), Some(previous_camera)) if !self.reset => {
                let step = camera.relative_to(previous_camera).as_dvec3();
                previous * DMat4::from_translation(step) * current.inverse()
            }
            _ => DMat4::IDENTITY,
        };
        self.previous_camera = Some(camera);
        let reset = self.reset;
        self.previous_view_proj = Some(current);
        self.reset = false;
        self.frame_index += 1;
        let color = graph.transient(TransientDesc {
            name: "taa scene color",
            width: extent.width,
            height: extent.height,
            format: HDR_FORMAT,
            // Written by the visibility resolve (storage) and the sky (attachment), sampled
            // by the upscalers and the display pass.
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        TaaFrame {
            jittered_projection: jittered_projection(projection, jitter, extent),
            jitter,
            color,
            extent,
            previous_from_current: previous_from_current.as_mat4(),
            reset,
        }
    }

    /// Declares the pass "temporal/motion vectors": for every pixel of the frame drawn with
    /// `depth` (the depth buffer the scene was drawn with), the offset in UV to where it was in
    /// the previous frame, without jitter (the camera's motion; the movers add theirs, #79).
    /// DLSS and FSR read it.
    pub fn motion_vectors<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        frame: &TaaFrame,
        depth: ImageHandle,
    ) -> ImageHandle {
        use vk::PipelineStageFlags2 as S;
        let frame = *frame;
        let extent = frame.extent;
        let motion = graph.transient(TransientDesc {
            name: "taa motion",
            width: extent.width,
            height: extent.height,
            format: MOTION_FORMAT,
            // Storage: written by a compute pass (#161), the movers' motion over the camera's (#79).
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let pipeline_motion = &self.pipeline_motion;
        graph
            .pass("temporal/motion vectors")
            .image(depth, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(motion, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline_motion);
                commands.push_constants(
                    pipeline_motion,
                    &MotionPush {
                        previous_from_current: frame.previous_from_current.to_cols_array(),
                        jitter: frame.jitter.to_array(),
                        depth: resources.sampled(depth).0,
                        width: extent.width,
                        height: extent.height,
                        motion: resources.storage(motion, 0).0,
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });
        motion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jitter_covers_the_pixel_evenly() {
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 2), 0.25);
        let points: Vec<Vec2> = (0..u64::from(JITTER_PHASES))
            .map(|i| jitter(i, JITTER_PHASES))
            .collect();
        let mean = points.iter().copied().sum::<Vec2>() / JITTER_PHASES as f32;
        assert!(mean.length() < 0.08, "{mean}");
        assert!(points.iter().all(|p| p.x.abs() < 0.5 && p.y.abs() < 0.5));
        assert_eq!(
            jitter(u64::from(JITTER_PHASES), JITTER_PHASES),
            jitter(0, JITTER_PHASES)
        );
        let native = vk::Extent2D {
            width: 1600,
            height: 900,
        };
        let half = vk::Extent2D {
            width: 800,
            height: 450,
        };
        assert_eq!(jitter_phases(native, native), JITTER_PHASES);
        assert_eq!(jitter_phases(half, native), 32);
    }

    #[test]
    fn a_jittered_projection_moves_the_scene_by_the_jitter() {
        let projection =
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.5, 0.05);
        let extent = vk::Extent2D {
            width: 300,
            height: 200,
        };
        let shifted = jittered_projection(projection, Vec2::new(0.25, -0.5), extent);
        let point = glam::Vec4::new(3.0, -2.0, -40.0, 1.0);
        let to_pixels = |clip: glam::Vec4| {
            let ndc = clip.truncate() / clip.w;
            Vec2::new((ndc.x * 0.5 + 0.5) * 300.0, (0.5 - ndc.y * 0.5) * 200.0)
        };
        let moved = to_pixels(shifted * point) - to_pixels(projection * point);
        assert!(moved.abs_diff_eq(Vec2::new(0.25, -0.5), 1e-3), "{moved}");
    }
}
