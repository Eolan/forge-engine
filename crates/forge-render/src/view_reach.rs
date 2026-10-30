//! How far the view reaches (D-088): the geometric mean of the distances a 32 × 18 grid of the
//! frame's pixels shows (`shaders/view_reach.slang`), read back by the CPU when the frame's slot
//! comes round again. The probes' finest cascade stands at most that far ahead of the camera
//! ([`crate::Probes::set_focus`]): round a room near the camera, down a street far from it. A fixed
//! 20 m made the village's room worse (against Forge's path, 0.230 to 0.272) where it helped the
//! Bistro's street (0.059 to 0.041).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

/// Mirrors `Push` in `view_reach.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReachPush {
    reach: u64,
    depth_image: u32,
    width: u32,
    height: u32,
    near: f32,
}

const _: () = assert!(std::mem::size_of::<ReachPush>() == 24);

/// The pass and a host-visible word per frame slot.
pub struct ViewReach {
    pipeline: Pipeline,
    buffers: Vec<GraphBuffer>,
    measured: [std::cell::Cell<bool>; FRAMES_IN_FLIGHT],
}

impl ViewReach {
    /// Compiles the shader and creates the buffers.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("view_reach.slang", "view_reach_main", ShaderStage::Compute)?,
            "view reach",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "view_reach_main"),
            push_constant_bytes: std::mem::size_of::<ReachPush>() as u32,
            name: "view reach",
        });
        device.destroy_shader_module(module);
        let buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: 16,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("view reach {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline: pipeline?,
            buffers,
            measured: Default::default(),
        })
    }

    /// The reach the frame that last used `slot` measured (its commands have completed), metres;
    /// `None` when it measured none.
    pub fn read(&self, slot: FrameSlot) -> Option<f32> {
        if !self.measured[slot.index].take() {
            return None;
        }
        let mut reach = [0.0_f32];
        self.buffers[slot.index].read(0, &mut reach);
        Some(reach[0]).filter(|r| r.is_finite())
    }

    /// Declares "gi/view reach": the reach of the frame whose reversed-Z `depth` (`extent`, the
    /// near plane `near`) is given, for [`Self::read`] when `slot` comes round again.
    pub fn measure<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        depth: ImageHandle,
        extent: vk::Extent2D,
        near: f32,
    ) {
        self.measured[slot.index].set(true);
        let buffer = &self.buffers[slot.index];
        let handle = graph.import_buffer(buffer);
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = &self.pipeline;
        graph
            .pass("gi/view reach")
            .image(depth, ImageAccess::Sampled(compute))
            .buffer(handle, BufferAccess::ShaderWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &ReachPush {
                        reach: buffer.address(),
                        depth_image: resources.sampled(depth).0,
                        width: extent.width,
                        height: extent.height,
                        near,
                    },
                );
                commands.dispatch(1, 1, 1);
                Ok(())
            });
        graph
            .pass("gi/view reach")
            .buffer(handle, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }
}
