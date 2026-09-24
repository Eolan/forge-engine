//! Temporal anti-aliasing (`shaders/taa.slang`).
//!
//! The scene is drawn into an HDR colour target with a Halton-jittered projection. A motion
//! pass reprojects every pixel into the previous frame from depth and the two cameras; the
//! resolve blends this frame's samples with the history fetched there, clipped to the
//! neighbourhood, and writes the next history, which is then blitted to the swapchain.
//! Ported from the previous project's `temporal.rs` (Karis 2014, Jimenez 2016, Playdead 2016).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Commands, Device, FullscreenPipelineDesc, Image, ImageDesc, Pipeline, Result, SampledImageId,
    ShaderCompiler, ShaderStage, vk,
};
use glam::{DMat4, Mat4, Vec2};

/// Colour format of the offscreen scene target and the history.
pub const HDR_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const MOTION_FORMAT: vk::Format = vk::Format::R16G16_SFLOAT;
/// Jitter phases before the sequence repeats.
pub const JITTER_PHASES: u32 = 8;

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

/// Sub-pixel jitter of frame `index`, in pixels (x right, y down), in (−0.5, 0.5).
pub fn jitter(index: u64) -> Vec2 {
    let i = (index % u64::from(JITTER_PHASES)) as u32 + 1;
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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolvePush {
    color: u32,
    motion: u32,
    depth: u32,
    history: u32,
    width: u32,
    height: u32,
    jitter: [f32; 2],
    blend: f32,
    pad: [u32; 3],
}

struct Targets {
    color: Image,
    color_id: SampledImageId,
    motion: Image,
    motion_id: SampledImageId,
    history: [Image; 2],
    history_ids: [SampledImageId; 2],
    extent: vk::Extent2D,
}

impl Targets {
    fn new(device: &Arc<Device>, extent: vk::Extent2D) -> Result<Self> {
        let make = |format: vk::Format, name: &str| {
            device.create_image(ImageDesc {
                width: extent.width,
                height: extent.height,
                format,
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name,
            })
        };
        let color = make(HDR_FORMAT, "taa scene color")?;
        let motion = make(MOTION_FORMAT, "taa motion")?;
        let history = [
            make(HDR_FORMAT, "taa history 0")?,
            make(HDR_FORMAT, "taa history 1")?,
        ];
        for h in &history {
            device.initialize_image_layout(
                h,
                vk::ImageAspectFlags::COLOR,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            )?;
        }
        let color_id =
            device.register_sampled_image(color.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let motion_id =
            device.register_sampled_image(motion.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let history_ids = [
            device.register_sampled_image(
                history[0].view(),
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ),
            device.register_sampled_image(
                history[1].view(),
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ),
        ];
        Ok(Self {
            color,
            color_id,
            motion,
            motion_id,
            history,
            history_ids,
            extent,
        })
    }

    fn release(&self, device: &Device) {
        device.release_sampled_image(self.color_id);
        device.release_sampled_image(self.motion_id);
        for &id in &self.history_ids {
            device.release_sampled_image(id);
        }
    }
}

/// What the current frame draws with.
#[derive(Clone, Copy, Debug)]
pub struct TaaFrame {
    /// Projection shifted by this frame's jitter: draw with it.
    pub jittered_projection: Mat4,
    /// This frame's jitter in pixels.
    pub jitter: Vec2,
    /// Colour target to draw into (HDR, `COLOR_ATTACHMENT_OPTIMAL` after `begin`).
    pub color_view: vk::ImageView,
}

/// The temporal anti-aliasing passes and their targets.
pub struct Taa {
    device: Arc<Device>,
    pipeline_motion: Pipeline,
    pipeline_resolve: Pipeline,
    targets: Targets,
    frame_index: u64,
    previous_view_proj: Option<DMat4>,
    reset: bool,
    /// Steady-state share of the current frame (0.1 is a typical TAA; 1 disables the history).
    pub blend: f32,
    /// When false, frames are drawn without jitter and resolved without history: the passes
    /// still run (so the output path is identical) but the image is the plain scene.
    pub enabled: bool,
}

impl Taa {
    /// Compiles the passes and creates targets for `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("taa.slang", "vert_main", ShaderStage::Vertex)?,
            "taa vs",
        )?;
        let motion = device.create_shader_module(
            &shaders.compile("taa.slang", "motion_main", ShaderStage::Fragment)?,
            "taa motion",
        )?;
        let resolve = device.create_shader_module(
            &shaders.compile("taa.slang", "resolve_main", ShaderStage::Fragment)?,
            "taa resolve",
        )?;
        let pipeline_motion = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (motion, "motion_main"),
            color_formats: &[MOTION_FORMAT],
            push_constant_bytes: std::mem::size_of::<MotionPush>() as u32,
            alpha_blend: false,
            name: "taa motion",
        })?;
        let pipeline_resolve = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (resolve, "resolve_main"),
            color_formats: &[HDR_FORMAT],
            push_constant_bytes: std::mem::size_of::<ResolvePush>() as u32,
            alpha_blend: false,
            name: "taa resolve",
        })?;
        for module in [vertex, motion, resolve] {
            device.destroy_shader_module(module);
        }
        let targets = Targets::new(device, extent)?;
        Ok(Self {
            device: Arc::clone(device),
            pipeline_motion,
            pipeline_resolve,
            targets,
            frame_index: 0,
            previous_view_proj: None,
            reset: true,
            blend: 0.1,
            enabled: true,
        })
    }

    /// Recreates the targets (device idle) and restarts the history.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.targets.release(&self.device);
        self.targets = Targets::new(&self.device, extent)?;
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

    /// Frames resolved so far (the jitter phase is derived from it).
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// The unjittered world-to-clip of the previous resolved frame, if any.
    pub fn previous_view_proj(&self) -> Option<DMat4> {
        self.previous_view_proj
    }

    /// Starts a frame: transitions the scene target for drawing and returns the jittered
    /// projection to draw with.
    pub fn begin(&mut self, commands: &Commands<'_>, projection: Mat4) -> TaaFrame {
        let jitter = if self.enabled {
            jitter(self.frame_index)
        } else {
            Vec2::ZERO
        };
        let extent = self.targets.extent;
        commands.image_barriers(&[color_barrier(self.targets.color.raw())
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)]);
        TaaFrame {
            jittered_projection: jittered_projection(projection, jitter, extent),
            jitter,
            color_view: self.targets.color.view(),
        }
    }

    /// Resolves the frame drawn into the scene target and writes the result to `output`
    /// (a swapchain image in `COLOR_ATTACHMENT_OPTIMAL`, left in that layout).
    ///
    /// `depth` is the depth buffer the scene was drawn with (in `DEPTH_ATTACHMENT_OPTIMAL`, left
    /// in `SHADER_READ_ONLY_OPTIMAL`); `view_proj` the *unjittered* world-to-clip of this frame.
    pub fn resolve(
        &mut self,
        commands: &Commands<'_>,
        frame: &TaaFrame,
        depth: (vk::Image, SampledImageId),
        view_proj: Mat4,
        output: vk::Image,
    ) {
        let extent = self.targets.extent;
        let current = view_proj.as_dmat4();
        let previous_from_current = match self.previous_view_proj {
            Some(previous) if !self.reset => previous * current.inverse(),
            _ => DMat4::IDENTITY,
        };
        let blend = if self.reset || !self.enabled {
            1.0
        } else {
            self.blend
        };
        self.previous_view_proj = Some(current);
        self.reset = false;
        let written = (self.frame_index % 2) as usize;
        let read = 1 - written;

        // Scene colour and depth become readable; the motion target becomes writable.
        let depth_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        commands.image_barriers(&[
            color_barrier(self.targets.color.raw())
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(depth.0)
                .subresource_range(depth_range)
                // Depth is written by the early *or* the late fragment tests: name both.
                .src_stage_mask(
                    vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                        | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
                )
                .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            color_barrier(self.targets.motion.raw())
                .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            color_barrier(self.targets.history[written].raw())
                .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        ]);

        // Motion vectors.
        fullscreen_pass(
            commands,
            self.targets.motion.view(),
            extent,
            &self.pipeline_motion,
            &MotionPush {
                previous_from_current: previous_from_current.as_mat4().to_cols_array(),
                jitter: frame.jitter.to_array(),
                depth: depth.1.0,
                width: extent.width,
                height: extent.height,
            },
        );
        commands.image_barriers(&[color_barrier(self.targets.motion.raw())
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]);
        commands.mark("temporal/motion vectors");

        // Resolve into the written history.
        fullscreen_pass(
            commands,
            self.targets.history[written].view(),
            extent,
            &self.pipeline_resolve,
            &ResolvePush {
                color: self.targets.color_id.0,
                motion: self.targets.motion_id.0,
                depth: depth.1.0,
                history: self.targets.history_ids[read].0,
                width: extent.width,
                height: extent.height,
                jitter: frame.jitter.to_array(),
                blend,
                pad: [0; 3],
            },
        );
        commands.mark("temporal/TAA resolve");

        // Blit the result to the output and leave the history readable for the next frame.
        let color_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        commands.image_barriers(&[
            color_barrier(self.targets.history[written].raw())
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(output)
                .subresource_range(color_range)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL),
        ]);
        commands.blit_image(
            self.targets.history[written].raw(),
            output,
            extent,
            vk::Filter::NEAREST,
        );
        commands.image_barriers(&[
            color_barrier(self.targets.history[written].raw())
                .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(output)
                .subresource_range(color_range)
                .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        ]);
        commands.mark("temporal/blit to swapchain");
        self.frame_index += 1;
    }
}

impl Drop for Taa {
    fn drop(&mut self) {
        self.targets.release(&self.device);
    }
}

fn color_barrier(image: vk::Image) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
}

fn fullscreen_pass<P: Pod>(
    commands: &Commands<'_>,
    target: vk::ImageView,
    extent: vk::Extent2D,
    pipeline: &Pipeline,
    push: &P,
) {
    let color = [vk::RenderingAttachmentInfo::default()
        .image_view(target)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::DONT_CARE)
        .store_op(vk::AttachmentStoreOp::STORE)];
    let info = vk::RenderingInfo::default()
        .render_area(vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        })
        .layer_count(1)
        .color_attachments(&color);
    commands.begin_rendering(&info);
    commands.bind_pipeline(pipeline);
    commands.set_viewport_full(extent);
    commands.push_constants(pipeline, push);
    commands.draw(3, 1);
    commands.end_rendering();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jitter_covers_the_pixel_evenly() {
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 2), 0.25);
        let points: Vec<Vec2> = (0..u64::from(JITTER_PHASES)).map(jitter).collect();
        let mean = points.iter().copied().sum::<Vec2>() / JITTER_PHASES as f32;
        assert!(mean.length() < 0.08, "{mean}");
        assert!(points.iter().all(|p| p.x.abs() < 0.5 && p.y.abs() < 0.5));
        assert_eq!(jitter(u64::from(JITTER_PHASES)), jitter(0));
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
