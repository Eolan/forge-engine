//! Temporal anti-aliasing (`shaders/taa.slang`).
//!
//! The scene is drawn into an HDR colour target with a Halton-jittered projection. A motion
//! pass reprojects every pixel into the previous frame from depth and the two cameras; the
//! resolve blends this frame's samples with the history fetched there, clipped to the
//! neighbourhood, and writes the next history (HDR) and the display image (through the tone
//! curve, [`crate::display`]) in one pass.
//! Ported from the previous project's `temporal.rs` (Karis 2014, Jimenez 2016, Playdead 2016).
//!
//! With [`Taa::sharpen`] (D-045), the resolve writes the history alone and a pass of its own
//! shows it, sharpened by AMD's FidelityFX RCAS on the display's signal: TAA's history,
//! resampled every frame, softens what moves.
//!
//! The scene is pre-exposed ([`crate::exposure`]): when the exposure changes between frames
//! the history, stored at the previous exposure, is rescaled by the ratio before blending.
//!
//! The colour and motion targets are transients of the render graph; the two histories are
//! persistent [`GraphImage`]s whose state the graph carries from frame to frame.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Commands, Device, FrameGraph, FullscreenPipelineDesc, GraphImage, ImageAccess, ImageDesc,
    ImageHandle, Pipeline, Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::{DMat4, Mat4, Vec2};

use crate::cells::CellPos;
use crate::display::{HdrOutput, OutputEncoding, OutputPush, ToneTables, ToneTablesPush, Tonemap};

/// Colour format of the offscreen scene target and the history.
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
    history_scale: f32,
    curve: u32,
    /// The bloom chain's top level (sampled index), or `u32::MAX` for none.
    bloom: u32,
    /// How much of the shown image is bloom (`crate::bloom`).
    bloom_strength: f32,
    output: OutputPush,
    /// The reactive mask (sampled index), or `u32::MAX` for none (#107).
    reactive: u32,
    tables: ToneTablesPush,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SharpenPush {
    resolved: u32,
    width: u32,
    height: u32,
    /// RCAS's scale, 2^-stops.
    sharpness: f32,
    curve: u32,
    bloom: u32,
    bloom_strength: f32,
    pad: u32,
    output: OutputPush,
    tables: ToneTablesPush,
}

/// A fullscreen pass of `taa.slang`: its `entry` writing `color_formats`.
fn taa_pipeline<P>(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    entry: &str,
    color_formats: &[vk::Format],
    name: &str,
) -> Result<Pipeline> {
    let vertex = device.create_shader_module(
        &shaders.compile("taa.slang", "vert_main", ShaderStage::Vertex)?,
        "taa vs",
    )?;
    let fragment = device.create_shader_module(
        &shaders.compile("taa.slang", entry, ShaderStage::Fragment)?,
        name,
    )?;
    let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
        vertex: (vertex, "vert_main"),
        fragment: (fragment, entry),
        color_formats,
        push_constant_bytes: std::mem::size_of::<P>() as u32,
        alpha_blend: false,
        depth_test: None,
        depth_write: false,
        name,
    });
    for module in [vertex, fragment] {
        device.destroy_shader_module(module);
    }
    pipeline
}

/// The resolve's pipeline, writing the history and an output image of `output_format`.
fn resolve_pipeline(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    output_format: vk::Format,
) -> Result<Pipeline> {
    taa_pipeline::<ResolvePush>(
        device,
        shaders,
        "resolve_main",
        &[HDR_FORMAT, output_format],
        "taa resolve",
    )
}

/// The sharpening pass's pipeline, writing an output image of `output_format`.
fn sharpen_pipeline(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    output_format: vk::Format,
) -> Result<Pipeline> {
    taa_pipeline::<SharpenPush>(
        device,
        shaders,
        "sharpen_main",
        &[output_format],
        "taa sharpen",
    )
}

fn create_history(device: &Arc<Device>, extent: vk::Extent2D) -> Result<[GraphImage; 2]> {
    let make = |name: &str| {
        GraphImage::new(
            device,
            ImageDesc {
                width: extent.width,
                height: extent.height,
                format: HDR_FORMAT,
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name,
            },
        )
    };
    Ok([make("taa history 0")?, make("taa history 1")?])
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
    blend: f32,
    /// This frame's exposure over the one the history was stored with.
    history_scale: f32,
    written: usize,
}

/// The temporal anti-aliasing passes and their histories.
pub struct Taa {
    device: Arc<Device>,
    pipeline_motion: Pipeline,
    pipeline_resolve: Pipeline,
    /// The resolve writing the history alone, and the pass that shows it sharpened.
    pipeline_history: Pipeline,
    pipeline_sharpen: Pipeline,
    history: [GraphImage; 2],
    extent: vk::Extent2D,
    frame_index: u64,
    previous_view_proj: Option<DMat4>,
    /// Where the camera stood for `previous_view_proj` (issue #93).
    previous_camera: Option<CellPos>,
    previous_exposure: f32,
    reset: bool,
    output_format: vk::Format,
    hdr: HdrOutput,
    tables: ToneTables,
    /// Steady-state share of the current frame (0.1 is a typical TAA; 1 disables the history).
    pub blend: f32,
    /// When false, frames are drawn without jitter and resolved without history: the passes
    /// still run (so the output path is identical) but the image is the plain scene.
    pub enabled: bool,
    /// How much of the shown image is bloom when a bloom chain is passed to
    /// [`Taa::resolve`] (0.04 by default).
    pub bloom_strength: f32,
    /// Length of the jitter sequence ([`JITTER_PHASES`] at native resolution, more when an
    /// upscaler draws below it: [`jitter_phases`]).
    pub jitter_phases: u32,
    /// The display image sharpened by RCAS this many stops below its strongest (D-045), while
    /// TAA is on; `None` shows the resolve as it is.
    pub sharpen: Option<f32>,
}

impl Taa {
    /// Compiles the passes and creates the histories for `extent`. The resolve writes the
    /// history and an output image of `output_format` (the swapchain's) in one pass.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
        output_format: vk::Format,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("taa.slang", "vert_main", ShaderStage::Vertex)?,
            "taa vs",
        )?;
        let motion = device.create_shader_module(
            &shaders.compile("taa.slang", "motion_main", ShaderStage::Fragment)?,
            "taa motion",
        )?;
        let pipeline_motion = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (motion, "motion_main"),
            color_formats: &[MOTION_FORMAT],
            push_constant_bytes: std::mem::size_of::<MotionPush>() as u32,
            alpha_blend: false,
            depth_test: None,
            depth_write: false,
            name: "taa motion",
        })?;
        for module in [vertex, motion] {
            device.destroy_shader_module(module);
        }
        let pipeline_resolve = resolve_pipeline(device, shaders, output_format)?;
        let pipeline_history = taa_pipeline::<ResolvePush>(
            device,
            shaders,
            "resolve_history_main",
            &[HDR_FORMAT],
            "taa resolve history",
        )?;
        let pipeline_sharpen = sharpen_pipeline(device, shaders, output_format)?;
        let history = create_history(device, extent)?;
        let mut tables = ToneTables::new(device)?;
        let hdr = HdrOutput::default();
        if OutputEncoding::for_format(output_format).is_hdr() {
            tables.set_hdr(hdr.preset)?;
        }
        Ok(Self {
            device: Arc::clone(device),
            pipeline_motion,
            pipeline_resolve,
            pipeline_history,
            pipeline_sharpen,
            history,
            extent,
            frame_index: 0,
            previous_view_proj: None,
            previous_camera: None,
            previous_exposure: 0.0,
            reset: true,
            output_format,
            hdr,
            tables,
            blend: 0.1,
            enabled: true,
            bloom_strength: 0.04,
            jitter_phases: JITTER_PHASES,
            sharpen: None,
        })
    }

    /// Follows the output (issue #94): a new target format recompiles the resolve (call it
    /// while no frame uses the old one, as after a swapchain's recreation), an HDR one bakes
    /// its preset's table.
    pub fn set_output(
        &mut self,
        shaders: &ShaderCompiler,
        format: vk::Format,
        hdr: HdrOutput,
    ) -> Result<()> {
        if format != self.output_format {
            self.pipeline_resolve = resolve_pipeline(&self.device, shaders, format)?;
            self.pipeline_sharpen = sharpen_pipeline(&self.device, shaders, format)?;
            self.output_format = format;
        }
        if OutputEncoding::for_format(format).is_hdr() {
            self.tables.set_hdr(hdr.preset)?;
        }
        self.hdr = hdr;
        Ok(())
    }

    /// Recreates the histories (device idle) and restarts the history.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.history = create_history(&self.device, extent)?;
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

    /// Frames resolved so far (the jitter phase is derived from it).
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// The unjittered world-to-clip of the previous resolved frame, if any.
    pub fn previous_view_proj(&self) -> Option<DMat4> {
        self.previous_view_proj
    }

    /// Starts a frame: declares the HDR colour target and returns the jittered projection to
    /// draw with. `view_proj` is the *unjittered* camera-relative-to-clip of this frame and
    /// `camera` where the camera stands (the reprojection into the previous frame is derived
    /// from them and the previous frame's: the previous clip of a point is its clip in the
    /// previous camera's frame, which sits `camera − previous camera` away, issue #93);
    /// `exposure` is the frame's pre-exposure (the history is rescaled when it changes).
    pub fn begin<'f>(
        &mut self,
        graph: &mut FrameGraph<'f>,
        projection: Mat4,
        view_proj: Mat4,
        camera: CellPos,
        exposure: f32,
    ) -> TaaFrame {
        let history_scale = if self.reset || self.previous_exposure <= 0.0 {
            1.0
        } else {
            exposure / self.previous_exposure
        };
        self.previous_exposure = exposure;
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
        let blend = if self.reset || !self.enabled {
            1.0
        } else {
            self.blend
        };
        let written = (self.frame_index % 2) as usize;
        self.previous_view_proj = Some(current);
        self.reset = false;
        self.frame_index += 1;
        let color = graph.transient(TransientDesc {
            name: "taa scene color",
            width: extent.width,
            height: extent.height,
            format: HDR_FORMAT,
            // Written by the visibility resolve (storage) and the sky (attachment), sampled
            // by the resolve passes.
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
            blend,
            history_scale,
            written,
        }
    }

    /// Declares the pass "temporal/motion vectors": for every pixel of the frame drawn with
    /// `depth` (the depth buffer the scene was drawn with), the offset in UV to where it was in
    /// the previous frame, without jitter (camera motion; nothing in the scene moves yet). The
    /// TAA resolve and DLSS read it.
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
            // Storage: the movers' motion is written over the camera's (#79).
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let pipeline_motion = &self.pipeline_motion;
        graph
            .pass("temporal/motion vectors")
            .image(depth, ImageAccess::Sampled(S::FRAGMENT_SHADER))
            .image(motion, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                fullscreen_pass(
                    commands,
                    &[resources.view(motion)],
                    extent,
                    pipeline_motion,
                    &MotionPush {
                        previous_from_current: frame.previous_from_current.to_cols_array(),
                        jitter: frame.jitter.to_array(),
                        depth: resources.sampled(depth).0,
                        width: extent.width,
                        height: extent.height,
                    },
                );
                Ok(())
            });
        motion
    }

    /// Declares the pass that resolves the frame drawn into `frame.color` with `depth` and
    /// `motion` (from [`Taa::motion_vectors`]) into the next history and, through `curve`,
    /// into `output` at once; with [`Taa::sharpen`], into `output` by a sharpening pass after
    /// it. Where `reactive` (an R8 mask, #107: the splashes' coverage) is
    /// set, a pixel takes at least that share of this frame, so what moves without motion
    /// vectors is not smeared. Returns the history it wrote.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        frame: &TaaFrame,
        depth: ImageHandle,
        motion: ImageHandle,
        output: ImageHandle,
        curve: Tonemap,
        bloom: Option<ImageHandle>,
        reactive: Option<ImageHandle>,
    ) -> ImageHandle {
        let encoding = OutputPush::new(
            OutputEncoding::for_format(self.output_format),
            &self.hdr,
            self.frame_index as u32,
        );
        let bloom_strength = self.bloom_strength;
        let tables = self.tables.push();
        use vk::PipelineStageFlags2 as S;
        let frame = *frame;
        let extent = frame.extent;
        let history_written = graph.import(&self.history[frame.written]);
        let history_read = graph.import(&self.history[1 - frame.written]);
        // Sharpened, the resolve writes the history alone and the next pass shows it.
        let sharpen = self.sharpen.filter(|_| self.enabled);
        let shown = if sharpen.is_some() {
            None
        } else {
            Some(output)
        };
        let pipeline_resolve = if sharpen.is_some() {
            &self.pipeline_history
        } else {
            &self.pipeline_resolve
        };
        let mut pass = graph
            .pass("temporal/TAA resolve")
            .image(frame.color, ImageAccess::Sampled(S::FRAGMENT_SHADER))
            .image(motion, ImageAccess::Sampled(S::FRAGMENT_SHADER))
            .image(depth, ImageAccess::Sampled(S::FRAGMENT_SHADER))
            .image(history_read, ImageAccess::Sampled(S::FRAGMENT_SHADER))
            .image(history_written, ImageAccess::ColorAttachment);
        if let Some(output) = shown {
            pass = pass.image(output, ImageAccess::ColorAttachment);
            if let Some(bloom) = bloom {
                pass = pass.image(bloom, ImageAccess::Sampled(S::FRAGMENT_SHADER));
            }
        }
        if let Some(reactive) = reactive {
            pass = pass.image(reactive, ImageAccess::Sampled(S::FRAGMENT_SHADER));
        }
        pass.run(move |resources, commands| {
            let mut targets = vec![resources.view(history_written)];
            targets.extend(shown.map(|o| resources.view(o)));
            fullscreen_pass(
                commands,
                &targets,
                extent,
                pipeline_resolve,
                &ResolvePush {
                    color: resources.sampled(frame.color).0,
                    motion: resources.sampled(motion).0,
                    depth: resources.sampled(depth).0,
                    history: resources.sampled(history_read).0,
                    width: extent.width,
                    height: extent.height,
                    jitter: frame.jitter.to_array(),
                    blend: frame.blend,
                    history_scale: frame.history_scale,
                    curve: curve.index(),
                    bloom: bloom
                        .filter(|_| shown.is_some())
                        .map_or(u32::MAX, |b| resources.sampled(b).0),
                    bloom_strength,
                    output: encoding,
                    reactive: reactive.map_or(u32::MAX, |r| resources.sampled(r).0),
                    tables,
                },
            );
            Ok(())
        });
        if let Some(stops) = sharpen {
            let pipeline_sharpen = &self.pipeline_sharpen;
            let mut pass = graph
                .pass("temporal/sharpen")
                .image(history_written, ImageAccess::Sampled(S::FRAGMENT_SHADER))
                .image(output, ImageAccess::ColorAttachment);
            if let Some(bloom) = bloom {
                pass = pass.image(bloom, ImageAccess::Sampled(S::FRAGMENT_SHADER));
            }
            pass.run(move |resources, commands| {
                fullscreen_pass(
                    commands,
                    &[resources.view(output)],
                    extent,
                    pipeline_sharpen,
                    &SharpenPush {
                        resolved: resources.sampled(history_written).0,
                        width: extent.width,
                        height: extent.height,
                        sharpness: (-stops).exp2(),
                        curve: curve.index(),
                        bloom: bloom.map_or(u32::MAX, |b| resources.sampled(b).0),
                        bloom_strength,
                        pad: 0,
                        output: encoding,
                        tables,
                    },
                );
                Ok(())
            });
        }
        history_written
    }
}

fn fullscreen_pass<P: Pod>(
    commands: &Commands<'_>,
    targets: &[vk::ImageView],
    extent: vk::Extent2D,
    pipeline: &Pipeline,
    push: &P,
) {
    let color: Vec<_> = targets
        .iter()
        .map(|&target| {
            vk::RenderingAttachmentInfo::default()
                .image_view(target)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
        })
        .collect();
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

    #[test]
    fn the_sharpening_push_matches_the_shader() {
        // `SharpenPush` in taa.slang: eight words, `Output`, then `ToneTables` (its pointers on
        // 8 bytes).
        assert_eq!(std::mem::size_of::<SharpenPush>(), 32 + 16 + 32);
        assert_eq!(std::mem::offset_of!(SharpenPush, tables), 48);
    }
}
