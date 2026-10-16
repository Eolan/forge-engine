//! The sample clock: what the simulation schedules sounds against.
//!
//! The mixer counts the frames it has rendered. The game reads that count through a
//! [`Clock`] and stamps a command with the frame it should act on; the mixer applies it on
//! that frame, within the block that contains it. A command stamped with a frame already
//! rendered acts at the start of the next block and is counted as late
//! ([`crate::MixerStats::late_commands`]).
//!
//! The simulation runs in ticks (`forge-sim`: 60 a second). A [`TickClock`] ties a tick to
//! a frame once, and from then on gives each tick its frame: 800 frames a tick at 48 kHz,
//! exactly, so an event the simulation stamps with a tick and a fraction of it plays on the
//! same sample in every run.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The mixer's frame count, shared between the audio thread (which advances it once a
/// block) and the game thread (which reads it).
#[derive(Clone, Debug, Default)]
pub struct Clock {
    frames: Arc<AtomicU64>,
}

impl Clock {
    /// A clock at frame 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The first frame of the next block the mixer will render: the earliest frame a
    /// command sent now can act on without being late. What is heard lags it by the
    /// output's buffering ([`crate::OutputRing`]) and the limiter's look-ahead
    /// ([`crate::Mixer::latency`]).
    pub fn now(&self) -> u64 {
        self.frames.load(Ordering::Acquire)
    }

    /// Set by the mixer after each block.
    pub(crate) fn set(&self, frames: u64) {
        self.frames.store(frames, Ordering::Release);
    }
}

/// Simulation ticks to sample frames: a tick tied to a frame, and the rates of both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickClock {
    /// The frame `origin_tick` starts on.
    pub origin_frame: u64,
    /// The tick that starts on `origin_frame`.
    pub origin_tick: u64,
    /// Ticks a second (`forge_sim::TICK_RATE`, 60).
    pub ticks_per_second: u32,
    /// The mixer's frames a second.
    pub sample_rate: u32,
}

impl TickClock {
    /// Ties `tick` to `frame`.
    pub fn new(
        origin_tick: u64,
        origin_frame: u64,
        ticks_per_second: u32,
        sample_rate: u32,
    ) -> Self {
        assert!(ticks_per_second > 0 && sample_rate > 0);
        Self {
            origin_frame,
            origin_tick,
            ticks_per_second,
            sample_rate,
        }
    }

    /// The frame a tick starts on (rounded down when a tick is not a whole number of
    /// frames). Ticks before the origin map to the origin.
    pub fn frame(&self, tick: u64) -> u64 {
        let ticks = tick.saturating_sub(self.origin_tick) as u128;
        self.origin_frame
            + (ticks * self.sample_rate as u128 / self.ticks_per_second as u128) as u64
    }

    /// The frame at `fraction` (0 to 1) of the way through `tick`.
    pub fn frame_at(&self, tick: u64, fraction: f64) -> u64 {
        let start = self.frame(tick);
        let length = self.frame(tick + 1) - start;
        start
            + (fraction.clamp(0.0, 1.0) * length as f64)
                .floor()
                .min(length as f64) as u64
    }

    /// The tick a frame falls in.
    pub fn tick(&self, frame: u64) -> u64 {
        let frames = frame.saturating_sub(self.origin_frame) as u128;
        self.origin_tick
            + (frames * self.ticks_per_second as u128 / self.sample_rate as u128) as u64
    }

    /// Frames a tick, as a real number (800 at 48 kHz and 60 ticks a second).
    pub fn frames_per_tick(&self) -> f64 {
        self.sample_rate as f64 / self.ticks_per_second as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_map_to_whole_frames() {
        let clock = TickClock::new(100, 4800, 60, 48_000);
        assert_eq!(clock.frame(100), 4800);
        assert_eq!(clock.frame(101), 5600);
        assert_eq!(clock.frame(160), 4800 + 48_000);
        assert_eq!(clock.frame(50), 4800);
        assert_eq!(clock.frame_at(101, 0.5), 6000);
        assert_eq!(clock.tick(5599), 100);
        assert_eq!(clock.tick(5600), 101);
        assert_eq!(clock.frames_per_tick(), 800.0);
    }

    #[test]
    fn uneven_rates_round_down_without_drift() {
        // 44.1 kHz at 60 ticks: 735 frames a tick, still whole; at 144 ticks: 306.25.
        let clock = TickClock::new(0, 0, 144, 44_100);
        assert_eq!(clock.frame(4), 1225);
        assert_eq!(clock.frame(144 * 3600), 44_100 * 3600);
    }
}
