//! Named GPU timing zones from timestamp queries.
//!
//! Every frame slot owns a range of timestamps, reset from the host when the slot is reused.
//! Each batch of the frame (one per submission, issue #77) starts with a timestamp
//! ([`GpuTimerSlot::start`]); every [`GpuTimerSlot::mark`] writes one more at the bottom of the
//! pipe and names the work since the previous one; [`GpuTimers::end`] closes the frame. When
//! the slot is reused two frames later, [`GpuTimers::read`] turns the timestamps into
//! [`GpuZone`]s: consecutive differences within each batch, so on one queue the passes of a
//! frame add up to that queue's work. Queues overlap, so the frame's time is its span
//! ([`GpuTimers::span_ms`]), not the sum of its zones. Labels are `group/name`
//! (`geometry/meshlet pass 1`); the profiler groups by prefix.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use ash::vk;

use crate::device::{Device, QueueKind};
use crate::error::Result;

/// Timestamps per frame slot (a start per batch, a mark per zone, the frame's end).
pub const MAX_MARKS_PER_FRAME: u32 = 96;

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
    /// The queue it ran on.
    pub queue: QueueKind,
}

/// What one timestamp was.
#[derive(Clone, Copy, Debug)]
struct Entry {
    label: &'static str,
    queue: QueueKind,
    /// The first timestamp of a batch (closes no zone).
    start: bool,
}

/// The timestamp range of one frame slot; [`crate::Commands::mark`] writes into it. Main
/// thread only (interior mutability without locks), like command recording.
pub struct GpuTimerSlot {
    device: Arc<Device>,
    pool: vk::QueryPool,
    base: u32,
    count: Cell<u32>,
    entries: RefCell<Vec<Entry>>,
    /// The queue of the batch being recorded, and whether its family writes timestamps.
    queue: Cell<QueueKind>,
    enabled: Cell<bool>,
}

impl GpuTimerSlot {
    /// Opens a batch on `queue`: a timestamp that waits for the work before it on the queue.
    pub fn start(&self, cb: vk::CommandBuffer, queue: QueueKind) {
        self.queue.set(queue);
        self.enabled.set(self.device.queue_timestamps(queue));
        self.write(cb, "start", true, vk::PipelineStageFlags2::ALL_COMMANDS);
    }

    /// Writes a timestamp (bottom of pipe) that closes the span called `label`. Silently
    /// ignored past [`MAX_MARKS_PER_FRAME`], and on a queue without timestamps.
    pub fn mark(&self, cb: vk::CommandBuffer, label: &'static str) {
        self.write(cb, label, false, vk::PipelineStageFlags2::BOTTOM_OF_PIPE);
    }

    fn write(
        &self,
        cb: vk::CommandBuffer,
        label: &'static str,
        start: bool,
        stage: vk::PipelineStageFlags2,
    ) {
        let count = self.count.get();
        if count >= MAX_MARKS_PER_FRAME || !self.enabled.get() {
            return;
        }
        // SAFETY: the command buffer is recording and the query index is inside the slot's
        // range, which `GpuTimers::reset` reset for this frame.
        unsafe {
            self.device
                .raw()
                .cmd_write_timestamp2(cb, stage, self.pool, self.base + count)
        };
        self.entries.borrow_mut().push(Entry {
            label,
            queue: self.queue.get(),
            start,
        });
        self.count.set(count + 1);
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
                    entries: RefCell::new(Vec::with_capacity(MAX_MARKS_PER_FRAME as usize)),
                    queue: Cell::new(QueueKind::Graphics),
                    enabled: Cell::new(false),
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

    /// Resets `slot`'s range from the host before its frame is recorded: every queue can
    /// then write into it, whatever order the batches run in. The slot's previous frame has
    /// completed (the caller waited).
    pub fn reset(&self, slot: usize) {
        let s = &self.slots[slot];
        // SAFETY: the pool is live and no pending work uses the slot's range.
        unsafe {
            self.device
                .raw()
                .reset_query_pool(self.pool, s.base, MAX_MARKS_PER_FRAME)
        };
        s.count.set(0);
        s.entries.borrow_mut().clear();
    }

    /// Ends a frame on `slot` in its last batch (graphics): the work since the last mark is
    /// `app/end of frame`.
    pub fn end(&mut self, cb: vk::CommandBuffer, slot: usize) {
        let s = &self.slots[slot];
        s.write(
            cb,
            "app/end of frame",
            false,
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
        let entries = s.entries.borrow();
        // Each batch opens with a start entry, so a mark's previous entry is its batch's.
        (1..count)
            .filter(|&i| !entries[i].start)
            .map(|i| GpuZone {
                label: entries[i].label,
                ms: stamps[i].saturating_sub(stamps[i - 1]) as f64 * period,
                start_ticks: stamps[i - 1],
                end_ticks: stamps[i],
                queue: entries[i].queue,
            })
            .collect()
    }

    /// The frame's GPU time: from the first zone's start to the last zone's end, on any
    /// queue. `None` without zones.
    pub fn span_ms(device: &Device, zones: &[GpuZone]) -> Option<f64> {
        let start = zones.iter().map(|z| z.start_ticks).min()?;
        let end = zones.iter().map(|z| z.end_ticks).max()?;
        Some(end.saturating_sub(start) as f64 * f64::from(device.timestamp_period_ns()) / 1.0e6)
    }
}

impl Drop for GpuTimers {
    fn drop(&mut self) {
        // SAFETY: the frames that used the pool have completed (`Frames` drops after idling),
        // and every slot handle only stores the raw pool handle.
        unsafe { self.device.raw().destroy_query_pool(self.pool, None) };
    }
}
