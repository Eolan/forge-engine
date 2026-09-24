//! What the GPU memory holds and what the OS allows (issue #9). Every allocation is counted
//! under a [`MemoryCategory`], every host write into GPU-visible memory is counted as
//! uploaded bytes, and `VK_EXT_memory_budget` gives each heap's usage and budget for the
//! whole process: the driver's own allocations, the swapchain and Streamline included.

use std::sync::atomic::{AtomicU64, Ordering};

use ash::vk;

use crate::device::Device;

/// What an allocation is for; the counters sum by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryCategory {
    /// Meshes, meshlets, instances and the scene tables uploaded with them.
    Geometry,
    /// Persistent images the GPU writes: histories, depth pyramids, lookup tables, the
    /// upscaler's output.
    Targets,
    /// The render graph's transient images (the whole heap when they alias in one).
    Transient,
    /// Images uploaded once and only sampled (fonts, constant tables).
    Textures,
    /// Buffers the GPU writes for itself: work lists, visible-cluster lists, histograms.
    Work,
    /// Host-written per-frame data (constants, indirect arguments, the overlay's cells): in
    /// device-local memory when the driver exposes Resizable BAR.
    Frame,
    /// Staging buffers of one-shot uploads and the buffers the host reads back (statistics,
    /// histograms, captures).
    Transfer,
}

impl MemoryCategory {
    /// Every category, in display order.
    pub const ALL: [Self; 7] = [
        Self::Geometry,
        Self::Targets,
        Self::Transient,
        Self::Textures,
        Self::Work,
        Self::Frame,
        Self::Transfer,
    ];

    /// Number of categories.
    pub const COUNT: usize = Self::ALL.len();

    /// Name for displays.
    pub fn name(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Targets => "render targets",
            Self::Transient => "transient heap",
            Self::Textures => "textures",
            Self::Work => "GPU work buffers",
            Self::Frame => "per-frame data",
            Self::Transfer => "staging + readback",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The device's running totals, updated by every allocation, free, upload and readback.
#[derive(Default)]
pub(crate) struct MemoryCounters {
    allocated: [AtomicU64; MemoryCategory::COUNT],
    uploaded: AtomicU64,
    read_back: AtomicU64,
}

impl MemoryCounters {
    pub(crate) fn allocate(&self, category: MemoryCategory, bytes: u64) {
        self.allocated[category.index()].fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn free(&self, category: MemoryCategory, bytes: u64) {
        self.allocated[category.index()].fetch_sub(bytes, Ordering::Relaxed);
    }

    pub(crate) fn upload(&self, bytes: u64) {
        self.uploaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn read_back(&self, bytes: u64) {
        self.read_back.fetch_add(bytes, Ordering::Relaxed);
    }

    fn allocated(&self) -> [u64; MemoryCategory::COUNT] {
        std::array::from_fn(|i| self.allocated[i].load(Ordering::Relaxed))
    }
}

/// One memory heap of the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapReport {
    /// Heap index.
    pub index: u32,
    /// Size in bytes.
    pub size: u64,
    /// Whether the heap is the GPU's own memory.
    pub device_local: bool,
    /// Whether the host can map memory of this heap: on a device-local heap that is Resizable
    /// BAR (the whole heap) or the 256 MiB window without it.
    pub host_visible: bool,
    /// Bytes this process uses in the heap, everything counted (`None` without
    /// `VK_EXT_memory_budget`).
    pub usage: Option<u64>,
    /// Bytes the process can use before the OS starts moving its memory out (the heap size
    /// without `VK_EXT_memory_budget`; capped by `FORGE_VRAM_BUDGET_MB` on device-local heaps).
    pub budget: u64,
}

impl HeapReport {
    /// Name for displays.
    pub fn name(&self) -> &'static str {
        match (self.device_local, self.host_visible) {
            (true, true) if self.size < 1 << 30 => "BAR window",
            (true, true) => "VRAM (ReBAR)",
            (true, false) => "VRAM",
            (false, _) => "system RAM",
        }
    }
}

/// A snapshot of the device's memory ([`Device::memory_report`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryReport {
    /// Every heap.
    pub heaps: Vec<HeapReport>,
    /// Bytes allocated per category, in [`MemoryCategory::ALL`] order.
    pub allocated: [u64; MemoryCategory::COUNT],
    /// Bytes of device memory the allocator holds: what it handed out plus the free space
    /// inside its blocks.
    pub reserved: u64,
    /// Host bytes written into GPU-visible memory since the device was created.
    pub uploaded: u64,
    /// Host bytes read from GPU-visible memory since the device was created.
    pub read_back: u64,
}

/// The share of the budget past which memory is short: the engine plans with the budget
/// minus 10 % (D-018), so a heap above this is eating into that reserve.
pub const BUDGET_WARNING: f64 = 0.9;

impl MemoryReport {
    /// Bytes allocated for `category`.
    pub fn allocated(&self, category: MemoryCategory) -> u64 {
        self.allocated[category.index()]
    }

    /// Bytes allocated over every category.
    pub fn total_allocated(&self) -> u64 {
        self.allocated.iter().sum()
    }

    /// Usage and budget summed over the device-local heaps (`None` without
    /// `VK_EXT_memory_budget`).
    pub fn device_local(&self) -> Option<(u64, u64)> {
        let mut usage = 0;
        let mut budget = 0;
        for heap in self.heaps.iter().filter(|h| h.device_local) {
            usage += heap.usage?;
            budget += heap.budget;
        }
        Some((usage, budget))
    }

    /// Bytes the process uses outside the engine's allocator: the driver, the swapchain,
    /// Streamline (`None` without `VK_EXT_memory_budget`).
    pub fn outside_allocator(&self) -> Option<u64> {
        let usage = self.heaps.iter().map(|h| h.usage).sum::<Option<u64>>()?;
        Some(usage.saturating_sub(self.reserved))
    }
}

impl Device {
    /// Queries the heaps' usage and budget and reads the counters. It asks the driver, which
    /// asks the OS: call it a few times per second, not per allocation.
    pub fn memory_report(&self) -> MemoryReport {
        let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
        let mut properties = vk::PhysicalDeviceMemoryProperties2::default();
        let has_budget = self.features().memory_budget;
        if has_budget {
            properties = properties.push_next(&mut budget);
        }
        // SAFETY: property query on the device's physical device with a properly chained
        // structure (the budget structure only when the extension is enabled).
        unsafe {
            self.instance()
                .raw()
                .get_physical_device_memory_properties2(self.physical(), &mut properties)
        };
        let memory = properties.memory_properties;
        let types = &memory.memory_types[..memory.memory_type_count as usize];
        let heaps = memory.memory_heaps[..memory.memory_heap_count as usize]
            .iter()
            .enumerate()
            .map(|(i, heap)| {
                let device_local = heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL);
                let reported = if has_budget {
                    budget.heap_budget[i]
                } else {
                    heap.size
                };
                HeapReport {
                    index: i as u32,
                    size: heap.size,
                    device_local,
                    host_visible: types.iter().any(|t| {
                        t.heap_index == i as u32
                            && t.property_flags
                                .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    }),
                    usage: has_budget.then(|| budget.heap_usage[i]),
                    budget: match self.budget_cap() {
                        Some(cap) if device_local => reported.min(cap),
                        _ => reported,
                    },
                }
            })
            .collect();
        let counters = self.memory_counters();
        MemoryReport {
            heaps,
            allocated: counters.allocated(),
            reserved: self.with_allocator(|a| a.capacity()),
            uploaded: counters.uploaded.load(Ordering::Relaxed),
            read_back: counters.read_back.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap(device_local: bool, usage: u64, budget: u64) -> HeapReport {
        HeapReport {
            index: 0,
            size: 16 << 30,
            device_local,
            host_visible: false,
            usage: Some(usage),
            budget,
        }
    }

    #[test]
    fn categories_are_listed_once_in_index_order() {
        for (i, category) in MemoryCategory::ALL.iter().enumerate() {
            assert_eq!(category.index(), i);
        }
        let mut names: Vec<_> = MemoryCategory::ALL.iter().map(|c| c.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MemoryCategory::COUNT);
    }

    #[test]
    fn counters_add_and_release_per_category() {
        let counters = MemoryCounters::default();
        counters.allocate(MemoryCategory::Geometry, 1000);
        counters.allocate(MemoryCategory::Frame, 64);
        counters.allocate(MemoryCategory::Geometry, 24);
        counters.free(MemoryCategory::Geometry, 1000);
        counters.upload(300);
        counters.upload(12);
        counters.read_back(8);
        let allocated = counters.allocated();
        assert_eq!(allocated[MemoryCategory::Geometry.index()], 24);
        assert_eq!(allocated[MemoryCategory::Frame.index()], 64);
        assert_eq!(counters.uploaded.load(Ordering::Relaxed), 312);
        assert_eq!(counters.read_back.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn device_local_sums_and_outside_allocator_subtracts_the_blocks() {
        let report = MemoryReport {
            heaps: vec![
                heap(true, 900, 1000),
                heap(false, 100, 5000),
                heap(true, 50, 200),
            ],
            allocated: [0; MemoryCategory::COUNT],
            reserved: 700,
            uploaded: 0,
            read_back: 0,
        };
        assert_eq!(report.device_local(), Some((950, 1200)));
        assert_eq!(report.outside_allocator(), Some(350));
        let without_extension = MemoryReport {
            heaps: vec![HeapReport {
                usage: None,
                ..heap(true, 0, 1000)
            }],
            ..report
        };
        assert_eq!(without_extension.device_local(), None);
        assert_eq!(without_extension.outside_allocator(), None);
    }

    #[test]
    fn heaps_are_named_by_what_the_host_can_reach() {
        let vram = heap(true, 0, 0);
        assert_eq!(vram.name(), "VRAM");
        let rebar = HeapReport {
            host_visible: true,
            ..vram
        };
        assert_eq!(rebar.name(), "VRAM (ReBAR)");
        let window = HeapReport {
            size: 256 << 20,
            ..rebar
        };
        assert_eq!(window.name(), "BAR window");
        assert_eq!(heap(false, 0, 0).name(), "system RAM");
    }
}
