//! An algorithmic reverb: a feedback delay network after Jot and Chaigne (AES 1991).
//!
//! Eight delay lines of mutually prime lengths (21 to 60 ms in a room of size 1) feed each
//! other through an 8×8 Hadamard matrix, which is orthogonal and so keeps the energy; each
//! line's own gain sets the decay: a line of `m` frames loses `60 m / (T₆₀ f)` dB a lap, so
//! every line, and the sum, falls 60 dB in `T₆₀` seconds whatever its length. A one-pole
//! low-pass in each loop makes the highs die sooner (`hf_ratio`: the high band's `T₆₀` over
//! the low band's), as air and soft surfaces do. A pre-delay sets the gap before the first
//! reflections. Its parameters are the acoustic ones a room or a zone has (D-011): the decay
//! time, how it darkens, the room's size.
//!
//! The lines are sized for the largest room when the reverb is made; changing parameters
//! while it runs moves read positions and coefficients, never memory.

use super::OnePole;

const LINES: usize = 8;
/// Line lengths in frames at 48 kHz for a room of size 1: primes, 21.5 to 59.6 ms.
const BASE_LENGTHS: [usize; LINES] = [1031, 1327, 1523, 1801, 2053, 2311, 2579, 2861];
const BASE_RATE: f32 = 48_000.0;
/// The largest size the lines are allocated for.
pub const MAX_SIZE: f32 = 2.5;
/// The longest pre-delay, seconds.
pub const MAX_PREDELAY: f32 = 0.2;

/// A reverb's acoustic parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// Seconds for the low and mid bands to fall by 60 dB.
    pub rt60: f32,
    /// The high band's decay time as a share of `rt60` (below 1: a darker tail).
    pub hf_ratio: f32,
    /// The room's size: scales the delay lines (1 is a room of a few metres; at most
    /// [`MAX_SIZE`]).
    pub size: f32,
    /// Seconds before the tail starts (at most [`MAX_PREDELAY`]).
    pub predelay: f32,
    /// The output's gain.
    pub wet: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self::ROOM
    }
}

impl ReverbParams {
    /// A furnished room.
    pub const ROOM: ReverbParams = ReverbParams {
        rt60: 0.6,
        hf_ratio: 0.6,
        size: 0.8,
        predelay: 0.008,
        wet: 1.0,
    };
    /// A stone hall or a church.
    pub const HALL: ReverbParams = ReverbParams {
        rt60: 2.4,
        hf_ratio: 0.5,
        size: 2.0,
        predelay: 0.025,
        wet: 1.0,
    };
    /// A cave: long, bright, large.
    pub const CAVE: ReverbParams = ReverbParams {
        rt60: 3.2,
        hf_ratio: 0.8,
        size: 2.4,
        predelay: 0.03,
        wet: 1.0,
    };
    /// Under water in a harbour: sound travels 4.3 times faster, so the space sounds 4.3
    /// times smaller (dense, quick echoes between the surface, the bottom and the quay),
    /// and its highs die soon.
    pub const UNDERWATER: ReverbParams = ReverbParams {
        rt60: 1.4,
        hf_ratio: 0.35,
        size: 0.3,
        predelay: 0.003,
        wet: 0.7,
    };
    /// Outdoors: a short, dark trace of what the ground and the trees return.
    pub const OPEN: ReverbParams = ReverbParams {
        rt60: 0.35,
        hf_ratio: 0.4,
        size: 1.5,
        predelay: 0.02,
        wet: 0.5,
    };

    /// The parameters `t` of the way from `self` to `other` (decay times in the log domain,
    /// so the change sounds even).
    pub fn lerp(&self, other: &ReverbParams, t: f32) -> ReverbParams {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let log_mix =
            |a: f32, b: f32| (a.max(1e-3).ln() + (b.max(1e-3).ln() - a.max(1e-3).ln()) * t).exp();
        ReverbParams {
            rt60: log_mix(self.rt60, other.rt60),
            hf_ratio: mix(self.hf_ratio, other.hf_ratio),
            size: mix(self.size, other.size),
            predelay: mix(self.predelay, other.predelay),
            wet: mix(self.wet, other.wet),
        }
    }
}

/// The reverb's state.
#[derive(Clone, Debug)]
pub struct Reverb {
    rate: u32,
    params: ReverbParams,
    lines: [Box<[f32]>; LINES],
    lengths: [usize; LINES],
    pos: [usize; LINES],
    gain: [f32; LINES],
    damp: [OnePole; LINES],
    damp_coef: [f32; LINES],
    predelay: Box<[f32]>,
    predelay_len: usize,
    predelay_pos: usize,
    out_gain: f32,
}

impl Reverb {
    /// A reverb at `rate` frames a second.
    pub fn new(params: ReverbParams, rate: u32) -> Self {
        let scale = rate as f32 / BASE_RATE;
        let lines = BASE_LENGTHS.map(|l| {
            vec![0.0; (l as f32 * scale * MAX_SIZE).ceil() as usize + 1].into_boxed_slice()
        });
        let mut reverb = Self {
            rate,
            params,
            lines,
            lengths: [1; LINES],
            pos: [0; LINES],
            gain: [0.0; LINES],
            damp: [OnePole::default(); LINES],
            damp_coef: [0.0; LINES],
            predelay: vec![0.0; (MAX_PREDELAY * rate as f32).ceil() as usize + 1]
                .into_boxed_slice(),
            predelay_len: 1,
            predelay_pos: 0,
            out_gain: 0.0,
        };
        reverb.set_params(params);
        reverb
    }

    /// The current parameters.
    pub fn params(&self) -> ReverbParams {
        self.params
    }

    /// Changes the parameters (no allocation).
    pub fn set_params(&mut self, params: ReverbParams) {
        let rate = self.rate as f32;
        let size = params.size.clamp(0.1, MAX_SIZE);
        let rt60 = params.rt60.max(0.05);
        let rt60_high = rt60 * params.hf_ratio.clamp(0.05, 1.0);
        for (i, &base) in BASE_LENGTHS.iter().enumerate() {
            let length = ((base as f32 * rate / BASE_RATE * size) as usize)
                .clamp(1, self.lines[i].len() - 1);
            self.lengths[i] = length;
            // −60 dB over rt60: 10^(−3 m / (T₆₀ f)) a lap, at low and at high frequencies.
            let g_low = 10f32.powf(-3.0 * length as f32 / (rt60 * rate));
            let g_high = 10f32.powf(-3.0 * length as f32 / (rt60_high * rate));
            // A one-pole low-pass of DC gain 1 and Nyquist gain (1 − b)/(1 + b) = g_high/g_low.
            let r = (g_high / g_low).clamp(1e-4, 1.0);
            let b = (1.0 - r) / (1.0 + r);
            self.gain[i] = g_low;
            self.damp_coef[i] = 1.0 - b;
        }
        self.predelay_len = ((params.predelay.clamp(0.0, MAX_PREDELAY) * rate) as usize)
            .clamp(1, self.predelay.len() - 1);
        // The network's output sums eight lines: scale back to about the input's level.
        self.out_gain = params.wet / (LINES as f32).sqrt();
        self.params = ReverbParams { size, ..params };
    }

    /// Reverberates a stereo block in place: the input is folded to mono, the output is
    /// the tail alone (100 % wet), decorrelated between the channels.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // Pre-delay.
            let input = 0.5 * (*l + *r);
            self.predelay[self.predelay_pos] = input;
            let read =
                (self.predelay_pos + self.predelay.len() - self.predelay_len) % self.predelay.len();
            let delayed = self.predelay[read];
            self.predelay_pos = (self.predelay_pos + 1) % self.predelay.len();

            // Read each line `length` back, damp it.
            let mut x = [0.0f32; LINES];
            for (i, xi) in x.iter_mut().enumerate() {
                let line = &self.lines[i];
                let at = (self.pos[i] + line.len() - self.lengths[i]) % line.len();
                let y = line[at] * self.gain[i];
                // One-pole with DC gain 1: y_d += (1 − b)(y − y_d).
                *xi = self.damp[i].tick_with(y, self.damp_coef[i]);
            }
            // Outputs: even lines left, odd lines right, alternate signs.
            let out_l = x[0] - x[2] + x[4] - x[6];
            let out_r = x[1] - x[3] + x[5] - x[7];
            // Feedback through the Hadamard matrix (fast Walsh–Hadamard, scaled by 1/√8).
            hadamard8(&mut x);
            for (i, &xi) in x.iter().enumerate() {
                let line = &mut self.lines[i];
                line[self.pos[i]] = xi + delayed;
                self.pos[i] = (self.pos[i] + 1) % line.len();
            }
            *l = out_l * self.out_gain;
            *r = out_r * self.out_gain;
        }
    }

    /// Whether the tail has died away (every line below −100 dB).
    pub fn is_silent(&self) -> bool {
        self.lines
            .iter()
            .all(|line| line.iter().all(|&x| x.abs() < 1e-5))
    }

    /// Clears the tail.
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.predelay.fill(0.0);
        for d in &mut self.damp {
            d.reset();
        }
    }
}

/// The 8-point Walsh–Hadamard transform, orthonormal.
#[inline]
fn hadamard8(x: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        for i in (0..LINES).step_by(h * 2) {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
        }
        h *= 2;
    }
    let norm = 1.0 / (LINES as f32).sqrt();
    for v in x.iter_mut() {
        *v *= norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decay time from a Schroeder backward integral, fitted between −5 and −35 dB.
    pub(crate) fn rt60_of(response: &[f32], rate: u32) -> f32 {
        let mut energy: Vec<f64> = response.iter().map(|&x| (x as f64) * (x as f64)).collect();
        for i in (0..energy.len() - 1).rev() {
            energy[i] += energy[i + 1];
        }
        let total = energy[0];
        let db: Vec<f64> = energy.iter().map(|&e| 10.0 * (e / total).log10()).collect();
        let t5 = db.iter().position(|&d| d <= -5.0).unwrap();
        let t35 = db.iter().position(|&d| d <= -35.0).unwrap();
        // 30 dB in (t35 − t5) frames: 60 dB in twice that.
        2.0 * (t35 - t5) as f32 / rate as f32
    }

    #[test]
    fn the_hadamard_keeps_energy() {
        let mut x = [1.0, -2.0, 0.5, 3.0, 0.0, 1.5, -1.0, 2.0];
        let before: f32 = x.iter().map(|v| v * v).sum();
        hadamard8(&mut x);
        let after: f32 = x.iter().map(|v| v * v).sum();
        assert!((before - after).abs() < 1e-4);
    }

    #[test]
    fn the_tail_falls_60_db_in_rt60() {
        let rate = 48_000;
        for rt60 in [0.5f32, 1.2, 2.5] {
            let params = ReverbParams {
                rt60,
                hf_ratio: 1.0,
                size: 1.0,
                predelay: 0.0,
                wet: 1.0,
            };
            let mut reverb = Reverb::new(params, rate);
            let frames = (rt60 * 1.5 * rate as f32) as usize;
            let mut l = vec![0.0f32; frames];
            l[0] = 1.0;
            let mut r = l.clone();
            reverb.process(&mut l, &mut r);
            let measured = rt60_of(&l, rate);
            assert!(
                (measured / rt60 - 1.0).abs() < 0.15,
                "RT60 {rt60}: measured {measured}"
            );
        }
    }
}
