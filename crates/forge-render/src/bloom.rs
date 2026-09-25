//! Bloom (issue #44, `shaders/bloom.slang`), after Jimenez 2014: the pre-exposed HDR image
//! filtered down a chain of half-size levels (a 13-tap filter, the first step weighted against
//! fireflies) and back up with a 3×3 tent, each level adding the one below. The top level,
//! at half the image's size, is what the TAA resolve blends into the image it shows
//! ([`crate::Taa::bloom_strength`]); the history stays unbloomed.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, vk,
};

/// Levels of the chain: half the image's size down to a 32nd.
pub const LEVELS: u32 = 6;
const GROUP: u32 = 8;

/// Mirrors `Push` in `bloom.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    src: u32,
    dst: u32,
    dst_size: [u32; 2],
    texel: [f32; 2],
    src_level: f32,
    karis: u32,
}

/// The bloom chain's two passes.
pub struct Bloom {
    downsample: Pipeline,
    upsample: Pipeline,
}

impl Bloom {
    /// Compiles the passes.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("bloom.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        Ok(Self {
            downsample: compute("downsample_main", "bloom down")?,
            upsample: compute("upsample_main", "bloom up")?,
        })
    }

    /// Declares the chain over `input` (a sampled HDR image of `extent`) and returns its top
    /// level: a transient at half the size with [`LEVELS`] mips, the bloom in mip 0.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        input: ImageHandle,
        extent: vk::Extent2D,
    ) -> ImageHandle {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let size = |level: u32| {
            [
                (extent.width >> (level + 1)).max(1),
                (extent.height >> (level + 1)).max(1),
            ]
        };
        let chain = graph.transient(TransientDesc {
            name: "bloom",
            width: size(0)[0],
            height: size(0)[1],
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: LEVELS,
        });
        for level in 0..LEVELS {
            let pipeline = &self.downsample;
            let dst = size(level);
            let (src_size, pass) = if level == 0 {
                (
                    [extent.width, extent.height],
                    graph
                        .pass("post/bloom")
                        .image(input, ImageAccess::Sampled(compute)),
                )
            } else {
                (
                    size(level - 1),
                    graph.pass("post/bloom").image_mip(
                        chain,
                        level - 1,
                        ImageAccess::Sampled(compute),
                    ),
                )
            };
            pass.image_mip(chain, level, ImageAccess::StorageWrite(compute))
                .run(move |resources, commands| {
                    let (src, src_level) = if level == 0 {
                        (resources.sampled(input).0, 0.0)
                    } else {
                        (resources.sampled(chain).0, (level - 1) as f32)
                    };
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &Push {
                            src,
                            dst: resources.storage(chain, level).0,
                            dst_size: dst,
                            texel: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                            src_level,
                            karis: u32::from(level == 0),
                        },
                    );
                    commands.dispatch(dst[0].div_ceil(GROUP), dst[1].div_ceil(GROUP), 1);
                    Ok(())
                });
        }
        for level in (0..LEVELS - 1).rev() {
            let pipeline = &self.upsample;
            let dst = size(level);
            let src_size = size(level + 1);
            graph
                .pass("post/bloom")
                .image_mip(chain, level + 1, ImageAccess::Sampled(compute))
                .image_mip(chain, level, ImageAccess::StorageReadWrite(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &Push {
                            src: resources.sampled(chain).0,
                            dst: resources.storage(chain, level).0,
                            dst_size: dst,
                            texel: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                            src_level: (level + 1) as f32,
                            karis: 0,
                        },
                    );
                    commands.dispatch(dst[0].div_ceil(GROUP), dst[1].div_ceil(GROUP), 1);
                    Ok(())
                });
        }
        chain
    }
}
