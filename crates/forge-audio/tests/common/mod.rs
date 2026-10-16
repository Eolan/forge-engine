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

/// The centres of the octave bands [`octaves`] measures, Hz.
pub const OCTAVES: [f32; 9] = [
    63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16_000.0,
];

/// The energy in each octave band ([`OCTAVES`]), in dB relative to the signal's whole
/// energy: each band through two cookbook band-passes of an octave.s width (Q = √2) in a
/// row, which leak 12 dB an octave outside the band.
pub fn octaves(x: &[f32], rate: u32) -> [f64; 9] {
    let total: f64 = x
        .iter()
        .map(|&v| v as f64 * v as f64)
        .sum::<f64>()
        .max(1e-30);
    OCTAVES.map(|centre| {
        let mut f = forge_audio::dsp::Biquad::band_pass(centre, std::f32::consts::SQRT_2, rate);
        let mut g = f;
        let e: f64 = x
            .iter()
            .map(|&v| {
                let y = g.tick(f.tick(v)) as f64;
                y * y
            })
            .sum();
        10.0 * (e.max(1e-30) / total).log10()
    })
}

/// The spectral centroid of [`octaves`], Hz (the bands' centres weighted by their energy, in
/// the log domain).
pub fn centroid(x: &[f32], rate: u32) -> f64 {
    let bands = octaves(x, rate);
    let (mut w, mut s) = (0.0, 0.0);
    for (db, &f) in bands.iter().zip(&OCTAVES) {
        let e = 10f64.powf(db / 10.0);
        w += e;
        s += e * (f as f64).ln();
    }
    (s / w).exp()
}

/// The root mean square of each window of `window` frames.
pub fn envelope(x: &[f32], window: usize) -> Vec<f64> {
    x.chunks(window).map(rms).collect()
}

/// Seconds from the loudest 5 ms window to the last one within `db` of it.
pub fn decay_time(x: &[f32], rate: u32, db: f64) -> f64 {
    let window = rate as usize / 200;
    let env = envelope(x, window);
    let (top, peak) = env.iter().enumerate().fold(
        (0, 0.0),
        |(i, m), (j, &v)| if v > m { (j, v) } else { (i, m) },
    );
    let floor = peak * 10f64.powf(-db / 20.0);
    let last = env.iter().rposition(|&v| v >= floor).unwrap_or(top);
    (last - top) as f64 * window as f64 / rate as f64
}

/// The energy above `cutoff` Hz relative to the whole, in dB: a fourth-order high-pass
/// (two cookbook sections). A click puts energy up to the Nyquist frequency; the sounds
/// made in code keep the top octave empty, so energy there is a click.
pub fn energy_above_db(x: &[f32], cutoff: f32, rate: u32) -> f64 {
    let mut a = forge_audio::dsp::Biquad::high_pass(cutoff, 0.541_196_1, rate);
    let mut b = forge_audio::dsp::Biquad::high_pass(cutoff, 1.306_563, rate);
    let total: f64 = x
        .iter()
        .map(|&v| v as f64 * v as f64)
        .sum::<f64>()
        .max(1e-30);
    let high: f64 = x
        .iter()
        .map(|&v| {
            let y = b.tick(a.tick(v)) as f64;
            y * y
        })
        .sum();
    10.0 * (high.max(1e-30) / total).log10()
}

/// The largest share of energy above `cutoff` Hz in any 1 ms window that is within 60 dB of
/// the loudest, in dB: a click (a jump in the waveform) puts a window's energy in the top
/// octave, where the sounds made in code have almost none.
pub fn worst_window_above_db(x: &[f32], cutoff: f32, rate: u32) -> f64 {
    let mut a = forge_audio::dsp::Biquad::high_pass(cutoff, 0.541_196_1, rate);
    let mut b = forge_audio::dsp::Biquad::high_pass(cutoff, 1.306_563, rate);
    let high: Vec<f32> = x.iter().map(|&v| b.tick(a.tick(v))).collect();
    let window = rate as usize / 1000;
    let all = envelope(x, window);
    let top = envelope(&high, window);
    let loudest = all.iter().cloned().fold(0.0, f64::max);
    all.iter()
        .zip(&top)
        .filter(|(a, _)| **a > loudest * 1e-3)
        .map(|(a, t)| db(t / a))
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The level between `low` and `high` Hz (fourth-order high- and low-passes), dBFS RMS.
pub fn band_db(x: &[f32], low: f32, high: f32, rate: u32) -> f64 {
    use forge_audio::dsp::Biquad;
    let mut f = [
        Biquad::high_pass(low, 0.541_196_1, rate),
        Biquad::high_pass(low, 1.306_563, rate),
        Biquad::low_pass(high, 0.541_196_1, rate),
        Biquad::low_pass(high, 1.306_563, rate),
    ];
    let y: Vec<f32> = x
        .iter()
        .map(|&v| f.iter_mut().fold(v, |s, b| b.tick(s)))
        .collect();
    db(rms(&y).max(1e-12))
}

/// The correlation of two signals (1: the same, 0: unrelated).
pub fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let ab: f64 = a.iter().zip(b).map(|(&x, &y)| x as f64 * y as f64).sum();
    ab / (rms(a) * rms(b) * a.len() as f64).max(1e-30)
}
