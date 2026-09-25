//! Ground-truth ambient occlusion (issue #48, `shaders/gtao.slang`): Jimenez et al. 2016 as
//! Intel's XeGTAO implements it, from the frame's depth alone. The resolve scales the sky's
//! light by it (the sun has its own shadow ray).
//!
//! Four groups of graph passes a frame, on transient images of the frame's size:
//! - `ao/depth chain`: the distance of every pixel along the view axis, and four levels
//!   below it, each a 2×2 average weighted towards the near samples;
//! - `ao/gtao`: per pixel, three directions and three samples each way along each, read
//!   from the level that matches the sample's distance; the highest horizons bound the
//!   visible arc, integrated against a normal rebuilt from the depth;
//! - `ao/denoise`: a 3×3 blur that does not cross depth edges.
//!
//! The noise that places the samples changes every frame (a Hilbert curve and the R2
//! sequence) and repeats with TAA's jitter, which averages it.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::Mat4;

/// Levels of the distance chain (`DEPTH_MIPS` in `gtao.slang`).
const DEPTH_MIPS: u32 = 5;
const GROUP: u32 = 8;

/// Mirrors `Push` in `gtao.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoPush {
    ndc_to_view_mul: [f32; 2],
    ndc_to_view_add: [f32; 2],
    pixel_size: [f32; 2],
    size: [u32; 2],
    near: f32,
    radius: f32,
    frame: u32,
    src: u32,
    src_level: u32,
    dst: u32,
    depth_mips: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GtaoPush>() == 64);

/// What a frame's occlusion needs.
#[derive(Clone, Copy, Debug)]
pub struct GtaoParams {
    /// The projection the depth was drawn with (jitter included): reversed-Z with an
    /// infinite far plane, as `FlyCamera::projection` makes it.
    pub projection: Mat4,
    /// The noise's frame index. The city passes TAA's frame modulo its jitter's period: the
    /// noise then repeats with the jitter and TAA settles on one pattern instead of drifting
    /// through XeGTAO's 64 (measured: the static view's slow change 0.24 % of pixels → 0.10 %,
    /// 0.09 % without occlusion).
    pub frame: u64,
    /// How far an occluder reaches, metres (XeGTAO's effect radius; the shader widens it by
    /// its radius multiplier).
    pub radius: f32,
}

/// The passes' pipelines.
pub struct Gtao {
    prefilter0: Pipeline,
    prefilter: Pipeline,
    main: Pipeline,
    denoise: Pipeline,
}

impl Gtao {
    /// Compiles the four entry points.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("gtao.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<GtaoPush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        Ok(Self {
            prefilter0: compute("prefilter0_main", "ao depth chain 0")?,
            prefilter: compute("prefilter_main", "ao depth chain")?,
            main: compute("gtao_main", "ao")?,
            denoise: compute("denoise_main", "ao denoise")?,
        })
    }

    /// Declares the passes over `depth` (the frame's depth, `extent` pixels) and returns the
    /// occlusion: an r32f image of the same size, 1 where the sky is fully seen.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        depth: ImageHandle,
        extent: vk::Extent2D,
        params: GtaoParams,
    ) -> ImageHandle {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let image = |name: &'static str, mip_levels: u32| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format: vk::Format::R32_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels,
        };
        let chain = graph.transient(image("ao depth chain", DEPTH_MIPS));
        let raw = graph.transient(image("ao raw", 1));
        let occlusion = graph.transient(image("ao", 1));
        // The jittered reversed-Z projection: x_ndc = P00 x / -z + jx, y likewise, and a
        // depth of near / -z. The screen position (y down) maps to view space by
        // x = (2u - 1 - jx) / P00 · z and y = (1 - 2v - jy) / P11 · z.
        let p = params.projection;
        let (p00, p11) = (p.x_axis.x, p.y_axis.y);
        let (jx, jy) = (-p.z_axis.x, -p.z_axis.y);
        let base = GtaoPush {
            ndc_to_view_mul: [2.0 / p00, -2.0 / p11],
            ndc_to_view_add: [(-1.0 - jx) / p00, (1.0 - jy) / p11],
            pixel_size: [
                1.0 / extent.width.max(1) as f32,
                1.0 / extent.height.max(1) as f32,
            ],
            size: [extent.width, extent.height],
            near: p.w_axis.z,
            radius: params.radius,
            frame: params.frame as u32,
            ..GtaoPush::zeroed()
        };
        for level in 0..DEPTH_MIPS {
            let pipeline = if level == 0 {
                &self.prefilter0
            } else {
                &self.prefilter
            };
            let size = [
                (extent.width >> level).max(1),
                (extent.height >> level).max(1),
            ];
            let pass = graph.pass("ao/depth chain");
            let pass = if level == 0 {
                pass.image(depth, ImageAccess::Sampled(compute))
            } else {
                pass.image_mip(chain, level - 1, ImageAccess::Sampled(compute))
            };
            pass.image_mip(chain, level, ImageAccess::StorageWrite(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &GtaoPush {
                            size,
                            src: if level == 0 {
                                resources.sampled(depth).0
                            } else {
                                resources.sampled(chain).0
                            },
                            src_level: level.saturating_sub(1),
                            dst: resources.storage(chain, level).0,
                            ..base
                        },
                    );
                    commands.dispatch(size[0].div_ceil(GROUP), size[1].div_ceil(GROUP), 1);
                    Ok(())
                });
        }
        let groups = [extent.width.div_ceil(GROUP), extent.height.div_ceil(GROUP)];
        let pipeline = &self.main;
        graph
            .pass("ao/gtao")
            .image(chain, ImageAccess::Sampled(compute))
            .image(raw, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &GtaoPush {
                        dst: resources.storage(raw, 0).0,
                        depth_mips: resources.sampled(chain).0,
                        ..base
                    },
                );
                commands.dispatch(groups[0], groups[1], 1);
                Ok(())
            });
        let pipeline = &self.denoise;
        graph
            .pass("ao/denoise")
            .image(chain, ImageAccess::Sampled(compute))
            .image(raw, ImageAccess::Sampled(compute))
            .image(occlusion, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &GtaoPush {
                        src: resources.sampled(raw).0,
                        dst: resources.storage(occlusion, 0).0,
                        depth_mips: resources.sampled(chain).0,
                        ..base
                    },
                );
                commands.dispatch(groups[0], groups[1], 1);
                Ok(())
            });
        occlusion
    }
}
