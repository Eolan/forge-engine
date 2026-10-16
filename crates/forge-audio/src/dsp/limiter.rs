//! A look-ahead peak limiter that never lets a sample past its ceiling.
//!
//! For each input frame the gain that would bring its peak (the larger of its two channels)
//! down to the ceiling is the frame's target. The target passes through a release follower
//! (it drops at once and rises back with a time constant), then a minimum over the last `L`
//! frames, then a moving average over the last `L` frames, while the audio is delayed by
//! `L − 1` frames. Every value the average takes in is a minimum over a window that contains
//! the delayed frame's own target, so the gain applied to a frame is never above that
//! frame's target: the ceiling holds by construction, and the average turns the gain down
//! along a straight ramp over the look-ahead instead of a step. The peak-hold-then-average
//! form follows Geraint Luff's "Designing a straightforward limiter" (Signalsmith Audio,
//! 2022). A final clamp at the ceiling only catches rounding; the mixer counts the samples it
//! touches by more than that.

use crate::db_to_gain;

/// How the limiter behaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimiterParams {
    /// The highest peak let through, in dBFS (−1 dB leaves room for the converters'
    /// inter-sample peaks).
    pub ceiling_db: f32,
    /// The look-ahead in frames: the audio's delay plus one, and the length of the gain's
    /// ramp down. 64 frames is 1.3 ms at 48 kHz.
    pub lookahead: usize,
    /// The time constant of the gain's recovery, in seconds.
    pub release: f32,
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self {
            ceiling_db: -1.0,
            lookahead: 64,
            release: 0.08,
        }
    }
}

/// The look-ahead limiter on the master bus.
#[derive(Clone, Debug)]
pub struct Limiter {
    ceiling: f32,
    len: usize,
    release_coef: f32,
    released: f32,
    // The audio's delay line, `len` frames of each channel; the oldest is `len − 1` back.
    delay_l: Box<[f32]>,
    delay_r: Box<[f32]>,
    delay_pos: usize,
    // The sliding minimum: a monotonic queue of (value, frame) in a ring of `len`.
    min_val: Box<[f32]>,
    min_at: Box<[u64]>,
    min_head: usize,
    min_count: usize,
    // The moving average: the last `len` minimums and their sum.
    avg_vals: Box<[f32]>,
    avg_pos: usize,
    avg_sum: f64,
    frame: u64,
    reduction: f32,
    overs: u64,
}

impl Limiter {
    /// A limiter for a stream at `rate` frames a second.
    pub fn new(params: LimiterParams, rate: u32) -> Self {
        let len = params.lookahead.max(1);
        let release_coef = 1.0 - (-1.0 / (params.release.max(1e-4) * rate as f32)).exp();
        Self {
            ceiling: db_to_gain(params.ceiling_db),
            len,
            release_coef,
            released: 1.0,
            delay_l: vec![0.0; len].into_boxed_slice(),
            delay_r: vec![0.0; len].into_boxed_slice(),
            delay_pos: 0,
            min_val: vec![0.0; len].into_boxed_slice(),
            min_at: vec![0; len].into_boxed_slice(),
            min_head: 0,
            min_count: 0,
            avg_vals: vec![1.0; len].into_boxed_slice(),
            avg_pos: 0,
            avg_sum: len as f64,
            frame: 0,
            reduction: 1.0,
            overs: 0,
        }
    }

    /// The ceiling as a linear gain.
    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// The delay the limiter adds, in frames.
    pub fn latency(&self) -> usize {
        self.len - 1
    }

    /// The lowest gain applied in the last block (1 when nothing was limited).
    pub fn reduction(&self) -> f32 {
        self.reduction
    }

    /// Samples the final clamp touched by more than rounding, since the start.
    pub fn overs(&self) -> u64 {
        self.overs
    }

    /// Limits a block of two channels in place.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = self.len;
        let ceiling = self.ceiling;
        let mut lowest = 1.0f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let peak = l.abs().max(r.abs());
            let target = if peak > ceiling { ceiling / peak } else { 1.0 };
            // Drops at once, recovers slowly, never above the target.
            self.released = if target < self.released {
                target
            } else {
                self.released + (target - self.released) * self.release_coef
            };

            // The minimum over the last `len` frames: first drop what left the window, so
            // the ring never holds more than `len`.
            if self.min_count > 0 && self.min_at[self.min_head] + len as u64 <= self.frame {
                self.min_head = (self.min_head + 1) % len;
                self.min_count -= 1;
            }
            let value = self.released;
            while self.min_count > 0 {
                let back = (self.min_head + self.min_count - 1) % len;
                if self.min_val[back] >= value {
                    self.min_count -= 1;
                } else {
                    break;
                }
            }
            let slot = (self.min_head + self.min_count) % len;
            self.min_val[slot] = value;
            self.min_at[slot] = self.frame;
            self.min_count += 1;
            let held = self.min_val[self.min_head];

            // The average of the last `len` minimums.
            self.avg_sum += held as f64 - self.avg_vals[self.avg_pos] as f64;
            self.avg_vals[self.avg_pos] = held;
            self.avg_pos += 1;
            if self.avg_pos == len {
                self.avg_pos = 0;
                // Re-add from scratch once a lap so rounding never accumulates.
                self.avg_sum = self.avg_vals.iter().map(|&v| v as f64).sum();
            }
            let gain = (self.avg_sum / len as f64) as f32;
            lowest = lowest.min(gain);

            // The frame `len − 1` back.
            self.delay_l[self.delay_pos] = *l;
            self.delay_r[self.delay_pos] = *r;
            self.delay_pos += 1;
            if self.delay_pos == len {
                self.delay_pos = 0;
            }
            let out_l = self.delay_l[self.delay_pos] * gain;
            let out_r = self.delay_r[self.delay_pos] * gain;
            *l = self.clamp(out_l);
            *r = self.clamp(out_r);
            self.frame += 1;
        }
        self.reduction = lowest;
    }

    #[inline]
    fn clamp(&mut self, x: f32) -> f32 {
        if x.abs() > self.ceiling {
            if x.abs() > self.ceiling * (1.0 + 1e-5) {
                self.overs += 1;
            }
            x.clamp(-self.ceiling, self.ceiling)
        } else {
            x
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_audio_passes_unchanged_after_the_delay() {
        let mut limiter = Limiter::new(LimiterParams::default(), 48_000);
        let input: Vec<f32> = (0..1000).map(|i| 0.5 * (i as f32 * 0.01).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        limiter.process(&mut l, &mut r);
        let delay = limiter.latency();
        for i in delay..input.len() {
            assert_eq!(l[i], input[i - delay]);
            assert_eq!(r[i], input[i - delay]);
        }
        assert!(l[..delay].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn a_step_is_ramped_and_never_passes_the_ceiling() {
        let params = LimiterParams::default();
        let mut limiter = Limiter::new(params, 48_000);
        let ceiling = limiter.ceiling();
        // Silence, then a full-scale +12 dB square.
        let mut l: Vec<f32> = (0..4800)
            .map(|i| {
                if i < 1000 {
                    0.0
                } else if i % 50 < 25 {
                    4.0
                } else {
                    -4.0
                }
            })
            .collect();
        let mut r = l.clone();
        limiter.process(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|&x| x.abs() <= ceiling));
        assert_eq!(limiter.overs(), 0);
        // Settled: the square comes out at the ceiling.
        assert!((l[4000].abs() - ceiling).abs() < 1e-3, "{}", l[4000]);
    }
}
