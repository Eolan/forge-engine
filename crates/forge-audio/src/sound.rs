//! Sounds held in memory, and the test sounds made in code.
//!
//! A [`SampleBuffer`] is decoded audio: interleaved `f32` frames at their own rate, shared by
//! every voice that plays it through an `Arc`. The generators below make the sounds the tests
//! and the headless scenes use (sines, seeded noise, clicks), so nothing has to be
//! downloaded to check the mixer.

use std::f64::consts::TAU;
use std::sync::Arc;

use forge_core::SplitMix64;

/// Decoded audio in memory: interleaved frames at a rate.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleBuffer {
    rate: u32,
    channels: u16,
    data: Box<[f32]>,
}

impl SampleBuffer {
    /// Wraps interleaved samples. `data`'s length must be a whole number of frames.
    pub fn new(rate: u32, channels: u16, data: Vec<f32>) -> Self {
        assert!(rate > 0, "a sound needs a rate");
        assert!(channels == 1 || channels == 2, "sounds are mono or stereo");
        assert_eq!(data.len() % channels as usize, 0, "a partial frame");
        Self {
            rate,
            channels,
            data: data.into_boxed_slice(),
        }
    }

    /// A mono sound.
    pub fn mono(rate: u32, data: Vec<f32>) -> Self {
        Self::new(rate, 1, data)
    }

    /// The same sound, shared.
    pub fn shared(self) -> Arc<SampleBuffer> {
        Arc::new(self)
    }

    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// 1 or 2.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Frames.
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels as usize
    }

    /// Seconds.
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }

    /// The interleaved samples.
    pub fn samples(&self) -> &[f32] {
        &self.data
    }

    /// Sample `channel` of frame `frame`.
    #[inline]
    pub fn sample(&self, frame: usize, channel: usize) -> f32 {
        self.data[frame * self.channels as usize + channel]
    }
}

/// A sine of `frequency` Hz and peak `amplitude`, `seconds` long, mono.
pub fn sine(frequency: f64, amplitude: f32, seconds: f64, rate: u32) -> SampleBuffer {
    let frames = (seconds * rate as f64).round() as usize;
    let step = TAU * frequency / rate as f64;
    let data = (0..frames)
        .map(|i| amplitude * (step * i as f64).sin() as f32)
        .collect();
    SampleBuffer::mono(rate, data)
}

/// A sine that loops without a seam: `cycles` whole periods in about `seconds`, the
/// frequency nudged to fit (the returned buffer's `frames / cycles` frames a period).
pub fn sine_loop(frequency: f64, amplitude: f32, seconds: f64, rate: u32) -> SampleBuffer {
    let frames = (seconds * rate as f64).round().max(1.0);
    let cycles = (frequency * frames / rate as f64).round().max(1.0);
    let step = TAU * cycles / frames;
    let data = (0..frames as usize)
        .map(|i| amplitude * (step * i as f64).sin() as f32)
        .collect();
    SampleBuffer::mono(rate, data)
}

/// White noise, uniform in ±`amplitude`, from a seed: the same seed gives the same samples.
pub fn noise(seed: u64, amplitude: f32, seconds: f64, rate: u32) -> SampleBuffer {
    let frames = (seconds * rate as f64).round() as usize;
    let mut rng = SplitMix64::new(seed);
    let data = (0..frames)
        .map(|_| rng.range_f32(-amplitude, amplitude))
        .collect();
    SampleBuffer::mono(rate, data)
}

/// Clicks: a single sample of `amplitude` every `period` frames from frame 0, `frames` long.
pub fn clicks(period: usize, amplitude: f32, frames: usize, rate: u32) -> SampleBuffer {
    assert!(period > 0);
    let data = (0..frames)
        .map(|i| if i % period == 0 { amplitude } else { 0.0 })
        .collect();
    SampleBuffer::mono(rate, data)
}

/// One sample of `amplitude` and nothing else: the impulse the scheduling and reverb tests
/// listen for.
pub fn impulse(amplitude: f32, rate: u32) -> SampleBuffer {
    SampleBuffer::mono(rate, vec![amplitude])
}

/// A stereo sound from two mono ones of the same rate and length.
pub fn stereo(left: &SampleBuffer, right: &SampleBuffer) -> SampleBuffer {
    assert_eq!(left.rate(), right.rate());
    assert_eq!(left.frames(), right.frames());
    assert!(left.channels() == 1 && right.channels() == 1);
    let data = left
        .samples()
        .iter()
        .zip(right.samples())
        .flat_map(|(&l, &r)| [l, r])
        .collect();
    SampleBuffer::new(left.rate(), 2, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generators_have_their_lengths_and_levels() {
        let s = sine(1000.0, 0.5, 0.5, 48_000);
        assert_eq!(s.frames(), 24_000);
        let peak = s.samples().iter().fold(0f32, |m, &x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 1e-3);
        let n = noise(7, 1.0, 1.0, 48_000);
        assert_eq!(n, noise(7, 1.0, 1.0, 48_000));
        assert_ne!(n, noise(8, 1.0, 1.0, 48_000));
        let rms =
            (n.samples().iter().map(|&x| (x * x) as f64).sum::<f64>() / n.frames() as f64).sqrt();
        // Uniform noise in ±1 has an RMS of 1/√3.
        assert!((rms - 1.0 / 3f64.sqrt()).abs() < 0.01, "{rms}");
        let l = sine_loop(440.0, 1.0, 0.1, 48_000);
        // The last sample leads into the first as any other pair does.
        let d_wrap = l.samples()[0] - l.samples()[l.frames() - 1];
        let d_first = l.samples()[1] - l.samples()[0];
        assert!((d_wrap - d_first).abs() < 1e-3);
    }
}
