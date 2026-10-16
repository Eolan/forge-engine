//! The mix thread: the mixer on a thread of its own, ahead of the device.
//!
//! The thread renders blocks into an output ring as long as the ring has room for one, and
//! sleeps (up to 2 ms, or until the device side wakes it) when it is full. The device's
//! callback, which must never wait, only copies frames out of the ring ([`OutputRing::pull`]):
//! a device that asks for odd sizes or late does not change how the mixer runs, and a mixer
//! block that runs long is absorbed by the ring (D-011; `docs/research/audio.md` §7, on
//! wrapping `cpal`). The ring's length is the output's added latency.
//!
//! Nothing here opens a device. The tests drive the ring from a plain thread; a device
//! layer (`cpal` behind a feature, in a binary the owner starts) is the next step.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{JoinHandle, Thread};
use std::time::Duration;

use rtrb::{Consumer, RingBuffer};

use crate::{BLOCK, CHANNELS, Mixer};

/// The device's end of the output ring.
pub struct OutputRing {
    consumer: Consumer<f32>,
    mixer_thread: Thread,
    underruns: Arc<AtomicU64>,
}

impl OutputRing {
    /// Copies up to `out.len()` samples (interleaved stereo) out of the ring and wakes the
    /// mix thread. What the ring lacks is filled with silence and counted. Never blocks.
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        let available = self.consumer.slots() / CHANNELS * CHANNELS;
        let n = available.min(out.len() / CHANNELS * CHANNELS);
        let (popped, _) = self.consumer.pop_partial_slice(&mut out[..n]);
        let got = popped.len();
        if got < out.len() {
            out[got..].fill(0.0);
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.mixer_thread.unpark();
        got / CHANNELS
    }

    /// Frames waiting in the ring.
    pub fn buffered(&self) -> usize {
        self.consumer.slots() / CHANNELS
    }

    /// Pulls that came up short.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

/// The mix thread. Stopping it gives the mixer back.
pub struct MixThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Mixer>>,
}

impl MixThread {
    /// Starts the mixer on a thread feeding a ring of `frames` frames (rounded up to whole
    /// blocks, at least two).
    pub fn spawn(mut mixer: Mixer, frames: usize) -> (MixThread, OutputRing) {
        let blocks = frames.div_ceil(BLOCK).max(2);
        let (mut producer, consumer) = RingBuffer::new(blocks * BLOCK * CHANNELS);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("forge-audio mix".into())
            .spawn(move || {
                let mut block = vec![0.0f32; BLOCK * CHANNELS];
                while !flag.load(Ordering::Acquire) {
                    if producer.slots() >= block.len() {
                        mixer.render(&mut block);
                        let (_, rest) = producer.push_partial_slice(&block);
                        debug_assert!(rest.is_empty());
                    } else {
                        std::thread::park_timeout(Duration::from_millis(2));
                    }
                }
                mixer
            })
            .expect("spawning the mix thread");
        let ring = OutputRing {
            consumer,
            mixer_thread: handle.thread().clone(),
            underruns: Arc::new(AtomicU64::new(0)),
        };
        (
            MixThread {
                stop,
                handle: Some(handle),
            },
            ring,
        )
    }

    /// Stops the thread and returns the mixer.
    pub fn stop(mut self) -> Mixer {
        self.stop.store(true, Ordering::Release);
        let handle = self.handle.take().expect("a running mix thread");
        handle.thread().unpark();
        handle.join().expect("the mix thread panicked")
    }
}

impl Drop for MixThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}
