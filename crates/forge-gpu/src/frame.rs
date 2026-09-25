use std::sync::Arc;

use ash::vk;

use crate::device::{Device, QueueKind};
use crate::error::Result;
use crate::timers::{GpuTimerSlot, GpuTimers, GpuZone};

/// CPU frames that may be in flight ahead of the GPU.
pub const FRAMES_IN_FLIGHT: usize = 2;

/// A slot's command buffers on one queue: its pool, and the buffers recorded so far this
/// frame (reused frame after frame).
struct QueuePool {
    pool: vk::CommandPool,
    buffers: Vec<vk::CommandBuffer>,
    used: usize,
}

struct PerSlot {
    /// Per [`QueueKind`], for the kinds that have a queue of their own.
    pools: [Option<QueuePool>; 3],
    image_available: vk::Semaphore,
}

/// A frame slot handed out by [`Frames::wait_for_slot`].
#[derive(Clone, Copy, Debug)]
pub struct FrameSlot {
    /// Slot index in `0..FRAMES_IN_FLIGHT`.
    pub index: usize,
    /// Monotonic frame number.
    pub frame_number: u64,
    /// GPU time of the previous frame that used this slot, in milliseconds: from its first
    /// timestamp to its last, on every queue (overlapping work counts once).
    pub previous_gpu_ms: Option<f64>,
}

/// One submission of a frame: a command buffer on a queue, what it waits for on the other
/// queues' timelines, and the value it signals on its own (issue #77).
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    /// The queue it runs on (already resolved: [`Device::resolve_queue`]).
    pub queue: QueueKind,
    /// Its commands, from [`Frames::command_buffer`].
    pub command_buffer: vk::CommandBuffer,
    /// Per queue: the timeline value to wait for (0 for none) and the stages that wait.
    pub waits: [(u64, vk::PipelineStageFlags2); 3],
    /// The value it signals on its queue's timeline ([`Frames::reserve_value`]).
    pub signal: u64,
}

/// Frames-in-flight bookkeeping on timeline semaphores, plus GPU timestamps.
///
/// Per frame: `wait_for_slot` → acquire the swapchain image with [`Frames::image_available`]
/// → `begin` → the render graph records its batches ([`Frames::command_buffer`],
/// [`Frames::push_batch`]) → `submit` → present with [`Frames::render_finished`]. The frame
/// timeline value signalled by frame `n` is `n + 1`; waiting for `n + 1 - FRAMES_IN_FLIGHT`
/// before reusing a slot keeps exactly `FRAMES_IN_FLIGHT` frames queued and never blocks on
/// the whole queue (the previous engine's lesson). The last batch of a frame is on the
/// graphics queue and waits for every other queue's last batch, so the frame timeline
/// covers all of the frame's work.
pub struct Frames {
    device: Arc<Device>,
    timeline: vk::Semaphore,
    /// Per queue kind: the timeline its batches signal, and the last value reserved.
    queue_timelines: [vk::Semaphore; 3],
    queue_values: [u64; 3],
    slots: Vec<PerSlot>,
    render_finished: Vec<vk::Semaphore>,
    timers: GpuTimers,
    last_zones: Vec<GpuZone>,
    frame_number: u64,
    batches: Vec<Batch>,
    /// Resources retired by [`Frames::destroy_later`], tagged with the frame that may still
    /// use them; dropped once that frame has completed on the GPU.
    garbage: Vec<(u64, Box<dyn std::any::Any>)>,
}

fn timeline_semaphore(device: &Device, name: &str) -> Result<vk::Semaphore> {
    let mut kind = vk::SemaphoreTypeCreateInfo::default()
        .semaphore_type(vk::SemaphoreType::TIMELINE)
        .initial_value(0);
    let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
    // SAFETY: valid create infos on a live device.
    let semaphore = unsafe { device.raw().create_semaphore(&info, None)? };
    device.set_name(semaphore, name);
    Ok(semaphore)
}

impl Frames {
    /// Creates the per-slot command pools (one per queue), the semaphores and the timestamp
    /// query pool.
    pub fn new(device: Arc<Device>, swapchain_image_count: usize) -> Result<Self> {
        let raw = device.raw();
        let timeline = timeline_semaphore(&device, "frame timeline")?;
        let mut queue_timelines = [vk::Semaphore::null(); 3];
        for kind in QueueKind::ALL {
            queue_timelines[kind.index()] =
                timeline_semaphore(&device, &format!("{} timeline", kind.name()))?;
        }
        let mut slots = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for i in 0..FRAMES_IN_FLIGHT {
            let mut pools: [Option<QueuePool>; 3] = [None, None, None];
            for kind in QueueKind::ALL {
                if device.resolve_queue(kind) != kind {
                    continue;
                }
                let pool_info = vk::CommandPoolCreateInfo::default()
                    .queue_family_index(device.queue_family(kind))
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT);
                // SAFETY: valid create info on a live device.
                let pool = unsafe { raw.create_command_pool(&pool_info, None)? };
                pools[kind.index()] = Some(QueuePool {
                    pool,
                    buffers: Vec::new(),
                    used: 0,
                });
            }
            // SAFETY: as above.
            let image_available =
                unsafe { raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)? };
            device.set_name(image_available, &format!("image available {i}"));
            slots.push(PerSlot {
                pools,
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
            queue_timelines,
            queue_values: [0; 3],
            slots,
            render_finished,
            timers,
            last_zones: Vec::new(),
            frame_number: 0,
            batches: Vec::new(),
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
            previous_gpu_ms = GpuTimers::span_ms(&self.device, &self.last_zones);
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

    /// Starts recording the slot's frame: resets its command pools and its timestamps. The
    /// slot's previous frame has completed ([`Frames::wait_for_slot`]).
    pub fn begin(&mut self, slot: FrameSlot) -> Result<()> {
        let raw = self.device.raw();
        for pool in self.slots[slot.index].pools.iter_mut().flatten() {
            // SAFETY: the slot's previous frame has completed, so its pool and command
            // buffers may be reset and re-recorded.
            unsafe { raw.reset_command_pool(pool.pool, vk::CommandPoolResetFlags::empty())? };
            pool.used = 0;
        }
        self.timers.reset(slot.index);
        self.batches.clear();
        Ok(())
    }

    /// A command buffer of `slot` for the queue of `kind` (resolved), begun. Each call gives
    /// another one; they are reset at the slot's next [`Frames::begin`].
    pub fn command_buffer(
        &mut self,
        slot: FrameSlot,
        kind: QueueKind,
    ) -> Result<vk::CommandBuffer> {
        let raw = self.device.raw();
        let kind = self.device.resolve_queue(kind);
        let pool = self.slots[slot.index].pools[kind.index()]
            .as_mut()
            .expect("a pool exists for every resolved queue kind");
        if pool.used == pool.buffers.len() {
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool.pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            // SAFETY: valid allocate info on the slot's live pool.
            let cb = unsafe { raw.allocate_command_buffers(&alloc)?[0] };
            pool.buffers.push(cb);
        }
        let cb = pool.buffers[pool.used];
        pool.used += 1;
        // SAFETY: the pool was reset at `begin`, so the buffer is in the initial state.
        unsafe {
            raw.begin_command_buffer(
                cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        Ok(cb)
    }

    /// Reserves the next value of `kind`'s timeline, for a batch to signal.
    pub fn reserve_value(&mut self, kind: QueueKind) -> u64 {
        let value = &mut self.queue_values[kind.index()];
        *value += 1;
        *value
    }

    /// Queues a batch for [`Frames::submit`], in submission order.
    pub fn push_batch(&mut self, batch: Batch) {
        self.batches.push(batch);
    }

    /// Ends and submits the frame's batches in order. The first graphics batch waits for the
    /// acquired image; the last batch (graphics) signals the per-image `render_finished`
    /// semaphore and the frame timeline, after closing the frame's timestamps.
    pub fn submit(&mut self, slot: FrameSlot, image_index: u32) -> Result<()> {
        let raw = self.device.raw();
        let per = &self.slots[slot.index];
        let last = self.batches.len().checked_sub(1);
        let first_graphics = self
            .batches
            .iter()
            .position(|b| b.queue == QueueKind::Graphics);
        for (i, batch) in self.batches.iter().enumerate() {
            let cb = batch.command_buffer;
            let is_last = Some(i) == last;
            if is_last {
                self.timers.end(cb, slot.index);
            }
            // SAFETY: the command buffer is in the recording state (`command_buffer`).
            unsafe { raw.end_command_buffer(cb)? };
            let mut waits = Vec::with_capacity(4);
            if Some(i) == first_graphics {
                waits.push(
                    vk::SemaphoreSubmitInfo::default()
                        .semaphore(per.image_available)
                        .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT),
                );
            }
            for kind in QueueKind::ALL {
                let (value, stages) = batch.waits[kind.index()];
                if value > 0 {
                    waits.push(
                        vk::SemaphoreSubmitInfo::default()
                            .semaphore(self.queue_timelines[kind.index()])
                            .value(value)
                            .stage_mask(stages),
                    );
                }
            }
            // A batch on a queue without graphics stages signals once its compute or copy
            // work is done; ALL_COMMANDS would wait for the end of the pipe on AMD.
            let signal_stage = match batch.queue {
                QueueKind::Graphics => vk::PipelineStageFlags2::ALL_COMMANDS,
                QueueKind::Compute => vk::PipelineStageFlags2::COMPUTE_SHADER,
                QueueKind::Transfer => vk::PipelineStageFlags2::ALL_TRANSFER,
            };
            let mut signals = vec![
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(self.queue_timelines[batch.queue.index()])
                    .value(batch.signal)
                    .stage_mask(signal_stage),
            ];
            if is_last {
                signals.push(
                    vk::SemaphoreSubmitInfo::default()
                        .semaphore(self.render_finished[image_index as usize])
                        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
                );
                signals.push(
                    vk::SemaphoreSubmitInfo::default()
                        .semaphore(self.timeline)
                        .value(slot.frame_number + 1)
                        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
                );
            }
            let cbs = [vk::CommandBufferSubmitInfo::default().command_buffer(cb)];
            let submit = vk::SubmitInfo2::default()
                .wait_semaphore_infos(&waits)
                .command_buffer_infos(&cbs)
                .signal_semaphore_infos(&signals);
            // SAFETY: the command buffer is ended, every semaphore is live, and every value
            // waited for was reserved by an earlier batch, submitted before this one.
            unsafe {
                raw.queue_submit2(self.device.queue(batch.queue), &[submit], vk::Fence::null())?
            };
        }
        self.batches.clear();
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
                for pool in slot.pools.into_iter().flatten() {
                    raw.destroy_command_pool(pool.pool, None);
                }
            }
            for sem in self.render_finished.drain(..) {
                raw.destroy_semaphore(sem, None);
            }
            for sem in self.queue_timelines {
                raw.destroy_semaphore(sem, None);
            }
            raw.destroy_semaphore(self.timeline, None);
        }
    }
}
