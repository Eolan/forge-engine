//! Heap allocations per frame (the audit's §2.2, 2026-10-25): with the `alloc-count` feature
//! (`forge_task::alloc_count`'s allocator) the shell marks the process's count at each frame's
//! end and logs the mean per frame at exit (`allocations:`), over the frames after the first
//! sixty. What a frame allocates every frame (a `format!`, a `Vec` built and dropped) shows here
//! before it shows in a profile. Without the feature nothing is counted or logged.

use forge_task::{COUNTING, allocations};

/// The mean allocations per frame over the counted frames, from the shell's marks.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerFrame {
    /// The count at the last frame's end.
    mark: u64,
    /// Allocations summed over the counted frames, and how many were counted.
    sum: u64,
    frames: u64,
}

impl PerFrame {
    /// Frames left out at the start: the scene settles, the caches fill.
    pub const WARM_UP: u64 = 60;

    /// Ends a frame: `frame` is its number from the demo's first; the first [`PerFrame::WARM_UP`]
    /// frames only set the mark.
    pub fn end_frame(&mut self, frame: u64) {
        let now = allocations();
        if frame >= Self::WARM_UP {
            self.sum += now.saturating_sub(self.mark);
            self.frames += 1;
        }
        self.mark = now;
    }

    /// The mean per counted frame, when frames were counted and allocations are counted.
    pub fn mean(&self) -> Option<f64> {
        (COUNTING && self.frames > 0).then(|| self.sum as f64 / self.frames as f64)
    }

    /// Frames counted.
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warm_up_sets_the_mark_and_the_rest_counts() {
        let mut per = PerFrame::default();
        for frame in 0..PerFrame::WARM_UP {
            per.end_frame(frame);
        }
        assert_eq!(per.frames(), 0);
        let _kept: Vec<Vec<u8>> = (0..8).map(|_| vec![0_u8; 16]).collect();
        per.end_frame(PerFrame::WARM_UP);
        assert_eq!(per.frames(), 1);
        if COUNTING {
            // The eight vectors above, at least (the test harness allocates too).
            assert!(per.mean().is_some_and(|m| m >= 8.0), "{per:?}");
        } else {
            assert_eq!(per.mean(), None);
        }
    }
}
