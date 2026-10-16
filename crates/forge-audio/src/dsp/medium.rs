//! The medium a bus is heard through: a low-pass, a stereo width and a gain, gliding when
//! they change.
//!
//! Under water the world above the surface is heard low, quiet and nearly without
//! direction (`docs/research/audio.md` §2.4: the surface costs about 29 dB, sound travels
//! 4.3 times faster so the ears' time differences shrink, and divers place a source only
//! within about 50°). [`crate::Control::BusFilter`] sets a bus's filter; the mixer moves to
//! it without a click: the cutoff glides at most a quarter of an octave a block (20 kHz to
//! 600 Hz in about 110 ms), the width and the gain (in decibels) follow with a 30 ms time constant, and the
//! filter is crossfaded in and out across a block when it engages or opens fully.

use super::SvfLowPass;

/// The Q of the two sections of a fourth-order Butterworth low-pass.
const Q: [f32; 2] = [0.541_196_1, 1.306_563];

/// Two state-variable sections: a fourth-order low-pass that glides without ringing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct LowPass([SvfLowPass; 2]);

impl LowPass {
    fn new(cutoff: f32, rate: u32) -> Self {
        Self(Q.map(|q| SvfLowPass::new(cutoff, q, rate)))
    }

    fn retune(&mut self, cutoff: f32, rate: u32) {
        for (s, q) in self.0.iter_mut().zip(Q) {
            s.retune(cutoff, q, rate);
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.0[0].tick(x);
        self.0[1].tick(y)
    }
}

/// A cutoff at or above this is no filter, Hz.
pub const OPEN_CUTOFF: f32 = 20_000.0;

/// Octaves the cutoff moves at most in a block.
const GLIDE: f32 = 0.25;

/// The time constant of the width and gain, seconds.
const SETTLE: f32 = 0.03;

/// What a bus's filter does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusFilter {
    /// A fourth-order low-pass's cutoff, Hz ([`OPEN_CUTOFF`] and above: none).
    pub cutoff: f32,
    /// A linear gain.
    pub gain: f32,
    /// The stereo width: 1 as it is, 0 mono (the sides scaled, the middle kept).
    pub width: f32,
}

impl BusFilter {
    /// Nothing done.
    pub const CLEAR: BusFilter = BusFilter {
        cutoff: OPEN_CUTOFF,
        gain: 1.0,
        width: 1.0,
    };

    /// The world above the surface heard from under water: low-passed at 600 Hz, 29 dB
    /// down, a fifth of its width.
    pub const UNDERWATER: BusFilter = BusFilter {
        cutoff: 600.0,
        gain: 0.035_481_34,
        width: 0.2,
    };

    fn is_clear(&self) -> bool {
        self.cutoff >= OPEN_CUTOFF && self.gain == 1.0 && self.width == 1.0
    }
}

/// A bus filter's state.
#[derive(Clone, Debug)]
pub struct MediumFilter {
    rate: u32,
    target: BusFilter,
    current: BusFilter,
    low_pass: [LowPass; 2],
    engaged: bool,
}

impl MediumFilter {
    /// A filter at `filter` from the start.
    pub fn new(filter: BusFilter, rate: u32) -> Self {
        let cutoff = filter.cutoff.min(OPEN_CUTOFF);
        Self {
            rate,
            target: filter,
            current: filter,
            low_pass: [LowPass::new(cutoff, rate); 2],
            engaged: filter.cutoff < OPEN_CUTOFF,
        }
    }

    /// Where it moves to.
    pub fn set(&mut self, filter: BusFilter) {
        self.target = filter;
    }

    /// Where it is.
    pub fn current(&self) -> BusFilter {
        self.current
    }

    /// The parameters at the end of the next block.
    fn next(&self, frames: usize) -> BusFilter {
        let (from, to) = (self.current.cutoff.max(1.0), self.target.cutoff.max(1.0));
        let octaves = (to / from).log2().clamp(-GLIDE, GLIDE);
        let mut cutoff = from * octaves.exp2();
        if (cutoff / to - 1.0).abs() < 1e-3 {
            cutoff = to;
        }
        let k = 1.0 - (-(frames as f32) / (SETTLE * self.rate as f32)).exp();
        let settle = |a: f32, b: f32| {
            let v = a + (b - a) * k;
            if (v - b).abs() < 1e-5 { b } else { v }
        };
        // The gain settles in the log domain, so 29 dB takes as long as 1 dB does.
        let ln = |g: f32| g.max(1e-4).ln();
        let gain = settle(ln(self.current.gain), ln(self.target.gain)).exp();
        BusFilter {
            cutoff,
            gain: if (gain / self.target.gain - 1.0).abs() < 1e-4 {
                self.target.gain
            } else {
                gain
            },
            width: settle(self.current.width, self.target.width),
        }
    }

    /// Moves the parameters by a block without sound (a bus nothing played into).
    pub fn skip(&mut self, frames: usize) {
        self.current = self.next(frames);
        if self.current.cutoff >= OPEN_CUTOFF && self.engaged {
            self.engaged = false;
            self.low_pass = [LowPass::new(OPEN_CUTOFF, self.rate); 2];
        }
    }

    /// Filters a stereo block in place.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.current.is_clear() && self.target.is_clear() && !self.engaged {
            return;
        }
        let n = left.len();
        let from = self.current;
        let to = self.next(n);
        // The low-pass, crossfaded in when it engages and out when it opens fully.
        let filtering = to.cutoff < OPEN_CUTOFF || self.engaged;
        if filtering {
            for lp in &mut self.low_pass {
                lp.retune(to.cutoff.min(OPEN_CUTOFF), self.rate);
            }
            let fade_in = !self.engaged;
            let fade_out = to.cutoff >= OPEN_CUTOFF;
            for (channel, samples) in [&mut *left, &mut *right].into_iter().enumerate() {
                let lp = &mut self.low_pass[channel];
                for (i, x) in samples.iter_mut().enumerate() {
                    let wet = lp.tick(*x);
                    let w = (i + 1) as f32 / n as f32;
                    *x = if fade_in {
                        *x + (wet - *x) * w
                    } else if fade_out {
                        wet + (*x - wet) * w
                    } else {
                        wet
                    };
                }
            }
            self.engaged = !fade_out;
            if fade_out {
                self.low_pass = [LowPass::new(OPEN_CUTOFF, self.rate); 2];
            }
        }
        // Width and gain, ramped across the block.
        if from.width != 1.0 || to.width != 1.0 || from.gain != 1.0 || to.gain != 1.0 {
            for i in 0..n {
                let w = (i + 1) as f32 / n as f32;
                let width = from.width + (to.width - from.width) * w;
                let gain = from.gain + (to.gain - from.gain) * w;
                let mid = 0.5 * (left[i] + right[i]);
                let side = 0.5 * (left[i] - right[i]) * width;
                left[i] = (mid + side) * gain;
                right[i] = (mid - side) * gain;
            }
        }
        self.current = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cutoff_glides_and_comes_back_clear() {
        let mut f = MediumFilter::new(BusFilter::CLEAR, 48_000);
        f.set(BusFilter::UNDERWATER);
        let mut blocks = 0;
        let (mut l, mut r) = (vec![0.5f32; 256], vec![-0.5f32; 256]);
        while f.current().cutoff != BusFilter::UNDERWATER.cutoff {
            f.process(&mut l, &mut r);
            blocks += 1;
            assert!(blocks < 100);
        }
        // 5.06 octaves at a quarter a block: 21 blocks, 112 ms; the gain is within 1 dB of
        // its target by then.
        assert_eq!(blocks, 21);
        assert!(f.current().gain < BusFilter::UNDERWATER.gain * 1.12);
        while f.current() != BusFilter::UNDERWATER {
            f.process(&mut l, &mut r);
        }
        f.set(BusFilter::CLEAR);
        while f.current() != BusFilter::CLEAR || f.engaged {
            f.process(&mut l, &mut r);
        }
        // Clear again: it passes the signal unchanged.
        let (mut a, mut b) = (vec![0.25f32; 256], vec![0.125f32; 256]);
        f.process(&mut a, &mut b);
        assert!(a.iter().all(|&x| x == 0.25) && b.iter().all(|&x| x == 0.125));
    }
}
