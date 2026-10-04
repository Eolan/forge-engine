//! AMD's FidelityFX shadow denoiser, ported to Slang (`shaders/ffx_shadows.slang`, MIT): the
//! sun's soft shadows denoised where NVIDIA's NRD is absent (#173, D-049).
//!
//! It reads the same rays as SIGMA ([`crate::MeshletRenderer::trace_sun_shadow`], here with the
//! first hit: it needs no distance) and writes the same output, the visibility's square root, so
//! the resolve reads either alike ([`crate::AmbientLight::sun_shadow`]). Its passes:
//! - `shadow/FFX pack`: the rays' results, one bit a pixel, a word per 8×4 tile;
//! - `shadow/FFX classify`: tiles whose surroundings are all lit or all shadowed are skipped;
//!   elsewhere the history is reprojected, clamped to a 17×17 neighbourhood's mean ± half its
//!   deviation, and blended, with the mean and variance kept over the frames;
//! - `shadow/FFX filter 1`, `2`, `3`: three edge-avoiding à-trous passes, steps 1, 2 and 4, whose
//!   width follows the variance (not the occluder's distance, D-049). The first one's result is
//!   the next frame's history.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation,
    Pipeline, Result, ShaderCompiler, ShaderStage, TransientBufferDesc, TransientDesc, vk,
};
use glam::{Mat4, Vec2};

/// The format of the denoised shadow FFX writes: its square root, as SIGMA's.
pub const FFX_SHADOW_FORMAT: vk::Format = vk::Format::R32_SFLOAT;

/// Mirrors `FfxFrame` in `ffx_shadows.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FfxFrameBlock {
    view_x: [f32; 4],
    view_y: [f32; 4],
    view_z: [f32; 4],
    previous_z: [f32; 4],
    unproject: [f32; 4],
    width: u32,
    height: u32,
    reset: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<FfxFrameBlock>() == 96);

/// Mirrors `PackPush`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PackPush {
    mask: u64,
    penumbra: u32,
    width: u32,
    height: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<PackPush>() == 24);

/// Mirrors `ClassifyPush`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ClassifyPush {
    frame: u64,
    mask: u64,
    meta: u64,
    view_z: u32,
    normal: u32,
    motion: u32,
    previous_view_z: u32,
    previous_moments: u32,
    history: u32,
    reprojection: u32,
    moments: u32,
    view_z_out: u32,
    pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<ClassifyPush>() == 72);

/// Mirrors `FilterPush`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FilterPush {
    frame: u64,
    meta: u64,
    input: u32,
    view_z: u32,
    normal: u32,
    output: u32,
    step: u32,
    last: u32,
    pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<FilterPush>() == 48);

/// The camera of one frame, as the denoiser reprojects with it (from
/// [`crate::SunShadowDenoiser::frame`]).
#[derive(Clone, Copy, Debug)]
pub struct FfxFrame {
    /// Camera-relative world to view: the camera's rotation.
    pub view: Mat4,
    /// The previous frame's view from this frame's camera-relative world (its step included).
    pub world_to_view_prev: Mat4,
    /// View to clip, without jitter.
    pub projection: Mat4,
    /// The frame's jitter in pixels, as the projection moved the scene (x right, y down).
    pub jitter: Vec2,
    /// No history (the first frame, a cut).
    pub reset: bool,
}

/// The images FFX reads: the trace's ([`crate::SunShadowRays`]) and TAA's motion vectors.
#[derive(Clone, Copy, Debug)]
pub struct FfxImages {
    /// The penumbra's radius, `65504` where the ray reached the sun.
    pub penumbra: ImageHandle,
    /// The world normal (xyz).
    pub normal: ImageHandle,
    /// The view depth.
    pub view_z: ImageHandle,
    /// The offset in UV to where each pixel was in the previous frame.
    pub motion: ImageHandle,
}

/// The persistent images of one frame size.
struct History {
    extent: vk::Extent2D,
    /// Mean, M2 and sample count, written and read in turn.
    moments: [GraphImage; 2],
    /// The view depth, written and read in turn.
    view_z: [GraphImage; 2],
    /// The first filter pass's result: the next frame's history.
    filtered: GraphImage,
}

impl History {
    fn new(device: &Arc<Device>, extent: vk::Extent2D) -> Result<Self> {
        let image = |name: &'static str, format: vk::Format| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: extent.width,
                    height: extent.height,
                    format,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        Ok(Self {
            extent,
            moments: [
                image("ffx shadow moments 0", vk::Format::R16G16B16A16_SFLOAT)?,
                image("ffx shadow moments 1", vk::Format::R16G16B16A16_SFLOAT)?,
            ],
            view_z: [
                image("ffx shadow view z 0", vk::Format::R32_SFLOAT)?,
                image("ffx shadow view z 1", vk::Format::R32_SFLOAT)?,
            ],
            filtered: image("ffx shadow history", vk::Format::R16G16_SFLOAT)?,
        })
    }
}

/// The denoiser's pipelines and history (see the module's documentation).
pub struct FfxShadows {
    device: Arc<Device>,
    pipeline_pack: Pipeline,
    pipeline_classify: Pipeline,
    pipeline_filter: Pipeline,
    history: History,
    /// The per-frame camera blocks, one per frame slot.
    frames: Vec<Buffer>,
    /// The moments and depth this frame writes (the other pair is the previous frame's).
    written: usize,
}

fn compute<P>(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    entry: &str,
    name: &str,
) -> Result<Pipeline> {
    let module = device.create_shader_module(
        &shaders.compile("ffx_shadows.slang", entry, ShaderStage::Compute)?,
        name,
    )?;
    let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
        shader: (module, entry),
        push_constant_bytes: std::mem::size_of::<P>() as u32,
        name,
    });
    device.destroy_shader_module(module);
    pipeline
}

impl FfxShadows {
    /// Builds the denoiser for frames of `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<FfxFrameBlock>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("ffx shadow frame {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            device: Arc::clone(device),
            pipeline_pack: compute::<PackPush>(device, shaders, "pack_main", "ffx shadow pack")?,
            pipeline_classify: compute::<ClassifyPush>(
                device,
                shaders,
                "classify_main",
                "ffx shadow classify",
            )?,
            pipeline_filter: compute::<FilterPush>(
                device,
                shaders,
                "filter_main",
                "ffx shadow filter",
            )?,
            history: History::new(device, extent)?,
            frames,
            written: 0,
        })
    }

    /// Recreates the history for a new frame size (the GPU done with the old one's frames).
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        if self.history.extent != extent {
            self.history = History::new(&self.device, extent)?;
        }
        Ok(())
    }

    /// Turns the history over: call once a frame, before [`FfxShadows::denoise`].
    pub fn advance(&mut self) {
        self.written = 1 - self.written;
    }

    /// Declares the denoiser's passes over `images` and returns the denoised shadow
    /// ([`FFX_SHADOW_FORMAT`], its square root).
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        images: FfxImages,
        frame: &FfxFrame,
    ) -> ImageHandle {
        use vk::PipelineStageFlags2 as S;
        let extent = self.history.extent;
        let (width, height) = (extent.width, extent.height);
        let jitter_ndc = Vec2::new(
            2.0 * frame.jitter.x / width as f32,
            -2.0 * frame.jitter.y / height as f32,
        );
        let block = FfxFrameBlock {
            view_x: frame.view.row(0).to_array(),
            view_y: frame.view.row(1).to_array(),
            view_z: frame.view.row(2).to_array(),
            previous_z: frame.world_to_view_prev.row(2).to_array(),
            unproject: [
                1.0 / frame.projection.x_axis.x,
                1.0 / frame.projection.y_axis.y,
                jitter_ndc.x,
                jitter_ndc.y,
            ],
            width,
            height,
            reset: frame.reset as u32,
            pad: 0,
        };
        let frame_buffer = &self.frames[slot.index];
        frame_buffer.write(0, &[block]);
        let frame_address = frame_buffer.address();

        let mask_tiles = (width.div_ceil(8) * height.div_ceil(4)) as u64;
        let meta_tiles = (width.div_ceil(8) * height.div_ceil(8)) as u64;
        let mask = graph.transient_buffer(TransientBufferDesc {
            name: "ffx shadow mask",
            size: mask_tiles * 4,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
        });
        let meta = graph.transient_buffer(TransientBufferDesc {
            name: "ffx shadow tiles",
            size: meta_tiles * 4,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
        });
        let image = |name: &'static str, format: vk::Format| TransientDesc {
            name,
            width,
            height,
            format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let reprojection =
            graph.transient(image("ffx shadow reprojected", vk::Format::R16G16_SFLOAT));
        let filtered_2 = graph.transient(image("ffx shadow filtered", vk::Format::R16G16_SFLOAT));
        let output = graph.transient(image("ffx shadow", FFX_SHADOW_FORMAT));
        let moments_written = graph.import(&self.history.moments[self.written]);
        let moments_read = graph.import(&self.history.moments[1 - self.written]);
        let view_z_written = graph.import(&self.history.view_z[self.written]);
        let view_z_read = graph.import(&self.history.view_z[1 - self.written]);
        let filtered = graph.import(&self.history.filtered);

        let pack = &self.pipeline_pack;
        graph
            .pass("shadow/FFX pack")
            .image(images.penumbra, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .buffer(mask, BufferAccess::ShaderWrite(S::COMPUTE_SHADER))
            .run(move |resources, commands| {
                commands.bind_pipeline(pack);
                commands.push_constants(
                    pack,
                    &PackPush {
                        mask: resources.buffer_address(mask),
                        penumbra: resources.sampled(images.penumbra).0,
                        width,
                        height,
                        pad: 0,
                    },
                );
                commands.dispatch(
                    width.div_ceil(8).div_ceil(8),
                    height.div_ceil(4).div_ceil(8),
                    1,
                );
                Ok(())
            });

        let classify = &self.pipeline_classify;
        graph
            .pass("shadow/FFX classify")
            .buffer(mask, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
            .buffer(meta, BufferAccess::ShaderWrite(S::COMPUTE_SHADER))
            .image(images.view_z, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(images.normal, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(images.motion, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(view_z_read, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(moments_read, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(filtered, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(reprojection, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
            .image(
                moments_written,
                ImageAccess::StorageWrite(S::COMPUTE_SHADER),
            )
            .image(view_z_written, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
            .run(move |resources, commands| {
                commands.bind_pipeline(classify);
                commands.push_constants(
                    classify,
                    &ClassifyPush {
                        frame: frame_address,
                        mask: resources.buffer_address(mask),
                        meta: resources.buffer_address(meta),
                        view_z: resources.sampled(images.view_z).0,
                        normal: resources.sampled(images.normal).0,
                        motion: resources.sampled(images.motion).0,
                        previous_view_z: resources.sampled(view_z_read).0,
                        previous_moments: resources.sampled(moments_read).0,
                        history: resources.sampled(filtered).0,
                        reprojection: resources.storage(reprojection, 0).0,
                        moments: resources.storage(moments_written, 0).0,
                        view_z_out: resources.storage(view_z_written, 0).0,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(width.div_ceil(8), height.div_ceil(8), 1);
                Ok(())
            });

        // Steps 1, 2 and 4; the first writes the next frame's history.
        let passes = [
            ("shadow/FFX filter 1", reprojection, filtered, 1, false),
            ("shadow/FFX filter 2", filtered, filtered_2, 2, false),
            ("shadow/FFX filter 3", filtered_2, output, 4, true),
        ];
        let filter = &self.pipeline_filter;
        for (name, input, written, step, last) in passes {
            graph
                .pass(name)
                .buffer(meta, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
                .image(input, ImageAccess::Sampled(S::COMPUTE_SHADER))
                .image(images.view_z, ImageAccess::Sampled(S::COMPUTE_SHADER))
                .image(images.normal, ImageAccess::Sampled(S::COMPUTE_SHADER))
                .image(written, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
                .run(move |resources, commands| {
                    commands.bind_pipeline(filter);
                    commands.push_constants(
                        filter,
                        &FilterPush {
                            frame: frame_address,
                            meta: resources.buffer_address(meta),
                            input: resources.sampled(input).0,
                            view_z: resources.sampled(images.view_z).0,
                            normal: resources.sampled(images.normal).0,
                            output: resources.storage(written, 0).0,
                            step,
                            last: last as u32,
                            pad: [0; 2],
                        },
                    );
                    commands.dispatch(width.div_ceil(8), height.div_ceil(8), 1);
                    Ok(())
                });
        }
        output
    }
}
