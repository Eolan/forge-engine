//! Filters: a one-pole low-pass, biquads, and the three-band gain that distance and
//! occlusion use.
//!
//! Air absorption and a wall's transmission are both given as gains in three bands, as
//! Steam Audio gives them (D-011): below 800 Hz, 800 Hz to 8 kHz, above 8 kHz. A
//! [`ThreeBand`] applies them as Steam Audio's EQ effect does (`core/src/core/eq_effect.cpp`,
//! Apache-2.0): the gains are divided by the largest (which becomes an overall gain) and
//! held above −24 dB, then a low shelf at 800 Hz, a peaking filter over 800 Hz–8 kHz and a
//! high shelf at 8 kHz, each a biquad from Robert Bristow-Johnson's cookbook, run in a row.
//! When the gains change, the old and the new filters both run over the block and the output
//! crossfades from one to the other, so a moving source never clicks.

use std::f32::consts::TAU;

/// The edges between the low, mid and high bands, in Hz.
pub const BAND_EDGES: [f32; 2] = [800.0, 8000.0];

/// The lowest band gain relative to the largest: −24 dB (Steam Audio's limit).
const MIN_RELATIVE_GAIN: f32 = 0.0625;

/// A one-pole low-pass: `y += a (x − y)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    /// A low-pass with its −3 dB point near `cutoff` Hz at `rate`.
    pub fn new(cutoff: f32, rate: u32) -> Self {
        Self {
            a: Self::coefficient(cutoff, rate),
            y: 0.0,
        }
    }

    /// The coefficient for `cutoff` at `rate`.
    pub fn coefficient(cutoff: f32, rate: u32) -> f32 {
        1.0 - (-TAU * cutoff / rate as f32).exp()
    }

    /// Changes the cutoff, keeping the state.
    pub fn set_cutoff(&mut self, cutoff: f32, rate: u32) {
        self.a = Self::coefficient(cutoff, rate);
    }

    /// One sample through.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }

    /// One sample through with the coefficient `a` instead of its own (a filter whose
    /// coefficient its owner keeps).
    #[inline]
    pub fn tick_with(&mut self, x: f32, a: f32) -> f32 {
        self.y += a * (x - self.y);
        self.y
    }

    /// Clears the state.
    pub fn reset(&mut self) {
        self.y = 0.0;
    }
}

/// A second-order section in transposed direct form II.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Biquad {
    /// Passes the signal unchanged.
    pub const IDENTITY: Biquad = Biquad {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        z1: 0.0,
        z2: 0.0,
    };

    fn from_coefficients(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b0: (b[0] / a[0]) as f32,
            b1: (b[1] / a[0]) as f32,
            b2: (b[2] / a[0]) as f32,
            a1: (a[1] / a[0]) as f32,
            a2: (a[2] / a[0]) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// A low shelf: `gain` (linear) below `cutoff`, 1 above (the cookbook's, slope set by
    /// `q`).
    pub fn low_shelf(cutoff: f32, q: f32, gain: f32, rate: u32) -> Self {
        let (cw, alpha, a) = shelf_terms(cutoff, q, gain, rate);
        let s = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            [
                a * ((a + 1.0) - (a - 1.0) * cw + s),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                a * ((a + 1.0) - (a - 1.0) * cw - s),
            ],
            [
                (a + 1.0) + (a - 1.0) * cw + s,
                -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                (a + 1.0) + (a - 1.0) * cw - s,
            ],
        )
    }

    /// A high shelf: `gain` above `cutoff`, 1 below.
    pub fn high_shelf(cutoff: f32, q: f32, gain: f32, rate: u32) -> Self {
        let (cw, alpha, a) = shelf_terms(cutoff, q, gain, rate);
        let s = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            [
                a * ((a + 1.0) + (a - 1.0) * cw + s),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                a * ((a + 1.0) + (a - 1.0) * cw - s),
            ],
            [
                (a + 1.0) - (a - 1.0) * cw + s,
                2.0 * ((a - 1.0) - (a + 1.0) * cw),
                (a + 1.0) - (a - 1.0) * cw - s,
            ],
        )
    }

    /// A peaking filter of `gain` between `low` and `high` Hz (centred on their geometric
    /// mean).
    pub fn peaking(low: f32, high: f32, gain: f32, rate: u32) -> Self {
        let centre = (low as f64 * high as f64).sqrt();
        let q_inverse = (high as f64 - low as f64) / centre;
        let w0 = std::f64::consts::TAU * centre / rate as f64;
        let alpha = w0.sin() * q_inverse / 2.0;
        let cw = w0.cos();
        let a = (gain as f64).sqrt();
        Self::from_coefficients(
            [1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a],
        )
    }

    /// One sample through.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Takes another section's state (its coefficients stay).
    pub fn copy_state(&mut self, other: &Biquad) {
        self.z1 = other.z1;
        self.z2 = other.z2;
    }

    /// Clears the state.
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

fn shelf_terms(cutoff: f32, q: f32, gain: f32, rate: u32) -> (f64, f64, f64) {
    let w0 = std::f64::consts::TAU * cutoff as f64 / rate as f64;
    (w0.cos(), w0.sin() / (2.0 * q as f64), (gain as f64).sqrt())
}

/// Gains in three bands applied to one signal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreeBand {
    rate: u32,
    /// The relative gains the current filters were made for.
    shape: [f32; 3],
    filters: [[Biquad; 3]; 2],
    current: usize,
    /// The overall gain at the end of the last block.
    overall: f32,
    first: bool,
}

impl ThreeBand {
    /// A three-band filter at `rate`, edges at [`BAND_EDGES`].
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            shape: [1.0; 3],
            filters: [[Biquad::IDENTITY; 3]; 2],
            current: 0,
            overall: 1.0,
            first: true,
        }
    }

    /// The overall gain and the relative gains (the largest 1, none below −24 dB).
    fn normalize(bands: [f32; 3]) -> (f32, [f32; 3]) {
        let max = bands[0].max(bands[1]).max(bands[2]);
        if max <= f32::MIN_POSITIVE {
            return (0.0, [1.0; 3]);
        }
        (max, bands.map(|b| (b / max).max(MIN_RELATIVE_GAIN)))
    }

    fn design(&self, shape: [f32; 3]) -> [Biquad; 3] {
        let [low, mid, high] = shape;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let section =
            |unity: bool, make: &dyn Fn() -> Biquad| if unity { Biquad::IDENTITY } else { make() };
        [
            section(low == 1.0, &|| {
                Biquad::low_shelf(BAND_EDGES[0], q, low, self.rate)
            }),
            section(mid == 1.0, &|| {
                Biquad::peaking(BAND_EDGES[0], BAND_EDGES[1], mid, self.rate)
            }),
            section(high == 1.0, &|| {
                Biquad::high_shelf(BAND_EDGES[1], q, high, self.rate)
            }),
        ]
    }

    /// Filters `samples` in place for `bands` (the gain in each band). The overall level
    /// ramps from the last block's across this one; a change of shape crossfades.
    pub fn process(&mut self, samples: &mut [f32], bands: [f32; 3]) {
        let (overall, shape) = Self::normalize(bands);
        let from = if self.first { overall } else { self.overall };
        let changed = shape
            .iter()
            .zip(&self.shape)
            .any(|(&a, &b)| (a / b - 1.0).abs() > 0.0116);
        if self.first || changed {
            let previous = self.current;
            let next = 1 - previous;
            // The new filters start from the old ones' state.
            let old = self.filters[previous];
            let designed = self.design(shape);
            for ((f, mut d), old) in self.filters[next].iter_mut().zip(designed).zip(&old) {
                d.copy_state(old);
                *f = d;
            }
            if self.first {
                run(&mut self.filters[next], samples);
            } else {
                let n = samples.len().max(1) as f32;
                for (i, x) in samples.iter_mut().enumerate() {
                    let w = (i + 1) as f32 / n;
                    let old = run_one(&mut self.filters[previous], *x);
                    let new = run_one(&mut self.filters[next], *x);
                    *x = old + (new - old) * w;
                }
            }
            self.current = next;
            self.shape = shape;
            self.first = false;
        } else if shape != [1.0; 3] {
            run(&mut self.filters[self.current], samples);
        }
        // The overall level, ramped.
        let n = samples.len().max(1) as f32;
        if from == overall {
            if overall != 1.0 {
                for x in samples.iter_mut() {
                    *x *= overall;
                }
            }
        } else {
            let step = (overall - from) / n;
            for (i, x) in samples.iter_mut().enumerate() {
                *x *= from + step * (i + 1) as f32;
            }
        }
        self.overall = overall;
    }

    /// Clears the state; the next block starts at its gains without a ramp.
    pub fn reset(&mut self) {
        for set in &mut self.filters {
            for f in set {
                f.reset();
            }
        }
        self.first = true;
    }
}

#[inline]
fn run_one(sections: &mut [Biquad; 3], x: f32) -> f32 {
    let y = sections[0].tick(x);
    let y = sections[1].tick(y);
    sections[2].tick(y)
}

fn run(sections: &mut [Biquad; 3], samples: &mut [f32]) {
    for x in samples.iter_mut() {
        *x = run_one(sections, *x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::sine;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|&v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn band_change_db(frequency: f64, bands: [f32; 3]) -> f32 {
        let rate = 48_000;
        let input = sine(frequency, 0.5, 0.2, rate).samples().to_vec();
        let mut x = input.clone();
        let mut f = ThreeBand::new(rate);
        for chunk in x.chunks_mut(256) {
            f.process(chunk, bands);
        }
        let tail = 2000..input.len();
        20.0 * (rms(&x[tail.clone()]) / rms(&input[tail])).log10()
    }

    #[test]
    fn unit_gains_leave_the_signal_alone() {
        let input = sine(3000.0, 0.5, 0.05, 48_000).samples().to_vec();
        let mut x = input.clone();
        ThreeBand::new(48_000).process(&mut x, [1.0; 3]);
        assert_eq!(x, input);
        // Equal gains are an overall gain.
        let mut z = input.clone();
        ThreeBand::new(48_000).process(&mut z, [0.5; 3]);
        for (a, b) in z.iter().zip(&input) {
            assert!((a - 0.5 * b).abs() < 1e-6);
        }
    }

    #[test]
    fn a_band_gain_cuts_its_band() {
        let muffled = [1.0, 0.5, 0.05];
        assert!(band_change_db(150.0, muffled).abs() < 1.0);
        assert!(
            band_change_db(16_000.0, muffled) < -20.0,
            "{}",
            band_change_db(16_000.0, muffled)
        );
        assert!(
            band_change_db(12_000.0, muffled) < -12.0,
            "{}",
            band_change_db(12_000.0, muffled)
        );
        let mid = band_change_db(2500.0, muffled);
        assert!((mid + 6.0).abs() < 2.0, "{mid}");
    }
}
