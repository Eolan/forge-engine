//! What the sounds made in code are built from: modal resonators, pink noise, decaying
//! sinusoid grains (a pebble's tick, a bubble's ring), force pulses.
//!
//! These run on the game thread (footsteps and impacts, synthesised when they happen) and on
//! the decode thread (the ambience beds), never in the mixer.

use std::f32::consts::{PI, TAU};

use forge_core::SplitMix64;

use super::Biquad;

/// A mode: a two-pole resonator whose impulse response is `gain · e^(−decay t) · sin(2π f t)`
/// (the modal synthesis of van den Doel, Kry and Pai, 2001: one resonator a mode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resonator {
    b: f32,
    a1: f32,
    a2: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    /// A mode of `frequency` Hz whose amplitude falls by `e` every `1 / decay` seconds (its
    /// 60 dB decay time is `6.9 / decay`), its impulse response peaking near `gain`.
    pub fn new(frequency: f32, decay: f32, gain: f32, rate: u32) -> Self {
        let w = TAU * frequency.min(0.49 * rate as f32) / rate as f32;
        let r = (-decay / rate as f32).exp();
        Self {
            b: gain * w.sin(),
            a1: -2.0 * r * w.cos(),
            a2: r * r,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// One sample through.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b * x - self.a1 * self.y1 - self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Pink noise (−3 dB an octave) from white: Paul Kellet's three-pole filter, within 0.5 dB
/// of the ideal slope above 40 Hz at 44.1 and 48 kHz (public domain, 1999).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PinkNoise {
    b: [f32; 3],
}

impl PinkNoise {
    /// The next sample for white input `white` (uniform in ±1 gives about ±1).
    #[inline]
    pub fn tick(&mut self, white: f32) -> f32 {
        self.b[0] = 0.99765 * self.b[0] + white * 0.099_046;
        self.b[1] = 0.96300 * self.b[1] + white * 0.296_516_4;
        self.b[2] = 0.57000 * self.b[2] + white * 1.052_691_3;
        (self.b[0] + self.b[1] + self.b[2] + white * 0.1848) * 0.25
    }
}

/// White noise in ±1.
#[inline]
pub fn white(rng: &mut SplitMix64) -> f32 {
    rng.range_f32(-1.0, 1.0)
}

/// A value spread evenly in the log domain between `low` and `high`.
pub fn log_uniform(rng: &mut SplitMix64, low: f32, high: f32) -> f32 {
    (low.ln() + (high.ln() - low.ln()) * rng.next_f32()).exp()
}

/// The wait to the next event of a Poisson process of `rate` events a second.
pub fn poisson_wait(rng: &mut SplitMix64, rate: f32) -> f32 {
    if rate <= 0.0 {
        return f32::INFINITY;
    }
    -(1.0 - rng.next_f32()).max(1e-7).ln() / rate
}

/// A contact's force: `sin²(π t / width)` over `width` seconds, 0 outside. It starts and ends
/// at 0 with a zero slope, so what it drives starts without a click; its spectrum falls past
/// `2 / width` Hz, so a short contact (a heel on stone) is bright and a long one (a foot
/// sinking into grass) dull.
#[inline]
pub fn force_pulse(t: f32, width: f32) -> f32 {
    if t <= 0.0 || t >= width {
        0.0
    } else {
        let s = (PI * t / width).sin();
        s * s
    }
}

/// A decaying sinusoid: a pebble's tick, a stalk snapping, a bubble ringing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grain {
    /// Hz at its start.
    pub frequency: f32,
    /// How fast the frequency rises, as a share of it a second (a bubble's rising pitch).
    pub chirp: f32,
    /// Amplitude falls by `e` every `1 / decay` seconds.
    pub decay: f32,
    /// Peak amplitude.
    pub amplitude: f32,
    /// Stereo position, −1 left to 1 right (equal power).
    pub pan: f32,
}

/// Frames a grain's attack ramps over (0.25 ms at 48 kHz): enough that it adds nothing above
/// 16 kHz, short enough that a tick stays a tick.
pub const GRAIN_ATTACK: usize = 12;

impl Grain {
    /// Frames until it has fallen 80 dB.
    pub fn frames(&self, rate: u32) -> usize {
        ((9.2 / self.decay.max(1.0)) * rate as f32).ceil() as usize + GRAIN_ATTACK
    }

    /// The bubble of `radius` metres (Minnaert's resonance, `f₀ ≈ 3.26 / r` Hz for air in
    /// water near the surface), its damping and rising pitch after van den Doel's model of
    /// liquid sounds (2005): `d = 0.13 f₀ + 0.0072 f₀^1.5`, the frequency rising by `0.1 d`
    /// of itself a second as the bubble nears the surface.
    pub fn bubble(radius: f32, amplitude: f32, pan: f32) -> Self {
        let f0 = 3.26 / radius.max(1e-4);
        let decay = 0.13 * f0 + 0.0072 * f0 * f0.sqrt();
        Self {
            frequency: f0,
            chirp: 0.1 * decay,
            decay,
            amplitude,
            pan,
        }
    }

    /// Adds the grain into `out` (mono) from frame `at`, as far as `out` reaches.
    pub fn add_mono(&self, out: &mut [f32], at: usize, rate: u32) {
        let n = self.frames(rate).min(out.len().saturating_sub(at));
        let mut osc = Oscillator::new(self, rate);
        for (i, x) in out[at..at + n].iter_mut().enumerate() {
            *x += osc.tick(i);
        }
    }
}

/// A grain played a sample at a time: a phasor turned and shrunk by one complex multiply a
/// sample (no sine in the loop), its step retuned every 16 samples when it chirps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oscillator {
    re: f32,
    im: f32,
    step_re: f32,
    step_im: f32,
    fall: f32,
    frequency: f32,
    chirp: f32,
    rate: f32,
    amplitude: f32,
}

impl Oscillator {
    /// The grain `grain` from its first sample.
    pub fn new(grain: &Grain, rate: u32) -> Self {
        let mut osc = Self {
            re: 1.0,
            im: 0.0,
            step_re: 1.0,
            step_im: 0.0,
            fall: (-grain.decay / rate as f32).exp(),
            frequency: grain.frequency,
            chirp: grain.chirp,
            rate: rate as f32,
            amplitude: grain.amplitude,
        };
        osc.retune(0);
        osc
    }

    fn retune(&mut self, i: usize) {
        let t = i as f32 / self.rate;
        let f = (self.frequency * (1.0 + self.chirp * t)).min(0.45 * self.rate);
        let w = TAU * f / self.rate;
        self.step_re = self.fall * w.cos();
        self.step_im = self.fall * w.sin();
    }

    /// Sample `i` of the grain (called with 0, 1, 2 and on).
    #[inline]
    pub fn tick(&mut self, i: usize) -> f32 {
        if self.chirp != 0.0 && i.is_multiple_of(16) && i > 0 {
            self.retune(i);
        }
        let ramp = if i < GRAIN_ATTACK {
            i as f32 / GRAIN_ATTACK as f32
        } else {
            1.0
        };
        let y = self.amplitude * ramp * self.im;
        let re = self.re * self.step_re - self.im * self.step_im;
        self.im = self.re * self.step_im + self.im * self.step_re;
        self.re = re;
        y
    }
}

/// A fourth-order Butterworth low-pass (two sections): what keeps the sounds made here clear
/// of the top octave, where a click would show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LowPass4 {
    sections: [Biquad; 2],
}

impl LowPass4 {
    /// Cut off at `cutoff` Hz.
    pub fn new(cutoff: f32, rate: u32) -> Self {
        Self {
            sections: [
                Biquad::low_pass(cutoff, 0.541_196_1, rate),
                Biquad::low_pass(cutoff, 1.306_563, rate),
            ],
        }
    }

    /// Moves the cutoff, keeping the state.
    pub fn retune(&mut self, cutoff: f32, rate: u32) {
        let next = Self::new(cutoff, rate);
        for (s, n) in self.sections.iter_mut().zip(&next.sections) {
            s.retune(n);
        }
    }

    /// One sample through.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.sections[0].tick(x);
        self.sections[1].tick(y)
    }

    /// A buffer through, in place.
    pub fn run(&mut self, samples: &mut [f32]) {
        for x in samples {
            *x = self.tick(*x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resonator_rings_at_its_frequency_and_decays() {
        let rate = 48_000;
        let mut r = Resonator::new(1000.0, 50.0, 1.0, rate);
        let mut out = vec![0.0f32; rate as usize / 10];
        out[0] = r.tick(1.0);
        for x in out.iter_mut().skip(1) {
            *x = r.tick(0.0);
        }
        // The peak of the first period is near the gain.
        let first = out[..48].iter().fold(0f32, |m, &v| m.max(v.abs()));
        assert!((first - 1.0).abs() < 0.02, "{first}");
        // After 20 ms the envelope is e^(−1).
        let later = out[960..1008].iter().fold(0f32, |m, &v| m.max(v.abs()));
        assert!((later / first - (-1.0f32).exp()).abs() < 0.02, "{later}");
    }

    #[test]
    fn a_bubble_rings_where_minnaert_says() {
        let g = Grain::bubble(0.003, 1.0, 0.0);
        assert!((g.frequency - 1086.7).abs() < 1.0);
        // 0.13 f + 0.0072 f^1.5 at 1087 Hz.
        assert!((g.decay - 399.1).abs() < 1.0, "{}", g.decay);
    }
}
