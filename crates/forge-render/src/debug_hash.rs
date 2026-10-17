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
    /// Where a float image differs from `other`: the words are the largest `y << 16 | x` of
    /// the differing texels and the largest of its complement (the smallest, inverted); with
    /// `columns`, `x << 16 | y`.
    Where {
        /// The image it should equal.
        other: ImageHandle,
        /// Ordered by column rather than by row.
        columns: bool,
    },
    /// The range of x over the texels where a float image differs from `other`: the largest
    /// of this image's and of `other`'s, as order-preserving bits; with `smallest`, the
    /// smallest, inverted.
    Range {
        /// The image it should equal.
        other: ImageHandle,
        /// The smallest rather than the largest.
        smallest: bool,
    },
}

impl HashKind {
    fn code(self) -> u32 {
        match self {
            Self::Float4 => 0,
            Self::Depth => 1,
            Self::Uint => 2,
            Self::Compare { .. } => 4,
            Self::Where { columns, .. } => 5 + u32::from(columns),
            Self::Range { smallest, .. } => 7 + u32::from(smallest),
        }
    }
}

/// Most images hashed in a frame.
pub const MAX_HASHED: usize = 32;
const SUM_BYTES: u64 = (MAX_HASHED * 2 * 4) as u64;
const LABEL: &str = "debug/image hash";
/// Most tiles [`ImageHasher::add_tiles`] hashes: a 4096 × 2304 image.
const MAX_TILES: u64 = 4096 * 2304 / 64;

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
    words: u64,
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
    /// #277: per slot, a hash per 8×8 tile of one image ([`ImageHasher::add_tiles`]).
    tiles: Vec<GraphBuffer>,
    tiled: [Cell<usize>; FRAMES_IN_FLIGHT],
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
        let tiles = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: MAX_TILES * 4,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("debug tile hashes {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline,
            sums,
            readback,
            tiles,
            tiled: Default::default(),
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

    /// The tile hashes of the frame that last used `slot` ([`ImageHasher::add_tiles`]), row by
    /// row; empty when it hashed none.
    pub fn take_tiles(&self, slot: FrameSlot) -> Vec<u32> {
        let count = self.tiled[slot.index].take();
        let mut raw = vec![0_u32; count];
        self.tiles[slot.index].read(0, &mut raw);
        raw
    }

    /// Hashes each 8×8 tile of `image` (a float colour, `extent` texels) at this point of the
    /// frame into the slot's tile buffer (#277): two runs' tiles show where they part. Once a
    /// frame; later calls in the frame are ignored.
    pub fn add_tiles<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        image: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let Some(recording) = self.recording.get() else {
            return;
        };
        let (columns, rows) = (extent.width.div_ceil(8), extent.height.div_ceil(8));
        let count = (columns * rows) as usize;
        if self.tiled[recording.slot].get() != 0 || count as u64 > MAX_TILES {
            return;
        }
        self.tiled[recording.slot].set(count);
        let buffer = &self.tiles[recording.slot];
        let handle = graph.import_buffer(buffer);
        graph
            .pass(LABEL)
            .buffer(handle, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(buffer, 0, count as u64 * 4, 0);
                Ok(())
            });
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = &self.pipeline;
        graph
            .pass(LABEL)
            .image(image, ImageAccess::Sampled(compute))
            .buffer(handle, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &HashPush {
                        out: 0,
                        image: resources.sampled(image).0,
                        kind: 12,
                        width: extent.width,
                        height: columns,
                        index: 0,
                        other: extent.height,
                        words: buffer.address(),
                    },
                );
                commands.dispatch(columns, rows, 1);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(handle, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
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
            HashKind::Compare { other }
            | HashKind::Where { other, .. }
            | HashKind::Range { other, .. } => {
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
                    words: 0,
                },
            );
            commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
            Ok(())
        });
    }

    /// Hashes the words of `buffer` (its first `bytes`, or all of it) at this point of the
    /// frame (#277): the buffers a pass reads through their addresses.
    pub fn add_buffer<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        buffer: BufferHandle,
        bytes: Option<u64>,
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
        let (pipeline, sums_buffer) = (&self.pipeline, &self.sums);
        graph
            .pass(LABEL)
            .buffer(buffer, BufferAccess::ShaderRead(compute))
            .buffer(recording.sums, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                let raw = resources.buffer(buffer);
                let words = (bytes.unwrap_or(raw.size()).min(raw.size()) / 4) as u32;
                const ROW: u32 = 4096;
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &HashPush {
                        out: sums_buffer.address(),
                        image: 0,
                        kind: 10,
                        width: words,
                        height: ROW,
                        index,
                        other: 0,
                        words: raw.address(),
                    },
                );
                commands.dispatch(ROW / 8, words.div_ceil(ROW * 8).max(1), 1);
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
