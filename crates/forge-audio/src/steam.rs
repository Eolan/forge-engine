//! Steam Audio's binaural rendering as a [`Spatializer`] (D-011; feature `steam-audio`).
//!
//! Each voice slot of the mixer gets its own binaural effect: the head-related transfer
//! function (HRTF) of the source's direction, convolved with the voice's block, so a source
//! behind sounds behind and one above sounds above, which equal-power panning cannot do. The
//! HRTF is Steam Audio's default (D-011 names SADIE II's for later, a download of its own).
//!
//! The mixer hands the spatializer a voice's mono block with its distance curve, air
//! absorption and occlusion already applied; this ramps the voice's gain from the last block
//! to this one, then lets Steam Audio render the direction (it crossfades between the last
//! block's HRTF and this one's). Inside the source's radius the effect blends toward the
//! centre (`spatial_blend`), as the panner pulls toward it. A voice that starts inside a block
//! is placed at its frame in a block-long buffer, since the effect works on whole frames of
//! [`BLOCK`] samples. Nothing here allocates once made.
//!
//! The library (`phonon.dll`) is Valve's Steam Audio 4.8.1 (Apache-2.0), reached through
//! `audionimbus`; it is found through `STEAMAUDIO_LIB_DIR` (`.cargo/config.toml` points it at
//! `steam-audio-sdk/`, which git ignores).

use audionimbus::{
    AudioBufferMut, AudioBufferRef, AudioSettings, BinauralEffect, BinauralEffectParams,
    BinauralEffectSettings, Context, ContextSettings, Direction, Hrtf, HrtfInterpolation,
    HrtfSettings, SteamAudioError,
};

use crate::BLOCK;
use crate::spatial::{DirectPath, Spatializer};

/// Renders spatial voices through Steam Audio's HRTF.
pub struct SteamSpatializer {
    effects: Vec<BinauralEffect>,
    params: BinauralEffectParams,
    input: Box<[f32]>,
    output: Box<[f32]>,
    // Kept alive as long as the effects that use them.
    _hrtf: Hrtf,
    _context: Context,
}

impl SteamSpatializer {
    /// A spatializer for `slots` voices at `rate` (44.1 or 48 kHz: Steam Audio's default HRTF
    /// is sampled for those), with bilinear interpolation between the HRTF's measured
    /// directions.
    pub fn new(rate: u32, slots: usize) -> Result<Self, SteamAudioError> {
        let context = Context::try_new(&ContextSettings::default())?;
        let settings = AudioSettings {
            sampling_rate: rate,
            frame_size: BLOCK as u32,
        };
        let hrtf = Hrtf::try_new(&context, &settings, &HrtfSettings::default())?;
        let effects = (0..slots)
            .map(|_| {
                BinauralEffect::try_new(
                    &context,
                    &settings,
                    &BinauralEffectSettings { hrtf: hrtf.clone() },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            effects,
            params: BinauralEffectParams {
                direction: Direction::new(0.0, 0.0, -1.0),
                interpolation: HrtfInterpolation::Bilinear,
                spatial_blend: 1.0,
                hrtf: hrtf.clone(),
                peak_delays: None,
            },
            input: vec![0.0; BLOCK].into_boxed_slice(),
            output: vec![0.0; 2 * BLOCK].into_boxed_slice(),
            _hrtf: hrtf,
            _context: context,
        })
    }

    /// A spatializer for a mixer made with `config`: a slot for every voice it mixes and every
    /// stolen voice fading out.
    pub fn for_mixer(config: &crate::MixerConfig) -> Result<Self, SteamAudioError> {
        Self::new(config.sample_rate, config.max_voices + config.tail_voices)
    }
}

impl Spatializer for SteamSpatializer {
    fn render(
        &mut self,
        slot: usize,
        previous: &DirectPath,
        path: &DirectPath,
        input: &[f32],
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let n = input.len().min(BLOCK);
        let begin = BLOCK - n;
        // The voice's gain, ramped from the last block's across this one.
        self.input[..begin].fill(0.0);
        let step = (path.gain - previous.gain) / n.max(1) as f32;
        for (i, (x, &v)) in self.input[begin..].iter_mut().zip(input).enumerate() {
            *x = v * (previous.gain + step * (i + 1) as f32);
        }
        let d = path.direction;
        let (direction, blend) = if path.distance > 0.0 && d.length_squared() > 0.0 {
            (d, path.distance / path.distance.max(path.spread))
        } else {
            (glam::Vec3::NEG_Z, 0.0)
        };
        self.params.direction = Direction::new(direction.x, direction.y, direction.z);
        self.params.spatial_blend = blend;
        let Some(effect) = self.effects.get_mut(slot) else {
            return;
        };
        let (Ok(source), Ok(mut out)) = (
            AudioBufferRef::try_new(&self.input, 1),
            AudioBufferMut::try_new(&mut self.output, 2),
        ) else {
            return;
        };
        if effect.apply(&self.params, &source, &mut out).is_err() {
            return;
        }
        let (l, r) = self.output.split_at(BLOCK);
        for i in 0..n {
            left[i] += l[begin + i];
            right[i] += r[begin + i];
        }
    }

    fn reset(&mut self, slot: usize) {
        if let Some(effect) = self.effects.get_mut(slot) {
            effect.reset();
        }
    }
}
