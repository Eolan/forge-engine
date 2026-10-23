//! The beds: endless textures made in code, each a [`Decoder`] the stream pump runs on the
//! decode thread (or before each block offline), so nothing repeats and the mixer only reads
//! a ring.
//!
//! Each bed reads one number, its intensity (the wind's speed, the rain's rate, how many
//! cicadas sing, how often birds call, the swell), from a [`BedControl`] the game thread
//! writes; the bed eases toward it over half a second. A bed asleep writes silence without
//! working; waking, it fades in over 50 ms. Everything a bed makes passes an eighth-order
//! low-pass at 10.5 kHz, so its top octave stays empty.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use forge_core::SplitMix64;

use super::AmbienceAssets;
use super::recordings::{BirdCalls, CicadaSong, Species};
use crate::decode::{DecodeError, Decoder};
use crate::dsp::synth::{
    Grain, LowPass4, Oscillator, PinkNoise, Resonator, log_uniform, poisson_wait, white,
};
use crate::dsp::{Biquad, OnePole};
use crate::foley::smoothstep;
use crate::spatial::pan_gains;
use crate::variation::pick_avoiding;

/// Frames a bed renders with one intensity.
const SUB: usize = 64;

/// Seconds the intensity takes to move by `e`.
const EASE: f32 = 0.5;

/// Seconds a bed takes to fade in when it wakes, or out when it sleeps.
const PRESENCE: f32 = 0.05;

/// The cutoff of the eighth-order low-pass every bed ends with, Hz: 33 dB down at 16 kHz.
const TOP: f32 = 10_500.0;

/// What the game thread tells a bed: its intensity and whether it should work.
#[derive(Debug)]
pub struct BedControl {
    intensity: AtomicU32,
    awake: AtomicBool,
}

impl BedControl {
    fn new(intensity: f32) -> Self {
        Self {
            intensity: AtomicU32::new(intensity.to_bits()),
            awake: AtomicBool::new(true),
        }
    }

    /// Sets the intensity.
    pub fn set_intensity(&self, intensity: f32) {
        self.intensity.store(intensity.to_bits(), Ordering::Relaxed);
    }

    /// The intensity.
    pub fn intensity(&self) -> f32 {
        f32::from_bits(self.intensity.load(Ordering::Relaxed))
    }

    /// Wakes the bed or puts it to sleep.
    pub fn set_awake(&self, awake: bool) {
        self.awake.store(awake, Ordering::Relaxed);
    }

    /// Whether it works.
    pub fn awake(&self) -> bool {
        self.awake.load(Ordering::Relaxed)
    }
}

/// A sound that goes on: what a bed renders.
pub trait Texture: Send {
    /// Adds `left.len()` frames for `intensity` into `left` and `right`.
    fn render(&mut self, intensity: f32, left: &mut [f32], right: &mut [f32]);
}

/// The beds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BedKind {
    /// Wind by its speed (m/s): a gusting roar, leaves rustling, wires whistling past 6 m/s.
    Wind,
    /// Rain by its rate (mm/h): drops on hard ground and in puddles, the wash of far ones.
    Rain,
    /// Cicadas (Cicada orni) by how many sing, 0 to 1.
    Cicadas,
    /// Birds by how often they call, 0 to 1: blackbirds, sparrows, serins, collared doves.
    Birds,
    /// The sea against the quays by its swell, 0 to 1.
    Sea,
    /// Under water: a rumble and bubbles.
    Underwater,
}

impl BedKind {
    /// Every bed.
    pub const ALL: [BedKind; 6] = [
        BedKind::Wind,
        BedKind::Rain,
        BedKind::Cicadas,
        BedKind::Birds,
        BedKind::Sea,
        BedKind::Underwater,
    ];

    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            BedKind::Wind => "wind",
            BedKind::Rain => "rain",
            BedKind::Cicadas => "cicadas",
            BedKind::Birds => "birds",
            BedKind::Sea => "sea",
            BedKind::Underwater => "underwater",
        }
    }

    /// Its texture, from `seed`.
    pub fn texture(self, seed: u64, rate: u32) -> Box<dyn Texture> {
        self.texture_with(seed, rate, &AmbienceAssets::default())
    }

    /// Its texture, from `seed`, using `assets` when they have what it plays.
    pub fn texture_with(self, seed: u64, rate: u32, assets: &AmbienceAssets) -> Box<dyn Texture> {
        if let (BedKind::Birds, Some(recorded)) = (self, &assets.birds) {
            return Box::new(
                Birds::with_recordings(seed, rate, recorded.clone()).with_mix(assets.bird_mix),
            );
        }
        if let (BedKind::Cicadas, Some(song)) = (self, &assets.cicadas) {
            return Box::new(RecordedCicadas::new(seed, rate, song.clone()));
        }
        match self {
            BedKind::Wind => Box::new(Wind::new(seed, rate)),
            BedKind::Rain => Box::new(Rain::new(seed, rate)),
            BedKind::Cicadas => Box::new(Cicadas::new(seed, rate)),
            BedKind::Birds => Box::new(Birds::new(seed, rate).with_mix(assets.bird_mix)),
            BedKind::Sea => Box::new(Sea::new(seed, rate)),
            BedKind::Underwater => Box::new(Underwater::new(seed, rate)),
        }
    }
}

/// A bed as a stream's decoder: stereo, endless.
pub struct Bed {
    texture: Box<dyn Texture>,
    control: Arc<BedControl>,
    rate: u32,
    intensity: Option<f32>,
    presence: f32,
    left: [f32; SUB],
    right: [f32; SUB],
    low_pass: [[LowPass4; 2]; 2],
}

impl Bed {
    /// The bed `kind` from `seed` at `intensity`, and the control the game keeps.
    pub fn new(kind: BedKind, seed: u64, rate: u32, intensity: f32) -> (Bed, Arc<BedControl>) {
        Self::new_with(kind, seed, rate, intensity, &AmbienceAssets::default())
    }

    /// The bed `kind`, using `assets` when they have what it plays.
    pub fn new_with(
        kind: BedKind,
        seed: u64,
        rate: u32,
        intensity: f32,
        assets: &AmbienceAssets,
    ) -> (Bed, Arc<BedControl>) {
        let control = Arc::new(BedControl::new(intensity));
        let bed = Bed {
            texture: kind.texture_with(seed, rate, assets),
            control: control.clone(),
            rate,
            intensity: None,
            presence: 1.0,
            left: [0.0; SUB],
            right: [0.0; SUB],
            low_pass: [[LowPass4::new(TOP, rate); 2]; 2],
        };
        (bed, control)
    }
}

impl Decoder for Bed {
    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn frames(&self) -> Option<u64> {
        None
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        let frames = out.len() / 2;
        let ease = 1.0 - (-(SUB as f32) / (EASE * self.rate as f32)).exp();
        let fade = 1.0 / (PRESENCE * self.rate as f32);
        for chunk in out[..frames * 2].chunks_mut(SUB * 2) {
            let n = chunk.len() / 2;
            let awake = self.control.awake();
            if !awake && self.presence == 0.0 {
                chunk.fill(0.0);
                continue;
            }
            let target = self.control.intensity();
            let intensity = match self.intensity {
                None => target,
                Some(i) => i + (target - i) * ease,
            };
            self.intensity = Some(intensity);
            let (left, right) = (&mut self.left[..n], &mut self.right[..n]);
            left.fill(0.0);
            right.fill(0.0);
            self.texture.render(intensity, left, right);
            let goal = if awake { 1.0 } else { 0.0 };
            for (i, frame) in chunk.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                self.presence += (goal - self.presence).clamp(-fade, fade);
                for (c, x) in [left[i], right[i]].into_iter().enumerate() {
                    let [a, b] = &mut self.low_pass[c];
                    frame[c] = b.tick(a.tick(x)) * self.presence;
                }
            }
        }
        Ok(frames)
    }

    fn rewind(&mut self) -> Result<(), DecodeError> {
        Ok(())
    }
}

/// A Gaussian sample of unit variance (four uniforms, Irwin–Hall).
fn gauss(rng: &mut SplitMix64) -> f32 {
    (rng.next_f32() + rng.next_f32() + rng.next_f32() + rng.next_f32() - 2.0) * 3f32.sqrt()
}

/// A slow random wander around 0 (an Ornstein–Uhlenbeck process): its time constant and its
/// spread, stepped every `step` seconds.
#[derive(Clone, Copy, Debug)]
struct Wander {
    x: f32,
    keep: f32,
    kick: f32,
}

impl Wander {
    fn new(tau: f32, spread: f32, step: f32) -> Self {
        let keep = (-step / tau).exp();
        Self {
            x: 0.0,
            keep,
            kick: spread * (1.0 - keep * keep).sqrt(),
        }
    }

    fn step(&mut self, rng: &mut SplitMix64) -> f32 {
        self.x = self.x * self.keep + self.kick * gauss(rng);
        self.x
    }
}

/// Grains playing: drops, bubbles, ticks, each with its own place in the stereo field.
struct GrainPool {
    rate: u32,
    grains: Vec<(Oscillator, usize, usize, [f32; 2])>,
}

impl GrainPool {
    fn new(rate: u32) -> Self {
        Self {
            rate,
            grains: Vec::with_capacity(256),
        }
    }

    fn spawn(&mut self, grain: Grain) {
        if self.grains.len() < 1024 {
            let gains = pan_gains(grain.pan);
            let len = grain.frames(self.rate);
            self.grains
                .push((Oscillator::new(&grain, self.rate), 0, len, gains));
        }
    }

    fn tick(&mut self) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for (osc, age, _, gains) in &mut self.grains {
            let y = osc.tick(*age);
            *age += 1;
            out[0] += y * gains[0];
            out[1] += y * gains[1];
        }
        self.grains.retain(|g| g.1 < g.2);
        out
    }
}

/// Wind: pink noise through a band-pass whose centre and level follow the gusts (a slow
/// wander and a fast one per ear), leaves rustling in the village's trees above 1.5 m/s,
/// a wire or a branch whistling past 6 m/s (a Strouhal tone, `0.2 v / d` for a 5 mm wire).
pub struct Wind {
    rng: SplitMix64,
    rate: u32,
    slow: Wander,
    fast: [Wander; 2],
    pink: [PinkNoise; 2],
    body: [Biquad; 2],
    whistle: [Biquad; 2],
    rustle: [Biquad; 2],
    amps: [[f32; 3]; 2],
}

impl Wind {
    /// From `seed`.
    pub fn new(seed: u64, rate: u32) -> Self {
        let step = SUB as f32 / rate as f32;
        Self {
            rng: SplitMix64::new(seed),
            rate,
            slow: Wander::new(3.0, 0.3, step),
            fast: [Wander::new(0.5, 0.12, step); 2],
            pink: [PinkNoise::default(); 2],
            body: [Biquad::band_pass(500.0, 0.8, rate); 2],
            whistle: [Biquad::band_pass(400.0, 15.0, rate); 2],
            rustle: [Biquad::high_pass(2500.0, 0.7, rate); 2],
            amps: [[0.0; 3]; 2],
        }
    }
}

impl Texture for Wind {
    fn render(&mut self, speed: f32, left: &mut [f32], right: &mut [f32]) {
        let slow = self.slow.step(&mut self.rng);
        let n = left.len() as f32;
        for (c, out) in [left, right].into_iter().enumerate() {
            let fast = self.fast[c].step(&mut self.rng);
            let v = (speed.max(0.0) * (1.0 + slow + fast)).max(0.0);
            self.body[c].retune(&Biquad::band_pass(120.0 + 45.0 * v, 0.8, self.rate));
            self.whistle[c].retune(&Biquad::band_pass(40.0 * v.max(1.0), 15.0, self.rate));
            let to = [
                0.9 * (v / 10.0).powf(1.5).min(3.0),
                0.5 * smoothstep(6.0, 14.0, v) * (v / 10.0),
                0.05 * smoothstep(1.5, 6.0, v) * (v / 8.0).min(2.0),
            ];
            let from = self.amps[c];
            self.amps[c] = to;
            for (i, x) in out.iter_mut().enumerate() {
                let w = (i + 1) as f32 / n;
                let a = |k: usize| from[k] + (to[k] - from[k]) * w;
                let p = self.pink[c].tick(white(&mut self.rng));
                *x += a(0) * self.body[c].tick(p)
                    + a(1) * self.whistle[c].tick(white(&mut self.rng))
                    + a(2) * self.rustle[c].tick(white(&mut self.rng));
            }
        }
    }
}

/// Rain: drops as a Poisson process of 40 a second for each mm/h, seven in ten on hard
/// ground (a tick of 2 to 8 kHz dying in a quarter of a millisecond), three in ten in
/// puddles (a bubble of 0.8 to 2.5 mm: Minnaert's 1.3 to 4 kHz), after Verron and Drettakis'
/// atoms and Liu, Cheng and Tong's statistical rain; and the wash of the far drops, pink
/// noise between 500 Hz and 7 kHz, its level the square root of the rate.
pub struct Rain {
    rng: SplitMix64,
    rate: u32,
    pool: GrainPool,
    until: f32,
    wash: [PinkNoise; 2],
    high_pass: [Biquad; 2],
    low_pass: [Biquad; 2],
}

impl Rain {
    /// From `seed`.
    pub fn new(seed: u64, rate: u32) -> Self {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        Self {
            rng: SplitMix64::new(seed),
            rate,
            pool: GrainPool::new(rate),
            until: 0.0,
            wash: [PinkNoise::default(); 2],
            high_pass: [Biquad::high_pass(500.0, q, rate); 2],
            low_pass: [Biquad::low_pass(7000.0, q, rate); 2],
        }
    }
}

impl Texture for Rain {
    fn render(&mut self, mm_h: f32, left: &mut [f32], right: &mut [f32]) {
        let mm_h = mm_h.max(0.0);
        let drops = (40.0 * mm_h).min(4000.0);
        let wash = 0.25 * (mm_h / 10.0).sqrt().min(2.0);
        let dt = 1.0 / self.rate as f32;
        for i in 0..left.len() {
            self.until -= dt;
            while self.until <= 0.0 && drops > 0.0 {
                self.until += poisson_wait(&mut self.rng, drops);
                let pan = self.rng.range_f32(-1.0, 1.0);
                let grain = if self.rng.next_f32() < 0.7 {
                    Grain {
                        frequency: log_uniform(&mut self.rng, 2000.0, 8000.0),
                        chirp: 0.0,
                        decay: self.rng.range_f32(3000.0, 5000.0),
                        amplitude: log_uniform(&mut self.rng, 0.02, 0.3),
                        pan,
                    }
                } else {
                    let radius = log_uniform(&mut self.rng, 0.0008, 0.0025);
                    Grain::bubble(radius, log_uniform(&mut self.rng, 0.02, 0.2), pan)
                };
                self.pool.spawn(grain);
            }
            if drops <= 0.0 {
                self.until = 0.0;
            }
            let [l, r] = self.pool.tick();
            let mut wash_of = |c: usize| {
                let p = self.wash[c].tick(white(&mut self.rng));
                wash * self.low_pass[c].tick(self.high_pass[c].tick(p))
            };
            left[i] += l + wash_of(0);
            right[i] += r + wash_of(1);
        }
    }
}

/// One cicada: a train of tymbal pulses (180 to 240 a second) ringing a resonance near
/// 4.8 kHz and a weaker one 1.6 times higher, cut into echemes (Cicada orni: 80 ± 30 ms of song every
/// 230 ± 70 ms, peak at 4.82 ± 0.49 kHz; Pinto-Juma et al., Zoological Studies 2005), sung in
/// bouts of 8 to 40 s with pauses of 1 to 8 s.
struct Singer {
    threshold: f32,
    gains: [f32; 2],
    pulse_rate: f32,
    phase: f32,
    resonators: [Resonator; 2],
    echeme: f32,
    gap_mean: f32,
    gap: f32,
    /// Seconds into the current echeme and gap.
    t: f32,
    length: f32,
    pause: f32,
    bout: f32,
    presence: f32,
}

/// A chorus of up to 14 cicadas; the intensity (0 to 1) sets how many sing.
pub struct Cicadas {
    rng: SplitMix64,
    rate: u32,
    singers: Vec<Singer>,
}

impl Cicadas {
    /// From `seed`.
    pub fn new(seed: u64, rate: u32) -> Self {
        let mut rng = SplitMix64::new(seed);
        let count = 14;
        let singers = (0..count)
            .map(|k| {
                let carrier = (4820.0 + 490.0 * gauss(&mut rng)).clamp(3900.0, 6500.0);
                // The first three to sing are near, their rhythm clear over the far chorus.
                let level = if k < 3 {
                    rng.range_f32(0.6, 1.0)
                } else {
                    log_uniform(&mut rng, 0.05, 0.3)
                };
                let echeme = rng.range_f32(0.06, 0.1);
                let gap = rng.range_f32(0.1, 0.2);
                Singer {
                    threshold: 0.9 * k as f32 / count as f32,
                    gains: pan_gains(rng.range_f32(-0.9, 0.9)),
                    pulse_rate: rng.range_f32(180.0, 240.0),
                    phase: rng.next_f32(),
                    resonators: [
                        Resonator::new(carrier, 4500.0, level, rate),
                        Resonator::new((1.6 * carrier).min(9000.0), 9000.0, 0.35 * level, rate),
                    ],
                    echeme,
                    gap_mean: gap,
                    gap,
                    t: rng.range_f32(0.0, echeme + gap),
                    length: echeme,
                    pause: 0.0,
                    bout: rng.range_f32(8.0, 40.0),
                    presence: 0.0,
                }
            })
            .collect();
        Self { rng, rate, singers }
    }
}

impl Texture for Cicadas {
    fn render(&mut self, activity: f32, left: &mut [f32], right: &mut [f32]) {
        let dt = 1.0 / self.rate as f32;
        let n = left.len();
        let block = n as f32 * dt;
        for s in &mut self.singers {
            // Bouts and pauses.
            if s.pause > 0.0 {
                s.pause -= block;
            } else {
                s.bout -= block;
                if s.bout <= 0.0 {
                    s.pause = self.rng.range_f32(1.0, 8.0);
                    s.bout = self.rng.range_f32(8.0, 40.0);
                }
            }
            let sings = activity > s.threshold && s.pause <= 0.0;
            let goal = if sings { 1.0 } else { 0.0 };
            // A singer starts or stops over a second.
            let from = s.presence;
            s.presence += (goal - s.presence).clamp(-block, block);
            if from == 0.0 && s.presence == 0.0 {
                continue;
            }
            for i in 0..n {
                s.t += dt;
                if s.t >= s.length + s.gap {
                    s.t -= s.length + s.gap;
                    s.length = s.echeme * self.rng.range_f32(0.85, 1.15);
                    s.gap = (s.gap_mean * self.rng.range_f32(0.75, 1.3)).clamp(0.06, 0.36);
                }
                // The echeme's envelope: 5 ms raised-cosine edges.
                let edge = 0.005f32;
                let env = if s.t >= s.length {
                    0.0
                } else {
                    let e = (s.t / edge).min((s.length - s.t) / edge).min(1.0);
                    0.5 - 0.5 * (std::f32::consts::PI * e).cos()
                };
                s.phase += s.pulse_rate * dt;
                // Each tymbal click a little early or late and of its own strength: a rasp, not
                // a tone.
                let pulse = if s.phase >= 1.0 {
                    s.phase -= 1.0 + self.rng.range_f32(-0.12, 0.12);
                    env * (from + (s.presence - from) * (i + 1) as f32 / n as f32)
                        * self.rng.range_f32(0.5, 1.0)
                } else {
                    0.0
                };
                let y = s.resonators[0].tick(pulse) + s.resonators[1].tick(pulse);
                left[i] += y * s.gains[0];
                right[i] += y * s.gains[1];
            }
        }
    }
}

/// One singer of the recorded chorus: the recording read from its own place, at its own
/// pitch, place and distance, in its own bouts.
struct Layer {
    threshold: f32,
    pos: f64,
    step: f64,
    gains: [f32; 2],
    dull: [OnePole; 2],
    presence: f32,
    bout: f32,
    pause: f32,
}

/// The cicadas from a recording: up to six singers, each the recording read from its own
/// place (so no two are in step), at its own pitch (±4 %), place and distance (two in the
/// middle distance first, the near one third, then three far and dulled), singing in bouts of 15 to 60 s with pauses
/// of 2 to 10 s. The intensity (0 to 1) sets how many sing, as for the cicadas made in code;
/// the recording loops with an equal-power seam of two seconds.
pub struct RecordedCicadas {
    rng: SplitMix64,
    rate: u32,
    song: Arc<[f32]>,
    seam: usize,
    layers: Vec<Layer>,
}

impl RecordedCicadas {
    /// From `seed`, singing `song` (made for this `rate`).
    pub fn new(seed: u64, rate: u32, song: Arc<CicadaSong>) -> Self {
        let mut rng = SplitMix64::new(seed);
        let samples = song.samples.clone();
        let len = samples.len().max(2);
        let ratio = song.rate as f64 / rate as f64;
        let near = [0.55, 0.45, 1.0, 0.3, 0.25, 0.2];
        let thresholds = [0.0, 0.15, 0.3, 0.5, 0.65, 0.8];
        let layers = (0..6)
            .map(|k| {
                let distance = near[k];
                let cutoff = 5000.0 + 7000.0 * distance;
                Layer {
                    threshold: thresholds[k],
                    pos: rng.range_f64(0.0, len as f64 * 0.9),
                    step: ratio * rng.range_f64(0.96, 1.04),
                    gains: pan_gains(rng.range_f32(-0.8, 0.8)).map(|g| g * distance),
                    dull: [OnePole::new(cutoff, rate); 2],
                    presence: 0.0,
                    bout: rng.range_f32(15.0, 60.0),
                    pause: 0.0,
                }
            })
            .collect();
        Self {
            rng,
            rate,
            seam: ((2 * rate) as usize).min(len / 3),
            song: samples,
            layers,
        }
    }
}

impl Texture for RecordedCicadas {
    fn render(&mut self, activity: f32, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        let block = n as f32 / self.rate as f32;
        let song = &self.song;
        let len = song.len();
        let seam = self.seam;
        if len < 2 * seam.max(2) {
            return;
        }
        let read = |p: f64| -> f32 {
            let k = (p as usize).min(len - 2);
            let t = (p - k as f64) as f32;
            song[k] + (song[k + 1] - song[k]) * t
        };
        for layer in &mut self.layers {
            if layer.pause > 0.0 {
                layer.pause -= block;
            } else {
                layer.bout -= block;
                if layer.bout <= 0.0 {
                    layer.pause = self.rng.range_f32(2.0, 10.0);
                    layer.bout = self.rng.range_f32(15.0, 60.0);
                }
            }
            let goal = if activity > layer.threshold && layer.pause <= 0.0 {
                1.0
            } else {
                0.0
            };
            let from = layer.presence;
            layer.presence += (goal - layer.presence).clamp(-block, block);
            if from == 0.0 && layer.presence == 0.0 {
                continue;
            }
            let tail = (len - seam) as f64;
            for i in 0..n {
                let g = from + (layer.presence - from) * (i + 1) as f32 / n as f32;
                let p = layer.pos;
                let mut x = read(p);
                // The loop's seam: the end fades out as the start fades in.
                if p >= tail {
                    let u = ((p - tail) / seam as f64) as f32 * std::f32::consts::FRAC_PI_2;
                    x = x * u.cos() + read(p - tail) * u.sin();
                }
                left[i] += g * layer.gains[0] * layer.dull[0].tick(x);
                right[i] += g * layer.gains[1] * layer.dull[1].tick(x);
                layer.pos += layer.step;
                if layer.pos >= (len - 1) as f64 {
                    layer.pos -= tail;
                }
            }
        }
    }
}

/// A note of a bird's call: a sine gliding from one pitch to another, with a little of its
/// octave, its edges raised-cosine.
#[derive(Clone, Copy, Debug)]
struct Note {
    start: f32,
    length: f32,
    from: f32,
    to: f32,
    amplitude: f32,
    edge: f32,
}

struct Call {
    notes: Vec<Note>,
    /// The note playing or next.
    next: usize,
    t: f32,
    /// The phasor (cos, sin) and its turn a sample, retuned every 16 samples along the glide.
    z: [f32; 2],
    turn: [f32; 2],
    tick: u32,
    gains: [f32; 2],
}

/// A recorded call playing: read at its own pitch (±3 %), dulled with distance.
struct Played {
    samples: Arc<[f32]>,
    pos: f64,
    step: f64,
    gains: [f32; 2],
    dull: OnePole,
}

/// The birds' species and how often each calls (#350), shares of the calls (normalised; 0
/// leaves a species out). The default is a Mediterranean village's, with the collared dove's coo
/// one call in eighteen. The owner heard it everywhere at three in twenty ("too much coucou").
/// A scene elsewhere sets its own ([`AmbienceAssets::bird_mix`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BirdMix {
    /// The blackbird's warble.
    pub blackbird: f32,
    /// The house sparrow's chirps.
    pub sparrow: f32,
    /// The serin's trill.
    pub serin: f32,
    /// The collared dove's coos.
    pub dove: f32,
}

impl Default for BirdMix {
    fn default() -> Self {
        Self {
            blackbird: 0.3,
            sparrow: 0.3,
            serin: 0.25,
            dove: 0.05,
        }
    }
}

/// One of [`BirdMix`]'s voices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BirdVoice {
    /// The blackbird.
    Blackbird,
    /// The house sparrow.
    Sparrow,
    /// The serin.
    Serin,
    /// The collared dove.
    Dove,
}

impl BirdMix {
    /// The voice a call takes for `u`, uniform in [0, 1): each by its share. A mix of no shares
    /// sings the blackbird.
    pub fn pick(&self, u: f32) -> BirdVoice {
        let shares = [
            (BirdVoice::Blackbird, self.blackbird),
            (BirdVoice::Sparrow, self.sparrow),
            (BirdVoice::Serin, self.serin),
            (BirdVoice::Dove, self.dove),
        ];
        let total: f32 = shares.iter().map(|s| s.1.max(0.0)).sum();
        let mut left = u * total;
        for (voice, share) in shares {
            let share = share.max(0.0);
            if share > 0.0 && left < share {
                return voice;
            }
            left -= share;
        }
        shares
            .iter()
            .rev()
            .find(|s| s.1 > 0.0)
            .map_or(BirdVoice::Blackbird, |s| s.0)
    }
}

/// Birds: calls arriving as a Poisson process of up to 5 a second, each from a bird at its
/// own place and distance. Four voices of a Mediterranean village: a blackbird's warble
/// (1.5 to 3.2 kHz glides), a house sparrow's chirps (4.6 falling to 3.1 kHz), a serin's
/// jingling trill (24 notes a second sweeping down from 6 kHz), a collared dove's three coos
/// near 500 Hz. With recordings ([`BirdCalls`]) the blackbird, the sparrow and the serin sing
/// their recorded calls instead, and the morning chorus plays far away while the dawn chorus
/// is at its fullest; the dove stays made in code.
pub struct Birds {
    rng: SplitMix64,
    rate: u32,
    until: f32,
    calls: Vec<Call>,
    recorded: Option<Arc<BirdCalls>>,
    played: Vec<Played>,
    /// Each species' last calls, not to be sung again soon.
    recent: [VecDeque<usize>; 3],
    chorus_at: usize,
    chorus_gain: f32,
    mix: BirdMix,
}

/// The morning chorus's crossfade where it loops, seconds.
const CHORUS_SEAM: f32 = 2.0;

impl Birds {
    /// From `seed`, every call made in code.
    pub fn new(seed: u64, rate: u32) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            rate,
            until: 0.0,
            calls: Vec::with_capacity(16),
            recorded: None,
            played: Vec::with_capacity(16),
            recent: Default::default(),
            chorus_at: 0,
            chorus_gain: 0.0,
            mix: BirdMix::default(),
        }
    }

    /// The same, its species called by `mix`.
    pub fn with_mix(mut self, mix: BirdMix) -> Self {
        self.mix = mix;
        self
    }

    /// From `seed`, singing the recorded calls of `recorded` (cut for this `rate`).
    pub fn with_recordings(seed: u64, rate: u32, recorded: Arc<BirdCalls>) -> Self {
        let mut birds = Self::new(seed, rate);
        if let Some(chorus) = &recorded.chorus {
            birds.chorus_at = birds.rng.below((chorus.len() / 2).max(1) as u64) as usize;
        }
        birds.recorded = Some(recorded);
        birds
    }

    /// Starts a call: the species by the mix, recorded when there is a recording of it.
    fn spawn(&mut self) {
        let voice = self.mix.pick(self.rng.next_f32());
        let species = match voice {
            BirdVoice::Blackbird => Some(Species::Blackbird),
            BirdVoice::Sparrow => Some(Species::Sparrow),
            BirdVoice::Serin => Some(Species::Serin),
            BirdVoice::Dove => None,
        };
        if let (Some(species), Some(recorded)) = (species, &self.recorded) {
            let count = recorded.of(species).count();
            if count > 0 {
                // Not one of the species' last three calls.
                let recent = &mut self.recent[species as usize];
                let pick = pick_avoiding(&mut self.rng, count, recent, 3);
                let call = recorded.of(species).nth(pick).expect("a call");
                let distance = log_uniform(&mut self.rng, 0.1, 1.0);
                let pitch = self.rng.range_f32(0.97, 1.03) as f64;
                // Far birds lose their highs: 4 kHz at the farthest, 12 kHz near.
                let cutoff = 4000.0 + 8000.0 * distance;
                self.played.push(Played {
                    samples: call.samples.clone(),
                    pos: 0.0,
                    step: pitch * recorded.rate as f64 / self.rate as f64,
                    gains: pan_gains(self.rng.range_f32(-0.9, 0.9)).map(|g| g * distance),
                    dull: OnePole::new(cutoff, self.rate),
                });
                return;
            }
        }
        let call = self.call(voice);
        self.calls.push(call);
    }

    fn call(&mut self, voice: BirdVoice) -> Call {
        let r = &mut self.rng;
        let mut notes = Vec::new();
        let mut t = 0.0;
        if voice == BirdVoice::Blackbird {
            // Blackbird.
            for k in 0..5 + r.below(5) {
                let length = r.range_f32(0.08, 0.25);
                let low = if k == 0 { 1500.0 } else { 1700.0 };
                notes.push(Note {
                    start: t,
                    length,
                    from: r.range_f32(low, 3200.0),
                    to: r.range_f32(low, 3200.0),
                    amplitude: r.range_f32(0.5, 1.0),
                    edge: 0.012,
                });
                t += length + r.range_f32(0.03, 0.09);
            }
        } else if voice == BirdVoice::Sparrow {
            // House sparrow.
            for _ in 0..3 + r.below(5) {
                let length = r.range_f32(0.06, 0.1);
                notes.push(Note {
                    start: t,
                    length,
                    from: r.range_f32(4200.0, 5000.0),
                    to: r.range_f32(2800.0, 3400.0),
                    amplitude: r.range_f32(0.4, 0.8),
                    edge: 0.008,
                });
                t += length + r.range_f32(0.1, 0.2);
            }
        } else if voice == BirdVoice::Serin {
            // Serin.
            let period = 1.0 / r.range_f32(20.0, 28.0);
            let count = (r.range_f32(1.2, 2.2) / period) as usize;
            for _ in 0..count {
                notes.push(Note {
                    start: t,
                    length: period * 0.75,
                    from: r.range_f32(5500.0, 7000.0),
                    to: r.range_f32(3800.0, 5000.0),
                    amplitude: r.range_f32(0.3, 0.6),
                    edge: 0.004,
                });
                t += period;
            }
        } else {
            // Collared dove: coo-COO-coo.
            let pitch = r.range_f32(480.0, 560.0);
            for (length, loud) in [(0.35, 0.6), (0.6, 1.0), (0.45, 0.7)] {
                notes.push(Note {
                    start: t,
                    length,
                    from: pitch,
                    to: pitch * 0.94,
                    amplitude: loud,
                    edge: 0.04,
                });
                t += length + r.range_f32(0.12, 0.18);
            }
        }
        let distance = log_uniform(r, 0.1, 1.0);
        Call {
            notes,
            next: 0,
            t: 0.0,
            z: [1.0, 0.0],
            turn: [1.0, 0.0],
            tick: 0,
            gains: pan_gains(r.range_f32(-0.9, 0.9)).map(|g| g * distance),
        }
    }
}

impl Texture for Birds {
    fn render(&mut self, activity: f32, left: &mut [f32], right: &mut [f32]) {
        let dt = 1.0 / self.rate as f32;
        let calls = 5.0 * activity.clamp(0.0, 1.5);
        self.until -= left.len() as f32 * dt;
        while self.until <= 0.0 && calls > 0.0 {
            self.until += poisson_wait(&mut self.rng, calls);
            if self.calls.len() + self.played.len() < 16 {
                self.spawn();
            }
        }
        if calls <= 0.0 {
            self.until = 0.0;
        }
        let tau = std::f32::consts::TAU;
        let pi = std::f32::consts::PI;
        for call in &mut self.calls {
            for i in 0..left.len() {
                let t = call.t;
                call.t += dt;
                while call
                    .notes
                    .get(call.next)
                    .is_some_and(|n| t >= n.start + n.length)
                {
                    call.next += 1;
                }
                let Some(note) = call.notes.get(call.next).filter(|n| t >= n.start) else {
                    continue;
                };
                if call.tick.is_multiple_of(16) {
                    let u = (t - note.start) / note.length;
                    let glide = 0.5 - 0.5 * (pi * u).cos();
                    let w = tau * (note.from + (note.to - note.from) * glide) * dt;
                    call.turn = [w.cos(), w.sin()];
                    let norm = call.z[0].hypot(call.z[1]).max(1e-6);
                    call.z = call.z.map(|v| v / norm);
                }
                call.tick = call.tick.wrapping_add(1);
                let [re, im] = call.z;
                call.z = [
                    re * call.turn[0] - im * call.turn[1],
                    re * call.turn[1] + im * call.turn[0],
                ];
                let e = ((t - note.start) / note.edge)
                    .min((note.start + note.length - t) / note.edge)
                    .min(1.0);
                let env = if e < 1.0 {
                    0.5 - 0.5 * (pi * e).cos()
                } else {
                    1.0
                };
                // The sine and its octave (sin 2φ = 2 sin φ cos φ).
                let y = 0.3 * note.amplitude * env * (im + 0.24 * re * im);
                left[i] += y * call.gains[0];
                right[i] += y * call.gains[1];
            }
        }
        self.calls.retain(|c| c.next < c.notes.len());

        // The recorded calls, read by linear interpolation.
        for p in &mut self.played {
            let len = p.samples.len();
            for i in 0..left.len() {
                let k = p.pos as usize;
                if k + 1 >= len {
                    p.pos = len as f64;
                    break;
                }
                let t = (p.pos - k as f64) as f32;
                let x = p.samples[k] + (p.samples[k + 1] - p.samples[k]) * t;
                let y = 1.2 * p.dull.tick(x);
                left[i] += y * p.gains[0];
                right[i] += y * p.gains[1];
                p.pos += p.step;
            }
        }
        self.played
            .retain(|p| (p.pos as usize) + 1 < p.samples.len());

        // The morning chorus, far away, while the dawn chorus is at its fullest; looped with
        // an equal-power crossfade of two seconds.
        if let Some(chorus) = self.recorded.as_ref().and_then(|r| r.chorus.clone()) {
            let frames = chorus.len() / 2;
            let seam = ((CHORUS_SEAM * self.rate as f32) as usize).min(frames / 2);
            let goal = smoothstep(0.35, 1.0, activity);
            let from = self.chorus_gain;
            self.chorus_gain = goal;
            if (from > 0.0 || goal > 0.0) && frames > 2 * seam {
                let n = left.len() as f32;
                for i in 0..left.len() {
                    let g = from + (goal - from) * (i + 1) as f32 / n;
                    let at = self.chorus_at;
                    let (mut l, mut r) = (chorus[2 * at], chorus[2 * at + 1]);
                    if at >= frames - seam {
                        let u = (at - (frames - seam)) as f32 / seam as f32;
                        let (fade, rise) = (
                            (u * std::f32::consts::FRAC_PI_2).cos(),
                            (u * std::f32::consts::FRAC_PI_2).sin(),
                        );
                        let b = at - (frames - seam);
                        l = l * fade + chorus[2 * b] * rise;
                        r = r * fade + chorus[2 * b + 1] * rise;
                    }
                    left[i] += g * l;
                    right[i] += g * r;
                    self.chorus_at += 1;
                    if self.chorus_at >= frames {
                        self.chorus_at = seam;
                    }
                }
            }
        }
    }
}

/// A wave against the quay: band-passed noise rising over 80 to 180 ms and dying over 0.35
/// to 0.7 s, then the gurgle of its bubbles (3 to 10 mm: 330 Hz to 1.1 kHz).
struct Wave {
    t: f32,
    attack: f32,
    decay: f32,
    amplitude: f32,
    gains: [f32; 2],
    band: [Biquad; 2],
    gurgle: f32,
}

/// The sea against the quays: waves every 2 to 5.5 s, shorter apart and louder with the
/// swell (0 to 1), a splash above the biggest, and a low wash under them.
pub struct Sea {
    rng: SplitMix64,
    rate: u32,
    until: f32,
    waves: Vec<Wave>,
    pool: GrainPool,
    wash: [PinkNoise; 2],
    wash_low: [LowPass4; 2],
    splash: [Biquad; 2],
    swell: Wander,
}

impl Sea {
    /// From `seed`.
    pub fn new(seed: u64, rate: u32) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            rate,
            until: 0.3,
            waves: Vec::with_capacity(8),
            pool: GrainPool::new(rate),
            wash: [PinkNoise::default(); 2],
            wash_low: [LowPass4::new(300.0, rate); 2],
            splash: [Biquad::high_pass(2000.0, 0.7, rate); 2],
            swell: Wander::new(3.0, 0.3, SUB as f32 / rate as f32),
        }
    }
}

impl Texture for Sea {
    fn render(&mut self, swell: f32, left: &mut [f32], right: &mut [f32]) {
        let swell = swell.clamp(0.0, 1.0);
        let dt = 1.0 / self.rate as f32;
        let n = left.len();
        self.until -= n as f32 * dt;
        if self.until <= 0.0 {
            let r = &mut self.rng;
            self.until += r.range_f32(2.0, 5.5) * (1.2 - 0.4 * swell);
            let centre = r.range_f32(250.0, 600.0);
            self.waves.push(Wave {
                t: 0.0,
                attack: r.range_f32(0.08, 0.18),
                decay: r.range_f32(0.35, 0.7),
                amplitude: (0.3 + 0.7 * swell) * r.range_f32(0.5, 1.0),
                gains: pan_gains(r.range_f32(-0.6, 0.6)),
                band: [Biquad::band_pass(centre, 0.7, self.rate); 2],
                gurgle: r.range_f32(0.2, 1.0),
            });
        }
        let wash = 0.12 * (0.5 + 0.5 * swell) * (1.0 + 0.5 * self.swell.step(&mut self.rng));
        for i in 0..n {
            let mut out = [0.0f32; 2];
            for w in &mut self.waves {
                w.t += dt;
                let env = if w.t < w.attack {
                    let s = (std::f32::consts::FRAC_PI_2 * w.t / w.attack).sin();
                    s * s
                } else {
                    (-(w.t - w.attack) / w.decay).exp()
                };
                let splash = if w.amplitude > 0.7 {
                    0.4 * (w.amplitude - 0.6) * (-(w.t - w.attack).max(0.0) / 0.12).exp()
                } else {
                    0.0
                };
                for (c, o) in out.iter_mut().enumerate() {
                    let x = white(&mut self.rng);
                    *o += w.gains[c]
                        * (w.amplitude * env * w.band[c].tick(x)
                            + splash * env.min(1.0) * self.splash[c].tick(white(&mut self.rng)));
                }
                // The gurgle: bubbles after the wave's peak.
                if w.t > w.attack && w.t < w.attack + w.gurgle {
                    let rate = 40.0 * w.amplitude;
                    if self.rng.next_f32() < rate * dt {
                        let radius = log_uniform(&mut self.rng, 0.003, 0.01);
                        let amp = 0.15 * w.amplitude * self.rng.range_f32(0.3, 1.0);
                        let pan = self.rng.range_f32(-0.6, 0.6);
                        self.pool.spawn(Grain::bubble(radius, amp, pan));
                    }
                }
            }
            let [l, r] = self.pool.tick();
            for (c, o) in out.iter().enumerate() {
                let p = self.wash[c].tick(white(&mut self.rng));
                let x = o + wash * self.wash_low[c].tick(p);
                if c == 0 {
                    left[i] += x + l;
                } else {
                    right[i] += x + r;
                }
            }
        }
        self.waves.retain(|w| w.t < w.attack + 8.0 * w.decay);
    }
}

/// Under water: a low rumble (pink noise under 160 Hz, nearly the same in both ears, as
/// sound under water has almost no direction), the diver's breath every 3.5 to 6 s (12 to
/// 30 bubbles of 2 to 7 mm over most of a second, near the middle), and far bubbles.
pub struct Underwater {
    rng: SplitMix64,
    rate: u32,
    rumble: PinkNoise,
    rumble_low: LowPass4,
    side: PinkNoise,
    side_low: LowPass4,
    swell: Wander,
    pool: GrainPool,
    breath_in: f32,
    breath: f32,
    far_until: f32,
}

impl Underwater {
    /// From `seed`.
    pub fn new(seed: u64, rate: u32) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            rate,
            rumble: PinkNoise::default(),
            rumble_low: LowPass4::new(160.0, rate),
            side: PinkNoise::default(),
            side_low: LowPass4::new(160.0, rate),
            swell: Wander::new(2.0, 0.3, SUB as f32 / rate as f32),
            pool: GrainPool::new(rate),
            breath_in: 1.0,
            breath: 0.0,
            far_until: 0.0,
        }
    }
}

impl Texture for Underwater {
    fn render(&mut self, _intensity: f32, left: &mut [f32], right: &mut [f32]) {
        let dt = 1.0 / self.rate as f32;
        let n = left.len();
        let block = n as f32 * dt;
        let level = 0.25 * (1.0 + 0.4 * self.swell.step(&mut self.rng));
        self.breath_in -= block;
        if self.breath_in <= 0.0 {
            self.breath_in += self.rng.range_f32(3.5, 6.0);
            self.breath = self.rng.range_f32(0.6, 1.0);
        }
        let r = &mut self.rng;
        for i in 0..n {
            if self.breath > 0.0 {
                self.breath -= dt;
                if r.next_f32() < 25.0 * dt {
                    let radius = log_uniform(r, 0.002, 0.007);
                    let amp = r.range_f32(0.1, 0.3);
                    let pan = r.range_f32(-0.2, 0.2);
                    self.pool.spawn(Grain::bubble(radius, amp, pan));
                }
            }
            self.far_until -= dt;
            if self.far_until <= 0.0 {
                self.far_until += poisson_wait(r, 1.5);
                let radius = log_uniform(r, 0.001, 0.004);
                let amp = r.range_f32(0.01, 0.04);
                let pan = r.range_f32(-0.8, 0.8);
                self.pool.spawn(Grain::bubble(radius, amp, pan));
            }
            let mid = level * self.rumble_low.tick(self.rumble.tick(white(r)));
            let side = 0.15 * level * self.side_low.tick(self.side.tick(white(r)));
            let [l, rr] = self.pool.tick();
            left[i] += mid + side + l;
            right[i] += mid - side + rr;
        }
    }
}
