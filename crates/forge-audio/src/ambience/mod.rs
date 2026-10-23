//! Ambience: beds of sound the simulation drives, by the hour, the weather and the medium
//! (issue #267; D-011's "rain, wind and water textures driven by the weather fields").
//!
//! Six beds play at once as streams ([`beds`]): wind, rain, cicadas, birds, the sea against
//! the quays, and under water. Each tick the game hands [`Ambience::update`] the state of the
//! world where the listener is ([`AmbienceState`]: the hour, the sun's times, the
//! temperature, the wind, the rain, the nearest shore, the medium), and the rules
//! ([`targets`]) turn it into each bed's gain, intensity and pan:
//! - **cicadas** sing in the heat of the day: from 2.5 to 4.5 hours after sunrise until
//!   sunset, above 22 °C (all of them by 27 °C); they stop within two seconds of rain, stay
//!   quiet while the place is wet (two minutes after the rain, or what the simulation says),
//!   and give way to the wind from 5 m/s (silent by 10 m/s);
//! - **birds** sing the dawn chorus from an hour before sunrise to 2.5 hours after, call a
//!   little through the day and again at dusk, and are quiet at night, in rain and in wind;
//! - **the place** ([`Habitat`], #350): the cicadas fall silent among houses with few trees, the
//!   birds thin to a third there, and indoors the walls take the outside's beds (−16 dB well
//!   inside); the birds' species and their shares are the scene's ([`BirdMix`]);
//! - **wind** and **rain** follow their speed and rate;
//! - **the sea** falls 3 dB each time the distance to the shore doubles past 4 m (a line of
//!   quay is a line source), is panned toward the water, and swells with the wind;
//! - **under water**, the world above the surface is filtered by its bus
//!   ([`crate::dsp::BusFilter::UNDERWATER`]: low-passed at 600 Hz, 29 dB down, a fifth of
//!   its width) and the underwater bed plays through its own reverb.
//!
//! The gains move toward their targets with time constants per bed, one rising and one
//! falling (the wind 1 s; the cicadas rise over 3 s and fall over 0.8 s; the medium 60 ms), so
//! a change in the simulation crossfades;
//! a bed whose gain has rested at 0 for a second sleeps and costs nothing.

pub mod beds;
pub mod recordings;

use std::sync::Arc;

use glam::Vec3;

pub use beds::{Bed, BedControl, BedKind, BirdMix, BirdVoice, Texture};
pub use recordings::{BirdCalls, CicadaSong, Species};

use crate::decode::DecodeError;
use crate::dsp::{BusFilter, ReverbParams};
use crate::foley::smoothstep;
use crate::spatial::Listener;
use crate::stream::Streams;
use crate::{AudioEngine, BusId, BusLayout, Control, PlayParams, VoiceId};

/// What the listener's head is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Medium {
    /// Air.
    Air,
    /// Water.
    Water,
}

impl Medium {
    /// The filter on the world's bus in this medium.
    pub fn filter(self) -> BusFilter {
        match self {
            Medium::Air => BusFilter::CLEAR,
            Medium::Water => BusFilter::UNDERWATER,
        }
    }
}

/// The nearest shore: the edge of the quay or the beach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shore {
    /// Metres from the listener.
    pub distance: f32,
    /// The direction toward the water, in the world.
    pub direction: Vec3,
}

/// The place round the listener (#350): what lives there, and whether walls stand between the
/// listener and the outside. The default is open country with trees, as the beds were made for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Habitat {
    /// How green the place is within earshot, 0 (streets and houses only) to 1 (gardens and
    /// woods): the share of the ground under trees, hedges and gardens, as the scene measures it.
    /// Cicadas sing from trees and fall silent among houses with few of them; birds thin out but
    /// stay (sparrows live on the houses); the dove calls only near woods.
    pub green: f32,
    /// 0 outdoors to 1 well inside a building: the walls take most of the outside's beds.
    pub indoors: f32,
}

impl Default for Habitat {
    fn default() -> Self {
        Self {
            green: 1.0,
            indoors: 0.0,
        }
    }
}

impl Habitat {
    /// The cicadas' share where the listener stands: none among houses (green under 0.1), all of
    /// them from a garden's worth of trees (0.5).
    pub fn cicadas(&self) -> f32 {
        smoothstep(0.1, 0.5, self.green)
    }

    /// The birds' share: a third among houses, all of them from 0.4 of green.
    pub fn birds(&self) -> f32 {
        1.0 - 0.65 * (1.0 - smoothstep(0.0, 0.4, self.green))
    }

    /// The share of [`BirdMix::dove`] that calls here: none among houses (green under 0.15), all
    /// of it in a wood (0.5). The owner wanted the coo now and then from a tree near the forest,
    /// not in every street (#350).
    pub fn doves(&self) -> f32 {
        smoothstep(0.15, 0.5, self.green)
    }

    /// What reaches the listener of the beds outside, through the walls: 1 outdoors, 0.15
    /// (about −16 dB) well inside.
    pub fn outside(&self) -> f32 {
        1.0 - 0.85 * self.indoors.clamp(0.0, 1.0)
    }
}

/// The world where the listener is, as the simulation knows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbienceState {
    /// Local solar time, hours (0 to 24).
    pub hour: f32,
    /// Sunrise, hours.
    pub sunrise: f32,
    /// Sunset, hours.
    pub sunset: f32,
    /// Air temperature, °C.
    pub temperature: f32,
    /// Wind speed, m/s.
    pub wind: f32,
    /// Rain, mm/h.
    pub rain: f32,
    /// The nearest shore, if the sea is within earshot.
    pub shore: Option<Shore>,
    /// What the listener's head is in.
    pub medium: Medium,
    /// The listener (its orientation pans the sea).
    pub listener: Listener,
    /// How wet the place still is after rain, 0 (dry) to 1. [`Ambience`] also remembers the
    /// rain itself (two minutes to dry); the wetter of the two counts.
    pub wet: f32,
    /// The place round the listener (#350).
    pub habitat: Habitat,
}

impl Default for AmbienceState {
    /// Noon on a calm, dry summer day in a Mediterranean village at 43° N: the sun up at
    /// 6:12 and down at 20:36, 28 °C.
    fn default() -> Self {
        Self {
            hour: 12.0,
            sunrise: 6.2,
            sunset: 20.6,
            temperature: 28.0,
            wind: 0.0,
            rain: 0.0,
            shore: None,
            medium: Medium::Air,
            listener: Listener::default(),
            wet: 0.0,
            habitat: Habitat::default(),
        }
    }
}

/// What the rules want of a bed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BedTarget {
    /// 0 to 1 (times the bed's level).
    pub gain: f32,
    /// The bed's intensity (its unit is the bed's).
    pub intensity: f32,
    /// −1 left to 1 right.
    pub pan: f32,
}

/// Each bed's level at a gain of 1, so that wind at 10 m/s, rain at 10 mm/h, the full
/// cicada chorus and the sea at 4 m play about −24 dBFS RMS, the underwater rumble −22 and
/// the dawn chorus −30 (`tests/ambience.rs` measures them).
pub const LEVELS: [f32; 6] = [0.375, 1.33, 1.0, 0.33, 1.8, 1.0];

/// Seconds each bed's gain takes to rise by `e` toward its target.
pub const RISE: [f32; 6] = [1.0, 2.0, 3.0, 3.0, 0.5, 0.06];

/// Seconds each bed's gain takes to fall by `e` toward its target: singers stop sooner than
/// they start.
pub const FALL: [f32; 6] = [1.0, 2.0, 0.8, 1.5, 0.5, 0.06];

/// Seconds the place takes to dry by `e` after the rain (in [`Ambience`]'s memory).
pub const DRYING: f32 = 120.0;

/// The rules: what each bed should do in `state` (in [`BedKind::ALL`]'s order).
pub fn targets(state: &AmbienceState) -> [BedTarget; 6] {
    let h = state.hour.rem_euclid(24.0);
    let (rise, set) = (state.sunrise, state.sunset);
    let rain = state.rain.max(0.0);
    let wind = state.wind.max(0.0);
    let window = |a: f32, b: f32, c: f32, d: f32| smoothstep(a, b, h) * (1.0 - smoothstep(c, d, h));

    let cicadas = window(rise + 2.5, rise + 4.5, set - 1.5, set)
        * smoothstep(22.0, 27.0, state.temperature)
        * (1.0 - smoothstep(0.2, 1.0, rain))
        * (1.0 - smoothstep(0.1, 0.5, state.wet))
        * (1.0 - smoothstep(5.0, 10.0, wind))
        * state.habitat.cicadas();
    let dawn = window(rise - 1.0, rise, rise + 0.5, rise + 2.5);
    let day = 0.15 * window(rise + 1.0, rise + 2.5, set - 1.0, set);
    let dusk = 0.4 * window(set - 1.5, set - 0.5, set, set + 0.75);
    let birds = dawn.max(day).max(dusk)
        * (1.0 - smoothstep(1.0, 5.0, rain))
        * (1.0 - smoothstep(6.0, 12.0, wind))
        * state.habitat.birds();

    let sea = state.shore.map_or(BedTarget::default(), |shore| {
        let d = shore.distance.max(0.0);
        let local = state.listener.to_local(shore.direction.normalize_or_zero());
        BedTarget {
            gain: (4.0 / d.max(4.0)).sqrt() * (1.0 - smoothstep(200.0, 400.0, d)),
            intensity: (0.25 + 0.06 * wind).clamp(0.0, 1.0),
            pan: 0.7 * local.x,
        }
    });
    let under = if state.medium == Medium::Water {
        1.0
    } else {
        0.0
    };
    // Indoors the walls take the beds of the outside (all but the one under water).
    let outside = state.habitat.outside();
    let mut beds = [
        BedTarget {
            gain: smoothstep(0.3, 1.5, wind),
            intensity: wind,
            pan: 0.0,
        },
        BedTarget {
            gain: smoothstep(0.02, 0.3, rain),
            intensity: rain,
            pan: 0.0,
        },
        BedTarget {
            gain: smoothstep(0.0, 0.05, cicadas),
            intensity: cicadas,
            pan: 0.0,
        },
        BedTarget {
            gain: smoothstep(0.0, 0.05, birds),
            intensity: birds,
            pan: 0.0,
        },
        sea,
        BedTarget {
            gain: under,
            intensity: 1.0,
            pan: 0.0,
        },
    ];
    for bed in &mut beds[..5] {
        bed.gain *= outside;
    }
    beds
}

/// What the beds can play besides what they make in code, and the birds' species.
#[derive(Clone, Debug, Default)]
pub struct AmbienceAssets {
    /// Recorded birds ([`BirdCalls::load`]): the birds bed sings them when they are here.
    pub birds: Option<Arc<BirdCalls>>,
    /// Recorded cicadas ([`CicadaSong::load`]): the cicada bed layers them into its chorus.
    pub cicadas: Option<Arc<CicadaSong>>,
    /// The birds' species and how often each calls (#350).
    pub bird_mix: BirdMix,
}

impl AmbienceAssets {
    /// The recordings `tools/fetch-sounds.sh` fetched into `dir` (`assets/sounds/`), for a
    /// mixer at `rate`: what is there, nothing when nothing is.
    pub fn load(dir: impl AsRef<std::path::Path>, rate: u32) -> Result<Self, DecodeError> {
        let birds = BirdCalls::load(dir.as_ref().join("birds"), rate)?;
        let cicadas = CicadaSong::load(dir.as_ref().join("cicadas"), rate)?;
        Ok(Self {
            birds: birds.map(Arc::new),
            cicadas: cicadas.map(Arc::new),
            bird_mix: BirdMix::default(),
        })
    }
}

/// The buses the ambience plays through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmbienceBuses {
    /// Everything heard above the water, with the medium's filter: the game's other buses
    /// (effects, footsteps, their reverb) go under it.
    pub world: BusId,
    /// The beds above the water (under `world`).
    pub beds: BusId,
    /// The underwater reverb (under the master).
    pub water_reverb: BusId,
    /// Sounds under the water, the underwater bed among them (under the master, sending to
    /// `water_reverb`).
    pub water: BusId,
}

impl AmbienceBuses {
    /// Adds the ambience's buses to `layout`.
    pub fn add(layout: &mut BusLayout) -> Self {
        let world = layout.add("world", BusId::MASTER);
        layout.set_filter(world, BusFilter::CLEAR);
        let beds = layout.add("beds", world);
        let water_reverb =
            layout.add_reverb("water reverb", BusId::MASTER, ReverbParams::UNDERWATER);
        let water = layout.add("water", BusId::MASTER);
        layout.send(water, water_reverb, 0.5);
        Self {
            world,
            beds,
            water_reverb,
            water,
        }
    }
}

/// A bed playing.
struct Playing {
    kind: BedKind,
    voice: VoiceId,
    control: Arc<BedControl>,
    gain: f32,
    sent: f32,
    pan: f32,
    quiet: f32,
}

/// The ambience: its beds playing and the rules driving them.
pub struct Ambience {
    beds: Vec<Playing>,
    buses: AmbienceBuses,
    medium: Medium,
    /// How wet the rain has left the place, 0 to 1.
    wet: f32,
}

impl Ambience {
    /// Starts the beds for `state`, varied from `seed`, streamed through `streams`, every
    /// sound made in code.
    pub fn start(
        engine: &mut AudioEngine,
        streams: &Streams,
        buses: AmbienceBuses,
        seed: u64,
        state: &AmbienceState,
    ) -> Result<Self, DecodeError> {
        Self::start_with(
            engine,
            streams,
            buses,
            seed,
            state,
            &AmbienceAssets::default(),
        )
    }

    /// Starts the beds, playing `assets` where they have something.
    pub fn start_with(
        engine: &mut AudioEngine,
        streams: &Streams,
        buses: AmbienceBuses,
        seed: u64,
        state: &AmbienceState,
        assets: &AmbienceAssets,
    ) -> Result<Self, DecodeError> {
        let rate = engine.sample_rate();
        let mut beds = Vec::with_capacity(BedKind::ALL.len());
        for (i, (kind, target)) in BedKind::ALL.into_iter().zip(targets(state)).enumerate() {
            let (bed, control) = Bed::new_with(
                kind,
                seed ^ (0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1)),
                rate,
                target.intensity,
                assets,
            );
            control.set_awake(target.gain > 0.0);
            control.set_green(state.habitat.green);
            let stream = streams.open(Box::new(bed), false)?;
            let voice = engine.play_stream(
                stream,
                PlayParams {
                    bus: if kind == BedKind::Underwater {
                        buses.water
                    } else {
                        buses.beds
                    },
                    gain: target.gain * LEVELS[i],
                    pan: target.pan,
                    priority: 255,
                    ..PlayParams::default()
                },
            );
            beds.push(Playing {
                kind,
                voice,
                control,
                gain: target.gain,
                sent: target.gain * LEVELS[i],
                pan: target.pan,
                quiet: 0.0,
            });
        }
        engine.control(Control::BusFilter {
            bus: buses.world,
            filter: state.medium.filter(),
        });
        Ok(Self {
            beds,
            buses,
            medium: state.medium,
            wet: state.wet,
        })
    }

    /// Moves every bed toward what `state` wants, `dt` seconds after the last call.
    pub fn update(&mut self, engine: &mut AudioEngine, state: &AmbienceState, dt: f32) {
        // Rain wets the place within seconds; it dries over minutes.
        let dt = dt.max(0.0);
        if state.rain > 0.2 {
            self.wet += (1.0 - self.wet) * (1.0 - (-dt / 3.0).exp());
        } else {
            self.wet *= (-dt / DRYING).exp();
        }
        let state = &AmbienceState {
            wet: state.wet.max(self.wet),
            ..*state
        };
        for (i, (bed, target)) in self.beds.iter_mut().zip(targets(state)).enumerate() {
            let tau = if target.gain < bed.gain {
                FALL[i]
            } else {
                RISE[i]
            };
            let k = 1.0 - (-dt / tau).exp();
            bed.gain += (target.gain - bed.gain) * k;
            if (bed.gain - target.gain).abs() < 1e-4 {
                bed.gain = target.gain;
            }
            bed.control.set_intensity(target.intensity);
            bed.control.set_green(state.habitat.green);
            // A bed resting at 0 for a second sleeps; wanted again, it wakes.
            if bed.gain == 0.0 && target.gain == 0.0 {
                bed.quiet += dt;
            } else {
                bed.quiet = 0.0;
            }
            bed.control.set_awake(bed.quiet < 1.0);
            let gain = bed.gain * LEVELS[i];
            if (gain - bed.sent).abs() > 0.002 * LEVELS[i] || (gain == 0.0 && bed.sent != 0.0) {
                engine.control(Control::Gain {
                    voice: bed.voice,
                    gain,
                });
                bed.sent = gain;
            }
            if (target.pan - bed.pan).abs() > 0.01 {
                engine.control(Control::Pan {
                    voice: bed.voice,
                    pan: target.pan,
                });
                bed.pan = target.pan;
            }
        }
        if state.medium != self.medium {
            self.medium = state.medium;
            engine.control(Control::BusFilter {
                bus: self.buses.world,
                filter: state.medium.filter(),
            });
        }
    }

    /// A bed's gain now, 0 to 1 (before its level).
    pub fn gain(&self, kind: BedKind) -> f32 {
        self.beds
            .iter()
            .find(|b| b.kind == kind)
            .map_or(0.0, |b| b.gain)
    }

    /// Whether a bed is working.
    pub fn awake(&self, kind: BedKind) -> bool {
        self.beds
            .iter()
            .find(|b| b.kind == kind)
            .is_some_and(|b| b.control.awake())
    }

    /// How wet the place is, as the ambience remembers the rain (0 to 1).
    pub fn wet(&self) -> f32 {
        self.wet
    }

    /// Its buses.
    pub fn buses(&self) -> AmbienceBuses {
        self.buses
    }
}
