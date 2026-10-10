//! Threads' priorities: the frames' thread above the work beside it (#220).
//!
//! A planet's tiles are cooked while the camera flies, on a pool whose tiles spread their loops
//! over every thread of the machine, and the frames waited for cores. Measured on the planet's
//! tour with every tile cooked (two runs each):
//!
//! - **everything at the normal priority:** 55 and 78 frames over 4 ms, 5 and 6 over 8;
//! - **the cooking threads below normal:** 142 and 134 over 4 ms, 42 and 39 over 8, worse. The
//!   frame thread then waits on what a preempted cooking thread holds (the heap, a lock);
//! - **the frame thread above normal, the cooking at normal:** 30 and 33 over 4 ms, 4 and 5 over
//!   8, the tiles as fast.

/// Raises the calling thread above the normal priority (`THREAD_PRIORITY_ABOVE_NORMAL` on
/// Windows), for the thread that makes the frames: the scheduler runs it first when other work
/// is ready too. Nothing elsewhere: on Linux raising a thread needs a privilege a game lacks.
pub fn raise_current_thread_priority() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut core::ffi::c_void;
            fn SetThreadPriority(thread: *mut core::ffi::c_void, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
        // SAFETY: `GetCurrentThread` returns the calling thread's pseudo-handle, which needs no
        // closing and is valid for this call; `SetThreadPriority` reads it and an integer.
        let done = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
        if done == 0 {
            tracing::debug!("the frame thread's priority could not be raised");
        }
    }
}
