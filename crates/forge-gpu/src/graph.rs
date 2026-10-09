//! The render graph: passes declare what they read and write, the graph derives every
//! barrier and layout transition, places transient images in one aliased heap and hands
//! retired resources to the frame slots for deferred destruction. Demos and renderers never
//! write a barrier; they cannot forget one either, which is the lesson this module exists
//! for (the previous engine's undeclared buffer uses made NVIDIA replay stale indirect
//! arguments).
//!
//! Shape of a frame:
//! 1. [`FrameGraph::new`]; imports ([`FrameGraph::import`], [`FrameGraph::import_buffer`],
//!    [`FrameGraph::import_raw`] for the swapchain image) and transients
//!    ([`FrameGraph::transient`]) return handles;
//! 2. passes ([`FrameGraph::pass`]) declare every handle they touch with an [`ImageAccess`]
//!    or [`BufferAccess`] and give a body that records commands from the resolved resources;
//! 3. [`RenderGraph::execute`] lays out the transients (reusing last frame's heap when the
//!    layout is unchanged), derives the barriers of every pass from the tracked state of
//!    every subresource (per mip level), records barriers + body + profiler mark per pass,
//!    and writes the final states back into the imported resources so the next frame
//!    continues from them.
//!
//! Rules: passes run in declaration order on the graphics queue; nothing is culled (every
//! declared pass runs, every declared access is honoured). Consecutive passes with the same
//! label share one profiler zone. Immutable resources (textures uploaded once) are imported
//! with their upload state ([`GraphImage::uploaded`]): declaring them costs nothing and keeps
//! the rule that every access is declared.
//!
//! Queues (issue #77): a pass may ask for the async compute or the transfer queue
//! ([`PassBuilder::queue`]; graphics when the device has no such queue, or with
//! `FORGE_ASYNC=0`). Such a pass moves up to just after the last pass it conflicts with (a
//! shared resource that one of them writes), so it overlaps the graphics passes in between.
//! The frame becomes a list of batches, one submission each: consecutive passes on one queue
//! that wait for the same batches of the other queues. Every resource remembers which batch
//! last wrote it and which read it since, on each queue, from frame to frame; an access from
//! another queue becomes a timeline wait, and a barrier on the new queue from all its earlier
//! work. A wait covers every later submission of its queue at its stages, so a batch leaves
//! out what an earlier batch of its queue waited for, in this frame or a previous one (#104:
//! a frame without streaming uploads no longer waits for the last copy again). The last
//! batch is graphics and waits for every other queue's last one, so a frame slot is free
//! when its graphics work is. Resources shared by queues must be `CONCURRENT`
//! (every buffer, and every image but a render target: [`Device::image_concurrent`]); an
//! async pass may not use a transient (its memory could be aliased while it runs).
//!
//! Transients (`depth`, the HDR colour target, motion vectors; and buffers that live within a
//! frame, [`FrameGraph::transient_buffer`], #78) live in one heap laid out from their
//! lifetimes: two transients whose pass ranges never overlap share memory, a buffer padded to
//! the buffer-image granularity so it never shares a page with an image. The first use of a
//! transient in a frame starts from `UNDEFINED` (its contents never survive a frame) and waits
//! for the last use of every transient overlapping its memory, in this frame or the previous
//! one, which is what makes aliasing safe across frames in flight. A transient buffer's
//! address is valid from its first declared pass to its last: a body takes it from
//! [`Resources::buffer`], never before [`RenderGraph::execute`].
//!
//! Debugging: `FORGE_GRAPH_LOG=1` logs the compiled plan (passes, barriers, transient
//! placement) whenever it changes; `FORGE_GRAPH_NO_ALIAS=1` gives every transient its own
//! memory, to tell an aliasing bug from anything else (`FORGE_GRAPH_NO_ALIAS=ao,taa` only to
//! the transients whose name contains one of the words); `FORGE_GRAPH_RELAYOUT=N` creates the
//! transients again on the graph's N-th frame, as a change of the frame's transients does
//! (#204; frames without transients, as a loading screen's, do not count);
//! `FORGE_GRAPH_POISON=1` fills each transient buffer with `0xDEADBEEF` after its
//! last pass, so a stale address read shows in the captures (the validation layers cannot see
//! one); `FORGE_GRAPH_ZERO=1` clears every
//! transient image to zero before its first pass (`FORGE_GRAPH_ZERO=ao,taa` only those whose
//! name contains one of the words), so a pass that reads texels nothing wrote this frame
//! shows by changing the image (#204).

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ash::vk;

use crate::bindless::{SampledImageId, StorageImageId};
use crate::commands::Commands;
use crate::device::{Device, QueueKind};
use crate::error::{GpuError, Result};
use crate::frame::{Batch, FrameSlot, Frames};
use crate::memory::{Buffer, BufferDesc, Image, ImageDesc, TransientHeap};
use crate::memory_report::MemoryCategory;

/// Which queue last touched a resource, and the batches of other queues a new access may
/// have to wait for (issue #77). Carried from frame to frame like [`ResourceState`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct QueueSync {
    /// The queue of the last access.
    last: QueueKind,
    /// The batch that last wrote the resource: its queue and timeline value.
    writer: Option<(QueueKind, u64)>,
    /// Per queue: the latest batch that read it since that write (0: none).
    readers: [u64; 3],
}

/// What each queue's batches already waited for on the other queues' timelines (issue #104),
/// carried from frame to frame like [`QueueSync`]. A semaphore wait of `vkQueueSubmit2`
/// covers every later command of its queue in submission order at its stages, so a later
/// batch needs no wait for a value at or below one its queue waited for at the same stages.
/// Without it, every frame without streaming uploads waited again for the pool's last copy,
/// and that wait split a graphics batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Waited {
    /// Per waiting queue, per queue waited for, per stage bit: the largest value waited for.
    values: [[[u64; 64]; 3]; 3],
}

impl Default for Waited {
    fn default() -> Self {
        Self {
            values: [[[0; 64]; 3]; 3],
        }
    }
}

impl Waited {
    /// Whether `queue` already waited for `value` or later of `kind`'s timeline at each of
    /// `stages`, or at every stage.
    fn covers(
        &self,
        queue: QueueKind,
        kind: QueueKind,
        value: u64,
        stages: vk::PipelineStageFlags2,
    ) -> bool {
        let known = &self.values[queue.index()][kind.index()];
        let all = known[vk::PipelineStageFlags2::ALL_COMMANDS
            .as_raw()
            .trailing_zeros() as usize];
        !stages.is_empty() && stage_bits(stages).all(|bit| known[bit].max(all) >= value)
    }

    /// Records the waits of `batch`, submitted before every later batch of its queue.
    fn record(&mut self, batch: &CompiledBatch) {
        for kind in QueueKind::ALL {
            let (value, stages) = batch.waits[kind.index()];
            if value == 0 {
                continue;
            }
            let known = &mut self.values[batch.queue.index()][kind.index()];
            for bit in stage_bits(stages) {
                known[bit] = known[bit].max(value);
            }
        }
    }
}

/// The index of each bit set in `stages`.
fn stage_bits(stages: vk::PipelineStageFlags2) -> impl Iterator<Item = usize> {
    let raw = stages.as_raw();
    (0..64).filter(move |bit| raw & (1 << bit) != 0)
}

/// The last known use of an image subresource or a buffer, kept between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceState {
    /// Current layout (`UNDEFINED` for buffers and for images whose contents are dead).
    pub layout: vk::ImageLayout,
    /// Stages of the last write, or of every read since it.
    pub stage: vk::PipelineStageFlags2,
    /// Accesses of the last write, or of every read since it.
    pub access: vk::AccessFlags2,
    /// Whether the last access wrote (the next access then needs a memory dependency).
    pub write: bool,
}

impl ResourceState {
    /// Never used: no layout, nothing to wait for.
    pub const UNDEFINED: Self = Self {
        layout: vk::ImageLayout::UNDEFINED,
        stage: vk::PipelineStageFlags2::NONE,
        access: vk::AccessFlags2::NONE,
        write: false,
    };
    /// Filled by [`Device::create_image_with_data`]: readable by every stage.
    pub const UPLOADED: Self = Self {
        layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        stage: vk::PipelineStageFlags2::ALL_COMMANDS,
        access: vk::AccessFlags2::SHADER_SAMPLED_READ,
        write: false,
    };
}

/// How a pass uses an image (or one mip level of it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAccess {
    /// Written (and loaded or blended) as a colour attachment.
    ColorAttachment,
    /// Tested and written as the depth attachment.
    DepthAttachment,
    /// Tested as a read-only depth attachment (`DEPTH_READ_ONLY_OPTIMAL`, store op `NONE`).
    DepthRead,
    /// Sampled or loaded through the bindless set by these shader stages.
    Sampled(vk::PipelineStageFlags2),
    /// Read as a storage image by these stages.
    StorageRead(vk::PipelineStageFlags2),
    /// Written as a storage image by these stages.
    StorageWrite(vk::PipelineStageFlags2),
    /// Read and written as a storage image by these stages.
    StorageReadWrite(vk::PipelineStageFlags2),
    /// Source of a blit or copy.
    TransferSrc,
    /// Destination of a blit or copy.
    TransferDst,
    /// Handed to the presentation engine after the pass (the swapchain image's last use).
    Present,
    /// Any other use, spelled out: third-party work recorded into the pass (DLSS clears its
    /// output at the transfer stage before writing it from compute shaders). A write when
    /// `access` holds any write bit.
    Custom {
        /// Layout during the pass.
        layout: vk::ImageLayout,
        /// Every stage that touches the image.
        stages: vk::PipelineStageFlags2,
        /// Every way those stages touch it.
        access: vk::AccessFlags2,
    },
}

/// How a pass uses a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferAccess {
    /// Read by these shader stages.
    ShaderRead(vk::PipelineStageFlags2),
    /// Written by these shader stages.
    ShaderWrite(vk::PipelineStageFlags2),
    /// Read and written (atomics, bit sets) by these shader stages.
    ShaderReadWrite(vk::PipelineStageFlags2),
    /// Read as indirect draw or dispatch arguments.
    IndirectArgs,
    /// Read as indirect arguments and by these shader stages (a count the draw also reads to
    /// find where its grid ends).
    IndirectArgsAndShaderRead(vk::PipelineStageFlags2),
    /// Read as indirect arguments, and read and written by these shader stages elsewhere in
    /// the buffer (a cull that dispatches from its grid and writes the next pass's).
    IndirectArgsAndShaderReadWrite(vk::PipelineStageFlags2),
    /// Read as the index buffer of draws and by these shader stages (a buffer of cluster
    /// pages the vertex shader also reads).
    IndexAndShaderRead(vk::PipelineStageFlags2),
    /// Source of a copy.
    TransferSrc,
    /// Destination of a copy.
    TransferDst,
    /// Read by the CPU once the frame has completed (readbacks): makes the device's writes
    /// visible to the host, which a fence or semaphore wait alone does not.
    HostRead,
    /// Read by an acceleration structure build: its instance records or geometry.
    BuildInput,
    /// Written by an acceleration structure build: the structure's storage, or its scratch.
    BuildWrite,
    /// Traced against as an acceleration structure by ray queries in these stages.
    AccelerationStructureRead(vk::PipelineStageFlags2),
}

/// Access bits that write.
const WRITES: vk::AccessFlags2 = vk::AccessFlags2::from_raw(
    vk::AccessFlags2::SHADER_WRITE.as_raw()
        | vk::AccessFlags2::SHADER_STORAGE_WRITE.as_raw()
        | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE.as_raw()
        | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE.as_raw()
        | vk::AccessFlags2::TRANSFER_WRITE.as_raw()
        | vk::AccessFlags2::HOST_WRITE.as_raw()
        | vk::AccessFlags2::MEMORY_WRITE.as_raw(),
);

impl ImageAccess {
    /// The state a subresource is in while a pass uses it this way. `sampled_layout` is the
    /// layout its sampled-image descriptor was registered with.
    fn state(self, sampled_layout: vk::ImageLayout) -> ResourceState {
        use vk::{AccessFlags2 as A, ImageLayout as L, PipelineStageFlags2 as S};
        let (layout, stage, access, write) = match self {
            Self::ColorAttachment => (
                L::COLOR_ATTACHMENT_OPTIMAL,
                S::COLOR_ATTACHMENT_OUTPUT,
                A::COLOR_ATTACHMENT_READ | A::COLOR_ATTACHMENT_WRITE,
                true,
            ),
            Self::DepthAttachment => (
                L::DEPTH_ATTACHMENT_OPTIMAL,
                S::EARLY_FRAGMENT_TESTS | S::LATE_FRAGMENT_TESTS,
                A::DEPTH_STENCIL_ATTACHMENT_READ | A::DEPTH_STENCIL_ATTACHMENT_WRITE,
                true,
            ),
            Self::DepthRead => (
                L::DEPTH_READ_ONLY_OPTIMAL,
                S::EARLY_FRAGMENT_TESTS | S::LATE_FRAGMENT_TESTS,
                A::DEPTH_STENCIL_ATTACHMENT_READ,
                false,
            ),
            Self::Sampled(stages) => (sampled_layout, stages, A::SHADER_SAMPLED_READ, false),
            Self::StorageRead(stages) => (L::GENERAL, stages, A::SHADER_STORAGE_READ, false),
            Self::StorageWrite(stages) => (L::GENERAL, stages, A::SHADER_STORAGE_WRITE, true),
            Self::StorageReadWrite(stages) => (
                L::GENERAL,
                stages,
                A::SHADER_STORAGE_READ | A::SHADER_STORAGE_WRITE,
                true,
            ),
            Self::TransferSrc => (
                L::TRANSFER_SRC_OPTIMAL,
                S::TRANSFER,
                A::TRANSFER_READ,
                false,
            ),
            Self::TransferDst => (
                L::TRANSFER_DST_OPTIMAL,
                S::TRANSFER,
                A::TRANSFER_WRITE,
                true,
            ),
            Self::Present => (L::PRESENT_SRC_KHR, S::BOTTOM_OF_PIPE, A::NONE, false),
            Self::Custom {
                layout,
                stages,
                access,
            } => (layout, stages, access, access.intersects(WRITES)),
        };
        ResourceState {
            layout,
            stage,
            access,
            write,
        }
    }

    /// Whether the access may be a transient's first use (it writes the whole image, or
    /// does not care what was there).
    fn initialises(self) -> bool {
        if let Self::Custom { access, .. } = self {
            return access.intersects(WRITES);
        }
        !matches!(
            self,
            Self::DepthRead
                | Self::Sampled(_)
                | Self::StorageRead(_)
                | Self::TransferSrc
                | Self::Present
        )
    }
}

impl BufferAccess {
    fn state(self) -> ResourceState {
        use vk::{AccessFlags2 as A, PipelineStageFlags2 as S};
        let (stage, access, write) = match self {
            Self::ShaderRead(stages) => (stages, A::SHADER_STORAGE_READ, false),
            Self::ShaderWrite(stages) => (stages, A::SHADER_STORAGE_WRITE, true),
            Self::ShaderReadWrite(stages) => (
                stages,
                A::SHADER_STORAGE_READ | A::SHADER_STORAGE_WRITE,
                true,
            ),
            Self::IndirectArgs => (S::DRAW_INDIRECT, A::INDIRECT_COMMAND_READ, false),
            Self::IndirectArgsAndShaderRead(stages) => (
                S::DRAW_INDIRECT | stages,
                A::INDIRECT_COMMAND_READ | A::SHADER_STORAGE_READ,
                false,
            ),
            Self::IndirectArgsAndShaderReadWrite(stages) => (
                S::DRAW_INDIRECT | stages,
                A::INDIRECT_COMMAND_READ | A::SHADER_STORAGE_READ | A::SHADER_STORAGE_WRITE,
                true,
            ),
            Self::IndexAndShaderRead(stages) => (
                S::INDEX_INPUT | stages,
                A::INDEX_READ | A::SHADER_STORAGE_READ,
                false,
            ),
            Self::TransferSrc => (S::TRANSFER, A::TRANSFER_READ, false),
            Self::TransferDst => (S::TRANSFER, A::TRANSFER_WRITE, true),
            Self::HostRead => (S::HOST, A::HOST_READ, false),
            Self::BuildInput => (
                S::ACCELERATION_STRUCTURE_BUILD_KHR,
                A::SHADER_READ | A::ACCELERATION_STRUCTURE_READ_KHR,
                false,
            ),
            Self::BuildWrite => (
                S::ACCELERATION_STRUCTURE_BUILD_KHR,
                A::ACCELERATION_STRUCTURE_READ_KHR | A::ACCELERATION_STRUCTURE_WRITE_KHR,
                true,
            ),
            Self::AccelerationStructureRead(stages) => {
                (stages, A::ACCELERATION_STRUCTURE_READ_KHR, false)
            }
        };
        ResourceState {
            layout: vk::ImageLayout::UNDEFINED,
            stage,
            access,
            write,
        }
    }
}

/// Moves a subresource from `state` to `dst`. Returns the barrier to record, or `None` when
/// a read follows reads in the same layout whose stages and accesses already cover it. The
/// new stages are remembered so a later write waits for every reader.
///
/// A read in a stage (or of an access) the earlier readers do not cover gets a barrier from
/// them (issue #71): the barrier that followed the last write reached those readers' stages
/// only, so without one this read could run before the write finished (the TAA motion
/// vectors' fragment reads of the depth, after the dust's compute read, did about once in 60
/// frames). Chained from the readers, it waits for the write and sees it.
fn transition(
    state: &mut ResourceState,
    dst: ResourceState,
) -> Option<(ResourceState, ResourceState)> {
    let needed = state.layout != dst.layout || state.write || dst.write;
    if !needed {
        let covered = (state.stage.contains(vk::PipelineStageFlags2::ALL_COMMANDS)
            || state.stage.contains(dst.stage))
            && state.access.contains(dst.access);
        let src = *state;
        state.stage |= dst.stage;
        state.access |= dst.access;
        return (!covered).then_some((src, dst));
    }
    let src = *state;
    *state = dst;
    Some((src, dst))
}

/// An image the graph tracks between frames: the image, its bindless handles (a sampled
/// slot when created with `SAMPLED` usage, one storage slot per mip level with `STORAGE`)
/// and the state of every mip level. Owned by whoever needs the contents to persist (depth
/// pyramids, temporal histories); per-frame targets are transients instead.
pub struct GraphImage {
    device: Arc<Device>,
    image: Image,
    aspect: vk::ImageAspectFlags,
    sampled_layout: vk::ImageLayout,
    sampled: Option<SampledImageId>,
    storage: Vec<StorageImageId>,
    states: Vec<Cell<ResourceState>>,
    sync: Cell<QueueSync>,
    name: String,
}

impl GraphImage {
    /// Creates the image and registers its bindless handles. Its first use in a graph
    /// transitions it from `UNDEFINED`.
    pub fn new(device: &Arc<Device>, desc: ImageDesc<'_>) -> Result<Self> {
        let image = device.create_image(desc)?;
        Ok(Self::wrap(device, image, &desc, ResourceState::UNDEFINED))
    }

    /// Creates a single-level colour image filled with `data` (see
    /// [`Device::create_image_with_data`]), tracked as readable by every stage.
    pub fn uploaded(device: &Arc<Device>, desc: ImageDesc<'_>, data: &[u8]) -> Result<Self> {
        let image = device.create_image_with_data(desc, data)?;
        let desc = ImageDesc {
            mip_levels: 1,
            ..desc
        };
        Ok(Self::wrap(device, image, &desc, ResourceState::UPLOADED))
    }

    fn wrap(
        device: &Arc<Device>,
        image: Image,
        desc: &ImageDesc<'_>,
        state: ResourceState,
    ) -> Self {
        let sampled_layout = if desc.usage.contains(vk::ImageUsageFlags::STORAGE) {
            vk::ImageLayout::GENERAL
        } else {
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        };
        let sampled = desc
            .usage
            .contains(vk::ImageUsageFlags::SAMPLED)
            .then(|| device.register_sampled_image(image.view(), sampled_layout));
        let storage = if desc.usage.contains(vk::ImageUsageFlags::STORAGE) {
            (0..image.mip_levels())
                .map(|level| device.register_storage_image(image.mip_view(level)))
                .collect()
        } else {
            Vec::new()
        };
        let states = (0..image.mip_levels()).map(|_| Cell::new(state)).collect();
        Self {
            device: Arc::clone(device),
            image,
            aspect: desc.aspect,
            sampled_layout,
            sampled,
            storage,
            states,
            sync: Cell::new(QueueSync::default()),
            name: desc.name.to_owned(),
        }
    }

    /// The image.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// The sampled-image handle.
    ///
    /// # Panics
    /// If the image was not created with `SAMPLED` usage.
    pub fn sampled(&self) -> SampledImageId {
        self.sampled
            .unwrap_or_else(|| panic!("image '{}' has no SAMPLED usage", self.name))
    }

    /// The storage-image handle of mip `level`.
    ///
    /// # Panics
    /// If the image was not created with `STORAGE` usage or `level` is out of range.
    pub fn storage(&self, level: u32) -> StorageImageId {
        *self
            .storage
            .get(level as usize)
            .unwrap_or_else(|| panic!("image '{}' has no storage view for mip {level}", self.name))
    }

    /// The tracked state of mip `level`.
    pub fn state(&self, level: u32) -> ResourceState {
        self.states[level as usize].get()
    }

    /// Debug name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn resolved(&self) -> ResolvedImage {
        ResolvedImage {
            raw: self.image.raw(),
            view: self.image.view(),
            mip_views: (0..self.image.mip_levels())
                .map(|l| self.image.mip_view(l))
                .collect(),
            extent: self.image.extent(),
            format: self.image.format(),
            usage: self.image.usage(),
            sampled_layout: self.sampled_layout,
            sampled: self.sampled,
            storage: self.storage.clone(),
        }
    }

    fn meta(&self) -> ImageMeta {
        ImageMeta {
            raw: self.image.raw(),
            aspect: self.aspect,
            mip_levels: self.image.mip_levels(),
            sampled_layout: self.sampled_layout,
            transient: None,
            concurrent: self.device.image_concurrent(self.image.usage()),
            zero: false,
            name: self.name.clone(),
        }
    }
}

impl std::ops::Deref for GraphImage {
    type Target = Image;
    fn deref(&self) -> &Image {
        &self.image
    }
}

impl Drop for GraphImage {
    fn drop(&mut self) {
        if let Some(id) = self.sampled.take() {
            self.device.release_sampled_image(id);
        }
        for id in self.storage.drain(..) {
            self.device.release_storage_image(id);
        }
    }
}

/// A buffer the graph tracks between frames (visibility bits, work lists, indirect
/// arguments, statistics): anything a shader writes.
pub struct GraphBuffer {
    buffer: Buffer,
    state: Cell<ResourceState>,
    sync: Cell<QueueSync>,
}

impl GraphBuffer {
    /// Wraps a buffer whose contents so far were written by the host or by a completed
    /// upload (nothing to wait for).
    pub fn new(buffer: Buffer) -> Self {
        Self {
            buffer,
            state: Cell::new(ResourceState::UNDEFINED),
            sync: Cell::new(QueueSync::default()),
        }
    }

    /// The tracked state.
    pub fn state(&self) -> ResourceState {
        self.state.get()
    }
}

impl std::ops::Deref for GraphBuffer {
    type Target = Buffer;
    fn deref(&self) -> &Buffer {
        &self.buffer
    }
}

/// An image the graph does not own or track between frames: the swapchain image, imported
/// every frame with the state the acquire left it in.
#[derive(Clone, Copy, Debug)]
pub struct RawImage {
    /// The image.
    pub image: vk::Image,
    /// Its view.
    pub view: vk::ImageView,
    /// Size.
    pub extent: vk::Extent2D,
    /// Format.
    pub format: vk::Format,
    /// Aspect of the barriers.
    pub aspect: vk::ImageAspectFlags,
    /// State on entry (for a swapchain image: `UNDEFINED`, waiting at the acquire stage).
    pub state: ResourceState,
    /// Its sampled-image handle, registered with `SHADER_READ_ONLY_OPTIMAL`, when shaders may
    /// read it (an HDR swapchain's images, issue #125).
    pub sampled: Option<SampledImageId>,
    /// Debug name.
    pub name: &'static str,
}

/// A per-frame image: created by the graph, aliased with other transients whose lifetimes
/// do not overlap, discarded at the end of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransientDesc {
    /// Debug name (also the profiler's).
    pub name: &'static str,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Format.
    pub format: vk::Format,
    /// Usage; `SAMPLED` registers a sampled handle, `STORAGE` one storage handle per mip.
    pub usage: vk::ImageUsageFlags,
    /// Aspect of the views and barriers.
    pub aspect: vk::ImageAspectFlags,
    /// Mip levels.
    pub mip_levels: u32,
}

impl TransientDesc {
    /// The image to create, with `extra` usage (`TRANSFER_DST` for `FORGE_GRAPH_ZERO`).
    fn image_desc(&self, extra: vk::ImageUsageFlags) -> ImageDesc<'static> {
        ImageDesc {
            width: self.width,
            height: self.height,
            format: self.format,
            usage: self.usage | extra,
            aspect: self.aspect,
            mip_levels: self.mip_levels.max(1),
            name: self.name,
        }
    }
}

/// A per-frame buffer (#78): created by the graph in the transients' heap, aliased with the
/// transient images and buffers whose lifetimes do not overlap its own. Its contents never
/// survive the frame, and its first use must write it. Device-local, with
/// `SHADER_DEVICE_ADDRESS` and `TRANSFER_DST` always added to `usage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransientBufferDesc {
    /// Debug name.
    pub name: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// Usage flags.
    pub usage: vk::BufferUsageFlags,
}

impl TransientBufferDesc {
    fn usage(&self) -> vk::BufferUsageFlags {
        self.usage | vk::BufferUsageFlags::TRANSFER_DST
    }
}

/// What a transient is: an image or a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TransientKind {
    Image(TransientDesc),
    Buffer(TransientBufferDesc),
}

impl TransientKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Image(desc) => desc.name,
            Self::Buffer(desc) => desc.name,
        }
    }
}

/// Handle of an image declared in a [`FrameGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageHandle(u32);

/// Handle of a buffer declared in a [`FrameGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferHandle(u32);

enum ImageEntry<'f> {
    Imported(&'f GraphImage),
    Raw(RawImage),
    Transient(TransientDesc),
}

enum BufferEntry<'f> {
    Imported(&'f GraphBuffer),
    Transient(TransientBufferDesc),
}

#[derive(Clone, Copy, Debug)]
struct ImageUse {
    handle: ImageHandle,
    mip: Option<u32>,
    access: ImageAccess,
}

#[derive(Clone, Copy, Debug)]
struct BufferUse {
    handle: BufferHandle,
    access: BufferAccess,
}

/// What a pass declared (the part of a pass the compiler looks at).
#[derive(Clone, Debug)]
struct PassDecl {
    label: &'static str,
    images: Vec<ImageUse>,
    buffers: Vec<BufferUse>,
    /// The queue it asked for; resolved against the device by [`RenderGraph::execute`].
    queue: QueueKind,
}

type PassBody<'f> = Box<dyn FnOnce(&Resources<'_>, &Commands<'_>) -> Result<()> + 'f>;

struct Pass<'f> {
    decl: PassDecl,
    run: PassBody<'f>,
}

/// One frame's declaration: resources and passes, built by renderers and executed by
/// [`RenderGraph::execute`]. Borrows the imported resources and the pass bodies for `'f`.
pub struct FrameGraph<'f> {
    extent: vk::Extent2D,
    images: Vec<ImageEntry<'f>>,
    buffers: Vec<BufferEntry<'f>>,
    passes: Vec<Pass<'f>>,
}

impl<'f> FrameGraph<'f> {
    /// An empty frame for a target of `extent` pixels.
    pub fn new(extent: vk::Extent2D) -> Self {
        Self {
            extent,
            images: Vec::new(),
            buffers: Vec::new(),
            passes: Vec::new(),
        }
    }

    /// The frame's target size.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// Declares a persistent image; its state continues from the previous frame.
    pub fn import(&mut self, image: &'f GraphImage) -> ImageHandle {
        self.push_image(ImageEntry::Imported(image))
    }

    /// Declares an untracked image with an explicit entry state (the swapchain image).
    pub fn import_raw(&mut self, image: RawImage) -> ImageHandle {
        self.push_image(ImageEntry::Raw(image))
    }

    /// Declares a per-frame image. Its first use must write it.
    pub fn transient(&mut self, desc: TransientDesc) -> ImageHandle {
        self.push_image(ImageEntry::Transient(desc))
    }

    /// Declares a persistent buffer; its state continues from the previous frame.
    pub fn import_buffer(&mut self, buffer: &'f GraphBuffer) -> BufferHandle {
        self.buffers.push(BufferEntry::Imported(buffer));
        BufferHandle(self.buffers.len() as u32 - 1)
    }

    /// Declares a per-frame buffer (#78). Its first use must write it. A pass body reaches it
    /// through [`Resources::buffer`]: its address is valid from its first declared pass to its
    /// last in this frame and nowhere else, since other transients reuse its memory around
    /// them (`FORGE_GRAPH_POISON=1` fills it after its last pass, to show a stale read).
    pub fn transient_buffer(&mut self, desc: TransientBufferDesc) -> BufferHandle {
        self.buffers.push(BufferEntry::Transient(desc));
        BufferHandle(self.buffers.len() as u32 - 1)
    }

    /// Starts declaring a pass called `label` (`group/name`, the profiler zone; consecutive
    /// passes with the same label share a zone).
    pub fn pass(&mut self, label: &'static str) -> PassBuilder<'_, 'f> {
        PassBuilder {
            graph: self,
            decl: PassDecl {
                label,
                images: Vec::new(),
                buffers: Vec::new(),
                queue: QueueKind::Graphics,
            },
        }
    }

    /// Passes declared so far.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    fn push_image(&mut self, entry: ImageEntry<'f>) -> ImageHandle {
        self.images.push(entry);
        ImageHandle(self.images.len() as u32 - 1)
    }
}

/// Declares one pass: its accesses, then its body with [`PassBuilder::run`].
pub struct PassBuilder<'b, 'f> {
    graph: &'b mut FrameGraph<'f>,
    decl: PassDecl,
}

impl<'f> PassBuilder<'_, 'f> {
    /// The pass uses every mip level of `image` as `access`.
    pub fn image(mut self, image: ImageHandle, access: ImageAccess) -> Self {
        self.decl.images.push(ImageUse {
            handle: image,
            mip: None,
            access,
        });
        self
    }

    /// The pass uses mip `level` of `image` as `access`.
    pub fn image_mip(mut self, image: ImageHandle, level: u32, access: ImageAccess) -> Self {
        self.decl.images.push(ImageUse {
            handle: image,
            mip: Some(level),
            access,
        });
        self
    }

    /// Runs the pass on `queue` (graphics by default). On the compute or transfer queue it
    /// moves up to just after the last pass it conflicts with and overlaps the graphics work
    /// in between; its accesses must be ones that queue supports (compute stages, or copies).
    pub fn queue(mut self, queue: QueueKind) -> Self {
        self.decl.queue = queue;
        self
    }

    /// The pass uses `buffer` as `access`.
    pub fn buffer(mut self, buffer: BufferHandle, access: BufferAccess) -> Self {
        self.decl.buffers.push(BufferUse {
            handle: buffer,
            access,
        });
        self
    }

    /// The pass declares what `declare` adds: several uses a caller keeps together (the
    /// movers' instances and acceleration structure, #79).
    pub fn with(self, declare: impl FnOnce(Self) -> Self) -> Self {
        declare(self)
    }

    /// The pass uses `buffer` as `access`, when the frame declares it: a buffer only some
    /// frames write, which the passes reading it declare then (the movers' instances, #79).
    pub fn buffer_if(self, buffer: Option<BufferHandle>, access: BufferAccess) -> Self {
        match buffer {
            Some(buffer) => self.buffer(buffer, access),
            None => self,
        }
    }

    /// Finishes the pass with the commands it records. The body runs at
    /// [`RenderGraph::execute`], after the graph's barriers, with the resolved resources.
    pub fn run(self, body: impl FnOnce(&Resources<'_>, &Commands<'_>) -> Result<()> + 'f) {
        self.graph.passes.push(Pass {
            decl: self.decl,
            run: Box::new(body),
        });
    }
}

/// An image as a pass body sees it.
#[derive(Clone, Debug)]
pub struct ResolvedImage {
    /// The image.
    pub raw: vk::Image,
    /// The view over every mip level.
    pub view: vk::ImageView,
    mip_views: Vec<vk::ImageView>,
    /// Size of level 0.
    pub extent: vk::Extent2D,
    /// Format.
    pub format: vk::Format,
    /// Usage flags (empty for raw images).
    pub usage: vk::ImageUsageFlags,
    /// The layout a `Sampled` access puts it in (`GENERAL` when it also has `STORAGE` usage).
    pub sampled_layout: vk::ImageLayout,
    /// Sampled-image handle, when the image has `SAMPLED` usage.
    pub sampled: Option<SampledImageId>,
    /// Storage-image handle per mip level, when the image has `STORAGE` usage.
    pub storage: Vec<StorageImageId>,
}

impl ResolvedImage {
    /// The view of one mip level.
    pub fn mip_view(&self, level: u32) -> vk::ImageView {
        self.mip_views[level as usize]
    }
}

/// The resolved resources of a frame, handed to pass bodies.
pub struct Resources<'r> {
    images: &'r [ResolvedImage],
    buffers: &'r [Option<&'r Buffer>],
    names: &'r [String],
    extent: vk::Extent2D,
}

impl Resources<'_> {
    /// The buffer behind `handle`: an imported one, or a transient (#78), whose address is
    /// valid only from its first declared pass to its last of this frame.
    ///
    /// # Panics
    /// If `handle` is a transient buffer no pass uses.
    pub fn buffer(&self, handle: BufferHandle) -> &Buffer {
        self.buffers[handle.0 as usize].expect("a transient buffer no pass uses has no memory")
    }

    /// The device address of the buffer behind `handle`, or 0 for a transient no pass uses
    /// (one a frame declares for passes it may leave out, #78).
    pub fn buffer_address(&self, handle: BufferHandle) -> vk::DeviceAddress {
        self.buffers[handle.0 as usize].map_or(0, Buffer::address)
    }

    /// The image behind `handle`.
    pub fn image(&self, handle: ImageHandle) -> &ResolvedImage {
        &self.images[handle.0 as usize]
    }

    /// The full view of `handle`.
    pub fn view(&self, handle: ImageHandle) -> vk::ImageView {
        self.image(handle).view
    }

    /// The sampled-image handle of `handle`.
    ///
    /// # Panics
    /// If the image has no `SAMPLED` usage.
    pub fn sampled(&self, handle: ImageHandle) -> SampledImageId {
        self.image(handle).sampled.unwrap_or_else(|| {
            panic!(
                "image '{}' has no SAMPLED usage",
                self.names[handle.0 as usize]
            )
        })
    }

    /// The storage-image handle of mip `level` of `handle`.
    ///
    /// # Panics
    /// If the image has no `STORAGE` usage or `level` is out of range.
    pub fn storage(&self, handle: ImageHandle, level: u32) -> StorageImageId {
        *self
            .image(handle)
            .storage
            .get(level as usize)
            .unwrap_or_else(|| {
                panic!(
                    "image '{}' has no storage view for mip {level}",
                    self.names[handle.0 as usize]
                )
            })
    }

    /// The frame's target size.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }
}

/// What the compiler needs to know about a buffer.
#[derive(Clone, Debug, Default)]
struct BufferMeta {
    /// For transients: the memory range in the heap (`None` when not aliased).
    transient: Option<Option<(u64, u64)>>,
    name: String,
}

/// What last used the heap's range `range` before a transient takes it over: the stages and
/// accesses of every transient image and buffer overlapping it, this frame's earlier
/// occupants or last frame's (their states persist in the cache).
fn heap_src(
    range: (u64, u64),
    images: &[ImageMeta],
    image_states: &[Vec<ResourceState>],
    buffers: &[BufferMeta],
    buffer_states: &[ResourceState],
) -> ResourceState {
    let meets = |other: Option<Option<(u64, u64)>>| {
        let (a, la) = range;
        matches!(other, Some(Some((b, lb))) if a < b + lb && b < a + la)
    };
    let mut src = ResourceState::UNDEFINED;
    let states = images
        .iter()
        .zip(image_states)
        .filter(|(meta, _)| meets(meta.transient))
        .flat_map(|(_, states)| states.iter())
        .chain(
            buffers
                .iter()
                .zip(buffer_states)
                .filter(|(meta, _)| meets(meta.transient))
                .map(|(_, state)| state),
        );
    for s in states {
        src.stage |= s.stage;
        src.access |= s.access;
    }
    src
}

/// What the compiler needs to know about an image.
#[derive(Clone, Debug)]
struct ImageMeta {
    raw: vk::Image,
    aspect: vk::ImageAspectFlags,
    mip_levels: u32,
    sampled_layout: vk::ImageLayout,
    /// For transients: the memory range in the heap (`None` when not aliased).
    transient: Option<Option<(u64, u64)>>,
    /// Whether it is `CONCURRENT`, so a pass on another queue than graphics may use it.
    concurrent: bool,
    /// `FORGE_GRAPH_ZERO` (#204): a transient cleared to zero before its first pass.
    zero: bool,
    name: String,
}

/// The barriers and mark of one pass, and the batch it is recorded in.
#[derive(Clone, Debug, Default)]
struct CompiledPass {
    image_barriers: Vec<vk::ImageMemoryBarrier2<'static>>,
    memory_barrier: Option<vk::MemoryBarrier2<'static>>,
    /// Closes the profiler zone after the pass (the next pass has another label, or runs in
    /// another batch).
    mark: Option<&'static str>,
    /// `FORGE_GRAPH_POISON=1` (#78): the transient buffers this pass uses last, filled with a
    /// sentinel after it, and the barrier before the fills.
    poison: Vec<u32>,
    poison_barrier: Option<vk::MemoryBarrier2<'static>>,
    /// `FORGE_GRAPH_ZERO` (#204): the transient images this pass uses first, cleared to zero
    /// before it (image, aspect, mips), and the barriers that put them in the transfer layout.
    zero: Vec<(vk::Image, vk::ImageAspectFlags, u32)>,
    zero_barriers: Vec<vk::ImageMemoryBarrier2<'static>>,
    /// Index into the batches.
    batch: usize,
}

/// One submission: consecutive passes on one queue that wait for the same batches of the
/// other queues (issue #77).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompiledBatch {
    queue: QueueKind,
    /// Per queue: the timeline value to wait for (0 for none) and the stages that wait.
    waits: [(u64, vk::PipelineStageFlags2); 3],
    /// The value it signals on its queue's timeline.
    signal: u64,
}

/// Derives the barriers of every pass on one queue (graphics) and advances `image_states` /
/// `buffer_states` to the end of the frame. Pure: tests run it on null handles.
#[cfg(test)]
fn compile(
    passes: &[PassDecl],
    images: &[ImageMeta],
    image_states: &mut [Vec<ResourceState>],
    buffer_states: &mut [ResourceState],
) -> Result<Vec<CompiledPass>> {
    let mut image_sync = vec![QueueSync::default(); images.len()];
    let mut buffer_sync = vec![QueueSync::default(); buffer_states.len()];
    let buffers = vec![BufferMeta::default(); buffer_states.len()];
    let mut next = [0_u64; 3];
    let (compiled, _) = compile_queued(
        passes,
        images,
        &buffers,
        image_states,
        buffer_states,
        &mut image_sync,
        &mut buffer_sync,
        &mut Waited::default(),
        false,
        &mut |kind| {
            next[kind.index()] += 1;
            next[kind.index()]
        },
    )?;
    Ok(compiled)
}

/// A barrier from everything this queue did before, for a resource another queue used
/// since: the timeline wait orders the other queue's work, this orders this queue's own.
fn cross_queue(
    state: &mut ResourceState,
    dst: ResourceState,
) -> Option<(ResourceState, ResourceState)> {
    let src = ResourceState {
        layout: state.layout,
        stage: vk::PipelineStageFlags2::ALL_COMMANDS,
        access: vk::AccessFlags2::MEMORY_WRITE,
        write: true,
    };
    *state = dst;
    Some((src, dst))
}

/// The stages a timeline wait blocks: the access's, or every stage for the host's or none.
fn wait_stages(stage: vk::PipelineStageFlags2) -> vk::PipelineStageFlags2 {
    if stage.is_empty() || stage.contains(vk::PipelineStageFlags2::HOST) {
        vk::PipelineStageFlags2::ALL_COMMANDS
    } else {
        stage
    }
}

/// Adds to `waits` what an access on `queue` to a resource with `sync` must wait for: the
/// batch of another queue that last wrote it, and for a write, the batches of other queues
/// that read it since; unless an earlier batch of `queue` waited for it already (`waited`).
fn access_waits(
    waits: &mut [(u64, vk::PipelineStageFlags2); 3],
    waited: &Waited,
    sync: &QueueSync,
    queue: QueueKind,
    write: bool,
    stage: vk::PipelineStageFlags2,
) {
    let mut wait = |kind: QueueKind, value: u64| {
        let stages = wait_stages(stage);
        if waited.covers(queue, kind, value, stages) {
            return;
        }
        let entry = &mut waits[kind.index()];
        entry.0 = entry.0.max(value);
        entry.1 |= stages;
    };
    if let Some((kind, value)) = sync.writer
        && kind != queue
    {
        wait(kind, value);
    }
    if write {
        for kind in QueueKind::ALL {
            let value = sync.readers[kind.index()];
            if kind != queue && value > 0 {
                wait(kind, value);
            }
        }
    }
}

/// Records an access on `queue`, in the batch that signals `signal`, in `sync`.
fn record_access(sync: &mut QueueSync, queue: QueueKind, write: bool, signal: u64) {
    sync.last = queue;
    if write {
        sync.writer = Some((queue, signal));
        sync.readers = [0; 3];
    } else {
        let reader = &mut sync.readers[queue.index()];
        *reader = (*reader).max(signal);
    }
}

/// Derives the batches, the barriers of every pass and the timeline waits between queues,
/// and advances the states and the queue records of every resource to the end of the frame.
/// `passes` are in execution order ([`schedule`]) with their queues resolved; `reserve`
/// hands out each queue's next timeline value. The last batch is graphics and waits for the
/// last batch of every other queue. A wait an earlier batch of the same queue made is left
/// out, and `waited` gains the frame's waits (#104). With `poison`, each transient buffer is
/// filled with a sentinel after its last pass (`FORGE_GRAPH_POISON=1`, #78). Pure: tests run
/// it on null handles.
#[allow(clippy::too_many_arguments)]
fn compile_queued(
    passes: &[PassDecl],
    images: &[ImageMeta],
    buffers: &[BufferMeta],
    image_states: &mut [Vec<ResourceState>],
    buffer_states: &mut [ResourceState],
    image_sync: &mut [QueueSync],
    buffer_sync: &mut [QueueSync],
    waited: &mut Waited,
    poison: bool,
    reserve: &mut dyn FnMut(QueueKind) -> u64,
) -> Result<(Vec<CompiledPass>, Vec<CompiledBatch>)> {
    let mut first_use = vec![true; images.len()];
    let mut first_buffer_use = vec![true; buffers.len()];
    // The pass each transient buffer is last used by, for the poison.
    let mut last_buffer_use = vec![usize::MAX; buffers.len()];
    for (index, pass) in passes.iter().enumerate() {
        for use_ in &pass.buffers {
            if let Some(last) = last_buffer_use.get_mut(use_.handle.0 as usize) {
                *last = index;
            }
        }
    }
    let mut compiled = Vec::with_capacity(passes.len());
    let mut batches: Vec<CompiledBatch> = Vec::new();
    for (index, pass) in passes.iter().enumerate() {
        let queue = pass.queue;
        // What the pass waits for on the other queues, and whether its resources may cross.
        let mut waits = [(0_u64, vk::PipelineStageFlags2::NONE); 3];
        for use_ in &pass.images {
            let i = use_.handle.0 as usize;
            let meta = images.get(i).ok_or_else(|| {
                GpuError::Graph(format!("pass '{}' uses an unknown image", pass.label))
            })?;
            if queue != QueueKind::Graphics && meta.transient.is_some() {
                return Err(GpuError::Graph(format!(
                    "pass '{}' on the {} queue uses transient '{}': its memory may be aliased while it runs",
                    pass.label,
                    queue.name(),
                    meta.name
                )));
            }
            if queue != QueueKind::Graphics && !meta.concurrent {
                return Err(GpuError::Graph(format!(
                    "pass '{}' on the {} queue uses '{}', which only the graphics queue may use (a render target or the swapchain)",
                    pass.label,
                    queue.name(),
                    meta.name
                )));
            }
            let dst = use_.access.state(meta.sampled_layout);
            access_waits(
                &mut waits,
                waited,
                &image_sync[i],
                queue,
                dst.write,
                dst.stage,
            );
        }
        for use_ in &pass.buffers {
            let i = use_.handle.0 as usize;
            let (Some(sync), Some(meta)) = (buffer_sync.get(i), buffers.get(i)) else {
                return Err(GpuError::Graph(format!(
                    "pass '{}' uses an unknown buffer",
                    pass.label
                )));
            };
            if queue != QueueKind::Graphics && meta.transient.is_some() {
                return Err(GpuError::Graph(format!(
                    "pass '{}' on the {} queue uses transient buffer '{}': its memory may be aliased while it runs",
                    pass.label,
                    queue.name(),
                    meta.name
                )));
            }
            let dst = use_.access.state();
            access_waits(&mut waits, waited, sync, queue, dst.write, dst.stage);
        }
        // The pass joins the open batch when that batch is on its queue and already waits
        // for everything it needs; otherwise it opens a batch.
        let joins = batches.last().is_some_and(|b| {
            b.queue == queue && (0..3).all(|k| waits[k].0 == 0 || waits[k].0 <= b.waits[k].0)
        });
        if joins {
            let batch = batches.last_mut().expect("checked above");
            for (open, need) in batch.waits.iter_mut().zip(waits) {
                if need.0 > 0 {
                    open.1 |= need.1;
                }
            }
        } else {
            batches.push(CompiledBatch {
                queue,
                waits,
                signal: reserve(queue),
            });
        }
        let batch = batches.len() - 1;
        let signal = batches[batch].signal;
        // The queue's later batches need not wait for these again.
        waited.record(&batches[batch]);

        let mut out = CompiledPass {
            batch,
            ..CompiledPass::default()
        };
        let mut seen: Vec<(u32, Option<u32>)> = Vec::new();
        for use_ in &pass.images {
            let i = use_.handle.0 as usize;
            let meta = &images[i];
            let clashes = seen.iter().any(|&(h, mip)| {
                h == use_.handle.0 && (mip.is_none() || use_.mip.is_none() || mip == use_.mip)
            });
            if clashes {
                return Err(GpuError::Graph(format!(
                    "pass '{}' declares image '{}' twice",
                    pass.label, meta.name
                )));
            }
            seen.push((use_.handle.0, use_.mip));
            let dst = use_.access.state(meta.sampled_layout);
            if let Some(range) = meta.transient
                && first_use[i]
            {
                if !use_.access.initialises() {
                    return Err(GpuError::Graph(format!(
                        "pass '{}' reads transient '{}' before anything wrote it",
                        pass.label, meta.name
                    )));
                }
                // The memory may still be in use by whatever aliased it: this frame's
                // earlier occupants, or last frame's (their states persist in the cache),
                // images and buffers alike.
                let src = match range {
                    Some(range) => heap_src(range, images, image_states, buffers, buffer_states),
                    None => image_states[i]
                        .iter()
                        .fold(ResourceState::UNDEFINED, |mut src, s| {
                            src.stage |= s.stage;
                            src.access |= s.access;
                            src
                        }),
                };
                let undefined = ResourceState {
                    layout: vk::ImageLayout::UNDEFINED,
                    write: true,
                    ..src
                };
                if meta.zero {
                    let fill = ImageAccess::TransferDst.state(meta.sampled_layout);
                    out.zero_barriers.push(image_barrier(
                        meta,
                        0,
                        meta.mip_levels,
                        undefined,
                        fill,
                    ));
                    out.zero.push((meta.raw, meta.aspect, meta.mip_levels));
                    image_states[i].fill(fill);
                } else {
                    image_states[i].fill(undefined);
                }
            }
            first_use[i] = false;
            let (base, count) = match use_.mip {
                Some(level) if level < meta.mip_levels => (level, 1),
                Some(level) => {
                    return Err(GpuError::Graph(format!(
                        "pass '{}' uses mip {level} of '{}', which has {} levels",
                        pass.label, meta.name, meta.mip_levels
                    )));
                }
                None => (0, meta.mip_levels),
            };
            let crossing = image_sync[i].last != queue;
            if crossing {
                // The mips this pass leaves alone were last used on the other queue too, and
                // the image now counts as this queue's: their next barrier here starts from
                // everything before it (the timeline wait orders the other queue's work),
                // not from stages this queue may not have (issue #105: the water's mips,
                // sampled by a vertex shader, then rebuilt on the compute queue).
                for level in (0..meta.mip_levels).filter(|l| !(base..base + count).contains(l)) {
                    let state = &mut image_states[i][level as usize];
                    *state = ResourceState {
                        stage: vk::PipelineStageFlags2::ALL_COMMANDS,
                        access: vk::AccessFlags2::MEMORY_WRITE,
                        write: true,
                        ..*state
                    };
                }
            }
            // One barrier per run of consecutive mips that need the same transition.
            let mut run: Option<(u32, u32, ResourceState, ResourceState)> = None;
            for level in base..base + count {
                let state = &mut image_states[i][level as usize];
                let step = if crossing {
                    cross_queue(state, dst)
                } else {
                    transition(state, dst)
                };
                let extends = matches!(
                    (step, run),
                    (Some((s, d)), Some((_, _, rs, rd))) if rs == s && rd == d
                );
                if extends {
                    if let Some((_, len, _, _)) = &mut run {
                        *len += 1;
                    }
                } else {
                    if let Some((start, len, src, dst)) = run.take() {
                        out.image_barriers
                            .push(image_barrier(meta, start, len, src, dst));
                    }
                    if let Some((src, dst)) = step {
                        run = Some((level, 1, src, dst));
                    }
                }
            }
            if let Some((start, len, src, dst)) = run.take() {
                out.image_barriers
                    .push(image_barrier(meta, start, len, src, dst));
            }
            record_access(&mut image_sync[i], queue, dst.write, signal);
        }
        let mut seen_buffers: Vec<u32> = Vec::new();
        let mut memory: Option<vk::MemoryBarrier2<'static>> = None;
        for use_ in &pass.buffers {
            let i = use_.handle.0 as usize;
            if seen_buffers.contains(&use_.handle.0) {
                return Err(GpuError::Graph(format!(
                    "pass '{}' declares a buffer twice",
                    pass.label
                )));
            }
            seen_buffers.push(use_.handle.0);
            let dst = use_.access.state();
            if let Some(range) = buffers[i].transient
                && first_buffer_use[i]
            {
                if !dst.write {
                    return Err(GpuError::Graph(format!(
                        "pass '{}' reads transient buffer '{}' before anything wrote it",
                        pass.label, buffers[i].name
                    )));
                }
                // As for an image: whatever last used its memory, this frame or the last.
                let src = match range {
                    Some(range) => heap_src(range, images, image_states, buffers, buffer_states),
                    None => buffer_states[i],
                };
                buffer_states[i] = ResourceState {
                    layout: vk::ImageLayout::UNDEFINED,
                    write: true,
                    ..src
                };
            }
            first_buffer_use[i] = false;
            let state = &mut buffer_states[i];
            let step = if buffer_sync[i].last != queue {
                cross_queue(state, dst)
            } else {
                transition(state, dst)
            };
            if let Some((src, dst)) = step
                && src.stage != vk::PipelineStageFlags2::NONE
            {
                let barrier = memory.get_or_insert_with(vk::MemoryBarrier2::default);
                barrier.src_stage_mask |= src.stage;
                barrier.src_access_mask |= src.access;
                barrier.dst_stage_mask |= dst.stage;
                barrier.dst_access_mask |= dst.access;
            }
            record_access(&mut buffer_sync[i], queue, dst.write, signal);
        }
        out.memory_barrier = memory;
        // The poison: each transient buffer this pass uses last, filled after it, so a read of
        // its stale address elsewhere shows.
        if poison {
            for use_ in &pass.buffers {
                let i = use_.handle.0 as usize;
                if buffers[i].transient.is_none() || last_buffer_use[i] != index {
                    continue;
                }
                let fill = BufferAccess::TransferDst.state();
                if let Some((src, dst)) = transition(&mut buffer_states[i], fill) {
                    let barrier = out
                        .poison_barrier
                        .get_or_insert_with(vk::MemoryBarrier2::default);
                    barrier.src_stage_mask |= src.stage;
                    barrier.src_access_mask |= src.access;
                    barrier.dst_stage_mask |= dst.stage;
                    barrier.dst_access_mask |= dst.access;
                }
                record_access(&mut buffer_sync[i], queue, true, signal);
                out.poison.push(use_.handle.0);
            }
        }
        compiled.push(out);
    }
    // The frame ends on the graphics queue, after every other queue's last batch.
    if batches
        .last()
        .is_none_or(|b| b.queue != QueueKind::Graphics)
    {
        batches.push(CompiledBatch {
            queue: QueueKind::Graphics,
            waits: [(0, vk::PipelineStageFlags2::NONE); 3],
            signal: reserve(QueueKind::Graphics),
        });
    }
    let last = batches.len() - 1;
    for kind in [QueueKind::Compute, QueueKind::Transfer] {
        let latest = batches
            .iter()
            .filter(|b| b.queue == kind)
            .map(|b| b.signal)
            .max();
        if let Some(value) = latest {
            let wait = &mut batches[last].waits[kind.index()];
            if wait.0 < value {
                *wait = (value, wait.1 | vk::PipelineStageFlags2::ALL_COMMANDS);
            }
        }
    }
    waited.record(&batches[last]);
    // Zones close where the label changes or the batch does.
    for index in 0..compiled.len() {
        let next = passes
            .get(index + 1)
            .map(|p| p.label)
            .zip(compiled.get(index + 1).map(|c| c.batch));
        if next != Some((passes[index].label, compiled[index].batch)) {
            compiled[index].mark = Some(passes[index].label);
        }
    }
    Ok((compiled, batches))
}

/// Whether two passes touch a resource that one of them writes.
fn conflicts(a: &PassDecl, b: &PassDecl) -> bool {
    let image_write = |u: &ImageUse| u.access.state(vk::ImageLayout::GENERAL).write;
    let images = a.images.iter().any(|ua| {
        b.images.iter().any(|ub| {
            ua.handle == ub.handle
                && (ua.mip.is_none() || ub.mip.is_none() || ua.mip == ub.mip)
                && (image_write(ua) || image_write(ub))
        })
    });
    let buffers = a.buffers.iter().any(|ua| {
        b.buffers.iter().any(|ub| {
            ua.handle == ub.handle && (ua.access.state().write || ub.access.state().write)
        })
    });
    images || buffers
}

/// The order passes run in: declaration order, except that a pass on another queue than
/// graphics moves up to just after the last pass before it that it conflicts with, so it
/// overlaps the graphics passes in between (issue #77).
fn schedule(passes: &[PassDecl]) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::with_capacity(passes.len());
    for (i, pass) in passes.iter().enumerate() {
        if pass.queue == QueueKind::Graphics {
            order.push(i);
            continue;
        }
        let at = order
            .iter()
            .rposition(|&k| conflicts(&passes[k], pass))
            .map_or(0, |p| p + 1);
        order.insert(at, i);
    }
    order
}

fn image_barrier(
    meta: &ImageMeta,
    base_mip: u32,
    level_count: u32,
    src: ResourceState,
    dst: ResourceState,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src.stage)
        .src_access_mask(src.access)
        .dst_stage_mask(dst.stage)
        .dst_access_mask(dst.access)
        .old_layout(src.layout)
        .new_layout(dst.layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(meta.raw)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: meta.aspect,
            base_mip_level: base_mip,
            level_count,
            base_array_layer: 0,
            layer_count: 1,
        })
}

/// A transient to place: its memory needs and the passes it lives through.
#[derive(Clone, Copy, Debug)]
struct Request {
    desc: TransientKind,
    size: u64,
    alignment: u64,
    memory_type_bits: u32,
    first: usize,
    last: usize,
}

/// Where a transient lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    desc: TransientKind,
    /// Offset in the heap (0 when not aliased: it then has its own memory).
    offset: u64,
    size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Placement {
    entries: Vec<Placed>,
    heap_size: u64,
    alignment: u64,
    memory_type_bits: u32,
    /// Whether the entries share one heap (false: no common memory type, or aliasing off).
    aliased: bool,
}

fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.div_ceil(alignment) * alignment
}

/// Lays the transients out in one heap: largest first, each at the lowest offset where it
/// overlaps no image whose lifetime intersects its own.
fn plan(requests: &[Request], allow_alias: bool) -> Placement {
    let memory_type_bits = requests
        .iter()
        .fold(u32::MAX, |bits, r| bits & r.memory_type_bits);
    let aliased = allow_alias && !requests.is_empty() && memory_type_bits != 0;
    if !aliased {
        return Placement {
            entries: requests
                .iter()
                .map(|r| Placed {
                    desc: r.desc,
                    offset: 0,
                    size: r.size,
                })
                .collect(),
            heap_size: 0,
            alignment: 1,
            memory_type_bits,
            aliased: false,
        };
    }
    let mut order: Vec<usize> = (0..requests.len()).collect();
    order.sort_by(|&a, &b| requests[b].size.cmp(&requests[a].size).then(a.cmp(&b)));
    let mut offsets = vec![0_u64; requests.len()];
    let mut placed: Vec<usize> = Vec::with_capacity(requests.len());
    for &i in &order {
        let r = requests[i];
        let mut offset = 0_u64;
        loop {
            let end = offset + r.size;
            let conflict = placed.iter().copied().find(|&j| {
                let o = requests[j];
                let lifetimes_meet = r.first <= o.last && o.first <= r.last;
                let memory_meets = offsets[j] < end && offset < offsets[j] + o.size;
                lifetimes_meet && memory_meets
            });
            match conflict {
                Some(j) => offset = align_up(offsets[j] + requests[j].size, r.alignment),
                None => break,
            }
        }
        offsets[i] = offset;
        placed.push(i);
    }
    let heap_size = requests
        .iter()
        .zip(&offsets)
        .map(|(r, o)| o + r.size)
        .max()
        .unwrap_or(0);
    let alignment = requests.iter().map(|r| r.alignment).max().unwrap_or(1);
    Placement {
        entries: requests
            .iter()
            .zip(&offsets)
            .map(|(r, &offset)| Placed {
                desc: r.desc,
                offset,
                size: r.size,
            })
            .collect(),
        heap_size,
        alignment,
        memory_type_bits,
        aliased: true,
    }
}

/// The transients of the previous frame, kept while their layout is unchanged.
struct TransientCache {
    placement: Placement,
    _heap: Option<Arc<TransientHeap>>,
    /// Per placed entry, in the placement's order.
    transients: Vec<CachedTransient>,
}

/// A transient the cache holds: an image or a buffer, with the state it was left in.
enum CachedTransient {
    Image(GraphImage),
    Buffer(GraphBuffer),
}

impl CachedTransient {
    fn image(&self) -> &GraphImage {
        match self {
            Self::Image(image) => image,
            Self::Buffer(_) => unreachable!("an image's request holds an image"),
        }
    }

    fn buffer(&self) -> &GraphBuffer {
        match self {
            Self::Buffer(buffer) => buffer,
            Self::Image(_) => unreachable!("a buffer's request holds a buffer"),
        }
    }
}

/// Counters of the last executed frame (the F1 overlay shows them).
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphStats {
    /// Passes recorded.
    pub passes: u32,
    /// Image barriers recorded (one per subresource run).
    pub image_barriers: u32,
    /// Global memory barriers recorded (one per pass with buffer hazards).
    pub memory_barriers: u32,
    /// Transient images in use.
    pub transient_images: u32,
    /// Transient buffers in use (#78).
    pub transient_buffers: u32,
    /// Bytes the transients would need with their own memory each.
    pub transient_bytes: u64,
    /// Bytes of the shared heap (equal to `transient_bytes` when not aliased).
    pub heap_bytes: u64,
    /// The most bytes of transients alive at once, over the passes: the least any heap could
    /// be, so `heap_bytes` against it is the placement's slack (#78).
    pub load_bytes: u64,
    /// Whether the transients share one heap.
    pub aliased: bool,
    /// Times the heap was rebuilt since start (a resize, a changed pass list).
    pub heap_rebuilds: u64,
    /// Resources waiting in the deferred-deletion queue.
    pub pending_destructions: usize,
    /// Submissions of the frame (one per batch, issue #77), each one command buffer.
    pub batches: u32,
    /// CPU milliseconds [`RenderGraph::execute`] spent ordering the passes, laying out the
    /// transients and deriving the barriers (#78).
    pub compile_ms: f32,
    /// CPU milliseconds it spent recording: the barriers and the pass bodies (#78).
    pub record_ms: f32,
    /// Passes per queue, in [`QueueKind::ALL`] order.
    pub queue_passes: [u32; 3],
}

/// The persistent side of the graph: the transient heap, statistics, logging.
pub struct RenderGraph {
    device: Arc<Device>,
    cache: Option<TransientCache>,
    requirements: HashMap<TransientDesc, vk::MemoryRequirements>,
    buffer_requirements: HashMap<TransientBufferDesc, vk::MemoryRequirements>,
    stats: GraphStats,
    log: bool,
    allow_alias: bool,
    /// `FORGE_GRAPH_NO_ALIAS=taa,sun` (#204): the transients whose name contains one of the
    /// words keep their memory for the whole frame, sharing it with no other.
    alone: Vec<String>,
    /// `FORGE_GRAPH_POISON=1` (#78): every transient buffer filled with a sentinel after its
    /// last pass, so a stale read shows in the captures.
    poison: bool,
    /// `FORGE_GRAPH_ZERO` (#204): the transient images cleared to zero before their first
    /// pass, all of them (no words) or those whose name contains one of the words.
    zero: Option<Vec<String>>,
    /// `FORGE_FRAME_BARRIER=1`: a full barrier at the start of each queue's first batch, which
    /// serialises frames on the GPU (a debugging aid).
    frame_barrier: bool,
    /// `FORGE_GRAPH_RELAYOUT=N` (#204): the transients created again on the graph's N-th frame
    /// with transients (counted from 1; a loading screen's frames have none), as when the
    /// frame's transients change, to reproduce what a new layout does to that frame.
    relayout_at: Option<u32>,
    /// The frames with transients this graph executed.
    executed: u32,
    /// What each queue's batches waited for on the other queues, from frame to frame (#104).
    waited: Waited,
    last_plan: u64,
}

/// A debugging variable that is off, on (`1`: no words) or limited to the names that contain
/// one of its comma-separated words.
fn words(name: &str) -> Option<Vec<String>> {
    let value = std::env::var(name).ok().filter(|v| v != "0")?;
    Some(
        value
            .split(',')
            .map(str::trim)
            .filter(|w| !w.is_empty() && *w != "1")
            .map(str::to_owned)
            .collect(),
    )
}

/// Whether `name` is selected by the [`words`] of a debugging variable.
fn selects(words: &Option<Vec<String>>, name: &str) -> bool {
    words
        .as_ref()
        .is_some_and(|w| w.is_empty() || w.iter().any(|w| name.contains(w.as_str())))
}

impl RenderGraph {
    /// A graph for `device`.
    pub fn new(device: &Arc<Device>) -> Self {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v != "0");
        Self {
            device: Arc::clone(device),
            cache: None,
            requirements: HashMap::new(),
            buffer_requirements: HashMap::new(),
            stats: GraphStats::default(),
            log: flag("FORGE_GRAPH_LOG"),
            allow_alias: words("FORGE_GRAPH_NO_ALIAS").is_none_or(|w| !w.is_empty()),
            alone: words("FORGE_GRAPH_NO_ALIAS").unwrap_or_default(),
            poison: flag("FORGE_GRAPH_POISON"),
            zero: words("FORGE_GRAPH_ZERO"),
            frame_barrier: flag("FORGE_FRAME_BARRIER"),
            relayout_at: std::env::var("FORGE_GRAPH_RELAYOUT")
                .ok()
                .and_then(|v| v.parse().ok()),
            executed: 0,
            waited: Waited::default(),
            last_plan: 0,
        }
    }

    /// Counters of the last executed frame.
    pub fn stats(&self) -> GraphStats {
        self.stats
    }

    /// Resolves each pass's queue, orders the passes ([`schedule`]), lays out the
    /// transients, derives the batches and barriers, records every batch into a command
    /// buffer of `slot` and queues it on `frames` for [`Frames::submit`], and writes the
    /// final resource states back. `frames` also receives resources retired by a transient
    /// relayout.
    pub fn execute(
        &mut self,
        frame: FrameGraph<'_>,
        frames: &mut Frames,
        slot: FrameSlot,
    ) -> Result<GraphStats> {
        let started = std::time::Instant::now();
        let FrameGraph {
            extent,
            images,
            buffers,
            passes,
        } = frame;
        // Queues as the device has them, then the order they run in.
        let mut passes = passes;
        for pass in &mut passes {
            pass.decl.queue = self.device.resolve_queue(pass.decl.queue);
        }
        let order = {
            let decls: Vec<PassDecl> = passes.iter().map(|p| p.decl.clone()).collect();
            schedule(&decls)
        };
        let mut slots: Vec<Option<Pass<'_>>> = passes.into_iter().map(Some).collect();
        let passes: Vec<Pass<'_>> = order
            .iter()
            .map(|&i| slots[i].take().expect("each pass is scheduled once"))
            .collect();
        let decls: Vec<PassDecl> = passes.iter().map(|p| p.decl.clone()).collect();

        // Transient lifetimes, then their placement; reuse the cache when it matches.
        let mut requests = Vec::new();
        let mut request_of_image: Vec<Option<usize>> = vec![None; images.len()];
        let extra = self.extra_usage();
        for (i, entry) in images.iter().enumerate() {
            let ImageEntry::Transient(desc) = entry else {
                continue;
            };
            let uses: Vec<usize> = decls
                .iter()
                .enumerate()
                .filter(|(_, p)| p.images.iter().any(|u| u.handle.0 as usize == i))
                .map(|(index, _)| index)
                .collect();
            let (Some(&first), Some(&last)) = (uses.first(), uses.last()) else {
                continue;
            };
            let req = *self.requirements.entry(*desc).or_insert_with(|| {
                let desc = desc.image_desc(extra);
                self.device.image_memory_requirements(&desc)
            });
            let (first, last) = self.lifetime(desc.name, first, last, decls.len());
            request_of_image[i] = Some(requests.len());
            requests.push(Request {
                desc: TransientKind::Image(*desc),
                size: req.size,
                alignment: req.alignment,
                memory_type_bits: req.memory_type_bits,
                first,
                last,
            });
        }
        // The transient buffers (#78), padded to the buffer-image granularity at both ends so
        // a buffer never shares a page with an image beside it.
        let granularity = self.device.limits().buffer_image_granularity.max(1);
        let mut request_of_buffer: Vec<Option<usize>> = vec![None; buffers.len()];
        for (i, entry) in buffers.iter().enumerate() {
            let BufferEntry::Transient(desc) = entry else {
                continue;
            };
            let uses: Vec<usize> = decls
                .iter()
                .enumerate()
                .filter(|(_, p)| p.buffers.iter().any(|u| u.handle.0 as usize == i))
                .map(|(index, _)| index)
                .collect();
            let (Some(&first), Some(&last)) = (uses.first(), uses.last()) else {
                continue;
            };
            let req = *self.buffer_requirements.entry(*desc).or_insert_with(|| {
                self.device
                    .buffer_memory_requirements(desc.size, desc.usage())
            });
            let (first, last) = self.lifetime(desc.name, first, last, decls.len());
            request_of_buffer[i] = Some(requests.len());
            requests.push(Request {
                desc: TransientKind::Buffer(*desc),
                size: align_up(req.size, granularity),
                alignment: req.alignment.max(granularity),
                memory_type_bits: req.memory_type_bits,
                first,
                last,
            });
        }
        let placement = plan(&requests, self.allow_alias);
        if !requests.is_empty() {
            self.executed += 1;
        }
        let relayout = !requests.is_empty() && self.relayout_at == Some(self.executed);
        if relayout {
            tracing::info!(
                frame = self.executed,
                "render graph transients laid out again on request (FORGE_GRAPH_RELAYOUT)"
            );
        }
        if relayout || self.cache.as_ref().is_none_or(|c| c.placement != placement) {
            self.rebuild_cache(placement, frames)?;
        }
        let cache = self.cache.as_ref().expect("transient cache built above");

        // Resolve every handle and load the entry states.
        let mut metas = Vec::with_capacity(images.len());
        let mut resolved = Vec::with_capacity(images.len());
        let mut image_states: Vec<Vec<ResourceState>> = Vec::with_capacity(images.len());
        for (i, entry) in images.iter().enumerate() {
            match entry {
                ImageEntry::Imported(image) => {
                    metas.push(image.meta());
                    resolved.push(image.resolved());
                    image_states.push(image.states.iter().map(Cell::get).collect());
                }
                ImageEntry::Raw(raw) => {
                    metas.push(ImageMeta {
                        raw: raw.image,
                        aspect: raw.aspect,
                        mip_levels: 1,
                        sampled_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        transient: None,
                        concurrent: false,
                        zero: false,
                        name: raw.name.to_owned(),
                    });
                    resolved.push(ResolvedImage {
                        raw: raw.image,
                        view: raw.view,
                        mip_views: vec![raw.view],
                        extent: raw.extent,
                        format: raw.format,
                        usage: vk::ImageUsageFlags::empty(),
                        sampled_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        sampled: raw.sampled,
                        storage: Vec::new(),
                    });
                    image_states.push(vec![raw.state]);
                }
                ImageEntry::Transient(desc) => match request_of_image[i] {
                    Some(r) => {
                        let image = cache.transients[r].image();
                        let placed = cache.placement.entries[r];
                        let mut meta = image.meta();
                        meta.transient = Some(
                            cache
                                .placement
                                .aliased
                                .then_some((placed.offset, placed.size)),
                        );
                        meta.zero = selects(&self.zero, desc.name);
                        metas.push(meta);
                        resolved.push(image.resolved());
                        // Contents never survive a frame; what the memory last did does.
                        image_states.push(
                            image
                                .states
                                .iter()
                                .map(|s| ResourceState {
                                    layout: vk::ImageLayout::UNDEFINED,
                                    ..s.get()
                                })
                                .collect(),
                        );
                    }
                    None => {
                        // Declared but used by no pass: nothing to resolve.
                        metas.push(ImageMeta {
                            raw: vk::Image::null(),
                            aspect: desc.aspect,
                            mip_levels: 1,
                            sampled_layout: vk::ImageLayout::UNDEFINED,
                            transient: None,
                            concurrent: false,
                            zero: false,
                            name: format!("{} (unused)", desc.name),
                        });
                        resolved.push(ResolvedImage {
                            raw: vk::Image::null(),
                            view: vk::ImageView::null(),
                            mip_views: Vec::new(),
                            extent: vk::Extent2D::default(),
                            format: vk::Format::UNDEFINED,
                            usage: vk::ImageUsageFlags::empty(),
                            sampled_layout: vk::ImageLayout::UNDEFINED,
                            sampled: None,
                            storage: Vec::new(),
                        });
                        image_states.push(vec![ResourceState::UNDEFINED]);
                    }
                },
            }
        }
        // The buffers: imported ones as they were left; a transient as what last used its
        // memory left it (its contents never survive a frame).
        let transient_buffer =
            |i: usize| request_of_buffer[i].map(|r| cache.transients[r].buffer());
        let buffer_metas: Vec<BufferMeta> = buffers
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                BufferEntry::Imported(_) => BufferMeta {
                    transient: None,
                    name: format!("imported buffer {i}"),
                },
                BufferEntry::Transient(desc) => BufferMeta {
                    transient: request_of_buffer[i].map(|r| {
                        let placed = cache.placement.entries[r];
                        cache
                            .placement
                            .aliased
                            .then_some((placed.offset, placed.size))
                    }),
                    name: desc.name.to_owned(),
                },
            })
            .collect();
        let resolved_buffers: Vec<Option<&Buffer>> = buffers
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                BufferEntry::Imported(buffer) => Some(&buffer.buffer),
                BufferEntry::Transient(_) => transient_buffer(i).map(|b| &b.buffer),
            })
            .collect();
        let mut buffer_states: Vec<ResourceState> = buffers
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                BufferEntry::Imported(buffer) => buffer.state.get(),
                BufferEntry::Transient(_) => {
                    transient_buffer(i).map_or(ResourceState::UNDEFINED, |b| b.state.get())
                }
            })
            .collect();
        let mut image_sync: Vec<QueueSync> = images
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                ImageEntry::Imported(image) => image.sync.get(),
                ImageEntry::Transient(_) => request_of_image[i]
                    .map_or_else(QueueSync::default, |r| {
                        cache.transients[r].image().sync.get()
                    }),
                ImageEntry::Raw(_) => QueueSync::default(),
            })
            .collect();
        let mut buffer_sync: Vec<QueueSync> = buffers
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                BufferEntry::Imported(buffer) => buffer.sync.get(),
                BufferEntry::Transient(_) => {
                    transient_buffer(i).map_or_else(QueueSync::default, |b| b.sync.get())
                }
            })
            .collect();

        // Kept only once the frame is recorded, like the resources' states.
        let mut waited = self.waited;
        let (compiled, batches) = compile_queued(
            &decls,
            &metas,
            &buffer_metas,
            &mut image_states,
            &mut buffer_states,
            &mut image_sync,
            &mut buffer_sync,
            &mut waited,
            self.poison,
            &mut |kind| frames.reserve_value(kind),
        )?;

        // Record.
        let recording = std::time::Instant::now();
        let names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let resources = Resources {
            images: &resolved,
            buffers: &resolved_buffers,
            names: &names,
            extent,
        };
        let is_buffer = |r: &&Request| matches!(r.desc, TransientKind::Buffer(_));
        // The most bytes alive at once: at each pass, the transients whose lifetime spans it.
        let load_bytes = (0..decls.len())
            .map(|p| {
                requests
                    .iter()
                    .filter(|r| r.first <= p && p <= r.last)
                    .map(|r| r.size)
                    .sum::<u64>()
            })
            .max()
            .unwrap_or(0);
        let mut stats = GraphStats {
            passes: passes.len() as u32,
            transient_images: requests.iter().filter(|r| !is_buffer(r)).count() as u32,
            transient_buffers: requests.iter().filter(is_buffer).count() as u32,
            load_bytes,
            transient_bytes: requests.iter().map(|r| r.size).sum(),
            heap_bytes: if cache.placement.aliased {
                cache.placement.heap_size
            } else {
                requests.iter().map(|r| r.size).sum()
            },
            aliased: cache.placement.aliased,
            heap_rebuilds: self.stats.heap_rebuilds,
            pending_destructions: frames.pending_destructions(),
            batches: batches.len() as u32,
            compile_ms: (recording - started).as_secs_f32() * 1e3,
            ..GraphStats::default()
        };
        let timers = frames.timer_slot(slot);
        let mut work = passes.into_iter().zip(&compiled).peekable();
        let mut started = [false; 3];
        for (index, batch) in batches.iter().enumerate() {
            let cb = frames.command_buffer(slot, batch.queue)?;
            timers.start(cb, batch.queue);
            let commands = Commands::new(&self.device, cb).with_timers(&timers);
            if self.frame_barrier && !started[batch.queue.index()] {
                // Debugging aid (`FORGE_FRAME_BARRIER=1`): serialise frames on the GPU.
                let everything = vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE;
                commands.memory_barrier(
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    everything,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    everything,
                );
            }
            started[batch.queue.index()] = true;
            while let Some((pass, plan)) = work.next_if(|(_, plan)| plan.batch == index) {
                if !plan.zero.is_empty() {
                    commands.barriers(&[], &plan.zero_barriers);
                    for &(image, aspect, mips) in &plan.zero {
                        commands.clear_image_to_zero(image, aspect, mips);
                    }
                }
                let memory = plan.memory_barrier.as_slice();
                commands.barriers(memory, &plan.image_barriers);
                stats.image_barriers += plan.image_barriers.len() as u32;
                stats.memory_barriers += memory.len() as u32;
                stats.queue_passes[batch.queue.index()] += 1;
                commands.set_pass(pass.decl.label);
                (pass.run)(&resources, &commands)?;
                if !plan.poison.is_empty() {
                    commands.barriers(plan.poison_barrier.as_slice(), &[]);
                    for &handle in &plan.poison {
                        let buffer = resources.buffer(BufferHandle(handle));
                        commands.fill_buffer(buffer, 0, buffer.size() & !3, 0xDEAD_BEEF);
                    }
                }
                if let Some(label) = plan.mark {
                    commands.mark(label);
                }
            }
            frames.push_batch(Batch {
                queue: batch.queue,
                command_buffer: cb,
                waits: batch.waits,
                signal: batch.signal,
            });
        }

        stats.record_ms = recording.elapsed().as_secs_f32() * 1e3;

        // Persist the states.
        for (i, entry) in images.iter().enumerate() {
            let states = &image_states[i];
            match entry {
                ImageEntry::Imported(image) => {
                    for (cell, state) in image.states.iter().zip(states) {
                        cell.set(*state);
                    }
                    image.sync.set(image_sync[i]);
                }
                ImageEntry::Transient(_) => {
                    if let Some(r) = request_of_image[i] {
                        for (cell, state) in cache.transients[r].image().states.iter().zip(states) {
                            cell.set(*state);
                        }
                        cache.transients[r].image().sync.set(image_sync[i]);
                    }
                }
                ImageEntry::Raw(_) => {}
            }
        }
        for (i, entry) in buffers.iter().enumerate() {
            let buffer = match entry {
                BufferEntry::Imported(buffer) => Some(*buffer),
                BufferEntry::Transient(_) => transient_buffer(i),
            };
            if let Some(buffer) = buffer {
                buffer.state.set(buffer_states[i]);
                buffer.sync.set(buffer_sync[i]);
            }
        }
        self.waited = waited;
        self.stats = stats;

        if self.log {
            let mut hasher = std::hash::DefaultHasher::new();
            for (decl, plan) in decls.iter().zip(&compiled) {
                decl.label.hash(&mut hasher);
                plan.image_barriers.len().hash(&mut hasher);
                plan.memory_barrier.is_some().hash(&mut hasher);
                plan.batch.hash(&mut hasher);
            }
            cache.placement.entries.len().hash(&mut hasher);
            let hash = hasher.finish();
            if hash != self.last_plan {
                self.last_plan = hash;
                tracing::info!(
                    "render graph plan:\n{}",
                    describe(
                        &decls,
                        &metas,
                        &compiled,
                        &batches,
                        &cache.placement,
                        &stats
                    )
                );
            }
        }
        Ok(stats)
    }

    /// The passes a transient's memory is held through: its uses, or the whole frame of
    /// `passes` when `FORGE_GRAPH_NO_ALIAS` names it.
    fn lifetime(&self, name: &str, first: usize, last: usize, passes: usize) -> (usize, usize) {
        if self.alone.iter().any(|w| name.contains(w.as_str())) {
            (0, passes)
        } else {
            (first, last)
        }
    }

    /// The usage every transient image gains: `TRANSFER_DST` for `FORGE_GRAPH_ZERO`'s clears.
    fn extra_usage(&self) -> vk::ImageUsageFlags {
        if self.zero.is_some() {
            vk::ImageUsageFlags::TRANSFER_DST
        } else {
            vk::ImageUsageFlags::empty()
        }
    }

    fn rebuild_cache(&mut self, placement: Placement, frames: &mut Frames) -> Result<()> {
        let heap = if placement.aliased && placement.heap_size > 0 {
            Some(self.device.create_transient_heap(
                placement.heap_size,
                placement.alignment,
                placement.memory_type_bits,
                "render graph transients",
            )?)
        } else {
            None
        };
        let mut transients = Vec::with_capacity(placement.entries.len());
        for placed in &placement.entries {
            let transient = match placed.desc {
                TransientKind::Image(desc) => {
                    let desc = desc.image_desc(self.extra_usage());
                    let image = match &heap {
                        Some(heap) => self.device.create_image_in(desc, heap, placed.offset)?,
                        None => self
                            .device
                            .allocate_image(&desc, MemoryCategory::Transient)?,
                    };
                    CachedTransient::Image(GraphImage::wrap(
                        &self.device,
                        image,
                        &desc,
                        ResourceState::UNDEFINED,
                    ))
                }
                TransientKind::Buffer(desc) => {
                    let desc = BufferDesc {
                        size: desc.size,
                        usage: desc.usage(),
                        location: gpu_allocator::MemoryLocation::GpuOnly,
                        category: MemoryCategory::Transient,
                        name: desc.name,
                    };
                    let buffer = match &heap {
                        Some(heap) => self.device.create_buffer_in(desc, heap, placed.offset)?,
                        None => self.device.create_buffer(desc)?,
                    };
                    CachedTransient::Buffer(GraphBuffer::new(buffer))
                }
            };
            transients.push(transient);
        }
        if let Some(old) = self.cache.take() {
            frames.destroy_later(old);
        }
        self.stats.heap_rebuilds += 1;
        tracing::info!(
            transients = placement.entries.len(),
            heap_bytes = placement.heap_size,
            requested_bytes = placement.entries.iter().map(|p| p.size).sum::<u64>(),
            aliased = placement.aliased,
            "render graph transients laid out"
        );
        self.cache = Some(TransientCache {
            placement,
            _heap: heap,
            transients,
        });
        Ok(())
    }
}

/// The compiled plan as text (`FORGE_GRAPH_LOG`).
fn describe(
    passes: &[PassDecl],
    images: &[ImageMeta],
    compiled: &[CompiledPass],
    batches: &[CompiledBatch],
    placement: &Placement,
    stats: &GraphStats,
) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "  {} passes, {} image barriers, {} memory barriers; transients {} images + {} buffers, {} KB requested, {} KB heap, {} KB alive at most{}",
        stats.passes,
        stats.image_barriers,
        stats.memory_barriers,
        stats.transient_images,
        stats.transient_buffers,
        stats.transient_bytes / 1024,
        stats.heap_bytes / 1024,
        stats.load_bytes / 1024,
        if stats.aliased { " (aliased)" } else { "" }
    );
    for placed in &placement.entries {
        let _ = writeln!(
            text,
            "  transient '{}' at {} KB, {} KB",
            placed.desc.name(),
            placed.offset / 1024,
            placed.size / 1024
        );
    }
    let mut batch = usize::MAX;
    for (pass, plan) in passes.iter().zip(compiled) {
        if plan.batch != batch {
            batch = plan.batch;
            let b = &batches[batch];
            let mut waits = String::new();
            for kind in QueueKind::ALL {
                let (value, stages) = b.waits[kind.index()];
                if value > 0 {
                    let _ = write!(waits, ", waits for {} {value} at {stages:?}", kind.name());
                }
            }
            let _ = writeln!(
                text,
                "  batch {batch} on {} (signals {}{waits})",
                b.queue.name(),
                b.signal
            );
        }
        let _ = writeln!(text, "  pass '{}'", pass.label);
        for b in &plan.image_barriers {
            let name = images
                .iter()
                .find(|m| m.raw == b.image)
                .map(|m| m.name.as_str())
                .unwrap_or("?");
            let _ = writeln!(
                text,
                "    image '{}' mips {}+{}: {:?} -> {:?}, {:?}/{:?} -> {:?}/{:?}",
                name,
                b.subresource_range.base_mip_level,
                b.subresource_range.level_count,
                b.old_layout,
                b.new_layout,
                b.src_stage_mask,
                b.src_access_mask,
                b.dst_stage_mask,
                b.dst_access_mask
            );
        }
        if let Some(b) = &plan.memory_barrier {
            let _ = writeln!(
                text,
                "    memory: {:?}/{:?} -> {:?}/{:?}",
                b.src_stage_mask, b.src_access_mask, b.dst_stage_mask, b.dst_access_mask
            );
        }
        if let Some(label) = plan.mark {
            let _ = writeln!(text, "    mark '{label}'");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use vk::{AccessFlags2 as A, ImageLayout as L, PipelineStageFlags2 as S};

    #[test]
    fn a_host_read_waits_for_the_copy_and_the_next_copy_for_the_host() {
        let mut state = BufferAccess::TransferDst.state();
        let (src, dst) = transition(&mut state, BufferAccess::HostRead.state()).expect("barrier");
        assert_eq!((src.stage, src.access), (S::TRANSFER, A::TRANSFER_WRITE));
        assert_eq!((dst.stage, dst.access), (S::HOST, A::HOST_READ));
        // Two frames later the slot's next copy overwrites it: a write after the host read.
        let (src, _) = transition(&mut state, BufferAccess::TransferDst.state()).expect("barrier");
        assert_eq!((src.stage, src.access), (S::HOST, A::HOST_READ));
    }

    #[test]
    fn a_custom_access_covers_every_stage_it_names() {
        // DLSS: a clear at the transfer stage, then compute writes, in `GENERAL`.
        let dlss = ImageAccess::Custom {
            layout: L::GENERAL,
            stages: S::CLEAR | S::COMPUTE_SHADER,
            access: A::TRANSFER_WRITE | A::SHADER_STORAGE_WRITE,
        };
        assert!(dlss.initialises());
        let mut state = ImageAccess::Sampled(S::FRAGMENT_SHADER).state(L::GENERAL);
        let (src, dst) = transition(&mut state, dlss.state(L::GENERAL)).expect("barrier");
        assert_eq!(src.stage, S::FRAGMENT_SHADER);
        assert_eq!(dst.stage, S::CLEAR | S::COMPUTE_SHADER);
        assert_eq!(dst.access, A::TRANSFER_WRITE | A::SHADER_STORAGE_WRITE);
        // The display pass then waits for both the clear and the compute writes.
        let (src, _) = transition(
            &mut state,
            ImageAccess::Sampled(S::FRAGMENT_SHADER).state(L::GENERAL),
        )
        .expect("barrier");
        assert_eq!(src.access, A::TRANSFER_WRITE | A::SHADER_STORAGE_WRITE);
        // A read-only custom access is not a write.
        let read = ImageAccess::Custom {
            layout: L::GENERAL,
            stages: S::COMPUTE_SHADER,
            access: A::SHADER_SAMPLED_READ,
        };
        assert!(!read.initialises() && !read.state(L::GENERAL).write);
    }

    fn meta(name: &str, mips: u32, transient: Option<Option<(u64, u64)>>) -> ImageMeta {
        ImageMeta {
            raw: vk::Image::null(),
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: mips,
            sampled_layout: if transient.is_some() {
                L::SHADER_READ_ONLY_OPTIMAL
            } else {
                L::GENERAL
            },
            transient,
            concurrent: true,
            zero: false,
            name: name.to_owned(),
        }
    }

    fn pass(label: &'static str, images: &[(u32, Option<u32>, ImageAccess)]) -> PassDecl {
        PassDecl {
            label,
            images: images
                .iter()
                .map(|&(h, mip, access)| ImageUse {
                    handle: ImageHandle(h),
                    mip,
                    access,
                })
                .collect(),
            buffers: Vec::new(),
            queue: QueueKind::Graphics,
        }
    }

    #[test]
    fn a_read_after_a_write_is_one_barrier_and_reads_accumulate() {
        let images = [meta("color", 1, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED]];
        let passes = [
            pass("a/draw", &[(0, None, ImageAccess::ColorAttachment)]),
            pass(
                "b/read",
                &[(0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER))],
            ),
            pass(
                "c/read",
                &[(0, None, ImageAccess::Sampled(S::COMPUTE_SHADER))],
            ),
            pass(
                "c/read again",
                &[(0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER))],
            ),
            pass("d/draw", &[(0, None, ImageAccess::ColorAttachment)]),
        ];
        let plan = compile(&passes, &images, &mut states, &mut []).unwrap();
        let first = &plan[0].image_barriers[0];
        assert_eq!(first.old_layout, L::UNDEFINED);
        assert_eq!(first.new_layout, L::COLOR_ATTACHMENT_OPTIMAL);
        assert_eq!(first.src_stage_mask, S::NONE);
        let read = &plan[1].image_barriers[0];
        assert_eq!(read.src_stage_mask, S::COLOR_ATTACHMENT_OUTPUT);
        assert_eq!(
            read.src_access_mask,
            A::COLOR_ATTACHMENT_READ | A::COLOR_ATTACHMENT_WRITE
        );
        assert_eq!(read.dst_stage_mask, S::FRAGMENT_SHADER);
        assert_eq!(
            read.new_layout,
            L::GENERAL,
            "imported storage-capable image samples in GENERAL"
        );
        // A read in a stage the first reader did not cover waits for it, which waited for the
        // write (issue #71); the layout stays.
        let chained = &plan[2].image_barriers[0];
        assert_eq!(chained.src_stage_mask, S::FRAGMENT_SHADER);
        assert_eq!(chained.dst_stage_mask, S::COMPUTE_SHADER);
        assert_eq!(chained.old_layout, L::GENERAL);
        assert_eq!(chained.new_layout, L::GENERAL);
        assert!(
            plan[3].image_barriers.is_empty(),
            "a read the earlier readers cover needs no barrier"
        );
        let write = &plan[4].image_barriers[0];
        assert_eq!(
            write.src_stage_mask,
            S::FRAGMENT_SHADER | S::COMPUTE_SHADER,
            "the write waits for every reader"
        );
        assert_eq!(states[0][0].layout, L::COLOR_ATTACHMENT_OPTIMAL);
        assert!(states[0][0].write);
        assert_eq!(plan.iter().filter(|p| p.mark.is_some()).count(), 5);
    }

    #[test]
    fn consecutive_passes_with_one_label_share_a_zone() {
        let images = [meta("hzb", 3, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED; 3]];
        let passes = [
            pass(
                "g/pyramid",
                &[(0, Some(0), ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
            ),
            pass(
                "g/pyramid",
                &[
                    (0, Some(0), ImageAccess::Sampled(S::COMPUTE_SHADER)),
                    (0, Some(1), ImageAccess::StorageWrite(S::COMPUTE_SHADER)),
                ],
            ),
            pass(
                "g/pyramid",
                &[
                    (0, Some(1), ImageAccess::Sampled(S::COMPUTE_SHADER)),
                    (0, Some(2), ImageAccess::StorageWrite(S::COMPUTE_SHADER)),
                ],
            ),
            pass(
                "g/draw",
                &[(0, None, ImageAccess::Sampled(S::TASK_SHADER_EXT))],
            ),
        ];
        let plan = compile(&passes, &images, &mut states, &mut []).unwrap();
        assert_eq!(plan[0].mark, None);
        assert_eq!(plan[1].mark, None);
        assert_eq!(plan[2].mark, Some("g/pyramid"));
        assert_eq!(plan[3].mark, Some("g/draw"));
        // Per-mip tracking: pass 2 barriers mip 0 (write→read) and mip 1 (undefined→write).
        let mips: Vec<u32> = plan[1]
            .image_barriers
            .iter()
            .map(|b| b.subresource_range.base_mip_level)
            .collect();
        assert_eq!(mips, vec![0, 1]);
        // The whole-image read at the end, in the task stage: mips 0 and 1 were last read by
        // compute (same layout: one barrier chained from those reads, issue #71), mip 2 was
        // written (a barrier from the write).
        let ranges: Vec<(u32, u32, vk::PipelineStageFlags2)> = plan[3]
            .image_barriers
            .iter()
            .map(|b| {
                (
                    b.subresource_range.base_mip_level,
                    b.subresource_range.level_count,
                    b.src_stage_mask,
                )
            })
            .collect();
        assert_eq!(
            ranges,
            vec![(0, 2, S::COMPUTE_SHADER), (2, 1, S::COMPUTE_SHADER)]
        );
        assert_eq!(
            plan[3].image_barriers[1].src_access_mask,
            A::SHADER_STORAGE_WRITE
        );
    }

    #[test]
    fn a_transient_read_before_a_write_is_an_error_and_a_double_declaration_too() {
        let images = [meta("t", 1, Some(Some((0, 16))))];
        let mut states = vec![vec![ResourceState::UNDEFINED]];
        let passes = [pass(
            "a/read",
            &[(0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER))],
        )];
        assert!(compile(&passes, &images, &mut states, &mut []).is_err());
        let passes = [pass(
            "a/twice",
            &[
                (0, None, ImageAccess::ColorAttachment),
                (0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER)),
            ],
        )];
        assert!(compile(&passes, &images, &mut states, &mut []).is_err());
    }

    #[test]
    fn an_aliased_transient_waits_for_its_previous_occupant() {
        // `a` and `b` share the memory range [0, 16); `c` lives elsewhere.
        let images = [
            meta("a", 1, Some(Some((0, 16)))),
            meta("b", 1, Some(Some((0, 16)))),
            meta("c", 1, Some(Some((16, 16)))),
        ];
        let mut states = vec![vec![ResourceState::UNDEFINED]; 3];
        let passes = [
            pass("p/a", &[(0, None, ImageAccess::ColorAttachment)]),
            pass(
                "p/a2",
                &[(0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER))],
            ),
            pass(
                "p/c",
                &[(2, None, ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
            ),
            pass("p/b", &[(1, None, ImageAccess::TransferDst)]),
        ];
        let plan = compile(&passes, &images, &mut states, &mut []).unwrap();
        let b = &plan[3].image_barriers[0];
        assert_eq!(b.old_layout, L::UNDEFINED);
        assert!(
            b.src_stage_mask.contains(S::FRAGMENT_SHADER),
            "waits for a's reader"
        );
        assert!(
            !b.src_stage_mask.contains(S::COMPUTE_SHADER),
            "c does not overlap"
        );
        // Next frame: `a` starts from UNDEFINED but waits for `b`'s transfer write.
        let mut next: Vec<Vec<ResourceState>> = states
            .iter()
            .map(|s| {
                s.iter()
                    .map(|st| ResourceState {
                        layout: L::UNDEFINED,
                        ..*st
                    })
                    .collect()
            })
            .collect();
        let plan = compile(&passes[..1], &images, &mut next, &mut []).unwrap();
        let a = &plan[0].image_barriers[0];
        assert!(a.src_stage_mask.contains(S::TRANSFER));
        assert!(a.src_access_mask.contains(A::TRANSFER_WRITE));
    }

    #[test]
    fn buffers_get_one_memory_barrier_per_pass_and_none_on_first_use() {
        let mut buffers = vec![ResourceState::UNDEFINED; 2];
        let passes = [
            PassDecl {
                label: "g/cull",
                images: Vec::new(),
                buffers: vec![
                    BufferUse {
                        handle: BufferHandle(0),
                        access: BufferAccess::ShaderWrite(S::COMPUTE_SHADER),
                    },
                    BufferUse {
                        handle: BufferHandle(1),
                        access: BufferAccess::ShaderWrite(S::COMPUTE_SHADER),
                    },
                ],
                queue: QueueKind::Graphics,
            },
            PassDecl {
                label: "g/draw",
                images: Vec::new(),
                buffers: vec![
                    BufferUse {
                        handle: BufferHandle(0),
                        access: BufferAccess::ShaderRead(S::TASK_SHADER_EXT),
                    },
                    BufferUse {
                        handle: BufferHandle(1),
                        access: BufferAccess::IndirectArgs,
                    },
                ],
                queue: QueueKind::Graphics,
            },
        ];
        let plan = compile(&passes, &[], &mut [], &mut buffers).unwrap();
        assert!(
            plan[0].memory_barrier.is_none(),
            "nothing wrote them before"
        );
        let b = plan[1].memory_barrier.unwrap();
        assert_eq!(b.src_stage_mask, S::COMPUTE_SHADER);
        assert_eq!(b.dst_stage_mask, S::TASK_SHADER_EXT | S::DRAW_INDIRECT);
        assert_eq!(
            b.dst_access_mask,
            A::SHADER_STORAGE_READ | A::INDIRECT_COMMAND_READ
        );
        // Persisted: a write next frame waits for both readers.
        let mut state = buffers[0];
        let (src, _) = transition(
            &mut state,
            BufferAccess::ShaderWrite(S::COMPUTE_SHADER).state(),
        )
        .unwrap();
        assert_eq!(src.stage, S::TASK_SHADER_EXT);
    }

    #[test]
    fn a_pass_that_dispatches_from_a_buffer_and_writes_it_is_a_writer() {
        // Issue #92: cluster cull 1 reads its grid and writes pass 2's in the same buffer.
        let mut buffers = vec![ResourceState::UNDEFINED; 1];
        let pass = |label, access| PassDecl {
            label,
            images: Vec::new(),
            buffers: vec![BufferUse {
                handle: BufferHandle(0),
                access,
            }],
            queue: QueueKind::Graphics,
        };
        let compute = S::COMPUTE_SHADER;
        let passes = [
            pass("g/instance cull", BufferAccess::ShaderWrite(compute)),
            pass(
                "g/cluster cull 1",
                BufferAccess::IndirectArgsAndShaderReadWrite(compute),
            ),
            pass(
                "g/cluster cull 2",
                BufferAccess::IndirectArgsAndShaderRead(compute),
            ),
        ];
        let plan = compile(&passes, &[], &mut [], &mut buffers).unwrap();
        let b = plan[1].memory_barrier.unwrap();
        assert_eq!(b.src_stage_mask, compute);
        assert_eq!(b.dst_stage_mask, S::DRAW_INDIRECT | compute);
        assert!(b.dst_access_mask.contains(A::SHADER_STORAGE_WRITE));
        // Its writes reach the next pass's indirect read.
        let b = plan[2]
            .memory_barrier
            .expect("pass 2 waits for pass 1's writes");
        assert!(b.src_stage_mask.contains(compute));
        assert!(b.src_access_mask.contains(A::SHADER_STORAGE_WRITE));
        assert!(b.dst_stage_mask.contains(S::DRAW_INDIRECT));
        assert!(b.dst_access_mask.contains(A::INDIRECT_COMMAND_READ));
    }

    /// A pass using images and buffers, on the graphics queue.
    fn mixed(
        label: &'static str,
        images: &[(u32, ImageAccess)],
        buffers: &[(u32, BufferAccess)],
    ) -> PassDecl {
        PassDecl {
            label,
            images: images
                .iter()
                .map(|&(h, access)| ImageUse {
                    handle: ImageHandle(h),
                    mip: None,
                    access,
                })
                .collect(),
            buffers: buffers
                .iter()
                .map(|&(h, access)| BufferUse {
                    handle: BufferHandle(h),
                    access,
                })
                .collect(),
            queue: QueueKind::Graphics,
        }
    }

    fn transient_buffer(range: Option<(u64, u64)>) -> BufferMeta {
        BufferMeta {
            transient: Some(range),
            name: "list".to_owned(),
        }
    }

    #[test]
    fn a_transient_buffer_must_be_written_first_and_not_on_another_queue() {
        let compute = S::COMPUTE_SHADER;
        let buffers = [transient_buffer(Some((0, 64)))];
        let run = |passes: &[PassDecl]| {
            compile_queued(
                passes,
                &[],
                &buffers,
                &mut [],
                &mut [ResourceState::UNDEFINED],
                &mut [],
                &mut [QueueSync::default()],
                &mut Waited::default(),
                false,
                &mut |_| 1,
            )
        };
        let read = [mixed(
            "a/read",
            &[],
            &[(0, BufferAccess::ShaderRead(compute))],
        )];
        assert!(run(&read).is_err(), "read before anything wrote it");
        let mut on_compute = mixed("a/write", &[], &[(0, BufferAccess::ShaderWrite(compute))]);
        on_compute.queue = QueueKind::Compute;
        assert!(
            run(&[on_compute]).is_err(),
            "its memory may be aliased while it runs"
        );
        let fill = [
            mixed("a/clear", &[], &[(0, BufferAccess::TransferDst)]),
            mixed("a/read", &[], &[(0, BufferAccess::ShaderRead(compute))]),
        ];
        assert!(run(&fill).is_ok());
    }

    #[test]
    fn a_transient_buffer_waits_for_the_image_whose_memory_it_takes_over() {
        // The image lives in [0, 128), the buffer in [64, 96) after it: the buffer's first
        // write waits for the image's last read, and the next frame's image for the buffer.
        let compute = S::COMPUTE_SHADER;
        let images = [meta("target", 1, Some(Some((0, 128))))];
        let buffers = [transient_buffer(Some((64, 32)))];
        let passes = [
            mixed("a/draw", &[(0, ImageAccess::ColorAttachment)], &[]),
            mixed(
                "b/read",
                &[(0, ImageAccess::Sampled(S::FRAGMENT_SHADER))],
                &[],
            ),
            mixed("c/list", &[], &[(0, BufferAccess::ShaderWrite(compute))]),
            mixed("d/use", &[], &[(0, BufferAccess::ShaderRead(compute))]),
        ];
        let mut image_states = vec![vec![ResourceState::UNDEFINED]];
        let mut buffer_states = [ResourceState::UNDEFINED];
        let frame = |image_states: &mut [Vec<ResourceState>],
                     buffer_states: &mut [ResourceState]| {
            compile_queued(
                &passes,
                &images,
                &buffers,
                image_states,
                buffer_states,
                &mut [QueueSync::default()],
                &mut [QueueSync::default()],
                &mut Waited::default(),
                false,
                &mut |_| 1,
            )
            .unwrap()
            .0
        };
        let plan = frame(&mut image_states, &mut buffer_states);
        let b = plan[2]
            .memory_barrier
            .expect("the buffer waits for the image");
        assert!(b.src_stage_mask.contains(S::FRAGMENT_SHADER));
        assert!(b.src_access_mask.contains(A::SHADER_SAMPLED_READ));
        assert_eq!(b.dst_stage_mask, compute);
        // The next frame: the image's first use waits for the buffer's last read.
        let plan = frame(&mut image_states, &mut buffer_states);
        let b = plan[0].image_barriers[0];
        assert!(b.src_stage_mask.contains(compute));
        assert!(b.src_access_mask.contains(A::SHADER_STORAGE_READ));
    }

    #[test]
    fn the_poison_fills_a_transient_buffer_after_its_last_pass() {
        let compute = S::COMPUTE_SHADER;
        let buffers = [transient_buffer(None), BufferMeta::default()];
        let passes = [
            mixed("a/write", &[], &[(0, BufferAccess::ShaderWrite(compute))]),
            mixed(
                "b/read",
                &[],
                &[
                    (0, BufferAccess::ShaderRead(compute)),
                    (1, BufferAccess::ShaderWrite(compute)),
                ],
            ),
            mixed("c/other", &[], &[(1, BufferAccess::ShaderRead(compute))]),
        ];
        let mut buffer_states = [ResourceState::UNDEFINED; 2];
        let (plan, _) = compile_queued(
            &passes,
            &[],
            &buffers,
            &mut [],
            &mut buffer_states,
            &mut [],
            &mut [QueueSync::default(); 2],
            &mut Waited::default(),
            true,
            &mut |_| 1,
        )
        .unwrap();
        assert!(plan[0].poison.is_empty());
        assert_eq!(
            plan[1].poison,
            vec![0],
            "the transient alone, after its last read"
        );
        let b = plan[1].poison_barrier.expect("the fill waits for the read");
        assert_eq!((b.src_stage_mask, b.dst_stage_mask), (compute, S::TRANSFER));
        assert!(plan[2].poison.is_empty());
        assert_eq!(buffer_states[0], BufferAccess::TransferDst.state());
    }

    #[test]
    fn a_debugging_variable_selects_all_names_or_those_with_its_words() {
        assert!(!selects(&None, "ao raw"));
        assert!(selects(&Some(Vec::new()), "ao raw"));
        let words = Some(vec!["ao".to_owned(), "taa".to_owned()]);
        assert!(selects(&words, "ao raw") && selects(&words, "taa motion"));
        assert!(!selects(&words, "sun penumbra"));
    }

    #[test]
    fn a_zeroed_transient_is_cleared_before_its_first_pass_only() {
        let compute = S::COMPUTE_SHADER;
        let mut zeroed = meta("t", 2, Some(Some((0, 16))));
        zeroed.zero = true;
        let images = [zeroed, meta("u", 1, Some(Some((16, 16))))];
        let mut states = vec![
            vec![ResourceState::UNDEFINED; 2],
            vec![ResourceState::UNDEFINED],
        ];
        let passes = [
            pass(
                "a/write",
                &[
                    (0, None, ImageAccess::StorageWrite(compute)),
                    (1, None, ImageAccess::StorageWrite(compute)),
                ],
            ),
            pass("b/read", &[(0, None, ImageAccess::Sampled(compute))]),
        ];
        let plan = compile(&passes, &images, &mut states, &mut []).unwrap();
        assert_eq!(plan[0].zero.len(), 1, "the zeroed transient alone");
        assert_eq!(plan[0].zero[0].2, 2, "every mip");
        let clear = plan[0].zero_barriers[0];
        assert_eq!(
            (clear.old_layout, clear.new_layout),
            (L::UNDEFINED, L::TRANSFER_DST_OPTIMAL)
        );
        assert_eq!(clear.subresource_range.level_count, 2);
        // The pass's own barrier then starts from the clear, not from undefined contents.
        let own = plan[0]
            .image_barriers
            .iter()
            .find(|b| b.old_layout == L::TRANSFER_DST_OPTIMAL)
            .expect("the write waits for the clear");
        assert_eq!(
            (own.src_stage_mask, own.dst_stage_mask),
            (S::TRANSFER, compute)
        );
        assert!(plan[1].zero.is_empty() && plan[1].zero_barriers.is_empty());
    }

    fn request(name: &'static str, size: u64, first: usize, last: usize) -> Request {
        Request {
            desc: TransientKind::Image(TransientDesc {
                name,
                width: 1,
                height: 1,
                format: vk::Format::R8_UNORM,
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
            }),
            size,
            alignment: 256,
            memory_type_bits: 0b11,
            first,
            last,
        }
    }

    #[test]
    fn transients_with_disjoint_lifetimes_share_memory() {
        let requests = [
            request("depth", 1000, 0, 2),
            request("color", 2000, 0, 3),
            request("motion", 500, 3, 4),
            request("bloom", 300, 4, 5),
        ];
        let placement = plan(&requests, true);
        assert!(placement.aliased);
        let offset = |name: &str| {
            placement
                .entries
                .iter()
                .find(|p| p.desc.name() == name)
                .unwrap()
                .offset
        };
        assert_eq!(offset("color"), 0, "largest first");
        assert_eq!(
            offset("depth"),
            2048,
            "overlaps colour in time: after it, aligned"
        );
        assert_eq!(
            offset("motion"),
            2048,
            "motion reuses depth's memory (depth is dead)"
        );
        assert_eq!(offset("bloom"), 0, "bloom reuses colour's memory");
        assert_eq!(placement.heap_size, 3048);
        assert_eq!(placement.alignment, 256);
        // Every pair that overlaps in time is disjoint in memory.
        for a in &placement.entries {
            for b in &placement.entries {
                if a.desc.name() == b.desc.name() {
                    continue;
                }
                let ra = requests
                    .iter()
                    .find(|r| r.desc.name() == a.desc.name())
                    .unwrap();
                let rb = requests
                    .iter()
                    .find(|r| r.desc.name() == b.desc.name())
                    .unwrap();
                let in_time = ra.first <= rb.last && rb.first <= ra.last;
                let in_memory = a.offset < b.offset + b.size && b.offset < a.offset + a.size;
                assert!(
                    !(in_time && in_memory),
                    "{} and {} collide",
                    a.desc.name(),
                    b.desc.name()
                );
            }
        }
        let separate = plan(&requests, false);
        assert!(!separate.aliased);
        assert!(separate.entries.iter().all(|p| p.offset == 0));
        let mut incompatible = requests;
        incompatible[0].memory_type_bits = 0b100;
        assert!(
            !plan(&incompatible, true).aliased,
            "no common memory type: no heap"
        );
    }

    fn on(queue: QueueKind, mut decl: PassDecl) -> PassDecl {
        decl.queue = queue;
        decl
    }

    /// The queues' timelines and what each queue waited for, from frame to frame.
    #[derive(Default)]
    struct Queues {
        next: [u64; 3],
        waited: Waited,
    }

    /// Runs the scheduler and the compiler as `execute` does, from persistent `sync` and
    /// `queues`.
    fn frame(
        passes: &[PassDecl],
        images: &[ImageMeta],
        states: &mut [Vec<ResourceState>],
        sync: &mut [QueueSync],
        queues: &mut Queues,
    ) -> Result<(Vec<&'static str>, Vec<CompiledPass>, Vec<CompiledBatch>)> {
        let order = schedule(passes);
        let ordered: Vec<PassDecl> = order.iter().map(|&i| passes[i].clone()).collect();
        let next = &mut queues.next;
        let (compiled, batches) = compile_queued(
            &ordered,
            images,
            &[],
            states,
            &mut [],
            sync,
            &mut [],
            &mut queues.waited,
            false,
            &mut |kind| {
                next[kind.index()] += 1;
                next[kind.index()]
            },
        )?;
        Ok((ordered.iter().map(|p| p.label).collect(), compiled, batches))
    }

    #[test]
    fn an_async_pass_moves_up_to_its_last_conflict_and_graphics_waits_for_its_result() {
        use QueueKind::{Compute, Graphics};
        let images = [meta("x", 1, None), meta("y", 1, None), meta("z", 1, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED]; 3];
        let mut sync = vec![QueueSync::default(); 3];
        let mut next = Queues::default();
        let write = ImageAccess::StorageWrite(S::COMPUTE_SHADER);
        let read = ImageAccess::Sampled(S::COMPUTE_SHADER);
        let passes = [
            pass("g/a", &[(0, None, write)]),
            pass("g/b", &[(1, None, write)]),
            on(
                Compute,
                pass("c/probes", &[(0, None, read), (2, None, write)]),
            ),
            pass("g/c", &[(2, None, read)]),
        ];
        let (order, compiled, batches) =
            frame(&passes, &images, &mut states, &mut sync, &mut next).unwrap();
        // The probes need only what g/a wrote: they run beside g/b.
        assert_eq!(order, ["g/a", "c/probes", "g/b", "g/c"]);
        assert_eq!(
            compiled.iter().map(|c| c.batch).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        let queues: Vec<_> = batches.iter().map(|b| b.queue).collect();
        assert_eq!(queues, [Graphics, Compute, Graphics, Graphics]);
        assert_eq!(batches[1].waits[Graphics.index()], (1, S::COMPUTE_SHADER));
        assert_eq!(batches[2].waits, [(0, S::NONE); 3], "g/b waits for nothing");
        assert_eq!(
            batches[3].waits[Compute.index()].0,
            1,
            "g/c waits for the probes"
        );
        // Crossing queues, the reads are ordered after everything their queue did before.
        let barrier = compiled[3].image_barriers[0];
        assert_eq!(barrier.src_stage_mask, S::ALL_COMMANDS);
        assert_eq!(sync[2].readers[Graphics.index()], batches[3].signal);

        // Next frame the probes write z again: after last frame's graphics read of it.
        let passes = [
            on(
                Compute,
                pass("c/probes", &[(0, None, read), (2, None, write)]),
            ),
            pass("g/c", &[(2, None, read)]),
        ];
        let (_, _, next_batches) =
            frame(&passes, &images, &mut states, &mut sync, &mut next).unwrap();
        assert_eq!(next_batches[0].queue, Compute);
        assert_eq!(next_batches[0].waits[Graphics.index()].0, batches[3].signal);
    }

    #[test]
    fn a_frame_ends_on_the_graphics_queue_after_every_async_batch() {
        use QueueKind::{Compute, Graphics};
        let images = [meta("x", 1, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED]];
        let mut sync = vec![QueueSync::default()];
        let mut next = Queues::default();
        let passes = [
            pass(
                "g/a",
                &[(0, None, ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
            ),
            on(
                Compute,
                pass(
                    "c/b",
                    &[(0, None, ImageAccess::StorageReadWrite(S::COMPUTE_SHADER))],
                ),
            ),
        ];
        let (_, _, batches) = frame(&passes, &images, &mut states, &mut sync, &mut next).unwrap();
        assert_eq!(batches.len(), 3);
        let join = batches[2];
        assert_eq!(join.queue, Graphics);
        assert_eq!(join.waits[Compute.index()].0, batches[1].signal);
    }

    #[test]
    fn a_wait_an_earlier_batch_of_the_queue_made_is_not_made_again() {
        use QueueKind::{Compute, Graphics, Transfer};
        // The city's streaming (#104): the pool copied on the transfer queue in one frame,
        // read by the culls in every frame after it, the probes rebuilt on the compute queue.
        let images = [
            meta("pool", 1, None),
            meta("probes", 1, None),
            meta("list", 1, None),
        ];
        let mut states = vec![vec![ResourceState::UNDEFINED]; 3];
        let mut sync = vec![QueueSync::default(); 3];
        let mut queues = Queues::default();
        let read = ImageAccess::Sampled(S::COMPUTE_SHADER);
        let list = ImageAccess::StorageReadWrite(S::COMPUTE_SHADER);
        let clear = pass("g/clear", &[(2, None, ImageAccess::TransferDst)]);
        let upload = on(
            Transfer,
            pass("t/upload", &[(0, None, ImageAccess::TransferDst)]),
        );
        let probes = on(
            Compute,
            pass(
                "c/probes",
                &[(1, None, ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
            ),
        );
        let cull = pass("g/cull", &[(0, None, read), (2, None, list)]);
        let shade = pass(
            "g/shade",
            &[(0, None, read), (1, None, read), (2, None, list)],
        );
        let passes = [
            clear.clone(),
            upload,
            probes.clone(),
            cull.clone(),
            shade.clone(),
        ];
        let (order, compiled, batches) =
            frame(&passes, &images, &mut states, &mut sync, &mut queues).unwrap();
        assert_eq!(
            order,
            ["c/probes", "t/upload", "g/clear", "g/cull", "g/shade"]
        );
        let copy = batches[compiled[1].batch].signal;
        let (cull_batch, shade_batch) = (batches[compiled[3].batch], batches[compiled[4].batch]);
        assert_eq!(
            cull_batch.waits[Transfer.index()],
            (copy, S::COMPUTE_SHADER)
        );
        // The shading's own wait for the copy is the cull's; it waits for the probes, and as
        // the frame's last batch for the copy at every stage.
        assert_eq!(shade_batch.waits[Compute.index()].0, 1);
        assert_eq!(shade_batch.waits[Transfer.index()], (copy, S::ALL_COMMANDS));
        assert!(
            queues
                .waited
                .covers(Graphics, Transfer, copy, S::VERTEX_SHADER)
        );

        // A frame without a copy: the cull joins the clear's batch, nothing waits for the copy.
        let passes = [clear, probes, cull, shade];
        let (order, compiled, batches) =
            frame(&passes, &images, &mut states, &mut sync, &mut queues).unwrap();
        assert_eq!(order, ["c/probes", "g/clear", "g/cull", "g/shade"]);
        let queues_of: Vec<_> = batches.iter().map(|b| b.queue).collect();
        assert_eq!(queues_of, [Compute, Graphics, Graphics]);
        assert_eq!(compiled[2].batch, compiled[1].batch);
        assert!(batches.iter().all(|b| b.waits[Transfer.index()].0 == 0));
    }

    #[test]
    fn a_wait_at_other_stages_is_still_made() {
        use QueueKind::{Compute, Graphics, Transfer};
        let images = [meta("pool", 1, None), meta("probes", 1, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED]; 2];
        let mut sync = vec![QueueSync::default(); 2];
        let mut queues = Queues::default();
        let passes = [
            on(
                Transfer,
                pass("t/upload", &[(0, None, ImageAccess::TransferDst)]),
            ),
            pass("g/a", &[(0, None, ImageAccess::Sampled(S::COMPUTE_SHADER))]),
            on(
                Compute,
                pass(
                    "c/probes",
                    &[(1, None, ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
                ),
            ),
            pass(
                "g/b",
                &[
                    (0, None, ImageAccess::Sampled(S::FRAGMENT_SHADER)),
                    (1, None, ImageAccess::Sampled(S::COMPUTE_SHADER)),
                ],
            ),
        ];
        let (order, compiled, batches) =
            frame(&passes, &images, &mut states, &mut sync, &mut queues).unwrap();
        assert_eq!(order, ["c/probes", "t/upload", "g/a", "g/b"]);
        // g/a's wait blocked compute shaders only: g/b's fragment shaders wait for the copy.
        let b = batches[compiled[3].batch];
        assert_eq!(b.waits[Transfer.index()], (1, S::FRAGMENT_SHADER));
        assert!(
            !queues
                .waited
                .covers(Graphics, Transfer, 1, S::VERTEX_SHADER)
        );
        assert!(queues.waited.covers(
            Graphics,
            Transfer,
            1,
            S::COMPUTE_SHADER | S::FRAGMENT_SHADER
        ));
    }

    #[test]
    fn mips_rebuilt_on_the_compute_queue_after_a_vertex_read_name_no_graphics_stage() {
        use QueueKind::Compute;
        // The water's images (issue #105): level 0 written, the mips built from it on the
        // compute queue, every level sampled by a vertex shader on the graphics queue.
        let images = [meta("waves", 3, None)];
        let mut states = vec![vec![ResourceState::UNDEFINED; 3]];
        let mut sync = vec![QueueSync::default()];
        let mut next = Queues::default();
        let write = ImageAccess::StorageWrite(S::COMPUTE_SHADER);
        let read = ImageAccess::Sampled(S::COMPUTE_SHADER);
        let passes = [
            on(Compute, pass("c/derive", &[(0, Some(0), write)])),
            on(
                Compute,
                pass("c/mips", &[(0, Some(0), read), (0, Some(1), write)]),
            ),
            on(
                Compute,
                pass("c/mips", &[(0, Some(1), read), (0, Some(2), write)]),
            ),
            pass(
                "g/surface",
                &[(0, None, ImageAccess::Sampled(S::VERTEX_SHADER))],
            ),
        ];
        for _ in 0..2 {
            let (_, compiled, batches) =
                frame(&passes, &images, &mut states, &mut sync, &mut next).unwrap();
            for (c, pass) in compiled.iter().enumerate() {
                if batches[pass.batch].queue != Compute {
                    continue;
                }
                for barrier in &pass.image_barriers {
                    assert!(
                        !barrier
                            .src_stage_mask
                            .intersects(S::VERTEX_SHADER | S::FRAGMENT_SHADER),
                        "compute pass {c} waits on a graphics stage: {:?}",
                        barrier.src_stage_mask
                    );
                }
            }
        }
    }

    #[test]
    fn an_async_pass_may_not_use_a_transient_or_a_render_target() {
        let mut states = vec![vec![ResourceState::UNDEFINED]];
        let passes = [on(
            QueueKind::Compute,
            pass(
                "c/a",
                &[(0, None, ImageAccess::StorageWrite(S::COMPUTE_SHADER))],
            ),
        )];
        let transient = [meta("t", 1, Some(None))];
        let mut sync = vec![QueueSync::default()];
        assert!(
            frame(
                &passes,
                &transient,
                &mut states,
                &mut sync,
                &mut Queues::default()
            )
            .is_err()
        );
        let mut target = meta("target", 1, None);
        target.concurrent = false;
        assert!(
            frame(
                &passes,
                &[target],
                &mut states,
                &mut sync,
                &mut Queues::default()
            )
            .is_err()
        );
    }
}
