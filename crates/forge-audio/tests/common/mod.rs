//! What the tests measure rendered sound with.

#![allow(dead_code)]

use std::f64::consts::TAU;

/// One channel of interleaved stereo.
pub fn channel(out: &[f32], c: usize) -> Vec<f32> {
    out.as_chunks::<2>().0.iter().map(|f| f[c]).collect()
}

/// Root mean square.
pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// The peak of |x|.
pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
}

/// The amplitude of the sinusoid at `frequency` in `x` (a single-bin DFT). Exact for a
/// whole number of periods.
pub fn tone(x: &[f32], frequency: f64, rate: u32) -> f64 {
    let w = TAU * frequency / rate as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        re += v as f64 * (w * i as f64).cos();
        im -= v as f64 * (w * i as f64).sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

/// The first index whose |value| exceeds `threshold`.
pub fn onset(x: &[f32], threshold: f32) -> Option<usize> {
    x.iter().position(|&v| v.abs() > threshold)
}

/// Decibels of a ratio of levels.
pub fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

/// The frequency of a tone from its upward zero crossings, interpolated.
pub fn frequency(x: &[f32], rate: u32) -> f64 {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        let (a, b) = (x[i - 1], x[i]);
        if a < 0.0 && b >= 0.0 {
            crossings.push(i as f64 - 1.0 + (a / (a - b)) as f64);
        }
    }
    let n = crossings.len();
    assert!(n >= 3, "too few crossings");
    (n - 1) as f64 * rate as f64 / (crossings[n - 1] - crossings[0])
}
