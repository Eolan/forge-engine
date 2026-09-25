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
//! Rules: one graphics queue; passes run in declaration order; nothing is culled or
//! reordered (every declared pass runs, every declared access is honoured). Consecutive
//! passes with the same label share one profiler zone. Cross-queue passes (async compute,
//! transfers) are the next extension: a pass gets a queue and edges that cross queues become
//! timeline waits and ownership transfers. Immutable resources (textures uploaded once) are
//! imported with their upload state ([`GraphImage::uploaded`]): declaring them costs nothing
//! and keeps the rule that every access is declared.
//!
//! Transients (`depth`, the HDR colour target, motion vectors) live in one heap laid out from
//! their lifetimes: two images whose pass ranges never overlap share memory. The first use of
//! a transient in a frame starts from `UNDEFINED` (its contents never survive a frame) and
//! waits for the last use of every image overlapping its memory, in this frame or the
//! previous one, which is what makes aliasing safe across frames in flight.
//!
//! Debugging: `FORGE_GRAPH_LOG=1` logs the compiled plan (passes, barriers, transient
//! placement) whenever it changes; `FORGE_GRAPH_NO_ALIAS=1` gives every transient its own
//! memory, to tell an aliasing bug from anything else.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ash::vk;

use crate::bindless::{SampledImageId, StorageImageId};
use crate::commands::Commands;
use crate::device::Device;
use crate::error::{GpuError, Result};
use crate::frame::Frames;
use crate::memory::{Buffer, Image, ImageDesc, TransientHeap};

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
            Self::IndexAndShaderRead(stages) => (
                S::INDEX_INPUT | stages,
                A::INDEX_READ | A::SHADER_STORAGE_READ,
                false,
            ),
            Self::TransferSrc => (S::TRANSFER, A::TRANSFER_READ, false),
            Self::TransferDst => (S::TRANSFER, A::TRANSFER_WRITE, true),
            Self::HostRead => (S::HOST, A::HOST_READ, false),
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
}

impl GraphBuffer {
    /// Wraps a buffer whose contents so far were written by the host or by a completed
    /// upload (nothing to wait for).
    pub fn new(buffer: Buffer) -> Self {
        Self {
            buffer,
            state: Cell::new(ResourceState::UNDEFINED),
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
    fn image_desc(&self) -> ImageDesc<'static> {
        ImageDesc {
            width: self.width,
            height: self.height,
            format: self.format,
            usage: self.usage,
            aspect: self.aspect,
            mip_levels: self.mip_levels.max(1),
            name: self.name,
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
    buffers: Vec<&'f GraphBuffer>,
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
        self.buffers.push(buffer);
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

    /// The pass uses `buffer` as `access`.
    pub fn buffer(mut self, buffer: BufferHandle, access: BufferAccess) -> Self {
        self.decl.buffers.push(BufferUse {
            handle: buffer,
            access,
        });
        self
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
    names: &'r [String],
    extent: vk::Extent2D,
}

impl Resources<'_> {
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

/// What the compiler needs to know about an image.
#[derive(Clone, Debug)]
struct ImageMeta {
    raw: vk::Image,
    aspect: vk::ImageAspectFlags,
    mip_levels: u32,
    sampled_layout: vk::ImageLayout,
    /// For transients: the memory range in the heap (`None` when not aliased).
    transient: Option<Option<(u64, u64)>>,
    name: String,
}

/// The barriers and mark of one pass.
#[derive(Clone, Debug, Default)]
struct CompiledPass {
    image_barriers: Vec<vk::ImageMemoryBarrier2<'static>>,
    memory_barrier: Option<vk::MemoryBarrier2<'static>>,
    /// Closes the profiler zone after the pass (the next pass has another label).
    mark: Option<&'static str>,
}

/// Derives the barriers of every pass and advances `image_states` / `buffer_states` to
/// the end of the frame. Pure: tests run it on null handles.
fn compile(
    passes: &[PassDecl],
    images: &[ImageMeta],
    image_states: &mut [Vec<ResourceState>],
    buffer_states: &mut [ResourceState],
) -> Result<Vec<CompiledPass>> {
    let mut first_use = vec![true; images.len()];
    let mut compiled = Vec::with_capacity(passes.len());
    for (index, pass) in passes.iter().enumerate() {
        let mut out = CompiledPass::default();
        let mut seen: Vec<(u32, Option<u32>)> = Vec::new();
        for use_ in &pass.images {
            let i = use_.handle.0 as usize;
            let meta = images.get(i).ok_or_else(|| {
                GpuError::Graph(format!("pass '{}' uses an unknown image", pass.label))
            })?;
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
                // earlier occupants, or last frame's (their states persist in the cache).
                let mut src = ResourceState::UNDEFINED;
                for (j, other) in images.iter().enumerate() {
                    let overlaps = match (range, other.transient) {
                        (Some((a, la)), Some(Some((b, lb)))) => a < b + lb && b < a + la,
                        _ => j == i,
                    };
                    if overlaps {
                        for s in &image_states[j] {
                            src.stage |= s.stage;
                            src.access |= s.access;
                        }
                    }
                }
                image_states[i].fill(ResourceState {
                    layout: vk::ImageLayout::UNDEFINED,
                    write: true,
                    ..src
                });
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
            // One barrier per run of consecutive mips that need the same transition.
            let mut run: Option<(u32, u32, ResourceState, ResourceState)> = None;
            for level in base..base + count {
                let step = transition(&mut image_states[i][level as usize], dst);
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
        }
        let mut seen_buffers: Vec<u32> = Vec::new();
        let mut memory: Option<vk::MemoryBarrier2<'static>> = None;
        for use_ in &pass.buffers {
            let i = use_.handle.0 as usize;
            let state = buffer_states.get_mut(i).ok_or_else(|| {
                GpuError::Graph(format!("pass '{}' uses an unknown buffer", pass.label))
            })?;
            if seen_buffers.contains(&use_.handle.0) {
                return Err(GpuError::Graph(format!(
                    "pass '{}' declares a buffer twice",
                    pass.label
                )));
            }
            seen_buffers.push(use_.handle.0);
            if let Some((src, dst)) = transition(state, use_.access.state())
                && src.stage != vk::PipelineStageFlags2::NONE
            {
                let barrier = memory.get_or_insert_with(vk::MemoryBarrier2::default);
                barrier.src_stage_mask |= src.stage;
                barrier.src_access_mask |= src.access;
                barrier.dst_stage_mask |= dst.stage;
                barrier.dst_access_mask |= dst.access;
            }
        }
        out.memory_barrier = memory;
        let next_label = passes.get(index + 1).map(|p| p.label);
        if next_label != Some(pass.label) {
            out.mark = Some(pass.label);
        }
        compiled.push(out);
    }
    Ok(compiled)
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
    desc: TransientDesc,
    size: u64,
    alignment: u64,
    memory_type_bits: u32,
    first: usize,
    last: usize,
}

/// Where a transient lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    desc: TransientDesc,
    /// Offset in the heap (0 when not aliased: the image then has its own memory).
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
    images: Vec<GraphImage>,
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
    /// Bytes the transients would need with their own memory each.
    pub transient_bytes: u64,
    /// Bytes of the shared heap (equal to `transient_bytes` when not aliased).
    pub heap_bytes: u64,
    /// Whether the transients share one heap.
    pub aliased: bool,
    /// Times the heap was rebuilt since start (a resize, a changed pass list).
    pub heap_rebuilds: u64,
    /// Resources waiting in the deferred-deletion queue.
    pub pending_destructions: usize,
}

/// The persistent side of the graph: the transient heap, statistics, logging.
pub struct RenderGraph {
    device: Arc<Device>,
    cache: Option<TransientCache>,
    requirements: HashMap<TransientDesc, vk::MemoryRequirements>,
    stats: GraphStats,
    log: bool,
    allow_alias: bool,
    last_plan: u64,
}

impl RenderGraph {
    /// A graph for `device`.
    pub fn new(device: &Arc<Device>) -> Self {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v != "0");
        Self {
            device: Arc::clone(device),
            cache: None,
            requirements: HashMap::new(),
            stats: GraphStats::default(),
            log: flag("FORGE_GRAPH_LOG"),
            allow_alias: !flag("FORGE_GRAPH_NO_ALIAS"),
            last_plan: 0,
        }
    }

    /// Counters of the last executed frame.
    pub fn stats(&self) -> GraphStats {
        self.stats
    }

    /// Lays out the transients, derives the barriers, records every pass into `commands` and
    /// writes the final resource states back. `frames` receives resources retired by a
    /// transient relayout.
    pub fn execute(
        &mut self,
        frame: FrameGraph<'_>,
        commands: &Commands<'_>,
        frames: &mut Frames,
    ) -> Result<GraphStats> {
        let FrameGraph {
            extent,
            images,
            buffers,
            passes,
        } = frame;
        let decls: Vec<PassDecl> = passes.iter().map(|p| p.decl.clone()).collect();

        // Transient lifetimes, then their placement; reuse the cache when it matches.
        let mut requests = Vec::new();
        let mut request_of_image: Vec<Option<usize>> = vec![None; images.len()];
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
            let req = *self
                .requirements
                .entry(*desc)
                .or_insert_with(|| self.device.image_memory_requirements(&desc.image_desc()));
            request_of_image[i] = Some(requests.len());
            requests.push(Request {
                desc: *desc,
                size: req.size,
                alignment: req.alignment,
                memory_type_bits: req.memory_type_bits,
                first,
                last,
            });
        }
        let placement = plan(&requests, self.allow_alias);
        if self.cache.as_ref().is_none_or(|c| c.placement != placement) {
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
                        sampled: None,
                        storage: Vec::new(),
                    });
                    image_states.push(vec![raw.state]);
                }
                ImageEntry::Transient(desc) => match request_of_image[i] {
                    Some(r) => {
                        let image = &cache.images[r];
                        let placed = cache.placement.entries[r];
                        let mut meta = image.meta();
                        meta.transient = Some(
                            cache
                                .placement
                                .aliased
                                .then_some((placed.offset, placed.size)),
                        );
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
        let mut buffer_states: Vec<ResourceState> = buffers.iter().map(|b| b.state.get()).collect();

        let compiled = compile(&decls, &metas, &mut image_states, &mut buffer_states)?;

        // Record.
        let names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let resources = Resources {
            images: &resolved,
            names: &names,
            extent,
        };
        let mut stats = GraphStats {
            passes: passes.len() as u32,
            transient_images: requests.len() as u32,
            transient_bytes: requests.iter().map(|r| r.size).sum(),
            heap_bytes: if cache.placement.aliased {
                cache.placement.heap_size
            } else {
                requests.iter().map(|r| r.size).sum()
            },
            aliased: cache.placement.aliased,
            heap_rebuilds: self.stats.heap_rebuilds,
            pending_destructions: frames.pending_destructions(),
            ..GraphStats::default()
        };
        for (pass, plan) in passes.into_iter().zip(&compiled) {
            let memory = plan.memory_barrier.as_slice();
            commands.barriers(memory, &plan.image_barriers);
            stats.image_barriers += plan.image_barriers.len() as u32;
            stats.memory_barriers += memory.len() as u32;
            commands.set_pass(pass.decl.label);
            (pass.run)(&resources, commands)?;
            if let Some(label) = plan.mark {
                commands.mark(label);
            }
        }

        // Persist the states.
        for (i, entry) in images.iter().enumerate() {
            let states = &image_states[i];
            match entry {
                ImageEntry::Imported(image) => {
                    for (cell, state) in image.states.iter().zip(states) {
                        cell.set(*state);
                    }
                }
                ImageEntry::Transient(_) => {
                    if let Some(r) = request_of_image[i] {
                        for (cell, state) in cache.images[r].states.iter().zip(states) {
                            cell.set(*state);
                        }
                    }
                }
                ImageEntry::Raw(_) => {}
            }
        }
        for (buffer, state) in buffers.iter().zip(&buffer_states) {
            buffer.state.set(*state);
        }
        self.stats = stats;

        if self.log {
            let mut hasher = std::hash::DefaultHasher::new();
            for (decl, plan) in decls.iter().zip(&compiled) {
                decl.label.hash(&mut hasher);
                plan.image_barriers.len().hash(&mut hasher);
                plan.memory_barrier.is_some().hash(&mut hasher);
            }
            cache.placement.entries.len().hash(&mut hasher);
            let hash = hasher.finish();
            if hash != self.last_plan {
                self.last_plan = hash;
                tracing::info!(
                    "render graph plan:\n{}",
                    describe(&decls, &metas, &compiled, &cache.placement, &stats)
                );
            }
        }
        Ok(stats)
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
        let mut images = Vec::with_capacity(placement.entries.len());
        for placed in &placement.entries {
            let desc = placed.desc.image_desc();
            let image = match &heap {
                Some(heap) => self.device.create_image_in(desc, heap, placed.offset)?,
                None => self
                    .device
                    .allocate_image(&desc, crate::memory_report::MemoryCategory::Transient)?,
            };
            images.push(GraphImage::wrap(
                &self.device,
                image,
                &desc,
                ResourceState::UNDEFINED,
            ));
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
            images,
        });
        Ok(())
    }
}

/// The compiled plan as text (`FORGE_GRAPH_LOG`).
fn describe(
    passes: &[PassDecl],
    images: &[ImageMeta],
    compiled: &[CompiledPass],
    placement: &Placement,
    stats: &GraphStats,
) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "  {} passes, {} image barriers, {} memory barriers; transients {} images, {} KB requested, {} KB heap{}",
        stats.passes,
        stats.image_barriers,
        stats.memory_barriers,
        stats.transient_images,
        stats.transient_bytes / 1024,
        stats.heap_bytes / 1024,
        if stats.aliased { " (aliased)" } else { "" }
    );
    for placed in &placement.entries {
        let _ = writeln!(
            text,
            "  transient '{}' at {} KB, {} KB",
            placed.desc.name,
            placed.offset / 1024,
            placed.size / 1024
        );
    }
    for (pass, plan) in passes.iter().zip(compiled) {
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

    fn request(name: &'static str, size: u64, first: usize, last: usize) -> Request {
        Request {
            desc: TransientDesc {
                name,
                width: 1,
                height: 1,
                format: vk::Format::R8_UNORM,
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
            },
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
                .find(|p| p.desc.name == name)
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
                if a.desc.name == b.desc.name {
                    continue;
                }
                let ra = requests
                    .iter()
                    .find(|r| r.desc.name == a.desc.name)
                    .unwrap();
                let rb = requests
                    .iter()
                    .find(|r| r.desc.name == b.desc.name)
                    .unwrap();
                let in_time = ra.first <= rb.last && rb.first <= ra.last;
                let in_memory = a.offset < b.offset + b.size && b.offset < a.offset + a.size;
                assert!(
                    !(in_time && in_memory),
                    "{} and {} collide",
                    a.desc.name,
                    b.desc.name
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
}
