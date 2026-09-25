//! Debugging aid (issue #71): an order-independent hash of chosen images at chosen points of
//! a frame (`shaders/debug_hash.slang`), read back when the frame slot comes back, so that two
//! runs can be compared frame by frame without the waits that hide a race. A demo turns it on
//! with `FORGE_HASH_IMAGES=1` and writes the hashes into its frame trace.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation,
    Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};

/// How the shader reads an image.
#[derive(Clone, Copy, Debug)]
pub enum HashKind {
    /// Any float colour format, read as `float4`.
    Float4,
    /// A depth format, read as `float`.
    Depth,
    /// `R32_UINT`.
    Uint,
    /// A float image compared texel by texel with `other`: the second word counts the texels
    /// whose bits differ.
    Compare {
        /// The image it should equal.
        other: ImageHandle,
    },
}

impl HashKind {
    fn code(self) -> u32 {
        match self {
            Self::Float4 => 0,
            Self::Depth => 1,
            Self::Uint => 2,
            Self::Compare { .. } => 4,
        }
    }
}

/// Most images hashed in a frame.
pub const MAX_HASHED: usize = 16;
const SUM_BYTES: u64 = (MAX_HASHED * 2 * 4) as u64;
const LABEL: &str = "debug/image hash";

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HashPush {
    out: u64,
    image: u32,
    kind: u32,
    width: u32,
    height: u32,
    index: u32,
    other: u32,
}

/// Per image: the hash sum of its texels, then the count of NaN or infinite texels (or of
/// differing ones for [`HashKind::Compare`]).
pub type ImageHash = [u32; 2];

/// The frame being recorded: its slot, the imported sums and readback, the hashes so far.
#[derive(Clone, Copy)]
struct Recording {
    slot: usize,
    sums: BufferHandle,
    readback: BufferHandle,
    count: usize,
}

/// Hashes images inside a frame (see the module documentation):
/// [`ImageHasher::begin`], then [`ImageHasher::add`] wherever an image should be hashed,
/// then [`ImageHasher::finish`].
pub struct ImageHasher {
    pipeline: Pipeline,
    sums: GraphBuffer,
    readback: Vec<GraphBuffer>,
    hashed: [Cell<usize>; FRAMES_IN_FLIGHT],
    recording: Cell<Option<Recording>>,
}

impl ImageHasher {
    /// Compiles the shader and creates the buffers.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("debug_hash.slang", "hash_main", ShaderStage::Compute)?,
            "debug image hash",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "hash_main"),
            push_constant_bytes: std::mem::size_of::<HashPush>() as u32,
            name: "debug image hash",
        })?;
        device.destroy_shader_module(module);
        let sums = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: SUM_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "debug image hashes",
        })?);
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: SUM_BYTES,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("debug image hashes readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline,
            sums,
            readback,
            hashed: Default::default(),
            recording: Cell::new(None),
        })
    }

    /// The hashes of the frame that last used `slot` (its commands have completed), in the
    /// order they were added; empty the first time round.
    pub fn take(&self, slot: FrameSlot) -> Vec<ImageHash> {
        let count = self.hashed[slot.index].take();
        let mut raw = vec![[0_u32; 2]; count];
        self.readback[slot.index].read(0, &mut raw);
        raw
    }

    /// Starts this frame's hashes: clears the sums.
    pub fn begin<'f>(&'f self, graph: &mut FrameGraph<'f>, slot: FrameSlot) {
        let sums_buffer = &self.sums;
        let sums = graph.import_buffer(sums_buffer);
        let readback = graph.import_buffer(&self.readback[slot.index]);
        self.recording.set(Some(Recording {
            slot: slot.index,
            sums,
            readback,
            count: 0,
        }));
        graph
            .pass(LABEL)
            .buffer(sums, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(sums_buffer, 0, SUM_BYTES, 0);
                Ok(())
            });
    }

    /// Hashes `image` (of `kind`, `extent` texels) at this point of the frame.
    pub fn add<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        image: ImageHandle,
        kind: HashKind,
        extent: vk::Extent2D,
    ) {
        let Some(mut recording) = self.recording.get() else {
            return;
        };
        if recording.count == MAX_HASHED {
            return;
        }
        let index = recording.count as u32;
        recording.count += 1;
        self.recording.set(Some(recording));
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let mut pass = graph
            .pass(LABEL)
            .image(image, ImageAccess::Sampled(compute))
            .buffer(recording.sums, BufferAccess::ShaderReadWrite(compute));
        let other = match kind {
            HashKind::Compare { other } => {
                pass = pass.image(other, ImageAccess::Sampled(compute));
                Some(other)
            }
            _ => None,
        };
        let (pipeline, sums_buffer) = (&self.pipeline, &self.sums);
        pass.run(move |resources, commands| {
            commands.bind_pipeline(pipeline);
            commands.push_constants(
                pipeline,
                &HashPush {
                    out: sums_buffer.address(),
                    image: resources.sampled(image).0,
                    kind: kind.code(),
                    width: extent.width,
                    height: extent.height,
                    index,
                    other: other.map_or(0, |o| resources.sampled(o).0),
                },
            );
            commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
            Ok(())
        });
    }

    /// Ends this frame's hashes: copies the sums into the slot's readback.
    pub fn finish<'f>(&'f self, graph: &mut FrameGraph<'f>) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        self.hashed[recording.slot].set(recording.count);
        let (sums_buffer, readback_buffer) = (&self.sums, &self.readback[recording.slot]);
        graph
            .pass(LABEL)
            .buffer(recording.sums, BufferAccess::TransferSrc)
            .buffer(recording.readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(sums_buffer, readback_buffer, SUM_BYTES);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(recording.readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }
}
