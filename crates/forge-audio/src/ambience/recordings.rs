//! Recorded birds: calls cut out of field recordings, for the birds bed to play.
//!
//! The recordings are not in the repository: `tools/fetch-sounds.sh` fetches them into
//! `assets/sounds/birds/` (CC0 and CC BY from Wikimedia Commons, `assets/sounds.tsv`). When
//! they are there, [`BirdCalls::load`] decodes each, folds it to mono, resamples it to the
//! mixer's rate, high-passes it above the species' song (the wind and the traffic of the
//! recording go), and cuts it into calls where its level stands 12 dB over the recording's
//! floor (the quietest fifth of its 10 ms windows): gaps under 120 ms close, calls under 60 ms
//! go, calls over 3 s are split at their quietest window, each gets 20 ms around it and 15 ms
//! raised-cosine edges and a peak of 0.5. The morning chorus recording stays whole, as the far
//! layer of the dawn. When they are not there, the birds bed makes its calls in code.

use std::path::Path;
use std::sync::Arc;

use crate::decode::{self, DecodeError};
use crate::dsp::Biquad;

/// A bird with recorded calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Species {
    /// The blackbird (*Turdus merula*): fluting phrases of 1.5 to 3.5 kHz.
    Blackbird,
    /// The house sparrow (*Passer domesticus*): its *tschilp* call.
    Sparrow,
    /// The European serin (*Serinus serinus*): a jingling trill of 4 to 8 kHz.
    Serin,
}

impl Species {
    /// The recorded species.
    pub const ALL: [Species; 3] = [Species::Blackbird, Species::Sparrow, Species::Serin];

    /// Its recording's file in `assets/sounds/birds/`.
    pub fn file(self) -> &'static str {
        match self {
            Species::Blackbird => "blackbird.ogg",
            Species::Sparrow => "sparrow.ogg",
            Species::Serin => "serin.ogg",
        }
    }

    /// Below this its recording is the place's noise, not the bird, Hz.
    pub fn low_cut(self) -> f32 {
        match self {
            Species::Blackbird => 1200.0,
            Species::Sparrow => 2000.0,
            Species::Serin => 2500.0,
        }
    }
}

/// One call, mono at the mixer's rate.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedCall {
    /// Whose.
    pub species: Species,
    /// The samples, starting and ending on 0.
    pub samples: Arc<[f32]>,
}

/// The calls cut out of the recordings, and the morning chorus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BirdCalls {
    /// The mixer's rate, frames a second.
    pub rate: u32,
    /// Every call, in the recordings' order.
    pub calls: Vec<RecordedCall>,
    /// The morning chorus, stereo interleaved, high-passed at 250 Hz, −26 dBFS RMS.
    pub chorus: Option<Arc<[f32]>>,
}

/// The chorus recording's file in `assets/sounds/birds/`.
pub const CHORUS_FILE: &str = "chorus.ogg";

impl BirdCalls {
    /// Cuts the recordings found in `dir` (any of them) for a mixer at `rate`. `Ok(None)`
    /// when none is there.
    pub fn load(dir: impl AsRef<Path>, rate: u32) -> Result<Option<BirdCalls>, DecodeError> {
        let dir = dir.as_ref();
        let mut calls = BirdCalls {
            rate,
            ..BirdCalls::default()
        };
        for species in Species::ALL {
            let path = dir.join(species.file());
            if !path.is_file() {
                continue;
            }
            let sound = decode::load(&path)?;
            let mono = resample(&fold(sound.samples(), sound.channels()), sound.rate(), rate);
            for samples in cut(&mono, rate, species.low_cut()) {
                calls.calls.push(RecordedCall {
                    species,
                    samples: samples.into(),
                });
            }
        }
        let chorus = dir.join(CHORUS_FILE);
        if chorus.is_file() {
            let sound = decode::load(&chorus)?;
            let channels = sound.channels() as usize;
            let mut sides: Vec<Vec<f32>> = (0..2)
                .map(|c| {
                    let side: Vec<f32> = sound
                        .samples()
                        .iter()
                        .skip(c.min(channels - 1))
                        .step_by(channels)
                        .copied()
                        .collect();
                    let mut side = resample(&side, sound.rate(), rate);
                    high_pass(&mut side, 250.0, rate);
                    side
                })
                .collect();
            let energy: f64 = sides
                .iter()
                .flatten()
                .map(|&v| v as f64 * v as f64)
                .sum::<f64>()
                / (2 * sides[0].len()).max(1) as f64;
            let scale = (0.05 / energy.sqrt().max(1e-9)) as f32;
            let right = sides.pop().expect("two sides");
            let left = sides.pop().expect("two sides");
            let interleaved: Vec<f32> = left
                .iter()
                .zip(&right)
                .flat_map(|(&l, &r)| [l * scale, r * scale])
                .collect();
            calls.chorus = Some(interleaved.into());
        }
        if calls.calls.is_empty() && calls.chorus.is_none() {
            return Ok(None);
        }
        Ok(Some(calls))
    }

    /// The calls of `species`.
    pub fn of(&self, species: Species) -> impl Iterator<Item = &RecordedCall> {
        self.calls.iter().filter(move |c| c.species == species)
    }
}

/// Interleaved frames folded to mono.
fn fold(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    samples
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// `x` at `from` frames a second read at `to`, by four-point Hermite interpolation.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let step = from as f64 / to as f64;
    let frames = ((x.len() as f64 - 1.0) / step).floor() as usize + 1;
    let at = |i: isize| x[i.clamp(0, x.len() as isize - 1) as usize];
    (0..frames)
        .map(|k| {
            let p = k as f64 * step;
            let i = p.floor() as isize;
            let t = (p - i as f64) as f32;
            let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let c = (x1 - xm1) * 0.5;
            let v = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
            let w = (x2 - xm1) * 0.5 + 1.5 * (x0 - x1);
            ((w * t + v) * t + c) * t + x0
        })
        .collect()
}

/// A fourth-order Butterworth high-pass at `cutoff`, in place.
fn high_pass(x: &mut [f32], cutoff: f32, rate: u32) {
    let mut a = Biquad::high_pass(cutoff, 0.541_196_1, rate);
    let mut b = Biquad::high_pass(cutoff, 1.306_563, rate);
    for v in x.iter_mut() {
        *v = b.tick(a.tick(*v));
    }
}

/// Cuts a mono recording into calls (see the module's notes).
pub fn cut(mono: &[f32], rate: u32, low_cut: f32) -> Vec<Vec<f32>> {
    let mut x = mono.to_vec();
    high_pass(&mut x, low_cut, rate);
    let window = (rate / 100) as usize;
    let env: Vec<f32> = x
        .chunks(window)
        .map(|w| (w.iter().map(|&v| v * v).sum::<f32>() / w.len() as f32).sqrt())
        .collect();
    if env.is_empty() {
        return Vec::new();
    }
    let mut sorted = env.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[sorted.len() / 5].max(1e-6);
    let top = sorted[sorted.len() - 1];
    let threshold = (floor * 10f32.powf(12.0 / 20.0)).max(top * 10f32.powf(-30.0 / 20.0));
    // Runs of windows over the threshold, gaps under 120 ms closed.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, &e) in env.iter().enumerate() {
        if e < threshold {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if i - *end <= 12 => *end = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    // Calls over 3 s split at their quietest window in their third second.
    let mut pieces = Vec::new();
    for (mut start, end) in runs {
        while end - start > 300 {
            let cut_at = (start + 200..start + 300)
                .min_by(|&a, &b| env[a].total_cmp(&env[b]))
                .expect("a window");
            pieces.push((start, cut_at));
            start = cut_at;
        }
        pieces.push((start, end));
    }
    let pad = window * 2;
    let edge = (rate as f32 * 0.015) as usize;
    pieces
        .into_iter()
        .filter(|&(s, e)| {
            e - s >= 6
                && env[s..e].iter().cloned().fold(0.0, f32::max) >= floor * 10f32.powf(15.0 / 20.0)
        })
        .map(|(s, e)| {
            let from = (s * window).saturating_sub(pad);
            let to = (e * window + pad).min(x.len());
            let mut call = x[from..to].to_vec();
            let n = call.len();
            let edge = edge.min(n / 2);
            for i in 0..edge {
                let w = 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / edge as f32).cos();
                call[i] *= w;
                call[n - 1 - i] *= w;
            }
            let peak = call.iter().fold(0.0f32, |m, &v| m.max(v.abs())).max(1e-9);
            for v in &mut call {
                *v *= 0.5 / peak;
            }
            call
        })
        .collect()
}
