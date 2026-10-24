//! The simulation's clock apart from the frame's (the audit's §4.2, 2026-10-25): the world
//! advances in whole ticks of [`crate::TICK`], however the frames come. A frame hands the
//! clock the wall time it took; the clock answers with the ticks to run, at most
//! [`SimClock::MAX_CATCH_UP`] of them (a stall never makes the sim run away), and keeps what is
//! left over for the next frame. Between two ticks the frame sits at [`SimClock::alpha`], for
//! the renderer to interpolate the poses of the last two ticks, never to extrapolate.
//!
//! A scripted run ([`SimClock::fixed`]) hands the clock exactly one tick a frame, so a capture
//! at frame N is tick N by construction, with no `--fixed-step` to remember.

use std::ops::Range;

use crate::TICK_RATE;

/// A tick's length in `f64`, exact for the clock's sums; [`crate::TICK`] is its `f32` for the
/// sim's own arithmetic.
const TICK_SECONDS: f64 = 1.0 / TICK_RATE as f64;
/// Slack on a whole tick's boundary, so sixty frames of a sixtieth are sixty ticks whatever
/// the last bit of their sum.
const SLACK: f64 = 1e-9;

/// The simulation's clock: the ticks run so far and the time left over since the last one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimClock {
    /// The ticks run so far: the tick the next advance starts from.
    pub tick: u64,
    /// Seconds since the last tick, under one tick.
    accumulator: f64,
    /// Whether every frame is exactly one tick (a scripted run).
    fixed: bool,
}

impl Default for SimClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SimClock {
    /// Most ticks one frame runs: a stall longer than this many ticks is dropped, so the sim
    /// catches up over a few frames rather than freezing the frame it returns on.
    pub const MAX_CATCH_UP: u64 = 4;

    /// A clock at tick 0 that follows the wall time.
    pub fn new() -> Self {
        Self {
            tick: 0,
            accumulator: 0.0,
            fixed: false,
        }
    }

    /// A clock that runs exactly one tick a frame, whatever the wall time: scripted runs.
    pub fn fixed() -> Self {
        Self {
            fixed: true,
            ..Self::new()
        }
    }

    /// Hands the clock a frame's `wall_dt` seconds and returns the ticks to run, in order,
    /// `tick..tick + n`: the whole ticks the time holds, at most [`Self::MAX_CATCH_UP`], the rest
    /// kept for the next frame. The clock counts them as run.
    pub fn advance(&mut self, wall_dt: f64) -> Range<u64> {
        let n = if self.fixed {
            1
        } else {
            self.accumulator += wall_dt.max(0.0);
            let whole = (self.accumulator / TICK_SECONDS + SLACK).floor();
            let n = (whole as u64).min(Self::MAX_CATCH_UP);
            // The time of the ticks dropped is dropped with them: the sim is late, not fast.
            self.accumulator = (self.accumulator - whole * TICK_SECONDS).max(0.0);
            n
        };
        let start = self.tick;
        self.tick += n;
        start..self.tick
    }

    /// Where the frame sits between the last tick and the next, in `[0, 1)`: the renderer
    /// interpolates the last two ticks' poses by it. Always 0 on a fixed clock.
    pub fn alpha(&self) -> f32 {
        if self.fixed {
            0.0
        } else {
            (self.accumulator / TICK_SECONDS).clamp(0.0, 0.999_999) as f32
        }
    }

    /// Seconds of simulated time since the start: the ticks run times a tick.
    pub fn seconds(&self) -> f64 {
        self.tick as f64 * TICK_SECONDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_runs_the_whole_ticks_its_time_holds_and_keeps_the_rest() {
        let mut clock = SimClock::new();
        // Half a tick: nothing yet, the frame sits halfway.
        assert_eq!(clock.advance(0.5 * TICK_SECONDS), 0..0);
        assert!((clock.alpha() - 0.5).abs() < 1e-5, "{}", clock.alpha());
        // Another: one tick, nothing left over.
        assert_eq!(clock.advance(0.5 * TICK_SECONDS), 0..1);
        assert!(clock.alpha() < 1e-5);
        // Two and a quarter ticks: two, a quarter kept.
        assert_eq!(clock.advance(2.25 * TICK_SECONDS), 1..3);
        assert!((clock.alpha() - 0.25).abs() < 1e-5);
        assert_eq!(clock.tick, 3);
        assert!((clock.seconds() - 3.0 / 60.0).abs() < 1e-9);
    }

    #[test]
    fn a_stall_is_caught_up_in_steps_and_its_extra_time_dropped() {
        let mut clock = SimClock::new();
        // A second's stall: four ticks now, not sixty, and nothing owed.
        assert_eq!(clock.advance(1.0), 0..SimClock::MAX_CATCH_UP);
        assert!(clock.alpha() < 1e-5, "{}", clock.alpha());
        assert_eq!(clock.advance(TICK_SECONDS), 4..5);
        // Nonsense time (a clock that stepped back) runs nothing.
        assert_eq!(clock.advance(-1.0), 5..5);
    }

    #[test]
    fn sixty_frames_of_a_sixtieth_are_sixty_ticks_without_drift() {
        let mut clock = SimClock::new();
        let mut ticks = 0;
        for _ in 0..600 {
            ticks += clock.advance(1.0 / 60.0).count();
        }
        assert_eq!(ticks, 600);
        assert!(clock.alpha() < 1e-3, "{}", clock.alpha());
    }

    #[test]
    fn a_fixed_clock_runs_one_tick_a_frame_whatever_the_time() {
        let mut clock = SimClock::fixed();
        assert_eq!(clock.advance(0.0), 0..1);
        assert_eq!(clock.advance(5.0), 1..2);
        assert_eq!(clock.alpha(), 0.0);
        assert_eq!(clock.tick, 2);
    }
}
