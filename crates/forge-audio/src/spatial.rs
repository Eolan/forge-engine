//! Where a sound is heard from: the listener, a source's place, and what the path between
//! them does to it.
//!
//! Positions are `f64` world positions (D-004); the mixer takes each source relative to the
//! listener and works in `f32` from there. A spatial voice is mono (a stereo sound is folded
//! to mono). Each block the mixer computes, per voice, its [`DirectPath`]: the direction in
//! the listener's frame, the distance, the gain of the distance curve, and three band gains
//! (air absorption and occlusion, in Steam Audio's bands: below 800 Hz, 800 Hz to 8 kHz,
//! above 8 kHz). A [`Spatializer`] then renders the filtered mono block into the two
//! channels. The default, [`PanSpatializer`], pans at equal power by the direction's lateral
//! part; Steam Audio's binaural effect (HRTF, D-011) is meant to slot in as another
//! `Spatializer`. Panning alone cannot tell front from back; a binaural renderer can.
//!
//! The listener's frame: +X to the right, +Y up, −Z ahead (the engine's convention).

use glam::{DVec3, Vec3};

/// Steam Audio's default air absorption, per metre in each band: the gain over `d` metres
/// is `exp(−k d)` (`core/src/core/air_absorption.cpp`, Apache-2.0). At 100 m that is
/// −0.2, −1.5 and −15.8 dB.
pub const AIR_ABSORPTION: [f32; 3] = [0.0002, 0.0017, 0.0182];

/// The speed of sound in air at 20 °C, m/s.
pub const SPEED_OF_SOUND: f32 = 343.0;

/// Who hears: a position, an orientation and a velocity (for Doppler).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    /// Where, in the world.
    pub position: DVec3,
    /// Where the listener faces (normalised by the mixer).
    pub forward: Vec3,
    /// The listener's up.
    pub up: Vec3,
    /// Metres a second.
    pub velocity: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            forward: Vec3::NEG_Z,
            up: Vec3::Y,
            velocity: Vec3::ZERO,
        }
    }
}

impl Listener {
    /// A listener at `position` facing −Z.
    pub fn at(position: DVec3) -> Self {
        Self {
            position,
            ..Self::default()
        }
    }

    /// The listener's right: forward × up.
    pub fn right(&self) -> Vec3 {
        self.forward.cross(self.up).normalize_or(Vec3::X)
    }

    /// A world-space offset in the listener's frame (+X right, +Y up, −Z ahead).
    pub fn to_local(&self, offset: Vec3) -> Vec3 {
        let forward = self.forward.normalize_or(Vec3::NEG_Z);
        let right = self.right();
        let up = right.cross(forward);
        Vec3::new(offset.dot(right), offset.dot(up), -offset.dot(forward))
    }
}

/// How loudness falls with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attenuation {
    /// No change with distance.
    None,
    /// The inverse-distance law, clamped: `reference / (reference + rolloff (d − reference))`
    /// with `d` held between `reference` and `max` (OpenAL's clamped inverse model). With a
    /// rolloff of 1 the level drops 6 dB each time the distance doubles past `reference`.
    Inverse {
        /// Metres within which the gain is 1.
        reference: f32,
        /// Metres past which the gain stops falling.
        max: f32,
        /// How fast it falls (1 is physical).
        rolloff: f32,
    },
    /// A straight line from 1 at `min` to 0 at `max`.
    Linear {
        /// Metres within which the gain is 1.
        min: f32,
        /// Metres from which the gain is 0.
        max: f32,
    },
}

impl Default for Attenuation {
    fn default() -> Self {
        Attenuation::Inverse {
            reference: 1.0,
            max: 1000.0,
            rolloff: 1.0,
        }
    }
}

impl Attenuation {
    /// The gain at `distance` metres.
    pub fn gain(&self, distance: f32) -> f32 {
        match *self {
            Attenuation::None => 1.0,
            Attenuation::Inverse {
                reference,
                max,
                rolloff,
            } => {
                let reference = reference.max(1e-3);
                let d = distance.clamp(reference, max.max(reference));
                reference / (reference + rolloff * (d - reference))
            }
            Attenuation::Linear { min, max } => {
                if distance <= min {
                    1.0
                } else if distance >= max {
                    0.0
                } else {
                    1.0 - (distance - min) / (max - min)
                }
            }
        }
    }
}

/// What stands between a source and the listener: how much of the direct path is blocked
/// and what the blocker lets through in each band.
///
/// The game finds `amount` (the share of rays from the listener to the source's volume that
/// a solid hits, as Steam Audio's volumetric occlusion does) and takes `transmission` from
/// the blocker's material row (D-007). The direct sound's band gains are then
/// `(1 − amount) + amount × transmission`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occlusion {
    /// 0 (clear) to 1 (fully behind the blocker).
    pub amount: f32,
    /// The blocker's gain in each band.
    pub transmission: [f32; 3],
}

impl Occlusion {
    /// A clear path.
    pub const NONE: Occlusion = Occlusion {
        amount: 0.0,
        transmission: [1.0; 3],
    };

    /// A path blocked by `amount` with `transmission` through the blocker.
    pub fn new(amount: f32, transmission: [f32; 3]) -> Self {
        Self {
            amount: amount.clamp(0.0, 1.0),
            transmission,
        }
    }

    /// `amount` of the path blocked by the surfaces `layers` (a wall, or several in a row):
    /// their transmissions multiply, as Steam Audio's do along a ray.
    pub fn through(amount: f32, layers: &[AcousticMaterial]) -> Self {
        let mut transmission = [1.0f32; 3];
        for layer in layers {
            for (t, l) in transmission.iter_mut().zip(layer.transmission) {
                *t *= l;
            }
        }
        Self::new(amount, transmission)
    }

    /// The direct sound's gain in each band.
    pub fn bands(&self) -> [f32; 3] {
        let a = self.amount;
        self.transmission.map(|t| (1.0 - a) + a * t.clamp(0.0, 1.0))
    }
}

impl Default for Occlusion {
    fn default() -> Self {
        Self::NONE
    }
}

/// A surface's acoustics in the three bands, as Steam Audio's `IPLMaterial` gives them: what it
/// absorbs when sound reflects off it, how much it scatters, what passes through it.
///
/// The presets are Steam Audio's (`core/src/core/phonon.h`, Apache-2.0), measured for bands
/// centred on 400 Hz, 2.5 kHz and 15 kHz. Steam Audio applies `transmission` as the band EQ's
/// gains on the occluded share of the direct sound, and so does [`Occlusion::through`]. The
/// world's material rows (D-007) are meant to carry these three arrays; until they do, a game
/// picks a preset per surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticMaterial {
    /// Share of the energy absorbed on reflection in each band, 0 to 1.
    pub absorption: [f32; 3],
    /// Share scattered in a random direction on reflection (0: a mirror, 1: fully diffuse).
    pub scattering: f32,
    /// What passes through, a gain in each band.
    pub transmission: [f32; 3],
}

impl AcousticMaterial {
    const fn new(absorption: [f32; 3], scattering: f32, transmission: [f32; 3]) -> Self {
        Self {
            absorption,
            scattering,
            transmission,
        }
    }

    /// Steam Audio's default.
    pub const GENERIC: Self = Self::new([0.10, 0.20, 0.30], 0.05, [0.100, 0.050, 0.030]);
    /// Brick.
    pub const BRICK: Self = Self::new([0.03, 0.04, 0.07], 0.05, [0.015, 0.015, 0.015]);
    /// Concrete.
    pub const CONCRETE: Self = Self::new([0.05, 0.07, 0.08], 0.05, [0.015, 0.002, 0.001]);
    /// Ceramic tiles.
    pub const CERAMIC: Self = Self::new([0.01, 0.02, 0.02], 0.05, [0.060, 0.044, 0.011]);
    /// Gravel.
    pub const GRAVEL: Self = Self::new([0.60, 0.70, 0.80], 0.05, [0.031, 0.012, 0.008]);
    /// Carpet.
    pub const CARPET: Self = Self::new([0.24, 0.69, 0.73], 0.05, [0.020, 0.005, 0.003]);
    /// Glass.
    pub const GLASS: Self = Self::new([0.06, 0.03, 0.02], 0.05, [0.060, 0.044, 0.011]);
    /// Plaster.
    pub const PLASTER: Self = Self::new([0.12, 0.06, 0.04], 0.05, [0.056, 0.056, 0.004]);
    /// Wood.
    pub const WOOD: Self = Self::new([0.11, 0.07, 0.06], 0.05, [0.070, 0.014, 0.005]);
    /// Metal.
    pub const METAL: Self = Self::new([0.20, 0.07, 0.06], 0.05, [0.200, 0.025, 0.010]);
    /// Rock.
    pub const ROCK: Self = Self::new([0.13, 0.20, 0.24], 0.05, [0.015, 0.002, 0.001]);

    /// The presets by name, as Steam Audio names them.
    pub const PRESETS: [(&'static str, AcousticMaterial); 11] = [
        ("generic", Self::GENERIC),
        ("brick", Self::BRICK),
        ("concrete", Self::CONCRETE),
        ("ceramic", Self::CERAMIC),
        ("gravel", Self::GRAVEL),
        ("carpet", Self::CARPET),
        ("glass", Self::GLASS),
        ("plaster", Self::PLASTER),
        ("wood", Self::WOOD),
        ("metal", Self::METAL),
        ("rock", Self::ROCK),
    ];

    /// The preset named `name`.
    pub fn preset(name: &str) -> Option<AcousticMaterial> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, m)| m)
    }
}

/// A voice's place in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    /// Where, in the world.
    pub position: DVec3,
    /// Metres a second (for Doppler).
    pub velocity: Vec3,
    /// The distance curve.
    pub attenuation: Attenuation,
    /// Air absorption per metre in each band ([`AIR_ABSORPTION`]; zeros for none).
    pub air: [f32; 3],
    /// What blocks the direct path.
    pub occlusion: Occlusion,
    /// How much of the physical Doppler shift to apply (0: none, 1: physical).
    pub doppler: f32,
    /// The source's radius in metres: inside it the sound comes from all around (the pan
    /// goes to the centre).
    pub spread: f32,
}

impl Spatial {
    /// A still source at `position` with the default curve, air absorption and Doppler.
    pub fn at(position: DVec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            attenuation: Attenuation::default(),
            air: AIR_ABSORPTION,
            occlusion: Occlusion::NONE,
            doppler: 1.0,
            spread: 0.5,
        }
    }
}

/// The direct path from a voice to the listener for one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectPath {
    /// Unit direction to the source in the listener's frame (+X right, +Y up, −Z ahead);
    /// zero when the source is at the listener.
    pub direction: Vec3,
    /// Metres.
    pub distance: f32,
    /// The voice's gain times the distance curve's.
    pub gain: f32,
    /// The source's radius ([`Spatial::spread`]).
    pub spread: f32,
}

impl DirectPath {
    /// Silence from straight ahead.
    pub const SILENT: DirectPath = DirectPath {
        direction: Vec3::NEG_Z,
        distance: 0.0,
        gain: 0.0,
        spread: 0.0,
    };
}

/// What the mixer computes for a spatial voice each block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub path: DirectPath,
    /// Air absorption and occlusion in each band.
    pub bands: [f32; 3],
    /// The Doppler factor on the playback rate.
    pub doppler: f32,
    /// How loud the voice is at the listener, for the voice budget.
    pub audibility: f32,
}

/// Places a voice of gain `gain` for `listener`.
pub(crate) fn place(
    listener: &Listener,
    spatial: &Spatial,
    gain: f32,
    speed_of_sound: f32,
) -> Placement {
    let offset = (spatial.position - listener.position).as_vec3();
    let distance = offset.length();
    let direction = if distance > 1e-6 {
        listener.to_local(offset / distance)
    } else {
        Vec3::ZERO
    };
    let curve = spatial.attenuation.gain(distance);
    let occlusion = spatial.occlusion.bands();
    let mut bands = [0.0; 3];
    for (b, band) in bands.iter_mut().enumerate() {
        *band = (-spatial.air[b] * distance).exp() * occlusion[b];
    }

    // Doppler: f' = f (c − v_l·u) / (c − v_s·u), u the unit vector from source to listener.
    let doppler = if spatial.doppler > 0.0 && distance > 1e-6 {
        let u = -offset / distance;
        let c = speed_of_sound;
        let heard = (c - spatial.doppler * listener.velocity.dot(u)).max(0.1 * c);
        let sent = (c - spatial.doppler * spatial.velocity.dot(u)).max(0.1 * c);
        (heard / sent).clamp(0.25, 4.0)
    } else {
        1.0
    };

    let path = DirectPath {
        direction,
        distance,
        gain: gain * curve,
        spread: spatial.spread,
    };
    Placement {
        path,
        bands,
        doppler,
        audibility: path.gain * bands[0].max(bands[1]).max(bands[2]),
    }
}

/// Renders a spatial voice's mono block into the output's two channels.
///
/// The mixer calls it once a block for each spatial voice that sounds, after the distance
/// curve, air absorption and occlusion have been applied, with the path at the start and the
/// end of the block (to ramp between them). A slot is a voice's place among the mixer's
/// voices: an HRTF renderer keeps its convolution history per slot, and `reset` tells it a
/// slot now plays another voice.
pub trait Spatializer: Send {
    /// Adds `input` rendered for `path` into `left` and `right` (all of one length), moving
    /// from `previous` at the first frame to `path` at the last.
    fn render(
        &mut self,
        slot: usize,
        previous: &DirectPath,
        path: &DirectPath,
        input: &[f32],
        left: &mut [f32],
        right: &mut [f32],
    );

    /// Slot `slot` starts a new voice.
    fn reset(&mut self, slot: usize);
}

/// Equal-power panning by the direction's lateral part: `left = cos θ`, `right = sin θ`
/// with `θ = (1 + x) π/4`, `x` the direction's rightward part, pulled to the centre inside
/// the source's spread. A source straight ahead, behind, above or below gets −3 dB in each
/// channel, as does a non-spatial mono sound at the centre.
#[derive(Clone, Copy, Debug, Default)]
pub struct PanSpatializer;

impl PanSpatializer {
    /// The two channels' gains for a path.
    pub fn gains(path: &DirectPath) -> [f32; 2] {
        let lateral = if path.distance > 0.0 {
            path.direction.x * (path.distance / path.distance.max(path.spread))
        } else {
            0.0
        };
        let [l, r] = pan_gains(lateral);
        [l * path.gain, r * path.gain]
    }
}

impl Spatializer for PanSpatializer {
    fn render(
        &mut self,
        _slot: usize,
        previous: &DirectPath,
        path: &DirectPath,
        input: &[f32],
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let from = Self::gains(previous);
        let to = Self::gains(path);
        ramp_add(input, from, to, left, right);
    }

    fn reset(&mut self, _slot: usize) {}
}

/// Equal-power gains for a pan from −1 (left) to 1 (right).
pub fn pan_gains(pan: f32) -> [f32; 2] {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [theta.cos(), theta.sin()]
}

/// Adds a mono block to two channels with gains moving linearly from `from` to `to`.
pub(crate) fn ramp_add(
    input: &[f32],
    from: [f32; 2],
    to: [f32; 2],
    left: &mut [f32],
    right: &mut [f32],
) {
    let n = input.len();
    if from == to {
        for i in 0..n {
            left[i] += input[i] * to[0];
            right[i] += input[i] * to[1];
        }
        return;
    }
    let step = [(to[0] - from[0]) / n as f32, (to[1] - from[1]) / n as f32];
    for i in 0..n {
        let t = (i + 1) as f32;
        left[i] += input[i] * (from[0] + step[0] * t);
        right[i] += input[i] * (from[1] + step[1] * t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listener_frame_follows_the_engine_convention() {
        let listener = Listener::default();
        assert_eq!(listener.right(), Vec3::X);
        let local = listener.to_local(Vec3::new(1.0, 2.0, -3.0));
        assert!((local - Vec3::new(1.0, 2.0, -3.0)).length() < 1e-6);
        // Turned to face +X: the world's −Z is now on the left.
        let turned = Listener {
            forward: Vec3::X,
            ..Listener::default()
        };
        let local = turned.to_local(Vec3::NEG_Z);
        assert!((local - Vec3::NEG_X).length() < 1e-6, "{local}");
    }

    #[test]
    fn inverse_law_drops_six_db_a_doubling() {
        let a = Attenuation::default();
        assert_eq!(a.gain(0.5), 1.0);
        let ratio = a.gain(20.0) / a.gain(10.0);
        assert!((20.0 * ratio.log10() + 6.02).abs() < 0.01);
        assert_eq!(Attenuation::Linear { min: 1.0, max: 3.0 }.gain(2.0), 0.5);
    }

    #[test]
    fn doppler_raises_an_approaching_source() {
        let listener = Listener::default();
        let mut s = Spatial::at(DVec3::new(0.0, 0.0, -100.0));
        s.velocity = Vec3::new(0.0, 0.0, 34.3);
        let p = place(&listener, &s, 1.0, SPEED_OF_SOUND);
        assert!((p.doppler - 1.0 / 0.9).abs() < 1e-4, "{}", p.doppler);
        s.velocity = -s.velocity;
        let p = place(&listener, &s, 1.0, SPEED_OF_SOUND);
        assert!((p.doppler - 1.0 / 1.1).abs() < 1e-4);
    }

    #[test]
    fn panning_is_equal_power() {
        for pan in [-1.0, -0.3, 0.0, 0.5, 1.0] {
            let [l, r] = pan_gains(pan);
            assert!((l * l + r * r - 1.0).abs() < 1e-6);
        }
        assert!(pan_gains(-1.0)[1].abs() < 1e-6);
    }
}
