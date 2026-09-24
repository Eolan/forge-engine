//! Named GPU timing zones from timestamp queries.
//!
//! Every frame slot owns a range of timestamps. [`GpuTimers::begin`] writes the first one
//! (top of pipe); every [`GpuTimerSlot::mark`] writes one more at the bottom of the pipe and
//! names the work since the previous mark; [`GpuTimers::end`] closes the frame. When the
//! slot is reused two frames later, [`GpuTimers::read`] turns the timestamps into
//! [`GpuZone`]s: consecutive differences, so the passes of a frame add up to the frame.
//! Labels are `group/name` (`geometry/meshlet pass 1`); the profiler groups by prefix.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use ash::vk;

use crate::device::Device;
use crate::error::Result;

/// Timestamps per frame slot (the first is the frame start, the last the frame end).
pub const MAX_MARKS_PER_FRAME: u32 = 64;

/// One measured span of GPU work.
#[derive(Clone, Copy, Debug)]
pub struct GpuZone {
    /// `group/name`.
    pub label: &'static str,
    /// Milliseconds between this mark and the previous one.
    pub ms: f64,
    /// Raw GPU timestamp of the previous mark (ticks of `timestamp_period_ns`).
    pub start_ticks: u64,
    /// Raw GPU timestamp of this mark.
    pub end_ticks: u64,
}

/// The timestamp range of one frame slot; [`crate::Commands::mark`] writes into it. Main
/// thread only (interior mutability without locks), like command recording.
pub struct GpuTimerSlot {
    device: Arc<Device>,
    pool: vk::QueryPool,
    base: u32,
    count: Cell<u32>,
    labels: RefCell<Vec<&'static str>>,
}

impl GpuTimerSlot {
    /// Writes a timestamp (bottom of pipe) that closes the span called `label`. Silently
    /// ignored past [`MAX_MARKS_PER_FRAME`].
    pub fn mark(&self, cb: vk::CommandBuffer, label: &'static str) {
        self.mark_at(cb, label, vk::PipelineStageFlags2::BOTTOM_OF_PIPE);
    }

    fn mark_at(&self, cb: vk::CommandBuffer, label: &'static str, stage: vk::PipelineStageFlags2) {
        let count = self.count.get();
        if count >= MAX_MARKS_PER_FRAME {
            return;
        }
        // SAFETY: the command buffer is recording and the query index is inside the slot's
        // range, which `GpuTimers::begin` reset for this frame.
        unsafe {
            self.device
                .raw()
                .cmd_write_timestamp2(cb, stage, self.pool, self.base + count)
        };
        self.labels.borrow_mut().push(label);
        self.count.set(count + 1);
    }

    fn reset(&self, cb: vk::CommandBuffer) {
        // SAFETY: recording; the previous frame on this slot has completed (the caller waited).
        unsafe {
            self.device
                .raw()
                .cmd_reset_query_pool(cb, self.pool, self.base, MAX_MARKS_PER_FRAME)
        };
        self.count.set(0);
        self.labels.borrow_mut().clear();
    }
}

/// The timestamp pool of all frame slots.
pub struct GpuTimers {
    device: Arc<Device>,
    pool: vk::QueryPool,
    slots: Vec<Rc<GpuTimerSlot>>,
    submitted: Vec<u32>,
}

impl GpuTimers {
    /// A pool with [`MAX_MARKS_PER_FRAME`] timestamps per slot.
    pub fn new(device: &Arc<Device>, slot_count: usize) -> Result<Self> {
        let total = slot_count as u32 * MAX_MARKS_PER_FRAME;
        let info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(total);
        // SAFETY: valid create info on a live device.
        let pool = unsafe { device.raw().create_query_pool(&info, None)? };
        device.set_name(pool, "gpu timers");
        // Queries must be reset before first use; host reset is core since Vulkan 1.2.
        // SAFETY: the pool is live and unused.
        unsafe { device.raw().reset_query_pool(pool, 0, total) };
        let slots = (0..slot_count)
            .map(|i| {
                Rc::new(GpuTimerSlot {
                    device: Arc::clone(device),
                    pool,
                    base: i as u32 * MAX_MARKS_PER_FRAME,
                    count: Cell::new(0),
                    labels: RefCell::new(Vec::with_capacity(MAX_MARKS_PER_FRAME as usize)),
                })
            })
            .collect();
        Ok(Self {
            device: Arc::clone(device),
            pool,
            slots,
            submitted: vec![0; slot_count],
        })
    }

    /// The slot's marker, to hand to [`crate::Commands::with_timers`].
    pub fn slot(&self, index: usize) -> Rc<GpuTimerSlot> {
        Rc::clone(&self.slots[index])
    }

    /// Starts a frame on `slot`: resets its range and writes the start timestamp. The start
    /// waits for all earlier commands (the previous frame's tail), so the first zone measures
    /// only its own work.
    pub fn begin(&self, cb: vk::CommandBuffer, slot: usize) {
        let s = &self.slots[slot];
        s.reset(cb);
        s.mark_at(cb, "start", vk::PipelineStageFlags2::ALL_COMMANDS);
    }

    /// Ends a frame on `slot`: the work since the last mark is `app/end of frame`.
    pub fn end(&mut self, cb: vk::CommandBuffer, slot: usize) {
        let s = &self.slots[slot];
        s.mark_at(
            cb,
            "app/end of frame",
            vk::PipelineStageFlags2::ALL_COMMANDS,
        );
        self.submitted[slot] = s.count.get();
    }

    /// The zones of the frame last submitted on `slot`; call after that frame completed.
    /// Empty when nothing was submitted on the slot yet or the results are unavailable.
    pub fn read(&self, slot: usize) -> Vec<GpuZone> {
        let s = &self.slots[slot];
        let count = self.submitted[slot] as usize;
        if count < 2 {
            return Vec::new();
        }
        let mut stamps = vec![0_u64; count];
        // SAFETY: the frame that wrote these queries has completed.
        let ok = unsafe {
            self.device
                .raw()
                .get_query_pool_results(
                    self.pool,
                    s.base,
                    &mut stamps,
                    vk::QueryResultFlags::TYPE_64,
                )
                .is_ok()
        };
        if !ok {
            return Vec::new();
        }
        let period = f64::from(self.device.timestamp_period_ns()) / 1.0e6;
        let labels = s.labels.borrow();
        (1..count)
            .map(|i| GpuZone {
                label: labels[i],
                ms: stamps[i].saturating_sub(stamps[i - 1]) as f64 * period,
                start_ticks: stamps[i - 1],
                end_ticks: stamps[i],
            })
            .collect()
    }
}

impl Drop for GpuTimers {
    fn drop(&mut self) {
        // SAFETY: the frames that used the pool have completed (`Frames` drops after idling),
        // and every slot handle only stores the raw pool handle.
        unsafe { self.device.raw().destroy_query_pool(self.pool, None) };
    }
}
