//! Birds perched in the world (#357): each calls now and then from where it sits, a voice placed
//! there and heard by its distance, where the birds' bed ([`super::beds::Birds`]) sings from
//! nowhere in particular. The owner, flying in over the village: the doves should call "from time
//! to time" and "match their physical presence", not be heard from high in the air.
//!
//! A scene gives the perches (a dove in a tree's crown near the woods). Each bird calls in bouts,
//! two to four calls [`PHRASE`] apart, then keeps quiet for [`BOUT`], longer at the edges of its
//! hours ([`dove_hours`]). Only the birds within [`EARSHOT`] of the listener start a call; the
//! others keep their time.

use std::sync::Arc;

use forge_core::SplitMix64;
use glam::DVec3;

use super::AmbienceState;
use super::beds::{BirdVoice, bird_call};
use crate::foley::smoothstep;
use crate::spatial::Attenuation;
use crate::{AudioEngine, BusId, PlayParams, SampleBuffer};

/// Metres from the listener within which a perched bird's call starts.
pub const EARSHOT: f64 = 150.0;

/// Seconds between the calls of a bout: from, to.
pub const PHRASE: (f32, f32) = (2.2, 3.4);

/// Seconds of quiet between a bird's bouts in its hours: from, to.
pub const BOUT: (f32, f32) = (45.0, 150.0);

/// A call's gain within [`REFERENCE`] metres: a dove 30 m off plays about as the bed's dove did.
pub const LEVEL: f32 = 0.5;

/// Metres within which a call keeps its [`LEVEL`]; past them it falls 6 dB each time the distance
/// doubles (and the air takes its highs).
pub const REFERENCE: f32 = 4.0;

/// The calls made ahead, which each call picks from.
const VARIANTS: usize = 6;

/// The share of a dove's calls in the hour, wind and rain of `state`, 0 to 1: from half an hour
/// before sunrise to a quarter of an hour after sunset, quiet at night, in rain and in a strong
/// wind (as the birds' bed).
pub fn dove_hours(state: &AmbienceState) -> f32 {
    let h = state.hour.rem_euclid(24.0);
    let day = smoothstep(state.sunrise - 0.5, state.sunrise + 0.5, h)
        * (1.0 - smoothstep(state.sunset - 0.5, state.sunset + 0.25, h));
    day * (1.0 - smoothstep(1.0, 5.0, state.rain.max(0.0)))
        * (1.0 - smoothstep(6.0, 12.0, state.wind.max(0.0)))
}

/// One bird on its perch.
struct Bird {
    at: DVec3,
    /// The time of its next call, seconds.
    next: f64,
    /// The calls left in its bout after the last.
    left: u8,
}

/// Birds of one voice perched in the world, calling now and then.
pub struct Perched {
    birds: Vec<Bird>,
    calls: Vec<Arc<SampleBuffer>>,
    rng: SplitMix64,
    now: f64,
    started: u64,
}

impl Perched {
    /// Collared doves on `perches` (in the listener's frame), varied from `seed`, for a mixer at
    /// `rate`.
    pub fn doves(perches: &[DVec3], seed: u64, rate: u32) -> Self {
        Self::new(BirdVoice::Dove, perches, seed, rate)
    }

    /// Birds of `voice` on `perches`.
    pub fn new(voice: BirdVoice, perches: &[DVec3], seed: u64, rate: u32) -> Self {
        let mut rng = SplitMix64::new(seed);
        let calls = (0..VARIANTS as u64)
            .map(|k| {
                let seed = seed ^ (k + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                Arc::new(bird_call(voice, seed, rate))
            })
            .collect();
        // Each first calls somewhere within a bout's quiet, not all at once.
        let birds = perches
            .iter()
            .map(|&at| Bird {
                at,
                next: f64::from(rng.range_f32(0.0, BOUT.1)),
                left: 0,
            })
            .collect();
        Self {
            birds,
            calls,
            rng,
            now: 0.0,
            started: 0,
        }
    }

    /// How many birds.
    pub fn len(&self) -> usize {
        self.birds.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.birds.is_empty()
    }

    /// The calls started so far.
    pub fn started(&self) -> u64 {
        self.started
    }

    /// Moves time on by `dt` seconds: the birds whose turn it is call into `bus`, placed where
    /// they sit, those within [`EARSHOT`] of `state`'s listener in their hours, through the walls
    /// when the listener is indoors ([`super::Habitat::outside`]).
    pub fn update(&mut self, engine: &mut AudioEngine, state: &AmbienceState, bus: BusId, dt: f32) {
        let Self {
            birds,
            calls,
            rng,
            now,
            started,
        } = self;
        *now += f64::from(dt.max(0.0));
        let hours = dove_hours(state);
        let listener = state.listener.position;
        let outside = state.habitat.outside();
        for bird in birds.iter_mut() {
            if *now < bird.next {
                continue;
            }
            let heard = bird.at.distance(listener) <= EARSHOT;
            if hours < 0.05 || !heard {
                // Out of its hours or of earshot: its bout ends, its time goes on.
                bird.left = 0;
                bird.next = *now + f64::from(rng.range_f32(BOUT.0, BOUT.1));
                continue;
            }
            let call = &calls[rng.below(calls.len() as u64) as usize];
            let mut params = PlayParams::at(bird.at);
            params.bus = bus;
            params.gain = LEVEL * outside;
            params.pitch = rng.range_f32(0.97, 1.03);
            params.priority = 64;
            if let Some(spatial) = &mut params.spatial {
                spatial.attenuation = Attenuation::Inverse {
                    reference: REFERENCE,
                    max: 400.0,
                    rolloff: 1.0,
                };
            }
            engine.play(call, params);
            *started += 1;
            // A bout's first call sets how many follow (one to three more).
            bird.left = if bird.left == 0 {
                1 + rng.below(3) as u8
            } else {
                bird.left - 1
            };
            bird.next = *now
                + f64::from(if bird.left > 0 {
                    rng.range_f32(PHRASE.0, PHRASE.1)
                } else {
                    rng.range_f32(BOUT.0, BOUT.1) / hours.max(0.25)
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambience::Habitat;
    use crate::{BusLayout, Listener, MixerConfig};

    fn engine() -> (AudioEngine, crate::Mixer) {
        crate::create(MixerConfig::default(), &BusLayout::new())
    }

    fn at_noon(listener: DVec3) -> AmbienceState {
        AmbienceState {
            hour: 13.0,
            listener: Listener {
                position: listener,
                ..Listener::default()
            },
            ..AmbienceState::default()
        }
    }

    /// Ten minutes a frame at a time: the calls started.
    fn calls(doves: &mut Perched, state: &AmbienceState) -> u64 {
        let (mut engine, mut mixer) = engine();
        let mut out = vec![0.0; 2 * 800];
        let before = doves.started();
        for _ in 0..36_000 {
            doves.update(&mut engine, state, BusId::MASTER, 1.0 / 60.0);
            engine.update();
            mixer.render(&mut out);
        }
        doves.started() - before
    }

    /// A dove's call is its three coos, a second and a half to three seconds long.
    #[test]
    fn a_dove_s_call_is_its_coos() {
        let call = bird_call(BirdVoice::Dove, 7, 48_000);
        let seconds = call.seconds();
        assert!((1.5..3.0).contains(&seconds), "{seconds:.2} s");
        let peak = call.samples().iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.1 && peak <= 0.4, "{peak}");
    }

    /// A dove near the listener calls a few bouts in ten minutes, two to four calls each; one
    /// past earshot, or any at night, never calls.
    #[test]
    fn doves_call_now_and_then_within_earshot_and_by_day() {
        let near = calls(
            &mut Perched::doves(&[DVec3::new(20.0, 8.0, 0.0)], 3, 48_000),
            &at_noon(DVec3::ZERO),
        );
        // Ten minutes: four to thirteen bouts of 45 to 150 s, two to four calls each.
        assert!((6..=50).contains(&near), "{near} calls");
        let far = calls(
            &mut Perched::doves(&[DVec3::new(EARSHOT + 10.0, 8.0, 0.0)], 3, 48_000),
            &at_noon(DVec3::ZERO),
        );
        assert_eq!(far, 0);
        let night = AmbienceState {
            hour: 2.0,
            ..at_noon(DVec3::ZERO)
        };
        let dark = calls(
            &mut Perched::doves(&[DVec3::new(20.0, 8.0, 0.0)], 3, 48_000),
            &night,
        );
        assert_eq!(dark, 0);
        // Indoors they still call, through the walls.
        let inside = AmbienceState {
            habitat: Habitat {
                indoors: 1.0,
                ..Habitat::default()
            },
            ..at_noon(DVec3::ZERO)
        };
        let walls = calls(
            &mut Perched::doves(&[DVec3::new(20.0, 8.0, 0.0)], 3, 48_000),
            &inside,
        );
        assert!(walls > 0);
    }

    /// Many doves don't call at once: their first calls spread over a bout's quiet.
    #[test]
    fn a_flock_s_first_calls_spread_out() {
        let perches: Vec<DVec3> = (0..40)
            .map(|k| DVec3::new(f64::from(k) * 2.0, 8.0, 0.0))
            .collect();
        let mut doves = Perched::doves(&perches, 11, 48_000);
        let (mut engine, _mixer) = engine();
        let state = at_noon(DVec3::ZERO);
        // The first ten seconds: a few of the forty, not all.
        for _ in 0..600 {
            doves.update(&mut engine, &state, BusId::MASTER, 1.0 / 60.0);
            engine.update();
        }
        assert!(doves.started() < 15, "{}", doves.started());
    }
}
