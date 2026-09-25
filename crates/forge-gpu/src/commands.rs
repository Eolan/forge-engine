//! Safe command recording. Each method wraps one Vulkan command whose preconditions are
//! carried by the types (`Pipeline`, `Image`) or stated in the method's documentation.

use ash::vk;
use bytemuck::Pod;

use crate::device::Device;
use crate::error::{GpuError, Result};
use crate::pipeline::Pipeline;
use crate::timers::GpuTimerSlot;

/// A command buffer in the recording state, tied to the device that owns it.
pub struct Commands<'a> {
    device: &'a Device,
    cb: vk::CommandBuffer,
    timers: Option<&'a GpuTimerSlot>,
    /// The label of the render-graph pass being recorded (the debugging barrier's filter).
    pass: std::cell::Cell<&'static str>,
}

impl<'a> Commands<'a> {
    /// Attaches the frame's GPU timer so [`Commands::mark`] records zones.
    pub fn with_timers(mut self, timers: &'a GpuTimerSlot) -> Self {
        self.timers = Some(timers);
        self
    }

    /// Closes a GPU timing zone: the work recorded since the previous mark is called `label`
    /// (`group/name`). A no-op without timers.
    pub fn mark(&self, label: &'static str) {
        if let Some(timers) = self.timers {
            timers.mark(self.cb, label);
        }
    }

    /// Records which render-graph pass the next commands belong to.
    pub(crate) fn set_pass(&self, label: &'static str) {
        self.pass.set(label);
    }

    /// `FORGE_PARANOID_BARRIERS=1`: a full memory barrier before every dispatch, draw and blit
    /// (debugging aid to tell an intra-frame ordering bug from anything else). A comma-separated
    /// list of pass-label prefixes (`ao/,dust/`) limits it to those passes.
    fn paranoid_barrier(&self) {
        static PARANOID: std::sync::OnceLock<Option<Vec<String>>> = std::sync::OnceLock::new();
        let filter = PARANOID.get_or_init(|| {
            let value = std::env::var("FORGE_PARANOID_BARRIERS").ok()?;
            match value.as_str() {
                "" | "0" => None,
                "1" => Some(Vec::new()),
                list => Some(list.split(',').map(str::to_owned).collect()),
            }
        });
        let pass = self.pass.get();
        if let Some(filter) = filter
            && (filter.is_empty() || filter.iter().any(|p| pass.starts_with(p.as_str())))
        {
            self.memory_barrier(
                vk::PipelineStageFlags2::ALL_COMMANDS,
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
                vk::PipelineStageFlags2::ALL_COMMANDS,
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
            );
        }
    }

    /// Wraps a command buffer that [`crate::Frames::begin`] put in the recording state.
    pub fn new(device: &'a Device, cb: vk::CommandBuffer) -> Self {
        Self {
            device,
            cb,
            timers: None,
            pass: std::cell::Cell::new(""),
        }
    }

    /// The raw command buffer.
    pub fn raw(&self) -> vk::CommandBuffer {
        self.cb
    }

    /// Records image layout transitions / memory barriers (synchronization2).
    pub fn image_barriers(&self, barriers: &[vk::ImageMemoryBarrier2<'_>]) {
        self.barriers(&[], barriers);
    }

    /// Records global memory barriers and image barriers in one `vkCmdPipelineBarrier2`
    /// (what the render graph emits before a pass). A no-op when both are empty.
    pub fn barriers(
        &self,
        memory: &[vk::MemoryBarrier2<'_>],
        images: &[vk::ImageMemoryBarrier2<'_>],
    ) {
        if memory.is_empty() && images.is_empty() {
            return;
        }
        let info = vk::DependencyInfo::default()
            .memory_barriers(memory)
            .image_memory_barriers(images);
        // SAFETY: recording state; the barriers reference live images owned by the caller.
        unsafe { self.device.raw().cmd_pipeline_barrier2(self.cb, &info) };
    }

    /// Begins dynamic rendering.
    pub fn begin_rendering(&self, info: &vk::RenderingInfo<'_>) {
        self.paranoid_barrier();
        // SAFETY: recording state, outside a render pass instance.
        unsafe { self.device.raw().cmd_begin_rendering(self.cb, info) };
    }

    /// Ends dynamic rendering.
    pub fn end_rendering(&self) {
        // SAFETY: inside a rendering instance begun by `begin_rendering`.
        unsafe { self.device.raw().cmd_end_rendering(self.cb) };
    }

    /// Binds a pipeline and the global bindless set (set 0) for its bind point.
    pub fn bind_pipeline(&self, pipeline: &Pipeline) {
        // SAFETY: the pipeline is alive for as long as the caller holds it; the bindless set
        // lives as long as the device.
        unsafe {
            self.device
                .raw()
                .cmd_bind_pipeline(self.cb, pipeline.bind_point(), pipeline.raw());
            self.device.raw().cmd_bind_descriptor_sets(
                self.cb,
                pipeline.bind_point(),
                pipeline.layout(),
                0,
                &[self.device.bindless_set()],
                &[],
            );
        }
    }

    /// Draws `vertex_count` vertices without vertex buffers (full-screen triangles).
    pub fn draw(&self, vertex_count: u32, instance_count: u32) {
        // SAFETY: a graphics pipeline without vertex input is bound inside a rendering instance.
        unsafe {
            self.device
                .raw()
                .cmd_draw(self.cb, vertex_count, instance_count, 0, 0)
        };
    }

    /// Dispatches compute workgroups.
    pub fn dispatch(&self, x: u32, y: u32, z: u32) {
        self.paranoid_barrier();
        // SAFETY: a compute pipeline is bound (the caller's responsibility).
        unsafe { self.device.raw().cmd_dispatch(self.cb, x, y, z) };
    }

    /// Dispatches the workgroup counts of the `VkDispatchIndirectCommand` (three `u32`) at
    /// `offset` in `buffer`, which must have `INDIRECT_BUFFER` usage.
    pub fn dispatch_indirect(&self, buffer: &crate::Buffer, offset: u64) {
        self.paranoid_barrier();
        // SAFETY: a compute pipeline is bound and the buffer holds a complete command at
        // `offset` (the caller's responsibility).
        unsafe {
            self.device
                .raw()
                .cmd_dispatch_indirect(self.cb, buffer.raw(), offset)
        };
    }

    /// Binds `buffer` (with `INDEX_BUFFER` usage) as the index buffer.
    pub fn bind_index_buffer(
        &self,
        buffer: &crate::Buffer,
        offset: u64,
        index_type: vk::IndexType,
    ) {
        // SAFETY: the buffer is alive while the command buffer is recorded and executed (the
        // caller holds it); the index type's feature is the caller's responsibility.
        unsafe {
            self.device
                .raw()
                .cmd_bind_index_buffer(self.cb, buffer.raw(), offset, index_type)
        };
    }

    /// `vkCmdDrawIndexedIndirectCount`: up to `max_draws` `VkDrawIndexedIndirectCommand`s,
    /// `stride` bytes apart from `offset` in `buffer`, as many as the `u32` at `count_offset`
    /// in `count_buffer` says. Both buffers need `INDIRECT_BUFFER` usage.
    pub fn draw_indexed_indirect_count(
        &self,
        buffer: &crate::Buffer,
        offset: u64,
        count_buffer: &crate::Buffer,
        count_offset: u64,
        max_draws: u32,
        stride: u32,
    ) {
        // SAFETY: a graphics pipeline and an index buffer are bound inside a rendering
        // instance, and the buffers hold the commands and the count (the caller's
        // responsibility).
        unsafe {
            self.device.raw().cmd_draw_indexed_indirect_count(
                self.cb,
                buffer.raw(),
                offset,
                count_buffer.raw(),
                count_offset,
                max_draws,
                stride,
            )
        };
    }

    /// A global memory barrier between two stage/access sets (buffers and images alike).
    pub fn memory_barrier(
        &self,
        src_stage: vk::PipelineStageFlags2,
        src_access: vk::AccessFlags2,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) {
        let barrier = [vk::MemoryBarrier2::default()
            .src_stage_mask(src_stage)
            .src_access_mask(src_access)
            .dst_stage_mask(dst_stage)
            .dst_access_mask(dst_access)];
        let info = vk::DependencyInfo::default().memory_barriers(&barrier);
        // SAFETY: recording state.
        unsafe { self.device.raw().cmd_pipeline_barrier2(self.cb, &info) };
    }

    /// Sets a full-size viewport with the y axis flipped so NDC +y is up (Y-up worlds render
    /// with counter-clockwise front faces), plus a matching scissor.
    pub fn set_viewport_full(&self, extent: vk::Extent2D) {
        let viewport = vk::Viewport {
            x: 0.0,
            y: extent.height as f32,
            width: extent.width as f32,
            height: -(extent.height as f32),
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        // SAFETY: recording state; negative viewport height is core since Vulkan 1.1.
        unsafe {
            self.device.raw().cmd_set_viewport(self.cb, 0, &[viewport]);
            self.device.raw().cmd_set_scissor(self.cb, 0, &[scissor]);
        }
    }

    /// Writes push constants for `pipeline`.
    pub fn push_constants<T: Pod>(&self, pipeline: &Pipeline, data: &T) {
        // SAFETY: `data` is plain bytes and fits the range declared at pipeline creation.
        unsafe {
            self.device.raw().cmd_push_constants(
                self.cb,
                pipeline.layout(),
                pipeline.push_constant_stages(),
                0,
                bytemuck::bytes_of(data),
            );
        }
    }

    /// Blits a whole 2-D colour image (`TRANSFER_SRC_OPTIMAL`) onto another of the same size
    /// (`TRANSFER_DST_OPTIMAL`), converting formats.
    pub fn blit_image(
        &self,
        src: vk::Image,
        dst: vk::Image,
        extent: vk::Extent2D,
        filter: vk::Filter,
    ) {
        self.paranoid_barrier();
        let layers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };
        let corners = [
            vk::Offset3D { x: 0, y: 0, z: 0 },
            vk::Offset3D {
                x: extent.width as i32,
                y: extent.height as i32,
                z: 1,
            },
        ];
        let region = vk::ImageBlit::default()
            .src_subresource(layers)
            .src_offsets(corners)
            .dst_subresource(layers)
            .dst_offsets(corners);
        // SAFETY: recording state; both images are in the transfer layouts documented above.
        unsafe {
            self.device.raw().cmd_blit_image(
                self.cb,
                src,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                dst,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
                filter,
            );
        }
    }

    /// Fills `size` bytes of `buffer` from `offset` with the 32-bit `value`
    /// (`vkCmdFillBuffer`; the buffer needs `TRANSFER_DST` usage, offset and size are
    /// multiples of 4).
    pub fn fill_buffer(&self, buffer: &crate::Buffer, offset: u64, size: u64, value: u32) {
        // SAFETY: recording state; the caller keeps the range inside the buffer and the
        // buffer alive until the frame completes.
        unsafe {
            self.device
                .raw()
                .cmd_fill_buffer(self.cb, buffer.raw(), offset, size, value);
        }
    }

    /// Writes `data` into `buffer` at `offset` from the command buffer (`vkCmdUpdateBuffer`:
    /// at most 65 536 bytes, `TRANSFER_DST` usage): small per-frame resets of GPU-written
    /// buffers, ordered by the render graph like any transfer.
    pub fn update_buffer(&self, buffer: &crate::Buffer, offset: u64, data: &[u32]) {
        debug_assert!(std::mem::size_of_val(data) <= 65_536);
        // SAFETY: recording state; the caller keeps the range inside the buffer and the
        // buffer alive until the frame completes. The data is copied into the command buffer.
        unsafe {
            self.device.raw().cmd_update_buffer(
                self.cb,
                buffer.raw(),
                offset,
                bytemuck::cast_slice(data),
            );
        }
    }

    /// Copies `size` bytes from the start of `src` to the start of `dst`
    /// (`TRANSFER_SRC` / `TRANSFER_DST` usage).
    pub fn copy_buffer(&self, src: &crate::Buffer, dst: &crate::Buffer, size: u64) {
        let region = vk::BufferCopy::default().size(size);
        // SAFETY: recording state; both buffers hold at least `size` bytes and outlive the
        // frame (the caller's responsibility).
        unsafe {
            self.device
                .raw()
                .cmd_copy_buffer(self.cb, src.raw(), dst.raw(), &[region]);
        }
    }

    /// Copies `regions` (source offset, destination offset, size) from `src` to `dst` in one
    /// command (`TRANSFER_SRC` / `TRANSFER_DST` usage). Nothing is recorded for no region.
    pub fn copy_buffer_regions(
        &self,
        src: &crate::Buffer,
        dst: &crate::Buffer,
        regions: &[(u64, u64, u64)],
    ) {
        if regions.is_empty() {
            return;
        }
        let regions: Vec<vk::BufferCopy> = regions
            .iter()
            .map(|&(src_offset, dst_offset, size)| {
                vk::BufferCopy::default()
                    .src_offset(src_offset)
                    .dst_offset(dst_offset)
                    .size(size)
            })
            .collect();
        // SAFETY: recording state; every region lies inside both buffers, the destination
        // regions do not overlap, and both buffers outlive the frame (the caller's
        // responsibility).
        unsafe {
            self.device
                .raw()
                .cmd_copy_buffer(self.cb, src.raw(), dst.raw(), &regions);
        }
    }

    /// Copies a whole 2-D colour image (in `TRANSFER_SRC_OPTIMAL` layout) into `buffer`,
    /// tightly packed, for readback. The buffer must hold `width × height × 4` bytes.
    pub fn copy_image_to_buffer(
        &self,
        image: vk::Image,
        extent: vk::Extent2D,
        buffer: &crate::Buffer,
    ) {
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            });
        // SAFETY: recording state; the image is in the transfer-source layout and the buffer
        // is large enough (documented precondition).
        unsafe {
            self.device.raw().cmd_copy_image_to_buffer(
                self.cb,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.raw(),
                &[region],
            );
        }
    }

    /// Launches task workgroups (`vkCmdDrawMeshTasksEXT`).
    pub fn draw_mesh_tasks(&self, x: u32, y: u32, z: u32) -> Result<()> {
        let loader = self
            .device
            .mesh_loader()
            .ok_or_else(|| GpuError::Unsupported("mesh shaders".into()))?;
        // SAFETY: a mesh pipeline is bound (the caller's responsibility) inside a rendering instance.
        unsafe { loader.cmd_draw_mesh_tasks(self.cb, x, y, z) };
        Ok(())
    }

    /// `vkCmdDrawIndirect` with one `VkDrawIndirectCommand` (four `u32`: vertex count,
    /// instance count, first vertex, first instance) at `offset` in `buffer`, which must have
    /// `INDIRECT_BUFFER` usage: a draw whose size the GPU decides, down to nothing.
    pub fn draw_indirect(&self, buffer: &crate::Buffer, offset: u64) {
        // SAFETY: a graphics pipeline without vertex input is bound inside a rendering
        // instance and the buffer holds a complete indirect command at `offset` (the caller's
        // responsibility).
        unsafe {
            self.device
                .raw()
                .cmd_draw_indirect(self.cb, buffer.raw(), offset, 1, 16)
        };
    }

    /// `vkCmdDrawMeshTasksIndirectEXT` with one `VkDrawMeshTasksIndirectCommandEXT` (three
    /// `u32`: x, y, z) at `offset` in `buffer`, which must have `INDIRECT_BUFFER` usage.
    pub fn draw_mesh_tasks_indirect(&self, buffer: &crate::Buffer, offset: u64) -> Result<()> {
        let loader = self
            .device
            .mesh_loader()
            .ok_or_else(|| GpuError::Unsupported("mesh shaders".into()))?;
        // SAFETY: a mesh pipeline is bound inside a rendering instance and the buffer holds a
        // complete indirect command at `offset` (the caller's responsibility).
        unsafe { loader.cmd_draw_mesh_tasks_indirect(self.cb, buffer.raw(), offset, 1, 12) };
        Ok(())
    }
}
