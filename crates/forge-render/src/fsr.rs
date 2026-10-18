//! AMD's FSR 3.1 upscaler in the frame (D-070): the anti-aliasing and the upscaler on every GPU
//! where DLSS is not used; it replaced Forge's TAA. The jittered HDR frame, its depth, its
//! motion and the reactive mask go in; an HDR image at the output size comes out, which the
//! display pass then takes through the tone curve, as it takes DLSS's ([`crate::DlssUpscaler`]).
//!
//! AMD's shaders, compiled by Forge: the eight passes of FSR 3.1.4 (FidelityFX SDK v1.1.4, MIT)
//! are AMD's headers in `shaders/third-party/fidelityfx/`, reached through Forge's bridge
//! (`shaders/fsr_bridge.slang`), one file a pass (`shaders/fsr_*.slang`). This module is AMD's
//! host side (`ffx_fsr3upscaler.cpp`) in Rust: the images, the constants each frame, and the
//! passes in the render graph, where each gets its barriers and its profiler zone.
//!
//! Forge's one change to AMD's code (the owner's choice, D-070): the history is clamped to the
//! neighbourhood's min/max box, not to its mean ± deviation (`docs/research/fsr-3-1.md`).
//!
//! Each frame, in the order AMD's dispatch runs them:
//! - "temporal/FSR clear": what AMD clears (the reconstructed previous depth, the downsampler's
//!   counter and pyramid; a reset's accumulation and frame info);
//! - "temporal/FSR inputs": dilated depth and motion, the previous frame's depth reconstructed,
//!   the luma;
//! - "temporal/FSR luma pyramid" and "temporal/FSR shading pyramid": the single-pass downsampler;
//! - "temporal/FSR shading change", "temporal/FSR reactivity" (masks, locks, accumulation),
//!   "temporal/FSR luma instability";
//! - "temporal/FSR accumulate": the upsampled colour into the history, and the output;
//! - "temporal/FSR sharpen" (RCAS), with [`FsrUpscaler::sharpness`].
//!
//! A second upscaler's passes take another group ([`FsrUpscaler::group`]): the scene behind the
//! tank's water is "liquid/scene FSR …".

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};

use crate::taa::{HDR_FORMAT, TaaFrame};
use crate::upscale::UpscaleCamera;

/// How much smaller FSR draws than it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FsrMode {
    /// At the output's size: FSR as the anti-aliasing (AMD's "native AA").
    #[default]
    NativeAa,
    /// 1.5 times smaller.
    Quality,
    /// 1.7 times smaller.
    Balanced,
    /// 2 times smaller.
    Performance,
    /// 3 times smaller.
    UltraPerformance,
}

impl FsrMode {
    /// Every mode, from the largest render down.
    pub const ALL: [Self; 5] = [
        Self::NativeAa,
        Self::Quality,
        Self::Balanced,
        Self::Performance,
        Self::UltraPerformance,
    ];

    /// The output's size over the render's (AMD's `ffxFsr3UpscalerGetUpscaleRatioFromQualityMode`).
    pub fn ratio(self) -> f32 {
        match self {
            Self::NativeAa => 1.0,
            Self::Quality => 1.5,
            Self::Balanced => 1.7,
            Self::Performance => 2.0,
            Self::UltraPerformance => 3.0,
        }
    }

    /// Short name (`r.fsr.mode`'s value; the asteroids' `--upscaler fsr-NAME`).
    pub fn name(self) -> &'static str {
        match self {
            Self::NativeAa => "native",
            Self::Quality => "quality",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::UltraPerformance => "ultra-performance",
        }
    }

    /// Name for the screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::NativeAa => "FSR",
            Self::Quality => "FSR Quality",
            Self::Balanced => "FSR Balanced",
            Self::Performance => "FSR Performance",
            Self::UltraPerformance => "FSR Ultra Performance",
        }
    }

    /// The mode with `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

/// Where an upscaler's passes show in the graph and the profiler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FsrGroup {
    /// "temporal/FSR …": the frame's anti-aliasing.
    #[default]
    Temporal,
    /// "liquid/scene FSR …": the scene behind the tank's water (#156), anti-aliased before the
    /// water bends it.
    Liquid,
}

impl FsrGroup {
    /// The passes' names, in the order the frame runs them (the module's list).
    fn labels(self) -> &'static [&'static str; 9] {
        match self {
            Self::Temporal => &[
                "temporal/FSR clear",
                "temporal/FSR inputs",
                "temporal/FSR luma pyramid",
                "temporal/FSR shading pyramid",
                "temporal/FSR shading change",
                "temporal/FSR reactivity",
                "temporal/FSR luma instability",
                "temporal/FSR accumulate",
                "temporal/FSR sharpen",
            ],
            Self::Liquid => &[
                "liquid/scene FSR clear",
                "liquid/scene FSR inputs",
                "liquid/scene FSR luma pyramid",
                "liquid/scene FSR shading pyramid",
                "liquid/scene FSR shading change",
                "liquid/scene FSR reactivity",
                "liquid/scene FSR luma instability",
                "liquid/scene FSR accumulate",
                "liquid/scene FSR sharpen",
            ],
        }
    }
}

/// AMD's three constant buffers, one block of the frame (`FsrFrame` of `fsr_bridge.slang`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct FsrFrame {
    render_size: [i32; 2],
    previous_render_size: [i32; 2],
    upscale_size: [i32; 2],
    previous_upscale_size: [i32; 2],
    max_render_size: [i32; 2],
    max_upscale_size: [i32; 2],
    device_to_view_depth: [f32; 4],
    jitter: [f32; 2],
    previous_jitter: [f32; 2],
    motion_vector_scale: [f32; 2],
    downscale_factor: [f32; 2],
    motion_vector_jitter_cancellation: [f32; 2],
    tan_half_fov: f32,
    jitter_sequence_length: f32,
    delta_time: f32,
    delta_pre_exposure: f32,
    view_space_to_meters: f32,
    frame_index: f32,
    velocity_factor: f32,
    reactiveness_scale: f32,
    shading_change_scale: f32,
    accumulation_added_per_frame: f32,
    min_disocclusion_accumulation: f32,
    spd_mips: u32,
    spd_work_groups: u32,
    pad: u32,
    spd_work_group_offset: [u32; 2],
    spd_render_size: [u32; 2],
    rcas_config: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<FsrFrame>() == 192);

/// A pass's push constants: the frame's block, then the bindless handles at AMD's binding
/// numbers of the pass (`FSR3UPSCALER_BIND_SRV_*` index `srv`, `_UAV_*` index `uav`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct FsrPush {
    frame: u64,
    srv: [u32; 12],
    uav: [u32; 12],
}

/// `fsr_clear.slang`'s push constants.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ClearPush {
    value: [f32; 4],
    image: u32,
    format: u32,
    width: u32,
    height: u32,
}

// `fsr_clear.slang`'s formats.
const CLEAR_R8: u32 = 0;
const CLEAR_R16F: u32 = 1;
const CLEAR_RG16F: u32 = 2;
const CLEAR_RGBA32F: u32 = 3;
const CLEAR_R32UI: u32 = 4;
const CLEAR_RGBA16F: u32 = 5;

/// The mips of the downsampler's pyramid AMD's passes write (levels 0 to 5).
const SPD_LEVELS: u32 = 6;

/// AMD's defaults (`fsr3upscalerCreate`).
const ACCUMULATION_ADDED_PER_FRAME: f32 = 1.0 / 3.0;
const MIN_DISOCCLUSION_ACCUMULATION: f32 = -1.0 / 3.0;

/// The compiled passes.
struct FsrPipelines {
    clear: Pipeline,
    prepare_inputs: Pipeline,
    luma_pyramid: Pipeline,
    shading_change_pyramid: Pipeline,
    shading_change: Pipeline,
    prepare_reactivity: Pipeline,
    luma_instability: Pipeline,
    accumulate: Pipeline,
    accumulate_sharpen: Pipeline,
    rcas: Pipeline,
}

/// The shader files and entry points of [`FsrPipelines`], in the order of its fields.
const PASSES: [(&str, &str); 10] = [
    ("fsr_clear.slang", "clear_main"),
    ("fsr_prepare_inputs.slang", "prepare_inputs_main"),
    ("fsr_luma_pyramid.slang", "luma_pyramid_main"),
    (
        "fsr_shading_change_pyramid.slang",
        "shading_change_pyramid_main",
    ),
    ("fsr_shading_change.slang", "shading_change_main"),
    ("fsr_prepare_reactivity.slang", "prepare_reactivity_main"),
    ("fsr_luma_instability.slang", "luma_instability_main"),
    ("fsr_accumulate.slang", "accumulate_main"),
    ("fsr_accumulate_sharpen.slang", "accumulate_main"),
    ("fsr_rcas.slang", "rcas_main"),
];

/// The images FSR keeps between frames, at the render's size or the output's.
struct FsrImages {
    render: vk::Extent2D,
    output: vk::Extent2D,
    /// How long each output pixel has accumulated (R8), read one frame, written the next.
    accumulation: [GraphImage; 2],
    /// This frame's luma and the last one's (R16F).
    luma: [GraphImage; 2],
    /// The luma's last frames, for the instability (RGBA16F).
    luma_history: [GraphImage; 2],
    /// The upsampled colour and its weight (RGBA16F at the output's size).
    upscaled: [GraphImage; 2],
    /// The scene's average luma and its exposure (RGBA32F, 1 × 1).
    frame_info: GraphImage,
    /// The single-pass downsampler's counter (R32UI, 1 × 1).
    spd_atomic: GraphImage,
    /// FSR's output (RGBA16F at the output's size).
    output_image: GraphImage,
    /// The locks the reactivity pass finds on thin features (R8 at the output's size): the
    /// accumulation reads them everywhere and zeroes them for the next frame.
    new_locks: GraphImage,
}

/// FSR 3.1's upscaler: its passes, its images and the state AMD's host code keeps.
pub struct FsrUpscaler {
    device: Arc<Device>,
    pipelines: FsrPipelines,
    images: FsrImages,
    /// 1 × 1 R32F holding 1: the scene is pre-exposed, as for DLSS.
    exposure: GraphImage,
    /// 1 × 1 R8 holding 0: no reactive or transparency mask.
    no_mask: GraphImage,
    frames: Vec<Buffer>,
    mode: FsrMode,
    /// RCAS after the accumulation, 0 (none) to 1 (AMD's strongest), or none at all.
    pub sharpness: Option<f32>,
    /// Where its passes show (the frame's anti-aliasing by default).
    pub group: FsrGroup,
    /// The constants kept from the last frame (sizes, jitter, the jitter's length, the index).
    last: FsrFrame,
    /// Which of each pair this frame writes.
    parity: usize,
    /// The next frame starts a new history (first frame, new mode or size).
    reset: bool,
    /// Every image is new: the luma and the accumulation are cleared too (AMD's first frame).
    first: bool,
    pre_exposure: f32,
}

impl FsrUpscaler {
    /// Compiles FSR's passes and makes its images for an output of `output_extent` in `mode`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        mode: FsrMode,
        output_extent: vk::Extent2D,
    ) -> Result<Self> {
        let mut pipelines = Vec::with_capacity(PASSES.len());
        for (file, entry) in PASSES {
            let module = device
                .create_shader_module(&shaders.compile(file, entry, ShaderStage::Compute)?, file)?;
            let push_constant_bytes = if file == "fsr_clear.slang" {
                std::mem::size_of::<ClearPush>()
            } else {
                std::mem::size_of::<FsrPush>()
            } as u32;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes,
                name: file,
            });
            device.destroy_shader_module(module);
            pipelines.push(pipeline?);
        }
        let mut pipelines = pipelines.into_iter();
        let mut next = || pipelines.next().expect("one pipeline per pass");
        let pipelines = FsrPipelines {
            clear: next(),
            prepare_inputs: next(),
            luma_pyramid: next(),
            shading_change_pyramid: next(),
            shading_change: next(),
            prepare_reactivity: next(),
            luma_instability: next(),
            accumulate: next(),
            accumulate_sharpen: next(),
            rcas: next(),
        };
        let one = |format, data: &[u8], name| {
            GraphImage::uploaded(
                device,
                ImageDesc {
                    width: 1,
                    height: 1,
                    format,
                    usage: vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
                data,
            )
        };
        let exposure = one(
            vk::Format::R32_SFLOAT,
            bytemuck::bytes_of(&1.0_f32),
            "fsr exposure",
        )?;
        let no_mask = one(vk::Format::R8_UNORM, &[0], "fsr no mask")?;
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<FsrFrame>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("fsr frame {i}"),
                })
            })
            .collect::<Result<_>>()?;
        let render = render_extent(mode, output_extent);
        Ok(Self {
            device: Arc::clone(device),
            pipelines,
            images: FsrImages::new(device, render, output_extent)?,
            exposure,
            no_mask,
            frames,
            mode,
            sharpness: None,
            group: FsrGroup::Temporal,
            last: FsrFrame::default(),
            parity: 0,
            reset: true,
            first: true,
            pre_exposure: 0.0,
        })
    }

    /// The mode.
    pub fn mode(&self) -> FsrMode {
        self.mode
    }

    /// The next frame starts a new history (FSR switched back on).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// Switches to `mode` for an output of `output_extent` (the device must be idle when a size
    /// changes); the next frame starts a new history.
    pub fn configure(&mut self, mode: FsrMode, output_extent: vk::Extent2D) -> Result<()> {
        let render = render_extent(mode, output_extent);
        if render != self.images.render || output_extent != self.images.output {
            self.images = FsrImages::new(&self.device, render, output_extent)?;
            self.first = true;
        }
        self.mode = mode;
        self.reset = true;
        Ok(())
    }

    /// The size to draw at for the current mode and output.
    pub fn render_extent(&self) -> vk::Extent2D {
        self.images.render
    }

    /// Declares FSR's passes, which upscale `frame.color` (drawn with `depth`, moved by `motion`
    /// from [`crate::Taa::motion_vectors`], with an optional `reactive` mask: 1 where the history
    /// should not be trusted) into the HDR output, and returns it. `exposure` is the frame's
    /// pre-exposure, `delta_time` the seconds since the last frame.
    #[allow(clippy::too_many_arguments)]
    pub fn upscale<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: &TaaFrame,
        camera: UpscaleCamera,
        depth: ImageHandle,
        motion: ImageHandle,
        reactive: Option<ImageHandle>,
        exposure: f32,
        delta_time: f32,
    ) -> ImageHandle {
        let reset = std::mem::take(&mut self.reset) || frame.reset;
        let first = std::mem::take(&mut self.first);
        let reset = reset || first;
        let constants = self.constants(frame, camera, exposure, delta_time, reset);
        self.frames[slot.index].write(0, &[constants]);
        let frame_address = self.frames[slot.index].address();
        let parity = self.parity;
        self.parity ^= 1;
        let sharpness = self.sharpness;
        let labels = self.group.labels();

        let this: &'f Self = self;
        let images = &this.images;
        let pipelines = &this.pipelines;
        let (render, output) = (images.render, images.output);
        let half = vk::Extent2D {
            width: (render.width / 2).max(1),
            height: (render.height / 2).max(1),
        };
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let sampled = ImageAccess::Sampled(compute);
        let write = ImageAccess::StorageWrite(compute);
        let read_write = ImageAccess::StorageReadWrite(compute);
        let transient = |graph: &mut FrameGraph<'f>, name, extent: vk::Extent2D, format, levels| {
            graph.transient(TransientDesc {
                name,
                width: extent.width,
                height: extent.height,
                format,
                usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: levels,
            })
        };
        let r16f = vk::Format::R16_SFLOAT;
        let r8 = vk::Format::R8_UNORM;
        let dilated_depth = transient(
            graph,
            "fsr dilated depth",
            render,
            vk::Format::R32_SFLOAT,
            1,
        );
        let dilated_motion = transient(
            graph,
            "fsr dilated motion",
            render,
            vk::Format::R16G16_SFLOAT,
            1,
        );
        let previous_depth =
            transient(graph, "fsr previous depth", render, vk::Format::R32_UINT, 1);
        let farthest_depth = transient(graph, "fsr farthest depth", render, r16f, 1);
        let farthest_mip1 = transient(graph, "fsr farthest depth mip 1", half, r16f, 1);
        let luma_instability = transient(graph, "fsr luma instability", render, r16f, 1);
        let shading_change = transient(graph, "fsr shading change", half, r8, 1);
        let reactive_masks = transient(
            graph,
            "fsr dilated reactive",
            render,
            vk::Format::R8G8B8A8_UNORM,
            1,
        );
        let spd_levels = SPD_LEVELS.min(32 - half.width.max(half.height).leading_zeros());
        let spd_mips = transient(
            graph,
            "fsr pyramid",
            half,
            vk::Format::R16G16_SFLOAT,
            spd_levels,
        );

        let accumulation_read = graph.import(&images.accumulation[parity]);
        let accumulation_write = graph.import(&images.accumulation[parity ^ 1]);
        let luma_current = graph.import(&images.luma[parity]);
        let luma_previous = graph.import(&images.luma[parity ^ 1]);
        let luma_history_read = graph.import(&images.luma_history[parity]);
        let luma_history_write = graph.import(&images.luma_history[parity ^ 1]);
        let upscaled_read = graph.import(&images.upscaled[parity]);
        let upscaled_write = graph.import(&images.upscaled[parity ^ 1]);
        let frame_info = graph.import(&images.frame_info);
        let spd_atomic = graph.import(&images.spd_atomic);
        let output_image = graph.import(&images.output_image);
        let new_locks = graph.import(&images.new_locks);
        let exposure_image = graph.import(&this.exposure);
        let no_mask = graph.import(&this.no_mask);
        // Without a mask of its own the frame's reactive mask is the empty one, declared once.
        let has_reactive = reactive.is_some();
        let reactive = reactive.unwrap_or(no_mask);
        let color = frame.color;

        // What AMD clears: every frame the reconstructed depth (reversed: to 0), the counter and
        // the pyramid; on a reset the accumulation read and the frame info. First, every image
        // kept between frames: AMD leaves them uninitialized, and on Vulkan their memory holds
        // anything (its new locks are read everywhere and written only on thin features).
        let mut clear = graph
            .pass(labels[0])
            .image(previous_depth, write)
            .image(spd_atomic, write)
            .image(spd_mips, write);
        if reset {
            clear = clear
                .image(accumulation_read, write)
                .image(frame_info, write);
        }
        if first {
            clear = clear
                .image(accumulation_write, write)
                .image(luma_current, write)
                .image(luma_previous, write)
                .image(luma_history_read, write)
                .image(luma_history_write, write)
                .image(upscaled_read, write)
                .image(upscaled_write, write)
                .image(new_locks, write);
        }
        clear.run(move |resources, commands| {
            commands.bind_pipeline(&pipelines.clear);
            let fill = |handle, level: u32, format, value: [f32; 4], extent: vk::Extent2D| {
                commands.push_constants(
                    &pipelines.clear,
                    &ClearPush {
                        value,
                        image: resources.storage(handle, level).0,
                        format,
                        width: extent.width,
                        height: extent.height,
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
            };
            let one = vk::Extent2D {
                width: 1,
                height: 1,
            };
            fill(previous_depth, 0, CLEAR_R32UI, [0.0; 4], render);
            fill(spd_atomic, 0, CLEAR_R32UI, [0.0; 4], one);
            for level in 0..spd_levels {
                let extent = vk::Extent2D {
                    width: (half.width >> level).max(1),
                    height: (half.height >> level).max(1),
                };
                fill(spd_mips, level, CLEAR_RG16F, [0.0; 4], extent);
            }
            if reset {
                fill(accumulation_read, 0, CLEAR_R8, [0.0; 4], render);
                fill(frame_info, 0, CLEAR_RGBA32F, [-1.0, 1.0, 0.0, 0.0], one);
            }
            if first {
                fill(accumulation_write, 0, CLEAR_R8, [0.0; 4], render);
                fill(luma_current, 0, CLEAR_R16F, [0.0; 4], render);
                fill(luma_previous, 0, CLEAR_R16F, [0.0; 4], render);
                for history in [luma_history_read, luma_history_write] {
                    fill(history, 0, CLEAR_RGBA16F, [0.0; 4], render);
                }
                for upscaled in [upscaled_read, upscaled_write] {
                    fill(upscaled, 0, CLEAR_RGBA16F, [0.0; 4], output);
                }
                fill(new_locks, 0, CLEAR_R8, [0.0; 4], output);
            }
            Ok(())
        });

        let groups = |extent: vk::Extent2D, size: u32| {
            (extent.width.div_ceil(size), extent.height.div_ceil(size))
        };
        let (spd_x, spd_y) = groups(render, 64);
        let half_groups = groups(
            vk::Extent2D {
                width: render.width / 2,
                height: render.height / 2,
            },
            8,
        );
        let push = move |srv: &[u32], uav: &[u32]| {
            let mut push = FsrPush {
                frame: frame_address,
                ..FsrPush::default()
            };
            push.srv[..srv.len()].copy_from_slice(srv);
            push.uav[..uav.len()].copy_from_slice(uav);
            push
        };

        graph
            .pass(labels[1])
            .image(motion, sampled)
            .image(depth, sampled)
            .image(color, sampled)
            .image(dilated_motion, write)
            .image(dilated_depth, write)
            .image(previous_depth, read_write)
            .image(farthest_depth, write)
            .image(luma_current, write)
            .run(move |resources, commands| {
                let s = |h| resources.sampled(h).0;
                let u = |h| resources.storage(h, 0).0;
                let pipeline = &pipelines.prepare_inputs;
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &push(
                        &[s(motion), s(depth), s(color)],
                        &[
                            u(dilated_motion),
                            u(dilated_depth),
                            u(previous_depth),
                            u(farthest_depth),
                            u(luma_current),
                        ],
                    ),
                );
                let (x, y) = groups(render, 8);
                commands.dispatch(x, y, 1);
                Ok(())
            });

        let spd_storage = move |resources: &forge_gpu::Resources<'_>, first_slot: &mut [u32]| {
            for (level, slot) in first_slot.iter_mut().enumerate() {
                *slot = resources
                    .storage(spd_mips, (level as u32).min(spd_levels - 1))
                    .0;
            }
        };
        graph
            .pass(labels[2])
            .image(luma_current, sampled)
            .image(farthest_depth, sampled)
            .image(spd_atomic, read_write)
            .image(frame_info, read_write)
            .image(spd_mips, read_write)
            .image(farthest_mip1, write)
            .run(move |resources, commands| {
                let s = |h| resources.sampled(h).0;
                let u = |h| resources.storage(h, 0).0;
                let mut uav = [0; 9];
                uav[0] = u(spd_atomic);
                uav[1] = u(frame_info);
                spd_storage(resources, &mut uav[2..8]);
                uav[8] = u(farthest_mip1);
                let pipeline = &pipelines.luma_pyramid;
                commands.bind_pipeline(pipeline);
                commands
                    .push_constants(pipeline, &push(&[s(luma_current), s(farthest_depth)], &uav));
                commands.dispatch(spd_x, spd_y, 1);
                Ok(())
            });

        graph
            .pass(labels[3])
            .image(luma_current, sampled)
            .image(luma_previous, sampled)
            .image(dilated_motion, sampled)
            .image(exposure_image, sampled)
            .image(spd_atomic, read_write)
            .image(spd_mips, read_write)
            .run(move |resources, commands| {
                let s = |h| resources.sampled(h).0;
                let mut uav = [0; 7];
                uav[0] = resources.storage(spd_atomic, 0).0;
                spd_storage(resources, &mut uav[1..7]);
                let pipeline = &pipelines.shading_change_pyramid;
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &push(
                        &[
                            s(luma_current),
                            s(luma_previous),
                            s(dilated_motion),
                            s(exposure_image),
                        ],
                        &uav,
                    ),
                );
                commands.dispatch(spd_x, spd_y, 1);
                Ok(())
            });

        graph
            .pass(labels[4])
            .image(spd_mips, sampled)
            .image(shading_change, write)
            .run(move |resources, commands| {
                let pipeline = &pipelines.shading_change;
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &push(
                        &[resources.sampled(spd_mips).0],
                        &[resources.storage(shading_change, 0).0],
                    ),
                );
                commands.dispatch(half_groups.0, half_groups.1, 1);
                Ok(())
            });

        let mut reactivity = graph
            .pass(labels[5])
            .image(previous_depth, sampled)
            .image(dilated_motion, sampled)
            .image(dilated_depth, sampled)
            .image(no_mask, sampled)
            .image(accumulation_read, sampled)
            .image(shading_change, sampled)
            .image(luma_current, sampled)
            .image(exposure_image, sampled)
            .image(reactive_masks, write)
            .image(new_locks, write)
            .image(accumulation_write, write);
        if has_reactive {
            reactivity = reactivity.image(reactive, sampled);
        }
        reactivity.run(move |resources, commands| {
            let s = |h| resources.sampled(h).0;
            let u = |h| resources.storage(h, 0).0;
            let pipeline = &pipelines.prepare_reactivity;
            commands.bind_pipeline(pipeline);
            commands.push_constants(
                pipeline,
                &push(
                    &[
                        s(previous_depth),
                        s(dilated_motion),
                        s(dilated_depth),
                        s(reactive),
                        s(no_mask),
                        s(accumulation_read),
                        s(shading_change),
                        s(luma_current),
                        s(exposure_image),
                    ],
                    &[u(reactive_masks), u(new_locks), u(accumulation_write)],
                ),
            );
            let (x, y) = groups(render, 8);
            commands.dispatch(x, y, 1);
            Ok(())
        });

        graph
            .pass(labels[6])
            .image(exposure_image, sampled)
            .image(reactive_masks, sampled)
            .image(dilated_motion, sampled)
            .image(frame_info, sampled)
            .image(luma_history_read, sampled)
            .image(farthest_mip1, sampled)
            .image(luma_current, sampled)
            .image(luma_history_write, write)
            .image(luma_instability, write)
            .run(move |resources, commands| {
                let s = |h| resources.sampled(h).0;
                let u = |h| resources.storage(h, 0).0;
                let pipeline = &pipelines.luma_instability;
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &push(
                        &[
                            s(exposure_image),
                            s(reactive_masks),
                            s(dilated_motion),
                            s(frame_info),
                            s(luma_history_read),
                            s(farthest_mip1),
                            s(luma_current),
                        ],
                        &[u(luma_history_write), u(luma_instability)],
                    ),
                );
                let (x, y) = groups(render, 8);
                commands.dispatch(x, y, 1);
                Ok(())
            });

        graph
            .pass(labels[7])
            .image(exposure_image, sampled)
            .image(reactive_masks, sampled)
            .image(dilated_motion, sampled)
            .image(upscaled_read, sampled)
            .image(farthest_mip1, sampled)
            .image(luma_current, sampled)
            .image(luma_instability, sampled)
            .image(color, sampled)
            .image(upscaled_write, write)
            .image(output_image, write)
            .image(new_locks, read_write)
            .run(move |resources, commands| {
                let s = |h| resources.sampled(h).0;
                let u = |h| resources.storage(h, 0).0;
                let pipeline = if sharpness.is_some() {
                    &pipelines.accumulate_sharpen
                } else {
                    &pipelines.accumulate
                };
                commands.bind_pipeline(pipeline);
                // AMD's slot 4 is the Lanczos table, unused here.
                commands.push_constants(
                    pipeline,
                    &push(
                        &[
                            s(exposure_image),
                            s(reactive_masks),
                            s(dilated_motion),
                            s(upscaled_read),
                            0,
                            s(farthest_mip1),
                            s(luma_current),
                            s(luma_instability),
                            s(color),
                        ],
                        &[u(upscaled_write), u(output_image), u(new_locks)],
                    ),
                );
                let (x, y) = groups(output, 8);
                commands.dispatch(x, y, 1);
                Ok(())
            });

        if sharpness.is_some() {
            graph
                .pass(labels[8])
                .image(exposure_image, sampled)
                .image(upscaled_write, sampled)
                .image(output_image, write)
                .run(move |resources, commands| {
                    let pipeline = &pipelines.rcas;
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &push(
                            &[
                                resources.sampled(exposure_image).0,
                                resources.sampled(upscaled_write).0,
                            ],
                            &[resources.storage(output_image, 0).0],
                        ),
                    );
                    let (x, y) = groups(output, 16);
                    commands.dispatch(x, y, 1);
                    Ok(())
                });
        }
        output_image
    }

    /// This frame's constants, as AMD's `fsr3upscalerDispatch` computes them, and the state kept
    /// for the next.
    fn constants(
        &mut self,
        frame: &TaaFrame,
        camera: UpscaleCamera,
        exposure: f32,
        delta_time: f32,
        reset: bool,
    ) -> FsrFrame {
        let (render, output) = (self.images.render, self.images.output);
        let size = |e: vk::Extent2D| [e.width as i32, e.height as i32];
        let aspect = render.width.max(1) as f32 / render.height.max(1) as f32;
        let tan_half_y = (0.5 * camera.vertical_fov).tan();
        // Reversed, infinite depth (AMD's `setupDeviceDepthToViewSpaceDepthParams`): the view
        // depth is near / (depth + ε).
        let device_to_view_depth = [-f32::EPSILON, camera.near, tan_half_y * aspect, tan_half_y];
        let phases = jitter_phase_count(render, output) as f32;
        let jitter_sequence_length = if reset || self.last.jitter_sequence_length == 0.0 {
            phases
        } else {
            // AMD moves it one step a frame towards the new count.
            self.last.jitter_sequence_length + (phases - self.last.jitter_sequence_length).signum()
        };
        let previous_pre_exposure = self.pre_exposure;
        self.pre_exposure = if exposure > 0.0 { exposure } else { 1.0 };
        let delta_pre_exposure = if previous_pre_exposure > 0.0 {
            self.pre_exposure / previous_pre_exposure
        } else {
            1.0
        };
        let (groups, offset, mips) = spd_setup(render);
        let frame = FsrFrame {
            render_size: size(render),
            previous_render_size: self.last.render_size,
            upscale_size: size(output),
            previous_upscale_size: self.last.upscale_size,
            max_render_size: size(render),
            max_upscale_size: size(output),
            device_to_view_depth,
            jitter: frame.jitter.to_array(),
            previous_jitter: self.last.jitter,
            // Forge's motion is already in UV (the previous frame's position minus this one's).
            motion_vector_scale: [1.0, 1.0],
            downscale_factor: [
                render.width as f32 / output.width as f32,
                render.height as f32 / output.height as f32,
            ],
            motion_vector_jitter_cancellation: [0.0, 0.0],
            tan_half_fov: tan_half_y * aspect,
            jitter_sequence_length,
            delta_time: delta_time.clamp(0.0, 1.0),
            delta_pre_exposure,
            view_space_to_meters: 1.0,
            frame_index: if reset {
                0.0
            } else {
                self.last.frame_index + 1.0
            },
            velocity_factor: 1.0,
            reactiveness_scale: 1.0,
            shading_change_scale: 1.0,
            accumulation_added_per_frame: ACCUMULATION_ADDED_PER_FRAME,
            min_disocclusion_accumulation: MIN_DISOCCLUSION_ACCUMULATION,
            spd_mips: mips,
            spd_work_groups: groups[0] * groups[1],
            pad: 0,
            spd_work_group_offset: offset,
            spd_render_size: [render.width, render.height],
            rcas_config: rcas_config(self.sharpness.unwrap_or(0.0)),
        };
        self.last = frame;
        frame
    }
}

impl FsrImages {
    fn new(device: &Arc<Device>, render: vk::Extent2D, output: vk::Extent2D) -> Result<Self> {
        let image = |extent: vk::Extent2D, format, name: &str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: extent.width,
                    height: extent.height,
                    format,
                    usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let pair = |extent, format, name: &str| -> Result<[GraphImage; 2]> {
            Ok([
                image(extent, format, &format!("{name} 1"))?,
                image(extent, format, &format!("{name} 2"))?,
            ])
        };
        let one = vk::Extent2D {
            width: 1,
            height: 1,
        };
        Ok(Self {
            render,
            output,
            accumulation: pair(render, vk::Format::R8_UNORM, "fsr accumulation")?,
            luma: pair(render, vk::Format::R16_SFLOAT, "fsr luma")?,
            luma_history: pair(render, HDR_FORMAT, "fsr luma history")?,
            upscaled: pair(output, HDR_FORMAT, "fsr upscaled")?,
            frame_info: image(one, vk::Format::R32G32B32A32_SFLOAT, "fsr frame info")?,
            spd_atomic: image(one, vk::Format::R32_UINT, "fsr pyramid counter")?,
            output_image: image(output, HDR_FORMAT, "fsr output")?,
            new_locks: image(output, vk::Format::R8_UNORM, "fsr new locks")?,
        })
    }
}

/// The render size of `mode` for an output of `output` (AMD truncates).
fn render_extent(mode: FsrMode, output: vk::Extent2D) -> vk::Extent2D {
    let ratio = mode.ratio();
    vk::Extent2D {
        width: ((output.width as f32 / ratio) as u32).max(1),
        height: ((output.height as f32 / ratio) as u32).max(1),
    }
}

/// AMD's jitter sequence length: 8 at the output's size, more as the render shrinks
/// (`ffxFsr3UpscalerGetJitterPhaseCount`, the same rule as [`crate::taa::jitter_phases`]).
fn jitter_phase_count(render: vk::Extent2D, output: vk::Extent2D) -> u32 {
    let scale = output.width as f32 / render.width.max(1) as f32;
    (8.0 * scale * scale) as u32
}

/// The single-pass downsampler's dispatch over `render` (AMD's `ffxSpdSetup` on the whole
/// image): the workgroups in x and y, the first one's offset, and the mip count.
fn spd_setup(render: vk::Extent2D) -> ([u32; 2], [u32; 2], u32) {
    let end_x = (render.width.max(1) - 1) / 64;
    let end_y = (render.height.max(1) - 1) / 64;
    let resolution = render.width.max(render.height).max(1);
    let mips = (resolution as f32).log2().floor().min(12.0) as u32;
    ([end_x + 1, end_y + 1], [0, 0], mips)
}

/// RCAS's constants for `sharpness` 0 to 1 (AMD's mapping to stops, then `FsrRcasCon`): the
/// linear sharpness as a float's bits, and as two halves.
fn rcas_config(sharpness: f32) -> [u32; 4] {
    let stops = -2.0 * sharpness.clamp(0.0, 1.0) + 2.0;
    let linear = (-stops).exp2();
    let half = u32::from(half_bits(linear));
    [linear.to_bits(), half | (half << 16), 0, 0]
}

/// `value` as an IEEE half's bits (round to nearest even), for RCAS's packed constant.
fn half_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x007f_ffff;
    if exponent <= 0 {
        return sign;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    let mut half = ((exponent as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        half += 1;
    }
    sign | half as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_shrink_as_amd_does() {
        let output = vk::Extent2D {
            width: 1920,
            height: 1080,
        };
        assert_eq!(render_extent(FsrMode::NativeAa, output), output);
        assert_eq!(
            render_extent(FsrMode::Quality, output),
            vk::Extent2D {
                width: 1280,
                height: 720
            }
        );
        assert_eq!(jitter_phase_count(output, output), 8);
        let half = render_extent(FsrMode::Performance, output);
        assert_eq!(jitter_phase_count(half, output), 32);
        assert_eq!(
            render_extent(FsrMode::UltraPerformance, output),
            vk::Extent2D {
                width: 640,
                height: 360
            }
        );
        for mode in FsrMode::ALL {
            assert_eq!(FsrMode::from_name(mode.name()), Some(mode));
        }
        assert_eq!(FsrMode::from_name("fsr"), None);
    }

    #[test]
    fn the_downsampler_covers_the_frame() {
        let (groups, offset, mips) = spd_setup(vk::Extent2D {
            width: 1600,
            height: 900,
        });
        assert_eq!(groups, [25, 15]);
        assert_eq!(offset, [0, 0]);
        assert_eq!(mips, 10);
    }

    #[test]
    fn rcas_strongest_is_one() {
        let config = rcas_config(1.0);
        assert_eq!(f32::from_bits(config[0]), 1.0);
        assert_eq!(config[1], 0x3c00_3c00);
        assert_eq!(f32::from_bits(rcas_config(0.0)[0]), 0.25);
        assert_eq!(half_bits(0.25), 0x3400);
    }
}
