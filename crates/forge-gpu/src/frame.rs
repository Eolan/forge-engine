use std::sync::Arc;

use ash::vk;

use crate::device::Device;
use crate::error::Result;
use crate::timers::{GpuTimerSlot, GpuTimers, GpuZone};

/// CPU frames that may be in flight ahead of the GPU.
pub const FRAMES_IN_FLIGHT: usize = 2;

struct PerSlot {
    pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    image_available: vk::Semaphore,
}

/// A frame slot handed out by [`Frames::wait_for_slot`].
#[derive(Clone, Copy, Debug)]
pub struct FrameSlot {
    /// Slot index in `0..FRAMES_IN_FLIGHT`.
    pub index: usize,
    /// Monotonic frame number.
    pub frame_number: u64,
    /// GPU time of the previous frame that used this slot, in milliseconds.
    pub previous_gpu_ms: Option<f64>,
}

/// Frames-in-flight bookkeeping on a timeline semaphore, plus GPU timestamps.
///
/// Per frame: `wait_for_slot` → acquire the swapchain image with
/// [`Frames::image_available`] → `begin` → record → `submit` → present with
/// [`Frames::render_finished`]. The timeline value signalled by frame `n` is `n + 1`; waiting
/// for `n + 1 - FRAMES_IN_FLIGHT` before reusing a slot keeps exactly `FRAMES_IN_FLIGHT`
/// frames queued and never blocks on the whole queue (the previous engine's lesson).
pub struct Frames {
    device: Arc<Device>,
    timeline: vk::Semaphore,
    slots: Vec<PerSlot>,
    render_finished: Vec<vk::Semaphore>,
    timers: GpuTimers,
    last_zones: Vec<GpuZone>,
    frame_number: u64,
    /// Resources retired by [`Frames::destroy_later`], tagged with the frame that may still
    /// use them; dropped once that frame has completed on the GPU.
    garbage: Vec<(u64, Box<dyn std::any::Any>)>,
}

impl Frames {
    /// Creates the per-slot command pools, semaphores and the timestamp query pool.
    pub fn new(device: Arc<Device>, swapchain_image_count: usize) -> Result<Self> {
        let raw = device.raw();
        let mut kind = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let timeline_info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        // SAFETY: valid create infos on a live device.
        let timeline = unsafe { raw.create_semaphore(&timeline_info, None)? };
        device.set_name(timeline, "frame timeline");
        let mut slots = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for i in 0..FRAMES_IN_FLIGHT {
            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(device.graphics_family())
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            // SAFETY: as above.
            let (pool, command_buffer, image_available) = unsafe {
                let pool = raw.create_command_pool(&pool_info, None)?;
                let alloc = vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);
                let cb = raw.allocate_command_buffers(&alloc)?[0];
                let sem = raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                (pool, cb, sem)
            };
            device.set_name(image_available, &format!("image available {i}"));
            slots.push(PerSlot {
                pool,
                command_buffer,
                image_available,
            });
        }
        let mut render_finished = Vec::with_capacity(swapchain_image_count);
        for i in 0..swapchain_image_count {
            // SAFETY: as above.
            let sem = unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? };
            device.set_name(sem, &format!("render finished {i}"));
            render_finished.push(sem);
        }
        let timers = GpuTimers::new(&device, FRAMES_IN_FLIGHT)?;
        Ok(Self {
            device,
            timeline,
            slots,
            render_finished,
            timers,
            last_zones: Vec::new(),
            frame_number: 0,
            garbage: Vec::new(),
        })
    }

    /// Keeps `item` alive until the frame being recorded (or about to be) has completed on
    /// the GPU, then drops it: the deferred deletion every resource that a frame in flight
    /// may still reference goes through (images and buffers are RAII, so dropping frees).
    pub fn destroy_later<T: 'static>(&mut self, item: T) {
        self.garbage.push((self.frame_number, Box::new(item)));
    }

    /// Resources waiting in the deferred-deletion queue.
    pub fn pending_destructions(&self) -> usize {
        self.garbage.len()
    }

    /// Recreates the per-image semaphores after a swapchain rebuild (device must be idle).
    pub fn resize_swapchain(&mut self, swapchain_image_count: usize) -> Result<()> {
        let raw = self.device.raw();
        // SAFETY: the device is idle after a swapchain recreate.
        unsafe {
            for sem in self.render_finished.drain(..) {
                raw.destroy_semaphore(sem, None);
            }
        }
        for _ in 0..swapchain_image_count {
            // SAFETY: valid create info.
            let sem = unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? };
            self.render_finished.push(sem);
        }
        Ok(())
    }

    /// Waits until the slot for the next frame is free and returns it.
    pub fn wait_for_slot(&mut self) -> Result<FrameSlot> {
        let index = (self.frame_number % FRAMES_IN_FLIGHT as u64) as usize;
        let mut previous_gpu_ms = None;
        if self.frame_number >= FRAMES_IN_FLIGHT as u64 {
            let wait_value = self.frame_number + 1 - FRAMES_IN_FLIGHT as u64;
            let semaphores = [self.timeline];
            let values = [wait_value];
            let info = vk::SemaphoreWaitInfo::default()
                .semaphores(&semaphores)
                .values(&values);
            // SAFETY: live timeline semaphore.
            unsafe { self.device.raw().wait_semaphores(&info, u64::MAX)? };
            // The frame that wrote the slot's timestamps has completed (timeline wait above).
            self.last_zones = self.timers.read(index);
            if !self.last_zones.is_empty() {
                previous_gpu_ms = Some(self.last_zones.iter().map(|z| z.ms).sum());
            }
            // Every frame up to that one has completed: its retired resources can go.
            let completed = self.frame_number - FRAMES_IN_FLIGHT as u64;
            self.garbage.retain(|(frame, _)| *frame > completed);
        }
        Ok(FrameSlot {
            index,
            frame_number: self.frame_number,
            previous_gpu_ms,
        })
    }

    /// The timing zones of the frame that last completed on the slot returned by the latest
    /// [`Frames::wait_for_slot`] (two frames ago), in recording order.
    pub fn gpu_zones(&self) -> &[GpuZone] {
        &self.last_zones
    }

    /// The marker of `slot`, for [`crate::Commands::with_timers`].
    pub fn timer_slot(&self, slot: FrameSlot) -> std::rc::Rc<GpuTimerSlot> {
        self.timers.slot(slot.index)
    }

    /// Semaphore to signal when the swapchain image for this slot is acquired.
    pub fn image_available(&self, slot: FrameSlot) -> vk::Semaphore {
        self.slots[slot.index].image_available
    }

    /// Semaphore signalled when rendering into swapchain image `image_index` is done.
    pub fn render_finished(&self, image_index: u32) -> vk::Semaphore {
        self.render_finished[image_index as usize]
    }

    /// Resets the slot's command pool and begins its command buffer (writes the start timestamp).
    pub fn begin(&self, slot: FrameSlot) -> Result<vk::CommandBuffer> {
        let raw = self.device.raw();
        let per = &self.slots[slot.index];
        // SAFETY: the slot's previous frame has completed (`wait_for_slot`), so its pool and
        // command buffer may be reset and re-recorded.
        unsafe {
            raw.reset_command_pool(per.pool, vk::CommandPoolResetFlags::empty())?;
            raw.begin_command_buffer(
                per.command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        self.timers.begin(per.command_buffer, slot.index);
        Ok(per.command_buffer)
    }

    /// Ends the command buffer and submits it: waits for the acquired image, signals the
    /// per-image `render_finished` semaphore and the timeline.
    pub fn submit(&mut self, slot: FrameSlot, image_index: u32) -> Result<()> {
        let raw = self.device.raw();
        let per = &self.slots[slot.index];
        let cb = per.command_buffer;
        self.timers.end(cb, slot.index);
        // SAFETY: the command buffer is in the recording state (begun by `begin`).
        unsafe { raw.end_command_buffer(cb)? };
        let waits = [vk::SemaphoreSubmitInfo::default()
            .semaphore(per.image_available)
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
        let signals = [
            vk::SemaphoreSubmitInfo::default()
                .semaphore(self.render_finished[image_index as usize])
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
            vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(slot.frame_number + 1)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
        ];
        let cbs = [vk::CommandBufferSubmitInfo::default().command_buffer(cb)];
        let submit = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .command_buffer_infos(&cbs)
            .signal_semaphore_infos(&signals);
        // SAFETY: the command buffer is ended and every semaphore is live.
        unsafe { raw.queue_submit2(self.device.graphics_queue(), &[submit], vk::Fence::null())? };
        self.frame_number += 1;
        Ok(())
    }

    /// Frames submitted so far.
    pub fn frame_number(&self) -> u64 {
        self.frame_number
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        self.device.wait_idle();
        // Nothing is in flight any more: retired resources can go now.
        self.garbage.clear();
        let raw = self.device.raw();
        // SAFETY: the GPU is idle, nothing references these objects any more.
        unsafe {
            for slot in self.slots.drain(..) {
                raw.destroy_semaphore(slot.image_available, None);
                raw.destroy_command_pool(slot.pool, None);
            }
            for sem in self.render_finished.drain(..) {
                raw.destroy_semaphore(sem, None);
            }
            raw.destroy_semaphore(self.timeline, None);
        }
    }
}
