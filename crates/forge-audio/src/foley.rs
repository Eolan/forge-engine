//! Footsteps and impacts, made in code for the surface they land on (D-011: "impacts and
//! footsteps from modal banks fitted per material"; D-007's material row names the surface).
//!
//! The game thread calls [`Foley::step`] or [`Foley::impact`] with the surface, the place, and
//! how hard: a walker's speed and weight, or a body's speed and mass. The sound is synthesised
//! into a short buffer (0.1 to 1 ms of work) on the engine's maker thread when it runs
//! ([`AudioEngine::make_off_thread`], the app's), started at the next frame, or in the call
//! otherwise, and played as a spatial voice; nothing is recorded or downloaded, and no two steps
//! are the same.
//!
//! A contact is a force pulse, `sin²` over its width ([`synth::force_pulse`]): short on hard
//! ground (0.4 ms on metal), long on soft (12 ms on grass), so a hard surface is excited high
//! and a soft one only low. What the force drives depends on the [`Surface`]:
//! - **modes** (stone, wood, metal): a bank of resonators, each a mode of the slab or the
//!   board, after van den Doel, Kry and Pai (FoleyAutomatic, 2001). Stone setts bedded in sand
//!   ring briefly and high, floorboards low and hollow, a steel plate for a second;
//! - **a click**: a band of noise that follows the contact and dies in a few milliseconds
//!   (the sole's slap);
//! - **a press**: what moves while the foot sinks in, as long as the press lasts: grains (a
//!   pebble ticking against another, a stalk snapping), each a decaying sinusoid at a random
//!   pitch, as many a second as the press is hard, after Cook's stochastic event model of
//!   shakers and gravel (PhISEM, 1997); or a band of noise (grass brushing a shoe, water
//!   sloshing);
//! - **bubbles** (water): Minnaert resonances whose pitch rises as they reach the surface
//!   ([`synth::Grain::bubble`]).
//!
//! A walking step is two contacts, the heel then the softer ball of the foot 85 ms later, and
//! often a scuff as the toes push off; running, the forefoot lands first and harder, 22 ms
//! apart, and every contact is shorter (brighter). The level follows the step's energy: the
//! amplitude is `√(weight × speed)` relative to 80 kg at 1.4 m/s, so doubling the weight adds
//! 3 dB.
//!
//! On hard ground (stone, wood, metal) the [`Footwear`] counts as much as the ground: the
//! contact lasts as long as the softer of sole and ground lets it (0.3 ms for a leather heel on
//! stone, 4 ms for a rubber sneaker, 10 ms for a bare foot), the slap takes the sole's band, a
//! boot thuds, a sandal slaps the heel as the foot lifts. On soft ground the ground decides.
//!
//! No two steps are the same, and the difference is meant to be heard: each draws its own
//! contact length (±25 %), click strength (±3 dB), modes (stone ±8 %), balance of heel and
//! ball, gap between them, scuff or none, and level (±1.5 dB); [`Foley`] also plays each at its
//! own pitch (±2.5 %).

use forge_core::SplitMix64;
use forge_core::material::SoundSurface;
use glam::DVec3;

use crate::dsp::Biquad;
use crate::dsp::synth::{self, Grain, LowPass4, Resonator, force_pulse, log_uniform};
use crate::spatial::{Attenuation, Spatial};
use crate::{AudioEngine, PlayParams, SampleBuffer, VoiceId, db_to_gain};

/// What a foot or a body lands on. The world's material rows (D-007) carry one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Limestone setts or flags bedded in sand: the village's streets.
    Stone,
    /// Loose gravel: paths, the square's edges.
    Gravel,
    /// Dry grass over soil.
    Grass,
    /// Floorboards, a jetty, stairs.
    Wood,
    /// A steel plate, a grate, a hatch.
    Metal,
    /// Shallow water, ankle-deep.
    Water,
}

impl Surface {
    /// Every surface.
    pub const ALL: [Surface; 6] = [
        Surface::Stone,
        Surface::Gravel,
        Surface::Grass,
        Surface::Wood,
        Surface::Metal,
        Surface::Water,
    ];

    /// Its name.
    pub fn name(self) -> &'static str {
        self.model().name
    }

    /// The surface named `name`.
    pub fn from_name(name: &str) -> Option<Surface> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// How it sounds.
    pub fn model(self) -> &'static SurfaceModel {
        match self {
            Surface::Stone => &STONE,
            Surface::Gravel => &GRAVEL,
            Surface::Grass => &GRASS,
            Surface::Wood => &WOOD,
            Surface::Metal => &METAL,
            Surface::Water => &WATER,
        }
    }
}

/// The surface a material row's sound layer names (D-007).
impl From<SoundSurface> for Surface {
    fn from(surface: SoundSurface) -> Self {
        match surface {
            SoundSurface::Stone => Surface::Stone,
            SoundSurface::Gravel => Surface::Gravel,
            SoundSurface::Grass => Surface::Grass,
            SoundSurface::Wood => Surface::Wood,
            SoundSurface::Metal => Surface::Metal,
            SoundSurface::Water => Surface::Water,
        }
    }
}

impl From<Surface> for SoundSurface {
    fn from(surface: Surface) -> Self {
        match surface {
            Surface::Stone => SoundSurface::Stone,
            Surface::Gravel => SoundSurface::Gravel,
            Surface::Grass => SoundSurface::Grass,
            Surface::Wood => SoundSurface::Wood,
            Surface::Metal => SoundSurface::Metal,
            Surface::Water => SoundSurface::Water,
        }
    }
}

/// A mode of a surface: a frequency, how fast it dies, how loud it rings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    /// Hz.
    pub frequency: f32,
    /// The amplitude falls by `e` every `1 / decay` seconds (60 dB in `6.9 / decay`).
    pub decay: f32,
    /// Its share of the sound.
    pub gain: f32,
}

const fn mode(frequency: f32, decay: f32, gain: f32) -> Mode {
    Mode {
        frequency,
        decay,
        gain,
    }
}

/// A band of noise driven by a contact or a press.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseBand {
    /// The band's edges, Hz.
    pub low: f32,
    /// Hz.
    pub high: f32,
    /// After the drive's peak, the level falls by `e` every `1 / decay` seconds.
    pub decay: f32,
    /// Its share of the sound.
    pub gain: f32,
}

/// Grains: short decaying sinusoids at random pitches, as many a second as the press is hard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grains {
    /// Grains a second at the press's peak.
    pub rate: f32,
    /// The pitches' range, Hz (spread evenly in the log domain).
    pub low: f32,
    /// Hz.
    pub high: f32,
    /// The range of their decay rates, 1/s.
    pub decay: [f32; 2],
    /// The loudest grain's share of the sound.
    pub gain: f32,
}

/// Bubbles after each contact (water).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bubbles {
    /// How many a contact, at least and at most.
    pub count: [u32; 2],
    /// Their radii, metres (spread evenly in the log domain).
    pub radius: [f32; 2],
    /// Seconds after the contact over which they rise.
    pub spread: f32,
    /// A 3 mm bubble's share of the sound (larger bubbles are louder).
    pub gain: f32,
}

/// How a surface answers a contact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceModel {
    /// Its name.
    pub name: &'static str,
    /// The peak of a step of 80 kg at 1.4 m/s in hard shoes.
    pub level: f32,
    /// Hard ground, where the sole matters as much as the ground.
    pub hard: bool,
    /// A hard heel's contact, seconds: shorter is harder and brighter.
    pub contact: f32,
    /// How long the foot presses in, seconds (what drives the grains and the body).
    pub press: f32,
    /// Its modes (none for loose or soft ground).
    pub modes: &'static [Mode],
    /// How far each step moves the modes, as a share (no two setts are the same).
    pub jitter: f32,
    /// The low thud of the contact: the foot and the ground under it.
    pub thump: Mode,
    /// The click of the contact.
    pub click: Option<NoiseBand>,
    /// Noise while the foot presses in.
    pub body: Option<NoiseBand>,
    /// Grains while the foot presses in.
    pub grains: Option<Grains>,
    /// Bubbles after each contact.
    pub bubbles: Option<Bubbles>,
}

/// Setts of limestone bedded in sand: modes high and short (the sand damps them), a sharp
/// click, a little grit under the sole.
pub const STONE: SurfaceModel = SurfaceModel {
    name: "stone",
    level: 0.5,
    hard: true,
    contact: 0.00035,
    press: 0.03,
    // A sett bedded in sand is damped within a millisecond: these colour the click, they do
    // not ring.
    modes: &[
        mode(650.0, 900.0, 0.25),
        mode(1400.0, 1100.0, 0.35),
        mode(2600.0, 1400.0, 0.35),
        mode(4200.0, 1800.0, 0.25),
    ],
    jitter: 0.08,
    thump: mode(95.0, 120.0, 0.12),
    click: Some(NoiseBand {
        low: 2000.0,
        high: 9000.0,
        decay: 350.0,
        gain: 0.5,
    }),
    body: None,
    grains: Some(Grains {
        rate: 400.0,
        low: 3000.0,
        high: 8000.0,
        decay: [900.0, 1600.0],
        gain: 0.08,
    }),
    bubbles: None,
};

/// Loose gravel: no modes, a crunch of grains as long as the foot presses, a soft thud.
pub const GRAVEL: SurfaceModel = SurfaceModel {
    name: "gravel",
    level: 0.45,
    hard: false,
    contact: 0.012,
    press: 0.07,
    modes: &[],
    jitter: 0.0,
    thump: mode(80.0, 50.0, 0.35),
    click: None,
    body: Some(NoiseBand {
        low: 600.0,
        high: 5000.0,
        decay: 60.0,
        gain: 0.08,
    }),
    grains: Some(Grains {
        rate: 2400.0,
        low: 1200.0,
        high: 7000.0,
        decay: [250.0, 700.0],
        gain: 0.5,
    }),
    bubbles: None,
};

/// Dry grass over soil: a dull thud, a swish of high noise, a few stalks snapping.
pub const GRASS: SurfaceModel = SurfaceModel {
    name: "grass",
    level: 0.25,
    hard: false,
    contact: 0.025,
    press: 0.09,
    modes: &[],
    jitter: 0.0,
    thump: mode(75.0, 40.0, 0.9),
    click: None,
    body: Some(NoiseBand {
        low: 2500.0,
        high: 8000.0,
        decay: 25.0,
        gain: 0.25,
    }),
    grains: Some(Grains {
        rate: 300.0,
        low: 2000.0,
        high: 6000.0,
        decay: [800.0, 1400.0],
        gain: 0.12,
    }),
    bubbles: None,
};

/// Floorboards: low hollow modes that ring a tenth of a second, a dull click.
pub const WOOD: SurfaceModel = SurfaceModel {
    name: "wood",
    level: 0.5,
    hard: true,
    contact: 0.0012,
    press: 0.03,
    modes: &[
        mode(190.0, 55.0, 0.9),
        mode(420.0, 70.0, 0.7),
        mode(710.0, 90.0, 0.55),
        mode(1080.0, 120.0, 0.4),
        mode(1620.0, 160.0, 0.3),
        mode(2400.0, 220.0, 0.2),
    ],
    jitter: 0.03,
    thump: mode(110.0, 60.0, 0.3),
    click: Some(NoiseBand {
        low: 800.0,
        high: 5000.0,
        decay: 300.0,
        gain: 0.2,
    }),
    body: None,
    grains: None,
    bubbles: None,
};

/// A steel plate: inharmonic modes that ring for a second, a bright click.
pub const METAL: SurfaceModel = SurfaceModel {
    name: "metal",
    level: 0.45,
    hard: true,
    contact: 0.00015,
    press: 0.03,
    modes: &[
        mode(480.0, 9.0, 0.15),
        mode(1150.0, 8.0, 0.3),
        mode(2050.0, 8.0, 0.4),
        mode(2890.0, 9.0, 0.5),
        mode(4120.0, 11.0, 0.55),
        mode(5600.0, 14.0, 0.45),
        mode(7300.0, 18.0, 0.3),
    ],
    jitter: 0.01,
    thump: mode(100.0, 50.0, 0.1),
    click: Some(NoiseBand {
        low: 2000.0,
        high: 10_000.0,
        decay: 400.0,
        gain: 0.2,
    }),
    body: None,
    grains: None,
    bubbles: None,
};

/// Ankle-deep water: a slosh, a splash, bubbles rising after the foot.
pub const WATER: SurfaceModel = SurfaceModel {
    name: "water",
    level: 0.4,
    hard: false,
    contact: 0.03,
    press: 0.12,
    modes: &[],
    jitter: 0.0,
    thump: mode(60.0, 30.0, 0.15),
    click: Some(NoiseBand {
        low: 1000.0,
        high: 6000.0,
        decay: 30.0,
        gain: 0.1,
    }),
    body: Some(NoiseBand {
        low: 150.0,
        high: 1200.0,
        decay: 18.0,
        gain: 0.35,
    }),
    grains: None,
    bubbles: Some(Bubbles {
        count: [8, 20],
        radius: [0.0015, 0.006],
        spread: 0.2,
        gain: 0.25,
    }),
};

/// What a walker wears on its feet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Footwear {
    /// Bare feet: a soft, fleshy pat.
    Barefoot,
    /// Rubber-soled trainers: a dull pat and a scuff.
    Sneakers,
    /// Hard leather soles and heels: a sharp click.
    LeatherShoes,
    /// Heavy boots with thick soles: a thud.
    Boots,
    /// Flip-flops: a soft pat, then the sole slapping the heel.
    Sandals,
}

impl Footwear {
    /// Every kind.
    pub const ALL: [Footwear; 5] = [
        Footwear::Barefoot,
        Footwear::Sneakers,
        Footwear::LeatherShoes,
        Footwear::Boots,
        Footwear::Sandals,
    ];

    /// Its name.
    pub fn name(self) -> &'static str {
        self.model().name
    }

    /// The kind named `name`.
    pub fn from_name(name: &str) -> Option<Footwear> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    /// How it sounds.
    pub fn model(self) -> &'static SoleModel {
        match self {
            Footwear::Barefoot => &BAREFOOT,
            Footwear::Sneakers => &SNEAKERS,
            Footwear::LeatherShoes => &LEATHER,
            Footwear::Boots => &BOOTS,
            Footwear::Sandals => &SANDALS,
        }
    }
}

/// How a sole changes a step on hard ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoleModel {
    /// Its name.
    pub name: &'static str,
    /// The sole's own contact, seconds: the step lasts the longer of it and the ground's.
    pub contact: f32,
    /// The band of the sole's slap, Hz (`None`: the ground's click as it is).
    pub click: Option<(f32, f32)>,
    /// The slap's gain against a hard heel's.
    pub click_gain: f32,
    /// The thud's gain against a hard heel's.
    pub thump: f32,
    /// The heel's strength, walking.
    pub heel: f32,
    /// The ball's strength, walking.
    pub ball: f32,
    /// The toes' scuff as they push off (0: none) and its band, Hz.
    pub scuff: f32,
    /// Hz.
    pub scuff_band: (f32, f32),
    /// The step's level against hard shoes'.
    pub level: f32,
    /// A sandal slapping the heel as the foot lifts (0: none).
    pub slap: f32,
}

/// Bare feet.
pub const BAREFOOT: SoleModel = SoleModel {
    name: "barefoot",
    contact: 0.010,
    click: Some((150.0, 1500.0)),
    click_gain: 0.8,
    thump: 0.8,
    heel: 0.8,
    ball: 0.7,
    scuff: 0.0,
    scuff_band: (1000.0, 4000.0),
    level: 0.55,
    slap: 0.0,
};

/// Trainers.
pub const SNEAKERS: SoleModel = SoleModel {
    name: "sneakers",
    contact: 0.004,
    click: Some((300.0, 3000.0)),
    click_gain: 0.7,
    thump: 1.0,
    heel: 0.9,
    ball: 0.55,
    scuff: 0.3,
    scuff_band: (1500.0, 6000.0),
    level: 0.75,
    slap: 0.0,
};

/// Leather soles and heels.
pub const LEATHER: SoleModel = SoleModel {
    name: "leather",
    contact: 0.0,
    click: None,
    click_gain: 1.0,
    thump: 0.7,
    heel: 1.0,
    ball: 0.45,
    scuff: 0.4,
    scuff_band: (2500.0, 8000.0),
    level: 1.0,
    slap: 0.0,
};

/// Boots.
pub const BOOTS: SoleModel = SoleModel {
    name: "boots",
    contact: 0.0018,
    click: Some((400.0, 4500.0)),
    click_gain: 1.0,
    thump: 1.8,
    heel: 1.0,
    ball: 0.6,
    scuff: 0.35,
    scuff_band: (1500.0, 6000.0),
    level: 1.15,
    slap: 0.0,
};

/// Flip-flops.
pub const SANDALS: SoleModel = SoleModel {
    name: "sandals",
    contact: 0.005,
    click: Some((300.0, 3500.0)),
    click_gain: 0.7,
    thump: 0.6,
    heel: 0.85,
    ball: 0.5,
    scuff: 0.15,
    scuff_band: (1500.0, 6000.0),
    level: 0.75,
    slap: 1.2,
};

/// A foot landing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// What it lands on.
    pub surface: Surface,
    /// Where, in the world.
    pub position: DVec3,
    /// The walker's speed over the ground, m/s (1.4 walking, 3.5 and up running).
    pub speed: f32,
    /// The walker's weight with what it carries, kg.
    pub weight: f32,
    /// What it wears on its feet.
    pub footwear: Footwear,
    /// The frame it lands on (`None`: the next block).
    pub start: Option<u64>,
}

impl Step {
    /// A step of 80 kg walking at 1.4 m/s in leather shoes.
    pub fn walk(surface: Surface, position: DVec3) -> Self {
        Self {
            surface,
            position,
            speed: 1.4,
            weight: 80.0,
            footwear: Footwear::LeatherShoes,
            start: None,
        }
    }
}

/// A body hitting a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// What it hits.
    pub surface: Surface,
    /// What the body is, when it sounds as a surface of its own. A softer body is to the
    /// surface what a soft sole is to hard ground: the contact lasts as long as it lets it (a
    /// crate on a steel plate presses as long as wood does, so the plate rings lower), and the
    /// click is its own (a sack on the plate: none).
    pub against: Option<Surface>,
    /// Where, in the world.
    pub position: DVec3,
    /// The speed along the surface's normal, m/s.
    pub speed: f32,
    /// The body's mass, kg.
    pub mass: f32,
    /// The frame it hits on (`None`: the next block).
    pub start: Option<u64>,
}

/// One of two bodies meeting ([`Impact::between`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactBody {
    /// What it sounds as (`None`: nothing of its own, only what it hits rings).
    pub surface: Option<Surface>,
    /// Its mass, kg (`None`: fixed in the world, the ground or a wall).
    pub mass: Option<f32>,
}

impl Impact {
    /// Two bodies meeting at `position`, closing at `speed` along their contact's normal: each
    /// that sounds as a surface rings, struck by the other, on frame `start`. The pair's reduced
    /// mass, `m₁m₂ / (m₁ + m₂)`, drives both (a body on fixed ground: its own mass), as it sets
    /// the contact's force; the contact lasts as long as the softer surface lets it. None when
    /// both are fixed.
    pub fn between(
        a: ImpactBody,
        b: ImpactBody,
        position: DVec3,
        speed: f32,
        start: Option<u64>,
    ) -> impl Iterator<Item = Impact> {
        let mass = match (a.mass, b.mass) {
            (Some(x), Some(y)) => Some(x * y / (x + y).max(f32::MIN_POSITIVE)),
            (Some(m), None) | (None, Some(m)) => Some(m),
            (None, None) => None,
        };
        let hit = move |struck: ImpactBody, by: ImpactBody| {
            Some(Impact {
                surface: struck.surface?,
                against: by.surface,
                position,
                speed,
                mass: mass?,
                start,
            })
        };
        [hit(a, b), hit(b, a)].into_iter().flatten()
    }
}

/// One contact of a sound being made.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contact {
    /// Seconds from the sound's start.
    at: f32,
    /// The force pulse's width, seconds.
    width: f32,
    /// The press's length, seconds.
    press: f32,
    /// Its strength (1: the heel of a walking step).
    amp: f32,
    /// How much more the press stirs (grains, rustle, slosh) than its strength says: a
    /// running foot brushes more.
    brush: f32,
}

/// A short burst of noise beside the contacts: a scuff, a sandal's slap.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Burst {
    at: f32,
    length: f32,
    band: NoiseBand,
}

/// What the sole does to the ground's click and thud.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sole {
    band: Option<(f32, f32)>,
    click: f32,
    thump: f32,
}

impl Sole {
    const NONE: Sole = Sole {
        band: None,
        click: 1.0,
        thump: 1.0,
    };
}

/// A step's sound: mono, at `rate`, from `seed` (the same seed gives the same samples).
pub fn step_sound(step: &Step, seed: u64, rate: u32) -> SampleBuffer {
    let model = step.surface.model();
    let shoe = step.footwear.model();
    let mut rng = SplitMix64::new(seed);
    let speed = step.speed.max(0.3);
    let run = smoothstep(1.8, 3.6, speed);
    let gap = lerp(0.085, 0.022, run) * rng.range_f32(0.85, 1.15);
    let shorter = lerp(1.0, 0.6, run);
    let press = model.press * lerp(1.0, 0.7, run);
    let brush = lerp(1.0, 1.8, run);
    // On hard ground the softer of sole and ground sets the contact.
    let contact = if model.hard {
        model.contact.max(shoe.contact)
    } else {
        model.contact
    } * rng.range_f32(0.75, 1.3);
    let ball = lerp(shoe.ball, 1.0, run) * rng.range_f32(0.7, 1.2);
    let contacts = [
        Contact {
            at: 0.001,
            width: contact * shorter,
            press,
            amp: lerp(shoe.heel, 0.35 * shoe.heel, run),
            brush,
        },
        Contact {
            at: 0.001 + gap,
            width: contact * shorter * 1.5,
            press,
            amp: ball,
            brush,
        },
    ];
    let sole = if model.hard {
        Sole {
            band: shoe.click,
            click: shoe.click_gain * db_to_gain(rng.range_f32(-3.0, 3.0)),
            thump: shoe.thump,
        }
    } else {
        Sole {
            click: db_to_gain(rng.range_f32(-3.0, 3.0)),
            ..Sole::NONE
        }
    };
    // The toes' scuff on hard ground, two steps in three; a sandal's slap on any ground.
    let mut bursts = Vec::with_capacity(2);
    let scuffs = rng.next_f32() < 0.65;
    let (scuff_at, scuff_length, scuff_gain) = (
        rng.range_f32(0.04, 0.09),
        rng.range_f32(0.03, 0.06),
        rng.range_f32(0.4, 1.0),
    );
    if model.hard && scuffs && shoe.scuff > 0.0 {
        let click = model.click.map_or(0.3, |c| c.gain);
        bursts.push(Burst {
            at: 0.001 + gap + scuff_at,
            length: scuff_length,
            band: NoiseBand {
                low: shoe.scuff_band.0,
                high: shoe.scuff_band.1,
                decay: 60.0,
                gain: shoe.scuff * click * scuff_gain,
            },
        });
    }
    let slap_at = rng.range_f32(0.12, 0.2);
    let slap_gain = rng.range_f32(0.7, 1.1);
    if shoe.slap > 0.0 {
        bursts.push(Burst {
            at: 0.001 + gap + slap_at,
            length: 0.008,
            band: NoiseBand {
                low: 500.0,
                high: 3500.0,
                decay: 150.0,
                gain: shoe.slap * slap_gain,
            },
        });
    }
    let level = model.level
        * shoe.level
        * (step.weight.max(1.0) / 80.0 * speed / 1.4).sqrt()
        * db_to_gain(rng.range_f32(-1.5, 1.5));
    synthesize(model, &contacts, &bursts, sole, level, &mut rng, rate)
}

/// An impact's sound: mono, at `rate`, from `seed`. A heavier or faster body presses longer
/// (`width ∝ mass^0.4 / speed^0.2`, as a Hertzian contact does) and sounds louder (the
/// amplitude `∝ √(mass) × speed`, relative to 1 kg at 2 m/s, at most +18 dB). A body softer than
/// the surface ([`Impact::against`]) presses as long as it lets the contact last and clicks in
/// its own band.
pub fn impact_sound(impact: &Impact, seed: u64, rate: u32) -> SampleBuffer {
    let model = impact.surface.model();
    let mut rng = SplitMix64::new(seed);
    let mass = impact.mass.max(0.01);
    let speed = impact.speed.max(0.05);
    // A softer body is to the surface what a soft sole is to hard ground: the contact lasts as
    // long as it lets it, and the click is its own (none for grass or gravel).
    let softer = impact
        .against
        .map(Surface::model)
        .filter(|by| by.contact > model.contact);
    let contact = softer.map_or(model.contact, |by| by.contact);
    let sole = softer.map_or(Sole::NONE, |by| Sole {
        band: by.click.map(|band| (band.low, band.high)),
        click: if by.click.is_some() { 1.0 } else { 0.0 },
        thump: 1.0,
    });
    let width = (contact * mass.powf(0.4) * (2.0 / speed).powf(0.2)).clamp(0.00015, 0.04);
    let press = (model.press * mass.powf(0.33)).clamp(0.01, 0.4);
    let contacts = [Contact {
        at: 0.001,
        width,
        press,
        amp: 1.0,
        brush: 1.0,
    }];
    let level = model.level * (mass.sqrt() * speed / 2.0).min(8.0);
    synthesize(model, &contacts, &[], sole, level, &mut rng, rate)
}

/// Seconds of fade at a sound's end: it ends on 0, whatever is still ringing.
const FADE_OUT: f32 = 0.005;

/// Makes the sound of `contacts` on `model`, its peak at `level`.
fn synthesize(
    model: &SurfaceModel,
    contacts: &[Contact],
    bursts: &[Burst],
    sole: Sole,
    level: f32,
    rng: &mut SplitMix64,
    rate: u32,
) -> SampleBuffer {
    let fs = rate as f32;
    let dt = 1.0 / fs;
    // How long: the last contact's press, then until the slowest part has fallen 80 dB.
    let end = contacts
        .iter()
        .map(|c| c.at + c.width.max(c.press))
        .chain(bursts.iter().map(|b| b.at + b.length + 9.2 / b.band.decay))
        .fold(0.0f32, f32::max);
    let mut slowest = model.thump.decay;
    for m in model.modes {
        slowest = slowest.min(m.decay);
    }
    for band in [model.click, model.body].into_iter().flatten() {
        slowest = slowest.min(band.decay);
    }
    let mut seconds = end + 9.2 / slowest.max(1.0);
    if let Some(b) = model.bubbles {
        let f0 = 3.26 / b.radius[1];
        seconds = seconds.max(end + b.spread + 9.2 / (0.13 * f0 + 0.0072 * f0 * f0.sqrt()));
    }
    let frames = ((seconds.min(2.5) + FADE_OUT) * fs).ceil() as usize;

    // The drives: the contacts' force (each normalised to a unit impulse, so a mode rings at
    // its gain whatever the width) and the presses (peak 1, times the brush).
    let mut hit = vec![0.0f32; frames];
    let mut hit_peak = vec![0.0f32; frames];
    let mut press = vec![0.0f32; frames];
    let mut first = frames;
    let mut last = 0;
    for c in contacts {
        let area = 0.5 * c.width * fs;
        let from = ((c.at * fs) as usize).min(frames);
        let to = (((c.at + c.width.max(c.press)) * fs).ceil() as usize).min(frames);
        first = first.min(from);
        last = last.max(to);
        for i in from..to {
            let t = i as f32 * dt - c.at;
            let f = force_pulse(t, c.width);
            hit[i] += c.amp * f / area;
            hit_peak[i] += c.amp * f;
            press[i] += c.amp * c.brush * force_pulse(t, c.press);
        }
    }
    // Each part runs from the first contact until it has fallen 80 dB.
    let until = |decay: f32| (last + (9.2 / decay.max(1.0) * fs) as usize).min(frames);

    let mut out = vec![0.0f32; frames];
    // Modes and the thump.
    let thump = Mode {
        gain: model.thump.gain * sole.thump,
        ..model.thump
    };
    for m in model.modes.iter().chain(std::iter::once(&thump)) {
        let jitter = 1.0 + model.jitter * rng.range_f32(-1.0, 1.0);
        let gain = m.gain * rng.range_f32(0.6, 1.4);
        let mut r = Resonator::new(m.frequency * jitter, m.decay, gain, rate);
        let end = until(m.decay);
        for (o, &x) in out[first..end].iter_mut().zip(&hit[first..end]) {
            *o += r.tick(x);
        }
    }
    // The click follows the contacts, the body the presses.
    if let Some(mut band) = model.click {
        if let Some((low, high)) = sole.band {
            // A soft sole slaps in its own band (air pressed out from under it).
            band.low = low;
            band.high = high;
        } else {
            // The ground's own click: a longer contact excites less of the top, so it stops
            // near 6 / contact (a heavy body on stone is darker than a light one).
            let width = contacts.iter().map(|c| c.width).fold(f32::MAX, f32::min);
            band.high = band.high.min(6.0 / width.max(1e-5)).max(band.low * 1.5);
        }
        band.gain *= sole.click;
        let end = until(band.decay);
        add_noise_band(&mut out[first..end], &hit_peak[first..end], band, rng, rate);
    }
    // The scuff and the slap: noise driven by their own `sin²`.
    for burst in bursts {
        let from = ((burst.at * fs) as usize).min(frames);
        let to = (((burst.at + burst.length) * fs) as usize).min(frames);
        let end = (to + (9.2 / burst.band.decay * fs) as usize).min(frames);
        let drive: Vec<f32> = (from..end)
            .map(|i| force_pulse(i as f32 * dt - burst.at, burst.length))
            .collect();
        add_noise_band(&mut out[from..end], &drive, burst.band, rng, rate);
    }
    if let Some(band) = model.body {
        let end = until(band.decay);
        add_noise_band(&mut out[first..end], &press[first..end], band, rng, rate);
    }
    // Grains, a Poisson process whose rate follows the press (by time rescaling: one draw an
    // event).
    if let Some(g) = model.grains {
        let mut next = -(1.0 - rng.next_f32()).max(1e-7).ln();
        let mut lambda = 0.0f32;
        for (i, &p) in press.iter().enumerate().take(last).skip(first) {
            lambda += g.rate * p * dt;
            while lambda >= next {
                next += -(1.0 - rng.next_f32()).max(1e-7).ln();
                let grain = Grain {
                    frequency: log_uniform(rng, g.low, g.high),
                    chirp: 0.0,
                    decay: rng.range_f32(g.decay[0], g.decay[1]),
                    amplitude: g.gain * log_uniform(rng, 0.1, 1.0) * p.sqrt(),
                    pan: 0.0,
                };
                grain.add_mono(&mut out, i, rate);
            }
        }
    }
    if let Some(b) = model.bubbles {
        for c in contacts {
            let count = b.count[0] + rng.below((b.count[1] - b.count[0] + 1) as u64) as u32;
            for _ in 0..count {
                let at = c.at + c.width + b.spread * rng.next_f32() * rng.next_f32();
                let radius = log_uniform(rng, b.radius[0], b.radius[1]);
                let amp = b.gain * c.amp * c.brush * (radius / 0.003) * log_uniform(rng, 0.3, 1.0);
                Grain::bubble(radius, amp, 0.0).add_mono(&mut out, (at * fs) as usize, rate);
            }
        }
    }

    // The peak at `level`, a fade to 0 at the end. Every part is band-limited where it is
    // made (the noise bands by their own low-pass, the modes and grains below 10 kHz), so
    // nothing reaches the top octave.
    let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let scale = if peak > 0.0 { level / peak } else { 0.0 };
    let fade = ((FADE_OUT * fs) as usize).min(frames);
    for (i, x) in out.iter_mut().enumerate() {
        *x *= scale;
        let left = frames - 1 - i;
        if left < fade {
            let w = left as f32 / fade as f32;
            *x *= 0.5 - 0.5 * (std::f32::consts::PI * w).cos();
        }
    }
    SampleBuffer::mono(rate, out)
}

/// Adds white noise shaped by `drive` (held from its peaks, falling at the band's decay) and
/// filtered to the band.
fn add_noise_band(
    out: &mut [f32],
    drive: &[f32],
    band: NoiseBand,
    rng: &mut SplitMix64,
    rate: u32,
) {
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let mut low = Biquad::high_pass(band.low, q, rate);
    // Eighth order above the band: noise leaves the top octave empty.
    let mut high = [LowPass4::new(band.high, rate); 2];
    let fall = (-band.decay / rate as f32).exp();
    let mut env = 0.0f32;
    for (o, &d) in out.iter_mut().zip(drive) {
        env = d.max(env * fall);
        let x = synth::white(rng) * env;
        let y = high[0].tick(low.tick(x));
        *o += band.gain * high[1].tick(y);
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 0 below `low`, 1 above `high`, smooth between.
pub(crate) fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Plays footsteps and impacts: synthesises each on the calling thread and starts it as a
/// spatial voice.
#[derive(Clone, Debug)]
pub struct Foley {
    rate: u32,
    rng: SplitMix64,
    /// How each sound plays: its bus, sends, priority and distance curve; the position and
    /// the start come from the event.
    pub params: PlayParams,
}

impl Foley {
    /// Footsteps at `rate`, varied from `seed`, played into `params.bus` (a spatial voice
    /// with a 1 m reference distance unless `params.spatial` says otherwise).
    pub fn new(rate: u32, seed: u64, params: PlayParams) -> Self {
        Self {
            rate,
            rng: SplitMix64::new(seed),
            params,
        }
    }

    /// How a sound at `position`, on frame `start`, plays: placed, at its own pitch.
    fn placed(&mut self, position: DVec3, start: Option<u64>) -> PlayParams {
        let mut params = self.params;
        let mut spatial = params.spatial.unwrap_or(Spatial {
            attenuation: Attenuation::Inverse {
                reference: 1.0,
                max: 200.0,
                rolloff: 1.0,
            },
            spread: 0.3,
            ..Spatial::at(position)
        });
        spatial.position = position;
        params.spatial = Some(spatial);
        params.start = start;
        params.looping = false;
        params.pitch *= self.rng.range_f32(0.975, 1.025);
        params
    }

    /// The listener's own foot lands: played without a place (straight below the head an
    /// HRTF has no measurements and a panner no direction), at the level of 1.6 m away and a
    /// little to the side of the foot (`foot`: −1 left, 1 right).
    pub fn own_step(&mut self, engine: &mut AudioEngine, step: &Step, foot: f32) -> VoiceId {
        let (step, seed, rate) = (*step, self.rng.next_u64(), self.rate);
        let mut params = self.params;
        params.spatial = None;
        params.gain *= 1.0 / 1.6;
        params.pan = 0.12 * foot.clamp(-1.0, 1.0);
        params.start = step.start;
        params.looping = false;
        params.pitch *= self.rng.range_f32(0.975, 1.025);
        engine.play_made(params, move || step_sound(&step, seed, rate))
    }

    /// A foot lands.
    pub fn step(&mut self, engine: &mut AudioEngine, step: &Step) -> VoiceId {
        let (step, seed, rate) = (*step, self.rng.next_u64(), self.rate);
        let params = self.placed(step.position, step.start);
        engine.play_made(params, move || step_sound(&step, seed, rate))
    }

    /// A body hits a surface.
    pub fn impact(&mut self, engine: &mut AudioEngine, impact: &Impact) -> VoiceId {
        let (impact, seed, rate) = (*impact, self.rng.next_u64(), self.rate);
        let params = self.placed(impact.position, impact.start);
        engine.play_made(params, move || impact_sound(&impact, seed, rate))
    }
}
