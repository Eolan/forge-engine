use std::ptr::NonNull;
use std::sync::Arc;

use ash::vk;
use bytemuck::Pod;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::device::Device;
use crate::error::Result;
use crate::memory_report::MemoryCategory;

/// Description of a buffer to create.
#[derive(Clone, Copy, Debug)]
pub struct BufferDesc<'a> {
    /// Size in bytes.
    pub size: u64,
    /// Usage flags; `SHADER_DEVICE_ADDRESS` is always added.
    pub usage: vk::BufferUsageFlags,
    /// Where the memory lives. `CpuToGpu` picks host-visible device-local memory when the
    /// driver exposes it (Resizable BAR), which is what per-frame data wants.
    pub location: MemoryLocation,
    /// What it holds, for the memory counters.
    pub category: MemoryCategory,
    /// Debug name.
    pub name: &'a str,
}

/// A buffer with its memory and device address. Freed on drop: the owner must make sure the
/// GPU is done with it (frame slots, or `Device::wait_idle` during teardown).
pub struct Buffer {
    device: Arc<Device>,
    raw: vk::Buffer,
    allocation: Option<Allocation>,
    category: MemoryCategory,
    size: u64,
    address: vk::DeviceAddress,
    mapped: Option<NonNull<u8>>,
}

// SAFETY: the mapped pointer is only dereferenced through methods that copy whole slices, and
// Vulkan buffers are freely shareable between threads.
unsafe impl Send for Buffer {}
// SAFETY: see above; concurrent reads of the mapping are fine, and the engine only writes a
// per-frame buffer from the one thread that owns that frame slot.
unsafe impl Sync for Buffer {}

impl Buffer {
    /// The Vulkan handle.
    pub fn raw(&self) -> vk::Buffer {
        self.raw
    }

    /// The device address, for shader pointers.
    pub fn address(&self) -> vk::DeviceAddress {
        self.address
    }

    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Whether the buffer is host-visible and mapped.
    pub fn is_mapped(&self) -> bool {
        self.mapped.is_some()
    }

    /// Copies `data` into the mapped buffer at `offset` bytes.
    ///
    /// # Panics
    /// If the buffer is not mapped or the write does not fit.
    pub fn write<T: Pod>(&self, offset: u64, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let mapped = self.mapped.expect("buffer is not host-visible");
        assert!(
            offset + bytes.len() as u64 <= self.size,
            "write exceeds buffer size"
        );
        self.device.memory_counters().upload(bytes.len() as u64);
        // SAFETY: the mapping covers `size` bytes and the bounds were checked above; the caller
        // ensures the GPU is not reading this range (frame slots).
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                mapped.as_ptr().add(offset as usize),
                bytes.len(),
            )
        };
    }

    /// Copies from the mapped buffer at `offset` into `out`.
    ///
    /// # Panics
    /// If the buffer is not mapped or the read does not fit.
    pub fn read<T: Pod>(&self, offset: u64, out: &mut [T]) {
        let bytes: &mut [u8] = bytemuck::cast_slice_mut(out);
        let mapped = self.mapped.expect("buffer is not host-visible");
        assert!(
            offset + bytes.len() as u64 <= self.size,
            "read exceeds buffer size"
        );
        self.device.memory_counters().read_back(bytes.len() as u64);
        // SAFETY: as in `write`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                mapped.as_ptr().add(offset as usize),
                bytes.as_mut_ptr(),
                bytes.len(),
            )
        };
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            self.device
                .memory_counters()
                .free(self.category, allocation.size());
            let _ = self.device.with_allocator(|a| a.free(allocation));
        }
        // SAFETY: the memory was released above and the owner guarantees the GPU is done.
        unsafe { self.device.raw().destroy_buffer(self.raw, None) };
    }
}

/// Description of a 2-D image to create.
#[derive(Clone, Copy, Debug)]
pub struct ImageDesc<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Format.
    pub format: vk::Format,
    /// Usage flags.
    pub usage: vk::ImageUsageFlags,
    /// Aspect for the default view.
    pub aspect: vk::ImageAspectFlags,
    /// Mip levels (1 = none). Each level also gets its own single-level view.
    pub mip_levels: u32,
    /// Debug name.
    pub name: &'a str,
}

/// A 2-D image with a full view and one view per mip level. Freed on drop (same rule as
/// [`Buffer`]).
pub struct Image {
    device: Arc<Device>,
    raw: vk::Image,
    view: vk::ImageView,
    mip_views: Vec<vk::ImageView>,
    /// Own memory, or `None` when the image is placed in a [`TransientHeap`].
    allocation: Option<Allocation>,
    /// The heap a placed image lives in (kept alive by the image).
    heap: Option<Arc<TransientHeap>>,
    category: MemoryCategory,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    extent: vk::Extent2D,
}

/// One block of device memory that several images share by living at different offsets
/// (the render graph's transient images, whose lifetimes never overlap when they alias).
/// The heap outlives every image placed in it: images hold an `Arc` to it.
pub struct TransientHeap {
    device: Arc<Device>,
    allocation: Option<Allocation>,
    size: u64,
    memory_type_bits: u32,
}

impl TransientHeap {
    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The memory types the heap is compatible with (a mask of type indices).
    pub fn memory_type_bits(&self) -> u32 {
        self.memory_type_bits
    }
}

impl Drop for TransientHeap {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            self.device
                .memory_counters()
                .free(MemoryCategory::Transient, allocation.size());
            let _ = self.device.with_allocator(|a| a.free(allocation));
        }
    }
}

impl Image {
    /// The Vulkan image.
    pub fn raw(&self) -> vk::Image {
        self.raw
    }
    /// The view over every mip level.
    pub fn view(&self) -> vk::ImageView {
        self.view
    }
    /// A view of one mip level.
    pub fn mip_view(&self, level: u32) -> vk::ImageView {
        self.mip_views[level as usize]
    }
    /// Number of mip levels.
    pub fn mip_levels(&self) -> u32 {
        self.mip_views.len() as u32
    }
    /// Format.
    pub fn format(&self) -> vk::Format {
        self.format
    }
    /// Usage flags it was created with.
    pub fn usage(&self) -> vk::ImageUsageFlags {
        self.usage
    }
    /// Size of level 0.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }
    /// The transient heap a placed image lives in (`None` for an image with its own memory).
    pub fn heap(&self) -> Option<&Arc<TransientHeap>> {
        self.heap.as_ref()
    }
    /// Size of mip `level`.
    pub fn mip_extent(&self, level: u32) -> vk::Extent2D {
        vk::Extent2D {
            width: (self.extent.width >> level).max(1),
            height: (self.extent.height >> level).max(1),
        }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        // SAFETY: the owner guarantees the GPU is done; memory is released after the views and image.
        unsafe {
            for view in self.mip_views.drain(..) {
                self.device.raw().destroy_image_view(view, None);
            }
            self.device.raw().destroy_image_view(self.view, None);
            self.device.raw().destroy_image(self.raw, None);
        }
        if let Some(allocation) = self.allocation.take() {
            self.device
                .memory_counters()
                .free(self.category, allocation.size());
            let _ = self.device.with_allocator(|a| a.free(allocation));
        }
    }
}

impl Device {
    /// Creates a buffer. Host-visible locations are persistently mapped.
    pub fn create_buffer(self: &Arc<Self>, desc: BufferDesc<'_>) -> Result<Buffer> {
        let size = desc.size.max(4);
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(desc.usage | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
        // Every buffer may be read on any queue (issue #77): concurrent costs nothing for
        // buffers.
        let families = self.queue_families();
        let info = if families.len() > 1 {
            info.sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(families)
        } else {
            info.sharing_mode(vk::SharingMode::EXCLUSIVE)
        };
        // SAFETY: valid create info on a live device.
        let raw = unsafe { self.raw().create_buffer(&info, None)? };
        // SAFETY: `raw` is a live buffer.
        let requirements = unsafe { self.raw().get_buffer_memory_requirements(raw) };
        let allocation = match self.with_allocator(|a| {
            a.allocate(&AllocationCreateDesc {
                name: desc.name,
                requirements,
                location: desc.location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
        }) {
            Ok(allocation) => allocation,
            Err(e) => {
                // SAFETY: the buffer has no memory bound and nothing references it.
                unsafe { self.raw().destroy_buffer(raw, None) };
                return Err(e.into());
            }
        };
        // SAFETY: the allocation satisfies the buffer's requirements.
        unsafe {
            self.raw()
                .bind_buffer_memory(raw, allocation.memory(), allocation.offset())?
        };
        let address_info = vk::BufferDeviceAddressInfo::default().buffer(raw);
        // SAFETY: the buffer was created with `SHADER_DEVICE_ADDRESS` usage.
        let address = unsafe { self.raw().get_buffer_device_address(&address_info) };
        let mapped = allocation.mapped_ptr().map(|p| p.cast::<u8>());
        self.set_name(raw, desc.name);
        self.memory_counters()
            .allocate(desc.category, allocation.size());
        Ok(Buffer {
            device: Arc::clone(self),
            raw,
            allocation: Some(allocation),
            category: desc.category,
            size,
            address,
            mapped,
        })
    }

    /// Records `record` (dispatches, without barriers of their own) into a one-shot command
    /// buffer, followed by a barrier that makes its shader writes visible to everything
    /// submitted later, submits it and waits: a compute pass that prepares data before the
    /// first frame (GPU placement). Initialisation only.
    pub fn execute_compute_once(&self, record: impl FnOnce(&crate::Commands<'_>)) -> Result<()> {
        use vk::PipelineStageFlags2 as S;
        self.execute_transient(|_, cb| {
            let commands = crate::Commands::new(self, cb);
            record(&commands);
            commands.memory_barrier(
                S::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_WRITE,
                S::ALL_COMMANDS,
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
            );
        })
    }

    /// Copies `size` bytes of `src` from `offset` into host memory: everything submitted
    /// before is waited for and made visible to the copy, the copy to the host. `src` needs
    /// `TRANSFER_SRC` usage. Initialisation and tests only (it waits).
    pub fn read_back(self: &Arc<Self>, src: &Buffer, offset: u64, size: u64) -> Result<Vec<u8>> {
        use vk::PipelineStageFlags2 as S;
        let readback = self.create_buffer(BufferDesc {
            size: size.max(4),
            usage: vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "read back",
        })?;
        self.execute_transient(|device, cb| {
            let commands = crate::Commands::new(self, cb);
            commands.memory_barrier(
                S::ALL_COMMANDS,
                vk::AccessFlags2::MEMORY_WRITE,
                S::COPY,
                vk::AccessFlags2::TRANSFER_READ,
            );
            let region = vk::BufferCopy::default().src_offset(offset).size(size);
            // SAFETY: both buffers are live and the copy is within bounds (the caller's
            // responsibility for `src`).
            unsafe { device.cmd_copy_buffer(cb, src.raw(), readback.raw(), &[region]) };
            commands.memory_barrier(
                S::COPY,
                vk::AccessFlags2::TRANSFER_WRITE,
                S::HOST,
                vk::AccessFlags2::HOST_READ,
            );
        })?;
        let mut bytes = vec![0_u8; size as usize];
        readback.read(0, &mut bytes);
        Ok(bytes)
    }

    /// Creates a device-local buffer initialised with `data` through a staging copy.
    pub fn create_buffer_with_data<T: Pod>(
        self: &Arc<Self>,
        data: &[T],
        usage: vk::BufferUsageFlags,
        category: MemoryCategory,
        name: &str,
    ) -> Result<Buffer> {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let size = bytes.len().max(4) as u64;
        let staging = self.create_buffer(BufferDesc {
            size,
            usage: vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Transfer,
            name: "staging",
        })?;
        staging.write(0, bytes);
        let buffer = self.create_buffer(BufferDesc {
            size,
            usage: usage | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category,
            name,
        })?;
        self.execute_transient(|device, cb| {
            let region = vk::BufferCopy::default().size(size);
            // SAFETY: both buffers are live and the copy is within bounds.
            unsafe { device.cmd_copy_buffer(cb, staging.raw(), buffer.raw(), &[region]) };
        })?;
        Ok(buffer)
    }

    /// Copies `data` into `dst` from byte `offset` through a staging copy and waits for it
    /// (`dst` needs `TRANSFER_DST` usage and must not be in use).
    pub fn write_buffer_staged(
        self: &Arc<Self>,
        dst: &Buffer,
        offset: u64,
        data: &[u8],
    ) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let staging = self.create_buffer(BufferDesc {
            size: data.len() as u64,
            usage: vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Transfer,
            name: "staging",
        })?;
        staging.write(0, data);
        self.execute_transient(|device, cb| {
            let region = vk::BufferCopy::default()
                .dst_offset(offset)
                .size(data.len() as u64);
            // SAFETY: both buffers are live, the copy is within bounds (the caller keeps
            // `offset + data.len()` inside `dst`) and `dst` is not in use.
            unsafe { device.cmd_copy_buffer(cb, staging.raw(), dst.raw(), &[region]) };
        })?;
        Ok(())
    }

    /// Creates a single-level colour image filled with `data` (tightly packed rows of the
    /// image's format) and leaves it in `SHADER_READ_ONLY_OPTIMAL`. `TRANSFER_DST` is added
    /// to the usage.
    pub fn create_image_with_data(
        self: &Arc<Self>,
        desc: ImageDesc<'_>,
        data: &[u8],
    ) -> Result<Image> {
        self.create_image_with_mips(
            ImageDesc {
                mip_levels: 1,
                ..desc
            },
            &[data],
        )
    }

    /// Creates a colour image with one mip level per entry of `levels` (each tightly packed
    /// rows of the image's format, level 0 first) and leaves it in
    /// `SHADER_READ_ONLY_OPTIMAL`. `TRANSFER_DST` is added to the usage.
    pub fn create_image_with_mips(
        self: &Arc<Self>,
        desc: ImageDesc<'_>,
        levels: &[&[u8]],
    ) -> Result<Image> {
        let desc = ImageDesc {
            usage: desc.usage | vk::ImageUsageFlags::TRANSFER_DST,
            mip_levels: levels.len().max(1) as u32,
            ..desc
        };
        let image = self.allocate_image(&desc, MemoryCategory::Textures)?;
        let total: usize = levels.iter().map(|l| l.len()).sum();
        let staging = self.create_buffer(BufferDesc {
            size: total.max(4) as u64,
            usage: vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Transfer,
            name: "image upload staging",
        })?;
        let mut regions = Vec::with_capacity(levels.len());
        let mut offset = 0_u64;
        for (level, data) in levels.iter().enumerate() {
            staging.write(offset, data);
            let extent = image.mip_extent(level as u32);
            regions.push(
                vk::BufferImageCopy::default()
                    .buffer_offset(offset)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: level as u32,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    }),
            );
            offset += data.len() as u64;
        }
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: desc.mip_levels,
            base_array_layer: 0,
            layer_count: 1,
        };
        let barrier = |src_stage, src_access, dst_stage, dst_access, old, new| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(src_stage)
                .src_access_mask(src_access)
                .dst_stage_mask(dst_stage)
                .dst_access_mask(dst_access)
                .old_layout(old)
                .new_layout(new)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image.raw())
                .subresource_range(range)
        };
        let to_transfer = [barrier(
            vk::PipelineStageFlags2::NONE,
            vk::AccessFlags2::NONE,
            vk::PipelineStageFlags2::TRANSFER,
            vk::AccessFlags2::TRANSFER_WRITE,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        )];
        let to_sampled = [barrier(
            vk::PipelineStageFlags2::TRANSFER,
            vk::AccessFlags2::TRANSFER_WRITE,
            vk::PipelineStageFlags2::ALL_COMMANDS,
            vk::AccessFlags2::SHADER_SAMPLED_READ,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        )];
        self.execute_transient(|raw, cb| {
            // SAFETY: recorded into the transient command buffer; the staging buffer outlives
            // the call (it is dropped after the fence wait inside `execute_transient` returns).
            unsafe {
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_transfer),
                );
                raw.cmd_copy_buffer_to_image(
                    cb,
                    staging.raw(),
                    image.raw(),
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &regions,
                );
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_sampled),
                );
            }
        })?;
        drop(staging);
        Ok(image)
    }

    /// Moves every level of `image` from `UNDEFINED` to `layout` in a one-shot submission.
    /// Initialisation only (images that live in one layout, such as storage pyramids).
    pub fn initialize_image_layout(
        &self,
        image: &Image,
        aspect: vk::ImageAspectFlags,
        layout: vk::ImageLayout,
    ) -> Result<()> {
        let barrier = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image.raw())
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: aspect,
                base_mip_level: 0,
                level_count: image.mip_levels(),
                base_array_layer: 0,
                layer_count: 1,
            })];
        let info = vk::DependencyInfo::default().image_memory_barriers(&barrier);
        self.execute_transient(|raw, cb| {
            // SAFETY: recorded into the transient command buffer owned by `execute_transient`.
            unsafe { raw.cmd_pipeline_barrier2(cb, &info) };
        })
    }

    /// Creates a 2-D image in device memory with a full view, counted as a render target
    /// (an image the GPU writes; uploaded images are [`Device::create_image_with_data`]'s).
    pub fn create_image(self: &Arc<Self>, desc: ImageDesc<'_>) -> Result<Image> {
        self.allocate_image(&desc, MemoryCategory::Targets)
    }

    /// Creates an image with its own memory, counted under `category`.
    pub(crate) fn allocate_image(
        self: &Arc<Self>,
        desc: &ImageDesc<'_>,
        category: MemoryCategory,
    ) -> Result<Image> {
        let (raw, extent, mip_levels) = self.create_unbound_image(desc)?;
        // SAFETY: `raw` is live.
        let requirements = unsafe { self.raw().get_image_memory_requirements(raw) };
        let allocation = match self.with_allocator(|a| {
            a.allocate(&AllocationCreateDesc {
                name: desc.name,
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
        }) {
            Ok(allocation) => allocation,
            Err(e) => {
                // SAFETY: the image has no memory bound and nothing references it.
                unsafe { self.raw().destroy_image(raw, None) };
                return Err(e.into());
            }
        };
        // SAFETY: the allocation satisfies the image's requirements.
        if let Err(e) = unsafe {
            self.raw()
                .bind_image_memory(raw, allocation.memory(), allocation.offset())
        } {
            // SAFETY: binding failed, so nothing references the image or the allocation.
            unsafe { self.raw().destroy_image(raw, None) };
            let _ = self.with_allocator(|a| a.free(allocation));
            return Err(e.into());
        }
        self.finish_image(
            raw,
            desc,
            extent,
            mip_levels,
            (Some(allocation), category),
            None,
        )
    }

    /// The memory an image of `desc` needs (size, alignment, compatible memory types),
    /// without creating it (`vkGetDeviceImageMemoryRequirements`, Vulkan 1.3).
    pub fn image_memory_requirements(&self, desc: &ImageDesc<'_>) -> vk::MemoryRequirements {
        let (info, _, _) = image_create_info(desc);
        let info = self.with_image_sharing(info, desc.usage);
        let query = vk::DeviceImageMemoryRequirements::default().create_info(&info);
        let mut requirements = vk::MemoryRequirements2::default();
        // SAFETY: valid create info; the out structure is a plain default.
        unsafe {
            self.raw()
                .get_device_image_memory_requirements(&query, &mut requirements)
        };
        requirements.memory_requirements
    }

    /// Allocates a block of device-local memory for placed images: `size` bytes aligned to
    /// `alignment`, in a memory type of `memory_type_bits` (the AND of the requirements of
    /// every image that will live in it).
    pub fn create_transient_heap(
        self: &Arc<Self>,
        size: u64,
        alignment: u64,
        memory_type_bits: u32,
        name: &str,
    ) -> Result<Arc<TransientHeap>> {
        let allocation = self.with_allocator(|a| {
            a.allocate(&AllocationCreateDesc {
                name,
                requirements: vk::MemoryRequirements {
                    size: size.max(1),
                    alignment: alignment.max(1),
                    memory_type_bits,
                },
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
        })?;
        self.memory_counters()
            .allocate(MemoryCategory::Transient, allocation.size());
        Ok(Arc::new(TransientHeap {
            device: Arc::clone(self),
            allocation: Some(allocation),
            size,
            memory_type_bits,
        }))
    }

    /// Creates an image whose memory is `heap` at `offset`. The caller laid the heap out from
    /// [`Device::image_memory_requirements`]: the range must fit, `offset` must honour the
    /// image's alignment and the heap's memory type must be one the image accepts.
    pub fn create_image_in(
        self: &Arc<Self>,
        desc: ImageDesc<'_>,
        heap: &Arc<TransientHeap>,
        offset: u64,
    ) -> Result<Image> {
        let (raw, extent, mip_levels) = self.create_unbound_image(&desc)?;
        // SAFETY: `raw` is live.
        let requirements = unsafe { self.raw().get_image_memory_requirements(raw) };
        let fits = offset.is_multiple_of(requirements.alignment.max(1))
            && offset + requirements.size <= heap.size
            && requirements.memory_type_bits & heap.memory_type_bits != 0;
        let allocation = heap.allocation.as_ref().filter(|_| fits);
        let Some(allocation) = allocation else {
            // SAFETY: nothing references the image.
            unsafe { self.raw().destroy_image(raw, None) };
            return Err(crate::error::GpuError::Unsupported(format!(
                "image '{}' does not fit its transient heap (offset {offset}, size {}, alignment {}, heap {} bytes, types {:#x} vs {:#x})",
                desc.name,
                requirements.size,
                requirements.alignment,
                heap.size,
                requirements.memory_type_bits,
                heap.memory_type_bits
            )));
        };
        // SAFETY: the range was checked against the requirements above; the heap's memory is
        // one the image accepts and outlives the image (it holds an `Arc` to the heap).
        if let Err(e) = unsafe {
            self.raw()
                .bind_image_memory(raw, allocation.memory(), allocation.offset() + offset)
        } {
            // SAFETY: binding failed, so nothing references the image.
            unsafe { self.raw().destroy_image(raw, None) };
            return Err(e.into());
        }
        self.finish_image(
            raw,
            &desc,
            extent,
            mip_levels,
            (None, MemoryCategory::Transient),
            Some(Arc::clone(heap)),
        )
    }

    /// `info` made `CONCURRENT` over the queue families when [`Device::image_concurrent`]
    /// says so for `usage`.
    fn with_image_sharing<'a>(
        &'a self,
        info: vk::ImageCreateInfo<'a>,
        usage: vk::ImageUsageFlags,
    ) -> vk::ImageCreateInfo<'a> {
        if self.image_concurrent(usage) {
            info.sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(self.queue_families())
        } else {
            info
        }
    }

    /// Creates the Vulkan image of `desc` without memory.
    fn create_unbound_image(&self, desc: &ImageDesc<'_>) -> Result<(vk::Image, vk::Extent2D, u32)> {
        let (info, extent, mip_levels) = image_create_info(desc);
        let info = self.with_image_sharing(info, desc.usage);
        // SAFETY: valid create info.
        let raw = unsafe { self.raw().create_image(&info, None)? };
        Ok((raw, extent, mip_levels))
    }

    /// Creates the views and names a bound image; counts its own memory under the category.
    fn finish_image(
        self: &Arc<Self>,
        raw: vk::Image,
        desc: &ImageDesc<'_>,
        extent: vk::Extent2D,
        mip_levels: u32,
        (allocation, category): (Option<Allocation>, MemoryCategory),
        heap: Option<Arc<TransientHeap>>,
    ) -> Result<Image> {
        let make_view = |base_mip_level: u32, level_count: u32| {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(raw)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(desc.format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: desc.aspect,
                    base_mip_level,
                    level_count,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            // SAFETY: valid view info on a bound image.
            unsafe { self.raw().create_image_view(&view_info, None) }
        };
        let view = make_view(0, mip_levels)?;
        let mip_views = (0..mip_levels)
            .map(|level| make_view(level, 1))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.set_name(raw, desc.name);
        if let Some(allocation) = &allocation {
            self.memory_counters().allocate(category, allocation.size());
        }
        Ok(Image {
            device: Arc::clone(self),
            raw,
            view,
            mip_views,
            allocation,
            heap,
            category,
            format: desc.format,
            usage: desc.usage,
            extent,
        })
    }
}

/// The create info of a 2-D optimal-tiling image, with the clamped extent and mip count.
fn image_create_info(desc: &ImageDesc<'_>) -> (vk::ImageCreateInfo<'static>, vk::Extent2D, u32) {
    let extent = vk::Extent2D {
        width: desc.width.max(1),
        height: desc.height.max(1),
    };
    let mip_levels = desc.mip_levels.max(1);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(desc.format)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(mip_levels)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(desc.usage)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    (info, extent, mip_levels)
}
