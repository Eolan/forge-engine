//! Heap allocations counted (the audit's §2.2, 2026-10-25): with the `alloc-count` feature the
//! process's allocator is the system's with a count of its allocations beside it, and the app
//! shell logs the mean per frame at exit (`forge_app::alloc_count`). The rule it serves: a
//! steady frame allocates next to nothing. Without the feature the count stays at zero.
//!
//! It lives here, with the crate's other `unsafe`, because a global allocator is an `unsafe
//! impl` and the app shell forbids `unsafe`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Allocations made in this process so far ([`allocations`]).
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

/// How many allocations the process has made so far; 0 without the `alloc-count` feature.
pub fn allocations() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}

/// Whether the allocations are counted (the `alloc-count` feature).
pub const COUNTING: bool = cfg!(feature = "alloc-count");

/// The system allocator with a count of its allocations.
pub struct Counting;

// SAFETY: every call is forwarded to the system allocator unchanged; the count is a relaxed
// atomic beside it, with no effect on the memory handed out.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller's contract is the system allocator's.
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: `ptr` came from `alloc` above with this `layout`.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[cfg(feature = "alloc-count")]
#[global_allocator]
static COUNTING_ALLOCATOR: Counting = Counting;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_grows_only_when_counting() {
        let before = allocations();
        let kept: Vec<Vec<u8>> = (0..8).map(|_| vec![0_u8; 16]).collect();
        let after = allocations();
        if COUNTING {
            assert!(after - before >= 8, "{before} -> {after}");
        } else {
            assert_eq!(before, 0);
            assert_eq!(after, 0);
        }
        drop(kept);
    }
}
