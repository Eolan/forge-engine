//! MaxCLL and MaxFALL from the frames shown (issue #125, `shaders/hdr_metadata.slang`): every
//! pixel's largest channel as a PQ signal over Rec.2020, counted into 256 bins with the
//! largest signal kept, read back `FRAMES_IN_FLIGHT` frames later like the luminance meter.
//! The shell keeps the largest values since the mode or the peak last changed and tells the
//! display through `VK_EXT_hdr_metadata`.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

use crate::hdr::{ContentLight, pq_decode};

/// Bins of the histogram (`BINS` in `hdr_metadata.slang`), over the PQ signal.
const BINS: usize = 256;
/// The bins, then the largest signal in 1/65535ths.
const WORDS: usize = BINS + 1;
const BYTES: u64 = (WORDS * 4) as u64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MetadataPush {
    bins: u64,
    image: u32,
    width: u32,
    height: u32,
    /// `OUTPUT_PQ` (2) or `OUTPUT_SCRGB` (3) of `tonemap.slang`.
    encoding: u32,
}

/// The histogram pass, its device-local bins and the per-slot readback buffers.
pub(crate) struct ContentLightMeter {
    pipeline: Pipeline,
    bins: GraphBuffer,
    readback: Vec<GraphBuffer>,
    /// The frame each slot's readback measured, when it holds one.
    measured: [Option<u64>; FRAMES_IN_FLIGHT],
}

impl ContentLightMeter {
    pub(crate) fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("hdr_metadata.slang", "histogram_main", ShaderStage::Compute)?,
            "hdr metadata histogram",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "histogram_main"),
            push_constant_bytes: std::mem::size_of::<MetadataPush>() as u32,
            name: "hdr metadata histogram",
        });
        device.destroy_shader_module(module);
        let bins = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "hdr metadata histogram",
        })?);
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: BYTES,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("hdr metadata readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline: pipeline?,
            bins,
            readback,
            measured: [None; FRAMES_IN_FLIGHT],
        })
    }

    /// Drops the measurements in flight (the mode or the peak changed under them).
    pub(crate) fn forget(&mut self) {
        self.measured = [None; FRAMES_IN_FLIGHT];
    }

    /// The frame number and content light the frame that last used `slot` measured (its
    /// commands have completed); `None` when it measured nothing.
    pub(crate) fn take(&mut self, slot: FrameSlot) -> Option<(u64, ContentLight)> {
        let frame = self.measured[slot.index].take()?;
        let mut words = [0_u32; WORDS];
        self.readback[slot.index].read(0, &mut words);
        content_light(&words).map(|light| (frame, light))
    }

    /// Declares "post/hdr metadata histogram" over `image`, the frame as shown (`encoding`:
    /// `OUTPUT_PQ` or `OUTPUT_SCRGB`), for frame `frame`: clear the bins, count, copy them to
    /// this slot's readback buffer and hand it to the host.
    pub(crate) fn measure<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        image: ImageHandle,
        extent: vk::Extent2D,
        encoding: u32,
    ) {
        const LABEL: &str = "post/hdr metadata histogram";
        self.measured[slot.index] = Some(frame);
        let this: &'f Self = self;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let (bins_buffer, readback_buffer) = (&this.bins, &this.readback[slot.index]);
        let bins = graph.import_buffer(bins_buffer);
        let readback = graph.import_buffer(readback_buffer);
        let pipeline = &this.pipeline;
        graph
            .pass(LABEL)
            .buffer(bins, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(bins_buffer, 0, BYTES, 0);
                Ok(())
            });
        graph
            .pass(LABEL)
            .image(image, ImageAccess::Sampled(compute))
            .buffer(bins, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &MetadataPush {
                        bins: bins_buffer.address(),
                        image: resources.sampled(image).0,
                        width: extent.width,
                        height: extent.height,
                        encoding,
                    },
                );
                // 32 × 32 pixels per group of 16 × 16 threads.
                commands.dispatch(extent.width.div_ceil(32), extent.height.div_ceil(32), 1);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(bins, BufferAccess::TransferSrc)
            .buffer(readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(bins_buffer, readback_buffer, BYTES);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }
}

/// A frame's MaxCLL (the largest signal, decoded) and frame average (each bin at its middle
/// signal's nits, within half a bin: 2 % of the luminance) from the histogram's words; `None`
/// for an empty frame.
fn content_light(words: &[u32; WORDS]) -> Option<ContentLight> {
    let pixels: u64 = words[..BINS].iter().map(|&n| u64::from(n)).sum();
    if pixels == 0 {
        return None;
    }
    let sum: f64 = words[..BINS]
        .iter()
        .enumerate()
        .map(|(bin, &n)| f64::from(n) * f64::from(pq_decode((bin as f32 + 0.5) / BINS as f32)))
        .sum();
    Some(ContentLight {
        max_cll: pq_decode(words[BINS] as f32 / 65535.0),
        max_fall: (sum / pixels as f64) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdr::pq_encode;

    /// The words `hdr_metadata.slang` would count for `pixels` of these nits.
    fn histogram(pixels: &[(f32, u32)]) -> [u32; WORDS] {
        let mut words = [0; WORDS];
        for &(nits, count) in pixels {
            let signal = pq_encode(nits);
            words[((signal * BINS as f32) as usize).min(BINS - 1)] += count;
            words[BINS] = words[BINS].max((signal * 65535.0).round() as u32);
        }
        words
    }

    #[test]
    fn max_cll_is_the_brightest_pixel() {
        let light =
            content_light(&histogram(&[(0.0, 1000), (100.0, 500), (812.0, 1)])).expect("measured");
        assert!((light.max_cll / 812.0 - 1.0).abs() < 1e-3, "{light:?}");
    }

    #[test]
    fn max_fall_is_the_frame_average_within_half_a_bin() {
        for (pixels, average) in [
            (vec![(100.0, 1000)], 100.0),
            (vec![(0.0, 900), (1000.0, 100)], 100.0),
            (vec![(10.0, 500), (400.0, 500)], 205.0),
        ] {
            let light = content_light(&histogram(&pixels)).expect("measured");
            assert!(
                (light.max_fall / average - 1.0).abs() < 0.03,
                "{pixels:?}: {light:?}"
            );
        }
        assert_eq!(content_light(&[0; WORDS]), None);
    }
}
